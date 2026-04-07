use crate::{
    dll::win32::{ApiHookResult, EmulatedThread, Win32Context},
    helper::{EXIT_ADDRESS, UnicornHelper},
};
use std::sync::atomic::Ordering;
use unicorn_engine::{RegisterX86, Unicorn};

// =========================================================
// Conversion
// =========================================================
// API: int atoi(const char* str)
// 역할: 문자열을 정수로 변환
pub(super) fn atoi(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let s_addr = uc.read_arg(0);
    let s = if s_addr != 0 {
        uc.read_euc_kr(s_addr as u64)
    } else {
        String::new()
    };
    let result = s.trim().parse::<i32>().unwrap_or(0);
    crate::emu_log!("[MSVCRT] atoi(\"{}\") -> int {}", s, result);
    Some(ApiHookResult::callee(1, Some(result)))
}

// API: char* _itoa(int value, char* str, int radix)
// 역할: 정수를 문자열로 변환
pub(super) fn _itoa(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let value = uc.read_arg(0) as i32;
    let buf_addr = uc.read_arg(1);
    let radix = uc.read_arg(2);
    let s = match radix {
        16 => format!("{:x}\0", value),
        8 => format!("{:o}\0", value),
        _ => format!("{}\0", value),
    };
    uc.mem_write(buf_addr as u64, s.as_bytes()).unwrap();
    crate::emu_log!(
        "[MSVCRT] _itoa({}, {:#x}, {}) -> char* {:#x}=\"{}\"",
        value,
        buf_addr,
        radix,
        buf_addr,
        &s[..s.len() - 1]
    );
    Some(ApiHookResult::callee(3, Some(buf_addr as i32)))
}

// API: unsigned long strtoul(const char* str, char** endptr, int base)
// 역할: 문자열을 무부호 장정수로 변환
pub(super) fn strtoul(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let s_addr = uc.read_arg(0);
    let endptr_addr = uc.read_arg(1);
    let base = uc.read_arg(2);

    let s = if s_addr != 0 {
        uc.read_euc_kr(s_addr as u64)
    } else {
        String::new()
    };

    // Trim leading whitespace
    let trimmed = s.trim_start();
    let offset = s.len() - trimmed.len();

    let (result, consumed) = if trimmed.is_empty() {
        (0, 0)
    } else {
        match u32::from_str_radix(trimmed, base) {
            Ok(val) => (val, trimmed.len()), // This is approximate: from_str_radix expects full match
            Err(_) => {
                // Manual parsing for partial matches
                let mut val: u64 = 0;
                let mut i = 0;
                let b = if base == 0 {
                    if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
                        i = 2;
                        16
                    } else if trimmed.starts_with('0') {
                        i = 1;
                        8
                    } else {
                        10
                    }
                } else {
                    base
                };

                let chars = trimmed.as_bytes();
                while i < chars.len() {
                    let digit = match chars[i] {
                        c @ b'0'..=b'9' => (c - b'0') as u32,
                        c @ b'a'..=b'z' => (c - b'a') as u32 + 10,
                        c @ b'A'..=b'Z' => (c - b'A') as u32 + 10,
                        _ => break,
                    };
                    if digit >= b {
                        break;
                    }
                    val = val * b as u64 + digit as u64;
                    if val > 0xFFFFFFFF {
                        val = 0xFFFFFFFF; // Overflow
                    }
                    i += 1;
                }
                (val as u32, i)
            }
        }
    };

    if endptr_addr != 0 {
        let final_ptr = s_addr + (offset + consumed) as u32;
        uc.write_u32(endptr_addr as u64, final_ptr);
    }

    crate::emu_log!(
        "[MSVCRT] strtoul(\"{}\", {:#x}, {}) -> unsigned long {}",
        s,
        endptr_addr,
        base,
        result
    );
    Some(ApiHookResult::callee(3, Some(result as i32)))
}

// =========================================================
// Time
// =========================================================
// API: time_t time(time_t* timer)
// 역할: 시스템의 현재 시간을 가져옴
pub(super) fn time(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let timer_addr = uc.read_arg(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as u32;
    if timer_addr != 0 {
        uc.write_u32(timer_addr as u64, now);
    }
    crate::emu_log!("[MSVCRT] time({:#x}) -> time_t {:#x}", timer_addr, now);
    Some(ApiHookResult::callee(1, Some(now as i32)))
}

// API: struct tm* localtime(const time_t* timer)
// 역할: 시간을 현지 시간 구조체로 변환
pub(super) fn localtime(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let timer_ptr = uc.read_arg(0);
    let time_val = if timer_ptr != 0 {
        uc.read_u32(timer_ptr as u64)
    } else {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as u32
    };

    // Simple unix time to tm conversion (approximate for dummy)
    let sec = (time_val % 60) as i32;
    let min = ((time_val / 60) % 60) as i32;
    let hour = ((time_val / 3600) % 24) as i32;
    let day = ((time_val / 86400) % 31) as i32 + 1;
    let mon = ((time_val / 2592000) % 12) as i32;
    let year = (time_val / 31536000) as i32 + 70;

    let mut tm_ptr = uc.get_data().tm_struct_ptr.load(Ordering::SeqCst);
    if tm_ptr == 0 {
        tm_ptr = uc.malloc(36) as u32;
        uc.get_data().tm_struct_ptr.store(tm_ptr, Ordering::SeqCst);
    }

    let mut data = [0u32; 9];
    data[0] = sec as u32; // tm_sec
    data[1] = min as u32; // tm_min
    data[2] = hour as u32; // tm_hour
    data[3] = day as u32; // tm_mday
    data[4] = mon as u32; // tm_mon
    data[5] = year as u32; // tm_year
    data[6] = 0; // tm_wday
    data[7] = 0; // tm_yday
    data[8] = 0; // tm_isdst

    for i in 0..9 {
        uc.write_u32((tm_ptr + (i * 4) as u32) as u64, data[i]);
    }

    crate::emu_log!(
        "[MSVCRT] localtime({:#x}:{}) -> struct tm* {:#x}",
        timer_ptr,
        time_val,
        tm_ptr
    );
    Some(ApiHookResult::callee(1, Some(tm_ptr as i32)))
}

// API: time_t mktime(struct tm* timeptr)
// 역할: tm 구조체를 time_t 값으로 변환
pub(super) fn mktime(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let timeptr_addr = uc.read_arg(0);
    if timeptr_addr == 0 {
        return Some(ApiHookResult::callee(1, Some(-1)));
    }

    let sec = uc.read_u32(timeptr_addr as u64);
    let min = uc.read_u32(timeptr_addr as u64 + 4);
    let hour = uc.read_u32(timeptr_addr as u64 + 8);
    let day = uc.read_u32(timeptr_addr as u64 + 12);
    let mon = uc.read_u32(timeptr_addr as u64 + 16);
    let year = uc.read_u32(timeptr_addr as u64 + 20);

    // Very crude conversion
    let t = sec
        + min * 60
        + hour * 3600
        + (day - 1) * 86400
        + mon * 2592000
        + (year - 70) * 31536000;

    crate::emu_log!("[MSVCRT] mktime({:#x}) -> time_t {:#x}", timeptr_addr, t);
    Some(ApiHookResult::callee(1, Some(t as i32)))
}

pub(super) fn _timezone(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let addr = uc.malloc(4);
    uc.write_u32(addr, 32400); // UTC+9
    crate::emu_log!("[MSVCRT] _timezone -> {:#x} (32400)", addr);
    Some(ApiHookResult::callee(0, Some(addr as i32)))
}

// =========================================================
// Random
// =========================================================
// API: int rand(void)
// 역할: 의사 난수를 생성
pub(super) fn rand(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let ctx = uc.get_data();
    let mut state = ctx.rand_state.load(Ordering::SeqCst);
    state = state.wrapping_mul(214013).wrapping_add(2531011);
    ctx.rand_state.store(state, Ordering::SeqCst);
    let val = (state >> 16) & 0x7FFF;
    crate::emu_log!("[MSVCRT] rand() -> int {}", val);
    Some(ApiHookResult::callee(0, Some(val as i32)))
}

// API: void srand(unsigned int seed)
// 역할: 난수 생성기의 시드를 설정
pub(super) fn srand(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let seed = uc.read_arg(0);
    uc.get_data().rand_state.store(seed, Ordering::SeqCst);
    crate::emu_log!("[MSVCRT] srand({:#x}) -> void", seed);
    Some(ApiHookResult::caller(None))
}

// =========================================================
// Environment
// =========================================================
// API: char* getenv(const char* varname)
// 역할: 특정 환경 변수의 값을 가져옴
pub(super) fn getenv(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let varname_addr = uc.read_arg(0);
    let varname = uc.read_euc_kr(varname_addr as u64);
    crate::emu_log!("[MSVCRT] getenv(\"{}\") -> char* 0x0", varname);
    Some(ApiHookResult::callee(1, Some(0))) // cdecl, NULL
}

// =========================================================
// Thread
// =========================================================
// API: uintptr_t _beginthreadex(void* security, unsigned stack_size, unsigned (*start_address)(void*), void* arglist, unsigned initflag, unsigned* thrdaddr)
// 역할: 새 스레드를 생성하고 협력적 스케줄러 큐에 등록 (Win32 API 기반 확장 버전)
pub(super) fn _beginthreadex(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let _security = uc.read_arg(0);
    let stack_size_arg = uc.read_arg(1);
    let start_address = uc.read_arg(2);
    let arglist = uc.read_arg(3);
    let _init_flag = uc.read_arg(4);
    let thread_addr_ptr = uc.read_arg(5);

    let stack_size = if stack_size_arg == 0 {
        512 * 1024usize
    } else {
        stack_size_arg as usize
    };

    // 스레드 전용 스택 힙 할당
    let stack_alloc = uc.malloc(stack_size) as u32;
    let stack_top = stack_alloc + stack_size as u32;

    // 초기 스택: [ESP] = EXIT_ADDRESS (리턴 주소), [ESP+4] = arglist (인자)
    uc.write_u32((stack_top - 8) as u64, EXIT_ADDRESS as u32);
    uc.write_u32((stack_top - 4) as u64, arglist);
    let initial_esp = stack_top - 8;

    let handle = uc.get_data().alloc_handle();
    let thread_id = uc.get_data().alloc_handle();

    if thread_addr_ptr != 0 {
        uc.write_u32(thread_addr_ptr as u64, thread_id);
    }

    uc.get_data().threads.lock().unwrap().push(EmulatedThread {
        handle,
        thread_id,
        stack_alloc,
        stack_size: stack_size as u32,
        eax: 0,
        ecx: 0,
        edx: 0,
        ebx: 0,
        esp: initial_esp,
        ebp: initial_esp,
        esi: 0,
        edi: 0,
        eip: start_address,
        alive: true,
        terminate_requested: false,
        suspended: false,
        resume_time: None,
        wait_deadline: None,
    });

    crate::emu_log!(
        "[MSVCRT] _beginthreadex(entry={:#x}, arg={:#x}) -> handle={:#x}, id={:#x}",
        start_address,
        arglist,
        handle,
        thread_id
    );
    Some(ApiHookResult::caller(Some(handle as i32)))
}

// API: void _endthreadex(unsigned retval)
// 역할: 현재 스레드를 종료 (terminate_requested 플래그 설정 후 emu_stop)
pub(super) fn _endthreadex(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let exit_code = uc.read_arg(0);
    let tid = uc
        .get_data()
        .current_thread_idx
        .load(std::sync::atomic::Ordering::SeqCst);
    crate::emu_log!(
        "[MSVCRT] _endthreadex({}) -> void (tid={:#x})",
        exit_code,
        tid
    );

    if tid > 0 {
        // 현재 스레드를 종료 예약하고 실행 중단
        {
            let ctx = uc.get_data();
            let mut threads = ctx.threads.lock().unwrap();
            if let Some(t) = threads.iter_mut().find(|t| t.thread_id == tid) {
                t.terminate_requested = true;
            }
        }
        let _ = uc.emu_stop();
    }
    Some(ApiHookResult::caller(None))
}

// API: uintptr_t _beginthread(void (*start_address)(void*), unsigned stack_size, void* arglist)
// 역할: 새 스레드를 생성하고 협력적 스케줄러 큐에 등록
pub(super) fn _beginthread(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let start_address = uc.read_arg(0);
    let stack_size_arg = uc.read_arg(1);
    let arglist = uc.read_arg(2);

    let stack_size = if stack_size_arg == 0 {
        512 * 1024usize
    } else {
        stack_size_arg as usize
    };

    // 스레드 전용 스택 힙 할당
    let stack_alloc = uc.malloc(stack_size) as u32;
    let stack_top = stack_alloc + stack_size as u32;

    // 초기 스택: [ESP] = EXIT_ADDRESS (리턴 주소), [ESP+4] = arglist (인자)
    uc.write_u32((stack_top - 8) as u64, EXIT_ADDRESS as u32);
    uc.write_u32((stack_top - 4) as u64, arglist);
    let initial_esp = stack_top - 8;

    let handle = uc.get_data().alloc_handle();
    let thread_id = uc.get_data().alloc_handle();

    uc.get_data().threads.lock().unwrap().push(EmulatedThread {
        handle,
        thread_id,
        stack_alloc,
        stack_size: stack_size as u32,
        eax: 0,
        ecx: 0,
        edx: 0,
        ebx: 0,
        esp: initial_esp,
        ebp: initial_esp,
        esi: 0,
        edi: 0,
        eip: start_address,
        alive: true,
        terminate_requested: false,
        suspended: false,
        resume_time: None,
        wait_deadline: None,
    });

    crate::emu_log!(
        "[MSVCRT] _beginthread(entry={:#x}, arg={:#x}) -> handle={:#x}, id={:#x}",
        start_address,
        arglist,
        handle,
        thread_id
    );
    Some(ApiHookResult::caller(Some(handle as i32)))
}

// =========================================================
// Exception / SEH
// =========================================================
// API: void __stdcall _CxxThrowException(void* pExceptionObject, _ThrowInfo* pThrowInfo)
// 역할: C++ 예외를 발생시킴
pub(super) fn __cxx_throw_exception(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let p_exception_object = uc.read_arg(0);
    let p_throw_info = uc.read_arg(1);
    crate::emu_log!(
        "[MSVCRT] _CxxThrowException({:#x}, {:#x}) -> void",
        p_exception_object,
        p_throw_info
    );

    // C++ 예외 전파 자체는 아직 구현하지 못했으므로, 최소한 호출자 코드로
    // 정상 복귀하지 않게 만들어 예외 경로 이후의 실행 오염을 막습니다.
    let esp = uc.reg_read(RegisterX86::ESP).unwrap_or(0);
    uc.write_u32(esp, EXIT_ADDRESS as u32);
    Some(ApiHookResult::callee(2, None))
}

// API: int _except_handler3(...)
// 역할: 내부 예외 처리기 (SEH)
pub(super) fn _except_handler3(_uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let p_exception_record = _uc.read_arg(0);
    let p_establisher_frame = _uc.read_arg(1);
    let p_context = _uc.read_arg(2);
    let p_dispatcher_context = _uc.read_arg(3);
    crate::emu_log!(
        "[MSVCRT] _except_handler3({:#x}, {:#x}, {:#x}, {:#x}) -> int 1",
        p_exception_record,
        p_establisher_frame,
        p_context,
        p_dispatcher_context
    );
    Some(ApiHookResult::caller(Some(1))) // cdecl
}

// API: int __CxxFrameHandler(...)
// 역할: C++ 프레임 처리기
pub(super) fn ___cxx_frame_handler(_uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let p_exception_record = _uc.read_arg(0);
    let p_establisher_frame = _uc.read_arg(1);
    let p_context = _uc.read_arg(2);
    let p_dispatcher_context = _uc.read_arg(3);
    crate::emu_log!(
        "[MSVCRT] __CxxFrameHandler({:#x}, {:#x}, {:#x}, {:#x}) -> int 0",
        p_exception_record,
        p_establisher_frame,
        p_context,
        p_dispatcher_context
    );
    Some(ApiHookResult::caller(Some(0))) // cdecl
}

// API: _se_translator_function _set_se_translator(_se_translator_function se_trans_func)
// 역할: Win32 예외를 C++ 예외로 변환하는 함수를 설정
pub(super) fn _set_se_translator(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let se_translator_function = uc.read_arg(0);
    crate::emu_log!(
        "[MSVCRT] _set_se_translator({:#x}) -> _se_translator_function 0",
        se_translator_function
    );
    Some(ApiHookResult::caller(Some(0))) // cdecl
}

// API: int _setjmp3(jmp_buf env, int count)
// 역할: 비로컬 jump를 위한 현재 상태를 저장
pub(super) fn _setjmp3(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let env = uc.read_arg(0);
    let count = uc.read_arg(1);
    crate::emu_log!("[MSVCRT] _setjmp3({:#x}, {:#x}) -> int 0", env, count);
    Some(ApiHookResult::caller(Some(0))) // cdecl, 바로 리턴
}

// API: void longjmp(jmp_buf env, int value)
// 역할: setjmp로 저장된 위치로 제어를 이동
pub(super) fn longjmp(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let env = uc.read_arg(0);
    let value = uc.read_arg(1);
    crate::emu_log!("[MSVCRT] longjmp({:#x}, {:#x}) -> void", env, value);
    Some(ApiHookResult::caller(None)) // cdecl
}

// =========================================================
// Init / Exit
// =========================================================
// API: void _initterm(_PVFV* begin, _PVFV* end)
// 역할: 함수 포인터 테이블을 순회하며 초기화 함수들을 호출
pub(super) fn _initterm(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    // _initterm(begin, end) - 함수 포인터 테이블을 순회하며 호출
    let begin = uc.read_arg(0) as u64;
    let end = uc.read_arg(1) as u64;
    crate::emu_log!("[MSVCRT] _initterm({:#x}, {:#x})", begin, end);

    let mut addr = begin;
    while addr < end {
        let func_ptr = uc.read_u32(addr);
        if func_ptr != 0 {
            crate::emu_log!("[MSVCRT] _initterm: calling {:#x}", func_ptr);

            // 콜백 호출 (void __cdecl func(void))
            // 리턴 주소를 스택에 push하고 emu_start로 콜백 실행
            let esp = uc.reg_read(unicorn_engine::RegisterX86::ESP).unwrap();
            uc.reg_write(unicorn_engine::RegisterX86::ESP, esp - 4)
                .unwrap();
            uc.write_u32(esp - 4, 0); // return addr = 0 → EXIT_ADDRESS처럼 동작

            uc.get_data()
                .emu_depth
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if let Err(e) = uc.emu_start(func_ptr as u64, 0, 0, 10000) {
                crate::emu_log!(
                    "[MSVCRT] _initterm: callback at {:#x} failed: {:?}",
                    func_ptr,
                    e
                );
            }
            uc.get_data()
                .emu_depth
                .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);

            // 스택 복원 (콜백이 스택을 건드렸을 수 있으므로 원래 ESP로 복구)
            uc.reg_write(unicorn_engine::RegisterX86::ESP, esp).unwrap();
        }
        addr += 4;
    }
    crate::emu_log!("[MSVCRT] _initterm({:#x}, {:#x}) -> void", begin, end);
    Some(ApiHookResult::caller(None)) // cdecl
}

// API: void _exit(int status)
// 역할: 프로세스를 즉시 종료
pub(super) fn _exit(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let status = uc.read_arg(0);
    crate::emu_log!("[MSVCRT] _exit({:#x}) -> void", status);
    let _ = uc.emu_stop();
    Some(ApiHookResult::caller(None)) // cdecl
}

// API: _onexit_t __dllonexit(_onexit_t func, _PVFV** begin, _PVFV** end)
// 역할: DLL 종료 시 호출될 함수를 등록
pub(super) fn __dllonexit(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let func = uc.read_arg(0);
    let _begin = uc.read_arg(1);
    let _end = uc.read_arg(2);

    let ctx = uc.get_data();
    let mut handlers = ctx.onexit_handlers.lock().unwrap();
    handlers.push(func);

    crate::emu_log!(
        "[MSVCRT] __dllonexit({:#x}, {:#x}, {:#x}) -> _onexit_t {:#x}",
        func,
        _begin,
        _end,
        func
    );
    Some(ApiHookResult::callee(3, Some(func as i32))) // cdecl, returns the function pointer on success
}

// API: _onexit_t _onexit(_onexit_t func)
// 역할: 프로그램 종료 시 호출될 함수를 등록
pub(super) fn _onexit(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let func = uc.read_arg(0);
    let ctx = uc.get_data();
    let mut handlers = ctx.onexit_handlers.lock().unwrap();
    handlers.push(func);

    crate::emu_log!("[MSVCRT] _onexit({:#x}) -> _onexit_t {:#x}", func, func);
    Some(ApiHookResult::callee(1, Some(func as i32))) // cdecl
}

pub(super) fn terminate(_uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    crate::emu_log!("[MSVCRT] terminate()");
    Some(ApiHookResult::callee(0, None))
}

pub(super) fn type_info(_uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    crate::emu_log!("[MSVCRT] type_info::~type_info()");
    Some(ApiHookResult::callee(0, None)) // thiscall 가능하지만 cdecl로 진입
}

pub(super) fn _adjust_fdiv(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    // 이것은 전역 변수: __adjust_fdiv는 FDIV 버그 플래그
    // 주소 반환 (0 값의 글로벌 변수 주소)
    let addr = uc.malloc(4);
    uc.write_u32(addr, 0);
    crate::emu_log!("[MSVCRT] _adjust_fdiv -> {:#x}", addr);
    Some(ApiHookResult::callee(0, Some(addr as i32)))
}

// API: void _purecall(void)
// 역할: 순수 가상 함수 호출 시의 에러 처리기
pub(super) fn _purecall(_uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    crate::emu_log!("[MSVCRT] _purecall() -> void");
    Some(ApiHookResult::callee(0, None))
}

// API: int* _errno(void)
// 역할: 현재 스레드의 오류 번호(errno) 포인터를 가져옴
pub(super) fn _errno(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    // _errno returns a pointer to thread-local errno
    let addr = uc.malloc(4);
    uc.write_u32(addr, 0);
    crate::emu_log!("[MSVCRT] _errno() -> int* {:#x}", addr);
    Some(ApiHookResult::callee(0, Some(addr as i32)))
}

// API: void qsort(void* base, size_t num, size_t width, int (*compare)(const void*, const void*))
// 역할: 퀵 정렬 알고리즘을 수행
pub(super) fn qsort(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let base = uc.read_arg(0);
    let num = uc.read_arg(1) as usize;
    let width = uc.read_arg(2) as usize;
    let compare_addr = uc.read_arg(3) as u64;

    if num <= 1 || width == 0 || compare_addr == 0 {
        return Some(ApiHookResult::callee(4, None));
    }

    let data = uc.mem_read_as_vec(base as u64, num * width).unwrap();
    let mut indices: Vec<usize> = (0..num).collect();

    // Use a simple sort for now to avoid re-entrancy issues if any
    // but try to use callback
    indices.sort_by(|&a, &b| {
        let ptr_a = base + (a * width) as u32;
        let ptr_b = base + (b * width) as u32;

        // 중첩 emu_start 호출 전에 레지스터를 저장합니다.
        let saved_esp = uc.reg_read(unicorn_engine::RegisterX86::ESP).unwrap();
        let saved_eip = uc.reg_read(unicorn_engine::RegisterX86::EIP).unwrap_or(0);
        let saved_ebp = uc.reg_read(unicorn_engine::RegisterX86::EBP).unwrap_or(0);

        // Setup stack for comparison: push ptr_b, push ptr_a, push exit_addr
        uc.push_u32(ptr_b);
        uc.push_u32(ptr_a);
        uc.push_u32(EXIT_ADDRESS as u32);

        // Run comparison function
        uc.get_data()
            .emu_depth
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Err(e) = uc.emu_start(compare_addr, EXIT_ADDRESS, 0, 0) {
            crate::emu_log!("[MSVCRT] qsort callback error: {:?}", e);
            uc.get_data()
                .emu_depth
                .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            let _ = uc.reg_write(unicorn_engine::RegisterX86::ESP, saved_esp);
            let _ = uc.reg_write(unicorn_engine::RegisterX86::EBP, saved_ebp);
            let _ = uc.reg_write(unicorn_engine::RegisterX86::EIP, saved_eip);
            return std::cmp::Ordering::Equal;
        }
        uc.get_data()
            .emu_depth
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);

        let res = uc.reg_read(unicorn_engine::RegisterX86::EAX).unwrap() as i32;

        // 레지스터 복원
        let _ = uc.reg_write(unicorn_engine::RegisterX86::ESP, saved_esp);
        let _ = uc.reg_write(unicorn_engine::RegisterX86::EBP, saved_ebp);
        let _ = uc.reg_write(unicorn_engine::RegisterX86::EIP, saved_eip);

        if res < 0 {
            std::cmp::Ordering::Less
        } else if res > 0 {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });

    // Reorder data based on indices
    let mut sorted_data = Vec::with_capacity(num * width);
    for idx in indices {
        sorted_data.extend_from_slice(&data[idx * width..(idx + 1) * width]);
    }

    uc.mem_write(base as u64, &sorted_data).unwrap();

    crate::emu_log!(
        "[MSVCRT] qsort({:#x}, {}, {}, {:#x}) -> void (sorted)",
        base,
        num,
        width,
        compare_addr
    );
    Some(ApiHookResult::callee(4, None))
}

// C++ exception related
pub(super) fn exception_ref(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
    let other_ptr = uc.read_arg(0);
    crate::emu_log!(
        "[MSVCRT] (this={:#x}) exception::exception({:#x}) -> (this={:#x})",
        this_ptr,
        other_ptr,
        this_ptr
    );
    Some(ApiHookResult::callee(1, Some(this_ptr as i32))) // thiscall/cdecl hybrid
}

pub(super) fn exception_ptr(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
    let ptr = uc.read_arg(0);
    crate::emu_log!(
        "[MSVCRT] (this={:#x}) exception::exception({:#x}) -> (this={:#x})",
        this_ptr,
        ptr,
        this_ptr
    );
    Some(ApiHookResult::callee(1, Some(0)))
}

// API: void _CxxThrowException(void* pExceptionObject, _ThrowInfo* pThrowInfo)
// 역할: C++ 예외를 발생시킴
pub(super) fn _cxx_throw_exception(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let p_exception_object = uc.read_arg(0);
    let p_throw_info = uc.read_arg(1);
    crate::emu_log!(
        "[MSVCRT] _CxxThrowException({:#x}, {:#x}) -> void",
        p_exception_object,
        p_throw_info
    );

    // 예외 미구현 상태에서 호출자에게 복귀하면 이후 프레임이 그대로 진행되어
    // 잘못된 API 인자와 크래시로 이어지므로 현재 emu_start를 즉시 끝냅니다.
    let esp = uc.reg_read(RegisterX86::ESP).unwrap_or(0);
    uc.write_u32(esp, EXIT_ADDRESS as u32);
    Some(ApiHookResult::callee(2, None))
}
