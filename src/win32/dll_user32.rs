use unicorn_engine::Unicorn;

use crate::win32::Win32Context;

pub struct DllUSER32 {}

impl DllUSER32 {
    pub fn message_box_a() -> Option<(usize, Option<i32>)>{
        println!("message_box_a");
        Some((0, None))
    }

    pub fn close_window() -> Option<(usize, Option<i32>)>{
        println!("close_window");
        Some((0, None))
    }

    pub fn enable_window() -> Option<(usize, Option<i32>)>{
        println!("enable_window");
        Some((0, None))
    }

    pub fn is_window_enabled() -> Option<(usize, Option<i32>)>{
        println!("is_window_enabled");
        Some((0, None))
    }

    pub fn move_window() -> Option<(usize, Option<i32>)>{
        println!("move_window");
        Some((0, None))
    }

    pub fn send_message_a() -> Option<(usize, Option<i32>)>{
        println!("send_message_a");
        Some((0, None))
    }

    pub fn load_cursor_a() -> Option<(usize, Option<i32>)>{
        println!("load_cursor_a");
        Some((0, None))
    }

    pub fn is_dialog_message_a() -> Option<(usize, Option<i32>)>{
        println!("is_dialog_message_a");
        Some((0, None))
    }

    pub fn post_quit_message() -> Option<(usize, Option<i32>)>{
        println!("post_quit_message");
        Some((0, None))
    }

    pub fn set_focus() -> Option<(usize, Option<i32>)>{
        println!("set_focus");
        Some((0, None))
    }

    pub fn dispatch_message_a() -> Option<(usize, Option<i32>)>{
        println!("dispatch_message_a");
        Some((0, None))
    }

    pub fn translate_message() -> Option<(usize, Option<i32>)>{
        println!("translate_message");
        Some((0, None))
    }

    pub fn peek_message_a() -> Option<(usize, Option<i32>)>{
        println!("peek_message_a");
        Some((0, None))
    }

    pub fn msg_wait_for_multiple_objects() -> Option<(usize, Option<i32>)>{
        println!("msg_wait_for_multiple_objects");
        Some((0, None))
    }

    pub fn update_window() -> Option<(usize, Option<i32>)>{
        println!("update_window");
        Some((0, None))
    }

    pub fn get_window() -> Option<(usize, Option<i32>)>{
        println!("get_window");
        Some((0, None))
    }

    pub fn get_menu_item_info_a() -> Option<(usize, Option<i32>)>{
        println!("get_menu_item_info_a");
        Some((0, None))
    }

    pub fn delete_menu() -> Option<(usize, Option<i32>)>{
        println!("delete_menu");
        Some((0, None))
    }

    pub fn get_system_menu() -> Option<(usize, Option<i32>)>{
        println!("get_system_menu");
        Some((0, None))
    }

    pub fn def_window_proc_a() -> Option<(usize, Option<i32>)>{
        println!("def_window_proc_a");
        Some((0, None))
    }

    pub fn create_window_ex_a() -> Option<(usize, Option<i32>)>{
        println!("create_window_ex_a");
        Some((0, None))
    }

    pub fn register_class_ex_a() -> Option<(usize, Option<i32>)>{
        println!("register_class_ex_a");
        Some((0, None))
    }

    pub fn get_class_info_ex_a() -> Option<(usize, Option<i32>)>{
        println!("get_class_info_ex_a");
        Some((0, None))
    }

    pub fn load_icon_a() -> Option<(usize, Option<i32>)>{
        println!("load_icon_a");
        Some((0, None))
    }

    pub fn def_m_d_i_child_proc_a() -> Option<(usize, Option<i32>)>{
        println!("def_m_d_i_child_proc_a");
        Some((0, None))
    }

    pub fn set_window_long_a() -> Option<(usize, Option<i32>)>{
        println!("set_window_long_a");
        Some((0, None))
    }

    pub fn call_window_proc_a() -> Option<(usize, Option<i32>)>{
        println!("call_window_proc_a");
        Some((0, None))
    }

    pub fn def_frame_proc_a() -> Option<(usize, Option<i32>)>{
        println!("def_frame_proc_a");
        Some((0, None))
    }

    pub fn get_message_a() -> Option<(usize, Option<i32>)>{
        println!("get_message_a");
        Some((0, None))
    }

    pub fn post_thread_message_a() -> Option<(usize, Option<i32>)>{
        println!("post_thread_message_a");
        Some((0, None))
    }

    pub fn begin_paint() -> Option<(usize, Option<i32>)>{
        println!("begin_paint");
        Some((0, None))
    }

    pub fn end_paint() -> Option<(usize, Option<i32>)>{
        println!("end_paint");
        Some((0, None))
    }

    pub fn scroll_window_ex() -> Option<(usize, Option<i32>)>{
        println!("scroll_window_ex");
        Some((0, None))
    }

    pub fn invalidate_rect() -> Option<(usize, Option<i32>)>{
        println!("invalidate_rect");
        Some((0, None))
    }

    pub fn set_scroll_info() -> Option<(usize, Option<i32>)>{
        println!("set_scroll_info");
        Some((0, None))
    }

    pub fn get_window_text_a() -> Option<(usize, Option<i32>)>{
        println!("get_window_text_a");
        Some((0, None))
    }

    pub fn get_d_c() -> Option<(usize, Option<i32>)>{
        println!("get_d_c");
        Some((0, None))
    }

    pub fn release_d_c() -> Option<(usize, Option<i32>)>{
        println!("release_d_c");
        Some((0, None))
    }

    pub fn kill_timer() -> Option<(usize, Option<i32>)>{
        println!("kill_timer");
        Some((0, None))
    }

    pub fn set_timer() -> Option<(usize, Option<i32>)>{
        println!("set_timer");
        Some((0, None))
    }

    pub fn remove_menu() -> Option<(usize, Option<i32>)>{
        println!("remove_menu");
        Some((0, None))
    }

    pub fn append_menu_a() -> Option<(usize, Option<i32>)>{
        println!("append_menu_a");
        Some((0, None))
    }

    pub fn create_menu() -> Option<(usize, Option<i32>)>{
        println!("create_menu");
        Some((0, None))
    }

    pub fn destroy_menu() -> Option<(usize, Option<i32>)>{
        println!("destroy_menu");
        Some((0, None))
    }

    pub fn get_desktop_window() -> Option<(usize, Option<i32>)>{
        println!("get_desktop_window");
        Some((0, None))
    }

    pub fn map_window_points() -> Option<(usize, Option<i32>)>{
        println!("map_window_points");
        Some((0, None))
    }

    pub fn system_parameters_info_a() -> Option<(usize, Option<i32>)>{
        println!("system_parameters_info_a");
        Some((0, None))
    }

    pub fn get_menu() -> Option<(usize, Option<i32>)>{
        println!("get_menu");
        Some((0, None))
    }

    pub fn adjust_window_rect_ex() -> Option<(usize, Option<i32>)>{
        println!("adjust_window_rect_ex");
        Some((0, None))
    }

    pub fn get_window_rect() -> Option<(usize, Option<i32>)>{
        println!("get_window_rect");
        Some((0, None))
    }

    pub fn get_client_rect() -> Option<(usize, Option<i32>)>{
        println!("get_client_rect");
        Some((0, None))
    }

    pub fn destroy_window() -> Option<(usize, Option<i32>)>{
        println!("destroy_window");
        Some((0, None))
    }

    pub fn get_parent() -> Option<(usize, Option<i32>)>{
        println!("get_parent");
        Some((0, None))
    }

    pub fn show_window() -> Option<(usize, Option<i32>)>{
        println!("show_window");
        Some((0, None))
    }

    pub fn get_window_long_a() -> Option<(usize, Option<i32>)>{
        println!("get_window_long_a");
        Some((0, None))
    }

    pub fn translate_m_d_i_sys_accel() -> Option<(usize, Option<i32>)>{
        println!("translate_m_d_i_sys_accel");
        Some((0, None))
    }

    pub fn draw_text_a() -> Option<(usize, Option<i32>)>{
        println!("draw_text_a");
        Some((0, None))
    }

    pub fn get_active_window() -> Option<(usize, Option<i32>)>{
        println!("get_active_window");
        Some((0, None))
    }

    pub fn set_window_pos() -> Option<(usize, Option<i32>)>{
        println!("set_window_pos");
        Some((0, None))
    }

    pub fn get_cursor_pos() -> Option<(usize, Option<i32>)>{
        println!("get_cursor_pos");
        Some((0, None))
    }

    pub fn is_window_visible() -> Option<(usize, Option<i32>)>{
        println!("is_window_visible");
        Some((0, None))
    }

    pub fn pt_in_rect() -> Option<(usize, Option<i32>)>{
        println!("pt_in_rect");
        Some((0, None))
    }

    pub fn set_rect() -> Option<(usize, Option<i32>)>{
        println!("set_rect");
        Some((0, None))
    }

    pub fn get_clipboard_data() -> Option<(usize, Option<i32>)>{
        println!("get_clipboard_data");
        Some((0, None))
    }

    pub fn get_focus() -> Option<(usize, Option<i32>)>{
        println!("get_focus");
        Some((0, None))
    }

    pub fn set_capture() -> Option<(usize, Option<i32>)>{
        println!("set_capture");
        Some((0, None))
    }

    pub fn get_capture() -> Option<(usize, Option<i32>)>{
        println!("get_capture");
        Some((0, None))
    }

    pub fn release_capture() -> Option<(usize, Option<i32>)>{
        println!("release_capture");
        Some((0, None))
    }

    pub fn screen_to_client() -> Option<(usize, Option<i32>)>{
        println!("screen_to_client");
        Some((0, None))
    }

    pub fn create_caret() -> Option<(usize, Option<i32>)>{
        println!("create_caret");
        Some((0, None))
    }

    pub fn destroy_caret() -> Option<(usize, Option<i32>)>{
        println!("destroy_caret");
        Some((0, None))
    }

    pub fn get_async_key_state() -> Option<(usize, Option<i32>)>{
        println!("get_async_key_state");
        Some((0, None))
    }

    pub fn show_caret() -> Option<(usize, Option<i32>)>{
        println!("show_caret");
        Some((0, None))
    }

    pub fn set_caret_pos() -> Option<(usize, Option<i32>)>{
        println!("set_caret_pos");
        Some((0, None))
    }

    pub fn hide_caret() -> Option<(usize, Option<i32>)>{
        println!("hide_caret");
        Some((0, None))
    }

    pub fn load_cursor_from_file_a() -> Option<(usize, Option<i32>)>{
        println!("load_cursor_from_file_a");
        Some((0, None))
    }

    pub fn get_sys_color() -> Option<(usize, Option<i32>)>{
        println!("get_sys_color");
        Some((0, None))
    }

    pub fn client_to_screen() -> Option<(usize, Option<i32>)>{
        println!("client_to_screen");
        Some((0, None))
    }

    pub fn close_clipboard() -> Option<(usize, Option<i32>)>{
        println!("close_clipboard");
        Some((0, None))
    }

    pub fn set_clipboard_data() -> Option<(usize, Option<i32>)>{
        println!("set_clipboard_data");
        Some((0, None))
    }

    pub fn empty_clipboard() -> Option<(usize, Option<i32>)>{
        println!("empty_clipboard");
        Some((0, None))
    }

    pub fn open_clipboard() -> Option<(usize, Option<i32>)>{
        println!("open_clipboard");
        Some((0, None))
    }

    pub fn get_window_d_c() -> Option<(usize, Option<i32>)>{
        println!("get_window_d_c");
        Some((0, None))
    }

    pub fn set_window_rgn() -> Option<(usize, Option<i32>)>{
        println!("set_window_rgn");
        Some((0, None))
    }

    pub fn equal_rect() -> Option<(usize, Option<i32>)>{
        println!("equal_rect");
        Some((0, None))
    }

    pub fn get_key_state() -> Option<(usize, Option<i32>)>{
        println!("get_key_state");
        Some((0, None))
    }

    pub fn is_clipboard_format_available() -> Option<(usize, Option<i32>)>{
        println!("is_clipboard_format_available");
        Some((0, None))
    }

    pub fn register_class_a() -> Option<(usize, Option<i32>)>{
        println!("register_class_a");
        Some((0, None))
    }

    pub fn post_message_a() -> Option<(usize, Option<i32>)>{
        println!("post_message_a");
        Some((0, None))
    }

    pub fn is_zoomed() -> Option<(usize, Option<i32>)>{
        println!("is_zoomed");
        Some((0, None))
    }

    pub fn is_iconic() -> Option<(usize, Option<i32>)>{
        println!("is_iconic");
        Some((0, None))
    }

    pub fn set_cursor() -> Option<(usize, Option<i32>)>{
        println!("set_cursor");
        Some((0, None))
    }

    pub fn wsprintf_a() -> Option<(usize, Option<i32>)>{
        println!("wsprintf_a");
        Some((0, None))
    }

    pub fn end_dialog() -> Option<(usize, Option<i32>)>{
        println!("end_dialog");
        Some((0, None))
    }

    pub fn get_last_active_popup() -> Option<(usize, Option<i32>)>{
        println!("get_last_active_popup");
        Some((0, None))
    }

    pub fn union_rect() -> Option<(usize, Option<i32>)>{
        println!("union_rect");
        Some((0, None))
    }

    pub fn destroy_cursor() -> Option<(usize, Option<i32>)>{
        println!("destroy_cursor");
        Some((0, None))
    }

    pub fn intersect_rect() -> Option<(usize, Option<i32>)>{
        println!("intersect_rect");
        Some((0, None))
    }

    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<(usize, Option<i32>)> {
        match func_name {
            "MessageBoxA" => DllUSER32::message_box_a(),
            "CloseWindow" => DllUSER32::close_window(),
            "EnableWindow" => DllUSER32::enable_window(),
            "IsWindowEnabled" => DllUSER32::is_window_enabled(),
            "MoveWindow" => DllUSER32::move_window(),
            "SendMessageA" => DllUSER32::send_message_a(),
            "LoadCursorA" => DllUSER32::load_cursor_a(),
            "IsDialogMessageA" => DllUSER32::is_dialog_message_a(),
            "PostQuitMessage" => DllUSER32::post_quit_message(),
            "SetFocus" => DllUSER32::set_focus(),
            "DispatchMessageA" => DllUSER32::dispatch_message_a(),
            "TranslateMessage" => DllUSER32::translate_message(),
            "PeekMessageA" => DllUSER32::peek_message_a(),
            "MsgWaitForMultipleObjects" => DllUSER32::msg_wait_for_multiple_objects(),
            "UpdateWindow" => DllUSER32::update_window(),
            "GetWindow" => DllUSER32::get_window(),
            "GetMenuItemInfoA" => DllUSER32::get_menu_item_info_a(),
            "DeleteMenu" => DllUSER32::delete_menu(),
            "GetSystemMenu" => DllUSER32::get_system_menu(),
            "DefWindowProcA" => DllUSER32::def_window_proc_a(),
            "CreateWindowExA" => DllUSER32::create_window_ex_a(),
            "RegisterClassExA" => DllUSER32::register_class_ex_a(),
            "GetClassInfoExA" => DllUSER32::get_class_info_ex_a(),
            "LoadIconA" => DllUSER32::load_icon_a(),
            "DefMDIChildProcA" => DllUSER32::def_m_d_i_child_proc_a(),
            "SetWindowLongA" => DllUSER32::set_window_long_a(),
            "CallWindowProcA" => DllUSER32::call_window_proc_a(),
            "DefFrameProcA" => DllUSER32::def_frame_proc_a(),
            "GetMessageA" => DllUSER32::get_message_a(),
            "PostThreadMessageA" => DllUSER32::post_thread_message_a(),
            "BeginPaint" => DllUSER32::begin_paint(),
            "EndPaint" => DllUSER32::end_paint(),
            "ScrollWindowEx" => DllUSER32::scroll_window_ex(),
            "InvalidateRect" => DllUSER32::invalidate_rect(),
            "SetScrollInfo" => DllUSER32::set_scroll_info(),
            "GetWindowTextA" => DllUSER32::get_window_text_a(),
            "GetDC" => DllUSER32::get_d_c(),
            "ReleaseDC" => DllUSER32::release_d_c(),
            "KillTimer" => DllUSER32::kill_timer(),
            "SetTimer" => DllUSER32::set_timer(),
            "RemoveMenu" => DllUSER32::remove_menu(),
            "AppendMenuA" => DllUSER32::append_menu_a(),
            "CreateMenu" => DllUSER32::create_menu(),
            "DestroyMenu" => DllUSER32::destroy_menu(),
            "GetDesktopWindow" => DllUSER32::get_desktop_window(),
            "MapWindowPoints" => DllUSER32::map_window_points(),
            "SystemParametersInfoA" => DllUSER32::system_parameters_info_a(),
            "GetMenu" => DllUSER32::get_menu(),
            "AdjustWindowRectEx" => DllUSER32::adjust_window_rect_ex(),
            "GetWindowRect" => DllUSER32::get_window_rect(),
            "GetClientRect" => DllUSER32::get_client_rect(),
            "DestroyWindow" => DllUSER32::destroy_window(),
            "GetParent" => DllUSER32::get_parent(),
            "ShowWindow" => DllUSER32::show_window(),
            "GetWindowLongA" => DllUSER32::get_window_long_a(),
            "TranslateMDISysAccel" => DllUSER32::translate_m_d_i_sys_accel(),
            "DrawTextA" => DllUSER32::draw_text_a(),
            "GetActiveWindow" => DllUSER32::get_active_window(),
            "SetWindowPos" => DllUSER32::set_window_pos(),
            "GetCursorPos" => DllUSER32::get_cursor_pos(),
            "IsWindowVisible" => DllUSER32::is_window_visible(),
            "PtInRect" => DllUSER32::pt_in_rect(),
            "SetRect" => DllUSER32::set_rect(),
            "GetClipboardData" => DllUSER32::get_clipboard_data(),
            "GetFocus" => DllUSER32::get_focus(),
            "SetCapture" => DllUSER32::set_capture(),
            "GetCapture" => DllUSER32::get_capture(),
            "ReleaseCapture" => DllUSER32::release_capture(),
            "ScreenToClient" => DllUSER32::screen_to_client(),
            "CreateCaret" => DllUSER32::create_caret(),
            "DestroyCaret" => DllUSER32::destroy_caret(),
            "GetAsyncKeyState" => DllUSER32::get_async_key_state(),
            "ShowCaret" => DllUSER32::show_caret(),
            "SetCaretPos" => DllUSER32::set_caret_pos(),
            "HideCaret" => DllUSER32::hide_caret(),
            "LoadCursorFromFileA" => DllUSER32::load_cursor_from_file_a(),
            "GetSysColor" => DllUSER32::get_sys_color(),
            "ClientToScreen" => DllUSER32::client_to_screen(),
            "CloseClipboard" => DllUSER32::close_clipboard(),
            "SetClipboardData" => DllUSER32::set_clipboard_data(),
            "EmptyClipboard" => DllUSER32::empty_clipboard(),
            "OpenClipboard" => DllUSER32::open_clipboard(),
            "GetWindowDC" => DllUSER32::get_window_d_c(),
            "SetWindowRgn" => DllUSER32::set_window_rgn(),
            "EqualRect" => DllUSER32::equal_rect(),
            "GetKeyState" => DllUSER32::get_key_state(),
            "IsClipboardFormatAvailable" => DllUSER32::is_clipboard_format_available(),
            "RegisterClassA" => DllUSER32::register_class_a(),
            "PostMessageA" => DllUSER32::post_message_a(),
            "IsZoomed" => DllUSER32::is_zoomed(),
            "IsIconic" => DllUSER32::is_iconic(),
            "SetCursor" => DllUSER32::set_cursor(),
            "wsprintfA" => DllUSER32::wsprintf_a(),
            "EndDialog" => DllUSER32::end_dialog(),
            "GetLastActivePopup" => DllUSER32::get_last_active_popup(),
            "UnionRect" => DllUSER32::union_rect(),
            "DestroyCursor" => DllUSER32::destroy_cursor(),
            "IntersectRect" => DllUSER32::intersect_rect(),
            _ => None
        }
    }
}
