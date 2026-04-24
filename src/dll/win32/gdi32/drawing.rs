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
const MAX_FRAME_INFERENCE_PIXELS: i64 = 4_000_000;
type RectInterval = (i32, i32);
type RectBand = (i32, i32, Vec<RectInterval>);

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

fn queue_fill_rects_for_clip(
    ctx: &Win32Context,
    surface_bitmap: u32,
    clip_rects: &[(i32, i32, i32, i32)],
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    color: u32,
) {
    for &clip_rect in clip_rects {
        if let Some((fill_left, fill_top, fill_right, fill_bottom)) =
            intersect_rect((left, top, right, bottom), clip_rect)
        {
            ctx.queue_surface_bitmap_fill_rect(
                surface_bitmap,
                fill_left,
                fill_top,
                fill_right,
                fill_bottom,
                color,
            );
        }
    }
}

fn region_complexity(rects: &[(i32, i32, i32, i32)]) -> i32 {
    match rects.len() {
        0 => NULLREGION,
        1 => SIMPLEREGION,
        _ => COMPLEXREGION,
    }
}

fn longest_empty_interval(row: &[bool]) -> Option<(i32, i32)> {
    let mut best = None;
    let mut best_width = 0usize;
    let mut x = 0usize;

    while x < row.len() {
        if row[x] {
            x += 1;
            continue;
        }

        let start = x;
        while x < row.len() && !row[x] {
            x += 1;
        }
        let width = x - start;
        if width > best_width {
            best_width = width;
            best = Some((start as i32, x as i32));
        }
    }

    best
}

/// 창 전체 DC에 선택된 border 클립 영역에서 중앙 client 구멍을 추정합니다.
fn infer_guest_frame_from_clip_rects(
    width: i32,
    height: i32,
    rects: &[(i32, i32, i32, i32)],
) -> Option<crate::dll::win32::user32::WindowFrameMetrics> {
    if width <= 2 || height <= 2 || rects.is_empty() {
        return None;
    }

    let area = i64::from(width) * i64::from(height);
    if area <= 0 || area > MAX_FRAME_INFERENCE_PIXELS {
        return None;
    }

    let width_usize = width as usize;
    let height_usize = height as usize;
    let mut occupied = vec![false; width_usize * height_usize];
    let mut has_any_rect = false;

    for &(left, top, right, bottom) in rects {
        let Some((left, top, right, bottom)) = normalize_rect((
            left.clamp(0, width),
            top.clamp(0, height),
            right.clamp(0, width),
            bottom.clamp(0, height),
        )) else {
            continue;
        };
        has_any_rect = true;

        for y in top as usize..bottom as usize {
            let row_offset = y * width_usize;
            for x in left as usize..right as usize {
                occupied[row_offset + x] = true;
            }
        }
    }

    if !has_any_rect {
        return None;
    }

    let mut groups = Vec::new();
    let mut current: Option<(i32, i32, i32, i32)> = None;

    for y in 0..height_usize {
        let row = &occupied[y * width_usize..(y + 1) * width_usize];
        let interval = longest_empty_interval(row)
            .filter(|&(left, right)| left > 0 && right < width && left < right);

        match (current.take(), interval) {
            (Some((left, right, start_y, _)), Some((next_left, next_right)))
                if left == next_left && right == next_right =>
            {
                current = Some((left, right, start_y, y as i32 + 1));
            }
            (Some(group), Some((next_left, next_right))) => {
                groups.push(group);
                current = Some((next_left, next_right, y as i32, y as i32 + 1));
            }
            (Some(group), None) => {
                groups.push(group);
                current = None;
            }
            (None, Some((next_left, next_right))) => {
                current = Some((next_left, next_right, y as i32, y as i32 + 1));
            }
            (None, None) => {}
        }
    }

    if let Some(group) = current {
        groups.push(group);
    }

    let (left, right, top, bottom) = groups
        .into_iter()
        .filter(|&(left, right, top, bottom)| {
            top > 0 && bottom < height && left > 0 && right < width && bottom > top
        })
        .max_by_key(|&(left, right, top, bottom)| (right - left) * (bottom - top))?;

    let metrics = crate::dll::win32::user32::WindowFrameMetrics {
        left,
        top,
        right: width - right,
        bottom: height - bottom,
    };

    if metrics.left <= 0
        || metrics.top <= 0
        || metrics.right <= 0
        || metrics.bottom <= 0
        || metrics.left + metrics.right >= width
        || metrics.top + metrics.bottom >= height
    {
        return None;
    }

    Some(metrics)
}

/// 사각형 집합을 정규형(canonical form)으로 변환합니다.
///
/// 스캔라인 밴드 단위로 재구성하여 x-구간을 병합하고, 세로로 인접한 밴드 중
/// x-구간이 동일한 것을 합칩니다. 기하학적으로 동일한 두 영역은 rect 분할/순서와
/// 무관하게 동일한 정규형을 생성합니다. `EqualRgn`의 의미 비교 및 `CombineRgn`
/// 결과 정리에 사용됩니다.
pub(crate) fn canonicalize_rects(rects: &[(i32, i32, i32, i32)]) -> Vec<(i32, i32, i32, i32)> {
    let clean: Vec<(i32, i32, i32, i32)> = rects
        .iter()
        .copied()
        .filter(|&(l, t, r, b)| l < r && t < b)
        .collect();
    if clean.is_empty() {
        return Vec::new();
    }

    let mut y_edges: Vec<i32> = clean.iter().flat_map(|&(_, t, _, b)| [t, b]).collect();
    y_edges.sort_unstable();
    y_edges.dedup();

    let mut bands: Vec<RectBand> = Vec::new();
    for pair in y_edges.windows(2) {
        let y0 = pair[0];
        let y1 = pair[1];
        if y0 >= y1 {
            continue;
        }
        let mut intervals: Vec<RectInterval> = clean
            .iter()
            .filter(|&&(_, t, _, b)| t <= y0 && y1 <= b)
            .map(|&(l, _, r, _)| (l, r))
            .collect();
        if intervals.is_empty() {
            continue;
        }
        intervals.sort_unstable();
        let mut merged: Vec<RectInterval> = Vec::new();
        for (l, r) in intervals {
            if let Some(last) = merged.last_mut()
                && last.1 >= l
            {
                last.1 = last.1.max(r);
                continue;
            }
            merged.push((l, r));
        }
        bands.push((y0, y1, merged));
    }

    let mut collapsed: Vec<RectBand> = Vec::new();
    for (y0, y1, xs) in bands {
        if let Some(last) = collapsed.last_mut()
            && last.1 == y0
            && last.2 == xs
        {
            last.1 = y1;
            continue;
        }
        collapsed.push((y0, y1, xs));
    }

    let mut out: Vec<(i32, i32, i32, i32)> = Vec::new();
    for (y0, y1, xs) in collapsed {
        for (l, r) in xs {
            out.push((l, y0, r, y1));
        }
    }
    out
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
        old_color = GDI32::argb_to_colorref(*bk_color);
        *bk_color = GDI32::colorref_to_argb(color);
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
                GDI32::argb_to_colorref(*bk_color)
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
        old_color = GDI32::argb_to_colorref(*text_color);
        *text_color = GDI32::colorref_to_argb(color);
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
                GDI32::argb_to_colorref(*text_color)
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
    ctx.gdi_objects.lock().unwrap().insert(hpen, {
        let color = GDI32::colorref_to_argb(color);
        GdiObject::Pen {
            style,
            width,
            color,
        }
    });
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
    ctx.gdi_objects.lock().unwrap().insert(hbrush, {
        let color = GDI32::colorref_to_argb(color);
        GdiObject::Brush { color }
    });
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
    // crate::emu_log!(
    //     "[GDI32] CreateRectRgn({:#x}, {:#x}, {:#x}, {:#x}) -> HRGN {:#x}",
    //     x1,
    //     y1,
    //     x2,
    //     y2,
    //     hrgn
    // );
    Some(ApiHookResult::callee(4, Some(hrgn as i32)))
}

// API: int SelectClipRgn(HDC hdc, HRGN hrgn)
// 역할: 지정된 영역(Region)을 디바이스 컨텍스트(DC)의 클리핑 영역으로 설정
pub(super) fn select_clip_rgn(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
    let hdc = uc.read_arg(0);
    let hrgn = uc.read_arg(1);
    let ctx = uc.get_data();
    let mut result = 0;
    let mut dc_info = None;
    let mut selected_copy = 0;
    let region_rects;
    {
        let mut gdi_objects = ctx.gdi_objects.lock().unwrap();
        region_rects = if hrgn != 0 {
            match gdi_objects.get(&hrgn) {
                Some(GdiObject::Region { rects }) => Some(rects.clone()),
                _ => None,
            }
        } else {
            None
        };
        if let Some(rects) = &region_rects {
            selected_copy = ctx.alloc_handle();
            gdi_objects.insert(
                selected_copy,
                GdiObject::Region {
                    rects: rects.clone(),
                },
            );
        }

        if let Some(GdiObject::Dc {
            associated_window,
            width,
            height,
            origin_x,
            origin_y,
            selected_region,
            ..
        }) = gdi_objects.get_mut(&hdc)
        {
            if hrgn == 0 || selected_copy != 0 {
                // Win32는 HRGN 자체가 아니라 region 내용을 DC에 복사합니다.
                *selected_region = selected_copy;
                result = region_rects
                    .as_deref()
                    .map(region_complexity)
                    .unwrap_or(SIMPLEREGION);
                dc_info = Some((*associated_window, *width, *height, *origin_x, *origin_y));
            } else {
                result = ERROR;
            }
        }
    }

    if let (Some((hwnd, width, height, 0, 0)), Some(rects)) = (dc_info, region_rects.as_ref())
        && hwnd != 0
        && let Some(metrics) = infer_guest_frame_from_clip_rects(width, height, &rects)
    {
        let mut win_event = ctx.win_event.lock().unwrap();
        if let Some(window) = win_event.windows.get_mut(&hwnd) {
            let changed = !window.use_native_frame
                && (window.guest_frame_left != metrics.left
                    || window.guest_frame_top != metrics.top
                    || window.guest_frame_right != metrics.right
                    || window.guest_frame_bottom != metrics.bottom
                    || !window.guest_frame_exact);

            if changed {
                window.guest_frame_left = metrics.left;
                window.guest_frame_top = metrics.top;
                window.guest_frame_right = metrics.right;
                window.guest_frame_bottom = metrics.bottom;
                window.guest_frame_exact = true;
                window.last_hittest_lparam = u32::MAX;
                win_event.bump_generation();
            }
        }
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
        let raw_rects = match fn_combine {
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
                let mut rects = GDI32::subtract_region_rects(&r1, &r2);
                rects.extend(GDI32::subtract_region_rects(&r2, &r1));
                rects
            }
            4 => GDI32::subtract_region_rects(&r1, &r2),
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
        // RGN_COPY(5)는 원본 형태를 보존해야 하므로 정규화하지 않는다.
        // 그 외 모드(AND/OR/XOR/DIFF, fallback)는 EqualRgn 비교가 안정되도록
        // 스캔라인 밴드 기반 정규형으로 변환한다.
        let new_rects = if fn_combine == 5 {
            raw_rects
        } else {
            canonicalize_rects(&raw_rects)
        };
        gdi_objects.insert(hrgn_dest, GdiObject::Region { rects: new_rects });
        if let Some(GdiObject::Region { rects }) = gdi_objects.get(&hrgn_dest) {
            result = region_complexity(rects);
        }
    }
    // crate::emu_log!(
    //     "[GDI32] CombineRgn({:#x}, {:#x}, {:#x}, {:#x}) -> int {:#x}",
    //     hrgn_dest,
    //     hrgn_src1,
    //     hrgn_src2,
    //     fn_combine,
    //     result
    // );
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

    if let (Some(r1), Some(r2)) = (region1, region2)
        && canonicalize_rects(r1) == canonicalize_rects(r2)
    {
        result = 1;
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
        origin_x,
        origin_y,
        ..
    }) = uc.get_data().gdi_objects.lock().unwrap().get(&hdc)
    {
        draw_params = Some((
            *selected_bitmap,
            *selected_pen,
            *selected_brush,
            *associated_window,
            *origin_x,
            *origin_y,
        ));
    }

    if let Some((hbmp, hpen, hbrush, hwnd, origin_x, origin_y)) = draw_params
        && hbmp != 0
    {
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
                left + origin_x,
                top + origin_y,
                right + origin_x,
                bottom + origin_y,
                pen_color,
                brush_color,
                &clip_rects,
            );
            drop(pixels);
            drop(gdi_objects);
            GDI32::flush_dib_pixels_to_memory(uc, hbmp);
            if hwnd != 0 {
                if let Some(color) = brush_color {
                    queue_fill_rects_for_clip(
                        uc.get_data(),
                        hbmp,
                        &clip_rects,
                        left + origin_x,
                        top + origin_y,
                        right + origin_x,
                        bottom + origin_y,
                        color,
                    );
                }
                if let Some(color) = pen_color {
                    queue_fill_rects_for_clip(
                        uc.get_data(),
                        hbmp,
                        &clip_rects,
                        left + origin_x,
                        top + origin_y,
                        right + origin_x,
                        top + origin_y + 1,
                        color,
                    );
                    queue_fill_rects_for_clip(
                        uc.get_data(),
                        hbmp,
                        &clip_rects,
                        left + origin_x,
                        bottom + origin_y - 1,
                        right + origin_x,
                        bottom + origin_y,
                        color,
                    );
                    queue_fill_rects_for_clip(
                        uc.get_data(),
                        hbmp,
                        &clip_rects,
                        left + origin_x,
                        top + origin_y,
                        left + origin_x + 1,
                        bottom + origin_y,
                        color,
                    );
                    queue_fill_rects_for_clip(
                        uc.get_data(),
                        hbmp,
                        &clip_rects,
                        right + origin_x - 1,
                        top + origin_y,
                        right + origin_x,
                        bottom + origin_y,
                        color,
                    );
                }
                uc.get_data().win_event.lock().unwrap().update_window(hwnd);
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
        origin_x,
        origin_y,
        ..
    }) = uc.get_data().gdi_objects.lock().unwrap().get_mut(&hdc)
    {
        draw_data = Some((
            *current_x,
            *current_y,
            *selected_bitmap,
            *text_color,
            *associated_window,
            *origin_x,
            *origin_y,
        ));
        *current_x = x;
        *current_y = y;
    }

    if let Some((x1, y1, hbmp, color, hwnd, origin_x, origin_y)) = draw_data
        && hbmp != 0
    {
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
                x1 + origin_x,
                y1 + origin_y,
                x + origin_x,
                y + origin_y,
                color,
                &clip_rects,
            );
            drop(pixels);
            drop(gdi_objects);
            GDI32::flush_dib_pixels_to_memory(uc, hbmp);
            if hwnd != 0 {
                uc.get_data().queue_surface_bitmap_line(
                    hbmp,
                    x1 + origin_x,
                    y1 + origin_y,
                    x + origin_x,
                    y + origin_y,
                    color,
                );
                uc.get_data().win_event.lock().unwrap().update_window(hwnd);
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
        selected_bitmap,
        origin_x,
        origin_y,
        ..
    }) = gdi.get(&hdc)
        && let Some(GdiObject::Bitmap {
            width,
            height,
            pixels,
            ..
        }) = gdi.get(selected_bitmap)
    {
        let x = x + *origin_x;
        let y = y + *origin_y;
        let pixels = pixels.lock().unwrap();
        if x >= 0 && x < *width as i32 && y >= 0 && y < *height as i32 {
            let p = pixels[(y as u32 * *width + x as u32) as usize];
            color = GDI32::argb_to_colorref(p);
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
            origin_x,
            origin_y,
            ..
        }) = gdi.get(&hdc)
        {
            draw_params = Some((*selected_bitmap, *associated_window, *origin_x, *origin_y));
        }
    }

    if let Some((hbmp, hwnd, origin_x, origin_y)) = draw_params
        && hbmp != 0
    {
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
            let x = x + origin_x;
            let y = y + origin_y;
            let mut pixels = pixels.lock().unwrap();
            if x >= 0 && x < width as i32 && y >= 0 && y < height as i32 {
                let idx = (y as u32 * width + x as u32) as usize;
                let p = pixels[idx];
                old_cr = GDI32::argb_to_colorref(p);
                pixels[idx] = GDI32::blend_source_over(p, GDI32::colorref_to_argb(cr));
            }
            drop(pixels);
            drop(gdi);
            GDI32::flush_dib_pixels_to_memory(uc, hbmp);
            if hwnd != 0 {
                uc.get_data().queue_surface_bitmap_fill_rect(
                    hbmp,
                    x,
                    y,
                    x + 1,
                    y + 1,
                    GDI32::colorref_to_argb(cr),
                );
                uc.get_data().win_event.lock().unwrap().update_window(hwnd);
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
            origin_x,
            origin_y,
            ..
        }) = gdi.get(&hdc)
        {
            draw_params = Some((
                *selected_bitmap,
                *selected_brush,
                *associated_window,
                *origin_x,
                *origin_y,
            ));
        }
    }

    if let Some((hbmp, hbrush, hwnd, origin_x, origin_y)) = draw_params
        && hbmp != 0
    {
        GDI32::sync_dib_pixels(uc, hbmp);

        let mut brush_color = None;
        match rop {
            0x00F00021 => {
                // PATCOPY: 현재 브러시로 채움
                let gdi = uc.get_data().gdi_objects.lock().unwrap();
                if let Some(GdiObject::Brush { color }) = gdi.get(&hbrush) {
                    brush_color = Some(*color);
                } else {
                    brush_color = Some(0xFFFF_FFFF);
                }
            }
            0x00000042 => {
                // BLACKNESS: 검정색으로 채움
                brush_color = Some(0xFF00_0000);
            }
            0x00FF0062 => {
                // WHITENESS: 흰색으로 채움
                brush_color = Some(0xFFFF_FFFF);
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
                for (left, top, right, bottom) in GDI32::intersect_rect_with_clip_rects(
                    &clip_rects,
                    x + origin_x,
                    y + origin_y,
                    x + origin_x + w,
                    y + origin_y + h,
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
                    let default_clip = vec![(0, 0, width as i32, height as i32)];
                    let clip_rects = clip_rects.unwrap_or(default_clip);
                    queue_fill_rects_for_clip(
                        uc.get_data(),
                        hbmp,
                        &clip_rects,
                        x + origin_x,
                        y + origin_y,
                        x + origin_x + w,
                        y + origin_y + h,
                        color,
                    );
                    uc.get_data().win_event.lock().unwrap().update_window(hwnd);
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

#[cfg(test)]
mod tests {
    use super::{
        SIMPLEREGION, canonicalize_rects, infer_guest_frame_from_clip_rects, select_clip_rgn,
    };
    use crate::dll::win32::{GdiObject, Win32Context, WindowState};
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

    fn sample_window_state(width: i32, height: i32) -> WindowState {
        WindowState {
            class_name: "TEST".to_string(),
            class_icon: 0,
            big_icon: 0,
            small_icon: 0,
            class_hbr_background: 0,
            title: "test".to_string(),
            x: 0,
            y: 0,
            width,
            height,
            style: 0,
            ex_style: 0,
            owner_thread_id: 0,
            parent: 0,
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
            z_order: 0,
        }
    }

    fn insert_dc(
        ctx: &Win32Context,
        hwnd: u32,
        width: i32,
        height: i32,
        origin_x: i32,
        origin_y: i32,
    ) -> u32 {
        let hdc = ctx.alloc_handle();
        ctx.gdi_objects.lock().unwrap().insert(
            hdc,
            GdiObject::Dc {
                associated_window: hwnd,
                width,
                height,
                origin_x,
                origin_y,
                selected_bitmap: 0,
                selected_font: 0,
                selected_brush: 0,
                selected_pen: 0,
                selected_region: 0,
                selected_palette: 0,
                bk_mode: 2,
                bk_color: 0,
                text_color: 0,
                rop2_mode: 13,
                current_x: 0,
                current_y: 0,
            },
        );
        hdc
    }

    fn insert_region(ctx: &Win32Context, rects: Vec<(i32, i32, i32, i32)>) -> u32 {
        let hrgn = ctx.alloc_handle();
        ctx.gdi_objects
            .lock()
            .unwrap()
            .insert(hrgn, GdiObject::Region { rects });
        hrgn
    }

    #[test]
    fn canonicalize_empty_input_returns_empty() {
        assert!(canonicalize_rects(&[]).is_empty());
    }

    #[test]
    fn canonicalize_degenerate_rect_returns_empty() {
        assert!(canonicalize_rects(&[(0, 0, 0, 0)]).is_empty());
        assert!(canonicalize_rects(&[(10, 10, 10, 20)]).is_empty());
        assert!(canonicalize_rects(&[(10, 10, 20, 10)]).is_empty());
        assert!(canonicalize_rects(&[(20, 10, 10, 20)]).is_empty());
    }

    #[test]
    fn canonicalize_degenerate_initial_empty_region_equals_empty() {
        assert_eq!(canonicalize_rects(&[(0, 0, 0, 0)]), canonicalize_rects(&[]));
    }

    #[test]
    fn canonicalize_single_rect_roundtrips() {
        assert_eq!(
            canonicalize_rects(&[(10, 10, 20, 20)]),
            vec![(10, 10, 20, 20)]
        );
    }

    #[test]
    fn canonicalize_merges_horizontally_adjacent_rects() {
        let split = vec![(10, 10, 15, 20), (15, 10, 20, 20)];
        assert_eq!(canonicalize_rects(&split), vec![(10, 10, 20, 20)]);
    }

    #[test]
    fn canonicalize_merges_overlapping_rects() {
        let overlap = vec![(10, 10, 18, 20), (15, 10, 25, 20)];
        assert_eq!(canonicalize_rects(&overlap), vec![(10, 10, 25, 20)]);
    }

    #[test]
    fn canonicalize_merges_vertically_adjacent_bands() {
        let scanlines: Vec<(i32, i32, i32, i32)> = (0..10).map(|y| (0, y, 10, y + 1)).collect();
        assert_eq!(canonicalize_rects(&scanlines), vec![(0, 0, 10, 10)]);
    }

    #[test]
    fn canonicalize_is_order_insensitive() {
        let a = vec![(10, 10, 20, 20), (30, 10, 40, 20)];
        let b = vec![(30, 10, 40, 20), (10, 10, 20, 20)];
        assert_eq!(canonicalize_rects(&a), canonicalize_rects(&b));
    }

    #[test]
    fn canonicalize_is_idempotent() {
        let input = vec![
            (0, 0, 100, 100),
            (50, 50, 150, 150),
            (200, 0, 210, 10),
            (0, 0, 0, 0),
        ];
        let once = canonicalize_rects(&input);
        let twice = canonicalize_rects(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn canonicalize_recognizes_band_split_equivalence() {
        // Single 10x10 square expressed two ways.
        let whole = vec![(0, 0, 10, 10)];
        let bands = vec![(0, 0, 10, 4), (0, 4, 10, 10)];
        assert_eq!(canonicalize_rects(&whole), canonicalize_rects(&bands));
    }

    #[test]
    fn canonicalize_recognizes_column_split_equivalence() {
        let whole = vec![(0, 0, 10, 10)];
        let cols = vec![(0, 0, 5, 10), (5, 0, 10, 10)];
        assert_eq!(canonicalize_rects(&whole), canonicalize_rects(&cols));
    }

    #[test]
    fn canonicalize_distinguishes_geometrically_different_regions() {
        let a = canonicalize_rects(&[(0, 0, 10, 10)]);
        let b = canonicalize_rects(&[(0, 0, 10, 11)]);
        assert_ne!(a, b);
    }

    #[test]
    fn canonicalize_handles_disjoint_rects() {
        let input = vec![(0, 0, 10, 10), (20, 0, 30, 10)];
        let canon = canonicalize_rects(&input);
        assert_eq!(canon, vec![(0, 0, 10, 10), (20, 0, 30, 10)]);
    }

    #[test]
    fn canonicalize_mixed_degenerate_and_real_rects() {
        let input = vec![(0, 0, 0, 0), (10, 10, 20, 20), (5, 5, 5, 5)];
        assert_eq!(canonicalize_rects(&input), vec![(10, 10, 20, 20)]);
    }

    #[test]
    fn canonicalize_l_shape_vs_rearranged_l_shape() {
        // Two rects forming an L: expressed as horizontal + vertical vs
        // three smaller pieces.
        let shape_a = vec![(0, 0, 10, 4), (0, 4, 4, 10)];
        let shape_b = vec![(0, 0, 10, 2), (0, 2, 10, 4), (0, 4, 4, 10)];
        assert_eq!(canonicalize_rects(&shape_a), canonicalize_rects(&shape_b));
    }

    #[test]
    fn infer_guest_frame_from_border_shell_clip() {
        let rects = vec![
            (0, 0, 100, 8),
            (0, 8, 12, 72),
            (88, 8, 100, 72),
            (0, 72, 100, 80),
        ];

        let metrics = infer_guest_frame_from_clip_rects(100, 80, &rects).unwrap();

        assert_eq!(metrics.left, 12);
        assert_eq!(metrics.top, 8);
        assert_eq!(metrics.right, 12);
        assert_eq!(metrics.bottom, 8);
    }

    #[test]
    #[cfg_attr(
        target_arch = "aarch64",
        ignore = "cargo test 러너에서 Unicorn 초기화가 SIGILL을 유발함"
    )]
    fn select_clip_rgn_updates_window_dc_guest_frame() {
        let mut uc = new_test_uc();
        {
            let mut win_event = uc.get_data().win_event.lock().unwrap();
            win_event.create_window(0x1000, sample_window_state(100, 80));
        }
        let hdc = insert_dc(uc.get_data(), 0x1000, 100, 80, 0, 0);
        let hrgn = insert_region(
            uc.get_data(),
            vec![
                (0, 0, 100, 8),
                (0, 8, 12, 72),
                (88, 8, 100, 72),
                (0, 72, 100, 80),
            ],
        );
        write_call_frame(&mut uc, &[hdc, hrgn]);

        let result = select_clip_rgn(&mut uc).expect("select_clip_rgn result");

        assert_eq!(result.return_value, Some(3));
        let win_event = uc.get_data().win_event.lock().unwrap();
        let window = win_event.windows.get(&0x1000).unwrap();
        assert_eq!(window.guest_frame_left, 12);
        assert_eq!(window.guest_frame_top, 8);
        assert_eq!(window.guest_frame_right, 12);
        assert_eq!(window.guest_frame_bottom, 8);
        assert!(window.guest_frame_exact);
    }

    #[test]
    #[cfg_attr(
        target_arch = "aarch64",
        ignore = "cargo test 러너에서 Unicorn 초기화가 SIGILL을 유발함"
    )]
    fn select_clip_rgn_copies_region_before_selection() {
        let mut uc = new_test_uc();
        let hdc = insert_dc(uc.get_data(), 0, 100, 80, 0, 0);
        let original_rects = vec![(5, 6, 30, 40)];
        let hrgn = insert_region(uc.get_data(), original_rects.clone());
        write_call_frame(&mut uc, &[hdc, hrgn]);

        let result = select_clip_rgn(&mut uc).expect("select_clip_rgn result");

        assert_eq!(result.return_value, Some(SIMPLEREGION));
        let gdi_objects = uc.get_data().gdi_objects.lock().unwrap();
        let selected_region = match gdi_objects.get(&hdc).unwrap() {
            GdiObject::Dc {
                selected_region, ..
            } => *selected_region,
            _ => unreachable!(),
        };
        assert_ne!(selected_region, hrgn);
        assert!(matches!(
            gdi_objects.get(&hrgn),
            Some(GdiObject::Region { rects }) if *rects == original_rects
        ));
        assert!(matches!(
            gdi_objects.get(&selected_region),
            Some(GdiObject::Region { rects }) if *rects == original_rects
        ));
    }

    #[test]
    #[cfg_attr(
        target_arch = "aarch64",
        ignore = "cargo test 러너에서 Unicorn 초기화가 SIGILL을 유발함"
    )]
    fn select_clip_rgn_ignores_client_origin_dc() {
        let mut uc = new_test_uc();
        {
            let mut win_event = uc.get_data().win_event.lock().unwrap();
            win_event.create_window(0x1000, sample_window_state(100, 80));
        }
        let hdc = insert_dc(uc.get_data(), 0x1000, 88, 72, 12, 8);
        let hrgn = insert_region(uc.get_data(), vec![(0, 0, 88, 72)]);
        write_call_frame(&mut uc, &[hdc, hrgn]);

        let _ = select_clip_rgn(&mut uc).expect("select_clip_rgn result");

        let win_event = uc.get_data().win_event.lock().unwrap();
        let window = win_event.windows.get(&0x1000).unwrap();
        assert!(!window.guest_frame_exact);
        assert_eq!(window.guest_frame_left, 0);
    }

    #[test]
    #[cfg_attr(
        target_arch = "aarch64",
        ignore = "cargo test 러너에서 Unicorn 초기화가 SIGILL을 유발함"
    )]
    fn select_clip_rgn_ignores_memory_dc_without_window() {
        let mut uc = new_test_uc();
        {
            let mut win_event = uc.get_data().win_event.lock().unwrap();
            win_event.create_window(0x1000, sample_window_state(100, 80));
        }
        let hdc = insert_dc(uc.get_data(), 0, 100, 80, 0, 0);
        let hrgn = insert_region(
            uc.get_data(),
            vec![(0, 0, 100, 8), (0, 8, 12, 72), (88, 8, 100, 72)],
        );
        write_call_frame(&mut uc, &[hdc, hrgn]);

        let _ = select_clip_rgn(&mut uc).expect("select_clip_rgn result");

        let win_event = uc.get_data().win_event.lock().unwrap();
        let window = win_event.windows.get(&0x1000).unwrap();
        assert!(!window.guest_frame_exact);
        assert_eq!(window.guest_frame_top, 0);
    }
}
