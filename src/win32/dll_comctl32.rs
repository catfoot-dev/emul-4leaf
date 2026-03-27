use unicorn_engine::Unicorn;

use crate::{
    helper::UnicornHelper,
    win32::{ApiHookResult, Win32Context},
};

/// `COMCTL32.dll` 프록시 구현 모듈
///
/// 공통 컨트롤(Common Controls) 라이브러리 관련 API 호출에 대한 가짜 응답을 제공
pub struct DllCOMCTL32;

impl DllCOMCTL32 {
    // API: BOOL _TrackMouseEvent(LPTRACKMOUSEEVENT lpEventTrack)
    // 역할: 지정된 윈도우에서 마우스 포인터가 벗어나거나 일정 시간 머무를 때 메시지를 게시하도록 요청
    pub fn _track_mouse_event(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let lp_event_track = uc.read_arg(0);
        let size = uc.read_u32(lp_event_track as u64 + 0);
        let flags = uc.read_u32(lp_event_track as u64 + 4);
        let hwnd = uc.read_u32(lp_event_track as u64 + 8);
        let hover_time = uc.read_u32(lp_event_track as u64 + 12);

        crate::emu_log!(
            "[COMCTL32] _TrackMouseEvent(size={}, flags={:#x}, hwnd={:#x}, hover={})",
            size,
            flags,
            hwnd,
            hover_time
        );

        let ctx = uc.get_data();
        let mut track = ctx.track_mouse_event.lock().unwrap();

        if flags & 0x00000080 != 0 {
            // TME_CANCEL
            *track = None;
        } else {
            *track = Some(crate::win32::TrackMouseEventState {
                hwnd,
                flags,
                hover_time,
            });
        }

        Some(ApiHookResult::callee(1, Some(1))) // TRUE
    }

    // API: BOOL InitCommonControlsEx(const INITCOMMONCONTROLSEX *picce)
    // 역할: 공통 컨트롤 DLL(Comctl32.dll)에서 특정 공통 컨트롤 클래스를 로드하고 초기화
    pub fn init_common_controls_ex(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let picce = uc.read_arg(0);
        let size = uc.read_u32(picce as u64);
        let icc_flags = uc.read_u32(picce as u64 + 4);

        crate::emu_log!(
            "[COMCTL32] InitCommonControlsEx(size={}, flags={:#x}) -> BOOL 1",
            size,
            icc_flags
        );
        Some(ApiHookResult::callee(1, Some(1))) // TRUE
    }

    /// 함수명 기준 `COMCTL32.dll` API 구현체
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        match func_name {
            "_TrackMouseEvent" => Self::_track_mouse_event(uc),
            "InitCommonControlsEx" => Self::init_common_controls_ex(uc),
            _ => {
                crate::emu_log!("[!] COMCTL32 Unhandled: {}", func_name);
                None
            }
        }
    }
}
