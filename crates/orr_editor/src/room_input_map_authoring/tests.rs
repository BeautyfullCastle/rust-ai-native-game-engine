use super::*;
use orr_input::Key;
fn key(key: Key) -> Button {
    Button::Keyboard { key }
}
#[test]
fn rejects_invalid_drafts_without_mutating_map_or_history() {
    let mut panel = Panel::default();
    let original = panel.document().clone();
    for draft in [
        vec![],
        vec![key(Key::R)],
        vec![key(Key::Escape)],
        vec![key(Key::D)],
        vec![key(Key::J), key(Key::J)],
    ] {
        panel.replace_draft(draft.clone()).unwrap();
        assert!(panel.apply_draft().is_err());
        assert_eq!(panel.draft(), draft);
        assert_eq!(panel.document(), &original);
        assert!(!panel.can_undo());
        assert!(panel.select_action(1).is_err());
        assert!(panel.use_defaults().is_err());
        assert!(panel.undo().is_err());
        assert!(panel.redo().is_err());
        panel.discard_draft();
    }
}
#[test]
fn bounded_history_and_semantic_selection_survive_reordered_imports() {
    let mut panel = Panel::default();
    for i in 0..40 {
        panel
            .replace_draft(vec![key(if i % 2 == 0 { Key::J } else { Key::K })])
            .unwrap();
        panel.apply_draft().unwrap();
    }
    for _ in 0..32 {
        assert!(panel.undo().unwrap());
    }
    assert!(!panel.undo().unwrap());
    for _ in 0..32 {
        assert!(panel.redo().unwrap());
    }
    assert!(!panel.redo().unwrap());
    let mut reordered = panel.document().clone();
    reordered.actions.reverse();
    panel.apply_document(reordered).unwrap();
    assert_eq!(panel.selected_action().name, "move_left");
    panel.undo().unwrap();
    assert_eq!(panel.selected_action().name, "move_left");
}
#[test]
fn safe_persistence_pending_draft_and_runtime_interpretation() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("controls.json");
    let mut panel = Panel::default();
    panel.replace_draft(vec![key(Key::J)]).unwrap();
    panel.apply_draft().unwrap();
    panel.replace_draft(vec![key(Key::R)]).unwrap();
    panel.save_as(&file).unwrap();
    assert!(panel.save_as(&file).is_err());
    assert!(panel.open(&file).is_err());
    assert_eq!(panel.draft(), &[key(Key::R)]);
    let saved = ActionMap::load_file(&file).unwrap();
    assert_eq!(&saved, panel.document());
    let controls = RoomControls::from_map(&saved).unwrap();
    assert_eq!(controls.input_from_keys([Key::J]).move_x, -1);
    assert_eq!(controls.input_from_keys([Key::J, Key::D]).move_x, 0);
    assert_eq!(controls.input_from_keys([Key::A]).move_x, 0);
    assert_eq!(controls.input_from_keys([Key::W]).move_z, -1);
    assert_ne!(controls.input_from_keys([Key::E]).buttons, 0);
    panel.discard_draft();
    panel.use_defaults().unwrap();
    panel.open(&file).unwrap();
    assert_eq!(panel.document(), &saved);
    let invalid = dir.path().join("bad.json");
    std::fs::write(&invalid, b"{").unwrap();
    assert!(panel.open(&invalid).is_err());
    assert_eq!(panel.document(), &saved);
    #[cfg(unix)]
    {
        let link = dir.path().join("link.json");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert!(panel.open(&link).is_err());
        assert!(panel.save_as(&link).is_err());
    }
}
#[test]
fn save_rejects_incompatible_root_document_names_without_writes() {
    let dir = tempfile::tempdir().unwrap();
    let panel = Panel::default();
    for name in [
        "orr.project.json",
        "ORR.PROJECT.json",
        "orr.packages.lock.json",
        "space name.json",
        ".hidden.json",
        "controls.JSON",
        "é.json",
    ] {
        let path = dir.path().join(name);
        assert!(panel.save_as(&path).is_err());
        assert!(!path.exists());
    }
}

#[test]
fn eight_aliases_roundtrip_and_overflow_and_failed_save_preserve_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("eight.json");
    let mut panel = Panel::default();
    let eight = vec![
        key(Key::B),
        key(Key::C),
        key(Key::F),
        key(Key::G),
        key(Key::H),
        key(Key::I),
        key(Key::J),
        key(Key::K),
    ];
    panel.replace_draft(eight.clone()).unwrap();
    panel.apply_draft().unwrap();
    panel.save_as(&path).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let accepted = panel.document().clone();
    let mut nine = eight.clone();
    nine.push(key(Key::L));
    assert!(panel.replace_draft(nine).is_err());
    assert_eq!(panel.draft(), eight);
    assert_eq!(panel.document(), &accepted);
    panel.replace_draft(vec![key(Key::Q)]).unwrap();
    assert!(panel.save_as(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(panel.document(), &accepted);
    assert_eq!(panel.draft(), &[key(Key::Q)]);
    panel.discard_draft();
    assert!(panel.undo().unwrap());
    assert!(!panel.can_undo());
    assert!(panel.redo().unwrap());
    assert_eq!(panel.document(), &accepted);
    let mut reopened = Panel::default();
    reopened.open(&path).unwrap();
    assert_eq!(reopened.document(), &accepted);
}
