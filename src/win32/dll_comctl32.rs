use unicorn_engine::Unicorn;

use crate::{
    helper::UnicornHelper,
    win32::{ApiHookResult, Win32Context, callee_result},
};

/// `COMCTL32.dll` 프록시 구현 모듈
///
/// 공통 컨트롤(Common Controls) 라이브러리 관련 API 호출에 대한 가짜 응답을 제공
pub struct DllCOMCTL32;

impl DllCOMCTL32 {
    /// 함수명 기준 `COMCTL32.dll` API 구현체
    ///
    /// 처리를 성공했다면 스택 보정값과 리턴값을 포함한 `ApiHookResult`를 반환
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        callee_result(match func_name {
            // API: BOOL _TrackMouseEvent(LPTRACKMOUSEEVENT lpEventTrack)
            // 역할: 지정된 윈도우에서 마우스 포인터가 벗어나거나 일정 시간 머무를 때 메시지를 게시하도록 요청
            "_TrackMouseEvent" => {
                let lp_event_track = uc.read_arg(0);
                crate::emu_log!(
                    "[COMCTL32] _TrackMouseEvent({:#x}) -> int {:#x}",
                    lp_event_track,
                    1
                );
                Some((1, Some(1))) // TRUE
            }

            // API: BOOL InitCommonControlsEx(const INITCOMMONCONTROLSEX *picce)
            // 역할: 공통 컨트롤 DLL(Comctl32.dll)에서 특정 공통 컨트롤 클래스를 로드하고 초기화
            "InitCommonControlsEx" => {
                let picce = uc.read_arg(0);
                crate::emu_log!(
                    "[COMCTL32] InitCommonControlsEx({:#x}) -> int {:#x}",
                    picce,
                    1
                );
                Some((1, Some(1))) // TRUE
            }

            _ => {
                crate::emu_log!("[COMCTL32] UNHANDLED: {}", func_name);
                None
            }
        })
    }
}
