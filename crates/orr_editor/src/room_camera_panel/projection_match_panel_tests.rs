//! CPU widget tests use an in-memory panel; no directory handle, Save or native window.
use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
fn panel() -> Panel {
    let document = Document::readable_default();
    Panel {
        path: PathBuf::from("/not-accessed/room.camera.json"),
        manifest_path: PathBuf::from("/not-accessed/orr.project.json"),
        root: Err("in-memory test".into()),
        candidate: document.clone(),
        saved: document.clone(),
        saved_bytes: document.to_bytes().unwrap(),
        document,
        manifest_bytes: Vec::new(),
        undo: Vec::new(),
        redo: Vec::new(),
        error: None,
        match_width: "1280".into(),
        match_height: "720".into(),
    }
}
fn harness(panel: Panel, editable: bool) -> Harness<'static, (Panel, usize)> {
    let mut h = Harness::builder()
        .with_size([1000.0, 1600.0])
        .build_ui_state(
            move |ui, state: &mut (Panel, usize)| {
                state.1 += usize::from(state.0.show(ui, editable));
            },
            (panel, 0),
        );
    h.run_steps(3);
    h.get_by_label("Room camera").click_accesskit();
    h.run_steps(3);
    if editable {
        h.get_by_label("Match projection scale").click_accesskit();
        h.run_steps(3);
    }
    h
}
fn click(h: &mut Harness<'_, (Panel, usize)>, label: &str) {
    h.get_by_label(label).click_accesskit();
    h.run_steps(3);
}
#[test]
fn matching_projection_button_stages_only_then_explicit_apply_undo_redo_use_existing_history() {
    let p = panel();
    let old = p.document.clone();
    let saved = p.saved_bytes.clone();
    let expected = projection_match::match_projection(&old, (1280, 720)).unwrap();
    let mut h = harness(p, true);
    assert!(h.query_by_label("Match projection aspect width").is_some());
    assert!(h.query_by_label("Match projection aspect height").is_some());
    click(&mut h, "Switch projection keeping target-plane scale");
    assert_eq!(h.state().0.candidate, expected);
    assert_eq!(h.state().0.document, old);
    assert_eq!(h.state().0.saved_bytes, saved);
    assert!(h.state().0.undo.is_empty());
    assert_eq!(h.state().1, 0);
    click(&mut h, "Apply camera");
    assert_eq!(h.state().0.document, expected);
    assert_eq!(h.state().1, 1);
    click(&mut h, "Undo camera edit");
    assert_eq!(h.state().0.document, old);
    assert_eq!(h.state().1, 2);
    click(&mut h, "Redo camera edit");
    assert_eq!(h.state().0.document, expected);
    assert_eq!(h.state().1, 3);
    assert_eq!(h.state().0.saved_bytes, saved);
}
#[test]
fn invalid_aspect_or_camera_preserves_candidate_applied_history_and_saved_bytes() {
    for case in ["empty", "negative", "zero", "overflow", "camera"] {
        let mut p = panel();
        let old = p.candidate.clone();
        let mut applied = old.clone();
        applied.target[0] = 1.0;
        p.apply_document(applied).unwrap();
        p.undo();
        match case {
            "empty" => p.match_width.clear(),
            "negative" => p.match_width = "-1".into(),
            "zero" => p.match_height = "0".into(),
            "overflow" => p.match_width = "4294967296".into(),
            _ => p.candidate.distance = f32::NAN,
        }
        let before = p.candidate.to_bytes().ok();
        let undo = p.undo.clone();
        let redo = p.redo.clone();
        let saved = p.saved_bytes.clone();
        p.stage_matching_projection();
        assert!(p.error.is_some(), "{case}");
        assert_eq!(p.candidate.to_bytes().ok(), before);
        assert_eq!(p.document, old);
        if case == "camera" {
            assert!(p.candidate.distance.is_nan());
        }
        assert_eq!(p.undo, undo);
        assert_eq!(p.redo, redo);
        assert_eq!(p.saved_bytes, saved);
    }
}
#[test]
fn matching_projection_controls_are_read_only_during_play() {
    let p = panel();
    let old = p.candidate.clone();
    let h = harness(p, false);
    assert!(h
        .get_by_label("Match projection scale")
        .accesskit_node()
        .is_disabled());
    assert_eq!(h.state().0.candidate, old);
    assert_eq!(h.state().1, 0);
    assert!(h.state().0.undo.is_empty());
}
