use crate::{
    dll::win32::{ApiHookResult, EmulatedThread, EventState, Win32Context},
    helper::{EXIT_ADDRESS, UnicornHelper},
};
use encoding_rs::EUC_KR;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use unicorn_engine::{RegisterX86, Unicorn};

/// `ERROR_INVALID_PARAMETER` 오류 코드입니다.
const ERROR_INVALID_PARAMETER: u32 = 87;
/// `FORMAT_MESSAGE_ALLOCATE_BUFFER` 플래그입니다.
const FORMAT_MESSAGE_ALLOCATE_BUFFER: u32 = 0x0000_0100;
/// `FORMAT_MESSAGE_IGNORE_INSERTS` 플래그입니다.
const FORMAT_MESSAGE_IGNORE_INSERTS: u32 = 0x0000_0200;
/// `FORMAT_MESSAGE_FROM_SYSTEM` 플래그입니다.
const FORMAT_MESSAGE_FROM_SYSTEM: u32 = 0x0000_1000;
/// `FORMAT_MESSAGE_MAX_WIDTH_MASK` 플래그 마스크입니다.
const FORMAT_MESSAGE_MAX_WIDTH_MASK: u32 = 0x0000_00FF;
/// `GMEM_ZEROINIT` 플래그입니다.
const GMEM_ZEROINIT: u32 = 0x0040;
/// `CREATE_SUSPENDED` 스레드 생성 플래그입니다.
const CREATE_SUSPENDED: u32 = 0x0000_0004;
/// `STACK_SIZE_PARAM_IS_A_RESERVATION` 스택 예약 크기 플래그입니다.
const STACK_SIZE_PARAM_IS_A_RESERVATION: u32 = 0x0001_0000;
/// 재시도형 대기 API를 다시 확인할 기본 폴링 간격입니다.
const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// `KERNEL32.dll` 프록시 구현 모듈
///
/// Windows 코어 서브시스템으로, 스레드, 메모리, 모듈 핸들, 뮤텍스(Mutex), 이벤트(Event) 등을 가상으로 프로비저닝
pub struct KERNEL32 {}

impl KERNEL32 {
    /// 지원하지 않는 플래그 비트를 계산합니다.
    fn unsupported_flag_bits(flags: u32, supported_mask: u32) -> u32 {
        flags & !supported_mask
    }

    /// 종료된 가상 스레드 엔트리와 해당 TLS 슬롯을 정리합니다.
    fn cleanup_finished_threads(ctx: &Win32Context) {
        let finished_thread_ids = {
            let mut threads = ctx.threads.lock().unwrap();
            let mut finished_ids = Vec::new();

            threads.retain(|thread| {
                if thread.alive {
                    true
                } else {
                    finished_ids.push(thread.thread_id);
                    false
                }
            });

            finished_ids
        };

        if finished_thread_ids.is_empty() {
            return;
        }

        let mut tls_slots = ctx.tls_slots.lock().unwrap();
        for thread_id in finished_thread_ids {
            tls_slots.remove(&thread_id);
        }
    }

    /// 현재 스레드(또는 메인 스레드)의 재시도형 대기 타임아웃 시각을 조회합니다.
    pub(crate) fn current_wait_deadline(ctx: &Win32Context, tid: u32) -> Option<Instant> {
        if tid == 0 {
            *ctx.main_wait_deadline.lock().unwrap()
        } else {
            ctx.threads
                .lock()
                .unwrap()
                .iter()
                .find(|thread| thread.thread_id == tid)
                .and_then(|thread| thread.wait_deadline)
        }
    }

    /// 현재 스레드(또는 메인 스레드)를 짧은 폴링 간격으로 다시 확인하도록 예약합니다.
    pub(crate) fn schedule_retry_wait(ctx: &Win32Context, tid: u32, deadline: Option<Instant>) {
        let now = Instant::now();
        let next_resume = deadline
            .map(|limit| (now + WAIT_POLL_INTERVAL).min(limit))
            .unwrap_or(now + WAIT_POLL_INTERVAL);

        if tid == 0 {
            *ctx.main_resume_time.lock().unwrap() = Some(next_resume);
            *ctx.main_wait_deadline.lock().unwrap() = deadline;
            return;
        }

        let mut threads = ctx.threads.lock().unwrap();
        if let Some(thread) = threads.iter_mut().find(|thread| thread.thread_id == tid) {
            thread.resume_time = Some(next_resume);
            thread.wait_deadline = deadline;
        }
    }

    /// 현재 스레드(또는 메인 스레드)의 재시도형 대기 상태를 해제합니다.
    pub(crate) fn clear_retry_wait(ctx: &Win32Context, tid: u32) {
        if tid == 0 {
            *ctx.main_resume_time.lock().unwrap() = None;
            *ctx.main_wait_deadline.lock().unwrap() = None;
            return;
        }

        let mut threads = ctx.threads.lock().unwrap();
        if let Some(thread) = threads.iter_mut().find(|thread| thread.thread_id == tid) {
            thread.resume_time = None;
            thread.wait_deadline = None;
        }
    }

    /// 이벤트 핸들이 신호 상태인지 확인하고 자동 리셋 이벤트면 소비합니다.
    fn try_consume_signaled_event(ctx: &Win32Context, handle: u32) -> bool {
        let mut events = ctx.events.lock().unwrap();
        if let Some(event) = events.get_mut(&handle) {
            if event.signaled {
                if !event.manual_reset {
                    event.signaled = false;
                }
                return true;
            }
        }
        false
    }

    // =========================================================
    // TLS (Thread Local Storage)
    // =========================================================
    // API: DWORD TlsAlloc(void)
    // 역할: 새 TLS(Thread Local Storage) 인덱스를 할당
    pub fn tls_alloc(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let ctx = uc.get_data();
        let index = ctx.tls_counter.fetch_add(1, Ordering::SeqCst);
        crate::emu_log!("[KERNEL32] TlsAlloc() -> DWORD {}", index);
        Some(ApiHookResult::callee(0, Some(index as i32)))
    }

    // API: BOOL TlsFree(DWORD dwTlsIndex)
    // 역할: 지정된 TLS 인덱스를 해제
    pub fn tls_free(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let index = uc.read_arg(0);
        let ctx = uc.get_data();
        let mut slots = ctx.tls_slots.lock().unwrap();
        for thread_slots in slots.values_mut() {
            thread_slots.remove(&index);
        }
        crate::emu_log!("[KERNEL32] TlsFree({}) -> BOOL 1", index);
        Some(ApiHookResult::callee(1, Some(1))) // TRUE
    }

    // API: LPVOID TlsGetValue(DWORD dwTlsIndex)
    // 역할: 현재 스레드의 TLS 슬롯에 저장된 값을 검색
    pub fn tls_get_value(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let index = uc.read_arg(0);
        let tid = uc.get_data().current_thread_idx.load(Ordering::SeqCst);
        let ctx = uc.get_data();
        let slots = ctx.tls_slots.lock().unwrap();
        let value = slots
            .get(&tid)
            .and_then(|t| t.get(&index))
            .copied()
            .unwrap_or(0);
        crate::emu_log!("[KERNEL32] TlsGetValue({}) -> LPVOID {:#x}", index, value);
        Some(ApiHookResult::callee(1, Some(value as i32)))
    }

    // API: BOOL TlsSetValue(DWORD dwTlsIndex, LPVOID lpTlsValue)
    // 역할: 현재 스레드의 TLS 슬롯에 값을 저장
    pub fn tls_set_value(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let index = uc.read_arg(0);
        let value = uc.read_arg(1);
        let tid = uc.get_data().current_thread_idx.load(Ordering::SeqCst);
        let ctx = uc.get_data();
        ctx.tls_slots
            .lock()
            .unwrap()
            .entry(tid)
            .or_insert_with(std::collections::HashMap::new)
            .insert(index, value);
        crate::emu_log!("[KERNEL32] TlsSetValue({}, {:#x}) -> BOOL 1", index, value);
        Some(ApiHookResult::callee(2, Some(1))) // TRUE
    }

    // =========================================================
    // Thread / Process
    // =========================================================
    // API: VOID Sleep(DWORD dwMilliseconds)
    // 역할: 지정된 밀리초 동안 스레드의 실행을 일시 중단
    pub fn sleep(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let dw_milliseconds = uc.read_arg(0);

        let tid = uc.get_data().current_thread_idx.load(Ordering::SeqCst);
        let in_thread = tid != 0;

        if dw_milliseconds > 0 && in_thread {
            let ctx = uc.get_data();
            let mut threads = ctx.threads.lock().unwrap();
            if let Some(t) = threads.iter_mut().find(|t| t.thread_id == tid) {
                if t.resume_time.is_none() {
                    t.resume_time =
                        Some(Instant::now() + Duration::from_millis(dw_milliseconds as u64));
                    t.wait_deadline = None;
                    return Some(ApiHookResult::retry());
                } else {
                    t.resume_time = None;
                    t.wait_deadline = None;
                }
            }
        }

        if !in_thread {
            let needs_yield = {
                let ctx = uc.get_data();
                let mut main_res = ctx.main_resume_time.lock().unwrap();
                if main_res.is_none() {
                    if dw_milliseconds > 0 {
                        *main_res =
                            Some(Instant::now() + Duration::from_millis(dw_milliseconds as u64));
                    }
                    *ctx.main_wait_deadline.lock().unwrap() = None;
                    true
                } else if Instant::now() >= main_res.unwrap() {
                    *main_res = None;
                    *ctx.main_wait_deadline.lock().unwrap() = None;
                    false
                } else {
                    *ctx.main_wait_deadline.lock().unwrap() = None;
                    true
                }
            };

            if needs_yield {
                KERNEL32::schedule_threads(uc);
                return Some(ApiHookResult::retry());
            } else {
                return Some(ApiHookResult::callee(1, None));
            }
        }

        Some(ApiHookResult::callee(1, None))
    }

    // API: DWORD GetCurrentThreadId(void)
    // 역할: 호출하는 스레드의 스레드 식별자를 검색
    pub fn get_current_thread_id(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let tid = uc.get_data().current_thread_idx.load(Ordering::SeqCst);
        let thread_id = if tid == 0 { 1u32 } else { tid };
        crate::emu_log!("[KERNEL32] GetCurrentThreadId() -> {}", thread_id);
        Some(ApiHookResult::callee(0, Some(thread_id as i32)))
    }

    // API: HANDLE GetCurrentThread(void)
    // 역할: 현재 스레드의 스레드 핸들을 반환
    pub fn get_current_thread(_uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        crate::emu_log!("[KERNEL32] GetCurrentThread() -> 0xFFFFFFFE");
        Some(ApiHookResult::callee(0, Some(-2i32))) // pseudo handle
    }

    // API: HANDLE GetCurrentProcess(void)
    // 역할: 현재 프로세스의 의사 핸들(pseudo handle)을 반환
    pub fn get_current_process(_uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        crate::emu_log!("[KERNEL32] GetCurrentProcess() -> 0xFFFFFFFF");
        Some(ApiHookResult::callee(0, Some(-1i32))) // pseudo handle
    }

    // API: DWORD WaitForSingleObject(HANDLE hHandle, DWORD dwMilliseconds)
    // 역할: 지정된 객체가 신호 상태가 되거나 시간제한이 초과될 때까지 대기.
    //       WSA 이벤트 핸들인 경우 소켓 readable 상태를 실제로 대기합니다.
    pub fn wait_for_single_object(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let h_handle = uc.read_arg(0);
        let dw_milliseconds = uc.read_arg(1);
        let ctx = uc.get_data();
        let tid = ctx.current_thread_idx.load(Ordering::SeqCst);
        let now = Instant::now();

        if Self::try_consume_signaled_event(ctx, h_handle) {
            Self::clear_retry_wait(ctx, tid);
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
                Self::clear_retry_wait(ctx, tid);
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
                Self::clear_retry_wait(ctx, tid);
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
                Self::clear_retry_wait(ctx, tid);
                return Some(ApiHookResult::callee(2, Some(0))); // WAIT_OBJECT_0
            }

            if dw_milliseconds == 0 {
                Self::clear_retry_wait(ctx, tid);
                crate::emu_log!(
                    "[KERNEL32] WaitForSingleObject(WSA event={:#x}) -> WAIT_TIMEOUT (poll)",
                    h_handle
                );
                return Some(ApiHookResult::callee(2, Some(0x102)));
            }

            let deadline = if dw_milliseconds == 0xFFFF_FFFF {
                None
            } else {
                Self::current_wait_deadline(ctx, tid)
                    .or(Some(now + Duration::from_millis(dw_milliseconds as u64)))
            };

            if let Some(limit) = deadline
                && now >= limit
            {
                Self::clear_retry_wait(ctx, tid);
                crate::emu_log!(
                    "[KERNEL32] WaitForSingleObject(WSA event={:#x}) -> WAIT_TIMEOUT",
                    h_handle
                );
                return Some(ApiHookResult::callee(2, Some(0x102)));
            }

            Self::schedule_retry_wait(ctx, tid, deadline);
            return Some(ApiHookResult::retry());
        }

        if dw_milliseconds == 0 {
            Self::clear_retry_wait(ctx, tid);
            crate::emu_log!(
                "[KERNEL32] WaitForSingleObject({:#x}, 0) -> WAIT_TIMEOUT",
                h_handle
            );
            return Some(ApiHookResult::callee(2, Some(0x102)));
        }

        let deadline = if dw_milliseconds == 0xFFFF_FFFF {
            None
        } else {
            Self::current_wait_deadline(ctx, tid)
                .or(Some(now + Duration::from_millis(dw_milliseconds as u64)))
        };

        if let Some(limit) = deadline
            && now >= limit
        {
            Self::clear_retry_wait(ctx, tid);
            crate::emu_log!(
                "[KERNEL32] WaitForSingleObject({:#x}, {}) -> WAIT_TIMEOUT",
                h_handle,
                dw_milliseconds
            );
            return Some(ApiHookResult::callee(2, Some(0x102)));
        }

        Self::schedule_retry_wait(ctx, tid, deadline);
        Some(ApiHookResult::retry())
    }

    // API: DWORD WaitForMultipleObjects(DWORD nCount, const HANDLE *lpHandles, BOOL bWaitAll, DWORD dwMilliseconds)
    // 역할: 하나 또는 모든 지정된 개체가 신호 상태가 될 때까지 대기.
    //       핸들 목록 중 WSA 이벤트가 있으면 소켓 readable을 실제로 대기합니다.
    pub fn wait_for_multiple_objects(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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

            if Self::try_consume_signaled_event(ctx, handle) {
                Self::clear_retry_wait(ctx, tid);
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
                    Self::clear_retry_wait(ctx, tid);
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
                    Self::clear_retry_wait(ctx, tid);
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
                    Self::clear_retry_wait(ctx, tid);
                    return Some(ApiHookResult::callee(4, Some(i as i32)));
                }
                // 채널 없음 또는 데이터 없음 → 다음 핸들 시도
            }
        }

        if dw_milliseconds == 0 {
            Self::clear_retry_wait(ctx, tid);
            crate::emu_log!(
                "[KERNEL32] WaitForMultipleObjects({}) -> WAIT_TIMEOUT",
                n_count
            );
            return Some(ApiHookResult::callee(4, Some(0x102)));
        }

        let deadline = if dw_milliseconds == 0xFFFF_FFFF {
            None
        } else {
            Self::current_wait_deadline(ctx, tid)
                .or(Some(now + Duration::from_millis(dw_milliseconds as u64)))
        };

        if let Some(limit) = deadline
            && now >= limit
        {
            Self::clear_retry_wait(ctx, tid);
            crate::emu_log!(
                "[KERNEL32] WaitForMultipleObjects({}) -> WAIT_TIMEOUT",
                n_count
            );
            return Some(ApiHookResult::callee(4, Some(0x102)));
        }

        Self::schedule_retry_wait(ctx, tid, deadline);
        crate::emu_log!("[KERNEL32] WaitForMultipleObjects({}) -> retry", n_count);
        Some(ApiHookResult::retry())
    }

    // API: BOOL TerminateThread(HANDLE hThread, DWORD dwExitCode)
    // 역할: 스레드를 강제 종료
    pub fn terminate_thread(_uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let h_thread = _uc.read_arg(0);
        let dw_exit_code = _uc.read_arg(1);
        crate::emu_log!(
            "[KERNEL32] TerminateThread({:#x}, {}) -> BOOL 1",
            h_thread,
            dw_exit_code
        );
        Some(ApiHookResult::callee(2, Some(1)))
    }

    // API: BOOL SetThreadPriority(HANDLE hThread, int nPriority)
    // 역할: 지정된 스레드의 우선 순위 값을 설정
    pub fn set_thread_priority(_uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let h_thread = _uc.read_arg(0);
        let n_priority = _uc.read_arg(1);
        crate::emu_log!(
            "[KERNEL32] SetThreadPriority({:#x}, {}) -> BOOL 1",
            h_thread,
            n_priority
        );
        Some(ApiHookResult::callee(2, Some(1)))
    }

    // API: BOOL DisableThreadLibraryCalls(HMODULE hLibModule)
    // 역할: DLL의 스레드 부착/분리 알림을 비활성화
    pub fn disable_thread_library_calls(_uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let h_lib_module = _uc.read_arg(0);
        crate::emu_log!(
            "[KERNEL32] DisableThreadLibraryCalls({:#x}) -> BOOL 1",
            h_lib_module
        );
        Some(ApiHookResult::callee(1, Some(1))) // TRUE
    }

    // API: BOOL CreateProcessA(LPCSTR lpApplicationName, LPSTR lpCommandLine, ...)
    // 역할: 새로운 프로세스와 그 기본 스레드를 생성
    pub fn create_process_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let lp_application_name = uc.read_arg(0);
        let lp_command_line = uc.read_arg(1);
        let lp_process_information = uc.read_arg(9);

        let app_name = if lp_application_name != 0 {
            uc.read_string(lp_application_name as u64)
        } else {
            "NULL".to_string()
        };
        let cmd_line = if lp_command_line != 0 {
            uc.read_string(lp_command_line as u64)
        } else {
            "NULL".to_string()
        };

        crate::emu_log!(
            "[KERNEL32] CreateProcessA(app=\"{}\", cmd=\"{}\")",
            app_name,
            cmd_line
        );

        if lp_process_information != 0 {
            let h_process = uc.get_data().alloc_handle();
            let h_thread = uc.get_data().alloc_handle();
            uc.write_u32(lp_process_information as u64, h_process); // hProcess
            uc.write_u32(lp_process_information as u64 + 4, h_thread); // hThread
            uc.write_u32(lp_process_information as u64 + 8, 1234); // dwProcessId
            uc.write_u32(lp_process_information as u64 + 12, 5678); // dwThreadId
        }

        Some(ApiHookResult::callee(10, Some(1))) // TRUE = 성공
    }

    // =========================================================
    // Handle
    // =========================================================
    // API: BOOL CloseHandle(HANDLE hObject)
    // 역할: 열려있는 개체 핸들을 닫음
    pub fn close_handle(_uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let h_handle = _uc.read_arg(0);
        crate::emu_log!("[KERNEL32] CloseHandle({:#x}) -> BOOL 1", h_handle);
        Some(ApiHookResult::callee(1, Some(1))) // TRUE
    }

    /// API: BOOL DuplicateHandle(HANDLE hSourceProcessHandle, HANDLE hSourceHandle, ...)
    /// 역할: 객체 핸들을 복제합니다.
    pub fn duplicate_handle(_uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let h_source_process_handle = _uc.read_arg(0);
        let h_source_handle = _uc.read_arg(1);
        let h_target_process_handle = _uc.read_arg(2);
        let lp_target_handle = _uc.read_arg(3);
        let dw_desired_access = _uc.read_arg(4);
        let b_inherit_handles = _uc.read_arg(5);
        let dw_options = _uc.read_arg(6);
        crate::emu_log!(
            "[KERNEL32] DuplicateHandle({:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 1",
            h_source_process_handle,
            h_source_handle,
            h_target_process_handle,
            lp_target_handle,
            dw_desired_access,
            b_inherit_handles,
            dw_options
        );
        Some(ApiHookResult::callee(7, Some(1))) // TRUE
    }

    // =========================================================
    // Error
    // =========================================================
    // API: DWORD GetLastError(void)
    // 역할: 호출하는 스레드의 가장 최근 오류 코드를 검색
    pub fn get_last_error(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let err = uc.get_data().last_error.load(Ordering::SeqCst);
        crate::emu_log!("[KERNEL32] GetLastError() -> DWORD {:#x}", err);
        Some(ApiHookResult::callee(0, Some(err as i32)))
    }

    // API: void SetLastError(DWORD dwErrCode)
    // 역할: 호출 스레드의 가장 최근 오류 코드를 설정
    pub fn set_last_error(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let code = uc.read_arg(0);
        uc.get_data().last_error.store(code, Ordering::SeqCst);
        crate::emu_log!("[KERNEL32] SetLastError({:#x}) -> VOID", code);
        Some(ApiHookResult::callee(1, None))
    }

    /// API: DWORD FormatMessageA(DWORD dwFlags, LPCVOID lpSource, DWORD dwMessageId, ...)
    /// 역할: 메시지 정의를 문자열로 포맷팅합니다.
    pub fn format_message_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let dw_flags = uc.read_arg(0);
        let _lp_source = uc.read_arg(1);
        let dw_message_id = uc.read_arg(2);
        let _dw_language_id = uc.read_arg(3);
        let lp_buffer = uc.read_arg(4);
        let n_size = uc.read_arg(5);
        let _arguments = uc.read_arg(6);

        crate::emu_log!(
            "[KERNEL32] FormatMessageA(flags={:#x}, id={:#x})",
            dw_flags,
            dw_message_id
        );

        let allowed_flags = FORMAT_MESSAGE_ALLOCATE_BUFFER
            | FORMAT_MESSAGE_IGNORE_INSERTS
            | FORMAT_MESSAGE_FROM_SYSTEM
            | FORMAT_MESSAGE_MAX_WIDTH_MASK;
        let unsupported = Self::unsupported_flag_bits(dw_flags, allowed_flags);
        if unsupported != 0 || dw_flags & FORMAT_MESSAGE_FROM_SYSTEM == 0 {
            uc.get_data()
                .last_error
                .store(ERROR_INVALID_PARAMETER, Ordering::SeqCst);
            crate::emu_log!(
                "[KERNEL32] FormatMessageA unsupported flags={:#x} unsupported={:#x}",
                dw_flags,
                unsupported
            );
            return Some(ApiHookResult::callee(7, Some(0)));
        }

        let msg = format!("Win32 Error {:#x}\0", dw_message_id);
        let msg_len = msg.len();

        if dw_flags & FORMAT_MESSAGE_ALLOCATE_BUFFER != 0 {
            // Allocate buffer and write pointer to lp_buffer
            let addr = uc.alloc_str(&msg);
            uc.write_u32(lp_buffer as u64, addr);
        } else {
            // Write to provided buffer
            if lp_buffer != 0 && n_size as usize >= msg_len {
                uc.mem_write(lp_buffer as u64, msg.as_bytes()).unwrap();
            }
        }

        Some(ApiHookResult::callee(7, Some(msg_len as i32 - 1)))
    }

    // =========================================================
    // Event / Sync
    // =========================================================
    // API: HANDLE CreateEventA(LPSECURITY_ATTRIBUTES lpEventAttributes, BOOL bManualReset, BOOL bInitialState, LPCSTR lpName)
    // 역할: 이벤트 개체를 생성하거나 오픈
    pub fn create_event_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
    pub fn set_event(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
    pub fn pulse_event(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let handle = uc.read_arg(0);
        crate::emu_log!("[KERNEL32] PulseEvent({:#x}) -> BOOL 1", handle);
        Some(ApiHookResult::callee(1, Some(1)))
    }

    // API: BOOL ResetEvent(HANDLE hEvent)
    // 역할: 지정된 이벤트 개체를 비신호(nonsignaled) 상태로 설정
    pub fn reset_event(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
    pub fn initialize_critical_section(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let lp_critical_section = uc.read_arg(0);
        crate::emu_log!(
            "[KERNEL32] InitializeCriticalSection({:#x}) -> VOID",
            lp_critical_section
        );
        Some(ApiHookResult::callee(1, None))
    }

    // API: void DeleteCriticalSection(LPCRITICAL_SECTION lpCriticalSection)
    // 역할: 가상 크리티컬 섹션 객체를 삭제
    pub fn delete_critical_section(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let lp_critical_section = uc.read_arg(0);
        crate::emu_log!(
            "[KERNEL32] DeleteCriticalSection({:#x}) -> VOID",
            lp_critical_section
        );
        Some(ApiHookResult::callee(1, None))
    }

    // API: void EnterCriticalSection(LPCRITICAL_SECTION lpCriticalSection)
    // 역할: 크리티컬 섹션에 진입
    pub fn enter_critical_section(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let lp_critical_section = uc.read_arg(0);
        crate::emu_log!(
            "[KERNEL32] EnterCriticalSection({:#x}) -> VOID",
            lp_critical_section
        );
        Some(ApiHookResult::callee(1, None))
    }

    // API: void LeaveCriticalSection(LPCRITICAL_SECTION lpCriticalSection)
    // 역할: 크리티컬 섹션을 떠남
    pub fn leave_critical_section(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
    pub fn create_mutex_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
    pub fn release_mutex(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let h_mutex = uc.read_arg(0);
        crate::emu_log!("[KERNEL32] ReleaseMutex({:#x}) -> BOOL 1", h_mutex);
        Some(ApiHookResult::callee(1, Some(1)))
    }

    // =========================================================
    // Debug
    // =========================================================
    // API: void OutputDebugStringA(LPCSTR lpOutputString)
    // 역할: 문자열을 디버거로 보내 화면에 출력
    pub fn output_debug_string_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let addr = uc.read_arg(0);
        let s = if addr != 0 {
            uc.read_euc_kr(addr as u64)
        } else {
            String::new()
        };
        crate::emu_log!("[KERNEL32] OutputDebugStringA(\"{s}\") -> VOID");
        Some(ApiHookResult::callee(1, None))
    }

    // =========================================================
    // String
    // =========================================================
    // API: int lstrlenA(LPCSTR lpString)
    // 역할: 지정된 문자열의 길이를 바이트 단위로 반환
    pub fn lstrlen_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let addr = uc.read_arg(0);
        let s = if addr != 0 {
            uc.read_euc_kr(addr as u64)
        } else {
            String::new()
        };
        let len = s.len() as i32;
        crate::emu_log!("[KERNEL32] lstrlenA(\"{}\") -> int {}", s, len);
        Some(ApiHookResult::callee(1, Some(len)))
    }

    // API: LPSTR lstrcpyA(LPSTR lpString1, LPCSTR lpString2)
    // 역할: 문자열을 한 버퍼에서 다른 버퍼로 복사
    pub fn lstrcpy_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let dst = uc.read_arg(0);
        let dst_str = if dst != 0 {
            uc.read_euc_kr(dst as u64)
        } else {
            String::new()
        };
        let src = uc.read_arg(1);
        let src_str = if src != 0 {
            uc.read_euc_kr(src as u64)
        } else {
            String::new()
        };
        uc.write_euc_kr(dst as u64, &src_str);
        crate::emu_log!("[KERNEL32] lstrcpyA(\"{dst_str}\", \"{src_str}\") -> LPSTR {dst:#x}",);
        Some(ApiHookResult::callee(2, Some(dst as i32)))
    }

    // API: LPSTR lstrcpynA(LPSTR lpString1, LPCSTR lpString2, int iMaxLength)
    // 역할: 지정된 수의 문자를 복사
    pub fn lstrcpyn_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let dst = uc.read_arg(0);
        let dst_str = if dst != 0 {
            uc.read_euc_kr(dst as u64)
        } else {
            String::new()
        };
        let src = uc.read_arg(1);
        let src_str = if src != 0 {
            uc.read_euc_kr(src as u64)
        } else {
            String::new()
        };
        let max_count = uc.read_arg(2) as usize;
        let (encoded, _, _) = EUC_KR.encode(&src_str);
        let encoded = encoded.as_ref();
        let copy_len = encoded.len().min(max_count.saturating_sub(1));
        let mut bytes = encoded[..copy_len].to_vec();
        bytes.push(0);
        uc.mem_write(dst as u64, &bytes).unwrap();
        crate::emu_log!(
            "[KERNEL32] lstrcpynA(\"{dst_str}\", \"{src_str}\", {max_count}) -> LPSTR {dst:#x}",
        );
        Some(ApiHookResult::callee(3, Some(dst as i32)))
    }

    // API: LPSTR lstrcatA(LPSTR lpString1, LPCSTR lpString2)
    // 역할: 한 문자열을 다른 문자열에 추가
    pub fn lstrcat_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let dst = uc.read_arg(0);
        let src = uc.read_arg(1);
        let dst_str = uc.read_euc_kr(dst as u64);
        let src_str = uc.read_euc_kr(src as u64);
        let (encoded_src, _, _) = EUC_KR.encode(&src_str);
        let mut bytes = encoded_src.as_ref().to_vec();
        bytes.push(0);
        let (encoded_dst, _, _) = EUC_KR.encode(&dst_str);
        uc.mem_write(dst as u64 + encoded_dst.len() as u64, &bytes)
            .unwrap();
        crate::emu_log!(
            "[KERNEL32] lstrcatA(\"{}\", \"{}\") -> LPSTR {:#x}",
            dst_str,
            src_str,
            dst
        );
        Some(ApiHookResult::callee(2, Some(dst as i32)))
    }

    // API: int lstrcmpA(LPCSTR lpString1, LPCSTR lpString2)
    // 역할: 두 개의 문자열을 비교
    pub fn lstrcmp_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let s1_addr = uc.read_arg(0);
        let s2_addr = uc.read_arg(1);
        let s1 = uc.read_string(s1_addr as u64);
        let s2 = uc.read_string(s2_addr as u64);
        let result = s1.cmp(&s2) as i32;
        crate::emu_log!(
            "[KERNEL32] lstrcmpA(\"{}\", \"{}\") -> int {}",
            s1,
            s2,
            result
        );
        Some(ApiHookResult::callee(2, Some(result)))
    }

    // =========================================================
    // Module
    // =========================================================
    // API: HMODULE GetModuleHandleA(LPCSTR lpModuleName)
    // 역할: 호출하는 프로세스에 이미 로드된 모듈 핸들을 검색
    pub fn get_module_handle_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let name_addr = uc.read_arg(0);
        if name_addr == 0 {
            // NULL = 현재 실행 모듈 (4Leaf.dll의 베이스)
            crate::emu_log!("[KERNEL32] GetModuleHandleA(NULL) -> HMODULE 0x35000000");
            Some(ApiHookResult::callee(1, Some(0x3500_0000u32 as i32)))
        } else {
            let name = uc.read_euc_kr(name_addr as u64);
            // 로드된 DLL에서 찾기
            let ctx = uc.get_data();
            let mut found_base: u32 = 0;
            let modules = ctx.dll_modules.lock().unwrap();
            for (dll_name, dll) in modules.iter() {
                if dll_name.eq_ignore_ascii_case(&name) || dll.name.ends_with(&name) {
                    found_base = dll.base_addr as u32;
                    break;
                }
            }
            crate::emu_log!(
                "[KERNEL32] GetModuleHandleA(\"{}\") -> HMODULE {:#x}",
                name,
                found_base
            );
            Some(ApiHookResult::callee(1, Some(found_base as i32)))
        }
    }

    // API: DWORD GetModuleFileNameA(HMODULE hModule, LPSTR lpFilename, DWORD nSize)
    // 역할: 모듈이 포함된 실행 파일의 절대 경로를 조회
    pub fn get_module_file_name_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let module = uc.read_arg(0);
        let buf_addr = uc.read_arg(1);
        let buf_size = uc.read_arg(2);
        let path = ".\\Resources\\4Leaf.exe\0";
        let bytes = path.as_bytes();
        let copy_len = bytes.len().min(buf_size as usize);
        uc.mem_write(buf_addr as u64, &bytes[..copy_len]).unwrap();
        crate::emu_log!(
            "[KERNEL32] GetModuleFileNameA({:#x}, {:#x}, {}) -> DWORD \"{}\"",
            module,
            buf_addr,
            buf_size,
            &path[..path.len() - 1]
        );
        Some(ApiHookResult::callee(3, Some((copy_len - 1) as i32)))
    }

    // API: HMODULE LoadLibraryA(LPCSTR lpLibFileName)
    // 역할: 지정된 모듈을 호출 컨텍스트의 주소 공간으로 매핑
    pub fn load_library_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let name_addr = uc.read_arg(0);
        let name = if name_addr != 0 {
            uc.read_euc_kr(name_addr as u64)
        } else {
            String::new()
        };
        // 이미 로드된 DLL이면 핸들 반환
        let ctx = uc.get_data();
        let mut found_base: u32 = 0;
        let modules = ctx.dll_modules.lock().unwrap();
        for (dll_name, dll) in modules.iter() {
            if dll_name.eq_ignore_ascii_case(&name) {
                found_base = dll.base_addr as u32;
                break;
            }
        }
        crate::emu_log!(
            "[KERNEL32] LoadLibraryA(\"{}\") -> HMODULE {:#x}",
            name,
            found_base
        );
        Some(ApiHookResult::callee(1, Some(found_base as i32)))
    }

    // API: BOOL FreeLibrary(HMODULE hLibModule)
    // 역할: 로드된 DLL 모듈을 해제
    pub fn free_library(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let module = uc.read_arg(0);
        crate::emu_log!("[KERNEL32] FreeLibrary({:#x}) -> BOOL 1", module);
        Some(ApiHookResult::callee(1, Some(1)))
    }

    // API: FARPROC GetProcAddress(HMODULE hModule, LPCSTR lpProcName)
    // 역할: DLL에서 지정된 익스포트 함수의 주소를 조회
    pub fn get_proc_address(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let module = uc.read_arg(0);
        let name_addr = uc.read_arg(1);
        let name = if name_addr != 0 {
            uc.read_euc_kr(name_addr as u64)
        } else {
            String::new()
        };
        let dll_name = {
            let ctx = uc.get_data();
            let modules = ctx.dll_modules.lock().unwrap();
            modules.iter().find_map(|(dll_name, dll)| {
                ((dll.base_addr as u32) == module).then_some(dll_name.clone())
            })
        };
        let proc_addr = if let Some(dll_name) = dll_name {
            let loaded_export = {
                let ctx = uc.get_data();
                let modules = ctx.dll_modules.lock().unwrap();
                modules
                    .get(&dll_name)
                    .and_then(|dll| dll.exports.get(&name).copied())
                    .map(|addr| addr as u32)
            };
            loaded_export.or_else(|| Win32Context::resolve_proxy_export(uc, &dll_name, &name))
        } else {
            None
        }
        .unwrap_or(0);
        crate::emu_log!(
            "[KERNEL32] GetProcAddress({:#x}, \"{}\") -> FARPROC {:#x}",
            module,
            name,
            proc_addr
        );
        Some(ApiHookResult::callee(2, Some(proc_addr as i32)))
    }

    // =========================================================
    // Math / Time
    // =========================================================
    // API: int MulDiv(int nNumber, int nNumerator, int nDenominator)
    // 역할: 두 개의 32비트 값을 곱한 후 세 번째 32비트 값으로 나누고 결과를 32비트 값으로 돌려줌
    pub fn mul_div(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let number = uc.read_arg(0) as i32;
        let numerator = uc.read_arg(1) as i32;
        let denominator = uc.read_arg(2) as i32;
        let result = if denominator == 0 {
            -1
        } else {
            ((number as i64 * numerator as i64) / denominator as i64) as i32
        };
        crate::emu_log!(
            "[KERNEL32] MulDiv({}, {}, {}) -> int {}",
            number,
            numerator,
            denominator,
            result
        );
        Some(ApiHookResult::callee(3, Some(result)))
    }

    // API: DWORD GetTickCount(void)
    // 역할: 시스템이 시작된 후 지난 밀리초 시간을 검색
    pub fn get_tick_count(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let elapsed = uc.get_data().start_time.elapsed().as_millis() as u32;
        // crate::emu_log!("[KERNEL32] GetTickCount() -> DWORD {}", elapsed);
        Some(ApiHookResult::callee(0, Some(elapsed as i32)))
    }

    // API: void GetLocalTime(LPSYSTEMTIME lpSystemTime)
    // 역할: 현재 로컬 날짜와 시간을 시스템 타임 구조체로 가져옴
    pub fn get_local_time(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let buf_addr = uc.read_arg(0);
        // SYSTEMTIME: 8 WORDs = 16 bytes, 0으로 채움
        let zeros = [0u8; 16];
        uc.mem_write(buf_addr as u64, &zeros).unwrap();
        crate::emu_log!("[KERNEL32] GetLocalTime({:#x}) -> VOID", buf_addr);
        Some(ApiHookResult::callee(1, None))
    }

    // API: BOOL SystemTimeToFileTime(const SYSTEMTIME *lpSystemTime, LPFILETIME lpFileTime)
    // 역할: 시스템 시간을 파일 시간 형식으로 변환
    pub fn system_time_to_file_time(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let system_time_addr = uc.read_arg(0);
        let file_time_addr = uc.read_arg(1);
        crate::emu_log!(
            "[KERNEL32] SystemTimeToFileTime({:#x}, {:#x}) -> BOOL 1",
            system_time_addr,
            file_time_addr
        );
        Some(ApiHookResult::callee(2, Some(1)))
    }

    // =========================================================
    // Interlocked
    // =========================================================
    // API: LONG InterlockedExchange(LONG volatile *Target, LONG Value)
    // 역할: 원자적 조작을 통해 두 개의 32비트 값을 교환
    pub fn interlocked_exchange(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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

    // =========================================================
    // Memory
    // =========================================================
    // API: HGLOBAL GlobalAlloc(UINT uFlags, SIZE_T dwBytes)
    // 역할: 힙에서 지정된 바이트의 메모리를 할당
    pub fn global_alloc(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let flags = uc.read_arg(0);
        let size = uc.read_arg(1);
        let addr = uc.malloc(size as usize);
        // `GMEM_ZEROINIT`일 때만 원본과 같이 0으로 초기화합니다.
        if flags & GMEM_ZEROINIT != 0 {
            let zeros = vec![0u8; size as usize];
            uc.mem_write(addr, &zeros).unwrap();
        }
        crate::emu_log!(
            "[KERNEL32] GlobalAlloc({:#x}, {}) -> HGLOBAL {:#x}",
            flags,
            size,
            addr
        );
        Some(ApiHookResult::callee(2, Some(addr as i32)))
    }

    // API: LPVOID GlobalLock(HGLOBAL hMem)
    // 역할: 메모리를 고정하여 첫 바이트에 대한 포인터를 반환
    pub fn global_lock(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let handle = uc.read_arg(0);
        // 핸들 = 메모리 포인터로 취급
        crate::emu_log!(
            "[KERNEL32] GlobalLock({:#x}) -> LPVOID {:#x}",
            handle,
            handle
        );
        Some(ApiHookResult::callee(1, Some(handle as i32)))
    }

    // API: BOOL GlobalUnlock(HGLOBAL hMem)
    // 역할: GlobalLock에 의해 잠긴 메모리를 해제
    pub fn global_unlock(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let handle = uc.read_arg(0);
        crate::emu_log!("[KERNEL32] GlobalUnlock({:#x}) -> BOOL 1", handle);
        Some(ApiHookResult::callee(1, Some(1)))
    }

    // API: HGLOBAL GlobalFree(HGLOBAL hMem)
    // 역할: 지정된 전역 메모리 개체를 해제
    pub fn global_free(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let handle = uc.read_arg(0);
        crate::emu_log!("[KERNEL32] GlobalFree({:#x}) -> HGLOBAL 0", handle);
        Some(ApiHookResult::callee(1, Some(0))) // 성공 시 NULL
    }

    // =========================================================
    // File System
    // =========================================================
    // API: HANDLE CreateFileA(LPCSTR lpFileName, ...)
    // 역할: 파일 또는 입출력 디바이스 개체를 생성하거나 오픈
    pub fn create_file_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let name_addr = uc.read_arg(0);
        let name = if name_addr != 0 {
            uc.read_euc_kr(name_addr as u64)
        } else {
            String::new()
        };
        let access = uc.read_arg(1);
        let share_mode = uc.read_arg(2);
        let security_attributes = uc.read_arg(3);
        let creation_disposition = uc.read_arg(4);
        let template_file = uc.read_arg(5);
        let ctx = uc.get_data();
        let handle = ctx.alloc_handle();
        crate::emu_log!(
            "[KERNEL32] CreateFileA(\"{}\", {:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> HANDLE {:#x}",
            name,
            access,
            share_mode,
            security_attributes,
            creation_disposition,
            template_file,
            handle
        );
        Some(ApiHookResult::callee(7, Some(handle as i32)))
    }

    // API: HANDLE FindFirstFileA(LPCSTR lpFileName, LPWIN32_FIND_DATAA lpFindFileData)
    // 역할: 지정된 이름과 일치하는 파일용 핸들을 검색/생성
    pub fn find_first_file_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let name_addr = uc.read_arg(0);
        let name = if name_addr != 0 {
            uc.read_euc_kr(name_addr as u64)
        } else {
            String::new()
        };
        let find_file_data_addr = uc.read_arg(1);
        crate::emu_log!(
            "[KERNEL32] FindFirstFileA(\"{}\", {:#x}) -> INVALID_HANDLE_VALUE",
            name,
            find_file_data_addr
        );
        Some(ApiHookResult::callee(2, Some(-1i32))) // INVALID_HANDLE_VALUE
    }

    // API: BOOL FindNextFileA(HANDLE hFindFile, LPWIN32_FIND_DATAA lpFindFileData)
    // 역할: FindFirstFileA의 추가 파일 찾기를 실행
    pub fn find_next_file_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hfindfile = uc.read_arg(0);
        let find_file_data_addr = uc.read_arg(1);
        crate::emu_log!(
            "[KERNEL32] FindNextFileA({:#x}, {:#x}) -> FALSE",
            hfindfile,
            find_file_data_addr
        );
        Some(ApiHookResult::callee(2, Some(0)))
    }

    // API: BOOL FindClose(HANDLE hFindFile)
    // 역할: FindFirstFileA에 의해 띄워진 파일 탐색 핸들을 닫음
    pub fn find_close(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hfindfile = uc.read_arg(0);
        crate::emu_log!("[KERNEL32] FindClose({:#x}) -> BOOL 1", hfindfile);
        Some(ApiHookResult::callee(1, Some(1)))
    }

    // API: DWORD GetFileAttributesA(LPCSTR lpFileName)
    // 역할: 지정된 파일 또는 디렉토리의 파일 시스템 속성을 검색
    pub fn get_file_attributes_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let name_addr = uc.read_arg(0);
        let name = if name_addr != 0 {
            uc.read_euc_kr(name_addr as u64)
        } else {
            String::new()
        };
        crate::emu_log!(
            "[KERNEL32] GetFileAttributesA(\"{}\") -> INVALID_FILE_ATTRIBUTES",
            name
        );
        Some(ApiHookResult::callee(1, Some(-1i32))) // INVALID_FILE_ATTRIBUTES
    }

    // API: BOOL SetFileAttributesA(LPCSTR lpFileName, DWORD dwFileAttributes)
    // 역할: 지정된 파일 또는 디렉토리의 속성을 설정
    pub fn set_file_attributes_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let name_addr = uc.read_arg(0);
        let name = if name_addr != 0 {
            uc.read_euc_kr(name_addr as u64)
        } else {
            String::new()
        };
        let attributes = uc.read_arg(1);
        crate::emu_log!(
            "[KERNEL32] SetFileAttributesA(\"{}\", {:#x}) -> BOOL 1",
            name,
            attributes
        );
        Some(ApiHookResult::callee(2, Some(1)))
    }

    // API: BOOL RemoveDirectoryA(LPCSTR lpPathName)
    // 역할: 기존의 빈 디렉터리를 삭제
    pub fn remove_directory_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let name_addr = uc.read_arg(0);
        let name = if name_addr != 0 {
            uc.read_euc_kr(name_addr as u64)
        } else {
            String::new()
        };
        crate::emu_log!("[KERNEL32] RemoveDirectoryA(\"{}\") -> BOOL 1", name);
        Some(ApiHookResult::callee(1, Some(1)))
    }

    // API: BOOL CreateDirectoryA(LPCSTR lpPathName, LPSECURITY_ATTRIBUTES lpSecurityAttributes)
    // 역할: 새 디렉토리를 생성
    pub fn create_directory_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let name_addr = uc.read_arg(0);
        let name = if name_addr != 0 {
            uc.read_euc_kr(name_addr as u64)
        } else {
            String::new()
        };
        let security_attributes = uc.read_arg(1);
        crate::emu_log!(
            "[KERNEL32] CreateDirectoryA(\"{}\", {:#x}) -> BOOL 1",
            name,
            security_attributes
        );
        Some(ApiHookResult::callee(2, Some(1)))
    }

    // API: BOOL DeleteFileA(LPCSTR lpFileName)
    // 역할: 기존 파일을 삭제
    pub fn delete_file_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let name_addr = uc.read_arg(0);
        let name = if name_addr != 0 {
            uc.read_euc_kr(name_addr as u64)
        } else {
            String::new()
        };
        crate::emu_log!("[KERNEL32] DeleteFileA(\"{}\") -> BOOL 1", name);
        Some(ApiHookResult::callee(1, Some(1)))
    }

    // API: BOOL CopyFileA(LPCSTR lpExistingFileName, LPCSTR lpNewFileName, BOOL bFailIfExists)
    // 역할: 기존 파일을 새 파일로 복사
    pub fn copy_file_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let src_addr = uc.read_arg(0);
        let src = if src_addr != 0 {
            uc.read_euc_kr(src_addr as u64)
        } else {
            String::new()
        };
        let dst_addr = uc.read_arg(1);
        let dst = if dst_addr != 0 {
            uc.read_euc_kr(dst_addr as u64)
        } else {
            String::new()
        };
        let fail_if_exists = uc.read_arg(2);
        crate::emu_log!(
            "[KERNEL32] CopyFileA(\"{}\", \"{}\", {}) -> BOOL 1",
            src,
            dst,
            fail_if_exists
        );
        Some(ApiHookResult::callee(3, Some(1)))
    }

    // API: DWORD GetTempPathA(DWORD nBufferLength, LPSTR lpBuffer)
    // 역할: 임시 파일용 디렉토리 경로를 지정
    pub fn get_temp_path_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let buf_size = uc.read_arg(0);
        let buf_addr = uc.read_arg(1);
        let path = ".\\Temp\\\0";
        uc.mem_write(buf_addr as u64, path.as_bytes()).unwrap();
        crate::emu_log!(
            "[KERNEL32] GetTempPathA({:#x}, {:#x}) -> \"{}\"",
            buf_size,
            buf_addr,
            path
        );
        Some(ApiHookResult::callee(2, Some((path.len() - 1) as i32)))
    }

    // API: DWORD GetShortPathNameA(LPCSTR lpszLongPath, LPSTR lpszShortPath, DWORD cchBuffer)
    // 역할: 지정된 경로의 짧은 경로(8.3 폼) 형태를 가져옴
    pub fn get_short_path_name_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let long_addr = uc.read_arg(0);
        let long_name = if long_addr != 0 {
            uc.read_euc_kr(long_addr as u64)
        } else {
            String::new()
        };
        let short_addr = uc.read_arg(1);
        let buf_size = uc.read_arg(2);
        let mut bytes = long_name.as_bytes().to_vec();
        bytes.push(0);
        if short_addr != 0 {
            uc.mem_write(short_addr as u64, &bytes).unwrap();
        }
        crate::emu_log!(
            "[KERNEL32] GetShortPathNameA(\"{}\", {:#x}, {}) -> {:#x}",
            long_name,
            short_addr,
            buf_size,
            short_addr
        );
        Some(ApiHookResult::callee(3, Some((bytes.len() - 1) as i32)))
    }

    // API: DWORD GetFullPathNameA(LPCSTR lpFileName, DWORD nBufferLength, LPSTR lpBuffer, LPSTR *lpFilePart)
    // 역할: 지정된 파일의 전체 경로와 파일 이름을 구함
    pub fn get_full_path_name_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let name_addr = uc.read_arg(0);
        let name = if name_addr != 0 {
            uc.read_euc_kr(name_addr as u64)
        } else {
            String::new()
        };
        let buf_size = uc.read_arg(1);
        let buf_addr = uc.read_arg(2);
        let file_part_addr = uc.read_arg(3);
        let full = format!("C:\\4Leaf\\{}\0", name);
        uc.mem_write(buf_addr as u64, full.as_bytes()).unwrap();
        if file_part_addr != 0 {
            uc.write_u32(file_part_addr as u64, buf_addr as u32);
        }
        crate::emu_log!(
            "[KERNEL32] GetFullPathNameA(\"{}\", {}, {:#x}, {:#x}) -> {:#x}",
            name,
            buf_size,
            buf_addr,
            file_part_addr,
            buf_addr
        );
        Some(ApiHookResult::callee(4, Some((full.len() - 1) as i32)))
    }

    // API: DWORD GetLongPathNameA(LPCSTR lpszShortPath, LPSTR lpszLongPath, DWORD cchBuffer)
    // 역할: 지정된 경로의 원래 긴 경로 형태를 가져옴
    pub fn get_long_path_name_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let short_addr = uc.read_arg(0);
        let short_name = if short_addr != 0 {
            uc.read_euc_kr(short_addr as u64)
        } else {
            String::new()
        };
        let long_addr = uc.read_arg(1);
        let buf_size = uc.read_arg(2);
        let mut bytes = short_name.as_bytes().to_vec();
        bytes.push(0);
        if long_addr != 0 {
            uc.mem_write(long_addr as u64, &bytes).unwrap();
        }
        crate::emu_log!(
            "[KERNEL32] GetLongPathNameA(\"{}\", {:#x}, {}) -> {:#x}",
            short_name,
            long_addr,
            buf_size,
            long_addr
        );
        Some(ApiHookResult::callee(3, Some((bytes.len() - 1) as i32)))
    }

    // API: BOOL SetFileTime(HANDLE hFile, const FILETIME *lpCreationTime, const FILETIME *lpLastAccessTime, const FILETIME *lpLastWriteTime)
    // 역할: 지정된 파일의 날짜 및 시간 정보를 지정
    pub fn set_file_time(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hfile = uc.read_arg(0);
        let creation_time = uc.read_arg(1);
        let last_access_time = uc.read_arg(2);
        let last_write_time = uc.read_arg(3);
        crate::emu_log!(
            "[KERNEL32] SetFileTime({:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 1",
            hfile,
            creation_time,
            last_access_time,
            last_write_time
        );
        Some(ApiHookResult::callee(4, Some(1)))
    }

    // =========================================================
    // Handle function dispatch
    // =========================================================

    /// 대기 중인 에뮬레이션 스레드들을 각각 QUANTUM 명령어만큼 실행하는 협력적 스케줄러
    pub(crate) fn schedule_threads(uc: &mut Unicorn<Win32Context>) {
        let has_threads = {
            let ctx = uc.get_data();
            ctx.threads.lock().unwrap().iter().any(|t| t.alive)
        };
        if !has_threads {
            return;
        }

        // 메인 스레드 레지스터 저장
        let caller_tid = uc.get_data().current_thread_idx.load(Ordering::SeqCst);
        let m_eip = uc.reg_read(RegisterX86::EIP).unwrap_or(0) as u32;
        let m_eax = uc.reg_read(RegisterX86::EAX).unwrap_or(0) as u32;
        let m_ecx = uc.reg_read(RegisterX86::ECX).unwrap_or(0) as u32;
        let m_edx = uc.reg_read(RegisterX86::EDX).unwrap_or(0) as u32;
        let m_ebx = uc.reg_read(RegisterX86::EBX).unwrap_or(0) as u32;
        let m_esp = uc.reg_read(RegisterX86::ESP).unwrap_or(0) as u32;
        let m_ebp = uc.reg_read(RegisterX86::EBP).unwrap_or(0) as u32;
        let m_esi = uc.reg_read(RegisterX86::ESI).unwrap_or(0) as u32;
        let m_edi = uc.reg_read(RegisterX86::EDI).unwrap_or(0) as u32;

        let count = uc.get_data().threads.lock().unwrap().len();
        for i in 0..count {
            let t_info = {
                let ctx = uc.get_data();
                ctx.threads.lock().unwrap().get(i).cloned()
            };
            let t = match t_info {
                Some(t) if t.alive => {
                    if t.eip == 0 {
                        let ctx = uc.get_data();
                        if let Some(thread) = ctx.threads.lock().unwrap().get_mut(i) {
                            thread.alive = false;
                        }
                        crate::emu_log!(
                            "[KERNEL32] Thread (handle={:#x}, id={}) has EIP=0 and will be terminated",
                            t.handle,
                            t.thread_id
                        );
                        continue;
                    }
                    if t.terminate_requested {
                        // 종료 요청 시 yield 무시하고 진행 (이후 alive check에서 정리됨)
                        t
                    } else if t.suspended {
                        continue; // 일시 중단된 스레드는 ResumeThread 전까지 건너뜀
                    } else if let Some(res_time) = t.resume_time {
                        if Instant::now() < res_time {
                            continue; // 아직 대기 중
                        }
                        t
                    } else {
                        t
                    }
                }
                _ => continue,
            };

            // 스레드 컨텍스트 표시 (Sleep/Wait 호출 시 재귀 방지)
            uc.get_data()
                .current_thread_idx
                .store(t.thread_id, Ordering::SeqCst);

            // crate::emu_log!(
            //     "[*] Scheduling thread id={} at EIP={:#x}",
            //     t.thread_id,
            //     t.eip
            // );

            // 스레드 레지스터 복원
            let _ = uc.reg_write(RegisterX86::EAX, t.eax as u64);
            let _ = uc.reg_write(RegisterX86::ECX, t.ecx as u64);
            let _ = uc.reg_write(RegisterX86::EDX, t.edx as u64);
            let _ = uc.reg_write(RegisterX86::EBX, t.ebx as u64);
            let _ = uc.reg_write(RegisterX86::ESP, t.esp as u64);
            let _ = uc.reg_write(RegisterX86::EBP, t.ebp as u64);
            let _ = uc.reg_write(RegisterX86::ESI, t.esi as u64);
            let _ = uc.reg_write(RegisterX86::EDI, t.edi as u64);

            // 스레드를 QUANTUM 명령어만큼 실행
            const QUANTUM: usize = 200_000;
            let res = uc.emu_start(t.eip as u64, EXIT_ADDRESS as u64, 0, QUANTUM);
            let new_eip_after_run = uc.reg_read(RegisterX86::EIP).unwrap_or(0) as u32;
            if let Err(e) = res {
                if new_eip_after_run == 0 {
                    crate::emu_log!(
                        "[KERNEL32] Thread id={} stopped with EIP=0 and will be terminated",
                        t.thread_id
                    );
                } else {
                    crate::emu_log!("[!] Thread id={} emu_start failed: {:?}", t.thread_id, e);
                }
            } else {
                // crate::emu_log!("[*] Thread id={} quantum finished", t.thread_id);
            }

            // 스레드 레지스터 저장
            let new_eip = new_eip_after_run;
            let new_eax = uc.reg_read(RegisterX86::EAX).unwrap_or(0) as u32;
            // crate::emu_log!(
            //     "[*] Thread id={} stopped at EIP={:#x}",
            //     t.thread_id,
            //     new_eip
            // );
            let new_ecx = uc.reg_read(RegisterX86::ECX).unwrap_or(0) as u32;
            let new_edx = uc.reg_read(RegisterX86::EDX).unwrap_or(0) as u32;
            let new_ebx = uc.reg_read(RegisterX86::EBX).unwrap_or(0) as u32;
            let new_esp = uc.reg_read(RegisterX86::ESP).unwrap_or(0) as u32;
            let new_ebp = uc.reg_read(RegisterX86::EBP).unwrap_or(0) as u32;
            let new_esi = uc.reg_read(RegisterX86::ESI).unwrap_or(0) as u32;
            let new_edi = uc.reg_read(RegisterX86::EDI).unwrap_or(0) as u32;

            // terminate_requested 플래그 또는 EXIT_ADDRESS 도달 시 스레드 종료
            let terminate_requested = {
                let ctx = uc.get_data();
                ctx.threads
                    .lock()
                    .unwrap()
                    .get(i)
                    .map(|t| t.terminate_requested)
                    .unwrap_or(false)
            };
            let alive = !terminate_requested && new_eip != EXIT_ADDRESS as u32 && new_eip != 0;
            {
                let ctx = uc.get_data();
                let mut threads = ctx.threads.lock().unwrap();
                if let Some(thread) = threads.get_mut(i) {
                    thread.alive = alive;
                    thread.eip = new_eip;
                    thread.eax = new_eax;
                    thread.ecx = new_ecx;
                    thread.edx = new_edx;
                    thread.ebx = new_ebx;
                    thread.esp = new_esp;
                    thread.ebp = new_ebp;
                    thread.esi = new_esi;
                    thread.edi = new_edi;
                }
            }
            if !alive {
                crate::emu_log!(
                    "[KERNEL32] Thread (handle={:#x}, id={}) finished",
                    t.handle,
                    t.thread_id
                );
            }
        }

        Self::cleanup_finished_threads(uc.get_data());

        // 호출자 스레드 컨텍스트로 복귀
        uc.get_data()
            .current_thread_idx
            .store(caller_tid, Ordering::SeqCst);

        // 메인 스레드 레지스터 복원
        let _ = uc.reg_write(RegisterX86::EIP, m_eip as u64);
        let _ = uc.reg_write(RegisterX86::EAX, m_eax as u64);
        let _ = uc.reg_write(RegisterX86::ECX, m_ecx as u64);
        let _ = uc.reg_write(RegisterX86::EDX, m_edx as u64);
        let _ = uc.reg_write(RegisterX86::EBX, m_ebx as u64);
        let _ = uc.reg_write(RegisterX86::ESP, m_esp as u64);
        let _ = uc.reg_write(RegisterX86::EBP, m_ebp as u64);
        let _ = uc.reg_write(RegisterX86::ESI, m_esi as u64);
        let _ = uc.reg_write(RegisterX86::EDI, m_edi as u64);
    }

    // API: HANDLE CreateThread(LPSECURITY_ATTRIBUTES, SIZE_T, LPTHREAD_START_ROUTINE, LPVOID, DWORD, LPDWORD)
    // 역할: 새로운 스레드를 생성하고 스케줄 큐에 등록
    pub fn create_thread(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let _lp_thread_attributes = uc.read_arg(0);
        let _dw_stack_size = uc.read_arg(1);
        let lp_start_address = uc.read_arg(2);
        let lp_parameter = uc.read_arg(3);
        let dw_creation_flags = uc.read_arg(4);
        let lp_thread_id = uc.read_arg(5);

        let allowed_creation_flags = CREATE_SUSPENDED | STACK_SIZE_PARAM_IS_A_RESERVATION;
        let unsupported = Self::unsupported_flag_bits(dw_creation_flags, allowed_creation_flags);
        if unsupported != 0 {
            uc.get_data()
                .last_error
                .store(ERROR_INVALID_PARAMETER, Ordering::SeqCst);
            crate::emu_log!(
                "[KERNEL32] CreateThread unsupported creation_flags={:#x} unsupported={:#x}",
                dw_creation_flags,
                unsupported
            );
            return Some(ApiHookResult::callee(6, Some(0)));
        }

        let suspended = dw_creation_flags & CREATE_SUSPENDED != 0;

        const THREAD_STACK_SIZE: u32 = 512 * 1024; // 512KB

        // 스레드 전용 스택 할당 (힙에서)
        let stack_alloc = uc.malloc(THREAD_STACK_SIZE as usize) as u32;
        let stack_top = stack_alloc + THREAD_STACK_SIZE;

        // 초기 스택 설정: [ESP] = EXIT_ADDRESS (리턴 주소), [ESP+4] = lp_parameter (인자)
        uc.write_u32((stack_top - 8) as u64, EXIT_ADDRESS as u32);
        uc.write_u32((stack_top - 4) as u64, lp_parameter);
        let initial_esp = stack_top - 8;

        let handle = uc.get_data().alloc_handle();
        let thread_id = uc.get_data().alloc_handle();

        if lp_thread_id != 0 {
            uc.write_u32(lp_thread_id as u64, thread_id);
        }

        let new_thread = EmulatedThread {
            handle,
            thread_id,
            stack_alloc,
            stack_size: THREAD_STACK_SIZE,
            eax: 0,
            ecx: 0,
            edx: 0,
            ebx: 0,
            esp: initial_esp,
            ebp: initial_esp,
            esi: 0,
            edi: 0,
            eip: lp_start_address,
            alive: true,
            terminate_requested: false,
            suspended,
            resume_time: None,
            wait_deadline: None,
        };

        uc.get_data().threads.lock().unwrap().push(new_thread);

        crate::emu_log!(
            "[KERNEL32] CreateThread(entry={:#x}, param={:#x}, flags={:#x}, suspended={}) -> handle={:#x}, id={:#x}",
            lp_start_address,
            lp_parameter,
            dw_creation_flags,
            suspended,
            handle,
            thread_id
        );
        Some(ApiHookResult::callee(6, Some(handle as i32)))
    }

    /// 함수명 기준 `KERNEL32.dll` API 구현체
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        match func_name {
            "TlsAlloc" => KERNEL32::tls_alloc(uc),
            "TlsFree" => KERNEL32::tls_free(uc),
            "TlsGetValue" => KERNEL32::tls_get_value(uc),
            "TlsSetValue" => KERNEL32::tls_set_value(uc),
            "Sleep" => KERNEL32::sleep(uc),
            "GetCurrentThreadId" => KERNEL32::get_current_thread_id(uc),
            "WaitForSingleObject" => KERNEL32::wait_for_single_object(uc),
            "TerminateThread" => KERNEL32::terminate_thread(uc),
            "CloseHandle" => KERNEL32::close_handle(uc),
            "DuplicateHandle" => KERNEL32::duplicate_handle(uc),
            "GetCurrentThread" => KERNEL32::get_current_thread(uc),
            "GetCurrentProcess" => KERNEL32::get_current_process(uc),
            "FormatMessageA" => KERNEL32::format_message_a(uc),
            "GetLastError" => KERNEL32::get_last_error(uc),
            "CreateEventA" => KERNEL32::create_event_a(uc),
            "SetEvent" => KERNEL32::set_event(uc),
            "PulseEvent" => KERNEL32::pulse_event(uc),
            "ResetEvent" => KERNEL32::reset_event(uc),
            "InitializeCriticalSection" => KERNEL32::initialize_critical_section(uc),
            "DeleteCriticalSection" => KERNEL32::delete_critical_section(uc),
            "EnterCriticalSection" => KERNEL32::enter_critical_section(uc),
            "LeaveCriticalSection" => KERNEL32::leave_critical_section(uc),
            "OutputDebugStringA" => KERNEL32::output_debug_string_a(uc),
            "DisableThreadLibraryCalls" => KERNEL32::disable_thread_library_calls(uc),
            "lstrlenA" => KERNEL32::lstrlen_a(uc),
            "MulDiv" => KERNEL32::mul_div(uc),
            "lstrcpynA" => KERNEL32::lstrcpyn_a(uc),
            "SetLastError" => KERNEL32::set_last_error(uc),
            "GetModuleHandleA" => KERNEL32::get_module_handle_a(uc),
            "InterlockedExchange" => KERNEL32::interlocked_exchange(uc),
            "GetTickCount" => KERNEL32::get_tick_count(uc),
            "lstrcpyA" => KERNEL32::lstrcpy_a(uc),
            "lstrcatA" => KERNEL32::lstrcat_a(uc),
            "GlobalAlloc" => KERNEL32::global_alloc(uc),
            "GlobalLock" => KERNEL32::global_lock(uc),
            "GlobalUnlock" => KERNEL32::global_unlock(uc),
            "GlobalFree" => KERNEL32::global_free(uc),
            "SetThreadPriority" => KERNEL32::set_thread_priority(uc),
            "FreeLibrary" => KERNEL32::free_library(uc),
            "FindNextFileA" => KERNEL32::find_next_file_a(uc),
            "FindClose" => KERNEL32::find_close(uc),
            "GetFileAttributesA" => KERNEL32::get_file_attributes_a(uc),
            "RemoveDirectoryA" => KERNEL32::remove_directory_a(uc),
            "GetTempPathA" => KERNEL32::get_temp_path_a(uc),
            "SystemTimeToFileTime" => KERNEL32::system_time_to_file_time(uc),
            "WaitForMultipleObjects" => KERNEL32::wait_for_multiple_objects(uc),
            "GetShortPathNameA" => KERNEL32::get_short_path_name_a(uc),
            "lstrcmpA" => KERNEL32::lstrcmp_a(uc),
            "GetLocalTime" => KERNEL32::get_local_time(uc),
            "CreateDirectoryA" => KERNEL32::create_directory_a(uc),
            "DeleteFileA" => KERNEL32::delete_file_a(uc),
            "CopyFileA" => KERNEL32::copy_file_a(uc),
            "ReleaseMutex" => KERNEL32::release_mutex(uc),
            "CreateProcessA" => KERNEL32::create_process_a(uc),
            "CreateMutexA" => KERNEL32::create_mutex_a(uc),
            "FindFirstFileA" => KERNEL32::find_first_file_a(uc),
            "GetFullPathNameA" => KERNEL32::get_full_path_name_a(uc),
            "GetModuleFileNameA" => KERNEL32::get_module_file_name_a(uc),
            "GetLongPathNameA" => KERNEL32::get_long_path_name_a(uc),
            "SetFileTime" => KERNEL32::set_file_time(uc),
            "CreateFileA" => KERNEL32::create_file_a(uc),
            "GetProcAddress" => KERNEL32::get_proc_address(uc),
            "LoadLibraryA" => KERNEL32::load_library_a(uc),
            "SetFileAttributesA" => KERNEL32::set_file_attributes_a(uc),
            "CreateThread" => KERNEL32::create_thread(uc),
            _ => {
                crate::emu_log!("[!] KERNEL32 Unhandled: {}", func_name);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_thread(thread_id: u32, alive: bool) -> EmulatedThread {
        EmulatedThread {
            handle: thread_id + 0x1000,
            thread_id,
            stack_alloc: 0,
            stack_size: 0,
            eax: 0,
            ecx: 0,
            edx: 0,
            ebx: 0,
            esp: 0,
            ebp: 0,
            esi: 0,
            edi: 0,
            eip: 0,
            alive,
            terminate_requested: !alive,
            suspended: false,
            resume_time: None,
            wait_deadline: None,
        }
    }

    #[test]
    fn cleanup_finished_threads_prunes_dead_entries_and_tls() {
        let ctx = Win32Context::new(None);
        {
            let mut threads = ctx.threads.lock().unwrap();
            threads.push(sample_thread(0x2001, true));
            threads.push(sample_thread(0x2002, false));
        }
        {
            let mut tls_slots = ctx.tls_slots.lock().unwrap();
            tls_slots.insert(0x2001, std::collections::HashMap::new());
            tls_slots.insert(0x2002, std::collections::HashMap::new());
        }

        KERNEL32::cleanup_finished_threads(&ctx);

        let threads = ctx.threads.lock().unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].thread_id, 0x2001);
        drop(threads);

        let tls_slots = ctx.tls_slots.lock().unwrap();
        assert!(tls_slots.contains_key(&0x2001));
        assert!(!tls_slots.contains_key(&0x2002));
    }
}
