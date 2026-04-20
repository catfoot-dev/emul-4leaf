use crate::{
    dll::win32::{WindowState, user32::USER32},
    ui::{UiCommand, WindowPositionMode},
};
use std::{
    collections::HashMap,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
};

static UI_WAKE_PROXY: OnceLock<winit::event_loop::EventLoopProxy<()>> = OnceLock::new();
static UI_WAKE_PENDING: AtomicBool = AtomicBool::new(false);
const WS_CHILD: u32 = 0x40000000;
const SW_HIDE: u32 = 0;
const SW_SHOWNORMAL: u32 = 1;
const SW_SHOWMINIMIZED: u32 = 2;
const SW_SHOWMAXIMIZED: u32 = 3;
const SW_SHOW: u32 = 5;
const SW_MINIMIZE: u32 = 6;
const SW_SHOWMINNOACTIVE: u32 = 7;
const SW_SHOWNA: u32 = 8;
const SW_RESTORE: u32 = 9;
const SW_SHOWDEFAULT: u32 = 10;

fn wake_ui_event_loop() {
    if let Some(proxy) = UI_WAKE_PROXY.get() {
        if !UI_WAKE_PENDING.swap(true, Ordering::SeqCst) {
            let _ = proxy.send_event(());
        }
    }
}

/// 에뮬레이터 사이드에서 윈도우 객체들을 관리하는 추상화 레이어입니다.
/// 실제 winit 윈도우 조작은 UiCommand 채널을 통해 UI 스레드에 요청합니다.
pub struct WinEvent {
    /// 가상 HWND 핸들 -> 윈도우 상태 맵
    pub windows: HashMap<u32, WindowState>,
    /// UI 스레드와의 통신 채널
    ui_tx: Option<Sender<UiCommand>>,
    /// 윈도우 상태 변경 시 증가하는 세대 카운터 (paint 최적화용)
    pub generation: u64,
}

impl WinEvent {
    /// 지정된 창의 비클라이언트 inset을 USER32 규칙으로 계산합니다.
    fn window_frame_metrics(&self, hwnd: u32) -> Option<(i32, i32)> {
        let state = self.windows.get(&hwnd)?;
        let metrics = USER32::get_window_frame_metrics(state);
        Some((metrics.left, metrics.top))
    }

    /// 지정된 창이 부모 클라이언트 좌표계를 따르는 진짜 child 창인지 판정합니다.
    fn uses_parent_client_coordinates(&self, hwnd: u32) -> bool {
        self.windows
            .get(&hwnd)
            .map(|state| state.parent != 0 && (state.style & WS_CHILD) != 0)
            .unwrap_or(false)
    }

    /// 지정된 부모 창의 직계 자식 HWND 목록을 안정적인 Z 순서로 반환합니다.
    fn child_windows(&self, parent_hwnd: u32) -> Vec<u32> {
        let mut children: Vec<_> = self
            .windows
            .iter()
            .filter_map(|(&hwnd, state)| {
                (state.parent == parent_hwnd).then_some((hwnd, state.z_order))
            })
            .collect();
        children.sort_unstable_by_key(|&(hwnd, z_order)| (z_order, hwnd));
        children.into_iter().map(|(hwnd, _)| hwnd).collect()
    }

    /// 지정된 창과 모든 자손 창을 부모부터 자식 순서로 반환합니다.
    fn window_subtree_preorder(&self, hwnd: u32) -> Vec<u32> {
        fn collect(win_event: &WinEvent, hwnd: u32, out: &mut Vec<u32>) {
            if !win_event.windows.contains_key(&hwnd) {
                return;
            }

            out.push(hwnd);
            for child_hwnd in win_event.child_windows(hwnd) {
                collect(win_event, child_hwnd, out);
            }
        }

        let mut result = Vec::new();
        collect(self, hwnd, &mut result);
        result
    }

    /// 지정된 창과 모든 자손 창을 자식부터 부모 순서로 반환합니다.
    pub fn window_subtree_postorder(&self, hwnd: u32) -> Vec<u32> {
        fn collect(win_event: &WinEvent, hwnd: u32, out: &mut Vec<u32>) {
            for child_hwnd in win_event.child_windows(hwnd) {
                collect(win_event, child_hwnd, out);
            }
            if win_event.windows.contains_key(&hwnd) {
                out.push(hwnd);
            }
        }

        let mut result = Vec::new();
        collect(self, hwnd, &mut result);
        result
    }

    /// 지정된 창의 바깥 사각형 좌상단 화면 좌표를 부모 체인을 따라 누적해 계산합니다.
    pub fn window_screen_origin(&self, hwnd: u32) -> Option<(i32, i32)> {
        let mut current = hwnd;
        let mut x = 0i32;
        let mut y = 0i32;

        for _ in 0..=self.windows.len() {
            let state = self.windows.get(&current)?;
            x += state.x;
            y += state.y;

            if !self.uses_parent_client_coordinates(current) {
                return Some((x, y));
            }

            let parent = state.parent;
            let (parent_client_left, parent_client_top) = self.window_frame_metrics(parent)?;
            x += parent_client_left;
            y += parent_client_top;

            current = parent;
        }

        None
    }

    /// 지정된 창의 클라이언트 원점 화면 좌표를 계산합니다.
    pub fn client_screen_origin(&self, hwnd: u32) -> Option<(i32, i32)> {
        let (x, y) = self.window_screen_origin(hwnd)?;
        let (client_left, client_top) = self.window_frame_metrics(hwnd)?;
        Some((x + client_left, y + client_top))
    }

    /// 지정된 창이 조상 가시성까지 포함해 실제로 호스트에 보여야 하는지 계산합니다.
    fn effective_host_visibility(&self, hwnd: u32) -> bool {
        let mut current = hwnd;

        for _ in 0..=self.windows.len() {
            let Some(state) = self.windows.get(&current) else {
                return false;
            };

            if !state.visible {
                return false;
            }

            if !self.uses_parent_client_coordinates(current) {
                return true;
            }

            current = state.parent;
        }

        false
    }

    /// 지정된 창의 호스트 위치 좌표계와 좌표를 계산합니다.
    fn host_window_origin(&self, hwnd: u32) -> Option<(WindowPositionMode, i32, i32)> {
        let state = self.windows.get(&hwnd)?;
        if self.uses_parent_client_coordinates(hwnd) {
            Some((WindowPositionMode::ParentClient, state.x, state.y))
        } else {
            let (x, y) = self.window_screen_origin(hwnd)?;
            Some((WindowPositionMode::Screen, x, y))
        }
    }

    /// 지정된 창 하나의 실제 호스트 위치와 크기를 UI 스레드에 동기화합니다.
    fn sync_host_geometry(&self, hwnd: u32) {
        let Some(state) = self.windows.get(&hwnd) else {
            return;
        };
        let Some((position_mode, x, y)) = self.host_window_origin(hwnd) else {
            return;
        };

        self.send_ui_command(UiCommand::MoveWindow {
            hwnd,
            x,
            y,
            position_mode,
            width: state.width.max(0) as u32,
            height: state.height.max(0) as u32,
        });
    }

    /// 지정된 subtree 전체의 실제 표시 상태를 UI 스레드에 동기화합니다.
    fn sync_subtree_host_visibility(&self, hwnd: u32) {
        for handle in self.window_subtree_preorder(hwnd) {
            self.send_ui_command(UiCommand::ShowWindow {
                hwnd: handle,
                visible: self.effective_host_visibility(handle),
            });
        }
    }

    /// UI 이벤트 루프를 깨우기 위한 프록시를 등록합니다.
    pub fn install_wake_proxy(proxy: winit::event_loop::EventLoopProxy<()>) {
        let _ = UI_WAKE_PROXY.set(proxy);
    }

    /// UI 스레드가 큐 적재를 처리하기 시작할 때 wake 보류 상태를 해제합니다.
    pub(crate) fn clear_wake_pending() {
        UI_WAKE_PENDING.store(false, Ordering::SeqCst);
    }

    /// 다른 스레드에서 UI 이벤트 루프를 깨웁니다.
    pub fn notify_wakeup() {
        wake_ui_event_loop();
    }

    /// 새로운 윈도우 이벤트 관리기를 생성합니다.
    pub fn new(ui_tx: Option<Sender<UiCommand>>) -> Self {
        Self {
            windows: HashMap::new(),
            ui_tx,
            generation: 0,
        }
    }

    /// UI 스레드에 임의의 커맨드를 전송합니다.
    pub fn send_ui_command(&self, command: UiCommand) {
        if let Some(tx) = &self.ui_tx {
            let _ = tx.send(command);
            wake_ui_event_loop();
        }
    }

    /// 윈도우 상태 변경을 알리는 세대 카운터를 증가시킵니다.
    #[inline]
    pub fn bump_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    /// 새 윈도우 상태를 등록합니다.
    pub fn create_window(&mut self, hwnd: u32, state: WindowState) {
        self.windows.insert(hwnd, state);
        self.bump_generation();
    }

    /// 이미 등록된 윈도우 상태를 바탕으로 UI 스레드에 실제 창 생성을 요청합니다.
    pub fn realize_window(&mut self, hwnd: u32) {
        let Some(state) = self.windows.get(&hwnd) else {
            return;
        };

        let title = state.title.clone();
        let (position_mode, x, y) =
            self.host_window_origin(hwnd)
                .unwrap_or((WindowPositionMode::Screen, state.x, state.y));
        let width = state.width.max(0) as u32;
        let height = state.height.max(0) as u32;
        let style = state.style;
        let ex_style = state.ex_style;
        let parent = state.parent;
        let visible = self.effective_host_visibility(hwnd);
        let use_native_frame = state.use_native_frame;
        let surface_bitmap = state.surface_bitmap;

        crate::emu_log!(
            "[UI] realize_window: HWND {:#x} class=\"{}\" title=\"{}\" visible={} size={}x{} parent={:#x} mode={:?} pos=({}, {})",
            hwnd,
            state.class_name,
            state.title,
            visible,
            width,
            height,
            parent,
            position_mode,
            x,
            y
        );

        self.send_ui_command(UiCommand::CreateWindow {
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
        });
    }

    /// 윈도우 파괴를 상태와 UI 양쪽에 반영합니다.
    pub fn destroy_window(&mut self, hwnd: u32) {
        let subtree = self.window_subtree_postorder(hwnd);
        for handle in &subtree {
            self.windows.remove(handle);
        }
        self.bump_generation();

        for handle in subtree {
            self.send_ui_command(UiCommand::DestroyWindow { hwnd: handle });
        }
    }

    /// 윈도우 크기 변경 시 상태를 업데이트합니다.
    pub fn resize_window(&mut self, hwnd: u32, width: u32, height: u32) {
        if let Some(state) = self.windows.get_mut(&hwnd) {
            state.width = width as i32;
            state.height = height as i32;
            state.last_hittest_lparam = u32::MAX;
            self.bump_generation();
        }
    }

    /// 특정 핸들의 윈도우 상태를 가져옵니다.
    pub fn get_window_mut(&mut self, hwnd: u32) -> Option<&mut WindowState> {
        self.windows.get_mut(&hwnd)
    }

    /// 윈도우 표시 상태를 변경하고 subtree 전체의 실제 표시 상태를 동기화합니다.
    pub fn show_window(&mut self, hwnd: u32, visible: bool) {
        if let Some(state) = self.windows.get_mut(&hwnd) {
            state.visible = visible;
            self.bump_generation();
        }
        self.sync_subtree_host_visibility(hwnd);
    }

    /// `ShowWindow` 명령값을 Win32 의미에 맞춰 윈도우 상태에 반영합니다.
    pub fn apply_show_window(&mut self, hwnd: u32, n_cmd_show: u32) {
        match n_cmd_show {
            SW_HIDE => self.show_window(hwnd, false),
            SW_SHOWMINIMIZED | SW_MINIMIZE | SW_SHOWMINNOACTIVE => {
                self.show_window(hwnd, true);
                self.minimize_window(hwnd);
            }
            SW_SHOWMAXIMIZED => {
                self.show_window(hwnd, true);
                self.maximize_window(hwnd);
            }
            SW_SHOWNORMAL | SW_RESTORE => {
                self.show_window(hwnd, true);
                self.restore_window(hwnd);
            }
            SW_SHOW | SW_SHOWNA | SW_SHOWDEFAULT => self.show_window(hwnd, true),
            _ => self.show_window(hwnd, n_cmd_show != 0),
        }
    }

    /// 윈도우 최소화 상태를 변경하고 UI에 알립니다.
    pub fn minimize_window(&mut self, hwnd: u32) {
        if let Some(state) = self.windows.get_mut(&hwnd) {
            state.iconic = true;
            state.zoomed = false;
        }
        self.send_ui_command(UiCommand::MinimizeWindow { hwnd });
    }

    /// 윈도우 최대화 상태를 변경하고 UI에 알립니다.
    pub fn maximize_window(&mut self, hwnd: u32) {
        if let Some(state) = self.windows.get_mut(&hwnd) {
            state.iconic = false;
            state.zoomed = true;
        }
        self.send_ui_command(UiCommand::MaximizeWindow { hwnd });
    }

    /// 윈도우 일반 상태를 변경하고 UI에 알립니다.
    pub fn restore_window(&mut self, hwnd: u32) {
        if let Some(state) = self.windows.get_mut(&hwnd) {
            state.iconic = false;
            state.zoomed = false;
        }
        self.send_ui_command(UiCommand::RestoreWindow { hwnd });
    }

    /// 윈도우 위치와 크기를 바꾸고 해당 창의 호스트 기하를 동기화합니다.
    pub fn move_window(&mut self, hwnd: u32, x: i32, y: i32, width: u32, height: u32) {
        if let Some(state) = self.windows.get_mut(&hwnd) {
            state.x = x;
            state.y = y;
            state.width = width as i32;
            state.height = height as i32;
            self.bump_generation();
        }
        self.sync_host_geometry(hwnd);
    }

    /// 호스트 OS 창 이동 결과를 내부 상태에 반영합니다.
    /// 최상위 창만 진실값으로 받아들이고 자식 창은 무시합니다.
    pub fn sync_host_window_position(&mut self, hwnd: u32, x: i32, y: i32) -> bool {
        let Some(state) = self.windows.get_mut(&hwnd) else {
            return false;
        };

        if state.parent != 0 && (state.style & WS_CHILD) != 0 {
            return false;
        }

        if state.x == x && state.y == y {
            return false;
        }

        state.x = x;
        state.y = y;
        state.last_hittest_lparam = u32::MAX;
        self.bump_generation();
        true
    }

    /// 윈도우 크기, 위치 및 Z 순서를 바꾸고 UI에 동기화합니다.
    #[allow(clippy::too_many_arguments)]
    pub fn set_window_pos(
        &mut self,
        hwnd: u32,
        insert_after: u32,
        x: u32,
        y: u32,
        cx: u32,
        cy: u32,
        flags: u32,
    ) {
        let mut visibility_changed = false;
        let parent = self.windows.get(&hwnd).map(|s| s.parent).unwrap_or(0);
        if flags & 0x0004 == 0 {
            let mut siblings = self
                .windows
                .iter()
                .filter_map(|(handle, state)| {
                    (state.parent == parent && *handle != hwnd).then_some((*handle, state.z_order))
                })
                .collect::<Vec<_>>();
            siblings.sort_unstable_by_key(|&(handle, z_order)| (z_order, handle));

            let insert_index = if insert_after == 0 {
                siblings.len()
            } else if insert_after == 1 {
                0
            } else if let Some(idx) = siblings
                .iter()
                .position(|&(handle, _)| handle == insert_after)
            {
                idx + 1
            } else {
                siblings.len()
            };

            siblings.insert(insert_index.min(siblings.len()), (hwnd, 0));

            for (index, (handle, _)) in siblings.into_iter().enumerate() {
                if let Some(state) = self.windows.get_mut(&handle) {
                    state.z_order = index as u32;
                }
            }
        }

        let Some(state) = self.windows.get_mut(&hwnd) else {
            return;
        };
        if flags & 0x0002 == 0 {
            state.x = x as i32;
            state.y = y as i32;
        }
        if flags & 0x0001 == 0 {
            state.width = cx as i32;
            state.height = cy as i32;
        }

        // 자식 창은 고전 UI 엔진이 SetWindowPos를 레이아웃/클리핑 보조로도 사용하므로
        // HIDE는 그대로 반영하지 않고, SHOW만 반영합니다.
        if flags & 0x0040 != 0 && !state.visible {
            state.visible = true;
            visibility_changed = true;
        }

        self.bump_generation();
        if visibility_changed {
            self.sync_subtree_host_visibility(hwnd);
        }
        self.sync_host_geometry(hwnd);
    }

    /// 윈도우 제목을 변경하고 UI에 알립니다.
    pub fn set_window_text(&mut self, hwnd: u32, text: String) {
        if let Some(state) = self.windows.get_mut(&hwnd) {
            state.title = text.clone();
        }
        self.send_ui_command(UiCommand::SetWindowText { hwnd, text });
    }

    /// 윈도우의 큰/작은 아이콘 상태를 갱신하고 UI에도 반영합니다.
    pub fn set_window_icon(&mut self, hwnd: u32, icon_type: u32, hicon: u32) -> u32 {
        let mut old_icon = 0;
        let mut next_effective_icon = 0;
        let mut should_notify = false;

        if let Some(state) = self.windows.get_mut(&hwnd) {
            let prev_effective_icon = if state.small_icon != 0 {
                state.small_icon
            } else if state.big_icon != 0 {
                state.big_icon
            } else {
                state.class_icon
            };

            if icon_type == 0 {
                old_icon = state.small_icon;
                state.small_icon = hicon;
            } else {
                old_icon = state.big_icon;
                state.big_icon = hicon;
            }

            next_effective_icon = if state.small_icon != 0 {
                state.small_icon
            } else if state.big_icon != 0 {
                state.big_icon
            } else {
                state.class_icon
            };

            should_notify = prev_effective_icon != next_effective_icon;
            self.bump_generation();
        }

        if should_notify {
            self.send_ui_command(UiCommand::SetWindowIcon {
                hwnd,
                hicon: next_effective_icon,
            });
        }

        old_icon
    }

    /// 윈도우의 특정 영역을 무효화하고 다시 그리도록 요청합니다.
    pub fn invalidate_rect(&mut self, hwnd: u32, _rect: *mut std::ffi::c_void) {
        if let Some(state) = self.windows.get_mut(&hwnd) {
            state.needs_paint = true;
            self.update_window(hwnd);
        }
    }

    /// 윈도우의 유효성을 검사하여 다시 그리기 요청을 해제합니다.
    pub fn validate_window(&mut self, hwnd: u32) {
        if let Some(state) = self.windows.get_mut(&hwnd) {
            state.needs_paint = false;
        }
    }

    /// 윈도우 강제 다시 그리기 요청을 UI에 전달합니다.
    pub fn update_window(&self, hwnd: u32) {
        self.send_ui_command(UiCommand::UpdateWindow { hwnd });
    }

    /// 메시지 박스를 표시하고 응답을 대기합니다.
    pub fn message_box(&mut self, caption: String, text: String, u_type: u32) -> i32 {
        let (tx, rx) = std::sync::mpsc::channel();
        self.send_ui_command(UiCommand::MessageBox {
            caption,
            text,
            u_type,
            response_tx: tx,
        });

        rx.recv().unwrap_or(1)
    }

    /// 윈도우 표시 여부를 반환합니다.
    pub fn is_window_visible(&self, hwnd: u32) -> bool {
        self.windows.get(&hwnd).map(|w| w.visible).unwrap_or(false)
    }

    /// 윈도우 활성화 여부를 반환합니다.
    pub fn is_window_enabled(&self, hwnd: u32) -> bool {
        self.windows.get(&hwnd).map(|w| w.enabled).unwrap_or(false)
    }

    /// 윈도우 닫기 요청을 UI에 전달합니다.
    pub fn close_window(&mut self, hwnd: u32) {
        self.minimize_window(hwnd);
    }

    /// 윈도우 활성화/비활성화 상태를 UI와 동기화합니다.
    pub fn enable_window(&mut self, hwnd: u32, enable: bool) -> bool {
        if let Some(state) = self.windows.get_mut(&hwnd) {
            state.enabled = enable;
            self.send_ui_command(UiCommand::EnableWindow {
                hwnd,
                enabled: enable,
            });
            true
        } else {
            false
        }
    }

    /// 윈도우 활성화를 UI에 요청합니다.
    pub fn activate_window(&mut self, hwnd: u32) {
        self.send_ui_command(UiCommand::ActivateWindow { hwnd });
    }

    /// 윈도우 스타일과 확장 스타일을 UI 스레드와 동기화합니다.
    pub fn sync_window_style(&mut self, hwnd: u32) {
        let Some(state) = self.windows.get(&hwnd) else {
            return;
        };

        self.send_ui_command(UiCommand::SyncWindowStyle {
            hwnd,
            style: state.style,
            ex_style: state.ex_style,
        });
    }

    /// 윈도우 드래그 모드를 시작합니다.
    pub fn drag_window(&mut self, hwnd: u32) {
        self.send_ui_command(UiCommand::DragWindow { hwnd });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_window_state() -> WindowState {
        WindowState {
            class_name: "TEST".to_string(),
            class_icon: 0,
            big_icon: 0,
            small_icon: 0,
            class_hbr_background: 0,
            title: "test".to_string(),
            x: 0,
            y: 0,
            width: 640,
            height: 480,
            style: 0,
            ex_style: 0,
            owner_thread_id: 0,
            parent: 0,
            id: 0,
            visible: false,
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
            z_order: 0,
        }
    }

    #[test]
    fn apply_show_window_marks_maximized_windows_as_zoomed() {
        let mut win_event = WinEvent::new(None);
        win_event.create_window(0x1000, sample_window_state());

        win_event.apply_show_window(0x1000, SW_SHOWMAXIMIZED);

        let state = win_event.windows.get(&0x1000).unwrap();
        assert!(state.visible);
        assert!(state.zoomed);
        assert!(!state.iconic);
    }

    #[test]
    fn apply_show_window_restore_clears_minimized_and_maximized_flags() {
        let mut win_event = WinEvent::new(None);
        let mut state = sample_window_state();
        state.visible = true;
        state.zoomed = true;
        state.iconic = true;
        win_event.create_window(0x1000, state);

        win_event.apply_show_window(0x1000, SW_RESTORE);

        let state = win_event.windows.get(&0x1000).unwrap();
        assert!(state.visible);
        assert!(!state.zoomed);
        assert!(!state.iconic);
    }

    #[test]
    fn window_screen_origin_includes_parent_client_inset_for_child_windows() {
        let mut win_event = WinEvent::new(None);

        let mut parent = sample_window_state();
        parent.x = 100;
        parent.y = 200;
        parent.ex_style = 0x0000_0200;
        parent.use_native_frame = false;
        win_event.create_window(0x1000, parent);

        let mut child = sample_window_state();
        child.style = WS_CHILD;
        child.parent = 0x1000;
        child.x = 10;
        child.y = 20;
        win_event.create_window(0x1001, child);

        let mut grandchild = sample_window_state();
        grandchild.style = WS_CHILD;
        grandchild.parent = 0x1001;
        grandchild.x = 3;
        grandchild.y = 4;
        win_event.create_window(0x1002, grandchild);

        assert_eq!(win_event.window_screen_origin(0x1001), Some((112, 222)));
        assert_eq!(win_event.window_screen_origin(0x1002), Some((115, 226)));
        assert_eq!(win_event.client_screen_origin(0x1000), Some((102, 202)));
    }

    #[test]
    fn owned_popup_keeps_screen_coordinates_without_parent_offset() {
        let mut win_event = WinEvent::new(None);

        let mut parent = sample_window_state();
        parent.x = 100;
        parent.y = 200;
        win_event.create_window(0x1000, parent);

        let mut popup = sample_window_state();
        popup.parent = 0x1000;
        popup.x = 300;
        popup.y = 400;
        popup.style = 0x8000_0000;
        win_event.create_window(0x1001, popup);

        assert_eq!(win_event.window_screen_origin(0x1001), Some((300, 400)));
    }

    #[test]
    fn owned_popup_is_not_treated_as_child_coordinates() {
        let mut win_event = WinEvent::new(None);

        let parent = sample_window_state();
        win_event.create_window(0x1000, parent);

        let mut popup = sample_window_state();
        popup.parent = 0x1000;
        popup.style = 0x8000_0000;
        win_event.create_window(0x1001, popup);

        assert!(!win_event.uses_parent_client_coordinates(0x1001));
    }

    #[test]
    fn sync_host_window_position_updates_only_top_level_windows() {
        let mut win_event = WinEvent::new(None);

        let mut parent = sample_window_state();
        parent.x = 10;
        parent.y = 20;
        parent.ex_style = 0x0000_0200;
        parent.use_native_frame = false;
        win_event.create_window(0x1000, parent);

        let mut child = sample_window_state();
        child.style = WS_CHILD;
        child.parent = 0x1000;
        child.x = 5;
        child.y = 6;
        win_event.create_window(0x1001, child);

        assert!(win_event.sync_host_window_position(0x1000, 30, 40));
        assert_eq!(win_event.window_screen_origin(0x1001), Some((37, 48)));
        assert!(!win_event.sync_host_window_position(0x1001, 50, 60));
    }

    #[test]
    fn destroy_window_removes_child_subtree_with_parent() {
        let mut win_event = WinEvent::new(None);

        let parent = sample_window_state();
        win_event.create_window(0x1000, parent);

        let mut child = sample_window_state();
        child.style = WS_CHILD;
        child.parent = 0x1000;
        win_event.create_window(0x1001, child);

        let mut grandchild = sample_window_state();
        grandchild.style = WS_CHILD;
        grandchild.parent = 0x1001;
        win_event.create_window(0x1002, grandchild);

        assert_eq!(
            win_event.window_subtree_postorder(0x1000),
            vec![0x1002, 0x1001, 0x1000]
        );

        win_event.destroy_window(0x1000);

        assert!(!win_event.windows.contains_key(&0x1000));
        assert!(!win_event.windows.contains_key(&0x1001));
        assert!(!win_event.windows.contains_key(&0x1002));
    }

    #[test]
    fn realize_visible_child_creates_host_window_with_parent_client_coordinates() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut win_event = WinEvent::new(Some(tx));

        let mut parent = sample_window_state();
        parent.x = 100;
        parent.y = 200;
        parent.ex_style = 0x0000_0200;
        parent.use_native_frame = false;
        parent.visible = true;
        win_event.create_window(0x1000, parent);

        let mut child = sample_window_state();
        child.style = WS_CHILD;
        child.parent = 0x1000;
        child.visible = true;
        child.x = 10;
        child.y = 20;
        win_event.create_window(0x1001, child);

        win_event.realize_window(0x1001);

        match rx.try_recv().expect("child create command") {
            UiCommand::CreateWindow {
                hwnd,
                x,
                y,
                position_mode,
                parent,
                visible,
                ..
            } => {
                assert_eq!(hwnd, 0x1001);
                assert_eq!((x, y), (10, 20));
                assert_eq!(position_mode, WindowPositionMode::ParentClient);
                assert_eq!(parent, 0x1000);
                assert!(visible);
            }
            _ => panic!("expected CreateWindow for child"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn update_window_targets_child_directly() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut win_event = WinEvent::new(Some(tx));

        let parent = sample_window_state();
        win_event.create_window(0x1000, parent);

        let mut child = sample_window_state();
        child.style = WS_CHILD;
        child.parent = 0x1000;
        win_event.create_window(0x1001, child);

        win_event.update_window(0x1001);

        match rx.try_recv().expect("child update command") {
            UiCommand::UpdateWindow { hwnd } => assert_eq!(hwnd, 0x1001),
            _ => panic!("expected UpdateWindow for child"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn moving_parent_leaves_child_host_positioning_to_winit() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut win_event = WinEvent::new(Some(tx));

        let mut parent = sample_window_state();
        parent.x = 100;
        parent.y = 200;
        parent.ex_style = 0x0000_0200;
        parent.use_native_frame = false;
        win_event.create_window(0x1000, parent);

        let mut child = sample_window_state();
        child.style = WS_CHILD;
        child.parent = 0x1000;
        child.x = 10;
        child.y = 20;
        win_event.create_window(0x1001, child);

        let mut grandchild = sample_window_state();
        grandchild.style = WS_CHILD;
        grandchild.parent = 0x1001;
        grandchild.x = 3;
        grandchild.y = 4;
        win_event.create_window(0x1002, grandchild);

        win_event.move_window(0x1000, 300, 400, 640, 480);

        match rx.try_recv().expect("parent move command") {
            UiCommand::MoveWindow {
                hwnd,
                x,
                y,
                position_mode,
                ..
            } => {
                assert_eq!(hwnd, 0x1000);
                assert_eq!((x, y), (300, 400));
                assert_eq!(position_mode, WindowPositionMode::Screen);
            }
            _ => panic!("expected MoveWindow for parent"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn moving_child_emits_parent_client_relative_host_geometry() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut win_event = WinEvent::new(Some(tx));

        let parent = sample_window_state();
        win_event.create_window(0x1000, parent);

        let mut child = sample_window_state();
        child.style = WS_CHILD;
        child.parent = 0x1000;
        win_event.create_window(0x1001, child);

        win_event.move_window(0x1001, 30, 40, 120, 130);

        match rx.try_recv().expect("child move command") {
            UiCommand::MoveWindow {
                hwnd,
                x,
                y,
                position_mode,
                width,
                height,
            } => {
                assert_eq!(hwnd, 0x1001);
                assert_eq!((x, y), (30, 40));
                assert_eq!(position_mode, WindowPositionMode::ParentClient);
                assert_eq!((width, height), (120, 130));
            }
            _ => panic!("expected MoveWindow for child"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn moving_parent_keeps_owned_popup_screen_position() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut win_event = WinEvent::new(Some(tx));

        let mut parent = sample_window_state();
        parent.x = 100;
        parent.y = 200;
        win_event.create_window(0x1000, parent);

        let mut popup = sample_window_state();
        popup.parent = 0x1000;
        popup.style = 0x8000_0000;
        popup.x = 300;
        popup.y = 400;
        win_event.create_window(0x1001, popup);

        win_event.move_window(0x1000, 500, 600, 640, 480);

        match rx.try_recv().expect("parent move command") {
            UiCommand::MoveWindow {
                hwnd,
                x,
                y,
                position_mode,
                ..
            } => {
                assert_eq!(hwnd, 0x1000);
                assert_eq!((x, y), (500, 600));
                assert_eq!(position_mode, WindowPositionMode::Screen);
            }
            _ => panic!("expected MoveWindow for parent"),
        }
        assert!(rx.try_recv().is_err());
        assert_eq!(win_event.window_screen_origin(0x1001), Some((300, 400)));
    }

    #[test]
    fn syncing_host_move_keeps_owned_popup_screen_position() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut win_event = WinEvent::new(Some(tx));

        let mut parent = sample_window_state();
        parent.x = 100;
        parent.y = 200;
        win_event.create_window(0x1000, parent);

        let mut popup = sample_window_state();
        popup.parent = 0x1000;
        popup.style = 0x8000_0000;
        popup.x = 300;
        popup.y = 400;
        win_event.create_window(0x1001, popup);

        assert!(win_event.sync_host_window_position(0x1000, 500, 600));

        assert!(rx.try_recv().is_err());
        assert_eq!(win_event.window_screen_origin(0x1001), Some((300, 400)));
    }

    #[test]
    fn syncing_host_move_does_not_emit_host_move_command() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut win_event = WinEvent::new(Some(tx));

        let mut parent = sample_window_state();
        parent.x = 100;
        parent.y = 200;
        win_event.create_window(0x1000, parent);

        assert!(win_event.sync_host_window_position(0x1000, 500, 600));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn destroy_window_emits_destroy_for_entire_subtree() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut win_event = WinEvent::new(Some(tx));

        let parent = sample_window_state();
        win_event.create_window(0x1000, parent);

        let mut child = sample_window_state();
        child.style = WS_CHILD;
        child.parent = 0x1000;
        win_event.create_window(0x1001, child);

        let mut grandchild = sample_window_state();
        grandchild.style = WS_CHILD;
        grandchild.parent = 0x1001;
        win_event.create_window(0x1002, grandchild);

        win_event.destroy_window(0x1000);

        match rx.try_recv().expect("grandchild destroy command") {
            UiCommand::DestroyWindow { hwnd } => assert_eq!(hwnd, 0x1002),
            _ => panic!("expected DestroyWindow for grandchild"),
        }
        match rx.try_recv().expect("child destroy command") {
            UiCommand::DestroyWindow { hwnd } => assert_eq!(hwnd, 0x1001),
            _ => panic!("expected DestroyWindow for child"),
        }
        match rx.try_recv().expect("parent destroy command") {
            UiCommand::DestroyWindow { hwnd } => assert_eq!(hwnd, 0x1000),
            _ => panic!("expected DestroyWindow for parent"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn close_window_requests_minimize_instead_of_destroy() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut win_event = WinEvent::new(Some(tx));

        win_event.create_window(0x1000, sample_window_state());
        win_event.close_window(0x1000);

        match rx.try_recv().expect("minimize command") {
            UiCommand::MinimizeWindow { hwnd } => assert_eq!(hwnd, 0x1000),
            _ => panic!("expected MinimizeWindow for close_window"),
        }
        assert!(rx.try_recv().is_err());
    }
}
