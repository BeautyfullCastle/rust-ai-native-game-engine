#![cfg(all(
    feature = "room-character",
    feature = "project-create",
    target_os = "linux"
))]
use orr_sample::{
    project_create::{self, CreateOptions},
    room_project::{CheckpointSupport, PreparedProject},
};

fn create(root: &std::path::Path) {
    project_create::create(&CreateOptions {
        output: root.into(),
        template: project_create::ROOM_CHARACTER_TEMPLATE.into(),
        seed: "courier".into(),
    })
    .unwrap();
}
fn open(root: &std::path::Path) -> Result<PreparedProject, String> {
    PreparedProject::open_with_capabilities(root, false, CheckpointSupport::Disabled, true)
}
#[test]
fn creator_is_owned_explicit_and_deterministic_without_simulation_changes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let a = root.join("character-a");
    let b = root.join("character-b");
    let plain = root.join("plain");
    create(&a);
    create(&b);
    project_create::create(&CreateOptions {
        output: plain.clone(),
        template: project_create::ROOM_TEMPLATE.into(),
        seed: "courier".into(),
    })
    .unwrap();
    assert!(PreparedProject::open(&a).is_err());
    assert!(PreparedProject::open_with_options(&a, false, CheckpointSupport::Disabled).is_err());
    let a_project = open(&a).unwrap();
    let b_project = open(&b).unwrap();
    let plain_project = PreparedProject::open(&plain).unwrap();
    assert_eq!(
        a_project.character().unwrap().bytes,
        b_project.character().unwrap().bytes
    );
    assert_eq!(
        a_project.scene().frame().checksum(),
        plain_project.scene().frame().checksum()
    );
    assert_eq!(a_project.scene().text(), plain_project.scene().text());
    assert!(plain_project.character().is_none());
    assert!(a_project
        .models()
        .assets
        .values()
        .any(|asset| asset.animated_model().is_some()));
    let mut a_sim = a_project.scene().simulation().unwrap();
    let mut plain_sim = plain_project.scene().simulation().unwrap();
    // Replay identical inputs with/without character presentation. Every sampled
    // pose leaves authoritative bytes unchanged, not only its final checksum.
    for tick in 0..90 {
        let input = orr_sample::room_game::RoomInput {
            move_x: 1,
            move_z: if tick < 30 { 0 } else { -1 },
            buttons: if tick % 7 == 0 {
                orr_sample::room_game::INTERACT
            } else {
                0
            },
            ..Default::default()
        };
        let mut inputs = orr_sim::TickInputs::new(a_sim.tick(), 1);
        inputs.set_input(orr_sim::PlayerSlot(0), input);
        a_sim.step(&inputs);
        plain_sim.step(&inputs);
        let before = a_sim.frame().to_bytes();
        orr_sample::room_view::character_placement(
            orr_bridge::FrameView::of(a_sim.frame()),
            a_project.scene().index(),
            a_project.models(),
            orr_sample::room_view::PresentationTime::default(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(before, a_sim.frame().to_bytes());
        assert_eq!(before, plain_sim.frame().to_bytes());
        assert_eq!(a_sim.frame().checksum(), plain_sim.frame().checksum());
    }
}
#[test]
fn character_admission_rejects_wrong_player_missing_and_foreign_clips() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("room");
    create(&root);
    let path = root.join("room.character.json");
    let original = std::fs::read(&path).unwrap();
    let mut doc: serde_json::Value = serde_json::from_slice(&original).unwrap();
    for replacement in [
        serde_json::json!(31),
        serde_json::json!(0),
        serde_json::json!(null),
        serde_json::json!(-1),
    ] {
        doc["carrying"] = replacement;
        std::fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
        assert!(open(&root).is_err());
    }
    let mut doc: serde_json::Value = serde_json::from_slice(&original).unwrap();
    doc["player"] = serde_json::json!("e_000000000000000000000000000000ff");
    std::fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
    assert!(open(&root).is_err());
    std::fs::write(&path, &original).unwrap();
    assert!(open(&root).is_ok());
    std::fs::remove_file(&path).unwrap();
    assert!(open(&root).is_err());
}
#[test]
fn installed_asset_tampering_and_descriptor_removal_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("room");
    create(&root);
    let project = open(&root).unwrap();
    let manifest_path = root.join("orr.project.json");
    let original = std::fs::read(&manifest_path).unwrap();
    let mut manifest: serde_json::Value = serde_json::from_slice(&original).unwrap();
    manifest["entry"]
        .as_object_mut()
        .unwrap()
        .remove("character");
    std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(open(&root).is_err());
    std::fs::write(&manifest_path, &original).unwrap();
    let asset = project
        .models()
        .assets
        .values()
        .find(|asset| asset.animated_model().is_some())
        .unwrap();
    let path = root
        .join(".orr/packages/objects")
        .join(asset.package_digest())
        .join(asset.asset());
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.push(b' ');
    std::fs::write(path, bytes).unwrap();
    assert!(open(&root).is_err());
}
