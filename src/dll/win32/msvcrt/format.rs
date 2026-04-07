use crate::{
    dll::win32::{ApiHookResult, Win32Context},
    helper::UnicornHelper,
};
use encoding_rs::EUC_KR;
use std::io::{Read, Write};
use unicorn_engine::Unicorn;

/// 포맷 문자열 파싱 및 에뮬레이트 메모리 기반 sprintf 구현
/// 포맷 문자열을 파싱하여 가변 인자 개수를 카운팅하는 헬퍼 (printf 계열)
/// 반환: 포맷 스펙이 소비하는 스택 슬롯 수 (double은 2 슬롯)

/// 포맷 문자열 파싱 및 에뮬레이트 메모리 기반 sprintf 구현
/// 반환: (결과 문자열, 소비된 전체 인자 수)
pub(super) fn format_string(
    uc: &mut Unicorn<Win32Context>,
    fmt_addr: u32,
    first_vararg_index: usize,
) -> (String, usize) {
    let fmt = uc.read_euc_kr(fmt_addr as u64);
    let mut result = String::new();
    let mut arg_idx = first_vararg_index;
    let mut chars = fmt.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '%' {
            result.push(ch);
            continue;
        }

        // Parse format specifier: %[flags][width][.precision][length]type
        let mut spec = String::new();
        let mut zero_pad = false;
        let mut width: Option<usize> = None;

        // Flags
        while let Some(&c) = chars.peek() {
            match c {
                '-' | '+' | ' ' | '#' => {
                    spec.push(c);
                    chars.next();
                }
                '0' => {
                    zero_pad = true;
                    spec.push(c);
                    chars.next();
                }
                _ => break,
            }
        }

        // Width
        let mut width_str = String::new();
        if chars.peek() == Some(&'*') {
            chars.next();
            width = Some(uc.read_arg(arg_idx) as usize);
            arg_idx += 1;
        } else {
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() {
                    width_str.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            if !width_str.is_empty() {
                width = width_str.parse().ok();
            }
        }

        // Precision
        if chars.peek() == Some(&'.') {
            chars.next();
            if chars.peek() == Some(&'*') {
                chars.next();
                let _precision = uc.read_arg(arg_idx);
                arg_idx += 1;
            } else {
                while let Some(&c) = chars.peek() {
                    if c.is_ascii_digit() {
                        chars.next();
                    } else {
                        break;
                    }
                }
            }
        }

        // Length modifier (l, h, etc)
        while let Some(&c) = chars.peek() {
            match c {
                'l' | 'h' | 'L' | 'z' | 'j' | 't' => {
                    chars.next();
                }
                _ => break,
            }
        }

        // Type
        if let Some(type_ch) = chars.next() {
            match type_ch {
                '%' => result.push('%'),
                'd' | 'i' => {
                    let val = uc.read_arg(arg_idx) as i32;
                    arg_idx += 1;
                    let s = format!("{}", val);
                    if let Some(w) = width {
                        if zero_pad {
                            result.push_str(&format!("{:0>width$}", s, width = w));
                        } else {
                            result.push_str(&format!("{:>width$}", s, width = w));
                        }
                    } else {
                        result.push_str(&s);
                    }
                }
                'u' => {
                    let val = uc.read_arg(arg_idx);
                    arg_idx += 1;
                    let s = format!("{}", val);
                    if let Some(w) = width {
                        result.push_str(&format!("{:>width$}", s, width = w));
                    } else {
                        result.push_str(&s);
                    }
                }
                'x' => {
                    let val = uc.read_arg(arg_idx);
                    arg_idx += 1;
                    let s = format!("{:x}", val);
                    if let Some(w) = width {
                        if zero_pad {
                            result.push_str(&format!("{:0>width$}", s, width = w));
                        } else {
                            result.push_str(&format!("{:>width$}", s, width = w));
                        }
                    } else {
                        result.push_str(&s);
                    }
                }
                'X' => {
                    let val = uc.read_arg(arg_idx);
                    arg_idx += 1;
                    let s = format!("{:X}", val);
                    if let Some(w) = width {
                        if zero_pad {
                            result.push_str(&format!("{:0>width$}", s, width = w));
                        } else {
                            result.push_str(&format!("{:>width$}", s, width = w));
                        }
                    } else {
                        result.push_str(&s);
                    }
                }
                'o' => {
                    let val = uc.read_arg(arg_idx);
                    arg_idx += 1;
                    result.push_str(&format!("{:o}", val));
                }
                'c' => {
                    let val = uc.read_arg(arg_idx) as u8 as char;
                    arg_idx += 1;
                    result.push(val);
                }
                's' => {
                    let str_addr = uc.read_arg(arg_idx);
                    arg_idx += 1;
                    if str_addr != 0 {
                        let s = uc.read_euc_kr(str_addr as u64);
                        result.push_str(&s);
                    } else {
                        result.push_str("(null)");
                    }
                }
                'p' => {
                    let val = uc.read_arg(arg_idx);
                    arg_idx += 1;
                    result.push_str(&format!("{:08X}", val));
                }
                'f' | 'e' | 'E' | 'g' | 'G' => {
                    // float/double: 스택에서 8바이트 (double). 간략화: 0.0으로 처리
                    arg_idx += 2; // double은 스택에서 8바이트
                    result.push_str("0.000000");
                }
                _ => {
                    result.push('%');
                    result.push(type_ch);
                }
            }
        }
    }
    (result, arg_idx)
}

// API: int sprintf(char* str, const char* format, ...)
// 역할: 서식화된 데이터를 문자열로 출력
pub(super) fn sprintf(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let buf_addr = uc.read_arg(0);
    let fmt_addr = uc.read_arg(1);
    let (result, total_args) = format_string(uc, fmt_addr, 2);
    let (encoded, _, _) = EUC_KR.encode(&result);
    let bytes = encoded.as_ref();
    let mut buf = bytes.to_vec();
    buf.push(0); // null terminator
    uc.mem_write(buf_addr as u64, &buf).unwrap();
    crate::emu_log!(
        "[MSVCRT] sprintf({:#x}, ...) -> \"{}\" (len={}, args={})",
        buf_addr,
        result,
        bytes.len(),
        total_args
    );
    Some(ApiHookResult::callee(total_args, Some(bytes.len() as i32))) // cdecl, 가변 인자 포함
}

// API: int _vsnprintf(char* str, size_t count, const char* format, va_list argptr)
// 역할: va_list를 사용하여 서식화된 데이터를 문자열 버퍼에 출력
pub(super) fn _vsnprintf(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let buf_addr = uc.read_arg(0);
    let size = uc.read_arg(1);
    let fmt_addr = uc.read_arg(2);
    let fmt = uc.read_euc_kr(fmt_addr as u64);
    let (result, total_args) = format_string(uc, fmt_addr, 3);
    let (encoded, _, _) = EUC_KR.encode(&result);
    let bytes = encoded.as_ref();
    let copy_len = bytes.len().min((size as usize).saturating_sub(1));
    let mut buf = bytes[..copy_len].to_vec();
    buf.push(0);
    uc.mem_write(buf_addr as u64, &buf).unwrap();
    crate::emu_log!(
        "[MSVCRT] _vsnprintf({:#x}, {}, \"{}\", ...) -> \"{}\"",
        buf_addr,
        size,
        fmt,
        result
    );
    Some(ApiHookResult::callee(total_args, Some(copy_len as i32)))
}

// API: int vsprintf(char* str, const char* format, va_list argptr)
// 역할: va_list를 사용하여 서식화된 데이터를 문자열로 출력
pub(super) fn vsprintf(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let buf_addr = uc.read_arg(0);
    let fmt_addr = uc.read_arg(1);
    let (result, total_args) = format_string(uc, fmt_addr, 2);
    let (encoded, _, _) = EUC_KR.encode(&result);
    let bytes = encoded.as_ref();
    let mut buf = bytes.to_vec();
    buf.push(0);
    uc.mem_write(buf_addr as u64, &buf).unwrap();
    crate::emu_log!("[MSVCRT] vsprintf({:#x}, ...) -> \"{}\"", buf_addr, result);
    Some(ApiHookResult::callee(total_args, Some(bytes.len() as i32)))
}

// API: int sscanf(const char* str, const char* format, ...)
// 역할: 문자열에서 서식화된 데이터를 읽음
pub(super) fn sscanf(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let input_addr = uc.read_arg(0);
    let fmt_addr = uc.read_arg(1);
    let input = uc.read_euc_kr(input_addr as u64);
    let fmt = uc.read_euc_kr(fmt_addr as u64);

    let mut arg_idx = 2;
    let mut count = 0;
    let mut input_ptr = input.as_str();

    let mut fmt_chars = fmt.chars().peekable();
    while let Some(ch) = fmt_chars.next() {
        if ch == '%' {
            if let Some(type_ch) = fmt_chars.next() {
                match type_ch {
                    'd' => {
                        let end = input_ptr
                            .find(|c: char| !c.is_numeric() && c != '-')
                            .unwrap_or(input_ptr.len());
                        let val_str = &input_ptr[..end];
                        if let Ok(val) = val_str.parse::<i32>() {
                            let target_addr = uc.read_arg(arg_idx);
                            uc.write_u32(target_addr as u64, val as u32);
                            arg_idx += 1;
                            count += 1;
                            input_ptr = &input_ptr[end..];
                        }
                    }
                    'x' => {
                        let end = input_ptr
                            .find(|c: char| !c.is_ascii_hexdigit())
                            .unwrap_or(input_ptr.len());
                        let val_str = &input_ptr[..end];
                        if let Ok(val) = u32::from_str_radix(val_str, 16) {
                            let target_addr = uc.read_arg(arg_idx);
                            uc.write_u32(target_addr as u64, val);
                            arg_idx += 1;
                            count += 1;
                            input_ptr = &input_ptr[end..];
                        }
                    }
                    's' => {
                        // Skip whitespace
                        input_ptr = input_ptr.trim_start();
                        let end = input_ptr
                            .find(|c: char| c.is_whitespace())
                            .unwrap_or(input_ptr.len());
                        let val_str = &input_ptr[..end];
                        let target_addr = uc.read_arg(arg_idx);
                        let mut bytes = val_str.as_bytes().to_vec();
                        bytes.push(0);
                        uc.mem_write(target_addr as u64, &bytes).unwrap();
                        arg_idx += 1;
                        count += 1;
                        input_ptr = &input_ptr[end..];
                    }
                    _ => {}
                }
            }
        } else if ch.is_whitespace() {
            input_ptr = input_ptr.trim_start();
        } else {
            if input_ptr.starts_with(ch) {
                input_ptr = &input_ptr[1..];
            }
        }
    }

    crate::emu_log!(
        "[MSVCRT] sscanf(\"{}\", \"{}\") -> int {}",
        input,
        fmt,
        count
    );
    Some(ApiHookResult::callee(arg_idx, Some(count as i32)))
}

// API: int fprintf(FILE* stream, const char* format, ...)
// 역할: 스트림에 서식화된 데이터를 출력
pub(super) fn fprintf(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let stream_handle = uc.read_arg(0);
    let fmt_addr = uc.read_arg(1);
    let (result, total_args) = format_string(uc, fmt_addr, 2);

    let bytes = result.as_bytes();
    let context = uc.get_data();
    let mut files = context.files.lock().unwrap();
    if let Some(file) = files.get_mut(&stream_handle) {
        let _ = file.write_all(bytes);
        crate::emu_log!(
            "[MSVCRT] fprintf({:#x}, ...) -> \"{}\" (len={}, args={})",
            stream_handle,
            result,
            bytes.len(),
            total_args
        );
        Some(ApiHookResult::callee(total_args, Some(bytes.len() as i32)))
    } else {
        crate::emu_log!(
            "[MSVCRT] fprintf({:#x}, ...) - handle not found",
            stream_handle
        );
        Some(ApiHookResult::callee(total_args, Some(-1)))
    }
}

// API: int fscanf(FILE* stream, const char* format, ...)
// 역할: 스트림에서 서식화된 데이터를 읽음
pub(super) fn fscanf(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let stream_handle = uc.read_arg(0);
    let fmt_addr = uc.read_arg(1);
    let fmt = uc.read_euc_kr(fmt_addr as u64);

    let mut data = Vec::new();
    {
        let context = uc.get_data();
        let mut files = context.files.lock().unwrap();
        if let Some(file) = files.get_mut(&stream_handle) {
            // Read everything for now (simplified)
            let _ = file.read_to_end(&mut data);
        }
    }

    if data.is_empty() {
        return Some(ApiHookResult::callee(2, Some(-1))); // EOF
    }

    let input = String::from_utf8_lossy(&data).to_string();
    let mut arg_idx = 2;
    let mut count = 0;
    let mut input_ptr = input.as_str();

    let mut fmt_chars = fmt.chars().peekable();
    while let Some(ch) = fmt_chars.next() {
        if ch == '%' {
            if let Some(type_ch) = fmt_chars.next() {
                match type_ch {
                    'd' => {
                        input_ptr = input_ptr.trim_start();
                        let end = input_ptr
                            .find(|c: char| !c.is_numeric() && c != '-')
                            .unwrap_or(input_ptr.len());
                        let val_str = &input_ptr[..end];
                        if let Ok(val) = val_str.parse::<i32>() {
                            let target_addr = uc.read_arg(arg_idx);
                            uc.write_u32(target_addr as u64, val as u32);
                            arg_idx += 1;
                            count += 1;
                            input_ptr = &input_ptr[end..];
                        }
                    }
                    'x' => {
                        input_ptr = input_ptr.trim_start();
                        let end = input_ptr
                            .find(|c: char| !c.is_ascii_hexdigit())
                            .unwrap_or(input_ptr.len());
                        let val_str = &input_ptr[..end];
                        if let Ok(val) = u32::from_str_radix(val_str, 16) {
                            let target_addr = uc.read_arg(arg_idx);
                            uc.write_u32(target_addr as u64, val);
                            arg_idx += 1;
                            count += 1;
                            input_ptr = &input_ptr[end..];
                        }
                    }
                    's' => {
                        input_ptr = input_ptr.trim_start();
                        let end = input_ptr
                            .find(|c: char| c.is_whitespace())
                            .unwrap_or(input_ptr.len());
                        let val_str = &input_ptr[..end];
                        let target_addr = uc.read_arg(arg_idx);
                        let mut bytes = val_str.as_bytes().to_vec();
                        bytes.push(0);
                        uc.mem_write(target_addr as u64, &bytes).unwrap();
                        arg_idx += 1;
                        count += 1;
                        input_ptr = &input_ptr[end..];
                    }
                    _ => {}
                }
            }
        } else if ch.is_whitespace() {
            input_ptr = input_ptr.trim_start();
        } else {
            if input_ptr.starts_with(ch) {
                input_ptr = &input_ptr[1..];
            }
        }
    }

    crate::emu_log!(
        "[MSVCRT] fscanf({:#x}, \"{}\") -> int {}",
        stream_handle,
        fmt,
        count
    );
    Some(ApiHookResult::callee(arg_idx, Some(count as i32)))
}
