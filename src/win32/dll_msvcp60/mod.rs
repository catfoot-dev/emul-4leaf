use unicorn_engine::Unicorn;

use crate::win32::Win32Context;

mod std;

pub struct DllMSVCP60 {}

impl DllMSVCP60 {

    pub fn set_se_translator() -> Option<(usize, Option<i32>)>{
        //
        Some((0, Some(1)))
    }
    
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<(usize, Option<i32>)> {
        match func_name {
            // "?_set_se_translator@@YAP6AXIPAU_EXCEPTION_POINTERS@@@ZP6AXI0@Z@Z" => std::ios_base::init(),
            _ => None
        }
    }
}
