use crate::{
    dll::win32::{ApiHookResult, GdiObject, Win32Context},
    helper::UnicornHelper,
    ui::gdi_renderer::GdiRenderer,
};
use std::sync::{Arc, Mutex};
use unicorn_engine::Unicorn;

use super::{BPP, GDI32, aligned_stride, dib_effective_stride, read_dib_header};

fn apply_bitmap_rop(dst_val: u32, src_val: u32, rop: u32) -> u32 {
    match rop {
        0x008800C6 => (dst_val & src_val) & 0x00FF_FFFF,
        0x00EE0086 => (dst_val | src_val) & 0x00FF_FFFF,
        0x00660046 => (dst_val ^ src_val) & 0x00FF_FFFF,
        _ => src_val & 0x00FF_FFFF,
    }
}

// API: HBITMAP CreateDIBSection(HDC hdc, const BITMAPINFO *pbmi, UINT usage, VOID **ppvBits, HANDLE hSection, DWORD offset)
// 역할: 애플리케이션이 직접 쓸 수 있는 DIB(장치 독립적 비트맵)를 생성
pub(super) fn create_dib_section(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hdc = uc.read_arg(0);
    let bmi_addr = uc.read_arg(1);
    let usage = uc.read_arg(2);
    let bits_ptr_addr = uc.read_arg(3);
    let hsection = uc.read_arg(4);
    let offset = uc.read_arg(5);

    let header = read_dib_header(uc, bmi_addr);
    let width = header.width;
    let height = header.height;
    let top_down = header.top_down;
    let stride = header.stride;
    let bit_count = header.bit_count;
    let bmp_size = stride * height;

    let bits_addr = uc.malloc(bmp_size as usize);
    let _ = uc.mem_write(bits_addr, &vec![0u8; bmp_size as usize]);
    if bits_ptr_addr != 0 {
        uc.write_u32(bits_ptr_addr as u64, bits_addr as u32);
    }
    let pixel_count = (width as usize).saturating_mul(height as usize);
    let pixels_vec = vec![0u32; pixel_count];
    debug_assert_eq!(
        pixels_vec.len(),
        pixel_count,
        "DIB section pixel buffer length must equal width*height"
    );
    let pixels = Arc::new(Mutex::new(pixels_vec));
    let ctx = uc.get_data();
    let hbmp = ctx.alloc_handle();
    ctx.gdi_objects.lock().unwrap().insert(
        hbmp,
        GdiObject::Bitmap {
            width,
            height,
            pixels,
            bits_addr: Some(bits_addr as u32),
            stride,
            bit_count,
            top_down,
            palette: header.palette,
            red_mask: header.red_mask,
            green_mask: header.green_mask,
            blue_mask: header.blue_mask,
            alpha_mask: header.alpha_mask,
        },
    );
    crate::emu_log!(
        "[GDI32] CreateDIBSection({:#x}, {:#x}, {}, {:#x}, {:#x}, {}) -> HBITMAP {:#x} ({}x{} stride={} top_down={})",
        hdc,
        bmi_addr,
        usage,
        bits_ptr_addr,
        hsection,
        offset,
        hbmp,
        width,
        height,
        stride,
        top_down
    );
    Some(ApiHookResult::callee(6, Some(hbmp as i32)))
}

// API: HBITMAP CreateCompatibleBitmap(HDC hdc, int cx, int cy)
// 역할: 지정된 디바이스 컨텍스트의 현재 설정과 호환되는 비트맵을 만듦
pub(super) fn create_compatible_bitmap(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hdc = uc.read_arg(0);
    let width = uc.read_arg(1);
    let height = uc.read_arg(2);
    let pixel_count = (width as usize).saturating_mul(height as usize);
    let pixels_vec = vec![0u32; pixel_count];
    debug_assert_eq!(
        pixels_vec.len(),
        pixel_count,
        "compatible bitmap pixel buffer length must equal width*height"
    );
    let pixels = Arc::new(Mutex::new(pixels_vec));
    let ctx = uc.get_data();
    let hbmp = ctx.alloc_handle();
    ctx.gdi_objects.lock().unwrap().insert(
        hbmp,
        GdiObject::Bitmap {
            width,
            height,
            pixels,
            bits_addr: None,
            stride: aligned_stride(width, BPP as u16),
            bit_count: BPP as u16,
            top_down: false,
            palette: Vec::new(),
            red_mask: 0,
            green_mask: 0,
            blue_mask: 0,
            alpha_mask: 0,
        },
    );
    crate::emu_log!(
        "[GDI32] CreateCompatibleBitmap({:#x}, {}, {}) -> HBITMAP {:#x}",
        hdc,
        width,
        height,
        hbmp,
    );
    Some(ApiHookResult::callee(3, Some(hbmp as i32)))
}

// API: HBITMAP CreateBitmap(int nWidth, int nHeight, UINT nPlanes, UINT nBitCount, const VOID *lpBits)
// 역할: 지정된 크기와 색 형식의 비트맵을 생성
pub(super) fn create_bitmap(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let width = uc.read_arg(0).max(1);
    let height = uc.read_arg(1).max(1);
    let _n_planes = uc.read_arg(2);
    let bpp = uc.read_arg(3).max(1);
    let lp_bits = uc.read_arg(4);

    let pixel_count = (width as usize).saturating_mul(height as usize);
    let pixels_vec = vec![0u32; pixel_count];
    debug_assert_eq!(
        pixels_vec.len(),
        pixel_count,
        "CreateBitmap pixel buffer length must equal width*height"
    );
    let pixels = Arc::new(Mutex::new(pixels_vec));
    let stride = dib_effective_stride(width, height, 0, bpp as u16);

    // 초기 비트 데이터가 있으면 읽어서 변환
    if lp_bits != 0 {
        let total_bytes = (stride * height) as usize;
        let raw = uc
            .mem_read_as_vec(lp_bits as u64, total_bytes)
            .unwrap_or_default();
        if !raw.is_empty() {
            let converted = super::raw_dib_to_pixels(
                &raw,
                width,
                height,
                stride,
                bpp as u16,
                false,
                &[],
                0,
                0,
                0,
                0,
            );
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
            stride,
            bit_count: bpp as u16,
            top_down: false,
            palette: Vec::new(),
            red_mask: 0,
            green_mask: 0,
            blue_mask: 0,
            alpha_mask: 0,
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

// API: BOOL BitBlt(HDC hdcDest, int xDest, int yDest, int nDestWidth, int nDestHeight, HDC hdcSrc, int xSrc, int ySrc, DWORD rop)
// 역할: 디바이스 컨텍스트(DC)의 지정된 위치에 픽셀로 설정
pub(super) fn bit_blt(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hdc_dest = uc.read_arg(0);
    let x_dest = uc.read_arg(1) as i32;
    let y_dest = uc.read_arg(2) as i32;
    let n_dest_width = uc.read_arg(3);
    let n_dest_height = uc.read_arg(4);
    let hdc_src = uc.read_arg(5);
    let x_src = uc.read_arg(6) as i32;
    let y_src = uc.read_arg(7) as i32;
    let rop = uc.read_arg(8);

    // SRCCOPY, SRCAND, SRCPAINT, SRCINVERT 등 비트맵 기반 ROP 처리
    if rop == 0x00CC0020 || rop == 0x008800C6 || rop == 0x00EE0086 || rop == 0x00660046 {
        let mut draw_params = None;
        {
            let gdi_objects = uc.get_data().gdi_objects.lock().unwrap();
            if let Some(GdiObject::Dc {
                selected_bitmap: hbmp_dest,
                associated_window: hwnd_dest,
                origin_x: dst_origin_x,
                origin_y: dst_origin_y,
                ..
            }) = gdi_objects.get(&hdc_dest)
                && let Some(GdiObject::Dc {
                    selected_bitmap: hbmp_src,
                    origin_x: src_origin_x,
                    origin_y: src_origin_y,
                    ..
                }) = gdi_objects.get(&hdc_src)
            {
                draw_params = Some((
                    *hbmp_dest,
                    *hbmp_src,
                    *hwnd_dest,
                    *dst_origin_x,
                    *dst_origin_y,
                    *src_origin_x,
                    *src_origin_y,
                ));
            }
        }

        if let Some((
            hbmp_dest,
            hbmp_src,
            hwnd_dest,
            dst_origin_x,
            dst_origin_y,
            src_origin_x,
            src_origin_y,
        )) = draw_params
            && hbmp_dest != 0
            && hbmp_src != 0
        {
            GDI32::sync_dib_pixels(uc, hbmp_dest);
            GDI32::sync_dib_pixels(uc, hbmp_src);
            let clip_rects = GDI32::clip_rects_for_hdc(uc, hdc_dest);
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
                for y in 0..n_dest_height as i32 {
                    let sy = y_src + y + src_origin_y;
                    let dy = y_dest + y + dst_origin_y;
                    if sy < 0 || sy >= sh as i32 || dy < 0 || dy >= dh as i32 {
                        continue;
                    }

                    for x in 0..n_dest_width as i32 {
                        let sx = x_src + x + src_origin_x;
                        let dx = x_dest + x + dst_origin_x;
                        if sx < 0 || sx >= sw as i32 || dx < 0 || dx >= dw as i32 {
                            continue;
                        }
                        if !GDI32::point_in_clip_rects(&clip_rects, dx, dy) {
                            continue;
                        }

                        let src_val = sp[(sy * sw as i32 + sx) as usize] & 0x00FF_FFFF;
                        let dst_idx = (dy as u32 * dw + dx as u32) as usize;
                        let dst_val = dp[dst_idx] & 0x00FF_FFFF;
                        dp[dst_idx] = apply_bitmap_rop(dst_val, src_val, rop);
                    }
                }
                drop(dp);
                drop(sp);
                drop(gdi_objects);
                GDI32::flush_dib_pixels_to_memory(uc, hbmp_dest);
                if hwnd_dest != 0 {
                    uc.get_data()
                        .win_event
                        .lock()
                        .unwrap()
                        .update_window(hwnd_dest);
                }
            }
        }
    } else if rop == 0x00F00021 || rop == 0x00000042 || rop == 0x00FF0062 {
        // PATCOPY, BLACKNESS, WHITENESS (Brush/Solid fill)
        let mut draw_params = None;
        {
            let gdi = uc.get_data().gdi_objects.lock().unwrap();
            if let Some(GdiObject::Dc {
                selected_bitmap,
                selected_brush,
                associated_window,
                origin_x,
                origin_y,
                ..
            }) = gdi.get(&hdc_dest)
            {
                let color = match rop {
                    0x00F00021 => {
                        // PATCOPY
                        if let Some(GdiObject::Brush { color }) = gdi.get(selected_brush) {
                            Some(*color)
                        } else {
                            Some(0x00FFFFFF)
                        }
                    }
                    0x00000042 => Some(0x00000000), // BLACKNESS
                    0x00FF0062 => Some(0x00FFFFFF), // WHITENESS
                    _ => None,
                };
                draw_params = Some((
                    *selected_bitmap,
                    color,
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
            let clip_rects = GDI32::clip_rects_for_hdc(uc, hdc_dest);
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
                    x_dest + origin_x,
                    y_dest + origin_y,
                    x_dest + origin_x + n_dest_width as i32,
                    y_dest + origin_y + n_dest_height as i32,
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

// API: BOOL StretchBlt(HDC hdcDest, int xDest, int yDest, int nDestWidth, int nDestHeight, HDC hdcSrc, int xSrc, int ySrc, int nSrcWidth, int nSrcHeight, DWORD rop)
// 역할: 소스 DC 비트맵을 스케일링하여 대상 DC에 복사
pub(super) fn stretch_blt(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
    let rop = uc.read_arg(10);

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

    if n_src_width == 0 || n_src_height == 0 || n_dest_width == 0 || n_dest_height == 0 {
        return Some(ApiHookResult::callee(11, Some(1)));
    }

    // 소스 비트맵 핸들 추출
    let (hbmp_src, hbmp_dest, hwnd_dest, src_origin_x, src_origin_y, dst_origin_x, dst_origin_y) = {
        let gdi = uc.get_data().gdi_objects.lock().unwrap();
        let (src_bmp, src_origin_x, src_origin_y) = if let Some(GdiObject::Dc {
            selected_bitmap,
            origin_x,
            origin_y,
            ..
        }) = gdi.get(&hdc_src)
        {
            (*selected_bitmap, *origin_x, *origin_y)
        } else {
            (0, 0, 0)
        };
        let (dst_bmp, hwnd, dst_origin_x, dst_origin_y) = if let Some(GdiObject::Dc {
            selected_bitmap,
            associated_window,
            origin_x,
            origin_y,
            ..
        }) = gdi.get(&hdc_dest)
        {
            (*selected_bitmap, *associated_window, *origin_x, *origin_y)
        } else {
            (0, 0, 0, 0)
        };
        (
            src_bmp,
            dst_bmp,
            hwnd,
            src_origin_x,
            src_origin_y,
            dst_origin_x,
            dst_origin_y,
        )
    };

    if hbmp_src == 0 || hbmp_dest == 0 {
        return Some(ApiHookResult::callee(11, Some(1)));
    }

    // DIBSection 동기화
    GDI32::sync_dib_pixels(uc, hbmp_dest);
    GDI32::sync_dib_pixels(uc, hbmp_src);
    let clip_rects = GDI32::clip_rects_for_hdc(uc, hdc_dest);

    let gdi = uc.get_data().gdi_objects.lock().unwrap();
    if rop == 0x00CC0020 || rop == 0x008800C6 || rop == 0x00EE0086 || rop == 0x00660046 {
        // SRCCOPY, SRCAND, SRCPAINT, SRCINVERT
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

            let abs_dw = n_dest_width.abs();
            let abs_dh = n_dest_height.abs();

            for dy in 0..abs_dh {
                let sy = if n_dest_height > 0 {
                    y_src + dy * n_src_height / n_dest_height
                } else {
                    y_src + (abs_dh - 1 - dy) * n_src_height / abs_dh
                };
                let sy = (sy + src_origin_y).clamp(0, sh - 1);

                for dx in 0..abs_dw {
                    let sx = if n_dest_width > 0 {
                        x_src + dx * n_src_width / n_dest_width
                    } else {
                        x_src + (abs_dw - 1 - dx) * n_src_width / abs_dw
                    };
                    let sx = (sx + src_origin_x).clamp(0, sw - 1);

                    let dst_x = x_dest + dx + dst_origin_x;
                    let dst_y = y_dest + dy + dst_origin_y;

                    if dst_x < 0 || dst_y < 0 || dst_x >= dw as i32 || dst_y >= dh as i32 {
                        continue;
                    }
                    if !GDI32::point_in_clip_rects(&clip_rects, dst_x, dst_y) {
                        continue;
                    }
                    let src_idx = (sy * sw + sx) as usize;
                    let dst_idx = (dst_y as u32 * dw + dst_x as u32) as usize;
                    if src_idx < sp.len() && dst_idx < dp.len() {
                        let src_val = sp[src_idx] & 0x00FF_FFFF;
                        let dst_val = dp[dst_idx] & 0x00FF_FFFF;
                        dp[dst_idx] = apply_bitmap_rop(dst_val, src_val, rop);
                    }
                }
            }
        }
    } else if rop == 0x00F00021 || rop == 0x00000042 || rop == 0x00FF0062 {
        // PATCOPY, BLACKNESS, WHITENESS
        let brush_color = {
            let hdc_dest = uc.read_arg(0);
            if let Some(GdiObject::Dc { selected_brush, .. }) = gdi.get(&hdc_dest) {
                match rop {
                    0x00F00021 => {
                        if let Some(GdiObject::Brush { color }) = gdi.get(selected_brush) {
                            Some(*color)
                        } else {
                            Some(0x00FFFFFF)
                        }
                    }
                    0x00000042 => Some(0x00000000),
                    0x00FF0062 => Some(0x00FFFFFF),
                    _ => None,
                }
            } else {
                None
            }
        };

        if let Some(color) = brush_color
            && let Some(GdiObject::Bitmap {
                width: dw,
                height: dh,
                pixels: dp,
                ..
            }) = gdi.get(&hbmp_dest)
        {
            let (dw, dh) = (*dw, *dh);
            let mut dp = dp.lock().unwrap();
            for (left, top, right, bottom) in GDI32::intersect_rect_with_clip_rects(
                &clip_rects,
                x_dest + dst_origin_x,
                y_dest + dst_origin_y,
                x_dest + dst_origin_x + n_dest_width.abs(),
                y_dest + dst_origin_y + n_dest_height.abs(),
            ) {
                GdiRenderer::draw_rect(
                    &mut dp,
                    dw,
                    dh,
                    left,
                    top,
                    right,
                    bottom,
                    None,
                    Some(color),
                );
            }
        }
    }

    drop(gdi);
    GDI32::flush_dib_pixels_to_memory(uc, hbmp_dest);

    if hwnd_dest != 0 {
        uc.get_data()
            .win_event
            .lock()
            .unwrap()
            .update_window(hwnd_dest);
    }
    Some(ApiHookResult::callee(11, Some(1)))
}

// API: int SetDIBitsToDevice(HDC hdc, int xDest, int yDest, DWORD dwWidth, DWORD dwHeight, int xSrc, int ySrc, UINT uStartScan, UINT cScans, const VOID *lpBits, const BITMAPINFO *lpBitsInfo, UINT uUsage)
// 역할: DIB 데이터를 DC의 비트맵에 직접 복사
pub(super) fn set_dib_its_to_device(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
    let header = read_dib_header(uc, lp_bits_info);
    let bmi_width = header.width;
    let top_down = header.top_down;
    let stride = header.stride;

    let total_bytes = (stride * c_scans) as usize;
    let offset_bits = lp_bits as u64 + (u_start_scan * stride) as u64;
    let raw = uc
        .mem_read_as_vec(offset_bits, total_bytes)
        .unwrap_or_default();
    if raw.is_empty() {
        return Some(ApiHookResult::callee(12, Some(c_scans as i32)));
    }

    // c_scans 개 행을 변환 (bottom-up 기준 uStartScan번째 행부터)
    let src_pixels = super::raw_dib_to_pixels(
        &raw,
        bmi_width,
        c_scans,
        stride,
        header.bit_count,
        top_down,
        &header.palette,
        header.red_mask,
        header.green_mask,
        header.blue_mask,
        header.alpha_mask,
    );

    // 대상 DC → 비트맵 찾기
    let (hbmp_dest, hwnd_dest, origin_x, origin_y) = {
        let gdi = uc.get_data().gdi_objects.lock().unwrap();
        if let Some(GdiObject::Dc {
            selected_bitmap,
            associated_window,
            origin_x,
            origin_y,
            ..
        }) = gdi.get(&hdc)
        {
            (*selected_bitmap, *associated_window, *origin_x, *origin_y)
        } else {
            return Some(ApiHookResult::callee(12, Some(c_scans as i32)));
        }
    };

    if hbmp_dest != 0 {
        GDI32::sync_dib_pixels(uc, hbmp_dest);
        let clip_rects = GDI32::clip_rects_for_hdc(uc, hdc);
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
            // SetDIBitsToDevice는 cScans만큼의 scan lines를 복사합니다.
            // uStartScan은 DIB 내에서 시작 scan line index입니다.
            // Win32 GDI coordinates: (xDest, yDest)는 대상의 시작점. uStartScan은 DIB 소스의 시작점.
            let src_dw = bmi_width; // src_pixels의 로우 너비는 bmi_width입니다.
            let src_dh = c_scans;

            // y_src는 DIB 논리 좌표에서의 시작 Y(top-down 변환 후 src_pixels에 그대로 적용).
            // 이전 구현은 y_src를 무시하고 항상 0행부터 읽어 서브-렉트 복사시 잘못된 행을 샘플링했습니다.
            for y in 0..c_scans as i32 {
                let sy = y_src + y;
                let dy = y_dest + y + origin_y;
                if sy < 0 || sy >= src_dh as i32 || dy < 0 || dy >= dh as i32 {
                    continue;
                }

                for x in 0..dw_width as i32 {
                    let sx = x_src + x;
                    let dx = x_dest + x + origin_x;
                    if sx < 0 || sx >= src_dw as i32 || dx < 0 || dx >= dw as i32 {
                        continue;
                    }
                    if !GDI32::point_in_clip_rects(&clip_rects, dx, dy) {
                        continue;
                    }

                    let src_idx = (sy * src_dw as i32 + sx) as usize;
                    let dst_idx = (dy as u32 * dw + dx as u32) as usize;
                    if src_idx < src_pixels.len() && dst_idx < dp.len() {
                        dp[dst_idx] = src_pixels[src_idx];
                    }
                }
            }
        }
        drop(gdi);
        GDI32::flush_dib_pixels_to_memory(uc, hbmp_dest);
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
pub(super) fn stretch_dib_its(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
    let rop = uc.read_arg(12);

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
    let header = read_dib_header(uc, lp_bits_info);
    let bmi_width = header.width;
    let bmi_height = header.height;
    let top_down = header.top_down;
    let stride = header.stride;

    let total_bytes = (stride * bmi_height) as usize;
    let raw = uc
        .mem_read_as_vec(lp_bits as u64, total_bytes)
        .unwrap_or_default();
    if raw.is_empty() {
        return Some(ApiHookResult::callee(13, Some(n_dest_height)));
    }

    let src_pixels = super::raw_dib_to_pixels(
        &raw,
        bmi_width,
        bmi_height,
        stride,
        header.bit_count,
        top_down,
        &header.palette,
        header.red_mask,
        header.green_mask,
        header.blue_mask,
        header.alpha_mask,
    );

    let (hbmp_dest, hwnd_dest, origin_x, origin_y) = {
        let gdi = uc.get_data().gdi_objects.lock().unwrap();
        if let Some(GdiObject::Dc {
            selected_bitmap,
            associated_window,
            origin_x,
            origin_y,
            ..
        }) = gdi.get(&hdc)
        {
            (*selected_bitmap, *associated_window, *origin_x, *origin_y)
        } else {
            return Some(ApiHookResult::callee(13, Some(n_dest_height)));
        }
    };

    if hbmp_dest != 0 {
        GDI32::sync_dib_pixels(uc, hbmp_dest);
        let clip_rects = GDI32::clip_rects_for_hdc(uc, hdc);
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
            if rop == 0x00CC0020 || rop == 0x008800C6 || rop == 0x00EE0086 || rop == 0x00660046 {
                // 비트맵 기반 ROP (SRCCOPY, SRCAND, SRCPAINT, SRCINVERT)
                let abs_dw = n_dest_width.abs();
                let abs_dh = n_dest_height.abs();

                for dy in 0..abs_dh {
                    let sy = if n_dest_height > 0 {
                        y_src + dy * n_src_height / n_dest_height
                    } else {
                        y_src + (abs_dh - 1 - dy) * n_src_height / abs_dh
                    };
                    let sy = sy.clamp(0, bmi_height as i32 - 1) as u32;

                    for dx in 0..abs_dw {
                        let sx = if n_dest_width > 0 {
                            x_src + dx * n_src_width / n_dest_width
                        } else {
                            x_src + (abs_dw - 1 - dx) * n_src_width / abs_dw
                        };
                        let sx = sx.clamp(0, bmi_width as i32 - 1) as u32;

                        let dst_x = x_dest + dx + origin_x;
                        let dst_y = y_dest + dy + origin_y;

                        if dst_x < 0 || dst_y < 0 || dst_x >= dw as i32 || dst_y >= dh as i32 {
                            continue;
                        }
                        if !GDI32::point_in_clip_rects(&clip_rects, dst_x, dst_y) {
                            continue;
                        }
                        let src_idx = (sy * bmi_width + sx) as usize;
                        let dst_idx = (dst_y as u32 * dw + dst_x as u32) as usize;
                        if src_idx < src_pixels.len() && dst_idx < dp.len() {
                            let src_val = src_pixels[src_idx];
                            match rop {
                                0x008800C6 => dp[dst_idx] &= src_val, // SRCAND
                                0x00EE0086 => dp[dst_idx] |= src_val, // SRCPAINT
                                0x00660046 => dp[dst_idx] ^= src_val, // SRCINVERT
                                _ => dp[dst_idx] = src_val,           // SRCCOPY
                            }
                        }
                    }
                }
            } else if rop == 0x00F00021 || rop == 0x00000042 || rop == 0x00FF0062 {
                // PATCOPY, BLACKNESS, WHITENESS
                let brush_color = match rop {
                    0x00F00021 => {
                        // DC의 현재 브러시 색상을 가져오려면 DC 핸들이 필요함
                        // StretchDIBits의 hdc (read_arg(0)) 사용
                        let hdc = uc.read_arg(0);
                        if let Some(GdiObject::Dc { selected_brush, .. }) = gdi.get(&hdc) {
                            if let Some(GdiObject::Brush { color }) = gdi.get(selected_brush) {
                                Some(*color)
                            } else {
                                Some(0x00FFFFFF)
                            }
                        } else {
                            Some(0x00FFFFFF)
                        }
                    }
                    0x00000042 => Some(0x00000000),
                    0x00FF0062 => Some(0x00FFFFFF),
                    _ => None,
                };
                if let Some(color) = brush_color {
                    for (left, top, right, bottom) in GDI32::intersect_rect_with_clip_rects(
                        &clip_rects,
                        x_dest + origin_x,
                        y_dest + origin_y,
                        x_dest + origin_x + n_dest_width.abs(),
                        y_dest + origin_y + n_dest_height.abs(),
                    ) {
                        GdiRenderer::draw_rect(
                            &mut dp,
                            dw,
                            dh,
                            left,
                            top,
                            right,
                            bottom,
                            None,
                            Some(color),
                        );
                    }
                }
            }
        }
        drop(gdi);
        GDI32::flush_dib_pixels_to_memory(uc, hbmp_dest);
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
pub(super) fn set_stretch_blt_mode(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hdc = uc.read_arg(0);
    let mode = uc.read_arg(1);
    crate::emu_log!("[GDI32] SetStretchBltMode({:#x}, {}) -> int 1", hdc, mode);
    Some(ApiHookResult::callee(2, Some(1)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dll::win32::GdiObject;
    use std::collections::HashMap;

    #[test]
    fn compatible_bitmap_inherits_monochrome_format_from_memory_dc() {
        let mut gdi_objects = HashMap::new();
        gdi_objects.insert(
            0x1001,
            GdiObject::Bitmap {
                width: 1,
                height: 1,
                pixels: Arc::new(Mutex::new(vec![0u32; 1])),
                bits_addr: None,
                stride: 4,
                bit_count: 1,
                top_down: false,
                palette: vec![0x000000, 0x00FF_FFFF],
                red_mask: 0,
                green_mask: 0,
                blue_mask: 0,
                alpha_mask: 0,
            },
        );
        gdi_objects.insert(
            0x2001,
            GdiObject::Dc {
                associated_window: 0,
                width: 1,
                height: 1,
                origin_x: 0,
                origin_y: 0,
                selected_bitmap: 0x1001,
                selected_font: 0,
                selected_brush: 0,
                selected_pen: 0,
                selected_region: 0,
                selected_palette: 0,
                bk_mode: 1,
                bk_color: 0x00FF_FFFF,
                text_color: 0x0000_0000,
                rop2_mode: 13,
                current_x: 0,
                current_y: 0,
            },
        );
    }
}
