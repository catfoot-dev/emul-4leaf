use crate::{
    dll::win32::{ApiHookResult, Win32Context, WindowState},
    helper::UnicornHelper,
};
use encoding_rs::EUC_KR;
use unicorn_engine::{RegisterX86, Unicorn};

use super::USER32;

// API: HWND CreateWindowExA(DWORD dwExStyle, LPCSTR lpClassName, LPCSTR lpWindowName, DWORD dwStyle, int X, int Y, int nWidth, int nHeight, HWND hWndParent, HMENU hMenu, HINSTANCE hInstance, LPVOID lpParam)
// 역할: 확장 스타일을 포함한 창을 생성
pub(super) fn create_window_ex_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let esp = uc.reg_read(RegisterX86::ESP).unwrap_or(0);
    let saved_call_frame: [u32; 13] = std::array::from_fn(|i| uc.read_u32(esp + (i as u64 * 4)));

    // Fake import 훅 진입 시 스택은 일반 stdcall 호출과 동일하게
    // [ESP] = return address, [ESP+4..] = 인자들이므로 `read_arg`만 사용
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

    let return_addr = uc.read_u32(esp);
    let caller_info = uc.resolve_address(return_addr);
    let ecx = uc.reg_read(RegisterX86::ECX).unwrap_or(0) as u32;
    let esi = uc.reg_read(RegisterX86::ESI).unwrap_or(0) as u32;
    let edi = uc.reg_read(RegisterX86::EDI).unwrap_or(0) as u32;
    let ebp = uc.reg_read(RegisterX86::EBP).unwrap_or(0) as u32;

    let hwnd = uc.get_data().alloc_handle();
    let (class_name, class_meta) = USER32::resolve_window_class(uc, class_addr);
    let title = if title_addr != 0 {
        uc.read_euc_kr(title_addr as u64)
    } else {
        String::new()
    };

    let (class_wnd_proc, class_cursor, hinstance) = {
        let ctx = uc.get_data();
        if let Some(class_meta) = class_meta {
            (
                class_meta.wnd_proc,
                class_meta.h_cursor,
                class_meta.hinstance,
            )
        } else if USER32::is_builtin_window_class(&class_name) {
            (USER32::def_window_proc_addr(ctx), 0, instance)
        } else {
            (0, 0, instance)
        }
    };
    let use_native_frame = USER32::is_builtin_window_class(&class_name);

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
        class_cursor,
        user_data: 0,
        use_native_frame,
        surface_bitmap,
        window_rgn: 0,
        needs_paint: true,
        last_hittest_lparam: u32::MAX,
        last_hittest_result: 0,
        z_order: 0,
    };

    {
        let ctx = uc.get_data();
        ctx.win_event
            .lock()
            .unwrap()
            .create_window(hwnd, window_state);
    }

    // WinCore의 `WndProcDispatcher`는 `WM_NCCREATE`에서 `HWND -> this` 매핑을 만든 뒤
    // `WM_CREATE`를 처리하므로, 실제 Win32와 같은 순서를 맞춰 간접 호출 경로를 엽니다.
    if class_wnd_proc != 0 {
        let cs_ptr = uc.malloc(USER32::CREATE_STRUCT_A_SIZE as usize);
        USER32::write_create_struct_a(
            uc, cs_ptr, param, hinstance, menu_or_id, parent, height, width, y, x, style,
            title_addr, class_addr, ex_style,
        );

        let nccreate_ret =
            USER32::dispatch_to_wndproc(uc, class_wnd_proc, hwnd, 0x0081, 0, cs_ptr as u32);
        // `CreateWindowExA` 훅은 현재 스택 프레임을 기준으로 RET 정리를 마치므로,
        // 생성 메시지 중 중첩 게스트 호출이 상위 호출 프레임을 건드려도 원래 인자/복귀
        // 레이아웃을 다시 맞춰 둡니다.
        for (index, value) in saved_call_frame.iter().enumerate() {
            uc.write_u32(esp + (index as u64 * 4), *value);
        }
        if nccreate_ret == 0 {
            let ctx = uc.get_data();
            USER32::cleanup_window_runtime_state(ctx, hwnd);
            ctx.win_event.lock().unwrap().destroy_window(hwnd);
            crate::emu_log!(
                "[USER32] CreateWindowExA(\"{}\") -> WM_NCCREATE rejected",
                class_name
            );
            return Some(ApiHookResult::callee(12, Some(0)));
        }

        let create_ret =
            USER32::dispatch_to_wndproc(uc, class_wnd_proc, hwnd, 0x0001, 0, cs_ptr as u32);
        for (index, value) in saved_call_frame.iter().enumerate() {
            uc.write_u32(esp + (index as u64 * 4), *value);
        }
        if create_ret == -1 {
            let ctx = uc.get_data();
            USER32::cleanup_window_runtime_state(ctx, hwnd);
            ctx.win_event.lock().unwrap().destroy_window(hwnd);
            crate::emu_log!(
                "[USER32] CreateWindowExA(\"{}\") -> WM_CREATE rejected",
                class_name
            );
            return Some(ApiHookResult::callee(12, Some(0)));
        }
    }

    // guest 쪽 생성 메시지가 모두 끝난 뒤에만 호스트 UI 창을 현실화하여,
    // 생성 도중 역주입되는 활성화/리사이즈 이벤트를 막습니다.
    uc.get_data().win_event.lock().unwrap().realize_window(hwnd);

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
        "[USER32] CreateWindowExA({:#x}, \"{}\", \"{}\", {:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> HWND {:#x} [caller: {}]",
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
        hwnd,
        caller_info
    );
    if caller_info.contains("?Create@TWindow@@QAEXPAV1@PBDHHHHIIPAUHMENU__@@@Z+0x7a") {
        let this_words = if esi >= 0x2000_0000 {
            (0..4)
                .map(|i| format!("{:#x}", uc.read_u32(esi as u64 + (i * 4) as u64)))
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            String::from("n/a")
        };
        crate::emu_log!(
            "[TRACE] CreateWindowExA caller regs: ECX={:#x} ESI(this?)={:#x} EDI={:#x} EBP={:#x} param={:#x} this_words=[{}]",
            ecx,
            esi,
            edi,
            ebp,
            param,
            this_words
        );
    }
    Some(ApiHookResult::callee(12, Some(hwnd as i32)))
}

// API: BOOL ShowWindow(HWND hWnd, int nCmdShow)
// 역할: 창의 표시 상태를 설정
pub(super) fn show_window(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn update_window(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let ctx = uc.get_data();
    let needs_paint = {
        let win_event = ctx.win_event.lock().unwrap();
        win_event
            .windows
            .get(&hwnd)
            .map(|state| state.needs_paint)
            .unwrap_or(false)
    };

    if needs_paint {
        let mut q = ctx.message_queue.lock().unwrap();
        if !q.iter().any(|m| m[0] == hwnd && m[1] == 0x000F) {
            let time = ctx.start_time.elapsed().as_millis() as u32;
            q.push_back([hwnd, 0x000F, 0, 0, time, 0, 0]);
        }
    } else {
        ctx.win_event.lock().unwrap().update_window(hwnd);
    }

    crate::emu_log!("[USER32] UpdateWindow({:#x}) -> BOOL 1", hwnd);
    Some(ApiHookResult::callee(1, Some(1)))
}

// API: BOOL DestroyWindow(HWND hWnd)
// 역할: 지정된 창을 파괴
pub(super) fn destroy_window(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let ctx = uc.get_data();
    USER32::cleanup_window_runtime_state(ctx, hwnd);
    ctx.win_event.lock().unwrap().destroy_window(hwnd);
    crate::emu_log!("[USER32] DestroyWindow({:#x}) -> BOOL 1", hwnd);
    Some(ApiHookResult::callee(1, Some(1)))
}

// API: BOOL CloseWindow(HWND hWnd)
// 역할: 지정된 창을 최소화
pub(super) fn close_window(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    uc.get_data().win_event.lock().unwrap().close_window(hwnd);
    crate::emu_log!("[USER32] CloseWindow({:#x}) -> BOOL 1", hwnd);
    Some(ApiHookResult::callee(1, Some(1)))
}

// API: BOOL EnableWindow(HWND hWnd, BOOL bEnable)
// 역할: 창의 마우스 및 키보드 입력을 활성화 또는 비활성화
pub(super) fn enable_window(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn is_window_enabled(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn is_window_visible(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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

// API: BOOL MoveWindow(HWND hWnd, int X, int Y, int nWidth, int nHeight, BOOL bRepaint)
// 역할: 창의 위치와 크기를 변경
pub(super) fn move_window(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let x = uc.read_arg(1) as i32;
    let y = uc.read_arg(2) as i32;
    let width = uc.read_arg(3);
    let height = uc.read_arg(4);
    let repaint = uc.read_arg(5);
    {
        let ctx = uc.get_data();
        let mut win_event = ctx.win_event.lock().unwrap();
        win_event.move_window(hwnd, x, y, width, height);
        if let Some(w) = win_event.windows.get_mut(&hwnd) {
            w.last_hittest_lparam = u32::MAX;
        }
    }
    uc.get_data().sync_window_surface_bitmap(hwnd);
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
pub(super) fn set_window_pos(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let insert_after = uc.read_arg(1);
    let x = uc.read_arg(2);
    let y = uc.read_arg(3);
    let cx = uc.read_arg(4);
    let cy = uc.read_arg(5);
    let flags = uc.read_arg(6);
    {
        let ctx = uc.get_data();
        let mut win_event = ctx.win_event.lock().unwrap();
        win_event.set_window_pos(hwnd, insert_after, x, y, cx, cy, flags);
        if let Some(w) = win_event.windows.get_mut(&hwnd) {
            w.last_hittest_lparam = u32::MAX;
        }
    }
    uc.get_data().sync_window_surface_bitmap(hwnd);
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
pub(super) fn get_window_rect(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn get_client_rect(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let rect_addr = uc.read_arg(1);
    let (w, h) = {
        let ctx = uc.get_data();
        let win_event = ctx.win_event.lock().unwrap();
        win_event
            .windows
            .get(&hwnd)
            .map(|win| {
                let (bw, bh, caption) = USER32::get_window_frame_size(win.style, win.ex_style);
                let mut rect_w = win.width;
                let mut rect_h = win.height;
                if !win.use_native_frame {
                    rect_w = (rect_w - bw * 2).max(0);
                    rect_h = (rect_h - bh * 2 - caption).max(0);
                }
                (rect_w, rect_h)
            })
            .unwrap_or((640, 480))
    };

    uc.write_u32(rect_addr as u64, 0);
    uc.write_u32(rect_addr as u64 + 4, 0);
    uc.write_u32(rect_addr as u64 + 8, w as u32);
    uc.write_u32(rect_addr as u64 + 12, h as u32);
    crate::emu_log!(
        "[USER32] GetClientRect({:#x}, {:#x}) -> BOOL 1 ({}x{})",
        hwnd,
        rect_addr,
        w,
        h
    );
    Some(ApiHookResult::callee(2, Some(1)))
}

// API: BOOL AdjustWindowRectEx(LPRECT lpRect, DWORD dwStyle, BOOL bMenu, DWORD dwExStyle)
// 역할: 클라이언트 영역의 크기를 기준으로 원하는 창의 크기를 계산
pub(super) fn adjust_window_rect_ex(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let rect_addr = uc.read_arg(0);
    let style = uc.read_arg(1);
    let menu = uc.read_arg(2);
    let ex_style = uc.read_arg(3);

    let left = uc.read_u32(rect_addr as u64) as i32;
    let top = uc.read_u32(rect_addr as u64 + 4) as i32;
    let right = uc.read_u32(rect_addr as u64 + 8) as i32;
    let bottom = uc.read_u32(rect_addr as u64 + 12) as i32;

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

// API: BOOL SetWindowTextA(HWND hWnd, LPCSTR lpString)
// 역할: 창의 제목 표시줄 텍스트 또는 컨트롤의 텍스트를 변경
pub(super) fn set_window_text_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn get_window_text_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let buf_addr = uc.read_arg(1);
    let max_count = uc.read_arg(2);

    let title_info = {
        let ctx = uc.get_data();
        let win_event = ctx.win_event.lock().unwrap();
        win_event.windows.get(&hwnd).map(|win| {
            let (encoded, _, _) = EUC_KR.encode(&win.title);
            let copy_len = encoded.len().min((max_count as usize).saturating_sub(1));
            (encoded[..copy_len].to_vec(), copy_len)
        })
    };

    let mut ret = 0;
    if let Some((bytes, len)) = title_info {
        USER32::write_ansi_bytes(uc, buf_addr as u64, &bytes);
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

// API: HWND GetWindow(HWND hWnd, UINT uCmd)
// 역할: 지정된 창과 관계가 있는 창의 핸들을 가져옴
pub(super) fn get_window(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn get_parent(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let ctx = uc.get_data();
    let win_event = ctx.win_event.lock().unwrap();
    let parent = win_event.windows.get(&hwnd).map(|w| w.parent).unwrap_or(0);
    crate::emu_log!("[USER32] GetParent({:#x}) -> HWND {:#x}", hwnd, parent);
    Some(ApiHookResult::callee(1, Some(parent as i32)))
}

// API: HWND GetDesktopWindow(void)
// 역할: 데스크톱 창의 핸들을 가져옴
pub(super) fn get_desktop_window(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let ctx = uc.get_data();
    let hwnd = ctx.desktop_hwnd.load(std::sync::atomic::Ordering::SeqCst);
    crate::emu_log!("[USER32] GetDesktopWindow() -> HWND {:#x}", hwnd);
    Some(ApiHookResult::callee(0, Some(hwnd as i32)))
}

// API: HWND SetActiveWindow(HWND hWnd)
// 역할: 지정된 창을 활성화함
pub(super) fn set_active_window(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn get_active_window(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let ctx = uc.get_data();
    let hwnd = ctx.active_hwnd.load(std::sync::atomic::Ordering::SeqCst);
    crate::emu_log!("[USER32] GetActiveWindow() -> HWND {:#x}", hwnd);
    Some(ApiHookResult::callee(0, Some(hwnd as i32)))
}

// API: HWND GetForegroundWindow(void)
// 역할: 포그라운드(전면) 창의 핸들을 가져옴
pub(super) fn get_foreground_window(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let ctx = uc.get_data();
    let hwnd = ctx
        .foreground_hwnd
        .load(std::sync::atomic::Ordering::SeqCst);
    crate::emu_log!("[USER32] GetForegroundWindow() -> HWND {:#x}", hwnd);
    Some(ApiHookResult::callee(0, Some(hwnd as i32)))
}

// API: BOOL SetForegroundWindow(HWND hWnd)
// 역할: 지정된 창을 포그라운드로 설정하고 활성화함
pub(super) fn set_foreground_window(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn get_last_active_popup(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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

// API: LONG SetWindowLongA(HWND hWnd, int nIndex, LONG dwNewLong)
// 역할: 윈도우의 롱을 설정
pub(super) fn set_window_long_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let index = uc.read_arg(1) as i32;
    let new_val = uc.read_arg(2);

    let mut old = 0;
    let found = {
        let ctx = uc.get_data();
        let mut win_event = ctx.win_event.lock().unwrap();
        let mut sync_style = false;
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
                    sync_style = true;
                }
                -20 => {
                    // GWL_EXSTYLE
                    old = win.ex_style;
                    win.ex_style = new_val;
                    sync_style = true;
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

            if sync_style {
                // Win32 앱이 프레임/캡션 비트를 바꾸면 호스트 창 외형도 즉시 맞춰 둡니다.
                win_event.sync_window_style(hwnd);
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
pub(super) fn get_window_long_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
                    // crate::emu_log!("[USER32] GetWindowLongA index {} not implemented", index);
                }
            }
            true
        } else {
            false
        }
    };

    if found {
        // crate::emu_log!(
        //     "[USER32] GetWindowLongA({:#x}, idx={}) -> {:#x}",
        //     hwnd,
        //     index,
        //     val
        // );
        Some(ApiHookResult::callee(2, Some(val as i32)))
    } else {
        // crate::emu_log!(
        //     "[USER32] GetWindowLongA({:#x}, idx={}) -> Window not found",
        //     hwnd,
        //     index
        // );
        Some(ApiHookResult::callee(2, Some(0)))
    }
}

// API: LONG_PTR SetWindowLongPtrA(HWND hWnd, int nIndex, LONG_PTR dwNewLong)
// 역할: 윈도우의 롱 포인터를 설정
pub(super) fn set_window_long_ptr_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    set_window_long_a(uc) // reuse SetWindowLongA for now
}

// API: LONG_PTR GetWindowLongPtrA(HWND hWnd, int nIndex)
// 역할: 윈도우의 롱 포인터를 가져옴
pub(super) fn get_window_long_ptr_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    get_window_long_a(uc) // reuse GetWindowLongA for now
}

// API: int SetWindowRgn(HWND hWnd, HRGN hRgn, BOOL bRedraw)
// 역할: 윈도우 영역 설정
pub(super) fn set_window_rgn(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let h_rgn = uc.read_arg(1);
    let b_redraw = uc.read_arg(2);

    let ctx = uc.get_data();
    let mut win_event = ctx.win_event.lock().unwrap();

    let ret = if let Some(win) = win_event.get_window_mut(hwnd) {
        let had_rgn = win.window_rgn != 0;
        let has_rgn = h_rgn != 0;
        win.window_rgn = h_rgn;
        win_event.bump_generation();
        if had_rgn != has_rgn {
            win_event.send_ui_command(crate::ui::UiCommand::SetWindowTransparent {
                hwnd,
                transparent: has_rgn,
            });
        }
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

// API: BOOL IsZoomed(HWND hWnd)
// 역할: 윈도우가 최대화되어 있는지 확인
pub(super) fn is_zoomed(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn is_iconic(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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

// API: BOOL IsWindow(HWND hWnd)
// 역할: 윈도우 핸들이 유효한지 확인
pub(super) fn is_window(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let ctx = uc.get_data();
    let win_event = ctx.win_event.lock().unwrap();
    let exists = win_event.windows.contains_key(&hwnd);
    crate::emu_log!("[USER32] IsWindow({:#x}) -> {}", hwnd, exists);
    Some(ApiHookResult::callee(1, Some(if exists { 1 } else { 0 })))
}

// API: HWND SetFocus(HWND hWnd)
// 역할: 포커스된 윈도우를 설정
pub(super) fn set_focus(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn get_focus(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let ctx = uc.get_data();
    let focus = ctx.focus_hwnd.load(std::sync::atomic::Ordering::SeqCst);
    crate::emu_log!("[USER32] GetFocus() -> HWND {:#x}", focus);
    Some(ApiHookResult::callee(0, Some(focus as i32)))
}
