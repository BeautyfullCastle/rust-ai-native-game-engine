//! Mandatory installed-cooked linear-RGB factor variant GPU acceptance. The current-source
//! CLI is supplied by ORR_STATIC_MODEL_RECOLOR_BIN; no authoring fallback exists.
//! Run --ignored --exact with ORR_REQUIRE_GPU=1 and a fresh absolute
//! ORR_STATIC_MODEL_RECOLOR_CAPTURE_DIR. Missing evidence/device is a failure.
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
const SOURCE_PATH: &str = "inputs/prop.model.json";
const PACKAGE: &str = "static-recolor-gpu";
const IDS: [&str; 2] = ["variants/rose.recolor.json", "variants/teal.recolor.json"];
const ASSETS: [&str; 2] = ["rose.model.json", "teal.model.json"];
const FACTORS: [[f64; 3]; 2] = [[0.65, 0.22, 0.48], [0.18, 0.72, 0.34]];
const OLD_FACTOR: [f64; 3] = [0.0, 0.4, 0.8];
const OTHER_FACTOR: [f64; 3] = [0.5, 0.9, 0.3];
const LIGHT: f64 = 0.6;
const SIZE: u32 = 256;
// Fixed before execution. Interior/background pixels have no mismatch allowance;
// 8% barycentric margins exclude edges and 2% texel margins exclude sampling ties.
const COLOR_EPSILON: u8 = 3;
const MIN_CONTROL_DIFFERENCES: usize = 500;
const MIN_UNAFFECTED_DIFFERENCES: usize = 100;
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
    [225, 83, 41, 0],
    [47, 198, 112, 63],
    [108, 56, 237, 149],
    [173, 219, 91, 231],
];

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "orr-static-recolor-gpu-{}-{}",
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
            std::env::var_os("ORR_STATIC_MODEL_RECOLOR_CAPTURE_DIR")
                .expect("mandatory acceptance requires ORR_STATIC_MODEL_RECOLOR_CAPTURE_DIR"),
        );
        assert!(path.is_absolute(), "capture directory must be absolute");
        fs::create_dir_all(&path).unwrap();
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
        eprintln!("ORR_STATIC_MODEL_RECOLOR_CAPTURE {record}");
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
fn source_model() -> StaticModel {
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
                [0.8 * sign, 0.0, 0.0, 0.0],
                [0.0, 0.65, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, y, 0.1, 1.0],
            ];
            source.primitives.push(primitive);
        }
    }
    source.images[0] = Image {
        width: 2,
        height: 2,
        rgba8: CHECKER.concat(),
    };
    source.materials[0].base_color = [0.0, 0.4, 0.8, 0.25];
    source.materials[1].image = 0;
    source.materials[1].base_color = [0.5, 0.9, 0.3, 0.75];
    source.materials[0].wrap_s = Wrap::Mirror;
    source.materials[0].wrap_t = Wrap::Repeat;
    source.materials[1].base_color[3] = 0.75;
    source.images.push(Image {
        width: 1,
        height: 1,
        rgba8: vec![17, 31, 47, 63],
    });
    let mut unused = source.materials[0].clone();
    unused.image = 2;
    unused.base_color = [0.25, 0.5, 0.75, 0.125];
    unused.linear_filter = true;
    source.materials.push(unused);
    StaticModel::new(source).unwrap()
}
fn produce(recipe: &Path, asset_id: &str, output: &Path) {
    let binary = PathBuf::from(
        std::env::var_os("ORR_STATIC_MODEL_RECOLOR_BIN")
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
fn install(scratch: &Scratch, captures: &Captures) -> (StaticModel, Vec<StaticModel>, Value) {
    let glb_record = captures.write("source.glb", ORIGINAL);
    let original = source_model();
    let source_bytes = original.to_bytes().unwrap();
    let source_record = captures.write("source.model.json", &source_bytes);
    let authoring = scratch.0.join("authoring");
    fs::create_dir_all(authoring.join("inputs")).unwrap();
    fs::write(authoring.join(SOURCE_PATH), &source_bytes).unwrap();
    let package_source = scratch.0.join("authored-package");
    fs::create_dir(&package_source).unwrap();
    let mut cooked = Vec::new();
    let mut records = Vec::new();
    let mut primitive_ids = BTreeSet::new();
    for (index, &asset) in ASSETS.iter().enumerate() {
        let recipe = serde_json::to_vec(&json!({"format":"orr_static_material_color","version":1,
            "source":SOURCE_PATH,"material":0,"linear_rgb":FACTORS[index]}))
        .unwrap();
        let recipe_name = Path::new(IDS[index]).file_name().unwrap();
        let recipe_record = captures.write(recipe_name.to_str().unwrap(), &recipe);
        let recipe_path = authoring.join(recipe_name);
        fs::write(&recipe_path, &recipe).unwrap();
        produce(&recipe_path, IDS[index], &package_source.join(asset));
        let bytes = fs::read(package_source.join(asset)).unwrap();
        let cooked_record = captures.write(asset, &bytes);
        let variant = StaticModel::from_bytes(&bytes).unwrap();
        let before = original.source();
        let after = variant.source();
        assert_eq!(after.asset_id, IDS[index]);
        assert_eq!(after.primitives.len(), 4);
        assert_eq!(after.images, before.images);
        let mut materials = before.materials.clone();
        for (channel, &factor) in FACTORS[index].iter().enumerate() {
            materials[0].base_color[channel] = factor as f32;
        }
        // CPU serialized-data assertions, independent of the renderer's OPAQUE output alpha.
        assert_eq!(
            after.materials[0].base_color[3].to_bits(),
            before.materials[0].base_color[3].to_bits()
        );
        assert_eq!(after.materials, materials);
        for (old, new) in before.primitives.iter().zip(&after.primitives) {
            let mut expected = old.clone();
            expected.id = format!("{}{}", IDS[index], old.id.strip_prefix(SOURCE_ID).unwrap());
            assert_eq!(new, &expected);
            assert!(
                primitive_ids.insert(new.id.clone()),
                "variant primitive namespaces must be disjoint"
            );
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
    let root = scratch.0.join("project");
    fs::create_dir(&root).unwrap();
    Project::open_for_install(&root, Runtime::content_only().engine_version)
        .unwrap()
        .install(std::slice::from_ref(&package_source))
        .unwrap();
    fs::rename(&authoring, scratch.0.join("hidden-authoring")).unwrap();
    fs::rename(&package_source, scratch.0.join("hidden-package-source")).unwrap();
    assert!(!authoring.exists());
    assert!(!package_source.exists());
    assert!(!root.join(SOURCE_PATH).exists());
    let mut runtime = Runtime::content_only();
    runtime.capabilities.insert("models".into());
    let project = Project::open(root, runtime).unwrap();
    let lock = project.verify().unwrap();
    assert_eq!(lock.packages[PACKAGE].files.len(), 2);
    let mut installed = Vec::new();
    for (&asset, expected) in ASSETS.iter().zip(&cooked) {
        let bytes = project.read_asset(PACKAGE, asset).unwrap();
        assert_eq!(&bytes, expected);
        let model = StaticModel::from_bytes(&bytes).unwrap();
        assert_eq!(&model.to_bytes().unwrap(), expected);
        installed.push(model);
    }
    assert_eq!(
        fs::read(scratch.0.join("hidden-authoring").join(SOURCE_PATH)).unwrap(),
        source_bytes
    );
    (
        original,
        installed,
        json!({"original_glb":glb_record,"source":source_record,"variants":records,
        "source_hidden_after_install":true,"installed_files":2,"cooked_reload_before_render":true}),
    )
}

// IEC sRGB transfer functions evaluated independently in f64. Texture RGB is
// decoded before multiplying by the replacement linear factor and fixed light.
fn decode_srgb(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
fn oracle_color(texel: [u8; 4], factor: [f64; 3]) -> [u8; 4] {
    let mut out = [0, 0, 0, 255];
    for i in 0..3 {
        let linear = decode_srgb(f64::from(texel[i]) / 255.0) * factor[i] * LIGHT;
        let encoded = if linear <= 0.0031308 {
            12.92 * linear
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        };
        out[i] = (encoded.clamp(0.0, 1.0) * 255.0).round() as u8;
    }
    out
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

// Independent fixture-space oracle: no model/helper/renderer matrices, vertices,
// material values, image pixels, UVs or camera projection helpers are consulted.
// mode 0 retains the old factor; modes 1/2 replace its linear RGB.
// Constants below deliberately do not read cooked models, material values or renderer helpers.
fn assert_pixels(pixels: &[u8], placements: &[(f64, usize)]) -> Value {
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
                        (world[0] - center_x) / (0.8 * sign),
                        (world[1] - center_y) / 0.65,
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
                        let color = if triangle == 3 {
                            markers[model][node] += 1;
                            oracle_color(CHECKER[0], OTHER_FACTOR)
                        } else {
                            let texel = [local[0] + 1.0, 1.0 - local[1]];
                            if texel.iter().any(|v| !(0.02..=0.98).contains(&v.fract())) {
                                continue;
                            }
                            let slot = texel[1].floor() as usize * 2 + texel[0].floor() as usize;
                            assert!(slot < CHECKER.len());
                            checker[slot] += 1;
                            let factor = match mode {
                                0 => OLD_FACTOR,
                                1 => FACTORS[0],
                                2 => FACTORS[1],
                                _ => panic!("unknown oracle mode"),
                            };
                            selected[model][node] += 1;
                            oracle_color(CHECKER[slot], factor)
                        };
                        assert!(
                            expected.replace(color).is_none(),
                            "fixture panels must not overlap"
                        );
                    }
                }
            }
            let offset = (y * SIZE as usize + x) * 4;
            let actual = &pixels[offset..offset + 4];
            if let Some(color) = expected {
                assert!(
                    matches(actual, &color),
                    "independent interior {x},{y}: {actual:?} != {color:?}"
                );
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
            counts.iter().all(|&count| count > 500),
            "both selected users must be nonvacuous: {counts:?}"
        );
    }
    for counts in &markers {
        assert!(
            counts.iter().all(|&count| count > 40),
            "both unaffected markers must be nonvacuous: {counts:?}"
        );
    }
    if placements.iter().any(|&(_, mode)| mode == 0) {
        assert!(
            checker.iter().all(|&count| count > 100),
            "all asymmetric source quadrants must be sampled: {checker:?}"
        );
    }
    assert!(clear > 40_000);
    json!({"selected_pixels_per_model_node":selected,"unaffected_pixels_per_model_node":markers,
        "source_checker_pixels":checker,"clear_checked":clear})
}
fn assert_unchanged_markers(source: &[u8], magenta: &[u8], cyan: &[u8]) -> usize {
    let mut checked = 0;
    for y in 0..SIZE as usize {
        for x in 0..SIZE as usize {
            let world = [
                (x as f64 + 0.5 - 128.0) / 32.0,
                (128.0 - y as f64 - 0.5) / 32.0,
            ];
            for (sign, center_y) in [(-1.0, 1.25), (1.0, -1.25)] {
                let local = [world[0] / (0.8 * sign), (world[1] - center_y) / 0.65];
                let weights = barycentric(local, POSITIONS[5], POSITIONS[6], POSITIONS[7]);
                if weights.iter().all(|&v| v > 0.08) {
                    let offset = (y * SIZE as usize + x) * 4;
                    assert_eq!(&source[offset..offset + 4], &magenta[offset..offset + 4]);
                    assert_eq!(&source[offset..offset + 4], &cyan[offset..offset + 4]);
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
        ambient: LIGHT as f32,
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
fn static_color_variants_match_independent_gpu_oracle() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let captures = Captures::new();
    let scratch = Scratch::new();
    let (original, variants, assets) = install(&scratch, &captures);
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
    assert!(
        g.is_software(),
        "mandatory acceptance requires an actual software adapter"
    );
    assert_eq!(adapter_record["backend"], "Vulkan");
    let target = g.create_texture(&TextureDesc {
        label: "mandatory static linear color variants",
        width: SIZE,
        height: SIZE,
        format: TextureFormat::Rgba8UnormSrgb,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let source = render(&g, original.clone(), &target);
    let rose = render(&g, variants[0].clone(), &target);
    let teal = render(&g, variants[1].clone(), &target);
    let (together, reversed) = render_pair(&g, &variants, &target);
    let mut frames = json!({"source":captures.frame("source", &source),
        "rose":captures.frame("rose", &rose), "teal":captures.frame("teal", &teal),
        "variants_together":captures.frame("variants-together", &together),
        "variants_reversed":captures.frame("variants-reversed", &reversed)});
    let coverage = json!({"source":assert_pixels(&source, &[(0.0, 0)]),
        "rose":assert_pixels(&rose, &[(0.0, 1)]), "teal":assert_pixels(&teal, &[(0.0, 2)]),
        "variants_together":assert_pixels(&together, &[(-2.0, 1), (2.0, 2)])});
    let unaffected_checked = assert_unchanged_markers(&source, &rose, &teal);
    assert_eq!(
        together, reversed,
        "variant namespaces must survive reversed batch/cache use"
    );

    // Each counterfactual goes through the actual renderer, not an oracle-only
    // arithmetic check. The correct frame already passed every fixed interior.
    let mut controls = serde_json::Map::new();
    for label in [
        "unchanged_factor",
        "old_factor_multiplied",
        "srgb_input",
        "wrong_material",
        "only_first_user",
        "all_material_tint",
        "swapped_channels",
        "wrong_uv",
        "wrong_transform",
        "offscreen",
    ] {
        let mut bad = variants[0].source().clone();
        match label {
            "unchanged_factor" => bad.materials[0] = original.source().materials[0].clone(),
            "old_factor_multiplied" => {
                for (i, &factor) in OLD_FACTOR.iter().enumerate() {
                    bad.materials[0].base_color[i] *= factor as f32;
                }
            }
            "srgb_input" => {
                for (i, &factor) in FACTORS[0].iter().enumerate() {
                    bad.materials[0].base_color[i] = decode_srgb(factor) as f32;
                }
            }
            "wrong_material" => {
                bad.materials[0] = original.source().materials[0].clone();
                for (i, &factor) in FACTORS[0].iter().enumerate() {
                    bad.materials[1].base_color[i] = factor as f32;
                }
            }
            "only_first_user" => {
                bad.materials.push(original.source().materials[0].clone());
                bad.primitives[2].material = 3;
            }
            "all_material_tint" => {
                for material in &mut bad.materials {
                    for (i, &factor) in FACTORS[0].iter().enumerate() {
                        material.base_color[i] = factor as f32;
                    }
                }
            }
            "swapped_channels" => bad.materials[0].base_color.swap(0, 2),
            "wrong_uv" => {
                for primitive in &mut bad.primitives {
                    for vertex in &mut primitive.vertices {
                        vertex.uv[0] = 1.0 - vertex.uv[0];
                    }
                }
            }
            "wrong_transform" => {
                for primitive in &mut bad.primitives {
                    primitive.transform = orr_model::IDENTITY;
                }
            }
            "offscreen" => {
                for primitive in &mut bad.primitives {
                    primitive.transform[3][0] += 100.0;
                }
            }
            _ => unreachable!(),
        }
        let pixels = render(&g, StaticModel::new(bad).unwrap(), &target);
        frames[label] = captures.frame(label, &pixels);
        let count = differences(&pixels, &rose);
        let minimum = if label == "all_material_tint" {
            MIN_UNAFFECTED_DIFFERENCES
        } else {
            MIN_CONTROL_DIFFERENCES
        };
        assert!(
            count > minimum,
            "rendered negative control {label}: {count} <= {minimum}"
        );
        controls.insert(label.to_owned(), json!(count));
    }
    assert!(differences(&rose, &teal) > MIN_CONTROL_DIFFERENCES);
    let receipt = json!({"schema":1,"status":"passed","width":SIZE,"height":SIZE,
        "adapter":adapter_record,"assets":assets,"frames":frames,"coverage":coverage,
        "control_pixel_differences":controls,"shared_texture_materials":true,
        "shared_material_users_checked":true,"unaffected_material_pixels_checked":unaffected_checked,
        "source_images_byte_identical":true,"source_factor_alpha_bits_preserved_cpu":true,
        "alpha_note":"CPU source alpha preservation is checked separately; OPAQUE framebuffer alpha does not prove source alpha preservation",
        "distinct_recipe_namespaces":true,"reversed_batch_order_byte_identical":true,
        "oracle":"independent f64 sRGB decode, replacement linear RGB, fixed 0.6 light, sRGB encode",
        "tolerances":{"color_bytes":COLOR_EPSILON,"minimum_control_differences":MIN_CONTROL_DIFFERENCES,
            "minimum_unaffected_differences":MIN_UNAFFECTED_DIFFERENCES}});
    captures.write(
        "receipt.json",
        &serde_json::to_vec_pretty(&receipt).unwrap(),
    );
    eprintln!("ORR_STATIC_MODEL_RECOLOR_ACCEPTANCE {receipt}");
}
