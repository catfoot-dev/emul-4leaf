use unicorn_engine::Unicorn;

use crate::{
    helper::UnicornHelper,
    win32::{ApiHookResult, Win32Context},
};

/// `OLE32.dll` 프록시 구현 모듈
///
/// COM 오브젝트 초기화 및 메모리 할당기 코어 함수들을 응답
pub struct DllOle32;

impl DllOle32 {
    // API: HRESULT CoCreateInstance(REFCLSID rclsid, LPUNKNOWN pUnkOuter, DWORD dwClsContext, REFIID riid, LPVOID *ppv)
    // 역할: 지정된 CLSID와 관련된 클래스의 초기화되지 않은 단일 개체를 만듬
    pub fn co_create_instance(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let rclsid = uc.read_arg(0);
        let p_unk_outer = uc.read_arg(1);
        let dw_cls_context = uc.read_arg(2);
        let riid = uc.read_arg(3);
        let ppv = uc.read_arg(4);
        crate::emu_log!(
            "[OLE32] CoCreateInstance({:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> HRESULT {:#x}",
            rclsid,
            p_unk_outer,
            dw_cls_context,
            riid,
            ppv,
            -2147467259i32
        );
        Some(ApiHookResult::callee(5, Some(-2147467259i32))) // E_NOINTERFACE (0x80004002)
    }

    // API: HRESULT CoInitialize(LPVOID pvReserved)
    // 역할: 현재 스레드의 COM 라이브러리를 초기화
    pub fn co_initialize(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let pv_reserved = uc.read_arg(0);
        crate::emu_log!(
            "[OLE32] CoInitialize({:#x}) -> HRESULT {:#x}",
            pv_reserved,
            0
        );
        Some(ApiHookResult::callee(1, Some(0))) // S_OK
    }

    /// 함수명 기준 `OLE32.dll` API 구현체
    ///
    /// 처리를 성공했다면 스택 보정값과 리턴값을 포함한 `ApiHookResult`를 반환
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        match func_name {
            "CoCreateInstance" => Self::co_create_instance(uc),
            "CoInitialize" => Self::co_initialize(uc),

            _ => {
                crate::emu_log!("[!] OLE32 Unhandled: {}", func_name);
                None
            }
        }
    }
}
