use unicorn_engine::Unicorn;

use crate::win32::{ApiHookResult, Win32Context, callee_result};

pub struct DllCOMCTL32 {}

impl DllCOMCTL32 {
    pub fn handle(_uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        callee_result(match func_name {
            "_TrackMouseEvent" => {
                println!("[COMCTL32] _TrackMouseEvent(...)");
                Some((1, Some(1))) // TRUE
            }
            "InitCommonControlsEx" => {
                println!("[COMCTL32] InitCommonControlsEx(...)");
                Some((1, Some(1))) // TRUE
            }
            _ => {
                println!("[COMCTL32] UNHANDLED: {}", func_name);
                None
            }
        })
    }
}
