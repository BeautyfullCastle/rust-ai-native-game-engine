use super::*;
fn payload(profile: Profile) -> Payload {
    let file = |path: &str, bytes: &[u8], mode, role| Descriptor {
        path: path.into(),
        bytes: bytes.len(),
        mode,
        sha256: hash(bytes),
        role,
    };
    Payload {
        version: 1,
        profile,
        target: Target::LinuxX86_64,
        source_revision: None,
        initial_checksum: 42,
        terrain_asset_id: "terrain-fixture".into(),
        terrain_revision: "a".repeat(64),
        graph_revision: "b".repeat(64),
        files: vec![
            file(SCENE_PATH, b"s", 0o644, Role::Scene),
            file("project/navigation/test.orrt", b"t", 0o644, Role::Terrain),
            file(RUNTIME_PATH, b"r", 0o755, Role::Runtime),
            file(LAUNCHER_PATH, LAUNCHER, 0o755, Role::Launcher),
        ],
    }
}
fn manifest(profile: Profile) -> Manifest {
    let payload = payload(profile);
    Manifest {
        content_sha256: digest(&payload).unwrap(),
        payload,
    }
}
#[test]
fn compiled_profile_follows_exact_registry_and_bundle_feature_pair() {
    let actual = super::super::compiled_profile();
    match (
        orr_games::navigation_yard3d_game::TERRAIN_PATROL_ENABLED,
        cfg!(feature = "terrain-patrol-bundle"),
    ) {
        (false, false) => assert_eq!(actual.unwrap(), "NavigationYard3DPointV1"),
        (true, true) => assert_eq!(actual.unwrap(), "NavigationYard3DPatrolV1"),
        _ => assert!(actual.is_err()),
    }
}
#[test]
fn legacy_point_payload_has_independent_prechange_content_hash() {
    assert_eq!(
        digest(&payload(Profile::NavigationYard3DPointV1)).unwrap(),
        "07b29ba3868336f754f44131f548c415830344fe83195cf11ef287ae439a6e66"
    );
}
#[test]
fn matching_profile_roundtrips_and_strict_wire_rejects_duplicates() {
    let Ok(profile) = Profile::compiled() else {
        assert!(Manifest::parse(b"{}").is_err());
        return;
    };
    let m = manifest(profile);
    let bytes = m.encode().unwrap();
    assert_eq!(Manifest::parse(&bytes).unwrap(), m);
    let text = String::from_utf8(bytes).unwrap();
    for bad in [
        text.replacen("\"version\": 1", "\"version\": 1, \"version\": 1", 1),
        text.replace("\"target\": \"linux-x86_64\"", "\"target\": {}"),
        format!("[{text}]"),
    ] {
        assert!(Manifest::parse(bad.as_bytes()).is_err());
    }
}
#[test]
fn opposite_profile_is_rejected_even_with_recomputed_content_hash() {
    let opposite = if cfg!(feature = "terrain-patrol-bundle") {
        Profile::NavigationYard3DPointV1
    } else {
        Profile::NavigationYard3DPatrolV1
    };
    let forged = manifest(opposite);
    let bytes = serde_json::to_vec(&forged).unwrap();
    assert!(Manifest::parse(&bytes).is_err());
    let Ok(profile) = Profile::compiled() else {
        return;
    };
    let mut m = manifest(profile);
    m.payload.initial_checksum += 1;
    assert!(Manifest::parse(&serde_json::to_vec(&m).unwrap()).is_err());
}
#[cfg(feature = "terrain-patrol-bundle")]
mod captured {
    use super::*;
    use std::fs;
    fn scene(root: &std::path::Path) -> std::path::PathBuf {
        fs::create_dir(root.join("navigation")).unwrap();
        let scene = root.join("patrol.scene.yaml");
        fs::write(
            root.join("navigation/point_demo.orrt"),
            include_bytes!("../../../../scenes/navigation/point_demo.orrt"),
        )
        .unwrap();
        fs::write(
            &scene,
            include_str!("../../../../scenes/navigation_point.scene.yaml"),
        )
        .unwrap();
        let mut doc =
            orr_remote::navigation_yard3d::navigation_yard3d_doc_from_path(&scene).unwrap();
        let pin = *doc
            .frame()
            .singleton::<orr_remote::navigation_yard3d::NavigationScenePin>();
        let mut config = orr_remote::navigation_yard3d::patrol::TerrainPatrol::default();
        config.enabled = 1;
        config.count = 2;
        config.looped = 1;
        config.wait_ticks = 1;
        config.graph_revision = pin.graph_revision;
        config.points[0] = pin.agent.goal;
        config.points[1] = pin.agent.start;
        orr_remote::navigation_yard3d::patrol::set_patrol(&mut doc, &config).unwrap();
        fs::write(&scene, doc.save_yaml()).unwrap();
        scene
    }
    #[test]
    fn saved_patrol_capture_binds_initial_frame_and_runtime_open_is_source_free() {
        let root = tempfile::tempdir().unwrap();
        let path = scene(root.path());
        let saved = super::super::super::SavedSource::capture(&path).unwrap();
        let mut player = AuthoredPlayer::open(&path).unwrap();
        assert_eq!(saved.initial_frame_bytes(), player.frame().to_bytes());
        assert_eq!(saved.initial_checksum(), player.checksum());
        let initial = player.checksum();
        fs::rename(
            root.path().join("navigation/point_demo.orrt"),
            root.path().join("hidden.orrt"),
        )
        .unwrap();
        let mut trajectory = Vec::new();
        for _ in 0..80 {
            player.step().unwrap();
            trajectory.push(player.checksum());
        }
        player.restart().unwrap();
        assert_eq!(player.checksum(), initial);
        for hash in trajectory {
            player.step().unwrap();
            assert_eq!(player.checksum(), hash);
        }
        assert!(saved.recheck().is_err());
    }
    #[test]
    fn captured_manifest_uses_patrol_profile_and_actual_admission_checksum() {
        let root = tempfile::tempdir().unwrap();
        let path = scene(root.path());
        let player = AuthoredPlayer::open(&path).unwrap();
        let source = Snapshot::read(&path, MAX_SCENE).unwrap();
        let terrain = Snapshot::read(
            &root.path().join("navigation/point_demo.orrt"),
            orr_terrain::MAX_FILE_BYTES,
        )
        .unwrap();
        let runtime_path = root.path().join("runtime");
        fs::write(&runtime_path, b"trusted fixture runtime snapshot").unwrap();
        let runtime = Snapshot::read(&runtime_path, MAX_RUNTIME).unwrap();
        let m = Manifest::from_snapshots(
            &source,
            "navigation/point_demo.orrt",
            &terrain,
            &runtime,
            None,
            &player,
        )
        .unwrap();
        assert_eq!(m.payload.profile, Profile::NavigationYard3DPatrolV1);
        assert_eq!(m.payload.initial_checksum, player.checksum());
        assert_eq!(Manifest::parse(&m.encode().unwrap()).unwrap(), m);
    }
}
