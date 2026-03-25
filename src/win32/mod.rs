mod dll_advapi32;
mod dll_comctl32;
mod dll_gdi32;
mod dll_imm32;
mod dll_kernel32;
mod dll_msvcp60;
mod dll_msvcrt;
mod dll_ole32;
mod dll_shell32;
mod dll_user32;
mod dll_winmm;
mod dll_ws2_32;

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

use crate::{
    helper::{FAKE_IMPORT_BASE, HEAP_BASE},
    server::packet_logger::PacketLogger,
    ui::{UiCommand, win_event::WinEvent},
    win32::{
        dll_advapi32::DllADVAPI32, dll_comctl32::DllCOMCTL32, dll_gdi32::DllGDI32,
        dll_imm32::DllIMM32, dll_kernel32::DllKERNEL32, dll_msvcp60::DllMSVCP60,
        dll_msvcrt::DllMSVCRT, dll_ole32::DllOle32, dll_shell32::DllSHELL32, dll_user32::DllUSER32,
        dll_winmm::DllWINMM, dll_ws2_32::DllWS2_32,
    },
};

/// 함수 호출이 끝난 뒤 스택을 어떻게 되돌려 놓을 것인지 명시하는 열거형
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackCleanup {
    Caller,
    Callee(usize),
}

/// Fake API (Win32 API 후킹) 호출 결과값을 에뮬레이터 코어에 어떻게 돌려줄지 정의
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiHookResult {
    pub cleanup: StackCleanup,
    pub return_value: Option<i32>,
}

impl ApiHookResult {
    pub const fn caller(return_value: Option<i32>) -> Self {
        Self {
            cleanup: StackCleanup::Caller,
            return_value,
        }
    }

    pub const fn callee(arg_count: usize, return_value: Option<i32>) -> Self {
        Self {
            cleanup: StackCleanup::Callee(arg_count),
            return_value,
        }
    }
}

impl From<(usize, Option<i32>)> for ApiHookResult {
    fn from((arg_count, return_value): (usize, Option<i32>)) -> Self {
        ApiHookResult::callee(arg_count, return_value)
    }
}

pub fn callee_result(result: Option<(usize, Option<i32>)>) -> Option<ApiHookResult> {
    result.map(ApiHookResult::from)
}

pub fn caller_result(result: Option<(usize, Option<i32>)>) -> Option<ApiHookResult> {
    result.map(|(_, return_value)| ApiHookResult::caller(return_value))
}

/// 메모리에 로드되어 에뮬레이팅될 준비가 끝난 프록시 DLL의 메타데이터 구조체
#[derive(Debug, Clone)]
pub struct LoadedDll {
    pub name: String,
    pub base_addr: u64,
    // pub size: usize,
    pub entry_point: u64,
    pub exports: HashMap<String, u64>,
}

#[derive(Debug, Clone)]
pub struct CursorFrame {
    pub width: u32,
    pub height: u32,
    pub hotspot_x: i32,
    pub hotspot_y: i32,
    pub pixels: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct IconFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}

/// 가상 GDI 오브젝트 종류
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum GdiObject {
    Font {
        name: String,
        height: i32,
    },
    Pen {
        style: u32,
        width: u32,
        color: u32,
    },
    Brush {
        color: u32,
    },
    Bitmap {
        width: u32,
        height: u32,
        pixels: Arc<Mutex<Vec<u32>>>,
    },
    Dc {
        associated_window: u32,
        width: i32,
        height: i32,
        selected_bitmap: u32,
        selected_font: u32,
        selected_brush: u32,
        selected_pen: u32,
        selected_region: u32,
        selected_palette: u32,
        bk_mode: i32,
        bk_color: u32,
        text_color: u32,
        rop2_mode: i32,
        current_x: i32,
        current_y: i32,
    },
    Region {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    },
    Palette {
        num_entries: u32,
    },
    StockObject(u32),
    Cursor {
        resource_id: u32,
        name: Option<String>,
        frames: Vec<CursorFrame>,
        is_animated: bool,
    },
    Icon {
        resource_id: u32,
        name: Option<String>,
        frames: Vec<IconFrame>,
    },
}

/// 실제 TCP 스트림을 보유하는 Winsock 소켓 상태 (Tokio 기반)
#[allow(dead_code)]
pub struct TokioSocket {
    pub af: u32,
    pub sock_type: u32,
    pub protocol: u32,
    /// 실제 연결된 TCP 스트림 (connect 후 Some으로 변경)
    pub stream: Option<tokio::net::TcpStream>,
    /// recv() 에서 미소비된 바이트 버퍼
    pub recv_buf: Vec<u8>,
    /// ioctlsocket(FIONBIO) 로 설정하는 논블로킹 모드
    pub non_blocking: bool,
    /// 연결된 원격 주소 문자열 (IP:port)
    pub remote_addr: Option<String>,
}

impl std::fmt::Debug for TokioSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokioSocket")
            .field("af", &self.af)
            .field("sock_type", &self.sock_type)
            .field("protocol", &self.protocol)
            .field("connected", &self.stream.is_some())
            .field("recv_buf_len", &self.recv_buf.len())
            .field("non_blocking", &self.non_blocking)
            .field("remote_addr", &self.remote_addr)
            .finish()
    }
}

/// 가상 소켓 상태 (레거시 호환용, 새 코드는 TokioSocket 사용)
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

/// 가상 이벤트 상태
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct EventState {
    pub signaled: bool,
    pub manual_reset: bool,
}

/// 가상 WNDCLASS 정보
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct WindowClass {
    pub class_name: String,
    pub wnd_proc: u32,
    pub style: u32,
    pub hinstance: u32,
    pub cb_cls_extra: i32,
    pub cb_wnd_extra: i32,
    pub h_icon: u32,
    pub h_cursor: u32,
    pub hbr_background: u32,
    pub menu_name: String,
}

/// 가상 윈도우 상태
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
    pub surface_bitmap: u32,
    pub window_rgn: u32,
}

/// Unicorn 엔진의 `User Data` 에 적재되어, 모든 Win32 가상 OS 환경의 전역 상태 트리를
/// 관리하고 유지하는 핵심 컨텍스트 블록
pub struct Win32Context {
    /// 힙(Heap) 메모리 할당을 위한 기준 포인터 (단순 증가형 메모리 할당 방식)
    pub heap_cursor: AtomicU32,
    // import 카운터
    pub import_address: AtomicU32,

    pub dll_modules: Arc<Mutex<HashMap<String, LoadedDll>>>,
    pub address_map: Arc<Mutex<HashMap<u64, String>>>,

    // === 새로 추가된 상태 ===
    /// Win32 GetLastError / SetLastError
    pub last_error: AtomicU32,
    /// 가상 핸들 카운터 (HWND, HDC, HFONT, SOCKET 등에 사용)
    pub handle_counter: AtomicU32,
    /// TCP 소켓 맵 (핸들 → TokioSocket, 실제 I/O 담당)
    pub tcp_sockets: Arc<Mutex<HashMap<u32, TokioSocket>>>,
    /// 가상 소켓 맵 (레거시 상태 추적용)
    pub sockets: Arc<Mutex<HashMap<u32, SocketState>>>,
    /// 윈도우 관리 프레임
    pub win_event: Arc<Mutex<WinEvent>>,
    /// 등록된 윈도우 클래스
    pub window_classes: Arc<Mutex<HashMap<String, WindowClass>>>,
    /// 가상 GDI 오브젝트 맵 (핸들 → 오브젝트)
    pub gdi_objects: Arc<Mutex<HashMap<u32, GdiObject>>>,
    /// 가상 이벤트 맵 (핸들 → 상태)
    pub events: Arc<Mutex<HashMap<u32, EventState>>>,
    /// TLS 슬롯 (인덱스 → 값)
    pub tls_slots: Arc<Mutex<HashMap<u32, u32>>>,
    /// TLS 슬롯 카운터
    pub tls_counter: AtomicU32,
    /// 가상 레지스트리 (키 경로 → 값)
    pub registry: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    /// 가상 레지스트리 핸들 맵 (HKEY → 키 경로)
    pub registry_handles: Arc<Mutex<HashMap<u32, String>>>,
    /// GetTickCount 기준 시간
    pub start_time: Instant,
    /// rand() 시드 상태
    pub rand_state: AtomicU32,
    /// 패킷 로거 (프로토콜 분석용)
    pub packet_logger: Arc<Mutex<PacketLogger>>,
    /// 가상 파일 맵 (핸들 -> 호스트 파일)
    pub files: Arc<Mutex<HashMap<u32, std::fs::File>>>,
    /// 포커스를 가진 윈도우 핸들
    pub focus_hwnd: AtomicU32,
    /// 활성화된 윈도우 핸들
    pub active_hwnd: AtomicU32,
    /// 전면에 있는 윈도우 핸들
    pub foreground_hwnd: AtomicU32,
    /// 마우스 캡처를 가진 윈도우 핸들
    pub capture_hwnd: AtomicU32,
    /// 애플리케이션 가상 메시지 큐 (hwnd, message, wParam, lParam, time, pt.x, pt.y)
    pub message_queue: Arc<Mutex<std::collections::VecDeque<[u32; 7]>>>,
    /// 활성화된 타이머 (ID -> elapse)
    pub timers: Arc<Mutex<HashMap<u32, u32>>>,
    /// 키보드 가상 키 상태 (기본 256키)
    pub key_states: Arc<Mutex<[bool; 256]>>,
    /// 가상 클립보드 데이터 버퍼
    pub clipboard_data: Arc<Mutex<Vec<u8>>>,
    /// 클립보드 점유 상태 확인용
    pub clipboard_open: AtomicU32,
    /// localtime() 등을 위한 정적 tm 구조체 주소
    pub tm_struct_ptr: AtomicU32,
    /// 데스크톱 창의 핸들
    pub desktop_hwnd: AtomicU32,
    /// 현재 활성화된 커서 핸들
    pub current_cursor: AtomicU32,
    /// 마우스 현재 X 좌표
    pub mouse_x: AtomicU32,
    /// 마우스 현재 Y 좌표
    pub mouse_y: AtomicU32,
    /// CRT exit handlers (__dllonexit, _onexit)
    pub onexit_handlers: Arc<Mutex<Vec<u32>>>,
}

impl Win32Context {
    pub fn new(ui_tx: Option<Sender<UiCommand>>) -> Self {
        let ctx = Win32Context {
            heap_cursor: AtomicU32::new(HEAP_BASE as u32),
            import_address: AtomicU32::new(FAKE_IMPORT_BASE as u32),
            dll_modules: Arc::new(Mutex::new(HashMap::new())),
            address_map: Arc::new(Mutex::new(HashMap::new())),
            // 새 상태
            last_error: AtomicU32::new(0),
            handle_counter: AtomicU32::new(0x1000), // 핸들은 0x1000부터 시작
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
            focus_hwnd: AtomicU32::new(0),
            active_hwnd: AtomicU32::new(0),
            foreground_hwnd: AtomicU32::new(0),
            capture_hwnd: AtomicU32::new(0),
            message_queue: Arc::new(Mutex::new(std::collections::VecDeque::new())),
            timers: Arc::new(Mutex::new(HashMap::new())),
            key_states: Arc::new(Mutex::new([false; 256])),
            clipboard_data: Arc::new(Mutex::new(Vec::new())),
            clipboard_open: AtomicU32::new(0),
            tm_struct_ptr: AtomicU32::new(0),
            desktop_hwnd: AtomicU32::new(0), // 초기값은 0, 필요 시 아래에서 설정
            current_cursor: AtomicU32::new(0),
            mouse_x: AtomicU32::new(320), // 기본 마우스 위치
            mouse_y: AtomicU32::new(240),
            onexit_handlers: Arc::new(Mutex::new(Vec::new())),
        };
        // 데스크톱 핸들 할당 (0x10000 등 고정값 대신 handle_counter 사용)
        let desktop_hwnd = ctx.alloc_handle();
        ctx.desktop_hwnd.store(desktop_hwnd, Ordering::SeqCst);
        ctx
    }

    /// 새 가상 핸들 발급
    pub fn alloc_handle(&self) -> u32 {
        self.handle_counter.fetch_add(1, Ordering::SeqCst)
    }

    /// 윈도우용 표면 비트맵 생성
    pub fn create_surface_bitmap(&self, width: u32, height: u32) -> u32 {
        let hbmp = self.alloc_handle();
        let pixels = Arc::new(Mutex::new(vec![0u32; (width * height) as usize]));
        self.gdi_objects.lock().unwrap().insert(
            hbmp,
            GdiObject::Bitmap {
                width,
                height,
                pixels,
            },
        );
        hbmp
    }

    pub fn handle(
        uc: &mut Unicorn<Win32Context>,
        dll_name: &str,
        func_name: &str,
    ) -> Option<ApiHookResult> {
        // 처리를 성공했다면 스택 보정값과 리턴값을 포함한 `ApiHookResult`를 반환
        match dll_name {
            "ADVAPI32.dll" => DllADVAPI32::handle(uc, func_name),
            "COMCTL32.dll" => DllCOMCTL32::handle(uc, func_name),
            "GDI32.dll" => DllGDI32::handle(uc, func_name),
            "IMM32.dll" => DllIMM32::handle(uc, func_name),
            "KERNEL32.dll" => DllKERNEL32::handle(uc, func_name),
            "MSVCP60.dll" => DllMSVCP60::handle(uc, func_name),
            "MSVCRT.dll" => DllMSVCRT::handle(uc, func_name),
            "ole32.dll" => DllOle32::handle(uc, func_name),
            "SHELL32.dll" => DllSHELL32::handle(uc, func_name),
            "USER32.dll" => DllUSER32::handle(uc, func_name),
            "WINMM.dll" => DllWINMM::handle(uc, func_name),
            "WS2_32.dll" => DllWS2_32::handle(uc, func_name),
            _ => {
                crate::emu_log!("[!] Undefined DLL: {}", dll_name);
                None
            }
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
            last_error: AtomicU32::new(self.last_error.load(Ordering::SeqCst)),
            handle_counter: AtomicU32::new(self.handle_counter.load(Ordering::SeqCst)),
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
            start_time: self.start_time.clone(),
            rand_state: AtomicU32::new(self.rand_state.load(Ordering::SeqCst)),
            packet_logger: self.packet_logger.clone(),
            files: self.files.clone(),
            focus_hwnd: AtomicU32::new(self.focus_hwnd.load(Ordering::SeqCst)),
            active_hwnd: AtomicU32::new(self.active_hwnd.load(Ordering::SeqCst)),
            foreground_hwnd: AtomicU32::new(self.foreground_hwnd.load(Ordering::SeqCst)),
            capture_hwnd: AtomicU32::new(self.capture_hwnd.load(Ordering::SeqCst)),
            message_queue: self.message_queue.clone(),
            timers: self.timers.clone(),
            key_states: self.key_states.clone(),
            clipboard_data: self.clipboard_data.clone(),
            clipboard_open: AtomicU32::new(self.clipboard_open.load(Ordering::SeqCst)),
            tm_struct_ptr: AtomicU32::new(self.tm_struct_ptr.load(Ordering::SeqCst)),
            desktop_hwnd: AtomicU32::new(self.desktop_hwnd.load(Ordering::SeqCst)),
            current_cursor: AtomicU32::new(self.current_cursor.load(Ordering::SeqCst)),
            mouse_x: AtomicU32::new(self.mouse_x.load(Ordering::SeqCst)),
            mouse_y: AtomicU32::new(self.mouse_y.load(Ordering::SeqCst)),
            onexit_handlers: self.onexit_handlers.clone(),
        }
    }
}
