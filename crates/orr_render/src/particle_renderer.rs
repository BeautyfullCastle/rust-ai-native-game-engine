//! Bounded camera-facing procedural discs. Scene integration supplies stored depth.
use bytemuck::{Pod, Zeroable};
use orr_rhi::{Binding, Blend, BufferDesc, BufferUsage, ColorAttachment, Command, Compare,
    DepthAttachment, DepthState, PipelineDesc, Rhi, TextureFormat, TextureUsage,
    VertexAttr, VertexFormat, VertexLayout, VertexStep};
use orr_view::particles::{ParticleBlend, ParticleDraw, MAX_PARTICLES};
use crate::{Camera3D, camera3d::Projection, math3};

const SHADER: &str = include_str!("shader_particle.wgsl");
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals { view_projection: [[f32; 4]; 4], right: [f32; 4], up: [f32; 4] }
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Instance { position_size: [f32; 4], color: [f32; 4] }
struct Run { blend: ParticleBlend, start: u32, end: u32 }

/// Owns immutable validated bytes. No reference to mutable renderer staging.
/// Portable between renderers only when target format/extent constraints match.
pub struct PreparedParticles {
    format: TextureFormat,
    size: (u32, u32),
    globals: Globals,
    instances: Vec<Instance>,
    runs: Vec<Run>,
}
impl PreparedParticles {
    pub fn particle_count(&self) -> usize { self.instances.len() }
    pub fn draw_count(&self) -> usize { self.runs.len() }
}
/// Metadata must describe the supplied views, both owned by the renderer's device.
/// RHI cannot introspect view identity. Color and depth must share size/sample count.
pub struct ParticleTarget<'a, B: Rhi> {
    pub color: &'a B::TextureView,
    pub depth: &'a B::TextureView,
    pub size: (u32, u32),
    pub color_format: TextureFormat,
    pub depth_format: TextureFormat,
    pub sample_count: u32,
    pub depth_size: (u32, u32),
    pub depth_sample_count: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleRenderError { UnsupportedTarget, InvalidViewport, InvalidCamera, InvalidParticle, Capacity, TargetMismatch }
impl std::fmt::Display for ParticleRenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "particle rendering: {self:?}") }
}
impl std::error::Error for ParticleRenderError {}

pub struct ParticleRenderer<B: Rhi> {
    rhi: B,
    format: TextureFormat,
    max_extent: u32,
    globals: B::Buffer,
    instances: B::Buffer,
    alpha: B::Pipeline,
    additive: B::Pipeline,
    alpha_bind: B::BindGroup,
    additive_bind: B::BindGroup,
}
impl<B: Rhi> ParticleRenderer<B> {
    /// Reject unsupported formats/capabilities before any GPU allocation.
    pub fn new(rhi: B, format: TextureFormat) -> Result<Self, ParticleRenderError> {
        let c = rhi.texture_format_capabilities(format);
        let d = rhi.texture_format_capabilities(TextureFormat::Depth32Float);
        if format.is_depth() || !c.blendable || !c.usages.contains(TextureUsage::RENDER_ATTACHMENT)
            || !d.usages.contains(TextureUsage::RENDER_ATTACHMENT)
            || !rhi.sample_count_supported(format, 1) || !rhi.sample_count_supported(TextureFormat::Depth32Float, 1)
            || c.max_dimension_2d == 0 || d.max_dimension_2d == 0 {
            return Err(ParticleRenderError::UnsupportedTarget);
        }
        let shader = rhi.create_shader("particle discs", SHADER);
        let globals = rhi.create_buffer(&BufferDesc { label: "particle globals", size: 96, usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST });
        let instances = rhi.create_buffer(&BufferDesc { label: "particle instances", size: (MAX_PARTICLES * 32) as u64, usage: BufferUsage::VERTEX | BufferUsage::COPY_DST });
        let attrs = [VertexAttr { location: 0, format: VertexFormat::Float32x4, offset: 0 }, VertexAttr { location: 1, format: VertexFormat::Float32x4, offset: 16 }];
        let make = |blend| {
            let layouts = [VertexLayout { stride: 32, step: VertexStep::Instance, attrs: &attrs }];
            let mut desc = PipelineDesc::<B>::color("particle discs", &shader, ("vs_particle", "fs_particle"), &layouts, format, blend);
            desc.depth = Some(DepthState { format: TextureFormat::Depth32Float, write: false, compare: Compare::LessEqual, bias: 0, slope_bias: 0.0 });
            rhi.create_pipeline(&desc)
        };
        let alpha = make(Blend::Alpha); let additive = make(Blend::Additive);
        // Auto-generated pipeline layouts have independent identities.
        let alpha_bind = rhi.create_bind_group(&alpha, 0, &[Binding::Uniform { binding: 0, buffer: &globals }]);
        let additive_bind = rhi.create_bind_group(&additive, 0, &[Binding::Uniform { binding: 0, buffer: &globals }]);
        Ok(Self { rhi, format, max_extent: c.max_dimension_2d.min(d.max_dimension_2d).min(8192), globals, instances, alpha, additive, alpha_bind, additive_bind })
    }
    pub fn format(&self) -> TextureFormat { self.format }
    /// Pure CPU preflight. Count check precedes allocation or iteration. All input
    /// is validated before an immutable packet is returned; no GPU/cache mutation.
    pub fn prepare(&self, camera: &Camera3D, size: (u32, u32), items: &[ParticleDraw]) -> Result<PreparedParticles, ParticleRenderError> {
        if items.len() > MAX_PARTICLES { return Err(ParticleRenderError::Capacity); }
        if size.0 == 0 || size.1 == 0 || size.0 > self.max_extent || size.1 > self.max_extent { return Err(ParticleRenderError::InvalidViewport); }
        let globals = camera_globals(camera, size)?;
        let forward = camera.forward();
        // Source ordinal is the explicit stable tie breaker; unstable sort avoids
        // an additional temporary allocation while preserving exact input ties.
        let mut sorted = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            if item.position.to_array().into_iter().any(|v| !v.is_finite() || v.abs() > 1_200_000.0)
                || !item.size.is_finite() || !(0.0..=1000.0).contains(&item.size)
                || item.color.into_iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(&v)) {
                return Err(ParticleRenderError::InvalidParticle);
            }
            let pos = item.position.to_array(); let depth = math3::dot(math3::sub(pos, camera.eye), forward);
            if !depth.is_finite() { return Err(ParticleRenderError::InvalidParticle); }
            sorted.push((depth, index, Instance { position_size: [pos[0], pos[1], pos[2], item.size], color: item.color }, item.blend));
        }
        sorted.sort_unstable_by(|a,b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut instances = Vec::with_capacity(sorted.len()); let mut runs: Vec<Run> = Vec::with_capacity(sorted.len());
        for (_, _, instance, blend) in sorted {
            let index = instances.len() as u32; instances.push(instance);
            if let Some(run) = runs.last_mut().filter(|r| r.blend == blend) { run.end = index + 1; }
            else { runs.push(Run { blend, start: index, end: index + 1 }); }
        }
        Ok(PreparedParticles { format: self.format, size, globals, instances, runs })
    }
    /// Uploads and encodes only. Caller owns the encoder and submits once after
    /// geometry, particles and optional postprocessing. Both attachments load;
    /// depth is tested but never written. Orders only this particle batch, not
    /// arbitrary transparent geometry already encoded by the caller.
    /// Use this renderer once per pending submission: its uniform/instance buffers
    /// must not be overwritten while a previous encoded draw remains unsubmitted.
    pub fn upload_and_encode(&mut self, encoder: &mut B::Encoder, target: ParticleTarget<'_, B>, prepared: &PreparedParticles) -> Result<(), ParticleRenderError> {
        if target.color_format != self.format || prepared.format != self.format || target.size != prepared.size
            || target.depth_format != TextureFormat::Depth32Float || target.sample_count != 1
            || target.depth_sample_count != 1 || target.depth_size != target.size
            || target.size.0 == 0 || target.size.1 == 0 || target.size.0 > self.max_extent || target.size.1 > self.max_extent {
            return Err(ParticleRenderError::TargetMismatch);
        }
        if prepared.instances.is_empty() { return Ok(()); }
        self.rhi.write_buffer(&self.globals, 0, bytemuck::bytes_of(&prepared.globals));
        self.rhi.write_buffer(&self.instances, 0, bytemuck::cast_slice(&prepared.instances));
        let mut commands = Vec::with_capacity(1 + prepared.runs.len() * 3);
        commands.push(Command::SetVertexBuffer(0, &self.instances));
        for run in &prepared.runs {
            let (pipeline, bind) = match run.blend { ParticleBlend::Alpha => (&self.alpha, &self.alpha_bind), ParticleBlend::Additive => (&self.additive, &self.additive_bind) };
            commands.push(Command::SetPipeline(pipeline)); commands.push(Command::SetBindGroup(0, bind));
            commands.push(Command::Draw { vertices: 0..6, instances: run.start..run.end });
        }
        self.rhi.encode_pass(encoder, "particles", Some(&ColorAttachment { view: target.color, clear: None, resolve: None }),
            Some(&DepthAttachment { view: target.depth, clear: None, store: true }), &commands);
        Ok(())
    }
}
fn camera_globals(camera: &Camera3D, size: (u32, u32)) -> Result<Globals, ParticleRenderError> {
    if camera.eye.into_iter().chain(camera.target).chain(camera.up).any(|v| !v.is_finite() || v.abs() > 1_000_000.0) {
        return Err(ParticleRenderError::InvalidCamera);
    }
    let direction = math3::sub(camera.target, camera.eye);
    if math3::length(direction) < 0.0001 || math3::length(math3::cross(camera.forward(), camera.up)) < 0.0001 { return Err(ParticleRenderError::InvalidCamera); }
    let valid = match camera.projection {
        Projection::Perspective { fov_y, near, far } => fov_y.is_finite() && (0.01..=3.13).contains(&fov_y) && near >= 0.001 && planes(near, far),
        Projection::Orthographic { half_height, near, far } => half_height.is_finite() && (0.001..=1_000_000.0).contains(&half_height) && planes(near, far),
    };
    if !valid { return Err(ParticleRenderError::InvalidCamera); }
    let matrix = camera.view_proj(size.0 as f32 / size.1 as f32).0;
    let r = camera.right(); let u = camera.camera_up();
    if matrix.into_iter().flatten().chain(r).chain(u).any(|v| !v.is_finite()) { return Err(ParticleRenderError::InvalidCamera); }
    Ok(Globals { view_projection: matrix, right: [r[0], r[1], r[2], 0.0], up: [u[0], u[1], u[2], 0.0] })
}
fn planes(near: f32, far: f32) -> bool { near.is_finite() && far.is_finite() && near.abs() <= 1_000_000.0 && far.abs() <= 1_000_000.0 && far - near >= 0.001 }
#[cfg(test)]
#[path = "particle_renderer_tests.rs"]
mod tests;
