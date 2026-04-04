pub mod common;

use std::sync::mpsc::{Receiver, Sender};

// 그래픽 관련
use embedded_graphics::{pixelcolor::Rgb888, prelude::*, text::Text};
use embedded_ttf::FontTextStyleBuilder;
use rusttype::Font as TtfFont;

static FONT_DATA: std::sync::OnceLock<&'static [u8]> = std::sync::OnceLock::new();
static TTF_FONT: std::sync::OnceLock<TtfFont<'static>> = std::sync::OnceLock::new();

pub const LOG_SCROLL_MAX: usize = 5000;

use crate::debug::common::{CpuContext, DebugCommand};
use crate::ui::Painter;

/// 현재 실행 중인 바이너리가 디버그 빌드인지 판별합니다.
#[inline(always)]
pub const fn is_debug_mode() -> bool {
    cfg!(debug_assertions)
}

/// 디버그 창을 생성할지 여부를 결정합니다.
#[inline(always)]
pub const fn should_create_debug_window() -> bool {
    is_debug_mode()
}

/// 디버그 창으로 상태와 로그를 전달할지 여부를 결정합니다.
#[inline(always)]
pub const fn should_send_debug_messages() -> bool {
    should_create_debug_window()
}

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
    last_socket_count: usize,
    log_scroll_offset: usize,
    input_buffer: String, // 서버로 보낼 Hex 데이터 입력 버퍼
    show_stack: bool,
    show_socket_log: bool,
    show_log: bool,
}

impl Debug {
    /// 디버그 창 페인터를 생성합니다.
    pub fn new(cmd_tx: Sender<DebugCommand>, state_rx: Receiver<CpuContext>) -> Self {
        Debug {
            cmd_tx: cmd_tx,
            state_rx: state_rx,
            cpu_state: None,
            waiting_for_step: false,
            auto_running: true,
            last_log_count: 0,
            last_socket_count: 0,
            log_scroll_offset: 0,
            input_buffer: String::new(),
            show_stack: true,
            show_socket_log: true,
            show_log: true,
        }
    }

    /// 최신 CPU 상태를 반영하고 스텝 대기 상태를 해제합니다.
    pub fn update_state(&mut self, state: CpuContext) {
        self.cpu_state = Some(state);
        self.waiting_for_step = false;
    }

    /// 마지막으로 반영한 로그 카운터 값을 반환합니다.
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
                    .with_title("4Leaf Emulator Debugger")
                    .with_inner_size(winit::dpi::LogicalSize::new(1600, 600)),
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
            self.last_log_count = current_log_count;
            needs_redraw = true;
        }

        // 소켓 로그 변경 확인
        let current_socket_count =
            crate::SOCKET_LOG_COUNT.load(std::sync::atomic::Ordering::Relaxed);
        if self.last_socket_count != current_socket_count {
            self.last_socket_count = current_socket_count;
            needs_redraw = true;
        }

        needs_redraw
    }

    fn poll_interval(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_millis(10))
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
                    PhysicalKey::Code(KeyCode::Backspace) => {
                        self.input_buffer.pop();
                        redraw = true;
                    }
                    PhysicalKey::Code(KeyCode::Enter) => {
                        if !self.input_buffer.is_empty() {
                            let trimmed = self.input_buffer.trim();
                            match hex::decode(trimmed) {
                                Ok(_bytes) => {
                                    // 채널 기반 구조로 변경 후 브로드캐스트 송신 제거됨
                                    crate::emu_socket_log!(
                                        "[UI] Packet input disabled (channel mode): {}",
                                        trimmed
                                    );
                                }
                                Err(e) => {
                                    crate::emu_socket_log!("[UI] Hex Error: {}", e);
                                }
                            }
                            self.input_buffer.clear();
                            redraw = true;
                        }
                    }
                    _ => {
                        // Hex 입력 처리 (0-9, A-F)
                        if let Some(text) = event.text.as_ref() {
                            for c in text.chars() {
                                if c.is_ascii_hexdigit() {
                                    self.input_buffer.push(c.to_ascii_uppercase());
                                    redraw = true;
                                } else if self.input_buffer.is_empty() {
                                    // 입력 버퍼가 비어있을 때만 토글 단축키 작동
                                    match c.to_ascii_uppercase() {
                                        'S' => {
                                            self.show_stack = !self.show_stack;
                                            redraw = true;
                                        }
                                        'K' => {
                                            self.show_socket_log = !self.show_socket_log;
                                            redraw = true;
                                        }
                                        'L' => {
                                            self.show_log = !self.show_log;
                                            redraw = true;
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
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
                let path = "gulim.ttf";
                std::fs::read(path).unwrap_or_else(|_| Vec::new()).leak()
            });
            TtfFont::try_from_bytes(data).expect("Failed to load font")
        });

        let style_w = FontTextStyleBuilder::new(ttf_font.clone())
            .font_size(18)
            .text_color(Rgb888::WHITE)
            .build();
        let style_r = FontTextStyleBuilder::new(ttf_font.clone())
            .font_size(18)
            .text_color(Rgb888::RED)
            .build();
        let style_y = FontTextStyleBuilder::new(ttf_font.clone())
            .font_size(18)
            .text_color(Rgb888::YELLOW)
            .build();
        let style_c = FontTextStyleBuilder::new(ttf_font.clone())
            .font_size(18)
            .text_color(Rgb888::CYAN)
            .build();

        // === GUI: Header (Mode & Shortcuts) ===
        let mode_str = if self.auto_running {
            "[AUTO-RUN]"
        } else {
            "[STEP]"
        };
        let mode_color = if self.auto_running {
            FontTextStyleBuilder::new(ttf_font.clone())
                .font_size(18)
                .text_color(Rgb888::new(0, 255, 128))
                .build()
        } else {
            style_y.clone()
        };

        Text::new(
            &format!(
                "Mode: {}  |  F5: Run/Pause  |  F10: Step  |  Toggles: [S]tate [K]Socket [L]Log  |  ESC: Quit",
                mode_str
            ),
            Point::new(3, 3),
            mode_color,
        )
        .draw(&mut display)
        .ok();

        // === 동적 레이아웃 계산 ===
        let header_h = 22i32;
        let usable_h = height as i32 - header_h;
        let active_panels =
            (self.show_stack as i32) + (self.show_socket_log as i32) + (self.show_log as i32);

        if active_panels == 0 {
            return;
        }

        let panel_h = usable_h / active_panels;
        let mut current_y = header_h;

        // === GUI: CPU State (Registers & Stack) ===
        if self.show_stack {
            // 구분선
            for x in 3..width.saturating_sub(3) {
                Pixel(Point::new(x as i32, current_y), Rgb888::new(60, 120, 60))
                    .draw(&mut display)
                    .ok();
            }
            current_y += 2;

            if let Some(state) = self.cpu_state.as_ref() {
                // 레지스터 출력
                let reg_names = [
                    "EAX", "EBX", "ECX", "EDX", "ESI", "EDI", "EBP", "ESP", "EIP",
                ];
                let mut reg_y = current_y + 3;

                for (i, val) in state.regs.iter().enumerate() {
                    let style = if i == 8 {
                        style_c.clone()
                    } else {
                        style_w.clone()
                    };
                    let text = format!("{}: 0x{:08x}", reg_names[i], val);
                    Text::new(&text, Point::new(3, reg_y), style)
                        .draw(&mut display)
                        .ok();
                    reg_y += 16;
                    if reg_y > current_y + panel_h - 10 {
                        break;
                    }
                }

                // 다음 명령어
                reg_y += 23;
                if reg_y < current_y + panel_h - 20 {
                    Text::new(&state.next_instr, Point::new(3, reg_y), style_c.clone())
                        .draw(&mut display)
                        .ok();
                }

                // 스택 뷰 (오른쪽)
                let stack_x = 200;
                let mut stack_y = current_y + 2;

                for (addr, val) in &state.stack {
                    let mark = if *addr == state.regs[7] { "<- ESP" } else { "" };
                    let text = format!("0x{:08x}: 0x{:08x} {}", addr, val, mark);
                    Text::new(&text, Point::new(stack_x, stack_y), style_w.clone())
                        .draw(&mut display)
                        .ok();
                    stack_y += 16;
                    if stack_y > current_y + panel_h - 10 {
                        break;
                    }
                }
            } else {
                Text::new(
                    "Waiting for CPU state...",
                    Point::new(3, current_y + 3),
                    style_w.clone(),
                )
                .draw(&mut display)
                .ok();
            }
            current_y += panel_h;
        }

        // === GUI: Socket Log Panel ===
        if self.show_socket_log {
            // 구분선
            for x in 3..width.saturating_sub(3) {
                Pixel(Point::new(x as i32, current_y), Rgb888::new(60, 120, 60))
                    .draw(&mut display)
                    .ok();
            }
            current_y += 3;

            let style_g = FontTextStyleBuilder::new(ttf_font.clone())
                .font_size(18)
                .text_color(Rgb888::new(100, 255, 100))
                .build();

            Text::new("SOCKET LOG", Point::new(3, current_y), style_y.clone())
                .draw(&mut display)
                .ok();

            let socket_content_h = panel_h - 40; // 제목 + 입력란 제외 높이
            let socket_lines_max = (socket_content_h / 18).max(1) as usize;
            if let Some(buf) = crate::SOCKET_LOG_BUFFER.get() {
                if let Ok(b) = buf.try_lock() {
                    let total = b.len();
                    let start = total.saturating_sub(socket_lines_max);
                    let mut sy = current_y + 18;
                    for line in b.iter().skip(start) {
                        Text::new(line, Point::new(3, sy), style_g.clone())
                            .draw(&mut display)
                            .ok();
                        sy += 18;
                    }
                }
            }

            // 입력 버퍼 표시
            Text::new(
                &format!("Input: {}", self.input_buffer),
                Point::new(3, current_y + socket_content_h + 16),
                style_y.clone(),
            )
            .draw(&mut display)
            .ok();

            current_y += panel_h;
        }

        // === GUI: Log Box ===
        if self.show_log {
            // 구분선
            for x in 3..width.saturating_sub(3) {
                Pixel(Point::new(x as i32, current_y), Rgb888::new(60, 120, 60))
                    .draw(&mut display)
                    .ok();
            }
            let log_y_start = current_y + 3;

            if let Some(buf) = crate::LOG_BUFFER.get()
                && let Ok(b) = buf.try_lock()
            {
                let current_count = crate::LOG_COUNT.load(std::sync::atomic::Ordering::Relaxed);
                self.last_log_count = current_count;

                let remaining = (height as i32 - log_y_start).max(0) as usize;
                let lines_to_show = remaining / 18;
                let total_logs = b.len();

                if total_logs < lines_to_show {
                    self.log_scroll_offset = 0;
                } else if self.log_scroll_offset > total_logs.saturating_sub(lines_to_show) {
                    self.log_scroll_offset = total_logs.saturating_sub(lines_to_show);
                }
                let end_idx = total_logs.saturating_sub(self.log_scroll_offset);
                let start_idx = end_idx.saturating_sub(lines_to_show);
                let row_count = end_idx - start_idx;

                let mut log_y = log_y_start
                    + (lines_to_show.saturating_sub(row_count).min(lines_to_show) as i32 * 18);
                for line in b.iter().skip(start_idx).take(row_count) {
                    let text = line.clone();
                    let (no, message) = if text.starts_with("[0x") && text.len() >= 12 {
                        (&text[1..9], format!("[             ] {}", &text[11..]))
                    } else {
                        (text.as_str(), String::new())
                    };

                    let style = if message.contains("[!]") {
                        style_r.clone()
                    } else if message.contains("[*]") {
                        style_c.clone()
                    } else if message.contains("_stricmp") {
                        style_y.clone()
                    } else {
                        style_w.clone()
                    };

                    Text::new(&message, Point::new(3, log_y), style)
                        .draw(&mut display)
                        .ok();
                    Text::new(&no, Point::new(9, log_y), style_w.clone())
                        .draw(&mut display)
                        .ok();
                    log_y += 18;
                }
            }
        }
    }
}
