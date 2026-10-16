//! Read-only projection of the navigation host's current Frame. Terrain and
//! terrain-free route geometry are separate static models in one shared scene
//! render pass. No view data ever advances or replaces the host navigator.
use orr_bridge::FrameView;
use orr_fp::FP;
use orr_model::{
    Dependency, Image, Material, ModelSource, Primitive, StaticModel, Vertex, Wrap, IDENTITY,
};
use orr_navigation::{NavigationStatus, Navigator, TerrainGraph};
use orr_remote::navigation_yard3d::{
    navigation_from_view, NavigationScenePin, NavigationStepStatus,
    NavigationView as HostNavigationView,
};
use orr_terrain::Terrain;
use std::sync::Arc;

const OVERLAY_ASSET: &str = "navigation/editor_overlay.orrmodel";
/// Every Q16 lattice value in this inclusive range has an exact f32 display.
/// The GPU-free host/core can admit broader coordinates; the production editor
/// keeps movement and seek display exact at every possible intermediate tick.
const EDITOR_MAX_COORDINATE_RAW: i64 = 256 * 65536;

/// Preflight ordinary Editor Build before the single atomic scene-pin patch.
/// The host remains renderer-free and may admit a wider headless scene.
pub fn validate_candidate(
    terrain: &Terrain,
    graph: &TerrainGraph,
    navigator: &Navigator,
    spec: orr_remote::navigation_yard3d::NavigationAgentSpec,
) -> Result<(), String> {
    orr_terrain_view::to_static_model(terrain, &[])
        .map_err(|e| e.to_string())?
        .ok_or("navigation route has no visible terrain triangles")?;
    let candidate = HostNavigationView {
        terrain: terrain.clone(),
        graph: graph.clone(),
        navigator: navigator.clone(),
        spec,
    };
    build_overlay(&candidate, false)?;
    Ok(())
}

#[derive(Default)]
pub struct NavigationView {
    pub admitted: bool,
    pub source: String,
    pub identity: String,
    pub revision: Option<[u8; 32]>,
    pub error: Option<String>,
    pub status: Option<NavigationStatus>,
    pub position: Option<[FP; 3]>,
    pub tick: u64,
    model: Option<Arc<StaticModel>>,
    overlay_current: Option<Arc<StaticModel>>,
    overlay_stale: Option<Arc<StaticModel>>,
    overlay_pin: Option<NavigationScenePin>,
    overlay_position: Option<[FP; 3]>,
}
impl NavigationView {
    /// This model contains the terrain surface exactly once.
    pub fn model(&self) -> Option<Arc<StaticModel>> {
        self.model.clone()
    }
    /// No terrain vertices are in this model. The app submits it after terrain
    /// as a second SceneModel, so the opaque terrain cannot erase the route.
    pub fn overlay_model(&self, stale: bool) -> Option<Arc<StaticModel>> {
        if stale {
            self.overlay_stale.clone()
        } else {
            self.overlay_current.clone()
        }
    }
    pub fn update(&mut self, frame: FrameView<'_>) {
        self.tick = frame.tick();
        let pin = *frame.singleton::<NavigationScenePin>();
        let status = frame.singleton::<NavigationStepStatus>();
        self.source = pin.source().unwrap_or_default().into();
        self.identity = pin.identity().unwrap_or_default().into();
        let runtime_error = (status.failed != 0).then(|| {
            format!(
                "Navigation stopped at tick {}: {}. Stop and rebuild the route",
                status.failed_tick,
                status.message()
            )
        });
        if pin.source_len == 0 {
            self.clear();
            self.error = runtime_error;
            return;
        }
        let result = (|| {
            let view = navigation_from_view(frame)?;
            let revision = view.terrain.revision();
            let model = if self.revision == Some(revision) && self.model.is_some() {
                self.model.clone()
            } else {
                orr_terrain_view::to_static_model(&view.terrain, &[])
                    .map_err(|e| e.to_string())?
                    .map(Arc::new)
            };
            if model.is_none() {
                return Err("navigation route has no visible terrain triangles".into());
            }
            let position = view.navigator.position();
            let (current, stale) = if self.overlay_pin == Some(pin)
                && self.overlay_position == Some(position)
                && self.overlay_current.is_some()
                && self.overlay_stale.is_some()
            {
                (self.overlay_current.clone(), self.overlay_stale.clone())
            } else {
                (
                    Some(Arc::new(build_overlay(&view, false)?)),
                    Some(Arc::new(build_overlay(&view, true)?)),
                )
            };
            Ok::<_, String>((
                revision,
                model,
                current,
                stale,
                view.navigator.status(),
                position,
            ))
        })();
        match result {
            Ok((revision, model, current, stale, nav_status, position)) => {
                self.admitted = true;
                self.revision = Some(revision);
                self.model = model;
                self.overlay_current = current;
                self.overlay_stale = stale;
                self.overlay_pin = Some(pin);
                self.overlay_position = Some(position);
                self.status = Some(nav_status);
                self.position = Some(position);
                self.error = runtime_error;
            }
            Err(error) => {
                self.clear();
                self.error = Some(error);
            }
        }
    }
    fn clear(&mut self) {
        self.admitted = false;
        self.revision = None;
        self.model = None;
        self.overlay_current = None;
        self.overlay_stale = None;
        self.overlay_pin = None;
        self.overlay_position = None;
        self.status = None;
        self.position = None;
    }
}

fn build_overlay(view: &HostNavigationView, stale: bool) -> Result<StaticModel, String> {
    // Spacing only sizes decorative geometry. Unlike authoritative positions,
    // the difference between two exact endpoint coordinates need not itself
    // be exactly representable (for example 512 minus one FP lattice unit).
    let spacing = view.terrain.spacing().to_f32();
    if !spacing.is_finite() || spacing <= 0.0 {
        return Err("invalid navigation overlay spacing".into());
    }
    for point in view.graph.vertices() {
        validate_point(*point)?;
    }
    validate_point(view.navigator.position())?;
    if let Some(path) = view.navigator.path() {
        for point in path.waypoints() {
            validate_point(*point)?;
        }
    }
    for xz in [view.spec.start, view.spec.goal] {
        if let Ok(projection) = view.graph.project(&view.terrain, xz) {
            validate_point(projection.position)?;
        }
    }
    let hex = view
        .graph
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
    for tri in view.graph.triangles() {
        let points = tri
            .vertices
            .map(|v| lifted(view.graph.vertices()[v as usize]));
        for i in 0..3 {
            ribbon(
                &mut source.primitives[0],
                &view.terrain,
                points[i],
                points[(i + 1) % 3],
                spacing * 0.008,
                0.0,
            );
        }
    }
    if let Some(path) = view.navigator.path() {
        for key in path.corridor() {
            if let Some(tri) = view.graph.triangle(*key) {
                let points = tri
                    .vertices
                    .map(|v| lifted(view.graph.vertices()[v as usize]));
                for i in 0..3 {
                    ribbon(
                        &mut source.primitives[1],
                        &view.terrain,
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
                &view.terrain,
                lifted(pair[0]),
                lifted(pair[1]),
                spacing * 0.07,
                spacing * 0.045,
            );
        }
    }
    if let Ok(start) = view.graph.project(&view.terrain, view.spec.start) {
        pyramid(
            &mut source.primitives[3],
            &view.terrain,
            lifted(start.position),
            spacing * 0.09,
        );
    }
    if let Ok(goal) = view.graph.project(&view.terrain, view.spec.goal) {
        pyramid(
            &mut source.primitives[4],
            &view.terrain,
            lifted(goal.position),
            spacing * 0.09,
        );
    }
    pyramid(
        &mut source.primitives[5],
        &view.terrain,
        lifted(view.navigator.position()),
        spacing * 0.12,
    );
    // Empty primitives are not accepted by StaticModel. Retain stable material
    // slots but omit meshes that a zero-length path did not generate.
    source.primitives.retain(|mesh| !mesh.indices.is_empty());
    StaticModel::new(source).map_err(|e| e.to_string())
}
fn exact_coordinate(value: FP) -> Result<f32, String> {
    if i128::from(value.raw()).abs() > i128::from(EDITOR_MAX_COORDINATE_RAW) {
        return Err(
            "navigation editor display requires exact f32 coordinates within +/-256".into(),
        );
    }
    exact_display_scalar(value)
}
fn exact_display_scalar(value: FP) -> Result<f32, String> {
    let display = value.to_f32();
    if !display.is_finite() || f64::from(display) * 65536.0 != value.raw() as f64 {
        return Err("navigation editor display requires exact f32 values".into());
    }
    Ok(display)
}
fn validate_point(point: [FP; 3]) -> Result<(), String> {
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
    use orr_navigation::{AgentProfile, SearchBudget};
    use orr_remote::navigation_yard3d::NavigationAgentSpec;
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

    fn candidate_at(origin: i32) -> Result<(), String> {
        let terrain = Terrain::new(
            "terrain/bound.orrt".into(),
            2,
            2,
            [FP::from_int(origin); 2],
            FP::ONE,
            vec![FP::ZERO; 4],
            vec![false],
        )
        .unwrap();
        let graph = TerrainGraph::build(&terrain, AgentProfile::default()).unwrap();
        let at = |quarter: i64| FP::from_raw(i64::from(origin) * 65536 + quarter * 16384);
        let spec = NavigationAgentSpec {
            start: [at(1), at(1)],
            goal: [at(3), at(3)],
            max_slope: FP::ONE,
            distance_per_tick: FP::from_raw(8192),
        };
        let mut navigator = Navigator::new(&graph, &terrain, spec.start).unwrap();
        navigator
            .replan(&graph, &terrain, spec.goal, SearchBudget::default())
            .unwrap();
        validate_candidate(&terrain, &graph, &navigator, spec)
    }
    #[test]
    fn exact_editor_bounds_include_positive_and_negative_256_but_reject_next_lattice() {
        assert!(exact_coordinate(FP::from_int(256)).is_ok());
        assert!(exact_coordinate(FP::from_int(-256)).is_ok());
        assert!(exact_coordinate(FP::from_raw(256 * 65536 + 1)).is_err());
        assert!(exact_coordinate(FP::from_raw(-256 * 65536 - 1)).is_err());
        assert!(
            candidate_at(255).is_ok(),
            "terrain ending at +256 is supported"
        );
        assert!(
            candidate_at(-256).is_ok(),
            "negative-coordinate terrain is supported"
        );
        assert!(candidate_at(256).unwrap_err().contains("+/-256"));
        assert!(candidate_at(-257).unwrap_err().contains("+/-256"));
    }
    #[test]
    fn scalar_spacing_is_allowed_when_all_positions_fit_bounds() {
        for spacing in [FP::from_int(512), FP::from_raw(512 * 65536 - 1)] {
            let terrain = Terrain::new(
                "terrain/span.orrt".into(),
                2,
                2,
                [FP::from_int(-256); 2],
                spacing,
                vec![FP::ZERO; 4],
                vec![false],
            )
            .unwrap();
            let graph = TerrainGraph::build(&terrain, AgentProfile::default()).unwrap();
            let spec = NavigationAgentSpec {
                start: [FP::from_int(-128); 2],
                goal: [FP::from_int(128); 2],
                max_slope: FP::ONE,
                distance_per_tick: FP::ONE,
            };
            let mut navigator = Navigator::new(&graph, &terrain, spec.start).unwrap();
            navigator
                .replan(&graph, &terrain, spec.goal, SearchBudget::default())
                .unwrap();
            assert!(validate_candidate(&terrain, &graph, &navigator, spec).is_ok());
        }
    }
}
