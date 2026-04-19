use crate::{
    dll::win32::{ApiHookResult, Win32Context},
    helper::UnicornHelper,
};
use unicorn_engine::Unicorn;

use super::GMEM_ZEROINIT;

// =========================================================
// Memory
// =========================================================
// API: HGLOBAL GlobalAlloc(UINT uFlags, SIZE_T dwBytes)
// 역할: 힙에서 지정된 바이트의 메모리를 할당
pub(super) fn global_alloc(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let flags = uc.read_arg(0);
    let size = uc.read_arg(1);
    let addr = uc.malloc(size as usize);
    // `GMEM_ZEROINIT`일 때만 원본과 같이 0으로 초기화합니다.
    if addr != 0 && flags & GMEM_ZEROINIT != 0 {
        let zeros = vec![0u8; size as usize];
        uc.mem_write(addr, &zeros).unwrap();
    }
    crate::emu_log!(
        "[KERNEL32] GlobalAlloc({:#x}, {}) -> HGLOBAL {:#x}",
        flags,
        size,
        addr
    );
    Some(ApiHookResult::callee(2, Some(addr as i32)))
}

// API: LPVOID GlobalLock(HGLOBAL hMem)
// 역할: 메모리를 고정하여 첫 바이트에 대한 포인터를 반환
pub(super) fn global_lock(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let handle = uc.read_arg(0);
    // 핸들 = 메모리 포인터로 취급
    crate::emu_log!(
        "[KERNEL32] GlobalLock({:#x}) -> LPVOID {:#x}",
        handle,
        handle
    );
    Some(ApiHookResult::callee(1, Some(handle as i32)))
}

// API: BOOL GlobalUnlock(HGLOBAL hMem)
// 역할: GlobalLock에 의해 잠긴 메모리를 해제
pub(super) fn global_unlock(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let handle = uc.read_arg(0);
    crate::emu_log!("[KERNEL32] GlobalUnlock({:#x}) -> BOOL 1", handle);
    Some(ApiHookResult::callee(1, Some(1)))
}

// API: HGLOBAL GlobalFree(HGLOBAL hMem)
// 역할: 지정된 전역 메모리 개체를 해제
pub(super) fn global_free(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let handle = uc.read_arg(0);
    let released = handle != 0 && uc.get_data().free_heap_block(handle);
    crate::emu_log!(
        "[KERNEL32] GlobalFree({:#x}) -> HGLOBAL 0 (released={})",
        handle,
        released
    );
    Some(ApiHookResult::callee(1, Some(0))) // 성공 시 NULL
}
