//! CPU egui controls, no native window or GPU.
use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
fn fixture() -> (tempfile::TempDir, Harness<'static, (Panel, Editor, bool)>) {
    let dir = tempfile::tempdir().unwrap();
    let mut editor = crate::mesh_workspace::mission::prepare(
        &dir.path().join("preview.scene.yaml"),
        &Default::default(),
    )
    .unwrap();
    let mut p = Panel::default();
    p.sync(&mut editor);
    let h = Harness::builder()
        .with_size([1200.0, 900.0])
        .build_ui_state(
            |ui, s: &mut (Panel, Editor, bool)| {
                let _ = s.0.static_preview_camera(&s.1, (800, 600));
                s.0.show_static_preview_controls(ui, &s.1, s.2);
            },
            (p, editor, true),
        );
    (dir, h)
}
#[test]
fn explicit_checkbox_default_off_preserves_authoring_and_frame_and_can_disable_when_host_blocked() {
    let (_dir, mut h) = fixture();
    h.run_steps(2);
    assert!(!h.state().0.static_preview_enabled());
    let frame = h.state().1.checksum();
    let dirty = h.state().0.dirty();
    let history = h.state().0.history.len();
    h.get_by_label("Preview static camera draft")
        .click_accesskit();
    h.run_steps(2);
    assert!(h.state().0.static_preview_enabled());
    {
        let s = h.state_mut();
        assert!(s.0.static_preview_camera(&s.1, (800, 600)).is_some());
    }
    assert_eq!(h.state().0.dirty(), dirty);
    assert_eq!(h.state().0.history.len(), history);
    assert_eq!(h.state().1.checksum(), frame);
    assert!(h.state().0.current_document().is_none());
    h.state_mut().2 = false;
    h.run_steps(2);
    h.get_by_label("Preview static camera draft")
        .click_accesskit();
    h.run_steps(2);
    assert!(!h.state().0.static_preview_enabled());
}
#[test]
fn follow_invalid_stale_busy_and_focus_guards_refuse_or_retire_preview() {
    let (_dir, mut h) = fixture();
    for case in 0..5 {
        h.state_mut().0.follow_agent = case == 0;
        h.state_mut().0.distance = if case == 1 { "NaN" } else { "16" }.into();
        h.state_mut().0.context_stale = case == 2;
        h.state_mut().0.set_static_preview_busy(case == 3);
        h.input_mut().focused = case != 4;
        h.run_steps(2);
        assert!(
            h.get_by_label("Preview static camera draft")
                .accesskit_node()
                .is_disabled(),
            "case {case}"
        );
        assert!(!h.state().0.static_preview_enabled());
    }
    h.input_mut().focused = true;
    h.run_steps(2);
    let (target_node, target_tree) = h
        .get_by_label("Preview static camera draft")
        .accesskit_node()
        .locate();
    h.input_mut().events.extend([
        egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
            target_node,
            target_tree,
            action: egui::accesskit::Action::Click,
            data: None,
        }),
        egui::Event::WindowFocused(false),
    ]);
    h.step();
    assert!(!h.state().0.static_preview_enabled());
    h.run_steps(2);
    h.get_by_label("Preview static camera draft")
        .click_accesskit();
    h.run_steps(2);
    assert!(h.state().0.static_preview_enabled());
    h.state_mut().0.follow_agent = true;
    h.run_steps(2);
    assert!(!h.state().0.static_preview_enabled());
}
