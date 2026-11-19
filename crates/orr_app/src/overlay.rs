//! The per-frame overlay of the views: text on the screen, lines and rectangles in the world.
//!
//! A view fills it through `ViewContext::draw()`. The app clears it before every frame of
//! view calls and draws it after the default renderer, so it is always on top of the picture.
//! The text is drawn with a real font when the run has one (the system Korean font by default,
//! or the one of `App::font`): glyphs are rasterized on demand into an atlas and drawn as
//! textured quads (see `glyphs.rs`). Without a font the text is the 5x7 bitmap font of
//! `orr_render::text` (ASCII only: digits, letters, a few signs; lowercase letters show as
//! capitals). A character the font cannot draw shows as a hollow box and is reported once on
//! stderr, so it never vanishes without a trace; see [`Overlay::missing_glyphs`].

use std::collections::BTreeSet;

use orr_render::orr_rhi::{Rhi, TextureFormat, Wgpu};
use orr_render::text::{draw_text_marked, has_glyph, pixel_camera, text_width, GLYPH_H};
use orr_render::{RenderList, RenderList3D, Renderer};

use crate::font::FontHandle;
use crate::glyphs::GlyphPass;
use crate::typeset::{wrap, Align, Anchor, Face, ScreenRect};

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
    /// The font of the run, or `None` for the 5x7 bitmap font. Kept when the frame is cleared.
    font: Option<FontHandle>,
    /// Window size in pixels (0 before the run knows it). Kept when the frame is cleared.
    screen: [f32; 2],
}

/// The characters of `text` that have no glyph (in `font`, or in the bitmap font when there is
/// none), are not in `known` yet, and are not control characters, in order of first use.
/// They are added to `known`.
fn new_missing(text: &str, font: Option<&FontHandle>, known: &mut BTreeSet<char>) -> Vec<char> {
    let mut found = Vec::new();
    let has = |c: char| font.map_or_else(|| has_glyph(c), |font| font.has_glyph(c));
    for c in text.chars().filter(|c| !has(*c) && !c.is_control()) {
        if known.insert(c) {
            found.push(c);
        }
    }
    found
}

/// Width and height in pixels of `text` drawn with capitals `size` pixels high in the 5x7
/// bitmap font. With a font use [`Overlay::measure`].
pub fn text_size(text: &str, size: f32) -> [f32; 2] {
    let scale = size / GLYPH_H as f32;
    [text_width(text, scale), size]
}

impl Overlay {
    /// Text with the top of its capital letters at screen position `pos` (pixels, y down) and its
    /// left edge at `pos[0]`. `size` is the height of a capital letter in pixels (7 pixels is the
    /// bitmap font's own size). One line: use [`Overlay::text_box`] for wrapping.
    pub fn text(&mut self, pos: [f32; 2], size: f32, color: [f32; 4], text: &str) {
        if size > 0.0 && size.is_finite() && !text.is_empty() {
            for c in new_missing(text, self.font.as_ref(), &mut self.missing) {
                eprintln!("overlay text: the font has no glyph for {c:?} (U+{:04X}); it is drawn as a box", u32::from(c));
            }
            self.texts.push(TextItem { pos, size, color, text: text.to_string() });
        }
    }

    /// Text in `rect`, broken into lines that fit its width (at spaces, and between two
    /// characters inside a word that is wider than the box, as Korean without spaces needs).
    /// A newline character starts a new line. Each line is placed by `align` between the left
    /// and right edge of `rect`; the first line starts at the top edge, and lines below the
    /// bottom edge are still drawn. A `rect` without width does not wrap. Returns the width of
    /// the widest line and the height of all lines, to place something below the text.
    pub fn text_box(&mut self, rect: ScreenRect, size: f32, color: [f32; 4], text: &str, align: Align) -> [f32; 2] {
        if !(size > 0.0 && size.is_finite()) || text.is_empty() {
            return [0.0, 0.0];
        }
        let font = self.font.clone();
        let face = Face::new(font.as_deref(), size);
        let lines = wrap(text, Some(rect.w), &mut |c| face.advance(c));
        let line_height = face.line_height();
        let mut widest = 0.0f32;
        for (i, line) in lines.iter().enumerate() {
            widest = widest.max(line.width);
            let top = rect.y + i as f32 * line_height;
            let x = rect.x + align.offset(rect.w, line.width);
            // `text` places the top of the capitals; the baseline is `ascent` below the top of the line.
            self.text([x, top + face.ascent() - size], size, color, &line.text);
        }
        [widest, lines.len() as f32 * line_height]
    }

    /// One line of text placed at `anchor` of the window, moved by `shift` pixels (x to the right,
    /// y down; use a negative shift to keep a margin from the right or bottom edge). Needs the
    /// window size: without it (a window-free run) the text starts at the top left corner.
    pub fn text_anchored(&mut self, anchor: Anchor, shift: [f32; 2], size: f32, color: [f32; 4], text: &str) {
        let [w, h] = self.measure(text, size);
        let placed = self.screen_rect().anchored(anchor, w, h, shift);
        self.text([placed.x.max(0.0), placed.y.max(0.0)], size, color, text);
    }

    /// Width and height in pixels of one line of `text` with capitals `size` pixels high, drawn
    /// with the font of this run. The height is `size`: the capital letters, without descenders.
    pub fn measure(&self, text: &str, size: f32) -> [f32; 2] {
        [Face::new(self.font.as_deref(), size).width(text), size]
    }

    /// The window size in pixels, `[0, 0]` while the run does not know it (a window-free run
    /// until [`Overlay::set_screen_size`]). With `--screenshot` it is the size of the picture.
    pub fn screen_size(&self) -> [f32; 2] {
        self.screen
    }

    /// The whole window as a rectangle, to place text and boxes relative to its edges.
    pub fn screen_rect(&self) -> ScreenRect {
        ScreenRect::new(0.0, 0.0, self.screen[0], self.screen[1])
    }

    pub fn set_screen_size(&mut self, size: [f32; 2]) {
        self.screen = size;
    }

    /// The font of this run, `None` for the 5x7 bitmap font.
    pub fn font(&self) -> Option<&FontHandle> {
        self.font.as_ref()
    }

    pub fn set_font(&mut self, font: Option<FontHandle>) {
        self.font = font;
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
    glyphs: GlyphPass,
}

impl TextPass {
    pub(crate) fn new(rhi: Wgpu, format: TextureFormat) -> Self {
        Self { glyphs: GlyphPass::new(rhi.clone(), format), renderer: Renderer::new(rhi, format), list: RenderList::new() }
    }

    /// Draws the text of `overlay` into `view` (`size` pixels) and keeps what is there: textured
    /// glyphs when the overlay has a font, else the quads of the 5x7 bitmap font.
    pub(crate) fn draw(&mut self, view: &<Wgpu as Rhi>::TextureView, size: (u32, u32), overlay: &Overlay) {
        if let Some(font) = overlay.font() {
            self.glyphs.draw(view, size, overlay, font);
            return;
        }
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
        assert_eq!(new_missing("SCORE 가가 ~", None, &mut known), vec!['가', '~']);
        assert!(new_missing("가 ~ A", None, &mut known).is_empty(), "reported once");
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

    fn lines_of(overlay: &Overlay) -> Vec<&str> {
        overlay.texts().iter().map(|t| t.text.as_str()).collect()
    }

    #[test]
    fn a_korean_text_box_wraps_at_its_width_and_keeps_every_glyph() {
        // The bitmap metrics: capitals 14 pixels high, 12 pixels per character, 21 per line.
        let mut overlay = Overlay::default();
        let used = overlay.text_box(ScreenRect::new(10.0, 20.0, 60.0, 0.0), 14.0, WHITE, "가나다라마바사", Align::Left);

        assert_eq!(lines_of(&overlay), ["가나다라마", "바사"], "five syllables fit 60 pixels, with no space to break at");
        assert_eq!(overlay.texts().iter().map(|t| t.text.chars().count()).sum::<usize>(), 7);
        assert_eq!(overlay.texts()[0].pos, [10.0, 20.0]);
        assert_eq!(overlay.texts()[1].pos, [10.0, 41.0]);
        assert_eq!(used, [60.0, 42.0], "the widest line and the height of two lines");
    }

    #[test]
    fn a_text_box_breaks_a_korean_sentence_at_its_spaces() {
        let mut overlay = Overlay::default();
        overlay.text_box(ScreenRect::new(0.0, 0.0, 84.0, 0.0), 14.0, WHITE, "별 5개를 모으세요", Align::Left);
        assert_eq!(lines_of(&overlay), ["별 5개를", "모으세요"]);
    }

    #[test]
    fn text_box_lines_are_aligned_inside_the_box() {
        let place = |align| {
            let mut overlay = Overlay::default();
            overlay.text_box(ScreenRect::new(10.0, 0.0, 60.0, 0.0), 14.0, WHITE, "가나다라마바사", align);
            overlay.texts().iter().map(|t| t.pos[0]).collect::<Vec<_>>()
        };
        // The second line is two characters (24 pixels) in a box 60 pixels wide.
        assert_eq!(place(Align::Left), [10.0, 10.0]);
        assert_eq!(place(Align::Center), [10.0, 28.0]);
        assert_eq!(place(Align::Right), [10.0, 46.0]);
    }

    #[test]
    fn text_anchored_uses_the_window_size_and_keeps_to_the_corner_without_one() {
        let mut overlay = Overlay::default();
        overlay.text_anchored(Anchor::Center, [0.0, 0.0], 14.0, WHITE, "ABCD");
        assert_eq!(overlay.texts()[0].pos, [0.0, 0.0], "no window size: the corner");

        overlay.clear();
        overlay.set_screen_size([800.0, 600.0]);
        assert_eq!(overlay.screen_size(), [800.0, 600.0]);
        overlay.text_anchored(Anchor::Center, [0.0, 0.0], 14.0, WHITE, "ABCD");
        overlay.text_anchored(Anchor::BottomRight, [-10.0, -10.0], 14.0, WHITE, "ABCD");
        // "ABCD" is 4 * 12 = 48 pixels wide and 14 high.
        assert_eq!(overlay.texts()[0].pos, [376.0, 293.0]);
        assert_eq!(overlay.texts()[1].pos, [742.0, 576.0]);
        assert_eq!(overlay.screen_rect(), ScreenRect::new(0.0, 0.0, 800.0, 600.0));
    }

    #[test]
    fn the_window_size_survives_a_new_frame() {
        let mut overlay = Overlay::default();
        overlay.set_screen_size([320.0, 200.0]);
        overlay.text([0.0; 2], 14.0, WHITE, "A");
        overlay.clear();
        assert_eq!(overlay.screen_size(), [320.0, 200.0]);
    }

    #[test]
    fn with_a_font_korean_is_drawable_and_only_a_character_the_font_lacks_is_reported() {
        let Some(font) = crate::font::system_font() else {
            eprintln!("no system font with Hangul on this machine: the font checks are skipped");
            return;
        };
        let mut overlay = Overlay::default();
        overlay.set_font(Some(font));

        overlay.text([0.0; 2], 14.0, WHITE, "별 5개를 모으세요 일시정지 승리!");
        assert!(overlay.missing_glyphs().is_empty(), "{:?}", overlay.missing_glyphs());
        overlay.text([0.0; 2], 14.0, WHITE, "\u{FFFF}");
        assert_eq!(overlay.missing_glyphs().iter().collect::<Vec<_>>(), vec![&'\u{FFFF}']);

        let [syllable, letter] = ["가", "i"].map(|t| overlay.measure(t, 14.0)[0]);
        assert!(syllable > letter, "a proportional font: a Hangul syllable is wider than an i");
        let mut boxed = Overlay::default();
        boxed.set_font(overlay.font().cloned());
        let rect = ScreenRect::new(0.0, 0.0, 100.0, 0.0);
        boxed.text_box(rect, 14.0, WHITE, "별 5개를 모으세요 별 5개를 모으세요", Align::Left);
        assert!(boxed.texts().len() >= 2, "the sentence is wider than the box");
        assert!(boxed.texts().iter().all(|t| boxed.measure(&t.text, 14.0)[0] <= 100.0));
    }
}
