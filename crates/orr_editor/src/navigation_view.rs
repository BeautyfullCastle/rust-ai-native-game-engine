//! Read-only projection of the navigation host's current Frame. Terrain and
//! terrain-free route geometry are separate static models in one shared scene
//! render pass. No view data ever advances or replaces the host navigator.
use orr_bridge::FrameView;
use orr_fp::FP;
use orr_model::StaticModel;
use orr_navigation::{NavigationStatus, Navigator, TerrainGraph};
use orr_remote::navigation_yard3d::{
    navigation_from_view, NavigationScenePin, NavigationStepStatus,
    NavigationView as HostNavigationView,
};
use orr_terrain::Terrain;
use std::sync::Arc;

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
    orr_navigation_view::overlay::build_overlay(
        &view.terrain,
        &view.graph,
        &view.navigator,
        view.spec.start,
        view.spec.goal,
        stale,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_navigation::{AgentProfile, SearchBudget};
    use orr_remote::navigation_yard3d::NavigationAgentSpec;
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
        assert!(orr_navigation_view::overlay::validate_point([FP::from_int(256); 3]).is_ok());
        assert!(orr_navigation_view::overlay::validate_point([FP::from_int(-256); 3]).is_ok());
        assert!(
            orr_navigation_view::overlay::validate_point([FP::from_raw(256 * 65536 + 1); 3])
                .is_err()
        );
        assert!(
            orr_navigation_view::overlay::validate_point([FP::from_raw(-256 * 65536 - 1); 3])
                .is_err()
        );
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
