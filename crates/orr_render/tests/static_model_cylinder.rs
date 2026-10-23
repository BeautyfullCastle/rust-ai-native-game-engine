//! Mandatory current-source CLI -> installed package -> both production renderers.
//! Independent trigonometric triangles and scalar pixel/depth/UV oracle. No skips.
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
    fs,
    path::{Path, PathBuf},
    process::Command,
};
const SIZE: u32 = 256;
const PACKAGE: &str = "static-cylinder-gpu";
const SPECS: [(f64, f64, usize); 2] = [(0.55, 2.0, 32), (1.0, 0.65, 16)];
const NAMES: [&str; 2] = ["tall", "wide"];
const TEXELS: [[u8; 4]; 4] = [
    [239, 31, 71, 255],
    [17, 211, 89, 255],
    [53, 97, 229, 255],
    [179, 137, 43, 255],
];
const EPSILON: u8 = 3;
const MIN_CONTROL: usize = 500;
fn hash(bytes: &[u8]) -> String {
    format!(
        "{:016x}",
        bytes
            .iter()
            .fold(0xcbf29ce484222325_u64, |h, b| (h ^ u64::from(*b))
                .wrapping_mul(0x100000001b3))
    )
}
fn write(root: &Path, name: &str, bytes: &[u8]) -> Value {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(name))
        .unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
    json!({"file":name,"bytes":bytes.len(),"fnv1a64":hash(bytes)})
}
fn capture(root: &Path, name: &str, bytes: &[u8]) -> Value {
    assert_eq!(bytes.len(), (SIZE * SIZE * 4) as usize);
    let raw = write(root, &format!("{name}.rgba"), bytes);
    let mut ppm = format!("P6\n{SIZE} {SIZE}\n255\n").into_bytes();
    for p in bytes.chunks_exact(4) {
        ppm.extend_from_slice(&p[..3]);
    }
    json!({"raw":raw,"image":write(root,&format!("{name}.ppm"),&ppm)})
}
#[derive(Clone, Copy)]
struct Vertex {
    p: [f64; 3],
    uv: [f64; 2],
    n: [f64; 3],
}
#[derive(Clone)]
struct Mesh {
    vertices: Vec<Vertex>,
    indices: Vec<usize>,
}
// Deliberately independent of the producer's fixed f32 quarter-circle table.
fn oracle_meshes((r, h, n): (f64, f64, usize)) -> Vec<Mesh> {
    let ring = |i: usize| {
        let a = std::f64::consts::TAU * (i % n) as f64 / n as f64;
        [a.cos(), a.sin()]
    };
    let mut sides = Mesh {
        vertices: vec![],
        indices: vec![],
    };
    for i in 0..=n {
        let [c, s] = ring(i);
        for (y, v) in [(-h / 2.0, 1.0), (h / 2.0, 0.0)] {
            sides.vertices.push(Vertex {
                p: [r * c, y, r * s],
                uv: [i as f64 / n as f64, v],
                n: [c, 0.0, s],
            });
        }
    }
    for i in 0..n {
        let b = 2 * i;
        sides.indices.extend([b, b + 1, b + 2, b + 1, b + 3, b + 2]);
    }
    let mut meshes = vec![sides];
    for sign in [1.0, -1.0] {
        let mut mesh = Mesh {
            vertices: vec![Vertex {
                p: [0.0, sign * h / 2.0, 0.0],
                uv: [0.5, 0.5],
                n: [0.0, sign, 0.0],
            }],
            indices: vec![],
        };
        for i in 0..n {
            let [c, s] = ring(i);
            mesh.vertices.push(Vertex {
                p: [r * c, sign * h / 2.0, r * s],
                uv: [0.5 + 0.5 * c, 0.5 + 0.5 * s],
                n: [0.0, sign, 0.0],
            });
        }
        for i in 0..n {
            let next = (i + 1) % n;
            if sign > 0.0 {
                mesh.indices.extend([0, next + 1, i + 1]);
            } else {
                mesh.indices.extend([0, i + 1, next + 1]);
            }
        }
        meshes.push(mesh);
    }
    meshes
}
fn topology(model: &StaticModel, spec: (f64, f64, usize), id: &str) {
    let source = model.source();
    assert_eq!(source.asset_id, id);
    assert_eq!(source.primitives.len(), 3);
    assert_eq!(source.materials.len(), 3);
    assert_eq!(source.images.len(), 1);
    assert_eq!(
        source.images[0],
        Image {
            width: 1,
            height: 1,
            rgba8: vec![255; 4]
        }
    );
    assert_eq!(source.dependencies.len(), 1);
    assert_eq!(source.dependencies[0].uri, "$source");
    for (slot, (actual, expected)) in source
        .primitives
        .iter()
        .zip(oracle_meshes(spec))
        .enumerate()
    {
        assert_eq!(actual.id, format!("{id}#node=0/mesh=0/primitive={slot}"));
        assert_eq!(actual.material, slot as u32);
        assert_eq!(actual.transform, orr_model::IDENTITY);
        assert_eq!(
            actual
                .indices
                .iter()
                .map(|v| *v as usize)
                .collect::<Vec<_>>(),
            expected.indices
        );
        assert_eq!(actual.vertices.len(), expected.vertices.len());
        for (a, b) in actual.vertices.iter().zip(expected.vertices) {
            for (got, want) in a
                .position
                .into_iter()
                .chain(a.normal)
                .chain(a.uv)
                .zip(b.p.into_iter().chain(b.n).chain(b.uv))
            {
                assert!(
                    (f64::from(got) - want).abs() < 0.000002,
                    "independent geometry/normal/UV: {got} != {want}"
                );
            }
        }
        let material = &source.materials[slot];
        assert_eq!(material.base_color, [1.0; 4]);
        assert_eq!(material.image, 0);
    }
}
fn installed(root: &Path) -> (Vec<StaticModel>, Value) {
    let authoring = root.join("authoring");
    let package = root.join("package-source");
    let project = root.join("project");
    for dir in [&authoring, &package, &project] {
        fs::create_dir(dir).unwrap();
    }
    let binary = PathBuf::from(
        std::env::var_os("ORR_STATIC_MODEL_CYLINDER_BIN")
            .expect("pinned current-source cylinder CLI required"),
    );
    assert!(binary.is_absolute() && binary.is_file());
    let mut records = vec![];
    let mut cooked = vec![];
    for (index, &(radius, height, segments)) in SPECS.iter().enumerate() {
        let name = NAMES[index];
        let id = format!("props/{name}.cylinder.json");
        let recipe=serde_json::to_vec_pretty(&json!({"format":"orr_static_model_cylinder","version":1,"radius":radius,"height":height,"segments":segments})).unwrap();
        fs::write(authoring.join(format!("{name}.json")), &recipe).unwrap();
        let out = Command::new(&binary)
            .arg("--recipe")
            .arg(authoring.join(format!("{name}.json")))
            .arg("--asset-id")
            .arg(&id)
            .arg("--output")
            .arg(authoring.join(format!("{name}.model.json")))
            .output()
            .unwrap();
        let stdout = write(root, &format!("cli-{name}.stdout.log"), &out.stdout);
        let stderr = write(root, &format!("cli-{name}.stderr.log"), &out.stderr);
        assert!(
            out.status.success(),
            "CLI {name}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let bytes = fs::read(authoring.join(format!("{name}.model.json"))).unwrap();
        fs::write(package.join(format!("{name}.orrmodel")), &bytes).unwrap();
        topology(&StaticModel::from_bytes(&bytes).unwrap(), SPECS[index], &id);
        records.push(json!({"asset_id":id,"recipe":write(root,&format!("{name}.recipe.json"),&recipe),"cooked":write(root,&format!("{name}.orrmodel"),&bytes),"stdout":stdout,"stderr":stderr}));
        cooked.push(bytes);
    }
    fs::write(package.join("orr.package.json"),serde_json::to_vec(&json!({"schema":1,"name":PACKAGE,"version":"1.0.0","engine":"*","capabilities":["models"],"dependencies":{},"files":["tall.orrmodel","wide.orrmodel"]})).unwrap()).unwrap();
    Project::open_for_install(&project, Runtime::content_only().engine_version)
        .unwrap()
        .install(std::slice::from_ref(&package))
        .unwrap();
    fs::rename(&authoring, root.join("hidden-authoring")).unwrap();
    fs::rename(&package, root.join("hidden-package-source")).unwrap();
    assert!(!authoring.exists() && !package.exists());
    let mut runtime = Runtime::content_only();
    runtime.capabilities.insert("models".into());
    let installed = Project::open(&project, runtime).unwrap();
    assert_eq!(installed.verify().unwrap().packages[PACKAGE].files.len(), 2);
    let models = NAMES
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let bytes = installed
                .read_asset(PACKAGE, &format!("{name}.orrmodel"))
                .unwrap();
            assert_eq!(bytes, cooked[i]);
            StaticModel::from_bytes(&bytes).unwrap()
        })
        .collect();
    (
        models,
        json!({"variants":records,"authoring_and_package_source_hidden":true,"installed_cooked_bytes_exact":true,"producer_fnv1a64":hash(&fs::read(binary).unwrap())}),
    )
}
fn diagnostic(model: &StaticModel) -> StaticModel {
    let mut source = model.source().clone();
    source.images[0] = Image {
        width: 2,
        height: 2,
        rgba8: TEXELS.concat(),
    };
    for m in &mut source.materials {
        m.linear_filter = false;
        m.wrap_s = Wrap::Clamp;
        m.wrap_t = Wrap::Clamp;
    }
    let result = StaticModel::new(source).unwrap();
    assert_eq!(result.source().primitives, model.source().primitives);
    result
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
fn render_lighting(
    g: &Wgpu,
    model: StaticModel,
    target: &<Wgpu as Rhi>::Texture,
    sign: f32,
    imported: bool,
    lit: bool,
) -> Vec<u8> {
    let mut lights = lighting();
    if lit {
        lights.sky = [0.8; 3];
        lights.ground = [0.2; 3];
    }
    let format = TextureFormat::Rgba8UnormSrgb;
    let camera = Camera3D::orthographic([0.0, sign * 3.0, 4.0], [0.0; 3], 2.0);
    let view = g.create_texture_view(target, None);
    let mut model = ModelRenderer::new(g.clone(), format, model).unwrap();
    model.clear = [0.0, 0.0, 0.0, 1.0];
    if imported {
        let mut renderer = ImportedSceneRenderer::new(g.clone(), format).unwrap();
        renderer.clear = [0.0, 0.0, 0.0, 1.0];
        let instances = [StaticInstance::default()];
        let mut batches = [ImportedBatch::StaticInstances {
            renderer: &mut model,
            instances: &instances,
        }];
        renderer
            .draw(
                ImportedSceneTarget {
                    view: &view,
                    size: (SIZE, SIZE),
                    format,
                    sample_count: 1,
                },
                &camera,
                &lights,
                &PointLightSettings::default(),
                &mut batches,
            )
            .unwrap();
    } else {
        model.draw(&view, (SIZE, SIZE), &camera, &lights).unwrap();
    }
    g.read_texture(target)
}
fn render(
    g: &Wgpu,
    model: StaticModel,
    target: &<Wgpu as Rhi>::Texture,
    sign: f32,
    imported: bool,
) -> Vec<u8> {
    render_lighting(g, model, target, sign, imported, false)
}
fn weights(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> [f64; 3] {
    let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    let u = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / d;
    let v = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / d;
    [u, v, 1.0 - u - v]
}
fn matches(a: &[u8], b: &[u8]) -> bool {
    a.iter().zip(b).all(|(a, b)| a.abs_diff(*b) <= EPSILON)
}
fn differences(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len());
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(a, b)| !matches(a, b))
        .count()
}
// No loaded geometry, indices, UVs, renderer camera/matrix/math helpers, or
// authoring implementation participates in expected pixels. Depth picks the
// frontmost independently projected triangle; strict interior and texel margins
// avoid raster ties, with ZERO permitted mismatches outside those margins.
type RasterHit = (f64, [f64; 3], [[f64; 2]; 3], usize);
fn oracle(pixels: &[u8], spec: (f64, f64, usize), sign: f64, textured: bool, lit: bool) -> Value {
    let meshes = oracle_meshes(spec);
    let mut counts = [0usize; 3];
    let mut clear = 0;
    for y in 0..SIZE as usize {
        for x in 0..SIZE as usize {
            let p = [
                (x as f64 + 0.5 - 128.0) / 64.0,
                (128.0 - y as f64 - 0.5) / 64.0,
            ];
            let mut nearest: Option<RasterHit> = None;
            let mut near = false;
            for (slot, mesh) in meshes.iter().enumerate() {
                for tri in mesh.indices.chunks_exact(3) {
                    let v = [
                        mesh.vertices[tri[0]],
                        mesh.vertices[tri[1]],
                        mesh.vertices[tri[2]],
                    ];
                    let q = v.map(|v| [v.p[0], 0.8 * v.p[1] - sign * 0.6 * v.p[2]]);
                    let area = (q[1][0] - q[0][0]) * (q[2][1] - q[0][1])
                        - (q[1][1] - q[0][1]) * (q[2][0] - q[0][0]);
                    if area <= 1e-10 {
                        continue;
                    }
                    let w = weights(p, q[0], q[1], q[2]);
                    near |= w.iter().all(|v| *v >= -0.04);
                    if w.iter().any(|v| *v < 0.0) {
                        continue;
                    }
                    let depth = (0..3)
                        .map(|i| w[i] * (sign * 0.6 * v[i].p[1] + 0.8 * v[i].p[2]))
                        .sum();
                    if nearest.as_ref().is_none_or(|old| depth > old.0) {
                        nearest = Some((depth, w, v.map(|v| v.uv), slot));
                    }
                }
            }
            let actual = &pixels[(y * SIZE as usize + x) * 4..(y * SIZE as usize + x + 1) * 4];
            if let Some((_, w, uv, slot)) = nearest {
                if w.iter().any(|v| *v <= 0.04) {
                    continue;
                }
                let coord: [f64; 2] =
                    std::array::from_fn(|axis| (0..3).map(|i| w[i] * uv[i][axis]).sum());
                if textured
                    && coord
                        .iter()
                        .any(|v| (2.0 * v - (2.0 * v).round()).abs() < 0.03)
                {
                    continue;
                }
                let color = if lit {
                    // sRGB encoding of linear hemispheric light .5/.8/.2 for
                    // independently known side/top/bottom unit-normal Y.
                    let channel = [188, 231, 124][slot];
                    [channel, channel, channel, 255]
                } else if textured {
                    TEXELS[(coord[1] * 2.0).floor().clamp(0.0, 1.0) as usize * 2
                        + (coord[0] * 2.0).floor().clamp(0.0, 1.0) as usize]
                } else {
                    [255; 4]
                };
                assert!(
                    matches(actual, &color),
                    "independent interior {x},{y} slot{slot}: {actual:?} != {color:?}"
                );
                counts[slot] += 1;
            } else if !near {
                assert!(
                    matches(actual, &[0, 0, 0, 255]),
                    "independent background {x},{y}: {actual:?}"
                );
                clear += 1;
            }
        }
    }
    assert!(counts[0] > 1000, "side coverage {counts:?}");
    let cap = if sign > 0.0 { 1 } else { 2 };
    assert!(counts[cap] > 500, "cap coverage {counts:?}");
    assert!(clear > 35_000);
    json!({"interior_per_surface":counts,"background":clear,"mismatches":0})
}
#[test]
#[ignore = "requires current-source CLI, software Vulkan and fresh absolute capture directory"]
fn static_cylinder_variants_match_independent_gpu_oracle() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let root = PathBuf::from(
        std::env::var_os("ORR_STATIC_MODEL_CYLINDER_CAPTURE_DIR")
            .expect("fresh evidence directory"),
    );
    assert!(root.is_absolute());
    fs::create_dir(&root).unwrap();
    let (models, assets) = installed(&root);
    let g = Wgpu::headless(WgpuOptions {
        force_software: true,
        ..Default::default()
    })
    .expect("mandatory software Vulkan device");
    let info = g.adapter().get_info();
    assert!(g.is_software());
    assert_eq!(format!("{:?}", info.backend), "Vulkan");
    let adapter = json!({"name":info.name,"backend":format!("{:?}",info.backend),"device_type":format!("{:?}",info.device_type),"software":g.is_software()});
    write(
        &root,
        "adapter.json",
        &serde_json::to_vec_pretty(&adapter).unwrap(),
    );
    let target = g.create_texture(&TextureDesc {
        label: "cylinder acceptance",
        width: SIZE,
        height: SIZE,
        format: TextureFormat::Rgba8UnormSrgb,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let original = orr_model::import::import_with_resolver(
        "original.glb",
        include_bytes!("../../orr_model/tests/fixtures/model.glb"),
        |_| panic!("embedded"),
    )
    .unwrap();
    let mut frames = serde_json::Map::new();
    let mut coverage = serde_json::Map::new();
    let mut controls = serde_json::Map::new();
    let mut whites = vec![];
    for (index, model) in models.iter().enumerate() {
        for sign in [1.0, -1.0] {
            let side = if sign > 0.0 { "top" } else { "bottom" };
            let prefix = format!("{}-{side}", NAMES[index]);
            let white = render(&g, model.clone(), &target, sign, false);
            let checker = diagnostic(model);
            let textured = render(&g, checker.clone(), &target, sign, false);
            let imported = render(&g, checker.clone(), &target, sign, true);
            assert_eq!(
                imported, textured,
                "dual renderers must agree byte-for-byte"
            );
            let imported_white = render(&g, model.clone(), &target, sign, true);
            assert_eq!(imported_white, white);
            coverage.insert(
                format!("{prefix}-white"),
                oracle(&white, SPECS[index], f64::from(sign), false, false),
            );
            coverage.insert(
                format!("{prefix}-checker"),
                oracle(&textured, SPECS[index], f64::from(sign), true, false),
            );
            for (kind, pixels) in [
                ("white", &white),
                ("checker", &textured),
                ("imported", &imported),
                ("imported-white", &imported_white),
            ] {
                frames.insert(
                    format!("{prefix}-{kind}"),
                    capture(&root, &format!("{prefix}-{kind}"), pixels),
                );
            }
            let original_pixels = render(&g, original.clone(), &target, sign, false);
            let count = differences(&white, &original_pixels);
            assert!(count > MIN_CONTROL);
            controls.insert(format!("{prefix}-original"), json!(count));
            frames.insert(
                format!("{prefix}-original"),
                capture(&root, &format!("{prefix}-original"), &original_pixels),
            );
            for fault in ["wrong-geometry", "wrong-uv", "wrong-winding"] {
                let mut bad = checker.source().clone();
                for primitive in &mut bad.primitives {
                    match fault {
                        "wrong-geometry" => {
                            for v in &mut primitive.vertices {
                                v.position[0] *= 0.5;
                                v.position[2] *= 0.5;
                            }
                        }
                        "wrong-uv" => {
                            for v in &mut primitive.vertices {
                                v.uv[0] = 1.0 - v.uv[0];
                            }
                        }
                        "wrong-winding" => {
                            for tri in primitive.indices.chunks_exact_mut(3) {
                                tri.swap(1, 2);
                            }
                        }
                        _ => unreachable!(),
                    }
                }
                let bad = StaticModel::new(bad).unwrap();
                let pixels = render(&g, bad.clone(), &target, sign, false);
                assert_eq!(pixels, render(&g, bad, &target, sign, true));
                let count = differences(&textured, &pixels);
                assert!(count > MIN_CONTROL, "{prefix} {fault}: {count}");
                controls.insert(format!("{prefix}-{fault}"), json!(count));
                frames.insert(
                    format!("{prefix}-{fault}"),
                    capture(&root, &format!("{prefix}-{fault}"), &pixels),
                );
            }
            let lit = render_lighting(&g, model.clone(), &target, sign, false, true);
            let imported_lit = render_lighting(&g, model.clone(), &target, sign, true, true);
            assert_eq!(lit, imported_lit);
            coverage.insert(
                format!("{prefix}-lit"),
                oracle(&lit, SPECS[index], f64::from(sign), false, true),
            );
            for (name, pixels) in [("lit", &lit), ("imported-lit", &imported_lit)] {
                frames.insert(
                    format!("{prefix}-{name}"),
                    capture(&root, &format!("{prefix}-{name}"), pixels),
                );
            }
            let mut wrong = model.source().clone();
            for primitive in &mut wrong.primitives {
                for vertex in &mut primitive.vertices {
                    vertex.normal = [0.0, 1.0, 0.0];
                }
            }
            let wrong = StaticModel::new(wrong).unwrap();
            let pixels = render_lighting(&g, wrong.clone(), &target, sign, false, true);
            assert_eq!(
                pixels,
                render_lighting(&g, wrong, &target, sign, true, true)
            );
            let count = differences(&lit, &pixels);
            assert!(count > MIN_CONTROL);
            controls.insert(format!("{prefix}-wrong-normal"), json!(count));
            frames.insert(
                format!("{prefix}-wrong-normal"),
                capture(&root, &format!("{prefix}-wrong-normal"), &pixels),
            );
            let mut missing = checker.source().clone();
            missing.primitives.remove(if sign > 0.0 { 1 } else { 2 });
            let missing = StaticModel::new(missing).unwrap();
            let pixels = render(&g, missing.clone(), &target, sign, false);
            assert_eq!(pixels, render(&g, missing, &target, sign, true));
            let count = differences(&textured, &pixels);
            assert!(count > MIN_CONTROL);
            controls.insert(format!("{prefix}-missing-cap"), json!(count));
            frames.insert(
                format!("{prefix}-missing-cap"),
                capture(&root, &format!("{prefix}-missing-cap"), &pixels),
            );
            if sign > 0.0 {
                whites.push(white);
            }
        }
    }
    let distinct = differences(&whites[0], &whites[1]);
    assert!(distinct > MIN_CONTROL);
    controls.insert("distinct-proportions".into(), json!(distinct));
    assert_eq!(frames.len(), 48);
    assert_eq!(coverage.len(), 12);
    assert_eq!(controls.len(), 25);
    write(&root,"receipt.json",&serde_json::to_vec_pretty(&json!({"schema":1,"status":"passed","test":"static_cylinder_variants_match_independent_gpu_oracle","mandatory_test_count":1,"width":SIZE,"height":SIZE,"frame_count":48,"adapter":adapter,"assets":assets,"frames":frames,"coverage":coverage,"control_pixel_differences":controls,"minimum_control_differences":MIN_CONTROL,"color_epsilon":EPSILON,"independent_triangle_uv_pixel_oracle":true,"independent_hemisphere_normal_oracle":true,"dual_renderers_byte_identical":true,"diagnostic_preserves_installed_geometry_uv_indices":true,"test_source_fnv1a64":hash(include_bytes!("static_model_cylinder.rs"))})).unwrap());
}
