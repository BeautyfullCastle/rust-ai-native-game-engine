use super::*;
use orr_input::{Key, MAX_FILE_BYTES};
use orr_sample::collect_app::input_actions::CollectAction;
use std::fs;

fn key(key: Key) -> Button {
    Button::Keyboard { key }
}

fn select(panel: &mut Panel, name: &str) {
    let index = panel
        .document()
        .actions
        .iter()
        .position(|action| action.name == name)
        .unwrap();
    panel.select_action(index).unwrap();
}

fn set(panel: &mut Panel, name: &str, keys: &[Key]) {
    select(panel, name);
    panel
        .replace_draft(keys.iter().copied().map(key).collect())
        .unwrap();
    panel.apply_draft().unwrap();
}

fn encoded(map: &ActionMap) -> Vec<u8> {
    let mut bytes = Vec::new();
    map.save(&mut bytes).unwrap();
    bytes
}

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    document: ActionMap,
    undo: Vec<ActionMap>,
    redo: Vec<ActionMap>,
    selected: usize,
    draft: Vec<Button>,
    path: String,
    message: String,
}

fn snapshot(panel: &Panel) -> Snapshot {
    Snapshot {
        document: panel.document.clone(),
        undo: panel.undo.clone(),
        redo: panel.redo.clone(),
        selected: panel.selected,
        draft: panel.draft.clone(),
        path: panel.path.clone(),
        message: panel.message.clone(),
    }
}

#[test]
fn initial_defaults_are_clean_exact_collect_contract() {
    let mut panel = Panel::default();
    assert_eq!(panel.document(), &default_map());
    assert_eq!(panel.document().actions.len(), 6);
    assert_eq!(
        CollectBindings::from_map(panel.document()).unwrap(),
        CollectBindings::default()
    );
    assert_eq!(panel.selected_action().name, "left");
    assert_eq!(panel.selected_action_index(), 0);
    assert_eq!(panel.draft(), panel.selected_action().bindings);
    assert!(!panel.has_pending_draft());
    assert!(!panel.can_undo());
    assert!(!panel.can_redo());
    assert!(!panel.undo().unwrap());
    assert!(!panel.redo().unwrap());
    panel.set_message("ready");
    assert_eq!(panel.message(), "ready");
}

#[test]
fn each_action_applies_one_complete_undoable_map() {
    let mut panel = Panel::default();
    let mut states = vec![panel.document().clone()];
    for (name, binding) in [
        ("left", Key::I),
        ("right", Key::L),
        ("up", Key::U),
        ("down", Key::K),
        ("restart", Key::R),
        ("quit", Key::Q),
    ] {
        select(&mut panel, name);
        panel.replace_draft(vec![key(binding)]).unwrap();
        assert_eq!(panel.document(), states.last().unwrap());
        assert!(panel.has_pending_draft());
        panel.apply_draft().unwrap();
        assert!(!panel.has_pending_draft());
        assert_eq!(panel.selected_action().bindings, vec![key(binding)]);
        CollectBindings::from_map(panel.document()).unwrap();
        states.push(panel.document().clone());
    }
    for expected in states[..states.len() - 1].iter().rev() {
        assert!(panel.undo().unwrap());
        assert_eq!(panel.document(), expected);
    }
    assert!(!panel.undo().unwrap());
    for expected in &states[1..] {
        assert!(panel.redo().unwrap());
        assert_eq!(panel.document(), expected);
    }
    assert!(!panel.redo().unwrap());
}

#[test]
fn draft_conflicts_preserve_document_history_selection_and_draft() {
    let mut panel = Panel::default();
    set(&mut panel, "left", &[Key::I]);
    panel.undo().unwrap();
    select(&mut panel, "right");
    for draft in [vec![key(Key::A)], vec![key(Key::L), key(Key::L)]] {
        panel.replace_draft(draft).unwrap();
        let before = snapshot(&panel);
        assert!(panel
            .apply_draft()
            .unwrap_err()
            .contains("duplicate physical binding"));
        assert_eq!(snapshot(&panel), before);
        assert!(panel.can_redo());
    }
    panel.discard_draft();
    assert!(panel.redo().unwrap());
    assert_eq!(panel.document().actions[0].bindings, vec![key(Key::I)]);
}

#[test]
fn complete_map_admission_rejects_wrong_schema_devices_and_conflicts_atomically() {
    let mut panel = Panel::default();
    set(&mut panel, "left", &[Key::I]);
    panel.undo().unwrap();
    let original = panel.document().clone();
    let mut candidates = Vec::new();
    let mut map = original.clone();
    map.version = 2;
    candidates.push(map);
    let mut map = original.clone();
    map.actions.pop();
    candidates.push(map);
    let mut map = original.clone();
    map.actions[0].name = "fire".into();
    candidates.push(map);
    let mut map = original.clone();
    map.actions[0].name = "quit".into();
    candidates.push(map);
    let mut map = original.clone();
    map.actions.push(Action {
        name: "pause".into(),
        bindings: Vec::new(),
    });
    candidates.push(map);
    for button in [
        Button::Mouse { button: 0 },
        Button::Gamepad {
            device_id: 0,
            button: 0,
        },
    ] {
        let mut map = original.clone();
        map.actions[0].bindings = vec![button];
        candidates.push(map);
    }
    let mut map = original.clone();
    map.actions[0].bindings = vec![key(Key::D)];
    candidates.push(map);
    let mut map = original.clone();
    map.actions[0].bindings = [
        Key::B,
        Key::C,
        Key::E,
        Key::F,
        Key::G,
        Key::H,
        Key::I,
        Key::J,
        Key::K,
    ]
    .map(key)
    .to_vec();
    candidates.push(map);
    for candidate in candidates {
        let before = snapshot(&panel);
        assert!(panel.apply_document(candidate).is_err());
        assert_eq!(snapshot(&panel), before);
    }
    assert!(panel.redo().unwrap());
}

#[test]
fn drafts_enforce_eight_keyboard_choices_without_early_conflict_validation() {
    let mut panel = Panel::default();
    let eight = [
        Key::B,
        Key::C,
        Key::E,
        Key::F,
        Key::G,
        Key::H,
        Key::I,
        Key::J,
    ]
    .map(key)
    .to_vec();
    panel.replace_draft(eight.clone()).unwrap();
    panel.apply_draft().unwrap();
    assert_eq!(panel.selected_action().bindings, eight);
    for invalid in [
        vec![key(Key::Z); MAX_BINDINGS + 1],
        vec![Button::Mouse { button: 0 }],
        vec![Button::Gamepad {
            device_id: 1,
            button: 0,
        }],
    ] {
        let before = snapshot(&panel);
        assert!(panel.replace_draft(invalid).is_err());
        assert_eq!(snapshot(&panel), before);
    }
    panel.replace_draft(vec![key(Key::D)]).unwrap();
    assert!(panel.has_pending_draft());
    assert!(panel.apply_draft().is_err());
}

#[test]
fn all_six_actions_may_be_unbound_including_restart_and_quit() {
    let mut panel = Panel::default();
    for name in ["left", "right", "up", "down", "restart", "quit"] {
        set(&mut panel, name, &[]);
    }
    assert!(panel
        .document()
        .actions
        .iter()
        .all(|action| action.bindings.is_empty()));
    let bindings = CollectBindings::from_map(panel.document()).unwrap();
    assert_eq!(bindings.hint(CollectAction::Restart), "Unbound");
    assert_eq!(bindings.hint(CollectAction::Quit), "Unbound");
    assert_eq!(bindings.controls_hint().matches("Unbound").count(), 6);
    assert_eq!(
        ActionMap::load(encoded(panel.document()).as_slice()).unwrap(),
        *panel.document()
    );
}

#[test]
fn undo_redo_history_is_exact_and_bounded_to_32_edits() {
    let mut panel = Panel::default();
    let mut states = vec![panel.document().clone()];
    for index in 0..70 {
        set(
            &mut panel,
            "left",
            &[if index % 2 == 0 { Key::I } else { Key::J }],
        );
        states.push(panel.document().clone());
        assert!(panel.undo.len() <= MAX_HISTORY);
        assert!(panel.redo.len() <= MAX_HISTORY);
    }
    for index in (states.len() - 1 - MAX_HISTORY..states.len() - 1).rev() {
        assert!(panel.undo().unwrap());
        assert_eq!(panel.document(), &states[index]);
    }
    assert!(!panel.undo().unwrap());
    for expected in &states[states.len() - MAX_HISTORY..] {
        assert!(panel.redo().unwrap());
        assert_eq!(panel.document(), expected);
    }
    assert!(!panel.redo().unwrap());
    assert_eq!(panel.undo.len(), MAX_HISTORY);
    panel.undo().unwrap();
    set(&mut panel, "left", &[Key::Z]);
    assert!(!panel.can_redo());
    assert_eq!(panel.undo.len(), MAX_HISTORY);
}

#[test]
fn no_op_apply_and_defaults_preserve_history_and_redo() {
    let mut panel = Panel::default();
    panel.apply_draft().unwrap();
    panel.use_defaults().unwrap();
    assert!(!panel.can_undo());
    set(&mut panel, "left", &[Key::I]);
    panel.undo().unwrap();
    let before = snapshot(&panel);
    panel.apply_draft().unwrap();
    panel.apply_document(panel.document().clone()).unwrap();
    panel.use_defaults().unwrap();
    assert_eq!(snapshot(&panel), before);
    panel.redo().unwrap();
    panel.use_defaults().unwrap();
    assert_eq!(panel.document(), &default_map());
    panel.undo().unwrap();
    assert_eq!(panel.selected_action().bindings, vec![key(Key::I)]);
}

#[test]
fn pending_draft_blocks_navigation_replacement_and_open_until_explicit_resolution() {
    let mut panel = Panel::default();
    set(&mut panel, "left", &[Key::I]);
    panel.undo().unwrap();
    panel.replace_draft(vec![key(Key::J)]).unwrap();
    let before = snapshot(&panel);
    assert!(panel.select_action(1).unwrap_err().contains("Unapplied"));
    assert!(panel.undo().unwrap_err().contains("Unapplied"));
    assert!(panel.redo().unwrap_err().contains("Unapplied"));
    assert!(panel.use_defaults().unwrap_err().contains("Unapplied"));
    assert!(panel
        .apply_document(default_map())
        .unwrap_err()
        .contains("Unapplied"));
    assert!(panel
        .open(Path::new("missing-map-that-must-not-be-opened.json"))
        .unwrap_err()
        .contains("Unapplied"));
    panel.select_action(panel.selected_action_index()).unwrap();
    assert_eq!(snapshot(&panel), before);
    panel.discard_draft();
    assert!(!panel.has_pending_draft());
    panel.select_action(1).unwrap();
    assert_eq!(panel.selected_action().name, "right");
    assert!(panel.redo().unwrap());
    assert_eq!(panel.document().actions[0].bindings, vec![key(Key::I)]);
    panel.replace_draft(vec![key(Key::L)]).unwrap();
    panel.apply_draft().unwrap();
    assert!(!panel.has_pending_draft());
}

#[test]
fn invalid_selection_is_atomic_and_import_order_preserves_semantic_selection() {
    let mut panel = Panel::default();
    select(&mut panel, "restart");
    let before = snapshot(&panel);
    assert!(panel.select_action(6).is_err());
    assert!(panel.select_action(usize::MAX).is_err());
    assert_eq!(snapshot(&panel), before);
    let mut reordered = panel.document().clone();
    reordered.actions.reverse();
    panel.apply_document(reordered.clone()).unwrap();
    assert_eq!(panel.selected_action().name, "restart");
    assert_eq!(panel.selected_action_index(), 1);
    assert_eq!(panel.draft(), panel.selected_action().bindings);
    panel.undo().unwrap();
    assert_eq!(panel.selected_action().name, "restart");
    assert_eq!(panel.selected_action_index(), 4);
    panel.redo().unwrap();
    assert_eq!(panel.selected_action().name, "restart");
    assert_eq!(panel.document(), &reordered);
}

#[test]
fn save_reopen_roundtrip_uses_existing_format_and_real_collect_loader() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("collect.json");
    let mut panel = Panel::default();
    set(&mut panel, "left", &[Key::I]);
    set(&mut panel, "restart", &[Key::R]);
    let expected = panel.document().clone();
    let before_save = snapshot(&panel);
    panel.save_as(&path).unwrap();
    assert_eq!(snapshot(&panel), before_save);
    assert_eq!(fs::read(&path).unwrap(), encoded(&expected));
    assert_eq!(ActionMap::load_file(&path).unwrap(), expected);
    assert_eq!(
        CollectBindings::load_file(&path).unwrap(),
        CollectBindings::from_map(&expected).unwrap()
    );
    let mut reopened = Panel::default();
    reopened.open(&path).unwrap();
    assert_eq!(reopened.document(), &expected);
    assert_eq!(reopened.undo.len(), 1);
    assert!(reopened.undo().unwrap());
    assert_eq!(reopened.document(), &default_map());
    assert!(reopened.redo().unwrap());
    assert_eq!(reopened.document(), &expected);
    let copy = directory.path().join("copy.json");
    reopened.save_as(&copy).unwrap();
    assert_eq!(fs::read(path).unwrap(), fs::read(copy).unwrap());
}

#[test]
fn save_accepted_state_preserves_even_invalid_unapplied_draft_and_redo() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("accepted.json");
    let mut panel = Panel::default();
    set(&mut panel, "left", &[Key::I]);
    panel.undo().unwrap();
    panel.replace_draft(vec![key(Key::D)]).unwrap();
    let before = snapshot(&panel);
    panel.save_as(&path).unwrap();
    assert_eq!(snapshot(&panel), before);
    assert_eq!(ActionMap::load_file(&path).unwrap(), before.document);
    assert_eq!(fs::read(&path).unwrap(), encoded(&default_map()));
    assert!(panel.has_pending_draft());
    assert!(panel.open(&path).is_err());
    assert_eq!(snapshot(&panel), before);
    panel.discard_draft();
    panel.open(&path).unwrap();
    assert!(panel.can_redo());
}

#[test]
fn failed_open_preserves_all_state_for_malformed_oversized_missing_and_directory_paths() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("invalid.json");
    let mut panel = Panel::default();
    set(&mut panel, "left", &[Key::I]);
    panel.undo().unwrap();
    select(&mut panel, "quit");
    panel.set_message("prior status");
    let before = snapshot(&panel);
    for bytes in [
        b"{".to_vec(),
        b"{\"version\":1,\"actions\":[],\"unknown\":true}".to_vec(),
        vec![b' '; MAX_FILE_BYTES + 1],
    ] {
        fs::write(&path, bytes).unwrap();
        assert!(panel.open(&path).is_err());
        assert_eq!(snapshot(&panel), before);
    }
    assert!(panel.open(&directory.path().join("missing.json")).is_err());
    assert!(panel.open(directory.path()).is_err());
    assert_eq!(snapshot(&panel), before);
    assert!(panel.redo().unwrap());
}

#[test]
fn open_rejects_shared_format_mouse_map_and_semantically_wrong_actions() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("unsupported.json");
    let mut panel = Panel::default();
    set(&mut panel, "left", &[Key::I]);
    panel.undo().unwrap();
    let before = snapshot(&panel);
    for unsupported in [
        Button::Mouse { button: 0 },
        Button::Gamepad {
            device_id: 0,
            button: 0,
        },
    ] {
        let mut map = default_map();
        map.actions[0].bindings = vec![unsupported];
        fs::write(&path, encoded(&map)).unwrap();
        assert!(panel.open(&path).unwrap_err().contains("keyboard"));
        assert_eq!(snapshot(&panel), before);
    }
    let mut map = default_map();
    map.actions[0].name = "fire".into();
    fs::write(&path, encoded(&map)).unwrap();
    assert!(panel.open(&path).unwrap_err().contains("exactly"));
    assert_eq!(snapshot(&panel), before);
}

#[test]
fn shared_loader_accepts_exact_64_kib_boundary_and_refuses_one_more_byte() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("bounded.json");
    let mut panel = Panel::default();
    let mut bytes = encoded(panel.document());
    bytes.resize(MAX_FILE_BYTES, b' ');
    fs::write(&path, &bytes).unwrap();
    panel.open(&path).unwrap();
    assert_eq!(panel.document(), &default_map());
    let before = snapshot(&panel);
    bytes.push(b' ');
    fs::write(&path, bytes).unwrap();
    assert!(panel.open(&path).unwrap_err().contains("64 KiB"));
    assert_eq!(snapshot(&panel), before);
}

#[test]
fn save_as_never_overwrites_existing_destinations_and_failed_save_preserves_state() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("existing.json");
    fs::write(&path, b"existing contents must remain exact").unwrap();
    let mut panel = Panel::default();
    set(&mut panel, "left", &[Key::I]);
    panel.replace_draft(vec![key(Key::J)]).unwrap();
    let before = snapshot(&panel);
    assert!(panel.save_as(&path).is_err());
    assert_eq!(
        fs::read(&path).unwrap(),
        b"existing contents must remain exact"
    );
    assert!(panel.save_as(directory.path()).is_err());
    assert!(panel
        .save_as(&directory.path().join("missing-parent/new.json"))
        .is_err());
    assert_eq!(snapshot(&panel), before);
    let names: Vec<_> = fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("existing.json")]);
}

#[cfg(unix)]
#[test]
fn direct_and_dangling_symlinks_cannot_be_opened_or_overwritten() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("map.json");
    let direct = directory.path().join("direct.json");
    let dangling = directory.path().join("dangling.json");
    let missing = directory.path().join("missing.json");
    let mut panel = Panel::default();
    panel.save_as(&path).unwrap();
    let bytes = fs::read(&path).unwrap();
    std::os::unix::fs::symlink(&path, &direct).unwrap();
    std::os::unix::fs::symlink(&missing, &dangling).unwrap();
    let before = snapshot(&panel);
    for link in [&direct, &dangling] {
        assert!(panel.open(link).is_err());
        assert!(panel.save_as(link).is_err());
        assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
        assert_eq!(snapshot(&panel), before);
    }
    assert_eq!(fs::read(path).unwrap(), bytes);
    assert!(!missing.exists());
}

#[cfg(unix)]
#[test]
fn preexisting_fifo_and_device_are_refused_without_opening_or_changing_state() {
    use std::os::unix::fs::FileTypeExt;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("bindings.fifo");
    assert!(std::process::Command::new("mkfifo")
        .arg(&path)
        .status()
        .unwrap()
        .success());
    assert!(fs::symlink_metadata(&path).unwrap().file_type().is_fifo());
    let mut panel = Panel::default();
    let before = snapshot(&panel);
    assert!(panel.open(&path).unwrap_err().contains("regular file"));
    assert!(panel
        .open(Path::new("/dev/null"))
        .unwrap_err()
        .contains("regular file"));
    assert!(panel.save_as(&path).is_err());
    assert!(fs::symlink_metadata(&path).unwrap().file_type().is_fifo());
    assert_eq!(snapshot(&panel), before);
}
