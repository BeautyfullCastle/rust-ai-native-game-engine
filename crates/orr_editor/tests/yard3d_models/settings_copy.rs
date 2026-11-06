use super::*;
fn form(
    panel: &ModelPanel,
) -> (
    String,
    String,
    orr_editor::model_bindings::ModelKind,
    orr_editor::model_bindings::LocalTransform,
) {
    (
        panel.package.clone(),
        panel.asset.clone(),
        panel.kind,
        panel.transform,
    )
}
#[test]
fn missing_binding_guard_preserves_draft_on_readonly_repository_scene() {
    let scene = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scenes/yard3d_authoring.scene.yaml")
        .canonicalize()
        .unwrap();
    let editor = editor(&scene);
    let mut panel = ModelPanel::default();
    panel.package = "draft".into();
    panel.asset = "draft.model.json".into();
    let before = form(&panel);
    let checksum = editor.checksum();
    assert!(panel.use_selected_settings(&editor).is_err());
    assert_eq!(form(&panel), before);
    assert_eq!(editor.checksum(), checksum);
}
#[cfg(target_os = "linux")]
#[test]
fn widget_copy_is_readonly_then_edit_assign_save_reopens_with_preserved_transform() {
    let temp = tempfile::tempdir().unwrap();
    let scene = install_and_scene(temp.path());
    let mut h = headless(&scene);
    settle(&mut h);
    click(&mut h, "box_right");
    click(&mut h, "Static model binding");
    click(&mut h, "Create model bindings");
    assert!(h
        .get_by_label("Use selected model settings")
        .accesskit_node()
        .is_disabled());
    configure_asset(h.state_mut());
    h.state_mut().models.transform.translation = [0.25, 0.5, 0.0];
    click(&mut h, "Assign static model");
    click(&mut h, "Save model bindings");
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
    let sidecar = binding_path(h.state());
    let bytes = std::fs::read(&sidecar).unwrap();
    h.state_mut().models.package = "draft".into();
    h.state_mut().models.asset = "different.glb".into();
    h.state_mut().models.transform = Default::default();
    click(&mut h, "Use selected model settings");
    assert_eq!(h.state().models.package, PACKAGE);
    assert_eq!(h.state().models.asset, ASSET);
    assert_eq!(h.state().models.transform.translation, [0.25, 0.5, 0.0]);
    assert_eq!(h.state().models.transform.scale, [0.75; 3]);
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &original
    );
    assert!(!h.state().models.bindings.as_ref().unwrap().dirty());
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history().entries.len(), history);
    assert_eq!(std::fs::read(&sidecar).unwrap(), bytes);
    click(&mut h, "Undo model");
    assert_eq!(count(h.state()), 0);
    click(&mut h, "Redo model");
    assert_eq!(count(h.state()), 1);
    h.state_mut().models.transform.scale = [0.5; 3];
    click(&mut h, "Assign static model");
    click(&mut h, "Save model bindings");
    let mut reopened = ModelPanel::default();
    reopened.open_for_editor(&h.state().editor, false).unwrap();
    let placements = reopened.placements(&h.state().editor);
    assert_eq!(placements.len(), 1);
    assert_eq!(placements[0].instance.scale, [0.5; 3]);
    let binding = reopened
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings
        .values()
        .next()
        .unwrap();
    assert_eq!(binding.transform.translation, [0.25, 0.5, 0.0]);
}
#[cfg(target_os = "linux")]
#[test]
fn stale_binding_and_play_guards_leave_form_and_document_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let scene = install_and_scene(temp.path());
    let mut app = EditorApp::new(editor(&scene), None);
    assert!(app.editor.select_named("box_right"));
    app.editor.sync();
    assign_direct(&mut app);
    let path = binding_path(&app);
    let original = app.models.bindings.as_ref().unwrap().document().clone();
    let mut stale = original.clone();
    let binding = stale.bindings.values_mut().next().unwrap();
    binding.source_hash = if binding.source_hash == "f".repeat(64) {
        "e".repeat(64)
    } else {
        "f".repeat(64)
    };
    app.models.bindings = Some(
        orr_editor::model_bindings::Bindings::from_document(path.clone(), stale.clone()).unwrap(),
    );
    app.models.package = "draft".into();
    app.models.asset = "draft.glb".into();
    let before = form(&app.models);
    assert!(app.models.use_selected_settings(&app.editor).is_err());
    assert_eq!(form(&app.models), before);
    assert_eq!(app.models.bindings.as_ref().unwrap().document(), &stale);
    app.models.bindings =
        Some(orr_editor::model_bindings::Bindings::from_document(path, original.clone()).unwrap());
    assert!(app.editor.start_play());
    app.editor.sync();
    assert!(app.models.use_selected_settings(&app.editor).is_err());
    assert_eq!(form(&app.models), before);
    assert_eq!(app.models.bindings.as_ref().unwrap().document(), &original);
}
