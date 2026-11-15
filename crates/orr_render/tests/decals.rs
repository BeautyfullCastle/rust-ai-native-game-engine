//! Decals of the imported scene, checked on the picture: they tint only what
//! is inside their box and faces the projection, in their shape or atlas region.
#![cfg(feature = "imported-scene")]
#![allow(clippy::float_arithmetic)]

use orr_model::{StaticModel, Vertex, IDENTITY};
use orr_render::orr_rhi::{Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions};
use orr_render::{
    Camera3D, Decal, DecalAtlasError, DecalShape, ImportedBatch, ImportedSceneRenderer, ImportedSceneTarget, Lighting, ModelRenderer,
    PointLightSettings, PostProcessSettings, StaticInstance,
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

/// A unit square facing +z in one light gray, made from the imported test model.
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
    source.materials[0].base_color = [0.8, 0.8, 0.8, 1.0];
    for image in &mut source.images {
        for pixel in image.rgba8.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[255, 255, 255, 255]);
        }
    }
    StaticModel::new(source).unwrap()
}

/// The square filling the view, lit evenly, with `decals` on it.
fn render(g: &Wgpu, decals: &[Decal], post: bool, atlas: Option<(u32, u32, &[u8])>) -> Vec<u8> {
    let mut scene = ImportedSceneRenderer::new(g.clone(), FORMAT).unwrap();
    scene.post_process = PostProcessSettings { enabled: post, bloom: false, ..Default::default() };
    scene.decals = decals.to_vec();
    if let Some((w, h, rgba)) = atlas {
        scene.set_decal_atlas(w, h, rgba).unwrap();
    }
    let format = scene.scene_format();
    let mut renderer = ModelRenderer::new(g.clone(), format, square()).unwrap();
    let texture = g.create_texture(&TextureDesc {
        label: "decal test",
        width: SIZE,
        height: SIZE,
        format: FORMAT,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let view = g.create_texture_view(&texture, None);
    let light = Lighting { sky: [1.0; 3], ground: [1.0; 3], ambient: 1.0, intensity: 0.0, shadows: false, tonemap: false, exposure: 1.0, ..Default::default() };
    let instances = [StaticInstance { scale: [3.0; 3], ..Default::default() }];
    scene
        .draw(
            ImportedSceneTarget { view: &view, size: (SIZE, SIZE), format: FORMAT, sample_count: 1 },
            &Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 1.0),
            &light,
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

/// A one meter box on the square, projected into it along -z.
fn on_square(color: [f32; 4], shape: DecalShape) -> Decal {
    Decal { center: [0.0; 3], direction: [0.0, 0.0, -1.0], up: [0.0, 1.0, 0.0], size: [1.0; 3], color, shape, softness: 0.0, region: None }
}

fn untouched(plain: &[u8], image: &[u8]) -> usize {
    plain.chunks_exact(4).zip(image.chunks_exact(4)).filter(|(a, b)| a[..3].iter().zip(&b[..3]).all(|(x, y)| x.abs_diff(*y) <= 2)).count()
}

#[test]
fn a_decal_tints_only_what_is_inside_its_box() {
    let Some(g) = gpu() else { return };
    for post in [true, false] {
        let plain = render(&g, &[], post, None);
        let image = render(&g, &[on_square([1.0, 0.1, 0.1, 1.0], DecalShape::Square)], post, None);
        let gray = px(&plain, 32, 32);
        assert!(gray[0] > 150 && gray[0] == gray[1], "the square is light gray: {gray:?}");
        let [r, g_, b] = px(&image, 32, 32);
        // The picture is sRGB encoded: a tenth of linear light shows at about a third.
        assert!(r + 3 >= gray[0] && g_ < gray[1] / 2 && b < gray[2] / 2, "the center is tinted red: {:?}, post {post}", [r, g_, b]);
        // The box covers a quarter of the view (1 of 2 meters each way).
        let same = untouched(&plain, &image);
        let total = (SIZE * SIZE) as usize;
        assert!((total * 70 / 100..total * 80 / 100).contains(&same), "outside the box is unchanged: {same} of {total}, post {post}");
    }
}

#[test]
fn a_round_decal_leaves_the_corners_of_its_box() {
    let Some(g) = gpu() else { return };
    let plain = render(&g, &[], true, None);
    let dark = [0.1, 0.1, 0.1, 1.0];
    let round = render(&g, &[on_square(dark, DecalShape::Round)], true, None);
    let square = render(&g, &[on_square(dark, DecalShape::Square)], true, None);
    // World (0.45, 0.45) is inside the box and outside the circle.
    assert_eq!(px(&round, 46, 17), px(&plain, 46, 17));
    assert!(px(&square, 46, 17)[0] < px(&plain, 46, 17)[0] / 2);
    assert!(px(&round, 32, 32)[0] < px(&plain, 32, 32)[0] / 2);
}

#[test]
fn surfaces_along_the_projection_and_outside_its_depth_are_left_alone() {
    let Some(g) = gpu() else { return };
    let plain = render(&g, &[], true, None);
    let dark = [0.1, 0.1, 0.1, 1.0];
    // Projected sideways, the square runs along the projection and fades out.
    let sideways = Decal { direction: [1.0, 0.0, 0.0], size: [1.0, 1.0, 3.0], ..on_square(dark, DecalShape::Square) };
    // A box in front of the square that ends before reaching it.
    let short = Decal { center: [0.0, 0.0, 1.0], ..on_square(dark, DecalShape::Square) };
    for decal in [sideways, short] {
        let image = render(&g, &[decal], true, None);
        assert_eq!(untouched(&plain, &image), (SIZE * SIZE) as usize, "{decal:?}");
    }
}

#[test]
fn a_decal_draws_its_atlas_region() {
    let Some(g) = gpu() else { return };
    let plain = render(&g, &[], true, None);
    // Left pixel opaque blue, right pixel transparent.
    let atlas: &[u8] = &[0, 0, 255, 255, 255, 255, 255, 0];
    let region = |r: [f32; 4]| Decal { region: Some(r), ..on_square([1.0; 4], DecalShape::Square) };
    let blue = render(&g, &[region([0.0, 0.0, 0.5, 1.0])], true, Some((2, 1, atlas)));
    let [r, _, b] = px(&blue, 32, 32);
    assert!(r < 60 && b > 150, "the opaque blue pixel tints the square: {:?}", px(&blue, 32, 32));
    let clear = render(&g, &[region([0.5, 0.0, 1.0, 1.0])], true, Some((2, 1, atlas)));
    assert_eq!(untouched(&plain, &clear), (SIZE * SIZE) as usize);
}

#[test]
fn a_wrong_atlas_is_refused() {
    let Some(g) = gpu() else { return };
    let mut scene = ImportedSceneRenderer::new(g, FORMAT).unwrap();
    assert_eq!(scene.set_decal_atlas(0, 1, &[]), Err(DecalAtlasError::Size));
    assert_eq!(scene.set_decal_atlas(2, 2, &[0; 12]), Err(DecalAtlasError::DataLength));
    assert_eq!(scene.set_decal_atlas(1, 1, &[0; 4]), Ok(()));
}
