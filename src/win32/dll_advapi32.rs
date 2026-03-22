use unicorn_engine::Unicorn;

use crate::helper::UnicornHelper;
use crate::win32::{ApiHookResult, Win32Context, callee_result};

/// `ADVAPI32.dll` 프록시 구현 모듈
///
/// 레지스트리(Registry) 및 고급 시스템 API 호출에 대한 가짜 응답을 제공
pub struct DllADVAPI32;

impl DllADVAPI32 {
    /// 함수명 기준 `ADVAPI32.dll` API 구현체
    ///
    /// 처리를 성공했다면 스택 보정값과 리턴값을 포함한 `ApiHookResult`를 반환
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        callee_result(match func_name {
            // API: LONG RegQueryValueExA(HKEY hKey, LPCSTR lpValueName, LPDWORD lpReserved, LPDWORD lpType, LPBYTE lpData, LPDWORD lpcbData)
            // 역할: 지정된 레지스트리 키에 연관된 특정 값의 유형과 데이터를 쿼리
            "RegQueryValueExA" => {
                let hkey = uc.read_arg(0);
                let name_addr = uc.read_arg(1);
                let reserved = uc.read_arg(2);
                let lp_type = uc.read_arg(3);
                let lp_data = uc.read_arg(4);
                let lpcb_data = uc.read_arg(5);
                let name = if name_addr != 0 {
                    uc.read_euc_kr(name_addr as u64)
                } else {
                    String::new()
                };
                crate::emu_log!(
                    "[ADVAPI32] RegQueryValueExA({:#x}, {:#x}=\"{}\", {}, {}, {:#x}, {:#x}) -> {:#x}",
                    hkey,
                    name_addr,
                    name,
                    reserved,
                    lp_type,
                    lp_data,
                    lpcb_data,
                    2
                );
                Some((6, Some(2))) // ERROR_FILE_NOT_FOUND
            }

            // API: LONG RegOpenKeyExA(HKEY hKey, LPCSTR lpSubKey, DWORD ulOptions, REGSAM samDesired, PHKEY phkResult)
            // 역할: 지정된 레지스트리 키를 오픈
            "RegOpenKeyExA" => {
                let hkey = uc.read_arg(0);
                let subkey_addr = uc.read_arg(1);
                let subkey = uc.read_euc_kr(subkey_addr as u64);
                let options = uc.read_arg(2);
                let sam_desired = uc.read_arg(3);
                let result_addr = uc.read_arg(4);
                let ctx = uc.get_data();
                let handle = ctx.alloc_handle();
                if result_addr != 0 {
                    uc.write_u32(result_addr as u64, handle);
                }
                crate::emu_log!(
                    "[ADVAPI32] RegOpenKeyExA({:#x}, {:#x}=\"{}\", {}, {}, {:#x}) -> {:#x}",
                    hkey,
                    subkey_addr,
                    subkey,
                    options,
                    sam_desired,
                    result_addr,
                    handle
                );
                Some((5, Some(0))) // ERROR_SUCCESS
            }

            // API: LONG RegCloseKey(HKEY hKey)
            // 역할: 지정된 레지스트리 키의 핸들을 닫음
            "RegCloseKey" => {
                let hkey = uc.read_arg(0);
                crate::emu_log!("[ADVAPI32] RegCloseKey({:#x}) -> {:#x}", hkey, 0);
                Some((1, Some(0))) // ERROR_SUCCESS
            }

            // API: LONG RegCreateKeyExA(HKEY hKey, LPCSTR lpSubKey, DWORD Reserved, LPSTR lpClass, DWORD dwOptions, REGSAM samDesired, LPSECURITY_ATTRIBUTES lpSecurityAttributes, PHKEY phkResult, LPDWORD lpdwDisposition)
            // 역할: 지정된 레지스트리 키를 생성하거나 이미 존재하면 오픈
            "RegCreateKeyExA" => {
                let hkey = uc.read_arg(0);
                let subkey_addr = uc.read_arg(1);
                let subkey = uc.read_euc_kr(subkey_addr as u64);
                let reserved = uc.read_arg(2);
                let lp_class = uc.read_arg(3);
                let dw_options = uc.read_arg(4);
                let sam_desired = uc.read_arg(5);
                let lp_security_attributes = uc.read_arg(6);
                let result_addr = uc.read_arg(7);
                let ctx = uc.get_data();
                let handle = ctx.alloc_handle();
                if result_addr != 0 {
                    uc.write_u32(result_addr as u64, handle);
                }
                crate::emu_log!(
                    "[ADVAPI32] RegCreateKeyExA({:#x}, {:#x}=\"{}\", {}, {}, {}, {}, {}, {:#x}) -> {:#x}",
                    hkey,
                    subkey_addr,
                    subkey,
                    reserved,
                    lp_class,
                    dw_options,
                    sam_desired,
                    lp_security_attributes,
                    result_addr,
                    handle
                );
                Some((9, Some(0))) // ERROR_SUCCESS
            }

            // API: LONG RegSetValueExA(HKEY hKey, LPCSTR lpValueName, DWORD Reserved, DWORD dwType, const BYTE *lpData, DWORD cbData)
            // 역할: 레지스트리 키의 지정된 값의 데이터와 유형을 설정
            "RegSetValueExA" => {
                let hkey = uc.read_arg(0);
                let name_addr = uc.read_arg(1);
                let name = if name_addr != 0 {
                    uc.read_euc_kr(name_addr as u64)
                } else {
                    String::new()
                };
                crate::emu_log!(
                    "[ADVAPI32] RegSetValueExA({:#x}, {:#x}=\"{}\") -> {:#x}",
                    hkey,
                    name_addr,
                    name,
                    0
                );
                Some((6, Some(0))) // ERROR_SUCCESS
            }

            _ => {
                crate::emu_log!("[ADVAPI32] UNHANDLED: {}", func_name);
                None
            }
        })
    }
}
