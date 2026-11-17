//! `PlayerSlot` and `PlayerLink` as components: byte layout and the scene file round trip.

use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_reflect::{Guid, Scene, TypeRegistry};
use orr_sim::{register_player_link, register_player_link_reflect, PlayerLink, PlayerSlot};

const SCENE: &str = "schema: orr.scene/1\nentities:\n  e_00000001:\n    PlayerLink: { player: 1 }\n  e_00000002:\n    PlayerLink: { player: 255 }\n";

fn types() -> TypeRegistry {
    let mut types = TypeRegistry::new();
    register_player_link_reflect(&mut types);
    types
}

fn frame() -> Frame {
    let mut builder = ComponentRegistryBuilder::new();
    register_player_link(&mut builder);
    Frame::new(builder.build())
}

#[test]
fn player_slot_and_link_are_plain_bytes() {
    assert_eq!(bytemuck::bytes_of(&PlayerSlot(3)), &[3]);
    assert_eq!(bytemuck::bytes_of(&PlayerSlot::NONE), &[0xFF]);
    assert_eq!(bytemuck::bytes_of(&PlayerLink { player: PlayerSlot(2) }), &[2]);
    assert_eq!(bytemuck::pod_read_unaligned::<PlayerLink>(&[7]).player, PlayerSlot(7));
    assert!(PlayerSlot(0).is_some());
    assert!(!PlayerSlot::NONE.is_some());
}

#[test]
fn registered_types_match_the_component_registry() {
    let mut builder = ComponentRegistryBuilder::new();
    register_player_link(&mut builder);
    let registry = builder.build();
    assert!(types().check_against(&registry).is_empty());
}

#[test]
fn player_link_round_trips_through_a_scene_file_as_a_number() {
    let types = types();
    let scene = Scene::parse(SCENE, &types).expect("the scene parses");
    let mut frame = frame();
    let index = scene.bake(&types, &mut frame).expect("the scene bakes");

    let mut links: Vec<PlayerLink> = Vec::new();
    for guid in ["e_00000001", "e_00000002"] {
        let entity = index.entity(&Guid::parse(guid).unwrap()).unwrap();
        links.push(*frame.get::<PlayerLink>(entity).unwrap());
    }
    assert_eq!(links, vec![PlayerLink { player: PlayerSlot(1) }, PlayerLink { player: PlayerSlot::NONE }]);

    let back = Scene::unbake(&types, &frame, Some(&index)).expect("the frame unbakes");
    assert_eq!(back.to_yaml(), SCENE);
}

#[test]
fn a_player_number_above_255_is_refused_by_the_scene_reader() {
    let text = SCENE.replace("255", "256");
    assert!(Scene::parse(&text, &types()).is_err());
}
