use unicorn_engine::Unicorn;

use crate::win32::Win32Context;

pub struct DllSHELL32 {}

impl DllSHELL32 {
    pub fn shell_execute_a() -> Option<(usize, Option<i32>)>{
        println!("shell_execute_a");
        Some((0, None))
    }


    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<(usize, Option<i32>)> {
        match func_name {
            "ShellExecuteA" => DllSHELL32::shell_execute_a(),
            _ => None
        }
    }
}
