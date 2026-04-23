use crate::ui::gdi_renderer::GdiRenderer;
use crate::{
    dll::win32::{ApiHookResult, GdiObject, Timer, Win32Context, gdi32::GDI32},
    helper::UnicornHelper,
};
use unicorn_engine::Unicorn;

const WS_CHILD: u32 = 0x4000_0000;
const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
const WS_CLIPCHILDREN: u32 = 0x0200_0000;

const WHITE_BRUSH: u32 = 0;
const LTGRAY_BRUSH: u32 = 1;
const GRAY_BRUSH: u32 = 2;
const DKGRAY_BRUSH: u32 = 3;
const BLACK_BRUSH: u32 = 4;
const NULL_BRUSH: u32 = 5;
const OPAQUE: i32 = 2;
const DEFAULT_BK_COLOR: u32 = 0xFFFF_FFFF;
const DEFAULT_TEXT_COLOR: u32 = 0xFF00_0000;
const R2_COPYPEN: i32 = 13;

fn system_color_to_rgb(index: u32) -> u32 {
    match index {
        5 => 0xFFFF_FFFF,  // COLOR_WINDOW
        8 => 0xFF00_0000,  // COLOR_WINDOWTEXT
        15 => 0xFFC0_C0C0, // COLOR_BTNFACE
        _ => 0xFFC0_C0C0,
    }
}

fn stock_brush_to_rgb(index: u32) -> Option<u32> {
    match index {
        WHITE_BRUSH => Some(0xFFFF_FFFF),
        LTGRAY_BRUSH => Some(0xFFC0_C0C0),
        GRAY_BRUSH => Some(0xFF80_8080),
        DKGRAY_BRUSH => Some(0xFF40_4040),
        BLACK_BRUSH => Some(0xFF00_0000),
        NULL_BRUSH => None,
        _ => None,
    }
}

fn normalize_rect(rect: (i32, i32, i32, i32)) -> Option<(i32, i32, i32, i32)> {
    let (left, top, right, bottom) = rect;
    (left < right && top < bottom).then_some((left, top, right, bottom))
}

/// 창 표면 내부에서 클라이언트 영역 사각형을 계산합니다.
fn window_client_rect(ctx: &Win32Context, hwnd: u32) -> Option<(i32, i32, i32, i32, u32, u32)> {
    let win_event = ctx.win_event.lock().unwrap();
    let state = win_event.windows.get(&hwnd)?;
    let metrics = crate::dll::win32::user32::USER32::get_window_frame_metrics(state);
    let left = metrics.left;
    let top = metrics.top;
    let mut right = state.width.max(0);
    let mut bottom = state.height.max(0);

    right -= metrics.right;
    bottom -= metrics.bottom;

    right = right.max(left);
    bottom = bottom.max(top);
    Some((
        left,
        top,
        right,
        bottom,
        state.surface_bitmap,
        u32::try_from(state.width.max(0)).ok()?,
    ))
}

/// 윈도우 스타일과 Z 순서를 고려해 클라이언트 클리핑 사각형 목록을 계산합니다.
fn build_client_clip_rects(ctx: &Win32Context, hwnd: u32) -> Option<Vec<(i32, i32, i32, i32)>> {
    let win_event = ctx.win_event.lock().unwrap();
    let state = win_event.windows.get(&hwnd)?;
    let metrics = crate::dll::win32::user32::USER32::get_window_frame_metrics(state);
    let base_rect = normalize_rect((
        metrics.left,
        metrics.top,
        (state.width.max(0) - metrics.right).max(metrics.left),
        (state.height.max(0) - metrics.bottom).max(metrics.top),
    ))?;

    let mut subtractors = Vec::new();

    if (state.style & WS_CLIPCHILDREN) != 0 {
        for (&child_hwnd, child) in &win_event.windows {
            if child_hwnd == hwnd
                || child.parent != hwnd
                || !child.visible
                || (child.style & WS_CHILD) == 0
            {
                continue;
            }

            if let Some(rect) = normalize_rect((
                metrics.left + child.x,
                metrics.top + child.y,
                metrics.left + child.x + child.width.max(0),
                metrics.top + child.y + child.height.max(0),
            )) {
                subtractors.push(rect);
            }
        }
    }

    if state.parent != 0 && (state.style & WS_CHILD) != 0 && (state.style & WS_CLIPSIBLINGS) != 0 {
        for (&sibling_hwnd, sibling) in &win_event.windows {
            if sibling_hwnd == hwnd
                || sibling.parent != state.parent
                || !sibling.visible
                || (sibling.style & WS_CHILD) == 0
                || sibling.z_order <= state.z_order
            {
                continue;
            }

            if let Some(rect) = normalize_rect((
                sibling.x - state.x,
                sibling.y - state.y,
                sibling.x - state.x + sibling.width.max(0),
                sibling.y - state.y + sibling.height.max(0),
            )) {
                subtractors.push(rect);
            }
        }
    }

    Some(GDI32::subtract_region_rects(&[base_rect], &subtractors))
}

/// 클라이언트 영역용 임시 클리핑 리전을 생성합니다.
fn create_client_clip_region(ctx: &Win32Context, hwnd: u32) -> u32 {
    let Some(rects) = build_client_clip_rects(ctx, hwnd) else {
        return 0;
    };
    let hrgn = ctx.alloc_handle();
    ctx.gdi_objects
        .lock()
        .unwrap()
        .insert(hrgn, GdiObject::Region { rects });
    hrgn
}

/// 브러시 핸들 또는 `COLOR_xxx + 1` 값을 실제 RGB 색상으로 풉니다.
pub(super) fn resolve_brush_color(ctx: &Win32Context, hbrush: u32) -> Option<u32> {
    if hbrush == 0 {
        return None;
    }

    if hbrush <= 0x1_0000 {
        return Some(system_color_to_rgb(hbrush.saturating_sub(1)));
    }

    let gdi = ctx.gdi_objects.lock().unwrap();
    match gdi.get(&hbrush) {
        Some(GdiObject::Brush { color }) => Some(*color),
        Some(GdiObject::StockObject(index)) => stock_brush_to_rgb(*index),
        _ => None,
    }
}

/// 창 클래스에 등록된 배경 브러시로 전체 클라이언트 영역을 지웁니다.
pub(super) fn erase_window_background(uc: &mut Unicorn<Win32Context>, hwnd: u32) -> bool {
    let (left, top, right, bottom, hbmp, hbr_background, clip_rects) = {
        let ctx = uc.get_data();
        let client_rect = window_client_rect(ctx, hwnd);
        let clip_rects = build_client_clip_rects(ctx, hwnd);
        let win_event = ctx.win_event.lock().unwrap();
        win_event
            .windows
            .get(&hwnd)
            .and_then(|state| {
                client_rect.map(|(left, top, right, bottom, surface_bitmap, _)| {
                    (
                        left,
                        top,
                        right,
                        bottom,
                        surface_bitmap,
                        state.class_hbr_background,
                        clip_rects.unwrap_or_else(|| vec![(left, top, right, bottom)]),
                    )
                })
            })
            .unwrap_or((0, 0, 0, 0, 0, 0, Vec::new()))
    };

    if hbmp == 0 {
        return false;
    }

    let Some(color) = resolve_brush_color(uc.get_data(), hbr_background) else {
        return false;
    };

    GDI32::sync_dib_pixels(uc, hbmp);

    let gdi = uc.get_data().gdi_objects.lock().unwrap();
    let Some(GdiObject::Bitmap {
        width,
        height,
        pixels,
        ..
    }) = gdi.get(&hbmp)
    else {
        return false;
    };

    let mut pixels = pixels.lock().unwrap();
    GdiRenderer::draw_rect_clipped(
        &mut pixels,
        *width,
        *height,
        left,
        top,
        right,
        bottom,
        None,
        Some(color),
        &clip_rects,
    );
    drop(pixels);
    true
}

// API: HDC BeginPaint(HWND hWnd, LPPAINTSTRUCT lpPaint)
// 역할: 그리기를 준비하고 PAINTSTRUCT를 채움. WM_PAINT 처리 시 사용됨.
pub(super) fn begin_paint(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hwnd = uc.read_arg(0);
    let lp_paint = uc.read_arg(1);

    let selected_region = create_client_clip_region(uc.get_data(), hwnd);
    let (width, height, origin_x, origin_y, surface_bitmap, hdc) = {
        let ctx = uc.get_data();
        let mut win_event = ctx.win_event.lock().unwrap();
        if let Some(state) = win_event.windows.get_mut(&hwnd) {
            state.needs_paint = false; // 그리기 시작했으므로 무효 영역 해제
            let metrics = crate::dll::win32::user32::USER32::get_window_frame_metrics(state);
            (
                (state.width - metrics.left - metrics.right).max(0) as u32,
                (state.height - metrics.top - metrics.bottom).max(0) as u32,
                metrics.left,
                metrics.top,
                state.surface_bitmap,
                ctx.alloc_handle(),
            )
        } else {
            return Some(ApiHookResult::callee(2, Some(0)));
        }
    };

    // 실제 Win32의 BeginPaint는 무조건 배경을 다시 칠하지 않습니다.
    // guest가 GetDC로 먼저 그려 둔 wallpaper/background가 있는데 여기서 강제로 지우면
    // WM_PAINT 시점에 다시 복원하지 않는 창은 검은 client 영역만 남게 됩니다.
    // 배경 지우기는 WM_ERASEBKGND 처리 경로에만 맡기고, BeginPaint는 DC 준비만 수행합니다.
    let erased = false;

    // WM_PAINT 경로에서도 일반 GetDC와 동일하게 창 표면에 연결된 DC를 제공합니다.
    uc.get_data().gdi_objects.lock().unwrap().insert(
        hdc,
        GdiObject::Dc {
            associated_window: hwnd,
            width: width as i32,
            height: height as i32,
            origin_x,
            origin_y,
            selected_bitmap: surface_bitmap,
            selected_font: 0,
            selected_brush: 0,
            selected_pen: 0,
            selected_region,
            selected_palette: 0,
            bk_mode: OPAQUE,
            bk_color: DEFAULT_BK_COLOR,
            text_color: DEFAULT_TEXT_COLOR,
            rop2_mode: R2_COPYPEN,
            current_x: 0,
            current_y: 0,
        },
    );

    // PAINTSTRUCT 채우기
    uc.write_u32(lp_paint as u64, hdc); // hdc
    uc.write_u32(lp_paint as u64 + 4, if erased { 1 } else { 0 }); // fErase
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
    let target_tid = ctx.window_owner_thread_id(hwnd);
    let mut win_event = ctx.win_event.lock().unwrap();
    if win_event.windows.contains_key(&hwnd) {
        win_event.invalidate_rect(hwnd, std::ptr::null_mut());
        drop(win_event);
        ctx.wake_thread_message_wait(target_tid);
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
            origin_x,
            origin_y,
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
                *origin_x,
                *origin_y,
                font_height,
            ))
        } else {
            None
        }
    };

    let Some((hbmp, text_color, bk_color, bk_mode, hwnd, origin_x, origin_y, font_height)) =
        draw_params
    else {
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
                    draw_x + origin_x,
                    draw_y + origin_y,
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
    let brush_color = resolve_brush_color(uc.get_data(), hbr);

    let mut draw_params = None;
    {
        let gdi = uc.get_data().gdi_objects.lock().unwrap();
        if let Some(GdiObject::Dc {
            selected_bitmap,
            associated_window,
            origin_x,
            origin_y,
            ..
        }) = gdi.get(&hdc)
        {
            draw_params = Some((
                *selected_bitmap,
                brush_color,
                *associated_window,
                *origin_x,
                *origin_y,
            ));
        }
    }

    if let Some((hbmp, Some(color), hwnd, origin_x, origin_y)) = draw_params
        && hbmp != 0
    {
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
            for (left, top, right, bottom) in GDI32::intersect_rect_with_clip_rects(
                &clip_rects,
                left + origin_x,
                top + origin_y,
                right + origin_x,
                bottom + origin_y,
            ) {
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
    let (w, h, origin_x, origin_y, surface_bitmap) = {
        let win_event = ctx.win_event.lock().unwrap();
        win_event
            .windows
            .get(&hwnd)
            .map(|win| {
                let metrics = crate::dll::win32::user32::USER32::get_window_frame_metrics(win);
                (
                    (win.width - metrics.left - metrics.right).max(0),
                    (win.height - metrics.top - metrics.bottom).max(0),
                    metrics.left,
                    metrics.top,
                    win.surface_bitmap,
                )
            })
            .unwrap_or((640, 480, 0, 0, 0))
    };
    // client clip 생성은 `win_event` 락을 푼 뒤에 수행해야 재귀 락으로 막히지 않습니다.
    let selected_region = create_client_clip_region(ctx, hwnd);
    ctx.gdi_objects.lock().unwrap().insert(
        hdc,
        GdiObject::Dc {
            associated_window: hwnd,
            width: w,
            height: h,
            origin_x,
            origin_y,
            selected_bitmap: surface_bitmap,
            selected_font: 0,
            selected_brush: 0,
            selected_pen: 0,
            selected_region,
            selected_palette: 0,
            bk_mode: OPAQUE,
            bk_color: DEFAULT_BK_COLOR,
            text_color: DEFAULT_TEXT_COLOR,
            rop2_mode: R2_COPYPEN,
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
            width: w,
            height: h,
            origin_x: 0,
            origin_y: 0,
            selected_bitmap: surface_bitmap,
            selected_font: 0,
            selected_brush: 0,
            selected_pen: 0,
            selected_region: 0,
            selected_palette: 0,
            bk_mode: OPAQUE,
            bk_color: DEFAULT_BK_COLOR,
            text_color: DEFAULT_TEXT_COLOR,
            rop2_mode: R2_COPYPEN,
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

    // 창 소유 스레드 ID를 캐시하여 타이머 필터링 시 windows 맵 역참조를 피합니다.
    let owner_thread_id = ctx.window_owner_thread_id(hwnd);
    timers.insert(
        id,
        Timer {
            hwnd,
            id,
            elapse,
            timer_proc: lp_timer_func,
            last_tick: std::time::Instant::now(),
            owner_thread_id,
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
    let before = timers.len();
    // Win32 명세: hwnd와 id가 모두 일치하는 타이머만 제거합니다.
    timers.retain(|_, timer| !(timer.hwnd == hwnd && timer.id == id));
    let removed = timers.len() < before;

    crate::emu_log!("[USER32] KillTimer({:#x}, {:#x}) -> {}", hwnd, id, removed);
    Some(ApiHookResult::callee(2, Some(if removed { 1 } else { 0 })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dll::win32::{GdiObject, WindowState};
    use crate::helper::UnicornHelper;
    use unicorn_engine::{Arch, Mode, RegisterX86, Unicorn};

    fn new_test_uc() -> Unicorn<'static, Win32Context> {
        let mut uc =
            Unicorn::new_with_data(Arch::X86, Mode::MODE_32, Win32Context::new(None)).unwrap();
        uc.setup(None, None).unwrap();
        uc
    }

    fn write_call_frame(uc: &mut Unicorn<Win32Context>, args: &[u32]) {
        let esp = uc.reg_read(RegisterX86::ESP).unwrap() as u32;
        uc.write_u32(esp as u64, 0xDEAD_BEEF);
        for (index, value) in args.iter().enumerate() {
            uc.write_u32(esp as u64 + 4 + (index as u64 * 4), *value);
        }
    }

    fn sample_window_state(style: u32, parent: u32, z_order: u32) -> WindowState {
        WindowState {
            class_name: "TEST".to_string(),
            class_icon: 0,
            big_icon: 0,
            small_icon: 0,
            class_hbr_background: 0,
            title: "test".to_string(),
            x: 0,
            y: 0,
            width: 100,
            height: 100,
            style,
            ex_style: 0,
            owner_thread_id: 0,
            parent,
            id: 0,
            visible: true,
            enabled: true,
            zoomed: false,
            iconic: false,
            wnd_proc: 0,
            class_cursor: 0,
            user_data: 0,
            use_native_frame: false,
            surface_bitmap: 0,
            window_rgn: 0,
            guest_frame_left: 0,
            guest_frame_top: 0,
            guest_frame_right: 0,
            guest_frame_bottom: 0,
            guest_frame_exact: false,
            needs_paint: false,
            last_hittest_lparam: u32::MAX,
            last_hittest_result: 0,
            z_order,
        }
    }

    #[test]
    fn clipchildren_excludes_visible_child_rectangles() {
        let ctx = Win32Context::new(None);
        {
            let mut win_event = ctx.win_event.lock().unwrap();
            let mut parent = sample_window_state(WS_CLIPCHILDREN, 0, 0);
            parent.width = 200;
            parent.height = 120;
            win_event.create_window(0x1000, parent);

            let mut child = sample_window_state(WS_CHILD, 0x1000, 0);
            child.x = 10;
            child.y = 20;
            child.width = 40;
            child.height = 30;
            win_event.create_window(0x1001, child);
        }

        let hrgn = create_client_clip_region(&ctx, 0x1000);
        let rects = {
            let gdi = ctx.gdi_objects.lock().unwrap();
            match gdi.get(&hrgn) {
                Some(GdiObject::Region { rects }) => rects.clone(),
                other => panic!("expected region, got {:?}", other),
            }
        };

        assert_eq!(
            rects,
            vec![
                (0, 0, 200, 20),
                (0, 50, 200, 120),
                (0, 20, 10, 50),
                (50, 20, 200, 50)
            ]
        );
    }

    #[test]
    fn clipchildren_does_not_exclude_owned_popup_rectangles() {
        let ctx = Win32Context::new(None);
        {
            let mut win_event = ctx.win_event.lock().unwrap();
            let mut parent = sample_window_state(WS_CLIPCHILDREN, 0, 0);
            parent.width = 200;
            parent.height = 120;
            win_event.create_window(0x1000, parent);

            let mut popup = sample_window_state(0x8000_0000, 0x1000, 0);
            popup.x = 10;
            popup.y = 20;
            popup.width = 40;
            popup.height = 30;
            win_event.create_window(0x1001, popup);
        }

        let hrgn = create_client_clip_region(&ctx, 0x1000);
        let rects = {
            let gdi = ctx.gdi_objects.lock().unwrap();
            match gdi.get(&hrgn) {
                Some(GdiObject::Region { rects }) => rects.clone(),
                other => panic!("expected region, got {:?}", other),
            }
        };

        assert_eq!(rects, vec![(0, 0, 200, 120)]);
    }

    #[test]
    fn clipsiblings_excludes_higher_z_sibling_overlap() {
        let ctx = Win32Context::new(None);
        {
            let mut win_event = ctx.win_event.lock().unwrap();
            win_event.create_window(0x1000, sample_window_state(0, 0, 0));

            let mut child = sample_window_state(WS_CHILD | WS_CLIPSIBLINGS, 0x1000, 0);
            child.x = 10;
            child.y = 10;
            child.width = 100;
            child.height = 80;
            win_event.create_window(0x1001, child);

            let mut sibling = sample_window_state(WS_CHILD, 0x1000, 1);
            sibling.x = 60;
            sibling.y = 40;
            sibling.width = 100;
            sibling.height = 80;
            win_event.create_window(0x1002, sibling);
        }

        let hrgn = create_client_clip_region(&ctx, 0x1001);
        let rects = {
            let gdi = ctx.gdi_objects.lock().unwrap();
            match gdi.get(&hrgn) {
                Some(GdiObject::Region { rects }) => rects.clone(),
                other => panic!("expected region, got {:?}", other),
            }
        };

        assert_eq!(rects, vec![(0, 0, 100, 30), (0, 30, 50, 80)]);
    }

    #[test]
    #[cfg_attr(
        target_arch = "aarch64",
        ignore = "cargo test 러너에서 Unicorn 초기화가 SIGILL을 유발함"
    )]
    fn get_dc_applies_client_clip_for_non_native_frame_windows() {
        let mut uc = new_test_uc();
        {
            let mut win_event = uc.get_data().win_event.lock().unwrap();
            let mut state = sample_window_state(0, 0, 0);
            state.width = 100;
            state.height = 80;
            win_event.create_window(0x1000, state);
        }
        write_call_frame(&mut uc, &[0x1000]);

        let result = get_dc(&mut uc).expect("get_dc result");
        let hdc = result.return_value.expect("hdc") as u32;

        assert_eq!(
            GDI32::clip_rects_for_hdc(&uc, hdc),
            Some(vec![(0, 0, 100, 80)])
        );
    }

    #[test]
    #[cfg_attr(
        target_arch = "aarch64",
        ignore = "cargo test 러너에서 Unicorn 초기화가 SIGILL을 유발함"
    )]
    fn get_window_dc_keeps_full_window_unclipped() {
        let mut uc = new_test_uc();
        {
            let mut win_event = uc.get_data().win_event.lock().unwrap();
            let mut state = sample_window_state(0, 0, 0);
            state.width = 100;
            state.height = 80;
            win_event.create_window(0x1000, state);
        }
        write_call_frame(&mut uc, &[0x1000]);

        let result = get_window_dc(&mut uc).expect("get_window_dc result");
        let hdc = result.return_value.expect("hdc") as u32;

        assert_eq!(GDI32::clip_rects_for_hdc(&uc, hdc), None);
    }
}
