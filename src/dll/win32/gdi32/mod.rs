mod bitmap;
mod dc;
mod drawing;
mod text;

use crate::dll::win32::{ApiHookResult, GdiObject, Win32Context};
use unicorn_engine::Unicorn;

/// `GDI32.dll` 프록시 구현 모듈
///
/// 그래픽 디바이스 인터페이스(GDI) 객체 (Font, DC, Bitmap 등) 생성/소멸 호출을 백그라운드에서 추적 및 가상화
pub struct GDI32;

impl GDI32 {
    /// 지정된 DC에 선택된 클리핑 영역 사각형 목록을 반환합니다.
    pub(crate) fn clip_rects_for_hdc(
        uc: &Unicorn<Win32Context>,
        hdc: u32,
    ) -> Option<Vec<(i32, i32, i32, i32)>> {
        let gdi = uc.get_data().gdi_objects.lock().unwrap();
        let selected_region = match gdi.get(&hdc) {
            Some(GdiObject::Dc {
                selected_region, ..
            }) if *selected_region != 0 => *selected_region,
            _ => return None,
        };

        match gdi.get(&selected_region) {
            Some(GdiObject::Region { rects }) if !rects.is_empty() => Some(rects.clone()),
            _ => None,
        }
    }

    /// 대상 좌표가 클리핑 영역 안에 있는지 검사합니다.
    pub(crate) fn point_in_clip_rects(
        clip_rects: &Option<Vec<(i32, i32, i32, i32)>>,
        x: i32,
        y: i32,
    ) -> bool {
        clip_rects.as_ref().is_none_or(|rects| {
            rects
                .iter()
                .any(|&(left, top, right, bottom)| x >= left && x < right && y >= top && y < bottom)
        })
    }

    /// 사각형을 클리핑 영역과 교차한 결과를 반환합니다.
    pub(crate) fn intersect_rect_with_clip_rects(
        clip_rects: &Option<Vec<(i32, i32, i32, i32)>>,
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    ) -> Vec<(i32, i32, i32, i32)> {
        match clip_rects {
            Some(rects) => rects
                .iter()
                .filter_map(|&(cl, ct, cr, cb)| {
                    let il = left.max(cl);
                    let it = top.max(ct);
                    let ir = right.min(cr);
                    let ib = bottom.min(cb);
                    (il < ir && it < ib).then_some((il, it, ir, ib))
                })
                .collect(),
            None => vec![(left, top, right, bottom)],
        }
    }

    /// DIBSection 픽셀 버퍼를 게스트가 받은 `bits` 메모리로 역동기화합니다.
    pub(crate) fn flush_dib_pixels_to_memory(uc: &mut Unicorn<Win32Context>, hbmp: u32) {
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
        let mut raw = vec![0u8; (stride * height) as usize];
        let pixels = pixels_arc.lock().unwrap();

        for row in 0..height as usize {
            let dst_row = if top_down {
                row
            } else {
                height as usize - 1 - row
            };
            let row_offset = dst_row * stride as usize;
            for col in 0..width as usize {
                let color = pixels[row * width as usize + col];
                let r = ((color >> 16) & 0xFF) as u8;
                let g = ((color >> 8) & 0xFF) as u8;
                let b = (color & 0xFF) as u8;

                match bpp {
                    24 => {
                        let idx = row_offset + col * 3;
                        if idx + 2 < raw.len() {
                            raw[idx] = b;
                            raw[idx + 1] = g;
                            raw[idx + 2] = r;
                        }
                    }
                    _ => {
                        let idx = row_offset + col * 4;
                        if idx + 3 < raw.len() {
                            raw[idx] = b;
                            raw[idx + 1] = g;
                            raw[idx + 2] = r;
                            raw[idx + 3] = 0xFF;
                        }
                    }
                }
            }
        }

        let _ = uc.mem_write(addr as u64, &raw);
    }

    // Helper: DIBSection 비트맵의 emulated memory 데이터를 GdiObject::Bitmap.pixels Vec에 동기화
    pub(crate) fn sync_dib_pixels(uc: &mut Unicorn<Win32Context>, hbmp: u32) {
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
        let converted = raw_dib_to_pixels(&raw, width, height, bpp, top_down);
        let mut pixels = pixels_arc.lock().unwrap();
        if pixels.len() == converted.len() {
            pixels.copy_from_slice(&converted);
        }
    }

    /// 함수명 기준 `GDI32.dll` API 구현체
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        match func_name {
            "CreateCompatibleDC" => dc::create_compatible_dc(uc),
            "DeleteDC" => dc::delete_dc(uc),
            "SelectObject" => dc::select_object(uc),
            "DeleteObject" => dc::delete_object(uc),
            "GetStockObject" => dc::get_stock_object(uc),
            "GetDeviceCaps" => dc::get_device_caps(uc),
            "GetObject" => dc::get_object(uc),
            "CreateDIBSection" => bitmap::create_dib_section(uc),
            "CreateCompatibleBitmap" => bitmap::create_compatible_bitmap(uc),
            "CreateBitmap" => bitmap::create_bitmap(uc),
            "BitBlt" => bitmap::bit_blt(uc),
            "StretchBlt" => bitmap::stretch_blt(uc),
            "SetDIBitsToDevice" => bitmap::set_dib_its_to_device(uc),
            "StretchDIBits" => bitmap::stretch_dib_its(uc),
            "SetStretchBltMode" => bitmap::set_stretch_blt_mode(uc),
            "CreateFontIndirectA" => text::create_font_indirect_a(uc),
            "CreateFontA" => text::create_font_a(uc),
            "GetTextMetricsA" => text::get_text_metrics_a(uc),
            "GetTextExtentPoint32A" => text::get_text_extent_point32_a(uc),
            "GetTextExtentPointA" => text::get_text_extent_point_a(uc),
            "GetCharWidthA" => text::get_char_width_a(uc),
            "TextOutA" => text::text_out_a(uc),
            "SetBkMode" => drawing::set_bk_mode(uc),
            "GetBkMode" => drawing::get_bk_mode(uc),
            "SetBkColor" => drawing::set_bk_color(uc),
            "GetBkColor" => drawing::get_bk_color(uc),
            "SetTextColor" => drawing::set_text_color(uc),
            "GetTextColor" => drawing::get_text_color(uc),
            "CreatePen" => drawing::create_pen(uc),
            "CreateSolidBrush" => drawing::create_solid_brush(uc),
            "CreateRectRgn" => drawing::create_rect_rgn(uc),
            "SelectClipRgn" => drawing::select_clip_rgn(uc),
            "CombineRgn" => drawing::combine_rgn(uc),
            "EqualRgn" => drawing::equal_rgn(uc),
            "GetRgnBox" => drawing::get_rgn_box(uc),
            "Rectangle" => drawing::rectangle(uc),
            "MoveToEx" => drawing::move_to_ex(uc),
            "LineTo" => drawing::line_to(uc),
            "SetROP2" => drawing::set_rop2(uc),
            "RealizePalette" => drawing::realize_palette(uc),
            "SelectPalette" => drawing::select_palette(uc),
            "CreatePalette" => drawing::create_palette(uc),
            "PatBlt" => drawing::pat_blt(uc),
            "GetPixel" => drawing::get_pixel(uc),
            "SetPixel" => drawing::set_pixel(uc),
            _ => {
                crate::emu_log!("[!] GDI32 Unhandled: {}", func_name);
                None
            }
        }
    }
}

// Helper: 원시 DIB 바이트 배열을 0x00RRGGBB Vec<u32>으로 변환 (BGR/BGRA, bottom-up 지원)
fn raw_dib_to_pixels(raw: &[u8], width: u32, height: u32, bpp: u32, top_down: bool) -> Vec<u32> {
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
