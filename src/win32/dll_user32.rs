use unicorn_engine::{RegisterX86, Unicorn};

use crate::helper::{EXIT_ADDRESS, UnicornHelper};
use crate::win32::{
    ApiHookResult, Win32Context, WindowClass, WindowState, dll_kernel32::DllKERNEL32,
};

/// `USER32.dll` 프록시 구현 모듈
///
/// 윈도우 창, 클래스 관리, 메시지 루프 가상화를 담당하여 그래픽 UI 요소가 에뮬레이터 환경에서 작동하는 것처럼 모방
pub struct DllUSER32;

impl DllUSER32 {
    // API: int MessageBoxA(HWND hWnd, LPCSTR lpText, LPCSTR lpCaption, UINT uType)
    // 역할: 메시지 박스를 화면에 표시
    pub fn message_box_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
            "[USER32] MessageBoxA({:#x}, \"{}\", \"{}\", {:#x}) -> int {:#x}",
            hwnd,
            caption,
            text,
            u_type,
            result
        );
        Some(ApiHookResult::callee(4, Some(result)))
    }

    // API: ATOM RegisterClassExA(const WNDCLASSEXA* lpwcx)
    // 역할: 창 클래스를 등록
    pub fn register_class_ex_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
        Some(ApiHookResult::callee(1, Some(atom as i32)))
    }

    // API: ATOM RegisterClassA(const WNDCLASSA* lpWndClass)
    // 역할: 창 클래스를 등록
    pub fn register_class_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
        Some(ApiHookResult::callee(1, Some(atom as i32)))
    }

    // API: HWND CreateWindowExA(DWORD dwExStyle, LPCSTR lpClassName, LPCSTR lpWindowName, DWORD dwStyle, int X, int Y, int nWidth, int nHeight, HWND hWndParent, HMENU hMenu, HINSTANCE hInstance, LPVOID lpParam)
    // 역할: 확장 스타일을 포함한 창을 생성
    pub fn create_window_ex_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
        let instance = uc.read_arg(10);
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

        let (class_wnd_proc, hinstance) = {
            let ctx = uc.get_data();
            let classes = ctx.window_classes.lock().unwrap();
            classes
                .get(&class_name)
                .map(|c| (c.wnd_proc, c.hinstance))
                .unwrap_or((0, 0))
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
            wnd_proc: class_wnd_proc,
            user_data: 0,
            surface_bitmap,
            window_rgn: 0,
            needs_paint: true,
        };

        {
            let ctx = uc.get_data();
            ctx.win_event
                .lock()
                .unwrap()
                .create_window(hwnd, window_state);
        }

        // WM_CREATE (0x0001) 전송
        if class_wnd_proc != 0 {
            let cs_ptr = uc.malloc(48);
            uc.write_u32(cs_ptr + 0, param); // lpCreateParams
            uc.write_u32(cs_ptr + 4, hinstance); // hInstance
            uc.write_u32(cs_ptr + 8, menu_or_id); // hMenu
            uc.write_u32(cs_ptr + 12, parent); // hwndParent
            uc.write_u32(cs_ptr + 16, height); // cy
            uc.write_u32(cs_ptr + 20, width); // cx
            uc.write_u32(cs_ptr + 24, y as u32); // y
            uc.write_u32(cs_ptr + 28, x as u32); // x
            uc.write_u32(cs_ptr + 32, style); // style
            uc.write_u32(cs_ptr + 36, title_addr); // lpszName
            uc.write_u32(cs_ptr + 40, class_addr); // lpszClass
            uc.write_u32(cs_ptr + 44, ex_style); // dwExStyle

            Self::dispatch_to_wndproc(uc, class_wnd_proc, hwnd, 0x0001, 0, cs_ptr as u32);
        }

        // 최상위 창이라면 활성화 및 포커스 설정
        if parent == 0 {
            use std::sync::atomic::Ordering;
            let ctx = uc.get_data();
            ctx.active_hwnd.store(hwnd, Ordering::SeqCst);
            ctx.foreground_hwnd.store(hwnd, Ordering::SeqCst);
            ctx.focus_hwnd.store(hwnd, Ordering::SeqCst);

            // UI 스레드에도 활성화 알림
            ctx.win_event.lock().unwrap().activate_window(hwnd);
        }

        crate::emu_log!(
            "[USER32] CreateWindowExA({:#x}, \"{}\", \"{}\", {:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> HWND {:#x}",
            ex_style,
            class_name,
            title,
            style,
            x,
            y,
            width,
            height,
            parent,
            menu_or_id,
            instance,
            param,
            hwnd
        );
        Some(ApiHookResult::callee(12, Some(hwnd as i32)))
    }

    // API: BOOL ShowWindow(HWND hWnd, int nCmdShow)
    // 역할: 창의 표시 상태를 설정
    pub fn show_window(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
        Some(ApiHookResult::callee(2, Some(1)))
    }

    // API: BOOL UpdateWindow(HWND hWnd)
    // 역할: 창의 클라이언트 영역을 강제로 업데이트
    pub fn update_window(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        uc.get_data().win_event.lock().unwrap().update_window(hwnd);
        crate::emu_log!("[USER32] UpdateWindow({:#x}) -> BOOL 1", hwnd);
        Some(ApiHookResult::callee(1, Some(1)))
    }

    // API: BOOL DestroyWindow(HWND hWnd)
    // 역할: 지정된 창을 파괴
    pub fn destroy_window(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        uc.get_data().win_event.lock().unwrap().destroy_window(hwnd);
        crate::emu_log!("[USER32] DestroyWindow({:#x}) -> BOOL 1", hwnd);
        Some(ApiHookResult::callee(1, Some(1)))
    }

    // API: BOOL CloseWindow(HWND hWnd)
    // 역할: 지정된 창을 최소화
    pub fn close_window(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        uc.get_data().win_event.lock().unwrap().close_window(hwnd);
        crate::emu_log!("[USER32] CloseWindow({:#x}) -> BOOL 1", hwnd);
        Some(ApiHookResult::callee(1, Some(1)))
    }

    // API: BOOL EnableWindow(HWND hWnd, BOOL bEnable)
    // 역할: 창의 마우스 및 키보드 입력을 활성화 또는 비활성화
    pub fn enable_window(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
        Some(ApiHookResult::callee(2, Some(1)))
    }

    // API: BOOL IsWindowEnabled(HWND hWnd)
    // 역할: 창이 활성화되어 있는지 확인
    pub fn is_window_enabled(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let enabled = uc
            .get_data()
            .win_event
            .lock()
            .unwrap()
            .is_window_enabled(hwnd);
        let ret = if enabled { 1 } else { 0 };
        crate::emu_log!("[USER32] IsWindowEnabled({:#x}) -> BOOL {}", hwnd, ret);
        Some(ApiHookResult::callee(1, Some(ret)))
    }

    // API: BOOL IsWindowVisible(HWND hWnd)
    // 역할: 창의 가시성 상태를 확인
    pub fn is_window_visible(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let visible = uc
            .get_data()
            .win_event
            .lock()
            .unwrap()
            .is_window_visible(hwnd);
        let ret = if visible { 1 } else { 0 };
        crate::emu_log!("[USER32] IsWindowVisible({:#x}) -> BOOL {}", hwnd, ret);
        Some(ApiHookResult::callee(1, Some(ret)))
    }

    // API: DWORD MsgWaitForMultipleObjects(DWORD nCount, const HANDLE* pHandles, BOOL fWaitAll, DWORD dwMilliseconds, DWORD dwWakeMask)
    // 역할: 하나 이상의 개체 또는 메시지가 큐에 도착할 때까지 대기
    // 구현 생략 사유: 다중 스레드 동기화 객체 대기 함수. 에뮬레이터 특성상 스레드를 멈추면 전체 엔진이 멈추므로 즉각 리턴(Timeout) 처리함.
    pub fn msg_wait_for_multiple_objects(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let n_count = uc.read_arg(0);
        let p_handles = uc.read_arg(1);
        let f_wait_all = uc.read_arg(2);
        let dw_milliseconds = uc.read_arg(3);
        let dw_wake_mask = uc.read_arg(4);
        // 타 스레드 스케줄링
        DllKERNEL32::schedule_threads(uc);

        crate::emu_log!(
            "[USER32] MsgWaitForMultipleObjects({:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> DWORD 0",
            n_count,
            p_handles,
            f_wait_all,
            dw_milliseconds,
            dw_wake_mask
        );
        Some(ApiHookResult::callee(5, Some(0)))
    }

    // API: HWND GetWindow(HWND hWnd, UINT uCmd)
    // 역할: 지정된 창과 관계가 있는 창의 핸들을 가져옴
    pub fn get_window(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
        Some(ApiHookResult::callee(2, Some(parent as i32)))
    }

    // API: HWND GetParent(HWND hWnd)
    // 역할: 지정된 창의 부모 또는 소유자 창의 핸들을 가져옴
    pub fn get_parent(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let ctx = uc.get_data();
        let win_event = ctx.win_event.lock().unwrap();
        let parent = win_event.windows.get(&hwnd).map(|w| w.parent).unwrap_or(0);
        crate::emu_log!("[USER32] GetParent({:#x}) -> HWND {:#x}", hwnd, parent);
        Some(ApiHookResult::callee(1, Some(parent as i32)))
    }

    // API: HWND GetDesktopWindow(void)
    // 역할: 데스크톱 창의 핸들을 가져옴
    pub fn get_desktop_window(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let ctx = uc.get_data();
        let hwnd = ctx.desktop_hwnd.load(std::sync::atomic::Ordering::SeqCst);
        crate::emu_log!("[USER32] GetDesktopWindow() -> HWND {:#x}", hwnd);
        Some(ApiHookResult::callee(0, Some(hwnd as i32)))
    }

    // API: HWND SetActiveWindow(HWND hWnd)
    // 역할: 지정된 창을 활성화함
    pub fn set_active_window(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let ctx = uc.get_data();
        let old = ctx
            .active_hwnd
            .swap(hwnd, std::sync::atomic::Ordering::SeqCst);
        // 활성화 시 포커스도 함께 이동하는 것이 일반적
        ctx.focus_hwnd
            .store(hwnd, std::sync::atomic::Ordering::SeqCst);

        // UI 스레드 활성화 알림
        ctx.win_event.lock().unwrap().activate_window(hwnd);

        crate::emu_log!("[USER32] SetActiveWindow({:#x}) -> {:#x}", hwnd, old);
        Some(ApiHookResult::callee(1, Some(old as i32)))
    }

    // API: HWND GetActiveWindow(void)
    // 역할: 현재 스레드와 연결된 활성 창의 핸들을 가져옴
    pub fn get_active_window(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let ctx = uc.get_data();
        let hwnd = ctx.active_hwnd.load(std::sync::atomic::Ordering::SeqCst);
        crate::emu_log!("[USER32] GetActiveWindow() -> HWND {:#x}", hwnd);
        Some(ApiHookResult::callee(0, Some(hwnd as i32)))
    }

    // API: HWND GetForegroundWindow(void)
    // 역할: 포그라운드(전면) 창의 핸들을 가져옴
    pub fn get_foreground_window(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let ctx = uc.get_data();
        let hwnd = ctx
            .foreground_hwnd
            .load(std::sync::atomic::Ordering::SeqCst);
        crate::emu_log!("[USER32] GetForegroundWindow() -> HWND {:#x}", hwnd);
        Some(ApiHookResult::callee(0, Some(hwnd as i32)))
    }

    // API: BOOL SetForegroundWindow(HWND hWnd)
    // 역할: 지정된 창을 포그라운드로 설정하고 활성화함
    pub fn set_foreground_window(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let ctx = uc.get_data();
        ctx.foreground_hwnd
            .store(hwnd, std::sync::atomic::Ordering::SeqCst);
        ctx.active_hwnd
            .store(hwnd, std::sync::atomic::Ordering::SeqCst);
        ctx.focus_hwnd
            .store(hwnd, std::sync::atomic::Ordering::SeqCst);

        // UI 스레드 활성화 알림
        ctx.win_event.lock().unwrap().activate_window(hwnd);

        crate::emu_log!("[USER32] SetForegroundWindow({:#x}) -> 1", hwnd);
        Some(ApiHookResult::callee(1, Some(1)))
    }

    // API: HWND GetLastActivePopup(HWND hWnd)
    // 역할: 지정된 창에서 마지막으로 활성화된 팝업 창을 확인
    // 구현 생략 사유: 다중 창 환경의 포커스 관리용. 팝업 창을 사용하지 않으므로 무시함.
    pub fn get_last_active_popup(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        // 에뮬레이터에서는 활성 팝업을 별도로 추적하지 않으므로, 윈도우가 존재하면 해당 윈도우를 반환
        let ctx = uc.get_data();
        let win_event = ctx.win_event.lock().unwrap();
        let ret = if win_event.windows.contains_key(&hwnd) {
            hwnd
        } else {
            0
        };
        crate::emu_log!(
            "[USER32] GetLastActivePopup({:#x}) -> HWND {:#x}",
            hwnd,
            ret
        );
        Some(ApiHookResult::callee(1, Some(ret as i32)))
    }

    // API: BOOL GetMenuItemInfoA(HMENU hMenu, UINT item, BOOL fByPos, LPMENUITEMINFOA lpmii)
    // 역할: 메뉴 항목에 대한 정보를 가져옴
    // 구현 생략 사유: 메뉴 아이템 속성 조회. 에뮬레이터에서는 렌더링 가능한 시스템 메뉴 바를 그리지 않으므로 무시함.
    pub fn get_menu_item_info_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
        Some(ApiHookResult::callee(4, Some(0)))
    }

    // API: BOOL DeleteMenu(HMENU hMenu, UINT uPosition, UINT uFlags)
    // 역할: 메뉴에서 항목을 삭제
    // 구현 생략 사유: 메뉴를 렌더링하지 않으므로 항목을 삭제할 필요 없음.
    pub fn delete_menu(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
    pub fn remove_menu(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
    pub fn get_system_menu(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
    pub fn get_menu(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let handle = uc.get_data().alloc_handle();
        crate::emu_log!("[USER32] GetMenu({:#x}) -> HMENU {:#x}", hwnd, handle);
        Some(ApiHookResult::callee(1, Some(handle as i32)))
    }

    // API: BOOL AppendMenuA(HMENU hMenu, UINT uFlags, UINT_PTR uIDNewItem, LPCSTR lpNewItem)
    // 역할: 메뉴 끝에 새 항목을 추가
    // 구현 생략 사유: 시스템 메뉴 확장을 요청하지만 렌더링하지 않으므로 No-op.
    pub fn append_menu_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
    pub fn create_menu(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let ctx = uc.get_data();
        let hmenu = ctx.alloc_handle();
        crate::emu_log!("[USER32] CreateMenu() -> HMENU {:#x}", hmenu);
        Some(ApiHookResult::callee(0, Some(hmenu as i32)))
    }

    // API: BOOL DestroyMenu(HMENU hMenu)
    // 역할: 메뉴를 파괴
    // 구현 생략 사유: 메뉴 객체를 시뮬레이션하지 않으므로 리소스 해제도 불필요함.
    pub fn destroy_menu(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hmenu = uc.read_arg(0);
        crate::emu_log!("[USER32] DestroyMenu({:#x}) -> BOOL 1", hmenu);
        Some(ApiHookResult::callee(1, Some(1)))
    }

    // API: BOOL MoveWindow(HWND hWnd, int X, int Y, int nWidth, int nHeight, BOOL bRepaint)
    // 역할: 창의 위치와 크기를 변경
    pub fn move_window(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
        Some(ApiHookResult::callee(6, Some(1)))
    }

    // API: BOOL SetWindowPos(HWND hWnd, HWND hWndInsertAfter, int X, int Y, int cx, int cy, UINT uFlags)
    // 역할: 창의 크기, 위치 및 Z 순서를 변경
    pub fn set_window_pos(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
        Some(ApiHookResult::callee(7, Some(1)))
    }

    // API: BOOL GetWindowRect(HWND hWnd, LPRECT lpRect)
    // 역할: 창의 화면 좌표상의 경계 사각형 좌표를 가져옴
    pub fn get_window_rect(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
        Some(ApiHookResult::callee(2, Some(1)))
    }

    // API: BOOL GetClientRect(HWND hWnd, LPRECT lpRect)
    // 역할: 창의 클라이언트 영역 좌표를 가져옴
    pub fn get_client_rect(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
        Some(ApiHookResult::callee(2, Some(1)))
    }

    // API: BOOL AdjustWindowRectEx(LPRECT lpRect, DWORD dwStyle, BOOL bMenu, DWORD dwExStyle)
    // 역할: 클라이언트 영역의 크기를 기준으로 원하는 창의 크기를 계산
    pub fn adjust_window_rect_ex(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let rect_addr = uc.read_arg(0);
        let style = uc.read_arg(1);
        let menu = uc.read_arg(2);
        let ex_style = uc.read_arg(3);

        let mut left = uc.read_u32(rect_addr as u64) as i32;
        let mut top = uc.read_u32(rect_addr as u64 + 4) as i32;
        let mut right = uc.read_u32(rect_addr as u64 + 8) as i32;
        let mut bottom = uc.read_u32(rect_addr as u64 + 12) as i32;

        // WS_CAPTION = 0x00C00000
        if style & 0x00C00000 == 0x00C00000 {
            top -= 23; // SM_CYCAPTION (Standard)
        }

        // WS_THICKFRAME = 0x00040000
        if style & 0x00040000 != 0 {
            left -= 4; // SM_CXFRAME
            top -= 4; // SM_CYFRAME
            right += 4;
            bottom += 4;
        } else if style & 0x00800000 != 0 {
            // WS_BORDER
            left -= 1; // SM_CXBORDER
            top -= 1; // SM_CYBORDER
            right += 1;
            bottom += 1;
        }

        if menu != 0 {
            top -= 19; // SM_CYMENU
        }

        // WS_EX_CLIENTEDGE = 0x00000200
        if ex_style & 0x00000200 != 0 {
            left -= 2; // SM_CXEDGE
            top -= 2;
            right += 2;
            bottom += 2;
        }

        uc.write_u32(rect_addr as u64, left as u32);
        uc.write_u32(rect_addr as u64 + 4, top as u32);
        uc.write_u32(rect_addr as u64 + 8, right as u32);
        uc.write_u32(rect_addr as u64 + 12, bottom as u32);

        crate::emu_log!(
            "[USER32] AdjustWindowRectEx({:#x}, {:#x}, {}, {:#x}) -> BOOL 1",
            rect_addr,
            style,
            menu,
            ex_style
        );
        Some(ApiHookResult::callee(4, Some(1)))
    }

    // API: int ScrollWindowEx(HWND hWnd, int dx, int dy, const RECT* prcScroll, const RECT* prcClip, HRGN hrgnUpdate, LPRECT prcUpdate, UINT flags)
    // 역할: 창의 클라이언트 영역 내용을 스크롤
    // 구현 생략 사유: 클라이언트 영역 픽셀을 물리적으로 스크롤하는 보조 함수. 게임은 자체 루프나 BitBlt을 사용하므로 생략함.
    pub fn scroll_window_ex(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        crate::emu_log!("[USER32] ScrollWindowEx({:#x}) stubbed", hwnd);
        Some(ApiHookResult::callee(8, Some(0)))
    }

    // API: int SetScrollInfo(HWND hWnd, int nBar, LPCSCROLLINFO lpsi, BOOL redraw)
    // 역할: 스크롤 바의 매개변수를 설정
    // 구현 생략 사유: 네이티브 스크롤바 컴포넌트는 사용하지 않음.
    pub fn set_scroll_info(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        crate::emu_log!("[USER32] SetScrollInfo({:#x}) stubbed", hwnd);
        Some(ApiHookResult::callee(4, Some(0)))
    }

    // API: BOOL SetWindowTextA(HWND hWnd, LPCSTR lpString)
    // 역할: 창의 제목 표시줄 텍스트 또는 컨트롤의 텍스트를 변경
    pub fn set_window_text_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
        Some(ApiHookResult::callee(2, Some(1)))
    }

    // API: int GetWindowTextA(HWND hWnd, LPSTR lpString, int nMaxCount)
    // 역할: 창의 제목 표시줄 텍스트 또는 컨트롤의 텍스트를 가져옴
    pub fn get_window_text_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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

        let mut ret = 0;
        if let Some((text, len)) = title_info {
            uc.write_string(buf_addr as u64, &text);
            ret = len as i32;
        }
        crate::emu_log!(
            "[USER32] GetWindowTextA({:#x}, {:#x}, {:#x}) -> int {}",
            hwnd,
            buf_addr,
            max_count,
            ret
        );
        Some(ApiHookResult::callee(3, Some(ret)))
    }

    // API: UINT_PTR SetTimer(HWND hWnd, UINT_PTR nIDEvent, UINT uElapse, TIMERPROC lpTimerFunc)
    // 역할: 타이머를 생성
    pub fn set_timer(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let mut id = uc.read_arg(1);
        let elapse = uc.read_arg(2);
        let lp_timer_func = uc.read_arg(3);

        let ctx = uc.get_data();
        let mut timers = ctx.timers.lock().unwrap();
        if id == 0 {
            id = ctx.alloc_handle();
        }

        timers.insert(
            id,
            crate::win32::Timer {
                hwnd,
                id,
                elapse,
                timer_proc: lp_timer_func,
                last_tick: std::time::Instant::now(),
            },
        );

        crate::emu_log!(
            "[USER32] SetTimer({:#x}, {:#x}, {:#x}, {:#x}) -> UINT_PTR {:#x}",
            hwnd,
            id,
            elapse,
            lp_timer_func,
            id
        );
        Some(ApiHookResult::callee(4, Some(id as i32)))
    }

    // API: BOOL KillTimer(HWND hWnd, UINT_PTR uIDEvent)
    // 역할: 타이머를 제거함
    pub fn kill_timer(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let id = uc.read_arg(1);

        let ctx = uc.get_data();
        let mut timers = ctx.timers.lock().unwrap();
        let removed = timers.remove(&id).is_some();

        crate::emu_log!("[USER32] KillTimer({:#x}, {:#x}) -> {}", hwnd, id, removed);
        Some(ApiHookResult::callee(2, Some(if removed { 1 } else { 0 })))
    }

    // API: HDC BeginPaint(HWND hWnd, LPPAINTSTRUCT lpPaint)
    // 역할: 그리기를 준비하고 PAINTSTRUCT를 채움. WM_PAINT 처리 시 사용됨.
    pub fn begin_paint(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let lp_paint = uc.read_arg(1);

        let (width, height, hdc) = {
            let ctx = uc.get_data();
            let mut win_event = ctx.win_event.lock().unwrap();
            if let Some(state) = win_event.windows.get_mut(&hwnd) {
                state.needs_paint = false; // 그리기 시작했으므로 무효 영역 해제
                (state.width as u32, state.height as u32, ctx.alloc_handle())
            } else {
                return Some(ApiHookResult::callee(2, Some(0)));
            }
        };

        // PAINTSTRUCT 채우기
        uc.write_u32(lp_paint as u64 + 0, hdc); // hdc
        uc.write_u32(lp_paint as u64 + 4, 0); // fErase
        uc.write_u32(lp_paint as u64 + 8, 0); // rcPaint.left
        uc.write_u32(lp_paint as u64 + 12, 0); // rcPaint.top
        uc.write_u32(lp_paint as u64 + 16, width); // rcPaint.right
        uc.write_u32(lp_paint as u64 + 20, height); // rcPaint.bottom

        crate::emu_log!("[USER32] BeginPaint({:#x}) -> HDC {:#x}", hwnd, hdc);
        Some(ApiHookResult::callee(2, Some(hdc as i32)))
    }

    // API: BOOL EndPaint(HWND hWnd, const PAINTSTRUCT *lpPaint)
    // 역할: 그리기를 종료함
    pub fn end_paint(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        crate::emu_log!("[USER32] EndPaint({:#x}) -> 1", hwnd);
        Some(ApiHookResult::callee(2, Some(1)))
    }

    // API: BOOL InvalidateRect(HWND hWnd, const RECT *lpRect, BOOL bErase)
    // 역할: 창의 특정 영역을 무효화하여 WM_PAINT가 발생하도록 함
    pub fn invalidate_rect(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let ctx = uc.get_data();
        let mut win_event = ctx.win_event.lock().unwrap();
        if let Some(state) = win_event.windows.get_mut(&hwnd) {
            state.needs_paint = true;
            crate::emu_log!("[USER32] InvalidateRect({:#x}) -> 1", hwnd);
            Some(ApiHookResult::callee(3, Some(1)))
        } else {
            Some(ApiHookResult::callee(3, Some(0)))
        }
    }

    // API: BOOL ValidateRect(HWND hWnd, const RECT *lpRect)
    // 역할: 창의 특정 영역을 유효화함
    pub fn validate_rect(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let ctx = uc.get_data();
        let mut win_event = ctx.win_event.lock().unwrap();
        if let Some(state) = win_event.windows.get_mut(&hwnd) {
            state.needs_paint = false;
            crate::emu_log!("[USER32] ValidateRect({:#x}) -> 1", hwnd);
            Some(ApiHookResult::callee(2, Some(1)))
        } else {
            Some(ApiHookResult::callee(2, Some(0)))
        }
    }
    // 역할: 지정된 창의 클라이언트 영역에 대한 DC를 가져옴
    pub fn get_dc(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
        Some(ApiHookResult::callee(1, Some(hdc as i32)))
    }

    // API: HDC GetWindowDC(HWND hWnd)
    // 역할: 지정된 창 전체(비클라이언트 영역 포함)에 대한 DC를 가져옴
    pub fn get_window_dc(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
        Some(ApiHookResult::callee(1, Some(hdc as i32)))
    }

    // API: int ReleaseDC(HWND hWnd, HDC hDC)
    // 역할: DC를 해제
    pub fn release_dc(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let hdc = uc.read_arg(1);
        let ctx = uc.get_data();
        ctx.gdi_objects.lock().unwrap().remove(&hdc);
        ctx.win_event.lock().unwrap().update_window(hwnd);
        crate::emu_log!("[USER32] ReleaseDC({:#x}, {:#x}) -> INT 1", hwnd, hdc);
        Some(ApiHookResult::callee(2, Some(1)))
    }

    // API: LRESULT SendMessageA(HWND hWnd, UINT Msg, WPARAM wParam, LPARAM lParam)
    // 역할: 지정된 창에 메시지를 전송하고 처리가 완료될 때까지 대기
    pub fn send_message_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let msg = uc.read_arg(1);
        let wparam = uc.read_arg(2);
        let lparam = uc.read_arg(3);

        let mut ret = 0;
        match msg {
            0x000C => {
                // WM_SETTEXT
                let text = uc.read_euc_kr(lparam as u64);
                uc.get_data()
                    .win_event
                    .lock()
                    .unwrap()
                    .set_window_text(hwnd, text.clone());
                ret = 1;
            }
            0x000D => {
                // WM_GETTEXT
                let max_count = wparam as usize;
                let buf_addr = lparam as u64;
                let title = {
                    let ctx = uc.get_data();
                    let win_event = ctx.win_event.lock().unwrap();
                    win_event.windows.get(&hwnd).map(|win| {
                        let len = win.title.len().min(max_count - 1);
                        win.title[..len].to_string()
                    })
                };
                if let Some(text) = title {
                    let len = text.len();
                    uc.write_string(buf_addr, &text);
                    ret = len as i32;
                }
            }
            0x000E => {
                // WM_GETTEXTLENGTH
                let ctx = uc.get_data();
                let win_event = ctx.win_event.lock().unwrap();
                if let Some(win) = win_event.windows.get(&hwnd) {
                    ret = win.title.len() as i32;
                }
            }
            0x0031 => {
                // WM_GETFONT
                ret = 0; // Default system font
            }
            0x0700 => {
                // WM_USER
                ret = 1;
            }
            _ => {}
        }
        crate::emu_log!(
            "[USER32] SendMessageA({:#x}, {:#x}, {:#x}, {:#x}) -> LRESULT {}",
            hwnd,
            msg,
            wparam,
            lparam,
            ret
        );
        Some(ApiHookResult::callee(4, Some(ret)))
    }

    // API: BOOL PostMessageA(HWND hWnd, UINT Msg, WPARAM wParam, LPARAM lParam)
    // 역할: 지정된 창의 메시지 큐에 메시지를 배치
    pub fn post_message_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
        Some(ApiHookResult::callee(4, Some(1)))
    }

    // API: HCURSOR LoadCursorA(HINSTANCE hInstance, LPCSTR lpCursorName)
    // 역할: 커서 리소스를 로드
    pub fn load_cursor_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let instance = uc.read_arg(0);
        let lpcursorname = uc.read_arg(1);
        let ctx = uc.get_data();

        let (res_id, name) = if lpcursorname < 0x10000 {
            (lpcursorname as u32, None)
        } else {
            (0, Some(uc.read_string(lpcursorname as u64)))
        };

        let handle = ctx.alloc_handle();
        ctx.gdi_objects.lock().unwrap().insert(
            handle,
            crate::win32::GdiObject::Cursor {
                resource_id: res_id,
                name: name.clone(),
                frames: Vec::new(),
                is_animated: false,
            },
        );

        crate::emu_log!(
            "[USER32] LoadCursorA({:#x}, {}) -> HCURSOR {:#x}",
            instance,
            if let Some(n) = name {
                n
            } else {
                format!("#{}", res_id)
            },
            handle
        );
        Some(ApiHookResult::callee(2, Some(handle as i32)))
    }

    // API: HCURSOR LoadCursorFromFileA(LPCSTR lpFileName)
    // 역할: 파일에서 커서를 로드
    pub fn load_cursor_from_file_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let lpfilename = uc.read_arg(0);
        let filename = uc.read_string(lpfilename as u64);
        let ctx = uc.get_data();

        let mut frames = Vec::new();
        let mut is_animated = false;

        if let Ok(data) = std::fs::read(&filename) {
            if data.starts_with(b"RIFF") && data.len() > 12 && &data[8..12] == b"ACON" {
                // simple ANI/RIFF parser
                is_animated = true;
                let mut pos = 12;
                while pos + 8 <= data.len() {
                    let chunk_id = &data[pos..pos + 4];
                    let chunk_size =
                        u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
                    pos += 8;

                    if chunk_id == b"LIST"
                        && pos + 4 <= data.len()
                        && &data[pos..pos + 4] == b"fram"
                    {
                        let mut list_pos = pos + 4;
                        let list_end = pos + chunk_size;
                        while list_pos + 8 <= list_end && list_pos + 8 <= data.len() {
                            let item_id = &data[list_pos..list_pos + 4];
                            let item_size = u32::from_le_bytes(
                                data[list_pos + 4..list_pos + 8].try_into().unwrap(),
                            ) as usize;
                            list_pos += 8;
                            if item_id == b"icon" {
                                if let Some(frame) =
                                    Self::parse_cur_data(&data[list_pos..list_pos + item_size])
                                {
                                    frames.push(frame);
                                }
                            }
                            list_pos += (item_size + 1) & !1;
                        }
                    }
                    pos += (chunk_size + 1) & !1;
                }
            } else if data.len() > 6 && data[0] == 0 && data[1] == 0 && data[2] == 2 && data[3] == 0
            {
                // .cur file
                if let Some(frame) = Self::parse_cur_data(&data) {
                    frames.push(frame);
                }
            }
        }

        let handle = ctx.alloc_handle();
        let frames_len = frames.len();
        ctx.gdi_objects.lock().unwrap().insert(
            handle,
            crate::win32::GdiObject::Cursor {
                resource_id: 0,
                name: Some(filename.clone()),
                frames,
                is_animated,
            },
        );

        crate::emu_log!(
            "[USER32] LoadCursorFromFileA(\"{}\") -> HCURSOR {:#x} (frames: {}, animated: {})",
            filename,
            handle,
            frames_len,
            is_animated
        );
        Some(ApiHookResult::callee(1, Some(handle as i32)))
    }

    // API: HICON LoadIconA(HINSTANCE hInstance, LPCSTR lpIconName)
    // 역할: 아이콘 리소스를 로드
    pub fn load_icon_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let instance = uc.read_arg(0);
        let lpiconname = uc.read_arg(1);
        let ctx = uc.get_data();

        let (res_id, name) = if lpiconname < 0x10000 {
            (lpiconname as u32, None)
        } else {
            (0, Some(uc.read_string(lpiconname as u64)))
        };

        let handle = ctx.alloc_handle();
        ctx.gdi_objects.lock().unwrap().insert(
            handle,
            crate::win32::GdiObject::Icon {
                resource_id: res_id,
                name: name.clone(),
                frames: Vec::new(),
            },
        );

        crate::emu_log!(
            "[USER32] LoadIconA({:#x}, {}) -> HICON {:#x}",
            instance,
            if let Some(n) = name {
                n
            } else {
                format!("#{}", res_id)
            },
            handle
        );
        Some(ApiHookResult::callee(2, Some(handle as i32)))
    }

    fn parse_cur_data(data: &[u8]) -> Option<crate::win32::CursorFrame> {
        if data.len() < 22 {
            return None;
        }
        let count = u16::from_le_bytes(data[4..6].try_into().ok()?) as usize;
        if count == 0 {
            return None;
        }

        // Take the first directory entry
        let entry_offset = 6;
        let mut width = data[entry_offset] as u32;
        let mut height = data[entry_offset + 1] as u32;
        let hotspot_x =
            u16::from_le_bytes(data[entry_offset + 4..entry_offset + 6].try_into().ok()?) as i32;
        let hotspot_y =
            u16::from_le_bytes(data[entry_offset + 6..entry_offset + 8].try_into().ok()?) as i32;
        let size =
            u32::from_le_bytes(data[entry_offset + 8..entry_offset + 12].try_into().ok()?) as usize;
        let offset = u32::from_le_bytes(data[entry_offset + 12..entry_offset + 16].try_into().ok()?)
            as usize;

        if offset + size > data.len() {
            return None;
        }

        let bmp_data = &data[offset..offset + size];
        if bmp_data.len() < 40 {
            return None;
        }

        let bi_size = u32::from_le_bytes(bmp_data[0..4].try_into().ok()?);
        let bi_width = i32::from_le_bytes(bmp_data[4..8].try_into().ok()?);
        let bi_height = i32::from_le_bytes(bmp_data[8..12].try_into().ok()?);
        let bi_bit_count = u16::from_le_bytes(bmp_data[14..16].try_into().ok()?);

        if width == 0 {
            width = bi_width.abs() as u32;
        }
        if height == 0 {
            height = (bi_height.abs() / 2) as u32;
        } // CUR height in BMP is double (XOR + AND)

        let mut pixels = vec![0u32; (width * height) as usize];

        if bi_bit_count == 32 {
            let pixel_data_offset = bi_size as usize;
            for y in 0..height {
                let src_y = height - 1 - y;
                let src_row_offset = pixel_data_offset + (src_y * width * 4) as usize;
                for x in 0..width {
                    let p = src_row_offset + (x * 4) as usize;
                    if p + 3 < bmp_data.len() {
                        let b = bmp_data[p] as u32;
                        let g = bmp_data[p + 1] as u32;
                        let r = bmp_data[p + 2] as u32;
                        let a = bmp_data[p + 3] as u32;
                        pixels[(y * width + x) as usize] = (a << 24) | (r << 16) | (g << 8) | b;
                    }
                }
            }
        } else {
            // Placeholder: Magenta
            for p in pixels.iter_mut() {
                *p = 0xFFFF00FF;
            }
        }

        Some(crate::win32::CursorFrame {
            width,
            height,
            hotspot_x,
            hotspot_y,
            pixels,
        })
    }

    // API: HCURSOR SetCursor(HCURSOR hCursor)
    // 역할: 마우스 커서를 설정
    pub fn set_cursor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hcursor = uc.read_arg(0);
        let ctx = uc.get_data();
        let old = ctx
            .current_cursor
            .swap(hcursor, std::sync::atomic::Ordering::SeqCst);

        // UI 스레드에도 커서 변경 알림 (현재 포커스된 창 기준)
        let hwnd = ctx.focus_hwnd.load(std::sync::atomic::Ordering::SeqCst);
        if hwnd != 0 {
            ctx.win_event
                .lock()
                .unwrap()
                .send_ui_command(crate::ui::UiCommand::SetCursor { hwnd, hcursor });
        }

        crate::emu_log!("[USER32] SetCursor({:#x}) -> HCURSOR {:#x}", hcursor, old);
        Some(ApiHookResult::callee(1, Some(old as i32)))
    }

    // API: BOOL DestroyCursor(HCURSOR hCursor)
    // 역할: 커서를 파괴하고 사용된 메모리를 해제
    pub fn destroy_cursor(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hcursor = uc.read_arg(0);
        let ctx = uc.get_data();
        ctx.gdi_objects.lock().unwrap().remove(&hcursor);
        crate::emu_log!("[USER32] DestroyCursor({:#x}) -> BOOL 1", hcursor);
        Some(ApiHookResult::callee(1, Some(1)))
    }

    // API: int MapWindowPoints(HWND hWndFrom, HWND hWndTo, LPPOINT lpPoints, UINT cPoints)
    // 역할: 한 창의 상대 좌표를 다른 창의 상대 좌표로 변환
    pub fn map_window_points(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd_from = uc.read_arg(0);
        let hwnd_to = uc.read_arg(1);
        let lp_points = uc.read_arg(2);
        let c_points = uc.read_arg(3);

        let (from_x, from_y) = if hwnd_from == 0 {
            (0, 0)
        } else {
            let win_event = uc.get_data().win_event.lock().unwrap();
            win_event
                .windows
                .get(&hwnd_from)
                .map(|w| (w.x, w.y))
                .unwrap_or((0, 0))
        };

        let (to_x, to_y) = if hwnd_to == 0 {
            (0, 0)
        } else {
            let win_event = uc.get_data().win_event.lock().unwrap();
            win_event
                .windows
                .get(&hwnd_to)
                .map(|w| (w.x, w.y))
                .unwrap_or((0, 0))
        };

        let dx = from_x - to_x;
        let dy = from_y - to_y;

        for i in 0..c_points {
            let offset = (i as u64) * 8;
            let x = uc.read_u32(lp_points as u64 + offset) as i32;
            let y = uc.read_u32(lp_points as u64 + offset + 4) as i32;
            uc.write_u32(lp_points as u64 + offset, (x + dx) as u32);
            uc.write_u32(lp_points as u64 + offset + 4, (y + dy) as u32);
        }

        // Low word of return value is pixels horizontal, high word is pixels vertical
        let ret = (dx as u16 as u32) | ((dy as u16 as u32) << 16);
        crate::emu_log!(
            "[USER32] MapWindowPoints({:#x}, {:#x}, {:#x}, {:#x}) -> int {}",
            hwnd_from,
            hwnd_to,
            lp_points,
            c_points,
            ret
        );
        Some(ApiHookResult::callee(4, Some(ret as i32)))
    }

    // API: BOOL SystemParametersInfoA(UINT uiAction, UINT uiParam, PVOID pvParam, UINT fWinIni)
    // 역할: 시스템 전체의 매개변수를 가져오거나 설정
    pub fn system_parameters_info_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let ui_action = uc.read_arg(0);
        let ui_param = uc.read_arg(1);
        let pv_param = uc.read_arg(2);
        let f_win_ini = uc.read_arg(3);

        match ui_action {
            0x30 => {
                // SPI_GETWORKAREA
                // Return full screen area as work area
                uc.write_u32(pv_param as u64, 0); // left
                uc.write_u32(pv_param as u64 + 4, 0); // top
                uc.write_u32(pv_param as u64 + 8, 800); // right
                uc.write_u32(pv_param as u64 + 12, 600); // bottom
            }
            _ => {}
        };
        crate::emu_log!(
            "[USER32] SystemParametersInfoA({:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 1",
            ui_action,
            ui_param,
            pv_param,
            f_win_ini
        );
        Some(ApiHookResult::callee(4, Some(1)))
    }

    // API: BOOL TranslateMDISysAccel(HWND hWndClient, LPMSG lpMsg)
    // 역할: MDI 자식 창의 바로 가기 키 메시지를 처리
    pub fn translate_mdi_sys_accel(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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

    // API: int DrawTextA(HDC hDC, LPCSTR lpchText, int nCount, LPRECT lpRect, UINT uFormat)
    // 역할: 서식화된 텍스트를 사각형 내에 그림
    pub fn draw_text_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc = uc.read_arg(0);
        let lpch_text = uc.read_arg(1);
        let n_count = uc.read_arg(2);
        let lp_rect = uc.read_arg(3);
        let u_format = uc.read_arg(4);

        let text = if n_count == 0xffffffff {
            uc.read_euc_kr(lpch_text as u64)
        } else {
            // Limited length read
            uc.read_euc_kr(lpch_text as u64) // Simplification
        };

        // DT_CALCRECT = 0x00000400
        if u_format & 0x00000400 != 0 {
            let width = text.len() as i32 * 8; // Estimate
            let height = 16;
            let left = uc.read_u32(lp_rect as u64) as i32;
            let top = uc.read_u32(lp_rect as u64 + 4) as i32;
            uc.write_u32(lp_rect as u64 + 8, (left + width) as u32);
            uc.write_u32(lp_rect as u64 + 12, (top + height) as u32);
        }

        crate::emu_log!(
            "[USER32] DrawTextA({:#x}, \"{}\", {}, {:#x}, {:#x}) -> int 16",
            hdc,
            text,
            n_count,
            lp_rect,
            u_format
        );
        Some(ApiHookResult::callee(5, Some(16))) // Return line height
    }

    // API: BOOL GetCursorPos(LPPOINT lpPoint)
    // 역할: 마우스 커서의 현재 위치를 화면 좌표로 가져옴
    pub fn get_cursor_pos(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let pt_addr = uc.read_arg(0);
        let ctx = uc.get_data();
        let x = ctx.mouse_x.load(std::sync::atomic::Ordering::SeqCst);
        let y = ctx.mouse_y.load(std::sync::atomic::Ordering::SeqCst);
        uc.write_u32(pt_addr as u64, x);
        uc.write_u32(pt_addr as u64 + 4, y);
        crate::emu_log!("[USER32] GetCursorPos({:#x}) -> BOOL 1", pt_addr);
        Some(ApiHookResult::callee(1, Some(1)))
    }

    // API: BOOL PtInRect(const RECT* lprc, POINT pt)
    // 역할: 점이 사각형 내부에 있는지 확인
    pub fn pt_in_rect(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let rect_addr = uc.read_arg(0);
        let pt_x = uc.read_arg(1) as i32;
        let pt_y = uc.read_arg(2) as i32;
        let left = uc.read_u32(rect_addr as u64) as i32;
        let top = uc.read_u32(rect_addr as u64 + 4) as i32;
        let right = uc.read_u32(rect_addr as u64 + 8) as i32;
        let bottom = uc.read_u32(rect_addr as u64 + 12) as i32;
        let inside = pt_x >= left && pt_x < right && pt_y >= top && pt_y < bottom;
        let ret = if inside { 1 } else { 0 };
        crate::emu_log!(
            "[USER32] PtInRect({:#x}, {{x:{}, y:{}}}) -> BOOL {}",
            rect_addr,
            pt_x,
            pt_y,
            ret
        );
        Some(ApiHookResult::callee(3, Some(ret)))
    }

    // API: BOOL SetRect(LPRECT lprc, int xLeft, int yTop, int xRight, int yBottom)
    // 역할: 사각형의 좌표를 설정
    pub fn set_rect(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
        Some(ApiHookResult::callee(5, Some(1)))
    }

    // API: BOOL EqualRect(const RECT* lprc1, const RECT* lprc2)
    // 역할: 두 사각형이 동일한지 확인
    pub fn equal_rect(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let r1 = uc.read_arg(0);
        let r2 = uc.read_arg(1);
        let mut eq = true;
        for i in 0..4 {
            if uc.read_u32(r1 as u64 + i * 4) != uc.read_u32(r2 as u64 + i * 4) {
                eq = false;
                break;
            }
        }
        let ret = if eq { 1 } else { 0 };
        crate::emu_log!("[USER32] EqualRect({:#x}, {:#x}) -> BOOL {}", r1, r2, ret);
        Some(ApiHookResult::callee(2, Some(ret)))
    }

    // API: BOOL UnionRect(LPRECT lprcDst, const RECT* lprcSrc1, const RECT* lprcSrc2)
    // 역할: 두 사각형을 모두 포함하는 최소 사각형을 계산
    pub fn union_rect(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
        Some(ApiHookResult::callee(3, Some(1)))
    }

    // API: BOOL IntersectRect(LPRECT lprcDst, const RECT* lprcSrc1, const RECT* lprcSrc2)
    // 역할: 두 사각형의 교집합 사각형을 계산
    pub fn intersect_rect(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
        let ret = if l < r && t < b {
            uc.write_u32(dst as u64, l as u32);
            uc.write_u32(dst as u64 + 4, t as u32);
            uc.write_u32(dst as u64 + 8, r as u32);
            uc.write_u32(dst as u64 + 12, b as u32);
            1
        } else {
            uc.write_u32(dst as u64, 0);
            uc.write_u32(dst as u64 + 4, 0);
            uc.write_u32(dst as u64 + 8, 0);
            uc.write_u32(dst as u64 + 12, 0);
            0
        };
        crate::emu_log!(
            "[USER32] IntersectRect({:#x}, {:#x}, {:#x}) -> BOOL {}",
            dst,
            src1,
            src2,
            ret
        );
        Some(ApiHookResult::callee(3, Some(ret)))
    }

    // API: HANDLE GetClipboardData(UINT uFormat)
    // 역할: 클립보드에서 데이터를 가져옴
    pub fn get_clipboard_data(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
                return Some(ApiHookResult::callee(1, Some(ptr as i32)));
            }
        }
        crate::emu_log!("[USER32] GetClipboardData({:#x}) -> int 0", format);
        Some(ApiHookResult::callee(1, Some(0)))
    }

    // API: BOOL OpenClipboard(HWND hWndNewOwner)
    // 역할: 클립보드를 엶
    pub fn open_clipboard(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
        Some(ApiHookResult::callee(
            1,
            Some(if opened == 0 { 1 } else { 0 }),
        ))
    }

    // API: BOOL CloseClipboard(void)
    // 역할: 클립보드를 닫음
    pub fn close_clipboard(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let ctx = uc.get_data();
        ctx.clipboard_open
            .store(0, std::sync::atomic::Ordering::SeqCst);
        crate::emu_log!("[USER32] CloseClipboard() -> BOOL 1");
        Some(ApiHookResult::callee(0, Some(1)))
    }

    // API: BOOL EmptyClipboard(void)
    // 역할: 클립보드 비우기
    pub fn empty_clipboard(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let ctx = uc.get_data();
        ctx.clipboard_data.lock().unwrap().clear();
        crate::emu_log!("[USER32] EmptyClipboard() -> BOOL 1");
        Some(ApiHookResult::callee(0, Some(1)))
    }

    // API: HANDLE SetClipboardData(UINT uFormat, HANDLE hMem)
    // 역할: 클립보드 데이터 설정
    pub fn set_clipboard_data(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
            return Some(ApiHookResult::callee(2, Some(hmem as i32)));
        }
        Some(ApiHookResult::callee(2, Some(0)))
    }

    // API: BOOL IsClipboardFormatAvailable(UINT format)
    // 역할: 클립보드 포맷 확인
    pub fn is_clipboard_format_available(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
            "[USER32] IsClipboardFormatAvailable({:#x}) -> BOOL {}",
            format,
            available
        );
        Some(ApiHookResult::callee(1, Some(available)))
    }

    // API: HWND SetCapture(HWND hWnd)
    // 역할: 마우스 캡처 설정
    pub fn set_capture(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let ctx = uc.get_data();
        let old = ctx
            .capture_hwnd
            .swap(hwnd, std::sync::atomic::Ordering::SeqCst);
        crate::emu_log!("[USER32] SetCapture({:#x}) -> HWND {:#x}", hwnd, old);
        Some(ApiHookResult::callee(1, Some(old as i32)))
    }

    // API: HWND GetCapture(void)
    // 역할: 마우스 캡처 창 핸들
    pub fn get_capture(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let ctx = uc.get_data();
        let hwnd = ctx.capture_hwnd.load(std::sync::atomic::Ordering::SeqCst);
        crate::emu_log!("[USER32] GetCapture() -> HWND {:#x}", hwnd);
        Some(ApiHookResult::callee(0, Some(hwnd as i32)))
    }

    // API: BOOL ReleaseCapture(void)
    // 역할: 마우스 캡처 해제
    pub fn release_capture(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let ctx = uc.get_data();
        ctx.capture_hwnd
            .store(0, std::sync::atomic::Ordering::SeqCst);
        crate::emu_log!("[USER32] ReleaseCapture() -> BOOL 1");
        Some(ApiHookResult::callee(0, Some(1)))
    }

    // API: BOOL ScreenToClient(HWND hWnd, LPPOINT lpPoint)
    // 역할: 화면 좌표를 클라이언트 좌표로
    pub fn screen_to_client(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
        crate::emu_log!(
            "[USER32] ScreenToClient({:#x}, {:#x}) -> BOOL 1",
            hwnd,
            pt_addr
        );
        Some(ApiHookResult::callee(2, Some(1)))
    }

    // API: BOOL ClientToScreen(HWND hWnd, LPPOINT lpPoint)
    // 역할: 클라이언트 좌표를 화면 좌표로
    pub fn client_to_screen(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
        Some(ApiHookResult::callee(2, Some(1)))
    }

    // API: BOOL CreateCaret(HWND hWnd, HBITMAP hBitmap, int nWidth, int nHeight)
    // 역할: 캐럿 생성
    pub fn create_caret(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let hbitmap = uc.read_arg(1);
        let nwidth = uc.read_arg(2);
        let nheight = uc.read_arg(3);
        crate::emu_log!(
            "[USER32] CreateCaret({:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 1",
            hwnd,
            hbitmap,
            nwidth,
            nheight
        );
        Some(ApiHookResult::callee(4, Some(1)))
    }

    // API: BOOL DestroyCaret(void)
    // 역할: 캐럿 파괴
    pub fn destroy_caret(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        crate::emu_log!("[USER32] DestroyCaret({:#x}) -> BOOL 1", hwnd);
        Some(ApiHookResult::callee(0, Some(1)))
    }

    // API: BOOL ShowCaret(HWND hWnd)
    // 역할: 캐럿 표시
    pub fn show_caret(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        crate::emu_log!("[USER32] ShowCaret({:#x}) -> BOOL 1", hwnd);
        Some(ApiHookResult::callee(1, Some(1)))
    }

    // API: BOOL HideCaret(HWND hWnd)
    // 역할: 캐럿 숨김
    pub fn hide_caret(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        crate::emu_log!("[USER32] HideCaret({:#x}) -> BOOL 1", hwnd);
        Some(ApiHookResult::callee(1, Some(1)))
    }

    // API: BOOL SetCaretPos(int X, int Y)
    // 역할: 캐럿 위치 설정
    pub fn set_caret_pos(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let x = uc.read_arg(0);
        let y = uc.read_arg(1);
        crate::emu_log!("[USER32] SetCaretPos({:#x}, {:#x}) -> BOOL 1", x, y);
        Some(ApiHookResult::callee(2, Some(1)))
    }
    // API: SHORT GetAsyncKeyState(int vKey)
    // 역할: 가상 키 상태 확인
    pub fn get_async_key_state(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let vkey = uc.read_arg(0) as usize;
        let ctx = uc.get_data();
        let ks = ctx.key_states.lock().unwrap();
        let mut state: i32 = 0;
        if vkey < 256 && ks[vkey] {
            state = -32768; // 0x8000
        }
        crate::emu_log!(
            "[USER32] GetAsyncKeyState({:#x}) -> SHORT {:#x}",
            vkey,
            state
        );
        Some(ApiHookResult::callee(1, Some(state)))
    }

    // API: SHORT GetKeyState(int nVirtKey)
    // 역할: 가상 키 상태 확인
    pub fn get_key_state(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let vkey = uc.read_arg(0) as usize;
        let ctx = uc.get_data();
        let ks = ctx.key_states.lock().unwrap();
        let mut state: i32 = 0;
        if vkey < 256 && ks[vkey] {
            state = -32768; // 0x8000
        }
        crate::emu_log!("[USER32] GetKeyState({:#x}) -> SHORT {:#x}", vkey, state);
        Some(ApiHookResult::callee(1, Some(state)))
    }

    // API: DWORD GetSysColor(int nIndex)
    // 역할: 시스템 색상 가져오기
    pub fn get_sys_color(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let index = uc.read_arg(0);
        let color = match index {
            5 => 0x00FFFFFF,  // COLOR_WINDOW
            8 => 0x00000000,  // COLOR_WINDOWTEXT
            15 => 0x00C0C0C0, // COLOR_BTNFACE
            _ => 0x00808080,
        };
        crate::emu_log!("[USER32] GetSysColor({:#x}) -> COLOR {:#x}", index, color);
        Some(ApiHookResult::callee(1, Some(color as i32)))
    }

    // API: int SetWindowRgn(HWND hWnd, HRGN hRgn, BOOL bRedraw)
    // 역할: 윈도우 영역 설정
    pub fn set_window_rgn(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let h_rgn = uc.read_arg(1);
        let b_redraw = uc.read_arg(2);

        let ctx = uc.get_data();
        let mut win_event = ctx.win_event.lock().unwrap();

        let ret = if let Some(win) = win_event.get_window_mut(hwnd) {
            win.window_rgn = h_rgn;
            if b_redraw != 0 {
                win_event.update_window(hwnd);
            }
            1
        } else {
            0
        };

        crate::emu_log!(
            "[USER32] SetWindowRgn({:#x}, {:#x}, {:#x}) -> int {}",
            hwnd,
            h_rgn,
            b_redraw,
            ret
        );
        Some(ApiHookResult::callee(3, Some(ret)))
    }

    // API: BOOL GetClassInfoExA(HINSTANCE hinst, LPCSTR lpszClass, PWNDCLASSEXA lpwcx)
    // 역할: 윈도우 클래스 정보 가져오기
    pub fn get_class_info_ex_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
            crate::emu_log!("[USER32] GetClassInfoExA(\"{}\") -> BOOL 1", class_name);
            Some(ApiHookResult::callee(3, Some(1)))
        } else {
            crate::emu_log!("[USER32] GetClassInfoExA(\"{}\") -> BOOL 0", class_name);
            Some(ApiHookResult::callee(3, Some(0)))
        }
    }

    // API: BOOL GetClassInfoA(HINSTANCE hinst, LPCSTR lpszClass, PWNDCLASSA lpwc)
    // 역할: 윈도우 클래스 정보 가져오기
    pub fn get_class_info_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let _hinst = uc.read_arg(0);
        let class_name_ptr = uc.read_arg(1);
        let lpwc = uc.read_arg(2);

        let class_name = if class_name_ptr < 0x10000 {
            format!("Atom_{}", class_name_ptr)
        } else {
            uc.read_euc_kr(class_name_ptr as u64)
        };

        let wc_opt = {
            let ctx = uc.get_data();
            let classes = ctx.window_classes.lock().unwrap();
            classes.get(&class_name).cloned()
        };

        if let Some(wc) = wc_opt {
            uc.write_u32(lpwc as u64, wc.style);
            uc.write_u32(lpwc as u64 + 4, wc.wnd_proc);
            uc.write_u32(lpwc as u64 + 8, wc.cb_cls_extra as u32);
            uc.write_u32(lpwc as u64 + 12, wc.cb_wnd_extra as u32);
            uc.write_u32(lpwc as u64 + 16, wc.hinstance);
            uc.write_u32(lpwc as u64 + 20, wc.h_icon);
            uc.write_u32(lpwc as u64 + 24, wc.h_cursor);
            uc.write_u32(lpwc as u64 + 28, wc.hbr_background);
            uc.write_u32(lpwc as u64 + 32, 0); // lpszMenuName stub
            uc.write_u32(lpwc as u64 + 36, class_name_ptr); // lpszClassName

            crate::emu_log!("[USER32] GetClassInfoA(\"{}\") -> BOOL 1", class_name);
            Some(ApiHookResult::callee(3, Some(1)))
        } else {
            crate::emu_log!("[USER32] GetClassInfoA(\"{}\") -> BOOL 0", class_name);
            Some(ApiHookResult::callee(3, Some(0)))
        }
    }

    // API: BOOL IsZoomed(HWND hWnd)
    // 역할: 윈도우가 최대화되어 있는지 확인
    pub fn is_zoomed(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let ctx = uc.get_data();
        let win_event = ctx.win_event.lock().unwrap();
        let zoomed = win_event
            .windows
            .get(&hwnd)
            .map(|w| w.zoomed)
            .unwrap_or(false);
        crate::emu_log!("[USER32] IsZoomed({:#x}) -> BOOL {}", hwnd, zoomed);
        Some(ApiHookResult::callee(1, Some(if zoomed { 1 } else { 0 })))
    }

    // API: BOOL IsIconic(HWND hWnd)
    // 역할: 윈도우가 아이콘화되어 있는지 확인
    pub fn is_iconic(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let ctx = uc.get_data();
        let win_event = ctx.win_event.lock().unwrap();
        let iconic = win_event
            .windows
            .get(&hwnd)
            .map(|w| w.iconic)
            .unwrap_or(false);
        crate::emu_log!("[USER32] IsIconic({:#x}) -> BOOL {}", hwnd, iconic);
        Some(ApiHookResult::callee(1, Some(if iconic { 1 } else { 0 })))
    }

    // API: int wsprintfA(LPSTR lpOut, LPCSTR lpFmt, ...)
    // 역할: 문자열을 포맷팅하여 출력
    pub fn wsprintf_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
            crate::emu_log!(
                "[USER32] wsprintfA({:#x}, {:#x}) -> int {}",
                buf_addr,
                fmt_addr,
                formatted.len()
            );
            Some(ApiHookResult::callee(arg_idx, Some(formatted.len() as i32)))
        } else {
            Some(ApiHookResult::callee(2, Some(0)))
        }
    }

    // API: BOOL EndDialog(HWND hDlg, INT_PTR nResult)
    // 역할: 다이얼로그를 닫음
    pub fn end_dialog(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let h_dlg = uc.read_arg(0);
        let n_result = uc.read_arg(1);

        // 간단한 구현: 다이얼로그 윈도우를 파괴함
        uc.get_data()
            .win_event
            .lock()
            .unwrap()
            .destroy_window(h_dlg);

        crate::emu_log!(
            "[USER32] EndDialog({:#x}, {}) -> BOOL 1",
            h_dlg,
            n_result as i32
        );
        Some(ApiHookResult::callee(2, Some(1)))
    }

    // API: LRESULT DefWindowProcA(HWND hWnd, UINT Msg, WPARAM wParam, LPARAM lParam)
    // 역할: 윈도우 프로시저를 호출
    /// 지정된 윈도우 프로시저를 호출합니다. (SendMessage, DispatchMessage 등에서 공통으로 사용)
    fn dispatch_to_wndproc(
        uc: &mut Unicorn<Win32Context>,
        wnd_proc: u32,
        hwnd: u32,
        msg: u32,
        wparam: u32,
        lparam: u32,
    ) -> i32 {
        if wnd_proc == 0 {
            return 0;
        }

        // Call Wnd assignment: HWND, UINT, WPARAM, LPARAM
        uc.push_u32(lparam);
        uc.push_u32(wparam);
        uc.push_u32(msg);
        uc.push_u32(hwnd);
        uc.push_u32(EXIT_ADDRESS as u32);

        if let Err(e) = uc.emu_start(wnd_proc as u64, EXIT_ADDRESS, 0, 0) {
            crate::emu_log!(
                "[USER32] dispatch_to_wndproc: execution failed at {:#x} (msg={:#x}): {:?}",
                wnd_proc,
                msg,
                e
            );
            return 0;
        }

        uc.reg_read(RegisterX86::EAX).unwrap() as i32
    }

    pub fn def_window_proc_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let msg = uc.read_arg(1);
        let w_param = uc.read_arg(2);
        let l_param = uc.read_arg(3);
        crate::emu_log!(
            "[USER32] DefWindowProcA({:#x}, {:#x}, {:#x}, {:#x}) -> LRESULT 0",
            hwnd,
            msg,
            w_param,
            l_param
        );
        Some(ApiHookResult::callee(4, Some(0)))
    }

    // API: LRESULT DefMDIChildProcA(HWND hWnd, UINT Msg, WPARAM wParam, LPARAM lParam)
    // 역할: MDI 자식 윈도우 프로시저를 호출
    pub fn def_mdi_child_proc_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let msg = uc.read_arg(1);
        let w_param = uc.read_arg(2);
        let l_param = uc.read_arg(3);
        crate::emu_log!(
            "[USER32] DefMDIChildProcA({:#x}, {:#x}, {:#x}, {:#x}) -> LRESULT 0",
            hwnd,
            msg,
            w_param,
            l_param
        );
        Some(ApiHookResult::callee(4, Some(0)))
    }

    // API: LRESULT DefFrameProcA(HWND hWnd, HWND hWndMDIClient, UINT Msg, WPARAM wParam, LPARAM lParam)
    // 역할: 프레임 윈도우 프로시저를 호출
    pub fn def_frame_proc_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let mdi_client = uc.read_arg(1);
        let msg = uc.read_arg(2);
        let w_param = uc.read_arg(3);
        let l_param = uc.read_arg(4);
        crate::emu_log!(
            "[USER32] DefFrameProcA({:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> LRESULT 0",
            hwnd,
            mdi_client,
            msg,
            w_param,
            l_param
        );
        Some(ApiHookResult::callee(5, Some(0)))
    }

    // API: LONG SetWindowLongA(HWND hWnd, int nIndex, LONG dwNewLong)
    // 역할: 윈도우의 롱을 설정
    pub fn set_window_long_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let index = uc.read_arg(1) as i32;
        let new_val = uc.read_arg(2);

        let mut old = 0;
        let found = {
            let ctx = uc.get_data();
            let mut win_event = ctx.win_event.lock().unwrap();
            if let Some(win) = win_event.windows.get_mut(&hwnd) {
                match index {
                    -4 => {
                        // GWL_WNDPROC
                        old = win.wnd_proc;
                        win.wnd_proc = new_val;
                    }
                    -12 => {
                        // GWL_ID (GW_ID)
                        old = win.id;
                        win.id = new_val;
                    }
                    -16 => {
                        // GWL_STYLE
                        old = win.style;
                        win.style = new_val;
                    }
                    -20 => {
                        // GWL_EXSTYLE
                        old = win.ex_style;
                        win.ex_style = new_val;
                    }
                    -21 => {
                        // GWL_USERDATA
                        old = win.user_data;
                        win.user_data = new_val;
                    }
                    _ => {
                        crate::emu_log!("[USER32] SetWindowLongA index {} not implemented", index);
                    }
                }
                true
            } else {
                false
            }
        };

        if found {
            crate::emu_log!(
                "[USER32] SetWindowLongA({:#x}, {}, {:#x}) -> LONG {:#x}",
                hwnd,
                index,
                new_val,
                old
            );
            Some(ApiHookResult::callee(3, Some(old as i32)))
        } else {
            crate::emu_log!(
                "[USER32] SetWindowLongA({:#x}, {}, {:#x}) -> Window not found",
                hwnd,
                index,
                new_val
            );
            Some(ApiHookResult::callee(3, Some(0)))
        }
    }

    // API: LONG GetWindowLongA(HWND hWnd, int nIndex)
    // 역할: 윈도우의 롱을 가져옴
    pub fn get_window_long_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let index = uc.read_arg(1) as i32;

        let mut val = 0;
        let found = {
            let ctx = uc.get_data();
            let win_event = ctx.win_event.lock().unwrap();
            if let Some(win) = win_event.windows.get(&hwnd) {
                match index {
                    -4 => val = win.wnd_proc,
                    -12 => val = win.id,
                    -16 => val = win.style,
                    -20 => val = win.ex_style,
                    -21 => val = win.user_data,
                    _ => {
                        crate::emu_log!("[USER32] GetWindowLongA index {} not implemented", index);
                    }
                }
                true
            } else {
                false
            }
        };

        if found {
            crate::emu_log!(
                "[USER32] GetWindowLongA({:#x}, idx={}) -> {:#x}",
                hwnd,
                index,
                val
            );
            Some(ApiHookResult::callee(2, Some(val as i32)))
        } else {
            crate::emu_log!(
                "[USER32] GetWindowLongA({:#x}, idx={}) -> Window not found",
                hwnd,
                index
            );
            Some(ApiHookResult::callee(2, Some(0)))
        }
    }

    // API: LONG_PTR SetWindowLongPtrA(HWND hWnd, int nIndex, LONG_PTR dwNewLong)
    // 역할: 윈도우의 롱 포인터를 설정
    pub fn set_window_long_ptr_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        Self::set_window_long_a(uc) // reuse SetWindowLongA for now
    }

    // API: LONG_PTR GetWindowLongPtrA(HWND hWnd, int nIndex)
    // 역할: 윈도우의 롱 포인터를 가져옴
    pub fn get_window_long_ptr_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        Self::get_window_long_a(uc) // reuse GetWindowLongA for now
    }

    // API: LRESULT CallWindowProcA(WNDPROC lpPrevWndFunc, HWND hWnd, UINT Msg, WPARAM wParam, LPARAM lParam)
    // 역할: 윈도우 프로시저를 호출
    pub fn call_window_proc_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let lp_prev_wnd_func = uc.read_arg(0);
        let hwnd = uc.read_arg(1);
        let msg = uc.read_arg(2);
        let w_param = uc.read_arg(3);
        let l_param = uc.read_arg(4);

        let ret = Self::dispatch_to_wndproc(uc, lp_prev_wnd_func, hwnd, msg, w_param, l_param);

        crate::emu_log!(
            "[USER32] CallWindowProcA({:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> LRESULT {:#x}",
            lp_prev_wnd_func,
            hwnd,
            msg,
            w_param,
            l_param,
            ret
        );
        Some(ApiHookResult::callee(5, Some(ret)))
    }

    // API: BOOL PostThreadMessageA(DWORD idThread, UINT Msg, WPARAM wParam, LPARAM lParam)
    // 역할: 스레드에 메시지를 보냄
    pub fn post_thread_message_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
            "[USER32] PostThreadMessageA({:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 1",
            thread_id,
            msg,
            w_param,
            l_param
        );
        Some(ApiHookResult::callee(4, Some(1)))
    }

    // API: BOOL IsDialogMessageA(HWND hDlg, LPMSG lpMsg)
    // 역할: 다이얼로그 메시지를 번역
    pub fn is_dialog_message_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let h_dlg = uc.read_arg(0);
        let lp_msg = uc.read_arg(1);
        crate::emu_log!(
            "[USER32] IsDialogMessageA({:#x}, {:#x}) -> BOOL 0",
            h_dlg,
            lp_msg
        );
        Some(ApiHookResult::callee(2, Some(0)))
    }

    // API: void PostQuitMessage(int nExitCode)
    // 역할: 프로그램 종료 메시지를 보냄
    pub fn post_quit_message(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let n_exit_code = uc.read_arg(0);
        crate::emu_log!("[USER32] PostQuitMessage({}) -> void", n_exit_code);
        Some(ApiHookResult::callee(1, None))
    }

    // API: HWND SetFocus(HWND hWnd)
    // 역할: 포커스된 윈도우를 설정
    pub fn set_focus(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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

        crate::emu_log!("[USER32] SetFocus({:#x}) -> HWND {:#x}", hwnd, old);
        Some(ApiHookResult::callee(1, Some(old as i32)))
    }

    // API: HWND GetFocus(void)
    // 역할: 포커스된 윈도우를 가져옴
    pub fn get_focus(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let ctx = uc.get_data();
        let focus = ctx.focus_hwnd.load(std::sync::atomic::Ordering::SeqCst);
        crate::emu_log!("[USER32] GetFocus() -> HWND {:#x}", focus);
        Some(ApiHookResult::callee(0, Some(focus as i32)))
    }

    // API: LRESULT DispatchMessageA(const MSG* lpMsg)
    // 역할: 메시지를 디스패치
    pub fn dispatch_message_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let lp_msg = uc.read_arg(0);
        let hwnd = uc.read_u32(lp_msg as u64);
        let msg = uc.read_u32(lp_msg as u64 + 4);
        let w_param = uc.read_u32(lp_msg as u64 + 8);
        let l_param = uc.read_u32(lp_msg as u64 + 12);

        let wnd_proc = {
            let ctx = uc.get_data();
            let win_event = ctx.win_event.lock().unwrap();
            win_event
                .windows
                .get(&hwnd)
                .map(|win| win.wnd_proc)
                .unwrap_or(0)
        };

        let ret = Self::dispatch_to_wndproc(uc, wnd_proc, hwnd, msg, w_param, l_param);

        Some(ApiHookResult::callee(1, Some(ret)))
    }

    // API: BOOL TranslateMessage(const MSG* lpMsg)
    // 역할: 메시지를 번역
    pub fn translate_message(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let lp_msg = uc.read_arg(0);
        let hwnd = uc.read_u32(lp_msg as u64 + 0);
        let msg = uc.read_u32(lp_msg as u64 + 4);
        let vk = uc.read_u32(lp_msg as u64 + 8);
        let l_param = uc.read_u32(lp_msg as u64 + 12);

        // WM_KEYDOWN(0x0100) 또는 WM_SYSKEYDOWN(0x0104)인 경우에만 번역 시도
        if msg == 0x100 || msg == 0x104 {
            let mut char_code = 0;

            // 단순 VK -> ASCII 매핑 (Shift 고려)
            let shifted = {
                let ctx = uc.get_data();
                let keys = ctx.key_states.lock().unwrap();
                keys[0x10] // VK_SHIFT
            };

            if (0x30..=0x39).contains(&vk) {
                // 숫자 키
                char_code = if shifted {
                    match vk {
                        0x30 => 0x29, // )
                        0x31 => 0x21, // !
                        0x32 => 0x40, // @
                        0x33 => 0x23, // #
                        0x34 => 0x24, // $
                        0x35 => 0x25, // %
                        0x36 => 0x5e, // ^
                        0x37 => 0x26, // &
                        0x38 => 0x2a, // *
                        0x39 => 0x28, // (
                        _ => vk,
                    }
                } else {
                    vk
                };
            } else if (0x41..=0x5A).contains(&vk) {
                // 알파벳 (A-Z)
                char_code = if shifted { vk } else { vk + 0x20 }; // 대문자 or 소문자
            } else if vk == 0x20 {
                // Space
                char_code = 0x20;
            } else if vk == 0x0D {
                // Enter
                char_code = 0x0D;
            } else if vk == 0x08 {
                // Backspace
                char_code = 0x08;
            } else if vk == 0x09 {
                // Tab
                char_code = 0x09;
            } else if vk == 0x1B {
                // Escape
                char_code = 0x1B;
            }

            if char_code != 0 {
                let ctx = uc.get_data();
                let mut q = ctx.message_queue.lock().unwrap();
                // WM_CHAR(0x0102) 또는 WM_SYSCHAR(0x0106) 추가
                let char_msg = if msg == 0x0100 { 0x0102 } else { 0x0106 };
                q.push_back([hwnd, char_msg, char_code, l_param, 0, 0, 0]);

                crate::emu_log!(
                    "[USER32] TranslateMessage: Generated char {:#x} ('{}') for VK {:#x}",
                    char_code,
                    (char_code as u8 as char),
                    vk
                );
                return Some(ApiHookResult::callee(1, Some(1)));
            }
        }

        crate::emu_log!("[USER32] TranslateMessage({:#x}) -> BOOL 0", lp_msg);
        Some(ApiHookResult::callee(1, Some(0)))
    }

    // API: BOOL PeekMessageA(LPMSG lpMsg, HWND hWnd, UINT wMsgFilterMin, UINT wMsgFilterMax, UINT wRemoveMsg)
    // 역할: 메시지 큐에서 메시지를 가져옴
    pub fn peek_message_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let lp_msg = uc.read_arg(0);
        let hwnd_filter = uc.read_arg(1);
        let msg_min = uc.read_arg(2);
        let msg_max = uc.read_arg(3);
        let remove_flag = uc.read_arg(4);

        // 타 스레드 스케줄링 (협력적 멀티태스킹 유도)
        DllKERNEL32::schedule_threads(uc);

        let msg = {
            let ctx = uc.get_data();

            // 1. 타이머 체크 및 WM_TIMER 생성
            {
                let mut timers = ctx.timers.lock().unwrap();
                let mut q = ctx.message_queue.lock().unwrap();
                let now = std::time::Instant::now();
                for timer in timers.values_mut() {
                    if now.duration_since(timer.last_tick).as_millis() >= timer.elapse as u128 {
                        timer.last_tick = now;
                        // WM_TIMER(0x0113): wParam=id, lParam=timer_proc
                        q.push_back([timer.hwnd, 0x0113, timer.id, timer.timer_proc, 0, 0, 0]);
                    }
                }
            }

            let mut q = ctx.message_queue.lock().unwrap();

            let mut found_idx = None;
            for (i, m) in q.iter().enumerate() {
                let m_hwnd = m[0];
                let m_type = m[1];

                // HWND 필터링: filter가 0이면 모든 창, 아니면 특정 창만
                if hwnd_filter != 0 && m_hwnd != hwnd_filter {
                    continue;
                }

                // 메시지 범위 필터링: min/max가 0이면 모든 메시지
                if msg_min != 0 || msg_max != 0 {
                    if m_type < msg_min || m_type > msg_max {
                        continue;
                    }
                }

                found_idx = Some(i);
                break;
            }

            if let Some(idx) = found_idx {
                if (remove_flag & 0x0001) != 0 {
                    q.remove(idx)
                } else {
                    Some(q[idx])
                }
            } else {
                // 2. WM_PAINT 합성 (큐가 비어있는 경우)
                drop(q); // win_event 락을 잡기 위해 q 락 해제
                let mut synthesized = None;
                {
                    let ctx = uc.get_data();
                    let mut win_event = ctx.win_event.lock().unwrap();
                    for (hwnd, state) in win_event.windows.iter_mut() {
                        if state.needs_paint {
                            if hwnd_filter != 0 && *hwnd != hwnd_filter {
                                continue;
                            }
                            if (msg_min != 0 || msg_max != 0)
                                && (0x000F < msg_min || 0x000F > msg_max)
                            {
                                continue;
                            }
                            synthesized = Some([*hwnd, 0x000F, 0, 0, 0, 0, 0]);
                            break;
                        }
                    }
                }
                synthesized
            }
        };

        let (time, pt_x, pt_y) = {
            let ctx = uc.get_data();
            let time = ctx.start_time.elapsed().as_millis() as u32;
            let x = ctx.mouse_x.load(std::sync::atomic::Ordering::SeqCst);
            let y = ctx.mouse_y.load(std::sync::atomic::Ordering::SeqCst);
            (time, x, y)
        };

        let ret = if let Some(m) = msg {
            // MSG 구조체 채우기
            uc.write_u32(lp_msg as u64 + 0, m[0]); // hwnd
            uc.write_u32(lp_msg as u64 + 4, m[1]); // message
            uc.write_u32(lp_msg as u64 + 8, m[2]); // wParam
            uc.write_u32(lp_msg as u64 + 12, m[3]); // lParam
            uc.write_u32(lp_msg as u64 + 16, time); // time
            uc.write_u32(lp_msg as u64 + 20, m[5].max(pt_x)); // pt.x (큐 메시지 좌표 or 현재 좌표)
            uc.write_u32(lp_msg as u64 + 24, m[6].max(pt_y)); // pt.y
            1
        } else {
            0
        };

        // if ret != 0 {
        //     let m = msg.unwrap();
        //     crate::emu_log!(
        //         "[USER32] PeekMessageA({:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> FOUND msg={:#x}",
        //         lp_msg,
        //         hwnd_filter,
        //         msg_min,
        //         msg_max,
        //         remove_flag,
        //         m[1]
        //     );
        // }

        // crate::emu_log!("[USER32] Returning from PeekMessageA -> {}", ret);
        Some(ApiHookResult::callee(5, Some(ret)))
    }

    // API: BOOL GetMessageA(LPMSG lpMsg, HWND hWnd, UINT wMsgFilterMin, UINT wMsgFilterMax)
    // 역할: 메시지 큐에서 메시지를 가져옴
    pub fn get_message_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let lp_msg = uc.read_arg(0);
        let _hwnd_filter = uc.read_arg(1);
        let _min = uc.read_arg(2);
        let _max = uc.read_arg(3);

        let msg = {
            let ctx = uc.get_data(); // Immutable borrow of ctx
            let mut q = ctx.message_queue.lock().unwrap();

            if q.is_empty() {
                // Synthesize WM_PAINT if needed
                let mut paint_hwnd = 0;
                let win_event = ctx.win_event.lock().unwrap(); // Immutable borrow of win_event
                for (&h, win) in win_event.windows.iter() {
                    if win.needs_paint {
                        paint_hwnd = h;
                        break;
                    }
                }
                if paint_hwnd != 0 {
                    let time = ctx.start_time.elapsed().as_millis() as u32;
                    q.push_back([paint_hwnd, 0x000F, 0, 0, time, 0, 0]);
                }
            }

            q.pop_front()
        };

        // 메시지가 없으면 타 스레드에게 기회를 준 뒤 재시도 (GetMessage는 블로킹 API)
        if msg.is_none() {
            DllKERNEL32::schedule_threads(uc);
            // 재귀 호출 대신 루프로 구현하는 것이 좋으나, Unicorn 환경에서는 루프 내에서 emu_stop/start 제어가 복잡하므로
            // 현재는 1회 스케줄링 후 메시지가 여전히 없으면 WM_NULL을 반환하거나 짧게 대기합니다.
            // 여기서는 안정성을 위해 10ms 대기 후 다시 큐를 확인합니다.
            let mut retry_count = 0;
            let mut final_msg = msg;
            while final_msg.is_none() && retry_count < 5 {
                std::thread::sleep(std::time::Duration::from_millis(10));
                DllKERNEL32::schedule_threads(uc);
                final_msg = {
                    let ctx = uc.get_data();
                    let mut q = ctx.message_queue.lock().unwrap();
                    q.pop_front()
                };
                retry_count += 1;
            }

            if let Some(m) = final_msg {
                for i in 0..7 {
                    uc.write_u32(lp_msg as u64 + (i * 4) as u64, m[i as usize]);
                }
                let is_quit = m[1] == 0x0012;
                return Some(ApiHookResult::callee(4, Some(if is_quit { 0 } else { 1 })));
            }
        }

        if let Some(m) = msg {
            for i in 0..7 {
                uc.write_u32(lp_msg as u64 + (i * 4) as u64, m[i as usize]);
            }
            let is_quit = m[1] == 0x0012;
            Some(ApiHookResult::callee(4, Some(if is_quit { 0 } else { 1 })))
        } else {
            // No message (Note: native GetMessage blocks, but for now we return WM_NULL)
            uc.write_u32(lp_msg as u64 + 4, 0); // message = 0 (WM_NULL)
            Some(ApiHookResult::callee(4, Some(1)))
        }
    }

    // API: BOOL GetPropA(HWND hWnd, LPCSTR lpString)
    // 역할: 윈도우에서 프로퍼티를 가져옴
    pub fn get_prop_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        crate::emu_log!("[USER32] GetPropA({:#x}) -> 0", hwnd);
        Some(ApiHookResult::callee(2, Some(0)))
    }

    // API: BOOL SetPropA(HWND hWnd, LPCSTR lpString, HANDLE hData)
    // 역할: 윈도우에 프로퍼티를 설정
    pub fn set_prop_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        crate::emu_log!("[USER32] SetPropA({:#x}) -> 1", hwnd);
        Some(ApiHookResult::callee(3, Some(1)))
    }

    // API: HANDLE RemovePropA(HWND hWnd, LPCSTR lpString)
    // 역할: 윈도우에서 프로퍼티를 제거
    pub fn remove_prop_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        crate::emu_log!("[USER32] RemovePropA({:#x}) -> 0", hwnd);
        Some(ApiHookResult::callee(2, Some(0)))
    }

    // API: BOOL IsWindow(HWND hWnd)
    // 역할: 윈도우 핸들이 유효한지 확인
    pub fn is_window(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let ctx = uc.get_data();
        let win_event = ctx.win_event.lock().unwrap();
        let exists = win_event.windows.contains_key(&hwnd);
        crate::emu_log!("[USER32] IsWindow({:#x}) -> {}", hwnd, exists);
        Some(ApiHookResult::callee(1, Some(if exists { 1 } else { 0 })))
    }

    fn wrap_result(func_name: &str, result: Option<ApiHookResult>) -> Option<ApiHookResult> {
        match func_name {
            "wsprintfA" => {
                if let Some(mut res) = result {
                    res.cleanup = crate::win32::StackCleanup::Caller;
                    Some(res)
                } else {
                    None
                }
            }
            _ => result,
        }
    }

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
                "ValidateRect" => Self::validate_rect(uc),
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
        let result =
            DllUSER32::wrap_result("wsprintfA", Some(ApiHookResult::callee(2, Some(0)))).unwrap();
        assert_eq!(result.cleanup, StackCleanup::Caller);
    }

    #[test]
    fn message_box_keeps_callee_cleanup() {
        let result =
            DllUSER32::wrap_result("MessageBoxA", Some(ApiHookResult::callee(4, Some(1)))).unwrap();
        assert_eq!(result.cleanup, StackCleanup::Callee(4));
    }
}
