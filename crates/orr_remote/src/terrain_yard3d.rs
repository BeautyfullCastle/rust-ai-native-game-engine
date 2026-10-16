//! Explicit sphere-only, scene-pinned terrain host. Filesystem access is confined
//! to bake admission; simulation and replay consume only immutable Frame data.
use crate::{GameHooks, HostLimits, LocalHost, ServerConfig, ViewStreamHook};
use orr_ecs::Frame;
use orr_edit::{BakeAdmission, EditError, EditorDoc};
use orr_reflect::{Scene, SceneIndex, TypeRegistry};
use orr_sim::Simulation;
use orr_terrain::Terrain;
use orr_terrain_physics3d::asset::admit_heightfield;
use orr_viewstream::{FrameMeta, Schema, StreamProducer};
use std::{
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

pub use crate::yard3d::{YARD_BUILD_ID, YARD_PLAYERS, YARD_SEED, YARD_TICK_RATE};
pub use orr_games::terrain_yard3d_game::{
    TerrainScenePin, TerrainStepStatus, TerrainYard3D, PIN_NAME,
};

pub fn terrain_yard3d_types() -> TypeRegistry {
    let mut types = TypeRegistry::new();
    orr_games::terrain_yard3d_game::register_reflect(&mut types);
    types
}

/// Immutable authority root, fixed when the scene host is created. A subsequent
/// ERP load cannot expand it by supplying a different save path.
#[derive(Clone, Debug)]
pub struct LocalTerrainAdmission {
    root: PathBuf,
}
impl LocalTerrainAdmission {
    pub fn for_scene(path: &Path) -> Result<Self, String> {
        let path = absolute_regular_file(path)?;
        Ok(Self {
            root: path
                .parent()
                .ok_or("terrain scene needs a parent directory")?
                .to_path_buf(),
        })
    }

    fn read_pin(&self, pin: &TerrainScenePin) -> Result<Vec<u8>, String> {
        let relative = pin.source()?;
        validate_relative_source(relative)?;
        let path = self.root.join(relative);
        let path = absolute_regular_file(&path)?;
        if !path.starts_with(&self.root) {
            return Err("terrain source escaped the scene directory".into());
        }
        let file = File::open(&path)
            .map_err(|e| format!("cannot open terrain {}: {e}", path.display()))?;
        if file.metadata().map_err(|e| e.to_string())?.len() > orr_terrain::MAX_FILE_BYTES as u64 {
            return Err("terrain source exceeds byte limit".into());
        }
        let mut bytes = Vec::new();
        file.take(orr_terrain::MAX_FILE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > orr_terrain::MAX_FILE_BYTES {
            return Err("terrain source exceeds byte limit".into());
        }
        let terrain = Terrain::load(&bytes).map_err(|e| e.to_string())?;
        if terrain.asset_id() != pin.identity()? {
            return Err("terrain asset identity does not match the scene pin".into());
        }
        if terrain.revision() != pin.revision {
            return Err("terrain full SHA-256 revision does not match the scene pin".into());
        }
        Ok(bytes)
    }
}
impl BakeAdmission for LocalTerrainAdmission {
    fn admit(&self, scene: &Scene, frame: &mut Frame, _: &SceneIndex) -> Result<(), EditError> {
        let fail = |message: String| EditError::Invalid(format!("terrain admission: {message}"));
        if !scene.singletons.iter().any(|(name, _)| name == PIN_NAME) {
            return Err(fail("an explicit TerrainScenePin is required".into()));
        }
        let pin = *frame.singleton::<TerrainScenePin>();
        let bytes = self.read_pin(&pin).map_err(fail)?;
        if bytemuck::bytes_of(frame.singleton::<TerrainStepStatus>())
            .iter()
            .any(|&byte| byte != 0)
        {
            return Err(fail(
                "runtime failure status cannot be authored into a scene".into(),
            ));
        }
        admit_heightfield(frame, &bytes, pin.revision, pin.collider())
            .map_err(|e| fail(e.to_string()))?;
        orr_games::terrain_yard3d_game::validate_scene_frame(frame).map_err(fail)
    }
    fn allow_play_edits(&self) -> bool {
        false
    }
}

fn validate_relative_source(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.contains('\\')
        || path.contains(':')
        || path.contains('\0')
        || path.starts_with('/')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == ".." || part == ".orr")
        || Path::new(path).extension().and_then(|x| x.to_str()) != Some("orrt")
    {
        return Err("terrain source must be a confined scene-relative .orrt path".into());
    }
    if !Path::new(path)
        .components()
        .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err("terrain source has an unsupported path component".into());
    }
    Ok(())
}

fn absolute_regular_file(path: &Path) -> Result<PathBuf, String> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    let mut checked = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => continue,
            Component::ParentDir => {
                return Err("terrain paths cannot contain parent traversal".into())
            }
            other => checked.push(other.as_os_str()),
        }
        let meta = fs::symlink_metadata(&checked)
            .map_err(|e| format!("cannot inspect {}: {e}", checked.display()))?;
        if meta.file_type().is_symlink() {
            return Err(format!(
                "terrain paths cannot contain symlinks: {}",
                checked.display()
            ));
        }
    }
    if !fs::metadata(&checked).map_err(|e| e.to_string())?.is_file() {
        return Err("terrain source and scene must be regular files".into());
    }
    Ok(checked)
}

pub fn terrain_yard3d_doc_from_yaml(
    yaml: &str,
    admission: LocalTerrainAdmission,
) -> Result<EditorDoc, EditError> {
    EditorDoc::from_yaml_with_admission(
        yaml,
        terrain_yard3d_types(),
        Simulation::<TerrainYard3D>::build_registry(),
        YARD_SEED,
        Some(Arc::new(admission)),
    )
}

pub fn terrain_yard3d_doc_from_path(path: &Path) -> Result<EditorDoc, String> {
    let admission = LocalTerrainAdmission::for_scene(path)?;
    let yaml =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    terrain_yard3d_doc_from_yaml(&yaml, admission).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn configure_terrain_yard3d(limits: &mut HostLimits) {
    limits.allow_scene_paths = false;
    limits.reject_scene_path_overrides = true;
    limits.player_count = YARD_PLAYERS;
    limits.tick_rate = YARD_TICK_RATE;
    limits.build_id = YARD_BUILD_ID;
    limits.game = GameHooks::new("TerrainYard3D");
    limits.view_stream = Some(ViewStreamHook::new(TerrainStream::new(
        limits.build_id,
        limits.player_count,
    )));
}

pub fn spawn_terrain_yard3d_scene_host(
    path: PathBuf,
    mut cfg: ServerConfig,
) -> Result<LocalHost, String> {
    LocalHost::spawn::<TerrainYard3D>(move || {
        let doc = terrain_yard3d_doc_from_path(&path)?;
        cfg.limits.scene_path = Some(path);
        configure_terrain_yard3d(&mut cfg.limits);
        Ok((doc, cfg))
    })
}

struct TerrainStream {
    yard: orr_sample::yard3d_stream::Yard3dStreamProducer,
    schema: Schema,
}
impl TerrainStream {
    fn new(build_id: u64, players: u8) -> Self {
        let yard = orr_sample::yard3d_stream::Yard3dStreamProducer::new(build_id, players);
        let mut schema = yard.schema().clone();
        schema.game = "TerrainYard3D".into();
        Self { yard, schema }
    }
}
impl StreamProducer for TerrainStream {
    fn schema(&self) -> &Schema {
        &self.schema
    }
    fn encode_frame(&mut self, cur: &Frame, prev: Option<&Frame>, meta: FrameMeta) -> Vec<u8> {
        self.yard.encode_frame(cur, prev, meta)
    }
}

/// Presentation reconstructs the admitted asset from the same immutable frame
/// as the bodies, never from an editor working copy or mutable source file.
pub fn terrain_from_view(frame: orr_bridge::FrameView<'_>) -> Result<Terrain, String> {
    use orr_terrain_physics3d::asset::{HeightfieldCollider, TerrainAsset};
    let asset = frame.singleton::<TerrainAsset>();
    let pin = frame.singleton::<TerrainScenePin>();
    if asset.present != 1
        || !frame.exists(asset.entity)
        || asset.revision != pin.revision
        || frame
            .get::<HeightfieldCollider>(asset.entity)
            .map(|c| c.revision)
            != Some(asset.revision)
    {
        return Err("snapshot has no valid admitted terrain".into());
    }
    let identity: Vec<u8> = frame
        .list(asset.identity)
        .iter()
        .map(|value| value.0)
        .collect();
    let identity = String::from_utf8(identity).map_err(|_| "invalid snapshot terrain identity")?;
    if identity != pin.identity()? {
        return Err("snapshot terrain identity mismatch".into());
    }
    let holes = frame.list(asset.holes);
    if holes.iter().any(|value| value.0 > 1) {
        return Err("snapshot terrain has a noncanonical hole".into());
    }
    let terrain = Terrain::new(
        identity,
        asset.width,
        asset.depth,
        asset.origin,
        asset.spacing,
        frame
            .list(asset.heights)
            .iter()
            .map(|value| value.0)
            .collect(),
        holes.iter().map(|value| value.0 != 0).collect(),
    )
    .map_err(|e| e.to_string())?;
    if terrain.revision() != pin.revision {
        return Err("snapshot terrain full revision mismatch".into());
    }
    Ok(terrain)
}
