use super::EditorApp;
use egui_kittest::Harness;

fn close(harness: &mut Harness<'_, EditorApp>) -> bool {
    harness
        .input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    harness.run_steps(1);
    harness
        .output()
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .unwrap()
        .commands
        .iter()
        .any(|command| matches!(command, egui::ViewportCommand::CancelClose))
}

#[test]
fn hidden_early_return_keeps_worker_owned_and_close_blocked_until_joined() {
    let dir = tempfile::tempdir().unwrap();
    let scene = dir.path().join("import.scene.yaml");
    let editor = crate::mesh_workspace::mission::prepare(&scene, &Default::default()).unwrap();
    let mut app = EditorApp::new(editor, None);
    let original = std::fs::read(&scene).unwrap();
    let checksum = app.editor.checksum();
    let history = app.editor.history().clone();
    let release = app
        .mesh_animation
        .start_blocked_import_for_test(&app.editor);
    assert!(app.mesh_animation_import_busy());
    assert!(!app.mesh_animation_dirty());
    assert!(app
        .mesh_workspace_base_blocker()
        .unwrap()
        .contains("imports"));
    let mut harness = Harness::builder().with_size([800., 600.]).build_ui_state(
        |ui, app: &mut EditorApp| app.with_close_review(ui, |_app, _ui| {}),
        app,
    );
    assert!(close(&mut harness));
    assert!(harness
        .state()
        .close_review_items()
        .iter()
        .any(|item| item.name == "Mesh animation import · worker owned"));
    release.send(()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while harness.state().mesh_animation_import_busy() && std::time::Instant::now() < deadline {
        harness.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(!harness.state().mesh_animation_import_busy());
    assert!(!harness.state().mesh_animation_dirty());
    assert!(!close(&mut harness));
    assert_eq!(harness.state().editor.checksum(), checksum);
    assert_eq!(harness.state().editor.history(), &history);
    assert_eq!(std::fs::read(scene).unwrap(), original);
}
