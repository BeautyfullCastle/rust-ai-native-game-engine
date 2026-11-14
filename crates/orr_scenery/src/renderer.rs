//! GPU side of the outdoor scene: resources, culling and the frame.

use crate::math::{add, cross, dot, frustum_planes, length, lerp3, look_at, mul, normalize, ortho, scale, smoothstep, sphere_visible, sub, transform, Mat4, V3};
use crate::mesh::{self, Mesh, PropVertex, TerrainVertex, WaterVertex, TREE_LEVELS};
use crate::world::{CoverChunk, Placed, World, BROADLEAF_VARIANTS, CHUNK, CONIFER_VARIANTS, COVER_VARIANTS, ROCK_VARIANTS};
use crate::settings::{Quality, Settings};
use crate::{Camera, Environment};
use bytemuck::{Pod, Zeroable};
use orr_rhi::{
    Binding, Blend, BufferDesc, BufferUsage, ColorAttachment, Command, Compare, Cull, DepthAttachment, DepthState, PipelineDesc, Rhi,
    SamplerDesc, TextureDesc, TextureFormat, TextureUsage, Topology, VertexAttr, VertexFormat, VertexLayout, VertexStep,
};

const HDR: TextureFormat = TextureFormat::Rgba16Float;
const DEPTH: TextureFormat = TextureFormat::Depth32Float;
/// Distance from the light plane to the middle of each shadow volume.
const SHADOW_DEPTH: [f32; 2] = [260.0, 1100.0];
/// Trees closer than the first distance use the detailed mesh, closer than the
/// second the middle one, and the simplest one beyond. Settings scale these.
const LOD_DISTANCE: [f32; 2] = [95.0, 260.0];
/// In the wide shadow map, trees beyond this distance use the simplest mesh.
const SHADOW_LOD_DISTANCE: f32 = 150.0;
const BLOOM_LEVELS: usize = 5;

const COMMON: &str = include_str!("shaders/common.wgsl");
const LIGHTING: &str = include_str!("shaders/lighting.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: Mat4,
    shadow0: Mat4,
    shadow1: Mat4,
    cam_right: [f32; 4],
    cam_up: [f32; 4],
    cam_forward: [f32; 4],
    camera: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    ambient_sky: [f32; 4],
    ambient_ground: [f32; 4],
    fog: [f32; 4],
    viewport: [f32; 4],
    cascades: [f32; 4],
    sky: [f32; 4],
    shadow: [f32; 4],
    wind: [f32; 4],
    water_color: [f32; 4],
    water: [f32; 4],
    leaf_tint: [f32; 4],
    detail: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PostUniform {
    sun: [f32; 4],
    grade: [f32; 4],
    sun_color: [f32; 4],
    tone: [f32; 4],
    meter: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct InstanceRaw {
    place: [f32; 4],
    look: [f32; 4],
}

impl InstanceRaw {
    fn new(p: &Placed, flag: f32) -> Self {
        Self { place: [p.position[0], p.position[1], p.position[2], p.scale], look: [p.yaw, p.tint[0], p.tint[1], flag] }
    }
}

#[derive(Clone, Copy)]
struct MeshRange {
    first_index: u32,
    index_count: u32,
    base_vertex: i32,
    radius: f32,
}

/// A tree or rock that is culled and sorted into draws every frame.
#[derive(Clone, Copy)]
struct Object {
    raw: InstanceRaw,
    /// Mesh per level of detail, closest first.
    meshes: [u32; TREE_LEVELS],
    radius: f32,
    /// Small rocks do not draw into the wide shadow map.
    wide_shadow: bool,
}

#[derive(Clone, Copy)]
struct Draw {
    mesh: u32,
    first: u32,
    count: u32,
}

struct Target<B: Rhi> {
    _texture: B::Texture,
    view: B::TextureView,
}

struct Targets<B: Rhi> {
    size: (u32, u32),
    meter: Target<B>,
    adapted: [Target<B>; 2],
    meter_group: B::BindGroup,
    adapt_groups: [B::BindGroup; 2],
    multisampled: Option<Target<B>>,
    depth: Target<B>,
    scene: Target<B>,
    bloom_down: Vec<Target<B>>,
    bloom_up: Vec<Target<B>>,
    shafts: Target<B>,
    prefilter_group: B::BindGroup,
    down_groups: Vec<B::BindGroup>,
    up_groups: Vec<B::BindGroup>,
    shafts_group: B::BindGroup,
    /// One per adapted-brightness target: the frame reads the one it just wrote.
    final_groups: [B::BindGroup; 2],
}

struct Cascade {
    matrix: Mat4,
    right: V3,
    up: V3,
    center: V3,
    radius: f32,
}

impl Cascade {
    fn new(focus: V3, sun: V3, radius: f32, depth: f32, size: u32) -> Self {
        let helper = if sun[1].abs() > 0.95 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
        let right = normalize(cross(helper, sun));
        let up = cross(sun, right);
        // Moving in whole texels keeps shadow edges still while the camera moves.
        let texel = 2.0 * radius / size as f32;
        let snap = |v: f32| (v / texel).floor() * texel;
        let center = add(add(scale(right, snap(dot(focus, right))), scale(up, snap(dot(focus, up)))), scale(sun, dot(focus, sun)));
        let view = look_at(add(center, scale(sun, depth)), center, up);
        Self { matrix: mul(&ortho(radius, radius, 0.0, 2.0 * depth), &view), right, up, center, radius }
    }

    /// True when a sphere can cast into this shadow map. Depth is not tested:
    /// casters toward the sun are flattened onto the near plane.
    fn covers(&self, center: V3, radius: f32) -> bool {
        let d = sub(center, self.center);
        dot(d, self.right).abs() < self.radius + radius && dot(d, self.up).abs() < self.radius + radius
    }
}

/// Work done by the last frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
    pub triangles: u64,
    pub instances: u32,
    pub draws: u32,
}

pub struct Renderer<B: Rhi> {
    rhi: B,
    target_format: TextureFormat,
    quality: Quality,
    samples: u32,
    shadow_size: [u32; 2],

    terrain_vertices: B::Buffer,
    terrain_indices: B::Buffer,
    terrain_index_count: u32,
    far_vertices: B::Buffer,
    far_indices: B::Buffer,
    far_index_count: u32,
    water_vertices: B::Buffer,
    water_indices: B::Buffer,
    water_index_count: u32,
    prop_vertices: B::Buffer,
    prop_indices: B::Buffer,
    meshes: Vec<MeshRange>,
    cover_mesh_base: u32,
    objects: Vec<Object>,
    object_instances: B::Buffer,
    cover_instances: B::Buffer,
    cover_chunks: Vec<CoverChunk>,

    globals: [B::Buffer; 3],
    post_uniform: B::Buffer,
    shadow_maps: [Target<B>; 2],
    linear_sampler: B::Sampler,

    terrain_pipeline: B::Pipeline,
    terrain_group: B::BindGroup,
    terrain_shadow_pipeline: B::Pipeline,
    terrain_shadow_groups: [B::BindGroup; 2],
    prop_pipeline: B::Pipeline,
    prop_group: B::BindGroup,
    prop_shadow_pipeline: B::Pipeline,
    prop_shadow_groups: [B::BindGroup; 2],
    sky_pipeline: B::Pipeline,
    sky_group: B::BindGroup,
    water_pipeline: B::Pipeline,
    water_group: B::BindGroup,
    prefilter_pipeline: B::Pipeline,
    down_pipeline: B::Pipeline,
    up_pipeline: B::Pipeline,
    shafts_pipeline: B::Pipeline,
    meter_pipeline: B::Pipeline,
    adapt_pipeline: B::Pipeline,
    final_pipeline: B::Pipeline,

    targets: Option<Targets<B>>,
    /// Frames drawn into the current targets.
    frame: u64,
    buckets: Vec<Vec<InstanceRaw>>,
    staged: Vec<InstanceRaw>,
    stats: FrameStats,
}

fn buffer_with<B: Rhi>(rhi: &B, label: &str, usage: BufferUsage, data: &[u8]) -> B::Buffer {
    let buffer = rhi.create_buffer(&BufferDesc { label, size: (data.len() as u64).max(16), usage: usage | BufferUsage::COPY_DST });
    if !data.is_empty() {
        rhi.write_buffer(&buffer, 0, data);
    }
    buffer
}

fn target<B: Rhi>(rhi: &B, label: &str, size: (u32, u32), format: TextureFormat, usage: TextureUsage, samples: u32) -> Target<B> {
    let texture = rhi.create_texture(&TextureDesc { label, width: size.0, height: size.1, format, usage, sample_count: samples, view_formats: &[] });
    let view = rhi.create_texture_view(&texture, None);
    Target { _texture: texture, view }
}

fn sampled_view<B: Rhi>(binding: u32, target: &Target<B>) -> Binding<'_, B> {
    Binding::Texture { binding, view: &target.view }
}

const fn attr(location: u32, format: VertexFormat, offset: u64) -> VertexAttr {
    VertexAttr { location, format, offset }
}

const TERRAIN_ATTRS: [VertexAttr; 3] = [attr(0, VertexFormat::Float32x3, 0), attr(1, VertexFormat::Float32x3, 12), attr(2, VertexFormat::Float32x4, 24)];
const WATER_ATTRS: [VertexAttr; 3] = [attr(0, VertexFormat::Float32x3, 0), attr(1, VertexFormat::Float32x4, 12), attr(2, VertexFormat::Float32x2, 28)];
const PROP_ATTRS: [VertexAttr; 4] =
    [attr(0, VertexFormat::Float32x3, 0), attr(1, VertexFormat::Float32x3, 12), attr(2, VertexFormat::Float32x3, 24), attr(3, VertexFormat::Float32x3, 36)];
const INSTANCE_ATTRS: [VertexAttr; 2] = [attr(4, VertexFormat::Float32x4, 0), attr(5, VertexFormat::Float32x4, 16)];

impl<B: Rhi> Renderer<B> {
    /// Uploads the world. `target_format` is the format of the view that
    /// [`Renderer::render`] draws into.
    pub fn new(rhi: B, target_format: TextureFormat, world: &World, quality: Quality) -> Self {
        let quality = quality.sanitized();
        let samples = rhi.supported_samples(HDR, quality.antialiasing);
        let shadow_size = [quality.shadow_resolution / 2, quality.shadow_resolution];

        let (terrain, terrain_index) = mesh::terrain(world);
        let (far, far_index) = mesh::far_terrain(world);
        let (water, water_index) = mesh::water(world);

        // Mesh order: conifers by level of detail, broadleaves by level of
        // detail, rocks, ground cover.
        let mut prop_meshes: Vec<Mesh> = Vec::new();
        for level in 0..TREE_LEVELS {
            for variant in 0..CONIFER_VARIANTS {
                prop_meshes.push(mesh::conifer(level, 100 + variant as u64));
            }
        }
        for level in 0..TREE_LEVELS {
            for variant in 0..BROADLEAF_VARIANTS {
                prop_meshes.push(mesh::broadleaf(level, 200 + variant as u64));
            }
        }
        for variant in 0..ROCK_VARIANTS {
            prop_meshes.push(mesh::rock(300 + variant as u64));
        }
        let cover_mesh_base = prop_meshes.len() as u32;
        prop_meshes.push(mesh::grass_tuft(false, 400));
        prop_meshes.push(mesh::grass_tuft(true, 401));
        prop_meshes.push(mesh::fern(402));
        debug_assert_eq!(prop_meshes.len(), cover_mesh_base as usize + COVER_VARIANTS);

        let mut prop_vertices: Vec<PropVertex> = Vec::new();
        let mut prop_indices: Vec<u32> = Vec::new();
        let mut meshes = Vec::new();
        for m in &prop_meshes {
            meshes.push(MeshRange {
                first_index: prop_indices.len() as u32,
                index_count: m.indices.len() as u32,
                base_vertex: prop_vertices.len() as i32,
                radius: m.radius(),
            });
            prop_vertices.extend_from_slice(&m.vertices);
            prop_indices.extend_from_slice(&m.indices);
        }

        let broadleaf_base = (TREE_LEVELS * CONIFER_VARIANTS) as u32;
        let rock_base = broadleaf_base + (TREE_LEVELS * BROADLEAF_VARIANTS) as u32;
        let mut objects = Vec::with_capacity(world.conifers.len() + world.broadleaves.len() + world.rocks.len());
        let mut object = |p: &Placed, object_meshes: [u32; TREE_LEVELS], flag: f32, wide_shadow: bool| {
            let radius = meshes[object_meshes[0] as usize].radius * p.scale;
            objects.push(Object { raw: InstanceRaw::new(p, flag), meshes: object_meshes, radius, wide_shadow });
        };
        for p in &world.conifers {
            object(p, std::array::from_fn(|level| (level * CONIFER_VARIANTS) as u32 + u32::from(p.variant)), 0.0, true);
        }
        for p in &world.broadleaves {
            object(p, std::array::from_fn(|level| broadleaf_base + (level * BROADLEAF_VARIANTS) as u32 + u32::from(p.variant)), 1.0, true);
        }
        for p in &world.rocks {
            object(p, [rock_base + u32::from(p.variant); TREE_LEVELS], p.tint[2], p.scale > 1.2);
        }
        let cover: Vec<InstanceRaw> = world.cover.iter().map(|p| InstanceRaw::new(p, 0.0)).collect();

        let vertex = BufferUsage::VERTEX;
        let index = BufferUsage::INDEX;
        let uniform = BufferUsage::UNIFORM;
        let globals_size = std::mem::size_of::<Globals>() as u64;
        let globals = std::array::from_fn(|_| rhi.create_buffer(&BufferDesc { label: "scenery globals", size: globals_size, usage: uniform | BufferUsage::COPY_DST }));
        let post_uniform =
            rhi.create_buffer(&BufferDesc { label: "scenery post", size: std::mem::size_of::<PostUniform>() as u64, usage: uniform | BufferUsage::COPY_DST });
        let object_instances = rhi.create_buffer(&BufferDesc {
            label: "scenery object instances",
            size: (objects.len() as u64 * 3 * std::mem::size_of::<InstanceRaw>() as u64).max(64),
            usage: vertex | BufferUsage::COPY_DST,
        });

        let shadow_usage = TextureUsage::RENDER_ATTACHMENT | TextureUsage::TEXTURE_BINDING;
        let shadow_maps = std::array::from_fn(|i| target(&rhi, "scenery shadow map", (shadow_size[i], shadow_size[i]), DEPTH, shadow_usage, 1));
        let shadow_sampler = rhi.create_sampler(&SamplerDesc { linear: true, compare: true, repeat: false, mipmaps: false });
        let linear_sampler = rhi.create_sampler(&SamplerDesc { linear: true, compare: false, repeat: false, mipmaps: false });

        let terrain_shader = rhi.create_shader("scenery terrain", &[COMMON, LIGHTING, include_str!("shaders/terrain.wgsl")].concat());
        let prop_shader = rhi.create_shader("scenery props", &[COMMON, LIGHTING, include_str!("shaders/props.wgsl")].concat());
        let sky_shader = rhi.create_shader("scenery sky", &[COMMON, include_str!("shaders/sky.wgsl")].concat());
        let water_shader = rhi.create_shader("scenery water", &[COMMON, LIGHTING, include_str!("shaders/water.wgsl")].concat());
        let post_shader = rhi.create_shader("scenery post", include_str!("shaders/post.wgsl"));

        let terrain_layout = [VertexLayout { stride: 40, step: VertexStep::Vertex, attrs: &TERRAIN_ATTRS }];
        let water_layout = [VertexLayout { stride: 36, step: VertexStep::Vertex, attrs: &WATER_ATTRS }];
        let prop_layout = [
            VertexLayout { stride: 48, step: VertexStep::Vertex, attrs: &PROP_ATTRS },
            VertexLayout { stride: 32, step: VertexStep::Instance, attrs: &INSTANCE_ATTRS },
        ];
        let shadow_depth = DepthState { format: DEPTH, write: true, compare: Compare::Less, bias: 2, slope_bias: 2.5 };
        let scene = |label: &'static str, shader, entries: (&'static str, &'static str), layout, blend, cull, depth| PipelineDesc::<B> {
            label,
            shader,
            vs_entry: entries.0,
            fs_entry: entries.1,
            vertex_buffers: layout,
            color_format: Some(HDR),
            blend,
            topology: Topology::TriangleList,
            cull,
            depth: Some(depth),
            samples,
        };
        let shadow = |label: &'static str, shader, vs_entry: &'static str, layout| PipelineDesc::<B> {
            label,
            shader,
            vs_entry,
            fs_entry: "",
            vertex_buffers: layout,
            color_format: None,
            blend: Blend::Opaque,
            topology: Topology::TriangleList,
            cull: Cull::None,
            depth: Some(shadow_depth),
            samples: 1,
        };
        let no_write = |compare| DepthState { format: DEPTH, write: false, compare, bias: 0, slope_bias: 0.0 };
        let terrain_pipeline =
            rhi.create_pipeline(&scene("scenery terrain", &terrain_shader, ("vs_terrain", "fs_terrain"), &terrain_layout, Blend::Opaque, Cull::Back, DepthState::opaque(DEPTH)));
        let prop_pipeline =
            rhi.create_pipeline(&scene("scenery props", &prop_shader, ("vs_prop", "fs_prop"), &prop_layout, Blend::Opaque, Cull::None, DepthState::opaque(DEPTH)));
        let sky_pipeline = rhi.create_pipeline(&scene("scenery sky", &sky_shader, ("vs_sky", "fs_sky"), &[], Blend::Opaque, Cull::None, no_write(Compare::LessEqual)));
        let water_pipeline =
            rhi.create_pipeline(&scene("scenery water", &water_shader, ("vs_water", "fs_water"), &water_layout, Blend::Alpha, Cull::None, no_write(Compare::Less)));
        let terrain_shadow_pipeline = rhi.create_pipeline(&shadow("scenery terrain shadow", &terrain_shader, "vs_terrain_shadow", &terrain_layout));
        let prop_shadow_pipeline = rhi.create_pipeline(&shadow("scenery prop shadow", &prop_shader, "vs_prop_shadow", &prop_layout));

        let lit_group = |pipeline: &B::Pipeline| {
            rhi.create_bind_group(
                pipeline,
                0,
                &[
                    Binding::Uniform { binding: 0, buffer: &globals[0] },
                    Binding::Texture { binding: 1, view: &shadow_maps[0].view },
                    Binding::Texture { binding: 2, view: &shadow_maps[1].view },
                    Binding::Sampler { binding: 3, sampler: &shadow_sampler },
                ],
            )
        };
        let plain_group = |pipeline: &B::Pipeline, buffer: &B::Buffer| rhi.create_bind_group(pipeline, 0, &[Binding::Uniform { binding: 0, buffer }]);
        let terrain_group = lit_group(&terrain_pipeline);
        let prop_group = lit_group(&prop_pipeline);
        let water_group = lit_group(&water_pipeline);
        let sky_group = plain_group(&sky_pipeline, &globals[0]);
        let terrain_shadow_groups = std::array::from_fn(|i| plain_group(&terrain_shadow_pipeline, &globals[i + 1]));
        let prop_shadow_groups = std::array::from_fn(|i| plain_group(&prop_shadow_pipeline, &globals[i + 1]));

        let post = |label: &'static str, fs_entry: &'static str, format| {
            rhi.create_pipeline(&PipelineDesc::color(label, &post_shader, ("vs_screen", fs_entry), &[], format, Blend::Opaque))
        };
        let prefilter_pipeline = post("scenery bloom prefilter", "fs_prefilter", HDR);
        let down_pipeline = post("scenery bloom down", "fs_down", HDR);
        let up_pipeline = post("scenery bloom up", "fs_up", HDR);
        let shafts_pipeline = post("scenery light shafts", "fs_shafts", HDR);
        let meter_pipeline = post("scenery light meter", "fs_meter", HDR);
        let adapt_pipeline = post("scenery eye adaptation", "fs_adapt", HDR);
        let final_pipeline = post("scenery display", "fs_final", target_format);

        let mesh_count = meshes.len();
        Self {
            terrain_vertices: buffer_with(&rhi, "scenery terrain vertices", vertex, bytemuck::cast_slice::<TerrainVertex, u8>(&terrain)),
            terrain_indices: buffer_with(&rhi, "scenery terrain indices", index, bytemuck::cast_slice(&terrain_index)),
            terrain_index_count: terrain_index.len() as u32,
            far_vertices: buffer_with(&rhi, "scenery backdrop vertices", vertex, bytemuck::cast_slice(&far)),
            far_indices: buffer_with(&rhi, "scenery backdrop indices", index, bytemuck::cast_slice(&far_index)),
            far_index_count: far_index.len() as u32,
            water_vertices: buffer_with(&rhi, "scenery water vertices", vertex, bytemuck::cast_slice::<WaterVertex, u8>(&water)),
            water_indices: buffer_with(&rhi, "scenery water indices", index, bytemuck::cast_slice(&water_index)),
            water_index_count: water_index.len() as u32,
            prop_vertices: buffer_with(&rhi, "scenery prop vertices", vertex, bytemuck::cast_slice(&prop_vertices)),
            prop_indices: buffer_with(&rhi, "scenery prop indices", index, bytemuck::cast_slice(&prop_indices)),
            meshes,
            cover_mesh_base,
            objects,
            object_instances,
            cover_instances: buffer_with(&rhi, "scenery cover instances", vertex, bytemuck::cast_slice(&cover)),
            cover_chunks: world.cover_chunks.clone(),
            globals,
            post_uniform,
            shadow_maps,
            linear_sampler,
            terrain_pipeline,
            terrain_group,
            terrain_shadow_pipeline,
            terrain_shadow_groups,
            prop_pipeline,
            prop_group,
            prop_shadow_pipeline,
            prop_shadow_groups,
            sky_pipeline,
            sky_group,
            water_pipeline,
            water_group,
            prefilter_pipeline,
            down_pipeline,
            up_pipeline,
            shafts_pipeline,
            meter_pipeline,
            adapt_pipeline,
            final_pipeline,
            targets: None,
            frame: 0,
            buckets: vec![Vec::new(); mesh_count * 3],
            staged: Vec::new(),
            stats: FrameStats::default(),
            target_format,
            quality,
            samples,
            shadow_size,
            rhi,
        }
    }

    /// The quality this renderer was built with, after rounding to supported values.
    pub fn quality(&self) -> Quality {
        self.quality
    }

    pub fn last_frame_stats(&self) -> FrameStats {
        self.stats
    }

    fn create_targets(&self, size: (u32, u32)) -> Targets<B> {
        let rhi = &self.rhi;
        let sampled = TextureUsage::RENDER_ATTACHMENT | TextureUsage::TEXTURE_BINDING;
        let scene = target(rhi, "scenery scene", size, HDR, sampled, 1);
        let multisampled = (self.samples > 1).then(|| target(rhi, "scenery scene multisampled", size, HDR, TextureUsage::RENDER_ATTACHMENT, self.samples));
        let depth = target(rhi, "scenery depth", size, DEPTH, TextureUsage::RENDER_ATTACHMENT, self.samples);
        let level_size = |level: usize| ((size.0 >> (level + 1)).max(1), (size.1 >> (level + 1)).max(1));
        let bloom_down: Vec<Target<B>> = (0..BLOOM_LEVELS).map(|level| target(rhi, "scenery bloom down", level_size(level), HDR, sampled, 1)).collect();
        let bloom_up: Vec<Target<B>> = (0..BLOOM_LEVELS - 1).map(|level| target(rhi, "scenery bloom up", level_size(level), HDR, sampled, 1)).collect();
        let shafts = target(rhi, "scenery light shafts", level_size(0), HDR, sampled, 1);

        let prefilter_group = rhi.create_bind_group(
            &self.prefilter_pipeline,
            0,
            &[sampled_view(0, &scene), Binding::Sampler { binding: 3, sampler: &self.linear_sampler }, Binding::Uniform { binding: 4, buffer: &self.post_uniform }],
        );
        let down_groups = (1..BLOOM_LEVELS)
            .map(|level| rhi.create_bind_group(&self.down_pipeline, 0, &[sampled_view(0, &bloom_down[level - 1]), Binding::Sampler { binding: 3, sampler: &self.linear_sampler }]))
            .collect();
        // Level `i` of the up chain adds the blurred smaller level to bloom_down[i].
        let up_groups = (0..BLOOM_LEVELS - 1)
            .map(|level| {
                let smaller = if level == BLOOM_LEVELS - 2 { &bloom_down[BLOOM_LEVELS - 1] } else { &bloom_up[level + 1] };
                rhi.create_bind_group(
                    &self.up_pipeline,
                    0,
                    &[sampled_view(0, smaller), sampled_view(1, &bloom_down[level]), Binding::Sampler { binding: 3, sampler: &self.linear_sampler }],
                )
            })
            .collect();
        let shafts_group = rhi.create_bind_group(
            &self.shafts_pipeline,
            0,
            &[sampled_view(0, &scene), Binding::Sampler { binding: 3, sampler: &self.linear_sampler }, Binding::Uniform { binding: 4, buffer: &self.post_uniform }],
        );
        let meter = target(rhi, "scenery light meter", (64, 36), HDR, sampled, 1);
        let adapted: [Target<B>; 2] = std::array::from_fn(|_| target(rhi, "scenery adapted brightness", (1, 1), HDR, sampled, 1));
        let meter_group = rhi.create_bind_group(&self.meter_pipeline, 0, &[sampled_view(0, &scene), Binding::Sampler { binding: 3, sampler: &self.linear_sampler }]);
        // Frame parity `i` writes adapted[i] from adapted[1 - i].
        let adapt_groups = std::array::from_fn(|i| {
            rhi.create_bind_group(
                &self.adapt_pipeline,
                0,
                &[sampled_view(0, &meter), sampled_view(1, &adapted[1 - i]), Binding::Uniform { binding: 4, buffer: &self.post_uniform }],
            )
        });
        let final_groups = std::array::from_fn(|i| {
            rhi.create_bind_group(
                &self.final_pipeline,
                0,
                &[
                    sampled_view(0, &scene),
                    sampled_view(1, &bloom_up[0]),
                    sampled_view(2, &shafts),
                    Binding::Sampler { binding: 3, sampler: &self.linear_sampler },
                    Binding::Uniform { binding: 4, buffer: &self.post_uniform },
                    sampled_view(5, &adapted[i]),
                ],
            )
        });
        Targets {
            size,
            meter,
            adapted,
            meter_group,
            adapt_groups,
            multisampled,
            depth,
            scene,
            bloom_down,
            bloom_up,
            shafts,
            prefilter_group,
            down_groups,
            up_groups,
            shafts_group,
            final_groups,
        }
    }

    /// Draws one frame into `target`, a view of `size` pixels in the format
    /// given to [`Renderer::new`].
    pub fn render(&mut self, target: &B::TextureView, size: (u32, u32), camera: &Camera, environment: &Environment, settings: &Settings) {
        let settings = settings.sanitized();
        let size = (size.0.max(1), size.1.max(1));
        if self.targets.as_ref().map(|t| t.size) != Some(size) {
            self.targets = Some(self.create_targets(size));
            self.frame = 0;
        }
        // The first frame takes the metered brightness as it is; later frames ease toward it.
        let elapsed = if environment.frame_seconds.is_finite() { environment.frame_seconds.clamp(0.0, 0.25) } else { 0.0 };
        let adaptation = if self.frame == 0 { 1.0 } else { 1.0 - (-elapsed * settings.look.adaptation_speed).exp() };
        let aspect = size.0 as f32 / size.1 as f32;
        let view_proj = camera.view_proj(aspect);
        let forward = camera.forward();
        let right = camera.right();
        let up = cross(right, forward);
        let tangent = (camera.fov_y * 0.5).tan();
        let sun = settings.sun_direction();
        let shadow_radius = [settings.shadows.near_range, settings.shadows.far_range];
        let cascades: [Cascade; 2] = std::array::from_fn(|i| {
            Cascade::new(add(camera.position, scale(forward, shadow_radius[i] * 0.6)), sun, shadow_radius[i], SHADOW_DEPTH[i], self.shadow_size[i])
        });

        let day = smoothstep(-0.05, 0.4, sun[1]);
        let sun_color = lerp3([1.0, 0.4, 0.15], [1.0, 0.9, 0.76], smoothstep(0.0, 0.62, sun[1]));
        let sun_intensity = settings.sun.intensity * smoothstep(-0.03, 0.12, sun[1]);
        let ambient_sky = scale(lerp3([0.16, 0.18, 0.34], [0.42, 0.58, 0.9], day), settings.sun.ambient);
        let ambient_ground = scale(lerp3([0.07, 0.06, 0.06], [0.22, 0.2, 0.12], day), settings.sun.ambient);
        let wide = |v: V3, w: f32| [v[0], v[1], v[2], w];
        let mut globals = Globals {
            view_proj,
            shadow0: cascades[0].matrix,
            shadow1: cascades[1].matrix,
            cam_right: wide(scale(right, tangent * aspect), 0.0),
            cam_up: wide(scale(up, tangent), 0.0),
            cam_forward: wide(forward, 0.0),
            camera: wide(camera.position, environment.time),
            sun_dir: wide(sun, sun_intensity),
            sun_color: wide(sun_color, 0.0),
            ambient_sky: wide(ambient_sky, 0.0),
            ambient_ground: wide(ambient_ground, 0.0),
            fog: [0.0017 * settings.fog.density, settings.fog.falloff, environment.fog_height, settings.fog.haze],
            viewport: [size.0 as f32, size.1 as f32, 0.0, settings.vegetation.grass_distance],
            cascades: [1.0 / self.shadow_size[0] as f32, 1.0 / self.shadow_size[1] as f32, 0.0, 0.0],
            // Clouds appear where the cloud noise is above this level: full cover at 0.16, none at 0.86.
            sky: [settings.sky.brightness, settings.sky.sun_glow, 0.86 - 0.7 * settings.sky.cloud_cover, settings.sky.cloud_speed],
            shadow: [settings.shadows.softness, f32::from(u8::from(settings.shadows.enabled)), 0.0, 0.0],
            wind: [settings.wind.strength, settings.wind.speed, 0.0, 0.0],
            water_color: wide(settings.water.color, settings.water.clarity),
            water: [settings.water.ripples, settings.water.flow_speed, settings.water.foam, settings.water.glint],
            leaf_tint: wide(settings.vegetation.tint, settings.vegetation.translucency),
            detail: [settings.vegetation.leaf_detail, settings.vegetation.autumn, settings.ground.bumps, settings.ground.caustics],
        };
        self.rhi.write_buffer(&self.globals[0], 0, bytemuck::bytes_of(&globals));
        for (i, cascade) in cascades.iter().enumerate() {
            globals.view_proj = cascade.matrix;
            self.rhi.write_buffer(&self.globals[i + 1], 0, bytemuck::bytes_of(&globals));
        }

        // The sun in texture coordinates, for the light shafts.
        let sun_clip = transform(&view_proj, add(camera.position, scale(sun, 1000.0)));
        let facing = dot(forward, sun);
        let (sun_uv, shaft_strength) = if sun_clip[3] > 1e-3 {
            ([sun_clip[0] / sun_clip[3] * 0.5 + 0.5, 0.5 - sun_clip[1] / sun_clip[3] * 0.5], settings.look.light_shafts * smoothstep(0.1, 0.7, facing) * smoothstep(-0.02, 0.1, sun[1]))
        } else {
            ([0.5, 0.5], 0.0)
        };
        let post = PostUniform {
            sun: [sun_uv[0], sun_uv[1], shaft_strength, aspect],
            grade: [settings.look.exposure, settings.look.bloom, if self.target_format.is_srgb() { 0.0 } else { 1.0 }, environment.time.fract() * 64.0],
            sun_color: wide(scale(sun_color, 1.3), adaptation),
            tone: [settings.look.saturation, settings.look.contrast, settings.look.vignette, settings.look.grain],
            meter: [f32::from(u8::from(settings.look.auto_exposure)), settings.look.bloom_threshold, 0.0, 0.0],
        };
        self.rhi.write_buffer(&self.post_uniform, 0, bytemuck::bytes_of(&post));

        // Sort trees and rocks into per-mesh instance lists: main view, near shadow, wide shadow.
        let mesh_count = self.meshes.len();
        for bucket in &mut self.buckets {
            bucket.clear();
        }
        let planes = frustum_planes(&view_proj);
        for object in &self.objects {
            let position = [object.raw.place[0], object.raw.place[1], object.raw.place[2]];
            let away = length(sub(position, camera.position));
            let detail = settings.vegetation.detail_distance;
            let level = if away < LOD_DISTANCE[0] * detail {
                0
            } else if away < LOD_DISTANCE[1] * detail {
                1
            } else {
                2
            };
            let mesh = object.meshes[level] as usize;
            // A rock that covers less than a pixel or two is not worth a draw.
            let visible_size = away < 380.0 || object.radius > 2.5;
            if visible_size && sphere_visible(&planes, position, object.radius) {
                self.buckets[mesh].push(object.raw);
            }
            if !settings.shadows.enabled {
                continue;
            }
            if cascades[0].covers(position, object.radius) {
                self.buckets[mesh_count + mesh].push(object.raw);
            }
            if object.wide_shadow && cascades[1].covers(position, object.radius) {
                let level = if away < SHADOW_LOD_DISTANCE * detail { 1 } else { 2 };
                self.buckets[2 * mesh_count + object.meshes[level] as usize].push(object.raw);
            }
        }
        self.staged.clear();
        let mut draws: [Vec<Draw>; 3] = Default::default();
        for (pass, list) in draws.iter_mut().enumerate() {
            for mesh in 0..mesh_count {
                let bucket = &self.buckets[pass * mesh_count + mesh];
                if !bucket.is_empty() {
                    list.push(Draw { mesh: mesh as u32, first: self.staged.len() as u32, count: bucket.len() as u32 });
                    self.staged.extend_from_slice(bucket);
                }
            }
        }
        if !self.staged.is_empty() {
            self.rhi.write_buffer(&self.object_instances, 0, bytemuck::cast_slice(&self.staged));
        }
        let mut cover_draws: Vec<Draw> = Vec::new();
        let reach = settings.vegetation.grass_distance + CHUNK * 0.71;
        for chunk in &self.cover_chunks {
            if length(sub(chunk.center, camera.position)) > reach || !sphere_visible(&planes, chunk.center, CHUNK * 0.9) {
                continue;
            }
            let mut first = chunk.first;
            for (variant, &count) in chunk.counts.iter().enumerate() {
                if count > 0 {
                    cover_draws.push(Draw { mesh: self.cover_mesh_base + variant as u32, first, count });
                }
                first += count;
            }
        }

        let mut stats = FrameStats::default();
        let meshes = &self.meshes;
        let mut prop_commands = |commands: &mut Vec<Command<'_, B>>, list: &[Draw]| {
            for draw in list {
                let m = meshes[draw.mesh as usize];
                commands.push(Command::DrawIndexed {
                    indices: m.first_index..m.first_index + m.index_count,
                    base_vertex: m.base_vertex,
                    instances: draw.first..draw.first + draw.count,
                });
                stats.triangles += u64::from(m.index_count / 3) * u64::from(draw.count);
                stats.instances += draw.count;
                stats.draws += 1;
            }
        };

        let targets = self.targets.as_ref().expect("targets exist for this size");
        let mut encoder = self.rhi.create_encoder("scenery frame");
        let mut shadow_terrain_triangles = 0;
        // With shadows off the maps are not drawn; the shaders do not read them.
        for cascade in 0..if settings.shadows.enabled { 2 } else { 0 } {
            let mut commands: Vec<Command<'_, B>> = vec![
                Command::SetPipeline(&self.terrain_shadow_pipeline),
                Command::SetBindGroup(0, &self.terrain_shadow_groups[cascade]),
                Command::SetVertexBuffer(0, &self.terrain_vertices),
                Command::SetIndexBuffer(&self.terrain_indices),
                Command::DrawIndexed { indices: 0..self.terrain_index_count, base_vertex: 0, instances: 0..1 },
            ];
            if cascade == 1 {
                // Distant mountains can shade the valley when the sun is low.
                commands.extend([
                    Command::SetVertexBuffer(0, &self.far_vertices),
                    Command::SetIndexBuffer(&self.far_indices),
                    Command::DrawIndexed { indices: 0..self.far_index_count, base_vertex: 0, instances: 0..1 },
                ]);
            }
            commands.extend([
                Command::SetPipeline(&self.prop_shadow_pipeline),
                Command::SetBindGroup(0, &self.prop_shadow_groups[cascade]),
                Command::SetVertexBuffer(0, &self.prop_vertices),
                Command::SetVertexBuffer(1, &self.object_instances),
                Command::SetIndexBuffer(&self.prop_indices),
            ]);
            prop_commands(&mut commands, &draws[cascade + 1]);
            shadow_terrain_triangles += u64::from((self.terrain_index_count + if cascade == 1 { self.far_index_count } else { 0 }) / 3);
            let depth = DepthAttachment { view: &self.shadow_maps[cascade].view, clear: Some(1.0), store: true };
            self.rhi.encode_pass(&mut encoder, "scenery shadow", None, Some(&depth), &commands);
        }

        let mut commands: Vec<Command<'_, B>> = vec![
            Command::SetPipeline(&self.terrain_pipeline),
            Command::SetBindGroup(0, &self.terrain_group),
            Command::SetVertexBuffer(0, &self.terrain_vertices),
            Command::SetIndexBuffer(&self.terrain_indices),
            Command::DrawIndexed { indices: 0..self.terrain_index_count, base_vertex: 0, instances: 0..1 },
            Command::SetVertexBuffer(0, &self.far_vertices),
            Command::SetIndexBuffer(&self.far_indices),
            Command::DrawIndexed { indices: 0..self.far_index_count, base_vertex: 0, instances: 0..1 },
            Command::SetPipeline(&self.prop_pipeline),
            Command::SetBindGroup(0, &self.prop_group),
            Command::SetVertexBuffer(0, &self.prop_vertices),
            Command::SetVertexBuffer(1, &self.object_instances),
            Command::SetIndexBuffer(&self.prop_indices),
        ];
        prop_commands(&mut commands, &draws[0]);
        commands.push(Command::SetVertexBuffer(1, &self.cover_instances));
        prop_commands(&mut commands, &cover_draws);
        commands.extend([
            Command::SetPipeline(&self.sky_pipeline),
            Command::SetBindGroup(0, &self.sky_group),
            Command::Draw { vertices: 0..3, instances: 0..1 },
            Command::SetPipeline(&self.water_pipeline),
            Command::SetBindGroup(0, &self.water_group),
            Command::SetVertexBuffer(0, &self.water_vertices),
            Command::SetIndexBuffer(&self.water_indices),
            Command::DrawIndexed { indices: 0..self.water_index_count, base_vertex: 0, instances: 0..1 },
        ]);
        stats.triangles += shadow_terrain_triangles + u64::from((self.terrain_index_count + self.far_index_count + self.water_index_count) / 3);
        let clear = Some([0.0, 0.0, 0.0, 1.0]);
        let color = match &targets.multisampled {
            Some(multisampled) => ColorAttachment { view: &multisampled.view, clear, resolve: Some(&targets.scene.view) },
            None => ColorAttachment { view: &targets.scene.view, clear, resolve: None },
        };
        let depth = DepthAttachment { view: &targets.depth.view, clear: Some(1.0), store: false };
        self.rhi.encode_pass(&mut encoder, "scenery scene", Some(&color), Some(&depth), &commands);
        drop(commands);

        let screen_pass = |encoder: &mut B::Encoder, label: &str, view: &B::TextureView, pipeline: &B::Pipeline, group: &B::BindGroup| {
            let color = ColorAttachment { view, clear: Some([0.0, 0.0, 0.0, 1.0]), resolve: None };
            let commands = [Command::SetPipeline(pipeline), Command::SetBindGroup(0, group), Command::Draw { vertices: 0..3, instances: 0..1 }];
            self.rhi.encode_render_pass(encoder, label, &color, &commands);
        };
        screen_pass(&mut encoder, "scenery bloom prefilter", &targets.bloom_down[0].view, &self.prefilter_pipeline, &targets.prefilter_group);
        for level in 1..BLOOM_LEVELS {
            screen_pass(&mut encoder, "scenery bloom down", &targets.bloom_down[level].view, &self.down_pipeline, &targets.down_groups[level - 1]);
        }
        for level in (0..BLOOM_LEVELS - 1).rev() {
            screen_pass(&mut encoder, "scenery bloom up", &targets.bloom_up[level].view, &self.up_pipeline, &targets.up_groups[level]);
        }
        screen_pass(&mut encoder, "scenery light shafts", &targets.shafts.view, &self.shafts_pipeline, &targets.shafts_group);
        let parity = (self.frame % 2) as usize;
        screen_pass(&mut encoder, "scenery light meter", &targets.meter.view, &self.meter_pipeline, &targets.meter_group);
        screen_pass(&mut encoder, "scenery eye adaptation", &targets.adapted[parity].view, &self.adapt_pipeline, &targets.adapt_groups[parity]);
        screen_pass(&mut encoder, "scenery display", target, &self.final_pipeline, &targets.final_groups[parity]);
        self.rhi.submit(encoder);
        self.stats = stats;
        self.frame += 1;
    }
}
