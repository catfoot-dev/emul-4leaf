use crate::dll::win32::{GdiObject, Win32Context, gdi32::GDI32};
use unicorn_engine::Unicorn;

/// 윈도우의 비클라이언트 영역(프레임, 캡션)을 그립니다.
pub fn draw_window_frame(uc: &mut Unicorn<Win32Context>, hwnd: u32) {
    let h_surface = {
        let ctx = uc.get_data();
        let win_event = ctx.win_event.lock().unwrap();
        let Some(win) = win_event.windows.get(&hwnd) else {
            return;
        };
        if win.use_native_frame {
            return;
        }

        win.surface_bitmap
    };

    if h_surface == 0 {
        return;
    }

    GDI32::sync_dib_pixels(uc, h_surface);

    let gdi_objects = uc.get_data().gdi_objects.lock().unwrap();
    if let Some(GdiObject::Bitmap { pixels, .. }) = gdi_objects.get(&h_surface) {
        let pixels = pixels.lock().unwrap();
        drop(pixels);
        drop(gdi_objects);
        GDI32::flush_dib_pixels_to_memory(uc, h_surface);
    }
}
