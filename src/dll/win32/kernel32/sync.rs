use crate::{
    dll::win32::{ApiHookResult, EventState, Win32Context},
    helper::UnicornHelper,
};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use unicorn_engine::Unicorn;

use super::KERNEL32;

// =========================================================
// Event / Sync
// =========================================================
// API: HANDLE CreateEventA(LPSECURITY_ATTRIBUTES lpEventAttributes, BOOL bManualReset, BOOL bInitialState, LPCSTR lpName)
// 역할: 이벤트 개체를 생성하거나 오픈
pub(super) fn create_event_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let lp_event_attributes = uc.read_arg(0);
    let manual_reset = uc.read_arg(1);
    let initial_state = uc.read_arg(2);
    let lp_name = uc.read_arg(3);
    let name = if lp_name != 0 {
        uc.read_euc_kr(lp_name as u64)
    } else {
        String::new()
    };
    let ctx = uc.get_data();
    let handle = ctx.alloc_handle();
    ctx.events.lock().unwrap().insert(
        handle,
        EventState {
            signaled: initial_state != 0,
            manual_reset: manual_reset != 0,
        },
    );
    crate::emu_log!(
        "[KERNEL32] CreateEventA({:#x}, {}, {}, {:#x}=\"{}\") -> HANDLE {:#x}",
        lp_event_attributes,
        manual_reset,
        initial_state,
        lp_name,
        name,
        handle
    );
    Some(ApiHookResult::callee(4, Some(handle as i32)))
}

// API: BOOL SetEvent(HANDLE hEvent)
// 역할: 지정된 이벤트 개체를 신호(signaled) 상태로 설정
pub(super) fn set_event(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let handle = uc.read_arg(0);
    let ctx = uc.get_data();
    let mut events = ctx.events.lock().unwrap();
    if let Some(evt) = events.get_mut(&handle) {
        evt.signaled = true;
    }
    crate::emu_log!("[KERNEL32] SetEvent({:#x}) -> BOOL 1", handle);
    Some(ApiHookResult::callee(1, Some(1)))
}

// API: BOOL PulseEvent(HANDLE hEvent)
// 역할: 이벤트 개체의 상태를 signaled로 설정한 후 다시 nonsignaled 상태로 재설정
pub(super) fn pulse_event(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let handle = uc.read_arg(0);
    crate::emu_log!("[KERNEL32] PulseEvent({:#x}) -> BOOL 1", handle);
    Some(ApiHookResult::callee(1, Some(1)))
}

// API: BOOL ResetEvent(HANDLE hEvent)
// 역할: 지정된 이벤트 개체를 비신호(nonsignaled) 상태로 설정
pub(super) fn reset_event(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let handle = uc.read_arg(0);
    let ctx = uc.get_data();
    let mut events = ctx.events.lock().unwrap();
    if let Some(evt) = events.get_mut(&handle) {
        evt.signaled = false;
    }
    crate::emu_log!("[KERNEL32] ResetEvent({:#x}) -> BOOL 1", handle);
    Some(ApiHookResult::callee(1, Some(1)))
}

// Critical Section (싱글 스레드이므로 no-op)
// API: void InitializeCriticalSection(LPCRITICAL_SECTION lpCriticalSection)
// 역할: 크리티컬 섹션 개체를 초기화
pub(super) fn initialize_critical_section(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let lp_critical_section = uc.read_arg(0);
    crate::emu_log!(
        "[KERNEL32] InitializeCriticalSection({:#x}) -> VOID",
        lp_critical_section
    );
    Some(ApiHookResult::callee(1, None))
}

// API: void DeleteCriticalSection(LPCRITICAL_SECTION lpCriticalSection)
// 역할: 가상 크리티컬 섹션 객체를 삭제
pub(super) fn delete_critical_section(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let lp_critical_section = uc.read_arg(0);
    crate::emu_log!(
        "[KERNEL32] DeleteCriticalSection({:#x}) -> VOID",
        lp_critical_section
    );
    Some(ApiHookResult::callee(1, None))
}

// API: void EnterCriticalSection(LPCRITICAL_SECTION lpCriticalSection)
// 역할: 크리티컬 섹션에 진입
pub(super) fn enter_critical_section(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let lp_critical_section = uc.read_arg(0);
    crate::emu_log!(
        "[KERNEL32] EnterCriticalSection({:#x}) -> VOID",
        lp_critical_section
    );
    Some(ApiHookResult::callee(1, None))
}

// API: void LeaveCriticalSection(LPCRITICAL_SECTION lpCriticalSection)
// 역할: 크리티컬 섹션을 떠남
pub(super) fn leave_critical_section(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let lp_critical_section = uc.read_arg(0);
    crate::emu_log!(
        "[KERNEL32] LeaveCriticalSection({:#x}) -> VOID",
        lp_critical_section
    );
    Some(ApiHookResult::callee(1, None))
}

// Mutex
// API: HANDLE CreateMutexA(LPSECURITY_ATTRIBUTES lpMutexAttributes, BOOL bInitialOwner, LPCSTR lpName)
// 역할: 뮤텍스 개체를 생성하거나 오픈
pub(super) fn create_mutex_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let lp_mutex_attributes = uc.read_arg(0);
    let b_initial_owner = uc.read_arg(1);
    let lp_name = uc.read_arg(2);
    let name = if lp_name != 0 {
        uc.read_euc_kr(lp_name as u64)
    } else {
        String::new()
    };
    let ctx = uc.get_data();
    let handle = ctx.alloc_handle();
    crate::emu_log!(
        "[KERNEL32] CreateMutexA({:#x}, {}, {:#x}=\"{}\") -> HANDLE {:#x}",
        lp_mutex_attributes,
        b_initial_owner,
        lp_name,
        name,
        handle
    );
    Some(ApiHookResult::callee(3, Some(handle as i32)))
}

// API: BOOL ReleaseMutex(HANDLE hMutex)
// 역할: 단일 뮤텍스 객체의 소유권을 해제
pub(super) fn release_mutex(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let h_mutex = uc.read_arg(0);
    crate::emu_log!("[KERNEL32] ReleaseMutex({:#x}) -> BOOL 1", h_mutex);
    Some(ApiHookResult::callee(1, Some(1)))
}

// API: DWORD WaitForSingleObject(HANDLE hHandle, DWORD dwMilliseconds)
// 역할: 지정된 객체가 신호 상태가 되거나 시간제한이 초과될 때까지 대기.
//       WSA 이벤트 핸들인 경우 소켓 readable 상태를 실제로 대기합니다.
pub(super) fn wait_for_single_object(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let h_handle = uc.read_arg(0);
    let dw_milliseconds = uc.read_arg(1);
    let ctx = uc.get_data();
    let tid = ctx.current_thread_idx.load(Ordering::SeqCst);
    let now = Instant::now();

    if try_consume_signaled_event(ctx, h_handle) {
        KERNEL32::clear_retry_wait(ctx, tid);
        crate::emu_log!(
            "[KERNEL32] WaitForSingleObject({:#x}, {}) -> WAIT_OBJECT_0 (event)",
            h_handle,
            dw_milliseconds
        );
        return Some(ApiHookResult::callee(2, Some(0)));
    }

    // WSA 이벤트 핸들인지 확인
    let wsa_entry = ctx.wsa_event_map.lock().unwrap().get(&h_handle).cloned();
    if let Some(entry) = wsa_entry {
        let sock = entry.socket;

        // 이미 pending 이벤트가 있으면 즉시 반환
        if entry.pending != 0 {
            crate::emu_log!(
                "[KERNEL32] WaitForSingleObject(WSA event={:#x}) -> WAIT_OBJECT_0 (pending={:#x})",
                h_handle,
                entry.pending
            );
            KERNEL32::clear_retry_wait(ctx, tid);
            return Some(ApiHookResult::callee(2, Some(0))); // WAIT_OBJECT_0
        }

        // recv_buf에 이미 데이터가 있는지 확인
        let has_data = {
            let sockets = ctx.tcp_sockets.lock().unwrap();
            sockets
                .get(&sock)
                .map(|s| !s.recv_buf.is_empty())
                .unwrap_or(false)
        };
        if has_data {
            ctx.wsa_event_map
                .lock()
                .unwrap()
                .entry(h_handle)
                .and_modify(|e| e.pending |= 0x01); // FD_READ
            crate::emu_log!(
                "[KERNEL32] WaitForSingleObject(WSA event={:#x}) -> WAIT_OBJECT_0 (buf data)",
                h_handle
            );
            KERNEL32::clear_retry_wait(ctx, tid);
            return Some(ApiHookResult::callee(2, Some(0))); // WAIT_OBJECT_0
        }

        // 채널에서 데이터 확인 (non-blocking)
        let data_from_chan = {
            let mut sockets = ctx.tcp_sockets.lock().unwrap();
            if let Some(s) = sockets.get_mut(&sock) {
                if let Some(chan_rx) = s.chan_rx.as_mut() {
                    match chan_rx.try_recv() {
                        Ok(data) => {
                            s.recv_buf.extend(data);
                            true
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => true, // EOF
                        Err(std::sync::mpsc::TryRecvError::Empty) => false,
                    }
                } else {
                    false
                }
            } else {
                false
            }
        };
        if data_from_chan {
            ctx.wsa_event_map
                .lock()
                .unwrap()
                .entry(h_handle)
                .and_modify(|e| e.pending |= 0x01); // FD_READ
            crate::emu_log!(
                "[KERNEL32] WaitForSingleObject(WSA event={:#x}) -> WAIT_OBJECT_0 (channel data)",
                h_handle
            );
            KERNEL32::clear_retry_wait(ctx, tid);
            return Some(ApiHookResult::callee(2, Some(0))); // WAIT_OBJECT_0
        }

        if dw_milliseconds == 0 {
            KERNEL32::clear_retry_wait(ctx, tid);
            crate::emu_log!(
                "[KERNEL32] WaitForSingleObject(WSA event={:#x}) -> WAIT_TIMEOUT (poll)",
                h_handle
            );
            return Some(ApiHookResult::callee(2, Some(0x102)));
        }

        let deadline = if dw_milliseconds == 0xFFFF_FFFF {
            None
        } else {
            KERNEL32::current_wait_deadline(ctx, tid)
                .or(Some(now + Duration::from_millis(dw_milliseconds as u64)))
        };

        if let Some(limit) = deadline
            && now >= limit
        {
            KERNEL32::clear_retry_wait(ctx, tid);
            crate::emu_log!(
                "[KERNEL32] WaitForSingleObject(WSA event={:#x}) -> WAIT_TIMEOUT",
                h_handle
            );
            return Some(ApiHookResult::callee(2, Some(0x102)));
        }

        KERNEL32::schedule_retry_wait(ctx, tid, deadline);
        return Some(ApiHookResult::retry());
    }

    if dw_milliseconds == 0 {
        KERNEL32::clear_retry_wait(ctx, tid);
        crate::emu_log!(
            "[KERNEL32] WaitForSingleObject({:#x}, 0) -> WAIT_TIMEOUT",
            h_handle
        );
        return Some(ApiHookResult::callee(2, Some(0x102)));
    }

    let deadline = if dw_milliseconds == 0xFFFF_FFFF {
        None
    } else {
        KERNEL32::current_wait_deadline(ctx, tid)
            .or(Some(now + Duration::from_millis(dw_milliseconds as u64)))
    };

    if let Some(limit) = deadline
        && now >= limit
    {
        KERNEL32::clear_retry_wait(ctx, tid);
        crate::emu_log!(
            "[KERNEL32] WaitForSingleObject({:#x}, {}) -> WAIT_TIMEOUT",
            h_handle,
            dw_milliseconds
        );
        return Some(ApiHookResult::callee(2, Some(0x102)));
    }

    KERNEL32::schedule_retry_wait(ctx, tid, deadline);
    Some(ApiHookResult::retry())
}

// API: DWORD WaitForMultipleObjects(DWORD nCount, const HANDLE *lpHandles, BOOL bWaitAll, DWORD dwMilliseconds)
// 역할: 하나 또는 모든 지정된 개체가 신호 상태가 될 때까지 대기.
//       핸들 목록 중 WSA 이벤트가 있으면 소켓 readable을 실제로 대기합니다.
pub(super) fn wait_for_multiple_objects(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let n_count = uc.read_arg(0);
    let lp_handles = uc.read_arg(1);
    let _b_wait_all = uc.read_arg(2);
    let dw_milliseconds = uc.read_arg(3);
    let ctx = uc.get_data();
    let tid = ctx.current_thread_idx.load(Ordering::SeqCst);
    let now = Instant::now();

    // 핸들 목록을 순회하며 첫 번째 WSA 이벤트 핸들을 찾아 실제 대기
    for i in 0..n_count.min(64) {
        let handle = uc.read_u32(lp_handles as u64 + i as u64 * 4);

        if try_consume_signaled_event(ctx, handle) {
            KERNEL32::clear_retry_wait(ctx, tid);
            crate::emu_log!(
                "[KERNEL32] WaitForMultipleObjects: event={:#x} -> WAIT_OBJECT_0+{}",
                handle,
                i
            );
            return Some(ApiHookResult::callee(4, Some(i as i32)));
        }

        let wsa_entry = ctx.wsa_event_map.lock().unwrap().get(&handle).cloned();
        if let Some(entry) = wsa_entry {
            let sock = entry.socket;

            // 이미 pending 이벤트가 있으면 즉시 반환
            if entry.pending != 0 {
                crate::emu_log!(
                    "[KERNEL32] WaitForMultipleObjects: WSA event={:#x} has pending={:#x} -> WAIT_OBJECT_0+{}",
                    handle,
                    entry.pending,
                    i
                );
                KERNEL32::clear_retry_wait(ctx, tid);
                return Some(ApiHookResult::callee(4, Some(i as i32))); // WAIT_OBJECT_0 + i
            }

            // recv_buf 데이터 확인
            let has_data = {
                let sockets = ctx.tcp_sockets.lock().unwrap();
                sockets
                    .get(&sock)
                    .map(|s| !s.recv_buf.is_empty())
                    .unwrap_or(false)
            };
            if has_data {
                ctx.wsa_event_map
                    .lock()
                    .unwrap()
                    .entry(handle)
                    .and_modify(|e| e.pending |= 0x01);
                KERNEL32::clear_retry_wait(ctx, tid);
                return Some(ApiHookResult::callee(4, Some(i as i32)));
            }

            // 채널에서 데이터 확인 (non-blocking)
            let data_from_chan = {
                let mut sockets = ctx.tcp_sockets.lock().unwrap();
                if let Some(s) = sockets.get_mut(&sock) {
                    if let Some(chan_rx) = s.chan_rx.as_mut() {
                        match chan_rx.try_recv() {
                            Ok(data) => {
                                s.recv_buf.extend(data);
                                true
                            }
                            Err(std::sync::mpsc::TryRecvError::Disconnected) => true, // EOF
                            Err(std::sync::mpsc::TryRecvError::Empty) => false,
                        }
                    } else {
                        false
                    }
                } else {
                    false
                }
            };
            if data_from_chan {
                ctx.wsa_event_map
                    .lock()
                    .unwrap()
                    .entry(handle)
                    .and_modify(|e| e.pending |= 0x01);
                crate::emu_log!(
                    "[KERNEL32] WaitForMultipleObjects: WSA event={:#x} channel data -> WAIT_OBJECT_0+{}",
                    handle,
                    i
                );
                KERNEL32::clear_retry_wait(ctx, tid);
                return Some(ApiHookResult::callee(4, Some(i as i32)));
            }
            // 채널 없음 또는 데이터 없음 → 다음 핸들 시도
        }
    }

    if dw_milliseconds == 0 {
        KERNEL32::clear_retry_wait(ctx, tid);
        crate::emu_log!(
            "[KERNEL32] WaitForMultipleObjects({}) -> WAIT_TIMEOUT",
            n_count
        );
        return Some(ApiHookResult::callee(4, Some(0x102)));
    }

    let deadline = if dw_milliseconds == 0xFFFF_FFFF {
        None
    } else {
        KERNEL32::current_wait_deadline(ctx, tid)
            .or(Some(now + Duration::from_millis(dw_milliseconds as u64)))
    };

    if let Some(limit) = deadline
        && now >= limit
    {
        KERNEL32::clear_retry_wait(ctx, tid);
        crate::emu_log!(
            "[KERNEL32] WaitForMultipleObjects({}) -> WAIT_TIMEOUT",
            n_count
        );
        return Some(ApiHookResult::callee(4, Some(0x102)));
    }

    KERNEL32::schedule_retry_wait(ctx, tid, deadline);
    crate::emu_log!("[KERNEL32] WaitForMultipleObjects({}) -> retry", n_count);
    Some(ApiHookResult::retry())
}

// =========================================================
// Interlocked
// =========================================================
// API: LONG InterlockedExchange(LONG volatile *Target, LONG Value)
// 역할: 원자적 조작을 통해 두 개의 32비트 값을 교환
pub(super) fn interlocked_exchange(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let target_addr = uc.read_arg(0);
    let new_value = uc.read_arg(1);
    let old_value = uc.read_u32(target_addr as u64);
    uc.write_u32(target_addr as u64, new_value);
    crate::emu_log!(
        "[KERNEL32] InterlockedExchange({:#x}, {}) -> LONG {}",
        target_addr,
        new_value,
        old_value
    );
    Some(ApiHookResult::callee(2, Some(old_value as i32)))
}

/// 이벤트 핸들이 신호 상태인지 확인하고 자동 리셋 이벤트면 소비합니다.
pub(super) fn try_consume_signaled_event(ctx: &Win32Context, handle: u32) -> bool {
    let mut events = ctx.events.lock().unwrap();
    if let Some(event) = events.get_mut(&handle)
        && event.signaled
    {
        if !event.manual_reset {
            event.signaled = false;
        }
        return true;
    }
    false
}

/// WSA 이벤트 핸들의 현재 상태를 점검하고 필요하면 pending 비트를 갱신합니다.
pub(super) fn poll_wsa_event(ctx: &Win32Context, handle: u32) -> bool {
    let entry = ctx.wsa_event_map.lock().unwrap().get(&handle).cloned();
    let Some(entry) = entry else {
        return false;
    };

    if entry.pending != 0 {
        return true;
    }

    let mut should_signal_read = false;
    {
        let mut sockets = ctx.tcp_sockets.lock().unwrap();
        let Some(socket) = sockets.get_mut(&entry.socket) else {
            return false;
        };

        if !socket.recv_buf.is_empty() {
            should_signal_read = true;
        } else if let Some(chan_rx) = socket.chan_rx.as_mut() {
            match chan_rx.try_recv() {
                Ok(data) => {
                    socket.recv_buf.extend(data);
                    should_signal_read = true;
                }
                // EOF도 읽기 가능 상태처럼 전달해 상위 루프가 닫힘을 감지하게 합니다.
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    should_signal_read = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
    }

    if should_signal_read {
        ctx.wsa_event_map
            .lock()
            .unwrap()
            .entry(handle)
            .and_modify(|event| event.pending |= 0x01);
        return true;
    }

    false
}
