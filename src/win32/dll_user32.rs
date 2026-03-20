use unicorn_engine::Unicorn;

use crate::helper::UnicornHelper;
use crate::win32::{
    ApiHookResult, Win32Context, WindowClass, WindowState, callee_result, caller_result,
};

pub struct DllUSER32 {}

impl DllUSER32 {
    fn wrap_result(func_name: &str, result: Option<(usize, Option<i32>)>) -> Option<ApiHookResult> {
        match func_name {
            "wsprintfA" => caller_result(result),
            _ => callee_result(result),
        }
    }

    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        DllUSER32::wrap_result(
            func_name,
            match func_name {
                "MessageBoxA" => {
                    let _hwnd = uc.read_arg(0);
                    let text_addr = uc.read_arg(1);
                    let caption_addr = uc.read_arg(2);
                    let text = uc.read_string(text_addr as u64);
                    let caption = uc.read_string(caption_addr as u64);
                    println!("[USER32] MessageBoxA(\"{}\", \"{}\")", caption, text);
                    Some((4, Some(1))) // IDOK
                }
                "RegisterClassExA" => {
                    // WNDCLASSEX는 48 bytes
                    let class_addr = uc.read_arg(0);
                    let wnd_proc = uc.read_u32(class_addr as u64 + 8);
                    let class_name_ptr = uc.read_u32(class_addr as u64 + 40);
                    let class_name = uc.read_string(class_name_ptr as u64);
                    let ctx = uc.get_data_mut();
                    let atom = ctx.alloc_handle();
                    ctx.window_classes.insert(
                        class_name.clone(),
                        WindowClass {
                            class_name: class_name.clone(),
                            wnd_proc,
                            style: 0,
                            hinstance: 0,
                        },
                    );
                    println!(
                        "[USER32] RegisterClassExA(\"{}\") -> atom {:#x}",
                        class_name, atom
                    );
                    Some((1, Some(atom as i32)))
                }
                "RegisterClassA" => {
                    let class_addr = uc.read_arg(0);
                    let wnd_proc = uc.read_u32(class_addr as u64 + 4);
                    let class_name_ptr = uc.read_u32(class_addr as u64 + 36);
                    let class_name = uc.read_string(class_name_ptr as u64);
                    let ctx = uc.get_data_mut();
                    let atom = ctx.alloc_handle();
                    ctx.window_classes.insert(
                        class_name.clone(),
                        WindowClass {
                            class_name: class_name.clone(),
                            wnd_proc,
                            style: 0,
                            hinstance: 0,
                        },
                    );
                    println!(
                        "[USER32] RegisterClassA(\"{}\") -> atom {:#x}",
                        class_name, atom
                    );
                    Some((1, Some(atom as i32)))
                }
                "CreateWindowExA" => {
                    let _ex_style = uc.read_arg(0);
                    let class_addr = uc.read_arg(1);
                    let title_addr = uc.read_arg(2);
                    let _style = uc.read_arg(3);
                    let class_name = if class_addr < 0x10000 {
                        format!("Atom_{}", class_addr)
                    } else {
                        uc.read_string(class_addr as u64)
                    };
                    let title = if title_addr != 0 {
                        uc.read_string(title_addr as u64)
                    } else {
                        String::new()
                    };
                    let ctx = uc.get_data_mut();
                    let hwnd = ctx.alloc_handle();
                    ctx.windows.insert(
                        hwnd,
                        WindowState {
                            class_name: class_name.clone(),
                            title: title.clone(),
                            x: 0,
                            y: 0,
                            width: 640,
                            height: 480,
                            style: _style,
                            parent: 0,
                            visible: false,
                            wnd_proc: 0,
                            user_data: 0,
                        },
                    );
                    println!(
                        "[USER32] CreateWindowExA(\"{}\", \"{}\") -> HWND {:#x}",
                        class_name, title, hwnd
                    );
                    Some((12, Some(hwnd as i32)))
                }
                "ShowWindow" => Some((2, Some(0))),
                "UpdateWindow" => Some((1, Some(1))),
                "DestroyWindow" => Some((1, Some(1))),
                "CloseWindow" => Some((1, Some(1))),
                "EnableWindow" => Some((2, Some(0))),
                "IsWindowEnabled" => Some((1, Some(1))),
                "IsWindowVisible" => Some((1, Some(0))),
                "MoveWindow" => Some((6, Some(1))),
                "SetWindowPos" => Some((7, Some(1))),
                "GetWindowRect" => {
                    let _hwnd = uc.read_arg(0);
                    let rect_addr = uc.read_arg(1);
                    // RECT: left, top, right, bottom (4 x i32)
                    uc.write_u32(rect_addr as u64, 0);
                    uc.write_u32(rect_addr as u64 + 4, 0);
                    uc.write_u32(rect_addr as u64 + 8, 640);
                    uc.write_u32(rect_addr as u64 + 12, 480);
                    Some((2, Some(1)))
                }
                "GetClientRect" => {
                    let _hwnd = uc.read_arg(0);
                    let rect_addr = uc.read_arg(1);
                    uc.write_u32(rect_addr as u64, 0);
                    uc.write_u32(rect_addr as u64 + 4, 0);
                    uc.write_u32(rect_addr as u64 + 8, 640);
                    uc.write_u32(rect_addr as u64 + 12, 480);
                    Some((2, Some(1)))
                }
                "AdjustWindowRectEx" => Some((4, Some(1))),
                "GetDC" => {
                    let ctx = uc.get_data_mut();
                    let hdc = ctx.alloc_handle();
                    println!("[USER32] GetDC(...) -> HDC {:#x}", hdc);
                    Some((1, Some(hdc as i32)))
                }
                "GetWindowDC" => {
                    let ctx = uc.get_data_mut();
                    let hdc = ctx.alloc_handle();
                    Some((1, Some(hdc as i32)))
                }
                "ReleaseDC" => Some((2, Some(1))),
                "SendMessageA" => Some((4, Some(0))),
                "PostMessageA" => Some((4, Some(1))),
                "LoadCursorA" => Some((2, Some(0x1001))),
                "LoadCursorFromFileA" => Some((1, Some(0x1002))),
                "LoadIconA" => Some((2, Some(0x1003))),
                "SetCursor" => Some((1, Some(0))),
                "DestroyCursor" => Some((1, Some(1))),
                "IsDialogMessageA" => Some((2, Some(0))),
                "PostQuitMessage" => Some((1, None)),
                "SetFocus" => Some((1, Some(0))),
                "GetFocus" => Some((0, Some(0))),
                "DispatchMessageA" => Some((1, Some(0))),
                "TranslateMessage" => Some((1, Some(0))),
                "PeekMessageA" => Some((5, Some(0))), // WM_QUIT 없음
                "GetMessageA" => Some((4, Some(0))),  // WM_QUIT (0 = 종료)
                "MsgWaitForMultipleObjects" => Some((5, Some(0))),
                "GetWindow" => Some((2, Some(0))),
                "GetParent" => Some((1, Some(0))),
                "GetDesktopWindow" => Some((0, Some(0x0001))),
                "GetActiveWindow" => Some((0, Some(0))),
                "GetLastActivePopup" => Some((1, Some(0))),
                "GetMenuItemInfoA" => Some((4, Some(0))),
                "DeleteMenu" => Some((3, Some(1))),
                "RemoveMenu" => Some((3, Some(1))),
                "GetSystemMenu" => Some((2, Some(0))),
                "GetMenu" => Some((1, Some(0))),
                "AppendMenuA" => Some((4, Some(1))),
                "CreateMenu" => {
                    let ctx = uc.get_data_mut();
                    let hmenu = ctx.alloc_handle();
                    Some((0, Some(hmenu as i32)))
                }
                "DestroyMenu" => Some((1, Some(1))),
                "DefWindowProcA" => Some((4, Some(0))),
                "DefMDIChildProcA" => Some((4, Some(0))),
                "DefFrameProcA" => Some((5, Some(0))),
                "SetWindowLongA" => Some((3, Some(0))),
                "GetWindowLongA" => Some((2, Some(0))),
                "CallWindowProcA" => Some((5, Some(0))),
                "PostThreadMessageA" => Some((4, Some(1))),
                "BeginPaint" => {
                    let _hwnd = uc.read_arg(0);
                    let ps_addr = uc.read_arg(1);
                    let ctx = uc.get_data_mut();
                    let hdc = ctx.alloc_handle();
                    // PAINTSTRUCT: HDC at offset 0
                    uc.write_u32(ps_addr as u64, hdc);
                    Some((2, Some(hdc as i32)))
                }
                "EndPaint" => Some((2, Some(1))),
                "ScrollWindowEx" => Some((8, Some(0))),
                "InvalidateRect" => Some((3, Some(1))),
                "SetScrollInfo" => Some((4, Some(0))),
                "GetWindowTextA" => Some((3, Some(0))),
                "KillTimer" => Some((2, Some(1))),
                "SetTimer" => {
                    let hwnd = uc.read_arg(0);
                    let id = uc.read_arg(1);
                    println!("[USER32] SetTimer({:#x}, {})", hwnd, id);
                    Some((4, Some(id as i32)))
                }
                "MapWindowPoints" => Some((4, Some(0))),
                "SystemParametersInfoA" => Some((4, Some(1))),
                "TranslateMDISysAccel" => Some((2, Some(0))),
                "DrawTextA" => Some((5, Some(0))),
                "GetCursorPos" => {
                    let pt_addr = uc.read_arg(0);
                    uc.write_u32(pt_addr as u64, 320);
                    uc.write_u32(pt_addr as u64 + 4, 240);
                    Some((1, Some(1)))
                }
                "PtInRect" => Some((2, Some(0))),
                "SetRect" => Some((5, Some(1))),
                "EqualRect" => Some((2, Some(0))),
                "UnionRect" => Some((3, Some(1))),
                "IntersectRect" => Some((3, Some(0))),
                "GetClipboardData" => Some((1, Some(0))),
                "OpenClipboard" => Some((1, Some(1))),
                "CloseClipboard" => Some((0, Some(1))),
                "EmptyClipboard" => Some((0, Some(1))),
                "SetClipboardData" => Some((2, Some(0))),
                "IsClipboardFormatAvailable" => Some((1, Some(0))),
                "SetCapture" => Some((1, Some(0))),
                "GetCapture" => Some((0, Some(0))),
                "ReleaseCapture" => Some((0, Some(1))),
                "ScreenToClient" => Some((2, Some(1))),
                "ClientToScreen" => Some((2, Some(1))),
                "CreateCaret" => Some((4, Some(1))),
                "DestroyCaret" => Some((0, Some(1))),
                "ShowCaret" => Some((1, Some(1))),
                "HideCaret" => Some((1, Some(1))),
                "SetCaretPos" => Some((2, Some(1))),
                "GetAsyncKeyState" => Some((1, Some(0))),
                "GetKeyState" => Some((1, Some(0))),
                "GetSysColor" => Some((1, Some(0x00C0C0C0u32 as i32))), // COLOR_BTNFACE
                "SetWindowRgn" => Some((3, Some(1))),
                "GetClassInfoExA" => Some((3, Some(0))),
                "IsZoomed" => Some((1, Some(0))),
                "IsIconic" => Some((1, Some(0))),
                "wsprintfA" => {
                    let buf_addr = uc.read_arg(0);
                    let fmt_addr = uc.read_arg(1);
                    if buf_addr == 0 || fmt_addr == 0 {
                        let eip = uc
                            .reg_read(unicorn_engine::RegisterX86::EIP as i32)
                            .unwrap();
                        let esp = uc
                            .reg_read(unicorn_engine::RegisterX86::ESP as i32)
                            .unwrap();
                        println!(
                            "[USER32] wsprintfA invalid args: buf={:#x}, fmt={:#x}, eip={:#x}, esp={:#x}",
                            buf_addr, fmt_addr, eip, esp
                        );
                    } else {
                        println!("[USER32] wsprintfA(...)");
                    }
                    Some((2, Some(0)))
                }
                "EndDialog" => Some((2, Some(1))),
                _ => {
                    println!("[USER32] UNHANDLED: {}", func_name);
                    None
                }
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::win32::StackCleanup;

    #[test]
    fn wsprintf_uses_caller_cleanup() {
        let result = DllUSER32::wrap_result("wsprintfA", Some((2, Some(0)))).unwrap();
        assert_eq!(result.cleanup, StackCleanup::Caller);
    }

    #[test]
    fn message_box_keeps_callee_cleanup() {
        let result = DllUSER32::wrap_result("MessageBoxA", Some((4, Some(1)))).unwrap();
        assert_eq!(result.cleanup, StackCleanup::Callee(4));
    }
}
