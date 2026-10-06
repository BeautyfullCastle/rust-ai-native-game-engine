//! Actual importer -> standalone cook/load -> RHI upload -> indexed GPU readback.
//! ORR_REQUIRE_GPU=1 makes adapter absence a failure, never an ignored acceptance.
#![cfg(feature = "models")]
#![allow(clippy::float_arithmetic)]
use orr_model::{StaticModel, Wrap};
use orr_render::orr_rhi::{Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions};
use orr_render::{math3::Mat4, Camera3D, Lighting, ModelRenderError, ModelRenderer};
const SIZE: u32 = 128;
fn gpu() -> Option<Wgpu> {
    match Wgpu::headless(WgpuOptions::default()) {
        Ok(g) => {
            eprintln!(
                "model acceptance adapter: {} (software: {})",
                g.adapter_name(),
                g.is_software()
            );
            Some(g)
        }
        Err(e) => {
            assert!(
                std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "required GPU unavailable: {e}"
            );
            eprintln!("SKIP: {e}");
            None
        }
    }
}
fn model() -> StaticModel {
    let source = orr_model::import::import_with_resolver(
        "fixtures/model.glb",
        include_bytes!("../../orr_model/tests/fixtures/model.glb"),
        |_| panic!("embedded GLB has no external dependencies"),
    )
    .unwrap();
    StaticModel::from_bytes(&source.to_bytes().unwrap()).unwrap()
}
fn camera() -> Camera3D {
    Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 2.0)
}
fn lighting() -> Lighting {
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
fn target(g: &Wgpu, format: TextureFormat, size: (u32, u32)) -> <Wgpu as Rhi>::Texture {
    g.create_texture(&TextureDesc {
        label: "model acceptance",
        width: size.0,
        height: size.1,
        format,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    })
}
fn at_world(pixels: &[u8], world: [f32; 3], camera: &Camera3D) -> [u8; 4] {
    let screen = camera.world_to_screen(world, (SIZE, SIZE)).unwrap();
    let x = screen[0] as usize;
    let y = screen[1] as usize;
    pixels[(y * SIZE as usize + x) * 4..(y * SIZE as usize + x) * 4 + 4]
        .try_into()
        .unwrap()
}
fn world(m: &StaticModel, local: [f32; 3]) -> [f32; 3] {
    let v = Mat4(m.source().primitives[0].transform).transform_point4(local);
    [v[0], v[1], v[2]]
}
fn near(a: [u8; 4], b: [u8; 4]) {
    assert!(
        a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 3),
        "actual {a:?}, expected {b:?}"
    );
}
#[test]
fn imported_uv_material_slots_child_transform_depth_camera_and_rejection_readback() {
    let Some(g) = gpu() else { return };
    let m = model();
    let cam = camera();
    let light = lighting();
    for format in [TextureFormat::Rgba8Unorm, TextureFormat::Rgba8UnormSrgb] {
        let t = target(&g, format, (SIZE, SIZE));
        let view = g.create_texture_view(&t, None);
        let mut renderer = ModelRenderer::new(g.clone(), format, m.clone()).unwrap();
        renderer.clear = [0.0, 0.0, 0.0, 1.0];
        renderer.draw(&view, (SIZE, SIZE), &cam, &light).unwrap();
        let pixels = g.read_texture(&t);
        for (p, c) in [
            ([-0.5, 0.5, 0.0], [255, 0, 0, 255]),
            ([0.5, 0.25, 0.0], [0, 255, 0, 255]),
            ([-0.5, -0.5, 0.0], [0, 0, 255, 255]),
            ([0.5, -0.5, 0.0], [128, 128, 128, 255]),
            ([0.9, 0.9, 0.0], [0, 0, 0, 255]),
            ([1.43, -0.35, 0.0], [188, 255, 0, 255]),
        ] {
            near(at_world(&pixels, world(&m, p), &cam), c);
        }
        // Save a reviewable screenshot only when explicitly requested by the test runner.
        if let Some(path) = std::env::var_os("ORR_MODEL_SCREENSHOT") {
            if format == TextureFormat::Rgba8UnormSrgb {
                let mut ppm = format!("P6\n{SIZE} {SIZE}\n255\n").into_bytes();
                for pixel in pixels.chunks_exact(4) {
                    ppm.extend_from_slice(&pixel[..3]);
                }
                std::fs::write(path, ppm).unwrap();
            }
        }
        let before = pixels;
        let mut invalid = cam;
        invalid.eye = [f32::NAN; 3];
        assert_eq!(
            renderer.draw(&view, (SIZE, SIZE), &invalid, &light),
            Err(ModelRenderError::InvalidCamera)
        );
        let mut collapsed = Camera3D::orthographic([0.0, 0.0, 1e8], [0.0; 3], 2.0);
        collapsed.up = [0.0, 1e-13, 0.0];
        collapsed.projection = orr_render::Projection::Orthographic {
            half_height: 2.0,
            near: 0.0,
            far: 1e9,
        };
        assert_eq!(
            renderer.draw(&view, (SIZE, SIZE), &collapsed, &light),
            Err(ModelRenderError::InvalidCamera)
        );
        let mut invalid_light = light;
        invalid_light.shadows = true;
        assert_eq!(
            renderer.draw(&view, (SIZE, SIZE), &cam, &invalid_light),
            Err(ModelRenderError::InvalidLighting)
        );
        assert_eq!(
            renderer.draw(&view, (0, SIZE), &cam, &light),
            Err(ModelRenderError::InvalidTarget)
        );
        assert_eq!(before, g.read_texture(&t));
        let moved = Camera3D::orthographic([0.4, 0.2, 5.0], [0.4, 0.2, 0.0], 1.6);
        renderer.draw(&view, (SIZE, SIZE), &moved, &light).unwrap();
        near(
            at_world(&g.read_texture(&t), world(&m, [-0.5, 0.5, 0.0]), &moved),
            [255, 0, 0, 255],
        );
        let wide = target(&g, format, (SIZE * 2, SIZE));
        let wideview = g.create_texture_view(&wide, None);
        renderer
            .draw(&wideview, (SIZE * 2, SIZE), &cam, &light)
            .unwrap();
        assert_eq!(g.read_texture(&wide).len(), (SIZE * SIZE * 2 * 4) as usize);
    }
    // Later-submitted geometry behind the front surface must fail depth testing.
    let mut raw = m.source().clone();
    let mut behind = raw.primitives[0].clone();
    behind.id.push_str("/behind");
    behind.material = 1;
    behind.transform[3][2] -= 0.5;
    raw.primitives.push(behind);
    let t = target(&g, TextureFormat::Rgba8Unorm, (SIZE, SIZE));
    let view = g.create_texture_view(&t, None);
    let mut renderer = ModelRenderer::new(
        g.clone(),
        TextureFormat::Rgba8Unorm,
        StaticModel::new(raw).unwrap(),
    )
    .unwrap();
    renderer.draw(&view, (SIZE, SIZE), &cam, &light).unwrap();
    near(
        at_world(&g.read_texture(&t), world(&m, [-0.5, 0.5, 0.0]), &cam),
        [255, 0, 0, 255],
    );
    // Mirroring transforms reverses geometric winding; renderer repairs indices.
    let mut raw = m.source().clone();
    for p in &mut raw.primitives {
        for r in 0..3 {
            p.transform[0][r] *= -1.0;
        }
    }
    let mirrored = StaticModel::new(raw).unwrap();
    let sample = world(&mirrored, [-0.5, 0.5, 0.0]);
    let mut renderer = ModelRenderer::new(g.clone(), TextureFormat::Rgba8Unorm, mirrored).unwrap();
    renderer.draw(&view, (SIZE, SIZE), &cam, &light).unwrap();
    near(
        at_world(&g.read_texture(&t), sample, &cam),
        [255, 0, 0, 255],
    );
    // Real inverse-transpose normal under shear/nonuniform scale, directional light.
    let mut raw = m.source().clone();
    for p in &mut raw.primitives {
        p.transform[0][2] = 0.5;
    }
    let tilted = StaticModel::new(raw).unwrap();
    let normal = orr_model::normal_matrix(tilted.source().primitives[0].transform).unwrap();
    let n = orr_render::math3::normalize([normal[2][0], normal[2][1], normal[2][2]]);
    let direct = Lighting {
        direction: [-n[0], -n[1], -n[2]],
        color: [1.0; 3],
        intensity: 1.0,
        ambient: 0.0,
        ..light
    };
    let sample = world(&tilted, [-0.5, 0.5, 0.0]);
    let mut renderer = ModelRenderer::new(g.clone(), TextureFormat::Rgba8Unorm, tilted).unwrap();
    renderer.draw(&view, (SIZE, SIZE), &cam, &direct).unwrap();
    near(
        at_world(&g.read_texture(&t), sample, &cam),
        [255, 0, 0, 255],
    );
}
fn decode(v: u8) -> f32 {
    let c = v as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
fn encode(c: f32) -> u8 {
    let c = if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (c.clamp(0.0, 1.0) * 255.0).round() as u8
}
fn reference(bytes: &[u8], uv: [f32; 2], wrap: Wrap, linear: bool) -> [u8; 4] {
    let address = |p: i32| match wrap {
        Wrap::Clamp => p.clamp(0, 1),
        Wrap::Repeat => p.rem_euclid(2),
        Wrap::Mirror => {
            let p = p.rem_euclid(4);
            if p < 2 {
                p
            } else {
                3 - p
            }
        }
    } as usize;
    let tap = |x, y, c| decode(bytes[(address(y) * 2 + address(x)) * 4 + c]);
    let p = [uv[0] * 2.0, uv[1] * 2.0];
    let mut result = [0, 0, 0, 255];
    for (c, out) in result[..3].iter_mut().enumerate() {
        let color = if !linear {
            tap(p[0].floor() as i32, p[1].floor() as i32, c)
        } else {
            let x = (p[0] - 0.5).floor() as i32;
            let y = (p[1] - 0.5).floor() as i32;
            let tx = p[0] - 0.5 - x as f32;
            let ty = p[1] - 0.5 - y as f32;
            let a = tap(x, y, c) * (1.0 - tx) + tap(x + 1, y, c) * tx;
            let b = tap(x, y + 1, c) * (1.0 - tx) + tap(x + 1, y + 1, c) * tx;
            a * (1.0 - ty) + b * ty
        };
        *out = encode(color);
    }
    result
}
#[test]
fn explicit_nearest_and_bilinear_wrap_seams_match_linear_space_cpu_reference() {
    let Some(g) = gpu() else { return };
    let m = model();
    let cam = camera();
    let t = target(&g, TextureFormat::Rgba8UnormSrgb, (SIZE, SIZE));
    let view = g.create_texture_view(&t, None);
    let sample = world(&m, [-0.4, 0.3, 0.0]);
    for wrap in [Wrap::Clamp, Wrap::Repeat, Wrap::Mirror] {
        for linear in [false, true] {
            for uv in [
                [-0.125, -0.125],
                [-1e-10, -1e-10],
                [0.0, 0.0],
                [0.999, 0.999],
                [1.0, 1.0],
                [1.125, 1.125],
                [-0.625, 0.375],
            ] {
                let mut raw = m.source().clone();
                raw.primitives.truncate(1);
                for v in &mut raw.primitives[0].vertices {
                    v.uv = uv;
                }
                raw.materials[0].wrap_s = wrap;
                raw.materials[0].wrap_t = wrap;
                raw.materials[0].linear_filter = linear;
                let expected = reference(&raw.images[0].rgba8, uv, wrap, linear);
                let mut renderer = ModelRenderer::new(
                    g.clone(),
                    TextureFormat::Rgba8UnormSrgb,
                    StaticModel::new(raw).unwrap(),
                )
                .unwrap();
                renderer
                    .draw(&view, (SIZE, SIZE), &cam, &lighting())
                    .unwrap();
                let actual = at_world(&g.read_texture(&t), sample, &cam);
                eprintln!("wrap {wrap:?} linear {linear} uv {uv:?}: {actual:?} / {expected:?}");
                near(actual, expected);
            }
        }
    }
}

#[test]
fn large_uvs_preserve_half_texel_wrap_seams_at_max_texture_extent() {
    let Some(g) = gpu() else { return };
    let m = model();
    let cam = camera();
    let t = target(&g, TextureFormat::Rgba8UnormSrgb, (SIZE, SIZE));
    let view = g.create_texture_view(&t, None);
    let sample = world(&m, [-0.4, 0.3, 0.0]);
    for (wrap, u, expected) in [
        (Wrap::Repeat, 65536.0, [188, 0, 188, 255]),
        (Wrap::Repeat, -65536.0, [188, 0, 188, 255]),
        (Wrap::Mirror, 65536.0, [255, 0, 0, 255]),
        (Wrap::Mirror, -65536.0, [255, 0, 0, 255]),
        (Wrap::Mirror, 65535.0, [0, 0, 255, 255]),
        (Wrap::Mirror, -65535.0, [0, 0, 255, 255]),
        (Wrap::Repeat, 65535.75, [188, 0, 188, 255]),
        (Wrap::Repeat, -65535.75, [188, 0, 188, 255]),
        (Wrap::Mirror, 65535.75, [188, 0, 188, 255]),
        (Wrap::Mirror, -65535.75, [188, 0, 188, 255]),
    ] {
        let mut raw = m.source().clone();
        raw.primitives.truncate(1);
        raw.images[0].width = 2048;
        raw.images[0].height = 1;
        raw.images[0].rgba8 = [255, 0, 0, 255, 0, 0, 255, 255].repeat(1024);
        raw.images[0].rgba8[..4].copy_from_slice(&[255, 0, 0, 255]);
        raw.images[0].rgba8[2047 * 4..].copy_from_slice(&[0, 0, 255, 255]);
        for v in &mut raw.primitives[0].vertices {
            v.uv = [u, 0.5];
        }
        raw.materials[0].wrap_s = wrap;
        raw.materials[0].linear_filter = true;
        let mut renderer = ModelRenderer::new(
            g.clone(),
            TextureFormat::Rgba8UnormSrgb,
            StaticModel::new(raw).unwrap(),
        )
        .unwrap();
        renderer
            .draw(&view, (SIZE, SIZE), &cam, &lighting())
            .unwrap();
        near(at_world(&g.read_texture(&t), sample, &cam), expected);
    }
}
