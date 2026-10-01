//! Procedural meshes: unit sphere, box, capsule and ground plane.
//!
//! All four are generated on the CPU once and live in one vertex and one
//! index buffer. Every instance scales a unit mesh, so a single mesh serves
//! every size:
//!
//! - sphere: unit radius; instance scale = radius (or an ellipsoid).
//! - box: half extent 1 on each axis; instance scale = half extents.
//! - plane: the square `x, z in [-1, 1]` at `y = 0`, normal +y; instance
//!   scale = `(half_x, 1, half_z)`.
//! - capsule: a unit sphere split at the equator. A vertex carries
//!   [`Vertex3::cap`] (`+1` upper half, `-1` lower half) and the vertex
//!   shader moves it along local y by `cap * half_length`, so any capsule
//!   keeps round caps (non uniform scaling would squash them). Instance
//!   scale = radius on every axis.

use bytemuck::{Pod, Zeroable};

/// A mesh vertex.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Vertex3 {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    /// Capsule half selector: `+1`, `-1`, or `0` for other meshes.
    pub cap: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MeshKind {
    Sphere = 0,
    Box = 1,
    Capsule = 2,
    Plane = 3,
}

impl MeshKind {
    pub const ALL: [MeshKind; 4] = [MeshKind::Sphere, MeshKind::Box, MeshKind::Capsule, MeshKind::Plane];
}

/// One mesh: vertices and `u32` indices local to those vertices.
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub vertices: Vec<Vertex3>,
    pub indices: Vec<u32>,
}

/// Longitude segments of the round meshes and latitude rings per hemisphere.
const SEGMENTS: u32 = 32;
const RINGS: u32 = 12;

/// Builds a surface of revolution from rings of `(polar angle, cap)`.
fn revolve(rings: &[(f32, f32)], segments: u32) -> Mesh {
    let mut m = Mesh::default();
    for &(theta, cap) in rings {
        let (st, ct) = theta.sin_cos();
        for c in 0..=segments {
            let phi = std::f32::consts::TAU * c as f32 / segments as f32;
            let (sp, cp) = phi.sin_cos();
            let n = [st * cp, ct, st * sp];
            m.vertices.push(Vertex3 { pos: n, normal: n, cap });
        }
    }
    let row = segments + 1;
    for r in 0..rings.len() as u32 - 1 {
        for c in 0..segments {
            let a = r * row + c;
            let b = a + 1;
            let d = a + row;
            let e = d + 1;
            m.indices.extend_from_slice(&[a, b, d, b, e, d]);
        }
    }
    m
}

pub fn sphere() -> Mesh {
    let n = RINGS * 2;
    let rings: Vec<(f32, f32)> = (0..=n).map(|i| (std::f32::consts::PI * i as f32 / n as f32, 0.0)).collect();
    revolve(&rings, SEGMENTS)
}

pub fn capsule() -> Mesh {
    let half = std::f32::consts::FRAC_PI_2;
    let mut rings: Vec<(f32, f32)> = (0..=RINGS).map(|i| (half * i as f32 / RINGS as f32, 1.0)).collect();
    rings.extend((0..=RINGS).map(|i| (half + half * i as f32 / RINGS as f32, -1.0)));
    revolve(&rings, SEGMENTS)
}

pub fn cuboid() -> Mesh {
    let mut m = Mesh::default();
    // (normal, u, v) with u x v = normal, so the corner order below is counter clockwise from outside.
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
    ];
    for (n, u, v) in faces {
        let base = m.vertices.len() as u32;
        for (su, sv) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let pos = [n[0] + su * u[0] + sv * v[0], n[1] + su * u[1] + sv * v[1], n[2] + su * u[2] + sv * v[2]];
            m.vertices.push(Vertex3 { pos, normal: n, cap: 0.0 });
        }
        m.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    m
}

pub fn plane() -> Mesh {
    let n = [0.0, 1.0, 0.0];
    let mut m = Mesh::default();
    for (x, z) in [(-1.0f32, -1.0f32), (-1.0, 1.0), (1.0, 1.0), (1.0, -1.0)] {
        m.vertices.push(Vertex3 { pos: [x, 0.0, z], normal: n, cap: 0.0 });
    }
    m.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    m
}

pub fn build(kind: MeshKind) -> Mesh {
    match kind {
        MeshKind::Sphere => sphere(),
        MeshKind::Box => cuboid(),
        MeshKind::Capsule => capsule(),
        MeshKind::Plane => plane(),
    }
}

/// All four meshes in one vertex and one index buffer.
#[derive(Clone, Debug)]
pub struct MeshSet {
    pub vertices: Vec<Vertex3>,
    pub indices: Vec<u32>,
    /// Per [`MeshKind`]: index range and the base vertex to add to its indices.
    pub ranges: [(std::ops::Range<u32>, i32); 4],
}

impl MeshSet {
    pub fn build() -> Self {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut ranges: [(std::ops::Range<u32>, i32); 4] = std::array::from_fn(|_| (0..0, 0));
        for kind in MeshKind::ALL {
            let m = build(kind);
            let base = vertices.len() as i32;
            let first = indices.len() as u32;
            vertices.extend_from_slice(&m.vertices);
            indices.extend_from_slice(&m.indices);
            ranges[kind as usize] = (first..indices.len() as u32, base);
        }
        Self { vertices, indices, ranges }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math3::{cross, dot, sub};

    /// Position as the vertex shader places it for a unit-scale instance with half length 1.
    fn placed(v: &Vertex3) -> [f32; 3] {
        [v.pos[0], v.pos[1] + v.cap, v.pos[2]]
    }

    #[test]
    fn every_triangle_faces_outward_and_normals_are_unit() {
        for kind in MeshKind::ALL {
            let m = build(kind);
            assert!(m.indices.len() % 3 == 0 && !m.vertices.is_empty(), "{kind:?}");
            for v in &m.vertices {
                let l = dot(v.normal, v.normal).sqrt();
                assert!((l - 1.0).abs() < 1e-4, "{kind:?} normal {:?}", v.normal);
            }
            for t in m.indices.chunks_exact(3) {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| m.vertices[i as usize]);
                let n = cross(sub(placed(&b), placed(&a)), sub(placed(&c), placed(&a)));
                let area2 = dot(n, n).sqrt();
                if area2 < 1e-5 {
                    continue; // a degenerate pole triangle
                }
                let avg = [
                    a.normal[0] + b.normal[0] + c.normal[0],
                    a.normal[1] + b.normal[1] + c.normal[1],
                    a.normal[2] + b.normal[2] + c.normal[2],
                ];
                assert!(dot(n, avg) > 0.0, "{kind:?}: triangle {t:?} winds against its normals");
            }
            assert!(m.indices.iter().all(|&i| (i as usize) < m.vertices.len()));
        }
    }

    #[test]
    fn sphere_vertices_lie_on_the_unit_sphere_and_box_on_the_cube() {
        for v in &sphere().vertices {
            assert!((dot(v.pos, v.pos).sqrt() - 1.0).abs() < 1e-4);
        }
        for v in &cuboid().vertices {
            assert!(v.pos.iter().all(|c| c.abs() == 1.0));
        }
    }

    #[test]
    fn capsule_halves_are_selected_by_cap() {
        let m = capsule();
        for v in &m.vertices {
            assert!(v.cap == 1.0 || v.cap == -1.0);
            // The upper half has y >= 0 and the lower y <= 0 before the offset.
            assert!(v.pos[1] * v.cap >= -1e-5, "{v:?}");
        }
    }

    #[test]
    fn mesh_set_ranges_cover_all_indices() {
        let s = MeshSet::build();
        assert_eq!(s.ranges[3].0.end as usize, s.indices.len());
        for (r, base) in &s.ranges {
            for &i in &s.indices[r.start as usize..r.end as usize] {
                assert!((i as i32 + base) < s.vertices.len() as i32);
            }
        }
    }
}
