use crate::{
    dll::win32::{ApiHookResult, Win32Context},
    helper::UnicornHelper,
};
use unicorn_engine::Unicorn;

// =========================================================
// Module
// =========================================================
// API: HMODULE GetModuleHandleA(LPCSTR lpModuleName)
// 역할: 호출하는 프로세스에 이미 로드된 모듈 핸들을 검색
pub(super) fn get_module_handle_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let name_addr = uc.read_arg(0);
    if name_addr == 0 {
        // NULL = 현재 실행 모듈 (4Leaf.dll의 베이스)
        crate::emu_log!("[KERNEL32] GetModuleHandleA(NULL) -> HMODULE 0x35000000");
        Some(ApiHookResult::callee(1, Some(0x3500_0000u32 as i32)))
    } else {
        let name = uc.read_euc_kr(name_addr as u64);
        // 로드된 DLL에서 찾기
        let ctx = uc.get_data();
        let mut found_base: u32 = 0;
        let modules = ctx.dll_modules.lock().unwrap();
        for (dll_name, dll) in modules.iter() {
            if dll_name.eq_ignore_ascii_case(&name) || dll.name.ends_with(&name) {
                found_base = dll.base_addr as u32;
                break;
            }
        }
        crate::emu_log!(
            "[KERNEL32] GetModuleHandleA(\"{}\") -> HMODULE {:#x}",
            name,
            found_base
        );
        Some(ApiHookResult::callee(1, Some(found_base as i32)))
    }
}

// API: DWORD GetModuleFileNameA(HMODULE hModule, LPSTR lpFilename, DWORD nSize)
// 역할: 모듈이 포함된 실행 파일의 절대 경로를 조회
pub(super) fn get_module_file_name_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let module = uc.read_arg(0);
    let buf_addr = uc.read_arg(1);
    let buf_size = uc.read_arg(2);
    let path = format!("{}\\4Leaf.exe\0", crate::resource_dir().display()).replace('/', "\\");
    let bytes = path.as_bytes();
    let copy_len = bytes.len().min(buf_size as usize);
    uc.mem_write(buf_addr as u64, &bytes[..copy_len]).unwrap();
    crate::emu_log!(
        "[KERNEL32] GetModuleFileNameA({:#x}, {:#x}, {}) -> DWORD \"{}\"",
        module,
        buf_addr,
        buf_size,
        &path[..path.len() - 1]
    );
    Some(ApiHookResult::callee(3, Some((copy_len - 1) as i32)))
}

// API: HMODULE LoadLibraryA(LPCSTR lpLibFileName)
// 역할: 지정된 모듈을 호출 컨텍스트의 주소 공간으로 매핑
pub(super) fn load_library_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let name_addr = uc.read_arg(0);
    let name = if name_addr != 0 {
        uc.read_euc_kr(name_addr as u64)
    } else {
        String::new()
    };
    // 이미 로드된 DLL이면 핸들 반환
    let ctx = uc.get_data();
    let mut found_base: u32 = 0;
    let modules = ctx.dll_modules.lock().unwrap();
    for (dll_name, dll) in modules.iter() {
        if dll_name.eq_ignore_ascii_case(&name) {
            found_base = dll.base_addr as u32;
            break;
        }
    }
    crate::emu_log!(
        "[KERNEL32] LoadLibraryA(\"{}\") -> HMODULE {:#x}",
        name,
        found_base
    );
    Some(ApiHookResult::callee(1, Some(found_base as i32)))
}

// API: BOOL FreeLibrary(HMODULE hLibModule)
// 역할: 로드된 DLL 모듈을 해제
pub(super) fn free_library(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let module = uc.read_arg(0);
    crate::emu_log!("[KERNEL32] FreeLibrary({:#x}) -> BOOL 1", module);
    Some(ApiHookResult::callee(1, Some(1)))
}

// API: FARPROC GetProcAddress(HMODULE hModule, LPCSTR lpProcName)
// 역할: DLL에서 지정된 익스포트 함수의 주소를 조회
pub(super) fn get_proc_address(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let module = uc.read_arg(0);
    let name_addr = uc.read_arg(1);
    let name = if name_addr != 0 {
        uc.read_euc_kr(name_addr as u64)
    } else {
        String::new()
    };
    let dll_name = {
        let ctx = uc.get_data();
        let modules = ctx.dll_modules.lock().unwrap();
        modules.iter().find_map(|(dll_name, dll)| {
            ((dll.base_addr as u32) == module).then_some(dll_name.clone())
        })
    };
    let proc_addr = if let Some(dll_name) = dll_name {
        let loaded_export = {
            let ctx = uc.get_data();
            let modules = ctx.dll_modules.lock().unwrap();
            modules
                .get(&dll_name)
                .and_then(|dll| dll.exports.get(&name).copied())
                .map(|addr| addr as u32)
        };
        loaded_export.or_else(|| Win32Context::resolve_proxy_export(uc, &dll_name, &name))
    } else {
        None
    }
    .unwrap_or(0);
    crate::emu_log!(
        "[KERNEL32] GetProcAddress({:#x}, \"{}\") -> FARPROC {:#x}",
        module,
        name,
        proc_addr
    );
    Some(ApiHookResult::callee(2, Some(proc_addr as i32)))
}

// API: BOOL DisableThreadLibraryCalls(HMODULE hLibModule)
// 역할: DLL의 스레드 부착/분리 알림을 비활성화
pub(super) fn disable_thread_library_calls(
    _uc: &mut Unicorn<Win32Context>,
) -> Option<ApiHookResult> {
    let h_lib_module = _uc.read_arg(0);
    crate::emu_log!(
        "[KERNEL32] DisableThreadLibraryCalls({:#x}) -> BOOL 1",
        h_lib_module
    );
    Some(ApiHookResult::callee(1, Some(1))) // TRUE
}
