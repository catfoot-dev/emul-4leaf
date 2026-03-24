use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::mpsc::Receiver;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::raw_window_handle::{DisplayHandle, HasDisplayHandle};
use winit::window::{Window, WindowId};

use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use softbuffer::{Context as SoftContext, Surface};

use crate::ui::{Painter, UiCommand};

// Windows 스타일 -> winit 속성 매핑
const WS_POPUP: u32 = 0x80000000;
const WS_CAPTION: u32 = 0x00C00000;
const WS_THICKFRAME: u32 = 0x00040000; // WS_SIZEBOX
const WS_MINIMIZEBOX: u32 = 0x00020000;
const WS_MAXIMIZEBOX: u32 = 0x00010000;
const WS_EX_TOPMOST: u32 = 0x00000008;

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
    /// gdi_objects 가상 메모리 맵
    pub gdi_objects: std::sync::Arc<std::sync::Mutex<HashMap<u32, crate::win32::GdiObject>>>,

    /// 초기 페인터 목록 (resumed에서 창 생성 후 painters로 이동)
    initial_painters: Vec<Box<dyn Painter>>,
}

impl WinFrame {
    pub fn new(
        ui_rx: Receiver<UiCommand>,
        initial_painters: Vec<Box<dyn Painter>>,
        gdi_objects: std::sync::Arc<std::sync::Mutex<HashMap<u32, crate::win32::GdiObject>>>,
    ) -> Self {
        Self {
            ui_rx,
            windows: HashMap::new(),
            painters: HashMap::new(),
            hwnd_to_id: HashMap::new(),
            id_to_hwnd: HashMap::new(),
            context: None,
            gdi_objects,
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
                    style,
                    ex_style,
                } => {
                    let mut attributes = Window::default_attributes()
                        .with_title(title)
                        .with_inner_size(winit::dpi::LogicalSize::new(width, height));

                    // 테두리 및 캡션 제어
                    if (style & WS_POPUP) != 0 {
                        attributes = attributes.with_decorations(false);
                    } else if (style & WS_CAPTION) == 0 {
                        attributes = attributes.with_decorations(false);
                    }

                    // 크기 조절 가능 여부
                    if (style & WS_THICKFRAME) != 0 {
                        attributes = attributes.with_resizable(true);
                    } else {
                        attributes = attributes.with_resizable(false);
                    }

                    // 최소/최대화 버튼 (winit에서는 개별 제어가 제한적일 수 있음)
                    let _has_min = (style & WS_MINIMIZEBOX) != 0;
                    let _has_max = (style & WS_MAXIMIZEBOX) != 0;

                    // 항상 위에 표시
                    if (ex_style & WS_EX_TOPMOST) != 0 {
                        attributes =
                            attributes.with_window_level(winit::window::WindowLevel::AlwaysOnTop);
                    }

                    let window = event_loop.create_window(attributes).unwrap();
                    let id = window.id();
                    self.hwnd_to_id.insert(hwnd, id);
                    self.id_to_hwnd.insert(id, hwnd);

                    // 에뮬레이션 윈도우용 기본 Painter
                    let painter = DefaultEmulatorPainter {
                        hwnd,
                        gdi_objects: self.gdi_objects.clone(),
                    };
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

                UiCommand::ActivateWindow { hwnd } => {
                    if let Some(id) = self.hwnd_to_id.get(&hwnd) {
                        if let Some(window) = self.windows.get(id) {
                            window.focus_window();
                        }
                    }
                }

                UiCommand::MessageBox {
                    caption,
                    text,
                    u_type,
                    response_tx,
                } => {
                    let mut dialog = MessageDialog::new()
                        .set_title(&caption)
                        .set_description(&text);

                    // 아이콘 설정 (MB_ICON*)
                    if (u_type & 0x10) != 0 {
                        dialog = dialog.set_level(MessageLevel::Error);
                    } else if (u_type & 0x30) == 0x30 {
                        dialog = dialog.set_level(MessageLevel::Warning);
                    } else if (u_type & 0x40) != 0 {
                        dialog = dialog.set_level(MessageLevel::Info);
                    }

                    // 버튼 설정 (MB_*)
                    let buttons = match u_type & 0xF {
                        1 => MessageButtons::OkCancel,
                        3 => MessageButtons::YesNoCancel,
                        4 => MessageButtons::YesNo,
                        _ => MessageButtons::Ok,
                    };
                    dialog = dialog.set_buttons(buttons);

                    let result = dialog.show();

                    // 결과 매핑 (IDOK = 1, IDCANCEL = 2, IDYES = 6, IDNO = 7)
                    let win_result = match result {
                        MessageDialogResult::Ok => 1,
                        MessageDialogResult::Cancel => 2,
                        MessageDialogResult::Yes => 6,
                        MessageDialogResult::No => 7,
                        _ => 1,
                    };
                    let _ = response_tx.send(win_result);
                }
            }
        }

        // 모든 Painter에게 백그라운드 상태 변경 알림 및 종료 체크
        let mut windows_to_remove = Vec::new();
        for (id, painter) in self.painters.iter_mut() {
            if painter.tick() {
                if let Some(window) = self.windows.get(id) {
                    window.request_redraw();
                }
            }
            if painter.should_close() {
                windows_to_remove.push(*id);
            }
        }

        for id in windows_to_remove {
            self.windows.remove(&id);
            self.painters.remove(&id);
            if let Some(hwnd) = self.id_to_hwnd.remove(&id) {
                self.hwnd_to_id.remove(&hwnd);
            }
        }

        if needs_redraw {
            for window in self.windows.values() {
                window.request_redraw();
            }
        }

        // 10ms 마다 다시 깨어나서 리시버를 확인하도록 설정 (ControlFlow::Wait면 이벤트를 기다림)
        event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(
            std::time::Instant::now() + std::time::Duration::from_millis(10),
        ));
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
    gdi_objects: std::sync::Arc<std::sync::Mutex<HashMap<u32, crate::win32::GdiObject>>>,
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

    fn paint(&mut self, buffer: &mut [u32], width: u32, height: u32) {
        let gdi_objects = self.gdi_objects.lock().unwrap();

        // 1. HWND에 해당하는 DC 찾기 (가장 최근에 생성된 DC 등)
        // 사실 USER32 상의 윈도우 surface_bitmap을 찾는게 더 정확함.
        // 하지만 WinFrame에는 WindowState가 없음.
        // 대신 gdi_objects에서 이 hwnd를 associated_window로 가지는 DC를 찾거나,
        // (더 좋은 방법) GdiObject::Dc에 surface_bitmap이 이미 들어있음.

        // 여기서는 간단히 gdi_objects를 순회하여 이 hwnd를 가진 DC의 selected_bitmap을 찾음.
        let mut surface_pixels = None;

        for obj in gdi_objects.values() {
            if let crate::win32::GdiObject::Dc {
                associated_window,
                selected_bitmap,
                ..
            } = obj
            {
                if *associated_window == self.hwnd && *selected_bitmap != 0 {
                    if let Some(crate::win32::GdiObject::Bitmap {
                        pixels: p,
                        width: sw,
                        height: sh,
                    }) = gdi_objects.get(selected_bitmap)
                    {
                        surface_pixels = Some((p.clone(), *sw, *sh));
                        break;
                    }
                }
            }
        }

        if let Some((p, sw, sh)) = surface_pixels {
            let p = p.lock().unwrap();
            let copy_w = width.min(sw);
            let copy_h = height.min(sh);

            for y in 0..copy_h {
                for x in 0..copy_w {
                    buffer[(y * width + x) as usize] = p[(y * sw + x) as usize];
                }
            }
        }
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
