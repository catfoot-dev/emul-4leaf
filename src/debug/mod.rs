pub mod common;

use std::sync::mpsc::{Receiver, Sender};

// 그래픽 관련
use embedded_graphics::{pixelcolor::Rgb888, prelude::*, text::Text};
use embedded_ttf::FontTextStyleBuilder;
use rusttype::Font as TtfFont;

static FONT_DATA: std::sync::OnceLock<&'static [u8]> = std::sync::OnceLock::new();
static TTF_FONT: std::sync::OnceLock<TtfFont<'static>> = std::sync::OnceLock::new();

pub const LOG_SCROLL_MAX: usize = 3000;

use crate::debug::common::{CpuContext, DebugCommand};
use crate::ui::Painter;

struct FrameBuffer<'a> {
    buffer: &'a mut [u32],
    width: u32,
}
impl<'a> DrawTarget for FrameBuffer<'a> {
    type Color = Rgb888;
    type Error = core::convert::Infallible;
    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(Point { x, y }, color) in pixels.into_iter() {
            if x >= 0 && x < self.width as i32 {
                let offset = (y as u32 * self.width + x as u32) as usize;
                if offset < self.buffer.len() {
                    self.buffer[offset] =
                        ((color.r() as u32) << 16) | ((color.g() as u32) << 8) | (color.b() as u32);
                }
            }
        }
        Ok(())
    }
}
impl<'a> OriginDimensions for FrameBuffer<'a> {
    fn size(&self) -> Size {
        Size::new(self.width, (self.buffer.len() as u32) / self.width)
    }
}

/// 백그라운드 스레드의 에뮬레이터 코어 상태에 동기화되어 주기적으로 화면(`winit` Window)을
/// 렌더링하고 유저의 핫키 조작을 에뮬레이터 코어에 전달하는 UI 객체
pub struct Debug {
    cmd_tx: Sender<DebugCommand>,
    state_rx: Receiver<CpuContext>,
    cpu_state: Option<CpuContext>,
    waiting_for_step: bool,
    auto_running: bool,
    last_log_count: usize,
    log_scroll_offset: usize,
}

impl Debug {
    pub fn new(cmd_tx: Sender<DebugCommand>, state_rx: Receiver<CpuContext>) -> Self {
        Debug {
            cmd_tx: cmd_tx,
            state_rx: state_rx,
            cpu_state: None,
            waiting_for_step: false,
            auto_running: true,
            last_log_count: 0,
            log_scroll_offset: 0,
        }
    }

    pub fn update_state(&mut self, state: CpuContext) {
        self.cpu_state = Some(state);
        self.waiting_for_step = false;
    }

    pub fn last_log_count(&self) -> usize {
        self.last_log_count
    }
}

impl Painter for Debug {
    fn create_window(
        &self,
        event_loop: &winit::event_loop::ActiveEventLoop,
    ) -> winit::window::Window {
        event_loop
            .create_window(
                winit::window::Window::default_attributes()
                    .with_title("emul-4leaf Debugger")
                    .with_inner_size(winit::dpi::LogicalSize::new(800, 600)),
            )
            .unwrap()
    }

    fn quit_on_close(&self) -> bool {
        true
    }

    fn tick(&mut self) -> bool {
        let mut needs_redraw = false;

        // CPU 상태 업데이트 확인
        while let Ok(new_state) = self.state_rx.try_recv() {
            self.update_state(new_state);
            needs_redraw = true;
        }

        // 로그 변경 확인
        let current_log_count = crate::LOG_COUNT.load(std::sync::atomic::Ordering::Relaxed);
        if self.last_log_count != current_log_count {
            needs_redraw = true;
        }

        needs_redraw
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn handle_event(
        &mut self,
        event: &winit::event::WindowEvent,
        event_loop: &winit::event_loop::ActiveEventLoop,
    ) -> bool {
        use winit::event::{ElementState, WindowEvent};
        use winit::keyboard::{KeyCode, PhysicalKey};

        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return false;
                }

                let mut redraw = false;

                if PhysicalKey::Code(KeyCode::F10) == event.physical_key && !self.waiting_for_step {
                    let _ = self.cmd_tx.send(DebugCommand::Step);
                    self.waiting_for_step = true;
                    redraw = true;
                }
                if PhysicalKey::Code(KeyCode::F5) == event.physical_key {
                    if self.auto_running {
                        self.auto_running = false;
                        let _ = self.cmd_tx.send(DebugCommand::Pause);
                        crate::emu_log!("[DEBUG] UI: Pause requested");
                    } else {
                        self.auto_running = true;
                        let _ = self.cmd_tx.send(DebugCommand::Run);
                        crate::emu_log!("[DEBUG] UI: Run requested");
                    }
                    redraw = true;
                }
                if PhysicalKey::Code(KeyCode::Escape) == event.physical_key {
                    event_loop.exit();
                }

                // Log Scrolling
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::PageUp) => {
                        self.log_scroll_offset = (self.log_scroll_offset + 10).min(LOG_SCROLL_MAX);
                        redraw = true;
                    }
                    PhysicalKey::Code(KeyCode::PageDown) => {
                        self.log_scroll_offset = self.log_scroll_offset.saturating_sub(10);
                        redraw = true;
                    }
                    PhysicalKey::Code(KeyCode::Home) => {
                        self.log_scroll_offset = LOG_SCROLL_MAX;
                        redraw = true;
                    }
                    PhysicalKey::Code(KeyCode::End) => {
                        self.log_scroll_offset = 0;
                        redraw = true;
                    }
                    PhysicalKey::Code(KeyCode::ArrowUp) => {
                        self.log_scroll_offset = (self.log_scroll_offset + 1).min(LOG_SCROLL_MAX);
                        redraw = true;
                    }
                    PhysicalKey::Code(KeyCode::ArrowDown) => {
                        self.log_scroll_offset = self.log_scroll_offset.saturating_sub(1);
                        redraw = true;
                    }
                    _ => {}
                }
                redraw
            }
            WindowEvent::MouseWheel { delta, .. } => {
                use winit::event::MouseScrollDelta;
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => *y as i32,
                    MouseScrollDelta::PixelDelta(pos) => (pos.y / 12.0) as i32,
                };
                if lines > 0 {
                    self.log_scroll_offset =
                        (self.log_scroll_offset + lines as usize).min(LOG_SCROLL_MAX);
                } else {
                    self.log_scroll_offset =
                        self.log_scroll_offset.saturating_sub((-lines) as usize);
                }
                true
            }
            _ => false,
        }
    }

    fn paint(&mut self, buffer: &mut [u32], width: u32, height: u32) {
        // 그리기 도구 준비
        let mut display = FrameBuffer { buffer, width };

        // TTF 폰트 초기화 (한글 지원)
        let ttf_font = TTF_FONT.get_or_init(|| {
            let data = FONT_DATA.get_or_init(|| {
                let path = "Dotum.ttf";
                std::fs::read(path).unwrap_or_else(|_| Vec::new()).leak()
            });
            TtfFont::try_from_bytes(data).expect("Failed to load font")
        });

        let style_w = FontTextStyleBuilder::new(ttf_font.clone())
            .font_size(12)
            .text_color(Rgb888::WHITE)
            .anti_aliasing_color(Rgb888::BLACK)
            .build();
        let style_r = FontTextStyleBuilder::new(ttf_font.clone())
            .font_size(12)
            .text_color(Rgb888::RED)
            .anti_aliasing_color(Rgb888::BLACK)
            .build();
        let style_y = FontTextStyleBuilder::new(ttf_font.clone())
            .font_size(12)
            .text_color(Rgb888::YELLOW)
            .anti_aliasing_color(Rgb888::BLACK)
            .build();
        let style_c = FontTextStyleBuilder::new(ttf_font.clone())
            .font_size(12)
            .text_color(Rgb888::CYAN)
            .anti_aliasing_color(Rgb888::BLACK)
            .build();

        if let Some(state) = self.cpu_state.as_ref() {
            // 그리기 로직
            // 레지스터 출력
            let reg_names = [
                "EAX", "EBX", "ECX", "EDX", "ESI", "EDI", "EBP", "ESP", "EIP",
            ];
            let mut y = 10;

            Text::new("REGISTERS", Point::new(10, y), style_y.clone())
                .draw(&mut display)
                .ok();
            y += 15;

            for (i, val) in state.regs.iter().enumerate() {
                let style = if i == 8 {
                    style_c.clone()
                } else {
                    style_w.clone()
                }; // EIP 강조
                let text = format!("{}: 0x{:08x}", reg_names[i], val);
                Text::new(&text, Point::new(10, y), style)
                    .draw(&mut display)
                    .ok();
                y += 13;
            }

            // 다음 명령어 출력
            y += 10;
            Text::new("NEXT OP:", Point::new(10, y), style_y.clone())
                .draw(&mut display)
                .ok();
            y += 15;
            Text::new(&state.next_instr, Point::new(10, y), style_c.clone())
                .draw(&mut display)
                .ok();

            // 스택 뷰 출력 (오른쪽)
            let stack_x = 200;
            let mut stack_y = 10;
            Text::new(
                "STACK (TOP 10)",
                Point::new(stack_x, stack_y),
                style_y.clone(),
            )
            .draw(&mut display)
            .ok();
            stack_y += 15;

            for (addr, val) in &state.stack {
                let mark = if *addr == state.regs[7] { "<- ESP" } else { "" };
                let text = format!("0x{:08x}: 0x{:08x} {}", addr, val, mark);
                Text::new(&text, Point::new(stack_x, stack_y), style_w.clone())
                    .draw(&mut display)
                    .ok();
                stack_y += 13;
            }
            let mode_str = if self.auto_running {
                "[AUTO-RUN]"
            } else {
                "[STEP]"
            };
            let mode_color = if self.auto_running {
                FontTextStyleBuilder::new(ttf_font.clone())
                    .font_size(12)
                    .text_color(Rgb888::new(0, 255, 128))
                    .anti_aliasing_color(Rgb888::BLACK)
                    .build()
            } else {
                style_y.clone()
            };
            Text::new(
                &format!(
                    "Mode: {}  |  F5: Run/Pause  |  F10: Step  |  ESC: Quit",
                    mode_str
                ),
                Point::new(10, 390),
                mode_color,
            )
            .draw(&mut display)
            .ok();
        } else {
            Text::new(
                "Waiting for CPU state...",
                Point::new(10, 10),
                style_w.clone(),
            )
            .draw(&mut display)
            .ok();
        }

        // === GUI: Log Box ===
        // Draw a separator line
        for x in 10..width.saturating_sub(10) {
            Pixel(Point::new(x as i32, 405), Rgb888::new(100, 100, 100))
                .draw(&mut display)
                .ok();
        }

        const LOG_Y_START: i32 = 407;
        const LOG_X_MAX: usize = 145;

        if let Some(buf) = crate::LOG_BUFFER.get()
            && let Ok(b) = buf.try_lock()
        {
            let current_count = crate::LOG_COUNT.load(std::sync::atomic::Ordering::Relaxed);
            self.last_log_count = current_count;

            let lines_to_show = height.saturating_sub(LOG_Y_START as u32) as usize / 13; // 14;
            let total_logs = b.len();

            // 스크롤 오프셋을 고려하여 보여줄 범위 계산
            if total_logs < lines_to_show {
                self.log_scroll_offset = 0;
            } else if self.log_scroll_offset > total_logs.saturating_sub(lines_to_show) {
                self.log_scroll_offset = total_logs.saturating_sub(lines_to_show);
            }
            let end_idx = total_logs.saturating_sub(self.log_scroll_offset);
            let start_idx = end_idx.saturating_sub(lines_to_show);
            let row_count = end_idx - start_idx;

            let mut log_y = LOG_Y_START
                + lines_to_show.saturating_sub(row_count).min(lines_to_show) as i32 * 13;
            for line in b.iter().skip(start_idx).take(row_count) {
                // Trim the line if it's too long
                let text = line.clone();
                // let text = if text.chars().count() > LOG_X_MAX {
                //     format!("{}...", &text[0..LOG_X_MAX])
                // } else {
                //     text.clone()
                // };
                let no = if text.starts_with("[0x") {
                    &text[1..9]
                } else {
                    &text
                };
                let message = if text.starts_with("[0x") {
                    format!("[             ] {}", &text[11..])
                } else {
                    format!("")
                };

                // 한글을 위해 TTF 스타일 사용
                let style = if message.contains("[!]") {
                    style_r.clone()
                } else if message.contains("[*]") {
                    style_c.clone()
                } else {
                    style_w.clone()
                };
                Text::new(&message, Point::new(10, log_y), style)
                    .draw(&mut display)
                    .ok();
                Text::new(&no, Point::new(14, log_y), style_w.clone())
                    .draw(&mut display)
                    .ok();

                log_y += 13;
            }
        }
    }
}
