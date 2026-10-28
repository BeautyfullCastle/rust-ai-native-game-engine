//! Opt-in CPU host for Stormglass starting-condition authoring.
//! Reuses DefenseGame through ERP/PlaySession; campaign length is bounded; runtime pools are not authored.
use std::{path::PathBuf, sync::Arc};

use orr_ecs::Frame;
use orr_edit::{BakeAdmission, EditError, EditorDoc};
use orr_games::coop_defense_game::{
    scenario_types, DefenseConfig, DefenseGame, SCENARIO_BUILD_ID, TICK_RATE,
};
use orr_reflect::{Scene, SceneIndex};
use orr_sim::Simulation;
use serde_json::json;

use crate::{GameHooks, HostLimits, LocalHost, ServerConfig};

pub const GAME: &str = "CoopDefense";
pub const INITIAL_SCENE: &str = "schema: orr.scene/1\nentities: {}\n";
pub use orr_games::coop_defense_scene::MAX_SOURCE_BYTES;
use orr_games::coop_defense_scene::{config_from_baked, validate_source};

struct InitialAdmission;

impl BakeAdmission for InitialAdmission {
    fn admit_source(&self, text: &str) -> Result<(), EditError> {
        validate_source(text).map_err(EditError::Invalid)
    }

    fn admit(&self, scene: &Scene, frame: &mut Frame, _: &SceneIndex) -> Result<(), EditError> {
        // The native player shares the same closed-shape and baked-default rules.
        let config = config_from_baked(scene, frame).map_err(EditError::Invalid)?;
        DefenseGame::setup_scenario(frame, &config, config.scenario);
        Ok(())
    }

    fn allow_play_edits(&self) -> bool {
        false
    }
}

/// Admit a bounded strict scene for the native default seed. Unknown keys,
/// types, duplicate keys and additional YAML documents fail in Scene::parse.
pub fn document(text: &str) -> Result<EditorDoc, String> {
    EditorDoc::from_yaml_with_admission(
        text,
        scenario_types(),
        Simulation::<DefenseGame>::build_registry(),
        u64::from(DefenseConfig::default().seed),
        Some(Arc::new(InitialAdmission)),
    )
    .map_err(|e| e.to_string())
}

/// Set this preset's simulation identity and scene location.
pub fn configure(limits: &mut HostLimits, path: Option<PathBuf>) {
    limits.player_count = 1;
    limits.tick_rate = TICK_RATE;
    limits.scene_path = path;
    limits.build_id = SCENARIO_BUILD_ID;
    limits.game = GameHooks::new(GAME);
    limits.view_stream = None;
}

/// Start the existing local host lifecycle with typed managed input.
/// Listener, authentication and request limits remain caller-owned.
pub fn spawn_host(
    text: String,
    path: Option<PathBuf>,
    mut cfg: ServerConfig,
    work_status: Option<crate::work_status::WorkStatusService>,
) -> Result<LocalHost, String> {
    LocalHost::spawn_with_input::<DefenseGame>(
        move || {
            let doc = document(&text)?;
            configure(&mut cfg.limits, path);
            Ok((doc, cfg))
        },
        "DefenseInput",
        1,
        |_, _| Vec::new(),
        json!({"action":0,"source_cell":0,"target_cell":0,"sequence":0}),
        work_status,
    )
}

#[cfg(test)]
mod tests;
