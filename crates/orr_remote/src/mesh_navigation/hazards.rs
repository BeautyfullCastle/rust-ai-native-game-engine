//! Explicit opt-in host profile; hazards never enter the legacy MeshNavigation registry.
//! This feature includes mesh-mission, so the legacy host retains its mission-enabled profile.
use super::{
    mesh_from_view, read_scene_yaml, MeshNavigationAdmission, MeshNavigationStream, MeshScenePin,
    MeshView, YARD_PLAYERS, YARD_SEED, YARD_TICK_RATE,
};
use crate::{GameHooks, HostLimits, LocalHost, ServerConfig, ViewStreamHook};
use orr_bridge::FrameView;
use orr_ecs::Frame;
use orr_edit::{BakeAdmission, EditError, EditorDoc};
use orr_games::mesh_navigation_game::hazards;
use orr_reflect::{Scene, SceneIndex, TypeRegistry};
use orr_sim::Simulation;
use serde_json::{json, Value as JsonValue};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub use orr_games::mesh_navigation_game::hazards::{
    HazardSphere, MeshHazardNavigation, MeshHazardRun, MeshHazards, HAZARDS_NAME, RUN_NAME,
};

pub const MESH_HAZARDS_BUILD_ID: u64 = 0x4d48_415a_0000_0001;
pub const MESH_HAZARDS_GAME: &str = "MeshHazardsV1";

pub fn hazards_types() -> TypeRegistry {
    let mut types = TypeRegistry::new();
    hazards::register_reflect(&mut types);
    types
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MeshHazardsAdmission;

impl BakeAdmission for MeshHazardsAdmission {
    fn admit(&self, scene: &Scene, frame: &mut Frame, index: &SceneIndex) -> Result<(), EditError> {
        let pin = *frame.singleton::<MeshScenePin>();
        MeshNavigationAdmission.admit(scene, frame, index)?;
        if pin.is_empty() {
            let config = hazards::read_config(frame).map_err(EditError::Invalid)?;
            let run = hazards::read_run(frame).map_err(EditError::Invalid)?;
            hazards::validate_config(&config, &pin).map_err(EditError::Invalid)?;
            hazards::validate_run(&config, &run).map_err(EditError::Invalid)?;
            if bytemuck::bytes_of(&run).iter().any(|byte| *byte != 0) {
                return Err(EditError::Invalid(
                    "runtime hazard loss cannot be authored".into(),
                ));
            }
            return Ok(());
        }
        hazards::admit(frame, pin)
            .map_err(|error| EditError::Invalid(format!("mesh hazards admission: {error}")))
    }

    fn allow_play_edits(&self) -> bool {
        false
    }
}

pub fn hazards_doc_from_yaml(yaml: &str) -> Result<EditorDoc, EditError> {
    EditorDoc::from_yaml_with_admission(
        yaml,
        hazards_types(),
        Simulation::<MeshHazardNavigation>::build_registry(),
        YARD_SEED,
        Some(Arc::new(MeshHazardsAdmission)),
    )
}

pub fn hazards_doc_from_path(path: &Path) -> Result<EditorDoc, String> {
    let yaml = read_scene_yaml(path)?;
    hazards_doc_from_yaml(&yaml).map_err(|error| format!("{}: {error}", path.display()))
}

pub fn configure_mesh_hazards(limits: &mut HostLimits) {
    limits.player_count = YARD_PLAYERS;
    limits.tick_rate = YARD_TICK_RATE;
    limits.build_id = MESH_HAZARDS_BUILD_ID;
    limits.game = GameHooks::new(MESH_HAZARDS_GAME);
    let mut stream = MeshNavigationStream::new(limits.build_id, limits.player_count);
    stream.schema.game = MESH_HAZARDS_GAME.into();
    limits.view_stream = Some(ViewStreamHook::new(stream));
}

pub fn spawn_mesh_hazards_scene_host(
    path: PathBuf,
    mut config: ServerConfig,
) -> Result<LocalHost, String> {
    LocalHost::spawn_configured::<MeshHazardNavigation>(
        move || {
            let document = hazards_doc_from_path(&path)?;
            config.limits.scene_path = Some(path);
            configure_mesh_hazards(&mut config.limits);
            Ok((document, config))
        },
        |server| {
            server.set_structured_input::<MeshHazardNavigation>(
                "MeshGoalInput",
                YARD_PLAYERS,
                |_, _| Vec::new(),
            );
            server.enable_managed_input();
        },
    )
}

pub struct HazardsView {
    pub config: MeshHazards,
    pub run: MeshHazardRun,
}

/// Read a snapshot from the explicitly selected MeshHazardsV1 registry.
/// Like other typed view helpers, this requires that registry's singletons.
pub fn from_view(frame: FrameView<'_>) -> Result<Option<HazardsView>, String> {
    let config = *frame.singleton::<MeshHazards>();
    let run = *frame.singleton::<MeshHazardRun>();
    let pin = *frame.singleton::<MeshScenePin>();
    hazards::validate_config(&config, &pin)?;
    hazards::validate_run(&config, &run)?;
    if run.lost_tick > frame.tick() {
        return Err("hazard loss tick is ahead of the snapshot".into());
    }
    if config.enabled == 0 {
        return Ok(None);
    }
    let mesh = mesh_from_view(frame)?;
    let mission = super::mission::from_view(frame)?;
    if run.lost != 0 {
        if mesh.agent.status != orr_navigation::NavigationStatus::Stopped {
            return Err("lost hazard snapshot agent must be stopped".into());
        }
        if mission.is_some_and(|mission| mission.run.won != 0) {
            return Err("hazard snapshot cannot be both lost and won".into());
        }
    }
    Ok(Some(HazardsView { config, run }))
}

pub fn patch_request(config: &MeshHazards, view: &MeshView) -> Result<JsonValue, String> {
    if view.pin.graph_revision != view.graph.revision() {
        return Err("hazards proposal graph does not match the admitted scene pin".into());
    }
    hazards::validate_config(config, &view.pin)?;
    let types = hazards_types();
    let descriptor = types
        .get(HAZARDS_NAME)
        .ok_or("MeshHazards reflection is missing")?;
    let value = crate::json::value_to_json(&descriptor.read(bytemuck::bytes_of(config)));
    if !matches!(value, JsonValue::Object(_)) {
        return Err("MeshHazards reflection did not produce an object".into());
    }
    Ok(json!({ "name": HAZARDS_NAME, "path": "", "value": value }))
}

#[cfg(test)]
#[path = "hazards/tests.rs"]
mod tests;
