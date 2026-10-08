//! Bounded CPU-only, static, sun-lit, single-bounce diffuse irradiance baking.
//!
//! Probe rays cover the **full sphere**, with weight 4π/N. A miss is black. At
//! the nearest visible opaque surface, outgoing radiance is
//! `rho / pi * E_sun * max(dot(n, l), 0) * secondary_visibility`, where
//! `E_sun = pi * sun.color * sun.intensity` matches the renderer's stylized sun.
//! Ambient, sky, point lights, emission, specular and existing probes never
//! enter this calculation. Radiance SH is cosine-convolved exactly once, with
//! band factors π, 2π/3 and π/4, into the existing positive real SH9 basis.
//!
//! Triangle winding determines the front face. Primary rays cull one-sided
//! backfaces; two-sided surfaces orient both normals toward the incoming ray.
//! Secondary visibility treats all triangles as opaque on both sides. Geometric
//! normals determine the offset and light-facing hemisphere; interpolated
//! shading normals determine Lambertian cosine. This avoids shading-normal
//! back-side leaks. Intersections and accumulation use f64; source positions
//! remain the exact snapshot's f32 values. The offset scales with hit coordinate
//! and triangle edge magnitude, rather than an arbitrary scene-sized bias.
//!
//! This module owns no GPU, scene, file or job state. Every error, cancellation,
//! elapsed-time limit or numerical failure returns **no coefficients**. The
//! caller alone publishes a successfully completed grid atomically.
#![allow(clippy::float_arithmetic)]

use crate::irradiance::{sh_basis, IrradianceGrid, Sh9, MAX_COEFFICIENT, MAX_PROBES, WORLD_LIMIT};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

pub const BAKE_ALGORITHM_VERSION: u32 = 1;
pub const MAX_BAKE_TRIANGLES: usize = 8192;
pub const MAX_BAKE_RAYS_PER_PROBE: u32 = 2048;
pub const MAX_BAKE_TEXTURE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_BAKE_TRIANGLE_TESTS: u64 = 32_000_000;
pub const MAX_BAKE_NODE_VISITS: u64 = 64_000_000;
pub const MAX_BAKE_DURATION: Duration = Duration::from_secs(30);
/// Multiplier applied to world-coordinate and edge magnitudes in f64.
pub const RAY_OFFSET_FACTOR: f64 = 128.0 * f64::EPSILON;
pub const MIN_RAY_OFFSET: f64 = 1.0e-9;
const MAX_TEXTURE_EXTENT: u32 = 4096;
const MAX_BVH_DEPTH: usize = 32;
const LEAF_TRIANGLES: usize = 4;
const PI: f64 = std::f64::consts::PI;
type V3 = [f64; 3];

#[derive(Clone, Debug)]
pub struct BakeScene {
    pub triangles: Vec<BakeTriangle>,
    pub materials: Vec<BakeMaterial>,
    pub sun: BakeSun,
}

#[derive(Clone, Debug)]
pub struct BakeTriangle {
    pub positions: [[f32; 3]; 3],
    pub normals: [[f32; 3]; 3],
    pub texcoords: [[f32; 2]; 3],
    pub material: usize,
}

#[derive(Clone, Debug)]
pub struct BakeMaterial {
    /// Linear diffuse reflectance, including the procedural (1 - metallic) term.
    pub base_color: [f32; 3],
    /// Opaque base color: alpha is ignored, as in the imported-model renderer.
    pub texture: Option<BakeTexture>,
    pub checker: Option<BakeChecker>,
    pub double_sided: bool,
}
impl Default for BakeMaterial {
    fn default() -> Self {
        Self {
            base_color: [1.0; 3],
            texture: None,
            checker: None,
            double_sided: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BakeWrap {
    Clamp,
    Repeat,
    Mirror,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BakeFilter {
    Nearest,
    Linear,
}

#[derive(Clone, Debug)]
pub struct BakeTexture {
    pub width: u32,
    pub height: u32,
    /// RGB is sRGB; every tap is decoded *before* linear-space interpolation.
    pub rgba8_srgb: Vec<u8>,
    pub wrap_s: BakeWrap,
    pub wrap_t: BakeWrap,
    pub filter: BakeFilter,
}

#[derive(Clone, Copy, Debug)]
pub struct BakeChecker {
    /// World-xz checker frequency. The procedural renderer uses 0.5.
    pub scale: f32,
    /// Multiplicative factor on odd cells. The procedural renderer uses 0.72.
    pub dark_multiplier: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct BakeSun {
    /// World direction **towards** the sun, opposite renderer light travel.
    pub direction: [f32; 3],
    /// Linear RGB, not sRGB.
    pub color: [f32; 3],
    pub intensity: f32,
}

#[derive(Clone, Debug)]
pub struct BakeSettings {
    /// An even count in 64..=2048. Antipodal Fibonacci pairs cover the sphere.
    pub rays_per_probe: u32,
    /// Total across all primary and secondary rays, never a per-ray allowance.
    pub max_triangle_tests: u64,
    pub max_node_visits: u64,
    /// Includes input validation, BVH assembly and tracing. At most 30 seconds.
    pub max_duration: Duration,
}
impl Default for BakeSettings {
    fn default() -> Self {
        Self {
            rays_per_probe: 1024,
            max_triangle_tests: MAX_BAKE_TRIANGLE_TESTS,
            max_node_visits: MAX_BAKE_NODE_VISITS,
            max_duration: Duration::from_secs(10),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BakeError {
    InvalidInput(String),
    Cancelled,
    WorkLimit(&'static str),
    TimedOut,
}
impl std::fmt::Display for BakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(reason) => write!(f, "invalid static diffuse bake: {reason}"),
            Self::Cancelled => f.write_str("static diffuse bake cancelled"),
            Self::WorkLimit(kind) => write!(f, "static diffuse bake {kind} budget exceeded"),
            Self::TimedOut => f.write_str("static diffuse bake elapsed-time budget exceeded"),
        }
    }
}
impl std::error::Error for BakeError {}
fn invalid(reason: &str) -> BakeError {
    BakeError::InvalidInput(reason.into())
}

/// Returns complete validated SH irradiance coefficients, in x-fastest order.
/// The input grid's coefficients are validated but are never a lighting source.
pub fn bake(
    scene: &BakeScene,
    grid: &IrradianceGrid,
    settings: &BakeSettings,
    cancelled: &AtomicBool,
) -> Result<Vec<Sh9>, BakeError> {
    bake_with_progress(scene, grid, settings, cancelled, &AtomicU32::new(0))
}

/// Progress is the number of fully traced probes, from zero to the grid count.
/// Progress does not grant access to partial coefficients and is reset on entry.
pub fn bake_with_progress(
    scene: &BakeScene,
    grid: &IrradianceGrid,
    settings: &BakeSettings,
    cancelled: &AtomicBool,
    completed_probes: &AtomicU32,
) -> Result<Vec<Sh9>, BakeError> {
    bake_internal(scene, grid, settings, cancelled, completed_probes, |_| {})
}

// The private observer makes cancellation-after-progress tests deterministic.
// Production instantiates a no-op observer; no callback can alter scene data.
fn bake_internal(
    scene: &BakeScene,
    grid: &IrradianceGrid,
    settings: &BakeSettings,
    cancelled: &AtomicBool,
    completed_probes: &AtomicU32,
    mut after_probe: impl FnMut(u32),
) -> Result<Vec<Sh9>, BakeError> {
    completed_probes.store(0, Ordering::Relaxed);
    let mut control = Control::new(settings, cancelled)?;
    control.check()?;
    grid.validate().map_err(BakeError::InvalidInput)?;
    let prepared = PreparedScene::new(scene, &mut control)?;
    let directions = sphere_directions(settings.rays_per_probe);
    let mut coefficients = Vec::with_capacity(grid.coefficients.len());
    for z in 0..grid.dimensions[2] {
        for y in 0..grid.dimensions[1] {
            for x in 0..grid.dimensions[0] {
                control.check()?;
                // Match the existing grid's representable f32 node positions.
                let indices = [x, y, z];
                let origin = std::array::from_fn(|axis| {
                    (grid.origin[axis] + grid.spacing[axis] * indices[axis] as f32) as f64
                });
                let mut sh = [[0.0; 3]; 9];
                for &direction in &directions {
                    control.check()?;
                    let radiance = prepared.radiance(scene, origin, direction, &mut control)?;
                    project_sample(
                        &mut sh,
                        direction,
                        radiance,
                        4.0 * PI / directions.len() as f64,
                    );
                }
                coefficients.push(finish_projection(sh)?);
                completed_probes.store(coefficients.len() as u32, Ordering::Relaxed);
                after_probe(coefficients.len() as u32);
            }
        }
    }
    control.check()?;
    if coefficients.len() > MAX_PROBES {
        return Err(invalid("too many probes"));
    }
    Ok(coefficients)
}

struct Control<'a> {
    settings: &'a BakeSettings,
    cancelled: &'a AtomicBool,
    start: Instant,
    triangle_tests: u64,
    node_visits: u64,
}
impl<'a> Control<'a> {
    fn new(settings: &'a BakeSettings, cancelled: &'a AtomicBool) -> Result<Self, BakeError> {
        if !(64..=MAX_BAKE_RAYS_PER_PROBE).contains(&settings.rays_per_probe)
            || settings.rays_per_probe % 2 != 0
            || settings.max_triangle_tests == 0
            || settings.max_triangle_tests > MAX_BAKE_TRIANGLE_TESTS
            || settings.max_node_visits == 0
            || settings.max_node_visits > MAX_BAKE_NODE_VISITS
            || settings.max_duration.is_zero()
            || settings.max_duration > MAX_BAKE_DURATION
        {
            return Err(invalid(
                "settings exceed the bounded ray, traversal or elapsed-time limits",
            ));
        }
        Ok(Self {
            settings,
            cancelled,
            start: Instant::now(),
            triangle_tests: 0,
            node_visits: 0,
        })
    }
    fn check(&self) -> Result<(), BakeError> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(BakeError::Cancelled);
        }
        if self.start.elapsed() >= self.settings.max_duration {
            return Err(BakeError::TimedOut);
        }
        Ok(())
    }
    fn triangle(&mut self) -> Result<(), BakeError> {
        if self.triangle_tests >= self.settings.max_triangle_tests {
            return Err(BakeError::WorkLimit("triangle-test"));
        }
        self.triangle_tests += 1;
        if self.triangle_tests % 64 == 0 {
            self.check()?;
        }
        Ok(())
    }
    fn node(&mut self) -> Result<(), BakeError> {
        if self.node_visits >= self.settings.max_node_visits {
            return Err(BakeError::WorkLimit("BVH traversal"));
        }
        self.node_visits += 1;
        if self.node_visits % 64 == 0 {
            self.check()?;
        }
        Ok(())
    }
}

fn add(a: V3, b: V3) -> V3 {
    std::array::from_fn(|i| a[i] + b[i])
}
fn sub(a: V3, b: V3) -> V3 {
    std::array::from_fn(|i| a[i] - b[i])
}
fn mul(a: V3, s: f64) -> V3 {
    a.map(|v| v * s)
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalize(a: V3) -> Option<V3> {
    let magnitude = dot(a, a).sqrt();
    (magnitude.is_finite() && magnitude > 0.0).then(|| mul(a, magnitude.recip()))
}
fn v64(a: [f32; 3]) -> V3 {
    a.map(f64::from)
}

#[derive(Clone, Copy)]
struct Bounds {
    min: V3,
    max: V3,
}
impl Bounds {
    fn empty() -> Self {
        Self {
            min: [f64::INFINITY; 3],
            max: [f64::NEG_INFINITY; 3],
        }
    }
    fn include(&mut self, point: V3) {
        for (axis, p) in point.into_iter().enumerate() {
            self.min[axis] = self.min[axis].min(p);
            self.max[axis] = self.max[axis].max(p);
        }
    }
    fn intersects(&self, origin: V3, direction: V3, maximum: f64) -> bool {
        let mut near: f64 = 0.0;
        let mut far = maximum;
        for axis in 0..3 {
            if direction[axis] == 0.0 {
                if origin[axis] < self.min[axis] || origin[axis] > self.max[axis] {
                    return false;
                }
            } else {
                let a = (self.min[axis] - origin[axis]) / direction[axis];
                let b = (self.max[axis] - origin[axis]) / direction[axis];
                near = near.max(a.min(b));
                far = far.min(a.max(b));
                if far < near {
                    return false;
                }
            }
        }
        far >= near
    }
}

struct PreparedTriangle {
    vertices: [V3; 3],
    geometric_normal: V3,
    max_edge: f64,
    bounds: Bounds,
    centroid: V3,
}
struct Node {
    bounds: Bounds,
    kind: NodeKind,
}
enum NodeKind {
    Leaf { start: usize, end: usize },
    Branch { left: usize, right: usize },
}
struct PreparedScene {
    triangles: Vec<PreparedTriangle>,
    order: Vec<usize>,
    nodes: Vec<Node>,
    sun_direction: V3,
}
#[derive(Clone, Copy, Debug)]
struct Hit {
    triangle: usize,
    distance: f64,
    u: f64,
    v: f64,
}

impl PreparedScene {
    fn new(scene: &BakeScene, control: &mut Control<'_>) -> Result<Self, BakeError> {
        if scene.triangles.len() > MAX_BAKE_TRIANGLES || scene.materials.len() > MAX_BAKE_TRIANGLES
        {
            return Err(invalid("scene exceeds 8192 triangles or materials"));
        }
        if !scene.sun.direction.iter().all(|x| x.is_finite())
            || !scene.sun.color.iter().all(|x| x.is_finite() && *x >= 0.0)
            || !scene.sun.intensity.is_finite()
            || scene.sun.intensity < 0.0
        {
            return Err(invalid(
                "sun must have finite direction and nonnegative finite RGB/intensity",
            ));
        }
        let sun_direction =
            normalize(v64(scene.sun.direction)).ok_or_else(|| invalid("sun direction is zero"))?;
        let mut texture_bytes = 0usize;
        for material in &scene.materials {
            control.check()?;
            if !material
                .base_color
                .iter()
                .all(|x| x.is_finite() && (0.0..=1.0).contains(x))
            {
                return Err(invalid("diffuse reflectance must be finite in 0..=1"));
            }
            if let Some(checker) = material.checker {
                if !checker.scale.is_finite()
                    || !(0.0..=WORLD_LIMIT).contains(&checker.scale)
                    || !checker.dark_multiplier.is_finite()
                    || !(0.0..=1.0).contains(&checker.dark_multiplier)
                {
                    return Err(invalid(
                        "invalid checker frequency or reflectance multiplier",
                    ));
                }
            }
            if let Some(texture) = &material.texture {
                if texture.width == 0
                    || texture.height == 0
                    || texture.width > MAX_TEXTURE_EXTENT
                    || texture.height > MAX_TEXTURE_EXTENT
                    || texture.rgba8_srgb.len()
                        != texture.width as usize * texture.height as usize * 4
                {
                    return Err(invalid(
                        "texture must be a complete RGBA8 image at most 4096 by 4096",
                    ));
                }
                texture_bytes = texture_bytes
                    .checked_add(texture.rgba8_srgb.len())
                    .ok_or_else(|| invalid("texture byte count overflow"))?;
                if texture_bytes > MAX_BAKE_TEXTURE_BYTES {
                    return Err(invalid("texture data exceeds 64 MiB"));
                }
            }
        }
        let mut triangles = Vec::with_capacity(scene.triangles.len());
        for triangle in &scene.triangles {
            control.check()?;
            if triangle.material >= scene.materials.len()
                || !triangle
                    .positions
                    .iter()
                    .flatten()
                    .all(|p| p.is_finite() && p.abs() <= WORLD_LIMIT)
                || !triangle
                    .normals
                    .iter()
                    .all(|n| n.iter().all(|x| x.is_finite()) && normalize(v64(*n)).is_some())
                || !triangle
                    .texcoords
                    .iter()
                    .flatten()
                    .all(|uv| uv.is_finite() && uv.abs() <= WORLD_LIMIT)
            {
                return Err(invalid(
                    "triangle has invalid material, position, normal or UV",
                ));
            }
            let vertices = triangle.positions.map(v64);
            let e1 = sub(vertices[1], vertices[0]);
            let e2 = sub(vertices[2], vertices[0]);
            let geometric_normal =
                normalize(cross(e1, e2)).ok_or_else(|| invalid("degenerate triangle"))?;
            let max_edge = [e1, e2, sub(vertices[2], vertices[1])]
                .iter()
                .map(|e| dot(*e, *e).sqrt())
                .fold(0.0_f64, f64::max);
            let mut bounds = Bounds::empty();
            for p in vertices {
                bounds.include(p);
            }
            triangles.push(PreparedTriangle {
                vertices,
                geometric_normal,
                max_edge,
                bounds,
                centroid: mul(add(add(vertices[0], vertices[1]), vertices[2]), 1.0 / 3.0),
            });
        }
        let mut prepared = Self {
            order: (0..triangles.len()).collect(),
            nodes: Vec::with_capacity(triangles.len() * 2),
            triangles,
            sun_direction,
        };
        if !prepared.triangles.is_empty() {
            prepared.build(0, prepared.triangles.len(), 0, control)?;
        }
        control.check()?;
        Ok(prepared)
    }
    fn build(
        &mut self,
        start: usize,
        end: usize,
        depth: usize,
        control: &mut Control<'_>,
    ) -> Result<usize, BakeError> {
        control.check()?;
        if depth >= MAX_BVH_DEPTH {
            return Err(BakeError::WorkLimit("BVH depth"));
        }
        let mut bounds = Bounds::empty();
        let mut centers = Bounds::empty();
        for &index in &self.order[start..end] {
            bounds.include(self.triangles[index].bounds.min);
            bounds.include(self.triangles[index].bounds.max);
            centers.include(self.triangles[index].centroid);
        }
        let node = self.nodes.len();
        self.nodes.push(Node {
            bounds,
            kind: NodeKind::Leaf { start, end },
        });
        if end - start > LEAF_TRIANGLES {
            let extent = sub(centers.max, centers.min);
            let mut axis = 0;
            for candidate in 1..3 {
                if extent[candidate] > extent[axis] {
                    axis = candidate;
                }
            }
            let triangles = &self.triangles;
            self.order[start..end].sort_unstable_by(|&a, &b| {
                triangles[a].centroid[axis]
                    .total_cmp(&triangles[b].centroid[axis])
                    .then(a.cmp(&b))
            });
            control.check()?;
            let middle = start + (end - start) / 2;
            let left = self.build(start, middle, depth + 1, control)?;
            let right = self.build(middle, end, depth + 1, control)?;
            self.nodes[node].kind = NodeKind::Branch { left, right };
        }
        Ok(node)
    }
    fn hit(
        &self,
        scene: &BakeScene,
        origin: V3,
        direction: V3,
        secondary: bool,
        control: &mut Control<'_>,
    ) -> Result<Option<Hit>, BakeError> {
        if self.nodes.is_empty() {
            return Ok(None);
        }
        let mut stack = [0usize; MAX_BVH_DEPTH];
        let mut count = 1;
        let mut nearest: Option<Hit> = None;
        while count != 0 {
            count -= 1;
            let node = &self.nodes[stack[count]];
            control.node()?;
            let maximum = nearest.map_or(f64::INFINITY, |hit| hit.distance);
            if !node.bounds.intersects(origin, direction, maximum) {
                continue;
            }
            match node.kind {
                NodeKind::Leaf { start, end } => {
                    for &index in &self.order[start..end] {
                        control.triangle()?;
                        let cull = !secondary
                            && !scene.materials[scene.triangles[index].material].double_sided;
                        if let Some(mut hit) =
                            intersection(&self.triangles[index], origin, direction, cull)
                        {
                            hit.triangle = index;
                            if secondary {
                                return Ok(Some(hit));
                            }
                            if nearest.is_none_or(|old| {
                                hit.distance < old.distance
                                    || (hit.distance == old.distance && index < old.triangle)
                            }) {
                                nearest = Some(hit);
                            }
                        }
                    }
                }
                NodeKind::Branch { left, right } => {
                    if count + 2 > stack.len() {
                        return Err(BakeError::WorkLimit("BVH traversal stack"));
                    }
                    stack[count] = right;
                    stack[count + 1] = left;
                    count += 2;
                }
            }
        }
        Ok(nearest)
    }
    fn radiance(
        &self,
        scene: &BakeScene,
        origin: V3,
        direction: V3,
        control: &mut Control<'_>,
    ) -> Result<V3, BakeError> {
        let Some(hit) = self.hit(scene, origin, direction, false, control)? else {
            return Ok([0.0; 3]);
        };
        let triangle = &scene.triangles[hit.triangle];
        let geometric = &self.triangles[hit.triangle];
        let material = &scene.materials[triangle.material];
        let bary = [1.0 - hit.u - hit.v, hit.u, hit.v];
        let mut normal = [0.0; 3];
        let mut uv = [0.0; 2];
        for (vertex, weight) in bary.into_iter().enumerate() {
            normal = add(normal, mul(v64(triangle.normals[vertex]), weight));
            for (axis, value) in uv.iter_mut().enumerate() {
                *value += triangle.texcoords[vertex][axis] as f64 * weight;
            }
        }
        let Some(mut normal) = normalize(normal) else {
            return Err(invalid("interpolated shading normal is zero"));
        };
        let mut geometric_normal = geometric.geometric_normal;
        if material.double_sided && dot(geometric_normal, direction) > 0.0 {
            geometric_normal = mul(geometric_normal, -1.0);
            normal = mul(normal, -1.0);
        }
        // An inconsistent supplied shading normal is an unsupported snapshot,
        // never silently flipped or clamped into the geometric hemisphere.
        if dot(normal, geometric_normal) < -1.0e-6 {
            return Err(invalid("shading normal opposes triangle winding"));
        }
        let cosine = dot(normal, self.sun_direction).max(0.0);
        if cosine == 0.0
            || dot(geometric_normal, self.sun_direction) <= 0.0
            || scene.sun.intensity == 0.0
        {
            return Ok([0.0; 3]);
        }
        let point = add(origin, mul(direction, hit.distance));
        let magnitude = point.iter().map(|x| x.abs()).fold(0.0_f64, f64::max);
        let offset = (RAY_OFFSET_FACTOR * (magnitude + geometric.max_edge)).max(MIN_RAY_OFFSET);
        let shadow_origin = add(point, mul(geometric_normal, offset));
        if self
            .hit(scene, shadow_origin, self.sun_direction, true, control)?
            .is_some()
        {
            return Ok([0.0; 3]);
        }
        let rho = material.reflectance(uv, point);
        // Keep the radiometric mapping explicit: E_sun = pi * stylized_sun.
        let e_sun = scene
            .sun
            .color
            .map(|c| PI * c as f64 * scene.sun.intensity as f64);
        let radiance = std::array::from_fn(|channel| rho[channel] / PI * e_sun[channel] * cosine);
        if !radiance.iter().all(|x| x.is_finite() && *x >= 0.0) {
            return Err(invalid("nonfinite outgoing radiance"));
        }
        Ok(radiance)
    }
}

fn intersection(triangle: &PreparedTriangle, origin: V3, direction: V3, cull: bool) -> Option<Hit> {
    let edge1 = sub(triangle.vertices[1], triangle.vertices[0]);
    let edge2 = sub(triangle.vertices[2], triangle.vertices[0]);
    let p = cross(direction, edge2);
    let determinant = dot(edge1, p);
    // Relative angular rejection, independent of the triangle's world scale.
    let threshold = dot(cross(edge1, edge2), cross(edge1, edge2)).sqrt() * 1.0e-12;
    if (cull && determinant <= threshold) || (!cull && determinant.abs() <= threshold) {
        return None;
    }
    let inverse = determinant.recip();
    let relative = sub(origin, triangle.vertices[0]);
    let u = dot(relative, p) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(relative, edge1);
    let v = dot(direction, q) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let distance = dot(edge2, q) * inverse;
    if distance <= 0.0 || !distance.is_finite() {
        return None;
    }
    Some(Hit {
        triangle: 0,
        distance,
        u,
        v,
    })
}

/// Antipodal Fibonacci quadrature: deterministic equal-solid-angle weights,
/// no importance-PDF division, no cosine term on the probe ray itself.
fn sphere_directions(count: u32) -> Vec<V3> {
    let mut directions = Vec::with_capacity(count as usize);
    let half = count / 2;
    let golden_angle = PI * (3.0 - 5.0_f64.sqrt());
    for i in 0..half {
        let z = (i as f64 + 0.5) / half as f64;
        let radius = (1.0 - z * z).sqrt();
        let phi = golden_angle * i as f64;
        let direction = [radius * phi.cos(), radius * phi.sin(), z];
        directions.push(direction);
        directions.push(mul(direction, -1.0));
    }
    directions
}
fn project_sample(coefficients: &mut [[f64; 3]; 9], direction: V3, radiance: V3, weight: f64) {
    // Use the consumer's exact basis convention and constants.
    let basis = sh_basis(direction.map(|v| v as f32)).expect("unit quadrature direction");
    for (coefficient, value) in coefficients.iter_mut().zip(basis) {
        for channel in 0..3 {
            coefficient[channel] += radiance[channel] * value as f64 * weight;
        }
    }
}
fn finish_projection(radiance: [[f64; 3]; 9]) -> Result<Sh9, BakeError> {
    let mut irradiance = [[0.0_f32; 3]; 9];
    for band in 0..9 {
        let convolution = if band == 0 {
            PI
        } else if band < 4 {
            2.0 * PI / 3.0
        } else {
            PI / 4.0
        };
        for channel in 0..3 {
            let value = radiance[band][channel] * convolution;
            if !value.is_finite() || value.abs() > MAX_COEFFICIENT as f64 {
                return Err(invalid(
                    "baked SH coefficient is nonfinite or exceeds +/-10000",
                ));
            }
            irradiance[band][channel] = value as f32;
        }
    }
    Ok(irradiance)
}

impl BakeMaterial {
    fn reflectance(&self, uv: [f64; 2], point: V3) -> V3 {
        let mut rho = v64(self.base_color);
        if let Some(texture) = &self.texture {
            let sample = texture.sample(uv);
            for channel in 0..3 {
                rho[channel] *= sample[channel];
            }
        }
        if let Some(checker) = self.checker {
            let cell = (point[0] * checker.scale as f64).floor()
                + (point[2] * checker.scale as f64).floor();
            if cell.rem_euclid(2.0) >= 1.0 {
                rho = mul(rho, checker.dark_multiplier as f64);
            }
        }
        rho
    }
}
fn srgb_to_linear(byte: u8) -> f64 {
    let value = byte as f64 / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}
fn address(position: i64, extent: u32, mode: BakeWrap) -> usize {
    let extent = i64::from(extent);
    (match mode {
        BakeWrap::Clamp => position.clamp(0, extent - 1),
        BakeWrap::Repeat => position.rem_euclid(extent),
        BakeWrap::Mirror => {
            let p = position.rem_euclid(2 * extent);
            if p < extent {
                p
            } else {
                2 * extent - 1 - p
            }
        }
    }) as usize
}
fn reduce_uv(value: f64, mode: BakeWrap) -> f64 {
    match mode {
        BakeWrap::Clamp => value.clamp(0.0, 1.0),
        // Preserve the last-texel side when tiny negative UVs round the
        // remainder up to one, matching the renderer's nearest seam rule.
        BakeWrap::Repeat => value.rem_euclid(1.0).min(1.0 - f64::EPSILON / 2.0),
        BakeWrap::Mirror => value.rem_euclid(2.0),
    }
}
impl BakeTexture {
    fn texel(&self, x: i64, y: i64) -> V3 {
        let index = (address(y, self.height, self.wrap_t) * self.width as usize
            + address(x, self.width, self.wrap_s))
            * 4;
        std::array::from_fn(|channel| srgb_to_linear(self.rgba8_srgb[index + channel]))
    }
    fn sample(&self, uv: [f64; 2]) -> V3 {
        let p = [
            reduce_uv(uv[0], self.wrap_s) * self.width as f64,
            reduce_uv(uv[1], self.wrap_t) * self.height as f64,
        ];
        if self.filter == BakeFilter::Nearest {
            return self.texel(p[0].floor() as i64, p[1].floor() as i64);
        }
        let low = [(p[0] - 0.5).floor(), (p[1] - 0.5).floor()];
        let t = [p[0] - 0.5 - low[0], p[1] - 0.5 - low[1]];
        let a = self.texel(low[0] as i64, low[1] as i64);
        let b = self.texel(low[0] as i64 + 1, low[1] as i64);
        let c = self.texel(low[0] as i64, low[1] as i64 + 1);
        let d = self.texel(low[0] as i64 + 1, low[1] as i64 + 1);
        std::array::from_fn(|i| {
            (a[i] * (1.0 - t[0]) + b[i] * t[0]) * (1.0 - t[1])
                + (c[i] * (1.0 - t[0]) + d[i] * t[0]) * t[1]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::irradiance::{IrradianceProvenance, IRRADIANCE_VERSION, Y00};

    fn settings() -> BakeSettings {
        BakeSettings {
            rays_per_probe: 2048,
            ..Default::default()
        }
    }
    fn grid() -> IrradianceGrid {
        IrradianceGrid {
            version: IRRADIANCE_VERSION,
            enabled: true,
            provenance: IrradianceProvenance::Authored,
            dimensions: [2; 3],
            origin: [0.0; 3],
            spacing: [0.1; 3],
            coefficients: vec![[[0.0; 3]; 9]; 8],
        }
    }
    fn triangle(points: [[f32; 3]; 3], material: usize) -> BakeTriangle {
        let n = normalize(cross(
            sub(v64(points[1]), v64(points[0])),
            sub(v64(points[2]), v64(points[0])),
        ))
        .unwrap()
        .map(|x| x as f32);
        BakeTriangle {
            positions: points,
            normals: [n; 3],
            texcoords: [[0.0, 0.0], [1.0, 1.0], [1.0, 0.0]],
            material,
        }
    }
    fn square(y: f32, half: f32, material: usize) -> Vec<BakeTriangle> {
        vec![
            triangle(
                [[-half, y, -half], [half, y, half], [half, y, -half]],
                material,
            ),
            triangle(
                [[-half, y, -half], [-half, y, half], [half, y, half]],
                material,
            ),
        ]
    }
    fn scene(color: [f32; 3]) -> BakeScene {
        BakeScene {
            triangles: square(-1.0, 1.0, 0),
            materials: vec![BakeMaterial {
                base_color: color,
                ..Default::default()
            }],
            sun: BakeSun {
                direction: [0.0, 1.0, 0.0],
                color: [1.0; 3],
                intensity: 1.0,
            },
        }
    }
    fn close(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "actual={actual}, expected={expected}, tolerance={tolerance}"
        );
    }
    fn evaluate(sh: Sh9, normal: [f32; 3]) -> V3 {
        let basis = sh_basis(normal).unwrap();
        std::array::from_fn(|c| (0..9).map(|i| sh[i][c] as f64 * basis[i] as f64).sum())
    }
    fn outgoing(scene: &BakeScene, origin: V3, direction: V3) -> V3 {
        let settings = settings();
        let cancel = AtomicBool::new(false);
        let mut control = Control::new(&settings, &cancel).unwrap();
        let prepared = PreparedScene::new(scene, &mut control).unwrap();
        prepared
            .radiance(scene, origin, normalize(direction).unwrap(), &mut control)
            .unwrap()
    }

    #[test]
    fn constant_full_sphere_radiance_is_pi_l_and_positive_real_basis() {
        let directions = sphere_directions(2048);
        let radiance = [0.25, 1.0, 2.0];
        let mut coefficients = [[0.0; 3]; 9];
        for direction in directions {
            project_sample(&mut coefficients, direction, radiance, 4.0 * PI / 2048.0);
        }
        let sh = finish_projection(coefficients).unwrap();
        close(sh[0][1] as f64, PI / Y00 as f64, 0.00001);
        for normal in [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [-1.0, -2.0, -3.0],
        ] {
            for (actual, l) in evaluate(sh, normal).into_iter().zip(radiance) {
                close(actual, PI * l, 0.005);
            }
        }
        let mut directional = [[0.0; 3]; 9];
        project_sample(&mut directional, [1.0, 0.0, 0.0], [1.0; 3], 1.0);
        let sh = finish_projection(directional).unwrap();
        close(sh[3][0] as f64, 0.488_602_52 * 2.0 * PI / 3.0, 1e-7);
        close(sh[6][0] as f64, -0.315_391_57 * PI / 4.0, 1e-7);
        close(sh[8][0] as f64, 0.546_274_24 * PI / 4.0, 1e-7);
        assert_eq!(sh[1], [0.0; 3]);
        assert_eq!(sh[2], [0.0; 3]);
    }

    #[test]
    fn square_patch_dc_matches_analytic_solid_angle_and_colored_bounce() {
        let scene = scene([0.8, 0.2, 0.05]);
        let sh = bake(&scene, &grid(), &settings(), &AtomicBool::new(false)).unwrap()[0];
        // Rectangle solid angle: 4 atan(ab / (d sqrt(d²+a²+b²))).
        let omega = 4.0 * (1.0 / 3.0_f64.sqrt()).atan();
        for (channel, rho) in scene.materials[0].base_color.into_iter().enumerate() {
            let analytic_dc = PI * Y00 as f64 * omega * rho as f64;
            close(sh[0][channel] as f64, analytic_dc, 0.025 * analytic_dc);
        }
        let light = outgoing(&scene, [0.0; 3], [0.0, -1.0, 0.0]);
        for (actual, rho) in light.into_iter().zip(scene.materials[0].base_color) {
            close(actual, rho as f64, 1e-12);
        }
        assert!(
            sh[1][0] < 0.0,
            "floor's positive-real y coefficient must be negative"
        );
        assert!(sh[0][0] > sh[0][1] && sh[0][1] > sh[0][2]);
    }

    #[test]
    fn actual_secondary_sun_visibility_blocks_bounce() {
        let mut scene = scene([0.8, 0.3, 0.1]);
        let origin = [0.0, 0.0, 2.0];
        let ray = [0.0, -1.0, -2.0];
        assert!(outgoing(&scene, origin, ray)[0] > 0.79);
        // A tiny overhead shield misses the probe ray, but intercepts the
        // secondary ray from the sampled floor point towards the sun.
        scene.triangles.extend(square(0.5, 0.2, 0));
        assert_eq!(outgoing(&scene, origin, ray), [0.0; 3]);
        scene.sun.direction = [0.0, -1.0, 0.0];
        assert_eq!(outgoing(&scene, origin, ray), [0.0; 3]);
    }

    #[test]
    fn nearest_probe_occluder_replaces_far_radiance() {
        let mut scene = scene([1.0, 0.0, 0.0]);
        scene.materials.push(BakeMaterial {
            base_color: [0.0; 3],
            ..Default::default()
        });
        assert_eq!(
            outgoing(&scene, [0.0; 3], [0.0, -1.0, 0.0]),
            [1.0, 0.0, 0.0]
        );
        scene.triangles.extend(square(-0.5, 0.8, 1));
        assert_eq!(outgoing(&scene, [0.0; 3], [0.0, -1.0, 0.0]), [0.0; 3]);
    }

    #[test]
    fn miss_black_albedo_and_zero_sun_are_exactly_black() {
        let cancel = AtomicBool::new(false);
        let mut scene = scene([0.0; 3]);
        assert!(bake(&scene, &grid(), &settings(), &cancel)
            .unwrap()
            .iter()
            .flatten()
            .flatten()
            .all(|x| *x == 0.0));
        scene.materials[0].base_color = [1.0; 3];
        scene.sun.intensity = 0.0;
        assert!(bake(&scene, &grid(), &settings(), &cancel)
            .unwrap()
            .iter()
            .flatten()
            .flatten()
            .all(|x| *x == 0.0));
        scene.sun.intensity = 1.0;
        scene.triangles.clear();
        let mut grid = grid();
        grid.coefficients
            .fill(crate::irradiance::constant_irradiance([30.0; 3]).unwrap());
        assert!(
            bake(&scene, &grid, &settings(), &cancel)
                .unwrap()
                .iter()
                .flatten()
                .flatten()
                .all(|x| *x == 0.0),
            "existing probes must not feed back"
        );
    }

    #[test]
    fn texture_decodes_srgb_before_bilinear_and_wraps_each_tap() {
        let mut texture = BakeTexture {
            width: 2,
            height: 1,
            rgba8_srgb: vec![0, 0, 0, 0, 255, 128, 64, 255],
            wrap_s: BakeWrap::Clamp,
            wrap_t: BakeWrap::Clamp,
            filter: BakeFilter::Linear,
        };
        let center = texture.sample([0.5, 0.5]);
        close(center[0], 0.5, 1e-12);
        close(center[1], srgb_to_linear(128) * 0.5, 1e-12);
        assert!(
            (center[0] - srgb_to_linear(128)).abs() > 0.2,
            "sRGB bytes must not be interpolated before decoding"
        );
        assert_eq!(texture.sample([0.0, 0.5]), [0.0; 3]);
        texture.wrap_s = BakeWrap::Repeat;
        close(texture.sample([0.0, 0.5])[0], 0.5, 1e-12);
        close(texture.sample([-1.0, 0.5])[0], 0.5, 1e-12);
        close(texture.sample([1_000_000.0, 0.5])[0], 0.5, 1e-12);
        texture.wrap_s = BakeWrap::Mirror;
        close(texture.sample([1.0, 0.5])[0], 1.0, 1e-12);
        close(texture.sample([-0.25, 0.5])[0], 0.0, 1e-12);
        texture.filter = BakeFilter::Nearest;
        texture.wrap_s = BakeWrap::Repeat;
        close(texture.sample([-0.25, 0.5])[0], 1.0, 1e-12);
        close(texture.sample([1.0, 0.5])[0], 0.0, 1e-12);
        close(texture.sample([-1e-40, 0.5])[0], 1.0, 1e-12);
    }

    #[test]
    fn world_checker_and_texture_tint_are_diffuse_reflectance() {
        let material = BakeMaterial {
            base_color: [0.5, 0.25, 0.125],
            texture: Some(BakeTexture {
                width: 1,
                height: 1,
                rgba8_srgb: vec![128, 255, 0, 0],
                wrap_s: BakeWrap::Clamp,
                wrap_t: BakeWrap::Clamp,
                filter: BakeFilter::Linear,
            }),
            checker: Some(BakeChecker {
                scale: 0.5,
                dark_multiplier: 0.72,
            }),
            ..Default::default()
        };
        let light = material.reflectance([0.5; 2], [0.1, 5.0, 0.1]);
        let dark = material.reflectance([0.5; 2], [-0.1, 5.0, 0.1]);
        close(light[0], 0.5 * srgb_to_linear(128), 1e-12);
        assert_eq!(light[1], 0.25);
        assert_eq!(light[2], 0.0);
        for channel in 0..3 {
            close(dark[channel], light[channel] * 0.72_f32 as f64, 1e-12);
        }
    }

    // Independent oracle: plane intersection and Gram-matrix barycentrics,
    // deliberately not the production Möller–Trumbore routine or BVH bounds.
    fn brute_force(scene: &BakeScene, origin: V3, direction: V3) -> Option<(usize, f64)> {
        let mut nearest = None;
        for (index, triangle) in scene.triangles.iter().enumerate() {
            let [a, b, c] = triangle.positions.map(v64);
            let e = sub(b, a);
            let f = sub(c, a);
            let normal = cross(e, f);
            let denominator = dot(normal, direction);
            if denominator >= -dot(normal, normal).sqrt() * 1e-12 {
                continue;
            }
            let t = dot(normal, sub(a, origin)) / denominator;
            if t <= 0.0 {
                continue;
            }
            let q = sub(add(origin, mul(direction, t)), a);
            let ee = dot(e, e);
            let ef = dot(e, f);
            let ff = dot(f, f);
            let determinant = ee * ff - ef * ef;
            let u = (ff * dot(q, e) - ef * dot(q, f)) / determinant;
            let v = (ee * dot(q, f) - ef * dot(q, e)) / determinant;
            if u < 0.0 || v < 0.0 || u + v > 1.0 {
                continue;
            }
            if nearest
                .is_none_or(|(old_index, old_t)| t < old_t || (t == old_t && index < old_index))
            {
                nearest = Some((index, t));
            }
        }
        nearest
    }

    #[test]
    fn bvh_matches_independent_brute_force_oracle_and_is_deterministic() {
        let mut scene = scene([1.0; 3]);
        let mut state = 1234567u64;
        let mut random = || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            ((state >> 32) as u32 as f64 / u32::MAX as f64 * 6.0 - 3.0) as f32
        };
        for _ in 0..200 {
            scene.triangles.push(triangle(
                [
                    [random(), random(), random()],
                    [random(), random(), random()],
                    [random(), random(), random()],
                ],
                0,
            ));
        }
        let settings = settings();
        let cancel = AtomicBool::new(false);
        let mut control = Control::new(&settings, &cancel).unwrap();
        let prepared = PreparedScene::new(&scene, &mut control).unwrap();
        let second = PreparedScene::new(&scene, &mut control).unwrap();
        assert_eq!(prepared.order, second.order);
        for direction in sphere_directions(512) {
            let origin = [0.137, 0.281, 0.479];
            let actual = prepared
                .hit(&scene, origin, direction, false, &mut control)
                .unwrap();
            let oracle = brute_force(&scene, origin, direction);
            assert_eq!(actual.map(|h| h.triangle), oracle.map(|h| h.0));
            if let (Some(hit), Some((_, t))) = (actual, oracle) {
                close(hit.distance, t, 1e-10);
            }
        }
    }

    #[test]
    fn backface_policy_and_geometric_offsets_are_explicit_at_large_world_scale() {
        let mut scene = scene([1.0; 3]);
        assert_eq!(
            outgoing(&scene, [0.0, -2.0, 0.0], [0.0, 1.0, 0.0]),
            [0.0; 3]
        );
        scene.materials[0].double_sided = true;
        scene.sun.direction = [0.0, -1.0, 0.0];
        assert_eq!(
            outgoing(&scene, [0.0, -2.0, 0.0], [0.0, 1.0, 0.0]),
            [1.0; 3]
        );
        scene.materials[0].double_sided = false;
        scene.sun.direction = [0.0, 1.0, 0.0];
        for triangle in &mut scene.triangles {
            for p in &mut triangle.positions {
                p[0] += 999_000.0;
                p[2] -= 999_000.0;
            }
        }
        assert_eq!(
            outgoing(&scene, [999_000.0, 0.0, -999_000.0], [0.0, -1.0, 0.0]),
            [1.0; 3],
            "must not self-shadow after world translation"
        );
    }

    #[test]
    fn reject_nonfinite_excess_and_degenerate_without_partial_coefficients() {
        let mut scene = scene([1.0; 3]);
        let cancel = AtomicBool::new(false);
        scene.sun.intensity = 1e20;
        assert!(matches!(
            bake(&scene, &grid(), &settings(), &cancel),
            Err(BakeError::InvalidInput(_))
        ));
        scene.sun.intensity = f32::NAN;
        assert!(matches!(
            bake(&scene, &grid(), &settings(), &cancel),
            Err(BakeError::InvalidInput(_))
        ));
        scene.sun.intensity = 1.0;
        scene.triangles[0].positions = [[0.0; 3]; 3];
        assert!(matches!(
            bake(&scene, &grid(), &settings(), &cancel),
            Err(BakeError::InvalidInput(_))
        ));
        let mut coefficients = [[0.0; 3]; 9];
        coefficients[8][2] = f64::NAN;
        assert!(finish_projection(coefficients).is_err());
        coefficients[8][2] = MAX_COEFFICIENT as f64 * 2.0;
        assert!(finish_projection(coefficients).is_err());
    }

    #[test]
    fn cancellation_checked_at_assembly_bvh_and_traversal_boundaries() {
        let scene = scene([1.0; 3]);
        let settings = settings();
        let cancel = AtomicBool::new(true);
        let progress = AtomicU32::new(99);
        assert_eq!(
            bake_with_progress(&scene, &grid(), &settings, &cancel, &progress),
            Err(BakeError::Cancelled)
        );
        assert_eq!(progress.load(Ordering::Relaxed), 0);
        let mut control = Control::new(&settings, &cancel).unwrap();
        assert!(matches!(
            PreparedScene::new(&scene, &mut control),
            Err(BakeError::Cancelled)
        ));
        cancel.store(false, Ordering::Relaxed);
        let mut prepared = PreparedScene::new(&scene, &mut control).unwrap();
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(
            prepared.build(0, 2, 0, &mut control),
            Err(BakeError::Cancelled)
        );
        control.node_visits = 63;
        assert!(matches!(
            prepared.hit(&scene, [0.0; 3], [0.0, -1.0, 0.0], false, &mut control),
            Err(BakeError::Cancelled)
        ));
        control.triangle_tests = 63;
        assert_eq!(control.triangle(), Err(BakeError::Cancelled));
    }

    #[test]
    fn hard_budgets_and_progress_are_bounded_and_no_partial_result_escapes() {
        let scene = scene([1.0; 3]);
        let cancel = AtomicBool::new(false);
        let progress = AtomicU32::new(0);
        let mut settings = settings();
        settings.max_triangle_tests = 1;
        assert!(matches!(
            bake_with_progress(&scene, &grid(), &settings, &cancel, &progress),
            Err(BakeError::WorkLimit("triangle-test"))
        ));
        assert!(progress.load(Ordering::Relaxed) < 8);
        settings.max_triangle_tests = MAX_BAKE_TRIANGLE_TESTS;
        settings.max_node_visits = 1;
        assert!(matches!(
            bake(&scene, &grid(), &settings, &cancel),
            Err(BakeError::WorkLimit("BVH traversal"))
        ));
        settings.max_node_visits = MAX_BAKE_NODE_VISITS;
        settings.max_duration = Duration::from_nanos(1);
        assert_eq!(
            bake(&scene, &grid(), &settings, &cancel),
            Err(BakeError::TimedOut)
        );
        settings = BakeSettings::default();
        settings.rays_per_probe = MAX_BAKE_RAYS_PER_PROBE + 2;
        assert!(matches!(
            bake(&scene, &grid(), &settings, &cancel),
            Err(BakeError::InvalidInput(_))
        ));
        settings = BakeSettings::default();
        bake_with_progress(&scene, &grid(), &settings, &cancel, &progress).unwrap();
        assert_eq!(progress.load(Ordering::Relaxed), 8);
    }

    #[test]
    fn cancellation_after_first_completed_probe_discards_the_entire_result() {
        let cancel = AtomicBool::new(false);
        let progress = AtomicU32::new(0);
        let result = bake_internal(
            &scene([1.0; 3]),
            &grid(),
            &BakeSettings::default(),
            &cancel,
            &progress,
            |completed| {
                assert_eq!(completed, 1);
                cancel.store(true, Ordering::Relaxed);
            },
        );
        assert_eq!(result, Err(BakeError::Cancelled));
        assert_eq!(progress.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn every_sh_band_matches_literal_positive_real_reference() {
        let length = 14.0_f64.sqrt();
        let [x, y, z] = [1.0 / length, 2.0 / length, 3.0 / length];
        let literal = [
            0.282_094_8,
            0.488_602_52 * y,
            0.488_602_52 * z,
            0.488_602_52 * x,
            1.092_548_5 * x * y,
            1.092_548_5 * y * z,
            0.315_391_57 * (3.0 * z * z - 1.0),
            1.092_548_5 * x * z,
            0.546_274_24 * (x * x - y * y),
        ];
        let mut radiance_sh = [[0.0; 3]; 9];
        project_sample(&mut radiance_sh, [x, y, z], [1.0, 2.0, 3.0], 0.7);
        let irradiance_sh = finish_projection(radiance_sh).unwrap();
        for band in 0..9 {
            let convolution = [
                PI,
                2.0 * PI / 3.0,
                2.0 * PI / 3.0,
                2.0 * PI / 3.0,
                PI / 4.0,
                PI / 4.0,
                PI / 4.0,
                PI / 4.0,
                PI / 4.0,
            ][band];
            for (channel, &coefficient) in irradiance_sh[band].iter().enumerate() {
                close(
                    coefficient as f64,
                    literal[band] * 0.7 * (channel + 1) as f64 * convolution,
                    3e-7,
                );
            }
        }
    }

    #[test]
    fn finite_square_patch_irradiance_matches_analytic_projected_solid_angle() {
        let scene = scene([0.8, 0.2, 0.05]);
        let settings = settings();
        let cancel = AtomicBool::new(false);
        let mut control = Control::new(&settings, &cancel).unwrap();
        let prepared = PreparedScene::new(&scene, &mut control).unwrap();
        let mut direct_irradiance = [0.0; 3];
        for direction in sphere_directions(settings.rays_per_probe) {
            let radiance = prepared
                .radiance(&scene, [0.0; 3], direction, &mut control)
                .unwrap();
            let weight = (-direction[1]).max(0.0) * 4.0 * PI / settings.rays_per_probe as f64;
            for channel in 0..3 {
                direct_irradiance[channel] += radiance[channel] * weight;
            }
        }
        // Centered rectangle half extents a=b=1 at h=1, receiver normal -Y:
        // E/L = 2[a/sqrt(a²+h²) atan(b/sqrt(a²+h²))
        //         + b/sqrt(b²+h²) atan(a/sqrt(b²+h²))].
        let projected_solid_angle = 4.0 / 2.0_f64.sqrt() * (1.0 / 2.0_f64.sqrt()).atan();
        let sh = bake(&scene, &grid(), &settings, &cancel).unwrap()[0];
        let sh_irradiance = evaluate(sh, [0.0, -1.0, 0.0]);
        for channel in 0..3 {
            let exact = projected_solid_angle * scene.materials[0].base_color[channel] as f64;
            close(direct_irradiance[channel], exact, exact * 0.02);
            // SH9 is a low-order angular approximation, so its tolerance also
            // includes truncation error, not merely quadrature error.
            close(sh_irradiance[channel], exact, exact * 0.04);
        }
    }
}
