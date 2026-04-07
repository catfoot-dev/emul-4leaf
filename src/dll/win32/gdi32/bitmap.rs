use crate::{
    dll::win32::{ApiHookResult, GdiObject, Win32Context},
    helper::UnicornHelper,
    ui::gdi_renderer::GdiRenderer,
};
use std::sync::{Arc, Mutex};
use unicorn_engine::Unicorn;

use super::GDI32;

// API: HBITMAP CreateDIBSection(HDC hdc, const BITMAPINFO *pbmi, UINT usage, VOID **ppvBits, HANDLE hSection, DWORD offset)
// 역할: 애플리케이션이 직접 쓸 수 있는 DIB(장치 독립적 비트맵)를 생성
pub(super) fn create_dib_section(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn create_compatible_bitmap(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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

// API: HBITMAP CreateBitmap(int nWidth, int nHeight, UINT nPlanes, UINT nBitCount, const VOID *lpBits)
// 역할: 지정된 크기와 색 형식의 비트맵을 생성
pub(super) fn create_bitmap(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
            let converted = super::raw_dib_to_pixels(&raw, width, height, bpp, false, &[]);
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
                // DIBSection이면 emulated memory와 pixels Vec을 먼저 맞춘 뒤 복사합니다.
                GDI32::sync_dib_pixels(uc, hbmp_dest);
                GDI32::sync_dib_pixels(uc, hbmp_src);
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
    GDI32::sync_dib_pixels(uc, hbmp_dest);
    GDI32::sync_dib_pixels(uc, hbmp_src);

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
    let src_pixels = super::raw_dib_to_pixels(&raw, bmi_width, c_scans, bpp, top_down, &palette);

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
        GDI32::sync_dib_pixels(uc, hbmp_dest);
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
        super::raw_dib_to_pixels(&raw, bmi_width, bmi_height, bpp, top_down, &palette);

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
        GDI32::sync_dib_pixels(uc, hbmp_dest);
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
