use super::{HistoryEntry, MAX_HISTORY, Panel, ProjectionMode, capture_document, parse_number};
use crate::mesh_camera::{Document, Projection};
use orr_fp::FP;
use orr_render::camera3d::OrbitCamera;
use std::path::Path;

#[cfg(feature = "mesh-mission")]
#[test]
fn workspace_save_is_explicit_and_preserves_external_conflicts() {
    let directory = tempfile::tempdir().unwrap();
    let scene = directory.path().join("camera-workspace.scene.yaml");
    let mut editor = crate::mesh_workspace::mission::prepare(
        &scene,
        &crate::mesh_workspace::mission::Settings::default(),
    )
    .unwrap();
    let mut panel = Panel::default();
    panel.sync(&mut editor);
    let sidecar = crate::mesh_camera_store::sidecar_path(&scene).unwrap();

    assert!(!panel.save_workspace_changes(&editor).unwrap());
    assert!(!sidecar.exists());

    panel.form_dirty = true;
    assert!(panel.workspace_save_blocker(&editor).is_some());
    assert!(panel.save_workspace_changes(&editor).is_err());
    assert!(!sidecar.exists());
    panel.form_dirty = false;

    let initial = document([1.0, 2.0, 3.0], 0.4);
    panel.push_initial(Some(initial.clone()));
    panel.push_state(Some(
        initial
            .clone()
            .with_follow_offset(Some([1.0, 0.0, 0.0]))
            .unwrap(),
    ));
    assert!(panel.save_workspace_changes(&editor).unwrap());
    assert!(!panel.dirty());
    let saved_bytes = std::fs::read(&sidecar).unwrap();
    assert!(!saved_bytes.is_empty());

    panel.push_state(Some(document([3.0, 2.0, 1.0], 0.8)));
    let external_bytes = b"external camera sidecar update";
    std::fs::write(&sidecar, external_bytes).unwrap();
    assert!(panel.save_workspace_changes(&editor).is_err());
    assert!(panel.dirty());
    assert_eq!(std::fs::read(&sidecar).unwrap(), external_bytes);
}

fn document(target: [f32; 3], yaw: f32) -> Document {
    Document::from_orbit(OrbitCamera::new(target, yaw, 0.4, 12.0)).unwrap()
}

#[test]
fn camera_form_parses_degrees_and_validates_projection_before_application() {
    let mut panel = Panel {
        target: ["1.25".into(), "2".into(), "-3".into()],
        yaw_degrees: "90".into(),
        pitch_degrees: "-20".into(),
        distance: "15".into(),
        fov_y_degrees: "65".into(),
        near: "0.2".into(),
        far: "800".into(),
        projection: ProjectionMode::Orthographic,
        half_height: "12".into(),
        ..Default::default()
    };
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
    let mut panel = Panel {
        follow_agent: true,
        follow_offset: ["1.5".into(), "2".into(), "-3".into()],
        ..Default::default()
    };
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
fn saved_sidecar_binding_matches_exact_optional_digest_only_when_admitted_cleanly() {
    let mut panel = Panel::default();
    panel.document_pending = false;
    panel.reset_history(None, Some(0));
    assert!(panel.matches_saved_sidecar(None));
    assert!(!panel.matches_saved_sidecar(Some("camera-digest")));

    panel.sidecar_sha = Some("camera-digest".into());
    assert!(panel.matches_saved_sidecar(Some("camera-digest")));
    assert!(!panel.matches_saved_sidecar(None));
    assert!(!panel.matches_saved_sidecar(Some("another-digest")));

    panel.form_dirty = true;
    assert!(!panel.matches_saved_sidecar(Some("camera-digest")));
    panel.form_dirty = false;
    panel.sidecar_error = Some("invalid sidecar".into());
    assert!(!panel.matches_saved_sidecar(Some("camera-digest")));
    panel.sidecar_error = None;
    panel.document_pending = true;
    assert!(!panel.matches_saved_sidecar(Some("camera-digest")));
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
    use orr_navigation::{AgentProfile, mesh::StaticMeshGraph};
    use orr_remote::mesh_navigation::{AgentSpec, authoring::create_mesh_scene, mesh_from_view};

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

#[cfg(feature = "mesh-hazards")]
#[test]
fn hazards_profile_camera_capture_history_save_reopen_play_and_restart() {
    use crate::{game::EditorGame, mesh_workspace::mission::Settings};

    let directory = tempfile::tempdir().unwrap();
    let scene = directory.path().join("camera-hazards.scene.yaml");
    let mut editor =
        crate::mesh_hazards_creator::prepare(&scene, &Settings::default(), true).unwrap();
    let mut panel = Panel::default();
    panel.sync(&mut editor);

    assert_eq!(editor.game(), EditorGame::MeshHazardsV1);
    assert_eq!(panel.profile, Some(EditorGame::MeshHazardsV1));
    assert!(panel.can_edit(&editor));
    assert!(panel.current_document().is_none());

    panel.capture_viewport(&editor);
    assert!(panel.form_dirty);
    panel.apply(&mut editor);
    assert!(panel.error.is_none(), "{:?}", panel.error);
    let captured = panel.current_document().unwrap().clone();
    assert!(panel.history_is_dirty());
    assert!(panel.play_blocker(&editor).is_some());

    panel.save(&editor);
    assert!(panel.error.is_none(), "{:?}", panel.error);
    assert!(!panel.dirty());
    assert!(panel.play_blocker(&editor).is_none());

    panel.target[0] = "3".into();
    panel.form_dirty = true;
    panel.apply(&mut editor);
    assert!(panel.error.is_none(), "{:?}", panel.error);
    assert!(panel.dirty());
    panel.undo(&mut editor);
    assert!(!panel.dirty());
    assert_eq!(panel.current_document(), Some(&captured));
    panel.redo(&mut editor);
    assert!(panel.dirty());
    panel.save(&editor);
    assert!(!panel.dirty());

    // A clean restart changes the scene-load epoch and reloads the authored
    // camera from its sidecar rather than carrying panel history across hosts.
    assert!(editor.start_play());
    assert!(panel.play_blocker(&editor).is_none());
    assert!(editor.restart());
    panel.sync(&mut editor);
    assert_eq!(panel.profile, Some(EditorGame::MeshHazardsV1));
    assert_eq!(panel.current_document().unwrap().target[0], 3.0);
    assert!(!panel.dirty());

    drop(editor);
    let mut reopened = crate::editor::Editor::open_game(&scene, EditorGame::MeshHazardsV1).unwrap();
    reopened.sync();
    let mut reopened_panel = Panel::default();
    reopened_panel.sync(&mut reopened);
    assert_eq!(reopened_panel.current_document(), panel.current_document());
    assert_eq!(reopened_panel.profile, Some(EditorGame::MeshHazardsV1));
    assert!(reopened_panel.play_blocker(&reopened).is_none());
}

#[cfg(feature = "mesh-hazards")]
#[test]
fn profile_switch_marks_dirty_camera_stale_even_for_same_scene_epoch_and_graph() {
    use crate::{game::EditorGame, mesh_workspace::mission::Settings};

    let directory = tempfile::tempdir().unwrap();
    let scene = directory.path().join("camera-profile-switch.scene.yaml");
    let mut editor =
        crate::mesh_hazards_creator::prepare(&scene, &Settings::default(), true).unwrap();
    let mut panel = Panel::default();
    panel.sync(&mut editor);
    let scene_epoch = panel.scene_epoch;
    let graph_revision = panel.graph_revision;
    panel.profile = Some(EditorGame::MeshNavigation);
    panel.form_dirty = true;

    panel.sync(&mut editor);

    assert_eq!(panel.scene_epoch, scene_epoch);
    assert_eq!(panel.graph_revision, graph_revision);
    assert_eq!(panel.profile, Some(EditorGame::MeshNavigation));
    assert!(panel.context_stale);
    assert!(panel.dirty());
    assert!(panel.play_blocker(&editor).is_some());

    panel.confirm_discard_changes = true;
    panel.discard_changes_and_reload(&mut editor);
    assert!(!panel.context_stale);
    assert_eq!(panel.profile, Some(EditorGame::MeshHazardsV1));
    assert!(!panel.dirty());
    assert!(panel.play_blocker(&editor).is_none());
}
