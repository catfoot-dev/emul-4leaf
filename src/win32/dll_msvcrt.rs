use unicorn_engine::RegisterX86;
use unicorn_engine::Unicorn;

use crate::helper::UnicornHelper;
use crate::win32::{ApiHookResult, Win32Context, callee_result, caller_result};

pub struct DllMSVCRT {}

impl DllMSVCRT {
    fn wrap_result(func_name: &str, result: Option<(usize, Option<i32>)>) -> Option<ApiHookResult> {
        match func_name {
            "_CxxThrowException" => callee_result(result),
            _ => caller_result(result),
        }
    }

    // =========================================================
    // Memory
    // =========================================================
    pub fn malloc(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let size = uc.read_arg(0);
        let addr = uc.malloc(size as usize);
        println!("[MSVCRT] malloc({}) -> {:#x}", size, addr);
        Some((1, Some(addr as i32)))
    }

    pub fn free(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        // 간이 힙이므로 free는 아무 작업도 수행하지 않음
        println!("[MSVCRT] free(...)");
        Some((1, None))
    }

    pub fn calloc(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let num = uc.read_arg(0);
        let size = uc.read_arg(1);
        let total = (num * size) as usize;
        let addr = uc.malloc(total);
        let zeros = vec![0u8; total];
        uc.mem_write(addr, &zeros).unwrap();
        println!("[MSVCRT] calloc({}, {}) -> {:#x}", num, size, addr);
        Some((2, Some(addr as i32)))
    }

    pub fn realloc(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let _ptr = uc.read_arg(0);
        let size = uc.read_arg(1);
        let addr = uc.malloc(size as usize);
        // 간이 구현: 이전 데이터의 복사는 생략
        println!("[MSVCRT] realloc({:#x}, {}) -> {:#x}", _ptr, size, addr);
        Some((2, Some(addr as i32)))
    }

    pub fn new_op(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let size = uc.read_arg(0);
        let addr = uc.malloc(size as usize);
        println!("[MSVCRT] operator new({}) -> {:#x}", size, addr);
        Some((1, Some(addr as i32)))
    }

    pub fn memmove(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let dst = uc.read_arg(0);
        let src = uc.read_arg(1);
        let size = uc.read_arg(2) as usize;
        if size > 0 {
            let data = uc.mem_read_as_vec(src as u64, size).unwrap_or_default();
            uc.mem_write(dst as u64, &data).unwrap();
        }
        println!("[MSVCRT] memmove({:#x}, {:#x}, {})", dst, src, size);
        Some((3, Some(dst as i32)))
    }

    pub fn memchr(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let buf = uc.read_arg(0);
        let ch = uc.read_arg(1) as u8;
        let count = uc.read_arg(2) as usize;
        let data = uc.mem_read_as_vec(buf as u64, count).unwrap_or_default();
        let result = data
            .iter()
            .position(|&b| b == ch)
            .map(|pos| buf + pos as u32)
            .unwrap_or(0);
        println!(
            "[MSVCRT] memchr({:#x}, {}, {}) -> {:#x}",
            buf, ch, count, result
        );
        Some((3, Some(result as i32)))
    }

    // =========================================================
    // String
    // =========================================================
    pub fn strncmp(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let s1_addr = uc.read_arg(0);
        let s2_addr = uc.read_arg(1);
        let n = uc.read_arg(2) as usize;
        let s1 = uc.read_string(s1_addr as u64);
        let s2 = uc.read_string(s2_addr as u64);
        let r1: Vec<u8> = s1.bytes().take(n).collect();
        let r2: Vec<u8> = s2.bytes().take(n).collect();
        let result = r1.cmp(&r2) as i32;
        println!(
            "[MSVCRT] strncmp(\"{}\", \"{}\", {}) -> {}",
            s1, s2, n, result
        );
        Some((3, Some(result)))
    }

    pub fn strcoll(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let s1_addr = uc.read_arg(0);
        let s2_addr = uc.read_arg(1);
        let s1 = uc.read_string(s1_addr as u64);
        let s2 = uc.read_string(s2_addr as u64);
        let result = s1.cmp(&s2) as i32;
        println!("[MSVCRT] strcoll(\"{}\", \"{}\") -> {}", s1, s2, result);
        Some((2, Some(result)))
    }

    pub fn strncpy(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let dst = uc.read_arg(0);
        let src = uc.read_arg(1);
        let n = uc.read_arg(2) as usize;
        let s = uc.read_string(src as u64);
        let mut bytes: Vec<u8> = s.bytes().take(n).collect();
        while bytes.len() < n {
            bytes.push(0);
        }
        uc.mem_write(dst as u64, &bytes).unwrap();
        println!("[MSVCRT] strncpy({:#x}, \"{}\", {})", dst, s, n);
        Some((3, Some(dst as i32)))
    }

    pub fn strstr(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let s1_addr = uc.read_arg(0);
        let s2_addr = uc.read_arg(1);
        let s1 = uc.read_string(s1_addr as u64);
        let s2 = uc.read_string(s2_addr as u64);
        let result = s1.find(&s2).map(|pos| s1_addr + pos as u32).unwrap_or(0);
        println!("[MSVCRT] strstr(\"{}\", \"{}\") -> {:#x}", s1, s2, result);
        Some((2, Some(result as i32)))
    }

    pub fn strrchr(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let s_addr = uc.read_arg(0);
        let ch = uc.read_arg(1) as u8 as char;
        let s = uc.read_string(s_addr as u64);
        let result = s.rfind(ch).map(|pos| s_addr + pos as u32).unwrap_or(0);
        println!("[MSVCRT] strrchr(\"{}\", '{}') -> {:#x}", s, ch, result);
        Some((2, Some(result as i32)))
    }

    pub fn strtok(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] strtok(...)");
        Some((2, Some(0))) // 간이: NULL 반환
    }

    pub fn strtoul(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let s_addr = uc.read_arg(0);
        let _endptr = uc.read_arg(1);
        let base = uc.read_arg(2);
        let s = uc.read_string(s_addr as u64);
        let result = u32::from_str_radix(s.trim(), base as u32).unwrap_or(0);
        println!("[MSVCRT] strtoul(\"{}\", ..., {}) -> {}", s, base, result);
        Some((3, Some(result as i32)))
    }

    pub fn _stricmp(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let s1_addr = uc.read_arg(0);
        let s2_addr = uc.read_arg(1);
        let s1 = uc.read_string(s1_addr as u64).to_lowercase();
        let s2 = uc.read_string(s2_addr as u64).to_lowercase();
        let result = s1.cmp(&s2) as i32;
        println!("[MSVCRT] _stricmp(\"{}\", \"{}\") -> {}", s1, s2, result);
        Some((2, Some(result)))
    }

    pub fn _strcmpi(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        DllMSVCRT::_stricmp(uc)
    }

    pub fn _strnicmp(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let s1_addr = uc.read_arg(0);
        let s2_addr = uc.read_arg(1);
        let n = uc.read_arg(2) as usize;
        let s1: String = uc
            .read_string(s1_addr as u64)
            .chars()
            .take(n)
            .collect::<String>()
            .to_lowercase();
        let s2: String = uc
            .read_string(s2_addr as u64)
            .chars()
            .take(n)
            .collect::<String>()
            .to_lowercase();
        let result = s1.cmp(&s2) as i32;
        println!(
            "[MSVCRT] _strnicmp(\"{}\", \"{}\", {}) -> {}",
            s1, s2, n, result
        );
        Some((3, Some(result)))
    }

    // =========================================================
    // Character classification
    // =========================================================
    pub fn isspace(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let ch = uc.read_arg(0) as u8;
        let result = if (ch as char).is_ascii_whitespace() {
            1
        } else {
            0
        };
        Some((1, Some(result)))
    }

    pub fn isdigit(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let ch = uc.read_arg(0) as u8;
        let result = if (ch as char).is_ascii_digit() { 1 } else { 0 };
        Some((1, Some(result)))
    }

    pub fn isalnum(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let ch = uc.read_arg(0) as u8;
        let result = if (ch as char).is_ascii_alphanumeric() {
            1
        } else {
            0
        };
        Some((1, Some(result)))
    }

    // =========================================================
    // Conversion
    // =========================================================
    pub fn atoi(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let s_addr = uc.read_arg(0);
        let s = uc.read_string(s_addr as u64);
        let result = s.trim().parse::<i32>().unwrap_or(0);
        println!("[MSVCRT] atoi(\"{}\") -> {}", s, result);
        Some((1, Some(result)))
    }

    pub fn _itoa(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let value = uc.read_arg(0) as i32;
        let buf_addr = uc.read_arg(1);
        let radix = uc.read_arg(2);
        let s = match radix {
            16 => format!("{:x}\0", value),
            8 => format!("{:o}\0", value),
            _ => format!("{}\0", value),
        };
        uc.mem_write(buf_addr as u64, s.as_bytes()).unwrap();
        println!(
            "[MSVCRT] _itoa({}, {:#x}, {}) -> \"{}\"",
            value,
            buf_addr,
            radix,
            &s[..s.len() - 1]
        );
        Some((3, Some(buf_addr as i32)))
    }

    // =========================================================
    // Time
    // =========================================================
    pub fn time(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let timer_addr = uc.read_arg(0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as u32;
        if timer_addr != 0 {
            uc.write_u32(timer_addr as u64, now);
        }
        println!("[MSVCRT] time(...) -> {}", now);
        Some((1, Some(now as i32)))
    }

    pub fn localtime(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] localtime(...)");
        Some((1, Some(0))) // NULL (간이)
    }

    pub fn mktime(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] mktime(...)");
        Some((1, Some(0)))
    }

    pub fn _timezone(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] _timezone");
        Some((0, Some(0)))
    }

    // =========================================================
    // Math
    // =========================================================
    pub fn floor(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] floor(...)");
        Some((0, Some(0))) // FPU 기반이라 스택 인자 아님
    }

    pub fn ceil(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] ceil(...)");
        Some((0, Some(0)))
    }

    pub fn frexp(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] frexp(...)");
        Some((0, Some(0)))
    }

    pub fn ldexp(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] ldexp(...)");
        Some((0, Some(0)))
    }

    pub fn __c_ipow(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] _CIpow(...)");
        Some((0, Some(0)))
    }

    pub fn _ftol(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] _ftol(...)");
        Some((0, Some(0)))
    }

    // =========================================================
    // Random
    // =========================================================
    pub fn rand(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let ctx = uc.get_data_mut();
        ctx.rand_state = ctx.rand_state.wrapping_mul(214013).wrapping_add(2531011);
        let val = (ctx.rand_state >> 16) & 0x7FFF;
        Some((0, Some(val as i32)))
    }

    pub fn srand(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let seed = uc.read_arg(0);
        uc.get_data_mut().rand_state = seed;
        println!("[MSVCRT] srand({})", seed);
        Some((1, None))
    }

    // =========================================================
    // =========================================================
    // Format / IO
    // =========================================================

    /// 포맷 문자열 파싱 및 에뮬레이트 메모리 기반 sprintf 구현
    /// 포맷 문자열을 파싱하여 가변 인자 개수를 카운팅하는 헬퍼 (printf 계열)
    /// 반환: 포맷 스펙이 소비하는 스택 슬롯 수 (double은 2 슬롯)
    fn count_printf_varargs(fmt: &str) -> usize {
        let mut count = 0usize;
        let mut chars = fmt.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch != '%' {
                continue;
            }
            // Flags
            while let Some(&c) = chars.peek() {
                match c {
                    '-' | '+' | ' ' | '#' | '0' => {
                        chars.next();
                    }
                    _ => break,
                }
            }
            // Width (* means arg consumed)
            if chars.peek() == Some(&'*') {
                chars.next();
                count += 1;
            } else {
                while let Some(&c) = chars.peek() {
                    if c.is_ascii_digit() {
                        chars.next();
                    } else {
                        break;
                    }
                }
            }
            // Precision
            if chars.peek() == Some(&'.') {
                chars.next();
                if chars.peek() == Some(&'*') {
                    chars.next();
                    count += 1;
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
            // Length modifier
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
                    '%' => {}                                  // %% — no arg consumed
                    'f' | 'e' | 'E' | 'g' | 'G' => count += 2, // double = 8 bytes = 2 slots
                    'n' => count += 1,
                    'd' | 'i' | 'u' | 'x' | 'X' | 'o' | 'c' | 's' | 'p' => count += 1,
                    _ => count += 1, // unknown specifier, assume 1
                }
            }
        }
        count
    }

    /// 포맷 문자열을 파싱하여 가변 인자 개수를 카운팅하는 헬퍼 (scanf 계열)
    /// 반환: 포맷 스펙이 소비하는 스택 슬롯 수 (각 포인터 인자 = 1 슬롯)
    fn count_scanf_varargs(fmt: &str) -> usize {
        let mut count = 0usize;
        let mut chars = fmt.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch != '%' {
                continue;
            }
            // Check for suppression (*)
            let suppressed = if chars.peek() == Some(&'*') {
                chars.next();
                true
            } else {
                false
            };
            // Width
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() {
                    chars.next();
                } else {
                    break;
                }
            }
            // Length modifier
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
                    '%' => {} // literal %
                    '[' => {
                        // scanset — skip until ]
                        if chars.peek() == Some(&']') {
                            chars.next();
                        }
                        while let Some(c) = chars.next() {
                            if c == ']' {
                                break;
                            }
                        }
                        if !suppressed {
                            count += 1;
                        }
                    }
                    _ => {
                        if !suppressed {
                            count += 1;
                        }
                    }
                }
            }
        }
        count
    }

    /// 포맷 문자열 파싱 및 에뮬레이트 메모리 기반 sprintf 구현
    /// 반환: (결과 문자열, 소비된 전체 인자 수)
    fn format_string(
        uc: &mut Unicorn<Win32Context>,
        fmt_addr: u32,
        first_vararg_index: usize,
    ) -> (String, usize) {
        let fmt = uc.read_string(fmt_addr as u64);
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
                            let s = uc.read_string(str_addr as u64);
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

    pub fn sprintf(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let esp_val = uc
            .reg_read(unicorn_engine::RegisterX86::ESP as i32)
            .unwrap();
        let mut buf_dump = [0u8; 32];
        uc.mem_read(esp_val, &mut buf_dump).unwrap();

        let buf_addr = uc.read_arg(0);
        let fmt_addr = uc.read_arg(1);
        if buf_addr == 0 || fmt_addr == 0 {
            let eip = uc.reg_read(RegisterX86::EIP as i32).unwrap();
            let esp = uc.reg_read(RegisterX86::ESP as i32).unwrap();
            println!(
                "[MSVCRT] sprintf invalid args: buf={:#x}, fmt={:#x}, eip={:#x}, esp={:#x}",
                buf_addr, fmt_addr, eip, esp
            );
        }
        let (result, total_args) = DllMSVCRT::format_string(uc, fmt_addr, 2);
        let bytes = result.as_bytes();
        let mut buf = bytes.to_vec();
        buf.push(0); // null terminator
        uc.mem_write(buf_addr as u64, &buf).unwrap();
        println!(
            "[MSVCRT] sprintf({:#x}, ...) -> \"{}\" (len={}, args={})",
            buf_addr,
            result,
            bytes.len(),
            total_args
        );
        Some((total_args, Some(bytes.len() as i32))) // cdecl, 가변 인자 포함
    }

    pub fn _vsnprintf(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let buf_addr = uc.read_arg(0);
        let _size = uc.read_arg(1);
        let fmt_addr = uc.read_arg(2);
        // va_list ptr at arg(3) - 에뮬레이터에서는 스택 기반이므로 직접 파싱 불가
        // 간략 구현: 포맷 문자열만 복사
        let fmt = uc.read_string(fmt_addr as u64);
        let mut buf = fmt.as_bytes().to_vec();
        buf.push(0);
        uc.mem_write(buf_addr as u64, &buf).unwrap();
        println!(
            "[MSVCRT] _vsnprintf({:#x}, {}, ...) -> \"{}\" ",
            buf_addr, _size, fmt
        );
        Some((3, Some(fmt.len() as i32)))
    }

    pub fn vsprintf(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let buf_addr = uc.read_arg(0);
        let fmt_addr = uc.read_arg(1);
        let fmt = uc.read_string(fmt_addr as u64);
        let mut buf = fmt.as_bytes().to_vec();
        buf.push(0);
        uc.mem_write(buf_addr as u64, &buf).unwrap();
        println!("[MSVCRT] vsprintf({:#x}, ...) -> \"{}\"", buf_addr, fmt);
        Some((3, Some(fmt.len() as i32)))
    }

    pub fn sscanf(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let _buf_addr = uc.read_arg(0);
        let fmt_addr = uc.read_arg(1);
        let fmt = uc.read_string(fmt_addr as u64);
        let vararg_count = DllMSVCRT::count_scanf_varargs(&fmt);
        let total_args = 2 + vararg_count; // buf + fmt + varargs
        println!("[MSVCRT] sscanf(..., \"{}\") -> args={}", fmt, total_args);
        Some((total_args, Some(0)))
    }

    pub fn fprintf(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let _stream = uc.read_arg(0);
        let fmt_addr = uc.read_arg(1);
        let fmt = uc.read_string(fmt_addr as u64);
        let vararg_count = DllMSVCRT::count_printf_varargs(&fmt);
        let total_args = 2 + vararg_count; // stream + fmt + varargs
        println!("[MSVCRT] fprintf(..., \"{}\") -> args={}", fmt, total_args);
        Some((total_args, Some(0)))
    }

    pub fn fscanf(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let _stream = uc.read_arg(0);
        let fmt_addr = uc.read_arg(1);
        let fmt = uc.read_string(fmt_addr as u64);
        let vararg_count = DllMSVCRT::count_scanf_varargs(&fmt);
        let total_args = 2 + vararg_count; // stream + fmt + varargs
        println!("[MSVCRT] fscanf(..., \"{}\") -> args={}", fmt, total_args);
        Some((total_args, Some(0)))
    }

    // =========================================================
    // File I/O
    // =========================================================
    pub fn fopen(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] fopen(...)");
        Some((2, Some(0))) // cdecl, NULL 반환
    }

    pub fn fclose(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] fclose(...)");
        Some((1, Some(0))) // cdecl
    }

    pub fn fread(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] fread(...)");
        Some((4, Some(0))) // cdecl
    }

    pub fn fwrite(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] fwrite(...)");
        Some((4, Some(0))) // cdecl
    }

    pub fn fseek(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] fseek(...)");
        Some((3, Some(0))) // cdecl
    }

    pub fn ftell(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] ftell(...)");
        Some((1, Some(0))) // cdecl
    }

    pub fn fflush(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] fflush(...)");
        Some((1, Some(0))) // cdecl
    }

    // Low-level I/O
    pub fn _open(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] _open(...)");
        Some((2, Some(-1))) // cdecl, 에러
    }

    pub fn _close(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] _close(...)");
        Some((1, Some(0))) // cdecl
    }

    pub fn _read(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] _read(...)");
        Some((3, Some(-1))) // cdecl
    }

    pub fn _write(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] _write(...)");
        Some((3, Some(-1))) // cdecl
    }

    pub fn _lseek(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] _lseek(...)");
        Some((3, Some(-1))) // cdecl
    }

    pub fn _pipe(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] _pipe(...)");
        Some((3, Some(-1))) // cdecl
    }

    pub fn _stat(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] _stat(...)");
        Some((2, Some(-1))) // cdecl
    }

    // =========================================================
    // Environment
    // =========================================================
    pub fn getenv(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] getenv(...)");
        Some((1, Some(0))) // cdecl, NULL
    }

    // =========================================================
    // Thread
    // =========================================================
    pub fn _beginthreadex(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] _beginthreadex(...)");
        Some((6, Some(0))) // cdecl
    }

    pub fn _endthreadex(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] _endthreadex(...)");
        Some((1, None)) // cdecl, void
    }

    pub fn _beginthread(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] _beginthread(...)");
        Some((3, Some(0))) // cdecl
    }

    // =========================================================
    // Exception / SEH
    // =========================================================
    pub fn __cxx_throw_exception(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] _CxxThrowException(...)");
        Some((2, None)) // stdcall 2 args
    }

    pub fn _except_handler3(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] _except_handler3(...)");
        Some((4, Some(1))) // cdecl
    }

    pub fn ___cxx_frame_handler(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] __CxxFrameHandler(...)");
        Some((4, Some(0))) // cdecl
    }

    pub fn _set_se_translator(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let se_translator_function = uc.read_arg(0);
        println!("[MSVCRT] _set_se_translator({:#x})", se_translator_function);
        Some((1, Some(0))) // cdecl
    }

    pub fn _setjmp3(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] _setjmp3(...)");
        Some((2, Some(0))) // cdecl, 바로 리턴
    }

    pub fn longjmp(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] longjmp(...)");
        Some((2, None)) // cdecl
    }

    // =========================================================
    // Init / Exit
    // =========================================================
    pub fn _initterm(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        // _initterm(begin, end) - 함수 포인터 테이블을 순회하며 호출
        let begin = uc.read_arg(0) as u64;
        let end = uc.read_arg(1) as u64;
        println!("[MSVCRT] _initterm({:#x}, {:#x})", begin, end);

        let mut addr = begin;
        while addr < end {
            let func_ptr = uc.read_u32(addr);
            if func_ptr != 0 {
                println!("[MSVCRT] _initterm: calling {:#x}", func_ptr);
                // 콜백 호출 (void __cdecl func(void))
                // 리턴 주소를 스택에 push하고 emu_start로 콜백 실행
                let esp = uc.reg_read(unicorn_engine::RegisterX86::ESP).unwrap();
                uc.reg_write(unicorn_engine::RegisterX86::ESP, esp - 4)
                    .unwrap();
                uc.write_u32(esp - 4, 0); // return addr = 0 → EXIT_ADDRESS처럼 동작

                if let Err(e) = uc.emu_start(func_ptr as u64, 0, 0, 10000) {
                    println!(
                        "[MSVCRT] _initterm: callback at {:#x} failed: {:?}",
                        func_ptr, e
                    );
                }

                // 스택 복원 (콜백이 스택을 건드렸을 수 있으므로 원래 ESP로 복구)
                uc.reg_write(unicorn_engine::RegisterX86::ESP, esp).unwrap();
            }
            addr += 4;
        }
        println!("[MSVCRT] _initterm done");
        Some((2, None)) // cdecl
    }

    pub fn _exit(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] _exit(...)");
        Some((1, None)) // cdecl
    }

    pub fn __dllonexit(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] __dllonexit(...)");
        Some((3, Some(0))) // cdecl
    }

    pub fn _onexit(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] _onexit(...)");
        Some((1, Some(0))) // cdecl
    }

    pub fn terminate(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] terminate()");
        Some((0, None))
    }

    pub fn type_info(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] type_info::~type_info()");
        Some((0, None)) // thiscall 가능하지만 cdecl로 진입
    }

    pub fn _adjust_fdiv(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        // 이것은 전역 변수: __adjust_fdiv는 FDIV 버그 플래그
        // 주소 반환 (0 값의 글로벌 변수 주소)
        let addr = uc.malloc(4);
        uc.write_u32(addr, 0);
        println!("[MSVCRT] _adjust_fdiv -> {:#x}", addr);
        Some((0, Some(addr as i32)))
    }

    pub fn _purecall(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] _purecall()");
        Some((0, None))
    }

    pub fn _errno(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        // _errno returns a pointer to thread-local errno
        let addr = uc.malloc(4);
        uc.write_u32(addr, 0);
        println!("[MSVCRT] _errno() -> {:#x}", addr);
        Some((0, Some(addr as i32)))
    }

    pub fn qsort(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] qsort(...)");
        Some((4, None)) // cdecl
    }

    // C++ exception related
    pub fn exception_ref(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] exception::exception(const&)");
        Some((1, Some(0))) // thiscall/cdecl hybrid
    }

    pub fn exception_ptr(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[MSVCRT] exception::exception(const char*)");
        Some((1, Some(0)))
    }

    // =========================================================
    // Dispatch
    // =========================================================
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        DllMSVCRT::wrap_result(
            func_name,
            match func_name {
                "localtime" => DllMSVCRT::localtime(uc),
                "strncmp" => DllMSVCRT::strncmp(uc),
                "strcoll" => DllMSVCRT::strcoll(uc),
                "strncpy" => DllMSVCRT::strncpy(uc),
                "isspace" => DllMSVCRT::isspace(uc),
                "isdigit" => DllMSVCRT::isdigit(uc),
                "_vsnprintf" => DllMSVCRT::_vsnprintf(uc),
                "_exit" => DllMSVCRT::_exit(uc),
                "_beginthreadex" => DllMSVCRT::_beginthreadex(uc),
                "_endthreadex" => DllMSVCRT::_endthreadex(uc),
                "sprintf" => DllMSVCRT::sprintf(uc),
                "?_set_se_translator@@YAP6AXIPAU_EXCEPTION_POINTERS@@@ZP6AXI0@Z@Z" => {
                    DllMSVCRT::_set_se_translator(uc)
                }
                "??2@YAPAXI@Z" => DllMSVCRT::new_op(uc),
                "_except_handler3" => DllMSVCRT::_except_handler3(uc),
                "memmove" => DllMSVCRT::memmove(uc),
                "memchr" => DllMSVCRT::memchr(uc),
                "__CxxFrameHandler" => DllMSVCRT::___cxx_frame_handler(uc),
                "_read" => DllMSVCRT::_read(uc),
                "mktime" => DllMSVCRT::mktime(uc),
                "atoi" => DllMSVCRT::atoi(uc),
                "free" => DllMSVCRT::free(uc),
                "__dllonexit" => DllMSVCRT::__dllonexit(uc),
                "_onexit" => DllMSVCRT::_onexit(uc),
                "?terminate@@YAXXZ" => DllMSVCRT::terminate(uc),
                "??1type_info@@UAE@XZ" => DllMSVCRT::type_info(uc),
                "_initterm" => DllMSVCRT::_initterm(uc),
                "malloc" => DllMSVCRT::malloc(uc),
                "_adjust_fdiv" => DllMSVCRT::_adjust_fdiv(uc),
                "_write" => DllMSVCRT::_write(uc),
                "_close" => DllMSVCRT::_close(uc),
                "_lseek" => DllMSVCRT::_lseek(uc),
                "_open" => DllMSVCRT::_open(uc),
                "_pipe" => DllMSVCRT::_pipe(uc),
                "time" => DllMSVCRT::time(uc),
                "_setjmp3" => DllMSVCRT::_setjmp3(uc),
                "fopen" => DllMSVCRT::fopen(uc),
                "fwrite" => DllMSVCRT::fwrite(uc),
                "fclose" => DllMSVCRT::fclose(uc),
                "longjmp" => DllMSVCRT::longjmp(uc),
                "strstr" => DllMSVCRT::strstr(uc),
                "fread" => DllMSVCRT::fread(uc),
                "_ftol" => DllMSVCRT::_ftol(uc),
                "fseek" => DllMSVCRT::fseek(uc),
                "ftell" => DllMSVCRT::ftell(uc),
                "_stricmp" => DllMSVCRT::_stricmp(uc),
                "fflush" => DllMSVCRT::fflush(uc),
                "sscanf" => DllMSVCRT::sscanf(uc),
                "getenv" => DllMSVCRT::getenv(uc),
                "_strcmpi" => DllMSVCRT::_strcmpi(uc),
                "vsprintf" => DllMSVCRT::vsprintf(uc),
                "calloc" => DllMSVCRT::calloc(uc),
                "floor" => DllMSVCRT::floor(uc),
                "realloc" => DllMSVCRT::realloc(uc),
                "qsort" => DllMSVCRT::qsort(uc),
                "frexp" => DllMSVCRT::frexp(uc),
                "_CIpow" => DllMSVCRT::__c_ipow(uc),
                "ldexp" => DllMSVCRT::ldexp(uc),
                "_errno" => DllMSVCRT::_errno(uc),
                "_purecall" => DllMSVCRT::_purecall(uc),
                "_beginthread" => DllMSVCRT::_beginthread(uc),
                "ceil" => DllMSVCRT::ceil(uc),
                "isalnum" => DllMSVCRT::isalnum(uc),
                "fprintf" => DllMSVCRT::fprintf(uc),
                "_strnicmp" => DllMSVCRT::_strnicmp(uc),
                "rand" => DllMSVCRT::rand(uc),
                "_itoa" => DllMSVCRT::_itoa(uc),
                "strrchr" => DllMSVCRT::strrchr(uc),
                "??0exception@@QAE@ABV0@@Z" => DllMSVCRT::exception_ref(uc),
                "??0exception@@QAE@ABQBD@Z" => DllMSVCRT::exception_ptr(uc),
                "srand" => DllMSVCRT::srand(uc),
                "fscanf" => DllMSVCRT::fscanf(uc),
                "strtok" => DllMSVCRT::strtok(uc),
                "strtoul" => DllMSVCRT::strtoul(uc),
                "_timezone" => DllMSVCRT::_timezone(uc),
                "_stat" => DllMSVCRT::_stat(uc),
                "_CxxThrowException" => DllMSVCRT::_cxx_throw_exception(uc),
                _ => {
                    println!("[MSVCRT] UNHANDLED: {}", func_name);
                    None
                }
            },
        )
    }

    pub fn _cxx_throw_exception(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let p_exception_object = uc.read_arg(0);
        let p_throw_info = uc.read_arg(1);
        println!(
            "[MSVCRT] _CxxThrowException(pExceptionObject={:#x}, pThrowInfo={:#x})",
            p_exception_object, p_throw_info
        );

        let _ = uc.emu_stop(); // 에뮬레이션 정지
        Some((2, None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::win32::StackCleanup;

    #[test]
    fn sprintf_uses_caller_cleanup() {
        let result = DllMSVCRT::wrap_result("sprintf", Some((3, Some(5)))).unwrap();
        assert_eq!(result.cleanup, StackCleanup::Caller);
    }

    #[test]
    fn cxx_throw_exception_keeps_callee_cleanup() {
        let result = DllMSVCRT::wrap_result("_CxxThrowException", Some((2, None))).unwrap();
        assert_eq!(result.cleanup, StackCleanup::Callee(2));
    }
}
