use std::collections::HashMap;
use std::sync::mpsc::Sender;
use crate::debug::common::UiCommand;
use crate::win32::WindowState;

/// 에뮬레이터 사이드에서 윈도우 객체들을 관리하는 추상화 레이어.
/// 실제 winit 윈도우 조작은 UiCommand 채널을 통해 UI 스레드에 요청함.
pub struct WinFrame {
    /// 가상 HWND 핸들 -> 윈도우 상태 맵
    pub windows: HashMap<u32, WindowState>,
    /// UI 스레드와의 통신 채널
    ui_tx: Option<Sender<UiCommand>>,
}

impl WinFrame {
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
        
        self.windows.insert(hwnd, state);

        if let Some(tx) = &self.ui_tx {
            let _ = tx.send(UiCommand::CreateWindow {
                hwnd,
                title,
                width,
                height,
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
}
