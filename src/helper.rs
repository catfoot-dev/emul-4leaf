use crate::{
    debug::common::{CpuContext, DebugCommand},
    dll::win32::{LoadedDll, StackCleanup, Win32Context, kernel32::KERNEL32},
};
use chardetng::EncodingDetector;
use encoding_rs::EUC_KR;
use goblin::pe::PE;
use std::{
    any::Any,
    collections::HashMap,
    fs,
    sync::{
        atomic::Ordering,
        mpsc::{Receiver, Sender},
    },
    time::{Duration, Instant},
    u8, vec,
};
use unicorn_engine::{HookType, Prot, RegisterX86, Unicorn};

// pub const HOOK_BASE: u64 = 0x1000_0000;
pub const HEAP_BASE: u64 = 0x2000_0000;
pub const HEAP_SIZE: u64 = 256 * 1024 * 1024;

pub const STACK_BASE: u64 = 0x5000_0000;
pub const STACK_SIZE: u64 = 1024 * 1024;
pub const STACK_TOP: u64 = STACK_BASE + STACK_SIZE;

pub const SHARED_MEM_BASE: u64 = 0x7000_0000;

// const FUNCTION_NAME_BASE: u64 = 0x8000_0000;

pub const TEB_BASE: u64 = 0x9000_0000;
pub const FAKE_IMPORT_BASE: u64 = 0xF000_0000;
pub const EXIT_ADDRESS: u64 = 0xFFFF_FFFF;

const SIZE_4KB: u64 = 4 * 1024;
const DEBUG_AUTO_QUANTUM: usize = 200_000;
const DEBUG_STEP_QUANTUM: usize = 1;
const DEBUG_STATE_SEND_INTERVAL: Duration = Duration::from_millis(250);

/// 함수 호출 규약에 따른 스택 정리(Cleanup) 시 이동해야 할 ESP의 상대적 위치를 계산합니다.
///
/// # 인자
/// * `esp`: 현재 스택 포인터(ESP) 값
/// * `cleanup`: 적용할 스택 정리 방식 (Caller 또는 Callee)
///
/// # 반환
/// * `Option<u64>`: 정리가 필요한 경우 대상 ESP 주소, 필요 없는 경우 `None`
fn stack_cleanup_target_esp(esp: u64, cleanup: StackCleanup) -> Option<u64> {
    match cleanup {
        StackCleanup::Caller | StackCleanup::Callee(0) => None,
        StackCleanup::Callee(arg_count) => Some(esp + (arg_count as u64 * 4)),
    }
}

/// 함수 호출이 완전히 끝난 후 리턴 주소(RET)까지 정리된 최종 ESP 값을 계산합니다.
///
/// # 인자
/// * `esp`: 현재 스택 포인터 값
/// * `cleanup`: 적용할 스택 정리 방식
fn stack_cleanup_final_esp(esp: u64, cleanup: StackCleanup) -> u64 {
    stack_cleanup_target_esp(esp, cleanup).unwrap_or(esp) + 4
}

fn capture_cpu_context(uc: &mut Unicorn<Win32Context>) -> CpuContext {
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

/// Unicorn 객체에 추가할 메소드 목록 정의
///
/// Unicorn 엔진을 확장하여 Win32 에뮬레이션에 필요한 메모리 조작, 스택 제어, DLL 로딩 등을 지원하는 헬퍼 트레잇
pub trait UnicornHelper {
    /// 에뮬레이터의 초기 환경을 구성
    /// 스택, 힙, 통신 채널 등을 설정하고 기본적인 API 후킹 준비를 완료
    ///
    /// # 인자
    /// * `state_tx`: UI(디버거)로 CPU 상태(`CpuContext`)를 전송하기 위한 Sender. 비디버그 모드에서는 `None`
    /// * `cmd_rx`: UI로부터 디버깅 명령어(`DebugCommand`)를 수신하기 위한 Receiver. 비디버그 모드에서는 `None`
    ///
    /// # 반환
    /// * `Result<(), ()>`: 성공 시 `Ok(())`, 메모리 매핑 요류 등 실패 시 `Err(())`
    fn setup(
        &mut self,
        state_tx: Option<Sender<CpuContext>>,
        cmd_rx: Option<Receiver<DebugCommand>>,
    ) -> Result<(), ()>;

    /// 지정된 경로의 DLL 파일을 메모리에 로드하고 재배치(Relocation)를 수행
    ///
    /// # 인자
    /// * `filename`: 로드할 DLL 파일의 경로
    /// * `target_base`: DLL을 로드할 대상 기준 메모리 주소 (ImageBase)
    ///
    /// # 반환
    /// * `Result<LoadedDll, ()>`: 로드가 성공하면 DLL의 메타데이터(`LoadedDll`)를 반환
    fn load_dll_with_reloc(&mut self, filename: &str, target_base: u64) -> Result<LoadedDll, ()>;

    /// 로드된 DLL의 Import Address Table (IAT)을 분석하여 의존성 함수 주소들을 채움
    ///
    /// # 인자
    /// * `target`: 분석할 메모리상의 DLL 객체(`LoadedDll`)의 참조
    ///
    /// # 반환
    /// * `Result<(), ()>`: 성공적으로 IAT를 구성하면 `Ok(())`
    fn resolve_imports(&mut self, target: &LoadedDll) -> Result<(), ()>;

    /// DLL의 엔트리 포인트(`DllMain`) 함수를 실행
    ///
    /// # 인자
    /// * `dll`: 엔트리 포인트를 실행할 DLL
    ///
    /// # 반환
    /// * `Result<(), ()>`: 실행 성공 시 `Ok(())`
    fn run_dll_entry(&mut self, dll: &LoadedDll) -> Result<(), ()>;

    /// 특정 DLL 내의 지정된 함수를 여러 인자와 함께 직접 호출 (테스트 및 특정 API 직접 실행용)
    ///
    /// # 인자
    /// * `dll_name`: 호출할 함수가 포함된 DLL 이름
    /// * `func_name`: 호출할 대상 함수의 이름 (예: `Main`)
    /// * `args`: 함수에 전달될 인자들 모음(`Any` 박스 형태)
    fn run_dll_func(&mut self, dll_name: &str, func_name: &str, args: Vec<Box<dyn Any>>);

    /// 메인 에뮬레이터 루프를 실행 (tid=0과 백그라운드 스레드를 교차 실행)
    fn run_emulator(
        &mut self,
        dll_name: &str,
        func_name: &str,
        args: Vec<Box<dyn Any>>,
        state_tx: Option<Sender<CpuContext>>,
        cmd_rx: Option<Receiver<DebugCommand>>,
    );

    /// 함수 호출을 위한 스택 및 EIP 환경을 준비 (실제 에뮬레이션은 시작하지 않음)
    fn prepare_dll_func(&mut self, dll_name: &str, func_name: &str, args: Vec<Box<dyn Any>>);

    // === 메모리 읽기/쓰기 (Heap/General) ===

    /// 특정 메모리 주소에서 32비트(4바이트) 정수를 리틀 엔디안(Little Endian)으로 읽음
    ///
    /// # 인자
    /// * `addr`: 읽고자 하는 메모리 주소
    /// # 반환
    /// * `u32`: 해당 주소에 저장된 32비트 값
    fn read_u32(&self, addr: u64) -> u32;
    fn read_i32(&self, addr: u64) -> i32;

    /// 특정 메모리 주소에서 16비트(2바이트) 정수를 리틀 엔디안(Little Endian)으로 읽음
    ///
    /// # 인자
    /// * `addr`: 읽고자 하는 메모리 주소
    /// # 반환
    /// * `u16`: 해당 주소에 저장된 16비트 값
    fn read_u16(&self, addr: u64) -> u16;

    fn write_u32(&mut self, addr: u64, value: u32);

    /// 특정 메모리 주소에 16비트(2바이트) 정수를 리틀 엔디안 방식으로 기록
    ///
    /// # 인자
    /// * `addr`: 기록하고자 하는 메모리 대상 주소
    /// * `value`: 기록할 16비트 정수 값
    fn write_u16(&mut self, addr: u64, value: u16);

    /// 함수 호출 시 전달된 인자(스택) 중 N번째 인자를 읽음 (cdecl/stdcall 호출 규약 기준)
    /// `index = 0` 일 때 첫 번째 인자 값 반환
    ///
    /// # 인자
    /// * `index`: 가져올 인자의 인덱스 번호 (0부터 시작)
    /// # 반환
    /// * `u32`: 해당 인자의 32비트 값
    fn read_arg(&self, index: usize) -> u32;

    /// C언어 스타일 널 종료 문자열(Null-Terminated String)을 읽어와서 Rust의 `String`으로 반환 (기본 ASCII/UTF-8 형태)
    ///
    /// # 인자
    /// * `addr`: 문자열 처리가 시작될 메모리 주소
    fn read_string(&self, addr: u64) -> String;
    fn read_u8(&self, addr: u64) -> u8;
    fn write_u8(&mut self, addr: u64, value: u8);

    /// 대상 메모리에서 페이지 경계를 고려하여 지정된 최대 길이까지 바이트 배열을 읽어옴
    ///
    /// # 인자
    /// * `addr`: 문자열 처리가 시작될 메모리 주소
    /// * `max_len`: 읽어올 최대 바이트 수
    fn read_string_bytes(&self, addr: u64, max_len: usize) -> Vec<u8>;

    /// 대상 메모리에 C언어 스타일 널 종료 문자열(Null-Terminated String)을 기록
    ///
    /// # 인자
    /// * `addr`: 문자열을 기록할 메모리 주소
    /// * `text`: 기록할 Rust 문자열 레퍼런스(`&str`)
    fn write_string(&mut self, addr: u64, text: &str);

    /// 대상 메모리에 ANSI(EUC-KR) 기반 널 종료 문자열을 기록합니다.
    ///
    /// # 인자
    /// * `addr`: 문자열을 기록할 메모리 주소
    /// * `text`: 기록할 Rust 문자열 레퍼런스(`&str`)
    fn write_euc_kr(&mut self, addr: u64, text: &str);

    /// 대상 메모리의 널 종료 문자열을 읽고, 내용이 EUC-KR 문자셋일 경우 디코딩하여 Rust `String`으로 반환 (한국어 호환 프로그램용)
    ///
    /// # 인자
    /// * `addr`: 문자열의 메모리 주소
    fn read_euc_kr(&self, addr: u64) -> String;

    // === 스택 조작 (Stack) ===

    /// 스택 포인터(`ESP`)를 4바이트 감소시키고 그 위치에 32비트 값을 `push`
    ///
    /// # 인자
    /// * `value`: 스택에 넣을 32비트 값
    fn push_u32(&mut self, value: u32);

    /// 스택에서 현재 `ESP`가 가리키는 32비트 값을 `pop` 하고, 스택 포인터를 4바이트 증가시킴
    ///
    /// # 반환
    /// * `u32`: 스택에서 뽑아낸(Pop된) 최상단 값
    fn pop_u32(&mut self) -> u32;

    /// Callee-cleanup(`stdcall`) 방식 등, 함수 실행이 끝난 뒤 스택을 안전하게 정리(보정)
    ///
    /// # 인자
    /// * `cleanup`: 정리할 바이트 수 혹은 Caller가 회수할 지 등을 묘사한 열거형
    fn apply_stack_cleanup(&mut self, cleanup: StackCleanup);

    /// 인자로 들어온 크기만큼 힙 메모리 영역에서 공간을 할당받음. 초기화되지 않은 메모리 주소가 반환됨
    ///
    /// # 인자
    /// * `size`: 할당할 바이트 수
    /// # 반환
    /// * `u64`: 할당된 힙 영역의 64비트 가상 주소(내부적으로 32비트 대역을 씀)
    fn malloc(&mut self, size: usize) -> u64;

    /// 문자열을 새로 힙 공간에 할당하고, 그 끝에 문자열 터미네이터(`\0`)를 자동으로 추가
    ///
    /// # 인자
    /// * `text`: 힙에 기록할 Rust 문자열 레퍼런스(`&str`)
    /// # 반환
    /// * `u32`: 생성된 C-Style 문자열이 기록된 힙의 32비트 주소
    fn alloc_str(&mut self, text: &str) -> u32; // 32비트 주소 반환

    /// 임의의 바이트 배열(특정 구조체 데이터 등)을 힙 공간을 할당받아 그대로 기록
    ///
    /// # 인자
    /// * `data`: 메모리에 복사할 바이트 슬라이스(`&[u8]`)
    /// # 반환
    /// * `u32`: 데이터가 복사된 힙의 32비트 주소
    fn alloc_bytes(&mut self, data: &[u8]) -> u32;

    /// 주어진 주소가 어떤 DLL의 어느 오프셋에 해당하는지 해석하여 문자열로 반환
    /// 가장 가까운 export 심볼이 있으면 함께 표시 (예: "4Leaf.dll+0x1234 (near Main+0x10)")
    fn resolve_address(&self, addr: u32) -> String;

    /// 대상 메모리에 32비트 정수 배열을 리틀 엔디안 방식으로 기록 (RECT 등 구조체 처리용)
    ///
    /// # 인자
    /// * `addr`: 기록할 메모리 시작 주소
    /// * `data`: 기록할 32비트 정수 슬라이스
    fn write_mem(&mut self, addr: u64, data: &[i32]);
}

// 모든 Unicorn<D> 타입에 대해 구현 (D는 Win32Context 등 무엇이든 가능)
impl UnicornHelper for Unicorn<'_, Win32Context> {
    /// 에뮬레이터의 초기 환경을 구성합니다.
    /// 스택, 힙, 통신 채널 등을 설정하고 기본적인 API 후킹 준비를 완료합니다.
    ///
    /// # 인자
    /// * `state_tx`: UI(디버거)로 CPU 상태(`CpuContext`)를 전송하기 위한 Sender. 비디버그 모드에서는 `None`
    /// * `cmd_rx`: UI로부터 디버깅 명령어(`DebugCommand`)를 수신하기 위한 Receiver. 비디버그 모드에서는 `None`
    ///
    /// # 반환
    /// * `Result<(), ()>`: 성공 시 `Ok(())`, 메모리 매핑 오류 등 실패 시 `Err(())`
    fn setup(
        &mut self,
        _state_tx: Option<Sender<CpuContext>>,
        _cmd_rx: Option<Receiver<DebugCommand>>,
    ) -> Result<(), ()> {
        // [1] 메모리 맵 설정
        crate::append_capture_line("emu.log", "[SETUP] map stack begin");
        self.mem_map(STACK_BASE, STACK_SIZE, Prot::ALL)
            .map_err(|e| crate::emu_log!("[!] Failed to map Stack: {:?}", e))?;
        crate::append_capture_line("emu.log", "[SETUP] map stack done");
        // 스택 오버플로우/경계 읽기 에러 방지 (스택 바로 뒤 4KB 추가 할당)
        crate::append_capture_line("emu.log", "[SETUP] map stack guard begin");
        self.mem_map(STACK_TOP, SIZE_4KB, Prot::ALL)
            .map_err(|e| crate::emu_log!("[!] Failed to map Stack Guard: {:?}", e))?;
        crate::append_capture_line("emu.log", "[SETUP] map stack guard done");

        crate::append_capture_line("emu.log", "[SETUP] map heap begin");
        self.mem_map(HEAP_BASE, HEAP_SIZE, Prot::ALL)
            .map_err(|e| crate::emu_log!("[!] Failed to map Heap: {:?}", e))?;
        crate::append_capture_line("emu.log", "[SETUP] map heap done");
        crate::append_capture_line("emu.log", "[SETUP] map shared mem begin");
        self.mem_map(SHARED_MEM_BASE, SIZE_4KB, Prot::ALL)
            .map_err(|e| crate::emu_log!("[!] Failed to map Shared Mem: {:?}", e))?;
        crate::append_capture_line("emu.log", "[SETUP] map shared mem done");

        // NULL 포인터 접근 방지 (0 ~ 128KB)
        // 읽기/쓰기만 허용하고 실행은 차단하여, EIP가 0으로 떨어졌을 때
        // FETCH_UNMAPPED 훅이 발동되어 호스트 프로세스 segfault를 방지합니다.
        crate::append_capture_line("emu.log", "[SETUP] map null page begin");
        self.mem_map(0, 0x2_0000, Prot::READ | Prot::WRITE)
            .map_err(|e| crate::emu_log!("[!] Failed to map Null Page: {:?}", e))?;
        crate::append_capture_line("emu.log", "[SETUP] map null page done");

        // [2] TEB (Thread Environment Block) 설정
        crate::append_capture_line("emu.log", "[SETUP] map teb begin");
        self.mem_map(TEB_BASE, SIZE_4KB, Prot::ALL)
            .map_err(|e| crate::emu_log!("[!] Failed to map TEB: {:?}", e))?;
        crate::append_capture_line("emu.log", "[SETUP] map teb done");
        // x86 SEH 체인의 끝은 `-1`이므로 초기 예외 리스트 헤더를 맞춰 둡니다.
        crate::append_capture_line("emu.log", "[SETUP] init teb contents begin");
        self.mem_write(TEB_BASE, &0xFFFF_FFFFu32.to_le_bytes())
            .map_err(|e| crate::emu_log!("[!] Failed to write TEB exception list: {:?}", e))?;
        // Self-pointer at TEB + 0x18
        self.mem_write(TEB_BASE + 0x18, &(TEB_BASE as u32).to_le_bytes())
            .map_err(|e| crate::emu_log!("[!] Failed to write TEB self-pointer: {:?}", e))?;
        // 현재 Unicorn 32-bit x86에서는 `FS_BASE` 쓰기가 no-op이므로, 게스트가 실제로
        // 사용하는 `fs:[0]` / `fs:[0x18]` 조회가 선형 주소 0 기반에서도 동작하도록
        // 최소 TEB 헤더를 저주소 별칭으로 함께 미러링합니다.
        self.mem_write(0, &0xFFFF_FFFFu32.to_le_bytes())
            .map_err(|e| crate::emu_log!("[!] Failed to mirror SEH head at null page: {:?}", e))?;
        self.mem_write(0x18, &(TEB_BASE as u32).to_le_bytes())
            .map_err(|e| {
                crate::emu_log!(
                    "[!] Failed to mirror TEB self-pointer at null page: {:?}",
                    e
                )
            })?;
        crate::append_capture_line("emu.log", "[SETUP] init teb contents done");

        // [3] Fake Import Area (API 후킹용 실행 영역)
        crate::append_capture_line("emu.log", "[SETUP] map fake import begin");
        self.mem_map(FAKE_IMPORT_BASE, 1024 * 1024, Prot::ALL | Prot::EXEC)
            .map_err(|e| crate::emu_log!("[!] Failed to map Fake Import Area: {:?}", e))?;
        crate::append_capture_line("emu.log", "[SETUP] map fake import done");
        // RET (0xC3) 으로 채우기: 코드 훅이 실행된 후 자연스럽게 RET로 복귀
        let ret_fill = vec![0xC3u8; 1024 * 1024];
        crate::append_capture_line("emu.log", "[SETUP] fill fake import begin");
        self.mem_write(FAKE_IMPORT_BASE, &ret_fill)
            .map_err(|e| crate::emu_log!("[!] Failed to fill Fake Import Area: {:?}", e))?;
        crate::append_capture_line("emu.log", "[SETUP] fill fake import done");

        // x86 세그먼트 레지스터(SS) 버그 방지를 위해 ESP를 페이지 경계에서 약간 띄움
        crate::append_capture_line("emu.log", "[SETUP] set initial esp begin");
        self.reg_write(RegisterX86::ESP, STACK_TOP - 0x1000)
            .map_err(|e| crate::emu_log!("[!] Failed to set initial ESP: {:?}", e))?;
        crate::append_capture_line("emu.log", "[SETUP] set initial esp done");

        // EXIT_ADDRESS(0xFFFFFFFF)로 return 시의 접근 예외 방용 영역
        crate::append_capture_line("emu.log", "[SETUP] map exit area begin");
        self.mem_map(0xFFFF_0000, 64 * 1024, Prot::ALL | Prot::EXEC)
            .map_err(|e| crate::emu_log!("[!] Failed to map Exit Area: {:?}", e))?;
        crate::append_capture_line("emu.log", "[SETUP] map exit area done");

        // [4] API Call Hook (Fake Address Range)
        // 0xF0000000 대역으로 점프 시 실행되는 훅
        crate::append_capture_line("emu.log", "[SETUP] add fake import hook begin");
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
                        let _ = uc.reg_write(RegisterX86::EIP, func_address as u64);
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
                            let final_esp = stack_cleanup_final_esp(esp_after, hook_result.cleanup);
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
        crate::append_capture_line("emu.log", "[SETUP] add fake import hook done");

        // [5] 전역 코드 훅 (JIT 블록 비활성화 + EIP=0 보호)
        // API 코드 훅에서 중첩 emu_start를 호출할 때 unicorn의 JIT 블록이 내부 상태를
        // 손상시킬 수 있으므로, 전체 주소 범위에 코드 훅을 설치하여 인터프리터 모드로 강제합니다.
        crate::append_capture_line("emu.log", "[SETUP] add global code hook begin");
        self.add_code_hook(
            0,
            u64::MAX,
            |uc: &mut Unicorn<Win32Context>, addr, _size| {
                if addr == 0x3100_23f4 {
                    let esp = uc.reg_read(RegisterX86::ESP).unwrap_or(0);
                    let eax = uc.reg_read(RegisterX86::EAX).unwrap_or(0) as u32;
                    let ecx = uc.reg_read(RegisterX86::ECX).unwrap_or(0) as u32;
                    let edx = uc.reg_read(RegisterX86::EDX).unwrap_or(0) as u32;
                    let esi = uc.reg_read(RegisterX86::ESI).unwrap_or(0) as u32;
                    let edi = uc.reg_read(RegisterX86::EDI).unwrap_or(0) as u32;
                    let frame = (0..10)
                        .map(|i| format!("{:#x}", uc.read_u32(esp + (i * 4))))
                        .collect::<Vec<_>>()
                        .join(", ");
                    crate::emu_log!(
                        "[TRACE] WinCore::TWindow::Create callsite EIP=0x310023f4 ESP={:#x} EAX={:#x} ECX={:#x} EDX={:#x} ESI={:#x} EDI={:#x} STACK=[{}]",
                        esp as u32,
                        eax,
                        ecx,
                        edx,
                        esi,
                        edi,
                        frame
                    );
                } else if addr == 0x3100_23fa {
                    let esp = uc.reg_read(RegisterX86::ESP).unwrap_or(0);
                    let eax = uc.reg_read(RegisterX86::EAX).unwrap_or(0) as u32;
                    let esi = uc.reg_read(RegisterX86::ESI).unwrap_or(0) as u32;
                    let hwnd_field = if esi >= 0x2000_0000 {
                        uc.read_u32(esi as u64 + 4)
                    } else {
                        0
                    };
                    crate::emu_log!(
                        "[TRACE] WinCore::TWindow::Create return EIP=0x310023fa ESP={:#x} EAX={:#x} ESI={:#x} [ESI+4]={:#x}",
                        esp as u32,
                        eax,
                        esi,
                        hwnd_field
                    );
                } else if addr == 0x3100_242f {
                    let eax = uc.reg_read(RegisterX86::EAX).unwrap_or(0) as u32;
                    let esi = uc.reg_read(RegisterX86::ESI).unwrap_or(0) as u32;
                    let hwnd_field = if esi >= 0x2000_0000 {
                        uc.read_u32(esi as u64 + 4)
                    } else {
                        0
                    };
                    crate::emu_log!(
                        "[TRACE] WinCore::TWindow::Create compare branch EAX={:#x} ESI={:#x} [ESI+4]={:#x}",
                        eax,
                        esi,
                        hwnd_field
                    );
                } else if addr == 0x3100_2434 {
                    let eax = uc.reg_read(RegisterX86::EAX).unwrap_or(0) as u32;
                    let esi = uc.reg_read(RegisterX86::ESI).unwrap_or(0) as u32;
                    let hwnd_field = if esi >= 0x2000_0000 {
                        uc.read_u32(esi as u64 + 4)
                    } else {
                        0
                    };
                    crate::emu_log!(
                        "[TRACE] WinCore::TWindow::Create throw path entered EAX={:#x} ESI={:#x} [ESI+4]={:#x}",
                        eax,
                        esi,
                        hwnd_field
                    );
                } else if addr == 0x3100_4964 || addr == 0x3100_35d0 {
                    let esp = uc.reg_read(RegisterX86::ESP).unwrap_or(0);
                    let ecx = uc.reg_read(RegisterX86::ECX).unwrap_or(0) as u32;
                    let esi = uc.reg_read(RegisterX86::ESI).unwrap_or(0) as u32;
                    let frame = (0..10)
                        .map(|i| format!("{:#x}", uc.read_u32(esp + (i * 4))))
                        .collect::<Vec<_>>()
                        .join(", ");
                    crate::emu_log!(
                        "[TRACE] WinCore child create direct USER32 call EIP={:#x} ESP={:#x} ECX={:#x} ESI={:#x} STACK=[{}]",
                        addr as u32,
                        esp as u32,
                        ecx,
                        esi,
                        frame
                    );
                }

                if addr == 0 {
                    crate::emu_log!("[!] Execution at address 0x0 detected. Stopping.");
                    uc.emu_stop().unwrap_or_default();
                }
            },
        )
        .map_err(|e| crate::emu_log!("[!] Failed to install global code hook: {:?}", e))?;
        crate::append_capture_line("emu.log", "[SETUP] add global code hook done");

        // [6] Unmapped Memory Access Hook
        crate::append_capture_line("emu.log", "[SETUP] add unmapped mem hook begin");
        self.add_mem_hook(
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
        crate::append_capture_line("emu.log", "[SETUP] add unmapped mem hook done");

        Ok(())
    }

    /// 지정된 경로의 DLL 파일을 메모리에 로드하고 재배치(Relocation)를 수행합니다.
    ///
    /// # 인자
    /// * `filename`: 로드할 DLL 파일의 경로
    /// * `target_base`: DLL을 로드할 대상 기준 메모리 주소 (ImageBase)
    ///
    /// # 반환
    /// * `Result<LoadedDll, ()>`: 로드가 성공하면 DLL의 메타데이터(`LoadedDll`)를 반환
    fn load_dll_with_reloc(&mut self, filename: &str, target_base: u64) -> Result<LoadedDll, ()> {
        // [1] DLL 파일 읽기
        let buffer = fs::read(filename).map_err(|e| {
            crate::emu_log!("[!] Failed to read DLL file {}: {:?}", filename, e);
        })?;

        // [2] PE 헤더 파싱
        let pe = PE::parse(&buffer).map_err(|e| {
            crate::emu_log!("[!] PE parsing failed for {}: {:?}", filename, e);
        })?;

        // [3] 메모리 매핑 (SizeOfImage 기준, 4KB 정렬)
        let image_size = pe
            .header
            .optional_header
            .ok_or_else(|| crate::emu_log!("[!] PE header missing optional header: {}", filename))?
            .windows_fields
            .size_of_image as u64;

        let aligned_size = (image_size + (SIZE_4KB - 1)) & !(SIZE_4KB - 1);

        self.mem_map(target_base, aligned_size, Prot::ALL)
            .map_err(|e| {
                crate::emu_log!(
                    "[!] Memory map failed for {} at {:#x}: {:?}",
                    filename,
                    target_base,
                    e
                );
            })?;

        crate::emu_log!(
            "[*] Loaded: {} at {:#x} (Size: {:#x})",
            filename,
            target_base,
            image_size
        );

        // [4] 섹션 데이터를 메모리에 복사
        for section in pe.sections {
            let start = target_base + section.virtual_address as u64;
            let data_start = section.pointer_to_raw_data as usize;
            let data_size = section.size_of_raw_data as usize;
            if data_size == 0 {
                continue;
            }

            // 버퍼 범위를 벗어나지 않는지 확인
            if data_start + data_size > buffer.len() {
                crate::emu_log!(
                    "[!] Section data out of bounds in {}: {}",
                    filename,
                    section.name().unwrap_or("?")
                );
                continue;
            }

            let data = &buffer[data_start..data_start + data_size];
            self.mem_write(start, data).map_err(|e| {
                crate::emu_log!(
                    "[!] Failed to write section {} at {:#x}: {:?}",
                    section.name().unwrap_or("?"),
                    start,
                    e
                );
            })?;
        }

        // [5] 기본 주소 재배치(Relocation) 수행
        let original_base = pe.image_base as u64;
        if original_base != target_base {
            let delta = target_base.wrapping_sub(original_base);

            if let Some(opt) = pe.header.optional_header
                && let Some(reloc_dir) = opt.data_directories.get_base_relocation_table()
            {
                let mut reloc_rva = reloc_dir.virtual_address as usize;
                let reloc_end = reloc_rva + reloc_dir.size as usize;

                while reloc_rva < reloc_end {
                    let mut block_header = [0u8; 8]; // VA(4) + Size(4)
                    if self
                        .mem_read(target_base + reloc_rva as u64, &mut block_header)
                        .is_err()
                    {
                        break;
                    }

                    let page_rva = u32::from_le_bytes(block_header[0..4].try_into().unwrap());
                    let block_size = u32::from_le_bytes(block_header[4..8].try_into().unwrap());

                    if block_size < 8 {
                        break;
                    } // 최소 헤더 크기보다 작으면 중단

                    let entries_count = (block_size as usize - 8) / 2;
                    let mut entries_buf = vec![0u8; (block_size - 8) as usize];
                    if self
                        .mem_read(target_base + reloc_rva as u64 + 8, &mut entries_buf)
                        .is_ok()
                    {
                        for i in 0..entries_count {
                            let entry = u16::from_le_bytes(
                                entries_buf[i * 2..(i + 1) * 2].try_into().unwrap(),
                            );
                            let reloc_type = entry >> 12; // 상위 4비트: 재배치 타입
                            let offset = entry & 0x0FFF; // 하위 12비트: 페이지 내 오프셋

                            // IMAGE_REL_BASED_HIGHLOW (3) 타입만 처리 (Win32 x86 핵심)
                            if reloc_type == 3 {
                                let target_addr = target_base + page_rva as u64 + offset as u64;
                                let mut val_buf = [0u8; 4];
                                if self.mem_read(target_addr, &mut val_buf).is_ok() {
                                    let original_val = u32::from_le_bytes(val_buf);
                                    let new_val = original_val.wrapping_add(delta as u32);
                                    let _ = self.mem_write(target_addr, &new_val.to_le_bytes());
                                }
                            }
                        }
                    }
                    reloc_rva += block_size as usize;
                }
            }
        }

        // [6] Export Table 정보를 수집하여 모듈 데이터 구성
        let mut exports = HashMap::new();
        for export in pe.exports {
            if let Some(name) = export.name {
                let addr = target_base + export.rva as u64;
                exports.insert(name.to_string(), addr);
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
            size: image_size,
            entry_point,
            exports,
        })
    }

    /// 로드된 DLL의 Import Address Table (IAT)을 분석하여 의존성 함수 주소들을 채웁니다.
    ///
    /// # 인자
    /// * `target`: IAT를 해결할 대상 DLL 객체 레퍼런스
    ///
    /// # 반환
    /// * `Result<(), ()>`: 성공적으로 모든 임포트를 처리하면 `Ok(())`
    fn resolve_imports(&mut self, target: &LoadedDll) -> Result<(), ()> {
        // [1] 의존성 확인을 위해 대상 DLL 다시 파싱
        let buffer = fs::read(&target.name).map_err(|e| {
            crate::emu_log!(
                "[!] Failed to read DLL {} for import resolution: {:?}",
                target.name,
                e
            );
        })?;
        let pe = PE::parse(&buffer).map_err(|e| {
            crate::emu_log!(
                "[!] PE parsing failed during import resolution for {}: {:?}",
                target.name,
                e
            );
        })?;

        let image_base = target.base_addr;
        let target_dll_name = target
            .name
            .split('/')
            .next_back()
            .unwrap_or(&target.name)
            .to_string();

        // [2] Import Directory 존재 여부 확인
        if let Some(opt) = pe.header.optional_header
            && let Some(import_dir) = opt.data_directories.get_import_table()
        {
            if import_dir.size == 0 {
                return Ok(());
            }

            let mut desc_addr = image_base + import_dir.virtual_address as u64;

            // [3] Import Descriptor Table 순회 (DLL 단위)
            loop {
                let mut desc_buf = [0u8; 20];
                if self.mem_read(desc_addr, &mut desc_buf).is_err() {
                    break;
                }

                let orig_first_thunk = u32::from_le_bytes(desc_buf[0..4].try_into().unwrap());
                let name_rva = u32::from_le_bytes(desc_buf[12..16].try_into().unwrap());
                let first_thunk = u32::from_le_bytes(desc_buf[16..20].try_into().unwrap());

                // Descriptor Table의 끝 (모두 0) 확인
                if orig_first_thunk == 0 && first_thunk == 0 {
                    break;
                }

                let dll_name = self.read_string(image_base + name_rva as u64);

                let mut ilt_rva = if orig_first_thunk != 0 {
                    orig_first_thunk
                } else {
                    first_thunk
                };
                let mut iat_rva = first_thunk;

                // [4] 각 DLL 내의 임포트 함수 순회
                loop {
                    let val = self.read_u32(image_base + ilt_rva as u64);
                    if val == 0 {
                        break;
                    }

                    // 오디널(Ordinal) 또는 이름(Name)으로 함수 식별
                    let func_name = if (val & 0x80000000) != 0 {
                        format!("Ordinal_{}", val & 0xFFFF)
                    } else {
                        // Import By Name 구조체 (Hint[2] + Name[...])
                        self.read_string(image_base + val as u64 + 2)
                    };

                    let iat_addr = image_base + iat_rva as u64;
                    let mut final_addr = 0;

                    // 1단계: 이미 로드된 다른 DLL 모듈에서 해당 함수(Export) 찾기
                    {
                        let context = self.get_data();
                        let dll_modules = context.dll_modules.lock().unwrap();
                        for (name, dll) in dll_modules.iter() {
                            if name.eq_ignore_ascii_case(&dll_name) {
                                if let Some(real_addr) = dll.exports.get(&func_name) {
                                    final_addr = *real_addr;
                                    break;
                                }
                            }
                        }
                    }

                    // 2단계: 못 찾았다면 프록시 DLL(Rust)에서 특수하게 처리하는 데이터 주소 등이 있는지 확인
                    let mut final_addr = if final_addr == 0 {
                        Win32Context::resolve_proxy_export(self, &dll_name, &func_name)
                            .map(|a| a as u64)
                            .unwrap_or(0)
                    } else {
                        final_addr
                    };

                    // 3단계: 여전히 못 찾았다면 Fake Address (Hooking 용) 할당
                    let context = self.get_data();
                    if final_addr == 0 {
                        final_addr = context.import_address.fetch_add(4, Ordering::SeqCst) as u64;
                    }

                    // 역방향 조회를 위해 주소 맵에 등록 (DLL!Function 형식)
                    context
                        .address_map
                        .lock()
                        .unwrap()
                        .insert(final_addr, format!("{}!{}", dll_name, func_name));

                    // IAT 패치: 실제 주소 또는 Fake 주소를 써넣음
                    self.write_u32(iat_addr, final_addr as u32);

                    ilt_rva += 4;
                    iat_rva += 4;
                }
                desc_addr += 20; // 다음 IMAGE_IMPORT_DESCRIPTOR로 이동
            }
        }

        // [5] 현재 모듈을 로드된 모듈 목록에 추가
        {
            let context = self.get_data();
            context
                .dll_modules
                .lock()
                .unwrap()
                .insert(target_dll_name, target.clone());
        }

        Ok(())
    }

    /// DLL의 엔트리 포인트(DllMain) 함수를 실행합니다.
    ///
    /// # 인자
    /// * `dll`: 엔트리 포인트를 실행할 DLL 객체
    ///
    /// # 반환
    /// * `Result<(), ()>`: 실행 성공 시 `Ok(())` (내부 에러 발생 시 로그 출력 후 진행)
    fn run_dll_entry(&mut self, dll: &LoadedDll) -> Result<(), ()> {
        if dll.entry_point == 0 {
            return Ok(());
        }

        let esp = self.reg_read(RegisterX86::ESP).map_err(|_| ())?;

        // DllMain(hInstance, fdwReason, lpReserved) 호출 준비
        // x86 stdcall: 인자를 역순으로 push
        self.push_u32(0u32); // lpReserved (arg3)
        self.push_u32(1u32); // fdwReason = DLL_PROCESS_ATTACH (arg2)
        self.push_u32(dll.base_addr as u32); // hInstance (arg1)
        self.push_u32(EXIT_ADDRESS as u32); // 리턴 주소

        crate::emu_log!(
            "[*] Calling DllMain for {} at {:#x}...",
            dll.name,
            dll.entry_point
        );

        if let Err(e) = self.emu_start(dll.entry_point, EXIT_ADDRESS as u64, 0, 0) {
            crate::emu_log!("[!] DllMain for {} failed: {:?}", dll.name, e);
        } else {
            crate::emu_log!("[*] DllMain for {} finished successfully.", dll.name);
        }

        // 스택 복구
        let _ = self.reg_write(RegisterX86::ESP, esp);
        Ok(())
    }

    /// 특정 DLL 내의 지정된 함수를 직접 호출합니다. (테스트 및 특정 API 명시적 실행용)
    ///
    /// # 인자
    /// * `dll_name`: 호출할 함수가 포함된 DLL 이름
    /// * `func_name`: 호출할 대상 함수 이름
    /// * `args`: 함수에 전달될 인자들 모음
    fn run_dll_func(&mut self, dll_name: &str, func_name: &str, args: Vec<Box<dyn Any>>) {
        self.prepare_dll_func(dll_name, func_name, args);
        let eip = self.reg_read(RegisterX86::EIP).unwrap_or(0);

        if let Err(e) = self.emu_start(eip, EXIT_ADDRESS as u64, 0, 0) {
            crate::emu_log!(
                "[!] Execution of {}!{} failed: {:?}",
                dll_name,
                func_name,
                e
            );
        }
    }

    fn run_emulator(
        &mut self,
        dll_name: &str,
        func_name: &str,
        args: Vec<Box<dyn Any>>,
        state_tx: Option<Sender<CpuContext>>,
        cmd_rx: Option<Receiver<DebugCommand>>,
    ) {
        self.prepare_dll_func(dll_name, func_name, args);
        let cmd_rx = cmd_rx;
        let mut debug_auto_run = true;
        let mut debug_last_state_sent = Instant::now();

        if let Some(state_tx) = state_tx.as_ref() {
            if state_tx.send(capture_cpu_context(self)).is_ok() {
                debug_last_state_sent = Instant::now();
            }
        }

        loop {
            let eip = self.reg_read(RegisterX86::EIP).unwrap_or(0);
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
                        Ok(DebugCommand::Stop) | Err(_) => {
                            self.emu_stop().unwrap_or_default();
                            break;
                        }
                        Ok(DebugCommand::Pause) => continue,
                    }
                } else {
                    match cmd_rx.try_recv() {
                        Ok(DebugCommand::Pause) => {
                            debug_auto_run = false;
                            if let Some(state_tx) = state_tx.as_ref() {
                                if state_tx.send(capture_cpu_context(self)).is_err() {
                                    self.emu_stop().unwrap_or_default();
                                    break;
                                }
                                debug_last_state_sent = Instant::now();
                            }
                            continue;
                        }
                        Ok(DebugCommand::Stop) => {
                            self.emu_stop().unwrap_or_default();
                            break;
                        }
                        Ok(DebugCommand::Run) | Ok(DebugCommand::Step) => {}
                        Err(std::sync::mpsc::TryRecvError::Empty) => {}
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            self.emu_stop().unwrap_or_default();
                            break;
                        }
                    }
                }
            }

            // 메인 스레드(tid=0) 실행
            let quantum = if cmd_rx.is_some() {
                if debug_auto_run {
                    DEBUG_AUTO_QUANTUM
                } else {
                    DEBUG_STEP_QUANTUM
                }
            } else {
                200_000
            };
            let _ = self.emu_start(eip, EXIT_ADDRESS as u64, 0, quantum);

            // 백그라운드 스레드 스케줄링
            KERNEL32::schedule_threads(self);

            if let Some(state_tx) = state_tx.as_ref() {
                let should_send_state = if debug_auto_run {
                    debug_last_state_sent.elapsed() >= DEBUG_STATE_SEND_INTERVAL
                } else {
                    true
                };
                if should_send_state {
                    if state_tx.send(capture_cpu_context(self)).is_err() {
                        self.emu_stop().unwrap_or_default();
                        break;
                    }
                    debug_last_state_sent = Instant::now();
                }
            }

            // 모든 스레드(메인 포함)가 대기 중인 경우 호스트 측에서 대기하여 CPU 점유율 조절
            let earliest_resume = {
                let context = self.get_data();
                let mut min_time = *context.main_resume_time.lock().unwrap();

                let threads = context.threads.lock().unwrap();
                for t in threads.iter().filter(|t| t.alive) {
                    if let Some(t_res) = t.resume_time {
                        if min_time.is_none() || t_res < min_time.unwrap() {
                            min_time = Some(t_res);
                        }
                    } else {
                        min_time = None;
                        break;
                    }
                }
                min_time
            };

            if let Some(res_time) = earliest_resume {
                let now = Instant::now();
                if res_time > now {
                    let diff = res_time.duration_since(now);
                    std::thread::sleep(diff.min(Duration::from_millis(10)));
                }
            }
        }

        crate::emu_log!("[*] Main emulator loop finished.");
    }

    /// 함수 호출을 위한 스택 및 EIP 환경을 준비 (실제 에뮬레이션은 시작하지 않음)
    fn prepare_dll_func(&mut self, dll_name: &str, func_name: &str, args: Vec<Box<dyn Any>>) {
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
            // 인자 처리 및 역순 push
            let mut push_values: Vec<u32> = Vec::new();
            for arg in args {
                if let Some(v) = arg.downcast_ref::<i32>() {
                    push_values.push(*v as u32);
                } else if let Some(v) = arg.downcast_ref::<u32>() {
                    push_values.push(*v);
                } else if let Some(v) = arg.downcast_ref::<&str>() {
                    let ptr = self.alloc_str(v);
                    push_values.push(ptr);
                }
            }

            for val in push_values.iter().rev() {
                self.push_u32(*val);
            }
            self.push_u32(EXIT_ADDRESS as u32); // 리턴 주소

            crate::emu_log!(
                "[*] Prepared {}!{}(...) at {:#x}",
                dll_name,
                func_name,
                func_address
            );

            let _ = self.reg_write(RegisterX86::EIP, func_address as u64);
        }
    }

    fn read_u32(&self, addr: u64) -> u32 {
        let mut buf = [0u8; 4];
        if self.mem_read(addr, &mut buf).is_ok() {
            u32::from_le_bytes(buf)
        } else {
            0
        }
    }

    fn read_i32(&self, addr: u64) -> i32 {
        self.read_u32(addr) as i32
    }

    fn read_u16(&self, addr: u64) -> u16 {
        let mut buf = [0u8; 2];
        if self.mem_read(addr, &mut buf).is_ok() {
            u16::from_le_bytes(buf)
        } else {
            0
        }
    }

    fn write_u32(&mut self, addr: u64, value: u32) {
        let _ = self.mem_write(addr, &value.to_le_bytes());
    }

    fn write_u16(&mut self, addr: u64, value: u16) {
        let _ = self.mem_write(addr, &value.to_le_bytes());
    }

    fn read_arg(&self, index: usize) -> u32 {
        let esp = self.reg_read(RegisterX86::ESP).unwrap_or(0);
        // [ESP] = Return Address, [ESP+4] = Arg0, ...
        let addr = esp + 4 + (index as u64 * 4);
        self.read_u32(addr)
    }

    fn read_string_bytes(&self, addr: u64, max_len: usize) -> Vec<u8> {
        let mut chars = Vec::new();
        let mut curr = addr;

        while chars.len() < max_len {
            let mut buf = [0u8; 1];
            if self.mem_read(curr, &mut buf).is_err() || buf[0] == 0 {
                break;
            }
            chars.push(buf[0]);
            curr += 1;
        }
        chars
    }

    fn read_string(&self, addr: u64) -> String {
        let bytes = self.read_string_bytes(addr, 1024);
        String::from_utf8_lossy(&bytes).to_string()
    }

    fn write_string(&mut self, addr: u64, text: &str) {
        let bytes = text.as_bytes();
        let _ = self.mem_write(addr, bytes);
        let _ = self.mem_write(addr + bytes.len() as u64, &[0u8]); // Null terminator
    }

    fn write_euc_kr(&mut self, addr: u64, text: &str) {
        let (encoded, _, _) = EUC_KR.encode(text);
        let bytes = encoded.as_ref();
        let _ = self.mem_write(addr, bytes);
        let _ = self.mem_write(addr + bytes.len() as u64, &[0u8]);
    }

    fn read_euc_kr(&self, addr: u64) -> String {
        let bytes = self.read_string_bytes(addr, 2048);
        if bytes.is_empty() {
            return String::new();
        }

        // EUC-KR 디코딩 (필요한 경우에만 수행)
        let filtered: Vec<u8> = bytes.iter().filter(|&&b| b > 127).copied().collect();
        if filtered.is_empty() {
            return String::from_utf8_lossy(&bytes).to_string();
        }

        let mut detector = EncodingDetector::new();
        detector.feed(&filtered, true);
        let encoding = detector.guess(None, true);

        if encoding.name().contains("UTF") {
            String::from_utf8_lossy(&bytes).to_string()
        } else {
            let (res, _, _) = EUC_KR.decode(&bytes);
            res.to_string()
        }
    }

    fn push_u32(&mut self, value: u32) {
        let esp = self.reg_read(RegisterX86::ESP).unwrap_or(0);
        let new_esp = esp - 4;
        self.write_u32(new_esp, value);
        let _ = self.reg_write(RegisterX86::ESP, new_esp);
    }

    fn pop_u32(&mut self) -> u32 {
        let esp = self.reg_read(RegisterX86::ESP).unwrap_or(0);
        let val = self.read_u32(esp);
        let _ = self.reg_write(RegisterX86::ESP, esp + 4);
        val
    }

    fn apply_stack_cleanup(&mut self, cleanup: StackCleanup) {
        let esp = self.reg_read(RegisterX86::ESP).unwrap_or(0);
        if let Some(new_esp) = stack_cleanup_target_esp(esp, cleanup) {
            // [현재 ESP]에 있는 리턴 주소를 [new_esp] 위치로 옮김
            let ret_addr = self.read_u32(esp);
            self.write_u32(new_esp, ret_addr);
            let _ = self.reg_write(RegisterX86::ESP, new_esp);
        }
    }

    fn malloc(&mut self, size: usize) -> u64 {
        let data = self.get_data();
        // 4바이트 정렬
        let aligned_size = (size as u32 + 3) & !3;
        let addr = data.heap_cursor.fetch_add(aligned_size, Ordering::SeqCst);

        if (addr as u64 + aligned_size as u64) > (HEAP_BASE + HEAP_SIZE) {
            crate::emu_log!("[!] HEAP OVERFLOW at {:#x}", addr);
        }
        addr as u64
    }

    fn alloc_str(&mut self, text: &str) -> u32 {
        let bytes = text.as_bytes();
        let addr = self.malloc(bytes.len() + 1);
        self.write_string(addr, text);
        addr as u32
    }

    fn alloc_bytes(&mut self, data: &[u8]) -> u32 {
        let addr = self.malloc(data.len());
        let _ = self.mem_write(addr, data);
        addr as u32
    }

    fn write_mem(&mut self, addr: u64, data: &[i32]) {
        for (i, &val) in data.iter().enumerate() {
            self.write_u32(addr + (i * 4) as u64, val as u32);
        }
    }

    fn read_u8(&self, addr: u64) -> u8 {
        let mut buf = [0u8; 1];
        self.mem_read(addr, &mut buf).unwrap();
        buf[0]
    }

    fn write_u8(&mut self, addr: u64, value: u8) {
        self.mem_write(addr, &[value]).unwrap();
    }

    fn resolve_address(&self, addr: u32) -> String {
        let ctx = self.get_data();
        let dll_modules = ctx.dll_modules.lock().unwrap();
        for dll in dll_modules.values() {
            let base = dll.base_addr as u32;
            let end = base.wrapping_add(dll.size as u32);
            if addr >= base && addr < end {
                let offset = addr - base;
                // 가장 가까운 export 심볼 찾기
                let mut nearest_name: Option<&str> = None;
                let mut nearest_dist: u32 = u32::MAX;
                for (name, &exp_addr) in &dll.exports {
                    let exp = exp_addr as u32;
                    if exp <= addr {
                        let dist = addr - exp;
                        if dist < nearest_dist {
                            nearest_dist = dist;
                            nearest_name = Some(name);
                        }
                    }
                }
                let dll_short = dll.name.rsplit('/').next().unwrap_or(&dll.name);
                return if let Some(sym) = nearest_name {
                    if nearest_dist == 0 {
                        format!("{}!{}", dll_short, sym)
                    } else {
                        format!("{}+{:#x} ({}+{:#x})", dll_short, offset, sym, nearest_dist)
                    }
                } else {
                    format!("{}+{:#x}", dll_short, offset)
                };
            }
        }
        format!("{:#x}", addr)
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
