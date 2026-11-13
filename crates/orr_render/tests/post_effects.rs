//! Post effects of the imported scene's HDR chain, checked on the picture:
//! vignette, color grade, light shafts and auto exposure.
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
    let mut scene = ImportedSceneRenderer::new(g.clone(), FORMAT).unwrap();
    scene.post_process = PostProcessSettings { enabled: true, bloom: false, ..Default::default() };
    for (slot, effect) in scene.post_process.effects.iter_mut().zip(effects) {
        *slot = Some(*effect);
    }
    // With HDR on, batches draw into the scene's linear RGBA16F target.
    let mut renderer = ModelRenderer::new(g.clone(), TextureFormat::Rgba16Float, square(color)).unwrap();
    let instances = [StaticInstance { scale: [scale; 3], ..Default::default() }];
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
            &mut [ImportedBatch::StaticInstances { renderer: &mut renderer, instances: &instances }],
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
