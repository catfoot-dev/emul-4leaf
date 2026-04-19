mod bitmap;
mod dc;
mod drawing;
mod text;

use crate::dll::win32::{ApiHookResult, GdiObject, Win32Context};
use crate::helper::UnicornHelper;
use std::sync::OnceLock;
use unicorn_engine::Unicorn;

pub const BPP: u32 = 24;
const BI_BITFIELDS: u32 = 3;
const BI_ALPHABITFIELDS: u32 = 6;

/// `GDI32.dll` 프록시 구현 모듈
///
/// 그래픽 디바이스 인터페이스(GDI) 객체 (Font, DC, Bitmap 등) 생성/소멸 호출을 백그라운드에서 추적 및 가상화
pub struct GDI32;

impl GDI32 {
    fn normalize_rect(rect: (i32, i32, i32, i32)) -> Option<(i32, i32, i32, i32)> {
        let (left, top, right, bottom) = rect;
        (left < right && top < bottom).then_some((left, top, right, bottom))
    }

    fn intersect_rect(
        a: (i32, i32, i32, i32),
        b: (i32, i32, i32, i32),
    ) -> Option<(i32, i32, i32, i32)> {
        Self::normalize_rect((a.0.max(b.0), a.1.max(b.1), a.2.min(b.2), a.3.min(b.3)))
    }

    fn subtract_rect(
        a: (i32, i32, i32, i32),
        b: (i32, i32, i32, i32),
    ) -> Vec<(i32, i32, i32, i32)> {
        let Some(intersection) = Self::intersect_rect(a, b) else {
            return vec![a];
        };

        let mut rects = Vec::new();
        if let Some(rect) = Self::normalize_rect((a.0, a.1, a.2, intersection.1)) {
            rects.push(rect);
        }
        if let Some(rect) = Self::normalize_rect((a.0, intersection.3, a.2, a.3)) {
            rects.push(rect);
        }
        if let Some(rect) =
            Self::normalize_rect((a.0, intersection.1, intersection.0, intersection.3))
        {
            rects.push(rect);
        }
        if let Some(rect) =
            Self::normalize_rect((intersection.2, intersection.1, a.2, intersection.3))
        {
            rects.push(rect);
        }
        rects
    }

    /// wallpaper 추적 로그를 활성화할지 여부를 반환합니다.
    pub(crate) fn should_trace_wallpaper() -> bool {
        static ENABLED: OnceLock<bool> = OnceLock::new();
        *ENABLED.get_or_init(|| {
            std::env::var("EMUL_WALLPAPER_TRACE")
                .ok()
                .as_deref()
                .is_some_and(|v| v == "1")
        })
    }

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
            Some(GdiObject::Region { rects }) => Some(rects.clone()),
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

    /// 기준 영역에서 여러 사각형을 순서대로 제외한 결과를 계산합니다.
    pub(crate) fn subtract_region_rects(
        region: &[(i32, i32, i32, i32)],
        subtractors: &[(i32, i32, i32, i32)],
    ) -> Vec<(i32, i32, i32, i32)> {
        let mut current = region.to_vec();
        for &sub in subtractors {
            let mut next = Vec::new();
            for rect in current {
                next.extend(Self::subtract_rect(rect, sub));
            }
            current = next;
        }
        current
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
                    stride,
                    bit_count,
                    top_down,
                    palette,
                    red_mask,
                    green_mask,
                    blue_mask,
                    alpha_mask,
                }) => Some((
                    *width,
                    *height,
                    pixels.clone(),
                    *addr,
                    *stride,
                    *bit_count,
                    *top_down,
                    palette.clone(),
                    *red_mask,
                    *green_mask,
                    *blue_mask,
                    *alpha_mask,
                )),
                _ => None,
            }
        };
        let (
            width,
            height,
            pixels_arc,
            addr,
            stored_stride,
            bit_count,
            top_down,
            palette,
            red_mask,
            green_mask,
            blue_mask,
            alpha_mask,
        ) = match bmp_info {
            Some(v) => v,
            None => return,
        };

        let stride = if stored_stride != 0 {
            stored_stride
        } else {
            aligned_stride(width, bit_count)
        };
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
                write_dib_pixel(
                    &mut raw, row_offset, col, color, bit_count, &palette, red_mask, green_mask,
                    blue_mask, alpha_mask,
                );
            }
        }

        drop(pixels);
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
                    stride,
                    bit_count,
                    top_down,
                    palette,
                    red_mask,
                    green_mask,
                    blue_mask,
                    alpha_mask,
                }) => Some((
                    *width,
                    *height,
                    pixels.clone(),
                    *addr,
                    *stride,
                    *bit_count,
                    *top_down,
                    palette.clone(),
                    *red_mask,
                    *green_mask,
                    *blue_mask,
                    *alpha_mask,
                )),
                _ => None,
            }
        };
        let (
            width,
            height,
            pixels_arc,
            addr,
            stored_stride,
            bit_count,
            top_down,
            palette,
            red_mask,
            green_mask,
            blue_mask,
            alpha_mask,
        ) = match bmp_info {
            Some(v) => v,
            None => return,
        };
        let stride = if stored_stride != 0 {
            stored_stride
        } else {
            aligned_stride(width, bit_count)
        };
        let total_bytes = (stride * height) as usize;
        let raw = uc
            .mem_read_as_vec(addr as u64, total_bytes)
            .unwrap_or_default();
        if raw.is_empty() {
            return;
        }
        let converted = raw_dib_to_pixels(
            &raw, width, height, stride, bit_count, top_down, &palette, red_mask, green_mask,
            blue_mask, alpha_mask,
        );
        let mut pixels = pixels_arc.lock().unwrap();
        if pixels.len() == converted.len() {
            pixels.copy_from_slice(&converted);
        }
        drop(pixels);
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

/// `BITMAPINFOHEADER`에서 추출한 DIB 메타데이터. 게스트가 `biSizeImage`를
/// 명시적으로 채운 경우 그 값을 신뢰해 스트라이드를 계산하므로 비트맵을
/// 기록·판독할 때 피치가 어긋나는 현상을 예방합니다.
#[derive(Debug, Clone)]
pub(crate) struct DibHeaderInfo {
    pub width: u32,
    pub height: u32,
    pub top_down: bool,
    pub stride: u32,
    pub bit_count: u16,
    pub palette: Vec<u32>,
    pub red_mask: u32,
    pub green_mask: u32,
    pub blue_mask: u32,
    pub alpha_mask: u32,
}

/// 지정한 색심도에 맞는 DWORD 정렬 스캔라인 바이트 수를 계산합니다.
///
/// 모든 DIB 스트라이드 계산의 단일 진입점입니다. `bit_count`는 실제 비트맵의 색심도를
/// 반영해야 하며, 24bpp를 하드코딩하면 1/4/8/16bpp 팔레트 DIB에서 스트라이드가
/// 잘못 계산되어 복원 시 색상 밴드 노이즈가 발생합니다.
pub(crate) fn aligned_stride(width: u32, bit_count: u16) -> u32 {
    let bits_per_line = width.saturating_mul(bit_count.max(1) as u32);
    bits_per_line.div_ceil(32) * 4
}

/// `biSizeImage`가 지정된 경우 `biSizeImage / |height|`를 스트라이드로 삼고,
/// 그렇지 않으면 DWORD 정렬 공식으로 계산합니다.
pub(crate) fn dib_effective_stride(
    width: u32,
    height: u32,
    bi_size_image: u32,
    bit_count: u16,
) -> u32 {
    if bi_size_image != 0 && height != 0 {
        let candidate = bi_size_image / height;
        // biSizeImage가 비정상적으로 작은 경우(예: 압축 DIB나 0이 아닌 잘못된 값) 폴백.
        let min_bytes = width.saturating_mul(bit_count.max(1) as u32).div_ceil(8);
        if candidate >= min_bytes {
            return candidate;
        }
    }
    aligned_stride(width, bit_count)
}

/// 게스트 메모리에서 `BITMAPINFO`를 읽어 스트라이드 / 팔레트를 포함한 메타데이터를 반환합니다.
///
/// `biSize` 필드로부터 팔레트 오프셋(`bmiColors`)을 계산하므로 표준 `BITMAPINFOHEADER`(40바이트)뿐
/// 아니라 `BITMAPV4HEADER`(108) / `BITMAPV5HEADER`(124) 확장 헤더도 올바르게 처리합니다.
pub(crate) fn read_dib_header(uc: &Unicorn<Win32Context>, bmi_addr: u32) -> DibHeaderInfo {
    use crate::helper::UnicornHelper;
    let bi_size = uc.read_u32(bmi_addr as u64).max(40);
    let width = uc.read_u32(bmi_addr as u64 + 4).max(1);
    let raw_height = uc.read_u32(bmi_addr as u64 + 8);
    let (height, top_down) = if raw_height > 0x7FFFFFFF {
        (raw_height.wrapping_neg().max(1), true)
    } else {
        (raw_height.max(1), false)
    };
    let bit_count = uc.read_u16(bmi_addr as u64 + 14).max(1);
    let compression = uc.read_u32(bmi_addr as u64 + 16);
    let bi_size_image = uc.read_u32(bmi_addr as u64 + 20);
    let colors_used = uc.read_u32(bmi_addr as u64 + 32);
    let (red_mask, green_mask, blue_mask, alpha_mask, palette_offset) =
        read_dib_masks(uc, bmi_addr, bi_size, compression);
    let palette = read_dib_palette(uc, palette_offset, bit_count, colors_used, compression);
    let stride = dib_effective_stride(width, height, bi_size_image, bit_count);

    DibHeaderInfo {
        width,
        height,
        top_down,
        stride,
        bit_count,
        palette,
        red_mask,
        green_mask,
        blue_mask,
        alpha_mask,
    }
}

// Helper: 원시 DIB 바이트 배열을 0x00RRGGBB Vec<u32>으로 변환 (팔레트/저색심도, bottom-up 지원)
//
// `stride`는 스캔라인 바이트 수입니다. 일반적으로 CreateDIBSection 시점에 계산된 값을
// 그대로 재사용하며, 0이면 DWORD 정렬 공식을 사용합니다.
#[allow(clippy::too_many_arguments)]
fn raw_dib_to_pixels(
    raw: &[u8],
    width: u32,
    height: u32,
    stride: u32,
    bit_count: u16,
    top_down: bool,
    palette: &[u32],
    red_mask: u32,
    green_mask: u32,
    blue_mask: u32,
    alpha_mask: u32,
) -> Vec<u32> {
    let stride = if stride != 0 {
        stride
    } else {
        aligned_stride(width, bit_count)
    };
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
            let color = read_dib_pixel(
                raw, row_offset, col, bit_count, palette, red_mask, green_mask, blue_mask,
                alpha_mask,
            );
            if dst_idx < pixels.len() {
                pixels[dst_idx] = color;
            }
        }
    }
    pixels
}

/// DIB 헤더에서 bitfield 마스크 위치와 값을 해석합니다.
fn read_dib_masks(
    uc: &Unicorn<Win32Context>,
    bmi_addr: u32,
    bi_size: u32,
    compression: u32,
) -> (u32, u32, u32, u32, u64) {
    if compression != BI_BITFIELDS && compression != BI_ALPHABITFIELDS {
        return (0, 0, 0, 0, bmi_addr as u64 + bi_size as u64);
    }

    let mask_base = if bi_size >= 52 {
        bmi_addr as u64 + 40
    } else {
        bmi_addr as u64 + bi_size as u64
    };
    let red_mask = uc.read_u32(mask_base);
    let green_mask = uc.read_u32(mask_base + 4);
    let blue_mask = uc.read_u32(mask_base + 8);
    let alpha_mask = if compression == BI_ALPHABITFIELDS || bi_size >= 56 {
        uc.read_u32(mask_base + 12)
    } else {
        0
    };
    let palette_offset = if bi_size >= 52 {
        bmi_addr as u64 + bi_size as u64
    } else {
        mask_base + if alpha_mask != 0 { 16 } else { 12 }
    };

    (red_mask, green_mask, blue_mask, alpha_mask, palette_offset)
}

/// 팔레트 기반 DIB의 RGBQUAD 테이블을 읽어 `0x00RRGGBB`로 변환합니다.
fn read_dib_palette(
    uc: &Unicorn<Win32Context>,
    palette_offset: u64,
    bit_count: u16,
    colors_used: u32,
    compression: u32,
) -> Vec<u32> {
    if bit_count > 8 || compression == BI_BITFIELDS || compression == BI_ALPHABITFIELDS {
        return Vec::new();
    }

    let entry_count = if colors_used != 0 {
        colors_used
    } else {
        1u32 << bit_count
    };
    let mut palette = Vec::with_capacity(entry_count as usize);
    for index in 0..entry_count {
        let entry_addr = palette_offset + (index as u64 * 4);
        let b = uc.read_u8(entry_addr) as u32;
        let g = uc.read_u8(entry_addr + 1) as u32;
        let r = uc.read_u8(entry_addr + 2) as u32;
        palette.push((r << 16) | (g << 8) | b);
    }

    palette
}

/// 지정한 비트필드 마스크에서 단일 채널 값을 8비트로 정규화합니다.
fn decode_masked_component(pixel: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let value = (pixel & mask) >> shift;
    let max = mask >> shift;
    if max == 0 {
        0
    } else {
        ((value * 255 + (max / 2)) / max) as u8
    }
}

/// 지정한 마스크에 맞춰 8비트 채널 값을 원본 비트필드 폭으로 축소합니다.
fn encode_masked_component(value: u8, mask: u32) -> u32 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let max = mask >> shift;
    ((u32::from(value) * max + 127) / 255) << shift
}

/// RGB 색상과 팔레트 사이의 가장 가까운 색 인덱스를 찾습니다.
fn nearest_palette_index(palette: &[u32], color: u32) -> u8 {
    let target_r = ((color >> 16) & 0xFF) as i32;
    let target_g = ((color >> 8) & 0xFF) as i32;
    let target_b = (color & 0xFF) as i32;
    let mut best_index = 0usize;
    let mut best_score = i64::MAX;

    for (index, entry) in palette.iter().enumerate() {
        let dr = target_r - ((entry >> 16) & 0xFF) as i32;
        let dg = target_g - ((entry >> 8) & 0xFF) as i32;
        let db = target_b - (entry & 0xFF) as i32;
        let score = i64::from(dr * dr + dg * dg + db * db);
        if score < best_score {
            best_score = score;
            best_index = index;
        }
    }

    best_index.min(u8::MAX as usize) as u8
}

/// DIB 한 픽셀을 읽어 `0x00RRGGBB`로 변환합니다.
#[allow(clippy::too_many_arguments)]
fn read_dib_pixel(
    raw: &[u8],
    row_offset: usize,
    col: usize,
    bit_count: u16,
    palette: &[u32],
    red_mask: u32,
    green_mask: u32,
    blue_mask: u32,
    alpha_mask: u32,
) -> u32 {
    match bit_count {
        1 => {
            let idx = row_offset + (col / 8);
            let bit = 7 - (col % 8);
            let palette_index = raw.get(idx).map(|byte| (byte >> bit) & 0x01).unwrap_or(0) as usize;
            palette.get(palette_index).copied().unwrap_or({
                if palette_index == 0 {
                    0x000000
                } else {
                    0x00FF_FFFF
                }
            })
        }
        4 => {
            let idx = row_offset + (col / 2);
            let palette_index = raw
                .get(idx)
                .map(|byte| {
                    if col.is_multiple_of(2) {
                        (byte >> 4) & 0x0F
                    } else {
                        byte & 0x0F
                    }
                })
                .unwrap_or(0) as usize;
            palette.get(palette_index).copied().unwrap_or_else(|| {
                let gray = (palette_index as u32 * 17) & 0xFF;
                (gray << 16) | (gray << 8) | gray
            })
        }
        8 => {
            let idx = row_offset + col;
            let palette_index = raw.get(idx).copied().unwrap_or(0) as usize;
            palette.get(palette_index).copied().unwrap_or_else(|| {
                let gray = palette_index as u32;
                (gray << 16) | (gray << 8) | gray
            })
        }
        16 => {
            let idx = row_offset + col * 2;
            if idx + 1 >= raw.len() {
                return 0;
            }
            let pixel = u16::from_le_bytes([raw[idx], raw[idx + 1]]) as u32;
            let (red_mask, green_mask, blue_mask, alpha_mask) =
                if red_mask != 0 && green_mask != 0 && blue_mask != 0 {
                    (red_mask, green_mask, blue_mask, alpha_mask)
                } else {
                    (0x7C00, 0x03E0, 0x001F, 0)
                };
            let r = decode_masked_component(pixel, red_mask) as u32;
            let g = decode_masked_component(pixel, green_mask) as u32;
            let b = decode_masked_component(pixel, blue_mask) as u32;
            let _a = decode_masked_component(pixel, alpha_mask);
            (r << 16) | (g << 8) | b
        }
        24 => {
            let idx = row_offset + col * 3;
            if idx + 2 >= raw.len() {
                return 0;
            }
            let b = raw[idx] as u32;
            let g = raw[idx + 1] as u32;
            let r = raw[idx + 2] as u32;
            (r << 16) | (g << 8) | b
        }
        32 => {
            let idx = row_offset + col * 4;
            if idx + 3 >= raw.len() {
                return 0;
            }
            let pixel = u32::from_le_bytes([raw[idx], raw[idx + 1], raw[idx + 2], raw[idx + 3]]);
            let (red_mask, green_mask, blue_mask, alpha_mask) =
                if red_mask != 0 && green_mask != 0 && blue_mask != 0 {
                    (red_mask, green_mask, blue_mask, alpha_mask)
                } else {
                    (0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000)
                };
            let r = decode_masked_component(pixel, red_mask) as u32;
            let g = decode_masked_component(pixel, green_mask) as u32;
            let b = decode_masked_component(pixel, blue_mask) as u32;
            let _a = decode_masked_component(pixel, alpha_mask);
            (r << 16) | (g << 8) | b
        }
        _ => 0,
    }
}

/// DIB 한 픽셀에 `0x00RRGGBB` 색상을 기록합니다.
#[allow(clippy::too_many_arguments)]
fn write_dib_pixel(
    raw: &mut [u8],
    row_offset: usize,
    col: usize,
    color: u32,
    bit_count: u16,
    palette: &[u32],
    red_mask: u32,
    green_mask: u32,
    blue_mask: u32,
    alpha_mask: u32,
) {
    match bit_count {
        1 => {
            let idx = row_offset + (col / 8);
            if idx >= raw.len() {
                return;
            }
            let bit = 7 - (col % 8);
            let palette_index = if palette.is_empty() {
                (((color >> 16) & 0xFF) + ((color >> 8) & 0xFF) + (color & 0xFF) >= 0x180) as u8
            } else {
                nearest_palette_index(palette, color) & 0x01
            };
            if palette_index != 0 {
                raw[idx] |= 1 << bit;
            } else {
                raw[idx] &= !(1 << bit);
            }
        }
        4 => {
            let idx = row_offset + (col / 2);
            if idx >= raw.len() {
                return;
            }
            let palette_index = if palette.is_empty() {
                ((((color >> 16) & 0xFF) + ((color >> 8) & 0xFF) + (color & 0xFF)) / 3 / 17) as u8
            } else {
                nearest_palette_index(palette, color) & 0x0F
            };
            if col.is_multiple_of(2) {
                raw[idx] = (raw[idx] & 0x0F) | (palette_index << 4);
            } else {
                raw[idx] = (raw[idx] & 0xF0) | (palette_index & 0x0F);
            }
        }
        8 => {
            let idx = row_offset + col;
            if idx >= raw.len() {
                return;
            }
            raw[idx] = if palette.is_empty() {
                (((color >> 16) & 0xFF) + ((color >> 8) & 0xFF) + (color & 0xFF)) as u8 / 3
            } else {
                nearest_palette_index(palette, color)
            };
        }
        16 => {
            let idx = row_offset + col * 2;
            if idx + 1 >= raw.len() {
                return;
            }
            let (red_mask, green_mask, blue_mask, alpha_mask) =
                if red_mask != 0 && green_mask != 0 && blue_mask != 0 {
                    (red_mask, green_mask, blue_mask, alpha_mask)
                } else {
                    (0x7C00, 0x03E0, 0x001F, 0)
                };
            let pixel = encode_masked_component(((color >> 16) & 0xFF) as u8, red_mask)
                | encode_masked_component(((color >> 8) & 0xFF) as u8, green_mask)
                | encode_masked_component((color & 0xFF) as u8, blue_mask)
                | encode_masked_component(0xFF, alpha_mask);
            raw[idx..idx + 2].copy_from_slice(&(pixel as u16).to_le_bytes());
        }
        24 => {
            let idx = row_offset + col * 3;
            if idx + 2 >= raw.len() {
                return;
            }
            raw[idx] = (color & 0xFF) as u8;
            raw[idx + 1] = ((color >> 8) & 0xFF) as u8;
            raw[idx + 2] = ((color >> 16) & 0xFF) as u8;
        }
        32 => {
            let idx = row_offset + col * 4;
            if idx + 3 >= raw.len() {
                return;
            }
            let (red_mask, green_mask, blue_mask, alpha_mask) =
                if red_mask != 0 && green_mask != 0 && blue_mask != 0 {
                    (red_mask, green_mask, blue_mask, alpha_mask)
                } else {
                    (0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000)
                };
            let pixel = encode_masked_component(((color >> 16) & 0xFF) as u8, red_mask)
                | encode_masked_component(((color >> 8) & 0xFF) as u8, green_mask)
                | encode_masked_component((color & 0xFF) as u8, blue_mask)
                | encode_masked_component(0xFF, alpha_mask);
            raw[idx..idx + 4].copy_from_slice(&pixel.to_le_bytes());
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{dib_effective_stride, raw_dib_to_pixels};

    #[test]
    fn paletted_8bpp_dib_is_decoded_with_palette() {
        let palette = vec![0x000000, 0x00FF0000, 0x0000FF00, 0x000000FF];
        let raw = vec![1u8, 2u8, 3u8, 0u8];
        let pixels = raw_dib_to_pixels(&raw, 4, 1, 4, 8, true, &palette, 0, 0, 0, 0);
        assert_eq!(pixels, vec![0x00FF0000, 0x0000FF00, 0x000000FF, 0x000000]);
    }

    #[test]
    fn stride_uses_actual_bit_count_for_8bpp() {
        assert_eq!(dib_effective_stride(28, 34, 0, 8), 28);
        assert_eq!(dib_effective_stride(29, 34, 0, 8), 32);
    }
}
