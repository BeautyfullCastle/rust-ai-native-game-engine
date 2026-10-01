//! A tiny 5x7 bitmap font for on-screen stats, drawn as small quads of a 2D
//! [`RenderList`] in screen pixels (so no font file, no texture, and the
//! overlay goes through the same 2D pass as everything else).
//!
//! Use [`pixel_camera`] with the target size so 1 world unit is 1 pixel, and
//! [`draw_text`] with top-left pixel coordinates. Lowercase letters draw as
//! uppercase. Characters without a glyph draw as blanks.

use crate::camera::Camera;
use crate::list::RenderList;

/// Glyph width and height in font pixels, and the advance (glyph plus a gap).
pub const GLYPH_W: usize = 5;
pub const GLYPH_H: usize = 7;
pub const ADVANCE: usize = 6;

/// A camera for which one world unit is one screen pixel (`y` up, origin at
/// the bottom left corner of a `size` target).
pub fn pixel_camera(size: (u32, u32)) -> Camera {
    let (w, h) = (size.0.max(1) as f32, size.1.max(1) as f32);
    Camera::new([w * 0.5, h * 0.5], h * 0.5)
}

fn glyph(c: char) -> [u8; GLYPH_H] {
    match c.to_ascii_uppercase() {
        '0' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '2' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
        '3' => [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110],
        '4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        '5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        '6' => [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
        '7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        '8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        '9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
        'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'B' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        'C' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        'D' => [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110],
        'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'F' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        'G' => [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111],
        'H' => [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'I' => [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        'J' => [0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100],
        'K' => [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        'N' => [0b10001, 0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001],
        'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        'Q' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
        'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        'S' => [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'V' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
        'W' => [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001],
        'X' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        'Y' => [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
        'Z' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
        '.' => [0, 0, 0, 0, 0, 0b01100, 0b01100],
        ':' => [0, 0b01100, 0b01100, 0, 0b01100, 0b01100, 0],
        ',' => [0, 0, 0, 0, 0b01100, 0b00100, 0b01000],
        '/' => [0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000],
        '|' => [0b00100; GLYPH_H],
        '-' => [0, 0, 0, 0b11111, 0, 0, 0],
        '+' => [0, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0],
        '=' => [0, 0, 0b11111, 0, 0b11111, 0, 0],
        '_' => [0, 0, 0, 0, 0, 0, 0b11111],
        '(' => [0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010],
        ')' => [0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000],
        '[' => [0b01110, 0b01000, 0b01000, 0b01000, 0b01000, 0b01000, 0b01110],
        ']' => [0b01110, 0b00010, 0b00010, 0b00010, 0b00010, 0b00010, 0b01110],
        '%' => [0b11001, 0b11010, 0b00010, 0b00100, 0b01000, 0b01011, 0b10011],
        '!' => [0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0, 0b00100],
        '?' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0, 0b00100],
        _ => [0; GLYPH_H],
    }
}

/// Width in pixels of `text` drawn with `scale` pixels per font pixel.
pub fn text_width(text: &str, scale: f32) -> f32 {
    let n = text.chars().count();
    if n == 0 {
        0.0
    } else {
        (n * ADVANCE - (ADVANCE - GLYPH_W)) as f32 * scale
    }
}

/// Height in pixels of one line drawn with `scale`.
pub fn text_height(scale: f32) -> f32 {
    GLYPH_H as f32 * scale
}

/// Draws `text` with its top left corner at pixel `(x, y_top)` (y down from
/// the top of a target `viewport_h` pixels high). Each font pixel is a
/// `scale` x `scale` quad. Use with [`pixel_camera`].
pub fn draw_text(list: &mut RenderList, viewport_h: f32, x: f32, y_top: f32, scale: f32, text: &str, color: [f32; 4]) {
    for (i, c) in text.chars().enumerate() {
        let g = glyph(c);
        let gx = x + (i * ADVANCE) as f32 * scale;
        for (row, bits) in g.iter().enumerate() {
            for col in 0..GLYPH_W {
                if bits & (1 << (GLYPH_W - 1 - col)) != 0 {
                    let cx = gx + (col as f32 + 0.5) * scale;
                    let cy = y_top + (row as f32 + 0.5) * scale;
                    list.quad([cx, viewport_h - cy], [scale * 0.5; 2], 0.0, color);
                }
            }
        }
    }
}

/// A filled rectangle with its top left corner at `(x, y_top)`, `w` x `h` pixels.
pub fn fill_rect(list: &mut RenderList, viewport_h: f32, x: f32, y_top: f32, w: f32, h: f32, color: [f32; 4]) {
    list.quad([x + w * 0.5, viewport_h - (y_top + h * 0.5)], [w * 0.5, h * 0.5], 0.0, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_and_letters_have_glyphs_and_unknown_is_blank() {
        for c in "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ.:/|-+()%".chars() {
            assert!(glyph(c).iter().any(|&r| r != 0), "{c}");
        }
        assert_eq!(glyph('~'), [0; GLYPH_H]);
        assert_eq!(glyph('a'), glyph('A'));
    }

    #[test]
    fn text_places_quads_inside_its_box() {
        let mut l = RenderList::new();
        draw_text(&mut l, 100.0, 10.0, 20.0, 2.0, "FPS 60", [1.0; 4]);
        assert!(!l.shapes.is_empty());
        let w = text_width("FPS 60", 2.0);
        for s in &l.shapes {
            assert!(s.center[0] >= 10.0 && s.center[0] <= 10.0 + w, "{:?}", s.center);
            // y is flipped: the text occupies pixel rows 20..34, i.e. world y 66..80.
            assert!(s.center[1] >= 66.0 && s.center[1] <= 80.0, "{:?}", s.center);
        }
        let cam = pixel_camera((200, 100));
        assert_eq!(cam.world_to_screen([30.0, 50.0], (200, 100)), [30.0, 50.0]);
    }
}
