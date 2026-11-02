//! UI contract tests: conversion edits the form, never applies or saves it.
#![cfg(feature = "mesh-mission")]
use super::super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use std::fs;

const AIM: &str = "Aim target at mesh agent";

fn fixture() -> (tempfile::TempDir, PathBuf, Editor, Panel) {
    let directory = tempfile::tempdir().unwrap();
    let scene = directory.path().join("projection-match.scene.yaml");
    let mut editor = crate::mesh_workspace::mission::prepare(
        &scene,
        &crate::mesh_workspace::mission::Settings::default(),
    )
    .unwrap();
    let mut panel = Panel::default();
    panel.sync(&mut editor);
    assert!(panel.can_edit(&editor));
    assert_eq!(panel.match_width, "16");
    assert_eq!(panel.match_height, "9");
    (directory, scene, editor, panel)
}

fn form(panel: &Panel) -> (Vec<String>, ProjectionMode, bool, bool) {
    let mut fields = panel.target.to_vec();
    fields.extend(panel.follow_offset.iter().cloned());
    fields.extend([
        panel.yaw_degrees.clone(),
        panel.pitch_degrees.clone(),
        panel.distance.clone(),
        panel.fov_y_degrees.clone(),
        panel.near.clone(),
        panel.far.clone(),
        panel.half_height.clone(),
        panel.match_width.clone(),
        panel.match_height.clone(),
    ]);
    (
        fields,
        panel.projection,
        panel.form_dirty,
        panel.follow_agent,
    )
}

type CameraHistory = (Vec<(u64, Option<Document>)>, usize, u64, Option<u64>);

fn history(panel: &Panel) -> CameraHistory {
    (
        panel
            .history
            .iter()
            .map(|entry| (entry.id, entry.document.clone()))
            .collect(),
        panel.cursor,
        panel.next_id,
        panel.saved_id,
    )
}

#[test]
fn actual_aim_changes_only_target_until_apply_then_undo_redo_save_reopen() {
    let (_temp, scene, editor, mut panel) = fixture();
    panel.target = ["123".into(), "45".into(), "-67".into()];
    let agent = mesh_from_view(editor.snapshot().unwrap().predicted())
        .unwrap()
        .agent
        .position
        .map(orr_fp::FP::to_f32);
    let expected = agent.map(format_number);
    assert_ne!(panel.target, expected);
    let before = form(&panel);
    let before_history = history(&panel);
    let scene_bytes = fs::read(&scene).unwrap();
    let editor_history = editor.history().clone();
    let checksum = editor.checksum();
    let viewport = editor.camera3d;
    let sidecar = crate::mesh_camera_store::sidecar_path(&scene).unwrap();
    let mut h = Harness::builder()
        .with_size([1400.0, 2000.0])
        .build_ui_state(
            |ui, state: &mut (Panel, Editor)| state.0.show(ui, &mut state.1),
            (panel, editor),
        );
    h.run_steps(2);
    h.get_by_label("Mesh game camera").click();
    h.run_steps(3);
    h.get_by_label(AIM).click();
    h.run_steps(3);
    let (panel, editor) = h.state();
    assert!(panel.error.is_none(), "{:?}", panel.error);
    assert_eq!(panel.target, expected);
    assert_eq!(&form(panel).0[3..], &before.0[3..]);
    assert_eq!(panel.projection, before.1);
    assert_eq!(panel.follow_agent, before.3);
    assert!(panel.form_dirty);
    assert_eq!(history(panel), before_history);
    assert_eq!(editor.camera3d, viewport);
    assert_eq!(editor.history(), &editor_history);
    assert_eq!(editor.checksum(), checksum);
    assert!(!sidecar.exists());
    assert!(!editor.has_mesh_camera());
    h.get_by_label("Apply game camera").click();
    h.run_steps(3);
    let applied = h.state().0.current_document().unwrap().clone();
    assert_eq!(applied.target, agent);
    assert!(!sidecar.exists());
    assert!(h.state().1.has_mesh_camera());
    h.get_by_label("Undo game camera").click();
    h.run_steps(3);
    assert!(h.state().0.current_document().is_none());
    h.get_by_label("Redo game camera").click();
    h.run_steps(3);
    assert_eq!(h.state().0.current_document(), Some(&applied));
    h.get_by_label("Save game camera").click();
    h.run_steps(3);
    assert!(sidecar.exists());
    assert!(!h.state().0.history_is_dirty());
    let (panel, editor) = h.state_mut();
    *panel = Panel::default();
    panel.sync(editor);
    assert_eq!(panel.current_document(), Some(&applied));
    assert_eq!(fs::read(scene).unwrap(), scene_bytes);
    assert_eq!(editor.history(), &editor_history);
    assert_eq!(editor.checksum(), checksum);
}

#[test]
fn follow_play_stale_and_pending_context_refuse_aim_atomically() {
    for reason in ["follow", "play", "stale", "pending"] {
        let (_temp, scene, mut editor, mut panel) = fixture();
        match reason {
            "follow" => panel.follow_agent = true,
            "play" => assert!(editor.start_play()),
            "stale" => panel.context_stale = true,
            _ => panel.document_pending = true,
        }
        let before = form(&panel);
        let before_history = history(&panel);
        let viewport = editor.camera3d;
        panel.aim_at_agent(&editor);
        assert!(panel
            .error
            .as_deref()
            .unwrap()
            .starts_with("Aim camera target failed:"));
        assert_eq!(form(&panel), before);
        assert_eq!(history(&panel), before_history);
        assert_eq!(editor.camera3d, viewport);
        assert!(!crate::mesh_camera_store::sidecar_path(&scene)
            .unwrap()
            .exists());
    }
}

#[test]
fn actual_follow_disables_aim_without_changing_form() {
    let (_temp, _scene, editor, mut panel) = fixture();
    panel.follow_agent = true;
    let before = form(&panel);
    let mut h = Harness::builder()
        .with_size([1400.0, 2000.0])
        .build_ui_state(
            |ui, state: &mut (Panel, Editor)| state.0.show(ui, &mut state.1),
            (panel, editor),
        );
    h.run_steps(2);
    h.get_by_label("Mesh game camera").click();
    h.run_steps(3);
    assert!(h.get_by_label(AIM).accesskit_node().is_disabled());
    h.get_by_label(AIM).click_accesskit();
    h.run_steps(3);
    assert_eq!(form(&h.state().0), before);
    assert!(!h.state().1.has_mesh_camera());
}

#[test]
fn same_frame_form_action_then_aim_keeps_new_error_or_converted_projection() {
    let (_temp, _scene, editor, mut panel) = fixture();
    panel.follow_agent = true;
    panel.handle_form_actions(true, false, &editor);
    panel.aim_at_agent(&editor);
    assert!(panel.error.as_deref().unwrap().contains("Turn Follow"));
    panel.follow_agent = false;
    panel.error = Some("old error".into());
    panel.handle_form_actions(true, true, &editor);
    assert_eq!(panel.projection, ProjectionMode::Orthographic);
    let half_height = panel.half_height.clone();
    let before_history = history(&panel);
    panel.aim_at_agent(&editor);
    assert!(panel.error.is_none());
    assert_eq!(panel.projection, ProjectionMode::Orthographic);
    assert_eq!(panel.half_height, half_height);
    assert_eq!(history(&panel), before_history);
    assert!(!editor.has_mesh_camera());
    assert!(panel.form_dirty);
    // Aim repairs only the target and deliberately does not rewrite unrelated invalid draft text.
    panel.distance = "invalid distance".into();
    panel.aim_at_agent(&editor);
    assert!(panel.error.is_none());
    assert_eq!(panel.distance, "invalid distance");
}

#[test]
fn projection_request_frame_disables_aim_and_preserves_conversion_error() {
    let (_temp, _scene, editor, mut panel) = fixture();
    panel.match_width = "invalid aspect".into();
    panel.handle_form_actions(true, true, &editor);
    let error = panel.error.clone();
    assert!(error.is_some());
    let before = form(&panel);
    let mut h = Harness::builder()
        .with_size([1200.0, 400.0])
        .build_ui_state(
            |ui, state: &mut (Panel, Editor)| state.0.show_focus_agent(ui, &state.1, true),
            (panel, editor),
        );
    h.run_steps(2);
    assert!(h.get_by_label(AIM).accesskit_node().is_disabled());
    h.get_by_label(AIM).click_accesskit();
    h.run_steps(2);
    assert_eq!(h.state().0.error, error);
    assert_eq!(form(&h.state().0), before);
}
