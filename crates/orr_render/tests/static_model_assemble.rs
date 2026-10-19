//! Mandatory current-CLI -> installed-package -> Vulkan production acceptance.
//! Both production renderers are checked against scalar f64 fixture-space oracles.
#![cfg(all(feature = "models", feature = "imported-scene"))]
#![allow(clippy::float_arithmetic)]
use orr_model::{Image, Material, Primitive, StaticModel, Vertex, Wrap};
use orr_package::{Manifest, Project, Runtime};
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
const SOURCE_ID: &str = "collision/source.glb";
const PACKAGE: &str = "static-assemble-gpu";
const ASSET: &str = "assembly.model.json";
const ASSET_ID: &str = "variants/scene.assemble.json";
const SIZE: u32 = 384;
const COLOR_EPSILON: u8 = 3;
const MIN_CONTROL_DIFFERENCES: usize = 80;
const COLORS: [[u8; 4]; 4] = [
    [240, 60, 40, 255],
    [50, 220, 80, 255],
    [45, 90, 230, 255],
    [220, 180, 35, 255],
];
const POINTS: [[f64; 2]; 3] = [[-0.65, -0.4], [0.65, -0.4], [-0.25, 0.4]];
type Matrix = [[f32; 4]; 4];
fn matrix(a: f32, b: f32, c: f32, d: f32, z: f32, x: f32, y: f32) -> Matrix {
    [
        [a, c, 0.0, 0.0],
        [b, d, 0.0, 0.0],
        [0.0, 0.0, z, 0.0],
        [x, y, 0.1, 1.0],
    ]
}
fn source_matrix(source: usize, node: usize) -> Matrix {
    let y = if node == 0 { -0.7 } else { 0.7 };
    if source == 0 {
        matrix(0.9, 0.2, 0.0, 0.8, 0.6, 0.12, y)
    } else {
        matrix(0.7, -0.25, 0.3, 0.9, 1.3, -0.12, y)
    }
}
fn placements() -> [Matrix; 3] {
    [
        matrix(0.0, -0.9, 0.8, 0.0, 1.2, -2.5, 0.2),
        matrix(-1.0, 0.0, 0.0, 0.8, 0.7, 0.0, -0.1),
        matrix(0.0, 1.0, -0.75, 0.0, 0.9, 2.5, 0.15),
    ]
}
fn mul(a: Matrix, b: Matrix) -> Matrix {
    std::array::from_fn(|col| {
        std::array::from_fn(|row| (0..4).map(|k| a[k][row] * b[col][k]).sum())
    })
}
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "orr-static-assemble-gpu-{}-{}",
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
            std::env::var_os("ORR_STATIC_MODEL_ASSEMBLE_CAPTURE_DIR")
                .expect("mandatory acceptance requires ORR_STATIC_MODEL_ASSEMBLE_CAPTURE_DIR"),
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
        eprintln!("ORR_STATIC_MODEL_ASSEMBLE_CAPTURE {record}");
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
fn source_model(index: usize) -> StaticModel {
    let imported = orr_model::import::import_with_resolver(SOURCE_ID, ORIGINAL, |_| {
        panic!("embedded fixture")
    })
    .unwrap();
    let mut source = imported.source().clone();
    source.images = COLORS[index * 2..index * 2 + 2]
        .iter()
        .map(|rgba| Image {
            width: 1,
            height: 1,
            rgba8: rgba.to_vec(),
        })
        .collect();
    source.materials = (0..2)
        .map(|node| Material {
            image: node,
            base_color: if node == 0 {
                [0.8, 0.6, 0.9, 0.25]
            } else {
                [0.5, 0.9, 0.7, 0.75]
            },
            linear_filter: index == 1,
            wrap_s: if index == 0 {
                Wrap::Mirror
            } else {
                Wrap::Repeat
            },
            wrap_t: if node == 0 { Wrap::Clamp } else { Wrap::Repeat },
        })
        .collect();
    source.primitives = (0..2)
        .map(|node| Primitive {
            id: format!("{SOURCE_ID}#node={node}/mesh=0/primitive=0"),
            vertices: POINTS
                .iter()
                .map(|p| Vertex {
                    position: [p[0] as f32, p[1] as f32, 0.0],
                    normal: [0.36, 0.48, 0.8],
                    uv: [0.3, 0.7],
                })
                .collect(),
            indices: vec![0, 1, 2],
            material: node as u32,
            transform: source_matrix(index, node),
        })
        .collect();
    StaticModel::new(source).unwrap()
}
fn produce(recipe: &Path, output: &Path) {
    let binary = PathBuf::from(
        std::env::var_os("ORR_STATIC_MODEL_ASSEMBLE_BIN")
            .expect("fresh current-source CLI required"),
    );
    assert!(binary.is_absolute() && binary.is_file());
    let result = Command::new(binary)
        .arg("--recipe")
        .arg(recipe)
        .arg("--asset-id")
        .arg(ASSET_ID)
        .arg("--output")
        .arg(output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "CLI: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}
fn install(scratch: &Scratch, captures: &Captures) -> (StaticModel, Value) {
    let authoring = scratch.0.join("authoring");
    let inputs = authoring.join("inputs");
    fs::create_dir_all(&inputs).unwrap();
    let sources = [source_model(0), source_model(1)];
    assert_eq!(sources[0].source().asset_id, sources[1].source().asset_id);
    assert_eq!(
        sources[0].source().primitives[0].id,
        sources[1].source().primitives[0].id
    );
    let source_records: Vec<_> = sources
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let bytes = m.to_bytes().unwrap();
            let name = format!("source-{i}.model.json");
            fs::write(inputs.join(&name), &bytes).unwrap();
            captures.write(&name, &bytes)
        })
        .collect();
    let recipe = serde_json::to_vec_pretty(
        &json!({"format":"orr_static_model_assemble","version":1,"parts":[
            {"key":"left","source":"inputs/source-0.model.json","placement":placements()[0]},
            {"key":"middle","source":"inputs/source-1.model.json","placement":placements()[1]},
            {"key":"repeat","source":"inputs/source-0.model.json","placement":placements()[2]}
        ]}),
    )
    .unwrap();
    let recipe_record = captures.write("assembly.recipe.json", &recipe);
    let recipe_path = authoring.join("assembly.recipe.json");
    fs::write(&recipe_path, &recipe).unwrap();
    let package_source = scratch.0.join("package-source");
    fs::create_dir(&package_source).unwrap();
    produce(&recipe_path, &package_source.join(ASSET));
    let bytes = fs::read(package_source.join(ASSET)).unwrap();
    let cooked_record = captures.write(ASSET, &bytes);
    let assembled = StaticModel::from_bytes(&bytes).unwrap();
    let out = assembled.source();
    assert_eq!(out.asset_id, ASSET_ID);
    assert_eq!(out.primitives.len(), 6);
    assert_eq!(out.images.len(), 4);
    assert_eq!(out.materials.len(), 4);
    assert_eq!(
        out.images,
        [
            sources[0].source().images.clone(),
            sources[1].source().images.clone()
        ]
        .concat()
    );
    for (source, model) in sources.iter().enumerate() {
        for (slot, material) in model.source().materials.iter().enumerate() {
            let mut expected = material.clone();
            expected.image += (source * 2) as u32;
            assert_eq!(out.materials[source * 2 + slot], expected);
        }
    }
    let mut ids = BTreeSet::new();
    for part in 0..3 {
        for node in 0..2 {
            let source = if part == 1 { 1 } else { 0 };
            let old = &sources[source].source().primitives[node];
            let new = &out.primitives[part * 2 + node];
            assert_eq!(new.vertices, old.vertices);
            assert_eq!(new.indices, old.indices);
            assert_eq!(new.material, (source * 2 + node) as u32);
            assert_eq!(new.transform, mul(placements()[part], old.transform));
            assert!(ids.insert(new.id.clone()));
        }
    }
    assert_eq!(
        out.dependencies
            .iter()
            .map(|d| d.uri.as_str())
            .collect::<Vec<_>>(),
        [
            "$source",
            "inputs/source-0.model.json",
            "inputs/source-1.model.json"
        ]
    );
    let manifest = Manifest {
        schema: 1,
        name: PACKAGE.into(),
        version: "1.0.0".into(),
        engine: "^0.0.1".into(),
        capabilities: ["models".into()].into(),
        dependencies: Default::default(),
        files: [ASSET.into()].into(),
    };
    fs::write(
        package_source.join("orr.package.json"),
        serde_json::to_vec(&manifest).unwrap(),
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
    assert!(!recipe_path.exists() && !inputs.exists() && !package_source.exists());
    let mut runtime = Runtime::content_only();
    runtime.capabilities.insert("models".into());
    let project = Project::open(root, runtime).unwrap();
    let lock = project.verify().unwrap();
    assert_eq!(lock.packages[PACKAGE].files.len(), 1);
    let installed = project.read_asset(PACKAGE, ASSET).unwrap();
    assert_eq!(installed, bytes);
    let reloaded = StaticModel::from_bytes(&installed).unwrap();
    assert_eq!(reloaded.to_bytes().unwrap(), bytes);
    (
        reloaded,
        json!({"sources":source_records,"recipe":recipe_record,"cooked":cooked_record,
        "source_hidden_after_install":true,"installed_files":1,"cooked_reload_before_render":true,
        "parts":3,"unique_sources":2,"primitives":6,"materials":4,"images":4,"colliding_source_identities":true,"repeated_source_tables_reused":true}),
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
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(a, b)| !matches(a, b))
        .count()
}
// Scalar oracle is defined only by fixture-space numbers, never by cooked bytes,
// authoring matrix helpers, renderer math, uploaded vertices, or GPU agreement.
// [a,b,c,d,z,tx,ty] describes x'=ax+by+tx,y'=cx+dy+ty,z'=z*z.
fn oracle_coefficients(part: usize, node: usize) -> [f64; 7] {
    let y = if node == 0 { -0.7 } else { 0.7 };
    match part {
        0 => [0.0, -0.72, 0.72, 0.16, 0.72, -2.5 - 0.9 * y, 0.296],
        1 => [-0.7, 0.25, 0.24, 0.72, 0.91, 0.12, -0.1 + 0.8 * y],
        2 => [0.0, 0.8, -0.675, -0.15, 0.54, 2.5 + y, 0.06],
        _ => unreachable!(),
    }
}
fn oracle_color(part: usize, node: usize) -> [u8; 4] {
    let [a, b, c, d, z, _, _] = oracle_coefficients(part, node);
    let det = a * d - b * c;
    let nx = (d * 0.36 - c * 0.48) / det;
    let ny = (-b * 0.36 + a * 0.48) / det;
    let nz = 0.8 / z;
    let normal_y = ny / (nx * nx + ny * ny + nz * nz).sqrt();
    let source = if part == 1 { 1 } else { 0 };
    let rgba = COLORS[source * 2 + node];
    let factors = if node == 0 {
        [0.8, 0.6, 0.9]
    } else {
        [0.5, 0.9, 0.7]
    };
    let mut color = [0, 0, 0, 255];
    for channel in 0..3 {
        let s = f64::from(rgba[channel]) / 255.0;
        let linear = if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        };
        let ground = [0.1, 0.2, 0.3][channel];
        let sky = [0.9, 0.7, 0.5][channel];
        let value =
            linear * factors[channel] * (ground + (sky - ground) * (normal_y * 0.5 + 0.5)) * 0.9;
        let encoded = if value <= 0.0031308 {
            value * 12.92
        } else {
            1.055 * value.powf(1.0 / 2.4) - 0.055
        };
        color[channel] = (encoded * 255.0).round() as u8;
    }
    color
}
fn assert_pixels(pixels: &[u8]) -> Value {
    let mut counts = [[0usize; 2]; 3];
    let mut clear = 0;
    for y in 0..SIZE as usize {
        for x in 0..SIZE as usize {
            let world = [
                (x as f64 + 0.5 - 192.0) / 48.0,
                (192.0 - y as f64 - 0.5) / 48.0,
            ];
            let mut near = false;
            let mut expected = None;
            for (part, part_counts) in counts.iter_mut().enumerate() {
                for (node, count) in part_counts.iter_mut().enumerate() {
                    let [a, b, c, d, _, tx, ty] = oracle_coefficients(part, node);
                    let det = a * d - b * c;
                    let dx = world[0] - tx;
                    let dy = world[1] - ty;
                    let local = [(d * dx - b * dy) / det, (-c * dx + a * dy) / det];
                    let w = barycentric(local, POINTS[0], POINTS[1], POINTS[2]);
                    near |= w.iter().all(|&v| v >= -0.08);
                    if w.iter().all(|&v| v > 0.08) {
                        *count += 1;
                        assert!(
                            expected.replace(oracle_color(part, node)).is_none(),
                            "no fixture overlap"
                        );
                    }
                }
            }
            let offset = (y * SIZE as usize + x) * 4;
            let actual = &pixels[offset..offset + 4];
            if let Some(color) = expected {
                assert!(
                    matches(actual, &color),
                    "scalar geometry/color/normal at {x},{y}: {actual:?} expected {color:?}"
                );
            } else if !near {
                assert!(
                    matches(actual, &[0, 0, 0, 255]),
                    "scalar background {x},{y}: {actual:?}"
                );
                clear += 1;
            }
        }
    }
    assert_eq!(
        counts,
        [[363, 347], [390, 391], [390, 363]],
        "fixed independent coverage counts"
    );
    assert_eq!(clear, 141_514);
    json!({"interior_per_part_material":counts,"background":clear,"normal_colors":(0..3).map(|p|(0..2).map(|n|oracle_color(p,n)).collect::<Vec<_>>()).collect::<Vec<_>>()})
}
fn lighting() -> Lighting {
    Lighting {
        sky: [0.9, 0.7, 0.5],
        ground: [0.1, 0.2, 0.3],
        ambient: 0.9,
        intensity: 0.0,
        shadows: false,
        tonemap: false,
        exposure: 1.0,
        ..Default::default()
    }
}
fn render(
    g: &Wgpu,
    model: StaticModel,
    target: &<Wgpu as Rhi>::Texture,
    imported: bool,
) -> Vec<u8> {
    let format = TextureFormat::Rgba8UnormSrgb;
    let view = g.create_texture_view(target, None);
    let camera = Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 4.0);
    let mut renderer = ModelRenderer::new(g.clone(), format, model).unwrap();
    renderer.clear = [0.0, 0.0, 0.0, 1.0];
    if imported {
        let mut scene = ImportedSceneRenderer::new(g.clone(), format).unwrap();
        scene.clear = [0.0, 0.0, 0.0, 1.0];
        let instances = [StaticInstance::default()];
        scene
            .draw(
                ImportedSceneTarget {
                    view: &view,
                    size: (SIZE, SIZE),
                    format,
                    sample_count: 1,
                },
                &camera,
                &lighting(),
                &PointLightSettings::default(),
                &mut [ImportedBatch::StaticInstances {
                    renderer: &mut renderer,
                    instances: &instances,
                }],
            )
            .unwrap();
    } else {
        renderer
            .draw(&view, (SIZE, SIZE), &camera, &lighting())
            .unwrap();
    }
    g.read_texture(target)
}
fn wrong_model(model: &StaticModel, control: &str) -> StaticModel {
    let mut source = model.source().clone();
    match control {
        "missing_part" => {
            source.primitives.drain(4..6);
        }
        "wrong_material_offset" => {
            for p in &mut source.primitives[2..4] {
                p.material -= 2;
            }
        }
        "wrong_image_offset" => {
            for m in &mut source.materials[2..4] {
                m.image -= 2;
            }
        }
        "missing_mirror_handling" => {
            for p in &mut source.primitives[2..4] {
                p.indices.swap(1, 2);
            }
        }
        "wrong_normals" => {
            for p in &mut source.primitives {
                for v in &mut p.vertices {
                    v.normal = [0.0, 0.0, 1.0];
                }
            }
        }
        "double_source_transform" | "reversed_order" | "ignored_placement" => {
            for (i, p) in source.primitives.iter_mut().enumerate() {
                let part = i / 2;
                let old = source_matrix(if part == 1 { 1 } else { 0 }, i % 2);
                p.transform = match control {
                    "double_source_transform" => mul(p.transform, old),
                    "reversed_order" => mul(old, placements()[part]),
                    _ => old,
                };
            }
        }
        _ => panic!("unknown control"),
    }
    StaticModel::new(source).unwrap()
}
#[test]
#[ignore = "requires fresh current-source CLI and mandatory Vulkan software GPU"]
fn static_assembly_installed_models_match_independent_oracles() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let captures = Captures::new();
    let scratch = Scratch::new();
    let (model, assets) = install(&scratch, &captures);
    let immutable = model.to_bytes().unwrap();
    let g = Wgpu::headless(WgpuOptions {
        force_software: true,
        ..Default::default()
    })
    .expect("mandatory Vulkan GPU cannot skip");
    let info = g.adapter().get_info();
    let adapter = json!({"name":info.name,"backend":format!("{:?}",info.backend),"device_type":format!("{:?}",info.device_type),"driver":info.driver,"driver_info":info.driver_info,"software":g.is_software()});
    captures.write(
        "adapter.json",
        &serde_json::to_vec_pretty(&adapter).unwrap(),
    );
    assert!(g.is_software());
    assert_eq!(adapter["backend"], "Vulkan");
    let target = g.create_texture(&TextureDesc {
        label: "assembly independent oracle",
        width: SIZE,
        height: SIZE,
        format: TextureFormat::Rgba8UnormSrgb,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let mut frames = serde_json::Map::new();
    let mut coverage = serde_json::Map::new();
    let mut controls = serde_json::Map::new();
    for (name, imported) in [("model", false), ("imported_scene", true)] {
        let actual = render(&g, model.clone(), &target, imported);
        frames.insert(name.into(), captures.frame(name, &actual));
        coverage.insert(name.into(), assert_pixels(&actual));
        for control in [
            "missing_part",
            "wrong_material_offset",
            "wrong_image_offset",
            "double_source_transform",
            "reversed_order",
            "missing_mirror_handling",
            "ignored_placement",
            "wrong_normals",
        ] {
            let wrong = render(&g, wrong_model(&model, control), &target, imported);
            let label = format!("{name}_{control}");
            frames.insert(label.clone(), captures.frame(&label, &wrong));
            let count = differences(&actual, &wrong);
            assert!(
                count > MIN_CONTROL_DIFFERENCES,
                "{label} must visibly fail: {count}"
            );
            controls.insert(label, json!(count));
        }
    }
    assert_eq!(immutable, model.to_bytes().unwrap());
    let receipt = json!({"schema":1,"status":"passed","width":SIZE,"height":SIZE,"adapter":adapter,"assets":assets,"frames":frames,"coverage":coverage,"control_pixel_differences":controls,
        "oracle":"independent scalar f64 inverse affine silhouette, sRGB material/image color, inverse-transpose normal hemisphere lighting; each production path checked independently",
        "immutable_assets":true,"tolerances":{"color_bytes":COLOR_EPSILON,"minimum_control_differences":MIN_CONTROL_DIFFERENCES,"minimum_interior_per_part_material":200,"minimum_background":135000,"barycentric_margin":0.08}});
    captures.write(
        "receipt.json",
        &serde_json::to_vec_pretty(&receipt).unwrap(),
    );
    eprintln!("ORR_STATIC_MODEL_ASSEMBLE_ACCEPTANCE {receipt}");
}
