use crate::dll::win32::{StackCleanup, Win32Context};
use unicorn_engine::{RegisterX86, Unicorn};

// pub const HOOK_BASE: u64 = 0x1000_0000;
pub const HEAP_BASE: u64 = 0x2000_0000;
pub const HEAP_SIZE: u64 = 256 * 1024 * 1024;

pub const STACK_BASE: u64 = 0x5000_0000;
pub const STACK_SIZE: u64 = 10 * 1024 * 1024;
pub const STACK_TOP: u64 = STACK_BASE + STACK_SIZE;

pub const SHARED_MEM_BASE: u64 = 0x7000_0000;

// const FUNCTION_NAME_BASE: u64 = 0x8000_0000;

pub const TEB_BASE: u64 = 0x9000_0000;
pub const FAKE_IMPORT_BASE: u64 = 0xF000_0000;
pub const EXIT_ADDRESS: u64 = 0xFFFF_FFFF;

pub(super) const SIZE_4KB: u64 = 4 * 1024;

pub(crate) fn read_u32_impl(uc: &Unicorn<Win32Context>, addr: u64) -> u32 {
    let mut buf = [0u8; 4];
    if uc.mem_read(addr, &mut buf).is_ok() {
        u32::from_le_bytes(buf)
    } else {
        0
    }
}

pub(crate) fn read_i32_impl(uc: &Unicorn<Win32Context>, addr: u64) -> i32 {
    read_u32_impl(uc, addr) as i32
}

pub(crate) fn read_u16_impl(uc: &Unicorn<Win32Context>, addr: u64) -> u16 {
    let mut buf = [0u8; 2];
    if uc.mem_read(addr, &mut buf).is_ok() {
        u16::from_le_bytes(buf)
    } else {
        0
    }
}

pub(crate) fn write_u32_impl(uc: &mut Unicorn<Win32Context>, addr: u64, value: u32) {
    let _ = uc.mem_write(addr, &value.to_le_bytes());
}

pub(crate) fn write_u16_impl(uc: &mut Unicorn<Win32Context>, addr: u64, value: u16) {
    let _ = uc.mem_write(addr, &value.to_le_bytes());
}

pub(crate) fn read_arg_impl(uc: &Unicorn<Win32Context>, index: usize) -> u32 {
    let esp = uc.reg_read(RegisterX86::ESP).unwrap_or(0);
    // [ESP] = Return Address, [ESP+4] = Arg0, ...
    let addr = esp + 4 + (index as u64 * 4);
    read_u32_impl(uc, addr)
}

pub(crate) fn read_u8_impl(uc: &Unicorn<Win32Context>, addr: u64) -> u8 {
    let mut buf = [0u8; 1];
    uc.mem_read(addr, &mut buf).unwrap();
    buf[0]
}

pub(crate) fn write_u8_impl(uc: &mut Unicorn<Win32Context>, addr: u64, value: u8) {
    uc.mem_write(addr, &[value]).unwrap();
}

pub(crate) fn read_string_bytes_impl(
    uc: &Unicorn<Win32Context>,
    addr: u64,
    max_len: usize,
) -> Vec<u8> {
    let mut chars = Vec::new();
    let mut curr = addr;

    while chars.len() < max_len {
        let mut buf = [0u8; 1];
        if uc.mem_read(curr, &mut buf).is_err() || buf[0] == 0 {
            break;
        }
        chars.push(buf[0]);
        curr += 1;
    }
    chars
}

pub(crate) fn read_string_impl(uc: &Unicorn<Win32Context>, addr: u64) -> String {
    let bytes = read_string_bytes_impl(uc, addr, 1024);
    String::from_utf8_lossy(&bytes).to_string()
}

pub(crate) fn write_string_impl(uc: &mut Unicorn<Win32Context>, addr: u64, text: &str) {
    let bytes = text.as_bytes();
    let _ = uc.mem_write(addr, bytes);
    let _ = uc.mem_write(addr + bytes.len() as u64, &[0u8]); // Null terminator
}

pub(crate) fn push_u32_impl(uc: &mut Unicorn<Win32Context>, value: u32) {
    let esp = uc.reg_read(RegisterX86::ESP).unwrap_or(0);
    let new_esp = esp - 4;
    write_u32_impl(uc, new_esp, value);
    let _ = uc.reg_write(RegisterX86::ESP, new_esp);
}

#[allow(dead_code)]
pub(crate) fn pop_u32_impl(uc: &mut Unicorn<Win32Context>) -> u32 {
    let esp = uc.reg_read(RegisterX86::ESP).unwrap_or(0);
    let val = read_u32_impl(uc, esp);
    let _ = uc.reg_write(RegisterX86::ESP, esp + 4);
    val
}

pub(crate) fn apply_stack_cleanup_impl(uc: &mut Unicorn<Win32Context>, cleanup: StackCleanup) {
    let esp = uc.reg_read(RegisterX86::ESP).unwrap_or(0);
    if let Some(new_esp) = super::stack_cleanup_target_esp(esp, cleanup) {
        // [현재 ESP]에 있는 리턴 주소를 [new_esp] 위치로 옮김
        let ret_addr = read_u32_impl(uc, esp);
        write_u32_impl(uc, new_esp, ret_addr);
        let _ = uc.reg_write(RegisterX86::ESP, new_esp);
    }
}

pub(crate) fn malloc_impl(uc: &mut Unicorn<Win32Context>, size: usize) -> u64 {
    let ctx = uc.get_data();
    match ctx.alloc_heap_block(size) {
        Some(addr) => {
            crate::diagnostics::record_heap_alloc(addr as u64, size.max(1));
            addr as u64
        }
        None => {
            let cursor = ctx.heap_cursor.load(std::sync::atomic::Ordering::SeqCst);
            crate::emu_log!(
                "[!] HEAP OVERFLOW size={} cursor={:#x} limit={:#x}",
                size.max(1),
                cursor,
                HEAP_BASE + HEAP_SIZE
            );
            0
        }
    }
}

pub(crate) fn alloc_str_impl(uc: &mut Unicorn<Win32Context>, text: &str) -> u32 {
    let bytes = text.as_bytes();
    let addr = malloc_impl(uc, bytes.len() + 1);
    write_string_impl(uc, addr, text);
    addr as u32
}

#[allow(dead_code)]
pub(crate) fn alloc_bytes_impl(uc: &mut Unicorn<Win32Context>, data: &[u8]) -> u32 {
    let addr = malloc_impl(uc, data.len());
    let _ = uc.mem_write(addr, data);
    addr as u32
}

pub(crate) fn write_mem_impl(uc: &mut Unicorn<Win32Context>, addr: u64, data: &[i32]) {
    for (i, &val) in data.iter().enumerate() {
        write_u32_impl(uc, addr + (i * 4) as u64, val as u32);
    }
}

pub(crate) fn resolve_address_impl(uc: &Unicorn<Win32Context>, addr: u32) -> String {
    let ctx = uc.get_data();
    if let Some(import_name) = ctx.address_map.lock().unwrap().get(&(addr as u64)).cloned() {
        return import_name;
    }

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
