use super::*;
#[test]
fn source_guard_keeps_unmatched_panel_and_frame_unchanged() {
    let scene = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scenes/yard3d_authoring.scene.yaml")
        .canonicalize()
        .unwrap();
    let editor = editor(&scene);
    let mut panel = ModelPanel::default();
    let checksum = editor.checksum();
    assert!(panel.remove_orphaned(&editor).is_err());
    assert!(panel.bindings.is_none());
    assert_eq!(editor.checksum(), checksum);
}
#[cfg(target_os = "linux")]
#[test]
fn cleanup_widget_preserves_live_model_recycled_slot_scene_and_single_undo_step() {
    let temp = tempfile::tempdir().unwrap();
    let scene = install_and_scene(temp.path());
    let mut h = headless(&scene);
    settle(&mut h);
    assert!(h.state_mut().editor.select_named("box_left"));
    assign_direct(h.state_mut());
    let removed_guid = h.state().editor.selected_guid().unwrap();
    assert!(h.state_mut().editor.select_named("box_right"));
    configure_asset(h.state_mut());
    {
        let app = h.state_mut();
        app.models.assign(&app.editor).unwrap();
        app.models.save(&app.editor).unwrap();
    }
    let survivor_guid = h.state().editor.selected_guid().unwrap();
    let survivor = h
        .state()
        .editor
        .row_of(h.state().editor.selection().unwrap())
        .unwrap()
        .entity;
    let live = h
        .state()
        .models
        .placements(&h.state().editor)
        .into_iter()
        .find(|p| p.entity == survivor)
        .unwrap()
        .model;
    let before = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    let bytes = std::fs::read(binding_path(h.state())).unwrap();
    assert!(h.state_mut().editor.select_named("box_left"));
    assert!(h.state_mut().editor.delete_selected());
    settle(&mut h);
    assert!(h
        .state_mut()
        .editor
        .spawn_yard_body(FPVec3::new(FP::from_int(-2), FP::ONE, FP::ZERO)));
    settle(&mut h);
    let replacement = h.state().editor.selected_guid().unwrap();
    assert_ne!(replacement, removed_guid);
    if h.query_by_label("Remove orphan model bindings").is_none() {
        click(&mut h, "Static model binding");
    }
    let survivor = h.state().editor.rows().iter().find(|row| row.guid.as_ref() == Some(&survivor_guid)).unwrap().entity;
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().entries.len();
    click(&mut h, "Remove orphan model bindings");
    assert!(
        h.state().models.error().is_none(),
        "{:?}",
        h.state().models.error()
    );
    assert_eq!(count(h.state()), 1);
    assert!(!h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings
        .contains_key(&replacement.to_string()));
    let placed = h.state().models.placements(&h.state().editor);
    assert_eq!(placed.len(), 1);
    assert_eq!(placed[0].entity, survivor);
    assert!(std::sync::Arc::ptr_eq(&live, &placed[0].model));
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history().entries.len(), history);
    assert_eq!(std::fs::read(binding_path(h.state())).unwrap(), bytes);
    {
        let app = h.state_mut();
        assert_eq!(app.models.remove_orphaned(&app.editor).unwrap(), 0);
    }
    click(&mut h, "Undo model");
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &before
    );
    assert_eq!(h.state().models.placements(&h.state().editor).len(), 1);
    click(&mut h, "Redo model");
    assert_eq!(count(h.state()), 1);
    assert!(h.state_mut().editor.save());
    settle(&mut h);
    click(&mut h, "Save model bindings");
    drop(h);
    let app = EditorApp::new(editor(&scene), None);
    let mut models = ModelPanel::default();
    models.open_for_editor(&app.editor, false).unwrap();
    assert_eq!(
        models.bindings.as_ref().unwrap().document().bindings.len(),
        1
    );
    assert_eq!(models.placements(&app.editor).len(), 1);
}
#[cfg(target_os = "linux")]
#[test]
fn incoherent_deletion_and_play_reject_cleanup_without_document_changes() {
    let temp = tempfile::tempdir().unwrap();
    let scene = install_and_scene(temp.path());
    let mut h = headless(&scene);
    settle(&mut h);
    assert!(h.state_mut().editor.select_named("box_right"));
    assign_direct(h.state_mut());
    let before = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    assert!(h.state_mut().editor.delete_selected());
    assert!(!h.state().editor.yard_rows_coherent());
    {
        let app = h.state_mut();
        assert!(app.models.remove_orphaned(&app.editor).is_err());
    }
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &before
    );
    settle(&mut h);
    assert!(h.state_mut().editor.start_play());
    settle(&mut h);
    {
        let app = h.state_mut();
        assert!(app.models.remove_orphaned(&app.editor).is_err());
    }
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &before
    );
}

#[cfg(target_os = "linux")]
#[test]
fn multiple_orphans_are_removed_and_restored_by_one_model_undo() {
    let temp = tempfile::tempdir().unwrap();
    let scene = install_and_scene(temp.path());
    let mut h = headless(&scene);
    settle(&mut h);
    assert!(h.state_mut().editor.select_named("box_left"));
    assign_direct(h.state_mut());
    assert!(h.state_mut().editor.select_named("box_right"));
    {
        let app = h.state_mut();
        app.models.assign(&app.editor).unwrap();
    }
    let before = h.state().models.bindings.as_ref().unwrap().document().clone();
    for name in ["box_left", "box_right"] {
        assert!(h.state_mut().editor.select_named(name));
        assert!(h.state_mut().editor.delete_selected());
        settle(&mut h);
    }
    let app = h.state_mut();
    assert_eq!(app.models.remove_orphaned(&app.editor).unwrap(), 2);
    assert!(app.models.bindings.as_ref().unwrap().document().bindings.is_empty());
    app.models.undo(&app.editor).unwrap();
    assert_eq!(app.models.bindings.as_ref().unwrap().document(), &before);
}
