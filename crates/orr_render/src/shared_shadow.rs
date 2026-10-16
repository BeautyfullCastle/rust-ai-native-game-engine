//! Coordinator-owned finite directional shadow coverage. One fixed 4 MiB map.
use crate::{
    Camera3D, Lighting,
    math3::Mat4,
    model_renderer::{Globals, ModelRenderError, prepare_globals},
};
use orr_rhi::*;
use std::sync::atomic::{AtomicU64, Ordering};

pub const SHARED_SHADOW_MAP_SIZE: u32 = 1024;
pub const MAX_IMPORTED_CASTERS: usize = 4096;

/// Validated CPU-only shadow state. Only the coordinator can produce this token.
pub(crate) struct PreparedShadow {
    pub matrix: Mat4,
    pub params: [f32; 4],
}
impl PreparedShadow {
    pub fn prepare(lighting: &Lighting) -> Result<Option<Self>, ModelRenderError> {
        if !lighting.shadows {
            return Ok(None);
        }
        if !lighting
            .shadow_center
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 1e6)
            || !lighting.shadow_radius.is_finite()
            || !(0.5..=1e5).contains(&lighting.shadow_radius)
        {
            return Err(ModelRenderError::InvalidLighting);
        }
        let matrix = crate::renderer3d::light_view_proj(lighting, SHARED_SHADOW_MAP_SIZE);
        let params = [
            1.0 / SHARED_SHADOW_MAP_SIZE as f32,
            3.0 * lighting.shadow_radius / SHARED_SHADOW_MAP_SIZE as f32,
            0.0004,
            1.0,
        ];
        if !matrix
            .0
            .iter()
            .flatten()
            .chain(params.iter())
            .all(|v| v.is_finite())
        {
            return Err(ModelRenderError::InvalidLighting);
        }
        Ok(Some(Self { matrix, params }))
    }
}
pub(crate) fn prepare_shared_globals(
    format: TextureFormat,
    size: (u32, u32),
    camera: &Camera3D,
    lighting: &Lighting,
) -> Result<(Globals, Option<PreparedShadow>), ModelRenderError> {
    let mut unshadowed = *lighting;
    unshadowed.shadows = false;
    let mut globals = prepare_globals(format, size, camera, &unshadowed)?;
    let shadow = PreparedShadow::prepare(lighting)?;
    if let Some(ready) = &shadow {
        globals.light_vp = ready.matrix.0;
        globals.shadow = ready.params;
    }
    Ok((globals, shadow))
}

pub(crate) struct SharedShadow<B: Rhi> {
    _texture: B::Texture,
    pub view: B::TextureView,
    pub sampler: B::Sampler,
    pub id: u64,
}
impl<B: Rhi> SharedShadow<B> {
    pub fn new(rhi: &B) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let texture = rhi.create_texture(&TextureDesc {
            label: "shared directional shadow map",
            width: SHARED_SHADOW_MAP_SIZE,
            height: SHARED_SHADOW_MAP_SIZE,
            format: TextureFormat::Depth32Float,
            usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::TEXTURE_BINDING,
            sample_count: 1,
            view_formats: &[],
        });
        let view = rhi.create_texture_view(&texture, None);
        let sampler = rhi.create_sampler(&SamplerDesc {
            linear: true,
            compare: true,
        });
        Self {
            _texture: texture,
            view,
            sampler,
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        }
    }
}
/// Inferred layouts are pipeline-specific even when their entries look identical.
pub(crate) struct ShadowPipelines<B: Rhi> {
    pub main: B::Pipeline,
    pub depth: B::Pipeline,
    receiver: Option<(u64, B::BindGroup)>,
}
pub(crate) struct ObjectBindings<B: Rhi> {
    pub main: B::BindGroup,
    pub depth: B::BindGroup,
}
impl<B: Rhi> ShadowPipelines<B> {
    pub fn new(
        rhi: &B,
        shader: &B::Shader,
        format: TextureFormat,
        layouts: &[VertexLayout<'_>],
    ) -> Self {
        let main = rhi.create_pipeline(&PipelineDesc::<B> {
            label: "shared shadow receiver",
            shader,
            vs_entry: "vs_main",
            fs_entry: "fs_composed",
            vertex_buffers: layouts,
            color_format: Some(format),
            blend: Blend::Opaque,
            topology: Topology::TriangleList,
            cull: Cull::Back,
            depth: Some(DepthState::opaque(TextureFormat::Depth32Float)),
            samples: 1,
        });
        // Imported opaque sheets cast from both sides. Closed procedural meshes retain front culling.
        let depth = rhi.create_pipeline(&PipelineDesc::<B> {
            label: "two-sided imported shadow caster",
            shader,
            vs_entry: "vs_shadow",
            fs_entry: "",
            vertex_buffers: layouts,
            color_format: None,
            blend: Blend::Opaque,
            topology: Topology::TriangleList,
            cull: Cull::None,
            depth: Some(DepthState {
                format: TextureFormat::Depth32Float,
                write: true,
                compare: Compare::Less,
                bias: 2,
                slope_bias: 2.0,
            }),
            samples: 1,
        });
        Self {
            main,
            depth,
            receiver: None,
        }
    }
    pub fn object(
        &self,
        rhi: &B,
        globals: &B::Buffer,
        uniform: &B::Buffer,
        image: &B::TextureView,
    ) -> ObjectBindings<B> {
        let main = rhi.create_bind_group(
            &self.main,
            0,
            &[
                Binding::Uniform {
                    binding: 0,
                    buffer: globals,
                },
                Binding::Texture {
                    binding: 1,
                    view: image,
                },
                Binding::Uniform {
                    binding: 2,
                    buffer: uniform,
                },
            ],
        );
        let depth = rhi.create_bind_group(
            &self.depth,
            0,
            &[
                Binding::Uniform {
                    binding: 0,
                    buffer: globals,
                },
                Binding::Uniform {
                    binding: 2,
                    buffer: uniform,
                },
            ],
        );
        ObjectBindings { main, depth }
    }
    pub fn bind_map(&mut self, rhi: &B, map: &SharedShadow<B>) {
        if self.receiver.as_ref().is_none_or(|(id, _)| *id != map.id) {
            self.receiver = Some((
                map.id,
                rhi.create_bind_group(
                    &self.main,
                    1,
                    &[
                        Binding::Texture {
                            binding: 0,
                            view: &map.view,
                        },
                        Binding::Sampler {
                            binding: 1,
                            sampler: &map.sampler,
                        },
                    ],
                ),
            ));
        }
    }
    pub fn receiver(&self) -> &B::BindGroup {
        &self
            .receiver
            .as_ref()
            .expect("validated shared map bound")
            .1
    }
}
