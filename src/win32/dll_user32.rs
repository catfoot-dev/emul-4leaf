use unicorn_engine::Unicorn;

use crate::helper::UnicornHelper;
use crate::win32::{
    ApiHookResult, Win32Context, WindowClass, WindowState, callee_result, caller_result,
};

/// `USER32.dll` 프록시 구현 모듈
///
/// 윈도우 창, 클래스 관리, 메시지 루프 가상화를 담당하여 그래픽 UI 요소가 에뮬레이터 환경에서 작동하는 것처럼 모방
pub struct DllUSER32;

impl DllUSER32 {
    // API: int MessageBoxA(HWND hWnd, LPCSTR lpText, LPCSTR lpCaption, UINT uType)
    // 역할: 메시지 박스를 화면에 표시
    pub fn message_box_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let text_addr = uc.read_arg(1);
        let caption_addr = uc.read_arg(2);
        let u_type = uc.read_arg(3);
        let text = uc.read_euc_kr(text_addr as u64);
        let caption = uc.read_euc_kr(caption_addr as u64);

        let result = uc.get_data().win_event.lock().unwrap().message_box(
            caption.clone(),
            text.clone(),
            u_type,
        );

        crate::emu_log!(
            "[USER32] MessageBoxA({:#x}, \"{}\", \"{}\", {:#x}) -> {:#x}",
            hwnd,
            caption,
            text,
            u_type,
            result
        );
        Some((4, Some(result)))
    }

    // API: ATOM RegisterClassExA(const WNDCLASSEXA* lpwcx)
    // 역할: 창 클래스를 등록
    pub fn register_class_ex_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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

        let class_name = uc.read_euc_kr(class_name_ptr as u64);
        let menu_name = if menu_name_ptr != 0 && menu_name_ptr > 0x10000 {
            uc.read_euc_kr(menu_name_ptr as u64)
        } else {
            String::new()
        };

        let ctx = uc.get_data();
        let atom = ctx.alloc_handle();
        ctx.window_classes.lock().unwrap().insert(
            class_name.clone(),
            WindowClass {
                class_name: class_name.clone(),
                wnd_proc,
                style,
                hinstance,
                cb_cls_extra,
                cb_wnd_extra,
                h_icon,
                h_cursor,
                hbr_background,
                menu_name,
            },
        );
        crate::emu_log!(
            "[USER32] RegisterClassExA(\"{}\") -> atom {:#x}",
            class_name,
            atom
        );
        Some((1, Some(atom as i32)))
    }

    // API: ATOM RegisterClassA(const WNDCLASSA* lpWndClass)
    // 역할: 창 클래스를 등록
    pub fn register_class_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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

        let ctx = uc.get_data();
        let atom = ctx.alloc_handle();
        ctx.window_classes.lock().unwrap().insert(
            class_name.clone(),
            WindowClass {
                class_name: class_name.clone(),
                wnd_proc,
                style,
                hinstance,
                cb_cls_extra,
                cb_wnd_extra,
                h_icon,
                h_cursor,
                hbr_background,
                menu_name,
            },
        );
        crate::emu_log!(
            "[USER32] RegisterClassA(\"{}\") -> atom {:#x}",
            class_name,
            atom
        );
        Some((1, Some(atom as i32)))
    }

    // API: HWND CreateWindowExA(DWORD dwExStyle, LPCSTR lpClassName, LPCSTR lpWindowName, DWORD dwStyle, int X, int Y, int nWidth, int nHeight, HWND hWndParent, HMENU hMenu, HINSTANCE hInstance, LPVOID lpParam)
    // 역할: 확장 스타일을 포함한 창을 생성
    pub fn create_window_ex_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let ex_style = uc.read_arg(0);
        let class_addr = uc.read_arg(1);
        let title_addr = uc.read_arg(2);
        let style = uc.read_arg(3);
        let x = uc.read_arg(4);
        let y = uc.read_arg(5);
        let width = uc.read_arg(6);
        let height = uc.read_arg(7);
        let parent = uc.read_arg(8);
        let menu_or_id = uc.read_arg(9);
        let _instance = uc.read_arg(10);
        let param = uc.read_arg(11);

        let hwnd = uc.get_data().alloc_handle();

        if param != 0 {
            // MFC 등에서 사용되는 패턴: *((HWND *)lpParam + 1) = hwnd;
            uc.write_u32(param as u64 + 4, hwnd);
        }

        let class_name = if class_addr < 0x10000 {
            format!("Atom_{}", class_addr)
        } else {
            uc.read_euc_kr(class_addr as u64)
        };
        let title = if title_addr != 0 {
            uc.read_euc_kr(title_addr as u64)
        } else {
            String::new()
        };

        let surface_bitmap = uc.get_data().create_surface_bitmap(width, height);

        let window_state = WindowState {
            class_name: class_name.clone(),
            title: title.clone(),
            x: x as i32,
            y: y as i32,
            width: width as i32,
            height: height as i32,
            style,
            ex_style,
            parent,
            id: if menu_or_id < 0x10000 { menu_or_id } else { 0 },
            visible: style & 0x10000000 != 0,
            enabled: true,
            zoomed: false,
            iconic: false,
            wnd_proc: 0,
            user_data: 0,
            surface_bitmap,
        };

        let ctx = uc.get_data();
        ctx.win_event
            .lock()
            .unwrap()
            .create_window(hwnd, window_state);

        // 최상위 창이라면 활성화 및 포커스 설정
        if parent == 0 {
            use std::sync::atomic::Ordering;
            ctx.active_hwnd.store(hwnd, Ordering::SeqCst);
            ctx.foreground_hwnd.store(hwnd, Ordering::SeqCst);
            ctx.focus_hwnd.store(hwnd, Ordering::SeqCst);

            // UI 스레드에도 활성화 알림
            ctx.win_event.lock().unwrap().activate_window(hwnd);
        }

        crate::emu_log!(
            "[USER32] CreateWindowExA(class='{}', title='{}', hwnd={:#x}) -> {:#x}",
            class_name,
            title,
            hwnd,
            hwnd
        );
        Some((12, Some(hwnd as i32)))
    }

    // API: BOOL ShowWindow(HWND hWnd, int nCmdShow)
    // 역할: 창의 표시 상태를 설정
    pub fn show_window(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let n_cmd_show = uc.read_arg(1);
        // SW_HIDE = 0, 그 외는 대부분 표시
        let visible = n_cmd_show != 0;
        uc.get_data()
            .win_event
            .lock()
            .unwrap()
            .show_window(hwnd, visible);
        crate::emu_log!(
            "[USER32] ShowWindow({:#x}, {:#x}) -> BOOL 1",
            hwnd,
            n_cmd_show
        );
        Some((2, Some(1)))
    }

    // API: BOOL UpdateWindow(HWND hWnd)
    // 역할: 창의 클라이언트 영역을 강제로 업데이트
    pub fn update_window(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        uc.get_data().win_event.lock().unwrap().update_window(hwnd);
        crate::emu_log!("[USER32] UpdateWindow({:#x}) -> BOOL 1", hwnd);
        Some((1, Some(1)))
    }

    // API: BOOL DestroyWindow(HWND hWnd)
    // 역할: 지정된 창을 파괴
    pub fn destroy_window(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        uc.get_data().win_event.lock().unwrap().destroy_window(hwnd);
        crate::emu_log!("[USER32] DestroyWindow({:#x}) -> BOOL 1", hwnd);
        Some((1, Some(1)))
    }

    // API: BOOL CloseWindow(HWND hWnd)
    // 역할: 지정된 창을 최소화
    pub fn close_window(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        uc.get_data().win_event.lock().unwrap().close_window(hwnd);
        crate::emu_log!("[USER32] CloseWindow({:#x}) -> BOOL 1", hwnd);
        Some((1, Some(1)))
    }

    // API: BOOL EnableWindow(HWND hWnd, BOOL bEnable)
    // 역할: 창의 마우스 및 키보드 입력을 활성화 또는 비활성화
    pub fn enable_window(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let b_enable = uc.read_arg(1);
        uc.get_data()
            .win_event
            .lock()
            .unwrap()
            .enable_window(hwnd, b_enable != 0);
        crate::emu_log!(
            "[USER32] EnableWindow({:#x}, {:#x}) -> BOOL 1",
            hwnd,
            b_enable
        );
        Some((2, Some(1)))
    }

    // API: BOOL IsWindowEnabled(HWND hWnd)
    // 역할: 창이 활성화되어 있는지 확인
    pub fn is_window_enabled(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let enabled = uc
            .get_data()
            .win_event
            .lock()
            .unwrap()
            .is_window_enabled(hwnd);
        let ret = if enabled { 1 } else { 0 };
        crate::emu_log!("[USER32] IsWindowEnabled({:#x}) -> BOOL {}", hwnd, ret);
        Some((1, Some(ret)))
    }

    // API: BOOL IsWindowVisible(HWND hWnd)
    // 역할: 창의 가시성 상태를 확인
    pub fn is_window_visible(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let visible = uc
            .get_data()
            .win_event
            .lock()
            .unwrap()
            .is_window_visible(hwnd);
        let ret = if visible { 1 } else { 0 };
        crate::emu_log!("[USER32] IsWindowVisible({:#x}) -> BOOL {}", hwnd, ret);
        Some((1, Some(ret)))
    }

    // API: DWORD MsgWaitForMultipleObjects(DWORD nCount, const HANDLE* pHandles, BOOL fWaitAll, DWORD dwMilliseconds, DWORD dwWakeMask)
    // 역할: 하나 이상의 개체 또는 메시지가 큐에 도착할 때까지 대기
    // 구현 생략 사유: 다중 스레드 동기화 객체 대기 함수. 에뮬레이터 특성상 스레드를 멈추면 전체 엔진이 멈추므로 즉각 리턴(Timeout) 처리함.
    pub fn msg_wait_for_multiple_objects(
        uc: &mut Unicorn<Win32Context>,
    ) -> Option<(usize, Option<i32>)> {
        let n_count = uc.read_arg(0);
        let p_handles = uc.read_arg(1);
        let f_wait_all = uc.read_arg(2);
        let dw_milliseconds = uc.read_arg(3);
        let dw_wake_mask = uc.read_arg(4);
        crate::emu_log!(
            "[USER32] MsgWaitForMultipleObjects({:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> DWORD 0",
            n_count,
            p_handles,
            f_wait_all,
            dw_milliseconds,
            dw_wake_mask
        );
        Some((5, Some(0)))
    }

    // API: HWND GetWindow(HWND hWnd, UINT uCmd)
    // 역할: 지정된 창과 관계가 있는 창의 핸들을 가져옴
    pub fn get_window(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let cmd = uc.read_arg(1);
        // Minimal stub: GW_OWNER = 4
        let parent = if cmd == 4 {
            let ctx = uc.get_data();
            let win_event = ctx.win_event.lock().unwrap();
            win_event.windows.get(&hwnd).map(|w| w.parent).unwrap_or(0)
        } else {
            0
        };
        crate::emu_log!(
            "[USER32] GetWindow({:#x}, {:#x}) -> HWND {:#x}",
            hwnd,
            cmd,
            parent
        );
        Some((2, Some(parent as i32)))
    }

    // API: HWND GetParent(HWND hWnd)
    // 역할: 지정된 창의 부모 또는 소유자 창의 핸들을 가져옴
    pub fn get_parent(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let ctx = uc.get_data();
        let win_event = ctx.win_event.lock().unwrap();
        let parent = win_event.windows.get(&hwnd).map(|w| w.parent).unwrap_or(0);
        crate::emu_log!("[USER32] GetParent({:#x}) -> HWND {:#x}", hwnd, parent);
        Some((1, Some(parent as i32)))
    }

    // API: HWND GetDesktopWindow(void)
    // 역할: 데스크톱 창의 핸들을 가져옴
    pub fn get_desktop_window(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[USER32] GetDesktopWindow() -> HWND {:#x}", 0x0001);
        Some((0, Some(0x0001)))
    }

    // API: HWND SetActiveWindow(HWND hWnd)
    // 역할: 지정된 창을 활성화함
    pub fn set_active_window(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let ctx = uc.get_data();
        let old = ctx
            .active_hwnd
            .swap(hwnd, std::sync::atomic::Ordering::SeqCst);
        // 활성화 시 포커스도 함께 이동하는 것이 일반적
        ctx.focus_hwnd.store(hwnd, std::sync::atomic::Ordering::SeqCst);

        // UI 스레드 활성화 알림
        ctx.win_event.lock().unwrap().activate_window(hwnd);

        crate::emu_log!("[USER32] SetActiveWindow({:#x}) -> {:#x}", hwnd, old);
        Some((1, Some(old as i32)))
    }

    // API: HWND GetActiveWindow(void)
    // 역할: 현재 스레드와 연결된 활성 창의 핸들을 가져옴
    pub fn get_active_window(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let ctx = uc.get_data();
        let hwnd = ctx.active_hwnd.load(std::sync::atomic::Ordering::SeqCst);
        crate::emu_log!("[USER32] GetActiveWindow() -> HWND {:#x}", hwnd);
        Some((0, Some(hwnd as i32)))
    }

    // API: HWND GetForegroundWindow(void)
    // 역할: 포그라운드(전면) 창의 핸들을 가져옴
    pub fn get_foreground_window(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let ctx = uc.get_data();
        let hwnd = ctx.foreground_hwnd.load(std::sync::atomic::Ordering::SeqCst);
        crate::emu_log!("[USER32] GetForegroundWindow() -> HWND {:#x}", hwnd);
        Some((0, Some(hwnd as i32)))
    }

    // API: BOOL SetForegroundWindow(HWND hWnd)
    // 역할: 지정된 창을 포그라운드로 설정하고 활성화함
    pub fn set_foreground_window(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let ctx = uc.get_data();
        ctx.foreground_hwnd.store(hwnd, std::sync::atomic::Ordering::SeqCst);
        ctx.active_hwnd.store(hwnd, std::sync::atomic::Ordering::SeqCst);
        ctx.focus_hwnd.store(hwnd, std::sync::atomic::Ordering::SeqCst);

        // UI 스레드 활성화 알림
        ctx.win_event.lock().unwrap().activate_window(hwnd);

        crate::emu_log!("[USER32] SetForegroundWindow({:#x}) -> 1", hwnd);
        Some((1, Some(1)))
    }

    // API: HWND GetLastActivePopup(HWND hWnd)
    // 역할: 지정된 창에서 마지막으로 활성화된 팝업 창을 확인
    // 구현 생략 사유: 다중 창 환경의 포커스 관리용. 팝업 창을 사용하지 않으므로 무시함.
    pub fn get_last_active_popup(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        crate::emu_log!("[USER32] GetLastActivePopup({:#x}) -> HWND {:#x}", hwnd, 0);
        Some((1, Some(0)))
    }

    // API: BOOL GetMenuItemInfoA(HMENU hMenu, UINT item, BOOL fByPos, LPMENUITEMINFOA lpmii)
    // 역할: 메뉴 항목에 대한 정보를 가져옴
    // 구현 생략 사유: 메뉴 아이템 속성 조회. 에뮬레이터에서는 렌더링 가능한 시스템 메뉴 바를 그리지 않으므로 무시함.
    pub fn get_menu_item_info_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hmenu = uc.read_arg(0);
        let item = uc.read_arg(1);
        let f_by_pos = uc.read_arg(2);
        let lpmii = uc.read_arg(3);
        crate::emu_log!(
            "[USER32] GetMenuItemInfoA({:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 0",
            hmenu,
            item,
            f_by_pos,
            lpmii
        );
        Some((4, Some(0)))
    }

    // API: BOOL DeleteMenu(HMENU hMenu, UINT uPosition, UINT uFlags)
    // 역할: 메뉴에서 항목을 삭제
    // 구현 생략 사유: 메뉴를 렌더링하지 않으므로 항목을 삭제할 필요 없음.
    pub fn delete_menu(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hmenu = uc.read_arg(0);
        let u_position = uc.read_arg(1);
        let u_flags = uc.read_arg(2);
        crate::emu_log!(
            "[USER32] DeleteMenu({:#x}, {:#x}, {:#x}) -> BOOL 1",
            hmenu,
            u_position,
            u_flags
        );
        Some((3, Some(1)))
    }

    // API: BOOL RemoveMenu(HMENU hMenu, UINT uPosition, UINT uFlags)
    // 역할: 메뉴 항목을 제거 (파괴하지 않음)
    // 구현 생략 사유: 위와 동일.
    pub fn remove_menu(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hmenu = uc.read_arg(0);
        let u_position = uc.read_arg(1);
        let u_flags = uc.read_arg(2);
        crate::emu_log!(
            "[USER32] RemoveMenu({:#x}, {:#x}, {:#x}) -> BOOL 1",
            hmenu,
            u_position,
            u_flags
        );
        Some((3, Some(1)))
    }

    // API: HMENU GetSystemMenu(HWND hWnd, BOOL bRevert)
    // 역할: 복사/수정용 시스템 메뉴 핸들을 가져옴
    pub fn get_system_menu(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let b_revert = uc.read_arg(1);
        let handle = uc.get_data().alloc_handle();
        crate::emu_log!(
            "[USER32] GetSystemMenu({:#x}, {:#x}) -> HMENU {:#x}",
            hwnd,
            b_revert,
            handle
        );
        Some((2, Some(handle as i32)))
    }

    // API: HMENU GetMenu(HWND hWnd)
    // 역할: 지정된 창의 메뉴 핸들을 가져옴
    pub fn get_menu(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let handle = uc.get_data().alloc_handle();
        crate::emu_log!("[USER32] GetMenu({:#x}) -> HMENU {:#x}", hwnd, handle);
        Some((1, Some(handle as i32)))
    }

    // API: BOOL AppendMenuA(HMENU hMenu, UINT uFlags, UINT_PTR uIDNewItem, LPCSTR lpNewItem)
    // 역할: 메뉴 끝에 새 항목을 추가
    // 구현 생략 사유: 시스템 메뉴 확장을 요청하지만 렌더링하지 않으므로 No-op.
    pub fn append_menu_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((4, Some(1)))
    }

    // API: HMENU CreateMenu(void)
    // 역할: 메뉴를 생성
    pub fn create_menu(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let ctx = uc.get_data();
        let hmenu = ctx.alloc_handle();
        crate::emu_log!("[USER32] CreateMenu() -> HMENU {:#x}", hmenu);
        Some((0, Some(hmenu as i32)))
    }

    // API: BOOL DestroyMenu(HMENU hMenu)
    // 역할: 메뉴를 파괴
    // 구현 생략 사유: 메뉴 객체를 시뮬레이션하지 않으므로 리소스 해제도 불필요함.
    pub fn destroy_menu(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hmenu = uc.read_arg(0);
        crate::emu_log!("[USER32] DestroyMenu({:#x}) -> BOOL 1", hmenu);
        Some((1, Some(1)))
    }

    // API: BOOL MoveWindow(HWND hWnd, int X, int Y, int nWidth, int nHeight, BOOL bRepaint)
    // 역할: 창의 위치와 크기를 변경
    pub fn move_window(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let x = uc.read_arg(1) as i32;
        let y = uc.read_arg(2) as i32;
        let width = uc.read_arg(3);
        let height = uc.read_arg(4);
        let repaint = uc.read_arg(5);
        uc.get_data()
            .win_event
            .lock()
            .unwrap()
            .move_window(hwnd, x, y, width, height);
        crate::emu_log!(
            "[USER32] MoveWindow({:#x}, {}, {}, {}, {}, {}) -> BOOL 1",
            hwnd,
            x,
            y,
            width,
            height,
            repaint
        );
        Some((6, Some(1)))
    }

    // API: BOOL SetWindowPos(HWND hWnd, HWND hWndInsertAfter, int X, int Y, int cx, int cy, UINT uFlags)
    // 역할: 창의 크기, 위치 및 Z 순서를 변경
    pub fn set_window_pos(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let insert_after = uc.read_arg(1);
        let x = uc.read_arg(2);
        let y = uc.read_arg(3);
        let cx = uc.read_arg(4);
        let cy = uc.read_arg(5);
        let flags = uc.read_arg(6);
        uc.get_data().win_event.lock().unwrap().set_window_pos(
            hwnd,
            insert_after,
            x,
            y,
            cx,
            cy,
            flags,
        );
        crate::emu_log!(
            "[USER32] SetWindowPos({:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 1",
            hwnd,
            insert_after,
            x,
            y,
            cx,
            cy,
            flags
        );
        Some((7, Some(1)))
    }

    // API: BOOL GetWindowRect(HWND hWnd, LPRECT lpRect)
    // 역할: 창의 화면 좌표상의 경계 사각형 좌표를 가져옴
    pub fn get_window_rect(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let rect_addr = uc.read_arg(1);
        let (x, y, w, h) = {
            let ctx = uc.get_data();
            let win_event = ctx.win_event.lock().unwrap();
            win_event
                .windows
                .get(&hwnd)
                .map(|win| (win.x, win.y, win.width, win.height))
                .unwrap_or((0, 0, 640, 480))
        };

        uc.write_u32(rect_addr as u64, x as u32);
        uc.write_u32(rect_addr as u64 + 4, y as u32);
        uc.write_u32(rect_addr as u64 + 8, (x + w) as u32);
        uc.write_u32(rect_addr as u64 + 12, (y + h) as u32);
        crate::emu_log!(
            "[USER32] GetWindowRect({:#x}, {:#x}) -> BOOL 1",
            hwnd,
            rect_addr
        );
        Some((2, Some(1)))
    }

    // API: BOOL GetClientRect(HWND hWnd, LPRECT lpRect)
    // 역할: 창의 클라이언트 영역 좌표를 가져옴
    pub fn get_client_rect(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let rect_addr = uc.read_arg(1);
        let (w, h) = {
            let ctx = uc.get_data();
            let win_event = ctx.win_event.lock().unwrap();
            win_event
                .windows
                .get(&hwnd)
                .map(|win| (win.width, win.height))
                .unwrap_or((640, 480))
        };

        uc.write_u32(rect_addr as u64, 0);
        uc.write_u32(rect_addr as u64 + 4, 0);
        uc.write_u32(rect_addr as u64 + 8, w as u32);
        uc.write_u32(rect_addr as u64 + 12, h as u32);
        crate::emu_log!(
            "[USER32] GetClientRect({:#x}, {:#x}) -> BOOL 1",
            hwnd,
            rect_addr
        );
        Some((2, Some(1)))
    }

    // API: BOOL AdjustWindowRectEx(LPRECT lpRect, DWORD dwStyle, BOOL bMenu, DWORD dwExStyle)
    // 역할: 클라이언트 영역의 크기를 기준으로 원하는 창의 크기를 계산
    // 구현 생략 사유: 클라이언트 영역을 기반으로 전체 창 크기를 계산하는 보조 함수. 에뮬레이터에서는 창 크기를 정밀하게 다루지 않으므로 성공(1)만 반환함.
    pub fn adjust_window_rect_ex(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[USER32] AdjustWindowRectEx stubbed");
        Some((4, Some(1)))
    }

    // API: HDC BeginPaint(HWND hWnd, LPPAINTSTRUCT lpPaint)
    // 역할: 그리기를 위해 창을 준비
    pub fn begin_paint(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let ps_addr = uc.read_arg(1);
        let ctx = uc.get_data();
        let hdc = ctx.alloc_handle();
        let (w, h) = {
            let win_event = ctx.win_event.lock().unwrap();
            win_event
                .windows
                .get(&hwnd)
                .map(|win| (win.width, win.height))
                .unwrap_or((640, 480))
        };
        ctx.gdi_objects.lock().unwrap().insert(
            hdc,
            crate::win32::GdiObject::Dc {
                associated_window: hwnd,
                width: w as i32,
                height: h as i32,
                selected_bitmap: 0,
                selected_font: 0,
                selected_brush: 0,
                selected_pen: 0,
                selected_region: 0,
                selected_palette: 0,
                bk_mode: 0,
                bk_color: 0,
                text_color: 0,
                rop2_mode: 0,
                current_x: 0,
                current_y: 0,
            },
        );
        // PAINTSTRUCT: HDC at offset 0
        uc.write_u32(ps_addr as u64, hdc);
        crate::emu_log!(
            "[USER32] BeginPaint({:#x}, {:#x}) -> HDC {:#x}",
            hwnd,
            ps_addr,
            hdc
        );
        Some((2, Some(hdc as i32)))
    }

    // API: BOOL EndPaint(HWND hWnd, const PAINTSTRUCT* lpPaint)
    // 역할: 그리기가 완료되었음을 알림
    pub fn end_paint(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let ps_addr = uc.read_arg(1);
        let hdc = uc.read_u32(ps_addr as u64);
        let ctx = uc.get_data();
        ctx.gdi_objects.lock().unwrap().remove(&hdc);
        crate::emu_log!("[USER32] EndPaint({:#x}, {:#x}) -> BOOL 1", hwnd, ps_addr);
        Some((2, Some(1)))
    }

    // API: int ScrollWindowEx(HWND hWnd, int dx, int dy, const RECT* prcScroll, const RECT* prcClip, HRGN hrgnUpdate, LPRECT prcUpdate, UINT flags)
    // 역할: 창의 클라이언트 영역 내용을 스크롤
    // 구현 생략 사유: 클라이언트 영역 픽셀을 물리적으로 스크롤하는 보조 함수. 게임은 자체 루프나 BitBlt을 사용하므로 생략함.
    pub fn scroll_window_ex(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        crate::emu_log!("[USER32] ScrollWindowEx({:#x}) stubbed", hwnd);
        Some((8, Some(0)))
    }

    // API: BOOL InvalidateRect(HWND hWnd, const RECT* lpRect, BOOL bErase)
    // 역할: 창의 클라이언트 영역 중 일부를 갱신 대상으로 설정
    pub fn invalidate_rect(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let rect_addr = uc.read_arg(1);
        let erase = uc.read_arg(2);
        uc.get_data().win_event.lock().unwrap().update_window(hwnd);
        crate::emu_log!(
            "[USER32] InvalidateRect({:#x}, {:#x}, {:#x}) -> BOOL 1",
            hwnd,
            rect_addr,
            erase
        );
        Some((3, Some(1)))
    }

    // API: int SetScrollInfo(HWND hWnd, int nBar, LPCSCROLLINFO lpsi, BOOL redraw)
    // 역할: 스크롤 바의 매개변수를 설정
    // 구현 생략 사유: 네이티브 스크롤바 컴포넌트는 사용하지 않음.
    pub fn set_scroll_info(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        crate::emu_log!("[USER32] SetScrollInfo({:#x}) stubbed", hwnd);
        Some((4, Some(0)))
    }

    // API: BOOL SetWindowTextA(HWND hWnd, LPCSTR lpString)
    // 역할: 창의 제목 표시줄 텍스트 또는 컨트롤의 텍스트를 변경
    pub fn set_window_text_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let text_addr = uc.read_arg(1);
        let text = uc.read_euc_kr(text_addr as u64);
        uc.get_data()
            .win_event
            .lock()
            .unwrap()
            .set_window_text(hwnd, text.clone());
        crate::emu_log!(
            "[USER32] SetWindowTextA({:#x}, \"{}\") -> BOOL 1",
            hwnd,
            text
        );
        Some((2, Some(1)))
    }

    // API: int GetWindowTextA(HWND hWnd, LPSTR lpString, int nMaxCount)
    // 역할: 창의 제목 표시줄 텍스트 또는 컨트롤의 텍스트를 가져옴
    pub fn get_window_text_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let buf_addr = uc.read_arg(1);
        let max_count = uc.read_arg(2);

        let title_info = {
            let ctx = uc.get_data();
            let win_event = ctx.win_event.lock().unwrap();
            win_event.windows.get(&hwnd).map(|win| {
                let len = win.title.len().min(max_count as usize - 1);
                (win.title[..len].to_string(), len)
            })
        };

        if let Some((text, len)) = title_info {
            uc.write_string(buf_addr as u64, &text);
            crate::emu_log!(
                "[USER32] GetWindowTextA({:#x}, {:#x}, {:#x}) -> int {}",
                hwnd,
                buf_addr,
                max_count,
                len
            );
            Some((3, Some(len as i32)))
        } else {
            crate::emu_log!(
                "[USER32] GetWindowTextA({:#x}, {:#x}, {:#x}) -> int 0",
                hwnd,
                buf_addr,
                max_count
            );
            Some((3, Some(0)))
        }
    }

    // API: BOOL KillTimer(HWND hWnd, UINT_PTR uIDEvent)
    // 역할: 타이머를 중지
    pub fn kill_timer(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let _hwnd = uc.read_arg(0);
        let id = uc.read_arg(1);
        let ctx = uc.get_data();
        ctx.timers.lock().unwrap().remove(&id);
        crate::emu_log!("[USER32] KillTimer({:#x}, {:#x}) -> BOOL 1", _hwnd, id);
        Some((2, Some(1)))
    }

    // API: UINT_PTR SetTimer(HWND hWnd, UINT_PTR nIDEvent, UINT uElapse, TIMERPROC lpTimerFunc)
    // 역할: 타이머를 생성
    pub fn set_timer(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let mut id = uc.read_arg(1);
        let elapse = uc.read_arg(2);
        let lp_timer_func = uc.read_arg(3);
        let ctx = uc.get_data();
        let mut timers = ctx.timers.lock().unwrap();
        if id == 0 {
            id = ctx.alloc_handle();
        }
        timers.insert(id, elapse);
        crate::emu_log!(
            "[USER32] SetTimer({:#x}, {:#x}, {:#x}, {:#x}) -> UINT_PTR {:#x}",
            hwnd,
            id,
            elapse,
            lp_timer_func,
            id
        );
        Some((4, Some(id as i32)))
    }

    // API: HDC GetDC(HWND hWnd)
    // 역할: 지정된 창의 클라이언트 영역에 대한 DC를 가져옴
    pub fn get_dc(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let ctx = uc.get_data();
        let hdc = ctx.alloc_handle();
        let (w, h, surface_bitmap) = {
            let win_event = ctx.win_event.lock().unwrap();
            win_event
                .windows
                .get(&hwnd)
                .map(|win| (win.width, win.height, win.surface_bitmap))
                .unwrap_or((640, 480, 0))
        };
        ctx.gdi_objects.lock().unwrap().insert(
            hdc,
            crate::win32::GdiObject::Dc {
                associated_window: hwnd,
                width: w as i32,
                height: h as i32,
                selected_bitmap: surface_bitmap,
                selected_font: 0,
                selected_brush: 0,
                selected_pen: 0,
                selected_region: 0,
                selected_palette: 0,
                bk_mode: 0,
                bk_color: 0,
                text_color: 0,
                rop2_mode: 0,
                current_x: 0,
                current_y: 0,
            },
        );
        crate::emu_log!("[USER32] GetDC({:#x}) -> HDC {:#x}", hwnd, hdc);
        Some((1, Some(hdc as i32)))
    }

    // API: HDC GetWindowDC(HWND hWnd)
    // 역할: 지정된 창 전체(비클라이언트 영역 포함)에 대한 DC를 가져옴
    pub fn get_window_dc(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let ctx = uc.get_data();
        let hdc = ctx.alloc_handle();
        let (w, h, surface_bitmap) = {
            let win_event = ctx.win_event.lock().unwrap();
            win_event
                .windows
                .get(&hwnd)
                .map(|win| (win.width, win.height, win.surface_bitmap))
                .unwrap_or((640, 480, 0))
        };
        ctx.gdi_objects.lock().unwrap().insert(
            hdc,
            crate::win32::GdiObject::Dc {
                associated_window: hwnd,
                width: w as i32,
                height: h as i32,
                selected_bitmap: surface_bitmap,
                selected_font: 0,
                selected_brush: 0,
                selected_pen: 0,
                selected_region: 0,
                selected_palette: 0,
                bk_mode: 0,
                bk_color: 0,
                text_color: 0,
                rop2_mode: 0,
                current_x: 0,
                current_y: 0,
            },
        );
        crate::emu_log!("[USER32] GetWindowDC({:#x}) -> HDC {:#x}", hwnd, hdc);
        Some((1, Some(hdc as i32)))
    }

    // API: int ReleaseDC(HWND hWnd, HDC hDC)
    // 역할: DC를 해제
    pub fn release_dc(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let hdc = uc.read_arg(1);
        let ctx = uc.get_data();
        ctx.gdi_objects.lock().unwrap().remove(&hdc);
        crate::emu_log!("[USER32] ReleaseDC({:#x}, {:#x}) -> INT 1", hwnd, hdc);
        Some((2, Some(1)))
    }

    // API: LRESULT SendMessageA(HWND hWnd, UINT Msg, WPARAM wParam, LPARAM lParam)
    // 역할: 지정된 창에 메시지를 전송하고 처리가 완료될 때까지 대기
    // 구현 생략 사유: 대상 창의 메시지 프로시저를 동기적으로 직접 호출하는 복잡한 라우팅이 필요함. 단순 에뮬레이션에서는 콜백 재귀 호출시 스택 오버플로우나 락(Lock) 데드락 위험이 커서 무시함.
    pub fn send_message_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let msg = uc.read_arg(1);
        let wparam = uc.read_arg(2);
        let lparam = uc.read_arg(3);
        crate::emu_log!(
            "[USER32] SendMessageA({:#x}, {:#x}, {:#x}, {:#x}) -> LRESULT 0",
            hwnd,
            msg,
            wparam,
            lparam
        );
        Some((4, Some(0)))
    }

    // API: BOOL PostMessageA(HWND hWnd, UINT Msg, WPARAM wParam, LPARAM lParam)
    // 역할: 지정된 창의 메시지 큐에 메시지를 배치
    pub fn post_message_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let msg = uc.read_arg(1);
        let wparam = uc.read_arg(2);
        let lparam = uc.read_arg(3);
        let time = uc.get_data().start_time.elapsed().as_millis() as u32;
        let ctx = uc.get_data();
        ctx.message_queue
            .lock()
            .unwrap()
            .push_back([hwnd, msg, wparam, lparam, time, 0, 0]);
        crate::emu_log!(
            "[USER32] PostMessageA({:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 1",
            hwnd,
            msg,
            wparam,
            lparam
        );
        Some((4, Some(1)))
    }

    // API: HCURSOR LoadCursorA(HINSTANCE hInstance, LPCSTR lpCursorName)
    // 역할: 커서 리소스를 로드
    pub fn load_cursor_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let instance = uc.read_arg(0);
        let lpcursorname = uc.read_arg(1);
        let handle = uc.get_data().alloc_handle();
        crate::emu_log!(
            "[USER32] LoadCursorA({:#x}, {:#x}) -> HCURSOR {:#x}",
            instance,
            lpcursorname,
            handle
        );
        Some((2, Some(handle as i32)))
    }

    // API: HCURSOR LoadCursorFromFileA(LPCSTR lpFileName)
    // 역할: 파일에서 커서를 로드
    pub fn load_cursor_from_file_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let lpfilename = uc.read_arg(0);
        let handle = uc.get_data().alloc_handle();
        crate::emu_log!(
            "[USER32] LoadCursorFromFileA({:#x}) -> HCURSOR {:#x}",
            lpfilename,
            handle
        );
        Some((1, Some(handle as i32)))
    }

    // API: HICON LoadIconA(HINSTANCE hInstance, LPCSTR lpIconName)
    // 역할: 아이콘 리소스를 로드
    pub fn load_icon_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let instance = uc.read_arg(0);
        let lpiconname = uc.read_arg(1);
        let handle = uc.get_data().alloc_handle();
        crate::emu_log!(
            "[USER32] LoadIconA({:#x}, {:#x}) -> HICON {:#x}",
            instance,
            lpiconname,
            handle
        );
        Some((2, Some(handle as i32)))
    }

    // API: HCURSOR SetCursor(HCURSOR hCursor)
    // 역할: 마우스 커서를 설정
    // 구현 생략 사유: 마우스 커서의 시각적 변경은 게임 내부 상태에 영향을 주지 않으므로 생략함.
    pub fn set_cursor(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hcursor = uc.read_arg(0);
        crate::emu_log!("[USER32] SetCursor({:#x}) -> HCURSOR 0", hcursor);
        Some((1, Some(0)))
    }

    // API: BOOL DestroyCursor(HCURSOR hCursor)
    // 역할: 커서를 파괴하고 사용된 메모리를 해제
    // 구현 생략 사유: 커서 리소스 해제는 운영체제 몫이며 에뮬레이터 내에서 누수를 추적할 만큼 중요한 리소스가 아님.
    pub fn destroy_cursor(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hcursor = uc.read_arg(0);
        crate::emu_log!("[USER32] DestroyCursor({:#x}) -> BOOL 1", hcursor);
        Some((1, Some(1)))
    }

    // API: int MapWindowPoints(HWND hWndFrom, HWND hWndTo, LPPOINT lpPoints, UINT cPoints)
    // 역할: 한 창의 상대 좌표를 다른 창의 상대 좌표로 변환
    pub fn map_window_points(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd_from = uc.read_arg(0);
        let hwnd_to = uc.read_arg(1);
        let lp_points = uc.read_arg(2);
        let c_points = uc.read_arg(3);
        crate::emu_log!(
            "[USER32] MapWindowPoints({:#x}, {:#x}, {:#x}, {:#x}) -> int 0",
            hwnd_from,
            hwnd_to,
            lp_points,
            c_points
        );
        Some((4, Some(0)))
    }

    // API: BOOL SystemParametersInfoA(UINT uiAction, UINT uiParam, PVOID pvParam, UINT fWinIni)
    // 역할: 시스템 전체의 매개변수를 가져오거나 설정
    pub fn system_parameters_info_a(
        uc: &mut Unicorn<Win32Context>,
    ) -> Option<(usize, Option<i32>)> {
        let ui_action = uc.read_arg(0);
        let ui_param = uc.read_arg(1);
        let pv_param = uc.read_arg(2);
        let f_win_ini = uc.read_arg(3);
        crate::emu_log!(
            "[USER32] SystemParametersInfoA({:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 1",
            ui_action,
            ui_param,
            pv_param,
            f_win_ini
        );
        Some((4, Some(1)))
    }

    // API: BOOL TranslateMDISysAccel(HWND hWndClient, LPMSG lpMsg)
    // 역할: MDI 자식 창의 바로 가기 키 메시지를 처리
    pub fn translate_mdi_sys_accel(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd_client = uc.read_arg(0);
        let lp_msg = uc.read_arg(1);
        crate::emu_log!(
            "[USER32] TranslateMDISysAccel({:#x}, {:#x}) -> BOOL 0",
            hwnd_client,
            lp_msg
        );
        Some((2, Some(0)))
    }

    // API: int DrawTextA(HDC hDC, LPCSTR lpchText, int nCount, LPRECT lpRect, UINT uFormat)
    // 역할: 서식화된 텍스트를 사각형 내에 그림
    pub fn draw_text_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hdc = uc.read_arg(0);
        let lpch_text = uc.read_arg(1);
        let n_count = uc.read_arg(2);
        let lp_rect = uc.read_arg(3);
        let u_format = uc.read_arg(4);
        crate::emu_log!(
            "[USER32] DrawTextA({:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> int 0",
            hdc,
            lpch_text,
            n_count,
            lp_rect,
            u_format
        );
        Some((5, Some(0)))
    }

    // API: BOOL GetCursorPos(LPPOINT lpPoint)
    // 역할: 마우스 커서의 현재 위치를 화면 좌표로 가져옴
    pub fn get_cursor_pos(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let pt_addr = uc.read_arg(0);
        uc.write_u32(pt_addr as u64, 320);
        uc.write_u32(pt_addr as u64 + 4, 240);
        crate::emu_log!("[USER32] GetCursorPos({:#x}) -> BOOL 1", pt_addr);
        Some((1, Some(1)))
    }

    // API: BOOL PtInRect(const RECT* lprc, POINT pt)
    // 역할: 점이 사각형 내부에 있는지 확인
    pub fn pt_in_rect(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let rect_addr = uc.read_arg(0);
        let pt_x = uc.read_arg(1) as i32;
        let pt_y = uc.read_arg(2) as i32;
        let left = uc.read_u32(rect_addr as u64) as i32;
        let top = uc.read_u32(rect_addr as u64 + 4) as i32;
        let right = uc.read_u32(rect_addr as u64 + 8) as i32;
        let bottom = uc.read_u32(rect_addr as u64 + 12) as i32;
        let inside = pt_x >= left && pt_x < right && pt_y >= top && pt_y < bottom;
        crate::emu_log!(
            "[USER32] PtInRect({:#x}, {{x:{}, y:{}}}) -> BOOL {}",
            rect_addr,
            pt_x,
            pt_y,
            if inside { 1 } else { 0 }
        );
        Some((3, Some(if inside { 1 } else { 0 })))
    }

    // API: BOOL SetRect(LPRECT lprc, int xLeft, int yTop, int xRight, int yBottom)
    // 역할: 사각형의 좌표를 설정
    pub fn set_rect(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let rect_addr = uc.read_arg(0);
        let left = uc.read_arg(1) as i32;
        let top = uc.read_arg(2) as i32;
        let right = uc.read_arg(3) as i32;
        let bottom = uc.read_arg(4) as i32;
        uc.write_u32(rect_addr as u64, left as u32);
        uc.write_u32(rect_addr as u64 + 4, top as u32);
        uc.write_u32(rect_addr as u64 + 8, right as u32);
        uc.write_u32(rect_addr as u64 + 12, bottom as u32);
        crate::emu_log!(
            "[USER32] SetRect({:#x}, {}, {}, {}, {}) -> BOOL 1",
            rect_addr,
            left,
            top,
            right,
            bottom
        );
        Some((5, Some(1)))
    }

    // API: BOOL EqualRect(const RECT* lprc1, const RECT* lprc2)
    // 역할: 두 사각형이 동일한지 확인
    pub fn equal_rect(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let r1 = uc.read_arg(0);
        let r2 = uc.read_arg(1);
        let mut eq = true;
        for i in 0..4 {
            if uc.read_u32(r1 as u64 + i * 4) != uc.read_u32(r2 as u64 + i * 4) {
                eq = false;
                break;
            }
        }
        crate::emu_log!(
            "[USER32] EqualRect({:#x}, {:#x}) -> BOOL {}",
            r1,
            r2,
            if eq { 1 } else { 0 }
        );
        Some((2, Some(if eq { 1 } else { 0 })))
    }

    // API: BOOL UnionRect(LPRECT lprcDst, const RECT* lprcSrc1, const RECT* lprcSrc2)
    // 역할: 두 사각형을 모두 포함하는 최소 사각형을 계산
    pub fn union_rect(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let dst = uc.read_arg(0);
        let src1 = uc.read_arg(1);
        let src2 = uc.read_arg(2);
        let l1 = uc.read_u32(src1 as u64) as i32;
        let t1 = uc.read_u32(src1 as u64 + 4) as i32;
        let r1 = uc.read_u32(src1 as u64 + 8) as i32;
        let b1 = uc.read_u32(src1 as u64 + 12) as i32;
        let l2 = uc.read_u32(src2 as u64) as i32;
        let t2 = uc.read_u32(src2 as u64 + 4) as i32;
        let r2 = uc.read_u32(src2 as u64 + 8) as i32;
        let b2 = uc.read_u32(src2 as u64 + 12) as i32;
        let l = l1.min(l2);
        let t = t1.min(t2);
        let r = r1.max(r2);
        let b = b1.max(b2);
        uc.write_u32(dst as u64, l as u32);
        uc.write_u32(dst as u64 + 4, t as u32);
        uc.write_u32(dst as u64 + 8, r as u32);
        uc.write_u32(dst as u64 + 12, b as u32);
        crate::emu_log!(
            "[USER32] UnionRect({:#x}, {:#x}, {:#x}) -> BOOL 1",
            dst,
            src1,
            src2
        );
        Some((3, Some(1)))
    }

    // API: BOOL IntersectRect(LPRECT lprcDst, const RECT* lprcSrc1, const RECT* lprcSrc2)
    // 역할: 두 사각형의 교집합 사각형을 계산
    pub fn intersect_rect(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let dst = uc.read_arg(0);
        let src1 = uc.read_arg(1);
        let src2 = uc.read_arg(2);
        let l1 = uc.read_u32(src1 as u64) as i32;
        let t1 = uc.read_u32(src1 as u64 + 4) as i32;
        let r1 = uc.read_u32(src1 as u64 + 8) as i32;
        let b1 = uc.read_u32(src1 as u64 + 12) as i32;
        let l2 = uc.read_u32(src2 as u64) as i32;
        let t2 = uc.read_u32(src2 as u64 + 4) as i32;
        let r2 = uc.read_u32(src2 as u64 + 8) as i32;
        let b2 = uc.read_u32(src2 as u64 + 12) as i32;
        let l = l1.max(l2);
        let t = t1.max(t2);
        let r = r1.min(r2);
        let b = b1.min(b2);
        if l < r && t < b {
            uc.write_u32(dst as u64, l as u32);
            uc.write_u32(dst as u64 + 4, t as u32);
            uc.write_u32(dst as u64 + 8, r as u32);
            uc.write_u32(dst as u64 + 12, b as u32);
            crate::emu_log!(
                "[USER32] IntersectRect({:#x}, {:#x}, {:#x}) -> BOOL 1",
                dst,
                src1,
                src2
            );
            Some((3, Some(1)))
        } else {
            uc.write_u32(dst as u64, 0);
            uc.write_u32(dst as u64 + 4, 0);
            uc.write_u32(dst as u64 + 8, 0);
            uc.write_u32(dst as u64 + 12, 0);
            crate::emu_log!(
                "[USER32] IntersectRect({:#x}, {:#x}, {:#x}) -> BOOL 0",
                dst,
                src1,
                src2
            );
            Some((3, Some(0)))
        }
    }

    // API: HANDLE GetClipboardData(UINT uFormat)
    // 역할: 클립보드에서 데이터를 가져옴
    pub fn get_clipboard_data(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let format = uc.read_arg(0);
        if format == 1 {
            let (ptr, data) = {
                let ctx = uc.get_data();
                let cb = ctx.clipboard_data.lock().unwrap();
                if cb.is_empty() {
                    (0, Vec::new())
                } else {
                    let ptr = ctx
                        .heap_cursor
                        .fetch_add(cb.len() as u32 + 1, std::sync::atomic::Ordering::SeqCst);
                    (ptr, cb.clone())
                }
            };
            if ptr != 0 {
                uc.mem_write(ptr as u64, &data).unwrap();
                uc.mem_write(ptr as u64 + data.len() as u64, &[0]).unwrap();
                crate::emu_log!("[USER32] GetClipboardData({:#x}) -> int {:#x}", format, ptr);
                return Some((1, Some(ptr as i32)));
            }
        }
        crate::emu_log!("[USER32] GetClipboardData({:#x}) -> int 0", format);
        Some((1, Some(0)))
    }

    // API: BOOL OpenClipboard(HWND hWndNewOwner)
    // 역할: 클립보드를 엶
    pub fn open_clipboard(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let ctx = uc.get_data();
        let opened = ctx
            .clipboard_open
            .swap(1, std::sync::atomic::Ordering::SeqCst);
        crate::emu_log!(
            "[USER32] OpenClipboard({:#x}) -> BOOL {}",
            hwnd,
            if opened == 0 { 1 } else { 0 }
        );
        Some((1, Some(if opened == 0 { 1 } else { 0 })))
    }

    // API: BOOL CloseClipboard(void)
    // 역할: 클립보드를 닫음
    pub fn close_clipboard(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let ctx = uc.get_data();
        ctx.clipboard_open
            .store(0, std::sync::atomic::Ordering::SeqCst);
        crate::emu_log!("[USER32] CloseClipboard() -> BOOL 1");
        Some((0, Some(1)))
    }

    // API: BOOL EmptyClipboard(void)
    // 역할: 클립보드 비우기
    pub fn empty_clipboard(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let ctx = uc.get_data();
        ctx.clipboard_data.lock().unwrap().clear();
        crate::emu_log!("[USER32] EmptyClipboard() -> BOOL 1");
        Some((0, Some(1)))
    }

    // API: HANDLE SetClipboardData(UINT uFormat, HANDLE hMem)
    // 역할: 클립보드 데이터 설정
    pub fn set_clipboard_data(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let format = uc.read_arg(0);
        let hmem = uc.read_arg(1);
        if format == 1 && hmem != 0 {
            let mut buf = Vec::new();
            let mut curr = hmem as u64;
            loop {
                let mut tmp = [0u8; 1];
                uc.mem_read(curr, &mut tmp).unwrap();
                if tmp[0] == 0 {
                    break;
                }
                buf.push(tmp[0]);
                curr += 1;
            }
            let ctx = uc.get_data();
            *ctx.clipboard_data.lock().unwrap() = buf;
            crate::emu_log!(
                "[USER32] SetClipboardData({:#x}) -> HANDLE {:#x}",
                format,
                hmem
            );
            return Some((2, Some(hmem as i32)));
        }
        Some((2, Some(0)))
    }

    // API: BOOL IsClipboardFormatAvailable(UINT format)
    // 역할: 클립보드 포맷 확인
    pub fn is_clipboard_format_available(
        uc: &mut Unicorn<Win32Context>,
    ) -> Option<(usize, Option<i32>)> {
        let format = uc.read_arg(0);
        let available = if format == 1 {
            let ctx = uc.get_data();
            if ctx.clipboard_data.lock().unwrap().is_empty() {
                0
            } else {
                1
            }
        } else {
            0
        };
        crate::emu_log!(
            "[USER32] IsClipboardFormatAvailable({:#x}) -> {}",
            format,
            available
        );
        Some((1, Some(available)))
    }

    // API: HWND SetCapture(HWND hWnd)
    // 역할: 마우스 캡처 설정
    pub fn set_capture(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let ctx = uc.get_data();
        let old = ctx
            .capture_hwnd
            .swap(hwnd, std::sync::atomic::Ordering::SeqCst);
        crate::emu_log!("[USER32] SetCapture({:#x}) -> HWND {:#x}", hwnd, old);
        Some((1, Some(old as i32)))
    }

    // API: HWND GetCapture(void)
    // 역할: 마우스 캡처 창 핸들
    pub fn get_capture(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let ctx = uc.get_data();
        let hwnd = ctx.capture_hwnd.load(std::sync::atomic::Ordering::SeqCst);
        crate::emu_log!("[USER32] GetCapture() -> HWND {:#x}", hwnd);
        Some((0, Some(hwnd as i32)))
    }

    // API: BOOL ReleaseCapture(void)
    // 역할: 마우스 캡처 해제
    pub fn release_capture(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let ctx = uc.get_data();
        ctx.capture_hwnd
            .store(0, std::sync::atomic::Ordering::SeqCst);
        crate::emu_log!("[USER32] ReleaseCapture() -> BOOL 1");
        Some((0, Some(1)))
    }

    // API: BOOL ScreenToClient(HWND hWnd, LPPOINT lpPoint)
    // 역할: 화면 좌표를 클라이언트 좌표로
    pub fn screen_to_client(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let pt_addr = uc.read_arg(1);
        let (win_x, win_y) = {
            let ctx = uc.get_data();
            let win_event = ctx.win_event.lock().unwrap();
            win_event
                .windows
                .get(&hwnd)
                .map(|w| (w.x, w.y))
                .unwrap_or((0, 0))
        };
        let x = uc.read_u32(pt_addr as u64) as i32;
        let y = uc.read_u32(pt_addr as u64 + 4) as i32;
        uc.write_u32(pt_addr as u64, (x - win_x) as u32);
        uc.write_u32(pt_addr as u64 + 4, (y - win_y) as u32);
        crate::emu_log!("[USER32] ScreenToClient({:#x}) -> BOOL 1", hwnd);
        Some((2, Some(1)))
    }

    // API: BOOL ClientToScreen(HWND hWnd, LPPOINT lpPoint)
    // 역할: 클라이언트 좌표를 화면 좌표로
    pub fn client_to_screen(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let pt_addr = uc.read_arg(1);
        let (win_x, win_y) = {
            let ctx = uc.get_data();
            let win_event = ctx.win_event.lock().unwrap();
            win_event
                .windows
                .get(&hwnd)
                .map(|w| (w.x, w.y))
                .unwrap_or((0, 0))
        };
        let x = uc.read_u32(pt_addr as u64) as i32;
        let y = uc.read_u32(pt_addr as u64 + 4) as i32;
        uc.write_u32(pt_addr as u64, (x + win_x) as u32);
        uc.write_u32(pt_addr as u64 + 4, (y + win_y) as u32);
        crate::emu_log!("[USER32] ClientToScreen({:#x}) -> BOOL 1", hwnd);
        Some((2, Some(1)))
    }

    // API: BOOL CreateCaret(HWND hWnd, HBITMAP hBitmap, int nWidth, int nHeight)
    // 역할: 캐럿 생성
    pub fn create_caret(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[USER32] CreateCaret stubbed");
        Some((4, Some(1)))
    }

    // API: BOOL DestroyCaret(void)
    pub fn destroy_caret(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[USER32] DestroyCaret stubbed");
        Some((0, Some(1)))
    }

    // API: BOOL ShowCaret(HWND hWnd)
    pub fn show_caret(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[USER32] ShowCaret stubbed");
        Some((1, Some(1)))
    }

    // API: BOOL HideCaret(HWND hWnd)
    pub fn hide_caret(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[USER32] HideCaret stubbed");
        Some((1, Some(1)))
    }

    // API: BOOL SetCaretPos(int X, int Y)
    pub fn set_caret_pos(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[USER32] SetCaretPos stubbed");
        Some((2, Some(1)))
    }
    // API: SHORT GetAsyncKeyState(int vKey)
    pub fn get_async_key_state(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let vkey = uc.read_arg(0) as usize;
        let ctx = uc.get_data();
        let ks = ctx.key_states.lock().unwrap();
        let mut state: i32 = 0;
        if vkey < 256 && ks[vkey] {
            state = -32768; // 0x8000
        }
        crate::emu_log!("[USER32] GetAsyncKeyState({:#x}) -> {:#x}", vkey, state);
        Some((1, Some(state)))
    }

    // API: SHORT GetKeyState(int nVirtKey)
    pub fn get_key_state(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let vkey = uc.read_arg(0) as usize;
        let ctx = uc.get_data();
        let ks = ctx.key_states.lock().unwrap();
        let mut state: i32 = 0;
        if vkey < 256 && ks[vkey] {
            state = -32768; // 0x8000
        }
        crate::emu_log!("[USER32] GetKeyState({:#x}) -> {:#x}", vkey, state);
        Some((1, Some(state)))
    }

    // API: DWORD GetSysColor(int nIndex)
    pub fn get_sys_color(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let index = uc.read_arg(0);
        let color = match index {
            5 => 0x00FFFFFF,  // COLOR_WINDOW
            8 => 0x00000000,  // COLOR_WINDOWTEXT
            15 => 0x00C0C0C0, // COLOR_BTNFACE
            _ => 0x00808080,
        };
        crate::emu_log!("[USER32] GetSysColor({:#x}) -> {:#x}", index, color);
        Some((1, Some(color as i32)))
    }

    // API: int SetWindowRgn(HWND hWnd, HRGN hRgn, BOOL bRedraw)
    pub fn set_window_rgn(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        crate::emu_log!("[USER32] SetWindowRgn({:#x}) stubbed", hwnd);
        Some((3, Some(1)))
    }

    // API: BOOL GetClassInfoExA(HINSTANCE hinst, LPCSTR lpszClass, PWNDCLASSEXA lpwcx)
    pub fn get_class_info_ex_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let _hinst = uc.read_arg(0);
        let class_name_ptr = uc.read_arg(1);
        let class_name = if class_name_ptr < 0x10000 {
            format!("Atom_{}", class_name_ptr)
        } else {
            uc.read_euc_kr(class_name_ptr as u64)
        };
        let wcx_addr = uc.read_arg(2);
        let wnd_proc = {
            let ctx = uc.get_data();
            let classes = ctx.window_classes.lock().unwrap();
            classes.get(&class_name).map(|wc| wc.wnd_proc)
        };
        if let Some(proc) = wnd_proc {
            uc.write_u32(wcx_addr as u64 + 8, proc);
            crate::emu_log!("[USER32] GetClassInfoExA(\"{}\") -> 1", class_name);
            Some((3, Some(1)))
        } else {
            crate::emu_log!("[USER32] GetClassInfoExA(\"{}\") -> 0", class_name);
            Some((3, Some(0)))
        }
    }

    // API: BOOL GetClassInfoA(HINSTANCE hinst, LPCSTR lpszClass, PWNDCLASSA lpwc)
    pub fn get_class_info_a(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[USER32] GetClassInfoA stubbed");
        Some((3, Some(0)))
    }

    // API: BOOL IsZoomed(HWND hWnd)
    pub fn is_zoomed(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let ctx = uc.get_data();
        let win_event = ctx.win_event.lock().unwrap();
        let zoomed = win_event
            .windows
            .get(&hwnd)
            .map(|w| w.zoomed)
            .unwrap_or(false);
        crate::emu_log!("[USER32] IsZoomed({:#x}) -> {}", hwnd, zoomed);
        Some((1, Some(if zoomed { 1 } else { 0 })))
    }

    // API: BOOL IsIconic(HWND hWnd)
    pub fn is_iconic(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let ctx = uc.get_data();
        let win_event = ctx.win_event.lock().unwrap();
        let iconic = win_event
            .windows
            .get(&hwnd)
            .map(|w| w.iconic)
            .unwrap_or(false);
        crate::emu_log!("[USER32] IsIconic({:#x}) -> {}", hwnd, iconic);
        Some((1, Some(if iconic { 1 } else { 0 })))
    }

    // API: int wsprintfA(LPSTR lpOut, LPCSTR lpFmt, ...)
    pub fn wsprintf_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let buf_addr = uc.read_arg(0);
        let fmt_addr = uc.read_arg(1);
        if buf_addr != 0 && fmt_addr != 0 {
            let fmt = uc.read_euc_kr(fmt_addr as u64);
            let mut arg_idx = 2;
            let mut formatted = String::new();
            let mut chars = fmt.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '%' {
                    if let Some(&next) = chars.peek() {
                        if next == '%' {
                            formatted.push('%');
                            chars.next();
                        } else {
                            let mut pad_char = ' ';
                            let mut padding = 0;
                            let mut format_char = *chars.peek().unwrap();
                            if format_char == '0' {
                                pad_char = '0';
                                chars.next();
                                format_char = *chars.peek().unwrap_or(&' ');
                            }
                            while format_char.is_ascii_digit() {
                                padding = padding * 10 + format_char.to_digit(10).unwrap();
                                chars.next();
                                format_char = *chars.peek().unwrap_or(&' ');
                            }
                            if format_char == 'l' || format_char == 'h' {
                                chars.next();
                                format_char = *chars.peek().unwrap_or(&' ');
                            }
                            chars.next();
                            let arg_val = uc.read_arg(arg_idx);
                            arg_idx += 1;
                            let mut s = match format_char {
                                'd' | 'i' => format!("{}", arg_val as i32),
                                'u' => format!("{}", arg_val as u32),
                                'x' => format!("{:x}", arg_val as u32),
                                'X' => format!("{:X}", arg_val as u32),
                                'c' => format!("{}", (arg_val as u8) as char),
                                's' | 'S' => {
                                    if arg_val != 0 {
                                        uc.read_euc_kr(arg_val as u64)
                                    } else {
                                        "(null)".to_string()
                                    }
                                }
                                _ => format!("%{}", format_char),
                            };
                            if padding > 0 && s.len() < padding as usize {
                                let pad = padding as usize - s.len();
                                s = pad_char.to_string().repeat(pad) + &s;
                            }
                            formatted.push_str(&s);
                        }
                    }
                } else {
                    formatted.push(c);
                }
            }
            uc.write_string(buf_addr as u64, &formatted);
            crate::emu_log!("[USER32] wsprintfA -> \"{}\"", formatted);
            Some((arg_idx, Some(formatted.len() as i32)))
        } else {
            Some((2, Some(0)))
        }
    }

    // API: BOOL EndDialog(HWND hDlg, INT_PTR nResult)
    pub fn end_dialog(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let h_dlg = uc.read_arg(0);
        crate::emu_log!("[USER32] EndDialog({:#x}) -> 1", h_dlg);
        Some((2, Some(1)))
    }

    // API: LRESULT DefWindowProcA(HWND hWnd, UINT Msg, WPARAM wParam, LPARAM lParam)
    pub fn def_window_proc_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let msg = uc.read_arg(1);
        let w_param = uc.read_arg(2);
        let l_param = uc.read_arg(3);
        crate::emu_log!(
            "[USER32] DefWindowProcA({:#x}, {:#x}, {:#x}, {:#x}) -> 0",
            hwnd,
            msg,
            w_param,
            l_param
        );
        Some((4, Some(0)))
    }

    // API: LRESULT DefMDIChildProcA(HWND hWnd, UINT Msg, WPARAM wParam, LPARAM lParam)
    pub fn def_mdi_child_proc_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let msg = uc.read_arg(1);
        let w_param = uc.read_arg(2);
        let l_param = uc.read_arg(3);
        crate::emu_log!(
            "[USER32] DefMDIChildProcA({:#x}, {:#x}, {:#x}, {:#x}) -> 0",
            hwnd,
            msg,
            w_param,
            l_param
        );
        Some((4, Some(0)))
    }

    // API: LRESULT DefFrameProcA(HWND hWnd, HWND hWndMDIClient, UINT Msg, WPARAM wParam, LPARAM lParam)
    pub fn def_frame_proc_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let msg = uc.read_arg(2);
        crate::emu_log!("[USER32] DefFrameProcA({:#x}, msg={:#x}) -> 0", hwnd, msg);
        Some((5, Some(0)))
    }

    // API: LONG SetWindowLongA(HWND hWnd, int nIndex, LONG dwNewLong)
    pub fn set_window_long_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let index = uc.read_arg(1) as i32;
        let new_val = uc.read_arg(2);
        crate::emu_log!(
            "[USER32] SetWindowLongA({:#x}, idx={}, val={:#x}) stubbed",
            hwnd,
            index,
            new_val
        );
        Some((3, Some(0)))
    }

    // API: LONG GetWindowLongA(HWND hWnd, int nIndex)
    pub fn get_window_long_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let index = uc.read_arg(1) as i32;
        crate::emu_log!("[USER32] GetWindowLongA({:#x}, idx={}) -> 0", hwnd, index);
        Some((2, Some(0)))
    }

    // API: LONG_PTR SetWindowLongPtrA(HWND hWnd, int nIndex, LONG_PTR dwNewLong)
    pub fn set_window_long_ptr_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        Self::set_window_long_a(uc) // reuse SetWindowLongA for now
    }

    // API: LONG_PTR GetWindowLongPtrA(HWND hWnd, int nIndex)
    pub fn get_window_long_ptr_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        Self::get_window_long_a(uc) // reuse GetWindowLongA for now
    }

    // API: LRESULT CallWindowProcA(WNDPROC lpPrevWndFunc, HWND hWnd, UINT Msg, WPARAM wParam, LPARAM lParam)
    pub fn call_window_proc_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let proc = uc.read_arg(0);
        let hwnd = uc.read_arg(1);
        let msg = uc.read_arg(2);
        crate::emu_log!(
            "[USER32] CallWindowProcA(proc={:#x}, hwnd={:#x}, msg={:#x}) -> 0",
            proc,
            hwnd,
            msg
        );
        Some((5, Some(0)))
    }

    // API: BOOL PostThreadMessageA(DWORD idThread, UINT Msg, WPARAM wParam, LPARAM lParam)
    pub fn post_thread_message_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let thread_id = uc.read_arg(0);
        let msg = uc.read_arg(1);
        let w_param = uc.read_arg(2);
        let l_param = uc.read_arg(3);
        let time = uc.get_data().start_time.elapsed().as_millis() as u32;
        let ctx = uc.get_data();
        ctx.message_queue
            .lock()
            .unwrap()
            .push_back([0, msg, w_param, l_param, time, 0, 0]);
        crate::emu_log!(
            "[USER32] PostThreadMessageA({:#x}, msg={:#x}, {:#x}, {:#x}) -> 1",
            thread_id,
            msg,
            w_param,
            l_param
        );
        Some((4, Some(1)))
    }

    // API: BOOL IsDialogMessageA(HWND hDlg, LPMSG lpMsg)
    pub fn is_dialog_message_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let _h_dlg = uc.read_arg(0);
        let _lp_msg = uc.read_arg(1);
        crate::emu_log!("[USER32] IsDialogMessageA stubbed");
        Some((2, Some(0)))
    }

    // API: void PostQuitMessage(int nExitCode)
    pub fn post_quit_message(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let n_exit_code = uc.read_arg(0);
        crate::emu_log!("[USER32] PostQuitMessage({})", n_exit_code);
        Some((1, None))
    }

    // API: HWND SetFocus(HWND hWnd)
    pub fn set_focus(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let ctx = uc.get_data();
        let old = ctx
            .focus_hwnd
            .swap(hwnd, std::sync::atomic::Ordering::SeqCst);

        // 포커스 설정 시 활성 창도 업데이트 (간단화된 구현)
        ctx.active_hwnd
            .store(hwnd, std::sync::atomic::Ordering::SeqCst);

        // UI 스레드 활성화 알림
        ctx.win_event.lock().unwrap().activate_window(hwnd);

        crate::emu_log!("[USER32] SetFocus({:#x}) -> {:#x}", hwnd, old);
        Some((1, Some(old as i32)))
    }

    // API: HWND GetFocus(void)
    pub fn get_focus(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let ctx = uc.get_data();
        let focus = ctx.focus_hwnd.load(std::sync::atomic::Ordering::SeqCst);
        crate::emu_log!("[USER32] GetFocus() -> {:#x}", focus);
        Some((0, Some(focus as i32)))
    }

    // API: LRESULT DispatchMessageA(const MSG* lpMsg)
    pub fn dispatch_message_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let lp_msg = uc.read_arg(0);
        crate::emu_log!("[USER32] DispatchMessageA({:#x}) stubbed", lp_msg);
        Some((1, Some(0)))
    }

    // API: BOOL TranslateMessage(const MSG* lpMsg)
    pub fn translate_message(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let lp_msg = uc.read_arg(0);
        crate::emu_log!("[USER32] TranslateMessage({:#x}) stubbed", lp_msg);
        Some((1, Some(0)))
    }

    // API: BOOL PeekMessageA(LPMSG lpMsg, HWND hWnd, UINT wMsgFilterMin, UINT wMsgFilterMax, UINT wRemoveMsg)
    pub fn peek_message_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let lp_msg = uc.read_arg(0);
        let _hwnd = uc.read_arg(1);
        let _min = uc.read_arg(2);
        let _max = uc.read_arg(3);
        let _remove = uc.read_arg(4);

        let msg = {
            let ctx = uc.get_data();
            let mut q = ctx.message_queue.lock().unwrap();
            q.pop_front()
        };

        if let Some(m) = msg {
            for i in 0..7 {
                uc.write_u32(lp_msg as u64 + i * 4, m[i as usize]);
            }
            crate::emu_log!("[USER32] PeekMessageA -> 1 (msg={:#x})", m[1]);
            Some((5, Some(1)))
        } else {
            crate::emu_log!("[USER32] PeekMessageA -> 0");
            Some((5, Some(0)))
        }
    }

    // API: BOOL GetMessageA(LPMSG lpMsg, HWND hWnd, UINT wMsgFilterMin, UINT wMsgFilterMax)
    pub fn get_message_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let lp_msg = uc.read_arg(0);
        let _hwnd = uc.read_arg(1);
        let _min = uc.read_arg(2);
        let _max = uc.read_arg(3);

        let msg = {
            let ctx = uc.get_data();
            let mut q = ctx.message_queue.lock().unwrap();
            q.pop_front()
        };

        if let Some(m) = msg {
            for i in 0..7 {
                uc.write_u32(lp_msg as u64 + i * 4, m[i as usize]);
            }
            crate::emu_log!("[USER32] GetMessageA -> 1 (msg={:#x})", m[1]);
            Some((4, Some(1)))
        } else {
            // No message
            crate::emu_log!("[USER32] GetMessageA (no message) -> 1 (WM_NULL)");
            // Return WM_NULL
            uc.write_u32(lp_msg as u64 + 4, 0); // message = 0
            Some((4, Some(1)))
        }
    }

    // API: BOOL GetPropA(HWND hWnd, LPCSTR lpString)
    pub fn get_prop_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        crate::emu_log!("[USER32] GetPropA({:#x}) -> 0", hwnd);
        Some((2, Some(0)))
    }

    // API: BOOL SetPropA(HWND hWnd, LPCSTR lpString, HANDLE hData)
    pub fn set_prop_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        crate::emu_log!("[USER32] SetPropA({:#x}) -> 1", hwnd);
        Some((3, Some(1)))
    }

    // API: HANDLE RemovePropA(HWND hWnd, LPCSTR lpString)
    pub fn remove_prop_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        crate::emu_log!("[USER32] RemovePropA({:#x}) -> 0", hwnd);
        Some((2, Some(0)))
    }

    // API: BOOL IsWindow(HWND hWnd)
    pub fn is_window(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hwnd = uc.read_arg(0);
        let ctx = uc.get_data();
        let win_event = ctx.win_event.lock().unwrap();
        let exists = win_event.windows.contains_key(&hwnd);
        crate::emu_log!("[USER32] IsWindow({:#x}) -> {}", hwnd, exists);
        Some((1, Some(if exists { 1 } else { 0 })))
    }

    fn wrap_result(func_name: &str, result: Option<(usize, Option<i32>)>) -> Option<ApiHookResult> {
        match func_name {
            "wsprintfA" => caller_result(result),
            _ => callee_result(result),
        }
    }

    /// 함수명 기준 `USER32.dll` API 구현체
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        DllUSER32::wrap_result(
            func_name,
            match func_name {
                "MessageBoxA" => Self::message_box_a(uc),
                "RegisterClassExA" => Self::register_class_ex_a(uc),
                "RegisterClassA" => Self::register_class_a(uc),
                "CreateWindowExA" => Self::create_window_ex_a(uc),
                "ShowWindow" => Self::show_window(uc),
                "UpdateWindow" => Self::update_window(uc),
                "DestroyWindow" => Self::destroy_window(uc),
                "CloseWindow" => Self::close_window(uc),
                "EnableWindow" => Self::enable_window(uc),
                "IsWindowEnabled" => Self::is_window_enabled(uc),
                "IsWindowVisible" => Self::is_window_visible(uc),
                "MoveWindow" => Self::move_window(uc),
                "SetWindowPos" => Self::set_window_pos(uc),
                "GetWindowRect" => Self::get_window_rect(uc),
                "GetClientRect" => Self::get_client_rect(uc),
                "AdjustWindowRectEx" => Self::adjust_window_rect_ex(uc),
                "GetDC" => Self::get_dc(uc),
                "GetWindowDC" => Self::get_window_dc(uc),
                "ReleaseDC" => Self::release_dc(uc),
                "SendMessageA" => Self::send_message_a(uc),
                "PostMessageA" => Self::post_message_a(uc),
                "LoadCursorA" => Self::load_cursor_a(uc),
                "LoadCursorFromFileA" => Self::load_cursor_from_file_a(uc),
                "LoadIconA" => Self::load_icon_a(uc),
                "SetCursor" => Self::set_cursor(uc),
                "DestroyCursor" => Self::destroy_cursor(uc),
                "DefWindowProcA" => Self::def_window_proc_a(uc),
                "DefMDIChildProcA" => Self::def_mdi_child_proc_a(uc),
                "DefFrameProcA" => Self::def_frame_proc_a(uc),
                "SetWindowLongA" => Self::set_window_long_a(uc),
                "GetWindowLongA" => Self::get_window_long_a(uc),
                "SetWindowLongPtrA" => Self::set_window_long_ptr_a(uc),
                "GetWindowLongPtrA" => Self::get_window_long_ptr_a(uc),
                "CallWindowProcA" => Self::call_window_proc_a(uc),
                "PostThreadMessageA" => Self::post_thread_message_a(uc),
                "IsDialogMessageA" => Self::is_dialog_message_a(uc),
                "PostQuitMessage" => Self::post_quit_message(uc),
                "SetFocus" => Self::set_focus(uc),
                "GetFocus" => Self::get_focus(uc),
                "DispatchMessageA" => Self::dispatch_message_a(uc),
                "TranslateMessage" => Self::translate_message(uc),
                "PeekMessageA" => Self::peek_message_a(uc),
                "GetMessageA" => Self::get_message_a(uc),
                "MsgWaitForMultipleObjects" => Self::msg_wait_for_multiple_objects(uc),
                "GetWindow" => Self::get_window(uc),
                "GetParent" => Self::get_parent(uc),
                "GetDesktopWindow" => Self::get_desktop_window(uc),
                "GetActiveWindow" => Self::get_active_window(uc),
                "SetActiveWindow" => Self::set_active_window(uc),
                "GetForegroundWindow" => Self::get_foreground_window(uc),
                "SetForegroundWindow" => Self::set_foreground_window(uc),
                "GetLastActivePopup" => Self::get_last_active_popup(uc),
                "GetMenuItemInfoA" => Self::get_menu_item_info_a(uc),
                "DeleteMenu" => Self::delete_menu(uc),
                "RemoveMenu" => Self::remove_menu(uc),
                "GetSystemMenu" => Self::get_system_menu(uc),
                "GetMenu" => Self::get_menu(uc),
                "AppendMenuA" => Self::append_menu_a(uc),
                "CreateMenu" => Self::create_menu(uc),
                "DestroyMenu" => Self::destroy_menu(uc),
                "BeginPaint" => Self::begin_paint(uc),
                "EndPaint" => Self::end_paint(uc),
                "ScrollWindowEx" => Self::scroll_window_ex(uc),
                "InvalidateRect" => Self::invalidate_rect(uc),
                "SetScrollInfo" => Self::set_scroll_info(uc),
                "SetWindowTextA" => Self::set_window_text_a(uc),
                "GetWindowTextA" => Self::get_window_text_a(uc),
                "KillTimer" => Self::kill_timer(uc),
                "SetTimer" => Self::set_timer(uc),
                "MapWindowPoints" => Self::map_window_points(uc),
                "SystemParametersInfoA" => Self::system_parameters_info_a(uc),
                "TranslateMDISysAccel" => Self::translate_mdi_sys_accel(uc),
                "DrawTextA" => Self::draw_text_a(uc),
                "GetCursorPos" => Self::get_cursor_pos(uc),
                "PtInRect" => Self::pt_in_rect(uc),
                "SetRect" => Self::set_rect(uc),
                "EqualRect" => Self::equal_rect(uc),
                "UnionRect" => Self::union_rect(uc),
                "IntersectRect" => Self::intersect_rect(uc),
                "GetClipboardData" => Self::get_clipboard_data(uc),
                "OpenClipboard" => Self::open_clipboard(uc),
                "CloseClipboard" => Self::close_clipboard(uc),
                "EmptyClipboard" => Self::empty_clipboard(uc),
                "SetClipboardData" => Self::set_clipboard_data(uc),
                "IsClipboardFormatAvailable" => Self::is_clipboard_format_available(uc),
                "SetCapture" => Self::set_capture(uc),
                "GetCapture" => Self::get_capture(uc),
                "ReleaseCapture" => Self::release_capture(uc),
                "ScreenToClient" => Self::screen_to_client(uc),
                "ClientToScreen" => Self::client_to_screen(uc),
                "CreateCaret" => Self::create_caret(uc),
                "DestroyCaret" => Self::destroy_caret(uc),
                "ShowCaret" => Self::show_caret(uc),
                "HideCaret" => Self::hide_caret(uc),
                "SetCaretPos" => Self::set_caret_pos(uc),
                "GetAsyncKeyState" => Self::get_async_key_state(uc),
                "GetKeyState" => Self::get_key_state(uc),
                "GetSysColor" => Self::get_sys_color(uc),
                "SetWindowRgn" => Self::set_window_rgn(uc),
                "GetClassInfoExA" => Self::get_class_info_ex_a(uc),
                "GetClassInfoA" => Self::get_class_info_a(uc),
                "IsZoomed" => Self::is_zoomed(uc),
                "IsIconic" => Self::is_iconic(uc),
                "wsprintfA" => Self::wsprintf_a(uc),
                "EndDialog" => Self::end_dialog(uc),
                "GetPropA" => Self::get_prop_a(uc),
                "SetPropA" => Self::set_prop_a(uc),
                "RemovePropA" => Self::remove_prop_a(uc),
                "IsWindow" => Self::is_window(uc),
                _ => {
                    crate::emu_log!("[!] USER32 Unhandled: {}", func_name);
                    None
                }
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::win32::StackCleanup;

    #[test]
    fn wsprintf_uses_caller_cleanup() {
        let result = DllUSER32::wrap_result("wsprintfA", Some((2, Some(0)))).unwrap();
        assert_eq!(result.cleanup, StackCleanup::Caller);
    }

    #[test]
    fn message_box_keeps_callee_cleanup() {
        let result = DllUSER32::wrap_result("MessageBoxA", Some((4, Some(1)))).unwrap();
        assert_eq!(result.cleanup, StackCleanup::Callee(4));
    }
}
