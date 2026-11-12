use super::*;
use std::fs;
fn wait(editor: &mut Editor, tick: Option<u64>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        editor.sync();
        let ready = editor
            .snapshot()
            .is_some_and(|s| tick.is_none_or(|t| s.predicted().tick() == t));
        if ready && editor.can_mutate() && editor.yard_rows_coherent() && tick.is_none() {
            return;
        }
        if ready
            && tick.is_some()
            && editor.mode() == Mode::Play
            && editor.timeline().is_some_and(|t| !t.playing)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Editor did not reach expected state: tick={tick:?}, mode={:?}, log={:?}",
            editor.mode(),
            editor.log()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn author(root: &Path) -> (Editor, NavigationPanel, TerrainPanel) {
    let scene = root.join("navigation.scene.yaml");
    fs::write(
        &scene,
        include_str!("../../../../scenes/navigation_blank.scene.yaml"),
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::NavigationYard3D).unwrap();
    wait(&mut editor, None);
    let mut terrain = TerrainPanel::default();
    terrain.local_path = "authored.orrt".into();
    terrain.asset_id = "navigation/editor-parity".into();
    terrain.dimensions = [9, 9];
    terrain.origin = ["-4".into(), "-4".into()];
    terrain.spacing = "1".into();
    terrain.initial_height = "0".into();
    terrain.create_for_editor(&editor).unwrap();
    assert!(terrain.document.dirty());
    terrain.save_for_editor(&editor).unwrap();
    assert!(!terrain.document.dirty());
    assert_eq!(
        fs::read(root.join("authored.orrt")).unwrap(),
        terrain.document.bytes().unwrap()
    );
    let mut navigation = NavigationPanel::default();
    navigation.start = ["-3".into(), "0".into()];
    navigation.goal = ["3".into(), "0".into()];
    navigation.distance_per_tick = "0.25".into();
    navigation.build_route(&mut editor, &terrain).unwrap();
    editor.sync();
    assert!(
        editor.save(),
        "actual ERP scene.save must write the authored pin"
    );
    let saved = fs::read_to_string(&scene).unwrap();
    assert!(saved.contains("terrain_revision:"));
    assert!(saved.contains("graph_revision:"));
    (editor, navigation, terrain)
}

#[test]
fn saved_route_semantic_scene_and_frame_admission_does_not_write() {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, n, t) = author(dir.path());
    let path = e.path().unwrap();
    let original = fs::read(&path).unwrap();
    let mut reformatted = b"\n".to_vec();
    reformatted.extend(&original);
    reformatted.extend(b"\n");
    fs::write(&path, &reformatted).unwrap();
    let history = e.history().clone();
    let clients_before: Vec<_> = e.agent_clients.keys().cloned().collect();
    let source = e.navigation_export_source(&n, &t).unwrap();
    assert_eq!(
        e.agent_clients.keys().cloned().collect::<Vec<_>>(),
        clients_before
    );
    assert!(!e.agent_clients.contains_key("navigation-export-admission"));
    assert_eq!(source.initial_checksum(), e.checksum());
    assert_eq!(source.scene_bytes(), reformatted);
    assert_eq!(fs::read(&path).unwrap(), reformatted);
    assert_eq!(e.history(), &history);
    // Comments are retained document metadata, not disposable formatting.
    let mut changed = b"# new document metadata\n".to_vec();
    changed.extend(&original);
    fs::write(&path, changed).unwrap();
    assert!(e.navigation_export_source(&n, &t).is_err());
}
#[test]
fn missing_snapshot_blank_dirty_transactions_and_play_refuse() {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, n, t) = author(dir.path());
    let snapshot = e.snapshot.take();
    assert!(e.navigation_export_source(&n, &t).is_err());
    e.snapshot = snapshot;
    e.sim.dirty = true;
    assert!(e.navigation_export_source(&n, &t).is_err());
    e.sim.dirty = false;
    e.history.in_tx = true;
    assert!(e.navigation_export_source(&n, &t).is_err());
    e.history.in_tx = false;
    e.sim.in_tx = true;
    assert!(e.navigation_export_source(&n, &t).is_err());
    e.sim.in_tx = false;
    e.batch_uncertain = true;
    assert!(e.navigation_export_source(&n, &t).is_err());
    e.batch_uncertain = false;
    e.scene_edit_allowed = false;
    assert!(e.navigation_export_source(&n, &t).is_err());
    e.scene_edit_allowed = true;
    e.step(0);
    wait(&mut e, Some(0));
    assert!(e.navigation_export_source(&n, &t).is_err());
    let blank = dir.path().join("blank.yaml");
    fs::write(
        &blank,
        include_str!("../../../../scenes/navigation_blank.scene.yaml"),
    )
    .unwrap();
    let mut blank = Editor::open_game(&blank, EditorGame::NavigationYard3D).unwrap();
    wait(&mut blank, None);
    assert!(blank
        .navigation_export_source(&NavigationPanel::default(), &TerrainPanel::default())
        .is_err());
}
#[test]
fn draft_dirty_and_saved_terrain_mismatch_survives_panel_close() {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, mut n, mut t) = author(dir.path());
    n.goal[0] = "2".into();
    assert!(e.navigation_export_source(&n, &t).is_err());
    n.goal[0] = "3".into();
    t.selection_mode = crate::terrain_pick::TerrainSelectionMode::Vertex;
    t.selected_vertex = [1, 1];
    t.height = "0.125".into();
    t.apply_height(&e).unwrap();
    assert!(e.navigation_export_source(&n, &t).is_err());
    t.save_for_editor(&e).unwrap();
    n.observe_terrain(&e, &t);
    t.close_for_editor(&e).unwrap();
    assert!(n.route_stale(&e, &t));
    assert!(e.navigation_export_source(&n, &t).is_err());
}
#[test]
fn generation_and_external_source_changes_during_capture_refuse() {
    for case in 0..4 {
        // Each capture starts from independent, admissible host state. In particular
        // an epoch injection cannot make a subsequent case refuse before its hook.
        let dir = tempfile::tempdir().unwrap();
        let (mut e, n, t) = author(dir.path());
        let path = e.path().unwrap();
        let mut reached = false;
        let result = e.navigation_export_source_with(&n, &t, |e| {
            reached = true;
            match case {
                0 => e.change_gen += 1,
                1 => e.state_generation += 1,
                2 => e.scene_load_epoch += 1,
                3 => {
                    let mut bytes = fs::read(&path).unwrap();
                    bytes.extend(b"\n# external change\n");
                    fs::write(&path, bytes).unwrap();
                }
                _ => unreachable!(),
            }
        });
        assert!(
            reached,
            "case {case} must reach the injected change before refusing"
        );
        assert!(result.is_err(), "case {case} must refuse the actual change");
        assert!(!e.agent_clients.contains_key("navigation-export-admission"));
    }
}

#[test]
fn changed_saved_document_and_missing_terrain_refuse() {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, n, t) = author(dir.path());
    let path = e.path().unwrap();
    let original = fs::read(&path).unwrap();
    let text = String::from_utf8(original.clone())
        .unwrap()
        .replace("distance_per_tick: 0.25", "distance_per_tick: 0.5");
    assert_ne!(text.as_bytes(), original);
    fs::write(&path, text).unwrap();
    assert!(e.navigation_export_source(&n, &t).is_err());
    fs::write(&path, original).unwrap();
    fs::remove_file(dir.path().join("authored.orrt")).unwrap();
    assert!(e.navigation_export_source(&n, &t).is_err());
}

#[test]
fn active_gesture_and_erp_listener_are_not_cancelled_by_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, n, t) = author(dir.path());
    e.begin_edit("unfinished navigation edit");
    assert!(e.in_gesture());
    assert!(e.navigation_export_source(&n, &t).is_err());
    assert!(e.in_gesture());
    drop(e);
    let spec = HostSpec::LocalGame {
        scene: dir.path().join("navigation.scene.yaml"),
        game: EditorGame::NavigationYard3D,
        listen: Some(orr_remote::ServerConfig::new(orr_remote::Auth::DevNoAuth)),
        debug_hooks: false,
    };
    let mut e = Editor::start(&spec).unwrap();
    wait(&mut e, None);
    let before = e.erp_status().expect("real listener");
    assert!(e.navigation_export_source(&n, &t).is_err());
    assert_eq!(e.erp_status(), Some(before));
}
