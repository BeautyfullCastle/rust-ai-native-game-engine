//! Mandatory software-GPU SH9 acceptance. No ignored tests or adapter skips.
//!
//! Expected pixels use analytic polynomials, original mesh coordinates and scalar
//! skinning, never IrradianceGrid::sample, packed uniforms or renderer SH helpers.
//! Readback is the RGBA16F scene before exposure, tonemapping, gamma and bloom.
#![cfg(feature = "irradiance-probes")]
#![allow(clippy::float_arithmetic)]

use std::sync::{Mutex, MutexGuard};

use orr_model::{IDENTITY, StaticModel, Vertex};
#[cfg(feature = "animation")]
use orr_model::{
    animation::{AnimatedModel, ChannelValues},
    animation_import,
};
use orr_render::orr_rhi::{Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions};
use orr_render::{
    Camera3D, ImportedBatch, ImportedSceneError, ImportedSceneRenderer, ImportedSceneTarget,
    IrradianceGrid, IrradianceProvenance, Lighting, Material, ModelRenderer, PointLight,
    PointLightSettings, RenderList3D, Renderer3D, Settings3D, StaticInstance,
};
#[cfg(feature = "animation")]
use orr_render::{SkinnedInstance, SkinnedModelRenderer};

type Texture = <Wgpu as Rhi>::Texture;
type View = <Wgpu as Rhi>::TextureView;
type Matrix = [[f32; 4]; 4];
const SIZE: (u32, u32) = (193, 193);
const FORMAT: TextureFormat = TextureFormat::Rgba8UnormSrgb;
const PI: f64 = std::f64::consts::PI;
static GPU_LOCK: Mutex<()> = Mutex::new(());

fn gpu() -> (MutexGuard<'static, ()>, Wgpu) {
    let lock = GPU_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let g = Wgpu::headless(WgpuOptions {
        force_software: true,
        ..Default::default()
    })
    .expect("SH9 acceptance requires an actual software GPU; adapter absence is a failure");
    assert!(
        g.is_software(),
        "the mandatory SH9 oracle must run on a software adapter"
    );
    let info = g.adapter().get_info();
    eprintln!(
        "SH9 acceptance adapter: name={:?} backend={:?} driver={:?} software={}",
        info.name,
        info.backend,
        info.driver_info,
        g.is_software()
    );
    (lock, g)
}

fn camera() -> Camera3D {
    Camera3D::orthographic([0.0, 0.0, 8.0], [0.0; 3], 2.3)
}

fn light(ambient: f32) -> Lighting {
    Lighting {
        sky: [1.0; 3],
        ground: [1.0; 3],
        ambient,
        intensity: 0.0,
        shadows: false,
        exposure: 1.0,
        tonemap: false,
        ..Default::default()
    }
}

fn target(g: &Wgpu, format: TextureFormat, size: (u32, u32)) -> Texture {
    g.create_texture(&TextureDesc {
        label: "SH9 acceptance display",
        width: size.0,
        height: size.1,
        format,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    })
}

fn scene(g: &Wgpu, format: TextureFormat, hdr: bool) -> ImportedSceneRenderer<Wgpu> {
    let mut s = ImportedSceneRenderer::new(g.clone(), format).unwrap();
    assert!(s.irradiance.is_none(), "irradiance must be opt-in");
    s.post_process.enabled = hdr;
    s.clear = [0.0, 0.0, 0.0, 1.0];
    s
}

#[allow(clippy::too_many_arguments)]
fn draw(
    s: &mut ImportedSceneRenderer<Wgpu>,
    view: &View,
    size: (u32, u32),
    camera: &Camera3D,
    light: &Lighting,
    points: &PointLightSettings,
    batches: &mut [ImportedBatch<'_, Wgpu>],
) -> Result<(), ImportedSceneError> {
    let format = s.format();
    s.draw(
        ImportedSceneTarget {
            view,
            size,
            format,
            sample_count: 1,
        },
        camera,
        light,
        points,
        batches,
    )
}

fn hdr(s: &ImportedSceneRenderer<Wgpu>) -> Vec<[f32; 4]> {
    let pixels = s.read_hdr_rgba().expect("accepted HDR scene readback");
    assert_eq!(pixels.len(), (SIZE.0 * SIZE.1) as usize);
    for (i, p) in pixels.iter().enumerate() {
        assert!(
            p.iter()
                .all(|v| v.is_finite() && *v >= 0.0 && *v <= 65504.0),
            "nonfinite/out-of-range HDR texel {i}: {p:?}"
        );
        assert_eq!(p[3], 1.0, "opaque HDR alpha at {i}");
    }
    pixels
}

fn near(actual: [f32; 4], expected: [f64; 3], label: &str) {
    for k in 0..3 {
        // At most a few half-float ULPs plus scalar interpolation roundoff.
        let tolerance = 0.000_08_f64.max(expected[k].abs() * 0.0025);
        assert!(
            (f64::from(actual[k]) - expected[k]).abs() <= tolerance,
            "{label}: channel {k}, actual={actual:?}, expected={expected:?}, tolerance={tolerance}"
        );
    }
    assert_eq!(actual[3], 1.0);
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.map(|x| x / norm)
}
fn transform(m: Matrix, p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|k| m[0][k] * p[0] + m[1][k] * p[1] + m[2][k] * p[2] + m[3][k])
}
fn rotate(p: [f32; 3], angles: [f32; 2]) -> [f32; 3] {
    let (sx, cx) = angles[0].sin_cos();
    let (sy, cy) = angles[1].sin_cos();
    let q = [p[0], cx * p[1] - sx * p[2], sx * p[1] + cx * p[2]];
    [cy * q[0] + sy * q[2], q[1], -sy * q[0] + cy * q[2]]
}
fn quaternion(angles: [f32; 2]) -> [f32; 4] {
    let (sx, cx) = (angles[0] * 0.5).sin_cos();
    let (sy, cy) = (angles[1] * 0.5).sin_cos();
    [sx * cy, cx * sy, -sx * sy, cx * cy]
}
fn placement(position: [f32; 3], scale: [f32; 3], angles: [f32; 2]) -> Matrix {
    let mut m = IDENTITY;
    for i in 0..3 {
        let mut axis = [0.0; 3];
        axis[i] = scale[i];
        m[i][..3].copy_from_slice(&rotate(axis, angles));
    }
    m[3][..3].copy_from_slice(&position);
    m
}

fn panel(half: [f32; 2], normal: [f32; 3], albedo: [f32; 3]) -> StaticModel {
    let imported = orr_model::import::import_with_resolver(
        "fixtures/model.glb",
        include_bytes!("../../orr_model/tests/fixtures/model.glb"),
        |_| panic!("embedded fixture"),
    )
    .unwrap();
    let mut source = imported.source().clone();
    source.primitives.truncate(1);
    let p = &mut source.primitives[0];
    p.vertices = [
        [-half[0], -half[1], 0.0],
        [half[0], -half[1], 0.0],
        [half[0], half[1], 0.0],
        [-half[0], half[1], 0.0],
    ]
    .into_iter()
    .map(|position| Vertex {
        position,
        normal,
        uv: [0.25; 2],
    })
    .collect();
    p.indices = vec![0, 1, 2, 0, 2, 3];
    p.transform = IDENTITY;
    p.material = 0;
    for m in &mut source.materials {
        m.base_color = [albedo[0], albedo[1], albedo[2], 1.0];
    }
    for image in &mut source.images {
        image.rgba8.fill(255);
    }
    let model = StaticModel::new(source).unwrap();
    StaticModel::from_bytes(&model.to_bytes().unwrap()).unwrap()
}

#[derive(Clone)]
struct Surface {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    indices: Vec<u32>,
    albedo: [f32; 3],
}
fn panel_surface(
    model: &StaticModel,
    position: [f32; 3],
    scale: [f32; 3],
    angles: [f32; 2],
) -> Surface {
    let p = &model.source().primitives[0];
    let m = placement(position, scale, angles);
    Surface {
        positions: p
            .vertices
            .iter()
            .map(|v| transform(m, v.position))
            .collect(),
        normals: p
            .vertices
            .iter()
            .map(|v| {
                unit(rotate(
                    std::array::from_fn(|k| v.normal[k] / scale[k]),
                    angles,
                ))
            })
            .collect(),
        indices: p.indices.clone(),
        albedo: model.source().materials[0].base_color[..3]
            .try_into()
            .unwrap(),
    }
}
fn box_surfaces(
    position: [f32; 3],
    half: [f32; 3],
    angles: [f32; 2],
    albedo: [f32; 3],
) -> Vec<Surface> {
    let m = placement(position, half, angles);
    let mut result = Vec::new();
    // Hand-authored six analytic box faces, independent of procedural mesh code.
    for (n, u, v) in [
        ([1., 0., 0.], [0., 1., 0.], [0., 0., 1.]),
        ([-1., 0., 0.], [0., 0., 1.], [0., 1., 0.]),
        ([0., 1., 0.], [0., 0., 1.], [1., 0., 0.]),
        ([0., -1., 0.], [1., 0., 0.], [0., 0., 1.]),
        ([0., 0., 1.], [1., 0., 0.], [0., 1., 0.]),
        ([0., 0., -1.], [0., 1., 0.], [1., 0., 0.]),
    ] {
        let normal = rotate(n, angles);
        if normal[2] <= 0.0001 {
            continue;
        } // Orthographic +Z eye, backface culling.
        let positions = [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)]
            .into_iter()
            .map(|(a, b)| transform(m, std::array::from_fn(|k| n[k] + a * u[k] + b * v[k])))
            .collect();
        result.push(Surface {
            positions,
            normals: vec![normal; 4],
            indices: vec![0, 1, 2, 0, 2, 3],
            albedo,
        });
    }
    result
}

/// SH normalizers derived from pi, instead of copying the shader's constants.
fn normalizers() -> [f64; 9] {
    [
        (1.0 / (4.0 * PI)).sqrt(),
        (3.0 / (4.0 * PI)).sqrt(),
        (3.0 / (4.0 * PI)).sqrt(),
        (3.0 / (4.0 * PI)).sqrt(),
        (15.0 / (4.0 * PI)).sqrt(),
        (15.0 / (4.0 * PI)).sqrt(),
        (5.0 / (16.0 * PI)).sqrt(),
        (15.0 / (4.0 * PI)).sqrt(),
        (15.0 / (16.0 * PI)).sqrt(),
    ]
}

// Each row is an outgoing-radiance polynomial coefficient in RGB. Converting
// to stored E requires exactly pi, while the renderer must divide by pi once.
const DIRECTIONAL: [[f64; 3]; 9] = [
    [0.8, 0.25, 0.2],
    [0.25, -0.7, 0.05],
    [0.15, 0.08, -0.9],
    [-0.3, 0.09, 0.12],
    [0.18, -0.1, 0.09],
    [-0.12, 0.15, -0.2],
    [0.1, -0.09, 0.06],
    [-0.11, 0.13, 0.16],
    [0.07, -0.06, -0.08],
];
// Adjacent x=+/-2 probes contain negative E while every tested interior
// surface remains positive: clamping per-node before interpolation is wrong.
fn spatial(p: [f32; 3]) -> [f64; 3] {
    let [x, y, z] = p.map(f64::from);
    [
        1.1 - 0.65 * x + 0.07 * y + 0.03 * z,
        0.4 + 0.04 * y - 0.02 * z,
        1.1 + 0.65 * x - 0.05 * y + 0.08 * z,
    ]
}
fn directional(n: [f32; 3]) -> [f64; 3] {
    let [x, y, z] = unit(n).map(f64::from);
    let terms = [
        1.0,
        y,
        z,
        x,
        x * y,
        y * z,
        3.0 * z * z - 1.0,
        x * z,
        x * x - y * y,
    ];
    std::array::from_fn(|k| {
        (0..9)
            .map(|i| DIRECTIONAL[i][k] * terms[i])
            .sum::<f64>()
            .max(0.0)
    })
}
fn grid_from_polynomial(mut polynomial: impl FnMut([f32; 3]) -> [[f64; 3]; 9]) -> IrradianceGrid {
    let mut coefficients = Vec::new();
    let norms = normalizers();
    for z in 0..4 {
        for y in 0..4 {
            for x in 0..4 {
                let p = [x, y, z].map(|v| -6.0 + 4.0 * v as f32);
                let terms = polynomial(p);
                coefficients.push(std::array::from_fn(|i| {
                    terms[i].map(|v| (PI * v / norms[i]) as f32)
                }));
            }
        }
    }
    let grid = IrradianceGrid {
        version: 1,
        enabled: true,
        provenance: IrradianceProvenance::Authored,
        dimensions: [4; 3],
        origin: [-6.0; 3],
        spacing: [4.0; 3],
        coefficients,
    };
    grid.validate().unwrap();
    grid
}
fn spatial_grid() -> IrradianceGrid {
    grid_from_polynomial(|p| {
        let mut c = [[0.0; 3]; 9];
        c[0] = spatial(p);
        c
    })
}
fn constant_grid(radiance: [f64; 3]) -> IrradianceGrid {
    grid_from_polynomial(|_| {
        let mut c = [[0.0; 3]; 9];
        c[0] = radiance;
        c
    })
}

fn barycentric(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> [f32; 3] {
    let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    let u = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / d;
    let v = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / d;
    [u, v, 1.0 - u - v]
}

/// Rasterize only confidently interior pixels, require every object's coverage,
/// and independently check the clear region. No "some pixels changed" oracle.
fn assert_oracle(
    pixels: &[[f32; 4]],
    surfaces: &[Surface],
    shade: impl Fn([f32; 3], [f32; 3]) -> [f64; 3],
) -> Vec<usize> {
    let cam = camera();
    let screen: Vec<Vec<_>> = surfaces
        .iter()
        .map(|s| {
            s.positions
                .iter()
                .map(|&p| cam.world_to_screen(p, SIZE).unwrap())
                .collect()
        })
        .collect();
    let mut counts = vec![0; surfaces.len()];
    let mut clear = 0;
    for y in 0..SIZE.1 {
        for x in 0..SIZE.0 {
            let mut closest: Option<(usize, [f32; 3], [f32; 3], bool)> = None;
            let mut edge = false;
            for (si, s) in surfaces.iter().enumerate() {
                for tri in s.indices.chunks_exact(3) {
                    let ids = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
                    let b = barycentric(
                        [x as f32 + 0.5, y as f32 + 0.5],
                        screen[si][ids[0]],
                        screen[si][ids[1]],
                        screen[si][ids[2]],
                    );
                    edge |= b.iter().all(|v| *v >= -0.05);
                    if b.iter().any(|v| *v < 0.0) {
                        continue;
                    }
                    let p = std::array::from_fn(|k| {
                        (0..3).map(|i| b[i] * s.positions[ids[i]][k]).sum()
                    });
                    let n =
                        std::array::from_fn(|k| (0..3).map(|i| b[i] * s.normals[ids[i]][k]).sum());
                    if closest.as_ref().is_none_or(|(_, q, _, _)| p[2] > q[2]) {
                        closest = Some((si, p, n, b.iter().all(|v| *v > 0.055)));
                    }
                }
            }
            let actual = pixels[(y * SIZE.0 + x) as usize];
            if let Some((si, p, n, true)) = closest {
                let incoming = shade(p, unit(n));
                let expected =
                    std::array::from_fn(|k| incoming[k] * f64::from(surfaces[si].albedo[k]));
                near(
                    actual,
                    expected,
                    &format!("surface {si}, pixel {x},{y}, p={p:?}, n={n:?}"),
                );
                counts[si] += 1;
            } else if closest.is_none() && !edge {
                near(actual, [0.0; 3], "clear background");
                clear += 1;
            }
        }
    }
    assert!(
        clear > 1500,
        "missing independently checked background: {clear}"
    );
    assert!(
        counts.iter().sum::<usize>() > 400,
        "insufficient interior samples: {counts:?}"
    );
    counts
}

#[test]
fn procedural_and_static_pixels_follow_world_space_red_blue_irradiance_with_one_pi() {
    let (_lock, g) = gpu();
    for format in [TextureFormat::Rgba8Unorm, TextureFormat::Rgba8UnormSrgb] {
        let t = target(&g, format, SIZE);
        let view = g.create_texture_view(&t, None);
        let mut s = scene(&g, format, true);
        s.irradiance = Some(spatial_grid());
        let asset = panel([0.36, 0.36], [0., 0., 1.], [0.7, 0.4, 0.9]);
        let mut model = ModelRenderer::new(g.clone(), s.scene_format(), asset.clone()).unwrap();
        let mut procedural =
            Renderer3D::with_settings(g.clone(), s.scene_format(), Settings3D::LOW);
        let mut light = light(2.4); // Must be REPLACED, not added to the probe term.
        light.exposure = 0.23;
        light.tonemap = true;
        let mut baseline_hdr = None;
        for swapped in [false, true] {
            let sign = if swapped { -1.0 } else { 1.0 };
            let mut list = RenderList3D::new();
            let mut surfaces = Vec::new();
            let mut instances = Vec::new();
            for x in [-1.0, 1.0] {
                let pos = [sign * x, 0.65, 0.0];
                instances.push(StaticInstance {
                    translation: pos,
                    ..Default::default()
                });
                surfaces.push(panel_surface(&asset, pos, [1.0; 3], [0.0; 2]));
                let pos = [sign * x, -0.65, 0.0];
                list.cuboid(
                    pos,
                    quaternion([0.0; 2]),
                    [0.36, 0.36, 0.15],
                    &Material::new([0.7, 0.4, 0.9]).rough(1.0),
                );
                surfaces.extend(box_surfaces(
                    pos,
                    [0.36, 0.36, 0.15],
                    [0.0; 2],
                    [0.7, 0.4, 0.9],
                ));
            }
            draw(
                &mut s,
                &view,
                SIZE,
                &camera(),
                &light,
                &PointLightSettings::default(),
                &mut [
                    ImportedBatch::Procedural {
                        renderer: &mut procedural,
                        list: &list,
                    },
                    ImportedBatch::StaticInstances {
                        renderer: &mut model,
                        instances: &instances,
                    },
                ],
            )
            .unwrap();
            let pixels = hdr(&s);
            let counts = assert_oracle(&pixels, &surfaces, |p, _| spatial(p));
            assert!(
                counts.iter().all(|v| *v > 100),
                "all four red/blue placements: {counts:?}"
            );
            if let Some(before) = &baseline_hdr {
                assert_eq!(&pixels, before, "instance order cannot change irradiance");
            }
            baseline_hdr = Some(pixels);
        }
        // Changing every display setting must leave the pre-display radiometry exact.
        light.exposure = 3.0;
        light.tonemap = false;
        s.post_process.bloom = true;
        let instances = [
            StaticInstance {
                translation: [-1.0, 0.65, 0.0],
                ..Default::default()
            },
            StaticInstance {
                translation: [1.0, 0.65, 0.0],
                ..Default::default()
            },
        ];
        let mut list = RenderList3D::new();
        for x in [-1.0, 1.0] {
            list.cuboid(
                [x, -0.65, 0.0],
                quaternion([0.0; 2]),
                [0.36, 0.36, 0.15],
                &Material::new([0.7, 0.4, 0.9]).rough(1.0),
            );
        }
        draw(
            &mut s,
            &view,
            SIZE,
            &camera(),
            &light,
            &PointLightSettings::default(),
            &mut [
                ImportedBatch::Procedural {
                    renderer: &mut procedural,
                    list: &list,
                },
                ImportedBatch::StaticInstances {
                    renderer: &mut model,
                    instances: &instances,
                },
            ],
        )
        .unwrap();
        assert_eq!(
            hdr(&s),
            baseline_hdr.unwrap(),
            "HDR SH is linear before exposure/gamma/bloom"
        );
    }
}

#[test]
fn signed_sh9_uses_transformed_world_normals_and_clamps_only_after_reconstruction() {
    let (_lock, g) = gpu();
    let t = target(&g, FORMAT, SIZE);
    let view = g.create_texture_view(&t, None);
    let mut s = scene(&g, FORMAT, true);
    s.irradiance = Some(grid_from_polynomial(|_| DIRECTIONAL));
    let asset = panel([0.55, 0.55], unit([0.35, 0.4, 0.8]), [0.8, 0.6, 0.9]);
    let mut model = ModelRenderer::new(g.clone(), s.scene_format(), asset.clone()).unwrap();
    let mut procedural = Renderer3D::with_settings(g.clone(), s.scene_format(), Settings3D::LOW);
    for angles in [[0.65, 0.55], [-0.75, -0.45], [0.0, 0.0]] {
        let scale = [0.65, 1.15, 0.45];
        let instances = [StaticInstance {
            translation: [-0.85, 0.0, 0.0],
            rotation: quaternion(angles),
            scale,
        }];
        let mut list = RenderList3D::new();
        list.cuboid(
            [0.8, 0., 0.],
            quaternion(angles),
            [0.55, 0.55, 0.35],
            &Material::new([0.8, 0.6, 0.9]).rough(1.0),
        );
        let mut surfaces = vec![panel_surface(&asset, [-0.85, 0., 0.], scale, angles)];
        surfaces.extend(box_surfaces(
            [0.8, 0., 0.],
            [0.55, 0.55, 0.35],
            angles,
            [0.8, 0.6, 0.9],
        ));
        draw(
            &mut s,
            &view,
            SIZE,
            &camera(),
            &light(1.7),
            &PointLightSettings::default(),
            &mut [
                ImportedBatch::StaticInstances {
                    renderer: &mut model,
                    instances: &instances,
                },
                ImportedBatch::Procedural {
                    renderer: &mut procedural,
                    list: &list,
                },
            ],
        )
        .unwrap();
        let counts = assert_oracle(&hdr(&s), &surfaces, |_, n| directional(n));
        assert!(
            counts[0] > 100,
            "transformed static normal oracle: {counts:?}"
        );
        assert!(
            counts[1..].iter().sum::<usize>() > 250,
            "rotated procedural normals: {counts:?}"
        );
    }
}

#[test]
fn disabled_outside_and_boundary_preserve_exact_legacy_pixels_and_fade_is_linear() {
    let (_lock, g) = gpu();
    for hdr_enabled in [false, true] {
        let t = target(&g, FORMAT, SIZE);
        let view = g.create_texture_view(&t, None);
        let mut s = scene(&g, FORMAT, hdr_enabled);
        let asset = panel([1.6, 0.7], [0., 0., 1.], [0.6, 0.8, 0.4]);
        let mut model = ModelRenderer::new(g.clone(), s.scene_format(), asset.clone()).unwrap();
        let lighting = light(0.35);
        draw(
            &mut s,
            &view,
            SIZE,
            &camera(),
            &lighting,
            &PointLightSettings::default(),
            &mut [ImportedBatch::Static(&mut model)],
        )
        .unwrap();
        let display = g.read_texture(&t);
        let linear = s.read_hdr_rgba();
        let generations = (
            s.depth_generation(),
            s.post_process_generation(),
            s.shadow_map_generation(),
            s.post_process_allocated_bytes(),
        );
        let mut disabled = constant_grid([2., 3., 4.]);
        disabled.enabled = false;
        let mut outside = constant_grid([2., 3., 4.]);
        outside.origin = [30.; 3];
        let mut boundary_min = constant_grid([2., 3., 4.]);
        boundary_min.origin[2] = 0.0;
        let mut boundary_max = constant_grid([2., 3., 4.]);
        boundary_max.origin[2] = -12.0;
        // 0.001 has a nonbinary reciprocal: without exact world-max handling,
        // (-origin) * (1 / spacing) is 2.9999998 instead of 3 at z=0.
        // Bright legal SH amplifies the erroneous tiny fade into an HDR ULP.
        let mut nonbinary_max = constant_grid([800.; 3]);
        nonbinary_max.spacing[2] = 0.001;
        nonbinary_max.origin[2] = -(3.0 * nonbinary_max.spacing[2]);
        assert_eq!(
            nonbinary_max.origin[2] + 3.0 * nonbinary_max.spacing[2],
            0.0
        );
        assert!((-nonbinary_max.origin[2]) * (1.0 / nonbinary_max.spacing[2]) < 3.0);
        for grid in [disabled, outside, boundary_min, boundary_max, nonbinary_max] {
            s.irradiance = Some(grid);
            draw(
                &mut s,
                &view,
                SIZE,
                &camera(),
                &lighting,
                &PointLightSettings::default(),
                &mut [ImportedBatch::Static(&mut model)],
            )
            .unwrap();
            assert_eq!(
                g.read_texture(&t),
                display,
                "disabled/outside/exact boundary must be byte-identical"
            );
            assert_eq!(
                s.read_hdr_rgba(),
                linear,
                "legacy HDR fallback must be exact"
            );
            assert_eq!(
                (
                    s.depth_generation(),
                    s.post_process_generation(),
                    s.shadow_map_generation(),
                    s.post_process_allocated_bytes()
                ),
                generations,
                "toggles must not replace resources"
            );
        }
        if hdr_enabled {
            let mut grid = constant_grid([1.3, 0.15, 2.2]);
            grid.origin = [-3.; 3];
            grid.spacing = [2.; 3];
            s.irradiance = Some(grid);
            draw(
                &mut s,
                &view,
                SIZE,
                &camera(),
                &lighting,
                &PointLightSettings::default(),
                &mut [ImportedBatch::Static(&mut model)],
            )
            .unwrap();
            let surfaces = [panel_surface(&asset, [0.; 3], [1.; 3], [0.; 2])];
            assert_oracle(&hdr(&s), &surfaces, |p, _| {
                // Grid central region is [-1,1], then one-cell fade to x=+/-3.
                let weight = ((3.0 - f64::from(p[0]).abs()) / 2.0).clamp(0.0, 1.0);
                [1.3, 0.15, 2.2].map(|v| 0.35 * (1.0 - weight) + v * weight)
            });
        }
    }
}

#[test]
fn probes_preserve_sun_point_and_real_shadow_terms_and_keep_hdr_finite() {
    let (_lock, g) = gpu();
    let t = target(&g, FORMAT, SIZE);
    let view = g.create_texture_view(&t, None);
    let mut s = scene(&g, FORMAT, true);
    let asset = panel([1.7, 1.7], [0., 0., 1.], [1.; 3]);
    let mut model = ModelRenderer::new(g.clone(), s.scene_format(), asset.clone()).unwrap();
    let mut procedural = Renderer3D::with_settings(g.clone(), s.scene_format(), Settings3D::LOW);
    let mut list = RenderList3D::new();
    list.cuboid(
        [0., 0., 0.8],
        quaternion([0.; 2]),
        [0.3, 0.4, 0.15],
        &Material::new([1.; 3]).rough(1.0),
    );
    let mut lighting = light(0.125);
    lighting.direction = [0.65, -0.4, -1.0];
    lighting.color = [0.8, 0.6, 0.4];
    lighting.intensity = 1.3;
    lighting.shadow_radius = 3.0;
    lighting.shadow_center = [0.; 3];
    let point = PointLightSettings {
        point_light: Some(PointLight {
            position: [-0.4, 0.7, 1.6],
            color: [0.2, 0.6, 1.0],
            intensity: 1.7,
            range: 4.0,
        }),
    };
    let mut direct_frames = Vec::new();
    for (shadows, points) in [
        (false, PointLightSettings::default()),
        (false, point.clone()),
        (true, point.clone()),
    ] {
        lighting.shadows = shadows;
        s.irradiance = None;
        draw(
            &mut s,
            &view,
            SIZE,
            &camera(),
            &lighting,
            &points,
            &mut [
                ImportedBatch::Static(&mut model),
                ImportedBatch::Procedural {
                    renderer: &mut procedural,
                    list: &list,
                },
            ],
        )
        .unwrap();
        let before = hdr(&s);
        s.irradiance = Some(constant_grid([0.5, 0.875, 1.25]));
        draw(
            &mut s,
            &view,
            SIZE,
            &camera(),
            &lighting,
            &points,
            &mut [
                ImportedBatch::Static(&mut model),
                ImportedBatch::Procedural {
                    renderer: &mut procedural,
                    list: &list,
                },
            ],
        )
        .unwrap();
        let after = hdr(&s);
        let mut checked = 0;
        for (i, (a, b)) in before.iter().zip(&after).enumerate() {
            if a[..3].iter().any(|v| *v > 0.0) {
                let expected = std::array::from_fn(|k| f64::from(a[k]) + [0.375, 0.75, 1.125][k]);
                near(
                    *b,
                    expected,
                    &format!("direct/shadow preservation pixel {i}"),
                );
                checked += 1;
            } else {
                assert_eq!(a, b, "clear pixels must stay clear");
            }
        }
        assert!(
            checked > 10000,
            "substantial direct/shadow coverage required"
        );
        direct_frames.push(before);
    }
    // Independent scene-linear direct-light oracle for the receiver outside
    // the occluder's silhouette. The comparison above additionally proves GI
    // leaves the genuinely shadowed values unchanged, without guessing PCF.
    let sun_z = -f64::from(lighting.direction[2])
        / lighting
            .direction
            .iter()
            .map(|v| f64::from(*v).powi(2))
            .sum::<f64>()
            .sqrt();
    let point_light = point.point_light.unwrap();
    let mut receiver_samples = 0;
    for y in 0..SIZE.1 {
        for x in 0..SIZE.0 {
            let wx = (f64::from(x) + 0.5) / f64::from(SIZE.0) * 4.6 - 2.3;
            let wy = 2.3 - (f64::from(y) + 0.5) / f64::from(SIZE.1) * 4.6;
            if wx.abs() > 1.6 || wy.abs() > 1.6 || (wx.abs() < 0.4 && wy.abs() < 0.5) {
                continue;
            }
            let delta = [
                f64::from(point_light.position[0]) - wx,
                f64::from(point_light.position[1]) - wy,
                f64::from(point_light.position[2]),
            ];
            let distance = delta.iter().map(|v| v * v).sum::<f64>().sqrt();
            let attenuation = (1.0 - distance / f64::from(point_light.range))
                .max(0.0)
                .powi(2);
            for (frame_index, frame) in direct_frames[..2].iter().enumerate() {
                let expected = std::array::from_fn(|k| {
                    let sun = f64::from(lighting.color[k]) * f64::from(lighting.intensity) * sun_z;
                    let local = if frame_index == 0 {
                        0.0
                    } else {
                        f64::from(point_light.color[k])
                            * f64::from(point_light.intensity)
                            * (delta[2] / distance).max(0.0)
                            * attenuation
                    };
                    0.125 + sun + local
                });
                near(
                    frame[(y * SIZE.0 + x) as usize],
                    expected,
                    "analytic sun/point receiver",
                );
            }
            receiver_samples += 1;
        }
    }
    assert!(
        receiver_samples > 10_000,
        "substantial direct-light radiometry coverage"
    );
    assert!(
        direct_frames[0]
            .iter()
            .zip(&direct_frames[1])
            .filter(|(a, b)| b[2] - a[2] > 0.04)
            .count()
            > 1000,
        "point light must actually contribute"
    );
    assert!(
        direct_frames[1]
            .iter()
            .zip(&direct_frames[2])
            .filter(|(a, b)| a[0] - b[0] > 0.08)
            .count()
            > 50,
        "real occluder must create a sun shadow"
    );
    assert_eq!(
        s.shadow_map_generation(),
        1,
        "probe toggles reuse the shadow map"
    );
    // Maximum legal signed coefficients and strong direct illumination remain
    // finite in scene HDR, even though the display is fully tone-mapped white.
    let mut maximum = constant_grid([0.; 3]);
    maximum.coefficients.fill([[10_000.; 3]; 9]);
    s.irradiance = Some(maximum);
    lighting.intensity = 10_000.;
    lighting.color = [10_000.; 3];
    lighting.exposure = 10_000.;
    lighting.tonemap = true;
    draw(
        &mut s,
        &view,
        SIZE,
        &camera(),
        &lighting,
        &point,
        &mut [ImportedBatch::Static(&mut model)],
    )
    .unwrap();
    let pixels = hdr(&s);
    assert!(
        pixels.iter().any(|p| p[0] == 65504.),
        "HDR clamps at representation range, not display white"
    );
}

#[cfg(feature = "animation")]
fn animated() -> AnimatedModel {
    let imported = animation_import::import_with_resolver(
        "fixtures/animated_strip.glb",
        include_bytes!("../../orr_model/tests/fixtures/animated_strip.glb"),
        |_| panic!("embedded fixture"),
    )
    .unwrap();
    let mut source = imported.source().clone();
    source.clips[0].channels[0].values = ChannelValues::Rotation(vec![
        [0., 0., 0., 1.],
        [0.5, 0., 0., 0.75_f32.sqrt()],
        [0., 0., 0., 1.],
    ]);
    for m in &mut source.materials {
        m.base_color = [0.7, 0.4, 0.9, 1.0];
    }
    for image in &mut source.images {
        image.rgba8.fill(255);
    }
    let model = AnimatedModel::new(source).unwrap();
    AnimatedModel::from_bytes(&model.to_bytes().unwrap()).unwrap()
}

#[cfg(feature = "animation")]
fn animated_surface(
    model: &AnimatedModel,
    angle: f32,
    position: [f32; 3],
    scale: [f32; 3],
) -> Surface {
    let p = &model.source().primitives[0];
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
        // Fixture's nonidentity bind pivot is y=1.7; no palette/deform helper.
        let rotated = [
            original[0],
            1.7 + c * (original[1] - 1.7) - s * original[2],
            s * (original[1] - 1.7) + c * original[2],
        ];
        positions.push(std::array::from_fn(|k| {
            position[k] + scale[k] * (original[k] * (1. - weight) + rotated[k] * weight)
        }));
        let a = 1. - weight + weight * c;
        let b = weight * s;
        let determinant = a * a + b * b;
        normals.push(unit([
            0.,
            -b / determinant / scale[1],
            a / determinant / scale[2],
        ]));
    }
    Surface {
        positions,
        normals,
        indices: p.indices.clone(),
        albedo: [0.7, 0.4, 0.9],
    }
}

#[cfg(feature = "animation")]
#[test]
fn mixed_procedural_static_and_skinned_probes_follow_independent_poses_and_world_positions() {
    let (_lock, g) = gpu();
    let t = target(&g, FORMAT, SIZE);
    let view = g.create_texture_view(&t, None);
    let mut s = scene(&g, FORMAT, true);
    let rig = animated();
    let poses = [
        rig.rest_pose().unwrap(),
        rig.sample_clip(0, 0.5).unwrap(),
        rig.sample_clip(0, 1.).unwrap(),
    ];
    let mut skinned = SkinnedModelRenderer::new(g.clone(), s.scene_format(), rig.clone()).unwrap();
    let asset = panel([0.3, 0.3], [0., 0., 1.], [0.7, 0.4, 0.9]);
    let mut model = ModelRenderer::new(g.clone(), s.scene_format(), asset.clone()).unwrap();
    let mut procedural = Renderer3D::with_settings(g.clone(), s.scene_format(), Settings3D::LOW);
    let scale = [0.9, 0.33, 0.65];
    let mut previous = None;
    for (pose_index, pose) in poses.iter().enumerate() {
        let angle = pose_index as f32 * std::f32::consts::PI / 6.;
        let left = [-1.1, 0.45, 0.0];
        let right = [1.1, 0.45, 0.0];
        let instances = [
            SkinnedInstance {
                pose,
                transform: placement(left, scale, [0.; 2]),
            },
            SkinnedInstance {
                pose: &poses[0],
                transform: placement(right, scale, [0.; 2]),
            },
        ];
        let static_instances = [
            StaticInstance {
                translation: [-1.1, -1.1, 0.],
                ..Default::default()
            },
            StaticInstance {
                translation: [1.1, -1.1, 0.],
                ..Default::default()
            },
        ];
        let mut list = RenderList3D::new();
        let mut surfaces = Vec::new();
        for x in [-1.1, 1.1] {
            surfaces.push(panel_surface(&asset, [x, -1.1, 0.], [1.; 3], [0.; 2]));
            list.cuboid(
                [x, -0.3, 0.],
                quaternion([0.; 2]),
                [0.3, 0.3, 0.1],
                &Material::new([0.7, 0.4, 0.9]).rough(1.),
            );
            surfaces.extend(box_surfaces(
                [x, -0.3, 0.],
                [0.3, 0.3, 0.1],
                [0.; 2],
                [0.7, 0.4, 0.9],
            ));
        }
        surfaces.push(animated_surface(&rig, angle, left, scale));
        surfaces.push(animated_surface(&rig, 0., right, scale));
        for directional_case in [false, true] {
            s.irradiance = Some(if directional_case {
                grid_from_polynomial(|_| DIRECTIONAL)
            } else {
                spatial_grid()
            });
            draw(
                &mut s,
                &view,
                SIZE,
                &camera(),
                &light(2.7),
                &PointLightSettings::default(),
                &mut [
                    ImportedBatch::Procedural {
                        renderer: &mut procedural,
                        list: &list,
                    },
                    ImportedBatch::StaticInstances {
                        renderer: &mut model,
                        instances: &static_instances,
                    },
                    ImportedBatch::Skinned {
                        renderer: &mut skinned,
                        instances: &instances,
                    },
                ],
            )
            .unwrap();
            let pixels = hdr(&s);
            let counts = assert_oracle(&pixels, &surfaces, |p, n| {
                if directional_case {
                    directional(n)
                } else {
                    spatial(p)
                }
            });
            assert!(
                counts.iter().all(|n| *n > 65),
                "each path and both instance locations must be checked: {counts:?}"
            );
            assert_eq!(
                skinned.bounds().len(),
                2,
                "two independent GPU-skinned instances"
            );
            if directional_case {
                if let Some(before) = &previous {
                    assert_ne!(
                        &pixels, before,
                        "actual pose motion must change SH-lit geometry"
                    );
                }
                previous = Some(pixels);
            }
        }
    }
}

#[cfg(feature = "animation")]
#[derive(Debug, PartialEq)]
struct Snapshot {
    display: Vec<u8>,
    hdr: Vec<[f32; 4]>,
    size: Option<(u32, u32)>,
    generation: u64,
    bytes: u64,
    depth_size: Option<(u32, u32)>,
    depth_generation: u64,
    shadow_size: Option<u32>,
    shadow_generation: u64,
}
#[cfg(feature = "animation")]
fn snapshot(g: &Wgpu, s: &ImportedSceneRenderer<Wgpu>, t: &Texture) -> Snapshot {
    Snapshot {
        display: g.read_texture(t),
        hdr: hdr(s),
        size: s.post_process_size(),
        generation: s.post_process_generation(),
        bytes: s.post_process_allocated_bytes(),
        depth_size: s.depth_size(),
        depth_generation: s.depth_generation(),
        shadow_size: s.shadow_map_size(),
        shadow_generation: s.shadow_map_generation(),
    }
}

#[cfg(feature = "animation")]
#[test]
fn malformed_grid_combined_with_resize_and_cache_growth_preserves_last_accepted_frame_and_bounds() {
    let (_lock, g) = gpu();
    let t = target(&g, FORMAT, SIZE);
    let view = g.create_texture_view(&t, None);
    let mut s = scene(&g, FORMAT, true);
    s.post_process.bloom = true;
    s.irradiance = Some(spatial_grid());
    let rig = animated();
    let rest = rig.rest_pose().unwrap();
    let bent = rig.sample_clip(0, 1.).unwrap();
    let foreign = animated().rest_pose().unwrap();
    let mut skinned = SkinnedModelRenderer::new(g.clone(), s.scene_format(), rig).unwrap();
    let asset = panel([0.3, 0.3], [0., 0., 1.], [0.7, 0.4, 0.9]);
    let mut model = ModelRenderer::new(g.clone(), s.scene_format(), asset).unwrap();
    let mut procedural = Renderer3D::with_settings(g.clone(), s.scene_format(), Settings3D::LOW);
    let mut list = RenderList3D::new();
    list.cuboid(
        [-1., -1., 0.],
        quaternion([0.; 2]),
        [0.25; 3],
        &Material::new([0.4; 3]).rough(1.),
    );
    let mut lighting = light(0.2);
    lighting.shadows = true;
    lighting.intensity = 0.5;
    lighting.shadow_radius = 4.;
    let initial = [SkinnedInstance {
        pose: &rest,
        transform: placement([0., -1., 0.], [0.7; 3], [0.; 2]),
    }];
    let static_initial = [StaticInstance::default()];
    draw(
        &mut s,
        &view,
        SIZE,
        &camera(),
        &lighting,
        &PointLightSettings::default(),
        &mut [
            ImportedBatch::Procedural {
                renderer: &mut procedural,
                list: &list,
            },
            ImportedBatch::StaticInstances {
                renderer: &mut model,
                instances: &static_initial,
            },
            ImportedBatch::Skinned {
                renderer: &mut skinned,
                instances: &initial,
            },
        ],
    )
    .unwrap();
    let accepted = snapshot(&g, &s, &t);
    let bounds = skinned.bounds().to_vec();
    let stats = procedural.last_frame_stats();
    let valid = s.irradiance.clone().unwrap();
    let mut invalids = Vec::new();
    let mut bad = valid.clone();
    bad.version = 2;
    invalids.push(bad);
    let mut bad = valid.clone();
    bad.spacing[0] = 0.;
    invalids.push(bad);
    let mut bad = valid.clone();
    bad.origin[1] = f32::NAN;
    invalids.push(bad);
    let mut bad = valid.clone();
    bad.dimensions = [4, 4, 5];
    invalids.push(bad);
    let mut bad = valid.clone();
    bad.coefficients.pop();
    invalids.push(bad);
    let mut bad = valid.clone();
    bad.coefficients[0][3][1] = f32::INFINITY;
    invalids.push(bad);
    let mut bad = valid.clone();
    bad.coefficients[1][8][2] = 10_001.;
    bad.enabled = false;
    invalids.push(bad);
    let changed = [
        SkinnedInstance {
            pose: &bent,
            transform: placement([1., -0.7, 0.], [0.7; 3], [0.; 2]),
        },
        SkinnedInstance {
            pose: &rest,
            transform: placement([-1., -0.7, 0.], [0.7; 3], [0.; 2]),
        },
    ];
    let static_growth = [
        StaticInstance::default(),
        StaticInstance {
            translation: [1., 1., 0.],
            ..Default::default()
        },
    ];
    let mut changed_list = list.clone();
    changed_list.cuboid(
        [0.5, 0.7, 0.],
        quaternion([0.; 2]),
        [0.3; 3],
        &Material::new([0.8; 3]),
    );
    for bad in invalids {
        s.irradiance = Some(bad);
        let result = draw(
            &mut s,
            &view,
            (197, 199),
            &camera(),
            &lighting,
            &PointLightSettings::default(),
            &mut [
                ImportedBatch::Procedural {
                    renderer: &mut procedural,
                    list: &changed_list,
                },
                ImportedBatch::StaticInstances {
                    renderer: &mut model,
                    instances: &static_growth,
                },
                ImportedBatch::Skinned {
                    renderer: &mut skinned,
                    instances: &changed,
                },
            ],
        );
        assert!(
            matches!(result, Err(ImportedSceneError::Irradiance(_))),
            "grid validation must win before resize/cache mutation: {result:?}"
        );
        assert_eq!(snapshot(&g, &s, &t), accepted);
        assert_eq!(skinned.bounds(), bounds);
        assert_eq!(procedural.last_frame_stats(), stats);
    }
    // A valid newly-authored grid also must not leak into an otherwise invalid
    // frame, even after earlier batches proposed more instance-cache entries.
    s.irradiance = Some(constant_grid([3., 2., 1.]));
    let wrong = [SkinnedInstance {
        pose: &foreign,
        transform: IDENTITY,
    }];
    assert!(
        draw(
            &mut s,
            &view,
            (197, 199),
            &camera(),
            &lighting,
            &PointLightSettings::default(),
            &mut [
                ImportedBatch::Procedural {
                    renderer: &mut procedural,
                    list: &changed_list
                },
                ImportedBatch::StaticInstances {
                    renderer: &mut model,
                    instances: &static_growth
                },
                ImportedBatch::Skinned {
                    renderer: &mut skinned,
                    instances: &wrong
                },
            ]
        )
        .is_err()
    );
    assert_eq!(snapshot(&g, &s, &t), accepted);
    assert_eq!(skinned.bounds(), bounds);
    assert_eq!(procedural.last_frame_stats(), stats);
    s.irradiance = Some(valid);
    draw(
        &mut s,
        &view,
        SIZE,
        &camera(),
        &lighting,
        &PointLightSettings::default(),
        &mut [
            ImportedBatch::Procedural {
                renderer: &mut procedural,
                list: &list,
            },
            ImportedBatch::StaticInstances {
                renderer: &mut model,
                instances: &static_initial,
            },
            ImportedBatch::Skinned {
                renderer: &mut skinned,
                instances: &initial,
            },
        ],
    )
    .unwrap();
    assert_eq!(
        snapshot(&g, &s, &t),
        accepted,
        "rejected grid/resize/cache growth must not poison recovery"
    );
}

#[cfg(feature = "animation")]
#[test]
fn all_three_default_off_paths_are_byte_identical_to_legacy_standalone_after_probe_use() {
    let (_lock, g) = gpu();
    for format in [TextureFormat::Rgba8Unorm, TextureFormat::Rgba8UnormSrgb] {
        let t = target(&g, format, SIZE);
        let view = g.create_texture_view(&t, None);
        let mut s = scene(&g, format, false);
        let asset = panel([0.65, 0.65], [0., 0., 1.], [0.5, 0.7, 0.9]);
        let mut model = ModelRenderer::new(g.clone(), format, asset).unwrap();
        model.clear = s.clear;
        let rig = animated();
        let pose = rig.sample_clip(0, 1.).unwrap();
        let mut skinned = SkinnedModelRenderer::new(g.clone(), format, rig).unwrap();
        skinned.clear = s.clear;
        let instances = [SkinnedInstance {
            pose: &pose,
            transform: placement([0., -1., 0.], [0.7; 3], [0.; 2]),
        }];
        let mut procedural = Renderer3D::with_settings(g.clone(), format, Settings3D::LOW);
        procedural.clear = s.clear;
        let mut list = RenderList3D::new();
        list.cuboid(
            [0.; 3],
            quaternion([0.; 2]),
            [0.65, 0.65, 0.2],
            &Material::new([0.5, 0.7, 0.9]).rough(0.6),
        );
        for tonemap in [false, true] {
            let mut lighting = light(0.35);
            lighting.exposure = 0.8;
            lighting.tonemap = tonemap;
            model.draw(&view, SIZE, &camera(), &lighting).unwrap();
            let baseline = g.read_texture(&t);
            assert_legacy_toggles(
                &g,
                &mut s,
                &t,
                &view,
                &lighting,
                &baseline,
                &mut [ImportedBatch::Static(&mut model)],
            );
            list.lighting = lighting;
            procedural.draw(&view, SIZE, &list, &camera());
            let baseline = g.read_texture(&t);
            assert_legacy_toggles(
                &g,
                &mut s,
                &t,
                &view,
                &lighting,
                &baseline,
                &mut [ImportedBatch::Procedural {
                    renderer: &mut procedural,
                    list: &list,
                }],
            );
            skinned
                .draw(&view, SIZE, &camera(), &lighting, &instances)
                .unwrap();
            let baseline = g.read_texture(&t);
            assert_legacy_toggles(
                &g,
                &mut s,
                &t,
                &view,
                &lighting,
                &baseline,
                &mut [ImportedBatch::Skinned {
                    renderer: &mut skinned,
                    instances: &instances,
                }],
            );
        }
        assert_eq!(s.post_process_generation(), 0);
        assert_eq!(s.post_process_allocated_bytes(), 0);
        assert!(s.hdr_scene_texture().is_none());
    }
}

#[cfg(feature = "animation")]
#[allow(clippy::too_many_arguments)]
fn assert_legacy_toggles(
    g: &Wgpu,
    s: &mut ImportedSceneRenderer<Wgpu>,
    t: &Texture,
    view: &View,
    light: &Lighting,
    baseline: &[u8],
    batches: &mut [ImportedBatch<'_, Wgpu>],
) {
    let active = constant_grid([0.8, 0.15, 1.5]);
    let mut disabled = active.clone();
    disabled.enabled = false;
    let mut outside = active.clone();
    outside.origin = [30.; 3];
    s.irradiance = Some(active);
    draw(
        s,
        view,
        SIZE,
        &camera(),
        light,
        &PointLightSettings::default(),
        batches,
    )
    .unwrap();
    assert_ne!(
        g.read_texture(t),
        baseline,
        "probe-on frame must actually shade the drawable"
    );
    let depth = s.depth_generation();
    for grid in [None, Some(disabled), Some(outside)] {
        s.irradiance = grid;
        draw(
            s,
            view,
            SIZE,
            &camera(),
            light,
            &PointLightSettings::default(),
            batches,
        )
        .unwrap();
        assert_eq!(
            g.read_texture(t),
            baseline,
            "default off, toggled off and outside fallback must reproduce legacy standalone bytes"
        );
        assert_eq!(s.depth_generation(), depth);
    }
}

#[test]
fn opposing_valid_vertex_normals_interpolating_to_zero_preserve_legacy_fallback() {
    let (_lock, g) = gpu();
    let t = target(&g, FORMAT, SIZE);
    let view = g.create_texture_view(&t, None);
    let mut s = scene(&g, FORMAT, true);
    let original = panel([2.3, 1.0], [1., 0., 0.], [0.6, 0.8, 0.4]);
    let mut source = original.source().clone();
    // All authored normals are valid and unit length. Interpolation along the
    // screen's exactly centered column cancels -X and +X to the zero vector.
    // Odd target dimensions put a real fragment exactly at world x=0.
    for v in &mut source.primitives[0].vertices {
        v.normal = [if v.position[0] < 0.0 { -1.0 } else { 1.0 }, 0.0, 0.0];
    }
    let asset = StaticModel::new(source).unwrap();
    let asset = StaticModel::from_bytes(&asset.to_bytes().unwrap()).unwrap();
    let mut model = ModelRenderer::new(g.clone(), s.scene_format(), asset).unwrap();
    let lighting = light(0.35);
    draw(
        &mut s,
        &view,
        SIZE,
        &camera(),
        &lighting,
        &PointLightSettings::default(),
        &mut [ImportedBatch::Static(&mut model)],
    )
    .unwrap();
    let before = hdr(&s);
    let center = (SIZE.1 / 2 * SIZE.0 + SIZE.0 / 2) as usize;
    near(
        before[center],
        [0.21, 0.28, 0.14],
        "legacy zero-normal hemisphere fallback",
    );
    s.irradiance = Some(constant_grid([2.0, 3.0, 4.0]));
    draw(
        &mut s,
        &view,
        SIZE,
        &camera(),
        &lighting,
        &PointLightSettings::default(),
        &mut [ImportedBatch::Static(&mut model)],
    )
    .unwrap();
    let after = hdr(&s);
    assert_eq!(
        after[center], before[center],
        "zero interpolated normal has no SH direction and must preserve exact legacy ambient"
    );
    for offset in [-10_isize, 10] {
        near(
            after[(center as isize + offset) as usize],
            [1.2, 2.4, 1.6],
            "neighboring nonzero interpolated normals must still receive probes",
        );
    }
}
