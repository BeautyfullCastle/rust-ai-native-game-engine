#![cfg(all(
    feature = "room-project",
    feature = "project-create",
    target_os = "linux"
))]
use orr_bridge::FrameView;
use orr_model::MaterialOverride;
use orr_model_bindings::model_bindings::Document;
use orr_sample::{
    project_create::{self, CreateOptions},
    room_game::{INTERACT, KEY, PLAYER, RoomActor, RoomInput, RoomRun},
    room_project::PreparedProject,
    room_view,
};
use std::{collections::BTreeMap, fs, sync::Arc};

#[test]
fn room_shared_asset_overrides_survive_collection_restart_and_do_not_touch_simulation() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("material-room");
    project_create::create(&CreateOptions {
        output: root.clone(),
        template: project_create::ROOM_TEMPLATE.into(),
        seed: "material-room".into(),
    })
    .unwrap();
    let plain = PreparedProject::open(&root).unwrap();
    let original_scene = plain.scene().text().to_owned();
    let original_assets: BTreeMap<_, _> = plain
        .models()
        .assets
        .iter()
        .map(|(key, loaded)| {
            (
                key.clone(),
                loaded.static_model().unwrap().to_bytes().unwrap(),
            )
        })
        .collect();
    let path = root.join("room.models.json");
    let mut document: Document = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    document.version = 3;
    let ids: BTreeMap<_, _> = [PLAYER, KEY]
        .into_iter()
        .map(|kind| {
            let entity = plain
                .scene()
                .frame()
                .entities()
                .find(|&entity| {
                    plain
                        .scene()
                        .frame()
                        .get::<RoomActor>(entity)
                        .is_some_and(|actor| actor.kind == kind)
                })
                .unwrap();
            let guid = plain.scene().index().guid(entity).unwrap().to_string();
            (kind, (entity, guid))
        })
        .collect();
    for (kind, factor) in [(PLAYER, [0.1, 0.9, 0.3]), (KEY, [0.8, 0.1, 0.6])] {
        document
            .bindings
            .get_mut(&ids[&kind].1)
            .unwrap()
            .material_override = Some(MaterialOverride {
            material_slot: 0,
            base_color_factor: factor,
        });
    }
    fs::write(&path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
    let authored = PreparedProject::open(&root).unwrap();
    assert_eq!(authored.models().document, document);
    assert_eq!(authored.scene().text(), original_scene);
    for (key, bytes) in original_assets {
        assert_eq!(
            authored.models().assets[&key]
                .static_model()
                .unwrap()
                .to_bytes()
                .unwrap(),
            bytes
        );
    }
    let mut sim = authored.scene().simulation().unwrap();
    let initial = sim.frame().to_bytes();
    let initial_checksum = sim.frame().checksum();
    let placements = room_view::placements(
        FrameView::of(sim.frame()),
        authored.scene().index(),
        authored.models(),
    )
    .unwrap();
    let player = placements
        .iter()
        .find(|p| p.entity == ids[&PLAYER].0)
        .unwrap();
    let key = placements.iter().find(|p| p.entity == ids[&KEY].0).unwrap();
    assert!(Arc::ptr_eq(&player.model, &key.model));
    assert_ne!(
        player.instance.material_override,
        key.instance.material_override
    );
    assert_eq!(sim.frame().to_bytes(), initial);
    assert_eq!(sim.frame().checksum(), initial_checksum);
    // Run the real interaction system with identical inputs on the plain and
    // authored projects. Presentation overrides cannot affect collection.
    let mut plain_sim = plain.scene().simulation().unwrap();
    let mut inputs = orr_sim::TickInputs::new(sim.tick(), 1);
    inputs.set_input(
        orr_sim::PlayerSlot(0),
        RoomInput {
            buttons: INTERACT,
            ..Default::default()
        },
    );
    sim.step(&inputs);
    plain_sim.step(&inputs);
    assert_eq!(sim.frame().singleton::<RoomRun>().key_collected, 1);
    assert_eq!(sim.frame().to_bytes(), plain_sim.frame().to_bytes());
    let collected = sim.frame().to_bytes();
    let placements = room_view::placements(
        FrameView::of(sim.frame()),
        authored.scene().index(),
        authored.models(),
    )
    .unwrap();
    assert!(!placements.iter().any(|p| p.entity == ids[&KEY].0));
    assert_eq!(
        placements
            .iter()
            .find(|p| p.entity == ids[&PLAYER].0)
            .unwrap()
            .instance
            .material_override,
        document.bindings[&ids[&PLAYER].1].material_override
    );
    assert_eq!(sim.frame().to_bytes(), collected);
    let restart = authored.scene().simulation().unwrap();
    let placements = room_view::placements(
        FrameView::of(restart.frame()),
        authored.scene().index(),
        authored.models(),
    )
    .unwrap();
    assert_eq!(
        placements
            .iter()
            .find(|p| p.entity == ids[&KEY].0)
            .unwrap()
            .instance
            .material_override,
        document.bindings[&ids[&KEY].1].material_override
    );
    assert_eq!(restart.frame().to_bytes(), initial);
    assert_eq!(restart.frame().singleton::<RoomRun>().key_collected, 0);
}

#[test]
fn room_v3_admission_rejects_invalid_slots_legacy_and_tampered_asset_without_retargeting() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("material-room");
    project_create::create(&CreateOptions {
        output: root.clone(),
        template: project_create::ROOM_TEMPLATE.into(),
        seed: "material-room".into(),
    })
    .unwrap();
    let path = root.join("room.models.json");
    let original = fs::read(&path).unwrap();
    let mut doc: Document = serde_json::from_slice(&original).unwrap();
    let id = doc.bindings.keys().next().unwrap().clone();
    doc.version = 3;
    doc.bindings.get_mut(&id).unwrap().material_override = Some(MaterialOverride {
        material_slot: 255,
        base_color_factor: [0.5; 3],
    });
    fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
    assert!(
        PreparedProject::open(&root)
            .err()
            .unwrap()
            .contains("used static material slot")
    );
    doc.bindings
        .get_mut(&id)
        .unwrap()
        .material_override
        .as_mut()
        .unwrap()
        .material_slot = 0;
    doc.version = 2;
    fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
    assert!(
        PreparedProject::open(&root)
            .err()
            .unwrap()
            .contains("version-3")
    );
    doc.version = 3;
    doc.bindings.get_mut(&id).unwrap().source_hash = "0".repeat(64);
    fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
    assert!(
        PreparedProject::open(&root)
            .err()
            .unwrap()
            .contains("stale")
    );
    fs::write(path, original).unwrap();
    PreparedProject::open(&root).unwrap();
}
