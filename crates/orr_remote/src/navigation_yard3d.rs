//! Local scene-relative admission for deterministic point navigation.
//! Filesystem reads occur only while baking; play, seek, replay and views use Frame bytes.
use crate::{GameHooks, HostLimits, LocalHost, ServerConfig, ViewStreamHook};
use orr_ecs::Frame;
use orr_edit::{BakeAdmission, EditError, EditorDoc};
use orr_navigation::{AgentProfile, Navigator, TerrainGraph};
use orr_navigation_runtime::{NavigationRuntime, RuntimeAgent, RuntimeState};
use orr_reflect::{Scene, SceneIndex, TypeRegistry};
use orr_sim::Simulation;
use orr_terrain::Terrain;
use orr_viewstream::{FrameMeta, Schema, StreamProducer};
use std::{
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

pub use crate::yard3d::{YARD_BUILD_ID, YARD_PLAYERS, YARD_SEED, YARD_TICK_RATE};
pub use orr_games::navigation_yard3d_game::{
    NavigationAgentSpec, NavigationScenePin, NavigationStepStatus, NavigationYard3D, PIN_NAME,
};

pub fn navigation_yard3d_types() -> TypeRegistry {
    let mut types = TypeRegistry::new();
    orr_games::navigation_yard3d_game::register_reflect(&mut types);
    types
}

/// The immutable authority root is fixed by the scene's original path.
/// A later ERP load cannot expand it by proposing a different save destination.
#[derive(Clone, Debug)]
pub struct LocalNavigationAdmission {
    root: PathBuf,
}
impl LocalNavigationAdmission {
    pub fn for_scene(path: &Path) -> Result<Self, String> {
        let path = absolute_regular_file(path)?;
        Ok(Self {
            root: path
                .parent()
                .ok_or("navigation scene needs a parent directory")?
                .to_path_buf(),
        })
    }

    fn read_pin(&self, pin: &NavigationScenePin) -> Result<Terrain, String> {
        let relative = pin.source()?;
        validate_relative_source(relative)?;
        let path = absolute_regular_file(&self.root.join(relative))?;
        if !path.starts_with(&self.root) {
            return Err("navigation source escaped the scene directory".into());
        }
        let file = File::open(&path)
            .map_err(|e| format!("cannot open navigation terrain {}: {e}", path.display()))?;
        if file.metadata().map_err(|e| e.to_string())?.len() > orr_terrain::MAX_FILE_BYTES as u64 {
            return Err("navigation terrain source exceeds byte limit".into());
        }
        let mut bytes = Vec::new();
        file.take(orr_terrain::MAX_FILE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > orr_terrain::MAX_FILE_BYTES {
            return Err("navigation terrain source exceeds byte limit".into());
        }
        let terrain = Terrain::load(&bytes).map_err(|e| e.to_string())?;
        if terrain.asset_id() != pin.identity()? {
            return Err("navigation asset identity does not match the scene pin".into());
        }
        if terrain.revision() != pin.terrain_revision {
            return Err(
                "navigation full terrain SHA-256 revision does not match the scene pin".into(),
            );
        }
        if terrain.width() > orr_navigation_runtime::MAX_SIDE
            || terrain.depth() > orr_navigation_runtime::MAX_SIDE
        {
            return Err("navigation terrain exceeds the 17 by 17 point-agent scope".into());
        }
        Ok(terrain)
    }
}
impl BakeAdmission for LocalNavigationAdmission {
    fn admit(&self, scene: &Scene, frame: &mut Frame, _: &SceneIndex) -> Result<(), EditError> {
        let fail = |message: String| EditError::Invalid(format!("navigation admission: {message}"));
        if !scene.entities.is_empty() || frame.alive_count() != 0 {
            return Err(fail(
                "NavigationYard3D rejects every authored entity, body and collider".into(),
            ));
        }
        if bytemuck::bytes_of(frame.singleton::<NavigationStepStatus>())
            .iter()
            .any(|&byte| byte != 0)
        {
            return Err(fail(
                "runtime failure status cannot be authored into a scene".into(),
            ));
        }
        let pin = *frame.singleton::<NavigationScenePin>();
        let explicit_pin = scene.singletons.iter().any(|(name, _)| name == PIN_NAME);
        if !explicit_pin || bytemuck::bytes_of(&pin).iter().all(|&byte| byte == 0) {
            // An empty document is a deliberate edit-only staging state. It
            // cannot play: the first runtime tick records an authoritative error.
            return Ok(());
        }
        if pin.agent.distance_per_tick <= orr_fp::FP::ZERO {
            return Err(fail("navigation distance per tick must be positive".into()));
        }
        let terrain = self.read_pin(&pin).map_err(fail)?;
        let profile = AgentProfile {
            max_slope: pin.agent.max_slope,
            radius: orr_fp::FP::ZERO,
            headroom: orr_fp::FP::ZERO,
            max_step: orr_fp::FP::ZERO,
        };
        let graph = TerrainGraph::build(&terrain, profile).map_err(|e| fail(e.to_string()))?;
        if graph.revision() != pin.graph_revision {
            return Err(fail(
                "navigation full graph SHA-256 revision does not match the scene pin".into(),
            ));
        }
        NavigationRuntime::admit(frame, &terrain, &graph, pin.agent.to_runtime())
            .map_err(|e| fail(e.to_string()))?;
        orr_games::navigation_yard3d_game::validate_scene_frame(frame).map_err(fail)
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
        return Err("navigation source must be a confined scene-relative .orrt path".into());
    }
    if !Path::new(path)
        .components()
        .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err("navigation source has an unsupported path component".into());
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
                return Err("navigation paths cannot contain parent traversal".into())
            }
            other => checked.push(other.as_os_str()),
        }
        let meta = fs::symlink_metadata(&checked)
            .map_err(|e| format!("cannot inspect {}: {e}", checked.display()))?;
        if meta.file_type().is_symlink() {
            return Err(format!(
                "navigation paths cannot contain symlinks: {}",
                checked.display()
            ));
        }
    }
    if !fs::metadata(&checked).map_err(|e| e.to_string())?.is_file() {
        return Err("navigation source and scene must be regular files".into());
    }
    Ok(checked)
}

pub fn navigation_yard3d_doc_from_yaml(
    yaml: &str,
    admission: LocalNavigationAdmission,
) -> Result<EditorDoc, EditError> {
    EditorDoc::from_yaml_with_admission(
        yaml,
        navigation_yard3d_types(),
        Simulation::<NavigationYard3D>::build_registry(),
        YARD_SEED,
        Some(Arc::new(admission)),
    )
}

pub fn navigation_yard3d_doc_from_path(path: &Path) -> Result<EditorDoc, String> {
    let admission = LocalNavigationAdmission::for_scene(path)?;
    let yaml =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    navigation_yard3d_doc_from_yaml(&yaml, admission)
        .map_err(|e| format!("{}: {e}", path.display()))
}

pub fn configure_navigation_yard3d(limits: &mut HostLimits) {
    limits.allow_scene_paths = false;
    limits.reject_scene_path_overrides = true;
    limits.player_count = YARD_PLAYERS;
    limits.tick_rate = YARD_TICK_RATE;
    limits.build_id = YARD_BUILD_ID;
    limits.game = GameHooks::new("NavigationYard3D");
    limits.view_stream = Some(ViewStreamHook::new(NavigationStream::new(
        limits.build_id,
        limits.player_count,
    )));
}

pub fn spawn_navigation_yard3d_scene_host(
    path: PathBuf,
    mut cfg: ServerConfig,
) -> Result<LocalHost, String> {
    LocalHost::spawn::<NavigationYard3D>(move || {
        let doc = navigation_yard3d_doc_from_path(&path)?;
        cfg.limits.scene_path = Some(path);
        configure_navigation_yard3d(&mut cfg.limits);
        Ok((doc, cfg))
    })
}

struct NavigationStream {
    yard: orr_sample::yard3d_stream::Yard3dStreamProducer,
    schema: Schema,
}
impl NavigationStream {
    fn new(build_id: u64, players: u8) -> Self {
        let yard = orr_sample::yard3d_stream::Yard3dStreamProducer::new(build_id, players);
        let mut schema = yard.schema().clone();
        schema.game = "NavigationYard3D".into();
        Self { yard, schema }
    }
}
impl StreamProducer for NavigationStream {
    fn schema(&self) -> &Schema {
        &self.schema
    }
    fn encode_frame(&mut self, cur: &Frame, prev: Option<&Frame>, meta: FrameMeta) -> Vec<u8> {
        self.yard.encode_frame(cur, prev, meta)
    }
}

/// Readback from the same immutable Frame as the simulation, never from the
/// editor's working copy or a source path that may have changed since admission.
pub struct NavigationView {
    pub terrain: Terrain,
    pub graph: TerrainGraph,
    pub navigator: Navigator,
    pub spec: NavigationAgentSpec,
}
pub fn navigation_from_view(frame: orr_bridge::FrameView<'_>) -> Result<NavigationView, String> {
    let state = *frame.singleton::<RuntimeState>();
    if !state.is_active() || !frame.exists(state.agent) || frame.count::<RuntimeAgent>() != 1 {
        return Err("snapshot has no admitted navigation route".into());
    }
    let agent = frame
        .get::<RuntimeAgent>(state.agent)
        .ok_or("snapshot navigation agent is missing")?;
    let (terrain, graph, navigator, spec) = orr_navigation_runtime::decode_scene(
        &state,
        agent,
        frame.list(state.terrain_bytes),
        frame.list(state.graph_bytes),
        frame.list(state.navigator_bytes),
    )
    .map_err(|e| e.to_string())?;
    let pin = frame.singleton::<NavigationScenePin>();
    if terrain.asset_id() != pin.identity()?
        || terrain.revision() != pin.terrain_revision
        || graph.revision() != pin.graph_revision
        || spec != pin.agent.to_runtime()
    {
        return Err("snapshot navigation route does not match its scene pin".into());
    }
    Ok(NavigationView {
        terrain,
        graph,
        navigator,
        spec: spec.into(),
    })
}
pub fn terrain_from_view(frame: orr_bridge::FrameView<'_>) -> Result<Terrain, String> {
    navigation_from_view(frame).map(|view| view.terrain)
}
