//! Surface shaders on the 3D renderer: one pipeline per shader, instances
//! grouped by shader and mesh kind each frame, drawn in the shadow pass with
//! the shared shadow pipeline and in the main pass with their own pipeline.

use super::{Renderer3D, DEPTH, INSTANCE_ATTRS, MESH_ATTRS};
use crate::list3d::{Instance3D, RenderList3D, SurfaceInstance};
use crate::mesh::{MeshKind, Vertex3};
use crate::stats::{FrameStats, PassStats};
use crate::surface::{SurfaceId, SurfaceShader, MAX_SURFACE_FLOATS, MAX_SURFACE_TEXTURES};
use crate::surface_texture::{default_textures, GpuTexture, SurfaceTexture, TextureId};
use orr_rhi::{Binding, Blend, Command, Cull, DepthState, PipelineDesc, Rhi, VertexAttr, VertexFormat, VertexLayout, VertexStep};

/// Parameters of one instance: four vec4 at locations 8 to 11.
const PARAM_ATTRS: [VertexAttr; 4] = [
    VertexAttr { location: 8, format: VertexFormat::Float32x4, offset: 0 },
    VertexAttr { location: 9, format: VertexFormat::Float32x4, offset: 16 },
    VertexAttr { location: 10, format: VertexFormat::Float32x4, offset: 32 },
    VertexAttr { location: 11, format: VertexFormat::Float32x4, offset: 48 },
];

pub(super) struct SurfaceGpu<B: Rhi> {
    pipeline: B::Pipeline,
    main_bind: B::BindGroup,
    /// What each slot reads while an instance picks no image.
    defaults: Vec<GpuTexture<B>>,
    /// Repeats and filters between mip levels.
    sampler: B::Sampler,
    /// Texture bindings (group 1) of the image sets this frame draws, by stamps.
    texture_binds: Vec<([u64; MAX_SURFACE_TEXTURES], B::BindGroup)>,
    /// Bound to the shared shadow map of the composed path, with that map's id.
    #[cfg(feature = "imported-scene")]
    composed_bind: Option<(u64, B::BindGroup)>,
}

/// The images of one group of instances.
type Images = [Option<TextureId>; MAX_SURFACE_TEXTURES];

/// Instances of one frame grouped by shader, images and mesh kind.
#[derive(Default)]
pub(super) struct SurfaceFrame {
    groups: Vec<(SurfaceId, Images, MeshKind, std::ops::Range<u32>)>,
    /// Stamps of the textures each group binds, by group.
    stamps: Vec<[u64; MAX_SURFACE_TEXTURES]>,
    order: Vec<u32>,
    instances: Vec<Instance3D>,
    params: Vec<[f32; MAX_SURFACE_FLOATS]>,
}

impl SurfaceFrame {
    pub(super) fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }
}

/// Draw calls a list's surface instances need, and how many of them cast shadows.
#[cfg(feature = "imported-scene")]
pub(crate) fn surface_draws(list: &RenderList3D) -> (usize, usize) {
    let mut keys: Vec<(SurfaceId, Images, u8)> = list.surfaces.iter().map(|s| (s.surface, s.textures, s.kind as u8)).collect();
    keys.sort_unstable();
    keys.dedup();
    let casters = keys.iter().filter(|(_, _, kind)| *kind != MeshKind::Plane as u8).count();
    (keys.len(), casters)
}

impl<B: Rhi> Renderer3D<B> {
    /// Adds a pipeline for `shader` and returns the id that render list
    /// instances use to pick it (see [`RenderList3D::with_surface`]).
    /// [`SurfaceShader::compile`] validated the shader against this renderer's
    /// own shader, so this creates no invalid GPU object.
    pub fn add_surface(&mut self, shader: &SurfaceShader) -> SurfaceId {
        let gpu = self.build_surface(shader);
        self.surfaces.push(gpu);
        SurfaceId(self.surfaces.len() as u32 - 1)
    }

    fn build_surface(&self, shader: &SurfaceShader) -> SurfaceGpu<B> {
        let source = shader.source_for(&self.engine_source, crate::surface::SHAPE_GLUE);
        let module = self.rhi.create_shader(&format!("surface shader {}", shader.name()), &source);
        let layouts = [
            VertexLayout { stride: std::mem::size_of::<Vertex3>() as u64, step: VertexStep::Vertex, attrs: &MESH_ATTRS },
            VertexLayout { stride: std::mem::size_of::<Instance3D>() as u64, step: VertexStep::Instance, attrs: &INSTANCE_ATTRS },
            VertexLayout { stride: (MAX_SURFACE_FLOATS * 4) as u64, step: VertexStep::Instance, attrs: &PARAM_ATTRS },
        ];
        let pipeline = self.rhi.create_pipeline(&PipelineDesc::<B> {
            label: "3d surface shader",
            shader: &module,
            vs_entry: "vs_surface",
            fs_entry: "fs_surface",
            vertex_buffers: &layouts,
            color_format: Some(self.format),
            blend: Blend::Opaque,
            topology: orr_rhi::Topology::TriangleList,
            cull: Cull::Back,
            depth: Some(DepthState::opaque(DEPTH)),
            samples: self.samples,
        });
        let main_bind = self.rhi.create_bind_group(
            &pipeline,
            0,
            &[
                Binding::Uniform { binding: 0, buffer: &self.globals },
                Binding::Texture { binding: 1, view: &self.shadow_view },
                Binding::Sampler { binding: 2, sampler: &self.sampler },
                Binding::Uniform { binding: 3, buffer: &self.no_cascades },
            ],
        );
        SurfaceGpu {
            pipeline,
            main_bind,
            defaults: default_textures(&self.rhi, shader),
            sampler: self.rhi.create_sampler(&orr_rhi::SamplerDesc::MATERIAL),
            texture_binds: Vec::new(),
            #[cfg(feature = "imported-scene")]
            composed_bind: None,
        }
    }

    /// Replaces the shader of `id` with `shader`, keeping the id. An id this
    /// renderer does not have is left alone.
    pub fn replace_surface(&mut self, id: SurfaceId, shader: &SurfaceShader) {
        if id.0 as usize >= self.surfaces.len() {
            return;
        }
        let gpu = self.build_surface(shader);
        self.surfaces[id.0 as usize] = gpu;
    }
    /// Adds an image for surface shader texture slots and returns the id that
    /// render list instances use to pick it (see [`RenderList3D::with_textured_surface`]).
    pub fn add_texture(&mut self, texture: &SurfaceTexture) -> TextureId {
        self.surface_textures.add(&self.rhi, texture)
    }

    /// Replaces the image of `id`, keeping the id. An id this renderer does
    /// not have is left alone.
    pub fn replace_texture(&mut self, id: TextureId, texture: &SurfaceTexture) {
        self.surface_textures.replace(&self.rhi, id, texture);
    }

    /// Groups the list's surface instances by shader, images and mesh kind and uploads them.
    /// Instances of an id this renderer does not have are skipped.
    pub(super) fn prepare_surfaces(&mut self, list: &RenderList3D, stats: &mut FrameStats) {
        let frame = &mut self.surface_frame;
        frame.groups.clear();
        frame.stamps.clear();
        frame.order.clear();
        frame.instances.clear();
        frame.params.clear();
        let known = self.surfaces.len() as u32;
        frame.order.extend((0..list.surfaces.len() as u32).filter(|&i| list.surfaces[i as usize].surface.0 < known));
        let key = |s: &SurfaceInstance| (s.surface, s.textures, s.kind as u8);
        frame.order.sort_by_key(|&i| key(&list.surfaces[i as usize]));
        for &i in &frame.order {
            let s = &list.surfaces[i as usize];
            let at = frame.instances.len() as u32;
            match frame.groups.last_mut() {
                Some((id, images, kind, range)) if *id == s.surface && *images == s.textures && *kind == s.kind => range.end = at + 1,
                _ => frame.groups.push((s.surface, s.textures, s.kind, at..at + 1)),
            }
            frame.instances.push(s.instance);
            frame.params.push(s.params);
        }
        // Each group's texture binding: built when its images are new, dropped when no group uses it.
        for (id, images, _, _) in &frame.groups {
            let gpu = &mut self.surfaces[id.0 as usize];
            let textures = self.surface_textures.resolve(images, &gpu.defaults);
            let stamps = textures.map(|t| t.stamp);
            if !gpu.texture_binds.iter().any(|(s, _)| *s == stamps) {
                let mut entries: Vec<Binding<'_, B>> = (0..MAX_SURFACE_TEXTURES).map(|k| Binding::Texture { binding: k as u32, view: &textures[k].view }).collect();
                entries.push(Binding::Sampler { binding: MAX_SURFACE_TEXTURES as u32, sampler: &gpu.sampler });
                let bind = self.rhi.create_bind_group(&gpu.pipeline, 1, &entries);
                gpu.texture_binds.push((stamps, bind));
            }
            frame.stamps.push(stamps);
        }
        for (index, gpu) in self.surfaces.iter_mut().enumerate() {
            let used: Vec<&[u64; MAX_SURFACE_TEXTURES]> = frame.groups.iter().zip(&frame.stamps).filter(|((id, ..), _)| id.0 as usize == index).map(|(_, s)| s).collect();
            gpu.texture_binds.retain(|(s, _)| used.contains(&s));
        }
        if frame.instances.is_empty() {
            return;
        }
        let rhi = &self.rhi;
        stats.buffer_reallocations += u32::from(self.surface_instances.reserve(rhi, frame.instances.len()));
        stats.buffer_reallocations += u32::from(self.surface_params.reserve(rhi, frame.params.len()));
        stats.upload(rhi, &self.surface_instances.buffer, 0, bytemuck::cast_slice(&frame.instances));
        stats.upload(rhi, &self.surface_params.buffer, 0, bytemuck::cast_slice(&frame.params));
        stats.mesh_instances += frame.instances.len() as u64;
    }

    /// Shadow casting surface instances, drawn with the shadow pipeline bound to `bind`.
    pub(super) fn surface_shadow_commands<'a>(&'a self, bind: &'a B::BindGroup, commands: &mut Vec<Command<'a, B>>, pass: &mut PassStats) {
        if self.surface_frame.is_empty() {
            return;
        }
        commands.extend([
            Command::SetPipeline(&self.shadow_pipeline),
            Command::SetBindGroup(0, bind),
            Command::SetVertexBuffer(0, &self.mesh_vertices),
            Command::SetVertexBuffer(1, &self.surface_instances.buffer),
            Command::SetIndexBuffer(&self.mesh_indices),
        ]);
        for (_, _, kind, instances) in &self.surface_frame.groups {
            if *kind == MeshKind::Plane {
                continue;
            }
            let (indices, base_vertex) = self.mesh_ranges[*kind as usize].clone();
            pass.draw(instances.end - instances.start);
            commands.push(Command::DrawIndexed { indices, base_vertex, instances: instances.clone() });
        }
    }

    /// Main pass draws of the surface instances. `composed` picks the bind
    /// group of the composed path's shared shadow map.
    pub(super) fn surface_main_commands<'a>(&'a self, composed: bool, commands: &mut Vec<Command<'a, B>>, pass: &mut PassStats) {
        let mut current = None;
        let mut images = None;
        for ((id, _, kind, instances), stamps) in self.surface_frame.groups.iter().zip(&self.surface_frame.stamps) {
            let gpu = &self.surfaces[id.0 as usize];
            if current != Some(*id) {
                current = Some(*id);
                images = None;
                let bind = if composed { self.composed_surface_bind(gpu) } else { &gpu.main_bind };
                commands.extend([
                    Command::SetPipeline(&gpu.pipeline),
                    Command::SetBindGroup(0, bind),
                    Command::SetVertexBuffer(0, &self.mesh_vertices),
                    Command::SetVertexBuffer(1, &self.surface_instances.buffer),
                    Command::SetVertexBuffer(2, &self.surface_params.buffer),
                    Command::SetIndexBuffer(&self.mesh_indices),
                ]);
            }
            if images != Some(stamps) {
                images = Some(stamps);
                let (_, bind) = gpu.texture_binds.iter().find(|(s, _)| s == stamps).expect("built in prepare_surfaces");
                commands.push(Command::SetBindGroup(1, bind));
            }
            let (indices, base_vertex) = self.mesh_ranges[*kind as usize].clone();
            pass.draw(instances.end - instances.start);
            commands.push(Command::DrawIndexed { indices, base_vertex, instances: instances.clone() });
        }
    }

    #[cfg(feature = "imported-scene")]
    fn composed_surface_bind<'a>(&'a self, gpu: &'a SurfaceGpu<B>) -> &'a B::BindGroup {
        gpu.composed_bind.as_ref().map_or(&gpu.main_bind, |(_, bind)| bind)
    }

    #[cfg(not(feature = "imported-scene"))]
    fn composed_surface_bind<'a>(&'a self, gpu: &'a SurfaceGpu<B>) -> &'a B::BindGroup {
        &gpu.main_bind
    }

    /// Binds every surface pipeline to the composed path's shared shadow map.
    #[cfg(feature = "imported-scene")]
    pub(super) fn bind_surfaces_to_shared_shadow(&mut self, map: &crate::shared_shadow::SharedShadow<B>) {
        for gpu in &mut self.surfaces {
            if gpu.composed_bind.as_ref().is_none_or(|(id, _)| *id != map.id) {
                let bind = self.rhi.create_bind_group(
                    &gpu.pipeline,
                    0,
                    &[
                        Binding::Uniform { binding: 0, buffer: &self.globals },
                        Binding::Texture { binding: 1, view: &map.view },
                        Binding::Sampler { binding: 2, sampler: &map.sampler },
                        Binding::Uniform { binding: 3, buffer: &map.cascades },
                    ],
                );
                gpu.composed_bind = Some((map.id, bind));
            }
        }
    }
}
