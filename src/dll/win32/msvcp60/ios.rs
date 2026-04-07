use crate::{
    dll::win32::{ApiHookResult, Win32Context},
    helper::UnicornHelper,
};
use unicorn_engine::Unicorn;

use super::{
    MSVCP60, BASIC_IOS_VTABLE, FACET_REFCOUNT_OFFSET, IOS_BASE_VTABLE, IOS_LOCALE_OFFSET,
    IOS_STATE_OFFSET, LOCALE_OBJECT_SIZE,
};

pub(super) fn basic_ios_destructor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    crate::emu_log!("[MSVCP60] (this={:#x}) basic_ios::~basic_ios()", this_ptr);
    Some(ApiHookResult::callee(0, None))
}

pub(super) fn basic_ios_ctor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    MSVCP60::init_basic_ios_layout(uc, this_ptr, BASIC_IOS_VTABLE, 0);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_ios::basic_ios() -> (this={:#x})",
        this_ptr,
        this_ptr
    );
    Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
}

pub(super) fn basic_ios_assign(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let other_ptr = uc.read_arg(0);
    MSVCP60::basic_ios_copy_assign(uc, this_ptr, other_ptr);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_ios::operator=({:#x}) -> (this={:#x})",
        this_ptr,
        other_ptr,
        this_ptr
    );
    Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
}

pub(super) fn basic_ios_clear(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let state = uc.read_arg(0);
    let _throw = uc.read_arg(1);
    uc.write_u32(this_ptr as u64 + IOS_STATE_OFFSET, state);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_ios::clear({})",
        this_ptr,
        state
    );
    Some(ApiHookResult::callee(2, None))
}

pub(super) fn basic_ios_setstate(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let state = uc.read_arg(0);
    let _throw = uc.read_arg(1);
    let next = uc.read_u32(this_ptr as u64 + IOS_STATE_OFFSET) | state;
    uc.write_u32(this_ptr as u64 + IOS_STATE_OFFSET, next);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_ios::setstate({})",
        this_ptr,
        state
    );
    Some(ApiHookResult::callee(2, None))
}

pub(super) fn basic_ios_init(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let buf = uc.read_arg(0);
    let _flags = uc.read_arg(1);
    MSVCP60::init_basic_ios_layout(uc, this_ptr, BASIC_IOS_VTABLE, buf);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_ios::init({:#x})",
        this_ptr,
        buf
    );
    Some(ApiHookResult::callee(2, None))
}

pub(super) fn basic_ios_widen(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let ch = uc.read_arg(0) as u8;
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) basic_ios::widen('{}') -> '{}'",
        this_ptr,
        ch as char,
        ch as char
    );
    Some(ApiHookResult::callee(1, Some(ch as i32)))
}

pub(super) fn ios_base_ctor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    MSVCP60::init_ios_base_layout(uc, this_ptr, IOS_BASE_VTABLE);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) ios_base::ios_base() -> (this={:#x})",
        this_ptr,
        this_ptr
    );
    Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
}

pub(super) fn ios_base_dtor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    crate::emu_log!("[MSVCP60] (this={:#x}) ios_base::~ios_base()", this_ptr);
    Some(ApiHookResult::callee(0, None))
}

pub(super) fn ios_base_assign(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let other_ptr = uc.read_arg(0);
    MSVCP60::ios_base_copy_assign(uc, this_ptr, other_ptr);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) ios_base::operator=({:#x}) -> (this={:#x})",
        this_ptr,
        other_ptr,
        this_ptr
    );
    Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
}

pub(super) fn ios_base_clear(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let state = uc.read_arg(0);
    let _throw = uc.read_arg(1);
    uc.write_u32(this_ptr as u64 + IOS_STATE_OFFSET, state);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) ios_base::clear({})",
        this_ptr,
        state
    );
    Some(ApiHookResult::callee(2, None))
}

pub(super) fn ios_base_copyfmt(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let other_ptr = uc.read_arg(0);
    MSVCP60::ios_base_copy_assign(uc, this_ptr, other_ptr);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) ios_base::copyfmt({:#x}) -> (this={:#x})",
        this_ptr,
        other_ptr,
        this_ptr
    );
    Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
}

pub(super) fn ios_base_getloc(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let ret_ptr = uc.read_arg(0);
    let result_ptr = if MSVCP60::is_mapped_ptr(uc, ret_ptr) {
        ret_ptr
    } else {
        MSVCP60::alloc_zeroed(uc, LOCALE_OBJECT_SIZE)
    };

    let locimp = uc.read_u32(this_ptr as u64 + IOS_LOCALE_OFFSET);
    let locimp = if locimp != 0 {
        MSVCP60::read_locale_impl(uc, locimp)
    } else {
        MSVCP60::locale_impl_addr(uc)
    };
    MSVCP60::write_locale_value(uc, result_ptr, locimp);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) ios_base::getloc() -> {:#x}",
        this_ptr,
        result_ptr
    );
    Some(ApiHookResult::callee(
        if MSVCP60::is_mapped_ptr(uc, ret_ptr) {
            1
        } else {
            0
        },
        Some(result_ptr as i32),
    ))
}

pub(super) fn ios_base_init_ctor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    crate::emu_log!("[MSVCP60] (this={:#x}) ios_base::Init::Init()", this_ptr);
    Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
}

pub(super) fn ios_base_init_dtor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    crate::emu_log!("[MSVCP60] (this={:#x}) ios_base::Init::~Init()", this_ptr);
    Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
}

pub(super) fn ios_base_init(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    MSVCP60::init_ios_base_layout(uc, this_ptr, IOS_BASE_VTABLE);
    crate::emu_log!("[MSVCP60] (this={:#x}) ios_base::_Init()", this_ptr);
    Some(ApiHookResult::callee(0, None))
}

pub(super) fn locale_init(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let locimp_addr = MSVCP60::locale_impl_addr(uc);
    crate::emu_log!("[MSVCP60] locale::_Init() -> {:#x}", locimp_addr);
    Some(ApiHookResult::caller(Some(locimp_addr as i32)))
}

pub(super) fn locale_ctor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let locimp_addr = MSVCP60::locale_impl_addr(uc);
    MSVCP60::write_locale_value(uc, this_ptr, locimp_addr);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) locale::locale() -> (this={:#x})",
        this_ptr,
        this_ptr
    );
    Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
}

pub(super) fn locale_destructor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    crate::emu_log!("[MSVCP60] (this={:#x}) locale::~locale()", this_ptr);
    Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
}

pub(super) fn locale_assign(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let other_ptr = uc.read_arg(0);
    let locimp = MSVCP60::read_locale_impl(uc, other_ptr);
    MSVCP60::write_locale_value(uc, this_ptr, locimp);
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) locale::operator=({:#x}) -> (this={:#x})",
        this_ptr,
        other_ptr,
        this_ptr
    );
    Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
}

pub(super) fn locale_facet_incref(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    if this_ptr != 0 {
        let current = uc.read_u32(this_ptr as u64 + FACET_REFCOUNT_OFFSET);
        let next = current.max(1).saturating_add(1);
        uc.write_u32(this_ptr as u64 + FACET_REFCOUNT_OFFSET, next);
    }
    crate::emu_log!("[MSVCP60] (this={:#x}) locale::facet::_Incref()", this_ptr);
    Some(ApiHookResult::callee(0, None))
}

pub(super) fn locale_facet_decref(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    let result = if this_ptr != 0 {
        let current = uc.read_u32(this_ptr as u64 + FACET_REFCOUNT_OFFSET).max(1);
        let next = current.saturating_sub(1);
        uc.write_u32(this_ptr as u64 + FACET_REFCOUNT_OFFSET, next);
        if next == 0 { 0 } else { this_ptr }
    } else {
        0
    };
    crate::emu_log!(
        "[MSVCP60] (this={:#x}) locale::facet::_Decref() -> {:#x}",
        this_ptr,
        result
    );
    Some(ApiHookResult::callee(0, Some(result as i32)))
}

pub(super) fn winit_ctor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    crate::emu_log!("[MSVCP60] (this={:#x}) _Winit::_Winit()", this_ptr);
    Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
}

pub(super) fn winit_dtor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    crate::emu_log!("[MSVCP60] (this={:#x}) _Winit::~_Winit()", this_ptr);
    Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
}

pub(super) fn lockit_ctor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    crate::emu_log!("[MSVCP60] (this={:#x}) _Lockit::_Lockit()", this_ptr);
    Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
}

pub(super) fn lockit_dtor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = MSVCP60::this_ptr(uc);
    crate::emu_log!("[MSVCP60] (this={:#x}) _Lockit::~_Lockit()", this_ptr);
    Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
}

pub(super) fn xlen(_uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    crate::emu_log!("[MSVCP60] std::_Xlen()");
    Some(ApiHookResult::caller(None))
}

pub(super) fn xran(_uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    crate::emu_log!("[MSVCP60] std::_Xran()");
    Some(ApiHookResult::caller(None))
}

pub(super) fn xoff(_uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    crate::emu_log!("[MSVCP60] std::_Xoff()");
    Some(ApiHookResult::caller(None))
}
