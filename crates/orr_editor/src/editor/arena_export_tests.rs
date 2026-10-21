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
    let receipt = editor
        .arena_export_source(dir.path(), &sprites, None)
        .unwrap();
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
fn cached_dirty_uncertain_and_transaction_flags_refused() {
    let (dir, mut editor, sprites) = fixture();
    editor.batch_uncertain = true;
    assert!(editor
        .arena_export_source(dir.path(), &sprites, None)
        .is_err());
    editor.batch_uncertain = false;
    editor.history.in_tx = true;
    assert!(editor
        .arena_export_source(dir.path(), &sprites, None)
        .is_err());
    editor.history.in_tx = false;
    editor.sim.in_tx = true;
    assert!(editor
        .arena_export_source(dir.path(), &sprites, None)
        .is_err());
    editor.sim.in_tx = false;
    editor.sim.dirty = true;
    assert!(editor
        .arena_export_source(dir.path(), &sprites, None)
        .is_err());
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
            editor
                .arena_export_source(dir.path(), &sprites, None)
                .is_err(),
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
        .arena_export_source(dir.path(), &sprites, None)
        .unwrap_err()
        .contains("Declared and loaded"));
}
#[test]
fn raw_scene_changes_are_pinned_and_semantic_changes_refused() {
    let (dir, mut editor, sprites) = fixture();
    let first = editor
        .arena_export_source(dir.path(), &sprites, None)
        .unwrap();
    let scene = dir.path().join("arena.scene.yaml");
    let mut text = fs::read_to_string(&scene).unwrap();
    text.push('\n');
    fs::write(&scene, &text).unwrap();
    let second = editor
        .arena_export_source(dir.path(), &sprites, None)
        .unwrap();
    assert_eq!(first.initial_checksum(), second.initial_checksum());
    assert_ne!(first, second);
    fs::write(scene, "schema: orr.scene/1\nentities: {}\nsingletons: {}\n").unwrap();
    assert!(editor
        .arena_export_source(dir.path(), &sprites, None)
        .is_err());
}
#[test]
fn bounded_reader_refuses_symlinks_and_over_limit_files() {
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
        .arena_export_source(&root, &panel, None)
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
        .arena_export_source(&root, &panel, None)
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
    editor.arena_export_source(&root, &panel, None).unwrap();
    let mut assets = admitted.assets.clone();
    let asset = assets.values_mut().next().unwrap();
    let mut doc: J = serde_json::from_str(&asset.document.to_json().unwrap()).unwrap();
    doc["atlas"]["image"] = json!("different-cached-image.png");
    asset.document =
        orr_sprite::SpriteDocument::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
    panel.install_prepared(
        &ctx,
        crate::sprite_bindings::Bindings::from_document(
            admitted.path.clone(),
            admitted.document.clone(),
        ),
        assets,
    );
    assert!(editor
        .arena_export_source(&root, &panel, None)
        .unwrap_err()
        .contains("document or pixels differ"));
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
            editor
                .arena_export_source(dir.path(), &sprites, None)
                .is_err(),
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
        assert!(editor
            .arena_export_source(dir.path(), &sprites, None)
            .is_err());
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
        assert!(editor
            .arena_export_source(dir.path(), &sprites, None)
            .is_err());
        assert!(editor.pending.contains_key(&u64::MAX));
        editor.pending.remove(&u64::MAX);
    }
    editor.backend.url = Some("ws://127.0.0.1:7777".into());
    assert!(editor
        .arena_export_source(dir.path(), &sprites, None)
        .is_err());
    assert!(editor.erp_status().is_some());
    editor.backend.url = None;
    editor.begin_edit("unfinished source gesture");
    assert!(editor.in_gesture());
    assert!(editor
        .arena_export_source(dir.path(), &sprites, None)
        .is_err());
    assert!(editor.in_gesture());
}
#[test]
fn missing_frame_stale_checksum_and_clean_saved_unloaded_sprites_fail_closed() {
    let (dir, mut editor, sprites) = fixture();
    let snapshot = editor.snapshot.take();
    assert!(editor
        .arena_export_source(dir.path(), &sprites, None)
        .is_err());
    editor.snapshot = snapshot;
    editor.checksum ^= 1;
    assert!(editor
        .arena_export_source(dir.path(), &sprites, None)
        .is_err());
    editor.checksum ^= 1;
    fs::write(
        dir.path().join("view.json"),
        br#"{"version":2,"scene":"arena.scene.yaml","project":".","bindings":{}}"#,
    )
    .unwrap();
    fs::write(dir.path().join("orr.project.json"),br#"{"schema":2,"engine":"*","entry":{"game":"arena","scene":"arena.scene.yaml","sprites":"view.json"}}"#).unwrap();
    assert!(editor
        .arena_export_source(dir.path(), &sprites, None)
        .unwrap_err()
        .contains("Declared and loaded"));
}

#[cfg(feature = "project-ui")]
#[test]
fn saved_ui_requires_exact_loaded_font_and_retains_whole_lock_verification() {
    let (dir, mut editor, sprites) = fixture();
    let package = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/game_ui_font")
        .canonicalize()
        .unwrap();
    let installer = orr_package::Project::open_for_install(
        dir.path(),
        orr_package::Runtime::content_only().engine_version,
    )
    .unwrap();
    installer.install(&[package]).unwrap();
    let manifest = dir.path().join("orr.project.json");
    let mut value: J = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["entry"]["ui"] = json!({"profile":"arena-korean-v1","font":{"package":"korean-game-ui","asset":"OrreryKoreanUI.otf"}});
    fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    let prepared = crate::project::PreparedProject::open(dir.path()).unwrap();
    let ui = prepared.ui().unwrap().clone();
    let ctx = egui::Context::default();
    let original = crate::project_ui::Preview::install(ui.clone(), &ctx);
    let source = editor
        .arena_export_source(
            dir.path(),
            &sprites,
            Some(original.arena_export_identity().unwrap()),
        )
        .unwrap();
    assert_eq!(source.inventory().len(), 3);
    assert!(source.ui().is_some());
    assert!(editor
        .arena_export_source(dir.path(), &sprites, None)
        .is_err());
    // The sfnt head checkSumAdjustment field is metadata. Mutating it preserves
    // length, parser validity and every corpus glyph, but not installation bytes.
    let mut changed = ui.clone();
    let count = u16::from_be_bytes(changed.font[4..6].try_into().unwrap()) as usize;
    let head = (0..count)
        .map(|i| 12 + 16 * i)
        .find(|&i| &changed.font[i..i + 4] == b"head")
        .unwrap();
    let offset = u32::from_be_bytes(changed.font[head + 8..head + 12].try_into().unwrap()) as usize;
    changed.font[offset + 8] ^= 1;
    assert_eq!(changed.font.len(), ui.font.len());
    assert_ne!(changed.font, ui.font);
    orr_sample::game_ui::GameUi::validate_font(&changed.font).unwrap();
    let stale = crate::project_ui::Preview::install(changed.clone(), &ctx);
    assert!(editor
        .arena_export_source(
            dir.path(),
            &sprites,
            Some(stale.arena_export_identity().unwrap())
        )
        .unwrap_err()
        .contains("Saved UI differs"));
    let restored = crate::project_ui::Preview::install(ui, &ctx);
    editor
        .arena_export_source(
            dir.path(),
            &sprites,
            Some(restored.arena_export_identity().unwrap()),
        )
        .unwrap();
    // Reinstall a valid changed package/lock without refreshing the loaded preview.
    let changed_package = tempfile::tempdir().unwrap();
    for entry in
        fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/game_ui_font"))
            .unwrap()
    {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            fs::copy(entry.path(), changed_package.path().join(entry.file_name())).unwrap();
        }
    }
    fs::write(
        changed_package.path().join("OrreryKoreanUI.otf"),
        &changed.font,
    )
    .unwrap();
    let package_manifest = changed_package.path().join("orr.package.json");
    let mut metadata: J = serde_json::from_slice(&fs::read(&package_manifest).unwrap()).unwrap();
    metadata["version"] = json!("1.0.1");
    fs::write(package_manifest, serde_json::to_vec(&metadata).unwrap()).unwrap();
    // The fixture changed entry.ui after the first install; open a fresh
    // package operation without weakening manifest freshness validation.
    let installer = orr_package::Project::open_for_install(
        dir.path(),
        orr_package::Runtime::content_only().engine_version,
    )
    .unwrap();
    installer
        .install(&[changed_package.path().to_path_buf()])
        .unwrap();
    assert!(editor
        .arena_export_source(
            dir.path(),
            &sprites,
            Some(restored.arena_export_identity().unwrap())
        )
        .unwrap_err()
        .contains("Saved UI differs"));
    let refreshed = crate::project_ui::Preview::install(
        crate::project::PreparedProject::open(dir.path())
            .unwrap()
            .ui()
            .unwrap()
            .clone(),
        &ctx,
    );
    editor
        .arena_export_source(
            dir.path(),
            &sprites,
            Some(refreshed.arena_export_identity().unwrap()),
        )
        .unwrap();
    // A separate active, unreferenced package must still be verified in full.
    installer
        .install(&[Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/sprite_demo")
            .canonicalize()
            .unwrap()])
        .unwrap();
    // Unreferenced package license still belongs to the whole active closure.
    let project =
        orr_package::Project::open(dir.path(), crate::sprite_bindings::compiled_runtime()).unwrap();
    let lock = project.verify().unwrap();
    let license = dir.path().join(format!(
        ".orr/packages/objects/{}/orr.package.json",
        lock.packages["sample-sprites"].digest
    ));
    fs::write(license, b"corrupt unrelated declared asset").unwrap();
    assert!(editor
        .arena_export_source(
            dir.path(),
            &sprites,
            Some(refreshed.arena_export_identity().unwrap())
        )
        .is_err());
}
