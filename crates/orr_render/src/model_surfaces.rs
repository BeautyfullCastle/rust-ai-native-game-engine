//! Surface shaders on the instances of an imported model, static or skinned
//! (see [`crate::surface`]). Each added shader gets two pipelines on the
//! model's vertex layout: one for frames without the shared sun shadow and one
//! that receives it. The shadow itself is still cast with the mesh's built-in
//! shadow pipeline.

use crate::shared_shadow::SharedShadow;
use crate::surface::{SurfaceId, SurfaceShader, MAX_SURFACE_FLOATS, MAX_SURFACE_TEXTURES};
use crate::surface_texture::{default_textures, GpuTexture, SurfaceTexture, TextureId, TextureTable};
use bytemuck::{Pod, Zeroable};
use orr_rhi::{Binding, Blend, BufferDesc, BufferUsage, Cull, DepthState, PipelineDesc, Rhi, TextureFormat, Topology, VertexAttr, VertexLayout, VertexStep};

/// The surface shader of one model instance, its parameter values and images.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelSurface {
    /// A shader added to the instance's renderer with `add_surface`.
    pub surface: SurfaceId,
    /// Values in the order the shader declares them (see [`SurfaceShader::default_values`]).
    pub params: [f32; MAX_SURFACE_FLOATS],
    /// The image of each declared texture slot, added to the same renderer
    /// with `add_texture`. `None` and an id the renderer does not have read
    /// the slot's declared default.
    pub textures: [Option<TextureId>; MAX_SURFACE_TEXTURES],
}

impl ModelSurface {
    /// `surface` with `params` and every texture slot at its default.
    pub fn new(surface: SurfaceId, params: [f32; MAX_SURFACE_FLOATS]) -> Self {
        Self { surface, params, textures: [None; MAX_SURFACE_TEXTURES] }
    }
}

/// The per-draw uniform `surface_values` of surface_model_glue.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Values {
    values: [f32; MAX_SURFACE_FLOATS],
    time: [f32; 4],
}

/// How a renderer's surface pipelines read its vertices.
pub(crate) struct VertexInput {
    /// The surface glue with its vertex entry point.
    pub glue: &'static str,
    pub vs_entry: &'static str,
    pub stride: u64,
    pub attrs: Vec<VertexAttr>,
}

pub(crate) struct SurfaceGpu<B: Rhi> {
    /// Changes when the shader is replaced, so draws rebuild their bindings.
    version: u64,
    pub main: B::Pipeline,
    pub receiver: B::Pipeline,
    receiver_bind: Option<(u64, B::BindGroup)>,
    /// What each slot reads while the instance picks no image.
    defaults: Vec<GpuTexture<B>>,
    /// Repeats and filters between mip levels.
    sampler: B::Sampler,
}

impl<B: Rhi> SurfaceGpu<B> {
    /// The shared shadow map for [`Self::receiver`]; bound before every frame with shadows.
    pub fn receiver_bind(&self) -> &B::BindGroup {
        &self.receiver_bind.as_ref().expect("shared shadow bound before encoding").1
    }
}

/// Bindings of one draw for one surface shader.
pub(crate) struct SurfaceDraw<B: Rhi> {
    surface: SurfaceId,
    version: u64,
    /// The textures bound to the slots, by stamp.
    stamps: [u64; MAX_SURFACE_TEXTURES],
    values: B::Buffer,
    pub main: B::BindGroup,
    pub receiver: B::BindGroup,
}

/// The buffers and image one draw binds besides its surface: the renderer's
/// globals, the primitive's base color image and the draw's object uniform.
pub(crate) struct DrawInputs<'a, B: Rhi> {
    pub globals: &'a B::Buffer,
    pub image: &'a B::TextureView,
    pub object: &'a B::Buffer,
}

pub(crate) struct ModelSurfaces<B: Rhi> {
    /// The model shader the surface pipelines are built on.
    engine: String,
    input: VertexInput,
    gpus: Vec<SurfaceGpu<B>>,
    /// Surface of each instance of the next frame, by instance index.
    uses: Vec<Option<ModelSurface>>,
    time: f32,
    next_version: u64,
    textures: TextureTable<B>,
}

fn finite(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

impl<B: Rhi> ModelSurfaces<B> {
    pub fn new(engine: String, input: VertexInput) -> Self {
        Self { engine, input, gpus: Vec::new(), uses: Vec::new(), time: 0.0, next_version: 0, textures: TextureTable::default() }
    }

    pub fn add(&mut self, rhi: &B, format: TextureFormat, shader: &SurfaceShader) -> SurfaceId {
        let gpu = self.build(rhi, format, shader);
        self.gpus.push(gpu);
        SurfaceId(self.gpus.len() as u32 - 1)
    }

    pub fn replace(&mut self, rhi: &B, format: TextureFormat, id: SurfaceId, shader: &SurfaceShader) {
        if (id.0 as usize) < self.gpus.len() {
            let gpu = self.build(rhi, format, shader);
            self.gpus[id.0 as usize] = gpu;
        }
    }

    pub fn add_texture(&mut self, rhi: &B, texture: &SurfaceTexture) -> TextureId {
        self.textures.add(rhi, texture)
    }

    pub fn replace_texture(&mut self, rhi: &B, id: TextureId, texture: &SurfaceTexture) {
        self.textures.replace(rhi, id, texture);
    }

    pub fn set_uses(&mut self, surfaces: &[Option<ModelSurface>], time: f32) {
        self.uses.clear();
        self.uses.extend_from_slice(surfaces);
        self.time = time;
    }

    pub fn bind_map(&mut self, rhi: &B, map: &SharedShadow<B>) {
        for gpu in &mut self.gpus {
            if gpu.receiver_bind.as_ref().is_none_or(|(id, _)| *id != map.id) {
                let bind = rhi.create_bind_group(&gpu.receiver, 1, &[Binding::Texture { binding: 0, view: &map.view }, Binding::Sampler { binding: 1, sampler: &map.sampler }, Binding::Uniform { binding: 2, buffer: &map.cascades }, Binding::Texture { binding: 3, view: &map.spot_view }]);
                gpu.receiver_bind = Some((map.id, bind));
            }
        }
    }

    fn build(&mut self, rhi: &B, format: TextureFormat, shader: &SurfaceShader) -> SurfaceGpu<B> {
        let source = shader.source_for(&self.engine, self.input.glue);
        let module = rhi.create_shader("model surface shader", &source);
        let layouts = [VertexLayout { stride: self.input.stride, step: VertexStep::Vertex, attrs: &self.input.attrs }];
        let pipeline = |label, fs_entry| {
            rhi.create_pipeline(&PipelineDesc::<B> {
                label,
                shader: &module,
                vs_entry: self.input.vs_entry,
                fs_entry,
                vertex_buffers: &layouts,
                color_format: Some(format),
                blend: Blend::Opaque,
                topology: Topology::TriangleList,
                cull: Cull::Back,
                depth: Some(DepthState::opaque(TextureFormat::Depth32Float)),
                samples: 1,
            })
        };
        let main = pipeline("model surface", "fs_model_surface");
        let receiver = pipeline("model surface shadow receiver", "fs_model_surface_shadowed");
        self.next_version += 1;
        SurfaceGpu { version: self.next_version, main, receiver, receiver_bind: None, defaults: default_textures(rhi, shader), sampler: rhi.create_sampler(&orr_rhi::SamplerDesc::MATERIAL) }
    }

    /// The surface instance `instance` asks for, if this renderer has it.
    fn wanted(&self, instance: usize) -> Option<ModelSurface> {
        let wanted = (*self.uses.get(instance)?)?;
        ((wanted.surface.0 as usize) < self.gpus.len()).then_some(wanted)
    }

    /// The pipelines and bindings a draw of `instance` uses, given the draw's
    /// built bindings, or `None` for the built-in look.
    pub fn active<'a>(&'a self, instance: usize, draw: Option<&'a SurfaceDraw<B>>) -> Option<(&'a SurfaceGpu<B>, &'a SurfaceDraw<B>)> {
        let wanted = self.wanted(instance)?;
        let gpu = &self.gpus[wanted.surface.0 as usize];
        let draw = draw?;
        let stamps = self.textures.resolve(&wanted.textures, &gpu.defaults).map(|t| t.stamp);
        (draw.surface == wanted.surface && draw.version == gpu.version && draw.stamps == stamps).then_some((gpu, draw))
    }

    /// Builds the bindings of a draw of `instance` in `slot` when they are
    /// missing or stale, and writes its values.
    pub fn write(&self, rhi: &B, instance: usize, slot: &mut Option<SurfaceDraw<B>>, inputs: DrawInputs<'_, B>) {
        let Some(wanted) = self.wanted(instance) else { return };
        let gpu = &self.gpus[wanted.surface.0 as usize];
        let textures = self.textures.resolve(&wanted.textures, &gpu.defaults);
        let stamps = textures.map(|t| t.stamp);
        if slot.as_ref().is_none_or(|d| d.surface != wanted.surface || d.version != gpu.version || d.stamps != stamps) {
            let values = slot.take().map_or_else(
                || rhi.create_buffer(&BufferDesc { label: "model surface values", size: std::mem::size_of::<Values>() as u64, usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST }),
                |old| old.values,
            );
            let bind = |pipeline| {
                rhi.create_bind_group(pipeline, 0, &[
                    Binding::Uniform { binding: 0, buffer: inputs.globals },
                    Binding::Texture { binding: 1, view: inputs.image },
                    Binding::Uniform { binding: 2, buffer: inputs.object },
                    Binding::Uniform { binding: 3, buffer: &values },
                    Binding::Texture { binding: 4, view: &textures[0].view },
                    Binding::Texture { binding: 5, view: &textures[1].view },
                    Binding::Texture { binding: 6, view: &textures[2].view },
                    Binding::Texture { binding: 7, view: &textures[3].view },
                    Binding::Sampler { binding: 8, sampler: &gpu.sampler },
                ])
            };
            let main = bind(&gpu.main);
            let receiver = bind(&gpu.receiver);
            *slot = Some(SurfaceDraw { surface: wanted.surface, version: gpu.version, stamps, values, main, receiver });
        }
        let values = Values { values: wanted.params.map(finite), time: [finite(self.time), 0.0, 0.0, 0.0] };
        rhi.write_buffer(&slot.as_ref().expect("built above").values, 0, bytemuck::bytes_of(&values));
    }
}
