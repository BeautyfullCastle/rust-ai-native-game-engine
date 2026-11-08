//! Actual CPU checkbox dispatch; no native window, OS input or GPU.
use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
fn fixture() -> (tempfile::TempDir, Harness<'static, (Panel, Editor, bool)>) {
    let dir = tempfile::tempdir().unwrap();
    let editor = crate::mesh_workspace::mission::prepare(
        &dir.path().join("preview.scene.yaml"),
        &Default::default(),
    )
    .unwrap();
    let mut panel = Panel::default();
    panel.sync(&editor);
    let h = Harness::builder()
        .with_size([1200.0, 900.0])
        .build_ui_state(
            |ui, s: &mut (Panel, Editor, bool)| {
                s.0.show_point_light_preview_controls(ui, &s.1, s.2)
            },
            (panel, editor, true),
        );
    (dir, h)
}
fn light() -> orr_render::PointLight {
    orr_render::PointLight {
        position: [2.0, 5.0, -1.0],
        color: [0.2, 0.7, 0.4],
        intensity: 7.0,
        range: 9.0,
    }
}
#[test]
fn explicit_checkbox_defaults_off_and_never_enables_light_or_changes_authoring_state() {
    let (_dir, mut h) = fixture();
    h.run_steps(2);
    assert!(!h.state().0.point_light_preview_enabled());
    {
        let state = h.state_mut();
        assert!(state.0.preview_point_light_settings(&state.1).is_none());
    }
    assert!(!h
        .get_by_label("Preview point light draft")
        .accesskit_node()
        .is_disabled());
    h.get_by_label("Preview point light draft")
        .click_accesskit();
    h.run_steps(2);
    assert!(h.state().0.point_light_preview_enabled());
    assert!(h.state().0.point_light.is_none());
    {
        let state = h.state_mut();
        assert!(state
            .0
            .preview_point_light_settings(&state.1)
            .unwrap()
            .point_light
            .is_none());
    }
    h.get_by_label("Preview point light draft")
        .click_accesskit();
    h.run_steps(2);
    h.state_mut().0.point_light = Some(light());
    h.run_steps(2);
    let checksum = h.state().1.checksum();
    let dirty = h.state().0.dirty();
    let history = h.state().0.history.len();
    h.get_by_label("Preview point light draft")
        .click_accesskit();
    h.run_steps(2);
    assert!(h.state().0.point_light_preview_enabled());
    {
        let s = h.state_mut();
        assert_eq!(
            s.0.preview_point_light_settings(&s.1).unwrap().point_light,
            Some(light())
        );
    }
    h.state_mut().0.point_light.as_mut().unwrap().intensity = 6.0;
    h.run_steps(2);
    assert!(h.state().0.point_light_preview_enabled());
    {
        let s = h.state_mut();
        assert_eq!(
            s.0.preview_point_light_settings(&s.1)
                .unwrap()
                .point_light
                .unwrap()
                .intensity,
            6.0
        );
    }
    h.state_mut().0.point_light = None;
    h.run_steps(2);
    assert!(h.state().0.point_light_preview_enabled());
    {
        let state = h.state_mut();
        assert!(state
            .0
            .preview_point_light_settings(&state.1)
            .unwrap()
            .point_light
            .is_none());
    }
    assert_eq!(h.state().0.dirty(), dirty);
    assert_eq!(h.state().0.history.len(), history);
    assert_eq!(h.state().1.checksum(), checksum);
    assert!(h.state().0.current_document().is_none());
    h.get_by_label("Preview point light draft")
        .click_accesskit();
    h.run_steps(2);
    assert!(!h.state().0.point_light_preview_enabled());
    h.get_by_label("Preview point light draft")
        .click_accesskit();
    h.run_steps(2);
    {
        let state = h.state_mut();
        assert!(state
            .0
            .preview_point_light_settings(&state.1)
            .unwrap()
            .point_light
            .is_none());
    }
}
#[test]
fn readonly_focus_and_invalid_draft_clear_or_refuse_preview_until_explicit_reenable() {
    let (_dir, mut h) = fixture();
    h.state_mut().0.point_light = Some(light());
    h.state_mut().2 = false;
    h.run_steps(2);
    assert!(h
        .get_by_label("Preview point light draft")
        .accesskit_node()
        .is_disabled());
    h.state_mut().2 = true;
    h.input_mut().focused = false;
    h.run_steps(2);
    assert!(h
        .get_by_label("Preview point light draft")
        .accesskit_node()
        .is_disabled());
    h.input_mut().focused = true;
    h.run_steps(2);
    let (target_node, target_tree) = h
        .get_by_label("Preview point light draft")
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
    assert!(!h.state().0.point_light_preview_enabled());
    h.run_steps(2);

    h.get_by_label("Preview point light draft")
        .click_accesskit();
    h.run_steps(2);
    assert!(h.state().0.point_light_preview_enabled());
    h.state_mut().0.point_light.as_mut().unwrap().range = 0.0;
    h.run_steps(2);
    assert!(!h.state().0.point_light_preview_enabled());
    assert!(h
        .get_by_label("Preview point light draft")
        .accesskit_node()
        .is_disabled());
    h.state_mut().0.point_light = Some(light());
    h.run_steps(2);
    assert!(!h.state().0.point_light_preview_enabled());
    h.get_by_label("Preview point light draft")
        .click_accesskit();
    h.run_steps(2);
    assert!(h.state().0.point_light_preview_enabled());
    h.state_mut().0.context_stale = true;
    h.run_steps(2);
    assert!(!h.state().0.point_light_preview_enabled());
}
