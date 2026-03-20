use unicorn_engine::Unicorn;

use crate::helper::UnicornHelper;
use crate::win32::{ApiHookResult, Win32Context, caller_result};

pub struct DllMSVCP60 {}

impl DllMSVCP60 {
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        // MSVCP60.dll은 Visual C++ 6.0의 C++ 표준 라이브러리 (STL)
        // 대부분의 함수는 thiscall (ECX = this) 이지만,
        // 이번 패치에서는 현행 동작 보존을 위해 caller-cleanup으로 유지

        caller_result(match func_name {
            // =========================================================
            // basic_string<char> (std::string)
            // =========================================================
            // std::basic_string<char>::basic_string(const allocator<char>&)
            "??0?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAE@ABV?$allocator@D@1@@Z" =>
            {
                // 기본 생성자: this->_Ptr = static empty string, this->_Len = 0, this->_Res = 0
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                if this_ptr != 0 {
                    // basic_string layout: _Allocator(4), _Ptr(4), _Len(4), _Res(4)
                    let empty_str = uc.alloc_str("");
                    uc.write_u32(this_ptr as u64 + 4, empty_str); // _Ptr
                    uc.write_u32(this_ptr as u64 + 8, 0); // _Len
                    uc.write_u32(this_ptr as u64 + 12, 0); // _Res
                }
                println!(
                    "[MSVCP60] basic_string::basic_string(alloc) this={:#x}",
                    this_ptr
                );
                Some((0, Some(this_ptr as i32)))
            }

            // std::basic_string::_Tidy(bool)
            "?_Tidy@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@AAEX_N@Z" => {
                println!("[MSVCP60] basic_string::_Tidy(...)");
                Some((0, None))
            }

            // std::basic_string::_Grow(size_t, bool)
            "?_Grow@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@AAE_NI_N@Z" => {
                println!("[MSVCP60] basic_string::_Grow(...)");
                Some((0, Some(1))) // TRUE = 성공
            }

            // std::basic_string::assign(const char*, size_t)
            "?assign@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEAAV12@PBDI@Z" =>
            {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                println!(
                    "[MSVCP60] basic_string::assign(ptr, len) this={:#x}",
                    this_ptr
                );
                Some((0, Some(this_ptr as i32)))
            }

            // std::basic_string::assign(const basic_string&, size_t, size_t)
            "?assign@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEAAV12@ABV12@II@Z" =>
            {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                println!(
                    "[MSVCP60] basic_string::assign(str&, off, count) this={:#x}",
                    this_ptr
                );
                Some((0, Some(this_ptr as i32)))
            }

            // std::basic_string::erase(size_t, size_t)
            "?erase@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@QAEAAV12@II@Z" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                println!(
                    "[MSVCP60] basic_string::erase(off, count) this={:#x}",
                    this_ptr
                );
                Some((0, Some(this_ptr as i32)))
            }

            // 정적 npos
            "?npos@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@2IB" => {
                // npos = static const size_t = 0xFFFFFFFF
                let addr = uc.malloc(4);
                uc.write_u32(addr, 0xFFFFFFFF);
                println!("[MSVCP60] basic_string::npos -> {:#x}", addr);
                Some((0, Some(addr as i32)))
            }

            // _Nullstr
            "?_C@?1??_Nullstr@?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@CAPBDXZ@4DB" =>
            {
                let addr = uc.alloc_str("");
                println!("[MSVCP60] basic_string::_Nullstr -> {:#x}", addr);
                Some((0, Some(addr as i32)))
            }

            // _Xlen
            "?_Xlen@std@@YAXXZ" => {
                println!("[MSVCP60] std::_Xlen() [throw length_error]");
                Some((0, None))
            }

            // =========================================================
            // iostream / fstream / streambuf
            // =========================================================

            // basic_ostream<char>::operator<<(int)
            "??6?$basic_ostream@DU?$char_traits@D@std@@@std@@QAEAAV01@H@Z" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                println!("[MSVCP60] basic_ostream::operator<<(int)");
                Some((0, Some(this_ptr as i32)))
            }

            // basic_ios::clear(iostate, bool)
            "?clear@?$basic_ios@DU?$char_traits@D@std@@@std@@QAEXH_N@Z" => {
                println!("[MSVCP60] basic_ios::clear(...)");
                Some((0, None))
            }

            // basic_ios::init(streambuf*, bool)
            "?init@?$basic_ios@DU?$char_traits@D@std@@@std@@IAEXPAV?$basic_streambuf@DU?$char_traits@D@std@@@2@_N@Z" =>
            {
                println!("[MSVCP60] basic_ios::init(...)");
                Some((0, None))
            }

            // basic_fstream destructor sequence
            "??_D?$basic_fstream@DU?$char_traits@D@std@@@std@@QAEXXZ" => {
                println!("[MSVCP60] basic_fstream::~basic_fstream()");
                Some((0, None))
            }

            // basic_ofstream
            "??0?$basic_ofstream@DU?$char_traits@D@std@@@std@@QAE@XZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                println!(
                    "[MSVCP60] basic_ofstream::basic_ofstream() this={:#x}",
                    this_ptr
                );
                Some((0, Some(this_ptr as i32)))
            }
            "??_D?$basic_ofstream@DU?$char_traits@D@std@@@std@@QAEXXZ" => {
                println!("[MSVCP60] basic_ofstream::~basic_ofstream()");
                Some((0, None))
            }

            // basic_istream::seekg(fpos)
            "?seekg@?$basic_istream@DU?$char_traits@D@std@@@std@@QAEAAV12@V?$fpos@H@2@@Z" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                println!("[MSVCP60] basic_istream::seekg(fpos)");
                Some((0, Some(this_ptr as i32)))
            }

            // basic_istream::getline(char*, int, char)
            "?getline@?$basic_istream@DU?$char_traits@D@std@@@std@@QAEAAV12@PADHD@Z" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                println!("[MSVCP60] basic_istream::getline(...)");
                Some((0, Some(this_ptr as i32)))
            }

            // basic_istream constructor
            "??0?$basic_istream@DU?$char_traits@D@std@@@std@@QAE@PAV?$basic_streambuf@DU?$char_traits@D@std@@@1@_N@Z" =>
            {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                println!(
                    "[MSVCP60] basic_istream::basic_istream(streambuf*) this={:#x}",
                    this_ptr
                );
                Some((0, Some(this_ptr as i32)))
            }

            // basic_filebuf::open(const char*, int)
            "?open@?$basic_filebuf@DU?$char_traits@D@std@@@std@@QAEPAV12@PBDH@Z" => {
                let _this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                println!("[MSVCP60] basic_filebuf::open(...)");
                Some((0, Some(0))) // NULL = 실패
            }

            // basic_filebuf constructor
            "??0?$basic_filebuf@DU?$char_traits@D@std@@@std@@QAE@PAU_iobuf@@@Z" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                println!(
                    "[MSVCP60] basic_filebuf::basic_filebuf(FILE*) this={:#x}",
                    this_ptr
                );
                Some((0, Some(this_ptr as i32)))
            }

            // basic_filebuf destructor (virtual)
            "??1?$basic_filebuf@DU?$char_traits@D@std@@@std@@UAE@XZ" => {
                println!("[MSVCP60] basic_filebuf::~basic_filebuf()");
                Some((0, None))
            }

            // basic_ostream destructor (virtual)
            "??1?$basic_ostream@DU?$char_traits@D@std@@@std@@UAE@XZ" => {
                println!("[MSVCP60] basic_ostream::~basic_ostream()");
                Some((0, None))
            }

            // basic_ios destructor (virtual)
            "??1?$basic_ios@DU?$char_traits@D@std@@@std@@UAE@XZ" => {
                println!("[MSVCP60] basic_ios::~basic_ios()");
                Some((0, None))
            }

            // ios_base constructor / destructor
            "??0ios_base@std@@IAE@XZ" => {
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap() as u32;
                println!("[MSVCP60] ios_base::ios_base() this={:#x}", this_ptr);
                Some((0, Some(this_ptr as i32)))
            }
            "??1ios_base@std@@UAE@XZ" => {
                println!("[MSVCP60] ios_base::~ios_base()");
                Some((0, None))
            }

            // ios_base::getloc()
            "?getloc@ios_base@std@@QBE?AVlocale@2@XZ" => {
                // locale 객체 반환 (가상 핸들)
                let addr = uc.malloc(16);
                println!("[MSVCP60] ios_base::getloc() -> {:#x}", addr);
                Some((0, Some(addr as i32)))
            }

            // streambuf::imbue(const locale&)
            "?imbue@?$basic_streambuf@DU?$char_traits@D@std@@@std@@MAEXABVlocale@2@@Z" => {
                println!("[MSVCP60] basic_streambuf::imbue(locale&)");
                Some((0, None))
            }

            // Init / _Winit
            "??0Init@ios_base@std@@QAE@XZ" => {
                println!("[MSVCP60] ios_base::Init::Init()");
                Some((0, None))
            }
            "??1Init@ios_base@std@@QAE@XZ" => {
                println!("[MSVCP60] ios_base::Init::~Init()");
                Some((0, None))
            }
            "??0_Winit@std@@QAE@XZ" => {
                println!("[MSVCP60] std::_Winit::_Winit()");
                Some((0, None))
            }
            "??1_Winit@std@@QAE@XZ" => {
                println!("[MSVCP60] std::_Winit::~_Winit()");
                Some((0, None))
            }

            // VTable 포인터 (static 데이터)
            name if name.starts_with("??_7") || name.starts_with("??_8") => {
                // vtable/vbtable pointer - 가상 주소 반환
                let addr = uc.malloc(64); // 가짜 vtable
                println!("[MSVCP60] vtable/vbtable: {} -> {:#x}", name, addr);
                Some((0, Some(addr as i32)))
            }

            // static _Fpz (zero fpos)
            "?_Fpz@std@@3_JB" => {
                let addr = uc.malloc(8);
                uc.write_u32(addr, 0);
                uc.write_u32(addr + 4, 0);
                println!("[MSVCP60] std::_Fpz -> {:#x}", addr);
                Some((0, Some(addr as i32)))
            }

            // std::operator<<(std::ostream&, const char*)
            "??6std@@YAAAV?$basic_ostream@DU?$char_traits@D@std@@@0@AAV10@PBD@Z" => {
                let os_ptr = uc.read_arg(0);
                let str_ptr = uc.read_arg(1);
                let text = if str_ptr != 0 {
                    uc.read_string(str_ptr as u64)
                } else {
                    String::from("(null)")
                };
                println!("[MSVCP60] std::operator<<(ostream&, const char*): {}", text);
                Some((0, Some(os_ptr as i32))) // Return ostream&
            }

            // std::flush(std::ostream&)
            "?flush@std@@YAAAV?$basic_ostream@DU?$char_traits@D@std@@@1@AAV21@@Z" => {
                let os_ptr = uc.read_arg(0);
                println!("[MSVCP60] std::flush(ostream&)");
                Some((0, Some(os_ptr as i32))) // Return ostream&
            }

            _ => {
                println!("[MSVCP60] UNHANDLED: {}", func_name);
                // 대부분의 MSVCP60 함수는 thiscall. ECX 반환
                let this_ptr = uc.reg_read(unicorn_engine::RegisterX86::ECX).unwrap_or(0) as u32;
                Some((0, Some(this_ptr as i32)))
            }
        })
    }
}
