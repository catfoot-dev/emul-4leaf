use unicorn_engine::Unicorn;

use crate::win32::Win32Context;

pub struct DllKERNEL32 {}

impl DllKERNEL32 {
    pub fn tls_alloc() -> Option<(usize, Option<i32>)>{
        println!("tls_alloc");
        Some((0, None))
    }

    pub fn tls_free() -> Option<(usize, Option<i32>)>{
        println!("tls_free");
        Some((0, None))
    }

    pub fn tls_get_value() -> Option<(usize, Option<i32>)>{
        println!("tls_get_value");
        Some((0, None))
    }

    pub fn tls_set_value() -> Option<(usize, Option<i32>)>{
        println!("tls_set_value");
        Some((0, None))
    }

    pub fn sleep() -> Option<(usize, Option<i32>)>{
        println!("sleep");
        Some((0, None))
    }

    pub fn get_current_thread_id() -> Option<(usize, Option<i32>)>{
        println!("get_current_thread_id");
        Some((0, None))
    }

    pub fn wait_for_single_object() -> Option<(usize, Option<i32>)>{
        println!("wait_for_single_object");
        Some((0, None))
    }

    pub fn terminate_thread() -> Option<(usize, Option<i32>)>{
        println!("terminate_thread");
        Some((0, None))
    }

    pub fn close_handle() -> Option<(usize, Option<i32>)>{
        println!("close_handle");
        Some((0, None))
    }

    pub fn duplicate_handle() -> Option<(usize, Option<i32>)>{
        println!("duplicate_handle");
        Some((0, None))
    }

    pub fn get_current_thread() -> Option<(usize, Option<i32>)>{
        println!("get_current_thread");
        Some((0, None))
    }

    pub fn get_current_process() -> Option<(usize, Option<i32>)>{
        println!("get_current_process");
        Some((0, None))
    }

    pub fn format_message_a() -> Option<(usize, Option<i32>)>{
        println!("format_message_a");
        Some((0, None))
    }

    pub fn get_last_error() -> Option<(usize, Option<i32>)>{
        println!("get_last_error");
        Some((0, None))
    }

    pub fn create_event_a() -> Option<(usize, Option<i32>)>{
        println!("create_event_a");
        Some((0, None))
    }

    pub fn set_event() -> Option<(usize, Option<i32>)>{
        println!("set_event");
        Some((0, None))
    }

    pub fn pulse_event() -> Option<(usize, Option<i32>)>{
        println!("pulse_event");
        Some((0, None))
    }

    pub fn reset_event() -> Option<(usize, Option<i32>)>{
        println!("reset_event");
        Some((0, None))
    }

    pub fn initialize_critical_section() -> Option<(usize, Option<i32>)>{
        println!("initialize_critical_section");
        Some((0, None))
    }

    pub fn delete_critical_section() -> Option<(usize, Option<i32>)>{
        println!("delete_critical_section");
        Some((0, None))
    }

    pub fn enter_critical_section() -> Option<(usize, Option<i32>)>{
        println!("enter_critical_section");
        Some((0, None))
    }

    pub fn leave_critical_section() -> Option<(usize, Option<i32>)>{
        println!("leave_critical_section");
        Some((0, None))
    }

    pub fn output_debug_string_a() -> Option<(usize, Option<i32>)>{
        println!("output_debug_string_a");
        Some((0, None))
    }

    pub fn disable_thread_library_calls() -> Option<(usize, Option<i32>)>{
        println!("disable_thread_library_calls");
        Some((0, None))
    }

    pub fn lstrlen_a() -> Option<(usize, Option<i32>)>{
        println!("lstrlen_a");
        Some((0, None))
    }

    pub fn mul_div() -> Option<(usize, Option<i32>)>{
        println!("mul_div");
        Some((0, None))
    }

    pub fn lstrcpyn_a() -> Option<(usize, Option<i32>)>{
        println!("lstrcpyn_a");
        Some((0, None))
    }

    pub fn set_last_error() -> Option<(usize, Option<i32>)>{
        println!("set_last_error");
        Some((0, None))
    }

    pub fn get_module_handle_a() -> Option<(usize, Option<i32>)>{
        println!("get_module_handle_a");
        Some((0, None))
    }

    pub fn interlocked_exchange() -> Option<(usize, Option<i32>)>{
        println!("interlocked_exchange");
        Some((0, None))
    }

    pub fn get_tick_count() -> Option<(usize, Option<i32>)>{
        println!("get_tick_count");
        Some((0, None))
    }

    pub fn lstrcpy_a() -> Option<(usize, Option<i32>)>{
        println!("lstrcpy_a");
        Some((0, None))
    }

    pub fn lstrcat_a() -> Option<(usize, Option<i32>)>{
        println!("lstrcat_a");
        Some((0, None))
    }

    pub fn global_alloc() -> Option<(usize, Option<i32>)>{
        println!("global_alloc");
        Some((0, None))
    }

    pub fn global_lock() -> Option<(usize, Option<i32>)>{
        println!("global_lock");
        Some((0, None))
    }

    pub fn global_unlock() -> Option<(usize, Option<i32>)>{
        println!("global_unlock");
        Some((0, None))
    }

    pub fn global_free() -> Option<(usize, Option<i32>)>{
        println!("global_free");
        Some((0, None))
    }

    pub fn set_thread_priority() -> Option<(usize, Option<i32>)>{
        println!("set_thread_priority");
        Some((0, None))
    }

    pub fn free_library() -> Option<(usize, Option<i32>)>{
        println!("free_library");
        Some((0, None))
    }

    pub fn find_next_file_a() -> Option<(usize, Option<i32>)>{
        println!("find_next_file_a");
        Some((0, None))
    }

    pub fn find_close() -> Option<(usize, Option<i32>)>{
        println!("find_close");
        Some((0, None))
    }

    pub fn get_file_attributes_a() -> Option<(usize, Option<i32>)>{
        println!("get_file_attributes_a");
        Some((0, None))
    }

    pub fn remove_directory_a() -> Option<(usize, Option<i32>)>{
        println!("remove_directory_a");
        Some((0, None))
    }

    pub fn get_temp_path_a() -> Option<(usize, Option<i32>)>{
        println!("get_temp_path_a");
        Some((0, None))
    }

    pub fn system_time_to_file_time() -> Option<(usize, Option<i32>)>{
        println!("system_time_to_file_time");
        Some((0, None))
    }

    pub fn wait_for_multiple_objects() -> Option<(usize, Option<i32>)>{
        println!("wait_for_multiple_objects");
        Some((0, None))
    }

    pub fn get_short_path_name_a() -> Option<(usize, Option<i32>)>{
        println!("get_short_path_name_a");
        Some((0, None))
    }

    pub fn lstrcmp_a() -> Option<(usize, Option<i32>)>{
        println!("lstrcmp_a");
        Some((0, None))
    }

    pub fn get_local_time() -> Option<(usize, Option<i32>)>{
        println!("get_local_time");
        Some((0, None))
    }

    pub fn create_directory_a() -> Option<(usize, Option<i32>)>{
        println!("create_directory_a");
        Some((0, None))
    }

    pub fn delete_file_a() -> Option<(usize, Option<i32>)>{
        println!("delete_file_a");
        Some((0, None))
    }

    pub fn copy_file_a() -> Option<(usize, Option<i32>)>{
        println!("copy_file_a");
        Some((0, None))
    }

    pub fn release_mutex() -> Option<(usize, Option<i32>)>{
        println!("release_mutex");
        Some((0, None))
    }

    pub fn create_process_a() -> Option<(usize, Option<i32>)>{
        println!("create_process_a");
        Some((0, None))
    }

    pub fn create_mutex_a() -> Option<(usize, Option<i32>)>{
        println!("create_mutex_a");
        Some((0, None))
    }

    pub fn find_first_file_a() -> Option<(usize, Option<i32>)>{
        println!("find_first_file_a");
        Some((0, None))
    }

    pub fn get_full_path_name_a() -> Option<(usize, Option<i32>)>{
        println!("get_full_path_name_a");
        Some((0, None))
    }

    pub fn get_module_file_name_a() -> Option<(usize, Option<i32>)>{
        println!("get_module_file_name_a");
        Some((0, None))
    }

    pub fn get_long_path_name_a() -> Option<(usize, Option<i32>)>{
        println!("get_long_path_name_a");
        Some((0, None))
    }

    pub fn set_file_time() -> Option<(usize, Option<i32>)>{
        println!("set_file_time");
        Some((0, None))
    }

    pub fn create_file_a() -> Option<(usize, Option<i32>)>{
        println!("create_file_a");
        Some((0, None))
    }

    pub fn get_proc_address() -> Option<(usize, Option<i32>)>{
        println!("get_proc_address");
        Some((0, None))
    }

    pub fn load_library_a() -> Option<(usize, Option<i32>)>{
        println!("load_library_a");
        Some((0, None))
    }

    pub fn set_file_attributes_a() -> Option<(usize, Option<i32>)>{
        println!("set_file_attributes_a");
        Some((0, None))
    }


    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<(usize, Option<i32>)> {
        match func_name {
            "TlsAlloc" => DllKERNEL32::tls_alloc(),
            "TlsFree" => DllKERNEL32::tls_free(),
            "TlsGetValue" => DllKERNEL32::tls_get_value(),
            "TlsSetValue" => DllKERNEL32::tls_set_value(),
            "Sleep" => DllKERNEL32::sleep(),
            "GetCurrentThreadId" => DllKERNEL32::get_current_thread_id(),
            "WaitForSingleObject" => DllKERNEL32::wait_for_single_object(),
            "TerminateThread" => DllKERNEL32::terminate_thread(),
            "CloseHandle" => DllKERNEL32::close_handle(),
            "DuplicateHandle" => DllKERNEL32::duplicate_handle(),
            "GetCurrentThread" => DllKERNEL32::get_current_thread(),
            "GetCurrentProcess" => DllKERNEL32::get_current_process(),
            "FormatMessageA" => DllKERNEL32::format_message_a(),
            "GetLastError" => DllKERNEL32::get_last_error(),
            "CreateEventA" => DllKERNEL32::create_event_a(),
            "SetEvent" => DllKERNEL32::set_event(),
            "PulseEvent" => DllKERNEL32::pulse_event(),
            "ResetEvent" => DllKERNEL32::reset_event(),
            "InitializeCriticalSection" => DllKERNEL32::initialize_critical_section(),
            "DeleteCriticalSection" => DllKERNEL32::delete_critical_section(),
            "EnterCriticalSection" => DllKERNEL32::enter_critical_section(),
            "LeaveCriticalSection" => DllKERNEL32::leave_critical_section(),
            "OutputDebugStringA" => DllKERNEL32::output_debug_string_a(),
            "DisableThreadLibraryCalls" => DllKERNEL32::disable_thread_library_calls(),
            "lstrlenA" => DllKERNEL32::lstrlen_a(),
            "MulDiv" => DllKERNEL32::mul_div(),
            "lstrcpynA" => DllKERNEL32::lstrcpyn_a(),
            "SetLastError" => DllKERNEL32::set_last_error(),
            "GetModuleHandleA" => DllKERNEL32::get_module_handle_a(),
            "InterlockedExchange" => DllKERNEL32::interlocked_exchange(),
            "GetTickCount" => DllKERNEL32::get_tick_count(),
            "lstrcpyA" => DllKERNEL32::lstrcpy_a(),
            "lstrcatA" => DllKERNEL32::lstrcat_a(),
            "GlobalAlloc" => DllKERNEL32::global_alloc(),
            "GlobalLock" => DllKERNEL32::global_lock(),
            "GlobalUnlock" => DllKERNEL32::global_unlock(),
            "GlobalFree" => DllKERNEL32::global_free(),
            "SetThreadPriority" => DllKERNEL32::set_thread_priority(),
            "FreeLibrary" => DllKERNEL32::free_library(),
            "FindNextFileA" => DllKERNEL32::find_next_file_a(),
            "FindClose" => DllKERNEL32::find_close(),
            "GetFileAttributesA" => DllKERNEL32::get_file_attributes_a(),
            "RemoveDirectoryA" => DllKERNEL32::remove_directory_a(),
            "GetTempPathA" => DllKERNEL32::get_temp_path_a(),
            "SystemTimeToFileTime" => DllKERNEL32::system_time_to_file_time(),
            "WaitForMultipleObjects" => DllKERNEL32::wait_for_multiple_objects(),
            "GetShortPathNameA" => DllKERNEL32::get_short_path_name_a(),
            "lstrcmpA" => DllKERNEL32::lstrcmp_a(),
            "GetLocalTime" => DllKERNEL32::get_local_time(),
            "CreateDirectoryA" => DllKERNEL32::create_directory_a(),
            "DeleteFileA" => DllKERNEL32::delete_file_a(),
            "CopyFileA" => DllKERNEL32::copy_file_a(),
            "ReleaseMutex" => DllKERNEL32::release_mutex(),
            "CreateProcessA" => DllKERNEL32::create_process_a(),
            "CreateMutexA" => DllKERNEL32::create_mutex_a(),
            "FindFirstFileA" => DllKERNEL32::find_first_file_a(),
            "GetFullPathNameA" => DllKERNEL32::get_full_path_name_a(),
            "GetModuleFileNameA" => DllKERNEL32::get_module_file_name_a(),
            "GetLongPathNameA" => DllKERNEL32::get_long_path_name_a(),
            "SetFileTime" => DllKERNEL32::set_file_time(),
            "CreateFileA" => DllKERNEL32::create_file_a(),
            "GetProcAddress" => DllKERNEL32::get_proc_address(),
            "LoadLibraryA" => DllKERNEL32::load_library_a(),
            "SetFileAttributesA" => DllKERNEL32::set_file_attributes_a(),
            _ => None
        }
    }
}
