use crate::{
    dll::win32::{ApiHookResult, Win32Context, WindowClass},
    helper::UnicornHelper,
};
use unicorn_engine::Unicorn;

use super::USER32;

// API: ATOM RegisterClassExA(const WNDCLASSEXA* lpwcx)
// 역할: 창 클래스를 등록
pub(super) fn register_class_ex_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    // WNDCLASSEX는 48 bytes
    let class_addr = uc.read_arg(0);
    let style = uc.read_u32(class_addr as u64 + 4);
    let wnd_proc = uc.read_u32(class_addr as u64 + 8);
    let cb_cls_extra = uc.read_u32(class_addr as u64 + 12) as i32;
    let cb_wnd_extra = uc.read_u32(class_addr as u64 + 16) as i32;
    let hinstance = uc.read_u32(class_addr as u64 + 20);
    let h_icon = uc.read_u32(class_addr as u64 + 24);
    let h_cursor = uc.read_u32(class_addr as u64 + 28);
    let hbr_background = uc.read_u32(class_addr as u64 + 32);
    let menu_name_ptr = uc.read_u32(class_addr as u64 + 36);
    let class_name_ptr = uc.read_u32(class_addr as u64 + 40);
    let h_icon_sm = uc.read_u32(class_addr as u64 + 44);

    let class_name = uc.read_euc_kr(class_name_ptr as u64);
    let menu_name = if menu_name_ptr != 0 && menu_name_ptr > 0x10000 {
        uc.read_euc_kr(menu_name_ptr as u64)
    } else {
        String::new()
    };
    let guest_class_name_ptr = USER32::clone_guest_c_string(uc, class_name_ptr);
    let guest_menu_name_ptr = USER32::clone_guest_c_string(uc, menu_name_ptr);

    let ctx = uc.get_data();
    let atom = ctx.alloc_handle();
    ctx.window_classes.lock().unwrap().insert(
        class_name.clone(),
        WindowClass {
            atom,
            class_name: class_name.clone(),
            class_name_ptr: guest_class_name_ptr,
            wnd_proc,
            style,
            hinstance,
            cb_cls_extra,
            cb_wnd_extra,
            h_icon,
            h_icon_sm,
            h_cursor,
            hbr_background,
            menu_name,
            menu_name_ptr: guest_menu_name_ptr,
        },
    );
    crate::emu_log!(
        "[USER32] RegisterClassExA(\"{}\") -> atom {:#x}",
        class_name,
        atom
    );
    Some(ApiHookResult::callee(1, Some(atom as i32)))
}

// API: ATOM RegisterClassA(const WNDCLASSA* lpWndClass)
// 역할: 창 클래스를 등록
pub(super) fn register_class_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let class_addr = uc.read_arg(0);
    let style = uc.read_u32(class_addr as u64 + 0);
    let wnd_proc = uc.read_u32(class_addr as u64 + 4);
    let cb_cls_extra = uc.read_u32(class_addr as u64 + 8) as i32;
    let cb_wnd_extra = uc.read_u32(class_addr as u64 + 12) as i32;
    let hinstance = uc.read_u32(class_addr as u64 + 16);
    let h_icon = uc.read_u32(class_addr as u64 + 20);
    let h_cursor = uc.read_u32(class_addr as u64 + 24);
    let hbr_background = uc.read_u32(class_addr as u64 + 28);
    let menu_name_ptr = uc.read_u32(class_addr as u64 + 32);
    let class_name_ptr = uc.read_u32(class_addr as u64 + 36);

    let class_name = uc.read_euc_kr(class_name_ptr as u64);
    let menu_name = if menu_name_ptr != 0 && menu_name_ptr > 0x10000 {
        uc.read_euc_kr(menu_name_ptr as u64)
    } else {
        String::new()
    };
    let guest_class_name_ptr = USER32::clone_guest_c_string(uc, class_name_ptr);
    let guest_menu_name_ptr = USER32::clone_guest_c_string(uc, menu_name_ptr);

    let ctx = uc.get_data();
    let atom = ctx.alloc_handle();
    ctx.window_classes.lock().unwrap().insert(
        class_name.clone(),
        WindowClass {
            atom,
            class_name: class_name.clone(),
            class_name_ptr: guest_class_name_ptr,
            wnd_proc,
            style,
            hinstance,
            cb_cls_extra,
            cb_wnd_extra,
            h_icon,
            h_icon_sm: 0,
            h_cursor,
            hbr_background,
            menu_name,
            menu_name_ptr: guest_menu_name_ptr,
        },
    );
    crate::emu_log!(
        "[USER32] RegisterClassA(\"{}\") -> atom {:#x}",
        class_name,
        atom
    );
    Some(ApiHookResult::callee(1, Some(atom as i32)))
}

// API: BOOL GetClassInfoExA(HINSTANCE hinst, LPCSTR lpszClass, PWNDCLASSEXA lpwcx)
// 역할: 윈도우 클래스 정보 가져오기
pub(super) fn get_class_info_ex_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let _hinst = uc.read_arg(0);
    let class_name_ptr = uc.read_arg(1);
    let wcx_addr = uc.read_arg(2);
    let (class_name, wc_opt) = USER32::resolve_window_class(uc, class_name_ptr);
    if let Some(wc) = wc_opt {
        uc.write_u32(wcx_addr as u64, 48);
        uc.write_u32(wcx_addr as u64 + 4, wc.style);
        uc.write_u32(wcx_addr as u64 + 8, wc.wnd_proc);
        uc.write_u32(wcx_addr as u64 + 12, wc.cb_cls_extra as u32);
        uc.write_u32(wcx_addr as u64 + 16, wc.cb_wnd_extra as u32);
        uc.write_u32(wcx_addr as u64 + 20, wc.hinstance);
        uc.write_u32(wcx_addr as u64 + 24, wc.h_icon);
        uc.write_u32(wcx_addr as u64 + 28, wc.h_cursor);
        uc.write_u32(wcx_addr as u64 + 32, wc.hbr_background);
        uc.write_u32(wcx_addr as u64 + 36, wc.menu_name_ptr);
        uc.write_u32(wcx_addr as u64 + 40, wc.class_name_ptr.max(class_name_ptr));
        uc.write_u32(wcx_addr as u64 + 44, wc.h_icon_sm);
        crate::emu_log!("[USER32] GetClassInfoExA(\"{}\") -> BOOL 1", class_name);
        Some(ApiHookResult::callee(3, Some(1)))
    } else {
        crate::emu_log!("[USER32] GetClassInfoExA(\"{}\") -> BOOL 0", class_name);
        Some(ApiHookResult::callee(3, Some(0)))
    }
}

// API: BOOL GetClassInfoA(HINSTANCE hinst, LPCSTR lpszClass, PWNDCLASSA lpwc)
// 역할: 윈도우 클래스 정보 가져오기
pub(super) fn get_class_info_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let _hinst = uc.read_arg(0);
    let class_name_ptr = uc.read_arg(1);
    let lpwc = uc.read_arg(2);

    let (class_name, wc_opt) = USER32::resolve_window_class(uc, class_name_ptr);

    if let Some(wc) = wc_opt {
        uc.write_u32(lpwc as u64, wc.style);
        uc.write_u32(lpwc as u64 + 4, wc.wnd_proc);
        uc.write_u32(lpwc as u64 + 8, wc.cb_cls_extra as u32);
        uc.write_u32(lpwc as u64 + 12, wc.cb_wnd_extra as u32);
        uc.write_u32(lpwc as u64 + 16, wc.hinstance);
        uc.write_u32(lpwc as u64 + 20, wc.h_icon);
        uc.write_u32(lpwc as u64 + 24, wc.h_cursor);
        uc.write_u32(lpwc as u64 + 28, wc.hbr_background);
        uc.write_u32(lpwc as u64 + 32, wc.menu_name_ptr);
        uc.write_u32(lpwc as u64 + 36, wc.class_name_ptr.max(class_name_ptr));

        crate::emu_log!("[USER32] GetClassInfoA(\"{}\") -> BOOL 1", class_name);
        Some(ApiHookResult::callee(3, Some(1)))
    } else {
        crate::emu_log!("[USER32] GetClassInfoA(\"{}\") -> BOOL 0", class_name);
        Some(ApiHookResult::callee(3, Some(0)))
    }
}
