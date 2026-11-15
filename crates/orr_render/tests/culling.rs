//! Culling of procedural shapes in the imported scene: it leaves out what is
//! off screen or behind a wall and never changes the picture, shadows included.
#![cfg(feature = "imported-scene")]
#![allow(clippy::float_arithmetic)]

use orr_render::orr_rhi::{Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions};
use orr_render::{
    Camera3D, Culling, ImportedBatch, ImportedSceneRenderer, ImportedSceneTarget, Lighting, Material, PointLightSettings, RenderList3D, Renderer3D, Settings3D,
    IDENTITY_ROT,
};

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

/// Ground, a wall with a ball hidden behind it, and a big ball just off the
/// left of the picture whose shadow the low sun throws into it.
fn scene() -> RenderList3D {
    let mut list = RenderList3D::new();
    list.plane([0.0; 3], 20.0, 20.0, &Material::new([0.8; 3]));
    list.cuboid([0.0, 2.0, 4.0], IDENTITY_ROT, [4.0, 2.0, 0.2], &Material::new([0.6, 0.3, 0.2]));
    list.sphere([0.0, 1.0, 0.0], IDENTITY_ROT, 0.5, &Material::new([0.2, 0.8, 0.2]));
    list.sphere([-12.0, 3.0, 0.0], IDENTITY_ROT, 1.5, &Material::new([0.2, 0.2, 0.9]));
    list
}

fn draw(g: &Wgpu, culling: Culling) -> (Vec<u8>, orr_render::FrameStats) {
    let texture = g.create_texture(&TextureDesc {
        label: "culling test",
        width: SIZE,
        height: SIZE,
        format: FORMAT,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let view = g.create_texture_view(&texture, None);
    let mut scene_renderer = ImportedSceneRenderer::new(g.clone(), FORMAT).unwrap();
    scene_renderer.culling = culling;
    let mut procedural = Renderer3D::with_settings(g.clone(), FORMAT, Settings3D::LOW);
    let list = scene();
    let light = Lighting { direction: [1.0, -0.3, 0.0], shadows: true, shadow_center: [0.0; 3], shadow_radius: 25.0, ..Lighting::default() };
    scene_renderer
        .draw(
            ImportedSceneTarget { view: &view, size: (SIZE, SIZE), format: FORMAT, sample_count: 1 },
            &Camera3D::perspective([0.0, 3.0, 12.0], [0.0, 1.0, 0.0], 60.0),
            &light,
            &PointLightSettings::default(),
            &mut [ImportedBatch::Procedural { renderer: &mut procedural, list: &list }],
        )
        .unwrap();
    (g.read_texture(&texture), procedural.last_frame_stats())
}

#[test]
fn culling_leaves_out_hidden_shapes_without_changing_the_picture() {
    let Some(g) = gpu() else { return };
    let (all, all_stats) = draw(&g, Culling { frustum: false, occlusion: false });
    let (frustum, frustum_stats) = draw(&g, Culling { frustum: true, occlusion: false });
    let (both, both_stats) = draw(&g, Culling::default());
    assert_eq!(all_stats.culled_instances, 0);
    assert_eq!(frustum_stats.culled_instances, 1, "the ball off the left of the picture");
    assert_eq!(both_stats.culled_instances, 2, "and the ball behind the wall");
    assert_eq!(both_stats.main.instances, all_stats.main.instances - 2);
    // The shadow pass still draws the off-screen ball, so its shadow stays in the picture.
    assert_eq!(both_stats.shadow.instances, all_stats.shadow.instances);
    assert!(all == frustum && all == both, "culling never changes a pixel");
}
