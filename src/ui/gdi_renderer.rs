use std::sync::{Arc, Mutex};

/// GDI 렌더링 엔진 (소프트웨어 래스터라이저)
pub struct GdiRenderer;

impl GdiRenderer {
    /// 선 그리기 (Bresenham 알고리즘 등)
    pub fn draw_line(
        pixels: &mut [u32],
        width: u32,
        height: u32,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        color: u32,
    ) {
        let mut x = x1;
        let mut y = y1;
        let dx = (x2 - x1).abs();
        let dy = (y2 - y1).abs();
        let sx = if x1 < x2 { 1 } else { -1 };
        let sy = if y1 < y2 { 1 } else { -1 };
        let mut err = dx - dy;

        loop {
            if x >= 0 && x < width as i32 && y >= 0 && y < height as i32 {
                pixels[(y * width as i32 + x) as usize] = color;
            }

            if x == x2 && y == y2 {
                break;
            }

            let e2 = 2 * err;
            if e2 > -dy {
                err -= dy;
                x += sx;
            }
            if e2 < dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// 직사각형 그리기 (테두리 + 채우기)
    pub fn draw_rect(
        pixels: &mut [u32],
        width: u32,
        height: u32,
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        pen_color: Option<u32>,
        brush_color: Option<u32>,
    ) {
        let x_start = left.max(0);
        let y_start = top.max(0);
        let x_end = right.min(width as i32);
        let y_end = bottom.min(height as i32);

        // 채우기 (Brush)
        if let Some(color) = brush_color {
            for y in y_start..y_end {
                for x in x_start..x_end {
                    pixels[(y * width as i32 + x) as usize] = color;
                }
            }
        }

        // 테두리 (Pen)
        if let Some(color) = pen_color {
            // 상/하
            for x in x_start..x_end {
                if top >= 0 && top < height as i32 {
                    pixels[(top * width as i32 + x) as usize] = color;
                }
                if bottom - 1 >= 0 && bottom - 1 < height as i32 {
                    pixels[((bottom - 1) * width as i32 + x) as usize] = color;
                }
            }
            // 좌/우
            for y in y_start..y_end {
                if left >= 0 && left < width as i32 {
                    pixels[(y * width as i32 + left) as usize] = color;
                }
                if right - 1 >= 0 && right - 1 < width as i32 {
                    pixels[(y * width as i32 + (right - 1)) as usize] = color;
                }
            }
        }
    }

    /// BitBlt (단순 픽셀 복사)
    pub fn bit_blt(
        dest_pixels: &mut [u32],
        dest_width: u32,
        dest_height: u32,
        x_dest: i32,
        y_dest: i32,
        width: u32,
        height: u32,
        src_pixels: &[u32],
        src_width: u32,
        src_height: u32,
        x_src: i32,
        y_src: i32,
    ) {
        for y in 0..height as i32 {
            let sy = y_src + y;
            let dy = y_dest + y;
            if sy < 0 || sy >= src_height as i32 || dy < 0 || dy >= dest_height as i32 {
                continue;
            }

            for x in 0..width as i32 {
                let sx = x_src + x;
                let dx = x_dest + x;
                if sx < 0 || sx >= src_width as i32 || dx < 0 || dx >= dest_width as i32 {
                    continue;
                }

                dest_pixels[(dy * dest_width as i32 + dx) as usize] =
                    src_pixels[(sy * src_width as i32 + sx) as usize];
            }
        }
    }
}
