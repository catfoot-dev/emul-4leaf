use crate::{
    dll::win32::{GdiObject, Win32Context},
    ui::{Painter, UiCommand, gdi_renderer::GdiRenderer},
};
use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use softbuffer::{Context as SoftContext, Surface};
use std::{collections::HashMap, num::NonZeroU32, sync::mpsc::Receiver};
#[cfg(target_os = "windows")]
use winit::platform::windows::{WindowAttributesExtWindows, WindowExtWindows};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, KeyEvent, MouseButton, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{Key, NamedKey},
    raw_window_handle::{DisplayHandle, HasDisplayHandle},
    window::{Window, WindowAttributes, WindowButtons, WindowId, WindowLevel},
};

// Windows 스타일 -> winit 속성 매핑
const WS_BORDER: u32 = 0x00800000;
const WS_CAPTION: u32 = 0x00C00000;
const WS_CHILD: u32 = 0x40000000;
const WS_CLIPSIBLINGS: u32 = 0x04000000;
const WS_DLGFRAME: u32 = 0x00400000;
const WS_GROUP: u32 = 0x00020000;
const WS_HSCROLL: u32 = 0x00100000;
const WS_SIZEBOX: u32 = 0x00040000;
const WS_SYSMENU: u32 = 0x00080000;
const WS_POPUP: u32 = 0x80000000;
const WS_THICKFRAME: u32 = 0x00040000; // WS_SIZEBOX
const WS_MINIMIZEBOX: u32 = 0x00020000;
const WS_MAXIMIZEBOX: u32 = 0x00010000;
const WS_EX_TOPMOST: u32 = 0x00000008;
const WS_EX_LAYERED: u32 = 0x00080000;
const CW_USEDEFAULT: i32 = i32::MIN;
#[cfg(target_os = "windows")]
const WS_EX_TOOLWINDOW: u32 = 0x00000080;
#[cfg(target_os = "windows")]
const WS_EX_APPWINDOW: u32 = 0x00040000;

type WinSurface = Surface<DisplayHandle<'static>, Window>;

#[derive(Clone, Copy)]
struct HostWindowStyle {
    decorations: bool,
    resizable: bool,
    transparent: bool,
    topmost: bool,
    #[cfg(target_os = "windows")]
    skip_taskbar: bool,
    enabled_buttons: WindowButtons,
}

impl HostWindowStyle {
    fn from_guest(style: u32, ex_style: u32, use_native_frame: bool) -> Self {
        let decorations = use_native_frame && (style & WS_POPUP) == 0 && (style & WS_CAPTION) != 0;
        let resizable = use_native_frame && (style & WS_THICKFRAME) != 0;
        let transparent = (ex_style & WS_EX_LAYERED) != 0;
        let topmost = (ex_style & WS_EX_TOPMOST) != 0;
        #[cfg(target_os = "windows")]
        let skip_taskbar = (ex_style & WS_EX_TOOLWINDOW) != 0 && (ex_style & WS_EX_APPWINDOW) == 0;

        let mut enabled_buttons = WindowButtons::empty();
        if decorations && (style & WS_SYSMENU) != 0 {
            enabled_buttons |= WindowButtons::CLOSE;
            if (style & WS_MINIMIZEBOX) != 0 {
                enabled_buttons |= WindowButtons::MINIMIZE;
            }
            if (style & WS_MAXIMIZEBOX) != 0 {
                enabled_buttons |= WindowButtons::MAXIMIZE;
            }
        }

        Self {
            decorations,
            resizable,
            transparent,
            topmost,
            #[cfg(target_os = "windows")]
            skip_taskbar,
            enabled_buttons,
        }
    }

    fn window_level(self) -> WindowLevel {
        if self.topmost {
            WindowLevel::AlwaysOnTop
        } else {
            WindowLevel::Normal
        }
    }
}

/// 커서 애니메이션의 현재 상태를 저장하는 구조체
struct CursorAnimState {
    /// 애니메이션 대상 커서 GDI 핸들
    hcursor: u32,
    /// 대상 윈도우 ID
    window_id: WindowId,
    /// 현재 표시 중인 프레임 인덱스
    frame_index: usize,
    /// 총 프레임 수
    frame_count: usize,
    /// 프레임 간 전환 간격
    interval: std::time::Duration,
    /// 마지막으로 프레임을 전환한 시각
    last_switch: std::time::Instant,
}

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
    /// 가상 HWND -> 호스트 네이티브 프레임 사용 여부
    hwnd_native_frame: HashMap<u32, bool>,

    /// softbuffer 컨텍스트
    sb_context: Option<SoftContext<DisplayHandle<'static>>>,
    /// Win32 컨텍스트 (공유 상태)
    pub emu_context: Win32Context,

    /// 초기 페인터 목록 (resumed에서 창 생성 후 painters로 이동)
    initial_painters: Vec<Box<dyn Painter>>,

    /// 현재 활성화된 커서 애니메이션 상태 (None이면 정적 커서)
    cursor_anim: Option<CursorAnimState>,
    /// 현재 적용된 커서 핸들과 윈도우 ID (중복 적용 방지용)
    current_cursor: Option<(WindowId, u32)>,
    /// 마지막으로 마우스 이벤트가 처리된 시간 (스로틀링용)
    last_cursor_moved: Option<std::time::Instant>,
    /// 마지막으로 게스트에게 전송된 마우스 좌표 (스로틀링 누락 감지용)
    last_sent_mouse_pos: Option<(u32, u32)>,
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
            hwnd_native_frame: HashMap::new(),
            sb_context: None,
            emu_context: context,
            initial_painters,
            cursor_anim: None,
            current_cursor: None,
            last_cursor_moved: None,
            last_sent_mouse_pos: None,
        }
    }

    fn get_window(&self, id: &WindowId) -> Option<&Window> {
        self.surfaces.get(id).map(|s| s.window())
    }

    /// 창을 제거하고 관련 상태(커서 애니메이션 포함)를 정리합니다.
    fn remove_window(&mut self, id: WindowId) {
        self.surfaces.remove(&id);
        self.painters.remove(&id);
        if let Some(hwnd) = self.id_to_hwnd.remove(&id) {
            self.hwnd_to_id.remove(&hwnd);
            self.hwnd_native_frame.remove(&hwnd);
        }

        // 창이 파괴되면 해당 창과 연결된 커서 상태도 정리합니다.
        if self.current_cursor.as_ref().map(|(cid, _)| *cid) == Some(id) {
            self.current_cursor = None;
        }
        if self.cursor_anim.as_ref().map(|a| a.window_id) == Some(id) {
            self.cursor_anim = None;
        }
    }

    /// 지정된 HWND와 그 자손에 해당하는 호스트 창을 모두 제거합니다.
    fn remove_window_tree_by_hwnd(&mut self, root_hwnd: u32) -> Vec<u32> {
        let subtree = {
            let win_event = self.emu_context.win_event.lock().unwrap();
            win_event.window_subtree_postorder(root_hwnd)
        };

        for hwnd in &subtree {
            if let Some(id) = self.hwnd_to_id.get(hwnd).copied() {
                self.remove_window(id);
            }
        }

        subtree
    }

    pub fn get_painter_mut<T: Painter + 'static>(
        &mut self,
        id: winit::window::WindowId,
    ) -> Option<&mut T> {
        self.painters
            .get_mut(&id)
            .and_then(|p| p.as_any_mut().downcast_mut::<T>())
    }

    /// 에뮬레이터 스레드를 즉시 깨워 새 메시지를 처리하도록 합니다.
    /// main_resume_time을 클리어하여 메인 스레드가 즉시 실행되게 합니다.
    fn wake_emulator(&self) {
        // 메인 스레드의 대기 시간을 해제하여 다음 루프에서 즉시 실행되도록 합니다.
        if let Ok(mut guard) = self.emu_context.main_resume_time.try_lock() {
            *guard = None;
        }
        if let Ok(guard) = self.emu_context.emu_thread.try_lock() {
            if let Some(thread) = guard.as_ref() {
                thread.unpark();
            }
        }
    }

    fn apply_guest_window_attributes(
        mut attributes: WindowAttributes,
        style: u32,
        ex_style: u32,
        use_native_frame: bool,
    ) -> WindowAttributes {
        let host_style = HostWindowStyle::from_guest(style, ex_style, use_native_frame);

        attributes = attributes
            .with_decorations(host_style.decorations)
            .with_resizable(host_style.resizable)
            .with_enabled_buttons(host_style.enabled_buttons)
            .with_transparent(host_style.transparent)
            .with_window_level(host_style.window_level());

        #[cfg(target_os = "windows")]
        {
            // 툴 윈도우/레이어드 윈도우 같은 Win32 확장 스타일을 가능한 범위에서 반영합니다.
            attributes = attributes
                .with_skip_taskbar(host_style.skip_taskbar)
                .with_undecorated_shadow(!host_style.decorations && !host_style.transparent);
        }

        attributes
    }

    fn apply_guest_window_style(
        window: &Window,
        style: u32,
        ex_style: u32,
        use_native_frame: bool,
    ) {
        let host_style = HostWindowStyle::from_guest(style, ex_style, use_native_frame);

        window.set_decorations(host_style.decorations);
        window.set_resizable(host_style.resizable);
        window.set_enabled_buttons(host_style.enabled_buttons);
        window.set_transparent(host_style.transparent);
        window.set_window_level(host_style.window_level());

        #[cfg(target_os = "windows")]
        {
            window.set_skip_taskbar(host_style.skip_taskbar);
            window.set_undecorated_shadow(!host_style.decorations && !host_style.transparent);
        }
    }

    fn process_ui_commands(&mut self, event_loop: &ActiveEventLoop) -> bool {
        let mut needs_redraw = false;

        while let Ok(cmd) = self.ui_rx.try_recv() {
            match cmd {
                UiCommand::CreateWindow {
                    hwnd,
                    title,
                    x,
                    y,
                    width,
                    height,
                    style,
                    ex_style,
                    parent,
                    visible,
                    use_native_frame,
                    surface_bitmap,
                } => {
                    // 자식 윈도우는 별도 호스트 OS 창으로 만들지 않고 guest 상태만 유지합니다.
                    // 그렇지 않으면 컨트롤/헬퍼 창마다 예기치 않은 활성화/크기 이벤트가 발생합니다.
                    if (style & WS_CHILD) != 0 {
                        crate::emu_log!(
                            "[UI] host create skipped for child HWND {:#x} parent={:#x} visible={} size={}x{} style={:#x}",
                            hwnd,
                            parent,
                            visible,
                            width,
                            height,
                            style
                        );
                        let _ = parent;
                        continue;
                    }

                    let mut attributes = Window::default_attributes()
                        .with_title(title)
                        // 게스트가 다루는 좌표계는 픽셀 기반이므로 backing store 크기와 1:1로 맞춥니다.
                        .with_inner_size(winit::dpi::PhysicalSize::new(width, height))
                        .with_visible(visible);

                    if x != CW_USEDEFAULT && y != CW_USEDEFAULT {
                        attributes =
                            attributes.with_position(winit::dpi::PhysicalPosition::new(x, y));
                    }

                    attributes = Self::apply_guest_window_attributes(
                        attributes,
                        style,
                        ex_style,
                        use_native_frame,
                    );

                    let window = event_loop.create_window(attributes).unwrap();
                    let id = window.id();
                    crate::emu_log!(
                        "[UI] host window created HWND {:#x} visible={} size={}x{} style={:#x} ex_style={:#x} parent={:#x}",
                        hwnd,
                        visible,
                        width,
                        height,
                        style,
                        ex_style,
                        parent
                    );
                    self.hwnd_to_id.insert(hwnd, id);
                    self.id_to_hwnd.insert(id, hwnd);
                    self.hwnd_native_frame.insert(hwnd, use_native_frame);

                    let painter = DefaultEmulatorPainter {
                        hwnd,
                        surface_bitmap,
                        emu_context: self.emu_context.clone(),
                        cached_windows: HashMap::new(),
                        cached_generation: u64::MAX, // 첫 paint에서 반드시 갱신되도록
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

                UiCommand::SyncWindowStyle {
                    hwnd,
                    style,
                    ex_style,
                } => {
                    if let Some(id) = self.hwnd_to_id.get(&hwnd)
                        && let Some(window) = self.get_window(id)
                    {
                        let use_native_frame =
                            self.hwnd_native_frame.get(&hwnd).copied().unwrap_or(true);

                        Self::apply_guest_window_style(window, style, ex_style, use_native_frame);
                        window.request_redraw();
                    }
                }

                UiCommand::DestroyWindow { hwnd } => {
                    if let Some(id) = self.hwnd_to_id.get(&hwnd).copied() {
                        self.remove_window(id);
                    }
                }

                UiCommand::ShowWindow { hwnd, visible } => {
                    if let Some(id) = self.hwnd_to_id.get(&hwnd)
                        && let Some(window) = self.get_window(id)
                    {
                        crate::emu_log!(
                            "[UI] host ShowWindow HWND {:#x} visible={}",
                            hwnd,
                            visible
                        );
                        window.set_visible(visible);
                        if visible {
                            window.request_redraw();
                        }
                    } else {
                        crate::emu_log!(
                            "[UI] host ShowWindow missed HWND {:#x} visible={} (no host window)",
                            hwnd,
                            visible
                        );
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
                            window.request_inner_size(winit::dpi::PhysicalSize::new(width, height));
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

                UiCommand::EnableWindow { hwnd, enabled } => {
                    if let Some(id) = self.hwnd_to_id.get(&hwnd)
                        && let Some(window) = self.get_window(id)
                    {
                        #[cfg(target_os = "windows")]
                        {
                            window.set_enable(enabled);
                        }

                        #[cfg(not(target_os = "windows"))]
                        let _ = (window, enabled);
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

                UiCommand::DragWindow { hwnd } => {
                    let window_id = self.hwnd_to_id.get(&hwnd).copied();
                    if let Some(id) = window_id {
                        if let Some(window) = self.get_window(&id) {
                            crate::emu_log!("[UI] DragWindow called for HWND {:#x}", hwnd);
                            let _ = window.drag_window();
                        }
                    }
                }

                UiCommand::SetWindowTransparent { hwnd, transparent } => {
                    if let Some(id) = self.hwnd_to_id.get(&hwnd)
                        && let Some(window) = self.get_window(id)
                    {
                        window.set_transparent(transparent);
                        window.request_redraw();
                    }
                }

                UiCommand::MinimizeWindow { hwnd } => {
                    if let Some(id) = self.hwnd_to_id.get(&hwnd)
                        && let Some(window) = self.get_window(id)
                    {
                        window.set_minimized(true);
                    }
                }

                UiCommand::MaximizeWindow { hwnd } => {
                    if let Some(id) = self.hwnd_to_id.get(&hwnd)
                        && let Some(window) = self.get_window(id)
                    {
                        window.set_maximized(true);
                    }
                }

                UiCommand::RestoreWindow { hwnd } => {
                    if let Some(id) = self.hwnd_to_id.get(&hwnd)
                        && let Some(window) = self.get_window(id)
                    {
                        window.set_minimized(false);
                        window.set_maximized(false);
                    }
                }

                UiCommand::SetCursor { hwnd, hcursor } => {
                    let window_id = self.hwnd_to_id.get(&hwnd).copied();
                    if let Some(id) = window_id {
                        // 이미 같은 커서가 같은 윈도우에 적용되어 있다면 무시합니다.
                        // 이를 통해 마우스 이동 시 발생하는 수많은 WM_SETCURSOR가 애니메이션을 리셋하는 것을 방지합니다.
                        if self.current_cursor == Some((id, hcursor)) {
                            continue;
                        }
                        self.current_cursor = Some((id, hcursor));

                        // GDI 오브젝트에서 커서 정보를 먼저 추출 (어떤 커서를 적용할지 결정)
                        enum CursorAction {
                            Animated {
                                cursor: winit::window::CustomCursor,
                                frame_count: usize,
                                interval: std::time::Duration,
                            },
                            Static(winit::window::CustomCursor),
                            SystemIcon(winit::window::CursorIcon),
                            Default,
                        }

                        let action = {
                            let gdi_objects = self.emu_context.gdi_objects.lock().unwrap();
                            if let Some(GdiObject::Cursor {
                                resource_id,
                                frames,
                                is_animated,
                                display_rate_jiffies,
                                ..
                            }) = gdi_objects.get(&hcursor)
                            {
                                if *is_animated && frames.len() > 1 {
                                    if let Some(custom) = Self::create_custom_cursor_from_frame(
                                        &frames[0], event_loop,
                                    ) {
                                        let rate = (*display_rate_jiffies / 2u32).max(1);
                                        let ms = (rate as u64) * 1000 / 60;
                                        CursorAction::Animated {
                                            cursor: custom,
                                            frame_count: frames.len(),
                                            interval: std::time::Duration::from_millis(ms),
                                        }
                                    } else {
                                        CursorAction::Default
                                    }
                                } else if let Some(frame) = frames.first()
                                    && !frame.pixels.is_empty()
                                {
                                    if let Some(custom) =
                                        Self::create_custom_cursor_from_frame(frame, event_loop)
                                    {
                                        CursorAction::Static(custom)
                                    } else {
                                        CursorAction::Default
                                    }
                                } else {
                                    // 시스템 커서 폴백 (resource_id 기반)
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
                                    CursorAction::SystemIcon(icon)
                                }
                            } else {
                                CursorAction::Default
                            }
                        };

                        // 결정된 커서를 윈도우에 적용하고 애니메이션 상태 갱신
                        if let Some(window) = self.get_window(&id) {
                            match &action {
                                CursorAction::Animated { cursor, .. } => {
                                    window.set_cursor(cursor.clone());
                                }
                                CursorAction::Static(cursor) => {
                                    window.set_cursor(cursor.clone());
                                }
                                CursorAction::SystemIcon(icon) => {
                                    window.set_cursor(*icon);
                                }
                                CursorAction::Default => {
                                    window.set_cursor(winit::window::CursorIcon::Default);
                                }
                            }
                        }

                        // window 참조 해제 후 cursor_anim 갱신
                        match action {
                            CursorAction::Animated {
                                frame_count,
                                interval,
                                ..
                            } => {
                                self.cursor_anim = Some(CursorAnimState {
                                    hcursor,
                                    window_id: id,
                                    frame_index: 0,
                                    frame_count,
                                    interval,
                                    last_switch: std::time::Instant::now(),
                                });
                            }
                            _ => {
                                self.cursor_anim = None;
                            }
                        }
                    }
                }
            }
        }

        needs_redraw
    }

    /// CursorFrame의 ARGB 픽셀을 RGBA로 변환하여 winit CustomCursor를 생성합니다.
    fn create_custom_cursor_from_frame(
        frame: &crate::dll::win32::CursorFrame,
        event_loop: &ActiveEventLoop,
    ) -> Option<winit::window::CustomCursor> {
        if frame.pixels.is_empty() {
            return None;
        }
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
        )
        .ok()?;
        Some(event_loop.create_custom_cursor(source))
    }

    /// 커서 애니메이션 타이머를 체크하고, 프레임 전환이 필요하면 실행합니다.
    fn tick_cursor_animation(&mut self, event_loop: &ActiveEventLoop) {
        let Some(anim) = self.cursor_anim.as_mut() else {
            return;
        };

        let now = std::time::Instant::now();
        if now.duration_since(anim.last_switch) < anim.interval {
            return;
        }

        // 다음 프레임으로 전환
        anim.frame_index = (anim.frame_index + 1) % anim.frame_count;
        anim.last_switch = now;

        let frame_index = anim.frame_index;
        let hcursor = anim.hcursor;
        let window_id = anim.window_id;

        // gdi_objects 락에서 해당 프레임 데이터를 읽어 커서 적용
        let frame_data = {
            let gdi_objects = self.emu_context.gdi_objects.lock().unwrap();
            if let Some(GdiObject::Cursor { frames, .. }) = gdi_objects.get(&hcursor) {
                frames.get(frame_index).cloned()
            } else {
                // 커서가 삭제되었으면 애니메이션 중단
                None
            }
        };

        if let Some(frame) = frame_data {
            if let Some(window) = self.get_window(&window_id) {
                if let Some(custom) = Self::create_custom_cursor_from_frame(&frame, event_loop) {
                    window.set_cursor(custom);
                }
            }
        } else {
            self.cursor_anim = None;
        }
    }

    fn next_poll_interval(&self) -> Option<std::time::Duration> {
        let painter_interval = self
            .painters
            .values()
            .filter_map(|painter| painter.poll_interval())
            .min();

        // 커서 애니메이션이 활성화되어 있으면 그 간격도 고려
        let cursor_interval = self.cursor_anim.as_ref().map(|anim| {
            let elapsed = anim.last_switch.elapsed();
            anim.interval.saturating_sub(elapsed)
        });

        match (painter_interval, cursor_interval) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
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

        // 커서 애니메이션 프레임 전환 체크
        self.tick_cursor_animation(event_loop);

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
            self.remove_window(id);
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
                // 호스트 redraw는 현재 guest 표면을 출력만 해야 합니다.
                // 여기서 다시 WM_PAINT를 만들면 EndPaint/UpdateWindow와 순환해 과도한 redraw가 발생합니다.
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
                            if painter.paint(&mut buffer, width, height) {
                                buffer.present().unwrap();
                            }
                        }
                    }
                }
            }

            WindowEvent::CloseRequested => {
                if let Some(&hwnd) = self.id_to_hwnd.get(&id) {
                    let mut q = self.emu_context.message_queue.lock().unwrap();
                    q.push_back([hwnd, 0x0010, 0, 0, 0, 0, 0]); // WM_CLOSE
                }
                self.wake_emulator();

                let hwnd = self.id_to_hwnd.get(&id).copied();

                if quit_on_close {
                    event_loop.exit();
                } else {
                    if let Some(hwnd) = hwnd {
                        let subtree = self.remove_window_tree_by_hwnd(hwnd);
                        let mut q = self.emu_context.message_queue.lock().unwrap();
                        for handle in subtree {
                            q.push_back([handle, 0x0002, 0, 0, 0, 0, 0]); // WM_DESTROY
                        }
                    }
                    self.wake_emulator();
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                let x = position.x as u32;
                let y = position.y as u32;

                if let Some(&top_hwnd) = self.id_to_hwnd.get(&id) {
                    self.emu_context
                        .mouse_x
                        .store(x, std::sync::atomic::Ordering::SeqCst);
                    self.emu_context
                        .mouse_y
                        .store(y, std::sync::atomic::Ordering::SeqCst);

                    let now = std::time::Instant::now();
                    if let Some(last) = self.last_cursor_moved {
                        if now.duration_since(last).as_millis() < 16 {
                            return; // 16ms 이내 스로틀링
                        }
                    }
                    self.last_cursor_moved = Some(now);

                    let (target_hwnd, target_x, target_y) = {
                        let win_event = self.emu_context.win_event.lock().unwrap();
                        let target =
                            win_event.child_window_from_point(top_hwnd, x as i32, y as i32);
                        if target != top_hwnd {
                            if let Some((origin_x, origin_y)) =
                                win_event.window_client_origin_in_host(target)
                            {
                                (
                                    target,
                                    (x as i32 - origin_x) as u32,
                                    (y as i32 - origin_y) as u32,
                                )
                            } else {
                                (top_hwnd, x, y)
                            }
                        } else {
                            (top_hwnd, x, y)
                        }
                    };

                    self.last_sent_mouse_pos = Some((x, y));

                    let time = self.emu_context.start_time.elapsed().as_millis() as u32;
                    let lparam = (target_y << 16) | (target_x & 0xFFFF);
                    let capture_hwnd = self
                        .emu_context
                        .capture_hwnd
                        .load(std::sync::atomic::Ordering::SeqCst);
                    let mut q = self.emu_context.message_queue.lock().unwrap();

                    // 단일 패스로 WM_SETCURSOR(0x0020)와 WM_MOUSEMOVE(0x0200) 인덱스를 동시에 탐색
                    let mut setcursor_idx = None;
                    let mut mousemove_idx = None;
                    for (i, m) in q.iter().enumerate() {
                        if m[0] == target_hwnd {
                            if m[1] == 0x0020 && setcursor_idx.is_none() {
                                setcursor_idx = Some(i);
                            } else if m[1] == 0x0200 && mousemove_idx.is_none() {
                                mousemove_idx = Some(i);
                            }
                            if setcursor_idx.is_some() && mousemove_idx.is_some() {
                                break;
                            }
                        }
                    }

                    if capture_hwnd == 0 {
                        let setcursor_lparam = (0x0200u32 << 16) | 0x0001; // WM_MOUSEMOVE | HTCLIENT
                        if let Some(idx) = setcursor_idx {
                            if let Some(message) = q.get_mut(idx) {
                                message[0] = target_hwnd;
                                message[2] = target_hwnd;
                                message[3] = setcursor_lparam;
                                message[4] = time;
                                message[5] = target_x;
                                message[6] = target_y;
                            }
                        } else {
                            let setcursor_message = [
                                target_hwnd,
                                0x0020,
                                target_hwnd,
                                setcursor_lparam,
                                time,
                                target_x,
                                target_y,
                            ];
                            if let Some(idx) = mousemove_idx {
                                q.insert(idx, setcursor_message);
                                // insert가 mousemove_idx를 1칸 밀었으므로 보정
                                mousemove_idx = mousemove_idx.map(|i| i + 1);
                            } else {
                                q.push_back(setcursor_message);
                            }
                        }
                    }

                    // WM_MOUSEMOVE(0x0200) 중복 제거: 이미 큐에 있으면 위치만 업데이트
                    if let Some(idx) = mousemove_idx {
                        if let Some(message) = q.get_mut(idx) {
                            message[0] = target_hwnd;
                            message[3] = lparam;
                            message[4] = time;
                            message[5] = target_x;
                            message[6] = target_y;
                        }
                    } else {
                        q.push_back([target_hwnd, 0x0200, 0, lparam, time, target_x, target_y]);
                    }

                    // 마우스 트래킹 (TrackMouseEvent) 처리
                    let mut track_opt = self.emu_context.track_mouse_event.lock().unwrap();
                    if let Some(track) = track_opt.as_ref() {
                        if track.hwnd != top_hwnd && (track.flags & 0x00000002 != 0) {
                            let time = self.emu_context.start_time.elapsed().as_millis() as u32;
                            q.push_back([track.hwnd, 0x02A3, 0, 0, time, 0, 0]); // WM_MOUSELEAVE
                            *track_opt = None;
                        }
                    }
                    self.wake_emulator();
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
                            self.wake_emulator();
                        }
                    }
                }
            }

            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(&top_hwnd) = self.id_to_hwnd.get(&id) {
                    let x = self
                        .emu_context
                        .mouse_x
                        .load(std::sync::atomic::Ordering::SeqCst);
                    let y = self
                        .emu_context
                        .mouse_y
                        .load(std::sync::atomic::Ordering::SeqCst);

                    // 스로틀링으로 인해 WM_MOUSEMOVE가 생략되었을 수 있으므로,
                    // 클릭 직전 현재 위치에 대한 이동 메시지 전송을 보장합니다.
                    if self.last_sent_mouse_pos != Some((x, y)) {
                        let time = self.emu_context.start_time.elapsed().as_millis() as u32;
                        let lparam = (y << 16) | (x & 0xFFFF);
                        let mut q = self.emu_context.message_queue.lock().unwrap();
                        q.push_back([top_hwnd, 0x0200, 0, lparam, time, x, y]);
                        self.last_sent_mouse_pos = Some((x, y));
                    }

                    // 실제 클릭이 발생한 자식 창 찾기
                    let (target_hwnd, target_x, target_y) = {
                        let win_event = self.emu_context.win_event.lock().unwrap();
                        let target =
                            win_event.child_window_from_point(top_hwnd, x as i32, y as i32);
                        if target != top_hwnd {
                            if let Some((origin_x, origin_y)) =
                                win_event.window_client_origin_in_host(target)
                            {
                                (
                                    target,
                                    (x as i32 - origin_x) as u32,
                                    (y as i32 - origin_y) as u32,
                                )
                            } else {
                                (top_hwnd, x, y)
                            }
                        } else {
                            (top_hwnd, x, y)
                        }
                    };

                    let lparam = (target_y << 16) | (target_x & 0xFFFF);
                    let mut wparam = 0;

                    // wparam에 버튼 상태 플래그 설정 (표준 Win32 behavior)
                    if state == ElementState::Pressed {
                        match button {
                            MouseButton::Left => wparam |= 0x0001,   // MK_LBUTTON
                            MouseButton::Right => wparam |= 0x0002,  // MK_RBUTTON
                            MouseButton::Middle => wparam |= 0x0010, // MK_MBUTTON
                            _ => {}
                        }
                    }

                    let msg = match (button, state) {
                        (MouseButton::Left, ElementState::Pressed) => 0x0201, // WM_LBUTTONDOWN
                        (MouseButton::Left, ElementState::Released) => 0x0202, // WM_LBUTTONUP
                        (MouseButton::Right, ElementState::Pressed) => 0x0204, // WM_RBUTTONDOWN
                        (MouseButton::Right, ElementState::Released) => 0x0205, // WM_RBUTTONUP
                        _ => 0,
                    };

                    if msg != 0 {
                        let time = self.emu_context.start_time.elapsed().as_millis() as u32;
                        let mut q = self.emu_context.message_queue.lock().unwrap();
                        q.push_back([target_hwnd, msg, wparam, lparam, time, target_x, target_y]);
                        drop(q);
                        self.wake_emulator();
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
                        drop(q);
                        self.wake_emulator();
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
                    self.wake_emulator();
                }
            }

            WindowEvent::Moved(position) => {
                if let Some(&hwnd) = self.id_to_hwnd.get(&id) {
                    let x = position.x;
                    let y = position.y;
                    let changed = self
                        .emu_context
                        .win_event
                        .lock()
                        .unwrap()
                        .sync_host_window_position(hwnd, x, y);

                    if changed {
                        let lparam = (x as i16 as u16 as u32) | ((y as i16 as u16 as u32) << 16);
                        let mut q = self.emu_context.message_queue.lock().unwrap();
                        q.push_back([hwnd, 0x0003, 0, lparam, 0, 0, 0]); // WM_MOVE
                        drop(q);
                        self.wake_emulator();
                    }
                }
            }

            WindowEvent::Resized(size) => {
                if let Some(&hwnd) = self.id_to_hwnd.get(&id) {
                    let width = size.width;
                    let height = size.height;
                    let lparam = (height << 16) | (width & 0xFFFF);

                    self.emu_context
                        .win_event
                        .lock()
                        .unwrap()
                        .resize_window(hwnd, width, height);
                    self.emu_context.sync_window_surface_bitmap(hwnd);

                    let mut q = self.emu_context.message_queue.lock().unwrap();
                    q.push_back([hwnd, 0x0005, 0, lparam, 0, 0, 0]); // WM_SIZE (SIZE_RESTORED)
                    drop(q);
                    self.wake_emulator();
                }
            }

            _ => (),
        }
    }
}

/// 에뮬레이터 윈도우용 기본 페인터
pub struct DefaultEmulatorPainter {
    hwnd: u32,
    surface_bitmap: u32,
    emu_context: Win32Context,
    /// 캐시된 윈도우 스냅샷 및 세대 카운터 (변경 시에만 재복제)
    cached_windows: HashMap<u32, crate::dll::win32::WindowState>,
    cached_generation: u64,
}

impl DefaultEmulatorPainter {
    /// 현재 호스트 창에 속한 자식 창들을 생성 순서에 가까운 핸들 순서로 반환합니다.
    fn child_windows(
        windows: &HashMap<u32, crate::dll::win32::WindowState>,
        parent: u32,
    ) -> Vec<u32> {
        let mut children = windows
            .iter()
            .filter_map(|(hwnd, state)| {
                (state.parent == parent && (state.style & WS_CHILD) != 0)
                    .then_some((*hwnd, state.z_order))
            })
            .collect::<Vec<_>>();
        children.sort_unstable_by_key(|&(hwnd, z_order)| (z_order, hwnd));
        children.into_iter().map(|(hwnd, _)| hwnd).collect()
    }

    /// 비트맵 하나를 대상 버퍼에 잘라서 복사합니다.
    fn blit_bitmap(
        buffer: &mut [u32],
        buffer_width: u32,
        buffer_height: u32,
        dest_x: i32,
        dest_y: i32,
        pixels: &[u32],
        src_width: u32,
        src_height: u32,
        clip_rects: &[(i32, i32, i32, i32)],
        exclusion_rects: &[(i32, i32, i32, i32)],
    ) {
        for src_y in 0..src_height as i32 {
            let dst_y = dest_y + src_y;
            if dst_y < 0 || dst_y >= buffer_height as i32 {
                continue;
            }

            let src_row_offset = (src_y as u32 * src_width) as usize;
            let dst_row_offset = (dst_y as u32 * buffer_width) as usize;

            for src_x in 0..src_width as i32 {
                let dst_x = dest_x + src_x;
                if dst_x < 0 || dst_x >= buffer_width as i32 {
                    continue;
                }

                // Inclusion 클리핑 체크
                let mut included = false;
                for (cl, ct, cr, cb) in clip_rects {
                    if src_x >= *cl && src_x < *cr && src_y >= *ct && src_y < *cb {
                        included = true;
                        break;
                    }
                }
                if !included {
                    continue;
                }

                let mut excluded = false;
                for (ex_l, ex_t, ex_r, ex_b) in exclusion_rects {
                    if src_x >= *ex_l && src_x < *ex_r && src_y >= *ex_t && src_y < *ex_b {
                        excluded = true;
                        break;
                    }
                }
                if excluded {
                    continue;
                }

                let dst_idx = dst_row_offset + dst_x as usize;
                let src_idx = src_row_offset + src_x as usize;
                if src_idx < pixels.len() && dst_idx < buffer.len() {
                    let p = pixels[src_idx];
                    // Lime Engine 투명도 키 (0x00FF00)는 이미 그려진 부모 배경을 유지해야 합니다.
                    if (p & 0x00FFFFFF) != 0x0000FF00 {
                        buffer[dst_idx] = p;
                    }
                }
            }
        }
    }

    /// 루트 창과 모든 자식 창 표면을 하나의 호스트 버퍼로 합성합니다.
    /// gdi_objects 락을 한 번만 잡고 재귀 전체에서 재사용합니다.
    fn composite_window_tree(
        &self,
        buffer: &mut [u32],
        buffer_width: u32,
        buffer_height: u32,
        windows: &HashMap<u32, crate::dll::win32::WindowState>,
        gdi_objects: &HashMap<u32, GdiObject>,
        hwnd: u32,
        dest_x: i32,
        dest_y: i32,
        is_root: bool,
    ) {
        let Some(state) = windows.get(&hwnd) else {
            return;
        };

        if !is_root && !state.visible {
            return;
        }

        let bitmap = gdi_objects.get(&state.surface_bitmap).and_then(|obj| {
            if let GdiObject::Bitmap {
                pixels,
                width,
                height,
                ..
            } = obj
            {
                Some((pixels.clone(), *width, *height))
            } else {
                None
            }
        });

        let clip_rects = if state.window_rgn != 0 {
            gdi_objects.get(&state.window_rgn).and_then(|obj| {
                if let GdiObject::Region { rects } = obj {
                    Some(rects.as_slice())
                } else {
                    None
                }
            })
        } else {
            None
        };

        let mut sibling_exclusion_rects = Vec::new();
        if !is_root && (state.style & WS_CLIPSIBLINGS) != 0 {
            let siblings = Self::child_windows(windows, state.parent);
            let my_pos = siblings.iter().position(|&h| h == hwnd).unwrap_or(0);

            // 부모 배경은 그대로 두되, 형제 창끼리는 위에 있는 창의 영역을 침범하지 않게 막습니다.
            for &sibling_hwnd in &siblings[my_pos + 1..] {
                let Some(sibling_state) = windows.get(&sibling_hwnd) else {
                    continue;
                };
                if !sibling_state.visible {
                    continue;
                }

                if sibling_state.window_rgn != 0 {
                    if let Some(GdiObject::Region { rects }) =
                        gdi_objects.get(&sibling_state.window_rgn)
                    {
                        for rect in rects {
                            sibling_exclusion_rects.push((
                                sibling_state.x - state.x + rect.0,
                                sibling_state.y - state.y + rect.1,
                                sibling_state.x - state.x + rect.2,
                                sibling_state.y - state.y + rect.3,
                            ));
                        }
                        continue;
                    }
                }

                sibling_exclusion_rects.push((
                    sibling_state.x - state.x,
                    sibling_state.y - state.y,
                    sibling_state.x - state.x + sibling_state.width,
                    sibling_state.y - state.y + sibling_state.height,
                ));
            }
        }

        if let Some((pixels, src_width, src_height)) = bitmap {
            let pixels = pixels.lock().unwrap();
            let final_dest_x = if is_root { 0 } else { dest_x };
            let final_dest_y = if is_root { 0 } else { dest_y };
            let default_clip = [(0, 0, src_width as i32, src_height as i32)];
            let clip_rects = clip_rects.unwrap_or(&default_clip);

            // 부모는 전체 배경을 먼저 남겨서 투명 키가 부모 배경을 드러내게 하고,
            // 형제 창끼리만 상위 z-order 영역을 제외해 픽셀 침범을 막습니다.
            Self::blit_bitmap(
                buffer,
                buffer_width,
                buffer_height,
                final_dest_x,
                final_dest_y,
                &pixels,
                src_width,
                src_height,
                clip_rects,
                &sibling_exclusion_rects,
            );

            // WS_BORDER 처리: 검은색 1px 테두리
            if (state.style & WS_BORDER) != 0 {
                GdiRenderer::draw_rect(
                    buffer,
                    buffer_width,
                    buffer_height,
                    final_dest_x,
                    final_dest_y,
                    final_dest_x + src_width as i32,
                    final_dest_y + src_height as i32,
                    Some(0xFF000000), // Black
                    None,
                );
            }

            // WS_DLGFRAME / WS_SIZEBOX (WS_THICKFRAME) 처리: 3D 테두리
            if !state.use_native_frame {
                if (state.style & WS_DLGFRAME) != 0 || (state.style & WS_SIZEBOX) != 0 {
                    GdiRenderer::draw_edge(
                        buffer,
                        buffer_width,
                        buffer_height,
                        final_dest_x,
                        final_dest_y,
                        final_dest_x + src_width as i32,
                        final_dest_y + src_height as i32,
                        false, // Raised
                    );
                }
            }

            // WS_HSCROLL 처리: 하단 가로 스크롤바 (단순 플레이스홀더)
            if (state.style & WS_HSCROLL) != 0 {
                let scroll_h = 16i32;
                let top = (final_dest_y + src_height as i32 - scroll_h).max(final_dest_y);
                GdiRenderer::draw_rect(
                    buffer,
                    buffer_width,
                    buffer_height,
                    final_dest_x,
                    top,
                    final_dest_x + src_width as i32,
                    final_dest_y + src_height as i32,
                    Some(0xFF808080), // Gray Pen
                    Some(0xFFC0C0C0), // Face Brush
                );
                // 양쪽 버튼
                GdiRenderer::draw_edge(
                    buffer,
                    buffer_width,
                    buffer_height,
                    final_dest_x,
                    top,
                    final_dest_x + 16,
                    final_dest_y + src_height as i32,
                    false,
                );
                GdiRenderer::draw_edge(
                    buffer,
                    buffer_width,
                    buffer_height,
                    final_dest_x + src_width as i32 - 16,
                    top,
                    final_dest_x + src_width as i32,
                    final_dest_y + src_height as i32,
                    false,
                );
            }
        }

        for child_hwnd in Self::child_windows(windows, hwnd) {
            let Some(child_state) = windows.get(&child_hwnd) else {
                continue;
            };

            let child_x = if is_root {
                child_state.x
            } else {
                dest_x + child_state.x
            };
            let child_y = if is_root {
                child_state.y
            } else {
                dest_y + child_state.y
            };

            self.composite_window_tree(
                buffer,
                buffer_width,
                buffer_height,
                windows,
                gdi_objects,
                child_hwnd,
                child_x,
                child_y,
                false,
            );
        }
    }
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

    fn paint(&mut self, buffer: &mut [u32], width: u32, height: u32) -> bool {
        // 세대 카운터가 변경된 경우에만 윈도우 상태를 재복제합니다.
        {
            let win_event = match self.emu_context.win_event.try_lock() {
                Ok(event) => event,
                Err(_) => return false,
            };
            if win_event.generation != self.cached_generation {
                self.cached_windows = win_event.windows.clone();
                self.cached_generation = win_event.generation;
            }
        }

        let gdi_objects = match self.emu_context.gdi_objects.try_lock() {
            Ok(g) => g,
            Err(_) => return false, // 락 획 실패 시 이번 프레임은 건너뜀 (데드락 방지)
        };

        if self.cached_windows.contains_key(&self.hwnd) {
            self.composite_window_tree(
                buffer,
                width,
                height,
                &self.cached_windows,
                &gdi_objects,
                self.hwnd,
                0,
                0,
                true,
            );
            return true;
        }

        if let Some(GdiObject::Bitmap {
            pixels: p,
            width: sw,
            height: sh,
            ..
        }) = gdi_objects.get(&self.surface_bitmap)
        {
            let p = p.lock().unwrap();
            let default_clip = [(0, 0, *sw as i32, *sh as i32)];
            Self::blit_bitmap(
                buffer,
                width,
                height,
                0,
                0,
                &p,
                *sw,
                *sh,
                &default_clip,
                &[],
            );
        }

        true
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
