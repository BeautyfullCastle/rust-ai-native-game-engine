//! CPU-only real control/history tests; no host, filesystem or GPU.
use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
fn harness(editable: bool) -> Harness<'static, (Panel, usize)> {
    Harness::builder()
        .with_size([1200.0, 1600.0])
        .build_ui_state(
            move |ui, state: &mut (Panel, usize)| {
                ui.add_enabled_ui(editable, |ui| {
                    state.1 += usize::from(state.0.show_point_light_controls(ui));
                });
            },
            (Panel::default(), 0usize),
        )
}
#[test]
fn actual_point_enable_and_reset_controls_stage_only_without_history_or_other_lighting_changes() {
    let mut h = harness(true);
    h.run_steps(2);
    let direction = h.state().0.direction.clone();
    let effects = h.state().0.post_process;
    h.get_by_label("Enable point light").click_accesskit();
    h.run_steps(2);
    assert_eq!(h.state().0.point_light, Some(defaults()));
    assert!(h.state().0.current_document().is_none());
    assert_eq!(h.state().0.history.len(), 1);
    assert_eq!(h.state().1, 1);
    h.state_mut().0.point_light.as_mut().unwrap().intensity = 7.0;
    h.get_by_label("Point light settings").click_accesskit();
    h.run_steps(2);
    h.get_by_label("Reset point light defaults")
        .click_accesskit();
    h.run_steps(2);
    assert_eq!(h.state().0.point_light, Some(defaults()));
    assert_eq!(h.state().0.direction, direction);
    assert_eq!(h.state().0.post_process, effects);
    assert!(h.state().0.current_document().is_none());
}
#[test]
fn parsed_point_document_history_undo_redo_preserves_effects_and_directional_lighting() {
    let mut panel = Panel::default();
    panel.point_light = Some(defaults());
    panel.post_process.enabled = true;
    panel.post_process_authored = true;
    let document = panel
        .parse_document(Path::new("point.scene.yaml"), [3; 32])
        .unwrap();
    assert_eq!(
        document.point_light_settings().unwrap().point_light,
        Some(defaults())
    );
    assert!(document.post_process_settings().enabled);
    panel.push_state(Some(document.clone()));
    panel.copy_current_fields();
    panel.undo();
    assert!(panel.point_light.is_none());
    panel.redo();
    assert_eq!(panel.point_light, Some(defaults()));
    assert_eq!(panel.current_document(), Some(&document));
    assert!(panel.post_process.enabled);
}
#[test]
fn invalid_point_data_rejects_parse_without_mutating_applied_document_or_history() {
    for bad in ["nan", "range", "color"] {
        let mut panel = Panel::default();
        panel.point_light = Some(defaults());
        match bad {
            "nan" => panel.point_light.as_mut().unwrap().position[0] = f32::NAN,
            "range" => panel.point_light.as_mut().unwrap().range = 0.0,
            _ => panel.point_light.as_mut().unwrap().color[0] = 10001.0,
        };
        assert!(panel
            .parse_document(Path::new("point.scene.yaml"), [3; 32])
            .is_err());
        assert!(panel.current_document().is_none());
        assert_eq!(panel.history.len(), 1);
        assert_eq!(panel.cursor, 0);
    }
}
#[test]
fn readonly_point_controls_do_not_stage_or_enable_a_light() {
    let mut h = harness(false);
    h.run_steps(2);
    assert!(h
        .get_by_label("Enable point light")
        .accesskit_node()
        .is_disabled());
    h.get_by_label("Enable point light").click_accesskit();
    h.run_steps(2);
    assert!(h.state().0.point_light.is_none());
    assert_eq!(h.state().1, 0);
}
