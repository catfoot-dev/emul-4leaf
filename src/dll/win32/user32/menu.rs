use crate::{
    dll::win32::{ApiHookResult, Win32Context},
    helper::UnicornHelper,
};
use unicorn_engine::Unicorn;

// API: BOOL GetMenuItemInfoA(HMENU hMenu, UINT item, BOOL fByPos, LPMENUITEMINFOA lpmii)
// 역할: 메뉴 항목에 대한 정보를 가져옴
pub(super) fn get_menu_item_info_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hmenu = uc.read_arg(0);
    let item = uc.read_arg(1);
    let f_by_pos = uc.read_arg(2);
    let lpmii = uc.read_arg(3);

    if lpmii != 0 {
        let cb_size = uc.read_u32(lpmii as u64);
        // 게스트가 초기화하지 않은 스택 데이터를 읽지 않도록 공용 필드를 기본값으로 채웁니다.
        if cb_size >= 0x2c {
            uc.write_u32(lpmii as u64 + 4, 0); // fMask
            uc.write_u32(
                lpmii as u64 + 8,
                if f_by_pos != 0 && item == 1 { 0x800 } else { 0 },
            ); // fType
            uc.write_u32(lpmii as u64 + 12, 0); // fState
            uc.write_u32(lpmii as u64 + 16, item); // wID
            uc.write_u32(lpmii as u64 + 20, 0); // hSubMenu
            uc.write_u32(lpmii as u64 + 24, 0); // hbmpChecked
            uc.write_u32(lpmii as u64 + 28, 0); // hbmpUnchecked
            uc.write_u32(lpmii as u64 + 32, 0); // dwItemData
            uc.write_u32(lpmii as u64 + 36, 0); // dwTypeData
            uc.write_u32(lpmii as u64 + 40, 0); // cch
        }
    }
    crate::emu_log!(
        "[USER32] GetMenuItemInfoA({:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 1",
        hmenu,
        item,
        f_by_pos,
        lpmii
    );
    Some(ApiHookResult::callee(4, Some(1)))
}

// API: BOOL DeleteMenu(HMENU hMenu, UINT uPosition, UINT uFlags)
// 역할: 메뉴에서 항목을 삭제
// 구현 생략 사유: 메뉴를 렌더링하지 않으므로 항목을 삭제할 필요 없음.
pub(super) fn delete_menu(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hmenu = uc.read_arg(0);
    let u_position = uc.read_arg(1);
    let u_flags = uc.read_arg(2);
    crate::emu_log!(
        "[USER32] DeleteMenu({:#x}, {:#x}, {:#x}) -> BOOL 1",
        hmenu,
        u_position,
        u_flags
    );
    Some(ApiHookResult::callee(3, Some(1)))
}

// API: BOOL RemoveMenu(HMENU hMenu, UINT uPosition, UINT uFlags)
// 역할: 메뉴 항목을 제거 (파괴하지 않음)
// 구현 생략 사유: 위와 동일.
pub(super) fn remove_menu(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hmenu = uc.read_arg(0);
    let u_position = uc.read_arg(1);
    let u_flags = uc.read_arg(2);
    crate::emu_log!(
        "[USER32] RemoveMenu({:#x}, {:#x}, {:#x}) -> BOOL 1",
        hmenu,
        u_position,
        u_flags
    );
    Some(ApiHookResult::callee(3, Some(1)))
}

// API: HMENU GetSystemMenu(HWND hWnd, BOOL bRevert)
// 역할: 복사/수정용 시스템 메뉴 핸들을 가져옴
pub(super) fn get_system_menu(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let b_revert = uc.read_arg(1);
    let handle = uc.get_data().alloc_handle();
    crate::emu_log!(
        "[USER32] GetSystemMenu({:#x}, {:#x}) -> HMENU {:#x}",
        hwnd,
        b_revert,
        handle
    );
    Some(ApiHookResult::callee(2, Some(handle as i32)))
}

// API: HMENU GetMenu(HWND hWnd)
// 역할: 지정된 창의 메뉴 핸들을 가져옴
pub(super) fn get_menu(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let handle = uc.get_data().alloc_handle();
    crate::emu_log!("[USER32] GetMenu({:#x}) -> HMENU {:#x}", hwnd, handle);
    Some(ApiHookResult::callee(1, Some(handle as i32)))
}

// API: BOOL AppendMenuA(HMENU hMenu, UINT uFlags, UINT_PTR uIDNewItem, LPCSTR lpNewItem)
// 역할: 메뉴 끝에 새 항목을 추가
// 구현 생략 사유: 시스템 메뉴 확장을 요청하지만 렌더링하지 않으므로 No-op.
pub(super) fn append_menu_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hmenu = uc.read_arg(0);
    let u_flags = uc.read_arg(1);
    let u_id_new_item = uc.read_arg(2);
    let lp_new_item = uc.read_arg(3);
    crate::emu_log!(
        "[USER32] AppendMenuA({:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 1",
        hmenu,
        u_flags,
        u_id_new_item,
        lp_new_item
    );
    Some(ApiHookResult::callee(4, Some(1)))
}

// API: HMENU CreateMenu(void)
// 역할: 메뉴를 생성
pub(super) fn create_menu(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let ctx = uc.get_data();
    let hmenu = ctx.alloc_handle();
    crate::emu_log!("[USER32] CreateMenu() -> HMENU {:#x}", hmenu);
    Some(ApiHookResult::callee(0, Some(hmenu as i32)))
}

// API: BOOL DestroyMenu(HMENU hMenu)
// 역할: 메뉴를 파괴
// 구현 생략 사유: 메뉴 객체를 시뮬레이션하지 않으므로 리소스 해제도 불필요함.
pub(super) fn destroy_menu(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hmenu = uc.read_arg(0);
    crate::emu_log!("[USER32] DestroyMenu({:#x}) -> BOOL 1", hmenu);
    Some(ApiHookResult::callee(1, Some(1)))
}

// API: BOOL TranslateMDISysAccel(HWND hWndClient, LPMSG lpMsg)
// 역할: MDI 자식 창의 바로 가기 키 메시지를 처리
pub(super) fn translate_mdi_sys_accel(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd_client = uc.read_arg(0);
    let lp_msg = uc.read_arg(1);
    // MSG: hwnd(0), message(4), wParam(8), lParam(12), time(16), pt(20)
    let msg = uc.read_u32(lp_msg as u64 + 4);
    let ret = if msg == 0x0100 || msg == 0x0104 {
        // WM_KEYDOWN, WM_SYSKEYDOWN
        0 // Simplified: not handled
    } else {
        0
    };
    crate::emu_log!(
        "[USER32] TranslateMDISysAccel({:#x}, {:#x}) -> BOOL {}",
        hwnd_client,
        lp_msg,
        ret
    );
    Some(ApiHookResult::callee(2, Some(ret)))
}
