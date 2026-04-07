use crate::{
    dll::win32::{ApiHookResult, Win32Context},
    helper::UnicornHelper,
};
use unicorn_engine::Unicorn;

use super::{
    MSVCP60, BASIC_OSTREAM_VTABLE, IOS_FLAGS_OFFSET, IOS_STREAMBUF_OFFSET,
};

pub(super) fn basic_ostream_copy_ctor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let other_ptr = uc.read_arg(0);
    MSVCP60::init_basic_ios_layout(
        uc,
        this_ptr,
        BASIC_OSTREAM_VTABLE,
        uc.read_u32(other_ptr as u64 + IOS_STREAMBUF_OFFSET),
    );
    MSVCP60::basic_ios_copy_assign(uc, this_ptr, other_ptr);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_ostream::basic_ostream({:#x}) -> (this={:#x})",
        this_ptr,
        other_ptr,
        this_ptr
    );
    Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
}

pub(super) fn basic_ostream_ctor3(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let buf = uc.read_arg(0);
    let flags = uc.read_arg(1);
    let tied = uc.read_arg(2);
    MSVCP60::init_basic_ios_layout(uc, this_ptr, BASIC_OSTREAM_VTABLE, buf);
    uc.write_u32(this_ptr as u64 + IOS_FLAGS_OFFSET, flags);
    let _ = tied;
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_ostream::basic_ostream({:#x}, {}, {}) -> (this={:#x})",
        this_ptr,
        buf,
        flags,
        tied,
        this_ptr
    );
    Some(ApiHookResult::callee(3, Some(this_ptr as i32)))
}

pub(super) fn basic_ostream_ctor2(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let buf = uc.read_arg(0);
    let flags = uc.read_arg(1);
    MSVCP60::init_basic_ios_layout(uc, this_ptr, BASIC_OSTREAM_VTABLE, buf);
    uc.write_u32(this_ptr as u64 + IOS_FLAGS_OFFSET, flags);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_ostream::basic_ostream({:#x}, {}) -> (this={:#x})",
        this_ptr,
        buf,
        flags,
        this_ptr
    );
    Some(ApiHookResult::callee(2, Some(this_ptr as i32)))
}

pub(super) fn basic_ostream_destructor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_ostream::~basic_ostream()",
        this_ptr
    );
    Some(ApiHookResult::callee(0, None))
}

pub(super) fn basic_ostream_write(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let ptr = uc.read_arg(0);
    let len = uc.read_arg(1) as usize;
    let bytes = MSVCP60::source_bytes_from_ptr(uc, ptr, Some(len));
    MSVCP60::basic_ostream_write_bytes(uc, this_ptr, &bytes);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_ostream::write({:#x}, {}) -> (this={:#x})",
        this_ptr,
        ptr,
        len,
        this_ptr
    );
    Some(ApiHookResult::callee(2, Some(this_ptr as i32)))
}

pub(super) fn ostream_insert_int(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let value = uc.read_arg(0);
    MSVCP60::basic_ostream_write_bytes(uc, this_ptr, value.to_string().as_bytes());
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_ostream::operator<<({}) -> (this={:#x})",
        this_ptr,
        value,
        this_ptr
    );
    Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
}

pub(super) fn ostream_insert_cstr(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let os_ptr = uc.read_arg(0);
    let str_ptr = uc.read_arg(1);
    let bytes = MSVCP60::source_bytes_from_ptr(uc, str_ptr, None);
    MSVCP60::basic_ostream_write_bytes(uc, os_ptr, &bytes);
    crate::emu_log!(
        "[MSVCP60] std::operator<<({:#x}, {:#x}) -> (this={:#x})",
        os_ptr,
        str_ptr,
        os_ptr
    );
    Some(ApiHookResult::callee(2, Some(os_ptr as i32)))
}

pub(super) fn ostream_insert_char(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let os_ptr = uc.read_arg(0);
    let ch = uc.read_arg(1) as u8;
    MSVCP60::basic_ostream_write_bytes(uc, os_ptr, &[ch]);
    crate::emu_log!(
        "[MSVCP60] std::operator<<({:#x}, '{}') -> (this={:#x})",
        os_ptr,
        ch as char,
        os_ptr
    );
    Some(ApiHookResult::callee(2, Some(os_ptr as i32)))
}

pub(super) fn ostream_flush(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let os_ptr = uc.read_arg(0);
    crate::emu_log!(
        "[MSVCP60] std::flush({:#x}) -> (this={:#x})",
        os_ptr,
        os_ptr
    );
    Some(ApiHookResult::callee(1, Some(os_ptr as i32)))
}

pub(super) fn ostream_endl(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let os_ptr = uc.read_arg(0);
    MSVCP60::basic_ostream_write_bytes(uc, os_ptr, b"\n");
    crate::emu_log!("[MSVCP60] std::endl({:#x}) -> (this={:#x})", os_ptr, os_ptr);
    Some(ApiHookResult::callee(1, Some(os_ptr as i32)))
}

pub(super) fn basic_ofstream_ctor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    MSVCP60::init_basic_ios_layout(uc, this_ptr, BASIC_OSTREAM_VTABLE, 0);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_ofstream::basic_ofstream() -> (this={:#x})",
        this_ptr,
        this_ptr
    );
    Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
}

pub(super) fn basic_ofstream_dtor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_ofstream::~basic_ofstream()",
        this_ptr
    );
    Some(ApiHookResult::callee(0, None))
}

pub(super) fn basic_fstream_dtor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_fstream::~basic_fstream()",
        this_ptr
    );
    Some(ApiHookResult::callee(0, None))
}
