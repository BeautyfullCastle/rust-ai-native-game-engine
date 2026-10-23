//! Keep the editor's serialized Room fixture identical to its original live setup.
//! Simulation construction belongs at this integration-test boundary, not in src.
#![cfg(feature = "room-project")]

use orr_sample::{
    room_game::{RoomConfig, RoomEscapeV1},
    room_project,
};

#[test]
fn room_initial_fixture_matches_original_simulation_bytes_and_admission() {
    let simulation = orr_sim::Simulation::<RoomEscapeV1>::new(RoomConfig, 60, room_project::SEED);
    let original = orr_reflect::Scene::unbake(&room_project::types(), simulation.frame(), None)
        .unwrap()
        .to_yaml();
    let fixture = include_str!("fixtures/room_initial.scene.yaml");
    assert_eq!(
        fixture, original,
        "preserve the original unit-test scene bytes"
    );
    let prepared = room_project::PreparedScene::parse(fixture).unwrap();
    assert_eq!(prepared.text(), fixture);
}
