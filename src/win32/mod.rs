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

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use unicorn_engine::Unicorn;

use crate::{
    helper::{FAKE_IMPORT_BASE, HEAP_BASE},
    win32::{
        dll_advapi32::DllADVAPI32,
        dll_comctl32::DllCOMCTL32,
        dll_gdi32::DllGDI32,
        dll_imm32::DllIMM32,
        dll_kernel32::DllKERNEL32,
        dll_msvcp60::DllMSVCP60,
        dll_msvcrt::DllMSVCRT,
        dll_ole32::DllOle32,
        dll_shell32::DllSHELL32,
        dll_user32::DllUSER32,
        dll_winmm::DllWINMM,
        dll_ws2_32::DllWS2_32
    }
};

#[derive(Debug, Clone)]
pub struct LoadedDll {
    pub name: String,
    pub base_addr: u64,
    // pub size: usize,
    pub entry_point: u64,
    pub exports: HashMap<String, u64>,
}

// Unicorn 엔진이 품고 다닐 데이터 (User Data)
pub struct Win32Context {
    // 힙 메모리 할당을 위한 포인터 (다음에 쓸 빈 공간의 주소)
    pub heap_cursor: u64,
    // import 카운터
    pub import_address: u64,

    pub dll_modules: Rc<RefCell<HashMap<String, LoadedDll>>>,
    pub address_map: HashMap<u64, String>,
}

impl Win32Context {
    pub fn new() -> Self {
        Win32Context {
            heap_cursor: HEAP_BASE,
            import_address: FAKE_IMPORT_BASE,
            dll_modules: Rc::new(RefCell::new(HashMap::new())),
            address_map: HashMap::new(),
        }
    }

    pub fn handle(
        uc: &mut Unicorn<Win32Context>,
        dll_name: &str,
        func_name: &str
    ) -> Option<(usize, Option<i32>)> {
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
            _ => None,
        }
    }
}
