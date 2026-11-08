//! Actual CPU widgets only; no OS input, native window, or GPU.
use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
fn fixture() -> (tempfile::TempDir, Harness<'static, (Panel, Editor, bool)>) {
    let dir = tempfile::tempdir().unwrap();
    let editor = crate::mesh_workspace::mission::prepare(
        &dir.path().join("pick.scene.yaml"),
        &Default::default(),
    )
    .unwrap();
    let mut panel = Panel::default();
    panel.sync(&editor);
    let h = Harness::builder()
        .with_size([1000.0, 900.0])
        .build_ui_state(
            |ui, state: &mut (Panel, Editor, bool)| {
                state
                    .0
                    .show_point_light_pick_controls(ui, &state.1, state.2);
            },
            (panel, editor, true),
        );
    (dir, h)
}
#[test]
fn explicit_pick_requires_existing_light_and_cancel_remains_available_when_host_disabled() {
    let (_dir, mut h) = fixture();
    h.run_steps(2);
    assert!(h
        .get_by_label("Pick point light X/Z")
        .accesskit_node()
        .is_disabled());
    h.get_by_label("Pick point light X/Z").click_accesskit();
    h.run_steps(2);
    assert!(h.state().0.point_light.is_none());
    assert!(!h.state().0.pick_pending());
    h.state_mut().0.point_light = Some(orr_render::PointLight {
        position: [0.0, 4.0, 0.0],
        color: [1.0, 0.9, 0.7],
        intensity: 3.0,
        range: 12.0,
    });
    h.run_steps(2);
    let before = h.state().0.point_light;
    let frame = h.state().1.checksum();
    h.get_by_label("Pick point light X/Z").click_accesskit();
    h.run_steps(2);
    assert!(h.state().0.pick_pending());
    h.state_mut().2 = false;
    h.run_steps(2);
    assert!(!h
        .get_by_label("Cancel point light pick")
        .accesskit_node()
        .is_disabled());
    h.get_by_label("Cancel point light pick").click_accesskit();
    h.run_steps(2);
    assert!(!h.state().0.pick_pending());
    assert_eq!(h.state().0.point_light, before);
    assert_eq!(h.state().1.checksum(), frame);
    assert_eq!(h.state().0.history.len(), 1);
    h.state_mut().2 = true;
    h.state_mut().0.set_viewport_pick_busy(true);
    h.run_steps(2);
    assert!(!h.state().0.pick_pending());
    assert!(h
        .get_by_label("Pick point light X/Z")
        .accesskit_node()
        .is_disabled());
    h.get_by_label("Pick point light X/Z").click_accesskit();
    h.run_steps(2);
    assert!(!h.state().0.pick_pending());
    h.state_mut().0.set_viewport_pick_busy(false);
    h.run_steps(2);
    assert!(!h
        .get_by_label("Pick point light X/Z")
        .accesskit_node()
        .is_disabled());
}
#[test]
fn readonly_stale_and_unfocused_actual_pick_controls_refuse_arming() {
    let (_dir, mut h) = fixture();
    h.state_mut().0.point_light = Some(orr_render::PointLight {
        position: [0.0, 4.0, 0.0],
        color: [1.0, 0.9, 0.7],
        intensity: 3.0,
        range: 12.0,
    });
    h.state_mut().2 = false;
    h.run_steps(2);
    assert!(h
        .get_by_label("Pick point light X/Z")
        .accesskit_node()
        .is_disabled());
    h.state_mut().2 = true;
    h.state_mut().0.context_stale = true;
    h.run_steps(2);
    assert!(h
        .get_by_label("Pick point light X/Z")
        .accesskit_node()
        .is_disabled());
    h.state_mut().0.context_stale = false;
    h.input_mut().focused = false;
    h.run_steps(2);
    assert!(h
        .get_by_label("Pick point light X/Z")
        .accesskit_node()
        .is_disabled());
    assert!(!h.state().0.pick_pending());
    h.input_mut().focused = true;
    h.run_steps(2);
    let (target_node, target_tree) = h
        .get_by_label("Pick point light X/Z")
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
    assert!(!h.state().0.pick_pending());
    assert!(h
        .get_by_label("Pick point light X/Z")
        .accesskit_node()
        .is_disabled());
}
