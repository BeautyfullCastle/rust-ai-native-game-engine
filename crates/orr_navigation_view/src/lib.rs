//! Optional view of the bounded terrain-triangle point-agent prototype.
//! Terrain, a portal-midpoint path, and an agent are primitives in ONE immutable
//! StaticModel, so ModelRenderer's independent clear/depth pass cannot erase them.
//! This is not an editor, scene, physics, avoidance, or general navmesh integration.
#![allow(clippy::float_arithmetic)]
use orr_fp::FP;
use orr_model::{Image, Material, Primitive, StaticModel, Vertex, Wrap, IDENTITY};
use orr_navigation::NavigationPath;
use orr_terrain::Terrain;

#[cfg(feature = "gpu")]
pub mod gpu;
pub mod package;
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Stale visualization is deliberately explicit. Simulation refuses to advance
/// the stale path; displaying its old geometry in red makes that failure visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathDisplay {
    Current,
    Stale,
}

fn coordinate(value: FP) -> Result<f32> {
    let display = value.to_f32();
    if display.abs() > 1.0e6 || f64::from(display) * 65536.0 != value.raw() as f64 {
        return Err("navigation display requires exact f32 coordinates within +/-1,000,000".into());
    }
    Ok(display)
}
fn position(point: [FP; 3]) -> Result<[f32; 3]> {
    Ok([
        coordinate(point[0])?,
        coordinate(point[1])?,
        coordinate(point[2])?,
    ])
}
fn solid(source: &mut orr_model::ModelSource, rgba: [u8; 4]) -> u32 {
    let image = source.images.len() as u32;
    source.images.push(Image {
        width: 1,
        height: 1,
        rgba8: rgba.to_vec(),
    });
    let material = source.materials.len() as u32;
    source.materials.push(Material {
        base_color: [1.0; 4],
        image,
        linear_filter: false,
        wrap_s: Wrap::Clamp,
        wrap_t: Wrap::Clamp,
    });
    material
}
fn normal(points: [[f32; 3]; 3]) -> [f32; 3] {
    let a = std::array::from_fn::<_, 3, _>(|i| f64::from(points[1][i]) - f64::from(points[0][i]));
    let b = std::array::from_fn::<_, 3, _>(|i| f64::from(points[2][i]) - f64::from(points[0][i]));
    let n = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    let length = n.iter().map(|v| v * v).sum::<f64>().sqrt();
    n.map(|v| (v / length) as f32)
}
fn triangle(mesh: &mut Primitive, points: [[f32; 3]; 3]) {
    let normal = normal(points);
    for position in points {
        mesh.indices.push(mesh.vertices.len() as u32);
        mesh.vertices.push(Vertex {
            position,
            normal,
            uv: [0.5; 2],
        });
    }
}
fn mesh(asset: &str, node: u32, material: u32) -> Primitive {
    Primitive {
        id: format!("{asset}#node={node}/mesh=0/primitive=0"),
        vertices: vec![],
        indices: vec![],
        material,
        transform: IDENTITY,
    }
}

/// An immutable model rebuild is required after a tick, replan, or terrain edit.
/// Path waypoints and agent anchors are fixed-point simulation outputs. Only this
/// display geometry is floating point; no presentation values drive navigation.
/// A stale path can be displayed only by deliberately selecting `Stale`.
pub fn to_static_model(
    terrain: &Terrain,
    path: Option<&NavigationPath>,
    agent: [FP; 3],
    display: PathDisplay,
) -> Result<StaticModel> {
    if display == PathDisplay::Current
        && path.is_some_and(|p| p.terrain_revision() != terrain.revision())
    {
        return Err("current navigation path is stale; explicitly display Stale or replan".into());
    }
    if terrain.sample(agent[0], agent[2]) != Some(agent[1]) {
        return Err("agent anchor does not match the loaded terrain surface".into());
    }
    let base =
        orr_terrain_view::to_static_model(terrain, &[])?.ok_or("no terrain geometry to display")?;
    let mut source = base.source().clone();
    let path_color = solid(
        &mut source,
        match display {
            PathDisplay::Current => [30, 210, 255, 255],
            PathDisplay::Stale => [255, 35, 45, 255],
        },
    );
    let agent_color = solid(&mut source, [255, 90, 24, 255]);
    let spacing = coordinate(terrain.spacing())?;
    let width = spacing * 0.075;
    let lift = spacing * 0.055;
    if let Some(path) = path {
        let extra = path
            .waypoints()
            .len()
            .saturating_sub(1)
            .saturating_mul(6)
            .saturating_add(24);
        let existing: usize = source.primitives.iter().map(|p| p.vertices.len()).sum();
        if extra > orr_model::MAX_VERTICES.saturating_sub(existing) {
            return Err("navigation display exceeds model vertex budget".into());
        }
        let mut ribbon = mesh(terrain.asset_id(), 2, path_color);
        for pair in path.waypoints().windows(2) {
            let a = position(pair[0])?;
            let b = position(pair[1])?;
            let dx = b[0] - a[0];
            let dz = b[2] - a[2];
            let length = (dx * dx + dz * dz).sqrt();
            if length == 0.0 {
                continue;
            }
            let offset = [-dz / length * width, dx / length * width];
            let corner = |p: [f32; 3], sign: f32| {
                [
                    p[0] + offset[0] * sign,
                    p[1] + lift,
                    p[2] + offset[1] * sign,
                ]
            };
            let [a0, a1, b0, b1] = [
                corner(a, -1.0),
                corner(a, 1.0),
                corner(b, -1.0),
                corner(b, 1.0),
            ];
            triangle(&mut ribbon, [a0, a1, b1]);
            triangle(&mut ribbon, [a0, b1, b0]);
        }
        if !ribbon.indices.is_empty() {
            source.primitives.push(ribbon);
        }
    }
    let [x, y, z] = position(agent)?;
    let radius = spacing * 0.18;
    let center_y = y + radius * 1.5;
    let ring = [
        [x - radius, center_y, z],
        [x, center_y, z + radius],
        [x + radius, center_y, z],
        [x, center_y, z - radius],
    ];
    let mut marker = mesh(terrain.asset_id(), 3, agent_color);
    for i in 0..4 {
        triangle(
            &mut marker,
            [ring[i], ring[(i + 1) % 4], [x, center_y + radius * 2.0, z]],
        );
        triangle(
            &mut marker,
            [ring[(i + 1) % 4], ring[i], [x, center_y - radius, z]],
        );
    }
    source.primitives.push(marker);
    Ok(StaticModel::new(source)?)
}

/// Flat 8x8 cell lab with a six-cell barrier. The shortest route passes through
/// its two-cell opening; the edit closes one opening cell and forces replanning.
pub fn fixture() -> Terrain {
    let mut holes = vec![false; 64];
    for z in 0..6 {
        holes[z * 8 + 3] = true;
    }
    Terrain::new(
        "terrain/navigation-lab.orrt".into(),
        9,
        9,
        [FP::from_int(-4); 2],
        FP::ONE,
        vec![FP::ZERO; 81],
        holes,
    )
    .expect("bounded lab terrain")
}
pub fn start() -> [FP; 2] {
    [FP::from_int(-3), FP::from_int(-3)]
}
pub fn goal() -> [FP; 2] {
    [FP::from_int(3), FP::from_int(-3)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_navigation::{AgentProfile, SearchBudget, TerrainGraph};
    #[test]
    fn terrain_path_agent_share_one_model_and_preserve_fixed_anchors() {
        let terrain = fixture();
        let graph = TerrainGraph::build(&terrain, AgentProfile::default()).unwrap();
        let path = graph
            .find_path(&terrain, start(), goal(), SearchBudget::default())
            .unwrap();
        let anchor = graph.project(&terrain, start()).unwrap().position;
        let model = to_static_model(&terrain, Some(&path), anchor, PathDisplay::Current).unwrap();
        assert_eq!(model.source().asset_id, terrain.asset_id());
        assert_eq!(model.source().primitives.len(), 3);
        assert_eq!(
            model.source().primitives[0].vertices.len(),
            terrain.triangles().len() * 3
        );
        assert_eq!(
            model.source().primitives[1].vertices.len(),
            (path.waypoints().len() - 1) * 6
        );
        assert_eq!(model.source().primitives[2].vertices.len(), 24);
        assert_eq!(anchor, [FP::from_int(-3), FP::ZERO, FP::from_int(-3)]);
        let stale = to_static_model(&terrain, Some(&path), anchor, PathDisplay::Stale).unwrap();
        assert_ne!(model.source().images, stale.source().images);
    }
    #[test]
    fn unrelated_edit_stales_current_path_even_when_anchors_still_sample_identically() {
        let terrain = fixture();
        let graph = TerrainGraph::build(&terrain, AgentProfile::default()).unwrap();
        let path = graph
            .find_path(&terrain, start(), goal(), SearchBudget::default())
            .unwrap();
        let anchor = graph.project(&terrain, start()).unwrap().position;
        let mut doc = orr_terrain::TerrainDocument::new(terrain);
        doc.apply(&[orr_terrain::Edit::SetHeight {
            x: 8,
            z: 8,
            height: FP::ONE,
        }])
        .unwrap();
        assert_eq!(doc.terrain().sample(anchor[0], anchor[2]), Some(anchor[1]));
        assert!(to_static_model(doc.terrain(), Some(&path), anchor, PathDisplay::Current).is_err());
        assert!(to_static_model(doc.terrain(), Some(&path), anchor, PathDisplay::Stale).is_ok());
    }
    #[test]
    fn invalid_agent_surface_is_rejected() {
        let terrain = fixture();
        assert!(to_static_model(&terrain, None, [FP::ZERO; 3], PathDisplay::Current).is_ok());
        assert!(to_static_model(
            &terrain,
            None,
            [FP::ZERO, FP::ONE, FP::ZERO],
            PathDisplay::Current
        )
        .is_err());
        assert!(to_static_model(
            &terrain,
            None,
            [FP::from_raw(-32768), FP::ZERO, FP::from_int(-3)],
            PathDisplay::Current
        )
        .is_err());
    }
}
