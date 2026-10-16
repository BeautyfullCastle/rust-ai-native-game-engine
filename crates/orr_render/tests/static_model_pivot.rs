//! Mandatory installed-cooked pivot-variant GPU acceptance. The current-source
//! CLI is supplied by ORR_STATIC_MODEL_PIVOT_BIN; no authoring fallback exists.
//! Run --ignored --exact with ORR_REQUIRE_GPU=1 and a fresh absolute
//! ORR_STATIC_MODEL_PIVOT_CAPTURE_DIR. Missing evidence/device is a failure.
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
const PACKAGE: &str = "static-pivot-gpu";
const IDS: [&str; 2] = [
    "variants/offset-a.pivot.json",
    "variants/offset-b.pivot.json",
];
const ASSETS: [&str; 2] = ["offset-a.model.json", "offset-b.model.json"];
const PIVOTS: [[f32; 3]; 2] = [[0.625, -0.375, 0.125], [-0.5, 0.75, 0.0]];
const SIZE: u32 = 256;
// Fixed before execution. Interior/background pixels have no mismatch allowance;
// 8% barycentric margins exclude edges and 2% texel margins exclude sampling ties.
const COLOR_EPSILON: u8 = 3;
const MIN_CONTROL_DIFFERENCES: usize = 200;
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
const MARKER: [u8; 4] = [188, 255, 0, 255];

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "orr-static-pivot-gpu-{}-{}",
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
            std::env::var_os("ORR_STATIC_MODEL_PIVOT_CAPTURE_DIR")
                .expect("mandatory acceptance requires ORR_STATIC_MODEL_PIVOT_CAPTURE_DIR"),
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
        eprintln!("ORR_STATIC_MODEL_PIVOT_CAPTURE {record}");
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
            primitive.transform = if sign < 0.0 {
                [
                    [-0.5, 0.0, 0.0, 0.0],
                    [0.0, 0.75, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [0.0, y, 0.125, 1.0],
                ]
            } else {
                [
                    [0.0, 0.5, 0.0, 0.0],
                    [-0.75, 0.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [0.0, y, 0.25, 1.0],
                ]
            };
            source.primitives.push(primitive);
        }
    }
    source.materials[0].base_color[3] = 0.25;
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
        std::env::var_os("ORR_STATIC_MODEL_PIVOT_BIN")
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
        let recipe = serde_json::to_vec(&json!({"format":"orr_static_model_pivot","version":1,
            "source":SOURCE_PATH,"pivot":PIVOTS[index]}))
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
        assert_eq!(after.materials, before.materials);
        for (old, new) in before.primitives.iter().zip(&after.primitives) {
            let mut expected = old.clone();
            expected.id = format!("{}{}", IDS[index], old.id.strip_prefix(SOURCE_ID).unwrap());
            for (axis, pivot) in PIVOTS[index].into_iter().enumerate() {
                if pivot != 0.0 {
                    expected.transform[3][axis] -= pivot;
                }
            }
            assert_eq!(new, &expected);
            for (actual_column, expected_column) in new.transform.iter().zip(&expected.transform) {
                for (actual, expected) in actual_column.iter().zip(expected_column) {
                    assert_eq!(actual.to_bits(), expected.to_bits());
                }
            }
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

fn barycentric(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> [f64; 3] {
    let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    let u = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / d;
    let v = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / d;
    [u, v, 1.0 - u - v]
}
fn matches(a: &[u8], b: &[u8]) -> bool {
    a.iter()
        .zip(b)
        .all(|(a, b)| a.abs_diff(*b) <= COLOR_EPSILON)
}
fn differences(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len());
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(a, b)| !matches(a, b))
        .count()
}

// Numeric fixture-space oracle, deliberately independent of helper/model matrices,
// actual vertex buffers, image tables, and renderer projection/composition helpers.
// E is a 180-degree Z rotation and nonuniform scale, then a world translation.
// Inverse E is scalar arithmetic; +pivot reverses T(-pivot), then inverse M.
fn assert_pixels(pixels: &[u8], placements: &[(usize, [f32; 3])]) -> Value {
    let mut panels = vec![[0_usize; 2]; placements.len()];
    let mut markers = vec![[0_usize; 2]; placements.len()];
    let mut checker = [0_usize; 4];
    let mut clear = 0;
    for y in 0..SIZE as usize {
        for x in 0..SIZE as usize {
            let world = [
                (x as f64 + 0.5 - 128.0) / 32.0,
                (128.0 - y as f64 - 0.5) / 32.0,
            ];
            let mut near = false;
            let mut expected = None;
            for (placement, &(instance, pivot)) in placements.iter().enumerate() {
                let (tx, sx, sy) = if instance == 0 {
                    (-2.0, 0.75, 0.5)
                } else {
                    (2.0, 0.5, 0.75)
                };
                let model = [
                    -(world[0] - tx) / sx + f64::from(pivot[0]),
                    -world[1] / sy + f64::from(pivot[1]),
                ];
                for node in 0..2 {
                    let local = if node == 0 {
                        [model[0] / -0.5, (model[1] - 1.25) / 0.75]
                    } else {
                        [(model[1] + 1.25) / 0.5, model[0] / -0.75]
                    };
                    for (triangle, ids) in TRIANGLES.iter().enumerate() {
                        let w = barycentric(
                            local,
                            POSITIONS[ids[0]],
                            POSITIONS[ids[1]],
                            POSITIONS[ids[2]],
                        );
                        near |= w.iter().all(|&v| v >= -0.08);
                        if !w.iter().all(|&v| v > 0.08) {
                            continue;
                        }
                        let color = if triangle == 3 {
                            markers[placement][node] += 1;
                            MARKER
                        } else {
                            let texel = [local[0] + 1.0, 1.0 - local[1]];
                            if texel.iter().any(|v| !(0.02..=0.98).contains(&v.fract())) {
                                continue;
                            }
                            let slot = texel[1].floor() as usize * 2 + texel[0].floor() as usize;
                            assert!(slot < CHECKER.len());
                            checker[slot] += 1;
                            panels[placement][node] += 1;
                            CHECKER[slot]
                        };
                        assert!(
                            expected.replace(color).is_none(),
                            "fixture surfaces must not overlap"
                        );
                    }
                }
            }
            let offset = (y * SIZE as usize + x) * 4;
            if let Some(color) = expected {
                assert!(
                    matches(&pixels[offset..offset + 4], &color),
                    "independent interior {x},{y}: {:?} != {color:?}",
                    &pixels[offset..offset + 4]
                );
            } else if !near {
                assert!(
                    matches(&pixels[offset..offset + 4], &[0, 0, 0, 255]),
                    "independent background {x},{y}"
                );
                clear += 1;
            }
        }
    }
    assert!(
        panels.iter().all(|pair| pair.iter().all(|&n| n > 120)),
        "nonvacuous both transformed nodes: {panels:?}"
    );
    assert!(
        markers.iter().all(|pair| pair.iter().all(|&n| n > 10)),
        "nonvacuous both material slots: {markers:?}"
    );
    assert!(checker.iter().all(|&n| n > 100));
    assert!(clear > 50_000);
    json!({"panel_pixels":panels,"marker_pixels":markers,"checker_pixels":checker,"clear_checked":clear})
}
fn instances(pivot: [f32; 3], sign: f32) -> [StaticInstance; 2] {
    [(-2.0, 0.75, 0.5), (2.0, 0.5, 0.75)].map(|(tx, sx, sy)| StaticInstance {
        // E*T(sign*pivot): Rz(pi)*S*p = [-sx*px,-sy*py,pz].
        // This compensates the ORIGINAL externally; it never edits source M.
        translation: [
            tx - sign * sx * pivot[0],
            -sign * sy * pivot[1],
            sign * pivot[2],
        ],
        rotation: [0.0, 0.0, 1.0, 0.0],
        scale: [sx, sy, 1.0],
    })
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
fn render(
    g: &Wgpu,
    models: &[StaticModel],
    placements: &[Vec<StaticInstance>],
    reverse: bool,
    target: &<Wgpu as Rhi>::Texture,
) -> Vec<u8> {
    let format = TextureFormat::Rgba8UnormSrgb;
    let mut scene = ImportedSceneRenderer::new(g.clone(), format).unwrap();
    scene.clear = [0.0, 0.0, 0.0, 1.0];
    let mut renderers: Vec<_> = models
        .iter()
        .map(|model| ModelRenderer::new(g.clone(), format, model.clone()).unwrap())
        .collect();
    let mut batches: Vec<_> = renderers
        .iter_mut()
        .zip(placements)
        .map(|(renderer, instances)| ImportedBatch::StaticInstances {
            renderer,
            instances,
        })
        .collect();
    if reverse {
        batches.reverse();
    }
    let view = g.create_texture_view(target, None);
    scene
        .draw(
            ImportedSceneTarget {
                view: &view,
                size: (SIZE, SIZE),
                format,
                sample_count: 1,
            },
            &Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 4.0),
            &lighting(),
            &PointLightSettings::default(),
            &mut batches,
        )
        .unwrap();
    g.read_texture(target)
}

#[test]
#[ignore = "requires pinned current-source CLI, actual software GPU, and fresh captures"]
fn static_pivot_variants_match_independent_compensated_original_oracle() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let captures = Captures::new();
    let scratch = Scratch::new();
    let (original, variants, assets) = install(&scratch, &captures);
    let original_bytes = original.to_bytes().unwrap();
    let variant_bytes: Vec<_> = variants.iter().map(|m| m.to_bytes().unwrap()).collect();
    // Explicit numeric geometry fixture verification, independent of pivot authoring.
    for (index, primitive) in original.source().primitives.iter().enumerate() {
        let range = if index % 2 == 0 { 0..5 } else { 5..8 };
        assert_eq!(primitive.vertices.len(), range.len());
        for (vertex, position) in primitive.vertices.iter().zip(range) {
            assert_eq!(
                vertex.position,
                [
                    POSITIONS[position][0] as f32,
                    POSITIONS[position][1] as f32,
                    0.0
                ]
            );
            assert_eq!(vertex.normal, [0.0, 0.0, 1.0]);
        }
    }
    let g = Wgpu::headless(WgpuOptions {
        force_software: true,
        ..Default::default()
    })
    .expect("mandatory software GPU acceptance cannot skip unavailable adapter");
    let adapter = g.adapter().get_info();
    let adapter_record = json!({"name":adapter.name,"backend":format!("{:?}",adapter.backend),
        "device_type":format!("{:?}",adapter.device_type),"driver":adapter.driver,"driver_info":adapter.driver_info,
        "software":g.is_software()});
    captures.write(
        "adapter.json",
        &serde_json::to_vec_pretty(&adapter_record).unwrap(),
    );
    assert!(g.is_software());
    assert_eq!(adapter_record["backend"], "Vulkan");
    let target = g.create_texture(&TextureDesc {
        label: "static pivot independent acceptance",
        width: SIZE,
        height: SIZE,
        format: TextureFormat::Rgba8UnormSrgb,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let base = instances([0.0; 3], 0.0).to_vec();
    let a = render(
        &g,
        &variants[..1],
        std::slice::from_ref(&base),
        false,
        &target,
    );
    let b = render(
        &g,
        &variants[1..],
        std::slice::from_ref(&base),
        false,
        &target,
    );
    let oracle_a = render(
        &g,
        std::slice::from_ref(&original),
        &[instances(PIVOTS[0], -1.0).to_vec()],
        false,
        &target,
    );
    let oracle_b = render(
        &g,
        std::slice::from_ref(&original),
        &[instances(PIVOTS[1], -1.0).to_vec()],
        false,
        &target,
    );
    let no_pivot = render(
        &g,
        std::slice::from_ref(&original),
        std::slice::from_ref(&base),
        false,
        &target,
    );
    let wrong_sign = render(
        &g,
        std::slice::from_ref(&original),
        &[instances(PIVOTS[0], 1.0).to_vec()],
        false,
        &target,
    );
    let mut wrong_order_instances = base.clone();
    for placement in &mut wrong_order_instances {
        for (translation, pivot) in placement.translation.iter_mut().zip(PIVOTS[0]) {
            *translation -= pivot;
        }
    }
    let wrong_order = render(
        &g,
        std::slice::from_ref(&original),
        &[wrong_order_instances],
        false,
        &target,
    );
    let mut wrong_uv = variants[0].source().clone();
    for primitive in &mut wrong_uv.primitives {
        for vertex in &mut primitive.vertices {
            vertex.uv[0] = 1.0 - vertex.uv[0];
        }
    }
    let wrong_uv = render(
        &g,
        &[StaticModel::new(wrong_uv).unwrap()],
        std::slice::from_ref(&base),
        false,
        &target,
    );
    let mut wrong_material = variants[0].source().clone();
    for primitive in &mut wrong_material.primitives {
        primitive.material = 1 - primitive.material;
    }
    let wrong_material = render(
        &g,
        &[StaticModel::new(wrong_material).unwrap()],
        std::slice::from_ref(&base),
        false,
        &target,
    );
    let together_placements = vec![vec![base[0]], vec![base[1]]];
    let together = render(&g, &variants, &together_placements, false, &target);
    let reversed = render(&g, &variants, &together_placements, true, &target);
    let frames = json!({"offset_a":captures.frame("offset-a",&a),"offset_b":captures.frame("offset-b",&b),
        "compensated_original_a":captures.frame("compensated-original-a",&oracle_a),
        "compensated_original_b":captures.frame("compensated-original-b",&oracle_b),
        "no_pivot":captures.frame("no-pivot",&no_pivot),"wrong_sign":captures.frame("wrong-sign",&wrong_sign),
        "wrong_order":captures.frame("wrong-order",&wrong_order),"wrong_uv":captures.frame("wrong-uv",&wrong_uv),
        "wrong_material":captures.frame("wrong-material",&wrong_material),
        "variants_together":captures.frame("variants-together",&together),"variants_reversed":captures.frame("variants-reversed",&reversed)});
    assert_eq!(
        a, oracle_a,
        "E*T(-pivot)*M independent compensated-original oracle A"
    );
    assert_eq!(
        b, oracle_b,
        "E*T(-pivot)*M independent compensated-original oracle B"
    );
    assert_eq!(
        together, reversed,
        "recipe namespaces remain isolated across reordered batches"
    );
    let coverage = json!({"offset_a":assert_pixels(&a,&[(0,PIVOTS[0]),(1,PIVOTS[0])]),
        "offset_b":assert_pixels(&b,&[(0,PIVOTS[1]),(1,PIVOTS[1])]),
        "variants_together":assert_pixels(&together,&[(0,PIVOTS[0]),(1,PIVOTS[1])])});
    let controls = json!({"no_pivot":differences(&a,&no_pivot),"wrong_sign":differences(&a,&wrong_sign),
        "wrong_order":differences(&a,&wrong_order),"wrong_uv":differences(&a,&wrong_uv),
        "wrong_material":differences(&a,&wrong_material),"distinct_pivots":differences(&a,&b)});
    for (label, count) in controls.as_object().unwrap() {
        assert!(
            count.as_u64().unwrap() > MIN_CONTROL_DIFFERENCES as u64,
            "negative control {label} must differ: {count}"
        );
    }
    assert_eq!(original.to_bytes().unwrap(), original_bytes);
    assert_eq!(
        variants
            .iter()
            .map(|m| m.to_bytes().unwrap())
            .collect::<Vec<_>>(),
        variant_bytes
    );
    let receipt = json!({"schema":1,"status":"passed","width":SIZE,"height":SIZE,"adapter":adapter_record,
        "assets":assets,"frames":frames,"coverage":coverage,"control_pixel_differences":controls,
        "external_instances":"two Rz(pi) rotations with nonuniform scales; node 1 mirrored, node 7 rotated",
        "oracle":"independent compensated original E*T(-pivot)*M and numeric fixture-space interior/background",
        "immutable_assets":true,"distinct_recipe_namespaces":true,"reversed_batch_order_byte_identical":true,
        "tolerances":{"color_bytes":COLOR_EPSILON,"minimum_control_differences":MIN_CONTROL_DIFFERENCES}});
    captures.write(
        "receipt.json",
        &serde_json::to_vec_pretty(&receipt).unwrap(),
    );
    eprintln!("ORR_STATIC_MODEL_PIVOT_ACCEPTANCE {receipt}");
}
