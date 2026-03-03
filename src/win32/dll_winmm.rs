use unicorn_engine::Unicorn;

use crate::win32::Win32Context;

pub struct DllWINMM {}

impl DllWINMM {
    pub fn time_get_time() -> Option<(usize, Option<i32>)>{
        println!("time_get_time");
        Some((0, None))
    }

    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<(usize, Option<i32>)> {
        match func_name {
            "timeGetTime" => DllWINMM::time_get_time(),
            _ => None
        }
    }
}
