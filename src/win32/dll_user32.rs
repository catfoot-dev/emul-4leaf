use unicorn_engine::Unicorn;

use crate::helper::UnicornHelper;
use crate::win32::{
    ApiHookResult, Win32Context, WindowClass, WindowState, callee_result, caller_result,
};

/// `USER32.dll` 프록시 구현 모듈
///
/// 윈도우 창, 클래스 관리, 메시지 루프 가상화를 담당하여 그래픽 UI 요소가 에뮬레이터 환경에서 작동하는 것처럼 모방
pub struct DllUSER32;

impl DllUSER32 {
    fn wrap_result(func_name: &str, result: Option<(usize, Option<i32>)>) -> Option<ApiHookResult> {
        match func_name {
            "wsprintfA" => caller_result(result),
            _ => callee_result(result),
        }
    }

    /// 함수명 기준 `USER32.dll` API 구현체
    ///
    /// 처리를 성공했다면 스택 보정값과 리턴값을 포함한 `ApiHookResult`를 반환
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        DllUSER32::wrap_result(
            func_name,
            match func_name {
                // API: int MessageBoxA(HWND hWnd, LPCSTR lpText, LPCSTR lpCaption, UINT uType)
                // 역할: 메시지 박스를 화면에 표시
                "MessageBoxA" => {
                    let _hwnd = uc.read_arg(0);
                    let text_addr = uc.read_arg(1);
                    let caption_addr = uc.read_arg(2);
                    let text = uc.read_euc_kr(text_addr as u64);
                    let caption = uc.read_euc_kr(caption_addr as u64);
                    crate::emu_log!("[USER32] MessageBoxA(\"{}\", \"{}\")", caption, text);
                    Some((4, Some(1))) // IDOK
                }

                // API: ATOM RegisterClassExA(const WNDCLASSEXA* lpwcx)
                // 역할: 창 클래스를 등록
                "RegisterClassExA" => {
                    // WNDCLASSEX는 48 bytes
                    let class_addr = uc.read_arg(0);
                    let wnd_proc = uc.read_u32(class_addr as u64 + 8);
                    let class_name_ptr = uc.read_u32(class_addr as u64 + 40);
                    let class_name = uc.read_euc_kr(class_name_ptr as u64);
                    let ctx = uc.get_data();
                    let atom = ctx.alloc_handle();
                    ctx.window_classes.lock().unwrap().insert(
                        class_name.clone(),
                        WindowClass {
                            class_name: class_name.clone(),
                            wnd_proc,
                            style: 0,
                            hinstance: 0,
                        },
                    );
                    crate::emu_log!(
                        "[USER32] RegisterClassExA(\"{}\") -> atom {:#x}",
                        class_name,
                        atom
                    );
                    Some((1, Some(atom as i32)))
                }

                // API: ATOM RegisterClassA(const WNDCLASSA* lpWndClass)
                // 역할: 창 클래스를 등록
                "RegisterClassA" => {
                    let class_addr = uc.read_arg(0);
                    let wnd_proc = uc.read_u32(class_addr as u64 + 4);
                    let class_name_ptr = uc.read_u32(class_addr as u64 + 36);
                    let class_name = uc.read_euc_kr(class_name_ptr as u64);
                    let ctx = uc.get_data();
                    let atom = ctx.alloc_handle();
                    ctx.window_classes.lock().unwrap().insert(
                        class_name.clone(),
                        WindowClass {
                            class_name: class_name.clone(),
                            wnd_proc,
                            style: 0,
                            hinstance: 0,
                        },
                    );
                    crate::emu_log!(
                        "[USER32] RegisterClassA(\"{}\") -> atom {:#x}",
                        class_name,
                        atom
                    );
                    Some((1, Some(atom as i32)))
                }

                // API: HWND CreateWindowExA(DWORD dwExStyle, LPCSTR lpClassName, LPCSTR lpWindowName, DWORD dwStyle, int X, int Y, int nWidth, int nHeight, HWND hWndParent, HMENU hMenu, HINSTANCE hInstance, LPVOID lpParam)
                // 역할: 확장 스타일을 포함한 창을 생성
                "CreateWindowExA" => {
                    let _ex_style = uc.read_arg(0);
                    let class_addr = uc.read_arg(1);
                    let title_addr = uc.read_arg(2);
                    let style = uc.read_arg(3);
                    let x = uc.read_arg(4);
                    let y = uc.read_arg(5);
                    let width = uc.read_arg(6);
                    let height = uc.read_arg(7);
                    let parent = uc.read_arg(8);
                    let _menu = uc.read_arg(9);
                    let _instance = uc.read_arg(10);
                    let param = uc.read_arg(11);

                    let hwnd = uc.get_data().alloc_handle();

                    if param != 0 {
                        // MFC 등에서 사용되는 패턴: *((HWND *)lpParam + 1) = hwnd;
                        uc.write_u32(param as u64 + 4, hwnd);
                    }

                    let class_name = if class_addr < 0x10000 {
                        format!("Atom_{}", class_addr)
                    } else {
                        uc.read_euc_kr(class_addr as u64)
                    };
                    let title = if title_addr != 0 {
                        uc.read_euc_kr(title_addr as u64)
                    } else {
                        String::new()
                    };

                    let window_state = WindowState {
                        class_name: class_name.clone(),
                        title: title.clone(),
                        x: x as i32,
                        y: y as i32,
                        width: width as i32,
                        height: height as i32,
                        style,
                        parent,
                        visible: false,
                        wnd_proc: 0,
                        user_data: 0,
                    };

                    uc.get_data()
                        .win_frame
                        .lock()
                        .unwrap()
                        .create_window(hwnd, window_state);

                    crate::emu_log!(
                        "[USER32] CreateWindowExA(\"{}\", \"{}\", param={:#x}) -> HWND {:#x}",
                        class_name,
                        title,
                        param,
                        hwnd
                    );
                    Some((12, Some(hwnd as i32)))
                }

                // API: BOOL ShowWindow(HWND hWnd, int nCmdShow)
                // 역할: 창의 표시 상태를 설정
                "ShowWindow" => {
                    let hwnd = uc.read_arg(0);
                    let n_cmd_show = uc.read_arg(1);
                    // SW_HIDE = 0, 그 외는 대부분 표시
                    let visible = n_cmd_show != 0;
                    uc.get_data()
                        .win_frame
                        .lock()
                        .unwrap()
                        .show_window(hwnd, visible);
                    Some((2, Some(1)))
                }

                // API: BOOL UpdateWindow(HWND hWnd)
                // 역할: 창의 클라이언트 영역을 강제로 업데이트
                "UpdateWindow" => {
                    let hwnd = uc.read_arg(0);
                    uc.get_data().win_frame.lock().unwrap().update_window(hwnd);
                    Some((1, Some(1)))
                }

                // API: BOOL DestroyWindow(HWND hWnd)
                // 역할: 지정된 창을 파괴
                "DestroyWindow" => {
                    let hwnd = uc.read_arg(0);
                    let ctx = uc.get_data();
                    ctx.win_frame.lock().unwrap().destroy_window(hwnd);
                    Some((1, Some(1)))
                }

                // API: BOOL CloseWindow(HWND hWnd)
                // 역할: 지정된 창을 최소화
                "CloseWindow" => Some((1, Some(1))),

                // API: BOOL EnableWindow(HWND hWnd, BOOL bEnable)
                // 역할: 창의 마우스 및 키보드 입력을 활성화 또는 비활성화
                "EnableWindow" => Some((2, Some(0))),

                // API: BOOL IsWindowEnabled(HWND hWnd)
                // 역할: 창이 활성화되어 있는지 확인
                "IsWindowEnabled" => {
                    let hwnd = uc.read_arg(0);
                    let ctx = uc.get_data();
                    let win_frame = ctx.win_frame.lock().unwrap();
                    let exists = win_frame.windows.contains_key(&hwnd);
                    Some((1, Some(if exists { 1 } else { 0 })))
                }

                // API: BOOL IsWindowVisible(HWND hWnd)
                // 역할: 창의 가시성 상태를 확인
                "IsWindowVisible" => {
                    let hwnd = uc.read_arg(0);
                    let ctx = uc.get_data();
                    let win_frame = ctx.win_frame.lock().unwrap();
                    let visible = win_frame
                        .windows
                        .get(&hwnd)
                        .map(|w| w.visible)
                        .unwrap_or(false);
                    Some((1, Some(if visible { 1 } else { 0 })))
                }

                // API: BOOL MoveWindow(HWND hWnd, int X, int Y, int nWidth, int nHeight, BOOL bRepaint)
                // 역할: 창의 위치와 크기를 변경
                "MoveWindow" => {
                    let hwnd = uc.read_arg(0);
                    let x = uc.read_arg(1) as i32;
                    let y = uc.read_arg(2) as i32;
                    let width = uc.read_arg(3) as u32;
                    let height = uc.read_arg(4) as u32;

                    let ctx = uc.get_data();
                    let mut win_frame = ctx.win_frame.lock().unwrap();
                    win_frame.move_window(hwnd, x, y, width, height);
                    Some((6, Some(1)))
                }

                // API: BOOL SetWindowPos(HWND hWnd, HWND hWndInsertAfter, int X, int Y, int cx, int cy, UINT uFlags)
                // 역할: 창의 크기, 위치 및 Z 순서를 변경
                "SetWindowPos" => {
                    let hwnd = uc.read_arg(0);
                    let x = uc.read_arg(2) as i32;
                    let y = uc.read_arg(3) as i32;
                    let cx = uc.read_arg(4) as u32;
                    let cy = uc.read_arg(5) as u32;
                    let _u_flags = uc.read_arg(6);

                    let ctx = uc.get_data();
                    let mut win_frame = ctx.win_frame.lock().unwrap();
                    win_frame.move_window(hwnd, x, y, cx, cy);
                    Some((7, Some(1)))
                }

                // API: BOOL GetWindowRect(HWND hWnd, LPRECT lpRect)
                // 역할: 창의 화면 좌표상의 경계 사각형 좌표를 가져옴
                "GetWindowRect" => {
                    let hwnd = uc.read_arg(0);
                    let rect_addr = uc.read_arg(1);
                    let (x, y, w, h) = {
                        let ctx = uc.get_data();
                        let win_frame = ctx.win_frame.lock().unwrap();
                        win_frame
                            .windows
                            .get(&hwnd)
                            .map(|win| (win.x, win.y, win.width, win.height))
                            .unwrap_or((0, 0, 640, 480))
                    };

                    uc.write_u32(rect_addr as u64, x as u32);
                    uc.write_u32(rect_addr as u64 + 4, y as u32);
                    uc.write_u32(rect_addr as u64 + 8, (x + w) as u32);
                    uc.write_u32(rect_addr as u64 + 12, (y + h) as u32);
                    Some((2, Some(1)))
                }

                // API: BOOL GetClientRect(HWND hWnd, LPRECT lpRect)
                // 역할: 창의 클라이언트 영역 좌표를 가져옴
                "GetClientRect" => {
                    let hwnd = uc.read_arg(0);
                    let rect_addr = uc.read_arg(1);
                    let (w, h) = {
                        let ctx = uc.get_data();
                        let win_frame = ctx.win_frame.lock().unwrap();
                        win_frame
                            .windows
                            .get(&hwnd)
                            .map(|win| (win.width, win.height))
                            .unwrap_or((640, 480))
                    };

                    uc.write_u32(rect_addr as u64, 0);
                    uc.write_u32(rect_addr as u64 + 4, 0);
                    uc.write_u32(rect_addr as u64 + 8, w as u32);
                    uc.write_u32(rect_addr as u64 + 12, h as u32);
                    Some((2, Some(1)))
                }

                // API: BOOL AdjustWindowRectEx(LPRECT lpRect, DWORD dwStyle, BOOL bMenu, DWORD dwExStyle)
                // 역할: 클라이언트 영역의 크기를 기준으로 원하는 창의 크기를 계산
                "AdjustWindowRectEx" => Some((4, Some(1))),

                // API: HDC GetDC(HWND hWnd)
                // 역할: 지정된 창의 클라이언트 영역에 대한 DC를 가져옴
                "GetDC" => {
                    let ctx = uc.get_data();
                    let hdc = ctx.alloc_handle();
                    crate::emu_log!("[USER32] GetDC(...) -> HDC {:#x}", hdc);
                    Some((1, Some(hdc as i32)))
                }

                // API: HDC GetWindowDC(HWND hWnd)
                // 역할: 지정된 창 전체(비클라이언트 영역 포함)에 대한 DC를 가져옴
                "GetWindowDC" => {
                    let ctx = uc.get_data();
                    let hdc = ctx.alloc_handle();
                    Some((1, Some(hdc as i32)))
                }

                // API: int ReleaseDC(HWND hWnd, HDC hDC)
                // 역할: DC를 해제
                "ReleaseDC" => Some((2, Some(1))),

                // API: LRESULT SendMessageA(HWND hWnd, UINT Msg, WPARAM wParam, LPARAM lParam)
                // 역할: 지정된 창에 메시지를 전송하고 처리가 완료될 때까지 대기
                "SendMessageA" => Some((4, Some(0))),

                // API: BOOL PostMessageA(HWND hWnd, UINT Msg, WPARAM wParam, LPARAM lParam)
                // 역할: 지정된 창의 메시지 큐에 메시지를 배치
                "PostMessageA" => Some((4, Some(1))),

                // API: HCURSOR LoadCursorA(HINSTANCE hInstance, LPCSTR lpCursorName)
                // 역할: 커서 리소스를 로드
                "LoadCursorA" => Some((2, Some(0x1001))),

                // API: HCURSOR LoadCursorFromFileA(LPCSTR lpFileName)
                // 역할: 파일에서 커서를 로드
                "LoadCursorFromFileA" => Some((1, Some(0x1002))),

                // API: HICON LoadIconA(HINSTANCE hInstance, LPCSTR lpIconName)
                // 역할: 아이콘 리소스를 로드
                "LoadIconA" => Some((2, Some(0x1003))),

                // API: HCURSOR SetCursor(HCURSOR hCursor)
                // 역할: 마우스 커서를 설정
                "SetCursor" => Some((1, Some(0))),

                // API: BOOL DestroyCursor(HCURSOR hCursor)
                // 역할: 커서를 파괴하고 사용된 메모리를 해제
                "DestroyCursor" => Some((1, Some(1))),

                // API: BOOL IsDialogMessageA(HWND hDlg, LPMSG lpMsg)
                // 역할: 메시지가 지정된 대화 상자용인지 확인하고 처리
                "IsDialogMessageA" => Some((2, Some(0))),

                // API: void PostQuitMessage(int nExitCode)
                // 역할: 시스템에 스레드가 조만간 종료될 것임을 알림
                "PostQuitMessage" => Some((1, None)),

                // API: HWND SetFocus(HWND hWnd)
                // 역할: 특정 창에 키보드 포커스를 설정
                "SetFocus" => Some((1, Some(0))),

                // API: HWND GetFocus(void)
                // 역할: 현재 키보드 포커스가 있는 창의 핸들을 가져옴
                "GetFocus" => Some((0, Some(0))),

                // API: LRESULT DispatchMessageA(const MSG* lpMsg)
                // 역할: 창 프로시저에 메시지를 전달
                "DispatchMessageA" => Some((1, Some(0))),

                // API: BOOL TranslateMessage(const MSG* lpMsg)
                // 역할: 가상 키 메시지를 문자 메시지로 변환
                "TranslateMessage" => Some((1, Some(0))),

                // API: BOOL PeekMessageA(LPMSG lpMsg, HWND hWnd, UINT wMsgFilterMin, UINT wMsgFilterMax, UINT wRemoveMsg)
                // 역할: 메시지 큐에서 메시지를 확인 (대기하지 않음)
                "PeekMessageA" => Some((5, Some(0))), // WM_QUIT 없음

                // API: BOOL GetMessageA(LPMSG lpMsg, HWND hWnd, UINT wMsgFilterMin, UINT wMsgFilterMax)
                // 역할: 메시지 큐에서 메시지를 가져옴 (메시지가 올 때까지 대기)
                "GetMessageA" => Some((4, Some(0))), // WM_QUIT (0 = 종료)

                // API: DWORD MsgWaitForMultipleObjects(DWORD nCount, const HANDLE* pHandles, BOOL fWaitAll, DWORD dwMilliseconds, DWORD dwWakeMask)
                // 역할: 하나 이상의 개체 또는 메시지가 큐에 도착할 때까지 대기
                "MsgWaitForMultipleObjects" => Some((5, Some(0))),

                // API: HWND GetWindow(HWND hWnd, UINT uCmd)
                // 역할: 지정된 창과 관계가 있는 창의 핸들을 가져옴
                "GetWindow" => Some((2, Some(0))),

                // API: HWND GetParent(HWND hWnd)
                // 역할: 지정된 창의 부모 또는 소유자 창의 핸들을 가져옴
                "GetParent" => Some((1, Some(0))),

                // API: HWND GetDesktopWindow(void)
                // 역할: 데스크톱 창의 핸들을 가져옴
                "GetDesktopWindow" => Some((0, Some(0x0001))),

                // API: HWND GetActiveWindow(void)
                // 역할: 현재 스레드와 연결된 활성 창의 핸들을 가져옴
                "GetActiveWindow" => Some((0, Some(0))),

                // API: HWND GetLastActivePopup(HWND hWnd)
                // 역할: 지정된 창에서 마지막으로 활성화된 팝업 창을 확인
                "GetLastActivePopup" => Some((1, Some(0))),

                // API: BOOL GetMenuItemInfoA(HMENU hMenu, UINT item, BOOL fByPos, LPMENUITEMINFOA lpmii)
                // 역할: 메뉴 항목에 대한 정보를 가져옴
                "GetMenuItemInfoA" => Some((4, Some(0))),

                // API: BOOL DeleteMenu(HMENU hMenu, UINT uPosition, UINT uFlags)
                // 역할: 메뉴에서 항목을 삭제
                "DeleteMenu" => Some((3, Some(1))),

                // API: BOOL RemoveMenu(HMENU hMenu, UINT uPosition, UINT uFlags)
                // 역할: 메뉴 항목을 제거 (파괴하지 않음)
                "RemoveMenu" => Some((3, Some(1))),

                // API: HMENU GetSystemMenu(HWND hWnd, BOOL bRevert)
                // 역할: 복사/수정용 시스템 메뉴 핸들을 가져옴
                "GetSystemMenu" => Some((2, Some(0))),

                // API: HMENU GetMenu(HWND hWnd)
                // 역할: 지정된 창의 메뉴 핸들을 가져옴
                "GetMenu" => Some((1, Some(0))),

                // API: BOOL AppendMenuA(HMENU hMenu, UINT uFlags, UINT_PTR uIDNewItem, LPCSTR lpNewItem)
                // 역할: 메뉴 끝에 새 항목을 추가
                "AppendMenuA" => Some((4, Some(1))),

                // API: HMENU CreateMenu(void)
                // 역할: 메뉴를 생성
                "CreateMenu" => {
                    let ctx = uc.get_data();
                    let hmenu = ctx.alloc_handle();
                    Some((0, Some(hmenu as i32)))
                }

                // API: BOOL DestroyMenu(HMENU hMenu)
                // 역할: 메뉴를 파괴
                "DestroyMenu" => Some((1, Some(1))),

                // API: LRESULT DefWindowProcA(HWND hWnd, UINT Msg, WPARAM wParam, LPARAM lParam)
                // 역할: 기본 창 프로시저를 호출하여 메시지를 처리
                "DefWindowProcA" => Some((4, Some(0))),

                // API: LRESULT DefMDIChildProcA(HWND hWnd, UINT Msg, WPARAM wParam, LPARAM lParam)
                // 역할: MDI 자식 창의 기본 메시지 처리를 수행
                "DefMDIChildProcA" => Some((4, Some(0))),

                // API: LRESULT DefFrameProcA(HWND hWnd, HWND hWndMDIClient, UINT Msg, WPARAM wParam, LPARAM lParam)
                // 역할: MDI 프레임 창의 기본 메시지 처리를 수행
                "DefFrameProcA" => Some((5, Some(0))),

                // API: LONG SetWindowLongA(HWND hWnd, int nIndex, LONG dwNewLong)
                // 역할: 창의 특성을 변경
                "SetWindowLongA" => Some((3, Some(0))),

                // API: LONG GetWindowLongA(HWND hWnd, int nIndex)
                // 역할: 창의 특성 정보를 가져옴
                "GetWindowLongA" => Some((2, Some(0))),

                // API: LRESULT CallWindowProcA(WNDPROC lpPrevWndFunc, HWND hWnd, UINT Msg, WPARAM wParam, LPARAM lParam)
                // 역할: 지정된 창 프로시저로 메시지를 전달
                "CallWindowProcA" => Some((5, Some(0))),

                // API: BOOL PostThreadMessageA(DWORD idThread, UINT Msg, WPARAM wParam, LPARAM lParam)
                // 역할: 특정 스레드의 메시지 큐에 메시지를 배치
                "PostThreadMessageA" => Some((4, Some(1))),

                // API: HDC BeginPaint(HWND hWnd, LPPAINTSTRUCT lpPaint)
                // 역할: 그리기를 위해 창을 준비
                "BeginPaint" => {
                    let _hwnd = uc.read_arg(0);
                    let ps_addr = uc.read_arg(1);
                    let ctx = uc.get_data();
                    let hdc = ctx.alloc_handle();
                    // PAINTSTRUCT: HDC at offset 0
                    uc.write_u32(ps_addr as u64, hdc);
                    Some((2, Some(hdc as i32)))
                }

                // API: BOOL EndPaint(HWND hWnd, const PAINTSTRUCT* lpPaint)
                // 역할: 그리기가 완료되었음을 알림
                "EndPaint" => Some((2, Some(1))),

                // API: int ScrollWindowEx(HWND hWnd, int dx, int dy, const RECT* prcScroll, const RECT* prcClip, HRGN hrgnUpdate, LPRECT prcUpdate, UINT flags)
                // 역할: 창의 클라이언트 영역 내용을 스크롤
                "ScrollWindowEx" => Some((8, Some(0))),

                // API: BOOL InvalidateRect(HWND hWnd, const RECT* lpRect, BOOL bErase)
                // 역할: 창의 클라이언트 영역 중 일부를 갱신 대상으로 설정
                "InvalidateRect" => Some((3, Some(1))),

                // API: int SetScrollInfo(HWND hWnd, int nBar, LPCSCROLLINFO lpsi, BOOL redraw)
                // 역할: 스크롤 바의 매개변수를 설정
                "SetScrollInfo" => Some((4, Some(0))),

                // API: BOOL SetWindowTextA(HWND hWnd, LPCSTR lpString)
                // 역할: 창의 제목 표시줄 텍스트 또는 컨트롤의 텍스트를 변경
                "SetWindowTextA" => {
                    let hwnd = uc.read_arg(0);
                    let text_addr = uc.read_arg(1);
                    let text = uc.read_euc_kr(text_addr as u64);
                    uc.get_data()
                        .win_frame
                        .lock()
                        .unwrap()
                        .set_window_text(hwnd, text.clone());
                    crate::emu_log!("[USER32] SetWindowTextA(HWND {:#x}, \"{}\")", hwnd, text);
                    Some((2, Some(1)))
                }

                // API: int GetWindowTextA(HWND hWnd, LPSTR lpString, int nMaxCount)
                // 역할: 창의 제목 표시줄 텍스트 또는 컨트롤의 텍스트를 가져옴
                "GetWindowTextA" => {
                    let hwnd = uc.read_arg(0);
                    let buf_addr = uc.read_arg(1);
                    let max_count = uc.read_arg(2);

                    let title_info = {
                        let ctx = uc.get_data();
                        let win_frame = ctx.win_frame.lock().unwrap();
                        win_frame.windows.get(&hwnd).map(|win| {
                            let len = win.title.len().min(max_count as usize - 1);
                            (win.title[..len].to_string(), len)
                        })
                    };

                    if let Some((text, len)) = title_info {
                        uc.write_string(buf_addr as u64, &text);
                        Some((3, Some(len as i32)))
                    } else {
                        Some((3, Some(0)))
                    }
                }

                // API: BOOL KillTimer(HWND hWnd, UINT_PTR uIDEvent)
                // 역할: 타이머를 중지
                "KillTimer" => Some((2, Some(1))),

                // API: UINT_PTR SetTimer(HWND hWnd, UINT_PTR nIDEvent, UINT uElapse, TIMERPROC lpTimerFunc)
                // 역할: 타이머를 생성
                "SetTimer" => {
                    let hwnd = uc.read_arg(0);
                    let id = uc.read_arg(1);
                    crate::emu_log!("[USER32] SetTimer({:#x}, {})", hwnd, id);
                    Some((4, Some(id as i32)))
                }

                // API: int MapWindowPoints(HWND hWndFrom, HWND hWndTo, LPPOINT lpPoints, UINT cPoints)
                // 역할: 한 창의 상대 좌표를 다른 창의 상대 좌표로 변환
                "MapWindowPoints" => Some((4, Some(0))),

                // API: BOOL SystemParametersInfoA(UINT uiAction, UINT uiParam, PVOID pvParam, UINT fWinIni)
                // 역할: 시스템 전체의 매개변수를 가져오거나 설정
                "SystemParametersInfoA" => Some((4, Some(1))),

                // API: BOOL TranslateMDISysAccel(HWND hWndClient, LPMSG lpMsg)
                // 역할: MDI 자식 창의 바로 가기 키 메시지를 처리
                "TranslateMDISysAccel" => Some((2, Some(0))),

                // API: int DrawTextA(HDC hDC, LPCSTR lpchText, int nCount, LPRECT lpRect, UINT uFormat)
                // 역할: 서식화된 텍스트를 사각형 내에 그림
                "DrawTextA" => Some((5, Some(0))),

                // API: BOOL GetCursorPos(LPPOINT lpPoint)
                // 역할: 마우스 커서의 현재 위치를 화면 좌표로 가져옴
                "GetCursorPos" => {
                    let pt_addr = uc.read_arg(0);
                    uc.write_u32(pt_addr as u64, 320);
                    uc.write_u32(pt_addr as u64 + 4, 240);
                    Some((1, Some(1)))
                }

                // API: BOOL PtInRect(const RECT* lprc, POINT pt)
                // 역할: 점이 사각형 내부에 있는지 확인
                "PtInRect" => Some((2, Some(0))),

                // API: BOOL SetRect(LPRECT lprc, int xLeft, int yTop, int xRight, int yBottom)
                // 역할: 사각형의 좌표를 설정
                "SetRect" => Some((5, Some(1))),

                // API: BOOL EqualRect(const RECT* lprc1, const RECT* lprc2)
                // 역할: 두 사각형이 동일한지 확인
                "EqualRect" => Some((2, Some(0))),

                // API: BOOL UnionRect(LPRECT lprcDst, const RECT* lprcSrc1, const RECT* lprcSrc2)
                // 역할: 두 사각형을 모두 포함하는 최소 사각형을 계산
                "UnionRect" => Some((3, Some(1))),

                // API: BOOL IntersectRect(LPRECT lprcDst, const RECT* lprcSrc1, const RECT* lprcSrc2)
                // 역할: 두 사각형의 교집합 사각형을 계산
                "IntersectRect" => Some((3, Some(0))),

                // API: HANDLE GetClipboardData(UINT uFormat)
                // 역할: 클립보드에서 데이터를 가져옴
                "GetClipboardData" => Some((1, Some(0))),

                // API: BOOL OpenClipboard(HWND hWndNewOwner)
                // 역할: 다른 창이 클립보드 내용을 수정하지 못하도록 클립보드를 엶
                "OpenClipboard" => Some((1, Some(1))),

                // API: BOOL CloseClipboard(void)
                // 역할: 클립보드를 닫음
                "CloseClipboard" => Some((0, Some(1))),

                // API: BOOL EmptyClipboard(void)
                // 역할: 클립보드 내용을 비우고 메모리 소유권을 가져옴
                "EmptyClipboard" => Some((0, Some(1))),

                // API: HANDLE SetClipboardData(UINT uFormat, HANDLE hMem)
                // 역할: 특정 포맷으로 데이터를 클립보드에 배치
                "SetClipboardData" => Some((2, Some(0))),

                // API: BOOL IsClipboardFormatAvailable(UINT format)
                // 역할: 클립보드에 특정 포맷의 데이터가 있는지 확인
                "IsClipboardFormatAvailable" => Some((1, Some(0))),

                // API: HWND SetCapture(HWND hWnd)
                // 역할: 마우스 캡처를 특정 창으로 설정
                "SetCapture" => Some((1, Some(0))),

                // API: HWND GetCapture(void)
                // 역할: 마우스 캡처가 있는 창의 핸들을 가져옴
                "GetCapture" => Some((0, Some(0))),

                // API: BOOL ReleaseCapture(void)
                // 역할: 마우스 캡처를 해제
                "ReleaseCapture" => Some((0, Some(1))),

                // API: BOOL ScreenToClient(HWND hWnd, LPPOINT lpPoint)
                // 역할: 화면 좌표를 특정 창의 클라이언트 영역 좌표로 변환
                "ScreenToClient" => Some((2, Some(1))),

                // API: BOOL ClientToScreen(HWND hWnd, LPPOINT lpPoint)
                // 역할: 클라이언트 영역 좌표를 화면 좌표로 변환
                "ClientToScreen" => Some((2, Some(1))),

                // API: BOOL CreateCaret(HWND hWnd, HBITMAP hBitmap, int nWidth, int nHeight)
                // 역할: 시스템 캐럿을 생성
                "CreateCaret" => Some((4, Some(1))),

                // API: BOOL DestroyCaret(void)
                // 역할: 현재 캐럿을 파괴
                "DestroyCaret" => Some((0, Some(1))),

                // API: BOOL ShowCaret(HWND hWnd)
                // 역할: 캐럿을 화면에 표시
                "ShowCaret" => Some((1, Some(1))),

                // API: BOOL HideCaret(HWND hWnd)
                // 역할: 화면에서 캐럿을 숨김
                "HideCaret" => Some((1, Some(1))),

                // API: BOOL SetCaretPos(int X, int Y)
                // 역할: 캐럿의 위치를 이동
                "SetCaretPos" => Some((2, Some(1))),

                // API: SHORT GetAsyncKeyState(int vKey)
                // 역할: 특정 키의 상태를 확인 (비동기식)
                "GetAsyncKeyState" => Some((1, Some(0))),

                // API: SHORT GetKeyState(int nVirtKey)
                // 역할: 특정 키의 상태를 확인 (메시지 큐 기반)
                "GetKeyState" => Some((1, Some(0))),

                // API: DWORD GetSysColor(int nIndex)
                // 역할: 시스템 요소의 색상 값을 가져옴
                "GetSysColor" => Some((1, Some(0x00C0C0C0u32 as i32))), // COLOR_BTNFACE

                // API: int SetWindowRgn(HWND hWnd, HRGN hRgn, BOOL bRedraw)
                // 역할: 창의 영역을 설정
                "SetWindowRgn" => Some((3, Some(1))),

                // API: BOOL GetClassInfoExA(HINSTANCE hinst, LPCSTR lpszClass, PWNDCLASSEXA lpwcx)
                // 역할: 창 클래스에 대한 정보를 가져옴
                "GetClassInfoExA" => Some((3, Some(0))),

                // API: BOOL IsZoomed(HWND hWnd)
                // 역할: 창이 최대화되어 있는지 확인
                "IsZoomed" => Some((1, Some(0))),

                // API: BOOL IsIconic(HWND hWnd)
                // 역할: 창이 최소화(아이콘화)되어 있는지 확인
                "IsIconic" => Some((1, Some(0))),

                // API: int wsprintfA(LPSTR lpOut, LPCSTR lpFmt, ...)
                // 역할: 서식화된 데이터를 문자열로 출력 (가변 인자)
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
                        crate::emu_log!(
                            "[USER32] wsprintfA invalid args: buf={:#x}, fmt={:#x}, eip={:#x}, esp={:#x}",
                            buf_addr,
                            fmt_addr,
                            eip,
                            esp
                        );
                    } else {
                        crate::emu_log!("[USER32] wsprintfA(...)");
                    }
                    Some((2, Some(0)))
                }

                // API: BOOL EndDialog(HWND hDlg, INT_PTR nResult)
                // 역할: 대화 상자를 종료
                "EndDialog" => Some((2, Some(1))),

                _ => {
                    crate::emu_log!("[USER32] UNHANDLED: {}", func_name);
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
