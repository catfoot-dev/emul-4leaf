use crate::dll::win32::{ApiHookResult, Win32Context};
use unicorn_engine::Unicorn;

/// `WINMM.dll` 프록시 구현 모듈
///
/// 윈도우 멀티미디어 API (밀리초 정밀도 시간 측정 등) 호출에 대해 가벼운 목(Mock) 환경을 구성
#[allow(clippy::upper_case_acronyms)]
pub struct WINMM;

impl WINMM {
    // API: DWORD timeGetTime(void)
    // 역할: 시스템 시간이 시작된 후 경과된 시간을 밀리초 단위로 검색
    pub fn time_get_time(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let elapsed = uc.get_data().start_time.elapsed().as_millis() as u32;
        // crate::emu_log!("[WINMM] timeGetTime() -> DWORD {}", elapsed);
        Some(ApiHookResult::callee(0, Some(elapsed as i32)))
    }

    /// 함수명 기준 `WINMM.dll` API 구현체
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        match func_name {
            "timeGetTime" => Self::time_get_time(uc),
            _ => {
                crate::emu_log!("[!] WINMM Unhandled: {}", func_name);
                None
            }
        }
    }
}
