use crate::{
    dll::win32::{ApiHookResult, Win32Context, kernel32::KERNEL32},
    helper::UnicornHelper,
};
use encoding_rs::EUC_KR;
use std::time::Instant;
use unicorn_engine::Unicorn;

use super::USER32;

fn write_point(uc: &mut Unicorn<Win32Context>, addr: u64, x: i32, y: i32) {
    uc.write_u32(addr, x as u32);
    uc.write_u32(addr + 4, y as u32);
}

/// `MSG` 구조체 7개 DWORD 슬롯을 0으로 초기화합니다.
fn clear_msg_struct(uc: &mut Unicorn<Win32Context>, lp_msg: u32) {
    for index in 0..7 {
        uc.write_u32(lp_msg as u64 + (index * 4) as u64, 0);
    }
}

fn translated_char_from_vk(vk: u32) -> Option<u32> {
    if (0x20..=0x7E).contains(&vk) || matches!(vk, 0x08 | 0x09 | 0x0D | 0x1B) {
        Some(vk)
    } else {
        None
    }
}

fn earlier_deadline(left: Option<Instant>, right: Option<Instant>) -> Option<Instant> {
    match (left, right) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn request_main_window_exit_fallback(ctx: &Win32Context, hwnd: u32, msg: u32, w_param: u32) {
    let should_exit = {
        let win_event = ctx.win_event.lock().unwrap();
        win_event.should_exit_after_guest_close_message(hwnd, msg, w_param)
    };

    if should_exit {
        crate::emu_log!(
            "[USER32] DispatchMessageA fallback exit requested for main HWND {:#x} msg={:#x} wParam={:#x}",
            hwnd,
            msg,
            w_param
        );
        ctx.win_event
            .lock()
            .unwrap()
            .send_ui_command(crate::ui::UiCommand::ExitApplication);
    }
}

// API: LRESULT SendMessageA(HWND hWnd, UINT Msg, WPARAM wParam, LPARAM lParam)
// 역할: 지정된 창에 메시지를 전송하고 처리가 완료될 때까지 대기
pub(super) fn send_message_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let msg = uc.read_arg(1);
    let wparam = uc.read_arg(2);
    let lparam = uc.read_arg(3);
    let wnd_proc = {
        let ctx = uc.get_data();
        let win_event = ctx.win_event.lock().unwrap();
        win_event
            .windows
            .get(&hwnd)
            .map(|win| win.wnd_proc)
            .unwrap_or(0)
    };

    let ret = match msg {
        0x000C => {
            // WM_SETTEXT
            let text = uc.read_euc_kr(lparam as u64);
            uc.get_data()
                .win_event
                .lock()
                .unwrap()
                .set_window_text(hwnd, text.clone());
            if wnd_proc != 0 {
                USER32::dispatch_to_wndproc(uc, wnd_proc, hwnd, msg, wparam, lparam)
            } else {
                1
            }
        }
        0x000D => {
            // WM_GETTEXT
            if wnd_proc != 0 {
                USER32::dispatch_to_wndproc(uc, wnd_proc, hwnd, msg, wparam, lparam)
            } else {
                let max_count = wparam as usize;
                let buf_addr = lparam as u64;
                let title = {
                    let ctx = uc.get_data();
                    let win_event = ctx.win_event.lock().unwrap();
                    win_event.windows.get(&hwnd).map(|win| {
                        let (encoded, _, _) = EUC_KR.encode(&win.title);
                        let copy_len = encoded.len().min(max_count.saturating_sub(1));
                        encoded[..copy_len].to_vec()
                    })
                };
                if let Some(bytes) = title {
                    let len = bytes.len();
                    USER32::write_ansi_bytes(uc, buf_addr, &bytes);
                    len as i32
                } else {
                    0
                }
            }
        }
        0x000E => {
            // WM_GETTEXTLENGTH
            if wnd_proc != 0 {
                USER32::dispatch_to_wndproc(uc, wnd_proc, hwnd, msg, wparam, lparam)
            } else {
                let ctx = uc.get_data();
                let win_event = ctx.win_event.lock().unwrap();
                win_event
                    .windows
                    .get(&hwnd)
                    .map(|win| win.title.len() as i32)
                    .unwrap_or(0)
            }
        }
        0x0031 => {
            // WM_GETFONT
            if wnd_proc != 0 {
                USER32::dispatch_to_wndproc(uc, wnd_proc, hwnd, msg, wparam, lparam)
            } else {
                0 // Default system font
            }
        }
        0x0700 => {
            // 게임이 `WM_USER` 이상 커스텀 메시지를 광범위하게 사용하므로 실제 wndproc로 전달합니다.
            if wnd_proc != 0 {
                USER32::dispatch_to_wndproc(uc, wnd_proc, hwnd, msg, wparam, lparam)
            } else {
                1
            }
        }
        _ => {
            if wnd_proc != 0 {
                USER32::dispatch_to_wndproc(uc, wnd_proc, hwnd, msg, wparam, lparam)
            } else {
                0
            }
        }
    };
    // crate::emu_log!(
    //     "[USER32] SendMessageA({:#x}, {:#x}, {:#x}, {:#x}) -> LRESULT {}",
    //     hwnd,
    //     msg,
    //     wparam,
    //     lparam,
    //     ret
    // );
    Some(ApiHookResult::callee(4, Some(ret)))
}

// API: BOOL PostMessageA(HWND hWnd, UINT Msg, WPARAM wParam, LPARAM lParam)
// 역할: 지정된 창의 메시지 큐에 메시지를 배치
pub(super) fn post_message_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let msg = uc.read_arg(1);
    let wparam = uc.read_arg(2);
    let lparam = uc.read_arg(3);
    let time = crate::diagnostics::virtual_millis(uc.get_data().start_time);
    let ctx = uc.get_data();
    let target_tid = ctx.queue_message_for_window(hwnd, [hwnd, msg, wparam, lparam, time, 0, 0]);
    ctx.wake_thread_message_wait(target_tid);
    crate::emu_log!(
        "[USER32] PostMessageA({:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 1",
        hwnd,
        msg,
        wparam,
        lparam
    );
    Some(ApiHookResult::callee(4, Some(1)))
}

// API: LRESULT DispatchMessageA(const MSG* lpMsg)
// 역할: 메시지를 디스패치
pub(super) fn dispatch_message_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let lp_msg = uc.read_arg(0);
    let hwnd = uc.read_u32(lp_msg as u64);
    let msg = uc.read_u32(lp_msg as u64 + 4);
    let w_param = uc.read_u32(lp_msg as u64 + 8);
    let l_param = uc.read_u32(lp_msg as u64 + 12);
    let time = uc.read_u32(lp_msg as u64 + 16);

    if msg == 0x0113 && l_param != 0 {
        USER32::dispatch_to_timer_proc(uc, l_param, hwnd, w_param, time);
        return Some(ApiHookResult::callee(1, Some(0)));
    }

    let wnd_proc = {
        let ctx = uc.get_data();
        let win_event = ctx.win_event.lock().unwrap();
        win_event
            .windows
            .get(&hwnd)
            .map(|win| win.wnd_proc)
            .unwrap_or(0)
    };

    let ret = USER32::dispatch_to_wndproc(uc, wnd_proc, hwnd, msg, w_param, l_param);
    request_main_window_exit_fallback(uc.get_data(), hwnd, msg, w_param);

    Some(ApiHookResult::callee(1, Some(ret)))
}

// API: BOOL TranslateMessage(const MSG* lpMsg)
// 역할: 메시지를 번역
pub(super) fn translate_message(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let lp_msg = uc.read_arg(0);
    let hwnd = uc.read_u32(lp_msg as u64);
    let msg = uc.read_u32(lp_msg as u64 + 4);
    let vk = uc.read_u32(lp_msg as u64 + 8);
    let l_param = uc.read_u32(lp_msg as u64 + 12);

    // WM_KEYDOWN(0x0100) 또는 WM_SYSKEYDOWN(0x0104)인 경우에만 번역 시도
    if msg == 0x100 || msg == 0x104 {
        // 현재 호스트 키 입력은 logical_key 기반으로 들어오므로,
        // printable ASCII는 이미 shift/layout이 반영된 최종 문자로 취급합니다.
        if let Some(char_code) = translated_char_from_vk(vk) {
            let ctx = uc.get_data();
            // WM_CHAR(0x0102) 또는 WM_SYSCHAR(0x0106) 추가
            let char_msg = if msg == 0x0100 { 0x0102 } else { 0x0106 };
            ctx.queue_message_for_window(hwnd, [hwnd, char_msg, char_code, l_param, 0, 0, 0]);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        dll::win32::{Win32Context, WindowState},
        ui::UiCommand,
    };

    fn sample_window_state() -> WindowState {
        WindowState {
            class_name: "TEST".to_string(),
            class_icon: 0,
            big_icon: 0,
            small_icon: 0,
            class_hbr_background: 0,
            title: "test".to_string(),
            x: 0,
            y: 0,
            width: 640,
            height: 480,
            style: 0,
            ex_style: 0,
            owner_thread_id: 0,
            parent: 0,
            id: 0,
            visible: true,
            enabled: true,
            zoomed: false,
            iconic: false,
            wnd_proc: 0,
            class_cursor: 0,
            user_data: 0,
            use_native_frame: false,
            surface_bitmap: 0,
            window_rgn: 0,
            guest_frame_left: 0,
            guest_frame_top: 0,
            guest_frame_right: 0,
            guest_frame_bottom: 0,
            guest_frame_exact: false,
            needs_paint: false,
            last_hittest_lparam: u32::MAX,
            last_hittest_result: 0,
            z_order: 0,
        }
    }

    #[test]
    fn printable_lowercase_vk_is_translated_to_char() {
        assert_eq!(translated_char_from_vk('a' as u32), Some('a' as u32));
    }

    #[test]
    fn printable_symbol_vk_is_translated_to_char() {
        assert_eq!(translated_char_from_vk('!' as u32), Some('!' as u32));
    }

    #[test]
    fn main_guest_close_fallback_sends_exit_command() {
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = Win32Context::new(Some(tx));
        ctx.win_event
            .lock()
            .unwrap()
            .create_window(0x1000, sample_window_state());

        request_main_window_exit_fallback(&ctx, 0x1000, 0x0112, 0xF060);

        match rx.try_recv().expect("exit command") {
            UiCommand::ExitApplication => {}
            _ => panic!("expected ExitApplication"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn non_main_or_destroyed_close_fallback_does_not_send_exit_command() {
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = Win32Context::new(Some(tx));
        {
            let mut win_event = ctx.win_event.lock().unwrap();
            win_event.create_window(0x1000, sample_window_state());
            win_event.create_window(0x1001, sample_window_state());
            win_event.destroy_window(0x1000);
        }

        match rx.try_recv().expect("destroy command") {
            UiCommand::DestroyWindow { hwnd } => assert_eq!(hwnd, 0x1000),
            _ => panic!("expected DestroyWindow"),
        }

        request_main_window_exit_fallback(&ctx, 0x1001, 0x0112, 0xF060);
        request_main_window_exit_fallback(&ctx, 0x1000, 0x0010, 0);

        assert!(rx.try_recv().is_err());
    }
}

// API: BOOL PeekMessageA(LPMSG lpMsg, HWND hWnd, UINT wMsgFilterMin, UINT wMsgFilterMax, UINT wRemoveMsg)
// 역할: 메시지 큐에서 메시지를 가져옴
pub(super) fn peek_message_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let lp_msg = uc.read_arg(0);
    let hwnd_filter = uc.read_arg(1);
    let msg_min = uc.read_arg(2);
    let msg_max = uc.read_arg(3);
    let remove_flag = uc.read_arg(4);

    let mut cleared_paint_hwnd = 0u32;
    let msg = {
        let ctx = uc.get_data();
        let tid = ctx.current_queue_thread_id();

        // 1. 타이머 체크 및 WM_TIMER 생성
        {
            let mut timers = ctx.timers.lock().unwrap();
            let mut queues = ctx.message_queues.lock().unwrap();
            let queue = queues.entry(tid).or_default();
            USER32::enqueue_elapsed_timer_messages(
                &mut timers,
                queue,
                tid,
                std::time::Instant::now(),
            );
        }

        let mut queues = ctx.message_queues.lock().unwrap();
        let q = queues.entry(tid).or_default();

        let mut found_idx = None;
        for (i, m) in q.iter().enumerate() {
            let m_hwnd = m[0];
            let m_type = m[1];

            // HWND 필터링: filter가 0이면 모든 창, 아니면 특정 창만
            if hwnd_filter != 0 && m_hwnd != hwnd_filter {
                continue;
            }

            // 메시지 범위 필터링: min/max가 0이면 모든 메시지
            if (msg_min != 0 || msg_max != 0) && (m_type < msg_min || m_type > msg_max) {
                continue;
            }

            found_idx = Some(i);
            break;
        }

        if let Some(idx) = found_idx {
            if (remove_flag & 0x0001) != 0 && q[idx][1] == 0x000F {
                cleared_paint_hwnd = q[idx][0];
            }
            if (remove_flag & 0x0001) != 0 {
                q.remove(idx)
            } else {
                Some(q[idx])
            }
        } else {
            // 2. WM_PAINT 합성 (큐가 비어있는 경우)
            let _ = q; // 큐 가변 참조를 여기서 끝내고 이후 win_event를 읽습니다.
            let mut synthesized = None;
            {
                let ctx = uc.get_data();
                let mut win_event = ctx.win_event.lock().unwrap();
                for (hwnd, state) in win_event.windows.iter_mut() {
                    if state.owner_thread_id != tid {
                        continue;
                    }
                    if state.needs_paint {
                        if hwnd_filter != 0 && *hwnd != hwnd_filter {
                            continue;
                        }
                        if (msg_min != 0 || msg_max != 0) && (0x000F < msg_min || 0x000F > msg_max)
                        {
                            continue;
                        }
                        synthesized = Some([*hwnd, 0x000F, 0, 0, 0, 0, 0]);
                        if (remove_flag & 0x0001) != 0 {
                            state.needs_paint = false;
                            cleared_paint_hwnd = *hwnd;
                        }
                        break;
                    }
                }
            }
            synthesized
        }
    };

    if cleared_paint_hwnd != 0 {
        let ctx = uc.get_data();
        if let Some(state) = ctx
            .win_event
            .lock()
            .unwrap()
            .windows
            .get_mut(&cleared_paint_hwnd)
        {
            state.needs_paint = false;
        }
        crate::emu_log!(
            "[USER32] PeekMessageA cleared paint state for HWND {:#x}",
            cleared_paint_hwnd
        );
    }

    let (time, pt_x, pt_y) = {
        let ctx = uc.get_data();
        let time = crate::diagnostics::virtual_millis(ctx.start_time);
        let x = ctx.mouse_x.load(std::sync::atomic::Ordering::SeqCst);
        let y = ctx.mouse_y.load(std::sync::atomic::Ordering::SeqCst);
        (time, x, y)
    };

    let ret = if let Some(mut m) = msg {
        if m[1] >= 0x0200 && m[1] <= 0x0209 {
            let hwnd = m[0];
            let wnd_proc = {
                let ctx = uc.get_data();
                let win_event = ctx.win_event.lock().unwrap();
                win_event
                    .windows
                    .get(&hwnd)
                    .map(|w| w.wnd_proc)
                    .unwrap_or(0)
            };
            if wnd_proc != 0 {
                let (win_x, win_y, cached_lparam, cached_result) = {
                    let ctx = uc.get_data();
                    let win_event = ctx.win_event.lock().unwrap();
                    win_event
                        .windows
                        .get(&hwnd)
                        .map(|w| (w.x, w.y, w.last_hittest_lparam, w.last_hittest_result))
                        .unwrap_or((0, 0, u32::MAX, 0))
                };
                let screen_x = (m[5] as i32) + win_x;
                let screen_y = (m[6] as i32) + win_y;
                let screen_lparam = ((screen_y as u32) << 16) | ((screen_x as u32) & 0xFFFF);

                let hit_test = if screen_lparam == cached_lparam {
                    cached_result as i32
                } else {
                    let result =
                        USER32::dispatch_to_wndproc(uc, wnd_proc, hwnd, 0x0084, 0, screen_lparam);
                    let ctx = uc.get_data();
                    let mut win_event = ctx.win_event.lock().unwrap();
                    if let Some(w) = win_event.windows.get_mut(&hwnd) {
                        w.last_hittest_lparam = screen_lparam;
                        w.last_hittest_result = result as u32;
                    }
                    result
                };
                if hit_test != 1 && hit_test != 0 {
                    m[1] = m[1] - 0x0200 + 0x00A0;
                    m[2] = hit_test as u32;
                    m[3] = screen_lparam;
                }
            }
        }

        // MSG 구조체 채우기
        uc.write_u32(lp_msg as u64, m[0]); // hwnd
        uc.write_u32(lp_msg as u64 + 4, m[1]); // message
        uc.write_u32(lp_msg as u64 + 8, m[2]); // wParam
        uc.write_u32(lp_msg as u64 + 12, m[3]); // lParam
        uc.write_u32(lp_msg as u64 + 16, time); // time
        uc.write_u32(lp_msg as u64 + 20, m[5].max(pt_x)); // pt.x (큐 메시지 좌표 or 현재 좌표)
        uc.write_u32(lp_msg as u64 + 24, m[6].max(pt_y)); // pt.y
        1
    } else {
        // 메시지가 없을 때 이전 MSG 내용이 남아 있으면 호출자가 오래된 커스텀 메시지를
        // 다시 Dispatch하는 루프에 빠질 수 있으므로 구조체 전체를 비웁니다.
        clear_msg_struct(uc, lp_msg);
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
pub(super) fn get_message_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let lp_msg = uc.read_arg(0);
    let _hwnd_filter = uc.read_arg(1);
    let _min = uc.read_arg(2);
    let _max = uc.read_arg(3);

    let mut cleared_paint_hwnd = 0u32;
    let msg = {
        let ctx = uc.get_data(); // Immutable borrow of ctx
        let tid = ctx.current_queue_thread_id();
        {
            let mut timers = ctx.timers.lock().unwrap();
            let mut queues = ctx.message_queues.lock().unwrap();
            let queue = queues.entry(tid).or_default();
            USER32::enqueue_elapsed_timer_messages(
                &mut timers,
                queue,
                tid,
                std::time::Instant::now(),
            );
        }
        let mut queues = ctx.message_queues.lock().unwrap();
        let q = queues.entry(tid).or_default();

        if q.is_empty() {
            // Synthesize WM_PAINT if needed
            let mut paint_hwnd = 0;
            let win_event = ctx.win_event.lock().unwrap(); // Immutable borrow of win_event
            for (&h, win) in win_event.windows.iter() {
                if win.owner_thread_id == tid && win.needs_paint {
                    paint_hwnd = h;
                    break;
                }
            }
            if paint_hwnd != 0 {
                let time = crate::diagnostics::virtual_millis(ctx.start_time);
                q.push_back([paint_hwnd, 0x000F, 0, 0, time, 0, 0]);
            }
        }

        let popped = q.pop_front();
        if let Some(msg) = popped
            && msg[1] == 0x000F
        {
            cleared_paint_hwnd = msg[0];
        }
        popped
    };

    // 메시지가 없으면 retry를 반환하여 에뮬레이터 메인 루프에 양보합니다.
    // 메인 루프가 schedule_threads() 호출 및 idle sleep을 처리하므로
    // 여기서 직접 sleep/polling 할 필요가 없습니다.
    if msg.is_none() {
        clear_msg_struct(uc, lp_msg);
        // 메인 스레드가 idle 상태임을 표시하여 메인 루프의 idle sleep이 작동하도록 합니다.
        let ctx = uc.get_data();
        let tid = ctx.current_queue_thread_id();
        KERNEL32::set_wait_handles(ctx, tid, &[]);
        KERNEL32::set_wait_sockets(ctx, tid, &[]);
        KERNEL32::schedule_retry_wait(ctx, tid, USER32::next_timer_deadline(ctx, tid));
        return Some(ApiHookResult::retry());
    }

    if let Some(mut m) = msg {
        {
            let ctx = uc.get_data();
            let tid = ctx.current_queue_thread_id();
            KERNEL32::clear_retry_wait(ctx, tid);
        }
        if cleared_paint_hwnd != 0 {
            let ctx = uc.get_data();
            if let Some(state) = ctx
                .win_event
                .lock()
                .unwrap()
                .windows
                .get_mut(&cleared_paint_hwnd)
            {
                state.needs_paint = false;
            }
            crate::emu_log!(
                "[USER32] GetMessageA cleared paint state for HWND {:#x}",
                cleared_paint_hwnd
            );
        }

        if m[1] >= 0x0200 && m[1] <= 0x0209 {
            let hwnd = m[0];
            let wnd_proc = {
                let ctx = uc.get_data();
                let win_event = ctx.win_event.lock().unwrap();
                win_event
                    .windows
                    .get(&hwnd)
                    .map(|w| w.wnd_proc)
                    .unwrap_or(0)
            };
            if wnd_proc != 0 {
                let (win_x, win_y, cached_lparam, cached_result) = {
                    let ctx = uc.get_data();
                    let win_event = ctx.win_event.lock().unwrap();
                    win_event
                        .windows
                        .get(&hwnd)
                        .map(|w| (w.x, w.y, w.last_hittest_lparam, w.last_hittest_result))
                        .unwrap_or((0, 0, u32::MAX, 0))
                };
                let screen_x = (m[5] as i32) + win_x;
                let screen_y = (m[6] as i32) + win_y;
                let screen_lparam = ((screen_y as u32) << 16) | ((screen_x as u32) & 0xFFFF);

                let hit_test = if screen_lparam == cached_lparam {
                    cached_result as i32
                } else {
                    let result =
                        USER32::dispatch_to_wndproc(uc, wnd_proc, hwnd, 0x0084, 0, screen_lparam);
                    let ctx = uc.get_data();
                    let mut win_event = ctx.win_event.lock().unwrap();
                    if let Some(w) = win_event.windows.get_mut(&hwnd) {
                        w.last_hittest_lparam = screen_lparam;
                        w.last_hittest_result = result as u32;
                    }
                    result
                };
                if hit_test != 1 && hit_test != 0 {
                    m[1] = m[1] - 0x0200 + 0x00A0;
                    m[2] = hit_test as u32;
                    m[3] = screen_lparam;
                }
            }
        }

        for i in 0..7 {
            uc.write_u32(lp_msg as u64 + (i * 4) as u64, m[i as usize]);
        }
        let is_quit = m[1] == 0x0012;
        Some(ApiHookResult::callee(4, Some(if is_quit { 0 } else { 1 })))
    } else {
        // No message (Note: native GetMessage blocks, but for now we return WM_NULL)
        clear_msg_struct(uc, lp_msg);
        Some(ApiHookResult::callee(4, Some(1)))
    }
}

// API: DWORD MsgWaitForMultipleObjects(DWORD nCount, const HANDLE* pHandles, BOOL fWaitAll, DWORD dwMilliseconds, DWORD dwWakeMask)
// 역할: 하나 이상의 개체 또는 메시지가 큐에 도착할 때까지 대기
pub(super) fn msg_wait_for_multiple_objects(
    uc: &mut Unicorn<Win32Context>,
) -> Option<ApiHookResult> {
    let n_count = uc.read_arg(0);
    let p_handles = uc.read_arg(1);
    let _f_wait_all = uc.read_arg(2);
    let dw_milliseconds = uc.read_arg(3);
    let _dw_wake_mask = uc.read_arg(4);

    let ctx = uc.get_data();
    let tid = ctx
        .current_thread_idx
        .load(std::sync::atomic::Ordering::SeqCst);
    let handles: Vec<u32> = if n_count != 0 && p_handles != 0 {
        (0..n_count.min(64))
            .map(|index| uc.read_u32(p_handles as u64 + index as u64 * 4))
            .collect()
    } else {
        Vec::new()
    };

    if USER32::has_pending_ui_message(ctx, tid) {
        KERNEL32::clear_retry_wait(ctx, tid);
        // crate::emu_log!(
        //     "[USER32] MsgWaitForMultipleObjects({:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> message",
        //     n_count,
        //     p_handles,
        //     f_wait_all,
        //     dw_milliseconds,
        //     dw_wake_mask
        // );
        return Some(ApiHookResult::callee(5, Some(n_count as i32)));
    }

    if let Some(index) = KERNEL32::first_ready_wait_handle(ctx, &handles) {
        KERNEL32::clear_retry_wait(ctx, tid);
        // crate::emu_log!(
        //     "[USER32] MsgWaitForMultipleObjects({:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> WAIT_OBJECT_0+{}",
        //     n_count,
        //     p_handles,
        //     f_wait_all,
        //     dw_milliseconds,
        //     dw_wake_mask,
        //     index
        // );
        return Some(ApiHookResult::callee(5, Some(index as i32)));
    }

    if dw_milliseconds == 0 {
        KERNEL32::clear_retry_wait(ctx, tid);
        // crate::emu_log!(
        //     "[USER32] MsgWaitForMultipleObjects({:#x}, {:#x}, {:#x}, 0, {:#x}) -> WAIT_TIMEOUT",
        //     n_count,
        //     p_handles,
        //     f_wait_all,
        //     dw_wake_mask
        // );
        return Some(ApiHookResult::callee(5, Some(0x102)));
    }

    let now = std::time::Instant::now();
    let handle_deadline = if dw_milliseconds == 0xFFFF_FFFF {
        None
    } else {
        KERNEL32::current_wait_deadline(ctx, tid).or(Some(
            now + std::time::Duration::from_millis(dw_milliseconds as u64),
        ))
    };
    let deadline = earlier_deadline(handle_deadline, USER32::next_timer_deadline(ctx, tid));

    if let Some(limit) = deadline
        && now >= limit
    {
        KERNEL32::clear_retry_wait(ctx, tid);
        // crate::emu_log!(
        //     "[USER32] MsgWaitForMultipleObjects({:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> WAIT_TIMEOUT",
        //     n_count,
        //     p_handles,
        //     f_wait_all,
        //     dw_milliseconds,
        //     dw_wake_mask
        // );
        return Some(ApiHookResult::callee(5, Some(0x102)));
    }

    KERNEL32::set_wait_handles(ctx, tid, &handles);
    KERNEL32::set_wait_sockets(ctx, tid, &[]);
    KERNEL32::schedule_retry_wait(ctx, tid, deadline);
    Some(ApiHookResult::retry())
}

// API: void PostQuitMessage(int nExitCode)
// 역할: 프로그램 종료 메시지를 보냄
pub(super) fn post_quit_message(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let n_exit_code = uc.read_arg(0);
    let ctx = uc.get_data();
    let tid = ctx
        .current_thread_idx
        .load(std::sync::atomic::Ordering::SeqCst);
    let time = crate::diagnostics::virtual_millis(ctx.start_time);
    let target_tid = ctx.queue_message_for_thread(tid, [0, 0x0012, n_exit_code, 0, time, 0, 0]);
    ctx.wake_thread_message_wait(target_tid);
    crate::emu_log!("[USER32] PostQuitMessage({}) -> void", n_exit_code);
    Some(ApiHookResult::callee(1, None))
}

// API: BOOL PostThreadMessageA(DWORD idThread, UINT Msg, WPARAM wParam, LPARAM lParam)
// 역할: 스레드에 메시지를 보냄
pub(super) fn post_thread_message_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let thread_id = uc.read_arg(0);
    let msg = uc.read_arg(1);
    let w_param = uc.read_arg(2);
    let l_param = uc.read_arg(3);
    let time = crate::diagnostics::virtual_millis(uc.get_data().start_time);
    let ctx = uc.get_data();
    let target_tid =
        ctx.queue_message_for_thread(thread_id, [0, msg, w_param, l_param, time, 0, 0]);
    ctx.wake_thread_message_wait(target_tid);
    crate::emu_log!(
        "[USER32] PostThreadMessageA({:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 1",
        thread_id,
        msg,
        w_param,
        l_param
    );
    Some(ApiHookResult::callee(4, Some(1)))
}

// API: LRESULT CallWindowProcA(WNDPROC lpPrevWndFunc, HWND hWnd, UINT Msg, WPARAM wParam, LPARAM lParam)
// 역할: 윈도우 프로시저를 호출
pub(super) fn call_window_proc_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let lp_prev_wnd_func = uc.read_arg(0);
    let hwnd = uc.read_arg(1);
    let msg = uc.read_arg(2);
    let w_param = uc.read_arg(3);
    let l_param = uc.read_arg(4);

    let ret = USER32::dispatch_to_wndproc(uc, lp_prev_wnd_func, hwnd, msg, w_param, l_param);

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

// API: BOOL IsDialogMessageA(HWND hDlg, LPMSG lpMsg)
// 역할: 다이얼로그 메시지를 번역
pub(super) fn is_dialog_message_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let h_dlg = uc.read_arg(0);
    let lp_msg = uc.read_arg(1);
    crate::emu_log!(
        "[USER32] IsDialogMessageA({:#x}, {:#x}) -> BOOL 0",
        h_dlg,
        lp_msg
    );
    Some(ApiHookResult::callee(2, Some(0)))
}

// API: LRESULT DefWindowProcA(HWND hWnd, UINT Msg, WPARAM wParam, LPARAM lParam)
// 역할: 윈도우 프로시저를 호출
pub(super) fn def_window_proc_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let msg = uc.read_arg(1);
    let w_param = uc.read_arg(2);
    let l_param = uc.read_arg(3);
    let default_ret = match msg {
        0x0006 => {
            // WM_ACTIVATE
            let state = w_param & 0xFFFF;
            if state != 0 {
                let ctx = uc.get_data();
                ctx.active_hwnd
                    .store(hwnd, std::sync::atomic::Ordering::SeqCst);
                ctx.focus_hwnd
                    .store(hwnd, std::sync::atomic::Ordering::SeqCst);
                ctx.win_event.lock().unwrap().activate_window(hwnd);
            }
            0
        }
        0x0007 => {
            // WM_SETFOCUS
            uc.get_data()
                .focus_hwnd
                .store(hwnd, std::sync::atomic::Ordering::SeqCst);
            0
        }
        0x0008 => {
            // WM_KILLFOCUS
            let ctx = uc.get_data();
            if ctx.focus_hwnd.load(std::sync::atomic::Ordering::SeqCst) == hwnd {
                ctx.focus_hwnd.store(0, std::sync::atomic::Ordering::SeqCst);
            }
            0
        }
        0x000C => {
            // WM_SETTEXT
            let text = uc.read_euc_kr(l_param as u64);
            uc.get_data()
                .win_event
                .lock()
                .unwrap()
                .set_window_text(hwnd, text);
            1
        }
        0x000D => {
            // WM_GETTEXT
            let max_count = w_param;
            let buf_addr = l_param;
            let title_info = {
                let ctx = uc.get_data();
                let win_event = ctx.win_event.lock().unwrap();
                win_event.windows.get(&hwnd).map(|win| {
                    let (encoded, _, _) = EUC_KR.encode(&win.title);
                    let copy_len = encoded.len().min((max_count as usize).saturating_sub(1));
                    (encoded[..copy_len].to_vec(), copy_len)
                })
            };
            if let Some((bytes, len)) = title_info {
                USER32::write_ansi_bytes(uc, buf_addr as u64, &bytes);
                len as i32
            } else {
                0
            }
        }
        0x000F => {
            // WM_PAINT
            uc.get_data()
                .win_event
                .lock()
                .unwrap()
                .validate_window(hwnd);
            0
        }
        0x0010 => {
            // WM_CLOSE
            let ctx = uc.get_data();
            USER32::destroy_window_tree(ctx, hwnd);
            0
        }
        0x0082 => {
            // WM_NCDESTROY
            0
        }
        0x0011 => 1, // WM_QUERYENDSESSION
        0x0014 => {
            // WM_ERASEBKGND
            if super::paint::erase_window_background(uc, hwnd) {
                1
            } else {
                0
            }
        }
        0x0020 => {
            let ctx = uc.get_data();
            let class_cursor = {
                let win_event = ctx.win_event.lock().unwrap();
                win_event
                    .windows
                    .get(&hwnd)
                    .map(|win| win.class_cursor)
                    .unwrap_or(0)
            };

            if class_cursor != 0 {
                ctx.current_cursor
                    .store(class_cursor, std::sync::atomic::Ordering::SeqCst);
                ctx.win_event
                    .lock()
                    .unwrap()
                    .send_ui_command(crate::ui::UiCommand::SetCursor {
                        hwnd,
                        hcursor: class_cursor,
                    });
                1
            } else {
                0
            }
        }
        0x0024 => {
            // WM_GETMINMAXINFO
            if l_param != 0 {
                const PT_MAX_SIZE_OFFSET: u64 = 8;
                const PT_MAX_POSITION_OFFSET: u64 = 16;
                const PT_MIN_TRACK_SIZE_OFFSET: u64 = 24;
                const PT_MAX_TRACK_SIZE_OFFSET: u64 = 32;

                let (work_left, work_top, work_right, work_bottom) = uc.get_data().work_area_rect();
                let work_width = (work_right - work_left).max(1);
                let work_height = (work_bottom - work_top).max(1);
                let (min_track_width, min_track_height, max_track_width, max_track_height) = {
                    let ctx = uc.get_data();
                    let win_event = ctx.win_event.lock().unwrap();
                    win_event
                        .windows
                        .get(&hwnd)
                        .map(|win| {
                            let metrics = USER32::get_window_frame_metrics(win);
                            (
                                (metrics.left + metrics.right + 1).max(1),
                                (metrics.top + metrics.bottom + 1).max(1),
                                work_width.max(win.width.max(1)),
                                work_height.max(win.height.max(1)),
                            )
                        })
                        .unwrap_or((1, 1, work_width, work_height))
                };
                let info_ptr = l_param as u64;
                write_point(uc, info_ptr + PT_MAX_SIZE_OFFSET, work_width, work_height);
                write_point(uc, info_ptr + PT_MAX_POSITION_OFFSET, work_left, work_top);
                write_point(
                    uc,
                    info_ptr + PT_MIN_TRACK_SIZE_OFFSET,
                    min_track_width,
                    min_track_height,
                );
                write_point(
                    uc,
                    info_ptr + PT_MAX_TRACK_SIZE_OFFSET,
                    max_track_width,
                    max_track_height,
                );
            }
            0
        }
        0x0080 => {
            // WM_SETICON
            let old_icon = uc
                .get_data()
                .win_event
                .lock()
                .unwrap()
                .set_window_icon(hwnd, w_param, l_param);
            old_icon as i32
        }
        0x0081 => {
            // WM_NCCREATE
            // 생성 승인/거부는 `CreateWindowExA`가 guest wndproc에 직접 `WM_NCCREATE`를 보내
            // 이미 판정했으므로, 기본 프로시저는 중복 상태 변경 없이 성공만 반환합니다.
            1
        }
        0x0084 => {
            // WM_NCHITTEST
            let screen_x = (l_param & 0xFFFF) as i16 as i32;
            let screen_y = ((l_param >> 16) & 0xFFFF) as i16 as i32;

            {
                let ctx = uc.get_data();
                let win_event = ctx.win_event.lock().unwrap();
                win_event
                    .windows
                    .get(&hwnd)
                    .map(|w| USER32::default_hit_test(w, screen_x, screen_y))
                    .unwrap_or(0)
            }
        }
        0x0083 => {
            // WM_NCCALCSIZE
            if w_param != 0 {
                let rect_ptr = l_param as u64;
                let metrics = {
                    let ctx = uc.get_data();
                    let win_event = ctx.win_event.lock().unwrap();
                    if let Some(win) = win_event.windows.get(&hwnd) {
                        USER32::get_window_frame_metrics(win)
                    } else {
                        crate::dll::win32::user32::WindowFrameMetrics::default()
                    }
                };

                if metrics.left > 0 || metrics.top > 0 || metrics.right > 0 || metrics.bottom > 0 {
                    let left = uc.read_u32(rect_ptr) as i32;
                    let top = uc.read_u32(rect_ptr + 4) as i32;
                    let right = uc.read_u32(rect_ptr + 8) as i32;
                    let bottom = uc.read_u32(rect_ptr + 12) as i32;

                    uc.write_u32(rect_ptr, (left + metrics.left) as u32);
                    uc.write_u32(rect_ptr + 4, (top + metrics.top) as u32);
                    uc.write_u32(rect_ptr + 8, (right - metrics.right) as u32);
                    uc.write_u32(rect_ptr + 12, (bottom - metrics.bottom) as u32);
                }
            }
            0
        }
        0x0085 => {
            // WM_NCPAINT
            super::nc_paint::draw_window_frame(uc, hwnd);
            0
        }
        0x0086 => {
            // WM_NCACTIVATE
            super::nc_paint::draw_window_frame(uc, hwnd);
            1
        }
        0x00A1 => {
            // WM_NCLBUTTONDOWN
            if w_param == 2 {
                // HTCAPTION
                uc.get_data().win_event.lock().unwrap().drag_window(hwnd);
            }
            0
        }
        0x00A2 => {
            // WM_NCLBUTTONUP
            let ctx = uc.get_data();
            let time = crate::diagnostics::virtual_millis(ctx.start_time);
            match w_param {
                20 => {
                    // HTCLOSE
                    ctx.queue_message_for_window(hwnd, [hwnd, 0x0112, 0xF060, l_param, time, 0, 0]); // WM_SYSCOMMAND, SC_CLOSE
                }
                8 => {
                    // HTMINBUTTON
                    ctx.queue_message_for_window(hwnd, [hwnd, 0x0112, 0xF020, l_param, time, 0, 0]); // WM_SYSCOMMAND, SC_MINIMIZE
                }
                9 => {
                    // HTMAXBUTTON
                    let win_event = ctx.win_event.lock().unwrap();
                    let is_zoomed = win_event
                        .windows
                        .get(&hwnd)
                        .map(|w| w.zoomed)
                        .unwrap_or(false);
                    let cmd = if is_zoomed { 0xF120 } else { 0xF030 }; // SC_RESTORE or SC_MAXIMIZE
                    drop(win_event);
                    ctx.queue_message_for_window(hwnd, [hwnd, 0x0112, cmd, l_param, time, 0, 0]);
                }
                _ => {}
            }
            0
        }
        0x0112 => {
            // WM_SYSCOMMAND
            let cmd = w_param & 0xFFF0;
            match cmd {
                0xF060 => {
                    // SC_CLOSE
                    let ctx = uc.get_data();
                    let time = crate::diagnostics::virtual_millis(ctx.start_time);
                    ctx.queue_message_for_window(hwnd, [hwnd, 0x0010, 0, 0, time, 0, 0]); // WM_CLOSE
                }
                0xF020 => {
                    // SC_MINIMIZE
                    uc.get_data()
                        .win_event
                        .lock()
                        .unwrap()
                        .minimize_window(hwnd);
                }
                0xF030 => {
                    // SC_MAXIMIZE
                    uc.get_data()
                        .win_event
                        .lock()
                        .unwrap()
                        .maximize_window(hwnd);
                }
                0xF120 => {
                    // SC_RESTORE
                    uc.get_data().win_event.lock().unwrap().restore_window(hwnd);
                }
                _ => {}
            }
            0
        }
        0x0600 => {
            if w_param == 0x50000 {
                let id = (l_param & 0xFFFF) as u16;
                match id {
                    // 0x9090 => {
                    //     // MINIMIZE
                    //     uc.get_data()
                    //         .win_event
                    //         .lock()
                    //         .unwrap()
                    //         .minimize_window(hwnd);
                    // }
                    0xc2c8 | 0x2ddc => {
                        // CLOSE
                        let ctx = uc.get_data();
                        USER32::destroy_window_tree(ctx, hwnd);
                    }
                    _ => {}
                }
            }
            0
        }
        _ => 0,
    };
    // crate::emu_log!(
    //     "[USER32] DefWindowProcA({:#x}, {:#x}, {:#x}, {:#x}) -> LRESULT {}",
    //     hwnd,
    //     msg,
    //     _w_param,
    //     _l_param,
    //     default_ret
    // );
    Some(ApiHookResult::callee(4, Some(default_ret)))
}

// API: LRESULT DefMDIChildProcA(HWND hWnd, UINT Msg, WPARAM wParam, LPARAM lParam)
// 역할: MDI 자식 윈도우 프로시저를 호출
pub(super) fn def_mdi_child_proc_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let msg = uc.read_arg(1);
    let w_param = uc.read_arg(2);
    let l_param = uc.read_arg(3);

    let default_ret = match msg {
        0x0081 => 1, // WM_NCCREATE
        _ => {
            if matches!(msg, 0x0083 | 0x0085 | 0x0086 | 0x00A1 | 0x00A2 | 0x0112) {
                return def_window_proc_a(uc);
            }
            0
        }
    };

    crate::emu_log!(
        "[USER32] DefMDIChildProcA({:#x}, {:#x}, {:#x}, {:#x}) -> LRESULT {}",
        hwnd,
        msg,
        w_param,
        l_param,
        default_ret
    );
    Some(ApiHookResult::callee(4, Some(default_ret)))
}

// API: LRESULT DefFrameProcA(HWND hWnd, HWND hWndMDIClient, UINT Msg, WPARAM wParam, LPARAM lParam)
// 역할: 프레임 윈도우 프로시저를 호출
pub(super) fn def_frame_proc_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let mdi_client = uc.read_arg(1);
    let msg = uc.read_arg(2);
    let w_param = uc.read_arg(3);
    let l_param = uc.read_arg(4);
    let default_ret = if msg == 0x0081 { 1 } else { 0 };
    crate::emu_log!(
        "[USER32] DefFrameProcA({:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> LRESULT {}",
        hwnd,
        mdi_client,
        msg,
        w_param,
        l_param,
        default_ret
    );
    Some(ApiHookResult::callee(5, Some(default_ret)))
}
