//! A small text renderer for the layer-shell prompt: ARGB8888 pixels with
//! ab_glyph and the default egui font (no system font needed).

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};

const BACKGROUND: u32 = 0xE010_1418;
const TITLE_COLOR: (u8, u8, u8) = (0xFF, 0xFF, 0xFF);
const HINT_COLOR: (u8, u8, u8) = (0xC8, 0xD0, 0xD8);

/// Draws the prompt: the title in large text and the hint below it, both centered.
/// Returns `width * height` premultiplied ARGB pixels.
pub fn prompt(width: u32, height: u32, title: &str, hint: Option<&str>) -> Vec<u32> {
    let mut canvas = Canvas {
        width,
        height,
        pixels: vec![BACKGROUND; (width * height) as usize],
    };
    let Ok(font) = FontRef::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT) else {
        return canvas.pixels;
    };
    let max_width = f32::from(u16::try_from(width).unwrap_or(u16::MAX)) * 0.8;
    let title_size = (height as f32 / 14.0).clamp(24.0, 96.0);
    let hint_size = title_size * 0.5;
    let title_lines = wrap(&font, title_size, title, max_width);
    let hint_lines: Vec<String> = hint
        .map(|h| {
            h.lines()
                .flat_map(|l| wrap(&font, hint_size, l, max_width))
                .collect()
        })
        .unwrap_or_default();
    let total = title_lines.len() as f32 * title_size * 1.3
        + hint_lines.len() as f32 * hint_size * 1.4
        + if hint_lines.is_empty() {
            0.0
        } else {
            title_size
        };
    let mut y = (height as f32 - total) / 2.0;
    for line in &title_lines {
        canvas.line(&font, title_size, line, y, TITLE_COLOR);
        y += title_size * 1.3;
    }
    y += title_size;
    for line in &hint_lines {
        canvas.line(&font, hint_size, line, y, HINT_COLOR);
        y += hint_size * 1.4;
    }
    canvas.pixels
}

fn line_width(font: &FontRef<'_>, size: f32, text: &str) -> f32 {
    let scaled = font.as_scaled(PxScale::from(size));
    text.chars()
        .map(|c| scaled.h_advance(font.glyph_id(c)))
        .sum()
}

/// Word wrap to a maximum width. A word that is too long stays on its own line.
fn wrap(font: &FontRef<'_>, size: f32, text: &str, max_width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let candidate = if current.is_empty() {
            word.to_owned()
        } else {
            format!("{current} {word}")
        };
        if line_width(font, size, &candidate) > max_width && !current.is_empty() {
            lines.push(std::mem::replace(&mut current, word.to_owned()));
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

struct Canvas {
    width: u32,
    height: u32,
    pixels: Vec<u32>,
}

impl Canvas {
    fn line(&mut self, font: &FontRef<'_>, size: f32, text: &str, top: f32, color: (u8, u8, u8)) {
        let scale = PxScale::from(size);
        let scaled = font.as_scaled(scale);
        let mut x = (self.width as f32 - line_width(font, size, text)) / 2.0;
        let baseline = top + scaled.ascent();
        for c in text.chars() {
            let id = font.glyph_id(c);
            let glyph = id.with_scale_and_position(scale, ab_glyph::point(x, baseline));
            x += scaled.h_advance(id);
            let Some(outline) = font.outline_glyph(glyph) else {
                continue;
            };
            let bounds = outline.px_bounds();
            outline.draw(|gx, gy, coverage| {
                let px = bounds.min.x as i64 + i64::from(gx);
                let py = bounds.min.y as i64 + i64::from(gy);
                self.blend(px, py, coverage, color);
            });
        }
    }

    fn blend(&mut self, x: i64, y: i64, coverage: f32, (r, g, b): (u8, u8, u8)) {
        if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            return;
        }
        let index = (y as u32 * self.width + x as u32) as usize;
        let old = self.pixels[index];
        let mix = |shift: u32, new: u8| {
            let old = f32::from(((old >> shift) & 0xFF) as u8);
            (old + (f32::from(new) - old) * coverage.clamp(0.0, 1.0)) as u32
        };
        self.pixels[index] =
            (old & 0xFF00_0000) | (mix(16, r) << 16) | (mix(8, g) << 8) | mix(0, b);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn draws_text_on_the_background() {
        let pixels = super::prompt(400, 200, "Press Return on MacBook", Some("Copy: Ctrl+C"));
        assert_eq!(pixels.len(), 400 * 200);
        assert!(
            pixels.iter().any(|&p| p != super::BACKGROUND),
            "no text was drawn"
        );
    }
}
