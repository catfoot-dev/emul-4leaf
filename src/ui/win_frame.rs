use crate::{
    dll::win32::{GdiObject, Win32Context},
    ui::{Painter, UiCommand},
};
use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use softbuffer::{Context as SoftContext, Surface};
use std::{collections::HashMap, num::NonZeroU32, sync::mpsc::Receiver};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, KeyEvent, MouseButton, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{Key, NamedKey},
    raw_window_handle::{DisplayHandle, HasDisplayHandle},
    window::{Window, WindowId},
};

// Windows 스타일 -> winit 속성 매핑
const WS_POPUP: u32 = 0x80000000;
const WS_CHILD: u32 = 0x40000000;
const WS_CAPTION: u32 = 0x00C00000;
const WS_THICKFRAME: u32 = 0x00040000; // WS_SIZEBOX
const WS_MINIMIZEBOX: u32 = 0x00020000;
const WS_MAXIMIZEBOX: u32 = 0x00010000;
const WS_EX_TOPMOST: u32 = 0x00000008;

type WinSurface = Surface<DisplayHandle<'static>, Window>;

/// 윈도우 애플리케이션 핸들러
/// 모든 winit 윈도우와 Painter를 관리함
pub struct WinFrame {
    ui_rx: Receiver<UiCommand>,

    /// 윈도우 ID -> softbuffer Surface (내부에 Window를 소유)
    surfaces: HashMap<WindowId, WinSurface>,
    /// 윈도우 ID -> Painter (그리기 로직)
    painters: HashMap<WindowId, Box<dyn Painter>>,
    /// 가상 HWND -> 윈도우 ID
    hwnd_to_id: HashMap<u32, WindowId>,
    /// 윈도우 ID -> 가상 HWND
    id_to_hwnd: HashMap<WindowId, u32>,

    /// softbuffer 컨텍스트
    sb_context: Option<SoftContext<DisplayHandle<'static>>>,
    /// Win32 컨텍스트 (공유 상태)
    pub emu_context: Win32Context,

    /// 초기 페인터 목록 (resumed에서 창 생성 후 painters로 이동)
    initial_painters: Vec<Box<dyn Painter>>,
}

impl WinFrame {
    pub fn new(
        ui_rx: Receiver<UiCommand>,
        initial_painters: Vec<Box<dyn Painter>>,
        context: Win32Context,
    ) -> Self {
        Self {
            ui_rx,
            surfaces: HashMap::new(),
            painters: HashMap::new(),
            hwnd_to_id: HashMap::new(),
            id_to_hwnd: HashMap::new(),
            sb_context: None,
            emu_context: context,
            initial_painters,
        }
    }

    fn get_window(&self, id: &WindowId) -> Option<&Window> {
        self.surfaces.get(id).map(|s| s.window())
    }

    pub fn get_painter_mut<T: Painter + 'static>(
        &mut self,
        id: winit::window::WindowId,
    ) -> Option<&mut T> {
        self.painters
            .get_mut(&id)
            .and_then(|p| p.as_any_mut().downcast_mut::<T>())
    }

    fn process_ui_commands(&mut self, event_loop: &ActiveEventLoop) -> bool {
        let mut needs_redraw = false;

        while let Ok(cmd) = self.ui_rx.try_recv() {
            match cmd {
                UiCommand::CreateWindow {
                    hwnd,
                    title,
                    width,
                    height,
                    style,
                    ex_style,
                    parent,
                    visible,
                    surface_bitmap,
                } => {
                    // 자식 윈도우는 별도 호스트 OS 창으로 만들지 않고 guest 상태만 유지합니다.
                    // 그렇지 않으면 컨트롤/헬퍼 창마다 예기치 않은 활성화/크기 이벤트가 발생합니다.
                    if (style & WS_CHILD) != 0 {
                        let _ = parent;
                        continue;
                    }

                    let mut attributes = Window::default_attributes()
                        .with_title(title)
                        .with_inner_size(winit::dpi::LogicalSize::new(width, height))
                        .with_visible(visible);

                    if (style & WS_POPUP) != 0 || (style & WS_CAPTION) == 0 {
                        attributes = attributes.with_decorations(false);
                    }

                    attributes = if (style & WS_THICKFRAME) != 0 {
                        attributes.with_resizable(true)
                    } else {
                        attributes.with_resizable(false)
                    };

                    let _has_min = (style & WS_MINIMIZEBOX) != 0;
                    let _has_max = (style & WS_MAXIMIZEBOX) != 0;

                    if (ex_style & WS_EX_TOPMOST) != 0 {
                        attributes =
                            attributes.with_window_level(winit::window::WindowLevel::AlwaysOnTop);
                    }

                    let window = event_loop.create_window(attributes).unwrap();
                    let id = window.id();
                    self.hwnd_to_id.insert(hwnd, id);
                    self.id_to_hwnd.insert(id, hwnd);

                    let painter = DefaultEmulatorPainter {
                        surface_bitmap,
                        emu_context: self.emu_context.clone(),
                    };
                    self.painters.insert(id, Box::new(painter));
                    let context = self
                        .sb_context
                        .as_ref()
                        .expect("Context should be initialized");
                    let surface = Surface::new(context, window).unwrap();
                    self.surfaces.insert(id, surface);
                    needs_redraw = true;
                }

                UiCommand::DestroyWindow { hwnd } => {
                    if let Some(id) = self.hwnd_to_id.remove(&hwnd) {
                        self.id_to_hwnd.remove(&id);
                        self.painters.remove(&id);
                        self.surfaces.remove(&id);
                    }
                }

                UiCommand::ShowWindow { hwnd, visible } => {
                    if let Some(id) = self.hwnd_to_id.get(&hwnd)
                        && let Some(window) = self.get_window(id)
                    {
                        window.set_visible(visible);
                    }
                }

                UiCommand::MoveWindow {
                    hwnd,
                    x,
                    y,
                    width,
                    height,
                } => {
                    if let Some(id) = self.hwnd_to_id.get(&hwnd)
                        && let Some(window) = self.get_window(id)
                    {
                        window.set_outer_position(winit::dpi::PhysicalPosition::new(x, y));
                        let _ =
                            window.request_inner_size(winit::dpi::LogicalSize::new(width, height));
                    }
                }

                UiCommand::SetWindowText { hwnd, text } => {
                    if let Some(id) = self.hwnd_to_id.get(&hwnd)
                        && let Some(window) = self.get_window(id)
                    {
                        window.set_title(&text);
                    }
                }

                UiCommand::UpdateWindow { hwnd } => {
                    if let Some(id) = self.hwnd_to_id.get(&hwnd)
                        && let Some(window) = self.get_window(id)
                    {
                        window.request_redraw();
                    }
                }

                UiCommand::ActivateWindow { hwnd } => {
                    if let Some(id) = self.hwnd_to_id.get(&hwnd)
                        && let Some(window) = self.get_window(id)
                    {
                        window.focus_window();
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

                    if (u_type & 0x10) != 0 {
                        dialog = dialog.set_level(MessageLevel::Error);
                    } else if (u_type & 0x30) == 0x30 {
                        dialog = dialog.set_level(MessageLevel::Warning);
                    } else if (u_type & 0x40) != 0 {
                        dialog = dialog.set_level(MessageLevel::Info);
                    }

                    let buttons = match u_type & 0xF {
                        1 => MessageButtons::OkCancel,
                        3 => MessageButtons::YesNoCancel,
                        4 => MessageButtons::YesNo,
                        _ => MessageButtons::Ok,
                    };
                    dialog = dialog.set_buttons(buttons);

                    let result = dialog.show();
                    let win_result = match result {
                        MessageDialogResult::Ok => 1,
                        MessageDialogResult::Cancel => 2,
                        MessageDialogResult::Yes => 6,
                        MessageDialogResult::No => 7,
                        _ => 1,
                    };
                    let _ = response_tx.send(win_result);
                }

                UiCommand::SetCursor { hwnd, hcursor } => {
                    if let Some(id) = self.hwnd_to_id.get(&hwnd)
                        && let Some(window) = self.get_window(id)
                    {
                        let mut cursor_applied = false;
                        {
                            let gdi_objects = self.emu_context.gdi_objects.lock().unwrap();
                            if let Some(GdiObject::Cursor {
                                resource_id,
                                frames,
                                ..
                            }) = gdi_objects.get(&hcursor)
                            {
                                if let Some(frame) = frames.first()
                                    && !frame.pixels.is_empty()
                                {
                                    let mut rgba = Vec::with_capacity(frame.pixels.len() * 4);
                                    for p in &frame.pixels {
                                        let a = (p >> 24) as u8;
                                        let r = (p >> 16) as u8;
                                        let g = (p >> 8) as u8;
                                        let b = *p as u8;
                                        rgba.push(r);
                                        rgba.push(g);
                                        rgba.push(b);
                                        rgba.push(a);
                                    }

                                    let source = winit::window::CustomCursor::from_rgba(
                                        rgba,
                                        frame.width as u16,
                                        frame.height as u16,
                                        frame.hotspot_x as u16,
                                        frame.hotspot_y as u16,
                                    );
                                    if let Ok(source) = source {
                                        let custom_cursor = event_loop.create_custom_cursor(source);
                                        window.set_cursor(custom_cursor);
                                        cursor_applied = true;
                                    }
                                }

                                if !cursor_applied {
                                    let icon = match *resource_id {
                                        32512 => winit::window::CursorIcon::Default,
                                        32513 => winit::window::CursorIcon::Text,
                                        32514 => winit::window::CursorIcon::Wait,
                                        32515 => winit::window::CursorIcon::Crosshair,
                                        32516 => winit::window::CursorIcon::NResize,
                                        32642 => winit::window::CursorIcon::NwseResize,
                                        32643 => winit::window::CursorIcon::NeswResize,
                                        32644 => winit::window::CursorIcon::EwResize,
                                        32645 => winit::window::CursorIcon::NsResize,
                                        32646 => winit::window::CursorIcon::Move,
                                        32648 => winit::window::CursorIcon::NotAllowed,
                                        32649 => winit::window::CursorIcon::Pointer,
                                        32650 => winit::window::CursorIcon::Progress,
                                        32651 => winit::window::CursorIcon::Help,
                                        _ => winit::window::CursorIcon::Default,
                                    };
                                    window.set_cursor(icon);
                                    cursor_applied = true;
                                }
                            }
                        }
                        if !cursor_applied {
                            window.set_cursor(winit::window::CursorIcon::Default);
                        }
                    }
                }
            }
        }

        needs_redraw
    }

    fn next_poll_interval(&self) -> Option<std::time::Duration> {
        self.painters
            .values()
            .filter_map(|painter| painter.poll_interval())
            .min()
    }
}

impl ApplicationHandler<()> for WinFrame {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.sb_context.is_none() {
            let display_handle = unsafe {
                std::mem::transmute::<DisplayHandle<'_>, DisplayHandle<'static>>(
                    event_loop.display_handle().unwrap(),
                )
            };
            self.sb_context = Some(SoftContext::new(display_handle).unwrap());
        }

        // 초기 페인터들을 위한 창 생성
        let mut initial_painters = std::mem::take(&mut self.initial_painters);
        for painter in initial_painters.drain(..) {
            let window = painter.create_window(event_loop);
            let id = window.id();
            self.painters.insert(id, painter);
            let context = self
                .sb_context
                .as_ref()
                .expect("Context should be initialized");
            let surface = Surface::new(context, window).unwrap();
            self.surfaces.insert(id, surface);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let needs_redraw = self.process_ui_commands(event_loop);

        // 모든 Painter에게 백그라운드 상태 변경 알림 및 종료 체크
        let mut windows_to_remove = Vec::new();
        let mut windows_to_redraw = Vec::new();
        for (id, painter) in self.painters.iter_mut() {
            if painter.tick() {
                windows_to_redraw.push(*id);
            }
            if painter.should_close() {
                windows_to_remove.push(*id);
            }
        }
        for id in windows_to_redraw {
            if let Some(window) = self.get_window(&id) {
                window.request_redraw();
            }
        }

        for id in windows_to_remove {
            self.surfaces.remove(&id);
            self.painters.remove(&id);
            if let Some(hwnd) = self.id_to_hwnd.remove(&id) {
                self.hwnd_to_id.remove(&hwnd);
            }
        }

        if needs_redraw {
            for window in self.surfaces.values().map(|s| s.window()) {
                window.request_redraw();
            }
        }

        if let Some(interval) = self.next_poll_interval() {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(
                std::time::Instant::now() + interval,
            ));
        } else {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if !self.surfaces.contains_key(&id) {
            return;
        }

        // 윈도우별 자체 이벤트 처리 위임 및 필요한 정보 추출
        let (handled, quit_on_close) = if let Some(painter) = self.painters.get_mut(&id) {
            (
                painter.handle_event(&event, event_loop),
                painter.quit_on_close(),
            )
        } else {
            (false, false)
        };

        if handled {
            if let Some(window) = self.get_window(&id) {
                window.request_redraw();
            }
        }

        match event {
            WindowEvent::RedrawRequested => {
                // 1. 게스트에게 그리기 요청 (더티 플래그 설정 및 메시지 큐 삽입)
                if let Some(&hwnd) = self.id_to_hwnd.get(&id) {
                    let mut win_event = self.emu_context.win_event.lock().unwrap();
                    if let Some(state) = win_event.windows.get_mut(&hwnd) {
                        state.needs_paint = true;
                    }

                    let mut q = self.emu_context.message_queue.lock().unwrap();
                    // 이미 WM_PAINT가 큐에 있는지 확인하고 없으면 삽입 (메시지 중복 방지)
                    if !q.iter().any(|m| m[0] == hwnd && m[1] == 0x000F) {
                        let time = self.emu_context.start_time.elapsed().as_millis() as u32;
                        q.push_back([hwnd, 0x000F, 0, 0, time, 0, 0]); // WM_PAINT
                    }
                }

                // 2. 호스트 화면에 현재 버퍼 출력 (캐시된 Surface 사용)
                if let Some(surface) = self.surfaces.get_mut(&id) {
                    let (width, height) = {
                        let size = surface.window().inner_size();
                        (size.width, size.height)
                    };

                    if let (Some(nw), Some(nh)) = (NonZeroU32::new(width), NonZeroU32::new(height))
                    {
                        surface.resize(nw, nh).unwrap();

                        let mut buffer = surface.buffer_mut().unwrap();
                        buffer.fill(0);

                        if let Some(painter) = self.painters.get_mut(&id) {
                            painter.paint(&mut buffer, width, height);
                        }

                        buffer.present().unwrap();
                    }
                }
            }

            WindowEvent::CloseRequested => {
                if let Some(&hwnd) = self.id_to_hwnd.get(&id) {
                    let mut q = self.emu_context.message_queue.lock().unwrap();
                    q.push_back([hwnd, 0x0010, 0, 0, 0, 0, 0]); // WM_CLOSE
                }

                if quit_on_close {
                    event_loop.exit();
                } else {
                    self.surfaces.remove(&id);
                    self.painters.remove(&id);
                    if let Some(hwnd) = self.id_to_hwnd.remove(&id) {
                        self.hwnd_to_id.remove(&hwnd);

                        let mut q = self.emu_context.message_queue.lock().unwrap();
                        q.push_back([hwnd, 0x0002, 0, 0, 0, 0, 0]); // WM_DESTROY

                        // 메인 윈도우라면 WM_QUIT 전송
                        if quit_on_close {
                            q.push_back([0, 0x0012, 0, 0, 0, 0, 0]); // WM_QUIT
                        }
                    }
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                let x = position.x as u32;
                let y = position.y as u32;

                if let Some(&hwnd) = self.id_to_hwnd.get(&id) {
                    self.emu_context
                        .mouse_x
                        .store(x, std::sync::atomic::Ordering::SeqCst);
                    self.emu_context
                        .mouse_y
                        .store(y, std::sync::atomic::Ordering::SeqCst);

                    let lparam = (y << 16) | (x & 0xFFFF);
                    let mut q = self.emu_context.message_queue.lock().unwrap();
                    // WM_MOUSEMOVE(0x0200) 중복 제거: 이미 큐에 있으면 위치만 업데이트
                    if let Some(m) = q.iter_mut().find(|m| m[0] == hwnd && m[1] == 0x0200) {
                        m[3] = lparam;
                        m[5] = x;
                        m[6] = y;
                    } else {
                        q.push_back([hwnd, 0x0200, 0, lparam, 0, x, y]);
                    }

                    // 마우스 트래킹 (TrackMouseEvent) 처리
                    let mut track_opt = self.emu_context.track_mouse_event.lock().unwrap();
                    if let Some(track) = track_opt.clone() {
                        if track.hwnd != hwnd && (track.flags & 0x00000002 != 0) {
                            let time = self.emu_context.start_time.elapsed().as_millis() as u32;
                            q.push_back([track.hwnd, 0x02A3, 0, 0, time, 0, 0]); // WM_MOUSELEAVE
                            *track_opt = None;
                        }
                    }
                }
            }

            WindowEvent::CursorLeft { .. } => {
                if let Some(&hwnd) = self.id_to_hwnd.get(&id) {
                    let mut track_opt = self.emu_context.track_mouse_event.lock().unwrap();
                    if let Some(track) = track_opt.clone() {
                        if track.hwnd == hwnd && (track.flags & 0x00000002 != 0) {
                            let time = self.emu_context.start_time.elapsed().as_millis() as u32;
                            let mut q = self.emu_context.message_queue.lock().unwrap();
                            q.push_back([track.hwnd, 0x02A3, 0, 0, time, 0, 0]); // WM_MOUSELEAVE
                            *track_opt = None;
                        }
                    }
                }
            }

            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(&hwnd) = self.id_to_hwnd.get(&id) {
                    let x = self
                        .emu_context
                        .mouse_x
                        .load(std::sync::atomic::Ordering::SeqCst);
                    let y = self
                        .emu_context
                        .mouse_y
                        .load(std::sync::atomic::Ordering::SeqCst);
                    let lparam = (y << 16) | (x & 0xFFFF);

                    let msg = match (button, state) {
                        (MouseButton::Left, ElementState::Pressed) => 0x0201, // WM_LBUTTONDOWN
                        (MouseButton::Left, ElementState::Released) => 0x0202, // WM_LBUTTONUP
                        (MouseButton::Right, ElementState::Pressed) => 0x0204, // WM_RBUTTONDOWN
                        (MouseButton::Right, ElementState::Released) => 0x0205, // WM_RBUTTONUP
                        _ => 0,
                    };

                    if msg != 0 {
                        let mut q = self.emu_context.message_queue.lock().unwrap();
                        q.push_back([hwnd, msg, 0, lparam, 0, x, y]);
                    }
                }
            }

            WindowEvent::KeyboardInput {
                event: KeyEvent {
                    logical_key, state, ..
                },
                ..
            } => {
                if let Some(&hwnd) = self.id_to_hwnd.get(&id) {
                    let vk = match logical_key {
                        Key::Named(NamedKey::ArrowLeft) => 0x25,
                        Key::Named(NamedKey::ArrowUp) => 0x26,
                        Key::Named(NamedKey::ArrowRight) => 0x27,
                        Key::Named(NamedKey::ArrowDown) => 0x28,
                        Key::Named(NamedKey::Enter) => 0x0D,
                        Key::Named(NamedKey::Escape) => 0x1B,
                        Key::Named(NamedKey::Space) => 0x20,
                        Key::Named(NamedKey::Backspace) => 0x08,
                        Key::Named(NamedKey::Tab) => 0x09,
                        Key::Named(NamedKey::Shift) => 0x10,
                        Key::Named(NamedKey::Control) => 0x11,
                        Key::Named(NamedKey::Alt) => 0x12,
                        Key::Character(s) => s.chars().next().unwrap_or('\0') as u32,
                        _ => 0,
                    };

                    if vk != 0 {
                        {
                            let mut keys = self.emu_context.key_states.lock().unwrap();
                            if vk < 256 {
                                keys[vk as usize] = state == ElementState::Pressed;
                            }
                        }

                        let msg = if state == ElementState::Pressed {
                            0x0100 // WM_KEYDOWN
                        } else {
                            0x0101 // WM_KEYUP
                        };
                        let mut q = self.emu_context.message_queue.lock().unwrap();
                        q.push_back([hwnd, msg, vk, 0, 0, 0, 0]);
                    }
                }
            }

            WindowEvent::Focused(focused) => {
                if let Some(&hwnd) = self.id_to_hwnd.get(&id) {
                    let mut q = self.emu_context.message_queue.lock().unwrap();
                    let wparam = if focused { 1 } else { 0 }; // WA_ACTIVE, WA_INACTIVE
                    q.push_back([hwnd, 0x0006, wparam, 0, 0, 0, 0]); // WM_ACTIVATE

                    if focused {
                        self.emu_context
                            .focus_hwnd
                            .store(hwnd, std::sync::atomic::Ordering::SeqCst);
                        self.emu_context
                            .active_hwnd
                            .store(hwnd, std::sync::atomic::Ordering::SeqCst);
                    }
                }
            }

            WindowEvent::Resized(size) => {
                if let Some(&hwnd) = self.id_to_hwnd.get(&id) {
                    let width = size.width;
                    let height = size.height;
                    let lparam = (height << 16) | (width & 0xFFFF);

                    let mut q = self.emu_context.message_queue.lock().unwrap();
                    q.push_back([hwnd, 0x0005, 0, lparam, 0, 0, 0]); // WM_SIZE (SIZE_RESTORED)

                    self.emu_context
                        .win_event
                        .lock()
                        .unwrap()
                        .resize_window(hwnd, width, height);
                }
            }

            _ => (),
        }
    }
}

/// 에뮬레이터 윈도우용 기본 페인터
pub struct DefaultEmulatorPainter {
    surface_bitmap: u32,
    emu_context: Win32Context,
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
        let gdi_objects = match self.emu_context.gdi_objects.try_lock() {
            Ok(g) => g,
            Err(_) => return, // 락 획득 실패 시 이번 프레임은 건너뜀 (데드락 방지)
        };

        let mut surface_pixels = None;

        if let Some(GdiObject::Bitmap {
            pixels: p,
            width: sw,
            height: sh,
            ..
        }) = gdi_objects.get(&self.surface_bitmap)
        {
            surface_pixels = Some((p.clone(), *sw, *sh));
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
