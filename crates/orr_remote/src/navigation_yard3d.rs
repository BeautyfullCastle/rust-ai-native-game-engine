//! Local scene-relative admission for deterministic point navigation.
//! Filesystem reads occur only while baking; play, seek, replay and views use Frame bytes.
use crate::{GameHooks, HostLimits, LocalHost, ServerConfig, ViewStreamHook};
use orr_ecs::Frame;
use orr_edit::{BakeAdmission, EditError, EditorDoc};
use orr_games::navigation_yard3d_game::{self as game, AdmissionMode};
use orr_navigation::{Navigator, TerrainGraph};
use orr_navigation_runtime::{RuntimeAgent, RuntimeState};
use orr_reflect::{Scene, SceneIndex, TypeRegistry};
use orr_sim::Simulation;
use orr_terrain::Terrain;
use orr_viewstream::{FrameMeta, Schema, StreamProducer};
use std::{
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
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

    fn read_pin(&self, pin: &NavigationScenePin) -> Result<Vec<u8>, String> {
        let relative = pin.source()?;
        game::validate_relative_source(relative)?;
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
        Ok(bytes)
    }
}
impl BakeAdmission for LocalNavigationAdmission {
    fn admit(&self, scene: &Scene, frame: &mut Frame, _: &SceneIndex) -> Result<(), EditError> {
        let fail = |message: String| EditError::Invalid(format!("navigation admission: {message}"));
        let explicit_pin = scene.singletons.iter().any(|(name, _)| name == PIN_NAME);
        let pin = game::admission_pin(
            frame,
            scene.entities.len(),
            explicit_pin,
            AdmissionMode::EditorStaging,
        )
        .map_err(fail)?;
        let bytes = pin
            .as_ref()
            .map(|pin| self.read_pin(pin))
            .transpose()
            .map_err(fail)?;
        game::admit_scene(
            frame,
            scene.entities.len(),
            explicit_pin,
            bytes.as_deref(),
            AdmissionMode::EditorStaging,
        )
        .map_err(fail)
    }
    fn allow_play_edits(&self) -> bool {
        false
    }
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

/// Captured startup bytes are consumed exactly once. Every later explicit edit
/// or rebake reopens the current confined source and checks its complete pin.
struct OwnedNavigationAdmission {
    live: LocalNavigationAdmission,
    initial: Mutex<Option<Vec<u8>>>,
}
impl BakeAdmission for OwnedNavigationAdmission {
    fn admit_source(&self, text: &str) -> Result<(), EditError> {
        if text.len() > 256 * 1024 {
            return Err(EditError::Invalid(
                "navigation scene exceeds 256 KiB".into(),
            ));
        }
        Ok(())
    }

    fn admit(&self, scene: &Scene, frame: &mut Frame, _: &SceneIndex) -> Result<(), EditError> {
        let fail = |message: String| EditError::Invalid(format!("navigation admission: {message}"));
        if scene.has_prefab_links()
            || !scene.entities.is_empty()
            || scene.singletons.len() != 1
            || scene.singletons[0].0 != PIN_NAME
        {
            return Err(fail("navigation project requires only an explicit NavigationScenePin and no entities or prefabs".into()));
        }
        let pin = game::admission_pin(frame, 0, true, AdmissionMode::RequiredPin)
            .map_err(fail)?
            .ok_or_else(|| fail("navigation project pin missing".into()))?;
        let captured = self
            .initial
            .lock()
            .map_err(|_| fail("navigation initial admission unavailable".into()))?
            .take();
        let bytes = match captured {
            Some(bytes) => bytes,
            None => self.live.read_pin(&pin).map_err(fail)?,
        };
        let terrain = Terrain::load(&bytes).map_err(|e| fail(e.to_string()))?;
        game::validate_project_bounds(&terrain, pin.agent).map_err(fail)?;
        game::admit_scene(frame, 0, true, Some(&bytes), AdmissionMode::RequiredPin).map_err(fail)
    }

    fn allow_play_edits(&self) -> bool {
        false
    }
}

/// Start a closed project document entirely from admitted owned bytes. The path
/// is already canonical in PreparedProject and is used only as future authority;
/// startup does not reopen the scene, terrain, or its containing directory.
pub fn navigation_yard3d_doc_from_owned(
    path: &Path,
    text: &str,
    terrain_bytes: Vec<u8>,
) -> Result<EditorDoc, String> {
    let normalized: PathBuf = path.components().collect();
    if !path.is_absolute()
        || path.file_name().is_none()
        || path
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
        || normalized.as_os_str() != path.as_os_str()
    {
        return Err("owned navigation scene path must be absolute and normalized".into());
    }
    let root = path
        .parent()
        .ok_or("navigation scene needs a parent directory")?
        .to_path_buf();
    EditorDoc::from_yaml_with_admission(
        text,
        navigation_yard3d_types(),
        Simulation::<NavigationYard3D>::build_registry(),
        YARD_SEED,
        Some(Arc::new(OwnedNavigationAdmission {
            live: LocalNavigationAdmission { root },
            initial: Mutex::new(Some(terrain_bytes)),
        })),
    )
    .map_err(|e| e.to_string())
}

pub fn spawn_navigation_yard3d_owned_host(
    path: PathBuf,
    text: String,
    terrain_bytes: Vec<u8>,
    mut cfg: ServerConfig,
) -> Result<LocalHost, String> {
    LocalHost::spawn::<NavigationYard3D>(move || {
        let doc = navigation_yard3d_doc_from_owned(&path, &text, terrain_bytes)?;
        cfg.limits.scene_path = Some(path);
        configure_navigation_yard3d(&mut cfg.limits);
        Ok((doc, cfg))
    })
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

#[cfg(test)]
mod owned_admission_tests {
    use super::*;

    const YAML: &str = include_str!("../../../scenes/navigation_point.scene.yaml");
    const TERRAIN: &[u8] = include_bytes!("../../../scenes/navigation/point_demo.orrt");

    #[test]
    fn owned_start_never_reopens_files_and_later_bakes_require_current_source() {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        struct Temporary(PathBuf);
        impl Drop for Temporary {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let root = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "orr-navigation-owned-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir(&root).unwrap();
        let _temporary = Temporary(root.clone());
        let path = root.join("point.scene.yaml");
        let mut doc = navigation_yard3d_doc_from_owned(&path, YAML, TERRAIN.to_vec()).unwrap();
        let before = doc.frame().to_bytes();
        assert_eq!(doc.frame().alive_count(), 1);
        assert!(!path.exists());
        assert!(doc.rebake().is_err());
        assert_eq!(doc.frame().to_bytes(), before);
        fs::create_dir(root.join("navigation")).unwrap();
        let terrain_path = root.join("navigation/point_demo.orrt");
        fs::write(&terrain_path, TERRAIN).unwrap();
        doc.rebake().unwrap();
        assert_eq!(doc.frame().to_bytes(), before);
        fs::write(&terrain_path, b"tampered after startup").unwrap();
        assert!(doc.rebake().is_err());
        assert_eq!(doc.frame().to_bytes(), before);
        // Valid disk bytes never substitute for rejected initial owned bytes.
        fs::write(&terrain_path, TERRAIN).unwrap();
        assert!(
            navigation_yard3d_doc_from_owned(&path, YAML, b"bad captured bytes".to_vec()).is_err()
        );
    }

    #[test]
    fn owned_start_rejects_noncanonical_paths_and_empty_staging() {
        for path in [
            "relative.scene.yaml",
            "/tmp/../point.scene.yaml",
            "/tmp//point.scene.yaml",
            "/tmp/./point.scene.yaml",
        ] {
            assert!(
                navigation_yard3d_doc_from_owned(Path::new(path), YAML, TERRAIN.to_vec()).is_err(),
                "{path}"
            );
        }
        assert!(navigation_yard3d_doc_from_owned(
            Path::new("/tmp/point.scene.yaml"),
            "schema: orr.scene/1\nsingletons: {}\nentities: {}\n",
            TERRAIN.to_vec()
        )
        .is_err());
    }
}
