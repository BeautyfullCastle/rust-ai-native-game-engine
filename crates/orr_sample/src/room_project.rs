//! Closed RoomEscapeV1 project admission. File and package reads finish before startup.
use orr_ecs::Frame;
use orr_fp::FrameRng;
use orr_games::room_escape_game::{self as game, RoomEscapeV1};
use orr_model_bindings::model_bindings::{Binding, Document, LoadedAsset, ModelKind};
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
pub const MAX_ANIMATION_WORK_PER_FRAME: usize = 1_000_000;

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
    #[cfg(feature = "room-animated-models")]
    runtime.capabilities.insert("animation".into());
    runtime
}
pub struct PreparedModels {
    pub path: PathBuf,
    pub document: Document,
    pub assets: BTreeMap<(String, String), LoadedAsset>,
}
#[cfg(feature = "room-lighting")]
pub struct PreparedRoomLighting {
    pub scene_path: PathBuf,
    pub path: PathBuf,
    pub document: crate::room_lighting::Document,
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
pub struct PreparedProject {
    #[cfg(feature = "room-lighting")]
    lighting: Option<PreparedRoomLighting>,
    camera: Option<PreparedRoomCamera>,
    root: PathBuf,
    path: PathBuf,
    scene: PreparedScene,
    models: PreparedModels,
}
impl PreparedProject {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, String> {
        Self::open_with_lighting(root, false)
    }
    /// Lighting requires explicit support from the consuming route, not feature unification.
    pub fn open_with_lighting(root: impl AsRef<Path>, lighting_supported: bool) -> Result<Self, String> {
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
        let manifest_path = project.root().join("orr.project.json");
        let manifest_bytes = crate::project::read_regular(&manifest_path, 1024 * 1024)?;
        let pinned: orr_package::ProjectManifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|e| format!("room manifest: {e}"))?;
        if pinned != *manifest { return Err("room manifest changed during admission".into()); }
        if entry.lighting.is_some() && !(lighting_supported && cfg!(feature = "room-lighting")) {
            return Err("room lighting requires explicit lighting consumer support".into());
        }
        #[cfg(feature = "room-lighting")]
        let lighting = entry.lighting.as_ref().map(|relative| {
            let path = crate::project::entry_file(project.root(), relative)?;
            let bytes = crate::project::read_regular(&path, crate::room_lighting::MAX_BYTES as u64)?;
            let document = crate::room_lighting::Document::parse(&bytes)?;
            Ok::<_, String>(PreparedRoomLighting {
                scene_path: crate::project::entry_file(project.root(), &entry.scene)?,
                path, document, bytes, manifest_bytes: manifest_bytes.clone(), manifest_path: manifest_path.clone(),
            })
        }).transpose()?;
        let camera = entry.camera.as_ref().map(|relative| {
            let path = crate::project::entry_file(project.root(), relative)?;
            let bytes = crate::project::read_regular(&path, crate::room_camera::MAX_BYTES as u64)?;
            let document = crate::room_camera::Document::parse(&bytes)?;
            Ok::<_, String>(PreparedRoomCamera { path, document, bytes,
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
            if binding.kind == ModelKind::Animated {
                #[cfg(not(feature = "room-animated-models"))]
                return Err("Room v1 supports static model bindings only in this build".into());
            }
            if (binding.kind == ModelKind::Static && binding.animation.is_some())
                || (binding.kind == ModelKind::Animated && binding.animation.is_none())
            {
                return Err("Room model kind and animation descriptor do not match".into());
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
        let animation_bindings = document
            .bindings
            .values()
            .map(|binding| {
                let key = (binding.package.clone(), binding.asset.clone());
                assets
                    .get(&key)
                    .map(|asset| (binding, asset))
                    .ok_or_else(|| "Room model binding asset missing after admission".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        validate_animation_workload(animation_bindings)?;
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
        if crate::project::read_regular(&manifest_path, 1024 * 1024)? != manifest_bytes {
            return Err("room manifest changed during admission".into());
        }
        Ok(Self {
            #[cfg(feature = "room-lighting")]
            lighting,
            camera,
            root: project.root().to_path_buf(),
            path,
            scene,
            models,
        })
    }
    #[cfg(feature = "room-lighting")]
    pub fn lighting(&self) -> Option<&PreparedRoomLighting> { self.lighting.as_ref() }
    #[cfg(feature = "room-lighting")]
    pub fn take_lighting(&mut self) -> Option<PreparedRoomLighting> { self.lighting.take() }
    pub fn point_light_settings(&self) -> orr_render::PointLightSettings {
        #[cfg(feature = "room-lighting")]
        if let Some(lighting) = &self.lighting { return lighting.document.settings(); }
        orr_render::PointLightSettings::default()
    }
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
        let mut size = 0usize;
        match asset.kind() {
            ModelKind::Static => {
                let model = asset.static_model().ok_or("invalid static Room model")?;
                let source = model.source();
                for primitive in &source.primitives {
                    size = add_bytes(
                        size,
                        primitive.vertices.len(),
                        std::mem::size_of::<orr_model::Vertex>(),
                    )?;
                    size = add_bytes(size, primitive.indices.len(), std::mem::size_of::<u32>())?;
                    size = add_bytes(size, primitive.id.len(), 1)?;
                }
                for dependency in &source.dependencies {
                    size = add_bytes(size, 1, std::mem::size_of::<orr_model::Dependency>())?;
                    size = add_bytes(size, dependency.uri.len(), 1)?;
                    size = add_bytes(size, dependency.sha256.len(), 1)?;
                }
                size = add_bytes(
                    size,
                    source.materials.len(),
                    std::mem::size_of::<orr_model::Material>(),
                )?;
                for image in &source.images {
                    size = add_bytes(size, image.rgba8.len(), 1)?;
                }
            }
            ModelKind::Animated => {
                #[cfg(feature = "room-animated-models")]
                {
                    let model = asset
                        .animated_model()
                        .ok_or("invalid animated Room model")?;
                    let source = model.source();
                    for primitive in &source.primitives {
                        size = add_bytes(
                            size,
                            primitive.vertices.len(),
                            std::mem::size_of::<orr_model::animation::SkinnedVertex>(),
                        )?;
                        size =
                            add_bytes(size, primitive.indices.len(), std::mem::size_of::<u32>())?;
                        size = add_bytes(size, primitive.id.len(), 1)?;
                    }
                    for dependency in &source.dependencies {
                        size = add_bytes(size, 1, std::mem::size_of::<orr_model::Dependency>())?;
                        size = add_bytes(size, dependency.uri.len(), 1)?;
                        size = add_bytes(size, dependency.sha256.len(), 1)?;
                    }
                    for node in &source.nodes {
                        size = add_bytes(
                            size,
                            1,
                            std::mem::size_of::<orr_model::animation::AnimatedNode>(),
                        )?;
                        size = add_bytes(size, node.name.len(), 1)?;
                        size = add_bytes(size, node.children.len(), std::mem::size_of::<u32>())?;
                    }
                    for skin in &source.skins {
                        size =
                            add_bytes(size, 1, std::mem::size_of::<orr_model::animation::Skin>())?;
                        size = add_bytes(size, skin.name.len(), 1)?;
                        size = add_bytes(size, skin.joints.len(), std::mem::size_of::<u32>())?;
                        size = add_bytes(
                            size,
                            skin.inverse_bind_matrices.len(),
                            std::mem::size_of::<orr_model::animation::Matrix4>(),
                        )?;
                    }
                    for clip in &source.clips {
                        size = add_bytes(
                            size,
                            1,
                            std::mem::size_of::<orr_model::animation::AnimationClip>(),
                        )?;
                        size = add_bytes(size, clip.name.len(), 1)?;
                        for channel in &clip.channels {
                            size = add_bytes(
                                size,
                                1,
                                std::mem::size_of::<orr_model::animation::AnimationChannel>(),
                            )?;
                            size =
                                add_bytes(size, channel.times.len(), std::mem::size_of::<f32>())?;
                            let (value_count, value_stride) = match &channel.values {
                                orr_model::animation::ChannelValues::Translation(values)
                                | orr_model::animation::ChannelValues::Scale(values) => {
                                    (values.len(), std::mem::size_of::<[f32; 3]>())
                                }
                                orr_model::animation::ChannelValues::Rotation(values) => {
                                    (values.len(), std::mem::size_of::<[f32; 4]>())
                                }
                            };
                            size = add_bytes(size, value_count, value_stride)?;
                        }
                    }
                    size = add_bytes(
                        size,
                        source.materials.len(),
                        std::mem::size_of::<orr_model::Material>(),
                    )?;
                    for image in &source.images {
                        size = add_bytes(size, image.rgba8.len(), 1)?;
                    }
                }
                #[cfg(not(feature = "room-animated-models"))]
                return Err("Room v1 supports static model bindings only in this build".into());
            }
        }
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

/// Bound repeated per-instance skeletal sampling work for one presented frame.
pub fn validate_animation_workload<'a>(
    bindings: impl IntoIterator<Item = (&'a Binding, &'a LoadedAsset)>,
) -> Result<(), String> {
    #[cfg(feature = "room-animated-models")]
    let mut work = 0usize;
    for (binding, asset) in bindings {
        if binding.kind != ModelKind::Animated {
            continue;
        }
        #[cfg(feature = "room-animated-models")]
        {
            binding.validate(asset)?;
            let model = asset
                .animated_model()
                .ok_or("Room animation binding does not reference an animated asset")?;
            let source = model.source();
            let animation = binding
                .animation
                .ok_or("Room animation binding has no selected clip")?;
            let clip = source
                .clips
                .get(animation.clip_index as usize)
                .ok_or("Room animation clip index is out of range")?;
            let mut instance_work = source.nodes.len();
            for primitive in &source.primitives {
                instance_work = instance_work
                    .checked_add(primitive.vertices.len())
                    .ok_or("Room animation workload overflow")?;
            }
            for skin in &source.skins {
                instance_work = instance_work
                    .checked_add(skin.joints.len())
                    .ok_or("Room animation workload overflow")?;
            }
            for channel in &clip.channels {
                let search_work = if channel.times.len() < 2 {
                    1
                } else {
                    (usize::BITS - (channel.times.len() - 1).leading_zeros()) as usize
                };
                instance_work = instance_work
                    .checked_add(1)
                    .and_then(|value| value.checked_add(search_work))
                    .ok_or("Room animation workload overflow")?;
            }
            work = work
                .checked_add(instance_work)
                .ok_or("Room animation workload overflow")?;
            if work > MAX_ANIMATION_WORK_PER_FRAME {
                return Err("Room aggregate animation workload limit exceeded".into());
            }
        }
        #[cfg(not(feature = "room-animated-models"))]
        {
            let _ = asset;
            return Err("Room supports static model bindings only in this build".into());
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
