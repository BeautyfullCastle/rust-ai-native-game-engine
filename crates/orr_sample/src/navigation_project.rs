//! Closed Terrain point-route project admission, completed before GPU or play.
//! Files are bounded, regular and symlink-free. Startup retains owned snapshots;
//! this is not a sandbox against concurrent hostile filesystem replacement.
use orr_ecs::Frame;
use orr_fp::FrameRng;
use orr_games::navigation_yard3d_game::{self as game, AdmissionMode, NavigationYard3D};
use orr_reflect::{Scene, SceneIndex, TypeRegistry};
use orr_sim::Simulation;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub const GAME: &str = "NavigationYard3D";
pub const SEED: u64 = 42;
pub const PLAYERS: u8 = 2;
pub const TICK_RATE: u32 = orr_games::yard3d_game::TICK_RATE;
pub const MAX_SCENE_BYTES: u64 = 256 * 1024;
pub const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

/// Preserve the existing NavigationYard3D host/replay identity.
pub const fn build_id() -> u64 {
    0
}

pub fn types() -> TypeRegistry {
    let mut types = TypeRegistry::new();
    game::register_reflect(&mut types);
    types
}

pub fn compiled_runtime() -> orr_package::Runtime {
    orr_package::Runtime::content_only()
}

/// Closed authored schema; derived agent, runtime status and physics state are
/// never authored into this project, even if an unrelated feature is unified.
pub fn validate_document(scene: &Scene) -> Result<(), String> {
    if scene.has_prefab_links()
        || !scene.entities.is_empty()
        || scene.singletons.len() != 1
        || scene.singletons[0].0 != game::PIN_NAME
    {
        return Err("navigation project requires only an explicit NavigationScenePin and no entities or prefabs".into());
    }
    Ok(())
}

fn bake(text: &str) -> Result<(Frame, SceneIndex), String> {
    if text.len() as u64 > MAX_SCENE_BYTES {
        return Err("navigation scene exceeds 256 KiB".into());
    }
    let types = types();
    let scene = Scene::parse(text, &types).map_err(|e| format!("navigation scene: {e}"))?;
    validate_document(&scene)?;
    let mut frame = Frame::new(Simulation::<NavigationYard3D>::build_registry());
    frame.set_singleton(FrameRng::new(SEED));
    let index = scene
        .bake(&types, &mut frame)
        .map_err(|e| format!("navigation bake: {e}"))?;
    game::admission_pin(&frame, 0, true, AdmissionMode::RequiredPin)?;
    Ok((frame, index))
}

pub struct PreparedScene {
    text: Arc<str>,
    frame: Frame,
    index: SceneIndex,
}
impl PreparedScene {
    pub fn parse(text: &str, terrain_bytes: &[u8]) -> Result<Self, String> {
        let (frame, index) = bake(text)?;
        Self::from_baked(text, frame, index, terrain_bytes)
    }

    fn from_baked(
        text: &str,
        mut frame: Frame,
        index: SceneIndex,
        terrain_bytes: &[u8],
    ) -> Result<Self, String> {
        if terrain_bytes.len() > orr_navigation_runtime::MAX_TERRAIN_BYTES {
            return Err("navigation project terrain exceeds the point-agent byte limit".into());
        }
        let terrain = orr_terrain::Terrain::load(terrain_bytes).map_err(|e| e.to_string())?;
        game::validate_project_bounds(
            &terrain,
            frame.singleton::<game::NavigationScenePin>().agent,
        )?;
        game::admit_scene(
            &mut frame,
            0,
            true,
            Some(terrain_bytes),
            AdmissionMode::RequiredPin,
        )?;
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
    pub fn simulation(&self) -> Result<Simulation<NavigationYard3D>, String> {
        Simulation::from_frame(&self.frame, TICK_RATE, build_id()).map_err(|e| e.to_string())
    }
    pub fn session(&self) -> Result<orr_bridge::PlaySession<NavigationYard3D>, String> {
        let mut config = orr_bridge::PlayConfig::new(PLAYERS, SEED, TICK_RATE);
        config.game_id = GAME.into();
        config.build_id = build_id();
        config.start_paused = false;
        orr_bridge::PlaySession::from_frame(config, &self.frame).map_err(|e| e.to_string())
    }
}

pub struct PreparedProject {
    root: PathBuf,
    path: PathBuf,
    manifest_bytes: Vec<u8>,
    scene: PreparedScene,
    terrain_path: PathBuf,
    terrain_bytes: Vec<u8>,
}
impl PreparedProject {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, String> {
        let project =
            orr_package::Project::open(root, compiled_runtime()).map_err(|e| e.to_string())?;
        let manifest = project
            .manifest()
            .ok_or("navigation project manifest missing")?;
        let entry = manifest
            .entry
            .as_ref()
            .ok_or("navigation project entry missing")?;
        if manifest.schema != 2
            || entry.game != orr_package::ProjectGame::TerrainPointRoute3dV1
            || entry.ui.is_some()
            || entry.camera.is_some()
            || entry.models.is_some()
            || entry.sprites.is_some()
            || manifest.progress.is_some()
        {
            return Err("navigation requires schema2 terrain-point-route-3d-v1 with no UI, camera, models, sprites or progress".into());
        }
        let before = project
            .verify()
            .map_err(|e| format!("navigation active lock: {e}"))?;
        let root = project.root().to_path_buf();
        let manifest_path = crate::project::entry_file(&root, "orr.project.json")?;
        let manifest_bytes = crate::project::read_regular(&manifest_path, MAX_MANIFEST_BYTES)?;
        let shape: serde_json::Value = serde_json::from_slice(&manifest_bytes)
            .map_err(|e| format!("navigation manifest JSON: {e}"))?;
        if !shape.is_object() || !shape.get("entry").is_some_and(serde_json::Value::is_object) {
            return Err("navigation manifest and entry must be JSON objects".into());
        }
        // Deserialize the original bytes, never the Value: duplicate fields and
        // unknown keys must retain the typed schema's strict rejection behavior.
        let captured: orr_package::ProjectManifest =
            serde_json::from_slice(&manifest_bytes).map_err(|e| e.to_string())?;
        if captured != *manifest {
            return Err("navigation manifest changed during admission".into());
        }
        let path = crate::project::entry_file(&root, &entry.scene)?;
        let scene_bytes = crate::project::read_regular(&path, MAX_SCENE_BYTES)?;
        let text =
            std::str::from_utf8(&scene_bytes).map_err(|_| "navigation scene is not UTF-8")?;
        let (frame, index) = bake(text)?;
        let pin = frame.singleton::<game::NavigationScenePin>();
        let scene_root = path
            .parent()
            .ok_or("navigation scene needs a parent directory")?;
        // The pin is scene-relative, never project-relative or working-directory relative.
        let terrain_path = crate::project::entry_file(scene_root, pin.source()?)?;
        let terrain_bytes = crate::project::read_regular(
            &terrain_path,
            orr_navigation_runtime::MAX_TERRAIN_BYTES as u64,
        )?;
        let scene = PreparedScene::from_baked(text, frame, index, &terrain_bytes)?;
        if project
            .verify()
            .map_err(|e| format!("navigation final active lock: {e}"))?
            != before
            || crate::project::read_regular(&manifest_path, MAX_MANIFEST_BYTES)? != manifest_bytes
            || crate::project::read_regular(&path, MAX_SCENE_BYTES)? != scene_bytes
            || crate::project::read_regular(
                &terrain_path,
                orr_navigation_runtime::MAX_TERRAIN_BYTES as u64,
            )? != terrain_bytes
        {
            return Err("navigation project changed during admission; retry".into());
        }
        Ok(Self {
            root,
            path,
            manifest_bytes,
            scene,
            terrain_path,
            terrain_bytes,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn manifest_bytes(&self) -> &[u8] {
        &self.manifest_bytes
    }
    pub fn scene(&self) -> &PreparedScene {
        &self.scene
    }
    pub fn terrain_path(&self) -> &Path {
        &self.terrain_path
    }
    pub fn terrain_bytes(&self) -> &[u8] {
        &self.terrain_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_sim::TickInputs;
    use std::fs;

    const YAML: &str = include_str!("../../../scenes/navigation_point.scene.yaml");
    const TERRAIN: &[u8] = include_bytes!("../../../scenes/navigation/point_demo.orrt");
    const MANIFEST: &str = r#"{"schema":2,"engine":"^0.0.1","entry":{"game":"terrain-point-route-3d-v1","scene":"scenes/point.scene.yaml"}}"#;

    struct Fixture {
        _temp: tempfile::TempDir,
        root: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap().join("point project");
            fs::create_dir_all(root.join("scenes/navigation")).unwrap();
            fs::write(root.join("orr.project.json"), MANIFEST).unwrap();
            fs::write(root.join("scenes/point.scene.yaml"), YAML).unwrap();
            fs::write(root.join("scenes/navigation/point_demo.orrt"), TERRAIN).unwrap();
            Self { _temp: temp, root }
        }
        fn rejected(&self) {
            assert!(PreparedProject::open(&self.root).is_err());
        }
    }

    fn scene_with_pin(change: impl FnOnce(&mut game::NavigationScenePin)) -> String {
        let (frame, _) = bake(YAML).unwrap();
        let mut pin = *frame.singleton::<game::NavigationScenePin>();
        change(&mut pin);
        let registry = types();
        let mut scene = Scene::parse(YAML, &registry).unwrap();
        scene.singletons[0].1 = registry
            .get(game::PIN_NAME)
            .unwrap()
            .read(bytemuck::bytes_of(&pin));
        scene.to_yaml()
    }

    #[test]
    fn exact_frame_session_simulation_parity_survives_every_source_deletion() {
        let fixture = Fixture::new();
        let project = PreparedProject::open(&fixture.root).unwrap();
        assert_eq!(project.root(), fixture.root);
        assert_eq!(project.path(), fixture.root.join("scenes/point.scene.yaml"));
        assert_eq!(
            project.terrain_path(),
            fixture.root.join("scenes/navigation/point_demo.orrt")
        );
        assert_eq!(project.terrain_bytes(), TERRAIN);
        assert_eq!(project.manifest_bytes(), MANIFEST.as_bytes());
        assert_eq!(project.scene().text(), YAML);
        let initial = project.scene().frame().to_bytes();
        fs::remove_dir_all(&fixture.root).unwrap();
        let mut simulation = project.scene().simulation().unwrap();
        let mut session = project.scene().session().unwrap();
        assert_eq!(session.frame().to_bytes(), initial);
        assert_eq!(simulation.frame().to_bytes(), initial);
        for _ in 0..12 {
            simulation.step(&TickInputs::new(simulation.tick(), PLAYERS));
            session.step_now().unwrap();
            assert_eq!(simulation.frame().to_bytes(), session.frame().to_bytes());
        }
        assert_eq!(
            simulation
                .frame()
                .singleton::<game::NavigationStepStatus>()
                .failed,
            0
        );
        assert_eq!(project.scene().frame().to_bytes(), initial);
        assert_eq!(
            project.scene().session().unwrap().frame().to_bytes(),
            initial
        );
    }

    #[test]
    fn closed_manifest_rejects_wrong_games_versions_and_unrelated_profiles() {
        let fixture = Fixture::new();
        for field in ["sprites", "models", "camera", "ui", "unknown"] {
            let mut manifest: serde_json::Value = serde_json::from_str(MANIFEST).unwrap();
            manifest["entry"][field] = serde_json::json!("unsupported.json");
            fs::write(
                fixture.root.join("orr.project.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            fixture.rejected();
        }
        for (path, value) in [
            ("schema", serde_json::json!(1)),
            ("schema", serde_json::json!(3)),
            ("engine", serde_json::json!(">=99.0.0")),
            ("progress", serde_json::json!({})),
        ] {
            let mut manifest: serde_json::Value = serde_json::from_str(MANIFEST).unwrap();
            manifest[path] = value;
            fs::write(
                fixture.root.join("orr.project.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            fixture.rejected();
        }
        fs::write(
            fixture.root.join("orr.project.json"),
            MANIFEST.replace("terrain-point-route-3d-v1", "arena"),
        )
        .unwrap();
        fixture.rejected();
        fs::write(fixture.root.join("orr.project.json"), MANIFEST).unwrap();
        fs::write(
            fixture.root.join("orr.packages.lock.json"),
            b"broken active lock",
        )
        .unwrap();
        fixture.rejected();
    }

    #[test]
    fn manifest_rejects_positional_struct_arrays_and_duplicate_fields() {
        let fixture = Fixture::new();
        for text in [
            r#"[2,"^0.0.1",{"game":"terrain-point-route-3d-v1","scene":"scenes/point.scene.yaml"}]"#,
            r#"[2,"^0.0.1",{"game":"terrain-point-route-3d-v1","scene":"scenes/point.scene.yaml"},null]"#,
            r#"{"schema":2,"engine":"^0.0.1","entry":[null,"terrain-point-route-3d-v1","scenes/point.scene.yaml",null,null,null]}"#,
            r#"{"schema":2,"schema":2,"engine":"^0.0.1","entry":{"game":"terrain-point-route-3d-v1","scene":"scenes/point.scene.yaml"}}"#,
            r#"{"schema":2,"engine":"^0.0.1","entry":{"game":"terrain-point-route-3d-v1","scene":"scenes/point.scene.yaml","scene":"scenes/point.scene.yaml"}}"#,
        ] {
            fs::write(fixture.root.join("orr.project.json"), text).unwrap();
            fixture.rejected();
        }
    }

    #[test]
    fn closed_scene_rejects_staging_entities_status_and_tampered_full_pins() {
        assert!(PreparedScene::parse(
            "schema: orr.scene/1\nsingletons: {}\nentities: {}\n",
            TERRAIN
        )
        .is_err());
        let registry = types();
        let mut scene = Scene::parse(YAML, &registry).unwrap();
        scene
            .entities
            .insert(orr_reflect::Guid::from_u32(1), Default::default());
        assert!(PreparedScene::parse(&scene.to_yaml(), TERRAIN).is_err());
        let mut scene = Scene::parse(YAML, &registry).unwrap();
        let status = registry.get("NavigationStepStatus").unwrap();
        scene.singletons.push((
            "NavigationStepStatus".into(),
            status.read(&status.default_bytes()),
        ));
        assert!(PreparedScene::parse(&scene.to_yaml(), TERRAIN).is_err());
        for target in ["identity", "terrain", "graph", "tail", "zero"] {
            let text = scene_with_pin(|pin| match target {
                "identity" => pin.asset_id[0] ^= 1,
                "terrain" => pin.terrain_revision[31] ^= 1,
                "graph" => pin.graph_revision[31] ^= 1,
                "tail" => pin.source_path[255] = 1,
                "zero" => *pin = bytemuck::Zeroable::zeroed(),
                _ => unreachable!(),
            });
            assert!(PreparedScene::parse(&text, TERRAIN).is_err(), "{target}");
        }
        let mut corrupt = TERRAIN.to_vec();
        corrupt[0] ^= 1;
        assert!(PreparedScene::parse(YAML, &corrupt).is_err());
        let oversized = vec![0; orr_navigation_runtime::MAX_TERRAIN_BYTES + 1];
        assert!(PreparedScene::parse(YAML, &oversized).is_err());
    }

    #[test]
    fn project_admission_checks_interior_height_before_route_or_presentation() {
        use orr_fp::FP;
        for raw in [256 * 65536 + 1, -256 * 65536 - 1] {
            let mut heights = vec![FP::ZERO; 9];
            heights[4] = FP::from_raw(raw);
            let terrain = orr_terrain::Terrain::new(
                "interior-bound".into(),
                3,
                3,
                [FP::ZERO; 2],
                FP::ONE,
                heights,
                vec![false; 4],
            )
            .unwrap();
            let graph = orr_navigation::TerrainGraph::build(
                &terrain,
                orr_navigation::AgentProfile::default(),
            )
            .unwrap();
            let text = scene_with_pin(|pin| {
                *pin = game::NavigationScenePin::new("terrain.orrt", &terrain, &graph, pin.agent)
                    .unwrap();
            });
            let error = PreparedScene::parse(&text, &terrain.cook()).err().unwrap();
            assert!(
                error.contains("terrain XYZ must all be within +/-256"),
                "{error}"
            );
        }
    }

    #[test]
    fn traversal_missing_tampered_and_oversize_files_are_rejected() {
        for source in [
            "../outside.orrt",
            "/outside.orrt",
            "navigation/../point_demo.orrt",
            ".orr/point_demo.orrt",
            "navigation\\point_demo.orrt",
        ] {
            let fixture = Fixture::new();
            let text = scene_with_pin(|pin| {
                pin.source_path.fill(0);
                pin.source_path[..source.len()].copy_from_slice(source.as_bytes());
                pin.source_len = source.len() as u32;
            });
            fs::write(fixture.root.join("scenes/point.scene.yaml"), text).unwrap();
            fixture.rejected();
        }
        for relative in [
            "orr.project.json",
            "scenes/point.scene.yaml",
            "scenes/navigation/point_demo.orrt",
        ] {
            let fixture = Fixture::new();
            let path = fixture.root.join(relative);
            fs::write(&path, b"tampered").unwrap();
            fixture.rejected();
            fs::write(&path, vec![b' '; MAX_MANIFEST_BYTES as usize + 1]).unwrap();
            fixture.rejected();
            fs::remove_file(path).unwrap();
            fixture.rejected();
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_and_fifos_are_rejected_before_opening() {
        for relative in [
            "orr.project.json",
            "scenes/point.scene.yaml",
            "scenes/navigation/point_demo.orrt",
        ] {
            let fixture = Fixture::new();
            let path = fixture.root.join(relative);
            let kept = fixture.root.join("kept");
            fs::rename(&path, &kept).unwrap();
            std::os::unix::fs::symlink(&kept, &path).unwrap();
            fixture.rejected();
            fs::remove_file(&path).unwrap();
            assert!(std::process::Command::new("mkfifo")
                .arg(&path)
                .status()
                .unwrap()
                .success());
            fixture.rejected();
        }
        let fixture = Fixture::new();
        let original = fixture.root.join("scenes/navigation");
        let kept = fixture.root.join("kept_directory");
        fs::rename(&original, &kept).unwrap();
        std::os::unix::fs::symlink(&kept, &original).unwrap();
        fixture.rejected();
    }
}
