//! Procedural meshes: terrain, water, trees, rocks and ground cover.
//! Leaves are real triangles, so shadows and antialiasing need no alpha test.

use crate::math::{add, cross, dot, lerp, lerp3, noise3, normalize, scale, smoothstep, sub, Rng, V3};
use crate::world::{far_coordinate, grid_coordinate, World, FAR_GRID, GRID, HALF};
use bytemuck::{Pod, Zeroable};
use std::f32::consts::{PI, TAU};

pub const KIND_BARK: f32 = 0.0;
pub const KIND_LEAF: f32 = 1.0;
pub const KIND_GRASS: f32 = 2.0;
pub const KIND_ROCK: f32 = 3.0;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct TerrainVertex {
    pub position: V3,
    pub normal: V3,
    /// Sky visibility, depth below the water surface, tree cover, meters from the water edge.
    pub data: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct WaterVertex {
    pub position: V3,
    /// Water depth, cascade amount, position across the stream (-1 to 1), surface slope.
    pub data: [f32; 4],
    /// Unit flow direction on the ground plane.
    pub flow: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PropVertex {
    pub position: V3,
    pub normal: V3,
    /// Linear base color.
    pub color: V3,
    /// Ambient visibility, wind sway in meters, surface kind.
    pub params: V3,
}

#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub vertices: Vec<PropVertex>,
    pub indices: Vec<u32>,
}

impl Mesh {
    fn vertex(&mut self, position: V3, normal: V3, color: V3, params: V3) -> u32 {
        self.vertices.push(PropVertex { position, normal: normalize(normal), color, params });
        (self.vertices.len() - 1) as u32
    }

    fn triangle(&mut self, a: u32, b: u32, c: u32) {
        self.indices.extend([a, b, c]);
    }

    /// Radius of the sphere around the origin that contains the mesh.
    pub fn radius(&self) -> f32 {
        self.vertices.iter().map(|v| dot(v.position, v.position)).fold(0.0, f32::max).sqrt()
    }

    /// A tapered tube with smooth sides and no caps.
    #[allow(clippy::too_many_arguments)]
    fn tube(&mut self, from: V3, to: V3, radius: [f32; 2], sides: u32, color: V3, sway: [f32; 2], ao: f32) {
        let axis = normalize(sub(to, from));
        let helper = if axis[1].abs() > 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
        let u = normalize(cross(axis, helper));
        let v = cross(axis, u);
        let first = self.vertices.len() as u32;
        for end in 0..2 {
            let center = if end == 0 { from } else { to };
            for side in 0..sides {
                let angle = side as f32 / sides as f32 * TAU;
                let out = add(scale(u, angle.cos()), scale(v, angle.sin()));
                self.vertex(add(center, scale(out, radius[end])), out, color, [ao, sway[end], KIND_BARK]);
            }
        }
        for side in 0..sides {
            let next = (side + 1) % sides;
            let (a, b, c, d) = (first + side, first + next, first + sides + side, first + sides + next);
            self.triangle(a, b, c);
            self.triangle(b, d, c);
        }
    }
}

fn icosphere(subdivisions: u32) -> (Vec<V3>, Vec<[u32; 3]>) {
    let t = (1.0 + 5.0_f32.sqrt()) * 0.5;
    let mut points: Vec<V3> = [
        [-1.0, t, 0.0], [1.0, t, 0.0], [-1.0, -t, 0.0], [1.0, -t, 0.0],
        [0.0, -1.0, t], [0.0, 1.0, t], [0.0, -1.0, -t], [0.0, 1.0, -t],
        [t, 0.0, -1.0], [t, 0.0, 1.0], [-t, 0.0, -1.0], [-t, 0.0, 1.0],
    ]
    .into_iter()
    .map(normalize)
    .collect();
    let mut faces: Vec<[u32; 3]> = vec![
        [0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11],
        [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8],
        [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9],
        [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1],
    ];
    for _ in 0..subdivisions {
        // Midpoints are shared through a sorted edge list, not a hash map.
        let mut edges: Vec<(u32, u32, u32)> = Vec::with_capacity(faces.len() * 3);
        for face in &faces {
            for k in 0..3 {
                let (a, b) = (face[k], face[(k + 1) % 3]);
                edges.push((a.min(b), a.max(b), 0));
            }
        }
        edges.sort_unstable();
        edges.dedup();
        for edge in &mut edges {
            edge.2 = points.len() as u32;
            let (a, b) = (points[edge.0 as usize], points[edge.1 as usize]);
            points.push(normalize(add(a, b)));
        }
        let middle = |a: u32, b: u32| {
            let key = (a.min(b), a.max(b));
            let found = edges.binary_search_by(|e| (e.0, e.1).cmp(&key)).expect("edge exists");
            edges[found].2
        };
        let mut next = Vec::with_capacity(faces.len() * 4);
        for [a, b, c] in faces {
            let (ab, bc, ca) = (middle(a, b), middle(b, c), middle(c, a));
            next.extend([[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
        }
        faces = next;
    }
    (points, faces)
}

/// Levels of detail of a tree mesh: seen close, at middle distance, far away.
pub const TREE_LEVELS: usize = 3;

/// A fir: a trunk and tiers of drooping branch skirts with a toothed rim.
/// `level` is 0 for the closest and most detailed mesh.
pub fn conifer(level: usize, seed: u64) -> Mesh {
    let mut rng = Rng::new(seed);
    let mut mesh = Mesh::default();
    let height = rng.range(16.0, 21.0);
    let spread = rng.range(3.1, 3.9);
    let bark = [0.085, 0.055, 0.038];
    let sway_at = |y: f32| 0.55 * (y / height) * (y / height);
    mesh.tube([0.0, -0.4, 0.0], [0.0, height * 0.96, 0.0], [0.34, 0.04], [7, 4, 3][level], bark, [0.0, sway_at(height)], 0.8);
    let tiers = [24, 7, 3][level];
    let teeth = [13, 6, 4][level];
    let inner = [0.022, 0.058, 0.03];
    let outer = [0.07, 0.165, 0.065];
    for tier in 0..tiers {
        let t = tier as f32 / (tiers - 1) as f32;
        let y = height * (0.15 + 0.8 * t);
        let radius = (spread * (1.0 - t).powf(0.85) + 0.3) * rng.range(0.82, 1.12);
        // Every level of detail ends at the same treetop, so a level change does not resize the tree.
        let rise = (height * 0.8 / tiers as f32 * [2.6, 1.7, 1.5][level]).min(height * 1.04 - y);
        let droop = radius * rng.range(0.34, 0.5);
        let turn = rng.range(0.0, TAU);
        let shade = 0.7 + 0.3 * t;
        let apex = mesh.vertex([0.0, y + rise, 0.0], [0.0, 1.0, 0.0], inner, [0.5 * shade, sway_at(y + rise), KIND_LEAF]);
        let ring = 2 * teeth;
        let middle_first = mesh.vertices.len() as u32;
        // A middle ring bends the skirt into a bell; the rim alternates tip and notch.
        for k in 0..ring {
            let angle = turn + k as f32 / ring as f32 * TAU;
            let out = [angle.cos(), 0.0, angle.sin()];
            let r = radius * 0.5 * rng.range(0.9, 1.1);
            let position = [out[0] * r, y + rise * 0.42, out[2] * r];
            mesh.vertex(position, [out[0], 1.1, out[2]], lerp3(inner, outer, 0.45), [0.72 * shade, sway_at(position[1]) + 0.03, KIND_LEAF]);
        }
        let rim_first = mesh.vertices.len() as u32;
        for k in 0..ring {
            let angle = turn + k as f32 / ring as f32 * TAU;
            let out = [angle.cos(), 0.0, angle.sin()];
            let tip = k % 2 == 0;
            let r = radius * if tip { rng.range(0.92, 1.12) } else { rng.range(0.56, 0.7) };
            let fall = if tip { droop } else { droop * 0.35 };
            let position = [out[0] * r, y - fall, out[2] * r];
            let color = scale(outer, if tip { rng.range(0.95, 1.25) } else { 0.8 });
            mesh.vertex(position, [out[0], 0.55, out[2]], color, [shade, sway_at(position[1]) + if tip { 0.1 } else { 0.05 }, KIND_LEAF]);
        }
        for k in 0..ring as u32 {
            let next = (k + 1) % ring as u32;
            mesh.triangle(apex, middle_first + next, middle_first + k);
            mesh.triangle(middle_first + k, middle_first + next, rim_first + k);
            mesh.triangle(middle_first + next, rim_first + next, rim_first + k);
        }
    }
    mesh
}

/// A broadleaf tree: a forked trunk and a crown of lumpy leaf masses with loose sprigs.
/// `level` is 0 for the closest and most detailed mesh.
pub fn broadleaf(level: usize, seed: u64) -> Mesh {
    let mut rng = Rng::new(seed);
    let mut mesh = Mesh::default();
    let trunk_height = rng.range(3.4, 5.0);
    let crown_radius = rng.range(3.3, 4.3);
    let crown: V3 = [rng.range(-0.4, 0.4), trunk_height + crown_radius * 0.8, rng.range(-0.4, 0.4)];
    let top = crown[1] + crown_radius;
    let bark = [0.1, 0.07, 0.048];
    let sway_at = |y: f32| 0.4 * (y / top) * (y / top);
    let sides = [8, 5, 3][level];
    let fork: V3 = [crown[0] * 0.4, trunk_height, crown[2] * 0.4];
    mesh.tube([0.0, -0.4, 0.0], fork, [0.38, 0.24], sides, bark, [0.0, sway_at(trunk_height)], 0.75);

    let masses = [13, 5, 2][level];
    let (points, faces) = icosphere([2, 1, 0][level]);
    let low = [0.024, 0.07, 0.02];
    let high = [0.09, 0.19, 0.045];
    let shading_center = sub(crown, [0.0, crown_radius * 0.55, 0.0]);
    let mut centers: Vec<(V3, f32)> = Vec::new();
    for mass in 0..masses {
        let (center, radius) = if mass == 0 {
            // With only two masses left, the first one stands for the whole crown.
            (add(crown, [0.0, crown_radius * 0.22, 0.0]), crown_radius * [0.72, 0.72, 0.98][level])
        } else {
            // The golden angle spreads the masses evenly around the crown.
            let angle = mass as f32 * 2.4 + rng.range(-0.3, 0.3);
            let away = crown_radius * rng.range(0.4, 0.74);
            let lift = crown_radius * rng.range(-0.42, 0.34);
            (add(crown, [angle.cos() * away, lift, angle.sin() * away]), crown_radius * rng.range(0.34, 0.5))
        };
        centers.push((center, radius));
        // A branch reaches into each leaf mass.
        mesh.tube(fork, add(center, [0.0, -radius * 0.3, 0.0]), [0.17, 0.05], sides.min(5), bark, [sway_at(fork[1]), sway_at(center[1])], 0.6);
        let offset = rng.range(0.0, 50.0);
        let first = mesh.vertices.len() as u32;
        for direction in &points {
            let lump = noise3(direction[0] * 2.1 + offset, direction[1] * 2.1, direction[2] * 2.1, seed as u32);
            let fine = noise3(direction[0] * 5.3, direction[1] * 5.3 + offset, direction[2] * 5.3, seed as u32 ^ 0x31);
            let r = radius * (0.72 + 0.6 * lump + 0.2 * fine);
            let position = add(center, [direction[0] * r, direction[1] * r * 0.82, direction[2] * r]);
            let outward = normalize(sub(position, shading_center));
            let normal = normalize(add(scale(*direction, 0.45), scale(outward, 0.55)));
            let h = ((position[1] - (crown[1] - crown_radius)) / (2.0 * crown_radius)).clamp(0.0, 1.0);
            let edge = (crate::math::length(sub(position, crown)) / crown_radius).clamp(0.0, 1.2);
            let color = scale(lerp3(low, high, h.powf(1.2)), 0.85 + 0.3 * fine);
            let ao = lerp(0.5, 1.0, (0.1 + 0.9 * h).min(1.0)) * lerp(0.65, 1.0, smoothstep(0.35, 1.0, edge));
            mesh.vertex(position, normal, color, [ao, sway_at(position[1]) + 0.05, KIND_LEAF]);
        }
        for face in &faces {
            mesh.triangle(first + face[0], first + face[1], first + face[2]);
        }
    }
    if level == 0 {
        // Loose sprigs break up the smooth outline of the leaf masses.
        for _ in 0..520 {
            let (center, radius) = centers[(rng.next_u32() as usize) % centers.len()];
            let direction = normalize([rng.range(-1.0, 1.0), rng.range(-0.6, 1.0), rng.range(-1.0, 1.0)]);
            let base = add(center, [direction[0] * radius * 0.96, direction[1] * radius * 0.79, direction[2] * radius * 0.96]);
            let side = normalize(cross(direction, [rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)]));
            let along = cross(direction, side);
            let size = rng.range(0.3, 0.55);
            // Sprigs lie close to the surface and lift a little, like the outer leaves.
            let tip = add(base, scale(add(scale(along, 0.85), scale(direction, 0.5)), size));
            let h = ((base[1] - (crown[1] - crown_radius)) / (2.0 * crown_radius)).clamp(0.0, 1.0);
            let color = scale(lerp3(low, high, h), rng.range(1.0, 1.45));
            let normal = normalize(add(direction, [0.0, 0.6, 0.0]));
            let params = [lerp(0.55, 1.0, h), sway_at(base[1]) + 0.1, KIND_LEAF];
            let a = mesh.vertex(add(base, scale(side, size * 0.5)), normal, color, params);
            let b = mesh.vertex(sub(base, scale(side, size * 0.5)), normal, color, params);
            let c = mesh.vertex(tip, normal, scale(color, 1.15), params);
            mesh.triangle(a, b, c);
        }
    }
    mesh
}

/// A boulder about one meter in radius.
pub fn rock(seed: u64) -> Mesh {
    let mut rng = Rng::new(seed);
    let mut mesh = Mesh::default();
    let (points, faces) = icosphere(2);
    let offset = rng.range(0.0, 90.0);
    let squash = [rng.range(0.85, 1.15), rng.range(0.6, 0.85), rng.range(0.8, 1.1)];
    let mut positions: Vec<V3> = Vec::with_capacity(points.len());
    for d in &points {
        let broad = noise3(d[0] * 1.4 + offset, d[1] * 1.4, d[2] * 1.4, seed as u32);
        // Folding the second octave leaves creases, like fractured stone.
        let crease = (noise3(d[0] * 3.1, d[1] * 3.1 + offset, d[2] * 3.1, seed as u32 ^ 0x47) - 0.5).abs();
        let r = 0.72 + 0.5 * broad - 0.35 * crease;
        positions.push([d[0] * r * squash[0], d[1] * r * squash[1], d[2] * r * squash[2]]);
    }
    let mut normals = vec![[0.0; 3]; positions.len()];
    for face in &faces {
        let [a, b, c] = face.map(|i| positions[i as usize]);
        let n = cross(sub(b, a), sub(c, a));
        for &i in face {
            normals[i as usize] = add(normals[i as usize], n);
        }
    }
    for (position, normal) in positions.iter().zip(&normals) {
        let tone = noise3(position[0] * 2.0 + offset, position[1] * 2.0, position[2] * 2.0, seed as u32 ^ 0x93);
        let color = scale([0.3, 0.29, 0.27], 0.75 + 0.5 * tone);
        let ao = lerp(0.45, 1.0, smoothstep(-0.5, 0.3, position[1]));
        mesh.vertex(*position, *normal, color, [ao, 0.0, KIND_ROCK]);
    }
    for face in &faces {
        mesh.triangle(face[0], face[1], face[2]);
    }
    mesh
}

fn blade(mesh: &mut Mesh, rng: &mut Rng, base: V3, height: f32, width: f32, lean: V3, colors: [V3; 2]) {
    let side = normalize(cross(lean, [0.0, 1.0, 0.0]));
    let levels = 3;
    let normal = [lean[0] * 0.35, 1.0, lean[2] * 0.35];
    let bend = height * rng.range(0.25, 0.6);
    let mut previous: Option<(u32, u32)> = None;
    for level in 0..=levels {
        let s = level as f32 / levels as f32;
        let center = add(base, add([0.0, height * s, 0.0], scale(lean, bend * s * s)));
        let color = lerp3(colors[0], colors[1], s);
        let params = [lerp(0.4, 1.0, s), 0.16 * s * s * height / 0.5, KIND_GRASS];
        if level == levels {
            let tip = mesh.vertex(center, normal, color, params);
            if let Some((left, right)) = previous {
                mesh.triangle(left, right, tip);
            }
        } else {
            let half = width * 0.5 * (1.0 - 0.55 * s);
            let left = mesh.vertex(sub(center, scale(side, half)), normal, color, params);
            let right = mesh.vertex(add(center, scale(side, half)), normal, color, params);
            if let Some((a, b)) = previous {
                mesh.triangle(a, b, left);
                mesh.triangle(b, right, left);
            }
            previous = Some((left, right));
        }
    }
}

/// A tuft of grass blades, optionally with a few flower heads.
pub fn grass_tuft(flowers: bool, seed: u64) -> Mesh {
    let mut rng = Rng::new(seed);
    let mut mesh = Mesh::default();
    let root = [0.02, 0.05, 0.012];
    let tip = [0.1, 0.2, 0.045];
    for _ in 0..14 {
        let angle = rng.range(0.0, TAU);
        let away = rng.range(0.0, 0.3);
        let lean = [angle.cos(), 0.0, angle.sin()];
        let base = [lean[0] * away, 0.0, lean[2] * away];
        let dry = rng.unit();
        let tip_color = lerp3(tip, [0.2, 0.2, 0.07], dry * dry * 0.6);
        // Tall in the middle of the tuft, short at its edge.
        let height = rng.range(0.32, 0.56) * (1.25 - 1.3 * away);
        blade(&mut mesh, &mut rng, base, height, 0.052, lean, [root, tip_color]);
    }
    if flowers {
        let petal = match seed % 3 {
            0 => [0.9, 0.86, 0.72],
            1 => [0.85, 0.6, 0.06],
            _ => [0.42, 0.22, 0.6],
        };
        for _ in 0..4 {
            let angle = rng.range(0.0, TAU);
            let away = rng.range(0.03, 0.25);
            let lean = [angle.cos(), 0.0, angle.sin()];
            let base = [lean[0] * away, 0.0, lean[2] * away];
            let height = rng.range(0.45, 0.7);
            blade(&mut mesh, &mut rng, base, height, 0.025, lean, [root, [0.1, 0.22, 0.05]]);
            let head = mesh.vertices.last().expect("blade has a tip").position;
            let params = [1.0, 0.16 * height / 0.5, KIND_GRASS];
            let center = mesh.vertex(add(head, [0.0, 0.01, 0.0]), [0.0, 1.0, 0.0], [0.8, 0.55, 0.05], params);
            let petals = 6;
            let first = mesh.vertices.len() as u32;
            for k in 0..petals {
                let a = k as f32 / petals as f32 * TAU;
                mesh.vertex(add(head, [a.cos() * 0.06, 0.03, a.sin() * 0.06]), [a.cos() * 0.4, 1.0, a.sin() * 0.4], petal, params);
            }
            for k in 0..petals {
                mesh.triangle(center, first + (k + 1) % petals, first + k);
            }
        }
    }
    mesh
}

/// A fern: arching fronds with a toothed outline.
pub fn fern(seed: u64) -> Mesh {
    let mut rng = Rng::new(seed);
    let mut mesh = Mesh::default();
    let dark = [0.018, 0.065, 0.02];
    let light = [0.06, 0.16, 0.04];
    let fronds = 8;
    for frond in 0..fronds {
        let angle = frond as f32 / fronds as f32 * TAU + rng.range(-0.25, 0.25);
        let out = [angle.cos(), 0.0, angle.sin()];
        let side = [-out[2], 0.0, out[0]];
        let length = rng.range(0.6, 0.95);
        let arch = rng.range(0.4, 0.62);
        let levels = 12;
        let mut previous: Option<(u32, u32)> = None;
        for level in 0..=levels {
            let s = level as f32 / levels as f32;
            let center = add(scale(out, length * s), [0.0, length * arch * (s * PI * 0.72).sin(), 0.0]);
            let slope = arch * PI * 0.72 * (s * PI * 0.72).cos();
            let normal = normalize([-out[0] * slope, 1.0, -out[2] * slope]);
            let tooth = if level % 2 == 0 { 1.0 } else { 0.55 };
            let half = 0.13 * (PI * s.powf(0.75)).sin().max(0.0) * tooth + 0.004;
            let color = lerp3(dark, light, s * 0.8 + 0.2 * tooth);
            let params = [lerp(0.45, 1.0, s), 0.09 * s * s, KIND_GRASS];
            let left = mesh.vertex(sub(center, scale(side, half)), normal, color, params);
            let right = mesh.vertex(add(center, scale(side, half)), normal, color, params);
            if let Some((a, b)) = previous {
                mesh.triangle(a, b, left);
                mesh.triangle(b, right, left);
            }
            previous = Some((left, right));
        }
    }
    mesh
}

fn sky_visibility(world: &World, x: f32, z: f32, y: f32) -> f32 {
    let mut blocked = 0.0;
    for direction in 0..8 {
        let angle = direction as f32 / 8.0 * TAU;
        let (dx, dz) = (angle.cos(), angle.sin());
        let mut horizon: f32 = 0.0;
        for distance in [4.0, 12.0, 32.0] {
            horizon = horizon.max((world.height(x + dx * distance, z + dz * distance) - y) / distance);
        }
        blocked += smoothstep(0.0, 1.2, horizon);
    }
    (1.0 - 0.8 * blocked / 8.0).max(0.25)
}

fn grid_indices(side: usize, keep: impl Fn(usize, usize) -> bool) -> Vec<u32> {
    let mut indices = Vec::with_capacity((side - 1) * (side - 1) * 6);
    for j in 0..side - 1 {
        for i in 0..side - 1 {
            if !keep(i, j) {
                continue;
            }
            let a = (j * side + i) as u32;
            let (b, c, d) = (a + 1, a + side as u32, a + side as u32 + 1);
            indices.extend([a, c, b, b, c, d]);
        }
    }
    indices
}

/// The detailed square around the stream.
pub fn terrain(world: &World) -> (Vec<TerrainVertex>, Vec<u32>) {
    let mut vertices = Vec::with_capacity(GRID * GRID);
    for j in 0..GRID {
        for i in 0..GRID {
            let (x, z) = (grid_coordinate(i), grid_coordinate(j));
            let y = world.heights[j * GRID + i];
            let forest = world.forest(x, z);
            vertices.push(TerrainVertex {
                position: [x, y, z],
                normal: world.normal(x, z),
                data: [
                    sky_visibility(world, x, z, y) * (1.0 - 0.3 * forest),
                    world.water_depth(x, z, y),
                    forest,
                    world.bank_distance(x, z).min(200.0),
                ],
            });
        }
    }
    (vertices, grid_indices(GRID, |_, _| true))
}

/// The coarse backdrop of distant mountains. It has a hole under the detailed
/// square and sits a little lower where the two overlap.
pub fn far_terrain(world: &World) -> (Vec<TerrainVertex>, Vec<u32>) {
    let cell = far_coordinate(1) - far_coordinate(0);
    let mut vertices = Vec::with_capacity(FAR_GRID * FAR_GRID);
    for j in 0..FAR_GRID {
        for i in 0..FAR_GRID {
            let (x, z) = (far_coordinate(i), far_coordinate(j));
            let inside = x.abs() < HALF && z.abs() < HALF;
            let y = world.ground(x, z) - if inside { 6.0 } else { 0.0 };
            let d = cell * 0.5;
            let normal = normalize([
                world.ground(x - d, z) - world.ground(x + d, z),
                2.0 * d,
                world.ground(x, z - d) - world.ground(x, z + d),
            ]);
            vertices.push(TerrainVertex { position: [x, y, z], normal, data: [0.85, -100.0, world.forest(x, z), 200.0] });
        }
    }
    let covered = |i: usize| far_coordinate(i).abs() < HALF - 8.0;
    let indices = grid_indices(FAR_GRID, |i, j| !(covered(i) && covered(i + 1) && covered(j) && covered(j + 1)));
    (vertices, indices)
}

/// The water surface: a ribbon that follows the stream and its level.
pub fn water(world: &World) -> (Vec<WaterVertex>, Vec<u32>) {
    let across_count = 13;
    let rows = (2.0 * HALF) as usize + 1;
    let mut vertices = Vec::with_capacity(rows * across_count);
    let mut indices = Vec::with_capacity((rows - 1) * (across_count - 1) * 6);
    for row in 0..rows {
        let z = -HALF + row as f32;
        let (here, ahead) = (world.stream(z), world.stream(z + 1.0));
        let flow = normalize([ahead.center - here.center, 0.0, 1.0]);
        let slope = here.level - ahead.level;
        for k in 0..across_count {
            let t = (k as f32 / (across_count - 1) as f32 * 2.0 - 1.0) * 1.3;
            let x = here.center + t * here.half_width;
            let depth = here.level - world.height(x, z);
            vertices.push(WaterVertex { position: [x, here.level, z], data: [depth, here.cascade, t, slope], flow: [flow[0], flow[2]] });
        }
    }
    for row in 0..rows - 1 {
        for k in 0..across_count - 1 {
            let a = (row * across_count + k) as u32;
            let (b, c, d) = (a + 1, a + across_count as u32, a + across_count as u32 + 1);
            indices.extend([a, c, b, b, c, d]);
        }
    }
    (vertices, indices)
}
