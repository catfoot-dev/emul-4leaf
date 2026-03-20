use unicorn_engine::Unicorn;

use crate::win32::{ApiHookResult, Win32Context, callee_result};

pub struct DllSHELL32 {}

impl DllSHELL32 {
    pub fn handle(_uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        callee_result(match func_name {
            "ShellExecuteA" => {
                println!("[SHELL32] ShellExecuteA(...)");
                Some((6, Some(42))) // > 32 = 성공
            }
            _ => {
                println!("[SHELL32] UNHANDLED: {}", func_name);
                None
            }
        })
    }
}
