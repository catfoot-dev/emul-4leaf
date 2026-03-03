use unicorn_engine::Unicorn;

use crate::win32::Win32Context;

pub struct DllADVAPI32 {}

impl DllADVAPI32 {
    pub fn reg_query_value_ex_a() -> Option<(usize, Option<i32>)>{
        println!("reg_query_value_ex_a");
        Some((0, None))
    }

    pub fn reg_open_key_ex_a() -> Option<(usize, Option<i32>)>{
        println!("reg_open_key_ex_a");
        Some((0, None))
    }

    pub fn reg_close_key() -> Option<(usize, Option<i32>)>{
        println!("reg_close_key");
        Some((0, None))
    }

    pub fn reg_create_key_ex_a() -> Option<(usize, Option<i32>)>{
        println!("reg_create_key_ex_a");
        Some((0, None))
    }

    pub fn reg_set_value_ex_a() -> Option<(usize, Option<i32>)>{
        println!("reg_set_value_ex_a");
        Some((0, None))
    }

    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<(usize, Option<i32>)> {
        match func_name {
            "RegQueryValueExA" => DllADVAPI32::reg_query_value_ex_a(),
            "RegOpenKeyExA" => DllADVAPI32::reg_open_key_ex_a(),
            "RegCloseKey" => DllADVAPI32::reg_close_key(),
            "RegCreateKeyExA" => DllADVAPI32::reg_create_key_ex_a(),
            "RegSetValueExA" => DllADVAPI32::reg_set_value_ex_a(),
            _ => None
        }
    }
}
