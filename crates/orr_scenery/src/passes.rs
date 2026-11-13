//! The outdoor scene as a pass of the engine's imported scene renderer
//! ([`orr_render::RenderPass`]): the sky, water surfaces and vegetation spread
//! over terrains. A scene sets them with `orr_gfx` components (`Sky`, `Water`,
//! `Scatter`); the editor or a game reads those into an [`OutdoorScene`] and
//! hands it to [`OutdoorPass::set_scene`].
//!
//! The pass draws in four stages:
//!
//! - shadow: trees and rocks into the engine's shared sun shadow map, so they
//!   shade the terrain and each other;
//! - opaque: trees, rocks and ground cover, lit by the scene's sun and
//!   ambient light and shaded by the shared shadow map;
//! - sky: the sky with sun disc and clouds wherever nothing was drawn;
//! - transparent: water surfaces, blended over the scene.
//!
//! The terrain itself is an engine model (`orr_gfx::Terrain`), and fog is the
//! post chain's height fog, so both are the same for every object.

use crate::math::{add, cross, frustum_planes, length, normalize, scale, sphere_visible, sub, Mat4, V3};
use crate::mesh::{self, Mesh, PropVertex, WaterVertex, TREE_LEVELS};
use crate::world::{BROADLEAF_VARIANTS, CONIFER_VARIANTS, COVER_VARIANTS, ROCK_VARIANTS};
use bytemuck::{Pod, Zeroable};
use orr_render::orr_rhi::{
    Binding, Blend, BufferDesc, BufferUsage, ColorAttachment, Command, Compare, Cull, DepthAttachment, DepthState, PipelineDesc, Rhi, SamplerDesc, TextureDesc,
    TextureFormat, TextureUsage, Topology, VertexAttr, VertexFormat, VertexLayout, VertexStep,
};
use orr_render::{PassFrame, PassStage, Projection, RenderPass};
use std::any::Any;
use std::sync::Arc;

pub(crate) const COMMON: &str = include_str!("shaders/common.wgsl");
const DEPTH: TextureFormat = TextureFormat::Depth32Float;
/// Trees closer than the first distance use the detailed mesh, closer than the second the middle one.
const LOD_DISTANCE: [f32; 2] = [95.0, 260.0];
/// In the shadow map, trees beyond this distance from the camera use the simplest mesh.
const SHADOW_LOD_DISTANCE: f32 = 150.0;
/// Ground cover is drawn this far from the camera; it shrinks into the ground before.
const GRASS_DISTANCE: f32 = 85.0;
/// Farthest a tree or rock is drawn.
const FAR_DISTANCE: f32 = 3000.0;

pub(crate) fn lighting_source() -> String {
    let lighting = include_str!("shaders/lighting.wgsl");
    let shading = &lighting[lighting.find("struct Surface").expect("lighting.wgsl defines Surface")..];
    format!("{}\n{shading}", include_str!("shaders/pass_lighting.wgsl"))
}

/// Heights of one terrain, as the view reads its heightmap.
#[derive(Clone, Debug)]
pub struct HeightField {
    /// World position of the terrain's center at height zero.
    pub center: V3,
    /// Size in meters along x and z.
    pub size: [f32; 2],
    /// Height in meters of a height of 1.
    pub height: f32,
    pub width: u32,
    pub depth: u32,
    /// Heights 0..1, row by row from the -z edge.
    pub heights: Arc<Vec<f32>>,
}

/// Two height fields are the same when they place the same heights the same
/// way; the heights themselves are compared by identity, not value by value.
impl PartialEq for HeightField {
    fn eq(&self, other: &Self) -> bool {
        self.center == other.center && self.size == other.size && self.height == other.height && self.width == other.width && self.depth == other.depth && Arc::ptr_eq(&self.heights, &other.heights)
    }
}

impl HeightField {
    /// World height of the ground at `(x, z)`, or `None` outside the terrain.
    pub fn ground(&self, x: f32, z: f32) -> Option<f32> {
        let u = (x - self.center[0]) / self.size[0] + 0.5;
        let v = (z - self.center[2]) / self.size[1] + 0.5;
        if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
            return None;
        }
        let fx = u * (self.width - 1) as f32;
        let fy = v * (self.depth - 1) as f32;
        let (x0, y0) = (fx.floor() as u32, fy.floor() as u32);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.depth - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let at = |x: u32, y: u32| self.heights[(y * self.width + x) as usize];
        let h = (at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx) * (1.0 - ty) + (at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx) * ty;
        Some(self.center[1] + h * self.height)
    }

    /// Unit normal of the ground at `(x, z)`.
    pub fn normal(&self, x: f32, z: f32) -> V3 {
        let e = (self.size[0] / (self.width - 1) as f32).max(self.size[1] / (self.depth - 1) as f32);
        let h = |x: f32, z: f32| self.ground(x, z);
        let center = h(x, z).unwrap_or(0.0);
        let dx = h(x + e, z).unwrap_or(center) - h(x - e, z).unwrap_or(center);
        let dz = h(x, z + e).unwrap_or(center) - h(x, z - e).unwrap_or(center);
        normalize([-dx, 2.0 * e, -dz])
    }
}

/// The drawn sky; fog belongs to the post chain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutdoorSky {
    pub brightness: f32,
    pub sun_glow: f32,
    pub cloud_cover: f32,
    pub cloud_speed: f32,
    pub wind_strength: f32,
    pub wind_speed: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutdoorWater {
    /// Center of the surface; its height is the water level.
    pub center: V3,
    pub size: [f32; 2],
    pub color: V3,
    pub clarity: f32,
    pub ripples: f32,
    pub flow: [f32; 2],
    pub glint: f32,
    /// The ground under the water, for its depth.
    pub ground: Option<HeightField>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScatterKind {
    Conifer,
    Broadleaf,
    Rock,
    Grass,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutdoorScatter {
    pub kind: ScatterKind,
    pub center: V3,
    pub area: [f32; 2],
    /// Lowest and highest ground in world height.
    pub heights: [f32; 2],
    /// Steepest ground in degrees.
    pub max_slope: f32,
    pub scale: [f32; 2],
    pub count: u32,
    pub seed: u32,
    pub cast_shadows: bool,
    pub ground: HeightField,
}

/// Everything the pass draws.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutdoorScene {
    /// `None` draws no sky.
    pub sky: Option<OutdoorSky>,
    pub waters: Vec<OutdoorWater>,
    pub scatters: Vec<OutdoorScatter>,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct Globals {
    pub(crate) view_proj: Mat4,
    pub(crate) shadow0: Mat4,
    pub(crate) shadow1: Mat4,
    pub(crate) cam_right: [f32; 4],
    pub(crate) cam_up: [f32; 4],
    pub(crate) cam_forward: [f32; 4],
    pub(crate) camera: [f32; 4],
    pub(crate) sun_dir: [f32; 4],
    pub(crate) sun_color: [f32; 4],
    pub(crate) ambient_sky: [f32; 4],
    pub(crate) ambient_ground: [f32; 4],
    pub(crate) fog: [f32; 4],
    pub(crate) viewport: [f32; 4],
    pub(crate) cascades: [f32; 4],
    pub(crate) sky: [f32; 4],
    pub(crate) shadow: [f32; 4],
    pub(crate) wind: [f32; 4],
    pub(crate) water_color: [f32; 4],
    pub(crate) water: [f32; 4],
    pub(crate) leaf_tint: [f32; 4],
    pub(crate) detail: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct InstanceRaw {
    place: [f32; 4],
    look: [f32; 4],
}

/// One placed tree, rock or tuft.
#[derive(Clone, Copy)]
struct Object {
    raw: InstanceRaw,
    /// Mesh per level of detail, closest first.
    meshes: [u32; TREE_LEVELS],
    radius: f32,
    grass: bool,
    casts: bool,
}

#[derive(Clone, Copy)]
struct MeshRange {
    first_index: u32,
    index_count: u32,
    base_vertex: i32,
    radius: f32,
}

struct WaterMesh<B: Rhi> {
    vertices: B::Buffer,
    indices: B::Buffer,
    count: u32,
    globals: B::Buffer,
    color: V3,
    clarity: f32,
    ripples: f32,
    glint: f32,
    bind: Option<B::BindGroup>,
}

/// Bind groups of the props, the sky and the shadow casters, made for one shadow map.
struct Binds<B: Rhi> {
    map_id: u64,
    props: B::BindGroup,
    sky: B::BindGroup,
    shadow: B::BindGroup,
}

struct Pipelines<B: Rhi> {
    format: TextureFormat,
    props: B::Pipeline,
    sky: B::Pipeline,
    water: B::Pipeline,
}

/// The outdoor pass. Add it once with `ImportedSceneRenderer::add_pass` and
/// change what it draws with [`OutdoorPass::set_scene`].
pub struct OutdoorPass<B: Rhi> {
    prop_shader: B::Shader,
    sky_shader: B::Shader,
    water_shader: B::Shader,
    pipelines: Option<Pipelines<B>>,
    prop_shadow: B::Pipeline,
    meshes: Vec<MeshRange>,
    prop_vertices: B::Buffer,
    prop_indices: B::Buffer,
    globals: B::Buffer,
    shadow_globals: B::Buffer,
    no_shadow: (B::Texture, B::TextureView),
    shadow_sampler: B::Sampler,
    scene: OutdoorScene,
    objects: Vec<Object>,
    waters: Vec<WaterMesh<B>>,
    instances: Option<(B::Buffer, u64)>,
    /// Draw ranges of this frame: (mesh, first instance, count), main then shadow.
    draws: Vec<(u32, u32, u32)>,
    shadow_draws: Vec<(u32, u32, u32)>,
    binds: Option<Binds<B>>,
    time: f32,
    /// True once objects and water were made for `scene`.
    placed: bool,
}

const PROP_ATTRS: [VertexAttr; 4] = [
    VertexAttr { location: 0, format: VertexFormat::Float32x3, offset: 0 },
    VertexAttr { location: 1, format: VertexFormat::Float32x3, offset: 12 },
    VertexAttr { location: 2, format: VertexFormat::Float32x3, offset: 24 },
    VertexAttr { location: 3, format: VertexFormat::Float32x3, offset: 36 },
];
const INSTANCE_ATTRS: [VertexAttr; 2] = [VertexAttr { location: 4, format: VertexFormat::Float32x4, offset: 0 }, VertexAttr { location: 5, format: VertexFormat::Float32x4, offset: 16 }];
const WATER_ATTRS: [VertexAttr; 3] = [
    VertexAttr { location: 0, format: VertexFormat::Float32x3, offset: 0 },
    VertexAttr { location: 1, format: VertexFormat::Float32x4, offset: 12 },
    VertexAttr { location: 2, format: VertexFormat::Float32x2, offset: 28 },
];

fn prop_layout() -> [VertexLayout<'static>; 2] {
    [VertexLayout { stride: 48, step: VertexStep::Vertex, attrs: &PROP_ATTRS }, VertexLayout { stride: 32, step: VertexStep::Instance, attrs: &INSTANCE_ATTRS }]
}

fn buffer_with<B: Rhi>(rhi: &B, label: &str, usage: BufferUsage, data: &[u8]) -> B::Buffer {
    let buffer = rhi.create_buffer(&BufferDesc { label, size: (data.len() as u64).max(4), usage: usage | BufferUsage::COPY_DST });
    rhi.write_buffer(&buffer, 0, data);
    buffer
}

/// A small deterministic random source for placing objects.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 40) as u32) as f32 / (1u32 << 24) as f32
    }
    fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.next()
    }
}

impl<B: Rhi> OutdoorPass<B> {
    pub fn new(rhi: &B) -> Self {
        let lighting = lighting_source();
        let prop_shader = rhi.create_shader("outdoor props", &[COMMON, &lighting, include_str!("shaders/props.wgsl")].concat());
        let sky_shader = rhi.create_shader("outdoor sky", &[COMMON, include_str!("shaders/sky.wgsl")].concat());
        let water_shader = rhi.create_shader("outdoor water", &[COMMON, &lighting, include_str!("shaders/water.wgsl")].concat());
        let layout = prop_layout();
        let prop_shadow = rhi.create_pipeline(&PipelineDesc::<B> {
            label: "outdoor prop shadow",
            shader: &prop_shader,
            vs_entry: "vs_prop_shadow",
            fs_entry: "",
            vertex_buffers: &layout,
            color_format: None,
            blend: Blend::Opaque,
            topology: Topology::TriangleList,
            cull: Cull::None,
            depth: Some(DepthState { format: DEPTH, write: true, compare: Compare::Less, bias: 2, slope_bias: 2.5 }),
            samples: 1,
        });
        // Mesh order: conifers by level of detail, broadleaves by level of detail, rocks, ground cover.
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
        prop_meshes.push(mesh::grass_tuft(false, 400));
        prop_meshes.push(mesh::grass_tuft(true, 401));
        prop_meshes.push(mesh::fern(402));
        let mut vertices: Vec<PropVertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut meshes = Vec::new();
        for m in &prop_meshes {
            meshes.push(MeshRange { first_index: indices.len() as u32, index_count: m.indices.len() as u32, base_vertex: vertices.len() as i32, radius: m.radius() });
            vertices.extend_from_slice(&m.vertices);
            indices.extend_from_slice(&m.indices);
        }
        let globals_desc = |label| BufferDesc { label, size: std::mem::size_of::<Globals>() as u64, usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST };
        let no_shadow = rhi.create_texture(&TextureDesc {
            label: "outdoor no shadow",
            width: 1,
            height: 1,
            format: DEPTH,
            usage: TextureUsage::TEXTURE_BINDING | TextureUsage::RENDER_ATTACHMENT,
            sample_count: 1,
            view_formats: &[],
        });
        let no_shadow_view = rhi.create_texture_view(&no_shadow, None);
        Self {
            prop_shader,
            sky_shader,
            water_shader,
            pipelines: None,
            prop_shadow,
            meshes,
            prop_vertices: buffer_with(rhi, "outdoor prop vertices", BufferUsage::VERTEX, bytemuck::cast_slice(&vertices)),
            prop_indices: buffer_with(rhi, "outdoor prop indices", BufferUsage::INDEX, bytemuck::cast_slice(&indices)),
            globals: rhi.create_buffer(&globals_desc("outdoor globals")),
            shadow_globals: rhi.create_buffer(&globals_desc("outdoor shadow globals")),
            no_shadow: (no_shadow, no_shadow_view),
            shadow_sampler: rhi.create_sampler(&SamplerDesc { linear: true, compare: true }),
            scene: OutdoorScene::default(),
            objects: Vec::new(),
            waters: Vec::new(),
            instances: None,
            draws: Vec::new(),
            shadow_draws: Vec::new(),
            binds: None,
            time: 0.0,
            placed: false,
        }
    }

    /// What the pass draws from the next frame on. Objects and water meshes
    /// are made again only when the scene changed.
    pub fn set_scene(&mut self, rhi: &B, scene: &OutdoorScene) {
        if self.placed && *scene == self.scene {
            return;
        }
        self.placed = true;
        self.scene = scene.clone();
        self.objects = self.place_objects();
        self.waters = scene.waters.iter().map(|w| water_mesh(rhi, w)).collect();
        self.binds = None;
    }

    /// The clock in seconds that wind, clouds and water ripples follow.
    pub fn set_time(&mut self, seconds: f32) {
        self.time = if seconds.is_finite() { seconds } else { 0.0 };
    }

    fn place_objects(&self) -> Vec<Object> {
        let broadleaf_base = (TREE_LEVELS * CONIFER_VARIANTS) as u32;
        let rock_base = broadleaf_base + (TREE_LEVELS * BROADLEAF_VARIANTS) as u32;
        let cover_base = rock_base + ROCK_VARIANTS as u32;
        let mut objects = Vec::new();
        for scatter in &self.scene.scatters {
            let mut rng = Rng(u64::from(scatter.seed).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ 0x5851_f42d_4c95_7f2d);
            let min_up = scatter.max_slope.to_radians().cos();
            for _ in 0..scatter.count {
                let x = scatter.center[0] + (rng.next() - 0.5) * scatter.area[0];
                let z = scatter.center[2] + (rng.next() - 0.5) * scatter.area[1];
                let (variant, yaw, hue, bright, size) = (rng.next(), rng.range(0.0, std::f32::consts::TAU), rng.next(), rng.range(0.85, 1.15), rng.range(scatter.scale[0], scatter.scale[1]));
                let Some(ground) = scatter.ground.ground(x, z) else { continue };
                if ground < scatter.heights[0] || ground > scatter.heights[1] || scatter.ground.normal(x, z)[1] < min_up {
                    continue;
                }
                // Nothing grows under water.
                if self.scene.waters.iter().any(|w| (x - w.center[0]).abs() < w.size[0] * 0.5 && (z - w.center[2]).abs() < w.size[1] * 0.5 && ground < w.center[1]) {
                    continue;
                }
                let pick = |count: usize| ((variant * count as f32) as usize).min(count - 1) as u32;
                let (meshes, flag, grass) = match scatter.kind {
                    ScatterKind::Conifer => (std::array::from_fn(|level| (level * CONIFER_VARIANTS) as u32 + pick(CONIFER_VARIANTS)), 0.0, false),
                    ScatterKind::Broadleaf => (std::array::from_fn(|level| broadleaf_base + (level * BROADLEAF_VARIANTS) as u32 + pick(BROADLEAF_VARIANTS)), 1.0, false),
                    ScatterKind::Rock => ([rock_base + pick(ROCK_VARIANTS); TREE_LEVELS], 0.0, false),
                    ScatterKind::Grass => ([cover_base + pick(COVER_VARIANTS); TREE_LEVELS], 0.0, true),
                };
                // Trees sink a little so slopes do not show their roots.
                let sink = if grass { 0.0 } else { 0.3 * size };
                objects.push(Object {
                    raw: InstanceRaw { place: [x, ground - sink, z, size], look: [yaw, hue, bright, flag] },
                    meshes,
                    radius: self.meshes[meshes[0] as usize].radius * size,
                    grass,
                    casts: scatter.cast_shadows && !grass,
                });
            }
        }
        objects
    }

    fn pipelines(&mut self, rhi: &B, format: TextureFormat) {
        if self.pipelines.as_ref().is_some_and(|p| p.format == format) {
            return;
        }
        let layout = prop_layout();
        let water_layout = [VertexLayout { stride: 36, step: VertexStep::Vertex, attrs: &WATER_ATTRS }];
        let desc = |label, shader, vs_entry, fs_entry, vertex_buffers, blend, cull, depth| PipelineDesc::<B> {
            label,
            shader,
            vs_entry,
            fs_entry,
            vertex_buffers,
            color_format: Some(format),
            blend,
            topology: Topology::TriangleList,
            cull,
            depth: Some(depth),
            samples: 1,
        };
        let no_write = |compare| DepthState { format: DEPTH, write: false, compare, bias: 0, slope_bias: 0.0 };
        self.pipelines = Some(Pipelines {
            format,
            props: rhi.create_pipeline(&desc("outdoor props", &self.prop_shader, "vs_prop", "fs_prop", &layout, Blend::Opaque, Cull::None, DepthState::opaque(DEPTH))),
            sky: rhi.create_pipeline(&desc("outdoor sky", &self.sky_shader, "vs_sky", "fs_sky", &[], Blend::Opaque, Cull::None, no_write(Compare::LessEqual))),
            water: rhi.create_pipeline(&desc("outdoor water", &self.water_shader, "vs_water", "fs_water", &water_layout, Blend::Alpha, Cull::None, no_write(Compare::Less))),
        });
        self.binds = None;
        for water in &mut self.waters {
            water.bind = None;
        }
    }

    fn globals(&self, frame: &PassFrame<'_, B>) -> Globals {
        frame_globals(frame, self.scene.sky, self.time)
    }

    /// Sorts the objects into draws by mesh: culled to `planes`, with the level of detail from the distance to `eye`.
    fn sort(&self, planes: &[[f32; 4]; 6], eye: V3, shadow: bool, out: &mut Vec<(u32, InstanceRaw)>) {
        for o in &self.objects {
            if shadow && !o.casts {
                continue;
            }
            let p = [o.raw.place[0], o.raw.place[1], o.raw.place[2]];
            let distance = length(sub(p, eye));
            if distance > if o.grass { GRASS_DISTANCE } else { FAR_DISTANCE } || !sphere_visible(planes, add(p, [0.0, o.radius * 0.5, 0.0]), o.radius) {
                continue;
            }
            let level = if shadow {
                if distance > SHADOW_LOD_DISTANCE { 2 } else { 1 }
            } else if distance < LOD_DISTANCE[0] {
                0
            } else if distance < LOD_DISTANCE[1] {
                1
            } else {
                2
            };
            out.push((o.meshes[level], o.raw));
        }
    }
}

/// The scenery shaders' uniforms for one frame of the imported scene.
pub(crate) fn frame_globals<B: Rhi>(frame: &PassFrame<'_, B>, sky: Option<OutdoorSky>, time: f32) -> Globals {
    let camera = frame.camera;
    let forward = normalize(sub(camera.target, camera.eye));
    let right = normalize(cross(forward, camera.up));
    let up = cross(right, forward);
    let aspect = frame.size.0 as f32 / frame.size.1.max(1) as f32;
    let tangent = match camera.projection {
        Projection::Perspective { fov_y, .. } => (fov_y * 0.5).tan(),
        Projection::Orthographic { .. } => 0.6,
    };
    let wide = |v: V3, w: f32| [v[0], v[1], v[2], w];
    let lighting = frame.lighting;
    let toward = normalize(scale(lighting.direction, -1.0));
    let sky = sky.unwrap_or(OutdoorSky { brightness: 1.5, sun_glow: 1.0, cloud_cover: 0.34, cloud_speed: 1.0, wind_strength: 1.0, wind_speed: 1.0 });
    let ambient = |c: V3| scale(c, lighting.ambient);
    let (shadow0, cascades, shadow) = match &frame.shadow {
        Some(s) => (s.light_vp, [s.params[0], 0.0, s.params[1], s.params[2]], [1.5, 1.0, 0.0, 0.0]),
        None => ([[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]], [0.0; 4], [1.5, 0.0, 0.0, 0.0]),
    };
    Globals {
        view_proj: frame.view_proj,
        shadow0,
        shadow1: shadow0,
        cam_right: wide(scale(right, tangent * aspect), 0.0),
        cam_up: wide(scale(up, tangent), 0.0),
        cam_forward: wide(forward, 0.0),
        camera: wide(camera.eye, time),
        sun_dir: wide(toward, lighting.intensity),
        sun_color: wide(lighting.color, 0.0),
        ambient_sky: wide(ambient(lighting.sky), 0.0),
        ambient_ground: wide(ambient(lighting.ground), 0.0),
        // Fog is the post chain's height fog, the same for every object.
        fog: [0.0; 4],
        viewport: [frame.size.0 as f32, frame.size.1 as f32, 0.0, GRASS_DISTANCE],
        cascades,
        sky: [sky.brightness, sky.sun_glow, 0.86 - 0.7 * sky.cloud_cover, sky.cloud_speed],
        shadow,
        wind: [sky.wind_strength, sky.wind_speed, 0.0, 0.0],
        water_color: [0.012, 0.05, 0.04, 1.0],
        water: [1.0, 1.0, 1.0, 1.0],
        leaf_tint: [1.0, 1.0, 1.0, 1.0],
        detail: [1.0, 0.14, 0.9, 0.5],
    }
}

fn water_mesh<B: Rhi>(rhi: &B, water: &OutdoorWater) -> WaterMesh<B> {
    let cells = [((water.size[0] / 4.0).ceil() as u32).clamp(1, 128), ((water.size[1] / 4.0).ceil() as u32).clamp(1, 128)];
    let mut vertices = Vec::new();
    for j in 0..=cells[1] {
        for i in 0..=cells[0] {
            let x = water.center[0] + (i as f32 / cells[0] as f32 - 0.5) * water.size[0];
            let z = water.center[2] + (j as f32 / cells[1] as f32 - 0.5) * water.size[1];
            let depth = water.ground.as_ref().and_then(|g| g.ground(x, z)).map_or(10.0, |ground| water.center[1] - ground);
            vertices.push(WaterVertex { position: [x, water.center[1], z], data: [depth, 0.0, 0.0, 0.0], flow: if length([water.flow[0], 0.0, water.flow[1]]) > 1e-4 { water.flow } else { [0.0, 1.0] } });
        }
    }
    let mut indices = Vec::new();
    let row = cells[0] + 1;
    for j in 0..cells[1] {
        for i in 0..cells[0] {
            let a = j * row + i;
            indices.extend_from_slice(&[a, a + row, a + 1, a + 1, a + row, a + row + 1]);
        }
    }
    WaterMesh {
        vertices: buffer_with(rhi, "outdoor water vertices", BufferUsage::VERTEX, bytemuck::cast_slice(&vertices)),
        indices: buffer_with(rhi, "outdoor water indices", BufferUsage::INDEX, bytemuck::cast_slice(&indices)),
        count: indices.len() as u32,
        globals: rhi.create_buffer(&BufferDesc { label: "outdoor water globals", size: std::mem::size_of::<Globals>() as u64, usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST }),
        color: water.color,
        clarity: water.clarity,
        ripples: water.ripples,
        glint: water.glint,
        bind: None,
    }
}

impl<B: Rhi> RenderPass<B> for OutdoorPass<B> {
    fn name(&self) -> &str {
        "outdoor"
    }

    fn stages(&self) -> &[PassStage] {
        &[PassStage::Shadow, PassStage::Opaque, PassStage::Sky, PassStage::Transparent]
    }

    fn prepare(&mut self, rhi: &B, frame: &PassFrame<'_, B>) {
        self.pipelines(rhi, frame.color_format);
        let globals = self.globals(frame);
        rhi.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        let mut shadow_globals = globals;
        if let Some(shadow) = &frame.shadow {
            shadow_globals.view_proj = shadow.light_vp;
        }
        rhi.write_buffer(&self.shadow_globals, 0, bytemuck::bytes_of(&shadow_globals));
        for water in &self.waters {
            let mut g = globals;
            g.water_color = [water.color[0], water.color[1], water.color[2], water.clarity];
            g.water = [water.ripples, 1.0, 1.0, water.glint];
            rhi.write_buffer(&water.globals, 0, bytemuck::bytes_of(&g));
        }

        // Bind groups hold the shadow map; they are made again when it changes.
        let map_id = frame.shadow.as_ref().map_or(0, |s| s.map_id);
        let pipelines = self.pipelines.as_ref().expect("made above");
        if self.binds.as_ref().is_none_or(|b| b.map_id != map_id) || self.waters.iter().any(|w| w.bind.is_none()) {
            let map = frame.shadow.as_ref().map_or(&self.no_shadow.1, |s| s.view);
            let lit = |pipeline: &B::Pipeline, globals: &B::Buffer| {
                rhi.create_bind_group(pipeline, 0, &[Binding::Uniform { binding: 0, buffer: globals }, Binding::Texture { binding: 1, view: map }, Binding::Sampler { binding: 3, sampler: &self.shadow_sampler }])
            };
            let props = lit(&pipelines.props, &self.globals);
            let sky = rhi.create_bind_group(&pipelines.sky, 0, &[Binding::Uniform { binding: 0, buffer: &self.globals }]);
            let shadow = rhi.create_bind_group(&self.prop_shadow, 0, &[Binding::Uniform { binding: 0, buffer: &self.shadow_globals }]);
            let water_binds: Vec<B::BindGroup> = self.waters.iter().map(|w| lit(&pipelines.water, &w.globals)).collect();
            for (water, bind) in self.waters.iter_mut().zip(water_binds) {
                water.bind = Some(bind);
            }
            self.binds = Some(Binds { map_id, props, sky, shadow });
        }

        // Cull and sort the objects for the camera and for the shadow map.
        let mut main = Vec::new();
        self.sort(&frustum_planes(&frame.view_proj), frame.camera.eye, false, &mut main);
        let mut cast = Vec::new();
        if let Some(shadow) = &frame.shadow {
            self.sort(&frustum_planes(&shadow.light_vp), frame.camera.eye, true, &mut cast);
        }
        main.sort_unstable_by_key(|(mesh, _)| *mesh);
        cast.sort_unstable_by_key(|(mesh, _)| *mesh);
        let ranges = |list: &[(u32, InstanceRaw)], offset: u32| {
            let mut out: Vec<(u32, u32, u32)> = Vec::new();
            for (i, (mesh, _)) in list.iter().enumerate() {
                match out.last_mut() {
                    Some(last) if last.0 == *mesh => last.2 += 1,
                    _ => out.push((*mesh, offset + i as u32, 1)),
                }
            }
            out
        };
        self.draws = ranges(&main, 0);
        self.shadow_draws = ranges(&cast, main.len() as u32);
        let raw: Vec<InstanceRaw> = main.iter().chain(&cast).map(|(_, r)| *r).collect();
        let bytes = (raw.len() * std::mem::size_of::<InstanceRaw>()) as u64;
        if self.instances.as_ref().is_none_or(|(_, size)| *size < bytes) {
            let size = bytes.next_power_of_two().max(1024);
            self.instances = Some((rhi.create_buffer(&BufferDesc { label: "outdoor instances", size, usage: BufferUsage::VERTEX | BufferUsage::COPY_DST }), size));
        }
        if !raw.is_empty() {
            rhi.write_buffer(&self.instances.as_ref().expect("made above").0, 0, bytemuck::cast_slice(&raw));
        }
    }

    fn encode(&self, rhi: &B, stage: PassStage, frame: &PassFrame<'_, B>, encoder: &mut B::Encoder, color: Option<&ColorAttachment<'_, B>>, depth: &DepthAttachment<'_, B>) {
        let (Some(pipelines), Some(Binds { props: props_bind, sky: sky_bind, shadow: shadow_bind, .. })) = (&self.pipelines, &self.binds) else { return };
        let props = |pipeline, bind, draws: &[(u32, u32, u32)]| {
            let Some((instances, _)) = &self.instances else { return Vec::new() };
            let mut commands = vec![Command::SetPipeline(pipeline), Command::SetBindGroup(0, bind), Command::SetVertexBuffer(0, &self.prop_vertices), Command::SetVertexBuffer(1, instances), Command::SetIndexBuffer(&self.prop_indices)];
            for (mesh, first, count) in draws {
                let m = &self.meshes[*mesh as usize];
                commands.push(Command::DrawIndexed { indices: m.first_index..m.first_index + m.index_count, base_vertex: m.base_vertex, instances: *first..*first + *count });
            }
            commands
        };
        match stage {
            PassStage::Shadow if frame.shadow.is_some() && !self.shadow_draws.is_empty() => {
                rhi.encode_pass(encoder, "outdoor shadow casters", None, Some(depth), &props(&self.prop_shadow, shadow_bind, &self.shadow_draws));
            }
            PassStage::Opaque if !self.draws.is_empty() => {
                rhi.encode_pass(encoder, "outdoor vegetation", color, Some(depth), &props(&pipelines.props, props_bind, &self.draws));
            }
            PassStage::Sky if self.scene.sky.is_some() => {
                rhi.encode_pass(encoder, "outdoor sky", color, Some(depth), &[Command::SetPipeline(&pipelines.sky), Command::SetBindGroup(0, sky_bind), Command::Draw { vertices: 0..3, instances: 0..1 }]);
            }
            PassStage::Transparent if !self.waters.is_empty() => {
                let mut commands = vec![Command::SetPipeline(&pipelines.water)];
                for water in &self.waters {
                    let Some(bind) = &water.bind else { continue };
                    commands.extend([Command::SetBindGroup(0, bind), Command::SetVertexBuffer(0, &water.vertices), Command::SetIndexBuffer(&water.indices), Command::DrawIndexed { indices: 0..water.count, base_vertex: 0, instances: 0..1 }]);
                }
                rhi.encode_pass(encoder, "outdoor water", color, Some(depth), &commands);
            }
            _ => {}
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(height: f32) -> HeightField {
        HeightField { center: [0.0, 0.0, 0.0], size: [100.0, 100.0], height, width: 3, depth: 3, heights: Arc::new(vec![0.5; 9]) }
    }

    #[test]
    fn a_height_field_gives_the_ground_and_its_normal() {
        let ramp = HeightField { heights: Arc::new(vec![0.0, 0.5, 1.0, 0.0, 0.5, 1.0, 0.0, 0.5, 1.0]), ..flat(10.0) };
        assert_eq!(ramp.ground(0.0, 0.0), Some(5.0));
        assert_eq!(ramp.ground(60.0, 0.0), None);
        let n = ramp.normal(0.0, 0.0);
        assert!(n[0] < 0.0 && n[1] > 0.9, "{n:?}");
    }
}
