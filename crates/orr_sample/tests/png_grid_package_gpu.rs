//! Twelve real Vulkan captures through the installed grid package and PlayHost.
//! The pixel oracle uses only literal source texels and fixed screen geometry.
#![cfg(all(feature = "project-create", target_os = "linux"))]
#![allow(clippy::float_arithmetic)]

use orr_bridge::{Bridge, PlayerSlot};
use orr_fp::FP;
use orr_render::orr_rhi::{Rhi, TextureFormat, Wgpu, WgpuOptions};
use orr_render::{Camera, OffscreenTarget, RenderList};
use orr_sample::{
    arena_game::{ArenaInput, Position},
    project_compositor::ProjectCompositor,
    project_runtime::{compiled_runtime, PreparedRuntime},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
#[path = "common/project.rs"]
mod project_fixture;
use project_fixture::ProjectFixture;

const HERO: &str = "e_00000001";
const R: [u8; 4] = [233, 31, 19, 255];
const G: [u8; 4] = [19, 211, 47, 255];
const B: [u8; 4] = [27, 61, 229, 255];
const P: [u8; 4] = [128, 71, 199, 128];
const Z: [u8; 4] = [231, 117, 53, 0];
// Four distinct 2x2 cells, each internally asymmetric. Row-major inside each cell.
const CELLS: [[[u8; 4]; 4]; 4] = [[R, G, P, Z], [B, Z, R, P], [G, P, Z, B], [P, R, B, G]];
const CAPTURE_TICKS: [u64; 11] = [0, 5, 6, 11, 12, 13, 18, 19, 24, 25, 26];

fn png(path: &Path, width: u32, height: u32, rgba: &[u8]) {
    let mut encoder = png::Encoder::new(fs::File::create(path).unwrap(), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(rgba)
        .unwrap();
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, at: &Path, result: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(at).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            assert!(!kind.is_symlink(), "fixture tree contains symlink");
            let path = entry.path();
            if kind.is_dir() {
                visit(root, &path, result);
            } else {
                assert!(kind.is_file());
                result.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}
fn linear(c: u8) -> f64 {
    let c = f64::from(c) / 255.;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
fn srgb(c: f64) -> u8 {
    let c = if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1. / 2.4) - 0.055
    };
    (c * 255.).round() as u8
}
fn oracle(cell: usize) -> Vec<u8> {
    let mut result = Vec::with_capacity(64 * 64 * 4);
    for y in 0..64 {
        for x in 0..64 {
            // Camera half extent 4 gives eight framebuffer pixels per world unit.
            // A 2x2 source cell is centered on the followed actor.
            let cx = (f64::from(x) + 0.5 - 32.) / 8. + 1.;
            let cy = (f64::from(y) + 0.5 - 32.) / 8. + 1.;
            let p = if (0.0..2.0).contains(&cx) && (0.0..2.0).contains(&cy) {
                CELLS[cell][cy.floor() as usize * 2 + cx.floor() as usize]
            } else {
                Z
            };
            let a = f64::from(p[3]) / 255.;
            result.extend([
                srgb(linear(p[0]) * a + 0.02 * (1. - a)),
                srgb(linear(p[1]) * a + 0.02 * (1. - a)),
                srgb(linear(p[2]) * a + 0.035 * (1. - a)),
                255,
            ]);
        }
    }
    result
}
fn matches(actual: &[u8], expected: &[u8]) -> bool {
    actual.len() == expected.len()
        && actual
            .iter()
            .zip(expected)
            .all(|(a, b)| a.abs_diff(*b) <= 3)
}
fn expected_cell(tick: u64, wrong_order: bool) -> usize {
    if (13..26).contains(&tick) {
        let second = tick >= 19;
        if second ^ wrong_order {
            3
        } else {
            2
        }
    } else if tick >= 26 {
        0
    } else {
        usize::from((6..12).contains(&tick))
    }
}

#[test]
#[ignore = "requires built grid CLI and Vulkan; explicit gate runs exact one test"]
fn grid_cli_installed_locomotion_twelve_capture_pixels_and_frame_parity() {
    assert_eq!(
        std::env::var("ORR_REQUIRE_GPU").as_deref(),
        Ok("1"),
        "explicit GPU acceptance must fail closed"
    );
    let binary =
        PathBuf::from(std::env::var_os("ORR_PNG_GRID_PACKAGE_BIN").expect("grid CLI required"))
            .canonicalize()
            .unwrap();
    let evidence = PathBuf::from(
        std::env::var_os("ORR_GRID_PIXEL_EVIDENCE").expect("fresh evidence directory required"),
    );
    fs::create_dir(&evidence).expect("evidence directory must not already exist");
    let fixture = ProjectFixture::new();
    let inputs = fixture.root.join("grid-private-inputs");
    let outputs = fixture.root.join("grid-generated");
    fs::create_dir(&inputs).unwrap();
    fs::create_dir(&outputs).unwrap();
    let producer = inputs.join("grid-producer");
    fs::copy(&binary, &producer).unwrap();
    let producer_hash = hash(&fs::read(&producer).unwrap());
    let mut sheet = Vec::new();
    for y in 0..4 {
        for x in 0..4 {
            sheet.extend(CELLS[(y / 2) * 2 + x / 2][(y % 2) * 2 + x % 2]);
        }
    }
    png(&inputs.join("sheet.png"), 4, 4, &sheet);
    let sheet_bytes = fs::read(inputs.join("sheet.png")).unwrap();
    fs::write(inputs.join("license.txt"), "Synthetic fixture, CC0-1.0\n").unwrap();
    let mut packages = Vec::new();
    for (name, order) in [("grid-good", [2, 3]), ("grid-wrong-order", [3, 2])] {
        let recipe = json!({"schema":1,"name":name,"version":"1.0.0","license":"license.txt","png":"sheet.png",
            "cell_width":2,"cell_height":2,"columns":2,"rows":2,
            "clips":[{"id":"idle","mode":"loop","frames":[{"cell":0,"duration_ms":100},{"cell":1,"duration_ms":100}]},
                {"id":"walk","mode":"once","frames":[{"cell":order[0],"duration_ms":100},{"cell":order[1],"duration_ms":100}]}]});
        let recipe_path = inputs.join(format!("{name}.json"));
        fs::write(&recipe_path, serde_json::to_vec(&recipe).unwrap()).unwrap();
        let output = outputs.join(name);
        let result = Command::new(&producer)
            .arg("--recipe")
            .arg(&recipe_path)
            .arg("--out")
            .arg(&output)
            .output()
            .unwrap();
        fs::write(
            evidence.join(format!("{name}-cli.log")),
            [result.stdout.as_slice(), result.stderr.as_slice()].concat(),
        )
        .unwrap();
        assert!(
            result.status.success(),
            "grid CLI failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(fs::read(output.join("atlas.png")).unwrap(), sheet_bytes);
        packages.push(output);
    }
    orr_package::Project::open(&fixture.root, compiled_runtime())
        .unwrap()
        .install(&packages)
        .unwrap();
    fs::remove_dir_all(&inputs).unwrap();
    fs::remove_dir_all(&outputs).unwrap();
    assert!(!producer.exists());
    orr_package::Project::open(&fixture.root, compiled_runtime())
        .unwrap()
        .verify()
        .unwrap();
    fs::write(fixture.root.join("arena.scene.yaml"), "schema: orr.scene/1\nsingletons:\n  Score: { kills: [0,0,0,0,0,0,0,0] }\nentities:\n  e_00000001:\n    Position: { pos: [0,0] }\n    PlayerTag: { slot: 0 }\n").unwrap();
    let gpu = Wgpu::headless(WgpuOptions::default()).expect("required Vulkan GPU unavailable");
    let backend = format!("{:?}", gpu.adapter().get_info().backend);
    assert_eq!(backend, "Vulkan");
    let target = OffscreenTarget::new(&gpu, 64, 64, TextureFormat::Rgba8UnormSrgb);
    let mut captures = Vec::new();
    let mut parity_checks = 0;
    for (name, wrong_order) in [("grid-good", false), ("grid-wrong-order", true)] {
        fs::write(fixture.root.join("arena.sprites.json"), serde_json::to_vec(&json!({"version":2,"scene":"arena.scene.yaml","project":".","camera_follow":HERO,
            "bindings":{HERO:{"package":name,"document":"sprites.json","source":{"Locomotion":{"idle":"idle","walk":"walk"}},"units_per_pixel":1.0}}})).unwrap()).unwrap();
        let before = tree(&fixture.root);
        let prepared = PreparedRuntime::open(&fixture.root).unwrap();
        let entity = prepared
            .index()
            .entity(&orr_reflect::Guid::parse(HERO).unwrap())
            .unwrap();
        let mut bridge = prepared.bridge().unwrap();
        let mut control = prepared.bridge().unwrap();
        let (_, mut presentation) = prepared.into_parts().unwrap();
        presentation.update(bridge.snapshot().as_ref()).unwrap();
        let admitted = &presentation.assets()[&(name.into(), "sprites.json".into())];
        assert_eq!(
            (
                admitted.document.atlas().width,
                admitted.document.atlas().height
            ),
            (4, 4)
        );
        assert_eq!(admitted.document.regions().len(), 4);
        assert_eq!(admitted.document.clips().len(), 2);
        let mut compositor =
            ProjectCompositor::new(gpu.clone(), target.format(), presentation.assets()).unwrap();
        for tick in 0..=26u64 {
            let moving = (13..26).contains(&tick);
            if tick > 0 {
                let input =
                    ArenaInput::new(if moving { FP::ONE } else { FP::ZERO }, FP::ZERO, false);
                bridge.set_input(PlayerSlot(0), input).unwrap();
                control.set_input(PlayerSlot(0), input).unwrap();
                bridge.step(1);
                control.step(1);
            }
            let immutable = bridge.host().session().frame().to_bytes();
            let checksum = bridge.host().session().frame().checksum();
            presentation.update(bridge.snapshot().as_ref()).unwrap();
            assert_eq!(presentation.state(HERO).moving, moving);
            let origin = if tick >= 26 {
                26
            } else if moving {
                13
            } else {
                0
            };
            assert_eq!(
                presentation.state(HERO).elapsed_ms,
                (tick - origin) * 1000 / 60
            );
            if (!wrong_order && CAPTURE_TICKS.contains(&tick)) || (wrong_order && tick == 19) {
                let position = bridge
                    .host()
                    .session()
                    .frame()
                    .get::<Position>(entity)
                    .unwrap()
                    .pos;
                let camera = Camera::new([position.x.to_f32(), position.y.to_f32()], 4.);
                assert_eq!(presentation.camera.center, camera.center);
                compositor
                    .draw(
                        target.render_view(),
                        target.size(),
                        &RenderList::new(),
                        presentation.sprites(),
                        &camera,
                    )
                    .unwrap();
                let image = target.read_rgba8();
                let cell = expected_cell(tick, wrong_order);
                // Persist the first failing readback before either pixel verdict.
                let file = format!("{name}-tick-{tick}.png");
                png(&evidence.join(&file), 64, 64, &image);
                captures.push(json!({"file":file,"sha256":hash(&fs::read(evidence.join(&file)).unwrap()),"tick":tick,"cell":cell,"wrong_order":wrong_order}));
                fs::write(evidence.join("partial-receipt.json"), serde_json::to_vec_pretty(&json!({
                    "schema":1,"completed":false,"pixel_verdict_pending":true,
                    "adapter":gpu.adapter_name(),"backend":backend,"producer_sha256":producer_hash,
                    "source_png_sha256":hash(&sheet_bytes),"capture_count":captures.len(),
                    "completed_parity_checks":parity_checks,"captures":captures
                })).unwrap()).unwrap();
                let expected = oracle(cell);
                let first_mismatch = image
                    .iter()
                    .zip(&expected)
                    .position(|(a, b)| a.abs_diff(*b) > 3);
                let max_difference = image
                    .iter()
                    .zip(&expected)
                    .map(|(a, b)| a.abs_diff(*b))
                    .max();
                assert!(
                    matches(&image, &expected),
                    "{name} tick={tick} expected literal cell={cell}; first differing byte={first_mismatch:?}, maximum difference={max_difference:?}"
                );
                if wrong_order {
                    assert!(
                        !matches(&image, &oracle(expected_cell(tick, false))),
                        "wrong-cell negative control did not distinguish oracle"
                    );
                }
            }
            assert_eq!(
                bridge.host().session().frame().to_bytes(),
                immutable,
                "presentation mutated frame"
            );
            assert_eq!(
                bridge.host().session().frame().to_bytes(),
                control.host().session().frame().to_bytes()
            );
            assert_eq!(bridge.host().session().frame().checksum(), checksum);
            assert_eq!(checksum, control.host().session().frame().checksum());
            parity_checks += 1;
        }
        assert_eq!(
            tree(&fixture.root),
            before,
            "runtime changed installed project bytes"
        );
    }
    assert_eq!(captures.len(), 12);
    assert_eq!(parity_checks, 54);
    fs::write(evidence.join("receipt.json"), serde_json::to_vec_pretty(&json!({"schema":1,"test":"grid_cli_installed_locomotion_twelve_capture_pixels_and_frame_parity",
        "adapter":gpu.adapter_name(),"backend":backend,"device_type":format!("{:?}",gpu.adapter().get_info().device_type),"is_software":gpu.is_software(),
        "producer_sha256":producer_hash,"source_png_sha256":hash(&sheet_bytes),"capture_count":12,"parity_checks":54,"source_hidden":true,"installed_bytes_preserved":true,"captures":captures})).unwrap()).unwrap();
    eprintln!("PNG grid: 12 Vulkan captures; 54 authoritative parity checks; source-hidden installed packages; Loop/Once and wrong-cell control");
}
