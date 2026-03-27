use unicorn_engine::Unicorn;

use crate::{
    helper::UnicornHelper,
    win32::{ApiHookResult, Win32Context},
};

/// `SHELL32.dll` 프록시 구현 모듈
///
/// 윈도우 쉘 환경에 접근하는 API(Drag & Drop, 바탕화면 실행 등)에 대한 가상 스텁을 제공
pub struct DllSHELL32;

impl DllSHELL32 {
    // API: HINSTANCE ShellExecuteA(HWND hwnd, LPCSTR lpOperation, LPCSTR lpFile, LPCSTR lpParameters, LPCSTR lpDirectory, INT nShowCmd)
    // 역할: 지정된 파일이나 응용 프로그램에 대한 작업을 수행
    pub fn shell_execute_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hwnd = uc.read_arg(0);
        let lp_operation = uc.read_arg(1);
        let lp_file = uc.read_arg(2);
        let lp_parameters = uc.read_arg(3);
        let lp_directory = uc.read_arg(4);
        let n_show_cmd = uc.read_arg(5);
        crate::emu_log!(
            "[SHELL32] ShellExecuteA({:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> HINSTANCE {:#x}",
            hwnd,
            lp_operation,
            lp_file,
            lp_parameters,
            lp_directory,
            n_show_cmd,
            42
        );
        Some(ApiHookResult::callee(6, Some(42))) // > 32 = 성공
    }

    /// 함수명 기준 `SHELL32.dll` API 구현체
    ///
    /// 처리를 성공했다면 스택 보정값과 리턴값을 포함한 `ApiHookResult`를 반환
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        match func_name {
            "ShellExecuteA" => Self::shell_execute_a(uc),

            _ => {
                crate::emu_log!("[!] SHELL32 Unhandled: {}", func_name);
                None
            }
        }
    }
}
