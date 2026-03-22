use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::mpsc::Receiver;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::raw_window_handle::{DisplayHandle, HasDisplayHandle};
use winit::window::{Window, WindowId};

use softbuffer::{Context as SoftContext, Surface};

use crate::ui::{Painter, UiCommand};

/// 윈도우 애플리케이션 핸들러
/// 모든 winit 윈도우와 Painter를 관리함
pub struct WinFrame {
    ui_rx: Receiver<UiCommand>,

    /// 윈도우 ID -> winit Window
    windows: HashMap<WindowId, Window>,
    /// 윈도우 ID -> Painter (그리기 로직)
    painters: HashMap<WindowId, Box<dyn Painter>>,
    /// 가상 HWND -> 윈도우 ID
    hwnd_to_id: HashMap<u32, WindowId>,
    /// 윈도우 ID -> 가상 HWND
    id_to_hwnd: HashMap<WindowId, u32>,

    /// softbuffer 컨텍스트
    context: Option<SoftContext<DisplayHandle<'static>>>,

    /// 초기 페인터 목록 (resumed에서 창 생성 후 painters로 이동)
    initial_painters: Vec<Box<dyn Painter>>,
}

impl WinFrame {
    pub fn new(ui_rx: Receiver<UiCommand>, initial_painters: Vec<Box<dyn Painter>>) -> Self {
        Self {
            ui_rx,
            windows: HashMap::new(),
            painters: HashMap::new(),
            hwnd_to_id: HashMap::new(),
            id_to_hwnd: HashMap::new(),
            context: None,
            initial_painters,
        }
    }
}

impl ApplicationHandler for WinFrame {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.context.is_none() {
            let display_handle = unsafe {
                std::mem::transmute::<DisplayHandle<'_>, DisplayHandle<'static>>(
                    event_loop.display_handle().unwrap(),
                )
            };
            self.context = Some(SoftContext::new(display_handle).unwrap());
        }

        // 초기 페인터들을 위한 창 생성
        let mut initial_painters = std::mem::take(&mut self.initial_painters);
        for painter in initial_painters.drain(..) {
            let window = painter.create_window(event_loop);
            let id = window.id();
            self.painters.insert(id, painter);
            self.windows.insert(id, window);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let mut needs_redraw = false;

        // UI 명령 처리 (윈도우 생성 등)
        while let Ok(cmd) = self.ui_rx.try_recv() {
            match cmd {
                UiCommand::CreateWindow {
                    hwnd,
                    title,
                    width,
                    height,
                } => {
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

                    // 에뮬레이션 윈도우용 기본 Painter (현재는 HWND 표시 수준)
                    let painter = DefaultEmulatorPainter { hwnd };
                    self.painters.insert(id, Box::new(painter));
                    self.windows.insert(id, window);
                    needs_redraw = true;
                }

                UiCommand::DestroyWindow { hwnd } => {
                    if let Some(id) = self.hwnd_to_id.remove(&hwnd) {
                        self.id_to_hwnd.remove(&id);
                        self.painters.remove(&id);
                        self.windows.remove(&id);
                    }
                }

                UiCommand::ShowWindow { hwnd, visible } => {
                    if let Some(id) = self.hwnd_to_id.get(&hwnd) {
                        if let Some(window) = self.windows.get(id) {
                            window.set_visible(visible);
                        }
                    }
                }

                UiCommand::MoveWindow {
                    hwnd,
                    x,
                    y,
                    width,
                    height,
                } => {
                    if let Some(id) = self.hwnd_to_id.get(&hwnd) {
                        if let Some(window) = self.windows.get(id) {
                            window.set_outer_position(winit::dpi::PhysicalPosition::new(x, y));
                            let _ = window
                                .request_inner_size(winit::dpi::LogicalSize::new(width, height));
                        }
                    }
                }

                UiCommand::SetWindowText { hwnd, text } => {
                    if let Some(id) = self.hwnd_to_id.get(&hwnd) {
                        if let Some(window) = self.windows.get(id) {
                            window.set_title(&text);
                        }
                    }
                }

                UiCommand::UpdateWindow { hwnd } => {
                    if let Some(id) = self.hwnd_to_id.get(&hwnd) {
                        if let Some(window) = self.windows.get(id) {
                            window.request_redraw();
                        }
                    }
                }
            }
        }

        // 모든 Painter에게 백그라운드 상태 변경 알림
        for (id, painter) in self.painters.iter_mut() {
            if painter.tick() {
                if let Some(window) = self.windows.get(id) {
                    window.request_redraw();
                }
            }
        }

        if needs_redraw {
            for window in self.windows.values() {
                window.request_redraw();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let window = match self.windows.get(&id) {
            Some(w) => w,
            None => return,
        };
        let painter = match self.painters.get_mut(&id) {
            Some(p) => p,
            None => return,
        };

        // 윈도우별 자체 이벤트 처리 위임
        if painter.handle_event(&event, event_loop) {
            window.request_redraw();
        }

        match event {
            WindowEvent::RedrawRequested => {
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
                buffer.fill(0);

                painter.paint(&mut buffer, width, height);

                buffer.present().unwrap();
            }

            WindowEvent::CloseRequested => {
                if painter.quit_on_close() {
                    event_loop.exit();
                } else {
                    self.windows.remove(&id);
                    self.painters.remove(&id);
                    if let Some(hwnd) = self.id_to_hwnd.remove(&id) {
                        self.hwnd_to_id.remove(&hwnd);
                    }
                }
            }

            _ => (),
        }
    }
}

/// 에뮬레이터 윈도우용 기본 페인터
struct DefaultEmulatorPainter {
    hwnd: u32,
}
impl Painter for DefaultEmulatorPainter {
    fn create_window(
        &self,
        _event_loop: &winit::event_loop::ActiveEventLoop,
    ) -> winit::window::Window {
        panic!("DefaultEmulatorPainter should not be used for initial windows yet");
    }

    fn quit_on_close(&self) -> bool {
        false
    }

    fn paint(&mut self, _buffer: &mut [u32], _width: u32, _height: u32) {
        // TODO: 실제 에뮬레이션의 그래픽 데이터를 그리는 로직 추가
    }

    fn handle_event(
        &mut self,
        _event: &winit::event::WindowEvent,
        _event_loop: &winit::event_loop::ActiveEventLoop,
    ) -> bool {
        false
    }

    fn tick(&mut self) -> bool {
        false
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// Painter 트레이트에 as_any_mut를 추가해야 함을 나중에 mod.rs에 반영해야겠군.
