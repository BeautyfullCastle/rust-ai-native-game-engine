//! Imported scene acceptance: genuine import/cook/load, shared depth, GPU skinning,
//! independent pixel lighting oracle, and all-or-nothing frame validation.
//! ORR_REQUIRE_GPU=1 makes adapter absence a failure, never a silent acceptance.
#![cfg(feature = "imported-scene")]
#![allow(clippy::float_arithmetic)]
#[cfg(feature = "animation")]
use orr_model::{
    animation::{AnimatedModel, ChannelValues},
    animation_import,
};
use orr_model::{StaticModel, Vertex, IDENTITY};
use orr_render::orr_rhi::{Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions};
use orr_render::{
    Camera3D, ImportedBatch, ImportedSceneError, ImportedSceneRenderer, ImportedSceneTarget,
    Lighting, ModelRenderer, PointLight, PointLightSettings,
};
#[cfg(feature = "animation")]
use orr_render::{SkinnedInstance, SkinnedModelRenderer};

type Matrix = [[f32; 4]; 4];
type Texture = <Wgpu as Rhi>::Texture;
type View = <Wgpu as Rhi>::TextureView;
const SIZE: u32 = 129; // A pixel center lies exactly on the origin for zero-distance tests.
const EXTENT: (u32, u32) = (SIZE, SIZE);
const FORMATS: [TextureFormat; 2] = [TextureFormat::Rgba8Unorm, TextureFormat::Rgba8UnormSrgb];

fn gpu() -> Option<Wgpu> {
    match Wgpu::headless(WgpuOptions::default()) {
        Ok(g) => {
            eprintln!(
                "imported scene acceptance adapter: {} (software: {})",
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
fn imported_model() -> StaticModel {
    let model = orr_model::import::import_with_resolver(
        "fixtures/model.glb",
        include_bytes!("../../orr_model/tests/fixtures/model.glb"),
        |_| panic!("fixture embeds every dependency"),
    )
    .unwrap();
    StaticModel::from_bytes(&model.to_bytes().unwrap()).unwrap()
}
/// Original test panels derived from the genuine imported source, then cooked and
/// reloaded again. Solid texels remove UV discontinuities from the lighting oracle.
fn panel(rect: [f32; 4], transform: Matrix, texel: [u8; 3], factor: [f32; 3]) -> StaticModel {
    let mut source = imported_model().source().clone();
    source.primitives.truncate(1);
    let p = &mut source.primitives[0];
    let [left, bottom, right, top] = rect;
    p.vertices = [
        [left, bottom, 0.0],
        [right, bottom, 0.0],
        [right, top, 0.0],
        [left, top, 0.0],
    ]
    .into_iter()
    .map(|position| Vertex {
        position,
        normal: [0.0, 0.0, 1.0],
        uv: [0.25, 0.25],
    })
    .collect();
    p.indices = vec![0, 1, 2, 0, 2, 3];
    p.transform = transform;
    p.material = 0;
    source.materials[0].base_color = [factor[0], factor[1], factor[2], 1.0];
    for image in &mut source.images {
        for pixel in image.rgba8.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[texel[0], texel[1], texel[2], 255]);
        }
    }
    let model = StaticModel::new(source).unwrap();
    StaticModel::from_bytes(&model.to_bytes().unwrap()).unwrap()
}
fn translated(x: f32, y: f32, z: f32) -> Matrix {
    let mut m = IDENTITY;
    m[3] = [x, y, z, 1.0];
    m
}
fn camera() -> Camera3D {
    Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 2.0)
}
fn lighting(ambient: f32) -> Lighting {
    Lighting {
        sky: [1.0; 3],
        ground: [1.0; 3],
        ambient,
        intensity: 0.0,
        shadows: false,
        tonemap: false,
        exposure: 1.0,
        ..Default::default()
    }
}
fn target(g: &Wgpu, format: TextureFormat, size: (u32, u32)) -> Texture {
    g.create_texture(&TextureDesc {
        label: "imported scene acceptance",
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
    scene.clear = [0.0, 0.0, 0.0, 1.0];
    assert_eq!(scene.format(), format);
    assert_eq!(scene.depth_size(), None);
    assert_eq!(scene.depth_generation(), 0);
    scene
}
#[allow(clippy::too_many_arguments)]
fn draw(
    scene: &mut ImportedSceneRenderer<Wgpu>,
    view: &View,
    size: (u32, u32),
    cam: &Camera3D,
    light: &Lighting,
    point: &PointLightSettings,
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
        cam,
        light,
        point,
        batches,
    )
}
fn near(actual: [u8; 4], expected: [u8; 4], label: &str) {
    assert!(
        actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 3),
        "{label}: actual {actual:?}, expected {expected:?}"
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
    let v = v.clamp(0.0, 1.0);
    ((if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }) * 255.0)
        .round() as u8
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn unit(v: [f32; 3]) -> [f32; 3] {
    let length = dot(v, v).sqrt();
    v.map(|x| x / length)
}
fn transform(m: Matrix, p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r])
}
fn barycentric(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> [f32; 3] {
    let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    let u = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / d;
    let v = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / d;
    [u, v, 1.0 - u - v]
}
#[derive(Clone)]
struct Surface {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    indices: Vec<u32>,
    albedo: [f32; 3],
}
fn surface(model: &StaticModel, normal: [f32; 3]) -> Surface {
    let p = &model.source().primitives[0];
    let m = &model.source().materials[p.material as usize];
    let texel = &model.source().images[m.image as usize].rgba8;
    Surface {
        positions: p
            .vertices
            .iter()
            .map(|v| transform(p.transform, v.position))
            .collect(),
        normals: vec![normal; p.vertices.len()],
        indices: p.indices.clone(),
        albedo: std::array::from_fn(|k| decode(texel[k]) * m.base_color[k]),
    }
}
/// Scalar CPU reference, deliberately independent of renderer uniform preparation
/// and model deformation. The point term is a bounded stylized diffuse falloff.
fn shade(
    position: [f32; 3],
    normal: [f32; 3],
    albedo: [f32; 3],
    light: &Lighting,
    settings: &PointLightSettings,
) -> [u8; 4] {
    let n = unit(normal);
    let sun = unit(light.direction).map(|x| -x);
    let mut point = [0.0; 3];
    if let Some(p) = settings.point_light {
        let delta = std::array::from_fn(|k| p.position[k] - position[k]);
        let distance = dot(delta, delta).sqrt();
        if distance > 1e-6 {
            let diffuse = dot(n, delta.map(|x| x / distance)).max(0.0);
            let falloff = (1.0 - distance / p.range).max(0.0).powi(2);
            point = p.color.map(|c| c * p.intensity * diffuse * falloff);
        }
    }
    let mut result = [0, 0, 0, 255];
    for k in 0..3 {
        let hemisphere = light.ground[k] * (0.5 - 0.5 * n[1]) + light.sky[k] * (0.5 + 0.5 * n[1]);
        let irradiance = hemisphere * light.ambient
            + light.color[k] * light.intensity * dot(n, sun).max(0.0)
            + point[k];
        let mut linear = albedo[k] * irradiance * light.exposure;
        if light.tonemap {
            linear = (linear * (2.51 * linear + 0.03)) / (linear * (2.43 * linear + 0.59) + 0.14);
        }
        assert!(linear.is_finite(), "CPU oracle produced nonfinite color");
        result[k] = encode(linear);
    }
    result
}
/// Rasterize confident triangle interiors at the actual pixel centers. Skip only
/// edge/depth-crossing tolerances; require substantial foreground and background.
/// Camera faces -Z so the nearest covered triangle has the largest world Z.
fn assert_oracle(
    pixels: &[u8],
    size: (u32, u32),
    cam: &Camera3D,
    surfaces: &[Surface],
    light: &Lighting,
    point: &PointLightSettings,
) -> Vec<usize> {
    let screen: Vec<Vec<_>> = surfaces
        .iter()
        .map(|s| {
            s.positions
                .iter()
                .map(|&p| cam.world_to_screen(p, size).unwrap())
                .collect()
        })
        .collect();
    let mut counts = vec![0; surfaces.len()];
    let mut clear = 0;
    for y in 0..size.1 as usize {
        for x in 0..size.0 as usize {
            let mut candidates = Vec::new();
            let mut near_geometry = false;
            for (si, s) in surfaces.iter().enumerate() {
                for t in s.indices.chunks_exact(3) {
                    let ids = [t[0] as usize, t[1] as usize, t[2] as usize];
                    let b = barycentric(
                        [x as f32 + 0.5, y as f32 + 0.5],
                        screen[si][ids[0]],
                        screen[si][ids[1]],
                        screen[si][ids[2]],
                    );
                    if b.iter().all(|&v| v >= -0.035) {
                        near_geometry = true;
                    }
                    if b.iter().all(|&v| v >= 0.0) {
                        let p: [f32; 3] = std::array::from_fn(|k| {
                            (0..3).map(|i| b[i] * s.positions[ids[i]][k]).sum()
                        });
                        let n = std::array::from_fn(|k| {
                            (0..3).map(|i| b[i] * s.normals[ids[i]][k]).sum()
                        });
                        candidates.push((si, p, n, b.iter().all(|&v| v > 0.035)));
                    }
                }
            }
            candidates.sort_by(|a, b| b.1[2].total_cmp(&a.1[2]));
            let offset = (y * size.0 as usize + x) * 4;
            let actual = pixels[offset..offset + 4].try_into().unwrap();
            if let Some(&(si, p, n, interior)) = candidates.first() {
                // Ignore only genuine intersections, not diagonal triangles of one surface.
                let depth_ambiguous = candidates
                    .iter()
                    .skip(1)
                    .any(|&(other, q, _, _)| other != si && (p[2] - q[2]).abs() < 0.025);
                if interior && !depth_ambiguous {
                    near(
                        actual,
                        shade(p, n, surfaces[si].albedo, light, point),
                        &format!("surface {si}, pixel {x},{y}"),
                    );
                    counts[si] += 1;
                }
            } else if !near_geometry {
                near(actual, [0, 0, 0, 255], &format!("clear pixel {x},{y}"));
                clear += 1;
            }
        }
    }
    assert!(
        counts.iter().sum::<usize>() > 200,
        "insufficient foreground oracle coverage: {counts:?}"
    );
    assert!(
        clear > 2000,
        "insufficient clear-region oracle coverage: {clear}"
    );
    counts
}
fn image_near(a: &[u8], b: &[u8]) {
    assert_eq!(a.len(), b.len());
    for (i, (a, b)) in a.chunks_exact(4).zip(b.chunks_exact(4)).enumerate() {
        near(
            a.try_into().unwrap(),
            b.try_into().unwrap(),
            &format!("linear/sRGB pixel {i}"),
        );
    }
}

#[test]
fn static_batches_share_depth_in_both_orders_and_ignore_private_clear_colors() {
    let Some(g) = gpu() else { return };
    let cam = camera();
    let light = lighting(1.0);
    let off = PointLightSettings::default();
    let front = panel(
        [-0.9, -0.7, 0.8, 0.9],
        translated(-0.35, 0.0, 0.4),
        [220, 40, 60],
        [1.0; 3],
    );
    let back = panel(
        [-0.9, -0.9, 0.9, 0.7],
        translated(0.45, 0.0, -0.4),
        [30, 100, 220],
        [1.0; 3],
    );
    let oracle = [
        surface(&front, [0.0, 0.0, 1.0]),
        surface(&back, [0.0, 0.0, 1.0]),
    ];
    for format in FORMATS {
        let t = target(&g, format, EXTENT);
        let view = g.create_texture_view(&t, None);
        let mut scene = scene(&g, format);
        let mut front = ModelRenderer::new(g.clone(), format, front.clone()).unwrap();
        let mut back = ModelRenderer::new(g.clone(), format, back.clone()).unwrap();
        front.clear = [1.0, 0.0, 1.0, 1.0];
        back.clear = [0.0, 1.0, 0.0, 1.0];
        let mut prior = None;
        for reverse in [false, true] {
            let mut batches = if reverse {
                [
                    ImportedBatch::Static(&mut back),
                    ImportedBatch::Static(&mut front),
                ]
            } else {
                [
                    ImportedBatch::Static(&mut front),
                    ImportedBatch::Static(&mut back),
                ]
            };
            draw(&mut scene, &view, EXTENT, &cam, &light, &off, &mut batches).unwrap();
            let pixels = g.read_texture(&t);
            let counts = assert_oracle(&pixels, EXTENT, &cam, &oracle, &light, &off);
            assert!(
                counts.iter().all(|&n| n > 300),
                "both imported assets must survive: {counts:?}"
            );
            if let Some(previous) = prior {
                assert_eq!(previous, pixels, "batch order changed opaque depth");
            }
            prior = Some(pixels);
            assert_eq!(scene.depth_size(), Some(EXTENT));
            assert_eq!(scene.depth_generation(), 1);
        }
    }
}

#[test]
fn point_light_near_far_range_backface_color_and_zero_match_scalar_oracle() {
    let Some(g) = gpu() else { return };
    let cam = camera();
    let light = lighting(0.12);
    let model = panel(
        [-1.3, -1.3, 1.3, 1.3],
        IDENTITY,
        [180, 130, 220],
        [0.8, 0.9, 0.7],
    );
    let oracle = [surface(&model, [0.0, 0.0, 1.0])];
    let p = PointLight {
        position: [0.0, 0.0, 1.0],
        color: [1.0, 0.35, 0.6],
        intensity: 1.8,
        range: 4.0,
    };
    let cases = [
        ("off", None),
        ("near", Some(p)),
        (
            "far",
            Some(PointLight {
                position: [0.0, 0.0, 3.0],
                ..p
            }),
        ),
        ("short range", Some(PointLight { range: 1.7, ..p })),
        (
            "colored",
            Some(PointLight {
                color: [0.1, 1.0, 0.2],
                ..p
            }),
        ),
        (
            "intensity zero",
            Some(PointLight {
                intensity: 0.0,
                ..p
            }),
        ),
        (
            "out of range",
            Some(PointLight {
                position: [0.0, 0.0, 5.0],
                range: 1.0,
                ..p
            }),
        ),
        (
            "backface",
            Some(PointLight {
                position: [0.0, 0.0, -1.0],
                ..p
            }),
        ),
        (
            "coincident",
            Some(PointLight {
                position: [0.0, 0.0, 0.0],
                ..p
            }),
        ),
        (
            "epsilon distance",
            Some(PointLight {
                position: [0.0, 0.0, 0.5e-6],
                ..p
            }),
        ),
    ];
    let mut images = Vec::new();
    for format in FORMATS {
        let t = target(&g, format, EXTENT);
        let view = g.create_texture_view(&t, None);
        let mut scene = scene(&g, format);
        let mut renderer = ModelRenderer::new(g.clone(), format, model.clone()).unwrap();
        let mut captures: Vec<Vec<u8>> = Vec::new();
        for (name, point_light) in cases {
            let point = PointLightSettings { point_light };
            draw(
                &mut scene,
                &view,
                EXTENT,
                &cam,
                &light,
                &point,
                &mut [ImportedBatch::Static(&mut renderer)],
            )
            .unwrap();
            let pixels = g.read_texture(&t);
            assert_oracle(&pixels, EXTENT, &cam, &oracle, &light, &point);
            if ["intensity zero", "out of range", "backface", "coincident"].contains(&name) {
                assert_eq!(
                    captures[0], pixels,
                    "{name} must be byte-identical to the point-off baseline"
                );
            }
            if name == "epsilon distance" {
                let center = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
                assert_eq!(
                    &captures[0][center..center + 4],
                    &pixels[center..center + 4],
                    "epsilon guard must preserve finite ambient at coincident pixel"
                );
            }
            captures.push(pixels);
        }
        let center = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
        assert!(
            captures[1][center] > captures[2][center] + 20,
            "near light must be brighter than far light"
        );
        assert!(
            captures[1][center] > captures[3][center] + 20,
            "range must change attenuation"
        );
        assert_ne!(
            captures[1], captures[4],
            "light color must change actual output"
        );
        renderer.clear = scene.clear;
        renderer.draw(&view, EXTENT, &cam, &light).unwrap();
        assert_eq!(
            g.read_texture(&t),
            captures[0],
            "off composed static shading must preserve standalone pixels"
        );
        images.push(captures);
    }
    for (linear, srgb) in images[0].iter().zip(&images[1]) {
        image_near(linear, srgb);
    }
}

#[test]
fn static_point_lighting_uses_transformed_world_position_and_inverse_transpose_normal() {
    let Some(g) = gpu() else { return };
    let cam = camera();
    // x->z shear, nonuniform scale, and translation make object-space light math
    // and direct-model-matrix normals measurably wrong over the whole surface.
    let mut placement = translated(0.3, -0.2, 0.35);
    placement[0] = [0.8, 0.0, 0.55, 0.0];
    placement[1][1] = 1.1;
    placement[2][2] = 1.5;
    let model = panel(
        [-1.0, -1.0, 1.0, 1.0],
        placement,
        [160, 205, 120],
        [0.75, 0.6, 0.9],
    );
    let normal = unit([-0.55 / 0.8, 0.0, 1.0]);
    let oracle = [surface(&model, normal)];
    let light = Lighting {
        direction: [0.2, -0.3, -1.0],
        color: [0.8, 0.6, 0.4],
        intensity: 0.2,
        sky: [0.3, 0.5, 0.9],
        ground: [0.7, 0.4, 0.1],
        ambient: 0.25,
        exposure: 0.8,
        ..lighting(0.0)
    };
    let point = PointLightSettings {
        point_light: Some(PointLight {
            position: [-0.6, 0.9, 2.1],
            color: [0.4, 0.8, 1.0],
            intensity: 2.4,
            range: 4.5,
        }),
    };
    let mut captures: Vec<Vec<u8>> = Vec::new();
    for format in FORMATS {
        let t = target(&g, format, EXTENT);
        let view = g.create_texture_view(&t, None);
        let mut scene = scene(&g, format);
        let mut renderer = ModelRenderer::new(g.clone(), format, model.clone()).unwrap();
        draw(
            &mut scene,
            &view,
            EXTENT,
            &cam,
            &light,
            &point,
            &mut [ImportedBatch::Static(&mut renderer)],
        )
        .unwrap();
        let pixels = g.read_texture(&t);
        assert_oracle(&pixels, EXTENT, &cam, &oracle, &light, &point);
        captures.push(pixels);
    }
    image_near(&captures[0], &captures[1]);
}

#[test]
fn resize_empty_frames_and_subsequent_far_geometry_never_reuse_stale_depth() {
    let Some(g) = gpu() else { return };
    let format = TextureFormat::Rgba8UnormSrgb;
    let mut scene = scene(&g, format);
    let cam = camera();
    let light = lighting(1.0);
    let off = PointLightSettings::default();
    let front = panel(
        [-1.2, -1.2, 1.2, 1.2],
        translated(0.0, 0.0, 1.0),
        [240, 30, 30],
        [1.0; 3],
    );
    let back = panel(
        [-1.2, -1.2, 1.2, 1.2],
        translated(0.0, 0.0, -1.0),
        [30, 210, 50],
        [1.0; 3],
    );
    let oracle = [surface(&back, [0.0, 0.0, 1.0])];
    let mut front = ModelRenderer::new(g.clone(), format, front).unwrap();
    let mut back = ModelRenderer::new(g.clone(), format, back).unwrap();
    for (i, size) in [EXTENT, (SIZE * 2, SIZE), EXTENT].into_iter().enumerate() {
        let t = target(&g, format, size);
        let view = g.create_texture_view(&t, None);
        draw(
            &mut scene,
            &view,
            size,
            &cam,
            &light,
            &off,
            &mut [ImportedBatch::Static(&mut front)],
        )
        .unwrap();
        assert_eq!(scene.depth_size(), Some(size));
        assert_eq!(scene.depth_generation(), i as u64 + 1);
        // A new populated frame must clear depth even with no intervening empty frame.
        draw(
            &mut scene,
            &view,
            size,
            &cam,
            &light,
            &off,
            &mut [ImportedBatch::Static(&mut back)],
        )
        .unwrap();
        assert_oracle(&g.read_texture(&t), size, &cam, &oracle, &light, &off);
        draw(&mut scene, &view, size, &cam, &light, &off, &mut []).unwrap();
        assert!(g
            .read_texture(&t)
            .chunks_exact(4)
            .all(|p| p == [0, 0, 0, 255]));
        draw(
            &mut scene,
            &view,
            size,
            &cam,
            &light,
            &off,
            &mut [ImportedBatch::Static(&mut back)],
        )
        .unwrap();
        assert_oracle(&g.read_texture(&t), size, &cam, &oracle, &light, &off);
        assert_eq!(
            scene.depth_generation(),
            i as u64 + 1,
            "same size must reuse allocation, never depth contents"
        );
    }
}

#[test]
fn invalid_later_static_batch_target_camera_and_light_leave_image_and_depth_unchanged() {
    let Some(g) = gpu() else { return };
    let format = TextureFormat::Rgba8Unorm;
    let t = target(&g, format, EXTENT);
    let view = g.create_texture_view(&t, None);
    let mut scene = scene(&g, format);
    let cam = camera();
    let light = lighting(0.3);
    let off = PointLightSettings::default();
    let model = panel([-1.0, -1.0, 1.0, 1.0], IDENTITY, [120, 180, 230], [1.0; 3]);
    let mut good = ModelRenderer::new(g.clone(), format, model.clone()).unwrap();
    let mut wrong = ModelRenderer::new(g.clone(), TextureFormat::Rgba8UnormSrgb, model).unwrap();
    draw(
        &mut scene,
        &view,
        EXTENT,
        &cam,
        &light,
        &off,
        &mut [ImportedBatch::Static(&mut good)],
    )
    .unwrap();
    let before = g.read_texture(&t);
    let generation = scene.depth_generation();
    // Requested resize would replace depth if validation were incremental. A bad
    // later batch must fail before even the first batch's globals or attachments.
    assert!(draw(
        &mut scene,
        &view,
        (SIZE + 1, SIZE),
        &cam,
        &light,
        &off,
        &mut [
            ImportedBatch::Static(&mut good),
            ImportedBatch::Static(&mut wrong)
        ]
    )
    .is_err());
    assert_eq!(g.read_texture(&t), before);
    assert_eq!(scene.depth_size(), Some(EXTENT));
    assert_eq!(scene.depth_generation(), generation);
    let base = PointLight {
        position: [0.0, 0.0, 1.0],
        color: [1.0; 3],
        intensity: 1.0,
        range: 3.0,
    };
    for point in [
        PointLight {
            position: [f32::NAN, 0.0, 0.0],
            ..base
        },
        PointLight {
            position: [f32::INFINITY, 0.0, 0.0],
            ..base
        },
        PointLight {
            color: [-0.1, 1.0, 1.0],
            ..base
        },
        PointLight {
            color: [f32::NAN, 1.0, 1.0],
            ..base
        },
        PointLight {
            intensity: -1.0,
            ..base
        },
        PointLight {
            intensity: f32::INFINITY,
            ..base
        },
        PointLight { range: 0.0, ..base },
        PointLight {
            range: f32::NAN,
            ..base
        },
    ] {
        assert!(draw(
            &mut scene,
            &view,
            (SIZE + 1, SIZE),
            &cam,
            &light,
            &PointLightSettings {
                point_light: Some(point)
            },
            &mut [ImportedBatch::Static(&mut good)]
        )
        .is_err());
        assert_eq!(g.read_texture(&t), before);
        assert_eq!(scene.depth_generation(), generation);
    }
    for size in [(0, SIZE), (SIZE, 0), (8193, SIZE), (SIZE, 8193)] {
        assert!(draw(
            &mut scene,
            &view,
            size,
            &cam,
            &light,
            &off,
            &mut [ImportedBatch::Static(&mut good)]
        )
        .is_err());
    }
    for (target_format, sample_count) in [(TextureFormat::Rgba8UnormSrgb, 1), (format, 4)] {
        assert!(scene
            .draw(
                ImportedSceneTarget {
                    view: &view,
                    size: EXTENT,
                    format: target_format,
                    sample_count
                },
                &cam,
                &light,
                &off,
                &mut [ImportedBatch::Static(&mut good)]
            )
            .is_err());
    }
    let mut invalid = cam;
    invalid.eye = [f32::NAN; 3];
    assert!(draw(
        &mut scene,
        &view,
        (SIZE + 1, SIZE),
        &invalid,
        &light,
        &off,
        &mut [ImportedBatch::Static(&mut good)]
    )
    .is_err());
    let mut invalid_light = light;
    invalid_light.shadows = true;
    invalid_light.shadow_radius = f32::NAN;
    assert!(draw(
        &mut scene,
        &view,
        (SIZE + 1, SIZE),
        &cam,
        &invalid_light,
        &off,
        &mut [ImportedBatch::Static(&mut good)]
    )
    .is_err());
    scene.clear[0] = f64::NAN;
    assert!(draw(
        &mut scene,
        &view,
        (SIZE + 1, SIZE),
        &cam,
        &light,
        &off,
        &mut [ImportedBatch::Static(&mut good)]
    )
    .is_err());
    scene.clear = [0.0, 0.0, 0.0, 1.0];
    assert_eq!(g.read_texture(&t), before);
    assert_eq!(scene.depth_generation(), generation);
    assert_eq!(scene.depth_size(), Some(EXTENT));
    draw(
        &mut scene,
        &view,
        EXTENT,
        &cam,
        &light,
        &off,
        &mut [ImportedBatch::Static(&mut good)],
    )
    .unwrap();
    assert_eq!(
        g.read_texture(&t),
        before,
        "failed requests must not poison the next valid frame"
    );
}

#[test]
fn saved_reloaded_and_failed_reload_settings_preserve_identical_gpu_output() {
    let Some(g) = gpu() else { return };
    let format = TextureFormat::Rgba8UnormSrgb;
    let t = target(&g, format, EXTENT);
    let view = g.create_texture_view(&t, None);
    let mut scene = scene(&g, format);
    let model = panel(
        [-1.0, -1.0, 1.0, 1.0],
        IDENTITY,
        [175, 145, 90],
        [0.8, 0.7, 1.0],
    );
    let mut renderer = ModelRenderer::new(g.clone(), format, model).unwrap();
    let cam = camera();
    let light = lighting(0.12);
    let original = PointLightSettings {
        point_light: Some(PointLight {
            position: [0.3, 0.5, 1.1],
            color: [0.4, 1.0, 0.7],
            intensity: 2.0,
            range: 4.0,
        }),
    };
    draw(
        &mut scene,
        &view,
        EXTENT,
        &cam,
        &light,
        &original,
        &mut [ImportedBatch::Static(&mut renderer)],
    )
    .unwrap();
    let before = g.read_texture(&t);
    let path = std::env::temp_dir().join(format!(
        "orr-imported-scene-point-light-{}.json",
        std::process::id()
    ));
    original.save(&path).unwrap();
    let loaded = PointLightSettings::load(&path).unwrap();
    assert_eq!(original, loaded);
    let from_bytes = PointLightSettings::from_bytes(&original.to_bytes().unwrap()).unwrap();
    let mut reloaded = PointLightSettings::default();
    reloaded.reload(&path).unwrap();
    for settings in [&loaded, &from_bytes, &reloaded] {
        draw(
            &mut scene,
            &view,
            EXTENT,
            &cam,
            &light,
            settings,
            &mut [ImportedBatch::Static(&mut renderer)],
        )
        .unwrap();
        assert_eq!(
            g.read_texture(&t),
            before,
            "settings persistence changed rendered light"
        );
    }
    std::fs::write(&path, b"{\"version\":1,\"point_light\":{\"range\":0}}").unwrap();
    assert!(reloaded.reload(&path).is_err());
    assert_eq!(reloaded, original, "invalid reload must be transactional");
    draw(
        &mut scene,
        &view,
        EXTENT,
        &cam,
        &light,
        &reloaded,
        &mut [ImportedBatch::Static(&mut renderer)],
    )
    .unwrap();
    assert_eq!(g.read_texture(&t), before);
    std::fs::remove_file(&path).unwrap();
}

#[cfg(feature = "animation")]
fn animated_model() -> AnimatedModel {
    let imported = animation_import::import_with_resolver(
        "fixtures/animated_strip.glb",
        include_bytes!("../../orr_model/tests/fixtures/animated_strip.glb"),
        |_| panic!("fixture embeds all dependencies"),
    )
    .unwrap();
    let reloaded = AnimatedModel::from_bytes(&imported.to_bytes().unwrap()).unwrap();
    let mut source = reloaded.source().clone();
    // Same nonidentity bind pivots and mixed original weights, but bend about X
    // so actual animated vertices cross the static occluder's depth plane.
    source.clips[0].channels[0].values = ChannelValues::Rotation(vec![
        [0.0, 0.0, 0.0, 1.0],
        [0.5, 0.0, 0.0, 0.75f32.sqrt()],
        [0.0, 0.0, 0.0, 1.0],
    ]);
    source.materials[0].base_color = [0.7, 0.9, 0.8, 1.0];
    for image in &mut source.images {
        for pixel in image.rgba8.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[45, 140, 235, 255]);
        }
    }
    let model = AnimatedModel::new(source).unwrap();
    AnimatedModel::from_bytes(&model.to_bytes().unwrap()).unwrap()
}
#[cfg(feature = "animation")]
fn animated_surface(model: &AnimatedModel, angle: f32, placement: Matrix) -> Surface {
    let p = &model.source().primitives[0];
    let m = &model.source().materials[p.material as usize];
    let texel = &model.source().images[m.image as usize].rgba8;
    let (s, c) = angle.sin_cos();
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    for v in &p.vertices {
        let original = v.vertex.position;
        let weight = v
            .joints
            .iter()
            .zip(v.weights)
            .filter_map(|(&j, w)| (j == 1).then_some(w))
            .sum::<f32>()
            / v.weights.iter().sum::<f32>();
        let rotated = [
            original[0],
            1.7 + c * (original[1] - 1.7) - s * original[2],
            s * (original[1] - 1.7) + c * original[2],
        ];
        let mixed = std::array::from_fn(|k| original[k] * (1.0 - weight) + rotated[k] * weight);
        positions.push(transform(placement, mixed));
        // Inverse transpose of blended I/Rx, followed by diagonal placement.
        // Normalize each vertex before interpolation, matching the skinning contract.
        let a = 1.0 - weight + weight * c;
        let b = weight * s;
        let determinant = a * a + b * b;
        normals.push(unit([
            0.0,
            -b / determinant / placement[1][1],
            a / determinant / placement[2][2],
        ]));
    }
    Surface {
        positions,
        normals,
        indices: p.indices.clone(),
        albedo: std::array::from_fn(|k| decode(texel[k]) * m.base_color[k]),
    }
}
#[cfg(feature = "animation")]
fn assert_bounds(renderer: &SkinnedModelRenderer<Wgpu>, surfaces: &[Surface]) {
    assert_eq!(renderer.bounds().len(), surfaces.len());
    for (bounds, surface) in renderer.bounds().iter().zip(surfaces) {
        for k in 0..3 {
            let min = surface
                .positions
                .iter()
                .map(|p| p[k])
                .fold(f32::INFINITY, f32::min);
            let max = surface
                .positions
                .iter()
                .map(|p| p[k])
                .fold(f32::NEG_INFINITY, f32::max);
            assert!(
                (bounds.min[k] - min).abs() < 3e-5 && (bounds.max[k] - max).abs() < 3e-5,
                "bounds axis {k}: {bounds:?}, expected {min}..{max}"
            );
        }
    }
}

#[cfg(feature = "animation")]
#[test]
fn static_skinned_batches_occlude_in_both_orders_as_animated_vertices_cross_plane() {
    let Some(g) = gpu() else { return };
    let model = animated_model();
    let poses = [
        model.rest_pose().unwrap(),
        model.sample_clip(0, 0.5).unwrap(),
        model.sample_clip(0, 1.0).unwrap(),
    ];
    let angles = [0.0, std::f32::consts::PI / 6.0, std::f32::consts::PI / 3.0];
    let cam = Camera3D::orthographic([0.0, 1.5, 5.0], [0.0, 1.5, 0.0], 2.2);
    let static_model = panel(
        [-1.0, 0.55, 1.0, 2.7],
        translated(0.0, 0.0, 0.25),
        [220, 65, 35],
        [1.0; 3],
    );
    let wall = surface(&static_model, [0.0, 0.0, 1.0]);
    let light = lighting(0.35);
    let point = PointLightSettings {
        point_light: Some(PointLight {
            position: [-0.7, 2.5, 2.0],
            color: [0.9, 0.8, 0.6],
            intensity: 1.5,
            range: 4.0,
        }),
    };
    let mut format_images = Vec::new();
    for format in FORMATS {
        let t = target(&g, format, EXTENT);
        let view = g.create_texture_view(&t, None);
        let mut scene = scene(&g, format);
        let mut fixed = ModelRenderer::new(g.clone(), format, static_model.clone()).unwrap();
        let mut animated = SkinnedModelRenderer::new(g.clone(), format, model.clone()).unwrap();
        fixed.clear = [1.0, 0.0, 1.0, 1.0];
        animated.clear = [0.0, 1.0, 0.0, 1.0];
        let mut frames = Vec::new();
        for (pose, angle) in poses.iter().zip(angles) {
            let animated_oracle = animated_surface(&model, angle, IDENTITY);
            let oracle = [wall.clone(), animated_oracle.clone()];
            let instances = [SkinnedInstance::new(pose)];
            let mut previous = None;
            for reverse in [false, true] {
                let mut batches = if reverse {
                    [
                        ImportedBatch::Skinned {
                            renderer: &mut animated,
                            instances: &instances,
                        },
                        ImportedBatch::Static(&mut fixed),
                    ]
                } else {
                    [
                        ImportedBatch::Static(&mut fixed),
                        ImportedBatch::Skinned {
                            renderer: &mut animated,
                            instances: &instances,
                        },
                    ]
                };
                draw(
                    &mut scene,
                    &view,
                    EXTENT,
                    &cam,
                    &light,
                    &point,
                    &mut batches,
                )
                .unwrap();
                let pixels = g.read_texture(&t);
                let counts = assert_oracle(&pixels, EXTENT, &cam, &oracle, &light, &point);
                assert!(
                    counts[0] > 900 && counts[1] > 40,
                    "both static and animated geometry must be visible: {counts:?}"
                );
                assert_bounds(&animated, std::slice::from_ref(&animated_oracle));
                if let Some(previous) = previous {
                    assert_eq!(
                        previous, pixels,
                        "static/skinned depth changed with batch order"
                    );
                }
                previous = Some(pixels);
            }
            frames.push(previous.unwrap());
        }
        assert_ne!(
            frames[0], frames[1],
            "midpoint pose must move silhouette/depth"
        );
        assert_ne!(frames[1], frames[2], "key pose must move silhouette/depth");
        // A vertex moves from behind to in front of the wall by skinning alone.
        let rest = animated_surface(&model, 0.0, IDENTITY);
        let key = animated_surface(&model, angles[2], IDENTITY);
        assert!(rest.positions.iter().all(|p| p[2] < 0.25));
        assert!(key.positions.iter().any(|p| p[2] > 0.25));
        assert!(key.positions.iter().any(|p| p[2] < 0.25));
        format_images.push(frames);
    }
    for (linear, srgb) in format_images[0].iter().zip(&format_images[1]) {
        image_near(linear, srgb);
    }
}

#[cfg(feature = "animation")]
#[test]
fn skinned_point_lighting_follows_deformed_world_positions_normals_and_separate_instances() {
    let Some(g) = gpu() else { return };
    let model = animated_model();
    let rest = model.rest_pose().unwrap();
    let bent = model.sample_clip(0, 1.0).unwrap();
    let mut left = translated(-0.9, -0.25, 0.25);
    left[0][0] = 0.9;
    left[1][1] = 0.85;
    left[2][2] = 1.4;
    let mut right = translated(0.9, -0.1, -0.3);
    right[0][0] = 0.8;
    right[1][1] = 0.7;
    right[2][2] = 1.1;
    let oracle = [
        animated_surface(&model, std::f32::consts::PI / 3.0, left),
        animated_surface(&model, 0.0, right),
    ];
    let cam = Camera3D::orthographic([0.0, 1.2, 5.0], [0.0, 1.2, 0.0], 2.1);
    let light = lighting(0.2);
    let settings = PointLightSettings {
        point_light: Some(PointLight {
            position: [-0.6, 2.2, 2.6],
            color: [0.3, 1.0, 0.7],
            intensity: 2.2,
            range: 4.5,
        }),
    };
    let instances = [
        SkinnedInstance {
            pose: &bent,
            transform: left,
        },
        SkinnedInstance {
            pose: &rest,
            transform: right,
        },
    ];
    let mut captures: Vec<Vec<u8>> = Vec::new();
    for format in FORMATS {
        let t = target(&g, format, EXTENT);
        let view = g.create_texture_view(&t, None);
        let mut scene = scene(&g, format);
        let mut renderer = SkinnedModelRenderer::new(g.clone(), format, model.clone()).unwrap();
        draw(
            &mut scene,
            &view,
            EXTENT,
            &cam,
            &light,
            &settings,
            &mut [ImportedBatch::Skinned {
                renderer: &mut renderer,
                instances: &instances,
            }],
        )
        .unwrap();
        let pixels = g.read_texture(&t);
        let counts = assert_oracle(&pixels, EXTENT, &cam, &oracle, &light, &settings);
        assert!(
            counts.iter().all(|&n| n > 100),
            "both instance regions need oracle coverage: {counts:?}"
        );
        assert_bounds(&renderer, &oracle);
        captures.push(pixels);
        // Disabling the added light preserves the exact standalone appearance.
        let off = PointLightSettings::default();
        draw(
            &mut scene,
            &view,
            EXTENT,
            &cam,
            &light,
            &off,
            &mut [ImportedBatch::Skinned {
                renderer: &mut renderer,
                instances: &instances,
            }],
        )
        .unwrap();
        let composed_off = g.read_texture(&t);
        renderer.clear = scene.clear;
        renderer
            .draw(&view, EXTENT, &cam, &light, &instances)
            .unwrap();
        assert_eq!(
            g.read_texture(&t),
            composed_off,
            "off composed skinning must preserve standalone pixels"
        );
        for point in [
            PointLight {
                intensity: 0.0,
                ..settings.point_light.unwrap()
            },
            PointLight {
                position: [100.0, 100.0, 100.0],
                range: 1.0,
                ..settings.point_light.unwrap()
            },
        ] {
            draw(
                &mut scene,
                &view,
                EXTENT,
                &cam,
                &light,
                &PointLightSettings {
                    point_light: Some(point),
                },
                &mut [ImportedBatch::Skinned {
                    renderer: &mut renderer,
                    instances: &instances,
                }],
            )
            .unwrap();
            assert_eq!(
                g.read_texture(&t),
                composed_off,
                "ineffective point light changed skinned output"
            );
        }
        draw(
            &mut scene,
            &view,
            EXTENT,
            &cam,
            &light,
            &off,
            &mut [ImportedBatch::Skinned {
                renderer: &mut renderer,
                instances: &[],
            }],
        )
        .unwrap();
        assert!(
            renderer.bounds().is_empty(),
            "accepted zero-instance batch must retire prior bounds"
        );
        assert!(g
            .read_texture(&t)
            .chunks_exact(4)
            .all(|p| p == [0, 0, 0, 255]));
    }
    image_near(&captures[0], &captures[1]);
}

#[cfg(feature = "animation")]
#[test]
fn invalid_later_pose_format_placement_or_light_preserves_all_submitted_bounds() {
    let Some(g) = gpu() else { return };
    let format = TextureFormat::Rgba8Unorm;
    let t = target(&g, format, EXTENT);
    let view = g.create_texture_view(&t, None);
    let mut scene = scene(&g, format);
    let model = animated_model();
    let rest = model.rest_pose().unwrap();
    let bent = model.sample_clip(0, 1.0).unwrap();
    let foreign = animated_model().rest_pose().unwrap();
    let cam = Camera3D::orthographic([0.0, 1.2, 5.0], [0.0, 1.2, 0.0], 2.1);
    let light = lighting(0.7);
    let off = PointLightSettings::default();
    let mut first = SkinnedModelRenderer::new(g.clone(), format, model.clone()).unwrap();
    let mut second = SkinnedModelRenderer::new(g.clone(), format, model).unwrap();
    let left = translated(-0.8, 0.0, 0.0);
    let right = translated(0.8, 0.0, 0.0);
    let initial_left = [SkinnedInstance {
        pose: &rest,
        transform: left,
    }];
    let initial_right = [SkinnedInstance {
        pose: &rest,
        transform: right,
    }];
    draw(
        &mut scene,
        &view,
        EXTENT,
        &cam,
        &light,
        &off,
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
    let before = g.read_texture(&t);
    let bounds = (first.bounds().to_vec(), second.bounds().to_vec());
    let generation = scene.depth_generation();
    let changed = [SkinnedInstance {
        pose: &bent,
        transform: left,
    }];
    let invalid_pose = [SkinnedInstance {
        pose: &foreign,
        transform: right,
    }];
    let mut singular = right;
    singular[0][0] = 0.0;
    let invalid_placement = [SkinnedInstance {
        pose: &rest,
        transform: singular,
    }];
    for invalid in [&invalid_pose[..], &invalid_placement[..]] {
        assert!(draw(
            &mut scene,
            &view,
            (SIZE + 1, SIZE),
            &cam,
            &light,
            &off,
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
        .is_err());
        assert_eq!(g.read_texture(&t), before);
        assert_eq!(first.bounds(), bounds.0);
        assert_eq!(second.bounds(), bounds.1);
        assert_eq!(scene.depth_generation(), generation);
        assert_eq!(scene.depth_size(), Some(EXTENT));
    }
    let mut wrong = ModelRenderer::new(
        g.clone(),
        TextureFormat::Rgba8UnormSrgb,
        panel([-1.0, -1.0, 1.0, 1.0], IDENTITY, [255; 3], [1.0; 3]),
    )
    .unwrap();
    assert!(draw(
        &mut scene,
        &view,
        (SIZE + 1, SIZE),
        &cam,
        &light,
        &off,
        &mut [
            ImportedBatch::Skinned {
                renderer: &mut first,
                instances: &changed
            },
            ImportedBatch::Static(&mut wrong),
        ]
    )
    .is_err());
    let invalid = PointLightSettings {
        point_light: Some(PointLight {
            position: [0.0; 3],
            color: [1.0; 3],
            intensity: f32::NAN,
            range: 1.0,
        }),
    };
    assert!(draw(
        &mut scene,
        &view,
        (SIZE + 1, SIZE),
        &cam,
        &light,
        &invalid,
        &mut [
            ImportedBatch::Skinned {
                renderer: &mut first,
                instances: &changed
            },
            ImportedBatch::Skinned {
                renderer: &mut second,
                instances: &initial_right
            },
        ]
    )
    .is_err());
    assert_eq!(g.read_texture(&t), before);
    assert_eq!(first.bounds(), bounds.0);
    assert_eq!(second.bounds(), bounds.1);
    assert_eq!(scene.depth_generation(), generation);
    // The valid work prepared before a later failure must not leak into recovery.
    draw(
        &mut scene,
        &view,
        EXTENT,
        &cam,
        &light,
        &off,
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
    assert_eq!(g.read_texture(&t), before);
}

#[test]
fn bounded_batch_and_aggregate_draw_limits_accept_boundary_and_reject_without_effects() {
    use orr_render::imported_scene::{MAX_IMPORTED_BATCHES, MAX_IMPORTED_DRAWS};
    let Some(g) = gpu() else { return };
    assert_eq!(MAX_IMPORTED_BATCHES, 256);
    assert_eq!(MAX_IMPORTED_DRAWS, 4096);
    let format = TextureFormat::Rgba8Unorm;
    let t = target(&g, format, EXTENT);
    let view = g.create_texture_view(&t, None);
    let mut scene = scene(&g, format);
    let cam = camera();
    let light = lighting(1.0);
    let off = PointLightSettings::default();
    let visible = panel([-1.0, -1.0, 1.0, 1.0], IDENTITY, [230, 50, 120], [1.0; 3]);
    let mut seed = ModelRenderer::new(g.clone(), format, visible).unwrap();
    draw(
        &mut scene,
        &view,
        EXTENT,
        &cam,
        &light,
        &off,
        &mut [ImportedBatch::Static(&mut seed)],
    )
    .unwrap();
    let before = g.read_texture(&t);
    let generation = scene.depth_generation();
    // Put budget-only geometry outside the frustum: this exercises real GPU
    // command submission without making the software adapter shade 4096 layers.
    let hidden = panel(
        [-0.1, -0.1, 0.1, 0.1],
        translated(100.0, 0.0, 0.0),
        [255; 3],
        [1.0; 3],
    );
    {
        let mut renderers: Vec<_> = (0..=MAX_IMPORTED_BATCHES)
            .map(|_| ModelRenderer::new(g.clone(), format, hidden.clone()).unwrap())
            .collect();
        let mut batches: Vec<_> = renderers.iter_mut().map(ImportedBatch::Static).collect();
        assert!(matches!(
            draw(
                &mut scene,
                &view,
                (SIZE + 1, SIZE),
                &cam,
                &light,
                &off,
                &mut batches
            ),
            Err(ImportedSceneError::BatchLimit)
        ));
        assert_eq!(g.read_texture(&t), before);
        assert_eq!(scene.depth_generation(), generation);
        assert_eq!(scene.depth_size(), Some(EXTENT));
        draw(
            &mut scene,
            &view,
            EXTENT,
            &cam,
            &light,
            &off,
            &mut batches[..MAX_IMPORTED_BATCHES],
        )
        .unwrap();
        assert!(g
            .read_texture(&t)
            .chunks_exact(4)
            .all(|p| p == [0, 0, 0, 255]));
    }
    draw(
        &mut scene,
        &view,
        EXTENT,
        &cam,
        &light,
        &off,
        &mut [ImportedBatch::Static(&mut seed)],
    )
    .unwrap();
    let mut source = hidden.source().clone();
    let primitive = source.primitives[0].clone();
    source.primitives = (0..1024)
        .map(|i| {
            let mut p = primitive.clone();
            p.id = format!("{}/budget/{i}", p.id);
            p
        })
        .collect();
    let many = StaticModel::new(source).unwrap();
    let mut renderers: Vec<_> = (0..4)
        .map(|_| ModelRenderer::new(g.clone(), format, many.clone()).unwrap())
        .collect();
    let mut batches: Vec<_> = renderers.iter_mut().map(ImportedBatch::Static).collect();
    batches.push(ImportedBatch::Static(&mut seed));
    assert!(matches!(
        draw(
            &mut scene,
            &view,
            (SIZE + 1, SIZE),
            &cam,
            &light,
            &off,
            &mut batches
        ),
        Err(ImportedSceneError::DrawLimit)
    ));
    assert_eq!(g.read_texture(&t), before);
    assert_eq!(scene.depth_generation(), generation);
    assert_eq!(scene.depth_size(), Some(EXTENT));
    batches.pop();
    draw(&mut scene, &view, EXTENT, &cam, &light, &off, &mut batches).unwrap();
    assert!(g
        .read_texture(&t)
        .chunks_exact(4)
        .all(|p| p == [0, 0, 0, 255]));
}

#[test]
fn point_light_is_added_before_exposure_and_aces_in_linear_and_srgb_targets() {
    let Some(g) = gpu() else { return };
    let cam = camera();
    let model = panel(
        [-1.2, -1.2, 1.2, 1.2],
        IDENTITY,
        [190, 150, 220],
        [0.9, 0.85, 0.8],
    );
    let oracle = [surface(&model, [0.0, 0.0, 1.0])];
    let point = PointLightSettings {
        point_light: Some(PointLight {
            position: [-0.4, 0.3, 1.1],
            color: [1.0, 0.55, 0.2],
            intensity: 3.8,
            range: 4.0,
        }),
    };
    let mut format_images = Vec::new();
    for format in FORMATS {
        let t = target(&g, format, EXTENT);
        let view = g.create_texture_view(&t, None);
        let mut scene = scene(&g, format);
        let mut renderer = ModelRenderer::new(g.clone(), format, model.clone()).unwrap();
        let mut exposures: Vec<Vec<u8>> = Vec::new();
        for exposure in [0.35, 1.6] {
            let light = Lighting {
                direction: [0.1, -0.2, -1.0],
                color: [0.8, 0.6, 0.9],
                intensity: 0.35,
                tonemap: true,
                exposure,
                ..lighting(0.15)
            };
            draw(
                &mut scene,
                &view,
                EXTENT,
                &cam,
                &light,
                &point,
                &mut [ImportedBatch::Static(&mut renderer)],
            )
            .unwrap();
            let pixels = g.read_texture(&t);
            // The scalar oracle sums ambient, sun, and point light, multiplies
            // material/exposure, applies ACES, then encodes once. These mixed,
            // unsaturated channels reject adding point light after either step.
            assert_oracle(&pixels, EXTENT, &cam, &oracle, &light, &point);
            let center = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
            near(
                pixels[center..center + 4].try_into().unwrap(),
                shade([0.0; 3], [0.0, 0.0, 1.0], oracle[0].albedo, &light, &point),
                "ACES point light at exact center",
            );
            exposures.push(pixels);
        }
        let center = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
        assert!(
            exposures[1][center] > exposures[0][center] + 20,
            "exposure must change the actual ACES-mapped point lighting"
        );
        format_images.push(exposures);
    }
    for (linear, srgb) in format_images[0].iter().zip(&format_images[1]) {
        image_near(linear, srgb);
    }
}

#[test]
fn procedural_and_static_instances_mutually_occlude_in_both_orders() {
    use orr_render::{
        Material, RenderList3D, Renderer3D, Settings3D, StaticInstance, IDENTITY_ROT,
    };
    let Some(g) = gpu() else { return };
    let cam = camera();
    let light = lighting(1.0);
    let off = PointLightSettings::default();
    let rect = [-0.4, -0.9, 0.4, 0.9];
    let source = panel(rect, translated(0.2, 0.0, 0.0), [255, 0, 0], [1.0; 3]);
    let instances = [
        StaticInstance {
            translation: [-0.8, 0.0, 0.5],
            ..Default::default()
        },
        StaticInstance {
            translation: [0.4, 0.0, -0.5],
            ..Default::default()
        },
    ];
    let oracle = [
        surface(
            &panel(rect, translated(-0.6, 0.0, 0.5), [255, 0, 0], [1.0; 3]),
            [0.0, 0.0, 1.0],
        ),
        surface(
            &panel(rect, translated(0.6, 0.0, -0.5), [255, 0, 0], [1.0; 3]),
            [0.0, 0.0, 1.0],
        ),
        surface(
            &panel(
                [-1.3, -0.6, 1.3, 0.6],
                translated(0.0, 0.0, 0.1),
                [0, 0, 255],
                [1.0; 3],
            ),
            [0.0, 0.0, 1.0],
        ),
    ];
    for format in FORMATS {
        let t = target(&g, format, EXTENT);
        let view = g.create_texture_view(&t, None);
        let mut scene = scene(&g, format);
        let mut model = ModelRenderer::new(g.clone(), format, source.clone()).unwrap();
        let mut procedural = Renderer3D::with_settings(g.clone(), format, Settings3D::LOW);
        procedural.clear = [0.0, 1.0, 0.0, 1.0];
        model.clear = [1.0, 1.0, 0.0, 1.0];
        let mut list = RenderList3D::new();
        // Its default shadowed lighting must not override coordinator lighting.
        list.cuboid(
            [0.0; 3],
            IDENTITY_ROT,
            [1.3, 0.6, 0.1],
            &Material::new([0.0, 0.0, 1.0]).rough(1.0),
        );
        let mut previous = None;
        for reverse in [false, true] {
            let mut batches = if reverse {
                [
                    ImportedBatch::StaticInstances {
                        renderer: &mut model,
                        instances: &instances,
                    },
                    ImportedBatch::Procedural {
                        renderer: &mut procedural,
                        list: &list,
                    },
                ]
            } else {
                [
                    ImportedBatch::Procedural {
                        renderer: &mut procedural,
                        list: &list,
                    },
                    ImportedBatch::StaticInstances {
                        renderer: &mut model,
                        instances: &instances,
                    },
                ]
            };
            draw(&mut scene, &view, EXTENT, &cam, &light, &off, &mut batches).unwrap();
            let pixels = g.read_texture(&t);
            let counts = assert_oracle(&pixels, EXTENT, &cam, &oracle, &light, &off);
            assert!(
                counts.iter().all(|&n| n > 150),
                "both placements and procedural geometry must survive: {counts:?}"
            );
            if let Some(previous) = previous {
                assert_eq!(
                    previous, pixels,
                    "mixed opaque depth changed with batch order"
                );
            }
            previous = Some(pixels);
        }
        assert_eq!(procedural.last_frame_stats().shadow.passes, 0);
        assert_eq!(procedural.last_frame_stats().attachment_allocations, 0);
        assert_eq!(scene.depth_generation(), 1);
    }
}

#[test]
fn external_static_trs_composes_mirrored_sheared_nodes_and_preserves_standalone() {
    use orr_render::StaticInstance;
    let Some(g) = gpu() else { return };
    let cam = camera();
    let light = lighting(0.2);
    let point = PointLightSettings {
        point_light: Some(PointLight {
            position: [-0.5, 1.0, 2.0],
            color: [0.4, 0.8, 1.0],
            intensity: 1.5,
            range: 4.0,
        }),
    };
    let mut node = translated(0.3, -0.2, 0.1);
    node[0] = [0.8, 0.0, 0.3, 0.0];
    node[1][1] = -1.1;
    node[2][2] = 1.5;
    let angle = 0.35_f32;
    let (s, c) = angle.sin_cos();
    let instance = StaticInstance {
        translation: [-0.2, 0.3, 0.4],
        rotation: [0.0, (angle * 0.5).sin(), 0.0, (angle * 0.5).cos()],
        scale: [1.2, 0.7, 0.9],
    };
    // Independent, explicitly expanded S/R/T * imported node reference.
    let composed = [
        [
            1.2 * 0.8 * c + 0.9 * 0.3 * s,
            0.0,
            -1.2 * 0.8 * s + 0.9 * 0.3 * c,
            0.0,
        ],
        [0.0, -0.7 * 1.1, 0.0, 0.0],
        [0.9 * 1.5 * s, 0.0, 0.9 * 1.5 * c, 0.0],
        [
            -0.2 + 1.2 * 0.3 * c + 0.9 * 0.1 * s,
            0.3 - 0.7 * 0.2,
            0.4 - 1.2 * 0.3 * s + 0.9 * 0.1 * c,
            1.0,
        ],
    ];
    for format in FORMATS {
        let t = target(&g, format, EXTENT);
        let view = g.create_texture_view(&t, None);
        let mut scene = scene(&g, format);
        let original = panel(
            [-0.7, -0.7, 0.7, 0.7],
            node,
            [180, 100, 210],
            [0.8, 0.7, 0.9],
        );
        let mut model = ModelRenderer::new(g.clone(), format, original.clone()).unwrap();
        let mut reference = ModelRenderer::new(
            g.clone(),
            format,
            panel(
                [-0.7, -0.7, 0.7, 0.7],
                composed,
                [180, 100, 210],
                [0.8, 0.7, 0.9],
            ),
        )
        .unwrap();
        draw(
            &mut scene,
            &view,
            EXTENT,
            &cam,
            &light,
            &point,
            &mut [ImportedBatch::Static(&mut reference)],
        )
        .unwrap();
        let expected = g.read_texture(&t);
        draw(
            &mut scene,
            &view,
            EXTENT,
            &cam,
            &light,
            &point,
            &mut [ImportedBatch::StaticInstances {
                renderer: &mut model,
                instances: &[instance],
            }],
        )
        .unwrap();
        image_near(&g.read_texture(&t), &expected);
        assert!(expected.chunks_exact(4).filter(|p| p[0] > 20).count() > 500);
        // A composed draw cannot overwrite the standalone immutable node bindings.
        model.clear = [0.0, 0.0, 0.0, 1.0];
        model.draw(&view, EXTENT, &cam, &light).unwrap();
        let original_pixels = g.read_texture(&t);
        let mut fresh = ModelRenderer::new(g.clone(), format, original).unwrap();
        fresh.clear = model.clear;
        fresh.draw(&view, EXTENT, &cam, &light).unwrap();
        assert_eq!(g.read_texture(&t), original_pixels);
    }
}

#[test]
fn invalid_mixed_frame_preserves_pixels_depth_and_procedural_statistics() {
    use orr_render::{
        Material, ProceduralSceneError, RenderList3D, Renderer3D, Settings3D, StaticInstance,
        StaticInstanceError, IDENTITY_ROT,
    };
    let Some(g) = gpu() else { return };
    let format = FORMATS[0];
    let t = target(&g, format, EXTENT);
    let view = g.create_texture_view(&t, None);
    let mut scene = scene(&g, format);
    let mut model = ModelRenderer::new(
        g.clone(),
        format,
        panel([-0.7, -0.7, 0.7, 0.7], IDENTITY, [255, 0, 0], [1.0; 3]),
    )
    .unwrap();
    let mut procedural = Renderer3D::with_settings(g.clone(), format, Settings3D::LOW);
    let mut list = RenderList3D::new();
    list.cuboid(
        [0.7, 0.0, 0.2],
        IDENTITY_ROT,
        [0.6; 3],
        &Material::new([0.0, 0.0, 1.0]).rough(1.0),
    );
    let light = lighting(1.0);
    let point = PointLightSettings::default();
    let placements = [StaticInstance::default()];
    draw(
        &mut scene,
        &view,
        EXTENT,
        &camera(),
        &light,
        &point,
        &mut [
            ImportedBatch::Procedural {
                renderer: &mut procedural,
                list: &list,
            },
            ImportedBatch::StaticInstances {
                renderer: &mut model,
                instances: &placements,
            },
        ],
    )
    .unwrap();
    let before = g.read_texture(&t);
    let stats = procedural.last_frame_stats();
    let invalid = [StaticInstance {
        rotation: [0.0; 4],
        ..Default::default()
    }];
    list.boxes[0].pos = [-1.0, 0.0, 0.9];
    assert!(matches!(
        draw(
            &mut scene,
            &view,
            EXTENT,
            &camera(),
            &light,
            &point,
            &mut [
                ImportedBatch::Procedural {
                    renderer: &mut procedural,
                    list: &list
                },
                ImportedBatch::StaticInstances {
                    renderer: &mut model,
                    instances: &invalid
                }
            ]
        ),
        Err(ImportedSceneError::StaticInstance(
            StaticInstanceError::InvalidPlacement
        ))
    ));
    assert_eq!(g.read_texture(&t), before);
    assert_eq!(procedural.last_frame_stats(), stats);
    list.boxes[0].pos[0] = f32::NAN;
    let more = [StaticInstance::default(); 3];
    assert!(matches!(
        draw(
            &mut scene,
            &view,
            EXTENT,
            &camera(),
            &light,
            &point,
            &mut [
                ImportedBatch::StaticInstances {
                    renderer: &mut model,
                    instances: &more
                },
                ImportedBatch::Procedural {
                    renderer: &mut procedural,
                    list: &list
                }
            ]
        ),
        Err(ImportedSceneError::Procedural(
            ProceduralSceneError::InvalidInstance
        ))
    ));
    assert_eq!(g.read_texture(&t), before);
    assert_eq!(procedural.last_frame_stats(), stats);
    assert_eq!(scene.depth_size(), Some(EXTENT));
    assert_eq!(scene.depth_generation(), 1);
}

#[cfg(feature = "animation")]
#[test]
fn procedural_static_and_skinned_share_one_depth_across_animation_and_batch_orders() {
    use orr_render::{
        Material, RenderList3D, Renderer3D, Settings3D, StaticInstance, IDENTITY_ROT,
    };
    let Some(g) = gpu() else { return };
    let animated_model = animated_model();
    let poses = [
        animated_model.rest_pose().unwrap(),
        animated_model.sample_clip(0, 1.0).unwrap(),
    ];
    let cam = Camera3D::orthographic([0.0, 1.5, 5.0], [0.0, 1.5, 0.0], 2.2);
    let light = lighting(0.6);
    let off = PointLightSettings::default();
    let fixed_model = panel(
        [-1.1, 0.6, -0.25, 2.3],
        translated(0.0, 0.0, 0.3),
        [220, 65, 35],
        [1.0; 3],
    );
    let procedural_surface = surface(
        &panel(
            [-1.4, 0.4, 1.4, 2.7],
            translated(0.0, 0.0, 0.15),
            [50, 90, 200],
            [1.0; 3],
        ),
        [0.0, 0.0, 1.0],
    );
    for format in FORMATS {
        let t = target(&g, format, EXTENT);
        let view = g.create_texture_view(&t, None);
        let mut scene = scene(&g, format);
        let mut fixed = ModelRenderer::new(g.clone(), format, fixed_model.clone()).unwrap();
        let mut animated =
            SkinnedModelRenderer::new(g.clone(), format, animated_model.clone()).unwrap();
        let mut procedural = Renderer3D::with_settings(g.clone(), format, Settings3D::LOW);
        let mut list = RenderList3D::new();
        list.cuboid(
            [0.0, 1.55, 0.05],
            IDENTITY_ROT,
            [1.4, 1.15, 0.1],
            &Material::new([decode(50), decode(90), decode(200)]).rough(1.0),
        );
        let placements = [StaticInstance::default()];
        let mut frames = Vec::new();
        for (pose, angle) in poses.iter().zip([0.0, std::f32::consts::PI / 3.0]) {
            let moving_surface = animated_surface(&animated_model, angle, IDENTITY);
            let oracle = [
                surface(&fixed_model, [0.0, 0.0, 1.0]),
                procedural_surface.clone(),
                moving_surface.clone(),
            ];
            let instances = [SkinnedInstance::new(pose)];
            let mut previous = None;
            for reverse in [false, true] {
                let mut batches = if reverse {
                    [
                        ImportedBatch::Skinned {
                            renderer: &mut animated,
                            instances: &instances,
                        },
                        ImportedBatch::StaticInstances {
                            renderer: &mut fixed,
                            instances: &placements,
                        },
                        ImportedBatch::Procedural {
                            renderer: &mut procedural,
                            list: &list,
                        },
                    ]
                } else {
                    [
                        ImportedBatch::Procedural {
                            renderer: &mut procedural,
                            list: &list,
                        },
                        ImportedBatch::StaticInstances {
                            renderer: &mut fixed,
                            instances: &placements,
                        },
                        ImportedBatch::Skinned {
                            renderer: &mut animated,
                            instances: &instances,
                        },
                    ]
                };
                draw(&mut scene, &view, EXTENT, &cam, &light, &off, &mut batches).unwrap();
                let pixels = g.read_texture(&t);
                let counts = assert_oracle(&pixels, EXTENT, &cam, &oracle, &light, &off);
                assert!(
                    counts.iter().all(|&count| count > 20),
                    "all three paths must remain visible: {counts:?}"
                );
                assert_bounds(&animated, std::slice::from_ref(&moving_surface));
                if let Some(previous) = previous {
                    assert_eq!(
                        previous, pixels,
                        "three-way shared depth changed with batch order"
                    );
                }
                previous = Some(pixels);
            }
            frames.push(previous.unwrap());
        }
        assert_ne!(
            frames[0], frames[1],
            "animation must change actual composed visibility"
        );
    }
}

#[test]
fn procedural_point_light_matches_diffuse_oracle_and_off_preserves_standalone_pixels() {
    use orr_render::{Material, RenderList3D, Renderer3D, Settings3D, IDENTITY_ROT};
    let Some(g) = gpu() else { return };
    let cam = camera();
    let light = lighting(0.2);
    let off = PointLightSettings::default();
    let on = PointLightSettings {
        point_light: Some(PointLight {
            position: [-0.2, 0.3, 1.5],
            color: [0.5, 0.8, 1.0],
            intensity: 1.2,
            range: 3.5,
        }),
    };
    let oracle = [surface(
        &panel(
            [-0.9, -0.9, 0.9, 0.9],
            translated(0.0, 0.0, 0.1),
            [160, 190, 220],
            [1.0; 3],
        ),
        [0.0, 0.0, 1.0],
    )];
    for format in FORMATS {
        let t = target(&g, format, EXTENT);
        let view = g.create_texture_view(&t, None);
        let mut scene = scene(&g, format);
        let mut renderer = Renderer3D::with_settings(g.clone(), format, Settings3D::LOW);
        renderer.clear = scene.clear;
        let mut list = RenderList3D::new();
        list.lighting = light;
        list.cuboid(
            [0.0; 3],
            IDENTITY_ROT,
            [0.9, 0.9, 0.1],
            &Material::new([decode(160), decode(190), decode(220)]).rough(1.0),
        );
        draw(
            &mut scene,
            &view,
            EXTENT,
            &cam,
            &light,
            &off,
            &mut [ImportedBatch::Procedural {
                renderer: &mut renderer,
                list: &list,
            }],
        )
        .unwrap();
        let baseline = g.read_texture(&t);
        assert_oracle(&baseline, EXTENT, &cam, &oracle, &light, &off);
        renderer.draw(&view, EXTENT, &list, &cam);
        assert_eq!(
            g.read_texture(&t),
            baseline,
            "point-off composed output must preserve standalone procedural shading"
        );
        draw(
            &mut scene,
            &view,
            EXTENT,
            &cam,
            &light,
            &on,
            &mut [ImportedBatch::Procedural {
                renderer: &mut renderer,
                list: &list,
            }],
        )
        .unwrap();
        let lit = g.read_texture(&t);
        assert_oracle(&lit, EXTENT, &cam, &oracle, &light, &on);
        assert_ne!(
            lit, baseline,
            "point light must change actual procedural pixels"
        );
    }
}
