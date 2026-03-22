pub mod common;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::raw_window_handle::{DisplayHandle, HasDisplayHandle};
use winit::window::{Window, WindowId};

use anyhow::Result;
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::mpsc::{Receiver, Sender};

// 그래픽 관련
use embedded_graphics::{
    pixelcolor::Rgb888,
    prelude::*,
    text::Text,
};
use embedded_ttf::FontTextStyleBuilder;
use rusttype::Font as TtfFont;
use softbuffer::{Context as SoftContext, Surface};

static FONT_DATA: std::sync::OnceLock<&'static [u8]> = std::sync::OnceLock::new();
static TTF_FONT: std::sync::OnceLock<TtfFont<'static>> = std::sync::OnceLock::new();

use crate::debug::common::{CpuContext, DebugCommand, UiCommand};

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
#[derive(Default)]
pub struct Debug {
    cmd_tx: Option<Sender<DebugCommand>>,
    state_rx: Option<Receiver<CpuContext>>,
    ui_rx: Option<Receiver<UiCommand>>,
    cpu_state: Option<CpuContext>,
    waiting_for_step: bool,
    auto_running: bool,
    debug_window_id: Option<WindowId>,
    windows: HashMap<WindowId, Window>,
    hwnd_to_id: HashMap<u32, WindowId>,
    id_to_hwnd: HashMap<WindowId, u32>,
    last_log_count: usize,
    context: Option<SoftContext<DisplayHandle<'static>>>,
    log_scroll_offset: usize,
}

impl Debug {
    /// 새로운 `Debug` UI 인스턴스를 컨트롤 채널 정보와 메모리 덤프 수신 대기 채널을 연결하여 초기화
    pub fn new(
        cmd_tx: Sender<DebugCommand>,
        state_rx: Receiver<CpuContext>,
        ui_rx: Receiver<UiCommand>,
    ) -> Self {
        Debug {
            cmd_tx: Some(cmd_tx),
            state_rx: Some(state_rx),
            ui_rx: Some(ui_rx),
            cpu_state: None,
            waiting_for_step: false,
            auto_running: true,
            debug_window_id: None,
            windows: HashMap::new(),
            hwnd_to_id: HashMap::new(),
            id_to_hwnd: HashMap::new(),
            last_log_count: 0,
            context: None,
            log_scroll_offset: 0,
        }
    }
}

impl ApplicationHandler for Debug {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = event_loop
            .create_window(
                Window::default_attributes()
                    .with_title("emul-4leaf Debugger")
                    .with_inner_size(winit::dpi::LogicalSize::new(800, 600)),
            )
            .unwrap();

        if self.context.is_none() {
            // Safety: The event loop is guaranteed to live as long as the application.
            let display_handle = unsafe {
                std::mem::transmute::<DisplayHandle<'_>, DisplayHandle<'static>>(
                    event_loop.display_handle().unwrap(),
                )
            };
            self.context = Some(SoftContext::new(display_handle).unwrap());
        }

        let id = window.id();
        self.debug_window_id = Some(id);
        self.windows.insert(id, window);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let mut needs_redraw = false;

        // UI 조작 커맨드 처리 (창 생성 등)
        if let Some(ui_rx) = self.ui_rx.as_ref() {
            while let Ok(cmd) = ui_rx.try_recv() {
                match cmd {
                    UiCommand::CreateWindow {
                        hwnd,
                        title,
                        width,
                        height,
                    } => {
                        crate::emu_log!(
                            "[DEBUG] UI: Creating window for HWND {:#x} (\"{}\")",
                            hwnd,
                            title
                        );
                        let window = event_loop
                            .create_window(
                                Window::default_attributes()
                                    .with_title(title)
                                    .with_inner_size(winit::dpi::LogicalSize::new(width, height)),
                            )
                            .unwrap();
                        let id = window.id();
                        self.hwnd_to_id.insert(hwnd, id);
                        self.id_to_hwnd.insert(id, hwnd);
                        self.windows.insert(id, window);
                        needs_redraw = true;
                    }
                    UiCommand::DestroyWindow { hwnd } => {
                        if let Some(id) = self.hwnd_to_id.remove(&hwnd) {
                            self.id_to_hwnd.remove(&id);
                            self.windows.remove(&id);
                        }
                    }
                }
            }
        }

        if let Some(rx) = self.state_rx.as_ref() {
            if let Ok(new_state) = rx.try_recv() {
                self.cpu_state = Some(new_state);
                self.waiting_for_step = false;
                needs_redraw = true;
            }
        }

        let current_log_count = crate::LOG_COUNT.load(std::sync::atomic::Ordering::Relaxed);
        if current_log_count != self.last_log_count {
            self.last_log_count = current_log_count;
            needs_redraw = true;
        }

        if needs_redraw {
            for window in self.windows.values() {
                window.request_redraw();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::RedrawRequested => {
                let window = match self.windows.get(&id) {
                    Some(w) => w,
                    None => return,
                };

                let is_debug_window = Some(id) == self.debug_window_id;

                let (width, height) = {
                    let size = window.inner_size();
                    (size.width, size.height)
                };

                let context = self
                    .context
                    .as_ref()
                    .expect("Context should be initialized");
                let mut surface = Surface::new(context, window).unwrap();
                if let (Some(nw), Some(nh)) = (NonZeroU32::new(width), NonZeroU32::new(height)) {
                    surface.resize(nw, nh).unwrap();
                }

                let mut buffer = surface.buffer_mut().unwrap();
                buffer.fill(0); // 화면 지우기

                // 그리기 도구 준비
                let mut display = FrameBuffer {
                    buffer: &mut buffer,
                    width,
                };

                if is_debug_window {
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
                        let mut y = 20;

                        Text::new("REGISTERS", Point::new(10, y), style_y.clone())
                            .draw(&mut display)
                            .ok();
                        y += 15;

                        for (i, val) in state.regs.iter().enumerate() {
                            let style = if i == 8 { style_c.clone() } else { style_w.clone() }; // EIP 강조
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
                        let mut stack_y = 20;
                        Text::new("STACK (TOP 10)", Point::new(stack_x, stack_y), style_y.clone())
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
                            Point::new(10, 400),
                            mode_color,
                        )
                        .draw(&mut display)
                        .ok();
                    } else {
                        Text::new("Waiting for CPU state...", Point::new(10, 20), style_w.clone())
                            .draw(&mut display)
                            .ok();
                    }

                    // === GUI: Log Box ===
                    // Draw a separator line
                    for x in 10..width.saturating_sub(10) {
                        Pixel(Point::new(x as i32, 415), Rgb888::new(100, 100, 100))
                            .draw(&mut display)
                            .ok();
                    }

                    let mut log_y = 430;
                    if let Some(buf) = crate::LOG_BUFFER.get() {
                        if let Ok(b) = buf.try_lock() {
                            let lines_to_show = 14;
                            let total_logs = b.len();

                            // 스크롤 오프셋을 고려하여 보여줄 범위 계산
                            let end_idx = total_logs.saturating_sub(self.log_scroll_offset);
                            let start_idx = end_idx.saturating_sub(lines_to_show);

                            for line in b.iter().skip(start_idx).take(end_idx - start_idx) {
                                // Trim the line if it's too long
                                let max_len = 130;
                                let text = if line.len() > max_len {
                                    format!("{}...", &line[0..max_len])
                                } else {
                                    line.clone()
                                };

                                // 한글을 위해 TTF 스타일 사용
                                Text::new(&text, Point::new(10, log_y), style_w.clone())
                                    .draw(&mut display)
                                    .ok();

                                log_y += 13;
                            }
                        }
                    }
                } else {
                    // emulated window content
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

                    let hwnd = self.id_to_hwnd.get(&id).cloned().unwrap_or(0);
                    Text::new(&format!("HWND {:#x}", hwnd), Point::new(10, 20), style_w)
                        .draw(&mut display)
                        .ok();
                }

                buffer.present().unwrap();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                if PhysicalKey::Code(KeyCode::F10) == event.physical_key && !self.waiting_for_step {
                    if let Some(tx) = self.cmd_tx.as_ref() {
                        let _ = tx.send(DebugCommand::Step);
                        self.waiting_for_step = true;
                    }
                }
                if PhysicalKey::Code(KeyCode::F5) == event.physical_key {
                    if let Some(tx) = self.cmd_tx.as_ref() {
                        if self.auto_running {
                            self.auto_running = false;
                            let _ = tx.send(DebugCommand::Pause);
                            crate::emu_log!("[DEBUG] UI: Pause requested");
                        } else {
                            self.auto_running = true;
                            let _ = tx.send(DebugCommand::Run);
                            crate::emu_log!("[DEBUG] UI: Run requested");
                        }
                    }
                    for window in self.windows.values() {
                        window.request_redraw();
                    }
                }
                if PhysicalKey::Code(KeyCode::Escape) == event.physical_key {
                    event_loop.exit();
                }

                // Log Scrolling
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::PageUp) => {
                        self.log_scroll_offset = (self.log_scroll_offset + 10).min(980);
                        if let Some(w) = self.windows.get(&id) {
                            w.request_redraw();
                        }
                    }
                    PhysicalKey::Code(KeyCode::PageDown) => {
                        self.log_scroll_offset = self.log_scroll_offset.saturating_sub(10);
                        if let Some(w) = self.windows.get(&id) {
                            w.request_redraw();
                        }
                    }
                    PhysicalKey::Code(KeyCode::Home) => {
                        self.log_scroll_offset = 980;
                        if let Some(w) = self.windows.get(&id) {
                            w.request_redraw();
                        }
                    }
                    PhysicalKey::Code(KeyCode::End) => {
                        self.log_scroll_offset = 0;
                        if let Some(w) = self.windows.get(&id) {
                            w.request_redraw();
                        }
                    }
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                use winit::event::MouseScrollDelta;
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as i32,
                    MouseScrollDelta::PixelDelta(pos) => (pos.y / 12.0) as i32,
                };
                if lines > 0 {
                    self.log_scroll_offset = (self.log_scroll_offset + lines as usize).min(980);
                } else {
                    self.log_scroll_offset =
                        self.log_scroll_offset.saturating_sub((-lines) as usize);
                }
                if let Some(id) = self.debug_window_id {
                    if let Some(window) = self.windows.get(&id) {
                        window.request_redraw();
                    }
                }
            }
            WindowEvent::CloseRequested => {
                crate::emu_log!("Window {:#?} close requested", id);
                if Some(id) == self.debug_window_id {
                    event_loop.exit();
                } else {
                    self.windows.remove(&id);
                    if let Some(hwnd) = self.id_to_hwnd.remove(&id) {
                        self.hwnd_to_id.remove(&hwnd);
                    }
                }
            }
            _ => (),
        }
    }
}

/// `winit` 윈도우 기반 이벤트 루프를 생성하고 폴링(Polling) 방식으로 동작시켜
/// 메인 스레드 내에서 디버거 UI를 블로킹하며 띄우는 함수. 프로그램 종료 시까지 반환하지 않음
pub fn create_debug_window(
    cmd_tx: Sender<DebugCommand>,
    state_rx: Receiver<CpuContext>,
    ui_rx: Receiver<UiCommand>,
) {
    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = Debug::new(cmd_tx, state_rx, ui_rx);
    event_loop.run_app(&mut app).unwrap();
}
