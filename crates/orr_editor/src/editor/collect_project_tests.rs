use super::*;

fn editor() -> Editor {
    let mut editor = Editor::open(&default_scene_path()).unwrap();
    editor.sync();
    editor
}

fn host_document(editor: &mut Editor) -> (J, J) {
    (
        editor.backend.erp.call("sim.state", J::Null).unwrap(),
        editor.backend.erp.call("history.list", J::Null).unwrap(),
    )
}

#[test]
fn clean_source_stamp_is_read_only_and_repeatable() {
    let mut editor = editor();
    let before = host_document(&mut editor);
    let stamp = editor.collect_project_source_stamp().unwrap();
    assert_eq!(stamp.path, default_scene_path());
    assert_eq!(stamp.checksum, editor.checksum());
    assert_eq!(stamp, editor.collect_project_source_stamp().unwrap());
    assert_eq!(host_document(&mut editor), before);
}

#[test]
fn collect_source_is_supported_without_mutation() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scenes/collect_dodge_v1.scene.yaml");
    let mut editor = Editor::open_game(&path, EditorGame::CollectDodge).unwrap();
    editor.sync();
    let before = host_document(&mut editor);
    assert_eq!(
        editor.collect_project_source_stamp().unwrap().game,
        EditorGame::CollectDodge
    );
    assert_eq!(host_document(&mut editor), before);
}

#[test]
fn stale_clean_caches_cannot_hide_dirty_host() {
    let mut editor = editor();
    editor.select_named("body_05");
    editor.sync();
    assert!(editor.set_field(
        &Owner::Component("orr_physics::Body".into()),
        "pos.x",
        Value::Fixed(FP::from_int(19)),
    ));
    editor.sim.dirty = false;
    editor.history.dirty = false;
    let before = host_document(&mut editor);
    assert!(editor.collect_project_source_stamp().is_err());
    assert_eq!(host_document(&mut editor), before);
}

#[test]
fn stale_caches_cannot_hide_transaction_or_proposal() {
    let mut editor = editor();
    editor
        .backend
        .erp
        .call("tx.begin", json!({"label":"Collect source guard"}))
        .unwrap();
    let before = host_document(&mut editor);
    assert!(editor.collect_project_source_stamp().is_err());
    assert_eq!(host_document(&mut editor), before);
    editor.backend.erp.call("tx.rollback", J::Null).unwrap();
    editor.sync();
    editor
        .backend
        .erp
        .call("proposal.begin", json!({"label":"Collect source guard"}))
        .unwrap();
    let before = editor.backend.erp.call("proposal.list", J::Null).unwrap();
    let document = host_document(&mut editor);
    assert!(editor.collect_project_source_stamp().is_err());
    assert_eq!(
        editor.backend.erp.call("proposal.list", J::Null).unwrap(),
        before
    );
    assert_eq!(host_document(&mut editor), document);
}

#[test]
fn stale_edit_cache_cannot_hide_play() {
    let mut editor = editor();
    editor.backend.erp.call("sim.start", J::Null).unwrap();
    let before = host_document(&mut editor);
    assert!(editor.collect_project_source_stamp().is_err());
    assert_eq!(host_document(&mut editor), before);
}

#[test]
fn pending_control_gesture_and_uncertain_edits_fail_closed() {
    let mut editor = editor();
    for phase in [
        input::Phase::Claiming,
        input::Phase::Active,
        input::Phase::Releasing,
    ] {
        editor.input.phase = phase;
        assert!(editor.collect_project_source_stamp().is_err());
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
        assert!(editor.collect_project_source_stamp().is_err());
        assert!(editor.pending.contains_key(&u64::MAX));
        editor.pending.remove(&u64::MAX);
    }
    editor.batch_uncertain = true;
    assert!(editor.collect_project_source_stamp().is_err());
    assert!(editor.batch_uncertain);
    editor.batch_uncertain = false;
    editor.begin_edit("Unfinished Collect source gesture");
    assert!(editor.in_gesture());
    assert!(editor.collect_project_source_stamp().is_err());
    assert!(editor.in_gesture());
}

#[test]
fn unsupported_remote_viewer_down_and_listener_sources_fail_closed() {
    let mut editor = editor();
    editor.backend.game = EditorGame::Yard3D;
    assert!(editor.collect_project_source_stamp().is_err());
    editor.backend.game = EditorGame::PhysGame;
    editor.sim.viewer = true;
    assert!(editor.collect_project_source_stamp().is_err());
    editor.sim.viewer = false;
    editor.down = Some(Down {
        reason: "guard fixture".into(),
        local: true,
    });
    assert!(editor.collect_project_source_stamp().is_err());
    editor.down = None;
    let spec = editor.backend.spec.clone();
    editor.backend.spec = HostSpec::remote("ws://127.0.0.1:1", None);
    assert!(editor.collect_project_source_stamp().is_err());
    editor.backend.spec = spec;
    // Exercise the editor's listener identity without requiring a listening
    // socket in the test environment; this guard never contacts that endpoint.
    editor.backend.url = Some("ws://127.0.0.1:7777".into());
    assert!(editor.erp_status().is_some());
    assert!(editor.collect_project_source_stamp().is_err());
}

#[test]
fn missing_or_stale_snapshot_and_path_fail_closed() {
    let mut editor = editor();
    let snapshot = editor.snapshot.take();
    assert!(editor.collect_project_source_stamp().is_err());
    editor.snapshot = snapshot;
    let checksum = editor.checksum;
    editor.checksum ^= 1;
    assert!(editor.collect_project_source_stamp().is_err());
    editor.checksum = checksum;
    let path = editor.sim.scene_path.take();
    assert!(editor.collect_project_source_stamp().is_err());
    editor.sim.scene_path = path;
    assert!(editor.collect_project_source_stamp().is_ok());
}

#[test]
fn restarted_host_invalidates_stamp_even_with_same_path_and_checksum() {
    let mut editor = editor();
    let before = editor.collect_project_source_stamp().unwrap();
    assert!(editor.restart());
    editor.sync();
    let after = editor.collect_project_source_stamp().unwrap();
    assert_eq!(before.path, after.path);
    assert_eq!(before.checksum, after.checksum);
    assert_ne!(before, after);
}

#[test]
fn edit_then_undo_invalidates_stamp_without_losing_history() {
    let mut editor = editor();
    let before = editor.collect_project_source_stamp().unwrap();
    editor.select_named("body_05");
    editor.sync();
    assert!(editor.set_field(
        &Owner::Component("orr_physics::Body".into()),
        "pos.x",
        Value::Fixed(FP::from_int(19)),
    ));
    assert!(editor.undo());
    editor.sync();
    let history = host_document(&mut editor);
    let after = editor.collect_project_source_stamp().unwrap();
    assert_eq!(before.checksum, after.checksum);
    assert_ne!(before, after);
    assert_eq!(host_document(&mut editor), history);
}

#[test]
fn malformed_authoritative_answers_fail_closed() {
    let mut editor = editor();
    let (state, history) = host_document(&mut editor);
    assert!(clean_state(&state).is_ok());
    assert!(clean_history(&history).is_ok());
    for field in [
        "mode",
        "playing",
        "dirty",
        "in_tx",
        "head_tick",
        "epoch",
        "scene_path",
        "checksum",
        "doc_checksum",
    ] {
        let mut malformed = state.clone();
        malformed.as_object_mut().unwrap().remove(field);
        assert!(clean_state(&malformed).is_err(), "missing {field}");
    }
    for field in ["dirty", "in_tx", "can_undo", "can_redo", "entries"] {
        let mut malformed = history.clone();
        malformed.as_object_mut().unwrap().remove(field);
        assert!(clean_history(&malformed).is_err(), "missing {field}");
    }
}

#[test]
fn changed_or_missing_saved_scene_blocks_even_when_host_is_clean() {
    let directory = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    let path = directory.path().join("scene.yaml");
    fs::copy(default_scene_path(), &path).unwrap();
    let mut editor = Editor::open(&path).unwrap();
    editor.sync();
    assert!(editor.collect_project_source_stamp().is_ok());
    let before = host_document(&mut editor);
    let text = fs::read_to_string(&path).unwrap();
    // Names are document-only, so a Frame checksum alone cannot detect this.
    let changed = text.replace("body_05", "external_rename");
    assert_ne!(changed, text);
    fs::write(&path, changed).unwrap();
    assert!(editor.collect_project_source_stamp().is_err());
    assert_eq!(host_document(&mut editor), before);
    fs::remove_file(path).unwrap();
    assert!(editor.collect_project_source_stamp().is_err());
    assert_eq!(host_document(&mut editor), before);
}

fn saved_ui() -> (tempfile::TempDir, PathBuf, Document) {
    let directory = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    let path = directory.path().join("ui.json");
    let document = Document::default_collect();
    fs::write(&path, document.to_bytes().unwrap()).unwrap();
    (directory, path, document)
}

#[test]
fn saved_ui_comparison_is_complete_and_read_only() {
    let (_directory, path, document) = saved_ui();
    let bytes = fs::read(&path).unwrap();
    assert!(saved_ui_matches(&path, &document).is_ok());
    let mut changed = document.clone();
    changed.nodes[0].offset[0] += 1;
    assert!(saved_ui_matches(&path, &changed).is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    fs::write(&path, changed.to_bytes().unwrap()).unwrap();
    assert!(saved_ui_matches(&path, &document).is_err());
    assert!(saved_ui_matches(&path, &changed).is_ok());
}

#[test]
fn unsafe_missing_malformed_oversized_and_directory_ui_fail_closed() {
    let (directory, path, document) = saved_ui();
    assert!(saved_ui_matches(Path::new("ui.json"), &document).is_err());
    assert!(saved_ui_matches(&directory.path().join("missing.json"), &document).is_err());
    assert!(saved_ui_matches(directory.path(), &document).is_err());
    fs::write(&path, b"not a document").unwrap();
    assert!(saved_ui_matches(&path, &document).is_err());
    fs::write(&path, vec![b' '; MAX_BYTES + 1]).unwrap();
    assert!(saved_ui_matches(&path, &document).is_err());
}

#[cfg(unix)]
#[test]
fn symlink_leaf_ancestor_and_special_ui_fail_closed() {
    use std::os::unix::fs::symlink;
    let (directory, path, document) = saved_ui();
    let alias = directory.path().join("alias.json");
    symlink(&path, &alias).unwrap();
    assert!(saved_ui_matches(&alias, &document).is_err());
    let parent_alias = directory.path().join("alias-directory");
    symlink(directory.path(), &parent_alias).unwrap();
    assert!(saved_ui_matches(&parent_alias.join("ui.json"), &document).is_err());
    let fifo = directory.path().join("special.json");
    assert!(std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap()
        .success());
    assert!(saved_ui_matches(&fifo, &document).is_err());
}
