use crate::{
    dll::win32::{ApiHookResult, GdiObject, Win32Context},
    helper::UnicornHelper,
    ui::gdi_renderer::GdiRenderer,
};
use unicorn_engine::Unicorn;

use super::GDI32;

const NULLREGION: i32 = 1;
const SIMPLEREGION: i32 = 2;
const COMPLEXREGION: i32 = 3;
const ERROR: i32 = 0;

fn normalize_rect(rect: (i32, i32, i32, i32)) -> Option<(i32, i32, i32, i32)> {
    let (left, top, right, bottom) = rect;
    (left < right && top < bottom).then_some((left, top, right, bottom))
}

fn intersect_rect(
    a: (i32, i32, i32, i32),
    b: (i32, i32, i32, i32),
) -> Option<(i32, i32, i32, i32)> {
    normalize_rect((a.0.max(b.0), a.1.max(b.1), a.2.min(b.2), a.3.min(b.3)))
}

fn subtract_rect(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> Vec<(i32, i32, i32, i32)> {
    let Some(intersection) = intersect_rect(a, b) else {
        return vec![a];
    };

    let mut rects = Vec::new();
    if let Some(rect) = normalize_rect((a.0, a.1, a.2, intersection.1)) {
        rects.push(rect);
    }
    if let Some(rect) = normalize_rect((a.0, intersection.3, a.2, a.3)) {
        rects.push(rect);
    }
    if let Some(rect) = normalize_rect((a.0, intersection.1, intersection.0, intersection.3)) {
        rects.push(rect);
    }
    if let Some(rect) = normalize_rect((intersection.2, intersection.1, a.2, intersection.3)) {
        rects.push(rect);
    }
    rects
}

fn subtract_region(
    region: &[(i32, i32, i32, i32)],
    subtractors: &[(i32, i32, i32, i32)],
) -> Vec<(i32, i32, i32, i32)> {
    let mut current = region.to_vec();
    for &sub in subtractors {
        let mut next = Vec::new();
        for rect in current {
            next.extend(subtract_rect(rect, sub));
        }
        current = next;
    }
    current
}

fn region_complexity(rects: &[(i32, i32, i32, i32)]) -> i32 {
    match rects.len() {
        0 => NULLREGION,
        1 => SIMPLEREGION,
        _ => COMPLEXREGION,
    }
}

// API: int SetBkMode(HDC hdc, int mode)
// 역할: 배경 혼합 모드를 설정
pub(super) fn set_bk_mode(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn get_bk_mode(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn set_bk_color(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn get_bk_color(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn set_text_color(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn get_text_color(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn create_pen(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn create_solid_brush(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn create_rect_rgn(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let x1 = uc.read_arg(0) as i32;
    let y1 = uc.read_arg(1) as i32;
    let x2 = uc.read_arg(2) as i32;
    let y2 = uc.read_arg(3) as i32;
    let ctx = uc.get_data();
    let hrgn = ctx.alloc_handle();
    ctx.gdi_objects.lock().unwrap().insert(
        hrgn,
        GdiObject::Region {
            rects: vec![(x1, y1, x2, y2)],
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
pub(super) fn select_clip_rgn(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn combine_rgn(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hrgn_dest = uc.read_arg(0);
    let hrgn_src1 = uc.read_arg(1);
    let hrgn_src2 = uc.read_arg(2);
    let fn_combine = uc.read_arg(3);
    let ctx = uc.get_data();
    let mut result = ERROR;
    let mut gdi_objects = ctx.gdi_objects.lock().unwrap();
    let region1 = if let Some(GdiObject::Region { rects }) = gdi_objects.get(&hrgn_src1) {
        Some(rects.clone())
    } else {
        None
    };
    let region2 = if let Some(GdiObject::Region { rects }) = gdi_objects.get(&hrgn_src2) {
        Some(rects.clone())
    } else {
        None
    };

    if let (Some(r1), Some(r2)) = (region1, region2) {
        let new_rects = match fn_combine {
            1 => {
                let mut intersections = Vec::new();
                for rect1 in &r1 {
                    for rect2 in &r2 {
                        if let Some(intersection) = intersect_rect(*rect1, *rect2) {
                            intersections.push(intersection);
                        }
                    }
                }
                intersections
            }
            2 => {
                let mut rects = r1;
                rects.extend(r2);
                rects
            }
            3 => {
                let mut rects = subtract_region(&r1, &r2);
                rects.extend(subtract_region(&r2, &r1));
                rects
            }
            4 => subtract_region(&r1, &r2),
            5 => r1,
            _ => {
                let mut rects = Vec::new();
                let (mut left, mut top, mut right, mut bottom) =
                    (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
                for r in r1.iter().chain(r2.iter()) {
                    left = left.min(r.0);
                    top = top.min(r.1);
                    right = right.max(r.2);
                    bottom = bottom.max(r.3);
                }
                rects.push((left, top, right, bottom));
                rects
            }
        };
        gdi_objects.insert(hrgn_dest, GdiObject::Region { rects: new_rects });
        if let Some(GdiObject::Region { rects }) = gdi_objects.get(&hrgn_dest) {
            result = region_complexity(rects);
        }
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
pub(super) fn equal_rgn(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hrgn1 = uc.read_arg(0);
    let hrgn2 = uc.read_arg(1);
    let ctx = uc.get_data();
    let mut result = 0;
    let gdi_objects = ctx.gdi_objects.lock().unwrap();
    let region1 = if let Some(GdiObject::Region { rects }) = gdi_objects.get(&hrgn1) {
        Some(rects)
    } else {
        None
    };
    let region2 = if let Some(GdiObject::Region { rects }) = gdi_objects.get(&hrgn2) {
        Some(rects)
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
pub(super) fn get_rgn_box(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hrgn = uc.read_arg(0);
    let lprc = uc.read_arg(1);

    let rect = {
        let ctx = uc.get_data();
        if let Some(GdiObject::Region { rects }) = ctx.gdi_objects.lock().unwrap().get(&hrgn) {
            if rects.is_empty() {
                None
            } else {
                let (mut left, mut top, mut right, mut bottom) =
                    (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
                for r in rects {
                    left = left.min(r.0);
                    top = top.min(r.1);
                    right = right.max(r.2);
                    bottom = bottom.max(r.3);
                }
                Some([left, top, right, bottom])
            }
        } else {
            None
        }
    };

    let mut result = NULLREGION;
    if let Some(r) = rect {
        uc.write_mem(lprc as u64, &r);
        result = if r[0] < r[2] && r[1] < r[3] {
            SIMPLEREGION
        } else {
            NULLREGION
        };
    }

    crate::emu_log!(
        "[GDI32] GetRgnBox({:#x}, {:#x}) -> int {:#x}",
        hrgn,
        lprc,
        result
    );
    Some(ApiHookResult::callee(2, Some(result)))
}

// API: BOOL Rectangle(HDC hdc, int left, int top, int right, int bottom)
// 역할: 현재 펜과 브러시를 사용하여 직사각형을 그림
pub(super) fn rectangle(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
            GDI32::sync_dib_pixels(uc, hbmp);
            let clip_rects = GDI32::clip_rects_for_hdc(uc, hdc);
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
                let default_clip = vec![(0, 0, width as i32, height as i32)];
                let clip_rects = clip_rects.unwrap_or(default_clip);
                let mut pixels = pixels.lock().unwrap();
                GdiRenderer::draw_rect_clipped(
                    &mut pixels,
                    width,
                    height,
                    left,
                    top,
                    right,
                    bottom,
                    pen_color,
                    brush_color,
                    &clip_rects,
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
pub(super) fn move_to_ex(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn line_to(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
                GdiRenderer::draw_line_clipped(
                    &mut pixels,
                    width,
                    height,
                    x1,
                    y1,
                    x,
                    y,
                    color,
                    &clip_rects,
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

    crate::emu_log!("[GDI32] LineTo({:#x}, {}, {}) -> BOOL 1", hdc, x, y);
    Some(ApiHookResult::callee(3, Some(1)))
}

// API: int SetROP2(HDC hdc, int nROP2)
// 역할: 디바이스 컨텍스트의 그리기 모드를 설정
pub(super) fn set_rop2(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn realize_palette(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hdc = uc.read_arg(0);
    let mut count = 0u32;
    let mut selected_bitmap = 0u32;
    let mut palette_entries: Option<Vec<[u8; 4]>> = None;

    let selected_palette = {
        let gdi_objects = uc.get_data().gdi_objects.lock().unwrap();
        if let Some(GdiObject::Dc {
            selected_palette,
            selected_bitmap: hbmp,
            ..
        }) = gdi_objects.get(&hdc)
        {
            selected_bitmap = *hbmp;
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
pub(super) fn select_palette(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
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
pub(super) fn create_palette(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let logpal_addr = uc.read_arg(0);
    let num_entries = uc.read_u16(logpal_addr as u64 + 2) as u32;
    let mut entries = Vec::with_capacity(num_entries as usize);
    let entries_addr = logpal_addr as u64 + 4;
    for i in 0..num_entries as u64 {
        let offset = entries_addr + i * 4;
        let red = uc.read_u8(offset);
        let green = uc.read_u8(offset + 1);
        let blue = uc.read_u8(offset + 2);
        let flags = uc.read_u8(offset + 3);
        entries.push([blue, green, red, flags]);
    }
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
pub(super) fn get_pixel(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hdc = uc.read_arg(0);
    let x = uc.read_arg(1) as i32;
    let y = uc.read_arg(2) as i32;
    let mut color = 0;

    let ctx = uc.get_data();
    let gdi = ctx.gdi_objects.lock().unwrap();
    if let Some(GdiObject::Dc {
        selected_bitmap, ..
    }) = gdi.get(&hdc)
    {
        if let Some(GdiObject::Bitmap {
            width,
            height,
            pixels,
            ..
        }) = gdi.get(selected_bitmap)
        {
            let pixels = pixels.lock().unwrap();
            if x >= 0 && x < *width as i32 && y >= 0 && y < *height as i32 {
                let p = pixels[(y as u32 * *width + x as u32) as usize];
                // 0x00RRGGBB -> 0x00BBGGRR (COLORREF)
                let r = (p >> 16) & 0xFF;
                let g = (p >> 8) & 0xFF;
                let b = p & 0xFF;
                color = (b << 16) | (g << 8) | r;
            }
        }
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
pub(super) fn set_pixel(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hdc = uc.read_arg(0);
    let x = uc.read_arg(1) as i32;
    let y = uc.read_arg(2) as i32;
    let cr = uc.read_arg(3); // COLORREF: 0x00BBGGRR
    let mut old_cr = 0;

    let mut draw_params = None;
    {
        let gdi = uc.get_data().gdi_objects.lock().unwrap();
        if let Some(GdiObject::Dc {
            selected_bitmap,
            associated_window,
            ..
        }) = gdi.get(&hdc)
        {
            draw_params = Some((*selected_bitmap, *associated_window));
        }
    }

    if let Some((hbmp, hwnd)) = draw_params {
        if hbmp != 0 {
            GDI32::sync_dib_pixels(uc, hbmp);
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
                if x >= 0 && x < width as i32 && y >= 0 && y < height as i32 {
                    let idx = (y as u32 * width + x as u32) as usize;
                    let p = pixels[idx];
                    // 0x00RRGGBB -> 0x00BBGGRR
                    let r = (p >> 16) & 0xFF;
                    let g = (p >> 8) & 0xFF;
                    let b = p & 0xFF;
                    old_cr = (b << 16) | (g << 8) | r;

                    // 0x00BBGGRR -> 0x00RRGGBB
                    let nr = cr & 0xFF;
                    let ng = (cr >> 8) & 0xFF;
                    let nb = (cr >> 16) & 0xFF;
                    pixels[idx] = (nr << 16) | (ng << 8) | nb;
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
        "[GDI32] SetPixel({:#x}, {}, {}, {:#x}) -> COLORREF {:#x}",
        hdc,
        x,
        y,
        cr,
        old_cr
    );
    Some(ApiHookResult::callee(4, Some(old_cr as i32)))
}

// API: BOOL PatBlt(HDC hdc, int x, int y, int w, int h, DWORD rop)
// 역할: 지정된 브러시를 사용하여 직사각형 영역을 드로잉 (Raster Operation)
pub(super) fn pat_blt(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hdc = uc.read_arg(0);
    let x = uc.read_arg(1) as i32;
    let y = uc.read_arg(2) as i32;
    let w = uc.read_arg(3) as i32;
    let h = uc.read_arg(4) as i32;
    let rop = uc.read_arg(5);

    let mut draw_params = None;
    {
        let gdi = uc.get_data().gdi_objects.lock().unwrap();
        if let Some(GdiObject::Dc {
            selected_bitmap,
            selected_brush,
            associated_window,
            ..
        }) = gdi.get(&hdc)
        {
            draw_params = Some((*selected_bitmap, *selected_brush, *associated_window));
        }
    }

    if let Some((hbmp, hbrush, hwnd)) = draw_params {
        if hbmp != 0 {
            GDI32::sync_dib_pixels(uc, hbmp);

            let mut brush_color = None;
            match rop {
                0x00F00021 => {
                    // PATCOPY: 현재 브러시로 채움
                    let gdi = uc.get_data().gdi_objects.lock().unwrap();
                    if let Some(GdiObject::Brush { color }) = gdi.get(&hbrush) {
                        brush_color = Some(*color);
                    } else {
                        brush_color = Some(0x00FFFFFF);
                    }
                }
                0x00000042 => {
                    // BLACKNESS: 검정색으로 채움
                    brush_color = Some(0x00000000);
                }
                0x00FF0062 => {
                    // WHITENESS: 흰색으로 채움
                    brush_color = Some(0x00FFFFFF);
                }
                _ => {
                    crate::emu_log!("[GDI32] PatBlt unhandled ROP: {:#x}", rop);
                }
            }

            if let Some(color) = brush_color {
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
                        GDI32::intersect_rect_with_clip_rects(&clip_rects, x, y, x + w, y + h)
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
    }

    crate::emu_log!(
        "[GDI32] PatBlt({:#x}, {}, {}, {}, {}, {:#x}) -> BOOL 1",
        hdc,
        x,
        y,
        w,
        h,
        rop
    );
    Some(ApiHookResult::callee(6, Some(1)))
}
