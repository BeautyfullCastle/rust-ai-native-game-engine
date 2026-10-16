//! The 2D renderer: draws a [`RenderList`] into any texture view.

use bytemuck::{Pod, Zeroable};
use orr_rhi::{
    Binding, Blend, BufferDesc, BufferUsage, ColorAttachment, Command, PipelineDesc, Rhi, TextureFormat,
    VertexAttr, VertexFormat, VertexLayout, VertexStep,
};

use crate::camera::Camera;
use crate::list::{LineInstance, RenderList, ShapeInstance};
use crate::stats::{CpuClock, FrameStats};

const SHADER: &str = include_str!("shader2d.wgsl");
const INITIAL_INSTANCES: usize = 1024;
/// The default background color (linear values as the shader writes them).
pub const DEFAULT_CLEAR: [f64; 4] = [0.02, 0.02, 0.035, 1.0];

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    center: [f32; 2],
    scale: [f32; 2],
    viewport: [f32; 2],
    pad: [f32; 2],
}

/// Instance buffer that grows (never shrinks) to fit a frame's data.
pub(crate) struct Growing<B: Rhi> {
    pub(crate) buffer: B::Buffer,
    capacity: usize,
    item_size: usize,
    label: &'static str,
}

impl<B: Rhi> Growing<B> {
    pub(crate) fn new(rhi: &B, label: &'static str, item_size: usize) -> Self {
        Self { buffer: Self::make(rhi, label, INITIAL_INSTANCES * item_size), capacity: INITIAL_INSTANCES, item_size, label }
    }

    fn make(rhi: &B, label: &'static str, bytes: usize) -> B::Buffer {
        rhi.create_buffer(&BufferDesc { label, size: bytes as u64, usage: BufferUsage::VERTEX | BufferUsage::COPY_DST })
    }

    /// Makes room for `count` items (contents are lost when it grows).
    pub(crate) fn reserve(&mut self, rhi: &B, count: usize) -> bool {
        if count > self.capacity {
            self.capacity = count.next_power_of_two();
            self.buffer = Self::make(rhi, self.label, self.capacity * self.item_size);
            return true;
        }
        false
    }

    fn upload(&mut self, rhi: &B, count: usize, bytes: &[u8], stats: &mut FrameStats) {
        stats.buffer_reallocations += u32::from(self.reserve(rhi, count));
        stats.upload(rhi, &self.buffer, 0, bytes);
    }
}

/// Draws render lists. Create one per target format (window surface format,
/// or the offscreen texture's render format).
pub struct Renderer<B: Rhi> {
    rhi: B,
    format: TextureFormat,
    /// Background color used by [`Renderer::draw`] (`None` keeps the target's contents).
    pub clear: Option<[f64; 4]>,
    shape_pipeline: B::Pipeline,
    line_pipeline: B::Pipeline,
    globals: B::Buffer,
    shape_bind: B::BindGroup,
    line_bind: B::BindGroup,
    shapes: Growing<B>,
    lines: Growing<B>,
    last_frame_stats: FrameStats,
}

const SHAPE_ATTRS: [VertexAttr; 5] = [
    VertexAttr { location: 0, format: VertexFormat::Float32x2, offset: 0 },
    VertexAttr { location: 1, format: VertexFormat::Float32x2, offset: 8 },
    VertexAttr { location: 2, format: VertexFormat::Float32, offset: 16 },
    VertexAttr { location: 3, format: VertexFormat::Uint32, offset: 20 },
    VertexAttr { location: 4, format: VertexFormat::Float32x4, offset: 24 },
];

const LINE_ATTRS: [VertexAttr; 4] = [
    VertexAttr { location: 0, format: VertexFormat::Float32x2, offset: 0 },
    VertexAttr { location: 1, format: VertexFormat::Float32x2, offset: 8 },
    VertexAttr { location: 2, format: VertexFormat::Float32, offset: 16 },
    VertexAttr { location: 3, format: VertexFormat::Float32x4, offset: 24 },
];

impl<B: Rhi> Renderer<B> {
    pub fn new(rhi: B, format: TextureFormat) -> Self {
        let shader = rhi.create_shader("orr_render 2d", SHADER);
        let globals = rhi.create_buffer(&BufferDesc {
            label: "globals",
            size: std::mem::size_of::<Globals>() as u64,
            usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST,
        });
        let make_pipeline = |label: &str, vs: &str, fs: &str, layout: VertexLayout| {
            rhi.create_pipeline(&PipelineDesc::<B>::color(label, &shader, (vs, fs), &[layout], format, Blend::Alpha))
        };
        let shape_pipeline = make_pipeline(
            "2d shapes",
            "vs_shape",
            "fs_shape",
            VertexLayout {
                stride: std::mem::size_of::<ShapeInstance>() as u64,
                step: VertexStep::Instance,
                attrs: &SHAPE_ATTRS,
            },
        );
        let line_pipeline = make_pipeline(
            "2d lines",
            "vs_line",
            "fs_line",
            VertexLayout {
                stride: std::mem::size_of::<LineInstance>() as u64,
                step: VertexStep::Instance,
                attrs: &LINE_ATTRS,
            },
        );
        let shape_bind = rhi.create_bind_group(&shape_pipeline, 0, &[Binding::Uniform { binding: 0, buffer: &globals }]);
        let line_bind = rhi.create_bind_group(&line_pipeline, 0, &[Binding::Uniform { binding: 0, buffer: &globals }]);
        let shapes = Growing::new(&rhi, "shape instances", std::mem::size_of::<ShapeInstance>());
        let lines = Growing::new(&rhi, "line instances", std::mem::size_of::<LineInstance>());
        Self {
            rhi,
            format,
            clear: Some(DEFAULT_CLEAR),
            shape_pipeline,
            line_pipeline,
            globals,
            shape_bind,
            line_bind,
            shapes,
            lines,
            last_frame_stats: FrameStats::default(),
        }
    }

    pub fn format(&self) -> TextureFormat {
        self.format
    }

    pub fn rhi(&self) -> &B {
        &self.rhi
    }

    /// Work submitted by the last completed draw, excluding target/presentation work.
    pub fn last_frame_stats(&self) -> FrameStats {
        self.last_frame_stats
    }

    /// Draws `list` into `view` (`size` pixels) seen through `camera`, and
    /// submits it. Shapes first (later on top), then lines.
    pub fn draw(&mut self, view: &B::TextureView, size: (u32, u32), list: &RenderList, camera: &Camera) {
        let prepare_clock = CpuClock::start();
        let mut stats = FrameStats {
            shape_instances: list.shapes.len() as u64,
            line_instances: list.lines.len() as u64,
            msaa_samples: 1,
            ..FrameStats::default()
        };
        let rhi = &self.rhi;
        let scale = camera.scale(size.0, size.1);
        let globals = Globals {
            center: camera.center,
            scale,
            viewport: [size.0.max(1) as f32, size.1.max(1) as f32],
            pad: [0.0; 2],
        };
        stats.upload(rhi, &self.globals, 0, bytemuck::bytes_of(&globals));
        if !list.shapes.is_empty() {
            self.shapes.upload(rhi, list.shapes.len(), bytemuck::cast_slice(&list.shapes), &mut stats);
        }
        if !list.lines.is_empty() {
            self.lines.upload(rhi, list.lines.len(), bytemuck::cast_slice(&list.lines), &mut stats);
        }

        stats.cpu_prepare_time = prepare_clock.elapsed();
        let encode_clock = CpuClock::start();
        let mut commands: Vec<Command<B>> = Vec::with_capacity(8);
        if !list.shapes.is_empty() {
            commands.push(Command::SetPipeline(&self.shape_pipeline));
            commands.push(Command::SetBindGroup(0, &self.shape_bind));
            commands.push(Command::SetVertexBuffer(0, &self.shapes.buffer));
            commands.push(Command::Draw { vertices: 0..6, instances: 0..list.shapes.len() as u32 });
            stats.main.draw(list.shapes.len() as u32);
        }
        if !list.lines.is_empty() {
            commands.push(Command::SetPipeline(&self.line_pipeline));
            commands.push(Command::SetBindGroup(0, &self.line_bind));
            commands.push(Command::SetVertexBuffer(0, &self.lines.buffer));
            commands.push(Command::Draw { vertices: 0..6, instances: 0..list.lines.len() as u32 });
            stats.main.draw(list.lines.len() as u32);
        }
        let mut encoder = rhi.create_encoder("2d frame");
        rhi.encode_render_pass(&mut encoder, "2d", &ColorAttachment { view, clear: self.clear, resolve: None }, &commands);
        stats.main.passes += 1;
        stats.cpu_encode_time = encode_clock.elapsed();
        let submit_clock = CpuClock::start();
        rhi.submit(encoder);
        stats.cpu_submit_time = submit_clock.elapsed();
        self.last_frame_stats = stats;
    }
}
