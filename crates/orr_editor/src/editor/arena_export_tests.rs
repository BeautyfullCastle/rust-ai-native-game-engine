use super::*;
fn fixture() -> (tempfile::TempDir, Editor, crate::sprite_panel::SpritePanel) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("arena.scene.yaml"),
        include_bytes!("../../../../scenes/arena_blank.scene.yaml"),
    )
    .unwrap();
    fs::write(
        dir.path().join("orr.project.json"),
        br#"{"schema":2,"engine":"*","entry":{"game":"arena","scene":"arena.scene.yaml"}}"#,
    )
    .unwrap();
    let mut editor =
        Editor::open_game(&dir.path().join("arena.scene.yaml"), EditorGame::Arena).unwrap();
    editor.sync();
    (dir, editor, Default::default())
}
#[test]
fn saved_source_inventory_and_equal_contents_reload_epoch() {
    let (dir, mut editor, sprites) = fixture();
    let receipt = editor.arena_export_source(dir.path(), &sprites).unwrap();
    assert_eq!(receipt.inventory().len(), 3);
    assert_eq!(receipt.initial_checksum(), editor.checksum());
    let epoch = editor.scene_load_epoch();
    assert!(editor.open_path(&dir.path().join("arena.scene.yaml")));
    editor.sync();
    assert_ne!(epoch, editor.scene_load_epoch());
    assert_eq!(receipt.initial_checksum(), editor.checksum());
    let epoch = editor.scene_load_epoch();
    let text = fs::read_to_string(dir.path().join("arena.scene.yaml")).unwrap();
    editor
        .host_call(
            "scene.load",
            json!({"text":text,"path":dir.path().join("arena.scene.yaml").display().to_string()}),
        )
        .unwrap();
    assert_ne!(epoch, editor.scene_load_epoch());
}
#[test]
fn dirty_gesture_control_uncertain_and_transaction_refused() {
    let (dir, mut editor, sprites) = fixture();
    editor.batch_uncertain = true;
    assert!(editor.arena_export_source(dir.path(), &sprites).is_err());
    editor.batch_uncertain = false;
    editor.history.in_tx = true;
    assert!(editor.arena_export_source(dir.path(), &sprites).is_err());
    editor.history.in_tx = false;
    editor.sim.in_tx = true;
    assert!(editor.arena_export_source(dir.path(), &sprites).is_err());
    editor.sim.in_tx = false;
    editor.sim.dirty = true;
    assert!(editor.arena_export_source(dir.path(), &sprites).is_err());
}
#[test]
fn declared_ui_tilemap_unloaded_bindings_and_unrelated_loaded_bindings_refused() {
    for field in ["ui", "tilemap", "sprites"] {
        let (dir, mut editor, sprites) = fixture();
        let path = dir.path().join("orr.project.json");
        let mut manifest: J = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest["entry"][field] = if field == "ui" {
            json!({"document":"ui.json"})
        } else {
            json!("view.json")
        };
        fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(
            editor.arena_export_source(dir.path(), &sprites).is_err(),
            "{field}"
        );
    }
    let (dir, mut editor, mut sprites) = fixture();
    sprites.bindings = Some(
        crate::sprite_bindings::Bindings::create(
            dir.path().join("other.json"),
            "arena.scene.yaml".into(),
            ".".into(),
        )
        .unwrap(),
    );
    assert!(editor
        .arena_export_source(dir.path(), &sprites)
        .unwrap_err()
        .contains("Declared and loaded"));
}
#[test]
fn raw_scene_changes_are_pinned_and_semantic_changes_refused() {
    let (dir, mut editor, sprites) = fixture();
    let first = editor.arena_export_source(dir.path(), &sprites).unwrap();
    let scene = dir.path().join("arena.scene.yaml");
    let mut text = fs::read_to_string(&scene).unwrap();
    text.push('\n');
    fs::write(&scene, &text).unwrap();
    let second = editor.arena_export_source(dir.path(), &sprites).unwrap();
    assert_eq!(first.initial_checksum(), second.initial_checksum());
    assert_ne!(first, second);
    fs::write(scene, "schema: orr.scene/1\nentities: {}\nsingletons: {}\n").unwrap();
    assert!(editor.arena_export_source(dir.path(), &sprites).is_err());
}
#[test]
fn bounded_reader_refuses_symlinks_and_fifo_without_opening_them() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("original");
    fs::write(&original, "ok").unwrap();
    let link = dir.path().join("link");
    symlink(&original, &link).unwrap();
    assert!(read_saved_file(&link, 10).is_err());
    assert!(read_saved_file(&original, 1).is_err());
    let linked_dir = dir.path().join("dirlink");
    symlink(dir.path(), &linked_dir).unwrap();
    assert!(read_saved_file(&linked_dir.join("original"), 10).is_err());
}
#[test]
fn referenced_cached_pixels_documents_and_missing_cache_are_checked() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/saved_arena_project")
        .canonicalize()
        .unwrap();
    let prepared = orr_sample::project::PreparedProject::open(
        &root,
        crate::sprite_bindings::compiled_runtime(),
    )
    .unwrap();
    let admitted = prepared.sprites().unwrap();
    let mut editor = Editor::open_game(prepared.scene().path(), EditorGame::Arena).unwrap();
    editor.sync();
    let mut panel = crate::sprite_panel::SpritePanel::default();
    panel.bindings = Some(crate::sprite_bindings::Bindings::from_document(
        admitted.path.clone(),
        admitted.document.clone(),
    ));
    assert!(editor
        .arena_export_source(&root, &panel)
        .unwrap_err()
        .contains("absent from the editor cache"));
    let ctx = egui::Context::default();
    let mut assets = admitted.assets.clone();
    assets.values_mut().next().unwrap().rgba[0] ^= 0xff;
    panel.install_prepared(
        &ctx,
        crate::sprite_bindings::Bindings::from_document(
            admitted.path.clone(),
            admitted.document.clone(),
        ),
        assets,
    );
    assert!(editor
        .arena_export_source(&root, &panel)
        .unwrap_err()
        .contains("pixels differ"));
    panel.install_prepared(
        &ctx,
        crate::sprite_bindings::Bindings::from_document(
            admitted.path.clone(),
            admitted.document.clone(),
        ),
        admitted.assets.clone(),
    );
    editor.arena_export_source(&root, &panel).unwrap();
}
#[test]
fn stale_cached_clean_state_does_not_hide_host_transaction_proposal_or_play() {
    for operation in ["tx.begin", "proposal.begin", "sim.start"] {
        let (dir, mut editor, sprites) = fixture();
        editor
            .backend
            .erp
            .call(operation, json!({"label":"source guard"}))
            .unwrap();
        let document = editor
            .backend
            .erp
            .call("scene.save", json!({"write":false,"source":"doc"}))
            .unwrap();
        assert!(
            editor.arena_export_source(dir.path(), &sprites).is_err(),
            "{operation}"
        );
        assert_eq!(
            editor
                .backend
                .erp
                .call("scene.save", json!({"write":false,"source":"doc"}))
                .unwrap(),
            document
        );
    }
}
#[test]
fn actual_managed_control_pending_gesture_and_listener_are_preserved_and_refused() {
    let (dir, mut editor, sprites) = fixture();
    for phase in [
        input::Phase::Claiming,
        input::Phase::Active,
        input::Phase::Releasing,
    ] {
        editor.input.phase = phase;
        assert!(editor.arena_export_source(dir.path(), &sprites).is_err());
        assert_eq!(editor.input.phase, phase);
    }
    editor.input.phase = input::Phase::Off;
    for kind in [
        Pend::Input(0, input::Op::Release),
        Pend::Gesture(0, gestures::Reply::Begin),
    ] {
        editor.pending.insert(
            u64::MAX,
            Pending {
                kind,
                posted_at: Instant::now(),
            },
        );
        assert!(editor.arena_export_source(dir.path(), &sprites).is_err());
        assert!(editor.pending.contains_key(&u64::MAX));
        editor.pending.remove(&u64::MAX);
    }
    editor.backend.url = Some("ws://127.0.0.1:7777".into());
    assert!(editor.arena_export_source(dir.path(), &sprites).is_err());
    assert!(editor.erp_status().is_some());
    editor.backend.url = None;
    editor.begin_edit("unfinished source gesture");
    assert!(editor.in_gesture());
    assert!(editor.arena_export_source(dir.path(), &sprites).is_err());
    assert!(editor.in_gesture());
}
#[test]
fn missing_frame_stale_checksum_and_clean_saved_unloaded_sprites_fail_closed() {
    let (dir, mut editor, sprites) = fixture();
    let snapshot = editor.snapshot.take();
    assert!(editor.arena_export_source(dir.path(), &sprites).is_err());
    editor.snapshot = snapshot;
    editor.checksum ^= 1;
    assert!(editor.arena_export_source(dir.path(), &sprites).is_err());
    editor.checksum ^= 1;
    fs::write(
        dir.path().join("view.json"),
        br#"{"version":2,"scene":"arena.scene.yaml","project":".","bindings":{}}"#,
    )
    .unwrap();
    fs::write(dir.path().join("orr.project.json"),br#"{"schema":2,"engine":"*","entry":{"game":"arena","scene":"arena.scene.yaml","sprites":"view.json"}}"#).unwrap();
    assert!(editor
        .arena_export_source(dir.path(), &sprites)
        .unwrap_err()
        .contains("Declared and loaded"));
}
