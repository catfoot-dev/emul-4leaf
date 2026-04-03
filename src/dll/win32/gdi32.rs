use crate::{
    dll::win32::{ApiHookResult, GdiObject, Win32Context},
    helper::UnicornHelper,
    ui::gdi_renderer::GdiRenderer,
};
use std::sync::{Arc, Mutex};
use unicorn_engine::Unicorn;

/// `GDI32.dll` 프록시 구현 모듈
///
/// 그래픽 디바이스 인터페이스(GDI) 객체 (Font, DC, Bitmap 등) 생성/소멸 호출을 백그라운드에서 추적 및 가상화
pub struct GDI32;

impl GDI32 {
    // API: HDC CreateCompatibleDC(HDC hdc)
    // 역할: 지정된 디바이스와 호환되는 메모리 디바이스 컨텍스트(DC)를 만듦
    pub fn create_compatible_dc(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc = uc.read_arg(0);
        let ctx = uc.get_data();
        let new_hdc = ctx.alloc_handle();

        // 참조 DC가 있으면 해당 크기를 상속, 없으면 기본값 사용
        let (width, height) = {
            let gdi_objects = ctx.gdi_objects.lock().unwrap();
            if let Some(GdiObject::Dc { width, height, .. }) = gdi_objects.get(&hdc) {
                (*width, *height)
            } else {
                (640, 480)
            }
        };

        ctx.gdi_objects.lock().unwrap().insert(
            new_hdc,
            GdiObject::Dc {
                associated_window: 0,
                width,
                height,
                selected_bitmap: 0,
                selected_font: 0,
                selected_brush: 0,
                selected_pen: 0,
                selected_region: 0,
                selected_palette: 0,
                bk_mode: 1, // TRANSPARENT(1) or OPAQUE(2)
                bk_color: 0x00FFFFFF,
                text_color: 0x00000000,
                rop2_mode: 13, // R2_COPYPEN
                current_x: 0,
                current_y: 0,
            },
        );
        crate::emu_log!(
            "[GDI32] CreateCompatibleDC({:#x}) -> HDC {:#x}",
            hdc,
            new_hdc
        );
        Some(ApiHookResult::callee(1, Some(new_hdc as i32)))
    }

    // API: BOOL DeleteDC(HDC hdc)
    // 역할: 지정된 디바이스 컨텍스트(DC)를 삭제
    pub fn delete_dc(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc = uc.read_arg(0);
        uc.get_data().gdi_objects.lock().unwrap().remove(&hdc);
        crate::emu_log!("[GDI32] DeleteDC({:#x}) -> BOOL 1", hdc);
        Some(ApiHookResult::callee(1, Some(1)))
    }

    // API: HBITMAP CreateDIBSection(HDC hdc, const BITMAPINFO *pbmi, UINT usage, VOID **ppvBits, HANDLE hSection, DWORD offset)
    // 역할: 애플리케이션이 직접 쓸 수 있는 DIB(장치 독립적 비트맵)를 생성
    pub fn create_dib_section(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc = uc.read_arg(0);
        let bmi_addr = uc.read_arg(1);
        let usage = uc.read_arg(2);
        let bits_ptr_addr = uc.read_arg(3);
        let hsection = uc.read_arg(4);
        let offset = uc.read_arg(5);
        // BITMAPINFOHEADER: biWidth(+4), biHeight(+8, 음수=탑다운), biBitCount(+14, WORD)
        let width = uc.read_u32(bmi_addr as u64 + 4).max(1);
        let raw_height = uc.read_u32(bmi_addr as u64 + 8);
        let (height, top_down) = if raw_height > 0x7FFFFFFF {
            (raw_height.wrapping_neg().max(1), true)
        } else {
            (raw_height.max(1), false)
        };
        let bpp = (uc.read_u32(bmi_addr as u64 + 14) & 0xFFFF).max(1);
        let row_size = (width * bpp).div_ceil(32) * 4;
        let bmp_size = row_size * height;
        let bits_addr = uc.malloc(bmp_size as usize);
        if bits_ptr_addr != 0 {
            uc.write_u32(bits_ptr_addr as u64, bits_addr as u32);
        }
        let pixels = Arc::new(Mutex::new(vec![0u32; (width * height) as usize]));
        let ctx = uc.get_data();
        let hbmp = ctx.alloc_handle();
        ctx.gdi_objects.lock().unwrap().insert(
            hbmp,
            GdiObject::Bitmap {
                width,
                height,
                pixels,
                bits_addr: Some(bits_addr as u32),
                bpp,
                top_down,
            },
        );
        crate::emu_log!(
            "[GDI32] CreateDIBSection({:#x}, {:#x}, {}, {:#x}, {:#x}, {}) -> HBITMAP {:#x} ({}x{} {}bpp top_down={})",
            hdc,
            bmi_addr,
            usage,
            bits_ptr_addr,
            hsection,
            offset,
            hbmp,
            width,
            height,
            bpp,
            top_down
        );
        Some(ApiHookResult::callee(6, Some(hbmp as i32)))
    }

    // API: HBITMAP CreateCompatibleBitmap(HDC hdc, int cx, int cy)
    // 역할: 지정된 디바이스 컨텍스트의 현재 설정과 호환되는 비트맵을 만듦
    pub fn create_compatible_bitmap(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc = uc.read_arg(0);
        let width = uc.read_arg(1);
        let height = uc.read_arg(2);
        let pixels = Arc::new(Mutex::new(vec![0u32; (width * height) as usize]));
        let ctx = uc.get_data();
        let hbmp = ctx.alloc_handle();
        ctx.gdi_objects.lock().unwrap().insert(
            hbmp,
            GdiObject::Bitmap {
                width,
                height,
                pixels,
                bits_addr: None,
                bpp: 32,
                top_down: false,
            },
        );
        crate::emu_log!(
            "[GDI32] CreateCompatibleBitmap({:#x}, {}, {}) -> HBITMAP {:#x}",
            hdc,
            width,
            height,
            hbmp
        );
        Some(ApiHookResult::callee(3, Some(hbmp as i32)))
    }

    // API: HGDIOBJ SelectObject(HDC hdc, HGDIOBJ h)
    // 역할: 지정된 DC로 객체를 선택하여 기존의 동일한 유형의 객체를 바꿈
    pub fn select_object(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc = uc.read_arg(0);
        let hobj = uc.read_arg(1);
        let ctx = uc.get_data();
        let mut gdi_objects = ctx.gdi_objects.lock().unwrap();
        let obj_clone = gdi_objects.get(&hobj).cloned();
        let mut old_hobj = 0;
        if let Some(GdiObject::Dc {
            selected_bitmap,
            selected_font,
            selected_brush,
            selected_pen,
            selected_region,
            selected_palette,
            ..
        }) = gdi_objects.get_mut(&hdc)
        {
            match obj_clone {
                Some(GdiObject::Bitmap { .. }) => {
                    old_hobj = *selected_bitmap;
                    *selected_bitmap = hobj;
                }
                Some(GdiObject::Font { .. }) => {
                    old_hobj = *selected_font;
                    *selected_font = hobj;
                }
                Some(GdiObject::Brush { .. }) => {
                    old_hobj = *selected_brush;
                    *selected_brush = hobj;
                }
                Some(GdiObject::Pen { .. }) => {
                    old_hobj = *selected_pen;
                    *selected_pen = hobj;
                }
                Some(GdiObject::Region { .. }) => {
                    old_hobj = *selected_region;
                    *selected_region = hobj;
                }
                Some(GdiObject::Palette { .. }) => {
                    old_hobj = *selected_palette;
                    *selected_palette = hobj;
                }
                Some(GdiObject::StockObject(id)) => {
                    if id < 5 || id == 18 {
                        old_hobj = *selected_brush;
                        *selected_brush = hobj;
                    } else if id >= 5 && id <= 8 {
                        old_hobj = *selected_pen;
                        *selected_pen = hobj;
                    } else if id >= 10 && id <= 17 {
                        old_hobj = *selected_font;
                        *selected_font = hobj;
                    }
                }
                None => { /* 알 수 없는 객체이거나 이미 삭제된 경우 */ }
                _ => { /* Dc 객체는 선택할 수 없음 */ }
            }
        }
        crate::emu_log!(
            "[GDI32] SelectObject({:#x}, {:#x}) -> HGDIOBJ {:#x}",
            hdc,
            hobj,
            old_hobj
        );
        Some(ApiHookResult::callee(2, Some(old_hobj as i32)))
    }

    // API: BOOL DeleteObject(HGDIOBJ ho)
    // 역할: 논리적 펜, 브러시, 폰트, 비트맵, 영역, 또는 팔레트를 삭제하여 시스템 리소스를 확보
    pub fn delete_object(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hobj = uc.read_arg(0);
        uc.get_data().gdi_objects.lock().unwrap().remove(&hobj);
        crate::emu_log!("[GDI32] DeleteObject({:#x}) -> BOOL 1", hobj);
        Some(ApiHookResult::callee(1, Some(1)))
    }

    // API: HGDIOBJ GetStockObject(int i)
    // 역할: 미리 정의된 펜, 브러시, 폰트 또는 팔레트 중 하나의 핸들을 가져옴
    pub fn get_stock_object(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let index = uc.read_arg(0);
        let ctx = uc.get_data();
        let handle = ctx.alloc_handle();
        ctx.gdi_objects
            .lock()
            .unwrap()
            .insert(handle, GdiObject::StockObject(index));
        crate::emu_log!("[GDI32] GetStockObject({}) -> HGDIOBJ {:#x}", index, handle);
        Some(ApiHookResult::callee(1, Some(handle as i32)))
    }

    // API: int GetDeviceCaps(HDC hdc, int index)
    // 역할: 지정된 디바이스에 대한 특정 장치 구성 정보를 가져옴
    pub fn get_device_caps(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc = uc.read_arg(0);
        let index = uc.read_arg(1);
        let ctx = uc.get_data();
        let gdi_objects = ctx.gdi_objects.lock().unwrap();
        let mut result = 0;
        if let Some(GdiObject::Dc { width, height, .. }) = gdi_objects.get(&hdc) {
            match index {
                1 => result = *width,  // HORZRES
                2 => result = *height, // VERTRES
                3 => result = *width,  // HORZSIZE
                4 => result = *height, // VERTSIZE
                5 => result = 96,      // LOGPIXELSX
                6 => result = 96,      // LOGPIXELSY
                10 => result = 1,      // BITSPIXEL
                11 => result = 1,      // PLANES
                14 => result = 1,      // SHADEBLENDCAPS
                16 => result = 1,      // BITSYPIXELS
                17 => result = 1,      // NUMRESERVED
                18 => result = 1,      // CURVECAPS
                19 => result = 1,      // FONTTYPE
                20 => result = 1,      // HORZRES
                21 => result = 1,      // VERTRES
                22 => result = 1,      // LOGPIXELSX
                23 => result = 1,      // LOGPIXELSY
                24 => result = 1,      // BITSYPIXELS
                25 => result = 1,      // NUMRESERVED
                26 => result = 1,      // CURVECAPS
                27 => result = 1,      // FONTTYPE
                _ => {}                // 알 수 없는 인덱스
            }
        }
        crate::emu_log!(
            "[GDI32] GetDeviceCaps({:#x}, {}) -> int {}",
            hdc,
            index,
            result
        );
        Some(ApiHookResult::callee(2, Some(result)))
    }

    // API: HFONT CreateFontIndirectA(const LOGFONTA *lplf)
    // 역할: 논리적 폰트 구조체에 지정된 특성을 가진 폰트를 생성
    pub fn create_font_indirect_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
    pub fn create_font_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
    pub fn get_text_metrics_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
    pub fn get_text_extent_point32_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
    pub fn get_text_extent_point_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
    pub fn get_char_width_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
    pub fn text_out_a(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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

    // API: int SetBkMode(HDC hdc, int mode)
    // 역할: 배경 혼합 모드를 설정
    pub fn set_bk_mode(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc = uc.read_arg(0);
        let mode = uc.read_arg(1) as i32;
        let mut old_mode = 1;
        if let Some(GdiObject::Dc { bk_mode, .. }) =
            uc.get_data().gdi_objects.lock().unwrap().get_mut(&hdc)
        {
            old_mode = *bk_mode;
            *bk_mode = mode;
        }
        crate::emu_log!(
            "[GDI32] SetBkMode({:#x}, {:#x}) -> int {:#x}",
            hdc,
            mode,
            old_mode
        );
        Some(ApiHookResult::callee(2, Some(old_mode)))
    }

    // API: int GetBkMode(HDC hdc)
    // 역할: 배경 혼합 모드를 가져옴
    pub fn get_bk_mode(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc = uc.read_arg(0);
        let mode = uc
            .get_data()
            .gdi_objects
            .lock()
            .unwrap()
            .get(&hdc)
            .map(|obj| {
                if let GdiObject::Dc { bk_mode, .. } = obj {
                    *bk_mode
                } else {
                    1
                }
            })
            .unwrap_or(1);
        crate::emu_log!("[GDI32] GetBkMode({:#x}) -> int {:#x}", hdc, mode);
        Some(ApiHookResult::callee(1, Some(mode)))
    }

    // API: COLORREF SetBkColor(HDC hdc, COLORREF color)
    // 역할: 배경 색상을 설정
    pub fn set_bk_color(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc = uc.read_arg(0);
        let color = uc.read_arg(1);
        let mut old_color = 0x00FFFFFF;
        if let Some(GdiObject::Dc { bk_color, .. }) =
            uc.get_data().gdi_objects.lock().unwrap().get_mut(&hdc)
        {
            old_color = *bk_color;
            *bk_color = color;
        }
        crate::emu_log!(
            "[GDI32] SetBkColor({:#x}, {:#x}) -> COLORREF {:#x}",
            hdc,
            color,
            old_color
        );
        Some(ApiHookResult::callee(2, Some(old_color as i32)))
    }

    // API: COLORREF GetBkColor(HDC hdc)
    // 역할: 배경 색상을 가져옴
    pub fn get_bk_color(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc = uc.read_arg(0);
        let color = uc
            .get_data()
            .gdi_objects
            .lock()
            .unwrap()
            .get(&hdc)
            .map(|obj| {
                if let GdiObject::Dc { bk_color, .. } = obj {
                    *bk_color
                } else {
                    0x00FFFFFF
                }
            })
            .unwrap_or(0x00FFFFFF);
        crate::emu_log!("[GDI32] GetBkColor({:#x}) -> COLORREF {:#x}", hdc, color);
        Some(ApiHookResult::callee(1, Some(color as i32)))
    }

    // API: COLORREF SetTextColor(HDC hdc, COLORREF color)
    // 역할: 텍스트 색상을 설정
    pub fn set_text_color(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc = uc.read_arg(0);
        let color = uc.read_arg(1);
        let mut old_color = 0;
        if let Some(GdiObject::Dc { text_color, .. }) =
            uc.get_data().gdi_objects.lock().unwrap().get_mut(&hdc)
        {
            old_color = *text_color;
            *text_color = color;
        }
        crate::emu_log!(
            "[GDI32] SetTextColor({:#x}, {:#x}) -> COLORREF {:#x}",
            hdc,
            color,
            old_color
        );
        Some(ApiHookResult::callee(2, Some(old_color as i32)))
    }

    // API: COLORREF GetTextColor(HDC hdc)
    // 역할: 텍스트 색상을 가져옴
    pub fn get_text_color(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc = uc.read_arg(0);
        let color = uc
            .get_data()
            .gdi_objects
            .lock()
            .unwrap()
            .get(&hdc)
            .map(|obj| {
                if let GdiObject::Dc { text_color, .. } = obj {
                    *text_color
                } else {
                    0
                }
            })
            .unwrap_or(0);
        crate::emu_log!("[GDI32] GetTextColor({:#x}) -> COLORREF {:#x}", hdc, color);
        Some(ApiHookResult::callee(1, Some(color as i32)))
    }

    // API: HPEN CreatePen(int iStyle, int cWidth, COLORREF color)
    // 역할: 지정된 스타일, 너비 및 색상을 가진 논리적 펜을 생성
    pub fn create_pen(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let style = uc.read_arg(0);
        let width = uc.read_arg(1);
        let color = uc.read_arg(2);
        let ctx = uc.get_data();
        let hpen = ctx.alloc_handle();
        ctx.gdi_objects.lock().unwrap().insert(
            hpen,
            GdiObject::Pen {
                style,
                width,
                color,
            },
        );
        crate::emu_log!(
            "[GDI32] CreatePen({:#x}, {}, {:#x}) -> HPEN {:#x}",
            style,
            width,
            color,
            hpen
        );
        Some(ApiHookResult::callee(3, Some(hpen as i32)))
    }

    // API: HBRUSH CreateSolidBrush(COLORREF color)
    // 역할: 지정된 단색을 가지는 논리적 브러시를 생성
    pub fn create_solid_brush(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let color = uc.read_arg(0);
        let ctx = uc.get_data();
        let hbrush = ctx.alloc_handle();
        ctx.gdi_objects
            .lock()
            .unwrap()
            .insert(hbrush, GdiObject::Brush { color });
        crate::emu_log!(
            "[GDI32] CreateSolidBrush({:#x}) -> HBRUSH {:#x}",
            color,
            hbrush
        );
        Some(ApiHookResult::callee(1, Some(hbrush as i32)))
    }

    // API: HRGN CreateRectRgn(int x1, int y1, int x2, int y2)
    // 역할: 직사각형 영역(Region)을 생성
    pub fn create_rect_rgn(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let x1 = uc.read_arg(0) as i32;
        let y1 = uc.read_arg(1) as i32;
        let x2 = uc.read_arg(2) as i32;
        let y2 = uc.read_arg(3) as i32;
        let ctx = uc.get_data();
        let hrgn = ctx.alloc_handle();
        ctx.gdi_objects.lock().unwrap().insert(
            hrgn,
            GdiObject::Region {
                left: x1,
                top: y1,
                right: x2,
                bottom: y2,
            },
        );
        crate::emu_log!(
            "[GDI32] CreateRectRgn({:#x}, {:#x}, {:#x}, {:#x}) -> HRGN {:#x}",
            x1,
            y1,
            x2,
            y2,
            hrgn
        );
        Some(ApiHookResult::callee(4, Some(hrgn as i32)))
    }

    // API: int SelectClipRgn(HDC hdc, HRGN hrgn)
    // 역할: 지정된 영역(Region)을 디바이스 컨텍스트(DC)의 클리핑 영역으로 설정
    pub fn select_clip_rgn(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc = uc.read_arg(0);
        let hrgn = uc.read_arg(1);
        let ctx = uc.get_data();
        let mut result = 0;
        if let Some(GdiObject::Dc {
            selected_region, ..
        }) = ctx.gdi_objects.lock().unwrap().get_mut(&hdc)
        {
            *selected_region = hrgn;
            result = 1;
        }
        crate::emu_log!(
            "[GDI32] SelectClipRgn({:#x}, {:#x}) -> int {:#x}",
            hdc,
            hrgn,
            result
        );
        Some(ApiHookResult::callee(2, Some(result)))
    }

    // API: int CombineRgn(HRGN hrgnDest, HRGN hrgnSrc1, HRGN hrgnSrc2, int fnCombine)
    // 역할: 두 영역(Region)을 결합하여 새로운 영역을 생성
    pub fn combine_rgn(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hrgn_dest = uc.read_arg(0);
        let hrgn_src1 = uc.read_arg(1);
        let hrgn_src2 = uc.read_arg(2);
        let fn_combine = uc.read_arg(3);
        let ctx = uc.get_data();
        let mut result = 0;
        let mut gdi_objects = ctx.gdi_objects.lock().unwrap();
        let region1 = if let Some(GdiObject::Region {
            left,
            top,
            right,
            bottom,
        }) = gdi_objects.get(&hrgn_src1)
        {
            Some((*left, *top, *right, *bottom))
        } else {
            None
        };
        let region2 = if let Some(GdiObject::Region {
            left,
            top,
            right,
            bottom,
        }) = gdi_objects.get(&hrgn_src2)
        {
            Some((*left, *top, *right, *bottom))
        } else {
            None
        };

        if let (Some(r1), Some(r2)) = (region1, region2) {
            let left = r1.0.min(r2.0);
            let top = r1.1.min(r2.1);
            let right = r1.2.max(r2.2);
            let bottom = r1.3.max(r2.3);
            gdi_objects.insert(
                hrgn_dest,
                GdiObject::Region {
                    left,
                    top,
                    right,
                    bottom,
                },
            );
            result = 1;
        }
        crate::emu_log!(
            "[GDI32] CombineRgn({:#x}, {:#x}, {:#x}, {:#x}) -> int {:#x}",
            hrgn_dest,
            hrgn_src1,
            hrgn_src2,
            fn_combine,
            result
        );
        Some(ApiHookResult::callee(4, Some(result)))
    }

    // API: BOOL EqualRgn(HRGN hrgn1, HRGN hrgn2)
    // 역할: 두 영역(Region)이 동일한지 확인
    pub fn equal_rgn(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hrgn1 = uc.read_arg(0);
        let hrgn2 = uc.read_arg(1);
        let ctx = uc.get_data();
        let mut result = 0;
        let gdi_objects = ctx.gdi_objects.lock().unwrap();
        let region1 = if let Some(GdiObject::Region {
            left,
            top,
            right,
            bottom,
        }) = gdi_objects.get(&hrgn1)
        {
            Some((*left, *top, *right, *bottom))
        } else {
            None
        };
        let region2 = if let Some(GdiObject::Region {
            left,
            top,
            right,
            bottom,
        }) = gdi_objects.get(&hrgn2)
        {
            Some((*left, *top, *right, *bottom))
        } else {
            None
        };

        if let (Some(r1), Some(r2)) = (region1, region2) {
            if r1 == r2 {
                result = 1;
            }
        }
        crate::emu_log!(
            "[GDI32] EqualRgn({:#x}, {:#x}) -> BOOL {:#x}",
            hrgn1,
            hrgn2,
            result
        );
        Some(ApiHookResult::callee(2, Some(result)))
    }

    // API: int GetRgnBox(HRGN hrgn, LPRECT lprc)
    // 역할: 영역(Region)의 경계 사각형(Bounding Rectangle)을 가져옴
    pub fn get_rgn_box(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hrgn = uc.read_arg(0);
        let lprc = uc.read_arg(1);

        let rect = {
            let ctx = uc.get_data();
            if let Some(GdiObject::Region {
                left,
                top,
                right,
                bottom,
            }) = ctx.gdi_objects.lock().unwrap().get(&hrgn)
            {
                Some([*left, *top, *right, *bottom])
            } else {
                None
            }
        };

        let mut result = 0;
        if let Some(r) = rect {
            uc.write_mem(lprc as u64, &r);
            result = 1;
        }

        crate::emu_log!(
            "[GDI32] GetRgnBox({:#x}, {:#x}) -> int {:#x}",
            hrgn,
            lprc,
            result
        );
        Some(ApiHookResult::callee(2, Some(result)))
    }

    // API: int SetDIBitsToDevice(HDC hdc, int xDest, int yDest, DWORD dwWidth, DWORD dwHeight, int xSrc, int ySrc, UINT uStartScan, UINT cScans, const VOID *lpBits, const BITMAPINFO *lpBitsInfo, UINT uUsage)
    // 역할: DIB 데이터를 DC의 비트맵에 직접 복사
    pub fn set_dib_its_to_device(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc = uc.read_arg(0);
        let x_dest = uc.read_arg(1) as i32;
        let y_dest = uc.read_arg(2) as i32;
        let dw_width = uc.read_arg(3);
        let dw_height = uc.read_arg(4);
        let x_src = uc.read_arg(5) as i32;
        let y_src = uc.read_arg(6) as i32;
        let u_start_scan = uc.read_arg(7);
        let c_scans = uc.read_arg(8);
        let lp_bits = uc.read_arg(9);
        let lp_bits_info = uc.read_arg(10);
        let _u_usage = uc.read_arg(11);

        crate::emu_log!(
            "[GDI32] SetDIBitsToDevice({:#x}, {}, {}, {}, {}, {}, {}, {}, {}, {:#x}, {:#x})",
            hdc,
            x_dest,
            y_dest,
            dw_width,
            dw_height,
            x_src,
            y_src,
            u_start_scan,
            c_scans,
            lp_bits,
            lp_bits_info
        );

        if lp_bits == 0 || lp_bits_info == 0 || c_scans == 0 {
            return Some(ApiHookResult::callee(12, Some(c_scans as i32)));
        }

        // BITMAPINFOHEADER 읽기
        let bmi_width = uc.read_u32(lp_bits_info as u64 + 4).max(1);
        let raw_height = uc.read_u32(lp_bits_info as u64 + 8);
        let (_bmi_height, top_down) = if raw_height > 0x7FFFFFFF {
            (raw_height.wrapping_neg().max(1), true)
        } else {
            (raw_height.max(1), false)
        };
        let bpp = (uc.read_u32(lp_bits_info as u64 + 14) & 0xFFFF).max(1);
        let stride = ((bmi_width * bpp + 31) / 32) * 4;

        // 8bpp 팔레트 읽기 (BITMAPINFOHEADER 크기 = 40)
        let mut palette: Vec<[u8; 4]> = Vec::new();
        if bpp == 8 {
            let clr_used = uc.read_u32(lp_bits_info as u64 + 32);
            let num_colors = if clr_used == 0 {
                256
            } else {
                clr_used.min(256)
            };
            let pal_offset = lp_bits_info as u64 + 40;
            for i in 0..num_colors as u64 {
                let b = uc.read_u32(pal_offset + i * 4) as u8;
                let g = (uc.read_u32(pal_offset + i * 4) >> 8) as u8;
                let r = (uc.read_u32(pal_offset + i * 4) >> 16) as u8;
                palette.push([b, g, r, 0]);
            }
        }

        let total_bytes = (stride * c_scans) as usize;
        let raw = uc
            .mem_read_as_vec(lp_bits as u64, total_bytes)
            .unwrap_or_default();
        if raw.is_empty() {
            return Some(ApiHookResult::callee(12, Some(c_scans as i32)));
        }

        // c_scans 개 행을 변환 (bottom-up 기준 uStartScan번째 행부터)
        let src_pixels = Self::raw_dib_to_pixels(&raw, bmi_width, c_scans, bpp, top_down, &palette);

        // 대상 DC → 비트맵 찾기
        let (hbmp_dest, hwnd_dest) = {
            let gdi = uc.get_data().gdi_objects.lock().unwrap();
            if let Some(GdiObject::Dc {
                selected_bitmap,
                associated_window,
                ..
            }) = gdi.get(&hdc)
            {
                (*selected_bitmap, *associated_window)
            } else {
                return Some(ApiHookResult::callee(12, Some(c_scans as i32)));
            }
        };

        if hbmp_dest != 0 {
            let gdi = uc.get_data().gdi_objects.lock().unwrap();
            if let Some(GdiObject::Bitmap {
                width: dw,
                height: dh,
                pixels: dp,
                ..
            }) = gdi.get(&hbmp_dest)
            {
                let dw = *dw;
                let dh = *dh;
                let mut dp = dp.lock().unwrap();
                // y_dest 기준으로 복사 (uStartScan + y_src 오프셋 적용)
                let dst_y_start = y_dest + u_start_scan as i32 - y_src;
                GdiRenderer::bit_blt(
                    &mut dp,
                    dw,
                    dh,
                    x_dest,
                    dst_y_start,
                    dw_width,
                    c_scans,
                    &src_pixels,
                    bmi_width,
                    c_scans,
                    x_src,
                    0,
                );
            }
        }
        if hwnd_dest != 0 {
            uc.get_data()
                .win_event
                .lock()
                .unwrap()
                .update_window(hwnd_dest);
        }
        Some(ApiHookResult::callee(12, Some(c_scans as i32)))
    }

    // API: int StretchDIBits(HDC hdc, int xDest, int yDest, int nDestWidth, int nDestHeight, int xSrc, int ySrc, int nSrcWidth, int nSrcHeight, const VOID *lpBits, const BITMAPINFO *lpBitsInfo, UINT uUsage, DWORD rop)
    // 역할: DIB 데이터를 스케일링하여 DC의 비트맵에 복사
    pub fn stretch_dib_its(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc = uc.read_arg(0);
        let x_dest = uc.read_arg(1) as i32;
        let y_dest = uc.read_arg(2) as i32;
        let n_dest_width = uc.read_arg(3) as i32;
        let n_dest_height = uc.read_arg(4) as i32;
        let x_src = uc.read_arg(5) as i32;
        let y_src = uc.read_arg(6) as i32;
        let n_src_width = uc.read_arg(7) as i32;
        let n_src_height = uc.read_arg(8) as i32;
        let lp_bits = uc.read_arg(9);
        let lp_bits_info = uc.read_arg(10);
        let _u_usage = uc.read_arg(11);
        let _rop = uc.read_arg(12);

        crate::emu_log!(
            "[GDI32] StretchDIBits({:#x}, {},{} {}x{} <- {},{} {}x{} bits={:#x})",
            hdc,
            x_dest,
            y_dest,
            n_dest_width,
            n_dest_height,
            x_src,
            y_src,
            n_src_width,
            n_src_height,
            lp_bits
        );

        if lp_bits == 0 || lp_bits_info == 0 || n_src_width <= 0 || n_src_height <= 0 {
            return Some(ApiHookResult::callee(13, Some(n_dest_height)));
        }

        // BITMAPINFOHEADER 읽기
        let bmi_width = uc.read_u32(lp_bits_info as u64 + 4).max(1);
        let raw_height = uc.read_u32(lp_bits_info as u64 + 8);
        let (bmi_height, top_down) = if raw_height > 0x7FFFFFFF {
            (raw_height.wrapping_neg().max(1), true)
        } else {
            (raw_height.max(1), false)
        };
        let bpp = (uc.read_u32(lp_bits_info as u64 + 14) & 0xFFFF).max(1);
        let stride = ((bmi_width * bpp + 31) / 32) * 4;

        let mut palette: Vec<[u8; 4]> = Vec::new();
        if bpp == 8 {
            let clr_used = uc.read_u32(lp_bits_info as u64 + 32);
            let num_colors = if clr_used == 0 {
                256
            } else {
                clr_used.min(256)
            };
            let pal_offset = lp_bits_info as u64 + 40;
            for i in 0..num_colors as u64 {
                let val = uc.read_u32(pal_offset + i * 4);
                palette.push([val as u8, (val >> 8) as u8, (val >> 16) as u8, 0]);
            }
        }

        let total_bytes = (stride * bmi_height) as usize;
        let raw = uc
            .mem_read_as_vec(lp_bits as u64, total_bytes)
            .unwrap_or_default();
        if raw.is_empty() {
            return Some(ApiHookResult::callee(13, Some(n_dest_height)));
        }

        let src_pixels =
            Self::raw_dib_to_pixels(&raw, bmi_width, bmi_height, bpp, top_down, &palette);

        let (hbmp_dest, hwnd_dest) = {
            let gdi = uc.get_data().gdi_objects.lock().unwrap();
            if let Some(GdiObject::Dc {
                selected_bitmap,
                associated_window,
                ..
            }) = gdi.get(&hdc)
            {
                (*selected_bitmap, *associated_window)
            } else {
                return Some(ApiHookResult::callee(13, Some(n_dest_height)));
            }
        };

        if hbmp_dest != 0 {
            let gdi = uc.get_data().gdi_objects.lock().unwrap();
            if let Some(GdiObject::Bitmap {
                width: dw,
                height: dh,
                pixels: dp,
                ..
            }) = gdi.get(&hbmp_dest)
            {
                let dw = *dw;
                let dh = *dh;
                let mut dp = dp.lock().unwrap();
                // 최근접 이웃(nearest-neighbor) 스케일링
                let _sw = n_src_width as u32;
                let _sh = n_src_height as u32;
                let dw_dest = n_dest_width as i32;
                let dh_dest = n_dest_height as i32;
                for dy in 0..dh_dest {
                    let sy = y_src + dy * n_src_height / n_dest_height;
                    let sy = sy.clamp(0, n_src_height - 1) as u32;
                    for dx in 0..dw_dest {
                        let sx = x_src + dx * n_src_width / n_dest_width;
                        let sx = sx.clamp(0, n_src_width - 1) as u32;
                        let dst_x = x_dest + dx;
                        let dst_y = y_dest + dy;
                        if dst_x < 0 || dst_y < 0 || dst_x >= dw as i32 || dst_y >= dh as i32 {
                            continue;
                        }
                        let src_idx = (sy * bmi_width + sx) as usize;
                        let dst_idx = (dst_y as u32 * dw + dst_x as u32) as usize;
                        if src_idx < src_pixels.len() && dst_idx < dp.len() {
                            dp[dst_idx] = src_pixels[src_idx];
                        }
                    }
                }
            }
        }
        if hwnd_dest != 0 {
            uc.get_data()
                .win_event
                .lock()
                .unwrap()
                .update_window(hwnd_dest);
        }
        Some(ApiHookResult::callee(13, Some(n_dest_height)))
    }

    // API: int SetStretchBltMode(HDC hdc, int mode)
    // 역할: 디바이스 컨텍스트(DC)의 스트레치 블릿(StretchBlt) 모드를 설정
    pub fn set_stretch_blt_mode(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc = uc.read_arg(0);
        let mode = uc.read_arg(1);
        crate::emu_log!("[GDI32] SetStretchBltMode({:#x}, {}) -> int 1", hdc, mode);
        Some(ApiHookResult::callee(2, Some(1)))
    }

    // Helper: 원시 DIB 바이트 배열을 0x00RRGGBB Vec<u32>으로 변환 (BGR/BGRA, bottom-up 지원)
    fn raw_dib_to_pixels(
        raw: &[u8],
        width: u32,
        height: u32,
        bpp: u32,
        top_down: bool,
        palette: &[[u8; 4]], // 8bpp 팔레트 (RGBQUAD 배열)
    ) -> Vec<u32> {
        let stride = ((width * bpp + 31) / 32) * 4;
        let mut pixels = vec![0u32; (width * height) as usize];
        for row in 0..height as usize {
            let src_row = if top_down {
                row
            } else {
                height as usize - 1 - row
            };
            let row_offset = src_row * stride as usize;
            for col in 0..width as usize {
                let dst_idx = row * width as usize + col;
                let color = match bpp {
                    8 => {
                        let idx = row_offset + col;
                        if idx < raw.len() {
                            let p = raw[idx] as usize;
                            if p < palette.len() {
                                let b = palette[p][0] as u32;
                                let g = palette[p][1] as u32;
                                let r = palette[p][2] as u32;
                                (r << 16) | (g << 8) | b
                            } else {
                                0
                            }
                        } else {
                            0
                        }
                    }
                    24 => {
                        let idx = row_offset + col * 3;
                        if idx + 2 < raw.len() {
                            let b = raw[idx] as u32;
                            let g = raw[idx + 1] as u32;
                            let r = raw[idx + 2] as u32;
                            (r << 16) | (g << 8) | b
                        } else {
                            0
                        }
                    }
                    _ => {
                        // 32bpp (BGRA)
                        let idx = row_offset + col * 4;
                        if idx + 2 < raw.len() {
                            let b = raw[idx] as u32;
                            let g = raw[idx + 1] as u32;
                            let r = raw[idx + 2] as u32;
                            (r << 16) | (g << 8) | b
                        } else {
                            0
                        }
                    }
                };
                if dst_idx < pixels.len() {
                    pixels[dst_idx] = color;
                }
            }
        }
        pixels
    }

    // Helper: DIBSection 비트맵의 emulated memory 데이터를 GdiObject::Bitmap.pixels Vec에 동기화
    fn sync_dib_pixels(uc: &mut Unicorn<Win32Context>, hbmp: u32) {
        let bmp_info = {
            let gdi = uc.get_data().gdi_objects.lock().unwrap();
            match gdi.get(&hbmp) {
                Some(GdiObject::Bitmap {
                    width,
                    height,
                    pixels,
                    bits_addr: Some(addr),
                    bpp,
                    top_down,
                    ..
                }) => Some((*width, *height, pixels.clone(), *addr, *bpp, *top_down)),
                _ => None,
            }
        };
        let (width, height, pixels_arc, addr, bpp, top_down) = match bmp_info {
            Some(v) => v,
            None => return,
        };
        let stride = ((width * bpp + 31) / 32) * 4;
        let total_bytes = (stride * height) as usize;
        let raw = uc
            .mem_read_as_vec(addr as u64, total_bytes)
            .unwrap_or_default();
        if raw.is_empty() {
            return;
        }
        let converted = Self::raw_dib_to_pixels(&raw, width, height, bpp, top_down, &[]);
        let mut pixels = pixels_arc.lock().unwrap();
        if pixels.len() == converted.len() {
            pixels.copy_from_slice(&converted);
        }
    }

    // API: BOOL BitBlt(HDC hdcDest, int xDest, int yDest, int nDestWidth, int nDestHeight, HDC hdcSrc, int xSrc, int ySrc, DWORD rop)
    // 역할: 디바이스 컨텍스트(DC)의 지정된 위치에 픽셀로 설정
    pub fn bit_blt(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc_dest = uc.read_arg(0);
        let x_dest = uc.read_arg(1) as i32;
        let y_dest = uc.read_arg(2) as i32;
        let n_dest_width = uc.read_arg(3);
        let n_dest_height = uc.read_arg(4);
        let hdc_src = uc.read_arg(5);
        let x_src = uc.read_arg(6) as i32;
        let y_src = uc.read_arg(7) as i32;
        let rop = uc.read_arg(8);

        if rop == 0x00CC0020 {
            // SRCCOPY
            let mut draw_params = None;
            {
                let gdi_objects = uc.get_data().gdi_objects.lock().unwrap();
                if let Some(GdiObject::Dc {
                    selected_bitmap: hbmp_dest,
                    associated_window: hwnd_dest,
                    ..
                }) = gdi_objects.get(&hdc_dest)
                {
                    if let Some(GdiObject::Dc {
                        selected_bitmap: hbmp_src,
                        ..
                    }) = gdi_objects.get(&hdc_src)
                    {
                        draw_params = Some((*hbmp_dest, *hbmp_src, *hwnd_dest));
                    }
                }
            }

            if let Some((hbmp_dest, hbmp_src, hwnd_dest)) = draw_params {
                if hbmp_dest != 0 && hbmp_src != 0 {
                    // DIBSection이면 emulated memory에서 pixels Vec으로 동기화
                    Self::sync_dib_pixels(uc, hbmp_src);
                    let gdi_objects = uc.get_data().gdi_objects.lock().unwrap();
                    if let (
                        Some(GdiObject::Bitmap {
                            width: dw,
                            height: dh,
                            pixels: dp,
                            ..
                        }),
                        Some(GdiObject::Bitmap {
                            width: sw,
                            height: sh,
                            pixels: sp,
                            ..
                        }),
                    ) = (gdi_objects.get(&hbmp_dest), gdi_objects.get(&hbmp_src))
                    {
                        let (dw, dh) = (*dw, *dh);
                        let (sw, sh) = (*sw, *sh);
                        let mut dp = dp.lock().unwrap();
                        let sp = sp.lock().unwrap();
                        GdiRenderer::bit_blt(
                            &mut dp,
                            dw,
                            dh,
                            x_dest,
                            y_dest,
                            n_dest_width,
                            n_dest_height,
                            &sp,
                            sw,
                            sh,
                            x_src,
                            y_src,
                        );
                        if hwnd_dest != 0 {
                            uc.get_data()
                                .win_event
                                .lock()
                                .unwrap()
                                .update_window(hwnd_dest);
                        }
                    }
                }
            }
        }

        crate::emu_log!(
            "[GDI32] BitBlt({:#x}, {}, {}, {}, {}, {:#x}, {}, {}, {:#x}) -> int 1",
            hdc_dest,
            x_dest,
            y_dest,
            n_dest_width,
            n_dest_height,
            hdc_src,
            x_src,
            y_src,
            rop
        );
        Some(ApiHookResult::callee(9, Some(1)))
    }

    // API: BOOL Rectangle(HDC hdc, int left, int top, int right, int bottom)
    // 역할: 현재 펜과 브러시를 사용하여 직사각형을 그림
    pub fn rectangle(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc = uc.read_arg(0);
        let left = uc.read_arg(1) as i32;
        let top = uc.read_arg(2) as i32;
        let right = uc.read_arg(3) as i32;
        let bottom = uc.read_arg(4) as i32;

        let mut draw_params = None;
        if let Some(GdiObject::Dc {
            selected_bitmap,
            selected_pen,
            selected_brush,
            associated_window,
            ..
        }) = uc.get_data().gdi_objects.lock().unwrap().get(&hdc)
        {
            draw_params = Some((
                *selected_bitmap,
                *selected_pen,
                *selected_brush,
                *associated_window,
            ));
        }

        if let Some((hbmp, hpen, hbrush, hwnd)) = draw_params {
            if hbmp != 0 {
                let (pen_color, brush_color) = {
                    let gdi_objects = uc.get_data().gdi_objects.lock().unwrap();
                    let pen_color = if hpen != 0 {
                        if let Some(GdiObject::Pen { color, .. }) = gdi_objects.get(&hpen) {
                            Some(*color)
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    let brush_color = if hbrush != 0 {
                        if let Some(GdiObject::Brush { color }) = gdi_objects.get(&hbrush) {
                            Some(*color)
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    (pen_color, brush_color)
                };

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
                    GdiRenderer::draw_rect(
                        &mut pixels,
                        width,
                        height,
                        left,
                        top,
                        right,
                        bottom,
                        pen_color,
                        brush_color,
                    );
                    if hwnd != 0 {
                        uc.get_data().win_event.lock().unwrap().update_window(hwnd);
                    }
                }
            }
        }

        crate::emu_log!(
            "[GDI32] Rectangle({:#x}, {}, {}, {}, {}) -> int 1",
            hdc,
            left,
            top,
            right,
            bottom
        );
        Some(ApiHookResult::callee(5, Some(1)))
    }

    // API: BOOL MoveToEx(HDC hdc, int x, int y, LPPOINT lppt)
    // 역할: 현재 그리기 위치를 지정된 좌표로 갱신
    pub fn move_to_ex(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc = uc.read_arg(0);
        let x = uc.read_arg(1) as i32;
        let y = uc.read_arg(2) as i32;
        let lppt = uc.read_arg(3);

        let mut old_x = 0;
        let mut old_y = 0;
        if let Some(GdiObject::Dc {
            current_x,
            current_y,
            ..
        }) = uc.get_data().gdi_objects.lock().unwrap().get_mut(&hdc)
        {
            old_x = *current_x;
            old_y = *current_y;
            *current_x = x;
            *current_y = y;
        }

        if lppt != 0 {
            uc.write_u32(lppt as u64, old_x as u32);
            uc.write_u32(lppt as u64 + 4, old_y as u32);
        }
        crate::emu_log!(
            "[GDI32] MoveToEx({:#x}, {}, {}, {:#x}) -> int 1",
            hdc,
            x,
            y,
            lppt
        );
        Some(ApiHookResult::callee(4, Some(1)))
    }

    // API: BOOL LineTo(HDC hdc, int x, int y)
    // 역할: 현재 위치에서 지정된 끝점까지 선을 그림
    pub fn line_to(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc = uc.read_arg(0);
        let x = uc.read_arg(1) as i32;
        let y = uc.read_arg(2) as i32;

        let mut draw_data = None;

        if let Some(GdiObject::Dc {
            current_x,
            current_y,
            selected_bitmap,
            text_color,
            associated_window,
            ..
        }) = uc.get_data().gdi_objects.lock().unwrap().get_mut(&hdc)
        {
            draw_data = Some((
                *current_x,
                *current_y,
                *selected_bitmap,
                *text_color,
                *associated_window,
            ));
            *current_x = x;
            *current_y = y;
        }

        if let Some((x1, y1, hbmp, color, hwnd)) = draw_data {
            if hbmp != 0 {
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
                    GdiRenderer::draw_line(&mut pixels, width, height, x1, y1, x, y, color);
                    if hwnd != 0 {
                        uc.get_data().win_event.lock().unwrap().update_window(hwnd);
                    }
                }
            }
        }

        crate::emu_log!("[GDI32] LineTo({:#x}, {}, {}) -> BOOL 1", hdc, x, y);
        Some(ApiHookResult::callee(3, Some(1)))
    }

    // API: int SetROP2(HDC hdc, int nROP2)
    // 역할: 디바이스 컨텍스트의 그리기 모드를 설정
    pub fn set_rop2(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc = uc.read_arg(0);
        let mode = uc.read_arg(1) as i32;
        let mut old_mode = 13;
        if let Some(GdiObject::Dc { rop2_mode, .. }) =
            uc.get_data().gdi_objects.lock().unwrap().get_mut(&hdc)
        {
            old_mode = *rop2_mode;
            *rop2_mode = mode;
        }
        crate::emu_log!(
            "[GDI32] SetROP2({:#x}, {}) -> int {:#x}",
            hdc,
            mode,
            old_mode
        );
        Some(ApiHookResult::callee(2, Some(old_mode)))
    }

    // API: UINT RealizePalette(HDC hdc)
    // 역할: 디바이스 컨텍스트의 팔레트를 실제 디바이스에 적용
    pub fn realize_palette(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc = uc.read_arg(0);
        let mut count = 0u32;

        let selected_palette = {
            let gdi_objects = uc.get_data().gdi_objects.lock().unwrap();
            if let Some(GdiObject::Dc {
                selected_palette, ..
            }) = gdi_objects.get(&hdc)
            {
                *selected_palette
            } else {
                0
            }
        };

        if selected_palette != 0 {
            let gdi_objects = uc.get_data().gdi_objects.lock().unwrap();
            if let Some(GdiObject::Palette { num_entries }) = gdi_objects.get(&selected_palette) {
                count = *num_entries;
            }
        }

        crate::emu_log!("[GDI32] RealizePalette({:#x}) -> UINT {:#x}", hdc, count);
        Some(ApiHookResult::callee(1, Some(count as i32)))
    }

    // API: HPALETTE SelectPalette(HDC hdc, HPALETTE hpal, BOOL bForceBkgd)
    // 역할: 디바이스 컨텍스트(DC)에 논리적 팔레트를 선택
    pub fn select_palette(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc = uc.read_arg(0);
        let hpal = uc.read_arg(1);
        let b_force_bkgd = uc.read_arg(2);
        let mut old_pal = 0;
        if let Some(GdiObject::Dc {
            selected_palette, ..
        }) = uc.get_data().gdi_objects.lock().unwrap().get_mut(&hdc)
        {
            old_pal = *selected_palette;
            *selected_palette = hpal;
        }
        crate::emu_log!(
            "[GDI32] SelectPalette({:#x}, {:#x}, {}) -> int {:#x}",
            hdc,
            hpal,
            b_force_bkgd,
            old_pal
        );
        Some(ApiHookResult::callee(3, Some(old_pal as i32)))
    }

    // API: HPALETTE CreatePalette(LPLOGPAL lpLogPalette)
    // 역할: 논리적 팔레트를 생성
    pub fn create_palette(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let logpal_addr = uc.read_arg(0);
        let num_entries = uc.read_u16(logpal_addr as u64 + 2) as u32;
        let ctx = uc.get_data();
        let hpal = ctx.alloc_handle();
        ctx.gdi_objects
            .lock()
            .unwrap()
            .insert(hpal, GdiObject::Palette { num_entries });
        crate::emu_log!(
            "[GDI32] CreatePalette({:#x}) -> HPAL {:#x}",
            logpal_addr,
            hpal
        );
        Some(ApiHookResult::callee(1, Some(hpal as i32)))
    }

    // API: COLORREF GetPixel(HDC hdc, int x, int y)
    // 역할: 지정된 좌표의 픽셀 색상을 가져옴
    pub fn get_pixel(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc = uc.read_arg(0);
        let x = uc.read_arg(1) as i32;
        let y = uc.read_arg(2) as i32;
        let mut color = 0;
        if let Some(GdiObject::Dc {
            selected_bitmap, ..
        }) = uc.get_data().gdi_objects.lock().unwrap().get(&hdc)
        {
            color = *selected_bitmap;
        }
        crate::emu_log!(
            "[GDI32] GetPixel({:#x}, {}, {}) -> COLORREF {:#x}",
            hdc,
            x,
            y,
            color
        );
        Some(ApiHookResult::callee(3, Some(color as i32)))
    }

    // API: COLORREF SetPixel(HDC hdc, int x, int y, COLORREF color)
    // 역할: 지정된 좌표의 픽셀 색상을 설정
    pub fn set_pixel(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc = uc.read_arg(0);
        let x = uc.read_arg(1) as i32;
        let y = uc.read_arg(2) as i32;
        let color = uc.read_arg(3);
        let mut old_color = 0;
        if let Some(GdiObject::Dc {
            selected_bitmap, ..
        }) = uc.get_data().gdi_objects.lock().unwrap().get_mut(&hdc)
        {
            old_color = *selected_bitmap;
            *selected_bitmap = color;
        }
        crate::emu_log!(
            "[GDI32] SetPixel({:#x}, {}, {}, {:#x}) -> COLORREF {:#x}",
            hdc,
            x,
            y,
            color,
            old_color
        );
        Some(ApiHookResult::callee(4, Some(old_color as i32)))
    }

    // API: BOOL StretchBlt(HDC hdcDest, int xDest, int yDest, int nDestWidth, int nDestHeight, HDC hdcSrc, int xSrc, int ySrc, int nSrcWidth, int nSrcHeight, DWORD rop)
    // 역할: 소스 DC 비트맵을 스케일링하여 대상 DC에 복사
    pub fn stretch_blt(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let hdc_dest = uc.read_arg(0);
        let x_dest = uc.read_arg(1) as i32;
        let y_dest = uc.read_arg(2) as i32;
        let n_dest_width = uc.read_arg(3) as i32;
        let n_dest_height = uc.read_arg(4) as i32;
        let hdc_src = uc.read_arg(5);
        let x_src = uc.read_arg(6) as i32;
        let y_src = uc.read_arg(7) as i32;
        let n_src_width = uc.read_arg(8) as i32;
        let n_src_height = uc.read_arg(9) as i32;
        let _rop = uc.read_arg(10);

        crate::emu_log!(
            "[GDI32] StretchBlt({:#x}, {},{} {}x{} <- {:#x} {},{} {}x{})",
            hdc_dest,
            x_dest,
            y_dest,
            n_dest_width,
            n_dest_height,
            hdc_src,
            x_src,
            y_src,
            n_src_width,
            n_src_height
        );

        if n_src_width <= 0 || n_src_height <= 0 || n_dest_width <= 0 || n_dest_height <= 0 {
            return Some(ApiHookResult::callee(11, Some(1)));
        }

        // 소스 비트맵 핸들 추출
        let (hbmp_src, hbmp_dest, hwnd_dest) = {
            let gdi = uc.get_data().gdi_objects.lock().unwrap();
            let src_bmp = if let Some(GdiObject::Dc {
                selected_bitmap, ..
            }) = gdi.get(&hdc_src)
            {
                *selected_bitmap
            } else {
                0
            };
            let (dst_bmp, hwnd) = if let Some(GdiObject::Dc {
                selected_bitmap,
                associated_window,
                ..
            }) = gdi.get(&hdc_dest)
            {
                (*selected_bitmap, *associated_window)
            } else {
                (0, 0)
            };
            (src_bmp, dst_bmp, hwnd)
        };

        if hbmp_src == 0 || hbmp_dest == 0 {
            return Some(ApiHookResult::callee(11, Some(1)));
        }

        // DIBSection 동기화
        Self::sync_dib_pixels(uc, hbmp_src);

        let gdi = uc.get_data().gdi_objects.lock().unwrap();
        if let (
            Some(GdiObject::Bitmap {
                width: sw,
                height: sh,
                pixels: sp,
                ..
            }),
            Some(GdiObject::Bitmap {
                width: dw,
                height: dh,
                pixels: dp,
                ..
            }),
        ) = (gdi.get(&hbmp_src), gdi.get(&hbmp_dest))
        {
            let (sw, sh) = (*sw as i32, *sh as i32);
            let (dw, dh) = (*dw, *dh);
            let sp = sp.lock().unwrap();
            let mut dp = dp.lock().unwrap();

            for dy in 0..n_dest_height {
                let sy = y_src + dy * n_src_height / n_dest_height;
                let sy = sy.clamp(0, sh - 1);
                for dx in 0..n_dest_width {
                    let sx = x_src + dx * n_src_width / n_dest_width;
                    let sx = sx.clamp(0, sw - 1);
                    let dst_x = x_dest + dx;
                    let dst_y = y_dest + dy;
                    if dst_x < 0 || dst_y < 0 || dst_x >= dw as i32 || dst_y >= dh as i32 {
                        continue;
                    }
                    let src_idx = (sy * sw + sx) as usize;
                    let dst_idx = (dst_y as u32 * dw + dst_x as u32) as usize;
                    if src_idx < sp.len() && dst_idx < dp.len() {
                        dp[dst_idx] = sp[src_idx];
                    }
                }
            }
        }
        drop(gdi);

        if hwnd_dest != 0 {
            uc.get_data()
                .win_event
                .lock()
                .unwrap()
                .update_window(hwnd_dest);
        }
        Some(ApiHookResult::callee(11, Some(1)))
    }

    // API: int GetObject(HANDLE h, int c, LPVOID pv)
    // 역할: GDI 오브젝트의 정보를 BITMAP 구조체 등으로 반환
    pub fn get_object(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let h = uc.read_arg(0);
        let _c = uc.read_arg(1);
        let pv = uc.read_arg(2);

        // 락을 먼저 해제하고 write (borrow 충돌 방지)
        let bitmap_info = {
            let gdi = uc.get_data().gdi_objects.lock().unwrap();
            match gdi.get(&h) {
                Some(GdiObject::Bitmap {
                    width,
                    height,
                    bpp,
                    bits_addr,
                    ..
                }) => {
                    let bytes_per_pixel = (*bpp / 8).max(1);
                    let width_bytes = (width * bytes_per_pixel + 3) & !3;
                    Some((*width, *height, width_bytes, *bpp, bits_addr.unwrap_or(0)))
                }
                Some(GdiObject::Font { .. }) => None,
                _ => None,
            }
        };

        let result = if let Some((width, height, width_bytes, bpp, bits)) = bitmap_info {
            if pv != 0 {
                uc.write_u32(pv as u64, 0); // bmType
                uc.write_u32(pv as u64 + 4, width); // bmWidth
                uc.write_u32(pv as u64 + 8, height); // bmHeight
                uc.write_u32(pv as u64 + 12, width_bytes); // bmWidthBytes
                uc.write_u32(pv as u64 + 16, 1); // bmPlanes
                uc.write_u32(pv as u64 + 20, bpp); // bmBitsPixel
                uc.write_u32(pv as u64 + 24, bits); // bmBits
            }
            28
        } else {
            let gdi = uc.get_data().gdi_objects.lock().unwrap();
            match gdi.get(&h) {
                Some(GdiObject::Font { .. }) => 60,
                _ => 0,
            }
        };
        crate::emu_log!("[GDI32] GetObject({:#x}) -> {}", h, result);
        Some(ApiHookResult::callee(3, Some(result)))
    }

    // API: HBITMAP CreateBitmap(int nWidth, int nHeight, UINT nPlanes, UINT nBitCount, const VOID *lpBits)
    // 역할: 지정된 크기와 색 형식의 비트맵을 생성
    pub fn create_bitmap(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let width = uc.read_arg(0).max(1);
        let height = uc.read_arg(1).max(1);
        let _n_planes = uc.read_arg(2);
        let bpp = uc.read_arg(3).max(1);
        let lp_bits = uc.read_arg(4);

        let pixels = Arc::new(Mutex::new(vec![0u32; (width * height) as usize]));

        // 초기 비트 데이터가 있으면 읽어서 변환
        if lp_bits != 0 {
            let stride = ((width * bpp + 31) / 32) * 4;
            let total_bytes = (stride * height) as usize;
            let raw = uc
                .mem_read_as_vec(lp_bits as u64, total_bytes)
                .unwrap_or_default();
            if !raw.is_empty() {
                let converted = Self::raw_dib_to_pixels(&raw, width, height, bpp, false, &[]);
                let mut p = pixels.lock().unwrap();
                if p.len() == converted.len() {
                    p.copy_from_slice(&converted);
                }
            }
        }

        let ctx = uc.get_data();
        let hbmp = ctx.alloc_handle();
        ctx.gdi_objects.lock().unwrap().insert(
            hbmp,
            GdiObject::Bitmap {
                width,
                height,
                pixels,
                bits_addr: None,
                bpp,
                top_down: false,
            },
        );
        crate::emu_log!(
            "[GDI32] CreateBitmap({}, {}, {}) -> HBITMAP {:#x}",
            width,
            height,
            bpp,
            hbmp
        );
        Some(ApiHookResult::callee(5, Some(hbmp as i32)))
    }

    /// 함수명 기준 `GDI32.dll` API 구현체
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        match func_name {
            "CreateCompatibleDC" => Self::create_compatible_dc(uc),
            "DeleteDC" => Self::delete_dc(uc),
            "CreateDIBSection" => Self::create_dib_section(uc),
            "CreateCompatibleBitmap" => Self::create_compatible_bitmap(uc),
            "SelectObject" => Self::select_object(uc),
            "DeleteObject" => Self::delete_object(uc),
            "GetStockObject" => Self::get_stock_object(uc),
            "GetDeviceCaps" => Self::get_device_caps(uc),
            "CreateFontIndirectA" => Self::create_font_indirect_a(uc),
            "CreateFontA" => Self::create_font_a(uc),
            "GetTextMetricsA" => Self::get_text_metrics_a(uc),
            "GetTextExtentPoint32A" => Self::get_text_extent_point32_a(uc),
            "GetTextExtentPointA" => Self::get_text_extent_point_a(uc),
            "GetCharWidthA" => Self::get_char_width_a(uc),
            "TextOutA" => Self::text_out_a(uc),
            "SetBkMode" => Self::set_bk_mode(uc),
            "GetBkMode" => Self::get_bk_mode(uc),
            "SetBkColor" => Self::set_bk_color(uc),
            "GetBkColor" => Self::get_bk_color(uc),
            "SetTextColor" => Self::set_text_color(uc),
            "GetTextColor" => Self::get_text_color(uc),
            "CreatePen" => Self::create_pen(uc),
            "CreateSolidBrush" => Self::create_solid_brush(uc),
            "CreateRectRgn" => Self::create_rect_rgn(uc),
            "SelectClipRgn" => Self::select_clip_rgn(uc),
            "CombineRgn" => Self::combine_rgn(uc),
            "EqualRgn" => Self::equal_rgn(uc),
            "GetRgnBox" => Self::get_rgn_box(uc),
            "SetDIBitsToDevice" => Self::set_dib_its_to_device(uc),
            "StretchDIBits" => Self::stretch_dib_its(uc),
            "SetStretchBltMode" => Self::set_stretch_blt_mode(uc),
            "BitBlt" => Self::bit_blt(uc),
            "StretchBlt" => Self::stretch_blt(uc),
            "GetObject" => Self::get_object(uc),
            "CreateBitmap" => Self::create_bitmap(uc),
            "Rectangle" => Self::rectangle(uc),
            "MoveToEx" => Self::move_to_ex(uc),
            "LineTo" => Self::line_to(uc),
            "SetROP2" => Self::set_rop2(uc),
            "RealizePalette" => Self::realize_palette(uc),
            "SelectPalette" => Self::select_palette(uc),
            "CreatePalette" => Self::create_palette(uc),
            "GetPixel" => Self::get_pixel(uc),
            "SetPixel" => Self::set_pixel(uc),
            _ => {
                crate::emu_log!("[!] GDI32 Unhandled: {}", func_name);
                None
            }
        }
    }
}
