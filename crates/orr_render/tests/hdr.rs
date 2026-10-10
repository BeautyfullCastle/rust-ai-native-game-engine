//! Mandatory GPU acceptance for scene-linear HDR and bounded bloom.
//!
//! These tests never turn missing adapters/capabilities into a successful skip.
//! Color expectations are independent scalar references, not renderer helpers.
#![cfg(feature = "imported-scene")]
#![allow(clippy::float_arithmetic)]

use std::sync::{Mutex, MutexGuard};

use orr_model::{IDENTITY, StaticModel, Vertex};
#[cfg(feature = "animation")]
use orr_model::{animation::AnimatedModel, animation_import};
use orr_render::orr_rhi::{
    Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions, decode_rgba16f_texel,
};
use orr_render::{
    Camera3D, IDENTITY_ROT, ImportedBatch, ImportedSceneError, ImportedSceneRenderer,
    ImportedSceneTarget, Lighting, Material, ModelRenderer, PointLight, PointLightSettings,
    PostProcessSettings, RenderList3D, Renderer3D, Settings3D, StaticInstance,
};
#[cfg(feature = "animation")]
use orr_render::{SkinnedInstance, SkinnedModelRenderer};

type Texture = <Wgpu as Rhi>::Texture;
type View = <Wgpu as Rhi>::TextureView;
const SIZE: (u32, u32) = (65, 67); // Unaligned rows and odd bloom extents.
const FORMATS: [TextureFormat; 2] = [TextureFormat::Rgba8Unorm, TextureFormat::Rgba8UnormSrgb];
static GPU_LOCK: Mutex<()> = Mutex::new(());

fn gpu() -> (MutexGuard<'static, ()>, Wgpu) {
    let lock = GPU_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
    let gpu = Wgpu::headless(WgpuOptions::default())
        .expect("HDR acceptance requires an adapter; absence is a failure, never a skip");
    let info = gpu.adapter().get_info();
    eprintln!(
        "HDR acceptance adapter: name={:?} backend={:?} device_type={:?} vendor={} device={} driver={:?} driver_info={:?} software={}",
        info.name,
        info.backend,
        info.device_type,
        info.vendor,
        info.device,
        info.driver,
        info.driver_info,
        gpu.is_software()
    );
    (lock, gpu)
}

fn camera() -> Camera3D {
    Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 1.0)
}

fn lighting(radiance: f32) -> Lighting {
    Lighting {
        sky: [1.0; 3],
        ground: [1.0; 3],
        ambient: radiance,
        intensity: 0.0,
        shadows: false,
        exposure: 1.0,
        tonemap: false,
        ..Default::default()
    }
}

fn target(g: &Wgpu, format: TextureFormat, size: (u32, u32)) -> Texture {
    g.create_texture(&TextureDesc {
        label: "HDR acceptance display",
        width: size.0,
        height: size.1,
        format,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    })
}

fn scene(g: &Wgpu, format: TextureFormat) -> ImportedSceneRenderer<Wgpu> {
    let mut scene = ImportedSceneRenderer::new(g.clone(), format).unwrap();
    assert!(!scene.post_process.enabled, "HDR must remain opt-in");
    assert_eq!(scene.scene_format(), format);
    scene.post_process.enabled = true;
    scene.post_process.bloom = false;
    scene.clear = [0.0, 0.0, 0.0, 1.0];
    assert_eq!(scene.scene_format(), TextureFormat::Rgba16Float);
    assert_eq!(scene.format(), format, "display target stays LDR");
    scene
}

fn draw(
    scene: &mut ImportedSceneRenderer<Wgpu>,
    view: &View,
    size: (u32, u32),
    camera: &Camera3D,
    lighting: &Lighting,
    batches: &mut [ImportedBatch<'_, Wgpu>],
) -> Result<(), ImportedSceneError> {
    let format = scene.format();
    scene.draw(
        ImportedSceneTarget {
            view,
            size,
            format,
            sample_count: 1,
        },
        camera,
        lighting,
        &PointLightSettings::default(),
        batches,
    )
}

/// Import, edit an original test panel, cook and reload. Every input texture
/// texel is white so the expected scene radiance needs no texture-sampling code.
fn panel(half: f32) -> StaticModel {
    let imported = orr_model::import::import_with_resolver(
        "fixtures/model.glb",
        include_bytes!("../../orr_model/tests/fixtures/model.glb"),
        |_| panic!("fixture embeds every dependency"),
    )
    .unwrap();
    let mut source = imported.source().clone();
    source.primitives.truncate(1);
    let primitive = &mut source.primitives[0];
    primitive.vertices = [
        [-half, -half, 0.0],
        [half, -half, 0.0],
        [half, half, 0.0],
        [-half, half, 0.0],
    ]
    .into_iter()
    .map(|position| Vertex {
        position,
        normal: [0.0, 0.0, 1.0],
        uv: [0.25, 0.25],
    })
    .collect();
    primitive.indices = vec![0, 1, 2, 0, 2, 3];
    primitive.transform = IDENTITY;
    primitive.material = 0;
    for material in &mut source.materials {
        material.base_color = [1.0; 4];
    }
    for image in &mut source.images {
        image.rgba8.fill(255);
    }
    let model = StaticModel::new(source).unwrap();
    StaticModel::from_bytes(&model.to_bytes().unwrap()).unwrap()
}

#[cfg(feature = "animation")]
fn animated() -> AnimatedModel {
    let imported = animation_import::import_with_resolver(
        "fixtures/animated_strip.glb",
        include_bytes!("../../orr_model/tests/fixtures/animated_strip.glb"),
        |_| panic!("fixture embeds every dependency"),
    )
    .unwrap();
    let mut source = imported.source().clone();
    for material in &mut source.materials {
        material.base_color = [1.0; 4];
    }
    for image in &mut source.images {
        image.rgba8.fill(255);
    }
    let model = AnimatedModel::new(source).unwrap();
    AnimatedModel::from_bytes(&model.to_bytes().unwrap()).unwrap()
}

fn hdr_bytes(g: &Wgpu, scene: &ImportedSceneRenderer<Wgpu>) -> Vec<u8> {
    let size = scene
        .post_process_size()
        .expect("accepted HDR frame allocated a scene target");
    let bytes = g.read_texture(scene.hdr_scene_texture().expect("HDR scene texture"));
    assert_eq!(bytes.len(), (size.0 * size.1 * 8) as usize);
    bytes
}

fn hdr_pixels(g: &Wgpu, scene: &ImportedSceneRenderer<Wgpu>) -> Vec<[f32; 4]> {
    hdr_bytes(g, scene)
        .chunks_exact(8)
        .map(|pixel| decode_rgba16f_texel(pixel.try_into().unwrap()))
        .collect()
}

fn near_hdr(actual: [f32; 4], expected: [f32; 3], label: &str) {
    for (channel, expected) in expected.into_iter().enumerate() {
        let tolerance = (expected.abs() * 0.001).max(0.000_01);
        assert!(
            (actual[channel] - expected).abs() <= tolerance,
            "{label}: channel {channel}, actual {actual:?}, expected {expected}"
        );
    }
    assert_eq!(actual[3], 1.0, "opaque scene alpha: {label}");
}

fn uniform_foreground(pixels: &[[f32; 4]], expected: [f32; 3], label: &str) {
    let mut foreground = 0;
    for &pixel in pixels {
        assert!(
            pixel
                .iter()
                .all(|v| v.is_finite() && *v >= 0.0 && *v <= 65504.0)
        );
        if pixel[0] > 0.0 || pixel[1] > 0.0 || pixel[2] > 0.0 {
            near_hdr(pixel, expected, label);
            foreground += 1;
        }
    }
    assert!(
        foreground > 100,
        "{label}: only {foreground} foreground pixels"
    );
    assert!(
        foreground < pixels.len() - 100,
        "{label}: expected a clear border"
    );
}

/// Independent f64 exposure -> ACES (optional) -> sRGB -> UNORM reference.
/// The same display bytes must result whether encoding is shader-side (UNORM)
/// or fixed-function (sRGB). In particular, exposure must not happen twice.
fn display(radiance: [f32; 3], light: &Lighting) -> [u8; 4] {
    let mut output = [0, 0, 0, 255];
    for (channel, radiance) in radiance.into_iter().enumerate() {
        let mut x = f64::from(radiance) * f64::from(light.exposure);
        if light.tonemap {
            x = x * (2.51 * x + 0.03) / (x * (2.43 * x + 0.59) + 0.14);
        }
        x = x.clamp(0.0, 1.0);
        let encoded = if x <= 0.003_130_8 {
            12.92 * x
        } else {
            1.055 * x.powf(1.0 / 2.4) - 0.055
        };
        output[channel] = (encoded * 255.0).round() as u8;
    }
    output
}

fn near_display(actual: &[u8], expected: [u8; 4], label: &str) {
    assert!(
        actual
            .iter()
            .zip(expected)
            .all(|(&a, b)| a.abs_diff(b) <= 2),
        "{label}: actual {actual:?}, expected {expected:?}"
    );
}

fn near_images(a: &[u8], b: &[u8], label: &str) {
    assert_eq!(a.len(), b.len());
    assert!(
        a.iter().zip(b).all(|(a, b)| a.abs_diff(*b) <= 2),
        "{label}: maximum difference {}",
        a.iter()
            .zip(b)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0)
    );
}

/// Optional reproducible evidence. Raw half-float scene data is deliberately
/// separate from the display-referred PPM and is never presented as an LDR image.
fn capture(
    g: &Wgpu,
    scene: &ImportedSceneRenderer<Wgpu>,
    texture: &Texture,
    light: &Lighting,
    label: &str,
) {
    let Some(directory) = std::env::var_os("ORR_HDR_CAPTURE_DIR") else {
        return;
    };
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory).expect("create HDR evidence directory");
    let size = scene.post_process_size().unwrap();
    let stem = format!("{label}-{:?}", scene.format());
    std::fs::write(
        directory.join(format!("{stem}.rgba16f")),
        hdr_bytes(g, scene),
    )
    .expect("write scene-linear little-endian RGBA16F evidence");
    let mut ppm = format!("P6\n{} {}\n255\n", size.0, size.1).into_bytes();
    for rgba in g.read_texture(texture).chunks_exact(4) {
        ppm.extend_from_slice(&rgba[..3]);
    }
    std::fs::write(directory.join(format!("{stem}.ppm")), ppm)
        .expect("write display-referred RGB8 evidence");
    let info = g.adapter().get_info();
    let metadata = serde_json::json!({
        "width": size.0, "height": size.1,
        "adapter": g.adapter_name(), "software": g.is_software(),
        "backend": format!("{:?}", info.backend), "device_type": format!("{:?}", info.device_type),
        "vendor": info.vendor, "device": info.device, "driver": info.driver, "driver_info": info.driver_info,
        "scene": { "file": format!("{stem}.rgba16f"), "format": "RGBA16F", "endian": "little", "row_order": "top_first", "color_space": "scene_linear_unexposed", "bytes_per_texel": 8 },
        "display": { "file": format!("{stem}.ppm"), "format": format!("{:?}", scene.format()), "encoding": "sRGB_display_RGB8", "exposure": light.exposure, "aces": light.tonemap },
        "lighting": { "ambient": light.ambient, "sky": light.sky, "ground": light.ground, "intensity": light.intensity },
        "bloom": { "enabled": scene.post_process.bloom, "threshold": scene.post_process.threshold, "strength": scene.post_process.strength, "radius": scene.post_process.radius, "iterations": scene.post_process.iterations }
    });
    std::fs::write(
        directory.join(format!("{stem}.json")),
        serde_json::to_vec_pretty(&metadata).unwrap(),
    )
    .expect("write HDR evidence metadata");
}

#[test]
fn procedural_and_static_store_unexposed_radiance_above_one_and_finite_half_maximum() {
    let (_lock, g) = gpu();
    let texture = target(&g, FORMATS[0], SIZE);
    let view = g.create_texture_view(&texture, None);
    let mut scene = scene(&g, FORMATS[0]);
    let mut model = ModelRenderer::new(g.clone(), scene.scene_format(), panel(0.65)).unwrap();
    let mut procedural =
        Renderer3D::with_settings(g.clone(), scene.scene_format(), Settings3D::LOW);
    let mut list = RenderList3D::new();
    list.cuboid(
        [0.0; 3],
        IDENTITY_ROT,
        [0.65, 0.65, 0.1],
        &Material::new([1.0; 3]).rough(1.0),
    );
    for radiance in [0.25_f32, 2.0, 8.0, 1e8] {
        let mut light = lighting(radiance.min(1e4));
        if radiance > 1e4 {
            light.sky = [1e4; 3];
            light.ground = [1e4; 3];
        }
        light.exposure = 0.125;
        light.tonemap = true;
        let expected = [radiance.min(65504.0); 3];
        draw(
            &mut scene,
            &view,
            SIZE,
            &camera(),
            &light,
            &mut [ImportedBatch::Static(&mut model)],
        )
        .unwrap();
        uniform_foreground(&hdr_pixels(&g, &scene), expected, "static scene radiance");
        capture(
            &g,
            &scene,
            &texture,
            &light,
            &format!("static-radiance-{radiance}"),
        );
        draw(
            &mut scene,
            &view,
            SIZE,
            &camera(),
            &light,
            &mut [ImportedBatch::Procedural {
                renderer: &mut procedural,
                list: &list,
            }],
        )
        .unwrap();
        uniform_foreground(
            &hdr_pixels(&g, &scene),
            expected,
            "procedural scene radiance",
        );
        capture(
            &g,
            &scene,
            &texture,
            &light,
            &format!("procedural-radiance-{radiance}"),
        );
    }
}

#[test]
fn aligned_view_and_light_keep_procedural_fresnel_radiance_finite() {
    let (_lock, g) = gpu();
    let texture = target(&g, FORMATS[0], SIZE);
    let view = g.create_texture_view(&texture, None);
    let mut scene = scene(&g, FORMATS[0]);
    let mut renderer = Renderer3D::with_settings(g.clone(), scene.scene_format(), Settings3D::LOW);
    let mut list = RenderList3D::new();
    list.cuboid(
        [0.0; 3],
        IDENTITY_ROT,
        [0.65, 0.65, 0.1],
        &Material::new([1.0; 3]).rough(0.04).metal(0.4),
    );
    // Nearly parallel normalized vectors can round dot(h, v) above one.
    // The first direction comes from an f32 witness yielding 1.000000119;
    // pow(1 - cosine, fractional-lowered exponent) must never see a negative base.
    for direction in [
        [0.006_841_496_6, 0.026_392_974, -0.999_628_3],
        [0.0, 0.0, -1.0],
        [0.01, 0.03, -1.0],
        [0.000_01, 0.000_01, -1.0],
    ] {
        let center = [0.0, 0.0, -0.1];
        let eye = std::array::from_fn(|i| center[i] + direction[i] * 5.0);
        let camera = Camera3D::orthographic(eye, center, 1.0);
        let mut light = lighting(0.0);
        light.direction = direction.map(|v| -v);
        light.intensity = 8.0;
        light.color = [1.0, 0.7, 0.2];
        light.exposure = 0.75;
        light.tonemap = true;
        for (roughness, metallic) in [(0.04, 0.4), (0.4, 1.0), (1.0, 0.0)] {
            list.boxes[0].material = [roughness, metallic, 0.0, 0.0];
            draw(
                &mut scene,
                &view,
                SIZE,
                &camera,
                &light,
                &mut [ImportedBatch::Procedural {
                    renderer: &mut renderer,
                    list: &list,
                }],
            )
            .unwrap();
            let pixels = hdr_pixels(&g, &scene);
            assert!(
                pixels
                    .iter()
                    .flatten()
                    .all(|v| v.is_finite() && *v >= 0.0 && *v <= 65504.0)
            );
            let center = pixels[((SIZE.1 / 2) * SIZE.0 + SIZE.0 / 2) as usize];
            assert!(
                center[0] > 1.0 && center[1] > 1.0,
                "aligned highlight must retain real radiance: {direction:?}, {center:?}"
            );
            assert!(pixels.iter().filter(|p| p[0] > 1.0).count() > 100);
        }
    }
}

#[cfg(feature = "animation")]
#[test]
fn genuine_gpu_skinned_vertices_preserve_radiance_at_rest_and_animated_pose() {
    let (_lock, g) = gpu();
    let texture = target(&g, FORMATS[0], SIZE);
    let view = g.create_texture_view(&texture, None);
    let mut scene = scene(&g, FORMATS[0]);
    let asset = animated();
    let poses = [
        asset.rest_pose().unwrap(),
        asset.sample_clip(0, 1.0).unwrap(),
    ];
    let mut model = SkinnedModelRenderer::new(g.clone(), scene.scene_format(), asset).unwrap();
    let camera = Camera3D::orthographic([0.0, 1.5, 5.0], [0.0, 1.5, 0.0], 2.0);
    let mut silhouettes = Vec::new();
    for pose in &poses {
        let instances = [SkinnedInstance::new(pose)];
        for radiance in [0.25_f32, 2.0, 8.0, 1e8] {
            let mut light = lighting(radiance.min(1e4));
            if radiance > 1e4 {
                light.sky = [1e4; 3];
                light.ground = [1e4; 3];
            }
            light.exposure = 0.125;
            light.tonemap = true;
            draw(
                &mut scene,
                &view,
                SIZE,
                &camera,
                &light,
                &mut [ImportedBatch::Skinned {
                    renderer: &mut model,
                    instances: &instances,
                }],
            )
            .unwrap();
            let pixels = hdr_pixels(&g, &scene);
            uniform_foreground(
                &pixels,
                [radiance.min(65504.0); 3],
                "skinned scene radiance",
            );
            if radiance == 8.0 {
                silhouettes.push(pixels.iter().map(|p| p[0] > 0.0).collect::<Vec<_>>());
            }
        }
    }
    assert_ne!(
        silhouettes[0], silhouettes[1],
        "test must exercise actual animated GPU deformation"
    );
}

#[test]
fn display_pass_matches_independent_exposure_aces_srgb_oracle_in_both_formats() {
    let (_lock, g) = gpu();
    let mut format_images = Vec::new();
    for format in FORMATS {
        let texture = target(&g, format, SIZE);
        let view = g.create_texture_view(&texture, None);
        let mut scene = scene(&g, format);
        let mut model = ModelRenderer::new(g.clone(), scene.scene_format(), panel(0.65)).unwrap();
        let mut images = Vec::new();
        // Both sides of the linear/sRGB breakpoint, mid-gray, and true HDR.
        for radiance in [0.001, 0.003, 0.003_130_8, 0.0033, 0.25, 2.0, 8.0] {
            for exposure in [0.0, 0.25, 1.0, 2.0] {
                for tonemap in [false, true] {
                    let mut light = lighting(radiance);
                    light.sky = [1.0, 0.5, 0.125];
                    light.ground = light.sky;
                    light.exposure = exposure;
                    light.tonemap = tonemap;
                    draw(
                        &mut scene,
                        &view,
                        SIZE,
                        &camera(),
                        &light,
                        &mut [ImportedBatch::Static(&mut model)],
                    )
                    .unwrap();
                    let hdr = hdr_pixels(&g, &scene);
                    let pixels = g.read_texture(&texture);
                    let expected_radiance = [radiance, radiance * 0.5, radiance * 0.125];
                    let expected_display = display(expected_radiance, &light);
                    let mut checked = 0;
                    for (hdr, pixel) in hdr.iter().zip(pixels.chunks_exact(4)) {
                        if hdr[0] > 0.0 {
                            near_hdr(*hdr, expected_radiance, "pre-display scene");
                            near_display(pixel, expected_display, "independent display oracle");
                            checked += 1;
                        } else {
                            near_display(pixel, [0, 0, 0, 255], "black border");
                        }
                    }
                    assert!(checked > 100);
                    images.push(pixels);
                }
            }
        }
        format_images.push(images);
    }
    for (linear, srgb) in format_images[0].iter().zip(&format_images[1]) {
        near_images(linear, srgb, "UNORM and sRGB display equivalence");
    }
}

#[test]
fn debug_lines_keep_hdr_color_until_the_display_pass() {
    let (_lock, g) = gpu();
    let texture = target(&g, FORMATS[0], SIZE);
    let view = g.create_texture_view(&texture, None);
    let mut scene = scene(&g, FORMATS[0]);
    let mut renderer = Renderer3D::with_settings(g.clone(), scene.scene_format(), Settings3D::LOW);
    let mut list = RenderList3D::new();
    list.line(
        [-0.7, 0.0, 0.0],
        [0.7, 0.0, 0.0],
        7.0,
        [8.0, 2.0, 0.25, 1.0],
    );
    let mut light = lighting(0.0);
    light.exposure = 0.25;
    light.tonemap = true;
    draw(
        &mut scene,
        &view,
        SIZE,
        &camera(),
        &light,
        &mut [ImportedBatch::Procedural {
            renderer: &mut renderer,
            list: &list,
        }],
    )
    .unwrap();
    let hdr = hdr_pixels(&g, &scene);
    uniform_foreground(&hdr, [8.0, 2.0, 0.25], "HDR debug line");
    let pixels = g.read_texture(&texture);
    let expected = display([8.0, 2.0, 0.25], &light);
    for (hdr, pixel) in hdr.iter().zip(pixels.chunks_exact(4)) {
        if hdr[0] > 0.0 {
            near_display(
                pixel,
                expected,
                "line exposure and tone mapping exactly once",
            );
        }
    }
}

#[test]
fn bloom_has_a_bounded_above_threshold_halo_and_zero_below_threshold_or_strength() {
    let (_lock, g) = gpu();
    for format in FORMATS {
        let texture = target(&g, format, SIZE);
        let view = g.create_texture_view(&texture, None);
        let mut scene = scene(&g, format);
        scene.post_process.threshold = 1.0;
        scene.post_process.strength = 0.5;
        scene.post_process.radius = 4;
        scene.post_process.iterations = 2;
        let mut model = ModelRenderer::new(g.clone(), scene.scene_format(), panel(0.08)).unwrap();
        for radiance in [0.0, 0.25, 1.0, 8.0] {
            let mut light = lighting(radiance);
            light.exposure = 0.25;
            light.tonemap = true;
            scene.post_process.bloom = false;
            draw(
                &mut scene,
                &view,
                SIZE,
                &camera(),
                &light,
                &mut [ImportedBatch::Static(&mut model)],
            )
            .unwrap();
            let off = g.read_texture(&texture);
            let original_hdr = hdr_bytes(&g, &scene);
            if radiance == 8.0 {
                capture(&g, &scene, &texture, &light, "bloom-off-radiance-8");
            }
            scene.post_process.bloom = true;
            draw(
                &mut scene,
                &view,
                SIZE,
                &camera(),
                &light,
                &mut [ImportedBatch::Static(&mut model)],
            )
            .unwrap();
            let on = g.read_texture(&texture);
            if radiance == 8.0 {
                capture(&g, &scene, &texture, &light, "bloom-on-radiance-8");
            }
            assert_eq!(
                hdr_bytes(&g, &scene),
                original_hdr,
                "bloom cannot rewrite scene radiance"
            );
            if radiance <= scene.post_process.threshold {
                assert_eq!(
                    on, off,
                    "at/below-threshold radiance must contribute zero bloom"
                );
            } else {
                let halo = off
                    .chunks_exact(4)
                    .zip(on.chunks_exact(4))
                    .filter(|(a, b)| a[..3] == [0, 0, 0] && b[0] > 2)
                    .count();
                assert!(
                    halo > 20,
                    "above-threshold scene must light neighboring background, got {halo} pixels"
                );
                let far_corner = &on[..4];
                near_display(far_corner, [0, 0, 0, 255], "bloom has bounded support");
                scene.post_process.strength = 0.0;
                draw(
                    &mut scene,
                    &view,
                    SIZE,
                    &camera(),
                    &light,
                    &mut [ImportedBatch::Static(&mut model)],
                )
                .unwrap();
                assert_eq!(
                    g.read_texture(&texture),
                    off,
                    "zero strength is pixel-identical to bloom off"
                );
                scene.post_process.strength = 0.5;
            }
        }
    }
}

#[test]
fn maximum_scene_plus_maximum_bloom_and_exposure_has_finite_display_output() {
    let (_lock, g) = gpu();
    for format in FORMATS {
        let texture = target(&g, format, SIZE);
        let view = g.create_texture_view(&texture, None);
        let mut scene = scene(&g, format);
        scene.clear = [65504.0, 65504.0, 65504.0, 1.0];
        scene.post_process.bloom = true;
        scene.post_process.threshold = 0.0;
        scene.post_process.strength = 8.0;
        scene.post_process.radius = 8;
        scene.post_process.iterations = 4;
        let mut light = lighting(0.0);
        for (exposure, tonemap) in [(1e4, true), (1e4, false), (0.0, true)] {
            light.exposure = exposure;
            light.tonemap = tonemap;
            draw(&mut scene, &view, SIZE, &camera(), &light, &mut []).unwrap();
            let expected = display([65504.0 * 9.0; 3], &light);
            for pixel in g.read_texture(&texture).chunks_exact(4) {
                near_display(pixel, expected, "maximum scene + bloom + exposure");
            }
            for pixel in hdr_pixels(&g, &scene) {
                near_hdr(pixel, [65504.0; 3], "finite maximum HDR source");
            }
        }
    }
}

#[test]
fn default_off_preserves_legacy_standalone_pixels_without_hdr_allocations() {
    let (_lock, g) = gpu();
    for format in FORMATS {
        let texture = target(&g, format, SIZE);
        let view = g.create_texture_view(&texture, None);
        let mut scene = ImportedSceneRenderer::new(g.clone(), format).unwrap();
        scene.clear = [0.0, 0.0, 0.0, 1.0];
        assert_eq!(scene.post_process, PostProcessSettings::default());
        assert!(!scene.post_process.enabled);
        assert_eq!(scene.scene_format(), format);
        let mut model = ModelRenderer::new(g.clone(), format, panel(0.65)).unwrap();
        model.clear = scene.clear;
        let mut renderer = Renderer3D::with_settings(g.clone(), format, Settings3D::LOW);
        renderer.clear = scene.clear;
        let mut list = RenderList3D::new();
        list.cuboid(
            [0.0; 3],
            IDENTITY_ROT,
            [0.65, 0.65, 0.1],
            &Material::new([0.1, 0.35, 0.7]).rough(1.0),
        );
        for tonemap in [false, true] {
            let mut light = lighting(0.7);
            light.exposure = 0.65;
            light.tonemap = tonemap;
            model.draw(&view, SIZE, &camera(), &light).unwrap();
            let baseline = g.read_texture(&texture);
            draw(
                &mut scene,
                &view,
                SIZE,
                &camera(),
                &light,
                &mut [ImportedBatch::Static(&mut model)],
            )
            .unwrap();
            assert_eq!(
                g.read_texture(&texture),
                baseline,
                "default-off static regression"
            );
            list.lighting = light;
            renderer.draw(&view, SIZE, &list, &camera());
            let baseline = g.read_texture(&texture);
            draw(
                &mut scene,
                &view,
                SIZE,
                &camera(),
                &light,
                &mut [ImportedBatch::Procedural {
                    renderer: &mut renderer,
                    list: &list,
                }],
            )
            .unwrap();
            assert_eq!(
                g.read_texture(&texture),
                baseline,
                "default-off procedural regression"
            );
        }
        #[cfg(feature = "animation")]
        {
            let asset = animated();
            let pose = asset.sample_clip(0, 1.0).unwrap();
            let mut skinned = SkinnedModelRenderer::new(g.clone(), format, asset).unwrap();
            skinned.clear = scene.clear;
            let instances = [SkinnedInstance::new(&pose)];
            let camera = Camera3D::orthographic([0.0, 1.5, 5.0], [0.0, 1.5, 0.0], 2.0);
            let mut light = lighting(0.7);
            light.exposure = 0.65;
            light.tonemap = true;
            skinned
                .draw(&view, SIZE, &camera, &light, &instances)
                .unwrap();
            let baseline = g.read_texture(&texture);
            draw(
                &mut scene,
                &view,
                SIZE,
                &camera,
                &light,
                &mut [ImportedBatch::Skinned {
                    renderer: &mut skinned,
                    instances: &instances,
                }],
            )
            .unwrap();
            assert_eq!(
                g.read_texture(&texture),
                baseline,
                "default-off skinned regression"
            );
        }
        assert!(scene.hdr_scene_texture().is_none());
        assert_eq!(scene.post_process_size(), None);
        assert_eq!(scene.post_process_generation(), 0);
        assert_eq!(scene.post_process_allocated_bytes(), 0);
    }
}

#[derive(Debug, PartialEq)]
struct Snapshot {
    pixels: Vec<u8>,
    hdr: Vec<u8>,
    size: Option<(u32, u32)>,
    generation: u64,
    allocated_bytes: u64,
    depth_size: Option<(u32, u32)>,
    depth_generation: u64,
}

fn snapshot(g: &Wgpu, scene: &ImportedSceneRenderer<Wgpu>, target: &Texture) -> Snapshot {
    Snapshot {
        pixels: g.read_texture(target),
        hdr: hdr_bytes(g, scene),
        size: scene.post_process_size(),
        generation: scene.post_process_generation(),
        allocated_bytes: scene.post_process_allocated_bytes(),
        depth_size: scene.depth_size(),
        depth_generation: scene.depth_generation(),
    }
}

#[test]
fn invalid_late_batch_target_settings_and_budget_leave_accepted_hdr_frame_untouched() {
    let (_lock, g) = gpu();
    let texture = target(&g, FORMATS[0], SIZE);
    let view = g.create_texture_view(&texture, None);
    let mut scene = scene(&g, FORMATS[0]);
    scene.post_process.bloom = true;
    let valid_settings = scene.post_process;
    let light = lighting(2.0);
    let mut model = ModelRenderer::new(g.clone(), scene.scene_format(), panel(0.65)).unwrap();
    let mut wrong_format = ModelRenderer::new(g.clone(), FORMATS[0], panel(0.2)).unwrap();
    let mut renderer = Renderer3D::with_settings(g.clone(), scene.scene_format(), Settings3D::LOW);
    let mut list = RenderList3D::new();
    list.cuboid(
        [0.0; 3],
        IDENTITY_ROT,
        [0.4; 3],
        &Material::new([0.2, 0.5, 1.0]).rough(1.0),
    );
    draw(
        &mut scene,
        &view,
        SIZE,
        &camera(),
        &light,
        &mut [ImportedBatch::Procedural {
            renderer: &mut renderer,
            list: &list,
        }],
    )
    .unwrap();
    let before = snapshot(&g, &scene, &texture);
    let stats = renderer.last_frame_stats();
    let resized = (SIZE.0 + 4, SIZE.1 + 6);
    let invalid_instances = [StaticInstance {
        rotation: [0.0; 4],
        ..Default::default()
    }];
    assert!(
        draw(
            &mut scene,
            &view,
            resized,
            &camera(),
            &light,
            &mut [
                ImportedBatch::Procedural {
                    renderer: &mut renderer,
                    list: &list
                },
                ImportedBatch::StaticInstances {
                    renderer: &mut model,
                    instances: &invalid_instances
                },
            ]
        )
        .is_err()
    );
    assert_eq!(snapshot(&g, &scene, &texture), before);
    assert_eq!(renderer.last_frame_stats(), stats);
    assert!(
        draw(
            &mut scene,
            &view,
            resized,
            &camera(),
            &light,
            &mut [
                ImportedBatch::Static(&mut model),
                ImportedBatch::Static(&mut wrong_format),
            ]
        )
        .is_err()
    );
    assert_eq!(snapshot(&g, &scene, &texture), before);

    for (size, format, sample_count) in [
        ((0, 67), FORMATS[0], 1),
        ((65, 0), FORMATS[0], 1),
        ((8193, 67), FORMATS[0], 1),
        (SIZE, FORMATS[1], 1),
        (SIZE, TextureFormat::Rgba16Float, 1),
        (SIZE, FORMATS[0], 4),
    ] {
        assert!(
            scene
                .draw(
                    ImportedSceneTarget {
                        view: &view,
                        size,
                        format,
                        sample_count
                    },
                    &camera(),
                    &light,
                    &PointLightSettings::default(),
                    &mut []
                )
                .is_err()
        );
        assert_eq!(snapshot(&g, &scene, &texture), before);
    }
    for invalid in [
        PostProcessSettings {
            threshold: f32::NAN,
            ..valid_settings
        },
        PostProcessSettings {
            threshold: -0.1,
            ..valid_settings
        },
        PostProcessSettings {
            threshold: f32::INFINITY,
            ..valid_settings
        },
        PostProcessSettings {
            threshold: 65505.0,
            ..valid_settings
        },
        PostProcessSettings {
            strength: f32::NAN,
            ..valid_settings
        },
        PostProcessSettings {
            strength: -0.1,
            ..valid_settings
        },
        PostProcessSettings {
            strength: f32::INFINITY,
            ..valid_settings
        },
        PostProcessSettings {
            strength: 8.01,
            ..valid_settings
        },
        PostProcessSettings {
            radius: 0,
            ..valid_settings
        },
        PostProcessSettings {
            radius: 9,
            ..valid_settings
        },
        PostProcessSettings {
            iterations: 0,
            ..valid_settings
        },
        PostProcessSettings {
            iterations: 5,
            ..valid_settings
        },
        PostProcessSettings {
            enabled: false,
            threshold: f32::NAN,
            ..valid_settings
        },
    ] {
        scene.post_process = invalid;
        assert!(
            scene
                .preflight(
                    resized,
                    &camera(),
                    &light,
                    &PointLightSettings::default(),
                    &list,
                    &[]
                )
                .is_err()
        );
        assert!(draw(&mut scene, &view, resized, &camera(), &light, &mut []).is_err());
        assert_eq!(snapshot(&g, &scene, &texture), before);
    }
    scene.post_process = valid_settings;
    assert!(matches!(
        draw(&mut scene, &view, (4096, 4096), &camera(), &light, &mut []),
        Err(ImportedSceneError::PostProcessBudget)
    ));
    assert_eq!(snapshot(&g, &scene, &texture), before);
    for invalid_clear in [
        [f64::NAN, 0.0, 0.0, 1.0],
        [-0.1, 0.0, 0.0, 1.0],
        [65505.0, 0.0, 0.0, 1.0],
    ] {
        scene.clear = invalid_clear;
        assert!(draw(&mut scene, &view, resized, &camera(), &light, &mut []).is_err());
        assert_eq!(snapshot(&g, &scene, &texture), before);
    }
    scene.clear = [0.0, 0.0, 0.0, 1.0];
    let invalid_point = PointLightSettings {
        point_light: Some(PointLight {
            position: [0.0; 3],
            color: [1.0; 3],
            intensity: f32::NAN,
            range: 1.0,
        }),
    };
    assert!(
        scene
            .draw(
                ImportedSceneTarget {
                    view: &view,
                    size: resized,
                    format: FORMATS[0],
                    sample_count: 1
                },
                &camera(),
                &light,
                &invalid_point,
                &mut []
            )
            .is_err()
    );
    assert_eq!(snapshot(&g, &scene, &texture), before);
    draw(
        &mut scene,
        &view,
        SIZE,
        &camera(),
        &light,
        &mut [ImportedBatch::Procedural {
            renderer: &mut renderer,
            list: &list,
        }],
    )
    .unwrap();
    assert_eq!(
        snapshot(&g, &scene, &texture),
        before,
        "failed frames must not poison recovery"
    );
}

#[cfg(feature = "animation")]
#[test]
fn invalid_late_skinned_pose_preserves_submitted_bounds_and_all_hdr_targets() {
    let (_lock, g) = gpu();
    let texture = target(&g, FORMATS[0], SIZE);
    let view = g.create_texture_view(&texture, None);
    let mut scene = scene(&g, FORMATS[0]);
    scene.post_process.bloom = true;
    let asset = animated();
    let rest = asset.rest_pose().unwrap();
    let bent = asset.sample_clip(0, 1.0).unwrap();
    let foreign = animated().rest_pose().unwrap();
    let mut first =
        SkinnedModelRenderer::new(g.clone(), scene.scene_format(), asset.clone()).unwrap();
    let mut second = SkinnedModelRenderer::new(g.clone(), scene.scene_format(), asset).unwrap();
    let camera = Camera3D::orthographic([0.0, 1.5, 5.0], [0.0, 1.5, 0.0], 2.0);
    let mut left = IDENTITY;
    left[3][0] = -0.5;
    let mut right = IDENTITY;
    right[3][0] = 0.5;
    let initial_left = [SkinnedInstance {
        pose: &rest,
        transform: left,
    }];
    let initial_right = [SkinnedInstance {
        pose: &rest,
        transform: right,
    }];
    let light = lighting(2.0);
    draw(
        &mut scene,
        &view,
        SIZE,
        &camera,
        &light,
        &mut [
            ImportedBatch::Skinned {
                renderer: &mut first,
                instances: &initial_left,
            },
            ImportedBatch::Skinned {
                renderer: &mut second,
                instances: &initial_right,
            },
        ],
    )
    .unwrap();
    let before = snapshot(&g, &scene, &texture);
    let bounds = (first.bounds().to_vec(), second.bounds().to_vec());
    let changed = [SkinnedInstance {
        pose: &bent,
        transform: right,
    }];
    let wrong_pose = [SkinnedInstance {
        pose: &foreign,
        transform: right,
    }];
    let mut singular = right;
    singular[0][0] = 0.0;
    let wrong_transform = [SkinnedInstance {
        pose: &rest,
        transform: singular,
    }];
    for invalid in [&wrong_pose[..], &wrong_transform[..]] {
        assert!(
            draw(
                &mut scene,
                &view,
                (69, 73),
                &camera,
                &light,
                &mut [
                    ImportedBatch::Skinned {
                        renderer: &mut first,
                        instances: &changed
                    },
                    ImportedBatch::Skinned {
                        renderer: &mut second,
                        instances: invalid
                    },
                ]
            )
            .is_err()
        );
        assert_eq!(first.bounds(), bounds.0);
        assert_eq!(second.bounds(), bounds.1);
        assert_eq!(snapshot(&g, &scene, &texture), before);
    }
    draw(
        &mut scene,
        &view,
        SIZE,
        &camera,
        &light,
        &mut [
            ImportedBatch::Skinned {
                renderer: &mut first,
                instances: &initial_left,
            },
            ImportedBatch::Skinned {
                renderer: &mut second,
                instances: &initial_right,
            },
        ],
    )
    .unwrap();
    assert_eq!(snapshot(&g, &scene, &texture), before);
    draw(
        &mut scene,
        &view,
        SIZE,
        &camera,
        &light,
        &mut [
            ImportedBatch::Skinned {
                renderer: &mut first,
                instances: &[],
            },
            ImportedBatch::Skinned {
                renderer: &mut second,
                instances: &[],
            },
        ],
    )
    .unwrap();
    assert!(first.bounds().is_empty());
    assert!(second.bounds().is_empty());
    assert!(
        g.read_texture(&texture)
            .chunks_exact(4)
            .all(|p| p == [0, 0, 0, 255])
    );
}

#[test]
fn odd_resizes_empty_frames_and_background_display_never_reuse_stale_hdr_or_bloom() {
    let (_lock, g) = gpu();
    for format in FORMATS {
        let mut scene = scene(&g, format);
        scene.post_process.bloom = true;
        let mut model = ModelRenderer::new(g.clone(), scene.scene_format(), panel(0.65)).unwrap();
        for (index, size) in [SIZE, (81, 55), (1, 1), SIZE].into_iter().enumerate() {
            let texture = target(&g, format, size);
            let view = g.create_texture_view(&texture, None);
            let light = lighting(8.0);
            draw(
                &mut scene,
                &view,
                size,
                &camera(),
                &light,
                &mut [ImportedBatch::Static(&mut model)],
            )
            .unwrap();
            assert_eq!(scene.post_process_size(), Some(size));
            assert_eq!(scene.post_process_generation(), index as u64 + 1);
            assert_eq!(scene.depth_size(), Some(size));
            let allocated = scene.post_process_allocated_bytes();
            assert!(allocated > 0);
            draw(&mut scene, &view, size, &camera(), &light, &mut []).unwrap();
            assert!(
                g.read_texture(&texture)
                    .chunks_exact(4)
                    .all(|p| p == [0, 0, 0, 255]),
                "empty frame must erase previous bloom"
            );
            assert!(
                hdr_pixels(&g, &scene)
                    .iter()
                    .all(|p| *p == [0.0, 0.0, 0.0, 1.0])
            );
            assert_eq!(scene.post_process_generation(), index as u64 + 1);
            assert_eq!(scene.post_process_allocated_bytes(), allocated);
            scene.post_process.bloom = false;
            scene.clear = [0.002, 0.25, 2.0, 1.0];
            let mut light = lighting(0.0);
            light.exposure = 0.7;
            light.tonemap = true;
            draw(&mut scene, &view, size, &camera(), &light, &mut []).unwrap();
            for pixel in g.read_texture(&texture).chunks_exact(4) {
                near_display(
                    pixel,
                    display([0.002, 0.25, 2.0], &light),
                    "background obeys display pass",
                );
            }
            for pixel in hdr_pixels(&g, &scene) {
                near_hdr(pixel, [0.002, 0.25, 2.0], "scene-linear clear");
            }
            scene.clear = [0.0, 0.0, 0.0, 1.0];
            scene.post_process.bloom = true;
        }
    }
}
