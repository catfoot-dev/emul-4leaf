mod class;
mod dialog;
mod input;
mod menu;
mod message;
mod nc_paint;
mod paint;
mod window;

use crate::{
    dll::win32::{ApiHookResult, StackCleanup, Timer, Win32Context, WindowClass, WindowState},
    helper::{EXIT_ADDRESS, UnicornHelper, run_nested_guest_until_exit},
};
use unicorn_engine::{RegisterX86, Unicorn};

/// `USER32.dll` 프록시 구현 모듈
///
/// 윈도우 창, 클래스 관리, 메시지 루프 가상화를 담당하여 그래픽 UI 요소가 에뮬레이터 환경에서 작동하는 것처럼 모방
pub struct USER32;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct WindowFrameMetrics {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) right: i32,
    pub(crate) bottom: i32,
}

impl USER32 {
    const CREATE_STRUCT_A_FIELD_COUNT: usize = 12;
    const CREATE_STRUCT_A_SIZE: u64 = 12 * 4;
    const CREATE_STRUCT_A_LP_CREATE_PARAMS_OFFSET: u64 = 0;
    const CREATE_STRUCT_A_HINSTANCE_OFFSET: u64 = 4;
    const CREATE_STRUCT_A_HMENU_OFFSET: u64 = 8;
    const CREATE_STRUCT_A_HWND_PARENT_OFFSET: u64 = 12;
    const CREATE_STRUCT_A_CY_OFFSET: u64 = 16;
    const CREATE_STRUCT_A_CX_OFFSET: u64 = 20;
    const CREATE_STRUCT_A_Y_OFFSET: u64 = 24;
    const CREATE_STRUCT_A_X_OFFSET: u64 = 28;
    const CREATE_STRUCT_A_STYLE_OFFSET: u64 = 32;
    const CREATE_STRUCT_A_LPSZ_NAME_OFFSET: u64 = 36;
    const CREATE_STRUCT_A_LPSZ_CLASS_OFFSET: u64 = 40;
    const CREATE_STRUCT_A_EX_STYLE_OFFSET: u64 = 44;
    pub const FRAME_BORDER_WIDTH: i32 = 3;
    pub const CAPTION_HEIGHT: i32 = 19;
    const WM_CREATE: u32 = 0x0001;
    const WM_NCCREATE: u32 = 0x0081;
    #[allow(dead_code)]
    const WM_PARENTNOTIFY: u32 = 0x0210;
    #[allow(dead_code)]
    const WS_CHILD: u32 = 0x4000_0000;
    #[allow(dead_code)]
    const WS_POPUP: u32 = 0x8000_0000;
    #[allow(dead_code)]
    const WS_EX_NOPARENTNOTIFY: u32 = 0x0000_0004;
    const HTNOWHERE: i32 = 0;
    const HTCLIENT: i32 = 1;
    const HTCAPTION: i32 = 2;
    const HTMINBUTTON: i32 = 8;
    const HTMAXBUTTON: i32 = 9;
    const HTCLOSE: i32 = 20;

    /// 만료된 타이머를 메시지 큐에 반영하되, 동일 타이머의 `WM_TIMER`는 하나만 유지합니다.
    fn enqueue_elapsed_timer_messages(
        timers: &mut std::collections::HashMap<u32, Timer>,
        queue: &mut std::collections::VecDeque<[u32; 7]>,
        now: std::time::Instant,
    ) {
        for timer in timers.values_mut() {
            if now.duration_since(timer.last_tick).as_millis() < timer.elapse as u128 {
                continue;
            }

            // 이미 같은 타이머 메시지가 큐에 있으면 추가 적재를 막아 장시간 실행 시 큐가
            // 끝없이 커지는 문제를 방지합니다.
            let already_queued = queue
                .iter()
                .any(|m| m[0] == timer.hwnd && m[1] == 0x0113 && m[2] == timer.id);
            if !already_queued {
                queue.push_back([timer.hwnd, 0x0113, timer.id, timer.timer_proc, 0, 0, 0]);
            }

            // 중복 여부와 관계없이 틱 기준 시각은 갱신하여 타이머 만료가 누적 적체되지 않게 합니다.
            timer.last_tick = now;
        }
    }

    /// 파괴된 창에 속한 런타임 상태를 정리합니다.
    fn cleanup_window_runtime_state(ctx: &Win32Context, hwnd: u32) {
        // 창이 파괴된 뒤에도 타이머와 큐 메시지가 남아 있으면 매 메시지 펌프마다
        // 불필요한 스캔과 합성이 반복되므로 즉시 정리합니다.
        ctx.timers
            .lock()
            .unwrap()
            .retain(|_, timer| timer.hwnd != hwnd);
        ctx.message_queue
            .lock()
            .unwrap()
            .retain(|msg| msg[0] != hwnd);
    }

    /// 지정된 창과 자식 창 전체를 런타임 상태까지 포함해 한 번에 파괴합니다.
    fn destroy_window_tree(ctx: &Win32Context, hwnd: u32) {
        let subtree = {
            let win_event = ctx.win_event.lock().unwrap();
            win_event.window_subtree_postorder(hwnd)
        };

        for handle in &subtree {
            Self::cleanup_window_runtime_state(ctx, *handle);
        }

        ctx.win_event.lock().unwrap().destroy_window(hwnd);
    }

    /// 현재 메시지 큐나 무효화된 창으로 인해 즉시 처리할 UI 메시지가 있는지 확인합니다.
    fn has_pending_ui_message(ctx: &Win32Context) -> bool {
        {
            let mut timers = ctx.timers.lock().unwrap();
            let mut queue = ctx.message_queue.lock().unwrap();
            Self::enqueue_elapsed_timer_messages(
                &mut timers,
                &mut queue,
                std::time::Instant::now(),
            );
            if !queue.is_empty() {
                return true;
            }
        }

        ctx.win_event
            .lock()
            .unwrap()
            .windows
            .values()
            .any(|state| state.needs_paint)
    }

    /// 게스트 ANSI 문자열을 힙에 그대로 복제해 이후에도 안정적으로 참조할 수 있게 합니다.
    fn clone_guest_c_string(uc: &mut Unicorn<Win32Context>, src_addr: u32) -> u32 {
        if src_addr == 0 || src_addr < 0x1_0000 {
            return 0;
        }

        let bytes = uc.read_string_bytes(src_addr as u64, 2048);
        let dst = uc.malloc(bytes.len() + 1);
        if !bytes.is_empty() {
            let _ = uc.mem_write(dst, &bytes);
        }
        let _ = uc.mem_write(dst + bytes.len() as u64, &[0]);
        dst as u32
    }

    /// ANSI(EUC-KR) 바이트 배열을 널 종료 문자열로 기록합니다.
    fn write_ansi_bytes(uc: &mut Unicorn<Win32Context>, addr: u64, bytes: &[u8]) {
        let _ = uc.mem_write(addr, bytes);
        let _ = uc.mem_write(addr + bytes.len() as u64, &[0]);
    }

    /// 등록된 클래스 정보를 이름 또는 atom으로 조회합니다.
    fn find_window_class(ctx: &Win32Context, class_addr: u32) -> Option<WindowClass> {
        let classes = ctx.window_classes.lock().unwrap();
        if class_addr < 0x1_0000 {
            classes.values().find(|wc| wc.atom == class_addr).cloned()
        } else {
            classes
                .values()
                .find(|wc| wc.class_name_ptr == class_addr)
                .cloned()
        }
    }

    /// 등록된 클래스 정보를 이름으로 조회합니다.
    fn find_window_class_by_name(ctx: &Win32Context, class_name: &str) -> Option<WindowClass> {
        ctx.window_classes.lock().unwrap().get(class_name).cloned()
    }

    /// USER32 기본 윈도우 프로시저의 가짜 import 주소를 찾아 반환합니다.
    fn def_window_proc_addr(ctx: &Win32Context) -> u32 {
        ctx.address_map
            .lock()
            .unwrap()
            .iter()
            .find_map(|(addr, import_name)| {
                (import_name == "USER32.dll!DefWindowProcA").then_some(*addr as u32)
            })
            .unwrap_or(0)
    }

    /// 현재 중첩된 wndproc 호출 컨텍스트를 스택에 기록합니다.
    fn push_cursor_dispatch_target(ctx: &Win32Context, hwnd: u32, msg: u32) {
        ctx.cursor_dispatch_stack.lock().unwrap().push((hwnd, msg));
    }

    /// 가장 최근 wndproc 호출 컨텍스트를 스택에서 제거합니다.
    fn pop_cursor_dispatch_target(ctx: &Win32Context) {
        ctx.cursor_dispatch_stack.lock().unwrap().pop();
    }

    /// `SetCursor`가 기본적으로 적용해야 할 대상 HWND를 결정합니다.
    pub(super) fn resolve_cursor_target_hwnd(ctx: &Win32Context) -> u32 {
        if let Some((hwnd, _)) = ctx.cursor_dispatch_stack.lock().unwrap().last().copied() {
            return hwnd;
        }

        let focus = ctx.focus_hwnd.load(std::sync::atomic::Ordering::SeqCst);
        if focus != 0 {
            return focus;
        }

        ctx.active_hwnd.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// USER32 내장 클래스는 최소한 기본 wndproc를 갖는 것으로 간주합니다.
    fn is_builtin_window_class(class_name: &str) -> bool {
        matches!(
            class_name.to_ascii_uppercase().as_str(),
            "BUTTON" | "EDIT" | "STATIC" | "LISTBOX" | "COMBOBOX" | "SCROLLBAR" | "MDICLIENT"
        )
    }

    /// `CreateWindowExA`의 `dwStyle`만 보고 호스트 네이티브 프레임 사용 여부를 판정합니다.
    ///
    /// 자식/팝업 창은 guest가 직접 외곽을 그리는 경우가 많으므로 제외하고,
    /// 일반 최상위 framed window만 네이티브 프레임 대상으로 취급합니다.
    pub(crate) fn should_use_native_frame(style: u32) -> bool {
        const WS_CAPTION: u32 = 0x00C0_0000;
        const WS_THICKFRAME: u32 = 0x0004_0000;
        const WS_DLGFRAME: u32 = 0x0040_0000;
        const WS_BORDER: u32 = 0x0080_0000;

        (style & WS_CAPTION) == WS_CAPTION
            || (style & WS_THICKFRAME) != 0
            || (style & WS_DLGFRAME) != 0
            || (style & WS_BORDER) != 0
    }

    /// `CreateWindowExA`에 들어온 클래스 식별자를 사람이 읽을 수 있는 이름과 클래스 메타데이터로 풉니다.
    fn resolve_window_class(
        uc: &mut Unicorn<Win32Context>,
        class_addr: u32,
    ) -> (String, Option<WindowClass>) {
        let ctx = uc.get_data();
        if let Some(wc) = Self::find_window_class(ctx, class_addr) {
            return (wc.class_name.clone(), Some(wc));
        }

        if class_addr < 0x1_0000 {
            return (format!("Atom_{}", class_addr), None);
        }

        let class_name = uc.read_euc_kr(class_addr as u64);
        let wc = Self::find_window_class_by_name(ctx, &class_name);
        (class_name, wc)
    }

    /// `CREATESTRUCTA`의 12개 DWORD 필드를 Win32 표준 순서로 정렬합니다.
    #[allow(clippy::too_many_arguments)]
    fn create_struct_a_words(
        lp_create_params: u32,
        hinstance: u32,
        hmenu: u32,
        hwnd_parent: u32,
        cy: u32,
        cx: u32,
        y: u32,
        x: u32,
        style: u32,
        lpsz_name: u32,
        lpsz_class: u32,
        ex_style: u32,
    ) -> [u32; Self::CREATE_STRUCT_A_FIELD_COUNT] {
        let mut words = [0; Self::CREATE_STRUCT_A_FIELD_COUNT];
        words[(Self::CREATE_STRUCT_A_LP_CREATE_PARAMS_OFFSET / 4) as usize] = lp_create_params;
        words[(Self::CREATE_STRUCT_A_HINSTANCE_OFFSET / 4) as usize] = hinstance;
        words[(Self::CREATE_STRUCT_A_HMENU_OFFSET / 4) as usize] = hmenu;
        words[(Self::CREATE_STRUCT_A_HWND_PARENT_OFFSET / 4) as usize] = hwnd_parent;
        words[(Self::CREATE_STRUCT_A_CY_OFFSET / 4) as usize] = cy;
        words[(Self::CREATE_STRUCT_A_CX_OFFSET / 4) as usize] = cx;
        words[(Self::CREATE_STRUCT_A_Y_OFFSET / 4) as usize] = y;
        words[(Self::CREATE_STRUCT_A_X_OFFSET / 4) as usize] = x;
        words[(Self::CREATE_STRUCT_A_STYLE_OFFSET / 4) as usize] = style;
        words[(Self::CREATE_STRUCT_A_LPSZ_NAME_OFFSET / 4) as usize] = lpsz_name;
        words[(Self::CREATE_STRUCT_A_LPSZ_CLASS_OFFSET / 4) as usize] = lpsz_class;
        words[(Self::CREATE_STRUCT_A_EX_STYLE_OFFSET / 4) as usize] = ex_style;
        words
    }

    /// 게스트 메모리에 `CREATESTRUCTA`를 Win32 표준 필드 순서로 기록합니다.
    #[allow(clippy::too_many_arguments)]
    fn write_create_struct_a(
        uc: &mut Unicorn<Win32Context>,
        addr: u64,
        lp_create_params: u32,
        hinstance: u32,
        hmenu: u32,
        hwnd_parent: u32,
        cy: u32,
        cx: u32,
        y: u32,
        x: u32,
        style: u32,
        lpsz_name: u32,
        lpsz_class: u32,
        ex_style: u32,
    ) {
        let words = Self::create_struct_a_words(
            lp_create_params,
            hinstance,
            hmenu,
            hwnd_parent,
            cy,
            cx,
            y,
            x,
            style,
            lpsz_name,
            lpsz_class,
            ex_style,
        );

        for (idx, value) in words.iter().enumerate() {
            uc.write_u32(addr + (idx as u64 * 4), *value);
        }
    }

    /// 중첩된 guest wndproc 호출 뒤에도 원래 `CreateWindowExA` 호출 프레임을 복원합니다.
    fn restore_saved_call_frame(
        uc: &mut Unicorn<Win32Context>,
        esp: u64,
        saved_call_frame: &[u32; 13],
    ) {
        for (index, value) in saved_call_frame.iter().enumerate() {
            uc.write_u32(esp + (index as u64 * 4), *value);
        }
    }

    /// 창 생성 중 wndproc를 호출한 뒤 원래 호출 프레임을 복원합니다.
    #[allow(clippy::too_many_arguments)]
    fn dispatch_to_wndproc_with_saved_frame(
        uc: &mut Unicorn<Win32Context>,
        esp: u64,
        saved_call_frame: &[u32; 13],
        wnd_proc: u32,
        hwnd: u32,
        msg: u32,
        w_param: u32,
        l_param: u32,
    ) -> i32 {
        let ret = Self::dispatch_to_wndproc(uc, wnd_proc, hwnd, msg, w_param, l_param);
        Self::restore_saved_call_frame(uc, esp, saved_call_frame);
        ret
    }

    /// 윈도우 스타일과 확장 스타일에 따른 프레임 두께 및 캡션 높이를 계산합니다.
    pub fn get_window_frame_size(style: u32, ex_style: u32) -> (i32, i32, i32) {
        const WS_THICKFRAME: u32 = 0x0004_0000;
        const WS_DLGFRAME: u32 = 0x0040_0000;
        const WS_BORDER: u32 = 0x0080_0000;
        const WS_CAPTION: u32 = 0x00C0_0000;
        const WS_EX_DLGMODALFRAME: u32 = 0x0000_0001;
        const WS_EX_WINDOWEDGE: u32 = 0x0000_0100;
        const WS_EX_CLIENTEDGE: u32 = 0x0000_0200;
        const WS_EX_STATICEDGE: u32 = 0x0002_0000;

        let mut bw = 0;
        let mut bh = 0;
        let mut caption = 0;

        if (style & WS_THICKFRAME) != 0 {
            bw = Self::FRAME_BORDER_WIDTH;
            bh = Self::FRAME_BORDER_WIDTH;
        } else if (style & WS_DLGFRAME) != 0 || (style & WS_BORDER) != 0 {
            bw = 1;
            bh = 1;
        }

        // 확장 스타일이 만드는 가장자리도 클라이언트 inset 계산에 반영해야
        // 자식 창 좌표계와 hit-test가 실제 배치와 어긋나지 않습니다.
        if (ex_style & (WS_EX_DLGMODALFRAME | WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE)) != 0 {
            bw += 2;
            bh += 2;
        } else if (ex_style & WS_EX_STATICEDGE) != 0 {
            bw += 1;
            bh += 1;
        }

        if (style & WS_CAPTION) == WS_CAPTION {
            caption = Self::CAPTION_HEIGHT;
        }

        (bw, bh, caption)
    }

    /// 창 상태에 맞는 실제 비클라이언트 inset을 계산합니다.
    ///
    /// 현재 구현은 guest가 선언한 `dwStyle`/`dwExStyle`만을 기준으로 삼아
    /// 클라이언트 inset을 구합니다. 리소스 아트 크기로 별도 보정하면 메시지 좌표계와
    /// 실제 child 배치 기준이 분리되므로 여기서는 일관된 Win32 기준만 사용합니다.
    pub(crate) fn get_window_frame_metrics(window: &WindowState) -> WindowFrameMetrics {
        if window.use_native_frame {
            return WindowFrameMetrics::default();
        }

        let (bw, bh, caption) = Self::get_window_frame_size(window.style, window.ex_style);
        let guest = WindowFrameMetrics {
            left: window.guest_frame_left.max(0),
            top: window.guest_frame_top.max(0),
            right: window.guest_frame_right.max(0),
            bottom: window.guest_frame_bottom.max(0),
        };
        WindowFrameMetrics {
            left: bw.max(0).max(guest.left),
            top: (bh + caption).max(0).max(guest.top),
            right: bw.max(0).max(guest.right),
            bottom: bh.max(0).max(guest.bottom),
        }
    }

    /// 기본 비클라이언트 hit-test를 계산하여 캡션 버튼과 드래그 영역을 구분합니다.
    fn default_hit_test(window: &WindowState, screen_x: i32, screen_y: i32) -> i32 {
        const WS_SYSMENU: u32 = 0x00080000;
        const WS_MINIMIZEBOX: u32 = 0x00020000;
        const WS_MAXIMIZEBOX: u32 = 0x00010000;

        if window.use_native_frame {
            return Self::HTCLIENT;
        }

        let x = screen_x - window.x;
        let y = screen_y - window.y;
        if x < 0 || x >= window.width || y < 0 || y >= window.height {
            return Self::HTNOWHERE;
        }

        let metrics = Self::get_window_frame_metrics(window);
        if metrics.top <= 0 {
            return Self::HTCLIENT;
        }

        let caption_top = 0;
        let caption_bottom = metrics.top;
        if y < caption_top || y >= caption_bottom {
            return Self::HTCLIENT;
        }

        // 우측 캡션 버튼은 닫기 -> 최대화 -> 최소화 순서로 배치됩니다.
        if (window.style & WS_SYSMENU) != 0 {
            let button_width = metrics.top.max(1);
            let mut right = window.width - metrics.right.max(0);

            let close_left = right - button_width;
            if x >= close_left && x < right {
                return Self::HTCLOSE;
            }
            right = close_left;

            if (window.style & WS_MAXIMIZEBOX) != 0 {
                let max_left = right - button_width;
                if x >= max_left && x < right {
                    return Self::HTMAXBUTTON;
                }
                right = max_left;
            }

            if (window.style & WS_MINIMIZEBOX) != 0 {
                let min_left = right - button_width;
                if x >= min_left && x < right {
                    return Self::HTMINBUTTON;
                }
            }
        }

        Self::HTCAPTION
    }

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

        // 중첩 emu_start 호출 전에 레지스터를 저장합니다.
        // WndProc이 정상적으로 ret 16 (stdcall)으로 복귀하면 ESP/EBP는 이미 복원되어 있으므로
        // 아래 복원은 no-op이 됩니다. 하지만 emu_start가 오류/emu_stop으로 중단된 경우에는
        // 스택 누수(20바이트)를 방지하고 코드 훅의 RET 명령이 올바른 주소로 복귀하도록 합니다.
        let saved_esp = uc.reg_read(RegisterX86::ESP).unwrap_or(0);
        let saved_eip = uc.reg_read(RegisterX86::EIP).unwrap_or(0);
        let saved_ebx = uc.reg_read(RegisterX86::EBX).unwrap_or(0);
        let saved_ebp = uc.reg_read(RegisterX86::EBP).unwrap_or(0);
        let saved_esi = uc.reg_read(RegisterX86::ESI).unwrap_or(0);
        let saved_edi = uc.reg_read(RegisterX86::EDI).unwrap_or(0);

        // Call Wnd assignment: HWND, UINT, WPARAM, LPARAM
        uc.push_u32(lparam);
        uc.push_u32(wparam);
        uc.push_u32(msg);
        uc.push_u32(hwnd);
        uc.push_u32(EXIT_ADDRESS as u32);

        Self::push_cursor_dispatch_target(uc.get_data(), hwnd, msg);
        uc.get_data()
            .emu_depth
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Err(e) = run_nested_guest_until_exit(uc, wnd_proc as u64) {
            let fault_eip = uc.reg_read(RegisterX86::EIP).unwrap_or(wnd_proc as u64) as u32;
            let fault_info = uc.resolve_address(fault_eip);
            let wnd_proc_info = uc.resolve_address(wnd_proc);
            crate::emu_log!(
                "[USER32] dispatch_to_wndproc: execution failed at {:#x} ({}) while dispatching {:#x} ({}) (msg={:#x}): {:?}",
                fault_eip,
                fault_info,
                wnd_proc,
                wnd_proc_info,
                msg,
                e
            );
        }
        uc.get_data()
            .emu_depth
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        Self::pop_cursor_dispatch_target(uc.get_data());

        let ret = uc.reg_read(RegisterX86::EAX).unwrap() as i32;
        let _ = uc.reg_write(RegisterX86::ESP, saved_esp);
        let _ = uc.reg_write(RegisterX86::EBX, saved_ebx);
        let _ = uc.reg_write(RegisterX86::EBP, saved_ebp);
        let _ = uc.reg_write(RegisterX86::ESI, saved_esi);
        let _ = uc.reg_write(RegisterX86::EDI, saved_edi);
        let _ = uc.reg_write(RegisterX86::EIP, saved_eip);

        ret
    }

    /// `DispatchMessageA`가 `WM_TIMER`와 함께 `TIMERPROC`를 직접 호출해야 하는 경로를 처리합니다.
    fn dispatch_to_timer_proc(
        uc: &mut Unicorn<Win32Context>,
        timer_proc: u32,
        hwnd: u32,
        timer_id: u32,
        tick_count: u32,
    ) {
        if timer_proc == 0 {
            return;
        }

        let saved_esp = uc.reg_read(RegisterX86::ESP).unwrap_or(0);
        let saved_eip = uc.reg_read(RegisterX86::EIP).unwrap_or(0);
        let saved_ebx = uc.reg_read(RegisterX86::EBX).unwrap_or(0);
        let saved_ebp = uc.reg_read(RegisterX86::EBP).unwrap_or(0);
        let saved_esi = uc.reg_read(RegisterX86::ESI).unwrap_or(0);
        let saved_edi = uc.reg_read(RegisterX86::EDI).unwrap_or(0);

        // TIMERPROC(HWND, UINT, UINT_PTR, DWORD)
        uc.push_u32(tick_count);
        uc.push_u32(timer_id);
        uc.push_u32(0x0113);
        uc.push_u32(hwnd);
        uc.push_u32(EXIT_ADDRESS as u32);

        uc.get_data()
            .emu_depth
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Err(e) = run_nested_guest_until_exit(uc, timer_proc as u64) {
            let fault_eip = uc.reg_read(RegisterX86::EIP).unwrap_or(timer_proc as u64) as u32;
            crate::emu_log!(
                "[USER32] dispatch_to_timer_proc: execution failed at {:#x} while dispatching timer {:#x}: {:?}",
                fault_eip,
                timer_proc,
                e
            );
        }
        uc.get_data()
            .emu_depth
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);

        let _ = uc.reg_write(RegisterX86::ESP, saved_esp);
        let _ = uc.reg_write(RegisterX86::EBX, saved_ebx);
        let _ = uc.reg_write(RegisterX86::EBP, saved_ebp);
        let _ = uc.reg_write(RegisterX86::ESI, saved_esi);
        let _ = uc.reg_write(RegisterX86::EDI, saved_edi);
        let _ = uc.reg_write(RegisterX86::EIP, saved_eip);
    }

    fn wrap_result(func_name: &str, result: Option<ApiHookResult>) -> Option<ApiHookResult> {
        match func_name {
            "wsprintfA" => {
                if let Some(mut res) = result {
                    res.cleanup = StackCleanup::Caller;
                    Some(res)
                } else {
                    None
                }
            }
            _ => result,
        }
    }

    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        USER32::wrap_result(
            func_name,
            match func_name {
                "GetClassInfoA" => class::get_class_info_a(uc),
                "GetClassInfoExA" => class::get_class_info_ex_a(uc),
                "RegisterClassA" => class::register_class_a(uc),
                "RegisterClassExA" => class::register_class_ex_a(uc),

                "wsprintfA" => dialog::wsprintf_a(uc),
                "EndDialog" => dialog::end_dialog(uc),
                "GetPropA" => dialog::get_prop_a(uc),
                "SetPropA" => dialog::set_prop_a(uc),
                "MessageBoxA" => dialog::message_box_a(uc),
                "RemovePropA" => dialog::remove_prop_a(uc),

                "GetCursorPos" => input::get_cursor_pos(uc),
                "PtInRect" => input::pt_in_rect(uc),
                "SetRect" => input::set_rect(uc),
                "EqualRect" => input::equal_rect(uc),
                "UnionRect" => input::union_rect(uc),
                "IntersectRect" => input::intersect_rect(uc),
                "GetClipboardData" => input::get_clipboard_data(uc),
                "OpenClipboard" => input::open_clipboard(uc),
                "CloseClipboard" => input::close_clipboard(uc),
                "EmptyClipboard" => input::empty_clipboard(uc),
                "SetClipboardData" => input::set_clipboard_data(uc),
                "IsClipboardFormatAvailable" => input::is_clipboard_format_available(uc),
                "SetCapture" => input::set_capture(uc),
                "GetCapture" => input::get_capture(uc),
                "ReleaseCapture" => input::release_capture(uc),
                "ScreenToClient" => input::screen_to_client(uc),
                "ClientToScreen" => input::client_to_screen(uc),
                "CreateCaret" => input::create_caret(uc),
                "DestroyCaret" => input::destroy_caret(uc),
                "ShowCaret" => input::show_caret(uc),
                "HideCaret" => input::hide_caret(uc),
                "SetCaretPos" => input::set_caret_pos(uc),
                "GetAsyncKeyState" => input::get_async_key_state(uc),
                "GetKeyState" => input::get_key_state(uc),
                "GetSysColor" => input::get_sys_color(uc),
                "MapWindowPoints" => input::map_window_points(uc),
                "SystemParametersInfoA" => input::system_parameters_info_a(uc),
                "LoadCursorA" => input::load_cursor_a(uc),
                "LoadCursorFromFileA" => input::load_cursor_from_file_a(uc),
                "LoadIconA" => input::load_icon_a(uc),
                "SetCursor" => input::set_cursor(uc),
                "DestroyCursor" => input::destroy_cursor(uc),

                "CreateMenu" => menu::create_menu(uc),
                "AppendMenuA" => menu::append_menu_a(uc),
                "DeleteMenu" => menu::delete_menu(uc),
                "DestroyMenu" => menu::destroy_menu(uc),
                "RemoveMenu" => menu::remove_menu(uc),
                "GetMenu" => menu::get_menu(uc),
                "GetMenuItemInfoA" => menu::get_menu_item_info_a(uc),
                "GetSystemMenu" => menu::get_system_menu(uc),
                "TranslateMDISysAccel" => menu::translate_mdi_sys_accel(uc),

                "SendMessageA" => message::send_message_a(uc),
                "PostMessageA" => message::post_message_a(uc),
                "DefWindowProcA" => message::def_window_proc_a(uc),
                "DefMDIChildProcA" => message::def_mdi_child_proc_a(uc),
                "DefFrameProcA" => message::def_frame_proc_a(uc),
                "CallWindowProcA" => message::call_window_proc_a(uc),
                "PostThreadMessageA" => message::post_thread_message_a(uc),
                "IsDialogMessageA" => message::is_dialog_message_a(uc),
                "PostQuitMessage" => message::post_quit_message(uc),
                "DispatchMessageA" => message::dispatch_message_a(uc),
                "TranslateMessage" => message::translate_message(uc),
                "PeekMessageA" => message::peek_message_a(uc),
                "GetMessageA" => message::get_message_a(uc),
                "MsgWaitForMultipleObjects" => message::msg_wait_for_multiple_objects(uc),

                "BeginPaint" => paint::begin_paint(uc),
                "EndPaint" => paint::end_paint(uc),
                "ScrollWindowEx" => paint::scroll_window_ex(uc),
                "InvalidateRect" => paint::invalidate_rect(uc),
                "ValidateRect" => paint::validate_rect(uc),
                "SetScrollInfo" => paint::set_scroll_info(uc),
                "GetDC" => paint::get_dc(uc),
                "GetWindowDC" => paint::get_window_dc(uc),
                "ReleaseDC" => paint::release_dc(uc),
                "KillTimer" => paint::kill_timer(uc),
                "SetTimer" => paint::set_timer(uc),
                "DrawTextA" => paint::draw_text_a(uc),
                "FillRect" => paint::fill_rect(uc),

                "SetFocus" => window::set_focus(uc),
                "GetFocus" => window::get_focus(uc),
                "GetWindow" => window::get_window(uc),
                "GetParent" => window::get_parent(uc),
                "GetDesktopWindow" => window::get_desktop_window(uc),
                "GetActiveWindow" => window::get_active_window(uc),
                "SetActiveWindow" => window::set_active_window(uc),
                "GetForegroundWindow" => window::get_foreground_window(uc),
                "SetForegroundWindow" => window::set_foreground_window(uc),
                "GetLastActivePopup" => window::get_last_active_popup(uc),
                "CreateWindowExA" => window::create_window_ex_a(uc),
                "ShowWindow" => window::show_window(uc),
                "UpdateWindow" => window::update_window(uc),
                "DestroyWindow" => window::destroy_window(uc),
                "CloseWindow" => window::close_window(uc),
                "EnableWindow" => window::enable_window(uc),
                "IsWindowEnabled" => window::is_window_enabled(uc),
                "IsWindowVisible" => window::is_window_visible(uc),
                "MoveWindow" => window::move_window(uc),
                "SetWindowPos" => window::set_window_pos(uc),
                "GetWindowRect" => window::get_window_rect(uc),
                "GetClientRect" => window::get_client_rect(uc),
                "AdjustWindowRectEx" => window::adjust_window_rect_ex(uc),
                "SetWindowTextA" => window::set_window_text_a(uc),
                "GetWindowTextA" => window::get_window_text_a(uc),
                "SetWindowLongA" => window::set_window_long_a(uc),
                "GetWindowLongA" => window::get_window_long_a(uc),
                "SetWindowLongPtrA" => window::set_window_long_ptr_a(uc),
                "GetWindowLongPtrA" => window::get_window_long_ptr_a(uc),
                "SetWindowRgn" => window::set_window_rgn(uc),
                "IsZoomed" => window::is_zoomed(uc),
                "IsIconic" => window::is_iconic(uc),
                "IsWindow" => window::is_window(uc),

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
    use crate::dll::win32::StackCleanup;
    use crate::helper::UnicornHelper;
    use std::collections::{HashMap, VecDeque};
    use std::time::{Duration, Instant};
    use unicorn_engine::{Arch, Mode, RegisterX86, Unicorn};

    fn new_test_uc() -> Unicorn<'static, Win32Context> {
        let mut uc =
            Unicorn::new_with_data(Arch::X86, Mode::MODE_32, Win32Context::new(None)).unwrap();
        uc.setup(None, None).unwrap();
        uc
    }

    fn write_call_frame(uc: &mut Unicorn<Win32Context>, args: &[u32]) {
        let esp = uc.reg_read(RegisterX86::ESP).unwrap() as u32;
        uc.write_u32(esp as u64, 0xDEAD_BEEF);
        for (index, value) in args.iter().enumerate() {
            uc.write_u32(esp as u64 + 4 + (index as u64 * 4), *value);
        }
    }

    fn sample_window_state(style: u32, use_native_frame: bool) -> WindowState {
        WindowState {
            class_name: "TEST".to_string(),
            class_icon: 0,
            big_icon: 0,
            small_icon: 0,
            class_hbr_background: 0,
            title: "test".to_string(),
            x: 100,
            y: 50,
            width: 200,
            height: 120,
            style,
            ex_style: 0,
            parent: 0,
            id: 0,
            visible: true,
            enabled: true,
            zoomed: false,
            iconic: false,
            wnd_proc: 0,
            class_cursor: 0,
            user_data: 0,
            use_native_frame,
            surface_bitmap: 0,
            window_rgn: 0,
            guest_frame_left: 0,
            guest_frame_top: 0,
            guest_frame_right: 0,
            guest_frame_bottom: 0,
            needs_paint: false,
            last_hittest_lparam: u32::MAX,
            last_hittest_result: 0,
            z_order: 0,
        }
    }

    #[test]
    fn wsprintf_uses_caller_cleanup() {
        let result =
            USER32::wrap_result("wsprintfA", Some(ApiHookResult::callee(2, Some(0)))).unwrap();
        assert_eq!(result.cleanup, StackCleanup::Caller);
    }

    #[test]
    fn message_box_keeps_callee_cleanup() {
        let result =
            USER32::wrap_result("MessageBoxA", Some(ApiHookResult::callee(4, Some(1)))).unwrap();
        assert_eq!(result.cleanup, StackCleanup::Callee(4));
    }

    #[test]
    fn resolve_cursor_target_prefers_current_dispatch_window() {
        let ctx = Win32Context::new(None);
        ctx.focus_hwnd
            .store(0x1002, std::sync::atomic::Ordering::SeqCst);
        ctx.active_hwnd
            .store(0x1003, std::sync::atomic::Ordering::SeqCst);

        USER32::push_cursor_dispatch_target(&ctx, 0x1001, 0x0020);
        assert_eq!(USER32::resolve_cursor_target_hwnd(&ctx), 0x1001);
        USER32::pop_cursor_dispatch_target(&ctx);
    }

    #[test]
    fn resolve_cursor_target_falls_back_to_focus_then_active() {
        let ctx = Win32Context::new(None);

        ctx.focus_hwnd
            .store(0x2001, std::sync::atomic::Ordering::SeqCst);
        ctx.active_hwnd
            .store(0x2002, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(USER32::resolve_cursor_target_hwnd(&ctx), 0x2001);

        ctx.focus_hwnd.store(0, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(USER32::resolve_cursor_target_hwnd(&ctx), 0x2002);
    }

    #[test]
    fn elapsed_timer_messages_are_coalesced() {
        let now = Instant::now();
        let mut timers = HashMap::new();
        let mut queue = VecDeque::new();

        timers.insert(
            7,
            Timer {
                hwnd: 0x1001,
                id: 7,
                elapse: 10,
                timer_proc: 0,
                last_tick: now - Duration::from_millis(20),
            },
        );
        queue.push_back([0x1001, 0x0113, 7, 0, 0, 0, 0]);

        USER32::enqueue_elapsed_timer_messages(&mut timers, &mut queue, now);

        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn destroy_window_cleanup_removes_window_timers_and_messages() {
        let ctx = Win32Context::new(None);
        ctx.message_queue.lock().unwrap().extend([
            [0x1001, 0x000F, 0, 0, 0, 0, 0],
            [0x2002, 0x000F, 0, 0, 0, 0, 0],
        ]);
        {
            let mut timers = ctx.timers.lock().unwrap();
            timers.insert(
                1,
                Timer {
                    hwnd: 0x1001,
                    id: 1,
                    elapse: 50,
                    timer_proc: 0,
                    last_tick: Instant::now(),
                },
            );
            timers.insert(
                2,
                Timer {
                    hwnd: 0x2002,
                    id: 2,
                    elapse: 50,
                    timer_proc: 0,
                    last_tick: Instant::now(),
                },
            );
        }

        USER32::cleanup_window_runtime_state(&ctx, 0x1001);

        let queue = ctx.message_queue.lock().unwrap();
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0][0], 0x2002);
        drop(queue);

        let timers = ctx.timers.lock().unwrap();
        assert_eq!(timers.len(), 1);
        assert!(timers.values().all(|timer| timer.hwnd == 0x2002));
    }

    #[test]
    fn destroy_window_tree_cleanup_removes_child_messages_and_timers() {
        let ctx = Win32Context::new(None);
        {
            let mut win_event = ctx.win_event.lock().unwrap();
            win_event.create_window(0x1000, sample_window_state(0, true));

            let mut child = sample_window_state(0x40000000, true);
            child.parent = 0x1000;
            win_event.create_window(0x1001, child);
        }

        ctx.message_queue.lock().unwrap().extend([
            [0x1000, 0x000F, 0, 0, 0, 0, 0],
            [0x1001, 0x000F, 0, 0, 0, 0, 0],
            [0x2002, 0x000F, 0, 0, 0, 0, 0],
        ]);
        {
            let mut timers = ctx.timers.lock().unwrap();
            timers.insert(
                1,
                Timer {
                    hwnd: 0x1000,
                    id: 1,
                    elapse: 50,
                    timer_proc: 0,
                    last_tick: Instant::now(),
                },
            );
            timers.insert(
                2,
                Timer {
                    hwnd: 0x1001,
                    id: 2,
                    elapse: 50,
                    timer_proc: 0,
                    last_tick: Instant::now(),
                },
            );
            timers.insert(
                3,
                Timer {
                    hwnd: 0x2002,
                    id: 3,
                    elapse: 50,
                    timer_proc: 0,
                    last_tick: Instant::now(),
                },
            );
        }

        USER32::destroy_window_tree(&ctx, 0x1000);

        let queue = ctx.message_queue.lock().unwrap();
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0][0], 0x2002);
        drop(queue);

        let timers = ctx.timers.lock().unwrap();
        assert_eq!(timers.len(), 1);
        assert!(timers.values().all(|timer| timer.hwnd == 0x2002));
        drop(timers);

        let windows = ctx.win_event.lock().unwrap();
        assert!(!windows.windows.contains_key(&0x1000));
        assert!(!windows.windows.contains_key(&0x1001));
    }

    #[test]
    fn write_create_struct_a_uses_win32_field_order() {
        let expected = [
            0x1111_1111,
            0x2222_2222,
            0x3333_3333,
            0x4444_4444,
            0x5555_5555,
            0x6666_6666,
            0x7777_7777,
            0x8888_8888,
            0x9999_9999,
            0xaaaa_aaaa,
            0xbbbb_bbbb,
            0xcccc_cccc,
        ];

        let words = USER32::create_struct_a_words(
            expected[0],
            expected[1],
            expected[2],
            expected[3],
            expected[4],
            expected[5],
            expected[6],
            expected[7],
            expected[8],
            expected[9],
            expected[10],
            expected[11],
        );

        assert_eq!(words, expected);
        assert_eq!(
            words[(USER32::CREATE_STRUCT_A_LP_CREATE_PARAMS_OFFSET / 4) as usize],
            expected[0]
        );
        assert_eq!(
            words[(USER32::CREATE_STRUCT_A_EX_STYLE_OFFSET / 4) as usize],
            expected[11]
        );
    }

    #[test]
    fn default_hit_test_detects_minimize_button() {
        let style = 0x00C00000 | 0x00080000 | 0x00020000 | 0x00010000;
        let window = sample_window_state(style, false);

        let result = USER32::default_hit_test(&window, 250, 60);

        assert_eq!(result, USER32::HTMINBUTTON);
    }

    #[test]
    fn default_hit_test_keeps_native_frame_as_client() {
        let style = 0x00C00000 | 0x00080000 | 0x00020000;
        let window = sample_window_state(style, true);

        let result = USER32::default_hit_test(&window, 250, 60);

        assert_eq!(result, USER32::HTCLIENT);
    }

    #[test]
    fn frame_metrics_include_clientedge_ex_style() {
        let mut window = sample_window_state(0, false);
        window.ex_style = 0x0000_0200;

        let metrics = USER32::get_window_frame_metrics(&window);

        assert_eq!(metrics.left, 2);
        assert_eq!(metrics.top, 2);
        assert_eq!(metrics.right, 2);
        assert_eq!(metrics.bottom, 2);
    }

    #[test]
    fn frame_metrics_include_staticedge_ex_style() {
        let mut window = sample_window_state(0, false);
        window.ex_style = 0x0002_0000;

        let metrics = USER32::get_window_frame_metrics(&window);

        assert_eq!(metrics.left, 1);
        assert_eq!(metrics.top, 1);
        assert_eq!(metrics.right, 1);
        assert_eq!(metrics.bottom, 1);
    }

    #[test]
    fn frame_metrics_stack_windowedge_on_thickframe() {
        let mut window = sample_window_state(0x0004_0000, false);
        window.ex_style = 0x0000_0100;

        let metrics = USER32::get_window_frame_metrics(&window);

        assert_eq!(metrics.left, USER32::FRAME_BORDER_WIDTH + 2);
        assert_eq!(metrics.top, USER32::FRAME_BORDER_WIDTH + 2);
        assert_eq!(metrics.right, USER32::FRAME_BORDER_WIDTH + 2);
        assert_eq!(metrics.bottom, USER32::FRAME_BORDER_WIDTH + 2);
    }

    #[test]
    fn frame_metrics_merge_guest_managed_popup_insets() {
        let mut window = sample_window_state(0, false);
        window.guest_frame_left = 8;
        window.guest_frame_top = 8;
        window.guest_frame_right = 8;
        window.guest_frame_bottom = 8;

        let metrics = USER32::get_window_frame_metrics(&window);

        assert_eq!(metrics.left, 8);
        assert_eq!(metrics.top, 8);
        assert_eq!(metrics.right, 8);
        assert_eq!(metrics.bottom, 8);
    }

    #[test]
    #[cfg_attr(
        target_arch = "aarch64",
        ignore = "cargo test 러너에서 Unicorn 초기화가 SIGILL을 유발함"
    )]
    fn get_client_rect_uses_zero_based_client_coordinates() {
        let mut uc = new_test_uc();
        {
            let mut win_event = uc.get_data().win_event.lock().unwrap();
            let mut state = sample_window_state(0, false);
            state.width = 200;
            state.height = 120;
            state.ex_style = 0x0000_0200;
            win_event.create_window(0x1000, state);
        }
        let rect_ptr = uc.malloc(16) as u32;
        for index in 0..4 {
            uc.write_u32(
                rect_ptr as u64 + (index * 4) as u64,
                0xAAAA_0000 | index as u32,
            );
        }
        write_call_frame(&mut uc, &[0x1000, rect_ptr]);

        let result = window::get_client_rect(&mut uc).expect("get_client_rect result");

        assert_eq!(result.return_value, Some(1));
        assert_eq!(uc.read_u32(rect_ptr as u64), 0);
        assert_eq!(uc.read_u32(rect_ptr as u64 + 4), 0);
        assert_eq!(uc.read_u32(rect_ptr as u64 + 8), 196);
        assert_eq!(uc.read_u32(rect_ptr as u64 + 12), 116);
    }

    #[test]
    #[cfg_attr(
        target_arch = "aarch64",
        ignore = "cargo test 러너에서 Unicorn 초기화가 SIGILL을 유발함"
    )]
    fn peek_message_clears_stale_msg_buffer_when_queue_is_empty() {
        let mut uc = new_test_uc();
        let msg_ptr = uc.malloc(28) as u32;
        for index in 0..7 {
            uc.write_u32(
                msg_ptr as u64 + (index * 4) as u64,
                0xAAAA_0000 | index as u32,
            );
        }
        write_call_frame(&mut uc, &[msg_ptr, 0, 0, 0, 0]);

        let result = crate::dll::win32::user32::message::peek_message_a(&mut uc)
            .expect("peek_message_a result");

        assert_eq!(result.return_value, Some(0));
        for index in 0..7 {
            assert_eq!(uc.read_u32(msg_ptr as u64 + (index * 4) as u64), 0);
        }
    }

    #[test]
    #[cfg_attr(
        target_arch = "aarch64",
        ignore = "cargo test 러너에서 Unicorn 초기화가 SIGILL을 유발함"
    )]
    fn def_window_proc_set_icon_updates_window_state_and_returns_previous_icon() {
        let mut uc = new_test_uc();
        {
            let mut win_event = uc.get_data().win_event.lock().unwrap();
            let mut state = sample_window_state(0, true);
            state.class_icon = 0x1111;
            state.big_icon = 0x2222;
            win_event.create_window(0x1000, state);
        }
        write_call_frame(&mut uc, &[0x1000, 0x0080, 1, 0x3333]);

        let result = message::def_window_proc_a(&mut uc).expect("def_window_proc_a result");

        assert_eq!(result.return_value, Some(0x2222));
        let win_event = uc.get_data().win_event.lock().unwrap();
        let state = win_event.windows.get(&0x1000).unwrap();
        assert_eq!(state.big_icon, 0x3333);
        assert_eq!(state.small_icon, 0);
    }

    #[test]
    #[cfg_attr(
        target_arch = "aarch64",
        ignore = "cargo test 러너에서 Unicorn 초기화가 SIGILL을 유발함"
    )]
    fn def_window_proc_get_minmaxinfo_populates_work_area_defaults() {
        let mut uc = new_test_uc();
        uc.get_data().set_work_area(10, 20, 310, 260);
        {
            let mut win_event = uc.get_data().win_event.lock().unwrap();
            win_event.create_window(0x1000, sample_window_state(0x00C00000, false));
        }
        let minmax_ptr = uc.malloc(40) as u32;
        for index in 0..10 {
            uc.write_u32(minmax_ptr as u64 + (index * 4) as u64, 0);
        }
        write_call_frame(&mut uc, &[0x1000, 0x0024, 0, minmax_ptr]);

        let result = message::def_window_proc_a(&mut uc).expect("def_window_proc_a result");

        assert_eq!(result.return_value, Some(0));
        assert_eq!(uc.read_u32(minmax_ptr as u64 + 8), 300);
        assert_eq!(uc.read_u32(minmax_ptr as u64 + 12), 240);
        assert_eq!(uc.read_u32(minmax_ptr as u64 + 16), 10);
        assert_eq!(uc.read_u32(minmax_ptr as u64 + 20), 20);
        assert_eq!(uc.read_u32(minmax_ptr as u64 + 24), 3);
        assert_eq!(uc.read_u32(minmax_ptr as u64 + 28), 22);
        assert_eq!(uc.read_u32(minmax_ptr as u64 + 32), 300);
        assert_eq!(uc.read_u32(minmax_ptr as u64 + 36), 240);
    }
}
