use unicorn_engine::Unicorn;

use crate::helper::UnicornHelper;
use crate::win32::{ApiHookResult, Win32Context, callee_result};

/// `ADVAPI32.dll` 프록시 구현 모듈
///
/// 레지스트리(Registry) 및 고급 시스템 API 호출에 대한 가짜 응답을 제공
pub struct DllADVAPI32;

impl DllADVAPI32 {
    // API: LONG RegQueryValueExA(HKEY hKey, LPCSTR lpValueName, LPDWORD lpReserved, LPDWORD lpType, LPBYTE lpData, LPDWORD lpcbData)
    // 역할: 레지스트리 키의 값을 읽어오는 함수
    pub fn reg_query_value_ex_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hkey = uc.read_arg(0);
        let name_addr = uc.read_arg(1);
        let reserved = uc.read_arg(2);
        let lp_type = uc.read_arg(3);
        let lp_data = uc.read_arg(4);
        let lpcb_data = uc.read_arg(5);

        let value_name = if name_addr != 0 {
            uc.read_euc_kr(name_addr as u64)
        } else {
            String::new()
        };

        let reg_data = {
            let ctx = uc.get_data();
            let handles = ctx.registry_handles.lock().unwrap();
            let path = handles
                .get(&hkey)
                .cloned()
                .unwrap_or_else(|| "UNKNOWN".to_string());
            let full_path = format!("{}\\{}", path, value_name);

            ctx.registry.lock().unwrap().get(&full_path).cloned()
        };

        let ret = if let Some(data) = reg_data {
            if lp_type != 0 {
                uc.write_u32(lp_type as u64, 1); // REG_SZ (simplified)
            }
            if lpcb_data != 0 {
                let buf_size = uc.read_u32(lpcb_data as u64);
                uc.write_u32(lpcb_data as u64, data.len() as u32);
                if lp_data != 0 && buf_size >= data.len() as u32 {
                    uc.mem_write(lp_data as u64, &data).unwrap();
                }
            }
            0 // ERROR_SUCCESS
        } else {
            2 // ERROR_FILE_NOT_FOUND
        };

        crate::emu_log!(
            "[ADVAPI32] RegQueryValueExA({:#x}, \"{}\", {:#x}, {:#x}, {:#x}, {:#x}) -> LONG {}",
            hkey,
            value_name,
            reserved,
            lp_type,
            lp_data,
            lpcb_data,
            ret
        );
        Some((6, Some(ret)))
    }

    // API: LONG RegCreateKeyExA(HKEY hKey, LPCSTR lpSubKey, DWORD Reserved, LPSTR lpClass, DWORD dwOptions, REGSAM samDesired, LPSECURITY_ATTRIBUTES lpSecurityAttributes, PHKEY phkResult, LPDWORD lpdwDisposition)
    // 역할: 레지스트리 키를 생성하거나 여는 함수
    pub fn reg_create_key_ex_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hkey = uc.read_arg(0);
        let subkey_addr = uc.read_arg(1);
        let subkey = if subkey_addr != 0 {
            uc.read_euc_kr(subkey_addr as u64)
        } else {
            String::new()
        };
        let reserved = uc.read_arg(2);
        let lp_class = uc.read_arg(3);
        let dw_options = uc.read_arg(4);
        let sam_desired = uc.read_arg(5);
        let lp_security_attributes = uc.read_arg(6);
        let result_addr = uc.read_arg(7);
        let lpdw_disposition = uc.read_arg(8);

        let (new_handle, new_path) = {
            let ctx = uc.get_data();
            let mut handles = ctx.registry_handles.lock().unwrap();
            let parent_path = handles
                .get(&hkey)
                .cloned()
                .unwrap_or_else(|| "UNKNOWN".to_string());
            let new_path = if subkey.is_empty() {
                parent_path
            } else {
                format!("{}\\{}", parent_path, subkey)
            };

            let new_handle = ctx.alloc_handle();
            handles.insert(new_handle, new_path.clone());
            (new_handle, new_path)
        };

        if result_addr != 0 {
            uc.write_u32(result_addr as u64, new_handle);
        }
        crate::emu_log!(
            "[ADVAPI32] RegCreateKeyExA({:#x}, \"{}\", {:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> LONG {:#x} (HKEY \"{}\")",
            hkey,
            subkey,
            reserved,
            lp_class,
            dw_options,
            sam_desired,
            lp_security_attributes,
            result_addr,
            lpdw_disposition,
            new_handle,
            new_path
        );
        Some((9, Some(0))) // ERROR_SUCCESS
    }

    // API: LONG RegOpenKeyExA(HKEY hKey, LPCSTR lpSubKey, DWORD ulOptions, REGSAM samDesired, PHKEY phkResult)
    // 역할: 레지스트리 키를 여는 함수
    pub fn reg_open_key_ex_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hkey = uc.read_arg(0);
        let subkey_addr = uc.read_arg(1);
        let subkey = if subkey_addr != 0 {
            uc.read_euc_kr(subkey_addr as u64)
        } else {
            String::new()
        };
        let ul_options = uc.read_arg(2);
        let sam_desired = uc.read_arg(3);
        let result_addr = uc.read_arg(4);

        let (new_handle, new_path) = {
            let ctx = uc.get_data();
            let mut handles = ctx.registry_handles.lock().unwrap();
            let parent_path = handles
                .get(&hkey)
                .cloned()
                .unwrap_or_else(|| "UNKNOWN".to_string());
            let new_path = if subkey.is_empty() {
                parent_path
            } else {
                format!("{}\\{}", parent_path, subkey)
            };

            let new_handle = ctx.alloc_handle();
            handles.insert(new_handle, new_path.clone());
            (new_handle, new_path)
        };

        if result_addr != 0 {
            uc.write_u32(result_addr as u64, new_handle);
        }
        crate::emu_log!(
            "[ADVAPI32] RegOpenKeyExA({:#x}, \"{}\", {:#x}, {:#x}, {:#x}) -> LONG {:#x} (HKEY \"{}\")",
            hkey,
            subkey,
            ul_options,
            sam_desired,
            result_addr,
            new_handle,
            new_path
        );
        Some((5, Some(0))) // ERROR_SUCCESS
    }

    // API: LONG RegCloseKey(HKEY hKey)
    // 역할: 열려 있는 레지스트리 키를 닫는 함수
    pub fn reg_close_key(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hkey = uc.read_arg(0);
        uc.get_data().registry_handles.lock().unwrap().remove(&hkey);
        crate::emu_log!("[ADVAPI32] RegCloseKey({:#x}) -> LONG 0", hkey);
        Some((1, Some(0))) // ERROR_SUCCESS
    }

    // API: LONG RegSetValueExA(HKEY hKey, LPCSTR lpValueName, DWORD Reserved, DWORD dwType, const BYTE *lpData, DWORD cbData)
    // 역할: 레지스트리 키에 값을 설정하는 함수
    pub fn reg_set_value_ex_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hkey = uc.read_arg(0);
        let name_addr = uc.read_arg(1);
        let reserved = uc.read_arg(2);
        let lp_type = uc.read_arg(3);
        let lp_data = uc.read_arg(4);
        let cb_data = uc.read_arg(5);

        let value_name = if name_addr != 0 {
            uc.read_euc_kr(name_addr as u64)
        } else {
            String::new()
        };

        let reg_data = {
            let ctx = uc.get_data();
            let handles = ctx.registry_handles.lock().unwrap();
            let path = handles
                .get(&hkey)
                .cloned()
                .unwrap_or_else(|| "UNKNOWN".to_string());
            let full_path = format!("{}\\{}", path, value_name);

            ctx.registry.lock().unwrap().get(&full_path).cloned()
        };

        let ret = if let Some(data) = reg_data {
            if lp_type != 0 {
                uc.write_u32(lp_type as u64, 1); // REG_SZ (simplified)
            }
            if cb_data != 0 {
                let buf_size = uc.read_u32(cb_data as u64);
                uc.write_u32(cb_data as u64, data.len() as u32);
                if lp_data != 0 && buf_size >= data.len() as u32 {
                    uc.mem_write(lp_data as u64, &data).unwrap();
                }
            }
            0 // ERROR_SUCCESS
        } else {
            2 // ERROR_FILE_NOT_FOUND
        };

        crate::emu_log!(
            "[ADVAPI32] RegSetValueExA({:#x}, \"{}\", {:#x}, {:#x}, {:#x}, {:#x}) -> LONG {}",
            hkey,
            value_name,
            reserved,
            lp_type,
            lp_data,
            cb_data,
            ret
        );
        Some((6, Some(ret)))
    }

    /// 함수명 기준 `ADVAPI32.dll` API 구현체
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        callee_result(match func_name {
            "RegQueryValueExA" => Self::reg_query_value_ex_a(uc),
            "RegOpenKeyExA" => Self::reg_open_key_ex_a(uc),
            "RegCloseKey" => Self::reg_close_key(uc),
            "RegCreateKeyExA" => Self::reg_create_key_ex_a(uc),
            "RegSetValueExA" => Self::reg_set_value_ex_a(uc),
            _ => {
                crate::emu_log!("[!] ADVAPI32 Unhandled: {}", func_name);
                None
            }
        })
    }
}
