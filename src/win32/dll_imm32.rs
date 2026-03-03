use unicorn_engine::Unicorn;

use crate::win32::Win32Context;

pub struct DllIMM32 {}

impl DllIMM32 {
    pub fn imm_is_ui_message_a() -> Option<(usize, Option<i32>)>{
        println!("imm_is_ui_message_a");
        Some((0, None))
    }

    pub fn imm_get_conversion_status() -> Option<(usize, Option<i32>)>{
        println!("imm_get_conversion_status");
        Some((0, None))
    }

    pub fn imm_get_context() -> Option<(usize, Option<i32>)>{
        println!("imm_get_context");
        Some((0, None))
    }

    pub fn imm_release_context() -> Option<(usize, Option<i32>)>{
        println!("imm_release_context");
        Some((0, None))
    }

    pub fn imm_set_conversion_status() -> Option<(usize, Option<i32>)>{
        println!("imm_set_conversion_status");
        Some((0, None))
    }


    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<(usize, Option<i32>)> {
        match func_name {
            "ImmIsUIMessageA" => DllIMM32::imm_is_ui_message_a(),
            "ImmGetConversionStatus" => DllIMM32::imm_get_conversion_status(),
            "ImmGetContext" => DllIMM32::imm_get_context(),
            "ImmReleaseContext" => DllIMM32::imm_release_context(),
            "ImmSetConversionStatus" => DllIMM32::imm_set_conversion_status(),
            _ => None
        }
    }
}
