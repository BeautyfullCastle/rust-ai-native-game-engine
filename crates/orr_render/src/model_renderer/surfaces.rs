//! Surface shaders on the instances of a static model (see [`crate::surface`]).
//! Each added shader gets two pipelines on this model's vertex layout: one
//! for frames without the shared sun shadow and one that receives it. The
//! shadow itself is still cast with the mesh's built-in shadow pipeline.

use super::{ModelRenderer, VERTEX_ATTRS};
use crate::shared_shadow::SharedShadow;
use crate::surface::{SurfaceId, SurfaceShader, MAX_SURFACE_FLOATS, MODEL_GLUE};
use bytemuck::{Pod, Zeroable};
use orr_rhi::{Binding, Blend, BufferDesc, BufferUsage, Cull, DepthState, PipelineDesc, Rhi, TextureFormat, Topology, VertexLayout, VertexStep};

/// The surface shader of one model instance and its parameter values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelSurface {
    /// A shader added with [`ModelRenderer::add_surface`].
    pub surface: SurfaceId,
    /// Values in the order the shader declares them (see [`SurfaceShader::default_values`]).
    pub params: [f32; MAX_SURFACE_FLOATS],
}

/// The per-draw uniform `surface_values` of surface_model_glue.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Values {
    values: [f32; MAX_SURFACE_FLOATS],
    time: [f32; 4],
}

pub(super) struct SurfaceGpu<B: Rhi> {
    /// Changes when the shader is replaced, so draws rebuild their bindings.
    version: u64,
    pub main: B::Pipeline,
    pub receiver: B::Pipeline,
    receiver_bind: Option<(u64, B::BindGroup)>,
}

impl<B: Rhi> SurfaceGpu<B> {
    /// The shared shadow map for [`Self::receiver`]; bound before every frame with shadows.
    pub fn receiver_bind(&self) -> &B::BindGroup {
        &self.receiver_bind.as_ref().expect("shared shadow bound before encoding").1
    }
}

/// Bindings of one instance draw for one surface shader.
pub(super) struct SurfaceDraw<B: Rhi> {
    surface: SurfaceId,
    version: u64,
    values: B::Buffer,
    pub main: B::BindGroup,
    pub receiver: B::BindGroup,
}

pub(super) struct ModelSurfaces<B: Rhi> {
    /// The model shader the surface pipelines are built on.
    engine: String,
    gpus: Vec<SurfaceGpu<B>>,
    /// Surface of each instance of the next frame, by instance index.
    uses: Vec<Option<ModelSurface>>,
    time: f32,
    next_version: u64,
}

impl<B: Rhi> ModelSurfaces<B> {
    pub fn new(engine: String) -> Self {
        Self { engine, gpus: Vec::new(), uses: Vec::new(), time: 0.0, next_version: 0 }
    }

    pub fn bind_map(&mut self, rhi: &B, map: &SharedShadow<B>) {
        for gpu in &mut self.gpus {
            if gpu.receiver_bind.as_ref().is_none_or(|(id, _)| *id != map.id) {
                let bind = rhi.create_bind_group(&gpu.receiver, 1, &[Binding::Texture { binding: 0, view: &map.view }, Binding::Sampler { binding: 1, sampler: &map.sampler }]);
                gpu.receiver_bind = Some((map.id, bind));
            }
        }
    }
}

fn finite(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

impl<B: Rhi> ModelRenderer<B> {
    /// Adds pipelines for `shader` and returns the id that
    /// [`Self::set_instance_surfaces`] uses to pick it.
    /// [`SurfaceShader::compile`] validated the shader against this
    /// renderer's own shader, so this creates no invalid GPU object.
    pub fn add_surface(&mut self, shader: &SurfaceShader) -> SurfaceId {
        let gpu = self.build_surface(shader);
        self.surfaces.gpus.push(gpu);
        SurfaceId(self.surfaces.gpus.len() as u32 - 1)
    }

    /// Replaces the shader of `id` with `shader`, keeping the id. An id this
    /// renderer does not have is left alone.
    pub fn replace_surface(&mut self, id: SurfaceId, shader: &SurfaceShader) {
        if (id.0 as usize) < self.surfaces.gpus.len() {
            let gpu = self.build_surface(shader);
            self.surfaces.gpus[id.0 as usize] = gpu;
        }
    }

    /// Surface shaders of the instances that the next frames draw in an
    /// imported scene, by instance index. `None`, a missing entry and an id
    /// this renderer does not have keep the built-in look. `time` is the clock
    /// in seconds that the shaders read. Numbers that are not finite are drawn
    /// as zero.
    pub fn set_instance_surfaces(&mut self, surfaces: &[Option<ModelSurface>], time: f32) {
        self.surfaces.uses.clear();
        self.surfaces.uses.extend_from_slice(surfaces);
        self.surfaces.time = time;
    }

    fn build_surface(&mut self, shader: &SurfaceShader) -> SurfaceGpu<B> {
        let source = shader.source_for(&self.surfaces.engine, MODEL_GLUE);
        let module = self.rhi.create_shader("model surface shader", &source);
        let layouts = [VertexLayout { stride: 32, step: VertexStep::Vertex, attrs: &VERTEX_ATTRS }];
        let pipeline = |label, fs_entry| {
            self.rhi.create_pipeline(&PipelineDesc::<B> {
                label,
                shader: &module,
                vs_entry: "vs_model_surface",
                fs_entry,
                vertex_buffers: &layouts,
                color_format: Some(self.format),
                blend: Blend::Opaque,
                topology: Topology::TriangleList,
                cull: Cull::Back,
                depth: Some(DepthState::opaque(TextureFormat::Depth32Float)),
                samples: 1,
            })
        };
        let main = pipeline("model surface", "fs_model_surface");
        let receiver = pipeline("model surface shadow receiver", "fs_model_surface_shadowed");
        self.surfaces.next_version += 1;
        SurfaceGpu { version: self.surfaces.next_version, main, receiver, receiver_bind: None }
    }

    /// The surface the instance of draw `index` asks for, if this renderer has it.
    fn wanted_surface(&self, index: usize) -> Option<ModelSurface> {
        let wanted = (*self.surfaces.uses.get(index / self.draws.len())?)?;
        ((wanted.surface.0 as usize) < self.surfaces.gpus.len()).then_some(wanted)
    }

    /// The pipelines and bindings that draw `index` uses, or `None` for the built-in look.
    pub(super) fn active_surface(&self, index: usize) -> Option<(&SurfaceGpu<B>, &SurfaceDraw<B>)> {
        let wanted = self.wanted_surface(index)?;
        let gpu = &self.surfaces.gpus[wanted.surface.0 as usize];
        let draw = self.instance_draws.get(index)?.surface.as_ref()?;
        (draw.surface == wanted.surface && draw.version == gpu.version).then_some((gpu, draw))
    }

    /// Builds missing bindings and writes the values of the first `count` instance draws.
    pub(super) fn write_surfaces(&mut self, count: usize) {
        let time = finite(self.surfaces.time);
        for index in 0..count {
            let Some(wanted) = self.wanted_surface(index) else { continue };
            let gpu = &self.surfaces.gpus[wanted.surface.0 as usize];
            let draw = &mut self.instance_draws[index];
            if draw.surface.as_ref().is_none_or(|d| d.surface != wanted.surface || d.version != gpu.version) {
                let values = draw.surface.take().map_or_else(
                    || self.rhi.create_buffer(&BufferDesc { label: "model surface values", size: std::mem::size_of::<Values>() as u64, usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST }),
                    |old| old.values,
                );
                let primitive = &self.model.source().primitives[index % self.draws.len()];
                let image = &self.views[self.model.source().materials[primitive.material as usize].image as usize];
                let bind = |pipeline| {
                    self.rhi.create_bind_group(pipeline, 0, &[
                        Binding::Uniform { binding: 0, buffer: &self.globals },
                        Binding::Texture { binding: 1, view: image },
                        Binding::Uniform { binding: 2, buffer: &draw.uniform },
                        Binding::Uniform { binding: 3, buffer: &values },
                    ])
                };
                let main = bind(&gpu.main);
                let receiver = bind(&gpu.receiver);
                draw.surface = Some(SurfaceDraw { surface: wanted.surface, version: gpu.version, values, main, receiver });
            }
            let values = Values { values: wanted.params.map(finite), time: [time, 0.0, 0.0, 0.0] };
            let buffer = &draw.surface.as_ref().expect("built above").values;
            self.rhi.write_buffer(buffer, 0, bytemuck::bytes_of(&values));
        }
    }
}
