use crate::{
    dll::win32::{GdiObject, Win32Context},
    ui::{Painter, UiCommand, WindowPositionMode, apply_platform_window_attributes},
};
use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use softbuffer::{Context as SoftContext, Surface};
use std::{collections::HashMap, num::NonZeroU32, sync::mpsc::Receiver};
#[cfg(target_os = "windows")]
use winit::platform::windows::{WindowAttributesExtWindows, WindowExtWindows};
#[cfg(target_os = "windows")]
use winit::raw_window_handle::RawWindowHandle;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::{ElementState, KeyEvent, MouseButton, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{Key, NamedKey},
    raw_window_handle::{DisplayHandle, HasDisplayHandle, HasWindowHandle},
    window::{Icon, Window, WindowAttributes, WindowButtons, WindowId, WindowLevel},
};

// Windows 스타일 -> winit 속성 매핑
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_POPUP: u32 = 0x8000_0000;
const WS_DLGFRAME: u32 = 0x0040_0000;
const WS_BORDER: u32 = 0x0080_0000;
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

type WinSurface = Surface<DisplayHandle<'static>, Window>;

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
    fn from_guest(style: u32, ex_style: u32, use_native_frame: bool) -> Self {
        // WS_POPUP 자체가 프레임을 금지하는 것은 아니므로,
        // 실제 장식 여부는 캡션/프레임 비트와 호스트 네이티브 프레임 사용 여부로만 결정합니다.
        let framing_bits = WS_CAPTION | WS_BORDER | WS_DLGFRAME | WS_THICKFRAME;
        let is_shaped_popup = (style & WS_POPUP) != 0 && (style & framing_bits) == 0;
        let decorations = use_native_frame && (style & WS_CAPTION) != 0;
        let resizable = use_native_frame && (style & WS_THICKFRAME) != 0;
        let transparent = (ex_style & WS_EX_LAYERED) != 0 || is_shaped_popup;
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
    /// 첫 번째 게스트 최상위 창 HWND
    main_guest_hwnd: Option<u32>,

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
}

impl WinFrame {
    fn activation_hwnds_in_order(&self, root_hwnd: u32) -> Vec<u32> {
        fn collect(
            windows: &HashMap<u32, crate::dll::win32::WindowState>,
            hwnd: u32,
            out: &mut Vec<u32>,
        ) {
            let Some(_) = windows.get(&hwnd) else {
                return;
            };

            out.push(hwnd);

            let mut children = windows
                .iter()
                .filter_map(|(&child_hwnd, state)| {
                    (state.parent == hwnd).then_some((child_hwnd, state.z_order))
                })
                .collect::<Vec<_>>();
            children.sort_unstable_by_key(|&(child_hwnd, z_order)| (z_order, child_hwnd));

            for (child_hwnd, _) in children {
                collect(windows, child_hwnd, out);
            }
        }

        let windows = {
            let win_event = self.emu_context.win_event.lock().unwrap();
            win_event.windows.clone()
        };
        let mut order = Vec::new();
        collect(&windows, root_hwnd, &mut order);
        order
    }

    fn activate_window_tree(&self, root_hwnd: u32) {
        for hwnd in self.activation_hwnds_in_order(root_hwnd) {
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

    fn apply_host_parent_link(
        &self,
        mut attributes: WindowAttributes,
        parent: u32,
        style: u32,
    ) -> WindowAttributes {
        let host_parent_link = Self::host_parent_link(parent, style);
        if host_parent_link == HostParentLink::None {
            return attributes;
        }

        let Some(parent_id) = self.hwnd_to_id.get(&parent) else {
            return attributes;
        };
        let Some(parent_window) = self.get_window(parent_id) else {
            return attributes;
        };

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

        #[cfg(not(target_os = "windows"))]
        {
            if let Ok(parent_handle) = parent_window.window_handle() {
                attributes = unsafe { attributes.with_parent_window(Some(parent_handle.as_raw())) };
            }
        }

        attributes
    }

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
            main_guest_hwnd: None,
            sb_context: None,
            emu_context: context,
            initial_painters,
            cursor_anim: None,
            current_cursor: None,
            hovered_window_id: None,
            focused_window_id: None,
            window_cursor_cache: HashMap::new(),
            last_cursor_moved: None,
            last_sent_mouse_pos: None,
        }
    }

    fn get_window(&self, id: &WindowId) -> Option<&Window> {
        self.surfaces.get(id).map(|s| s.window())
    }

    /// 첫 번째 게스트 최상위 창을 메인 창으로 기록합니다.
    fn register_main_guest_window(&mut self, hwnd: u32, style: u32) {
        if (style & WS_CHILD) == 0 && self.main_guest_hwnd.is_none() {
            self.main_guest_hwnd = Some(hwnd);
        }
    }

    /// 파괴 대상 HWND가 메인 게스트 창이면 종료 대상으로 판정하고 기록을 비웁니다.
    fn take_main_guest_window_close(&mut self, hwnd: u32) -> bool {
        if self.main_guest_hwnd == Some(hwnd) {
            self.main_guest_hwnd = None;
            true
        } else {
            false
        }
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
        self.surfaces.remove(&id);
        self.painters.remove(&id);
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

    #[allow(dead_code)]
    pub fn get_painter_mut<T: Painter + 'static>(
        &mut self,
        id: winit::window::WindowId,
    ) -> Option<&mut T> {
        self.painters
            .get_mut(&id)
            .and_then(|p| p.as_any_mut().downcast_mut::<T>())
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
    /// main_resume_time을 클리어하여 메인 스레드가 즉시 실행되게 합니다.
    fn wake_emulator(&self) {
        // 메인 스레드의 대기 시간을 해제하여 다음 루프에서 즉시 실행되도록 합니다.
        if let Ok(mut guard) = self.emu_context.main_resume_time.try_lock() {
            *guard = None;
        }
        if let Ok(guard) = self.emu_context.emu_thread.try_lock()
            && let Some(thread) = guard.as_ref()
        {
            thread.unpark();
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
                    let mut attributes = Window::default_attributes()
                        // 게스트가 다루는 좌표계는 픽셀 기반이므로 backing store 크기와 1:1로 맞춥니다.
                        .with_title(title)
                        .with_inner_size(PhysicalSize::new(width, height))
                        .with_min_inner_size(PhysicalSize::new(width, height))
                        .with_visible(visible);
                    attributes = apply_platform_window_attributes(attributes);
                    attributes = self.apply_host_parent_link(attributes, parent, style);

                    let class_icon = {
                        let win_event = self.emu_context.win_event.lock().unwrap();
                        win_event.windows.get(&hwnd).map(|state| {
                            if state.small_icon != 0 {
                                state.small_icon
                            } else if state.big_icon != 0 {
                                state.big_icon
                            } else {
                                state.class_icon
                            }
                        })
                    }
                    .unwrap_or(0);
                    attributes = attributes.with_window_icon(self.host_window_icon(class_icon));

                    if x != CW_USEDEFAULT && y != CW_USEDEFAULT {
                        let position = winit::dpi::PhysicalPosition::new(x, y);
                        attributes = match position_mode {
                            WindowPositionMode::Screen | WindowPositionMode::ParentClient => {
                                attributes.with_position(position)
                            }
                        };
                    }

                    attributes = Self::apply_guest_window_attributes(
                        attributes,
                        style,
                        ex_style,
                        use_native_frame,
                    );

                    let window = event_loop.create_window(attributes).unwrap();
                    let id = window.id();
                    if parent == 0 {
                        self.register_main_guest_window(hwnd, style);
                    }
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
                    let should_exit = self.take_main_guest_window_close(hwnd);
                    if let Some(id) = self.hwnd_to_id.get(&hwnd).copied() {
                        self.remove_window(id);
                    }
                    if should_exit {
                        event_loop.exit();
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
                        && let Some(window) = self.get_window(id)
                    {
                        window.request_redraw();
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
                        self.cache_window_cursor(id, hcursor);
                        if self.is_cursor_owner(id) {
                            self.apply_effective_cursor_for_window(id, event_loop);
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

        if handled && let Some(window) = self.get_window(&id) {
            window.request_redraw();
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

                        if let Some(painter) = self.painters.get_mut(&id)
                            && painter.paint(&mut buffer, width, height)
                        {
                            buffer.present().unwrap();
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
                let should_exit_main = hwnd
                    .map(|handle| self.take_main_guest_window_close(handle))
                    .unwrap_or(false);

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
                    if should_exit_main {
                        event_loop.exit();
                    }
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

                    let time = self.emu_context.start_time.elapsed().as_millis() as u32;
                    let lparam = (y << 16) | (x & 0xFFFF);
                    let capture_hwnd = self
                        .emu_context
                        .capture_hwnd
                        .load(std::sync::atomic::Ordering::SeqCst);
                    let mut q = self.emu_context.message_queue.lock().unwrap();

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

                    // 마우스 트래킹 (TrackMouseEvent) 처리
                    let mut track_opt = self.emu_context.track_mouse_event.lock().unwrap();
                    if let Some(track) = track_opt.as_ref()
                        && track.hwnd != hwnd
                        && (track.flags & 0x00000002 != 0)
                    {
                        let time = self.emu_context.start_time.elapsed().as_millis() as u32;
                        q.push_back([track.hwnd, 0x02A3, 0, 0, time, 0, 0]); // WM_MOUSELEAVE
                        *track_opt = None;
                    }
                    self.wake_emulator();
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
                        let time = self.emu_context.start_time.elapsed().as_millis() as u32;
                        let mut q = self.emu_context.message_queue.lock().unwrap();
                        q.push_back([track.hwnd, 0x02A3, 0, 0, time, 0, 0]); // WM_MOUSELEAVE
                        *track_opt = None;
                        self.wake_emulator();
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
                    if self.last_sent_mouse_pos != Some((hwnd, x, y)) {
                        let time = self.emu_context.start_time.elapsed().as_millis() as u32;
                        let lparam = (y << 16) | (x & 0xFFFF);
                        let mut q = self.emu_context.message_queue.lock().unwrap();
                        q.push_back([hwnd, 0x0200, 0, lparam, time, x, y]);
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
                        let time = self.emu_context.start_time.elapsed().as_millis() as u32;
                        let mut q = self.emu_context.message_queue.lock().unwrap();
                        q.push_back([hwnd, msg, wparam, lparam, time, x, y]);
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
                    drop(q);
                    if focused {
                        self.activate_window_tree(hwnd);
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
                    enqueue_window_message(&mut q, [hwnd, 0x0005, 0, lparam, 0, 0, 0]); // WM_SIZE (SIZE_RESTORED)
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
}

impl DefaultEmulatorPainter {
    /// 윈도우 영역(SetWindowRgn) 외부의 픽셀을 0(검정/투명)으로 마스킹합니다.
    /// 호스트 native frame이 alpha=0을 투명으로 처리하지 못하더라도,
    /// 최소한 영역 외부에는 그리지 않도록 보장합니다.
    fn apply_window_rgn_mask(
        buffer: &mut [u32],
        width: u32,
        height: u32,
        rects: &[(i32, i32, i32, i32)],
    ) {
        for y in 0..height as i32 {
            let row_offset = (y as u32 * width) as usize;
            for x in 0..width as i32 {
                if !crate::ui::gdi_renderer::GdiRenderer::point_in_clip_rects(rects, x, y) {
                    let idx = row_offset + x as usize;
                    if idx < buffer.len() {
                        buffer[idx] = 0;
                    }
                }
            }
        }
    }

    /// 비트맵 하나를 현재 윈도우 버퍼에 그대로 복사합니다.
    fn blit_bitmap(
        buffer: &mut [u32],
        buffer_width: u32,
        buffer_height: u32,
        pixels: &[u32],
        src_width: u32,
        src_height: u32,
    ) {
        for src_y in 0..src_height as i32 {
            let dst_y = src_y;
            if dst_y < 0 || dst_y >= buffer_height as i32 {
                continue;
            }

            let src_row_offset = (src_y as u32 * src_width) as usize;
            let dst_row_offset = (dst_y as u32 * buffer_width) as usize;

            for src_x in 0..src_width as i32 {
                let dst_x = src_x;
                if dst_x < 0 || dst_x >= buffer_width as i32 {
                    continue;
                }

                let dst_idx = dst_row_offset + dst_x as usize;
                let src_idx = src_row_offset + src_x as usize;
                if src_idx < pixels.len() && dst_idx < buffer.len() {
                    buffer[dst_idx] = pixels[src_idx];
                }
            }
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
        // 윈도우에 SetWindowRgn으로 비직사각형 영역이 설정되어 있으면 해당 rect 목록을 미리 복사한다.
        // gdi_objects 락과 win_event 락을 동시에 보유하지 않도록 먼저 win_event에서 rect 리스트만 추출.
        let rgn_handle: u32 = match self.emu_context.win_event.try_lock() {
            Ok(win_event) => win_event
                .windows
                .get(&self.hwnd)
                .map(|w| w.window_rgn)
                .unwrap_or(0),
            Err(_) => 0,
        };

        let gdi_objects = match self.emu_context.gdi_objects.try_lock() {
            Ok(g) => g,
            Err(_) => return false, // 락 획득 실패 시 이번 프레임은 건너뜀 (데드락 방지)
        };

        let region_rects: Option<Vec<(i32, i32, i32, i32)>> = if rgn_handle != 0 {
            match gdi_objects.get(&rgn_handle) {
                Some(GdiObject::Region { rects }) => Some(rects.clone()),
                _ => None,
            }
        } else {
            None
        };

        if let Some(GdiObject::Bitmap {
            pixels: p,
            width: sw,
            height: sh,
            ..
        }) = gdi_objects.get(&self.surface_bitmap)
        {
            let p = p.lock().unwrap();
            Self::blit_bitmap(buffer, width, height, &p, *sw, *sh);
        }

        drop(gdi_objects);

        if let Some(rects) = region_rects {
            Self::apply_window_rgn_mask(buffer, width, height, &rects);
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

#[cfg(test)]
mod tests {
    use super::{
        HostWindowStyle, WS_CAPTION, WS_POPUP, WS_SYSMENU, WinFrame, enqueue_window_message,
    };
    use crate::dll::win32::{Win32Context, WindowState};
    use winit::window::{WindowButtons, WindowId};

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
    fn activation_order_visits_children_from_lower_z_to_higher_z() {
        let (_tx, rx) = std::sync::mpsc::channel();
        let context = Win32Context::new(None);
        {
            let mut win_event = context.win_event.lock().unwrap();
            win_event.create_window(0x1000, sample_window_state(0, 0));
            win_event.create_window(0x1001, sample_window_state(0x1000, 2));
            win_event.create_window(0x1002, sample_window_state(0x1000, 1));
            win_event.create_window(0x1003, sample_window_state(0x1002, 0));
        }

        let frame = WinFrame::new(rx, Vec::new(), context);
        assert_eq!(
            frame.activation_hwnds_in_order(0x1000),
            vec![0x1000, 0x1002, 0x1003, 0x1001]
        );
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
    fn framed_popup_keeps_native_decorations() {
        let host_style =
            HostWindowStyle::from_guest(0x8000_0000 | WS_CAPTION | WS_SYSMENU, 0, true);

        assert!(host_style.decorations);
        assert!(host_style.enabled_buttons.contains(WindowButtons::CLOSE));
    }

    #[test]
    fn plain_popup_stays_undecorated() {
        let host_style = HostWindowStyle::from_guest(0x8000_0000, 0, true);

        assert!(!host_style.decorations);
        assert!(host_style.enabled_buttons.is_empty());
    }

    #[test]
    fn plain_popup_is_transparent_for_region_clipping() {
        let host_style = HostWindowStyle::from_guest(WS_POPUP, 0, true);

        assert!(host_style.transparent);
    }

    #[test]
    fn framed_popup_stays_opaque() {
        let host_style = HostWindowStyle::from_guest(WS_POPUP | WS_CAPTION, 0, true);

        assert!(!host_style.transparent);
    }
}
