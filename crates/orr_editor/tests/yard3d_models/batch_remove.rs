use super::*;
use orr_reflect::Guid;
#[test]
fn unmatched_source_rejects_removal_without_document_or_frame_change() {
    let scene = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scenes/yard3d_authoring.scene.yaml")
        .canonicalize()
        .unwrap();
    let editor = editor(&scene);
    let mut panel = ModelPanel::default();
    let checksum = editor.checksum();
    assert!(panel.remove_targets(&editor, &[Guid::from_u32(2)]).is_err());
    assert!(panel.bindings.is_none());
    assert_eq!(editor.checksum(), checksum);
}
#[cfg(target_os = "linux")]
fn guids(app: &EditorApp) -> Vec<Guid> {
    ["box_left", "box_right"]
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
        .collect()
}
#[cfg(target_os = "linux")]
#[test]
fn actual_remove_widgets_restore_all_previous_transforms_and_preserve_survivor_save_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let scene = install_and_scene(temp.path());
    let mut h = headless(&scene);
    settle(&mut h);
    assert!(h.state_mut().editor.select_named("ground"));
    assign_direct(h.state_mut());
    for (name, offset) in [("box_left", 0.25), ("box_right", -0.5)] {
        let app = h.state_mut();
        assert!(app.editor.select_named(name));
        app.models.transform.translation = [offset, 0.0, 0.0];
        app.models.assign(&app.editor).unwrap();
    }
    {
        let app = h.state_mut();
        app.models.save(&app.editor).unwrap();
    }
    let original = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    let live = h
        .state()
        .models
        .placements(&h.state().editor)
        .into_iter()
        .find(|pose| {
            pose.entity
                == h.state()
                    .editor
                    .rows()
                    .iter()
                    .find(|row| row.name.as_deref() == Some("ground"))
                    .unwrap()
                    .entity
        })
        .unwrap()
        .model;
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().entries.len();
    let scene_bytes = std::fs::read(&scene).unwrap();
    let sidecar = binding_path(h.state());
    let bytes = std::fs::read(&sidecar).unwrap();
    settle(&mut h);
    if h.query_by_label("Batch model targets").is_none() {
        click(&mut h, "Static model binding");
    }
    if h.query_by_label("Clear model targets").is_none() {
        click(&mut h, "Batch model targets");
    }
    let targets = guids(h.state());
    for (name, guid) in ["box_left", "box_right"].iter().zip(&targets) {
        click(&mut h, &format!("Model target: {name} [{guid}]"));
    }
    click(&mut h, "Remove models from marked bodies");
    assert!(
        h.state().models.error().is_none(),
        "{:?}",
        h.state().models.error()
    );
    assert_eq!(count(h.state()), 1);
    assert!(std::sync::Arc::ptr_eq(
        &live,
        &h.state().models.placements(&h.state().editor)[0].model
    ));
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history().entries.len(), history);
    assert_eq!(std::fs::read(&sidecar).unwrap(), bytes);
    assert!(h
        .get_by_label("Remove models from marked bodies")
        .accesskit_node()
        .is_disabled());
    {
        let app = h.state_mut();
        assert_eq!(app.models.remove_targets(&app.editor, &targets).unwrap(), 0);
    }
    click(&mut h, "Undo model");
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &original
    );
    click(&mut h, "Redo model");
    click(&mut h, "Save model bindings");
    assert_eq!(std::fs::read(&scene).unwrap(), scene_bytes);
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
            .len(),
        1
    );
    assert_eq!(reopened.placements(&app.editor).len(), 1);
}
#[cfg(target_os = "linux")]
#[test]
fn invalid_or_missing_batch_preserves_saved_bindings_cache_and_undo_then_all_yard_bindings_can_remove(
) {
    let temp = tempfile::tempdir().unwrap();
    let scene = install_and_scene(temp.path());
    let mut h = headless(&scene);
    settle(&mut h);
    assert!(h.state_mut().editor.select_named("box_right"));
    assign_direct(h.state_mut());
    let targets = guids(h.state());
    let app = h.state_mut();
    app.models.save(&app.editor).unwrap();
    let before = app.models.bindings.as_ref().unwrap().document().clone();
    let bytes = std::fs::read(binding_path(app)).unwrap();
    let old = app.models.placements(&app.editor)[0].model.clone();
    let checksum = app.editor.checksum();
    for bad in [
        vec![],
        vec![targets[1].clone(), targets[1].clone()],
        vec![targets[1].clone(), Guid::from_u32(0xffff)],
        vec![targets[0].clone(); 129],
    ] {
        assert!(app.models.remove_targets(&app.editor, &bad).is_err());
        assert_eq!(app.models.bindings.as_ref().unwrap().document(), &before);
    }
    assert_eq!(
        app.models
            .remove_targets(&app.editor, &[targets[0].clone()])
            .unwrap(),
        0
    );
    assert_eq!(app.editor.checksum(), checksum);
    assert_eq!(std::fs::read(binding_path(app)).unwrap(), bytes);
    assert!(std::sync::Arc::ptr_eq(
        &old,
        &app.models.placements(&app.editor)[0].model
    ));
    assert!(!app.models.bindings.as_ref().unwrap().dirty());
    app.models.undo(&app.editor).unwrap(); // failed/no-op removals added no history
    assert!(app
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings
        .is_empty());
    app.models.redo(&app.editor).unwrap();
    assert_eq!(app.models.remove_targets(&app.editor, &targets).unwrap(), 1);
    assert!(app
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings
        .is_empty());
    app.models.save(&app.editor).unwrap();
    let mut reopened = ModelPanel::default();
    reopened.open_for_editor(&app.editor, false).unwrap();
    assert!(reopened.placements(&app.editor).is_empty());
}
#[cfg(target_os = "linux")]
#[test]
fn marked_removal_is_disabled_for_deleted_recycled_target_and_rejected_in_play() {
    let temp = tempfile::tempdir().unwrap();
    let scene = install_and_scene(temp.path());
    let mut h = headless(&scene);
    settle(&mut h);
    assert!(h.state_mut().editor.select_named("box_right"));
    assign_direct(h.state_mut());
    settle(&mut h);
    if h.query_by_label("Batch model targets").is_none() {
        click(&mut h, "Static model binding");
    }
    if h.query_by_label("Clear model targets").is_none() {
        click(&mut h, "Batch model targets");
    }
    let targets = guids(h.state());
    for (name, guid) in ["box_left", "box_right"].iter().zip(&targets) {
        click(&mut h, &format!("Model target: {name} [{guid}]"));
    }
    let before = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    assert!(h.state_mut().editor.select_named("box_left"));
    assert!(h.state_mut().editor.delete_selected());
    {
        let app = h.state_mut();
        assert!(app.models.remove_targets(&app.editor, &targets).is_err());
    }
    settle(&mut h);
    assert!(h
        .state_mut()
        .editor
        .spawn_yard_body(FPVec3::new(FP::ZERO, FP::ONE, FP::ZERO)));
    settle(&mut h);
    assert!(!targets.contains(&h.state().editor.selected_guid().unwrap()));
    assert!(h
        .get_by_label("Remove models from marked bodies")
        .accesskit_node()
        .is_disabled());
    {
        let app = h.state_mut();
        assert!(app.models.remove_targets(&app.editor, &targets).is_err());
    }
    assert!(h.state_mut().editor.start_play());
    settle(&mut h);
    let app = h.state_mut();
    assert!(app
        .models
        .remove_targets(&app.editor, &[targets[1].clone()])
        .is_err());
    assert_eq!(app.models.bindings.as_ref().unwrap().document(), &before);
}
