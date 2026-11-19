//! Fonts of the overlay text: a TrueType or OpenType file read with `ab_glyph`.
//!
//! A [`Font`] measures and rasterizes single characters. It does no shaping: one character is one
//! glyph, placed one after the other (right for Latin, Hangul syllables, kana and ideographs;
//! not for scripts that need ligatures or reordering). The run picks a font when it starts:
//! the one given with `App::font` or `App::font_file`, or else a system font that has Hangul
//! ([`system_font`]), or else none, and the overlay keeps its 5x7 bitmap font.

use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ab_glyph::{Font as _, FontArc};

use crate::glyphs::Raster;

/// Capital letters are this fraction of the em of the font. The `size` of overlay text is the
/// height of a capital letter, so the em is `size / CAP_RATIO`.
pub(crate) const CAP_RATIO: f32 = 0.7;
/// Smallest and largest em in pixels that is rasterized. A text outside the range is drawn at the
/// nearest end, so that one glyph always fits the atlas.
const MIN_EM: f32 = 4.0;
const MAX_EM: f32 = 400.0;
/// Fonts larger than this are not read (a CJK collection is about 20 MiB).
const MAX_FONT_BYTES: u64 = 48 << 20;
/// Advance of the box that stands for a character the font lacks, in ems.
const MISSING_ADVANCE: f32 = 0.6;

/// Why a font could not be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontError(String);

impl std::fmt::Display for FontError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FontError {}

/// One font: outlines and metrics, shared by [`FontHandle`]s.
pub struct Font {
    name: String,
    inner: FontArc,
}

impl std::fmt::Debug for Font {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Font({:?})", self.name)
    }
}

/// The em in whole pixels for text whose capital letters are `size` pixels high.
pub(crate) fn em_for(size: f32) -> f32 {
    (size / CAP_RATIO).round().clamp(MIN_EM, MAX_EM)
}

impl Font {
    /// A font from the bytes of a `.ttf`, `.otf` or `.ttc` file (the first font of a collection).
    /// `name` appears in messages.
    pub fn from_bytes(name: &str, bytes: Vec<u8>) -> Result<Self, FontError> {
        let inner = FontArc::try_from_vec(bytes).map_err(|e| FontError(format!("font {name}: {e}")))?;
        Ok(Self { name: name.to_string(), inner })
    }

    /// A font read from `path`.
    pub fn from_file(path: &Path) -> Result<Self, FontError> {
        let bytes = read_font_file(path).map_err(FontError)?;
        let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        Self::from_bytes(&name, bytes)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the font has a glyph for `c`. White space always counts: it is spacing, not ink.
    pub fn has_glyph(&self, c: char) -> bool {
        c.is_whitespace() || self.inner.glyph_id(c).0 != 0
    }

    /// Pixels per font unit at `em` pixels per em.
    fn unit(&self, em: f32) -> f32 {
        em / self.inner.units_per_em().unwrap_or(1000.0)
    }

    /// Distance from one character to the next, in pixels. A character without a glyph has the
    /// advance of the box that replaces it.
    pub fn advance(&self, c: char, em: f32) -> f32 {
        if c.is_control() {
            return 0.0;
        }
        if !self.has_glyph(c) {
            return MISSING_ADVANCE * em;
        }
        self.inner.h_advance_unscaled(self.inner.glyph_id(c)) * self.unit(em)
    }

    /// Distance from the top of a line to its baseline.
    pub fn ascent(&self, em: f32) -> f32 {
        self.inner.ascent_unscaled() * self.unit(em)
    }

    /// Distance from the baseline of one line to the baseline of the next.
    pub fn line_height(&self, em: f32) -> f32 {
        (self.inner.height_unscaled() + self.inner.line_gap_unscaled()) * self.unit(em)
    }

    /// The picture of `c` at `em` pixels per em. `None` for a character without ink (a space).
    /// A character the font lacks gives the hollow box ([`crate::glyphs::MISSING`] asks for it).
    pub(crate) fn raster(&self, c: char, em: f32) -> Option<Raster> {
        if c == crate::glyphs::MISSING {
            return Some(missing_box(em));
        }
        let scale = ab_glyph::PxScale::from(self.unit(em) * self.inner.height_unscaled());
        let glyph = self.inner.glyph_id(c).with_scale_and_position(scale, ab_glyph::point(0.0, 0.0));
        let outline = self.inner.outline_glyph(glyph)?;
        let bounds = outline.px_bounds();
        let (w, h) = (bounds.width().ceil() as u32, bounds.height().ceil() as u32);
        if w == 0 || h == 0 {
            return None;
        }
        let mut coverage = vec![0u8; (w * h) as usize];
        outline.draw(|x, y, c| {
            if x < w && y < h {
                coverage[(y * w + x) as usize] = (c.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        });
        Some(Raster { w, h, left: bounds.min.x, top: bounds.min.y, coverage })
    }
}

/// The hollow box of a character without a glyph: as high as a capital letter, on the baseline.
fn missing_box(em: f32) -> Raster {
    let (w, h) = ((MISSING_ADVANCE * em * 0.8).round().max(3.0) as u32, (CAP_RATIO * em).round().max(3.0) as u32);
    let line = ((em / 14.0).round() as u32).max(1);
    let mut coverage = vec![0u8; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            if x < line || y < line || x >= w - line.min(w) || y >= h - line.min(h) {
                coverage[(y * w + x) as usize] = 255;
            }
        }
    }
    Raster { w, h, left: (MISSING_ADVANCE * em * 0.1).round(), top: -(h as f32), coverage }
}

/// A shared font. Cloning is cheap; two handles are equal when they are the same font object.
#[derive(Clone)]
pub struct FontHandle(Arc<Font>);

impl FontHandle {
    pub fn new(font: Font) -> Self {
        Self(Arc::new(font))
    }
}

impl Deref for FontHandle {
    type Target = Font;
    fn deref(&self) -> &Font {
        &self.0
    }
}

impl PartialEq for FontHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for FontHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

fn read_font_file(path: &Path) -> Result<Vec<u8>, String> {
    let size = std::fs::metadata(path).map_err(|e| format!("font file {}: {e}", path.display()))?.len();
    if size == 0 || size > MAX_FONT_BYTES {
        return Err(format!("font file {}: {size} bytes is not a usable size (at most {MAX_FONT_BYTES})", path.display()));
    }
    std::fs::read(path).map_err(|e| format!("font file {}: {e}", path.display()))
}

/// The files that are tried for the default font, in order: a Korean font of Windows, then
/// the common Hangul fonts of Linux and macOS. Nothing is bundled with the engine.
pub fn system_font_candidates() -> Vec<PathBuf> {
    let root = std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from("C:\\Windows"), PathBuf::from);
    vec![
        root.join("Fonts").join("malgun.ttf"),
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc".into(),
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc".into(),
        "/usr/share/fonts/truetype/nanum/NanumGothic.ttf".into(),
        "/System/Library/Fonts/AppleSDGothicNeo.ttc".into(),
    ]
}

/// The first system font of [`system_font_candidates`] that can be read, or `None`.
pub fn system_font() -> Option<FontHandle> {
    system_font_candidates().iter().find_map(|path| Font::from_file(path).ok()).map(FontHandle::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_that_are_not_a_font_are_an_error_that_names_the_font() {
        let error = Font::from_bytes("my font", vec![0; 64]).unwrap_err();
        assert!(error.to_string().contains("my font"), "{error}");
        assert!(Font::from_file(Path::new("no/such/font.ttf")).is_err());
    }

    #[test]
    fn the_em_follows_the_capital_height_and_stays_in_range() {
        assert_eq!(em_for(14.0), 20.0);
        assert_eq!(em_for(56.0), 80.0);
        assert_eq!(em_for(0.5), MIN_EM);
        assert_eq!(em_for(10_000.0), MAX_EM);
    }

    #[test]
    fn a_system_font_measures_and_rasterizes_korean() {
        let Some(font) = system_font() else {
            eprintln!("no system font with Hangul on this machine: the font checks are skipped");
            return;
        };
        let em = em_for(20.0);
        assert!(font.has_glyph('가') && font.has_glyph('A') && font.has_glyph(' '));
        assert!(!font.has_glyph('\u{FFFF}'));
        assert!(font.advance('가', em) > font.advance('i', em));
        assert_eq!(font.advance('\n', em), 0.0);
        assert_eq!(font.advance('\u{FFFF}', em), MISSING_ADVANCE * em, "a missing character takes the room of its box");
        assert!(font.line_height(em) > em && font.ascent(em) > CAP_RATIO * em);

        let syllable = font.raster('가', em).expect("a syllable has ink");
        assert_eq!(syllable.coverage.len(), (syllable.w * syllable.h) as usize);
        assert!(syllable.coverage.iter().any(|c| *c > 200) && syllable.top < 0.0, "ink above the baseline");
        assert!(font.raster(' ', em).is_none(), "a space has no ink");
        let boxed = font.raster(crate::glyphs::MISSING, em).expect("the box of a missing character");
        assert_eq!(boxed.coverage[0], 255, "the corner of the box is inked");
        assert_eq!(boxed.coverage[(boxed.w * boxed.h / 2 + boxed.w / 2) as usize], 0, "and its middle is hollow");
    }
}
