use unicorn_engine::RegisterX86;
use unicorn_engine::Unicorn;

use crate::helper::UnicornHelper;
use crate::win32::{ApiHookResult, Win32Context, callee_result, caller_result};
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::atomic::Ordering;

/// `MSVCRT.dll` 프록시 구현 모듈
///
/// C 런타임 라이브러리(CRT) 함수를 우회/구현하며 메모리 할당(malloc), 문자열 포맷팅, 예외 처리 등을 담당
pub struct DllMSVCRT;

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
    // API: void* malloc(size_t size)
    // 역할: 지정된 데이터만큼 메모리를 할당
    pub fn malloc(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let size = uc.read_arg(0);
        let addr = uc.malloc(size as usize);
        crate::emu_log!("[MSVCRT] malloc({}) -> {:#x}", size, addr);
        Some((1, Some(addr as i32)))
    }

    // API: void free(void* ptr)
    // 역할: 할당된 메모리를 해제
    pub fn free(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        // 간이 힙이므로 free는 아무 작업도 수행하지 않음
        crate::emu_log!("[MSVCRT] free(...)");
        Some((1, None))
    }

    // API: void* calloc(size_t num, size_t size)
    // 역할: 메모리를 할당하고 0으로 초기화
    pub fn calloc(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let num = uc.read_arg(0);
        let size = uc.read_arg(1);
        let total = (num * size) as usize;
        let addr = uc.malloc(total);
        let zeros = vec![0u8; total];
        uc.mem_write(addr, &zeros).unwrap();
        crate::emu_log!("[MSVCRT] calloc({}, {}) -> {:#x}", num, size, addr);
        Some((2, Some(addr as i32)))
    }

    // API: void* realloc(void* ptr, size_t size)
    // 역할: 이미 할당된 메모리의 크기를 조정
    pub fn realloc(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let _ptr = uc.read_arg(0);
        let size = uc.read_arg(1);
        let addr = uc.malloc(size as usize);
        // 간이 구현: 이전 데이터의 복사는 생략
        crate::emu_log!("[MSVCRT] realloc({:#x}, {}) -> {:#x}", _ptr, size, addr);
        Some((2, Some(addr as i32)))
    }

    pub fn new_op(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let size = uc.read_arg(0);
        let addr = uc.malloc(size as usize);
        crate::emu_log!("[MSVCRT] operator new({}) -> {:#x}", size, addr);
        Some((1, Some(addr as i32)))
    }

    // API: void* memmove(void* dest, const void* src, size_t count)
    // 역할: 메모리 블록을 다른 위치로 복사 (겹침 허용)
    pub fn memmove(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let dst = uc.read_arg(0);
        let src = uc.read_arg(1);
        let size = uc.read_arg(2) as usize;
        if size > 0 {
            let data = uc.mem_read_as_vec(src as u64, size).unwrap_or_default();
            uc.mem_write(dst as u64, &data).unwrap();
        }
        crate::emu_log!("[MSVCRT] memmove({:#x}, {:#x}, {})", dst, src, size);
        Some((3, Some(dst as i32)))
    }

    // API: void* memchr(const void* ptr, int ch, size_t count)
    // 역할: 메모리에서 특정 문자를 검색
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
        crate::emu_log!(
            "[MSVCRT] memchr({:#x}, {}, {}) -> {:#x}",
            buf,
            ch,
            count,
            result
        );
        Some((3, Some(result as i32)))
    }

    // =========================================================
    // String
    // =========================================================
    // API: int strncmp(const char* str1, const char* str2, size_t count)
    // 역할: 두 문자열을 지정된 길이만큼 비교
    pub fn strncmp(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let s1_addr = uc.read_arg(0);
        let s2_addr = uc.read_arg(1);
        let n = uc.read_arg(2) as usize;
        let s1 = uc.read_euc_kr(s1_addr as u64);
        let s2 = uc.read_euc_kr(s2_addr as u64);
        let r1: Vec<u8> = s1.bytes().take(n).collect();
        let r2: Vec<u8> = s2.bytes().take(n).collect();
        let result = r1.cmp(&r2) as i32;
        crate::emu_log!(
            "[MSVCRT] strncmp(\"{}\", \"{}\", {}) -> {}",
            s1,
            s2,
            n,
            result
        );
        Some((3, Some(result)))
    }

    // API: int strcoll(const char* str1, const char* str2)
    // 역할: 현재 로캘을 사용하여 두 문자열을 비교
    pub fn strcoll(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let s1_addr = uc.read_arg(0);
        let s2_addr = uc.read_arg(1);
        let s1 = uc.read_euc_kr(s1_addr as u64);
        let s2 = uc.read_euc_kr(s2_addr as u64);
        let result = s1.cmp(&s2) as i32;
        crate::emu_log!("[MSVCRT] strcoll(\"{}\", \"{}\") -> {}", s1, s2, result);
        Some((2, Some(result)))
    }

    // API: char* strncpy(char* dest, const char* src, size_t count)
    // 역할: 문자열을 지정된 길이만큼 복사
    pub fn strncpy(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let dst = uc.read_arg(0);
        let src = uc.read_arg(1);
        let n = uc.read_arg(2) as usize;
        let s = uc.read_euc_kr(src as u64);
        let mut bytes: Vec<u8> = s.bytes().take(n).collect();
        while bytes.len() < n {
            bytes.push(0);
        }
        uc.mem_write(dst as u64, &bytes).unwrap();
        crate::emu_log!("[MSVCRT] strncpy({:#x}, \"{}\", {})", dst, s, n);
        Some((3, Some(dst as i32)))
    }

    // API: char* strstr(const char* str, const char* substr)
    // 역할: 문자열 내에서 부분 문자열을 검색
    pub fn strstr(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let s1_addr = uc.read_arg(0);
        let s2_addr = uc.read_arg(1);
        let s1 = uc.read_euc_kr(s1_addr as u64);
        let s2 = uc.read_euc_kr(s2_addr as u64);
        let result = s1.find(&s2).map(|pos| s1_addr + pos as u32).unwrap_or(0);
        crate::emu_log!("[MSVCRT] strstr(\"{}\", \"{}\") -> {:#x}", s1, s2, result);
        Some((2, Some(result as i32)))
    }

    // API: char* strrchr(const char* str, int ch)
    // 역할: 문자열에서 특정 문자가 마지막으로 나타나는 위치를 검색
    pub fn strrchr(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let s_addr = uc.read_arg(0);
        let ch = uc.read_arg(1) as u8 as char;
        let s = uc.read_euc_kr(s_addr as u64);
        let result = s.rfind(ch).map(|pos| s_addr + pos as u32).unwrap_or(0);
        crate::emu_log!("[MSVCRT] strrchr(\"{}\", '{}') -> {:#x}", s, ch, result);
        Some((2, Some(result as i32)))
    }

    pub fn strtok(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[MSVCRT] strtok(...)");
        Some((2, Some(0))) // 간이: NULL 반환
    }

    // API: unsigned long strtoul(const char* str, char** endptr, int base)
    // 역할: 문자열을 무부호 장정수로 변환
    pub fn strtoul(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let s_addr = uc.read_arg(0);
        let _endptr = uc.read_arg(1);
        let base = uc.read_arg(2);
        let s = uc.read_euc_kr(s_addr as u64);
        let result = u32::from_str_radix(s.trim(), base).unwrap_or(0);
        crate::emu_log!("[MSVCRT] strtoul(\"{}\", ..., {}) -> {}", s, base, result);
        Some((3, Some(result as i32)))
    }

    // API: int _stricmp(const char* str1, const char* str2)
    // 역할: 대소문자 구분 없이 두 문자열을 비교
    pub fn _stricmp(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let s1_addr = uc.read_arg(0);
        let s2_addr = uc.read_arg(1);
        let s1 = uc.read_euc_kr(s1_addr as u64).to_lowercase();
        let s2 = uc.read_euc_kr(s2_addr as u64).to_lowercase();
        let result = s1.cmp(&s2) as i32;
        crate::emu_log!("[MSVCRT] _stricmp(\"{}\", \"{}\") -> {}", s1, s2, result);
        Some((2, Some(result)))
    }

    pub fn _strcmpi(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        DllMSVCRT::_stricmp(uc)
    }

    // API: int _strnicmp(const char* str1, const char* str2, size_t count)
    // 역할: 대소문자 구분 없이 지정된 길이만큼 두 문자열을 비교
    pub fn _strnicmp(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let s1_addr = uc.read_arg(0);
        let s2_addr = uc.read_arg(1);
        let n = uc.read_arg(2) as usize;
        let s1: String = uc
            .read_euc_kr(s1_addr as u64)
            .chars()
            .take(n)
            .collect::<String>()
            .to_lowercase();
        let s2: String = uc
            .read_euc_kr(s2_addr as u64)
            .chars()
            .take(n)
            .collect::<String>()
            .to_lowercase();
        let result = s1.cmp(&s2) as i32;
        crate::emu_log!(
            "[MSVCRT] _strnicmp(\"{}\", \"{}\", {}) -> {}",
            s1,
            s2,
            n,
            result
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
    // API: int atoi(const char* str)
    // 역할: 문자열을 정수로 변환
    pub fn atoi(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let s_addr = uc.read_arg(0);
        let s = uc.read_euc_kr(s_addr as u64);
        let result = s.trim().parse::<i32>().unwrap_or(0);
        crate::emu_log!("[MSVCRT] atoi(\"{}\") -> {}", s, result);
        Some((1, Some(result)))
    }

    // API: char* _itoa(int value, char* str, int radix)
    // 역할: 정수를 문자열로 변환
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
        crate::emu_log!(
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
    // API: time_t time(time_t* timer)
    // 역할: 시스템의 현재 시간을 가져옴
    pub fn time(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let timer_addr = uc.read_arg(0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as u32;
        if timer_addr != 0 {
            uc.write_u32(timer_addr as u64, now);
        }
        crate::emu_log!("[MSVCRT] time(...) -> {}", now);
        Some((1, Some(now as i32)))
    }

    // API: struct tm* localtime(const time_t* timer)
    // 역할: 시간을 현지 시간 구조체로 변환
    pub fn localtime(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[MSVCRT] localtime(...)");
        Some((1, Some(0))) // NULL (간이)
    }

    // API: time_t mktime(struct tm* timeptr)
    // 역할: tm 구조체를 time_t 값으로 변환
    pub fn mktime(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[MSVCRT] mktime(...)");
        Some((1, Some(0)))
    }

    pub fn _timezone(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[MSVCRT] _timezone");
        Some((0, Some(0)))
    }

    // =========================================================
    // Math
    // =========================================================
    // API: double floor(double x)
    // 역할: 지정된 값보다 작거나 같은 최대 정수를 계산
    pub fn floor(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[MSVCRT] floor(...)");
        Some((0, Some(0))) // FPU 기반이라 스택 인자 아님
    }

    // API: double ceil(double x)
    // 역할: 지정된 값보다 크거나 같은 최소 정수를 계산
    pub fn ceil(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[MSVCRT] ceil(...)");
        Some((0, Some(0)))
    }

    pub fn frexp(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[MSVCRT] frexp(...)");
        Some((0, Some(0)))
    }

    pub fn ldexp(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[MSVCRT] ldexp(...)");
        Some((0, Some(0)))
    }

    pub fn __c_ipow(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[MSVCRT] _CIpow(...)");
        Some((0, Some(0)))
    }

    pub fn _ftol(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[MSVCRT] _ftol(...)");
        Some((0, Some(0)))
    }

    // =========================================================
    // Random
    // =========================================================
    // API: int rand(void)
    // 역할: 의사 난수를 생성
    pub fn rand(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let ctx = uc.get_data();
        let mut state = ctx.rand_state.load(Ordering::SeqCst);
        state = state.wrapping_mul(214013).wrapping_add(2531011);
        ctx.rand_state.store(state, Ordering::SeqCst);
        let val = (state >> 16) & 0x7FFF;
        Some((0, Some(val as i32)))
    }

    // API: void srand(unsigned int seed)
    // 역할: 난수 생성기의 시드를 설정
    pub fn srand(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let seed = uc.read_arg(0);
        uc.get_data().rand_state.store(seed, Ordering::SeqCst);
        crate::emu_log!("[MSVCRT] srand({})", seed);
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
                        for c in chars.by_ref() {
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
            crate::emu_log!(
                "[MSVCRT] sprintf invalid args: buf={:#x}, fmt={:#x}, eip={:#x}, esp={:#x}",
                buf_addr,
                fmt_addr,
                eip,
                esp
            );
        }
        let (result, total_args) = DllMSVCRT::format_string(uc, fmt_addr, 2);
        let bytes = result.as_bytes();
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
        Some((total_args, Some(bytes.len() as i32))) // cdecl, 가변 인자 포함
    }

    // API: int _vsnprintf(char* str, size_t count, const char* format, va_list argptr)
    // 역할: va_list를 사용하여 서식화된 데이터를 문자열 버퍼에 출력
    pub fn _vsnprintf(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let buf_addr = uc.read_arg(0);
        let _size = uc.read_arg(1);
        let fmt_addr = uc.read_arg(2);
        // va_list ptr at arg(3) - 에뮬레이터에서는 스택 기반이므로 직접 파싱 불가
        // 간략 구현: 포맷 문자열만 복사
        let fmt = uc.read_euc_kr(fmt_addr as u64);
        let mut buf = fmt.as_bytes().to_vec();
        buf.push(0);
        uc.mem_write(buf_addr as u64, &buf).unwrap();
        crate::emu_log!(
            "[MSVCRT] _vsnprintf({:#x}, {}, ...) -> \"{}\" ",
            buf_addr,
            _size,
            fmt
        );
        Some((3, Some(fmt.len() as i32)))
    }

    // API: int vsprintf(char* str, const char* format, va_list argptr)
    // 역할: va_list를 사용하여 서식화된 데이터를 문자열로 출력
    pub fn vsprintf(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let buf_addr = uc.read_arg(0);
        let fmt_addr = uc.read_arg(1);
        let fmt = uc.read_euc_kr(fmt_addr as u64);
        let mut buf = fmt.as_bytes().to_vec();
        buf.push(0);
        uc.mem_write(buf_addr as u64, &buf).unwrap();
        crate::emu_log!("[MSVCRT] vsprintf({:#x}, ...) -> \"{}\"", buf_addr, fmt);
        Some((3, Some(fmt.len() as i32)))
    }

    // API: int sscanf(const char* str, const char* format, ...)
    // 역할: 문자열에서 서식화된 데이터를 읽음
    pub fn sscanf(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let _buf_addr = uc.read_arg(0);
        let fmt_addr = uc.read_arg(1);
        let fmt = uc.read_euc_kr(fmt_addr as u64);
        let vararg_count = DllMSVCRT::count_scanf_varargs(&fmt);
        let total_args = 2 + vararg_count; // buf + fmt + varargs
        crate::emu_log!("[MSVCRT] sscanf(..., \"{}\") -> args={}", fmt, total_args);
        Some((total_args, Some(0)))
    }

    // API: int fprintf(FILE* stream, const char* format, ...)
    // 역할: 스트림에 서식화된 데이터를 출력
    pub fn fprintf(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let _stream = uc.read_arg(0);
        let fmt_addr = uc.read_arg(1);
        let fmt = uc.read_euc_kr(fmt_addr as u64);
        let vararg_count = DllMSVCRT::count_printf_varargs(&fmt);
        let total_args = 2 + vararg_count; // stream + fmt + varargs
        crate::emu_log!("[MSVCRT] fprintf(..., \"{}\") -> args={}", fmt, total_args);
        Some((total_args, Some(0)))
    }

    // API: int fscanf(FILE* stream, const char* format, ...)
    // 역할: 스트림에서 서식화된 데이터를 읽음
    pub fn fscanf(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let _stream = uc.read_arg(0);
        let fmt_addr = uc.read_arg(1);
        let fmt = uc.read_euc_kr(fmt_addr as u64);
        let vararg_count = DllMSVCRT::count_scanf_varargs(&fmt);
        let total_args = 2 + vararg_count; // stream + fmt + varargs
        crate::emu_log!("[MSVCRT] fscanf(..., \"{}\") -> args={}", fmt, total_args);
        Some((total_args, Some(0)))
    }

    // =========================================================
    // File I/O
    // =========================================================
    // API: FILE* fopen(const char* filename, const char* mode)
    // 역할: 파일을 오픈
    pub fn fopen(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let filename_addr = uc.read_arg(0);
        let mode_addr = uc.read_arg(1);
        let filename = uc.read_euc_kr(filename_addr as u64);
        let mode = uc.read_euc_kr(mode_addr as u64);

        let mut options = std::fs::OpenOptions::new();
        // Parse mode: r, w, a, +, b, t

        for c in mode.chars() {
            match c {
                'r' => {
                    options.read(true);
                }
                'w' => {
                    options.write(true).create(true).truncate(true);
                }
                'a' => {
                    options.append(true).create(true);
                }
                '+' => {
                    options.read(true).write(true);
                }
                _ => {}
            }
        }

        let mut file_result = options.open(&filename);
        if file_result.is_err() && !filename.contains('/') && !filename.contains('\\') {
            let alt_path = format!("Resources/{}", filename);
            file_result = options.open(&alt_path);
        }

        match file_result {
            Ok(file) => {
                let context = uc.get_data();
                let handle = context.alloc_handle();
                context.files.lock().unwrap().insert(handle, file);
                crate::emu_log!(
                    "[MSVCRT] fopen(\"{}\", \"{}\") -> handle {:#x}",
                    filename,
                    mode,
                    handle
                );
                Some((2, Some(handle as i32)))
            }
            Err(e) => {
                crate::emu_log!(
                    "[MSVCRT] fopen(\"{}\", \"{}\") failed: {:?}",
                    filename,
                    mode,
                    e
                );
                Some((2, Some(0)))
            }
        }
    }

    // API: int fclose(FILE* stream)
    // 역할: 파일을 닫음
    pub fn fclose(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let stream_handle = uc.read_arg(0);
        let context = uc.get_data();
        let mut files = context.files.lock().unwrap();
        if files.remove(&{ stream_handle }).is_some() {
            crate::emu_log!("[MSVCRT] fclose(handle {:#x})", stream_handle);
            Some((1, Some(0)))
        } else {
            crate::emu_log!("[MSVCRT] fclose(handle {:#x}) failed", stream_handle);
            Some((1, Some(-1))) // EOF
        }
    }

    // API: size_t fread(void* buffer, size_t size, size_t count, FILE* stream)
    // 역할: 스트림에서 데이터를 읽음
    pub fn fread(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let buffer_addr = uc.read_arg(0);
        let size = uc.read_arg(1);
        let count = uc.read_arg(2);
        let stream_handle = uc.read_arg(3);
        let total_size = (size * count) as usize;

        if total_size == 0 {
            return Some((4, Some(0)));
        }

        let mut data = vec![0u8; total_size];
        let bytes_read = {
            let context = uc.get_data();
            let mut files = context.files.lock().unwrap();
            if let Some(file) = files.get_mut(&{ stream_handle }) {
                file.read(&mut data).unwrap_or(0)
            } else {
                0
            }
        };

        if bytes_read > 0 {
            uc.mem_write(buffer_addr as u64, &data[..bytes_read])
                .unwrap();
        }

        let actual_count = (bytes_read as u32 / size) as i32;
        crate::emu_log!(
            "[MSVCRT] fread(handle {:#x}, size={}, count={}) -> {}",
            stream_handle,
            size,
            count,
            actual_count
        );
        Some((4, Some(actual_count)))
    }

    // API: size_t fwrite(const void* buffer, size_t size, size_t count, FILE* stream)
    // 역할: 스트림에 데이터를 씀
    pub fn fwrite(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let buffer_addr = uc.read_arg(0);
        let size = uc.read_arg(1);
        let count = uc.read_arg(2);
        let stream_handle = uc.read_arg(3);
        let total_size = (size * count) as usize;

        if total_size == 0 {
            return Some((4, Some(0)));
        }

        let data = uc.mem_read_as_vec(buffer_addr as u64, total_size).unwrap();
        let bytes_written = {
            let context = uc.get_data();
            let mut files = context.files.lock().unwrap();
            if let Some(file) = files.get_mut(&{ stream_handle }) {
                file.write(&data).unwrap_or(0)
            } else {
                0
            }
        };

        let actual_count = (bytes_written as u32 / size) as i32;
        crate::emu_log!(
            "[MSVCRT] fwrite(handle {:#x}, size={}, count={}) -> {}",
            stream_handle,
            size,
            count,
            actual_count
        );
        Some((4, Some(actual_count)))
    }

    // API: int fseek(FILE* stream, long offset, int origin)
    // 역할: 파일 포인터를 특정 위치로 이동
    pub fn fseek(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let stream_handle = uc.read_arg(0);
        let offset = uc.read_arg(1) as i32 as i64; // Sign-extend long
        let origin = uc.read_arg(2); // 0=SEEK_SET, 1=SEEK_CUR, 2=SEEK_END

        let pos = match origin {
            0 => SeekFrom::Start(offset as u64),
            1 => SeekFrom::Current(offset),
            2 => SeekFrom::End(offset),
            _ => return Some((3, Some(-1))),
        };

        let context = uc.get_data();
        let mut files = context.files.lock().unwrap();
        if let Some(file) = files.get_mut(&{ stream_handle }) {
            match file.seek(pos) {
                Ok(new_pos) => {
                    crate::emu_log!(
                        "[MSVCRT] fseek(handle {:#x}, offset={}, origin={}) -> pos {}",
                        stream_handle,
                        offset,
                        origin,
                        new_pos
                    );
                    Some((3, Some(0)))
                }
                Err(e) => {
                    crate::emu_log!(
                        "[MSVCRT] fseek(handle {:#x}) failed: {:?}",
                        stream_handle,
                        e
                    );
                    Some((3, Some(-1)))
                }
            }
        } else {
            crate::emu_log!(
                "[MSVCRT] fseek(handle {:#x}) - handle not found",
                stream_handle
            );
            Some((3, Some(-1)))
        }
    }

    // API: long ftell(FILE* stream)
    // 역할: 현재 파일 포인터의 위치를 가져옴
    pub fn ftell(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let stream_handle = uc.read_arg(0);
        let context = uc.get_data();
        let mut files = context.files.lock().unwrap();
        if let Some(file) = files.get_mut(&{ stream_handle }) {
            match file.stream_position() {
                Ok(pos) => {
                    crate::emu_log!("[MSVCRT] ftell(handle {:#x}) -> {}", stream_handle, pos);
                    Some((1, Some(pos as i32)))
                }
                Err(_) => Some((1, Some(-1))),
            }
        } else {
            crate::emu_log!(
                "[MSVCRT] ftell(handle {:#x}) - handle not found",
                stream_handle
            );
            Some((1, Some(-1)))
        }
    }

    // API: int fflush(FILE* stream)
    // 역할: 스트림의 버퍼를 플러시(비움)
    pub fn fflush(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let stream_handle = uc.read_arg(0);
        let context = uc.get_data();
        let mut files = context.files.lock().unwrap();
        if let Some(file) = files.get_mut(&{ stream_handle }) {
            file.flush().unwrap();
            Some((1, Some(0)))
        } else {
            Some((1, Some(-1)))
        }
    }

    // Low-level I/O
    // API: int _open(const char* filename, int oflag, ...)
    // 역할: 저수준 파일 기술자를 오픈
    pub fn _open(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let filename_addr = uc.read_arg(0);
        let oflag = uc.read_arg(1);
        let filename = uc.read_euc_kr(filename_addr as u64);

        let mut options = std::fs::OpenOptions::new();
        // oflag (from fcntl.h/io.h): O_RDONLY=0, O_WRONLY=1, O_RDWR=2, O_APPEND=8, O_CREAT=0x100, O_TRUNC=0x200
        if oflag & 0x1 != 0 {
            options.write(true);
        } else if oflag & 0x2 != 0 {
            options.read(true).write(true);
        } else {
            options.read(true);
        }

        if oflag & 0x0008 != 0 {
            options.append(true);
        }
        if oflag & 0x0100 != 0 {
            options.create(true);
        }
        if oflag & 0x0200 != 0 {
            options.truncate(true);
        }

        let mut file_result = options.open(&filename);
        if file_result.is_err() && !filename.contains('/') && !filename.contains('\\') {
            let alt_path = format!("Resources/{}", filename);
            file_result = options.open(&alt_path);
        }

        match file_result {
            Ok(file) => {
                let context = uc.get_data();
                let handle = context.alloc_handle();
                context.files.lock().unwrap().insert(handle, file);
                crate::emu_log!(
                    "[MSVCRT] _open(\"{}\", {:#x}) -> fd {:#x}",
                    filename,
                    oflag,
                    handle
                );
                Some((3, Some(handle as i32))) // cdecl, may have pmode
            }
            Err(e) => {
                crate::emu_log!(
                    "[MSVCRT] _open(\"{}\", {:#x}) failed: {:?}",
                    filename,
                    oflag,
                    e
                );
                Some((3, Some(-1)))
            }
        }
    }

    // API: int _close(int fd)
    // 역할: 파일 기술자를 닫음
    pub fn _close(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let fd = uc.read_arg(0);
        let context = uc.get_data();
        if context.files.lock().unwrap().remove(&fd).is_some() {
            crate::emu_log!("[MSVCRT] _close(fd {:#x})", fd);
            Some((1, Some(0)))
        } else {
            crate::emu_log!("[MSVCRT] _close(fd {:#x}) failed", fd);
            Some((1, Some(-1)))
        }
    }

    // API: int _read(int fd, void* buffer, unsigned int count)
    // 역할: 파일 기술자에서 데이터를 읽음
    pub fn _read(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let fd = uc.read_arg(0);
        let buffer_addr = uc.read_arg(1);
        let count = uc.read_arg(2);

        let mut data = vec![0u8; count as usize];
        let bytes_read = {
            let context = uc.get_data();
            let mut files = context.files.lock().unwrap();
            if let Some(file) = files.get_mut(&fd) {
                file.read(&mut data).unwrap_or(0)
            } else {
                0
            }
        };

        if bytes_read > 0 {
            uc.mem_write(buffer_addr as u64, &data[..bytes_read])
                .unwrap();
        }

        crate::emu_log!(
            "[MSVCRT] _read(fd {:#x}, count={}) -> bytes {}",
            fd,
            count,
            bytes_read
        );
        Some((3, Some(bytes_read as i32)))
    }

    // API: int _write(int fd, const void* buffer, unsigned int count)
    // 역할: 파일 기술자에 데이터를 씀
    pub fn _write(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let fd = uc.read_arg(0);
        let buffer_addr = uc.read_arg(1);
        let count = uc.read_arg(2);

        let data = uc
            .mem_read_as_vec(buffer_addr as u64, count as usize)
            .unwrap();
        let bytes_written = {
            let context = uc.get_data();
            let mut files = context.files.lock().unwrap();
            if let Some(file) = files.get_mut(&fd) {
                file.write(&data).unwrap_or(0)
            } else {
                0
            }
        };

        crate::emu_log!(
            "[MSVCRT] _write(fd {:#x}, count={}) -> bytes {}",
            fd,
            count,
            bytes_written
        );
        Some((3, Some(bytes_written as i32)))
    }

    // API: long _lseek(int fd, long offset, int origin)
    // 역할: 파일 기술자의 읽기/쓰기 위치를 이동
    pub fn _lseek(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let fd = uc.read_arg(0);
        let offset = uc.read_arg(1) as i32 as i64;
        let origin = uc.read_arg(2);

        let pos = match origin {
            0 => SeekFrom::Start(offset as u64),
            1 => SeekFrom::Current(offset),
            2 => SeekFrom::End(offset),
            _ => return Some((3, Some(-1))),
        };

        let context = uc.get_data();
        let mut files = context.files.lock().unwrap();
        if let Some(file) = files.get_mut(&fd) {
            match file.seek(pos) {
                Ok(new_pos) => {
                    crate::emu_log!(
                        "[MSVCRT] _lseek(fd {:#x}, offset={}, origin={}) -> pos {}",
                        fd,
                        offset,
                        origin,
                        new_pos
                    );
                    Some((3, Some(new_pos as i32)))
                }
                Err(_) => Some((3, Some(-1))),
            }
        } else {
            crate::emu_log!("[MSVCRT] _lseek(fd {:#x}) - fd not found", fd);
            Some((3, Some(-1)))
        }
    }

    // API: int _pipe(int* phandles, unsigned int size, int oflag)
    // 역할: 익명 파이프를 생성
    pub fn _pipe(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[MSVCRT] _pipe(...)");
        Some((3, Some(-1))) // cdecl
    }

    // API: int _stat(const char* filename, struct _stat* buffer)
    // 역할: 파일의 상태 정보를 가져옴
    pub fn _stat(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let filename_addr = uc.read_arg(0);
        let buffer_addr = uc.read_arg(1);
        let filename = uc.read_euc_kr(filename_addr as u64);

        if let Ok(metadata) = std::fs::metadata(&filename) {
            let mut stat_buf = vec![0u8; 64];
            let size = metadata.len() as u32;
            let mode = if metadata.is_dir() { 0x4000 } else { 0x8000 } | 0o666;

            // Simplified VC6 _stat layout
            stat_buf[4..6].copy_from_slice(&(mode as u16).to_le_bytes());
            stat_buf[14..18].copy_from_slice(&size.to_le_bytes());

            uc.mem_write(buffer_addr as u64, &stat_buf).unwrap();
            Some((2, Some(0)))
        } else {
            Some((2, Some(-1)))
        }
    }

    // =========================================================
    // Environment
    // =========================================================
    // API: char* getenv(const char* varname)
    // 역할: 특정 환경 변수의 값을 가져옴
    pub fn getenv(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[MSVCRT] getenv(...)");
        Some((1, Some(0))) // cdecl, NULL
    }

    // =========================================================
    // Thread
    // =========================================================
    // API: uintptr_t _beginthreadex(void* security, unsigned stack_size, unsigned (*start_address)(void*), void* arglist, unsigned initflag, unsigned* thrdaddr)
    // 역할: 새 스레드를 생성 (Win32 API 기반 확장 버전)
    pub fn _beginthreadex(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let _security = uc.read_arg(0);
        let stack_size = uc.read_arg(1);
        let start_address = uc.read_arg(2) as u64;
        let arglist = uc.read_arg(3);
        let _init_flag = uc.read_arg(4);
        let _thread_addr_ptr = uc.read_arg(5);

        let ctx = uc.get_data();
        let handle = ctx.alloc_handle();

        crate::emu_log!(
            "[MSVCRT] _beginthreadex({:#x}, {}, {:#x}) -> handle {:#x}",
            start_address,
            stack_size,
            arglist,
            handle
        );

        std::thread::spawn(move || {
            crate::emu_log!("[MSVCRT] Thread ex {:#x} started on host", start_address);
            // TODO: Create new engine and run guest code
        });

        Some((6, Some(handle as i32)))
    }

    // API: void _endthreadex(unsigned retval)
    // 역할: _beginthreadex로 생성된 스레드를 종료
    pub fn _endthreadex(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let exit_code = uc.read_arg(0);
        crate::emu_log!("[MSVCRT] _endthreadex({})", exit_code);
        Some((1, None)) // cdecl, void
    }

    // API: uintptr_t _beginthread(void (*start_address)(void*), unsigned stack_size, void* arglist)
    // 역할: 새 스레드를 생성
    pub fn _beginthread(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let start_address = uc.read_arg(0) as u64;
        let stack_size = uc.read_arg(1);
        let arglist = uc.read_arg(2);

        let ctx = uc.get_data();
        let handle = ctx.alloc_handle();

        crate::emu_log!(
            "[MSVCRT] _beginthread({:#x}, {}, {:#x}) -> handle {:#x}",
            start_address,
            stack_size,
            arglist,
            handle
        );

        // Native Thread Spawn
        std::thread::spawn(move || {
            crate::emu_log!("[MSVCRT] Thread {:#x} started on host", start_address);
            // TODO: Create new engine and run guest code
        });

        Some((3, Some(handle as i32)))
    }

    // =========================================================
    // Exception / SEH
    // =========================================================
    // API: void __stdcall _CxxThrowException(void* pExceptionObject, _ThrowInfo* pThrowInfo)
    // 역할: C++ 예외를 발생시킴
    pub fn __cxx_throw_exception(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[MSVCRT] _CxxThrowException(...)");
        Some((2, None)) // stdcall 2 args
    }

    // API: int _except_handler3(...)
    // 역할: 내부 예외 처리기 (SEH)
    pub fn _except_handler3(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[MSVCRT] _except_handler3(...)");
        Some((4, Some(1))) // cdecl
    }

    // API: int __CxxFrameHandler(...)
    // 역할: C++ 프레임 처리기
    pub fn ___cxx_frame_handler(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[MSVCRT] __CxxFrameHandler(...)");
        Some((4, Some(0))) // cdecl
    }

    // API: _se_translator_function _set_se_translator(_se_translator_function se_trans_func)
    // 역할: Win32 예외를 C++ 예외로 변환하는 함수를 설정
    pub fn _set_se_translator(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let se_translator_function = uc.read_arg(0);
        crate::emu_log!("[MSVCRT] _set_se_translator({:#x})", se_translator_function);
        Some((1, Some(0))) // cdecl
    }

    // API: int _setjmp3(jmp_buf env, int count)
    // 역할: 비로컬 jump를 위한 현재 상태를 저장
    pub fn _setjmp3(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[MSVCRT] _setjmp3(...)");
        Some((2, Some(0))) // cdecl, 바로 리턴
    }

    // API: void longjmp(jmp_buf env, int value)
    // 역할: setjmp로 저장된 위치로 제어를 이동
    pub fn longjmp(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[MSVCRT] longjmp(...)");
        Some((2, None)) // cdecl
    }

    // =========================================================
    // Init / Exit
    // =========================================================
    // API: void _initterm(_PVFV* begin, _PVFV* end)
    // 역할: 함수 포인터 테이블을 순회하며 초기화 함수들을 호출
    pub fn _initterm(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        // _initterm(begin, end) - 함수 포인터 테이블을 순회하며 호출
        let begin = uc.read_arg(0) as u64;
        let end = uc.read_arg(1) as u64;
        crate::emu_log!("[MSVCRT] _initterm({:#x}, {:#x})", begin, end);

        let mut addr = begin;
        while addr < end {
            let func_ptr = uc.read_u32(addr);
            if func_ptr != 0 {
                // crate::emu_log!("[MSVCRT] _initterm: calling {:#x}", func_ptr);

                // 콜백 호출 (void __cdecl func(void))
                // 리턴 주소를 스택에 push하고 emu_start로 콜백 실행
                let esp = uc.reg_read(unicorn_engine::RegisterX86::ESP).unwrap();
                uc.reg_write(unicorn_engine::RegisterX86::ESP, esp - 4)
                    .unwrap();
                uc.write_u32(esp - 4, 0); // return addr = 0 → EXIT_ADDRESS처럼 동작

                if let Err(e) = uc.emu_start(func_ptr as u64, 0, 0, 10000) {
                    crate::emu_log!(
                        "[MSVCRT] _initterm: callback at {:#x} failed: {:?}",
                        func_ptr,
                        e
                    );
                }

                // 스택 복원 (콜백이 스택을 건드렸을 수 있으므로 원래 ESP로 복구)
                uc.reg_write(unicorn_engine::RegisterX86::ESP, esp).unwrap();
            }
            addr += 4;
        }
        // crate::emu_log!("[MSVCRT] _initterm done");
        Some((2, None)) // cdecl
    }

    // API: void _exit(int status)
    // 역할: 프로세스를 즉시 종료
    pub fn _exit(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[MSVCRT] _exit(...)");
        Some((1, None)) // cdecl
    }

    // API: _onexit_t __dllonexit(_onexit_t func, _PVFV** begin, _PVFV** end)
    // 역할: DLL 종료 시 호출될 함수를 등록
    pub fn __dllonexit(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[MSVCRT] __dllonexit(...)");
        Some((3, Some(0))) // cdecl
    }

    // API: _onexit_t _onexit(_onexit_t func)
    // 역할: 프로그램 종료 시 호출될 함수를 등록
    pub fn _onexit(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[MSVCRT] _onexit(...)");
        Some((1, Some(0))) // cdecl
    }

    pub fn terminate(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[MSVCRT] terminate()");
        Some((0, None))
    }

    pub fn type_info(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[MSVCRT] type_info::~type_info()");
        Some((0, None)) // thiscall 가능하지만 cdecl로 진입
    }

    pub fn _adjust_fdiv(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        // 이것은 전역 변수: __adjust_fdiv는 FDIV 버그 플래그
        // 주소 반환 (0 값의 글로벌 변수 주소)
        let addr = uc.malloc(4);
        uc.write_u32(addr, 0);
        crate::emu_log!("[MSVCRT] _adjust_fdiv -> {:#x}", addr);
        Some((0, Some(addr as i32)))
    }

    // API: void _purecall(void)
    // 역할: 순수 가상 함수 호출 시의 에러 처리기
    pub fn _purecall(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[MSVCRT] _purecall()");
        Some((0, None))
    }

    // API: int* _errno(void)
    // 역할: 현재 스레드의 오류 번호(errno) 포인터를 가져옴
    pub fn _errno(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        // _errno returns a pointer to thread-local errno
        let addr = uc.malloc(4);
        uc.write_u32(addr, 0);
        crate::emu_log!("[MSVCRT] _errno() -> {:#x}", addr);
        Some((0, Some(addr as i32)))
    }

    // API: void qsort(void* base, size_t num, size_t width, int (*compare)(const void*, const void*))
    // 역할: 퀵 정렬 알고리즘을 수행
    pub fn qsort(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[MSVCRT] qsort(...)");
        Some((4, None)) // cdecl
    }

    // C++ exception related
    pub fn exception_ref(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[MSVCRT] exception::exception(const&)");
        Some((1, Some(0))) // thiscall/cdecl hybrid
    }

    pub fn exception_ptr(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[MSVCRT] exception::exception(const char*)");
        Some((1, Some(0)))
    }

    // API: void _CxxThrowException(void* pExceptionObject, _ThrowInfo* pThrowInfo)
    // 역할: C++ 예외를 발생시킴
    pub fn _cxx_throw_exception(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let p_exception_object = uc.read_arg(0);
        let p_throw_info = uc.read_arg(1);
        crate::emu_log!(
            "[MSVCRT] _CxxThrowException(pExceptionObject={:#x}, pThrowInfo={:#x})",
            p_exception_object,
            p_throw_info
        );

        let _ = uc.emu_stop(); // 에뮬레이션 정지
        Some((2, None))
    }

    // =========================================================
    // MSVCRT handle logic
    // =========================================================

    /// 함수명 기준 `MSVCRT.dll` API 구현체입니다. 처리를 성공했다면 스택 보정값과 리턴값을 포함한 `ApiHookResult`를 반환합니다.
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
                    crate::emu_log!("[MSVCRT] UNHANDLED: {}", func_name);
                    None
                }
            },
        )
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
