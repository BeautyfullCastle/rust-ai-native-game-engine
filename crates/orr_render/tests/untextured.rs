//! Installed source-hidden factor-only import -> cook/reload -> actual renderer proof.
//! Binding persistence is independently tested in orr_model_bindings.
#![cfg(feature = "imported-scene")]
#![allow(clippy::float_arithmetic)]
#[path = "../../orr_model/tests/common/untextured_fixture.rs"]
mod fixture;
use orr_package::{Project, Runtime};
use orr_render::orr_rhi::{Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions};
use orr_render::{Camera3D, Lighting, ModelRenderer};
const SIZE: u32 = 192;
fn gpu() -> Wgpu {
    let g = Wgpu::headless(WgpuOptions::default())
        .expect("explicit factor-only device fixture requires an adapter");
    let a = g.adapter().get_info();
    eprintln!("factor adapter: name={:?} backend={:?} device_type={:?} vendor={} device={} driver={:?} driver_info={:?} software={}",a.name,a.backend,a.device_type,a.vendor,a.device,a.driver,a.driver_info,g.is_software());
    g
}
fn light() -> Lighting {
    Lighting {
        sky: [1.0; 3],
        ground: [1.0; 3],
        ambient: 1.0,
        intensity: 0.0,
        shadows: false,
        tonemap: false,
        exposure: 1.0,
        ..Default::default()
    }
}
fn installed(animated: bool) -> (fixture::Scratch, Project) {
    let temp = fixture::Scratch::new(if animated {
        "gpu-animation"
    } else {
        "gpu-static"
    });
    let source = temp.0.join("source");
    fixture::package(&source, animated);
    let root = temp.0.join("project");
    std::fs::create_dir(&root).unwrap();
    Project::open_for_install(&root, Runtime::content_only().engine_version)
        .unwrap()
        .install(std::slice::from_ref(&source))
        .unwrap();
    std::fs::rename(&source, temp.0.join("hidden-source")).unwrap();
    let mut runtime = Runtime::content_only();
    runtime
        .capabilities
        .insert(if animated { "animation" } else { "models" }.into());
    let project = Project::open(&root, runtime).unwrap();
    project.verify().unwrap();
    (temp, project)
}
fn bytes(project: &Project, asset: &str) -> Vec<u8> {
    project.read_asset("factor-fixture", asset).unwrap()
}
fn save(name: &str, pixels: &[u8]) {
    let root = std::path::PathBuf::from(
        std::env::var_os("ORR_FACTOR_CAPTURE_DIR").expect("explicit capture directory required"),
    );
    assert_eq!(pixels.len(), SIZE as usize * SIZE as usize * 4);
    std::fs::create_dir_all(&root).unwrap();
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(format!("{name}.rgba")))
        .unwrap();
    file.write_all(pixels).unwrap();
    file.sync_all().unwrap();
}
fn target(g: &Wgpu) -> <Wgpu as Rhi>::Texture {
    g.create_texture(&TextureDesc {
        label: "factor acceptance",
        width: SIZE,
        height: SIZE,
        format: TextureFormat::Rgba8Unorm,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    })
}
// Independent f64 standard sRGB transfer. The existing renderer manually encodes
// display RGB when its target is Rgba8Unorm; an sRGB target encodes in hardware.
fn display_byte(linear: f64) -> u8 {
    let encoded = if linear <= 0.0031308 {
        12.92 * linear
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round() as u8
}
fn linear_value(display: u8) -> f64 {
    let encoded = f64::from(display) / 255.0;
    if encoded <= 0.04045 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    }
}
fn color_matches(pixel: &[u8], linear: &[f64; 4]) -> bool {
    pixel.iter().zip(linear).all(|(&actual, &expected)| {
        actual.abs_diff(display_byte(expected)) <= 2
            && (linear_value(actual) - expected).abs() <= 2.0 / 255.0
    })
}
fn colors(pixels: &[u8], two_materials: bool) {
    // Authored linear factors, checked against display encoding and decoded linear values.
    let expected = [[0.25, 0.5, 0.75, 1.0], [1.0, 0.25, 0.5, 1.0]];
    let mut counts = [0_usize; 2];
    let mut clear = 0;
    for pixel in pixels.chunks_exact(4) {
        if pixel == [0, 0, 0, 255] {
            clear += 1;
            continue;
        }
        let index = expected
            .iter()
            .position(|e| color_matches(pixel, e))
            .expect("foreground must preserve the authored linear factor");
        counts[index] += 1;
    }
    assert!(
        counts[0] >= 100 && clear >= 100,
        "nonvacuous first material and background: {counts:?}/{clear}"
    );
    if two_materials {
        assert!(
            counts[1] >= 20,
            "second material must remain visible: {counts:?}"
        );
    }
    eprintln!("factor color controls: counts={counts:?} clear={clear}");
}
#[test]
fn package_fixture_bytes_and_white_reference_preserve_geometry() {
    assert_eq!(
        [0.0, 0.25, 0.5, 0.75, 1.0].map(display_byte),
        [0, 137, 188, 225, 255]
    );
    for factor in [[0.25, 0.5, 0.75, 1.0], [1.0, 0.25, 0.5, 1.0]] {
        let correct = factor.map(display_byte);
        assert!(color_matches(&correct, &factor));
        let missing = factor.map(|v| (v * 255.0).round() as u8);
        assert!(!color_matches(&missing, &factor));
        let doubled = correct.map(|v| display_byte(f64::from(v) / 255.0));
        assert!(!color_matches(&doubled, &factor));
    }

    let (v, b) = fixture::factor(false, false);
    let factor =
        orr_model::import::import_with_resolver("factor.glb", &fixture::glb(&v, &b), |u| {
            fixture::resolve(u, &b)
        })
        .unwrap();
    let (mut v, b) = fixture::factor(false, true);
    fixture::white_textured(&mut v);
    let white = orr_model::import::import_with_resolver("white.glb", &fixture::glb(&v, &b), |u| {
        fixture::resolve(u, &b)
    })
    .unwrap();
    assert_eq!(
        factor.source().primitives.len(),
        white.source().primitives.len()
    );
    for (a, b) in factor
        .source()
        .primitives
        .iter()
        .zip(&white.source().primitives)
    {
        assert_eq!(a.indices, b.indices);
        assert_eq!(a.transform, b.transform);
        assert_eq!(a.vertices.len(), b.vertices.len());
        for (a, b) in a.vertices.iter().zip(&b.vertices) {
            assert_eq!(a.position, b.position);
            assert_eq!(a.normal, b.normal);
        }
    }
}
#[test]
#[ignore = "requires explicit leased GPU capture"]
fn installed_factor_static_matches_white_textured_linear_readback() {
    let g = gpu();
    let (_temp, project) = installed(false);
    let before = project.verify().unwrap();
    let t = target(&g);
    let view = g.create_texture_view(&t, None);
    let cam = Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 2.0);
    let mut results = Vec::new();
    for (name, asset) in [
        ("static-factor", "factor.glb"),
        ("static-white", "white.glb"),
    ] {
        let m = orr_model::import::import_with_resolver(asset, &bytes(&project, asset), |u| {
            Ok(bytes(&project, u))
        })
        .unwrap();
        let m = orr_model::StaticModel::from_bytes(&m.to_bytes().unwrap()).unwrap();
        let mut renderer = ModelRenderer::new(g.clone(), TextureFormat::Rgba8Unorm, m).unwrap();
        renderer.clear = [0.0, 0.0, 0.0, 1.0];
        renderer.draw(&view, (SIZE, SIZE), &cam, &light()).unwrap();
        let pixels = g.read_texture(&t);
        save(name, &pixels);
        colors(&pixels, true);
        results.push(pixels);
    }
    assert_eq!(results[0], results[1]);
    assert_eq!(project.verify().unwrap(), before);
}
#[cfg(feature = "animation")]
#[test]
#[ignore = "requires explicit leased GPU capture"]
fn installed_factor_animation_matches_white_at_three_fixed_poses() {
    use orr_render::{SkinnedInstance, SkinnedModelRenderer};
    let g = gpu();
    let (_temp, project) = installed(true);
    let before = project.verify().unwrap();
    let t = target(&g);
    let view = g.create_texture_view(&t, None);
    let cam = Camera3D::orthographic([0.0, 1.5, 5.0], [0.0, 1.5, 0.0], 3.0);
    let mut pairs = Vec::new();
    for (name, asset) in [
        ("animated-factor", "factor.glb"),
        ("animated-white", "white.glb"),
    ] {
        let m = orr_model::animation_import::import_with_resolver(
            asset,
            &bytes(&project, asset),
            |u| Ok(bytes(&project, u)),
        )
        .unwrap();
        let m = orr_model::animation::AnimatedModel::from_bytes(&m.to_bytes().unwrap()).unwrap();
        let mut renderer =
            SkinnedModelRenderer::new(g.clone(), TextureFormat::Rgba8Unorm, m.clone()).unwrap();
        renderer.clear = [0.0, 0.0, 0.0, 1.0];
        let mut frames = Vec::new();
        for (label, time) in [("start", 0.0), ("midpoint", 0.5), ("key", 1.0)] {
            let pose = m.sample_clip(0, time).unwrap();
            renderer
                .draw(
                    &view,
                    (SIZE, SIZE),
                    &cam,
                    &light(),
                    &[SkinnedInstance::new(&pose)],
                )
                .unwrap();
            let pixels = g.read_texture(&t);
            save(&format!("{name}-{label}"), &pixels);
            colors(&pixels, false);
            frames.push(pixels);
        }
        assert_ne!(frames[0], frames[1]);
        assert_ne!(frames[1], frames[2]);
        pairs.push(frames);
    }
    assert_eq!(pairs[0], pairs[1]);
    assert_eq!(project.verify().unwrap(), before);
}
