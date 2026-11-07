#[test]
fn legacy_bundle_capture_rejects_new_compiled_profile_before_source_io() {
    let error = crate::bundle::SavedSource::capture(std::path::Path::new(
        "/missing-terrain-patrol.scene.yaml",
    ))
    .err()
    .unwrap();
    assert_eq!(
        error.contains("does not support terrain patrol profile"),
        orr_games::navigation_yard3d_game::TERRAIN_PATROL_ENABLED
    );
}

#[test]
fn legacy_surface_bundle_capture_rejects_new_compiled_profile_before_source_io() {
    let error = crate::surface_bundle::SavedSource::capture(std::path::Path::new(
        "/missing-terrain-patrol.scene.yaml",
    ))
    .err()
    .unwrap();
    assert_eq!(
        error.contains("does not support terrain patrol profile"),
        orr_games::navigation_yard3d_game::TERRAIN_PATROL_ENABLED
    );
}

#[cfg(feature = "terrain-patrol")]
#[test]
fn authored_reader_restarts_from_frame_after_source_is_hidden() {
    // This test is selected only with the explicit game/host patrol feature in CI.
    let root = tempfile::tempdir().unwrap();
    let scene = root.path().join("patrol.scene.yaml");
    let asset = root.path().join("navigation/point_demo.orrt");
    std::fs::create_dir(root.path().join("navigation")).unwrap();
    std::fs::write(
        &asset,
        include_bytes!("../../../scenes/navigation/point_demo.orrt"),
    )
    .unwrap();
    std::fs::write(
        &scene,
        include_str!("../../../scenes/navigation_point.scene.yaml"),
    )
    .unwrap();
    let mut doc = orr_remote::navigation_yard3d::navigation_yard3d_doc_from_path(&scene).unwrap();
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
    std::fs::write(&scene, doc.save_yaml()).unwrap();
    let mut reader = crate::authored::AuthoredPlayer::open(&scene).unwrap();
    let initial = reader.checksum();
    std::fs::rename(&asset, asset.with_extension("hidden")).unwrap();
    let mut checksums = Vec::new();
    let mut cursors = std::collections::BTreeSet::new();
    for _ in 0..120 {
        reader.step().unwrap();
        checksums.push(reader.checksum());
        let view = orr_remote::navigation_yard3d::patrol::from_view(orr_bridge::FrameView::of(
            reader.frame(),
        ))
        .unwrap()
        .unwrap();
        cursors.insert(view.run.cursor);
    }
    assert_eq!(cursors.len(), 2);
    reader.restart().unwrap();
    assert_eq!(reader.checksum(), initial);
    for expected in checksums {
        reader.step().unwrap();
        assert_eq!(reader.checksum(), expected);
    }
}
#[test]
fn runtime_capabilities_and_bundle_commands_match_compiled_profile() {
    let enabled = orr_games::navigation_yard3d_game::TERRAIN_PATROL_ENABLED;
    if enabled {
        assert_eq!(
            crate::player::capabilities(),
            "ORR_TERRAIN_PATROL_SCENE_PLAYER_V1"
        );
        assert_eq!(
            crate::surface_player::capabilities(),
            "ORR_TERRAIN_PATROL_SURFACE_SCENE_PLAYER_V1"
        );
    } else {
        assert_eq!(
            crate::player::capabilities(),
            "ORR_NAVIGATION_PLAYER_V1\nORR_NAVIGATION_BUNDLE_V1"
        );
        assert_eq!(
            crate::surface_player::capabilities(),
            if cfg!(feature = "gpu") {
                "ORR_NAVIGATION_SURFACE_PLAYER_V2\nORR_NAVIGATION_SURFACE_CAPTURE_V2"
            } else {
                "ORR_NAVIGATION_SURFACE_PLAYER_V2"
            }
        );
    }
    let args = || {
        ["--bundle", "/missing-terrain-patrol-bundle", "--ticks", "0"]
            .into_iter()
            .map(std::ffi::OsString::from)
    };
    assert_eq!(crate::player::parse_command(args()).is_err(), enabled);
    assert_eq!(
        crate::surface_player::parse_command(args()).is_err(),
        enabled
    );
    let scene_args = || {
        ["--scene", "/missing-terrain-patrol-bundle", "--ticks", "0"]
            .into_iter()
            .map(std::ffi::OsString::from)
    };
    let plain = crate::player::parse(scene_args()).unwrap().unwrap();
    let surface = crate::surface_player::parse(scene_args()).unwrap().unwrap();
    assert_eq!(
        crate::player::run_bundle(plain)
            .unwrap_err()
            .contains("does not support terrain patrol profile"),
        enabled
    );
    assert_eq!(
        crate::surface_player::run_bundle(surface)
            .unwrap_err()
            .contains("does not support terrain patrol profile"),
        enabled
    );
}
