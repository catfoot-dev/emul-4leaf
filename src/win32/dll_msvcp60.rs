use unicorn_engine::Unicorn;

use crate::helper::UnicornHelper;
use crate::win32::{ApiHookResult, Win32Context};

/// `MSVCP60.dll` 프록시 구현 모듈
///
/// C++ 표준 라이브러리(STL) 관련 특정 함수 및 문자열 처리를 가상화하여 호환성을 확보
pub struct DllMSVCP60;

impl DllMSVCP60 {
    // API: std::basic_string<char>::basic_string(const allocator<char>&)
    // 역할: std::string의 기본 생성자. 빈 문자열로 초기화
    pub fn basic_string_constructor(
        uc: &mut Unicorn<Win32Context>,
    ) -> Option<ApiHookResult> {
        // 기본 생성자: this->_Ptr = static empty string, this->_Len = 0, this->_Res = 0
        let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
        let allocator = uc.read_arg(0);
        if this_ptr != 0 {
            // basic_string layout: _Allocator(4), _Ptr(4), _Len(4), _Res(4)
            let empty_str = uc.alloc_str("");
            uc.write_u32(this_ptr as u64 + 4, empty_str); // _Ptr
            uc.write_u32(this_ptr as u64 + 8, 0); // _Len
            uc.write_u32(this_ptr as u64 + 12, 0); // _Res
        }
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::basic_string({:#x}) -> (this={:#x})",
            this_ptr,
            allocator,
            this_ptr
        );
        Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
    }

    // API: std::basic_string<char>::_Tidy(bool)
    // 역할: 문자열 버퍼를 해제하고 초기 상태로 되돌림
    pub fn basic_string_tidy(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
        let b = uc.read_arg(0);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::_Tidy({}) -> VOID",
            this_ptr,
            b
        );
        Some(ApiHookResult::callee(1, None))
    }

    // API: std::basic_string<char>::_Grow(size_t, bool)
    // 역할: 문자열 버퍼 크기를 확장
    pub fn basic_string_grow(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
        let n = uc.read_arg(0);
        let b = uc.read_arg(1);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::_Grow({}, {}) -> BOOL 1",
            this_ptr,
            n,
            b
        );
        Some(ApiHookResult::callee(2, Some(1))) // TRUE = 성공
    }

    // API: std::basic_string<char>::assign(const char*, size_t)
    // 역할: 문자열에 특정 포인터의 데이터를 버퍼만큼 할당
    pub fn basic_string_assign_param2(
        uc: &mut Unicorn<Win32Context>,
    ) -> Option<ApiHookResult> {
        let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
        let ptr = uc.read_arg(0);
        let len = uc.read_arg(1);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::assign({:#x}, {}) -> (this={:#x})",
            this_ptr,
            ptr,
            len,
            this_ptr
        );
        Some(ApiHookResult::callee(2, Some(this_ptr as i32)))
    }

    // API: std::basic_string<char>::assign(const basic_string&, size_t, size_t)
    // 역할: 다른 string 객체의 일부를 할당
    pub fn basic_string_assign_param3(
        uc: &mut Unicorn<Win32Context>,
    ) -> Option<ApiHookResult> {
        let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
        let other_ptr = uc.read_arg(0);
        let offset = uc.read_arg(1);
        let count = uc.read_arg(2);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::assign({:#x}, {:#x}, {:#x}) -> (this={:#x})",
            this_ptr,
            other_ptr,
            offset,
            count,
            this_ptr
        );
        Some(ApiHookResult::callee(3, Some(this_ptr as i32)))
    }

    // API: std::basic_string<char>::erase(size_t, size_t)
    // 역할: 문자열의 일부를 삭제
    pub fn basic_string_erase(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
        let offset = uc.read_arg(0);
        let count = uc.read_arg(1);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::erase({:#x}, {:#x}) -> (this={:#x})",
            this_ptr,
            offset,
            count,
            this_ptr
        );
        Some(ApiHookResult::callee(2, Some(this_ptr as i32)))
    }

    /// `MSVCP60.dll`에서 데이터로 취급되어야 할 심볼들을 메모리에 할당하고 주소를 반환합니다.
    pub fn resolve_export(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<u32> {
        match func_name {
            // 정적 npos = static const size_t = 0xFFFFFFFF
            "?npos@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@2IB" => {
                let addr = uc.malloc(4);
                uc.write_u32(addr, 0xFFFFFFFF);
                crate::emu_log!("[MSVCP60] basic_string::npos resolved to {:#x}", addr);
                Some(addr as u32)
            }
            // _Nullstr
            "?_C@?1??_Nullstr@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@CAPBDXZ@4DB" =>
            {
                let addr = uc.alloc_str("");
                crate::emu_log!("[MSVCP60] basic_string::_Nullstr resolved to {:#x}", addr);
                Some(addr as u32)
            }
            // static _Fpz (zero fpos)
            "?_Fpz@std@@3_JB" => {
                let addr = uc.malloc(8);
                uc.write_u32(addr, 0);
                uc.write_u32(addr + 4, 0);
                crate::emu_log!("[MSVCP60] std::_Fpz resolved to {:#x}", addr);
                Some(addr as u32)
            }
            // VTable / Vbtable pointers
            name if name.starts_with("??_7") || name.starts_with("??_8") => {
                let addr = uc.malloc(64); // Fake vtable
                crate::emu_log!("[MSVCP60] vtable/vbtable {} resolved to {:#x}", name, addr);
                Some(addr as u32)
            }
            // cin, cout, cerr, clog (Global objects)
            "?cin@std@@3V?$basic_istream@DU?$char_traits@D@std@@@1@A"
            | "?cout@std@@3V?$basic_ostream@DU?$char_traits@D@std@@@1@A"
            | "?cerr@std@@3V?$basic_ostream@DU?$char_traits@D@std@@@1@A"
            | "?clog@std@@3V?$basic_ostream@DU?$char_traits@D@std@@@1@A" => {
                let addr = uc.malloc(128); // Object size placeholder
                crate::emu_log!(
                    "[MSVCP60] Global object {} resolved to {:#x}",
                    func_name,
                    addr
                );
                Some(addr as u32)
            }
            // Global locale implementation pointer (important for 0xC3C3C3C7 fix)
            "?_Global@_Locimp@locale@std@@0PAV123@A"
            | "?_Clocptr@_Locimp@locale@std@@0PAV123@A" => {
                let addr = uc.malloc(4);
                uc.write_u32(addr, 0); // Initialize as NULL
                crate::emu_log!(
                    "[MSVCP60] Global locale ptr {} resolved to {:#x}",
                    func_name,
                    addr
                );
                Some(addr as u32)
            }
            // Facet ID counter
            "?_Id_cnt@facet@locale@std@@0HA" => {
                let addr = uc.malloc(4);
                uc.write_u32(addr, 0);
                crate::emu_log!(
                    "[MSVCP60] Facet ID counter {} resolved to {:#x}",
                    func_name,
                    addr
                );
                Some(addr as u32)
            }
            // Static init flag in basic_filebuf::_Init
            "?_Stinit@?1??_Init@?$basic_filebuf@DU?$char_traits@D@std@@@std@@IAEXPAU_iobuf@@W4_Initfl@23@@Z@4HA" =>
            {
                let addr = uc.malloc(4);
                uc.write_u32(addr, 0);
                crate::emu_log!("[MSVCP60] Static init flag resolved to {:#x}", addr);
                Some(addr as u32)
            }
            _ => None,
        }
    }

    /// 함수명 기준 `MSVCP60.dll` API 구현체
    ///
    /// 처리를 성공했다면 스택 보정값과 리턴값을 포함한 `ApiHookResult`를 반환
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        // MSVCP60.dll은 Visual C++ 6.0의 C++ 표준 라이브러리 (STL)
        // mangled name 에서 호출 규약을 판별
        let is_cdecl = func_name.contains("@YA") || func_name.contains("@Y?A");
        let result = match func_name {
            // =========================================================
            // basic_string<char> (std::string)
            // =========================================================
            "??0?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAE@ABV?$allocator@D@1@@Z" => {
                Self::basic_string_constructor(uc)
            }
            "?_Tidy@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@AAEX_N@Z" => {
                Self::basic_string_tidy(uc)
            }
            "?_Grow@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@AAE_NI_N@Z" => {
                Self::basic_string_grow(uc)
            }
            "?assign@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEAAV12@PBDI@Z" => {
                Self::basic_string_assign_param2(uc)
            }
            "?assign@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEAAV12@ABV12@II@Z" => {
                Self::basic_string_assign_param3(uc)
            }
            "?erase@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEAAV12@II@Z" => {
                Self::basic_string_erase(uc)
            }

            // _Xoff
            "?_Xoff@std@@YAXXZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!("[MSVCP60] (this={:#x}) std::_Xoff()", this_ptr);
                Some(ApiHookResult::callee(0, None))
            }

            // _Xlen
            "?_Xlen@std@@YAXXZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) std::_Xlen() [throw length_error]",
                    this_ptr
                );
                Some(ApiHookResult::callee(0, None))
            }

            // =========================================================
            // iostream / fstream / streambuf
            // =========================================================

            // API: std::basic_ostream<char>& operator<<(int)
            // 역할: 정수 값을 출력 스트림에 삽입
            "??6?$basic_ostream@DU?$char_traits@D@std@@@std@@QAEAAV01@H@Z" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                let val = uc.read_arg(0) as i32;
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_ostream::operator<<({}) -> (this={:#x})",
                    this_ptr,
                    val,
                    this_ptr
                );
                // 실제 스트림 버퍼에 쓰는 대신 로그에 출력하는 것으로 갈음
                Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
            }

            // basic_ostream constructor (3 arguments)
            "??0?$basic_ostream@DU?$char_traits@D@std@@@std@@QAE@PAV?$basic_streambuf@DU?$char_traits@D@std@@@1@_N1@Z" =>
            {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                let buf = uc.read_arg(0);
                let b1 = uc.read_arg(1);
                let b2 = uc.read_arg(2);
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_ostream::basic_ostream({:#x}, {}, {}) -> (this={:#x})",
                    this_ptr,
                    buf,
                    b1,
                    b2,
                    this_ptr
                );
                Some(ApiHookResult::callee(3, Some(this_ptr as i32)))
            }

            // basic_ostream constructor (2 arguments)
            "??0?$basic_ostream@DU?$char_traits@D@std@@@std@@QAE@PAV?$basic_streambuf@DU?$char_traits@D@std@@@1@_N@Z" =>
            {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                let buf = uc.read_arg(0);
                let b1 = uc.read_arg(1);
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_ostream::basic_ostream({:#x}, {}) -> (this={:#x})",
                    this_ptr,
                    buf,
                    b1,
                    this_ptr
                );
                Some(ApiHookResult::callee(2, Some(this_ptr as i32)))
            }

            // basic_string::operator=(const char*)
            "?assign@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEAAV12@PBD@Z" =>
            {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                let str_ptr = uc.read_arg(0);
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_string::operator=({:#x}) -> (this={:#x})",
                    this_ptr,
                    str_ptr,
                    this_ptr
                );
                Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
            }

            // API: std::basic_ios<char>::clear(iostate, bool)
            // 역할: 스트림의 오류 상태를 설정
            "?clear@?$basic_ios@DU?$char_traits@D@std@@@std@@QAEXH_N@Z" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                let state = uc.read_arg(0);
                let _b = uc.read_arg(1);
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_ios::clear({})",
                    this_ptr,
                    state
                );
                // MSVC 6.0 basic_ios layout guess: iostate at +16
                if this_ptr != 0 {
                    uc.write_u32(this_ptr as u64 + 16, state);
                }
                Some(ApiHookResult::callee(2, None))
            }

            // basic_ios::init(streambuf*, bool)
            "?init@?$basic_ios@DU?$char_traits@D@std@@@std@@IAEXPAV?$basic_streambuf@DU?$char_traits@D@std@@@2@_N@Z" =>
            {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                let buf = uc.read_arg(0);
                let _b = uc.read_arg(1);
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_ios::init({:#x})",
                    this_ptr,
                    buf
                );
                // MSVC 6.0 basic_ios: streambuf* at +4
                if this_ptr != 0 {
                    uc.write_u32(this_ptr as u64 + 4, buf);
                }
                Some(ApiHookResult::callee(2, None))
            }

            // API: void std::ios_base::_Init()
            // 역할: C++ 표준 라이브러리의 ios_base를 초기화
            "?_Init@ios_base@std@@IAEXXZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!(
                    "[MSVCRT] (this={:#x}) std::ios_base::_Init() -> void",
                    this_ptr
                );
                Some(ApiHookResult::callee(0, None))
            }

            // API: _Locimp * __cdecl std::locale::_Init()
            // 역할: C++ 표준 라이브러리의 locale을 초기화
            "?_Init@locale@std@@CAPAV_Locimp@12@XZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!(
                    "[MSVCRT] (this={:#x}) std::locale::_Init() -> void",
                    this_ptr
                );
                Some(ApiHookResult::callee(0, None))
            }

            // API: void __thiscall std::strstreambuf::_Init(int, char* buffer, size_t size, char* end)
            // 역할: C++ 표준 라이브러리의 strstreambuf를 초기화
            "?_Init@strstreambuf@std@@IAEXHPAD0H@Z" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                let arg1 = uc.read_arg(0);
                let arg2 = uc.read_arg(1);
                let arg3 = uc.read_arg(2);
                let arg4 = uc.read_arg(3);
                crate::emu_log!(
                    "[MSVCRT] (this={:#x}) std::strstreambuf::_Init({:#x}, {:#x}, {:#x}, {:#x}) -> void",
                    this_ptr,
                    arg1,
                    arg2,
                    arg3,
                    arg4
                );
                Some(ApiHookResult::callee(4, None))
            }

            // basic_fstream destructor sequence
            "??_D?$basic_fstream@DU?$char_traits@D@std@@@std@@QAEXXZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_fstream::~basic_fstream() -> VOID",
                    this_ptr
                );
                Some(ApiHookResult::callee(0, None))
            }

            // API: std::basic_ofstream<char>::basic_ofstream()
            // 역할: 파일 출력 스트림 객체 생성
            "??0?$basic_ofstream@DU?$char_traits@D@std@@@std@@QAE@XZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_ofstream::basic_ofstream() -> (this={:#x})",
                    this_ptr,
                    this_ptr
                );
                Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
            }

            "??_D?$basic_ofstream@DU?$char_traits@D@std@@@std@@QAEXXZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_ofstream::~basic_ofstream() -> VOID",
                    this_ptr
                );
                Some(ApiHookResult::callee(0, None))
            }

            // API: std::basic_istream<char>::seekg(fpos)
            // 역할: 입력 스트림의 읽기 위치를 이동
            "?seekg@?$basic_istream@DU?$char_traits@D@std@@@std@@QAEAAV12@V?$fpos@H@2@@Z" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                let pos = uc.read_arg(0);
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_istream::seekg({:#x}) -> (this={:#x})",
                    this_ptr,
                    pos,
                    this_ptr
                );
                Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
            }

            // API: std::basic_istream<char>::getline(char*, int, char)
            // 역할: 입력 스트림에서 한 줄을 읽음
            "?getline@?$basic_istream@DU?$char_traits@D@std@@@std@@QAEAAV12@PADHD@Z" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                let buf_addr = uc.read_arg(0);
                let count = uc.read_arg(1);
                let delim = uc.read_arg(2) as u8;

                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_istream::getline({:#x}, {}, '{}')",
                    this_ptr,
                    buf_addr,
                    count,
                    delim as char
                );

                if buf_addr != 0 && count > 0 {
                    // 현재는 실제 입력 장치가 없으므로 빈 문자열(또는 EOF 상태)로 처리
                    // 버퍼를 NULL로 초기화
                    uc.write_u8(buf_addr as u64, 0);

                    // ios 상태를 조작하여 failbit/eofbit을 설정할 필요가 있을 수 있음
                    // (this + offset)에 접근하여 상태 비트 업데이트 구현 가능
                }

                Some(ApiHookResult::callee(3, Some(this_ptr as i32)))
            }

            // basic_istream constructor
            "??0?$basic_istream@DU?$char_traits@D@std@@@std@@QAE@PAV?$basic_streambuf@DU?$char_traits@D@std@@@1@_N@Z" =>
            {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                let buf = uc.read_arg(0);
                let b = uc.read_arg(1);
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_istream::basic_istream({:#x}, {}) -> (this={:#x})",
                    this_ptr,
                    buf,
                    b,
                    this_ptr
                );
                Some(ApiHookResult::callee(2, Some(this_ptr as i32)))
            }

            // API: std::basic_filebuf<char>::open(const char*, int)
            // 역할: 파일을 오픈하고 버퍼에 연결
            "?open@?$basic_filebuf@DU?$char_traits@D@std@@@std@@QAEPAV12@PBDH@Z" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                let filename = uc.read_arg(0);
                let mode = uc.read_arg(1);
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_filebuf::open({:#x}, {}) -> (this={:#x})",
                    this_ptr,
                    filename,
                    mode,
                    this_ptr
                );
                Some(ApiHookResult::callee(2, Some(this_ptr as i32))) // NULL = 실패
            }

            // basic_filebuf constructor
            "??0?$basic_filebuf@DU?$char_traits@D@std@@@std@@QAE@PAU_iobuf@@@Z" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                let file_ptr = uc.read_arg(0);
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_filebuf::basic_filebuf({:#x}) -> (this={:#x})",
                    this_ptr,
                    file_ptr,
                    this_ptr
                );
                Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
            }

            // basic_filebuf destructor (virtual)
            "??1?$basic_filebuf@DU?$char_traits@D@std@@@std@@UAE@XZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_filebuf::~basic_filebuf() -> VOID",
                    this_ptr
                );
                Some(ApiHookResult::callee(0, None))
            }

            // basic_ostream destructor (virtual)
            "??1?$basic_ostream@DU?$char_traits@D@std@@@std@@UAE@XZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_ostream::~basic_ostream() -> VOID",
                    this_ptr
                );
                Some(ApiHookResult::callee(0, None))
            }

            // basic_ios destructor (virtual)
            "??1?$basic_ios@DU?$char_traits@D@std@@@std@@UAE@XZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_ios::~basic_ios() -> VOID",
                    this_ptr
                );
                Some(ApiHookResult::callee(0, None))
            }

            // ios_base constructor / destructor
            "??0ios_base@std@@IAE@XZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) ios_base::ios_base() -> (this={:#x})",
                    this_ptr,
                    this_ptr
                );
                Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
            }

            "??1ios_base@std@@UAE@XZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) ios_base::~ios_base() -> VOID",
                    this_ptr
                );
                Some(ApiHookResult::callee(0, None))
            }

            // ios_base::getloc()
            "?getloc@ios_base@std@@QBE?AVlocale@2@XZ" => {
                // locale 객체 반환 (가상 핸들)
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                let addr = uc.malloc(16);
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) ios_base::getloc() -> {:#x}",
                    this_ptr,
                    addr
                );
                Some(ApiHookResult::callee(1, Some(addr as i32)))
            }

            // streambuf::imbue(const locale&)
            "?imbue@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MAEXABVlocale@2@@Z" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                let locale_ptr = uc.read_arg(0);
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_streambuf::imbue({:#x}) -> VOID",
                    this_ptr,
                    locale_ptr
                );
                Some(ApiHookResult::callee(1, None))
            }

            // Init
            "??0Init@ios_base@std@@QAE@XZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!("[MSVCP60] (this={:#x}) ios_base::Init::Init()", this_ptr);
                Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
            }
            "??1Init@ios_base@std@@QAE@XZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!("[MSVCP60] (this={:#x}) ios_base::Init::~Init()", this_ptr);
                Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
            }

            // _Winit
            "??0_Winit@std@@QAE@XZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!("[MSVCP60] (this={:#x}) std::_Winit::_Winit()", this_ptr);
                Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
            }
            "??1_Winit@std@@QAE@XZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!("[MSVCP60] (this={:#x}) std::_Winit::~_Winit()", this_ptr);
                Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
            }

            // _Lockit
            "??0_Lockit@std@@QAE@XZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!("[MSVCP60] (this={:#x}) std::_Lockit::_Lockit()", this_ptr);
                Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
            }
            "??1_Lockit@std@@QAE@XZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!("[MSVCP60] (this={:#x}) std::_Lockit::~_Lockit()", this_ptr);
                Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
            }

            // basic_iostream constructor
            "??0?$basic_iostream@DU?$char_traits@D@std@@@std@@QAE@PAV?$basic_streambuf@DU?$char_traits@D@std@@@1@@Z" =>
            {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                let sb_ptr = uc.read_arg(0);
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_iostream::basic_iostream({:#x}) -> (this={:#x})",
                    this_ptr,
                    sb_ptr,
                    this_ptr
                );
                Some(ApiHookResult::callee(1, Some(this_ptr as i32)))
            }

            // API: std::operator<<(std::ostream&, const char*)
            // 역할: C 스타일 문자열을 출력 스트림에 삽입
            "??6std@@YAAAV?$basic_ostream@DU?$char_traits@D@std@@@0@AAV10@PBD@Z" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                let os_ptr = uc.read_arg(0);
                let str_ptr = uc.read_arg(1);
                let text = if str_ptr != 0 {
                    uc.read_string(str_ptr as u64)
                } else {
                    String::from("(null)")
                };
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) std::operator<<({:#x}, {:#x}=\"{}\") -> (this={:#x})",
                    this_ptr,
                    os_ptr,
                    str_ptr,
                    text,
                    os_ptr
                );
                Some(ApiHookResult::callee(2, Some(os_ptr as i32))) // Return ostream&
            }

            // API: std::flush(std::ostream&)
            // 역할: 출력 스트림의 버퍼를 플러시(비움)
            "?flush@std@@YAAAV?$basic_ostream@DU?$char_traits@D@std@@@1@AAV21@@Z" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                let os_ptr = uc.read_arg(0);
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) std::flush({:#x}) -> (this={:#x})",
                    this_ptr,
                    os_ptr,
                    os_ptr
                );
                Some(ApiHookResult::callee(1, Some(os_ptr as i32))) // Return ostream&
            }

            _ => {
                crate::emu_log!("[!] MSVCP60 Unhandled: {}", func_name);
                // 대부분의 MSVCP60 함수는 thiscall. ECX 반환
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap_or(0) as u32;
                Some(ApiHookResult::callee(0, Some(this_ptr as i32)))
            }
        };

        if is_cdecl {
            if let Some(mut r) = result {
                r.cleanup = crate::win32::StackCleanup::Caller;
                Some(r)
            } else {
                None
            }
        } else {
            result
        }
    }
}
