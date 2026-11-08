//! Actual CPU egui authoring -> applied history -> saved sidecar -> fresh Editor.
//! No native window, GPU output, or simulation stepping is claimed.
use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
#[test]
fn preview_checkbox_live_draft_applies_undo_redo_saves_reopens_default_off() {
    let dir = tempfile::tempdir().unwrap();
    let scene = dir.path().join("point.scene.yaml");
    let editor = crate::mesh_workspace::mission::prepare(
        &scene,
        &crate::mesh_workspace::mission::Settings::default(),
    )
    .unwrap();
    let checksum = editor.checksum();
    let scene_bytes = std::fs::read(&scene).unwrap();
    let history = editor.history().clone();
    let mut panel = Panel::default();
    panel.sync(&editor);
    assert!(panel.applied_point_light_settings(&editor).is_none());
    let mut h = Harness::builder()
        .with_size([1400.0, 2800.0])
        .build_ui_state(
            |ui, state: &mut (Panel, Editor)| state.0.show(ui, &state.1),
            (panel, editor),
        );
    h.run_steps(3);
    h.get_by_label("Mesh game lighting").click_accesskit();
    h.run_steps(3);
    h.get_by_label("Enable point light").click_accesskit();
    h.run_steps(3);
    assert!(h.state().0.current_document().is_none());
    assert!(h
        .state()
        .0
        .applied_point_light_settings(&h.state().1)
        .is_none());
    h.get_by_label("Point light settings").click_accesskit();
    h.run_steps(3);
    for (label, value) in [
        ("Point light position X", 2.0),
        ("Point light position Y", 5.0),
        ("Point light position Z", -1.0),
        ("Point light linear R", 0.2),
        ("Point light linear G", 0.7),
        ("Point light linear B", 0.4),
        ("Point light intensity", 7.0),
        ("Point light range", 9.0),
    ] {
        let (target_node, target_tree) = h.get_by_label(label).accesskit_node().locate();
        h.event(egui::Event::AccessKitActionRequest(
            egui::accesskit::ActionRequest {
                target_node,
                target_tree,
                action: egui::accesskit::Action::SetValue,
                data: Some(egui::accesskit::ActionData::NumericValue(value)),
            },
        ));
        h.run_steps(3);
    }
    let staged = h.state().0.point_light.unwrap();
    assert!(!h.state().0.point_light_preview_enabled());
    let dirty_before = h.state().0.dirty();
    let history_before = h.state().0.history.len();
    h.get_by_label("Preview point light draft")
        .click_accesskit();
    h.run_steps(3);
    assert!(h.state().0.point_light_preview_enabled());
    {
        let s = h.state_mut();
        assert_eq!(
            s.0.preview_point_light_settings(&s.1).unwrap().point_light,
            Some(staged)
        );
    }
    assert_eq!(h.state().0.dirty(), dirty_before);
    assert_eq!(h.state().0.history.len(), history_before);
    assert!(!crate::mesh_lighting_store::has_sidecar(&scene).unwrap());

    assert_eq!(staged.position, [2.0, 5.0, -1.0]);
    assert_eq!(staged.intensity, 7.0);
    assert_eq!(staged.range, 9.0);
    assert_eq!(h.state().1.checksum(), checksum);
    assert_eq!(h.state().1.history(), &history);
    assert_eq!(std::fs::read(&scene).unwrap(), scene_bytes);
    h.get_by_label("Apply game lighting").click_accesskit();
    h.run_steps(3);
    assert!(h.state().0.error.is_none(), "{:?}", h.state().0.error);
    assert!(
        !h.state().0.point_light_preview_enabled(),
        "Apply must retire preview"
    );
    let applied = h.state().0.current_document().unwrap().clone();
    assert_eq!(
        applied.point_light_settings().unwrap().point_light,
        Some(staged)
    );
    assert_eq!(
        h.state()
            .0
            .applied_point_light_settings(&h.state().1)
            .unwrap()
            .point_light,
        Some(staged)
    );
    h.get_by_label("Preview point light draft")
        .click_accesskit();
    h.run_steps(3);
    assert!(h.state().0.point_light_preview_enabled());
    h.get_by_label("Undo game lighting").click_accesskit();
    h.run_steps(3);
    assert!(
        !h.state().0.point_light_preview_enabled(),
        "Undo game lighting must retire preview"
    );
    assert!(h
        .state()
        .0
        .applied_point_light_settings(&h.state().1)
        .is_none());
    h.get_by_label("Preview point light draft")
        .click_accesskit();
    h.run_steps(3);
    assert!(h.state().0.point_light_preview_enabled());
    h.get_by_label("Redo game lighting").click_accesskit();
    h.run_steps(3);
    assert!(
        !h.state().0.point_light_preview_enabled(),
        "Redo game lighting must retire preview"
    );
    assert_eq!(h.state().0.current_document(), Some(&applied));
    h.get_by_label("Preview point light draft")
        .click_accesskit();
    h.run_steps(3);
    assert!(h.state().0.point_light_preview_enabled());
    h.get_by_label("Save game lighting").click_accesskit();
    h.run_steps(3);
    assert!(
        !h.state().0.point_light_preview_enabled(),
        "Save game lighting must retire preview"
    );
    assert!(!h.state().0.dirty());
    let revision = h.state().0.graph_revision.unwrap();
    let (stored, _) = crate::mesh_lighting_store::load(&scene, revision)
        .unwrap()
        .unwrap();
    assert_eq!(stored, applied);
    let fresh =
        crate::mesh_workspace::prepare(&scene, crate::mesh_workspace::Intent::Open).unwrap();
    let mut reopened = Panel::default();
    reopened.sync(&fresh);
    assert!(!reopened.point_light_preview_enabled());
    assert!(reopened.preview_point_light_settings(&fresh).is_none());

    assert_eq!(reopened.current_document(), Some(&applied));
    assert_eq!(
        reopened
            .applied_point_light_settings(&fresh)
            .unwrap()
            .point_light,
        Some(staged)
    );
    assert_eq!(fresh.checksum(), checksum);
    reopened.context_stale = true;
    assert!(reopened.applied_point_light_settings(&fresh).is_none());
    reopened.context_stale = false;
    reopened.sidecar_error = Some("invalid source".into());
    assert!(reopened.applied_point_light_settings(&fresh).is_none());
    reopened.sidecar_error = None;
    reopened.document_pending = true;
    assert!(reopened.applied_point_light_settings(&fresh).is_none());
    assert_eq!(std::fs::read(&scene).unwrap(), scene_bytes);
    assert_eq!(h.state().1.checksum(), checksum);
    assert_eq!(h.state().1.history(), &history);
}
