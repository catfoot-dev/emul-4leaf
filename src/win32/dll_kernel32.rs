use std::{thread, time};

use unicorn_engine::Unicorn;

use std::sync::atomic::Ordering;
use crate::helper::UnicornHelper;
use crate::win32::{ApiHookResult, EventState, Win32Context, callee_result};

/// `KERNEL32.dll` 프록시 구현 모듈
///
/// Windows 코어 서브시스템으로, 스레드, 메모리, 모듈 핸들, 뮤텍스(Mutex), 이벤트(Event) 등을 가상으로 프로비저닝
pub struct DllKERNEL32 {}

impl DllKERNEL32 {
    // =========================================================
    // TLS (Thread Local Storage)
    // =========================================================
    // API: DWORD TlsAlloc(void)
    // 역할: 새 TLS(Thread Local Storage) 인덱스를 할당
    pub fn tls_alloc(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let ctx = uc.get_data();
        let index = ctx.tls_counter.fetch_add(1, Ordering::SeqCst);
        ctx.tls_slots.lock().unwrap().insert(index, 0);
        crate::emu_log!("[KERNEL32] TlsAlloc() -> {}", index);
        Some((0, Some(index as i32)))
    }

    // API: BOOL TlsFree(DWORD dwTlsIndex)
    // 역할: 지정된 TLS 인덱스를 해제
    pub fn tls_free(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let index = uc.read_arg(0);
        let ctx = uc.get_data();
        ctx.tls_slots.lock().unwrap().remove(&index);
        crate::emu_log!("[KERNEL32] TlsFree({})", index);
        Some((1, Some(1))) // TRUE
    }

    // API: LPVOID TlsGetValue(DWORD dwTlsIndex)
    // 역할: 현재 스레드의 TLS 슬롯에 저장된 값을 검색
    pub fn tls_get_value(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let index = uc.read_arg(0);
        let ctx = uc.get_data();
        let slots = ctx.tls_slots.lock().unwrap();
        let value = *slots.get(&index).unwrap_or(&0);
        crate::emu_log!("[KERNEL32] TlsGetValue({}) -> {:#x}", index, value);
        Some((1, Some(value as i32)))
    }

    // API: BOOL TlsSetValue(DWORD dwTlsIndex, LPVOID lpTlsValue)
    // 역할: 현재 스레드의 TLS 슬롯에 값을 저장
    pub fn tls_set_value(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let index = uc.read_arg(0);
        let value = uc.read_arg(1);
        let ctx = uc.get_data();
        ctx.tls_slots.lock().unwrap().insert(index, value);
        crate::emu_log!("[KERNEL32] TlsSetValue({}, {:#x})", index, value);
        Some((2, Some(1))) // TRUE
    }

    // =========================================================
    // Thread / Process
    // =========================================================
    // API: VOID Sleep(DWORD dwMilliseconds)
    // 역할: 지정된 밀리초 동안 스레드의 실행을 일시 중단
    pub fn sleep(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        // sleep은 단일 스레드 에뮬이므로 no-op
        // crate::emu_log!("[KERNEL32] Sleep(...)");
        let dw_milliseconds = uc.read_arg(0);
        thread::sleep(time::Duration::from_millis(dw_milliseconds as u64));
        Some((1, None))
    }

    // API: DWORD GetCurrentThreadId(void)
    // 역할: 호출하는 스레드의 스레드 식별자를 검색
    pub fn get_current_thread_id(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[KERNEL32] GetCurrentThreadId() -> 1");
        Some((0, Some(1)))
    }

    // API: HANDLE GetCurrentThread(void)
    // 역할: 현재 스레드의 스레드 핸들을 반환
    pub fn get_current_thread(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[KERNEL32] GetCurrentThread() -> 0xFFFFFFFE");
        Some((0, Some(-2i32))) // pseudo handle
    }

    // API: HANDLE GetCurrentProcess(void)
    // 역할: 현재 프로세스의 의사 핸들(pseudo handle)을 반환
    pub fn get_current_process(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[KERNEL32] GetCurrentProcess() -> 0xFFFFFFFF");
        Some((0, Some(-1i32))) // pseudo handle
    }

    // API: DWORD WaitForSingleObject(HANDLE hHandle, DWORD dwMilliseconds)
    // 역할: 저정된 객체가 신호 상태가 되거나 시간제한이 초과될 때까지 대기
    pub fn wait_for_single_object(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[KERNEL32] WaitForSingleObject(...)");
        Some((2, Some(0))) // WAIT_OBJECT_0
    }

    // API: DWORD WaitForMultipleObjects(DWORD nCount, const HANDLE *lpHandles, BOOL bWaitAll, DWORD dwMilliseconds)
    // 역할: 하나 또는 모든 지정된 개체가 신호 상태가 될 때까지 대기
    pub fn wait_for_multiple_objects(
        _uc: &mut Unicorn<Win32Context>,
    ) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[KERNEL32] WaitForMultipleObjects(...)");
        Some((4, Some(0))) // WAIT_OBJECT_0
    }

    // API: BOOL TerminateThread(HANDLE hThread, DWORD dwExitCode)
    // 역할: 스레드를 강제 종료
    pub fn terminate_thread(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[KERNEL32] TerminateThread(...)");
        Some((2, Some(1)))
    }

    // API: BOOL SetThreadPriority(HANDLE hThread, int nPriority)
    // 역할: 지정된 스레드의 우선 순위 값을 설정
    pub fn set_thread_priority(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[KERNEL32] SetThreadPriority(...)");
        Some((2, Some(1)))
    }

    // API: BOOL DisableThreadLibraryCalls(HMODULE hLibModule)
    // 역할: DLL의 스레드 부착/분리 알림을 비활성화
    pub fn disable_thread_library_calls(
        _uc: &mut Unicorn<Win32Context>,
    ) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[KERNEL32] DisableThreadLibraryCalls(...)");
        Some((1, Some(1))) // TRUE
    }

    // API: BOOL CreateProcessA(LPCSTR lpApplicationName, LPSTR lpCommandLine, ...)
    // 역할: 새로운 프로세스와 그 기본 스레드를 생성
    pub fn create_process_a(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[KERNEL32] CreateProcessA(...)");
        Some((10, Some(0))) // FALSE
    }

    // =========================================================
    // Handle
    // =========================================================
    // API: BOOL CloseHandle(HANDLE hObject)
    // 역할: 열려있는 개체 핸들을 닫음
    pub fn close_handle(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[KERNEL32] CloseHandle(...)");
        Some((1, Some(1))) // TRUE
    }

    /// API: BOOL DuplicateHandle(HANDLE hSourceProcessHandle, HANDLE hSourceHandle, ...)
    /// 역할: 객체 핸들을 복제합니다.
    pub fn duplicate_handle(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[KERNEL32] DuplicateHandle(...)");
        Some((7, Some(1))) // TRUE
    }

    // =========================================================
    // Error
    // =========================================================
    // API: DWORD GetLastError(void)
    // 역할: 호출하는 스레드의 가장 최근 오류 코드를 검색
    pub fn get_last_error(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let err = uc.get_data().last_error.load(Ordering::SeqCst);
        crate::emu_log!("[KERNEL32] GetLastError() -> {}", err);
        Some((0, Some(err as i32)))
    }

    // API: void SetLastError(DWORD dwErrCode)
    // 역할: 호출 스레드의 가장 최근 오류 코드를 설정
    pub fn set_last_error(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let code = uc.read_arg(0);
        uc.get_data().last_error.store(code, Ordering::SeqCst);
        crate::emu_log!("[KERNEL32] SetLastError({})", code);
        Some((1, None))
    }

    // API: DWORD FormatMessageA(DWORD dwFlags, LPCVOID lpSource, ...)
    // 역할: 메시지 문자열을 포맷
    pub fn format_message_a(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[KERNEL32] FormatMessageA(...)");
        Some((7, Some(0)))
    }

    // =========================================================
    // Event / Sync
    // =========================================================
    // API: HANDLE CreateEventA(LPSECURITY_ATTRIBUTES lpEventAttributes, BOOL bManualReset, BOOL bInitialState, LPCSTR lpName)
    // 역할: 이벤트 개체를 생성하거나 오픈
    pub fn create_event_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let manual_reset = uc.read_arg(1);
        let initial_state = uc.read_arg(2);
        let ctx = uc.get_data();
        let handle = ctx.alloc_handle();
        ctx.events.lock().unwrap().insert(
            handle,
            EventState {
                signaled: initial_state != 0,
                manual_reset: manual_reset != 0,
            },
        );
        crate::emu_log!("[KERNEL32] CreateEventA(...) -> handle {:#x}", handle);
        Some((4, Some(handle as i32)))
    }

    // API: BOOL SetEvent(HANDLE hEvent)
    // 역할: 지정된 이벤트 개체를 신호(signaled) 상태로 설정
    pub fn set_event(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let handle = uc.read_arg(0);
        let ctx = uc.get_data();
        let mut events = ctx.events.lock().unwrap();
        if let Some(evt) = events.get_mut(&handle) {
            evt.signaled = true;
        }
        crate::emu_log!("[KERNEL32] SetEvent({:#x})", handle);
        Some((1, Some(1)))
    }

    // API: BOOL PulseEvent(HANDLE hEvent)
    // 역할: 이벤트 개체의 상태를 signaled로 설정한 후 다시 nonsignaled 상태로 재설정
    pub fn pulse_event(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let handle = uc.read_arg(0);
        crate::emu_log!("[KERNEL32] PulseEvent({:#x})", handle);
        Some((1, Some(1)))
    }

    // API: BOOL ResetEvent(HANDLE hEvent)
    // 역할: 지정된 이벤트 개체를 비신호(nonsignaled) 상태로 설정
    pub fn reset_event(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let handle = uc.read_arg(0);
        let ctx = uc.get_data();
        let mut events = ctx.events.lock().unwrap();
        if let Some(evt) = events.get_mut(&handle) {
            evt.signaled = false;
        }
        crate::emu_log!("[KERNEL32] ResetEvent({:#x})", handle);
        Some((1, Some(1)))
    }

    // Critical Section (싱글 스레드이므로 no-op)
    // API: void InitializeCriticalSection(LPCRITICAL_SECTION lpCriticalSection)
    // 역할: 크리티컬 섹션 개체를 초기화
    pub fn initialize_critical_section(
        _uc: &mut Unicorn<Win32Context>,
    ) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[KERNEL32] InitializeCriticalSection(...)");
        Some((1, None))
    }

    // API: void DeleteCriticalSection(LPCRITICAL_SECTION lpCriticalSection)
    // 역할: 가상 크리티컬 섹션 객체를 삭제
    pub fn delete_critical_section(
        _uc: &mut Unicorn<Win32Context>,
    ) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[KERNEL32] DeleteCriticalSection(...)");
        Some((1, None))
    }

    // API: void EnterCriticalSection(LPCRITICAL_SECTION lpCriticalSection)
    // 역할: 크리티컬 섹션에 진입
    pub fn enter_critical_section(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[KERNEL32] EnterCriticalSection(...)");
        Some((1, None))
    }

    // API: void LeaveCriticalSection(LPCRITICAL_SECTION lpCriticalSection)
    // 역할: 크리티컬 섹션을 떠남
    pub fn leave_critical_section(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[KERNEL32] LeaveCriticalSection(...)");
        Some((1, None))
    }

    // Mutex
    // API: HANDLE CreateMutexA(LPSECURITY_ATTRIBUTES lpMutexAttributes, BOOL bInitialOwner, LPCSTR lpName)
    // 역할: 뮤텍스 개체를 생성하거나 오픈
    pub fn create_mutex_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let ctx = uc.get_data();
        let handle = ctx.alloc_handle();
        crate::emu_log!("[KERNEL32] CreateMutexA(...) -> {:#x}", handle);
        Some((3, Some(handle as i32)))
    }

    // API: BOOL ReleaseMutex(HANDLE hMutex)
    // 역할: 단일 뮤텍스 객체의 소유권을 해제
    pub fn release_mutex(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[KERNEL32] ReleaseMutex(...)");
        Some((1, Some(1)))
    }

    // =========================================================
    // Debug
    // =========================================================
    // API: void OutputDebugStringA(LPCSTR lpOutputString)
    // 역할: 문자열을 디버거로 보내 화면에 출력
    pub fn output_debug_string_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let addr = uc.read_arg(0);
        let s = uc.read_string(addr as u64);
        crate::emu_log!("[KERNEL32] OutputDebugStringA(\"{s}\")");
        Some((1, None))
    }

    // =========================================================
    // String
    // =========================================================
    // API: int lstrlenA(LPCSTR lpString)
    // 역할: 지정된 문자열의 길이를 바이트 단위로 반환
    pub fn lstrlen_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let addr = uc.read_arg(0);
        let s = uc.read_string(addr as u64);
        let len = s.len() as i32;
        crate::emu_log!("[KERNEL32] lstrlenA(\"{}\") -> {}", s, len);
        Some((1, Some(len)))
    }

    // API: LPSTR lstrcpyA(LPSTR lpString1, LPCSTR lpString2)
    // 역할: 문자열을 한 버퍼에서 다른 버퍼로 복사
    pub fn lstrcpy_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let dst = uc.read_arg(0);
        let src = uc.read_arg(1);
        let s = uc.read_string(src as u64);
        let mut bytes = s.as_bytes().to_vec();
        bytes.push(0);
        uc.mem_write(dst as u64, &bytes).unwrap();
        crate::emu_log!("[KERNEL32] lstrcpyA({:#x}, \"{}\")", dst, s);
        Some((2, Some(dst as i32)))
    }

    // API: LPSTR lstrcpynA(LPSTR lpString1, LPCSTR lpString2, int iMaxLength)
    // 역할: 지정된 수의 문자를 복사
    pub fn lstrcpyn_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let dst = uc.read_arg(0);
        let src = uc.read_arg(1);
        let max_count = uc.read_arg(2) as usize;
        let s = uc.read_string(src as u64);
        let copy_len = s.len().min(max_count.saturating_sub(1));
        let mut bytes = s.as_bytes()[..copy_len].to_vec();
        bytes.push(0);
        uc.mem_write(dst as u64, &bytes).unwrap();
        crate::emu_log!("[KERNEL32] lstrcpynA({:#x}, \"{}\", {})", dst, s, max_count);
        Some((3, Some(dst as i32)))
    }

    // API: LPSTR lstrcatA(LPSTR lpString1, LPCSTR lpString2)
    // 역할: 한 문자열을 다른 문자열에 추가
    pub fn lstrcat_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let dst = uc.read_arg(0);
        let src = uc.read_arg(1);
        let dst_str = uc.read_string(dst as u64);
        let src_str = uc.read_string(src as u64);
        let mut bytes = src_str.as_bytes().to_vec();
        bytes.push(0);
        uc.mem_write(dst as u64 + dst_str.len() as u64, &bytes)
            .unwrap();
        crate::emu_log!("[KERNEL32] lstrcatA(\"{}\", \"{}\")", dst_str, src_str);
        Some((2, Some(dst as i32)))
    }

    // API: int lstrcmpA(LPCSTR lpString1, LPCSTR lpString2)
    // 역할: 두 개의 문자열을 비교
    pub fn lstrcmp_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let s1_addr = uc.read_arg(0);
        let s2_addr = uc.read_arg(1);
        let s1 = uc.read_string(s1_addr as u64);
        let s2 = uc.read_string(s2_addr as u64);
        let result = s1.cmp(&s2) as i32;
        crate::emu_log!("[KERNEL32] lstrcmpA(\"{}\", \"{}\") -> {}", s1, s2, result);
        Some((2, Some(result)))
    }

    // =========================================================
    // Module
    // =========================================================
    // API: HMODULE GetModuleHandleA(LPCSTR lpModuleName)
    // 역할: 호출하는 프로세스에 이미 로드된 모듈 핸들을 검색
    pub fn get_module_handle_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let name_addr = uc.read_arg(0);
        if name_addr == 0 {
            // NULL = 현재 실행 모듈 (4Leaf.dll의 베이스)
            crate::emu_log!("[KERNEL32] GetModuleHandleA(NULL) -> 0x35000000");
            Some((1, Some(0x3500_0000u32 as i32)))
        } else {
            let name = uc.read_string(name_addr as u64);
            crate::emu_log!("[KERNEL32] GetModuleHandleA(\"{}\") -> 0", name);
            // 로드된 DLL에서 찾기
            let ctx = uc.get_data();
            let mut found_base: u32 = 0;
            let modules = ctx.dll_modules.lock().unwrap();
            for (dll_name, dll) in modules.iter() {
                if dll_name.eq_ignore_ascii_case(&name) || dll.name.ends_with(&name) {
                    found_base = dll.base_addr as u32;
                    break;
                }
            }
            Some((1, Some(found_base as i32)))
        }
    }

    // API: DWORD GetModuleFileNameA(HMODULE hModule, LPSTR lpFilename, DWORD nSize)
    // 역할: 모듈이 포함된 실행 파일의 절대 경로를 조회
    pub fn get_module_file_name_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let _module = uc.read_arg(0);
        let buf_addr = uc.read_arg(1);
        let buf_size = uc.read_arg(2);
        let path = "C:\\4Leaf\\4Leaf.exe\0";
        let bytes = path.as_bytes();
        let copy_len = bytes.len().min(buf_size as usize);
        uc.mem_write(buf_addr as u64, &bytes[..copy_len]).unwrap();
        crate::emu_log!(
            "[KERNEL32] GetModuleFileNameA(...) -> \"{}\"",
            &path[..path.len() - 1]
        );
        Some((3, Some((copy_len - 1) as i32)))
    }

    // API: HMODULE LoadLibraryA(LPCSTR lpLibFileName)
    // 역할: 지정된 모듈을 호출 컨텍스트의 주소 공간으로 매핑
    pub fn load_library_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let name_addr = uc.read_arg(0);
        let name = uc.read_string(name_addr as u64);
        crate::emu_log!("[KERNEL32] LoadLibraryA(\"{}\") -> 0", name);
        // 이미 로드된 DLL이면 핸들 반환
        let ctx = uc.get_data();
        let mut found_base: u32 = 0;
        let modules = ctx.dll_modules.lock().unwrap();
        for (dll_name, dll) in modules.iter() {
            if dll_name.eq_ignore_ascii_case(&name) {
                found_base = dll.base_addr as u32;
                break;
            }
        }
        Some((1, Some(found_base as i32)))
    }

    // API: BOOL FreeLibrary(HMODULE hLibModule)
    // 역할: 로드된 DLL 모듈을 해제
    pub fn free_library(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[KERNEL32] FreeLibrary(...)");
        Some((1, Some(1)))
    }

    // API: FARPROC GetProcAddress(HMODULE hModule, LPCSTR lpProcName)
    // 역할: DLL에서 지정된 익스포트 함수의 주소를 조회
    pub fn get_proc_address(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let _module = uc.read_arg(0);
        let name_addr = uc.read_arg(1);
        let name = uc.read_string(name_addr as u64);
        crate::emu_log!("[KERNEL32] GetProcAddress(..., \"{}\") -> 0", name);
        Some((2, Some(0)))
    }

    // =========================================================
    // Math / Time
    // =========================================================
    // API: int MulDiv(int nNumber, int nNumerator, int nDenominator)
    // 역할: 두 개의 32비트 값을 곱한 후 세 번째 32비트 값으로 나누고 결과를 32비트 값으로 돌려줌
    pub fn mul_div(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let a = uc.read_arg(0) as i32;
        let b = uc.read_arg(1) as i32;
        let c = uc.read_arg(2) as i32;
        let result = if c == 0 {
            -1
        } else {
            ((a as i64 * b as i64) / c as i64) as i32
        };
        crate::emu_log!("[KERNEL32] MulDiv({}, {}, {}) -> {}", a, b, c, result);
        Some((3, Some(result)))
    }

    // API: DWORD GetTickCount(void)
    // 역할: 시스템이 시작된 후 지난 밀리초 시간을 검색
    pub fn get_tick_count(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let elapsed = uc.get_data().start_time.elapsed().as_millis() as u32;
        crate::emu_log!("[KERNEL32] GetTickCount() -> {}", elapsed);
        Some((0, Some(elapsed as i32)))
    }

    // API: void GetLocalTime(LPSYSTEMTIME lpSystemTime)
    // 역할: 현재 로컬 날짜와 시간을 시스템 타임 구조체로 가져옴
    pub fn get_local_time(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let buf_addr = uc.read_arg(0);
        // SYSTEMTIME: 8 WORDs = 16 bytes, 0으로 채움
        let zeros = [0u8; 16];
        uc.mem_write(buf_addr as u64, &zeros).unwrap();
        crate::emu_log!("[KERNEL32] GetLocalTime(...)");
        Some((1, None))
    }

    // API: BOOL SystemTimeToFileTime(const SYSTEMTIME *lpSystemTime, LPFILETIME lpFileTime)
    // 역할: 시스템 시간을 파일 시간 형식으로 변환
    pub fn system_time_to_file_time(
        _uc: &mut Unicorn<Win32Context>,
    ) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[KERNEL32] SystemTimeToFileTime(...)");
        Some((2, Some(1)))
    }

    // =========================================================
    // Interlocked
    // =========================================================
    // API: LONG InterlockedExchange(LONG volatile *Target, LONG Value)
    // 역할: 원자적 조작을 통해 두 개의 32비트 값을 교환
    pub fn interlocked_exchange(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let target_addr = uc.read_arg(0);
        let new_value = uc.read_arg(1);
        let old_value = uc.read_u32(target_addr as u64);
        uc.write_u32(target_addr as u64, new_value);
        crate::emu_log!(
            "[KERNEL32] InterlockedExchange({:#x}, {}) -> {}",
            target_addr,
            new_value,
            old_value
        );
        Some((2, Some(old_value as i32)))
    }

    // =========================================================
    // Memory
    // =========================================================
    // API: HGLOBAL GlobalAlloc(UINT uFlags, SIZE_T dwBytes)
    // 역할: 힙에서 지정된 바이트의 메모리를 할당
    pub fn global_alloc(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let _flags = uc.read_arg(0);
        let size = uc.read_arg(1);
        let addr = uc.malloc(size as usize);
        // GMEM_ZEROINIT (0x0040) 일 수 있으므로 0으로 초기화
        let zeros = vec![0u8; size as usize];
        uc.mem_write(addr, &zeros).unwrap();
        crate::emu_log!(
            "[KERNEL32] GlobalAlloc({}, {}) -> {:#x}",
            _flags,
            size,
            addr
        );
        Some((2, Some(addr as i32)))
    }

    // API: LPVOID GlobalLock(HGLOBAL hMem)
    // 역할: 메모리를 고정하여 첫 바이트에 대한 포인터를 반환
    pub fn global_lock(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let handle = uc.read_arg(0);
        // 핸들 = 메모리 포인터로 취급
        crate::emu_log!("[KERNEL32] GlobalLock({:#x}) -> {:#x}", handle, handle);
        Some((1, Some(handle as i32)))
    }

    // API: BOOL GlobalUnlock(HGLOBAL hMem)
    // 역할: GlobalLock에 의해 잠긴 메모리를 해제
    pub fn global_unlock(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[KERNEL32] GlobalUnlock(...)");
        Some((1, Some(1)))
    }

    // API: HGLOBAL GlobalFree(HGLOBAL hMem)
    // 역할: 지정된 전역 메모리 개체를 해제
    pub fn global_free(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[KERNEL32] GlobalFree(...)");
        Some((1, Some(0))) // 성공 시 NULL
    }

    // =========================================================
    // File System
    // =========================================================
    // API: HANDLE CreateFileA(LPCSTR lpFileName, ...)
    // 역할: 파일 또는 입출력 디바이스 개체를 생성하거나 오픈
    pub fn create_file_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let name_addr = uc.read_arg(0);
        let name = uc.read_string(name_addr as u64);
        let ctx = uc.get_data();
        let handle = ctx.alloc_handle();
        crate::emu_log!("[KERNEL32] CreateFileA(\"{}\") -> {:#x}", name, handle);
        Some((7, Some(handle as i32)))
    }

    // API: HANDLE FindFirstFileA(LPCSTR lpFileName, LPWIN32_FIND_DATAA lpFindFileData)
    // 역할: 지정된 이름과 일치하는 파일용 핸들을 검색/생성
    pub fn find_first_file_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let name_addr = uc.read_arg(0);
        let name = uc.read_string(name_addr as u64);
        crate::emu_log!(
            "[KERNEL32] FindFirstFileA(\"{}\") -> INVALID_HANDLE_VALUE",
            name
        );
        Some((2, Some(-1i32))) // INVALID_HANDLE_VALUE
    }

    // API: BOOL FindNextFileA(HANDLE hFindFile, LPWIN32_FIND_DATAA lpFindFileData)
    // 역할: FindFirstFileA의 추가 파일 찾기를 실행
    pub fn find_next_file_a(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[KERNEL32] FindNextFileA(...) -> FALSE");
        Some((2, Some(0)))
    }

    // API: BOOL FindClose(HANDLE hFindFile)
    // 역할: FindFirstFileA에 의해 띄워진 파일 탐색 핸들을 닫음
    pub fn find_close(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[KERNEL32] FindClose(...)");
        Some((1, Some(1)))
    }

    // API: DWORD GetFileAttributesA(LPCSTR lpFileName)
    // 역할: 지정된 파일 또는 디렉토리의 파일 시스템 속성을 검색
    pub fn get_file_attributes_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let name_addr = uc.read_arg(0);
        let name = uc.read_string(name_addr as u64);
        crate::emu_log!("[KERNEL32] GetFileAttributesA(\"{}\") -> INVALID", name);
        Some((1, Some(-1i32))) // INVALID_FILE_ATTRIBUTES
    }

    // API: BOOL SetFileAttributesA(LPCSTR lpFileName, DWORD dwFileAttributes)
    // 역할: 지정된 파일 또는 디렉토리의 속성을 설정
    pub fn set_file_attributes_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let name_addr = uc.read_arg(0);
        let name = uc.read_string(name_addr as u64);
        crate::emu_log!("[KERNEL32] SetFileAttributesA(\"{}\")", name);
        Some((2, Some(1)))
    }

    // API: BOOL RemoveDirectoryA(LPCSTR lpPathName)
    // 역할: 기존의 빈 디렉터리를 삭제
    pub fn remove_directory_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let name_addr = uc.read_arg(0);
        let name = uc.read_string(name_addr as u64);
        crate::emu_log!("[KERNEL32] RemoveDirectoryA(\"{}\")", name);
        Some((1, Some(1)))
    }

    // API: BOOL CreateDirectoryA(LPCSTR lpPathName, LPSECURITY_ATTRIBUTES lpSecurityAttributes)
    // 역할: 새 디렉토리를 생성
    pub fn create_directory_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let name_addr = uc.read_arg(0);
        let name = uc.read_string(name_addr as u64);
        crate::emu_log!("[KERNEL32] CreateDirectoryA(\"{}\")", name);
        Some((2, Some(1)))
    }

    // API: BOOL DeleteFileA(LPCSTR lpFileName)
    // 역할: 기존 파일을 삭제
    pub fn delete_file_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let name_addr = uc.read_arg(0);
        let name = uc.read_string(name_addr as u64);
        crate::emu_log!("[KERNEL32] DeleteFileA(\"{}\")", name);
        Some((1, Some(1)))
    }

    // API: BOOL CopyFileA(LPCSTR lpExistingFileName, LPCSTR lpNewFileName, BOOL bFailIfExists)
    // 역할: 기존 파일을 새 파일로 복사
    pub fn copy_file_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let src_addr = uc.read_arg(0);
        let src = uc.read_string(src_addr as u64);
        crate::emu_log!("[KERNEL32] CopyFileA(\"{}\")", src);
        Some((3, Some(1)))
    }

    // API: DWORD GetTempPathA(DWORD nBufferLength, LPSTR lpBuffer)
    // 역할: 임시 파일용 디렉토리 경로를 지정
    pub fn get_temp_path_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let _buf_size = uc.read_arg(0);
        let buf_addr = uc.read_arg(1);
        let path = "C:\\Temp\\\0";
        uc.mem_write(buf_addr as u64, path.as_bytes()).unwrap();
        crate::emu_log!("[KERNEL32] GetTempPathA(...) -> \"C:\\\\Temp\\\\\"");
        Some((2, Some((path.len() - 1) as i32)))
    }

    // API: DWORD GetShortPathNameA(LPCSTR lpszLongPath, LPSTR lpszShortPath, DWORD cchBuffer)
    // 역할: 지정된 경로의 짧은 경로(8.3 폼) 형태를 가져옴
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
        crate::emu_log!("[KERNEL32] GetShortPathNameA(\"{}\")", long_name);
        Some((3, Some((bytes.len() - 1) as i32)))
    }

    // API: DWORD GetFullPathNameA(LPCSTR lpFileName, DWORD nBufferLength, LPSTR lpBuffer, LPSTR *lpFilePart)
    // 역할: 지정된 파일의 전체 경로와 파일 이름을 구함
    pub fn get_full_path_name_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let name_addr = uc.read_arg(0);
        let _buf_size = uc.read_arg(1);
        let buf_addr = uc.read_arg(2);
        let name = uc.read_string(name_addr as u64);
        let full = format!("C:\\4Leaf\\{}\0", name);
        uc.mem_write(buf_addr as u64, full.as_bytes()).unwrap();
        crate::emu_log!("[KERNEL32] GetFullPathNameA(\"{}\")", name);
        Some((4, Some((full.len() - 1) as i32)))
    }

    // API: DWORD GetLongPathNameA(LPCSTR lpszShortPath, LPSTR lpszLongPath, DWORD cchBuffer)
    // 역할: 지정된 경로의 원래 긴 경로 형태를 가져옴
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
        crate::emu_log!("[KERNEL32] GetLongPathNameA(\"{}\")", short_name);
        Some((3, Some((bytes.len() - 1) as i32)))
    }

    // API: BOOL SetFileTime(HANDLE hFile, const FILETIME *lpCreationTime, const FILETIME *lpLastAccessTime, const FILETIME *lpLastWriteTime)
    // 역할: 지정된 파일의 날짜 및 시간 정보를 지정
    pub fn set_file_time(_uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        crate::emu_log!("[KERNEL32] SetFileTime(...)");
        Some((4, Some(1)))
    }

    // =========================================================
    // Handle function dispatch
    // =========================================================

    /// 함수명 기준 `KERNEL32.dll` API 구현체
    ///
    /// 처리를 성공했다면 스택 보정값과 리턴값을 포함한 `ApiHookResult`를 반환
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
                crate::emu_log!("[KERNEL32] UNHANDLED: {}", func_name);
                None
            }
        })
    }
}
