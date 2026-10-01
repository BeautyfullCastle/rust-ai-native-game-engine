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
            (TextureFormat::Depth32Float, _) => TextureFormat::Depth32Float,
        }
    }

    /// True when byte 0 of a texel is blue (needs a swap to read as RGBA).
    pub const fn is_bgra(self) -> bool {
        matches!(self, TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb)
    }
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
    /// non-sRGB twin of an sRGB texture).
    pub view_formats: &'a [TextureFormat],
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
    Draw { vertices: Range<u32>, instances: Range<u32> },
    DrawIndexed { indices: Range<u32>, base_vertex: i32, instances: Range<u32> },
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

    fn create_texture(&self, desc: &TextureDesc) -> Self::Texture;
    /// A view of the whole texture. `format: None` uses the texture's format;
    /// `Some(f)` must be the format or one of its `view_formats`.
    fn create_texture_view(&self, texture: &Self::Texture, format: Option<TextureFormat>) -> Self::TextureView;

    fn create_sampler(&self, desc: &SamplerDesc) -> Self::Sampler;

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
    /// The texture needs `TextureUsage::COPY_SRC`.
    fn read_texture(&self, texture: &Self::Texture) -> Vec<u8>;

    fn surface_format(&self, surface: &Self::Surface) -> TextureFormat;
    fn resize_surface(&self, surface: &mut Self::Surface, width: u32, height: u32);
    fn acquire_frame(&self, surface: &mut Self::Surface) -> Acquire<Self::Frame>;
    fn frame_view<'a>(&self, frame: &'a Self::Frame) -> &'a Self::TextureView;
    fn present(&self, frame: Self::Frame);
}
