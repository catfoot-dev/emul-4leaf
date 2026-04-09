use crate::ui::gdi_renderer::GdiRenderer;
use crate::{
    dll::win32::{ApiHookResult, GdiObject, Timer, Win32Context, gdi32::GDI32},
    helper::UnicornHelper,
};
use unicorn_engine::Unicorn;

// API: HDC BeginPaint(HWND hWnd, LPPAINTSTRUCT lpPaint)
// 역할: 그리기를 준비하고 PAINTSTRUCT를 채움. WM_PAINT 처리 시 사용됨.
pub(super) fn begin_paint(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let lp_paint = uc.read_arg(1);

    let (width, height, surface_bitmap, hdc) = {
        let ctx = uc.get_data();
        let mut win_event = ctx.win_event.lock().unwrap();
        if let Some(state) = win_event.windows.get_mut(&hwnd) {
            state.needs_paint = false; // 그리기 시작했으므로 무효 영역 해제
            (
                state.width as u32,
                state.height as u32,
                state.surface_bitmap,
                ctx.alloc_handle(),
            )
        } else {
            return Some(ApiHookResult::callee(2, Some(0)));
        }
    };

    // WM_PAINT 경로에서도 일반 GetDC와 동일하게 창 표면에 연결된 DC를 제공합니다.
    uc.get_data().gdi_objects.lock().unwrap().insert(
        hdc,
        GdiObject::Dc {
            associated_window: hwnd,
            width: width as i32,
            height: height as i32,
            selected_bitmap: surface_bitmap,
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

    // PAINTSTRUCT 채우기
    uc.write_u32(lp_paint as u64 + 0, hdc); // hdc
    uc.write_u32(lp_paint as u64 + 4, 0); // fErase
    uc.write_u32(lp_paint as u64 + 8, 0); // rcPaint.left
    uc.write_u32(lp_paint as u64 + 12, 0); // rcPaint.top
    uc.write_u32(lp_paint as u64 + 16, width); // rcPaint.right
    uc.write_u32(lp_paint as u64 + 20, height); // rcPaint.bottom

    crate::emu_log!("[USER32] BeginPaint({:#x}) -> HDC {:#x}", hwnd, hdc);
    Some(ApiHookResult::callee(2, Some(hdc as i32)))
}

// API: BOOL EndPaint(HWND hWnd, const PAINTSTRUCT *lpPaint)
// 역할: 그리기를 종료함
pub(super) fn end_paint(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let lp_paint = uc.read_arg(1);
    let hdc = uc.read_u32(lp_paint as u64);
    let ctx = uc.get_data();
    ctx.gdi_objects.lock().unwrap().remove(&hdc);
    ctx.win_event.lock().unwrap().update_window(hwnd);
    crate::emu_log!("[USER32] EndPaint({:#x}) -> 1", hwnd);
    Some(ApiHookResult::callee(2, Some(1)))
}

// API: BOOL InvalidateRect(HWND hWnd, const RECT *lpRect, BOOL bErase)
// 역할: 창의 특정 영역을 무효화하여 WM_PAINT가 발생하도록 함
pub(super) fn invalidate_rect(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let ctx = uc.get_data();
    let mut win_event = ctx.win_event.lock().unwrap();
    if win_event.windows.contains_key(&hwnd) {
        win_event.invalidate_rect(hwnd, std::ptr::null_mut());
        crate::emu_log!("[USER32] InvalidateRect({:#x}) -> 1", hwnd);
        Some(ApiHookResult::callee(3, Some(1)))
    } else {
        Some(ApiHookResult::callee(3, Some(0)))
    }
}

// API: BOOL ValidateRect(HWND hWnd, const RECT *lpRect)
// 역할: 창의 특정 영역을 유효화함
pub(super) fn validate_rect(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let ctx = uc.get_data();
    let mut win_event = ctx.win_event.lock().unwrap();
    if let Some(state) = win_event.windows.get_mut(&hwnd) {
        state.needs_paint = false;
        crate::emu_log!("[USER32] ValidateRect({:#x}) -> 1", hwnd);
        Some(ApiHookResult::callee(2, Some(1)))
    } else {
        Some(ApiHookResult::callee(2, Some(0)))
    }
}

// API: int ScrollWindowEx(HWND hWnd, int dx, int dy, const RECT* prcScroll, const RECT* prcClip, HRGN hrgnUpdate, LPRECT prcUpdate, UINT flags)
// 역할: 창의 클라이언트 영역 내용을 스크롤
// 구현 생략 사유: 클라이언트 영역 픽셀을 물리적으로 스크롤하는 보조 함수. 게임은 자체 루프나 BitBlt을 사용하므로 생략함.
pub(super) fn scroll_window_ex(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let _dx = uc.read_arg(1) as i32;
    let _dy = uc.read_arg(2) as i32;
    let _prc_scroll = uc.read_arg(3);
    let _prc_clip = uc.read_arg(4);
    let _hrgn_update = uc.read_arg(5);
    let prc_update = uc.read_arg(6);
    let flags = uc.read_arg(7);

    // `SW_INVALIDATE` / `SW_ERASE`가 요청되면 최소한 다시 그리기 요청은 남깁니다.
    if flags & 0x0002 != 0 || flags & 0x0004 != 0 {
        uc.get_data()
            .win_event
            .lock()
            .unwrap()
            .invalidate_rect(hwnd, std::ptr::null_mut());
    }

    if prc_update != 0 {
        uc.write_u32(prc_update as u64, 0);
        uc.write_u32(prc_update as u64 + 4, 0);
        uc.write_u32(prc_update as u64 + 8, 0);
        uc.write_u32(prc_update as u64 + 12, 0);
    }

    crate::emu_log!(
        "[USER32] ScrollWindowEx({:#x}, flags={:#x}) -> NULLREGION",
        hwnd,
        flags
    );
    Some(ApiHookResult::callee(8, Some(1)))
}

// API: int SetScrollInfo(HWND hWnd, int nBar, LPCSCROLLINFO lpsi, BOOL redraw)
// 역할: 스크롤 바의 매개변수를 설정
// 구현 생략 사유: 네이티브 스크롤바 컴포넌트는 사용하지 않음.
pub(super) fn set_scroll_info(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    crate::emu_log!("[USER32] SetScrollInfo({:#x}) stubbed", hwnd);
    Some(ApiHookResult::callee(4, Some(0)))
}

// API: int DrawTextA(HDC hDC, LPCSTR lpchText, int nCount, LPRECT lpRect, UINT uFormat)
// 역할: 서식화된 텍스트를 사각형 내에 그림
pub(super) fn draw_text_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    const DT_CENTER: u32 = 0x0001;
    const DT_RIGHT: u32 = 0x0002;
    const DT_VCENTER: u32 = 0x0004;
    const DT_BOTTOM: u32 = 0x0008;
    const DT_WORDBREAK: u32 = 0x0010;
    const DT_SINGLELINE: u32 = 0x0020;
    const DT_CALCRECT: u32 = 0x0400;

    let hdc = uc.read_arg(0);
    let lpch_text = uc.read_arg(1);
    let n_count = uc.read_arg(2);
    let lp_rect = uc.read_arg(3);
    let u_format = uc.read_arg(4);

    let raw_text = if n_count == 0xffffffff {
        uc.read_euc_kr(lpch_text as u64)
    } else {
        uc.read_euc_kr(lpch_text as u64)
            .chars()
            .take(n_count as usize)
            .collect::<String>()
    };

    if lp_rect == 0 {
        crate::emu_log!(
            "[USER32] DrawTextA({:#x}, \"{}\", {}, {:#x}, {:#x}) -> int 0",
            hdc,
            raw_text,
            n_count,
            lp_rect,
            u_format
        );
        return Some(ApiHookResult::callee(5, Some(0)));
    }

    let left = uc.read_u32(lp_rect as u64) as i32;
    let top = uc.read_u32(lp_rect as u64 + 4) as i32;
    let right = uc.read_u32(lp_rect as u64 + 8) as i32;
    let bottom = uc.read_u32(lp_rect as u64 + 12) as i32;
    let rect_width = (right - left).max(0);
    let rect_height = (bottom - top).max(0);

    let draw_params = {
        let gdi_objects = uc.get_data().gdi_objects.lock().unwrap();
        if let Some(GdiObject::Dc {
            selected_bitmap,
            selected_font,
            text_color,
            bk_color,
            bk_mode,
            associated_window,
            ..
        }) = gdi_objects.get(&hdc)
        {
            let font_height =
                if let Some(GdiObject::Font { height, .. }) = gdi_objects.get(selected_font) {
                    *height
                } else {
                    12
                };
            Some((
                *selected_bitmap,
                *text_color,
                *bk_color,
                *bk_mode,
                *associated_window,
                font_height,
            ))
        } else {
            None
        }
    };

    let Some((hbmp, text_color, bk_color, bk_mode, hwnd, font_height)) = draw_params else {
        crate::emu_log!(
            "[USER32] DrawTextA({:#x}, \"{}\", {}, {:#x}, {:#x}) -> int 0",
            hdc,
            raw_text,
            n_count,
            lp_rect,
            u_format
        );
        return Some(ApiHookResult::callee(5, Some(0)));
    };

    let font_size = font_height.abs().max(1) as f32;
    let (line_height, _, _) = GdiRenderer::font_metrics(font_size);
    let line_height = line_height.max(1);
    let normalized_text = raw_text.replace("\r\n", "\n").replace('\r', "\n");
    let single_line = (u_format & DT_SINGLELINE) != 0;
    let max_line_width = if rect_width > 0 { rect_width } else { i32::MAX };

    // `DT_WORDBREAK`가 들어온 경우만 간단한 폭 기준 줄바꿈을 적용하고,
    // 그 외에는 게스트가 만든 개행을 그대로 존중합니다.
    let lines = if single_line {
        vec![normalized_text.replace('\n', " ")]
    } else {
        let mut lines = Vec::new();
        for paragraph in normalized_text.split('\n') {
            if paragraph.is_empty() {
                lines.push(String::new());
                continue;
            }

            if (u_format & DT_WORDBREAK) == 0 || max_line_width == i32::MAX {
                lines.push(paragraph.to_string());
                continue;
            }

            let mut current = String::new();
            for word in paragraph.split_whitespace() {
                let candidate = if current.is_empty() {
                    word.to_string()
                } else {
                    format!("{} {}", current, word)
                };
                if current.is_empty()
                    || GdiRenderer::measure_text_width(&candidate, font_size) <= max_line_width
                {
                    current = candidate;
                } else {
                    lines.push(current);
                    current = word.to_string();
                }
            }

            if current.is_empty() {
                lines.push(paragraph.to_string());
            } else {
                lines.push(current);
            }
        }

        if lines.is_empty() {
            vec![String::new()]
        } else {
            lines
        }
    };

    let measured_width = lines
        .iter()
        .map(|line| GdiRenderer::measure_text_width(line, font_size))
        .max()
        .unwrap_or(0);
    let measured_height = line_height * lines.len().max(1) as i32;

    if (u_format & DT_CALCRECT) != 0 {
        uc.write_u32(
            lp_rect as u64 + 8,
            left.saturating_add(measured_width) as u32,
        );
        uc.write_u32(
            lp_rect as u64 + 12,
            top.saturating_add(measured_height) as u32,
        );
    } else if hbmp != 0 {
        GDI32::sync_dib_pixels(uc, hbmp);
        let clip_rects = GDI32::clip_rects_for_hdc(uc, hdc);
        let gdi_objects = uc.get_data().gdi_objects.lock().unwrap();
        if let Some(GdiObject::Bitmap {
            width,
            height,
            pixels,
            ..
        }) = gdi_objects.get(&hbmp)
        {
            let width = *width;
            let height = *height;
            let default_clip = vec![(0, 0, width as i32, height as i32)];
            let clip_rects = clip_rects.unwrap_or(default_clip);
            let mut pixels = pixels.lock().unwrap();
            let block_y = if (u_format & DT_VCENTER) != 0 {
                top + (rect_height - measured_height).max(0) / 2
            } else if (u_format & DT_BOTTOM) != 0 {
                bottom - measured_height
            } else {
                top
            };

            for (index, line) in lines.iter().enumerate() {
                let line_width = GdiRenderer::measure_text_width(line, font_size);
                let draw_x = if (u_format & DT_RIGHT) != 0 {
                    right - line_width
                } else if (u_format & DT_CENTER) != 0 {
                    left + (rect_width - line_width).max(0) / 2
                } else {
                    left
                };
                let draw_y = block_y + index as i32 * line_height;

                GdiRenderer::draw_text_clipped(
                    &mut pixels,
                    width,
                    height,
                    draw_x,
                    draw_y,
                    line,
                    font_size,
                    text_color,
                    if bk_mode == 2 { Some(bk_color) } else { None },
                    &clip_rects,
                );
            }

            drop(pixels);
            drop(gdi_objects);
            GDI32::flush_dib_pixels_to_memory(uc, hbmp);
            if hwnd != 0 {
                uc.get_data().win_event.lock().unwrap().update_window(hwnd);
            }
        }
    }

    crate::emu_log!(
        "[USER32] DrawTextA({:#x}, \"{}\", {}, {:#x}, {:#x}) -> int {}",
        hdc,
        raw_text,
        n_count,
        lp_rect,
        u_format,
        measured_height
    );
    Some(ApiHookResult::callee(5, Some(measured_height)))
}

// API: int FillRect(HDC hDC, const RECT *lprc, HBRUSH hbr)
// 역할: 지정된 브러시를 사용하여 직사각형 영역을 채움
pub(super) fn fill_rect(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hdc = uc.read_arg(0);
    let lprc = uc.read_arg(1);
    let hbr = uc.read_arg(2);

    if lprc == 0 || hbr == 0 {
        return Some(ApiHookResult::callee(3, Some(0)));
    }

    let left = uc.read_i32(lprc as u64);
    let top = uc.read_i32(lprc as u64 + 4);
    let right = uc.read_i32(lprc as u64 + 8);
    let bottom = uc.read_i32(lprc as u64 + 12);

    let mut draw_params = None;
    {
        let gdi = uc.get_data().gdi_objects.lock().unwrap();
        if let Some(GdiObject::Dc {
            selected_bitmap,
            associated_window,
            ..
        }) = gdi.get(&hdc)
        {
            let mut brush_color = None;
            if hbr <= 0x1_0000 {
                // Stock brush 또는 System Color index (COLOR_xxx + 1)
                // 간단히 Light Gray 또는 흰색으로 기본처리
                brush_color = Some(0x00C0C0C0);
            } else if let Some(GdiObject::Brush { color }) = gdi.get(&hbr) {
                brush_color = Some(*color);
            }
            draw_params = Some((*selected_bitmap, brush_color, *associated_window));
        }
    }

    if let Some((hbmp, Some(color), hwnd)) = draw_params {
        if hbmp != 0 {
            GDI32::sync_dib_pixels(uc, hbmp);
            let clip_rects = GDI32::clip_rects_for_hdc(uc, hdc);
            let gdi = uc.get_data().gdi_objects.lock().unwrap();
            if let Some(GdiObject::Bitmap {
                width,
                height,
                pixels,
                ..
            }) = gdi.get(&hbmp)
            {
                let width = *width;
                let height = *height;
                let mut pixels = pixels.lock().unwrap();
                for (left, top, right, bottom) in
                    GDI32::intersect_rect_with_clip_rects(&clip_rects, left, top, right, bottom)
                {
                    GdiRenderer::draw_rect(
                        &mut pixels,
                        width,
                        height,
                        left,
                        top,
                        right,
                        bottom,
                        None,
                        Some(color),
                    );
                }
                drop(pixels);
                drop(gdi);
                GDI32::flush_dib_pixels_to_memory(uc, hbmp);
                if hwnd != 0 {
                    uc.get_data().win_event.lock().unwrap().update_window(hwnd);
                }
            }
        }
    }

    crate::emu_log!(
        "[USER32] FillRect({:#x}, {:#x}, {:#x}) -> 1",
        hdc,
        lprc,
        hbr
    );
    Some(ApiHookResult::callee(3, Some(1)))
}

// API: HDC GetDC(HWND hWnd)
// 역할: 지정된 창의 클라이언트 영역에 대한 DC를 가져옴
pub(super) fn get_dc(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let ctx = uc.get_data();
    let hdc = ctx.alloc_handle();
    let (w, h, surface_bitmap) = {
        let win_event = ctx.win_event.lock().unwrap();
        win_event
            .windows
            .get(&hwnd)
            .map(|win| (win.width, win.height, win.surface_bitmap))
            .unwrap_or((640, 480, 0))
    };
    ctx.gdi_objects.lock().unwrap().insert(
        hdc,
        GdiObject::Dc {
            associated_window: hwnd,
            width: w as i32,
            height: h as i32,
            selected_bitmap: surface_bitmap,
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
    Some(ApiHookResult::callee(1, Some(hdc as i32)))
}

// API: HDC GetWindowDC(HWND hWnd)
// 역할: 지정된 창 전체(비클라이언트 영역 포함)에 대한 DC를 가져옴
pub(super) fn get_window_dc(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let ctx = uc.get_data();
    let hdc = ctx.alloc_handle();
    let (w, h, surface_bitmap) = {
        let win_event = ctx.win_event.lock().unwrap();
        win_event
            .windows
            .get(&hwnd)
            .map(|win| (win.width, win.height, win.surface_bitmap))
            .unwrap_or((640, 480, 0))
    };
    ctx.gdi_objects.lock().unwrap().insert(
        hdc,
        GdiObject::Dc {
            associated_window: hwnd,
            width: w as i32,
            height: h as i32,
            selected_bitmap: surface_bitmap,
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
    Some(ApiHookResult::callee(1, Some(hdc as i32)))
}

// API: int ReleaseDC(HWND hWnd, HDC hDC)
// 역할: DC를 해제
pub(super) fn release_dc(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let hdc = uc.read_arg(1);
    let ctx = uc.get_data();
    ctx.gdi_objects.lock().unwrap().remove(&hdc);
    ctx.win_event.lock().unwrap().update_window(hwnd);
    crate::emu_log!("[USER32] ReleaseDC({:#x}, {:#x}) -> INT 1", hwnd, hdc);
    Some(ApiHookResult::callee(2, Some(1)))
}

// API: UINT_PTR SetTimer(HWND hWnd, UINT_PTR nIDEvent, UINT uElapse, TIMERPROC lpTimerFunc)
// 역할: 타이머를 생성
pub(super) fn set_timer(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let mut id = uc.read_arg(1);
    let elapse = uc.read_arg(2);
    let lp_timer_func = uc.read_arg(3);

    let ctx = uc.get_data();
    let mut timers = ctx.timers.lock().unwrap();
    if id == 0 {
        id = ctx.alloc_handle();
    }

    timers.insert(
        id,
        Timer {
            hwnd,
            id,
            elapse,
            timer_proc: lp_timer_func,
            last_tick: std::time::Instant::now(),
        },
    );

    crate::emu_log!(
        "[USER32] SetTimer({:#x}, {:#x}, {:#x}, {:#x}) -> UINT_PTR {:#x}",
        hwnd,
        id,
        elapse,
        lp_timer_func,
        id
    );
    Some(ApiHookResult::callee(4, Some(id as i32)))
}

// API: BOOL KillTimer(HWND hWnd, UINT_PTR uIDEvent)
// 역할: 타이머를 제거함
pub(super) fn kill_timer(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let id = uc.read_arg(1);

    let ctx = uc.get_data();
    let mut timers = ctx.timers.lock().unwrap();
    let removed = timers.remove(&id).is_some();

    crate::emu_log!("[USER32] KillTimer({:#x}, {:#x}) -> {}", hwnd, id, removed);
    Some(ApiHookResult::callee(2, Some(if removed { 1 } else { 0 })))
}
