use unicorn_engine::Unicorn;

use crate::helper::UnicornHelper;
use crate::win32::{ApiHookResult, Win32Context, callee_result};

pub struct DllADVAPI32 {}

impl DllADVAPI32 {
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        callee_result(match func_name {
            "RegQueryValueExA" => {
                let _hkey = uc.read_arg(0);
                let name_addr = uc.read_arg(1);
                let name = if name_addr != 0 {
                    uc.read_string(name_addr as u64)
                } else {
                    String::new()
                };
                println!("[ADVAPI32] RegQueryValueExA(\"{}\")", name);
                Some((6, Some(2))) // ERROR_FILE_NOT_FOUND
            }
            "RegOpenKeyExA" => {
                let _hkey = uc.read_arg(0);
                let subkey_addr = uc.read_arg(1);
                let subkey = uc.read_string(subkey_addr as u64);
                let result_addr = uc.read_arg(4);
                let ctx = uc.get_data_mut();
                let handle = ctx.alloc_handle();
                if result_addr != 0 {
                    uc.write_u32(result_addr as u64, handle);
                }
                println!("[ADVAPI32] RegOpenKeyExA(\"{}\") -> {:#x}", subkey, handle);
                Some((5, Some(0))) // ERROR_SUCCESS
            }
            "RegCloseKey" => {
                println!("[ADVAPI32] RegCloseKey(...)");
                Some((1, Some(0))) // ERROR_SUCCESS
            }
            "RegCreateKeyExA" => {
                let _hkey = uc.read_arg(0);
                let subkey_addr = uc.read_arg(1);
                let subkey = uc.read_string(subkey_addr as u64);
                let result_addr = uc.read_arg(7);
                let ctx = uc.get_data_mut();
                let handle = ctx.alloc_handle();
                if result_addr != 0 {
                    uc.write_u32(result_addr as u64, handle);
                }
                println!(
                    "[ADVAPI32] RegCreateKeyExA(\"{}\") -> {:#x}",
                    subkey, handle
                );
                Some((9, Some(0))) // ERROR_SUCCESS
            }
            "RegSetValueExA" => {
                let _hkey = uc.read_arg(0);
                let name_addr = uc.read_arg(1);
                let name = if name_addr != 0 {
                    uc.read_string(name_addr as u64)
                } else {
                    String::new()
                };
                println!("[ADVAPI32] RegSetValueExA(\"{}\")", name);
                Some((6, Some(0))) // ERROR_SUCCESS
            }
            _ => {
                println!("[ADVAPI32] UNHANDLED: {}", func_name);
                None
            }
        })
    }
}
