//! Mandatory current-source CLI -> cooked-only installed package -> production GPU.
//! Run this single ignored test with ORR_REQUIRE_GPU=1, an absolute pinned
//! ORR_STATIC_MODEL_PLANAR_UV_BIN and fresh ORR_STATIC_MODEL_PLANAR_UV_CAPTURE_DIR.
#![cfg(all(feature = "models", feature = "imported-scene"))]
#![allow(clippy::float_arithmetic)]
#[path = "common/static_model_planar_uv_oracle.rs"]
mod oracle;
use oracle::{Mode, SIZE};
use orr_model::StaticModel;
use orr_package::{Project, Runtime};
use orr_render::orr_rhi::{Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions};
use orr_render::{
    Camera3D, ImportedBatch, ImportedSceneRenderer, ImportedSceneTarget, Lighting, ModelRenderer,
    PointLightSettings, StaticInstance,
};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};
const ASSET_ID: &str = "variants/tile.planar-uv.json";
const ASSET: &str = "tile.model.json";
const PACKAGE: &str = "static-planar-uv-gpu";
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
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(name))
        .expect("fresh evidence only");
    f.write_all(bytes).unwrap();
    f.sync_all().unwrap();
    json!({"file":name,"bytes":bytes.len(),"fnv1a64":hash(bytes)})
}
// Minimal test-owned PNG encoder, stored DEFLATE blocks, no new dependency.
fn png(pixels: &[u8]) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let mut crc = !0_u32;
        for &b in kind.iter().chain(data) {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb88320 & 0_u32.wrapping_sub(crc & 1));
            }
        }
        out.extend_from_slice(&(!crc).to_be_bytes());
    }
    let mut raw = Vec::new();
    for row in pixels.chunks_exact((SIZE * 4) as usize) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut compressed = vec![0x78, 0x01];
    let chunks = raw.chunks(65535);
    let count = chunks.len();
    for (i, block) in chunks.enumerate() {
        compressed.push(u8::from(i + 1 == count));
        let len = block.len() as u16;
        compressed.extend_from_slice(&len.to_le_bytes());
        compressed.extend_from_slice(&(!len).to_le_bytes());
        compressed.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for byte in raw {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    compressed.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&SIZE.to_be_bytes());
    ihdr.extend_from_slice(&SIZE.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut bytes, b"IHDR", &ihdr);
    chunk(&mut bytes, b"IDAT", &compressed);
    chunk(&mut bytes, b"IEND", &[]);
    bytes
}
fn capture(root: &Path, name: &str, pixels: &[u8]) -> Value {
    json!({"rgba":write(root,&format!("{name}.rgba"),pixels),"png":write(root,&format!("{name}.png"),&png(pixels))})
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn produce(binary: &Path, recipe: &Path, output: &Path, evidence: &Path, label: &str) {
    let result = Command::new(binary)
        .args(["--recipe"])
        .arg(recipe)
        .args(["--asset-id", ASSET_ID, "--output"])
        .arg(output)
        .output()
        .expect("execute explicit producer");
    write(evidence, &format!("{label}-stdout.txt"), &result.stdout);
    write(evidence, &format!("{label}-stderr.txt"), &result.stderr);
    assert!(
        result.status.success(),
        "producer failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
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
    Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 3.0)
}
fn render(g: &Wgpu, model: StaticModel, target: &<Wgpu as Rhi>::Texture, scene: bool) -> Vec<u8> {
    let format = TextureFormat::Rgba8UnormSrgb;
    let mut renderer = ModelRenderer::new(g.clone(), format, model).unwrap();
    renderer.clear = [0.0, 0.0, 0.0, 1.0];
    let view = g.create_texture_view(target, None);
    if scene {
        let mut combined = ImportedSceneRenderer::new(g.clone(), format).unwrap();
        combined.clear = [0.0, 0.0, 0.0, 1.0];
        let instances = [StaticInstance::default()];
        combined
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
                &mut [ImportedBatch::StaticInstances {
                    renderer: &mut renderer,
                    instances: &instances,
                }],
            )
            .unwrap();
    } else {
        renderer
            .draw(&view, (SIZE, SIZE), &camera(), &lighting())
            .unwrap();
    }
    g.read_texture(target)
}
fn control(original: &StaticModel, mode: Mode) -> StaticModel {
    let mut source = original.source().clone();
    for p in &mut source.primitives {
        if p.material == 0 {
            for v in &mut p.vertices {
                v.uv =
                    oracle::projected([f64::from(v.position[0]), f64::from(v.position[1])], mode)
                        .map(|v| v as f32);
            }
        }
    }
    StaticModel::new(source).unwrap()
}
fn assert_preserved(before: &StaticModel, after: &StaticModel) {
    let (a, b) = (before.source(), after.source());
    assert_eq!(b.asset_id, ASSET_ID);
    assert_eq!(a.images, b.images);
    assert_eq!(a.materials, b.materials);
    assert_eq!(a.primitives.len(), b.primitives.len());
    for (old, new) in a.primitives.iter().zip(&b.primitives) {
        assert_eq!(
            new.id,
            format!(
                "{ASSET_ID}{}",
                old.id.strip_prefix(oracle::SOURCE_ID).unwrap()
            )
        );
        assert_eq!(old.material, new.material);
        assert_eq!(old.transform, new.transform);
        assert_eq!(old.indices, new.indices);
        assert_eq!(old.vertices.len(), new.vertices.len());
        for (old_v, new_v) in old.vertices.iter().zip(&new.vertices) {
            assert_eq!(
                old_v.position.map(f32::to_bits),
                new_v.position.map(f32::to_bits)
            );
            assert_eq!(
                old_v.normal.map(f32::to_bits),
                new_v.normal.map(f32::to_bits)
            );
            if old.material != 0 {
                assert_eq!(old_v.uv.map(f32::to_bits), new_v.uv.map(f32::to_bits));
            } else {
                assert_eq!(old_v.uv, [0.0, 0.0]);
                // Stored fixture Z was rounded to f32 when authored. Preserve
                // that input rounding for the exact-bit vertex check; the
                // independent screen oracle uses the analytic plane instead.
                let [x, y, z] = old_v.position.map(f64::from);
                let model_x = ((x - 0.25 * y) + 0.5 * z) - 0.25;
                let model_y = (0.25 * x + y) + 0.125;
                let want = [
                    ((model_x - 0.125) / 0.75) as f32,
                    ((model_y + 0.25) / 0.625) as f32,
                ];
                assert_eq!(new_v.uv.map(f32::to_bits), want.map(f32::to_bits));
            }
        }
    }
    assert_eq!(
        b.dependencies
            .iter()
            .map(|d| d.uri.as_str())
            .collect::<Vec<_>>(),
        ["$source", oracle::SOURCE_PATH]
    );
}
#[test]
#[ignore = "requires explicit current-source producer, software Vulkan, fresh retained evidence"]
fn static_planar_uv_matches_independent_gpu_oracle() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let evidence = PathBuf::from(
        std::env::var_os("ORR_STATIC_MODEL_PLANAR_UV_CAPTURE_DIR")
            .expect("explicit retained captures required"),
    );
    assert!(evidence.is_absolute());
    fs::create_dir_all(&evidence).unwrap();
    assert_eq!(fs::read_dir(&evidence).unwrap().count(), 0);
    let binary = PathBuf::from(
        std::env::var_os("ORR_STATIC_MODEL_PLANAR_UV_BIN")
            .expect("explicit current-source producer required, no fallback"),
    );
    assert!(binary.is_absolute());
    let provenance = json!({"producer_path":binary,"producer_fnv1a64":hash(&fs::read(&binary).unwrap()),
        "test_source_fnv1a64":hash(include_bytes!("static_model_planar_uv.rs")),
        "oracle_source_fnv1a64":hash(include_bytes!("common/static_model_planar_uv_oracle.rs")),
        "cli_source_fnv1a64":hash(include_bytes!("../../orr_model/examples/static_model_planar_uv.rs")),
        "authoring_source_fnv1a64":hash(include_bytes!("../../orr_model/examples/static_model_planar_uv/authoring.rs"))});
    write(
        &evidence,
        "provenance.json",
        &serde_json::to_vec_pretty(&provenance).unwrap(),
    );
    let imported_record = write(&evidence, "imported-source.glb", oracle::ORIGINAL);
    let root = std::env::temp_dir().join(format!("orr-planar-uv-gpu-{}", std::process::id()));
    fs::create_dir(&root).expect("fresh owned scratch");
    let scratch = Scratch(fs::canonicalize(root).unwrap());
    let authoring = scratch.0.join("authoring");
    fs::create_dir_all(authoring.join("inputs")).unwrap();
    let original = oracle::fixture();
    let source_bytes = original.to_bytes().unwrap();
    let source_record = write(&evidence, "source.model.json", &source_bytes);
    fs::write(authoring.join(oracle::SOURCE_PATH), &source_bytes).unwrap();
    let recipe_bytes = serde_json::to_vec(&oracle::recipe()).unwrap();
    let recipe_record = write(&evidence, "tile.planar-uv.json", &recipe_bytes);
    let recipe_path = authoring.join("tile.planar-uv.json");
    fs::write(&recipe_path, &recipe_bytes).unwrap();
    let package = scratch.0.join("authored-package");
    fs::create_dir(&package).unwrap();
    produce(
        &binary,
        &recipe_path,
        &package.join(ASSET),
        &evidence,
        "producer",
    );
    produce(
        &binary,
        &recipe_path,
        &scratch.0.join("deterministic.model.json"),
        &evidence,
        "repeat-producer",
    );
    let cooked = fs::read(package.join(ASSET)).unwrap();
    let cooked_record = write(&evidence, ASSET, &cooked);
    assert_eq!(
        cooked,
        fs::read(scratch.0.join("deterministic.model.json")).unwrap()
    );
    let manifest=serde_json::to_vec(&json!({"schema":1,"name":PACKAGE,"version":"1.0.0","engine":"^0.0.1","capabilities":["models"],"dependencies":{},"files":[ASSET]})).unwrap();
    fs::write(package.join("orr.package.json"), &manifest).unwrap();
    let manifest_record = write(&evidence, "orr.package.json", &manifest);
    let assets = json!({"imported_glb":imported_record,"source":source_record,"recipe":recipe_record,
        "cooked":cooked_record,"manifest":manifest_record,"asset_id":ASSET_ID,"source_path":oracle::SOURCE_PATH});
    write(
        &evidence,
        "assets.json",
        &serde_json::to_vec_pretty(&assets).unwrap(),
    );
    let project_root = scratch.0.join("project");
    fs::create_dir(&project_root).unwrap();
    Project::open_for_install(&project_root, Runtime::content_only().engine_version)
        .unwrap()
        .install(std::slice::from_ref(&package))
        .unwrap();
    fs::rename(&authoring, scratch.0.join("hidden-authoring")).unwrap();
    fs::rename(&package, scratch.0.join("hidden-package")).unwrap();
    assert!(
        !authoring.exists()
            && !package.exists()
            && !project_root.join(oracle::SOURCE_PATH).exists()
    );
    let mut runtime = Runtime::content_only();
    runtime.capabilities.insert("models".into());
    let project = Project::open(&project_root, runtime).unwrap();
    assert_eq!(project.verify().unwrap().packages[PACKAGE].files.len(), 1);
    let installed = project.read_asset(PACKAGE, ASSET).unwrap();
    assert_eq!(installed, cooked);
    let authored = StaticModel::from_bytes(&installed).unwrap();
    assert_eq!(authored.to_bytes().unwrap(), installed);
    let g = Wgpu::headless(WgpuOptions {
        force_software: true,
        ..Default::default()
    })
    .expect("mandatory GPU may not skip");
    let adapter = g.adapter().get_info();
    let adapter = json!({"name":adapter.name,"backend":format!("{:?}",adapter.backend),"device_type":format!("{:?}",adapter.device_type),"driver":adapter.driver,"driver_info":adapter.driver_info,"vendor":adapter.vendor,"device":adapter.device,"software":g.is_software()});
    write(
        &evidence,
        "adapter.json",
        &serde_json::to_vec_pretty(&adapter).unwrap(),
    );
    assert!(g.is_software());
    assert_eq!(adapter["backend"], "Vulkan");
    let target = g.create_texture(&TextureDesc {
        label: "planar UV acceptance",
        width: SIZE,
        height: SIZE,
        format: TextureFormat::Rgba8UnormSrgb,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let source = render(&g, original.clone(), &target, false);
    let projected = render(&g, authored.clone(), &target, false);
    let scene = render(&g, authored.clone(), &target, true);
    let axis = render(&g, control(&original, Mode::WrongAxis), &target, false);
    let repeat = render(&g, control(&original, Mode::WrongRepeat), &target, false);
    let local = render(&g, control(&original, Mode::LocalSpace), &target, false);
    let mut affine = original.source().clone();
    for p in &mut affine.primitives {
        if p.material == 0 {
            for v in &mut p.vertices {
                v.uv = [0.25, 0.75];
            }
        }
    }
    let affine = render(&g, StaticModel::new(affine).unwrap(), &target, false);
    // Every raw/PNG frame, input and provenance survives any following assertion.
    let frames = json!({"source":capture(&evidence,"source",&source),"projected":capture(&evidence,"projected",&projected),"scene":capture(&evidence,"scene",&scene),"wrong_axis":capture(&evidence,"wrong-axis",&axis),"wrong_repeat":capture(&evidence,"wrong-repeat",&repeat),"local_space":capture(&evidence,"local-space",&local),"affine_collapsed":capture(&evidence,"affine-collapsed",&affine)});
    write(
        &evidence,
        "frames.json",
        &serde_json::to_vec_pretty(&frames).unwrap(),
    );
    assert_preserved(&original, &authored);
    let coverage = json!({"source":oracle::assert_pixels(&source,Mode::Original),"projected":oracle::assert_pixels(&projected,Mode::Projected),"wrong_axis":oracle::assert_pixels(&axis,Mode::WrongAxis),"wrong_repeat":oracle::assert_pixels(&repeat,Mode::WrongRepeat),"local_space":oracle::assert_pixels(&local,Mode::LocalSpace)});
    assert_eq!(projected, scene, "both production renderer paths agree");
    let mut differences = [0usize; 5];
    let mut untouched = 0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            match oracle::expected(x, y, Mode::Projected) {
                oracle::Expected::Interior { target: true, .. } => {
                    for (n, control) in [&source, &axis, &repeat, &local, &affine]
                        .into_iter()
                        .enumerate()
                    {
                        if !oracle::matches(&projected[i..i + 4], &control[i..i + 4]) {
                            differences[n] += 1;
                        }
                    }
                }
                oracle::Expected::Interior { target: false, .. } => {
                    assert_eq!(&source[i..i + 4], &projected[i..i + 4]);
                    untouched += 1;
                }
                _ => (),
            }
        }
    }
    assert!(
        differences.iter().all(|&v| v > 3000),
        "visible controls must differ on target interior: {differences:?}"
    );
    assert!(untouched > 500);
    assert_eq!(
        fs::read(scratch.0.join("hidden-authoring").join(oracle::SOURCE_PATH)).unwrap(),
        source_bytes
    );
    let receipt = json!({"schema":1,"status":"passed","width":SIZE,"height":SIZE,"assets":assets,"adapter":adapter,"provenance":provenance,"frames":frames,"coverage":coverage,
        "control_order":["original_allzero","wrong_axis","wrong_repeat","local_space","affine_collapsed"],"control_pixel_differences":differences,"non_target_identical_pixels":untouched,
        "installed_exact_cli_bytes":true,"authoring_hidden":true,"source_unchanged":true,"geometry_preserved":true,"production_renderers_equal":true,
        "camera":{"eye":[0,0,5],"target":[0,0,0],"orthographic_half_height":3},"lighting":{"ambient":1,"sky":[1,1,1],"ground":[1,1,1],"direct_intensity":0,"tonemap":false},
        "thresholds":{"color_bytes":oracle::EPSILON,"barycentric_margin":0.04,"texel_tie_margin":0.002,"minimum_control_pixels":3000}});
    write(
        &evidence,
        "receipt.json",
        &serde_json::to_vec_pretty(&receipt).unwrap(),
    );
    eprintln!("ORR_STATIC_MODEL_PLANAR_UV_ACCEPTANCE {receipt}");
}
