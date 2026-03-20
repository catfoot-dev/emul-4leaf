use unicorn_engine::Unicorn;

use crate::helper::UnicornHelper;
use crate::win32::{ApiHookResult, Win32Context, callee_result};

pub struct DllIMM32 {}

impl DllIMM32 {
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        callee_result(match func_name {
            "ImmIsUIMessageA" => {
                println!("[IMM32] ImmIsUIMessageA(...)");
                Some((4, Some(0))) // FALSE
            }
            "ImmGetConversionStatus" => {
                println!("[IMM32] ImmGetConversionStatus(...)");
                Some((3, Some(0))) // FALSE
            }
            "ImmGetContext" => {
                let _hwnd = uc.read_arg(0);
                let ctx = uc.get_data_mut();
                let himc = ctx.alloc_handle();
                println!("[IMM32] ImmGetContext(...) -> {:#x}", himc);
                Some((1, Some(himc as i32)))
            }
            "ImmReleaseContext" => {
                println!("[IMM32] ImmReleaseContext(...)");
                Some((2, Some(1))) // TRUE
            }
            "ImmSetConversionStatus" => {
                println!("[IMM32] ImmSetConversionStatus(...)");
                Some((3, Some(1))) // TRUE
            }
            _ => {
                println!("[IMM32] UNHANDLED: {}", func_name);
                None
            }
        })
    }
}
