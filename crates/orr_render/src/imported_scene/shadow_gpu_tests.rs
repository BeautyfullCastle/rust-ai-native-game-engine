//! Real software-GPU acceptance for the coordinator-owned directional shadow map.
//! The diagnostic pass samples that exact map; it never re-renders test geometry.
//! Nearest-comparison depth reconstruction also works on GLSL backends, which
//! reject textureLoad for depth textures; its bounded depth error is <= 1/65536.
//! ORR_REQUIRE_GPU=1 turns adapter absence into a failure.
#![allow(clippy::float_arithmetic)]

use super::{ImportedBatch, ImportedSceneRenderer, ImportedSceneTarget};
use crate::{
    Camera3D, IDENTITY_ROT, Lighting, Material, ModelRenderer, PointLightSettings, RenderList3D,
    Renderer3D, SHARED_SHADOW_MAP_SIZE, Settings3D, SkinnedInstance, SkinnedModelRenderer,
    StaticInstance,
};
use orr_model::{
    IDENTITY, StaticModel, Vertex,
    animation::{AnimatedModel, ChannelValues, Matrix4, Pose, SkinnedVertex},
    animation_import,
};
use orr_rhi::{
    Binding, Blend, ColorAttachment, Command, PipelineDesc, Rhi, SamplerDesc, TextureDesc,
    TextureFormat, TextureUsage, Wgpu, WgpuOptions,
};

type Texture = <Wgpu as Rhi>::Texture;
const SIZE: u32 = 192;
const FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;

fn gpu() -> Option<Wgpu> {
    match Wgpu::headless(WgpuOptions {
        force_software: true,
        ..WgpuOptions::default()
    }) {
        Ok(g) => {
            eprintln!(
                "shared shadow acceptance adapter: {} (software: {})",
                g.adapter_name(),
                g.is_software()
            );
            assert!(
                g.is_software(),
                "acceptance must exercise the software adapter"
            );
            Some(g)
        }
        Err(error) => {
            assert!(
                std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "required software GPU unavailable: {error}"
            );
            eprintln!("SKIP software GPU: {error}");
            None
        }
    }
}

fn target(g: &Wgpu, size: (u32, u32), format: TextureFormat) -> Texture {
    g.create_texture(&TextureDesc {
        label: "shared shadow acceptance readback",
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
    assert_eq!(scene.shadow_map_size(), None);
    assert_eq!(scene.shadow_map_generation(), 0);
    scene
}

fn draw(
    g: &Wgpu,
    scene: &mut ImportedSceneRenderer<Wgpu>,
    target: &Texture,
    size: (u32, u32),
    camera: &Camera3D,
    lighting: &Lighting,
    batches: &mut [ImportedBatch<'_, Wgpu>],
) -> Vec<u8> {
    let view = g.create_texture_view(target, None);
    let format = scene.format();
    scene
        .draw(
            ImportedSceneTarget {
                view: &view,
                size,
                format,
                sample_count: 1,
            },
            camera,
            lighting,
            &PointLightSettings::default(),
            batches,
        )
        .unwrap();
    g.read_texture(target)
}

/// Encode actual sampled depth into RGB24 and coverage into alpha. The GLSL
/// software backend rejects depth textureLoad, so probe the existing map at exact
/// texel centers with a nearest LessEqual comparison sampler. Sixteen binary
/// search steps bound reconstruction error by 1/65536; there is no geometry
/// re-render, filtered sampling, or production resource-usage change. An exact
/// reference=1 comparison distinguishes clear texels from all covered texels.
/// RGB24 packing contributes less than 6e-8 additional quantization error.
fn read_shadow(g: &Wgpu, scene: &ImportedSceneRenderer<Wgpu>) -> Vec<u8> {
    const SHADER: &str = r#"
        @group(0) @binding(0) var actual_shadow: texture_depth_2d;
        @group(0) @binding(1) var nearest_compare: sampler_comparison;
        @vertex fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
            var corners = array<vec2<f32>, 3>(vec2(-1.0,-1.0),vec2(3.0,-1.0),vec2(-1.0,3.0));
            return vec4(corners[index],0.0,1.0);
        }
        @fragment fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
            let uv = p.xy / vec2<f32>(1024.0);
            var lower = 0.0;
            var upper = 1.0;
            for (var step = 0; step < 16; step = step + 1) {
                let reference = (lower + upper) * 0.5;
                let passed = textureSampleCompareLevel(actual_shadow, nearest_compare, uv, reference);
                if passed > 0.5 { lower = reference; } else { upper = reference; }
            }
            let clear = textureSampleCompareLevel(actual_shadow, nearest_compare, uv, 1.0) > 0.5;
            let depth = select((lower + upper) * 0.5, 1.0, clear);
            let packed = u32(round(clamp(depth,0.0,1.0)*16777215.0));
            return vec4<f32>(f32(packed & 255u), f32((packed >> 8u) & 255u),
                f32((packed >> 16u) & 255u), select(255.0,0.0,clear))/255.0;
        }
    "#;
    let size = SHARED_SHADOW_MAP_SIZE;
    let t = target(g, (size, size), FORMAT);
    let view = g.create_texture_view(&t, None);
    let shader = g.create_shader("inspect actual shared map", SHADER);
    let pipeline = g.create_pipeline(&PipelineDesc::color(
        "inspect actual shared map",
        &shader,
        ("vs", "fs"),
        &[],
        FORMAT,
        Blend::Opaque,
    ));
    let sampler = g.create_sampler(&SamplerDesc {
        linear: false,
        compare: true,
    });
    let group = g.create_bind_group(
        &pipeline,
        0,
        &[
            Binding::Texture {
                binding: 0,
                view: &scene
                    .shadow
                    .as_ref()
                    .expect("accepted shadowed frame allocated its map")
                    .view,
            },
            Binding::Sampler {
                binding: 1,
                sampler: &sampler,
            },
        ],
    );
    let mut encoder = g.create_encoder("actual shared shadow visualization");
    g.encode_render_pass(
        &mut encoder,
        "probe actual depth map into RGBA8 COPY_SRC",
        &ColorAttachment {
            view: &view,
            clear: Some([0.0; 4]),
            resolve: None,
        },
        &[
            Command::SetPipeline(&pipeline),
            Command::SetBindGroup(0, &group),
            Command::Draw {
                vertices: 0..3,
                instances: 0..1,
            },
        ],
    );
    g.submit(encoder);
    g.read_texture(&t)
}

fn depth(pixel: &[u8]) -> f32 {
    (u32::from(pixel[0]) | (u32::from(pixel[1]) << 8) | (u32::from(pixel[2]) << 16)) as f32
        / 16_777_215.0
}
fn coverage(map: &[u8]) -> usize {
    map.chunks_exact(4).filter(|p| p[3] == 255).count()
}
fn capture(name: &str, pixels: &[u8], size: (u32, u32), shadow: bool) {
    if let Some(dir) = std::env::var_os("ORR_SHARED_SHADOW_CAPTURE_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        let mut ppm = format!("P6\n{} {}\n255\n", size.0, size.1).into_bytes();
        for p in pixels.chunks_exact(4) {
            if shadow {
                let value = if p[3] == 0 {
                    255
                } else {
                    (depth(p) * 255.0).round() as u8
                };
                ppm.extend_from_slice(&[value; 3]);
            } else {
                ppm.extend_from_slice(&p[..3]);
            }
        }
        std::fs::write(
            std::path::PathBuf::from(dir).join(format!("{name}.ppm")),
            ppm,
        )
        .unwrap();
    }
}

fn translated(x: f32, y: f32, z: f32) -> Matrix4 {
    let mut m = IDENTITY;
    m[3] = [x, y, z, 1.0];
    m
}
fn transform(m: Matrix4, p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|k| m[0][k] * p[0] + m[1][k] * p[1] + m[2][k] * p[2] + m[3][k])
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn unit(v: [f32; 3]) -> [f32; 3] {
    let length = dot(v, v).sqrt();
    v.map(|x| x / length)
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn animated_fixture() -> AnimatedModel {
    let imported = animation_import::import_with_resolver(
        "fixtures/animated_strip.glb",
        include_bytes!("../../../orr_model/tests/fixtures/animated_strip.glb"),
        |_| panic!("original fixture embeds every dependency"),
    )
    .unwrap();
    let reloaded = AnimatedModel::from_bytes(&imported.to_bytes().unwrap()).unwrap();
    let mut source = reloaded.source().clone();
    // Retain the original hierarchy, x=7 mesh node, bind pivot and mixed weights.
    // Rotate around X to test changing depth as well as silhouette.
    source.clips[0].channels[0].values = ChannelValues::Rotation(vec![
        [0.0, 0.0, 0.0, 1.0],
        [0.5, 0.0, 0.0, 0.75f32.sqrt()],
        [0.0, 0.0, 0.0, 1.0],
    ]);
    source.materials[0].base_color = [1.0; 4];
    for image in &mut source.images {
        image.rgba8.fill(255);
    }
    let model = AnimatedModel::new(source).unwrap();
    AnimatedModel::from_bytes(&model.to_bytes().unwrap()).unwrap()
}
fn quad(half: f32) -> Vec<Vertex> {
    [
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
    .collect()
}
fn static_panel(half: f32, color: [f32; 3]) -> StaticModel {
    let imported = orr_model::import::import_with_resolver(
        "fixtures/model.glb",
        include_bytes!("../../../orr_model/tests/fixtures/model.glb"),
        |_| panic!("original fixture embeds every dependency"),
    )
    .unwrap();
    let mut source = imported.source().clone();
    source.primitives.truncate(1);
    source.primitives[0].vertices = quad(half);
    source.primitives[0].indices = vec![0, 1, 2, 0, 2, 3];
    source.primitives[0].transform = IDENTITY;
    source.primitives[0].material = 0;
    source.materials[0].base_color = [color[0], color[1], color[2], 1.0];
    for image in &mut source.images {
        image.rgba8.fill(255);
    }
    let model = StaticModel::new(source).unwrap();
    StaticModel::from_bytes(&model.to_bytes().unwrap()).unwrap()
}
fn animated_panel(half: f32, color: [f32; 3]) -> AnimatedModel {
    let mut source = animated_fixture().source().clone();
    source.primitives[0].vertices = quad(half)
        .into_iter()
        .map(|vertex| SkinnedVertex {
            vertex,
            joints: [0; 4],
            weights: [1.0, 0.0, 0.0, 0.0],
        })
        .collect();
    source.primitives[0].indices = vec![0, 1, 2, 0, 2, 3];
    source.materials[0].base_color = [color[0], color[1], color[2], 1.0];
    let model = AnimatedModel::new(source).unwrap();
    AnimatedModel::from_bytes(&model.to_bytes().unwrap()).unwrap()
}
fn light() -> Lighting {
    Lighting {
        direction: [-1.0, 0.0, -1.0],
        color: [1.0; 3],
        intensity: 0.85,
        sky: [1.0; 3],
        ground: [1.0; 3],
        ambient: 0.12,
        shadows: true,
        shadow_center: [0.0; 3],
        shadow_radius: 3.0,
        tonemap: false,
        exposure: 1.0,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Procedural,
    Static,
    Skinned,
}
enum Asset {
    Procedural {
        renderer: Box<Renderer3D<Wgpu>>,
        list: Box<RenderList3D>,
    },
    Static {
        renderer: Box<ModelRenderer<Wgpu>>,
        instances: [StaticInstance; 1],
    },
    Skinned {
        renderer: Box<SkinnedModelRenderer<Wgpu>>,
    },
}
impl Asset {
    fn new(
        g: &Wgpu,
        format: TextureFormat,
        kind: Kind,
        half: f32,
        color: [f32; 3],
        model: &AnimatedModel,
    ) -> Self {
        match kind {
            Kind::Procedural => {
                let mut list = RenderList3D::new();
                list.cuboid(
                    [0.0, 0.0, -0.02],
                    IDENTITY_ROT,
                    [half, half, 0.02],
                    &Material::new(color).rough(1.0),
                );
                Self::Procedural {
                    renderer: Box::new(Renderer3D::with_settings(
                        g.clone(),
                        format,
                        Settings3D::LOW,
                    )),
                    list: Box::new(list),
                }
            }
            Kind::Static => Self::Static {
                renderer: Box::new(
                    ModelRenderer::new(g.clone(), format, static_panel(half, color)).unwrap(),
                ),
                instances: [StaticInstance::default()],
            },
            Kind::Skinned => Self::Skinned {
                renderer: Box::new(
                    SkinnedModelRenderer::new(g.clone(), format, model.clone()).unwrap(),
                ),
            },
        }
    }
    fn move_to(&mut self, position: [f32; 3]) {
        match self {
            Self::Procedural { list, .. } => {
                list.boxes[0].pos = [position[0], position[1], position[2] - 0.02]
            }
            Self::Static { instances, .. } => instances[0].translation = position,
            Self::Skinned { .. } => {}
        }
    }
    fn batch<'a>(&'a mut self, instances: &'a [SkinnedInstance<'a>]) -> ImportedBatch<'a, Wgpu> {
        match self {
            Self::Procedural { renderer, list } => ImportedBatch::Procedural { renderer, list },
            Self::Static {
                renderer,
                instances,
            } => ImportedBatch::StaticInstances {
                renderer,
                instances,
            },
            Self::Skinned { renderer } => ImportedBatch::Skinned {
                renderer,
                instances,
            },
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn pair_frame(
    g: &Wgpu,
    scene: &mut ImportedSceneRenderer<Wgpu>,
    t: &Texture,
    cam: &Camera3D,
    lighting: &Lighting,
    receiver: &mut Asset,
    receiver_pose: &Pose,
    caster: &mut Asset,
    caster_pose: &Pose,
    caster_position: [f32; 3],
    reverse: bool,
) -> Vec<u8> {
    receiver.move_to([0.0; 3]);
    caster.move_to(caster_position);
    let receiver_instances = [SkinnedInstance::new(receiver_pose)];
    let caster_instances = [SkinnedInstance {
        pose: caster_pose,
        transform: translated(caster_position[0], caster_position[1], caster_position[2]),
    }];
    let mut batches = [
        receiver.batch(&receiver_instances),
        caster.batch(&caster_instances),
    ];
    if reverse {
        batches.reverse();
    }
    draw(g, scene, t, (SIZE, SIZE), cam, lighting, &mut batches)
}

fn assert_receiver_shadow(off: &[u8], on: &[u8], cam: &Camera3D, center: [f32; 3], label: &str) {
    let screen = cam.world_to_screen(center, (SIZE, SIZE)).unwrap();
    let mut checked = 0;
    for y in (screen[1] as i32 - 5)..=(screen[1] as i32 + 5) {
        for x in (screen[0] as i32 - 5)..=(screen[0] as i32 + 5) {
            let at = (y as usize * SIZE as usize + x as usize) * 4;
            let (a, b) = (&off[at..at + 4], &on[at..at + 4]);
            assert!(
                a[0] > 140 && a[0].abs_diff(a[1]) <= 2 && a[1].abs_diff(a[2]) <= 2,
                "{label}: oracle region must be visible white receiver: {a:?}"
            );
            assert!(
                a[0] >= b[0] + 35 && a[1] >= b[1] + 35 && a[2] >= b[2] + 35,
                "{label}: actual receiver did not darken at {x},{y}: off {a:?}, on {b:?}"
            );
            checked += 1;
        }
    }
    assert!(checked >= 100);
}

#[test]
fn shared_shadow_gpu_all_cross_type_cast_receive_pairs_and_reversed_batches() {
    let Some(g) = gpu() else { return };
    let cam = Camera3D::orthographic([0.0, 0.0, 6.0], [0.0; 3], 2.1);
    for format in [FORMAT, TextureFormat::Rgba8UnormSrgb] {
        for receiver_kind in [Kind::Procedural, Kind::Static, Kind::Skinned] {
            for caster_kind in [Kind::Procedural, Kind::Static, Kind::Skinned] {
                if receiver_kind == caster_kind {
                    continue;
                }
                let label = format!("{format:?}-{caster_kind:?}-onto-{receiver_kind:?}");
                let receiver_model = animated_panel(1.8, [0.7; 3]);
                let caster_model = animated_panel(0.32, [0.8, 0.04, 0.04]);
                let receiver_pose = receiver_model.rest_pose().unwrap();
                let caster_pose = caster_model.rest_pose().unwrap();
                let mut receiver =
                    Asset::new(&g, format, receiver_kind, 1.8, [0.7; 3], &receiver_model);
                let mut caster = Asset::new(
                    &g,
                    format,
                    caster_kind,
                    0.32,
                    [0.8, 0.04, 0.04],
                    &caster_model,
                );
                let mut scene = scene(&g, format);
                let t = target(&g, (SIZE, SIZE), format);
                let on = light();
                let off = Lighting {
                    shadows: false,
                    ..on
                };
                let position = [0.45, 0.0, 1.0];
                let baseline = pair_frame(
                    &g,
                    &mut scene,
                    &t,
                    &cam,
                    &off,
                    &mut receiver,
                    &receiver_pose,
                    &mut caster,
                    &caster_pose,
                    position,
                    false,
                );
                assert_eq!(scene.shadow_map_size(), None);
                assert_eq!(scene.shadow_map_generation(), 0);
                let shadowed = pair_frame(
                    &g,
                    &mut scene,
                    &t,
                    &cam,
                    &on,
                    &mut receiver,
                    &receiver_pose,
                    &mut caster,
                    &caster_pose,
                    position,
                    false,
                );
                assert_receiver_shadow(&baseline, &shadowed, &cam, [-0.53, 0.0, 0.0], &label);
                assert_eq!(scene.shadow_map_size(), Some(1024));
                assert_eq!(scene.shadow_map_generation(), 1);
                let map = read_shadow(&g, &scene);
                assert!(
                    coverage(&map) > 10_000,
                    "{label}: no substantial actual map coverage"
                );
                let reversed = pair_frame(
                    &g,
                    &mut scene,
                    &t,
                    &cam,
                    &on,
                    &mut receiver,
                    &receiver_pose,
                    &mut caster,
                    &caster_pose,
                    position,
                    true,
                );
                assert_eq!(shadowed, reversed, "{label}: color depends on batch order");
                assert_eq!(
                    map,
                    read_shadow(&g, &scene),
                    "{label}: actual map depends on batch order"
                );
                let switched_off = pair_frame(
                    &g,
                    &mut scene,
                    &t,
                    &cam,
                    &off,
                    &mut receiver,
                    &receiver_pose,
                    &mut caster,
                    &caster_pose,
                    position,
                    false,
                );
                assert_eq!(
                    baseline, switched_off,
                    "{label}: turning shadows off must restore baseline"
                );
                let moved = pair_frame(
                    &g,
                    &mut scene,
                    &t,
                    &cam,
                    &on,
                    &mut receiver,
                    &receiver_pose,
                    &mut caster,
                    &caster_pose,
                    [-0.35, 0.0, 1.0],
                    false,
                );
                let moved_off = pair_frame(
                    &g,
                    &mut scene,
                    &t,
                    &cam,
                    &off,
                    &mut receiver,
                    &receiver_pose,
                    &mut caster,
                    &caster_pose,
                    [-0.35, 0.0, 1.0],
                    false,
                );
                assert_receiver_shadow(&moved_off, &moved, &cam, [-1.33, 0.0, 0.0], &label);
                assert_ne!(
                    shadowed, moved,
                    "{label}: moving caster did not move its shadow"
                );
                let changed_sun = Lighting {
                    direction: [1.0, 0.0, -1.0],
                    ..on
                };
                let sun = pair_frame(
                    &g,
                    &mut scene,
                    &t,
                    &cam,
                    &changed_sun,
                    &mut receiver,
                    &receiver_pose,
                    &mut caster,
                    &caster_pose,
                    position,
                    false,
                );
                assert_receiver_shadow(&baseline, &sun, &cam, [1.43, 0.0, 0.0], &label);
                assert_ne!(
                    shadowed, sun,
                    "{label}: changing sun did not move its shadow"
                );
                assert_eq!(scene.shadow_map_generation(), 1);
                capture(&format!("{label}-main"), &shadowed, (SIZE, SIZE), false);
                capture(&format!("{label}-depth"), &map, (1024, 1024), true);
            }
        }
    }
}

#[derive(Clone)]
struct Surface {
    positions: Vec<[f32; 3]>,
    indices: Vec<u32>,
}
/// Scalar blend about the fixture's nonidentity bind pivot. Deliberately avoids
/// renderer palettes, hierarchy traversal, skin matrices and model.deform.
fn analytic_surface(model: &AnimatedModel, angle: f32, placement: Matrix4) -> Surface {
    let p = &model.source().primitives[0];
    let (s, c) = angle.sin_cos();
    let positions = p
        .vertices
        .iter()
        .map(|v| {
            let original = v.vertex.position;
            let weight = v
                .joints
                .iter()
                .zip(v.weights)
                .filter_map(|(&joint, w)| (joint == 1).then_some(w))
                .sum::<f32>()
                / v.weights.iter().sum::<f32>();
            let rotated = [
                original[0],
                1.7 + c * (original[1] - 1.7) - s * original[2],
                s * (original[1] - 1.7) + c * original[2],
            ];
            transform(
                placement,
                std::array::from_fn(|k| original[k] * (1.0 - weight) + rotated[k] * weight),
            )
        })
        .collect();
    Surface {
        positions,
        indices: p.indices.clone(),
    }
}
fn barycentric(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> [f32; 3] {
    let denominator = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    let u = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / denominator;
    let v = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / denominator;
    [u, v, 1.0 - u - v]
}
/// Independent scalar light projection, including the documented texel snapping.
/// It does not call light_view_proj, Mat4, or any renderer preparation helper.
fn light_project(p: [f32; 3], lighting: &Lighting) -> [f32; 3] {
    let forward = unit(lighting.direction);
    let helper = if forward[1].abs() > 0.95 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let side = unit(cross(forward, helper));
    let up = cross(side, forward);
    let radius = lighting.shadow_radius;
    let texel = 2.0 * radius / 1024.0;
    let centered =
        |axis| dot(p, axis) - (dot(lighting.shadow_center, axis) / texel).round() * texel;
    [
        (centered(side) / radius * 0.5 + 0.5) * 1024.0,
        (0.5 - centered(up) / radius * 0.5) * 1024.0,
        (centered(forward) + 3.0 * radius - 0.1) / (6.0 * radius - 0.1),
    ]
}
#[derive(Clone, Copy)]
struct Triangle {
    p: [[f32; 3]; 3],
    slope: f32,
}
fn projected_triangles(
    surfaces: &[Surface],
    project: impl Fn([f32; 3]) -> [f32; 3],
) -> Vec<Triangle> {
    let mut triangles = Vec::new();
    for surface in surfaces {
        let positions: Vec<_> = surface.positions.iter().map(|&p| project(p)).collect();
        for ids in surface.indices.chunks_exact(3) {
            let p = [
                positions[ids[0] as usize],
                positions[ids[1] as usize],
                positions[ids[2] as usize],
            ];
            let d = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1])
                - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
            assert!(d.abs() > 1e-5, "oracle triangle unexpectedly degenerate");
            let dx = ((p[1][2] - p[0][2]) * (p[2][1] - p[0][1])
                - (p[2][2] - p[0][2]) * (p[1][1] - p[0][1]))
                / d;
            let dy = ((p[1][0] - p[0][0]) * (p[2][2] - p[0][2])
                - (p[2][0] - p[0][0]) * (p[1][2] - p[0][2]))
                / d;
            triangles.push(Triangle {
                p,
                slope: dx.abs().max(dy.abs()),
            });
        }
    }
    triangles
}
fn oracle_sample(triangles: &[Triangle], x: u32, y: u32) -> (bool, Option<(f32, f32)>) {
    let mut near = false;
    let mut expected: Option<(f32, f32)> = None;
    for triangle in triangles {
        let p = triangle.p;
        let b = barycentric(
            [x as f32 + 0.5, y as f32 + 0.5],
            [p[0][0], p[0][1]],
            [p[1][0], p[1][1]],
            [p[2][0], p[2][1]],
        );
        near |= b.iter().all(|&w| w >= -0.04);
        if b.iter().all(|&w| w > 0.04) {
            let z = (0..3).map(|k| b[k] * p[k][2]).sum();
            if expected.is_none_or(|(old, _)| z < old) {
                expected = Some((z, triangle.slope));
            }
        }
    }
    (near, expected)
}
fn assert_shadow_oracle(map: &[u8], surfaces: &[Surface], lighting: &Lighting, label: &str) {
    let triangles = projected_triangles(surfaces, |p| light_project(p, lighting));
    let mut checked = 0;
    let mut clear_checked = 0;
    // Full actual texture is read; a regular 2x2 sample grid bounds CPU cost.
    for y in (0..1024).step_by(2) {
        for x in (0..1024).step_by(2) {
            let (near, expected) = oracle_sample(&triangles, x, y);
            let at = (y as usize * 1024 + x as usize) * 4;
            let actual = &map[at..at + 4];
            if let Some((z, slope)) = expected {
                assert_eq!(
                    actual[3], 255,
                    "{label}: missing actual shadow coverage at {x},{y}"
                );
                // Production depth bias is 2 * maximum depth slope plus two
                // representable depth units. Comparison reconstruction is
                // <= 1/65536; allow only another 5e-6 for CPU/GPU plane math.
                let biased = z + 2.0 * slope;
                assert!(
                    (depth(actual) - biased).abs() < 1.0 / 65536.0 + 5e-6,
                    "{label}: actual depth {} != analytic biased depth {biased} (unbiased {z}, slope {slope}) at {x},{y}",
                    depth(actual)
                );
                checked += 1;
            } else if !near {
                assert_eq!(
                    actual,
                    [255, 255, 255, 0],
                    "{label}: unexpected actual caster coverage at {x},{y}"
                );
                clear_checked += 1;
            }
        }
    }
    assert!(
        checked > 500,
        "{label}: insufficient analytic depth coverage ({checked})"
    );
    assert!(
        clear_checked > 100_000,
        "{label}: insufficient clear map coverage ({clear_checked})"
    );
}
fn assert_main_oracle(
    pixels: &[u8],
    surfaces: &[Surface],
    camera: &Camera3D,
    size: (u32, u32),
    label: &str,
) {
    let triangles = projected_triangles(surfaces, |p| {
        let q = camera.world_to_screen(p, size).unwrap();
        [q[0], q[1], -p[2]]
    });
    let mut checked = 0;
    let mut clear_checked = 0;
    for y in 0..size.1 {
        for x in 0..size.0 {
            let (near, expected) = oracle_sample(&triangles, x, y);
            let at = (y as usize * size.0 as usize + x as usize) * 4;
            let actual = &pixels[at..at + 4];
            if expected.is_some() {
                assert!(
                    actual[..3].iter().all(|&v| v >= 252),
                    "{label}: main pass disagrees with analytic geometry at {x},{y}: {actual:?}"
                );
                checked += 1;
            } else if !near {
                assert_eq!(
                    actual,
                    [0, 0, 0, 255],
                    "{label}: main pass outside analytic geometry at {x},{y}"
                );
                clear_checked += 1;
            }
        }
    }
    assert!(
        checked > 100,
        "{label}: insufficient main oracle coverage: {checked}"
    );
    assert!(
        clear_checked > 5_000,
        "{label}: insufficient main background: {clear_checked}"
    );
}

#[test]
fn shared_shadow_gpu_actual_depth_matches_independent_cpu_skinning_and_light_projection() {
    let Some(g) = gpu() else { return };
    let model = animated_fixture();
    let primitive = &model.source().primitives[0];
    assert_eq!(
        model.source().nodes[primitive.node as usize]
            .rest
            .translation[0],
        7.0
    );
    let poses = [
        model.rest_pose().unwrap(),
        model.sample_clip(0, 0.5).unwrap(),
        model.sample_clip(0, 1.0).unwrap(),
    ];
    let mut nonuniform = translated(0.6, -0.25, 0.4);
    nonuniform[0][0] = 0.8;
    nonuniform[1][1] = 1.15;
    nonuniform[2][2] = 0.65;
    let camera = Camera3D::orthographic([0.0, 1.5, 8.0], [0.0, 1.5, 0.0], 3.0);
    let lighting = Lighting {
        direction: [-0.45, -0.2, -1.0],
        shadow_center: [0.13, 1.63, 0.17],
        shadow_radius: 3.5,
        intensity: 0.0,
        ambient: 1.0,
        ..light()
    };
    for format in [FORMAT, TextureFormat::Rgba8UnormSrgb] {
        let mut scene = scene(&g, format);
        let mut renderer = SkinnedModelRenderer::new(g.clone(), format, model.clone()).unwrap();
        let t = target(&g, (SIZE, SIZE), format);
        for (placement_index, placement) in [IDENTITY, nonuniform].into_iter().enumerate() {
            let mut previous_map = None;
            let mut previous_main = None;
            for ((name, angle), pose) in [
                ("rest", 0.0),
                ("mid", std::f32::consts::PI / 6.0),
                ("key", std::f32::consts::PI / 3.0),
            ]
            .into_iter()
            .zip(&poses)
            {
                let label = format!("{format:?}-{placement_index}-{name}");
                let surface = analytic_surface(&model, angle, placement);
                let cpu = model.deform(pose).unwrap();
                for (actual, expected) in cpu[0].vertices.iter().zip(&surface.positions) {
                    let actual = transform(placement, actual.position);
                    assert!(
                        actual
                            .iter()
                            .zip(expected)
                            .all(|(a, b)| (a - b).abs() < 3e-5),
                        "CPU deformation differs from fixture analytic reference"
                    );
                }
                let instances = [SkinnedInstance {
                    pose,
                    transform: placement,
                }];
                let pixels = draw(
                    &g,
                    &mut scene,
                    &t,
                    (SIZE, SIZE),
                    &camera,
                    &lighting,
                    &mut [ImportedBatch::Skinned {
                        renderer: &mut renderer,
                        instances: &instances,
                    }],
                );
                let map = read_shadow(&g, &scene);
                assert_shadow_oracle(&map, std::slice::from_ref(&surface), &lighting, &label);
                assert_main_oracle(
                    &pixels,
                    std::slice::from_ref(&surface),
                    &camera,
                    (SIZE, SIZE),
                    &label,
                );
                if let Some(previous) = previous_map {
                    assert_ne!(
                        previous, map,
                        "{label}: pose did not change actual depth map"
                    );
                }
                if let Some(previous) = previous_main {
                    assert_ne!(
                        previous, pixels,
                        "{label}: pose did not change main silhouette"
                    );
                }
                capture(
                    &format!("analytic-{label}-main"),
                    &pixels,
                    (SIZE, SIZE),
                    false,
                );
                capture(&format!("analytic-{label}-depth"), &map, (1024, 1024), true);
                previous_map = Some(map);
                previous_main = Some(pixels);
            }
        }
        assert_eq!(scene.shadow_map_generation(), 1);
    }
}

#[test]
fn shared_shadow_gpu_two_independent_poses_repeat_empty_and_resize_without_stale_depth() {
    let Some(g) = gpu() else { return };
    let model = animated_fixture();
    let rest = model.rest_pose().unwrap();
    let mid = model.sample_clip(0, 0.5).unwrap();
    let key = model.sample_clip(0, 1.0).unwrap();
    let mut renderer = SkinnedModelRenderer::new(g.clone(), FORMAT, model.clone()).unwrap();
    let mut scene = scene(&g, FORMAT);
    let t = target(&g, (SIZE, SIZE), FORMAT);
    let camera = Camera3D::orthographic([0.0, 1.5, 8.0], [0.0, 1.5, 0.0], 3.5);
    let lighting = Lighting {
        direction: [0.0, 0.0, -1.0],
        shadow_center: [0.0, 1.5, 0.0],
        shadow_radius: 3.5,
        intensity: 0.0,
        ambient: 1.0,
        ..light()
    };
    let placements = [translated(-1.3, 0.0, 0.0), translated(1.3, 0.0, 0.3)];
    let instances = [
        SkinnedInstance {
            pose: &rest,
            transform: placements[0],
        },
        SkinnedInstance {
            pose: &key,
            transform: placements[1],
        },
    ];
    let surfaces = [
        analytic_surface(&model, 0.0, placements[0]),
        analytic_surface(&model, std::f32::consts::PI / 3.0, placements[1]),
    ];
    let first = draw(
        &g,
        &mut scene,
        &t,
        (SIZE, SIZE),
        &camera,
        &lighting,
        &mut [ImportedBatch::Skinned {
            renderer: &mut renderer,
            instances: &instances,
        }],
    );
    let first_map = read_shadow(&g, &scene);
    assert_shadow_oracle(&first_map, &surfaces, &lighting, "independent rest/key");
    assert_main_oracle(
        &first,
        &surfaces,
        &camera,
        (SIZE, SIZE),
        "independent rest/key",
    );
    let repeated = draw(
        &g,
        &mut scene,
        &t,
        (SIZE, SIZE),
        &camera,
        &lighting,
        &mut [ImportedBatch::Skinned {
            renderer: &mut renderer,
            instances: &instances,
        }],
    );
    assert_eq!(first, repeated, "identical frame changed main pixels");
    assert_eq!(
        first_map,
        read_shadow(&g, &scene),
        "identical frame changed actual map"
    );
    // Compare both map halves and both main regions to rendering each pose alone.
    for (index, instance) in instances.iter().enumerate() {
        let single = draw(
            &g,
            &mut scene,
            &t,
            (SIZE, SIZE),
            &camera,
            &lighting,
            &mut [ImportedBatch::Skinned {
                renderer: &mut renderer,
                instances: std::slice::from_ref(instance),
            }],
        );
        let single_map = read_shadow(&g, &scene);
        for y in 0..SIZE as usize {
            let (start, end) = if index == 0 {
                (0, SIZE as usize / 2)
            } else {
                (SIZE as usize / 2, SIZE as usize)
            };
            let range = (y * SIZE as usize + start) * 4..(y * SIZE as usize + end) * 4;
            assert_eq!(
                first[range.clone()],
                single[range],
                "independent main pose region {index}"
            );
        }
        for y in 0..1024usize {
            let (start, end) = if index == 0 { (0, 512) } else { (512, 1024) };
            let range = (y * 1024 + start) * 4..(y * 1024 + end) * 4;
            assert_eq!(
                first_map[range.clone()],
                single_map[range],
                "independent actual map region {index}"
            );
        }
    }
    let changed_instances = [
        SkinnedInstance {
            pose: &mid,
            transform: placements[0],
        },
        instances[1],
    ];
    let changed = draw(
        &g,
        &mut scene,
        &t,
        (SIZE, SIZE),
        &camera,
        &lighting,
        &mut [ImportedBatch::Skinned {
            renderer: &mut renderer,
            instances: &changed_instances,
        }],
    );
    let changed_map = read_shadow(&g, &scene);
    let changed_surfaces = [
        analytic_surface(&model, std::f32::consts::PI / 6.0, placements[0]),
        surfaces[1].clone(),
    ];
    assert_shadow_oracle(
        &changed_map,
        &changed_surfaces,
        &lighting,
        "only first pose changed",
    );
    assert_main_oracle(
        &changed,
        &changed_surfaces,
        &camera,
        (SIZE, SIZE),
        "only first pose changed",
    );
    assert_ne!(first, changed);
    assert_ne!(first_map, changed_map);
    for y in 0..SIZE as usize {
        let range = (y * SIZE as usize + SIZE as usize / 2) * 4..(y + 1) * SIZE as usize * 4;
        assert_eq!(
            first[range.clone()],
            changed[range],
            "changing first pose changed second pose main pixels"
        );
    }
    for y in 0..1024usize {
        let range = (y * 1024 + 512) * 4..(y + 1) * 1024 * 4;
        assert_eq!(
            first_map[range.clone()],
            changed_map[range],
            "changing first pose changed second pose's map"
        );
    }
    let resized_size = (237, 161);
    let resized = target(&g, resized_size, FORMAT);
    let pixels = draw(
        &g,
        &mut scene,
        &resized,
        resized_size,
        &camera,
        &lighting,
        &mut [ImportedBatch::Skinned {
            renderer: &mut renderer,
            instances: &changed_instances,
        }],
    );
    assert_main_oracle(
        &pixels,
        &changed_surfaces,
        &camera,
        resized_size,
        "resized main",
    );
    assert_eq!(
        changed_map,
        read_shadow(&g, &scene),
        "viewport resize changed fixed world-space map"
    );
    assert_eq!(scene.depth_size(), Some(resized_size));
    assert_eq!(scene.depth_generation(), 2);
    assert_eq!(scene.shadow_map_size(), Some(1024));
    assert_eq!(scene.shadow_map_generation(), 1);
    let empty = draw(
        &g,
        &mut scene,
        &resized,
        resized_size,
        &camera,
        &lighting,
        &mut [],
    );
    assert!(
        empty.chunks_exact(4).all(|p| p == [0, 0, 0, 255]),
        "empty frame retained main geometry"
    );
    let empty_map = read_shadow(&g, &scene);
    assert_eq!(
        coverage(&empty_map),
        0,
        "empty frame retained previous casters"
    );
    assert!(empty_map.chunks_exact(4).all(|p| p == [255, 255, 255, 0]));
    let restored = draw(
        &g,
        &mut scene,
        &t,
        (SIZE, SIZE),
        &camera,
        &lighting,
        &mut [ImportedBatch::Skinned {
            renderer: &mut renderer,
            instances: &instances,
        }],
    );
    assert_eq!(
        first, restored,
        "reuse after empty frame changed main pixels"
    );
    assert_eq!(
        first_map,
        read_shadow(&g, &scene),
        "reuse after empty frame changed actual map"
    );
    assert_eq!(scene.shadow_map_generation(), 1);
}

#[test]
fn shared_shadow_gpu_offscreen_casters_survive_and_outside_coverage_receivers_are_lit() {
    let Some(g) = gpu() else { return };
    let camera = Camera3D::orthographic([0.0, 0.0, 6.0], [0.0; 3], 1.0);
    for (receiver_kind, caster_kind) in [
        (Kind::Procedural, Kind::Static),
        (Kind::Static, Kind::Skinned),
        (Kind::Skinned, Kind::Procedural),
    ] {
        let receiver_model = animated_panel(1.8, [0.7; 3]);
        let caster_model = animated_panel(0.32, [0.8, 0.04, 0.04]);
        let receiver_pose = receiver_model.rest_pose().unwrap();
        let caster_pose = caster_model.rest_pose().unwrap();
        let mut receiver = Asset::new(&g, FORMAT, receiver_kind, 1.8, [0.7; 3], &receiver_model);
        let mut caster = Asset::new(
            &g,
            FORMAT,
            caster_kind,
            0.32,
            [0.8, 0.04, 0.04],
            &caster_model,
        );
        let mut scene = scene(&g, FORMAT);
        let t = target(&g, (SIZE, SIZE), FORMAT);
        let lighting = light();
        let off = Lighting {
            shadows: false,
            ..lighting
        };
        // Main camera covers x=-1..1. The nearest caster edge is x=1.13,
        // so every caster vertex is outside the main view, inside the light view.
        let position = [1.45, 0.0, 1.0];
        assert!(
            camera
                .world_to_screen([position[0] - 0.32, 0.0, 1.0], (SIZE, SIZE))
                .unwrap()[0]
                > SIZE as f32
        );
        let baseline = pair_frame(
            &g,
            &mut scene,
            &t,
            &camera,
            &off,
            &mut receiver,
            &receiver_pose,
            &mut caster,
            &caster_pose,
            position,
            false,
        );
        let shadowed = pair_frame(
            &g,
            &mut scene,
            &t,
            &camera,
            &lighting,
            &mut receiver,
            &receiver_pose,
            &mut caster,
            &caster_pose,
            position,
            true,
        );
        assert_receiver_shadow(
            &baseline,
            &shadowed,
            &camera,
            [0.47, 0.0, 0.0],
            "offscreen main caster",
        );
        let outside = Lighting {
            shadow_center: [8.0, 0.0, 0.0],
            shadow_radius: 1.0,
            ..lighting
        };
        let outside_pixels = pair_frame(
            &g,
            &mut scene,
            &t,
            &camera,
            &outside,
            &mut receiver,
            &receiver_pose,
            &mut caster,
            &caster_pose,
            position,
            false,
        );
        assert_eq!(
            baseline, outside_pixels,
            "{receiver_kind:?}: receiver outside finite map coverage must remain fully lit"
        );
        let map = read_shadow(&g, &scene);
        assert_eq!(
            coverage(&map),
            0,
            "off-coverage scene unexpectedly entered map"
        );
        assert_eq!(scene.shadow_map_generation(), 1);
    }
}

#[test]
fn shared_shadow_gpu_joint_pose_moves_shadow_on_another_renderer_receiver() {
    let Some(g) = gpu() else { return };
    let model = animated_fixture();
    let poses = [
        model.rest_pose().unwrap(),
        model.sample_clip(0, 0.5).unwrap(),
        model.sample_clip(0, 1.0).unwrap(),
    ];
    let mut caster = SkinnedModelRenderer::new(g.clone(), FORMAT, model.clone()).unwrap();
    let mut receiver = ModelRenderer::new(g.clone(), FORMAT, static_panel(3.5, [0.7; 3])).unwrap();
    let receiver_instances = [StaticInstance {
        translation: [0.0, 1.5, -1.0],
        material_override: Some(orr_model::MaterialOverride {
            material_slot: 0,
            // Preserve the independent gray-receiver mask classifier below,
            // while replacing the imported 0.7 factor with a distinct value.
            base_color_factor: [0.8; 3],
        }),
        ..Default::default()
    }];
    let camera = Camera3D::orthographic([-0.7, 1.5, 7.0], [-0.7, 1.5, 0.0], 2.4);
    let lighting = Lighting {
        shadow_center: [-0.5, 1.5, 0.0],
        shadow_radius: 4.0,
        ..light()
    };
    let mut scene = scene(&g, FORMAT);
    let t = target(&g, (SIZE, SIZE), FORMAT);
    let mut previous = None;
    for ((name, angle), pose) in [
        ("rest", 0.0),
        ("mid", std::f32::consts::PI / 6.0),
        ("key", std::f32::consts::PI / 3.0),
    ]
    .into_iter()
    .zip(&poses)
    {
        let instances = [SkinnedInstance::new(pose)];
        let off = draw(
            &g,
            &mut scene,
            &t,
            (SIZE, SIZE),
            &camera,
            &Lighting {
                shadows: false,
                ..lighting
            },
            &mut [
                ImportedBatch::Skinned {
                    renderer: &mut caster,
                    instances: &instances,
                },
                ImportedBatch::StaticInstances {
                    renderer: &mut receiver,
                    instances: &receiver_instances,
                },
            ],
        );
        let on = draw(
            &g,
            &mut scene,
            &t,
            (SIZE, SIZE),
            &camera,
            &lighting,
            &mut [
                ImportedBatch::StaticInstances {
                    renderer: &mut receiver,
                    instances: &receiver_instances,
                },
                ImportedBatch::Skinned {
                    renderer: &mut caster,
                    instances: &instances,
                },
            ],
        );
        let surface = analytic_surface(&model, angle, IDENTITY);
        // Intersect independently deformed vertices' sunlight rays with z=-1.
        let footprint = Surface {
            positions: surface
                .positions
                .iter()
                .map(|p| [p[0] - p[2] - 1.0, p[1], -1.0])
                .collect(),
            indices: surface.indices.clone(),
        };
        let screen_triangles = projected_triangles(&[footprint], |p| {
            let q = camera.world_to_screen(p, (SIZE, SIZE)).unwrap();
            [q[0], q[1], 0.0]
        });
        let caster_triangles = projected_triangles(&[surface], |p| {
            let q = camera.world_to_screen(p, (SIZE, SIZE)).unwrap();
            [q[0], q[1], -p[2]]
        });
        let mut checked = 0;
        let mut shadow_mask = vec![false; (SIZE * SIZE) as usize];
        for y in 0..SIZE {
            for x in 0..SIZE {
                let (_, expected) = oracle_sample(&screen_triangles, x, y);
                let (near_caster, _) = oracle_sample(&caster_triangles, x, y);
                // A three-pixel erosion rejects only the filter/normal-offset
                // boundary, not incorrect interiors or a wrong pose footprint.
                let interior = expected.is_some()
                    && !near_caster
                    && x >= 3
                    && y >= 3
                    && x + 3 < SIZE
                    && y + 3 < SIZE
                    && [(x - 3, y), (x + 3, y), (x, y - 3), (x, y + 3)]
                        .into_iter()
                        .all(|(a, b)| oracle_sample(&screen_triangles, a, b).1.is_some());
                let at = (y as usize * SIZE as usize + x as usize) * 4;
                if interior {
                    assert!(
                        off[at] >= on[at] + 35 && off[at + 1] >= on[at + 1] + 35,
                        "{name}: CPU-projected joint shadow missing on static receiver at {x},{y}: off {:?}, on {:?}",
                        &off[at..at + 4],
                        &on[at..at + 4]
                    );
                    checked += 1;
                }
                // Restrict to visibly gray receiver, so pose silhouette changes
                // cannot satisfy this independently measured receiver-shadow mask.
                shadow_mask[(y * SIZE + x) as usize] = !near_caster
                    && off[at].abs_diff(off[at + 1]) <= 2
                    && off[at] > 140
                    && i16::from(off[at]) - i16::from(on[at]) > 35;
            }
        }
        assert!(
            checked > 100,
            "{name}: insufficient confident projected receiver shadow: {checked}"
        );
        if let Some(previous) = previous {
            assert_ne!(
                previous, shadow_mask,
                "joint pose did not change receiver shadow mask"
            );
        }
        previous = Some(shadow_mask);
        capture(
            &format!("animated-{name}-receiver"),
            &on,
            (SIZE, SIZE),
            false,
        );
    }
}
