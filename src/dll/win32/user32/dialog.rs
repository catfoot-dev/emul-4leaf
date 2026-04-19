use crate::{
    dll::win32::{ApiHookResult, Win32Context},
    helper::UnicornHelper,
};
use encoding_rs::EUC_KR;
use unicorn_engine::Unicorn;

use super::USER32;

// API: int MessageBoxA(HWND hWnd, LPCSTR lpText, LPCSTR lpCaption, UINT uType)
// 역할: 메시지 박스를 화면에 표시
pub(super) fn message_box_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let text_addr = uc.read_arg(1);
    let caption_addr = uc.read_arg(2);
    let u_type = uc.read_arg(3);
    let text = uc.read_euc_kr(text_addr as u64);
    let caption = uc.read_euc_kr(caption_addr as u64);

    let result =
        uc.get_data()
            .win_event
            .lock()
            .unwrap()
            .message_box(caption.clone(), text.clone(), u_type);

    crate::emu_log!(
        "[USER32] MessageBoxA({:#x}, \"{}\", \"{}\", {:#x}) -> int {:#x}",
        hwnd,
        caption,
        text,
        u_type,
        result
    );
    Some(ApiHookResult::callee(4, Some(result)))
}

// API: BOOL EndDialog(HWND hDlg, INT_PTR nResult)
// 역할: 다이얼로그를 닫음
pub(super) fn end_dialog(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let h_dlg = uc.read_arg(0);
    let n_result = uc.read_arg(1);

    // 간단한 구현: 다이얼로그 윈도우를 파괴함
    super::USER32::destroy_window_tree(uc.get_data(), h_dlg);

    crate::emu_log!(
        "[USER32] EndDialog({:#x}, {}) -> BOOL 1",
        h_dlg,
        n_result as i32
    );
    Some(ApiHookResult::callee(2, Some(1)))
}

// API: int wsprintfA(LPSTR lpOut, LPCSTR lpFmt, ...)
// 역할: 문자열을 포맷팅하여 출력
pub(super) fn wsprintf_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let buf_addr = uc.read_arg(0);
    let fmt_addr = uc.read_arg(1);
    if buf_addr != 0 && fmt_addr != 0 {
        let fmt = uc.read_euc_kr(fmt_addr as u64);
        let mut arg_idx = 2;
        let mut formatted = String::new();
        let mut chars = fmt.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '%' {
                if let Some(&next) = chars.peek() {
                    if next == '%' {
                        formatted.push('%');
                        chars.next();
                    } else {
                        let mut pad_char = ' ';
                        let mut padding = 0;
                        let mut format_char = *chars.peek().unwrap();
                        if format_char == '0' {
                            pad_char = '0';
                            chars.next();
                            format_char = *chars.peek().unwrap_or(&' ');
                        }
                        while format_char.is_ascii_digit() {
                            padding = padding * 10 + format_char.to_digit(10).unwrap();
                            chars.next();
                            format_char = *chars.peek().unwrap_or(&' ');
                        }
                        if format_char == 'l' || format_char == 'h' {
                            chars.next();
                            format_char = *chars.peek().unwrap_or(&' ');
                        }
                        chars.next();
                        let arg_val = uc.read_arg(arg_idx);
                        arg_idx += 1;
                        let mut s = match format_char {
                            'd' | 'i' => format!("{}", arg_val as i32),
                            'u' => format!("{}", { arg_val }),
                            'x' => format!("{:x}", { arg_val }),
                            'X' => format!("{:X}", { arg_val }),
                            'c' => format!("{}", (arg_val as u8) as char),
                            's' | 'S' => {
                                if arg_val != 0 {
                                    uc.read_euc_kr(arg_val as u64)
                                } else {
                                    "(null)".to_string()
                                }
                            }
                            _ => format!("%{}", format_char),
                        };
                        if padding > 0 && s.len() < padding as usize {
                            let pad = padding as usize - s.len();
                            s = pad_char.to_string().repeat(pad) + &s;
                        }
                        formatted.push_str(&s);
                    }
                }
            } else {
                formatted.push(c);
            }
        }
        let (encoded, _, _) = EUC_KR.encode(&formatted);
        crate::emu_log!(
            "[USER32] wsprintfA({:#x}, {:#x}) -> int {}",
            buf_addr,
            fmt_addr,
            encoded.len()
        );
        USER32::write_ansi_bytes(uc, buf_addr as u64, encoded.as_ref());
        Some(ApiHookResult::callee(arg_idx, Some(encoded.len() as i32)))
    } else {
        Some(ApiHookResult::callee(2, Some(0)))
    }
}

// API: BOOL GetPropA(HWND hWnd, LPCSTR lpString)
// 역할: 윈도우에서 프로퍼티를 가져옴
pub(super) fn get_prop_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    crate::emu_log!("[USER32] GetPropA({:#x}) -> 0", hwnd);
    Some(ApiHookResult::callee(2, Some(0)))
}

// API: BOOL SetPropA(HWND hWnd, LPCSTR lpString, HANDLE hData)
// 역할: 윈도우에 프로퍼티를 설정
pub(super) fn set_prop_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    crate::emu_log!("[USER32] SetPropA({:#x}) -> 1", hwnd);
    Some(ApiHookResult::callee(3, Some(1)))
}

// API: HANDLE RemovePropA(HWND hWnd, LPCSTR lpString)
// 역할: 윈도우에서 프로퍼티를 제거
pub(super) fn remove_prop_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    crate::emu_log!("[USER32] RemovePropA({:#x}) -> 0", hwnd);
    Some(ApiHookResult::callee(2, Some(0)))
}
