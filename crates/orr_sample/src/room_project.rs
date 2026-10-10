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
    compiled_runtime_with_character(false)
}
pub fn compiled_runtime_with_character(character_supported: bool) -> orr_package::Runtime {
    let mut runtime = orr_package::Runtime::content_only();
    if character_supported && cfg!(feature = "room-character") { runtime.capabilities.insert("animation".into()); }
    runtime.capabilities.insert("models".into());
    runtime
}
pub struct PreparedModels {
    #[cfg(feature = "room-character")]
    pub character: Option<crate::room_character::Document>,
    pub path: PathBuf,
    pub document: Document,
    pub assets: BTreeMap<(String, String), LoadedAsset>,
}
#[cfg(feature = "room-character")]
pub struct PreparedRoomCharacter {
    pub path: PathBuf,
    pub document: crate::room_character::Document,
    pub bytes: Vec<u8>,
    pub manifest_bytes: Vec<u8>,
    pub manifest_path: PathBuf,
}
pub struct PreparedRoomCamera {
    pub path: PathBuf,
    pub document: crate::room_camera::Document,
    pub bytes: Vec<u8>,
    pub manifest_bytes: Vec<u8>,
    pub manifest_path: PathBuf,
}
#[cfg(feature = "room-ui")]
#[derive(Clone)]
pub struct PreparedRoomUi {
    pub scene_path: PathBuf,
    pub manifest_path: PathBuf,
    pub manifest_bytes: Vec<u8>,
    pub path: PathBuf,
    pub document: crate::authored_ui::Document,
    pub font: Vec<u8>,
}

/// Explicit consuming-route support, never inferred from unified dependencies.
/// MetadataOnly admits a checkpoint declaration but performs no profile storage
/// or environment access. Consumers opt in only when built for checkpoint use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CheckpointSupport {
    #[default]
    Disabled,
    MetadataOnly,
}

/// Shared font requirement for checkpoint admission and explicit editor enable.
#[cfg(feature = "room-ui")]
pub fn validate_checkpoint_font(font: &[u8]) -> Result<(), String> {
    let corpus: String = (b' '..=b'~').map(char::from).collect();
    crate::game_ui::GameUi::validate_font_with_corpus(font, &corpus)
}

pub struct PreparedProject {
    manifest_bytes: Vec<u8>,
    checkpoint: Option<orr_package::ProjectProgress>,
    #[cfg(feature = "room-ui")]
    ui: Option<PreparedRoomUi>,
    camera: Option<PreparedRoomCamera>,
    #[cfg(feature = "room-character")]
    character: Option<PreparedRoomCharacter>,
    root: PathBuf,
    path: PathBuf,
    scene: PreparedScene,
    models: PreparedModels,
}
impl PreparedProject {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, String> {
        Self::open_with_ui(root, false)
    }

    pub fn open_with_ui(root: impl AsRef<Path>, ui_supported: bool) -> Result<Self, String> {
        Self::open_with_options(root, ui_supported, CheckpointSupport::Disabled)
    }

    pub fn open_with_options(
        root: impl AsRef<Path>,
        ui_supported: bool,
        checkpoint_support: CheckpointSupport,
    ) -> Result<Self, String> {
        Self::open_with_capabilities(root, ui_supported, checkpoint_support, false)
    }
    /// Consumer support is explicit; unified dependencies cannot enable character admission.
    pub fn open_with_capabilities(
        root: impl AsRef<Path>, ui_supported: bool,
        checkpoint_support: CheckpointSupport, character_supported: bool,
    ) -> Result<Self, String> {
        let project = orr_package::Project::open(root.as_ref(), compiled_runtime_with_character(character_supported))
            .map_err(|e| e.to_string())?;
        let manifest = project.manifest().ok_or("room project manifest missing")?;
        let manifest_path = project.root().join("orr.project.json");
        let manifest_bytes = crate::project::read_regular(&manifest_path, 1024 * 1024)?;
        let pinned_manifest: orr_package::ProjectManifest =
            serde_json::from_slice(&manifest_bytes).map_err(|e| e.to_string())?;
        if &pinned_manifest != manifest {
            return Err("room manifest changed during admission; retry".into());
        }
        let entry = manifest.entry.as_ref().ok_or("room entry missing")?;
        if entry.character.is_some() && !(character_supported && cfg!(feature = "room-character")) {
            return Err("room character requires explicit character consumer support".into());
        }
        let checkpoint = match (manifest.schema, &manifest.progress, checkpoint_support) {
            (2, None, _) => None,
            (4, Some(progress), CheckpointSupport::MetadataOnly)
                if progress.profile == orr_package::ProgressProfile::RoomKeyCheckpointV1 =>
            {
                progress.validate()?;
                Some(progress.clone())
            }
            _ => {
                return Err(
                    "room checkpoint requires schema4 and explicit checkpoint consumer support"
                        .into(),
                )
            }
        };
        if !matches!(manifest.schema, 2 | 4)
            || entry.game != orr_package::ProjectGame::RoomEscapeV1
            || entry.sprites.is_some()
            || (entry.ui.is_some() && (!ui_supported || !cfg!(feature = "room-ui")))
        {
            return Err("room requires schema2/4 room-escape-v1, no sprites, and explicit UI consumer support".into());
        }
        let before = project
            .verify()
            .map_err(|e| format!("room active lock: {e}"))?;
        #[cfg(feature = "room-ui")]
        let ui = entry
            .ui
            .as_ref()
            .map(|descriptor| {
                if descriptor.profile != orr_package::ProjectUiProfile::RoomAuthoredV1 {
                    return Err("Room UI profile mismatch".into());
                }
                let path = crate::project::entry_file(
                    project.root(),
                    descriptor
                        .document
                        .as_ref()
                        .ok_or("Room UI document missing")?,
                )?;
                let document = crate::authored_ui::Document::parse_for(
                    &crate::project::read_regular(&path, crate::authored_ui::MAX_BYTES as u64)?,
                    crate::authored_ui::Profile::Room,
                )?;
                let package = before
                    .packages
                    .get(&descriptor.font.package)
                    .ok_or("Room UI font package absent from active lock")?;
                if !package.manifest.files.contains(&descriptor.font.asset) {
                    return Err("Room UI font absent from package manifest".into());
                }
                let font = project
                    .read_asset(&descriptor.font.package, &descriptor.font.asset)
                    .map_err(|e| e.to_string())?;
                crate::collect_ui::CollectUi::validate_font_for(
                    &font,
                    &document,
                    crate::authored_ui::Profile::Room,
                )?;
                if checkpoint.is_some() {
                    validate_checkpoint_font(&font)?;
                }
                let manifest_path = project.root().join("orr.project.json");
                let manifest_bytes = crate::project::read_regular(&manifest_path, 1024 * 1024)?;
                let pinned: orr_package::ProjectManifest =
                    serde_json::from_slice(&manifest_bytes).map_err(|e| e.to_string())?;
                if pinned != *manifest {
                    return Err("Room manifest changed during UI admission".into());
                }
                Ok::<_, String>(PreparedRoomUi {
                    scene_path: crate::project::entry_file(project.root(), &entry.scene)?,
                    manifest_path,
                    manifest_bytes,
                    path,
                    document,
                    font,
                })
            })
            .transpose()?;
        let camera = if let Some(relative) = &entry.camera {
            let path = crate::project::entry_file(project.root(), relative)?;
            let bytes = crate::project::read_regular(&path, crate::room_camera::MAX_BYTES as u64)?;
            let document = crate::room_camera::Document::parse(&bytes)?;
            let manifest_path = project.root().join("orr.project.json");
            let manifest_bytes = crate::project::read_regular(&manifest_path, 1024 * 1024)?;
            let pinned: orr_package::ProjectManifest = serde_json::from_slice(&manifest_bytes)
                .map_err(|e| format!("room camera manifest: {e}"))?;
            if pinned != *manifest {
                return Err("room manifest changed during camera admission".into());
            }
            Some(PreparedRoomCamera {
                path,
                document,
                bytes,
                manifest_bytes,
                manifest_path,
            })
        } else {
            None
        };
        #[cfg(feature = "room-character")]
        let character = entry.character.as_ref().map(|relative| {
            let path = crate::project::entry_file(project.root(), relative)?;
            let bytes = crate::project::read_regular(&path, crate::room_character::MAX_BYTES as u64)?;
            let document = crate::room_character::Document::parse(&bytes)?;
            Ok::<_, String>(PreparedRoomCharacter { path, document, bytes,
                manifest_bytes: manifest_bytes.clone(), manifest_path: manifest_path.clone() })
        }).transpose()?;
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
        if let Some(camera) = &camera {
            camera
                .document
                .validate_frame(orr_bridge::FrameView::of(scene.frame()), scene.index())?;
        }
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
            if (binding.kind != ModelKind::Static || binding.animation.is_some())
                && !(character_supported && entry.character.is_some() && cfg!(feature = "room-character")) {
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
        #[cfg(feature = "room-character")]
        if let Some(character) = &character {
            character.document.validate_bindings(orr_bridge::FrameView::of(scene.frame()), scene.index(), &document, &assets)?;
        }
        let models = PreparedModels {
            #[cfg(feature = "room-character")]
            character: character.as_ref().map(|c| c.document.clone()),
            path: model_path,
            document,
            assets,
        };
        crate::room_view::admit_presentation(
            orr_bridge::FrameView::of(scene.frame()),
            scene.index(),
            &models,
        )?;
        if crate::project::read_regular(&manifest_path, 1024 * 1024)? != manifest_bytes {
            return Err("room manifest changed during admission; retry".into());
        }
        Ok(Self {
            manifest_bytes,
            checkpoint,
            #[cfg(feature = "room-ui")]
            ui,
            camera,
            #[cfg(feature = "room-character")]
            character,
            root: project.root().to_path_buf(),
            path,
            scene,
            models,
        })
    }
    #[cfg(feature = "room-ui")]
    pub fn ui(&self) -> Option<&PreparedRoomUi> {
        self.ui.as_ref()
    }
    #[cfg(feature = "room-ui")]
    pub fn take_ui(&mut self) -> Option<PreparedRoomUi> {
        self.ui.take()
    }
    pub fn manifest_bytes(&self) -> &[u8] {
        &self.manifest_bytes
    }
    pub fn checkpoint(&self) -> Option<&orr_package::ProjectProgress> {
        self.checkpoint.as_ref()
    }
    #[cfg(feature = "room-character")]
    pub fn character(&self) -> Option<&PreparedRoomCharacter> { self.character.as_ref() }
    #[cfg(feature = "room-character")]
    pub fn take_character(&mut self) -> Option<PreparedRoomCharacter> { self.character.take() }
    pub fn camera(&self) -> Option<&PreparedRoomCamera> {
        self.camera.as_ref()
    }
    pub fn take_camera(&mut self) -> Option<PreparedRoomCamera> {
        self.camera.take()
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
        #[cfg(feature = "room-character")]
        if let Some(model) = asset.animated_model() {
            decoded = decoded.checked_add(crate::room_character::decoded_model_bytes(model)?)
                .ok_or("room model aggregate overflow")?;
            if decoded > MAX_DECODED_MODEL_BYTES { return Err("room aggregate decoded model limit exceeded".into()); }
            continue;
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

#[cfg(test)]
mod checkpoint_tests {
    use super::*;
    use std::fs;

    fn manifest() -> serde_json::Value {
        serde_json::json!({
            "schema":4,"engine":"*",
            "entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json",
                "ui":{"profile":"room-authored-v1","document":"room.ui.json","font":{"package":"font","asset":"font.otf"}}},
            "progress":{"schema":1,"game_id":"12345678-1234-4234-8234-123456789abc","profile":"room-key-checkpoint-v1"}
        })
    }

    #[test]
    fn checkpoint_default_off_rejects_before_content_or_storage_access() {
        assert_eq!(CheckpointSupport::default(), CheckpointSupport::Disabled);
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let bytes = serde_json::to_vec(&manifest()).unwrap();
        fs::write(root.join("orr.project.json"), &bytes).unwrap();
        // No lock, scene, UI, model, or profile directory exists: the explicit
        // consuming-route decision must precede every content read.
        for result in [
            PreparedProject::open(&root),
            PreparedProject::open_with_ui(&root, true),
            PreparedProject::open_with_options(&root, true, CheckpointSupport::Disabled),
        ] {
            let error = result.err().expect("default-off checkpoint admission");
            assert!(
                error.contains("explicit checkpoint consumer support"),
                "{error}"
            );
        }
        let error =
            PreparedProject::open_with_options(&root, false, CheckpointSupport::MetadataOnly)
                .err()
                .expect("checkpoint cannot grant UI support");
        assert!(error.contains("explicit UI consumer support"), "{error}");
        assert_eq!(fs::read(root.join("orr.project.json")).unwrap(), bytes);
        assert_eq!(fs::read_dir(root).unwrap().count(), 1);
    }

    #[test]
    fn invalid_checkpoint_metadata_fails_before_content_admission() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        for value in [
            serde_json::Value::Null,
            serde_json::json!({"schema":1,"game_id":"12345678-1234-4234-8234-123456789abc","profile":"collect-dodge-highscore-v1"}),
            serde_json::json!({"schema":1,"game_id":"../profile","profile":"room-key-checkpoint-v1"}),
            serde_json::json!({"schema":2,"game_id":"12345678-1234-4234-8234-123456789abc","profile":"room-key-checkpoint-v1"}),
        ] {
            let mut candidate = manifest();
            candidate["progress"] = value;
            fs::write(
                root.join("orr.project.json"),
                serde_json::to_vec(&candidate).unwrap(),
            )
            .unwrap();
            assert!(orr_package::Project::open(&root, compiled_runtime()).is_err());
            assert!(PreparedProject::open_with_options(
                &root,
                true,
                CheckpointSupport::MetadataOnly
            )
            .is_err());
        }
        let valid = manifest();
        let duplicate = format!(
            "{{\"progress\":{},{}",
            valid["progress"],
            &valid.to_string()[1..]
        );
        fs::write(root.join("orr.project.json"), duplicate).unwrap();
        assert!(
            PreparedProject::open_with_options(&root, true, CheckpointSupport::MetadataOnly)
                .is_err()
        );
        assert_eq!(fs::read_dir(root).unwrap().count(), 1);
    }

    #[cfg(all(
        feature = "room-ui",
        feature = "project-create",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    #[test]
    fn metadata_only_admission_preserves_legacy_scene_and_owns_checkpoint() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap().join("room checkpoint");
        crate::project_create::create(&crate::project_create::CreateOptions {
            output: root.clone(),
            template: crate::project_create::ROOM_UI_TEMPLATE.into(),
            seed: "checkpoint-admission".into(),
        })
        .unwrap();
        let legacy = PreparedProject::open_with_ui(&root, true).unwrap();
        assert!(legacy.checkpoint().is_none());
        let opted_in_legacy =
            PreparedProject::open_with_options(&root, true, CheckpointSupport::MetadataOnly)
                .unwrap();
        assert!(opted_in_legacy.checkpoint().is_none());
        let path = root.join("orr.project.json");
        let mut upgraded: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        upgraded["schema"] = 4.into();
        upgraded["progress"] = manifest()["progress"].clone();
        let bytes = serde_json::to_vec(&upgraded).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(PreparedProject::open_with_ui(&root, true).is_err());
        let checkpoint =
            PreparedProject::open_with_options(&root, true, CheckpointSupport::MetadataOnly)
                .unwrap();
        assert_eq!(
            checkpoint.scene().frame().to_bytes(),
            legacy.scene().frame().to_bytes()
        );
        assert_eq!(
            checkpoint.camera().unwrap().bytes,
            legacy.camera().unwrap().bytes
        );
        assert_eq!(checkpoint.ui().unwrap().font, legacy.ui().unwrap().font);
        let identity = checkpoint.checkpoint().unwrap().clone();
        assert_eq!(
            identity.profile,
            orr_package::ProgressProfile::RoomKeyCheckpointV1
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(checkpoint.checkpoint(), Some(&identity));
        assert_eq!(
            checkpoint.scene().simulation().unwrap().frame().checksum(),
            legacy.scene().frame().checksum()
        );
        #[cfg(not(all(feature = "room-checkpoint", target_os = "linux")))]
        assert!(crate::room_app::run_window(checkpoint)
            .unwrap_err()
            .contains("requires the Linux room-checkpoint consumer"));
    }
}
