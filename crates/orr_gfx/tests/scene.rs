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
      shadow_range: 0
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
    tone_mapping: clamp
    effects:
      - { kind: bloom, enabled: true, params: [1, 0.3, 3, 2] }
      - { kind: vignette, enabled: true, params: [0.4, 0.8, 0.5, 0] }
      - { kind: none, enabled: false, params: [0, 0, 0, 0] }
      - { kind: none, enabled: false, params: [0, 0, 0, 0] }
      - { kind: none, enabled: false, params: [0, 0, 0, 0] }
      - { kind: none, enabled: false, params: [0, 0, 0, 0] }
      - { kind: none, enabled: false, params: [0, 0, 0, 0] }
      - { kind: none, enabled: false, params: [0, 0, 0, 0] }
  orr_gfx::Sky:
    brightness: 1.5
    sun_glow: 1
    cloud_cover: 0.34
    cloud_speed: 1
    fog_density: 1
    fog_falloff: 0.03
    fog_height: 0
    haze: 0.08
    wind_strength: 1
    wind_speed: 1
    enabled: false
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
    assert_eq!((post.tone_mapping, post.effects[0].kind, post.effects[0].params[2]), (TONE_CLAMP, orr_gfx::EFFECT_BLOOM, FP::from_int(3)));
    assert_eq!(post.effects[1].kind, orr_gfx::EFFECT_VIGNETTE);

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
    for needle in ["orr_gfx::Light", "Half angle in degrees", "The render layers this light affects", "orr_gfx::PostProcess", "The effects, in the order they run", "orr_gfx::Camera", "orr_gfx::Terrain", "Grayscale PNG next to the scene file"] {
        assert!(schema.contains(needle), "the JSON schema lacks {needle:?}");
    }
}

#[test]
fn a_material_names_its_shader_as_text_in_the_scene_file() {
    use orr_gfx::{Material, MATERIAL_VALUES, SHADER_NAME_BYTES};
    let types = types();
    let values: Vec<String> = (0..MATERIAL_VALUES).map(|i| if i == 3 { "12.5".into() } else { "0".into() }).collect();
    let text = format!("schema: orr.scene/1\nentities:\n  e_00000001:\n    orr_gfx::Material:\n      shader: stripes\n      values: [{}]\n      overrides: 8\n", values.join(", "));
    let scene = Scene::parse(&text, &types).unwrap_or_else(|e| panic!("{e}"));
    let mut frame = frame();
    let index = scene.bake(&types, &mut frame).unwrap();
    let e = index.entity(&Guid::parse("e_00000001").unwrap()).unwrap();
    let material = frame.get::<Material>(e).unwrap();
    assert_eq!(material.shader_name(), "stripes");
    assert_eq!((material.values[3], material.overrides), (FP::from_ratio(25, 2), 8));
    let written = Scene::unbake(&types, &frame, Some(&index)).unwrap().to_yaml();
    assert!(written.contains("shader: stripes"), "{written}");

    // A name that needs quotes in YAML is written with them and reads back the same.
    let mut odd = *material;
    odd.shader = Material::new("true").shader;
    frame.add(e, odd);
    let written = Scene::unbake(&types, &frame, Some(&index)).unwrap().to_yaml();
    assert!(written.contains("shader: \"true\""), "{written}");
    let again = Scene::parse(&written, &types).unwrap();
    let mut other = self::frame();
    let index = again.bake(&types, &mut other).unwrap();
    assert_eq!(other.get::<Material>(index.entity(&Guid::parse("e_00000001").unwrap()).unwrap()).unwrap().shader_name(), "true");

    // Text longer than the field and text with control characters are refused.
    let long = text.replace("shader: stripes", &format!("shader: {}", "x".repeat(SHADER_NAME_BYTES + 1)));
    assert!(Scene::parse(&long, &types).is_err());
    let control = text.replace("shader: stripes", "shader: \"a\tb\"");
    assert!(Scene::parse(&control, &types).is_err());
    assert_eq!(Material::new(&"y".repeat(100)).shader_name().len(), SHADER_NAME_BYTES);
}

#[test]
fn a_camera_and_a_terrain_round_trip_through_a_scene_file() {
    let types = types();
    let text = "schema: orr.scene/1
entities:
  e_00000001:
    name: eye
    orr_gfx::Camera:
      fov_y: 40
      half_height: 10
      near: 0.5
      far: 3000
      projection: perspective
      layers: [default, layer2]
      priority: 2
      active: true
  e_00000002:
    name: ground
    orr_gfx::Terrain:
      heightmap: terrain/valley.png
      size: [800, 600]
      height: 120
      resolution: 256
      cast_shadows: true
";
    let scene = Scene::parse(text, &types).unwrap_or_else(|e| panic!("{e}"));
    let mut frame = frame();
    let index = scene.bake(&types, &mut frame).unwrap();
    let ground = index.entity(&Guid::parse("e_00000002").unwrap()).unwrap();
    assert_eq!(frame.get::<orr_gfx::Terrain>(ground).unwrap().heightmap_path(), "terrain/valley.png");
    // Writing the frame back keeps both components as written; it adds the default singletons.
    let again = Scene::unbake(&types, &frame, Some(&index)).unwrap().to_yaml();
    let entities = &text[text.find("entities:").unwrap()..];
    assert!(again.ends_with(entities), "{again}");
    // A heightmap must be a relative PNG path.
    let bad = text.replace("terrain/valley.png", "../valley.png");
    assert!(Scene::parse(&bad, &types).is_err());
}
