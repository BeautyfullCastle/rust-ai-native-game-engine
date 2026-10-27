//! Opt-in CPU host for Stormglass's fixed initial preset.
//!
//! This reuses DefenseGame through ERP/PlaySession; it is not an editor view
//! or an authored wave format. Only an empty scene is admitted. Comments
//! survive Save, but authoritative initial state comes from Game::setup.
use std::{path::PathBuf, sync::Arc};

use orr_ecs::Frame;
use orr_edit::{BakeAdmission, EditError, EditorDoc};
use orr_games::coop_defense_game::{DefenseConfig, DefenseGame, TICK_RATE};
use orr_reflect::{Scene, SceneIndex, TypeRegistry};
use orr_sim::{Game, Simulation};
use serde_json::json;

use crate::{GameHooks, HostLimits, LocalHost, ServerConfig};

pub const GAME: &str = "CoopDefense";
pub const INITIAL_SCENE: &str = "schema: orr.scene/1\nentities: {}\n";
/// Bound parsing and source text retained for Save, not the world size.
pub const MAX_SOURCE_BYTES: usize = 64 * 1024;

struct InitialAdmission;

impl BakeAdmission for InitialAdmission {
    fn admit_source(&self, text: &str) -> Result<(), EditError> {
        if text.len() > MAX_SOURCE_BYTES {
            return Err(EditError::Invalid(
                "CoopDefense source exceeds 64 KiB".into(),
            ));
        }
        Ok(())
    }

    fn admit(&self, scene: &Scene, frame: &mut Frame, _: &SceneIndex) -> Result<(), EditError> {
        if !scene.entities.is_empty() || !scene.singletons.is_empty() || scene.has_prefab_links() {
            return Err(EditError::Invalid(
                "CoopDefense accepts only its empty initial scene".into(),
            ));
        }
        // Baking supplies a fresh candidate, never a live game frame.
        DefenseGame::setup(frame, &DefenseConfig::default());
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
        TypeRegistry::new(),
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
    limits.build_id = crate::default_build_id(GAME);
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
