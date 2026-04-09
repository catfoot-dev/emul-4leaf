use crate::{dll::win32::WindowState, ui::UiCommand};
use std::{
    collections::HashMap,
    sync::{OnceLock, mpsc::Sender},
};

static UI_WAKE_PROXY: OnceLock<winit::event_loop::EventLoopProxy<()>> = OnceLock::new();
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
        let _ = proxy.send_event(());
    }
}

/// 에뮬레이터 사이드에서 윈도우 객체들을 관리하는 추상화 레이어.
/// 실제 winit 윈도우 조작은 UiCommand 채널을 통해 UI 스레드에 요청함.
pub struct WinEvent {
    /// 가상 HWND 핸들 -> 윈도우 상태 맵
    pub windows: HashMap<u32, WindowState>,
    /// UI 스레드와의 통신 채널
    ui_tx: Option<Sender<UiCommand>>,
    /// 윈도우 상태 변경 시 증가하는 세대 카운터 (paint 최적화용)
    pub generation: u64,
}

impl WinEvent {
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

    /// 특정 부모 창 내의 좌표 (x, y)에 위치한 가장 깊은 자식 창을 찾습니다.
    /// x, y는 parent_hwnd의 클라이언트 영역 기준 좌표입니다.
    pub fn child_window_from_point(&self, parent_hwnd: u32, x: i32, y: i32) -> u32 {
        // Z-order가 높은 순서(가장 전면)부터 검사하기 위해 정렬된 자식 목록 생성
        let mut children: Vec<_> = self
            .windows
            .iter()
            .filter(|(_, w)| w.parent == parent_hwnd && w.visible)
            .collect();

        // z_order 기준 내림차순 정렬
        children.sort_by_key(|(_, w)| std::cmp::Reverse(w.z_order));

        for (&hwnd, state) in children {
            // 자식 창의 범위 안에 있는지 확인 (자식 창의 x, y는 부모 클라이언트 기준)
            if x >= state.x
                && x < state.x + state.width
                && y >= state.y
                && y < state.y + state.height
            {
                // 더 깊은 자식 창이 있는지 재귀적으로 탐색
                // 좌표를 자식 창의 클라이언트 영역 기준으로 변환하여 다음 단계로 전달
                return self.child_window_from_point(hwnd, x - state.x, y - state.y);
            }
        }

        parent_hwnd
    }

    /// 화면 갱신을 실제로 담당하는 호스트 윈도우 HWND를 찾습니다.
    fn redraw_target_for(&self, hwnd: u32) -> Option<u32> {
        let mut current = hwnd;

        loop {
            let state = self.windows.get(&current)?;
            if state.style & WS_CHILD == 0 || state.parent == 0 {
                return Some(current);
            }
            current = state.parent;
        }
    }

    /// 지정된 창의 좌상단 화면 좌표를 부모 체인을 따라 누적해 계산합니다.
    pub fn window_screen_origin(&self, hwnd: u32) -> Option<(i32, i32)> {
        let mut current = hwnd;
        let mut x = 0i32;
        let mut y = 0i32;

        for _ in 0..=self.windows.len() {
            let state = self.windows.get(&current)?;
            x += state.x;
            y += state.y;

            if (state.style & WS_CHILD) == 0 || state.parent == 0 {
                return Some((x, y));
            }

            current = state.parent;
        }

        None
    }

    /// 지정된 창의 좌상단이 루트 호스트 창 클라이언트 기준 어디에 있는지 반환합니다.
    pub fn window_client_origin_in_host(&self, hwnd: u32) -> Option<(i32, i32)> {
        let root = self.redraw_target_for(hwnd)?;
        let (screen_x, screen_y) = self.window_screen_origin(hwnd)?;
        let (root_x, root_y) = self.window_screen_origin(root)?;
        Some((screen_x - root_x, screen_y - root_y))
    }

    /// 자식 창 변경이 화면에 반영되도록 호스트 윈도우에 다시 그리기를 요청합니다.
    fn request_visual_refresh(&self, hwnd: u32) {
        if let Some(target) = self.redraw_target_for(hwnd) {
            self.send_ui_command(UiCommand::UpdateWindow { hwnd: target });
        }
    }

    /// UI 이벤트 루프를 깨우기 위한 프록시를 등록합니다.
    pub fn install_wake_proxy(proxy: winit::event_loop::EventLoopProxy<()>) {
        let _ = UI_WAKE_PROXY.set(proxy);
    }

    /// 다른 스레드에서 UI 이벤트 루프를 깨웁니다.
    pub fn notify_wakeup() {
        wake_ui_event_loop();
    }

    pub fn new(ui_tx: Option<Sender<UiCommand>>) -> Self {
        Self {
            windows: HashMap::new(),
            ui_tx,
            generation: 0,
        }
    }

    /// UI 스레드에 임의의 커맨드 전송
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

        if (state.style & WS_CHILD) != 0 {
            crate::emu_log!(
                "[UI] realize_window: child HWND {:#x} class=\"{}\" parent={:#x} visible={} size={}x{}",
                hwnd,
                state.class_name,
                state.parent,
                state.visible,
                state.width,
                state.height
            );
            if state.visible {
                self.request_visual_refresh(hwnd);
            }
            return;
        }

        let title = state.title.clone();
        let x = state.x;
        let y = state.y;
        let width = state.width as u32;
        let height = state.height as u32;
        let style = state.style;
        let ex_style = state.ex_style;
        let parent = state.parent;
        let visible = state.visible;
        let use_native_frame = state.use_native_frame;
        let surface_bitmap = state.surface_bitmap;

        crate::emu_log!(
            "[UI] realize_window: top-level HWND {:#x} class=\"{}\" title=\"{}\" visible={} size={}x{} parent={:#x}",
            hwnd,
            state.class_name,
            state.title,
            visible,
            width,
            height,
            parent
        );

        self.send_ui_command(UiCommand::CreateWindow {
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
        });
    }

    /// 윈도우 파괴 및 UI 스레드에 알림
    pub fn destroy_window(&mut self, hwnd: u32) {
        for child_hwnd in self.child_windows(hwnd) {
            self.destroy_window(child_hwnd);
        }

        let is_child = self
            .windows
            .get(&hwnd)
            .map(|state| (state.style & WS_CHILD) != 0)
            .unwrap_or(false);
        let redraw_target = is_child.then(|| self.redraw_target_for(hwnd)).flatten();
        self.windows.remove(&hwnd);
        self.bump_generation();
        if !is_child {
            self.send_ui_command(UiCommand::DestroyWindow { hwnd });
        }
        if let Some(target) = redraw_target {
            self.send_ui_command(UiCommand::UpdateWindow { hwnd: target });
        }
    }

    /// 윈도우 크기 변경 시 상태 업데이트
    pub fn resize_window(&mut self, hwnd: u32, width: u32, height: u32) {
        if let Some(state) = self.windows.get_mut(&hwnd) {
            state.width = width as i32;
            state.height = height as i32;
            state.last_hittest_lparam = u32::MAX;
            self.bump_generation();
        }
    }

    /// 특정 핸들의 윈도우 상태 가져오기
    pub fn get_window_mut(&mut self, hwnd: u32) -> Option<&mut WindowState> {
        self.windows.get_mut(&hwnd)
    }

    /// 윈도우 표시 상태 변경 및 UI 알림
    pub fn show_window(&mut self, hwnd: u32, visible: bool) {
        let is_child = self
            .windows
            .get(&hwnd)
            .map(|state| (state.style & WS_CHILD) != 0)
            .unwrap_or(false);
        if let Some(state) = self.windows.get_mut(&hwnd) {
            state.visible = visible;
            self.bump_generation();
        }
        if is_child {
            self.request_visual_refresh(hwnd);
        } else {
            self.send_ui_command(UiCommand::ShowWindow { hwnd, visible });
        }
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

    /// 윈도우 최소화 상태 변경 및 UI 알림
    pub fn minimize_window(&mut self, hwnd: u32) {
        if let Some(state) = self.windows.get_mut(&hwnd) {
            state.iconic = true;
            state.zoomed = false;
        }
        self.send_ui_command(UiCommand::MinimizeWindow { hwnd });
    }

    /// 윈도우 최대화 상태 변경 및 UI 알림
    pub fn maximize_window(&mut self, hwnd: u32) {
        if let Some(state) = self.windows.get_mut(&hwnd) {
            state.iconic = false;
            state.zoomed = true;
        }
        self.send_ui_command(UiCommand::MaximizeWindow { hwnd });
    }

    /// 윈도우 일반 상태 변경 및 UI 알림
    pub fn restore_window(&mut self, hwnd: u32) {
        if let Some(state) = self.windows.get_mut(&hwnd) {
            state.iconic = false;
            state.zoomed = false;
        }
        self.send_ui_command(UiCommand::RestoreWindow { hwnd });
    }

    /// 윈도우 위치 및 크기 변경, UI 알림
    pub fn move_window(&mut self, hwnd: u32, x: i32, y: i32, width: u32, height: u32) {
        let is_child = self
            .windows
            .get(&hwnd)
            .map(|state| (state.style & WS_CHILD) != 0)
            .unwrap_or(false);
        if let Some(state) = self.windows.get_mut(&hwnd) {
            state.x = x;
            state.y = y;
            state.width = width as i32;
            state.height = height as i32;
            self.bump_generation();
        }
        if is_child {
            self.request_visual_refresh(hwnd);
        } else {
            self.send_ui_command(UiCommand::MoveWindow {
                hwnd,
                x,
                y,
                width,
                height,
            });
        }
    }

    /// 호스트 OS 창 이동 결과를 내부 창 좌표에 반영합니다.
    pub fn sync_host_window_position(&mut self, hwnd: u32, x: i32, y: i32) -> bool {
        let Some(state) = self.windows.get_mut(&hwnd) else {
            return false;
        };

        if (state.style & WS_CHILD) != 0 || (state.x == x && state.y == y) {
            return false;
        }

        state.x = x;
        state.y = y;
        state.last_hittest_lparam = u32::MAX;
        self.bump_generation();
        true
    }

    /// 윈도우 크기, 위치 및 Z 순서 변경, UI 알림
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
        let is_child = self
            .windows
            .get(&hwnd)
            .map(|state| (state.style & WS_CHILD) != 0)
            .unwrap_or(false);
        let mut visibility_changed = false;
        let mut new_visibility = false;

        let parent = self.windows.get(&hwnd).map(|s| s.parent).unwrap_or(0);

        // SWP_NOZORDER = 0x0004
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

        // SWP_NOMOVE = 0x0002, SWP_NOSIZE = 0x0001
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
        // 자식 창은 고전 UI 엔진이 SetWindowPos를 레이아웃/클리핑 보조로도 사용하므로,
        // HIDE를 그대로 반영하면 배경용 child가 통째로 사라질 수 있습니다.
        // 대신 SHOW는 반영해서, 생성 후 SetWindowPos(..., SWP_SHOWWINDOW)로 드러나는 child는 살립니다.
        if flags & 0x0040 != 0 && !state.visible {
            state.visible = true;
            visibility_changed = true;
            new_visibility = true;
        }
        if !is_child && flags & 0x0080 != 0 && state.visible {
            state.visible = false;
            visibility_changed = true;
            new_visibility = false;
        }
        let (final_x, final_y, final_w, final_h) =
            (state.x, state.y, state.width as u32, state.height as u32);
        self.bump_generation();
        // 자식 창은 부모 표면 합성으로 그리므로 호스트 창만 다시 그리면 됩니다.
        if is_child {
            self.request_visual_refresh(hwnd);
        } else {
            if visibility_changed {
                self.send_ui_command(UiCommand::ShowWindow {
                    hwnd,
                    visible: new_visibility,
                });
            }
            self.send_ui_command(UiCommand::MoveWindow {
                hwnd,
                x: final_x,
                y: final_y,
                width: final_w,
                height: final_h,
            });
        }
    }

    /// 윈도우 제목 변경 및 UI 알림
    pub fn set_window_text(&mut self, hwnd: u32, text: String) {
        if let Some(state) = self.windows.get_mut(&hwnd) {
            state.title = text.clone();
        }
        self.send_ui_command(UiCommand::SetWindowText { hwnd, text });
    }

    /// 윈도우의 특정 영역을 무효화하여 다시 그리도록 요청 (needs_paint 플래그 설정)
    pub fn invalidate_rect(&mut self, hwnd: u32, _rect: *mut std::ffi::c_void) {
        if let Some(state) = self.windows.get_mut(&hwnd) {
            state.needs_paint = true;
            self.update_window(hwnd);
        }
    }

    /// 윈도우의 유효성을 검사하여 다시 그리기 요청을 해제합니다. (needs_paint 플래그 해제)
    pub fn validate_window(&mut self, hwnd: u32) {
        if let Some(state) = self.windows.get_mut(&hwnd) {
            state.needs_paint = false;
        }
    }

    /// 윈도우 강제 다시 그리기 (UpdateWindow) 알림
    pub fn update_window(&self, hwnd: u32) {
        self.request_visual_refresh(hwnd);
    }

    /// 메시지 박스 표시 및 응답 대기 (동기)
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

    /// 윈도우 표시 여부 반환
    pub fn is_window_visible(&self, hwnd: u32) -> bool {
        self.windows.get(&hwnd).map(|w| w.visible).unwrap_or(false)
    }

    /// 윈도우 활성화 여부를 반환합니다.
    pub fn is_window_enabled(&self, hwnd: u32) -> bool {
        self.windows.get(&hwnd).map(|w| w.enabled).unwrap_or(false)
    }

    /// 윈도우 닫기 요청
    pub fn close_window(&mut self, hwnd: u32) {
        self.send_ui_command(UiCommand::DestroyWindow { hwnd });
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

    /// 윈도우 활성화 요청 (포커스)
    pub fn activate_window(&mut self, hwnd: u32) {
        self.send_ui_command(UiCommand::ActivateWindow { hwnd });
    }

    /// 윈도우 스타일/확장 스타일을 UI 스레드와 동기화합니다.
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
            title: "test".to_string(),
            x: 0,
            y: 0,
            width: 640,
            height: 480,
            style: 0,
            ex_style: 0,
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
    fn window_screen_origin_accumulates_parent_chain() {
        let mut win_event = WinEvent::new(None);

        let mut parent = sample_window_state();
        parent.x = 100;
        parent.y = 200;
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

        assert_eq!(win_event.window_screen_origin(0x1002), Some((113, 224)));
        assert_eq!(
            win_event.window_client_origin_in_host(0x1002),
            Some((13, 24))
        );
    }

    #[test]
    fn sync_host_window_position_updates_only_top_level_windows() {
        let mut win_event = WinEvent::new(None);

        let mut parent = sample_window_state();
        parent.x = 10;
        parent.y = 20;
        win_event.create_window(0x1000, parent);

        let mut child = sample_window_state();
        child.style = WS_CHILD;
        child.parent = 0x1000;
        child.x = 5;
        child.y = 6;
        win_event.create_window(0x1001, child);

        assert!(win_event.sync_host_window_position(0x1000, 30, 40));
        assert_eq!(win_event.window_screen_origin(0x1001), Some((35, 46)));
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
}
