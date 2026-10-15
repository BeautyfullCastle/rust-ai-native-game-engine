//! Mandatory installed-cooked-subtree GPU acceptance with independent geometry,
//! UV/material, mirrored-world-transform, descendant, and omitted-content oracles.
//! Run only with --ignored --exact, ORR_REQUIRE_GPU=1, and a fresh absolute
//! ORR_STATIC_MODEL_EXTRACT_CAPTURE_DIR. Adapter absence is a failure, never a skip.
#![cfg(all(feature = "models", feature = "imported-scene"))]
#![allow(clippy::float_arithmetic)]

#[allow(dead_code)]
#[path = "../../orr_model/examples/static_model_extract/authoring.rs"]
mod authoring;

use orr_model::StaticModel;
use orr_package::{Project, Runtime};
use orr_render::orr_rhi::{Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions};
use orr_render::{Camera3D, Lighting, ModelRenderer};
use serde_json::{json, Value};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

const ORIGINAL: &[u8] = include_bytes!("../../orr_model/tests/fixtures/model.glb");
const LOGICAL_SOURCE: &str = "scene/source.glb";
const PACKAGE: &str = "static-extract-gpu";
const ASSET: &str = "selected.model.json";
const SIZE: u32 = 256;
// Fixed before device execution: <=3 display bytes accounts for sRGB quantization.
// Interior/background pixels have no mismatch allowance. 8% barycentric margins
// exclude raster edges; a 2% texel margin excludes nearest-sampling ties.
const COLOR_EPSILON: u8 = 3;
const MIN_CONTROL_DIFFERENCES: usize = 500;
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
            "orr-static-extract-gpu-{}-{}",
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
        bytes.iter().fold(0xcbf29ce484222325_u64, |hash, byte| (hash
            ^ u64::from(*byte))
        .wrapping_mul(0x100000001b3))
    )
}
struct Captures(PathBuf);
impl Captures {
    fn new() -> Self {
        let path = PathBuf::from(
            std::env::var_os("ORR_STATIC_MODEL_EXTRACT_CAPTURE_DIR")
                .expect("mandatory acceptance requires ORR_STATIC_MODEL_EXTRACT_CAPTURE_DIR"),
        );
        assert!(path.is_absolute(), "capture directory must be absolute");
        fs::create_dir_all(&path).unwrap();
        Self(path)
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
        eprintln!("ORR_STATIC_MODEL_EXTRACT_CAPTURE {record}");
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

// Test-authored scene topology, independent of the extractor. Original binary
// geometry/images remain byte-identical. Node 1 selects itself and child 4;
// sibling 3 and separate root 2 must be omitted. Node 5 is empty, 6 out of scene.
fn fixture() -> Vec<u8> {
    let json_len = u32::from_le_bytes(ORIGINAL[12..16].try_into().unwrap()) as usize;
    let mut source: Value = serde_json::from_slice(&ORIGINAL[20..20 + json_len]).unwrap();
    let binary = &ORIGINAL[28 + json_len..];
    source["scenes"] = json!([{"nodes":[0,2,5]}]);
    source["nodes"] = json!([
        {"name":"ancestor","translation":[-2,1,0],"scale":[0.5,0.5,1],"children":[1,3]},
        {"name":"selected mirrored prop","mesh":0,"translation":[0.5,0,0.1],"scale":[-1,1,1],"children":[4]},
        {"name":"omitted separate root","mesh":0,"translation":[2,-1,0.1],"scale":[0.5,0.5,1]},
        {"name":"omitted sibling","mesh":0,"translation":[8,0,0.1]},
        {"name":"selected descendant","mesh":0,"translation":[0,-4,0]},
        {"name":"empty root"},
        {"name":"outside default scene","mesh":0}
    ]);
    // Unused image/material entries must survive, with all original slots intact.
    source["images"]
        .as_array_mut()
        .unwrap()
        .push(json!({"bufferView":3,"mimeType":"image/png"}));
    source["textures"]
        .as_array_mut()
        .unwrap()
        .push(json!({"source":2,"sampler":0}));
    source["materials"].as_array_mut().unwrap().push(json!({"pbrMetallicRoughness":{
        "baseColorTexture":{"index":2},"baseColorFactor":[0.25,0.5,0.75,1],"metallicFactor":0,"roughnessFactor":1
    }}));
    let mut document = serde_json::to_vec(&source).unwrap();
    while document.len() % 4 != 0 {
        document.push(b' ');
    }
    let mut output = Vec::new();
    for word in [
        0x46546c67_u32,
        2,
        (28 + document.len() + binary.len()) as u32,
        document.len() as u32,
        0x4e4f534a,
    ] {
        output.extend(word.to_le_bytes());
    }
    output.extend(document);
    output.extend((binary.len() as u32).to_le_bytes());
    output.extend(0x004e4942_u32.to_le_bytes());
    output.extend(binary);
    output
}

fn install(scratch: &Scratch, captures: &Captures) -> (StaticModel, StaticModel, Value) {
    let input = fixture();
    let source_record = captures.write("source.glb", &input);
    let original = orr_model::import::import_with_resolver(LOGICAL_SOURCE, &input, |_| {
        panic!("embedded source must not resolve external content")
    })
    .unwrap();
    let selected = authoring::extract_glb_node(&input, LOGICAL_SOURCE, 1).unwrap();
    let before = original.source();
    let after = selected.source();
    assert_eq!(before.primitives.len(), 8);
    assert_eq!(after.primitives.len(), 4);
    assert_eq!(after.asset_id, LOGICAL_SOURCE);
    assert_eq!(after.dependencies, before.dependencies);
    assert_eq!(after.materials.len(), 3);
    assert_eq!(after.images.len(), 3);
    assert_eq!(after.materials, before.materials);
    assert_eq!(after.images, before.images);
    for (index, primitive) in after.primitives.iter().enumerate() {
        let node = if index < 2 { 1 } else { 4 };
        assert_eq!(
            primitive.id,
            format!(
                "{LOGICAL_SOURCE}#node={node}/mesh=0/primitive={}",
                index % 2
            )
        );
        assert_eq!(
            primitive.transform,
            [
                [-0.5, 0.0, 0.0, 0.0],
                [0.0, 0.5, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [-1.75, if node == 1 { 1.0 } else { -1.0 }, 0.1, 1.0]
            ]
        );
        assert_eq!(
            primitive, &before.primitives[index],
            "exact source primitive preservation"
        );
    }
    let cooked = selected.to_bytes().unwrap();
    let cooked_record = captures.write(ASSET, &cooked);
    let source_path = scratch.0.join("input.glb");
    fs::write(&source_path, &input).unwrap();
    let package_source = scratch.0.join("authored-package");
    fs::create_dir(&package_source).unwrap();
    fs::write(package_source.join(ASSET), &cooked).unwrap();
    fs::write(
        package_source.join("orr.package.json"),
        serde_json::to_vec(&json!({
            "schema":1,"name":PACKAGE,"version":"1.0.0","engine":"^0.0.1",
            "capabilities":["models"],"dependencies":{},"files":[ASSET]
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
    fs::rename(&source_path, scratch.0.join("hidden-input.glb")).unwrap();
    fs::rename(&package_source, scratch.0.join("hidden-package-source")).unwrap();
    assert!(!source_path.exists());
    assert!(!package_source.exists());
    assert!(!root.join(LOGICAL_SOURCE).exists());
    let mut runtime = Runtime::content_only();
    runtime.capabilities.insert("models".into());
    let project = Project::open(root, runtime).unwrap();
    let lock = project.verify().unwrap();
    assert_eq!(lock.packages[PACKAGE].files.len(), 1);
    let installed_bytes = project.read_asset(PACKAGE, ASSET).unwrap();
    assert_eq!(installed_bytes, cooked);
    let reloaded = StaticModel::from_bytes(&installed_bytes).unwrap();
    assert_eq!(reloaded.source(), selected.source());
    assert_eq!(reloaded.to_bytes().unwrap(), cooked);
    assert_eq!(fs::read(scratch.0.join("hidden-input.glb")).unwrap(), input);
    (
        original,
        reloaded,
        json!({"source":source_record,"cooked":cooked_record,
        "source_hidden_after_install":true,"installed_files":1,"cooked_reload_before_render":true}),
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

// This oracle never reads extractor output, imported vertices, matrices, UVs,
// materials, images, camera projection helpers, or renderer math. It uses the
// fixture's authored topology and original mesh/texture specification directly.
fn assert_pixels(pixels: &[u8], full_scene: bool) -> Value {
    assert_eq!(pixels.len(), (SIZE * SIZE * 4) as usize);
    let mut panels = vec![(-1.75, 1.0, -1.0), (-1.75, -1.0, -1.0)];
    if full_scene {
        panels.extend([(2.0, 1.0, 1.0), (2.0, -1.0, 1.0)]);
    }
    let mut color_counts = [0usize; 5];
    let mut clear = 0;
    let mut omitted_clear = 0;
    let foreground = pixels
        .chunks_exact(4)
        .filter(|p| p[..3] != [0, 0, 0])
        .count();
    for y in 0..SIZE as usize {
        for x in 0..SIZE as usize {
            // Fixed camera spans [-4,+4], 32 pixels per world unit, pixel centers.
            let world = [
                (x as f64 + 0.5 - 128.0) / 32.0,
                (128.0 - y as f64 - 0.5) / 32.0,
            ];
            let mut near_triangle = false;
            let mut expected = None;
            for &(center_x, center_y, sign) in &panels {
                let local = [
                    (world[0] - center_x) / (0.5 * sign),
                    (world[1] - center_y) / 0.5,
                ];
                for (index, ids) in TRIANGLES.iter().enumerate() {
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
                    if index == 3 {
                        expected = Some((4, MARKER));
                    } else {
                        // Panel UVs are u=(x+1)/2 and v=(1-y)/2 everywhere,
                        // including the asymmetric clipped top-right corner.
                        let texel = [local[0] + 1.0, 1.0 - local[1]];
                        if texel.iter().any(|v| v.fract() < 0.02 || v.fract() > 0.98) {
                            continue;
                        }
                        let color = texel[1].floor() as usize * 2 + texel[0].floor() as usize;
                        assert!(color < CHECKER.len());
                        expected = Some((color, CHECKER[color]));
                    }
                }
            }
            let offset = (y * SIZE as usize + x) * 4;
            let actual = &pixels[offset..offset + 4];
            if let Some((slot, color)) = expected {
                assert!(
                    matches(actual, &color),
                    "full_scene={full_scene} independent interior {x},{y}: {actual:?} != {color:?}"
                );
                color_counts[slot] += 1;
            } else if !near_triangle {
                assert!(
                    matches(actual, &[0, 0, 0, 255]),
                    "full_scene={full_scene} independent background {x},{y}: {actual:?}"
                );
                clear += 1;
                if !full_scene
                    && world[0] > 1.5
                    && world[0] < 2.5
                    && world[1].abs() > 0.5
                    && world[1].abs() < 1.5
                {
                    omitted_clear += 1;
                }
            }
        }
    }
    for (index, count) in color_counts.iter().enumerate() {
        assert!(
            *count > 40,
            "nonvacuous independent color/material slot {index}: {count}"
        );
    }
    assert!(color_counts.iter().sum::<usize>() > 600);
    assert!(clear > 50_000);
    assert!(
        foreground > if full_scene { 3000 } else { 1500 },
        "nonblank required: {foreground}"
    );
    if !full_scene {
        assert!(
            omitted_clear > 1500,
            "omitted root/sibling coverage: {omitted_clear}"
        );
    }
    json!({"color_pixels":color_counts,"clear_checked":clear,"omitted_clear_checked":omitted_clear,"foreground":foreground})
}

fn render(g: &Wgpu, model: StaticModel, target: &<Wgpu as Rhi>::Texture) -> Vec<u8> {
    let mut renderer = ModelRenderer::new(g.clone(), TextureFormat::Rgba8UnormSrgb, model).unwrap();
    renderer.clear = [0.0, 0.0, 0.0, 1.0];
    let camera = Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 4.0);
    let light = Lighting {
        sky: [1.0; 3],
        ground: [1.0; 3],
        ambient: 1.0,
        intensity: 0.0,
        shadows: false,
        tonemap: false,
        exposure: 1.0,
        ..Default::default()
    };
    let view = g.create_texture_view(target, None);
    renderer.draw(&view, (SIZE, SIZE), &camera, &light).unwrap();
    g.read_texture(target)
}

#[test]
#[ignore = "requires mandatory software GPU and fresh capture directory"]
fn extracted_static_subtree_matches_independent_gpu_oracle() {
    assert_eq!(
        std::env::var("ORR_REQUIRE_GPU").as_deref(),
        Ok("1"),
        "GPU acceptance must explicitly require its device"
    );
    let captures = Captures::new();
    let scratch = Scratch::new();
    let (original, selected, asset_record) = install(&scratch, &captures);
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
    eprintln!("ORR_STATIC_MODEL_EXTRACT_ADAPTER {adapter_record}");
    assert!(
        g.is_software(),
        "dedicated acceptance requires an actual software adapter"
    );
    assert_eq!(adapter_record["backend"], "Vulkan");
    let target = g.create_texture(&TextureDesc {
        label: "mandatory extracted static subtree",
        width: SIZE,
        height: SIZE,
        format: TextureFormat::Rgba8UnormSrgb,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let full = render(&g, original, &target);
    let extracted = render(&g, selected.clone(), &target);
    let mut wrong_transform = selected.source().clone();
    for primitive in &mut wrong_transform.primitives {
        primitive.transform = orr_model::IDENTITY;
    }
    let wrong_transform = render(&g, StaticModel::new(wrong_transform).unwrap(), &target);
    let mut wrong_uv = selected.source().clone();
    for primitive in &mut wrong_uv.primitives {
        for vertex in &mut primitive.vertices {
            vertex.uv[0] = 1.0 - vertex.uv[0];
        }
    }
    let wrong_uv = render(&g, StaticModel::new(wrong_uv).unwrap(), &target);
    let mut wrong_material = selected.source().clone();
    for primitive in &mut wrong_material.primitives {
        primitive.material = 1 - primitive.material;
    }
    let wrong_material = render(&g, StaticModel::new(wrong_material).unwrap(), &target);
    // Keep all actual readbacks before visual assertions, including failures.
    let frames = json!({
        "full_scene":captures.frame("full-scene", &full),
        "extracted_subtree":captures.frame("extracted-subtree", &extracted),
        "wrong_transform":captures.frame("wrong-transform", &wrong_transform),
        "wrong_uv":captures.frame("wrong-uv", &wrong_uv),
        "wrong_material":captures.frame("wrong-material", &wrong_material)
    });
    let full_coverage = assert_pixels(&full, true);
    let extracted_coverage = assert_pixels(&extracted, false);
    let controls = json!({"omitted_geometry":differences(&full,&extracted),
        "world_transform":differences(&wrong_transform,&extracted),
        "asymmetric_uv":differences(&wrong_uv,&extracted),
        "material_slot":differences(&wrong_material,&extracted)});
    for (label, count) in controls.as_object().unwrap() {
        assert!(
            count.as_u64().unwrap() > MIN_CONTROL_DIFFERENCES as u64,
            "negative control {label} must visibly differ: {count}"
        );
    }
    // Selected content is unchanged in full-scene rendering on this same device;
    // the entire left half is therefore byte-identical, including mirrored edges.
    for row in 0..SIZE as usize {
        let start = row * SIZE as usize * 4;
        let end = start + (SIZE as usize / 2) * 4;
        assert_eq!(&full[start..end], &extracted[start..end]);
    }
    let receipt = json!({"schema":1,"status":"passed","width":SIZE,"height":SIZE,
        "selected_node":1,"selected_primitives":4,"source_primitives":8,
        "adapter":adapter_record,"assets":asset_record,"frames":frames,
        "full_coverage":full_coverage,"extracted_coverage":extracted_coverage,
        "control_pixel_differences":controls,"selected_left_half_byte_identical":true,
        "tolerances":{"color_bytes":COLOR_EPSILON,"minimum_control_differences":MIN_CONTROL_DIFFERENCES}});
    captures.write(
        "receipt.json",
        &serde_json::to_vec_pretty(&receipt).unwrap(),
    );
    eprintln!("ORR_STATIC_MODEL_EXTRACT_ACCEPTANCE {receipt}");
}
