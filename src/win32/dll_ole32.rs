use unicorn_engine::Unicorn;

use crate::win32::{ApiHookResult, Win32Context, callee_result};

/// `OLE32.dll` 프록시 구현 모듈
///
/// COM 오브젝트 초기화 및 메모리 할당기 코어 함수들을 응답
pub struct DllOle32;

impl DllOle32 {
    /// 함수명 기준 `OLE32.dll` API 구현체
    ///
    /// 처리를 성공했다면 스택 보정값과 리턴값을 포함한 `ApiHookResult`를 반환
    pub fn handle(_uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        callee_result(match func_name {
            // API: HRESULT CoCreateInstance(REFCLSID rclsid, LPUNKNOWN pUnkOuter, DWORD dwClsContext, REFIID riid, LPVOID *ppv)
            // 역할: 지정된 CLSID와 관련된 클래스의 초기화되지 않은 단일 개체를 만듬
            "CoCreateInstance" => {
                crate::emu_log!("[OLE32] CoCreateInstance(...)");
                Some((5, Some(-2147467259i32))) // E_NOINTERFACE (0x80004002)
            }

            // API: HRESULT CoInitialize(LPVOID pvReserved)
            // 역할: 현재 스레드의 COM 라이브러리를 초기화
            "CoInitialize" => {
                crate::emu_log!("[OLE32] CoInitialize(...)");
                Some((1, Some(0))) // S_OK
            }

            _ => {
                crate::emu_log!("[OLE32] UNHANDLED: {}", func_name);
                None
            }
        })
    }
}
