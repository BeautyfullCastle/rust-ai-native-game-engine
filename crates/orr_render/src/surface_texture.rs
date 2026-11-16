//! Images that surface shaders read (see the textures section of [`crate::surface`]).
//! A texture is uploaded with its whole mip chain, made on the CPU, and read
//! through a repeating sampler that filters between levels.

use crate::surface::TextureKind;
use orr_rhi::{mip_level_size, Rhi, TextureDesc, TextureFormat, TextureUpload, TextureUsage};
use std::sync::atomic::{AtomicU64, Ordering};

/// Largest width and height of a surface texture's image, in pixels.
pub const MAX_SURFACE_TEXTURE_SIZE: u32 = 2048;

/// A texture added to a renderer. Index into the textures added to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextureId(pub u32);

/// An image prepared for surface shaders: RGBA8 pixels of every mip level.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceTexture {
    kind: TextureKind,
    width: u32,
    height: u32,
    /// RGBA8 rows of each level, level 0 first, each half the size of the one before.
    levels: Vec<Vec<u8>>,
}

fn srgb_to_linear(byte: u8) -> f32 {
    let c = f32::from(byte) / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(value: f32) -> u8 {
    let c = value.clamp(0.0, 1.0);
    let encoded = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (encoded * 255.0).round() as u8
}

/// Reads a byte as a linear value, and writes one back.
type Decode = fn(u8) -> f32;
type Encode = fn(f32) -> u8;

fn unorm(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The pixels of a `src` long side that destination pixel `dst` of a `dst_len` long side covers,
/// each with the share of that source pixel the destination covers (1 when the sides halve evenly).
fn overlaps(src: u32, dst_len: u32, dst: u32) -> impl Iterator<Item = (u32, f32)> {
    let (src, dst_len, dst) = (u64::from(src), u64::from(dst_len), u64::from(dst));
    // Both sides in units of 1 / dst_len of a source pixel: the destination pixel spans dst * src .. (dst + 1) * src.
    let (from, to) = (dst * src, (dst + 1) * src);
    (from / dst_len..to.div_ceil(dst_len)).map(move |i| {
        let covered = to.min((i + 1) * dst_len) - from.max(i * dst_len);
        (i as u32, covered as f32 / dst_len as f32)
    })
}

impl SurfaceTexture {
    /// Prepares `rgba`, `width` × `height` RGBA8 pixels in rows from top to
    /// bottom, and its smaller levels. A color texture holds sRGB colors and
    /// its levels are averaged in linear light; a data texture holds numbers
    /// and its levels are averaged as they are.
    pub fn from_rgba8(kind: TextureKind, width: u32, height: u32, rgba: &[u8]) -> Result<Self, String> {
        if width == 0 || height == 0 || width > MAX_SURFACE_TEXTURE_SIZE || height > MAX_SURFACE_TEXTURE_SIZE {
            return Err(format!("a surface texture must be 1 to {MAX_SURFACE_TEXTURE_SIZE} pixels per side, not {width} x {height}"));
        }
        if rgba.len() != width as usize * height as usize * 4 {
            return Err(format!("{width} x {height} RGBA pixels need {} bytes, not {}", width as usize * height as usize * 4, rgba.len()));
        }
        let (decode, encode): (Decode, Encode) = match kind {
            TextureKind::Color => (srgb_to_linear, linear_to_srgb),
            TextureKind::Data => (|b| f32::from(b) / 255.0, unorm),
        };
        let mut levels = vec![rgba.to_vec()];
        // Linear values of the current level; alpha is always linear.
        let mut values: Vec<[f32; 4]> = rgba.chunks_exact(4).map(|p| [decode(p[0]), decode(p[1]), decode(p[2]), f32::from(p[3]) / 255.0]).collect();
        let (mut w, mut h) = (width, height);
        for level in 1..orr_rhi::mip_level_count(width, height) {
            let (nw, nh) = mip_level_size(width, height, level);
            let mut next = vec![[0.0f32; 4]; nw as usize * nh as usize];
            // Each pixel averages the area it covers below it: a 2 x 2 block, or, on an odd side, a
            // block that takes in part of the next pixel too. A side of one pixel averages 1 x 2.
            for y in 0..nh {
                for x in 0..nw {
                    let mut sum = [0.0f32; 4];
                    let mut total = 0.0f32;
                    for (sy, wy) in overlaps(h, nh, y) {
                        for (sx, wx) in overlaps(w, nw, x) {
                            let tap = values[(sy * w + sx) as usize];
                            let weight = wx * wy;
                            for c in 0..4 {
                                sum[c] += tap[c] * weight;
                            }
                            total += weight;
                        }
                    }
                    next[(y * nw + x) as usize] = sum.map(|s| s / total);
                }
            }
            levels.push(next.iter().flat_map(|p| [encode(p[0]), encode(p[1]), encode(p[2]), unorm(p[3])]).collect());
            values = next;
            (w, h) = (nw, nh);
        }
        Ok(Self { kind, width, height, levels })
    }

    /// A one pixel texture of `rgba`, each channel from 0 to 1.
    pub fn solid(kind: TextureKind, rgba: [f32; 4]) -> Self {
        let channel = if kind == TextureKind::Color { linear_to_srgb } else { unorm };
        let pixel = vec![channel(rgba[0]), channel(rgba[1]), channel(rgba[2]), unorm(rgba[3])];
        Self { kind, width: 1, height: 1, levels: vec![pixel] }
    }

    pub fn kind(&self) -> TextureKind {
        self.kind
    }

    /// Width and height of the image, level 0.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Mip levels, level 0 included.
    pub fn level_count(&self) -> u32 {
        self.levels.len() as u32
    }
}

/// Stamps are unique in the process, so a cached binding never confuses two textures.
static NEXT_STAMP: AtomicU64 = AtomicU64::new(1);

/// A surface texture on the GPU.
pub(crate) struct GpuTexture<B: Rhi> {
    /// Changes with every upload, so bindings that use the texture are rebuilt.
    pub stamp: u64,
    _texture: B::Texture,
    pub view: B::TextureView,
}

impl<B: Rhi> GpuTexture<B> {
    pub fn upload(rhi: &B, texture: &SurfaceTexture) -> Self {
        let format = match texture.kind {
            TextureKind::Color => TextureFormat::Rgba8UnormSrgb,
            TextureKind::Data => TextureFormat::Rgba8Unorm,
        };
        let desc = TextureDesc {
            label: "surface texture",
            width: texture.width,
            height: texture.height,
            format,
            usage: TextureUsage::TEXTURE_BINDING | TextureUsage::COPY_DST,
            sample_count: 1,
            view_formats: &[],
        };
        let gpu = rhi.create_texture_mipmapped(&desc, texture.level_count());
        for (level, pixels) in texture.levels.iter().enumerate() {
            let (width, height) = mip_level_size(texture.width, texture.height, level as u32);
            let upload = TextureUpload { origin: [0, 0], width, height, bytes_per_row: width * 4, data: pixels };
            // A backend without mip levels refuses the smaller levels and samples level 0.
            if rhi.write_texture_rgba8_mip(&gpu, level as u32, &upload).is_err() {
                break;
            }
        }
        let view = rhi.create_texture_view(&gpu, None);
        Self { stamp: NEXT_STAMP.fetch_add(1, Ordering::Relaxed), _texture: gpu, view }
    }
}

/// The textures added to one renderer, by [`TextureId`].
pub(crate) struct TextureTable<B: Rhi> {
    textures: Vec<GpuTexture<B>>,
}

impl<B: Rhi> Default for TextureTable<B> {
    fn default() -> Self {
        Self { textures: Vec::new() }
    }
}

impl<B: Rhi> TextureTable<B> {
    pub fn add(&mut self, rhi: &B, texture: &SurfaceTexture) -> TextureId {
        self.textures.push(GpuTexture::upload(rhi, texture));
        TextureId(self.textures.len() as u32 - 1)
    }

    /// Replaces the texture of `id`; an id the table does not have is left alone.
    pub fn replace(&mut self, rhi: &B, id: TextureId, texture: &SurfaceTexture) {
        if let Some(slot) = self.textures.get_mut(id.0 as usize) {
            *slot = GpuTexture::upload(rhi, texture);
        }
    }

    pub fn get(&self, id: TextureId) -> Option<&GpuTexture<B>> {
        self.textures.get(id.0 as usize)
    }

    /// The texture of each slot: the chosen one where the table has it, the shader's default elsewhere.
    pub fn resolve<'a>(&'a self, chosen: &[Option<TextureId>; crate::surface::MAX_SURFACE_TEXTURES], defaults: &'a [GpuTexture<B>]) -> [&'a GpuTexture<B>; crate::surface::MAX_SURFACE_TEXTURES] {
        std::array::from_fn(|k| chosen[k].and_then(|id| self.get(id)).unwrap_or(&defaults[k]))
    }
}

/// One default texture per slot of `shader`: its declared default, white for an undeclared slot.
pub(crate) fn default_textures<B: Rhi>(rhi: &B, shader: &crate::SurfaceShader) -> Vec<GpuTexture<B>> {
    (0..crate::surface::MAX_SURFACE_TEXTURES)
        .map(|k| {
            let solid = shader.textures().get(k).map_or(SurfaceTexture::solid(TextureKind::Color, [1.0; 4]), |slot| SurfaceTexture::solid(slot.kind, slot.default));
            GpuTexture::upload(rhi, &solid)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_level_halves_the_one_before_down_to_one_pixel() {
        // 4 x 2: levels 4 x 2, 2 x 1 and 1 x 1.
        let mut rgba = Vec::new();
        for v in [0u8, 255, 0, 255, 255, 0, 255, 0] {
            rgba.extend([v, v, v, 255]);
        }
        let texture = SurfaceTexture::from_rgba8(TextureKind::Data, 4, 2, &rgba).unwrap();
        assert_eq!((texture.size(), texture.level_count()), ((4, 2), 3));
        // Each level 1 pixel averages a 2 x 2 block of level 0; level 2 averages the 2 x 1 row.
        assert_eq!([texture.levels[1][0], texture.levels[1][4]], [128, 128]);
        assert_eq!(texture.levels[1].len(), 2 * 4);
        assert_eq!(texture.levels[2], [128, 128, 128, 255]);
    }

    #[test]
    fn color_levels_average_in_linear_light() {
        // Black and white average to linear 0.5, which is sRGB 188, not 128.
        let rgba = [0, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255];
        let texture = SurfaceTexture::from_rgba8(TextureKind::Color, 2, 2, &rgba).unwrap();
        assert_eq!(texture.levels[1][0], 188);
    }

    #[test]
    fn sizes_outside_the_limits_and_short_pixel_data_are_refused() {
        assert!(SurfaceTexture::from_rgba8(TextureKind::Color, 0, 4, &[]).is_err());
        assert!(SurfaceTexture::from_rgba8(TextureKind::Color, MAX_SURFACE_TEXTURE_SIZE + 1, 1, &vec![0; (MAX_SURFACE_TEXTURE_SIZE as usize + 1) * 4]).is_err());
        assert!(SurfaceTexture::from_rgba8(TextureKind::Color, 2, 2, &[0; 15]).is_err());
    }

    #[test]
    fn a_solid_texture_is_one_pixel_of_its_color() {
        let texture = SurfaceTexture::solid(TextureKind::Data, [0.5, 0.5, 1.0, 1.0]);
        assert_eq!((texture.size(), texture.levels.as_slice()), ((1, 1), [vec![128, 128, 255, 255]].as_slice()));
    }

    fn data_row(values: &[u8]) -> Vec<u8> {
        values.iter().flat_map(|&v| [v, v, v, v]).collect()
    }

    #[test]
    fn an_odd_side_keeps_its_last_pixel_in_the_next_level() {
        // 3 x 1 black, black, white: the one pixel level averages all three, one third white.
        let texture = SurfaceTexture::from_rgba8(TextureKind::Data, 3, 1, &data_row(&[0, 0, 255])).unwrap();
        assert_eq!(texture.levels[1], [85, 85, 85, 85]);
        // The same along the other side.
        let column = SurfaceTexture::from_rgba8(TextureKind::Data, 1, 3, &data_row(&[255, 0, 0])).unwrap();
        assert_eq!(column.levels[1], [85, 85, 85, 85]);
        // The white pixel in the first place counts the same.
        let first = SurfaceTexture::from_rgba8(TextureKind::Data, 3, 1, &data_row(&[255, 0, 0])).unwrap();
        assert_eq!(first.levels[1], [85, 85, 85, 85]);
    }

    #[test]
    fn odd_sizes_in_both_sides_weigh_every_pixel_by_the_area_it_covers() {
        // 3 x 3 with one white corner pixel: its area share in the 1 x 1 level is one ninth.
        let mut rgba = data_row(&[0; 9]);
        rgba[32..36].copy_from_slice(&[255; 4]);
        let texture = SurfaceTexture::from_rgba8(TextureKind::Data, 3, 3, &rgba).unwrap();
        assert_eq!(texture.levels[1], [28, 28, 28, 28]);
        // 5 x 3 goes to 2 x 1: the middle column, and the last one, reach into both pixels.
        let mut wide = data_row(&[0; 15]);
        for y in 0..3 {
            wide[(y * 5 + 4) * 4..(y * 5 + 5) * 4].copy_from_slice(&[255; 4]);
        }
        let texture = SurfaceTexture::from_rgba8(TextureKind::Data, 5, 3, &wide).unwrap();
        assert_eq!(texture.levels[1], [0, 0, 0, 0, 102, 102, 102, 102]);
        assert_eq!(texture.levels[2], [51, 51, 51, 51]);
    }
}
