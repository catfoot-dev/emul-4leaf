use unicorn_engine::Unicorn;

use crate::win32::{ApiHookResult, Win32Context, callee_result};

pub struct DllWINMM {}

impl DllWINMM {
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        callee_result(match func_name {
            "timeGetTime" => {
                let elapsed = uc.get_data_mut().start_time.elapsed().as_millis() as u32;
                Some((0, Some(elapsed as i32)))
            }
            _ => {
                println!("[WINMM] UNHANDLED: {}", func_name);
                None
            }
        })
    }
}
