use unicorn_engine::Unicorn;

use crate::helper::UnicornHelper;
use crate::win32::{ApiHookResult, Win32Context, callee_result, caller_result};

/// `MSVCP60.dll` 프록시 구현 모듈
///
/// C++ 표준 라이브러리(STL) 관련 특정 함수 및 문자열 처리를 가상화하여 호환성을 확보
pub struct DllMSVCP60;

impl DllMSVCP60 {
    // API: std::basic_string<char>::basic_string(const allocator<char>&)
    // 역할: std::string의 기본 생성자. 빈 문자열로 초기화
    pub fn basic_string_constructor(
        uc: &mut Unicorn<Win32Context>,
    ) -> Option<(usize, Option<i32>)> {
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
        Some((1, Some(this_ptr as i32)))
    }

    // API: std::basic_string<char>::_Tidy(bool)
    // 역할: 문자열 버퍼를 해제하고 초기 상태로 되돌림
    pub fn basic_string_tidy(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
        let b = uc.read_arg(0);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::_Tidy({}) -> VOID",
            this_ptr,
            b
        );
        Some((1, None))
    }

    // API: std::basic_string<char>::_Grow(size_t, bool)
    // 역할: 문자열 버퍼 크기를 확장
    pub fn basic_string_grow(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
        let n = uc.read_arg(0);
        let b = uc.read_arg(1);
        crate::emu_log!(
            "[MSVCP60] (this={:#x}) basic_string::_Grow({}, {}) -> BOOL 1",
            this_ptr,
            n,
            b
        );
        Some((2, Some(1))) // TRUE = 성공
    }

    // API: std::basic_string<char>::assign(const char*, size_t)
    // 역할: 문자열에 특정 포인터의 데이터를 버퍼만큼 할당
    pub fn basic_string_assign_param2(
        uc: &mut Unicorn<Win32Context>,
    ) -> Option<(usize, Option<i32>)> {
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
        Some((2, Some(this_ptr as i32)))
    }

    // API: std::basic_string<char>::assign(const basic_string&, size_t, size_t)
    // 역할: 다른 string 객체의 일부를 할당
    pub fn basic_string_assign_param3(
        uc: &mut Unicorn<Win32Context>,
    ) -> Option<(usize, Option<i32>)> {
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
        Some((3, Some(this_ptr as i32)))
    }

    // API: std::basic_string<char>::erase(size_t, size_t)
    // 역할: 문자열의 일부를 삭제
    pub fn basic_string_erase(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((2, Some(this_ptr as i32)))
    }

    /// 함수명 기준 `MSVCP60.dll` API 구현체
    ///
    /// 처리를 성공했다면 스택 보정값과 리턴값을 포함한 `ApiHookResult`를 반환
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        // MSVCP60.dll은 Visual C++ 6.0의 C++ 표준 라이브러리 (STL)
        // mangled name 에서 호출 규약을 판별
        // ??...QAE : thiscall (callee cleanup)
        // ??...YA  : cdecl (caller cleanup)
        // ??...YG  : stdcall (callee cleanup)

        let is_cdecl = func_name.contains("@YA") || func_name.contains("@Y?A");
        let wrap = if is_cdecl {
            caller_result
        } else {
            callee_result
        };

        wrap(match func_name {
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

            // 정적 npos
            "?npos@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@2IB" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                // npos = static const size_t = 0xFFFFFFFF
                let addr = uc.malloc(4);
                uc.write_u32(addr, 0xFFFFFFFF);
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_string::npos -> {:#x}",
                    this_ptr,
                    addr
                );
                Some((0, Some(addr as i32)))
            }

            // _Nullstr
            "?_C@?1??_Nullstr@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@CAPBDXZ@4DB" =>
            {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                let addr = uc.alloc_str("");
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_string::_Nullstr -> {:#x}",
                    this_ptr,
                    addr
                );
                Some((0, Some(addr as i32)))
            }

            // _Xlen
            "?_Xlen@std@@YAXXZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) std::_Xlen() [throw length_error]",
                    this_ptr
                );
                Some((0, None))
            }

            // =========================================================
            // iostream / fstream / streambuf
            // =========================================================

            // API: std::basic_ostream<char>::operator<<(int)
            // 역할: 정수 값을 출력 스트림에 삽입
            "??6?$basic_ostream@DU?$char_traits@D@std@@@std@@QAEAAV01@H@Z" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                let val = uc.read_arg(0);
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_ostream::operator<<({}) -> (this={:#x})",
                    this_ptr,
                    val,
                    this_ptr
                );
                Some((1, Some(this_ptr as i32)))
            }

            // API: std::basic_ios<char>::clear(iostate, bool)
            // 역할: 스트림의 오류 상태를 설정
            "?clear@?$basic_ios@DU?$char_traits@D@std@@@std@@QAEXH_N@Z" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                let state = uc.read_arg(0);
                let b = uc.read_arg(1);
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_ios::clear({}, {}) -> VOID",
                    this_ptr,
                    state,
                    b
                );
                Some((2, None))
            }

            // basic_ios::init(streambuf*, bool)
            "?init@?$basic_ios@DU?$char_traits@D@std@@@std@@IAEXPAV?$basic_streambuf@DU?$char_traits@D@std@@@2@_N@Z" =>
            {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                let buf = uc.read_arg(0);
                let b = uc.read_arg(1);
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_ios::init({:#x}, {}) -> VOID",
                    this_ptr,
                    buf,
                    b
                );
                Some((2, None))
            }

            // API: void std::ios_base::_Init()
            // 역할: C++ 표준 라이브러리의 ios_base를 초기화
            "?_Init@ios_base@std@@IAEXXZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!(
                    "[MSVCRT] (this={:#x}) std::ios_base::_Init() -> void",
                    this_ptr
                );
                Some((0, None))
            }

            // API: _Locimp * __cdecl std::locale::_Init()
            // 역할: C++ 표준 라이브러리의 locale을 초기화
            "?_Init@locale@std@@CAPAV_Locimp@12@XZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!(
                    "[MSVCRT] (this={:#x}) std::locale::_Init() -> void",
                    this_ptr
                );
                Some((0, None))
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
                Some((4, None))
            }

            // basic_fstream destructor sequence
            "??_D?$basic_fstream@DU?$char_traits@D@std@@@std@@QAEXXZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_fstream::~basic_fstream() -> VOID",
                    this_ptr
                );
                Some((0, None))
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
                Some((0, Some(this_ptr as i32)))
            }

            "??_D?$basic_ofstream@DU?$char_traits@D@std@@@std@@QAEXXZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_ofstream::~basic_ofstream() -> VOID",
                    this_ptr
                );
                Some((0, None))
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
                Some((1, Some(this_ptr as i32)))
            }

            // API: std::basic_istream<char>::getline(char*, int, char)
            // 역할: 입력 스트림에서 한 줄을 읽음
            "?getline@?$basic_istream@DU?$char_traits@D@std@@@std@@QAEAAV12@PADHD@Z" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                let buf = uc.read_arg(0);
                let count = uc.read_arg(1);
                let delim = uc.read_arg(2);
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_istream::getline({:#x}, {}, {}) -> (this={:#x})",
                    this_ptr,
                    buf,
                    count,
                    delim,
                    this_ptr
                );
                Some((3, Some(this_ptr as i32)))
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
                Some((2, Some(this_ptr as i32)))
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
                Some((2, Some(this_ptr as i32))) // NULL = 실패
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
                Some((1, Some(this_ptr as i32)))
            }

            // basic_filebuf destructor (virtual)
            "??1?$basic_filebuf@DU?$char_traits@D@std@@@std@@UAE@XZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_filebuf::~basic_filebuf() -> VOID",
                    this_ptr
                );
                Some((0, None))
            }

            // basic_ostream destructor (virtual)
            "??1?$basic_ostream@DU?$char_traits@D@std@@@std@@UAE@XZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_ostream::~basic_ostream() -> VOID",
                    this_ptr
                );
                Some((0, None))
            }

            // basic_ios destructor (virtual)
            "??1?$basic_ios@DU?$char_traits@D@std@@@std@@UAE@XZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) basic_ios::~basic_ios() -> VOID",
                    this_ptr
                );
                Some((0, None))
            }

            // ios_base constructor / destructor
            "??0ios_base@std@@IAE@XZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) ios_base::ios_base() -> (this={:#x})",
                    this_ptr,
                    this_ptr
                );
                Some((0, Some(this_ptr as i32)))
            }

            "??1ios_base@std@@UAE@XZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                crate::emu_log!(
                    "[MSVCP60] (this={:#x}) ios_base::~ios_base() -> VOID",
                    this_ptr
                );
                Some((0, None))
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
                Some((1, Some(addr as i32)))
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
                Some((1, None))
            }

            // Init
            "??0Init@ios_base@std@@QAE@XZ" => {
                crate::emu_log!("[MSVCP60] ios_base::Init::Init()");
                Some((0, None))
            }

            "??1Init@ios_base@std@@QAE@XZ" => {
                crate::emu_log!("[MSVCP60] ios_base::Init::~Init()");
                Some((0, None))
            }

            // _Winit
            "??0_Winit@std@@QAE@XZ" => {
                crate::emu_log!("[MSVCP60] std::_Winit::_Winit()");
                Some((0, None))
            }

            "??1_Winit@std@@QAE@XZ" => {
                crate::emu_log!("[MSVCP60] std::_Winit::~_Winit()");
                Some((0, None))
            }

            // _Lockit
            "??0_Lockit@std@@QAE@XZ" => {
                crate::emu_log!("[MSVCP60] std::_Lockit::_Lockit()");
                Some((0, None))
            }
            "??1_Lockit@std@@QAE@XZ" => {
                crate::emu_log!("[MSVCP60] std::_Lockit::~_Lockit()");
                Some((0, None))
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
                Some((1, Some(this_ptr as i32)))
            }

            // VTable 포인터 (static 데이터)
            name if name.starts_with("??_7") || name.starts_with("??_8") => {
                // vtable/vbtable pointer - 가상 주소 반환
                let addr = uc.malloc(64); // 가짜 vtable
                crate::emu_log!("[MSVCP60] vtable/vbtable: {} -> {:#x}", name, addr);
                Some((0, Some(addr as i32)))
            }

            // static _Fpz (zero fpos)
            "?_Fpz@std@@3_JB" => {
                let addr = uc.malloc(8);
                uc.write_u32(addr, 0);
                uc.write_u32(addr + 4, 0);
                crate::emu_log!("[MSVCP60] std::_Fpz -> {:#x}", addr);
                Some((0, Some(addr as i32)))
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
                Some((2, Some(os_ptr as i32))) // Return ostream&
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
                Some((1, Some(os_ptr as i32))) // Return ostream&
            }

            _ => {
                crate::emu_log!("[!] MSVCP60 Unhandled: {}", func_name);
                // 대부분의 MSVCP60 함수는 thiscall. ECX 반환
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap_or(0) as u32;
                Some((0, Some(this_ptr as i32)))
            }
        })
    }
}
