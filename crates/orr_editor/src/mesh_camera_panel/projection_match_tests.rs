//! UI contract tests: conversion edits the form, never applies or saves it.
#![cfg(feature = "mesh-mission")]
use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use std::fs;

const SWITCH: &str = "Switch projection keeping target-plane scale";

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
fn projection_match_button_changes_form_only_until_explicit_apply_and_save() {
    let (_directory, scene, editor, panel) = fixture();
    let scene_bytes = fs::read(&scene).unwrap();
    let sidecar = crate::mesh_camera_store::sidecar_path(&scene).unwrap();
    let editor_history = editor.history().clone();
    let checksum = editor.checksum();
    let orbit = editor.camera3d;
    let camera_history = history(&panel);
    let mut harness = Harness::builder()
        .with_size([1200.0, 1800.0])
        .build_ui_state(
            |ui, state: &mut (Panel, Editor)| state.0.show(ui, &mut state.1),
            (panel, editor),
        );
    harness.run_steps(2);
    harness.get_by_label("Mesh game camera").click();
    harness.run_steps(3);
    harness.get_by_label(SWITCH).click();
    harness.run_steps(3);
    let (panel, editor) = harness.state();
    assert!(panel.error.is_none(), "{:?}", panel.error);
    assert_eq!(panel.projection, ProjectionMode::Orthographic);
    assert!(panel.form_dirty);
    assert_eq!(history(panel), camera_history);
    assert!(!editor.has_mesh_camera());
    assert_eq!(editor.camera3d, orbit);
    assert_eq!(editor.history(), &editor_history);
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    assert!(!sidecar.exists());
    assert!(panel.play_blocker(editor).is_some());
    assert!(harness
        .get_by_label("Save game camera")
        .accesskit_node()
        .is_disabled());

    harness.get_by_label("Apply game camera").click();
    harness.run_steps(3);
    let (panel, editor) = harness.state();
    assert!(panel.error.is_none(), "{:?}", panel.error);
    assert!(!panel.form_dirty);
    assert!(editor.has_mesh_camera());
    assert!(matches!(
        panel.current_document().unwrap().projection,
        Projection::Orthographic { .. }
    ));
    assert!(panel.history_is_dirty());
    assert_eq!(panel.history.len(), camera_history.0.len() + 1);
    assert!(!sidecar.exists());
    assert_eq!(editor.history(), &editor_history);
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);

    harness.get_by_label("Save game camera").click();
    harness.run_steps(3);
    let (panel, editor) = harness.state();
    assert!(panel.error.is_none(), "{:?}", panel.error);
    assert!(!panel.dirty());
    assert!(sidecar.exists());
    assert_eq!(editor.history(), &editor_history);
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
}

#[test]
fn invalid_dimensions_and_invalid_form_leave_every_field_and_history_unchanged() {
    let (_directory, scene, editor, mut panel) = fixture();
    let scene_bytes = fs::read(&scene).unwrap();
    let sidecar = crate::mesh_camera_store::sidecar_path(&scene).unwrap();
    let editor_history = editor.history().clone();
    let checksum = editor.checksum();
    let initial = form(&panel);
    for (width, height) in [
        ("0", "9"),
        ("16", "0"),
        ("16385", "9"),
        ("16", "16385"),
        ("NaN", "9"),
        ("16", "1.5"),
        ("-1", "9"),
        ("", "9"),
    ] {
        panel.match_width = width.into();
        panel.match_height = height.into();
        let before = form(&panel);
        let before_history = history(&panel);
        panel.match_projection(&editor);
        assert!(panel.error.is_some(), "{width}x{height}");
        assert_eq!(form(&panel), before);
        assert_eq!(history(&panel), before_history);
    }
    panel.match_width = "16".into();
    panel.match_height = "9".into();
    assert_eq!(form(&panel), initial);
    for value in ["NaN", "0", "bad"] {
        panel.distance = value.into();
        let before = form(&panel);
        let before_history = history(&panel);
        panel.match_projection(&editor);
        assert!(panel.error.is_some());
        assert_eq!(form(&panel), before);
        assert_eq!(history(&panel), before_history);
    }
    assert!(!editor.has_mesh_camera());
    assert_eq!(editor.history(), &editor_history);
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    assert!(!sidecar.exists());
}

#[test]
fn stale_and_play_contexts_reject_direct_conversion_and_disable_the_button() {
    for play in [false, true] {
        let (_directory, scene, mut editor, mut panel) = fixture();
        let scene_bytes = fs::read(&scene).unwrap();
        if play {
            assert!(editor.start_play());
            assert_eq!(editor.mode(), Mode::Play);
        } else {
            panel.context_stale = true;
        }
        let before = form(&panel);
        let before_history = history(&panel);
        let editor_history = editor.history().clone();
        panel.match_projection(&editor);
        assert!(panel.error.is_some());
        assert_eq!(form(&panel), before);
        assert_eq!(history(&panel), before_history);
        let mut harness = Harness::builder()
            .with_size([1200.0, 1800.0])
            .build_ui_state(
                |ui, state: &mut (Panel, Editor)| state.0.show(ui, &mut state.1),
                (panel, editor),
            );
        harness.run_steps(2);
        harness.get_by_label("Mesh game camera").click();
        harness.run_steps(3);
        assert!(harness.get_by_label(SWITCH).accesskit_node().is_disabled());
        harness.get_by_label(SWITCH).click_accesskit();
        harness.run_steps(2);
        let (panel, editor) = harness.state();
        assert_eq!(form(panel), before);
        assert_eq!(history(panel), before_history);
        assert_eq!(editor.history(), &editor_history);
        assert!(!editor.has_mesh_camera());
        assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
        assert!(!crate::mesh_camera_store::sidecar_path(&scene)
            .unwrap()
            .exists());
    }
}
