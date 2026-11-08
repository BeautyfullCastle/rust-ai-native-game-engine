//! Actual CPU egui authoring -> applied history -> saved sidecar -> fresh Editor.
//! The production CPU pick API receives a synthetic projected pixel after the real Pick widget.
//! No native window, OS input, GPU output, or simulation stepping is claimed.
use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
#[test]
fn viewport_pick_stages_xz_preserves_height_then_applies_undo_redo_saves_reopens() {
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
    let before_pick = h.state().0.point_light.unwrap();
    h.get_by_label("Pick point light X/Z").click_accesskit();
    h.run_steps(3);
    assert!(h.state().0.pick_pending());
    let camera = orr_render::Camera3D {
        eye: [0.0, 10.0, 10.0],
        target: [0.0; 3],
        up: [0.0, 1.0, 0.0],
        projection: orr_render::Projection::Perspective {
            fov_y: 60.0_f32.to_radians(),
            near: 0.1,
            far: 500.0,
        },
    };
    let size = (800, 600);
    let pixel = camera.world_to_screen([3.0, 0.0, -2.0], size).unwrap();
    {
        let state = h.state_mut();
        assert!(state
            .0
            .handle_viewport_pick(&state.1, &camera, size, Some(pixel), true, false));
    }
    h.run_steps(3);
    assert!(!h.state().0.pick_pending());
    let staged = h.state().0.point_light.unwrap();
    assert!((staged.position[0] - 3.0).abs() < 0.0001);
    assert!((staged.position[2] + 2.0).abs() < 0.0001);
    assert_eq!(staged.position[1], before_pick.position[1]);
    assert_eq!(staged.color, before_pick.color);
    assert_eq!(staged.intensity, before_pick.intensity);
    assert_eq!(staged.range, before_pick.range);
    assert!(h.state().0.current_document().is_none());
    assert_eq!(h.state().0.history.len(), 1);
    assert_eq!(h.state().1.checksum(), checksum);
    assert_eq!(h.state().1.history(), &history);
    assert_eq!(std::fs::read(&scene).unwrap(), scene_bytes);
    h.get_by_label("Apply game lighting").click_accesskit();
    h.run_steps(3);
    assert!(h.state().0.error.is_none(), "{:?}", h.state().0.error);
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
    h.get_by_label("Undo game lighting").click_accesskit();
    h.run_steps(3);
    assert!(h
        .state()
        .0
        .applied_point_light_settings(&h.state().1)
        .is_none());
    h.get_by_label("Redo game lighting").click_accesskit();
    h.run_steps(3);
    assert_eq!(h.state().0.current_document(), Some(&applied));
    h.get_by_label("Save game lighting").click_accesskit();
    h.run_steps(3);
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
    assert_eq!(reopened.current_document(), Some(&applied));
    assert_eq!(
        reopened
            .applied_point_light_settings(&fresh)
            .unwrap()
            .point_light,
        Some(staged)
    );
    assert_eq!(fresh.checksum(), checksum);
    // A concurrent writer after Begin must retire this draft-only pick at hit time.
    reopened.begin_viewport_pick(&fresh).unwrap();
    let before_external = reopened.point_light;
    let before_history = reopened.history.len();
    let sidecar = crate::mesh_lighting_store::sidecar_path(&scene).unwrap();
    let mut external_bytes = std::fs::read(&sidecar).unwrap();
    external_bytes.push(b'\n');
    std::fs::write(&sidecar, &external_bytes).unwrap();
    assert!(reopened.handle_viewport_pick(&fresh, &camera, size, Some(pixel), true, false));
    assert!(!reopened.pick_pending());
    assert_eq!(reopened.point_light, before_external);
    assert_eq!(reopened.history.len(), before_history);
    assert_eq!(reopened.current_document(), Some(&applied));
    assert!(reopened.error.as_deref().unwrap().contains("changed"));
    assert_eq!(std::fs::read(&sidecar).unwrap(), external_bytes);
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
