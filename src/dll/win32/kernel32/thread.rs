use crate::{
    dll::win32::{ApiHookResult, EmulatedThread, Win32Context},
    helper::{EXIT_ADDRESS, UnicornHelper},
};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use unicorn_engine::{RegisterX86, Unicorn};

use super::{
    CREATE_SUSPENDED, ERROR_INVALID_PARAMETER, KERNEL32, STACK_SIZE_PARAM_IS_A_RESERVATION,
};

// =========================================================
// TLS (Thread Local Storage)
// =========================================================
// API: DWORD TlsAlloc(void)
// 역할: 새 TLS(Thread Local Storage) 인덱스를 할당
pub(super) fn tls_alloc(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let ctx = uc.get_data();
    let index = ctx.tls_counter.fetch_add(1, Ordering::SeqCst);
    crate::emu_log!("[KERNEL32] TlsAlloc() -> DWORD {}", index);
    Some(ApiHookResult::callee(0, Some(index as i32)))
}

// API: BOOL TlsFree(DWORD dwTlsIndex)
// 역할: 지정된 TLS 인덱스를 해제
pub(super) fn tls_free(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn tls_get_value(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn tls_set_value(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn sleep(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn get_current_thread_id(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let tid = uc.get_data().current_thread_idx.load(Ordering::SeqCst);
    let thread_id = if tid == 0 { 1u32 } else { tid };
    crate::emu_log!("[KERNEL32] GetCurrentThreadId() -> {}", thread_id);
    Some(ApiHookResult::callee(0, Some(thread_id as i32)))
}

// API: HANDLE GetCurrentThread(void)
// 역할: 현재 스레드의 스레드 핸들을 반환
pub(super) fn get_current_thread(_uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    crate::emu_log!("[KERNEL32] GetCurrentThread() -> 0xFFFFFFFE");
    Some(ApiHookResult::callee(0, Some(-2i32))) // pseudo handle
}

// API: HANDLE GetCurrentProcess(void)
// 역할: 현재 프로세스의 의사 핸들(pseudo handle)을 반환
pub(super) fn get_current_process(_uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    crate::emu_log!("[KERNEL32] GetCurrentProcess() -> 0xFFFFFFFF");
    Some(ApiHookResult::callee(0, Some(-1i32))) // pseudo handle
}

// API: DWORD GetCurrentProcessId(void)
// 역할: 현재 프로세스의 프로세스 식별자를 검색
pub(super) fn get_current_process_id(_uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    crate::emu_log!("[KERNEL32] GetCurrentProcessId() -> 1234");
    Some(ApiHookResult::callee(0, Some(1234)))
}

// API: BOOL TerminateThread(HANDLE hThread, DWORD dwExitCode)
// 역할: 스레드를 강제 종료
pub(super) fn terminate_thread(_uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn set_thread_priority(_uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let h_thread = _uc.read_arg(0);
    let n_priority = _uc.read_arg(1);
    crate::emu_log!(
        "[KERNEL32] SetThreadPriority({:#x}, {}) -> BOOL 1",
        h_thread,
        n_priority
    );
    Some(ApiHookResult::callee(2, Some(1)))
}

// API: BOOL CreateProcessA(LPCSTR lpApplicationName, LPSTR lpCommandLine, ...)
// 역할: 새로운 프로세스와 그 기본 스레드를 생성
pub(super) fn create_process_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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

// API: HANDLE CreateThread(LPSECURITY_ATTRIBUTES, SIZE_T, LPTHREAD_START_ROUTINE, LPVOID, DWORD, LPDWORD)
// 역할: 새로운 스레드를 생성하고 스케줄 큐에 등록
pub(super) fn create_thread(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let _lp_thread_attributes = uc.read_arg(0);
    let _dw_stack_size = uc.read_arg(1);
    let lp_start_address = uc.read_arg(2);
    let lp_parameter = uc.read_arg(3);
    let dw_creation_flags = uc.read_arg(4);
    let lp_thread_id = uc.read_arg(5);

    let allowed_creation_flags = CREATE_SUSPENDED | STACK_SIZE_PARAM_IS_A_RESERVATION;
    let unsupported = unsupported_flag_bits(dw_creation_flags, allowed_creation_flags);
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

/// 지원하지 않는 플래그 비트를 계산합니다.
fn unsupported_flag_bits(flags: u32, supported_mask: u32) -> u32 {
    flags & !supported_mask
}

/// 대기 중인 에뮬레이션 스레드들을 각각 QUANTUM 명령어만큼 실행하는 협력적 스케줄러
pub(super) fn schedule_threads_impl(uc: &mut Unicorn<Win32Context>) {
    // 스레드 목록을 한 번만 잠그고 실행할 스레드 스냅샷을 수집합니다.
    // 이전에는 스레드당 5~7회 lock/unlock이 발생했으나, 이제는 루프 전후 각 1회로 줄입니다.
    let runnable: Vec<(usize, EmulatedThread)> = {
        let ctx = uc.get_data();
        let mut threads = ctx.threads.lock().unwrap();
        let now = Instant::now();
        let mut result = Vec::new();
        for (i, t) in threads.iter_mut().enumerate() {
            if !t.alive {
                continue;
            }
            if t.eip == 0 {
                t.alive = false;
                crate::emu_log!(
                    "[KERNEL32] Thread (handle={:#x}, id={}) has EIP=0 and will be terminated",
                    t.handle,
                    t.thread_id
                );
                continue;
            }
            if t.terminate_requested {
                result.push((i, t.clone()));
            } else if t.suspended {
                continue;
            } else if let Some(res_time) = t.resume_time {
                if now < res_time {
                    continue;
                }
                result.push((i, t.clone()));
            } else {
                result.push((i, t.clone()));
            }
        }
        result
    };

    if runnable.is_empty() {
        return;
    }

    // 메인 스레드 레지스터를 일괄 저장
    let caller_tid = uc.get_data().current_thread_idx.load(Ordering::SeqCst);
    let saved_regs = read_regs(uc);

    for (i, t) in &runnable {
        uc.get_data()
            .current_thread_idx
            .store(t.thread_id, Ordering::SeqCst);

        // 스레드 레지스터 일괄 복원 (EIP는 emu_start에서 설정하므로 0으로 채움)
        write_regs(
            uc,
            [0, t.eax, t.ecx, t.edx, t.ebx, t.esp, t.ebp, t.esi, t.edi],
        );

        const QUANTUM: usize = 200_000;
        let res = uc.emu_start(t.eip as u64, EXIT_ADDRESS as u64, 0, QUANTUM);
        let new_eip = uc.reg_read(RegisterX86::EIP).unwrap_or(0) as u32;

        if let Err(e) = res {
            if new_eip == 0 {
                crate::emu_log!(
                    "[KERNEL32] Thread id={} stopped with EIP=0 and will be terminated",
                    t.thread_id
                );
            } else {
                crate::emu_log!("[!] Thread id={} emu_start failed: {:?}", t.thread_id, e);
            }
        }

        // 스레드 레지스터를 일괄 읽기
        let new_regs = read_regs(uc);

        // terminate_requested는 emu_start 도중 변경될 수 있으므로 여기서 확인
        let terminate_requested = {
            let ctx = uc.get_data();
            ctx.threads
                .lock()
                .unwrap()
                .get(*i)
                .map(|t| t.terminate_requested)
                .unwrap_or(false)
        };
        let alive = !terminate_requested && new_eip != EXIT_ADDRESS as u32 && new_eip != 0;

        {
            let ctx = uc.get_data();
            let mut threads = ctx.threads.lock().unwrap();
            if let Some(thread) = threads.get_mut(*i) {
                thread.alive = alive;
                thread.eip = new_eip;
                // read_regs: [0]=EIP, [1]=EAX, [2]=ECX, ..., [8]=EDI
                thread.eax = new_regs[1];
                thread.ecx = new_regs[2];
                thread.edx = new_regs[3];
                thread.ebx = new_regs[4];
                thread.esp = new_regs[5];
                thread.ebp = new_regs[6];
                thread.esi = new_regs[7];
                thread.edi = new_regs[8];
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

    cleanup_finished_threads_impl(uc.get_data());

    // 호출자 스레드 컨텍스트로 복귀
    uc.get_data()
        .current_thread_idx
        .store(caller_tid, Ordering::SeqCst);
    write_regs(uc, saved_regs);
    let _ = uc.reg_write(RegisterX86::EIP, saved_regs[0] as u64);
}

/// 현재 CPU 레지스터를 [EIP, EAX, ECX, EDX, EBX, ESP, EBP, ESI, EDI] 배열로 일괄 읽기
#[inline]
fn read_regs(uc: &mut Unicorn<Win32Context>) -> [u32; 9] {
    [
        uc.reg_read(RegisterX86::EIP).unwrap_or(0) as u32,
        uc.reg_read(RegisterX86::EAX).unwrap_or(0) as u32,
        uc.reg_read(RegisterX86::ECX).unwrap_or(0) as u32,
        uc.reg_read(RegisterX86::EDX).unwrap_or(0) as u32,
        uc.reg_read(RegisterX86::EBX).unwrap_or(0) as u32,
        uc.reg_read(RegisterX86::ESP).unwrap_or(0) as u32,
        uc.reg_read(RegisterX86::EBP).unwrap_or(0) as u32,
        uc.reg_read(RegisterX86::ESI).unwrap_or(0) as u32,
        uc.reg_read(RegisterX86::EDI).unwrap_or(0) as u32,
    ]
}

/// [EAX, ECX, EDX, EBX, ESP, EBP, ESI, EDI]를 일괄 기록 (EIP 제외)
#[inline]
fn write_regs(uc: &mut Unicorn<Win32Context>, r: [u32; 9]) {
    // r[0] = EIP — 별도 처리 필요
    let _ = uc.reg_write(RegisterX86::EAX, r[1] as u64);
    let _ = uc.reg_write(RegisterX86::ECX, r[2] as u64);
    let _ = uc.reg_write(RegisterX86::EDX, r[3] as u64);
    let _ = uc.reg_write(RegisterX86::EBX, r[4] as u64);
    let _ = uc.reg_write(RegisterX86::ESP, r[5] as u64);
    let _ = uc.reg_write(RegisterX86::EBP, r[6] as u64);
    let _ = uc.reg_write(RegisterX86::ESI, r[7] as u64);
    let _ = uc.reg_write(RegisterX86::EDI, r[8] as u64);
}

/// 종료된 가상 스레드 엔트리와 해당 TLS 슬롯을 정리합니다.
pub(super) fn cleanup_finished_threads_impl(ctx: &Win32Context) {
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
