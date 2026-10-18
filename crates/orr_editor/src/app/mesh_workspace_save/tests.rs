use super::EditorApp;
use crate::{mesh_workspace::mission::Settings, Mode};
use orr_fp::fp;
use orr_remote::mesh_navigation::{mesh_from_view, mission};
use std::{fs, path::PathBuf};

fn workspace() -> (tempfile::TempDir, PathBuf, EditorApp) {
    let directory = tempfile::tempdir().unwrap();
    let scene = directory.path().join("workspace.scene.yaml");
    let editor = crate::mesh_workspace::mission::prepare(&scene, &Settings::default()).unwrap();
    let mut app = EditorApp::new(editor, None);
    app.sync_mesh_presentation();
    (directory, scene, app)
}

fn edit_exit_radius(app: &mut EditorApp) {
    let snapshot = app.editor.snapshot().unwrap();
    let view = mesh_from_view(snapshot.predicted()).unwrap();
    let mut config = mission::from_view(snapshot.predicted())
        .unwrap()
        .unwrap()
        .config;
    config.exit.radius = fp!(0.75);
    let request = mission::patch_request(&config, &view).unwrap();
    app.editor
        .host_call("world.singleton.patch", request)
        .unwrap();
    app.editor.mesh_navigation_mark_edited();
    app.editor.sync();
    app.sync_mesh_presentation();
    assert!(app.editor.is_dirty());
}

fn assert_optional_sidecars_absent(scene: &std::path::Path) {
    for suffix in [
        ".mesh-art.json",
        ".mesh-props.json",
        ".mesh-camera.json",
        ".mesh-lighting.json",
        ".mesh-hud.json",
        ".mesh-animation.json",
    ] {
        let name = format!("{}{}", scene.file_name().unwrap().to_str().unwrap(), suffix);
        assert!(!scene.with_file_name(name).exists());
    }
}

#[test]
fn clean_workspace_save_keeps_optional_sidecars_absent_and_history_unchanged() {
    let (_directory, scene, mut app) = workspace();
    let checksum = app.editor.checksum();
    let history = app.editor.history().clone();
    let bytes = fs::read(&scene).unwrap();
    assert!(app.mesh_workspace_save_blocker().is_none());
    assert!(app.save_mesh_workspace());
    assert_eq!(app.editor.checksum(), checksum);
    assert_eq!(app.editor.history(), &history);
    assert_eq!(fs::read(&scene).unwrap(), bytes);
    assert_optional_sidecars_absent(&scene);
}

#[test]
fn applied_scene_change_saves_with_its_mission_without_creating_sidecars() {
    let (_directory, scene, mut app) = workspace();
    let old_bytes = fs::read(&scene).unwrap();
    edit_exit_radius(&mut app);
    let checksum = app.editor.checksum();
    let history_len = app.editor.history().entries.len();
    assert!(app.save_mesh_workspace());
    assert!(!app.editor.is_dirty());
    assert_eq!(app.editor.checksum(), checksum);
    assert_eq!(app.editor.history().entries.len(), history_len);
    assert_ne!(fs::read(&scene).unwrap(), old_bytes);
    let reopened =
        crate::mesh_workspace::prepare(&scene, crate::mesh_workspace::Intent::Open).unwrap();
    assert_eq!(reopened.checksum(), checksum);
    assert_optional_sidecars_absent(&scene);
}

#[test]
fn unapplied_geometry_form_blocks_every_write_and_preserves_dirty_scene() {
    let (_directory, scene, mut app) = workspace();
    let old_bytes = fs::read(&scene).unwrap();
    edit_exit_radius(&mut app);
    app.mesh_navigation.start[0] = "not-a-number".into();
    let checksum = app.editor.checksum();
    let history = app.editor.history().clone();
    assert!(app.mesh_workspace_save_blocker().is_some());
    assert!(!app.save_mesh_workspace());
    assert!(app.editor.is_dirty());
    assert_eq!(app.editor.checksum(), checksum);
    assert_eq!(app.editor.history(), &history);
    assert_eq!(fs::read(&scene).unwrap(), old_bytes);
    assert_optional_sidecars_absent(&scene);
}

#[test]
fn invalid_saved_hud_blocks_scene_save_without_repairing_or_deleting_it() {
    let (_directory, scene, _) = workspace();
    let hud = scene.with_file_name(format!(
        "{}.mesh-hud.json",
        scene.file_name().unwrap().to_str().unwrap()
    ));
    fs::write(&hud, b"invalid HUD sidecar").unwrap();
    let editor =
        crate::mesh_workspace::prepare(&scene, crate::mesh_workspace::Intent::Open).unwrap();
    let mut app = EditorApp::new(editor, None);
    app.sync_mesh_presentation();
    edit_exit_radius(&mut app);
    let scene_bytes = fs::read(&scene).unwrap();
    let hud_bytes = fs::read(&hud).unwrap();
    let checksum = app.editor.checksum();
    assert!(app.mesh_workspace_save_blocker().is_some());
    assert!(!app.save_mesh_workspace());
    assert!(app.editor.is_dirty());
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    assert_eq!(fs::read(&hud).unwrap(), hud_bytes);
    assert_eq!(app.editor.checksum(), checksum);
}

#[test]
fn play_mode_blocks_workspace_save_without_touching_authored_files() {
    let (_directory, scene, mut app) = workspace();
    let bytes = fs::read(&scene).unwrap();
    assert!(app.editor.start_play());
    app.editor.sync();
    let checksum = app.editor.checksum();
    assert!(app.mesh_workspace_save_blocker().is_some());
    assert!(!app.save_mesh_workspace());
    assert_eq!(app.editor.mode(), Mode::Play);
    assert_eq!(app.editor.checksum(), checksum);
    assert_eq!(fs::read(&scene).unwrap(), bytes);
    assert_optional_sidecars_absent(&scene);
}
