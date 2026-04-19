mod file;
mod memory;
mod misc;
mod module;
mod sync;
mod thread;

use crate::dll::win32::{ApiHookResult, Win32Context};
use std::time::{Duration, Instant};
use unicorn_engine::Unicorn;

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
/// 명시적인 타임아웃이 있는 대기 API를 다시 확인할 기본 폴링 간격입니다.
const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(5);
/// 무한 대기 계열을 다시 확인할 기본 폴링 간격입니다.
///
/// UI 스레드가 메시지를 넣으면 wake_emulator()가 main_resume_time을 즉시 클리어하므로
/// 이 값은 외부 이벤트 없이 폴백으로만 쓰입니다.
const WAIT_POLL_INTERVAL_IDLE: Duration = Duration::from_millis(5);

/// `KERNEL32.dll` 프록시 구현 모듈
///
/// Windows 코어 서브시스템으로, 스레드, 메모리, 모듈 핸들, 뮤텍스(Mutex), 이벤트(Event) 등을 가상으로 프로비저닝
pub struct KERNEL32 {}

impl KERNEL32 {
    /// 종료된 가상 스레드 엔트리와 해당 TLS 슬롯을 정리합니다.
    #[allow(dead_code)]
    fn cleanup_finished_threads(ctx: &Win32Context) {
        thread::cleanup_finished_threads_impl(ctx);
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
            .unwrap_or(now + WAIT_POLL_INTERVAL_IDLE);

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

    /// 대기 핸들 배열에서 즉시 준비된 첫 번째 항목의 인덱스를 반환합니다.
    pub(crate) fn first_ready_wait_handle(ctx: &Win32Context, handles: &[u32]) -> Option<usize> {
        handles.iter().enumerate().find_map(|(index, handle)| {
            if sync::try_consume_signaled_event(ctx, *handle) || sync::poll_wsa_event(ctx, *handle)
            {
                Some(index)
            } else {
                None
            }
        })
    }

    /// 대기 중인 에뮬레이션 스레드들을 각각 QUANTUM 명령어만큼 실행하는 협력적 스케줄러
    pub(crate) fn schedule_threads(uc: &mut Unicorn<Win32Context>) {
        thread::schedule_threads_impl(uc);
    }

    /// 함수명 기준 `KERNEL32.dll` API 구현체
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        match func_name {
            "TlsAlloc" => thread::tls_alloc(uc),
            "TlsFree" => thread::tls_free(uc),
            "TlsGetValue" => thread::tls_get_value(uc),
            "TlsSetValue" => thread::tls_set_value(uc),
            "Sleep" => thread::sleep(uc),
            "GetCurrentThreadId" => thread::get_current_thread_id(uc),
            "GetCurrentThread" => thread::get_current_thread(uc),
            "GetCurrentProcess" => thread::get_current_process(uc),
            "GetCurrentProcessId" => thread::get_current_process_id(uc),
            "TerminateThread" => thread::terminate_thread(uc),
            "SetThreadPriority" => thread::set_thread_priority(uc),
            "CreateProcessA" => thread::create_process_a(uc),
            "CreateThread" => thread::create_thread(uc),
            "WaitForSingleObject" => sync::wait_for_single_object(uc),
            "WaitForMultipleObjects" => sync::wait_for_multiple_objects(uc),
            "CreateEventA" => sync::create_event_a(uc),
            "SetEvent" => sync::set_event(uc),
            "PulseEvent" => sync::pulse_event(uc),
            "ResetEvent" => sync::reset_event(uc),
            "InitializeCriticalSection" => sync::initialize_critical_section(uc),
            "DeleteCriticalSection" => sync::delete_critical_section(uc),
            "EnterCriticalSection" => sync::enter_critical_section(uc),
            "LeaveCriticalSection" => sync::leave_critical_section(uc),
            "CreateMutexA" => sync::create_mutex_a(uc),
            "ReleaseMutex" => sync::release_mutex(uc),
            "InterlockedExchange" => sync::interlocked_exchange(uc),
            "GetModuleHandleA" => module::get_module_handle_a(uc),
            "GetModuleFileNameA" => module::get_module_file_name_a(uc),
            "LoadLibraryA" => module::load_library_a(uc),
            "FreeLibrary" => module::free_library(uc),
            "GetProcAddress" => module::get_proc_address(uc),
            "DisableThreadLibraryCalls" => module::disable_thread_library_calls(uc),
            "GlobalAlloc" => memory::global_alloc(uc),
            "GlobalLock" => memory::global_lock(uc),
            "GlobalUnlock" => memory::global_unlock(uc),
            "GlobalFree" => memory::global_free(uc),
            "CreateFileA" => file::create_file_a(uc),
            "FindFirstFileA" => file::find_first_file_a(uc),
            "FindNextFileA" => file::find_next_file_a(uc),
            "FindClose" => file::find_close(uc),
            "GetFileAttributesA" => file::get_file_attributes_a(uc),
            "SetFileAttributesA" => file::set_file_attributes_a(uc),
            "RemoveDirectoryA" => file::remove_directory_a(uc),
            "CreateDirectoryA" => file::create_directory_a(uc),
            "DeleteFileA" => file::delete_file_a(uc),
            "CopyFileA" => file::copy_file_a(uc),
            "GetTempPathA" => file::get_temp_path_a(uc),
            "GetShortPathNameA" => file::get_short_path_name_a(uc),
            "GetFullPathNameA" => file::get_full_path_name_a(uc),
            "GetLongPathNameA" => file::get_long_path_name_a(uc),
            "SetFileTime" => file::set_file_time(uc),
            "CloseHandle" => misc::close_handle(uc),
            "DuplicateHandle" => misc::duplicate_handle(uc),
            "GetLastError" => misc::get_last_error(uc),
            "SetLastError" => misc::set_last_error(uc),
            "FormatMessageA" => misc::format_message_a(uc),
            "OutputDebugStringA" => misc::output_debug_string_a(uc),
            "lstrlenA" => misc::lstrlen_a(uc),
            "lstrcpyA" => misc::lstrcpy_a(uc),
            "lstrcpynA" => misc::lstrcpyn_a(uc),
            "lstrcatA" => misc::lstrcat_a(uc),
            "lstrcmpA" => misc::lstrcmp_a(uc),
            "MulDiv" => misc::mul_div(uc),
            "GetTickCount" => misc::get_tick_count(uc),
            "GetLocalTime" => misc::get_local_time(uc),
            "SystemTimeToFileTime" => misc::system_time_to_file_time(uc),
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
    use crate::dll::win32::{EmulatedThread, VirtualSocket, WsaEventEntry};

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
        let finished_stack = ctx.alloc_heap_block(0x1000).unwrap();
        {
            let mut threads = ctx.threads.lock().unwrap();
            threads.push(sample_thread(0x2001, true));
            let mut finished = sample_thread(0x2002, false);
            finished.stack_alloc = finished_stack;
            finished.stack_size = 0x1000;
            threads.push(finished);
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
        drop(tls_slots);

        let reused_stack = ctx.alloc_heap_block(0x800).unwrap();
        assert_eq!(reused_stack, finished_stack);
    }

    #[test]
    fn first_ready_wait_handle_detects_buffered_wsa_events() {
        let ctx = Win32Context::new(None);
        let sock = 0x2200;
        let event = 0x3300;

        ctx.tcp_sockets.lock().unwrap().insert(
            sock,
            VirtualSocket {
                af: 2,
                sock_type: 1,
                protocol: 6,
                chan_tx: None,
                chan_rx: None,
                connected: true,
                recv_buf: vec![0x41],
                non_blocking: false,
                remote_addr: None,
            },
        );
        ctx.wsa_event_map.lock().unwrap().insert(
            event,
            WsaEventEntry {
                socket: sock,
                interest: 0x01,
                pending: 0,
            },
        );

        let ready = KERNEL32::first_ready_wait_handle(&ctx, &[0x1111, event]);

        assert_eq!(ready, Some(1));
        let pending = ctx
            .wsa_event_map
            .lock()
            .unwrap()
            .get(&event)
            .map(|entry| entry.pending)
            .unwrap_or(0);
        assert_eq!(pending & 0x01, 0x01);
    }
}
