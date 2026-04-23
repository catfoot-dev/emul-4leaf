use rusttype::{Font as TtfFont, Scale, point};
use std::sync::OnceLock;

use crate::dll::win32::GDI32;

static GDI_FONT: OnceLock<TtfFont<'static>> = OnceLock::new();

/// 글로벌 TTF 폰트를 초기화하고 반환합니다.
fn get_ttf_font() -> Option<&'static TtfFont<'static>> {
    GDI_FONT.get_or_init(|| {
        TtfFont::try_from_bytes(crate::ui::GULIM_FONT_DATA)
            .expect("Failed to load gulim.ttf for GDI rendering")
    });
    GDI_FONT.get()
}

/// GDI 렌더링 엔진 (소프트웨어 래스터라이저)
pub struct GdiRenderer;

#[allow(dead_code, clippy::too_many_arguments)]
impl GdiRenderer {
    fn glyph_color(color: u32, coverage: u8) -> u32 {
        let alpha = ((color >> 24) & 0xFF) * u32::from(coverage);
        (((alpha + 127) / 255) << 24) | (color & 0x00FF_FFFF)
    }

    fn put_pixel(pixels: &mut [u32], idx: usize, color: u32) {
        pixels[idx] = GDI32::blend_source_over(pixels[idx], color);
    }

    /// 좌표가 클리핑 사각형 목록 안에 포함되는지 확인합니다.
    pub(crate) fn point_in_clip_rects(clip_rects: &[(i32, i32, i32, i32)], x: i32, y: i32) -> bool {
        clip_rects
            .iter()
            .any(|&(left, top, right, bottom)| x >= left && x < right && y >= top && y < bottom)
    }

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
                let idx = (y * width as i32 + x) as usize;
                Self::put_pixel(pixels, idx, color);
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

    /// 클리핑 사각형을 적용하여 선을 그립니다.
    pub fn draw_line_clipped(
        pixels: &mut [u32],
        width: u32,
        height: u32,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        color: u32,
        clip_rects: &[(i32, i32, i32, i32)],
    ) {
        let mut x = x1;
        let mut y = y1;
        let dx = (x2 - x1).abs();
        let dy = (y2 - y1).abs();
        let sx = if x1 < x2 { 1 } else { -1 };
        let sy = if y1 < y2 { 1 } else { -1 };
        let mut err = dx - dy;

        loop {
            if x >= 0
                && x < width as i32
                && y >= 0
                && y < height as i32
                && Self::point_in_clip_rects(clip_rects, x, y)
            {
                let idx = (y * width as i32 + x) as usize;
                Self::put_pixel(pixels, idx, color);
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
                    let idx = (y * width as i32 + x) as usize;
                    Self::put_pixel(pixels, idx, color);
                }
            }
        }

        // 테두리 (Pen)
        if let Some(color) = pen_color {
            // 상/하
            for x in x_start..x_end {
                if top >= 0 && top < height as i32 {
                    let idx = (top * width as i32 + x) as usize;
                    Self::put_pixel(pixels, idx, color);
                }
                if bottom > 0 && bottom - 1 < height as i32 {
                    let idx = ((bottom - 1) * width as i32 + x) as usize;
                    Self::put_pixel(pixels, idx, color);
                }
            }
            // 좌/우
            for y in y_start..y_end {
                if left >= 0 && left < width as i32 {
                    let idx = (y * width as i32 + left) as usize;
                    Self::put_pixel(pixels, idx, color);
                }
                if right > 0 && right - 1 < width as i32 {
                    let idx = (y * width as i32 + (right - 1)) as usize;
                    Self::put_pixel(pixels, idx, color);
                }
            }
        }
    }

    /// 클리핑 사각형을 적용하여 직사각형을 그립니다.
    pub fn draw_rect_clipped(
        pixels: &mut [u32],
        width: u32,
        height: u32,
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        pen_color: Option<u32>,
        brush_color: Option<u32>,
        clip_rects: &[(i32, i32, i32, i32)],
    ) {
        if let Some(color) = brush_color {
            for &(cl, ct, cr, cb) in clip_rects {
                let il = left.max(cl);
                let it = top.max(ct);
                let ir = right.min(cr);
                let ib = bottom.min(cb);
                if il < ir && it < ib {
                    Self::draw_rect(pixels, width, height, il, it, ir, ib, None, Some(color));
                }
            }
        }

        if let Some(color) = pen_color {
            Self::draw_line_clipped(
                pixels,
                width,
                height,
                left,
                top,
                right - 1,
                top,
                color,
                clip_rects,
            );
            Self::draw_line_clipped(
                pixels,
                width,
                height,
                left,
                bottom - 1,
                right - 1,
                bottom - 1,
                color,
                clip_rects,
            );
            Self::draw_line_clipped(
                pixels,
                width,
                height,
                left,
                top,
                left,
                bottom - 1,
                color,
                clip_rects,
            );
            Self::draw_line_clipped(
                pixels,
                width,
                height,
                right - 1,
                top,
                right - 1,
                bottom - 1,
                color,
                clip_rects,
            );
        }
    }

    /// BitBlt (픽셀 복사 및 래스터 연산 지원)
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
        rop: u32,
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

                let src_val = src_pixels[(sy * src_width as i32 + sx) as usize];
                let dst_idx = (dy * dest_width as i32 + dx) as usize;

                match rop {
                    0x008800C6 => {
                        // SRCAND: dst = dst & src
                        let rgb = (dest_pixels[dst_idx] & src_val) & 0x00FF_FFFF;
                        dest_pixels[dst_idx] = (dest_pixels[dst_idx] & 0xFF00_0000) | rgb;
                    }
                    0x00EE0086 => {
                        // SRCPAINT: dst = dst | src
                        let rgb = (dest_pixels[dst_idx] | src_val) & 0x00FF_FFFF;
                        dest_pixels[dst_idx] = (dest_pixels[dst_idx] & 0xFF00_0000) | rgb;
                    }
                    0x00660046 => {
                        // SRCINVERT: dst = dst ^ src
                        let rgb = (dest_pixels[dst_idx] ^ src_val) & 0x00FF_FFFF;
                        dest_pixels[dst_idx] = (dest_pixels[dst_idx] & 0xFF00_0000) | rgb;
                    }
                    0x00CC0020 => {
                        // SRCCOPY: source-over 알파 합성
                        Self::put_pixel(dest_pixels, dst_idx, src_val);
                    }
                    _ => {
                        Self::put_pixel(dest_pixels, dst_idx, src_val);
                    }
                }
            }
        }
    }

    /// TTF 폰트를 사용한 텍스트 그리기
    pub fn draw_text(
        pixels: &mut [u32],
        width: u32,
        height: u32,
        x: i32,
        y: i32,
        text: &str,
        font_size: f32,
        color: u32,
        bg_color: Option<u32>,
    ) {
        let Some(font) = get_ttf_font() else { return };
        let scale = Scale::uniform(font_size);
        let v_metrics = font.v_metrics(scale);
        let offset = point(x as f32, y as f32 + v_metrics.ascent);
        let text_width = Self::measure_text_width(text, font_size);
        let text_height = (v_metrics.ascent - v_metrics.descent).ceil() as i32;

        let glyphs: Vec<_> = font.layout(text, scale, offset).collect();

        if let Some(bg_color) = bg_color {
            let bg_left = x.max(0);
            let bg_top = y.max(0);
            let bg_right = (x + text_width).min(width as i32);
            let bg_bottom = (y + text_height).min(height as i32);

            for py in bg_top..bg_bottom {
                for px in bg_left..bg_right {
                    let idx = (py as u32 * width + px as u32) as usize;
                    Self::put_pixel(pixels, idx, bg_color);
                }
            }
        }

        for glyph in glyphs {
            if let Some(bb) = glyph.pixel_bounding_box() {
                glyph.draw(|gx, gy, v| {
                    let px = bb.min.x + gx as i32;
                    let py = bb.min.y + gy as i32;
                    if px < 0 || px >= width as i32 || py < 0 || py >= height as i32 {
                        return;
                    }
                    let idx = (py as u32 * width + px as u32) as usize;
                    if idx >= pixels.len() {
                        return;
                    }
                    let a = (v.clamp(0.0, 1.0) * 255.0) as u8;
                    if a == 0 {
                        return;
                    }
                    Self::put_pixel(pixels, idx, Self::glyph_color(color, a));
                });
            }
        }
    }

    /// 클리핑 사각형을 적용하여 텍스트를 그립니다.
    pub fn draw_text_clipped(
        pixels: &mut [u32],
        width: u32,
        height: u32,
        x: i32,
        y: i32,
        text: &str,
        font_size: f32,
        color: u32,
        bg_color: Option<u32>,
        clip_rects: &[(i32, i32, i32, i32)],
    ) {
        let Some(font) = get_ttf_font() else { return };
        let scale = Scale::uniform(font_size);
        let v_metrics = font.v_metrics(scale);
        let offset = point(x as f32, y as f32 + v_metrics.ascent);
        let text_width = Self::measure_text_width(text, font_size);
        let text_height = (v_metrics.ascent - v_metrics.descent).ceil() as i32;

        let glyphs: Vec<_> = font.layout(text, scale, offset).collect();

        if let Some(bg_color) = bg_color {
            let bg_left = x.max(0);
            let bg_top = y.max(0);
            let bg_right = (x + text_width).min(width as i32);
            let bg_bottom = (y + text_height).min(height as i32);

            for py in bg_top..bg_bottom {
                for px in bg_left..bg_right {
                    if Self::point_in_clip_rects(clip_rects, px, py) {
                        let idx = (py as u32 * width + px as u32) as usize;
                        Self::put_pixel(pixels, idx, bg_color);
                    }
                }
            }
        }

        for glyph in glyphs {
            if let Some(bb) = glyph.pixel_bounding_box() {
                glyph.draw(|gx, gy, v| {
                    let px = bb.min.x + gx as i32;
                    let py = bb.min.y + gy as i32;
                    if px < 0
                        || px >= width as i32
                        || py < 0
                        || py >= height as i32
                        || !Self::point_in_clip_rects(clip_rects, px, py)
                    {
                        return;
                    }
                    let idx = (py as u32 * width + px as u32) as usize;
                    if idx >= pixels.len() {
                        return;
                    }
                    let a = (v.clamp(0.0, 1.0) * 255.0) as u8;
                    if a == 0 {
                        return;
                    }
                    Self::put_pixel(pixels, idx, Self::glyph_color(color, a));
                });
            }
        }
    }

    /// TTF 폰트로 텍스트 폭(픽셀)을 계산합니다.
    pub fn measure_text_width(text: &str, font_size: f32) -> i32 {
        let Some(font) = get_ttf_font() else {
            return text.len() as i32 * (font_size as i32 / 2);
        };
        let scale = Scale::uniform(font_size);
        let glyphs: Vec<_> = font.layout(text, scale, point(0.0, 0.0)).collect();
        glyphs
            .last()
            .map(|g| {
                g.pixel_bounding_box()
                    .map(|bb| bb.max.x)
                    .unwrap_or_else(|| {
                        (g.position().x + g.unpositioned().h_metrics().advance_width) as i32
                    })
            })
            .unwrap_or(0)
    }

    /// TTF 폰트의 세로 메트릭을 반환합니다: (height, ascent, descent)
    pub fn font_metrics(font_size: f32) -> (i32, i32, i32) {
        let Some(font) = get_ttf_font() else {
            return (
                font_size as i32,
                font_size as i32 * 4 / 5,
                font_size as i32 / 5,
            );
        };
        let scale = Scale::uniform(font_size);
        let v = font.v_metrics(scale);
        let height = (v.ascent - v.descent + v.line_gap).ceil() as i32;
        let ascent = v.ascent.ceil() as i32;
        let descent = (-v.descent).ceil() as i32;
        (height, ascent, descent)
    }
}

#[cfg(test)]
mod tests {
    use super::GdiRenderer;

    #[test]
    fn draw_rect_uses_source_over_alpha() {
        let mut pixels = vec![0xFF00_00FF];

        GdiRenderer::draw_rect(&mut pixels, 1, 1, 0, 0, 1, 1, None, Some(0x80FF_0000));

        assert_eq!(pixels, vec![0xFF80_007F]);
    }
}
