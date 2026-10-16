//! Optional presentation bridge for deterministic heightfields.
//!
//! Canonical topology and query results come from `orr_terrain`; this crate only
//! converts positions/normals to floats for `orr_model`. Enable `gpu` for the
//! existing `orr_render::ModelRenderer` and the offscreen `terrain_lab` binary.
//! Simulation crates must not depend on this presentation crate.
#![allow(clippy::float_arithmetic)]

use orr_fp::FP;
use orr_model::{
    Dependency, Image, Material, ModelSource, Primitive, StaticModel, Vertex, Wrap, IDENTITY,
};
use orr_terrain::Terrain;

#[derive(Debug)]
pub enum Error {
    CoordinatePrecision,
    VertexBudget,
    QueryMismatch,
    Model(orr_model::Error),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CoordinatePrecision => {
                f.write_str("terrain display requires exact f32 coordinates within +/-1,000,000")
            }
            Self::VertexBudget => {
                f.write_str("terrain and markers exceed the static model vertex budget")
            }
            Self::QueryMismatch => {
                f.write_str("query marker no longer matches this terrain; resample after editing")
            }
            Self::Model(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for Error {}
impl From<orr_model::Error> for Error {
    fn from(e: orr_model::Error) -> Self {
        Self::Model(e)
    }
}
fn display_coordinate(v: FP) -> Result<f32, Error> {
    let display = v.to_f32();
    if display.abs() > 1.0e6 || f64::from(display) * 65536.0 != v.raw() as f64 {
        return Err(Error::CoordinatePrecision);
    }
    Ok(display)
}

#[cfg(feature = "gpu")]
pub mod gpu;
pub mod package;

/// A height-query marker. Invalid/outside/hole queries produce no marker.
/// The stored anchor is the exact simulation result; display geometry is raised
/// slightly above the anchor to avoid z-fighting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueryMarker {
    anchor: [FP; 3],
}
impl QueryMarker {
    pub fn anchor(&self) -> [FP; 3] {
        self.anchor
    }
    pub fn sample(terrain: &Terrain, x: FP, z: FP) -> Option<Self> {
        terrain.sample(x, z).map(|y| Self { anchor: [x, y, z] })
    }
}

/// Build the canonical, non-hole triangles with flat face normals and a small
/// green checker texture. Markers share the same model/depth pass as the terrain.
///
/// `None` means there are neither terrain triangles nor markers: StaticModel
/// deliberately disallows empty geometry. No fake triangles fill terrain holes.
/// The existing static-model coordinate/vertex budgets are also enforced here.
pub fn to_static_model(
    terrain: &Terrain,
    markers: &[QueryMarker],
) -> Result<Option<StaticModel>, Error> {
    let triangles = terrain.triangles();
    if markers.len() > orr_model::MAX_VERTICES / 24
        || triangles.len() * 3 + markers.len() * 24 > orr_model::MAX_VERTICES
    {
        return Err(Error::VertexBudget);
    }
    let asset_id = terrain.asset_id().to_owned();
    let mut primitives = Vec::new();
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for triangle in triangles {
        let mut points = [[0.0; 3]; 3];
        for (point, index) in points.iter_mut().zip(triangle) {
            let fixed = terrain
                .vertex_position(index)
                .expect("canonical terrain vertex");
            for (out, v) in point.iter_mut().zip(fixed) {
                *out = display_coordinate(v)?;
            }
        }
        let normal = face_normal(points);
        for point in points {
            indices.push(vertices.len() as u32);
            vertices.push(Vertex {
                position: point,
                normal,
                uv: [
                    (point[0] - terrain.origin()[0].to_f32()) / terrain.spacing().to_f32(),
                    (point[2] - terrain.origin()[1].to_f32()) / terrain.spacing().to_f32(),
                ],
            });
        }
    }
    if !indices.is_empty() {
        primitives.push(Primitive {
            id: format!("{asset_id}#node=0/mesh=0/primitive=0"),
            vertices,
            indices,
            material: 0,
            transform: IDENTITY,
        });
    }
    // One combined marker primitive bounds allocation and draw count.
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for marker in markers {
        let [a, b, c] = marker.anchor;
        if terrain.sample(a, c) != Some(b) {
            return Err(Error::QueryMismatch);
        }
        let [x, y, z] = [
            display_coordinate(a)?,
            display_coordinate(b)?,
            display_coordinate(c)?,
        ];
        let r = terrain.spacing().to_f32() * 0.12;
        let center_y = y + r * 1.5;
        let ring = [
            [x - r, center_y, z],
            [x, center_y, z + r],
            [x + r, center_y, z],
            [x, center_y, z - r],
        ];
        let top = [x, center_y + r * 2.0, z];
        let bottom = [x, center_y - r, z];
        for i in 0..4 {
            for triangle in [
                [ring[i], ring[(i + 1) % 4], top],
                [ring[(i + 1) % 4], ring[i], bottom],
            ] {
                let normal = face_normal(triangle);
                for position in triangle {
                    indices.push(vertices.len() as u32);
                    vertices.push(Vertex {
                        position,
                        normal,
                        uv: [0.5; 2],
                    });
                }
            }
        }
    }
    if !indices.is_empty() {
        primitives.push(Primitive {
            id: format!("{asset_id}#node=1/mesh=0/primitive=0"),
            vertices,
            indices,
            material: 1,
            transform: IDENTITY,
        });
    }
    if primitives.is_empty() {
        return Ok(None);
    }
    let revision: String = terrain
        .revision()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    StaticModel::new(ModelSource {
        format: "orr_static_model".into(),
        version: 1,
        asset_id,
        dependencies: vec![Dependency {
            uri: "$source".into(),
            sha256: revision,
        }],
        primitives,
        materials: vec![material(0), material(1)],
        images: vec![
            Image {
                width: 2,
                height: 2,
                rgba8: vec![
                    62, 150, 91, 255, 89, 179, 113, 255, 89, 179, 113, 255, 62, 150, 91, 255,
                ],
            },
            Image {
                width: 1,
                height: 1,
                rgba8: vec![255, 90, 24, 255],
            },
        ],
    })
    .map(Some)
    .map_err(Error::from)
}
fn material(image: u32) -> Material {
    Material {
        base_color: [1.0; 4],
        image,
        linear_filter: false,
        wrap_s: Wrap::Repeat,
        wrap_t: Wrap::Repeat,
    }
}
fn face_normal(p: [[f32; 3]; 3]) -> [f32; 3] {
    let u = std::array::from_fn::<_, 3, _>(|i| f64::from(p[1][i]) - f64::from(p[0][i]));
    let v = std::array::from_fn::<_, 3, _>(|i| f64::from(p[2][i]) - f64::from(p[0][i]));
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let len = n.iter().map(|a| a * a).sum::<f64>().sqrt();
    // Degenerate float-space geometry deliberately fails StaticModel validation.
    n.map(|v| (v / len) as f32)
}

/// Small synthetic starting point used by both the runnable lab and acceptance.
/// Nine vertices per side, eight cells, no assets or network needed.
pub fn fixture() -> Terrain {
    let mut heights = Vec::new();
    for z in 0_i32..9 {
        for x in 0_i32..9 {
            let rise = (3 - (x - 4).abs()).max(0) * (3 - (z - 4).abs()).max(0);
            heights.push(FP::from_raw(i64::from(rise) * 16384));
        }
    }
    Terrain::new(
        "terrain/lab.orrt".into(),
        9,
        9,
        [FP::from_int(-4); 2],
        FP::ONE,
        heights,
        vec![false; 64],
    )
    .expect("bounded fixture")
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_terrain::{Edit, TerrainDocument};
    #[test]
    fn canonical_topology_normals_holes_queries_and_reload_match() {
        let mut document = TerrainDocument::new(fixture());
        document
            .apply(&[Edit::SetHole {
                x: 2,
                z: 3,
                hole: true,
            }])
            .unwrap();
        let terrain = Terrain::load(&document.terrain().cook()).unwrap();
        let model = to_static_model(&terrain, &[]).unwrap().unwrap();
        let mesh = &model.source().primitives[0];
        let canonical: Vec<_> = terrain.triangles().into_iter().collect();
        assert_eq!(mesh.indices.len(), canonical.len() * 3);
        for (triangle, expected) in mesh.vertices.chunks_exact(3).zip(canonical) {
            let positions = expected.map(|index| terrain.vertex_position(index).unwrap());
            let centroid = |axis: usize| {
                FP::from_raw(
                    (positions
                        .iter()
                        .map(|p| i128::from(p[axis].raw()))
                        .sum::<i128>()
                        / 3) as i64,
                )
            };
            let surface = terrain.surface(centroid(0), centroid(2)).unwrap();
            for (display, exact) in triangle[0].normal.iter().zip(surface.normal) {
                assert!((*display - exact.to_f32()).abs() < 4.0 / 65536.0);
            }
            for (v, index) in triangle.iter().zip(expected) {
                assert_eq!(
                    v.position,
                    terrain.vertex_position(index).unwrap().map(FP::to_f32)
                );
                assert!(v.normal[1] > 0.0);
            }
        }
        assert!(
            QueryMarker::sample(&terrain, FP::from_raw(-98304), FP::from_raw(-32768)).is_none()
        );
        let marker = QueryMarker::sample(&terrain, FP::ZERO, FP::ZERO).unwrap();
        assert_eq!(
            marker.anchor[1],
            terrain.sample(FP::ZERO, FP::ZERO).unwrap()
        );
        assert_eq!(
            to_static_model(&terrain, &[marker])
                .unwrap()
                .unwrap()
                .source()
                .primitives
                .len(),
            2
        );
        let empty = Terrain::new(
            "empty.orrt".into(),
            2,
            2,
            [FP::ZERO; 2],
            FP::ONE,
            vec![FP::ZERO; 4],
            vec![true],
        )
        .unwrap();
        assert!(to_static_model(&empty, &[]).unwrap().is_none());
    }
    #[test]
    fn display_rejects_large_and_subprecision_coordinates_without_touching_core() {
        for origin in [FP::from_int(1_000_001), FP::from_raw(16_777_217)] {
            let terrain = Terrain::new(
                "range.orrt".into(),
                2,
                2,
                [origin; 2],
                FP::ONE,
                vec![FP::ZERO; 4],
                vec![false],
            )
            .unwrap();
            assert_eq!(terrain.sample(origin, origin), Some(FP::ZERO));
            assert!(matches!(
                to_static_model(&terrain, &[]),
                Err(Error::CoordinatePrecision)
            ));
        }
        let terrain = Terrain::new(
            "small.orrt".into(),
            2,
            2,
            [FP::ZERO; 2],
            FP::from_raw(1),
            vec![FP::ZERO; 4],
            vec![false],
        )
        .unwrap();
        assert!(to_static_model(&terrain, &[]).unwrap().is_some());
    }
    #[test]
    fn marker_budget_is_checked_before_geometry_allocation() {
        let terrain = fixture();
        let marker = QueryMarker::sample(&terrain, FP::ZERO, FP::ZERO).unwrap();
        let markers = vec![marker; orr_model::MAX_VERTICES / 24 + 1];
        assert!(matches!(
            to_static_model(&terrain, &markers),
            Err(Error::VertexBudget)
        ));
    }
    #[test]
    fn stale_query_marker_is_rejected_after_height_or_hole_edit() {
        let mut doc = TerrainDocument::new(fixture());
        let marker = QueryMarker::sample(doc.terrain(), FP::ZERO, FP::ZERO).unwrap();
        doc.apply(&[Edit::SetHeight {
            x: 4,
            z: 4,
            height: FP::from_int(4),
        }])
        .unwrap();
        assert!(matches!(
            to_static_model(doc.terrain(), &[marker]),
            Err(Error::QueryMismatch)
        ));
        let marker = QueryMarker::sample(doc.terrain(), FP::ZERO, FP::ZERO).unwrap();
        doc.apply(&[Edit::SetHole {
            x: 4,
            z: 4,
            hole: true,
        }])
        .unwrap();
        assert!(matches!(
            to_static_model(doc.terrain(), &[marker]),
            Err(Error::QueryMismatch)
        ));
    }
}
