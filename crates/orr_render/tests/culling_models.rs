//! Culling of instanced static and skinned models in the imported scene: draws
//! off screen or behind a procedural wall leave the main pass, the picture and
//! the shadows stay the same.
#![cfg(feature = "imported-scene")]
#![allow(clippy::float_arithmetic)]

use orr_model::{StaticModel, Vertex, IDENTITY};
use orr_render::orr_rhi::{Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions};
use orr_render::{
    Camera3D, Culling, ImportedBatch, ImportedSceneRenderer, ImportedSceneTarget, Lighting, Material, ModelRenderer, PointLightSettings, RenderList3D, Renderer3D,
    Settings3D, StaticInstance, IDENTITY_ROT,
};
#[cfg(feature = "animation")]
use orr_render::{SkinnedInstance, SkinnedModelRenderer};

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

/// A unit square facing +z, made from the imported test model.
fn square() -> StaticModel {
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
    source.materials[0].base_color = [0.9, 0.5, 0.2, 1.0];
    for image in &mut source.images {
        image.rgba8.fill(255);
    }
    StaticModel::new(source).unwrap()
}

/// Ground and a wall 8 m wide and 4 m tall, 4 m in front of the origin.
fn walls() -> RenderList3D {
    let mut list = RenderList3D::new();
    list.plane([0.0; 3], 20.0, 20.0, &Material::new([0.8; 3]));
    list.cuboid([0.0, 2.0, 4.0], IDENTITY_ROT, [4.0, 2.0, 0.2], &Material::new([0.6, 0.3, 0.2]));
    list
}

/// Squares in front of the wall, behind it, and off the left of the picture,
/// turned to the low sun so its shadow falls into the picture.
fn squares() -> [StaticInstance; 3] {
    let turned = [0.0, std::f32::consts::FRAC_1_SQRT_2, 0.0, std::f32::consts::FRAC_1_SQRT_2];
    [
        StaticInstance { translation: [2.0, 1.0, 7.0], ..Default::default() },
        StaticInstance { translation: [0.0, 1.0, 0.0], ..Default::default() },
        StaticInstance { translation: [-12.0, 3.0, 0.0], rotation: turned, scale: [3.0; 3] },
    ]
}

fn target(g: &Wgpu) -> (<Wgpu as Rhi>::Texture, <Wgpu as Rhi>::TextureView) {
    let texture = g.create_texture(&TextureDesc {
        label: "model culling test",
        width: SIZE,
        height: SIZE,
        format: FORMAT,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let view = g.create_texture_view(&texture, None);
    (texture, view)
}

fn camera() -> Camera3D {
    Camera3D::perspective([0.0, 3.0, 12.0], [0.0, 1.0, 0.0], 60.0)
}

fn light() -> Lighting {
    Lighting { direction: [1.0, -0.3, 0.0], shadows: true, shadow_center: [0.0; 3], shadow_radius: 25.0, ..Lighting::default() }
}

fn draw_static(g: &Wgpu, culling: Culling) -> (Vec<u8>, usize) {
    let (texture, view) = target(g);
    let mut scene = ImportedSceneRenderer::new(g.clone(), FORMAT).unwrap();
    scene.culling = culling;
    let mut procedural = Renderer3D::with_settings(g.clone(), FORMAT, Settings3D::LOW);
    let mut model = ModelRenderer::new(g.clone(), scene.scene_format(), square()).unwrap();
    let (list, instances) = (walls(), squares());
    scene
        .draw(
            ImportedSceneTarget { view: &view, size: (SIZE, SIZE), format: FORMAT, sample_count: 1 },
            &camera(),
            &light(),
            &PointLightSettings::default(),
            &mut [ImportedBatch::Procedural { renderer: &mut procedural, list: &list }, ImportedBatch::StaticInstances { renderer: &mut model, instances: &instances }],
        )
        .unwrap();
    (g.read_texture(&texture), scene.culled_model_draws())
}

#[test]
fn static_models_off_screen_or_behind_a_wall_are_culled_without_changing_the_picture() {
    let Some(g) = gpu() else { return };
    let (all, none_culled) = draw_static(&g, Culling { frustum: false, occlusion: false });
    let (frustum, frustum_culled) = draw_static(&g, Culling { frustum: true, occlusion: false });
    let (both, both_culled) = draw_static(&g, Culling::default());
    assert_eq!(none_culled, 0);
    assert_eq!(frustum_culled, 1, "the square off the left of the picture");
    assert_eq!(both_culled, 2, "and the square behind the wall");
    assert!(all == frustum && all == both, "culling never changes a pixel, shadows included");
}

/// One model of three squares, each placed by its own node transform: in front of
/// the wall, behind it, and off the left of the picture turned to the low sun.
fn placed_squares() -> StaticModel {
    let mut source = square().source().clone();
    let first = source.primitives[0].clone();
    let at = |x: f32, y: f32, z: f32| [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [x, y, z, 1.0]];
    let turned_and_big = [[0.0, 0.0, -3.0, 0.0], [0.0, 3.0, 0.0, 0.0], [3.0, 0.0, 0.0, 0.0], [-12.0, 3.0, 0.0, 1.0]];
    source.primitives = [at(2.0, 1.0, 7.0), at(0.0, 1.0, 0.0), turned_and_big]
        .into_iter()
        .enumerate()
        .map(|(index, transform)| {
            let mut p = first.clone();
            p.id = format!("{}#{index}", first.id);
            p.transform = transform;
            p
        })
        .collect();
    StaticModel::new(source).unwrap()
}

fn draw_static_batch(g: &Wgpu, culling: Culling) -> (Vec<u8>, usize) {
    let (texture, view) = target(g);
    let mut scene = ImportedSceneRenderer::new(g.clone(), FORMAT).unwrap();
    scene.culling = culling;
    let mut procedural = Renderer3D::with_settings(g.clone(), FORMAT, Settings3D::LOW);
    let mut model = ModelRenderer::new(g.clone(), scene.scene_format(), placed_squares()).unwrap();
    let list = walls();
    scene
        .draw(
            ImportedSceneTarget { view: &view, size: (SIZE, SIZE), format: FORMAT, sample_count: 1 },
            &camera(),
            &light(),
            &PointLightSettings::default(),
            &mut [ImportedBatch::Procedural { renderer: &mut procedural, list: &list }, ImportedBatch::Static(&mut model)],
        )
        .unwrap();
    (g.read_texture(&texture), scene.culled_model_draws())
}

#[test]
fn draws_of_a_static_batch_off_screen_or_behind_a_wall_are_culled_one_by_one_without_changing_the_picture() {
    let Some(g) = gpu() else { return };
    let (all, none_culled) = draw_static_batch(&g, Culling { frustum: false, occlusion: false });
    let (frustum, frustum_culled) = draw_static_batch(&g, Culling { frustum: true, occlusion: false });
    let (both, both_culled) = draw_static_batch(&g, Culling::default());
    assert_eq!(none_culled, 0);
    assert_eq!(frustum_culled, 1, "the square off the left of the picture");
    assert_eq!(both_culled, 2, "and the square behind the wall");
    assert!(all == frustum && all == both, "culling never changes a pixel, shadows included");
}

#[cfg(feature = "animation")]
fn animated() -> orr_model::animation::AnimatedModel {
    let imported = orr_model::animation_import::import_with_resolver(
        "fixtures/animated_strip.glb",
        include_bytes!("../../orr_model/tests/fixtures/animated_strip.glb"),
        |_| panic!("fixture embeds every dependency"),
    )
    .unwrap();
    orr_model::animation::AnimatedModel::new(imported.source().clone()).unwrap()
}

#[cfg(feature = "animation")]
fn translation(x: f32, y: f32, z: f32) -> [[f32; 4]; 4] {
    [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [x, y, z, 1.0]]
}

#[cfg(feature = "animation")]
#[test]
fn skinned_models_behind_a_wall_are_culled_without_changing_the_picture() {
    let Some(g) = gpu() else { return };
    let asset = animated();
    let rest = asset.rest_pose().unwrap();
    let draw = |culling: Culling| {
        let (texture, view) = target(&g);
        let mut scene = ImportedSceneRenderer::new(g.clone(), FORMAT).unwrap();
        scene.culling = culling;
        let mut procedural = Renderer3D::with_settings(g.clone(), FORMAT, Settings3D::LOW);
        let mut model = SkinnedModelRenderer::new(g.clone(), scene.scene_format(), asset.clone()).unwrap();
        let instances = [
            SkinnedInstance { pose: &rest, transform: translation(2.5, 0.0, 7.0) },
            SkinnedInstance { pose: &rest, transform: translation(0.0, 0.0, 0.0) },
        ];
        let list = walls();
        scene
            .draw(
                ImportedSceneTarget { view: &view, size: (SIZE, SIZE), format: FORMAT, sample_count: 1 },
                &camera(),
                &light(),
                &PointLightSettings::default(),
                &mut [ImportedBatch::Procedural { renderer: &mut procedural, list: &list }, ImportedBatch::Skinned { renderer: &mut model, instances: &instances }],
            )
            .unwrap();
        (g.read_texture(&texture), scene.culled_model_draws())
    };
    let (all, none_culled) = draw(Culling { frustum: false, occlusion: false });
    let (both, both_culled) = draw(Culling::default());
    let primitives = asset.source().primitives.len();
    assert_eq!(none_culled, 0);
    assert_eq!(both_culled, primitives, "every draw of the strip behind the wall");
    assert!(all == both, "culling never changes a pixel");
}
