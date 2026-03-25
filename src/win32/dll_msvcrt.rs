use unicorn_engine::Unicorn;

use crate::helper::{EXIT_ADDRESS, UnicornHelper};
use crate::win32::{ApiHookResult, Win32Context, callee_result, caller_result};
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::atomic::Ordering;

/// `MSVCRT.dll` 프록시 구현 모듈
///
/// C 런타임 라이브러리(CRT) 함수를 우회/구현하며 메모리 할당(malloc), 문자열 포맷팅, 예외 처리 등을 담당
pub struct DllMSVCRT;

impl DllMSVCRT {
    fn wrap_result(func_name: &str, result: Option<(usize, Option<i32>)>) -> Option<ApiHookResult> {
        // MSVCRT.dll은 대부분 cdecl 이지만 C++ 예외/기능 일부는 thiscall/stdcall 임
        let is_thiscall = func_name.contains("@QAE") || func_name.contains("@IAE");
        let is_stdcall = func_name.contains("@YG") || func_name == "_CxxThrowException";

        if is_thiscall || is_stdcall {
            callee_result(result)
        } else {
            caller_result(result)
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
        crate::emu_log!("[MSVCRT] malloc({}) -> void* {:#x}", size, addr);
        Some((1, Some(addr as i32)))
    }

    // API: void free(void* ptr)
    // 역할: 할당된 메모리를 해제
    pub fn free(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        // 간이 힙이므로 free는 아무 작업도 수행하지 않음
        let ptr = uc.read_arg(0);
        crate::emu_log!("[MSVCRT] free({:#x}) -> void", ptr);
        Some((1, None))
    }

    // API: void* calloc(size_t num, size_t size)
    // 역할: 메모리를 할당하고 0으로 초기화
    pub fn calloc(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let num = uc.read_arg(0);
        let size = uc.read_arg(1);
        let total = (num * size) as usize;
        let addr = uc.malloc(total);
        if total > 0 {
            let zeros = vec![0u8; total];
            uc.mem_write(addr, &zeros).unwrap();
        }
        crate::emu_log!("[MSVCRT] calloc({}, {}) -> void* {:#x}", num, size, addr);
        Some((2, Some(addr as i32)))
    }

    // API: void* realloc(void* ptr, size_t size)
    // 역할: 이미 할당된 메모리의 크기를 조정
    pub fn realloc(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let ptr = uc.read_arg(0);
        let size = uc.read_arg(1) as usize;
        if size == 0 {
            crate::emu_log!("[MSVCRT] realloc({:#x}, 0) -> NULL", ptr);
            return Some((2, Some(0)));
        }
        let addr = uc.malloc(size);
        if ptr != 0 {
            // We don't know the exact original size, so we copy up to 'size' bytes.
            // This is a limitation of our simple monotonic heap.
            let data = uc.mem_read_as_vec(ptr as u64, size).unwrap_or_default();
            uc.mem_write(addr, &data).unwrap();
        }
        crate::emu_log!(
            "[MSVCRT] realloc({:#x}, {}) -> void* {:#x}",
            ptr,
            size,
            addr
        );
        Some((2, Some(addr as i32)))
    }

    pub fn new_op(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let size = uc.read_arg(0);
        let addr = uc.malloc(size as usize);
        crate::emu_log!("[MSVCRT] operator new({}) -> void* {:#x}", size, addr);
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
        crate::emu_log!(
            "[MSVCRT] memmove({:#x}, {:#x}, {}) -> void* {:#x}",
            dst,
            src,
            size,
            dst
        );
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
            "[MSVCRT] memchr({:#x}, {}, {}) -> void* {:#x}",
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
        let n = uc.read_arg(2) as usize;
        let r1: Vec<u8> = s1.bytes().take(n).collect();
        let r2: Vec<u8> = s2.bytes().take(n).collect();
        let result = r1.cmp(&r2) as i32;
        crate::emu_log!(
            "[MSVCRT] strncmp(\"{}\", \"{}\", {}) -> int {}",
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
        Some((2, Some(result)))
    }

    // API: char* strncpy(char* dest, const char* src, size_t count)
    // 역할: 문자열을 지정된 길이만큼 복사
    pub fn strncpy(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let dst = uc.read_arg(0);
        let src = uc.read_arg(1);
        let s = if src != 0 {
            uc.read_euc_kr(src as u64)
        } else {
            String::new()
        };
        let n = uc.read_arg(2) as usize;
        let mut bytes: Vec<u8> = s.bytes().take(n).collect();
        while bytes.len() < n {
            bytes.push(0);
        }
        uc.mem_write(dst as u64, &bytes).unwrap();
        crate::emu_log!(
            "[MSVCRT] strncpy({:#x}, \"{}\", {}) -> char* {:#x}",
            dst,
            s,
            n,
            dst
        );
        Some((3, Some(dst as i32)))
    }

    // API: char* strstr(const char* str, const char* substr)
    // 역할: 문자열 내에서 부분 문자열을 검색
    pub fn strstr(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((2, Some(result as i32)))
    }

    // API: char* strrchr(const char* str, int ch)
    // 역할: 문자열에서 특정 문자가 마지막으로 나타나는 위치를 검색
    pub fn strrchr(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((2, Some(result as i32)))
    }

    // API: char* strtok(char* str, const char* sep)
    // 역할: 문자열을 구분자로 분리
    pub fn strtok(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let s_addr = uc.read_arg(0);
        let s = if s_addr != 0 {
            uc.read_euc_kr(s_addr as u64)
        } else {
            String::new()
        };
        let sep = uc.read_arg(1);
        crate::emu_log!(
            "[MSVCRT] strtok(\"{}\", '{}') -> void* {:#x}",
            s,
            sep as u8 as char,
            s_addr
        );
        Some((2, Some(s_addr as i32)))
    }

    // API: unsigned long strtoul(const char* str, char** endptr, int base)
    // 역할: 문자열을 무부호 장정수로 변환
    pub fn strtoul(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let s_addr = uc.read_arg(0);
        let s = if s_addr != 0 {
            uc.read_euc_kr(s_addr as u64)
        } else {
            String::new()
        };
        let endptr = uc.read_arg(1);
        let base = uc.read_arg(2);
        let result = u32::from_str_radix(s.trim(), base).unwrap_or(0);
        crate::emu_log!(
            "[MSVCRT] strtoul(\"{}\", {}, {}) -> unsigned long {}",
            s,
            endptr,
            base,
            result
        );
        Some((3, Some(result as i32)))
    }

    // API: int _stricmp(const char* str1, const char* str2)
    // 역할: 대소문자 구분 없이 두 문자열을 비교
    pub fn _stricmp(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
            "[MSVCRT] _strnicmp(\"{}\", \"{}\", {}) -> int {}",
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
        crate::emu_log!("[MSVCRT] isspace({}) -> int {}", ch, result);
        Some((1, Some(result)))
    }

    pub fn isdigit(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let ch = uc.read_arg(0) as u8;
        let result = if (ch as char).is_ascii_digit() { 1 } else { 0 };
        crate::emu_log!("[MSVCRT] isdigit({}) -> int {}", ch, result);
        Some((1, Some(result)))
    }

    pub fn isalnum(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let ch = uc.read_arg(0) as u8;
        let result = if (ch as char).is_ascii_alphanumeric() {
            1
        } else {
            0
        };
        crate::emu_log!("[MSVCRT] isalnum({}) -> int {}", ch, result);
        Some((1, Some(result)))
    }

    // =========================================================
    // Conversion
    // =========================================================
    // API: int atoi(const char* str)
    // 역할: 문자열을 정수로 변환
    pub fn atoi(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let s_addr = uc.read_arg(0);
        let s = if s_addr != 0 {
            uc.read_euc_kr(s_addr as u64)
        } else {
            String::new()
        };
        let result = s.trim().parse::<i32>().unwrap_or(0);
        crate::emu_log!("[MSVCRT] atoi(\"{}\") -> int {}", s, result);
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
            "[MSVCRT] _itoa({}, {:#x}, {}) -> char* {:#x}=\"{}\"",
            value,
            buf_addr,
            radix,
            buf_addr,
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
        crate::emu_log!("[MSVCRT] time({:#x}) -> time_t {:#x}", timer_addr, now);
        Some((1, Some(now as i32)))
    }

    // API: struct tm* localtime(const time_t* timer)
    // 역할: 시간을 현지 시간 구조체로 변환
    pub fn localtime(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let timer_ptr = uc.read_arg(0);
        let time_val = if timer_ptr != 0 {
            uc.read_u32(timer_ptr as u64)
        } else {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as u32
        };

        // Simple unix time to tm conversion (approximate for dummy)
        let sec = (time_val % 60) as i32;
        let min = ((time_val / 60) % 60) as i32;
        let hour = ((time_val / 3600) % 24) as i32;
        let day = ((time_val / 86400) % 31) as i32 + 1;
        let mon = ((time_val / 2592000) % 12) as i32;
        let year = (time_val / 31536000) as i32 + 70;

        let mut tm_ptr = uc.get_data().tm_struct_ptr.load(Ordering::SeqCst);
        if tm_ptr == 0 {
            tm_ptr = uc.malloc(36) as u32;
            uc.get_data().tm_struct_ptr.store(tm_ptr, Ordering::SeqCst);
        }

        let mut data = [0u32; 9];
        data[0] = sec as u32; // tm_sec
        data[1] = min as u32; // tm_min
        data[2] = hour as u32; // tm_hour
        data[3] = day as u32; // tm_mday
        data[4] = mon as u32; // tm_mon
        data[5] = year as u32; // tm_year
        data[6] = 0; // tm_wday
        data[7] = 0; // tm_yday
        data[8] = 0; // tm_isdst

        for i in 0..9 {
            uc.write_u32((tm_ptr + (i * 4) as u32) as u64, data[i]);
        }

        crate::emu_log!(
            "[MSVCRT] localtime({:#x}:{}) -> struct tm* {:#x}",
            timer_ptr,
            time_val,
            tm_ptr
        );
        Some((1, Some(tm_ptr as i32)))
    }

    // API: time_t mktime(struct tm* timeptr)
    // 역할: tm 구조체를 time_t 값으로 변환
    pub fn mktime(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let timeptr_addr = uc.read_arg(0);
        if timeptr_addr == 0 {
            return Some((1, Some(-1)));
        }

        let sec = uc.read_u32(timeptr_addr as u64);
        let min = uc.read_u32(timeptr_addr as u64 + 4);
        let hour = uc.read_u32(timeptr_addr as u64 + 8);
        let day = uc.read_u32(timeptr_addr as u64 + 12);
        let mon = uc.read_u32(timeptr_addr as u64 + 16);
        let year = uc.read_u32(timeptr_addr as u64 + 20);

        // Very crude conversion
        let t = sec
            + min * 60
            + hour * 3600
            + (day - 1) * 86400
            + mon * 2592000
            + (year - 70) * 31536000;

        crate::emu_log!("[MSVCRT] mktime({:#x}) -> time_t {:#x}", timeptr_addr, t);
        Some((1, Some(t as i32)))
    }

    // =========================================================
    // Math
    // =========================================================
    // API: double floor(double x)
    // 역할: 지정된 값보다 작거나 같은 최대 정수를 계산
    pub fn floor(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let x_low = uc.read_arg(0);
        let x_high = uc.read_arg(1);
        let x = f64::from_bits((x_low as u64) | ((x_high as u64) << 32));
        let res = x.floor();
        crate::emu_log!("[MSVCRT] floor({}) -> double {}", x, res);
        // double return usually in ST(0) or EAX:EDX. Here we return 0 for EAX and let ST(0) be handled if needed.
        Some((2, Some(0)))
    }

    // API: double ceil(double x)
    // 역할: 지정된 값보다 크거나 같은 최소 정수를 계산
    pub fn ceil(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let x_low = uc.read_arg(0);
        let x_high = uc.read_arg(1);
        let x = f64::from_bits((x_low as u64) | ((x_high as u64) << 32));
        let res = x.ceil();
        crate::emu_log!("[MSVCRT] ceil({}) -> double {}", x, res);
        Some((2, Some(0)))
    }

    pub fn frexp(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let x_low = uc.read_arg(0);
        let x_high = uc.read_arg(1);
        let exp_ptr = uc.read_arg(2);
        let x = f64::from_bits((x_low as u64) | ((x_high as u64) << 32));
        // Simple dummy: x = m * 2^e
        uc.write_u32(exp_ptr as u64, 0);
        crate::emu_log!("[MSVCRT] frexp({}, {:#x}) -> double {}", x, exp_ptr, x);
        Some((3, Some(0)))
    }

    pub fn ldexp(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let x_low = uc.read_arg(0);
        let x_high = uc.read_arg(1);
        let exp = uc.read_arg(2) as i32;
        let x = f64::from_bits((x_low as u64) | ((x_high as u64) << 32));
        let res = x * 2.0f64.powi(exp);
        crate::emu_log!("[MSVCRT] ldexp({}, {}) -> double {}", x, exp, res);
        Some((3, Some(0)))
    }

    pub fn _ftol(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        // _ftol: converts ST(0) to integer in EAX
        crate::emu_log!("[MSVCRT] _ftol() -> stub 0");
        Some((0, Some(0)))
    }

    pub fn __c_ipow(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        // _CIpow usually takes args from FPU stack ST(0), ST(1)
        crate::emu_log!("[MSVCRT] _CIpow() -> double st(0)^st(1) stub 0");
        Some((0, Some(0)))
    }

    // =========================================================
    // Math

    pub fn _timezone(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let addr = uc.malloc(4);
        uc.write_u32(addr, 32400); // UTC+9
        crate::emu_log!("[MSVCRT] _timezone -> {:#x} (32400)", addr);
        Some((0, Some(addr as i32)))
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
        crate::emu_log!("[MSVCRT] rand() -> int {}", val);
        Some((0, Some(val as i32)))
    }

    // API: void srand(unsigned int seed)
    // 역할: 난수 생성기의 시드를 설정
    pub fn srand(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let seed = uc.read_arg(0);
        uc.get_data().rand_state.store(seed, Ordering::SeqCst);
        crate::emu_log!("[MSVCRT] srand({:#x}) -> void", seed);
        Some((1, None))
    }

    // =========================================================
    // =========================================================
    // Format / IO
    // =========================================================

    /// 포맷 문자열 파싱 및 에뮬레이트 메모리 기반 sprintf 구현
    /// 포맷 문자열을 파싱하여 가변 인자 개수를 카운팅하는 헬퍼 (printf 계열)
    /// 반환: 포맷 스펙이 소비하는 스택 슬롯 수 (double은 2 슬롯)

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
        let buf_addr = uc.read_arg(0);
        let fmt_addr = uc.read_arg(1);
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
        let size = uc.read_arg(1);
        let fmt_addr = uc.read_arg(2);
        let fmt = uc.read_euc_kr(fmt_addr as u64);
        let (result, total_args) = DllMSVCRT::format_string(uc, fmt_addr, 3);
        let bytes = result.as_bytes();
        let mut buf = bytes.to_vec();
        buf.push(0);
        uc.mem_write(buf_addr as u64, &buf).unwrap();
        crate::emu_log!(
            "[MSVCRT] _vsnprintf({:#x}, {}, \"{}\", ...) -> \"{}\"",
            buf_addr,
            size,
            fmt,
            result
        );
        Some((total_args, Some(bytes.len() as i32)))
    }

    // API: int vsprintf(char* str, const char* format, va_list argptr)
    // 역할: va_list를 사용하여 서식화된 데이터를 문자열로 출력
    pub fn vsprintf(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let buf_addr = uc.read_arg(0);
        let fmt_addr = uc.read_arg(1);
        let (result, total_args) = DllMSVCRT::format_string(uc, fmt_addr, 2);
        let bytes = result.as_bytes();
        let mut buf = bytes.to_vec();
        buf.push(0);
        uc.mem_write(buf_addr as u64, &buf).unwrap();
        crate::emu_log!("[MSVCRT] vsprintf({:#x}, ...) -> \"{}\"", buf_addr, result);
        Some((total_args, Some(bytes.len() as i32)))
    }

    // API: int sscanf(const char* str, const char* format, ...)
    // 역할: 문자열에서 서식화된 데이터를 읽음
    pub fn sscanf(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
                            let end = input_ptr.find(|c: char| !c.is_numeric() && c != '-').unwrap_or(input_ptr.len());
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
                            let end = input_ptr.find(|c: char| !c.is_ascii_hexdigit()).unwrap_or(input_ptr.len());
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
                           let end = input_ptr.find(|c: char| c.is_whitespace()).unwrap_or(input_ptr.len());
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
        Some((arg_idx, Some(count as i32)))
    }

    // API: int fprintf(FILE* stream, const char* format, ...)
    // 역할: 스트림에 서식화된 데이터를 출력
    pub fn fprintf(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let stream_handle = uc.read_arg(0);
        let fmt_addr = uc.read_arg(1);
        let (result, total_args) = DllMSVCRT::format_string(uc, fmt_addr, 2);

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
            Some((total_args, Some(bytes.len() as i32)))
        } else {
            crate::emu_log!(
                "[MSVCRT] fprintf({:#x}, ...) - handle not found",
                stream_handle
            );
            Some((total_args, Some(-1)))
        }
    }

    // API: int fscanf(FILE* stream, const char* format, ...)
    // 역할: 스트림에서 서식화된 데이터를 읽음
    pub fn fscanf(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
            return Some((2, Some(-1))); // EOF
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
                            let end = input_ptr.find(|c: char| !c.is_numeric() && c != '-').unwrap_or(input_ptr.len());
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
                            let end = input_ptr.find(|c: char| !c.is_ascii_hexdigit()).unwrap_or(input_ptr.len());
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
                           let end = input_ptr.find(|c: char| c.is_whitespace()).unwrap_or(input_ptr.len());
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
        Some((arg_idx, Some(count as i32)))
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
                    "[MSVCRT] fopen(\"{}\", \"{}\") -> FILE* {:#x}",
                    filename,
                    mode,
                    handle
                );
                Some((2, Some(handle as i32)))
            }
            Err(e) => {
                crate::emu_log!(
                    "[MSVCRT] fopen(\"{}\", \"{}\") -> FILE* 0 {:?}",
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
            crate::emu_log!("[MSVCRT] fclose({:#x}) -> int 0", stream_handle);
            Some((1, Some(0)))
        } else {
            crate::emu_log!("[MSVCRT] fclose({:#x}) -> int -1", stream_handle);
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
            "[MSVCRT] fread({:#x}, {:#x}, {:#x}, {:#x}) -> size_t {:#x}",
            stream_handle,
            size,
            count,
            buffer_addr,
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
            "[MSVCRT] fwrite({:#x}, {:#x}, {:#x}, {:#x}) -> size_t {:#x}",
            stream_handle,
            size,
            count,
            buffer_addr,
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
                        "[MSVCRT] fseek({:#x}, {:#x}, {:#x}) -> int {:#x}",
                        stream_handle,
                        offset,
                        origin,
                        new_pos
                    );
                    Some((3, Some(0)))
                }
                Err(e) => {
                    crate::emu_log!(
                        "[MSVCRT] fseek({:#x}, {:#x}, {:#x}) -> int -1 {:?}",
                        stream_handle,
                        offset,
                        origin,
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
                    crate::emu_log!("[MSVCRT] ftell({:#x}) -> long {:#x}", stream_handle, pos);
                    Some((1, Some(pos as i32)))
                }
                Err(_) => Some((1, Some(-1))),
            }
        } else {
            crate::emu_log!(
                "[MSVCRT] ftell({:#x}) -> long -1 (handle not found)",
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
            crate::emu_log!("[MSVCRT] fflush({:#x}) -> int 0", stream_handle);
            Some((1, Some(0)))
        } else {
            crate::emu_log!("[MSVCRT] fflush({:#x}) -> int -1", stream_handle);
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
                    "[MSVCRT] _open(\"{}\", {:#x}) -> int {:#x}",
                    filename,
                    oflag,
                    handle
                );
                Some((3, Some(handle as i32))) // cdecl, may have pmode
            }
            Err(e) => {
                crate::emu_log!(
                    "[MSVCRT] _open(\"{}\", {:#x}) -> int -1: {:?}",
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
            crate::emu_log!("[MSVCRT] _close(fd {:#x}) -> int 0", fd);
            Some((1, Some(0)))
        } else {
            crate::emu_log!("[MSVCRT] _close(fd {:#x}) -> int -1", fd);
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
            "[MSVCRT] _read({:#x}, {:#x}, {}) -> int {}",
            fd,
            buffer_addr,
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
            "[MSVCRT] _write({:#x}, {:#x}, {}) -> int {}",
            fd,
            buffer_addr,
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
                        "[MSVCRT] _lseek({:#x}, {}, {}) -> int {:#x}",
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
            crate::emu_log!(
                "[MSVCRT] _lseek({:#x}, {}, {}) -> int -1 (fd not found)",
                fd,
                offset,
                origin
            );
            Some((3, Some(-1)))
        }
    }

    // API: int _pipe(int* phandles, unsigned int size, int oflag)
    // 역할: 익명 파이프를 생성
    pub fn _pipe(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let phandles = uc.read_arg(0);
        let size = uc.read_arg(1);
        let oflag = uc.read_arg(2);
        crate::emu_log!(
            "[MSVCRT] _pipe({:#x}, {}, {}) -> int -1",
            phandles,
            size,
            oflag
        );
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
            crate::emu_log!(
                "[MSVCRT] _stat(\"{}\", {:#x}) -> int 0",
                filename,
                buffer_addr
            );
            Some((2, Some(0)))
        } else {
            crate::emu_log!(
                "[MSVCRT] _stat(\"{}\", {:#x}) -> int -1",
                filename,
                buffer_addr
            );
            Some((2, Some(-1)))
        }
    }

    // =========================================================
    // Environment
    // =========================================================
    // API: char* getenv(const char* varname)
    // 역할: 특정 환경 변수의 값을 가져옴
    pub fn getenv(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let varname_addr = uc.read_arg(0);
        let varname = uc.read_euc_kr(varname_addr as u64);
        crate::emu_log!("[MSVCRT] getenv(\"{}\") -> char* 0x0", varname);
        Some((1, Some(0))) // cdecl, NULL
    }

    // =========================================================
    // Thread
    // =========================================================
    // API: uintptr_t _beginthreadex(void* security, unsigned stack_size, unsigned (*start_address)(void*), void* arglist, unsigned initflag, unsigned* thrdaddr)
    // 역할: 새 스레드를 생성 (Win32 API 기반 확장 버전)
    // 주의: 실제 멀티스레드 에뮬은 불가능. 대신 스레드 콜백이 설정할 "ready" 플래그를 직접 세팅하여
    //       TNetDrv::Main() 등이 실행된 것처럼 시뮬레이트.
    pub fn _beginthreadex(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let _security = uc.read_arg(0);
        let stack_size = uc.read_arg(1);
        let start_address = uc.read_arg(2) as u64;
        let arglist = uc.read_arg(3);
        let _init_flag = uc.read_arg(4);
        let thread_addr_ptr = uc.read_arg(5);

        let ctx = uc.get_data();
        let handle = ctx.alloc_handle();

        crate::emu_log!(
            "[MSVCRT] _beginthreadex({:#x}, {}, {:#x}, {:#x}, {}, {:#x}) -> uintptr_t {:#x}",
            _security,
            stack_size,
            start_address,
            arglist,
            _init_flag,
            thread_addr_ptr,
            handle
        );

        // thrdaddr에 가짜 스레드 ID 기록
        if thread_addr_ptr != 0 {
            uc.write_u32(thread_addr_ptr as u64, handle);
        }

        // TThread/TNetDrv 패턴 시뮬레이션:
        // 스레드 콜백이 arglist(=this) 객체의 "ready" 플래그를 세팅하는 것을 직접 수행.
        // TNetDrv::Main()은 this+41 바이트를 1로 세팅하고 네트워크 루프를 도는 구조이므로,
        // 실제 스레드 실행 없이 플래그만 직접 세팅하여 Connect()의 대기 루프 탈출을 유도.
        if arglist != 0 {
            crate::emu_log!(
                "[MSVCRT] _beginthreadex: setting ready flag at {:#x}+41 = 1 (thread sim)",
                arglist
            );
            uc.mem_write(arglist as u64 + 41, &[1u8]).ok();
        }

        Some((6, Some(handle as i32)))
    }

    // API: void _endthreadex(unsigned retval)
    // 역할: _beginthreadex로 생성된 스레드를 종료
    pub fn _endthreadex(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let exit_code = uc.read_arg(0);
        crate::emu_log!("[MSVCRT] _endthreadex({}) -> void", exit_code);
        Some((1, None)) // cdecl, void
    }

    // API: uintptr_t _beginthread(void (*start_address)(void*), unsigned stack_size, void* arglist)
    // 역할: 새 스레드를 생성
    // 주의: _beginthreadex와 동일한 "ready flag" 시뮬레이션 적용.
    pub fn _beginthread(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let start_address = uc.read_arg(0) as u64;
        let stack_size = uc.read_arg(1);
        let arglist = uc.read_arg(2);

        let ctx = uc.get_data();
        let handle = ctx.alloc_handle();

        crate::emu_log!(
            "[MSVCRT] _beginthread({:#x}, {}, {:#x}) -> uintptr_t {:#x}",
            start_address,
            stack_size,
            arglist,
            handle
        );

        // TThread/TNetDrv 패턴: 스레드 콜백이 arglist(=this)+41에 ready=1을 세팅하는 것을 직접 수행
        if arglist != 0 {
            crate::emu_log!(
                "[MSVCRT] _beginthread: setting ready flag at {:#x}+41 = 1 (thread sim)",
                arglist
            );
            uc.mem_write(arglist as u64 + 41, &[1u8]).ok();
        }

        Some((3, Some(handle as i32)))
    }

    // =========================================================
    // Exception / SEH
    // =========================================================
    // API: void __stdcall _CxxThrowException(void* pExceptionObject, _ThrowInfo* pThrowInfo)
    // 역할: C++ 예외를 발생시킴
    pub fn __cxx_throw_exception(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let p_exception_object = uc.read_arg(0);
        let p_throw_info = uc.read_arg(1);
        crate::emu_log!(
            "[MSVCRT] _CxxThrowException({:#x}, {:#x}) -> void",
            p_exception_object,
            p_throw_info
        );
        Some((2, None)) // stdcall 2 args
    }

    // API: int _except_handler3(...)
    // 역할: 내부 예외 처리기 (SEH)
    pub fn _except_handler3(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let p_exception_record = _uc.read_arg(0);
        let p_establisher_frame = _uc.read_arg(1);
        let p_context = _uc.read_arg(2);
        let p_dispatcher_context = _uc.read_arg(3);
        crate::emu_log!(
            "[MSVCRT] _except_handler3({:#x}, {:#x}, {:#x}, {:#x}) -> int 1",
            p_exception_record,
            p_establisher_frame,
            p_context,
            p_dispatcher_context
        );
        Some((4, Some(1))) // cdecl
    }

    // API: int __CxxFrameHandler(...)
    // 역할: C++ 프레임 처리기
    pub fn ___cxx_frame_handler(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let p_exception_record = _uc.read_arg(0);
        let p_establisher_frame = _uc.read_arg(1);
        let p_context = _uc.read_arg(2);
        let p_dispatcher_context = _uc.read_arg(3);
        crate::emu_log!(
            "[MSVCRT] __CxxFrameHandler({:#x}, {:#x}, {:#x}, {:#x}) -> int 0",
            p_exception_record,
            p_establisher_frame,
            p_context,
            p_dispatcher_context
        );
        Some((4, Some(0))) // cdecl
    }

    // API: _se_translator_function _set_se_translator(_se_translator_function se_trans_func)
    // 역할: Win32 예외를 C++ 예외로 변환하는 함수를 설정
    pub fn _set_se_translator(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let se_translator_function = uc.read_arg(0);
        crate::emu_log!(
            "[MSVCRT] _set_se_translator({:#x}) -> _se_translator_function 0",
            se_translator_function
        );
        Some((1, Some(0))) // cdecl
    }

    // API: int _setjmp3(jmp_buf env, int count)
    // 역할: 비로컬 jump를 위한 현재 상태를 저장
    pub fn _setjmp3(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let env = uc.read_arg(0);
        let count = uc.read_arg(1);
        crate::emu_log!("[MSVCRT] _setjmp3({:#x}, {:#x}) -> int 0", env, count);
        Some((2, Some(0))) // cdecl, 바로 리턴
    }

    // API: void longjmp(jmp_buf env, int value)
    // 역할: setjmp로 저장된 위치로 제어를 이동
    pub fn longjmp(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let env = uc.read_arg(0);
        let value = uc.read_arg(1);
        crate::emu_log!("[MSVCRT] longjmp({:#x}, {:#x}) -> void", env, value);
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
        crate::emu_log!("[MSVCRT] _initterm({:#x}, {:#x}) -> void", begin, end);
        Some((2, None)) // cdecl
    }

    // API: void _exit(int status)
    // 역할: 프로세스를 즉시 종료
    pub fn _exit(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let status = uc.read_arg(0);
        crate::emu_log!("[MSVCRT] _exit({:#x}) -> void", status);
        let _ = uc.emu_stop();
        Some((1, None)) // cdecl
    }

    // API: _onexit_t __dllonexit(_onexit_t func, _PVFV** begin, _PVFV** end)
    // 역할: DLL 종료 시 호출될 함수를 등록
    pub fn __dllonexit(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let func = uc.read_arg(0);
        let _begin = uc.read_arg(1);
        let _end = uc.read_arg(2);
        
        let ctx = uc.get_data();
        let mut handlers = ctx.onexit_handlers.lock().unwrap();
        handlers.push(func);
        
        crate::emu_log!(
            "[MSVCRT] __dllonexit({:#x}, {:#x}, {:#x}) -> _onexit_t {:#x}",
            func,
            _begin,
            _end,
            func
        );
        Some((3, Some(func as i32))) // cdecl, returns the function pointer on success
    }

    // API: _onexit_t _onexit(_onexit_t func)
    // 역할: 프로그램 종료 시 호출될 함수를 등록
    pub fn _onexit(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let func = uc.read_arg(0);
        let ctx = uc.get_data();
        let mut handlers = ctx.onexit_handlers.lock().unwrap();
        handlers.push(func);
        
        crate::emu_log!("[MSVCRT] _onexit({:#x}) -> _onexit_t {:#x}", func, func);
        Some((1, Some(func as i32))) // cdecl
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
        crate::emu_log!("[MSVCRT] _purecall() -> void");
        Some((0, None))
    }

    // API: int* _errno(void)
    // 역할: 현재 스레드의 오류 번호(errno) 포인터를 가져옴
    pub fn _errno(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        // _errno returns a pointer to thread-local errno
        let addr = uc.malloc(4);
        uc.write_u32(addr, 0);
        crate::emu_log!("[MSVCRT] _errno() -> int* {:#x}", addr);
        Some((0, Some(addr as i32)))
    }

    // API: void qsort(void* base, size_t num, size_t width, int (*compare)(const void*, const void*))
    // 역할: 퀵 정렬 알고리즘을 수행
    pub fn qsort(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let base = uc.read_arg(0);
        let num = uc.read_arg(1) as usize;
        let width = uc.read_arg(2) as usize;
        let compare_addr = uc.read_arg(3) as u64;

        if num <= 1 || width == 0 || compare_addr == 0 {
            return Some((4, None));
        }

        let data = uc.mem_read_as_vec(base as u64, num * width).unwrap();
        let mut indices: Vec<usize> = (0..num).collect();

        // Use a simple sort for now to avoid re-entrancy issues if any
        // but try to use callback
        indices.sort_by(|&a, &b| {
            let ptr_a = base + (a * width) as u32;
            let ptr_b = base + (b * width) as u32;

            // Setup stack for comparison: push ptr_b, push ptr_a, push exit_addr
            let esp = uc.reg_read(unicorn_engine::RegisterX86::ESP).unwrap();
            uc.push_u32(ptr_b);
            uc.push_u32(ptr_a);
            uc.push_u32(EXIT_ADDRESS as u32);

            // Run comparison function
            if let Err(e) = uc.emu_start(compare_addr, EXIT_ADDRESS, 0, 0) {
                crate::emu_log!("[MSVCRT] qsort callback error: {:?}", e);
                return std::cmp::Ordering::Equal;
            }

            let res = uc.reg_read(unicorn_engine::RegisterX86::EAX).unwrap() as i32;

            // Restore stack
            uc.reg_write(unicorn_engine::RegisterX86::ESP, esp).unwrap();

            if res < 0 {
                std::cmp::Ordering::Less
            } else if res > 0 {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        });

        // Reorder data based on indices
        let mut sorted_data = Vec::with_capacity(num * width);
        for idx in indices {
            sorted_data.extend_from_slice(&data[idx * width..(idx + 1) * width]);
        }

        uc.mem_write(base as u64, &sorted_data).unwrap();

        crate::emu_log!(
            "[MSVCRT] qsort({:#x}, {}, {}, {:#x}) -> void (sorted)",
            base,
            num,
            width,
            compare_addr
        );
        Some((4, None))
    }

    // C++ exception related
    pub fn exception_ref(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
        let other_ptr = uc.read_arg(0);
        crate::emu_log!(
            "[MSVCRT] (this={:#x}) exception::exception({:#x}) -> (this={:#x})",
            this_ptr,
            other_ptr,
            this_ptr
        );
        Some((1, Some(this_ptr as i32))) // thiscall/cdecl hybrid
    }

    pub fn exception_ptr(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
        let ptr = uc.read_arg(0);
        crate::emu_log!(
            "[MSVCRT] (this={:#x}) exception::exception({:#x}) -> (this={:#x})",
            this_ptr,
            ptr,
            this_ptr
        );
        Some((1, Some(0)))
    }

    // API: void _CxxThrowException(void* pExceptionObject, _ThrowInfo* pThrowInfo)
    // 역할: C++ 예외를 발생시킴
    pub fn _cxx_throw_exception(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let p_exception_object = uc.read_arg(0);
        let p_throw_info = uc.read_arg(1);
        crate::emu_log!(
            "[MSVCRT] _CxxThrowException({:#x}, {:#x}) -> void",
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
                    crate::emu_log!("[!] MSVCRT Unhandled: {}", func_name);
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
