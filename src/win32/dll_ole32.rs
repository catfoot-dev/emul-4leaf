use unicorn_engine::Unicorn;

use crate::win32::{ApiHookResult, Win32Context, callee_result};

pub struct DllOle32 {}

impl DllOle32 {
    pub fn handle(_uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        callee_result(match func_name {
            "CoCreateInstance" => {
                println!("[OLE32] CoCreateInstance(...)");
                Some((5, Some(-2147467259i32))) // E_NOINTERFACE (0x80004002)
            }
            "CoInitialize" => {
                println!("[OLE32] CoInitialize(...)");
                Some((1, Some(0))) // S_OK
            }
            _ => {
                println!("[OLE32] UNHANDLED: {}", func_name);
                None
            }
        })
    }
}
