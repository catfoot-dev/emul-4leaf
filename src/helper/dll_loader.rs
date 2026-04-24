use crate::dll::win32::{LoadedDll, Win32Context};
use goblin::pe::PE;
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    sync::atomic::Ordering,
};
use unicorn_engine::{Prot, RegisterX86, Unicorn};

use super::UnicornHelper;
use super::memory::{EXIT_ADDRESS, SIZE_4KB};

const IMAGE_SCN_MEM_EXECUTE: u32 = 0x2000_0000;
const IMAGE_SCN_MEM_READ: u32 = 0x4000_0000;
const IMAGE_SCN_MEM_WRITE: u32 = 0x8000_0000;

fn section_prot(characteristics: u32) -> Prot {
    let mut prot = Prot::NONE;
    if (characteristics & IMAGE_SCN_MEM_READ) != 0 {
        prot |= Prot::READ;
    }
    if (characteristics & IMAGE_SCN_MEM_WRITE) != 0 {
        prot |= Prot::WRITE;
    }
    if (characteristics & IMAGE_SCN_MEM_EXECUTE) != 0 {
        prot |= Prot::EXEC;
    }
    if prot == Prot::NONE { Prot::READ } else { prot }
}

fn finalize_image_protection(
    uc: &mut Unicorn<Win32Context>,
    image_base: u64,
    pe: &PE<'_>,
) -> Result<(), ()> {
    let mut page_perms: BTreeMap<u64, Prot> = BTreeMap::new();
    for section in &pe.sections {
        let section_size = u64::from(section.virtual_size.max(section.size_of_raw_data));
        if section_size == 0 {
            continue;
        }

        let start = image_base + u64::from(section.virtual_address);
        let end = start + section_size;
        let aligned_start = start & !(SIZE_4KB - 1);
        let aligned_end = (end + (SIZE_4KB - 1)) & !(SIZE_4KB - 1);
        let prot = section_prot(section.characteristics);

        let mut page = aligned_start;
        while page < aligned_end {
            page_perms
                .entry(page)
                .and_modify(|existing| *existing |= prot)
                .or_insert(prot);
            page += SIZE_4KB;
        }
    }

    for (page, prot) in page_perms {
        uc.mem_protect(page, SIZE_4KB, prot).map_err(|e| {
            crate::emu_log!(
                "[!] Failed to protect image page {:#x} with {:?}: {:?}",
                page,
                prot,
                e
            );
        })?;
    }
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
pub(crate) fn load_dll_with_reloc_impl(
    uc: &mut Unicorn<Win32Context>,
    filename: &str,
    target_base: u64,
) -> Result<LoadedDll, ()> {
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

    uc.mem_map(target_base, aligned_size, Prot::READ | Prot::WRITE)
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
        uc.mem_write(start, data).map_err(|e| {
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
                if uc
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
                if uc
                    .mem_read(target_base + reloc_rva as u64 + 8, &mut entries_buf)
                    .is_ok()
                {
                    for i in 0..entries_count {
                        let entry =
                            u16::from_le_bytes(entries_buf[i * 2..(i + 1) * 2].try_into().unwrap());
                        let reloc_type = entry >> 12; // 상위 4비트: 재배치 타입
                        let offset = entry & 0x0FFF; // 하위 12비트: 페이지 내 오프셋

                        // IMAGE_REL_BASED_HIGHLOW (3) 타입만 처리 (Win32 x86 핵심)
                        if reloc_type == 3 {
                            let target_addr = target_base + page_rva as u64 + offset as u64;
                            let mut val_buf = [0u8; 4];
                            if uc.mem_read(target_addr, &mut val_buf).is_ok() {
                                let original_val = u32::from_le_bytes(val_buf);
                                let new_val = original_val.wrapping_add(delta as u32);
                                let _ = uc.mem_write(target_addr, &new_val.to_le_bytes());
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
pub(crate) fn resolve_imports_impl(
    uc: &mut Unicorn<Win32Context>,
    target: &LoadedDll,
) -> Result<(), ()> {
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
            if uc.mem_read(desc_addr, &mut desc_buf).is_err() {
                break;
            }

            let orig_first_thunk = u32::from_le_bytes(desc_buf[0..4].try_into().unwrap());
            let name_rva = u32::from_le_bytes(desc_buf[12..16].try_into().unwrap());
            let first_thunk = u32::from_le_bytes(desc_buf[16..20].try_into().unwrap());

            // Descriptor Table의 끝 (모두 0) 확인
            if orig_first_thunk == 0 && first_thunk == 0 {
                break;
            }

            let dll_name = uc.read_string(image_base + name_rva as u64);

            let mut ilt_rva = if orig_first_thunk != 0 {
                orig_first_thunk
            } else {
                first_thunk
            };
            let mut iat_rva = first_thunk;

            // [4] 각 DLL 내의 임포트 함수 순회
            loop {
                let val = uc.read_u32(image_base + ilt_rva as u64);
                if val == 0 {
                    break;
                }

                // 오디널(Ordinal) 또는 이름(Name)으로 함수 식별
                let func_name = if (val & 0x80000000) != 0 {
                    format!("Ordinal_{}", val & 0xFFFF)
                } else {
                    // Import By Name 구조체 (Hint[2] + Name[...])
                    uc.read_string(image_base + val as u64 + 2)
                };

                let iat_addr = image_base + iat_rva as u64;
                let mut final_addr = 0;

                // 1단계: 이미 로드된 다른 DLL 모듈에서 해당 함수(Export) 찾기
                {
                    let context = uc.get_data();
                    let dll_modules = context.dll_modules.lock().unwrap();
                    for (name, dll) in dll_modules.iter() {
                        if name.eq_ignore_ascii_case(&dll_name)
                            && let Some(real_addr) = dll.exports.get(&func_name)
                        {
                            final_addr = *real_addr;
                            break;
                        }
                    }
                }

                // 2단계: 못 찾았다면 프록시 DLL(Rust)에서 특수하게 처리하는 데이터 주소 등이 있는지 확인
                let mut final_addr = if final_addr == 0 {
                    Win32Context::resolve_proxy_export(uc, &dll_name, &func_name)
                        .map(|a| a as u64)
                        .unwrap_or(0)
                } else {
                    final_addr
                };

                // 3단계: 여전히 못 찾았다면 Fake Address (Hooking 용) 할당
                let context = uc.get_data();
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
                uc.write_u32(iat_addr, final_addr as u32);

                ilt_rva += 4;
                iat_rva += 4;
            }
            desc_addr += 20; // 다음 IMAGE_IMPORT_DESCRIPTOR로 이동
        }
    }

    // [5] 현재 모듈을 로드된 모듈 목록에 추가
    {
        let context = uc.get_data();
        context
            .dll_modules
            .lock()
            .unwrap()
            .insert(target_dll_name, target.clone());
    }

    finalize_image_protection(uc, image_base, &pe)?;

    Ok(())
}

/// DLL의 엔트리 포인트(DllMain) 함수를 실행합니다.
///
/// # 인자
/// * `dll`: 엔트리 포인트를 실행할 DLL 객체
///
/// # 반환
/// * `Result<(), ()>`: 실행 성공 시 `Ok(())` (내부 에러 발생 시 로그 출력 후 진행)
pub(crate) fn run_dll_entry_impl(
    uc: &mut Unicorn<Win32Context>,
    dll: &LoadedDll,
) -> Result<(), ()> {
    if dll.entry_point == 0 {
        return Ok(());
    }

    let esp = uc.reg_read(RegisterX86::ESP).map_err(|_| ())?;

    // DllMain(hInstance, fdwReason, lpReserved) 호출 준비
    // x86 stdcall: 인자를 역순으로 push
    uc.push_u32(0u32); // lpReserved (arg3)
    uc.push_u32(1u32); // fdwReason = DLL_PROCESS_ATTACH (arg2)
    uc.push_u32(dll.base_addr as u32); // hInstance (arg1)
    uc.push_u32(EXIT_ADDRESS as u32); // 리턴 주소

    crate::emu_log!(
        "[*] Calling DllMain for {} at {:#x}...",
        dll.name,
        dll.entry_point
    );

    if let Err(e) = uc.emu_start(dll.entry_point, EXIT_ADDRESS, 0, 0) {
        crate::emu_log!("[!] DllMain for {} failed: {:?}", dll.name, e);
    } else {
        crate::emu_log!("[*] DllMain for {} finished successfully.", dll.name);
    }

    // 스택 복구
    let _ = uc.reg_write(RegisterX86::ESP, esp);
    Ok(())
}
