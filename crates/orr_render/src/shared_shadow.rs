//! Coordinator-owned finite directional shadow coverage: one 16 MiB map that
//! holds up to four cascades, each in its own tile.
use crate::{
    Camera3D, Lighting,
    math3::Mat4,
    model_renderer::{Globals, ModelRenderError, prepare_globals},
};
use orr_rhi::*;
use std::sync::atomic::{AtomicU64, Ordering};

/// Width and height of the shared shadow map. One cascade uses all of it;
/// two to four cascades each use a quarter tile of half this size.
pub const SHARED_SHADOW_MAP_SIZE: u32 = 2048;
use crate::MAX_SHADOW_CASCADES;

const _: () = assert!(std::mem::size_of::<ShadowCascadesUniform>() as u64 == crate::SHADOW_CASCADES_UNIFORM_BYTES);

/// The uniform `ShadowCascades` of shadow_cascades.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShadowCascadesUniform {
    pub vp: [[[f32; 4]; 4]; 4],
    pub tiles: [[f32; 4]; 4],
    pub ranges: [[f32; 4]; 4],
    pub eye: [f32; 4],
    pub forward: [f32; 4],
}

/// One cascade of a frame: what its casters draw with and where.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowCascade {
    /// World to the cascade's clip space.
    pub light_vp: [[f32; 4]; 4],
    /// The cascade's tile of the map in pixels: x, y, width, height.
    pub viewport: [f32; 4],
}

/// The world-space sphere a light matrix covers, with its texel snapping.
fn sun_matrix(direction: [f32; 3], center: [f32; 3], radius: f32, texels: u32, back: f32) -> Mat4 {
    use crate::math3::{cross, dot, normalize};
    let dir = normalize(direction);
    let dir = if dot(dir, dir) < 0.5 { [0.0, -1.0, 0.0] } else { dir };
    let helper = if dir[1].abs() > 0.95 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let s = normalize(cross(dir, helper));
    let u = cross(s, dir);
    // Snapping the center to whole texels keeps edges still while the view moves.
    let texel = 2.0 * radius / texels.max(1) as f32;
    let snap = |v: f32| (v / texel).round() * texel;
    let (cs, cu, cf) = (snap(dot(center, s)), snap(dot(center, u)), snap(dot(center, dir)));
    let c = [s[0] * cs + u[0] * cu + dir[0] * cf, s[1] * cs + u[1] * cu + dir[1] * cf, s[2] * cs + u[2] * cu + dir[2] * cf];
    // Casters up to `back` meters toward the sun still reach the cascade.
    let eye = [c[0] - dir[0] * (radius + back), c[1] - dir[1] * (radius + back), c[2] - dir[2] * (radius + back)];
    let view = Mat4::look_at(eye, c, u);
    Mat4::orthographic(-radius, radius, -radius, radius, 0.1, 2.0 * radius + back).mul(&view)
}

/// The part of each cascade's far end where it fades into the next one.
const CASCADE_FADE: f32 = 0.12;

/// Far ends of `count` cascades from `near` to `far` along the view: three
/// parts growing steps to one part equal steps, so near cascades are short.
fn cascade_splits(near: f32, far: f32, count: u32) -> [f32; 4] {
    let mut splits = [far; 4];
    for (i, split) in splits.iter_mut().enumerate().take(count as usize) {
        let t = (i + 1) as f32 / count as f32;
        let log = near * (far / near).powf(t);
        let linear = near + (far - near) * t;
        *split = 0.75 * log + 0.25 * linear;
    }
    splits
}

/// The center along the view and the radius of a sphere around the part of
/// the view between distances `near` and `far`.
fn slice_sphere(camera: &Camera3D, aspect: f32, near: f32, far: f32) -> (f32, f32) {
    match camera.projection {
        crate::Projection::Perspective { fov_y, .. } => {
            let ty = (fov_y * 0.5).tan();
            let k2 = ty * ty * (1.0 + aspect * aspect);
            // The sphere through the near and far corner rings; its center never passes the far plane.
            let center = (0.5 * (near + far) * (1.0 + k2)).min(far);
            let radius = ((far - center).powi(2) + far * far * k2).sqrt();
            (center, radius)
        }
        crate::Projection::Orthographic { half_height, .. } => {
            let half_width = half_height * aspect;
            let center = 0.5 * (near + far);
            (center, (0.25 * (far - near).powi(2) + half_height * half_height + half_width * half_width).sqrt())
        }
    }
}

pub const MAX_IMPORTED_CASTERS: usize = 4096;

/// Validated CPU-only shadow state. Only the coordinator can produce this token.
pub(crate) struct PreparedShadow {
    /// The first cascade's matrix, also the whole map's with one cascade.
    pub matrix: Mat4,
    pub params: [f32; 4],
    pub cascades: Vec<ShadowCascade>,
    pub uniform: ShadowCascadesUniform,
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
        let size = SHARED_SHADOW_MAP_SIZE as f32;
        // One cascade: the whole map, any distance along the view.
        let uniform = ShadowCascadesUniform { vp: [matrix.0; 4], tiles: [[0.0, 0.0, 1.0, params[2]]; 4], ranges: [[params[1], 1.0e30, params[0], 0.0]; 4], eye: [0.0, 0.0, 0.0, 1.0], forward: [0.0, 0.0, 1.0, 0.0] };
        let cascades = vec![ShadowCascade { light_vp: matrix.0, viewport: [0.0, 0.0, size, size] }];
        Ok(Some(Self { matrix, params, cascades, uniform }))
    }

    /// Shadows in `count` cascades (2 to 4) along the view of `camera`, up to
    /// 1.5 × `lighting.shadow_radius` from the eye. Each cascade covers a
    /// sphere around its stretch of the view and is drawn into a quarter of
    /// the map; one cascade keeps the sphere of [`Self::prepare`].
    pub fn prepare_cascaded(lighting: &Lighting, camera: &Camera3D, aspect: f32, count: u32) -> Result<Option<Self>, ModelRenderError> {
        let Some(single) = Self::prepare(lighting)? else { return Ok(None) };
        let count = count.clamp(1, MAX_SHADOW_CASCADES);
        if count == 1 {
            return Ok(Some(single));
        }
        let (near, _) = match camera.projection {
            crate::Projection::Perspective { near, far, .. } | crate::Projection::Orthographic { near, far, .. } => (near, far),
        };
        let near = near.max(0.05);
        let far = (1.5 * lighting.shadow_radius).max(near * 2.0);
        let splits = cascade_splits(near, far, count);
        let forward = camera.forward();
        let tile = SHARED_SHADOW_MAP_SIZE / 2;
        let mut uniform = single.uniform;
        let mut cascades = Vec::with_capacity(count as usize);
        let mut start = near;
        for (k, end) in splits.iter().copied().enumerate().take(count as usize) {
            let (along, radius) = slice_sphere(camera, aspect.max(1e-3), start, end);
            // Whole-meter steps keep a cascade's size, and so its texels, still while the view turns.
            let radius = radius.ceil().max(1.0);
            let center = [camera.eye[0] + forward[0] * along, camera.eye[1] + forward[1] * along, camera.eye[2] + forward[2] * along];
            let back = (4.0 * radius).max(200.0);
            let matrix = sun_matrix(lighting.direction, center, radius, tile, back);
            let texel_world = 2.0 * radius / tile as f32;
            let (x, y) = ((k as u32 % 2) * tile, (k as u32 / 2) * tile);
            uniform.vp[k] = matrix.0;
            uniform.tiles[k] = [x as f32 / SHARED_SHADOW_MAP_SIZE as f32, y as f32 / SHARED_SHADOW_MAP_SIZE as f32, 0.5, 1.5 * texel_world / (2.0 * radius + back)];
            uniform.ranges[k] = [1.5 * texel_world, end, 1.0 / SHARED_SHADOW_MAP_SIZE as f32, 0.0];
            cascades.push(ShadowCascade { light_vp: matrix.0, viewport: [x as f32, y as f32, tile as f32, tile as f32] });
            // The next cascade also covers this one's fade band, so the fade always has a partner.
            start = end * (1.0 - CASCADE_FADE);
        }
        uniform.eye = [camera.eye[0], camera.eye[1], camera.eye[2], count as f32];
        uniform.forward = [forward[0], forward[1], forward[2], CASCADE_FADE];
        if !uniform.vp.iter().flatten().flatten().chain(uniform.tiles.iter().flatten()).chain(uniform.ranges.iter().flatten()).all(|v| v.is_finite()) {
            return Err(ModelRenderError::InvalidLighting);
        }
        Ok(Some(Self { matrix: Mat4(uniform.vp[0]), params: single.params, cascades, uniform }))
    }
}
pub(crate) fn prepare_shared_globals(
    format: TextureFormat,
    size: (u32, u32),
    camera: &Camera3D,
    lighting: &Lighting,
    cascades: u32,
) -> Result<(Globals, Option<PreparedShadow>), ModelRenderError> {
    let mut unshadowed = *lighting;
    unshadowed.shadows = false;
    let mut globals = prepare_globals(format, size, camera, &unshadowed)?;
    let aspect = size.0.max(1) as f32 / size.1.max(1) as f32;
    let shadow = PreparedShadow::prepare_cascaded(lighting, camera, aspect, cascades)?;
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
    /// The frame's [`ShadowCascadesUniform`].
    pub cascades: B::Buffer,
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
        let sampler = rhi.create_sampler(&SamplerDesc::SHADOW);
        let cascades = rhi.create_buffer(&BufferDesc {
            label: "shadow cascades",
            size: std::mem::size_of::<ShadowCascadesUniform>() as u64,
            usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST,
        });
        Self {
            _texture: texture,
            view,
            sampler,
            cascades,
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
                        Binding::Uniform {
                            binding: 2,
                            buffer: &map.cascades,
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
