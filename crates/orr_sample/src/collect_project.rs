//! Initial-only, versioned CollectDodgeV1 authoring admission.
//! Every launch host uses these rules before publishing a candidate Frame.
use orr_ecs::Frame;
use orr_fp::{FPVec2, FrameRng};
use orr_games::collect_dodge_game::{
    self as game, CollectActor, CollectDodgeV1, CollectLevel, CollectRun, HazardSpec,
};
use orr_reflect::{Scene, SceneIndex, TypeRegistry};
use orr_sim::Simulation;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

pub const GAME: &str = "CollectDodgeV1";
pub const ACTOR: &str = "CollectDodgeV1::Actor";
pub const RUN: &str = "CollectDodgeV1::Run";
pub const SEED: u64 = 42;
pub const TICK_RATE: u32 = 60;
pub const MAX_BYTES: u64 = 64 * 1024;
pub fn types() -> TypeRegistry {
    let mut types = TypeRegistry::new();
    game::register_reflect(&mut types);
    types
}

/// Stable game-code/schema profile, not a per-project identity or saved score key.
pub fn build_id() -> u64 {
    let text = format!("orr_remote_host/{}/{GAME}", env!("CARGO_PKG_VERSION"));
    let id = text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    orr_sim::frame_build_id(id)
}

/// Called only on a freshly baked initial scene, never on a live/resumed Frame.
/// Validation completes before any candidate runtime field is written.
pub fn admit_initial(scene: &Scene, frame: &mut Frame, index: &SceneIndex) -> Result<(), String> {
    if scene.to_yaml().len() as u64 > MAX_BYTES {
        return Err("canonical CollectDodgeV1 scene exceeds 64 KiB".into());
    }
    if scene
        .entities
        .values()
        .any(|entity| entity.name.as_ref().is_some_and(|name| name.len() > 128))
    {
        return Err("CollectDodgeV1 actor names are at most 128 UTF-8 bytes".into());
    }
    if scene.entities.len() < 2
        || scene.entities.len() > 1 + game::MAX_COLLECTIBLES + game::MAX_HAZARDS
    {
        return Err("CollectDodgeV1 requires 2..=49 authored actors".into());
    }
    if scene.singletons.len() != 1 || scene.singletons[0].0 != RUN {
        return Err("CollectDodgeV1 requires exactly its Run time-limit singleton".into());
    }
    let mut player = None;
    let mut collectibles = BTreeMap::new();
    let mut hazards = BTreeMap::new();
    let mut entities = Vec::new();
    for (guid, entity) in &scene.entities {
        if entity.components.len() != 1 || entity.components[0].0 != ACTOR {
            return Err("every CollectDodgeV1 entity must contain exactly one Actor".into());
        }
        let handle = index
            .entity(guid)
            .ok_or("missing baked CollectDodgeV1 actor")?;
        let actor = frame
            .get::<CollectActor>(handle)
            .ok_or("missing baked Actor component")?;
        if actor.initial_position != FPVec2::ZERO
            || actor.initial_velocity != FPVec2::ZERO
            || actor.active != 0
            || actor.reserved != 0
        {
            return Err("runtime actor state cannot be supplied as initial authoring data".into());
        }
        match actor.kind {
            game::PLAYER if actor.ordinal == 0 && actor.velocity == FPVec2::ZERO => {
                if player.replace(actor.position).is_some() {
                    return Err("exactly one player is required".into());
                }
            }
            game::COLLECTIBLE if actor.velocity == FPVec2::ZERO => {
                if collectibles.insert(actor.ordinal, actor.position).is_some() {
                    return Err("duplicate collectible ordinal".into());
                }
            }
            game::HAZARD => {
                if hazards
                    .insert(
                        actor.ordinal,
                        HazardSpec {
                            position: actor.position,
                            velocity: actor.velocity,
                        },
                    )
                    .is_some()
                {
                    return Err("duplicate hazard ordinal".into());
                }
            }
            _ => return Err("invalid actor kind, player ordinal or non-hazard velocity".into()),
        }
        entities.push(handle);
    }
    for keys in [
        collectibles.keys().copied().collect::<Vec<_>>(),
        hazards.keys().copied().collect::<Vec<_>>(),
    ] {
        if keys.iter().enumerate().any(|(i, k)| *k != i as u32) {
            return Err("actor ordinals must be contiguous from zero within each kind".into());
        }
    }
    let run = *frame.singleton::<CollectRun>();
    if run.phase != 0
        || run.score != 0
        || run.elapsed_ticks != 0
        || run.goal != 0
        || run.restart_held != 0
    {
        return Err("runtime run state cannot be supplied as initial authoring data".into());
    }
    let goal = collectibles.len() as u32;
    CollectLevel::new(
        player.ok_or("exactly one player is required")?,
        collectibles.into_values().collect(),
        hazards.into_values().collect(),
        run.time_limit_ticks,
    )?;
    for entity in entities {
        let actor = frame
            .get_mut::<CollectActor>(entity)
            .expect("validated actor");
        actor.initial_position = actor.position;
        actor.initial_velocity = actor.velocity;
        actor.active = 1;
    }
    frame.set_singleton(CollectRun { goal, ..run });
    Ok(())
}

pub struct PreparedScene {
    text: Arc<str>,
    frame: Frame,
    index: SceneIndex,
}
impl PreparedScene {
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() as u64 > MAX_BYTES {
            return Err("CollectDodgeV1 scene exceeds 64 KiB".into());
        }
        let types = types();
        let scene = Scene::parse(text, &types).map_err(|e| format!("CollectDodgeV1 scene: {e}"))?;
        let mut frame = Frame::new(Simulation::<CollectDodgeV1>::build_registry());
        frame.set_singleton(FrameRng::new(SEED));
        let index = scene
            .bake(&types, &mut frame)
            .map_err(|e| format!("CollectDodgeV1 bake: {e}"))?;
        admit_initial(&scene, &mut frame, &index)?;
        Ok(Self {
            text: text.into(),
            frame,
            index,
        })
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn frame(&self) -> &Frame {
        &self.frame
    }
    pub fn index(&self) -> &SceneIndex {
        &self.index
    }
    pub fn session(&self) -> Result<orr_bridge::PlaySession<CollectDodgeV1>, String> {
        let mut config = orr_bridge::PlayConfig::new(1, SEED, TICK_RATE);
        config.game_id = GAME.into();
        config.build_id = build_id();
        config.start_paused = false;
        orr_bridge::PlaySession::from_frame(config, &self.frame).map_err(|e| e.to_string())
    }
    pub fn simulation(&self) -> Result<Simulation<CollectDodgeV1>, String> {
        Simulation::from_frame(&self.frame, TICK_RATE, build_id()).map_err(|e| e.to_string())
    }
}

pub struct PreparedProject {
    root: PathBuf,
    path: PathBuf,
    scene: PreparedScene,
}
impl PreparedProject {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, String> {
        // Capability inventory is route-local, not Cargo feature unification.
        // This first authored adapter intentionally consumes no sprite/UI package.
        let project =
            orr_package::Project::open(root.as_ref(), orr_package::Runtime::content_only())
                .map_err(|e| e.to_string())?;
        project.verify().map_err(|e| e.to_string())?;
        let manifest = project
            .manifest()
            .ok_or("CollectDodgeV1 requires schema-2 project manifest")?;
        let entry = manifest
            .entry
            .as_ref()
            .ok_or("CollectDodgeV1 requires a project entry")?;
        if manifest.schema != 2 || entry.game != orr_package::ProjectGame::CollectDodgeV1 {
            return Err("project entry must be schema 2 and collect-dodge-v1".into());
        }
        if entry.sprites.is_some() || entry.ui.is_some() {
            return Err(
                "CollectDodgeV1 authored route does not yet support sprite or UI declarations"
                    .into(),
            );
        }
        let root = project.root().to_path_buf();
        let path = crate::project::entry_file(&root, &entry.scene)?;
        let bytes = crate::project::read_regular(&path, MAX_BYTES)?;
        let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
        let scene = PreparedScene::parse(&text)?;
        Ok(Self { root, path, scene })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn scene(&self) -> &PreparedScene {
        &self.scene
    }
}
