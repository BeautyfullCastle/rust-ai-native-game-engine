//! Motion vectors of the imported scene: where each drawn pixel was on the
//! screen in the last frame, so temporal anti-aliasing and motion blur follow
//! moving objects and not only the camera.
//!
//! When an effect that reads them is on, the coordinator draws one more pass
//! after the opaque geometry. Every instanced batch draws its geometry again
//! into the velocity target, depth tested against the scene's depth and with
//! the previous placement of each instance next to the current one. A pixel
//! holds the move in texture coordinates in `rg` and 1 in `a`. Pixels no batch
//! drew (the sky, added passes, particles) keep `a = 0`, and the effects fall
//! back to the depth and camera reprojection there.
//!
//! A batch matches an instance with its place in the last frame by index. It
//! trusts that place only when the batch was drawn in the frame just before
//! and has the same number of instances. Otherwise every instance of the batch
//! counts as new for one frame: its previous placement is the current one, so
//! only the camera's move shows.

use bytemuck::{Pod, Zeroable};
use orr_rhi::{
    Blend, BufferDesc, BufferUsage, Compare, Cull, DepthState, PipelineDesc, Rhi, TextureDesc, TextureFormat, TextureUsage, VertexAttr, VertexFormat, VertexLayout,
    VertexStep,
};

use crate::post_effects::ScreenEffect;

/// Format of the velocity target. `rg` is the move, `a` is 1 where it is known.
/// Two channels would do for the move, but the flag needs a third, and this
/// is the float color format the post effects already use on every backend.
pub(crate) const FORMAT: TextureFormat = TextureFormat::Rgba16Float;

const SHADER: &str = include_str!("shader_velocity.wgsl");

/// Whether `effects` read motion vectors.
pub(crate) fn wanted(effects: &[Option<ScreenEffect>]) -> bool {
    effects.iter().flatten().any(|e| matches!(e, ScreenEffect::Taa { .. } | ScreenEffect::MotionBlur { .. }))
}

/// The matrices every velocity draw of a frame shares.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct Camera {
    /// World to clip space as the scene was drawn: jittered when anti-aliasing jitters.
    pub raster: [[f32; 4]; 4],
    /// World to clip space without the jitter.
    pub clean: [[f32; 4]; 4],
    /// The same of the last frame.
    pub previous: [[f32; 4]; 4],
}

/// What the coordinator tells every batch about the frame.
#[derive(Clone, Copy)]
pub(crate) struct Frame {
    pub camera: Camera,
    /// The number of this frame; it grows by one for every frame the coordinator draws.
    pub number: u64,
}

/// What a batch drew in the last frame it wrote velocity for.
pub(crate) struct Trail<T> {
    frame: u64,
    items: Vec<T>,
}

impl<T> Default for Trail<T> {
    fn default() -> Self {
        Self { frame: 0, items: Vec::new() }
    }
}

impl<T: Copy> Trail<T> {
    /// Takes this frame's `items` and returns the previous placements of the same
    /// instances: the last frame's when the batch was written in the frame
    /// before and has as many instances, else the current ones.
    pub(crate) fn advance(&mut self, frame: u64, items: &[T]) -> Vec<T> {
        let previous = if self.frame + 1 == frame && self.items.len() == items.len() { self.items.clone() } else { items.to_vec() };
        self.frame = frame;
        self.items.clear();
        self.items.extend_from_slice(items);
        previous
    }
}

/// The target the velocity pass draws into, at the scene's size.
pub(crate) struct Target<B: Rhi> {
    _texture: B::Texture,
    pub view: B::TextureView,
    pub size: (u32, u32),
}

impl<B: Rhi> Target<B> {
    pub(crate) fn new(rhi: &B, size: (u32, u32)) -> Self {
        let texture = rhi.create_texture(&TextureDesc {
            label: "scene motion vectors",
            width: size.0,
            height: size.1,
            format: FORMAT,
            usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::TEXTURE_BINDING,
            sample_count: 1,
            view_formats: &[],
        });
        let view = rhi.create_texture_view(&texture, None);
        Self { _texture: texture, view, size }
    }
}

/// The depth state of every velocity draw. The scene's depth is read, not written.
/// The small pull toward the eye keeps a surface from failing against its own
/// depth when the two vertex shaders round differently.
const DEPTH: DepthState = DepthState { format: TextureFormat::Depth32Float, write: false, compare: Compare::LessEqual, bias: -2, slope_bias: -1.0 };

/// A velocity pipeline and the buffer of the frame's camera matrices.
pub(crate) struct Gpu<B: Rhi> {
    pub pipeline: B::Pipeline,
    pub camera: B::Buffer,
}

impl<B: Rhi> Gpu<B> {
    /// A pipeline of the vertex entry `vs` over `buffers`.
    pub(crate) fn new(rhi: &B, label: &str, vs: &str, buffers: &[VertexLayout<'_>], cull: Cull) -> Self {
        let shader = rhi.create_shader("scene motion vectors", SHADER);
        let pipeline = rhi.create_pipeline(&PipelineDesc::<B> {
            label,
            shader: &shader,
            vs_entry: vs,
            fs_entry: "fs_velocity",
            vertex_buffers: buffers,
            color_format: Some(FORMAT),
            blend: Blend::Opaque,
            topology: orr_rhi::Topology::TriangleList,
            cull,
            depth: Some(DEPTH),
            samples: 1,
        });
        let camera = rhi.create_buffer(&BufferDesc { label: "motion vector camera", size: std::mem::size_of::<Camera>() as u64, usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST });
        Self { pipeline, camera }
    }

    pub(crate) fn write_camera(&self, rhi: &B, frame: &Frame) {
        rhi.write_buffer(&self.camera, 0, bytemuck::bytes_of(&frame.camera));
    }
}

/// The position of a vertex, location 0, in a vertex of `stride` bytes.
pub(crate) fn position_layout(stride: u64) -> VertexLayout<'static> {
    const POSITION: [VertexAttr; 1] = [VertexAttr { location: 0, format: VertexFormat::Float32x3, offset: 0 }];
    VertexLayout { stride, step: VertexStep::Vertex, attrs: &POSITION }
}
