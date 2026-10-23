use super::*;

#[test]
fn hazard_starters_have_canonical_mission_floor_and_optional_sphere() {
    for include in [false, true] {
        let yaml = scene_yaml(&Settings::default(), include).unwrap();
        let doc = hazards::hazards_doc_from_yaml(&yaml).unwrap();
        let frame = FrameView::of(doc.frame());
        let mesh = mesh_from_view(frame).unwrap();
        assert!(!mesh.pin.is_empty());
        assert!(mission::from_view(frame).unwrap().is_some());
        assert_eq!(hazards::from_view(frame).unwrap().is_some(), include);
        assert_eq!(
            *frame.singleton::<hazards::MeshHazardRun>(),
            Default::default()
        );
        if !include {
            assert_eq!(
                *frame.singleton::<hazards::MeshHazards>(),
                Default::default()
            );
        }
        assert!(orr_remote::mesh_navigation::mesh_doc_from_yaml(&yaml).is_err());
    }
}

#[test]
fn invalid_settings_and_existing_paths_never_write() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let path = root.join("hazard.scene.yaml");
    assert!(prepare(
        &path,
        &Settings {
            width: 0,
            ..Settings::default()
        },
        true
    )
    .is_err());
    assert!(!path.exists());
    std::fs::write(&path, b"keep me").unwrap();
    assert!(prepare(&path, &Settings::default(), false).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"keep me");
}

#[test]
fn orphan_sidecars_and_relative_paths_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let path = root.join("hazard.scene.yaml");
    std::fs::write(root.join("hazard.scene.yaml.mesh-hud.json"), b"keep me").unwrap();
    assert!(prepare(&path, &Settings::default(), false).is_err());
    assert!(!path.exists());
    assert!(prepare(Path::new("hazard.scene.yaml"), &Settings::default(), false).is_err());
}

#[test]
fn creation_opens_explicit_hazard_profile_and_repeat_never_overwrites() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp
        .path()
        .canonicalize()
        .unwrap()
        .join("hazard.scene.yaml");
    let editor = prepare(&path, &Settings::default(), true).unwrap();
    assert_eq!(editor.game(), EditorGame::MeshHazardsV1);
    assert!(!editor.is_dirty());
    let before = std::fs::read(&path).unwrap();
    assert!(prepare(&path, &Settings::default(), false).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn optional_starter_sphere_leaves_the_collect_and_exit_route_winnable() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    for (index, (width, depth, count)) in
        [(6, 4, 8), (8, 6, 2), (64, 64, 1)].into_iter().enumerate()
    {
        let settings = Settings {
            width,
            depth,
            collectible_count: count,
            distance_per_tick: "0.5".into(),
        };
        let mut editor = prepare(
            &root.join(format!("safe-{index}.scene.yaml")),
            &settings,
            true,
        )
        .unwrap();
        assert!(editor.start_play());
        editor.step(160);
        editor.sync();
        let frame = editor.snapshot().unwrap().predicted();
        let mission = mission::from_view(frame).unwrap().unwrap();
        assert_eq!(mission.run.collected, (1 << count) - 1);
        assert_eq!(mission.run.won, 1);
        assert_eq!(frame.singleton::<hazards::MeshHazardRun>().lost, 0);
    }
}
