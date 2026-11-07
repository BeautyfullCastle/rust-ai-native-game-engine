use super::*;
#[test]
fn removing_mixed_models_and_distinct_animation_clips_restores_one_undo_and_reopens() {
    let temp = tempfile::tempdir().unwrap();
    let scene = scene(temp.path());
    let mut app = EditorApp::new(editor(&scene), None);
    open(&mut app);
    assign(&mut app, "box_left", ModelKind::Animated, 0);
    assign(&mut app, "box_right", ModelKind::Animated, 1);
    assign(&mut app, "ground", ModelKind::Static, 0);
    assign(&mut app, "model_static", ModelKind::Static, 0);
    let targets: Vec<_> = ["box_left", "box_right", "model_static"]
        .iter()
        .map(|name| {
            app.editor
                .rows()
                .iter()
                .find(|row| row.name.as_deref() == Some(name))
                .unwrap()
                .guid
                .clone()
                .unwrap()
        })
        .collect();
    let before = app.models.bindings.as_ref().unwrap().document().clone();
    let old = app.models.placements(&app.editor)[0].model.clone();
    let checksum = app.editor.checksum();
    assert_eq!(app.models.remove_targets(&app.editor, &targets).unwrap(), 3);
    assert!(app
        .models
        .animated_placements(&app.editor)
        .unwrap()
        .is_empty());
    assert!(std::sync::Arc::ptr_eq(
        &old,
        &app.models.placements(&app.editor)[0].model
    ));
    assert_eq!(app.editor.checksum(), checksum);
    app.models.undo(&app.editor).unwrap();
    assert_eq!(app.models.bindings.as_ref().unwrap().document(), &before);
    assert_eq!(
        app.models.animated_placements(&app.editor).unwrap().len(),
        2
    );
    app.models.redo(&app.editor).unwrap();
    app.models.save(&app.editor).unwrap();
    app.models = Default::default();
    app.models.open_for_editor(&app.editor, false).unwrap();
    assert!(app
        .models
        .animated_placements(&app.editor)
        .unwrap()
        .is_empty());
    assert_eq!(app.models.placements(&app.editor).len(), 1);
}
