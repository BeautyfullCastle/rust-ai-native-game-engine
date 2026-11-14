//! Post effects of the imported scene's HDR chain, checked on the picture:
//! vignette, color grade, light shafts, auto exposure, ambient occlusion,
//! toon steps, outlines, FXAA and upscaling.
#![cfg(feature = "imported-scene")]
#![allow(clippy::float_arithmetic)]

use orr_model::{StaticModel, Vertex, IDENTITY};
use orr_render::orr_rhi::{Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions};
use orr_render::{
    Camera3D, ImportedBatch, ImportedSceneRenderer, ImportedSceneTarget, Lighting, ModelRenderer, PointLightSettings, PostProcessSettings,
    ScreenEffect, StaticInstance,
};

const SIZE: u32 = 64;
const FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;

fn gpu() -> Option<Wgpu> {
    match Wgpu::headless(WgpuOptions::default()) {
        Ok(g) if g.texture_format_capabilities(TextureFormat::Rgba16Float).supports_hdr_postprocessing => Some(g),
        Ok(_) => {
            eprintln!("SKIP: no HDR post processing on this adapter");
            None
        }
        Err(e) => {
            assert!(std::env::var_os("ORR_REQUIRE_GPU").is_none(), "required GPU unavailable: {e}");
            eprintln!("SKIP: {e}");
            None
        }
    }
}

/// A unit square facing the camera, in one solid color, made from the imported test model.
fn square(color: [f32; 3]) -> StaticModel {
    let imported = orr_model::import::import_with_resolver("fixtures/model.glb", include_bytes!("../../orr_model/tests/fixtures/model.glb"), |_| {
        panic!("fixture embeds every dependency")
    })
    .unwrap();
    let mut source = StaticModel::from_bytes(&imported.to_bytes().unwrap()).unwrap().source().clone();
    source.primitives.truncate(1);
    let p = &mut source.primitives[0];
    p.vertices = [[-0.5, -0.5, 0.0], [0.5, -0.5, 0.0], [0.5, 0.5, 0.0], [-0.5, 0.5, 0.0]]
        .into_iter()
        .map(|position| Vertex { position, normal: [0.0, 0.0, 1.0], uv: [0.25, 0.25] })
        .collect();
    p.indices = vec![0, 1, 2, 0, 2, 3];
    p.transform = IDENTITY;
    p.material = 0;
    source.materials[0].base_color = [color[0], color[1], color[2], 1.0];
    for image in &mut source.images {
        for pixel in image.rgba8.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[255, 255, 255, 255]);
        }
    }
    StaticModel::new(source).unwrap()
}

fn light(ambient: f32) -> Lighting {
    Lighting { sky: [1.0; 3], ground: [1.0; 3], ambient, intensity: 0.0, shadows: false, tonemap: false, exposure: 1.0, ..Default::default() }
}

fn render(g: &Wgpu, color: [f32; 3], scale: f32, light: &Lighting, effects: &[ScreenEffect]) -> Vec<u8> {
    render_from(g, &Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 1.0), color, scale, light, effects)
}

fn render_from(g: &Wgpu, camera: &Camera3D, color: [f32; 3], scale: f32, light: &Lighting, effects: &[ScreenEffect]) -> Vec<u8> {
    render_instances(g, camera, color, &[StaticInstance { scale: [scale; 3], ..Default::default() }], light, effects)
}

fn render_instances(g: &Wgpu, camera: &Camera3D, color: [f32; 3], instances: &[StaticInstance], light: &Lighting, effects: &[ScreenEffect]) -> Vec<u8> {
    let mut scene = ImportedSceneRenderer::new(g.clone(), FORMAT).unwrap();
    scene.post_process = PostProcessSettings { enabled: true, bloom: false, ..Default::default() };
    for (slot, effect) in scene.post_process.effects.iter_mut().zip(effects) {
        *slot = Some(*effect);
    }
    // With HDR on, batches draw into the scene's linear RGBA16F target.
    let mut renderer = ModelRenderer::new(g.clone(), TextureFormat::Rgba16Float, square(color)).unwrap();
    let texture = g.create_texture(&TextureDesc {
        label: "post effect test",
        width: SIZE,
        height: SIZE,
        format: FORMAT,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let view = g.create_texture_view(&texture, None);
    scene
        .draw(
            ImportedSceneTarget { view: &view, size: (SIZE, SIZE), format: FORMAT, sample_count: 1 },
            camera,
            light,
            &PointLightSettings::default(),
            &mut [ImportedBatch::StaticInstances { renderer: &mut renderer, instances }],
        )
        .unwrap();
    g.read_texture(&texture)
}

fn px(image: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * SIZE + x) * 4) as usize;
    [image[i], image[i + 1], image[i + 2]]
}

#[test]
fn a_vignette_darkens_the_corners_and_leaves_the_center() {
    let Some(g) = gpu() else { return };
    let plain = render(&g, [0.5; 3], 4.0, &light(1.0), &[]);
    let vignette = render(&g, [0.5; 3], 4.0, &light(1.0), &[ScreenEffect::Vignette { strength: 1.0, radius: 0.3, softness: 0.1 }]);
    assert!(px(&plain, 1, 1)[0].abs_diff(px(&plain, 32, 32)[0]) <= 2, "the plain picture is even");
    assert!(px(&vignette, 32, 32)[0].abs_diff(px(&plain, 32, 32)[0]) <= 2, "the center keeps its brightness");
    assert!(px(&vignette, 1, 1)[0] < 20, "a corner is dark: {:?}", px(&vignette, 1, 1));
}

#[test]
fn color_grade_with_no_saturation_makes_the_picture_gray() {
    let Some(g) = gpu() else { return };
    let graded = render(&g, [0.8, 0.1, 0.1], 4.0, &light(1.0), &[ScreenEffect::ColorGrade { saturation: 0.0, contrast: 0.0, warmth: 0.0, tint: 0.0 }]);
    let [r, gr, b] = px(&graded, 32, 32);
    assert!(r.abs_diff(gr) <= 2 && gr.abs_diff(b) <= 2, "gray: {:?}", [r, gr, b]);
}

#[test]
fn light_shafts_brighten_open_sky_around_the_sun() {
    let Some(g) = gpu() else { return };
    // The sun is straight behind the camera's view: its light travels toward the camera.
    let mut sunny = light(1.0);
    sunny.direction = [0.0, 0.0, 1.0];
    sunny.intensity = 2.0;
    let effects = [ScreenEffect::LightShafts { strength: 2.0, length: 0.9 }];
    // Shafts need a perspective camera: an orthographic view has no point where the sun sits.
    let camera = Camera3D::perspective([0.0, 0.0, 5.0], [0.0; 3], 50.0);
    let without = render_from(&g, &camera, [0.2; 3], 1.0, &sunny, &[]);
    let with = render_from(&g, &camera, [0.2; 3], 1.0, &sunny, &effects);
    // Open sky beside the small square, near the sun at the center.
    let (a, b) = (px(&without, 32, 12)[0], px(&with, 32, 12)[0]);
    assert!(b > a + 40, "shafts add light near the sun: {a} without, {b} with");
}

#[test]
fn auto_exposure_brightens_a_dark_picture() {
    let Some(g) = gpu() else { return };
    let effects = [ScreenEffect::AutoExposure { darkest: 0.5, brightest: 8.0, speed: 2.0, target: 0.18 }];
    let fixed = render(&g, [0.5; 3], 4.0, &light(0.05), &[]);
    let auto = render(&g, [0.5; 3], 4.0, &light(0.05), &effects);
    let (a, b) = (px(&fixed, 32, 32)[0], px(&auto, 32, 32)[0]);
    assert!(b > a + 30, "the dark picture is exposed up: {a} fixed, {b} auto");
}

#[test]
fn effect_numbers_out_of_range_are_refused() {
    let mut settings = PostProcessSettings { enabled: true, ..Default::default() };
    settings.effects[0] = Some(ScreenEffect::Vignette { strength: 2.0, radius: 0.5, softness: 0.2 });
    assert!(settings.validate().is_err());
    settings.effects[0] = Some(ScreenEffect::AutoExposure { darkest: 4.0, brightest: 1.0, speed: 1.0, target: 0.2 });
    assert!(settings.validate().is_err(), "darkest above brightest");
    settings.effects[0] = Some(ScreenEffect::FilmGrain { strength: f32::NAN });
    assert!(settings.validate().is_err());
    settings.effects[0] = Some(ScreenEffect::FilmGrain { strength: 0.5 });
    assert!(settings.validate().is_ok());
}

#[test]
fn height_fog_turns_far_geometry_toward_the_sky_color() {
    let Some(g) = gpu() else { return };
    let mut day = light(1.0);
    day.direction = [0.3, -0.8, -0.5];
    let camera = Camera3D::perspective([0.0, 0.0, 5.0], [0.0; 3], 50.0);
    let fog = [ScreenEffect::HeightFog { density: 0.05, falloff: 0.0, height: 0.0, haze: 0.0, sky_brightness: 1.5, sun_glow: 1.0 }];
    // A red wall 4 meters away fills the view.
    let clear = render_from(&g, &camera, [0.8, 0.05, 0.05], 20.0, &day, &[]);
    let foggy = render_from(&g, &camera, [0.8, 0.05, 0.05], 20.0, &day, &fog);
    let (a, b) = (px(&clear, 32, 32), px(&foggy, 32, 32));
    assert!(a[0] > a[2] + 100, "without fog the wall is red: {a:?}");
    assert!(b[2] > a[2] + 40 && b[0] < a[0], "fog brings in the sky color: {a:?} clear, {b:?} foggy");
}

fn ortho() -> Camera3D {
    Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 1.0)
}

/// A wide square in the back and a small one 0.4 meters in front of it, both facing the camera.
fn step_scene() -> [StaticInstance; 2] {
    [
        StaticInstance { scale: [4.0; 3], ..Default::default() },
        StaticInstance { translation: [0.0, 0.0, 0.4], scale: [0.5; 3], ..Default::default() },
    ]
}

#[test]
fn ambient_occlusion_darkens_the_floor_beside_a_raised_square_only() {
    let Some(g) = gpu() else { return };
    let effects = [ScreenEffect::Ssao { strength: 2.0, radius: 1.5 }];
    let plain = render_instances(&g, &ortho(), [0.6; 3], &step_scene(), &light(1.0), &[]);
    let occluded = render_instances(&g, &ortho(), [0.6; 3], &step_scene(), &light(1.0), &effects);
    // The raised square ends near x = 40; the floor just past it sits in the crease.
    let a = px(&plain, 44, 32)[0];
    let b = (40..48).map(|x| px(&occluded, x, 32)[0]).min().unwrap();
    assert!(b + 20 < a, "the floor next to the raised square is darker: {a} plain, {b} occluded");
    let (a, b) = (px(&plain, 4, 4)[0], px(&occluded, 4, 4)[0]);
    assert!(a.abs_diff(b) <= 3, "open floor far from it keeps its light: {a} plain, {b} occluded");
    let (a, b) = (px(&plain, 32, 32)[0], px(&occluded, 32, 32)[0]);
    assert!(a.abs_diff(b) <= 3, "the top of the raised square is open: {a} plain, {b} occluded");
}

#[test]
fn toon_steps_put_nearby_brightness_in_one_band() {
    let Some(g) = gpu() else { return };
    let effects = [ScreenEffect::Toon { bands: 2.0, softness: 0.0 }];
    let (dark, light_) = (render(&g, [0.3; 3], 4.0, &light(1.0), &[]), render(&g, [0.4; 3], 4.0, &light(1.0), &[]));
    let (dark_toon, light_toon) = (render(&g, [0.3; 3], 4.0, &light(1.0), &effects), render(&g, [0.4; 3], 4.0, &light(1.0), &effects));
    assert!(px(&light_, 32, 32)[0] > px(&dark, 32, 32)[0] + 10, "the plain pictures differ");
    let (a, b) = (px(&dark_toon, 32, 32), px(&light_toon, 32, 32));
    assert!(a[0].abs_diff(b[0]) <= 2, "both land on the same step: {a:?} and {b:?}");
    assert!(a[0].abs_diff(a[2]) <= 2, "gray stays gray: {a:?}");
    let shade = [ScreenEffect::Toon { bands: 4.0, softness: 0.0 }];
    let (plain, stepped) = (render(&g, [0.03; 3], 4.0, &light(1.0), &[]), render(&g, [0.03; 3], 4.0, &light(1.0), &shade));
    assert!(px(&stepped, 32, 32)[0] >= px(&plain, 32, 32)[0], "the darkest step is not black: {:?}", px(&stepped, 32, 32));
}

#[test]
fn an_outline_follows_the_depth_step_and_leaves_flat_areas() {
    let Some(g) = gpu() else { return };
    let effects = [ScreenEffect::Outline { thickness: 1.0, sensitivity: 0.02, darkness: 1.0 }];
    let plain = render_instances(&g, &ortho(), [0.6; 3], &step_scene(), &light(1.0), &[]);
    let lined = render_instances(&g, &ortho(), [0.6; 3], &step_scene(), &light(1.0), &effects);
    let darkest = (37..44).map(|x| px(&lined, x, 32)[0]).min().unwrap();
    assert!(darkest < 30, "a dark line at the edge of the raised square: {darkest}");
    for (x, y) in [(32, 32), (4, 4)] {
        assert!(px(&plain, x, y)[0].abs_diff(px(&lined, x, y)[0]) <= 2, "flat area at {x},{y} is unchanged");
    }
}

#[test]
fn fxaa_softens_a_stair_stepped_diagonal_edge() {
    let Some(g) = gpu() else { return };
    let turn = std::f32::consts::FRAC_PI_8;
    let tilted = [StaticInstance { rotation: [0.0, 0.0, turn.sin(), turn.cos()], scale: [1.2; 3], ..Default::default() }];
    let plain = render_instances(&g, &ortho(), [1.0; 3], &tilted, &light(1.0), &[]);
    let smooth = render_instances(&g, &ortho(), [1.0; 3], &tilted, &light(1.0), &[ScreenEffect::Fxaa { strength: 1.0 }]);
    // Red is 149 on the clear background and 255 on the square; anti-aliased edge pixels fall between.
    let between = |image: &[u8]| (0..SIZE * SIZE).filter(|i| (156..250).contains(&image[(*i * 4) as usize])).count();
    assert!(between(&smooth) > between(&plain) + 20, "edge pixels take in-between values: {} plain, {} fxaa", between(&plain), between(&smooth));
    assert!(px(&plain, 32, 32)[0].abs_diff(px(&smooth, 32, 32)[0]) <= 2, "the flat inside is unchanged");
}

#[test]
fn upscaling_from_half_size_keeps_flat_colors_and_the_target_size() {
    let Some(g) = gpu() else { return };
    let effects = [ScreenEffect::Upscale { scale: 0.5, sharpness: 0.5 }];
    let plain = render(&g, [0.6, 0.3, 0.1], 1.0, &light(1.0), &[]);
    let upscaled = render(&g, [0.6, 0.3, 0.1], 1.0, &light(1.0), &effects);
    assert_eq!(upscaled.len(), (SIZE * SIZE * 4) as usize);
    for (x, y) in [(32, 32), (2, 2), (61, 61)] {
        let (a, b) = (px(&plain, x, y), px(&upscaled, x, y));
        assert!((0..3).all(|c| a[c].abs_diff(b[c]) <= 3), "flat color at {x},{y}: {a:?} plain, {b:?} upscaled");
    }
    // The square's edge is still where it was, within the half-size grid.
    let inside = |image: &[u8]| (0..SIZE).filter(|x| px(image, *x, 32)[0] > 100).count() as i64;
    assert!((inside(&plain) - inside(&upscaled)).abs() <= 2, "{} plain, {} upscaled", inside(&plain), inside(&upscaled));
}

#[test]
fn the_scene_draws_smaller_only_with_an_upscale_effect() {
    use orr_render::post_effects::render_size;
    assert_eq!(render_size(&[None], (1920, 1080)), (1920, 1080));
    assert_eq!(render_size(&[Some(ScreenEffect::Upscale { scale: 0.75, sharpness: 0.2 })], (1920, 1080)), (1440, 810));
    assert_eq!(render_size(&[Some(ScreenEffect::Upscale { scale: 0.5, sharpness: 0.2 })], (1, 1)), (1, 1));
}


/// Draws `frames` frames with one renderer, so temporal effects keep their history;
/// `camera(i)` is the camera of frame `i`. Returns the last picture.
#[allow(clippy::too_many_arguments)] // The same scene arguments as render_instances, plus frames.
fn render_frames(g: &Wgpu, camera: impl Fn(usize) -> Camera3D, color: [f32; 3], instances: &[StaticInstance], light: &Lighting, effects: &[ScreenEffect], frames: usize) -> Vec<u8> {
    let mut scene = ImportedSceneRenderer::new(g.clone(), FORMAT).unwrap();
    scene.post_process = PostProcessSettings { enabled: true, bloom: false, ..Default::default() };
    for (slot, effect) in scene.post_process.effects.iter_mut().zip(effects) {
        *slot = Some(*effect);
    }
    let mut renderer = ModelRenderer::new(g.clone(), TextureFormat::Rgba16Float, square(color)).unwrap();
    let texture = g.create_texture(&TextureDesc {
        label: "temporal effect test",
        width: SIZE,
        height: SIZE,
        format: FORMAT,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let view = g.create_texture_view(&texture, None);
    for frame in 0..frames {
        scene
            .draw(
                ImportedSceneTarget { view: &view, size: (SIZE, SIZE), format: FORMAT, sample_count: 1 },
                &camera(frame),
                light,
                &PointLightSettings::default(),
                &mut [ImportedBatch::StaticInstances { renderer: &mut renderer, instances }],
            )
            .unwrap();
    }
    g.read_texture(&texture)
}

fn tilted() -> [StaticInstance; 1] {
    let turn = std::f32::consts::FRAC_PI_8;
    [StaticInstance { rotation: [0.0, 0.0, turn.sin(), turn.cos()], scale: [1.2; 3], ..Default::default() }]
}

/// Pixels whose red lies strictly between the clear background (149) and the white square (255).
fn in_between(image: &[u8]) -> usize {
    (0..SIZE * SIZE).filter(|i| (156..250).contains(&image[(*i * 4) as usize])).count()
}

#[test]
fn taa_smooths_a_stair_stepped_edge_over_frames() {
    let Some(g) = gpu() else { return };
    let plain = render_frames(&g, |_| ortho(), [1.0; 3], &tilted(), &light(1.0), &[], 16);
    let taa = render_frames(&g, |_| ortho(), [1.0; 3], &tilted(), &light(1.0), &[ScreenEffect::Taa { blend: 0.1 }], 16);
    assert!(in_between(&taa) > in_between(&plain) + 20, "{} plain, {} taa", in_between(&plain), in_between(&taa));
    for (x, y) in [(32, 32), (1, 1)] {
        assert!(px(&plain, x, y)[0].abs_diff(px(&taa, x, y)[0]) <= 2, "flat area at {x},{y} keeps its color");
    }
}

#[test]
fn taa_leaves_no_trail_after_the_camera_jumps() {
    let Some(g) = gpu() else { return };
    let moved = || Camera3D::orthographic([0.9, 0.0, 5.0], [0.9, 0.0, 0.0], 1.0);
    let effects = [ScreenEffect::Taa { blend: 0.1 }];
    let jumped = render_frames(&g, |frame| if frame < 12 { ortho() } else { moved() }, [1.0; 3], &tilted(), &light(1.0), &effects, 13);
    let fresh = render_frames(&g, |_| moved(), [1.0; 3], &tilted(), &light(1.0), &[], 1);
    let before = render_frames(&g, |_| ortho(), [1.0; 3], &tilted(), &light(1.0), &[], 1);
    // Pixels the square covered before the jump and open background after it, away from
    // the square's new edge, where anti-aliasing rightly mixes the two.
    let open = |x: u32| (x.saturating_sub(2)..=(x + 2).min(SIZE - 1)).all(|n| px(&fresh, n, 32)[0] <= 152);
    let uncovered: Vec<u32> = (0..SIZE).filter(|&x| px(&before, x, 32)[0] >= 250 && open(x)).collect();
    assert!(uncovered.len() >= 3, "the jump uncovers background: {uncovered:?}");
    for x in uncovered {
        assert!(px(&jumped, x, 32)[0].abs_diff(px(&fresh, x, 32)[0]) <= 12, "no trail at {x}: {:?} after the jump, {:?} fresh", px(&jumped, x, 32), px(&fresh, x, 32));
    }
}

#[test]
fn temporal_upscaling_keeps_flat_colors_and_smooths_edges() {
    let Some(g) = gpu() else { return };
    let effects = [ScreenEffect::Upscale { scale: 0.5, sharpness: 0.3 }, ScreenEffect::Taa { blend: 0.1 }];
    let plain = render_frames(&g, |_| ortho(), [1.0; 3], &tilted(), &light(1.0), &[], 1);
    let upscaled = render_frames(&g, |_| ortho(), [1.0; 3], &tilted(), &light(1.0), &effects, 24);
    for (x, y) in [(32, 32), (1, 1), (62, 62)] {
        let (a, b) = (px(&plain, x, y), px(&upscaled, x, y));
        assert!((0..3).all(|c| a[c].abs_diff(b[c]) <= 3), "flat color at {x},{y}: {a:?} plain, {b:?} upscaled");
    }
    assert!(in_between(&upscaled) > in_between(&plain) + 20, "{} plain, {} upscaled", in_between(&plain), in_between(&upscaled));
}

/// Draws one renderer per (color, instances) group for `frames` frames, the camera of frame `i`
/// from `camera(i)`, and returns the last picture.
fn render_groups(g: &Wgpu, camera: impl Fn(usize) -> Camera3D, groups: &[([f32; 3], Vec<StaticInstance>)], light: &Lighting, effects: &[ScreenEffect], frames: usize) -> Vec<u8> {
    render_lit_groups(g, camera, groups, light, &[], effects, frames)
}

/// [`render_groups`] with scene lights.
#[allow(clippy::too_many_arguments)] // render_groups' arguments plus the lights.
fn render_lit_groups(
    g: &Wgpu,
    camera: impl Fn(usize) -> Camera3D,
    groups: &[([f32; 3], Vec<StaticInstance>)],
    light: &Lighting,
    lights: &[orr_render::SceneLight],
    effects: &[ScreenEffect],
    frames: usize,
) -> Vec<u8> {
    let mut scene = ImportedSceneRenderer::new(g.clone(), FORMAT).unwrap();
    for scene_light in lights {
        scene.scene_lights.push(*scene_light).unwrap();
    }
    scene.post_process = PostProcessSettings { enabled: true, bloom: false, ..Default::default() };
    for (slot, effect) in scene.post_process.effects.iter_mut().zip(effects) {
        *slot = Some(*effect);
    }
    let mut renderers: Vec<ModelRenderer<Wgpu>> = groups.iter().map(|(color, _)| ModelRenderer::new(g.clone(), TextureFormat::Rgba16Float, square(*color)).unwrap()).collect();
    let texture = g.create_texture(&TextureDesc {
        label: "scene stage test",
        width: SIZE,
        height: SIZE,
        format: FORMAT,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let view = g.create_texture_view(&texture, None);
    for frame in 0..frames {
        let mut batches: Vec<ImportedBatch<'_, Wgpu>> =
            renderers.iter_mut().zip(groups).map(|(renderer, (_, instances))| ImportedBatch::StaticInstances { renderer, instances }).collect();
        scene
            .draw(ImportedSceneTarget { view: &view, size: (SIZE, SIZE), format: FORMAT, sample_count: 1 }, &camera(frame), light, &PointLightSettings::default(), &mut batches)
            .unwrap();
    }
    g.read_texture(&texture)
}

/// A dark floor facing up and a red wall standing on it, 1 meter back.
fn floor_and_wall() -> Vec<([f32; 3], Vec<StaticInstance>)> {
    let half = std::f32::consts::FRAC_PI_4;
    let floor = StaticInstance { rotation: [-half.sin(), 0.0, 0.0, half.cos()], scale: [8.0; 3], ..Default::default() };
    let wall = StaticInstance { translation: [0.0, 0.5, -1.0], scale: [1.5, 1.0, 1.0], ..Default::default() };
    vec![([0.05; 3], vec![floor]), ([0.9, 0.05, 0.05], vec![wall])]
}

#[test]
fn reflections_put_the_wall_on_the_floor_in_front_of_it() {
    let Some(g) = gpu() else { return };
    let camera = |_| Camera3D::perspective([0.0, 0.6, 3.0], [0.0, 0.2, -1.0], 50.0);
    let plain = render_groups(&g, camera, &floor_and_wall(), &light(1.0), &[], 1);
    let ssr = [ScreenEffect::Ssr { strength: 1.0, distance: 6.0, thickness: 0.5, floors: 0.9 }];
    let mirrored = render_groups(&g, camera, &floor_and_wall(), &light(1.0), &ssr, 1);
    // Redness of the lower half of the picture, where only floor is.
    let red = |image: &[u8]| (SIZE * 3 / 4..SIZE).flat_map(|y| (0..SIZE).map(move |x| (x, y))).map(|(x, y)| i64::from(px(image, x, y)[0]) - i64::from(px(image, x, y)[1])).sum::<i64>();
    assert!(red(&mirrored) > red(&plain) + 2000, "the floor shows the red wall: {} plain, {} with reflections", red(&plain), red(&mirrored));
    // The wall itself faces the camera, not up: it does not reflect.
    let (a, b) = (px(&plain, 32, 20), px(&mirrored, 32, 20));
    assert!((0..3).all(|c| a[c].abs_diff(b[c]) <= 3), "the wall is unchanged: {a:?} {b:?}");
}

#[test]
fn depth_of_field_blurs_what_is_out_of_focus_only() {
    let Some(g) = gpu() else { return };
    let camera = |_| Camera3D::perspective([0.0, 0.0, 3.0], [0.0; 3], 50.0);
    let square = vec![([1.0; 3], vec![StaticInstance::default()])];
    let plain = render_groups(&g, camera, &square, &light(1.0), &[], 1);
    let focused = render_groups(&g, camera, &square, &light(1.0), &[ScreenEffect::DepthOfField { focus: 3.0, range: 0.5, blur: 8.0 }], 1);
    let blurred = render_groups(&g, camera, &square, &light(1.0), &[ScreenEffect::DepthOfField { focus: 0.5, range: 0.2, blur: 8.0 }], 1);
    assert!(in_between(&focused) <= in_between(&plain) + 4, "in focus stays sharp: {} plain, {} focused", in_between(&plain), in_between(&focused));
    assert!(in_between(&blurred) > in_between(&plain) + 40, "out of focus blurs: {} plain, {} blurred", in_between(&plain), in_between(&blurred));
}

#[test]
fn motion_blur_streaks_only_when_the_camera_moved() {
    let Some(g) = gpu() else { return };
    let square = vec![([1.0; 3], vec![StaticInstance { scale: [1.2; 3], ..Default::default() }])];
    let effects = [ScreenEffect::MotionBlur { strength: 1.0, limit: 32.0 }];
    let still = render_groups(&g, |_| ortho(), &square, &light(1.0), &effects, 2);
    let panning = |frame: usize| {
        let x = frame as f32 * 0.15;
        Camera3D::orthographic([x, 0.0, 5.0], [x, 0.0, 0.0], 1.0)
    };
    let sharp = render_groups(&g, panning, &square, &light(1.0), &[], 2);
    let streaked = render_groups(&g, panning, &square, &light(1.0), &effects, 2);
    assert!(in_between(&still) <= 4, "a still camera has no streaks: {}", in_between(&still));
    assert!(in_between(&streaked) > in_between(&sharp) + 40, "a moving camera streaks the edges: {} sharp, {} blurred", in_between(&sharp), in_between(&streaked));
}

#[test]
fn chromatic_aberration_splits_colors_toward_the_edges_only() {
    let Some(g) = gpu() else { return };
    let square = vec![([1.0; 3], vec![StaticInstance { scale: [1.6; 3], ..Default::default() }])];
    let plain = render_groups(&g, |_| ortho(), &square, &light(1.0), &[], 1);
    let split = render_groups(&g, |_| ortho(), &square, &light(1.0), &[ScreenEffect::ChromaticAberration { strength: 1.0 }], 1);
    assert_eq!(px(&plain, 32, 32), px(&split, 32, 32), "the center is unchanged");
    let fringe = (0..SIZE).flat_map(|y| (0..SIZE).map(move |x| (x, y))).filter(|&(x, y)| {
        let (a, b) = (px(&plain, x, y), px(&split, x, y));
        a[0].abs_diff(b[0]) > 20 || a[2].abs_diff(b[2]) > 20
    });
    assert!(fringe.count() > 20, "the square's edges near the border get color fringes");
}

/// A gray floor facing up and a small square floating 1.2 meters over it, off to the side.
fn floor_and_caster() -> Vec<([f32; 3], Vec<StaticInstance>)> {
    let half = std::f32::consts::FRAC_PI_4;
    let up = [-half.sin(), 0.0, 0.0, half.cos()];
    let floor = StaticInstance { rotation: up, scale: [8.0; 3], ..Default::default() };
    let caster = StaticInstance { translation: [-0.8, 1.2, 0.0], rotation: up, scale: [0.6; 3] };
    vec![([0.7; 3], vec![floor]), ([0.7; 3], vec![caster])]
}

fn spot_from_above(shadows: bool) -> orr_render::SceneLight {
    orr_render::SceneLight {
        kind: orr_render::SceneLightKind::Spot,
        position: [0.0, 4.0, 0.0],
        direction: [0.0, -1.0, 0.0],
        intensity: 3.0,
        range: 10.0,
        inner_angle: 0.5,
        outer_angle: 0.7,
        shadows,
        ..Default::default()
    }
}

fn image_red(image: &[u8], pixel: u32) -> u8 {
    image[(pixel * 4) as usize]
}

#[test]
fn a_spot_light_casts_a_shadow_with_and_without_a_sun_shadow() {
    let Some(g) = gpu() else { return };
    let camera = |_| Camera3D::perspective([0.0, 6.0, 2.5], [0.0; 3], 50.0);
    let mut dark = light(0.05);
    dark.tonemap = false;
    let mut sunny = dark;
    sunny.shadows = true;
    sunny.intensity = 0.2;
    sunny.direction = [0.3, -1.0, 0.2];
    for scene_light in [dark, sunny] {
        let lit = render_lit_groups(&g, camera, &floor_and_caster(), &scene_light, &[spot_from_above(false)], &[], 1);
        let shadowed = render_lit_groups(&g, camera, &floor_and_caster(), &scene_light, &[spot_from_above(true)], &[], 1);
        // The shadow falls on the floor beyond the caster, away from the light.
        let darker = (0..SIZE * SIZE).filter(|i| image_red(&lit, *i) > image_red(&shadowed, *i) + 60).count();
        assert!(darker > 40, "the caster's spot shadow darkens the floor: {darker} pixels (sun shadow {})", scene_light.shadows);
        // A corner outside the spot's cone is the same either way.
        assert!(px(&lit, 1, 1)[0].abs_diff(px(&shadowed, 1, 1)[0]) <= 2);
    }
}

#[test]
fn spot_shadows_alone_leave_the_sun_unshadowed_on_shapes() {
    let Some(g) = gpu() else { return };
    let draw = |with_spot: bool| {
        let mut scene = ImportedSceneRenderer::new(g.clone(), FORMAT).unwrap();
        if with_spot {
            // Far to the side: it lights nothing in view, but its shadow turns on the shared path.
            scene.scene_lights.push(orr_render::SceneLight { position: [40.0, 4.0, 0.0], range: 3.0, ..spot_from_above(true) }).unwrap();
        }
        let mut list = orr_render::RenderList3D::new();
        // In front of the origin, where a stray read of the zero-filled sun map would shadow it.
        list.cuboid([0.0, 0.0, 0.5], orr_render::IDENTITY_ROT, [3.0, 3.0, 0.02], &orr_render::Material::new([0.8; 3]).rough(1.0));
        let mut renderer = orr_render::Renderer3D::with_settings(g.clone(), FORMAT, orr_render::Settings3D::LOW);
        let mut sun = light(0.2);
        sun.intensity = 1.0;
        sun.direction = [0.0, 0.0, -1.0];
        let texture = g.create_texture(&TextureDesc {
            label: "spot-only shadow test",
            width: SIZE,
            height: SIZE,
            format: FORMAT,
            usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
            sample_count: 1,
            view_formats: &[],
        });
        let view = g.create_texture_view(&texture, None);
        scene
            .draw(
                ImportedSceneTarget { view: &view, size: (SIZE, SIZE), format: FORMAT, sample_count: 1 },
                &ortho(),
                &sun,
                &PointLightSettings::default(),
                &mut [ImportedBatch::Procedural { renderer: &mut renderer, list: &list }],
            )
            .unwrap();
        g.read_texture(&texture)
    };
    let (plain, spotted) = (draw(false), draw(true));
    for (x, y) in [(32, 32), (8, 50)] {
        assert_eq!(px(&plain, x, y), px(&spotted, x, y), "the sunlit shape at {x},{y} gets no shadow from an unused map");
    }
}

/// A dark wall 11 meters ahead of a camera at eye height and, with `roof`, a wide plate 2 meters overhead.
fn under_a_roof(roof: bool) -> Vec<([f32; 3], Vec<StaticInstance>)> {
    let half = std::f32::consts::FRAC_PI_4;
    let wall = StaticInstance { translation: [0.0, 1.0, -6.0], scale: [16.0; 3], ..Default::default() };
    let mut groups = vec![([0.05; 3], vec![wall])];
    if roof {
        let plate = StaticInstance { translation: [0.0, 3.0, 0.0], rotation: [half.sin(), 0.0, 0.0, half.cos()], scale: [40.0; 3] };
        groups.push(([0.05; 3], vec![plate]));
    }
    groups
}

fn sun_from_above() -> Lighting {
    let mut sun = light(0.05);
    sun.intensity = 2.0;
    sun.direction = [0.0, -1.0, 0.0];
    sun.shadows = true;
    sun.shadow_center = [0.0, 1.0, 0.0];
    sun.shadow_radius = 20.0;
    sun
}

#[test]
fn volumetric_fog_lights_the_air_except_in_shadow() {
    let Some(g) = gpu() else { return };
    let camera = |_| Camera3D::perspective([0.0, 1.0, 5.0], [0.0, 1.0, -5.0], 50.0);
    let fog = [ScreenEffect::VolumetricFog { density: 0.1, anisotropy: 0.0, distance: 20.0, intensity: 4.0 }];
    // Mean red of the lower half, under the roof line.
    let lower = |image: &[u8]| (SIZE / 2..SIZE).flat_map(|y| (0..SIZE).map(move |x| (x, y))).map(|(x, y)| f32::from(px(image, x, y)[0])).sum::<f32>() / (SIZE * SIZE / 2) as f32;
    let clear = render_groups(&g, camera, &under_a_roof(false), &sun_from_above(), &[], 1);
    let open = render_groups(&g, camera, &under_a_roof(false), &sun_from_above(), &fog, 1);
    let roofed = render_groups(&g, camera, &under_a_roof(true), &sun_from_above(), &fog, 1);
    let (a, b, c) = (lower(&clear), lower(&open), lower(&roofed));
    assert!(b > a + 20.0, "sunlit fog brightens the view: {a} clear, {b} fog");
    assert!(c + 15.0 < b, "fog in the roof's shadow gets far less light: {b} open, {c} roofed");
}
