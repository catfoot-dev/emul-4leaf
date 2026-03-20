use unicorn_engine::Unicorn;

use crate::win32::{ApiHookResult, Win32Context, callee_result};

/// `WINMM.dll` 프록시 구현 모듈
///
/// 윈도우 멀티미디어 API (밀리초 정밀도 시간 측정 등) 호출에 대해 가벼운 목(Mock) 환경을 구성
pub struct DllWINMM;

impl DllWINMM {
    /// 함수명 기준 `WINMM.dll` API 구현체
    ///
    /// 처리를 성공했다면 스택 보정값과 리턴값을 포함한 `ApiHookResult`를 반환
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        callee_result(match func_name {
            // API: DWORD timeGetTime(void)
            // 역할: 시스템 시간이 시작된 후 경과된 시간을 밀리초 단위로 검색
            "timeGetTime" => {
                let elapsed = uc.get_data().start_time.elapsed().as_millis() as u32;
                Some((0, Some(elapsed as i32)))
            }

            _ => {
                crate::emu_log!("[WINMM] UNHANDLED: {}", func_name);
                None
            }
        })
    }
}
