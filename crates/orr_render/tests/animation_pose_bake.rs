//! Fresh real CLI cooked-animation -> sampled static bake -> two GPU consumers.
//! Run the ignored acceptance with ORR_REQUIRE_GPU=1, an absolute current-source
//! ORR_ANIMATION_POSE_BAKE_BIN and fresh ORR_ANIMATION_POSE_BAKE_CAPTURE_DIR.
//! Neither CPU deformation nor agreement between renderers supplies the oracle.
#![cfg(all(feature = "animation", feature = "imported-scene"))]
#![allow(clippy::float_arithmetic)]

use orr_model::{
    IDENTITY, Image, StaticModel, Vertex, Wrap,
    animation::{
        AnimatedModel, AnimatedNode, AnimationChannel, AnimationClip, ChannelValues, Interpolation,
        Matrix4, Skin, SkinnedVertex, Trs,
    },
    animation_import,
};
use orr_render::orr_rhi::{Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions};
use orr_render::{
    Camera3D, ImportedBatch, ImportedSceneRenderer, ImportedSceneTarget, Lighting, ModelRenderer,
    PointLightSettings, SkinnedInstance, SkinnedModelRenderer, StaticInstance,
};
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    process::Command,
};

const SIZE: u32 = 256;
const FORMAT: TextureFormat = TextureFormat::Rgba8UnormSrgb;
const WEIGHTS: [f64; 4] = [0.1, 0.4, 0.2, 0.3];
const SCALE: [[f64; 3]; 4] = [
    [1.0, 1.0, 1.0],
    [1.8, 0.55, 1.1],
    [0.7, 1.4, 0.8],
    [1.3, 0.8, 1.5],
];
const QUAD: [[f64; 2]; 4] = [[-0.65, -0.65], [0.65, -0.65], [0.65, 0.65], [-0.65, 0.65]];
const UV: [[f64; 2]; 4] = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
const TRI: [[usize; 3]; 2] = [[0, 1, 2], [0, 2, 3]];
const CHECKER: [[u8; 4]; 4] = [
    [225, 83, 41, 17],
    [47, 198, 112, 63],
    [108, 56, 237, 149],
    [173, 219, 91, 231],
];
const FACTOR: [f64; 3] = [0.8, 0.7, 0.9];
const PLACEMENT_SCALE: [f64; 3] = [1.1, 0.85, 1.4];
const COLOR_EPSILON: u8 = 3;
const MIN_DIFFERENCES: usize = 100;

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let path = PathBuf::from(
            std::env::var_os("ORR_ANIMATION_POSE_BAKE_CAPTURE_DIR")
                .expect("mandatory fresh capture directory"),
        );
        assert!(path.is_absolute());
        fs::create_dir_all(&path).unwrap();
        let path = fs::canonicalize(path).unwrap();
        assert!(
            fs::read_dir(&path).unwrap().next().is_none(),
            "evidence directory must be empty"
        );
        Self(path)
    }
    fn write(&self, name: &str, bytes: &[u8]) {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(self.0.join(name))
            .unwrap();
        file.write_all(bytes).unwrap();
        file.sync_all().unwrap();
    }
    fn frame(&self, name: &str, bytes: &[u8]) {
        self.write(&format!("{name}.rgba"), bytes);
        let mut ppm = format!("P6\n{SIZE} {SIZE}\n255\n").into_bytes();
        for p in bytes.chunks_exact(4) {
            ppm.extend_from_slice(&p[..3]);
        }
        self.write(&format!("{name}.ppm"), &ppm);
    }
}
fn rotation(degrees: f64) -> [f32; 4] {
    let a = degrees.to_radians() / 2.0;
    [0.0, 0.0, a.sin() as f32, a.cos() as f32]
}
fn fixture(weights: [f64; 4]) -> AnimatedModel {
    let imported = animation_import::import_with_resolver(
        "fixtures/animated_strip.glb",
        include_bytes!("../../orr_model/tests/fixtures/animated_strip.glb"),
        |_| panic!("embedded fixture"),
    )
    .unwrap();
    let mut source = imported.source().clone();
    source.nodes = vec![AnimatedNode {
        name: "nonjoint ancestor".into(),
        children: vec![1, 2, 3, 4],
        rest: Trs {
            translation: [-0.9, 0.1, 0.0],
            scale: [1.1, 0.9, 1.2],
            ..Trs::default()
        },
    }];
    for (i, degrees) in [0.0, 0.0, -25.0, 20.0].into_iter().enumerate() {
        source.nodes.push(AnimatedNode {
            name: format!("joint {i}"),
            children: vec![],
            rest: Trs {
                translation: [0.2, 0.3, 0.0],
                rotation: rotation(degrees),
                scale: SCALE[i].map(|v| v as f32),
            },
        });
    }
    source.nodes.push(AnimatedNode {
        name: "mesh translation must not apply twice".into(),
        children: vec![],
        rest: Trs {
            translation: [7.0, 0.0, 0.0],
            ..Trs::default()
        },
    });
    source.nodes.push(AnimatedNode {
        name: "rigid nonuniform".into(),
        children: vec![],
        rest: Trs {
            translation: [1.1, -0.65, 0.0],
            scale: [0.7, 0.8, 1.3],
            ..Trs::default()
        },
    });
    let mut inverse = IDENTITY;
    inverse[3] = [-0.2, -0.3, 0.0, 1.0];
    source.skins = vec![Skin {
        name: "four distinct weighted joints".into(),
        joints: vec![1, 2, 3, 4],
        inverse_bind_matrices: vec![inverse; 4],
    }];
    let n = 1.0_f32 / 3.0_f32.sqrt();
    let mut primitive = source.primitives[0].clone();
    primitive.node = 5;
    primitive.skin = Some(0);
    primitive.id = format!("{}#node=5/mesh=0/primitive=0", source.asset_id);
    primitive.material = 0;
    primitive.indices = TRI.iter().flatten().map(|&i| i as u32).collect();
    primitive.vertices = QUAD
        .iter()
        .zip(UV)
        .map(|(p, uv)| SkinnedVertex {
            vertex: Vertex {
                position: [p[0] as f32, p[1] as f32, 0.0],
                normal: [n; 3],
                uv: uv.map(|v| v as f32),
            },
            joints: [0, 1, 2, 3],
            weights: weights.map(|v| v as f32),
        })
        .collect();
    let mut rigid = primitive.clone();
    rigid.id = format!("{}#node=6/mesh=1/primitive=0", source.asset_id);
    rigid.node = 6;
    rigid.skin = None;
    for v in &mut rigid.vertices {
        v.joints = [0; 4];
        v.weights = [1.0, 0.0, 0.0, 0.0];
    }
    source.primitives = vec![primitive, rigid];
    source.images = vec![Image {
        width: 2,
        height: 2,
        rgba8: CHECKER.concat(),
    }];
    source.materials.truncate(1);
    source.materials[0].image = 0;
    source.materials[0].base_color = [0.8, 0.7, 0.9, 0.25];
    source.materials[0].linear_filter = false;
    source.materials[0].wrap_s = Wrap::Clamp;
    source.materials[0].wrap_t = Wrap::Clamp;
    source.clips = vec![AnimationClip {
        name: "bend".into(),
        channels: vec![AnimationChannel {
            node: 2,
            interpolation: Interpolation::Linear,
            times: vec![0.0, 1.0],
            values: ChannelValues::Rotation(vec![rotation(0.0), rotation(60.0)]),
        }],
    }];
    // Round trip the exact cooked input subsequently read by the subprocess.
    let model = AnimatedModel::new(source).unwrap();
    AnimatedModel::from_bytes(&model.to_bytes().unwrap()).unwrap()
}
fn bake(files: &Files, name: &str, model: &AnimatedModel, time: f64) -> StaticModel {
    let input = format!("{name}.animated.json");
    files.write(&input, &model.to_bytes().unwrap());
    let recipe =
        json!({"format":"orr_animation_pose_bake","version":1,"source":input,"clip":0,"time":time});
    let recipe_name = format!("{name}.pose.json");
    files.write(&recipe_name, &serde_json::to_vec(&recipe).unwrap());
    let binary = PathBuf::from(
        std::env::var_os("ORR_ANIMATION_POSE_BAKE_BIN")
            .expect("absolute current-source CLI binary is mandatory"),
    );
    assert!(binary.is_absolute() && binary.is_file());
    let output = files.0.join(format!("{name}.model.json"));
    assert!(!output.exists());
    let result = Command::new(&binary)
        .arg("--recipe")
        .arg(files.0.join(recipe_name))
        .arg("--asset-id")
        .arg(format!("poses/{name}.pose.json"))
        .arg("--output")
        .arg(&output)
        .output()
        .expect("execute actual CLI");
    files.write(&format!("{name}.stdout"), &result.stdout);
    files.write(&format!("{name}.stderr"), &result.stderr);
    assert!(
        result.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    StaticModel::from_bytes(&fs::read(output).unwrap()).unwrap()
}
fn camera() -> Camera3D {
    Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 2.5)
}
fn light() -> Lighting {
    Lighting {
        direction: [0.0, 0.0, -1.0],
        color: [1.0; 3],
        intensity: 0.85,
        sky: [1.0; 3],
        ground: [1.0; 3],
        ambient: 0.1,
        shadows: false,
        tonemap: false,
        exposure: 1.0,
        ..Default::default()
    }
}
fn placement() -> Matrix4 {
    [
        [1.1, 0.0, 0.0, 0.0],
        [0.0, 0.85, 0.0, 0.0],
        [0.0, 0.0, 1.4, 0.0],
        [0.1, 0.2, 0.0, 1.0],
    ]
}
fn render_static(g: &Wgpu, target: &<Wgpu as Rhi>::Texture, model: StaticModel) -> Vec<u8> {
    let mut renderer = ModelRenderer::new(g.clone(), FORMAT, model).unwrap();
    let mut scene = ImportedSceneRenderer::new(g.clone(), FORMAT).unwrap();
    scene.clear = [0.0, 0.0, 0.0, 1.0];
    let instances = [StaticInstance {
        translation: [0.1, 0.2, 0.0],
        scale: [1.1, 0.85, 1.4],
        ..Default::default()
    }];
    let view = g.create_texture_view(target, None);
    scene
        .draw(
            ImportedSceneTarget {
                view: &view,
                size: (SIZE, SIZE),
                format: FORMAT,
                sample_count: 1,
            },
            &camera(),
            &light(),
            &PointLightSettings::default(),
            &mut [ImportedBatch::StaticInstances {
                renderer: &mut renderer,
                instances: &instances,
            }],
        )
        .unwrap();
    g.read_texture(target)
}

// Independent scalar f64 affine calculation. No model pose/palette/deform/normal
// helper or baked output is read to construct expected vertices or pixel colors.
// XY is [a c; b d], Z scale e, translation tx/ty; mesh node x=7 is absent.
fn joint_affine(j: usize, time: f64) -> [f64; 7] {
    let angle = [0.0, 60.0 * time, -25.0, 20.0][j].to_radians();
    let (s, c) = angle.sin_cos();
    let a = c * SCALE[j][0];
    let b = s * SCALE[j][0];
    let cc = -s * SCALE[j][1];
    let d = c * SCALE[j][1];
    [
        1.1 * a,
        0.9 * b,
        1.1 * cc,
        0.9 * d,
        1.2 * SCALE[j][2],
        -0.9 + 1.1 * (0.2 - a * 0.2 - cc * 0.3),
        0.1 + 0.9 * (0.3 - b * 0.2 - d * 0.3),
    ]
}
fn blended(time: f64, weights: [f64; 4]) -> [f64; 7] {
    std::array::from_fn(|k| (0..4).map(|j| weights[j] * joint_affine(j, time)[k]).sum())
}
fn normalize(v: [f64; 3]) -> [f64; 3] {
    let length = v.iter().map(|v| v * v).sum::<f64>().sqrt();
    v.map(|v| v / length)
}
fn normal(m: [f64; 7]) -> [f64; 3] {
    let [a, b, c, d, e, _, _] = m;
    let det = a * d - b * c;
    normalize([(d - b) / det, (a - c) / det, 1.0 / e])
}
fn place_normal(n: [f64; 3]) -> [f64; 3] {
    normalize(std::array::from_fn(|i| n[i] / PLACEMENT_SCALE[i]))
}
fn affine(part: usize) -> [f64; 7] {
    if part == 0 {
        blended(0.5, WEIGHTS)
    } else {
        [0.7, 0.0, 0.0, 0.8, 1.3, 1.1, -0.65]
    }
}
fn expected_positions(part: usize) -> [[f64; 3]; 4] {
    let [a, b, c, d, _, tx, ty] = affine(part);
    QUAD.map(|p| [a * p[0] + c * p[1] + tx, b * p[0] + d * p[1] + ty, 0.0])
}
fn screen(p: [f64; 3]) -> [f64; 2] {
    let x = p[0] * PLACEMENT_SCALE[0] + 0.1;
    let y = p[1] * PLACEMENT_SCALE[1] + 0.2;
    [
        (x / 2.5 + 1.0) * f64::from(SIZE) / 2.0,
        (1.0 - y / 2.5) * f64::from(SIZE) / 2.0,
    ]
}
fn barycentric(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> [f64; 3] {
    let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    let u = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / d;
    let v = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / d;
    [u, v, 1.0 - u - v]
}
fn decode(v: u8) -> f64 {
    let v = f64::from(v) / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
fn encode(v: f64) -> u8 {
    ((if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    })
    .clamp(0.0, 1.0)
        * 255.0)
        .round() as u8
}
struct Oracle {
    pixels: Vec<Option<[u8; 4]>>,
    interior: [usize; 2],
    clear: usize,
}
fn oracle() -> Oracle {
    let screens = [
        expected_positions(0).map(screen),
        expected_positions(1).map(screen),
    ];
    let mut oracle = Oracle {
        pixels: Vec::new(),
        interior: [0; 2],
        clear: 0,
    };
    for y in 0..SIZE {
        for x in 0..SIZE {
            let mut expected = None;
            let mut near = false;
            for (part, vertices) in screens.iter().enumerate() {
                for tri in TRI {
                    let b = barycentric(
                        [f64::from(x) + 0.5, f64::from(y) + 0.5],
                        vertices[tri[0]],
                        vertices[tri[1]],
                        vertices[tri[2]],
                    );
                    near |= b.iter().all(|&b| b >= -0.08);
                    if !b.iter().all(|&b| b > 0.08) {
                        continue;
                    }
                    let uv: [f64; 2] =
                        std::array::from_fn(|k| (0..3).map(|i| b[i] * UV[tri[i]][k]).sum());
                    let tex = uv.map(|v| v * 2.0);
                    if tex.iter().any(|v| v.fract() < 0.03 || v.fract() > 0.97) {
                        continue;
                    }
                    let texel = CHECKER[(tex[1].floor() as usize) * 2 + tex[0].floor() as usize];
                    let brightness = 0.1 + 0.85 * place_normal(normal(affine(part)))[2];
                    let mut color = [0, 0, 0, 255];
                    for k in 0..3 {
                        color[k] = encode(decode(texel[k]) * FACTOR[k] * brightness);
                    }
                    expected = Some(color);
                    oracle.interior[part] += 1;
                }
            }
            if !near {
                expected = Some([0, 0, 0, 255]);
                oracle.clear += 1;
            }
            oracle.pixels.push(expected);
        }
    }
    assert!(
        oracle.interior.iter().all(|&n| n > 500),
        "per-primitive interior: {:?}",
        oracle.interior
    );
    assert!(oracle.clear > 40_000, "clear region: {}", oracle.clear);
    oracle
}
fn differences(pixels: &[u8], oracle: &Oracle) -> usize {
    assert_eq!(pixels.len(), (SIZE * SIZE * 4) as usize);
    pixels
        .chunks_exact(4)
        .zip(&oracle.pixels)
        .filter(|(actual, expected)| {
            expected.is_some_and(|e| {
                actual
                    .iter()
                    .zip(e)
                    .any(|(&a, b)| a.abs_diff(b) > COLOR_EPSILON)
            })
        })
        .count()
}
fn assert_oracle(label: &str, pixels: &[u8], oracle: &Oracle) {
    let differences = differences(pixels, oracle);
    if differences > 0 {
        let first = pixels.chunks_exact(4).zip(&oracle.pixels).enumerate().find(
            |(_, (actual, expected))| {
                expected.is_some_and(|e| {
                    actual
                        .iter()
                        .zip(e)
                        .any(|(&a, b)| a.abs_diff(b) > COLOR_EPSILON)
                })
            },
        );
        panic!("{label}: {differences} analytic oracle mismatches, first {first:?}");
    }
}
fn close(actual: [f32; 3], expected: [f64; 3], label: &str) {
    assert!(
        actual
            .iter()
            .zip(expected)
            .all(|(&a, b)| (f64::from(a) - b).abs() < 3e-5),
        "{label}: {actual:?} != {expected:?}"
    );
}
fn assert_geometry(baked: &StaticModel) {
    assert_eq!(baked.source().primitives.len(), 2);
    for (part, p) in baked.source().primitives.iter().enumerate() {
        assert_eq!(p.transform, IDENTITY);
        assert_eq!(p.indices, vec![0, 1, 2, 0, 2, 3]);
        for (i, v) in p.vertices.iter().enumerate() {
            close(
                v.position,
                expected_positions(part)[i],
                "independent baked position",
            );
            close(v.normal, normal(affine(part)), "independent baked normal");
            assert_eq!(v.uv, UV[i].map(|v| v as f32));
        }
    }
}
fn wrong_blended_normal() -> [f32; 3] {
    // Deliberately wrong: blend each joint's inverse transpose instead of
    // inverse-transposing the weighted joint matrix. Do not normalize each joint.
    let mut sum = [0.0; 3];
    for (j, w) in WEIGHTS.into_iter().enumerate() {
        let [a, b, c, d, e, _, _] = joint_affine(j, 0.5);
        let det = a * d - b * c;
        let n = [(d - b) / det, (a - c) / det, 1.0 / e];
        for k in 0..3 {
            sum[k] += w * n[k];
        }
    }
    normalize(sum).map(|v| v as f32)
}

#[test]
#[ignore = "requires fresh real producer, mandatory GPU and capture directory"]
fn fresh_cli_pose_bake_matches_independent_gpu_oracle() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let files = Files::new();
    let model = fixture(WEIGHTS);
    let baked = bake(&files, "midpoint", &model, 0.5);
    assert_geometry(&baked);
    assert_eq!(baked.source().materials, model.source().materials);
    assert_eq!(baked.source().images, model.source().images);
    let g = Wgpu::headless(WgpuOptions::default())
        .expect("required GPU unavailable; skipping forbidden");
    let info = g.adapter().get_info();
    let adapter = json!({"name":info.name,"backend":format!("{:?}",info.backend),"driver":info.driver,
        "driver_info":info.driver_info,"software":g.is_software()});
    eprintln!("pose-bake adapter: {adapter}");
    let target = g.create_texture(&TextureDesc {
        label: "pose-bake acceptance",
        width: SIZE,
        height: SIZE,
        format: FORMAT,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let expected = oracle();
    let static_pixels = render_static(&g, &target, baked.clone());
    files.frame("static-midpoint", &static_pixels);
    assert_oracle("static baked", &static_pixels, &expected);
    let pose = model.sample_clip(0, 0.5).unwrap();
    let mut skinned = SkinnedModelRenderer::new(g.clone(), FORMAT, model.clone()).unwrap();
    skinned.clear = [0.0, 0.0, 0.0, 1.0];
    let view = g.create_texture_view(&target, None);
    skinned
        .draw(
            &view,
            (SIZE, SIZE),
            &camera(),
            &light(),
            &[SkinnedInstance {
                pose: &pose,
                transform: placement(),
            }],
        )
        .unwrap();
    let animated_pixels = g.read_texture(&target);
    files.frame("skinned-midpoint", &animated_pixels);
    assert_oracle("unchanged GPU skinned", &animated_pixels, &expected);
    // Compare on the independent confidence mask, never let renderer agreement
    // substitute for the two independent expected-color checks above.
    for ((a, b), expected) in static_pixels
        .chunks_exact(4)
        .zip(animated_pixels.chunks_exact(4))
        .zip(&expected.pixels)
    {
        if expected.is_some() {
            assert!(
                a.iter()
                    .zip(b)
                    .all(|(a, b)| a.abs_diff(*b) <= COLOR_EPSILON)
            );
        }
    }
    for axis in 0..3 {
        let points: Vec<_> = (0..2).flat_map(expected_positions).collect();
        let lo = points
            .iter()
            .map(|p| p[axis] * PLACEMENT_SCALE[axis] + [0.1, 0.2, 0.0][axis])
            .fold(f64::INFINITY, f64::min);
        let hi = points
            .iter()
            .map(|p| p[axis] * PLACEMENT_SCALE[axis] + [0.1, 0.2, 0.0][axis])
            .fold(f64::NEG_INFINITY, f64::max);
        let bounds = skinned.bounds()[0];
        assert!(
            (f64::from(bounds.min[axis]) - lo).abs() < 3e-5
                && (f64::from(bounds.max[axis]) - hi).abs() < 3e-5
        );
    }
    let wrong_pose = bake(&files, "wrong-pose", &model, 0.0);
    let wrong_weights = bake(&files, "wrong-weights", &fixture([1.0, 0.0, 0.0, 0.0]), 0.5);
    let mut doubled = baked.source().clone();
    doubled.primitives[0].transform[3][0] = 7.0;
    let mut bad_normals = baked.source().clone();
    for v in &mut bad_normals.primitives[0].vertices {
        v.normal = wrong_blended_normal();
    }
    let mut controls = Vec::<Value>::new();
    for (name, model) in [
        ("wrong-pose", wrong_pose),
        ("wrong-weights", wrong_weights),
        ("double-mesh-transform", StaticModel::new(doubled).unwrap()),
        (
            "blended-normal-matrices",
            StaticModel::new(bad_normals).unwrap(),
        ),
    ] {
        let pixels = render_static(&g, &target, model);
        files.frame(name, &pixels);
        let count = differences(&pixels, &expected);
        assert!(
            count >= MIN_DIFFERENCES,
            "negative control {name} insufficiently sensitive: {count}"
        );
        controls.push(json!({"name":name,"oracle_mismatches":count}));
    }
    let report = json!({"adapter":adapter,"size":SIZE,"tolerance":COLOR_EPSILON,"interior":expected.interior,
        "clear":expected.clear,"static_oracle_mismatches":0,"skinned_oracle_mismatches":0,"controls":controls});
    files.write("report.json", &serde_json::to_vec_pretty(&report).unwrap());
    eprintln!("ORR_ANIMATION_POSE_BAKE_GPU {report}");
}
