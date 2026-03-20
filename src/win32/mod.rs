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

use std::{cell::RefCell, collections::HashMap, rc::Rc, time::Instant};

use unicorn_engine::Unicorn;

use crate::{
    helper::{FAKE_IMPORT_BASE, HEAP_BASE},
    packet_logger::PacketLogger,
    win32::{
        dll_advapi32::DllADVAPI32, dll_comctl32::DllCOMCTL32, dll_gdi32::DllGDI32,
        dll_imm32::DllIMM32, dll_kernel32::DllKERNEL32, dll_msvcp60::DllMSVCP60,
        dll_msvcrt::DllMSVCRT, dll_ole32::DllOle32, dll_shell32::DllSHELL32, dll_user32::DllUSER32,
        dll_winmm::DllWINMM, dll_ws2_32::DllWS2_32,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackCleanup {
    Caller,
    Callee(usize),
}

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

#[derive(Debug, Clone)]
pub struct LoadedDll {
    pub name: String,
    pub base_addr: u64,
    // pub size: usize,
    pub entry_point: u64,
    pub exports: HashMap<String, u64>,
}

/// 가상 GDI 오브젝트 종류
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
        bits_ptr: u64,
    },
    Dc {
        associated_window: u32,
    },
    Region {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    },
    Palette,
    StockObject(u32),
}

/// 가상 소켓 상태
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
#[derive(Debug, Clone)]
pub struct EventState {
    pub signaled: bool,
    pub manual_reset: bool,
}

/// 가상 WNDCLASS 정보
#[derive(Debug, Clone)]
pub struct WindowClass {
    pub class_name: String,
    pub wnd_proc: u32,
    pub style: u32,
    pub hinstance: u32,
}

/// 가상 윈도우 상태
#[derive(Debug, Clone)]
pub struct WindowState {
    pub class_name: String,
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub style: u32,
    pub parent: u32,
    pub visible: bool,
    pub wnd_proc: u32,
    pub user_data: u32,
}

// Unicorn 엔진이 품고 다닐 데이터 (User Data)
pub struct Win32Context {
    // 힙 메모리 할당을 위한 포인터 (다음에 쓸 빈 공간의 주소)
    pub heap_cursor: u64,
    // import 카운터
    pub import_address: u64,

    pub dll_modules: Rc<RefCell<HashMap<String, LoadedDll>>>,
    pub address_map: HashMap<u64, String>,

    // === 새로 추가된 상태 ===
    /// Win32 GetLastError / SetLastError
    pub last_error: u32,
    /// 가상 핸들 카운터 (HWND, HDC, HFONT, SOCKET 등에 사용)
    pub handle_counter: u32,
    /// 가상 소켓 맵 (핸들 → 상태)
    pub sockets: HashMap<u32, SocketState>,
    /// 가상 윈도우 맵 (HWND → 상태)
    pub windows: HashMap<u32, WindowState>,
    /// 등록된 윈도우 클래스
    pub window_classes: HashMap<String, WindowClass>,
    /// 가상 GDI 오브젝트 맵 (핸들 → 오브젝트)
    pub gdi_objects: HashMap<u32, GdiObject>,
    /// 가상 이벤트 맵 (핸들 → 상태)
    pub events: HashMap<u32, EventState>,
    /// TLS 슬롯 (인덱스 → 값)
    pub tls_slots: HashMap<u32, u32>,
    /// TLS 슬롯 카운터
    pub tls_counter: u32,
    /// 가상 레지스트리 (키 경로 → 값)
    pub registry: HashMap<String, Vec<u8>>,
    /// GetTickCount 기준 시간
    pub start_time: Instant,
    /// rand() 시드 상태
    pub rand_state: u32,
    /// 패킷 로거 (프로토콜 분석용)
    pub packet_logger: PacketLogger,
}

impl Win32Context {
    pub fn new() -> Self {
        Win32Context {
            heap_cursor: HEAP_BASE,
            import_address: FAKE_IMPORT_BASE,
            dll_modules: Rc::new(RefCell::new(HashMap::new())),
            address_map: HashMap::new(),
            // 새 상태
            last_error: 0,
            handle_counter: 0x1000, // 핸들은 0x1000부터 시작
            sockets: HashMap::new(),
            windows: HashMap::new(),
            window_classes: HashMap::new(),
            gdi_objects: HashMap::new(),
            events: HashMap::new(),
            tls_slots: HashMap::new(),
            tls_counter: 0,
            registry: HashMap::new(),
            start_time: Instant::now(),
            rand_state: 12345,
            packet_logger: PacketLogger::new(),
        }
    }

    /// 새 가상 핸들 발급
    pub fn alloc_handle(&mut self) -> u32 {
        let handle = self.handle_counter;
        self.handle_counter += 1;
        handle
    }

    pub fn handle(
        uc: &mut Unicorn<Win32Context>,
        dll_name: &str,
        func_name: &str,
    ) -> Option<ApiHookResult> {
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
                println!("Undefined DLL: {}", dll_name);
                None
            }
        }
    }
}
