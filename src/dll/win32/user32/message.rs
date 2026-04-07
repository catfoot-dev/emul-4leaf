use crate::{
    dll::win32::{ApiHookResult, Win32Context, kernel32::KERNEL32},
    helper::UnicornHelper,
};
use encoding_rs::EUC_KR;
use std::time::Instant;
use unicorn_engine::Unicorn;

use super::USER32;

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

// API: LRESULT DispatchMessageA(const MSG* lpMsg)
// 역할: 메시지를 디스패치
pub(super) fn dispatch_message_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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

    let ret = USER32::dispatch_to_wndproc(uc, wnd_proc, hwnd, msg, w_param, l_param);

    Some(ApiHookResult::callee(1, Some(ret)))
}

// API: BOOL TranslateMessage(const MSG* lpMsg)
// 역할: 메시지를 번역
pub(super) fn translate_message(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn peek_message_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let lp_msg = uc.read_arg(0);
    let hwnd_filter = uc.read_arg(1);
    let msg_min = uc.read_arg(2);
    let msg_max = uc.read_arg(3);
    let remove_flag = uc.read_arg(4);

    // 타 스레드 스케줄링 (협력적 멀티태스킹 유도)
    KERNEL32::schedule_threads(uc);

    let msg = {
        let ctx = uc.get_data();

        // 1. 타이머 체크 및 WM_TIMER 생성
        {
            let mut timers = ctx.timers.lock().unwrap();
            let mut q = ctx.message_queue.lock().unwrap();
            USER32::enqueue_elapsed_timer_messages(&mut timers, &mut q, std::time::Instant::now());
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
                        if (msg_min != 0 || msg_max != 0) && (0x000F < msg_min || 0x000F > msg_max)
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
pub(super) fn get_message_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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

    // 메시지가 없으면 retry를 반환하여 에뮬레이터 메인 루프에 양보합니다.
    // 메인 루프가 schedule_threads() 호출 및 idle sleep을 처리하므로
    // 여기서 직접 sleep/polling 할 필요가 없습니다.
    if msg.is_none() {
        // 메인 스레드가 idle 상태임을 표시하여 메인 루프의 idle sleep이 작동하도록 합니다.
        let ctx = uc.get_data();
        let resume = Instant::now() + std::time::Duration::from_millis(1);
        *ctx.main_resume_time.lock().unwrap() = Some(resume);
        return Some(ApiHookResult::retry());
    }

    if let Some(mut m) = msg {
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
        uc.write_u32(lp_msg as u64 + 4, 0); // message = 0 (WM_NULL)
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
    let f_wait_all = uc.read_arg(2);
    let dw_milliseconds = uc.read_arg(3);
    let dw_wake_mask = uc.read_arg(4);

    // 타 스레드 스케줄링
    KERNEL32::schedule_threads(uc);

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

    if USER32::has_pending_ui_message(ctx) {
        KERNEL32::clear_retry_wait(ctx, tid);
        crate::emu_log!(
            "[USER32] MsgWaitForMultipleObjects({:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> message",
            n_count,
            p_handles,
            f_wait_all,
            dw_milliseconds,
            dw_wake_mask
        );
        return Some(ApiHookResult::callee(5, Some(n_count as i32)));
    }

    if let Some(index) = KERNEL32::first_ready_wait_handle(ctx, &handles) {
        KERNEL32::clear_retry_wait(ctx, tid);
        crate::emu_log!(
            "[USER32] MsgWaitForMultipleObjects({:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> WAIT_OBJECT_0+{}",
            n_count,
            p_handles,
            f_wait_all,
            dw_milliseconds,
            dw_wake_mask,
            index
        );
        return Some(ApiHookResult::callee(5, Some(index as i32)));
    }

    if dw_milliseconds == 0 {
        KERNEL32::clear_retry_wait(ctx, tid);
        crate::emu_log!(
            "[USER32] MsgWaitForMultipleObjects({:#x}, {:#x}, {:#x}, 0, {:#x}) -> WAIT_TIMEOUT",
            n_count,
            p_handles,
            f_wait_all,
            dw_wake_mask
        );
        return Some(ApiHookResult::callee(5, Some(0x102)));
    }

    let now = std::time::Instant::now();
    let deadline = if dw_milliseconds == 0xFFFF_FFFF {
        None
    } else {
        KERNEL32::current_wait_deadline(ctx, tid).or(Some(
            now + std::time::Duration::from_millis(dw_milliseconds as u64),
        ))
    };

    if let Some(limit) = deadline
        && now >= limit
    {
        KERNEL32::clear_retry_wait(ctx, tid);
        crate::emu_log!(
            "[USER32] MsgWaitForMultipleObjects({:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> WAIT_TIMEOUT",
            n_count,
            p_handles,
            f_wait_all,
            dw_milliseconds,
            dw_wake_mask
        );
        return Some(ApiHookResult::callee(5, Some(0x102)));
    }

    KERNEL32::schedule_retry_wait(ctx, tid, deadline);
    Some(ApiHookResult::retry())
}

// API: void PostQuitMessage(int nExitCode)
// 역할: 프로그램 종료 메시지를 보냄
pub(super) fn post_quit_message(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let n_exit_code = uc.read_arg(0);
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
    let _w_param = uc.read_arg(2);
    let _l_param = uc.read_arg(3);
    let default_ret = match msg {
        0x0081 => 1, // WM_NCCREATE
        0x00A1 => {
            // WM_NCLBUTTONDOWN
            if _w_param == 2 {
                // HTCAPTION
                uc.get_data().win_event.lock().unwrap().drag_window(hwnd);
            }
            0
        }
        0x00A2 => {
            // WM_NCLBUTTONUP
            match _w_param {
                20 => {
                    // HTCLOSE
                    let ctx = uc.get_data();
                    let time = ctx.start_time.elapsed().as_millis() as u32;
                    let mut q = ctx.message_queue.lock().unwrap();
                    q.push_back([hwnd, 0x0112, 0xF060, _l_param, time, 0, 0]); // WM_SYSCOMMAND, SC_CLOSE
                }
                8 => {
                    // HTMINBUTTON
                    let ctx = uc.get_data();
                    let time = ctx.start_time.elapsed().as_millis() as u32;
                    let mut q = ctx.message_queue.lock().unwrap();
                    q.push_back([hwnd, 0x0112, 0xF020, _l_param, time, 0, 0]); // WM_SYSCOMMAND, SC_MINIMIZE
                }
                9 => {
                    // HTMAXBUTTON
                    let ctx = uc.get_data();
                    let time = ctx.start_time.elapsed().as_millis() as u32;
                    let mut q = ctx.message_queue.lock().unwrap();
                    q.push_back([hwnd, 0x0112, 0xF030, _l_param, time, 0, 0]); // WM_SYSCOMMAND, SC_MAXIMIZE
                }
                _ => {}
            }
            0
        }
        0x0112 => {
            // WM_SYSCOMMAND
            let cmd = _w_param & 0xFFF0;
            match cmd {
                0xF060 => {
                    // SC_CLOSE
                    let ctx = uc.get_data();
                    let time = ctx.start_time.elapsed().as_millis() as u32;
                    let mut q = ctx.message_queue.lock().unwrap();
                    q.push_back([hwnd, 0x0010, 0, 0, time, 0, 0]); // WM_CLOSE
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
                    let ctx = uc.get_data();
                    let mut win_event = ctx.win_event.lock().unwrap();
                    let is_zoomed = win_event
                        .windows
                        .get(&hwnd)
                        .map(|w| w.zoomed)
                        .unwrap_or(false);
                    if is_zoomed {
                        win_event.restore_window(hwnd);
                    } else {
                        win_event.maximize_window(hwnd);
                    }
                }
                0xF120 => {
                    // SC_RESTORE
                    uc.get_data().win_event.lock().unwrap().restore_window(hwnd);
                }
                _ => {}
            }
            0
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
        _ => 0,
    };
    // crate::emu_log!(
    //     "[USER32] DefWindowProcA({:#x}, {:#x}, {:#x}, {:#x}) -> LRESULT {}",
    //     hwnd,
    //     msg,
    //     w_param,
    //     l_param,
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
    let default_ret = if msg == 0x0081 { 1 } else { 0 };
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
