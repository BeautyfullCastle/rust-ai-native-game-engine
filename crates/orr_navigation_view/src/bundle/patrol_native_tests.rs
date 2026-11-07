//! Closed real native CPU evidence; no GPU, window, physical-input claim.
use super::*;
#[test]
fn runtime_capability_reply_is_exact_and_rejects_old_or_overlay_claims() {
    validate_capability_reply(
        b"ORR_TERRAIN_PATROL_SCENE_PLAYER_V1\nORR_TERRAIN_PATROL_BUNDLE_V1\n",
    )
    .unwrap();
    for bad in [
        b"ORR_NAVIGATION_PLAYER_V1\nORR_NAVIGATION_BUNDLE_V1\n".as_slice(),
        b"ORR_TERRAIN_PATROL_SCENE_PLAYER_V1\n",
        b"ORR_TERRAIN_PATROL_SCENE_PLAYER_V1\nORR_TERRAIN_PATROL_BUNDLE_V1\nextra",
        b"prefixORR_TERRAIN_PATROL_SCENE_PLAYER_V1\nORR_TERRAIN_PATROL_BUNDLE_V1\n",
    ] {
        assert!(validate_capability_reply(bad).is_err());
    }
}
#[test]
fn runtime_probe_requires_final_newline_and_order_without_ignoring_extra_output() {
    for bad in [
        b"ORR_TERRAIN_PATROL_SCENE_PLAYER_V1\nORR_TERRAIN_PATROL_BUNDLE_V1".as_slice(),
        b"ORR_TERRAIN_PATROL_BUNDLE_V1\nORR_TERRAIN_PATROL_SCENE_PLAYER_V1\n",
        b"ORR_TERRAIN_PATROL_SCENE_PLAYER_V1\nORR_TERRAIN_PATROL_BUNDLE_V1\n\n",
    ] {
        assert!(validate_capability_reply(bad).is_err());
    }
}
#[test]
#[ignore = "requires explicitly trusted ORR_TERRAIN_PATROL_BUNDLE_RUNTIME and ORR_TERRAIN_PATROL_LEGACY_RUNTIME executables"]
fn native_patrol_bundle_relocates_source_hidden_and_matches_two_hundred_tick_restart() {
    let runtime = PathBuf::from(
        std::env::var_os("ORR_TERRAIN_PATROL_BUNDLE_RUNTIME").expect("trusted runtime required"),
    );
    assert!(runtime.is_absolute());
    let legacy_runtime = PathBuf::from(
        std::env::var_os("ORR_TERRAIN_PATROL_LEGACY_RUNTIME")
            .expect("trusted legacy runtime required"),
    );
    assert!(legacy_runtime.is_absolute());
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let author = root.join("author");
    fs::create_dir_all(author.join("navigation")).unwrap();
    let scene = author.join("point.yaml");
    let terrain = author.join("navigation/point_demo.orrt");
    fs::write(
        &scene,
        include_bytes!("../../../../scenes/navigation_point.scene.yaml"),
    )
    .unwrap();
    fs::write(
        &terrain,
        include_bytes!("../../../../scenes/navigation/point_demo.orrt"),
    )
    .unwrap();
    let mut doc = orr_remote::navigation_yard3d::navigation_yard3d_doc_from_path(&scene).unwrap();
    let pin = *doc.frame().singleton::<NavigationScenePin>();
    let mut c = orr_remote::navigation_yard3d::patrol::TerrainPatrol {
        enabled: 1,
        count: 2,
        looped: 1,
        wait_ticks: 2,
        graph_revision: pin.graph_revision,
        ..Default::default()
    };
    c.points[0] = pin.agent.goal;
    c.points[1] = pin.agent.start;
    orr_remote::navigation_yard3d::patrol::set_patrol(&mut doc, &c).unwrap();
    fs::write(&scene, doc.save_yaml()).unwrap();
    let original_scene = fs::read(&scene).unwrap();
    let original_terrain = fs::read(&terrain).unwrap();
    let source = super::super::SavedSource::capture(&scene).unwrap();
    let mut independent = AuthoredPlayer::open(&scene).unwrap();
    let initial = independent.frame().to_bytes();
    let mut trajectory = Vec::new();
    let mut saw_wait = false;
    let mut saw_loop = false;
    for _ in 0..200 {
        independent.step().unwrap();
        trajectory.push(independent.checksum());
        let p = orr_remote::navigation_yard3d::patrol::from_view(orr_bridge::FrameView::of(
            independent.frame(),
        ))
        .unwrap()
        .unwrap();
        saw_wait |= p.run.wait_remaining > 0;
        saw_loop |= p.run.cycled == 1;
    }
    assert!(saw_wait && saw_loop);
    let final_frame = independent.frame().to_bytes();
    let expected_checksum = independent.checksum();
    let expected = player::run(player::Options {
        scene: scene.clone(),
        ticks: 200,
        restart_check: true,
        capture: None,
    })
    .unwrap();
    independent.restart().unwrap();
    assert_eq!(independent.frame().to_bytes(), initial);
    for checksum in trajectory {
        independent.step().unwrap();
        assert_eq!(independent.checksum(), checksum);
    }
    assert_eq!(independent.frame().to_bytes(), final_frame);
    let local_runtime = root.join("trusted-player");
    fs::copy(&runtime, &local_runtime).unwrap();
    let runtime_bytes = fs::read(&local_runtime).unwrap();
    let dest = root.join("published");
    let request = ExportRequest {
        runtime: local_runtime.clone(),
        runtime_sha256: hash(&runtime_bytes),
        output: dest.clone(),
        trusted_runtime: true,
        source_revision: Some("user-claim-only".into()),
    };
    let legacy_bytes = fs::read(&legacy_runtime).unwrap();
    let rejected_destination = root.join("legacy-runtime-refused");
    let legacy_request = ExportRequest {
        runtime: legacy_runtime.clone(),
        runtime_sha256: hash(&legacy_bytes),
        output: rejected_destination.clone(),
        trusted_runtime: true,
        source_revision: None,
    };
    let error = export_for_source(&legacy_request, &source).unwrap_err();
    assert!(
        error.contains("trusted runtime lacks exact navigation bundle capabilities"),
        "actual legacy runtime must fail its capability probe: {error}"
    );
    assert!(!rejected_destination.exists());
    assert_eq!(fs::read(&legacy_runtime).unwrap(), legacy_bytes);
    assert_eq!(fs::read(&scene).unwrap(), original_scene);
    assert_eq!(fs::read(&terrain).unwrap(), original_terrain);
    export_for_source(&request, &source).unwrap();
    assert_eq!(fs::read(&scene).unwrap(), original_scene);
    assert_eq!(fs::read(&terrain).unwrap(), original_terrain);
    assert_eq!(fs::read(&local_runtime).unwrap(), runtime_bytes);
    assert!(
        export_for_source(&request, &source).is_err(),
        "new-only output must not be replaced"
    );
    let relocated = root.join("relocated");
    fs::rename(&dest, &relocated).unwrap();
    fs::remove_file(&scene).unwrap();
    fs::remove_file(&terrain).unwrap();
    fs::remove_file(&local_runtime).unwrap();
    let manifest = fs::read(relocated.join(MANIFEST_PATH)).unwrap();
    let m: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    assert_eq!(m["payload"]["profile"], "NavigationYard3DPatrolV1");
    let invoke = || {
        let mut command = Command::new(relocated.join(RUNTIME_PATH));
        command
            .arg("--bundle")
            .arg(&relocated)
            .args(["--ticks", "200", "--restart-check"]);
        let bytes = run_bounded(&mut command, Duration::from_secs(30), 65536).unwrap();
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
    };
    let first = invoke();
    assert_eq!(first, invoke());
    assert_eq!(first, expected);
    assert_eq!(first["checksum"], expected_checksum);
    assert_eq!(first["profile"], "NavigationYard3DPatrolV1");
    assert_eq!(first["restart_checked"], true);
    assert_eq!(first["patrol"]["count"], 2);
    assert_eq!(first["patrol"]["looped"], 1);
    assert_eq!(fs::read(relocated.join(MANIFEST_PATH)).unwrap(), manifest);
    let reopened = verify_bundle(&relocated, None).unwrap();
    assert_eq!(reopened.frame().to_bytes(), initial);
}
