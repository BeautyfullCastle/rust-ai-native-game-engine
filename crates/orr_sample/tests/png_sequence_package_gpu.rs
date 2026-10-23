//! Real CLI -> immutable package install -> public sprite runtime -> GPU readback.
//! Build the example first and set ORR_PNG_SEQUENCE_PACKAGE_BIN to its absolute path.
//! ORR_REQUIRE_GPU=1 makes both missing CLI and missing GPU fatal.
#![cfg(all(feature = "project-create", target_os = "linux"))]
#![allow(clippy::float_arithmetic)]

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use orr_render::orr_rhi::{Rhi, TextureFormat, Wgpu, WgpuOptions};
use orr_render::{Camera, OffscreenTarget, RenderList};
use orr_sample::project_compositor::ProjectCompositor;
use orr_sample::project_runtime::{compiled_runtime, PreparedRuntime};

#[path = "common/project.rs"]
mod project_fixture;
use project_fixture::ProjectFixture;

// These are source-side expectations, never obtained from the generated atlas,
// generated regions, clip metadata, or a previous GPU capture. Every 3x2 frame
// is asymmetric and distinct, including a non-endpoint sRGB value.
const R: [u8; 4] = [255, 0, 0, 255];
const G: [u8; 4] = [0, 255, 0, 255];
const B: [u8; 4] = [0, 0, 255, 255];
const Y: [u8; 4] = [255, 255, 0, 255];
const C: [u8; 4] = [0, 255, 255, 255];
const M: [u8; 4] = [255, 0, 255, 255];
const GRAY: [u8; 4] = [128, 128, 128, 255];
const FRAMES: [[[u8; 4]; 6]; 3] = [
    [R, G, B, Y, C, M],
    [C, GRAY, Y, B, M, R],
    [M, B, C, GRAY, R, G],
];

fn write_png(path: &Path, pixels: &[[u8; 4]; 6]) {
    let file = fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(file, 3, 2);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let bytes: Vec<_> = pixels.iter().flatten().copied().collect();
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&bytes)
        .unwrap();
}

fn tree_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                visit(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

fn pixel(capture: &[u8], x: usize, y: usize) -> [u8; 4] {
    let offset = (y * 96 + x) * 4;
    capture[offset..offset + 4].try_into().unwrap()
}

fn matches_frame(capture: &[u8], expected: &[[u8; 4]; 6]) -> bool {
    // Camera half extent 2 and a 3x2 sprite: x=24..72, y=16..48.
    // Sample all six texel centers, safely away from edges and neighboring cells.
    [(32, 24), (48, 24), (64, 24), (32, 40), (48, 40), (64, 40)]
        .into_iter()
        .zip(expected)
        .all(|((x, y), expected)| {
            pixel(capture, x, y)
                .iter()
                .zip(expected)
                .all(|(actual, expected)| actual.abs_diff(*expected) <= 3)
        })
}

#[test]
fn cli_sequence_installed_runtime_loop_once_and_wrong_order_gpu_readback() {
    let Some(binary) = std::env::var_os("ORR_PNG_SEQUENCE_PACKAGE_BIN") else {
        assert!(std::env::var_os("ORR_REQUIRE_GPU").is_none(),
            "required CLI missing: set ORR_PNG_SEQUENCE_PACKAGE_BIN to the built png_sequence_package example");
        eprintln!("SKIP: ORR_PNG_SEQUENCE_PACKAGE_BIN not set");
        return;
    };
    let binary = Path::new(&binary).canonicalize().expect("built CLI exists");
    let fixture = ProjectFixture::new();
    let inputs = fixture.root.join("authoring-inputs");
    fs::create_dir(&inputs).unwrap();
    // Deliberately nonlexical order: a directory scan/sort must not replace recipe order.
    let names = ["z-first.png", "a-second.png", "m-third.png"];
    for (name, pixels) in names.iter().zip(&FRAMES) {
        write_png(&inputs.join(name), pixels);
    }
    fs::write(inputs.join("license.txt"), "CC0-1.0 test-owned pixels\n").unwrap();
    let packages = fixture.root.join("generated-packages");
    fs::create_dir(&packages).unwrap();
    let mut sources = Vec::new();
    for (name, mode, order) in [
        ("sequence-loop", "loop", [0, 1, 2]),
        ("sequence-once", "once", [0, 1, 2]),
        ("sequence-wrong-order", "loop", [2, 1, 0]),
    ] {
        let recipe = serde_json::json!({
            "schema": 1, "name": name, "version": "1.0.0",
            "license": "license.txt", "clip": "run", "mode": mode,
            "columns": 2,
            "frames": [
                {"png": names[order[0]], "duration_ms": 100},
                {"png": names[order[1]], "duration_ms": 150},
                {"png": names[order[2]], "duration_ms": 200}
            ]
        });
        let recipe_path = inputs.join(format!("{name}.json"));
        fs::write(&recipe_path, serde_json::to_vec(&recipe).unwrap()).unwrap();
        let output = packages.join(name);
        let result = Command::new(&binary)
            .current_dir(&inputs)
            .arg("--recipe")
            .arg(&recipe_path)
            .arg("--out")
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "CLI {name}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        for duplicate in ["--recipe", "--out"] {
            let rejected_output = packages.join(format!("rejected-{name}-{duplicate}"));
            let result = Command::new(&binary)
                .arg("--recipe")
                .arg(&recipe_path)
                .arg("--out")
                .arg(&rejected_output)
                .arg(duplicate)
                .arg(if duplicate == "--recipe" {
                    &recipe_path
                } else {
                    &rejected_output
                })
                .output()
                .unwrap();
            assert!(!result.status.success(), "duplicate {duplicate} must fail");
            assert!(
                !rejected_output.exists(),
                "duplicate option must not publish"
            );
        }
        sources.push(output);
    }
    orr_package::Project::open(&fixture.root, compiled_runtime())
        .unwrap()
        .install(&sources)
        .unwrap();
    // Admission and rendering must survive removal of every source/recipe/output.
    fs::remove_dir_all(&inputs).unwrap();
    fs::remove_dir_all(&packages).unwrap();
    let installed = orr_package::Project::open(&fixture.root, compiled_runtime()).unwrap();
    installed.verify().unwrap();
    for name in ["sequence-loop", "sequence-once", "sequence-wrong-order"] {
        let verified = installed.verify_package(name).unwrap();
        assert!(verified.verified_packages.contains_key(name));
    }
    fs::write(fixture.root.join("arena.scene.yaml"),
        "schema: orr.scene/1\nsingletons:\n  Score: { kills: [0,0,0,0,0,0,0,0] }\nentities:\n  e_00000001:\n    Position: { pos: [0,0] }\n    PlayerTag: { slot: 0 }\n").unwrap();

    let gpu = match Wgpu::headless(WgpuOptions::default()) {
        Ok(gpu) => gpu,
        Err(error) => {
            assert!(
                std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "required GPU unavailable: {error}"
            );
            eprintln!("SKIP: {error}");
            return;
        }
    };
    eprintln!(
        "PNG sequence adapter: {} (software: {})",
        gpu.adapter_name(),
        gpu.is_software()
    );
    let target = OffscreenTarget::new(&gpu, 96, 64, TextureFormat::Rgba8UnormSrgb);
    // With the 3:2 viewport Camera normalizes by the smaller dimension:
    // half extent 2 yields 16 screen pixels per world unit.
    let camera = Camera::new([0.0, 0.0], 2.0);
    let mut captures = 0;
    for (name, once, wrong_order) in [
        ("sequence-loop", false, false),
        ("sequence-once", true, false),
        ("sequence-wrong-order", false, true),
    ] {
        fs::write(fixture.root.join("arena.sprites.json"), serde_json::to_vec(&serde_json::json!({
            "version": 1, "scene": "arena.scene.yaml", "project": ".",
            "bindings": {"e_00000001": {"package": name, "document": "sprites.json", "source": {"Clip": "run"}, "units_per_pixel": 1.0}}
        })).unwrap()).unwrap();
        let before_read = tree_bytes(&fixture.root);
        let admitted_project =
            orr_sample::project::PreparedProject::open(&fixture.root, compiled_runtime()).unwrap();
        assert!(admitted_project.sprites().is_some());
        let prepared = PreparedRuntime::open(&fixture.root).unwrap();
        let initial_checksum = prepared.initial_frame().checksum();
        let mut bridge = prepared.bridge().unwrap();
        let (_, mut presentation) = prepared.into_parts().unwrap();
        use orr_bridge::Bridge;
        presentation.update(bridge.snapshot().as_ref()).unwrap();
        assert_eq!(presentation.sprites().len(), 1);
        let admitted = presentation
            .assets()
            .get(&(name.into(), "sprites.json".into()))
            .unwrap();
        assert_eq!(
            (
                admitted.document.atlas().width,
                admitted.document.atlas().height
            ),
            (6, 4)
        );
        let binding = &presentation.document().unwrap().bindings["e_00000001"];
        let mut compositor =
            ProjectCompositor::new(gpu.clone(), target.format(), presentation.assets()).unwrap();
        for (time, loop_frame, once_frame) in [
            (0, 0, 0),
            (99, 0, 0),
            (100, 1, 1),
            (249, 1, 1),
            (250, 2, 2),
            (449, 2, 2),
            (450, 0, 2),
            (550, 1, 2),
            (900, 0, 2),
            (u64::MAX, 0, 2),
        ] {
            let expected = if once { once_frame } else { loop_frame };
            let mut sprite = presentation.sprites()[0].clone();
            sprite.instance.region = binding.region(&admitted.document, time).unwrap();
            compositor
                .draw(
                    target.render_view(),
                    target.size(),
                    &RenderList::new(),
                    &[sprite],
                    &camera,
                )
                .unwrap();
            let capture = target.read_rgba8();
            let actual_frame = if wrong_order { 2 - expected } else { expected };
            assert!(matches_frame(&capture, &FRAMES[actual_frame]),
                "{name} at {time}ms: six source-known texels differ, expected source frame {actual_frame}; samples {:?}",
                [(32,24),(48,24),(64,24),(32,40),(48,40),(64,40)].map(|(x,y)| pixel(&capture,x,y)));
            if wrong_order && expected != 1 {
                assert!(!matches_frame(&capture, &FRAMES[expected]), "wrong-order negative control must fail the correct recipe pixel oracle at {time}ms");
            }
            captures += 1;
        }
        assert_eq!(
            bridge.host().session().frame().checksum(),
            initial_checksum,
            "sampling and GPU readback must not alter simulation"
        );
        // Also drive the real PlayHost clock and presentation, without manually
        // selecting a region. At 60 Hz these cross the 100/250/450 ms boundaries.
        let mut previous_tick = 0;
        for (tick, loop_frame, once_frame) in [
            (0, 0, 0),
            (5, 0, 0),
            (6, 1, 1),
            (14, 1, 1),
            (15, 2, 2),
            (26, 2, 2),
            (27, 0, 2),
            (33, 1, 2),
            (54, 0, 2),
        ] {
            // Feed every coherent snapshot: the production presenter intentionally
            // resets its cursor after a gap greater than eight ticks.
            for _ in previous_tick..tick {
                bridge.step(1);
                presentation.update(bridge.snapshot().as_ref()).unwrap();
            }
            previous_tick = tick;
            let before_render = bridge.host().session().frame().checksum();
            presentation.update(bridge.snapshot().as_ref()).unwrap();
            let expected = if once { once_frame } else { loop_frame };
            let expected = if wrong_order { 2 - expected } else { expected };
            compositor
                .draw(
                    target.render_view(),
                    target.size(),
                    presentation.shapes(),
                    presentation.sprites(),
                    &camera,
                )
                .unwrap();
            let capture = target.read_rgba8();
            assert!(
                matches_frame(&capture, &FRAMES[expected]),
                "{name} real PlayHost tick {tick}: expected source frame {expected}; samples {:?}",
                [(32, 24), (48, 24), (64, 24), (32, 40), (48, 40), (64, 40)]
                    .map(|(x, y)| pixel(&capture, x, y))
            );
            assert_eq!(
                bridge.host().session().frame().checksum(),
                before_render,
                "presentation/readback must not mutate PlayHost at tick {tick}"
            );
            captures += 1;
        }
        assert_eq!(
            tree_bytes(&fixture.root),
            before_read,
            "admission/playback/readback must not modify installed project bytes"
        );
    }
    eprintln!("PNG sequence: {captures} production GPU captures; source-hidden installed packages; Loop/Once boundaries; reversed-recipe negative control");
}
