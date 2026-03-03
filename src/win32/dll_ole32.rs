use unicorn_engine::Unicorn;

use crate::win32::Win32Context;

pub struct DllOle32 {}

impl DllOle32 {
    pub fn co_create_instance() -> Option<(usize, Option<i32>)>{
        println!("co_create_instance");
        Some((0, None))
    }

    pub fn co_initialize() -> Option<(usize, Option<i32>)>{
        println!("co_initialize");
        Some((0, None))
    }

    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<(usize, Option<i32>)> {
        match func_name {
            "CoCreateInstance" => DllOle32::co_create_instance(),
            "CoInitialize" => DllOle32::co_initialize(),
            _ => None
        }
    }
}
