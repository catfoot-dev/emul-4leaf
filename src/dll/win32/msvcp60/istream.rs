use crate::{
    dll::win32::{ApiHookResult, Win32Context},
    helper::UnicornHelper,
};
use std::io::SeekFrom;
use unicorn_engine::Unicorn;

use super::{
    MSVCP60, BASIC_ISTREAM_VTABLE, BASIC_IOSTREAM_VTABLE, IOS_FLAGS_OFFSET, IOS_STATE_OFFSET,
    STREAMBUF_READ_POS_OFFSET,
};

pub(super) fn basic_istream_ctor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let buf = uc.read_arg(0);
    let flags = uc.read_arg(1);
    MSVCP60::init_basic_ios_layout(uc, this_ptr, BASIC_ISTREAM_VTABLE, buf);
    uc.write_u32(this_ptr as u64 + IOS_FLAGS_OFFSET, flags);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_istream::basic_istream({:#x}, {}) -> (this={:#x})",
        this_ptr,
        buf,
        flags,
        this_ptr
    );
    Some(ApiHookResult::callee(2, Some(this_ptr as i32)))
}

pub(super) fn basic_istream_destructor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_istream::~basic_istream()",
        this_ptr
    );
    Some(ApiHookResult::callee(0, None))
}

pub(super) fn basic_istream_seekg(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let pos = uc.read_arg(0);
    let _high = uc.read_arg(1);
    let streambuf_ptr = MSVCP60::read_basic_ios_streambuf_ptr(uc, this_ptr);
    if MSVCP60::seek_streambuf_file(uc, streambuf_ptr, SeekFrom::Start(pos as u64)).is_some() {
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_istream::seekg({:#x}) -> (this={:#x})",
            this_ptr,
            pos,
            this_ptr
        );
        return Some(ApiHookResult::callee(2, Some(this_ptr as i32)));
    }
    MSVCP60::write_streambuf_field(uc, streambuf_ptr, STREAMBUF_READ_POS_OFFSET, pos);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_istream::seekg({:#x}) -> (this={:#x})",
        this_ptr,
        pos,
        this_ptr
    );
    Some(ApiHookResult::callee(2, Some(this_ptr as i32)))
}

pub(super) fn basic_istream_extract_unsigned_short(
    uc: &mut Unicorn<Win32Context>,
) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let out_ptr = uc.read_arg(0);
    let streambuf_ptr = MSVCP60::read_basic_ios_streambuf_ptr(uc, this_ptr);
    let used_fallback = MSVCP60::attach_version_dat_fallback(uc, streambuf_ptr);

    while let Some(byte) = MSVCP60::streambuf_peek_byte(uc, streambuf_ptr) {
        if !(byte as char).is_ascii_whitespace() {
            break;
        }
        let _ = MSVCP60::streambuf_take_byte(uc, streambuf_ptr);
    }

    let mut token = Vec::new();
    while let Some(byte) = MSVCP60::streambuf_peek_byte(uc, streambuf_ptr) {
        if !(byte as char).is_ascii_digit() {
            break;
        }
        if let Some(next) = MSVCP60::streambuf_take_byte(uc, streambuf_ptr) {
            token.push(next);
        }
    }

    let mut state = 0;
    if token.is_empty() {
        state |= 0x4;
        if MSVCP60::streambuf_peek_byte(uc, streambuf_ptr).is_none() {
            state |= 0x2;
        }
    } else if let Ok(text) = std::str::from_utf8(&token) {
        if let Ok(value) = text.parse::<u16>() {
            if out_ptr != 0 {
                uc.write_u16(out_ptr as u64, value);
            }
            if MSVCP60::streambuf_peek_byte(uc, streambuf_ptr).is_none() {
                state |= 0x2;
            }
        } else {
            state |= 0x4;
        }
    } else {
        state |= 0x4;
    }
    uc.write_u32(this_ptr as u64 + IOS_STATE_OFFSET, state);

    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_istream::operator>>({:#x}) fallback={}",
        this_ptr,
        out_ptr,
        used_fallback
    );
    Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
}

pub(super) fn basic_istream_getline(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let buf_addr = uc.read_arg(0);
    let count = uc.read_arg(1) as usize;
    let delim = uc.read_arg(2) as u8;
    if buf_addr != 0 && count != 0 {
        uc.write_u8(buf_addr as u64, 0);
    }
    let state = 0x2 | 0x4;
    uc.write_u32(this_ptr as u64 + IOS_STATE_OFFSET, state);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_istream::getline({:#x}, {}, '{}')",
        this_ptr,
        buf_addr,
        count,
        delim as char
    );
    Some(ApiHookResult::callee(3, Some(this_ptr as i32)))
}

pub(super) fn basic_iostream_ctor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let streambuf_ptr = uc.read_arg(0);
    MSVCP60::init_basic_ios_layout(uc, this_ptr, BASIC_IOSTREAM_VTABLE, streambuf_ptr);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_iostream::basic_iostream({:#x}) -> (this={:#x})",
        this_ptr,
        streambuf_ptr,
        this_ptr
    );
    Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
}

pub(super) fn basic_iostream_destructor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_iostream::~basic_iostream()",
        this_ptr
    );
    Some(ApiHookResult::callee(0, None))
}

pub(super) fn basic_ifstream_vbase_dtor(
    uc: &mut Unicorn<Win32Context>,
) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_ifstream::`vbase dtor`()",
        this_ptr
    );
    Some(ApiHookResult::callee(0, None))
}
