use crate::{
    debug::common::{CpuContext, DebugCommand},
    dll::win32::{Win32Context, kernel32::KERNEL32},
};
use std::{
    any::Any,
    sync::mpsc::{Receiver, Sender},
    time::{Duration, Instant},
};
use unicorn_engine::{HookType, Prot, RegisterX86, Unicorn};

use super::UnicornHelper;
use super::memory::*;

pub(super) const DEBUG_AUTO_QUANTUM: usize = 20_000;
pub(super) const DEBUG_STEP_QUANTUM: usize = 1;
pub(super) const DEBUG_STATE_SEND_INTERVAL: Duration = Duration::from_millis(250);
pub(super) const EMULATOR_IDLE_SLEEP_SLICE: Duration = Duration::from_millis(5);

pub(crate) fn capture_cpu_context(uc: &mut Unicorn<Win32Context>) -> CpuContext {
    let regs = [
        uc.reg_read(RegisterX86::EAX).unwrap_or(0) as u32,
        uc.reg_read(RegisterX86::EBX).unwrap_or(0) as u32,
        uc.reg_read(RegisterX86::ECX).unwrap_or(0) as u32,
        uc.reg_read(RegisterX86::EDX).unwrap_or(0) as u32,
        uc.reg_read(RegisterX86::ESI).unwrap_or(0) as u32,
        uc.reg_read(RegisterX86::EDI).unwrap_or(0) as u32,
        uc.reg_read(RegisterX86::EBP).unwrap_or(0) as u32,
        uc.reg_read(RegisterX86::ESP).unwrap_or(0) as u32,
        uc.reg_read(RegisterX86::EIP).unwrap_or(0) as u32,
    ];

    let esp = regs[7] as u64;
    let mut stack = Vec::new();
    let mut buf = [0u8; 4];
    for i in 0..10 {
        let target = esp + (i as u64 * 4);
        if uc.mem_read(target, &mut buf).is_ok() {
            stack.push((target as u32, u32::from_le_bytes(buf)));
        }
    }

    // 디스어셈블러가 없으므로 현재 EIP 주변 바이트를 보여줘 다음 위치 파악에 사용합니다.
    let mut code = [0u8; 8];
    let code_len = if uc.mem_read(regs[8] as u64, &mut code).is_ok() {
        code.len()
    } else {
        0
    };
    let next_instr = code[..code_len]
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<Vec<_>>()
        .join(" ");

    CpuContext {
        regs,
        stack,
        next_instr,
    }
}

/// 중첩 게스트 호출을 실제 반환 지점(`EXIT_ADDRESS`)까지 계속 실행합니다.
///
/// `emu_stop()`로 인해 한 번의 `emu_start`가 조기 중단되어도 현재 EIP에서 재개하며,
/// 사이사이에 백그라운드 스레드를 스케줄링해 Sleep/Wait 기반 yield도 소화합니다.
pub(crate) fn run_nested_guest_until_exit(
    uc: &mut Unicorn<Win32Context>,
    entry_eip: u64,
) -> Result<(), String> {
    let mut next_eip = entry_eip;

    loop {
        if next_eip == EXIT_ADDRESS {
            return Ok(());
        }
        if next_eip == 0 {
            return Err(String::from("execution reached 0x0"));
        }

        if let Err(e) = uc.emu_start(next_eip, EXIT_ADDRESS, 0, DEBUG_AUTO_QUANTUM) {
            return Err(format!("{e:?}"));
        }

        let cur_eip = uc.reg_read(RegisterX86::EIP).unwrap_or(0);
        if cur_eip == EXIT_ADDRESS {
            return Ok(());
        }
        if cur_eip == 0 {
            return Err(String::from("execution reached 0x0"));
        }

        KERNEL32::schedule_threads(uc);
        next_eip = uc.reg_read(RegisterX86::EIP).unwrap_or(0);
    }
}

/// 에뮬레이터의 초기 환경을 구성합니다.
/// 스택, 힙, 통신 채널 등을 설정하고 기본적인 API 후킹 준비를 완료합니다.
///
/// # 인자
/// * `state_tx`: UI(디버거)로 CPU 상태(`CpuContext`)를 전송하기 위한 Sender. 비디버그 모드에서는 `None`
/// * `cmd_rx`: UI로부터 디버깅 명령어(`DebugCommand`)를 수신하기 위한 Receiver. 비디버그 모드에서는 `None`
///
/// # 반환
/// * `Result<(), ()>`: 성공 시 `Ok(())`, 메모리 매핑 오류 등 실패 시 `Err(())`
pub(crate) fn setup_impl(
    uc: &mut Unicorn<'_, Win32Context>,
    _state_tx: Option<Sender<CpuContext>>,
    _cmd_rx: Option<Receiver<DebugCommand>>,
) -> Result<(), ()> {
    // [1] 메모리 맵 설정
    uc.mem_map(STACK_BASE, STACK_SIZE, Prot::ALL)
        .map_err(|e| crate::emu_log!("[!] Failed to map Stack: {:?}", e))?;
    // 스택 오버플로우/경계 읽기 에러 방지 (스택 바로 뒤 4KB 추가 할당)
    uc.mem_map(STACK_TOP, SIZE_4KB, Prot::ALL)
        .map_err(|e| crate::emu_log!("[!] Failed to map Stack Guard: {:?}", e))?;

    uc.mem_map(HEAP_BASE, HEAP_SIZE, Prot::ALL)
        .map_err(|e| crate::emu_log!("[!] Failed to map Heap: {:?}", e))?;
    uc.mem_map(SHARED_MEM_BASE, SIZE_4KB, Prot::ALL)
        .map_err(|e| crate::emu_log!("[!] Failed to map Shared Mem: {:?}", e))?;

    // NULL 포인터 접근 방지 (0 ~ 128KB)
    // 읽기/쓰기만 허용하고 실행은 차단하여, EIP가 0으로 떨어졌을 때
    // FETCH_UNMAPPED 훅이 발동되어 호스트 프로세스 segfault를 방지합니다.
    uc.mem_map(0, 0x2_0000, Prot::READ | Prot::WRITE)
        .map_err(|e| crate::emu_log!("[!] Failed to map Null Page: {:?}", e))?;

    // [2] TEB (Thread Environment Block) 설정
    uc.mem_map(TEB_BASE, SIZE_4KB, Prot::ALL)
        .map_err(|e| crate::emu_log!("[!] Failed to map TEB: {:?}", e))?;
    // x86 SEH 체인의 끝은 `-1`이므로 초기 예외 리스트 헤더를 맞춰 둡니다.
    uc.mem_write(TEB_BASE, &0xFFFF_FFFFu32.to_le_bytes())
        .map_err(|e| crate::emu_log!("[!] Failed to write TEB exception list: {:?}", e))?;
    // Self-pointer at TEB + 0x18
    uc.mem_write(TEB_BASE + 0x18, &(TEB_BASE as u32).to_le_bytes())
        .map_err(|e| crate::emu_log!("[!] Failed to write TEB self-pointer: {:?}", e))?;
    // 현재 Unicorn 32-bit x86에서는 `FS_BASE` 쓰기가 no-op이므로, 게스트가 실제로
    // 사용하는 `fs:[0]` / `fs:[0x18]` 조회가 선형 주소 0 기반에서도 동작하도록
    // 최소 TEB 헤더를 저주소 별칭으로 함께 미러링합니다.
    uc.mem_write(0, &0xFFFF_FFFFu32.to_le_bytes())
        .map_err(|e| crate::emu_log!("[!] Failed to mirror SEH head at null page: {:?}", e))?;
    uc.mem_write(0x18, &(TEB_BASE as u32).to_le_bytes())
        .map_err(|e| {
            crate::emu_log!(
                "[!] Failed to mirror TEB self-pointer at null page: {:?}",
                e
            )
        })?;

    // [3] Fake Import Area (API 후킹용 실행 영역)
    uc.mem_map(FAKE_IMPORT_BASE, 1024 * 1024, Prot::ALL | Prot::EXEC)
        .map_err(|e| crate::emu_log!("[!] Failed to map Fake Import Area: {:?}", e))?;
    // RET (0xC3) 으로 채우기: 코드 훅이 실행된 후 자연스럽게 RET로 복귀
    let ret_fill = vec![0xC3u8; 1024 * 1024];
    uc.mem_write(FAKE_IMPORT_BASE, &ret_fill)
        .map_err(|e| crate::emu_log!("[!] Failed to fill Fake Import Area: {:?}", e))?;

    // x86 세그먼트 레지스터(SS) 버그 방지를 위해 ESP를 페이지 경계에서 약간 띄움
    uc.reg_write(RegisterX86::ESP, STACK_TOP - 0x1000)
        .map_err(|e| crate::emu_log!("[!] Failed to set initial ESP: {:?}", e))?;

    // EXIT_ADDRESS(0xFFFFFFFF)로 return 시의 접근 예외 방용 영역
    uc.mem_map(0xFFFF_0000, 64 * 1024, Prot::ALL | Prot::EXEC)
        .map_err(|e| crate::emu_log!("[!] Failed to map Exit Area: {:?}", e))?;

    // [4] API Call Hook (Fake Address Range)
    // 0xF0000000 대역으로 점프 시 실행되는 훅
    uc.add_code_hook(
        FAKE_IMPORT_BASE,
        FAKE_IMPORT_BASE + 1024 * 1024,
        |uc: &mut Unicorn<Win32Context>, addr, _size| {
            let import_func = {
                let context = uc.get_data();
                let address_map = context.address_map.lock().unwrap();
                address_map.get(&addr).cloned()
            };

            if let Some(import_func) = import_func {
                let splits: Vec<&str> = import_func.split('!').collect();
                if splits.len() < 2 {
                    return;
                }
                let dll_name = splits[0];
                let func_name = splits[1];

                let esp_before = uc.reg_read(RegisterX86::ESP).unwrap_or(0);

                // 1. 이미 로드된 DLL 내부 오프셋에 매핑된 함수가 있는지 확인
                let func_address = {
                    let context = uc.get_data();
                    let dll_modules = context.dll_modules.lock().unwrap();
                    dll_modules
                        .get(dll_name)
                        .and_then(|dll| dll.exports.get(func_name).copied())
                };

                if let Some(func_address) = func_address {
                    if dll_name.eq_ignore_ascii_case("WinCore.dll")
                        && func_name.contains("?Create@T")
                    {
                        let ecx = uc.reg_read(RegisterX86::ECX).unwrap_or(0) as u32;
                        let edx = uc.reg_read(RegisterX86::EDX).unwrap_or(0) as u32;
                        let stack_words = (0..6)
                            .map(|i| format!("{:#x}", uc.read_u32(esp_before + (i * 4))))
                            .collect::<Vec<_>>()
                            .join(", ");
                        crate::emu_log!(
                            "[TRACE] Internal call {} -> {:#x} ECX={:#x} EDX={:#x} ESP={:#x} RET={:#x} STACK=[{}]",
                            import_func,
                            func_address,
                            ecx,
                            edx,
                            esp_before as u32,
                            uc.read_u32(esp_before),
                            stack_words
                        );
                    }

                    // 이미 게스트가 `call [IAT]`를 수행해 원래 복귀 주소를 스택에 올려 둔 상태이므로,
                    // 내부 DLL export는 중첩 `emu_start`로 별도 실행하지 않고 현재 실행 흐름의 EIP만
                    // 실제 함수 주소로 넘겨 같은 게스트 call frame 안에서 계속 실행시킵니다.
                    //
                    // 이 경로는 C++ thiscall / SEH 프레임까지 포함한 원래 호출 구조를 보존하므로,
                    // WinCore 내부 메서드가 다시 다른 guest 함수를 호출할 때 상위 프레임이 오염되는
                    // 문제를 막습니다.
                    let _ = uc.reg_write(RegisterX86::EIP, func_address);
                    uc.emu_stop().unwrap_or_default();
                    return;
                }

                // 2. DLL 핸들러(Proxy DLL)에 정의된 함수인지 확인
                if let Some(hook_result) = Win32Context::handle(uc, dll_name, func_name) {
                    if hook_result.retry {
                        // 재시도 요청 시: EIP를 현재 후킹 지점으로 유지하고 실행 중단 (멀티태스킹 양보)
                        uc.emu_stop().unwrap_or_default();
                        return;
                    }

                    if let Some(eax) = hook_result.return_value {
                        uc.reg_write(RegisterX86::EAX, eax as u64)
                            .unwrap_or_default();
                    }

                    if dll_name.eq_ignore_ascii_case("USER32.dll") {
                        // USER32의 일부 호출(CreateWindowExA 등)은 가짜 import RET가
                        // 실행되기 전에 동일 훅으로 재진입하면서 현재 호출 프레임을
                        // 다시 읽어 가짜 인자를 만드는 문제가 있습니다.
                        //
                        // 이 DLL에 한해서는 훅 안에서 caller의 EIP/ESP로 즉시
                        // 복귀를 완료하여 RET 재진입을 차단합니다.
                        let esp_after = uc.reg_read(RegisterX86::ESP).unwrap_or(esp_before);
                        let return_addr = uc.read_u32(esp_after);
                        let final_esp =
                            super::stack_cleanup_final_esp(esp_after, hook_result.cleanup);
                        let _ = uc.reg_write(RegisterX86::ESP, final_esp);
                        let _ = uc.reg_write(RegisterX86::EIP, return_addr as u64);
                        uc.emu_stop().unwrap_or_default();
                    } else {
                        // 그 외 DLL은 원래 x86 호출 흐름을 최대한 그대로 두어,
                        // cdecl 호출자 정리 코드(pop ecx; ret 등)가 같은 quantum 안에서
                        // 자연스럽게 이어지도록 합니다.
                        uc.apply_stack_cleanup(hook_result.cleanup);
                    }
                    return;
                }

                // 미구현 함수: 스택 정리 불가 (arg_count 불명)
                // RET만 실행되므로 stdcall 인자가 스택에 잔류하여 ESP가 어긋남
                crate::emu_log!(
                    "[!] Function not implemented: {} — ESP may be corrupted (no stack cleanup)",
                    import_func
                );
            } else {
                crate::emu_log!("[!] Call to unknown fake address: {:#x}", addr);
            }

            // 미구현 함수 기본 응답: EAX=1
            uc.reg_write(RegisterX86::EAX, 1).unwrap_or_default();
        },
    )
    .map_err(|e| crate::emu_log!("[!] Failed to install API hook: {:?}", e))?;

    // [5] EIP=0 보호를 위한 전용 코드 훅
    // 이전에는 JIT 방지를 위해 전체 주소 범위를 훅했으나, 성능 향상을 위해
    // 실제 문제가 되는 0번지 진입만 차단하는 가벼운 훅으로 대체합니다.
    uc.add_code_hook(0, 0, |uc: &mut Unicorn<Win32Context>, _addr, _size| {
        crate::emu_log!("[!] Execution at address 0x0 detected. Stopping.");
        uc.emu_stop().unwrap_or_default();
    })
    .map_err(|e| crate::emu_log!("[!] Failed to install null-eip hook: {:?}", e))?;

    // [6] Unmapped Memory Access Hook
    uc.add_mem_hook(
        HookType::MEM_READ_UNMAPPED
            | HookType::MEM_WRITE_UNMAPPED
            | HookType::MEM_FETCH_UNMAPPED,
        0,
        -1i64 as u64,
        |uc, access, addr, size, value| {
            let eip = uc.reg_read(RegisterX86::EIP).unwrap_or(0);
            let esp = uc.reg_read(RegisterX86::ESP).unwrap_or(0);
            crate::emu_log!(
                "[!] Unmapped memory access: {:?} at {:#x} (Size: {}, Val: {:#x}) EIP={:#x} ESP={:#x}",
                access,
                addr,
                size,
                value,
                eip,
                esp
            );
            false // 중단
        },
    )
    .map_err(|e| crate::emu_log!("[!] Failed to install memory hook: {:?}", e))?;

    Ok(())
}

/// 특정 DLL 내의 지정된 함수를 직접 호출합니다. (테스트 및 특정 API 명시적 실행용)
///
/// # 인자
/// * `dll_name`: 호출할 함수가 포함된 DLL 이름
/// * `func_name`: 호출할 대상 함수 이름
/// * `args`: 함수에 전달될 인자들 모음
#[allow(dead_code)]
pub(crate) fn run_dll_func_impl(
    uc: &mut Unicorn<Win32Context>,
    dll_name: &str,
    func_name: &str,
    args: Vec<Box<dyn Any>>,
) {
    uc.prepare_dll_func(dll_name, func_name, args);
    let eip = uc.reg_read(RegisterX86::EIP).unwrap_or(0);

    if let Err(e) = uc.emu_start(eip, EXIT_ADDRESS, 0, 0) {
        crate::emu_log!(
            "[!] Execution of {}!{} failed: {:?}",
            dll_name,
            func_name,
            e
        );
    }
}

pub(crate) fn run_emulator_impl(
    uc: &mut Unicorn<Win32Context>,
    dll_name: &str,
    func_name: &str,
    args: Vec<Box<dyn Any>>,
    state_tx: Option<Sender<CpuContext>>,
    cmd_rx: Option<Receiver<DebugCommand>>,
) {
    uc.prepare_dll_func(dll_name, func_name, args);
    let cmd_rx = cmd_rx;
    let mut debug_auto_run = true;
    let mut debug_last_state_sent = Instant::now();

    // 에뮬레이터 스레드 핸들을 저장하여 UI 스레드에서 unpark로 즉시 깨울 수 있도록 합니다.
    *uc.get_data().emu_thread.lock().unwrap() = Some(std::thread::current());

    if let Some(state_tx) = state_tx.as_ref()
        && state_tx.send(capture_cpu_context(uc)).is_ok()
    {
        debug_last_state_sent = Instant::now();
    }

    loop {
        let eip = uc.reg_read(RegisterX86::EIP).unwrap_or(0);
        if eip == EXIT_ADDRESS || eip == 0 {
            break;
        }

        if let Some(cmd_rx) = cmd_rx.as_ref() {
            if !debug_auto_run {
                // 일시정지 상태에서는 명령어를 받기 전까지 실행하지 않습니다.
                match cmd_rx.recv() {
                    Ok(DebugCommand::Step) => {}
                    Ok(DebugCommand::Run) => {
                        debug_auto_run = true;
                        continue;
                    }
                    Ok(DebugCommand::Pause) => continue,
                    Err(_) => {
                        uc.emu_stop().unwrap_or_default();
                        break;
                    }
                }
            } else {
                match cmd_rx.try_recv() {
                    Ok(DebugCommand::Pause) => {
                        debug_auto_run = false;
                        if let Some(state_tx) = state_tx.as_ref() {
                            if state_tx.send(capture_cpu_context(uc)).is_err() {
                                uc.emu_stop().unwrap_or_default();
                                break;
                            }
                            debug_last_state_sent = Instant::now();
                        }
                        continue;
                    }
                    Ok(DebugCommand::Run) | Ok(DebugCommand::Step) => {}
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        uc.emu_stop().unwrap_or_default();
                        break;
                    }
                }
            }
        }

        // 메인 스레드(tid=0) 실행 — resume_time이 아직 도래하지 않았으면 건너뜁니다.
        // GetMessage 등이 retry()를 반환할 때 설정한 대기 시간을 존중하여
        // 메시지가 없을 때 불필요한 spin을 방지합니다.
        let should_run_main = {
            let ctx = uc.get_data();
            if ctx.main_ready.load(std::sync::atomic::Ordering::SeqCst) != 0 {
                true
            } else {
                let resume = *ctx.main_resume_time.lock().unwrap();
                resume.is_some_and(|t| Instant::now() >= t)
            }
        };

        if should_run_main {
            let quantum = if cmd_rx.is_some() {
                if debug_auto_run {
                    DEBUG_AUTO_QUANTUM
                } else {
                    DEBUG_STEP_QUANTUM
                }
            } else {
                200_000
            };
            let _ = uc.emu_start(eip, EXIT_ADDRESS, 0, quantum);
        }

        // 백그라운드 스레드 스케줄링
        KERNEL32::schedule_threads(uc);

        if let Some(state_tx) = state_tx.as_ref() {
            let should_send_state = if debug_auto_run {
                debug_last_state_sent.elapsed() >= DEBUG_STATE_SEND_INTERVAL
            } else {
                true
            };
            if should_send_state {
                if state_tx.send(capture_cpu_context(uc)).is_err() {
                    uc.emu_stop().unwrap_or_default();
                    break;
                }
                debug_last_state_sent = Instant::now();
            }
        }

        // 모든 스레드(메인 포함)가 대기 중인 경우 호스트 측에서 대기하여 CPU 점유율 조절
        let (has_ready_work, earliest_resume) = {
            let context = uc.get_data();
            let now = Instant::now();
            let mut ready = context.main_ready.load(std::sync::atomic::Ordering::SeqCst) != 0;
            let mut min_time = None;

            if let Some(main_resume) = *context.main_resume_time.lock().unwrap() {
                if main_resume <= now {
                    ready = true;
                } else {
                    min_time = Some(main_resume);
                }
            }

            let threads = context.threads.lock().unwrap();
            for t in threads.iter().filter(|t| t.alive) {
                if t.ready {
                    ready = true;
                    break;
                }
                if let Some(t_res) = t.resume_time {
                    if t_res <= now {
                        ready = true;
                        break;
                    }
                    if min_time.is_none() || t_res < min_time.unwrap() {
                        min_time = Some(t_res);
                    }
                }
            }
            (ready, min_time)
        };

        if !has_ready_work {
            if let Some(res_time) = earliest_resume {
                let now = Instant::now();
                if res_time > now {
                    let diff = res_time.duration_since(now);
                    std::thread::park_timeout(diff.min(EMULATOR_IDLE_SLEEP_SLICE));
                }
            } else {
                std::thread::park_timeout(EMULATOR_IDLE_SLEEP_SLICE);
            }
        }
    }

    crate::emu_log!("[*] Main emulator loop finished.");
}

/// 함수 호출을 위한 스택 및 EIP 환경을 준비 (실제 에뮬레이션은 시작하지 않음)
pub(crate) fn prepare_dll_func_impl(
    uc: &mut Unicorn<Win32Context>,
    dll_name: &str,
    func_name: &str,
    args: Vec<Box<dyn Any>>,
) {
    let func_address = {
        let context = uc.get_data();
        context
            .dll_modules
            .lock()
            .unwrap()
            .get(dll_name)
            .and_then(|module| module.exports.get(func_name).copied())
    };

    if let Some(func_address) = func_address {
        // 인자 처리 및 역순 push
        let mut push_values: Vec<u32> = Vec::new();
        for arg in args {
            if let Some(v) = arg.downcast_ref::<i32>() {
                push_values.push(*v as u32);
            } else if let Some(v) = arg.downcast_ref::<u32>() {
                push_values.push(*v);
            } else if let Some(v) = arg.downcast_ref::<&str>() {
                let ptr = uc.alloc_str(v);
                push_values.push(ptr);
            }
        }

        for val in push_values.iter().rev() {
            uc.push_u32(*val);
        }
        uc.push_u32(EXIT_ADDRESS as u32); // 리턴 주소

        crate::emu_log!(
            "[*] Prepared {}!{}(...) at {:#x}",
            dll_name,
            func_name,
            func_address
        );

        let _ = uc.reg_write(RegisterX86::EIP, func_address);
    }
}
