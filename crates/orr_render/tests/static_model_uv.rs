//! Mandatory installed-cooked UV-variant GPU acceptance. The current-source
//! CLI is supplied by ORR_STATIC_MODEL_UV_BIN; no authoring fallback exists.
//! Run --ignored --exact with ORR_REQUIRE_GPU=1 and a fresh absolute
//! ORR_STATIC_MODEL_UV_CAPTURE_DIR. Missing evidence/device is a failure.
#![cfg(all(feature = "models", feature = "imported-scene"))]
#![allow(clippy::float_arithmetic)]

use orr_model::{Image, StaticModel, Wrap};
use orr_package::{Project, Runtime};
use orr_render::orr_rhi::{Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions};
use orr_render::{
    Camera3D, ImportedBatch, ImportedSceneRenderer, ImportedSceneTarget, Lighting, ModelRenderer,
    PointLightSettings, StaticInstance,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

const ORIGINAL: &[u8] = include_bytes!("../../orr_model/tests/fixtures/model.glb");
const SOURCE_ID: &str = "scene/source.glb";
const SOURCE_PATH: &str = "inputs/source.model.json";
const PACKAGE: &str = "static-uv-gpu";
const IDS: [&str; 2] = ["variants/rotate.uv.json", "variants/counter.uv.json"];
const ASSETS: [&str; 2] = ["rotate.model.json", "counter.model.json"];
const SIZE: u32 = 256;
// Fixed before execution. Interior/background pixels have no mismatch allowance;
// 8% barycentric margins exclude edges and 2% texel margins exclude sampling ties.
const COLOR_EPSILON: u8 = 3;
const MIN_CONTROL_DIFFERENCES: usize = 500;
const MIN_UNAFFECTED_DIFFERENCES: usize = 30;
const POSITIONS: [[f64; 2]; 8] = [
    [-1.0, -1.0],
    [1.0, -1.0],
    [1.0, 0.4],
    [0.4, 1.0],
    [-1.0, 1.0],
    [1.1, -0.75],
    [1.8, -0.75],
    [1.4, 0.4],
];
const TRIANGLES: [[usize; 3]; 4] = [[0, 1, 2], [0, 2, 3], [0, 3, 4], [5, 6, 7]];
const CHECKER: [[u8; 4]; 4] = [
    [255, 0, 0, 255],
    [0, 255, 0, 255],
    [0, 0, 255, 255],
    [128, 128, 128, 255],
];

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "orr-static-uv-gpu-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("fresh owned fixture directory");
        Self(fs::canonicalize(path).unwrap())
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn fnv1a64(bytes: &[u8]) -> String {
    format!(
        "{:016x}",
        bytes.iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
    )
}
struct Captures(PathBuf);
impl Captures {
    fn new() -> Self {
        let path = PathBuf::from(
            std::env::var_os("ORR_STATIC_MODEL_UV_CAPTURE_DIR")
                .expect("mandatory acceptance requires ORR_STATIC_MODEL_UV_CAPTURE_DIR"),
        );
        assert!(path.is_absolute(), "capture directory must be absolute");
        fs::create_dir_all(&path).unwrap();
        assert_eq!(
            fs::read_dir(&path).unwrap().count(),
            0,
            "fresh capture directory required"
        );
        Self(fs::canonicalize(path).unwrap())
    }
    fn write(&self, name: &str, bytes: &[u8]) -> Value {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.0.join(name))
            .expect("capture output must be fresh; never overwrite prior evidence");
        file.write_all(bytes).unwrap();
        file.sync_all().unwrap();
        let record = json!({"file":name,"bytes":bytes.len(),"fnv1a64":fnv1a64(bytes)});
        eprintln!("ORR_STATIC_MODEL_UV_CAPTURE {record}");
        record
    }
    fn frame(&self, name: &str, pixels: &[u8]) -> Value {
        assert_eq!(pixels.len(), (SIZE * SIZE * 4) as usize);
        let raw = self.write(&format!("{name}.rgba"), pixels);
        let mut ppm = format!("P6\n{SIZE} {SIZE}\n255\n").into_bytes();
        for pixel in pixels.chunks_exact(4) {
            ppm.extend_from_slice(&pixel[..3]);
        }
        let image = self.write(&format!("{name}.ppm"), &ppm);
        json!({"raw":raw,"image":image})
    }
}
// The fixture source is authored independently from the CLI. Its two material
// slots intentionally reference image0; every UV is an affine function of local
// position. Two mirrored/transformed nodes use each slot and slot2 is unused.
fn source_model(linear_filter: bool, wrap: Wrap) -> StaticModel {
    let imported = orr_model::import::import_with_resolver(SOURCE_ID, ORIGINAL, |_| {
        panic!("embedded GLB must not resolve external inputs")
    })
    .unwrap();
    let mut source = imported.source().clone();
    let original_primitives = source.primitives.clone();
    source.primitives.clear();
    for (node, sign, y) in [(1, -1.0, 1.25), (7, 1.0, -1.25)] {
        for (slot, primitive) in original_primitives.iter().enumerate() {
            let mut primitive = primitive.clone();
            primitive.id = format!("{SOURCE_ID}#node={node}/mesh=0/primitive={slot}");
            primitive.transform = [
                [0.75 * sign, 0.0, 0.0, 0.0],
                [0.0, 0.75, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, y, 0.1, 1.0],
            ];
            for vertex in &mut primitive.vertices {
                vertex.uv = [
                    (vertex.position[0] + 1.0) * 0.5,
                    (1.0 - vertex.position[1]) * 0.5,
                ];
            }
            source.primitives.push(primitive);
        }
    }
    source.images[0] = Image {
        width: 2,
        height: 2,
        rgba8: CHECKER.concat(),
    };
    for (index, material) in source.materials.iter_mut().enumerate() {
        material.base_color = [1.0, 1.0, 1.0, if index == 0 { 0.25 } else { 0.75 }];
        material.image = 0;
        material.linear_filter = linear_filter;
        material.wrap_s = wrap;
        material.wrap_t = wrap;
    }
    source.images.push(Image {
        width: 1,
        height: 1,
        rgba8: vec![17, 31, 47, 63],
    });
    let mut unused = source.materials[0].clone();
    unused.image = 2;
    unused.base_color = [0.25, 0.5, 0.75, 0.125];
    source.materials.push(unused);
    StaticModel::new(source).unwrap()
}
fn produce(recipe: &Path, asset_id: &str, output: &Path) {
    let binary = PathBuf::from(
        std::env::var_os("ORR_STATIC_MODEL_UV_BIN")
            .expect("mandatory consumer requires the current-source pinned CLI"),
    );
    assert!(binary.is_absolute(), "producer path must be absolute");
    assert!(fs::metadata(&binary).unwrap().is_file());
    let result = Command::new(binary)
        .arg("--recipe")
        .arg(recipe)
        .arg("--asset-id")
        .arg(asset_id)
        .arg("--output")
        .arg(output)
        .output()
        .expect("execute pinned current-source producer");
    assert!(
        result.status.success(),
        "producer failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}
fn install(
    scratch: &Scratch,
    captures: &Captures,
    case: &str,
    linear: bool,
    wrap: Wrap,
) -> (StaticModel, Vec<StaticModel>, Value) {
    let original = source_model(linear, wrap);
    let source_bytes = original.to_bytes().unwrap();
    let source_record = captures.write(&format!("{case}-source.model.json"), &source_bytes);
    let authoring = scratch.0.join(case).join("authoring");
    fs::create_dir_all(authoring.join("inputs")).unwrap();
    fs::write(authoring.join(SOURCE_PATH), &source_bytes).unwrap();
    let package_source = scratch.0.join(case).join("authored-package");
    fs::create_dir(&package_source).unwrap();
    let mut cooked = Vec::new();
    let mut records = Vec::new();
    let mut primitive_ids = BTreeSet::new();
    for (index, &asset) in ASSETS.iter().enumerate() {
        let recipe = serde_json::to_vec(&json!({
            "format":"orr_static_material_uv","version":1,"source":SOURCE_PATH,"material":0,
            "offset":if index == 0 { [0.25,0.125] } else { [-0.375,0.75] },
            "scale":if index == 0 { [1.5,0.5] } else { [0.5,1.5] },
            "quarter_turns":if index == 0 {1} else {3}
        }))
        .unwrap();
        let recipe_name = Path::new(IDS[index]).file_name().unwrap();
        let recipe_record = captures.write(
            &format!("{case}-{}", recipe_name.to_str().unwrap()),
            &recipe,
        );
        let recipe_path = authoring.join(recipe_name);
        fs::write(&recipe_path, &recipe).unwrap();
        produce(&recipe_path, IDS[index], &package_source.join(asset));
        let bytes = fs::read(package_source.join(asset)).unwrap();
        let cooked_record = captures.write(&format!("{case}-{asset}"), &bytes);
        let variant = StaticModel::from_bytes(&bytes).unwrap();
        let before = original.source();
        let after = variant.source();
        assert_eq!(after.asset_id, IDS[index]);
        assert_eq!(after.primitives.len(), 4);
        assert_eq!(after.images, before.images);
        assert_eq!(after.materials, before.materials);
        for (old, new) in before.primitives.iter().zip(&after.primitives) {
            let mut expected = old.clone();
            expected.id = format!("{}{}", IDS[index], old.id.strip_prefix(SOURCE_ID).unwrap());
            if expected.material == 0 {
                for vertex in &mut expected.vertices {
                    let [u, v] = vertex.uv;
                    vertex.uv = if index == 0 {
                        [0.25 - 0.5 * v, 0.125 + 1.5 * u]
                    } else {
                        [-0.375 + 1.5 * v, 0.75 - 0.5 * u]
                    };
                }
            }
            assert_eq!(new, &expected);
            assert!(primitive_ids.insert(new.id.clone()));
        }
        assert_eq!(
            after.primitives.iter().filter(|p| p.material == 0).count(),
            2
        );
        assert_eq!(
            after
                .dependencies
                .iter()
                .map(|d| d.uri.as_str())
                .collect::<Vec<_>>(),
            ["$source", SOURCE_PATH]
        );
        assert_eq!(variant.to_bytes().unwrap(), bytes);
        cooked.push(bytes);
        records.push(json!({"asset_id":IDS[index],"recipe":recipe_record,"cooked":cooked_record}));
    }
    assert_eq!(primitive_ids.len(), 8);
    assert_ne!(cooked[0], cooked[1]);
    fs::write(
        package_source.join("orr.package.json"),
        serde_json::to_vec(&json!({
            "schema":1,"name":PACKAGE,"version":"1.0.0","engine":"^0.0.1",
            "capabilities":["models"],"dependencies":{},"files":ASSETS
        }))
        .unwrap(),
    )
    .unwrap();
    let root = scratch.0.join(case).join("project");
    fs::create_dir(&root).unwrap();
    Project::open_for_install(&root, Runtime::content_only().engine_version)
        .unwrap()
        .install(std::slice::from_ref(&package_source))
        .unwrap();
    fs::rename(&authoring, scratch.0.join(case).join("hidden-authoring")).unwrap();
    fs::rename(
        &package_source,
        scratch.0.join(case).join("hidden-package-source"),
    )
    .unwrap();
    assert!(!authoring.exists() && !package_source.exists() && !root.join(SOURCE_PATH).exists());
    let mut runtime = Runtime::content_only();
    runtime.capabilities.insert("models".into());
    let project = Project::open(root, runtime).unwrap();
    assert_eq!(project.verify().unwrap().packages[PACKAGE].files.len(), 2);
    let installed = ASSETS
        .iter()
        .zip(&cooked)
        .map(|(&asset, expected)| {
            let bytes = project.read_asset(PACKAGE, asset).unwrap();
            assert_eq!(&bytes, expected);
            let model = StaticModel::from_bytes(&bytes).unwrap();
            assert_eq!(&model.to_bytes().unwrap(), expected);
            model
        })
        .collect();
    assert_eq!(
        fs::read(
            scratch
                .0
                .join(case)
                .join("hidden-authoring")
                .join(SOURCE_PATH)
        )
        .unwrap(),
        source_bytes
    );
    (
        original,
        installed,
        json!({"source":source_record,"variants":records,
        "source_hidden_after_install":true,"installed_files":2,"cooked_reload_before_render":true}),
    )
}

fn barycentric(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> [f64; 3] {
    let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    let u = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / d;
    let v = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / d;
    [u, v, 1.0 - u - v]
}
fn matches(actual: &[u8], expected: &[u8]) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(a, b)| a.abs_diff(*b) <= COLOR_EPSILON)
}
fn differences(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len());
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(a, b)| !matches(a, b))
        .count()
}

// Scalar texture oracle, independent from model/renderer sampling utilities.
// Normalized 2x2 texture coordinates have texel centers at .25 and .75. Linear
// filtering interpolates decoded sRGB values before framebuffer sRGB encoding.
fn address(index: i64, wrap: Wrap) -> usize {
    match wrap {
        Wrap::Clamp => index.clamp(0, 1) as usize,
        Wrap::Repeat => index.rem_euclid(2) as usize,
        Wrap::Mirror => match index.rem_euclid(4) {
            0 | 3 => 0,
            1 | 2 => 1,
            _ => unreachable!(),
        },
    }
}
fn linear_channel(value: u8) -> f64 {
    let v = f64::from(value) / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
fn srgb_channel(value: f64) -> u8 {
    let v = if value <= 0.0031308 {
        12.92 * value
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}
fn sample(uv: [f64; 2], linear: bool, wrap: Wrap) -> Option<([u8; 4], usize)> {
    let scaled = [uv[0] * 2.0, uv[1] * 2.0];
    if !linear {
        // Exclude exact nearest-texel boundaries instead of granting mismatches.
        if scaled.iter().any(|v| (v - v.round()).abs() < 0.02) {
            return None;
        }
        let slot =
            address(scaled[1].floor() as i64, wrap) * 2 + address(scaled[0].floor() as i64, wrap);
        return Some((CHECKER[slot], slot));
    }
    let position = [scaled[0] - 0.5, scaled[1] - 0.5];
    let base = [position[0].floor() as i64, position[1].floor() as i64];
    let frac = [
        position[0] - position[0].floor(),
        position[1] - position[1].floor(),
    ];
    let mut out = [0, 0, 0, 255];
    for (channel, byte) in out.iter_mut().enumerate().take(3) {
        let mut value = 0.0;
        for dy in 0..2 {
            for dx in 0..2 {
                let slot = address(base[1] + dy, wrap) * 2 + address(base[0] + dx, wrap);
                let weight = (if dx == 0 { 1.0 - frac[0] } else { frac[0] })
                    * (if dy == 0 { 1.0 - frac[1] } else { frac[1] });
                value += weight * linear_channel(CHECKER[slot][channel]);
            }
        }
        *byte = srgb_channel(value);
    }
    let slot =
        address(scaled[1].floor() as i64, wrap) * 2 + address(scaled[0].floor() as i64, wrap);
    Some((out, slot))
}
// No live model geometry, matrices, UVs, recipe parser or producer helper is
// consulted: local coordinates, camera mapping and affine equations are explicit.
fn assert_pixels(pixels: &[u8], placements: &[(f64, usize)], linear: bool, wrap: Wrap) -> Value {
    assert_eq!(pixels.len(), (SIZE * SIZE * 4) as usize);
    let mut selected = vec![[0_usize; 2]; placements.len()];
    let mut markers = vec![[0_usize; 2]; placements.len()];
    let mut checker = [0_usize; 4];
    let mut clear = 0;
    for y in 0..SIZE as usize {
        for x in 0..SIZE as usize {
            let world = [
                (x as f64 + 0.5 - 128.0) / 32.0,
                (128.0 - y as f64 - 0.5) / 32.0,
            ];
            let mut near_triangle = false;
            let mut expected = None;
            for (model, &(center_x, mode)) in placements.iter().enumerate() {
                for (node, (sign, center_y)) in [(-1.0, 1.25), (1.0, -1.25)].into_iter().enumerate()
                {
                    let local = [
                        (world[0] - center_x) / (0.75 * sign),
                        (world[1] - center_y) / 0.75,
                    ];
                    for (triangle, ids) in TRIANGLES.iter().enumerate() {
                        let weights = barycentric(
                            local,
                            POSITIONS[ids[0]],
                            POSITIONS[ids[1]],
                            POSITIONS[ids[2]],
                        );
                        near_triangle |= weights.iter().all(|&v| v >= -0.08);
                        if !weights.iter().all(|&v| v > 0.08) {
                            continue;
                        }
                        let u = (local[0] + 1.0) * 0.5;
                        let v = (1.0 - local[1]) * 0.5;
                        let uv = if triangle == 3 || mode == 0 {
                            [u, v]
                        } else if mode == 1 {
                            [0.25 - 0.5 * v, 0.125 + 1.5 * u]
                        } else if mode == 2 {
                            [-0.375 + 1.5 * v, 0.75 - 0.5 * u]
                        } else {
                            panic!("unknown oracle mode")
                        };
                        let Some((color, slot)) = sample(uv, linear, wrap) else {
                            continue;
                        };
                        if triangle == 3 {
                            markers[model][node] += 1;
                        } else {
                            selected[model][node] += 1;
                            if mode == 0 {
                                checker[slot] += 1;
                            }
                        }
                        assert!(expected.replace(color).is_none(), "fixture panels overlap");
                    }
                }
            }
            let offset = (y * SIZE as usize + x) * 4;
            let actual = &pixels[offset..offset + 4];
            if let Some(color) = expected {
                assert!(matches(actual,&color),"independent interior {x},{y}: {actual:?} != {color:?}; linear={linear}, wrap={wrap:?}");
            } else if !near_triangle {
                assert!(
                    matches(actual, &[0, 0, 0, 255]),
                    "independent background {x},{y}: {actual:?}"
                );
                clear += 1;
            }
        }
    }
    for counts in &selected {
        assert!(
            counts.iter().all(|&n| n > 500),
            "selected users vacuous: {counts:?}"
        );
    }
    for counts in &markers {
        assert!(
            counts.iter().all(|&n| n > 40),
            "unselected users vacuous: {counts:?}"
        );
    }
    if placements.iter().any(|&(_, mode)| mode == 0) {
        assert!(
            checker.iter().all(|&n| n > 100),
            "source quadrants vacuous: {checker:?}"
        );
    }
    assert!(clear > 40_000);
    json!({"selected_pixels_per_model_node":selected,"unaffected_pixels_per_model_node":markers,
        "source_checker_pixels":checker,"clear_checked":clear})
}
fn assert_unchanged_markers(source: &[u8], rotate: &[u8], counter: &[u8]) -> usize {
    let mut checked = 0;
    for y in 0..SIZE as usize {
        for x in 0..SIZE as usize {
            let world = [
                (x as f64 + 0.5 - 128.0) / 32.0,
                (128.0 - y as f64 - 0.5) / 32.0,
            ];
            for (sign, center_y) in [(-1.0, 1.25), (1.0, -1.25)] {
                let local = [world[0] / (0.75 * sign), (world[1] - center_y) / 0.75];
                let weights = barycentric(local, POSITIONS[5], POSITIONS[6], POSITIONS[7]);
                if weights.iter().all(|&v| v > 0.08) {
                    let offset = (y * SIZE as usize + x) * 4;
                    assert_eq!(&source[offset..offset + 4], &rotate[offset..offset + 4]);
                    assert_eq!(&source[offset..offset + 4], &counter[offset..offset + 4]);
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 100);
    checked
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
fn camera() -> Camera3D {
    Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 4.0)
}
fn render(g: &Wgpu, model: StaticModel, target: &<Wgpu as Rhi>::Texture) -> Vec<u8> {
    let mut renderer = ModelRenderer::new(g.clone(), TextureFormat::Rgba8UnormSrgb, model).unwrap();
    renderer.clear = [0.0, 0.0, 0.0, 1.0];
    let view = g.create_texture_view(target, None);
    renderer
        .draw(&view, (SIZE, SIZE), &camera(), &lighting())
        .unwrap();
    g.read_texture(target)
}
fn render_pair(
    g: &Wgpu,
    models: &[StaticModel],
    target: &<Wgpu as Rhi>::Texture,
) -> (Vec<u8>, Vec<u8>) {
    let format = TextureFormat::Rgba8UnormSrgb;
    let mut scene = ImportedSceneRenderer::new(g.clone(), format).unwrap();
    scene.clear = [0.0, 0.0, 0.0, 1.0];
    let mut a = ModelRenderer::new(g.clone(), format, models[0].clone()).unwrap();
    let mut b = ModelRenderer::new(g.clone(), format, models[1].clone()).unwrap();
    let left = [StaticInstance {
        translation: [-2.0, 0.0, 0.0],
        ..StaticInstance::default()
    }];
    let right = [StaticInstance {
        translation: [2.0, 0.0, 0.0],
        ..StaticInstance::default()
    }];
    let view = g.create_texture_view(target, None);
    let mut frames = Vec::new();
    for reversed in [false, true] {
        let mut batches = if reversed {
            [
                ImportedBatch::StaticInstances {
                    renderer: &mut b,
                    instances: &right,
                },
                ImportedBatch::StaticInstances {
                    renderer: &mut a,
                    instances: &left,
                },
            ]
        } else {
            [
                ImportedBatch::StaticInstances {
                    renderer: &mut a,
                    instances: &left,
                },
                ImportedBatch::StaticInstances {
                    renderer: &mut b,
                    instances: &right,
                },
            ]
        };
        scene
            .draw(
                ImportedSceneTarget {
                    view: &view,
                    size: (SIZE, SIZE),
                    format,
                    sample_count: 1,
                },
                &camera(),
                &lighting(),
                &PointLightSettings::default(),
                &mut batches,
            )
            .unwrap();
        frames.push(g.read_texture(target));
    }
    (frames.remove(0), frames.remove(0))
}

#[test]
#[ignore = "requires pinned current-source producer, mandatory software GPU, and fresh captures"]
fn static_uv_variants_match_independent_gpu_oracle() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let captures = Captures::new();
    let scratch = Scratch::new();
    let original_glb = captures.write("source.glb", ORIGINAL);
    let g = Wgpu::headless(WgpuOptions {
        force_software: true,
        ..WgpuOptions::default()
    })
    .expect("mandatory software GPU acceptance cannot skip an unavailable adapter");
    let adapter = g.adapter().get_info();
    let adapter_record = json!({"name":adapter.name,"backend":format!("{:?}",adapter.backend),
        "device_type":format!("{:?}",adapter.device_type),"vendor":adapter.vendor,"device":adapter.device,
        "driver":adapter.driver,"driver_info":adapter.driver_info,"software":g.is_software()});
    captures.write(
        "adapter.json",
        &serde_json::to_vec_pretty(&adapter_record).unwrap(),
    );
    eprintln!("ORR_STATIC_MODEL_UV_ADAPTER {adapter_record}");
    assert!(g.is_software(), "actual software adapter required");
    assert_eq!(adapter_record["backend"], "Vulkan");
    let target = g.create_texture(&TextureDesc {
        label: "mandatory static UV variants",
        width: SIZE,
        height: SIZE,
        format: TextureFormat::Rgba8UnormSrgb,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let mut cases = serde_json::Map::new();
    for (linear, filter) in [(false, "nearest"), (true, "linear")] {
        for (wrap, wrap_name) in [
            (Wrap::Repeat, "repeat"),
            (Wrap::Mirror, "mirror"),
            (Wrap::Clamp, "clamp"),
        ] {
            let case = format!("{filter}-{wrap_name}");
            let (original, variants, mut assets) =
                install(&scratch, &captures, &case, linear, wrap);
            assets["original_glb"] = original_glb.clone();
            let source = render(&g, original.clone(), &target);
            let rotate = render(&g, variants[0].clone(), &target);
            let counter = render(&g, variants[1].clone(), &target);
            let (together, reversed) = render_pair(&g, &variants, &target);
            let mut wrong_order = variants[0].source().clone();
            for (primitive, old) in wrong_order
                .primitives
                .iter_mut()
                .zip(&original.source().primitives)
            {
                if primitive.material == 0 {
                    for (vertex, old) in primitive.vertices.iter_mut().zip(&old.vertices) {
                        let [u, v] = old.uv;
                        // Rotate first, scale second: deliberately wrong with nonuniform scale.
                        vertex.uv = [0.25 - 1.5 * v, 0.125 + 0.5 * u];
                    }
                }
            }
            let wrong_order = render(&g, StaticModel::new(wrong_order).unwrap(), &target);
            let mut all_material = variants[0].source().clone();
            for primitive in &mut all_material.primitives {
                if primitive.material == 1 {
                    for vertex in &mut primitive.vertices {
                        let [u, v] = vertex.uv;
                        vertex.uv = [0.25 - 0.5 * v, 0.125 + 1.5 * u];
                    }
                }
            }
            let all_material = render(&g, StaticModel::new(all_material).unwrap(), &target);
            let mut first_only = variants[0].source().clone();
            first_only.primitives[2].vertices = original.source().primitives[2].vertices.clone();
            let first_only = render(&g, StaticModel::new(first_only).unwrap(), &target);
            let mut wrong_transform = variants[0].source().clone();
            for primitive in &mut wrong_transform.primitives {
                primitive.transform = orr_model::IDENTITY;
            }
            let wrong_transform = render(&g, StaticModel::new(wrong_transform).unwrap(), &target);
            let frames = json!({
                "source":captures.frame(&format!("{case}-source"),&source),
                "rotate":captures.frame(&format!("{case}-rotate"),&rotate),
                "counter":captures.frame(&format!("{case}-counter"),&counter),
                "variants_together":captures.frame(&format!("{case}-variants-together"),&together),
                "variants_reversed":captures.frame(&format!("{case}-variants-reversed"),&reversed),
                "wrong_order":captures.frame(&format!("{case}-wrong-order"),&wrong_order),
                "all_material":captures.frame(&format!("{case}-all-material"),&all_material),
                "only_first_user":captures.frame(&format!("{case}-only-first-user"),&first_only),
                "wrong_transform":captures.frame(&format!("{case}-wrong-transform"),&wrong_transform),
            });
            let coverage = json!({"source":assert_pixels(&source,&[(0.0,0)],linear,wrap),
                "rotate":assert_pixels(&rotate,&[(0.0,1)],linear,wrap),
                "counter":assert_pixels(&counter,&[(0.0,2)],linear,wrap),
                "variants_together":assert_pixels(&together,&[(-2.0,1),(2.0,2)],linear,wrap)});
            let unaffected_checked = assert_unchanged_markers(&source, &rotate, &counter);
            assert_eq!(
                together, reversed,
                "batch/cache order changed disjoint variants"
            );
            let controls = json!({"unchanged_to_rotate":differences(&source,&rotate),
                "unchanged_to_counter":differences(&source,&counter),"distinct_variants":differences(&rotate,&counter),
                "wrong_order":differences(&wrong_order,&rotate),"all_material":differences(&all_material,&rotate),
                "shared_material_user":differences(&first_only,&rotate),"world_transform":differences(&wrong_transform,&rotate)});
            for (label, count) in controls.as_object().unwrap() {
                let minimum = if label == "all_material" {
                    MIN_UNAFFECTED_DIFFERENCES
                } else {
                    MIN_CONTROL_DIFFERENCES
                };
                assert!(
                    count.as_u64().unwrap() > minimum as u64,
                    "{case} negative control {label} is vacuous: {count}"
                );
            }
            let control_pairs = json!({"unchanged_to_rotate":["source","rotate"],
                "unchanged_to_counter":["source","counter"],"distinct_variants":["rotate","counter"],
                "wrong_order":["wrong_order","rotate"],"all_material":["all_material","rotate"],
                "shared_material_user":["only_first_user","rotate"],"world_transform":["wrong_transform","rotate"]});
            cases.insert(case,json!({"linear_filter":linear,"wrap_s":wrap_name,"wrap_t":wrap_name,
                "assets":assets,"frames":frames,"coverage":coverage,"control_pixel_differences":controls,
                "control_pairs":control_pairs,"shared_material_users_checked":true,
                "unaffected_material_byte_identical":true,"unaffected_material_pixels_checked":unaffected_checked,
                "same_image_unselected_material":true,"distinct_recipe_namespaces":true,
                "reversed_batch_order_byte_identical":true,"opaque_alpha_checked":true}));
        }
    }
    let binary = PathBuf::from(std::env::var_os("ORR_STATIC_MODEL_UV_BIN").unwrap());
    let receipt = json!({"schema":1,"status":"passed","width":SIZE,"height":SIZE,
        "adapter":adapter_record,"cases":cases,
        "producer_fnv1a64":fnv1a64(&fs::read(binary).unwrap()),
        "test_source_fnv1a64":fnv1a64(include_bytes!("static_model_uv.rs")),
        "cli_source_fnv1a64":fnv1a64(include_bytes!("../../orr_model/examples/static_model_uv.rs")),
        "authoring_source_fnv1a64":fnv1a64(include_bytes!("../../orr_model/examples/static_model_uv/authoring.rs")),
        "tolerances":{"color_bytes":COLOR_EPSILON,"minimum_control_differences":MIN_CONTROL_DIFFERENCES,
            "minimum_unaffected_differences":MIN_UNAFFECTED_DIFFERENCES}});
    captures.write(
        "receipt.json",
        &serde_json::to_vec_pretty(&receipt).unwrap(),
    );
    eprintln!("ORR_STATIC_MODEL_UV_ACCEPTANCE {receipt}");
}
