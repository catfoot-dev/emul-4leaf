pub mod gdi_renderer;
pub mod splash;
pub mod win_event;
pub mod win_frame;

#[cfg(target_os = "macos")]
use winit::platform::macos::WindowAttributesExtMacOS;
use winit::window::WindowAttributes;

/// 내장 폰트 데이터 (gulim.ttf)
pub const GULIM_FONT_DATA: &[u8] = include_bytes!("../../gulim.ttf");

/// 호스트 창 위치 좌표가 어떤 기준을 따르는지 나타냅니다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowPositionMode {
    /// 화면 절대 좌표를 사용합니다.
    Screen,
    /// 부모 클라이언트 영역 기준 좌표를 사용합니다.
    ParentClient,
}

/// 에뮬레이터 코어(Win32 API)가 UI 스레드에 요청하는 창 조작 커맨드
pub enum UiCommand {
    /// 새로운 윈도우 창 생성 요청
    CreateWindow {
        /// 가상 HWND 핸들
        hwnd: u32,
        /// 창 제목
        title: String,
        /// 위치 좌표계 기준 X 위치
        x: i32,
        /// 위치 좌표계 기준 Y 위치
        y: i32,
        /// `x`, `y`가 따르는 위치 좌표계
        position_mode: WindowPositionMode,
        /// 너비
        width: u32,
        /// 높이
        height: u32,
        /// 윈도우 스타일 (WS_*)
        style: u32,
        /// 확장 스타일 (WS_EX_*)
        ex_style: u32,
        /// guest 논리 부모 HWND
        parent: u32,
        /// 초기 표시 여부
        visible: bool,
        /// 호스트 네이티브 프레임 사용 여부
        use_native_frame: bool,
        /// 표면 비트맵 핸들
        surface_bitmap: u32,
    },
    /// 윈도우 스타일/확장 스타일 동기화 요청
    SyncWindowStyle {
        /// 가상 HWND 핸들
        hwnd: u32,
        /// 윈도우 스타일 (WS_*)
        style: u32,
        /// 확장 스타일 (WS_EX_*)
        ex_style: u32,
    },
    /// 특정 윈도우 창 파괴 요청
    DestroyWindow {
        /// 가상 HWND 핸들
        hwnd: u32,
    },
    /// 윈도우 표시 상태 변경 요청
    ShowWindow { hwnd: u32, visible: bool },
    /// 윈도우 위치/크기 변경 요청
    MoveWindow {
        hwnd: u32,
        /// 위치 좌표계 기준 X 위치
        x: i32,
        /// 위치 좌표계 기준 Y 위치
        y: i32,
        /// `x`, `y`가 따르는 위치 좌표계
        position_mode: WindowPositionMode,
        width: u32,
        height: u32,
    },
    /// 윈도우 제목 변경 요청
    SetWindowText { hwnd: u32, text: String },
    /// 윈도우 아이콘 변경 요청
    SetWindowIcon { hwnd: u32, hicon: u32 },
    /// 윈도우 강제 렌더링(업데이트) 요청
    UpdateWindow { hwnd: u32 },
    /// 윈도우 활성화(포커스) 요청
    ActivateWindow { hwnd: u32 },
    /// 윈도우 활성화/비활성화 요청
    EnableWindow {
        /// 가상 HWND 핸들
        hwnd: u32,
        /// 입력 활성화 여부
        enabled: bool,
    },
    /// 메시지 박스 표시 요청 (동기 응답 채널 포함)
    MessageBox {
        caption: String,
        text: String,
        u_type: u32,
        response_tx: std::sync::mpsc::Sender<i32>,
    },
    /// 윈도우 커서 변경 요청
    SetCursor { hwnd: u32, hcursor: u32 },
    /// 윈도우 드래그 시작 요청
    DragWindow { hwnd: u32 },
    /// 윈도우 투명 모드 변경 요청 (SetWindowRgn 등에서 사용)
    SetWindowTransparent { hwnd: u32, transparent: bool },
    /// 윈도우 최소화 요청
    MinimizeWindow { hwnd: u32 },
    /// 윈도우 최대화 요청
    MaximizeWindow { hwnd: u32 },
    /// 윈도우 기본 상태 복구 요청
    RestoreWindow { hwnd: u32 },
}

/// 플랫폼별 기본 창 속성을 적용합니다.
pub(crate) fn apply_platform_window_attributes(attributes: WindowAttributes) -> WindowAttributes {
    #[cfg(target_os = "macos")]
    {
        use winit::{platform::macos::OptionAsAlt, window::WindowButtons};

        return attributes
            .with_enabled_buttons(WindowButtons::empty())
            .with_title_hidden(true)
            .with_titlebar_transparent(true)
            .with_fullsize_content_view(true)
            .with_option_as_alt(OptionAsAlt::Both)
            .with_borderless_game(true)
            .with_active(true);
    }

    #[allow(unreachable_code)]
    attributes
}

/// 윈도우 콘텐츠를 그리는 인터페이스
#[allow(dead_code)]
pub trait Painter: std::any::Any {
    fn create_window(
        &self,
        event_loop: &winit::event_loop::ActiveEventLoop,
    ) -> winit::window::Window;
    fn quit_on_close(&self) -> bool;
    fn should_close(&self) -> bool {
        false
    }
    /// 버퍼에 현재 프레임을 정상적으로 그렸으면 `true`를 반환합니다.
    fn paint(&mut self, buffer: &mut [u32], width: u32, height: u32) -> bool;
    fn handle_event(
        &mut self,
        _event: &winit::event::WindowEvent,
        _event_loop: &winit::event_loop::ActiveEventLoop,
    ) -> bool;
    fn tick(&mut self) -> bool;
    /// 주기적으로 `tick()`을 호출해야 하는 경우 원하는 간격을 반환합니다.
    fn poll_interval(&self) -> Option<std::time::Duration> {
        None
    }
    fn as_any(&self) -> &dyn std::any::Any;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

use std::sync::mpsc::Receiver;

/// UI 이벤트 루프를 시작하고 모든 윈도우를 관리함
pub fn run_ui(
    ui_rx: Receiver<UiCommand>,
    initial_painters: Vec<Box<dyn Painter>>,
    context: crate::dll::win32::Win32Context,
) {
    let event_loop = winit::event_loop::EventLoop::<()>::with_user_event()
        .build()
        .unwrap();
    win_event::WinEvent::install_wake_proxy(event_loop.create_proxy());
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);

    let mut app = win_frame::WinFrame::new(ui_rx, initial_painters, context);
    event_loop.run_app(&mut app).unwrap();
}
