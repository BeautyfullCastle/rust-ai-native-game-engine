//! The per-frame overlay of the views: text on the screen, lines and rectangles in the world.
//!
//! A view fills it through `ViewContext::draw()`. The app clears it before every frame of
//! view calls and draws it after the default renderer, so it is always on top of the picture.
//! The text is the 5x7 bitmap font of `orr_render::text` (ASCII only: digits, letters, a few
//! signs; lowercase letters show as capitals). A character the font cannot draw (Korean, other
//! letters, `~`) shows as a hollow box and is reported once on stderr, so it never vanishes
//! without a trace; see [`Overlay::missing_glyphs`].

use std::collections::BTreeSet;

use orr_render::orr_rhi::{Rhi, TextureFormat, Wgpu};
use orr_render::text::{draw_text_marked, has_glyph, pixel_camera, text_width, GLYPH_H};
use orr_render::{RenderList, RenderList3D, Renderer};

/// Pixel width of the lines of the overlay.
const LINE_WIDTH: f32 = 2.0;

/// A piece of text at a screen position.
#[derive(Clone, Debug, PartialEq)]
pub struct TextItem {
    /// Top left corner in pixels from the top left corner of the window.
    pub pos: [f32; 2],
    /// Height of a capital letter in pixels.
    pub size: f32,
    pub color: [f32; 4],
    pub text: String,
}

/// A line in the world. A 2D run draws `x` and `y` and ignores `z`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineItem {
    pub a: [f32; 3],
    pub b: [f32; 3],
    /// Width in pixels.
    pub width: f32,
    pub color: [f32; 4],
}

/// A filled rectangle in the plane `z = 0` of the world. A 3D run draws its outline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectItem {
    pub min: [f32; 2],
    pub max: [f32; 2],
    pub color: [f32; 4],
}

/// What the views want drawn on top of this frame. See `ViewContext::draw()`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Overlay {
    texts: Vec<TextItem>,
    lines: Vec<LineItem>,
    rects: Vec<RectItem>,
    /// The characters without a glyph that were drawn so far (kept when the frame is cleared).
    missing: BTreeSet<char>,
}

/// The characters of `text` that have no glyph, are not in `known` yet, and are not control
/// characters, in order of first use. They are added to `known`.
fn new_missing(text: &str, known: &mut BTreeSet<char>) -> Vec<char> {
    let mut found = Vec::new();
    for c in text.chars().filter(|c| !has_glyph(*c) && !c.is_control()) {
        if known.insert(c) {
            found.push(c);
        }
    }
    found
}

/// Width and height in pixels of `text` drawn with capitals `size` pixels high.
/// Use it to place text by its right edge or its middle.
pub fn text_size(text: &str, size: f32) -> [f32; 2] {
    let scale = size / GLYPH_H as f32;
    [text_width(text, scale), size]
}

impl Overlay {
    /// Text with its top left corner at screen position `pos` (pixels, y down). `size` is the
    /// height of a capital letter in pixels (7 pixels is the font's own size).
    pub fn text(&mut self, pos: [f32; 2], size: f32, color: [f32; 4], text: &str) {
        if size > 0.0 && size.is_finite() && !text.is_empty() {
            for c in new_missing(text, &mut self.missing) {
                eprintln!("overlay text: the font has no glyph for {c:?} (U+{:04X}); it is drawn as a box", u32::from(c));
            }
            self.texts.push(TextItem { pos, size, color, text: text.to_string() });
        }
    }

    /// A line between two world points, 2 pixels wide.
    pub fn line(&mut self, a: [f32; 2], b: [f32; 2], color: [f32; 4]) {
        self.line3([a[0], a[1], 0.0], [b[0], b[1], 0.0], color);
    }

    /// The same for a point in space. A 2D run draws `x` and `y` only.
    pub fn line3(&mut self, a: [f32; 3], b: [f32; 3], color: [f32; 4]) {
        self.lines.push(LineItem { a, b, width: LINE_WIDTH, color });
    }

    /// A filled rectangle in the world, from corner `min` to corner `max`.
    /// A 3D run draws only its outline, in the plane `z = 0`.
    pub fn rect(&mut self, min: [f32; 2], max: [f32; 2], color: [f32; 4]) {
        self.rects.push(RectItem { min, max, color });
    }

    /// The twelve edges of an axis aligned box in space (a 2D run draws its flat outline).
    pub fn aabb3(&mut self, min: [f32; 3], max: [f32; 3], color: [f32; 4]) {
        let corner = |i: usize| [if i & 1 == 0 { min[0] } else { max[0] }, if i & 2 == 0 { min[1] } else { max[1] }, if i & 4 == 0 { min[2] } else { max[2] }];
        for i in 0..8usize {
            for bit in [1usize, 2, 4] {
                if i & bit == 0 {
                    self.line3(corner(i), corner(i | bit), color);
                }
            }
        }
    }

    /// The characters drawn so far that the font has no glyph for. Each one was reported once
    /// on stderr and is drawn as a box.
    pub fn missing_glyphs(&self) -> &BTreeSet<char> {
        &self.missing
    }

    pub fn texts(&self) -> &[TextItem] {
        &self.texts
    }

    pub fn lines(&self) -> &[LineItem] {
        &self.lines
    }

    pub fn rects(&self) -> &[RectItem] {
        &self.rects
    }

    pub fn is_empty(&self) -> bool {
        self.texts.is_empty() && self.lines.is_empty() && self.rects.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.texts.clear();
        self.lines.clear();
        self.rects.clear();
    }

    /// Adds the world items to the 2D draw list of the scene (after its own items).
    pub(crate) fn add_to_list(&self, list: &mut RenderList) {
        for rect in &self.rects {
            let half = [(rect.max[0] - rect.min[0]) * 0.5, (rect.max[1] - rect.min[1]) * 0.5];
            list.quad([rect.min[0] + half[0], rect.min[1] + half[1]], [half[0].abs(), half[1].abs()], 0.0, rect.color);
        }
        for line in &self.lines {
            list.line([line.a[0], line.a[1]], [line.b[0], line.b[1]], line.width, line.color);
        }
    }

    /// Adds the world items to the 3D draw list of the scene.
    pub(crate) fn add_to_list3(&self, list: &mut RenderList3D) {
        for rect in &self.rects {
            let corners = [[rect.min[0], rect.min[1]], [rect.max[0], rect.min[1]], [rect.max[0], rect.max[1]], [rect.min[0], rect.max[1]]];
            for i in 0..4 {
                let (a, b) = (corners[i], corners[(i + 1) % 4]);
                list.line([a[0], a[1], 0.0], [b[0], b[1], 0.0], LINE_WIDTH, rect.color);
            }
        }
        for line in &self.lines {
            list.line(line.a, line.b, line.width, line.color);
        }
    }

    /// Adds the text as quads of a draw list for a picture of `size` pixels, to be drawn with
    /// `orr_render::text::pixel_camera(size)`.
    pub(crate) fn fill_text_list(&self, list: &mut RenderList, size: (u32, u32)) {
        for item in &self.texts {
            draw_text_marked(list, size.1 as f32, item.pos[0], item.pos[1], item.size / GLYPH_H as f32, &item.text, item.color);
        }
    }
}

/// Draws the text of an [`Overlay`] over a picture that is already drawn.
pub(crate) struct TextPass {
    renderer: Renderer<Wgpu>,
    list: RenderList,
}

impl TextPass {
    pub(crate) fn new(rhi: Wgpu, format: TextureFormat) -> Self {
        Self { renderer: Renderer::new(rhi, format), list: RenderList::new() }
    }

    /// Draws the text of `overlay` into `view` (`size` pixels) and keeps what is there.
    pub(crate) fn draw(&mut self, view: &<Wgpu as Rhi>::TextureView, size: (u32, u32), overlay: &Overlay) {
        self.list.clear();
        overlay.fill_text_list(&mut self.list, size);
        if self.list.is_empty() {
            return;
        }
        let clear = self.renderer.clear.take();
        self.renderer.draw(view, size, &self.list, &pixel_camera(size));
        self.renderer.clear = clear;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: [f32; 4] = [1.0; 4];

    #[test]
    fn a_character_without_a_glyph_is_recorded_once_and_the_text_is_kept() {
        let mut known = BTreeSet::new();
        assert_eq!(new_missing("SCORE 가가 ~", &mut known), vec!['가', '~']);
        assert!(new_missing("가 ~ A
", &mut known).is_empty(), "reported once");
        let mut overlay = Overlay::default();
        overlay.text([0.0, 0.0], 14.0, WHITE, "OK 가");
        overlay.text([0.0, 20.0], 14.0, WHITE, "가가");
        assert_eq!(overlay.missing_glyphs().iter().collect::<Vec<_>>(), vec![&'가']);
        assert_eq!(overlay.texts().len(), 2, "the text is still drawn");
        overlay.clear();
        assert_eq!(overlay.missing_glyphs().len(), 1, "a new frame keeps what was reported");
    }

    #[test]
    fn missing_glyphs_become_boxes_in_the_text_quads() {
        let mut overlay = Overlay::default();
        overlay.text([0.0, 0.0], 7.0, WHITE, "가");
        let mut list = RenderList::new();
        overlay.fill_text_list(&mut list, (100, 100));
        assert!(!list.is_empty(), "the box is visible");
    }

    #[test]
    fn text_size_follows_the_advance_of_the_font() {
        // 5x7 font, advance 6: n letters are 6n - 1 font pixels wide.
        assert_eq!(text_size("ABC", 7.0), [17.0, 7.0]);
        assert_eq!(text_size("ABC", 14.0), [34.0, 14.0]);
        assert_eq!(text_size("", 7.0), [0.0, 7.0]);
    }

    #[test]
    fn text_becomes_quads_inside_its_box_and_blanks_make_none() {
        let mut overlay = Overlay::default();
        overlay.text([10.0, 20.0], 14.0, WHITE, "SCORE 12");
        overlay.text([0.0, 0.0], 14.0, WHITE, "   ");
        let mut list = RenderList::new();
        overlay.fill_text_list(&mut list, (200, 100));
        assert!(!list.shapes.is_empty());
        let [w, h] = text_size("SCORE 12", 14.0);
        for quad in &list.shapes {
            assert!(quad.center[0] >= 10.0 && quad.center[0] <= 10.0 + w);
            // y is flipped to world (up) in a picture 100 pixels high: the box is rows 20..20+h.
            assert!(quad.center[1] >= 100.0 - 20.0 - h && quad.center[1] <= 80.0);
        }
        // "SCORE 12" has 7 inked glyphs and a gap: more quads than letters, and the blank text adds none.
        assert!(list.shapes.len() > 7);
    }

    #[test]
    fn a_box_has_twelve_edges_and_empty_or_bad_text_is_ignored() {
        let mut overlay = Overlay::default();
        overlay.aabb3([0.0; 3], [1.0; 3], WHITE);
        assert_eq!(overlay.lines().len(), 12);
        overlay.text([0.0; 2], 0.0, WHITE, "A");
        overlay.text([0.0; 2], f32::NAN, WHITE, "A");
        overlay.text([0.0; 2], 7.0, WHITE, "");
        assert!(overlay.texts().is_empty());
    }

    #[test]
    fn the_world_items_go_to_the_2d_list_and_a_rect_is_an_outline_in_3d() {
        let mut overlay = Overlay::default();
        overlay.rect([-1.0, -2.0], [3.0, 2.0], WHITE);
        overlay.line([0.0, 0.0], [1.0, 1.0], WHITE);
        let mut list = RenderList::new();
        overlay.add_to_list(&mut list);
        assert_eq!((list.shapes.len(), list.lines.len()), (1, 1));
        assert_eq!((list.shapes[0].center, list.shapes[0].half_size), ([1.0, 0.0], [2.0, 2.0]));
        let mut list3 = RenderList3D::new();
        overlay.add_to_list3(&mut list3);
        assert_eq!(list3.lines.len(), 5);
    }
}
