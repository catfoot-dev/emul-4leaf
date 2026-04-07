use crate::{
    dll::win32::{ApiHookResult, Win32Context},
    helper::UnicornHelper,
};
use encoding_rs::EUC_KR;
use std::sync::atomic::Ordering;
use unicorn_engine::Unicorn;

use super::{
    ERROR_INVALID_PARAMETER, FORMAT_MESSAGE_ALLOCATE_BUFFER, FORMAT_MESSAGE_FROM_SYSTEM,
    FORMAT_MESSAGE_IGNORE_INSERTS, FORMAT_MESSAGE_MAX_WIDTH_MASK,
};

/// 지원하지 않는 플래그 비트를 계산합니다.
fn unsupported_flag_bits(flags: u32, supported_mask: u32) -> u32 {
    flags & !supported_mask
}

// =========================================================
// Handle
// =========================================================
// API: BOOL CloseHandle(HANDLE hObject)
// 역할: 열려있는 개체 핸들을 닫음
pub(super) fn close_handle(_uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let h_handle = _uc.read_arg(0);
    crate::emu_log!("[KERNEL32] CloseHandle({:#x}) -> BOOL 1", h_handle);
    Some(ApiHookResult::callee(1, Some(1))) // TRUE
}

/// API: BOOL DuplicateHandle(HANDLE hSourceProcessHandle, HANDLE hSourceHandle, ...)
/// 역할: 객체 핸들을 복제합니다.
pub(super) fn duplicate_handle(_uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let h_source_process_handle = _uc.read_arg(0);
    let h_source_handle = _uc.read_arg(1);
    let h_target_process_handle = _uc.read_arg(2);
    let lp_target_handle = _uc.read_arg(3);
    let dw_desired_access = _uc.read_arg(4);
    let b_inherit_handles = _uc.read_arg(5);
    let dw_options = _uc.read_arg(6);
    crate::emu_log!(
        "[KERNEL32] DuplicateHandle({:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 1",
        h_source_process_handle,
        h_source_handle,
        h_target_process_handle,
        lp_target_handle,
        dw_desired_access,
        b_inherit_handles,
        dw_options
    );
    Some(ApiHookResult::callee(7, Some(1))) // TRUE
}

// =========================================================
// Error
// =========================================================
// API: DWORD GetLastError(void)
// 역할: 호출하는 스레드의 가장 최근 오류 코드를 검색
pub(super) fn get_last_error(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let err = uc.get_data().last_error.load(Ordering::SeqCst);
    crate::emu_log!("[KERNEL32] GetLastError() -> DWORD {:#x}", err);
    Some(ApiHookResult::callee(0, Some(err as i32)))
}

// API: void SetLastError(DWORD dwErrCode)
// 역할: 호출 스레드의 가장 최근 오류 코드를 설정
pub(super) fn set_last_error(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let code = uc.read_arg(0);
    uc.get_data().last_error.store(code, Ordering::SeqCst);
    crate::emu_log!("[KERNEL32] SetLastError({:#x}) -> VOID", code);
    Some(ApiHookResult::callee(1, None))
}

/// API: DWORD FormatMessageA(DWORD dwFlags, LPCVOID lpSource, DWORD dwMessageId, ...)
/// 역할: 메시지 정의를 문자열로 포맷팅합니다.
pub(super) fn format_message_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let dw_flags = uc.read_arg(0);
    let _lp_source = uc.read_arg(1);
    let dw_message_id = uc.read_arg(2);
    let _dw_language_id = uc.read_arg(3);
    let lp_buffer = uc.read_arg(4);
    let n_size = uc.read_arg(5);
    let _arguments = uc.read_arg(6);

    crate::emu_log!(
        "[KERNEL32] FormatMessageA(flags={:#x}, id={:#x})",
        dw_flags,
        dw_message_id
    );

    let allowed_flags = FORMAT_MESSAGE_ALLOCATE_BUFFER
        | FORMAT_MESSAGE_IGNORE_INSERTS
        | FORMAT_MESSAGE_FROM_SYSTEM
        | FORMAT_MESSAGE_MAX_WIDTH_MASK;
    let unsupported = unsupported_flag_bits(dw_flags, allowed_flags);
    if unsupported != 0 || dw_flags & FORMAT_MESSAGE_FROM_SYSTEM == 0 {
        uc.get_data()
            .last_error
            .store(ERROR_INVALID_PARAMETER, Ordering::SeqCst);
        crate::emu_log!(
            "[KERNEL32] FormatMessageA unsupported flags={:#x} unsupported={:#x}",
            dw_flags,
            unsupported
        );
        return Some(ApiHookResult::callee(7, Some(0)));
    }

    let msg = format!("Win32 Error {:#x}\0", dw_message_id);
    let msg_len = msg.len();

    if dw_flags & FORMAT_MESSAGE_ALLOCATE_BUFFER != 0 {
        // Allocate buffer and write pointer to lp_buffer
        let addr = uc.alloc_str(&msg);
        uc.write_u32(lp_buffer as u64, addr);
    } else {
        // Write to provided buffer
        if lp_buffer != 0 && n_size as usize >= msg_len {
            uc.mem_write(lp_buffer as u64, msg.as_bytes()).unwrap();
        }
    }

    Some(ApiHookResult::callee(7, Some(msg_len as i32 - 1)))
}

// =========================================================
// Debug
// =========================================================
// API: void OutputDebugStringA(LPCSTR lpOutputString)
// 역할: 문자열을 디버거로 보내 화면에 출력
pub(super) fn output_debug_string_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let addr = uc.read_arg(0);
    let s = if addr != 0 {
        uc.read_euc_kr(addr as u64)
    } else {
        String::new()
    };
    crate::emu_log!("[KERNEL32] OutputDebugStringA(\"{s}\") -> VOID");
    Some(ApiHookResult::callee(1, None))
}

// =========================================================
// String
// =========================================================
// API: int lstrlenA(LPCSTR lpString)
// 역할: 지정된 문자열의 길이를 바이트 단위로 반환
pub(super) fn lstrlen_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let addr = uc.read_arg(0);
    let s = if addr != 0 {
        uc.read_euc_kr(addr as u64)
    } else {
        String::new()
    };
    let len = s.len() as i32;
    crate::emu_log!("[KERNEL32] lstrlenA(\"{}\") -> int {}", s, len);
    Some(ApiHookResult::callee(1, Some(len)))
}

// API: LPSTR lstrcpyA(LPSTR lpString1, LPCSTR lpString2)
// 역할: 문자열을 한 버퍼에서 다른 버퍼로 복사
pub(super) fn lstrcpy_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let dst = uc.read_arg(0);
    let dst_str = if dst != 0 {
        uc.read_euc_kr(dst as u64)
    } else {
        String::new()
    };
    let src = uc.read_arg(1);
    let src_str = if src != 0 {
        uc.read_euc_kr(src as u64)
    } else {
        String::new()
    };
    uc.write_euc_kr(dst as u64, &src_str);
    crate::emu_log!("[KERNEL32] lstrcpyA(\"{dst_str}\", \"{src_str}\") -> LPSTR {dst:#x}",);
    Some(ApiHookResult::callee(2, Some(dst as i32)))
}

// API: LPSTR lstrcpynA(LPSTR lpString1, LPCSTR lpString2, int iMaxLength)
// 역할: 지정된 수의 문자를 복사
pub(super) fn lstrcpyn_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let dst = uc.read_arg(0);
    let dst_str = if dst != 0 {
        uc.read_euc_kr(dst as u64)
    } else {
        String::new()
    };
    let src = uc.read_arg(1);
    let src_str = if src != 0 {
        uc.read_euc_kr(src as u64)
    } else {
        String::new()
    };
    let max_count = uc.read_arg(2) as usize;
    let (encoded, _, _) = EUC_KR.encode(&src_str);
    let encoded = encoded.as_ref();
    let copy_len = encoded.len().min(max_count.saturating_sub(1));
    let mut bytes = encoded[..copy_len].to_vec();
    bytes.push(0);
    uc.mem_write(dst as u64, &bytes).unwrap();
    crate::emu_log!(
        "[KERNEL32] lstrcpynA(\"{dst_str}\", \"{src_str}\", {max_count}) -> LPSTR {dst:#x}",
    );
    Some(ApiHookResult::callee(3, Some(dst as i32)))
}

// API: LPSTR lstrcatA(LPSTR lpString1, LPCSTR lpString2)
// 역할: 한 문자열을 다른 문자열에 추가
pub(super) fn lstrcat_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let dst = uc.read_arg(0);
    let src = uc.read_arg(1);
    let dst_str = uc.read_euc_kr(dst as u64);
    let src_str = uc.read_euc_kr(src as u64);
    let (encoded_src, _, _) = EUC_KR.encode(&src_str);
    let mut bytes = encoded_src.as_ref().to_vec();
    bytes.push(0);
    let (encoded_dst, _, _) = EUC_KR.encode(&dst_str);
    uc.mem_write(dst as u64 + encoded_dst.len() as u64, &bytes)
        .unwrap();
    crate::emu_log!(
        "[KERNEL32] lstrcatA(\"{}\", \"{}\") -> LPSTR {:#x}",
        dst_str,
        src_str,
        dst
    );
    Some(ApiHookResult::callee(2, Some(dst as i32)))
}

// API: int lstrcmpA(LPCSTR lpString1, LPCSTR lpString2)
// 역할: 두 개의 문자열을 비교
pub(super) fn lstrcmp_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let s1_addr = uc.read_arg(0);
    let s2_addr = uc.read_arg(1);
    let s1 = uc.read_string(s1_addr as u64);
    let s2 = uc.read_string(s2_addr as u64);
    let result = s1.cmp(&s2) as i32;
    crate::emu_log!(
        "[KERNEL32] lstrcmpA(\"{}\", \"{}\") -> int {}",
        s1,
        s2,
        result
    );
    Some(ApiHookResult::callee(2, Some(result)))
}

// =========================================================
// Math / Time
// =========================================================
// API: int MulDiv(int nNumber, int nNumerator, int nDenominator)
// 역할: 두 개의 32비트 값을 곱한 후 세 번째 32비트 값으로 나누고 결과를 32비트 값으로 돌려줌
pub(super) fn mul_div(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let number = uc.read_arg(0) as i32;
    let numerator = uc.read_arg(1) as i32;
    let denominator = uc.read_arg(2) as i32;
    let result = if denominator == 0 {
        -1
    } else {
        ((number as i64 * numerator as i64) / denominator as i64) as i32
    };
    crate::emu_log!(
        "[KERNEL32] MulDiv({}, {}, {}) -> int {}",
        number,
        numerator,
        denominator,
        result
    );
    Some(ApiHookResult::callee(3, Some(result)))
}

// API: DWORD GetTickCount(void)
// 역할: 시스템이 시작된 후 지난 밀리초 시간을 검색
pub(super) fn get_tick_count(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let elapsed = uc.get_data().start_time.elapsed().as_millis() as u32;
    // crate::emu_log!("[KERNEL32] GetTickCount() -> DWORD {}", elapsed);
    Some(ApiHookResult::callee(0, Some(elapsed as i32)))
}

// API: void GetLocalTime(LPSYSTEMTIME lpSystemTime)
// 역할: 현재 로컬 날짜와 시간을 시스템 타임 구조체로 가져옴
pub(super) fn get_local_time(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let buf_addr = uc.read_arg(0);
    // SYSTEMTIME: 8 WORDs = 16 bytes, 0으로 채움
    let zeros = [0u8; 16];
    uc.mem_write(buf_addr as u64, &zeros).unwrap();
    crate::emu_log!("[KERNEL32] GetLocalTime({:#x}) -> VOID", buf_addr);
    Some(ApiHookResult::callee(1, None))
}

// API: BOOL SystemTimeToFileTime(const SYSTEMTIME *lpSystemTime, LPFILETIME lpFileTime)
// 역할: 시스템 시간을 파일 시간 형식으로 변환
pub(super) fn system_time_to_file_time(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let system_time_addr = uc.read_arg(0);
    let file_time_addr = uc.read_arg(1);
    crate::emu_log!(
        "[KERNEL32] SystemTimeToFileTime({:#x}, {:#x}) -> BOOL 1",
        system_time_addr,
        file_time_addr
    );
    Some(ApiHookResult::callee(2, Some(1)))
}
