//! `ViewRef`: a view asset id in a component, in scene files and in the frame.

use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_fp::{FPVec3, FP};
use orr_gfx::{names, view_asset_id, Transform3, ViewRef};
use orr_reflect::{Guid, Kind, Scene, TypeRegistry};

fn types() -> TypeRegistry {
    let mut types = TypeRegistry::new();
    orr_gfx::register_reflect(&mut types);
    orr_gfx::register_view_ref_reflect(&mut types);
    types
}

fn frame() -> Frame {
    let mut builder = ComponentRegistryBuilder::new();
    orr_gfx::register(&mut builder);
    orr_gfx::register_view_ref(&mut builder);
    Frame::new(builder.build())
}

#[test]
fn a_view_ref_is_eight_plain_bytes_and_the_id_comes_from_the_path() {
    let knight = ViewRef::at("avatars/knight.orrview");
    assert_eq!(bytemuck::bytes_of(&knight), &knight.asset.to_le_bytes());
    assert_eq!(knight.asset, view_asset_id("avatars/knight.orrview"));
    assert_ne!(knight.asset, ViewRef::at("avatars/mage.orrview").asset);
    assert_ne!(knight.asset, 0);
    assert_eq!(ViewRef::default().asset, 0, "zero is no asset");
    // The hash is a fixed function of the bytes of the path: this value must never change,
    // or every scene that stores an id would lose its asset.
    assert_eq!(view_asset_id("a.orrview"), 0x9e0945d919a704de, "FNV-1a 64 of the path");
}

#[test]
fn the_registries_agree_and_the_game_that_does_not_opt_in_keeps_its_frame_layout() {
    let frame = frame();
    assert_eq!(types().check_against(frame.registry()), Vec::<String>::new());

    let mut plain = ComponentRegistryBuilder::new();
    orr_gfx::register(&mut plain);
    let plain = plain.build();
    assert_eq!(plain.component_count() + 1, frame.registry().component_count());
    let mut only_gfx = TypeRegistry::new();
    orr_gfx::register_reflect(&mut only_gfx);
    assert!(only_gfx.get(names::VIEW_REF).is_none(), "register_reflect does not add ViewRef");
}

#[test]
fn the_descriptor_marks_the_number_as_a_view_asset() {
    let types = types();
    let desc = types.get(names::VIEW_REF).expect("registered").desc();
    let Kind::Struct { fields, .. } = &desc.kind else { panic!("a struct") };
    assert_eq!(fields.len(), 1);
    assert!(matches!(&fields[0].ty.kind, Kind::Int { asset: Some(kind), .. } if kind == "view"));
    let schema = types.json_schema();
    let at = schema.find("orr_gfx::ViewRef").expect("the schema names the type");
    assert!(schema[at..].contains("view asset"), "{}", &schema[at..(at + 600).min(schema.len())]);
}

#[test]
fn a_view_ref_round_trips_through_a_scene_file_also_above_the_signed_range() {
    let big = u64::MAX - 5;
    let text = format!(
        "schema: orr.scene/1\nentities:\n  e_00000001:\n    orr_gfx::Transform3:\n      position: [1, 0, 2]\n      rotation: [0, 0, 0]\n      scale: [1, 1, 1]\n    orr_gfx::ViewRef:\n      asset: {big}\n  e_00000002:\n    orr_gfx::ViewRef:\n      asset: {}\n",
        view_asset_id("avatars/knight.orrview")
    );
    let types = types();
    let scene = Scene::parse(&text, &types).expect("the scene parses");
    let mut frame = frame();
    let index = scene.bake(&types, &mut frame).expect("the scene bakes");

    let first = index.entity(&Guid::parse("e_00000001").unwrap()).unwrap();
    let second = index.entity(&Guid::parse("e_00000002").unwrap()).unwrap();
    assert_eq!(frame.get::<ViewRef>(first), Some(&ViewRef { asset: big }));
    assert_eq!(frame.get::<ViewRef>(second), Some(&ViewRef::at("avatars/knight.orrview")));
    assert!(frame.has::<Transform3>(first));

    let back = Scene::unbake(&types, &frame, Some(&index)).expect("the frame unbakes");
    let yaml = back.to_yaml();
    assert!(yaml.contains("orr_gfx::ViewRef: { asset: 18446744073709551610 }"), "{yaml}");
    assert!(yaml.contains(&format!("orr_gfx::ViewRef: {{ asset: {} }}", ViewRef::at("avatars/knight.orrview").asset)), "{yaml}");
}

#[test]
fn a_view_ref_above_the_unsigned_range_is_refused() {
    let text = "schema: orr.scene/1\nentities:\n  e_00000001:\n    orr_gfx::ViewRef:\n      asset: 18446744073709551616\n";
    assert!(Scene::parse(text, &types()).is_err());
}

const FRAGMENT: &str = "schema: orr.scene/1
entities:
  e_00000001:
    name: knight
    orr_gfx::Transform3: { position: [0, 1, 0], rotation: [0, 0, 0], scale: [2, 2, 2] }
    orr_gfx::ViewRef: { asset: 77 }
  e_00000002:
    name: marker
    orr_gfx::ViewRef: { asset: 78 }
";

#[test]
fn a_scene_fragment_bakes_into_prefabs_in_guid_order_that_spawn_with_their_values() {
    let types = types();
    let mut builder = ComponentRegistryBuilder::new();
    orr_gfx::register(&mut builder);
    orr_gfx::register_view_ref(&mut builder);
    orr_ecs::prefab::register(&mut builder);
    let registry = builder.build();
    let scene = Scene::parse(FRAGMENT, &types).expect("the fragment parses");

    let prefabs = scene.bake_prefabs(&types, &registry).expect("the fragment bakes");
    assert_eq!(prefabs.len(), 2);
    let mut frame = Frame::new(registry.clone());
    orr_ecs::prefab::install(&mut frame, &prefabs).unwrap();
    let knight = orr_ecs::spawn_prefab(&mut frame, orr_ecs::PrefabId(0)).unwrap();
    let marker = orr_ecs::spawn_prefab(&mut frame, orr_ecs::PrefabId(1)).unwrap();

    assert_eq!(frame.get::<ViewRef>(knight), Some(&ViewRef { asset: 77 }));
    assert_eq!(frame.get::<Transform3>(knight).map(|t| t.scale), Some(FPVec3::new(FP::from_int(2), FP::from_int(2), FP::from_int(2))));
    assert_eq!(frame.get::<ViewRef>(marker), Some(&ViewRef { asset: 78 }));
    assert!(!frame.has::<Transform3>(marker), "a component the fragment does not name is not added");
}

#[test]
fn a_fragment_that_names_a_component_the_game_did_not_register_is_refused_with_its_name() {
    let types = types();
    let text = "schema: orr.scene/1
entities:
  e_00000001:
    orr_gfx::Transform3: { position: [0, 0, 0], rotation: [0, 0, 0], scale: [1, 1, 1] }
";
    let mut builder = ComponentRegistryBuilder::new();
    builder.register_component::<ViewRef>(names::VIEW_REF);
    let registry = builder.build();
    let scene = Scene::parse(text, &types).unwrap();

    let error = scene.bake_prefabs(&types, &registry).unwrap_err();

    assert!(error.to_string().contains("orr_gfx::Transform3"), "{error}");
}
