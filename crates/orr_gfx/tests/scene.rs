//! The rendering components in scene files: names, defaults, reading, writing and baking.

use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_fp::{FPVec3, FP};
use orr_gfx::{names, Environment, Light, PostProcess, RenderLayers, Transform3, ALL_LAYERS, LIGHT_SPOT, TONE_CLAMP};
use orr_reflect::{Guid, Scene, TypeRegistry};

fn types() -> TypeRegistry {
    let mut types = TypeRegistry::new();
    orr_gfx::register_reflect(&mut types);
    types
}

fn frame() -> Frame {
    let mut builder = ComponentRegistryBuilder::new();
    orr_gfx::register(&mut builder);
    Frame::new(builder.build())
}

#[test]
fn simulation_and_scene_registries_agree() {
    let frame = frame();
    assert_eq!(types().check_against(frame.registry()), Vec::<String>::new());
}

#[test]
fn a_light_written_in_a_scene_file_bakes_into_the_frame() {
    let text = "schema: orr.scene/1
entities:
  e_00000001:
    name: desk_lamp
    orr_gfx::Transform3:
      position: [1.5, 3, -2]
      rotation: [60, 90, 0]
      scale: [1, 1, 1]
    orr_gfx::Light:
      color: [1, 0.8, 0.5]
      intensity: 2.5
      range: 6
      inner_angle: 15
      outer_angle: 25
      shadow_strength: 1
      kind: spot
      enabled: true
      cast_shadows: false
      layers: [default, layer3]
  e_00000002:
    name: hero
    orr_gfx::RenderLayers:
      layers: [layer3]
singletons:
  orr_gfx::Environment:
    sky_color: [0.2, 0.3, 0.6]
    ground_color: [0.1, 0.1, 0.1]
    ambient: 0.5
  orr_gfx::PostProcess:
    exposure: 1.5
    bloom_threshold: 1
    bloom_strength: 0.3
    tone_mapping: clamp
    bloom: true
    bloom_radius: 3
    bloom_iterations: 2
";
    let types = types();
    let scene = Scene::parse(text, &types).unwrap_or_else(|e| panic!("{e}"));
    let mut frame = frame();
    let index = scene.bake(&types, &mut frame).unwrap();
    let lamp = index.entity(&Guid::parse("e_00000001").unwrap()).unwrap();
    let hero = index.entity(&Guid::parse("e_00000002").unwrap()).unwrap();

    let light = frame.get::<Light>(lamp).unwrap();
    assert_eq!(light.kind, LIGHT_SPOT);
    assert_eq!((light.intensity, light.range), (FP::from_ratio(5, 2), FP::from_int(6)));
    assert_eq!((light.enabled, light.cast_shadows), (1, 0));
    assert_eq!(light.layers, 0b101);
    let transform = frame.get::<Transform3>(lamp).unwrap();
    assert_eq!(transform.position, FPVec3::new(FP::from_ratio(3, 2), FP::from_int(3), FP::from_int(-2)));
    assert_eq!(transform.rotation.x, FP::from_int(60));
    assert_eq!(frame.get::<RenderLayers>(hero).unwrap().layers, 0b100);
    assert!(frame.get::<RenderLayers>(lamp).is_none());

    let environment = frame.singleton::<Environment>();
    assert_eq!(environment.ambient, FP::from_ratio(1, 2));
    let post = frame.singleton::<PostProcess>();
    assert_eq!((post.tone_mapping, post.bloom, post.bloom_radius), (TONE_CLAMP, 1, 3));

    // Writing the frame back gives the same scene.
    let again = Scene::unbake(&types, &frame, Some(&index)).unwrap();
    assert_eq!(Scene::parse(&again.to_yaml(), &types).unwrap().to_yaml(), scene.to_yaml());
}

#[test]
fn a_scene_without_the_singletons_gets_their_defaults() {
    let types = types();
    let scene = Scene::parse("schema: orr.scene/1\nentities: {}\n", &types).unwrap();
    let mut frame = frame();
    scene.bake(&types, &mut frame).unwrap();
    assert_eq!(*frame.singleton::<Environment>(), Environment::default());
    assert_eq!(*frame.singleton::<PostProcess>(), PostProcess::default());
}

#[test]
fn a_component_added_without_values_starts_from_useful_defaults() {
    // The inspector's "Add component" and `orr add` start from these values.
    let types = types();
    let light = types.get(names::LIGHT).unwrap();
    let value = light.default_bytes();
    let light: &Light = bytemuck::from_bytes(&value);
    assert_eq!(*light, Light::default());
    assert_eq!((light.enabled, light.layers, light.intensity), (1, ALL_LAYERS, FP::ONE));
    let transform = types.get(names::TRANSFORM).unwrap().default_bytes();
    assert_eq!(*bytemuck::from_bytes::<Transform3>(&transform), Transform3::default());
    let layers = types.get(names::RENDER_LAYERS).unwrap().default_bytes();
    assert_eq!(bytemuck::from_bytes::<RenderLayers>(&layers).layers, orr_gfx::DEFAULT_LAYER);
}

#[test]
fn values_outside_their_ranges_are_rejected_in_scene_files() {
    let types = types();
    for bad in [
        "    orr_gfx::Light: { color: [2, 1, 1], intensity: 1, range: 1, inner_angle: 1, outer_angle: 2, shadow_strength: 1, kind: point, enabled: true, cast_shadows: false, layers: [] }",
        "    orr_gfx::Light: { color: [1, 1, 1], intensity: 1, range: 1, inner_angle: 1, outer_angle: 2, shadow_strength: 1, kind: laser, enabled: true, cast_shadows: false, layers: [] }",
        "    orr_gfx::RenderLayers: { layers: [layer9] }",
    ] {
        let text = format!("schema: orr.scene/1\nentities:\n  e_00000001:\n{bad}\n");
        assert!(Scene::parse(&text, &types).is_err(), "accepted: {bad}");
    }
}

#[test]
fn the_schema_documents_every_field() {
    let schema = types().json_schema();
    for needle in ["orr_gfx::Light", "Half angle in degrees", "The render layers this light affects", "orr_gfx::PostProcess", "Glow around very bright areas"] {
        assert!(schema.contains(needle), "the JSON schema lacks {needle:?}");
    }
}
