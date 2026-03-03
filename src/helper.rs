use std::{any::Any, collections::HashMap, fs, sync::mpsc::{Receiver, Sender}};
use goblin::pe::PE;
use unicorn_engine::{HookType, Prot, RegisterX86, Unicorn};
use crate::{debug::common::{CpuContext, DebugCommand}, win32::{LoadedDll, Win32Context}};

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

// =========================================================
// [Debug Helper] 메모리 덤프 함수 (Hex Dump 스타일)
// =========================================================
fn print_hexdump(uc: &Unicorn<Win32Context>, address: u64, size: usize) {
    let mut buffer = vec![0u8; size];
    
    // 메모리 읽기 시도
    if let Err(_) = uc.mem_read(address, &mut buffer) {
        println!("[DEBUG] Failed to read memory at 0x{:x} (Size: {})", address, size);
        return;
    }

    println!("[DEBUG] Memory Dump at 0x{:x} ({} bytes):", address, size);
    
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
        println!();
    }
    println!("------------------------------------------------------------");
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
        println!("\n[DEBUG] Checking: {}", $label);
        print_hexdump($uc, $addr as u64, $size as usize);
    };
}

// =========================================================
// [Debug Macro] 스택 뷰어 매크로 (현재 ESP 기준)
// 사용법: dump_stack!(unicorn, 5); // 스택 상위 5개 값 출력
// =========================================================
macro_rules! dump_stack {
    ($uc:expr, $count:expr) => {
        println!("\n[DEBUG] Stack Trace (Top {} items):", $count);
        if let Ok(esp) = $uc.reg_read(RegisterX86::ESP) {
            let mut buf = [0u8; 4];
            for i in 0..$count {
                let addr = esp + (i * 4) as u64;
                if $uc.mem_read(addr, &mut buf).is_ok() {
                    let val = u32::from_le_bytes(buf);
                    // ESP 위치 표시 화살표
                    let marker = if i == 0 { "<- ESP" } else { "" };
                    println!("  0x{:08x}: 0x{:08x} ({}) {}", addr, val, val, marker);
                } else {
                    println!("  0x{:08x}: [UNMAPPED]", addr);
                }
            }
        } else {
            println!("  [Error] Failed to read ESP register");
        }
        println!("------------------------------------------------------------");
    };
}

// =========================================================
// [Debug Macro] 레지스터 뷰어 매크로
// 사용법: dump_regs!(unicorn);
// =========================================================
macro_rules! dump_regs {
    ($uc:expr) => {
        println!("\n[DEBUG] Registers:");
        let eax = $uc.reg_read(RegisterX86::EAX).unwrap_or(0);
        let ebx = $uc.reg_read(RegisterX86::EBX).unwrap_or(0);
        let ecx = $uc.reg_read(RegisterX86::ECX).unwrap_or(0);
        let edx = $uc.reg_read(RegisterX86::EDX).unwrap_or(0);
        let esi = $uc.reg_read(RegisterX86::ESI).unwrap_or(0);
        let edi = $uc.reg_read(RegisterX86::EDI).unwrap_or(0);
        let esp = $uc.reg_read(RegisterX86::ESP).unwrap_or(0);
        let ebp = $uc.reg_read(RegisterX86::EBP).unwrap_or(0);
        let eip = $uc.reg_read(RegisterX86::EIP).unwrap_or(0);
        
        println!("  EAX: 0x{:08x}  EBX: 0x{:08x}  ECX: 0x{:08x}  EDX: 0x{:08x}", eax, ebx, ecx, edx);
        println!("  ESI: 0x{:08x}  EDI: 0x{:08x}  ESP: 0x{:08x}  EBP: 0x{:08x}", esi, edi, esp, ebp);
        println!("  EIP: 0x{:08x}", eip);
        println!("------------------------------------------------------------");
    };
}


// Unicorn 객체에 추가할 메소드 목록 정의
pub trait UnicornHelper {
    fn setup(&mut self, state_tx: Sender<CpuContext>, cmd_rx: Receiver<DebugCommand>) -> Result<(), ()>;

    fn load_dll_with_reloc(&mut self, filename: &str, target_base: u64) -> Result<LoadedDll, ()>;

    fn resolve_imports(&mut self, target: &LoadedDll) -> Result<(), ()>;

    fn run_dll_main(&mut self, dll: &LoadedDll) -> Result<(), ()>;
    fn run_dll_func(&mut self, dll_name: &str, func_name: &str, args: Vec<Box<dyn Any>>);

    // === 메모리 읽기/쓰기 (Heap/General) ===
    fn read_u32(&self, addr: u64) -> u32;
    fn write_u32(&mut self, addr: u64, value: u32);

    fn read_arg(&self, index: usize) -> u32;
    
    // 문자열 읽기 (C-String: NULL 만날 때까지)
    fn read_string(&self, addr: u64) -> String;

    // === 스택 조작 (Stack) ===
    fn push_u32(&mut self, value: u32);
    fn pop_u32(&mut self) -> u32;

    // 간단한 메모리 할당 (malloc)
    fn malloc(&mut self, size: usize) -> u64;

    // 문자열을 힙에 쓰고, 그 주소를 반환 (C-String: 끝에 NULL 추가)
    fn alloc_str(&mut self, text: &str) -> u32; // 32비트 주소 반환

    // 바이트 배열(구조체 등)을 힙에 쓰고, 그 주소를 반환
    fn alloc_bytes(&mut self, data: &[u8]) -> u32;
}

// 모든 Unicorn<D> 타입에 대해 구현 (D는 Win32Context 등 무엇이든 가능)
impl UnicornHelper for Unicorn<'_, Win32Context> {
    fn setup(&mut self, state_tx: Sender<CpuContext>, cmd_rx: Receiver<DebugCommand>) -> Result<(), ()> {
        self.mem_map(STACK_BASE, STACK_SIZE, Prot::ALL).unwrap();
        // 스택 오버플로우/경계 읽기 에러 방지 (스택 바로 뒤 4KB 추가 할당)
        self.mem_map(STACK_TOP, SIZE_4KB, Prot::ALL).unwrap();

        self.mem_map(HEAP_BASE, HEAP_SIZE, Prot::ALL).unwrap();
        self.mem_map(SHARED_MEM_BASE, SIZE_4KB, Prot::ALL).unwrap();

        // println!("[*] Mapping Low Memory (0x0 ~ 0x20000) to bypass NULL pointer access");
        // NULL 포인터 접근 방지 (0 ~ 128KB)
        self.mem_map(0, 0x2_0000, Prot::ALL).unwrap();

        // TEB
        self.mem_map(TEB_BASE, SIZE_4KB, Prot::ALL).unwrap();
        self.mem_write(TEB_BASE + 0x18, &(TEB_BASE as u32).to_le_bytes()).unwrap();
        self.reg_write(RegisterX86::FS, TEB_BASE).unwrap();

        // Fake Import Area
        self.mem_map(FAKE_IMPORT_BASE, 1024 * 1024, Prot::ALL | Prot::EXEC).unwrap();

        self.reg_write(RegisterX86::ESP, STACK_TOP).unwrap();
        
        // let modules = loaded_modules.clone();
        
        // self.add_code_hook(
        //     0,
        //     0x10 as u64,
        //     |uc, addr, _|
        // {
        //     println!("[!] Run code at {addr:#x}");
        //     // dump_mem!(uc, addr & 0xFFFF_FFF0, 16);
        // }).expect("Failed to install code hook(Dll Code)");

        // self.add_code_hook(
        //     // 0x3000_0000,
        //     // 0x4FFF_FFFF,
        //     0x20000,
        //     -1i64 as u64,
        //     |uc, addr, _|
        // {
        //     println!("[!] Run code at {addr:#x}");
        //     // dump_mem!(uc, addr & 0xFFFF_FFF0, 16);
        // }).expect("Failed to install code hook(Dll Code)");

        // API Call Hook (Fake Address Range)
        self.add_code_hook(
            FAKE_IMPORT_BASE,
            FAKE_IMPORT_BASE + 1024 * 1024,
            |uc: &mut Unicorn<Win32Context>, addr, size|
        {
            println!("[!] Fake API Call at {addr:#x} - ({size} bytes)");

            let context = uc.get_data_mut();
            let address_map = context.address_map.clone();
            if let Some(import_func) = address_map.get(&addr) {
                println!("    Real function -> {import_func}");

                let splits: Vec<&str> = import_func.split('!').collect();
                let dll_name = splits[0];
                let func_name = splits[1];

                // 삽입한 dll에 있는지 찾아서 실행
                if let Some(dll) = context.dll_modules.clone().borrow().get(dll_name) {
                    if let Some(func_address) = dll.exports.get(func_name) {
                        println!("[*] Function address: {func_address:#x}");
                        println!("[*] Calling {dll_name}!{func_name}(...args)...");

                        if let Err(e) = uc.emu_start(*func_address, EXIT_ADDRESS, 0, 0) {
                            println!("\n[!] Emulation stopped/failed: {e:?}");
                            let pc = uc.reg_read(RegisterX86::EIP).unwrap();
                            println!("    Stopped at EIP: {pc:#x}\n");
                        } else {
                            println!("[*] {dll_name}!{func_name} finished successfully.");
                        }

                        return;
                    }
                }

                // 따로 정의한 함수가 있는지 찾아서 실행
                if let Some((size, result)) = Win32Context::handle(uc, dll_name, func_name) {
                    if let Some(eax) = result {
                        uc.reg_write(RegisterX86::EAX, eax as u64).unwrap(); // Success
                    }

                    // let eip = uc.reg_read(RegisterX86::EIP).unwrap();
                    // let ret_addr = uc.read_u32(eip) as u64;
                    // println!("    Return address: {ret_addr:#010x}");
                    
                    // EIP 복귀 및 스택 정리 (Pop Return Addr + Pop Args)
                    let esp = uc.reg_read(RegisterX86::ESP).unwrap();
                    let esp = esp + 4 + (size as u64 * 4);
                    println!("    Return address: {esp:#010x}");
                    uc.reg_write(RegisterX86::ESP, esp).unwrap();
                    // uc.reg_write(RegisterX86::EIP, ret_addr).unwrap();
                
                    return;
                }

                // 매핑된 주소 값이나 dll은 있지만 매칭되는 함수가 없음
                println!("[!] Can not found function. {import_func}");
            } else {
                // 매핑된 주소 값이 없음
                println!("[!] Can not found addr. {addr:#x}");
            }

            uc.reg_write(RegisterX86::EAX, 1).unwrap(); // Success
            
            let esp = uc.reg_read(RegisterX86::ESP).unwrap();
            let mut buf = [0u8; 4];
            if uc.mem_read(esp, &mut buf).is_ok() {
                let ret = u32::from_le_bytes(buf) as u64;
                uc.reg_write(RegisterX86::ESP, esp + 4).unwrap();
                uc.reg_write(RegisterX86::EIP, ret).unwrap();
            } else {
                uc.emu_stop().unwrap();
            }
        }).expect("Failed to install code hook(Fake Address)");

        // self.add_code_hook(0, 0x2_000, |uc, _, _|
        // {
        //     println!("\n[!] Detected execution at 0x00. Assuming successful return from function.");
        //     println!("    (Cause: Stack pointer drift due to stdcall mismatch)");

        //     dump_stack!(uc, 4);
        //     dump_regs!(uc);

        //     uc.emu_stop().unwrap();
        // }).expect("Failed to install code hook(Address 0x00)");

        self.add_code_hook(0, -1i64 as u64, move |uc, addr, size| {
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
            let instr_str = code.iter()
                .map(|b| format!("{:02x}", b))
                .collect::<Vec<_>>()
                .join(" ");

            // 4. 상태 전송
            if state_tx.send(CpuContext { regs, stack, next_instr: instr_str }).is_err() {
                uc.emu_stop().unwrap();
                return;
            }

            if regs[8] == 0 {
                println!("\n[!] Detected execution at 0x00. Assuming successful return from function.");
                println!("    (Cause: Stack pointer drift due to stdcall mismatch)");
                uc.emu_stop().unwrap();
                return;
            }

            // 5. 명령 대기 (Step)
            match cmd_rx.recv() {
                Ok(DebugCommand::Step) => {}, // 진행
                _ => { uc.emu_stop().unwrap(); } // Stop or Error
            }
        }).expect("Failed to install code hook.");

        // Error Debug Hook
        self.add_mem_hook(
            HookType::MEM_READ_UNMAPPED | HookType::MEM_WRITE_UNMAPPED | HookType::MEM_FETCH_UNMAPPED,
            0,
            -1i64 as u64,
            |uc, access, addr, size, value|
        {
            if value == 0 {
                println!("\n[!] Detected execution at 0x00. Assuming successful return from function.");
                println!("    (Cause: Stack pointer drift due to stdcall mismatch)");
                println!("    Address: {addr:#010x}");

                return false;
            }

            println!("\n[!!!!!!] MEMORY ERROR DETECTED: {:?} at {:#x} (Size: {})", access, addr, size);

            match access {
                unicorn_engine::unicorn_const::MemType::READ_UNMAPPED => print!("    Type: READ_UNMAPPED"),
                unicorn_engine::unicorn_const::MemType::WRITE_UNMAPPED => print!("    Type: WRITE_UNMAPPED"),
                unicorn_engine::unicorn_const::MemType::FETCH_UNMAPPED => print!("    Type: FETCH_UNMAPPED"),
                _ => print!("    Type: Unknown"),
            }

            println!("    Trying to address: {:#010x}", value); // 시도한 주소 값

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
        let image_size = pe.header.optional_header.unwrap().windows_fields.size_of_image as u64;
        let _4095 = SIZE_4KB - 1;
        let aligned_size = (image_size + _4095) & !_4095;

        self.mem_map(target_base, aligned_size, Prot::ALL).expect("메모리 매핑 실패");
        println!("Load: {} at {:#x} (Size: {:#x})", filename, target_base, image_size);

        // 3. 섹션 복사
        for section in pe.sections {
            let start = target_base + section.virtual_address as u64;
            let data_start = section.pointer_to_raw_data as usize;
            let data_size = section.size_of_raw_data as usize;
            if data_size == 0 { continue; }

            let data = &buffer[data_start..data_start + data_size];
            self.mem_write(start, data).expect("섹션 데이터 쓰기 실패");
        }

        // 4. IAT 패치
        let original_base = pe.image_base as u64;

        if original_base != target_base {
            println!("    Relocating from 0x{:x} to 0x{:x}...", original_base, target_base);
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
                        self.mem_read(target_base + reloc_rva as u64, &mut block_header).unwrap();
                        
                        let page_rva = u32::from_le_bytes(block_header[0..4].try_into().unwrap());
                        let block_size = u32::from_le_bytes(block_header[4..8].try_into().unwrap());

                        if block_size == 0 { break; } // Safety break

                        let entries_count = (block_size as usize - 8) / 2;
                        let mut entries_buf = vec![0u8; (block_size - 8) as usize];
                        self.mem_read(target_base + reloc_rva as u64 + 8, &mut entries_buf).unwrap();

                        for i in 0..entries_count {
                            let entry = u16::from_le_bytes(entries_buf[i*2..(i+1)*2].try_into().unwrap());
                            let reloc_type = entry >> 12;      // 상위 4비트
                            let offset = entry & 0x0FFF;       // 하위 12비트

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
                // println!("    Export: {} -> 0x{:x}", name, addr);
            }
        }

        let entry_point = target_base + pe.header.optional_header.unwrap().standard_fields.address_of_entry_point as u64;

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
                    println!("[DEBUG] Import Directory size is 0!");
                    return Ok(());
                }

                let mut desc_addr = image_base + import_dir.virtual_address as u64;
                println!("[DEBUG] Import Descriptor Table at {desc_addr:#x}"); // 로그 추가
                
                loop {
                    let mut desc_buf = [0u8; 20];
                    if self.mem_read(desc_addr, &mut desc_buf).is_err() { break; }
                    
                    let orig_first_thunk = u32::from_le_bytes(desc_buf[0..4].try_into().unwrap());
                    let name_rva = u32::from_le_bytes(desc_buf[12..16].try_into().unwrap());
                    let first_thunk = u32::from_le_bytes(desc_buf[16..20].try_into().unwrap());

                    if orig_first_thunk == 0 && first_thunk == 0 { break; }

                    let dll_name = self.read_string(image_base + name_rva as u64);
                    println!("[DEBUG] Processing Import DLL: {dll_name}"); // 로그 추가
                    
                    // 의존성 DLL인지 확인 (Case-insensitive)
                    // let dependency = dependencies.iter().find(|(name, _)| name.eq_ignore_ascii_case(&dll_name)).map(|(_, dll)| dll);

                    let mut ilt_rva = if orig_first_thunk != 0 { orig_first_thunk } else { first_thunk };
                    let mut iat_rva = first_thunk;

                    loop {
                        // let mut val_buf = [0u8; 4];
                        // self.mem_read(image_base + ilt_rva as u64, &mut val_buf).unwrap();
                        // let val = u32::from_le_bytes(val_buf);
                        let val = self.read_u32(image_base + ilt_rva as u64);
                        // println!("rva: {:#x}, address: {:#x}", ilt_rva, image_base + ilt_rva as u64);
                        if val == 0 { break; }

                        let func_name = if (val & 0x80000000) != 0 {
                            format!("Ordinal_{}", val & 0xFFFF)
                        } else {
                            self.read_string(image_base + val as u64 + 2)
                        };

                        let iat_addr = image_base + iat_rva as u64;
                        let mut final_addr = 0;

                        // 1. 의존성 DLL에 있는 함수인가?
                        let context = self.get_data_mut();
                        context.dll_modules.borrow_mut()
                            .iter()
                            .find(|(name, dll)| {
                                if name.eq_ignore_ascii_case(&dll_name) == false { return false; }
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
                        //         // println!("    Linked: {}!{} -> 0x{:x}", dll_name, func_name, final_addr);
                        //     }
                        // }

                        // 2. 없다면 Fake Address 할당 (전역 카운터 사용)
                        if final_addr == 0 {
                            final_addr = context.import_address;
                            context.import_address += 4; // 다음 함수를 위해 4바이트 증가
                            // println!("{dll_name} - {func_name}: {final_addr:#010x}");
                        }
                        // let context = self.get_data_mut();
                        // final_addr = context.import_address;
                        // context.import_address += 4; // 다음 함수를 위해 4바이트 증가

                        context.address_map.insert(final_addr, format!("{dll_name}!{func_name}"));

                        // [디버그] 패치하는 주소와 값 출력
                        println!("[DEBUG] Patching IAT at 0x{:x} -> 0x{:x} ({})", iat_addr, final_addr, func_name);
                        
                        self.write_u32(iat_addr, final_addr as u32);
                        // self.mem_write(iat_addr, &(final_addr as u32).to_le_bytes()).unwrap();

                        ilt_rva += 4;
                        iat_rva += 4;
                    }
                    desc_addr += 20;
                }
            }
        }

        let mut dll_modules = {
            let context = self.get_data_mut();
            context.dll_modules.borrow_mut()
        };
        dll_modules.insert(dll_name.clone(), target.clone());

        Ok(())
    }

    fn run_dll_main(&mut self, dll: &LoadedDll) -> Result<(), ()> {
        if dll.entry_point == 0 { return Ok(()); }

        let esp = self.reg_read(RegisterX86::ESP as i32).unwrap();

        self.push_u32(0u32);
        self.push_u32(1u32);
        self.push_u32(dll.base_addr as u32);
        self.push_u32(EXIT_ADDRESS as u32);

        println!("[*] Function address: 0x{:x}", dll.entry_point);
        println!("[*] Calling DllMain(0x{:x}, 1, 0)...", dll.entry_point);

        // DllMain 오류나도 무시하고 진행
        // self.emu_start(dll.entry_point, EXIT_ADDRESS, 0, 0).unwrap_err();

        if let Err(e) = self.emu_start(dll.entry_point, EXIT_ADDRESS, 0, 0) {
            println!("\n[!] Emulation stopped/failed: {e:?}");
            let pc = self.reg_read(RegisterX86::EIP).unwrap();
            println!("    Stopped at EIP: {pc:#x}\n");
        } else {
            println!("[*] {} finished successfully.", dll.name);
        }

        // Stack 복구 (간이)
        self.reg_write(RegisterX86::ESP, esp).unwrap();

        Ok(())
    }

    fn run_dll_func(
        &mut self,
        dll_name: &str,
        func_name: &str,
        args: Vec<Box<dyn Any>>,
    ) {
        println!("\n[*] Looking for '{func_name}' in {dll_name}...");
        let context = self.get_data_mut();
        if let Some(module) = context.dll_modules.clone().borrow().get(dll_name) {
            if let Some(func_address) = module.exports.get(func_name) {
                let esp = self.reg_read(RegisterX86::ESP as i32).unwrap();

                let mut arguments = String::new();
                for arg in args.iter() {
                    if arguments.len() > 0 { arguments += ", "; }
                    if let Some(v) = arg.downcast_ref::<i32>() {
                        self.push_u32(*v as u32);
                        arguments += format!("{v}").as_str();
                    } else if let Some(v) = arg.downcast_ref::<u32>() {
                        self.push_u32(*v as u32);
                        arguments += format!("{v}").as_str();
                    } else if let Some(v) = arg.downcast_ref::<&str>() {
                        let str_ptr = self.alloc_str(*v);
                        self.push_u32(str_ptr);
                        arguments += format!("\"{v}\"").as_str();
                    }
                }
                self.push_u32(EXIT_ADDRESS as u32);
                
                println!("[*] Function address: {func_address:#x}");
                println!("[*] Calling {func_name}({arguments})...");

                if let Err(e) = self.emu_start(*func_address, EXIT_ADDRESS, 0, 0) {
                    println!("\n[!] Emulation stopped/failed: {e:?}");
                    let pc = self.reg_read(RegisterX86::EIP).unwrap();
                    println!("    Stopped at EIP: {pc:#x}\n");
                } else {
                    println!("[*] {func_name} finished successfully.");
                }

                // Stack 복구 (간이)
                self.reg_write(RegisterX86::ESP, esp).unwrap();
            }
        }
    }

    fn read_u32(&self, addr: u64) -> u32 {
        let data = self.mem_read_as_vec(addr, 4).expect("메모리 읽기 실패");
        u32::from_le_bytes(data.try_into().unwrap())
    }

    fn write_u32(&mut self, addr: u64, value: u32) {
        self.mem_write(addr, &value.to_le_bytes()).expect("메모리 쓰기 실패");
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
            if byte == 0 { break; } // NULL 문자 만나면 종료
            chars.push(byte);
            curr += 1;
            
            // 안전장치: 너무 길면 끊기 (예: 1KB)
            if chars.len() > 1024 { break; }
        }
        String::from_utf8_lossy(&chars).to_string()
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

    fn malloc(&mut self, size: usize) -> u64 {
        let data = self.get_data_mut();
        let addr = data.heap_cursor;
        
        // 4바이트 정렬 (속도와 안정성을 위해)
        // (size + 3) & !3 은 size를 4의 배수로 올림 처리하는 비트 연산입니다.
        let aligned_size = (size as u64 + 3) & !3;
        
        data.heap_cursor += aligned_size;
        
        // 주의: 실제로는 여기서 addr가 mem_map 된 범위를 넘는지 체크하면 더 좋습니다.
        addr
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
