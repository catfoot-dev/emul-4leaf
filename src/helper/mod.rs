pub mod dll_loader;
pub mod emulator;
pub mod memory;
pub mod string;

pub(crate) use emulator::run_nested_guest_until_exit;
pub use memory::*;

use crate::{
    debug::common::{CpuContext, DebugCommand},
    dll::win32::{LoadedDll, StackCleanup, Win32Context},
};
use std::{
    any::Any,
    sync::mpsc::{Receiver, Sender},
};
use unicorn_engine::Unicorn;

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

/// Unicorn 객체에 추가할 메소드 목록 정의
///
/// Unicorn 엔진을 확장하여 Win32 에뮬레이션에 필요한 메모리 조작, 스택 제어, DLL 로딩 등을 지원하는 헬퍼 트레잇
#[allow(dead_code)]
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
    fn setup(
        &mut self,
        state_tx: Option<Sender<CpuContext>>,
        cmd_rx: Option<Receiver<DebugCommand>>,
    ) -> Result<(), ()> {
        emulator::setup_impl(self, state_tx, cmd_rx)
    }

    fn load_dll_with_reloc(&mut self, filename: &str, target_base: u64) -> Result<LoadedDll, ()> {
        dll_loader::load_dll_with_reloc_impl(self, filename, target_base)
    }

    fn resolve_imports(&mut self, target: &LoadedDll) -> Result<(), ()> {
        dll_loader::resolve_imports_impl(self, target)
    }

    fn run_dll_entry(&mut self, dll: &LoadedDll) -> Result<(), ()> {
        dll_loader::run_dll_entry_impl(self, dll)
    }

    fn run_dll_func(&mut self, dll_name: &str, func_name: &str, args: Vec<Box<dyn Any>>) {
        emulator::run_dll_func_impl(self, dll_name, func_name, args)
    }

    fn run_emulator(
        &mut self,
        dll_name: &str,
        func_name: &str,
        args: Vec<Box<dyn Any>>,
        state_tx: Option<Sender<CpuContext>>,
        cmd_rx: Option<Receiver<DebugCommand>>,
    ) {
        emulator::run_emulator_impl(self, dll_name, func_name, args, state_tx, cmd_rx)
    }

    fn prepare_dll_func(&mut self, dll_name: &str, func_name: &str, args: Vec<Box<dyn Any>>) {
        emulator::prepare_dll_func_impl(self, dll_name, func_name, args)
    }

    fn read_u32(&self, addr: u64) -> u32 {
        memory::read_u32_impl(self, addr)
    }

    fn read_i32(&self, addr: u64) -> i32 {
        memory::read_i32_impl(self, addr)
    }

    fn read_u16(&self, addr: u64) -> u16 {
        memory::read_u16_impl(self, addr)
    }

    fn write_u32(&mut self, addr: u64, value: u32) {
        memory::write_u32_impl(self, addr, value)
    }

    fn write_u16(&mut self, addr: u64, value: u16) {
        memory::write_u16_impl(self, addr, value)
    }

    fn read_arg(&self, index: usize) -> u32 {
        memory::read_arg_impl(self, index)
    }

    fn read_string_bytes(&self, addr: u64, max_len: usize) -> Vec<u8> {
        memory::read_string_bytes_impl(self, addr, max_len)
    }

    fn read_string(&self, addr: u64) -> String {
        memory::read_string_impl(self, addr)
    }

    fn write_string(&mut self, addr: u64, text: &str) {
        memory::write_string_impl(self, addr, text)
    }

    fn write_euc_kr(&mut self, addr: u64, text: &str) {
        string::write_euc_kr_impl(self, addr, text)
    }

    fn read_euc_kr(&self, addr: u64) -> String {
        string::read_euc_kr_impl(self, addr)
    }

    fn push_u32(&mut self, value: u32) {
        memory::push_u32_impl(self, value)
    }

    fn pop_u32(&mut self) -> u32 {
        memory::pop_u32_impl(self)
    }

    fn apply_stack_cleanup(&mut self, cleanup: StackCleanup) {
        memory::apply_stack_cleanup_impl(self, cleanup)
    }

    fn malloc(&mut self, size: usize) -> u64 {
        memory::malloc_impl(self, size)
    }

    fn alloc_str(&mut self, text: &str) -> u32 {
        memory::alloc_str_impl(self, text)
    }

    fn alloc_bytes(&mut self, data: &[u8]) -> u32 {
        memory::alloc_bytes_impl(self, data)
    }

    fn write_mem(&mut self, addr: u64, data: &[i32]) {
        memory::write_mem_impl(self, addr, data)
    }

    fn read_u8(&self, addr: u64) -> u8 {
        memory::read_u8_impl(self, addr)
    }

    fn write_u8(&mut self, addr: u64, value: u8) {
        memory::write_u8_impl(self, addr, value)
    }

    fn resolve_address(&self, addr: u32) -> String {
        memory::resolve_address_impl(self, addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dll::win32::StackCleanup;

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
