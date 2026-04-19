use crate::{
    dll::win32::{ApiHookResult, Win32Context},
    helper::UnicornHelper,
};
use unicorn_engine::Unicorn;

// API: void* malloc(size_t size)
// 역할: 지정된 데이터만큼 메모리를 할당
pub(super) fn malloc(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let size = uc.read_arg(0);
    let addr = uc.malloc(size as usize);
    crate::emu_log!("[MSVCRT] malloc({}) -> void* {:#x}", size, addr);
    Some(ApiHookResult::callee(1, Some(addr as i32)))
}

// API: void free(void* ptr)
// 역할: 할당된 메모리를 해제
pub(super) fn free(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let ptr = uc.read_arg(0);
    let released = ptr != 0 && uc.get_data().free_heap_block(ptr);
    crate::emu_log!("[MSVCRT] free({:#x}) -> void (released={})", ptr, released);
    Some(ApiHookResult::caller(None))
}

// API: void* calloc(size_t num, size_t size)
// 역할: 메모리를 할당하고 0으로 초기화
pub(super) fn calloc(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let num = uc.read_arg(0);
    let size = uc.read_arg(1);
    let total = (num * size) as usize;
    let addr = uc.malloc(total);
    if total > 0 {
        let zeros = vec![0u8; total];
        uc.mem_write(addr, &zeros).unwrap();
    }
    crate::emu_log!("[MSVCRT] calloc({}, {}) -> void* {:#x}", num, size, addr);
    Some(ApiHookResult::callee(2, Some(addr as i32)))
}

// API: void* realloc(void* ptr, size_t size)
// 역할: 이미 할당된 메모리의 크기를 조정
pub(super) fn realloc(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let ptr = uc.read_arg(0);
    let size = uc.read_arg(1) as usize;
    if size == 0 {
        if ptr != 0 {
            let _ = uc.get_data().free_heap_block(ptr);
        }
        crate::emu_log!("[MSVCRT] realloc({:#x}, 0) -> NULL", ptr);
        return Some(ApiHookResult::callee(2, Some(0)));
    }
    let addr = uc.malloc(size);
    if addr == 0 {
        crate::emu_log!("[MSVCRT] realloc({:#x}, {}) -> void* 0x0", ptr, size);
        return Some(ApiHookResult::callee(2, Some(0)));
    }
    if ptr != 0 {
        let copy_len = uc
            .get_data()
            .heap_block_size(ptr)
            .map(|block_size| (block_size as usize).min(size))
            .unwrap_or(size);
        let data = uc.mem_read_as_vec(ptr as u64, copy_len).unwrap_or_default();
        uc.mem_write(addr, &data).unwrap();
        let _ = uc.get_data().free_heap_block(ptr);
    }
    crate::emu_log!(
        "[MSVCRT] realloc({:#x}, {}) -> void* {:#x}",
        ptr,
        size,
        addr
    );
    Some(ApiHookResult::callee(2, Some(addr as i32)))
}

pub(super) fn new_op(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let size = uc.read_arg(0);
    let addr = uc.malloc(size as usize);
    crate::emu_log!("[MSVCRT] operator new({}) -> void* {:#x}", size, addr);
    Some(ApiHookResult::callee(1, Some(addr as i32)))
}

// API: void* memmove(void* dest, const void* src, size_t count)
// 역할: 메모리 블록을 다른 위치로 복사 (겹침 허용)
pub(super) fn memmove(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let dst = uc.read_arg(0);
    let src = uc.read_arg(1);
    let size = uc.read_arg(2) as usize;
    if size > 0 {
        let data = uc.mem_read_as_vec(src as u64, size).unwrap_or_default();
        uc.mem_write(dst as u64, &data).unwrap();
    }
    crate::emu_log!(
        "[MSVCRT] memmove({:#x}, {:#x}, {}) -> void* {:#x}",
        dst,
        src,
        size,
        dst
    );
    Some(ApiHookResult::callee(3, Some(dst as i32)))
}

// API: void* memchr(const void* ptr, int ch, size_t count)
// 역할: 메모리에서 특정 문자를 검색
pub(super) fn memchr(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let buf = uc.read_arg(0);
    let ch = uc.read_arg(1) as u8;
    let count = uc.read_arg(2) as usize;
    let data = uc.mem_read_as_vec(buf as u64, count).unwrap_or_default();
    let result = data
        .iter()
        .position(|&b| b == ch)
        .map(|pos| buf + pos as u32)
        .unwrap_or(0);
    crate::emu_log!(
        "[MSVCRT] memchr({:#x}, {}, {}) -> void* {:#x}",
        buf,
        ch,
        count,
        result
    );
    Some(ApiHookResult::callee(3, Some(result as i32)))
}
