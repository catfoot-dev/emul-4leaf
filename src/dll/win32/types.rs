//! # Win32 에뮬레이션 타입 정의
//!
//! Win32 API 에뮬레이션에 사용되는 데이터 구조체 및 열거형을 정의합니다.

use std::collections::HashMap;
use std::fs::File;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// WSA 이벤트 핸들과 소켓을 연결하는 상태 구조체입니다.
#[derive(Debug, Clone)]
pub struct WsaEventEntry {
    /// 이 이벤트와 연결된 소켓 핸들
    pub socket: u32,
    /// WSAEventSelect에서 등록한 관심 이벤트 마스크 (FD_READ 등)
    pub interest: u32,
    /// 발생했지만 아직 소비되지 않은 이벤트 (WSAEnumNetworkEvents가 읽고 클리어)
    pub pending: u32,
}

/// 에뮬레이션되는 가상 스레드의 상태를 저장하는 구조체
#[derive(Debug, Clone)]
pub struct EmulatedThread {
    pub handle: u32,
    pub thread_id: u32,
    /// 스레드 스택 블록의 시작 주소 (힙에서 할당)
    pub stack_alloc: u32,
    pub stack_size: u32,
    // 저장된 CPU 레지스터
    pub eax: u32,
    pub ecx: u32,
    pub edx: u32,
    pub ebx: u32,
    pub esp: u32,
    pub ebp: u32,
    pub esi: u32,
    pub edi: u32,
    pub eip: u32,
    pub alive: bool,
    /// _endthreadex / _endthread 에 의해 종료가 요청된 경우 true
    pub terminate_requested: bool,
    /// `CREATE_SUSPENDED` 등으로 인해 스케줄링되면 안 되는 경우 true
    pub suspended: bool,
    /// 스레드가 다시 실행될 수 있는 최소 시각 (Yield/Sleep 용)
    pub resume_time: Option<Instant>,
    /// 재시도형 대기 API의 최종 타임아웃 시각
    pub wait_deadline: Option<Instant>,
}

/// 함수 호출 후 스택 정리 방식을 정의하는 열거형입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackCleanup {
    /// 호출자(Caller)가 스택을 정리하는 방식 (예: x86 cdecl)
    Caller,
    /// 피호출자(Callee)가 지정된 인자 크기만큼 스택을 정리하는 방식 (예: x86 stdcall)
    Callee(usize),
}

/// Win32 API 후킹(Fake API) 호출 결과를 정의하는 구조체입니다.
/// 에뮬레이터 코어가 함수 실행 후 레지스터와 스택을 어떻게 갱신할지 결정합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiHookResult {
    /// 함수 종료 시 적용할 스택 정리 방식
    pub cleanup: StackCleanup,
    /// EAX 레지스터에 기록될 리턴값 (None일 경우 레지스터를 변경하지 않음)
    pub return_value: Option<i32>,
    /// true일 경우 리턴 처리를 하지 않고 동일한 API 호출을 재시도하도록 유도 (yield용)
    pub retry: bool,
}

impl ApiHookResult {
    /// 호출자 정리 방식(cdecl)의 결과를 생성합니다.
    pub const fn caller(return_value: Option<i32>) -> Self {
        Self {
            cleanup: StackCleanup::Caller,
            return_value,
            retry: false,
        }
    }

    /// 재시도(yield)를 요청하는 결과를 생성합니다.
    pub const fn retry() -> Self {
        Self {
            cleanup: StackCleanup::Caller,
            return_value: None,
            retry: true,
        }
    }

    /// 피호출자 정리 방식(stdcall)의 결과를 생성합니다.
    pub const fn callee(arg_count: usize, return_value: Option<i32>) -> Self {
        Self {
            cleanup: StackCleanup::Callee(arg_count),
            return_value,
            retry: false,
        }
    }
}

impl From<(usize, Option<i32>)> for ApiHookResult {
    fn from((arg_count, return_value): (usize, Option<i32>)) -> Self {
        ApiHookResult::callee(arg_count, return_value)
    }
}

/// 메모리에 로드되어 에뮬레이팅될 준비가 끝난 프록시 DLL의 메타데이터 구조체
#[derive(Debug, Clone)]
pub struct LoadedDll {
    pub name: String,
    pub base_addr: u64,
    pub size: u64,
    pub entry_point: u64,
    pub exports: HashMap<String, u64>,
}

/// 커서(Cursor)의 단일 프레임 데이터를 저장합니다.
#[derive(Debug, Clone)]
pub struct CursorFrame {
    pub width: u32,
    pub height: u32,
    pub hotspot_x: i32,
    pub hotspot_y: i32,
    /// RGBA8888 픽셀 데이터
    pub pixels: Vec<u32>,
}

/// 아이콘(Icon)의 단일 프레임 데이터를 저장합니다.
#[derive(Debug, Clone)]
pub struct IconFrame {
    pub width: u32,
    pub height: u32,
    /// RGBA8888 픽셀 데이터
    pub pixels: Vec<u32>,
}

/// 가상 GDI(Graphics Device Interface) 오브젝트를 정의하는 열거형입니다.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum GdiObject {
    /// 폰트 오브젝트 (LOGFONT 기반)
    Font { name: String, height: i32 },
    /// 펜 오브젝트 (선의 스타일, 굵기, 색상)
    Pen { style: u32, width: u32, color: u32 },
    /// 브러시 오브젝트 (채우기 색상)
    Brush { color: u32 },
    /// 비트맵 오브젝트 (픽셀 버퍼 포함)
    Bitmap {
        width: u32,
        height: u32,
        pixels: Arc<Mutex<Vec<u32>>>,
        /// DIBSection용 에뮬레이터 힙 주소 (CreateDIBSection이 프로그램에 반환한 bits 포인터)
        bits_addr: Option<u32>,
        /// 픽셀 비트 심도 (8/24/32)
        bpp: u32,
        /// true = 탑다운(top-down) DIB (biHeight < 0)
        top_down: bool,
    },
    /// 디바이스 컨텍스트(DC) - 그래픽 작업의 상태를 유지합니다.
    Dc {
        /// 연결된 윈도우 핸들 (없을 경우 0)
        associated_window: u32,
        width: i32,
        height: i32,
        /// 선택된 GDI 오브젝트 핸들들
        selected_bitmap: u32,
        selected_font: u32,
        selected_brush: u32,
        selected_pen: u32,
        selected_region: u32,
        selected_palette: u32,
        /// 배경 모드 (OPAQUE, TRANSPARENT)
        bk_mode: i32,
        bk_color: u32,
        text_color: u32,
        /// 래스터 연산 모드 (R2_COPYPEN 등)
        rop2_mode: i32,
        /// 현재 그리기 위치 (MoveToEx 등으로 설정)
        current_x: i32,
        current_y: i32,
    },
    /// 영역(Region) 오브젝트 - 클리핑이나 히터 테스트에 사용됩니다.
    Region {
        /// 영역을 구성하는 직사각형 목록 (Win32 RGNDATA의 RECT 배열에 해당)
        rects: Vec<(i32, i32, i32, i32)>,
    },
    /// 팔레트(Palette) 오브젝트
    Palette { num_entries: u32 },
    /// Stock Object (WHITE_BRUSH 등 시스템 정의 오브젝트)
    StockObject(u32),
    /// 커서 오브젝트
    Cursor {
        resource_id: u32,
        name: Option<String>,
        frames: Vec<CursorFrame>,
        is_animated: bool,
        /// ANI 기본 프레임 표시 간격 (jiffies 단위, 1 jiffy = 1/60초)
        display_rate_jiffies: u32,
    },
    /// 아이콘 오브젝트
    Icon {
        resource_id: u32,
        name: Option<String>,
        frames: Vec<IconFrame>,
    },
}

/// 채널 기반 가상 소켓 상태입니다. DNet 핸들러 스레드와 mpsc 채널로 통신합니다.
#[allow(dead_code)]
pub struct VirtualSocket {
    pub af: u32,
    pub sock_type: u32,
    pub protocol: u32,
    /// 게스트 → DNet 핸들러 스레드로 데이터를 보내는 송신단
    pub chan_tx: Option<std::sync::mpsc::Sender<Vec<u8>>>,
    /// DNet 핸들러 스레드 → 게스트로 데이터를 받는 수신단
    pub chan_rx: Option<std::sync::mpsc::Receiver<Vec<u8>>>,
    /// chan_tx가 활성화되면 true (FD_CONNECT, FD_WRITE 판단에 사용)
    pub connected: bool,
    /// recv() 시 미소비된 데이터를 보관하는 내부 버퍼
    pub recv_buf: Vec<u8>,
    /// Non-blocking 모드 여부 (ioctlsocket FIONBIO 설정값)
    pub non_blocking: bool,
    /// 연결된 원격 주소 문자열 (IP:Port)
    pub remote_addr: Option<String>,
}

impl std::fmt::Debug for VirtualSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VirtualSocket")
            .field("af", &self.af)
            .field("sock_type", &self.sock_type)
            .field("protocol", &self.protocol)
            .field("connected", &self.connected)
            .field("has_chan_tx", &self.chan_tx.is_some())
            .field("has_chan_rx", &self.chan_rx.is_some())
            .field("recv_buf_len", &self.recv_buf.len())
            .field("non_blocking", &self.non_blocking)
            .field("remote_addr", &self.remote_addr)
            .finish()
    }
}

/// 가상 소켓의 상태를 나타내는 열거형입니다. (레거시 추적용)
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum SocketState {
    Created {
        af: u32,
        sock_type: u32,
        protocol: u32,
    },
    Connected {
        remote_addr: String,
        remote_port: u16,
    },
    Listening {
        local_port: u16,
    },
    Closed,
}

/// 가상 이벤트(Event) 객체의 상태를 나타냅니다.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct EventState {
    pub signaled: bool,
    pub manual_reset: bool,
}

/// 가상 C 런타임 파일 핸들의 상태를 저장합니다.
#[derive(Debug)]
pub struct FileState {
    /// 실제 호스트 파일 객체
    pub file: File,
    /// 디버깅용 원본 경로 문자열
    pub path: String,
    /// 마지막 읽기에서 EOF를 만났는지 여부
    pub eof: bool,
    /// 마지막 I/O에서 에러가 발생했는지 여부
    pub error: bool,
}

/// 가상 윈도우 클래스(WNDCLASS) 정보를 저장합니다.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct WindowClass {
    pub atom: u32,
    pub class_name: String,
    pub class_name_ptr: u32,
    pub wnd_proc: u32,
    pub style: u32,
    pub hinstance: u32,
    pub cb_cls_extra: i32,
    pub cb_wnd_extra: i32,
    pub h_icon: u32,
    pub h_icon_sm: u32,
    pub h_cursor: u32,
    pub hbr_background: u32,
    pub menu_name: String,
    pub menu_name_ptr: u32,
}

/// 가상 타이머(Timer) 정보를 저장합니다.
#[derive(Debug, Clone)]
pub struct Timer {
    pub hwnd: u32,
    pub id: u32,
    pub elapse: u32,
    pub timer_proc: u32,
    pub last_tick: std::time::Instant,
}

/// 가상 마우스 트래킹(TrackMouseEvent) 상태를 저장합니다.
#[derive(Debug, Clone)]
pub struct TrackMouseEventState {
    pub hwnd: u32,
    pub flags: u32,
    pub hover_time: u32,
}

/// 가상 윈도우(HWND)의 현재 상태를 저장합니다.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct WindowState {
    pub class_name: String,
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub style: u32,
    pub ex_style: u32,
    pub parent: u32,
    pub id: u32,
    pub visible: bool,
    pub enabled: bool,
    pub zoomed: bool,
    pub iconic: bool,
    pub wnd_proc: u32,
    /// `WM_SETCURSOR` 기본 처리에서 사용할 클래스 기본 커서 핸들
    pub class_cursor: u32,
    pub user_data: u32,
    /// 호스트가 네이티브 캡션/프레임을 유지할지 여부
    pub use_native_frame: bool,
    /// 윈도우 렌더링을 위한 백버퍼 비트맵 핸들
    pub surface_bitmap: u32,
    /// 윈도우의 가시 영역(Region) 핸들
    pub window_rgn: u32,
    /// 윈도우가 다시 그려져야 하는지 여부 (WM_PAINT 생성용)
    pub needs_paint: bool,
    /// WM_NCHITTEST 캐시: 마지막 테스트 좌표 (LPARAM 형식)
    pub last_hittest_lparam: u32,
    /// WM_NCHITTEST 캐시: 마지막 결과 값
    pub last_hittest_result: u32,
    /// 윈도우 Z 순서를 결정합니다. 값이 높을수록 전면에 표시됩니다.
    pub z_order: u32,
}
