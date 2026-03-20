use unicorn_engine::Unicorn;

use crate::helper::UnicornHelper;
use crate::win32::{ApiHookResult, EventState, Win32Context, callee_result};

pub struct DllKERNEL32 {}

impl DllKERNEL32 {
    // =========================================================
    // TLS (Thread Local Storage)
    // =========================================================
    pub fn tls_alloc(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let ctx = uc.get_data_mut();
        let index = ctx.tls_counter;
        ctx.tls_counter += 1;
        ctx.tls_slots.insert(index, 0);
        println!("[KERNEL32] TlsAlloc() -> {}", index);
        Some((0, Some(index as i32)))
    }

    pub fn tls_free(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let index = uc.read_arg(0);
        let ctx = uc.get_data_mut();
        ctx.tls_slots.remove(&index);
        println!("[KERNEL32] TlsFree({})", index);
        Some((1, Some(1))) // TRUE
    }

    pub fn tls_get_value(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let index = uc.read_arg(0);
        let ctx = uc.get_data_mut();
        let value = *ctx.tls_slots.get(&index).unwrap_or(&0);
        println!("[KERNEL32] TlsGetValue({}) -> {:#x}", index, value);
        Some((1, Some(value as i32)))
    }

    pub fn tls_set_value(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let index = uc.read_arg(0);
        let value = uc.read_arg(1);
        let ctx = uc.get_data_mut();
        ctx.tls_slots.insert(index, value);
        println!("[KERNEL32] TlsSetValue({}, {:#x})", index, value);
        Some((2, Some(1))) // TRUE
    }

    // =========================================================
    // Thread / Process
    // =========================================================
    pub fn sleep(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        // sleep은 단일 스레드 에뮬이므로 no-op
        println!("[KERNEL32] Sleep(...)");
        Some((1, None))
    }

    pub fn get_current_thread_id(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[KERNEL32] GetCurrentThreadId() -> 1");
        Some((0, Some(1)))
    }

    pub fn get_current_thread(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[KERNEL32] GetCurrentThread() -> 0xFFFFFFFE");
        Some((0, Some(-2i32))) // pseudo handle
    }

    pub fn get_current_process(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[KERNEL32] GetCurrentProcess() -> 0xFFFFFFFF");
        Some((0, Some(-1i32))) // pseudo handle
    }

    pub fn wait_for_single_object(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[KERNEL32] WaitForSingleObject(...)");
        Some((2, Some(0))) // WAIT_OBJECT_0
    }

    pub fn wait_for_multiple_objects(
        _uc: &mut Unicorn<Win32Context>,
    ) -> Option<(usize, Option<i32>)> {
        println!("[KERNEL32] WaitForMultipleObjects(...)");
        Some((4, Some(0))) // WAIT_OBJECT_0
    }

    pub fn terminate_thread(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[KERNEL32] TerminateThread(...)");
        Some((2, Some(1)))
    }

    pub fn set_thread_priority(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[KERNEL32] SetThreadPriority(...)");
        Some((2, Some(1)))
    }

    pub fn disable_thread_library_calls(
        _uc: &mut Unicorn<Win32Context>,
    ) -> Option<(usize, Option<i32>)> {
        println!("[KERNEL32] DisableThreadLibraryCalls(...)");
        Some((1, Some(1))) // TRUE
    }

    pub fn create_process_a(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[KERNEL32] CreateProcessA(...)");
        Some((10, Some(0))) // FALSE
    }

    // =========================================================
    // Handle
    // =========================================================
    pub fn close_handle(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[KERNEL32] CloseHandle(...)");
        Some((1, Some(1))) // TRUE
    }

    pub fn duplicate_handle(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[KERNEL32] DuplicateHandle(...)");
        Some((7, Some(1))) // TRUE
    }

    // =========================================================
    // Error
    // =========================================================
    pub fn get_last_error(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let err = uc.get_data_mut().last_error;
        println!("[KERNEL32] GetLastError() -> {}", err);
        Some((0, Some(err as i32)))
    }

    pub fn set_last_error(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let code = uc.read_arg(0);
        uc.get_data_mut().last_error = code;
        println!("[KERNEL32] SetLastError({})", code);
        Some((1, None))
    }

    pub fn format_message_a(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[KERNEL32] FormatMessageA(...)");
        Some((7, Some(0)))
    }

    // =========================================================
    // Event / Sync
    // =========================================================
    pub fn create_event_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let manual_reset = uc.read_arg(1);
        let initial_state = uc.read_arg(2);
        let ctx = uc.get_data_mut();
        let handle = ctx.alloc_handle();
        ctx.events.insert(
            handle,
            EventState {
                signaled: initial_state != 0,
                manual_reset: manual_reset != 0,
            },
        );
        println!("[KERNEL32] CreateEventA(...) -> handle {:#x}", handle);
        Some((4, Some(handle as i32)))
    }

    pub fn set_event(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let handle = uc.read_arg(0);
        let ctx = uc.get_data_mut();
        if let Some(evt) = ctx.events.get_mut(&handle) {
            evt.signaled = true;
        }
        println!("[KERNEL32] SetEvent({:#x})", handle);
        Some((1, Some(1)))
    }

    pub fn pulse_event(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let handle = uc.read_arg(0);
        println!("[KERNEL32] PulseEvent({:#x})", handle);
        Some((1, Some(1)))
    }

    pub fn reset_event(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let handle = uc.read_arg(0);
        let ctx = uc.get_data_mut();
        if let Some(evt) = ctx.events.get_mut(&handle) {
            evt.signaled = false;
        }
        println!("[KERNEL32] ResetEvent({:#x})", handle);
        Some((1, Some(1)))
    }

    // Critical Section (싱글 스레드이므로 no-op)
    pub fn initialize_critical_section(
        _uc: &mut Unicorn<Win32Context>,
    ) -> Option<(usize, Option<i32>)> {
        println!("[KERNEL32] InitializeCriticalSection(...)");
        Some((1, None))
    }

    pub fn delete_critical_section(
        _uc: &mut Unicorn<Win32Context>,
    ) -> Option<(usize, Option<i32>)> {
        println!("[KERNEL32] DeleteCriticalSection(...)");
        Some((1, None))
    }

    pub fn enter_critical_section(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[KERNEL32] EnterCriticalSection(...)");
        Some((1, None))
    }

    pub fn leave_critical_section(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[KERNEL32] LeaveCriticalSection(...)");
        Some((1, None))
    }

    // Mutex
    pub fn create_mutex_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let ctx = uc.get_data_mut();
        let handle = ctx.alloc_handle();
        println!("[KERNEL32] CreateMutexA(...) -> {:#x}", handle);
        Some((3, Some(handle as i32)))
    }

    pub fn release_mutex(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[KERNEL32] ReleaseMutex(...)");
        Some((1, Some(1)))
    }

    // =========================================================
    // Debug
    // =========================================================
    pub fn output_debug_string_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let addr = uc.read_arg(0);
        let s = uc.read_string(addr as u64);
        println!("[KERNEL32] OutputDebugStringA(\"{s}\")");
        Some((1, None))
    }

    // =========================================================
    // String
    // =========================================================
    pub fn lstrlen_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let addr = uc.read_arg(0);
        let s = uc.read_string(addr as u64);
        let len = s.len() as i32;
        println!("[KERNEL32] lstrlenA(\"{}\") -> {}", s, len);
        Some((1, Some(len)))
    }

    pub fn lstrcpy_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let dst = uc.read_arg(0);
        let src = uc.read_arg(1);
        let s = uc.read_string(src as u64);
        let mut bytes = s.as_bytes().to_vec();
        bytes.push(0);
        uc.mem_write(dst as u64, &bytes).unwrap();
        println!("[KERNEL32] lstrcpyA({:#x}, \"{}\")", dst, s);
        Some((2, Some(dst as i32)))
    }

    pub fn lstrcpyn_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let dst = uc.read_arg(0);
        let src = uc.read_arg(1);
        let max_count = uc.read_arg(2) as usize;
        let s = uc.read_string(src as u64);
        let copy_len = s.len().min(max_count.saturating_sub(1));
        let mut bytes = s.as_bytes()[..copy_len].to_vec();
        bytes.push(0);
        uc.mem_write(dst as u64, &bytes).unwrap();
        println!("[KERNEL32] lstrcpynA({:#x}, \"{}\", {})", dst, s, max_count);
        Some((3, Some(dst as i32)))
    }

    pub fn lstrcat_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let dst = uc.read_arg(0);
        let src = uc.read_arg(1);
        let dst_str = uc.read_string(dst as u64);
        let src_str = uc.read_string(src as u64);
        let mut bytes = src_str.as_bytes().to_vec();
        bytes.push(0);
        uc.mem_write(dst as u64 + dst_str.len() as u64, &bytes)
            .unwrap();
        println!("[KERNEL32] lstrcatA(\"{}\", \"{}\")", dst_str, src_str);
        Some((2, Some(dst as i32)))
    }

    pub fn lstrcmp_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let s1_addr = uc.read_arg(0);
        let s2_addr = uc.read_arg(1);
        let s1 = uc.read_string(s1_addr as u64);
        let s2 = uc.read_string(s2_addr as u64);
        let result = s1.cmp(&s2) as i32;
        println!("[KERNEL32] lstrcmpA(\"{}\", \"{}\") -> {}", s1, s2, result);
        Some((2, Some(result)))
    }

    // =========================================================
    // Module
    // =========================================================
    pub fn get_module_handle_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let name_addr = uc.read_arg(0);
        if name_addr == 0 {
            // NULL = 현재 실행 모듈 (4Leaf.dll의 베이스)
            println!("[KERNEL32] GetModuleHandleA(NULL) -> 0x35000000");
            Some((1, Some(0x3500_0000u32 as i32)))
        } else {
            let name = uc.read_string(name_addr as u64);
            println!("[KERNEL32] GetModuleHandleA(\"{}\") -> 0", name);
            // 로드된 DLL에서 찾기
            let ctx = uc.get_data_mut();
            let mut found_base: u32 = 0;
            for (dll_name, dll) in ctx.dll_modules.borrow().iter() {
                if dll_name.eq_ignore_ascii_case(&name) || dll.name.ends_with(&name) {
                    found_base = dll.base_addr as u32;
                    break;
                }
            }
            Some((1, Some(found_base as i32)))
        }
    }

    pub fn get_module_file_name_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let _module = uc.read_arg(0);
        let buf_addr = uc.read_arg(1);
        let buf_size = uc.read_arg(2);
        let path = "C:\\4Leaf\\4Leaf.exe\0";
        let bytes = path.as_bytes();
        let copy_len = bytes.len().min(buf_size as usize);
        uc.mem_write(buf_addr as u64, &bytes[..copy_len]).unwrap();
        println!(
            "[KERNEL32] GetModuleFileNameA(...) -> \"{}\"",
            &path[..path.len() - 1]
        );
        Some((3, Some((copy_len - 1) as i32)))
    }

    pub fn load_library_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let name_addr = uc.read_arg(0);
        let name = uc.read_string(name_addr as u64);
        println!("[KERNEL32] LoadLibraryA(\"{}\") -> 0", name);
        // 이미 로드된 DLL이면 핸들 반환
        let ctx = uc.get_data_mut();
        let mut found_base: u32 = 0;
        for (dll_name, dll) in ctx.dll_modules.borrow().iter() {
            if dll_name.eq_ignore_ascii_case(&name) {
                found_base = dll.base_addr as u32;
                break;
            }
        }
        Some((1, Some(found_base as i32)))
    }

    pub fn free_library(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[KERNEL32] FreeLibrary(...)");
        Some((1, Some(1)))
    }

    pub fn get_proc_address(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let _module = uc.read_arg(0);
        let name_addr = uc.read_arg(1);
        let name = uc.read_string(name_addr as u64);
        println!("[KERNEL32] GetProcAddress(..., \"{}\") -> 0", name);
        Some((2, Some(0)))
    }

    // =========================================================
    // Math / Time
    // =========================================================
    pub fn mul_div(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let a = uc.read_arg(0) as i32;
        let b = uc.read_arg(1) as i32;
        let c = uc.read_arg(2) as i32;
        let result = if c == 0 {
            -1
        } else {
            ((a as i64 * b as i64) / c as i64) as i32
        };
        println!("[KERNEL32] MulDiv({}, {}, {}) -> {}", a, b, c, result);
        Some((3, Some(result)))
    }

    pub fn get_tick_count(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let elapsed = uc.get_data_mut().start_time.elapsed().as_millis() as u32;
        println!("[KERNEL32] GetTickCount() -> {}", elapsed);
        Some((0, Some(elapsed as i32)))
    }

    pub fn get_local_time(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let buf_addr = uc.read_arg(0);
        // SYSTEMTIME: 8 WORDs = 16 bytes, 0으로 채움
        let zeros = [0u8; 16];
        uc.mem_write(buf_addr as u64, &zeros).unwrap();
        println!("[KERNEL32] GetLocalTime(...)");
        Some((1, None))
    }

    pub fn system_time_to_file_time(
        _uc: &mut Unicorn<Win32Context>,
    ) -> Option<(usize, Option<i32>)> {
        println!("[KERNEL32] SystemTimeToFileTime(...)");
        Some((2, Some(1)))
    }

    // =========================================================
    // Interlocked
    // =========================================================
    pub fn interlocked_exchange(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let target_addr = uc.read_arg(0);
        let new_value = uc.read_arg(1);
        let old_value = uc.read_u32(target_addr as u64);
        uc.write_u32(target_addr as u64, new_value);
        println!(
            "[KERNEL32] InterlockedExchange({:#x}, {}) -> {}",
            target_addr, new_value, old_value
        );
        Some((2, Some(old_value as i32)))
    }

    // =========================================================
    // Memory
    // =========================================================
    pub fn global_alloc(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let _flags = uc.read_arg(0);
        let size = uc.read_arg(1);
        let addr = uc.malloc(size as usize);
        // GMEM_ZEROINIT (0x0040) 일 수 있으므로 0으로 초기화
        let zeros = vec![0u8; size as usize];
        uc.mem_write(addr, &zeros).unwrap();
        println!(
            "[KERNEL32] GlobalAlloc({}, {}) -> {:#x}",
            _flags, size, addr
        );
        Some((2, Some(addr as i32)))
    }

    pub fn global_lock(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let handle = uc.read_arg(0);
        // 핸들 = 메모리 포인터로 취급
        println!("[KERNEL32] GlobalLock({:#x}) -> {:#x}", handle, handle);
        Some((1, Some(handle as i32)))
    }

    pub fn global_unlock(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[KERNEL32] GlobalUnlock(...)");
        Some((1, Some(1)))
    }

    pub fn global_free(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[KERNEL32] GlobalFree(...)");
        Some((1, Some(0))) // 성공 시 NULL
    }

    // =========================================================
    // File System
    // =========================================================
    pub fn create_file_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let name_addr = uc.read_arg(0);
        let name = uc.read_string(name_addr as u64);
        let ctx = uc.get_data_mut();
        let handle = ctx.alloc_handle();
        println!("[KERNEL32] CreateFileA(\"{}\") -> {:#x}", name, handle);
        Some((7, Some(handle as i32)))
    }

    pub fn find_first_file_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let name_addr = uc.read_arg(0);
        let name = uc.read_string(name_addr as u64);
        println!(
            "[KERNEL32] FindFirstFileA(\"{}\") -> INVALID_HANDLE_VALUE",
            name
        );
        Some((2, Some(-1i32))) // INVALID_HANDLE_VALUE
    }

    pub fn find_next_file_a(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[KERNEL32] FindNextFileA(...) -> FALSE");
        Some((2, Some(0)))
    }

    pub fn find_close(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[KERNEL32] FindClose(...)");
        Some((1, Some(1)))
    }

    pub fn get_file_attributes_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let name_addr = uc.read_arg(0);
        let name = uc.read_string(name_addr as u64);
        println!("[KERNEL32] GetFileAttributesA(\"{}\") -> INVALID", name);
        Some((1, Some(-1i32))) // INVALID_FILE_ATTRIBUTES
    }

    pub fn set_file_attributes_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let name_addr = uc.read_arg(0);
        let name = uc.read_string(name_addr as u64);
        println!("[KERNEL32] SetFileAttributesA(\"{}\")", name);
        Some((2, Some(1)))
    }

    pub fn remove_directory_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let name_addr = uc.read_arg(0);
        let name = uc.read_string(name_addr as u64);
        println!("[KERNEL32] RemoveDirectoryA(\"{}\")", name);
        Some((1, Some(1)))
    }

    pub fn create_directory_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let name_addr = uc.read_arg(0);
        let name = uc.read_string(name_addr as u64);
        println!("[KERNEL32] CreateDirectoryA(\"{}\")", name);
        Some((2, Some(1)))
    }

    pub fn delete_file_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let name_addr = uc.read_arg(0);
        let name = uc.read_string(name_addr as u64);
        println!("[KERNEL32] DeleteFileA(\"{}\")", name);
        Some((1, Some(1)))
    }

    pub fn copy_file_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let src_addr = uc.read_arg(0);
        let src = uc.read_string(src_addr as u64);
        println!("[KERNEL32] CopyFileA(\"{}\")", src);
        Some((3, Some(1)))
    }

    pub fn get_temp_path_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let _buf_size = uc.read_arg(0);
        let buf_addr = uc.read_arg(1);
        let path = "C:\\Temp\\\0";
        uc.mem_write(buf_addr as u64, path.as_bytes()).unwrap();
        println!("[KERNEL32] GetTempPathA(...) -> \"C:\\\\Temp\\\\\"");
        Some((2, Some((path.len() - 1) as i32)))
    }

    pub fn get_short_path_name_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let long_addr = uc.read_arg(0);
        let short_addr = uc.read_arg(1);
        let _buf_size = uc.read_arg(2);
        let long_name = uc.read_string(long_addr as u64);
        let mut bytes = long_name.as_bytes().to_vec();
        bytes.push(0);
        if short_addr != 0 {
            uc.mem_write(short_addr as u64, &bytes).unwrap();
        }
        println!("[KERNEL32] GetShortPathNameA(\"{}\")", long_name);
        Some((3, Some((bytes.len() - 1) as i32)))
    }

    pub fn get_full_path_name_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let name_addr = uc.read_arg(0);
        let _buf_size = uc.read_arg(1);
        let buf_addr = uc.read_arg(2);
        let name = uc.read_string(name_addr as u64);
        let full = format!("C:\\4Leaf\\{}\0", name);
        uc.mem_write(buf_addr as u64, full.as_bytes()).unwrap();
        println!("[KERNEL32] GetFullPathNameA(\"{}\")", name);
        Some((4, Some((full.len() - 1) as i32)))
    }

    pub fn get_long_path_name_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let short_addr = uc.read_arg(0);
        let long_addr = uc.read_arg(1);
        let _buf_size = uc.read_arg(2);
        let short_name = uc.read_string(short_addr as u64);
        let mut bytes = short_name.as_bytes().to_vec();
        bytes.push(0);
        if long_addr != 0 {
            uc.mem_write(long_addr as u64, &bytes).unwrap();
        }
        println!("[KERNEL32] GetLongPathNameA(\"{}\")", short_name);
        Some((3, Some((bytes.len() - 1) as i32)))
    }

    pub fn set_file_time(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        println!("[KERNEL32] SetFileTime(...)");
        Some((4, Some(1)))
    }

    // =========================================================
    // Handle function dispatch
    // =========================================================
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        callee_result(match func_name {
            "TlsAlloc" => DllKERNEL32::tls_alloc(uc),
            "TlsFree" => DllKERNEL32::tls_free(uc),
            "TlsGetValue" => DllKERNEL32::tls_get_value(uc),
            "TlsSetValue" => DllKERNEL32::tls_set_value(uc),
            "Sleep" => DllKERNEL32::sleep(uc),
            "GetCurrentThreadId" => DllKERNEL32::get_current_thread_id(uc),
            "WaitForSingleObject" => DllKERNEL32::wait_for_single_object(uc),
            "TerminateThread" => DllKERNEL32::terminate_thread(uc),
            "CloseHandle" => DllKERNEL32::close_handle(uc),
            "DuplicateHandle" => DllKERNEL32::duplicate_handle(uc),
            "GetCurrentThread" => DllKERNEL32::get_current_thread(uc),
            "GetCurrentProcess" => DllKERNEL32::get_current_process(uc),
            "FormatMessageA" => DllKERNEL32::format_message_a(uc),
            "GetLastError" => DllKERNEL32::get_last_error(uc),
            "CreateEventA" => DllKERNEL32::create_event_a(uc),
            "SetEvent" => DllKERNEL32::set_event(uc),
            "PulseEvent" => DllKERNEL32::pulse_event(uc),
            "ResetEvent" => DllKERNEL32::reset_event(uc),
            "InitializeCriticalSection" => DllKERNEL32::initialize_critical_section(uc),
            "DeleteCriticalSection" => DllKERNEL32::delete_critical_section(uc),
            "EnterCriticalSection" => DllKERNEL32::enter_critical_section(uc),
            "LeaveCriticalSection" => DllKERNEL32::leave_critical_section(uc),
            "OutputDebugStringA" => DllKERNEL32::output_debug_string_a(uc),
            "DisableThreadLibraryCalls" => DllKERNEL32::disable_thread_library_calls(uc),
            "lstrlenA" => DllKERNEL32::lstrlen_a(uc),
            "MulDiv" => DllKERNEL32::mul_div(uc),
            "lstrcpynA" => DllKERNEL32::lstrcpyn_a(uc),
            "SetLastError" => DllKERNEL32::set_last_error(uc),
            "GetModuleHandleA" => DllKERNEL32::get_module_handle_a(uc),
            "InterlockedExchange" => DllKERNEL32::interlocked_exchange(uc),
            "GetTickCount" => DllKERNEL32::get_tick_count(uc),
            "lstrcpyA" => DllKERNEL32::lstrcpy_a(uc),
            "lstrcatA" => DllKERNEL32::lstrcat_a(uc),
            "GlobalAlloc" => DllKERNEL32::global_alloc(uc),
            "GlobalLock" => DllKERNEL32::global_lock(uc),
            "GlobalUnlock" => DllKERNEL32::global_unlock(uc),
            "GlobalFree" => DllKERNEL32::global_free(uc),
            "SetThreadPriority" => DllKERNEL32::set_thread_priority(uc),
            "FreeLibrary" => DllKERNEL32::free_library(uc),
            "FindNextFileA" => DllKERNEL32::find_next_file_a(uc),
            "FindClose" => DllKERNEL32::find_close(uc),
            "GetFileAttributesA" => DllKERNEL32::get_file_attributes_a(uc),
            "RemoveDirectoryA" => DllKERNEL32::remove_directory_a(uc),
            "GetTempPathA" => DllKERNEL32::get_temp_path_a(uc),
            "SystemTimeToFileTime" => DllKERNEL32::system_time_to_file_time(uc),
            "WaitForMultipleObjects" => DllKERNEL32::wait_for_multiple_objects(uc),
            "GetShortPathNameA" => DllKERNEL32::get_short_path_name_a(uc),
            "lstrcmpA" => DllKERNEL32::lstrcmp_a(uc),
            "GetLocalTime" => DllKERNEL32::get_local_time(uc),
            "CreateDirectoryA" => DllKERNEL32::create_directory_a(uc),
            "DeleteFileA" => DllKERNEL32::delete_file_a(uc),
            "CopyFileA" => DllKERNEL32::copy_file_a(uc),
            "ReleaseMutex" => DllKERNEL32::release_mutex(uc),
            "CreateProcessA" => DllKERNEL32::create_process_a(uc),
            "CreateMutexA" => DllKERNEL32::create_mutex_a(uc),
            "FindFirstFileA" => DllKERNEL32::find_first_file_a(uc),
            "GetFullPathNameA" => DllKERNEL32::get_full_path_name_a(uc),
            "GetModuleFileNameA" => DllKERNEL32::get_module_file_name_a(uc),
            "GetLongPathNameA" => DllKERNEL32::get_long_path_name_a(uc),
            "SetFileTime" => DllKERNEL32::set_file_time(uc),
            "CreateFileA" => DllKERNEL32::create_file_a(uc),
            "GetProcAddress" => DllKERNEL32::get_proc_address(uc),
            "LoadLibraryA" => DllKERNEL32::load_library_a(uc),
            "SetFileAttributesA" => DllKERNEL32::set_file_attributes_a(uc),
            _ => {
                println!("[KERNEL32] UNHANDLED: {}", func_name);
                None
            }
        })
    }
}
