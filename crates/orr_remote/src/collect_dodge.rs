//! ERP host for the closed CollectDodgeV1 authored contract.
use crate::{GameHooks, HostLimits, LocalHost, ServerConfig};
use orr_ecs::Frame;
use orr_edit::{BakeAdmission, EditError, EditorDoc};
use orr_reflect::{Scene, SceneIndex};
use orr_sample::collect_game::CollectDodgeV1;
pub use orr_sample::collect_project::GAME;
use orr_sample::collect_project::{self as project, PreparedScene};
use orr_sim::Simulation;
use std::{path::PathBuf, sync::Arc};

pub struct InitialAdmission;
impl BakeAdmission for InitialAdmission {
    fn admit_source(&self, text: &str) -> Result<(), EditError> {
        if text.len() as u64 > project::MAX_BYTES {
            return Err(EditError::Invalid(
                "CollectDodgeV1 source exceeds 64 KiB".into(),
            ));
        }
        Ok(())
    }
    fn admit(&self, scene: &Scene, frame: &mut Frame, index: &SceneIndex) -> Result<(), EditError> {
        if scene.has_prefab_links() && !cfg!(feature = "linked-prefabs") {
            return Err(EditError::Invalid("this host was built without linked-prefabs support".into()));
        }
        project::admit_initial(scene, frame, index).map_err(EditError::Invalid)
    }
    fn allow_play_edits(&self) -> bool {
        false
    }
}
pub fn document(text: &str) -> Result<EditorDoc, String> {
    // Enforce the same byte bound and initial admission as standalone consumers.
    PreparedScene::parse(text)?;
    EditorDoc::from_yaml_with_admission(
        text,
        project::types(),
        Simulation::<CollectDodgeV1>::build_registry(),
        project::SEED,
        Some(Arc::new(InitialAdmission)),
    )
    .map_err(|e| e.to_string())
}
pub fn configure(limits: &mut HostLimits, path: Option<PathBuf>) {
    limits.player_count = 1;
    limits.tick_rate = project::TICK_RATE;
    limits.scene_path = path;
    limits.build_id = project::build_id();
    limits.game = GameHooks::new(GAME);
    limits.view_stream = None;
}
pub fn spawn_host(
    text: String,
    path: Option<PathBuf>,
    mut cfg: ServerConfig,
) -> Result<LocalHost, String> {
    LocalHost::spawn_configured::<CollectDodgeV1>(
        move || {
            let doc = document(&text)?;
            configure(&mut cfg.limits, path);
            Ok((doc, cfg))
        },
        |server| {
            server.set_structured_input::<CollectDodgeV1>("CollectInput", 1, |_, _| Vec::new());
            server.enable_managed_input();
        },
    )
}
