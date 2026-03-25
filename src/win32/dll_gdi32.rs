use std::sync::{Arc, Mutex};
use unicorn_engine::Unicorn;

use crate::helper::UnicornHelper;
use crate::ui::gdi_renderer::GdiRenderer;
use crate::win32::{ApiHookResult, GdiObject, Win32Context, callee_result};

/// `GDI32.dll` 프록시 구현 모듈
///
/// 그래픽 디바이스 인터페이스(GDI) 객체 (Font, DC, Bitmap 등) 생성/소멸 호출을 백그라운드에서 추적 및 가상화
pub struct DllGDI32;

impl DllGDI32 {
    // API: HDC CreateCompatibleDC(HDC hdc)
    // 역할: 지정된 디바이스와 호환되는 메모리 디바이스 컨텍스트(DC)를 만듦
    pub fn create_compatible_dc(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hdc = uc.read_arg(0);
        let ctx = uc.get_data();
        let new_hdc = ctx.alloc_handle();
        ctx.gdi_objects.lock().unwrap().insert(
            new_hdc,
            GdiObject::Dc {
                associated_window: 0,
                width: 640,
                height: 480,
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
        Some((1, Some(new_hdc as i32)))
    }

    // API: BOOL DeleteDC(HDC hdc)
    // 역할: 지정된 디바이스 컨텍스트(DC)를 삭제
    pub fn delete_dc(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hdc = uc.read_arg(0);
        uc.get_data().gdi_objects.lock().unwrap().remove(&hdc);
        crate::emu_log!("[GDI32] DeleteDC({:#x}) -> BOOL 1", hdc);
        Some((1, Some(1)))
    }

    // API: HBITMAP CreateDIBSection(HDC hdc, const BITMAPINFO *pbmi, UINT usage, VOID **ppvBits, HANDLE hSection, DWORD offset)
    // 역할: 애플리케이션이 직접 쓸 수 있는 DIB(장치 독립적 비트맵)를 생성
    pub fn create_dib_section(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hdc = uc.read_arg(0);
        let bmi_addr = uc.read_arg(1);
        let usage = uc.read_arg(2);
        let bits_ptr_addr = uc.read_arg(3);
        let hsection = uc.read_arg(4);
        let offset = uc.read_arg(5);
        // BITMAPINFOHEADER: width at +4, height at +8
        let width = uc.read_u32(bmi_addr as u64 + 4);
        let height = uc.read_u32(bmi_addr as u64 + 8);
        let bpp = uc.read_u32(bmi_addr as u64 + 14);
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
            },
        );
        crate::emu_log!(
            "[GDI32] CreateDIBSection({:#x}, {:#x}, {}, {:#x}, {:#x}, {}) -> HBITMAP {:#x}",
            hdc,
            bmi_addr,
            usage,
            bits_ptr_addr,
            hsection,
            offset,
            hbmp
        );
        Some((6, Some(hbmp as i32)))
    }

    // API: HBITMAP CreateCompatibleBitmap(HDC hdc, int cx, int cy)
    // 역할: 지정된 디바이스 컨텍스트의 현재 설정과 호환되는 비트맵을 만듦
    pub fn create_compatible_bitmap(
        uc: &mut Unicorn<Win32Context>,
    ) -> Option<(usize, Option<i32>)> {
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
            },
        );
        crate::emu_log!(
            "[GDI32] CreateCompatibleBitmap({:#x}, {}, {}) -> HBITMAP {:#x}",
            hdc,
            width,
            height,
            hbmp
        );
        Some((3, Some(hbmp as i32)))
    }

    // API: HGDIOBJ SelectObject(HDC hdc, HGDIOBJ h)
    // 역할: 지정된 DC로 객체를 선택하여 기존의 동일한 유형의 객체를 바꿈
    pub fn select_object(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((2, Some(old_hobj as i32)))
    }

    // API: BOOL DeleteObject(HGDIOBJ ho)
    // 역할: 논리적 펜, 브러시, 폰트, 비트맵, 영역, 또는 팔레트를 삭제하여 시스템 리소스를 확보
    pub fn delete_object(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hobj = uc.read_arg(0);
        uc.get_data().gdi_objects.lock().unwrap().remove(&hobj);
        crate::emu_log!("[GDI32] DeleteObject({:#x}) -> BOOL 1", hobj);
        Some((1, Some(1)))
    }

    // API: HGDIOBJ GetStockObject(int i)
    // 역할: 미리 정의된 펜, 브러시, 폰트 또는 팔레트 중 하나의 핸들을 가져옴
    pub fn get_stock_object(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let index = uc.read_arg(0);
        let ctx = uc.get_data();
        let handle = ctx.alloc_handle();
        ctx.gdi_objects
            .lock()
            .unwrap()
            .insert(handle, GdiObject::StockObject(index));
        crate::emu_log!("[GDI32] GetStockObject({}) -> HGDIOBJ {:#x}", index, handle);
        Some((1, Some(handle as i32)))
    }

    // API: int GetDeviceCaps(HDC hdc, int index)
    // 역할: 지정된 디바이스에 대한 특정 장치 구성 정보를 가져옴
    pub fn get_device_caps(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((2, Some(result)))
    }

    // API: HFONT CreateFontIndirectA(const LOGFONTA *lplf)
    // 역할: 논리적 폰트 구조체에 지정된 특성을 가진 폰트를 생성
    pub fn create_font_indirect_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let lplf = uc.read_arg(0);
        // LOGFONTA: face_name at +36
        let face_name_addr = uc.read_u32(lplf as u64 + 36);
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
                name: face_name,
                height: 12,
            },
        );
        crate::emu_log!(
            "[GDI32] CreateFontIndirectA({:#x}) -> HFONT {:#x}",
            lplf,
            hfont
        );
        Some((1, Some(hfont as i32)))
    }

    // API: HFONT CreateFontA(int cHeight, int cWidth, int cEscapement, int cOrientation, int cWeight, DWORD bItalic, DWORD bUnderline, DWORD bStrikeOut, DWORD iCharSet, DWORD iOutPrecision, DWORD iClipPrecision, DWORD iQuality, DWORD iPitchAndFamily, LPCSTR pszFaceName)
    // 역할: 지정된 특성을 가진 논리적 폰트를 생성
    pub fn create_font_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
                height: 12,
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
        Some((14, Some(hfont as i32)))
    }

    // API: BOOL GetTextMetricsA(HDC hdc, LPTEXTMETRICA lptm)
    // 역할: 지정된 장치 컨텍스트의 텍스트 메트릭스를 가져옴
    pub fn get_text_metrics_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
            uc.write_u32(lptm as u64, height as u32);
            uc.write_u32(lptm as u64 + 4, (height * 4 / 5) as u32);
            uc.write_u32(lptm as u64 + 8, (height * 1 / 5) as u32);
            uc.write_u32(lptm as u64 + 20, (height * 3 / 5) as u32);
            uc.write_u32(lptm as u64 + 24, (height * 3 / 5) as u32);
            ret = 1;
        }

        crate::emu_log!(
            "[GDI32] GetTextMetricsA({:#x}, {:#x}) -> BOOL {}",
            hdc,
            lptm,
            ret
        );
        Some((2, Some(ret)))
    }

    // API: BOOL GetTextExtentPoint32A(HDC hdc, LPCSTR lpString, int cbString, LPSIZE lpSize)
    // 역할: 지정된 장치 컨텍스트에서 문자열의 크기를 가져옴
    pub fn get_text_extent_point32_a(
        uc: &mut Unicorn<Win32Context>,
    ) -> Option<(usize, Option<i32>)> {
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

        let mut ret = 0;
        if let Some(height) = font_height {
            uc.write_u32(lp_size as u64, height as u32);
            uc.write_u32(lp_size as u64 + 4, (height * 4 / 5) as u32);
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
        Some((4, Some(ret)))
    }

    // API: BOOL GetTextExtentPointA(HDC hdc, LPCSTR lpString, int cbString, LPSIZE lpSize)
    // 역할: 지정된 장치 컨텍스트에서 문자열의 크기를 가져옴
    pub fn get_text_extent_point_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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

        let mut ret = 0;
        if let Some(height) = font_height {
            uc.write_u32(lp_size as u64, height as u32);
            uc.write_u32(lp_size as u64 + 4, (height * 4 / 5) as u32);
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
        Some((4, Some(ret)))
    }

    // API: GetCharWidthA(HDC hdc, UINT FirstChar, UINT LastChar, LPINT lpBuffer)
    // 역할: 지정된 장치 컨텍스트에서 문자열의 크기를 가져옴
    pub fn get_char_width_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
            for i in first_char..=last_char {
                uc.write_u32(lp_buffer as u64 + (i * 4) as u64, height as u32);
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
        Some((4, Some(ret)))
    }

    // API: BOOL TextOutA(HDC hdc, int nXStart, int nYStart, LPCSTR lpString, int cbString)
    // 역할: 지정된 장치 컨텍스트에서 문자열을 출력
    pub fn text_out_a(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
                text_color,
                bk_color,
                bk_mode,
                associated_window,
                ..
            }) = gdi_objects.get(&hdc)
            {
                draw_params = Some((
                    *selected_bitmap,
                    *selected_pen,
                    *text_color,
                    *bk_color,
                    *bk_mode,
                    *associated_window,
                ));
            }
        }

        if let Some((hbmp, _hpen, text_color, bk_color, bk_mode, hwnd)) = draw_params {
            if hbmp != 0 {
                let gdi_objects = uc.get_data().gdi_objects.lock().unwrap();
                if let Some(GdiObject::Bitmap {
                    width,
                    height,
                    pixels,
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
                        text_color,
                        if bk_mode == 2 { Some(bk_color) } else { None }, // OPAQUE=2
                    );
                    drop(pixels);
                    drop(gdi_objects);
                    if hwnd != 0 {
                        uc.get_data()
                            .win_event
                            .lock()
                            .unwrap()
                            .update_window(hwnd);
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
        Some((5, Some(1)))
    }

    // API: int SetBkMode(HDC hdc, int mode)
    // 역할: 배경 혼합 모드를 설정
    pub fn set_bk_mode(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((2, Some(old_mode)))
    }

    // API: int GetBkMode(HDC hdc)
    // 역할: 배경 혼합 모드를 가져옴
    pub fn get_bk_mode(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((1, Some(mode)))
    }

    // API: COLORREF SetBkColor(HDC hdc, COLORREF color)
    // 역할: 배경 색상을 설정
    pub fn set_bk_color(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((2, Some(old_color as i32)))
    }

    // API: COLORREF GetBkColor(HDC hdc)
    // 역할: 배경 색상을 가져옴
    pub fn get_bk_color(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((1, Some(color as i32)))
    }

    // API: COLORREF SetTextColor(HDC hdc, COLORREF color)
    // 역할: 텍스트 색상을 설정
    pub fn set_text_color(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((2, Some(old_color as i32)))
    }

    // API: COLORREF GetTextColor(HDC hdc)
    // 역할: 텍스트 색상을 가져옴
    pub fn get_text_color(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((1, Some(color as i32)))
    }

    // API: HPEN CreatePen(int iStyle, int cWidth, COLORREF color)
    // 역할: 지정된 스타일, 너비 및 색상을 가진 논리적 펜을 생성
    pub fn create_pen(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((3, Some(hpen as i32)))
    }

    // API: HBRUSH CreateSolidBrush(COLORREF color)
    // 역할: 지정된 단색을 가지는 논리적 브러시를 생성
    pub fn create_solid_brush(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((1, Some(hbrush as i32)))
    }

    // API: HRGN CreateRectRgn(int x1, int y1, int x2, int y2)
    // 역할: 직사각형 영역(Region)을 생성
    pub fn create_rect_rgn(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((4, Some(hrgn as i32)))
    }

    // API: int SelectClipRgn(HDC hdc, HRGN hrgn)
    // 역할: 지정된 영역(Region)을 디바이스 컨텍스트(DC)의 클리핑 영역으로 설정
    pub fn select_clip_rgn(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((2, Some(result)))
    }

    // API: int CombineRgn(HRGN hrgnDest, HRGN hrgnSrc1, HRGN hrgnSrc2, int fnCombine)
    // 역할: 두 영역(Region)을 결합하여 새로운 영역을 생성
    pub fn combine_rgn(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hrgn_dest = uc.read_arg(0);
        let hrgn_src1 = uc.read_arg(1);
        let hrgn_src2 = uc.read_arg(2);
        let fn_combine = uc.read_arg(3);
        let ctx = uc.get_data();
        let mut result = 0;
        let mut gdi_objects = ctx.gdi_objects.lock().unwrap();
        let region1 = if let Some(GdiObject::Region { left, top, right, bottom }) = gdi_objects.get(&hrgn_src1) {
            Some((*left, *top, *right, *bottom))
        } else {
            None
        };
        let region2 = if let Some(GdiObject::Region { left, top, right, bottom }) = gdi_objects.get(&hrgn_src2) {
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
        Some((4, Some(result)))
    }

    // API: BOOL EqualRgn(HRGN hrgn1, HRGN hrgn2)
    // 역할: 두 영역(Region)이 동일한지 확인
    pub fn equal_rgn(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hrgn1 = uc.read_arg(0);
        let hrgn2 = uc.read_arg(1);
        let ctx = uc.get_data();
        let mut result = 0;
        let gdi_objects = ctx.gdi_objects.lock().unwrap();
        let region1 = if let Some(GdiObject::Region { left, top, right, bottom }) = gdi_objects.get(&hrgn1) {
            Some((*left, *top, *right, *bottom))
        } else {
            None
        };
        let region2 = if let Some(GdiObject::Region { left, top, right, bottom }) = gdi_objects.get(&hrgn2) {
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
        Some((2, Some(result)))
    }

    // API: int GetRgnBox(HRGN hrgn, LPRECT lprc)
    // 역할: 영역(Region)의 경계 사각형(Bounding Rectangle)을 가져옴
    pub fn get_rgn_box(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((2, Some(result)))
    }

    // API: int SetDIBitsToDevice(HDC hdc, int xDest, int yDest, DWORD dwWidth, DWORD dwHeight, int xSrc, int ySrc, UINT uStartScan, UINT cScans, const VOID *lpBits, const BITMAPINFO *lpBitsInfo, UINT uUsage)
    // 역할: DIB(Device-Independent Bitmap) 데이터를 디바이스 컨텍스트(DC)의 지정된 위치에 픽셀로 설정
    pub fn set_dib_its_to_device(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        let u_usage = uc.read_arg(11);

        crate::emu_log!(
            "[GDI32] SetDIBitsToDevice({:#x}, {}, {}, {}, {}, {}, {}, {}, {}, {:#x}, {:#x}, {}) -> int {}",
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
            lp_bits_info,
            u_usage,
            c_scans
        );
        Some((12, Some(c_scans as i32)))
    }

    // API: int StretchDIBits(HDC hdc, int xDest, int yDest, int nDestWidth, int nDestHeight, int xSrc, int ySrc, int nSrcWidth, int nSrcHeight, const VOID *lpBits, const BITMAPINFO *lpBitsInfo, UINT uUsage, DWORD rop)
    // 역할: DIB(Device-Independent Bitmap) 데이터를 디바이스 컨텍스트(DC)의 지정된 위치에 픽셀로 설정
    pub fn stretch_dib_its(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hdc = uc.read_arg(0);
        let x_dest = uc.read_arg(1) as i32;
        let y_dest = uc.read_arg(2) as i32;
        let n_dest_width = uc.read_arg(3);
        let n_dest_height = uc.read_arg(4);
        let x_src = uc.read_arg(5) as i32;
        let y_src = uc.read_arg(6) as i32;
        let n_src_width = uc.read_arg(7);
        let n_src_height = uc.read_arg(8);
        let lp_bits = uc.read_arg(9);
        let lp_bits_info = uc.read_arg(10);
        let u_usage = uc.read_arg(11);
        let rop = uc.read_arg(12);
        crate::emu_log!(
            "[GDI32] StretchDIBits({:#x}, {}, {}, {}, {}, {}, {}, {}, {}, {:#x}, {:#x}, {}, {:#x}) -> int 1",
            hdc,
            x_dest,
            y_dest,
            n_dest_width,
            n_dest_height,
            x_src,
            y_src,
            n_src_width,
            n_src_height,
            lp_bits,
            lp_bits_info,
            u_usage,
            rop
        );
        Some((13, Some(1)))
    }

    // API: int SetStretchBltMode(HDC hdc, int mode)
    // 역할: 디바이스 컨텍스트(DC)의 스트레치 블릿(StretchBlt) 모드를 설정
    pub fn set_stretch_blt_mode(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
        let hdc = uc.read_arg(0);
        let mode = uc.read_arg(1);
        crate::emu_log!("[GDI32] SetStretchBltMode({:#x}, {}) -> int 1", hdc, mode);
        Some((2, Some(1)))
    }

    // API: BOOL BitBlt(HDC hdcDest, int xDest, int yDest, int nDestWidth, int nDestHeight, HDC hdcSrc, int xSrc, int ySrc, DWORD rop)
    // 역할: 디바이스 컨텍스트(DC)의 지정된 위치에 픽셀로 설정
    pub fn bit_blt(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
                    let gdi_objects = uc.get_data().gdi_objects.lock().unwrap();
                    if let (
                        Some(GdiObject::Bitmap {
                            width: dw,
                            height: dh,
                            pixels: dp,
                        }),
                        Some(GdiObject::Bitmap {
                            width: sw,
                            height: sh,
                            pixels: sp,
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
        Some((9, Some(1)))
    }

    // API: BOOL Rectangle(HDC hdc, int left, int top, int right, int bottom)
    // 역할: 현재 펜과 브러시를 사용하여 직사각형을 그림
    pub fn rectangle(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((5, Some(1)))
    }

    // API: BOOL MoveToEx(HDC hdc, int x, int y, LPPOINT lppt)
    // 역할: 현재 그리기 위치를 지정된 좌표로 갱신
    pub fn move_to_ex(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((4, Some(1)))
    }

    // API: BOOL LineTo(HDC hdc, int x, int y)
    // 역할: 현재 위치에서 지정된 끝점까지 선을 그림
    pub fn line_to(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((3, Some(1)))
    }

    // API: int SetROP2(HDC hdc, int nROP2)
    // 역할: 디바이스 컨텍스트의 그리기 모드를 설정
    pub fn set_rop2(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((2, Some(old_mode)))
    }

    // API: UINT RealizePalette(HDC hdc)
    // 역할: 디바이스 컨텍스트의 팔레트를 실제 디바이스에 적용
    pub fn realize_palette(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((1, Some(count as i32)))
    }

    // API: HPALETTE SelectPalette(HDC hdc, HPALETTE hpal, BOOL bForceBkgd)
    // 역할: 디바이스 컨텍스트(DC)에 논리적 팔레트를 선택
    pub fn select_palette(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((3, Some(old_pal as i32)))
    }

    // API: HPALETTE CreatePalette(LPLOGPAL lpLogPalette)
    // 역할: 논리적 팔레트를 생성
    pub fn create_palette(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((1, Some(hpal as i32)))
    }

    // API: COLORREF GetPixel(HDC hdc, int x, int y)
    // 역할: 지정된 좌표의 픽셀 색상을 가져옴
    pub fn get_pixel(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((3, Some(color as i32)))
    }

    // API: COLORREF SetPixel(HDC hdc, int x, int y, COLORREF color)
    // 역할: 지정된 좌표의 픽셀 색상을 설정
    pub fn set_pixel(uc: &mut Unicorn<Win32Context>) -> Option<(usize, Option<i32>)> {
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
        Some((4, Some(old_color as i32)))
    }

    /// 함수명 기준 `GDI32.dll` API 구현체
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        callee_result(match func_name {
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
        })
    }
}
