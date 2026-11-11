//! Mandatory CLI file IO -> installed package -> cook/reload -> GPU acceptance,
//! including half/double-speed extraction against independently mapped source time.
//!
//! Run this target with BOTH animation,imported-scene and an explicit, fresh
//! ORR_CLIP_EXTRACT_CAPTURE_DIR. Missing adapters or captures are failures, never
//! skips. It is explicitly ignored in aggregate suites; dedicated acceptance must
//! select it with --ignored --exact and require one passed / zero ignored tests.
//! The test shares the production CLI entry point, not its extraction math.
#![cfg(all(feature = "animation", feature = "imported-scene"))]
#![allow(clippy::float_arithmetic)]

#[path = "../../orr_model/examples/animation_clip_extract/authoring.rs"]
mod authoring;

use orr_model::animation::{AnimatedModel, Pose};
use orr_package::{Project, Runtime};
use orr_render::orr_rhi::{Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions};
use orr_render::{Camera3D, Lighting, SkinnedInstance, SkinnedModelRenderer};
use serde_json::{json, Value};
use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

const ORIGINAL: &[u8] = include_bytes!("../../orr_model/tests/fixtures/animated_strip.glb");
const SIZE: u32 = 192;
const START: f32 = 0.25;
const END: f32 = 1.5;
const LOCAL_TIMES: [f32; 5] = [0.0, 0.25, 0.75, 1.0, 1.25];
const CLIP_NAME: &str = "bend-extracted-quarter-to-three-halves";

// Fixed BEFORE device execution, not adjusted to fit observed captures:
// f32 quaternion/skin math is compared with a separate f64 fixture oracle to
// 3e-5 world units (<0.001 pixel at our 32 px/unit orthographic scale).
// Three display-byte levels cover sRGB conversion/quantization. At most 32 of
// 36,864 pixels may cross a raster/nearest-texel boundary between equivalent
// draws (<0.087%). Independently checked confident interior/background pixels
// have NO such mismatch allowance. Wrong-time and moving-frame controls must
// change >100 pixels, well beyond the equivalent-frame boundary allowance.
const POSITION_EPSILON: f64 = 3e-5;
const COLOR_EPSILON: u8 = 3;
const MAX_BOUNDARY_PIXELS: usize = 32;
const MIN_CHANGED_PIXELS: usize = 100;
const TRIANGLES: [[usize; 3]; 6] = [
    [0, 1, 2],
    [1, 3, 2],
    [2, 3, 4],
    [3, 5, 4],
    [4, 5, 6],
    [5, 7, 6],
];
const POSITIONS: [[f64; 3]; 8] = [
    [-0.28, 0.0, 0.0],
    [0.42, 0.0, 0.0],
    [-0.22, 1.0, 0.0],
    [0.34, 1.0, 0.0],
    [-0.35, 2.0, 0.0],
    [0.27, 2.0, 0.0],
    [-0.18, 3.0, 0.0],
    [0.46, 3.0, 0.0],
];
const CHECKER: [[usize; 4]; 4] = [[0, 0, 1, 1], [0, 3, 1, 2], [2, 2, 3, 3], [2, 1, 3, 0]];
const COLORS: [[u8; 4]; 4] = [
    [245, 113, 40, 255],
    [35, 190, 213, 255],
    [133, 85, 219, 255],
    [250, 231, 166, 255],
];

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "orr-clip-extract-gpu-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("fresh owned fixture directory");
        // Resolve only our newly created directory, so OS temp aliases such as
        // macOS /var do not reach the authoring tool's link-free path checks.
        Self(fs::canonicalize(path).unwrap())
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fnv1a64(bytes: &[u8]) -> String {
    let hash = bytes.iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    });
    format!("{hash:016x}")
}

struct Captures(PathBuf);
impl Captures {
    fn new() -> Self {
        let path = PathBuf::from(
            std::env::var_os("ORR_CLIP_EXTRACT_CAPTURE_DIR")
                .expect("mandatory GPU acceptance needs an explicit ORR_CLIP_EXTRACT_CAPTURE_DIR"),
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
            .expect("use a fresh capture directory for each invocation");
        file.write_all(bytes).unwrap();
        file.sync_all().unwrap();
        let record = json!({"file":name,"bytes":bytes.len(),"fnv1a64":fnv1a64(bytes)});
        eprintln!("ORR_CLIP_EXTRACT_CAPTURE {record}");
        record
    }
    fn frame(&self, name: &str, pixels: &[u8]) -> Value {
        assert_eq!(pixels.len(), SIZE as usize * SIZE as usize * 4);
        let raw = self.write(&format!("{name}.rgba"), pixels);
        let mut ppm = format!("P6\n{SIZE} {SIZE}\n255\n").into_bytes();
        for pixel in pixels.chunks_exact(4) {
            ppm.extend_from_slice(&pixel[..3]);
        }
        let image = self.write(&format!("{name}.ppm"), &ppm);
        json!({"raw":raw,"image":image})
    }
}

// Only inspect preservation here. Boundary sampling/rebasing is exclusively
// performed by the imported production authoring module above.
fn chunks(bytes: &[u8]) -> (Value, &[u8]) {
    assert_eq!(&bytes[..4], b"glTF");
    let word = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
    assert_eq!(word(4), 2);
    assert_eq!(word(8), bytes.len());
    assert_eq!(&bytes[16..20], b"JSON");
    let end = 20 + word(12);
    assert_eq!(&bytes[end + 4..end + 8], b"BIN\0");
    assert_eq!(end + 8 + word(end), bytes.len());
    (
        serde_json::from_slice(&bytes[20..end]).unwrap(),
        &bytes[end + 8..],
    )
}

fn assert_preserved(original: &[u8], extracted: &[u8]) {
    let (before, binary) = chunks(original);
    let (mut after, appended) = chunks(extracted);
    assert!(
        appended.starts_with(binary),
        "original BIN including padding must remain byte-identical"
    );
    for table in ["accessors", "bufferViews", "animations"] {
        let old = before[table].as_array().unwrap();
        let new = after[table].as_array().unwrap();
        assert!(new.len() > old.len(), "{table} must append entries");
        assert_eq!(&new[..old.len()], old, "original {table} entries changed");
        if table == "animations" {
            assert_eq!(new.len(), old.len() + 1);
            assert_eq!(new.last().unwrap()["name"], CLIP_NAME);
        }
        after[table] = before[table].clone();
    }
    assert!(
        after["buffers"][0]["byteLength"].as_u64().unwrap()
            > before["buffers"][0]["byteLength"].as_u64().unwrap()
    );
    after["buffers"][0]["byteLength"] = before["buffers"][0]["byteLength"].clone();
    assert_eq!(after, before, "all other GLB content must remain unchanged");
}

fn installed(scratch: &Scratch) -> (Project, Vec<u8>) {
    installed_with_speed(scratch, "trimmed.glb", None)
}

fn installed_with_speed(scratch: &Scratch, asset: &str, speed: Option<&str>) -> (Project, Vec<u8>) {
    let source = scratch.0.join("source");
    fs::create_dir(&source).unwrap();
    let input = source.join("original.glb");
    let output = source.join(asset);
    fs::write(&input, ORIGINAL).unwrap();
    let mut arguments = vec![
        OsString::from("--input"),
        input.into_os_string(),
        OsString::from("--output"),
        output.clone().into_os_string(),
        OsString::from("--clip"),
        OsString::from("0"),
        OsString::from("--start"),
        OsString::from("0.25"),
        OsString::from("--end"),
        OsString::from("1.5"),
        OsString::from("--name"),
        OsString::from(CLIP_NAME),
    ];
    if let Some(speed) = speed {
        arguments.extend([OsString::from("--speed"), OsString::from(speed)]);
    }
    authoring::run(arguments).expect("the same run() used by the CLI must publish a new GLB");
    assert_eq!(fs::read(source.join("original.glb")).unwrap(), ORIGINAL);
    let authored = fs::read(output).unwrap();
    assert_preserved(ORIGINAL, &authored);
    fs::write(
        source.join("orr.package.json"),
        serde_json::to_vec(&json!({
            "schema":1,"name":"clip-extract-acceptance","version":"1.0.0",
            "engine":"^0.0.1","capabilities":["animation"],"dependencies":{},
            "files":["original.glb",asset]
        }))
        .unwrap(),
    )
    .unwrap();
    let root = scratch.0.join("project");
    fs::create_dir(&root).unwrap();
    Project::open_for_install(&root, Runtime::content_only().engine_version)
        .unwrap()
        .install(std::slice::from_ref(&source))
        .unwrap();
    fs::rename(&source, scratch.0.join("source-hidden-after-install")).unwrap();
    assert!(!source.exists());
    let mut runtime = Runtime::content_only();
    runtime.capabilities.insert("animation".into());
    let project = Project::open(root, runtime).unwrap();
    project.verify().unwrap();
    assert_eq!(
        project
            .read_asset("clip-extract-acceptance", "original.glb")
            .unwrap(),
        ORIGINAL
    );
    assert_eq!(
        project
            .read_asset("clip-extract-acceptance", asset)
            .unwrap(),
        authored
    );
    (project, authored)
}

fn cook_reload(project: &Project, asset: &str, captures: &Captures) -> (AnimatedModel, Value) {
    let bytes = project
        .read_asset("clip-extract-acceptance", asset)
        .unwrap();
    // Identical logical asset ID makes primitive/content comparison meaningful;
    // dependency hashes still retain the actual, distinct source-byte provenance.
    let imported =
        orr_model::animation_import::import_with_resolver("animated.glb", &bytes, |_| {
            Err(orr_model::Error::message(
                "embedded GLB must not resolve any external resource",
            ))
        })
        .unwrap();
    let cooked = imported.to_bytes().unwrap();
    let file = format!("{asset}.cooked.json");
    let record = captures.write(&file, &cooked);
    let reloaded = AnimatedModel::from_bytes(&fs::read(captures.0.join(file)).unwrap()).unwrap();
    assert_eq!(
        imported.source(),
        reloaded.source(),
        "standalone cooked reload must preserve source"
    );
    (reloaded, record)
}

fn camera() -> Camera3D {
    Camera3D::orthographic([0.0, 1.5, 5.0], [0.0, 1.5, 0.0], 3.0)
}
fn light() -> Lighting {
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

// Original CC0 animated_strip_generate.py defines a 0 -> 60 -> 0 degree
// rotation at source times 0,1,2, around bind pivot (.3,1.7), with row weights
// 0,.2,.7,1. This f64 oracle uses no imported channels, runtime interpolation,
// pose hierarchy, palette, deformation helper, or authoring implementation.
fn analytic_positions(source_time: f32) -> Vec<[f64; 3]> {
    let t = f64::from(source_time);
    let angle = std::f64::consts::PI / 3.0 * if t <= 1.0 { t } else { 2.0 - t };
    let (s, c) = angle.sin_cos();
    POSITIONS
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let weight = [0.0, 0.2, 0.7, 1.0][i / 2];
            let rotated = [
                0.3 + c * (p[0] - 0.3) - s * (p[1] - 1.7),
                1.7 + s * (p[0] - 0.3) + c * (p[1] - 1.7),
                p[2],
            ];
            std::array::from_fn(|axis| p[axis] * (1.0 - weight) + rotated[axis] * weight)
        })
        .collect()
}

fn assert_pose(model: &AnimatedModel, pose: &Pose, source_time: f32) {
    let expected = analytic_positions(source_time);
    let deformed = model.deform(pose).unwrap();
    assert_eq!(deformed.len(), 1);
    assert_eq!(deformed[0].vertices.len(), expected.len());
    for (vertex, expected) in deformed[0].vertices.iter().zip(expected) {
        for (actual, reference) in vertex.position.iter().zip(expected) {
            assert!(
                (f64::from(*actual) - reference).abs() < POSITION_EPSILON,
                "source t={source_time}: position {actual} != analytic {reference}"
            );
        }
    }
}

fn barycentric(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> [f64; 3] {
    let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    let u = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / d;
    let v = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / d;
    [u, v, 1.0 - u - v]
}
fn color_matches(actual: &[u8], expected: &[u8]) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(a, b)| a.abs_diff(*b) <= COLOR_EPSILON)
}
fn assert_pixels(pixels: &[u8], source_time: f32) -> Value {
    assert_eq!(pixels.len(), SIZE as usize * SIZE as usize * 4);
    // Independent closed-form projection for the fixed axis-aligned camera.
    let screen: Vec<[f64; 2]> = analytic_positions(source_time)
        .iter()
        .map(|p| [96.0 + 32.0 * p[0], 96.0 - 32.0 * (p[1] - 1.5)])
        .collect();
    let mut interior = 0;
    let mut clear = 0;
    let foreground = pixels
        .chunks_exact(4)
        .filter(|p| p[..3] != [0, 0, 0])
        .count();
    for y in 0..SIZE as usize {
        for x in 0..SIZE as usize {
            let mut near_triangle = false;
            let mut expected = None;
            for ids in TRIANGLES {
                let b = barycentric(
                    [x as f64 + 0.5, y as f64 + 0.5],
                    screen[ids[0]],
                    screen[ids[1]],
                    screen[ids[2]],
                );
                // Exclude raster edges (6% interior / 8% outer barycentric
                // margin) and 2.5% of each nearest-texel boundary. These fixed
                // margins avoid tie-breaking differences, not visible errors.
                near_triangle |= b.iter().all(|&v| v >= -0.08);
                if b.iter().all(|&v| v > 0.06) {
                    let u: f64 = (0..3).map(|i| b[i] * (ids[i] % 2) as f64).sum();
                    let v: f64 = (0..3)
                        .map(|i| b[i] * (1.0 - (ids[i] / 2) as f64 / 3.0))
                        .sum();
                    let texel = [u.clamp(0.0, 1.0) * 4.0, v.clamp(0.0, 1.0) * 4.0];
                    if texel.iter().any(|v| v.fract() < 0.025 || v.fract() > 0.975) {
                        continue;
                    }
                    expected = Some(
                        COLORS[CHECKER[(texel[1].floor() as usize).min(3)]
                            [(texel[0].floor() as usize).min(3)]],
                    );
                }
            }
            let offset = (y * SIZE as usize + x) * 4;
            let actual = &pixels[offset..offset + 4];
            if let Some(color) = expected {
                assert!(color_matches(actual, &color),
                    "analytic interior source t={source_time} pixel {x},{y}: {actual:?} != {color:?}");
                interior += 1;
            } else if !near_triangle {
                assert!(
                    color_matches(actual, &[0, 0, 0, 255]),
                    "analytic clear source t={source_time} pixel {x},{y}: {actual:?}"
                );
                clear += 1;
            }
        }
    }
    assert!(interior > 200, "nonvacuous analytic interior: {interior}");
    assert!(foreground > 500, "nonblank mesh coverage: {foreground}");
    assert!(clear > 20_000, "nonvacuous analytic background: {clear}");
    json!({"interior_checked":interior,"clear_checked":clear,"foreground":foreground})
}

fn differing_pixels(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len());
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(a, b)| !color_matches(a, b))
        .count()
}

fn render(
    g: &Wgpu,
    renderer: &mut SkinnedModelRenderer<Wgpu>,
    pose: &Pose,
    target: &<Wgpu as Rhi>::Texture,
    view: &<Wgpu as Rhi>::TextureView,
) -> Vec<u8> {
    renderer
        .draw(
            view,
            (SIZE, SIZE),
            &camera(),
            &light(),
            &[SkinnedInstance::new(pose)],
        )
        .unwrap();
    g.read_texture(target)
}

#[test]
#[ignore = "requires explicit device/capture acceptance"]
fn cli_extract_installed_cooked_clip_matches_source_time_gpu_and_analytic_oracle() {
    let captures = Captures::new();
    let scratch = Scratch::new();
    let (project, authored) = installed(&scratch);
    let source_record = captures.write("original.glb", ORIGINAL);
    let output_record = captures.write("trimmed.glb", &authored);
    let (original, original_cooked) = cook_reload(&project, "original.glb", &captures);
    let (trimmed, trimmed_cooked) = cook_reload(&project, "trimmed.glb", &captures);
    let old = original.source();
    let new = trimmed.source();
    assert_eq!(old.clips.len(), 2);
    assert_eq!(new.clips.len(), old.clips.len() + 1);
    assert_eq!(&new.clips[..old.clips.len()], &old.clips);
    assert_eq!(new.nodes, old.nodes);
    assert_eq!(new.skins, old.skins);
    assert_eq!(new.primitives, old.primitives);
    assert_eq!(new.materials, old.materials);
    assert_eq!(new.images, old.images);
    let extracted_index = old.clips.len() as u32;
    let extracted = &new.clips[extracted_index as usize];
    assert_eq!(extracted.name, CLIP_NAME);
    assert_eq!(extracted.duration(), END - START);
    assert_eq!(extracted.channels.len(), 1);
    assert_eq!(extracted.channels[0].times, [0.0, 0.75, 1.25]);

    let require_software =
        std::env::var_os("ORR_CLIP_EXTRACT_REQUIRE_SOFTWARE").is_some_and(|value| value == "1");
    let g = Wgpu::headless(WgpuOptions {
        force_software: require_software,
        ..WgpuOptions::default()
    })
        .expect("mandatory animation clip extraction acceptance requires a GPU adapter; skipping is forbidden");
    let adapter = g.adapter().get_info();
    let adapter_record = json!({
        "name":adapter.name,"backend":format!("{:?}",adapter.backend),
        "device_type":format!("{:?}",adapter.device_type),"vendor":adapter.vendor,
        "device":adapter.device,"driver":adapter.driver,"driver_info":adapter.driver_info,
        "software":g.is_software()
    });
    captures.write(
        "adapter.json",
        &serde_json::to_vec_pretty(&adapter_record).unwrap(),
    );
    eprintln!("ORR_CLIP_EXTRACT_ADAPTER {adapter_record}");
    if require_software {
        assert!(
            g.is_software(),
            "mandatory CI must observe a software adapter"
        );
        assert_eq!(
            adapter_record["backend"], "Vulkan",
            "mandatory software CI requires Vulkan"
        );
    }
    let format = TextureFormat::Rgba8UnormSrgb;
    let target = g.create_texture(&TextureDesc {
        label: "mandatory extracted clip acceptance",
        width: SIZE,
        height: SIZE,
        format,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let view = g.create_texture_view(&target, None);
    let mut source_renderer =
        SkinnedModelRenderer::new(g.clone(), format, original.clone()).unwrap();
    let mut extracted_renderer =
        SkinnedModelRenderer::new(g.clone(), format, trimmed.clone()).unwrap();
    source_renderer.clear = [0.0, 0.0, 0.0, 1.0];
    extracted_renderer.clear = source_renderer.clear;
    let mut samples = Vec::new();
    let mut previous: Option<Vec<u8>> = None;
    for (index, local_time) in LOCAL_TIMES.into_iter().enumerate() {
        let source_time = START + local_time;
        let source_pose = original.sample_clip(0, source_time).unwrap();
        let extracted_pose = trimmed.sample_clip(extracted_index, local_time).unwrap();
        let preserved_pose = trimmed.sample_clip(0, source_time).unwrap();
        let wrong_time_pose = original.sample_clip(0, local_time).unwrap();
        assert_pose(&original, &source_pose, source_time);
        assert_pose(&trimmed, &extracted_pose, source_time);
        assert_pose(&trimmed, &preserved_pose, source_time);
        assert_pose(&original, &wrong_time_pose, local_time);
        let source_pixels = render(&g, &mut source_renderer, &source_pose, &target, &view);
        let extracted_pixels = render(&g, &mut extracted_renderer, &extracted_pose, &target, &view);
        let preserved_pixels = render(&g, &mut extracted_renderer, &preserved_pose, &target, &view);
        let control_pixels = render(&g, &mut source_renderer, &wrong_time_pose, &target, &view);
        // Persist all actual readbacks before pixel assertions, including failures.
        let images = json!({
            "source":captures.frame(&format!("sample-{index}-original-source-time"), &source_pixels),
            "extracted":captures.frame(&format!("sample-{index}-trimmed-rebased-time"), &extracted_pixels),
            "preserved":captures.frame(&format!("sample-{index}-preserved-original-clip"), &preserved_pixels),
            "control":captures.frame(&format!("sample-{index}-wrong-unrebased-time"), &control_pixels)
        });
        let source_coverage = assert_pixels(&source_pixels, source_time);
        let extracted_coverage = assert_pixels(&extracted_pixels, source_time);
        assert_pixels(&preserved_pixels, source_time);
        assert_pixels(&control_pixels, local_time);
        let equivalent_differences = differing_pixels(&source_pixels, &extracted_pixels);
        let preserved_differences = differing_pixels(&source_pixels, &preserved_pixels);
        let wrong_time_differences = differing_pixels(&extracted_pixels, &control_pixels);
        assert!(equivalent_differences <= MAX_BOUNDARY_PIXELS,
            "source t={source_time}/extracted t={local_time}: {equivalent_differences} pixel differences");
        // Unlike resampled boundaries, unchanged original keys have identical
        // inputs on this same adapter and therefore require byte-exact output.
        assert!(
            source_pixels == preserved_pixels,
            "retained original clip must render byte-identically"
        );
        assert!(
            wrong_time_differences > MIN_CHANGED_PIXELS,
            "wrong-time negative control must differ visibly: {wrong_time_differences}"
        );
        let temporal_differences = previous
            .as_ref()
            .map(|p| differing_pixels(p, &extracted_pixels));
        if let Some(changed) = temporal_differences {
            assert!(
                changed > MIN_CHANGED_PIXELS,
                "extracted clip must visibly animate: {changed}"
            );
        }
        previous = Some(extracted_pixels);
        let sample = json!({
            "local_time":local_time,"source_time":source_time,"images":images,
            "source_coverage":source_coverage,"extracted_coverage":extracted_coverage,
            "equivalent_pixel_differences":equivalent_differences,
            "preserved_pixel_differences":preserved_differences,
            "wrong_time_pixel_differences":wrong_time_differences,
            "temporal_pixel_differences":temporal_differences
        });
        captures.write(
            &format!("sample-{index}.json"),
            &serde_json::to_vec_pretty(&sample).unwrap(),
        );
        samples.push(sample);
    }

    let mut retiming = Vec::new();
    for (label, speed_argument, speed) in
        [("half-speed", "0.5", 0.5_f32), ("double-speed", "2", 2.0)]
    {
        let retimed_scratch = Scratch::new();
        let asset = format!("retimed-{label}.glb");
        let (retimed_project, retimed_authored) =
            installed_with_speed(&retimed_scratch, &asset, Some(speed_argument));
        let authored_record = captures.write(&asset, &retimed_authored);
        let (retimed, retimed_cooked) = cook_reload(&retimed_project, &asset, &captures);
        let retimed_source = retimed.source();
        assert_eq!(retimed_source.clips.len(), old.clips.len() + 1);
        assert_eq!(&retimed_source.clips[..old.clips.len()], &old.clips);
        assert_eq!(retimed_source.nodes, old.nodes);
        assert_eq!(retimed_source.skins, old.skins);
        assert_eq!(retimed_source.primitives, old.primitives);
        assert_eq!(retimed_source.materials, old.materials);
        assert_eq!(retimed_source.images, old.images);
        let clip = &retimed_source.clips[extracted_index as usize];
        assert_eq!(clip.name, CLIP_NAME);
        assert_eq!(clip.duration(), (END - START) / speed);
        assert_eq!(clip.channels.len(), 1);
        assert_eq!(clip.channels[0].times, [0.0, 0.75 / speed, 1.25 / speed]);
        let mut retimed_renderer =
            SkinnedModelRenderer::new(g.clone(), format, retimed.clone()).unwrap();
        retimed_renderer.clear = source_renderer.clear;
        let mut retimed_samples = Vec::new();
        let mut previous: Option<Vec<u8>> = None;
        let mut wrong_rate_control = None;
        for (index, source_offset) in LOCAL_TIMES.into_iter().enumerate() {
            // All constants are dyadic, so both directions of this mapping
            // are exact f32 operations. No production retiming helper is used.
            let source_time = START + source_offset;
            let local_time = source_offset / speed;
            assert_eq!(START + local_time * speed, source_time);
            let source_pose = original.sample_clip(0, source_time).unwrap();
            let retimed_pose = retimed.sample_clip(extracted_index, local_time).unwrap();
            let preserved_pose = retimed.sample_clip(0, source_time).unwrap();
            assert_pose(&original, &source_pose, source_time);
            assert_pose(&retimed, &retimed_pose, source_time);
            assert_pose(&retimed, &preserved_pose, source_time);
            let source_pixels = render(&g, &mut source_renderer, &source_pose, &target, &view);
            let retimed_pixels = render(&g, &mut retimed_renderer, &retimed_pose, &target, &view);
            let preserved_pixels =
                render(&g, &mut retimed_renderer, &preserved_pose, &target, &view);
            let images = json!({
                "source":captures.frame(&format!("retimed-{label}-sample-{index}-original-source-time"), &source_pixels),
                "retimed":captures.frame(&format!("retimed-{label}-sample-{index}-retimed-mapped-time"), &retimed_pixels),
                "preserved":captures.frame(&format!("retimed-{label}-sample-{index}-preserved-original-clip"), &preserved_pixels)
            });
            // Forgetting the speed factor gives t=.75 for half speed and
            // t=.375 for double speed, rather than the correct source t=.5.
            // These interior dyadic times avoid clamping and the fixture's
            // symmetric poses either side of t=1, so both controls must move.
            if index == 1 {
                let wrong_source_time = START + local_time;
                assert!(wrong_source_time > START && wrong_source_time < END);
                assert_ne!(wrong_source_time, source_time);
                let wrong_pose = original.sample_clip(0, wrong_source_time).unwrap();
                assert_pose(&original, &wrong_pose, wrong_source_time);
                let wrong_pixels = render(&g, &mut source_renderer, &wrong_pose, &target, &view);
                let image = captures.frame(
                    &format!("retimed-{label}-wrong-rate-control"),
                    &wrong_pixels,
                );
                let coverage = assert_pixels(&wrong_pixels, wrong_source_time);
                let differences = differing_pixels(&retimed_pixels, &wrong_pixels);
                assert!(
                    differences > MIN_CHANGED_PIXELS,
                    "{label} wrong-rate control must differ visibly: {differences}"
                );
                wrong_rate_control = Some(json!({
                    "sample_index":index,"local_time":local_time,
                    "correct_source_time":source_time,"wrong_source_time":wrong_source_time,
                    "image":image,"coverage":coverage,"pixel_differences":differences
                }));
            }
            let source_coverage = assert_pixels(&source_pixels, source_time);
            let retimed_coverage = assert_pixels(&retimed_pixels, source_time);
            let preserved_coverage = assert_pixels(&preserved_pixels, source_time);
            let equivalent_differences = differing_pixels(&source_pixels, &retimed_pixels);
            let preserved_differences = differing_pixels(&source_pixels, &preserved_pixels);
            assert!(equivalent_differences <= MAX_BOUNDARY_PIXELS,
                "{label} source t={source_time}/retimed t={local_time}: {equivalent_differences} pixel differences");
            assert!(
                source_pixels == preserved_pixels,
                "{label} retained original clip must render byte-identically"
            );
            let temporal_differences = previous
                .as_ref()
                .map(|p| differing_pixels(p, &retimed_pixels));
            if let Some(changed) = temporal_differences {
                assert!(
                    changed > MIN_CHANGED_PIXELS,
                    "{label} retimed clip must visibly animate: {changed}"
                );
            }
            previous = Some(retimed_pixels);
            let sample = json!({
                "local_time":local_time,"source_time":source_time,"images":images,
                "source_coverage":source_coverage,"retimed_coverage":retimed_coverage,
                "preserved_coverage":preserved_coverage,
                "equivalent_pixel_differences":equivalent_differences,
                "preserved_pixel_differences":preserved_differences,
                "temporal_pixel_differences":temporal_differences
            });
            captures.write(
                &format!("retimed-{label}-sample-{index}.json"),
                &serde_json::to_vec_pretty(&sample).unwrap(),
            );
            retimed_samples.push(sample);
        }
        retimed_project.verify().unwrap();
        assert_eq!(
            retimed_project
                .read_asset("clip-extract-acceptance", "original.glb")
                .unwrap(),
            ORIGINAL
        );
        assert_eq!(
            retimed_project
                .read_asset("clip-extract-acceptance", &asset)
                .unwrap(),
            retimed_authored
        );
        let retimed_report = json!({
            "status":"passed","label":label,"speed":speed,
            "source_clip":0,"extracted_clip":extracted_index,"start":START,"end":END,
            "duration":clip.duration(),"key_times":clip.channels[0].times,
            "authored":authored_record,"cooked":retimed_cooked,"samples":retimed_samples,
            "wrong_rate_control":wrong_rate_control.expect("both speeds require a wrong-rate control"),
            "source_hidden_after_install":true,"cooked_reload_before_render":true
        });
        captures.write(
            &format!("retimed-{label}.json"),
            &serde_json::to_vec_pretty(&retimed_report).unwrap(),
        );
        retiming.push(retimed_report);
    }

    // A fresh empty draw must clear the previously rendered mesh; stale contents
    // cannot satisfy any nonblank or movement assertion above.
    extracted_renderer
        .draw(&view, (SIZE, SIZE), &camera(), &light(), &[])
        .unwrap();
    let cleared = g.read_texture(&target);
    let clear_record = captures.frame("empty-draw-clear", &cleared);
    assert!(extracted_renderer.bounds().is_empty());
    assert!(cleared.chunks_exact(4).all(|p| p == [0, 0, 0, 255]));
    project.verify().unwrap();
    assert_eq!(
        project
            .read_asset("clip-extract-acceptance", "original.glb")
            .unwrap(),
        ORIGINAL
    );
    assert_eq!(
        project
            .read_asset("clip-extract-acceptance", "trimmed.glb")
            .unwrap(),
        authored
    );
    let report = json!({
        "schema_version":1,"status":"passed","mandatory_gpu":true,
        "required_software_vulkan":require_software,
        "suite":"animation-clip-extract-consumer","adapter":adapter_record,
        "resolution":[SIZE,SIZE],"format":"Rgba8UnormSrgb","source_clip":0,
        "extracted_clip":extracted_index,"start":START,"end":END,
        "source":source_record,"authored":output_record,
        "original_cooked":original_cooked,"trimmed_cooked":trimmed_cooked,
        "position_tolerance":POSITION_EPSILON,"color_byte_tolerance":COLOR_EPSILON,
        "max_boundary_pixels":MAX_BOUNDARY_PIXELS,"min_changed_pixels_exclusive":MIN_CHANGED_PIXELS,
        "samples":samples,"retiming":retiming,"empty_draw":clear_record,
        "source_hidden_after_install":true,"cooked_reload_before_render":true
    });
    captures.write(
        "acceptance.json",
        &serde_json::to_vec_pretty(&report).unwrap(),
    );
    eprintln!("ORR_CLIP_EXTRACT_ACCEPTANCE {report}");
}

// The reverse gate deliberately invokes a separately built executable, rather
// than authoring::run. Its retained producer/hash is also checked by the CI
// driver. Never substitute cargo run or an in-process fallback here.
fn installed_reverse(
    scratch: &Scratch,
    captures: &Captures,
    cli: &std::path::Path,
    cli_bytes: &[u8],
    label: &str,
    speed: &str,
) -> (Project, AnimatedModel, AnimatedModel, Value) {
    let source = scratch.0.join("source");
    fs::create_dir(&source).unwrap();
    let input = source.join("original.glb");
    let asset = format!("reverse-{label}.glb");
    let output = source.join(&asset);
    fs::write(&input, ORIGINAL).unwrap();
    let arguments = vec![
        OsString::from("--input"),
        input.clone().into_os_string(),
        OsString::from("--output"),
        output.clone().into_os_string(),
        OsString::from("--clip"),
        OsString::from("0"),
        OsString::from("--start"),
        OsString::from("0.25"),
        OsString::from("--end"),
        OsString::from("1.5"),
        OsString::from("--name"),
        OsString::from(CLIP_NAME),
        OsString::from("--speed"),
        OsString::from(speed),
        OsString::from("--direction"),
        OsString::from("reverse"),
    ];
    assert_eq!(fs::read(cli).unwrap(), cli_bytes, "CLI producer changed");
    let result = std::process::Command::new(cli)
        .args(&arguments)
        .output()
        .expect("mandatory reverse acceptance must execute the actual built CLI");
    let stdout = captures.write(&format!("reverse-{label}-cli.stdout"), &result.stdout);
    let stderr = captures.write(&format!("reverse-{label}-cli.stderr"), &result.stderr);
    assert!(
        result.status.success(),
        "reverse CLI failed: {:?}",
        result.status
    );
    assert_eq!(fs::read(cli).unwrap(), cli_bytes, "CLI producer changed");
    assert_eq!(fs::read(&input).unwrap(), ORIGINAL);
    let authored = fs::read(&output).unwrap();
    assert_preserved(ORIGINAL, &authored);
    let authored_record = captures.write(&asset, &authored);
    let mut cooked_records = Vec::new();
    let mut files = vec![String::from("original.glb"), asset.clone()];
    let mut snapshots = vec![
        (String::from("original.glb"), ORIGINAL.to_vec()),
        (asset.clone(), authored),
    ];
    // Production import -> cook -> reload first. Both raw and cooked assets are
    // then installed in a real package and reopened with the source hidden.
    for (name, bytes) in snapshots.clone() {
        let imported =
            orr_model::animation_import::import_with_resolver("animated.glb", &bytes, |_| {
                Err(orr_model::Error::message(
                    "embedded GLB must not resolve any external resource",
                ))
            })
            .unwrap();
        let cooked = imported.to_bytes().unwrap();
        let reloaded = AnimatedModel::from_bytes(&cooked).unwrap();
        assert_eq!(imported.source(), reloaded.source());
        let filename = format!("{name}.cooked.json");
        fs::write(source.join(&filename), &cooked).unwrap();
        cooked_records.push(captures.write(&format!("reverse-{label}-{filename}"), &cooked));
        files.push(filename.clone());
        snapshots.push((filename, cooked));
    }
    fs::write(
        source.join("orr.package.json"),
        serde_json::to_vec(&json!({
            "schema":1,"name":"clip-extract-acceptance","version":"1.0.0",
            "engine":"^0.0.1","capabilities":["animation"],"dependencies":{},"files":files
        }))
        .unwrap(),
    )
    .unwrap();
    let root = scratch.0.join("project");
    fs::create_dir(&root).unwrap();
    Project::open_for_install(&root, Runtime::content_only().engine_version)
        .unwrap()
        .install(std::slice::from_ref(&source))
        .unwrap();
    fs::rename(&source, scratch.0.join("source-hidden-after-install")).unwrap();
    assert!(!source.exists());
    let mut runtime = Runtime::content_only();
    runtime.capabilities.insert("animation".into());
    let project = Project::open(root, runtime).unwrap();
    project.verify().unwrap();
    for (name, bytes) in &snapshots {
        assert_eq!(
            &project.read_asset("clip-extract-acceptance", name).unwrap(),
            bytes
        );
    }
    let original = AnimatedModel::from_bytes(
        &project
            .read_asset("clip-extract-acceptance", "original.glb.cooked.json")
            .unwrap(),
    )
    .unwrap();
    let reversed = AnimatedModel::from_bytes(
        &project
            .read_asset("clip-extract-acceptance", &format!("{asset}.cooked.json"))
            .unwrap(),
    )
    .unwrap();
    let record = json!({
        "argv":arguments.iter().map(|v|v.to_string_lossy()).collect::<Vec<_>>(),
        "exit_code":result.status.code(),"stdout":stdout,"stderr":stderr,
        "authored":authored_record,"cooked":cooked_records,
        "installed_files":files,"source_hidden_after_install":true,
        "installed_cooked_reload_before_render":true
    });
    (project, original, reversed, record)
}

fn assert_reverse_quaternion(pose: &Pose, source_time: f32) {
    // Separate f64 half-angle oracle, including sign-invariant quaternion
    // comparison. No runtime interpolation or authoring output is consulted.
    let t = f64::from(source_time);
    let angle = std::f64::consts::PI / 3.0 * if t <= 1.0 { t } else { 2.0 - t };
    let expected = [0.0, 0.0, (angle / 2.0).sin(), (angle / 2.0).cos()];
    let actual = pose.local()[2].rotation.map(f64::from);
    let dot: f64 = actual.iter().zip(expected).map(|(a, b)| a * b).sum();
    let sign = if dot < 0.0 { -1.0 } else { 1.0 };
    for (a, b) in actual.into_iter().zip(expected) {
        assert!(
            (a * sign - b).abs() < POSITION_EPSILON,
            "source t={source_time}: quaternion component {a} != {b}"
        );
    }
}

#[test]
#[ignore = "requires explicit built CLI and device/capture acceptance"]
fn cli_reverse_extract_installed_cooked_clip_matches_source_time_gpu_and_analytic_oracle() {
    let captures = Captures::new();
    let cli = PathBuf::from(
        std::env::var_os("ORR_CLIP_EXTRACT_CLI")
            .expect("mandatory reverse acceptance requires the actual built ORR_CLIP_EXTRACT_CLI"),
    );
    assert!(cli.is_absolute(), "CLI producer must be an absolute path");
    let cli_bytes = fs::read(&cli).expect("read actual built CLI producer");
    assert!(!cli_bytes.is_empty());
    let cli_record = json!({"path":cli,"bytes":cli_bytes.len(),"fnv1a64":fnv1a64(&cli_bytes)});
    let source_record = captures.write("reverse-original.glb", ORIGINAL);
    let require_software =
        std::env::var_os("ORR_CLIP_EXTRACT_REQUIRE_SOFTWARE").is_some_and(|value| value == "1");
    let g = Wgpu::headless(WgpuOptions {
        force_software: require_software,
        ..WgpuOptions::default()
    })
    .expect("mandatory reverse acceptance requires a GPU adapter; skipping is forbidden");
    let adapter = g.adapter().get_info();
    let adapter_record = json!({
        "name":adapter.name,"backend":format!("{:?}",adapter.backend),
        "device_type":format!("{:?}",adapter.device_type),"vendor":adapter.vendor,
        "device":adapter.device,"driver":adapter.driver,"driver_info":adapter.driver_info,
        "software":g.is_software()
    });
    captures.write(
        "reverse-adapter.json",
        &serde_json::to_vec_pretty(&adapter_record).unwrap(),
    );
    eprintln!("ORR_CLIP_EXTRACT_REVERSE_ADAPTER {adapter_record}");
    if require_software {
        assert!(
            g.is_software(),
            "mandatory CI must observe a software adapter"
        );
        assert_eq!(
            adapter_record["backend"], "Vulkan",
            "mandatory software CI requires Vulkan"
        );
    }
    let format = TextureFormat::Rgba8UnormSrgb;
    let target = g.create_texture(&TextureDesc {
        label: "mandatory reverse clip acceptance",
        width: SIZE,
        height: SIZE,
        format,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let view = g.create_texture_view(&target, None);
    let mut speeds = Vec::new();
    for (label, argument, speed) in [
        ("quarter", "0.25", 0.25_f32),
        ("half", "0.5", 0.5),
        ("unit", "1", 1.0),
        ("double", "2", 2.0),
        ("quadruple", "4", 4.0),
    ] {
        let scratch = Scratch::new();
        let (project, original, reversed, installation) =
            installed_reverse(&scratch, &captures, &cli, &cli_bytes, label, argument);
        let old = original.source();
        let new = reversed.source();
        assert_eq!(old.clips.len(), 2);
        assert_eq!(new.clips.len(), old.clips.len() + 1);
        assert_eq!(&new.clips[..old.clips.len()], &old.clips);
        assert_eq!(new.nodes, old.nodes);
        assert_eq!(new.skins, old.skins);
        assert_eq!(new.primitives, old.primitives);
        assert_eq!(new.materials, old.materials);
        assert_eq!(new.images, old.images);
        let extracted_index = old.clips.len() as u32;
        let clip = &new.clips[extracted_index as usize];
        assert_eq!(clip.name, CLIP_NAME);
        assert_eq!(clip.duration(), (END - START) / speed);
        assert_eq!(clip.channels.len(), 1);
        assert_eq!(clip.channels[0].times, [0.0, 0.5 / speed, 1.25 / speed]);
        let mut source_renderer =
            SkinnedModelRenderer::new(g.clone(), format, original.clone()).unwrap();
        let mut reverse_renderer =
            SkinnedModelRenderer::new(g.clone(), format, reversed.clone()).unwrap();
        source_renderer.clear = [0.0, 0.0, 0.0, 1.0];
        reverse_renderer.clear = source_renderer.clear;
        let mut samples = Vec::new();
        let mut previous: Option<Vec<u8>> = None;
        // Include exact endpoints, both interiors, and the reversed interior key.
        // Dyadic values make independent time mapping exact at all five speeds.
        for (index, offset) in [0.0_f32, 0.25, 0.5, 0.75, 1.0, 1.25]
            .into_iter()
            .enumerate()
        {
            let local_time = offset / speed;
            let source_time = END - local_time * speed;
            assert_eq!(source_time, END - offset);
            let source_pose = original.sample_clip(0, source_time).unwrap();
            let reverse_pose = reversed.sample_clip(extracted_index, local_time).unwrap();
            let preserved_pose = reversed.sample_clip(0, source_time).unwrap();
            for (model, pose) in [
                (&original, &source_pose),
                (&reversed, &reverse_pose),
                (&reversed, &preserved_pose),
            ] {
                assert_pose(model, pose, source_time);
                assert_reverse_quaternion(pose, source_time);
            }
            let source_pixels = render(&g, &mut source_renderer, &source_pose, &target, &view);
            let reverse_pixels = render(&g, &mut reverse_renderer, &reverse_pose, &target, &view);
            let preserved_pixels =
                render(&g, &mut reverse_renderer, &preserved_pose, &target, &view);
            let images = json!({
                "source":captures.frame(&format!("reverse-{label}-{index}-source"), &source_pixels),
                "reversed":captures.frame(&format!("reverse-{label}-{index}-reversed"), &reverse_pixels),
                "preserved":captures.frame(&format!("reverse-{label}-{index}-preserved"), &preserved_pixels)
            });
            let source_coverage = assert_pixels(&source_pixels, source_time);
            let reverse_coverage = assert_pixels(&reverse_pixels, source_time);
            assert_pixels(&preserved_pixels, source_time);
            let differences = differing_pixels(&source_pixels, &reverse_pixels);
            assert!(
                differences <= MAX_BOUNDARY_PIXELS,
                "{label} reverse equivalent differences: {differences}"
            );
            assert_eq!(
                source_pixels, preserved_pixels,
                "retained original clip renders byte-identically"
            );
            // Forward rather than reverse maps this interior sample to .5,
            // while reverse maps it to 1.25. Those poses are not symmetric.
            let wrong_direction = if index == 1 {
                let wrong_source_time = START + local_time * speed;
                let wrong_pose = original.sample_clip(0, wrong_source_time).unwrap();
                assert_pose(&original, &wrong_pose, wrong_source_time);
                assert_reverse_quaternion(&wrong_pose, wrong_source_time);
                let pixels = render(&g, &mut source_renderer, &wrong_pose, &target, &view);
                let image = captures.frame(&format!("reverse-{label}-wrong-direction"), &pixels);
                let coverage = assert_pixels(&pixels, wrong_source_time);
                let changed = differing_pixels(&reverse_pixels, &pixels);
                assert!(
                    changed > MIN_CHANGED_PIXELS,
                    "{label} wrong-direction control must differ visibly: {changed}"
                );
                Some(
                    json!({"source_time":wrong_source_time,"image":image,"coverage":coverage,"pixel_differences":changed}),
                )
            } else {
                None
            };
            let temporal_differences = previous
                .as_ref()
                .map(|p| differing_pixels(p, &reverse_pixels));
            if let Some(changed) = temporal_differences {
                assert!(
                    changed > MIN_CHANGED_PIXELS,
                    "{label} reversed clip must visibly animate: {changed}"
                );
            }
            previous = Some(reverse_pixels);
            samples.push(json!({
                "local_time":local_time,"source_time":source_time,"images":images,
                "source_coverage":source_coverage,"reverse_coverage":reverse_coverage,
                "equivalent_pixel_differences":differences,"preserved_pixel_differences":0,
                "wrong_direction_control":wrong_direction,"temporal_pixel_differences":temporal_differences
            }));
        }
        reverse_renderer
            .draw(&view, (SIZE, SIZE), &camera(), &light(), &[])
            .unwrap();
        let cleared = g.read_texture(&target);
        let clear_record = captures.frame(&format!("reverse-{label}-empty-draw-clear"), &cleared);
        assert!(reverse_renderer.bounds().is_empty());
        assert!(cleared.chunks_exact(4).all(|p| p == [0, 0, 0, 255]));
        project.verify().unwrap();
        assert_eq!(
            project
                .read_asset("clip-extract-acceptance", "original.glb")
                .unwrap(),
            ORIGINAL
        );
        let record = json!({
            "status":"passed","label":label,"speed":speed,"direction":"reverse",
            "source_clip":0,"extracted_clip":extracted_index,"start":START,"end":END,
            "duration":clip.duration(),"key_times":clip.channels[0].times,
            "installation":installation,"samples":samples,"empty_draw":clear_record
        });
        captures.write(
            &format!("reverse-{label}.json"),
            &serde_json::to_vec_pretty(&record).unwrap(),
        );
        speeds.push(record);
    }
    assert_eq!(fs::read(&cli).unwrap(), cli_bytes);
    let report = json!({
        "schema_version":1,"status":"passed","mandatory_gpu":true,
        "suite":"animation-clip-reverse-extract-consumer","direction":"reverse",
        "required_software_vulkan":require_software,"adapter":adapter_record,
        "resolution":[SIZE,SIZE],"format":"Rgba8UnormSrgb","source":source_record,
        "cli":cli_record,"speeds":speeds,"source_clip":0,"start":START,"end":END,
        "position_tolerance":POSITION_EPSILON,"color_byte_tolerance":COLOR_EPSILON,
        "max_boundary_pixels":MAX_BOUNDARY_PIXELS,"min_changed_pixels_exclusive":MIN_CHANGED_PIXELS,
        "source_hidden_after_install":true,"cooked_reload_before_render":true,
        "installed_cooked_reload_before_render":true
    });
    captures.write(
        "reverse-acceptance.json",
        &serde_json::to_vec_pretty(&report).unwrap(),
    );
    eprintln!("ORR_CLIP_EXTRACT_REVERSE_ACCEPTANCE {report}");
}
