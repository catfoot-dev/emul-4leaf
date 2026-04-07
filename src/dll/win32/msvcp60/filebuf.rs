use crate::{
    dll::win32::{ApiHookResult, Win32Context},
    helper::UnicornHelper,
};
use unicorn_engine::Unicorn;

use super::MSVCP60;

pub(super) fn basic_filebuf_ctor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let file_ptr = uc.read_arg(0);
    MSVCP60::init_filebuf_layout(uc, this_ptr, file_ptr, 0);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_filebuf::basic_filebuf({:#x}) -> (this={:#x})",
        this_ptr,
        file_ptr,
        this_ptr
    );
    Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
}

pub(super) fn basic_filebuf_init(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let file_ptr = uc.read_arg(0);
    let init_flag = uc.read_arg(1);
    MSVCP60::init_filebuf_layout(uc, this_ptr, file_ptr, init_flag);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_filebuf::_Init({:#x}, {})",
        this_ptr,
        file_ptr,
        init_flag
    );
    Some(ApiHookResult::callee(2, None))
}

pub(super) fn basic_filebuf_open(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let filename_ptr = uc.read_arg(0);
    let mode = uc.read_arg(1);
    let (result, filename) = if let Some((file_handle, filename)) =
        MSVCP60::open_host_file_from_guest(uc, filename_ptr, mode)
    {
        MSVCP60::close_streambuf_file_handle(uc, this_ptr);
        MSVCP60::init_filebuf_layout(uc, this_ptr, file_handle, mode);
        (this_ptr, filename)
    } else {
        (
            0,
            if filename_ptr != 0 {
                uc.read_euc_kr(filename_ptr as u64)
            } else {
                String::new()
            },
        )
    };
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_filebuf::open(\"{}\", {}) -> (this={:#x})",
        this_ptr,
        filename,
        mode,
        result
    );
    Some(ApiHookResult::callee(2, Some(result as i32)))
}

pub(super) fn basic_filebuf_initcvt(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    crate::emu_log!("[MSVCP60] (this={:#x}) basic_filebuf::_Initcvt()", this_ptr);
    Some(ApiHookResult::callee(0, None))
}

pub(super) fn basic_filebuf_destructor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let _ = super::streambuf::basic_streambuf_sync(uc);
    MSVCP60::close_streambuf_file_handle(uc, this_ptr);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_filebuf::~basic_filebuf()",
        this_ptr
    );
    Some(ApiHookResult::callee(0, None))
}

pub(super) fn fiopen(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let filename_ptr = uc.read_arg(0);
    let mode = uc.read_arg(1);
    let (handle, filename) = if let Some((file_handle, filename)) =
        MSVCP60::open_host_file_from_guest(uc, filename_ptr, mode)
    {
        (file_handle, filename)
    } else {
        (
            0,
            if filename_ptr != 0 {
                uc.read_euc_kr(filename_ptr as u64)
            } else {
                String::new()
            },
        )
    };
    crate::emu_log!(
        "[MSVCP60] __Fiopen(\"{}\", {}) -> {:#x}",
        filename,
        mode,
        handle
    );
    Some(ApiHookResult::callee(2, Some(handle as i32)))
}
