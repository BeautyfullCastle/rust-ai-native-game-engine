use super::*;
use orr_editor::model_bindings::Bindings;
#[test]
fn room_cleanup_keeps_closed_live_binding_and_saved_runtime_consumption() {
    let fixture = Fixture::new(false);
    let (editor, mut panel) = fixture.panel();
    let original = panel.bindings.as_ref().unwrap().document().clone();
    let cached = panel.placements(&editor)[0].model.clone();
    let unknown = Guid::parse("e_ffffffffffffffffffffffffffffffff").unwrap();
    let loaded = model_bindings::load_asset(&fixture.root, "admission-models", "one.glb").unwrap();
    panel
        .bindings
        .as_mut()
        .unwrap()
        .assign_validated(
            &[unknown],
            original.bindings.values().next().unwrap(),
            &loaded,
        )
        .unwrap();
    let bytes = fs::read(fixture.root.join("room.models.json")).unwrap();
    let checksum = editor.checksum();
    assert_eq!(panel.remove_orphaned(&editor).unwrap(), 1);
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &original);
    assert!(Arc::ptr_eq(&cached, &panel.placements(&editor)[0].model));
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(
        fs::read(fixture.root.join("room.models.json")).unwrap(),
        bytes
    );
    panel.save(&editor).unwrap();
    let prepared = PreparedProject::open(&fixture.root).unwrap();
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
#[test]
fn room_cleanup_cannot_commit_empty_or_stale_retained_state() {
    let fixture = Fixture::new(false);
    let (editor, mut panel) = fixture.panel();
    let original = panel.bindings.as_ref().unwrap().document().clone();
    let path = panel.bindings.as_ref().unwrap().path.clone();
    let unknown = Guid::parse("e_ffffffffffffffffffffffffffffffff").unwrap();
    let mut orphan = original.clone();
    let binding = orphan.bindings.values().next().unwrap().clone();
    orphan.bindings.clear();
    orphan.bindings.insert(unknown.to_string(), binding.clone());
    panel.bindings = Some(Bindings::from_document(path.clone(), orphan.clone()).unwrap());
    assert!(panel.remove_orphaned(&editor).is_err());
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &orphan);
    let mut stale = original.clone();
    stale.bindings.values_mut().next().unwrap().source_hash = "0".repeat(64);
    stale.bindings.insert(unknown.to_string(), binding);
    panel.bindings = Some(Bindings::from_document(path, stale.clone()).unwrap());
    assert!(panel.remove_orphaned(&editor).is_err());
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &stale);
    assert_eq!(
        fs::read(fixture.root.join("room.models.json")).unwrap(),
        serde_json::to_vec(&original).unwrap()
    );
}
