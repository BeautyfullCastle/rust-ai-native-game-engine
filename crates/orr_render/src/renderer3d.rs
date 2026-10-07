//! The 3D renderer: shadow pass, then one multisampled color+depth pass.
//!
//! # Frame
//!
//! 1. The instances of every mesh kind are written to one instance buffer
//!    (four `write_buffer` calls, no copy on the CPU), the debug lines to
//!    another.
//! 2. Shadow pass (when [`Lighting::shadows`]): all spheres, boxes and
//!    capsules are drawn depth only from the sun into a 2048 x 2048
//!    `Depth32Float` map. The map is an orthographic box around
//!    `shadow_center` with `shadow_radius`; its center snaps to whole texels
//!    so the shadows do not shimmer when the center moves.
//! 3. Main pass into an MSAA (4x when the adapter has it, else 1x) color
//!    texture with a depth buffer, resolved into the target view: meshes,
//!    then debug lines (depth tested, not written).
//!
//! # Shading
//!
//! A metal/roughness Blinn-Phong: see [`crate::Material`] and
//! `shader3d.wgsl`. Linear lighting, ACES tone mapping (optional), and the
//! sRGB encode is done by the hardware when the target format is sRGB and by
//! the shader otherwise, so the picture looks the same either way.
//!
//! # Shadows
//!
//! Hardware compare sampler (`textureSampleCompareLevel` on a
//! `Depth32Float` map with linear filtering, which is a 2x2 bilinear compare)
//! taken on a 3x3 grid: a smooth PCF edge about 4 texels wide. Acne is
//! controlled by a slope scaled depth bias in the shadow pipeline, front
//! face culling there, a normal offset of 1.5 texels and a small constant
//! depth bias.

use bytemuck::{Pod, Zeroable};
use orr_rhi::{
    Binding, Blend, BufferDesc, BufferUsage, ColorAttachment, Command, Compare, Cull, DepthAttachment, DepthState,
    PipelineDesc, Rhi, SamplerDesc, TextureDesc, TextureFormat, TextureUsage, VertexAttr, VertexFormat, VertexLayout,
    VertexStep,
};

use crate::camera3d::{Camera3D, Projection};
use crate::list3d::{Instance3D, Lighting, LineInstance3D, RenderList3D};
use crate::math3::{cross, dot, normalize, quat_rotate, scale, sub, Mat4, Vec3};
use crate::mesh::{MeshKind, MeshSet, Vertex3};
use crate::renderer::Growing;
use crate::stats::{CpuClock, FrameStats, PassStats};

#[cfg(feature = "imported-scene")]
mod composed;
#[cfg(feature = "imported-scene")]
pub use composed::{ProceduralSceneError, MAX_PROCEDURAL_INSTANCES};
#[cfg(feature = "imported-scene")]
pub(crate) use composed::{validate_procedural_list, PreparedProcedural};

const SHADER: &str = include_str!("shader3d.wgsl");
const DEPTH: TextureFormat = TextureFormat::Depth32Float;

/// Background color of [`Renderer3D`] (linear).
pub const DEFAULT_CLEAR_3D: [f64; 4] = [0.30, 0.42, 0.62, 1.0];

/// Quality settings fixed when the renderer is created.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Settings3D {
    /// Requested MSAA sample count (1, 2, 4, 8). The renderer uses the
    /// largest the adapter supports that is not above it.
    pub msaa: u32,
    /// Side of the square shadow map in texels.
    pub shadow_map_size: u32,
    /// Longitude segments of the sphere and capsule meshes (3 to 64; the latitude rings follow).
    pub mesh_segments: u32,
}

impl Default for Settings3D {
    fn default() -> Self {
        Self {
            msaa: 4,
            shadow_map_size: 2048,
            mesh_segments: crate::mesh::DEFAULT_SEGMENTS,
        }
    }
}

impl Settings3D {
    /// The mobile / weak-GPU preset: no MSAA (the biggest saving on tile and software renderers),
    /// a 512 texel shadow map and 12-segment round meshes (a sphere is 192 triangles instead of 1,536).
    pub const LOW: Settings3D = Settings3D {
        msaa: 1,
        shadow_map_size: 512,
        mesh_segments: 12,
    };
}

/// Opt-in sphere detail for the main pass. Shadows keep `Settings3D::mesh_segments`.
/// The radius cutoff is a conservative projected bound in physical target pixels.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SphereLod3D {
    pub far_segments: u32,
    pub max_projected_radius_px: f32,
}

impl Default for SphereLod3D {
    fn default() -> Self {
        Self {
            far_segments: 12,
            max_projected_radius_px: 6.0,
        }
    }
}

impl SphereLod3D {
    pub const LOW: Self = Self {
        far_segments: 6,
        max_projected_radius_px: 6.0,
    };

    fn validate(self, settings: Settings3D) -> Result<(), SphereLodError3D> {
        if !(3..=settings.mesh_segments.clamp(3, 64)).contains(&self.far_segments) {
            return Err(SphereLodError3D::FarSegments);
        }
        if !self.max_projected_radius_px.is_finite() || self.max_projected_radius_px <= 0.0 {
            return Err(SphereLodError3D::ProjectedRadius);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SphereLodError3D {
    /// Far detail must be in 3..=the effective near detail (clamped to 3..=64).
    FarSegments,
    /// The cutoff must be finite and positive.
    ProjectedRadius,
}

impl std::fmt::Display for SphereLodError3D {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::FarSegments => "sphere far segments must be in 3..=effective near segments",
            Self::ProjectedRadius => "sphere projected radius cutoff must be finite and positive",
        })
    }
}

impl std::error::Error for SphereLodError3D {}

/// Sphere-only work of the last draw, separate from the existing `FrameStats` layout.
/// Index invocations are submitted indices times instances, not visible triangles.
#[derive(Clone, Copy, Debug, Default)]
pub struct SphereLodStats3D {
    pub enabled: bool,
    pub near_instances: u32,
    pub far_instances: u32,
    pub fallback_instances: u32,
    pub main_draw_calls: u32,
    pub shadow_draw_calls: u32,
    pub main_index_invocations: u64,
    pub shadow_index_invocations: u64,
    pub upload_calls: u32,
    pub upload_bytes: u64,
    /// Additional vertex/index payload allocated and uploaded once at construction.
    /// Excludes allocator/driver overhead; frame upload counters exclude it.
    pub additional_static_mesh_bytes: u64,
    pub staging_capacity: usize,
    /// Capacity in classifications (`Vec<bool>` bits), not bytes.
    pub classification_capacity: usize,
    /// Number of retained CPU allocations whose capacity grew this draw (0..=2).
    pub staging_reallocations: u32,
    /// Native CPU classification + packing duration; unavailable on wasm.
    pub cpu_classify_time: Option<std::time::Duration>,
}

struct SphereLodState {
    policy: SphereLod3D,
    far_range: Option<(std::ops::Range<u32>, i32)>,
    static_bytes: u64,
    packed: Vec<Instance3D>,
    is_far: Vec<bool>,
}

/// Invalid/near-plane bounds fail closed to the original detail. No culling.
fn projected_sphere_radius(instance: &Instance3D, camera: &Camera3D, size: (u32, u32)) -> Option<f32> {
    if size.0 == 0
        || size.1 == 0
        || !camera
            .eye
            .iter()
            .chain(&camera.target)
            .chain(&camera.up)
            .all(|x| x.is_finite())
        || !bytemuck::cast_slice::<Instance3D, f32>(std::slice::from_ref(instance))
            .iter()
            .all(|x| x.is_finite())
        || instance.scale.iter().any(|&x| x <= 0.0)
    {
        return None;
    }
    let forward = sub(camera.target, camera.eye);
    let right = cross(forward, camera.up);
    if !dot(forward, forward).is_finite()
        || dot(forward, forward) <= 1e-12
        || !dot(right, right).is_finite()
        || dot(right, right) <= 1e-12
    {
        return None;
    }
    match camera.projection {
        Projection::Perspective { fov_y, near, far } => {
            if !fov_y.is_finite()
                || fov_y <= 0.0
                || fov_y >= std::f32::consts::PI
                || !near.is_finite()
                || near <= 0.0
                || !far.is_finite()
                || far <= near
            {
                return None;
            }
        }
        Projection::Orthographic { half_height, near, far } => {
            if !half_height.is_finite() || half_height <= 0.0 || !near.is_finite() || !far.is_finite() || far <= near {
                return None;
            }
        }
    }
    let q = instance.rot;
    let norm = q.iter().map(|v| v * v).sum::<f32>();
    if !norm.is_finite() || (norm - 1.0).abs() > 1e-3 {
        return None;
    }
    // Bound the actual shader transform, including nonuniform scale and rounding
    // in a nearly-unit public quaternion. A transformed local cube encloses the mesh.
    let axes = [
        quat_rotate(q, [instance.scale[0], 0.0, 0.0]),
        quat_rotate(q, [0.0, instance.scale[1], 0.0]),
        quat_rotate(q, [0.0, 0.0, instance.scale[2]]),
    ];
    let extent: Vec3 = std::array::from_fn(|i| axes.iter().map(|a| a[i].abs()).sum::<f32>());
    let vp = camera.view_proj(size.0 as f32 / size.1 as f32);
    if !extent.iter().all(|x| x.is_finite()) || !vp.0.iter().flatten().all(|x| x.is_finite()) {
        return None;
    }
    let project = |p| {
        let c = vp.transform_point4(p);
        if !c.iter().all(|x| x.is_finite()) || c[3] <= 1e-6 || c[2] <= 0.0 {
            return None;
        }
        let screen = [c[0] / c[3] * size.0 as f32 * 0.5, c[1] / c[3] * size.1 as f32 * 0.5];
        screen.iter().all(|x| x.is_finite()).then_some(screen)
    };
    let center = project(instance.pos)?;
    let mut radius_squared = 0.0f32;
    for x in [-1.0, 1.0] {
        for y in [-1.0, 1.0] {
            for z in [-1.0, 1.0] {
                let p = [
                    instance.pos[0] + x * extent[0],
                    instance.pos[1] + y * extent[1],
                    instance.pos[2] + z * extent[2],
                ];
                let screen = project(p)?;
                let dx = screen[0] - center[0];
                let dy = screen[1] - center[1];
                radius_squared = radius_squared.max(dx * dx + dy * dy);
            }
        }
    }
    radius_squared.is_finite().then(|| radius_squared.sqrt())
}

impl SphereLodState {
    fn prepare(&mut self, spheres: &[Instance3D], camera: &Camera3D, size: (u32, u32)) -> SphereLodStats3D {
        let mut stats = SphereLodStats3D {
            enabled: true,
            additional_static_mesh_bytes: self.static_bytes,
            ..Default::default()
        };
        if self.far_range.is_none() {
            stats.near_instances = spheres.len() as u32;
            return stats;
        }
        let clock = CpuClock::start();
        let old_capacities = (self.packed.capacity(), self.is_far.capacity());
        self.packed.clear();
        self.is_far.clear();
        self.packed.reserve(spheres.len());
        self.is_far.reserve(spheres.len());
        for instance in spheres {
            let radius = projected_sphere_radius(instance, camera, size);
            let far = radius.is_some_and(|r| r <= self.policy.max_projected_radius_px);
            stats.fallback_instances += u32::from(radius.is_none());
            stats.far_instances += u32::from(far);
            self.is_far.push(far);
        }
        stats.near_instances = spheres.len() as u32 - stats.far_instances;
        for far in [false, true] {
            self.packed.extend(
                spheres
                    .iter()
                    .zip(&self.is_far)
                    .filter(|(_, flag)| **flag == far)
                    .map(|(instance, _)| *instance),
            );
        }
        stats.staging_capacity = self.packed.capacity();
        stats.classification_capacity = self.is_far.capacity();
        stats.staging_reallocations = u32::from(self.packed.capacity() != old_capacities.0)
            + u32::from(self.is_far.capacity() != old_capacities.1);
        stats.cpu_classify_time = clock.elapsed();
        stats
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    light_vp: [[f32; 4]; 4],
    cam_pos: [f32; 4],
    light_dir: [f32; 4],
    light_color: [f32; 4],
    sky: [f32; 4],
    ground: [f32; 4],
    params: [f32; 4],
    shadow: [f32; 4],
    viewport: [f32; 4],
    point_position_range: [f32; 4],
    point_color_intensity: [f32; 4],
}

const MESH_ATTRS: [VertexAttr; 3] = [
    VertexAttr {
        location: 0,
        format: VertexFormat::Float32x3,
        offset: 0,
    },
    VertexAttr {
        location: 1,
        format: VertexFormat::Float32x3,
        offset: 12,
    },
    VertexAttr {
        location: 2,
        format: VertexFormat::Float32,
        offset: 24,
    },
];

const INSTANCE_ATTRS: [VertexAttr; 5] = [
    VertexAttr {
        location: 3,
        format: VertexFormat::Float32x4,
        offset: 0,
    },
    VertexAttr {
        location: 4,
        format: VertexFormat::Float32x4,
        offset: 16,
    },
    VertexAttr {
        location: 5,
        format: VertexFormat::Float32x4,
        offset: 32,
    },
    VertexAttr {
        location: 6,
        format: VertexFormat::Float32x4,
        offset: 48,
    },
    VertexAttr {
        location: 7,
        format: VertexFormat::Float32x4,
        offset: 64,
    },
];

const LINE_ATTRS: [VertexAttr; 3] = [
    VertexAttr {
        location: 0,
        format: VertexFormat::Float32x4,
        offset: 0,
    },
    VertexAttr {
        location: 1,
        format: VertexFormat::Float32x4,
        offset: 16,
    },
    VertexAttr {
        location: 2,
        format: VertexFormat::Float32x4,
        offset: 32,
    },
];

/// Size dependent textures: the multisampled color buffer and the depth buffer.
struct Attachments<B: Rhi> {
    size: (u32, u32),
    // Kept alive: the views borrow nothing but the textures own the memory.
    _color: Option<B::Texture>,
    color_view: Option<B::TextureView>,
    _depth: B::Texture,
    depth_view: B::TextureView,
}

/// The matrix of the sun's orthographic shadow camera.
///
/// The box is `2 * radius` wide and deep and `6 * radius` long (the eye sits
/// `3 * radius` from the center). Its center is snapped to whole shadow
/// texels along all three light axes.
pub fn light_view_proj(lighting: &Lighting, shadow_map_size: u32) -> Mat4 {
    let dir = normalize(lighting.direction);
    let dir = if dot(dir, dir) < 0.5 { [0.0, -1.0, 0.0] } else { dir };
    let r = lighting.shadow_radius.max(0.5);
    let helper = if dir[1].abs() > 0.95 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let s = normalize(cross(dir, helper));
    let u = cross(s, dir);
    let texel = 2.0 * r / shadow_map_size.max(1) as f32;
    let snap = |v: f32| (v / texel).round() * texel;
    let c = lighting.shadow_center;
    let center: Vec3 = {
        let (cs, cu, cf) = (snap(dot(c, s)), snap(dot(c, u)), snap(dot(c, dir)));
        [
            s[0] * cs + u[0] * cu + dir[0] * cf,
            s[1] * cs + u[1] * cu + dir[1] * cf,
            s[2] * cs + u[2] * cu + dir[2] * cf,
        ]
    };
    let eye = [
        center[0] - dir[0] * 3.0 * r,
        center[1] - dir[1] * 3.0 * r,
        center[2] - dir[2] * 3.0 * r,
    ];
    let view = Mat4::look_at(eye, center, u);
    Mat4::orthographic(-r, r, -r, r, 0.1, 6.0 * r).mul(&view)
}

/// Draws [`RenderList3D`]s. Create one per target format.
pub struct Renderer3D<B: Rhi> {
    rhi: B,
    format: TextureFormat,
    settings: Settings3D,
    samples: u32,
    /// Background color, linear RGBA.
    pub clear: [f64; 4],
    shadow_pipeline: B::Pipeline,
    main_pipeline: B::Pipeline,
    line_pipeline: B::Pipeline,
    globals: B::Buffer,
    shadow_globals: B::Buffer,
    main_bind: B::BindGroup,
    shadow_bind: B::BindGroup,
    line_bind: B::BindGroup,
    _shadow_texture: B::Texture,
    shadow_view: B::TextureView,
    _sampler: B::Sampler,
    mesh_vertices: B::Buffer,
    mesh_indices: B::Buffer,
    mesh_ranges: [(std::ops::Range<u32>, i32); 4],
    instances: Growing<B>,
    lines: Growing<B>,
    attachments: Option<Attachments<B>>,
    last_frame_stats: FrameStats,
    sphere_lod: Option<SphereLodState>,
    last_sphere_lod_stats: SphereLodStats3D,
}

impl<B: Rhi> Renderer3D<B> {
    pub fn new(rhi: B, format: TextureFormat) -> Self {
        Self::with_settings(rhi, format, Settings3D::default())
    }

    pub fn with_settings(rhi: B, format: TextureFormat, settings: Settings3D) -> Self {
        Self::build(rhi, format, settings, None)
    }

    /// Creates a renderer with optional main-pass sphere detail. Existing constructors disable LOD.
    /// Validates the policy before allocating GPU resources; equal near/far detail is one batch.
    pub fn with_sphere_lod(
        rhi: B,
        format: TextureFormat,
        settings: Settings3D,
        policy: SphereLod3D,
    ) -> Result<Self, SphereLodError3D> {
        policy.validate(settings)?;
        Ok(Self::build(rhi, format, settings, Some(policy)))
    }

    fn build(rhi: B, format: TextureFormat, settings: Settings3D, policy: Option<SphereLod3D>) -> Self {
        let samples = rhi.supported_samples(format, settings.msaa);
        let shader = rhi.create_shader("orr_render 3d", SHADER);

        let mesh_layout = VertexLayout {
            stride: std::mem::size_of::<Vertex3>() as u64,
            step: VertexStep::Vertex,
            attrs: &MESH_ATTRS,
        };
        let instance_layout = VertexLayout {
            stride: std::mem::size_of::<Instance3D>() as u64,
            step: VertexStep::Instance,
            attrs: &INSTANCE_ATTRS,
        };
        let line_layout = VertexLayout {
            stride: std::mem::size_of::<LineInstance3D>() as u64,
            step: VertexStep::Instance,
            attrs: &LINE_ATTRS,
        };

        let shadow_pipeline = rhi.create_pipeline(&PipelineDesc::<B> {
            label: "3d shadow",
            shader: &shader,
            vs_entry: "vs_shadow",
            fs_entry: "",
            vertex_buffers: &[mesh_layout, instance_layout],
            color_format: None,
            blend: Blend::Opaque,
            topology: orr_rhi::Topology::TriangleList,
            cull: Cull::Front,
            depth: Some(DepthState {
                format: DEPTH,
                write: true,
                compare: Compare::Less,
                bias: 2,
                slope_bias: 2.0,
            }),
            samples: 1,
        });
        let main_pipeline = rhi.create_pipeline(&PipelineDesc::<B> {
            label: "3d main",
            shader: &shader,
            vs_entry: "vs_main",
            fs_entry: "fs_main",
            vertex_buffers: &[mesh_layout, instance_layout],
            color_format: Some(format),
            blend: Blend::Opaque,
            topology: orr_rhi::Topology::TriangleList,
            cull: Cull::Back,
            depth: Some(DepthState::opaque(DEPTH)),
            samples,
        });
        let line_pipeline = rhi.create_pipeline(&PipelineDesc::<B> {
            label: "3d lines",
            shader: &shader,
            vs_entry: "vs_line",
            fs_entry: "fs_line",
            vertex_buffers: &[line_layout],
            color_format: Some(format),
            blend: Blend::Alpha,
            topology: orr_rhi::Topology::TriangleList,
            cull: Cull::None,
            depth: Some(DepthState {
                format: DEPTH,
                write: false,
                compare: Compare::LessEqual,
                bias: -2,
                slope_bias: -1.0,
            }),
            samples,
        });

        let uniform = |label: &str| {
            rhi.create_buffer(&BufferDesc {
                label,
                size: std::mem::size_of::<Globals>() as u64,
                usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST,
            })
        };
        let globals = uniform("3d globals");
        let shadow_globals = uniform("3d shadow globals");

        let shadow_texture = rhi.create_texture(&TextureDesc {
            label: "shadow map",
            width: settings.shadow_map_size,
            height: settings.shadow_map_size,
            format: DEPTH,
            usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::TEXTURE_BINDING,
            sample_count: 1,
            view_formats: &[],
        });
        let shadow_view = rhi.create_texture_view(&shadow_texture, None);
        let sampler = rhi.create_sampler(&SamplerDesc {
            linear: true,
            compare: true,
        });

        let main_bind = rhi.create_bind_group(
            &main_pipeline,
            0,
            &[
                Binding::Uniform {
                    binding: 0,
                    buffer: &globals,
                },
                Binding::Texture {
                    binding: 1,
                    view: &shadow_view,
                },
                Binding::Sampler {
                    binding: 2,
                    sampler: &sampler,
                },
            ],
        );
        let shadow_bind = rhi.create_bind_group(
            &shadow_pipeline,
            0,
            &[Binding::Uniform {
                binding: 0,
                buffer: &shadow_globals,
            }],
        );
        let line_bind = rhi.create_bind_group(
            &line_pipeline,
            0,
            &[Binding::Uniform {
                binding: 0,
                buffer: &globals,
            }],
        );

        let mut meshes = MeshSet::build_with(settings.mesh_segments);
        let sphere_lod = policy.map(|policy| {
            let original_vertex_count = meshes.vertices.len();
            let original_index_count = meshes.indices.len();
            let far_range = if policy.far_segments < settings.mesh_segments.clamp(3, 64) {
                let far = crate::mesh::sphere_with(policy.far_segments);
                let base = meshes.vertices.len() as u32;
                let first = meshes.indices.len() as u32;
                meshes.vertices.extend_from_slice(&far.vertices);
                // WebGL2 has no base-vertex indexed instancing. Resolve the new
                // far mesh's indices at construction, so its draw uses base 0.
                meshes.indices.extend(far.indices.iter().map(|index| base + index));
                Some((first..meshes.indices.len() as u32, 0))
            } else {
                None
            };
            let static_bytes = ((meshes.vertices.len() - original_vertex_count) * std::mem::size_of::<Vertex3>()
                + (meshes.indices.len() - original_index_count) * 4) as u64;
            SphereLodState {
                policy,
                far_range,
                static_bytes,
                packed: Vec::new(),
                is_far: Vec::new(),
            }
        });
        let mesh_vertices = rhi.create_buffer(&BufferDesc {
            label: "mesh vertices",
            size: (meshes.vertices.len() * std::mem::size_of::<Vertex3>()) as u64,
            usage: BufferUsage::VERTEX | BufferUsage::COPY_DST,
        });
        rhi.write_buffer(&mesh_vertices, 0, bytemuck::cast_slice(&meshes.vertices));
        let mesh_indices = rhi.create_buffer(&BufferDesc {
            label: "mesh indices",
            size: (meshes.indices.len() * 4) as u64,
            usage: BufferUsage::INDEX | BufferUsage::COPY_DST,
        });
        rhi.write_buffer(&mesh_indices, 0, bytemuck::cast_slice(&meshes.indices));

        let instances = Growing::new(&rhi, "3d instances", std::mem::size_of::<Instance3D>());
        let lines = Growing::new(&rhi, "3d lines", std::mem::size_of::<LineInstance3D>());
        Self {
            rhi,
            format,
            settings,
            samples,
            clear: DEFAULT_CLEAR_3D,
            shadow_pipeline,
            main_pipeline,
            line_pipeline,
            globals,
            shadow_globals,
            main_bind,
            shadow_bind,
            line_bind,
            _shadow_texture: shadow_texture,
            shadow_view,
            _sampler: sampler,
            mesh_vertices,
            mesh_indices,
            mesh_ranges: meshes.ranges,
            instances,
            lines,
            attachments: None,
            last_frame_stats: FrameStats::default(),
            last_sphere_lod_stats: SphereLodStats3D {
                enabled: sphere_lod.is_some(),
                additional_static_mesh_bytes: sphere_lod.as_ref().map_or(0, |s| s.static_bytes),
                ..Default::default()
            },
            sphere_lod,
        }
    }

    pub fn format(&self) -> TextureFormat {
        self.format
    }

    pub fn rhi(&self) -> &B {
        &self.rhi
    }

    /// The MSAA sample count in use (the request, or less when unsupported).
    pub fn samples(&self) -> u32 {
        self.samples
    }

    pub fn settings(&self) -> Settings3D {
        self.settings
    }

    /// Work submitted by the last completed draw, including shadow-map clear passes.
    pub fn last_frame_stats(&self) -> FrameStats {
        self.last_frame_stats
    }

    pub fn last_sphere_lod_stats(&self) -> SphereLodStats3D {
        self.last_sphere_lod_stats
    }

    fn ensure_attachments(&mut self, size: (u32, u32)) -> u32 {
        if self.attachments.as_ref().is_some_and(|a| a.size == size) {
            return 0;
        }
        let rhi = &self.rhi;
        let (color, color_view) = if self.samples > 1 {
            let t = rhi.create_texture(&TextureDesc {
                label: "3d msaa color",
                width: size.0,
                height: size.1,
                format: self.format,
                usage: TextureUsage::RENDER_ATTACHMENT,
                sample_count: self.samples,
                view_formats: &[],
            });
            let v = rhi.create_texture_view(&t, None);
            (Some(t), Some(v))
        } else {
            (None, None)
        };
        let depth = rhi.create_texture(&TextureDesc {
            label: "3d depth",
            width: size.0,
            height: size.1,
            format: DEPTH,
            usage: TextureUsage::RENDER_ATTACHMENT,
            sample_count: self.samples,
            view_formats: &[],
        });
        let depth_view = rhi.create_texture_view(&depth, None);
        self.attachments = Some(Attachments {
            size,
            _color: color,
            color_view,
            _depth: depth,
            depth_view,
        });
        1 + u32::from(self.samples > 1)
    }

    fn globals(&self, camera: &Camera3D, size: (u32, u32), lighting: &Lighting, light_vp: &Mat4) -> Globals {
        let (w, h) = (size.0.max(1) as f32, size.1.max(1) as f32);
        let to_light = normalize(scale(lighting.direction, -1.0));
        let texel_world = 2.0 * lighting.shadow_radius.max(0.5) / self.settings.shadow_map_size.max(1) as f32;
        Globals {
            view_proj: camera.view_proj(w / h).0,
            light_vp: light_vp.0,
            cam_pos: [camera.eye[0], camera.eye[1], camera.eye[2], 1.0],
            light_dir: [to_light[0], to_light[1], to_light[2], lighting.intensity],
            light_color: [lighting.color[0], lighting.color[1], lighting.color[2], 1.0],
            sky: [lighting.sky[0], lighting.sky[1], lighting.sky[2], lighting.ambient],
            ground: [lighting.ground[0], lighting.ground[1], lighting.ground[2], 1.0],
            params: [
                if lighting.shadows { 1.0 } else { 0.0 },
                if lighting.tonemap { 1.0 } else { 0.0 },
                if self.format.is_srgb() { 0.0 } else { 1.0 },
                lighting.exposure,
            ],
            shadow: [
                1.0 / self.settings.shadow_map_size.max(1) as f32,
                texel_world * 1.5,
                0.0004,
                0.0,
            ],
            viewport: [w, h, 0.0, 0.0],
            point_position_range: [0.0; 4],
            point_color_intensity: [0.0; 4],
        }
    }

    /// The clear color as the target stores it: a non sRGB target is written
    /// without hardware encoding, so the value is encoded here too.
    fn clear_value(&self) -> [f64; 4] {
        if self.format.is_srgb() {
            return self.clear;
        }
        let enc = |c: f64| {
            if c <= 0.003_130_8 {
                c * 12.92
            } else {
                1.055 * c.powf(1.0 / 2.4) - 0.055
            }
        };
        [
            enc(self.clear[0]),
            enc(self.clear[1]),
            enc(self.clear[2]),
            self.clear[3],
        ]
    }

    /// Draws `list` into `view` (`size` pixels, multisampled internally and
    /// resolved into `view`) seen through `camera`, and submits it.
    pub fn draw(&mut self, view: &B::TextureView, size: (u32, u32), list: &RenderList3D, camera: &Camera3D) {
        let prepare_clock = CpuClock::start();
        let replacing_attachments = self.attachments.is_some();
        let mut stats = FrameStats {
            mesh_instances: list.instance_count() as u64,
            line_instances: list.lines.len() as u64,
            msaa_samples: self.samples,
            attachment_allocations: self.ensure_attachments(size),
            ..FrameStats::default()
        };
        if replacing_attachments {
            stats.attachment_reallocations = stats.attachment_allocations;
        }
        let light_vp = light_view_proj(&list.lighting, self.settings.shadow_map_size);
        let globals = self.globals(camera, size, &list.lighting, &light_vp);
        let shadow_globals = Globals {
            view_proj: light_vp.0,
            ..globals
        };
        let rhi = &self.rhi;
        stats.upload(rhi, &self.globals, 0, bytemuck::bytes_of(&globals));
        stats.upload(rhi, &self.shadow_globals, 0, bytemuck::bytes_of(&shadow_globals));

        let mut lod_stats = self.sphere_lod.as_mut().map_or_else(
            || SphereLodStats3D {
                near_instances: list.spheres.len() as u32,
                ..Default::default()
            },
            |lod| lod.prepare(&list.spheres, camera, size),
        );

        // One instance buffer, the kinds back to back in `MeshKind` order.
        let total = list.instance_count();
        let mut ranges = [0u32..0u32, 0..0, 0..0, 0..0];
        if total > 0 {
            stats.buffer_reallocations += u32::from(self.instances.reserve(rhi, total));
            let item = std::mem::size_of::<Instance3D>() as u64;
            let mut first = 0u32;
            for kind in MeshKind::ALL {
                let part = if kind == MeshKind::Sphere {
                    self.sphere_lod
                        .as_ref()
                        .filter(|lod| lod.far_range.is_some())
                        .map_or(list.instances(kind), |lod| lod.packed.as_slice())
                } else {
                    list.instances(kind)
                };
                if !part.is_empty() {
                    stats.upload(
                        rhi,
                        &self.instances.buffer,
                        u64::from(first) * item,
                        bytemuck::cast_slice(part),
                    );
                    if kind == MeshKind::Sphere {
                        lod_stats.upload_calls = 1;
                        lod_stats.upload_bytes = part.len() as u64 * item;
                    }
                }
                ranges[kind as usize] = first..first + part.len() as u32;
                first += part.len() as u32;
            }
        }
        if !list.lines.is_empty() {
            stats.buffer_reallocations += u32::from(self.lines.reserve(rhi, list.lines.len()));
            stats.upload(rhi, &self.lines.buffer, 0, bytemuck::cast_slice(&list.lines));
        }

        stats.cpu_prepare_time = prepare_clock.elapsed();
        let encode_clock = CpuClock::start();
        let draw_meshes = |commands: &mut Vec<Command<B>>, with_planes: bool, pass: &mut PassStats| {
            for kind in MeshKind::ALL {
                if kind == MeshKind::Plane && !with_planes {
                    continue;
                }
                let instances = ranges[kind as usize].clone();
                if instances.is_empty() {
                    continue;
                }
                if kind == MeshKind::Sphere && with_planes {
                    let near_end = instances.start + lod_stats.near_instances;
                    if near_end > instances.start {
                        let (indices, base_vertex) = self.mesh_ranges[kind as usize].clone();
                        pass.draw(lod_stats.near_instances);
                        commands.push(Command::DrawIndexed {
                            indices,
                            base_vertex,
                            instances: instances.start..near_end,
                        });
                    }
                    if lod_stats.far_instances > 0 {
                        let (indices, base_vertex) = self
                            .sphere_lod
                            .as_ref()
                            .and_then(|lod| lod.far_range.clone())
                            .expect("far instances have a far mesh");
                        pass.draw(lod_stats.far_instances);
                        commands.push(Command::DrawIndexed {
                            indices,
                            base_vertex,
                            instances: near_end..instances.end,
                        });
                    }
                } else {
                    let (indices, base_vertex) = self.mesh_ranges[kind as usize].clone();
                    pass.draw(instances.end - instances.start);
                    commands.push(Command::DrawIndexed {
                        indices,
                        base_vertex,
                        instances,
                    });
                }
            }
        };

        let mut encoder = rhi.create_encoder("3d frame");
        if list.lighting.shadows && total > 0 {
            let mut commands: Vec<Command<B>> = vec![
                Command::SetPipeline(&self.shadow_pipeline),
                Command::SetBindGroup(0, &self.shadow_bind),
                Command::SetVertexBuffer(0, &self.mesh_vertices),
                Command::SetVertexBuffer(1, &self.instances.buffer),
                Command::SetIndexBuffer(&self.mesh_indices),
            ];
            draw_meshes(&mut commands, false, &mut stats.shadow);
            rhi.encode_pass(
                &mut encoder,
                "3d shadow",
                None,
                Some(&DepthAttachment {
                    view: &self.shadow_view,
                    clear: Some(1.0),
                    store: true,
                }),
                &commands,
            );
        } else {
            // Nothing was drawn into the map: clear it so stale shadows never show.
            rhi.encode_pass(
                &mut encoder,
                "3d shadow clear",
                None,
                Some(&DepthAttachment {
                    view: &self.shadow_view,
                    clear: Some(1.0),
                    store: true,
                }),
                &[],
            );
        }

        stats.shadow.passes += 1;

        let mut commands: Vec<Command<B>> = Vec::with_capacity(24);
        if total > 0 {
            commands.push(Command::SetPipeline(&self.main_pipeline));
            commands.push(Command::SetBindGroup(0, &self.main_bind));
            commands.push(Command::SetVertexBuffer(0, &self.mesh_vertices));
            commands.push(Command::SetVertexBuffer(1, &self.instances.buffer));
            commands.push(Command::SetIndexBuffer(&self.mesh_indices));
            draw_meshes(&mut commands, true, &mut stats.main);
        }
        if !list.lines.is_empty() {
            commands.push(Command::SetPipeline(&self.line_pipeline));
            commands.push(Command::SetBindGroup(0, &self.line_bind));
            commands.push(Command::SetVertexBuffer(0, &self.lines.buffer));
            commands.push(Command::Draw {
                vertices: 0..6,
                instances: 0..list.lines.len() as u32,
            });
            stats.main.draw(list.lines.len() as u32);
        }
        let att = self.attachments.as_ref().expect("attachments created above");
        let color = match &att.color_view {
            Some(msaa) => ColorAttachment {
                view: msaa,
                clear: Some(self.clear_value()),
                resolve: Some(view),
            },
            None => ColorAttachment {
                view,
                clear: Some(self.clear_value()),
                resolve: None,
            },
        };
        rhi.encode_pass(
            &mut encoder,
            "3d main",
            Some(&color),
            Some(&DepthAttachment {
                view: &att.depth_view,
                clear: Some(1.0),
                store: false,
            }),
            &commands,
        );
        stats.main.passes += 1;
        stats.cpu_encode_time = encode_clock.elapsed();
        let submit_clock = CpuClock::start();
        rhi.submit(encoder);
        stats.cpu_submit_time = submit_clock.elapsed();
        let near_indices = u64::from(self.mesh_ranges[MeshKind::Sphere as usize].0.len() as u32);
        let far_indices = self
            .sphere_lod
            .as_ref()
            .and_then(|lod| lod.far_range.as_ref())
            .map_or(near_indices, |(range, _)| range.len() as u64);
        lod_stats.main_draw_calls = u32::from(lod_stats.near_instances > 0) + u32::from(lod_stats.far_instances > 0);
        lod_stats.shadow_draw_calls = u32::from(list.lighting.shadows && !list.spheres.is_empty());
        lod_stats.main_index_invocations =
            u64::from(lod_stats.near_instances) * near_indices + u64::from(lod_stats.far_instances) * far_indices;
        lod_stats.shadow_index_invocations = if list.lighting.shadows {
            list.spheres.len() as u64 * near_indices
        } else {
            0
        };
        self.last_sphere_lod_stats = lod_stats;
        self.last_frame_stats = stats;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globals_layout_includes_point_light_uniforms() {
        // Match shader3d.wgsl: retain the 256-byte prefix, then two vec4s.
        // gpu3d upload budgets count two complete uniforms on every draw.
        assert_eq!(std::mem::offset_of!(Globals, viewport), 240);
        assert_eq!(std::mem::offset_of!(Globals, point_position_range), 256);
        assert_eq!(std::mem::offset_of!(Globals, point_color_intensity), 272);
        assert_eq!(std::mem::size_of::<Globals>(), 288);
    }

    #[test]
    fn light_matrix_maps_the_center_to_the_middle_of_the_box() {
        let l = Lighting {
            shadow_center: [3.0, 0.5, -2.0],
            shadow_radius: 20.0,
            ..Lighting::default()
        };
        let m = light_view_proj(&l, 2048);
        let c = m.transform_point4(l.shadow_center);
        // Snapping moves the center by less than one texel (2 * 20 / 2048 world units, 1 / 1024 clip).
        assert!(c[0].abs() < 2.0 / 1024.0 && c[1].abs() < 2.0 / 1024.0, "{c:?}");
        assert!(c[2] > 0.0 && c[2] < 1.0, "{c:?}");
    }

    #[test]
    fn light_matrix_is_stable_while_the_center_moves_less_than_a_texel() {
        let a = Lighting {
            shadow_center: [0.0, 0.0, 0.0],
            shadow_radius: 20.0,
            ..Lighting::default()
        };
        let b = Lighting {
            shadow_center: [0.003, 0.0, 0.0],
            ..a
        };
        assert_eq!(light_view_proj(&a, 2048), light_view_proj(&b, 2048));
    }

    #[test]
    fn straight_down_light_does_not_break_the_basis() {
        let l = Lighting {
            direction: [0.0, -1.0, 0.0],
            ..Lighting::default()
        };
        let m = light_view_proj(&l, 1024);
        assert!(m.0.iter().flatten().all(|v| v.is_finite()));
    }

    fn lod_sphere(radius: f32, marker: f32) -> Instance3D {
        let mut list = RenderList3D::new();
        list.sphere(
            [0.0, 0.0, 0.0],
            crate::IDENTITY_ROT,
            radius,
            &crate::Material::new([marker, 0.2, 0.3]),
        );
        list.spheres[0]
    }

    fn lod_state(policy: SphereLod3D, equal_detail: bool) -> SphereLodState {
        SphereLodState {
            policy,
            far_range: (!equal_detail).then_some((100..244, 20)),
            static_bytes: if equal_detail { 0 } else { 2000 },
            packed: Vec::new(),
            is_far: Vec::new(),
        }
    }

    #[test]
    fn sphere_lod_policy_rejects_invalid_cutoffs_and_detail_before_construction() {
        for cutoff in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(
                SphereLod3D {
                    max_projected_radius_px: cutoff,
                    ..Default::default()
                }
                .validate(Settings3D::default()),
                Err(SphereLodError3D::ProjectedRadius)
            );
        }
        for far_segments in [0, 2, 33, 65] {
            assert_eq!(
                SphereLod3D {
                    far_segments,
                    ..Default::default()
                }
                .validate(Settings3D::default()),
                Err(SphereLodError3D::FarSegments)
            );
        }
        assert!(SphereLod3D::LOW.validate(Settings3D::LOW).is_ok());
        assert!(SphereLod3D {
            far_segments: 3,
            ..Default::default()
        }
        .validate(Settings3D {
            mesh_segments: 0,
            ..Settings3D::LOW
        })
        .is_ok());
        assert!(SphereLod3D {
            far_segments: 64,
            ..Default::default()
        }
        .validate(Settings3D {
            mesh_segments: 100,
            ..Settings3D::default()
        })
        .is_ok());
    }

    #[test]
    fn sphere_lod_cutoff_equality_is_far_and_resize_reclassifies() {
        let instance = lod_sphere(0.05, 0.1);
        let camera = Camera3D::orthographic([0.0, 0.0, 10.0], [0.0; 3], 2.0);
        let radius = projected_sphere_radius(&instance, &camera, (200, 200)).unwrap();
        let mut state = lod_state(
            SphereLod3D {
                max_projected_radius_px: radius,
                ..Default::default()
            },
            false,
        );
        let at = state.prepare(&[instance], &camera, (200, 200));
        assert_eq!((at.near_instances, at.far_instances, at.fallback_instances), (0, 1, 0));
        let larger = state.prepare(&[instance], &camera, (400, 400));
        assert_eq!((larger.near_instances, larger.far_instances), (1, 0));
        let smaller = state.prepare(&[instance], &camera, (100, 100));
        assert_eq!((smaller.near_instances, smaller.far_instances), (0, 1));
    }

    #[test]
    fn sphere_lod_perspective_uses_current_camera_and_near_plane() {
        let instance = lod_sphere(0.1, 0.1);
        let far = Camera3D::perspective([0.0, 0.0, 10.0], [0.0; 3], 60.0);
        let close = Camera3D::perspective([0.0, 0.0, 1.0], [0.0; 3], 60.0);
        let mut state = lod_state(SphereLod3D::default(), false);
        assert_eq!(state.prepare(&[instance], &far, (200, 200)).far_instances, 1);
        assert_eq!(state.prepare(&[instance], &close, (200, 200)).near_instances, 1);
        let touching = Camera3D {
            projection: Projection::Perspective {
                fov_y: 1.0,
                near: 0.9,
                far: 10.0,
            },
            ..close
        };
        assert!(projected_sphere_radius(&instance, &touching, (200, 200)).is_none());
        let behind = Instance3D {
            pos: [0.0, 0.0, 11.0],
            ..instance
        };
        assert!(projected_sphere_radius(&behind, &far, (200, 200)).is_none());
    }

    #[test]
    fn sphere_lod_invalid_camera_viewport_and_transform_keep_original_detail() {
        let instance = lod_sphere(0.01, 0.1);
        let camera = Camera3D::perspective([0.0, 0.0, 10.0], [0.0; 3], 60.0);
        for size in [(0, 200), (200, 0)] {
            assert!(projected_sphere_radius(&instance, &camera, size).is_none());
        }
        for invalid in [
            Camera3D {
                eye: [f32::NAN, 0.0, 10.0],
                ..camera
            },
            Camera3D {
                target: camera.eye,
                ..camera
            },
            Camera3D {
                up: [0.0, 0.0, 1.0],
                ..camera
            },
            Camera3D {
                projection: Projection::Perspective {
                    fov_y: 0.0,
                    near: 0.1,
                    far: 500.0,
                },
                ..camera
            },
            Camera3D {
                projection: Projection::Perspective {
                    fov_y: 1.0,
                    near: -1.0,
                    far: 500.0,
                },
                ..camera
            },
            Camera3D {
                projection: Projection::Orthographic {
                    half_height: f32::INFINITY,
                    near: -1.0,
                    far: 1.0,
                },
                ..camera
            },
            Camera3D {
                projection: Projection::Orthographic {
                    half_height: 1.0,
                    near: 1.0,
                    far: 1.0,
                },
                ..camera
            },
        ] {
            assert!(
                projected_sphere_radius(&instance, &invalid, (200, 200)).is_none(),
                "{invalid:?}"
            );
        }
        for invalid in [
            Instance3D {
                scale: [-1.0, 0.1, 0.1],
                ..instance
            },
            Instance3D {
                scale: [0.0, 0.1, 0.1],
                ..instance
            },
            Instance3D {
                scale: [f32::INFINITY; 3],
                ..instance
            },
            Instance3D {
                pos: [f32::MAX; 3],
                ..instance
            },
            Instance3D {
                rot: [0.0; 4],
                ..instance
            },
            Instance3D {
                color: [f32::NAN; 4],
                ..instance
            },
        ] {
            let stats = lod_state(SphereLod3D::default(), false).prepare(&[invalid], &camera, (200, 200));
            assert_eq!(
                (stats.near_instances, stats.far_instances, stats.fallback_instances),
                (1, 0, 1)
            );
        }
    }

    #[test]
    fn sphere_lod_bound_encloses_rotated_nonuniform_actual_mesh() {
        let camera = Camera3D::perspective([4.0, 3.0, 10.0], [1.0, 0.0, 0.0], 60.0);
        let instance = Instance3D {
            pos: [1.0, 0.0, 0.0],
            scale: [0.3, 0.06, 0.1],
            rot: [0.0, 0.0, (0.7f32 / 2.0).sin(), (0.7f32 / 2.0).cos()],
            ..lod_sphere(0.1, 0.1)
        };
        let size = (500, 300);
        let bound = projected_sphere_radius(&instance, &camera, size).unwrap();
        let center = camera.world_to_screen(instance.pos, size).unwrap();
        for vertex in crate::mesh::sphere().vertices {
            let scaled = std::array::from_fn(|i| vertex.pos[i] * instance.scale[i]);
            let offset = quat_rotate(instance.rot, scaled);
            let world = std::array::from_fn(|i| instance.pos[i] + offset[i]);
            let screen = camera.world_to_screen(world, size).unwrap();
            let distance = ((screen[0] - center[0]).powi(2) + (screen[1] - center[1]).powi(2)).sqrt();
            assert!(distance <= bound, "actual vertex distance {distance} > bound {bound}");
        }
    }

    #[test]
    fn sphere_lod_partition_preserves_every_byte_and_order_and_reuses_capacity() {
        let camera = Camera3D::orthographic([0.0, 0.0, 10.0], [0.0; 3], 2.0);
        let source = [
            lod_sphere(0.01, 1.0),
            lod_sphere(0.5, 2.0),
            lod_sphere(0.02, 3.0),
            lod_sphere(0.8, 4.0),
        ];
        let original = bytemuck::cast_slice::<Instance3D, u8>(&source).to_vec();
        let mut state = lod_state(SphereLod3D::default(), false);
        let first = state.prepare(&source, &camera, (200, 200));
        assert_eq!((first.near_instances, first.far_instances), (2, 2));
        assert_eq!(
            state.packed.iter().map(|i| i.color[0]).collect::<Vec<_>>(),
            [2.0, 4.0, 1.0, 3.0]
        );
        let expected = [source[1], source[3], source[0], source[2]];
        assert_eq!(
            bytemuck::cast_slice::<Instance3D, u8>(&state.packed),
            bytemuck::cast_slice::<Instance3D, u8>(&expected)
        );
        assert_eq!(bytemuck::cast_slice::<Instance3D, u8>(&source), original);
        let all_near = state.prepare(&source, &camera, (10000, 10000));
        assert_eq!(
            (
                all_near.near_instances,
                all_near.far_instances,
                all_near.staging_reallocations
            ),
            (4, 0, 0)
        );
        assert_eq!(
            (all_near.staging_capacity, all_near.classification_capacity),
            (first.staging_capacity, first.classification_capacity)
        );
        let empty = state.prepare(&[], &camera, (200, 200));
        assert_eq!(
            (empty.near_instances, empty.far_instances, empty.staging_reallocations),
            (0, 0, 0)
        );
        assert!(state.packed.is_empty());
        assert_eq!(empty.staging_capacity, first.staging_capacity);
    }

    #[test]
    fn sphere_lod_equal_detail_bypasses_classification_and_extra_storage() {
        let spheres = [lod_sphere(0.01, 0.2), lod_sphere(0.5, 0.3)];
        let camera = Camera3D::perspective([0.0; 3], [0.0; 3], 60.0);
        let mut state = lod_state(
            SphereLod3D {
                far_segments: 12,
                ..SphereLod3D::LOW
            },
            true,
        );
        let stats = state.prepare(&spheres, &camera, (0, 0));
        assert_eq!(
            (
                stats.near_instances,
                stats.far_instances,
                stats.fallback_instances,
                stats.additional_static_mesh_bytes
            ),
            (2, 0, 0, 0)
        );
        assert_eq!(
            (
                stats.staging_capacity,
                stats.classification_capacity,
                stats.staging_reallocations
            ),
            (0, 0, 0)
        );
        assert!(stats.cpu_classify_time.is_none());
    }
}
