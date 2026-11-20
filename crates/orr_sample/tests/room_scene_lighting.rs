//! Room lighting lives in the scene: an environment, a sun and a point light are entities and
//! singletons of the Room frame, read by the editor and by the exported player alike.
//! cargo test -p orr_sample --features room-project,project-create --test room_scene_lighting
#![cfg(feature = "room-project")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

#[path = "support/room_lit_scene.rs"]
mod room_lit_scene;

use orr_bridge::FrameView;
use orr_fp::FPVec3;
use orr_gfx::{Environment, Light, LIGHT_DIRECTIONAL, LIGHT_POINT};
use orr_sample::{
    room_game::{RoomActor, RoomConfig, RoomEscapeV1, RoomInput, RoomLightRule, RoomRun, PLAYER, TICK_RATE},
    room_project::{self, PreparedScene},
    room_view,
};
use orr_sim::{PlayerSlot, Simulation, TickInputs};

/// The text of the canonical Room, as a scene file holds it.
fn base_text() -> String {
    let simulation = Simulation::<RoomEscapeV1>::new(RoomConfig, TICK_RATE, room_project::SEED);
    orr_reflect::Scene::unbake(&room_project::types(), simulation.frame(), None).unwrap().to_yaml()
}

fn parse(text: &str) -> Result<PreparedScene, String> {
    PreparedScene::parse(text)
}

fn walk_right(sim: &mut Simulation<RoomEscapeV1>, ticks: u32) {
    for _ in 0..ticks {
        let mut inputs = TickInputs::new(sim.tick() + 1, 1);
        inputs.set_input(PlayerSlot(0), RoomInput { move_x: 1, ..Default::default() });
        sim.step(&inputs);
    }
}

fn player_position(frame: &orr_ecs::Frame) -> FPVec3 {
    let view = FrameView::of(frame);
    let (entity, _) = view.iter::<RoomActor>().find(|(_, actor)| actor.kind == PLAYER).unwrap();
    frame.get::<orr_physics3d::Body>(entity).unwrap().pos
}

#[test]
fn a_scene_with_lights_is_admitted_and_keeps_them_in_its_frame() {
    let plain = parse(&base_text()).unwrap();
    let lit_text = room_lit_scene::with_lights(&base_text());
    let lit = parse(&lit_text).unwrap();
    let frame = lit.frame();
    let kinds: Vec<u32> = FrameView::of(frame).iter::<Light>().map(|(_, light)| light.kind).collect();
    assert_eq!(kinds.len(), 2);
    assert!(kinds.contains(&LIGHT_DIRECTIONAL) && kinds.contains(&LIGHT_POINT));
    assert_eq!(frame.count::<RoomLightRule>(), 1);
    assert_eq!(frame.singleton::<Environment>().ambient, orr_fp::fp!(0.4));
    assert_ne!(frame.checksum(), plain.frame().checksum(), "the lights are part of the frame");
    // The scene text is kept as it was given, and the lights survive a save.
    assert_eq!(lit.text(), lit_text);
    let saved = orr_reflect::Scene::unbake(&room_project::types(), frame, Some(lit.index())).unwrap().to_yaml();
    let reopened = parse(&saved).unwrap();
    assert_eq!(reopened.frame().checksum(), frame.checksum());
    // A session and a simulation start from the frame as it is.
    lit.session().unwrap();
    assert_eq!(lit.simulation().unwrap().frame().checksum(), frame.checksum());
}

#[test]
fn lights_do_not_change_the_simulation_and_restart_restores_the_frame() {
    let plain = parse(&base_text()).unwrap();
    let lit = parse(&room_lit_scene::with_lights(&base_text())).unwrap();
    let mut a = plain.simulation().unwrap();
    let mut b = lit.simulation().unwrap();
    walk_right(&mut a, 30);
    walk_right(&mut b, 30);
    assert_eq!(player_position(a.frame()), player_position(b.frame()));
    assert_eq!(a.frame().singleton::<RoomRun>(), b.frame().singleton::<RoomRun>());
    assert_eq!(b.frame().count::<Light>(), 2);
    // The lights read the run state: the point light follows the walking player.
    let at_start = room_view::point_light_settings(FrameView::of(lit.frame())).point_light.unwrap();
    let walking = room_view::point_light_settings(FrameView::of(b.frame())).point_light.unwrap();
    assert_eq!(at_start.position, [0.0, 2.5, 0.0]);
    assert!(walking.position[0] > 0.1 && walking.position[1] == 2.5, "{:?}", walking.position);
    let restarted = lit.simulation().unwrap();
    assert_eq!(restarted.frame().checksum(), lit.frame().checksum());
}

#[test]
fn a_scene_refuses_what_a_light_entity_may_not_carry_and_more_than_two_lights() {
    let lit = room_lit_scene::with_lights(&base_text());
    parse(&lit).unwrap();
    // Each text and a part of the reason the scene gives for refusing it.
    let refused: Vec<(&str, String, &str)> = vec![
        // Another render component next to the light.
        ("RenderLayers on a light", lit.replacen("    name: point_light\n", "    name: point_light\n    orr_gfx::RenderLayers:\n      layers: [default]\n", 1), "Transform3, Light and optionally LightRule"),
        // A rule on the sun.
        ("rule on the sun", lit.replacen("    name: sun\n", "    name: sun\n    RoomEscapeV1::LightRule:\n      follow: false\n      activation: always\n      offset: [0, 0, 0]\n", 1), "light rule belongs to a point light"),
        // A light with an intensity past the inspector range.
        ("intensity out of range", lit.replacen("    orr_gfx::Light:\n      color: [1, 0.9, 0.7]\n      intensity: 3\n      range: 12\n", "    orr_gfx::Light:\n      color: [1, 0.9, 0.7]\n      intensity: 300\n      range: 12\n", 1), "300 is outside [0, 100]"),
        // A third light.
        ("third light", format!("{lit}{}", room_lit_scene::point_entity("e_20000003")), "at most one directional light and one point light"),
        // A light that is not placed.
        ("Light without Transform3", lit.replacen("    orr_gfx::Transform3:\n      position: [0, 4, 0]\n      rotation: [0, 0, 0]\n      scale: [1, 1, 1]\n", "", 1), "Transform3, Light and optionally LightRule"),
        // A rule that follows with a wrong offset size.
        ("offset out of range", lit.replacen("offset: [0, 2, 0]", "offset: [0, 40, 0]", 1), "40 is outside [-32, 32]"),
        // An unknown singleton.
        ("unknown singleton", lit.replacen("singletons:\n", "singletons:\n  orr_gfx::Camera: {}\n", 1), ""),
        // A wall may not carry a render component either.
        ("RenderLayers on a wall", base_text().replacen("    RoomEscapeV1::Actor: { kind: 4, ordinal: 0 }\n", "    RoomEscapeV1::Actor: { kind: 4, ordinal: 0 }\n    orr_gfx::RenderLayers:\n      layers: [default]\n", 1), "Transform3, Light and optionally LightRule"),
    ];
    for (what, text, reason) in refused {
        let error = parse(&text).err().unwrap_or_else(|| panic!("{what} was admitted"));
        assert!(error.contains(reason), "{what}: refused for another reason: {error}");
    }
}

#[test]
fn a_scene_saved_before_the_lighting_components_opens_with_its_old_lighting() {
    // The scene as it was saved before the game had render components: no gfx singleton at all.
    let text = base_text();
    let start = text.find("  orr_gfx::Environment:").unwrap();
    let end = text.find("  orr_physics3d::PhysicsState:").unwrap();
    let old = format!("{}{}", &text[..start], &text[end..]);
    let prepared = parse(&old).unwrap();
    assert_eq!(*prepared.frame().singleton::<Environment>(), Environment::default());
    let frame = FrameView::of(prepared.frame());
    let mut legacy = orr_sample::yard3d_view::yard_lighting();
    legacy.shadows = false;
    assert_eq!(room_view::lighting_for_frame(frame), legacy, "no light, so the game's own lighting");
    assert_eq!(room_view::point_light_settings(frame), Default::default());
    // The same frame as the freshly built room.
    assert_eq!(prepared.frame().checksum(), parse(&text).unwrap().frame().checksum());
}
