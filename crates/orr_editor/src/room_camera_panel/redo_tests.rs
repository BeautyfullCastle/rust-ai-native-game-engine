use super::*;

fn fixture() -> (tempfile::TempDir, Panel) {
    let dir = tempfile::tempdir().unwrap();
    let document = Document::readable_default();
    let bytes = document.to_bytes().unwrap();
    let path = dir.path().join("room.camera.json");
    let manifest_path = dir.path().join("orr.project.json");
    let manifest_bytes = b"camera fixture".to_vec();
    std::fs::write(&path, &bytes).unwrap();
    std::fs::write(&manifest_path, &manifest_bytes).unwrap();
    let panel = Panel::new(PreparedRoomCamera {
        path,
        manifest_path,
        document,
        bytes,
        manifest_bytes,
    });
    (dir, panel)
}

#[test]
fn redo_restores_full_camera_and_candidate_without_writing() {
    let (_dir, mut panel) = fixture();
    let original = panel.document.clone();
    let mut changed = original.clone();
    changed.target = [1.0, 2.0, 3.0];
    changed.yaw += 0.25;
    changed.distance += 2.0;
    panel.apply_document(changed.clone()).unwrap();
    assert!(panel.undo());
    panel.candidate.distance += 7.0; // draft is discarded by history navigation.
    assert!(panel.redo());
    assert_eq!(panel.document, changed);
    assert_eq!(panel.candidate, changed);
    assert_eq!(panel.undo, vec![original.clone()]);
    assert!(!panel.redo());
    assert_eq!(
        std::fs::read(&panel.path).unwrap(),
        original.to_bytes().unwrap()
    );
}

#[test]
fn no_op_and_invalid_apply_preserve_redo_but_new_branch_clears_it() {
    let (_dir, mut panel) = fixture();
    let mut changed = panel.document.clone();
    changed.distance += 1.0;
    panel.apply_document(changed.clone()).unwrap();
    assert!(panel.undo());
    let baseline = panel.document.clone();
    panel.apply_document(baseline.clone()).unwrap();
    let mut invalid = baseline.clone();
    invalid.distance = f32::NAN;
    assert!(panel.apply_document(invalid).is_err());
    assert_eq!(panel.document, baseline);
    assert!(panel.undo.is_empty());
    assert_eq!(panel.redo, vec![changed]);
    assert!(panel.redo());
    assert!(panel.undo());
    let mut branch = baseline;
    branch.distance += 3.0;
    panel.apply_document(branch.clone()).unwrap();
    assert!(!panel.redo());
    assert_eq!(panel.document, branch);
}

#[test]
fn bounded_history_round_trips_and_save_preserves_redo() {
    let (_dir, mut panel) = fixture();
    for _ in 0..MAX_UNDO + 7 {
        let mut next = panel.document.clone();
        next.distance += 1.0;
        panel.apply_document(next).unwrap();
    }
    let last = panel.document.clone();
    for _ in 0..MAX_UNDO {
        assert!(panel.undo());
    }
    assert!(!panel.undo());
    assert_eq!(panel.redo.len(), MAX_UNDO);
    panel.save().unwrap();
    assert!(!panel.dirty());
    for _ in 0..MAX_UNDO {
        assert!(panel.redo());
    }
    assert!(!panel.redo());
    assert_eq!(panel.document, last);
    assert_eq!(panel.undo.len(), MAX_UNDO);
    assert!(panel.dirty());
    panel.save().unwrap();
    assert_eq!(
        Document::parse(&std::fs::read(&panel.path).unwrap()).unwrap(),
        last
    );
    assert!(!panel.dirty());
}

#[test]
fn failed_save_preserves_both_history_stacks() {
    let (_dir, mut panel) = fixture();
    for _ in 0..2 {
        let mut next = panel.document.clone();
        next.distance += 1.0;
        panel.apply_document(next).unwrap();
    }
    assert!(panel.undo());
    let before = (
        panel.document.clone(),
        panel.undo.clone(),
        panel.redo.clone(),
    );
    std::fs::write(&panel.path, b"external edit").unwrap();
    assert!(panel.save().is_err());
    assert_eq!(
        (
            panel.document.clone(),
            panel.undo.clone(),
            panel.redo.clone()
        ),
        before
    );
    assert!(panel.redo());
    assert_eq!(std::fs::read(&panel.path).unwrap(), b"external edit");
}

#[test]
fn redo_widget_resets_live_once_and_is_disabled_in_play() {
    use egui_kittest::{
        kittest::{NodeT, Queryable},
        Harness,
    };
    let (_dir, mut panel) = fixture();
    let original = panel.document.clone();
    let mut changed = original.clone();
    changed.distance += 1.0;
    panel.apply_document(changed.clone()).unwrap();
    assert!(panel.undo());
    let mut h = Harness::builder().with_size([600.0, 800.0]).build_ui_state(
        |ui, state: &mut (Panel, bool, usize)| {
            state.2 += usize::from(state.0.show(ui, state.1));
        },
        (panel, false, 0),
    );
    h.run_steps(3);
    h.get_by_label("Room camera").click();
    h.run_steps(3);
    assert!(h
        .get_by_label("Redo camera edit")
        .accesskit_node()
        .is_disabled());
    h.get_by_label("Redo camera edit").click_accesskit();
    h.run_steps(3);
    assert_eq!(h.state().0.document, original);
    assert_eq!(h.state().2, 0);
    h.state_mut().1 = true;
    h.run_steps(3);
    h.get_by_label("Redo camera edit").click_accesskit();
    h.run_steps(3);
    assert_eq!(h.state().0.document, changed);
    assert_eq!(h.state().2, 1);
    assert!(h
        .get_by_label("Redo camera edit")
        .accesskit_node()
        .is_disabled());
    h.get_by_label("Redo camera edit").click_accesskit();
    h.run_steps(3);
    assert_eq!(h.state().2, 1);
}

#[test]
fn capture_style_reapply_and_reset_defaults_keep_one_history_branch() {
    let (_dir, mut panel) = fixture();
    let original = panel.document.clone();
    let mut captured = original.clone();
    captured.distance += 4.0;
    panel.apply_document(captured.clone()).unwrap();
    assert!(panel.undo());
    // Existing capture-specific Redo intentionally reapplies its pinned document.
    panel.apply_document(captured.clone()).unwrap();
    assert!(panel.redo.is_empty());
    assert_eq!(panel.undo, vec![original.clone()]);
    assert!(panel.undo());
    assert!(panel.redo());
    panel.apply_document(Document::readable_default()).unwrap();
    assert_eq!(panel.document, original);
    assert!(panel.undo());
    assert_eq!(panel.document, captured);
    assert!(panel.redo());
    assert_eq!(panel.document, original);
}
