//! Real egui inputs and local Arena host, without a native display. Package
//! loading is real; GPU output is covered by animated_preview separately.
#![cfg(feature = "animated-models")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui::{accesskit::Role, Key, Modifiers};
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::game::EditorGame;
use orr_editor::{Editor, EditorApp};
use orr_package::{Project, Runtime};
use std::path::Path;

fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(3);
    h.get_by_label(label).click();
    for _ in 0..3 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
}
fn type_into(h: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(2);
}
fn install(root: &Path) -> Project {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/animation_demo")
        .canonicalize()
        .unwrap();
    let project = Project::open_for_install(root, Runtime::content_only().engine_version).unwrap();
    project.install(&[source]).unwrap();
    project
}
fn setup(root: &Path) -> Harness<'static, EditorApp> {
    let scene = root.join("arena.yaml");
    std::fs::write(
        &scene,
        include_str!("../../../scenes/arena_blank.scene.yaml"),
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::Arena).unwrap();
    editor.sync();
    assert!(editor.spawn_arena_player(0, [0.0, 0.0]));
    editor.sync();
    assert!(editor.spawn_arena_player(1, [20.0, 0.0]));
    editor.sync();
    assert!(editor.save());
    editor.sync();
    editor.select(None);
    Harness::builder()
        .with_step_dt(1.0 / 60.0)
        .with_size([1800.0, 1800.0])
        .build_eframe(move |_| EditorApp::new(editor, None))
}
fn create_and_load(h: &mut Harness<'_, EditorApp>, root: &Path) {
    click(h, "Animated model authoring");
    type_into(
        h,
        "Animation sidecar path",
        root.join("arena.animation.json").to_str().unwrap(),
    );
    type_into(h, "Animation scene relative path", "arena.yaml");
    click(h, "Create animation bindings");
    type_into(h, "Animated package name", "sample-animation");
    type_into(h, "Animated asset path in package", "animated.glb");
    click(h, "Load animated model");
    assert!(
        h.state().animated_models.error().is_none(),
        "{:?}",
        h.state().animated_models.error()
    );
}
fn binding_count(h: &Harness<'_, EditorApp>) -> usize {
    h.state()
        .animated_models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings
        .len()
}

#[test]
fn real_widgets_install_load_select_assign_undo_save_reopen_and_preview_controls() {
    let dir = tempfile::tempdir().unwrap();
    install(dir.path());
    let mut h = setup(dir.path());
    create_and_load(&mut h, dir.path());
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().entries.len();
    // The production hierarchy widget resolves a persistent GUID selection.
    click(&mut h, "player_0");
    h.get_by_role_and_label(Role::ComboBox, "Animation clip")
        .click();
    h.run_steps(3);
    click(&mut h, "1: pulse (1.00s)");
    click(&mut h, "Once");
    click(&mut h, "Assign animated clip to selection");
    assert_eq!(binding_count(&h), 1);
    let guid = h.state().editor.selected_guids()[0].clone();
    click(&mut h, "Assign animated clip to selection");
    click(&mut h, "Undo animation binding");
    assert_eq!(
        binding_count(&h),
        0,
        "repeated assignment must not create duplicate history"
    );
    click(&mut h, "Redo animation binding");
    click(&mut h, "Play animation preview");
    assert_eq!(
        h.state().animated_models.player(&guid).unwrap().state(),
        orr_model::animation::PlaybackState::Playing
    );
    click(&mut h, "Pause animation preview");
    assert_eq!(
        h.state().animated_models.player(&guid).unwrap().state(),
        orr_model::animation::PlaybackState::Paused
    );
    click(&mut h, "Stop animation preview");
    assert_eq!(
        h.state().animated_models.player(&guid).unwrap().state(),
        orr_model::animation::PlaybackState::Stopped
    );
    click(&mut h, "Save animation bindings");
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history().entries.len(), history);
    assert!(!h.state().animated_models.bindings.as_ref().unwrap().dirty());
    click(&mut h, "Close animation bindings");
    assert!(h.state().animated_models.bindings.is_none());
    assert!(h
        .state_mut()
        .editor
        .open_path(&dir.path().join("arena.yaml")));
    h.state_mut().editor.sync();
    h.run_steps(3);
    click(&mut h, "player_0");
    click(&mut h, "Open animation bindings");
    assert_eq!(binding_count(&h), 1);
    let binding = h
        .state()
        .animated_models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings
        .values()
        .next()
        .unwrap();
    assert_eq!(binding.clip_index, 1);
    assert_eq!(
        binding.playback.mode,
        orr_editor::animated_bindings::PlaybackMode::Once
    );
    assert_ne!(
        h.state().animated_models.player(&guid).unwrap().state(),
        orr_model::animation::PlaybackState::Playing
    );
    assert_eq!(h.state().editor.checksum(), checksum);

    click(&mut h, "Remove selected animation bindings");
    click(&mut h, "Close animation bindings");
    assert!(
        h.state().animated_models.bindings.is_some(),
        "dirty close must wait for explicit discard"
    );
    click(&mut h, "Keep animation bindings open");
    assert!(h.state().animated_models.bindings.as_ref().unwrap().dirty());
    click(&mut h, "Close animation bindings");
    click(&mut h, "Discard animation changes and close");
    click(&mut h, "Open animation bindings");
    assert_eq!(
        binding_count(&h),
        1,
        "discard must preserve the saved assignment"
    );
    let old_guid = guid;
    assert!(h.state_mut().editor.delete_selected());
    h.state_mut().editor.sync();
    assert!(h.state_mut().editor.spawn_arena_player(0, [0.0, 0.0]));
    h.state_mut().editor.sync();
    h.run_steps(3);
    let new_guid = h.state().editor.selected_guids()[0].clone();
    assert_ne!(new_guid, old_guid);
    assert!(h.state().animated_models.player(&old_guid).is_none());
    assert!(h.state().animated_models.player(&new_guid).is_none());
    assert!(h
        .state()
        .animated_models
        .diagnostics(&h.state().editor)
        .iter()
        .any(|e| e.contains("Orphan") && e.contains(&old_guid.to_string())));
}

#[test]
fn failed_reload_remove_reinstall_and_scene_save_as_are_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let project = install(dir.path());
    let mut h = setup(dir.path());
    create_and_load(&mut h, dir.path());
    click(&mut h, "player_0");
    click(&mut h, "Assign animated clip to selection");
    let guid = h.state().editor.selected_guids()[0].clone();
    click(&mut h, "Play animation preview");
    project.remove("sample-animation").unwrap();
    click(&mut h, "Reload animated assets");
    assert!(h.state().animated_models.error().is_some());
    assert!(h.state().animated_models.player(&guid).is_none());
    assert_eq!(binding_count(&h), 1);
    assert!(h.state().animated_models.bindings.as_ref().unwrap().dirty());
    install(dir.path());
    click(&mut h, "Reload animated assets");
    assert!(h.state().animated_models.error().is_none());
    assert!(h.state().animated_models.player(&guid).is_some());
    let other = dir.path().join("other.yaml");
    assert!(h.state_mut().editor.save_as(&other));
    h.state_mut().editor.sync();
    h.run_steps(3);
    assert!(!h.state().animated_models.scene_matches(&h.state().editor));
    assert!(h.state().animated_models.bindings.as_ref().unwrap().dirty());
    assert!(h.state().animated_models.player(&guid).is_none());
    click(&mut h, "Save animation bindings");
    assert!(!h.state().animated_models.bindings.as_ref().unwrap().dirty());
    assert_eq!(
        h.state()
            .animated_models
            .bindings
            .as_ref()
            .unwrap()
            .document()
            .scene,
        "arena.yaml"
    );
    // Switch back, then delete the real entity. Binding remains as an orphan.
    assert!(h
        .state_mut()
        .editor
        .open_path(&dir.path().join("arena.yaml")));
    h.state_mut().editor.sync();
    h.run_steps(3);
    assert!(h.state().animated_models.scene_matches(&h.state().editor));
    h.state_mut().editor.select_named("player_0");
    assert!(h.state_mut().editor.delete_selected());
    h.state_mut().editor.sync();
    h.run_steps(3);
    assert!(h
        .state()
        .animated_models
        .diagnostics(&h.state().editor)
        .iter()
        .any(|e| e.contains("Orphan")));
    assert!(h.state().animated_models.player(&guid).is_none());
}
