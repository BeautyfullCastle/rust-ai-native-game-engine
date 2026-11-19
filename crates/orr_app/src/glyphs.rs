//! The glyph atlas of the overlay text and the pass that draws textured quads from it.
//!
//! [`GlyphAtlas`] is the bookkeeping on the CPU: which glyph (character and size) sits where in
//! a square picture of bounded size. A glyph is rasterized the first time a frame needs it and
//! then reused. When the picture is full, it is cleared and the glyphs of the frame are placed
//! again (grow, then reset), so a long run with many sizes and characters stays within one
//! texture. [`GlyphPass`] keeps the texture on the GPU and draws one textured quad per glyph.

use std::collections::BTreeMap;

use orr_render::orr_rhi::{
    Binding, Blend, BufferDesc, BufferUsage, ColorAttachment, Command, PipelineDesc, Rhi, SamplerDesc, TextureDesc, TextureFormat, TextureUpload, TextureUsage, VertexAttr, VertexFormat, VertexLayout,
    VertexStep, Wgpu,
};

use crate::font::{em_for, FontHandle};
use crate::overlay::Overlay;

/// Stands for every character the font lacks: the atlas holds one hollow box for it.
/// U+FFFF is a noncharacter, so no font has a glyph for it.
pub(crate) const MISSING: char = '\u{FFFF}';

/// Width and height of the atlas texture in pixels (4 MiB as RGBA8).
pub(crate) const ATLAS_EXTENT: u32 = 1024;

/// Empty pixels around every glyph, so that a neighbor never shows through.
const PAD: u32 = 1;

/// The coverage picture of one glyph and where it sits relative to its pen position on the
/// baseline (`left` to the right, `top` down, so it is negative above the baseline).
pub(crate) struct Raster {
    pub(crate) w: u32,
    pub(crate) h: u32,
    pub(crate) left: f32,
    pub(crate) top: f32,
    /// `w * h` coverage values, row by row, top row first.
    pub(crate) coverage: Vec<u8>,
}

/// A glyph of the atlas: the character and the em size in whole pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct GlyphKey {
    pub(crate) ch: char,
    pub(crate) em: u16,
}

/// Where a glyph is in the atlas (without its padding).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Slot {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) w: u32,
    pub(crate) h: u32,
    pub(crate) left: f32,
    pub(crate) top: f32,
}

/// A rectangle of the atlas to write: RGBA8, white with the coverage as alpha, padding included.
pub(crate) struct Upload {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) w: u32,
    pub(crate) h: u32,
    pub(crate) rgba: Vec<u8>,
}

struct Full;

pub(crate) struct GlyphAtlas {
    extent: u32,
    /// `None` for a glyph without ink (a space) or one that is larger than the atlas.
    slots: BTreeMap<GlyphKey, Option<Slot>>,
    /// Next free place of the current row, and the height of the row.
    cursor: [u32; 2],
    row: u32,
}

impl GlyphAtlas {
    pub(crate) fn new(extent: u32) -> Self {
        Self { extent, slots: BTreeMap::new(), cursor: [0, 0], row: 0 }
    }

    pub(crate) fn extent(&self) -> u32 {
        self.extent
    }

    pub(crate) fn slot(&self, key: GlyphKey) -> Option<&Slot> {
        self.slots.get(&key).and_then(Option::as_ref)
    }

    pub(crate) fn clear(&mut self) {
        self.slots.clear();
        self.cursor = [0, 0];
        self.row = 0;
    }

    fn place(&mut self, key: GlyphKey, raster: &mut dyn FnMut(GlyphKey) -> Option<Raster>, uploads: &mut Vec<Upload>) -> Result<(), Full> {
        if self.slots.contains_key(&key) {
            return Ok(());
        }
        let Some(r) = raster(key) else {
            self.slots.insert(key, None);
            return Ok(());
        };
        let (pw, ph) = (r.w + 2 * PAD, r.h + 2 * PAD);
        if pw > self.extent || ph > self.extent {
            self.slots.insert(key, None);
            return Ok(());
        }
        if self.cursor[0] + pw > self.extent {
            self.cursor = [0, self.cursor[1] + self.row];
            self.row = 0;
        }
        if self.cursor[1] + ph > self.extent {
            return Err(Full);
        }
        let [x, y] = self.cursor;
        let mut rgba = vec![0u8; (pw * ph * 4) as usize];
        for row in 0..r.h {
            for col in 0..r.w {
                let at = (((row + PAD) * pw + col + PAD) * 4) as usize;
                rgba[at..at + 4].copy_from_slice(&[255, 255, 255, r.coverage[(row * r.w + col) as usize]]);
            }
        }
        uploads.push(Upload { x, y, w: pw, h: ph, rgba });
        self.slots.insert(key, Some(Slot { x: x + PAD, y: y + PAD, w: r.w, h: r.h, left: r.left, top: r.top }));
        self.cursor[0] += pw;
        self.row = self.row.max(ph);
        Ok(())
    }

    /// Makes sure every glyph of `keys` has a place, calling `raster` for the new ones only and
    /// adding what to write to `uploads`. When the atlas is full it is cleared once and all the
    /// keys are placed again (the old uploads are dropped). Returns how many glyphs still do not
    /// fit (the frame needs more than one atlas holds); they are not drawn.
    pub(crate) fn prepare(&mut self, keys: &[GlyphKey], raster: &mut dyn FnMut(GlyphKey) -> Option<Raster>, uploads: &mut Vec<Upload>) -> usize {
        if keys.iter().any(|key| self.place(*key, raster, uploads).is_err()) {
            self.clear();
            uploads.clear();
        }
        keys.iter().filter(|key| self.place(**key, raster, uploads).is_err()).count()
    }
}

/// One glyph quad: `[x, y, w, h]` in pixels, `[u0, v0, u1, v1]` in the atlas, `[r, g, b, a]`.
type Instance = [f32; 12];

const ATTRS: [VertexAttr; 3] = [
    VertexAttr { location: 0, format: VertexFormat::Float32x4, offset: 0 },
    VertexAttr { location: 1, format: VertexFormat::Float32x4, offset: 16 },
    VertexAttr { location: 2, format: VertexFormat::Float32x4, offset: 32 },
];

/// Draws the text of an overlay with a font: textured quads from the glyph atlas.
pub(crate) struct GlyphPass {
    rhi: Wgpu,
    pipeline: <Wgpu as Rhi>::Pipeline,
    bind: <Wgpu as Rhi>::BindGroup,
    globals: <Wgpu as Rhi>::Buffer,
    texture: <Wgpu as Rhi>::Texture,
    instances: <Wgpu as Rhi>::Buffer,
    capacity: usize,
    atlas: GlyphAtlas,
    /// The font of the glyphs in the atlas.
    font: Option<FontHandle>,
    keys: Vec<GlyphKey>,
    uploads: Vec<Upload>,
    quads: Vec<Instance>,
    warned_full: bool,
}

impl GlyphPass {
    pub(crate) fn new(rhi: Wgpu, format: TextureFormat) -> Self {
        let texture = rhi.create_texture(&TextureDesc {
            label: "overlay glyph atlas",
            width: ATLAS_EXTENT,
            height: ATLAS_EXTENT,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsage::TEXTURE_BINDING.or(TextureUsage::COPY_DST),
            sample_count: 1,
            view_formats: &[],
        });
        let view = rhi.create_texture_view(&texture, None);
        let sampler = rhi.create_sampler(&SamplerDesc { linear: false, compare: false, repeat: false, mipmaps: false });
        let globals = rhi.create_buffer(&BufferDesc { label: "overlay glyph size", size: 16, usage: BufferUsage::UNIFORM.or(BufferUsage::COPY_DST) });
        let shader = rhi.create_shader("overlay glyphs", include_str!("glyph.wgsl"));
        let pipeline = rhi.create_pipeline(&PipelineDesc::color(
            "overlay glyphs",
            &shader,
            ("vs_glyph", "fs_glyph"),
            &[VertexLayout { stride: std::mem::size_of::<Instance>() as u64, step: VertexStep::Instance, attrs: &ATTRS }],
            format,
            Blend::Alpha,
        ));
        let bind = rhi.create_bind_group(
            &pipeline,
            0,
            &[Binding::Uniform { binding: 0, buffer: &globals }, Binding::Texture { binding: 1, view: &view }, Binding::Sampler { binding: 2, sampler: &sampler }],
        );
        let capacity = 256;
        let instances = Self::buffer(&rhi, capacity);
        Self { rhi, pipeline, bind, globals, texture, instances, capacity, atlas: GlyphAtlas::new(ATLAS_EXTENT), font: None, keys: Vec::new(), uploads: Vec::new(), quads: Vec::new(), warned_full: false }
    }

    fn buffer(rhi: &Wgpu, capacity: usize) -> <Wgpu as Rhi>::Buffer {
        rhi.create_buffer(&BufferDesc { label: "overlay glyph quads", size: (capacity * std::mem::size_of::<Instance>()) as u64, usage: BufferUsage::VERTEX.or(BufferUsage::COPY_DST) })
    }

    /// Draws the text of `overlay` with `font` into `view` (`size` pixels) and keeps what is there.
    pub(crate) fn draw(&mut self, view: &<Wgpu as Rhi>::TextureView, size: (u32, u32), overlay: &Overlay, font: &FontHandle) {
        if self.font.as_ref() != Some(font) {
            self.atlas.clear();
            self.font = Some(font.clone());
        }
        self.quads.clear();
        self.keys.clear();
        for item in overlay.texts() {
            let em = em_for(item.size) as u16;
            for c in item.text.chars().filter(|c| !c.is_control()) {
                self.keys.push(GlyphKey { ch: if font.has_glyph(c) { c } else { MISSING }, em });
            }
        }
        self.keys.sort_unstable();
        self.keys.dedup();
        self.uploads.clear();
        let unplaced = self.atlas.prepare(&self.keys, &mut |key| font.raster(key.ch, f32::from(key.em)), &mut self.uploads);
        if unplaced > 0 && !self.warned_full {
            self.warned_full = true;
            eprintln!("overlay text: {unplaced} glyphs do not fit the {0}x{0} glyph atlas in one frame and are not drawn", self.atlas.extent());
        }
        for upload in &self.uploads {
            let write = self.rhi.write_texture_rgba8(&self.texture, &TextureUpload { origin: [upload.x, upload.y], width: upload.w, height: upload.h, bytes_per_row: upload.w * 4, data: &upload.rgba });
            if let Err(e) = write {
                eprintln!("overlay text: the glyph atlas could not be written: {e:?}");
            }
        }
        glyph_quads(overlay, font, &self.atlas, &mut self.quads);
        if self.quads.is_empty() {
            return;
        }
        if self.quads.len() > self.capacity {
            self.capacity = self.quads.len().next_power_of_two();
            self.instances = Self::buffer(&self.rhi, self.capacity);
        }
        self.rhi.write_buffer(&self.globals, 0, bytemuck::cast_slice(&[size.0 as f32, size.1 as f32, 0.0, 0.0]));
        self.rhi.write_buffer(&self.instances, 0, bytemuck::cast_slice(&self.quads));
        let commands = [
            Command::SetPipeline(&self.pipeline),
            Command::SetBindGroup(0, &self.bind),
            Command::SetVertexBuffer(0, &self.instances),
            Command::Draw { vertices: 0..6, instances: 0..self.quads.len() as u32 },
        ];
        let mut encoder = self.rhi.create_encoder("overlay glyphs");
        self.rhi.encode_render_pass(&mut encoder, "overlay glyphs", &ColorAttachment { view, clear: None, resolve: None }, &commands);
        self.rhi.submit(encoder);
    }
}

/// The quads of all the text of `overlay`, in order. Glyphs sit on whole pixels, so the atlas
/// is read one texel to one pixel. A glyph that has no place in the atlas is left out.
pub(crate) fn glyph_quads(overlay: &Overlay, font: &FontHandle, atlas: &GlyphAtlas, quads: &mut Vec<Instance>) {
    let extent = atlas.extent() as f32;
    for item in overlay.texts() {
        let em = em_for(item.size);
        let baseline = (item.pos[1] + item.size).round();
        let mut pen = item.pos[0];
        for c in item.text.chars() {
            let advance = font.advance(c, em);
            if c.is_control() {
                continue;
            }
            let key = GlyphKey { ch: if font.has_glyph(c) { c } else { MISSING }, em: em as u16 };
            if let Some(slot) = atlas.slot(key) {
                let (x, y) = (pen.round() + slot.left, baseline + slot.top);
                let (u0, v0) = (slot.x as f32 / extent, slot.y as f32 / extent);
                let (u1, v1) = ((slot.x + slot.w) as f32 / extent, (slot.y + slot.h) as f32 / extent);
                let [r, g, b, a] = item.color;
                quads.push([x, y, slot.w as f32, slot.h as f32, u0, v0, u1, v1, r, g, b, a]);
            }
            pen += advance;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raster(w: u32, h: u32) -> Raster {
        Raster { w, h, left: 1.0, top: -(h as f32), coverage: vec![200; (w * h) as usize] }
    }

    fn key(c: char) -> GlyphKey {
        GlyphKey { ch: c, em: 20 }
    }

    #[test]
    fn a_glyph_is_rasterized_once_and_then_reused() {
        let mut atlas = GlyphAtlas::new(256);
        let mut calls = Vec::new();
        let mut uploads = Vec::new();
        let keys = [key('가'), key('나'), key(' ')];

        let mut make = |k: GlyphKey| {
            calls.push(k.ch);
            (k.ch != ' ').then(|| raster(10, 12))
        };
        assert_eq!(atlas.prepare(&keys, &mut make, &mut uploads), 0);
        assert_eq!(uploads.len(), 2, "a space has no picture to write");
        uploads.clear();
        assert_eq!(atlas.prepare(&keys, &mut make, &mut uploads), 0);

        assert_eq!(calls, vec!['가', '나', ' '], "the second frame rasterized nothing, not even the space");
        assert!(uploads.is_empty());
        assert!(atlas.slot(key('가')).is_some() && atlas.slot(key(' ')).is_none());
        assert_ne!(atlas.slot(key('가')).map(|s| (s.x, s.y)), atlas.slot(key('나')).map(|s| (s.x, s.y)));
    }

    #[test]
    fn the_same_character_at_another_size_is_another_glyph() {
        let mut atlas = GlyphAtlas::new(256);
        let mut uploads = Vec::new();
        let count = std::cell::Cell::new(0);
        let mut make = |_: GlyphKey| {
            count.set(count.get() + 1);
            Some(raster(8, 8))
        };
        atlas.prepare(&[key('가'), GlyphKey { ch: '가', em: 40 }], &mut make, &mut uploads);
        assert_eq!(count.get(), 2);
    }

    #[test]
    fn glyphs_never_leave_the_atlas_and_never_overlap() {
        let extent = 64;
        let mut atlas = GlyphAtlas::new(extent);
        let mut uploads = Vec::new();
        let mut make = |_: GlyphKey| Some(raster(10, 10));
        let keys: Vec<GlyphKey> = ('가'..).take(20).map(key).collect();
        let unplaced = atlas.prepare(&keys, &mut make, &mut uploads);

        // 12 x 12 with padding: 5 per row, 5 rows = 25 places, so 20 glyphs fit at once.
        assert_eq!(unplaced, 0);
        let slots: Vec<Slot> = keys.iter().map(|k| *atlas.slot(*k).unwrap()).collect();
        for (i, a) in slots.iter().enumerate() {
            assert!(a.x + a.w <= extent && a.y + a.h <= extent, "{a:?} is inside the atlas");
            for b in &slots[i + 1..] {
                let apart = a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
                assert!(apart, "{a:?} and {b:?} overlap");
            }
        }
        for u in &uploads {
            assert!(u.x + u.w <= extent && u.y + u.h <= extent);
            assert_eq!(u.rgba.len(), (u.w * u.h * 4) as usize);
        }
    }

    #[test]
    fn a_full_atlas_is_cleared_once_and_the_frame_is_placed_again() {
        let mut atlas = GlyphAtlas::new(64);
        let mut uploads = Vec::new();
        let count = std::cell::Cell::new(0);
        let mut make = |_: GlyphKey| {
            count.set(count.get() + 1);
            Some(raster(10, 10))
        };
        let old: Vec<GlyphKey> = ('가'..).take(20).map(key).collect();
        atlas.prepare(&old, &mut make, &mut uploads);
        let before = count.get();

        let new: Vec<GlyphKey> = ('A'..).take(10).map(key).collect();
        uploads.clear();
        let unplaced = atlas.prepare(&new, &mut make, &mut uploads);

        assert_eq!(unplaced, 0);
        assert_eq!(count.get() - before, 16, "six before the clear (the sixth found no room), ten after it");
        assert!(atlas.slot(old[0]).is_none(), "the old glyphs are gone");
        assert!(new.iter().all(|k| atlas.slot(*k).is_some()));
        assert_eq!(uploads.len(), 10, "only the places of the new frame are written");
    }

    #[test]
    fn a_frame_larger_than_the_atlas_draws_what_fits_and_counts_the_rest() {
        let mut atlas = GlyphAtlas::new(64);
        let mut uploads = Vec::new();
        let mut make = |_: GlyphKey| Some(raster(10, 10));
        let keys: Vec<GlyphKey> = ('가'..).take(40).map(key).collect();

        let unplaced = atlas.prepare(&keys, &mut make, &mut uploads);

        assert_eq!(unplaced, 15, "25 places of 12 x 12 in 64 x 64");
        assert_eq!(keys.iter().filter(|k| atlas.slot(**k).is_some()).count(), 25);
    }

    #[test]
    fn a_glyph_larger_than_the_atlas_is_skipped_without_clearing_it() {
        let mut atlas = GlyphAtlas::new(32);
        let mut uploads = Vec::new();
        let mut make = |k: GlyphKey| Some(if k.ch == 'X' { raster(40, 40) } else { raster(8, 8) });
        let unplaced = atlas.prepare(&[key('가'), key('X')], &mut make, &mut uploads);
        assert_eq!(unplaced, 0);
        assert!(atlas.slot(key('X')).is_none() && atlas.slot(key('가')).is_some());
    }

    /// Draws `overlay` with the glyph pass into a black picture and returns the RGBA8 pixels.
    fn render(pass: &mut GlyphPass, rhi: &Wgpu, overlay: &Overlay, size: (u32, u32)) -> Vec<u8> {
        let target = orr_render::OffscreenTarget::new(rhi, size.0, size.1, TextureFormat::Rgba8UnormSrgb);
        target.render_into(|_, view, size| pass.draw(view, size, overlay, overlay.font().expect("the overlay has a font")));
        target.read_rgba8()
    }

    #[test]
    fn korean_text_pixels_appear_in_the_text_box_and_nowhere_else_and_a_second_frame_is_the_same() {
        let Some(font) = crate::font::system_font() else {
            eprintln!("no system font with Hangul on this machine: the GPU text check is skipped");
            return;
        };
        let Ok(rhi) = Wgpu::headless(orr_render::orr_rhi::WgpuOptions::default()) else {
            eprintln!("no GPU adapter: the GPU text check is skipped");
            return;
        };
        let (width, height) = (320u32, 160u32);
        let mut overlay = Overlay::default();
        overlay.set_font(Some(font));
        overlay.set_screen_size([width as f32, height as f32]);
        let rect = crate::typeset::ScreenRect::new(40.0, 50.0, 240.0, 60.0);
        let used = overlay.text_box(rect, 20.0, [1.0; 4], "별 5개를 모으세요 일시정지 승리!", crate::typeset::Align::Center)[1];
        assert!(overlay.texts().len() >= 2, "the sentence wraps in a box 240 pixels wide");

        let mut pass = GlyphPass::new(rhi.clone(), TextureFormat::Rgba8UnormSrgb);
        let first = render(&mut pass, &rhi, &overlay, (width, height));

        let (mut inside, mut outside) = (0, 0);
        for y in 0..height {
            for x in 0..width {
                if first[((y * width + x) * 4) as usize] < 128 {
                    continue;
                }
                // The lines the box returned, plus a few pixels for the bearing and the strokes of a glyph.
                let near = (x as f32) >= rect.x - 3.0 && (x as f32) < rect.x + rect.w + 3.0 && (y as f32) >= rect.y - 3.0 && (y as f32) < rect.y + used + 3.0;
                if near {
                    inside += 1;
                } else {
                    outside += 1;
                }
            }
        }
        assert!(inside > 150, "Korean text pixels in the box: {inside}");
        assert_eq!(outside, 0, "no text pixels outside the box");

        let second = render(&mut pass, &rhi, &overlay, (width, height));
        assert_eq!(first, second, "the glyphs of the second frame come from the atlas and look the same");
    }
}
