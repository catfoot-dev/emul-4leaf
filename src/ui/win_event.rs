use crate::ui::UiCommand;
use crate::win32::WindowState;
use std::collections::HashMap;
use std::sync::mpsc::Sender;

/// 에뮬레이터 사이드에서 윈도우 객체들을 관리하는 추상화 레이어.
/// 실제 winit 윈도우 조작은 UiCommand 채널을 통해 UI 스레드에 요청함.
pub struct WinEvent {
    /// 가상 HWND 핸들 -> 윈도우 상태 맵
    pub windows: HashMap<u32, WindowState>,
    /// UI 스레드와의 통신 채널
    ui_tx: Option<Sender<UiCommand>>,
}

impl WinEvent {
    pub fn new(ui_tx: Option<Sender<UiCommand>>) -> Self {
        Self {
            windows: HashMap::new(),
            ui_tx,
        }
    }

    /// 새 윈도우 생성 및 UI 스레드에 알림
    pub fn create_window(&mut self, hwnd: u32, state: WindowState) {
        let title = state.title.clone();
        let width = state.width as u32;
        let height = state.height as u32;
        let style = state.style;
        let ex_style = state.ex_style;

        self.windows.insert(hwnd, state);

        if let Some(tx) = &self.ui_tx {
            let _ = tx.send(UiCommand::CreateWindow {
                hwnd,
                title,
                width,
                height,
                style,
                ex_style,
            });
        }
    }

    /// 윈도우 파괴 및 UI 스레드에 알림
    pub fn destroy_window(&mut self, hwnd: u32) {
        self.windows.remove(&hwnd);

        if let Some(tx) = &self.ui_tx {
            let _ = tx.send(UiCommand::DestroyWindow { hwnd });
        }
    }

    /// 특정 핸들의 윈도우 상태 가져오기
    pub fn get_window_mut(&mut self, hwnd: u32) -> Option<&mut WindowState> {
        self.windows.get_mut(&hwnd)
    }

    /// 윈도우 표시 상태 변경 및 UI 알림
    pub fn show_window(&mut self, hwnd: u32, visible: bool) {
        if let Some(state) = self.windows.get_mut(&hwnd) {
            state.visible = visible;
        }
        if let Some(tx) = &self.ui_tx {
            let _ = tx.send(UiCommand::ShowWindow { hwnd, visible });
        }
    }

    /// 윈도우 위치 및 크기 변경, UI 알림
    pub fn move_window(&mut self, hwnd: u32, x: i32, y: i32, width: u32, height: u32) {
        if let Some(state) = self.windows.get_mut(&hwnd) {
            state.x = x;
            state.y = y;
            state.width = width as i32;
            state.height = height as i32;
        }
        if let Some(tx) = &self.ui_tx {
            let _ = tx.send(UiCommand::MoveWindow {
                hwnd,
                x,
                y,
                width,
                height,
            });
        }
    }

    /// 윈도우 크기, 위치 및 Z 순서 변경, UI 알림
    pub fn set_window_pos(
        &mut self,
        hwnd: u32,
        _insert_after: u32,
        x: u32,
        y: u32,
        cx: u32,
        cy: u32,
        _flags: u32,
    ) {
        // SWP_NOMOVE = 0x0002, SWP_NOSIZE = 0x0001
        if let Some(state) = self.windows.get_mut(&hwnd) {
            if _flags & 0x0002 == 0 {
                state.x = x as i32;
                state.y = y as i32;
            }
            if _flags & 0x0001 == 0 {
                state.width = cx as i32;
                state.height = cy as i32;
            }
        }
        // UI 스레드에는 일단 MoveWindow 명령으로 전달 (Z-order 등은 현재 UI에서 미지원할 수 있음)
        if let Some(tx) = &self.ui_tx {
            let _ = tx.send(UiCommand::MoveWindow {
                hwnd,
                x: x as i32,
                y: y as i32,
                width: cx,
                height: cy,
            });
        }
    }

    /// 윈도우 제목 변경 및 UI 알림
    pub fn set_window_text(&mut self, hwnd: u32, text: String) {
        if let Some(state) = self.windows.get_mut(&hwnd) {
            state.title = text.clone();
        }
        if let Some(tx) = &self.ui_tx {
            let _ = tx.send(UiCommand::SetWindowText { hwnd, text });
        }
    }

    /// 윈도우 강제 다시 그리기 (UpdateWindow) 알림
    pub fn update_window(&self, hwnd: u32) {
        if let Some(tx) = &self.ui_tx {
            let _ = tx.send(UiCommand::UpdateWindow { hwnd });
        }
    }

    /// 메시지 박스 표시 및 응답 대기 (동기)
    pub fn message_box(&mut self, caption: String, text: String, u_type: u32) -> i32 {
        let (tx, rx) = std::sync::mpsc::channel();
        if let Some(ui_tx) = &self.ui_tx {
            let _ = ui_tx.send(UiCommand::MessageBox {
                caption,
                text,
                u_type,
                response_tx: tx,
            });
        }

        rx.recv().unwrap_or(1)
    }

    /// 윈도우 표시 여부 반환
    pub fn is_window_visible(&self, hwnd: u32) -> bool {
        self.windows.get(&hwnd).map(|w| w.visible).unwrap_or(false)
    }

    /// 윈도우 활성화 여부 반환 (현재는 항상 true)
    pub fn is_window_enabled(&self, _hwnd: u32) -> bool {
        true
    }

    /// 윈도우 닫기 요청
    pub fn close_window(&mut self, hwnd: u32) {
        if let Some(tx) = &self.ui_tx {
            let _ = tx.send(UiCommand::DestroyWindow { hwnd });
        }
    }

    /// 윈도우 활성화/비활성화 설정 (스텁)
    pub fn enable_window(&mut self, _hwnd: u32, _enable: bool) -> bool {
        true
    }

    /// 윈도우 활성화 요청 (포커스)
    pub fn activate_window(&mut self, hwnd: u32) {
        if let Some(tx) = &self.ui_tx {
            let _ = tx.send(UiCommand::ActivateWindow { hwnd });
        }
    }
}
