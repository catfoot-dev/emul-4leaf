use crate::dll::win32::{GdiObject, Win32Context, gdi32::GDI32};
use crate::ui::gdi_renderer::GdiRenderer;
use unicorn_engine::Unicorn;

use super::USER32;

/// 윈도우의 비클라이언트 영역(프레임, 캡션)을 그립니다.
pub fn draw_window_frame(uc: &mut Unicorn<Win32Context>, hwnd: u32) {
    let (style, ex_style, title, h_surface, frame_w, frame_h, is_active) = {
        let ctx = uc.get_data();
        let win_event = ctx.win_event.lock().unwrap();
        let Some(win) = win_event.windows.get(&hwnd) else {
            return;
        };
        if win.use_native_frame {
            return;
        }

        let (bw, bh, caption) = USER32::get_window_frame_size(win.style, win.ex_style);
        if bw == 0 && bh == 0 && caption == 0 {
            return;
        }

        let active_hwnd = ctx.active_hwnd.load(std::sync::atomic::Ordering::SeqCst);
        let is_active = active_hwnd == hwnd;

        (
            win.style,
            win.ex_style,
            win.title.clone(),
            win.surface_bitmap,
            win.width,
            win.height,
            is_active,
        )
    };

    if h_surface == 0 {
        return;
    }

    GDI32::sync_dib_pixels(uc, h_surface);
    let gdi_objects = uc.get_data().gdi_objects.lock().unwrap();
    if let Some(GdiObject::Bitmap {
        pixels,
        width,
        height,
        ..
    }) = gdi_objects.get(&h_surface)
    {
        let mut pixels = pixels.lock().unwrap();
        let w = *width as i32;
        let h = *height as i32;

        let (bw, bh, caption) = USER32::get_window_frame_size(style, ex_style);

        // 1. 전체 배경 채우기 (밝은 회색)
        // let color_face = 0xFFC0C0C0;
        // 캡션이 있거나 테두리가 있는 경우에만 배경을 먼저 채웁니다.
        // 클라이언트 영역은 나중에 WM_PAINT에서 채워지므로 여기서는 프레임 영역만 주로 다룹니다.

        // 2. 3D 테두리 그리기
        if bw > 0 || bh > 0 {
            GdiRenderer::draw_edge(&mut pixels, w as u32, h as u32, 0, 0, w, h, false);
        }

        // 3. 캡션 바 그리기
        if caption > 0 {
            let cap_left = bw;
            let cap_top = bh;
            let cap_right = w - bw;
            let cap_bottom = bh + caption;

            // 캡션 배경색 (활성: 파랑, 비활성: 회색)
            let cap_color = if is_active { 0xFF000080 } else { 0xFF808080 };
            GdiRenderer::draw_rect(
                &mut pixels,
                w as u32,
                h as u32,
                cap_left,
                cap_top,
                cap_right,
                cap_bottom,
                None,
                Some(cap_color),
            );

            // 캡션 텍스트 (흰색)
            GdiRenderer::draw_text(
                &mut pixels,
                w as u32,
                h as u32,
                cap_left + 4,
                cap_top + 2,
                &title,
                14.0,
                0xFFFFFFFF,
                None,
            );

            // 닫기 버튼 모양 (간단히 X 표시)
            let btn_size = caption - 4;
            let btn_left = cap_right - btn_size - 2;
            let btn_top = cap_top + 2;
            GdiRenderer::draw_edge(
                &mut pixels,
                w as u32,
                h as u32,
                btn_left,
                btn_top,
                btn_left + btn_size,
                btn_top + btn_size,
                false,
            );
            GdiRenderer::draw_text(
                &mut pixels,
                w as u32,
                h as u32,
                btn_left + 4,
                btn_top,
                "x",
                12.0,
                0xFF000000,
                None,
            );
        }

        drop(pixels);
        drop(gdi_objects);
        GDI32::flush_dib_pixels_to_memory(uc, h_surface);
    }
}
