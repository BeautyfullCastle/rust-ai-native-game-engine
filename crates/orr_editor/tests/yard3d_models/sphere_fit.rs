use super::*;
#[test]
fn readonly_sphere_guard_preserves_form_and_host() {
    let scene = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scenes/yard3d_authoring.scene.yaml")
        .canonicalize()
        .unwrap();
    let editor = editor(&scene);
    let mut panel = ModelPanel::default();
    panel.transform.translation = [3.0, 2.0, 1.0];
    let before = panel.transform;
    let checksum = editor.checksum();
    assert!(panel.fit_selected_sphere(&editor).is_err());
    assert_eq!(panel.transform, before);
    assert_eq!(editor.checksum(), checksum);
}

#[cfg(target_os = "linux")]
#[test]
fn sphere_widget_assign_save_reopen_keeps_vertices_inside_sphere_without_physics_changes() {
    let temp = tempfile::tempdir().unwrap();
    let scene = install_and_scene(temp.path());
    let text = std::fs::read_to_string(&scene).unwrap().replace(
        "shape: { kind: box, half_extents: [0.5, 0.5, 0.5] }",
        "shape: { kind: sphere, radius: 0.5 }",
    );
    std::fs::write(&scene, text).unwrap();
    let mut h = headless(&scene);
    settle(&mut h);
    click(&mut h, "box_right");
    click(&mut h, "Static model binding");
    click(&mut h, "Create model bindings");
    configure_asset(h.state_mut());
    click(&mut h, "Load model asset");
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
    click(&mut h, "Fit rest model to sphere");
    assert!(
        h.state().models.error().is_none(),
        "{:?}",
        h.state().models.error()
    );
    let fit = h.state().models.transform;
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &document
    );
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history().entries.len(), history);
    assert!(!binding_path(h.state()).exists());
    click(&mut h, "Assign static model");
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
        fit
    );
    let placements = reopened.placements(&app.editor);
    for primitive in &placements[0].model.source().primitives {
        for vertex in &primitive.vertices {
            let point: [f32; 3] = std::array::from_fn(|axis| {
                let p = (0..3)
                    .map(|column| primitive.transform[column][axis] * vertex.position[column])
                    .sum::<f32>()
                    + primitive.transform[3][axis];
                p * fit.scale[axis] + fit.translation[axis]
            });
            assert!(point.iter().map(|v| v * v).sum::<f32>() <= 0.25);
        }
    }
    assert_eq!(app.editor.checksum(), checksum);
}

#[cfg(target_os = "linux")]
#[test]
fn sphere_fit_rejects_boxes_bad_form_bad_lock_and_play_atomically() {
    let temp = tempfile::tempdir().unwrap();
    let scene = install_and_scene(temp.path());
    let mut app = EditorApp::new(editor(&scene), None);
    app.models.open_for_editor(&app.editor, true).unwrap();
    assert!(app.editor.select_named("box_right"));
    configure_asset(&mut app);
    app.models.load_candidate(&app.editor).unwrap();
    let before = app.models.transform;
    assert!(app.models.fit_selected_sphere(&app.editor).is_err());
    assert_eq!(app.models.transform, before);
    // An actual sphere fixture tests stale form/project rejection before mutation.
    let sphere_scene = temp.path().join("sphere.scene.yaml");
    let text = std::fs::read_to_string(&scene).unwrap().replace(
        "shape: { kind: box, half_extents: [0.5, 0.5, 0.5] }",
        "shape: { kind: sphere, radius: 0.5 }",
    );
    std::fs::write(&sphere_scene, text).unwrap();
    let mut app = EditorApp::new(editor(&sphere_scene), None);
    app.models.open_for_editor(&app.editor, true).unwrap();
    assert!(app.editor.select_named("box_right"));
    configure_asset(&mut app);
    let before = app.models.transform;
    assert!(app.models.fit_selected_sphere(&app.editor).is_err());
    assert_eq!(app.models.transform, before);
    app.models.load_candidate(&app.editor).unwrap();
    app.models.asset = "wrong.glb".into();
    assert!(app.models.fit_selected_sphere(&app.editor).is_err());
    assert_eq!(app.models.transform, before);
    app.models.asset = ASSET.into();
    let lock = temp.path().join("orr.packages.lock.json");
    let bytes = std::fs::read(&lock).unwrap();
    std::fs::write(&lock, b"bad lock").unwrap();
    assert!(app.models.fit_selected_sphere(&app.editor).is_err());
    assert_eq!(app.models.transform, before);
    std::fs::write(lock, bytes).unwrap();
    assert!(app.editor.start_play());
    app.editor.sync();
    assert!(app.models.fit_selected_sphere(&app.editor).is_err());
    assert_eq!(app.models.transform, before);
}
