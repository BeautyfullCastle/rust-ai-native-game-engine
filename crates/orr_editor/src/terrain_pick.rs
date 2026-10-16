//! Presentation-only picking and exact XZ query classification for terrain.
//!
//! Numeric vertex/cell indices remain authoritative. Picking never writes a
//! floating-point position back into the terrain. Cells include their virtual
//! triangles even when marked as holes, so a hole can always be selected.
use orr_fp::FP;
use orr_render::Camera3D;
use orr_terrain::{Surface, Terrain};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TerrainSelectionMode {
    #[default]
    Entity,
    Vertex,
    Cell,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainQuery {
    Outside,
    Hole { cell: [u32; 2] },
    Surface { cell: [u32; 2], surface: Surface },
}

/// The same inclusive outer-edge and +X/+Z interior-seam ownership as Terrain.
/// Subtractions and products use i128 to preserve the entire raw FP range.
pub fn owner_cell(terrain: &Terrain, x: FP, z: FP) -> Option<[u32; 2]> {
    let spacing = i128::from(terrain.spacing().raw());
    let axis = |value: FP, origin: FP, size: u32| {
        let offset = i128::from(value.raw()) - i128::from(origin.raw());
        if offset < 0 || offset > i128::from(size - 1) * spacing {
            return None;
        }
        Some((offset / spacing).min(i128::from(size - 2)) as u32)
    };
    Some([
        axis(x, terrain.origin()[0], terrain.width())?,
        axis(z, terrain.origin()[1], terrain.depth())?,
    ])
}

pub fn query(terrain: &Terrain, x: FP, z: FP) -> TerrainQuery {
    let Some(cell) = owner_cell(terrain, x, z) else {
        return TerrainQuery::Outside;
    };
    match terrain.surface(x, z) {
        Some(surface) => TerrainQuery::Surface { cell, surface },
        None => TerrainQuery::Hole { cell },
    }
}

pub fn cell_vertices(terrain: &Terrain, cell: [u32; 2]) -> Option<[[FP; 3]; 4]> {
    let [x, z] = cell;
    if x >= terrain.width() - 1 || z >= terrain.depth() - 1 {
        return None;
    }
    let a = z * terrain.width() + x;
    Some([
        terrain.vertex_position(a)?,
        terrain.vertex_position(a + 1)?,
        terrain.vertex_position(a + terrain.width() + 1)?,
        terrain.vertex_position(a + terrain.width())?,
    ])
}

/// Closest projected vertex within a small pixel radius. Equal screen distances
/// prefer the nearest depth, then stable row-major order; holes are included.
pub fn pick_vertex(
    terrain: &Terrain,
    camera: &Camera3D,
    pixel: [f32; 2],
    size: (u32, u32),
    radius: f32,
) -> Option<[u32; 2]> {
    if !valid_input(pixel, size) || !radius.is_finite() || radius < 0.0 {
        return None;
    }
    let mut closest: Option<(f32, f32, u32)> = None;
    let forward = camera.forward();
    for index in 0..terrain.width() * terrain.depth() {
        let point = terrain.vertex_position(index)?.map(FP::to_f32);
        let depth = dot(sub(point, camera.eye), forward);
        if !depth.is_finite() || depth < 0.0 {
            continue;
        }
        let Some(screen) = camera.world_to_screen(point, size) else {
            continue;
        };
        let distance = (screen[0] - pixel[0]).powi(2) + (screen[1] - pixel[1]).powi(2);
        if !distance.is_finite() || distance > radius * radius {
            continue;
        }
        let candidate = (distance, depth, index);
        if closest.is_none_or(|current| {
            distance
                .total_cmp(&current.0)
                .then_with(|| depth.total_cmp(&current.1))
                .then_with(|| index.cmp(&current.2))
                .is_lt()
        }) {
            closest = Some(candidate);
        }
    }
    closest.map(|(_, _, i)| [i % terrain.width(), i / terrain.width()])
}

/// Ray-pick the canonical a-d diagonal triangles, including hole cells.
/// This is a selection convenience, never simulation or surface-query authority.
pub fn pick_cell(
    terrain: &Terrain,
    camera: &Camera3D,
    pixel: [f32; 2],
    size: (u32, u32),
) -> Option<[u32; 2]> {
    if !valid_input(pixel, size) {
        return None;
    }
    let (origin, direction) = camera.screen_ray(pixel, size);
    if origin.iter().chain(&direction).any(|v| !v.is_finite()) {
        return None;
    }
    let mut closest: Option<(f32, [u32; 2])> = None;
    for z in 0..terrain.depth() - 1 {
        for x in 0..terrain.width() - 1 {
            let [a, b, d, c] = cell_vertices(terrain, [x, z])?.map(|p| p.map(FP::to_f32));
            for triangle in [[a, c, d], [a, d, b]] {
                if let Some(distance) = ray_triangle(origin, direction, triangle) {
                    // Later cells own seam ties, matching exact XZ seam ownership.
                    if closest.is_none_or(|(old, _)| distance <= old) {
                        closest = Some((distance, [x, z]));
                    }
                }
            }
        }
    }
    closest.map(|(_, cell)| cell)
}

fn valid_input(pixel: [f32; 2], size: (u32, u32)) -> bool {
    size.0 > 0 && size.1 > 0 && pixel.iter().all(|v| v.is_finite())
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn ray_triangle(origin: [f32; 3], direction: [f32; 3], p: [[f32; 3]; 3]) -> Option<f32> {
    let e1 = sub(p[1], p[0]);
    let e2 = sub(p[2], p[0]);
    let h = cross(direction, e2);
    let determinant = dot(e1, h);
    if determinant == 0.0 || !determinant.is_finite() {
        return None;
    }
    let s = sub(origin, p[0]);
    let u = dot(s, h) / determinant;
    let q = cross(s, e1);
    let v = dot(direction, q) / determinant;
    let distance = dot(e2, q) / determinant;
    (u >= 0.0 && v >= 0.0 && u + v <= 1.0 && distance >= 0.0 && distance.is_finite())
        .then_some(distance)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn terrain() -> Terrain {
        Terrain::new(
            "picking".into(),
            3,
            3,
            [FP::ZERO; 2],
            FP::ONE,
            vec![FP::ZERO; 9],
            vec![false, true, false, false],
        )
        .unwrap()
    }
    fn camera() -> Camera3D {
        let mut camera = Camera3D::orthographic([1.0, 10.0, 1.0], [1.0, 0.0, 1.0], 2.0);
        camera.up = [0.0, 0.0, -1.0];
        camera
    }
    #[test]
    fn picks_vertices_and_hole_cells_without_altering_exact_geometry() {
        let terrain = terrain();
        let camera = camera();
        let revision = terrain.revision();
        let pixel = camera.world_to_screen([1.5, 0.0, 0.5], (600, 600)).unwrap();
        assert_eq!(
            pick_cell(&terrain, &camera, pixel, (600, 600)),
            Some([1, 0])
        );
        let pixel = camera.world_to_screen([2.0, 0.0, 0.0], (600, 600)).unwrap();
        assert_eq!(
            pick_vertex(&terrain, &camera, pixel, (600, 600), 9.0),
            Some([2, 0])
        );
        assert_eq!(
            pick_vertex(&terrain, &camera, [0.0, 0.0], (600, 600), 9.0),
            None
        );
        assert_eq!(
            pick_cell(&terrain, &camera, [f32::NAN, 0.0], (600, 600)),
            None
        );
        assert_eq!(pick_cell(&terrain, &camera, [0.0, 0.0], (0, 600)), None);
        assert_eq!(revision, terrain.revision());
    }
    #[test]
    fn exact_queries_distinguish_holes_boundaries_and_outside() {
        let terrain = terrain();
        assert_eq!(
            query(&terrain, FP::ONE, FP::ZERO),
            TerrainQuery::Hole { cell: [1, 0] }
        );
        assert_eq!(
            query(&terrain, FP::from_int(2), FP::ZERO),
            TerrainQuery::Hole { cell: [1, 0] }
        );
        assert_eq!(
            query(&terrain, FP::from_raw(-1), FP::ZERO),
            TerrainQuery::Outside
        );
        assert_eq!(
            query(&terrain, FP::from_raw(2 * 65536 + 1), FP::ZERO),
            TerrainQuery::Outside
        );
        assert!(matches!(
            query(&terrain, FP::from_int(2), FP::from_int(2)),
            TerrainQuery::Surface { cell: [1, 1], .. }
        ));
        assert_eq!(
            owner_cell(&terrain, FP::from_raw(i64::MIN), FP::from_raw(i64::MAX)),
            None
        );
    }
    #[test]
    fn picking_nearest_terrain_triangle_uses_height_not_a_flat_plane() {
        let terrain = Terrain::new(
            "raised".into(),
            2,
            2,
            [FP::ZERO; 2],
            FP::ONE,
            vec![FP::from_int(2); 4],
            vec![true],
        )
        .unwrap();
        let camera = Camera3D::perspective([0.5, 5.0, 5.0], [0.5, 2.0, 0.5], 55.0);
        let pixel = camera.world_to_screen([0.5, 2.0, 0.5], (800, 600)).unwrap();
        assert_eq!(
            pick_cell(&terrain, &camera, pixel, (800, 600)),
            Some([0, 0])
        );
        assert_eq!(
            query(&terrain, FP::from_raw(32768), FP::from_raw(32768)),
            TerrainQuery::Hole { cell: [0, 0] }
        );
    }
}
