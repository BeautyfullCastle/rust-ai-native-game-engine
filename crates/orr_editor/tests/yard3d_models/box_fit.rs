use super::*;

#[test]
fn readonly_fit_guard_preserves_draft_and_host_checksum() {
    let scene = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scenes/yard3d_authoring.scene.yaml")
        .canonicalize()
        .unwrap();
    let editor = editor(&scene);
    let mut panel = ModelPanel::default();
    panel.transform.translation = [1.0, 2.0, 3.0];
    let before = panel.transform;
    let checksum = editor.checksum();
    assert!(panel.fit_selected_box(&editor).is_err());
    assert_eq!(panel.transform, before);
    assert_eq!(editor.checksum(), checksum);
}

#[cfg(target_os = "linux")]
#[test]
fn fit_widget_is_draft_only_then_assign_save_reopen_uses_centered_model() {
    let temp = tempfile::tempdir().unwrap();
    let scene = install_and_scene(temp.path());
    let mut h = headless(&scene);
    settle(&mut h);
    click(&mut h, "box_right");
    click(&mut h, "Static model binding");
    click(&mut h, "Create model bindings");
    configure_asset(h.state_mut());
    click(&mut h, "Load model asset");
    let original = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().entries.len();
    h.state_mut().models.transform.rotation = [0.0, 1.0, 0.0, 0.0];
    click(&mut h, "Fit rest model to box");
    assert!(
        h.state().models.error().is_none(),
        "{:?}",
        h.state().models.error()
    );
    let fit = h.state().models.transform;
    assert_eq!(fit.rotation, [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(fit.scale[0], fit.scale[1]);
    assert_eq!(fit.scale[1], fit.scale[2]);
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &original
    );
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history().entries.len(), history);
    assert!(!binding_path(h.state()).exists());
    click(&mut h, "Assign static model");
    click(&mut h, "Save model bindings");
    let placed = h.state().models.placements(&h.state().editor);
    let model = &placed[0].model;
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for primitive in &model.source().primitives {
        for vertex in &primitive.vertices {
            for axis in 0..3 {
                let p = (0..3)
                    .map(|c| primitive.transform[c][axis] * vertex.position[c])
                    .sum::<f32>()
                    + primitive.transform[3][axis];
                let fitted = p * fit.scale[axis] + fit.translation[axis];
                min[axis] = min[axis].min(fitted);
                max[axis] = max[axis].max(fitted);
            }
        }
    }
    for axis in 0..3 {
        assert!(min[axis] >= -0.5 && max[axis] <= 0.5);
        assert!((min[axis] + max[axis]).abs() < 1e-5);
    }
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
        fit
    );
    assert_eq!(
        reopened.placements(&app.editor)[0].instance,
        placed[0].instance
    );
}

#[cfg(target_os = "linux")]
#[test]
fn missing_candidate_changed_form_nonbox_and_play_leave_draft_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let scene = install_and_scene(temp.path());
    let text = std::fs::read_to_string(&scene).unwrap().replace(
        "shape: { kind: box, half_extents: [0.5, 0.5, 0.5] }",
        "shape: { kind: sphere, radius: 0.5 }",
    );
    std::fs::write(&scene, text).unwrap();
    let mut app = EditorApp::new(editor(&scene), None);
    app.models.open_for_editor(&app.editor, true).unwrap();
    assert!(app.editor.select_named("ground"));
    configure_asset(&mut app);
    let before = app.models.transform;
    let document = app.models.bindings.as_ref().unwrap().document().clone();
    assert!(app.models.fit_selected_box(&app.editor).is_err());
    assert_eq!(app.models.transform, before);
    app.models.load_candidate(&app.editor).unwrap();
    app.models.asset = "wrong.glb".into();
    assert!(app.models.fit_selected_box(&app.editor).is_err());
    assert_eq!(app.models.transform, before);
    app.models.asset = ASSET.into();
    assert!(app.editor.select_named("box_right"));
    assert!(app.models.fit_selected_box(&app.editor).is_err());
    assert_eq!(app.models.transform, before);
    assert!(app.editor.select_named("ground"));
    let lock = temp.path().join("orr.packages.lock.json");
    let bytes = std::fs::read(&lock).unwrap();
    std::fs::write(&lock, b"invalid lock").unwrap();
    assert!(app.models.fit_selected_box(&app.editor).is_err());
    assert_eq!(app.models.transform, before);
    std::fs::write(lock, bytes).unwrap();
    assert!(app.editor.start_play());
    app.editor.sync();
    assert!(app.models.fit_selected_box(&app.editor).is_err());
    assert_eq!(app.models.transform, before);
    assert_eq!(app.models.bindings.as_ref().unwrap().document(), &document);
}
