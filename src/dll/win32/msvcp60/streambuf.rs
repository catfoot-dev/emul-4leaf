use crate::{
    dll::win32::{ApiHookResult, Win32Context},
    helper::UnicornHelper,
};
use std::io::SeekFrom;
use unicorn_engine::Unicorn;

use super::{
    BASIC_STREAMBUF_VTABLE, MSVCP60, STREAMBUF_BUFFER_OFFSET, STREAMBUF_CAPACITY_OFFSET,
    STREAMBUF_READ_POS_OFFSET, STREAMBUF_WRITE_POS_OFFSET,
};

pub(super) fn basic_streambuf_copy_ctor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let other_ptr = uc.read_arg(0);
    MSVCP60::init_streambuf_layout(uc, this_ptr, BASIC_STREAMBUF_VTABLE);
    MSVCP60::streambuf_copy_assign(uc, this_ptr, other_ptr);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_streambuf::basic_streambuf({:#x}) -> (this={:#x})",
        this_ptr,
        other_ptr,
        this_ptr
    );
    Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
}

pub(super) fn basic_streambuf_destructor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_streambuf::~basic_streambuf()",
        this_ptr
    );
    Some(ApiHookResult::callee(0, None))
}

pub(super) fn basic_streambuf_assign(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let other_ptr = uc.read_arg(0);
    MSVCP60::streambuf_copy_assign(uc, this_ptr, other_ptr);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_streambuf::operator=({:#x}) -> (this={:#x})",
        this_ptr,
        other_ptr,
        this_ptr
    );
    Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
}

pub(super) fn basic_streambuf_setbuf(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let buf_ptr = uc.read_arg(0);
    let len = uc.read_arg(1);
    MSVCP60::write_streambuf_field(uc, this_ptr, STREAMBUF_BUFFER_OFFSET, buf_ptr);
    MSVCP60::write_streambuf_field(uc, this_ptr, STREAMBUF_CAPACITY_OFFSET, len);
    MSVCP60::write_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET, 0);
    MSVCP60::write_streambuf_field(uc, this_ptr, STREAMBUF_WRITE_POS_OFFSET, 0);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_streambuf::setbuf({:#x}, {}) -> (this={:#x})",
        this_ptr,
        buf_ptr,
        len,
        this_ptr
    );
    Some(ApiHookResult::callee(2, Some(this_ptr as i32)))
}

pub(super) fn basic_streambuf_init(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    MSVCP60::init_streambuf_layout(uc, this_ptr, BASIC_STREAMBUF_VTABLE);
    crate::emu_log!("[MSVCP60] (this={:#x}) basic_streambuf::_Init()", this_ptr);
    Some(ApiHookResult::callee(0, None))
}

pub(super) fn basic_streambuf_init_ranges(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let arg0 = uc.read_arg(0);
    let arg1 = uc.read_arg(1);
    let arg2 = uc.read_arg(2);
    let arg3 = uc.read_arg(3);
    let arg4 = uc.read_arg(4);
    let arg5 = uc.read_arg(5);

    MSVCP60::init_streambuf_layout(uc, this_ptr, BASIC_STREAMBUF_VTABLE);
    for ptr in [arg0, arg1, arg3, arg4] {
        if MSVCP60::is_mapped_ptr(uc, ptr) {
            uc.write_u32(ptr as u64, 0);
        }
    }
    for ptr in [arg2, arg5] {
        if MSVCP60::is_mapped_ptr(uc, ptr) {
            uc.write_u32(ptr as u64, 0);
        }
    }

    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_streambuf::_Init({:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x})",
        this_ptr,
        arg0,
        arg1,
        arg2,
        arg3,
        arg4,
        arg5
    );
    Some(ApiHookResult::callee(6, None))
}

pub(super) fn basic_streambuf_setg(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let begin = uc.read_arg(0);
    let current = uc.read_arg(1);
    let end = uc.read_arg(2);
    MSVCP60::write_streambuf_field(uc, this_ptr, STREAMBUF_BUFFER_OFFSET, begin);
    MSVCP60::write_streambuf_field(
        uc,
        this_ptr,
        STREAMBUF_CAPACITY_OFFSET,
        end.saturating_sub(begin),
    );
    MSVCP60::write_streambuf_field(
        uc,
        this_ptr,
        STREAMBUF_READ_POS_OFFSET,
        current.saturating_sub(begin),
    );
    MSVCP60::write_streambuf_field(
        uc,
        this_ptr,
        STREAMBUF_WRITE_POS_OFFSET,
        end.saturating_sub(begin),
    );
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_streambuf::setg({:#x}, {:#x}, {:#x})",
        this_ptr,
        begin,
        current,
        end
    );
    Some(ApiHookResult::callee(3, None))
}

pub(super) fn basic_streambuf_setp(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let begin = uc.read_arg(0);
    let end = uc.read_arg(1);
    MSVCP60::write_streambuf_field(uc, this_ptr, STREAMBUF_BUFFER_OFFSET, begin);
    MSVCP60::write_streambuf_field(
        uc,
        this_ptr,
        STREAMBUF_CAPACITY_OFFSET,
        end.saturating_sub(begin),
    );
    MSVCP60::write_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET, 0);
    MSVCP60::write_streambuf_field(uc, this_ptr, STREAMBUF_WRITE_POS_OFFSET, 0);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_streambuf::setp({:#x}, {:#x})",
        this_ptr,
        begin,
        end
    );
    Some(ApiHookResult::callee(2, None))
}

pub(super) fn basic_streambuf_seekoff(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let ret_ptr = uc.read_arg(0);
    let has_hidden_ret = MSVCP60::is_mapped_ptr(uc, ret_ptr);
    let off_index = if has_hidden_ret { 1 } else { 0 };
    let dir_index = if has_hidden_ret { 2 } else { 1 };
    let off = uc.read_arg(off_index) as i32;
    let seekdir = uc.read_arg(dir_index);
    if let Some(next) = MSVCP60::seek_streambuf_file(
        uc,
        this_ptr,
        match seekdir {
            1 => SeekFrom::Current(off as i64),
            2 => SeekFrom::End(off as i64),
            _ => SeekFrom::Start(off.max(0) as u64),
        },
    ) {
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_streambuf::seekoff({}, {}) -> {}",
            this_ptr,
            off,
            seekdir,
            next
        );
        return Some(MSVCP60::streambuf_return_fpos(
            uc,
            next,
            3,
            if has_hidden_ret { 4 } else { 3 },
        ));
    }
    let available = MSVCP60::read_streambuf_field(uc, this_ptr, STREAMBUF_WRITE_POS_OFFSET) as i32;
    let current = MSVCP60::read_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET) as i32;

    let next = match seekdir {
        1 => current.saturating_add(off),
        2 => available.saturating_add(off),
        _ => off,
    }
    .clamp(0, available) as u32;
    MSVCP60::write_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET, next);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_streambuf::seekoff({}, {}) -> {}",
        this_ptr,
        off,
        seekdir,
        next
    );
    Some(MSVCP60::streambuf_return_fpos(
        uc,
        next,
        3,
        if has_hidden_ret { 4 } else { 3 },
    ))
}

pub(super) fn basic_streambuf_seekpos(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let ret_ptr = uc.read_arg(0);
    let has_hidden_ret = MSVCP60::is_mapped_ptr(uc, ret_ptr);
    let pos_index = if has_hidden_ret { 1 } else { 0 };
    let pos = uc.read_arg(pos_index);
    let mode_index = if has_hidden_ret { 3 } else { 2 };
    let mode = uc.read_arg(mode_index);
    if let Some(next) = MSVCP60::seek_streambuf_file(uc, this_ptr, SeekFrom::Start(pos as u64)) {
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_streambuf::seekpos({}, {}) -> {}",
            this_ptr,
            pos,
            mode,
            next
        );
        return Some(MSVCP60::streambuf_return_fpos(
            uc,
            next,
            3,
            if has_hidden_ret { 4 } else { 3 },
        ));
    }
    let write_pos = MSVCP60::read_streambuf_field(uc, this_ptr, STREAMBUF_WRITE_POS_OFFSET);
    let next = pos.min(write_pos);
    MSVCP60::write_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET, next);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_streambuf::seekpos({}, {}) -> {}",
        this_ptr,
        pos,
        mode,
        next
    );
    Some(MSVCP60::streambuf_return_fpos(
        uc,
        next,
        3,
        if has_hidden_ret { 4 } else { 3 },
    ))
}

pub(super) fn basic_streambuf_xsputn(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let ptr = uc.read_arg(0);
    let len = uc.read_arg(1) as usize;
    let bytes = MSVCP60::source_bytes_from_ptr(uc, ptr, Some(len));
    let written = MSVCP60::write_bytes_to_streambuf(uc, this_ptr, &bytes);

    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_streambuf::xsputn({:#x}, {}) -> {}",
        this_ptr,
        ptr,
        len,
        written
    );
    Some(ApiHookResult::callee(2, Some(written as i32)))
}

pub(super) fn basic_streambuf_xsgetn(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let dst_ptr = uc.read_arg(0);
    let requested = uc.read_arg(1) as usize;
    let mut copied = 0usize;

    while copied < requested {
        MSVCP60::prepare_streambuf_read(uc, this_ptr);
        let buffer_ptr = MSVCP60::read_streambuf_field(uc, this_ptr, STREAMBUF_BUFFER_OFFSET);
        let read_pos =
            MSVCP60::read_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET) as usize;
        let available = MSVCP60::streambuf_available(uc, this_ptr) as usize;
        if buffer_ptr == 0 || available == 0 {
            break;
        }

        let chunk = (requested - copied).min(available);
        if dst_ptr != 0 {
            let bytes = MSVCP60::read_exact_bytes(uc, buffer_ptr + read_pos as u32, chunk);
            let _ = uc.mem_write(dst_ptr as u64 + copied as u64, &bytes);
        }
        MSVCP60::write_streambuf_field(
            uc,
            this_ptr,
            STREAMBUF_READ_POS_OFFSET,
            (read_pos + chunk) as u32,
        );
        copied += chunk;
    }

    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_streambuf::xsgetn({:#x}, {}) -> {}",
        this_ptr,
        dst_ptr,
        requested,
        copied
    );
    Some(ApiHookResult::callee(2, Some(copied as i32)))
}

pub(super) fn basic_streambuf_underflow(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    MSVCP60::prepare_streambuf_read(uc, this_ptr);
    let buffer_ptr = MSVCP60::read_streambuf_field(uc, this_ptr, STREAMBUF_BUFFER_OFFSET);
    let read_pos = MSVCP60::read_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET);
    let available = MSVCP60::streambuf_available(uc, this_ptr);
    let value = if buffer_ptr != 0 && available != 0 {
        uc.read_u8(buffer_ptr as u64 + read_pos as u64) as i32
    } else {
        -1
    };
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_streambuf::underflow() -> {}",
        this_ptr,
        value
    );
    Some(ApiHookResult::callee(0, Some(value)))
}

pub(super) fn basic_streambuf_uflow(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let value = basic_streambuf_underflow(uc)?.return_value.unwrap_or(-1);
    if value >= 0 {
        let read_pos = MSVCP60::read_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET);
        MSVCP60::write_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET, read_pos + 1);
    }
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_streambuf::uflow() -> {}",
        this_ptr,
        value
    );
    Some(ApiHookResult::callee(0, Some(value)))
}

pub(super) fn basic_streambuf_showmanyc(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    MSVCP60::prepare_streambuf_read(uc, this_ptr);
    let available = MSVCP60::streambuf_available(uc, this_ptr);
    let value = if available == 0 && MSVCP60::streambuf_file_eof(uc, this_ptr) {
        -1
    } else {
        available as i32
    };
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_streambuf::showmanyc() -> {}",
        this_ptr,
        value
    );
    Some(ApiHookResult::callee(0, Some(value)))
}

pub(super) fn basic_streambuf_pbackfail(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let ch = uc.read_arg(0) as i32;
    MSVCP60::prepare_streambuf_read(uc, this_ptr);
    let buffer_ptr = MSVCP60::read_streambuf_field(uc, this_ptr, STREAMBUF_BUFFER_OFFSET);
    let read_pos = MSVCP60::read_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET);
    let result = if buffer_ptr != 0 && read_pos != 0 {
        let new_pos = read_pos - 1;
        MSVCP60::write_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET, new_pos);
        if ch >= 0 {
            uc.write_u8(buffer_ptr as u64 + new_pos as u64, ch as u8);
        }
        uc.read_u8(buffer_ptr as u64 + new_pos as u64) as i32
    } else {
        -1
    };
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_streambuf::pbackfail({}) -> {}",
        this_ptr,
        ch,
        result
    );
    Some(ApiHookResult::callee(1, Some(result)))
}

pub(super) fn basic_streambuf_sync(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let file_handle = MSVCP60::streambuf_file_handle(uc, this_ptr);
    let result = if file_handle != 0 {
        let context = uc.get_data();
        let mut files = context.files.lock().unwrap();
        if let Some(state) = files.get_mut(&file_handle) {
            use std::io::Write;
            state
                .file
                .flush()
                .map(|_| {
                    state.error = false;
                    0
                })
                .unwrap_or_else(|_| {
                    state.error = true;
                    -1
                })
        } else {
            -1
        }
    } else {
        0
    };
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_streambuf::sync() -> {}",
        this_ptr,
        result
    );
    Some(ApiHookResult::callee(0, Some(result)))
}

pub(super) fn streambuf_imbue(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let locale_ptr = uc.read_arg(0);
    let fallback_locale = MSVCP60::locale_value_addr(uc);
    let locale_value = if locale_ptr != 0 {
        locale_ptr
    } else {
        fallback_locale
    };
    MSVCP60::write_streambuf_field(uc, this_ptr, super::STREAMBUF_LOCALE_OFFSET, locale_value);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_streambuf::imbue({:#x})",
        this_ptr,
        locale_ptr
    );
    Some(ApiHookResult::callee(1, None))
}

pub(super) fn streambuf_init_strstream(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let flags = uc.read_arg(0);
    let buffer = uc.read_arg(1);
    let end = uc.read_arg(2);
    let len = uc.read_arg(3);
    MSVCP60::init_streambuf_layout(uc, this_ptr, BASIC_STREAMBUF_VTABLE);
    MSVCP60::write_streambuf_field(uc, this_ptr, STREAMBUF_BUFFER_OFFSET, buffer);
    MSVCP60::write_streambuf_field(uc, this_ptr, STREAMBUF_CAPACITY_OFFSET, len);
    MSVCP60::write_streambuf_field(
        uc,
        this_ptr,
        STREAMBUF_WRITE_POS_OFFSET,
        end.saturating_sub(buffer),
    );
    let _ = flags;
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) strstreambuf::_Init({}, {:#x}, {:#x}, {})",
        this_ptr,
        flags,
        buffer,
        end,
        len
    );
    Some(ApiHookResult::callee(4, None))
}
