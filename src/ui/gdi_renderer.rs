use rusttype::{Font as TtfFont, Scale, point};
use std::sync::OnceLock;

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

        let fg_r = ((color >> 16) & 0xFF) as u16;
        let fg_g = ((color >> 8) & 0xFF) as u16;
        let fg_b = (color & 0xFF) as u16;

        if let Some(bg_color) = bg_color {
            let bg_left = x.max(0);
            let bg_top = y.max(0);
            let bg_right = (x + text_width).min(width as i32);
            let bg_bottom = (y + text_height).min(height as i32);

            for py in bg_top..bg_bottom {
                for px in bg_left..bg_right {
                    pixels[(py as u32 * width + px as u32) as usize] = bg_color;
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
                    let a = (v.clamp(0.0, 1.0) * 255.0) as u16;
                    if a == 0 {
                        return;
                    }
                    if a >= 250 {
                        pixels[idx] = color;
                    } else {
                        let bg = pixels[idx];
                        let bg_r = ((bg >> 16) & 0xFF) as u16;
                        let bg_g = ((bg >> 8) & 0xFF) as u16;
                        let bg_b = (bg & 0xFF) as u16;
                        let r = (a * fg_r + (255 - a) * bg_r) / 255;
                        let g = (a * fg_g + (255 - a) * bg_g) / 255;
                        let b = (a * fg_b + (255 - a) * bg_b) / 255;
                        pixels[idx] = (r as u32) << 16 | (g as u32) << 8 | b as u32;
                    }
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

    /// Win32 DrawEdge 스타일의 3D 테두리를 그립니다.
    pub fn draw_edge(
        pixels: &mut [u32],
        width: u32,
        height: u32,
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        is_sunken: bool,
    ) {
        let color_light = 0xFFFFFFFF; // 흰색
        let color_shadow = 0xFF808080; // 회색
        let color_dark = 0xFF000000; // 검은색
        // let color_face = 0xFFC0C0C0; // 배경색 (밝은 회색)

        if is_sunken {
            // Sunken (눌린 효과)
            Self::draw_line(
                pixels,
                width,
                height,
                left,
                top,
                right - 1,
                top,
                color_shadow,
            );
            Self::draw_line(
                pixels,
                width,
                height,
                left,
                top,
                left,
                bottom - 1,
                color_shadow,
            );
            Self::draw_line(
                pixels,
                width,
                height,
                left + 1,
                top + 1,
                right - 2,
                top + 1,
                color_dark,
            );
            Self::draw_line(
                pixels,
                width,
                height,
                left + 1,
                top + 1,
                left + 1,
                bottom - 2,
                color_dark,
            );

            Self::draw_line(
                pixels,
                width,
                height,
                left,
                bottom - 1,
                right - 1,
                bottom - 1,
                color_light,
            );
            Self::draw_line(
                pixels,
                width,
                height,
                right - 1,
                top,
                right - 1,
                bottom - 1,
                color_light,
            );
        } else {
            // Raised (튀어나온 효과)
            Self::draw_line(
                pixels,
                width,
                height,
                left,
                top,
                right - 2,
                top,
                color_light,
            );
            Self::draw_line(
                pixels,
                width,
                height,
                left,
                top,
                left,
                bottom - 2,
                color_light,
            );

            Self::draw_line(
                pixels,
                width,
                height,
                left,
                bottom - 1,
                right - 1,
                bottom - 1,
                color_dark,
            );
            Self::draw_line(
                pixels,
                width,
                height,
                right - 1,
                top,
                right - 1,
                bottom - 1,
                color_dark,
            );
            Self::draw_line(
                pixels,
                width,
                height,
                left + 1,
                bottom - 2,
                right - 2,
                bottom - 2,
                color_shadow,
            );
            Self::draw_line(
                pixels,
                width,
                height,
                right - 2,
                top + 1,
                right - 2,
                bottom - 2,
                color_shadow,
            );
        }
    }
}
