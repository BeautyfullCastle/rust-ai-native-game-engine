use super::*;
#[test]
fn room_batch_removes_optional_binding_saves_runtime_and_preserves_last_required_model() {
    let fixture = Fixture::new(false);
    let (mut editor, mut panel) = fixture.panel();
    editor.select(Some(orr_editor::Target::Guid(fixture.guids[&KEY].clone())));
    panel.package = "admission-models".into();
    panel.asset = "two.glb".into();
    panel.assign(&editor).unwrap();
    panel.save(&editor).unwrap();
    let original = panel.bindings.as_ref().unwrap().document().clone();
    let bytes = fs::read(fixture.root.join("room.models.json")).unwrap();
    let checksum = editor.checksum();
    assert_eq!(
        panel
            .remove_targets(&editor, &[fixture.guids[&KEY].clone()])
            .unwrap(),
        1
    );
    assert_eq!(
        panel.bindings.as_ref().unwrap().document().bindings.len(),
        1
    );
    let remaining = panel.bindings.as_ref().unwrap().document().clone();
    let cached = panel.placements(&editor)[0].model.clone();
    assert!(panel
        .remove_targets(
            &editor,
            &[fixture.guids[&PLAYER].clone(), fixture.guids[&KEY].clone()]
        )
        .is_err());
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &remaining);
    assert!(Arc::ptr_eq(&cached, &panel.placements(&editor)[0].model));
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(
        fs::read(fixture.root.join("room.models.json")).unwrap(),
        bytes
    );
    panel.undo(&editor).unwrap();
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &original);
    panel.redo(&editor).unwrap();
    panel.save(&editor).unwrap();
    let prepared = PreparedProject::open(&fixture.root).unwrap();
    assert_eq!(prepared.models().document, remaining);
    let counts = orr_sample::room_view::check_model_poses(
        orr_bridge::FrameView::of(prepared.scene().frame()),
        prepared.scene().index(),
        prepared.models(),
        #[cfg(feature = "room-character")]
        None,
    )
    .unwrap();
    assert_eq!(counts.static_models, 1);
}
