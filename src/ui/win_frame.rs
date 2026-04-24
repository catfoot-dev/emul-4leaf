use crate::{
    dll::win32::{GdiObject, Win32Context},
    ui::{
        Painter, UiCommand, WindowPositionMode,
        render::{
            RenderFrameError, RenderOutcome, SurfaceAcquireError, UiGpuContext, WindowRenderTarget,
        },
    },
};
use rfd::{AsyncMessageDialog, MessageButtons, MessageDialogResult, MessageLevel};
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    pin::Pin,
    sync::{Arc, mpsc::Receiver},
    task::{Context, Poll, Wake, Waker},
    time::{Duration, Instant},
};
#[cfg(target_os = "windows")]
use winit::platform::windows::{WindowAttributesExtWindows, WindowExtWindows};
#[cfg(target_os = "windows")]
use winit::raw_window_handle::RawWindowHandle;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::{ElementState, Ime, KeyEvent, MouseButton, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{Key, NamedKey},
    raw_window_handle::HasWindowHandle,
    window::{Icon, Window, WindowAttributes, WindowButtons, WindowId, WindowLevel},
};

// Windows 스타일 -> winit 속성 매핑
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000; // WS_SIZEBOX
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_EX_TOPMOST: u32 = 0x0000_0008;
const WS_EX_LAYERED: u32 = 0x0008_0000;
const CW_USEDEFAULT: i32 = i32::MIN;
#[cfg(target_os = "windows")]
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
#[cfg(target_os = "windows")]
const WS_EX_APPWINDOW: u32 = 0x0004_0000;
const WM_CHAR: u32 = 0x0102;
const WM_IME_CHAR: u32 = 0x0286;
const GUEST_REDRAW_COALESCE_DELAY: Duration = Duration::from_millis(2);
const SURFACE_RETRY_DELAY: Duration = Duration::from_millis(16);

fn trace_ui_enabled() -> bool {
    std::env::var("EMUL_TRACE_UI").ok().as_deref() == Some("1")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostParentLink {
    None,
    Child,
    Owner,
}

fn enqueue_window_message(queue: &mut std::collections::VecDeque<[u32; 7]>, message: [u32; 7]) {
    if message[1] == 0x0005 {
        // 같은 창의 대기 중인 WM_SIZE는 마지막 크기만 의미가 있으므로 최신 항목만 남깁니다.
        queue.retain(|queued| !(queued[0] == message[0] && queued[1] == 0x0005));
    }
    queue.push_back(message);
}

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
    fn from_guest(style: u32, ex_style: u32, use_native_frame: bool, has_window_rgn: bool) -> Self {
        // WS_POPUP 자체가 프레임을 금지하는 것은 아니므로,
        // 실제 장식 여부는 캡션/프레임 비트와 호스트 네이티브 프레임 사용 여부로만 결정합니다.
        let decorations = use_native_frame && (style & WS_CAPTION) != 0;
        let resizable = (style & WS_THICKFRAME) != 0;
        // WS_POPUP 단독은 투명 창이 아니며, 리전이나 layered 스타일이 있을 때만 compositor 투명이 필요합니다.
        let transparent = (ex_style & WS_EX_LAYERED) != 0 || has_window_rgn;
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

/// 실제 호스트 창에 적용할 커서 동작입니다.
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

/// 비동기 메시지 박스 완료를 기다리는 상태입니다.
struct PendingMessageDialog {
    /// `rfd`가 반환한 완료 future입니다.
    future: Pin<Box<dyn Future<Output = MessageDialogResult> + Send>>,
    /// guest 쪽 대기자에게 결과를 돌려줄 채널입니다.
    response_tx: std::sync::mpsc::Sender<i32>,
}

/// 비동기 dialog 완료 시 UI 이벤트 루프를 다시 깨우는 waker입니다.
struct UiEventLoopWake;

impl Wake for UiEventLoopWake {
    fn wake(self: Arc<Self>) {
        crate::ui::win_event::WinEvent::notify_wakeup();
    }
}

/// 윈도우 애플리케이션 핸들러입니다.
///
/// 모든 `winit` 윈도우와 `wgpu` 렌더 타깃을 관리합니다.
pub struct WinFrame {
    ui_rx: Receiver<UiCommand>,

    /// 윈도우 ID -> `wgpu` 렌더 타깃
    render_targets: HashMap<WindowId, WindowRenderTarget>,
    /// 가상 HWND -> 윈도우 ID
    hwnd_to_id: HashMap<u32, WindowId>,
    /// 윈도우 ID -> 가상 HWND
    id_to_hwnd: HashMap<WindowId, u32>,
    /// 가상 HWND -> 호스트 네이티브 프레임 사용 여부
    hwnd_native_frame: HashMap<u32, bool>,
    /// 첫 번째 게스트 최상위 창 HWND
    main_guest_hwnd: Option<u32>,
    /// 메인 게스트 창을 이미 한 번이라도 등록했는지 여부
    main_guest_window_registered: bool,
    /// guest가 ShowWindow를 보내기 전이라도 첫 프레임 렌더 시 메인 창을 노출했는지 여부
    main_guest_window_forced_visible: bool,

    /// 공유 `wgpu` 장치와 파이프라인 상태
    gpu_context: Option<UiGpuContext>,
    /// Win32 컨텍스트 (공유 상태)
    pub emu_context: Win32Context,

    /// 초기 페인터 목록 (resumed에서 창 생성 후 render_targets로 이동)
    initial_painters: Vec<Box<dyn Painter>>,

    /// 현재 활성화된 커서 애니메이션 상태 (None이면 정적 커서)
    cursor_anim: Option<CursorAnimState>,
    /// 현재 적용된 커서 핸들과 윈도우 ID (중복 적용 방지용)
    current_cursor: Option<(WindowId, u32)>,
    /// 포인터가 현재 올라간 호스트 윈도우 ID
    hovered_window_id: Option<WindowId>,
    /// 포커스를 가진 호스트 윈도우 ID
    focused_window_id: Option<WindowId>,
    /// 윈도우별 마지막 guest 요청 커서 핸들 캐시
    window_cursor_cache: HashMap<WindowId, u32>,
    /// 마지막으로 마우스 이벤트가 처리된 시간 (스로틀링용)
    last_cursor_moved: Option<(u32, std::time::Instant)>,
    /// 마지막으로 게스트에게 전송된 마우스 좌표 (윈도우별 스로틀링 누락 감지용)
    last_sent_mouse_pos: Option<(u32, u32, u32)>,
    /// UI 스레드에 떠 있는 비동기 메시지 박스 목록
    pending_message_dialogs: Vec<PendingMessageDialog>,
    /// guest UpdateWindow 요청을 짧게 모아 중간 GDI 프레임 노출을 줄이는 대기 목록
    pending_dirty_windows: HashMap<WindowId, Instant>,
}

impl WinFrame {
    fn encode_committed_char_for_ansi(ch: char) -> Vec<u32> {
        if (ch as u32) < 0x80 {
            return vec![ch as u32];
        }

        let text = ch.to_string();
        let (encoded, _, _) = encoding_rs::EUC_KR.encode(&text);
        let bytes = encoded.as_ref();
        if bytes.is_empty() {
            return Vec::new();
        }

        let mut result = Vec::new();
        let mut index = 0usize;
        while index < bytes.len() {
            if index + 1 < bytes.len() {
                result.push(bytes[index] as u32 | ((bytes[index + 1] as u32) << 8));
                index += 2;
            } else {
                result.push(bytes[index] as u32);
                index += 1;
            }
        }
        result
    }

    fn committed_char_messages(ch: char) -> Vec<(u32, u32)> {
        if (ch as u32) < 0x80 {
            return vec![(WM_CHAR, ch as u32)];
        }

        Self::encode_committed_char_for_ansi(ch)
            .into_iter()
            .map(|value| (WM_IME_CHAR, value))
            .collect()
    }

    fn activation_focus_targets(&self, root_hwnd: u32) -> Vec<u32> {
        if self.hwnd_to_id.contains_key(&root_hwnd) {
            vec![root_hwnd]
        } else {
            Vec::new()
        }
    }

    /// 지정된 창 하나에만 호스트 포커스를 요청합니다.
    fn activate_window_tree(&self, root_hwnd: u32) {
        for hwnd in self.activation_focus_targets(root_hwnd) {
            if let Some(id) = self.hwnd_to_id.get(&hwnd)
                && let Some(window) = self.get_window(id)
            {
                window.focus_window();
            }
        }
    }

    fn host_parent_link(parent: u32, style: u32) -> HostParentLink {
        if parent == 0 {
            HostParentLink::None
        } else if (style & WS_CHILD) != 0 {
            HostParentLink::Child
        } else {
            HostParentLink::Owner
        }
    }

    fn generate_window_attributes(
        &self,
        hwnd: u32,
        title: String,
        x: i32,
        y: i32,
        position_mode: WindowPositionMode,
        width: u32,
        height: u32,
        parent: u32,
        visible: bool,
        style: u32,
        ex_style: u32,
        use_native_frame: bool,
    ) -> WindowAttributes {
        let host_visible = if parent == 0 { true } else { visible };

        let (class_icon, has_window_rgn) = {
            let win_event = self.emu_context.win_event.lock().unwrap();
            win_event.windows.get(&hwnd).map(|state| {
                let class_icon = if state.small_icon != 0 {
                    state.small_icon
                } else if state.big_icon != 0 {
                    state.big_icon
                } else {
                    state.class_icon
                };
                (class_icon, state.window_rgn != 0)
            })
        }
        .unwrap_or((0, false));
        let host_style =
            HostWindowStyle::from_guest(style, ex_style, use_native_frame, has_window_rgn);

        let mut attributes = Window::default_attributes()
            // 게스트가 다루는 좌표계는 픽셀 기반이므로 backing store 크기와 1:1로 맞춥니다.
            .with_active(true)
            .with_inner_size(PhysicalSize::new(width, height))
            .with_min_inner_size(PhysicalSize::new(width, height))
            .with_resizable(host_style.resizable)
            .with_title(title)
            .with_transparent(host_style.transparent)
            .with_visible(host_visible)
            .with_window_icon(self.host_window_icon(class_icon))
            .with_window_level(host_style.window_level());

        if x != CW_USEDEFAULT && y != CW_USEDEFAULT {
            let position = winit::dpi::PhysicalPosition::new(x, y);
            attributes = match position_mode {
                WindowPositionMode::Screen | WindowPositionMode::ParentClient => {
                    attributes.with_position(position)
                }
            };
        }

        #[cfg(target_os = "macos")]
        {
            use winit::platform::macos::{OptionAsAlt, WindowAttributesExtMacOS};

            attributes = attributes
                .with_decorations(false)
                .with_has_shadow(false)
                .with_option_as_alt(OptionAsAlt::Both);
        }

        #[cfg(target_os = "windows")]
        {
            // 툴 윈도우/레이어드 윈도우 같은 Win32 확장 스타일을 가능한 범위에서 반영합니다.
            attributes = attributes
                .with_skip_taskbar(host_style.skip_taskbar)
                .with_undecorated_shadow(!host_style.decorations && !host_style.transparent);
        }

        let parent_id = if Self::host_parent_link(parent, style) != HostParentLink::None {
            self.hwnd_to_id.get(&parent)
        } else {
            None
        };
        if parent_id.is_some() {
            let parent_window = self.get_window(parent_id.unwrap()).unwrap();

            #[cfg(not(target_os = "windows"))]
            {
                if let Ok(parent_handle) = parent_window.window_handle() {
                    return unsafe { attributes.with_parent_window(Some(parent_handle.as_raw())) };
                }
            }

            #[cfg(target_os = "windows")]
            {
                if let Ok(parent_handle) = parent_window.window_handle() {
                    let raw = parent_handle.as_raw();
                    match host_parent_link {
                        HostParentLink::None => return attributes,
                        HostParentLink::Child => {
                            return unsafe { attributes.with_parent_window(Some(raw)) };
                        }
                        HostParentLink::Owner => {
                            if let RawWindowHandle::Win32(handle) = raw {
                                return attributes.with_owner_window(handle.hwnd.get() as _);
                            }
                        }
                    }
                }
            }
        }

        #[allow(unreachable_code)]
        attributes
    }

    fn set_macos_corner_radius(&self, window: &Window, radius: f32) {
        #[cfg(target_os = "macos")]
        {
            use objc2_app_kit::{NSColor, NSView};
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

            let Ok(handle) = window.window_handle() else {
                return;
            };

            let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
                return;
            };

            unsafe {
                let ns_view: &NSView = appkit.ns_view.cast::<NSView>().as_ref();
                ns_view.setWantsLayer(true);

                if let Some(ns_window) = ns_view.window() {
                    ns_window.setOpaque(false);
                    ns_window.setBackgroundColor(Some(&NSColor::clearColor()));
                    ns_window.setHasShadow(true);
                }

                if let Some(layer) = ns_view.layer() {
                    layer.setCornerRadius(radius.into());
                    layer.setMasksToBounds(true);
                }
            }
        }
    }

    /// UI 채널과 초기 painter 목록으로 새 프레임 핸들러를 생성합니다.
    pub fn new(
        ui_rx: Receiver<UiCommand>,
        initial_painters: Vec<Box<dyn Painter>>,
        context: Win32Context,
    ) -> Self {
        Self {
            ui_rx,
            render_targets: HashMap::new(),
            hwnd_to_id: HashMap::new(),
            id_to_hwnd: HashMap::new(),
            hwnd_native_frame: HashMap::new(),
            main_guest_hwnd: None,
            main_guest_window_registered: false,
            main_guest_window_forced_visible: false,
            gpu_context: None,
            emu_context: context,
            initial_painters,
            cursor_anim: None,
            current_cursor: None,
            hovered_window_id: None,
            focused_window_id: None,
            window_cursor_cache: HashMap::new(),
            last_cursor_moved: None,
            last_sent_mouse_pos: None,
            pending_message_dialogs: Vec::new(),
            pending_dirty_windows: HashMap::new(),
        }
    }

    fn ensure_gpu_context(&mut self) -> Result<&UiGpuContext, String> {
        if self.gpu_context.is_none() {
            self.gpu_context = Some(UiGpuContext::new()?);
        }
        Ok(self.gpu_context.as_ref().unwrap())
    }

    fn get_window(&self, id: &WindowId) -> Option<&Window> {
        self.render_targets.get(id).map(WindowRenderTarget::window)
    }

    /// 첫 번째 게스트 최상위 창을 메인 창으로 기록합니다.
    fn register_main_guest_window(&mut self, hwnd: u32, style: u32) {
        if (style & WS_CHILD) == 0 && !self.main_guest_window_registered {
            self.main_guest_hwnd = Some(hwnd);
            self.main_guest_window_registered = true;
            self.main_guest_window_forced_visible = false;
        }
    }

    /// 파괴 대상 HWND가 메인 게스트 창이면 종료 대상으로 판정하고 기록을 비웁니다.
    fn take_main_guest_window_close(&mut self, hwnd: u32) -> bool {
        if self.main_guest_hwnd == Some(hwnd) {
            self.main_guest_hwnd = None;
            self.main_guest_window_forced_visible = false;
            true
        } else {
            false
        }
    }

    /// 호스트 닫기 요청이 즉시 UI 종료로 이어져야 하는지 판정합니다.
    ///
    /// 메인 게스트 창은 guest가 `WM_CLOSE`를 소비해도 사용자는 종료를 기대하므로
    /// host 종료 fallback을 유지합니다. 팝업/보조 창은 guest destroy 경로를 기다립니다.
    fn should_exit_on_close_request(&self, hwnd: Option<u32>, quit_on_close: bool) -> bool {
        quit_on_close || hwnd.is_some_and(|handle| self.main_guest_hwnd == Some(handle))
    }

    /// 가상 아이콘 핸들을 호스트 윈도우 아이콘으로 변환합니다.
    fn host_window_icon(&self, hicon: u32) -> Option<Icon> {
        if hicon == 0 {
            return None;
        }

        let frame = {
            let gdi_objects = self.emu_context.gdi_objects.lock().unwrap();
            match gdi_objects.get(&hicon) {
                Some(GdiObject::Icon { frames, .. }) => frames
                    .iter()
                    .max_by_key(|frame| frame.width.saturating_mul(frame.height))
                    .cloned(),
                _ => None,
            }
        }?;

        let rgba = frame
            .pixels
            .iter()
            .flat_map(|pixel| {
                let a = ((pixel >> 24) & 0xFF) as u8;
                let r = ((pixel >> 16) & 0xFF) as u8;
                let g = ((pixel >> 8) & 0xFF) as u8;
                let b = (pixel & 0xFF) as u8;
                [r, g, b, a]
            })
            .collect::<Vec<u8>>();

        Icon::from_rgba(rgba, frame.width, frame.height).ok()
    }

    /// 창을 제거하고 관련 상태(커서 애니메이션 포함)를 정리합니다.
    fn remove_window(&mut self, id: WindowId) {
        self.render_targets.remove(&id);
        self.pending_dirty_windows.remove(&id);
        if let Some(hwnd) = self.id_to_hwnd.remove(&id) {
            self.hwnd_to_id.remove(&hwnd);
            self.hwnd_native_frame.remove(&hwnd);
        }
        self.window_cursor_cache.remove(&id);
        if self.hovered_window_id == Some(id) {
            self.hovered_window_id = None;
        }
        if self.focused_window_id == Some(id) {
            self.focused_window_id = None;
        }

        // 창이 파괴되면 해당 창과 연결된 커서 상태도 정리합니다.
        self.clear_cursor_state_for_window(id);
    }

    /// 현재 커서 적용 규칙에 따른 소유 창을 계산합니다.
    fn current_cursor_owner(&self) -> Option<WindowId> {
        self.hovered_window_id.or(self.focused_window_id)
    }

    /// 지정된 창에 연결된 커서 적용 상태를 무효화합니다.
    fn clear_cursor_state_for_window(&mut self, id: WindowId) {
        if self.current_cursor.as_ref().map(|(cid, _)| *cid) == Some(id) {
            self.current_cursor = None;
        }
        if self.cursor_anim.as_ref().map(|anim| anim.window_id) == Some(id) {
            self.cursor_anim = None;
        }
    }

    /// 호버 중인 창을 갱신합니다.
    fn set_hovered_window(&mut self, next: Option<WindowId>) -> bool {
        if self.hovered_window_id == next {
            return false;
        }

        if let Some(prev) = self.hovered_window_id
            && Some(prev) != next
        {
            self.clear_cursor_state_for_window(prev);
        }

        self.hovered_window_id = next;
        true
    }

    /// 포커스된 창을 갱신합니다.
    fn set_focused_window(&mut self, next: Option<WindowId>) -> bool {
        if self.focused_window_id == next {
            return false;
        }

        if self.hovered_window_id.is_none()
            && let Some(prev) = self.focused_window_id
            && Some(prev) != next
        {
            self.clear_cursor_state_for_window(prev);
        }

        self.focused_window_id = next;
        true
    }

    /// 지정된 창이 현재 커서 즉시 적용 대상인지 반환합니다.
    fn is_cursor_owner(&self, id: WindowId) -> bool {
        self.current_cursor_owner() == Some(id)
    }

    /// 윈도우별 마지막 guest 커서 핸들을 저장합니다.
    fn cache_window_cursor(&mut self, id: WindowId, hcursor: u32) {
        self.window_cursor_cache.insert(id, hcursor);
    }

    /// 지정된 창에 대해 실제로 적용해야 할 커서 핸들을 계산합니다.
    fn effective_cursor_handle_for_window(&self, id: WindowId) -> u32 {
        if let Some(&hcursor) = self.window_cursor_cache.get(&id) {
            return hcursor;
        }

        let Some(hwnd) = self.id_to_hwnd.get(&id).copied() else {
            return 0;
        };

        self.emu_context
            .win_event
            .lock()
            .unwrap()
            .windows
            .get(&hwnd)
            .map(|window| window.class_cursor)
            .unwrap_or(0)
    }

    /// 시스템 커서 리소스 ID를 winit 커서 아이콘으로 변환합니다.
    fn system_cursor_icon(resource_id: u32) -> winit::window::CursorIcon {
        match resource_id {
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
        }
    }

    /// 커서 핸들을 실제 호스트 커서 동작으로 변환합니다.
    fn resolve_cursor_action(&self, hcursor: u32, event_loop: &ActiveEventLoop) -> CursorAction {
        if hcursor == 0 {
            return CursorAction::Default;
        }

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
                if let Some(custom) = Self::create_custom_cursor_from_frame(&frames[0], event_loop)
                {
                    let rate = (*display_rate_jiffies / 2u32).max(1);
                    let ms = (rate as u64) * 1000 / 60;
                    return CursorAction::Animated {
                        cursor: custom,
                        frame_count: frames.len(),
                        interval: std::time::Duration::from_millis(ms),
                    };
                }
                return CursorAction::Default;
            }

            if let Some(frame) = frames.first()
                && !frame.pixels.is_empty()
            {
                if let Some(custom) = Self::create_custom_cursor_from_frame(frame, event_loop) {
                    return CursorAction::Static(custom);
                }
                return CursorAction::Default;
            }

            return CursorAction::SystemIcon(Self::system_cursor_icon(*resource_id));
        }

        CursorAction::Default
    }

    /// 계산된 커서 동작을 호스트 창에 반영합니다.
    fn apply_cursor_action(window: &Window, action: &CursorAction) {
        match action {
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

    /// 지정된 창의 현재 유효 커서를 계산해 즉시 적용합니다.
    fn apply_effective_cursor_for_window(
        &mut self,
        id: WindowId,
        event_loop: &ActiveEventLoop,
    ) -> bool {
        let hcursor = self.effective_cursor_handle_for_window(id);
        if self.current_cursor == Some((id, hcursor)) {
            return false;
        }

        let action = self.resolve_cursor_action(hcursor, event_loop);
        {
            let Some(window) = self.get_window(&id) else {
                self.clear_cursor_state_for_window(id);
                return false;
            };
            Self::apply_cursor_action(window, &action);
        }
        self.current_cursor = Some((id, hcursor));

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

        true
    }

    /// 현재 커서 소유 규칙에 맞는 창으로 호스트 커서를 다시 맞춥니다.
    fn reapply_cursor_for_current_owner(&mut self, event_loop: &ActiveEventLoop) -> bool {
        if let Some(id) = self.current_cursor_owner() {
            self.apply_effective_cursor_for_window(id, event_loop)
        } else {
            self.current_cursor = None;
            self.cursor_anim = None;
            false
        }
    }

    /// 에뮬레이터 스레드를 즉시 깨워 새 메시지를 처리하도록 합니다.
    fn wake_emulator(&self, target_thread_id: Option<u32>) {
        if let Some(thread_id) = target_thread_id {
            self.emu_context.wake_thread_message_wait(thread_id);
        }
        if let Ok(guard) = self.emu_context.emu_thread.try_lock()
            && let Some(thread) = guard.as_ref()
        {
            thread.unpark();
        }
    }

    fn queue_message_for_window(&self, hwnd: u32, message: [u32; 7]) -> u32 {
        self.emu_context.queue_message_for_window(hwnd, message)
    }

    fn apply_guest_window_style(
        window: &Window,
        style: u32,
        ex_style: u32,
        use_native_frame: bool,
        has_window_rgn: bool,
    ) {
        let host_style =
            HostWindowStyle::from_guest(style, ex_style, use_native_frame, has_window_rgn);

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

    /// `rfd` 결과를 Win32 `MessageBoxA` 반환값으로 변환합니다.
    fn message_dialog_result_to_win32(result: MessageDialogResult) -> i32 {
        match result {
            MessageDialogResult::Ok => 1,
            MessageDialogResult::Cancel => 2,
            MessageDialogResult::Yes => 6,
            MessageDialogResult::No => 7,
            MessageDialogResult::Custom(_) => 1,
        }
    }

    /// 비동기 메시지 박스를 생성하고 완료 future를 추적 목록에 추가합니다.
    fn queue_message_dialog(
        &mut self,
        owner_hwnd: u32,
        caption: String,
        text: String,
        u_type: u32,
        response_tx: std::sync::mpsc::Sender<i32>,
    ) {
        let mut dialog = AsyncMessageDialog::new()
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

        if owner_hwnd != 0
            && let Some(id) = self.hwnd_to_id.get(&owner_hwnd)
            && let Some(window) = self.get_window(id)
        {
            dialog = dialog.set_parent(window);
        }

        let future: Pin<Box<dyn Future<Output = MessageDialogResult> + Send>> =
            Box::pin(dialog.show());
        self.pending_message_dialogs.push(PendingMessageDialog {
            future,
            response_tx,
        });
    }

    /// 완료된 비동기 메시지 박스를 수거하고 guest 쪽 대기자에게 결과를 전달합니다.
    fn poll_pending_message_dialogs(&mut self) {
        if self.pending_message_dialogs.is_empty() {
            return;
        }

        let waker = Waker::from(Arc::new(UiEventLoopWake));
        let mut cx = Context::from_waker(&waker);
        let mut completed = Vec::new();

        for (index, pending) in self.pending_message_dialogs.iter_mut().enumerate() {
            if let Poll::Ready(result) = pending.future.as_mut().poll(&mut cx) {
                let _ = pending
                    .response_tx
                    .send(Self::message_dialog_result_to_win32(result));
                completed.push(index);
            }
        }

        for index in completed.into_iter().rev() {
            self.pending_message_dialogs.swap_remove(index);
        }
    }

    fn process_ui_commands(&mut self, event_loop: &ActiveEventLoop) -> HashSet<WindowId> {
        crate::ui::win_event::WinEvent::clear_wake_pending();
        let mut dirty_windows = HashSet::new();

        while let Ok(cmd) = self.ui_rx.try_recv() {
            let start = std::time::Instant::now();
            match cmd {
                UiCommand::CreateWindow {
                    hwnd,
                    title,
                    x,
                    y,
                    position_mode,
                    width,
                    height,
                    style,
                    ex_style,
                    parent,
                    visible,
                    use_native_frame,
                    surface_bitmap,
                    ..
                } => {
                    if parent == 0 {
                        self.register_main_guest_window(hwnd, style);
                    }
                    let attributes = self.generate_window_attributes(
                        hwnd,
                        title,
                        x,
                        y,
                        position_mode,
                        width,
                        height,
                        parent,
                        visible,
                        style,
                        ex_style,
                        use_native_frame,
                    );
                    let window = Arc::new(event_loop.create_window(attributes).unwrap());
                    // IME(한글 등 조합 문자)를 입력받기 위해 활성화합니다.
                    window.set_ime_allowed(true);
                    self.set_macos_corner_radius(&window, if parent == 0 { 8.0 } else { 10.0 });
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
                    if parent == 0 && !self.main_guest_window_forced_visible {
                        crate::emu_log!("[UI] host eager-show main HWND {:#x}", hwnd);
                        window.set_visible(true);
                        self.main_guest_window_forced_visible = true;
                        dirty_windows.insert(id);
                    }
                    self.ensure_gpu_context().unwrap();
                    let target = {
                        let gpu = self.gpu_context.as_ref().unwrap();
                        WindowRenderTarget::new_guest(gpu, window, hwnd, surface_bitmap).unwrap()
                    };
                    self.render_targets.insert(id, target);
                    dirty_windows.insert(id);
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
                        let has_window_rgn = self
                            .emu_context
                            .win_event
                            .lock()
                            .unwrap()
                            .windows
                            .get(&hwnd)
                            .map(|state| state.window_rgn != 0)
                            .unwrap_or(false);

                        Self::apply_guest_window_style(
                            window,
                            style,
                            ex_style,
                            use_native_frame,
                            has_window_rgn,
                        );
                        dirty_windows.insert(*id);
                    }
                }

                UiCommand::DestroyWindow { hwnd } => {
                    let should_exit = self.take_main_guest_window_close(hwnd);
                    if let Some(id) = self.hwnd_to_id.get(&hwnd).copied() {
                        self.remove_window(id);
                    }
                    if should_exit {
                        event_loop.exit();
                    }
                }

                UiCommand::ExitApplication => {
                    event_loop.exit();
                }

                UiCommand::ShowWindow { hwnd, visible } => {
                    if let Some(id) = self.hwnd_to_id.get(&hwnd).copied() {
                        if self.main_guest_hwnd == Some(hwnd) {
                            if visible {
                                self.main_guest_window_forced_visible = false;
                            } else if self.main_guest_window_forced_visible {
                                crate::emu_log!(
                                    "[UI] host keep main HWND {:#x} visible during startup",
                                    hwnd
                                );
                                dirty_windows.insert(id);
                                continue;
                            }
                        }
                        if let Some(window) = self.get_window(&id) {
                            crate::emu_log!(
                                "[UI] host ShowWindow HWND {:#x} visible={}",
                                hwnd,
                                visible
                            );
                            window.set_visible(visible);
                            if visible {
                                dirty_windows.insert(id);
                            }
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
                    position_mode,
                    width,
                    height,
                } => {
                    if let Some(id) = self.hwnd_to_id.get(&hwnd)
                        && let Some(window) = self.get_window(id)
                    {
                        let position = winit::dpi::PhysicalPosition::new(x, y);
                        match position_mode {
                            WindowPositionMode::Screen | WindowPositionMode::ParentClient => {
                                window.set_outer_position(position);
                            }
                        }
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

                UiCommand::SetWindowIcon { hwnd, hicon } => {
                    if let Some(id) = self.hwnd_to_id.get(&hwnd)
                        && let Some(window) = self.get_window(id)
                    {
                        window.set_window_icon(self.host_window_icon(hicon));
                    }
                }

                UiCommand::UpdateWindow { hwnd } => {
                    if let Some(id) = self.hwnd_to_id.get(&hwnd)
                        && let Some(_window) = self.get_window(id)
                    {
                        self.schedule_dirty_window(*id, GUEST_REDRAW_COALESCE_DELAY);
                    }
                }

                UiCommand::ActivateWindow { hwnd } => {
                    self.activate_window_tree(hwnd);
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
                    owner_hwnd,
                    caption,
                    text,
                    u_type,
                    response_tx,
                } => {
                    let _ = event_loop;
                    self.queue_message_dialog(owner_hwnd, caption, text, u_type, response_tx);
                }

                UiCommand::DragWindow { hwnd } => {
                    let window_id = self.hwnd_to_id.get(&hwnd).copied();
                    if let Some(id) = window_id
                        && let Some(window) = self.get_window(&id)
                    {
                        crate::emu_log!("[UI] DragWindow called for HWND {:#x}", hwnd);
                        let _ = window.drag_window();
                    }
                }

                UiCommand::SetWindowTransparent { hwnd, transparent } => {
                    if let Some(id) = self.hwnd_to_id.get(&hwnd)
                        && let Some(window) = self.get_window(id)
                    {
                        window.set_transparent(transparent);
                        dirty_windows.insert(*id);
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
                        self.cache_window_cursor(id, hcursor);
                        if self.is_cursor_owner(id) {
                            self.apply_effective_cursor_for_window(id, event_loop);
                        }
                    }
                }
            }

            let elapsed = start.elapsed();
            if elapsed.as_millis() > 50 {
                println!("[WATCHDOG] UI Command took {:?} to process", elapsed);
            }
        }

        dirty_windows
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
            if let Some(window) = self.get_window(&window_id)
                && let Some(custom) = Self::create_custom_cursor_from_frame(&frame, event_loop)
            {
                window.set_cursor(custom);
            }
        } else {
            self.cursor_anim = None;
        }
    }

    fn next_poll_interval(&self) -> Option<std::time::Duration> {
        let painter_interval = self
            .render_targets
            .values()
            .filter_map(WindowRenderTarget::poll_interval)
            .min();

        // 커서 애니메이션이 활성화되어 있으면 그 간격도 고려
        let cursor_interval = self.cursor_anim.as_ref().map(|anim| {
            let elapsed = anim.last_switch.elapsed();
            anim.interval.saturating_sub(elapsed)
        });

        let dialog_interval = (!self.pending_message_dialogs.is_empty())
            .then_some(std::time::Duration::from_millis(16));

        match (painter_interval, cursor_interval, dialog_interval) {
            (Some(a), Some(b), Some(c)) => Some(a.min(b).min(c)),
            (Some(a), Some(b), None) => Some(a.min(b)),
            (Some(a), None, Some(c)) => Some(a.min(c)),
            (None, Some(b), Some(c)) => Some(b.min(c)),
            (Some(a), None, None) => Some(a),
            (None, Some(b), None) => Some(b),
            (None, None, Some(c)) => Some(c),
            (None, None, None) => None,
        }
    }

    /// 지연 시간이 지난 guest redraw 요청을 즉시 처리 목록으로 옮깁니다.
    fn collect_ready_guest_redraws(
        &mut self,
        now: Instant,
        dirty_windows: &mut HashSet<WindowId>,
    ) -> Option<Instant> {
        let mut next_deadline = None;
        self.pending_dirty_windows.retain(|id, ready_at| {
            if now >= *ready_at {
                dirty_windows.insert(*id);
                false
            } else {
                next_deadline = Some(
                    next_deadline.map_or(*ready_at, |current: Instant| current.min(*ready_at)),
                );
                true
            }
        });
        next_deadline
    }

    /// 즉시 재요청 대신 짧은 지연 뒤 redraw를 다시 시도하도록 예약합니다.
    fn schedule_dirty_window(&mut self, id: WindowId, delay: Duration) {
        let ready_at = Instant::now() + delay;
        self.pending_dirty_windows
            .entry(id)
            .and_modify(|current| {
                if ready_at < *current {
                    *current = ready_at;
                }
            })
            .or_insert(ready_at);
    }

    /// 지정된 창의 현재 프레임을 이벤트 대기 없이 즉시 제출합니다.
    fn render_window(&mut self, id: WindowId) -> bool {
        if !self.should_render_window(id) {
            return false;
        }

        let hwnd = self.id_to_hwnd.get(&id).copied();
        let Some(gpu) = self.gpu_context.as_ref() else {
            return false;
        };

        if trace_ui_enabled() {
            eprintln!("[TRACE_UI] render_window {:?}", id);
        }

        let rendered = {
            let Some(target) = self.render_targets.get_mut(&id) else {
                return false;
            };
            match target.render(gpu, &self.emu_context) {
                Ok(RenderOutcome::Rendered) => true,
                Ok(RenderOutcome::Skipped) => false,
                Err(RenderFrameError::Surface(err)) => {
                    match err {
                        SurfaceAcquireError::Lost | SurfaceAcquireError::Outdated => {
                            target.reconfigure_surface(gpu);
                            self.schedule_dirty_window(id, SURFACE_RETRY_DELAY);
                        }
                        SurfaceAcquireError::Timeout | SurfaceAcquireError::Occluded => {
                            crate::emu_log!("[UI] wgpu surface timeout for {:?}", id);
                            self.schedule_dirty_window(id, SURFACE_RETRY_DELAY);
                        }
                        SurfaceAcquireError::Validation => {
                            crate::emu_log!("[UI] wgpu surface error for {:?}", id);
                            self.schedule_dirty_window(id, SURFACE_RETRY_DELAY);
                        }
                    }
                    true
                }
            }
        };

        if rendered
            && hwnd.is_some_and(|handle| self.main_guest_hwnd == Some(handle))
            && !self.main_guest_window_forced_visible
            && let Some(window) = self.get_window(&id)
        {
            crate::emu_log!("[UI] host auto-show main HWND {:#x}", hwnd.unwrap_or(0));
            window.set_visible(true);
            self.main_guest_window_forced_visible = true;
        }

        rendered
    }

    /// 숨김 상태인 guest 창은 실제로 표시되기 전까지 렌더를 건너뜁니다.
    fn should_render_window(&self, id: WindowId) -> bool {
        let Some(hwnd) = self.id_to_hwnd.get(&id).copied() else {
            return true;
        };

        if self.main_guest_hwnd == Some(hwnd) {
            return true;
        }

        self.emu_context
            .win_event
            .lock()
            .unwrap()
            .windows
            .get(&hwnd)
            .map(|state| state.visible)
            .unwrap_or(true)
    }
}

impl ApplicationHandler<()> for WinFrame {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if trace_ui_enabled() {
            eprintln!(
                "[TRACE_UI] resumed initial_painters={}",
                self.initial_painters.len()
            );
        }
        if let Some(monitor) = event_loop.primary_monitor() {
            let position = monitor.position();
            let size = monitor.size();
            // 게스트 좌표계가 실제 화면 배치와 최대한 일치하도록 기본 작업 영역을 초기화합니다.
            self.emu_context.set_work_area(
                position.x,
                position.y,
                position.x + size.width as i32,
                position.y + size.height as i32,
            );
        }

        // 초기 페인터들을 위한 창 생성
        let mut initial_painters = std::mem::take(&mut self.initial_painters);
        for painter in initial_painters.drain(..) {
            if trace_ui_enabled() {
                eprintln!("[TRACE_UI] creating initial painter window");
            }
            let window = Arc::new(painter.create_window(event_loop));
            let id = window.id();
            if trace_ui_enabled() {
                eprintln!("[TRACE_UI] created window {:?}", id);
            }
            self.ensure_gpu_context().unwrap();
            let target = {
                let gpu = self.gpu_context.as_ref().unwrap();
                WindowRenderTarget::new_cpu_painter(gpu, window.clone(), painter).unwrap()
            };
            self.render_targets.insert(id, target);
            window.request_redraw();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        crate::UI_HEARTBEAT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut dirty_windows = self.process_ui_commands(event_loop);
        let next_dirty_deadline =
            self.collect_ready_guest_redraws(Instant::now(), &mut dirty_windows);
        self.poll_pending_message_dialogs();

        // 커서 애니메이션 프레임 전환 체크
        self.tick_cursor_animation(event_loop);

        // 모든 Painter에게 백그라운드 상태 변경 알림 및 종료 체크
        let mut windows_to_remove = Vec::new();
        for (id, target) in self.render_targets.iter_mut() {
            if target.tick() {
                dirty_windows.insert(*id);
            }
            if target.should_close() {
                windows_to_remove.push(*id);
            }
        }

        for id in windows_to_remove {
            self.remove_window(id);
        }

        for id in dirty_windows {
            if self.should_render_window(id)
                && !self.render_window(id)
                && let Some(window) = self.get_window(&id)
            {
                window.request_redraw();
            }
        }

        let dirty_interval =
            next_dirty_deadline.map(|deadline| deadline.saturating_duration_since(Instant::now()));
        let interval = match (self.next_poll_interval(), dirty_interval) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };

        if let Some(interval) = interval {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(
                Instant::now() + interval,
            ));
        } else {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if !self.render_targets.contains_key(&id) {
            return;
        }

        // 윈도우별 자체 이벤트 처리 위임 및 필요한 정보 추출
        let (handled, quit_on_close) = if let Some(target) = self.render_targets.get_mut(&id) {
            (
                target.handle_event(&event, event_loop),
                target.quit_on_close(),
            )
        } else {
            (false, false)
        };

        if handled && let Some(window) = self.get_window(&id) {
            window.request_redraw();
        }

        match event {
            WindowEvent::RedrawRequested => {
                // 호스트 redraw는 현재 guest 표면을 출력만 해야 합니다.
                // 여기서 다시 WM_PAINT를 만들면 EndPaint/UpdateWindow와 순환해 과도한 redraw가 발생합니다.
                self.render_window(id);
            }

            WindowEvent::CloseRequested => {
                let hwnd = self.id_to_hwnd.get(&id).copied();
                if let Some(hwnd) = hwnd {
                    let target_tid =
                        self.queue_message_for_window(hwnd, [hwnd, 0x0010, 0, 0, 0, 0, 0]); // WM_CLOSE
                    self.wake_emulator(Some(target_tid));
                }

                if self.should_exit_on_close_request(hwnd, quit_on_close) {
                    event_loop.exit();
                }
            }

            WindowEvent::CursorEntered { .. } => {
                if self.set_hovered_window(Some(id)) {
                    self.reapply_cursor_for_current_owner(event_loop);
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                if self.set_hovered_window(Some(id)) {
                    self.reapply_cursor_for_current_owner(event_loop);
                }

                let x = position.x as u32;
                let y = position.y as u32;

                if let Some(&hwnd) = self.id_to_hwnd.get(&id) {
                    self.emu_context
                        .mouse_x
                        .store(x, std::sync::atomic::Ordering::SeqCst);
                    self.emu_context
                        .mouse_y
                        .store(y, std::sync::atomic::Ordering::SeqCst);

                    let now = std::time::Instant::now();
                    if let Some((last_hwnd, last)) = self.last_cursor_moved
                        && last_hwnd == hwnd
                        && now.duration_since(last).as_millis() < 16
                    {
                        return; // 16ms 이내 스로틀링
                    }
                    self.last_cursor_moved = Some((hwnd, now));
                    self.last_sent_mouse_pos = Some((hwnd, x, y));

                    let time = crate::diagnostics::virtual_millis(self.emu_context.start_time);
                    let lparam = (y << 16) | (x & 0xFFFF);
                    let capture_hwnd = self
                        .emu_context
                        .capture_hwnd
                        .load(std::sync::atomic::Ordering::SeqCst);
                    let target_tid = self.emu_context.window_owner_thread_id(hwnd);
                    self.emu_context.with_thread_message_queue(target_tid, |q| {
                        // 단일 패스로 WM_SETCURSOR(0x0020)와 WM_MOUSEMOVE(0x0200) 인덱스를 동시에 탐색
                        let mut setcursor_idx = None;
                        let mut mousemove_idx = None;
                        for (i, m) in q.iter().enumerate() {
                            if m[0] == hwnd {
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
                                    message[0] = hwnd;
                                    message[2] = hwnd;
                                    message[3] = setcursor_lparam;
                                    message[4] = time;
                                    message[5] = x;
                                    message[6] = y;
                                }
                            } else {
                                let setcursor_message =
                                    [hwnd, 0x0020, hwnd, setcursor_lparam, time, x, y];
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
                                message[0] = hwnd;
                                message[3] = lparam;
                                message[4] = time;
                                message[5] = x;
                                message[6] = y;
                            }
                        } else {
                            q.push_back([hwnd, 0x0200, 0, lparam, time, x, y]);
                        }
                    });

                    // 마우스 트래킹 (TrackMouseEvent) 처리
                    let mut track_opt = self.emu_context.track_mouse_event.lock().unwrap();
                    if let Some(track) = track_opt.as_ref()
                        && track.hwnd != hwnd
                        && (track.flags & 0x00000002 != 0)
                    {
                        let time = crate::diagnostics::virtual_millis(self.emu_context.start_time);
                        let track_tid = self.queue_message_for_window(
                            track.hwnd,
                            [track.hwnd, 0x02A3, 0, 0, time, 0, 0],
                        ); // WM_MOUSELEAVE
                        *track_opt = None;
                        self.wake_emulator(Some(track_tid));
                    }
                    self.wake_emulator(Some(target_tid));
                }
            }

            WindowEvent::CursorLeft { .. } => {
                if self.hovered_window_id == Some(id) && self.set_hovered_window(None) {
                    self.reapply_cursor_for_current_owner(event_loop);
                }

                if let Some(&hwnd) = self.id_to_hwnd.get(&id) {
                    let mut track_opt = self.emu_context.track_mouse_event.lock().unwrap();
                    if let Some(track) = track_opt.clone()
                        && track.hwnd == hwnd
                        && (track.flags & 0x00000002 != 0)
                    {
                        let time = crate::diagnostics::virtual_millis(self.emu_context.start_time);
                        let target_tid = self.queue_message_for_window(
                            track.hwnd,
                            [track.hwnd, 0x02A3, 0, 0, time, 0, 0],
                        ); // WM_MOUSELEAVE
                        *track_opt = None;
                        self.wake_emulator(Some(target_tid));
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

                    // 스로틀링으로 인해 WM_MOUSEMOVE가 생략되었을 수 있으므로,
                    // 클릭 직전 현재 위치에 대한 이동 메시지 전송을 보장합니다.
                    let target_tid = self.emu_context.window_owner_thread_id(hwnd);
                    if self.last_sent_mouse_pos != Some((hwnd, x, y)) {
                        let time = crate::diagnostics::virtual_millis(self.emu_context.start_time);
                        let lparam = (y << 16) | (x & 0xFFFF);
                        self.emu_context.with_thread_message_queue(target_tid, |q| {
                            q.push_back([hwnd, 0x0200, 0, lparam, time, x, y]);
                        });
                        self.last_sent_mouse_pos = Some((hwnd, x, y));
                    }

                    let lparam = (y << 16) | (x & 0xFFFF);
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
                        let time = crate::diagnostics::virtual_millis(self.emu_context.start_time);
                        self.emu_context.with_thread_message_queue(target_tid, |q| {
                            q.push_back([hwnd, msg, wparam, lparam, time, x, y]);
                        });
                        self.wake_emulator(Some(target_tid));
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
                        // 0x80 이상 유니코드(한글 등)는 IME Commit 이벤트에서 처리합니다.
                        Key::Character(s) => {
                            let ch = s.chars().next().unwrap_or('\0');
                            if (ch as u32) < 0x80 { ch as u32 } else { 0 }
                        }
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
                        let target_tid =
                            self.queue_message_for_window(hwnd, [hwnd, msg, vk, 0, 0, 0, 0]);
                        self.wake_emulator(Some(target_tid));
                    }
                }
            }

            // IME로 조합 완성된 문자는 ANSI 앱 기준으로
            // ASCII는 `WM_CHAR`, DBCS(한글)는 `WM_IME_CHAR`로 전달합니다.
            WindowEvent::Ime(Ime::Commit(s)) => {
                if let Some(&hwnd) = self.id_to_hwnd.get(&id) {
                    let target_tid = self.emu_context.window_owner_thread_id(hwnd);
                    for ch in s.chars() {
                        let committed_messages = Self::committed_char_messages(ch);
                        self.emu_context.with_thread_message_queue(target_tid, |q| {
                            for &(message, value) in &committed_messages {
                                q.push_back([hwnd, message, value, 0, 0, 0, 0]);
                            }
                        });
                    }
                    self.wake_emulator(Some(target_tid));
                }
            }

            WindowEvent::Focused(focused) => {
                if focused {
                    if self.set_focused_window(Some(id)) && self.hovered_window_id.is_none() {
                        self.reapply_cursor_for_current_owner(event_loop);
                    }
                } else if self.focused_window_id == Some(id)
                    && self.set_focused_window(None)
                    && self.hovered_window_id.is_none()
                {
                    self.reapply_cursor_for_current_owner(event_loop);
                }

                if let Some(&hwnd) = self.id_to_hwnd.get(&id) {
                    let wparam = if focused { 1 } else { 0 }; // WA_ACTIVE, WA_INACTIVE
                    let target_tid =
                        self.queue_message_for_window(hwnd, [hwnd, 0x0006, wparam, 0, 0, 0, 0]); // WM_ACTIVATE

                    if focused {
                        self.emu_context
                            .focus_hwnd
                            .store(hwnd, std::sync::atomic::Ordering::SeqCst);
                        self.emu_context
                            .active_hwnd
                            .store(hwnd, std::sync::atomic::Ordering::SeqCst);
                    }
                    self.wake_emulator(Some(target_tid));
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
                        let target_tid =
                            self.queue_message_for_window(hwnd, [hwnd, 0x0003, 0, lparam, 0, 0, 0]); // WM_MOVE
                        self.wake_emulator(Some(target_tid));
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
                    if let Some(gpu) = self.gpu_context.as_ref()
                        && let Some(target) = self.render_targets.get_mut(&id)
                    {
                        target.reconfigure_surface(gpu);
                    }

                    let target_tid = self.emu_context.window_owner_thread_id(hwnd);
                    self.emu_context.with_thread_message_queue(target_tid, |q| {
                        enqueue_window_message(q, [hwnd, 0x0005, 0, lparam, 0, 0, 0]); // WM_SIZE (SIZE_RESTORED)
                    });
                    self.wake_emulator(Some(target_tid));
                    if let Some(window) = self.get_window(&id) {
                        window.request_redraw();
                    }
                }
            }

            _ => (),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        HostWindowStyle, WS_CAPTION, WS_EX_LAYERED, WS_SYSMENU, WinFrame, enqueue_window_message,
    };
    use crate::dll::win32::{Win32Context, WindowState};
    use winit::window::{WindowButtons, WindowId};

    const WS_POPUP: u32 = 0x8000_0000;

    #[test]
    fn duplicated_resize_messages_keep_only_latest_one() {
        let mut queue = std::collections::VecDeque::from([
            [0x1001, 0x0005, 0, 0x0010_0020, 0, 0, 0],
            [0x1001, 0x000F, 0, 0, 0, 0, 0],
            [0x1001, 0x0005, 0, 0x0030_0040, 0, 0, 0],
        ]);

        enqueue_window_message(&mut queue, [0x1001, 0x0005, 0, 0x0050_0060, 0, 0, 0]);

        assert_eq!(queue.len(), 2);
        assert_eq!(queue[0][1], 0x000F);
        assert_eq!(queue[1][1], 0x0005);
        assert_eq!(queue[1][3], 0x0050_0060);
    }

    fn sample_window_state(parent: u32, z_order: u32) -> WindowState {
        WindowState {
            class_name: "TEST".to_string(),
            class_icon: 0,
            big_icon: 0,
            small_icon: 0,
            class_hbr_background: 0,
            title: "test".to_string(),
            x: 0,
            y: 0,
            width: 100,
            height: 100,
            style: 0,
            ex_style: 0,
            owner_thread_id: 0,
            parent,
            id: 0,
            visible: true,
            enabled: true,
            zoomed: false,
            iconic: false,
            wnd_proc: 0,
            class_cursor: 0,
            user_data: 0,
            use_native_frame: true,
            surface_bitmap: 0,
            window_rgn: 0,
            guest_frame_left: 0,
            guest_frame_top: 0,
            guest_frame_right: 0,
            guest_frame_bottom: 0,
            guest_frame_exact: false,
            needs_paint: false,
            last_hittest_lparam: u32::MAX,
            last_hittest_result: 0,
            z_order,
        }
    }

    fn dummy_window_id(raw: u64) -> WindowId {
        WindowId::from(raw)
    }

    #[test]
    fn main_guest_window_keeps_close_fallback() {
        let (_tx, rx) = std::sync::mpsc::channel();
        let context = Win32Context::new(None);
        let mut frame = WinFrame::new(rx, Vec::new(), context);
        frame.main_guest_hwnd = Some(0x1000);

        assert!(frame.should_exit_on_close_request(Some(0x1000), false));
        assert!(!frame.should_exit_on_close_request(Some(0x1001), false));
        assert!(!frame.should_exit_on_close_request(None, false));
    }

    #[test]
    fn hovered_window_overrides_focused_window_for_cursor_owner() {
        let (_tx, rx) = std::sync::mpsc::channel();
        let context = Win32Context::new(None);
        let mut frame = WinFrame::new(rx, Vec::new(), context);
        let hovered = dummy_window_id(1);
        let focused = dummy_window_id(2);

        assert!(frame.set_focused_window(Some(focused)));
        assert_eq!(frame.current_cursor_owner(), Some(focused));

        assert!(frame.set_hovered_window(Some(hovered)));
        assert_eq!(frame.current_cursor_owner(), Some(hovered));

        assert!(frame.set_hovered_window(None));
        assert_eq!(frame.current_cursor_owner(), Some(focused));
    }

    #[test]
    fn focus_only_drives_cursor_owner_when_no_window_is_hovered() {
        let (_tx, rx) = std::sync::mpsc::channel();
        let context = Win32Context::new(None);
        let mut frame = WinFrame::new(rx, Vec::new(), context);
        let hovered = dummy_window_id(3);
        let focused = dummy_window_id(4);

        frame.set_hovered_window(Some(hovered));
        frame.set_focused_window(Some(focused));

        assert_eq!(frame.current_cursor_owner(), Some(hovered));
        assert!(!frame.is_cursor_owner(focused));
        assert!(frame.is_cursor_owner(hovered));
    }

    #[test]
    fn cached_cursor_is_isolated_per_window() {
        let (_tx, rx) = std::sync::mpsc::channel();
        let context = Win32Context::new(None);
        let mut frame = WinFrame::new(rx, Vec::new(), context.clone());
        let first = dummy_window_id(5);
        let second = dummy_window_id(6);

        frame.id_to_hwnd.insert(first, 0x1001);
        frame.id_to_hwnd.insert(second, 0x1002);
        frame.hwnd_to_id.insert(0x1001, first);
        frame.hwnd_to_id.insert(0x1002, second);

        let mut second_window = sample_window_state(0, 0);
        second_window.class_cursor = 0x2222;
        {
            let mut win_event = context.win_event.lock().unwrap();
            win_event.create_window(0x1001, sample_window_state(0, 0));
            win_event.create_window(0x1002, second_window);
        }

        frame.cache_window_cursor(first, 0x1111);

        assert_eq!(frame.effective_cursor_handle_for_window(first), 0x1111);
        assert_eq!(frame.effective_cursor_handle_for_window(second), 0x2222);
    }

    #[test]
    fn leaving_window_clears_hover_and_stale_cursor_animation() {
        let (_tx, rx) = std::sync::mpsc::channel();
        let context = Win32Context::new(None);
        let mut frame = WinFrame::new(rx, Vec::new(), context);
        let hovered = dummy_window_id(7);

        frame.hovered_window_id = Some(hovered);
        frame.current_cursor = Some((hovered, 0x1234));
        frame.cursor_anim = Some(super::CursorAnimState {
            hcursor: 0x1234,
            window_id: hovered,
            frame_index: 0,
            frame_count: 2,
            interval: std::time::Duration::from_millis(100),
            last_switch: std::time::Instant::now(),
        });

        assert!(frame.set_hovered_window(None));
        assert_eq!(frame.hovered_window_id, None);
        assert_eq!(frame.current_cursor, None);
        assert_eq!(frame.cursor_anim.as_ref().map(|anim| anim.window_id), None);
    }

    #[test]
    fn owned_popups_do_not_expand_host_focus_targets() {
        let (_tx, rx) = std::sync::mpsc::channel();
        let context = Win32Context::new(None);
        let mut frame = WinFrame::new(rx, Vec::new(), context.clone());
        let root_id = dummy_window_id(10);
        let first_popup_id = dummy_window_id(11);
        let second_popup_id = dummy_window_id(12);

        frame.hwnd_to_id.insert(0x1000, root_id);
        frame.hwnd_to_id.insert(0x1001, first_popup_id);
        frame.hwnd_to_id.insert(0x1002, second_popup_id);

        let mut first_popup = sample_window_state(0x1000, 1);
        first_popup.style = WS_POPUP;
        let mut second_popup = sample_window_state(0x1000, 2);
        second_popup.style = WS_POPUP;
        {
            let mut win_event = context.win_event.lock().unwrap();
            win_event.create_window(0x1000, sample_window_state(0, 0));
            win_event.create_window(0x1001, first_popup);
            win_event.create_window(0x1002, second_popup);
        }

        assert_eq!(frame.activation_focus_targets(0x1000), vec![0x1000]);
    }

    #[test]
    fn framed_popup_keeps_native_decorations() {
        let host_style =
            HostWindowStyle::from_guest(0x8000_0000 | WS_CAPTION | WS_SYSMENU, 0, true, false);

        assert!(host_style.decorations);
        assert!(host_style.enabled_buttons.contains(WindowButtons::CLOSE));
    }

    #[test]
    fn plain_popup_stays_undecorated() {
        let host_style = HostWindowStyle::from_guest(0x8000_0000, 0, true, false);

        assert!(!host_style.decorations);
        assert!(host_style.enabled_buttons.is_empty());
    }

    #[test]
    fn plain_popup_stays_opaque_without_region() {
        let host_style = HostWindowStyle::from_guest(WS_POPUP, 0, true, false);

        assert!(!host_style.transparent);
    }

    #[test]
    fn layered_popup_is_transparent() {
        let host_style = HostWindowStyle::from_guest(WS_POPUP, WS_EX_LAYERED, true, false);

        assert!(host_style.transparent);
    }

    #[test]
    fn region_popup_is_transparent() {
        let host_style = HostWindowStyle::from_guest(WS_POPUP, 0, true, true);

        assert!(host_style.transparent);
    }

    #[test]
    fn framed_popup_stays_opaque() {
        let host_style = HostWindowStyle::from_guest(WS_POPUP | WS_CAPTION, 0, true, false);

        assert!(!host_style.transparent);
    }

    #[test]
    fn ansi_commit_encoding_keeps_ascii_as_single_byte() {
        assert_eq!(WinFrame::encode_committed_char_for_ansi('A'), vec![0x41]);
    }

    #[test]
    fn ansi_commit_encoding_packs_hangul_into_single_wparam() {
        let packed = WinFrame::encode_committed_char_for_ansi('한');
        assert_eq!(packed.len(), 1);
        assert_eq!(packed[0], 0xD1C7);
    }
}
