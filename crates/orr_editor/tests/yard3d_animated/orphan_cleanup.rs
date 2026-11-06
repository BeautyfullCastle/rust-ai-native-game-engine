use super::*;
#[test]
fn animated_orphan_cleanup_and_independent_scene_model_undo_restore_clip() {
    let temp = tempfile::tempdir().unwrap();
    let scene = scene(temp.path());
    let mut app = EditorApp::new(editor(&scene), None);
    open(&mut app);
    assign(&mut app, "box_right", ModelKind::Animated, 1);
    app.models.save(&app.editor).unwrap();
    let original = app.models.bindings.as_ref().unwrap().document().clone();
    assert!(app.editor.delete_selected());
    sync_clock(&mut app.editor);
    assert_eq!(app.models.remove_orphaned(&app.editor).unwrap(), 1);
    assert!(app
        .models
        .animated_placements(&app.editor)
        .unwrap()
        .is_empty());
    app.models.undo(&app.editor).unwrap();
    assert_eq!(app.models.bindings.as_ref().unwrap().document(), &original);
    assert!(app
        .models
        .animated_placements(&app.editor)
        .unwrap()
        .is_empty());
    assert!(app.editor.undo());
    sync_clock(&mut app.editor);
    let poses = app.models.animated_placements(&app.editor).unwrap();
    assert_eq!(poses.len(), 1);
    assert_eq!(
        app.models
            .bindings
            .as_ref()
            .unwrap()
            .document()
            .bindings
            .values()
            .next()
            .unwrap()
            .animation
            .unwrap()
            .clip_index,
        1
    );
}
