//! Text layout without a window or a GPU: line breaks, alignment and screen anchors.
//!
//! The functions take the width of a character as a function, so they work with a real font
//! ([`Face`]) and with the 5x7 bitmap font alike.

use orr_render::text::{ADVANCE, GLYPH_H};

use crate::font::{em_for, Font};

/// How the lines of a text box sit between its left and right edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

impl Align {
    /// The distance from the left edge of a box `box_width` wide to the left end of a line
    /// `line_width` wide. A line wider than the box starts at the left edge.
    pub fn offset(self, box_width: f32, line_width: f32) -> f32 {
        let free = (box_width - line_width).max(0.0);
        match self {
            Align::Left => 0.0,
            Align::Center => free * 0.5,
            Align::Right => free,
        }
    }
}

/// Where a rectangle sits inside another one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    /// Horizontal and vertical place, each 0 (left, top), 0.5 or 1 (right, bottom).
    fn fractions(self) -> [f32; 2] {
        match self {
            Anchor::TopLeft => [0.0, 0.0],
            Anchor::Top => [0.5, 0.0],
            Anchor::TopRight => [1.0, 0.0],
            Anchor::Left => [0.0, 0.5],
            Anchor::Center => [0.5, 0.5],
            Anchor::Right => [1.0, 0.5],
            Anchor::BottomLeft => [0.0, 1.0],
            Anchor::Bottom => [0.5, 1.0],
            Anchor::BottomRight => [1.0, 1.0],
        }
    }
}

/// A rectangle in window pixels: `x` and `y` of the top left corner (y down), then size.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScreenRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl ScreenRect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// The rectangle without the `margin` pixels at every edge (it never gets a negative size).
    pub fn inset(self, margin: f32) -> Self {
        Self { x: self.x + margin, y: self.y + margin, w: (self.w - 2.0 * margin).max(0.0), h: (self.h - 2.0 * margin).max(0.0) }
    }

    /// A rectangle `w` x `h` placed at `anchor` inside this one, moved by `shift` pixels
    /// (x to the right, y down).
    pub fn anchored(self, anchor: Anchor, w: f32, h: f32, shift: [f32; 2]) -> Self {
        let [fx, fy] = anchor.fractions();
        Self { x: self.x + (self.w - w) * fx + shift[0], y: self.y + (self.h - h) * fy + shift[1], w, h }
    }
}

/// One line of a broken text: its characters and its width in pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    pub width: f32,
}

/// Breaks `text` into lines no wider than `max_width` (`None`: only at `'\n'`).
///
/// A line breaks at a space; the spaces at the break are dropped. A word that is wider than a
/// whole line (a Korean sentence typed without spaces) breaks between two characters, so every
/// Hangul syllable is a place to break and every line is filled to its edge. A line always has
/// at least one character, so a box narrower than one character still ends. `'\n'` starts a new
/// line; an empty line stays in the result.
pub fn wrap(text: &str, max_width: Option<f32>, advance: &mut dyn FnMut(char) -> f32) -> Vec<Line> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        wrap_paragraph(paragraph, max_width, advance, &mut lines);
    }
    lines
}

struct Builder {
    text: String,
    width: f32,
}

impl Builder {
    fn new() -> Self {
        Self { text: String::new(), width: 0.0 }
    }

    fn push(&mut self, c: char, w: f32) {
        self.text.push(c);
        self.width += w;
    }

    fn finish(&mut self, lines: &mut Vec<Line>) {
        lines.push(Line { text: std::mem::take(&mut self.text), width: std::mem::replace(&mut self.width, 0.0) });
    }
}

fn wrap_paragraph(paragraph: &str, max_width: Option<f32>, advance: &mut dyn FnMut(char) -> f32, lines: &mut Vec<Line>) {
    let Some(max) = max_width.filter(|m| *m > 0.0 && m.is_finite()) else {
        let width = paragraph.chars().map(&mut *advance).sum();
        lines.push(Line { text: paragraph.to_string(), width });
        return;
    };
    let mut line = Builder::new();
    let mut chars = paragraph.chars().peekable();
    // Leading spaces of the paragraph are kept (an indent); spaces at a break are not.
    while let Some(&c) = chars.peek() {
        if c != ' ' {
            break;
        }
        line.push(c, advance(c));
        chars.next();
    }
    let mut pending_space = 0.0f32;
    let mut pending_count = 0usize;
    while chars.peek().is_some() {
        // The spaces before the next word, then the word.
        while chars.peek() == Some(&' ') {
            pending_space += advance(' ');
            pending_count += 1;
            chars.next();
        }
        let mut word = Vec::new();
        while let Some(&c) = chars.peek() {
            if c == ' ' {
                break;
            }
            word.push((c, advance(c)));
            chars.next();
        }
        if word.is_empty() {
            break;
        }
        let word_width: f32 = word.iter().map(|(_, w)| *w).sum();
        let has_text = !line.text.is_empty();
        if !has_text || line.width + pending_space + word_width <= max {
            if has_text {
                for _ in 0..pending_count {
                    line.push(' ', pending_space / pending_count as f32);
                }
            }
        } else if word_width <= max {
            line.finish(lines);
        } else if line.width + pending_space + word[0].1 <= max {
            // A long word starts on the rest of the line.
            for _ in 0..pending_count {
                line.push(' ', pending_space / pending_count as f32);
            }
        } else {
            line.finish(lines);
        }
        pending_space = 0.0;
        pending_count = 0;
        for (c, w) in word {
            if !line.text.is_empty() && line.width + w > max {
                line.finish(lines);
            }
            line.push(c, w);
        }
    }
    line.finish(lines);
}

/// What the overlay needs to measure text: a font, or the 5x7 bitmap font when there is none.
pub(crate) struct Face<'a> {
    font: Option<&'a Font>,
    /// Height of a capital letter in pixels.
    size: f32,
}

impl<'a> Face<'a> {
    pub(crate) fn new(font: Option<&'a Font>, size: f32) -> Self {
        Self { font, size }
    }

    pub(crate) fn advance(&self, c: char) -> f32 {
        match self.font {
            Some(font) => font.advance(c, em_for(self.size)),
            None if c.is_control() => 0.0,
            None => ADVANCE as f32 * self.size / GLYPH_H as f32,
        }
    }

    /// From the top of a line to its baseline.
    pub(crate) fn ascent(&self) -> f32 {
        self.font.map_or(self.size, |font| font.ascent(em_for(self.size)))
    }

    /// From the baseline of a line to the baseline of the next.
    pub(crate) fn line_height(&self) -> f32 {
        self.font.map_or(self.size * 1.5, |font| font.line_height(em_for(self.size)))
    }

    pub(crate) fn width(&self, text: &str) -> f32 {
        text.chars().map(|c| self.advance(c)).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake font: a Hangul syllable is 10 pixels wide, a space 4, everything else 6.
    fn width(c: char) -> f32 {
        match c {
            ' ' => 4.0,
            '\u{AC00}'..='\u{D7A3}' => 10.0,
            _ => 6.0,
        }
    }

    fn lines(text: &str, max: Option<f32>) -> Vec<String> {
        wrap(text, max, &mut width).into_iter().map(|l| l.text).collect()
    }

    #[test]
    fn a_korean_sentence_breaks_at_its_spaces() {
        // "별 5개를 모으세요": 별(10) + space + 5(6) + 개를(20) = 10+4+6+20 = 40 before the next word.
        let text = "별 5개를 모으세요";
        assert_eq!(lines(text, None), vec![text]);
        assert_eq!(lines(text, Some(100.0)), vec![text]);
        // "모으세요" is 40 wide: 40 + 4 + 40 = 84 > 60 so it moves to the next line.
        assert_eq!(lines(text, Some(60.0)), vec!["별 5개를", "모으세요"]);
    }

    #[test]
    fn a_korean_text_without_spaces_breaks_between_syllables() {
        let out = wrap("가나다라마바사", Some(35.0), &mut width);
        assert_eq!(out.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), vec!["가나다", "라마바", "사"]);
        assert!(out.iter().all(|l| l.width <= 35.0));
        assert_eq!(out[0].width, 30.0);
    }

    #[test]
    fn a_long_word_after_a_short_one_fills_the_rest_of_the_line() {
        // "AB " is 6 + 6 + 4; the long word then starts on the same line.
        let out = lines("AB 가나다라마바", Some(40.0));
        assert_eq!(out, vec!["AB 가나", "다라마바"]);
    }

    #[test]
    fn spaces_at_a_break_are_dropped_and_newlines_are_kept() {
        assert_eq!(lines("AA   BB", Some(20.0)), vec!["AA", "BB"]);
        assert_eq!(lines("A\n\nB", Some(100.0)), vec!["A", "", "B"]);
        assert_eq!(lines("A\n\nB", None), vec!["A", "", "B"]);
    }

    #[test]
    fn a_box_narrower_than_one_character_still_ends() {
        assert_eq!(lines("가나", Some(1.0)), vec!["가", "나"]);
        assert_eq!(lines("가나", Some(0.0)), vec!["가나"], "no width means no wrapping");
    }

    #[test]
    fn a_line_has_the_width_of_its_characters() {
        let out = wrap("가 A", Some(100.0), &mut width);
        assert_eq!(out[0].width, 10.0 + 4.0 + 6.0);
    }

    #[test]
    fn alignment_moves_a_line_inside_its_box_and_never_left_of_it() {
        assert_eq!(Align::Left.offset(100.0, 40.0), 0.0);
        assert_eq!(Align::Center.offset(100.0, 40.0), 30.0);
        assert_eq!(Align::Right.offset(100.0, 40.0), 60.0);
        assert_eq!(Align::Center.offset(100.0, 140.0), 0.0);
        assert_eq!(Align::Right.offset(0.0, 40.0), 0.0);
    }

    #[test]
    fn rectangles_are_placed_by_anchor_and_inset() {
        let screen = ScreenRect::new(0.0, 0.0, 800.0, 600.0);
        assert_eq!(screen.anchored(Anchor::Center, 200.0, 100.0, [0.0, 0.0]), ScreenRect::new(300.0, 250.0, 200.0, 100.0));
        assert_eq!(screen.anchored(Anchor::BottomRight, 200.0, 100.0, [-10.0, -10.0]), ScreenRect::new(590.0, 490.0, 200.0, 100.0));
        assert_eq!(screen.anchored(Anchor::Top, 200.0, 100.0, [0.0, 20.0]), ScreenRect::new(300.0, 20.0, 200.0, 100.0));
        assert_eq!(screen.inset(50.0), ScreenRect::new(50.0, 50.0, 700.0, 500.0));
        assert_eq!(ScreenRect::new(0.0, 0.0, 10.0, 10.0).inset(50.0).w, 0.0);
    }

    #[test]
    fn the_bitmap_face_has_the_advance_of_the_5x7_font() {
        let face = Face::new(None, 14.0);
        assert_eq!(face.width("ABC"), 3.0 * 12.0);
        assert_eq!(face.ascent(), 14.0);
    }
}
