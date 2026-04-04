mod advapi32;
mod comctl32;
mod gdi32;
mod imm32;
pub(crate) mod kernel32;
mod msvcp60;
mod msvcrt;
mod ole32;
mod shell32;
mod user32;
mod winmm;
mod ws2_32;

use crate::{
    dll::{
        rare::{Rare, RareAudioEngine, RareContextState, RareSoundState},
        win32::{
            advapi32::ADVAPI32, comctl32::COMCTL32, gdi32::GDI32, imm32::IMM32, kernel32::KERNEL32,
            msvcp60::MSVCP60, msvcrt::MSVCRT, ole32::Ole32, shell32::SHELL32, user32::USER32,
            winmm::WINMM, ws2_32::WS2_32,
        },
    },
    helper::{FAKE_IMPORT_BASE, HEAP_BASE},
    server::packet_logger::PacketLogger,
    ui::{UiCommand, win_event::WinEvent},
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
        mpsc::Sender,
    },
    time::Instant,
};
use unicorn_engine::Unicorn;

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
    /// 영역(Region) 오브젝트 - 클리핑이나 히트 테스트에 사용됩니다.
    Region {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
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
    pub user_data: u32,
    /// 호스트가 네이티브 캡션/프레임을 유지할지 여부
    pub use_native_frame: bool,
    /// 윈도우 렌더링을 위한 백버퍼 비트맵 핸들
    pub surface_bitmap: u32,
    /// 윈도우의 가시 영역(Region) 핸들
    pub window_rgn: u32,
    /// 윈도우가 다시 그려져야 하는지 여부 (WM_PAINT 생성용)
    pub needs_paint: bool,
}

/// Unicorn 엔진의 `User Data`에 적재되어, 모든 Win32 가상 OS 환경의 전역 상태를
/// 관리하고 유지하는 핵심 컨텍스트 블록입니다.
pub struct Win32Context {
    /// 가상 힙(Heap) 메모리 할당을 위한 현재 포인터입니다. (단순 증가형)
    pub heap_cursor: AtomicU32,
    /// Fake API(Import) 주소 할당을 위한 카운터입니다.
    pub import_address: AtomicU32,

    /// 로드된 DLL 모듈들의 맵 (이름 ->LoadedDll)
    pub dll_modules: Arc<Mutex<HashMap<String, LoadedDll>>>,
    /// 가상 주소와 함수명 간의 역방향 매핑 (디버깅용)
    pub address_map: Arc<Mutex<HashMap<u64, String>>>,
    /// 프록시 DLL이 생성한 전역/데이터 export 주소 캐시
    pub proxy_exports: Arc<Mutex<HashMap<String, u32>>>,

    /// Win32 에러 코드 (GetLastError / SetLastError)
    pub last_error: Arc<AtomicU32>,
    /// 가상 핸들(HWND, HDC, SOCKET 등) 발급을 위한 전역 카운터
    pub handle_counter: Arc<AtomicU32>,
    /// 채널 기반 가상 소켓 맵
    pub tcp_sockets: Arc<Mutex<HashMap<u32, VirtualSocket>>>,
    /// 소켓의 논리적 상태를 추적하는 맵
    pub sockets: Arc<Mutex<HashMap<u32, SocketState>>>,
    /// 윈도우 이벤트 관리부 (UI와 신호 교환)
    pub win_event: Arc<Mutex<WinEvent>>,
    /// 등록된 가상 윈도우 클래스 정보
    pub window_classes: Arc<Mutex<HashMap<String, WindowClass>>>,
    /// 가상 GDI 오브젝트 맵 (핸들 -> 오브젝트)
    pub gdi_objects: Arc<Mutex<HashMap<u32, GdiObject>>>,
    /// 가상 동기화 이벤트 맵
    pub events: Arc<Mutex<HashMap<u32, EventState>>>,
    /// TLS(Thread Local Storage) 슬롯 데이터 (outer key = thread_id, inner = tls_index → value)
    pub tls_slots: Arc<Mutex<HashMap<u32, HashMap<u32, u32>>>>,
    /// TLS 슬롯 할당을 위한 카운터
    pub tls_counter: AtomicU32,
    /// 가상 레지스트리 데이터 (키 경로 -> 값 데이터)
    pub registry: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    /// 가상 레지스트리 핸들 맵 (HKEY -> 키 경로)
    pub registry_handles: Arc<Mutex<HashMap<u32, String>>>,
    /// 에뮬레이터 구동 시작 시간 (GetTickCount 등의 기준)
    pub start_time: Instant,
    /// rand() 함수를 위한 가상 시드 상태
    pub rand_state: AtomicU32,
    /// 패킷 로거 (Winsock 통신 분석용)
    pub packet_logger: Arc<Mutex<PacketLogger>>,
    /// 가상 파일 핸들 맵 (HFILE -> 호스트 파일 객체)
    pub files: Arc<Mutex<HashMap<u32, std::fs::File>>>,
    /// WSA 이벤트 핸들 맵 (event_handle → WsaEventEntry)
    /// WSAEventSelect / WSAEnumNetworkEvents / WaitForSingleObject 구현에 사용
    pub wsa_event_map: Arc<Mutex<HashMap<u32, WsaEventEntry>>>,

    /// 포커스를 가진 윈도우 핸들
    pub focus_hwnd: Arc<AtomicU32>,
    /// 현재 활성(Active) 상태인 윈도우 핸들
    pub active_hwnd: Arc<AtomicU32>,
    /// 최상위 전면(Foreground) 윈도우 핸들
    pub foreground_hwnd: Arc<AtomicU32>,
    /// 마우스 캡처를 보유한 윈도우 핸들
    pub capture_hwnd: Arc<AtomicU32>,
    /// 마우스 트래킹 상태 (_TrackMouseEvent 용)
    pub track_mouse_event: Arc<Mutex<Option<TrackMouseEventState>>>,

    /// 애플리케이션용 가상 메시지 큐
    pub message_queue: Arc<Mutex<std::collections::VecDeque<[u32; 7]>>>,
    /// 활성화된 타이머 맵 (ID -> Timer 객체)
    pub timers: Arc<Mutex<HashMap<u32, Timer>>>,
    /// 가상 키보드 키 상태 배열 (256키)
    pub key_states: Arc<Mutex<[bool; 256]>>,

    /// 가상 클립보드 데이터 버퍼
    pub clipboard_data: Arc<Mutex<Vec<u8>>>,
    /// 클립보드 열림 상태 (소유 핸들)
    pub clipboard_open: AtomicU32,
    /// 정적 시간(tm) 구조체를 위한 가상 주소
    pub tm_struct_ptr: AtomicU32,
    /// 데스크톱 창의 가상 핸들
    pub desktop_hwnd: AtomicU32,
    /// 현재 표시되는 커서의 핸들
    pub current_cursor: AtomicU32,
    /// 마우스 현재 X 좌표
    pub mouse_x: Arc<AtomicU32>,
    /// 마우스 현재 Y 좌표
    pub mouse_y: Arc<AtomicU32>,
    /// CRT 종료 핸들러 리스트
    pub onexit_handlers: Arc<Mutex<Vec<u32>>>,
    /// 에뮬레이션된 가상 스레드 목록
    pub threads: Arc<Mutex<Vec<EmulatedThread>>>,
    /// 현재 실행 중인 스레드 ID (0 = 메인 스레드)
    pub current_thread_idx: Arc<AtomicU32>,
    /// 메인 스레드(tid=0)용 재실행 대기 시각
    pub main_resume_time: Arc<Mutex<Option<Instant>>>,
    /// 메인 스레드의 재시도형 대기 API 타임아웃 시각
    pub main_wait_deadline: Arc<Mutex<Option<Instant>>>,
    /// 중첩 emu_start 호출 깊이 (코드 훅 내 재귀 제한용)
    pub emu_depth: Arc<AtomicU32>,
    /// Rare.dll 프록시가 보유한 CPAL 오디오 엔진 상태
    pub(crate) rare_audio: Arc<Mutex<Option<RareAudioEngine>>>,
    /// Rare.dll 프록시가 생성한 컨텍스트 객체 맵
    pub(crate) rare_contexts: Arc<Mutex<HashMap<u32, RareContextState>>>,
    /// Rare.dll 프록시가 생성한 사운드 객체 맵
    pub(crate) rare_sounds: Arc<Mutex<HashMap<u32, RareSoundState>>>,
}

impl Win32Context {
    /// 새로운 Win32 환경 컨텍스트를 생성합니다.
    ///
    /// # 인자
    /// * `ui_tx`: UI로 명령을 보낼 송신 채널
    pub fn new(ui_tx: Option<Sender<UiCommand>>) -> Self {
        let ctx = Win32Context {
            heap_cursor: AtomicU32::new(HEAP_BASE as u32),
            import_address: AtomicU32::new(FAKE_IMPORT_BASE as u32),
            dll_modules: Arc::new(Mutex::new(HashMap::new())),
            address_map: Arc::new(Mutex::new(HashMap::new())),
            proxy_exports: Arc::new(Mutex::new(HashMap::new())),
            last_error: Arc::new(AtomicU32::new(0)),
            handle_counter: Arc::new(AtomicU32::new(0x1000)),
            tcp_sockets: Arc::new(Mutex::new(HashMap::new())),
            sockets: Arc::new(Mutex::new(HashMap::new())),
            win_event: Arc::new(Mutex::new(WinEvent::new(ui_tx))),
            window_classes: Arc::new(Mutex::new(HashMap::new())),
            gdi_objects: Arc::new(Mutex::new(HashMap::new())),
            events: Arc::new(Mutex::new(HashMap::new())),
            tls_slots: Arc::new(Mutex::new(HashMap::new())),
            tls_counter: AtomicU32::new(1),
            registry: Arc::new(Mutex::new(HashMap::new())),
            registry_handles: Arc::new(Mutex::new({
                let mut m = HashMap::new();
                m.insert(0x80000000, "HKEY_CLASSES_ROOT".to_string());
                m.insert(0x80000001, "HKEY_CURRENT_USER".to_string());
                m.insert(0x80000002, "HKEY_LOCAL_MACHINE".to_string());
                m.insert(0x80000003, "HKEY_USERS".to_string());
                m
            })),
            start_time: Instant::now(),
            rand_state: AtomicU32::new(12345),
            packet_logger: Arc::new(Mutex::new(PacketLogger::new())),
            files: Arc::new(Mutex::new(HashMap::new())),
            wsa_event_map: Arc::new(Mutex::new(HashMap::new())),
            focus_hwnd: Arc::new(AtomicU32::new(0)),
            active_hwnd: Arc::new(AtomicU32::new(0)),
            foreground_hwnd: Arc::new(AtomicU32::new(0)),
            capture_hwnd: Arc::new(AtomicU32::new(0)),
            track_mouse_event: Arc::new(Mutex::new(None)),
            message_queue: Arc::new(Mutex::new(std::collections::VecDeque::new())),
            timers: Arc::new(Mutex::new(HashMap::new())),
            key_states: Arc::new(Mutex::new([false; 256])),
            clipboard_data: Arc::new(Mutex::new(Vec::new())),
            clipboard_open: AtomicU32::new(0),
            tm_struct_ptr: AtomicU32::new(0),
            desktop_hwnd: AtomicU32::new(0),
            current_cursor: AtomicU32::new(0),
            mouse_x: Arc::new(AtomicU32::new(320)),
            mouse_y: Arc::new(AtomicU32::new(240)),
            onexit_handlers: Arc::new(Mutex::new(Vec::new())),
            threads: Arc::new(Mutex::new(Vec::new())),
            current_thread_idx: Arc::new(AtomicU32::new(0)),
            main_resume_time: Arc::new(Mutex::new(None)),
            main_wait_deadline: Arc::new(Mutex::new(None)),
            emu_depth: Arc::new(AtomicU32::new(0)),
            rare_audio: Arc::new(Mutex::new(None)),
            rare_contexts: Arc::new(Mutex::new(HashMap::new())),
            rare_sounds: Arc::new(Mutex::new(HashMap::new())),
        };

        // 데스크톱 핸들 선행 할당
        let desktop_hwnd = ctx.alloc_handle();
        ctx.desktop_hwnd.store(desktop_hwnd, Ordering::SeqCst);
        ctx
    }

    /// 새로운 가상 핸들(u32)을 발급합니다.
    pub fn alloc_handle(&self) -> u32 {
        self.handle_counter.fetch_add(1, Ordering::SeqCst)
    }

    /// 윈도우용 표면(Surface) 비트맵을 생성하고 GDI 오브젝트로 등록합니다.
    ///
    /// # 인자
    /// * `width`: 비트맵 너비
    /// * `height`: 비트맵 높이
    ///
    /// # 반환
    /// * `u32`: 생성된 비트맵의 가상 핸들
    pub fn create_surface_bitmap(&self, width: u32, height: u32) -> u32 {
        let hbmp = self.alloc_handle();
        let pixels = Arc::new(Mutex::new(vec![0u32; (width * height) as usize]));
        self.gdi_objects.lock().unwrap().insert(
            hbmp,
            GdiObject::Bitmap {
                width,
                height,
                pixels,
                bits_addr: None,
                bpp: 32,
                top_down: false,
            },
        );
        hbmp
    }

    /// DLL 이름과 함수 이름을 기반으로 적절한 Win32 API 핸들러로 분기합니다.
    ///
    /// # 인자
    /// * `uc`: Unicorn 엔진 인스턴스 (Win32Context 포함)
    /// * `dll_name`: 호출된 DLL의 이름
    /// * `func_name`: 호출된 함수의 이름
    ///
    /// # 반환
    /// * `Option<ApiHookResult>`: 핸들러 실행 결과 (성공 시 Some, 정의되지 않은 경우 None)
    pub fn handle(
        uc: &mut Unicorn<Win32Context>,
        dll_name: &str,
        func_name: &str,
    ) -> Option<ApiHookResult> {
        match dll_name {
            "ADVAPI32.dll" => ADVAPI32::handle(uc, func_name),
            "COMCTL32.dll" => COMCTL32::handle(uc, func_name),
            "GDI32.dll" => GDI32::handle(uc, func_name),
            "IMM32.dll" => IMM32::handle(uc, func_name),
            "KERNEL32.dll" => KERNEL32::handle(uc, func_name),
            "MSVCP60.dll" => MSVCP60::handle(uc, func_name),
            "MSVCRT.dll" => MSVCRT::handle(uc, func_name),
            "Rare.dll" => Rare::handle(uc, func_name),
            "ole32.dll" => Ole32::handle(uc, func_name),
            "SHELL32.dll" => SHELL32::handle(uc, func_name),
            "USER32.dll" => USER32::handle(uc, func_name),
            "WINMM.dll" => WINMM::handle(uc, func_name),
            "WS2_32.dll" => WS2_32::handle(uc, func_name),
            _ => {
                crate::emu_log!("[!] Undefined DLL: {}", dll_name);
                None
            }
        }
    }

    /// DLL의 임포트 해소(Resolve) 과정에서 프록시 DLL이 특수하게 관리하는 데이터 주소 등이 있는지 확인합니다.
    pub fn resolve_proxy_export(
        uc: &mut Unicorn<Win32Context>,
        dll_name: &str,
        func_name: &str,
    ) -> Option<u32> {
        match dll_name {
            "MSVCRT.dll" => MSVCRT::resolve_export(uc, func_name),
            "MSVCP60.dll" => MSVCP60::resolve_export(uc, func_name),
            "Rare.dll" => Rare::resolve_export(uc, func_name),
            _ => None,
        }
    }
}

impl Clone for Win32Context {
    fn clone(&self) -> Self {
        Self {
            heap_cursor: AtomicU32::new(self.heap_cursor.load(Ordering::SeqCst)),
            import_address: AtomicU32::new(self.import_address.load(Ordering::SeqCst)),
            dll_modules: self.dll_modules.clone(),
            address_map: self.address_map.clone(),
            proxy_exports: self.proxy_exports.clone(),
            last_error: self.last_error.clone(),
            handle_counter: self.handle_counter.clone(),
            tcp_sockets: self.tcp_sockets.clone(),
            sockets: self.sockets.clone(),
            win_event: self.win_event.clone(),
            window_classes: self.window_classes.clone(),
            gdi_objects: self.gdi_objects.clone(),
            events: self.events.clone(),
            tls_slots: self.tls_slots.clone(),
            tls_counter: AtomicU32::new(self.tls_counter.load(Ordering::SeqCst)),
            registry: self.registry.clone(),
            registry_handles: self.registry_handles.clone(),
            start_time: self.start_time,
            rand_state: AtomicU32::new(self.rand_state.load(Ordering::SeqCst)),
            packet_logger: self.packet_logger.clone(),
            files: self.files.clone(),
            wsa_event_map: self.wsa_event_map.clone(),
            focus_hwnd: self.focus_hwnd.clone(),
            active_hwnd: self.active_hwnd.clone(),
            foreground_hwnd: self.foreground_hwnd.clone(),
            capture_hwnd: self.capture_hwnd.clone(),
            track_mouse_event: self.track_mouse_event.clone(),
            message_queue: self.message_queue.clone(),
            timers: self.timers.clone(),
            key_states: self.key_states.clone(),
            clipboard_data: self.clipboard_data.clone(),
            clipboard_open: AtomicU32::new(self.clipboard_open.load(Ordering::SeqCst)),
            tm_struct_ptr: AtomicU32::new(self.tm_struct_ptr.load(Ordering::SeqCst)),
            desktop_hwnd: AtomicU32::new(self.desktop_hwnd.load(Ordering::SeqCst)),
            current_cursor: AtomicU32::new(self.current_cursor.load(Ordering::SeqCst)),
            mouse_x: self.mouse_x.clone(),
            mouse_y: self.mouse_y.clone(),
            onexit_handlers: self.onexit_handlers.clone(),
            threads: self.threads.clone(),
            current_thread_idx: self.current_thread_idx.clone(),
            main_resume_time: self.main_resume_time.clone(),
            main_wait_deadline: self.main_wait_deadline.clone(),
            emu_depth: self.emu_depth.clone(),
            rare_audio: self.rare_audio.clone(),
            rare_contexts: self.rare_contexts.clone(),
            rare_sounds: self.rare_sounds.clone(),
        }
    }
}
