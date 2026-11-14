//! Sun shadows in cascades: a long ground seen in perspective, one caster near
//! the camera and one far away. With four cascades both shadows show, and the
//! near one comes from a much finer tile than one map over the whole range.
#![cfg(feature = "imported-scene")]
#![allow(clippy::float_arithmetic)]

use orr_model::{StaticModel, Vertex, IDENTITY};
use orr_render::orr_rhi::{Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions};
use orr_render::{Camera3D, ImportedBatch, ImportedSceneRenderer, ImportedSceneTarget, Lighting, ModelRenderer, PointLightSettings, StaticInstance};

const SIZE: (u32, u32) = (320, 200);
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

/// A white unit square in the xy plane facing +z, from the imported test model.
fn square() -> StaticModel {
    let imported = orr_model::import::import_with_resolver("fixtures/model.glb", include_bytes!("../../orr_model/tests/fixtures/model.glb"), |_| panic!("fixture embeds every dependency")).unwrap();
    let mut source = StaticModel::from_bytes(&imported.to_bytes().unwrap()).unwrap().source().clone();
    source.primitives.truncate(1);
    let p = &mut source.primitives[0];
    p.vertices = [[-0.5, -0.5, 0.0], [0.5, -0.5, 0.0], [0.5, 0.5, 0.0], [-0.5, 0.5, 0.0]].into_iter().map(|position| Vertex { position, normal: [0.0, 0.0, 1.0], uv: [0.25, 0.25] }).collect();
    p.indices = vec![0, 1, 2, 0, 2, 3];
    p.transform = IDENTITY;
    p.material = 0;
    source.materials[0].base_color = [1.0; 4];
    for image in &mut source.images {
        for pixel in image.rgba8.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[255, 255, 255, 255]);
        }
    }
    StaticModel::new(source).unwrap()
}

/// The square laid flat facing up: a quarter turn about x.
fn ground(center: [f32; 3], side: f32) -> StaticInstance {
    let half = std::f32::consts::FRAC_PI_4;
    StaticInstance { translation: center, rotation: [-half.sin(), 0.0, 0.0, half.cos()], scale: [side, side, 1.0] }
}

fn panel(center: [f32; 3], side: f32) -> StaticInstance {
    StaticInstance { translation: center, scale: [side, side, 1.0], ..Default::default() }
}

fn camera() -> Camera3D {
    Camera3D::perspective([0.0, 12.0, 10.0], [0.0, 0.0, -40.0], 60.0)
}

/// A sun behind the panels, so each throws its shadow toward the camera.
fn sun() -> Lighting {
    let d = [0.0f32, -1.0, 0.8];
    let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    Lighting {
        direction: d.map(|c| c / length),
        color: [1.0; 3],
        intensity: 1.0,
        sky: [1.0; 3],
        ground: [1.0; 3],
        ambient: 0.15,
        shadows: true,
        shadow_center: [0.0, 0.0, -80.0],
        shadow_radius: 120.0,
        tonemap: false,
        exposure: 1.0,
    }
}

fn render(g: &Wgpu, cascades: u32) -> Vec<u8> {
    let mut scene = ImportedSceneRenderer::new(g.clone(), FORMAT).unwrap();
    scene.shadow_cascades = cascades;
    let mut renderer = ModelRenderer::new(g.clone(), FORMAT, square()).unwrap();
    let instances = [ground([0.0, 0.0, -80.0], 240.0), panel([0.0, 1.5, -6.0], 2.0), panel([0.0, 6.0, -110.0], 8.0)];
    let texture = g.create_texture(&TextureDesc { label: "cascades", width: SIZE.0, height: SIZE.1, format: FORMAT, usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC, sample_count: 1, view_formats: &[] });
    let view = g.create_texture_view(&texture, None);
    scene
        .draw(ImportedSceneTarget { view: &view, size: SIZE, format: FORMAT, sample_count: 1 }, &camera(), &sun(), &PointLightSettings::default(), &mut [ImportedBatch::StaticInstances { renderer: &mut renderer, instances: &instances }])
        .unwrap();
    g.read_texture(&texture)
}

/// The red channel of the ground pixel at world position `at`.
fn ground_at(image: &[u8], at: [f32; 3]) -> u8 {
    let screen = camera().world_to_screen(at, SIZE).expect("on screen");
    let (x, y) = (screen[0] as usize, screen[1] as usize);
    image[(y * SIZE.0 as usize + x) * 4]
}

/// How many pixels of the row through `at` step from light to dark or back:
/// a sharp shadow edge is one step, a blurred one spreads over several.
fn edge_width(image: &[u8], at: [f32; 3]) -> usize {
    let screen = camera().world_to_screen(at, SIZE).expect("on screen");
    let y = screen[1] as usize;
    let row: Vec<u8> = (0..SIZE.0 as usize).map(|x| image[(y * SIZE.0 as usize + x) * 4]).collect();
    let (lit, dark) = (*row.iter().max().unwrap(), *row.iter().min().unwrap());
    let (low, high) = (dark as u32 + (lit - dark) as u32 / 5, lit as u32 - (lit - dark) as u32 / 5);
    row.iter().filter(|v| (low..high).contains(&u32::from(**v))).count()
}

#[test]
fn four_cascades_show_a_sharp_near_shadow_and_still_a_far_one() {
    let Some(g) = gpu() else { return };
    let cascaded = render(&g, 4);
    // The near panel's shadow falls toward the camera, the far panel's too.
    let (near_shadow, near_lit) = (ground_at(&cascaded, [0.0, 0.0, -5.0]), ground_at(&cascaded, [4.0, 0.0, -5.0]));
    let (far_shadow, far_lit) = (ground_at(&cascaded, [0.0, 0.0, -105.0]), ground_at(&cascaded, [14.0, 0.0, -105.0]));
    assert!(near_shadow + 60 < near_lit, "near shadow: {near_shadow} vs lit {near_lit}");
    assert!(far_shadow + 60 < far_lit, "far shadow: {far_shadow} vs lit {far_lit}");

    // One map over the same range blurs the near edge over more pixels.
    let single = render(&g, 1);
    let (sharp, soft) = (edge_width(&cascaded, [0.0, 0.0, -5.0]), edge_width(&single, [0.0, 0.0, -5.0]));
    assert!(sharp < soft, "near edge pixels: cascades {sharp}, one map {soft}");
}
