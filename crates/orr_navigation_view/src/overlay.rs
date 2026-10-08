//! Shared read-only route geometry. Submit this terrain-free model alongside
//! terrain in one ImportedSceneRenderer pass; never draw independent clearing passes.
use orr_fp::FP;
use orr_model::{
    Dependency, Image, Material, ModelSource, Primitive, StaticModel, Vertex, Wrap, IDENTITY,
};
use orr_navigation::{Navigator, TerrainGraph};
use orr_terrain::Terrain;

// Preserve the existing asset identity used by editor capture consumers.
const OVERLAY_ASSET: &str = "navigation/editor_overlay.orrmodel";
/// Every Q16 lattice point inside this inclusive XYZ envelope is exact in f32.
pub const MAX_COORDINATE_RAW: i64 = 256 * 65536;

/// Terrain-free overlay shared by the production editor and standalone player.
/// This function reads fixed-point results; it never moves or replans the agent.
pub fn build_overlay(
    terrain: &Terrain,
    graph: &TerrainGraph,
    navigator: &Navigator,
    start: [FP; 2],
    goal: [FP; 2],
    stale: bool,
) -> Result<StaticModel, String> {
    // Spacing only sizes decorative geometry. Unlike authoritative positions,
    // the difference between two exact endpoint coordinates need not itself
    // be exactly representable (for example 512 minus one FP lattice unit).
    let spacing = terrain.spacing().to_f32();
    if !spacing.is_finite() || spacing <= 0.0 {
        return Err("invalid navigation overlay spacing".into());
    }
    for point in graph.vertices() {
        validate_point(*point)?;
    }
    validate_point(navigator.position())?;
    if let Some(path) = navigator.path() {
        for point in path.waypoints() {
            validate_point(*point)?;
        }
    }
    for xz in [start, goal] {
        if let Ok(projection) = graph.project(terrain, xz) {
            validate_point(projection.position)?;
        }
    }
    let hex = graph
        .revision()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let mut source = ModelSource {
        format: "orr_static_model".into(),
        version: 1,
        asset_id: OVERLAY_ASSET.into(),
        dependencies: vec![Dependency {
            uri: "$source".into(),
            sha256: hex,
        }],
        primitives: Vec::new(),
        materials: Vec::new(),
        images: Vec::new(),
    };
    let route_color = if stale {
        [255, 35, 45, 255]
    } else {
        [0, 235, 255, 255]
    };
    let corridor_color = if stale {
        [255, 35, 45, 255]
    } else {
        [255, 185, 25, 255]
    };
    let colors = [
        [22, 175, 105, 255],
        corridor_color,
        route_color,
        [45, 245, 65, 255],
        [255, 130, 25, 255],
        [250, 25, 235, 255],
    ];
    for (index, rgba8) in colors.into_iter().enumerate() {
        let image = source.images.len() as u32;
        source.images.push(Image {
            width: 1,
            height: 1,
            rgba8: rgba8.to_vec(),
        });
        let material = source.materials.len() as u32;
        source.materials.push(Material {
            base_color: [1.0; 4],
            image,
            linear_filter: false,
            wrap_s: Wrap::Clamp,
            wrap_t: Wrap::Clamp,
        });
        source.primitives.push(Primitive {
            id: format!("{OVERLAY_ASSET}#node={index}/mesh=0/primitive=0"),
            vertices: Vec::new(),
            indices: Vec::new(),
            material,
            transform: IDENTITY,
        });
    }
    let lifted = |point: [FP; 3]| {
        let mut p = point.map(FP::to_f32);
        p[1] += spacing * 0.045;
        p
    };
    // Accepted triangles only: holes/rejected slopes contribute no face or edge.
    for tri in graph.triangles() {
        let points = tri.vertices.map(|v| lifted(graph.vertices()[v as usize]));
        for i in 0..3 {
            ribbon(
                &mut source.primitives[0],
                terrain,
                points[i],
                points[(i + 1) % 3],
                spacing * 0.008,
                0.0,
            );
        }
    }
    if let Some(path) = navigator.path() {
        for key in path.corridor() {
            if let Some(tri) = graph.triangle(*key) {
                let points = tri.vertices.map(|v| lifted(graph.vertices()[v as usize]));
                for i in 0..3 {
                    ribbon(
                        &mut source.primitives[1],
                        terrain,
                        points[i],
                        points[(i + 1) % 3],
                        spacing * 0.022,
                        spacing * 0.018,
                    );
                }
            }
        }
        for pair in path.waypoints().windows(2) {
            ribbon(
                &mut source.primitives[2],
                terrain,
                lifted(pair[0]),
                lifted(pair[1]),
                spacing * 0.07,
                spacing * 0.045,
            );
        }
    }
    if let Ok(start) = graph.project(terrain, start) {
        pyramid(
            &mut source.primitives[3],
            terrain,
            lifted(start.position),
            spacing * 0.09,
        );
    }
    if let Ok(goal) = graph.project(terrain, goal) {
        pyramid(
            &mut source.primitives[4],
            terrain,
            lifted(goal.position),
            spacing * 0.09,
        );
    }
    pyramid(
        &mut source.primitives[5],
        terrain,
        lifted(navigator.position()),
        spacing * 0.12,
    );
    // Empty primitives are not accepted by StaticModel. Retain stable material
    // slots but omit meshes that a zero-length path did not generate.
    source.primitives.retain(|mesh| !mesh.indices.is_empty());
    StaticModel::new(source).map_err(|e| e.to_string())
}
fn exact_coordinate(value: FP) -> Result<f32, String> {
    if i128::from(value.raw()).abs() > i128::from(MAX_COORDINATE_RAW) {
        return Err("navigation display requires exact f32 coordinates within +/-256".into());
    }
    exact_display_scalar(value)
}
fn exact_display_scalar(value: FP) -> Result<f32, String> {
    let display = value.to_f32();
    if !display.is_finite() || f64::from(display) * 65536.0 != value.raw() as f64 {
        return Err("navigation display requires exact f32 values".into());
    }
    Ok(display)
}
pub fn validate_point(point: [FP; 3]) -> Result<(), String> {
    for value in point {
        exact_coordinate(value)?;
    }
    Ok(())
}
fn triangle(mesh: &mut Primitive, points: [[f32; 3]; 3]) {
    for position in points {
        mesh.indices.push(mesh.vertices.len() as u32);
        mesh.vertices.push(Vertex {
            position,
            normal: [0.0, 1.0, 0.0],
            uv: [0.5; 2],
        });
    }
}
fn ribbon(
    mesh: &mut Primitive,
    terrain: &Terrain,
    a: [f32; 3],
    b: [f32; 3],
    width: f32,
    lift: f32,
) {
    let dx = b[0] - a[0];
    let dz = b[2] - a[2];
    let length = (dx * dx + dz * dz).sqrt();
    if length < 1e-8 {
        return;
    }
    let offset = [-dz / length * width * 0.5, dx / length * width * 0.5];
    let corner = |p: [f32; 3], sign: f32| {
        let mut corner = [
            p[0] + offset[0] * sign,
            p[1] + lift,
            p[2] + offset[1] * sign,
        ];
        raise_above_surface(terrain, &mut corner, width * 0.25);
        corner
    };
    let mut p = [
        corner(a, -1.0),
        corner(a, 1.0),
        corner(b, 1.0),
        corner(b, -1.0),
    ];
    // A strip may cross a canonical terrain diagonal between its corners.
    // Check its center and edge midpoints before rasterization as well.
    for (i, j) in [(0, 2), (1, 3), (0, 1), (2, 3)] {
        let x = (p[i][0] + p[j][0]) * 0.5;
        let z = (p[i][2] + p[j][2]) * 0.5;
        if let Some(surface) = sampled_height(terrain, x, z) {
            let floor = surface + width * 0.25;
            p[i][1] = p[i][1].max(floor);
            p[j][1] = p[j][1].max(floor);
        }
    }
    triangle(mesh, [p[0], p[1], p[2]]);
    triangle(mesh, [p[0], p[2], p[3]]);
    triangle(mesh, [p[2], p[1], p[0]]);
    triangle(mesh, [p[3], p[2], p[0]]);
}
fn pyramid(mesh: &mut Primitive, terrain: &Terrain, center: [f32; 3], radius: f32) {
    let y = center[1] + radius * 0.6;
    let mut base = [
        [center[0] - radius, y, center[2]],
        [center[0], y, center[2] + radius],
        [center[0] + radius, y, center[2]],
        [center[0], y, center[2] - radius],
    ];
    for point in &mut base {
        raise_above_surface(terrain, point, radius * 0.25);
    }
    let tip = [
        center[0],
        base.iter().map(|p| p[1]).fold(y, f32::max) + radius * 2.2,
        center[2],
    ];
    for i in 0..4 {
        triangle(mesh, [base[i], base[(i + 1) % 4], tip]);
        triangle(mesh, [tip, base[(i + 1) % 4], base[i]]);
    }
}
/// Presentation-only resampling avoids wide ribbons disappearing below a
/// steep but accepted slope. Quantized lookups never feed back into the host.
fn raise_above_surface(terrain: &Terrain, point: &mut [f32; 3], lift: f32) {
    if let Some(y) = sampled_height(terrain, point[0], point[2]) {
        point[1] = point[1].max(y + lift);
    }
}
fn sampled_height(terrain: &Terrain, x: f32, z: f32) -> Option<f32> {
    let fp = |v: f32| FP::from_raw((f64::from(v) * 65536.0).round() as i64);
    terrain.sample(fp(x), fp(z)).map(FP::to_f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn widened_ribbon_follows_a_steep_admitted_surface() {
        let terrain = Terrain::new(
            "terrain/steep.orrt".into(),
            2,
            2,
            [FP::ZERO; 2],
            FP::ONE,
            vec![FP::ZERO, FP::from_int(8), FP::ZERO, FP::from_int(8)],
            vec![false],
        )
        .unwrap();
        let mut mesh = Primitive {
            id: String::new(),
            vertices: Vec::new(),
            indices: Vec::new(),
            material: 0,
            transform: IDENTITY,
        };
        ribbon(
            &mut mesh,
            &terrain,
            [0.5, 4.05, 0.1],
            [0.5, 4.05, 0.9],
            0.1,
            0.0,
        );
        assert!(!mesh.vertices.is_empty());
        for vertex in mesh.vertices {
            let [x, y, z] = vertex.position;
            let fp = |v: f32| FP::from_raw((f64::from(v) * 65536.0).round() as i64);
            let surface = terrain.sample(fp(x), fp(z)).unwrap().to_f32();
            assert!(
                y > surface,
                "ribbon corner {x},{z} fell below the steep terrain: {y} <= {surface}"
            );
        }
    }

    #[test]
    fn exact_xyz_bounds_accept_endpoints_and_reject_next_lattice() {
        for axis in 0..3 {
            for raw in [-MAX_COORDINATE_RAW, MAX_COORDINATE_RAW] {
                let mut point = [FP::ZERO; 3];
                point[axis] = FP::from_raw(raw);
                assert!(validate_point(point).is_ok());
                point[axis] = FP::from_raw(raw + raw.signum());
                assert!(validate_point(point).is_err());
            }
        }
    }
}
