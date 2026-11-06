use super::*;
use orr_reflect::Guid;
#[test]
fn unmatched_source_rejects_batch_without_document_or_frame_change() {
    let scene = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scenes/yard3d_authoring.scene.yaml")
        .canonicalize()
        .unwrap();
    let editor = editor(&scene);
    let mut panel = ModelPanel::default();
    let checksum = editor.checksum();
    assert!(panel.assign_targets(&editor, &[Guid::from_u32(2)]).is_err());
    assert!(panel.bindings.is_none());
    assert_eq!(editor.checksum(), checksum);
}
#[cfg(target_os = "linux")]
fn targets(app: &EditorApp) -> Vec<Guid> {
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
fn actual_marked_widgets_assign_two_bodies_one_undo_and_save_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let scene = install_and_scene(temp.path());
    let mut h = headless(&scene);
    settle(&mut h);
    click(&mut h, "box_right");
    click(&mut h, "Static model binding");
    click(&mut h, "Create model bindings");
    configure_asset(h.state_mut());
    h.state_mut().models.transform.translation = [0.0, 0.5, 0.0];
    click(&mut h, "Batch model targets");
    let guids = targets(h.state());
    for (name, guid) in ["box_left", "box_right"].iter().zip(&guids) {
        click(&mut h, &format!("Model target: {name} [{guid}]"));
    }
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().entries.len();
    let scene_bytes = std::fs::read(&scene).unwrap();
    click(&mut h, "Assign model to marked bodies");
    assert!(
        h.state().models.error().is_none(),
        "{:?}",
        h.state().models.error()
    );
    assert_eq!(count(h.state()), 2);
    let poses = h.state().models.placements(&h.state().editor);
    assert_eq!(poses.len(), 2);
    assert!(std::sync::Arc::ptr_eq(&poses[0].model, &poses[1].model));
    let applied = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    for guid in &guids {
        assert_eq!(
            applied.bindings[&guid.to_string()].transform,
            h.state().models.transform
        );
    }
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history().entries.len(), history);
    assert!(!binding_path(h.state()).exists());
    click(&mut h, "Assign model to marked bodies"); // no-op: no extra history
    click(&mut h, "Undo model");
    assert_eq!(count(h.state()), 0);
    click(&mut h, "Redo model");
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &applied
    );
    click(&mut h, "Save model bindings");
    assert_eq!(std::fs::read(&scene).unwrap(), scene_bytes);
    let app = h.state_mut();
    let mut reopened = ModelPanel::default();
    reopened.open_for_editor(&app.editor, false).unwrap();
    assert_eq!(reopened.bindings.as_ref().unwrap().document(), &applied);
    assert_eq!(reopened.placements(&app.editor).len(), 2);
    click(&mut h, "Clear model targets");
    assert!(h
        .get_by_label("Assign model to marked bodies")
        .accesskit_node()
        .is_disabled());
}
#[cfg(target_os = "linux")]
#[test]
fn invalid_and_aggregate_targets_fail_atomically_preserving_saved_model_and_undo() {
    let temp = tempfile::tempdir().unwrap();
    let scene = install_and_scene(temp.path());
    let mut h = headless(&scene);
    settle(&mut h);
    assert!(h.state_mut().editor.select_named("box_right"));
    assign_direct(h.state_mut());
    let guids = targets(h.state());
    let app = h.state_mut();
    app.models.save(&app.editor).unwrap();
    let path = binding_path(app);
    let bytes = std::fs::read(&path).unwrap();
    let original = app.models.bindings.as_ref().unwrap().document().clone();
    let old = app.models.placements(&app.editor)[0].model.clone();
    let checksum = app.editor.checksum();
    for bad in [
        vec![],
        vec![guids[0].clone(), guids[0].clone()],
        vec![guids[0].clone(), Guid::from_u32(0xffff)],
        vec![guids[0].clone(); 129],
    ] {
        assert!(app.models.assign_targets(&app.editor, &bad).is_err());
        assert_eq!(app.models.bindings.as_ref().unwrap().document(), &original);
    }
    app.models.reserve_scene_models(255);
    assert!(app.models.assign_targets(&app.editor, &guids).is_err());
    app.models.reserve_scene_models(0);
    assert_eq!(app.models.bindings.as_ref().unwrap().document(), &original);
    assert!(std::sync::Arc::ptr_eq(
        &old,
        &app.models.placements(&app.editor)[0].model
    ));
    assert_eq!(app.editor.checksum(), checksum);
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    assert!(!app.models.bindings.as_ref().unwrap().dirty());
    app.models.undo(&app.editor).unwrap();
    assert!(app
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings
        .is_empty());
}
#[cfg(target_os = "linux")]
#[test]
fn play_and_incoherent_scene_reject_every_target() {
    let temp = tempfile::tempdir().unwrap();
    let scene = install_and_scene(temp.path());
    let mut h = headless(&scene);
    settle(&mut h);
    assert!(h.state_mut().editor.select_named("box_right"));
    assign_direct(h.state_mut());
    let guids = targets(h.state());
    let original = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    assert!(h.state_mut().editor.delete_selected());
    {
        let app = h.state_mut();
        assert!(!app.editor.yard_rows_coherent());
        assert!(app.models.assign_targets(&app.editor, &guids).is_err());
    }
    settle(&mut h);
    assert!(h.state_mut().editor.start_play());
    settle(&mut h);
    let app = h.state_mut();
    assert!(app.models.assign_targets(&app.editor, &guids).is_err());
    assert_eq!(app.models.bindings.as_ref().unwrap().document(), &original);
}

#[cfg(target_os = "linux")]
#[test]
fn deleted_marked_guid_blocks_widget_after_recycling_and_open_resets_marks() {
    let temp = tempfile::tempdir().unwrap();
    let scene = install_and_scene(temp.path());
    let mut h = headless(&scene);
    settle(&mut h);
    click(&mut h, "box_right");
    click(&mut h, "Static model binding");
    click(&mut h, "Create model bindings");
    configure_asset(h.state_mut());
    click(&mut h, "Batch model targets");
    let guids = targets(h.state());
    for (name, guid) in ["box_left", "box_right"].iter().zip(&guids) {
        click(&mut h, &format!("Model target: {name} [{guid}]"));
    }
    assert!(h.state_mut().editor.select_named("box_left"));
    assert!(h.state_mut().editor.delete_selected());
    settle(&mut h);
    assert!(h
        .state_mut()
        .editor
        .spawn_yard_body(FPVec3::new(FP::ZERO, FP::ONE, FP::ZERO)));
    settle(&mut h);
    assert!(!guids.contains(&h.state().editor.selected_guid().unwrap()));
    assert!(h
        .get_by_label("Assign model to marked bodies")
        .accesskit_node()
        .is_disabled());
    assert!(h
        .query_by_label("1 marked GUID(s) are missing; Clear model targets and mark again")
        .is_some());
    assert_eq!(count(h.state()), 0);
    {
        let app = h.state_mut();
        assert!(app.models.assign_targets(&app.editor, &guids).is_err());
    }
    click(&mut h, "Clear model targets");
    click(&mut h, &format!("Model target: box_right [{}]", guids[1]));
    assert!(!h
        .get_by_label("Assign model to marked bodies")
        .accesskit_node()
        .is_disabled());
    click(&mut h, "Save model bindings");
    click(&mut h, "Open model bindings");
    assert!(
        h.state().models.error().is_none(),
        "{:?}",
        h.state().models.error()
    );
    assert!(h
        .get_by_label("Assign model to marked bodies")
        .accesskit_node()
        .is_disabled());
}
