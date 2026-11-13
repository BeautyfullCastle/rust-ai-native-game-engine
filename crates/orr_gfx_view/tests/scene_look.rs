//! Reading the rendering components of a frame into renderer inputs.
#![allow(clippy::float_arithmetic)]

use orr_bridge::FrameView;
use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_fp::{FPVec3, FP};
use orr_gfx::{Environment, Light, PostProcess, RenderLayers, Transform3, LIGHT_DIRECTIONAL, LIGHT_POINT, LIGHT_SPOT, TONE_CLAMP};
use orr_gfx_view::{facing, layers_of, scene_look};
use orr_render::{Lighting, RenderList3D, SceneLightKind, MAX_SCENE_LIGHTS};

fn gfx_frame() -> Frame {
    let mut builder = ComponentRegistryBuilder::new();
    orr_physics3d::register(&mut builder);
    orr_gfx::register(&mut builder);
    let mut frame = Frame::new(builder.build());
    orr_gfx::init(&mut frame);
    frame
}

/// A directional light that casts shadows, as a scene's sun is usually set up.
fn sun(color: FPVec3, intensity: FP) -> Light {
    Light { color, intensity, kind: LIGHT_DIRECTIONAL, cast_shadows: 1, ..Light::default() }
}

fn fallback() -> Lighting {
    Lighting { direction: [0.0, -1.0, 0.0], color: [0.9, 0.9, 0.8], intensity: 2.0, shadow_radius: 25.0, ..Lighting::default() }
}

fn int(v: i32) -> FP {
    FP::from_int(v)
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1.0e-3)
}

fn light_at(frame: &mut Frame, position: [i32; 3], rotation: [i32; 3], light: Light) -> orr_ecs::Entity {
    let e = frame.spawn();
    frame.add(e, Transform3 { position: FPVec3::new(int(position[0]), int(position[1]), int(position[2])), rotation: FPVec3::new(int(rotation[0]), int(rotation[1]), int(rotation[2])), ..Transform3::default() });
    frame.add(e, light);
    e
}

#[test]
fn a_game_without_the_components_keeps_its_own_lighting() {
    let mut builder = ComponentRegistryBuilder::new();
    orr_physics3d::register(&mut builder);
    let frame = Frame::new(builder.build());
    let look = scene_look(FrameView::of(&frame), &fallback());
    assert_eq!(look.lighting, fallback());
    assert!(look.scene_lights.lights().is_empty() && look.bloom.is_none() && look.lights.is_empty());
    let e = orr_ecs::Entity { index: 0, version: 0 };
    assert_eq!(layers_of(FrameView::of(&frame), e), orr_gfx::DEFAULT_LAYER);
}

#[test]
fn a_scene_with_default_values_and_no_lights_looks_like_the_game() {
    let frame = gfx_frame();
    let look = scene_look(FrameView::of(&frame), &fallback());
    assert_eq!(look.lighting, fallback());
    assert_eq!(look.scene_lights, orr_render::SceneLights::default());
}

#[test]
fn the_first_directional_light_is_the_sun_and_the_rest_are_scene_lights() {
    let mut frame = gfx_frame();
    // Pitch 90 points straight down.
    let sun_entity = light_at(&mut frame, [0, 10, 0], [90, 0, 0], Light { layers: 0b11, shadow_strength: FP::from_ratio(1, 2), ..sun(FPVec3::new(FP::ONE, FP::from_ratio(1, 2), FP::ZERO), int(3)) });
    let lamp = light_at(&mut frame, [2, 1, -3], [0, 0, 0], Light { kind: LIGHT_POINT, range: int(5), layers: 0b10, ..Light::default() });
    let spot = light_at(&mut frame, [0, 4, 0], [90, 0, 0], Light { kind: LIGHT_SPOT, inner_angle: int(10), outer_angle: int(20), ..Light::default() });
    let second_sun = light_at(&mut frame, [0, 0, 0], [0, 0, 0], Light { kind: LIGHT_DIRECTIONAL, ..Light::default() });
    let look = scene_look(FrameView::of(&frame), &fallback());

    assert!(close(look.lighting.direction, [0.0, -1.0, 0.0]), "{:?}", look.lighting.direction);
    assert!(close(look.lighting.color, [1.0, 0.5, 0.0]) && look.lighting.intensity == 3.0 && look.lighting.shadows);
    assert_eq!(look.lighting.shadow_radius, 25.0, "the game's shadow area is kept");
    assert_eq!((look.scene_lights.sun_layers, look.scene_lights.sun_shadow_strength), (0b11, 0.5));

    let lights = look.scene_lights.lights();
    assert_eq!(lights.len(), 3);
    assert_eq!((lights[0].kind, lights[0].position, lights[0].range, lights[0].layers), (SceneLightKind::Point, [2.0, 1.0, -3.0], 5.0, 0b10));
    assert_eq!(lights[1].kind, SceneLightKind::Spot);
    assert!((lights[1].inner_angle - 10_f32.to_radians()).abs() < 1.0e-5 && (lights[1].outer_angle - 20_f32.to_radians()).abs() < 1.0e-5);
    assert_eq!(lights[2].kind, SceneLightKind::Directional, "a second directional light is an ordinary light");
    let sun = sun_entity;
    let entities: Vec<_> = look.lights.iter().map(|placed| (placed.entity, placed.sun)).collect();
    assert_eq!(entities, [(sun, true), (lamp, false), (spot, false), (second_sun, false)]);
}

#[test]
fn disabled_and_dark_lights_give_no_light_and_a_scene_of_only_those_has_no_sun() {
    let mut frame = gfx_frame();
    light_at(&mut frame, [0, 5, 0], [90, 0, 0], Light { enabled: 0, ..sun(FPVec3::new(FP::ONE, FP::ONE, FP::ONE), int(2)) });
    light_at(&mut frame, [0, 1, 0], [0, 0, 0], Light { intensity: FP::ZERO, ..Light::default() });
    let look = scene_look(FrameView::of(&frame), &fallback());
    assert_eq!(look.lighting.intensity, 0.0, "the scene's lights replace the game's sun");
    assert!(!look.lighting.shadows && look.scene_lights.lights().is_empty());
    // Both lights still have a marker, so a person can find them and switch them on.
    assert_eq!(look.lights.len(), 2);
    assert!(look.lights.iter().all(|placed| !placed.enabled && !placed.sun));
    let mut list = RenderList3D::new();
    look.draw_markers(&mut list, None);
    assert!(list.lines.iter().all(|line| line.color == [0.45, 0.45, 0.45, 0.8]), "switched off lights have gray markers");
}

#[test]
fn values_a_system_wrote_out_of_range_are_brought_back_into_range() {
    let mut frame = gfx_frame();
    light_at(&mut frame, [0, 1, 0], [0, 0, 0], Light { color: FPVec3::new(int(5), int(-1), FP::ONE), intensity: int(500), range: FP::ZERO, inner_angle: int(-30), outer_angle: int(200), ..Light::default() });
    frame.set_singleton(PostProcess { exposure: FP::ZERO, bloom: 1, bloom_threshold: int(-1), bloom_strength: int(100), ..PostProcess::default() });
    let look = scene_look(FrameView::of(&frame), &fallback());
    let light = look.scene_lights.lights().first().expect("the light is kept, not dropped");
    assert_eq!((light.color, light.intensity, light.range), ([1.0, 0.0, 1.0], 100.0, 0.01));
    assert!(light.inner_angle == 0.0 && (light.outer_angle - 89_f32.to_radians()).abs() < 1.0e-5);
    assert_eq!(look.dropped_lights, 0);
    assert_eq!(look.lighting.exposure, 0.01);
    let bloom = look.bloom.unwrap();
    assert_eq!((bloom.threshold, bloom.strength), (0.0, 8.0));
    // The renderer accepts every value: the post chain validates bloom and refuses bad values.
    let settings = orr_render::PostProcessSettings { enabled: true, bloom: true, threshold: bloom.threshold, strength: bloom.strength, radius: bloom.radius, iterations: bloom.iterations };
    assert!(settings.validate().is_ok());
}

#[test]
fn a_light_without_layers_lights_nothing_and_an_entity_without_layers_is_on_the_default_layer() {
    let mut frame = gfx_frame();
    light_at(&mut frame, [0, 1, 0], [0, 0, 0], Light { layers: 0, ..Light::default() });
    let e = frame.spawn();
    frame.add(e, RenderLayers::new(0));
    let look = scene_look(FrameView::of(&frame), &fallback());
    assert_eq!(look.scene_lights.lights()[0].layers, 0);
    assert_eq!(layers_of(FrameView::of(&frame), e), 0, "the renderer draws layer mask 0 on the default layer");
}

#[test]
fn lights_beyond_the_frame_limit_are_counted_not_drawn() {
    let mut frame = gfx_frame();
    for i in 0..MAX_SCENE_LIGHTS as i32 + 3 {
        light_at(&mut frame, [i, 1, 0], [0, 0, 0], Light::default());
    }
    let look = scene_look(FrameView::of(&frame), &fallback());
    assert_eq!(look.scene_lights.lights().len(), MAX_SCENE_LIGHTS);
    assert_eq!(look.dropped_lights, 3);
}

#[test]
fn a_light_on_a_physics_body_moves_with_the_body() {
    let mut frame = gfx_frame();
    let e = frame.spawn();
    let mut body: orr_physics3d::Body = bytemuck::Zeroable::zeroed();
    body.rot.w = FP::ONE;
    body.pos = FPVec3::new(int(4), int(2), int(1));
    frame.add(e, body);
    frame.add(e, Transform3 { position: FPVec3::new(int(-50), int(0), int(0)), ..Transform3::default() });
    frame.add(e, Light::default());
    let look = scene_look(FrameView::of(&frame), &fallback());
    assert_eq!(look.scene_lights.lights()[0].position, [4.0, 2.0, 1.0], "the body places the light, not the Transform3");
}

#[test]
fn environment_and_post_process_set_ambient_exposure_tone_mapping_and_bloom() {
    let mut frame = gfx_frame();
    frame.set_singleton(Environment { sky_color: FPVec3::new(FP::ZERO, FP::ZERO, FP::ONE), ground_color: FPVec3::ZERO, ambient: int(2) });
    frame.set_singleton(PostProcess { exposure: FP::from_ratio(3, 2), tone_mapping: TONE_CLAMP, bloom: 1, bloom_radius: 20, bloom_iterations: 0, ..PostProcess::default() });
    let look = scene_look(FrameView::of(&frame), &fallback());
    assert_eq!((look.lighting.sky, look.lighting.ground, look.lighting.ambient), ([0.0, 0.0, 1.0], [0.0; 3], 2.0));
    assert_eq!((look.lighting.exposure, look.lighting.tonemap), (1.5, false));
    let bloom = look.bloom.expect("bloom is on");
    assert_eq!((bloom.radius, bloom.iterations), (8, 1), "out-of-range counts are clamped");
    assert!((bloom.strength - 0.15).abs() < 1.0e-3);
}

#[test]
fn render_layers_of_an_entity_and_the_facing_direction() {
    let mut frame = gfx_frame();
    let plain = frame.spawn();
    let tagged = frame.spawn();
    frame.add(tagged, RenderLayers::new(0b1010));
    let view = FrameView::of(&frame);
    assert_eq!((layers_of(view, plain), layers_of(view, tagged)), (1, 0b1010));
    assert!(close(facing([0.0, 0.0, 0.0]), [0.0, 0.0, 1.0]));
    assert!(close(facing([0.0, 90.0, 0.0]), [1.0, 0.0, 0.0]));
    assert!(close(facing([90.0, 0.0, 0.0]), [0.0, -1.0, 0.0]));
}

#[test]
fn the_look_fills_a_render_list_and_draws_light_markers() {
    let mut frame = gfx_frame();
    let lamp = light_at(&mut frame, [0, 2, 0], [0, 0, 0], Light::default());
    light_at(&mut frame, [0, 6, 0], [45, 0, 0], Light { kind: LIGHT_SPOT, ..Light::default() });
    let look = scene_look(FrameView::of(&frame), &fallback());
    let mut list = RenderList3D::new();
    look.apply(&mut list);
    assert_eq!(list.scene_lights.lights().len(), 2);
    look.draw_markers(&mut list, None);
    // A point light draws a three-line cross; a spot light a cross and its direction.
    assert_eq!(list.lines.len(), 3 + 4);
    let thin = list.lines[0].width;
    list.lines.clear();
    look.draw_markers(&mut list, Some(lamp));
    assert!(list.lines[0].width > thin, "the selected light's marker is wider");
}

#[test]
fn materials_give_overridden_values_and_shader_defaults_elsewhere() {
    let shader = orr_render::SurfaceShader::compile(
        "stripes",
        "// @param tint color = 0.5, 0.25, 1\n// @param stripes float 1..40 = 8\nfn surface(in: SurfaceInput) -> Surface { return default_surface(in); }",
    )
    .unwrap();
    let mut frame = gfx_frame();
    let plain = frame.spawn();
    let e = frame.spawn();
    let mut material = orr_gfx::Material::new("stripes");
    material.values[1] = FP::from_ratio(3, 4);
    material.values[3] = int(100);
    material.values[2] = int(9);
    material.overrides = 0b1010;
    frame.add(e, material);
    let found = orr_gfx_view::materials(FrameView::of(&frame));
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].entity, found[0].shader.as_str()), (e, "stripes"));
    let values = found[0].values_for(&shader);
    // Bit 1 takes the stored 0.75; bit 3 takes 100, clamped to the declared 40; bit 2 is not set.
    assert_eq!(&values[..4], &[0.5, 0.75, 1.0, 40.0]);
    assert!(orr_gfx_view::materials(FrameView::of(&frame)).iter().all(|m| m.entity != plain));
}
