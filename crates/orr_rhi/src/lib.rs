//! `orr_rhi`: a thin render hardware interface over wgpu (design doc 4.2).
//!
//! The [`Rhi`] trait is the seam for other backends. It is deliberately small:
//! it creates the handful of objects a 2D/3D forward renderer needs (buffers,
//! textures, shaders, pipelines, bind groups, surfaces), records render passes
//! from a backend neutral command list, submits, presents and reads back a
//! texture. Backend specific start-up (choosing an adapter, creating the
//! instance) is not in the trait: it is an inherent constructor of the
//! backend, see [`wgpu_backend::Wgpu`].
//!
//! Descriptors ([`BufferDesc`], [`PipelineDesc`], ...) are owned by this crate
//! so that callers never name a wgpu type. Shaders are WGSL text (design doc
//! 4.3). This is view layer code: it may use floats and the wall clock.
#![allow(clippy::float_arithmetic)]
#![allow(clippy::disallowed_types)]

pub mod wgpu_backend;

use std::ops::Range;

#[cfg(target_arch = "wasm32")]
pub use wgpu_backend::WebBackend;
pub use wgpu_backend::{Wgpu, WgpuOptions};

/// What a window must offer so a surface can be made from it.
pub trait WindowHandle:
    raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle + Send + Sync + 'static
{
}
impl<T: raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle + Send + Sync + 'static> WindowHandle
    for T
{
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextureFormat {
    Rgba8Unorm,
    Rgba8UnormSrgb,
    Bgra8Unorm,
    Bgra8UnormSrgb,
    /// Linear HDR color: four IEEE 754 binary16 (half-float) channels.
    Rgba16Float,
    /// 32 bit float depth (depth buffers and shadow maps; not readable by [`Rhi::read_texture`]).
    Depth32Float,
}

impl TextureFormat {
    /// True for depth formats.
    pub const fn is_depth(self) -> bool {
        matches!(self, TextureFormat::Depth32Float)
    }

    pub const fn is_srgb(self) -> bool {
        matches!(self, TextureFormat::Rgba8UnormSrgb | TextureFormat::Bgra8UnormSrgb)
    }

    /// The same channels with the other (sRGB or linear) encoding.
    pub const fn with_srgb(self, srgb: bool) -> TextureFormat {
        match (self, srgb) {
            (TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb, true) => TextureFormat::Rgba8UnormSrgb,
            (TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb, false) => TextureFormat::Rgba8Unorm,
            (TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb, true) => TextureFormat::Bgra8UnormSrgb,
            (TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb, false) => TextureFormat::Bgra8Unorm,
            (TextureFormat::Rgba16Float | TextureFormat::Depth32Float, _) => self,
        }
    }

    /// True when byte 0 of a texel is blue (needs a swap to read as RGBA).
    pub const fn is_bgra(self) -> bool {
        matches!(self, TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb)
    }

    /// Bytes per texel returned by [`Rhi::read_texture`], or `None` for depth.
    /// RGBA16F channels are little-endian binary16, not normalized bytes.
    pub const fn readback_bytes_per_texel(self) -> Option<u32> {
        match self {
            Self::Rgba8Unorm | Self::Rgba8UnormSrgb | Self::Bgra8Unorm | Self::Bgra8UnormSrgb => Some(4),
            Self::Rgba16Float => Some(8),
            Self::Depth32Float => None,
        }
    }
}

/// Decodes one [`TextureFormat::Rgba16Float`] readback texel in RGBA order.
/// Channels are little-endian IEEE 754 binary16. Values above one, negative
/// values, signed zeros, infinities and NaNs are preserved (no tone mapping).
pub fn decode_rgba16f_texel(bytes: [u8; 8]) -> [f32; 4] {
    std::array::from_fn(|channel| {
        let offset = channel * 2;
        binary16_to_f32(u16::from_le_bytes([bytes[offset], bytes[offset + 1]]))
    })
}

fn binary16_to_f32(bits: u16) -> f32 {
    let sign = u32::from(bits & 0x8000) << 16;
    let exponent = u32::from((bits >> 10) & 0x1f);
    let mut fraction = u32::from(bits & 0x03ff);
    let value = match exponent {
        0 if fraction == 0 => sign,
        0 => {
            let mut exponent = 113;
            while fraction & 0x0400 == 0 {
                fraction <<= 1;
                exponent -= 1;
            }
            sign | (exponent << 23) | ((fraction & 0x03ff) << 13)
        }
        31 => sign | 0x7f80_0000 | (fraction << 13),
        _ => sign | ((exponent + 112) << 23) | (fraction << 13),
    };
    f32::from_bits(value)
}

/// Bit set of buffer uses.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct BufferUsage(pub u32);

impl BufferUsage {
    pub const VERTEX: Self = Self(1);
    pub const UNIFORM: Self = Self(2);
    pub const COPY_SRC: Self = Self(4);
    pub const COPY_DST: Self = Self(8);
    pub const INDEX: Self = Self(16);
    pub const fn or(self, o: Self) -> Self {
        Self(self.0 | o.0)
    }
    pub const fn contains(self, o: Self) -> bool {
        self.0 & o.0 == o.0
    }
}

impl std::ops::BitOr for BufferUsage {
    type Output = Self;
    fn bitor(self, o: Self) -> Self {
        self.or(o)
    }
}

/// Bit set of texture uses.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct TextureUsage(pub u32);

impl TextureUsage {
    pub const RENDER_ATTACHMENT: Self = Self(1);
    /// Can be sampled by a shader (also by egui).
    pub const TEXTURE_BINDING: Self = Self(2);
    pub const COPY_SRC: Self = Self(4);
    pub const COPY_DST: Self = Self(8);
    pub const fn or(self, o: Self) -> Self {
        Self(self.0 | o.0)
    }
    pub const fn contains(self, o: Self) -> bool {
        self.0 & o.0 == o.0
    }
}

impl std::ops::BitOr for TextureUsage {
    type Output = Self;
    fn bitor(self, o: Self) -> Self {
        self.or(o)
    }
}

/// Device-usable capabilities for a single-sample, single-layer 2D texture.
///
/// These describe the enabled device as well as the adapter. They do not
/// enable optional features or permit alternate texture-view formats. Check
/// dimensions and all required usages before creating optional HDR resources.
/// Multiple attachments/bindings still need to obey their aggregate limits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextureFormatCapabilities {
    pub usages: TextureUsage,
    pub filterable: bool,
    pub blendable: bool,
    pub max_dimension_2d: u32,
    /// Enabled staging-buffer limit. Readback needs
    /// `align_up(width * bytes_per_texel, 256) * height` bytes.
    pub max_readback_buffer_size: u64,
    /// Whether this format can support the bounded HDR postprocess path:
    /// two textures, one filtering sampler and a 32-byte uniform, four
    /// bindings in one group, with a second group available for scene use.
    /// This does not guarantee arbitrary geometry/skin-palette requirements.
    pub supports_hdr_postprocessing: bool,
}

#[derive(Clone, Debug)]
pub struct BufferDesc<'a> {
    pub label: &'a str,
    pub size: u64,
    pub usage: BufferUsage,
}

#[derive(Clone, Debug)]
pub struct TextureDesc<'a> {
    pub label: &'a str,
    pub width: u32,
    pub height: u32,
    pub format: TextureFormat,
    pub usage: TextureUsage,
    /// Samples per texel: 1, or a multisample count the adapter supports
    /// (see [`Rhi::supported_samples`]). A multisampled texture is a render
    /// attachment that is resolved into a single sample one; it cannot be
    /// sampled or read back.
    pub sample_count: u32,
    /// Other formats a view of this texture may use (for example the
    /// non-sRGB twin of an sRGB texture). Nonempty lists require
    /// [`Rhi::texture_view_formats_supported`].
    pub view_formats: &'a [TextureFormat],
}

/// A mip-zero RGBA8 upload, with rows ordered top to bottom.
///
/// `bytes_per_row` must be a multiple of four and at least `width * 4`.
/// `data` must contain exactly `(height - 1) * bytes_per_row + width * 4`
/// bytes: padding is allowed between rows, but not after the last row.
/// No 256-byte row alignment is required. Bytes are stored unchanged;
/// an sRGB destination determines how shaders interpret them.
#[derive(Clone, Copy, Debug)]
pub struct TextureUpload<'a> {
    pub origin: [u32; 2],
    pub width: u32,
    pub height: u32,
    pub bytes_per_row: u32,
    pub data: &'a [u8],
}

/// CPU-side upload rejection. Invalid requests never enqueue a GPU write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureUploadError {
    UnsupportedBackend,
    UnsupportedTexture,
    UnsupportedFormat,
    Multisampled,
    MissingCopyDst,
    EmptyExtent,
    OutOfBounds,
    InvalidRowStride,
    SizeOverflow,
    InvalidDataLength { expected: u64, actual: usize },
}

impl std::fmt::Display for TextureUploadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedBackend => f.write_str("backend does not support RGBA8 uploads"),
            Self::UnsupportedTexture => f.write_str("upload requires a single-layer 2D texture"),
            Self::UnsupportedFormat => f.write_str("upload requires an RGBA8 unorm or sRGB texture"),
            Self::Multisampled => f.write_str("upload requires a single-sample texture"),
            Self::MissingCopyDst => f.write_str("upload requires COPY_DST texture usage"),
            Self::EmptyExtent => f.write_str("upload and texture dimensions must be nonzero"),
            Self::OutOfBounds => f.write_str("upload region is outside the texture"),
            Self::InvalidRowStride => f.write_str("upload row stride must cover the row and be divisible by four"),
            Self::SizeOverflow => f.write_str("upload layout size overflows"),
            Self::InvalidDataLength { expected, actual } => {
                write!(f, "upload needs {expected} bytes, got {actual}")
            }
        }
    }
}

impl std::error::Error for TextureUploadError {}

fn validate_texture_upload(desc: &TextureDesc<'_>, upload: &TextureUpload<'_>) -> Result<(), TextureUploadError> {
    use TextureUploadError as E;
    if !matches!(desc.format, TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb) {
        return Err(E::UnsupportedFormat);
    }
    if desc.sample_count != 1 {
        return Err(E::Multisampled);
    }
    if !desc.usage.contains(TextureUsage::COPY_DST) {
        return Err(E::MissingCopyDst);
    }
    if desc.width == 0 || desc.height == 0 || upload.width == 0 || upload.height == 0 {
        return Err(E::EmptyExtent);
    }
    if upload.origin[0].checked_add(upload.width).is_none_or(|end| end > desc.width)
        || upload.origin[1].checked_add(upload.height).is_none_or(|end| end > desc.height)
    {
        return Err(E::OutOfBounds);
    }
    let row = upload.width.checked_mul(4).ok_or(E::SizeOverflow)?;
    if upload.bytes_per_row < row || upload.bytes_per_row % 4 != 0 {
        return Err(E::InvalidRowStride);
    }
    let expected = u64::from(upload.height - 1) * u64::from(upload.bytes_per_row) + u64::from(row);
    if u64::try_from(upload.data.len()).ok() != Some(expected) {
        return Err(E::InvalidDataLength { expected, actual: upload.data.len() });
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VertexFormat {
    Float32,
    Float32x2,
    Float32x3,
    Float32x4,
    Uint32,
}

impl VertexFormat {
    pub const fn size(self) -> u64 {
        match self {
            VertexFormat::Float32 | VertexFormat::Uint32 => 4,
            VertexFormat::Float32x2 => 8,
            VertexFormat::Float32x3 => 12,
            VertexFormat::Float32x4 => 16,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VertexAttr {
    pub location: u32,
    pub format: VertexFormat,
    pub offset: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VertexStep {
    Vertex,
    Instance,
}

#[derive(Clone, Copy, Debug)]
pub struct VertexLayout<'a> {
    pub stride: u64,
    pub step: VertexStep,
    pub attrs: &'a [VertexAttr],
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Blend {
    Opaque,
    /// Straight alpha over.
    Alpha,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Topology {
    #[default]
    TriangleList,
    LineList,
}

/// Which triangle faces are not drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Cull {
    #[default]
    None,
    /// Counter clockwise triangles are front faces; back faces are dropped.
    Back,
    /// Front faces are dropped (shadow passes).
    Front,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Compare {
    Less,
    LessEqual,
    Always,
}

/// Depth test and write of a pipeline.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DepthState {
    pub format: TextureFormat,
    pub write: bool,
    pub compare: Compare,
    /// Constant and slope scaled depth bias (shadow map passes).
    pub bias: i32,
    pub slope_bias: f32,
}

impl DepthState {
    /// Depth test `Less`, writing.
    pub const fn opaque(format: TextureFormat) -> Self {
        Self { format, write: true, compare: Compare::Less, bias: 0, slope_bias: 0.0 }
    }
}

pub struct PipelineDesc<'a, B: Rhi> {
    pub label: &'a str,
    pub shader: &'a B::Shader,
    pub vs_entry: &'a str,
    pub fs_entry: &'a str,
    pub vertex_buffers: &'a [VertexLayout<'a>],
    /// `None` for a depth only pipeline (no fragment stage).
    pub color_format: Option<TextureFormat>,
    pub blend: Blend,
    pub topology: Topology,
    pub cull: Cull,
    pub depth: Option<DepthState>,
    /// Sample count of the attachments the pipeline draws to.
    pub samples: u32,
}

impl<'a, B: Rhi> PipelineDesc<'a, B> {
    /// A 2D style pipeline: one color target, no depth, no culling, one sample.
    pub fn color(
        label: &'a str,
        shader: &'a B::Shader,
        (vs_entry, fs_entry): (&'a str, &'a str),
        vertex_buffers: &'a [VertexLayout<'a>],
        color_format: TextureFormat,
        blend: Blend,
    ) -> Self {
        Self {
            label,
            shader,
            vs_entry,
            fs_entry,
            vertex_buffers,
            color_format: Some(color_format),
            blend,
            topology: Topology::TriangleList,
            cull: Cull::None,
            depth: None,
            samples: 1,
        }
    }
}

/// Sampler settings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SamplerDesc {
    /// Linear (true) or nearest filtering.
    pub linear: bool,
    /// A comparison (shadow) sampler: `textureSampleCompare` passes where the
    /// reference is `<=` the stored depth. Clamps to the edge.
    pub compare: bool,
}

/// One binding of a bind group.
pub enum Binding<'a, B: Rhi> {
    Uniform { binding: u32, buffer: &'a B::Buffer },
    Texture { binding: u32, view: &'a B::TextureView },
    Sampler { binding: u32, sampler: &'a B::Sampler },
}

pub struct ColorAttachment<'a, B: Rhi> {
    pub view: &'a B::TextureView,
    /// `Some(color)` clears first, `None` keeps the old contents.
    pub clear: Option<[f64; 4]>,
    /// For a multisampled `view`: the single sample view the result is
    /// resolved into (the multisampled contents are then discarded).
    pub resolve: Option<&'a B::TextureView>,
}

pub struct DepthAttachment<'a, B: Rhi> {
    pub view: &'a B::TextureView,
    /// `Some(depth)` clears first, `None` keeps the old contents.
    pub clear: Option<f32>,
    /// Keep the depth after the pass (a shadow map that is sampled later).
    pub store: bool,
}

/// A backend neutral render command (recorded, then replayed by the backend).
pub enum Command<'a, B: Rhi> {
    SetPipeline(&'a B::Pipeline),
    SetBindGroup(u32, &'a B::BindGroup),
    SetVertexBuffer(u32, &'a B::Buffer),
    /// A buffer of `u32` indices.
    SetIndexBuffer(&'a B::Buffer),
    Draw {
        vertices: Range<u32>,
        instances: Range<u32>,
    },
    DrawIndexed {
        indices: Range<u32>,
        base_vertex: i32,
        instances: Range<u32>,
    },
}

/// What a surface frame acquire found.
pub enum Acquire<F> {
    Frame(F),
    /// Skip this frame (window hidden, being resized, ...).
    Skip,
}

/// The backend seam. Handles are cheap clones (`Arc`-like) of the device.
pub trait Rhi: Clone + 'static {
    type Buffer;
    type Texture;
    type TextureView;
    type Shader;
    type Pipeline;
    type BindGroup;
    type Sampler;
    type Encoder;
    type Surface;
    type Frame;

    fn adapter_name(&self) -> String;

    fn create_buffer(&self, desc: &BufferDesc) -> Self::Buffer;
    fn write_buffer(&self, buffer: &Self::Buffer, offset: u64, data: &[u8]);

    /// Whether a texture may declare alternate (sRGB/UNORM twin) view formats.
    /// This is a downlevel capability, not a requestable device feature.
    fn texture_view_formats_supported(&self) -> bool;

    fn create_texture(&self, desc: &TextureDesc) -> Self::Texture;

    /// Enqueues a GPU-only, byte-preserving copy of a whole single-sample color
    /// texture. Both textures must have equal dimensions and copy-compatible
    /// formats (identical, or differing only in sRGB encoding), with COPY_SRC
    /// and COPY_DST usage respectively. No color conversion or CPU wait occurs.
    fn copy_texture(&self, source: &Self::Texture, destination: &Self::Texture);
    /// Enqueues a checked upload into mip zero of a single-layer, single-sample
    /// RGBA8 texture created by this backend with `COPY_DST` usage.
    /// Validation errors leave the texture unchanged. A successful write is
    /// ordered before subsequent submissions (including readback); it does not
    /// wait for GPU completion. As with other RHI handles, the caller must use
    /// a texture owned by this device; cross-device handles are not validated.
    /// Other backends may return `UnsupportedBackend`.
    fn write_texture_rgba8(
        &self,
        _texture: &Self::Texture,
        _upload: &TextureUpload<'_>,
    ) -> Result<(), TextureUploadError> {
        Err(TextureUploadError::UnsupportedBackend)
    }

    /// A view of the whole texture. `format: None` uses the texture's format;
    /// `Some(f)` must be the format or one of its `view_formats`.
    fn create_texture_view(&self, texture: &Self::Texture, format: Option<TextureFormat>) -> Self::TextureView;

    fn create_sampler(&self, desc: &SamplerDesc) -> Self::Sampler;

    /// Capabilities available without changing the enabled device features.
    /// The default is deliberately unsupported so other backends fail closed.
    fn texture_format_capabilities(&self, _format: TextureFormat) -> TextureFormatCapabilities {
        TextureFormatCapabilities::default()
    }

    /// True when `format` can be rendered with `samples` samples per texel.
    fn sample_count_supported(&self, format: TextureFormat, samples: u32) -> bool;

    /// The largest supported sample count not above `requested` (at least 1).
    fn supported_samples(&self, format: TextureFormat, requested: u32) -> u32 {
        [8, 4, 2].into_iter().find(|&n| n <= requested && self.sample_count_supported(format, n)).unwrap_or(1)
    }

    /// Shader module from WGSL source.
    fn create_shader(&self, label: &str, wgsl: &str) -> Self::Shader;
    fn create_pipeline(&self, desc: &PipelineDesc<Self>) -> Self::Pipeline;
    /// A bind group for group `group` of `pipeline` (its layout comes from the shader).
    fn create_bind_group(&self, pipeline: &Self::Pipeline, group: u32, bindings: &[Binding<Self>]) -> Self::BindGroup;

    fn create_encoder(&self, label: &str) -> Self::Encoder;
    /// Records one render pass with `commands` into `encoder`. At least one
    /// of `color` and `depth` must be given.
    fn encode_pass(
        &self,
        encoder: &mut Self::Encoder,
        label: &str,
        color: Option<&ColorAttachment<Self>>,
        depth: Option<&DepthAttachment<Self>>,
        commands: &[Command<Self>],
    );
    /// A color only pass ([`Rhi::encode_pass`] without depth).
    fn encode_render_pass(
        &self,
        encoder: &mut Self::Encoder,
        label: &str,
        color: &ColorAttachment<Self>,
        commands: &[Command<Self>],
    ) {
        self.encode_pass(encoder, label, Some(color), None, commands);
    }
    fn submit(&self, encoder: Self::Encoder);
    /// Blocks until all submitted work has finished on the GPU.
    fn wait_idle(&self);

    /// Blocking readback of a whole texture: tightly packed rows, top row
    /// first, texel bytes as stored (a BGRA texture reads as BGRA).
    /// RGBA16F returns eight bytes per texel, little-endian binary16 channels;
    /// use [`decode_rgba16f_texel`] to interpret them without clipping HDR values.
    /// The texture must be single-sample, single-layer 2D color with
    /// `TextureUsage::COPY_SRC`. Depth readback is unsupported.
    fn read_texture(&self, texture: &Self::Texture) -> Vec<u8>;

    fn surface_format(&self, surface: &Self::Surface) -> TextureFormat;
    fn resize_surface(&self, surface: &mut Self::Surface, width: u32, height: u32);
    fn acquire_frame(&self, surface: &mut Self::Surface) -> Acquire<Self::Frame>;
    /// Reads back the pixels of an acquired frame (call after the frame was
    /// drawn and submitted, before [`Rhi::present`]): tightly packed rows,
    /// top row first, texel bytes as stored (see [`Rhi::surface_format`]).
    /// `None` when the surface cannot be copied from.
    fn read_frame(&self, frame: &Self::Frame) -> Option<Vec<u8>>;
    fn frame_view<'a>(&self, frame: &'a Self::Frame) -> &'a Self::TextureView;
    fn present(&self, frame: Self::Frame);
}

#[cfg(test)]
mod format_tests {
    use super::*;

    #[test]
    fn hdr_encoding_and_readback_size_are_explicit() {
        assert_eq!(TextureFormat::Rgba16Float.with_srgb(true), TextureFormat::Rgba16Float);
        assert_eq!(TextureFormat::Rgba16Float.with_srgb(false), TextureFormat::Rgba16Float);
        assert!(!TextureFormat::Rgba16Float.is_srgb());
        assert!(!TextureFormat::Rgba16Float.is_bgra());
        assert!(!TextureFormat::Rgba16Float.is_depth());
        assert_eq!(TextureFormat::Rgba16Float.readback_bytes_per_texel(), Some(8));
        for format in [
            TextureFormat::Rgba8Unorm,
            TextureFormat::Rgba8UnormSrgb,
            TextureFormat::Bgra8Unorm,
            TextureFormat::Bgra8UnormSrgb,
        ] {
            assert_eq!(format.readback_bytes_per_texel(), Some(4));
            assert_eq!(format.with_srgb(true).with_srgb(false), format.with_srgb(false));
        }
        assert_eq!(TextureFormat::Depth32Float.readback_bytes_per_texel(), None);
    }

    #[test]
    fn rgba16f_readback_is_little_endian_and_not_normalized() {
        assert_eq!(decode_rgba16f_texel([0x00, 0x40, 0x00, 0xbc, 0x00, 0x38, 0x00, 0x3c]), [2.0, -1.0, 0.5, 1.0]);
        let specials = decode_rgba16f_texel([0x00, 0x80, 0x00, 0x7c, 0x00, 0xfc, 0x01, 0x7e]);
        assert_eq!(specials[0].to_bits(), (-0.0_f32).to_bits());
        assert_eq!(specials[1], f32::INFINITY);
        assert_eq!(specials[2], f32::NEG_INFINITY);
        assert!(specials[3].is_nan());
    }

    #[test]
    fn every_binary16_encoding_decodes_exactly() {
        // Independent arithmetic reference, including subnormals and both signs.
        for bits in 0..=u16::MAX {
            let exponent = (bits >> 10) & 0x1f;
            let fraction = bits & 0x03ff;
            let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
            let actual = binary16_to_f32(bits);
            if exponent == 31 {
                if fraction == 0 {
                    assert_eq!(actual, sign * f32::INFINITY);
                } else {
                    assert!(actual.is_nan());
                }
            } else {
                let expected = if exponent == 0 {
                    sign * f32::from(fraction) * 2_f32.powi(-24)
                } else {
                    sign * (1.0 + f32::from(fraction) / 1024.0) * 2_f32.powi(i32::from(exponent) - 15)
                };
                assert_eq!(actual.to_bits(), expected.to_bits(), "binary16 bits {bits:#06x}");
            }
        }
    }
}

#[cfg(test)]
mod upload_tests {
    use super::*;

    fn desc() -> TextureDesc<'static> {
        TextureDesc {
            label: "test",
            width: 8,
            height: 8,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsage::COPY_DST,
            sample_count: 1,
            view_formats: &[],
        }
    }

    fn upload(data: &[u8]) -> TextureUpload<'_> {
        TextureUpload { origin: [1, 2], width: 2, height: 2, bytes_per_row: 12, data }
    }

    #[test]
    fn upload_accepts_tight_padded_and_single_rows() {
        let bytes = [0; 20];
        let mut u = upload(&bytes);
        assert_eq!(validate_texture_upload(&desc(), &u), Ok(()));
        u.bytes_per_row = 8;
        u.data = &bytes[..16];
        assert_eq!(validate_texture_upload(&desc(), &u), Ok(()));
        u.height = 1;
        u.bytes_per_row = 256;
        u.data = &bytes[..8];
        assert_eq!(validate_texture_upload(&desc(), &u), Ok(()));
        let mut d = desc();
        d.format = TextureFormat::Rgba8UnormSrgb;
        assert_eq!(validate_texture_upload(&d, &u), Ok(()));
    }

    #[test]
    fn upload_rejects_format_usage_and_samples() {
        let bytes = [0; 20];
        let u = upload(&bytes);
        for format in [
            TextureFormat::Bgra8Unorm,
            TextureFormat::Bgra8UnormSrgb,
            TextureFormat::Rgba16Float,
            TextureFormat::Depth32Float,
        ] {
            let mut d = desc();
            d.format = format;
            assert_eq!(validate_texture_upload(&d, &u), Err(TextureUploadError::UnsupportedFormat));
        }
        let mut d = desc();
        d.usage = TextureUsage::TEXTURE_BINDING;
        assert_eq!(validate_texture_upload(&d, &u), Err(TextureUploadError::MissingCopyDst));
        for samples in [0, 2, 4] {
            d = desc();
            d.sample_count = samples;
            assert_eq!(validate_texture_upload(&d, &u), Err(TextureUploadError::Multisampled));
        }
    }

    #[test]
    fn upload_rejects_zero_outside_and_overflowing_regions() {
        let bytes = [0; 20];
        for (width, height) in [(0, 2), (2, 0)] {
            let mut u = upload(&bytes);
            u.width = width;
            u.height = height;
            assert_eq!(validate_texture_upload(&desc(), &u), Err(TextureUploadError::EmptyExtent));
        }
        for origin in [[7, 2], [1, 7], [u32::MAX, 0], [0, u32::MAX]] {
            let mut u = upload(&bytes);
            u.origin = origin;
            assert_eq!(validate_texture_upload(&desc(), &u), Err(TextureUploadError::OutOfBounds));
        }
        for (width, height) in [(0, 8), (8, 0)] {
            let mut d = desc();
            d.width = width;
            d.height = height;
            assert_eq!(validate_texture_upload(&d, &upload(&bytes)), Err(TextureUploadError::EmptyExtent));
        }
        let mut d = desc();
        d.width = u32::MAX;
        let u = TextureUpload { origin: [0, 0], width: u32::MAX, height: 1, bytes_per_row: u32::MAX, data: &[] };
        assert_eq!(validate_texture_upload(&d, &u), Err(TextureUploadError::SizeOverflow));
    }

    #[test]
    fn upload_rejects_short_misaligned_and_extra_bytes() {
        let bytes = [0; 24];
        for stride in [0, 4, 9, 10, 11] {
            let mut u = upload(&bytes[..20]);
            u.bytes_per_row = stride;
            assert_eq!(validate_texture_upload(&desc(), &u), Err(TextureUploadError::InvalidRowStride));
        }
        for length in [0, 19, 21, 24] {
            assert_eq!(
                validate_texture_upload(&desc(), &upload(&bytes[..length])),
                Err(TextureUploadError::InvalidDataLength { expected: 20, actual: length })
            );
        }
        let mut d = desc();
        d.height = u32::MAX;
        let u = TextureUpload { origin: [0, 0], width: 1, height: u32::MAX, bytes_per_row: u32::MAX - 3, data: &[] };
        assert_eq!(
            validate_texture_upload(&d, &u),
            Err(TextureUploadError::InvalidDataLength {
                expected: u64::from(u32::MAX - 1) * u64::from(u32::MAX - 3) + 4,
                actual: 0,
            })
        );
    }
}
