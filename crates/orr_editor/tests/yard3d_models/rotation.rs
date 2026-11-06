use super::*;
#[test]
fn source_guard_keeps_rotation_draft_and_host_unchanged() {
    let scene = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scenes/yard3d_authoring.scene.yaml")
        .canonicalize()
        .unwrap();
    let editor = editor(&scene);
    let mut panel = ModelPanel::default();
    let before = panel.transform;
    let checksum = editor.checksum();
    assert!(panel.rotate_local_quarter(&editor, 1, 1).is_err());
    assert!(panel.reset_local_rotation(&editor).is_err());
    assert_eq!(panel.transform, before);
    assert_eq!(editor.checksum(), checksum);
}
#[cfg(target_os = "linux")]
#[test]
fn actual_rotation_widgets_save_reopen_and_preserve_body_offset_and_history() {
    let temp = tempfile::tempdir().unwrap();
    let scene = install_and_scene(temp.path());
    let mut h = headless(&scene);
    settle(&mut h);
    click(&mut h, "box_right");
    click(&mut h, "Static model binding");
    click(&mut h, "Create model bindings");
    configure_asset(h.state_mut());
    h.state_mut().models.transform.translation = [0.25, 0.5, 0.0];
    let document = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().entries.len();
    click(&mut h, "Rotate X +90");
    click(&mut h, "Reset model rotation");
    assert_eq!(h.state().models.transform.rotation, [0.0, 0.0, 0.0, 1.0]);
    click(&mut h, "Rotate Y +90");
    let rotated = h.state().models.transform;
    assert_eq!(rotated.translation, [0.25, 0.5, 0.0]);
    assert_eq!(rotated.scale, [0.75; 3]);
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &document
    );
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history().entries.len(), history);
    assert!(!binding_path(h.state()).exists());
    click(&mut h, "Assign static model");
    click(&mut h, "Save model bindings");
    click(&mut h, "Undo model");
    assert_eq!(count(h.state()), 0);
    click(&mut h, "Redo model");
    click(&mut h, "Save model bindings");
    let app = h.state_mut();
    let mut reopened = ModelPanel::default();
    reopened.open_for_editor(&app.editor, false).unwrap();
    assert_eq!(
        reopened
            .bindings
            .as_ref()
            .unwrap()
            .document()
            .bindings
            .values()
            .next()
            .unwrap()
            .transform,
        rotated
    );
    let instance = reopened.placements(&app.editor)[0].instance;
    assert_eq!(instance.translation, [2.25, 3.5, 0.0]);
    let q = instance.rotation;
    let v = orr_view::Quat::new(q[0], q[1], q[2], q[3]).rotate(orr_view::Vec3::new(0.0, 0.0, 1.0));
    assert!((v.x - 1.0).abs() < 1e-5 && v.y.abs() < 1e-5 && v.z.abs() < 1e-5);
}
#[cfg(target_os = "linux")]
#[test]
fn invalid_rotation_is_atomic_reset_recovers_and_play_blocks_both_actions() {
    let temp = tempfile::tempdir().unwrap();
    let scene = install_and_scene(temp.path());
    let mut app = EditorApp::new(editor(&scene), None);
    app.models.open_for_editor(&app.editor, true).unwrap();
    app.models.transform.rotation = [0.0; 4];
    app.models.transform.translation = [1.0, 2.0, 3.0];
    let before = app.models.transform;
    assert!(app.models.rotate_local_quarter(&app.editor, 0, 1).is_err());
    assert_eq!(app.models.transform, before);
    app.models.reset_local_rotation(&app.editor).unwrap();
    assert_eq!(app.models.transform.translation, before.translation);
    assert_eq!(app.models.transform.rotation, [0.0, 0.0, 0.0, 1.0]);
    let before = app.models.transform;
    assert!(app.models.rotate_local_quarter(&app.editor, 3, 1).is_err());
    assert_eq!(app.models.transform, before);
    assert!(app.editor.start_play());
    app.editor.sync();
    assert!(app.models.rotate_local_quarter(&app.editor, 1, 1).is_err());
    assert!(app.models.reset_local_rotation(&app.editor).is_err());
    assert_eq!(app.models.transform, before);
}
