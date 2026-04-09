use crate::{
    dll::win32::{ApiHookResult, Win32Context},
    helper::UnicornHelper,
};
use unicorn_engine::Unicorn;

use super::MSVCP60;

pub(super) fn basic_string_ctor_default(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let allocator = uc.read_arg(0);
    MSVCP60::init_basic_string_empty(uc, this_ptr);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_string::basic_string({:#x}) -> (this={:#x})",
        this_ptr,
        allocator,
        this_ptr
    );
    Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
}

pub(super) fn basic_string_ctor_cstr(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let str_ptr = uc.read_arg(0);
    let allocator = uc.read_arg(1);
    let bytes = MSVCP60::source_bytes_from_ptr(uc, str_ptr, None);
    MSVCP60::set_basic_string_bytes(uc, this_ptr, &bytes);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_string::basic_string({:#x}, {:#x}) -> (this={:#x})",
        this_ptr,
        str_ptr,
        allocator,
        this_ptr
    );
    Some(ApiHookResult::callee(2, Some(this_ptr as i32)))
}

pub(super) fn basic_string_destructor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    MSVCP60::init_basic_string_empty(uc, this_ptr);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_string::~basic_string()",
        this_ptr
    );
    Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
}

pub(super) fn basic_string_tidy(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let preserve = uc.read_arg(0);
    MSVCP60::init_basic_string_empty(uc, this_ptr);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_string::_Tidy({}) -> VOID",
        this_ptr,
        preserve
    );
    Some(ApiHookResult::callee(1, None))
}

pub(super) fn basic_string_grow(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let size = uc.read_arg(0) as usize;
    let preserve = uc.read_arg(1) != 0;
    MSVCP60::ensure_basic_string_capacity(uc, this_ptr, size, preserve);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_string::_Grow({}, {}) -> BOOL 1",
        this_ptr,
        size,
        preserve
    );
    Some(ApiHookResult::callee(2, Some(1)))
}

pub(super) fn basic_string_copy(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let size = uc.read_arg(0) as usize;
    MSVCP60::ensure_basic_string_capacity(uc, this_ptr, size, true);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_string::_Copy({}) -> VOID",
        this_ptr,
        size
    );
    Some(ApiHookResult::callee(1, None))
}

pub(super) fn basic_string_eos(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let size = uc.read_arg(0) as usize;
    let mut current = MSVCP60::basic_string_bytes(uc, this_ptr);
    current.resize(size, 0);
    MSVCP60::set_basic_string_bytes(uc, this_ptr, &current);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_string::_Eos({}) -> VOID",
        this_ptr,
        size
    );
    Some(ApiHookResult::callee(1, None))
}

pub(super) fn basic_string_freeze(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    crate::emu_log!("[MSVCP60] (this={:#x}) basic_string::_Freeze()", this_ptr);
    Some(ApiHookResult::callee(0, None))
}

pub(super) fn basic_string_split(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    crate::emu_log!("[MSVCP60] (this={:#x}) basic_string::_Split()", this_ptr);
    Some(ApiHookResult::callee(0, None))
}

pub(super) fn basic_string_assign_ptr_len(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let ptr = uc.read_arg(0);
    let len = uc.read_arg(1) as usize;
    let bytes = MSVCP60::source_bytes_from_ptr(uc, ptr, Some(len));
    MSVCP60::set_basic_string_bytes(uc, this_ptr, &bytes);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_string::assign({:#x}, {}) -> (this={:#x})",
        this_ptr,
        ptr,
        len,
        this_ptr
    );
    Some(ApiHookResult::callee(2, Some(this_ptr as i32)))
}

pub(super) fn basic_string_assign_ptr(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let ptr = uc.read_arg(0);
    let bytes = MSVCP60::source_bytes_from_ptr(uc, ptr, None);
    MSVCP60::set_basic_string_bytes(uc, this_ptr, &bytes);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_string::assign({:#x}) -> (this={:#x})",
        this_ptr,
        ptr,
        this_ptr
    );
    Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
}

pub(super) fn basic_string_assign_substr(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let other_ptr = uc.read_arg(0);
    let offset = uc.read_arg(1);
    let count = uc.read_arg(2);
    let bytes = MSVCP60::basic_string_subrange(uc, other_ptr, offset, count);
    MSVCP60::set_basic_string_bytes(uc, this_ptr, &bytes);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_string::assign({:#x}, {}, {}) -> (this={:#x})",
        this_ptr,
        other_ptr,
        offset,
        count,
        this_ptr
    );
    Some(ApiHookResult::callee(3, Some(this_ptr as i32)))
}

pub(super) fn basic_string_append_substr(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let other_ptr = uc.read_arg(0);
    let offset = uc.read_arg(1);
    let count = uc.read_arg(2);

    let mut bytes = MSVCP60::basic_string_bytes(uc, this_ptr);
    bytes.extend(MSVCP60::basic_string_subrange(uc, other_ptr, offset, count));
    MSVCP60::set_basic_string_bytes(uc, this_ptr, &bytes);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_string::append({:#x}, {}, {}) -> (this={:#x})",
        this_ptr,
        other_ptr,
        offset,
        count,
        this_ptr
    );
    Some(ApiHookResult::callee(3, Some(this_ptr as i32)))
}

pub(super) fn basic_string_compare_other(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let other_ptr = uc.read_arg(0);
    let lhs = MSVCP60::basic_string_bytes(uc, this_ptr);
    let rhs = MSVCP60::basic_string_bytes(uc, other_ptr);
    let cmp = lhs.cmp(&rhs);
    let result = match cmp {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    };
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_string::compare({:#x}) -> {}",
        this_ptr,
        other_ptr,
        result
    );
    Some(ApiHookResult::callee(1, Some(result)))
}

pub(super) fn basic_string_compare_ptr(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let offset = uc.read_arg(0);
    let count = uc.read_arg(1);
    let ptr = uc.read_arg(2);
    let len = uc.read_arg(3) as usize;

    let lhs = MSVCP60::basic_string_subrange(uc, this_ptr, offset, count);
    let rhs = MSVCP60::source_bytes_from_ptr(uc, ptr, Some(len));
    let cmp = lhs.cmp(&rhs);
    let result = match cmp {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    };
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_string::compare({}, {}, {:#x}, {}) -> {}",
        this_ptr,
        offset,
        count,
        ptr,
        len,
        result
    );
    Some(ApiHookResult::callee(4, Some(result)))
}

pub(super) fn basic_string_erase(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let offset = uc.read_arg(0) as usize;
    let count = uc.read_arg(1) as usize;
    MSVCP60::basic_string_replace_range(uc, this_ptr, offset, count, &[]);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_string::erase({}, {}) -> (this={:#x})",
        this_ptr,
        offset,
        count,
        this_ptr
    );
    Some(ApiHookResult::callee(2, Some(this_ptr as i32)))
}

pub(super) fn basic_string_replace_repeat(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let pos = uc.read_arg(0) as usize;
    let remove_len = uc.read_arg(1) as usize;
    let repeat = uc.read_arg(2) as usize;
    let ch = uc.read_arg(3) as u8;
    let replacement = vec![ch; repeat];
    MSVCP60::basic_string_replace_range(uc, this_ptr, pos, remove_len, &replacement);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_string::replace({}, {}, {}, '{}') -> (this={:#x})",
        this_ptr,
        pos,
        remove_len,
        repeat,
        ch as char,
        this_ptr
    );
    Some(ApiHookResult::callee(4, Some(this_ptr as i32)))
}

pub(super) fn basic_string_replace_range_ptrs(
    uc: &mut Unicorn<Win32Context>,
) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let begin_ptr = uc.read_arg(0);
    let end_ptr = uc.read_arg(1);
    let src_begin = uc.read_arg(2);
    let src_end = uc.read_arg(3);

    let base_ptr = MSVCP60::basic_string_ptr(uc, this_ptr);
    let len = MSVCP60::basic_string_len(uc, this_ptr);
    let start = begin_ptr.saturating_sub(base_ptr).min(len) as usize;
    let end = end_ptr.saturating_sub(base_ptr).min(len) as usize;
    let replacement_len = src_end.saturating_sub(src_begin) as usize;
    let replacement = MSVCP60::read_exact_bytes(uc, src_begin, replacement_len);
    MSVCP60::basic_string_replace_range(
        uc,
        this_ptr,
        start,
        end.saturating_sub(start),
        &replacement,
    );
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_string::replace({:#x}, {:#x}, {:#x}, {:#x}) -> (this={:#x})",
        this_ptr,
        begin_ptr,
        end_ptr,
        src_begin,
        src_end,
        this_ptr
    );
    Some(ApiHookResult::callee(4, Some(this_ptr as i32)))
}

pub(super) fn basic_string_resize(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let target = uc.read_arg(0) as usize;
    let mut current = MSVCP60::basic_string_bytes(uc, this_ptr);
    current.resize(target, 0);
    MSVCP60::set_basic_string_bytes(uc, this_ptr, &current);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_string::resize({}) -> VOID",
        this_ptr,
        target
    );
    Some(ApiHookResult::callee(1, None))
}

pub(super) fn basic_string_swap(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let other_ptr = uc.read_arg(0);
    if this_ptr != 0 && other_ptr != 0 {
        for offset in [
            super::BASIC_STRING_PTR_OFFSET,
            super::BASIC_STRING_LEN_OFFSET,
            super::BASIC_STRING_RES_OFFSET,
        ] {
            let lhs = uc.read_u32(this_ptr as u64 + offset);
            let rhs = uc.read_u32(other_ptr as u64 + offset);
            uc.write_u32(this_ptr as u64 + offset, rhs);
            uc.write_u32(other_ptr as u64 + offset, lhs);
        }
    }
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_string::swap({:#x})",
        this_ptr,
        other_ptr
    );
    Some(ApiHookResult::callee(1, None))
}

pub(super) fn basic_string_c_str(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let ptr = MSVCP60::basic_string_ptr(uc, this_ptr);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_string::c_str() -> {:#x}",
        this_ptr,
        ptr
    );
    Some(ApiHookResult::callee(0, Some(ptr as i32)))
}

pub(super) fn basic_string_end(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let ptr = MSVCP60::basic_string_ptr(uc, this_ptr);
    let end = ptr.saturating_add(MSVCP60::basic_string_len(uc, this_ptr));
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_string::end() -> {:#x}",
        this_ptr,
        end
    );
    Some(ApiHookResult::callee(0, Some(end as i32)))
}

pub(super) fn basic_string_size(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let size = MSVCP60::basic_string_len(uc, this_ptr);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_string::size() -> {}",
        this_ptr,
        size
    );
    Some(ApiHookResult::callee(0, Some(size as i32)))
}

pub(super) fn basic_string_max_size(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let max_size = 0x7fff_fffeu32;
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_string::max_size() -> {}",
        this_ptr,
        max_size
    );
    Some(ApiHookResult::callee(0, Some(max_size as i32)))
}

pub(super) fn basic_string_substr(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let ret_ptr = uc.read_arg(0);
    let has_hidden_ret = MSVCP60::is_mapped_ptr(uc, ret_ptr);
    let offset_index = if has_hidden_ret { 1 } else { 0 };
    let count_index = if has_hidden_ret { 2 } else { 1 };
    let offset = uc.read_arg(offset_index);
    let count = uc.read_arg(count_index);
    let bytes = MSVCP60::basic_string_subrange(uc, this_ptr, offset, count);

    let result_ptr = if has_hidden_ret {
        ret_ptr
    } else {
        MSVCP60::alloc_zeroed(uc, 16)
    };
    MSVCP60::init_basic_string_empty(uc, result_ptr);
    MSVCP60::set_basic_string_bytes(uc, result_ptr, &bytes);

    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_string::substr({}, {}) -> {:#x}",
        this_ptr,
        offset,
        count,
        result_ptr
    );
    Some(ApiHookResult::callee(
        if has_hidden_ret { 3 } else { 2 },
        Some(result_ptr as i32),
    ))
}

pub(super) fn nullstr(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let addr = MSVCP60::empty_c_string_addr(uc);
    crate::emu_log!("[MSVCP60] basic_string::_Nullstr() -> {:#x}", addr);
    Some(ApiHookResult::caller(Some(addr as i32)))
}
