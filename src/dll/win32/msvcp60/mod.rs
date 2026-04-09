mod basic_string;
mod filebuf;
mod ios;
mod istream;
mod ostream;
mod streambuf;

use crate::{
    dll::win32::{ApiHookResult, FileState, StackCleanup, Win32Context},
    helper::UnicornHelper,
};
use std::{
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom, Write},
};
use unicorn_engine::{RegisterX86, Unicorn};

const BASIC_STRING_PTR_OFFSET: u64 = 4;
const BASIC_STRING_LEN_OFFSET: u64 = 8;
const BASIC_STRING_RES_OFFSET: u64 = 12;

const IOS_STREAMBUF_OFFSET: u64 = 4;
const IOS_STREAMBUF_ALT_OFFSET: u64 = 40;
const IOS_FLAGS_OFFSET: u64 = 8;
const IOS_STATE_OFFSET: u64 = 16;
const IOS_LOCALE_OFFSET: u64 = 20;

const STREAMBUF_BUFFER_OFFSET: u64 = 4;
const STREAMBUF_CAPACITY_OFFSET: u64 = 8;
const STREAMBUF_READ_POS_OFFSET: u64 = 12;
const STREAMBUF_WRITE_POS_OFFSET: u64 = 16;
const STREAMBUF_LOCALE_OFFSET: u64 = 20;
const STREAMBUF_LAST_CHAR_OFFSET: u64 = 24;
const STREAMBUF_FILE_HANDLE_OFFSET: u64 = 28;
const STREAMBUF_FILE_MODE_OFFSET: u64 = 32;

const FACET_REFCOUNT_OFFSET: u64 = 4;

const STREAM_OBJECT_SIZE: usize = 0x80;
const STREAMBUF_OBJECT_SIZE: usize = 0x40;
const LOCALE_OBJECT_SIZE: usize = 0x20;
const LOCIMP_OBJECT_SIZE: usize = 0x20;
const STREAMBUF_HOST_BUFFER_SIZE: usize = 0x400;

const EMPTY_STRING_CACHE_KEY: &str = "$internal_empty_cstr";
const LOCALE_IMPL_CACHE_KEY: &str = "$internal_locale_locimp";
const LOCALE_VALUE_CACHE_KEY: &str = "$internal_locale_value";

const BASIC_STREAMBUF_VTABLE: &str = "??_7?$basic_streambuf@DU?$char_traits@D@std@@@std@@6B@";
const BASIC_FILEBUF_VTABLE: &str = "??_7?$basic_filebuf@DU?$char_traits@D@std@@@std@@6B@";
const BASIC_OSTREAM_VTABLE: &str = "??_7?$basic_ostream@DU?$char_traits@D@std@@@std@@6B@";
const BASIC_ISTREAM_VTABLE: &str = "??_7?$basic_istream@DU?$char_traits@D@std@@@std@@6B@";
const BASIC_IOSTREAM_VTABLE: &str = "??_7?$basic_iostream@DU?$char_traits@D@std@@@std@@6B@";
const BASIC_IOS_VTABLE: &str = "??_7?$basic_ios@DU?$char_traits@D@std@@@std@@6B@";
const IOS_BASE_VTABLE: &str = "??_7ios_base@std@@6B@";

const SYMBOL_BASIC_STRING_NPOS: &str =
    "?npos@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@2IB";
const SYMBOL_BASIC_STRING_NULLSTR_DATA: &str =
    "?_C@?1??_Nullstr@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@CAPBDXZ@4DB";
const SYMBOL_BASIC_STRING_ASSIGN_PTR_LEN: &str =
    "?assign@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEAAV12@PBDI@Z";
const SYMBOL_BASIC_STRING_NULLSTR_FN: &str =
    "?_Nullstr@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@CAPBDXZ";
const SYMBOL_STD_FPZ: &str = "?_Fpz@std@@3_JB";
const SYMBOL_STD_CIN: &str = "?cin@std@@3V?$basic_istream@DU?$char_traits@D@std@@@1@A";
const SYMBOL_STD_COUT: &str = "?cout@std@@3V?$basic_ostream@DU?$char_traits@D@std@@@1@A";
const SYMBOL_STD_CERR: &str = "?cerr@std@@3V?$basic_ostream@DU?$char_traits@D@std@@@1@A";
const SYMBOL_STD_CLOG: &str = "?clog@std@@3V?$basic_ostream@DU?$char_traits@D@std@@@1@A";
const SYMBOL_LOCALE_GLOBAL_PTR: &str = "?_Global@_Locimp@locale@std@@0PAV123@A";
const SYMBOL_LOCALE_CLOCPTR: &str = "?_Clocptr@_Locimp@locale@std@@0PAV123@A";
const SYMBOL_LOCALE_FACET_ID_CNT: &str = "?_Id_cnt@facet@locale@std@@0HA";
const SYMBOL_LOCALE_ID_ID_CNT: &str = "?_Id_cnt@id@locale@std@@0HA";
const SYMBOL_BASIC_FILEBUF_STATIC_INIT_FLAG: &str =
    "?_Stinit@?1??_Init@?$basic_filebuf@DU?$char_traits@D@std@@@std@@IAEXPAU_iobuf@@W4_Initfl@23@@Z@4HA";
const SYMBOL_STD_FLUSH: &str =
    "?flush@std@@YAAAV?$basic_ostream@DU?$char_traits@D@std@@@1@AAV21@@Z";

const GLOBAL_STREAM_OBJECT_SYMBOLS: [&str; 4] = [
    SYMBOL_STD_CIN,
    SYMBOL_STD_COUT,
    SYMBOL_STD_CERR,
    SYMBOL_STD_CLOG,
];

const FN_BASIC_STRING_CTOR_DEFAULT: &str =
    "??0?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAE@ABV?$allocator@D@1@@Z";
const FN_BASIC_STRING_CTOR_CSTR: &str =
    "??0?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAE@PBDABV?$allocator@D@1@@Z";
const FN_BASIC_STRING_DTOR: &str =
    "??1?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAE@XZ";
const FN_BASIC_STRING_TIDY: &str =
    "?_Tidy@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@AAEX_N@Z";
const FN_BASIC_STRING_GROW: &str =
    "?_Grow@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@AAE_NI_N@Z";
const FN_BASIC_STRING_COPY: &str =
    "?_Copy@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@AAEXI@Z";
const FN_BASIC_STRING_EOS: &str =
    "?_Eos@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@AAEXI@Z";
const FN_BASIC_STRING_FREEZE: &str =
    "?_Freeze@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@AAEXXZ";
const FN_BASIC_STRING_SPLIT: &str =
    "?_Split@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@AAEXXZ";
const FN_BASIC_STRING_ASSIGN_PTR: &str =
    "?assign@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEAAV12@PBD@Z";
const FN_BASIC_STRING_ASSIGN_SUBSTR: &str =
    "?assign@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEAAV12@ABV12@II@Z";
const FN_BASIC_STRING_APPEND_SUBSTR: &str =
    "?append@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEAAV12@ABV12@II@Z";
const FN_BASIC_STRING_COMPARE_OTHER: &str =
    "?compare@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QBEHABV12@@Z";
const FN_BASIC_STRING_COMPARE_PTR: &str =
    "?compare@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QBEHIIPBDI@Z";
const FN_BASIC_STRING_ERASE: &str =
    "?erase@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEAAV12@II@Z";
const FN_BASIC_STRING_REPLACE_REPEAT: &str =
    "?replace@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEAAV12@IIID@Z";
const FN_BASIC_STRING_REPLACE_RANGE_PTRS: &str =
    "?replace@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEAAV12@PAD0PBD1@Z";
const FN_BASIC_STRING_RESIZE: &str =
    "?resize@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEXI@Z";
const FN_BASIC_STRING_SWAP: &str =
    "?swap@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEXAAV12@@Z";
const FN_BASIC_STRING_SUBSTR: &str =
    "?substr@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QBE?AV12@II@Z";
const FN_BASIC_STRING_C_STR: &str =
    "?c_str@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QBEPBDXZ";
const FN_BASIC_STRING_END: &str =
    "?end@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEPADXZ";
const FN_BASIC_STRING_SIZE: &str =
    "?size@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QBEIXZ";
const FN_BASIC_STRING_MAX_SIZE: &str =
    "?max_size@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QBEIXZ";

const FN_BASIC_STREAMBUF_COPY_CTOR: &str =
    "??0?$basic_streambuf@DU?$char_traits@D@std@@@std@@QAE@ABV01@@Z";
const FN_BASIC_STREAMBUF_INIT: &str =
    "?_Init@?$basic_streambuf@DU?$char_traits@D@std@@@std@@IAEXXZ";
const FN_BASIC_STREAMBUF_INIT_RANGES: &str =
    "?_Init@?$basic_streambuf@DU?$char_traits@D@std@@@std@@IAEXPAPAD0PAH001@Z";
const FN_BASIC_STREAMBUF_DTOR: &str = "??1?$basic_streambuf@DU?$char_traits@D@std@@@std@@UAE@XZ";
const FN_BASIC_STREAMBUF_ASSIGN: &str =
    "??4?$basic_streambuf@DU?$char_traits@D@std@@@std@@QAEAAV01@ABV01@@Z";
const FN_BASIC_STREAMBUF_SETG: &str =
    "?setg@?$basic_streambuf@DU?$char_traits@D@std@@@std@@IAEXPAD00@Z";
const FN_BASIC_STREAMBUF_SETP: &str =
    "?setp@?$basic_streambuf@DU?$char_traits@D@std@@@std@@IAEXPAD0@Z";
const FN_BASIC_STREAMBUF_IMBUE: &str =
    "?imbue@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MAEXABVlocale@2@@Z";
const FN_BASIC_STREAMBUF_SETBUF: &str =
    "?setbuf@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MAEPAV12@PADH@Z";
const FN_BASIC_STREAMBUF_SEEKOFF: &str =
    "?seekoff@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MAE?AV?$fpos@H@2@JW4seekdir@ios_base@2@H@Z";
const FN_BASIC_STREAMBUF_SEEKPOS: &str =
    "?seekpos@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MAE?AV?$fpos@H@2@V32@H@Z";
const FN_BASIC_STREAMBUF_XSPUTN: &str =
    "?xsputn@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MAEHPBDH@Z";
const FN_BASIC_STREAMBUF_XSGETN: &str =
    "?xsgetn@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MAEHPADH@Z";
const FN_BASIC_STREAMBUF_UNDERFLOW: &str =
    "?underflow@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MAEHXZ";
const FN_BASIC_STREAMBUF_UFLOW: &str =
    "?uflow@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MAEHXZ";
const FN_BASIC_STREAMBUF_SHOWMANYC: &str =
    "?showmanyc@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MAEHXZ";
const FN_BASIC_STREAMBUF_PBACKFAIL: &str =
    "?pbackfail@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MAEHH@Z";
const FN_BASIC_STREAMBUF_SYNC: &str = "?sync@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MAEHXZ";

const FN_BASIC_OSTREAM_COPY_CTOR: &str =
    "??0?$basic_ostream@DU?$char_traits@D@std@@@std@@QAE@ABV01@@Z";
const FN_BASIC_OSTREAM_CTOR3: &str =
    "??0?$basic_ostream@DU?$char_traits@D@std@@@std@@QAE@PAV?$basic_streambuf@DU?$char_traits@D@std@@@1@_N1@Z";
const FN_BASIC_OSTREAM_CTOR2: &str =
    "??0?$basic_ostream@DU?$char_traits@D@std@@@std@@QAE@PAV?$basic_streambuf@DU?$char_traits@D@std@@@1@_N@Z";
const FN_BASIC_OSTREAM_DTOR: &str = "??1?$basic_ostream@DU?$char_traits@D@std@@@std@@UAE@XZ";
const FN_OSTREAM_INSERT_INT: &str = "??6?$basic_ostream@DU?$char_traits@D@std@@@std@@QAEAAV01@H@Z";
const FN_BASIC_OSTREAM_WRITE: &str =
    "?write@?$basic_ostream@DU?$char_traits@D@std@@@std@@QAEAAV12@PBDH@Z";

const FN_BASIC_ISTREAM_CTOR: &str =
    "??0?$basic_istream@DU?$char_traits@D@std@@@std@@QAE@PAV?$basic_streambuf@DU?$char_traits@D@std@@@1@_N@Z";
const FN_BASIC_ISTREAM_DTOR: &str = "??1?$basic_istream@DU?$char_traits@D@std@@@std@@UAE@XZ";
const FN_BASIC_ISTREAM_SEEKG: &str =
    "?seekg@?$basic_istream@DU?$char_traits@D@std@@@std@@QAEAAV12@V?$fpos@H@2@@Z";
const FN_BASIC_ISTREAM_GETLINE: &str =
    "?getline@?$basic_istream@DU?$char_traits@D@std@@@std@@QAEAAV12@PADHD@Z";
const FN_BASIC_IOSTREAM_CTOR: &str =
    "??0?$basic_iostream@DU?$char_traits@D@std@@@std@@QAE@PAV?$basic_streambuf@DU?$char_traits@D@std@@@1@@Z";
const FN_BASIC_IOSTREAM_DTOR: &str = "??1?$basic_iostream@DU?$char_traits@D@std@@@std@@UAE@XZ";
const FN_BASIC_IFSTREAM_VBASE_DTOR: &str =
    "??_D?$basic_ifstream@DU?$char_traits@D@std@@@std@@QAEXXZ";
const FN_BASIC_ISTREAM_EXTRACT_UNSIGNED_SHORT: &str =
    "??5?$basic_istream@DU?$char_traits@D@std@@@std@@QAEAAV01@AAG@Z";

const FN_BASIC_FILEBUF_CTOR: &str =
    "??0?$basic_filebuf@DU?$char_traits@D@std@@@std@@QAE@PAU_iobuf@@@Z";
const FN_BASIC_FILEBUF_INIT: &str =
    "?_Init@?$basic_filebuf@DU?$char_traits@D@std@@@std@@IAEXPAU_iobuf@@W4_Initfl@12@@Z";
const FN_BASIC_FILEBUF_OPEN: &str =
    "?open@?$basic_filebuf@DU?$char_traits@D@std@@@std@@QAEPAV12@PBDH@Z";
const FN_BASIC_FILEBUF_INITCVT: &str =
    "?_Initcvt@?$basic_filebuf@DU?$char_traits@D@std@@@std@@IAEXXZ";
const FN_BASIC_FILEBUF_DTOR: &str = "??1?$basic_filebuf@DU?$char_traits@D@std@@@std@@UAE@XZ";

const FN_BASIC_IOS_CTOR: &str = "??0?$basic_ios@DU?$char_traits@D@std@@@std@@IAE@XZ";
const FN_BASIC_IOS_CLEAR: &str = "?clear@?$basic_ios@DU?$char_traits@D@std@@@std@@QAEXH_N@Z";
const FN_BASIC_IOS_SETSTATE: &str = "?setstate@?$basic_ios@DU?$char_traits@D@std@@@std@@QAEXH_N@Z";
const FN_BASIC_IOS_INIT: &str =
    "?init@?$basic_ios@DU?$char_traits@D@std@@@std@@IAEXPAV?$basic_streambuf@DU?$char_traits@D@std@@@2@_N@Z";
const FN_BASIC_IOS_WIDEN: &str = "?widen@?$basic_ios@DU?$char_traits@D@std@@@std@@QBEDD@Z";
const FN_BASIC_IOS_DTOR: &str = "??1?$basic_ios@DU?$char_traits@D@std@@@std@@UAE@XZ";
const FN_BASIC_IOS_ASSIGN: &str = "??4?$basic_ios@DU?$char_traits@D@std@@@std@@QAEAAV01@ABV01@@Z";

const FN_IOS_BASE_CTOR: &str = "??0ios_base@std@@IAE@XZ";
const FN_IOS_BASE_DTOR: &str = "??1ios_base@std@@UAE@XZ";
const FN_IOS_BASE_ASSIGN: &str = "??4ios_base@std@@QAEAAV01@ABV01@@Z";
const FN_IOS_BASE_CLEAR: &str = "?clear@ios_base@std@@QAEXH_N@Z";
const FN_IOS_BASE_COPYFMT: &str = "?copyfmt@ios_base@std@@QAEAAV12@ABV12@@Z";
const FN_IOS_BASE_GETLOC: &str = "?getloc@ios_base@std@@QBE?AVlocale@2@XZ";
const FN_IOS_BASE_INIT: &str = "?_Init@ios_base@std@@IAEXXZ";
const FN_IOS_BASE_INIT_CTOR: &str = "??0Init@ios_base@std@@QAE@XZ";
const FN_IOS_BASE_INIT_DTOR: &str = "??1Init@ios_base@std@@QAE@XZ";

const FN_LOCALE_INIT: &str = "?_Init@locale@std@@CAPAV_Locimp@12@XZ";
const FN_LOCALE_CTOR: &str = "??0locale@std@@QAE@XZ";
const FN_LOCALE_DTOR: &str = "??1locale@std@@QAE@XZ";
const FN_LOCALE_ASSIGN: &str = "??4locale@std@@QAEAAV01@ABV01@@Z";
const FN_LOCALE_FACET_INCREF: &str = "?_Incref@facet@locale@std@@QAEXXZ";
const FN_LOCALE_FACET_DECREF: &str = "?_Decref@facet@locale@std@@QAEPAV123@XZ";

const FN_STRSTREAMBUF_INIT: &str = "?_Init@strstreambuf@std@@IAEXHPAD0H@Z";
const FN_WINIT_CTOR: &str = "??0_Winit@std@@QAE@XZ";
const FN_WINIT_DTOR: &str = "??1_Winit@std@@QAE@XZ";
const FN_LOCKIT_CTOR: &str = "??0_Lockit@std@@QAE@XZ";
const FN_LOCKIT_DTOR: &str = "??1_Lockit@std@@QAE@XZ";
const FN_STD_OSTREAM_INSERT_CSTR: &str =
    "??6std@@YAAAV?$basic_ostream@DU?$char_traits@D@std@@@0@AAV10@PBD@Z";
const FN_STD_OSTREAM_INSERT_CHAR: &str =
    "??6std@@YAAAV?$basic_ostream@DU?$char_traits@D@std@@@0@AAV10@D@Z";
const FN_STD_ENDL: &str = "?endl@std@@YAAAV?$basic_ostream@DU?$char_traits@D@std@@@1@AAV21@@Z";
const FN_STD_XLEN: &str = "?_Xlen@std@@YAXXZ";
const FN_STD_XRAN: &str = "?_Xran@std@@YAXXZ";
const FN_STD_XOFF: &str = "?_Xoff@std@@YAXXZ";
const FN_STD_FIOPEN: &str = "?__Fiopen@std@@YAPAU_iobuf@@PBDH@Z";
const FN_BASIC_OFSTREAM_CTOR: &str = "??0?$basic_ofstream@DU?$char_traits@D@std@@@std@@QAE@XZ";
const FN_BASIC_OFSTREAM_DTOR: &str = "??_D?$basic_ofstream@DU?$char_traits@D@std@@@std@@QAEXXZ";
const FN_BASIC_FSTREAM_DTOR: &str = "??_D?$basic_fstream@DU?$char_traits@D@std@@@std@@QAEXXZ";

/// `MSVCP60.dll` 프록시 구현 모듈
///
/// VC6 STL의 문자열/스트림/locale 초기화에 필요한 최소 런타임 상태를 에뮬레이션합니다.
pub struct MSVCP60;

impl MSVCP60 {
    fn is_cdecl_symbol(func_name: &str) -> bool {
        func_name.contains("@YA") || func_name.contains("@Y?A")
    }

    fn this_ptr(uc: &Unicorn<Win32Context>) -> u32 {
        uc.reg_read(RegisterX86::ECX).unwrap_or(0) as u32
    }

    fn proxy_cache_key(func_name: &str) -> String {
        format!("MSVCP60.dll!{}", func_name)
    }

    fn alloc_zeroed(uc: &mut Unicorn<Win32Context>, size: usize) -> u32 {
        let addr = uc.malloc(size) as u32;
        if size != 0 {
            let zeros = vec![0u8; size];
            let _ = uc.mem_write(addr as u64, &zeros);
        }
        addr
    }

    fn read_exact_bytes(uc: &Unicorn<Win32Context>, addr: u32, len: usize) -> Vec<u8> {
        if addr == 0 || len == 0 {
            return Vec::new();
        }

        let mut buf = vec![0u8; len];
        if uc.mem_read(addr as u64, &mut buf).is_err() {
            return Vec::new();
        }
        buf
    }

    fn is_mapped_ptr(uc: &Unicorn<Win32Context>, addr: u32) -> bool {
        if addr == 0 {
            return false;
        }
        let mut probe = [0u8; 1];
        uc.mem_read(addr as u64, &mut probe).is_ok()
    }

    fn cached_proxy_export<F>(uc: &mut Unicorn<Win32Context>, func_name: &str, init: F) -> u32
    where
        F: FnOnce(&mut Unicorn<Win32Context>) -> u32,
    {
        let key = Self::proxy_cache_key(func_name);
        if let Some(addr) = {
            let context = uc.get_data();
            context.proxy_exports.lock().unwrap().get(&key).copied()
        } {
            return addr;
        }

        let addr = init(uc);
        {
            let context = uc.get_data();
            context.proxy_exports.lock().unwrap().insert(key, addr);
        }
        addr
    }

    fn vtable_export_addr(uc: &mut Unicorn<Win32Context>, name: &str) -> u32 {
        Self::cached_proxy_export(uc, name, |uc| Self::alloc_zeroed(uc, 64))
    }

    fn empty_c_string_addr(uc: &mut Unicorn<Win32Context>) -> u32 {
        Self::cached_proxy_export(uc, EMPTY_STRING_CACHE_KEY, |uc| {
            let addr = Self::alloc_zeroed(uc, 1);
            uc.write_u8(addr as u64, 0);
            addr
        })
    }

    fn locale_impl_addr(uc: &mut Unicorn<Win32Context>) -> u32 {
        Self::cached_proxy_export(uc, LOCALE_IMPL_CACHE_KEY, |uc| {
            let addr = Self::alloc_zeroed(uc, LOCIMP_OBJECT_SIZE);
            uc.write_u32(addr as u64 + FACET_REFCOUNT_OFFSET, 1);
            addr
        })
    }

    fn locale_value_addr(uc: &mut Unicorn<Win32Context>) -> u32 {
        Self::cached_proxy_export(uc, LOCALE_VALUE_CACHE_KEY, |uc| {
            let addr = Self::alloc_zeroed(uc, LOCALE_OBJECT_SIZE);
            let locimp_addr = Self::locale_impl_addr(uc);
            Self::write_locale_value(uc, addr, locimp_addr);
            addr
        })
    }

    fn write_locale_value(uc: &mut Unicorn<Win32Context>, locale_addr: u32, locimp_addr: u32) {
        if locale_addr != 0 {
            // 원본은 locale 스택 객체의 뒤쪽 필드도 상태 분기에 사용하므로
            // locimp 포인터만 쓰지 말고 전체 객체를 0으로 초기화합니다.
            let _ = uc.mem_write(locale_addr as u64, &[0u8; LOCALE_OBJECT_SIZE]);
            uc.write_u32(locale_addr as u64, locimp_addr);
        }
    }

    fn read_locale_impl(uc: &Unicorn<Win32Context>, locale_addr: u32) -> u32 {
        if locale_addr == 0 {
            return 0;
        }
        uc.read_u32(locale_addr as u64)
    }

    fn init_streambuf_layout(uc: &mut Unicorn<Win32Context>, this_ptr: u32, vtable_name: &str) {
        if this_ptr == 0 {
            return;
        }

        let vtable_addr = Self::vtable_export_addr(uc, vtable_name);
        let locale_addr = Self::locale_value_addr(uc);
        uc.write_u32(this_ptr as u64, vtable_addr);
        uc.write_u32(this_ptr as u64 + STREAMBUF_BUFFER_OFFSET, 0);
        uc.write_u32(this_ptr as u64 + STREAMBUF_CAPACITY_OFFSET, 0);
        uc.write_u32(this_ptr as u64 + STREAMBUF_READ_POS_OFFSET, 0);
        uc.write_u32(this_ptr as u64 + STREAMBUF_WRITE_POS_OFFSET, 0);
        uc.write_u32(this_ptr as u64 + STREAMBUF_LOCALE_OFFSET, locale_addr);
        uc.write_u32(this_ptr as u64 + STREAMBUF_LAST_CHAR_OFFSET, u32::MAX);
        uc.write_u32(this_ptr as u64 + STREAMBUF_FILE_HANDLE_OFFSET, 0);
        uc.write_u32(this_ptr as u64 + STREAMBUF_FILE_MODE_OFFSET, 0);
    }

    fn init_filebuf_layout(
        uc: &mut Unicorn<Win32Context>,
        this_ptr: u32,
        file_handle: u32,
        mode: u32,
    ) {
        Self::init_streambuf_layout(uc, this_ptr, BASIC_FILEBUF_VTABLE);
        let buffer_addr = Self::alloc_zeroed(uc, STREAMBUF_HOST_BUFFER_SIZE);
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_BUFFER_OFFSET, buffer_addr);
        Self::write_streambuf_field(
            uc,
            this_ptr,
            STREAMBUF_CAPACITY_OFFSET,
            STREAMBUF_HOST_BUFFER_SIZE as u32,
        );
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_FILE_HANDLE_OFFSET, file_handle);
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_FILE_MODE_OFFSET, mode);
    }

    fn init_ios_base_layout(uc: &mut Unicorn<Win32Context>, this_ptr: u32, vtable_name: &str) {
        if this_ptr == 0 {
            return;
        }

        let vtable_addr = Self::vtable_export_addr(uc, vtable_name);
        let locale_addr = Self::locale_value_addr(uc);
        uc.write_u32(this_ptr as u64, vtable_addr);
        uc.write_u32(this_ptr as u64 + IOS_FLAGS_OFFSET, 0);
        uc.write_u32(this_ptr as u64 + IOS_STATE_OFFSET, 0);
        uc.write_u32(this_ptr as u64 + IOS_LOCALE_OFFSET, locale_addr);
    }

    fn init_basic_ios_layout(
        uc: &mut Unicorn<Win32Context>,
        this_ptr: u32,
        vtable_name: &str,
        streambuf_ptr: u32,
    ) {
        Self::init_ios_base_layout(uc, this_ptr, vtable_name);
        Self::write_basic_ios_streambuf_ptr(uc, this_ptr, streambuf_ptr);
    }

    fn write_basic_ios_streambuf_ptr(
        uc: &mut Unicorn<Win32Context>,
        this_ptr: u32,
        streambuf_ptr: u32,
    ) {
        if this_ptr == 0 {
            return;
        }
        uc.write_u32(this_ptr as u64 + IOS_STREAMBUF_OFFSET, streambuf_ptr);
        uc.write_u32(this_ptr as u64 + IOS_STREAMBUF_ALT_OFFSET, streambuf_ptr);
    }

    fn looks_like_streambuf_ptr(uc: &Unicorn<Win32Context>, ptr: u32) -> bool {
        ptr != 0 && Self::is_mapped_ptr(uc, ptr) && uc.read_u32(ptr as u64) != 0
    }

    fn read_basic_ios_streambuf_ptr(uc: &Unicorn<Win32Context>, this_ptr: u32) -> u32 {
        if this_ptr == 0 {
            return 0;
        }
        let primary = uc.read_u32(this_ptr as u64 + IOS_STREAMBUF_OFFSET);
        if Self::looks_like_streambuf_ptr(uc, primary) {
            return primary;
        }
        let alternate = uc.read_u32(this_ptr as u64 + IOS_STREAMBUF_ALT_OFFSET);
        if Self::looks_like_streambuf_ptr(uc, alternate) {
            return alternate;
        }
        primary
    }

    fn init_global_stream_object(uc: &mut Unicorn<Win32Context>, func_name: &str) -> u32 {
        Self::cached_proxy_export(uc, func_name, |uc| {
            let stream_addr = Self::alloc_zeroed(uc, STREAM_OBJECT_SIZE);
            let buffer_addr = Self::alloc_zeroed(uc, STREAMBUF_OBJECT_SIZE);
            Self::init_streambuf_layout(uc, buffer_addr, BASIC_STREAMBUF_VTABLE);

            let vtable = match func_name {
                SYMBOL_STD_CIN => BASIC_ISTREAM_VTABLE,
                _ => BASIC_OSTREAM_VTABLE,
            };
            Self::init_basic_ios_layout(uc, stream_addr, vtable, buffer_addr);
            stream_addr
        })
    }

    fn init_basic_string_empty(uc: &mut Unicorn<Win32Context>, this_ptr: u32) {
        if this_ptr == 0 {
            return;
        }

        let empty_addr = Self::empty_c_string_addr(uc);
        uc.write_u32(this_ptr as u64 + BASIC_STRING_PTR_OFFSET, empty_addr);
        uc.write_u32(this_ptr as u64 + BASIC_STRING_LEN_OFFSET, 0);
        uc.write_u32(this_ptr as u64 + BASIC_STRING_RES_OFFSET, 0);
    }

    fn basic_string_len(uc: &Unicorn<Win32Context>, this_ptr: u32) -> u32 {
        if this_ptr == 0 {
            return 0;
        }
        uc.read_u32(this_ptr as u64 + BASIC_STRING_LEN_OFFSET)
    }

    fn basic_string_ptr(uc: &Unicorn<Win32Context>, this_ptr: u32) -> u32 {
        if this_ptr == 0 {
            return 0;
        }
        uc.read_u32(this_ptr as u64 + BASIC_STRING_PTR_OFFSET)
    }

    fn basic_string_bytes(uc: &Unicorn<Win32Context>, this_ptr: u32) -> Vec<u8> {
        let len = Self::basic_string_len(uc, this_ptr) as usize;
        let ptr = Self::basic_string_ptr(uc, this_ptr);
        Self::read_exact_bytes(uc, ptr, len)
    }

    fn source_bytes_from_ptr(
        uc: &Unicorn<Win32Context>,
        ptr: u32,
        explicit_len: Option<usize>,
    ) -> Vec<u8> {
        if ptr == 0 {
            return Vec::new();
        }

        match explicit_len {
            Some(len) => Self::read_exact_bytes(uc, ptr, len),
            None => uc.read_string_bytes(ptr as u64, 4096),
        }
    }

    fn write_bytes_to_new_buffer(uc: &mut Unicorn<Win32Context>, data: &[u8]) -> u32 {
        if data.is_empty() {
            return Self::empty_c_string_addr(uc);
        }

        let addr = Self::alloc_zeroed(uc, data.len() + 1);
        let _ = uc.mem_write(addr as u64, data);
        uc.write_u8(addr as u64 + data.len() as u64, 0);
        addr
    }

    fn set_basic_string_bytes(uc: &mut Unicorn<Win32Context>, this_ptr: u32, data: &[u8]) {
        if this_ptr == 0 {
            return;
        }

        let ptr = Self::write_bytes_to_new_buffer(uc, data);
        uc.write_u32(this_ptr as u64 + BASIC_STRING_PTR_OFFSET, ptr);
        uc.write_u32(this_ptr as u64 + BASIC_STRING_LEN_OFFSET, data.len() as u32);
        uc.write_u32(this_ptr as u64 + BASIC_STRING_RES_OFFSET, data.len() as u32);
    }

    fn ensure_basic_string_capacity(
        uc: &mut Unicorn<Win32Context>,
        this_ptr: u32,
        capacity: usize,
        preserve_current: bool,
    ) {
        if this_ptr == 0 {
            return;
        }

        let current = if preserve_current {
            Self::basic_string_bytes(uc, this_ptr)
        } else {
            Vec::new()
        };
        let preserved_len = current.len().min(capacity);
        let new_ptr = if capacity == 0 {
            Self::empty_c_string_addr(uc)
        } else {
            let addr = Self::alloc_zeroed(uc, capacity + 1);
            if preserved_len != 0 {
                let _ = uc.mem_write(addr as u64, &current[..preserved_len]);
            }
            uc.write_u8(addr as u64 + preserved_len as u64, 0);
            addr
        };

        uc.write_u32(this_ptr as u64 + BASIC_STRING_PTR_OFFSET, new_ptr);
        uc.write_u32(
            this_ptr as u64 + BASIC_STRING_LEN_OFFSET,
            preserved_len as u32,
        );
        uc.write_u32(this_ptr as u64 + BASIC_STRING_RES_OFFSET, capacity as u32);
    }

    fn basic_string_subrange(
        uc: &Unicorn<Win32Context>,
        source_ptr: u32,
        offset: u32,
        count: u32,
    ) -> Vec<u8> {
        let bytes = Self::basic_string_bytes(uc, source_ptr);
        let start = (offset as usize).min(bytes.len());
        let end = if count == u32::MAX {
            bytes.len()
        } else {
            start.saturating_add(count as usize).min(bytes.len())
        };
        bytes[start..end].to_vec()
    }

    fn basic_string_replace_range(
        uc: &mut Unicorn<Win32Context>,
        this_ptr: u32,
        pos: usize,
        remove_len: usize,
        replacement: &[u8],
    ) {
        let current = Self::basic_string_bytes(uc, this_ptr);
        let start = pos.min(current.len());
        let end = start.saturating_add(remove_len).min(current.len());

        let mut next = Vec::with_capacity(current.len() + replacement.len());
        next.extend_from_slice(&current[..start]);
        next.extend_from_slice(replacement);
        next.extend_from_slice(&current[end..]);
        Self::set_basic_string_bytes(uc, this_ptr, &next);
    }

    fn read_streambuf_field(uc: &Unicorn<Win32Context>, this_ptr: u32, offset: u64) -> u32 {
        if this_ptr == 0 {
            return 0;
        }
        uc.read_u32(this_ptr as u64 + offset)
    }

    fn write_streambuf_field(
        uc: &mut Unicorn<Win32Context>,
        this_ptr: u32,
        offset: u64,
        value: u32,
    ) {
        if this_ptr != 0 {
            uc.write_u32(this_ptr as u64 + offset, value);
        }
    }

    fn streambuf_file_handle(uc: &Unicorn<Win32Context>, this_ptr: u32) -> u32 {
        Self::read_streambuf_field(uc, this_ptr, STREAMBUF_FILE_HANDLE_OFFSET)
    }

    fn ensure_streambuf_buffer(uc: &mut Unicorn<Win32Context>, this_ptr: u32) -> (u32, usize) {
        let mut buffer_ptr = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_BUFFER_OFFSET);
        let mut capacity =
            Self::read_streambuf_field(uc, this_ptr, STREAMBUF_CAPACITY_OFFSET) as usize;
        if buffer_ptr == 0 || capacity == 0 {
            buffer_ptr = Self::alloc_zeroed(uc, STREAMBUF_HOST_BUFFER_SIZE);
            capacity = STREAMBUF_HOST_BUFFER_SIZE;
            Self::write_streambuf_field(uc, this_ptr, STREAMBUF_BUFFER_OFFSET, buffer_ptr);
            Self::write_streambuf_field(uc, this_ptr, STREAMBUF_CAPACITY_OFFSET, capacity as u32);
        }
        (buffer_ptr, capacity)
    }

    fn open_host_file_by_name(
        uc: &mut Unicorn<Win32Context>,
        raw_filename: &str,
        mode: u32,
    ) -> Option<u32> {
        if raw_filename.is_empty() {
            return None;
        }

        let mut candidates = vec![raw_filename.to_string()];
        if !raw_filename.contains('/') && !raw_filename.contains('\\') {
            candidates.insert(
                0,
                crate::resource_dir()
                    .join(raw_filename)
                    .to_string_lossy()
                    .to_string(),
            );
        }

        let want_read = mode == 0 || (mode & 0x01) != 0;
        let mut want_write = (mode & 0x02) != 0;
        let seek_to_end = (mode & 0x04) != 0;
        let append = (mode & 0x08) != 0;
        let truncate = (mode & 0x10) != 0;

        if append {
            want_write = true;
        }

        for candidate in candidates {
            let mut options = OpenOptions::new();
            options.read(want_read || !want_write);
            options.write(want_write);
            options.append(append);
            options.create(want_write);
            options.truncate(truncate);

            let mut file = match options.open(&candidate) {
                Ok(file) => file,
                Err(_) => continue,
            };

            if seek_to_end {
                let _ = file.seek(SeekFrom::End(0));
            }

            let handle = {
                let context = uc.get_data();
                let handle = context.alloc_handle();
                context.files.lock().unwrap().insert(
                    handle,
                    FileState {
                        file,
                        path: candidate.clone(),
                        eof: false,
                        error: false,
                    },
                );
                handle
            };
            return Some(handle);
        }

        None
    }

    fn open_host_file_from_guest(
        uc: &mut Unicorn<Win32Context>,
        filename_ptr: u32,
        mode: u32,
    ) -> Option<(u32, String)> {
        let filename = if filename_ptr != 0 {
            uc.read_euc_kr(filename_ptr as u64)
        } else {
            String::new()
        };
        Self::open_host_file_by_name(uc, &filename, mode).map(|handle| (handle, filename))
    }

    fn attach_version_dat_fallback(uc: &mut Unicorn<Win32Context>, this_ptr: u32) -> bool {
        if this_ptr == 0 || Self::streambuf_file_handle(uc, this_ptr) != 0 {
            return false;
        }
        if Self::streambuf_available(uc, this_ptr) != 0 {
            return false;
        }

        let mode = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_FILE_MODE_OFFSET);
        let fallback_mode = if mode == 0 { 1 } else { mode };
        let Some(file_handle) = Self::open_host_file_by_name(uc, "version.dat", fallback_mode)
        else {
            return false;
        };

        // 원본 런타임에서는 이 시점에 Version.dat가 이미 연결돼 있어야 합니다.
        // 현재 에뮬레이터는 일부 생성 경로가 누락돼 있어, 빈 filebuf일 때만 국소적으로 보정합니다.
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_FILE_HANDLE_OFFSET, file_handle);
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_FILE_MODE_OFFSET, fallback_mode);
        let _ = Self::refill_streambuf_from_file(uc, this_ptr);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) attached fallback Resources/version.dat",
            this_ptr
        );
        true
    }

    fn close_streambuf_file_handle(uc: &mut Unicorn<Win32Context>, this_ptr: u32) {
        let file_handle = Self::streambuf_file_handle(uc, this_ptr);
        if file_handle == 0 {
            return;
        }

        let context = uc.get_data();
        let mut files = context.files.lock().unwrap();
        files.remove(&file_handle);
        drop(files);
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_FILE_HANDLE_OFFSET, 0);
    }

    fn refill_streambuf_from_file(uc: &mut Unicorn<Win32Context>, this_ptr: u32) -> usize {
        let file_handle = Self::streambuf_file_handle(uc, this_ptr);
        if file_handle == 0 {
            return 0;
        }

        let (buffer_ptr, capacity) = Self::ensure_streambuf_buffer(uc, this_ptr);
        let mut data = vec![0u8; capacity];
        let bytes_read = {
            let context = uc.get_data();
            let mut files = context.files.lock().unwrap();
            if let Some(state) = files.get_mut(&file_handle) {
                match state.file.read(&mut data) {
                    Ok(bytes_read) => {
                        state.eof = bytes_read < capacity;
                        if bytes_read > 0 {
                            state.error = false;
                        }
                        bytes_read
                    }
                    Err(_) => {
                        state.error = true;
                        0
                    }
                }
            } else {
                0
            }
        };

        if bytes_read != 0 {
            let _ = uc.mem_write(buffer_ptr as u64, &data[..bytes_read]);
        }
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET, 0);
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_WRITE_POS_OFFSET, bytes_read as u32);
        bytes_read
    }

    fn prepare_streambuf_read(uc: &mut Unicorn<Win32Context>, this_ptr: u32) {
        if Self::streambuf_available(uc, this_ptr) == 0 {
            let _ = Self::refill_streambuf_from_file(uc, this_ptr);
        }
    }

    fn streambuf_peek_byte(uc: &mut Unicorn<Win32Context>, this_ptr: u32) -> Option<u8> {
        Self::prepare_streambuf_read(uc, this_ptr);
        let buffer_ptr = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_BUFFER_OFFSET);
        let read_pos = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET);
        let available = Self::streambuf_available(uc, this_ptr);
        if buffer_ptr == 0 || available == 0 {
            None
        } else {
            Some(uc.read_u8(buffer_ptr as u64 + read_pos as u64))
        }
    }

    fn streambuf_take_byte(uc: &mut Unicorn<Win32Context>, this_ptr: u32) -> Option<u8> {
        let value = Self::streambuf_peek_byte(uc, this_ptr)?;
        let read_pos = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET);
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET, read_pos + 1);
        Some(value)
    }

    fn write_bytes_to_streambuf(
        uc: &mut Unicorn<Win32Context>,
        this_ptr: u32,
        bytes: &[u8],
    ) -> usize {
        if bytes.is_empty() {
            return 0;
        }

        let file_handle = Self::streambuf_file_handle(uc, this_ptr);
        if file_handle != 0 {
            let context = uc.get_data();
            let mut files = context.files.lock().unwrap();
            if let Some(state) = files.get_mut(&file_handle) {
                return match state.file.write(bytes) {
                    Ok(bytes_written) => {
                        state.error = false;
                        state.eof = false;
                        bytes_written
                    }
                    Err(_) => {
                        state.error = true;
                        0
                    }
                };
            }
            return 0;
        }

        let buffer_ptr = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_BUFFER_OFFSET);
        let capacity = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_CAPACITY_OFFSET);
        let write_pos =
            Self::read_streambuf_field(uc, this_ptr, STREAMBUF_WRITE_POS_OFFSET) as usize;
        if buffer_ptr == 0 || capacity == 0 {
            return 0;
        }

        let writable = bytes.len().min(capacity as usize).saturating_sub(write_pos);
        if writable == 0 {
            return 0;
        }

        let _ = uc.mem_write(buffer_ptr as u64 + write_pos as u64, &bytes[..writable]);
        Self::write_streambuf_field(
            uc,
            this_ptr,
            STREAMBUF_WRITE_POS_OFFSET,
            (write_pos + writable) as u32,
        );
        writable
    }

    fn seek_streambuf_file(
        uc: &mut Unicorn<Win32Context>,
        this_ptr: u32,
        position: SeekFrom,
    ) -> Option<u32> {
        let file_handle = Self::streambuf_file_handle(uc, this_ptr);
        if file_handle == 0 {
            return None;
        }

        let next = {
            let context = uc.get_data();
            let mut files = context.files.lock().unwrap();
            let state = files.get_mut(&file_handle)?;
            let next = state.file.seek(position).ok()? as u32;
            state.error = false;
            state.eof = false;
            next
        };

        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET, 0);
        Self::write_streambuf_field(uc, this_ptr, STREAMBUF_WRITE_POS_OFFSET, 0);
        Some(next)
    }

    fn streambuf_copy_assign(uc: &mut Unicorn<Win32Context>, this_ptr: u32, other_ptr: u32) {
        if this_ptr == 0 || other_ptr == 0 {
            return;
        }

        for offset in [
            STREAMBUF_BUFFER_OFFSET,
            STREAMBUF_CAPACITY_OFFSET,
            STREAMBUF_READ_POS_OFFSET,
            STREAMBUF_WRITE_POS_OFFSET,
            STREAMBUF_LOCALE_OFFSET,
            STREAMBUF_LAST_CHAR_OFFSET,
        ] {
            let value = uc.read_u32(other_ptr as u64 + offset);
            uc.write_u32(this_ptr as u64 + offset, value);
        }
    }

    fn ios_base_copy_assign(uc: &mut Unicorn<Win32Context>, this_ptr: u32, other_ptr: u32) {
        if this_ptr == 0 || other_ptr == 0 {
            return;
        }

        for offset in [IOS_FLAGS_OFFSET, IOS_STATE_OFFSET, IOS_LOCALE_OFFSET] {
            let value = uc.read_u32(other_ptr as u64 + offset);
            uc.write_u32(this_ptr as u64 + offset, value);
        }
    }

    fn basic_ios_copy_assign(uc: &mut Unicorn<Win32Context>, this_ptr: u32, other_ptr: u32) {
        Self::ios_base_copy_assign(uc, this_ptr, other_ptr);
        if this_ptr != 0 && other_ptr != 0 {
            let streambuf_ptr = Self::read_basic_ios_streambuf_ptr(uc, other_ptr);
            Self::write_basic_ios_streambuf_ptr(uc, this_ptr, streambuf_ptr);
        }
    }

    fn basic_ostream_write_bytes(uc: &mut Unicorn<Win32Context>, this_ptr: u32, bytes: &[u8]) {
        if this_ptr == 0 || bytes.is_empty() {
            return;
        }

        let streambuf_ptr = Self::read_basic_ios_streambuf_ptr(uc, this_ptr);
        if streambuf_ptr == 0 {
            return;
        }
        let _ = Self::write_bytes_to_streambuf(uc, streambuf_ptr, bytes);
    }

    fn streambuf_available(uc: &Unicorn<Win32Context>, this_ptr: u32) -> u32 {
        let read_pos = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_READ_POS_OFFSET);
        let write_pos = Self::read_streambuf_field(uc, this_ptr, STREAMBUF_WRITE_POS_OFFSET);
        write_pos.saturating_sub(read_pos)
    }

    fn streambuf_file_eof(uc: &Unicorn<Win32Context>, this_ptr: u32) -> bool {
        let file_handle = Self::streambuf_file_handle(uc, this_ptr);
        if file_handle == 0 {
            return false;
        }

        uc.get_data()
            .files
            .lock()
            .unwrap()
            .get(&file_handle)
            .map(|state| state.eof)
            .unwrap_or(false)
    }

    fn streambuf_return_fpos(
        uc: &mut Unicorn<Win32Context>,
        pos: u32,
        cleanup_without_hidden: usize,
        cleanup_with_hidden: usize,
    ) -> ApiHookResult {
        let ret_ptr = uc.read_arg(0);
        if Self::is_mapped_ptr(uc, ret_ptr) {
            uc.write_u32(ret_ptr as u64, pos);
            uc.write_u32(ret_ptr as u64 + 4, 0);
            ApiHookResult::callee(cleanup_with_hidden, Some(ret_ptr as i32))
        } else {
            ApiHookResult::callee(cleanup_without_hidden, Some(pos as i32))
        }
    }

    /// `MSVCP60.dll`의 데이터/전역 export 주소를 해소합니다.
    ///
    /// 함수 호출이 아닌 전역 객체, vtable, 정적 데이터 심볼만 처리합니다.
    pub fn resolve_export(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<u32> {
        match func_name {
            SYMBOL_BASIC_STRING_NPOS => Some(Self::cached_proxy_export(uc, func_name, |uc| {
                let addr = Self::alloc_zeroed(uc, 4);
                uc.write_u32(addr as u64, u32::MAX);
                crate::emu_log!("[MSVCP60] basic_string::npos resolved to {:#x}", addr);
                addr
            })),
            SYMBOL_BASIC_STRING_NULLSTR_DATA => {
                let addr = Self::empty_c_string_addr(uc);
                crate::emu_log!(
                    "[MSVCP60] basic_string::_Nullstr data resolved to {:#x}",
                    addr
                );
                Some(addr)
            }
            SYMBOL_STD_FPZ => Some(Self::cached_proxy_export(uc, func_name, |uc| {
                let addr = Self::alloc_zeroed(uc, 8);
                crate::emu_log!("[MSVCP60] std::_Fpz resolved to {:#x}", addr);
                addr
            })),
            name if name.starts_with("??_7") || name.starts_with("??_8") => {
                let addr = Self::vtable_export_addr(uc, name);
                crate::emu_log!("[MSVCP60] vtable/vbtable {} resolved to {:#x}", name, addr);
                Some(addr)
            }
            name if GLOBAL_STREAM_OBJECT_SYMBOLS.contains(&name) => {
                let addr = Self::init_global_stream_object(uc, func_name);
                crate::emu_log!(
                    "[MSVCP60] Global object {} resolved to {:#x}",
                    func_name,
                    addr
                );
                Some(addr)
            }
            SYMBOL_LOCALE_GLOBAL_PTR | SYMBOL_LOCALE_CLOCPTR => {
                Some(Self::cached_proxy_export(uc, func_name, |uc| {
                    let addr = Self::alloc_zeroed(uc, 4);
                    let locimp_addr = Self::locale_impl_addr(uc);
                    uc.write_u32(addr as u64, locimp_addr);
                    crate::emu_log!(
                        "[MSVCP60] Global locale ptr {} resolved to {:#x}",
                        func_name,
                        addr
                    );
                    addr
                }))
            }
            SYMBOL_LOCALE_FACET_ID_CNT | SYMBOL_LOCALE_ID_ID_CNT => {
                Some(Self::cached_proxy_export(uc, func_name, |uc| {
                    let addr = Self::alloc_zeroed(uc, 4);
                    crate::emu_log!(
                        "[MSVCP60] Facet ID counter {} resolved to {:#x}",
                        func_name,
                        addr
                    );
                    addr
                }))
            }
            SYMBOL_BASIC_FILEBUF_STATIC_INIT_FLAG => {
                Some(Self::cached_proxy_export(uc, func_name, |uc| {
                    let addr = Self::alloc_zeroed(uc, 4);
                    crate::emu_log!("[MSVCP60] Static init flag resolved to {:#x}", addr);
                    addr
                }))
            }
            _ => None,
        }
    }

    /// 함수명 기준으로 `MSVCP60.dll` API를 처리합니다.
    ///
    /// 초기화 경로에서 실제 호출되는 STL 심볼을 guest 메모리 상태와 함께 최소 구현합니다.
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        let is_cdecl = Self::is_cdecl_symbol(func_name);
        let result = match func_name {
            FN_BASIC_STRING_CTOR_DEFAULT => basic_string::basic_string_ctor_default(uc),
            FN_BASIC_STRING_CTOR_CSTR => basic_string::basic_string_ctor_cstr(uc),
            FN_BASIC_STRING_DTOR => basic_string::basic_string_destructor(uc),
            FN_BASIC_STRING_TIDY => basic_string::basic_string_tidy(uc),
            FN_BASIC_STRING_GROW => basic_string::basic_string_grow(uc),
            FN_BASIC_STRING_COPY => basic_string::basic_string_copy(uc),
            FN_BASIC_STRING_EOS => basic_string::basic_string_eos(uc),
            FN_BASIC_STRING_FREEZE => basic_string::basic_string_freeze(uc),
            FN_BASIC_STRING_SPLIT => basic_string::basic_string_split(uc),
            SYMBOL_BASIC_STRING_ASSIGN_PTR_LEN => basic_string::basic_string_assign_ptr_len(uc),
            FN_BASIC_STRING_ASSIGN_PTR => basic_string::basic_string_assign_ptr(uc),
            FN_BASIC_STRING_ASSIGN_SUBSTR => basic_string::basic_string_assign_substr(uc),
            FN_BASIC_STRING_APPEND_SUBSTR => basic_string::basic_string_append_substr(uc),
            FN_BASIC_STRING_COMPARE_OTHER => basic_string::basic_string_compare_other(uc),
            FN_BASIC_STRING_COMPARE_PTR => basic_string::basic_string_compare_ptr(uc),
            FN_BASIC_STRING_ERASE => basic_string::basic_string_erase(uc),
            FN_BASIC_STRING_REPLACE_REPEAT => basic_string::basic_string_replace_repeat(uc),
            FN_BASIC_STRING_REPLACE_RANGE_PTRS => basic_string::basic_string_replace_range_ptrs(uc),
            FN_BASIC_STRING_RESIZE => basic_string::basic_string_resize(uc),
            FN_BASIC_STRING_SWAP => basic_string::basic_string_swap(uc),
            FN_BASIC_STRING_SUBSTR => basic_string::basic_string_substr(uc),
            FN_BASIC_STRING_C_STR => basic_string::basic_string_c_str(uc),
            FN_BASIC_STRING_END => basic_string::basic_string_end(uc),
            FN_BASIC_STRING_SIZE => basic_string::basic_string_size(uc),
            FN_BASIC_STRING_MAX_SIZE => basic_string::basic_string_max_size(uc),
            SYMBOL_BASIC_STRING_NULLSTR_FN => basic_string::nullstr(uc),

            FN_BASIC_STREAMBUF_COPY_CTOR => streambuf::basic_streambuf_copy_ctor(uc),
            FN_BASIC_STREAMBUF_INIT => streambuf::basic_streambuf_init(uc),
            FN_BASIC_STREAMBUF_INIT_RANGES => streambuf::basic_streambuf_init_ranges(uc),
            FN_BASIC_STREAMBUF_DTOR => streambuf::basic_streambuf_destructor(uc),
            FN_BASIC_STREAMBUF_ASSIGN => streambuf::basic_streambuf_assign(uc),
            FN_BASIC_STREAMBUF_SETG => streambuf::basic_streambuf_setg(uc),
            FN_BASIC_STREAMBUF_SETP => streambuf::basic_streambuf_setp(uc),
            FN_BASIC_STREAMBUF_IMBUE => streambuf::streambuf_imbue(uc),
            FN_BASIC_STREAMBUF_SETBUF => streambuf::basic_streambuf_setbuf(uc),
            FN_BASIC_STREAMBUF_SEEKOFF => streambuf::basic_streambuf_seekoff(uc),
            FN_BASIC_STREAMBUF_SEEKPOS => streambuf::basic_streambuf_seekpos(uc),
            FN_BASIC_STREAMBUF_XSPUTN => streambuf::basic_streambuf_xsputn(uc),
            FN_BASIC_STREAMBUF_XSGETN => streambuf::basic_streambuf_xsgetn(uc),
            FN_BASIC_STREAMBUF_UNDERFLOW => streambuf::basic_streambuf_underflow(uc),
            FN_BASIC_STREAMBUF_UFLOW => streambuf::basic_streambuf_uflow(uc),
            FN_BASIC_STREAMBUF_SHOWMANYC => streambuf::basic_streambuf_showmanyc(uc),
            FN_BASIC_STREAMBUF_PBACKFAIL => streambuf::basic_streambuf_pbackfail(uc),
            FN_BASIC_STREAMBUF_SYNC => streambuf::basic_streambuf_sync(uc),

            FN_BASIC_OSTREAM_COPY_CTOR => ostream::basic_ostream_copy_ctor(uc),
            FN_BASIC_OSTREAM_CTOR3 => ostream::basic_ostream_ctor3(uc),
            FN_BASIC_OSTREAM_CTOR2 => ostream::basic_ostream_ctor2(uc),
            FN_BASIC_OSTREAM_DTOR => ostream::basic_ostream_destructor(uc),
            FN_OSTREAM_INSERT_INT => ostream::ostream_insert_int(uc),
            FN_BASIC_OSTREAM_WRITE => ostream::basic_ostream_write(uc),

            FN_BASIC_ISTREAM_CTOR => istream::basic_istream_ctor(uc),
            FN_BASIC_ISTREAM_DTOR => istream::basic_istream_destructor(uc),
            FN_BASIC_ISTREAM_SEEKG => istream::basic_istream_seekg(uc),
            FN_BASIC_ISTREAM_GETLINE => istream::basic_istream_getline(uc),

            FN_BASIC_IOSTREAM_CTOR => istream::basic_iostream_ctor(uc),
            FN_BASIC_IOSTREAM_DTOR => istream::basic_iostream_destructor(uc),

            FN_BASIC_FILEBUF_CTOR => filebuf::basic_filebuf_ctor(uc),
            FN_BASIC_FILEBUF_INIT => filebuf::basic_filebuf_init(uc),
            FN_BASIC_FILEBUF_OPEN => filebuf::basic_filebuf_open(uc),
            FN_BASIC_FILEBUF_INITCVT => filebuf::basic_filebuf_initcvt(uc),
            FN_BASIC_FILEBUF_DTOR => filebuf::basic_filebuf_destructor(uc),
            FN_BASIC_IFSTREAM_VBASE_DTOR => istream::basic_ifstream_vbase_dtor(uc),

            FN_BASIC_IOS_CTOR => ios::basic_ios_ctor(uc),
            FN_BASIC_IOS_CLEAR => ios::basic_ios_clear(uc),
            FN_BASIC_IOS_SETSTATE => ios::basic_ios_setstate(uc),
            FN_BASIC_IOS_INIT => ios::basic_ios_init(uc),
            FN_BASIC_IOS_WIDEN => ios::basic_ios_widen(uc),
            FN_BASIC_IOS_DTOR => ios::basic_ios_destructor(uc),
            FN_BASIC_IOS_ASSIGN => ios::basic_ios_assign(uc),

            FN_IOS_BASE_CTOR => ios::ios_base_ctor(uc),
            FN_IOS_BASE_DTOR => ios::ios_base_dtor(uc),
            FN_IOS_BASE_ASSIGN => ios::ios_base_assign(uc),
            FN_IOS_BASE_CLEAR => ios::ios_base_clear(uc),
            FN_IOS_BASE_COPYFMT => ios::ios_base_copyfmt(uc),
            FN_IOS_BASE_GETLOC => ios::ios_base_getloc(uc),
            FN_IOS_BASE_INIT => ios::ios_base_init(uc),
            FN_IOS_BASE_INIT_CTOR => ios::ios_base_init_ctor(uc),
            FN_IOS_BASE_INIT_DTOR => ios::ios_base_init_dtor(uc),

            FN_LOCALE_INIT => ios::locale_init(uc),
            FN_LOCALE_CTOR => ios::locale_ctor(uc),
            FN_LOCALE_DTOR => ios::locale_destructor(uc),
            FN_LOCALE_ASSIGN => ios::locale_assign(uc),
            FN_LOCALE_FACET_INCREF => ios::locale_facet_incref(uc),
            FN_LOCALE_FACET_DECREF => ios::locale_facet_decref(uc),

            FN_STRSTREAMBUF_INIT => streambuf::streambuf_init_strstream(uc),

            FN_WINIT_CTOR => ios::winit_ctor(uc),
            FN_WINIT_DTOR => ios::winit_dtor(uc),
            FN_LOCKIT_CTOR => ios::lockit_ctor(uc),
            FN_LOCKIT_DTOR => ios::lockit_dtor(uc),

            FN_STD_OSTREAM_INSERT_CSTR => ostream::ostream_insert_cstr(uc),
            FN_STD_OSTREAM_INSERT_CHAR => ostream::ostream_insert_char(uc),
            SYMBOL_STD_FLUSH => ostream::ostream_flush(uc),
            FN_STD_ENDL => ostream::ostream_endl(uc),
            FN_STD_XLEN => ios::xlen(uc),
            FN_STD_XRAN => ios::xran(uc),
            FN_STD_XOFF => ios::xoff(uc),
            FN_STD_FIOPEN => filebuf::fiopen(uc),

            FN_BASIC_OFSTREAM_CTOR => ostream::basic_ofstream_ctor(uc),
            FN_BASIC_OFSTREAM_DTOR => ostream::basic_ofstream_dtor(uc),
            FN_BASIC_FSTREAM_DTOR => ostream::basic_fstream_dtor(uc),
            FN_BASIC_ISTREAM_EXTRACT_UNSIGNED_SHORT => {
                istream::basic_istream_extract_unsigned_short(uc)
            }

            _ => {
                crate::emu_log!("[!] MSVCP60 Unhandled: {}", func_name);
                let this_ptr = Self::this_ptr(uc);
                Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
            }
        };

        if is_cdecl {
            result.map(|mut value| {
                value.cleanup = StackCleanup::Caller;
                value
            })
        } else {
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helper::UnicornHelper;
    use std::fs;
    use unicorn_engine::{Arch, Mode};

    fn new_test_uc() -> Unicorn<'static, Win32Context> {
        let mut uc =
            Unicorn::new_with_data(Arch::X86, Mode::MODE_32, Win32Context::new(None)).unwrap();
        uc.setup(None, None).unwrap();
        uc
    }

    fn write_call_frame(uc: &mut Unicorn<Win32Context>, args: &[u32]) {
        let esp = uc.reg_read(RegisterX86::ESP).unwrap() as u32;
        uc.write_u32(esp as u64, 0xDEAD_BEEF);
        for (index, value) in args.iter().enumerate() {
            uc.write_u32(esp as u64 + 4 + (index as u64 * 4), *value);
        }
    }

    #[test]
    fn proxy_cache_key_prefixes_dll_name() {
        assert_eq!(
            MSVCP60::proxy_cache_key(SYMBOL_STD_COUT),
            format!("MSVCP60.dll!{}", SYMBOL_STD_COUT)
        );
    }

    #[test]
    fn cdecl_detection_matches_msvc_mangling() {
        assert!(MSVCP60::is_cdecl_symbol(SYMBOL_STD_FLUSH));
        assert!(!MSVCP60::is_cdecl_symbol(
            SYMBOL_BASIC_STRING_ASSIGN_PTR_LEN
        ));
    }

    #[test]
    #[cfg_attr(
        target_arch = "aarch64",
        ignore = "cargo test 러너에서 Unicorn 초기화가 SIGILL을 유발함"
    )]
    fn resolve_export_reuses_cached_global_addresses() {
        let mut uc = new_test_uc();

        let vtable_a = MSVCP60::resolve_export(&mut uc, BASIC_OSTREAM_VTABLE).unwrap();
        let vtable_b = MSVCP60::resolve_export(&mut uc, BASIC_OSTREAM_VTABLE).unwrap();
        assert_eq!(vtable_a, vtable_b);

        let global_a = MSVCP60::resolve_export(&mut uc, SYMBOL_LOCALE_GLOBAL_PTR).unwrap();
        let global_b = MSVCP60::resolve_export(&mut uc, SYMBOL_LOCALE_GLOBAL_PTR).unwrap();
        assert_eq!(global_a, global_b);
        assert_ne!(uc.read_u32(global_a as u64), 0);
    }

    #[test]
    #[cfg_attr(
        target_arch = "aarch64",
        ignore = "cargo test 러너에서 Unicorn 초기화가 SIGILL을 유발함"
    )]
    fn basic_string_operations_update_guest_layout() {
        let mut uc = new_test_uc();
        let string_a = MSVCP60::alloc_zeroed(&mut uc, 32);
        let string_b = MSVCP60::alloc_zeroed(&mut uc, 32);
        let string_sub = MSVCP60::alloc_zeroed(&mut uc, 32);
        let hello = uc.alloc_str("abcd");
        let other = uc.alloc_str("xyz");

        uc.reg_write(RegisterX86::ECX, string_a as u64).unwrap();
        write_call_frame(&mut uc, &[0]);
        MSVCP60::handle(&mut uc, FN_BASIC_STRING_CTOR_DEFAULT).unwrap();

        uc.reg_write(RegisterX86::ECX, string_a as u64).unwrap();
        write_call_frame(&mut uc, &[hello, 4]);
        MSVCP60::handle(&mut uc, SYMBOL_BASIC_STRING_ASSIGN_PTR_LEN).unwrap();
        assert_eq!(MSVCP60::basic_string_bytes(&uc, string_a), b"abcd");

        uc.reg_write(RegisterX86::ECX, string_b as u64).unwrap();
        write_call_frame(&mut uc, &[other, 0]);
        MSVCP60::handle(&mut uc, FN_BASIC_STRING_CTOR_CSTR).unwrap();

        uc.reg_write(RegisterX86::ECX, string_a as u64).unwrap();
        write_call_frame(&mut uc, &[string_b, 1, 2]);
        MSVCP60::handle(&mut uc, FN_BASIC_STRING_APPEND_SUBSTR).unwrap();
        assert_eq!(MSVCP60::basic_string_bytes(&uc, string_a), b"abcdyz");

        uc.reg_write(RegisterX86::ECX, string_a as u64).unwrap();
        write_call_frame(&mut uc, &[1, 2, 3, b'Q' as u32]);
        MSVCP60::handle(&mut uc, FN_BASIC_STRING_REPLACE_REPEAT).unwrap();
        assert_eq!(MSVCP60::basic_string_bytes(&uc, string_a), b"aQQQdyz");

        uc.reg_write(RegisterX86::ECX, string_a as u64).unwrap();
        write_call_frame(&mut uc, &[string_sub, 2, 3]);
        let substr_result = MSVCP60::handle(&mut uc, FN_BASIC_STRING_SUBSTR).unwrap();
        assert_eq!(substr_result.cleanup, StackCleanup::Callee(3));
        assert_eq!(substr_result.return_value, Some(string_sub as i32));
        assert_eq!(MSVCP60::basic_string_bytes(&uc, string_sub), b"QQd");

        uc.reg_write(RegisterX86::ECX, string_a as u64).unwrap();
        write_call_frame(&mut uc, &[]);
        let c_str_result = MSVCP60::handle(&mut uc, FN_BASIC_STRING_C_STR).unwrap();
        let c_str_ptr = c_str_result.return_value.unwrap() as u32;
        assert_eq!(uc.read_string(c_str_ptr as u64), "aQQQdyz");

        uc.reg_write(RegisterX86::ECX, string_a as u64).unwrap();
        write_call_frame(&mut uc, &[]);
        let size_result = MSVCP60::handle(&mut uc, FN_BASIC_STRING_SIZE).unwrap();
        assert_eq!(size_result.return_value, Some(7));
    }

    #[test]
    #[cfg_attr(
        target_arch = "aarch64",
        ignore = "cargo test 러너에서 Unicorn 초기화가 SIGILL을 유발함"
    )]
    fn cdecl_and_thiscall_cleanup_are_distinguished() {
        let mut uc = new_test_uc();
        let string_ptr = MSVCP60::alloc_zeroed(&mut uc, 32);

        uc.reg_write(RegisterX86::ECX, string_ptr as u64).unwrap();
        write_call_frame(&mut uc, &[0, 0]);
        let assign_result = MSVCP60::handle(&mut uc, SYMBOL_BASIC_STRING_ASSIGN_PTR_LEN).unwrap();
        assert_eq!(assign_result.cleanup, StackCleanup::Callee(2));

        write_call_frame(&mut uc, &[0x1234]);
        let flush_result = MSVCP60::handle(&mut uc, SYMBOL_STD_FLUSH).unwrap();
        assert_eq!(flush_result.cleanup, StackCleanup::Caller);
    }

    #[test]
    #[cfg_attr(
        target_arch = "aarch64",
        ignore = "cargo test 러너에서 Unicorn 초기화가 SIGILL을 유발함"
    )]
    fn locale_singletons_stay_stable_across_init_and_refcounting() {
        let mut uc = new_test_uc();
        let global_ptr = MSVCP60::resolve_export(&mut uc, SYMBOL_LOCALE_GLOBAL_PTR).unwrap();
        let initial_locimp = uc.read_u32(global_ptr as u64);

        write_call_frame(&mut uc, &[]);
        let init_result = MSVCP60::handle(&mut uc, FN_LOCALE_INIT).unwrap();
        assert_eq!(init_result.cleanup, StackCleanup::Caller);
        assert_eq!(init_result.return_value, Some(initial_locimp as i32));

        let facet = MSVCP60::alloc_zeroed(&mut uc, 0x10);
        uc.write_u32(facet as u64 + FACET_REFCOUNT_OFFSET, 1);
        uc.reg_write(RegisterX86::ECX, facet as u64).unwrap();
        write_call_frame(&mut uc, &[]);
        MSVCP60::handle(&mut uc, FN_LOCALE_FACET_INCREF).unwrap();
        assert_eq!(uc.read_u32(facet as u64 + FACET_REFCOUNT_OFFSET), 2);

        uc.reg_write(RegisterX86::ECX, facet as u64).unwrap();
        write_call_frame(&mut uc, &[]);
        let decref_result = MSVCP60::handle(&mut uc, FN_LOCALE_FACET_DECREF).unwrap();
        assert_eq!(uc.read_u32(facet as u64 + FACET_REFCOUNT_OFFSET), 1);
        assert_eq!(decref_result.return_value, Some(facet as i32));

        let global_again = MSVCP60::resolve_export(&mut uc, SYMBOL_LOCALE_GLOBAL_PTR).unwrap();
        assert_eq!(global_ptr, global_again);
        assert_eq!(uc.read_u32(global_again as u64), initial_locimp);
    }

    #[test]
    #[cfg_attr(
        target_arch = "aarch64",
        ignore = "cargo test 러너에서 Unicorn 초기화가 SIGILL을 유발함"
    )]
    fn locale_ctor_and_streambuf_init_fill_default_layout() {
        let mut uc = new_test_uc();
        let locale_obj = MSVCP60::alloc_zeroed(&mut uc, LOCALE_OBJECT_SIZE);
        let streambuf_obj = MSVCP60::alloc_zeroed(&mut uc, STREAMBUF_OBJECT_SIZE);

        uc.reg_write(RegisterX86::ECX, locale_obj as u64).unwrap();
        write_call_frame(&mut uc, &[]);
        let ctor_result = MSVCP60::handle(&mut uc, FN_LOCALE_CTOR).unwrap();
        assert_eq!(ctor_result.cleanup, StackCleanup::Callee(0));
        assert_eq!(ctor_result.return_value, Some(locale_obj as i32));
        assert_ne!(MSVCP60::read_locale_impl(&uc, locale_obj), 0);
        assert_eq!(uc.read_u32(locale_obj as u64 + 0x1c), 0);

        uc.reg_write(RegisterX86::ECX, streambuf_obj as u64)
            .unwrap();
        write_call_frame(&mut uc, &[]);
        let init_result = MSVCP60::handle(&mut uc, FN_BASIC_STREAMBUF_INIT).unwrap();
        assert_eq!(init_result.cleanup, StackCleanup::Callee(0));
        assert_eq!(
            uc.read_u32(streambuf_obj as u64),
            MSVCP60::vtable_export_addr(&mut uc, BASIC_STREAMBUF_VTABLE)
        );
        assert_ne!(
            uc.read_u32(streambuf_obj as u64 + STREAMBUF_LOCALE_OFFSET),
            0
        );
    }

    #[test]
    #[cfg_attr(
        target_arch = "aarch64",
        ignore = "cargo test 러너에서 Unicorn 초기화가 SIGILL을 유발함"
    )]
    fn filebuf_open_and_extract_unsigned_short_reads_host_file() {
        let mut uc = new_test_uc();
        let temp_path = std::env::temp_dir().join("emul4leaf_msvcp60_extract_u16.txt");
        fs::write(&temp_path, b" 4321 ").unwrap();

        let filebuf = MSVCP60::alloc_zeroed(&mut uc, STREAMBUF_OBJECT_SIZE);
        let istream = MSVCP60::alloc_zeroed(&mut uc, STREAM_OBJECT_SIZE);
        let out_value = MSVCP60::alloc_zeroed(&mut uc, 2);
        let path_ptr = uc.alloc_str(temp_path.to_string_lossy().as_ref());

        uc.reg_write(RegisterX86::ECX, filebuf as u64).unwrap();
        write_call_frame(&mut uc, &[0]);
        MSVCP60::handle(&mut uc, FN_BASIC_FILEBUF_CTOR).unwrap();

        uc.reg_write(RegisterX86::ECX, filebuf as u64).unwrap();
        write_call_frame(&mut uc, &[path_ptr, 1]);
        let open_result = MSVCP60::handle(&mut uc, FN_BASIC_FILEBUF_OPEN).unwrap();
        assert_eq!(open_result.return_value, Some(filebuf as i32));

        uc.reg_write(RegisterX86::ECX, istream as u64).unwrap();
        write_call_frame(&mut uc, &[filebuf, 0]);
        MSVCP60::handle(&mut uc, FN_BASIC_ISTREAM_CTOR).unwrap();

        uc.reg_write(RegisterX86::ECX, istream as u64).unwrap();
        write_call_frame(&mut uc, &[out_value]);
        let extract_result =
            MSVCP60::handle(&mut uc, FN_BASIC_ISTREAM_EXTRACT_UNSIGNED_SHORT).unwrap();
        assert_eq!(extract_result.return_value, Some(istream as i32));
        assert_eq!(uc.read_u16(out_value as u64), 4321);
        assert_eq!(uc.read_u32(istream as u64 + IOS_STATE_OFFSET) & 0x6, 0);
    }

    #[test]
    #[cfg_attr(
        target_arch = "aarch64",
        ignore = "cargo test 러너에서 Unicorn 초기화가 SIGILL을 유발함"
    )]
    fn extract_unsigned_short_uses_version_dat_fallback_for_empty_filebuf() {
        let mut uc = new_test_uc();
        let filebuf = MSVCP60::alloc_zeroed(&mut uc, STREAMBUF_OBJECT_SIZE);
        let istream = MSVCP60::alloc_zeroed(&mut uc, STREAM_OBJECT_SIZE);
        let out_value = MSVCP60::alloc_zeroed(&mut uc, 2);

        uc.reg_write(RegisterX86::ECX, filebuf as u64).unwrap();
        write_call_frame(&mut uc, &[0]);
        MSVCP60::handle(&mut uc, FN_BASIC_FILEBUF_CTOR).unwrap();

        uc.reg_write(RegisterX86::ECX, istream as u64).unwrap();
        write_call_frame(&mut uc, &[filebuf, 0]);
        MSVCP60::handle(&mut uc, FN_BASIC_ISTREAM_CTOR).unwrap();

        uc.reg_write(RegisterX86::ECX, istream as u64).unwrap();
        write_call_frame(&mut uc, &[out_value]);
        let extract_result =
            MSVCP60::handle(&mut uc, FN_BASIC_ISTREAM_EXTRACT_UNSIGNED_SHORT).unwrap();
        assert_eq!(extract_result.return_value, Some(istream as i32));
        assert_eq!(uc.read_u16(out_value as u64), 54);
        assert_eq!(uc.read_u32(istream as u64 + IOS_STATE_OFFSET) & 0x6, 0);
        assert_ne!(MSVCP60::streambuf_file_handle(&uc, filebuf), 0);
    }
}
