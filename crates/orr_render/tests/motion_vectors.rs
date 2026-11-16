//! Per-object motion vectors: temporal anti-aliasing and motion blur follow
//! objects that move under a still camera, and a camera move alone keeps the
//! picture it gave before. The scenes are fine black and white bars whose edges
//! sit on pixel boundaries, so a wrong history shows as gray in the picture.
#![cfg(feature = "imported-scene")]
#![allow(clippy::float_arithmetic)]

use orr_model::{StaticModel, Vertex, IDENTITY};
use orr_render::orr_rhi::{Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions};
use orr_render::{
    Camera3D, ImportedBatch, ImportedSceneRenderer, ImportedSceneTarget, Lighting, Material, ModelRenderer, PointLightSettings, PostProcessSettings, RenderList3D, Renderer3D, Settings3D,
    ScreenEffect, StaticInstance, IDENTITY_ROT,
};

const SIZE: u32 = 64;
const FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;
/// Meters per pixel of the orthographic camera below.
const PIXEL: f32 = 2.0 / SIZE as f32;

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

fn light() -> Lighting {
    Lighting { sky: [1.0; 3], ground: [1.0; 3], ambient: 1.0, intensity: 0.0, shadows: false, tonemap: false, exposure: 1.0, ..Default::default() }
}

fn camera(x: f32) -> Camera3D {
    Camera3D::orthographic([x, 0.0, 5.0], [x, 0.0, 0.0], 1.0)
}

/// White and black bars, each 4 pixels wide, side by side, moved by `shift` meters.
/// `skip_first` leaves out the first white bar.
fn bars(shift: f32, skip_first: bool) -> [Vec<StaticInstance>; 2] {
    let bar = |center: f32| StaticInstance { translation: [center, 0.0, 0.0], scale: [4.0 * PIXEL, 3.0, 1.0], ..Default::default() };
    let at = |offset: f32| (-6..22).map(move |k| bar(-1.0 + 8.0 * PIXEL * k as f32 + offset + shift)).collect::<Vec<_>>();
    let mut white = at(2.0 * PIXEL);
    if skip_first {
        white.remove(0);
    }
    [white, at(6.0 * PIXEL)]
}

/// Draws the scene of every frame and returns the last picture. `scene(frame)` gives the camera and the instances of the white and black bars.
fn render(g: &Wgpu, effects: &[ScreenEffect], frames: usize, scene: impl Fn(usize) -> (Camera3D, [Vec<StaticInstance>; 2])) -> Vec<u8> {
    let mut renderer = ImportedSceneRenderer::new(g.clone(), FORMAT).unwrap();
    renderer.post_process = PostProcessSettings { enabled: true, bloom: false, ..Default::default() };
    for (slot, effect) in renderer.post_process.effects.iter_mut().zip(effects) {
        *slot = Some(*effect);
    }
    let mut models = [ModelRenderer::new(g.clone(), TextureFormat::Rgba16Float, square([1.0; 3])).unwrap(), ModelRenderer::new(g.clone(), TextureFormat::Rgba16Float, square([0.0; 3])).unwrap()];
    let texture = g.create_texture(&TextureDesc {
        label: "motion vector test",
        width: SIZE,
        height: SIZE,
        format: FORMAT,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let view = g.create_texture_view(&texture, None);
    for frame in 0..frames {
        let (camera, instances) = scene(frame);
        let [white, black] = &mut models;
        renderer
            .draw(
                ImportedSceneTarget { view: &view, size: (SIZE, SIZE), format: FORMAT, sample_count: 1 },
                &camera,
                &light(),
                &PointLightSettings::default(),
                &mut [ImportedBatch::StaticInstances { renderer: white, instances: &instances[0] }, ImportedBatch::StaticInstances { renderer: black, instances: &instances[1] }],
            )
            .unwrap();
    }
    g.read_texture(&texture)
}


/// Mean absolute difference of the red channel, 0 to 255.
fn mean_difference(a: &[u8], b: &[u8]) -> f32 {
    let total: u32 = (0..SIZE * SIZE).map(|i| u32::from(a[(i * 4) as usize].abs_diff(b[(i * 4) as usize]))).sum();
    total as f32 / (SIZE * SIZE) as f32
}

/// Pixels that are neither the white nor the black of the bars.
fn gray(image: &[u8]) -> usize {
    (0..SIZE * SIZE).filter(|i| (8..248).contains(&image[(*i * 4) as usize])).count()
}

const FRAMES: usize = 14;

/// The temporal result of a scene whose frames differ, against the same scene left still at its last frame:
/// what accumulating along the motion keeps and a wrong history loses.
fn motion_error(g: &Wgpu, scene: impl Fn(usize) -> (Camera3D, [Vec<StaticInstance>; 2])) -> f32 {
    let effects = [ScreenEffect::Taa { blend: 0.2 }];
    let moved = render(g, &effects, FRAMES, &scene);
    let settled = render(g, &effects, FRAMES, |_| scene(FRAMES - 1));
    let difference = mean_difference(&settled, &moved);
    eprintln!("motion error {difference}, gray {} moved, {} settled", gray(&moved), gray(&settled));
    difference
}

#[test]
fn taa_follows_bars_that_move_under_a_still_camera() {
    let Some(g) = gpu() else { return };
    let error = motion_error(&g, |frame| (camera(0.0), bars(2.0 * PIXEL * frame as f32, false)));
    assert!(error < 3.0, "moving bars match the settled picture: {error}");
}

#[test]
fn taa_keeps_still_bars_sharp_under_a_moving_camera() {
    let Some(g) = gpu() else { return };
    let error = motion_error(&g, |frame| (camera(2.0 * PIXEL * frame as f32), bars(0.0, false)));
    assert!(error < 3.0, "still bars match the settled picture: {error}");
}

#[test]
fn motion_blur_streaks_moving_bars_under_a_still_camera_only() {
    let Some(g) = gpu() else { return };
    let effects = [ScreenEffect::MotionBlur { strength: 1.0, limit: 32.0 }];
    let still = render(&g, &effects, 3, |_| (camera(0.0), bars(0.0, false)));
    let moving = render(&g, &effects, 3, |frame| (camera(0.0), bars(2.0 * PIXEL * frame as f32, false)));
    assert!(gray(&still) <= 4, "still bars have no streaks: {}", gray(&still));
    assert!(gray(&moving) > 100, "moving bars streak along their motion: {}", gray(&moving));
}

#[test]
fn an_instance_that_appears_has_no_motion() {
    let Some(g) = gpu() else { return };
    let effects = [ScreenEffect::MotionBlur { strength: 1.0, limit: 32.0 }];
    // The first white bar is new in the last frame and shifts every other bar's place in the list.
    let appearing = render(&g, &effects, 3, |frame| (camera(0.0), bars(0.0, frame < 2)));
    assert!(gray(&appearing) <= 4, "nothing moved, so nothing streaks: {}", gray(&appearing));
}

/// Draws a box at `x(frame)` with the procedural renderer for `frames` frames and returns the last picture.
fn render_box(g: &Wgpu, effects: &[ScreenEffect], frames: usize, x: impl Fn(usize) -> f32) -> Vec<u8> {
    let mut renderer = ImportedSceneRenderer::new(g.clone(), FORMAT).unwrap();
    renderer.post_process = PostProcessSettings { enabled: true, bloom: false, ..Default::default() };
    for (slot, effect) in renderer.post_process.effects.iter_mut().zip(effects) {
        *slot = Some(*effect);
    }
    let mut meshes = Renderer3D::with_settings(g.clone(), TextureFormat::Rgba16Float, Settings3D::LOW);
    let texture = g.create_texture(&TextureDesc {
        label: "procedural motion vector test",
        width: SIZE,
        height: SIZE,
        format: FORMAT,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let view = g.create_texture_view(&texture, None);
    for frame in 0..frames {
        let mut list = RenderList3D::default();
        list.cuboid([x(frame), 0.0, 0.0], IDENTITY_ROT, [0.3, 0.3, 0.3], &Material::new([1.0; 3]));
        renderer
            .draw(
                ImportedSceneTarget { view: &view, size: (SIZE, SIZE), format: FORMAT, sample_count: 1 },
                &camera(0.0),
                &light(),
                &PointLightSettings::default(),
                &mut [ImportedBatch::Procedural { renderer: &mut meshes, list: &list }],
            )
            .unwrap();
    }
    g.read_texture(&texture)
}

#[test]
fn motion_blur_streaks_a_moving_procedural_box_only() {
    let Some(g) = gpu() else { return };
    let effects = [ScreenEffect::MotionBlur { strength: 1.0, limit: 32.0 }];
    let still = |_| 0.0;
    let moving = |frame: usize| 0.2 * frame as f32;
    let blurred_still = mean_difference(&render_box(&g, &[], 3, still), &render_box(&g, &effects, 3, still));
    let blurred_moving = mean_difference(&render_box(&g, &[], 3, moving), &render_box(&g, &effects, 3, moving));
    assert!(blurred_still < 0.1, "a still box is not blurred: {blurred_still}");
    assert!(blurred_moving > 0.5, "a moving box is blurred along its motion: {blurred_moving}");
}

/// Draws the animated strip model at `x(frame)` in the pose of its clip at `time(frame)` seconds.
#[cfg(feature = "animation")]
fn render_strip(g: &Wgpu, effects: &[ScreenEffect], frames: usize, x: impl Fn(usize) -> f32, time: impl Fn(usize) -> f32) -> Vec<u8> {
    use orr_model::animation::AnimatedModel;
    use orr_render::{SkinnedInstance, SkinnedModelRenderer};
    let imported = orr_model::animation_import::import_with_resolver(
        "fixtures/animated_strip.glb",
        include_bytes!("../../orr_model/tests/fixtures/animated_strip.glb"),
        |_| panic!("fixture embeds every dependency"),
    )
    .unwrap();
    let model = AnimatedModel::from_bytes(&imported.to_bytes().unwrap()).unwrap();
    let mut renderer = ImportedSceneRenderer::new(g.clone(), FORMAT).unwrap();
    renderer.post_process = PostProcessSettings { enabled: true, bloom: false, ..Default::default() };
    for (slot, effect) in renderer.post_process.effects.iter_mut().zip(effects) {
        *slot = Some(*effect);
    }
    let mut strip = SkinnedModelRenderer::new(g.clone(), TextureFormat::Rgba16Float, model.clone()).unwrap();
    let texture = g.create_texture(&TextureDesc {
        label: "skinned motion vector test",
        width: SIZE,
        height: SIZE,
        format: FORMAT,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let view = g.create_texture_view(&texture, None);
    for frame in 0..frames {
        let pose = model.sample_clip(0, time(frame)).unwrap();
        let mut placement = orr_model::IDENTITY;
        placement[3][0] = x(frame);
        let instances = [SkinnedInstance { pose: &pose, transform: placement }];
        renderer
            .draw(
                ImportedSceneTarget { view: &view, size: (SIZE, SIZE), format: FORMAT, sample_count: 1 },
                &Camera3D::orthographic([0.0, 1.5, 5.0], [0.0, 1.5, 0.0], 3.0),
                &light(),
                &PointLightSettings::default(),
                &mut [ImportedBatch::Skinned { renderer: &mut strip, instances: &instances }],
            )
            .unwrap();
    }
    g.read_texture(&texture)
}

#[cfg(feature = "animation")]
#[test]
fn motion_blur_streaks_a_moving_or_bending_skinned_model_only() {
    let Some(g) = gpu() else { return };
    let effects = [ScreenEffect::MotionBlur { strength: 1.0, limit: 32.0 }];
    let blur = |x: &dyn Fn(usize) -> f32, time: &dyn Fn(usize) -> f32| mean_difference(&render_strip(&g, &[], 3, x, time), &render_strip(&g, &effects, 3, x, time));
    let still = blur(&|_| 0.0, &|_| 0.5);
    let sliding = blur(&|frame| 0.3 * frame as f32, &|_| 0.5);
    let bending = blur(&|_| 0.0, &|frame| 0.5 * frame as f32);
    assert!(still < 0.05, "a still model is not blurred: {still}");
    assert!(sliding > 0.3, "a sliding model is blurred along its motion: {sliding}");
    assert!(bending > 0.3, "a bending model is blurred along its joints' motion: {bending}");
}
