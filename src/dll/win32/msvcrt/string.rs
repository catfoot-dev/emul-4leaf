use crate::{
    dll::win32::{ApiHookResult, Win32Context},
    helper::UnicornHelper,
};
use unicorn_engine::Unicorn;

fn read_c_bytes(uc: &Unicorn<Win32Context>, addr: u32, limit: usize) -> Vec<u8> {
    if addr == 0 || limit == 0 {
        return Vec::new();
    }

    let mut out = Vec::new();
    for offset in 0..limit {
        let byte = uc.read_u8(addr as u64 + offset as u64);
        out.push(byte);
        if byte == 0 {
            break;
        }
    }
    out
}

// API: int strncmp(const char* str1, const char* str2, size_t count)
// 역할: 두 문자열을 지정된 길이만큼 비교
pub(super) fn strncmp(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let s1_addr = uc.read_arg(0);
    let s2_addr = uc.read_arg(1);
    let n = uc.read_arg(2) as usize;
    let mut result = 0i32;
    let mut r1 = Vec::with_capacity(n);
    let mut r2 = Vec::with_capacity(n);

    for offset in 0..n {
        let b1 = if s1_addr != 0 {
            uc.read_u8(s1_addr as u64 + offset as u64)
        } else {
            0
        };
        let b2 = if s2_addr != 0 {
            uc.read_u8(s2_addr as u64 + offset as u64)
        } else {
            0
        };
        r1.push(b1);
        r2.push(b2);
        if b1 != b2 {
            result = (b1 as i32) - (b2 as i32);
            break;
        }
        if b1 == 0 {
            break;
        }
    }
    crate::emu_log!(
        "[MSVCRT] strncmp({:02x?}, {:02x?}, {}) -> int {}",
        r1,
        r2,
        n,
        result
    );
    Some(ApiHookResult::callee(3, Some(result)))
}

// API: int strcoll(const char* str1, const char* str2)
// 역할: 현재 로캘을 사용하여 두 문자열을 비교
pub(super) fn strcoll(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let s1_addr = uc.read_arg(0);
    let s1 = if s1_addr != 0 {
        uc.read_euc_kr(s1_addr as u64)
    } else {
        String::new()
    };
    let s2_addr = uc.read_arg(1);
    let s2 = if s2_addr != 0 {
        uc.read_euc_kr(s2_addr as u64)
    } else {
        String::new()
    };
    let result = s1.cmp(&s2) as i32;
    crate::emu_log!("[MSVCRT] strcoll(\"{}\", \"{}\") -> int {}", s1, s2, result);
    Some(ApiHookResult::callee(2, Some(result)))
}

// API: char* strncpy(char* dest, const char* src, size_t count)
// 역할: 문자열을 지정된 길이만큼 복사
pub(super) fn strncpy(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let dst = uc.read_arg(0);
    let src = uc.read_arg(1);
    let n = uc.read_arg(2) as usize;
    let mut bytes = Vec::with_capacity(n);

    for offset in 0..n {
        let byte = if src != 0 {
            uc.read_u8(src as u64 + offset as u64)
        } else {
            0
        };
        bytes.push(byte);
        if byte == 0 {
            bytes.resize(n, 0);
            break;
        }
    }
    while bytes.len() < n {
        bytes.push(0);
    }
    uc.mem_write(dst as u64, &bytes).unwrap();
    crate::emu_log!(
        "[MSVCRT] strncpy({:#x}, {:02x?}, {}) -> char* {:#x}",
        dst,
        bytes,
        n,
        dst
    );
    Some(ApiHookResult::callee(3, Some(dst as i32)))
}

// API: char* strstr(const char* str, const char* substr)
// 역할: 문자열 내에서 부분 문자열을 검색
pub(super) fn strstr(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let s1_addr = uc.read_arg(0);
    let s1 = if s1_addr != 0 {
        uc.read_euc_kr(s1_addr as u64)
    } else {
        String::new()
    };
    let s2_addr = uc.read_arg(1);
    let s2 = if s2_addr != 0 {
        uc.read_euc_kr(s2_addr as u64)
    } else {
        String::new()
    };
    let result = s1.find(&s2).map(|pos| s1_addr + pos as u32).unwrap_or(0);
    crate::emu_log!(
        "[MSVCRT] strstr(\"{}\", \"{}\") -> char* {:#x}",
        s1,
        s2,
        result
    );
    Some(ApiHookResult::callee(2, Some(result as i32)))
}

// API: char* strrchr(const char* str, int ch)
// 역할: 문자열에서 특정 문자가 마지막으로 나타나는 위치를 검색
pub(super) fn strrchr(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let s_addr = uc.read_arg(0);
    let s = if s_addr != 0 {
        uc.read_euc_kr(s_addr as u64)
    } else {
        String::new()
    };
    let ch = uc.read_arg(1) as u8 as char;
    let result = s.rfind(ch).map(|pos| s_addr + pos as u32).unwrap_or(0);
    crate::emu_log!(
        "[MSVCRT] strrchr(\"{}\", '{}') -> char* {:#x}",
        s,
        ch,
        result
    );
    Some(ApiHookResult::callee(2, Some(result as i32)))
}

// API: char* strtok(char* str, const char* sep)
// 역할: 문자열을 구분자로 분리
pub(super) fn strtok(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let s_addr = uc.read_arg(0);
    let sep_addr = uc.read_arg(1);
    let sep = if sep_addr != 0 {
        uc.read_euc_kr(sep_addr as u64)
    } else {
        String::new()
    };

    // Static state for strtok (global across calls)
    static mut NEXT_TOKEN: u32 = 0;

    let mut current_pos = if s_addr != 0 {
        s_addr
    } else {
        unsafe { NEXT_TOKEN }
    };

    if current_pos == 0 {
        crate::emu_log!("[MSVCRT] strtok(NULL, \"{}\") -> NULL (no state)", sep);
        return Some(ApiHookResult::callee(2, Some(0)));
    }

    // Skip leading separators
    loop {
        let ch = uc.read_u8(current_pos as u64);
        if ch == 0 {
            unsafe { NEXT_TOKEN = 0 };
            crate::emu_log!("[MSVCRT] strtok(..., \"{}\") -> NULL (empty)", sep);
            return Some(ApiHookResult::callee(2, Some(0)));
        }
        if !sep.contains(ch as char) {
            break;
        }
        current_pos += 1;
    }

    let token_start = current_pos;

    // Find next separator or end of string
    loop {
        let ch = uc.read_u8(current_pos as u64);
        if ch == 0 {
            unsafe { NEXT_TOKEN = 0 };
            break;
        }
        if sep.contains(ch as char) {
            // Terminate token by writing NULL
            uc.write_u8(current_pos as u64, 0);
            unsafe { NEXT_TOKEN = current_pos + 1 };
            break;
        }
        current_pos += 1;
    }

    let token_str = uc.read_euc_kr(token_start as u64);
    crate::emu_log!(
        "[MSVCRT] strtok({:#x}, \"{}\") -> char* {:#x} (\"{}\")",
        s_addr,
        sep,
        token_start,
        token_str
    );

    Some(ApiHookResult::callee(2, Some(token_start as i32)))
}

// API: int _stricmp(const char* str1, const char* str2)
// 역할: 대소문자 구분 없이 두 문자열을 비교
pub(super) fn _stricmp(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let s1_addr = uc.read_arg(0);
    let s1 = if s1_addr != 0 {
        uc.read_euc_kr(s1_addr as u64)
    } else {
        String::new()
    };
    let s2_addr = uc.read_arg(1);
    let s2 = if s2_addr != 0 {
        uc.read_euc_kr(s2_addr as u64)
    } else {
        String::new()
    };
    let result = s1.to_lowercase().cmp(&s2.to_lowercase()) as i32;
    crate::emu_log!(
        "[MSVCRT] _stricmp(\"{}\", \"{}\") -> int {}",
        s1,
        s2,
        result
    );
    Some(ApiHookResult::callee(2, Some(result)))
}

pub(super) fn _strcmpi(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    _stricmp(uc)
}

// API: int _strnicmp(const char* str1, const char* str2, size_t count)
// 역할: 대소문자 구분 없이 지정된 길이만큼 두 문자열을 비교
pub(super) fn _strnicmp(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let s1_addr = uc.read_arg(0);
    let s2_addr = uc.read_arg(1);
    let n = uc.read_arg(2) as usize;
    let b1 = read_c_bytes(uc, s1_addr, n)
        .into_iter()
        .map(|b| b.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let b2 = read_c_bytes(uc, s2_addr, n)
        .into_iter()
        .map(|b| b.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let result = b1.cmp(&b2) as i32;
    crate::emu_log!(
        "[MSVCRT] _strnicmp({:02x?}, {:02x?}, {}) -> int {}",
        b1,
        b2,
        n,
        result
    );
    Some(ApiHookResult::callee(3, Some(result)))
}

// =========================================================
// Character classification
// =========================================================
pub(super) fn isspace(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let ch = uc.read_arg(0) as u8;
    let result = if (ch as char).is_ascii_whitespace() {
        1
    } else {
        0
    };
    crate::emu_log!("[MSVCRT] isspace({}) -> int {}", ch, result);
    Some(ApiHookResult::callee(1, Some(result)))
}

pub(super) fn isdigit(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let ch = uc.read_arg(0) as u8;
    let result = if (ch as char).is_ascii_digit() { 1 } else { 0 };
    crate::emu_log!("[MSVCRT] isdigit({}) -> int {}", ch, result);
    Some(ApiHookResult::callee(1, Some(result)))
}

pub(super) fn isalnum(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let ch = uc.read_arg(0) as u8;
    let result = if (ch as char).is_ascii_alphanumeric() {
        1
    } else {
        0
    };
    crate::emu_log!("[MSVCRT] isalnum({}) -> int {}", ch, result);
    Some(ApiHookResult::callee(1, Some(result)))
}

// API: void* memcmp — handled in memory module but memchr is in string context
// memmove and memchr are in memory.rs
