use unicorn_engine::Unicorn;

use crate::helper::UnicornHelper;
use crate::win32::{ApiHookResult, Win32Context};

/// `IMM32.dll` 프록시 구현 모듈
///
/// 보조 입력 메소드 (Input Method Manager) 제어를 위한 가짜 응답을 제공
pub struct DllIMM32;

impl DllIMM32 {
    // API: BOOL ImmIsUIMessageA(HWND hWndIME, UINT msg, WPARAM wParam, LPARAM lParam)
    // 역할: IME 창을 위한 메시지인지 확인
    pub fn imm_is_ui_message_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let msg = uc.read_arg(1);
        let wparam = uc.read_arg(2);
        let lparam = uc.read_arg(3);
        crate::emu_log!(
            "[IMM32] ImmIsUIMessageA({:#x}, {}, {}, {}) -> BOOL 0",
            hwnd,
            msg,
            wparam,
            lparam
        );
        Some(ApiHookResult::callee(4, Some(0))) // FALSE
    }

    // API: BOOL ImmGetConversionStatus(HIMC hIMC, LPDWORD lpfdwConversion, LPDWORD lpfdwSentence)
    // 역할: 현재 변환 상태를 가져옴
    pub fn imm_get_conversion_status(
        uc: &mut Unicorn<Win32Context>,
    ) -> Option<ApiHookResult> {
        let himc = uc.read_arg(0);
        let lpfdw_conversion = uc.read_arg(1);
        let lpfdw_sentence = uc.read_arg(2);
        crate::emu_log!(
            "[IMM32] ImmGetConversionStatus({:#x}, {:#x}, {:#x}) -> BOOL 0",
            himc,
            lpfdw_conversion,
            lpfdw_sentence
        );
        Some(ApiHookResult::callee(3, Some(0))) // FALSE
    }

    // API: HIMC ImmGetContext(HWND hWnd)
    // 역할: 지정된 윈도우에 연결된 입력 컨텍스트를 가져옴
    pub fn imm_get_context(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let ctx = uc.get_data();
        let himc = ctx.alloc_handle();
        crate::emu_log!("[IMM32] ImmGetContext({:#x}) -> HIMC {:#x}", hwnd, himc);
        Some(ApiHookResult::callee(1, Some(himc as i32)))
    }

    // API: BOOL ImmReleaseContext(HWND hWnd, HIMC hIMC)
    // 역할: 입력 컨텍스트를 해제하고 컨텍스트에 할당된 메모리를 잠금 해제
    pub fn imm_release_context(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let himc = uc.read_arg(1);
        crate::emu_log!(
            "[IMM32] ImmReleaseContext({:#x}, {:#x}) -> BOOL 1",
            hwnd,
            himc
        );
        Some(ApiHookResult::callee(2, Some(1))) // TRUE
    }

    // API: BOOL ImmSetConversionStatus(HIMC hIMC, DWORD fdwConversion, DWORD fdwSentence)
    // 역할: 현재 변환 상태를 설정
    pub fn imm_set_conversion_status(
        uc: &mut Unicorn<Win32Context>,
    ) -> Option<ApiHookResult> {
        let himc = uc.read_arg(0);
        let fdw_conversion = uc.read_arg(1);
        let fdw_sentence = uc.read_arg(2);
        crate::emu_log!(
            "[IMM32] ImmSetConversionStatus({:#x}, {:#x}, {:#x}) -> BOOL 1",
            himc,
            fdw_conversion,
            fdw_sentence
        );
        Some(ApiHookResult::callee(3, Some(1))) // TRUE
    }

    /// 함수명 기준 `IMM32.dll` API 구현체
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        match func_name {
            "ImmIsUIMessageA" => Self::imm_is_ui_message_a(uc),
            "ImmGetConversionStatus" => Self::imm_get_conversion_status(uc),
            "ImmGetContext" => Self::imm_get_context(uc),
            "ImmReleaseContext" => Self::imm_release_context(uc),
            "ImmSetConversionStatus" => Self::imm_set_conversion_status(uc),
            _ => {
                crate::emu_log!("[!] IMM32 Unhandled: {}", func_name);
                None
            }
        }
    }
}
