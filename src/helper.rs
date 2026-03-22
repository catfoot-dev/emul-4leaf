use crate::{
    debug::common::{CpuContext, DebugCommand},
    win32::{LoadedDll, StackCleanup, Win32Context},
};
use encoding_rs::EUC_KR;
use goblin::pe::PE;
use std::{
    any::Any,
    collections::HashMap,
    fs,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, Sender},
    },
};
use unicorn_engine::{HookType, Prot, RegisterX86, Unicorn};

// pub const HOOK_BASE: u64 = 0x1000_0000;
pub const HEAP_BASE: u64 = 0x2000_0000;
pub const HEAP_SIZE: u64 = 2 * 1024 * 1024;

pub const STACK_BASE: u64 = 0x5000_0000;
pub const STACK_SIZE: u64 = 1024 * 1024;
pub const STACK_TOP: u64 = STACK_BASE + STACK_SIZE as u64;

pub const SHARED_MEM_BASE: u64 = 0x7000_0000;

// const FUNCTION_NAME_BASE: u64 = 0x8000_0000;

pub const TEB_BASE: u64 = 0x9000_0000;
pub const FAKE_IMPORT_BASE: u64 = 0xF000_0000;
pub const EXIT_ADDRESS: u64 = 0xFFFF_FFFF;

const SIZE_4KB: u64 = 4 * 1024;

fn stack_cleanup_target_esp(esp: u64, cleanup: StackCleanup) -> Option<u64> {
    match cleanup {
        StackCleanup::Caller | StackCleanup::Callee(0) => None,
        StackCleanup::Callee(arg_count) => Some(esp + (arg_count as u64 * 4)),
    }
}

fn stack_cleanup_final_esp(esp: u64, cleanup: StackCleanup) -> u64 {
    stack_cleanup_target_esp(esp, cleanup).unwrap_or(esp) + 4
}

// =========================================================
// [Debug Helper] 메모리 덤프 함수 (Hex Dump 스타일)
// =========================================================
fn print_hexdump(uc: &Unicorn<Win32Context>, address: u64, size: usize) {
    let mut buffer = vec![0u8; size];

    // 메모리 읽기 시도
    if let Err(_) = uc.mem_read(address, &mut buffer) {
        crate::emu_log!(
            "[DEBUG] Failed to read memory at 0x{:x} (Size: {})",
            address,
            size
        );
        return;
    }

    crate::emu_log!("[DEBUG] Memory Dump at 0x{:x} ({} bytes):", address, size);

    for (i, chunk) in buffer.chunks(16).enumerate() {
        // 1. 주소 출력
        print!("  0x{:08x}: ", address + (i * 16) as u64);

        // 2. Hex 값 출력
        for byte in chunk {
            print!("{:02x} ", byte);
        }

        // 패딩 (16바이트가 안 될 경우 정렬 맞춤)
        if chunk.len() < 16 {
            for _ in 0..(16 - chunk.len()) {
                print!("   ");
            }
        }

        print!(" | ");

        // 3. ASCII 출력
        for byte in chunk {
            if *byte >= 32 && *byte <= 126 {
                print!("{}", *byte as char);
            } else {
                print!(".");
            }
        }
        crate::emu_log!();
    }
    crate::emu_log!("------------------------------------------------------------");
}

// =========================================================
// [Debug Macro] 메모리 덤프 매크로
// 사용법: dump_mem!(unicorn, 0x40000000, 0x40);
// =========================================================
macro_rules! dump_mem {
    ($uc:expr, $addr:expr, $size:expr) => {
        print_hexdump($uc, $addr as u64, $size as usize);
    };
    ($uc:expr, $addr:expr, $size:expr, $label:expr) => {
        crate::emu_log!("\n[DEBUG] Checking: {}", $label);
        print_hexdump($uc, $addr as u64, $size as usize);
    };
}

// =========================================================
// [Debug Macro] 스택 뷰어 매크로 (현재 ESP 기준)
// 사용법: dump_stack!(unicorn, 5); // 스택 상위 5개 값 출력
// =========================================================
macro_rules! dump_stack {
    ($uc:expr, $count:expr) => {
        crate::emu_log!("\n[DEBUG] Stack Trace (Top {} items):", $count);
        if let Ok(esp) = $uc.reg_read(RegisterX86::ESP) {
            let mut buf = [0u8; 4];
            for i in 0..$count {
                let addr = esp + (i * 4) as u64;
                if $uc.mem_read(addr, &mut buf).is_ok() {
                    let val = u32::from_le_bytes(buf);
                    // ESP 위치 표시 화살표
                    let marker = if i == 0 { "<- ESP" } else { "" };
                    crate::emu_log!("  0x{:08x}: 0x{:08x} ({}) {}", addr, val, val, marker);
                } else {
                    crate::emu_log!("  0x{:08x}: [UNMAPPED]", addr);
                }
            }
        } else {
            crate::emu_log!("  [Error] Failed to read ESP register");
        }
        crate::emu_log!("------------------------------------------------------------");
    };
}

// =========================================================
// [Debug Macro] 레지스터 뷰어 매크로
// 사용법: dump_regs!(unicorn);
// =========================================================
macro_rules! dump_regs {
    ($uc:expr) => {
        crate::emu_log!("\n[DEBUG] Registers:");
        let eax = $uc.reg_read(RegisterX86::EAX).unwrap_or(0);
        let ebx = $uc.reg_read(RegisterX86::EBX).unwrap_or(0);
        let ecx = $uc.reg_read(RegisterX86::ECX).unwrap_or(0);
        let edx = $uc.reg_read(RegisterX86::EDX).unwrap_or(0);
        let esi = $uc.reg_read(RegisterX86::ESI).unwrap_or(0);
        let edi = $uc.reg_read(RegisterX86::EDI).unwrap_or(0);
        let esp = $uc.reg_read(RegisterX86::ESP).unwrap_or(0);
        let ebp = $uc.reg_read(RegisterX86::EBP).unwrap_or(0);
        let eip = $uc.reg_read(RegisterX86::EIP).unwrap_or(0);

        crate::emu_log!(
            "  EAX: 0x{:08x}  EBX: 0x{:08x}  ECX: 0x{:08x}  EDX: 0x{:08x}",
            eax,
            ebx,
            ecx,
            edx
        );
        crate::emu_log!(
            "  ESI: 0x{:08x}  EDI: 0x{:08x}  ESP: 0x{:08x}  EBP: 0x{:08x}",
            esi,
            edi,
            esp,
            ebp
        );
        crate::emu_log!("  EIP: 0x{:08x}", eip);
        crate::emu_log!("------------------------------------------------------------");
    };
}

/// Unicorn 객체에 추가할 메소드 목록 정의
///
/// Unicorn 엔진을 확장하여 Win32 에뮬레이션에 필요한 메모리 조작, 스택 제어, DLL 로딩 등을 지원하는 헬퍼 트레잇
pub trait UnicornHelper {
    /// 에뮬레이터의 초기 환경을 구성
    /// 스택, 힙, 통신 채널 등을 설정하고 기본적인 API 후킹 준비를 완료
    ///
    /// # 인자 (Parameters)
    /// - `state_tx`: UI(디버거)로 CPU 상태(`CpuContext`)를 전송하기 위한 Sender
    /// - `cmd_rx`: UI로부터 디버깅 명령어(`DebugCommand`)를 수신하기 위한 Receiver
    ///
    /// # 반환 (Return)
    /// - `Result<(), ()>`: 성공 시 `Ok(())`, 메모리 매핑 요류 등 실패 시 `Err(())`
    fn setup(
        &mut self,
        state_tx: Sender<CpuContext>,
        cmd_rx: Receiver<DebugCommand>,
    ) -> Result<(), ()>;

    /// 지정된 경로의 DLL 파일을 메모리에 로드하고 재배치(Relocation)를 수행
    ///
    /// # 인자
    /// - `filename`: 로드할 DLL 파일의 경로
    /// - `target_base`: DLL을 로드할 대상 기준 메모리 주소 (ImageBase)
    ///
    /// # 반환
    /// - `Result<LoadedDll, ()>`: 로드가 성공하면 DLL의 메타데이터(`LoadedDll`)를 반환
    fn load_dll_with_reloc(&mut self, filename: &str, target_base: u64) -> Result<LoadedDll, ()>;

    /// 로드된 DLL의 Import Address Table (IAT)을 분석하여 의존성 함수 주소들을 채움
    ///
    /// # 인자
    /// - `target`: 분석할 메모리상의 DLL 객체(`LoadedDll`)의 참조
    ///
    /// # 반환
    /// - `Result<(), ()>`: 성공적으로 IAT를 구성하면 `Ok(())`
    fn resolve_imports(&mut self, target: &LoadedDll) -> Result<(), ()>;

    /// DLL의 엔트리 포인트(`DllMain`) 함수를 실행
    ///
    /// # 인자
    /// - `dll`: 엔트리 포인트를 실행할 DLL
    ///
    /// # 반환
    /// - `Result<(), ()>`: 실행 성공 시 `Ok(())`
    fn run_dll_entry(&mut self, dll: &LoadedDll) -> Result<(), ()>;

    /// 특정 DLL 내의 지정된 함수를 여러 인자와 함께 직접 호출 (테스트 및 특정 API 직접 실행용)
    ///
    /// # 인자
    /// - `dll_name`: 호출할 함수가 포함된 DLL 이름
    /// - `func_name`: 호출할 대상 함수의 이름 (예: `Main`)
    /// - `args`: 함수에 전달될 인자들 모음(`Any` 박스 형태)
    fn run_dll_func(&mut self, dll_name: &str, func_name: &str, args: Vec<Box<dyn Any>>);

    // === 메모리 읽기/쓰기 (Heap/General) ===

    /// 특정 메모리 주소에서 32비트(4바이트) 정수를 리틀 엔디안(Little Endian)으로 읽음
    ///
    /// # 인자
    /// - `addr`: 읽고자 하는 메모리 주소
    /// # 반환
    /// - `u32`: 해당 주소에 저장된 32비트 값
    fn read_u32(&self, addr: u64) -> u32;

    /// 특정 메모리 주소에 32비트(4바이트) 정수를 리틀 엔디안 방식으로 기록
    ///
    /// # 인자
    /// - `addr`: 기록하고자 하는 메모리 대상 주소
    /// - `value`: 기록할 32비트 정수 값
    fn write_u32(&mut self, addr: u64, value: u32);

    /// 함수 호출 시 전달된 인자(스택) 중 N번째 인자를 읽음 (cdecl/stdcall 호출 규약 기준)
    /// `index = 0` 일 때 첫 번째 인자 값 반환
    ///
    /// # 인자
    /// - `index`: 가져올 인자의 인덱스 번호 (0부터 시작)
    /// # 반환
    /// - `u32`: 해당 인자의 32비트 값
    fn read_arg(&self, index: usize) -> u32;

    /// C언어 스타일 널 종료 문자열(Null-Terminated String)을 읽어와서 Rust의 `String`으로 반환 (기본 ASCII/UTF-8 형태)
    ///
    /// # 인자
    /// - `addr`: 문자열 처리가 시작될 메모리 주소
    fn read_string(&self, addr: u64) -> String;

    /// 대상 메모리에 C언어 스타일 널 종료 문자열(Null-Terminated String)을 기록
    ///
    /// # 인자
    /// - `addr`: 문자열을 기록할 메모리 주소
    /// - `text`: 기록할 Rust 문자열 레퍼런스(`&str`)
    fn write_string(&mut self, addr: u64, text: &str);

    /// 대상 메모리의 널 종료 문자열을 읽고, 내용이 EUC-KR 문자셋일 경우 디코딩하여 Rust `String`으로 반환 (한국어 호환 프로그램용)
    ///
    /// # 인자
    /// - `addr`: 문자열의 메모리 주소
    fn read_euc_kr(&self, addr: u64) -> String;

    // === 스택 조작 (Stack) ===

    /// 스택 포인터(`ESP`)를 4바이트 감소시키고 그 위치에 32비트 값을 `push`
    ///
    /// # 인자
    /// - `value`: 스택에 넣을 32비트 값
    fn push_u32(&mut self, value: u32);

    /// 스택에서 현재 `ESP`가 가리키는 32비트 값을 `pop` 하고, 스택 포인터를 4바이트 증가시킴
    ///
    /// # 반환
    /// - `u32`: 스택에서 뽑아낸(Pop된) 최상단 값
    fn pop_u32(&mut self) -> u32;

    /// Callee-cleanup(`stdcall`) 방식 등, 함수 실행이 끝난 뒤 스택을 안전하게 정리(보정)
    ///
    /// # 인자
    /// - `cleanup`: 정리할 바이트 수 혹은 Caller가 회수할 지 등을 묘사한 열거형
    fn apply_stack_cleanup(&mut self, cleanup: StackCleanup);

    /// 인자로 들어온 크기만큼 힙 메모리 영역에서 공간을 할당받음. 초기화되지 않은 메모리 주소가 반환됨
    ///
    /// # 인자
    /// - `size`: 할당할 바이트 수
    /// # 반환
    /// - `u64`: 할당된 힙 영역의 64비트 가상 주소(내부적으로 32비트 대역을 씀)
    fn malloc(&mut self, size: usize) -> u64;

    /// 문자열을 새로 힙 공간에 할당하고, 그 끝에 문자열 터미네이터(`\0`)를 자동으로 추가
    ///
    /// # 인자
    /// - `text`: 힙에 기록할 Rust 문자열 레퍼런스(`&str`)
    /// # 반환
    /// - `u32`: 생성된 C-Style 문자열이 기록된 힙의 32비트 주소
    fn alloc_str(&mut self, text: &str) -> u32; // 32비트 주소 반환

    /// 임의의 바이트 배열(특정 구조체 데이터 등)을 힙 공간을 할당받아 그대로 기록
    ///
    /// # 인자
    /// - `data`: 메모리에 복사할 바이트 슬라이스(`&[u8]`)
    /// # 반환
    /// - `u32`: 데이터가 복사된 힙의 32비트 주소
    fn alloc_bytes(&mut self, data: &[u8]) -> u32;
}

// 모든 Unicorn<D> 타입에 대해 구현 (D는 Win32Context 등 무엇이든 가능)
impl UnicornHelper for Unicorn<'_, Win32Context> {
    fn setup(
        &mut self,
        state_tx: Sender<CpuContext>,
        cmd_rx: Receiver<DebugCommand>,
    ) -> Result<(), ()> {
        self.mem_map(STACK_BASE, STACK_SIZE, Prot::ALL).unwrap();
        // 스택 오버플로우/경계 읽기 에러 방지 (스택 바로 뒤 4KB 추가 할당)
        self.mem_map(STACK_TOP, SIZE_4KB, Prot::ALL).unwrap();

        self.mem_map(HEAP_BASE, HEAP_SIZE, Prot::ALL).unwrap();
        self.mem_map(SHARED_MEM_BASE, SIZE_4KB, Prot::ALL).unwrap();

        // crate::emu_log!("[*] Mapping Low Memory (0x0 ~ 0x20000) to bypass NULL pointer access");
        // NULL 포인터 접근 방지 (0 ~ 128KB)
        self.mem_map(0, 0x2_0000, Prot::ALL).unwrap();

        // TEB
        self.mem_map(TEB_BASE, SIZE_4KB, Prot::ALL).unwrap();
        // 메모리에만 기록하고 FS 레지스터는 건드리지 않음 (0 유지)
        self.mem_write(TEB_BASE + 0x18, &(TEB_BASE as u32).to_le_bytes())
            .unwrap();

        // Fake Import Area
        self.mem_map(FAKE_IMPORT_BASE, 1024 * 1024, Prot::ALL | Prot::EXEC)
            .unwrap();
        // RET (0xC3) 으로 채우기: 코드 훅이 실행된 후 자연스럽게 RET로 복귀
        let ret_fill = vec![0xC3u8; 1024 * 1024];
        self.mem_write(FAKE_IMPORT_BASE, &ret_fill).unwrap();

        // Unicorn Engine의 내부 x86 세그먼트 레지스터(SS) 캐시가 16비트로 동작하는
        // 버그(혹은 미초기화 현상)로 인해 SP 역산 시 0xFFFF -> 0x0000 랩어라운드가 발생
        // 스택 포인터가 0x...0000 (16비트 경계)에 물리지 않도록 4KB 여유 공간을 둠
        self.reg_write(RegisterX86::ESP, STACK_TOP - 0x1000)
            .unwrap();

        // let modules = loaded_modules.clone();

        // EXIT_ADDRESS(0xFFFF_FFFF)로 return 시 메모리 접근 예외가 발생해 EIP가 0으로 튀는 현상 방지
        self.mem_map(0xFFFF_0000, 64 * 1024, Prot::ALL | Prot::EXEC)
            .unwrap();

        // self.add_code_hook(
        //     0,
        //     0x10 as u64,
        //     |uc, addr, _|
        // {
        //     crate::emu_log!("[!] Run code at {addr:#x}");
        //     // dump_mem!(uc, addr & 0xFFFF_FFF0, 16);
        // }).expect("Failed to install code hook(Dll Code)");

        // self.add_code_hook(
        //     // 0x3000_0000,
        //     // 0x4FFF_FFFF,
        //     0x20000,
        //     -1i64 as u64,
        //     |uc, addr, _|
        // {
        //     crate::emu_log!("[!] Run code at {addr:#x}");
        //     // dump_mem!(uc, addr & 0xFFFF_FFF0, 16);
        // }).expect("Failed to install code hook(Dll Code)");

        // API Call Hook (Fake Address Range)
        self.add_code_hook(
            FAKE_IMPORT_BASE,
            FAKE_IMPORT_BASE + 1024 * 1024,
            |uc: &mut Unicorn<Win32Context>, addr, _size| {
                let import_func = {
                    let context = uc.get_data();
                    let address_map = context.address_map.lock().unwrap();
                    address_map.get(&addr).cloned()
                };

                if let Some(import_func) = import_func {
                    crate::emu_log!("[!] {import_func} -> {addr:#x}");

                    let splits: Vec<&str> = import_func.split('!').collect();
                    let dll_name = splits[0];
                    let func_name = splits[1];

                    // 삽입한 dll에 있는지 찾아서 실행
                    let func_address = {
                        let context = uc.get_data();
                        let dll_modules = context.dll_modules.lock().unwrap();
                        dll_modules
                            .get(dll_name)
                            .and_then(|dll| dll.exports.get(func_name).copied())
                    };

                    if let Some(func_address) = func_address {
                        crate::emu_log!("[*] Function address: {func_address:#x}");
                        crate::emu_log!("[*] Calling {dll_name}!{func_name}(...args)...");

                        if let Err(e) = uc.emu_start(func_address as u64, EXIT_ADDRESS, 0, 0) {
                            crate::emu_log!("\n[!] Emulation stopped/failed: {e:?}");
                            let pc = uc.reg_read(RegisterX86::EIP).unwrap();
                            crate::emu_log!("    Stopped at EIP: {pc:#x}\n");
                        } else {
                            crate::emu_log!("[*] {dll_name}!{func_name} finished successfully.");
                        }

                        return;
                    }

                    // 따로 정의한 함수가 있는지 찾아서 실행
                    if let Some(hook_result) = Win32Context::handle(uc, dll_name, func_name) {
                        if let Some(eax) = hook_result.return_value {
                            uc.reg_write(RegisterX86::EAX, eax as u64).unwrap();
                        }

                        uc.apply_stack_cleanup(hook_result.cleanup);
                        // RET 명령어(0xC3)가 ret_addr을 pop하여 자연스럽게 복귀
                        return;
                    }

                    // 매핑된 주소 값이나 dll은 있지만 매칭되는 함수가 없음
                    crate::emu_log!("[!] Can not found function. {import_func}");
                } else {
                    // 매핑된 주소 값이 없음
                    crate::emu_log!("[!] Can not found addr. {addr:#x}");
                }

                // 매칭 안 된 함수: EAX=1, 스택은 건드리지 않고 RET가 처리
                uc.reg_write(RegisterX86::EAX, 1).unwrap();
                // RET(0xC3)이 자연스럽게 ret_addr을 pop하여 복귀
            },
        )
        .expect("Failed to install code hook(Fake Address)");

        // self.add_code_hook(0, 0x2_000, |uc, _, _|
        // {
        //     crate::emu_log!("\n[!] Detected execution at 0x00. Assuming successful return from function.");
        //     crate::emu_log!("    (Cause: Stack pointer drift due to stdcall mismatch)");

        //     dump_stack!(uc, 4);
        //     dump_regs!(uc);

        //     uc.emu_stop().unwrap();
        // }).expect("Failed to install code hook(Address 0x00)");

        // 자동 실행 모드 플래그 (true = 자동, false = F10 스텝)
        let auto_run = Arc::new(AtomicBool::new(true));
        let auto_run_hook = auto_run.clone();

        let mut inst_count = 0u64;
        self.add_code_hook(0, -1i64 as u64, move |uc, addr, size| {
            inst_count += 1;
            let is_auto = auto_run_hook.load(Ordering::Relaxed);

            // 1. 레지스터 읽기
            let regs = [
                uc.reg_read(RegisterX86::EAX).unwrap() as u32,
                uc.reg_read(RegisterX86::EBX).unwrap() as u32,
                uc.reg_read(RegisterX86::ECX).unwrap() as u32,
                uc.reg_read(RegisterX86::EDX).unwrap() as u32,
                uc.reg_read(RegisterX86::ESI).unwrap() as u32,
                uc.reg_read(RegisterX86::EDI).unwrap() as u32,
                uc.reg_read(RegisterX86::EBP).unwrap() as u32,
                uc.reg_read(RegisterX86::ESP).unwrap() as u32,
                uc.reg_read(RegisterX86::EIP).unwrap() as u32,
            ];

            if regs[8] == 0 {
                crate::emu_log!(
                    "\n[!] Detected execution at 0x00. Assuming successful return from function."
                );
                crate::emu_log!("    (Cause: Stack pointer drift due to stdcall mismatch)");

                // Crash 직전 상태 전송
                let esp = regs[7] as u64;
                let mut stack = Vec::new();
                let mut buf = [0u8; 4];
                for i in 0..10 {
                    let target = esp + (i as u64 * 4);
                    if uc.mem_read(target, &mut buf).is_ok() {
                        stack.push((target as u32, u32::from_le_bytes(buf)));
                    }
                }
                let _ = state_tx.send(CpuContext {
                    regs,
                    stack,
                    next_instr: "00 (Crash)".to_string(),
                });

                uc.emu_stop().unwrap();
                return;
            }

            let mut send_state = false;

            if is_auto {
                // === 자동 실행 모드 ===
                if inst_count % 10000 == 0 {
                    send_state = true;
                }

                // 비차단으로 Pause 명령만 확인
                match cmd_rx.try_recv() {
                    Ok(DebugCommand::Pause) => {
                        auto_run_hook.store(false, Ordering::Relaxed);
                        crate::emu_log!("[DEBUG] Paused (switched to step mode)");
                        send_state = true;
                    }
                    Ok(DebugCommand::Stop) => {
                        uc.emu_stop().unwrap();
                        return;
                    }
                    _ => {}
                }
            } else {
                // 스텝 모드: 항상 갱신
                send_state = true;
            }

            if send_state || !is_auto {
                // 2. 스택 읽기 (Top 10)
                let esp = regs[7] as u64;
                let mut stack = Vec::new();
                let mut buf = [0u8; 4];
                for i in 0..10 {
                    let target = esp + (i as u64 * 4);
                    if uc.mem_read(target, &mut buf).is_ok() {
                        stack.push((target as u32, u32::from_le_bytes(buf)));
                    }
                }

                // 3. 명령어 바이트 -> 문자열
                let mut code = vec![0u8; size as usize];
                let _ = uc.mem_read(addr, &mut code);
                let instr_str = code
                    .iter()
                    .map(|b| format!("{:02x}", b))
                    .collect::<Vec<_>>()
                    .join(" ");

                // 4. 상태 전송
                if state_tx
                    .send(CpuContext {
                        regs,
                        stack,
                        next_instr: instr_str,
                    })
                    .is_err()
                {
                    uc.emu_stop().unwrap();
                    return;
                }
            }

            // 5. 명령 대기 (스텝 모드에서만 블로킹)
            if !auto_run_hook.load(Ordering::Relaxed) {
                match cmd_rx.recv() {
                    Ok(DebugCommand::Step) => {} // 한 스텝 진행
                    Ok(DebugCommand::Run) => {
                        // 자동 실행 모드 전환
                        auto_run_hook.store(true, Ordering::Relaxed);
                        crate::emu_log!("[DEBUG] Running (auto-run mode)");
                    }
                    Ok(DebugCommand::Pause) => {} // 이미 스텝 모드
                    _ => {
                        uc.emu_stop().unwrap();
                    } // Stop or Error
                }
            }
        })
        .expect("Failed to install code hook.");

        // Error Debug Hook
        self.add_mem_hook(
            HookType::MEM_READ_UNMAPPED | HookType::MEM_WRITE_UNMAPPED | HookType::MEM_FETCH_UNMAPPED,
            0,
            -1i64 as u64,
            |_uc, access, addr, size, value|
        {
            if value == 0 {
                crate::emu_log!("\n[!] Detected execution at 0x00. Assuming successful return from function.");
                crate::emu_log!("    (Cause: Stack pointer drift due to stdcall mismatch)");
                crate::emu_log!("    Address: {addr:#010x}");

                return false;
            }

            crate::emu_log!("\n[!!!!!!] MEMORY ERROR DETECTED: {:?} at {:#x} (Size: {})", access, addr, size);

            match access {
                unicorn_engine::unicorn_const::MemType::READ_UNMAPPED => print!("    Type: READ_UNMAPPED"),
                unicorn_engine::unicorn_const::MemType::WRITE_UNMAPPED => print!("    Type: WRITE_UNMAPPED"),
                unicorn_engine::unicorn_const::MemType::FETCH_UNMAPPED => print!("    Type: FETCH_UNMAPPED"),
                _ => print!("    Type: Unknown"),
            }

            crate::emu_log!("    Trying to address: {:#010x}", value); // 시도한 주소 값

            false
        }).expect("Failed to install memory hook");

        Ok(())
    }

    fn load_dll_with_reloc(&mut self, filename: &str, target_base: u64) -> Result<LoadedDll, ()> {
        // 1. 파일 읽기 및 파싱
        let buffer = fs::read(filename).expect("파일을 찾을 수 없습니다.");
        let pe = PE::parse(&buffer).expect("PE 파싱 실패");

        // 2. 메모리 매핑
        // image_size는 페이지 크기(4KB) 단위로 정렬해주는 게 안전
        let image_size = pe
            .header
            .optional_header
            .unwrap()
            .windows_fields
            .size_of_image as u64;
        let size_4095 = SIZE_4KB - 1;
        let aligned_size = (image_size + size_4095) & !size_4095;

        self.mem_map(target_base, aligned_size, Prot::ALL)
            .expect("메모리 매핑 실패");
        crate::emu_log!(
            "Load: {} at {:#x} (Size: {:#x})",
            filename,
            target_base,
            image_size
        );

        // 3. 섹션 복사
        for section in pe.sections {
            let start = target_base + section.virtual_address as u64;
            let data_start = section.pointer_to_raw_data as usize;
            let data_size = section.size_of_raw_data as usize;
            if data_size == 0 {
                continue;
            }

            let data = &buffer[data_start..data_start + data_size];
            self.mem_write(start, data).expect("섹션 데이터 쓰기 실패");
        }

        // 4. IAT 패치
        let original_base = pe.image_base as u64;

        if original_base != target_base {
            crate::emu_log!(
                "    Relocating from 0x{:x} to 0x{:x}...",
                original_base,
                target_base
            );
            // let delta = (target_base as i64 - original_base as i64) as u64; // 차이값
            let delta = target_base.wrapping_sub(original_base);

            // PE 헤더에서 재배치 정보 파싱
            if let Some(opt) = pe.header.optional_header {
                if let Some(reloc_dir) = opt.data_directories.get_base_relocation_table() {
                    let mut reloc_rva = reloc_dir.virtual_address as usize;
                    let reloc_end = reloc_rva + reloc_dir.size as usize;

                    // .reloc 섹션 데이터 읽기 (메모리에서 읽는 게 편함)
                    while reloc_rva < reloc_end {
                        let mut block_header = [0u8; 8]; // VA(4) + Size(4)
                        self.mem_read(target_base + reloc_rva as u64, &mut block_header)
                            .unwrap();

                        let page_rva = u32::from_le_bytes(block_header[0..4].try_into().unwrap());
                        let block_size = u32::from_le_bytes(block_header[4..8].try_into().unwrap());

                        if block_size == 0 {
                            break;
                        } // Safety break

                        let entries_count = (block_size as usize - 8) / 2;
                        let mut entries_buf = vec![0u8; (block_size - 8) as usize];
                        self.mem_read(target_base + reloc_rva as u64 + 8, &mut entries_buf)
                            .unwrap();

                        for i in 0..entries_count {
                            let entry = u16::from_le_bytes(
                                entries_buf[i * 2..(i + 1) * 2].try_into().unwrap(),
                            );
                            let reloc_type = entry >> 12; // 상위 4비트
                            let offset = entry & 0x0FFF; // 하위 12비트

                            // IMAGE_REL_BASED_HIGHLOW (3) 인 경우만 수정 (32bit)
                            if reloc_type == 3 {
                                let target_addr = target_base + page_rva as u64 + offset as u64;
                                let mut val_buf = [0u8; 4];
                                self.mem_read(target_addr, &mut val_buf).unwrap();
                                let original_val = u32::from_le_bytes(val_buf);

                                // 값 수정: 원래값 + delta
                                let new_val = original_val.wrapping_add(delta as u32);
                                self.mem_write(target_addr, &new_val.to_le_bytes()).unwrap();
                            }
                        }
                        reloc_rva += block_size as usize;
                    }
                }
            }
        }

        // 4. Export Table 파싱 (다른 DLL이 이 함수들을 찾을 수 있게)
        let mut exports = HashMap::new();
        for export in pe.exports {
            if let Some(name) = export.name {
                let addr = target_base + export.rva as u64;
                exports.insert(name.to_string(), addr);
                // crate::emu_log!("    Export: {} -> 0x{:x}", name, addr);
            }
        }

        let entry_point = target_base
            + pe.header
                .optional_header
                .unwrap()
                .standard_fields
                .address_of_entry_point as u64;

        Ok(LoadedDll {
            name: filename.to_string(),
            base_addr: target_base,
            // size: aligned_size as usize,
            entry_point,
            exports,
        })
    }

    fn resolve_imports(&mut self, target: &LoadedDll) -> Result<(), ()> {
        // 타겟 DLL 파일 다시 파싱 (Import Directory 찾기 위해)
        let buffer = fs::read(&target.name).unwrap();
        let pe = PE::parse(&buffer).unwrap();
        let image_base = target.base_addr; // 주의: 파일의 image_base가 아니라 로드된 base 사용
        let dll_name = target.name.split('/').last().unwrap().to_string();

        // Fake Address Counter (스태틱처럼 사용하기 위해 고정값 + 오프셋 방식 권장)

        if let Some(opt) = pe.header.optional_header {
            if let Some(import_dir) = opt.data_directories.get_import_table() {
                if import_dir.size == 0 {
                    // crate::emu_log!("[DEBUG] Import Directory size is 0!");
                    return Ok(());
                }

                let mut desc_addr = image_base + import_dir.virtual_address as u64;
                // crate::emu_log!("[DEBUG] Import Descriptor Table at {desc_addr:#x}"); // 로그 추가

                loop {
                    let mut desc_buf = [0u8; 20];
                    if self.mem_read(desc_addr, &mut desc_buf).is_err() {
                        break;
                    }

                    let orig_first_thunk = u32::from_le_bytes(desc_buf[0..4].try_into().unwrap());
                    let name_rva = u32::from_le_bytes(desc_buf[12..16].try_into().unwrap());
                    let first_thunk = u32::from_le_bytes(desc_buf[16..20].try_into().unwrap());

                    if orig_first_thunk == 0 && first_thunk == 0 {
                        break;
                    }

                    let dll_name = self.read_string(image_base + name_rva as u64);
                    // crate::emu_log!("[DEBUG] Processing Import DLL: {dll_name}"); // 로그 추가

                    // 의존성 DLL인지 확인 (Case-insensitive)
                    // let dependency = dependencies.iter().find(|(name, _)| name.eq_ignore_ascii_case(&dll_name)).map(|(_, dll)| dll);

                    let mut ilt_rva = if orig_first_thunk != 0 {
                        orig_first_thunk
                    } else {
                        first_thunk
                    };
                    let mut iat_rva = first_thunk;

                    loop {
                        // let mut val_buf = [0u8; 4];
                        // self.mem_read(image_base + ilt_rva as u64, &mut val_buf).unwrap();
                        // let val = u32::from_le_bytes(val_buf);
                        let val = self.read_u32(image_base + ilt_rva as u64);
                        // crate::emu_log!("rva: {:#x}, address: {:#x}", ilt_rva, image_base + ilt_rva as u64);
                        if val == 0 {
                            break;
                        }

                        let func_name = if (val & 0x80000000) != 0 {
                            format!("Ordinal_{}", val & 0xFFFF)
                        } else {
                            self.read_string(image_base + val as u64 + 2)
                        };

                        let iat_addr = image_base + iat_rva as u64;
                        let mut final_addr = 0;

                        // 1. 의존성 DLL에 있는 함수인가?
                        let context = self.get_data();
                        context
                            .dll_modules
                            .lock()
                            .unwrap()
                            .iter()
                            .find(|(name, dll)| {
                                if name.eq_ignore_ascii_case(&dll_name) == false {
                                    return false;
                                }
                                if let Some(real_addr) = dll.exports.get(&func_name) {
                                    final_addr = *real_addr;
                                }
                                return true;
                            });
                        // if let Some(real_addr) = dll.exports.get(&func_name) {
                        //     final_addr = *real_addr;
                        // }
                        // if let Some(dep_dll) = dependency {
                        //     if let Some(real_addr) = dep_dll.exports.get(&func_name) {
                        //         final_addr = *real_addr;
                        //         // crate::emu_log!("    Linked: {}!{} -> 0x{:x}", dll_name, func_name, final_addr);
                        //     }
                        // }

                        // 2. 없다면 Fake Address 할당 (전역 카운터 사용)
                        if final_addr == 0 {
                            final_addr =
                                context.import_address.fetch_add(4, Ordering::SeqCst) as u64;
                        }

                        context
                            .address_map
                            .lock()
                            .unwrap()
                            .insert(final_addr, format!("{dll_name}!{func_name}"));

                        // [디버그] 패치하는 주소와 값 출력
                        // crate::emu_log!(
                        //     "[DEBUG] Patching IAT at 0x{:x} -> 0x{:x} ({})",
                        //     iat_addr,
                        //     final_addr,
                        //     func_name
                        // );

                        self.write_u32(iat_addr, final_addr as u32);
                        // self.mem_write(iat_addr, &(final_addr as u32).to_le_bytes()).unwrap();

                        ilt_rva += 4;
                        iat_rva += 4;
                    }
                    desc_addr += 20;
                }
            }
        }

        {
            let context = self.get_data();
            context
                .dll_modules
                .lock()
                .unwrap()
                .insert(dll_name.clone(), target.clone());
        }

        Ok(())
    }

    fn run_dll_entry(&mut self, dll: &LoadedDll) -> Result<(), ()> {
        if dll.entry_point == 0 {
            return Ok(());
        }

        let esp = self.reg_read(RegisterX86::ESP as i32).unwrap();

        // entry(hInstance, fdwReason, lpReserved)
        // x86 cdecl/stdcall: 마지막 인자를 먼저 push
        self.push_u32(0u32); // lpReserved (arg3 - 마지막 인자 먼저)
        self.push_u32(1u32); // fdwReason = DLL_PROCESS_ATTACH (arg2)
        self.push_u32(dll.base_addr as u32); // hInstance (arg1)
        self.push_u32(EXIT_ADDRESS as u32); // return address

        crate::emu_log!("[*] Function address: 0x{:x}", dll.entry_point);
        crate::emu_log!("[*] Calling entry(0x{:x}, 1, 0)...", dll.entry_point);

        // entry 오류나도 무시하고 진행
        // self.emu_start(dll.entry_point, EXIT_ADDRESS, 0, 0).unwrap_err();

        if let Err(e) = self.emu_start(dll.entry_point, EXIT_ADDRESS, 0, 0) {
            crate::emu_log!("\n[!] Emulation stopped/failed: {e:?}");
            let pc = self.reg_read(RegisterX86::EIP).unwrap();
            crate::emu_log!("    Stopped at EIP: {pc:#x}\n");
        } else {
            crate::emu_log!("[*] {} finished successfully.", dll.name);
        }

        // Stack 복구 (간이)
        self.reg_write(RegisterX86::ESP, esp).unwrap();

        Ok(())
    }

    fn run_dll_func(&mut self, dll_name: &str, func_name: &str, args: Vec<Box<dyn Any>>) {
        crate::emu_log!("\n[*] Looking for '{func_name}' in {dll_name}...");

        let func_address = {
            let context = self.get_data();
            context
                .dll_modules
                .lock()
                .unwrap()
                .get(dll_name)
                .and_then(|module| module.exports.get(func_name).copied())
        };

        if let Some(func_address) = func_address {
            let esp = self.reg_read(RegisterX86::ESP as i32).unwrap();

            let mut arg_strings = Vec::new();

            // x86 calling convention: 마지막 인자를 먼저 push
            // 먼저 모든 인자를 수집한 뒤 역순으로 push
            let mut push_values: Vec<u32> = Vec::new();
            for arg in args.iter() {
                if let Some(v) = arg.downcast_ref::<i32>() {
                    push_values.push(*v as u32);
                    arg_strings.push(format!("{v}"));
                } else if let Some(v) = arg.downcast_ref::<u32>() {
                    push_values.push(*v);
                    arg_strings.push(format!("{v}"));
                } else if let Some(v) = arg.downcast_ref::<&str>() {
                    let str_ptr = self.alloc_str(*v);
                    push_values.push(str_ptr);
                    arg_strings.push(format!("\"{v}\""));
                }
            }
            let arguments = arg_strings.join(", ");

            // 역순으로 push (마지막 인자부터)
            for val in push_values.iter().rev() {
                self.push_u32(*val);
            }
            self.push_u32(EXIT_ADDRESS as u32); // return address

            crate::emu_log!("[*] Function address: {func_address:#x}");
            crate::emu_log!("[*] Calling {func_name}({arguments})...");

            if let Err(e) = self.emu_start(func_address as u64, EXIT_ADDRESS, 0, 0) {
                crate::emu_log!("\n[!] Emulation stopped/failed: {e:?}");
                let pc = self.reg_read(RegisterX86::EIP).unwrap();
                crate::emu_log!("    Stopped at EIP: {pc:#x}\n");
            } else {
                crate::emu_log!("[*] {func_name} finished successfully.");
            }

            // Stack 복구 (간이)
            self.reg_write(RegisterX86::ESP, esp).unwrap();
        }
    }

    fn read_u32(&self, addr: u64) -> u32 {
        let data = self.mem_read_as_vec(addr, 4).expect("메모리 읽기 실패");
        u32::from_le_bytes(data.try_into().unwrap())
    }

    fn write_u32(&mut self, addr: u64, value: u32) {
        self.mem_write(addr, &value.to_le_bytes())
            .expect("메모리 쓰기 실패");
    }

    // 스택에서 n번째 인자 읽기 (stdcall 기준)
    fn read_arg(&self, index: usize) -> u32 {
        let esp = self.reg_read(RegisterX86::ESP as i32).unwrap();
        // ESP에는 리턴 주소가 있으므로, 인자는 ESP + 4 + (index * 4) 위치에 있음
        let addr = esp + 4 + (index as u64 * 4);

        let val_bytes = self.mem_read_as_vec(addr, 4).unwrap();
        u32::from_le_bytes(val_bytes.try_into().unwrap())
    }

    fn read_string(&self, addr: u64) -> String {
        let mut chars = Vec::new();
        let mut curr = addr;

        loop {
            let byte = self.mem_read_as_vec(curr, 1).unwrap()[0];
            if byte == 0 {
                break;
            } // NULL 문자 만나면 종료
            chars.push(byte);
            curr += 1;

            // 안전장치: 너무 길면 끊기 (예: 1KB)
            if chars.len() > 1024 {
                break;
            }
        }
        String::from_utf8_lossy(&chars).to_string()
    }

    fn write_string(&mut self, addr: u64, text: &str) {
        let bytes = text.as_bytes();
        self.mem_write(addr, bytes).expect("메모리 쓰기 실패");
    }

    fn read_euc_kr(&self, addr: u64) -> String {
        let mut chars = Vec::new();
        let mut curr = addr;

        loop {
            let byte = self.mem_read_as_vec(curr, 1).unwrap()[0];
            if byte == 0 {
                break;
            } // NULL 문자 만나면 종료
            chars.push(byte);
            curr += 1;

            // 안전장치: 너무 길면 끊기 (예: 1KB)
            if chars.len() > 1024 {
                break;
            }
        }
        let (res, _, _) = EUC_KR.decode(&chars);
        res.to_string()
    }

    fn push_u32(&mut self, value: u32) {
        // 1. ESP 감소 (Stack grows down)
        let esp = self.reg_read(RegisterX86::ESP as i32).unwrap();
        let new_esp = esp - 4;

        // 2. 값 쓰기
        self.write_u32(new_esp, value);

        // 3. ESP 업데이트
        self.reg_write(RegisterX86::ESP as i32, new_esp).unwrap();
    }

    fn pop_u32(&mut self) -> u32 {
        // 1. 현재 ESP 위치의 값 읽기
        let esp = self.reg_read(RegisterX86::ESP as i32).unwrap();
        let value = self.read_u32(esp);

        // 2. ESP 증가
        self.reg_write(RegisterX86::ESP as i32, esp + 4).unwrap();

        value
    }

    fn apply_stack_cleanup(&mut self, cleanup: StackCleanup) {
        if let Some(new_esp) =
            stack_cleanup_target_esp(self.reg_read(RegisterX86::ESP as i32).unwrap(), cleanup)
        {
            // 현재 스택: [ret_addr][arg1][arg2]...
            // 목표:      [ret_addr] ← ESP (args 제거됨)
            let esp = self.reg_read(RegisterX86::ESP as i32).unwrap();
            let ret_addr = self.read_u32(esp);
            self.write_u32(new_esp, ret_addr);
            self.reg_write(RegisterX86::ESP as i32, new_esp).unwrap();
        }
    }

    fn malloc(&mut self, size: usize) -> u64 {
        let data = self.get_data();
        let addr = data.heap_cursor.load(Ordering::SeqCst);

        // 4바이트 정렬 (속도와 안정성을 위해)
        let aligned_size = (size as u32 + 3) & !3;

        data.heap_cursor.fetch_add(aligned_size, Ordering::SeqCst);

        addr as u64
    }

    fn alloc_bytes(&mut self, data: &[u8]) -> u32 {
        // 1. 공간 확보
        let addr = self.malloc(data.len());
        // 2. 데이터 쓰기
        self.mem_write(addr, data).expect("힙 쓰기 실패");
        // 3. 주소 반환
        addr as u32
    }

    fn alloc_str(&mut self, text: &str) -> u32 {
        // Rust 문자열을 바이트로 변환 + NULL 문자(\0) 추가
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(0); // C-String Terminator

        self.alloc_bytes(&bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_stack_cleanup_preserves_caller_layout() {
        let esp_before = 0x5000_0100;
        assert_eq!(
            stack_cleanup_target_esp(esp_before, StackCleanup::Caller),
            None
        );
        assert_eq!(
            stack_cleanup_final_esp(esp_before, StackCleanup::Caller),
            esp_before + 4
        );
    }

    #[test]
    fn apply_stack_cleanup_advances_callee_layout() {
        let esp_before = 0x5000_0100;
        assert_eq!(
            stack_cleanup_target_esp(esp_before, StackCleanup::Callee(3)),
            Some(esp_before + 12)
        );
        assert_eq!(
            stack_cleanup_final_esp(esp_before, StackCleanup::Callee(3)),
            esp_before + 16
        );
    }
}
