use super::{capture_document, parse_number, HistoryEntry, Panel, ProjectionMode, MAX_HISTORY};
use crate::mesh_camera::{Document, Projection};
use orr_fp::FP;
use orr_render::camera3d::OrbitCamera;
use std::path::Path;

fn document(target: [f32; 3], yaw: f32) -> Document {
    Document::from_orbit(OrbitCamera::new(target, yaw, 0.4, 12.0)).unwrap()
}

#[test]
fn camera_form_parses_degrees_and_validates_projection_before_application() {
    let mut panel = Panel::default();
    panel.target = ["1.25".into(), "2".into(), "-3".into()];
    panel.yaw_degrees = "90".into();
    panel.pitch_degrees = "-20".into();
    panel.distance = "15".into();
    panel.fov_y_degrees = "65".into();
    panel.near = "0.2".into();
    panel.far = "800".into();
    panel.projection = ProjectionMode::Orthographic;
    panel.half_height = "12".into();
    let document = panel.parse_form(Path::new("scene.yaml"), [7; 32]).unwrap();
    assert_eq!(document.target, [1.25, 2.0, -3.0]);
    assert!((document.yaw.to_degrees() - 90.0).abs() < 1.0e-5);
    assert!((document.pitch.to_degrees() + 20.0).abs() < 1.0e-5);
    assert_eq!(
        document.projection,
        Projection::Orthographic { half_height: 12.0 }
    );

    panel.fov_y_degrees = "NaN".into();
    assert!(panel.parse_form(Path::new("scene.yaml"), [7; 32]).is_err());
    panel.fov_y_degrees = "65".into();
    panel.distance = "0.1".into();
    assert!(panel.parse_form(Path::new("scene.yaml"), [7; 32]).is_err());
    assert!(parse_number(&"1".repeat(49), "target").is_err());
}

#[test]
fn follow_form_is_applied_only_as_a_validated_offset() {
    let mut panel = Panel::default();
    panel.follow_agent = true;
    panel.follow_offset = ["1.5".into(), "2".into(), "-3".into()];
    let document = panel.parse_form(Path::new("scene.yaml"), [7; 32]).unwrap();
    assert_eq!(document.follow_offset, Some([1.5, 2.0, -3.0]));

    panel.follow_offset[1] = "NaN".into();
    assert!(panel.parse_form(Path::new("scene.yaml"), [7; 32]).is_err());
    panel.follow_offset[1] = "2".into();
    panel.follow_offset[2] = "1".repeat(49);
    assert!(panel.parse_form(Path::new("scene.yaml"), [7; 32]).is_err());

    panel.follow_agent = false;
    let fixed = panel.parse_form(Path::new("scene.yaml"), [7; 32]).unwrap();
    assert_eq!(fixed.follow_offset, None);
}

#[test]
fn history_tracks_saved_camera_and_bounds_undo_depth() {
    let mut panel = Panel::default();
    let original = document([0.0, 2.0, 0.0], 0.4);
    panel.reset_history(Some(original.clone()), Some(0));
    assert!(!panel.dirty());

    let changed = document([1.0, 2.0, 3.0], 0.8);
    panel.push_state(Some(changed.clone()));
    assert!(panel.dirty());
    panel.cursor -= 1;
    assert!(!panel.dirty());
    panel.cursor += 1;
    assert!(panel.dirty());
    panel.saved_id = panel.history[panel.cursor].id.into();
    assert!(!panel.history_is_dirty());

    for index in 0..MAX_HISTORY + 2 {
        panel.push_state(Some(document(
            [index as f32, 2.0, 3.0],
            0.8 + index as f32 * 0.01,
        )));
    }
    assert_eq!(panel.history.len(), MAX_HISTORY);
    assert!(panel.dirty());
}

#[test]
fn form_and_context_changes_are_dirty_without_mutating_saved_history() {
    let mut panel = Panel::default();
    let saved = document([0.0, 2.0, 0.0], 0.4);
    panel.reset_history(Some(saved.clone()), Some(0));
    panel.form_dirty = true;
    assert!(panel.dirty());
    assert_eq!(panel.current_document(), Some(&saved));

    panel.form_dirty = false;
    panel.context_stale = true;
    assert!(panel.dirty());
    assert_eq!(panel.current_document(), Some(&saved));
}

#[test]
fn no_op_history_push_does_not_create_an_undo_entry() {
    let mut panel = Panel::default();
    let saved = document([0.0, 2.0, 0.0], 0.4);
    panel.reset_history(Some(saved.clone()), Some(0));
    panel.push_state(Some(saved));
    assert_eq!(panel.history.len(), 1);
    assert_eq!(panel.cursor, 0);
}

#[test]
fn reset_history_starts_with_a_single_saved_entry() {
    let mut panel = Panel::default();
    panel.history.push(HistoryEntry {
        id: 1,
        document: Some(document([1.0, 0.0, 0.0], 0.2)),
    });
    panel.reset_history(None, Some(0));
    assert_eq!(panel.history.len(), 1);
    assert_eq!(panel.cursor, 0);
    assert_eq!(panel.saved_id, Some(0));
    assert!(panel.current_document().is_none());
}

#[test]
fn capturing_after_orthographic_zoom_preserves_view_framing() {
    let saved = Document::from_orbit(OrbitCamera::new([1.0, 2.0, 3.0], 0.4, 0.3, 16.0))
        .unwrap()
        .with_projection(Projection::Orthographic { half_height: 7.0 }, 0.2, 600.0)
        .unwrap();
    let mut zoomed_orbit = saved.orbit();
    zoomed_orbit.distance = 8.0;

    let before_capture = saved.camera(&zoomed_orbit, (1280, 720)).unwrap();
    let captured = capture_document(zoomed_orbit, Some(&saved)).unwrap();
    let after_capture = captured.camera(&zoomed_orbit, (1280, 720)).unwrap();

    assert_eq!(after_capture.eye, before_capture.eye);
    assert_eq!(after_capture.target, before_capture.target);
    assert_eq!(after_capture.projection, before_capture.projection);
}

#[test]
fn capturing_viewport_preserves_the_follow_offset_setting() {
    let saved = Document::from_orbit(OrbitCamera::new([1.0, 2.0, 3.0], 0.4, 0.3, 16.0))
        .unwrap()
        .with_follow_offset(Some([0.5, 1.0, -2.0]))
        .unwrap();
    let captured = capture_document(saved.orbit(), Some(&saved)).unwrap();
    assert_eq!(captured.follow_offset, saved.follow_offset);
}

#[test]
fn capturing_a_panned_follow_camera_preserves_its_effective_view() {
    use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
    use orr_remote::mesh_navigation::{authoring::create_mesh_scene, mesh_from_view, AgentSpec};

    let saved = Document::from_orbit(OrbitCamera::new([1.0, 2.0, 3.0], 0.4, 0.3, 16.0))
        .unwrap()
        .with_projection(Projection::Orthographic { half_height: 7.0 }, 0.2, 600.0)
        .unwrap()
        .with_follow_offset(Some([0.5, 1.0, -2.0]))
        .unwrap();
    let panned_orbit = OrbitCamera::new([3.0, 4.0, 8.0], 0.4, 0.3, 16.0);
    let graph = StaticMeshGraph::bake(
        &[
            [FP::from_int(0), FP::ZERO, FP::from_int(0)],
            [FP::from_int(0), FP::ZERO, FP::from_int(1)],
            [FP::from_int(1), FP::ZERO, FP::from_int(0)],
            [FP::from_int(1), FP::ZERO, FP::from_int(1)],
        ],
        &[[0, 1, 3], [0, 3, 2]],
        AgentProfile::default(),
    )
    .unwrap();
    let spec = AgentSpec {
        start: [FP::ZERO; 3],
        goal: [FP::from_int(1), FP::ZERO, FP::from_int(1)],
        max_vertical: FP::ZERO,
        distance_per_tick: FP::ONE,
    };
    let document = create_mesh_scene(&graph, spec).unwrap();
    let view = mesh_from_view(orr_bridge::FrameView::of(document.frame())).unwrap();

    let before_capture = saved
        .camera_for_view(&panned_orbit, (1280, 720), &view)
        .unwrap();
    let captured = capture_document(panned_orbit, Some(&saved)).unwrap();
    let after_capture = captured
        .camera_for_view(&panned_orbit, (1280, 720), &view)
        .unwrap();

    assert_eq!(captured.follow_offset, Some([2.5, 3.0, 3.0]));
    assert_eq!(after_capture, before_capture);
    assert_eq!(after_capture.view(), before_capture.view());
}
