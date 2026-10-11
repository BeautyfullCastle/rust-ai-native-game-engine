//! Real import -> standalone cook/load -> original vertex GPU skinning -> readback.
//! With ORR_REQUIRE_GPU=1, adapter absence fails rather than silently skipping.
#![cfg(feature = "animation")]
#![allow(clippy::float_arithmetic)]
use orr_model::{
    animation::{AnimatedModel, ChannelValues, Matrix4},
    animation_import, IDENTITY,
};
use orr_render::orr_rhi::{Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions};
use orr_render::{
    math3::Mat4, Camera3D, Lighting, SkinnedInstance, SkinnedModelRenderer, SkinnedRenderError,
};
const SIZE: u32 = 192;

fn gpu() -> Option<Wgpu> {
    match Wgpu::headless(WgpuOptions::default()) {
        Ok(g) => {
            eprintln!(
                "skinned acceptance adapter: {} (software: {})",
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
fn load(bytes: &[u8], path: &str) -> AnimatedModel {
    let imported = animation_import::import_with_resolver(path, bytes, |_| {
        panic!("fixture embeds all dependencies")
    })
    .unwrap();
    AnimatedModel::from_bytes(&imported.to_bytes().unwrap()).unwrap()
}
fn fixture() -> AnimatedModel {
    load(
        include_bytes!("../../orr_model/tests/fixtures/animated_strip.glb"),
        "fixtures/animated_strip.glb",
    )
}
fn camera() -> Camera3D {
    Camera3D::orthographic([0.0, 1.5, 5.0], [0.0, 1.5, 0.0], 3.0)
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
fn placement(x: f32) -> Matrix4 {
    let mut m = IDENTITY;
    m[3][0] = x;
    m
}
fn target(g: &Wgpu, format: TextureFormat) -> <Wgpu as Rhi>::Texture {
    g.create_texture(&TextureDesc {
        label: "skinned acceptance",
        width: SIZE,
        height: SIZE,
        format,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    })
}
fn near(actual: [u8; 4], expected: [u8; 4], label: &str) {
    assert!(
        actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 3),
        "{label}: actual {actual:?}, expected {expected:?}"
    );
}
fn close(a: [f32; 3], b: [f32; 3]) {
    assert!(
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 2e-5),
        "{a:?} != {b:?}"
    );
}
fn decode(v: u8) -> f32 {
    let v = v as f32 / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
fn encode(v: f32) -> u8 {
    ((if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    })
    .clamp(0.0, 1.0)
        * 255.0)
        .round() as u8
}

/// Analytic reference independent of runtime hierarchy, palette, and deformation
/// helpers: fixture joint 1 rotates around the nonidentity bind pivot (.3,1.7).
/// This detects missing ancestors, inverse-bind errors, or applying mesh x=7 twice.
fn reference_positions(model: &AnimatedModel, angle: f32, placement: Matrix4) -> Vec<[f32; 3]> {
    let (s, c) = angle.sin_cos();
    model.source().primitives[0]
        .vertices
        .iter()
        .map(|v| {
            let p = v.vertex.position;
            let q = [
                0.3 + c * (p[0] - 0.3) - s * (p[1] - 1.7),
                1.7 + s * (p[0] - 0.3) + c * (p[1] - 1.7),
                p[2],
            ];
            let total = v.weights.iter().sum::<f32>();
            let weight = v
                .joints
                .iter()
                .zip(v.weights)
                .filter_map(|(&j, w)| if j == 1 { Some(w / total) } else { None })
                .sum::<f32>();
            let mixed = std::array::from_fn(|i| p[i] * (1.0 - weight) + q[i] * weight);
            let world = Mat4(placement).transform_point4(mixed);
            [world[0], world[1], world[2]]
        })
        .collect()
}
fn barycentric(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> [f32; 3] {
    let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    let u = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / d;
    let v = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / d;
    [u, v, 1.0 - u - v]
}
/// Check hundreds of confident interior pixels against independently skinned
/// triangles and interpolated nearest UVs. Also check the entire clear region
/// beyond a conservative edge margin, rather than accepting any moving pixels.
fn assert_pixels(pixels: &[u8], model: &AnimatedModel, positions: &[[f32; 3]], cam: &Camera3D) {
    let p = &model.source().primitives[0];
    let m = &model.source().materials[p.material as usize];
    let texture = &model.source().images[m.image as usize];
    assert!(!m.linear_filter, "reference fixture uses nearest texels");
    let screen: Vec<_> = positions
        .iter()
        .map(|&v| cam.world_to_screen(v, (SIZE, SIZE)).unwrap())
        .collect();
    let mut checked = 0;
    let mut clear_checked = 0;
    for y in 0..SIZE as usize {
        for x in 0..SIZE as usize {
            let mut near_triangle = false;
            let mut expected = None;
            for indices in p.indices.chunks_exact(3) {
                let ids = [
                    indices[0] as usize,
                    indices[1] as usize,
                    indices[2] as usize,
                ];
                let b = barycentric(
                    [x as f32 + 0.5, y as f32 + 0.5],
                    screen[ids[0]],
                    screen[ids[1]],
                    screen[ids[2]],
                );
                if b.iter().all(|&v| v >= -0.08) {
                    near_triangle = true;
                }
                if b.iter().all(|&v| v > 0.06) {
                    let uv: [f32; 2] = std::array::from_fn(|k| {
                        (0..3).map(|i| b[i] * p.vertices[ids[i]].vertex.uv[k]).sum()
                    });
                    let uv = [uv[0].clamp(0.0, 1.0), uv[1].clamp(0.0, 1.0)];
                    let u = uv[0] * texture.width as f32;
                    let v = uv[1] * texture.height as f32;
                    // Texture discontinuities have their own pixel rounding boundary.
                    if u.fract() < 0.025
                        || u.fract() > 0.975
                        || v.fract() < 0.025
                        || v.fract() > 0.975
                    {
                        continue;
                    }
                    let tx = (u.floor() as usize).min(texture.width as usize - 1);
                    let ty = (v.floor() as usize).min(texture.height as usize - 1);
                    let offset = (ty * texture.width as usize + tx) * 4;
                    let mut color = [0, 0, 0, 255];
                    for (k, value) in color.iter_mut().take(3).enumerate() {
                        *value = encode(decode(texture.rgba8[offset + k]) * m.base_color[k]);
                    }
                    expected = Some(color);
                }
            }
            let at = (y * SIZE as usize + x) * 4;
            let actual = pixels[at..at + 4].try_into().unwrap();
            if let Some(color) = expected {
                near(actual, color, &format!("pixel {x},{y}"));
                checked += 1;
            } else if !near_triangle {
                near(actual, [0, 0, 0, 255], &format!("clear pixel {x},{y}"));
                clear_checked += 1;
            }
        }
    }
    assert!(checked > 200, "insufficient interior coverage: {checked}");
    assert!(
        clear_checked > 20_000,
        "insufficient clear-region coverage: {clear_checked}"
    );
}
fn capture(name: &str, pixels: &[u8]) {
    if let Some(dir) = std::env::var_os("ORR_SKINNED_CAPTURE_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        let path = std::path::PathBuf::from(dir).join(format!("{name}.ppm"));
        let mut bytes = format!("P6\n{SIZE} {SIZE}\n255\n").into_bytes();
        for p in pixels.chunks_exact(4) {
            bytes.extend_from_slice(&p[..3]);
        }
        std::fs::write(path, bytes).unwrap();
    }
}
#[test]
fn gltf_and_glb_cook_reload_rest_midpoint_key_match_analytic_pixels_and_current_bounds() {
    let Some(g) = gpu() else { return };
    let cam = camera();
    for (source, bytes) in [
        (
            "fixtures/animated_strip.gltf",
            include_bytes!("../../orr_model/tests/fixtures/animated_strip.gltf").as_slice(),
        ),
        (
            "fixtures/animated_strip.glb",
            include_bytes!("../../orr_model/tests/fixtures/animated_strip.glb").as_slice(),
        ),
    ] {
        let model = load(bytes, source);
        assert_eq!(model.source().primitives.len(), 1);
        let p = &model.source().primitives[0];
        assert_eq!(
            model.source().skins[p.skin.unwrap() as usize].joints.len(),
            2
        );
        assert_eq!(
            model.source().nodes[p.node as usize].rest.translation[0],
            7.0
        );
        for format in [TextureFormat::Rgba8Unorm, TextureFormat::Rgba8UnormSrgb] {
            let t = target(&g, format);
            let view = g.create_texture_view(&t, None);
            let mut renderer = SkinnedModelRenderer::new(g.clone(), format, model.clone()).unwrap();
            renderer.clear = [0.0, 0.0, 0.0, 1.0];
            let rest = model.rest_pose().unwrap();
            let mid = model.sample_clip(0, 0.5).unwrap();
            let key = model.sample_clip(0, 1.0).unwrap();
            let mut previous = None;
            for (name, pose, angle) in [
                ("rest", &rest, 0.0),
                ("midpoint", &mid, std::f32::consts::PI / 6.0),
                ("key", &key, std::f32::consts::PI / 3.0),
            ] {
                let expected = reference_positions(&model, angle, IDENTITY);
                let oracle = model.deform(pose).unwrap();
                for (actual, expected) in oracle[0].vertices.iter().zip(&expected) {
                    close(actual.position, *expected);
                }
                renderer
                    .draw(
                        &view,
                        (SIZE, SIZE),
                        &cam,
                        &light(),
                        &[SkinnedInstance::new(pose)],
                    )
                    .unwrap();
                let pixels = g.read_texture(&t);
                assert_pixels(&pixels, &model, &expected, &cam);
                let bounds = renderer.bounds()[0];
                for r in 0..3 {
                    let min = expected.iter().map(|p| p[r]).fold(f32::INFINITY, f32::min);
                    let max = expected
                        .iter()
                        .map(|p| p[r])
                        .fold(f32::NEG_INFINITY, f32::max);
                    assert!(
                        (bounds.min[r] - min).abs() < 2e-5 && (bounds.max[r] - max).abs() < 2e-5
                    );
                }
                if let Some(previous) = previous {
                    assert_ne!(previous, pixels, "{name} must deform original geometry");
                }
                if source.ends_with(".glb") && format == TextureFormat::Rgba8UnormSrgb {
                    capture(name, &pixels);
                }
                previous = Some(pixels);
            }
        }
    }
}
#[test]
fn two_instances_keep_separate_palettes_one_clear_and_change_independently() {
    let Some(g) = gpu() else { return };
    let model = fixture();
    let cam = camera();
    let rest = model.rest_pose().unwrap();
    let mid = model.sample_clip(0, 0.5).unwrap();
    let key = model.sample_clip(0, 1.0).unwrap();
    let t = target(&g, TextureFormat::Rgba8UnormSrgb);
    let view = g.create_texture_view(&t, None);
    let mut renderer =
        SkinnedModelRenderer::new(g.clone(), TextureFormat::Rgba8UnormSrgb, model.clone()).unwrap();
    renderer.clear = [0.0, 0.0, 0.0, 1.0];
    let left = placement(-1.0);
    let right = placement(1.6);
    let instances = [
        SkinnedInstance {
            pose: &rest,
            transform: left,
        },
        SkinnedInstance {
            pose: &key,
            transform: right,
        },
    ];
    renderer
        .draw(&view, (SIZE, SIZE), &cam, &light(), &instances)
        .unwrap();
    let first = g.read_texture(&t);
    capture("two_instances", &first);
    let old_bounds = renderer.bounds().to_vec();
    // Each region must exactly match rendering that pose alone. This checks the
    // first instance survived the final draw and did not acquire the last palette.
    for (instance, half) in [(instances[0], 0), (instances[1], 1)] {
        let mut solo =
            SkinnedModelRenderer::new(g.clone(), TextureFormat::Rgba8UnormSrgb, model.clone())
                .unwrap();
        solo.clear = renderer.clear;
        solo.draw(&view, (SIZE, SIZE), &cam, &light(), &[instance])
            .unwrap();
        let pixels = g.read_texture(&t);
        let mut foreground = 0;
        for y in 0..SIZE as usize {
            for x in half * SIZE as usize / 2..(half + 1) * SIZE as usize / 2 {
                let i = (y * SIZE as usize + x) * 4;
                assert_eq!(
                    &first[i..i + 4],
                    &pixels[i..i + 4],
                    "instance {half} mismatch {x},{y}"
                );
                if pixels[i..i + 3] != [0, 0, 0] {
                    foreground += 1;
                }
            }
        }
        assert!(foreground > 300, "instance {half} absent");
    }
    renderer
        .draw(
            &view,
            (SIZE, SIZE),
            &cam,
            &light(),
            &[
                SkinnedInstance {
                    pose: &mid,
                    transform: left,
                },
                instances[1],
            ],
        )
        .unwrap();
    let changed = g.read_texture(&t);
    capture("two_instances_changed", &changed);
    let mut left_changed = 0;
    for y in 0..SIZE as usize {
        for x in 0..SIZE as usize {
            let i = (y * SIZE as usize + x) * 4;
            if x >= SIZE as usize / 2 {
                assert_eq!(
                    &first[i..i + 4],
                    &changed[i..i + 4],
                    "unmodified instance changed"
                );
            } else if first[i..i + 4] != changed[i..i + 4] {
                left_changed += 1;
            }
        }
    }
    assert!(
        left_changed > 100,
        "changing first pose must change its silhouette"
    );
    assert_ne!(old_bounds[0], renderer.bounds()[0]);
    assert_eq!(old_bounds[1], renderer.bounds()[1]);
    // Empty submission clears once and has no stale instances/bounds.
    renderer
        .draw(&view, (SIZE, SIZE), &cam, &light(), &[])
        .unwrap();
    assert!(renderer.bounds().is_empty());
    assert!(g
        .read_texture(&t)
        .chunks_exact(4)
        .all(|p| p == [0, 0, 0, 255]));
}
#[test]
fn invalid_pose_placement_camera_and_shadow_requests_leave_frame_unchanged() {
    let Some(g) = gpu() else { return };
    let model = fixture();
    let pose = model.rest_pose().unwrap();
    let cam = camera();
    let t = target(&g, TextureFormat::Rgba8Unorm);
    let view = g.create_texture_view(&t, None);
    let mut renderer =
        SkinnedModelRenderer::new(g.clone(), TextureFormat::Rgba8Unorm, model.clone()).unwrap();
    renderer
        .draw(
            &view,
            (SIZE, SIZE),
            &cam,
            &light(),
            &[SkinnedInstance::new(&pose)],
        )
        .unwrap();
    let before = g.read_texture(&t);
    let bounds = renderer.bounds().to_vec();
    // Valid keys can still create a singular weighted linear blend at runtime.
    // Opposite rotations with 50/50 weights must fail before any GPU write.
    let mut source = model.source().clone();
    for v in &mut source.primitives[0].vertices {
        v.weights = [0.5, 0.5, 0.0, 0.0];
    }
    let channel = &mut source.clips[0].channels[0];
    channel.values = ChannelValues::Rotation(vec![
        [0.0, 0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]);
    let singular_model = AnimatedModel::new(source).unwrap();
    let collapsed = singular_model.sample_clip(0, 1.0).unwrap();
    let mut singular_renderer =
        SkinnedModelRenderer::new(g.clone(), TextureFormat::Rgba8Unorm, singular_model).unwrap();
    assert!(matches!(
        singular_renderer.draw(
            &view,
            (SIZE, SIZE),
            &cam,
            &light(),
            &[SkinnedInstance::new(&collapsed)]
        ),
        Err(SkinnedRenderError::InvalidPose(_))
    ));
    let foreign = fixture().rest_pose().unwrap();
    assert!(matches!(
        renderer.draw(
            &view,
            (SIZE, SIZE),
            &cam,
            &light(),
            &[SkinnedInstance::new(&foreign)]
        ),
        Err(SkinnedRenderError::InvalidPose(_))
    ));
    for value in [0.0, -1.0, f32::NAN] {
        let mut transform = IDENTITY;
        transform[0][0] = value;
        assert_eq!(
            renderer.draw(
                &view,
                (SIZE, SIZE),
                &cam,
                &light(),
                &[SkinnedInstance {
                    pose: &pose,
                    transform
                }]
            ),
            Err(SkinnedRenderError::InvalidPlacement)
        );
    }
    let mut bad_cam = cam;
    bad_cam.eye = [f32::NAN; 3];
    assert_eq!(
        renderer.draw(
            &view,
            (SIZE, SIZE),
            &bad_cam,
            &light(),
            &[SkinnedInstance::new(&pose)]
        ),
        Err(SkinnedRenderError::InvalidCamera)
    );
    let mut shadow = light();
    shadow.shadows = true;
    assert_eq!(
        renderer.draw(
            &view,
            (SIZE, SIZE),
            &cam,
            &shadow,
            &[SkinnedInstance::new(&pose)]
        ),
        Err(SkinnedRenderError::InvalidLighting)
    );
    assert_eq!(
        renderer.draw(
            &view,
            (0, SIZE),
            &cam,
            &light(),
            &[SkinnedInstance::new(&pose)]
        ),
        Err(SkinnedRenderError::InvalidTarget)
    );
    let too_many = vec![SkinnedInstance::new(&pose); 257];
    assert_eq!(
        renderer.draw(&view, (SIZE, SIZE), &cam, &light(), &too_many),
        Err(SkinnedRenderError::InstanceLimit)
    );
    assert_eq!(before, g.read_texture(&t));
    assert_eq!(bounds, renderer.bounds());
}

#[test]
fn gpu_normals_use_inverse_transpose_of_blended_skin_with_nonuniform_placement() {
    let Some(g) = gpu() else { return };
    let mut source = fixture().source().clone();
    let n = 1.0f32 / 3.0f32.sqrt();
    for v in &mut source.primitives[0].vertices {
        v.vertex.normal = [n; 3];
        // Exercise the last two scalar integer joint attributes as well.
        v.joints = [0, 0, 0, 1];
        v.weights = [0.0, 0.0, 0.5, 0.5];
    }
    for image in &mut source.images {
        image.rgba8.fill(255);
    }
    source.clips[0]
        .channels
        .push(orr_model::animation::AnimationChannel {
            node: 2,
            interpolation: orr_model::animation::Interpolation::Linear,
            times: vec![0.0],
            values: ChannelValues::Scale(vec![[2.0, 0.5, 1.0]]),
        });
    let model = AnimatedModel::new(source).unwrap();
    let pose = model.sample_clip(0, 1.0).unwrap();
    let vertices = model.deform(&pose).unwrap();
    // Independent analytic inverse transpose of (I + Rz(60) S(2,.5,1))/2.
    // Its XY block is [a c; b d]. A blended-normal-matrices shortcut produces a
    // measurably different z component and therefore fails this directional test.
    let a = 1.0;
    let b = 3.0f32.sqrt() / 2.0;
    let c = -3.0f32.sqrt() / 8.0;
    let d = 0.625;
    let determinant = a * d - b * c;
    let normal = orr_render::math3::normalize([(d - b) / determinant, (a - c) / determinant, 1.0]);
    for v in &vertices[0].vertices {
        close(v.normal, normal);
    }
    let mut transform = IDENTITY;
    transform[0][0] = 1.2;
    transform[1][1] = 0.8;
    transform[2][2] = 1.7;
    let placed_normal =
        orr_render::math3::normalize([normal[0] / 1.2, normal[1] / 0.8, normal[2] / 1.7]);
    let brightness = encode(placed_normal[2]);
    let cam = camera();
    let directional = Lighting {
        direction: [0.0, 0.0, -1.0],
        color: [1.0; 3],
        intensity: 1.0,
        ambient: 0.0,
        ..light()
    };
    let t = target(&g, TextureFormat::Rgba8UnormSrgb);
    let view = g.create_texture_view(&t, None);
    let mut renderer =
        SkinnedModelRenderer::new(g.clone(), TextureFormat::Rgba8UnormSrgb, model.clone()).unwrap();
    renderer.clear = [0.0, 0.0, 0.0, 1.0];
    renderer
        .draw(
            &view,
            (SIZE, SIZE),
            &cam,
            &directional,
            &[SkinnedInstance {
                pose: &pose,
                transform,
            }],
        )
        .unwrap();
    let pixels = g.read_texture(&t);
    let screen: Vec<_> = vertices[0]
        .vertices
        .iter()
        .map(|v| {
            let p = Mat4(transform).transform_point4(v.position);
            cam.world_to_screen([p[0], p[1], p[2]], (SIZE, SIZE))
                .unwrap()
        })
        .collect();
    let mut checked = 0;
    for y in 0..SIZE as usize {
        for x in 0..SIZE as usize {
            let inside = model.source().primitives[0]
                .indices
                .chunks_exact(3)
                .any(|t| {
                    barycentric(
                        [x as f32 + 0.5, y as f32 + 0.5],
                        screen[t[0] as usize],
                        screen[t[1] as usize],
                        screen[t[2] as usize],
                    )
                    .iter()
                    .all(|&b| b > 0.08)
                });
            if inside {
                let i = (y * SIZE as usize + x) * 4;
                near(
                    pixels[i..i + 4].try_into().unwrap(),
                    [brightness, brightness, brightness, 255],
                    "inverse-transpose normal",
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 200, "insufficient normal coverage: {checked}");
}

#[test]
fn cubic_sampled_skin_matches_independent_hermite_rotation_pixels() {
    let Some(g) = gpu() else { return };
    let mut source = fixture().source().clone();
    source.clips[0].channels[0].interpolation = orr_model::animation::Interpolation::CubicSpline;
    source.clips[0].channels[0].times = vec![0.0, 2.0];
    source.clips[0].channels[0].values = ChannelValues::Rotation(vec![
        [0.0; 4],
        [0.0, 0.0, 0.0, 1.0],
        [0.0, 0.0, 0.2, 0.0],
        [0.0; 4],
        [0.0, 0.0, 0.5, 3.0f32.sqrt() / 2.0],
        [0.0; 4],
    ]);
    let model = AnimatedModel::new(source).unwrap();
    let model = AnimatedModel::from_bytes(&model.to_bytes().unwrap()).unwrap();
    let cam = camera();
    let t = target(&g, TextureFormat::Rgba8UnormSrgb);
    let view = g.create_texture_view(&t, None);
    let mut renderer =
        SkinnedModelRenderer::new(g.clone(), TextureFormat::Rgba8UnormSrgb, model.clone()).unwrap();
    renderer.clear = [0.0, 0.0, 0.0, 1.0];
    for time in [0.0f32, 0.5, 1.0, 2.0, 0.5] {
        let pose = model.sample_clip(0, time).unwrap();
        let u = time / 2.0;
        let smooth = 3.0 * u * u - 2.0 * u * u * u;
        let outgoing = u * (1.0 - u) * (1.0 - u);
        let z = 0.5 * smooth + 2.0 * 0.2 * outgoing;
        let w = 1.0 + (3.0f32.sqrt() / 2.0 - 1.0) * smooth;
        // Normalization cancels in atan2. This analytic angle is independent
        // of the sampler, hierarchy, skin palette and CPU deformation helpers.
        let positions = reference_positions(&model, 2.0 * z.atan2(w), IDENTITY);
        for (vertex, expected) in model.deform(&pose).unwrap()[0]
            .vertices
            .iter()
            .zip(&positions)
        {
            close(vertex.position, *expected);
        }
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
        assert_pixels(&pixels, &model, &positions, &cam);
        if time == 0.5 {
            capture("cubic-quarter", &pixels);
        }
    }
}
