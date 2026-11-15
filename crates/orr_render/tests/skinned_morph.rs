//! Morph target weights on the skinned renderer: the drawn vertices and the
//! published bounds follow each instance's weights.
#![cfg(feature = "animation")]
#![allow(clippy::float_arithmetic)]

use orr_model::animation::{AnimatedModel, MorphTarget, PrimitiveMorph};
use orr_model::IDENTITY;
use orr_render::orr_rhi::{Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions};
use orr_render::{Camera3D, Lighting, SkinnedInstance, SkinnedModelRenderer};

const SIZE: u32 = 64;
const FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;

fn gpu() -> Option<Wgpu> {
    match Wgpu::headless(WgpuOptions::default()) {
        Ok(g) => Some(g),
        Err(e) => {
            assert!(std::env::var_os("ORR_REQUIRE_GPU").is_none(), "required GPU unavailable: {e}");
            eprintln!("SKIP: {e}");
            None
        }
    }
}

/// The test strip with one target that moves every vertex 1 m along x, at default weight 0.
fn strip() -> AnimatedModel {
    let model = orr_model::animation_import::import_with_resolver(
        "animated_strip.gltf",
        include_bytes!("../../orr_model/tests/fixtures/animated_strip.gltf"),
        |_| panic!("self-contained fixture"),
    )
    .unwrap();
    let mut source = model.source().clone();
    let count = source.primitives[0].vertices.len();
    let target = MorphTarget { name: "slide".into(), positions: vec![[1.0, 0.0, 0.0]; count], normals: Vec::new() };
    source.morph.primitives.push(PrimitiveMorph { primitive: 0, targets: vec![target], weights: vec![0.0] });
    AnimatedModel::new(source).unwrap()
}

/// Draws `instances` copies of the strip at rest, each 3.5 m below the last, and reads the picture.
fn draw(g: &Wgpu, renderer: &mut SkinnedModelRenderer<Wgpu>, instances: usize) -> Result<Vec<u8>, orr_render::SkinnedRenderError> {
    let texture = g.create_texture(&TextureDesc {
        label: "morph test",
        width: SIZE,
        height: SIZE,
        format: FORMAT,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let view = g.create_texture_view(&texture, None);
    let pose = renderer.model().rest_pose().unwrap();
    let placed: Vec<_> = (0..instances)
        .map(|i| {
            let mut transform = IDENTITY;
            transform[3][1] = -3.5 * i as f32;
            SkinnedInstance { pose: &pose, transform }
        })
        .collect();
    let light = Lighting { shadows: false, tonemap: false, ambient: 1.0, intensity: 0.0, sky: [1.0; 3], ground: [1.0; 3], ..Default::default() };
    renderer.draw(&view, (SIZE, SIZE), &Camera3D::orthographic([0.5, 0.0, 7.0], [0.5, 0.0, 0.0], 4.0), &light, &placed)?;
    Ok(g.read_texture(&texture))
}

/// Mean x of the pixels that are not the black clear color.
fn center_x(image: &[u8]) -> f32 {
    let lit: Vec<u32> = (0..SIZE * SIZE).filter(|i| image[(*i * 4) as usize..(*i * 4 + 3) as usize].iter().any(|c| *c > 8)).map(|i| i % SIZE).collect();
    assert!(!lit.is_empty(), "the strip is drawn");
    lit.iter().sum::<u32>() as f32 / lit.len() as f32
}

#[test]
fn morph_weights_move_the_drawn_strip_and_its_bounds() {
    let Some(g) = gpu() else { return };
    let mut renderer = SkinnedModelRenderer::new(g.clone(), FORMAT, strip()).unwrap();
    renderer.clear = [0.0, 0.0, 0.0, 1.0];
    let rest = draw(&g, &mut renderer, 1).unwrap();
    let rest_bounds = renderer.bounds()[0];
    renderer.set_morph_weights(&[vec![vec![1.0]]]);
    let slid = draw(&g, &mut renderer, 1).unwrap();
    let slid_bounds = renderer.bounds()[0];
    assert!((slid_bounds.min[0] - rest_bounds.min[0] - 1.0).abs() < 1e-4 && (slid_bounds.max[0] - rest_bounds.max[0] - 1.0).abs() < 1e-4, "the bounds move 1 m along x: {rest_bounds:?} -> {slid_bounds:?}");
    // The view is 8 m tall in 64 pixels: 1 m is 8 pixels.
    let shift = center_x(&slid) - center_x(&rest);
    assert!((shift - 8.0).abs() < 1.0, "the drawn strip moves 8 pixels right: {shift}");
    // Back at weight zero the shared vertices draw the same picture as before.
    renderer.set_morph_weights(&[vec![vec![0.0]]]);
    assert_eq!(draw(&g, &mut renderer, 1).unwrap(), rest);
}

#[test]
fn each_instance_keeps_its_own_weights_and_bad_weights_fail_the_draw() {
    let Some(g) = gpu() else { return };
    let mut renderer = SkinnedModelRenderer::new(g.clone(), FORMAT, strip()).unwrap();
    // Only the first instance has weights; the second keeps the model's default.
    renderer.set_morph_weights(&[vec![vec![1.0]]]);
    draw(&g, &mut renderer, 2).unwrap();
    let [first, second] = [renderer.bounds()[0], renderer.bounds()[1]];
    assert!((first.min[0] - second.min[0] - 1.0).abs() < 1e-4, "only the first instance slides: {first:?} {second:?}");
    renderer.set_morph_weights(&[vec![vec![1.0, 2.0]]]);
    assert!(draw(&g, &mut renderer, 2).is_err(), "one weight per target");
    assert_eq!(renderer.bounds()[0], first, "a failed draw keeps the published bounds");
}
