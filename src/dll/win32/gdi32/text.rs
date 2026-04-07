use crate::{
    dll::win32::{ApiHookResult, GdiObject, Win32Context},
    helper::UnicornHelper,
    ui::gdi_renderer::GdiRenderer,
};
use unicorn_engine::Unicorn;

use super::GDI32;

// API: HFONT CreateFontIndirectA(const LOGFONTA *lplf)
// 역할: 논리적 폰트 구조체에 지정된 특성을 가진 폰트를 생성
pub(super) fn create_font_indirect_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let lplf = uc.read_arg(0);
    // LOGFONTA: lfHeight at +0, face_name at +28 (inline char[32])
    let lf_height = uc.read_u32(lplf as u64) as i32;
    let face_name = uc.read_euc_kr(lplf as u64 + 28);
    let ctx = uc.get_data();
    let hfont = ctx.alloc_handle();
    ctx.gdi_objects.lock().unwrap().insert(
        hfont,
        GdiObject::Font {
            name: face_name,
            height: if lf_height == 0 { 12 } else { lf_height },
        },
    );
    crate::emu_log!(
        "[GDI32] CreateFontIndirectA({:#x}) -> HFONT {:#x}",
        lplf,
        hfont
    );
    Some(ApiHookResult::callee(1, Some(hfont as i32)))
}

// API: HFONT CreateFontA(int cHeight, int cWidth, int cEscapement, int cOrientation, int cWeight, DWORD bItalic, DWORD bUnderline, DWORD bStrikeOut, DWORD iCharSet, DWORD iOutPrecision, DWORD iClipPrecision, DWORD iQuality, DWORD iPitchAndFamily, LPCSTR pszFaceName)
// 역할: 지정된 특성을 가진 논리적 폰트를 생성
pub(super) fn create_font_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let height = uc.read_arg(0) as i32;
    let width = uc.read_arg(1) as i32;
    let escapement = uc.read_arg(2) as i32;
    let orientation = uc.read_arg(3) as i32;
    let weight = uc.read_arg(4) as i32;
    let italic = uc.read_arg(5) as i32;
    let underline = uc.read_arg(6) as i32;
    let strikeout = uc.read_arg(7) as i32;
    let charset = uc.read_arg(8) as i32;
    let out_precision = uc.read_arg(9) as i32;
    let clip_precision = uc.read_arg(10) as i32;
    let quality = uc.read_arg(11) as i32;
    let pitch_and_family = uc.read_arg(12) as i32;
    let face_name_addr = uc.read_arg(13);
    let face_name = if face_name_addr != 0 {
        uc.read_euc_kr(face_name_addr as u64)
    } else {
        String::new()
    };
    let ctx = uc.get_data();
    let hfont = ctx.alloc_handle();
    ctx.gdi_objects.lock().unwrap().insert(
        hfont,
        GdiObject::Font {
            name: face_name.clone(),
            height: if height == 0 { 12 } else { height },
        },
    );
    crate::emu_log!(
        "[GDI32] CreateFontA({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, \"{}\") -> HFONT {:#x}",
        height,
        width,
        escapement,
        orientation,
        weight,
        italic,
        underline,
        strikeout,
        charset,
        out_precision,
        clip_precision,
        quality,
        pitch_and_family,
        face_name,
        hfont
    );
    Some(ApiHookResult::callee(14, Some(hfont as i32)))
}

// API: BOOL GetTextMetricsA(HDC hdc, LPTEXTMETRICA lptm)
// 역할: 지정된 장치 컨텍스트의 텍스트 메트릭스를 가져옴
pub(super) fn get_text_metrics_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hdc = uc.read_arg(0);
    let lptm = uc.read_arg(1);

    let font_height = {
        let ctx = uc.get_data();
        let gdi_objects = ctx.gdi_objects.lock().unwrap();
        if let Some(GdiObject::Dc { selected_font, .. }) = gdi_objects.get(&hdc) {
            if let Some(GdiObject::Font { height, .. }) = gdi_objects.get(selected_font) {
                Some(*height)
            } else {
                None
            }
        } else {
            None
        }
    };

    let mut ret = 0;
    if let Some(height) = font_height {
        let font_size = height.abs().max(1) as f32;
        let (tm_height, tm_ascent, tm_descent) = GdiRenderer::font_metrics(font_size);
        let avg_width = GdiRenderer::measure_text_width("x", font_size);
        uc.write_u32(lptm as u64, tm_height as u32); // tmHeight
        uc.write_u32(lptm as u64 + 4, tm_ascent as u32); // tmAscent
        uc.write_u32(lptm as u64 + 8, tm_descent as u32); // tmDescent
        uc.write_u32(lptm as u64 + 20, avg_width as u32); // tmAveCharWidth
        uc.write_u32(lptm as u64 + 24, avg_width as u32); // tmMaxCharWidth
        ret = 1;
    }

    crate::emu_log!(
        "[GDI32] GetTextMetricsA({:#x}, {:#x}) -> BOOL {}",
        hdc,
        lptm,
        ret
    );
    Some(ApiHookResult::callee(2, Some(ret)))
}

// API: BOOL GetTextExtentPoint32A(HDC hdc, LPCSTR lpString, int cbString, LPSIZE lpSize)
// 역할: 지정된 장치 컨텍스트에서 문자열의 크기를 가져옴
pub(super) fn get_text_extent_point32_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hdc = uc.read_arg(0);
    let lpstring = uc.read_arg(1);
    let cbstring = uc.read_arg(2);
    let lp_size = uc.read_arg(3);

    let font_height = {
        let ctx = uc.get_data();
        let gdi_objects = ctx.gdi_objects.lock().unwrap();
        if let Some(GdiObject::Dc { selected_font, .. }) = gdi_objects.get(&hdc) {
            if let Some(GdiObject::Font { height, .. }) = gdi_objects.get(selected_font) {
                Some(*height)
            } else {
                None
            }
        } else {
            None
        }
    };

    let text = uc.read_euc_kr(lpstring as u64);
    let text = if cbstring != 0xFFFFFFFF {
        text.chars().take(cbstring as usize).collect::<String>()
    } else {
        text
    };

    let mut ret = 0;
    if let Some(height) = font_height {
        let font_size = height.abs().max(1) as f32;
        let (tm_height, _, _) = GdiRenderer::font_metrics(font_size);
        let text_width = GdiRenderer::measure_text_width(&text, font_size);
        uc.write_u32(lp_size as u64, text_width as u32); // cx
        uc.write_u32(lp_size as u64 + 4, tm_height as u32); // cy
        ret = 1;
    }

    crate::emu_log!(
        "[GDI32] GetTextExtentPoint32A({:#x}, {:#x}, {}, {:#x}) -> BOOL {}",
        hdc,
        lpstring,
        cbstring,
        lp_size,
        ret
    );
    Some(ApiHookResult::callee(4, Some(ret)))
}

// API: BOOL GetTextExtentPointA(HDC hdc, LPCSTR lpString, int cbString, LPSIZE lpSize)
// 역할: 지정된 장치 컨텍스트에서 문자열의 크기를 가져옴
pub(super) fn get_text_extent_point_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hdc = uc.read_arg(0);
    let lpstring = uc.read_arg(1);
    let cbstring = uc.read_arg(2);
    let lp_size = uc.read_arg(3);

    let font_height = {
        let ctx = uc.get_data();
        let gdi_objects = ctx.gdi_objects.lock().unwrap();
        if let Some(GdiObject::Dc { selected_font, .. }) = gdi_objects.get(&hdc) {
            if let Some(GdiObject::Font { height, .. }) = gdi_objects.get(selected_font) {
                Some(*height)
            } else {
                None
            }
        } else {
            None
        }
    };

    let text = uc.read_euc_kr(lpstring as u64);
    let text = if cbstring != 0xFFFFFFFF {
        text.chars().take(cbstring as usize).collect::<String>()
    } else {
        text
    };

    let mut ret = 0;
    if let Some(height) = font_height {
        let font_size = height.abs().max(1) as f32;
        let (tm_height, _, _) = GdiRenderer::font_metrics(font_size);
        let text_width = GdiRenderer::measure_text_width(&text, font_size);
        uc.write_u32(lp_size as u64, text_width as u32); // cx
        uc.write_u32(lp_size as u64 + 4, tm_height as u32); // cy
        ret = 1;
    }

    crate::emu_log!(
        "[GDI32] GetTextExtentPointA({:#x}, {:#x}, {}, {:#x}) -> BOOL {}",
        hdc,
        lpstring,
        cbstring,
        lp_size,
        ret
    );
    Some(ApiHookResult::callee(4, Some(ret)))
}

// API: GetCharWidthA(HDC hdc, UINT FirstChar, UINT LastChar, LPINT lpBuffer)
// 역할: 지정된 장치 컨텍스트에서 문자열의 크기를 가져옴
pub(super) fn get_char_width_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hdc = uc.read_arg(0);
    let first_char = uc.read_arg(1);
    let last_char = uc.read_arg(2);
    let lp_buffer = uc.read_arg(3);

    let font_height = {
        let ctx = uc.get_data();
        let gdi_objects = ctx.gdi_objects.lock().unwrap();
        if let Some(GdiObject::Dc { selected_font, .. }) = gdi_objects.get(&hdc) {
            if let Some(GdiObject::Font { height, .. }) = gdi_objects.get(selected_font) {
                Some(*height)
            } else {
                None
            }
        } else {
            None
        }
    };

    let mut ret = 0;
    if let Some(height) = font_height {
        let font_size = height.abs().max(1) as f32;
        for i in first_char..=last_char {
            let ch = char::from_u32(i).unwrap_or(' ');
            let w = GdiRenderer::measure_text_width(&ch.to_string(), font_size);
            let offset = (i - first_char) * 4;
            uc.write_u32(lp_buffer as u64 + offset as u64, w as u32);
        }
        ret = 1;
    }

    crate::emu_log!(
        "[GDI32] GetCharWidthA({:#x}, {}, {}, {:#x}) -> BOOL {}",
        hdc,
        first_char,
        last_char,
        lp_buffer,
        ret
    );
    Some(ApiHookResult::callee(4, Some(ret)))
}

// API: BOOL TextOutA(HDC hdc, int nXStart, int nYStart, LPCSTR lpString, int cbString)
// 역할: 지정된 장치 컨텍스트에서 문자열을 출력
pub(super) fn text_out_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hdc = uc.read_arg(0);
    let n_x_start = uc.read_arg(1) as i32;
    let n_y_start = uc.read_arg(2) as i32;
    let lp_string = uc.read_arg(3);
    let cb_string = uc.read_arg(4);

    let text = uc.read_euc_kr(lp_string as u64);
    let text = if cb_string != 0xFFFFFFFF {
        text.chars().take(cb_string as usize).collect::<String>()
    } else {
        text
    };

    let mut draw_params = None;
    {
        let gdi_objects = uc.get_data().gdi_objects.lock().unwrap();
        if let Some(GdiObject::Dc {
            selected_bitmap,
            selected_pen,
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
            draw_params = Some((
                *selected_bitmap,
                *selected_pen,
                *text_color,
                *bk_color,
                *bk_mode,
                *associated_window,
                font_height,
            ));
        }
    }

    if let Some((hbmp, _hpen, text_color, bk_color, bk_mode, hwnd, font_height)) = draw_params {
        if hbmp != 0 {
            GDI32::sync_dib_pixels(uc, hbmp);
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
                let mut pixels = pixels.lock().unwrap();
                GdiRenderer::draw_text(
                    &mut pixels,
                    width,
                    height,
                    n_x_start,
                    n_y_start,
                    &text,
                    font_height.abs().max(1) as f32,
                    text_color,
                    if bk_mode == 2 { Some(bk_color) } else { None }, // OPAQUE=2
                );
                drop(pixels);
                drop(gdi_objects);
                GDI32::flush_dib_pixels_to_memory(uc, hbmp);
                if hwnd != 0 {
                    uc.get_data().win_event.lock().unwrap().update_window(hwnd);
                }
            }
        }
    }

    crate::emu_log!(
        "[GDI32] TextOutA({:#x}, {}, {}, \"{}\") -> BOOL 1",
        hdc,
        n_x_start,
        n_y_start,
        text
    );
    Some(ApiHookResult::callee(5, Some(1)))
}
