use super::*;
#[test]
fn room_batch_assign_save_reopen_consumes_every_guid_and_undo_is_one_step() {
    let fixture = Fixture::new(false);
    let (editor, mut panel) = fixture.panel();
    let original = panel.bindings.as_ref().unwrap().document().clone();
    let bytes = fs::read(fixture.root.join("room.models.json")).unwrap();
    let checksum = editor.checksum();
    let guids = vec![fixture.guids[&PLAYER].clone(), fixture.guids[&KEY].clone()];
    panel.package = "admission-models".into();
    panel.asset = "two.glb".into();
    panel.assign_targets(&editor, &guids).unwrap();
    assert_eq!(
        panel.bindings.as_ref().unwrap().document().bindings.len(),
        2
    );
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
    assert_eq!(prepared.models().document.bindings.len(), 2);
    for guid in &guids {
        assert_eq!(
            prepared.models().document.bindings[&guid.to_string()].asset,
            "two.glb"
        );
    }
    let counts = orr_sample::room_view::check_model_poses(
        orr_bridge::FrameView::of(prepared.scene().frame()),
        prepared.scene().index(),
        prepared.models(),
        #[cfg(feature = "room-character")]
        None,
    )
    .unwrap();
    assert_eq!(counts.static_models, 2);
}
