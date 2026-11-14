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
            // Each pixel averages the 2 x 2 block below it; a side of one pixel averages 1 x 2.
            let (sx, sy) = (if w > 1 { 2 } else { 1 }, if h > 1 { 2 } else { 1 });
            for y in 0..nh {
                for x in 0..nw {
                    let mut sum = [0.0f32; 4];
                    for dy in 0..sy {
                        for dx in 0..sx {
                            let tap = values[((y * sy + dy) * w + x * sx + dx) as usize];
                            for c in 0..4 {
                                sum[c] += tap[c];
                            }
                        }
                    }
                    let n = (sx * sy) as f32;
                    next[(y * nw + x) as usize] = sum.map(|s| s / n);
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
}
