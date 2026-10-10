//! Closed RoomEscapeV1 project admission. File and package reads finish before startup.
use orr_ecs::Frame;
use orr_fp::FrameRng;
use orr_games::room_escape_game::{self as game, RoomEscapeV1};
use orr_model_bindings::model_bindings::{Document, LoadedAsset, ModelKind};
use orr_reflect::{Guid, Scene, SceneIndex, TypeRegistry};
use orr_sim::Simulation;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

pub const GAME: &str = "RoomEscapeV1";
pub const ACTOR: &str = "RoomEscapeV1::Actor";
pub const RUN: &str = "RoomEscapeV1::Run";
pub const SEED: u64 = 42;
pub const MAX_SCENE_BYTES: u64 = 256 * 1024;
pub const MAX_MODEL_ASSETS: usize = 8;
pub const MAX_MODEL_BINDINGS: usize = 68;
pub const MAX_DECODED_MODEL_BYTES: usize = 64 * 1024 * 1024;

pub fn types() -> TypeRegistry {
    let mut types = TypeRegistry::new();
    game::register_reflect(&mut types);
    types
}
pub fn build_id() -> u64 {
    let text = format!("orr_remote_host/{}/{GAME}", env!("CARGO_PKG_VERSION"));
    let id = text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    orr_sim::frame_build_id(id)
}

pub struct PreparedScene {
    text: Arc<str>,
    frame: Frame,
    index: SceneIndex,
}
impl PreparedScene {
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() as u64 > MAX_SCENE_BYTES {
            return Err("room scene exceeds 256 KiB".into());
        }
        let types = types();
        let scene = Scene::parse(text, &types).map_err(|e| format!("room scene: {e}"))?;
        validate_document(&scene)?;
        let mut frame = Frame::new(Simulation::<RoomEscapeV1>::build_registry());
        frame.set_singleton(FrameRng::new(SEED));
        let index = scene
            .bake(&types, &mut frame)
            .map_err(|e| format!("room bake: {e}"))?;
        game::validate_initial_frame(&frame)?;
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
    pub fn session(&self) -> Result<orr_bridge::PlaySession<RoomEscapeV1>, String> {
        let mut config = orr_bridge::PlayConfig::new(1, SEED, game::TICK_RATE);
        config.game_id = GAME.into();
        config.build_id = build_id();
        config.start_paused = false;
        orr_bridge::PlaySession::from_frame(config, &self.frame).map_err(|e| e.to_string())
    }
    pub fn simulation(&self) -> Result<Simulation<RoomEscapeV1>, String> {
        Simulation::from_frame(&self.frame, game::TICK_RATE, build_id()).map_err(|e| e.to_string())
    }
}
/// The same pre-bake contract is used by the editor's scene validator.
pub fn validate_document(scene: &Scene) -> Result<(), String> {
    if scene.has_prefab_links() {
        return Err("room projects do not yet support linked prefabs".into());
    }
    if !(3..=68).contains(&scene.entities.len()) {
        return Err("room requires 3..=68 actors".into());
    }
    let singleton_names: BTreeSet<_> = scene
        .singletons
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    if scene.singletons.len() != 2
        || singleton_names != BTreeSet::from([RUN, "orr_physics3d::PhysicsState"])
    {
        return Err("room requires exactly RoomEscapeV1::Run and PhysicsState singletons".into());
    }
    for entity in scene.entities.values() {
        if entity.name.as_ref().is_some_and(|name| name.len() > 128) {
            return Err("room actor name exceeds128 bytes".into());
        }
        let names: BTreeSet<_> = entity
            .components
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        if entity.components.len() != 3
            || names != BTreeSet::from([ACTOR, "orr_physics3d::Body", "orr_physics3d::Collider"])
        {
            return Err("room actors require exactly Actor, Body and Collider".into());
        }
    }
    Ok(())
}

/// Explicit closed consuming-route inventory; unified animation/UI features grant no support.
pub fn compiled_runtime() -> orr_package::Runtime {
    let mut runtime = orr_package::Runtime::content_only();
    runtime.capabilities.insert("models".into());
    runtime
}
pub struct PreparedModels {
    pub path: PathBuf,
    pub document: Document,
    pub assets: BTreeMap<(String, String), LoadedAsset>,
}
pub struct PreparedProject {
    root: PathBuf,
    path: PathBuf,
    scene: PreparedScene,
    models: PreparedModels,
}
impl PreparedProject {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, String> {
        let project = orr_package::Project::open(root.as_ref(), compiled_runtime())
            .map_err(|e| e.to_string())?;
        let before = project
            .verify()
            .map_err(|e| format!("room active lock: {e}"))?;
        let manifest = project.manifest().ok_or("room project manifest missing")?;
        let entry = manifest.entry.as_ref().ok_or("room entry missing")?;
        if manifest.schema != 2
            || entry.game != orr_package::ProjectGame::RoomEscapeV1
            || entry.sprites.is_some()
            || entry.ui.is_some()
            || manifest.progress.is_some()
        {
            return Err("room requires schema2 room-escape-v1 without sprites/UI/progress".into());
        }
        let models_relative = entry.models.as_ref().ok_or("room model sidecar missing")?;
        // First profile places the existing sidecar at the root. Its local hints
        // cannot redirect project identity or escape the admitted root.
        if models_relative.contains('/') {
            return Err("room model sidecar must be at project root".into());
        }
        let path = crate::project::entry_file(project.root(), &entry.scene)?;
        let bytes = crate::project::read_regular(&path, MAX_SCENE_BYTES)?;
        let scene = PreparedScene::parse(
            std::str::from_utf8(&bytes).map_err(|_| "room scene is not UTF-8")?,
        )?;
        let model_path = crate::project::entry_file(project.root(), models_relative)?;
        let bytes = crate::project::read_regular(&model_path, 1024 * 1024)?;
        let document: Document =
            serde_json::from_slice(&bytes).map_err(|e| format!("room model bindings: {e}"))?;
        document.validate()?;
        if document.project != "." || document.scene != entry.scene {
            return Err("room model sidecar must name this exact project and entry scene".into());
        }
        if document.bindings.is_empty() || document.bindings.len() > MAX_MODEL_BINDINGS {
            return Err("room requires1..=68 real model bindings".into());
        }
        let mut assets = BTreeMap::new();
        for (guid, binding) in &document.bindings {
            let guid = Guid::parse(guid)?;
            if scene.index.entity(&guid).is_none() {
                return Err("room model binding GUID is absent from scene".into());
            }
            if binding.kind != ModelKind::Static || binding.animation.is_some() {
                return Err("room v1 supports explicit static model bindings only".into());
            }
            let key = (binding.package.clone(), binding.asset.clone());
            if !assets.contains_key(&key) {
                if assets.len() == MAX_MODEL_ASSETS {
                    return Err("room model asset limit exceeded".into());
                }
                let asset = orr_model_bindings::model_bindings::load_binding_from_project(
                    &project, binding,
                )?;
                assets.insert(key.clone(), asset);
                validate_decoded_model_budget(assets.values())?;
            }
            binding.validate(assets.get(&key).expect("admitted model"))?;
        }
        validate_decoded_model_budget(assets.values())?;
        let after = project
            .verify()
            .map_err(|e| format!("room final active lock: {e}"))?;
        if before != after {
            return Err("room active lock changed during admission; retry".into());
        }
        let models = PreparedModels {
            path: model_path,
            document,
            assets,
        };
        crate::room_view::admit_presentation(
            orr_bridge::FrameView::of(scene.frame()),
            scene.index(),
            &models,
        )?;
        Ok(Self {
            root: project.root().to_path_buf(),
            path,
            scene,
            models,
        })
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
    pub fn models(&self) -> &PreparedModels {
        &self.models
    }
    pub fn into_parts(self) -> (PathBuf, PathBuf, PreparedScene, PreparedModels) {
        (self.root, self.path, self.scene, self.models)
    }
}
/// Charge each verified package/asset once, including conservative metadata overhead.
/// Shared by initial project admission and transactional editor mutations.
pub fn validate_decoded_model_budget<'a>(
    assets: impl IntoIterator<Item = &'a LoadedAsset>,
) -> Result<(), String> {
    let mut identities = BTreeSet::new();
    let mut decoded = 0usize;
    for asset in assets {
        if !identities.insert((asset.package(), asset.asset())) {
            continue;
        }
        if identities.len() > MAX_MODEL_ASSETS {
            return Err("room model asset limit exceeded".into());
        }
        let model = asset.static_model().ok_or("room requires static model")?;
        let source = model.source();
        let mut size = 0usize;
        for primitive in &source.primitives {
            size = add_bytes(
                size,
                primitive.vertices.len(),
                std::mem::size_of::<orr_model::Vertex>(),
            )?;
            size = add_bytes(size, primitive.indices.len(), std::mem::size_of::<u32>())?;
            size = add_bytes(size, primitive.id.len(), 1)?;
        }
        for image in &source.images {
            size = add_bytes(size, image.rgba8.len(), 1)?;
        }
        size = add_bytes(
            size,
            source.materials.len(),
            std::mem::size_of::<orr_model::Material>(),
        )?;
        // Bounded metadata/containers also charged conservatively per asset.
        size = add_bytes(size, 1024 * 1024, 1)?;
        decoded = decoded
            .checked_add(size)
            .ok_or("room model aggregate overflow")?;
        if decoded > MAX_DECODED_MODEL_BYTES {
            return Err("room aggregate decoded model limit exceeded".into());
        }
    }
    Ok(())
}
fn add_bytes(total: usize, count: usize, stride: usize) -> Result<usize, String> {
    count
        .checked_mul(stride)
        .and_then(|n| total.checked_add(n))
        .ok_or_else(|| "room decoded model size overflow".into())
}
