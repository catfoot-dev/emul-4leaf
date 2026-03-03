use unicorn_engine::Unicorn;

use crate::win32::Win32Context;

pub struct DllMSVCRT {}

impl DllMSVCRT {
    pub fn localtime() -> Option<(usize, Option<i32>)>{
        println!("localtime");
        Some((0, None))
    }

    pub fn strncmp() -> Option<(usize, Option<i32>)>{
        println!("strncmp");
        Some((0, None))
    }

    pub fn strcoll() -> Option<(usize, Option<i32>)>{
        println!("strcoll");
        Some((0, None))
    }

    pub fn strncpy() -> Option<(usize, Option<i32>)>{
        println!("strncpy");
        Some((0, None))
    }

    pub fn isspace() -> Option<(usize, Option<i32>)>{
        println!("isspace");
        Some((0, None))
    }

    pub fn isdigit() -> Option<(usize, Option<i32>)>{
        println!("isdigit");
        Some((0, None))
    }

    pub fn _vsnprintf() -> Option<(usize, Option<i32>)>{
        println!("_vsnprintf");
        Some((0, None))
    }

    pub fn __cxx_throw_exception() -> Option<(usize, Option<i32>)>{
        println!("__cxx_throw_exception");
        Some((0, None))
    }

    pub fn _exit() -> Option<(usize, Option<i32>)>{
        println!("_exit");
        Some((0, None))
    }

    pub fn _beginthreadex() -> Option<(usize, Option<i32>)>{
        println!("_beginthreadex");
        Some((0, None))
    }

    pub fn _endthreadex() -> Option<(usize, Option<i32>)>{
        println!("_endthreadex");
        Some((0, None))
    }

    pub fn sprintf() -> Option<(usize, Option<i32>)>{
        println!("sprintf");
        Some((0, None))
    }

    pub fn _set_se_translator() -> Option<(usize, Option<i32>)>{
        println!("_set_se_translator(void (*)(unit,_EXCEPTION_POINTEERS *))");
        Some((0, None))
    }

    pub fn new() -> Option<(usize, Option<i32>)>{
        println!("operator new(uint)");
        Some((0, None))
    }

    pub fn _except_handler3() -> Option<(usize, Option<i32>)>{
        println!("_except_handler3");
        Some((0, None))
    }

    pub fn memmove() -> Option<(usize, Option<i32>)>{
        println!("memmove");
        Some((0, None))
    }

    pub fn memchr() -> Option<(usize, Option<i32>)>{
        println!("memchr");
        Some((0, None))
    }

    pub fn ___cxx_frame_handler() -> Option<(usize, Option<i32>)>{
        println!("___cxx_frame_handler");
        Some((0, None))
    }

    pub fn _read() -> Option<(usize, Option<i32>)>{
        println!("_read");
        Some((0, None))
    }

    pub fn mktime() -> Option<(usize, Option<i32>)>{
        println!("mktime");
        Some((0, None))
    }

    pub fn atoi() -> Option<(usize, Option<i32>)>{
        println!("atoi");
        Some((0, None))
    }

    pub fn free() -> Option<(usize, Option<i32>)>{
        println!("free");
        Some((0, None))
    }

    pub fn __dllonexit() -> Option<(usize, Option<i32>)>{
        println!("__dllonexit");
        Some((0, None))
    }

    pub fn _onexit() -> Option<(usize, Option<i32>)>{
        println!("_onexit");
        Some((0, None))
    }

    pub fn terminate() -> Option<(usize, Option<i32>)>{
        println!("terminate(void)");
        Some((0, None))
    }

    pub fn type_info() -> Option<(usize, Option<i32>)>{
        println!("type_info::~type_info(void)");
        Some((0, None))
    }

    pub fn _initterm() -> Option<(usize, Option<i32>)>{
        println!("_initterm");
        Some((0, None))
    }

    pub fn malloc() -> Option<(usize, Option<i32>)>{
        println!("malloc");
        Some((0, None))
    }

    pub fn _adjust_fdiv() -> Option<(usize, Option<i32>)>{
        println!("_adjust_fdiv");
        Some((0, None))
    }

    pub fn _write() -> Option<(usize, Option<i32>)>{
        println!("_write");
        Some((0, None))
    }

    pub fn _close() -> Option<(usize, Option<i32>)>{
        println!("_close");
        Some((0, None))
    }

    pub fn _lseek() -> Option<(usize, Option<i32>)>{
        println!("_lseek");
        Some((0, None))
    }

    pub fn _open() -> Option<(usize, Option<i32>)>{
        println!("_open");
        Some((0, None))
    }

    pub fn _pipe() -> Option<(usize, Option<i32>)>{
        println!("_pipe");
        Some((0, None))
    }

    pub fn time() -> Option<(usize, Option<i32>)>{
        println!("time");
        Some((0, None))
    }

    pub fn _setjmp3() -> Option<(usize, Option<i32>)>{
        println!("_setjmp3");
        Some((0, None))
    }

    pub fn fopen() -> Option<(usize, Option<i32>)>{
        println!("fopen");
        Some((0, None))
    }

    pub fn fwrite() -> Option<(usize, Option<i32>)>{
        println!("fwrite");
        Some((0, None))
    }

    pub fn fclose() -> Option<(usize, Option<i32>)>{
        println!("fclose");
        Some((0, None))
    }

    pub fn longjmp() -> Option<(usize, Option<i32>)>{
        println!("longjmp");
        Some((0, None))
    }

    pub fn strstr() -> Option<(usize, Option<i32>)>{
        println!("strstr");
        Some((0, None))
    }

    pub fn fread() -> Option<(usize, Option<i32>)>{
        println!("fread");
        Some((0, None))
    }

    pub fn _ftol() -> Option<(usize, Option<i32>)>{
        println!("_ftol");
        Some((0, None))
    }

    pub fn fseek() -> Option<(usize, Option<i32>)>{
        println!("fseek");
        Some((0, None))
    }

    pub fn ftell() -> Option<(usize, Option<i32>)>{
        println!("ftell");
        Some((0, None))
    }

    pub fn _stricmp() -> Option<(usize, Option<i32>)>{
        println!("_stricmp");
        Some((0, None))
    }

    pub fn fflush() -> Option<(usize, Option<i32>)>{
        println!("fflush");
        Some((0, None))
    }

    pub fn sscanf() -> Option<(usize, Option<i32>)>{
        println!("sscanf");
        Some((0, None))
    }

    pub fn getenv() -> Option<(usize, Option<i32>)>{
        println!("getenv");
        Some((0, None))
    }

    pub fn _strcmpi() -> Option<(usize, Option<i32>)>{
        println!("_strcmpi");
        Some((0, None))
    }

    pub fn vsprintf() -> Option<(usize, Option<i32>)>{
        println!("vsprintf");
        Some((0, None))
    }

    pub fn calloc() -> Option<(usize, Option<i32>)>{
        println!("calloc");
        Some((0, None))
    }

    pub fn floor() -> Option<(usize, Option<i32>)>{
        println!("floor");
        Some((0, None))
    }

    pub fn realloc() -> Option<(usize, Option<i32>)>{
        println!("realloc");
        Some((0, None))
    }

    pub fn qsort() -> Option<(usize, Option<i32>)>{
        println!("qsort");
        Some((0, None))
    }

    pub fn frexp() -> Option<(usize, Option<i32>)>{
        println!("frexp");
        Some((0, None))
    }

    pub fn __c_ipow() -> Option<(usize, Option<i32>)>{
        println!("__c_ipow");
        Some((0, None))
    }

    pub fn ldexp() -> Option<(usize, Option<i32>)>{
        println!("ldexp");
        Some((0, None))
    }

    pub fn _errno() -> Option<(usize, Option<i32>)>{
        println!("_errno");
        Some((0, None))
    }

    pub fn _purecall() -> Option<(usize, Option<i32>)>{
        println!("_purecall");
        Some((0, None))
    }

    pub fn _beginthread() -> Option<(usize, Option<i32>)>{
        println!("_beginthread");
        Some((0, None))
    }

    pub fn ceil() -> Option<(usize, Option<i32>)>{
        println!("ceil");
        Some((0, None))
    }

    pub fn isalnum() -> Option<(usize, Option<i32>)>{
        println!("isalnum");
        Some((0, None))
    }

    pub fn fprintf() -> Option<(usize, Option<i32>)>{
        println!("fprintf");
        Some((0, None))
    }

    pub fn _strnicmp() -> Option<(usize, Option<i32>)>{
        println!("_strnicmp");
        Some((0, None))
    }

    pub fn rand() -> Option<(usize, Option<i32>)>{
        println!("rand");
        Some((0, None))
    }

    pub fn _itoa() -> Option<(usize, Option<i32>)>{
        println!("_itoa");
        Some((0, None))
    }

    pub fn strrchr() -> Option<(usize, Option<i32>)>{
        println!("strrchr");
        Some((0, None))
    }

    pub fn exception_ref() -> Option<(usize, Option<i32>)>{
        println!("0xB08AC | exception::exception(exception const &)");
        Some((0, None))
    }

    pub fn exception_ptr() -> Option<(usize, Option<i32>)>{
        println!("0xB08B0 | exception::exception(char const * const &)");
        Some((0, None))
    }

    pub fn srand() -> Option<(usize, Option<i32>)>{
        println!("srand");
        Some((0, None))
    }

    pub fn fscanf() -> Option<(usize, Option<i32>)>{
        println!("fscanf");
        Some((0, None))
    }

    pub fn strtok() -> Option<(usize, Option<i32>)>{
        println!("strtok");
        Some((0, None))
    }

    pub fn strtoul() -> Option<(usize, Option<i32>)>{
        println!("strtoul");
        Some((0, None))
    }

    pub fn _timezone() -> Option<(usize, Option<i32>)>{
        println!("_timezone");
        Some((0, None))
    }

    pub fn _stat() -> Option<(usize, Option<i32>)>{
        println!("_stat");
        Some((0, None))
    }

    
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<(usize, Option<i32>)> {
        match func_name {
            "localtime" => DllMSVCRT::localtime(),
            "strncmp" => DllMSVCRT::strncmp(),
            "strcoll" => DllMSVCRT::strcoll(),
            "strncpy" => DllMSVCRT::strncpy(),
            "isspace" => DllMSVCRT::isspace(),
            "isdigit" => DllMSVCRT::isdigit(),
            "_vsnprintf" => DllMSVCRT::_vsnprintf(),
            "_CxxThrowException" => DllMSVCRT::__cxx_throw_exception(),
            "_exit" => DllMSVCRT::_exit(),
            "_beginthreadex" => DllMSVCRT::_beginthreadex(),
            "_endthreadex" => DllMSVCRT::_endthreadex(),
            "sprintf" => DllMSVCRT::sprintf(),
            "?_set_se_translator@@YAP6AXIPAU_EXCEPTION_POINTERS@@@ZP6AXI0@Z@Z" => DllMSVCRT::_set_se_translator(),
            "??2@YAPAXI@Z" => DllMSVCRT::new(),
            "_except_handler3" => DllMSVCRT::_except_handler3(),
            "memmove" => DllMSVCRT::memmove(),
            "memchr" => DllMSVCRT::memchr(),
            "__CxxFrameHandler" => DllMSVCRT::___cxx_frame_handler(),
            "_read" => DllMSVCRT::_read(),
            "mktime" => DllMSVCRT::mktime(),
            "atoi" => DllMSVCRT::atoi(),
            "free" => DllMSVCRT::free(),
            "__dllonexit" => DllMSVCRT::__dllonexit(),
            "_onexit" => DllMSVCRT::_onexit(),
            "?terminate@@YAXXZ" => DllMSVCRT::terminate(),
            "??1type_info@@UAE@XZ" => DllMSVCRT::type_info(),
            "_initterm" => DllMSVCRT::_initterm(),
            "malloc" => DllMSVCRT::malloc(),
            "_adjust_fdiv" => DllMSVCRT::_adjust_fdiv(),
            "_write" => DllMSVCRT::_write(),
            "_close" => DllMSVCRT::_close(),
            "_lseek" => DllMSVCRT::_lseek(),
            "_open" => DllMSVCRT::_open(),
            "_pipe" => DllMSVCRT::_pipe(),
            "time" => DllMSVCRT::time(),
            "_setjmp3" => DllMSVCRT::_setjmp3(),
            "fopen" => DllMSVCRT::fopen(),
            "fwrite" => DllMSVCRT::fwrite(),
            "fclose" => DllMSVCRT::fclose(),
            "longjmp" => DllMSVCRT::longjmp(),
            "strstr" => DllMSVCRT::strstr(),
            "fread" => DllMSVCRT::fread(),
            "_ftol" => DllMSVCRT::_ftol(),
            "fseek" => DllMSVCRT::fseek(),
            "ftell" => DllMSVCRT::ftell(),
            "_stricmp" => DllMSVCRT::_stricmp(),
            "fflush" => DllMSVCRT::fflush(),
            "sscanf" => DllMSVCRT::sscanf(),
            "getenv" => DllMSVCRT::getenv(),
            "_strcmpi" => DllMSVCRT::_strcmpi(),
            "vsprintf" => DllMSVCRT::vsprintf(),
            "calloc" => DllMSVCRT::calloc(),
            "floor" => DllMSVCRT::floor(),
            "realloc" => DllMSVCRT::realloc(),
            "qsort" => DllMSVCRT::qsort(),
            "frexp" => DllMSVCRT::frexp(),
            "_CIpow" => DllMSVCRT::__c_ipow(),
            "ldexp" => DllMSVCRT::ldexp(),
            "_errno" => DllMSVCRT::_errno(),
            "_purecall" => DllMSVCRT::_purecall(),
            "_beginthread" => DllMSVCRT::_beginthread(),
            "ceil" => DllMSVCRT::ceil(),
            "isalnum" => DllMSVCRT::isalnum(),
            "fprintf" => DllMSVCRT::fprintf(),
            "_strnicmp" => DllMSVCRT::_strnicmp(),
            "rand" => DllMSVCRT::rand(),
            "_itoa" => DllMSVCRT::_itoa(),
            "strrchr" => DllMSVCRT::strrchr(),
            "??0exception@@QAE@ABV0@@Z" => DllMSVCRT::exception_ref(),
            "??0exception@@QAE@ABQBD@Z" => DllMSVCRT::exception_ptr(),
            "srand" => DllMSVCRT::srand(),
            "fscanf" => DllMSVCRT::fscanf(),
            "strtok" => DllMSVCRT::strtok(),
            "strtoul" => DllMSVCRT::strtoul(),
            "_timezone" => DllMSVCRT::_timezone(),
            "_stat" => DllMSVCRT::_stat(),
            _ => None
        }
    }
}
