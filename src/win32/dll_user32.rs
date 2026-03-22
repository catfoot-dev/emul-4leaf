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
                    let hwnd = uc.read_arg(0);
                    let text_addr = uc.read_arg(1);
                    let caption_addr = uc.read_arg(2);
                    let u_type = uc.read_arg(3);
                    let text = uc.read_euc_kr(text_addr as u64);
                    let caption = uc.read_euc_kr(caption_addr as u64);

                    let result = uc
                        .get_data()
                        .win_event
                        .lock()
                        .unwrap()
                        .message_box(caption.clone(), text.clone(), u_type);

                    crate::emu_log!(
                        "[USER32] MessageBoxA({:#x}, \"{}\", \"{}\", {:#x}) -> {:#x}",
                        hwnd,
                        caption,
                        text,
                        u_type,
                        result
                    );
                    Some((4, Some(result)))
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
                    let ex_style = uc.read_arg(0);
                    let class_addr = uc.read_arg(1);
                    let title_addr = uc.read_arg(2);
                    let style = uc.read_arg(3);
                    let x = uc.read_arg(4);
                    let y = uc.read_arg(5);
                    let width = uc.read_arg(6);
                    let height = uc.read_arg(7);
                    let parent = uc.read_arg(8);
                    let menu_or_id = uc.read_arg(9);
                    let instance = uc.read_arg(10);
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
                        ex_style,
                        parent,
                        id: menu_or_id,
                        visible: false,
                        enabled: true,
                        zoomed: false,
                        iconic: false,
                        wnd_proc: 0,
                        user_data: 0,
                    };

                    uc.get_data()
                        .win_event
                        .lock()
                        .unwrap()
                        .create_window(hwnd, window_state);

                    crate::emu_log!(
                        "[USER32] CreateWindowExA({:#x}, \"{}\", \"{}\", {:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> HWND {:#x}",
                        ex_style,
                        class_name,
                        title,
                        style,
                        x,
                        y,
                        width,
                        height,
                        parent,
                        menu_or_id,
                        instance,
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
                        .win_event
                        .lock()
                        .unwrap()
                        .show_window(hwnd, visible);
                    crate::emu_log!(
                        "[USER32] ShowWindow({:#x}, {:#x}) -> BOOL 1",
                        hwnd,
                        n_cmd_show
                    );
                    Some((2, Some(1)))
                }

                // API: BOOL UpdateWindow(HWND hWnd)
                // 역할: 창의 클라이언트 영역을 강제로 업데이트
                "UpdateWindow" => {
                    let hwnd = uc.read_arg(0);
                    uc.get_data().win_event.lock().unwrap().update_window(hwnd);
                    crate::emu_log!("[USER32] UpdateWindow({:#x}) -> BOOL 1", hwnd);
                    Some((1, Some(1)))
                }

                // API: BOOL DestroyWindow(HWND hWnd)
                // 역할: 지정된 창을 파괴
                "DestroyWindow" => {
                    let hwnd = uc.read_arg(0);
                    let ctx = uc.get_data();
                    ctx.win_event.lock().unwrap().destroy_window(hwnd);
                    crate::emu_log!("[USER32] DestroyWindow({:#x}) -> BOOL 1", hwnd);
                    Some((1, Some(1)))
                }

                // API: BOOL CloseWindow(HWND hWnd)
                // 역할: 지정된 창을 최소화
                // 구현 생략 사유: 창을 최소화하는 시각적 효과만 가지므로 에뮬레이터 핵심 로직과 무관함.
                "CloseWindow" => {
                    let hwnd = uc.read_arg(0);
                    crate::emu_log!("[USER32] CloseWindow({:#x}) -> BOOL 1", hwnd);
                    Some((1, Some(1)))
                }

                // API: BOOL EnableWindow(HWND hWnd, BOOL bEnable)
                // 역할: 창의 마우스 및 키보드 입력을 활성화 또는 비활성화
                "EnableWindow" => {
                    let hwnd = uc.read_arg(0);
                    let enable = uc.read_arg(1) != 0;
                    let ctx = uc.get_data();
                    let mut win_event = ctx.win_event.lock().unwrap();
                    let old_state = if let Some(win) = win_event.windows.get_mut(&hwnd) {
                        let prev = win.enabled;
                        win.enabled = enable;
                        prev
                    } else {
                        false
                    };
                    crate::emu_log!(
                        "[USER32] EnableWindow({:#x}, {}) -> BOOL {}",
                        hwnd,
                        enable,
                        if old_state { 1 } else { 0 }
                    );
                    Some((2, Some(if old_state { 1 } else { 0 })))
                }

                // API: BOOL IsWindowEnabled(HWND hWnd)
                // 역할: 창이 활성화되어 있는지 확인
                "IsWindowEnabled" => {
                    let hwnd = uc.read_arg(0);
                    let ctx = uc.get_data();
                    let win_event = ctx.win_event.lock().unwrap();
                    let enabled = win_event
                        .windows
                        .get(&hwnd)
                        .map(|w| w.enabled)
                        .unwrap_or(false);
                    crate::emu_log!(
                        "[USER32] IsWindowEnabled({:#x}) -> BOOL {}",
                        hwnd,
                        if enabled { 1 } else { 0 }
                    );
                    Some((1, Some(if enabled { 1 } else { 0 })))
                }

                // API: BOOL IsWindowVisible(HWND hWnd)
                // 역할: 창의 가시성 상태를 확인
                "IsWindowVisible" => {
                    let hwnd = uc.read_arg(0);
                    let ctx = uc.get_data();
                    let win_event = ctx.win_event.lock().unwrap();
                    let visible = win_event
                        .windows
                        .get(&hwnd)
                        .map(|w| w.visible)
                        .unwrap_or(false);
                    crate::emu_log!(
                        "[USER32] IsWindowVisible({:#x}) -> BOOL {}",
                        hwnd,
                        if visible { 1 } else { 0 }
                    );
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
                    let b_repaint = uc.read_arg(5);

                    let ctx = uc.get_data();
                    let mut win_event = ctx.win_event.lock().unwrap();
                    win_event.move_window(hwnd, x, y, width, height);
                    crate::emu_log!(
                        "[USER32] MoveWindow({:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 1",
                        hwnd,
                        x,
                        y,
                        width,
                        height,
                        b_repaint
                    );
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
                    let u_flags = uc.read_arg(6);

                    let ctx = uc.get_data();
                    let mut win_event = ctx.win_event.lock().unwrap();
                    win_event.move_window(hwnd, x, y, cx, cy);
                    crate::emu_log!(
                        "[USER32] SetWindowPos({:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 1",
                        hwnd,
                        x,
                        y,
                        cx,
                        cy,
                        u_flags
                    );
                    Some((7, Some(1)))
                }

                // API: BOOL GetWindowRect(HWND hWnd, LPRECT lpRect)
                // 역할: 창의 화면 좌표상의 경계 사각형 좌표를 가져옴
                "GetWindowRect" => {
                    let hwnd = uc.read_arg(0);
                    let rect_addr = uc.read_arg(1);
                    let (x, y, w, h) = {
                        let ctx = uc.get_data();
                        let win_event = ctx.win_event.lock().unwrap();
                        win_event
                            .windows
                            .get(&hwnd)
                            .map(|win| (win.x, win.y, win.width, win.height))
                            .unwrap_or((0, 0, 640, 480))
                    };

                    uc.write_u32(rect_addr as u64, x as u32);
                    uc.write_u32(rect_addr as u64 + 4, y as u32);
                    uc.write_u32(rect_addr as u64 + 8, (x + w) as u32);
                    uc.write_u32(rect_addr as u64 + 12, (y + h) as u32);
                    crate::emu_log!(
                        "[USER32] GetWindowRect({:#x}, {:#x}) -> BOOL 1",
                        hwnd,
                        rect_addr
                    );
                    Some((2, Some(1)))
                }

                // API: BOOL GetClientRect(HWND hWnd, LPRECT lpRect)
                // 역할: 창의 클라이언트 영역 좌표를 가져옴
                "GetClientRect" => {
                    let hwnd = uc.read_arg(0);
                    let rect_addr = uc.read_arg(1);
                    let (w, h) = {
                        let ctx = uc.get_data();
                        let win_event = ctx.win_event.lock().unwrap();
                        win_event
                            .windows
                            .get(&hwnd)
                            .map(|win| (win.width, win.height))
                            .unwrap_or((640, 480))
                    };

                    uc.write_u32(rect_addr as u64, 0);
                    uc.write_u32(rect_addr as u64 + 4, 0);
                    uc.write_u32(rect_addr as u64 + 8, w as u32);
                    uc.write_u32(rect_addr as u64 + 12, h as u32);
                    crate::emu_log!(
                        "[USER32] GetClientRect({:#x}, {:#x}) -> BOOL 1",
                        hwnd,
                        rect_addr
                    );
                    Some((2, Some(1)))
                }

                // API: BOOL AdjustWindowRectEx(LPRECT lpRect, DWORD dwStyle, BOOL bMenu, DWORD dwExStyle)
                // 역할: 클라이언트 영역의 크기를 기준으로 원하는 창의 크기를 계산
                // 구현 생략 사유: 클라이언트 영역을 기반으로 전체 창 크기를 계산하는 보조 함수. 에뮬레이터에서는 창 크기를 정밀하게 다루지 않으므로 성공(1)만 반환함.
                "AdjustWindowRectEx" => {
                    crate::emu_log!("[USER32] AdjustWindowRectEx stubbed");
                    Some((4, Some(1)))
                }

                // API: HDC GetDC(HWND hWnd)
                // 역할: 지정된 창의 클라이언트 영역에 대한 DC를 가져옴
                "GetDC" => {
                    let hwnd = uc.read_arg(0);
                    let ctx = uc.get_data();
                    let hdc = ctx.alloc_handle();
                    ctx.gdi_objects.lock().unwrap().insert(
                        hdc,
                        crate::win32::GdiObject::Dc {
                            associated_window: 0,
                            selected_bitmap: 0,
                            selected_font: 0,
                            selected_brush: 0,
                            selected_pen: 0,
                            selected_region: 0,
                            selected_palette: 0,
                            bk_mode: 0,
                            bk_color: 0,
                            text_color: 0,
                            rop2_mode: 0,
                            current_x: 0,
                            current_y: 0,
                        },
                    );
                    crate::emu_log!("[USER32] GetDC({:#x}) -> HDC {:#x}", hwnd, hdc);
                    Some((1, Some(hdc as i32)))
                }

                // API: HDC GetWindowDC(HWND hWnd)
                // 역할: 지정된 창 전체(비클라이언트 영역 포함)에 대한 DC를 가져옴
                "GetWindowDC" => {
                    let hwnd = uc.read_arg(0);
                    let ctx = uc.get_data();
                    let hdc = ctx.alloc_handle();
                    ctx.gdi_objects.lock().unwrap().insert(
                        hdc,
                        crate::win32::GdiObject::Dc {
                            associated_window: 0,
                            selected_bitmap: 0,
                            selected_font: 0,
                            selected_brush: 0,
                            selected_pen: 0,
                            selected_region: 0,
                            selected_palette: 0,
                            bk_mode: 0,
                            bk_color: 0,
                            text_color: 0,
                            rop2_mode: 0,
                            current_x: 0,
                            current_y: 0,
                        },
                    );
                    crate::emu_log!("[USER32] GetWindowDC({:#x}) -> HDC {:#x}", hwnd, hdc);
                    Some((1, Some(hdc as i32)))
                }

                // API: int ReleaseDC(HWND hWnd, HDC hDC)
                // 역할: DC를 해제
                "ReleaseDC" => {
                    let hwnd = uc.read_arg(0);
                    let hdc = uc.read_arg(1);
                    let ctx = uc.get_data();
                    ctx.gdi_objects.lock().unwrap().remove(&hdc);
                    crate::emu_log!("[USER32] ReleaseDC({:#x}, {:#x}) -> INT 1", hwnd, hdc);
                    Some((2, Some(1)))
                }

                // API: LRESULT SendMessageA(HWND hWnd, UINT Msg, WPARAM wParam, LPARAM lParam)
                // 역할: 지정된 창에 메시지를 전송하고 처리가 완료될 때까지 대기
                // 구현 생략 사유: 대상 창의 메시지 프로시저를 동기적으로 직접 호출하는 복잡한 라우팅이 필요함. 단순 에뮬레이션에서는 콜백 재귀 호출시 스택 오버플로우나 락(Lock) 데드락 위험이 커서 무시함.
                "SendMessageA" => {
                    let hwnd = uc.read_arg(0);
                    let msg = uc.read_arg(1);
                    let wparam = uc.read_arg(2);
                    let lparam = uc.read_arg(3);
                    crate::emu_log!(
                        "[USER32] SendMessageA({:#x}, {:#x}, {:#x}, {:#x}) -> LRESULT 0",
                        hwnd,
                        msg,
                        wparam,
                        lparam
                    );
                    Some((4, Some(0)))
                }

                // API: BOOL PostMessageA(HWND hWnd, UINT Msg, WPARAM wParam, LPARAM lParam)
                // 역할: 지정된 창의 메시지 큐에 메시지를 배치
                "PostMessageA" => {
                    let hwnd = uc.read_arg(0);
                    let msg = uc.read_arg(1);
                    let wparam = uc.read_arg(2);
                    let lparam = uc.read_arg(3);
                    let time = uc.get_data().start_time.elapsed().as_millis() as u32;
                    let ctx = uc.get_data();
                    ctx.message_queue
                        .lock()
                        .unwrap()
                        .push_back([hwnd, msg, wparam, lparam, time, 0, 0]);
                    crate::emu_log!(
                        "[USER32] PostMessageA({:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 1",
                        hwnd,
                        msg,
                        wparam,
                        lparam
                    );
                    Some((4, Some(1)))
                }

                // API: HCURSOR LoadCursorA(HINSTANCE hInstance, LPCSTR lpCursorName)
                // 역할: 커서 리소스를 로드
                "LoadCursorA" => {
                    let instance = uc.read_arg(0);
                    let lpcursorname = uc.read_arg(1);
                    let handle = uc.get_data().alloc_handle();
                    crate::emu_log!(
                        "[USER32] LoadCursorA({:#x}, {:#x}) -> HCURSOR {:#x}",
                        instance,
                        lpcursorname,
                        handle
                    );
                    Some((2, Some(handle as i32)))
                }

                // API: HCURSOR LoadCursorFromFileA(LPCSTR lpFileName)
                // 역할: 파일에서 커서를 로드
                "LoadCursorFromFileA" => {
                    let lpfilename = uc.read_arg(0);
                    let handle = uc.get_data().alloc_handle();
                    crate::emu_log!(
                        "[USER32] LoadCursorFromFileA({:#x}) -> HCURSOR {:#x}",
                        lpfilename,
                        handle
                    );
                    Some((1, Some(handle as i32)))
                }

                // API: HICON LoadIconA(HINSTANCE hInstance, LPCSTR lpIconName)
                // 역할: 아이콘 리소스를 로드
                "LoadIconA" => {
                    let instance = uc.read_arg(0);
                    let lpiconname = uc.read_arg(1);
                    let handle = uc.get_data().alloc_handle();
                    crate::emu_log!(
                        "[USER32] LoadIconA({:#x}, {:#x}) -> HICON {:#x}",
                        instance,
                        lpiconname,
                        handle
                    );
                    Some((2, Some(handle as i32)))
                }

                // API: HCURSOR SetCursor(HCURSOR hCursor)
                // 역할: 마우스 커서를 설정
                // 구현 생략 사유: 마우스 커서의 시각적 변경은 게임 내부 상태에 영향을 주지 않으므로 생략함.
                "SetCursor" => {
                    let hcursor = uc.read_arg(0);
                    crate::emu_log!("[USER32] SetCursor({:#x}) -> HCURSOR 0", hcursor);
                    Some((1, Some(0)))
                }

                // API: BOOL DestroyCursor(HCURSOR hCursor)
                // 역할: 커서를 파괴하고 사용된 메모리를 해제
                // 구현 생략 사유: 커서 리소스 해제는 운영체제 몫이며 에뮬레이터 내에서 누수를 추적할 만큼 중요한 리소스가 아님.
                "DestroyCursor" => {
                    let hcursor = uc.read_arg(0);
                    crate::emu_log!("[USER32] DestroyCursor({:#x}) -> BOOL 1", hcursor);
                    Some((1, Some(1)))
                }

                // API: BOOL IsDialogMessageA(HWND hDlg, LPMSG lpMsg)
                // 역할: 메시지가 지정된 대화 상자용인지 확인하고 처리
                // 구현 생략 사유: 모달/모델리스 대화상자 특수 메시지 처리기. 게임 엔진에서는 보통 쓰이지 않으므로 무시함.
                "IsDialogMessageA" => {
                    let h_dlg = uc.read_arg(0);
                    let lp_msg = uc.read_arg(1);
                    crate::emu_log!(
                        "[USER32] IsDialogMessageA({:#x}, {:#x}) -> BOOL 0",
                        h_dlg,
                        lp_msg
                    );
                    Some((2, Some(0)))
                }

                // API: void PostQuitMessage(int nExitCode)
                // 역할: 시스템에 스레드가 조만간 종료될 것임을 알림
                "PostQuitMessage" => {
                    let exit_code = uc.read_arg(0);
                    let time = uc.get_data().start_time.elapsed().as_millis() as u32;
                    let ctx = uc.get_data();
                    // WM_QUIT = 0x0012
                    ctx.message_queue
                        .lock()
                        .unwrap()
                        .push_back([0, 0x0012, exit_code, 0, time, 0, 0]);
                    crate::emu_log!("[USER32] PostQuitMessage({:#x}) -> void", exit_code);
                    Some((1, None))
                }

                // API: HWND SetFocus(HWND hWnd)
                // 역할: 특정 창에 키보드 포커스를 설정
                "SetFocus" => {
                    let hwnd = uc.read_arg(0);
                    let ctx = uc.get_data();
                    let old = ctx
                        .focus_hwnd
                        .swap(hwnd, std::sync::atomic::Ordering::SeqCst);
                    crate::emu_log!("[USER32] SetFocus({:#x}) -> HWND {:#x}", hwnd, old);
                    Some((1, Some(old as i32)))
                }

                // API: HWND GetFocus(void)
                // 역할: 현재 키보드 포커스가 있는 창의 핸들을 가져옴
                "GetFocus" => {
                    let ctx = uc.get_data();
                    let hwnd = ctx.focus_hwnd.load(std::sync::atomic::Ordering::SeqCst);
                    crate::emu_log!("[USER32] GetFocus() -> HWND {:#x}", hwnd);
                    Some((0, Some(hwnd as i32)))
                }

                // API: LRESULT DispatchMessageA(const MSG* lpMsg)
                // 역할: 창 프로시저에 메시지를 전달
                // 구현 생략 사유: GetMessageA를 통해 가져온 메시지를 윈도우 프로시저로 디스패치해야 하나, 에뮬레이터가 스레드를 강제로 제어하기 어려워 No-op 처리함.
                "DispatchMessageA" => {
                    let lpmsg = uc.read_arg(0);
                    crate::emu_log!("[USER32] DispatchMessageA({:#x}) -> LRESULT 0", lpmsg);
                    Some((1, Some(0)))
                }
                // API: BOOL TranslateMessage(const MSG* lpMsg)
                // 역할: 가상 키 메시지를 문자 메시지로 변환
                // 구현 생략 사유: 키보드 스캔 코드를 문자 메시지(WM_CHAR)로 변환하는 함수. 키 입력 처리는 별도 로직으로 우회할 예정이므로 무시함.
                "TranslateMessage" => {
                    let lpmsg = uc.read_arg(0);
                    crate::emu_log!("[USER32] TranslateMessage({:#x}) -> BOOL 1", lpmsg);
                    Some((1, Some(1)))
                }
                // API: BOOL PeekMessageA(LPMSG lpMsg, HWND hWnd, UINT wMsgFilterMin, UINT wMsgFilterMax, UINT uRemoveMsg)
                // 역할: 메시지 큐에서 메시지를 확인 (대기하지 않음)
                "PeekMessageA" => {
                    let msg_addr = uc.read_arg(0);
                    let hwnd = uc.read_arg(1); // Filtering ignored for emulator
                    let min = uc.read_arg(2);
                    let max = uc.read_arg(3);
                    let remove_msg = uc.read_arg(4); // PM_REMOVE = 0x0001

                    let mut found = None;
                    {
                        let ctx = uc.get_data();
                        let mut mq = ctx.message_queue.lock().unwrap();
                        if let Some(msg) = mq.front().copied() {
                            if remove_msg & 1 != 0 {
                                mq.pop_front();
                            }
                            found = Some(msg);
                        }
                    }

                    if let Some(msg) = found {
                        for i in 0..7 {
                            uc.write_u32(msg_addr as u64 + (i as u64 * 4), msg[i]);
                        }
                        if msg[1] == 0x0012 {
                            // WM_QUIT
                            crate::emu_log!(
                                "[USER32] PeekMessageA({:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 0",
                                msg_addr,
                                hwnd,
                                min,
                                max,
                                remove_msg
                            );
                            Some((5, Some(0)))
                        } else {
                            crate::emu_log!(
                                "[USER32] PeekMessageA({:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 1",
                                msg_addr,
                                hwnd,
                                min,
                                max,
                                remove_msg
                            );
                            Some((5, Some(1)))
                        }
                    } else {
                        crate::emu_log!(
                            "[USER32] PeekMessageA({:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 0",
                            msg_addr,
                            hwnd,
                            min,
                            max,
                            remove_msg
                        );
                        Some((5, Some(0)))
                    }
                }

                // API: BOOL GetMessageA(LPMSG lpMsg, HWND hWnd, UINT wMsgFilterMin, UINT wMsgFilterMax)
                // 역할: 메시지 큐에서 메시지를 가져옴 (메시지가 올 때까지 대기)
                "GetMessageA" => {
                    let msg_addr = uc.read_arg(0);
                    let hwnd = uc.read_arg(1);
                    let min = uc.read_arg(2);
                    let max = uc.read_arg(3);

                    let mut found = None;
                    {
                        let ctx = uc.get_data();
                        let mut mq = ctx.message_queue.lock().unwrap();
                        if let Some(msg) = mq.pop_front() {
                            found = Some(msg);
                        }
                    }

                    if let Some(msg) = found {
                        for i in 0..7 {
                            uc.write_u32(msg_addr as u64 + (i as u64 * 4), msg[i]);
                        }
                        if msg[1] == 0x0012 {
                            // WM_QUIT
                            crate::emu_log!(
                                "[USER32] GetMessageA({:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 0",
                                msg_addr,
                                hwnd,
                                min,
                                max
                            );
                            Some((4, Some(0)))
                        } else {
                            crate::emu_log!(
                                "[USER32] GetMessageA({:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 1",
                                msg_addr,
                                hwnd,
                                min,
                                max
                            );
                            Some((4, Some(1)))
                        }
                    } else {
                        // In a real env, this blocks. For emulator, we just return -1 or 0 to yield?
                        // If we block, we freeze. Just yield or fake a WM_PAINT/WM_TIMER?
                        // Returning 0 causes the app to exit. Let's return -1 (error) or just a synthetic message.
                        // For emulator idle, a synthetic WM_NULL (0) or WM_TIMER.
                        let time = uc.get_data().start_time.elapsed().as_millis() as u32;
                        let dummy_msg = [0, 0, 0, 0, time, 0, 0];
                        for i in 0..7 {
                            uc.write_u32(msg_addr as u64 + (i as u64 * 4), dummy_msg[i]);
                        }
                        crate::emu_log!(
                            "[USER32] GetMessageA({:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 1",
                            msg_addr,
                            hwnd,
                            min,
                            max
                        );
                        Some((4, Some(1))) // Proceed execution normally
                    }
                }

                // API: DWORD MsgWaitForMultipleObjects(DWORD nCount, const HANDLE* pHandles, BOOL fWaitAll, DWORD dwMilliseconds, DWORD dwWakeMask)
                // 역할: 하나 이상의 개체 또는 메시지가 큐에 도착할 때까지 대기
                // 구현 생략 사유: 다중 스레드 동기화 객체 대기 함수. 에뮬레이터 특성상 스레드를 멈추면 전체 엔진이 멈추므로 즉각 리턴(Timeout) 처리함.
                "MsgWaitForMultipleObjects" => {
                    let n_count = uc.read_arg(0);
                    let p_handles = uc.read_arg(1);
                    let f_wait_all = uc.read_arg(2);
                    let dw_milliseconds = uc.read_arg(3);
                    let dw_wake_mask = uc.read_arg(4);
                    crate::emu_log!(
                        "[USER32] MsgWaitForMultipleObjects({:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> DWORD 0",
                        n_count,
                        p_handles,
                        f_wait_all,
                        dw_milliseconds,
                        dw_wake_mask
                    );
                    Some((5, Some(0)))
                }
                // API: HWND GetWindow(HWND hWnd, UINT uCmd)
                // 역할: 지정된 창과 관계가 있는 창의 핸들을 가져옴
                "GetWindow" => {
                    let hwnd = uc.read_arg(0);
                    let cmd = uc.read_arg(1);
                    // Minimal stub: GW_OWNER = 4
                    let parent = if cmd == 4 {
                        let ctx = uc.get_data();
                        let win_event = ctx.win_event.lock().unwrap();
                        win_event.windows.get(&hwnd).map(|w| w.parent).unwrap_or(0)
                    } else {
                        0
                    };
                    crate::emu_log!(
                        "[USER32] GetWindow({:#x}, {:#x}) -> HWND {:#x}",
                        hwnd,
                        cmd,
                        parent
                    );
                    Some((2, Some(parent as i32)))
                }

                // API: HWND GetParent(HWND hWnd)
                // 역할: 지정된 창의 부모 또는 소유자 창의 핸들을 가져옴
                "GetParent" => {
                    let hwnd = uc.read_arg(0);
                    let ctx = uc.get_data();
                    let win_event = ctx.win_event.lock().unwrap();
                    let parent = win_event.windows.get(&hwnd).map(|w| w.parent).unwrap_or(0);
                    crate::emu_log!("[USER32] GetParent({:#x}) -> HWND {:#x}", hwnd, parent);
                    Some((1, Some(parent as i32)))
                }

                // API: HWND GetDesktopWindow(void)
                // 역할: 데스크톱 창의 핸들을 가져옴
                "GetDesktopWindow" => {
                    crate::emu_log!("[USER32] GetDesktopWindow() -> HWND {:#x}", 0x0001);
                    Some((0, Some(0x0001)))
                }

                // API: HWND GetActiveWindow(void)
                // 역할: 현재 스레드와 연결된 활성 창의 핸들을 가져옴
                "GetActiveWindow" => {
                    let ctx = uc.get_data();
                    let hwnd = ctx.focus_hwnd.load(std::sync::atomic::Ordering::SeqCst);
                    crate::emu_log!("[USER32] GetActiveWindow() -> HWND {:#x}", hwnd);
                    Some((0, Some(hwnd as i32)))
                }

                // API: HWND GetLastActivePopup(HWND hWnd)
                // 역할: 지정된 창에서 마지막으로 활성화된 팝업 창을 확인
                // 구현 생략 사유: 다중 창 환경의 포커스 관리용. 팝업 창을 사용하지 않으므로 무시함.
                "GetLastActivePopup" => {
                    let hwnd = uc.read_arg(0);
                    crate::emu_log!("[USER32] GetLastActivePopup({:#x}) -> HWND {:#x}", hwnd, 0);
                    Some((1, Some(0)))
                }

                // API: BOOL GetMenuItemInfoA(HMENU hMenu, UINT item, BOOL fByPos, LPMENUITEMINFOA lpmii)
                // 역할: 메뉴 항목에 대한 정보를 가져옴
                // 구현 생략 사유: 메뉴 아이템 속성 조회. 에뮬레이터에서는 렌더링 가능한 시스템 메뉴 바를 그리지 않으므로 무시함.
                "GetMenuItemInfoA" => {
                    let hmenu = uc.read_arg(0);
                    let item = uc.read_arg(1);
                    let f_by_pos = uc.read_arg(2);
                    let lpmii = uc.read_arg(3);
                    crate::emu_log!(
                        "[USER32] GetMenuItemInfoA({:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 0",
                        hmenu,
                        item,
                        f_by_pos,
                        lpmii
                    );
                    Some((4, Some(0)))
                }

                // API: BOOL DeleteMenu(HMENU hMenu, UINT uPosition, UINT uFlags)
                // 역할: 메뉴에서 항목을 삭제
                // 구현 생략 사유: 메뉴를 렌더링하지 않으므로 항목을 삭제할 필요 없음.
                "DeleteMenu" => {
                    let hmenu = uc.read_arg(0);
                    let u_position = uc.read_arg(1);
                    let u_flags = uc.read_arg(2);
                    crate::emu_log!(
                        "[USER32] DeleteMenu({:#x}, {:#x}, {:#x}) -> BOOL 1",
                        hmenu,
                        u_position,
                        u_flags
                    );
                    Some((3, Some(1)))
                }

                // API: BOOL RemoveMenu(HMENU hMenu, UINT uPosition, UINT uFlags)
                // 역할: 메뉴 항목을 제거 (파괴하지 않음)
                // 구현 생략 사유: 위와 동일.
                "RemoveMenu" => {
                    let hmenu = uc.read_arg(0);
                    let u_position = uc.read_arg(1);
                    let u_flags = uc.read_arg(2);
                    crate::emu_log!(
                        "[USER32] RemoveMenu({:#x}, {:#x}, {:#x}) -> BOOL 1",
                        hmenu,
                        u_position,
                        u_flags
                    );
                    Some((3, Some(1)))
                }
                // API: HMENU GetSystemMenu(HWND hWnd, BOOL bRevert)
                // 역할: 복사/수정용 시스템 메뉴 핸들을 가져옴
                "GetSystemMenu" => {
                    let hwnd = uc.read_arg(0);
                    let b_revert = uc.read_arg(1);
                    let handle = uc.get_data().alloc_handle();
                    crate::emu_log!(
                        "[USER32] GetSystemMenu({:#x}, {:#x}) -> HMENU {:#x}",
                        hwnd,
                        b_revert,
                        handle
                    );
                    Some((2, Some(handle as i32)))
                }

                // API: HMENU GetMenu(HWND hWnd)
                // 역할: 지정된 창의 메뉴 핸들을 가져옴
                "GetMenu" => {
                    let hwnd = uc.read_arg(0);
                    let handle = uc.get_data().alloc_handle();
                    crate::emu_log!("[USER32] GetMenu({:#x}) -> HMENU {:#x}", hwnd, handle);
                    Some((1, Some(handle as i32)))
                }

                // API: BOOL AppendMenuA(HMENU hMenu, UINT uFlags, UINT_PTR uIDNewItem, LPCSTR lpNewItem)
                // 역할: 메뉴 끝에 새 항목을 추가
                // 구현 생략 사유: 시스템 메뉴 확장을 요청하지만 렌더링하지 않으므로 No-op.
                "AppendMenuA" => {
                    let hmenu = uc.read_arg(0);
                    let u_flags = uc.read_arg(1);
                    let u_id_new_item = uc.read_arg(2);
                    let lp_new_item = uc.read_arg(3);
                    crate::emu_log!(
                        "[USER32] AppendMenuA({:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 1",
                        hmenu,
                        u_flags,
                        u_id_new_item,
                        lp_new_item
                    );
                    Some((4, Some(1)))
                }

                // API: HMENU CreateMenu(void)
                // 역할: 메뉴를 생성
                "CreateMenu" => {
                    let ctx = uc.get_data();
                    let hmenu = ctx.alloc_handle();
                    crate::emu_log!("[USER32] CreateMenu() -> HMENU {:#x}", hmenu);
                    Some((0, Some(hmenu as i32)))
                }

                // API: BOOL DestroyMenu(HMENU hMenu)
                // 역할: 메뉴를 파괴
                // 구현 생략 사유: 메뉴 객체를 시뮬레이션하지 않으므로 리소스 해제도 불필요함.
                "DestroyMenu" => {
                    let hmenu = uc.read_arg(0);
                    crate::emu_log!("[USER32] DestroyMenu({:#x}) -> BOOL 1", hmenu);
                    Some((1, Some(1)))
                }

                // API: LRESULT DefWindowProcA(HWND hWnd, UINT Msg, WPARAM wParam, LPARAM lParam)
                // 역할: 기본 창 프로시저를 호출하여 메시지를 처리
                // 구현 생략 사유: 운영체제 기본 창 프로시저 처리기. 에뮬레이터 내에서 창의 디폴트 액션을 수행할 인프라가 없으므로 무시함.
                "DefWindowProcA" => {
                    let hwnd = uc.read_arg(0);
                    let msg = uc.read_arg(1);
                    let w_param = uc.read_arg(2);
                    let l_param = uc.read_arg(3);
                    crate::emu_log!(
                        "[USER32] DefWindowProcA({:#x}, {:#x}, {:#x}, {:#x}) -> LRESULT 0",
                        hwnd,
                        msg,
                        w_param,
                        l_param
                    );
                    Some((4, Some(0)))
                }

                // API: LRESULT DefMDIChildProcA(HWND hWnd, UINT Msg, WPARAM wParam, LPARAM lParam)
                // 역할: MDI 자식 창의 기본 메시지 처리를 수행
                // 구현 생략 사유: MDI 자식 창은 사용되지 않음.
                "DefMDIChildProcA" => {
                    let hwnd = uc.read_arg(0);
                    let msg = uc.read_arg(1);
                    let w_param = uc.read_arg(2);
                    let l_param = uc.read_arg(3);
                    crate::emu_log!(
                        "[USER32] DefMDIChildProcA({:#x}, {:#x}, {:#x}, {:#x}) -> LRESULT 0",
                        hwnd,
                        msg,
                        w_param,
                        l_param
                    );
                    Some((4, Some(0)))
                }

                // API: LRESULT DefFrameProcA(HWND hWnd, HWND hWndMDIClient, UINT Msg, WPARAM wParam, LPARAM lParam)
                // 역할: MDI 프레임 창의 기본 메시지 처리를 수행
                // 구현 생략 사유: MDI 기반 프레임 창은 시뮬레이션하지 않음.
                "DefFrameProcA" => {
                    let hwnd = uc.read_arg(0);
                    let hwnd_mdi_client = uc.read_arg(1);
                    let msg = uc.read_arg(2);
                    let w_param = uc.read_arg(3);
                    let l_param = uc.read_arg(4);
                    crate::emu_log!(
                        "[USER32] DefFrameProcA({:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> LRESULT 0",
                        hwnd,
                        hwnd_mdi_client,
                        msg,
                        w_param,
                        l_param
                    );
                    Some((5, Some(0)))
                }

                // API: LONG SetWindowLongA(HWND hWnd, int nIndex, LONG dwNewLong)
                // 역할: 창의 특성을 변경
                "SetWindowLongA" => {
                    let hwnd = uc.read_arg(0);
                    let index = uc.read_arg(1) as i32;
                    let new_val = uc.read_arg(2);
                    let mut old_val = 0;

                    let ctx = uc.get_data();
                    let mut win_event = ctx.win_event.lock().unwrap();
                    if let Some(win) = win_event.windows.get_mut(&hwnd) {
                        old_val = match index {
                            -4 => std::mem::replace(&mut win.wnd_proc, new_val), // GWL_WNDPROC
                            -12 => std::mem::replace(&mut win.id, new_val),      // GWL_ID
                            -16 => std::mem::replace(&mut win.style, new_val),   // GWL_STYLE
                            -20 => std::mem::replace(&mut win.ex_style, new_val), // GWL_EXSTYLE
                            -21 => std::mem::replace(&mut win.user_data, new_val), // GWL_USERDATA
                            _ => 0,
                        };
                    }
                    crate::emu_log!(
                        "[USER32] SetWindowLongA({:#x}, {:#x}, {:#x}) -> LONG {:#x}",
                        hwnd,
                        index,
                        new_val,
                        old_val
                    );
                    Some((3, Some(old_val as i32)))
                }

                // API: LONG GetWindowLongA(HWND hWnd, int nIndex)
                // 역할: 창의 특성 정보를 가져옴
                "GetWindowLongA" => {
                    let hwnd = uc.read_arg(0);
                    let index = uc.read_arg(1) as i32;
                    let mut val = 0;

                    let ctx = uc.get_data();
                    let win_event = ctx.win_event.lock().unwrap();
                    if let Some(win) = win_event.windows.get(&hwnd) {
                        val = match index {
                            -4 => win.wnd_proc,
                            -12 => win.id,
                            -16 => win.style,
                            -20 => win.ex_style,
                            -21 => win.user_data,
                            _ => 0,
                        };
                    }
                    crate::emu_log!(
                        "[USER32] GetWindowLongA({:#x}, {:#x}) -> LONG {:#x}",
                        hwnd,
                        index,
                        val
                    );
                    Some((2, Some(val as i32)))
                }

                // API: LRESULT CallWindowProcA(WNDPROC lpPrevWndFunc, HWND hWnd, UINT Msg, WPARAM wParam, LPARAM lParam)
                // 역할: 지정된 창 프로시저로 메시지를 전달
                // 구현 생략 사유: 윈도우 프로시저 체이닝. 스택과 레지스터를 조작하여 콜백을 부르는 과정이 고도의 안정성을 요구하며 필수적이지 않아 무시함.
                "CallWindowProcA" => {
                    let lp_prev_wnd_func = uc.read_arg(0);
                    let hwnd = uc.read_arg(1);
                    let msg = uc.read_arg(2);
                    let w_param = uc.read_arg(3);
                    let l_param = uc.read_arg(4);
                    crate::emu_log!(
                        "[USER32] CallWindowProcA({:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> LRESULT 0",
                        lp_prev_wnd_func,
                        hwnd,
                        msg,
                        w_param,
                        l_param
                    );
                    Some((5, Some(0)))
                }

                // API: BOOL PostThreadMessageA(DWORD idThread, UINT Msg, WPARAM wParam, LPARAM lParam)
                // 역할: 특정 스레드의 메시지 큐에 메시지를 배치
                "PostThreadMessageA" => {
                    let thread_id = uc.read_arg(0);
                    let msg = uc.read_arg(1);
                    let wparam = uc.read_arg(2);
                    let lparam = uc.read_arg(3);
                    let time = uc.get_data().start_time.elapsed().as_millis() as u32;
                    let ctx = uc.get_data();
                    ctx.message_queue
                        .lock()
                        .unwrap()
                        .push_back([0, msg, wparam, lparam, time, 0, 0]);
                    crate::emu_log!(
                        "[USER32] PostThreadMessageA({:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 1",
                        thread_id,
                        msg,
                        wparam,
                        lparam
                    );
                    Some((4, Some(1)))
                }

                // API: HDC BeginPaint(HWND hWnd, LPPAINTSTRUCT lpPaint)
                // 역할: 그리기를 위해 창을 준비
                "BeginPaint" => {
                    let hwnd = uc.read_arg(0);
                    let ps_addr = uc.read_arg(1);
                    let ctx = uc.get_data();
                    let hdc = ctx.alloc_handle();
                    ctx.gdi_objects.lock().unwrap().insert(
                        hdc,
                        crate::win32::GdiObject::Dc {
                            associated_window: 0,
                            selected_bitmap: 0,
                            selected_font: 0,
                            selected_brush: 0,
                            selected_pen: 0,
                            selected_region: 0,
                            selected_palette: 0,
                            bk_mode: 0,
                            bk_color: 0,
                            text_color: 0,
                            rop2_mode: 0,
                            current_x: 0,
                            current_y: 0,
                        },
                    );
                    // PAINTSTRUCT: HDC at offset 0
                    uc.write_u32(ps_addr as u64, hdc);
                    crate::emu_log!(
                        "[USER32] BeginPaint({:#x}, {:#x}) -> HDC {:#x}",
                        hwnd,
                        ps_addr,
                        hdc
                    );
                    Some((2, Some(hdc as i32)))
                }

                // API: BOOL EndPaint(HWND hWnd, const PAINTSTRUCT* lpPaint)
                // 역할: 그리기가 완료되었음을 알림
                "EndPaint" => {
                    let hwnd = uc.read_arg(0);
                    let ps_addr = uc.read_arg(1);
                    let hdc = uc.read_u32(ps_addr as u64);
                    let ctx = uc.get_data();
                    ctx.gdi_objects.lock().unwrap().remove(&hdc);
                    crate::emu_log!("[USER32] EndPaint({:#x}, {:#x}) -> BOOL 1", hwnd, ps_addr);
                    Some((2, Some(1)))
                }

                // API: int ScrollWindowEx(HWND hWnd, int dx, int dy, const RECT* prcScroll, const RECT* prcClip, HRGN hrgnUpdate, LPRECT prcUpdate, UINT flags)
                // 역할: 창의 클라이언트 영역 내용을 스크롤
                // 구현 생략 사유: 클라이언트 영역 픽셀을 물리적으로 스크롤하는 함수. 게임은 자체 GDI/DirectX 그리기 루프를 쓰므로 무시함.
                "ScrollWindowEx" => {
                    let hwnd = uc.read_arg(0);
                    let dx = uc.read_arg(1);
                    let dy = uc.read_arg(2);
                    let prc_scroll = uc.read_arg(3);
                    let prc_clip = uc.read_arg(4);
                    let hrgn_update = uc.read_arg(5);
                    let prc_update = uc.read_arg(6);
                    let flags = uc.read_arg(7);
                    crate::emu_log!(
                        "[USER32] ScrollWindowEx({:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> int 0",
                        hwnd,
                        dx,
                        dy,
                        prc_scroll,
                        prc_clip,
                        hrgn_update,
                        prc_update,
                        flags
                    );
                    Some((8, Some(0)))
                }

                // API: BOOL InvalidateRect(HWND hWnd, const RECT* lpRect, BOOL bErase)
                // 역할: 창의 클라이언트 영역 중 일부를 갱신 대상으로 설정
                "InvalidateRect" => {
                    let hwnd = uc.read_arg(0);
                    let rect_addr = uc.read_arg(1);
                    let erase = uc.read_arg(2);
                    uc.get_data().win_event.lock().unwrap().update_window(hwnd);
                    crate::emu_log!(
                        "[USER32] InvalidateRect({:#x}, {:#x}, {:#x}) -> BOOL 1",
                        hwnd,
                        rect_addr,
                        erase
                    );
                    Some((3, Some(1)))
                }

                // API: int SetScrollInfo(HWND hWnd, int nBar, LPCSCROLLINFO lpsi, BOOL redraw)
                // 역할: 스크롤 바의 매개변수를 설정
                // 구현 생략 사유: 네이티브 스크롤바 컴포넌트는 사용하지 않음.
                "SetScrollInfo" => {
                    let hwnd = uc.read_arg(0);
                    let nbar = uc.read_arg(1);
                    let lpsi = uc.read_arg(2);
                    let redraw = uc.read_arg(3);
                    crate::emu_log!(
                        "[USER32] SetScrollInfo({:#x}, {:#x}, {:#x}, {:#x}) -> int 0",
                        hwnd,
                        nbar,
                        lpsi,
                        redraw
                    );
                    Some((4, Some(0)))
                }
                // API: BOOL SetWindowTextA(HWND hWnd, LPCSTR lpString)
                // 역할: 창의 제목 표시줄 텍스트 또는 컨트롤의 텍스트를 변경
                "SetWindowTextA" => {
                    let hwnd = uc.read_arg(0);
                    let text_addr = uc.read_arg(1);
                    let text = uc.read_euc_kr(text_addr as u64);
                    uc.get_data()
                        .win_event
                        .lock()
                        .unwrap()
                        .set_window_text(hwnd, text.clone());
                    crate::emu_log!(
                        "[USER32] SetWindowTextA({:#x}, \"{}\") -> BOOL 1",
                        hwnd,
                        text
                    );
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
                        let win_event = ctx.win_event.lock().unwrap();
                        win_event.windows.get(&hwnd).map(|win| {
                            let len = win.title.len().min(max_count as usize - 1);
                            (win.title[..len].to_string(), len)
                        })
                    };

                    if let Some((text, len)) = title_info {
                        uc.write_string(buf_addr as u64, &text);
                        crate::emu_log!(
                            "[USER32] GetWindowTextA({:#x}, {:#x}, {:#x}) -> int {}",
                            hwnd,
                            buf_addr,
                            max_count,
                            len
                        );
                        Some((3, Some(len as i32)))
                    } else {
                        crate::emu_log!(
                            "[USER32] GetWindowTextA({:#x}, {:#x}, {:#x}) -> int 0",
                            hwnd,
                            buf_addr,
                            max_count
                        );
                        Some((3, Some(0)))
                    }
                }

                // API: BOOL KillTimer(HWND hWnd, UINT_PTR uIDEvent)
                // 역할: 타이머를 중지
                "KillTimer" => {
                    let _hwnd = uc.read_arg(0);
                    let id = uc.read_arg(1);
                    let ctx = uc.get_data();
                    ctx.timers.lock().unwrap().remove(&id);
                    crate::emu_log!("[USER32] KillTimer({:#x}, {:#x}) -> BOOL 1", _hwnd, id);
                    Some((2, Some(1)))
                }

                // API: UINT_PTR SetTimer(HWND hWnd, UINT_PTR nIDEvent, UINT uElapse, TIMERPROC lpTimerFunc)
                // 역할: 타이머를 생성
                "SetTimer" => {
                    let hwnd = uc.read_arg(0);
                    let mut id = uc.read_arg(1);
                    let elapse = uc.read_arg(2);
                    let lp_timer_func = uc.read_arg(3);
                    let ctx = uc.get_data();
                    let mut timers = ctx.timers.lock().unwrap();
                    if id == 0 {
                        id = ctx.alloc_handle();
                    }
                    timers.insert(id, elapse);
                    crate::emu_log!(
                        "[USER32] SetTimer({:#x}, {:#x}, {:#x}, {:#x}) -> UINT_PTR {:#x}",
                        hwnd,
                        id,
                        elapse,
                        lp_timer_func,
                        id
                    );
                    Some((4, Some(id as i32)))
                }

                // API: int MapWindowPoints(HWND hWndFrom, HWND hWndTo, LPPOINT lpPoints, UINT cPoints)
                // 역할: 한 창의 상대 좌표를 다른 창의 상대 좌표로 변환
                // 구현 생략 사유: 창 좌표계 변환 함수이나 1개의 창만 띄우기 때문에 좌표계 변환의 실익이 없음.
                "MapWindowPoints" => {
                    let hwnd_from = uc.read_arg(0);
                    let hwnd_to = uc.read_arg(1);
                    let lp_points = uc.read_arg(2);
                    let c_points = uc.read_arg(3);
                    crate::emu_log!(
                        "[USER32] MapWindowPoints({:#x}, {:#x}, {:#x}, {:#x}) -> int 0",
                        hwnd_from,
                        hwnd_to,
                        lp_points,
                        c_points
                    );
                    Some((4, Some(0)))
                }
                // API: BOOL SystemParametersInfoA(UINT uiAction, UINT uiParam, PVOID pvParam, UINT fWinIni)
                // 역할: 시스템 전체의 매개변수를 가져오거나 설정
                // 구현 생략 사유: 제어판 설정 및 OS 환경 변수 조회 함수. 게임 진행을 막지 않도록 성공으로 위장함.
                "SystemParametersInfoA" => {
                    let ui_action = uc.read_arg(0);
                    let ui_param = uc.read_arg(1);
                    let pv_param = uc.read_arg(2);
                    let f_win_ini = uc.read_arg(3);
                    crate::emu_log!(
                        "[USER32] SystemParametersInfoA({:#x}, {:#x}, {:#x}, {:#x}) -> BOOL 1",
                        ui_action,
                        ui_param,
                        pv_param,
                        f_win_ini
                    );
                    Some((4, Some(1)))
                }
                // API: BOOL TranslateMDISysAccel(HWND hWndClient, LPMSG lpMsg)
                // 역할: MDI 자식 창의 바로 가기 키 메시지를 처리
                // 구현 생략 사유: MDI 단축키 처리. MDI 환경이 아니므로 무시.
                "TranslateMDISysAccel" => {
                    let hwnd_client = uc.read_arg(0);
                    let lp_msg = uc.read_arg(1);
                    crate::emu_log!(
                        "[USER32] TranslateMDISysAccel({:#x}, {:#x}) -> BOOL 0",
                        hwnd_client,
                        lp_msg
                    );
                    Some((2, Some(0)))
                }
                // API: int DrawTextA(HDC hDC, LPCSTR lpchText, int nCount, LPRECT lpRect, UINT uFormat)
                // 역할: 서식화된 텍스트를 사각형 내에 그림
                // 구현 생략 사유: 단순 텍스트 렌더링용 GDI 함수. 게임 그래픽은 보통 텍스처와 자체 본문을 쓰거나 BitBlt을 사용하므로 생략.
                "DrawTextA" => {
                    let hdc = uc.read_arg(0);
                    let lpch_text = uc.read_arg(1);
                    let n_count = uc.read_arg(2);
                    let lp_rect = uc.read_arg(3);
                    let u_format = uc.read_arg(4);
                    crate::emu_log!(
                        "[USER32] DrawTextA({:#x}, {:#x}, {:#x}, {:#x}, {:#x}) -> int 0",
                        hdc,
                        lpch_text,
                        n_count,
                        lp_rect,
                        u_format
                    );
                    Some((5, Some(0)))
                }
                // API: BOOL GetCursorPos(LPPOINT lpPoint)
                // 역할: 마우스 커서의 현재 위치를 화면 좌표로 가져옴
                "GetCursorPos" => {
                    let pt_addr = uc.read_arg(0);
                    uc.write_u32(pt_addr as u64, 320);
                    uc.write_u32(pt_addr as u64 + 4, 240);
                    crate::emu_log!("[USER32] GetCursorPos({:#x}) -> BOOL 1", pt_addr);
                    Some((1, Some(1)))
                }

                // API: BOOL PtInRect(const RECT* lprc, POINT pt)
                // 역할: 점이 사각형 내부에 있는지 확인
                "PtInRect" => {
                    let rect_addr = uc.read_arg(0);
                    // POINT is passed by value (8 bytes) -> takes 2 stack arguments
                    let pt_x = uc.read_arg(1) as i32;
                    let pt_y = uc.read_arg(2) as i32;

                    let left = uc.read_u32(rect_addr as u64) as i32;
                    let top = uc.read_u32(rect_addr as u64 + 4) as i32;
                    let right = uc.read_u32(rect_addr as u64 + 8) as i32;
                    let bottom = uc.read_u32(rect_addr as u64 + 12) as i32;

                    let inside = pt_x >= left && pt_x < right && pt_y >= top && pt_y < bottom;
                    crate::emu_log!(
                        "[USER32] PtInRect({:#x}, {{x:{}, y:{}}}) -> BOOL {}",
                        rect_addr,
                        pt_x,
                        pt_y,
                        if inside { 1 } else { 0 }
                    );
                    Some((3, Some(if inside { 1 } else { 0 })))
                }

                // API: BOOL SetRect(LPRECT lprc, int xLeft, int yTop, int xRight, int yBottom)
                // 역할: 사각형의 좌표를 설정
                "SetRect" => {
                    let rect_addr = uc.read_arg(0);
                    let left = uc.read_arg(1) as i32;
                    let top = uc.read_arg(2) as i32;
                    let right = uc.read_arg(3) as i32;
                    let bottom = uc.read_arg(4) as i32;
                    uc.write_u32(rect_addr as u64, left as u32);
                    uc.write_u32(rect_addr as u64 + 4, top as u32);
                    uc.write_u32(rect_addr as u64 + 8, right as u32);
                    uc.write_u32(rect_addr as u64 + 12, bottom as u32);
                    crate::emu_log!(
                        "[USER32] SetRect({:#x}, {}, {}, {}, {}) -> BOOL 1",
                        rect_addr,
                        left,
                        top,
                        right,
                        bottom
                    );
                    Some((5, Some(1)))
                }

                // API: BOOL EqualRect(const RECT* lprc1, const RECT* lprc2)
                // 역할: 두 사각형이 동일한지 확인
                "EqualRect" => {
                    let r1 = uc.read_arg(0);
                    let r2 = uc.read_arg(1);
                    let mut eq = true;
                    for i in 0..4 {
                        if uc.read_u32(r1 as u64 + i * 4) != uc.read_u32(r2 as u64 + i * 4) {
                            eq = false;
                            break;
                        }
                    }
                    crate::emu_log!(
                        "[USER32] EqualRect({:#x}, {:#x}) -> BOOL {}",
                        r1,
                        r2,
                        if eq { 1 } else { 0 }
                    );
                    Some((2, Some(if eq { 1 } else { 0 })))
                }

                // API: BOOL UnionRect(LPRECT lprcDst, const RECT* lprcSrc1, const RECT* lprcSrc2)
                // 역할: 두 사각형을 모두 포함하는 최소 사각형을 계산
                "UnionRect" => {
                    let dst = uc.read_arg(0);
                    let src1 = uc.read_arg(1);
                    let src2 = uc.read_arg(2);

                    let l1 = uc.read_u32(src1 as u64) as i32;
                    let t1 = uc.read_u32(src1 as u64 + 4) as i32;
                    let r1 = uc.read_u32(src1 as u64 + 8) as i32;
                    let b1 = uc.read_u32(src1 as u64 + 12) as i32;

                    let l2 = uc.read_u32(src2 as u64) as i32;
                    let t2 = uc.read_u32(src2 as u64 + 4) as i32;
                    let r2 = uc.read_u32(src2 as u64 + 8) as i32;
                    let b2 = uc.read_u32(src2 as u64 + 12) as i32;

                    let l = l1.min(l2);
                    let t = t1.min(t2);
                    let r = r1.max(r2);
                    let b = b1.max(b2);

                    uc.write_u32(dst as u64, l as u32);
                    uc.write_u32(dst as u64 + 4, t as u32);
                    uc.write_u32(dst as u64 + 8, r as u32);
                    uc.write_u32(dst as u64 + 12, b as u32);
                    crate::emu_log!(
                        "[USER32] UnionRect({:#x}, {:#x}, {:#x}) -> BOOL 1",
                        dst,
                        src1,
                        src2
                    );
                    Some((3, Some(1)))
                }

                // API: BOOL IntersectRect(LPRECT lprcDst, const RECT* lprcSrc1, const RECT* lprcSrc2)
                // 역할: 두 사각형의 교집합 사각형을 계산
                "IntersectRect" => {
                    let dst = uc.read_arg(0);
                    let src1 = uc.read_arg(1);
                    let src2 = uc.read_arg(2);

                    let l1 = uc.read_u32(src1 as u64) as i32;
                    let t1 = uc.read_u32(src1 as u64 + 4) as i32;
                    let r1 = uc.read_u32(src1 as u64 + 8) as i32;
                    let b1 = uc.read_u32(src1 as u64 + 12) as i32;

                    let l2 = uc.read_u32(src2 as u64) as i32;
                    let t2 = uc.read_u32(src2 as u64 + 4) as i32;
                    let r2 = uc.read_u32(src2 as u64 + 8) as i32;
                    let b2 = uc.read_u32(src2 as u64 + 12) as i32;

                    let l = l1.max(l2);
                    let t = t1.max(t2);
                    let r = r1.min(r2);
                    let b = b1.min(b2);

                    if l < r && t < b {
                        uc.write_u32(dst as u64, l as u32);
                        uc.write_u32(dst as u64 + 4, t as u32);
                        uc.write_u32(dst as u64 + 8, r as u32);
                        uc.write_u32(dst as u64 + 12, b as u32);
                        crate::emu_log!(
                            "[USER32] IntersectRect({:#x}, {:#x}, {:#x}) -> BOOL 1",
                            dst,
                            src1,
                            src2
                        );
                        Some((3, Some(1)))
                    } else {
                        uc.write_u32(dst as u64, 0);
                        uc.write_u32(dst as u64 + 4, 0);
                        uc.write_u32(dst as u64 + 8, 0);
                        uc.write_u32(dst as u64 + 12, 0);
                        crate::emu_log!(
                            "[USER32] IntersectRect({:#x}, {:#x}, {:#x}) -> BOOL 0",
                            dst,
                            src1,
                            src2
                        );
                        Some((3, Some(0)))
                    }
                }

                // API: HANDLE GetClipboardData(UINT uFormat)
                // 역할: 클립보드에서 데이터를 가져옴
                "GetClipboardData" => {
                    let format = uc.read_arg(0);
                    if format == 1 {
                        // CF_TEXT
                        let (ptr, data) = {
                            let ctx = uc.get_data();
                            let cb = ctx.clipboard_data.lock().unwrap();
                            if cb.is_empty() {
                                (0, Vec::new())
                            } else {
                                let ptr = ctx.heap_cursor.fetch_add(
                                    cb.len() as u32 + 1,
                                    std::sync::atomic::Ordering::SeqCst,
                                );
                                (ptr, cb.clone())
                            }
                        };
                        if ptr == 0 {
                            crate::emu_log!("[USER32] GetClipboardData({:#x}) -> int 0", format);
                            Some((1, Some(0)))
                        } else {
                            uc.mem_write(ptr as u64, &data).unwrap();
                            uc.mem_write(ptr as u64 + data.len() as u64, &[0]).unwrap();
                            crate::emu_log!(
                                "[USER32] GetClipboardData({:#x}) -> int {:#x}",
                                format,
                                ptr
                            );
                            Some((1, Some(ptr as i32)))
                        }
                    } else {
                        crate::emu_log!("[USER32] GetClipboardData({:#x}) -> int 0", format);
                        Some((1, Some(0)))
                    }
                }
                // API: BOOL OpenClipboard(HWND hWndNewOwner)
                // 역할: 다른 창이 클립보드 내용을 수정하지 못하도록 클립보드를 엶
                "OpenClipboard" => {
                    let hwnd = uc.read_arg(0);
                    let ctx = uc.get_data();
                    let opened = ctx
                        .clipboard_open
                        .swap(1, std::sync::atomic::Ordering::SeqCst);
                    crate::emu_log!(
                        "[USER32] OpenClipboard({:#x}) -> BOOL {}",
                        hwnd,
                        if opened == 0 { 1 } else { 0 }
                    );
                    Some((1, Some(if opened == 0 { 1 } else { 0 })))
                }
                // API: BOOL CloseClipboard(void)
                // 역할: 클립보드를 닫음
                "CloseClipboard" => {
                    let ctx = uc.get_data();
                    ctx.clipboard_open
                        .store(0, std::sync::atomic::Ordering::SeqCst);
                    crate::emu_log!("[USER32] CloseClipboard() -> BOOL 1");
                    Some((0, Some(1)))
                }
                // API: BOOL EmptyClipboard(void)
                // 역할: 클립보드 내용을 비우고 메모리 소유권을 가져옴
                "EmptyClipboard" => {
                    let ctx = uc.get_data();
                    ctx.clipboard_data.lock().unwrap().clear();
                    crate::emu_log!("[USER32] EmptyClipboard() -> BOOL 1");
                    Some((0, Some(1)))
                }
                // API: HANDLE SetClipboardData(UINT uFormat, HANDLE hMem)
                // 역할: 특정 포맷으로 데이터를 클립보드에 배치
                "SetClipboardData" => {
                    let format = uc.read_arg(0);
                    let hmem = uc.read_arg(1);
                    if format == 1 && hmem != 0 {
                        let mut buf = Vec::new();
                        let mut curr = hmem as u64;
                        loop {
                            let mut tmp = [0u8; 1];
                            uc.mem_read(curr, &mut tmp).unwrap();
                            let b = tmp[0];
                            if b == 0 {
                                break;
                            }
                            buf.push(b);
                            curr += 1;
                        }
                        let ctx = uc.get_data();
                        *ctx.clipboard_data.lock().unwrap() = buf;
                        crate::emu_log!(
                            "[USER32] SetClipboardData({:#x}, {:#x}) -> HANDLE {:#x}",
                            format,
                            hmem,
                            hmem
                        );
                        Some((2, Some(hmem as i32)))
                    } else {
                        crate::emu_log!(
                            "[USER32] SetClipboardData({:#x}, {:#x}) -> HANDLE 0",
                            format,
                            hmem
                        );
                        Some((2, Some(0)))
                    }
                }

                // API: BOOL IsClipboardFormatAvailable(UINT format)
                // 역할: 클립보드에 특정 포맷의 데이터가 있는지 확인
                "IsClipboardFormatAvailable" => {
                    let format = uc.read_arg(0);
                    let available = if format == 1 {
                        let ctx = uc.get_data();
                        if ctx.clipboard_data.lock().unwrap().is_empty() {
                            0
                        } else {
                            1
                        }
                    } else {
                        0
                    };
                    crate::emu_log!(
                        "[USER32] IsClipboardFormatAvailable({:#x}) -> BOOL {}",
                        format,
                        available
                    );
                    Some((1, Some(available)))
                }

                // API: HWND SetCapture(HWND hWnd)
                // 역할: 마우스 캡처를 특정 창으로 설정
                "SetCapture" => {
                    let hwnd = uc.read_arg(0);
                    let ctx = uc.get_data();
                    let old = ctx
                        .capture_hwnd
                        .swap(hwnd, std::sync::atomic::Ordering::SeqCst);
                    crate::emu_log!("[USER32] SetCapture({:#x}) -> HWND {:#x}", hwnd, old);
                    Some((1, Some(old as i32)))
                }

                // API: HWND GetCapture(void)
                // 역할: 마우스 캡처가 있는 창의 핸들을 가져옴
                "GetCapture" => {
                    let ctx = uc.get_data();
                    let hwnd = ctx.capture_hwnd.load(std::sync::atomic::Ordering::SeqCst);
                    crate::emu_log!("[USER32] GetCapture() -> HWND {:#x}", hwnd);
                    Some((0, Some(hwnd as i32)))
                }

                // API: BOOL ReleaseCapture(void)
                // 역할: 마우스 캡처를 해제
                "ReleaseCapture" => {
                    let ctx = uc.get_data();
                    ctx.capture_hwnd
                        .store(0, std::sync::atomic::Ordering::SeqCst);
                    crate::emu_log!("[USER32] ReleaseCapture() -> BOOL 1");
                    Some((0, Some(1)))
                }

                // API: BOOL ScreenToClient(HWND hWnd, LPPOINT lpPoint)
                // 역할: 화면 좌표를 특정 창의 클라이언트 영역 좌표로 변환
                "ScreenToClient" => {
                    let hwnd = uc.read_arg(0);
                    let pt_addr = uc.read_arg(1);

                    let (win_x, win_y) = {
                        let ctx = uc.get_data();
                        let win_event = ctx.win_event.lock().unwrap();
                        win_event
                            .windows
                            .get(&hwnd)
                            .map(|w| (w.x, w.y))
                            .unwrap_or((0, 0))
                    };

                    let x = uc.read_u32(pt_addr as u64) as i32;
                    let y = uc.read_u32(pt_addr as u64 + 4) as i32;
                    uc.write_u32(pt_addr as u64, (x - win_x) as u32);
                    uc.write_u32(pt_addr as u64 + 4, (y - win_y) as u32);
                    crate::emu_log!(
                        "[USER32] ScreenToClient({:#x}, {{x:{}, y:{}}}) -> BOOL 1",
                        hwnd,
                        x - win_x,
                        y - win_y
                    );
                    Some((2, Some(1)))
                }

                // API: BOOL ClientToScreen(HWND hWnd, LPPOINT lpPoint)
                // 역할: 클라이언트 영역 좌표를 화면 좌표로 변환
                "ClientToScreen" => {
                    let hwnd = uc.read_arg(0);
                    let pt_addr = uc.read_arg(1);

                    let (win_x, win_y) = {
                        let ctx = uc.get_data();
                        let win_event = ctx.win_event.lock().unwrap();
                        win_event
                            .windows
                            .get(&hwnd)
                            .map(|w| (w.x, w.y))
                            .unwrap_or((0, 0))
                    };

                    let x = uc.read_u32(pt_addr as u64) as i32;
                    let y = uc.read_u32(pt_addr as u64 + 4) as i32;
                    uc.write_u32(pt_addr as u64, (x + win_x) as u32);
                    uc.write_u32(pt_addr as u64 + 4, (y + win_y) as u32);
                    crate::emu_log!(
                        "[USER32] ClientToScreen({:#x}, {{x:{}, y:{}}}) -> BOOL 1",
                        hwnd,
                        x + win_x,
                        y + win_y
                    );
                    Some((2, Some(1)))
                }

                // API: BOOL CreateCaret(HWND hWnd, HBITMAP hBitmap, int nWidth, int nHeight)
                // 역할: 시스템 캐럿을 생성
                // 구현 생략 사유: 텍스트 입력 커서(캐럿) 시각 효과는 시스템에 위임하거나 무시함.
                "CreateCaret" => {
                    let hwnd = uc.read_arg(0);
                    let hbitmap = uc.read_arg(1);
                    let n_width = uc.read_arg(2);
                    let n_height = uc.read_arg(3);
                    crate::emu_log!(
                        "[USER32] CreateCaret({:#x}, {:#x}, {}, {}) -> BOOL 1",
                        hwnd,
                        hbitmap,
                        n_width,
                        n_height
                    );
                    Some((4, Some(1)))
                }

                // API: BOOL DestroyCaret(void)
                // 역할: 현재 캐럿을 파괴
                // 구현 생략 사유: 텍스트 입력 캐럿 파괴. 무시함.
                "DestroyCaret" => {
                    crate::emu_log!("[USER32] DestroyCaret() -> BOOL 1");
                    Some((0, Some(1)))
                }

                // API: BOOL ShowCaret(HWND hWnd)
                // 역할: 캐럿을 화면에 표시
                // 구현 생략 사유: 캐럿 노출 조작 불가. 무시함.
                "ShowCaret" => {
                    let hwnd = uc.read_arg(0);
                    crate::emu_log!("[USER32] ShowCaret({:#x}) -> BOOL 1", hwnd);
                    Some((1, Some(1)))
                }

                // API: BOOL HideCaret(HWND hWnd)
                // 역할: 화면에서 캐럿을 숨김
                // 구현 생략 사유: 시각적 캐럿 조작. 생략.
                "HideCaret" => {
                    let hwnd = uc.read_arg(0);
                    crate::emu_log!("[USER32] HideCaret({:#x}) -> BOOL 1", hwnd);
                    Some((1, Some(1)))
                }

                // API: BOOL SetCaretPos(int X, int Y)
                // 역할: 캐럿의 위치를 이동
                // 구현 생략 사유: 캐럿 좌표 기반 포커싱. 생략.
                "SetCaretPos" => {
                    let x = uc.read_arg(0);
                    let y = uc.read_arg(1);
                    crate::emu_log!("[USER32] SetCaretPos({:#x}, {:#x}) -> BOOL 1", x, y);
                    Some((2, Some(1)))
                }

                // API: SHORT GetAsyncKeyState(int vKey)
                // 역할: 특정 키의 상태를 확인 (비동기식)
                "GetAsyncKeyState" => {
                    let vkey = uc.read_arg(0) as usize;
                    let ctx = uc.get_data();
                    let ks = ctx.key_states.lock().unwrap();
                    let mut state: i32 = 0;
                    if vkey < 256 && ks[vkey] {
                        state = -32768; // 0x8000
                    }
                    crate::emu_log!(
                        "[USER32] GetAsyncKeyState({:#x}) -> SHORT {:#x}",
                        vkey,
                        state
                    );
                    Some((1, Some(state)))
                }

                // API: SHORT GetKeyState(int nVirtKey)
                // 역할: 특정 키의 상태를 확인 (메시지 큐 기반)
                "GetKeyState" => {
                    let vkey = uc.read_arg(0) as usize;
                    let ctx = uc.get_data();
                    let ks = ctx.key_states.lock().unwrap();
                    let mut state: i32 = 0;
                    if vkey < 256 && ks[vkey] {
                        state = -32768; // 0x8000
                    }
                    crate::emu_log!("[USER32] GetKeyState({:#x}) -> SHORT {:#x}", vkey, state);
                    Some((1, Some(state)))
                }

                // API: DWORD GetSysColor(int nIndex)
                // 역할: 시스템 요소의 색상 값을 가져옴
                "GetSysColor" => {
                    let index = uc.read_arg(0);
                    let color = match index {
                        5 => 0x00FFFFFF,  // COLOR_WINDOW
                        8 => 0x00000000,  // COLOR_WINDOWTEXT
                        15 => 0x00C0C0C0, // COLOR_BTNFACE
                        _ => 0x00808080,
                    };
                    crate::emu_log!("[USER32] GetSysColor({:#x}) -> COLOR {:#x}", index, color);
                    Some((1, Some(color as i32)))
                }

                // API: int SetWindowRgn(HWND hWnd, HRGN hRgn, BOOL bRedraw)
                // 역할: 창의 영역을 설정
                // 구현 생략 사유: 창의 외곽선을 다각형으로 깎는 함수(비정형 윈도우). 에뮬레이션 불필요.
                "SetWindowRgn" => {
                    let hwnd = uc.read_arg(0);
                    let hrgn = uc.read_arg(1);
                    let b_redraw = uc.read_arg(2);
                    crate::emu_log!(
                        "[USER32] SetWindowRgn({:#x}, {:#x}, {}) -> INT 1",
                        hwnd,
                        hrgn,
                        b_redraw
                    );
                    Some((3, Some(1)))
                }

                // API: BOOL GetClassInfoExA(HINSTANCE hinst, LPCSTR lpszClass, PWNDCLASSEXA lpwcx)
                // 역할: 창 클래스에 대한 정보를 가져옴
                "GetClassInfoExA" => {
                    let _hinst = uc.read_arg(0);
                    let class_name_ptr = uc.read_arg(1);
                    let class_name = if class_name_ptr < 0x10000 {
                        format!("Atom_{}", class_name_ptr)
                    } else {
                        uc.read_euc_kr(class_name_ptr as u64)
                    };
                    let wcx_addr = uc.read_arg(2); // PWNDCLASSEXA (48 bytes)

                    let wnd_proc = {
                        let ctx = uc.get_data();
                        let classes = ctx.window_classes.lock().unwrap();
                        classes.get(&class_name).map(|wc| wc.wnd_proc)
                    };

                    if let Some(proc) = wnd_proc {
                        uc.write_u32(wcx_addr as u64 + 8, proc);
                        crate::emu_log!(
                            "[USER32] GetClassInfoExA(\"{}\", {:#x}) -> BOOL 1",
                            class_name,
                            wcx_addr
                        );
                        Some((3, Some(1)))
                    } else {
                        crate::emu_log!(
                            "[USER32] GetClassInfoExA(\"{}\", {:#x}) -> BOOL 0",
                            class_name,
                            wcx_addr
                        );
                        Some((3, Some(0)))
                    }
                }

                // API: BOOL IsZoomed(HWND hWnd)
                // 역할: 창이 최대화되어 있는지 확인
                "IsZoomed" => {
                    let hwnd = uc.read_arg(0);
                    let ctx = uc.get_data();
                    let win_event = ctx.win_event.lock().unwrap();
                    let zoomed = win_event
                        .windows
                        .get(&hwnd)
                        .map(|w| w.zoomed)
                        .unwrap_or(false);
                    crate::emu_log!(
                        "[USER32] IsZoomed({:#x}) -> BOOL {}",
                        hwnd,
                        if zoomed { 1 } else { 0 }
                    );
                    Some((1, Some(if zoomed { 1 } else { 0 })))
                }

                // API: BOOL IsIconic(HWND hWnd)
                // 역할: 창이 최소화(아이콘화)되어 있는지 확인
                "IsIconic" => {
                    let hwnd = uc.read_arg(0);
                    let ctx = uc.get_data();
                    let win_event = ctx.win_event.lock().unwrap();
                    let iconic = win_event
                        .windows
                        .get(&hwnd)
                        .map(|w| w.iconic)
                        .unwrap_or(false);
                    crate::emu_log!(
                        "[USER32] IsIconic({:#x}) -> BOOL {}",
                        hwnd,
                        if iconic { 1 } else { 0 }
                    );
                    Some((1, Some(if iconic { 1 } else { 0 })))
                }

                // API: int wsprintfA(LPSTR lpOut, LPCSTR lpFmt, ...)
                // 역할: 서식화된 데이터를 문자열로 출력 (가변 인자)
                "wsprintfA" => {
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
                                            padding =
                                                padding * 10 + format_char.to_digit(10).unwrap();
                                            chars.next();
                                            format_char = *chars.peek().unwrap_or(&' ');
                                        }
                                        if format_char == 'l' || format_char == 'h' {
                                            chars.next();
                                            format_char = *chars.peek().unwrap_or(&' ');
                                        }

                                        chars.next(); // consume format_char

                                        let arg_val = uc.read_arg(arg_idx);
                                        arg_idx += 1;

                                        let mut s = match format_char {
                                            'd' | 'i' => format!("{}", arg_val as i32),
                                            'u' => format!("{}", arg_val as u32),
                                            'x' => format!("{:x}", arg_val as u32),
                                            'X' => format!("{:X}", arg_val as u32),
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
                                            let pad_string = pad_char.to_string().repeat(pad);
                                            s = pad_string + &s;
                                        }
                                        formatted.push_str(&s);
                                    }
                                }
                            } else {
                                formatted.push(c);
                            }
                        }

                        uc.write_string(buf_addr as u64, &formatted);
                        crate::emu_log!("[USER32] wsprintfA(\"{}\") -> \"{}\"", fmt, formatted);
                        Some((arg_idx, Some(formatted.len() as i32)))
                    } else {
                        Some((2, Some(0)))
                    }
                }

                // API: BOOL EndDialog(HWND hDlg, INT_PTR nResult)
                // 역할: 대화 상자를 종료
                // 구현 생략 사유: 네이티브 대화상자 종료. 에뮬레이터에서는 모달 대화상자 루프가 불가하므로 무시.
                "EndDialog" => {
                    let h_dlg = uc.read_arg(0);
                    let n_result = uc.read_arg(1);
                    crate::emu_log!(
                        "[USER32] EndDialog({:#x}, {:#x}) -> BOOL 1",
                        h_dlg,
                        n_result
                    );
                    Some((2, Some(1)))
                }
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
