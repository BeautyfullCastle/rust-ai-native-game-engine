use super::EditorApp;
use crate::{editor::Editor, game::EditorGame, mesh_workspace::mission::Settings, Mode, Owner};
use orr_reflect::Value;
use std::{fs, path::Path};

fn physics_app(root: &Path) -> EditorApp {
    let scene = root.join("owned-physics.scene.yaml");
    fs::copy(EditorGame::PhysGame.default_scene_path(), &scene).unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::PhysGame).unwrap();
    editor.sync();
    EditorApp::new(editor, None)
}

#[test]
fn modal_blocks_other_workspaces_but_can_prepare_its_own_game() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let mut app = physics_app(&root);
    let original_path = app.editor.path().unwrap();
    let original_bytes = fs::read(&original_path).unwrap();
    let output = root.join("new-game.scene.yaml");
    app.mesh_game_creator.open();

    assert!(app.mesh_game_creator.busy());
    assert!(app.mesh_workspace_blocker().is_some());
    assert!(app.mesh_workspace_base_blocker().is_none());
    assert!(app.create_mesh_game(&output, &Settings::default()));
    assert_eq!(app.editor.game(), EditorGame::MeshNavigation);
    assert_eq!(app.editor.mode(), Mode::Edit);
    assert!(!app.editor.is_dirty());
    assert!(!app.mesh_game_creator.busy());
    assert_eq!(fs::read(original_path).unwrap(), original_bytes);
    assert_eq!(
        app.editor.path().unwrap().canonicalize().unwrap(),
        output.canonicalize().unwrap()
    );
    assert!(app.mesh_presentation_blocker().is_none());
}

#[test]
fn invalid_game_request_keeps_the_current_host_history_and_files() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let mut app = physics_app(&root);
    let original_path = app.editor.path();
    let original_checksum = app.editor.checksum();
    let history = app.editor.history().clone();
    let output = root.join("invalid.scene.yaml");
    let settings = Settings {
        width: 0,
        ..Settings::default()
    };
    app.mesh_game_creator.open();

    assert!(!app.create_mesh_game(&output, &settings));
    assert_eq!(app.editor.game(), EditorGame::PhysGame);
    assert_eq!(app.editor.path(), original_path);
    assert_eq!(app.editor.checksum(), original_checksum);
    assert_eq!(app.editor.history(), &history);
    assert!(!output.exists());
    assert!(app.mesh_game_creator.busy());
}

#[test]
fn play_session_blocks_new_game_without_creating_a_scene() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let mut app = physics_app(&root);
    assert!(app.editor.start_play());
    app.editor.sync();
    let checksum = app.editor.checksum();
    let output = root.join("blocked-play.scene.yaml");

    assert!(!app.create_mesh_game(&output, &Settings::default()));
    assert_eq!(app.editor.mode(), Mode::Play);
    assert_eq!(app.editor.checksum(), checksum);
    assert!(!output.exists());
}

#[test]
fn dirty_current_scene_blocks_new_game_without_losing_its_edit() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let mut app = physics_app(&root);
    app.editor.select_named("body_05");
    app.editor.sync();
    assert!(app.editor.set_field(
        &Owner::Component("orr_physics::Body".into()),
        "pos.x",
        Value::Fixed(orr_fp::fp!(2))
    ));
    app.editor.sync();
    assert!(app.editor.is_dirty());
    let original_path = app.editor.path();
    let checksum = app.editor.checksum();
    let history = app.editor.history().clone();
    let output = root.join("blocked-dirty.scene.yaml");

    assert!(!app.create_mesh_game(&output, &Settings::default()));
    assert_eq!(app.editor.path(), original_path);
    assert_eq!(app.editor.checksum(), checksum);
    assert_eq!(app.editor.history(), &history);
    assert!(app.editor.is_dirty());
    assert!(!output.exists());
}
