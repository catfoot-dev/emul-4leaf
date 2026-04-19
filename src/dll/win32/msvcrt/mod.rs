mod format;
mod io;
mod math;
mod memory;
mod misc;
mod string;

use crate::{
    dll::win32::{ApiHookResult, StackCleanup, Win32Context},
    helper::UnicornHelper,
};
use unicorn_engine::Unicorn;

/// `MSVCRT.dll` 프록시 구현 모듈
///
/// C 런타임 라이브러리(CRT) 함수를 우회/구현하며 메모리 할당(malloc), 문자열 포맷팅, 예외 처리 등을 담당
#[allow(clippy::upper_case_acronyms)]
pub struct MSVCRT;

impl MSVCRT {
    fn wrap_result(func_name: &str, result: Option<ApiHookResult>) -> Option<ApiHookResult> {
        if func_name == "_CxxThrowException" {
            if let Some(mut r) = result {
                r.cleanup = StackCleanup::Callee(2);
                return Some(r);
            }
            return None;
        }

        // MSVCRT.dll은 대부분 cdecl 이지만 C++ 예외/기능 일부는 thiscall/stdcall 임
        let is_thiscall = func_name.contains("@QAE") || func_name.contains("@IAE");
        let is_stdcall = func_name.contains("@YG") || func_name == "__CxxFrameHandler";

        if !is_thiscall
            && !is_stdcall
            && let Some(mut r) = result
        {
            r.cleanup = StackCleanup::Caller;
            return Some(r);
        }
        result
    }

    /// `MSVCRT.dll`에서 데이터로 취급되어야 할 심볼들을 메모리에 할당하고 주소를 반환합니다.
    pub fn resolve_export(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<u32> {
        match func_name {
            "_adjust_fdiv" => {
                let addr = uc.malloc(4);
                uc.write_u32(addr, 0); // FDIV 버그 없음
                Some(addr as u32)
            }
            "_timezone" => {
                let addr = uc.malloc(4);
                uc.write_u32(addr, 32400); // UTC+9
                Some(addr as u32)
            }
            "_daylight" => {
                let addr = uc.malloc(4);
                uc.write_u32(addr, 0);
                Some(addr as u32)
            }
            "__argc" => {
                let addr = uc.malloc(4);
                uc.write_u32(addr, 1);
                Some(addr as u32)
            }
            "__argv" => {
                let argv0 = uc.alloc_str("4Leaf.exe");
                let addr = uc.malloc(8);
                uc.write_u32(addr, argv0);
                uc.write_u32(addr + 4, 0);
                Some(addr as u32)
            }
            _ => None,
        }
    }

    /// 함수명 기준 `MSVCRT.dll` API 구현체입니다. 처리를 성공했다면 스택 보정값과 리턴값을 포함한 `ApiHookResult`를 반환합니다.
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        MSVCRT::wrap_result(
            func_name,
            match func_name {
                // string
                "strncmp" => string::strncmp(uc),
                "strcoll" => string::strcoll(uc),
                "strncpy" => string::strncpy(uc),
                "strstr" => string::strstr(uc),
                "strrchr" => string::strrchr(uc),
                "strtok" => string::strtok(uc),
                "_stricmp" => string::_stricmp(uc),
                "_strcmpi" => string::_strcmpi(uc),
                "_strnicmp" => string::_strnicmp(uc),
                "isspace" => string::isspace(uc),
                "isdigit" => string::isdigit(uc),
                "isalnum" => string::isalnum(uc),

                // memory
                "malloc" => memory::malloc(uc),
                "free" => memory::free(uc),
                "calloc" => memory::calloc(uc),
                "realloc" => memory::realloc(uc),
                "??2@YAPAXI@Z" => memory::new_op(uc),
                "memmove" => memory::memmove(uc),
                "memchr" => memory::memchr(uc),

                // io
                "fopen" => io::fopen(uc),
                "fclose" => io::fclose(uc),
                "fread" => io::fread(uc),
                "fwrite" => io::fwrite(uc),
                "fseek" => io::fseek(uc),
                "ftell" => io::ftell(uc),
                "fflush" => io::fflush(uc),
                "feof" => io::feof(uc),
                "ferror" => io::ferror(uc),
                "clearerr" => io::clearerr(uc),
                "rewind" => io::rewind(uc),
                "_open" => io::_open(uc),
                "_close" => io::_close(uc),
                "_read" => io::_read(uc),
                "_write" => io::_write(uc),
                "_lseek" => io::_lseek(uc),
                "_pipe" => io::_pipe(uc),
                "_stat" => io::_stat(uc),

                // format
                "sprintf" => format::sprintf(uc),
                "_vsnprintf" => format::_vsnprintf(uc),
                "vsprintf" => format::vsprintf(uc),
                "sscanf" => format::sscanf(uc),
                "fprintf" => format::fprintf(uc),
                "fscanf" => format::fscanf(uc),

                // math
                "floor" => math::floor(uc),
                "ceil" => math::ceil(uc),
                "frexp" => math::frexp(uc),
                "ldexp" => math::ldexp(uc),
                "_ftol" => math::_ftol(uc),
                "_CIpow" => math::__c_ipow(uc),

                // misc
                "localtime" => misc::localtime(uc),
                "mktime" => misc::mktime(uc),
                "time" => misc::time(uc),
                "_timezone" => misc::_timezone(uc),
                "atoi" => misc::atoi(uc),
                "_itoa" => misc::_itoa(uc),
                "strtoul" => misc::strtoul(uc),
                "rand" => misc::rand(uc),
                "srand" => misc::srand(uc),
                "getenv" => misc::getenv(uc),
                "_beginthreadex" => misc::_beginthreadex(uc),
                "_endthreadex" => misc::_endthreadex(uc),
                "_beginthread" => misc::_beginthread(uc),
                "__cxx_throw_exception" => misc::__cxx_throw_exception(uc),
                "_except_handler3" => misc::_except_handler3(uc),
                "__CxxFrameHandler" => misc::___cxx_frame_handler(uc),
                "?_set_se_translator@@YAP6AXIPAU_EXCEPTION_POINTERS@@@ZP6AXI0@Z@Z" => {
                    misc::_set_se_translator(uc)
                }
                "_setjmp3" => misc::_setjmp3(uc),
                "longjmp" => misc::longjmp(uc),
                "_initterm" => misc::_initterm(uc),
                "_exit" => misc::_exit(uc),
                "__dllonexit" => misc::__dllonexit(uc),
                "_onexit" => misc::_onexit(uc),
                "?terminate@@YAXXZ" => misc::terminate(uc),
                "??1type_info@@UAE@XZ" => misc::type_info(uc),
                "_adjust_fdiv" => misc::_adjust_fdiv(uc),
                "_purecall" => misc::_purecall(uc),
                "_errno" => misc::_errno(uc),
                "qsort" => misc::qsort(uc),
                "??0exception@@QAE@ABV0@@Z" => misc::exception_ref(uc),
                "??0exception@@QAE@ABQBD@Z" => misc::exception_ptr(uc),
                "_CxxThrowException" => misc::_cxx_throw_exception(uc),
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
    use crate::dll::win32::StackCleanup;

    #[test]
    fn sprintf_uses_caller_cleanup() {
        let result =
            MSVCRT::wrap_result("sprintf", Some(ApiHookResult::callee(3, Some(5)))).unwrap();
        assert_eq!(result.cleanup, StackCleanup::Caller);
    }

    #[test]
    fn cxx_throw_exception_keeps_callee_cleanup() {
        let result =
            MSVCRT::wrap_result("_CxxThrowException", Some(ApiHookResult::caller(None))).unwrap();
        assert_eq!(result.cleanup, StackCleanup::Callee(2));
    }
}
