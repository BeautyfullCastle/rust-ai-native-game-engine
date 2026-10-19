//! Schema-3 CLI acceptance: one installed document feeds real Arena locomotion.
//! Pixel expectations come only from source PNG literals and analytic geometry.
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
const Z: [u8; 4] = [0, 0, 0, 0];
const R: [u8; 4] = [233, 31, 19, 255];
const G: [u8; 4] = [19, 211, 47, 255];
const B: [u8; 4] = [27, 61, 229, 255];
const P: [u8; 4] = [128, 71, 199, 128];
const H: [u8; 4] = [231, 117, 53, 0];
const FOOT: [u8; 4] = [255, 0, 255, 255];
// Three differently sized PNGs share the foot at canvas (2, 3).
const F0: [[u8; 4]; 9] = [R, P, H, B, G, R, Z, FOOT, B];
const F1: [[u8; 4]; 16] = [G, Z, B, H, P, B, R, G, Z, R, G, P, B, Z, FOOT, R];
const F2: [[u8; 4]; 4] = [B, P, FOOT, H];
const SOURCES: [&[[u8; 4]]; 3] = [&F0, &F1, &F2];
const SIZES: [(usize, usize); 3] = [(3, 3), (4, 4), (2, 2)];
const OFFSETS: [(usize, usize); 3] = [(1, 1), (0, 0), (2, 2)];
const TIMES: [u64; 15] = [
    0,
    69,
    70,
    99,
    100,
    169,
    170,
    199,
    200,
    249,
    250,
    299,
    300,
    400,
    u64::MAX,
];
const PACKAGES: [&str; 4] = [
    "clips-good",
    "clips-wrong-order",
    "clips-wrong-offset",
    "clips-wrong-duration",
];

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
fn bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, at: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, out);
            } else {
                out.insert(
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
fn source_frame(time: u64, moving: bool, variant: usize) -> usize {
    if !moving {
        return if time % 250 < 100 { 0 } else { 2 };
    }
    let first_end = if variant == 3 { 170 } else { 70 };
    let index = if time < first_end {
        0
    } else if time < 200 {
        1
    } else {
        2
    };
    if variant == 1 {
        [2, 1, 1][index]
    } else {
        [1, 2, 1][index]
    }
}
fn source_pixel(frame: usize, x: usize, y: usize, dx: usize) -> [u8; 4] {
    let (ox, oy) = OFFSETS[frame];
    let (w, h) = SIZES[frame];
    if x >= ox + dx && x < ox + dx + w && y >= oy && y < oy + h {
        SOURCES[frame][(y - oy) * w + x - ox - dx]
    } else {
        Z
    }
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
fn oracle(frame: usize, dx: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(128 * 128 * 4);
    for y in 0..128 {
        for x in 0..128 {
            // Camera half extent 8: eight framebuffer pixels per world unit.
            let cx = (f64::from(x) + 0.5 - 64.) / 8. + 3.;
            let cy = (f64::from(y) + 0.5 - 64.) / 8. + 2.5;
            let p = if (0.0..6.0).contains(&cx) && (0.0..5.0).contains(&cy) {
                source_pixel(frame, cx.floor() as usize, cy.floor() as usize, dx)
            } else {
                Z
            };
            let a = f64::from(p[3]) / 255.;
            out.extend([
                srgb(linear(p[0]) * a + 0.02 * (1. - a)),
                srgb(linear(p[1]) * a + 0.02 * (1. - a)),
                srgb(linear(p[2]) * a + 0.035 * (1. - a)),
                255,
            ]);
        }
    }
    out
}
fn matches(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.abs_diff(*b) <= 3)
}
fn assert_pixels(actual: &[u8], expected: &[u8], label: &str) {
    if !matches(actual, expected) {
        let first = actual
            .chunks_exact(4)
            .zip(expected.chunks_exact(4))
            .position(|(a, b)| !matches(a, b))
            .unwrap();
        panic!(
            "{label}, pixel ({},{}): actual {:?}, expected {:?}",
            first % 128,
            first / 128,
            &actual[first * 4..first * 4 + 4],
            &expected[first * 4..first * 4 + 4]
        );
    }
}
fn record(evidence: &Path, records: &mut Vec<serde_json::Value>, name: &str, image: &[u8]) {
    let file = format!("{name}.png");
    png(&evidence.join(&file), 128, 128, image);
    let hash = Sha256::digest(fs::read(evidence.join(&file)).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    records.push(json!({"file":file,"sha256":hash}));
}

#[test]
fn cli_named_clips_same_document_locomotion_pixels_and_authoritative_parity() {
    let Some(binary) = std::env::var_os("ORR_PNG_SEQUENCE_PACKAGE_BIN") else {
        assert!(
            std::env::var_os("ORR_REQUIRE_GPU").is_none(),
            "required PNG producer CLI missing"
        );
        eprintln!("SKIP: ORR_PNG_SEQUENCE_PACKAGE_BIN not set");
        return;
    };
    let fixture = ProjectFixture::new();
    let temp = tempfile::tempdir().unwrap();
    let evidence = std::env::var_os("ORR_PNG_CLIPS_EVIDENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| temp.path().join("evidence"))
        .join("sample");
    fs::create_dir_all(&evidence).unwrap();
    assert!(
        !evidence.join("receipt.json").exists(),
        "refuse stale receipt"
    );
    let inputs = fixture.root.join("private-authoring");
    fs::create_dir(&inputs).unwrap();
    // The only executable invoked by this test is removed with every private input.
    let producer = inputs.join("producer");
    fs::copy(Path::new(&binary).canonicalize().unwrap(), &producer).unwrap();
    let names = ["z-first.png", "a-second.png", "m-shared.png"];
    for i in 0..3 {
        png(
            &inputs.join(names[i]),
            SIZES[i].0 as u32,
            SIZES[i].1 as u32,
            &SOURCES[i].iter().flatten().copied().collect::<Vec<_>>(),
        );
    }
    fs::write(
        inputs.join("license.txt"),
        "CC0-1.0 test-owned source pixels\n",
    )
    .unwrap();
    let outputs = fixture.root.join("generated-packages");
    fs::create_dir(&outputs).unwrap();
    let mut packages = Vec::new();
    for (variant, name) in PACKAGES.into_iter().enumerate() {
        let dx = usize::from(variant == 2);
        let order = if variant == 1 { [2, 1, 1] } else { [1, 2, 1] };
        let durations = if variant == 3 {
            [170, 30, 100]
        } else {
            [70, 130, 100]
        };
        let recipe = json!({"schema":3,"name":name,"version":"1.0.0","license":"license.txt","columns":2,
            "canvas":{"width":6,"height":5},
            "frames":(0..3).map(|i|json!({"png":names[i],"offset":{"x":OFFSETS[i].0+dx,"y":OFFSETS[i].1}})).collect::<Vec<_>>(),
            "clips":[{"id":"idle","mode":"loop","frames":[{"frame":0,"duration_ms":100},{"frame":2,"duration_ms":150}]},
                {"id":"walk","mode":"once","frames":(0..3).map(|i|json!({"frame":order[i],"duration_ms":durations[i]})).collect::<Vec<_>>()}]});
        let recipe_path = inputs.join(format!("{name}.json"));
        fs::write(&recipe_path, serde_json::to_vec(&recipe).unwrap()).unwrap();
        let output = outputs.join(name);
        let result = Command::new(&producer)
            .current_dir(&inputs)
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
            "{name}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        packages.push(output);
    }
    orr_package::Project::open(&fixture.root, compiled_runtime())
        .unwrap()
        .install(&packages)
        .unwrap();
    fs::remove_dir_all(&inputs).unwrap();
    fs::remove_dir_all(&outputs).unwrap();
    assert!(!producer.exists());
    let installed = orr_package::Project::open(&fixture.root, compiled_runtime()).unwrap();
    installed.verify().unwrap();
    for name in PACKAGES {
        assert!(installed
            .verify_package(name)
            .unwrap()
            .verified_packages
            .contains_key(name));
    }
    fs::write(fixture.root.join("arena.scene.yaml"), "schema: orr.scene/1\nsingletons:\n  Score: { kills: [0,0,0,0,0,0,0,0] }\nentities:\n  e_00000001:\n    Position: { pos: [0,0] }\n    PlayerTag: { slot: 0 }\n").unwrap();
    let gpu = match Wgpu::headless(WgpuOptions::default()) {
        Ok(gpu) => gpu,
        Err(error) => {
            assert!(
                std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "required GPU: {error}"
            );
            eprintln!("SKIP: {error}");
            return;
        }
    };
    let backend = format!("{:?}", gpu.adapter().get_info().backend);
    assert_eq!(
        backend, "Vulkan",
        "acceptance requires actual Vulkan readback"
    );
    let target = OffscreenTarget::new(&gpu, 128, 128, TextureFormat::Rgba8UnormSrgb);
    let mut records = Vec::new();
    let mut negative_records = Vec::new();
    for (case, variant, wrong_mapping) in [
        ("good", 0, false),
        ("wrong-mapping", 0, true),
        ("wrong-order", 1, false),
        ("wrong-offset", 2, false),
        ("wrong-duration", 3, false),
    ] {
        let name = PACKAGES[variant];
        let (idle, walk) = if wrong_mapping {
            ("walk", "idle")
        } else {
            ("idle", "walk")
        };
        fs::write(fixture.root.join("arena.sprites.json"),serde_json::to_vec(&json!({"version":2,"scene":"arena.scene.yaml","project":".","camera_follow":HERO,
            "bindings":{HERO:{"package":name,"document":"sprites.json","source":{"Locomotion":{"idle":idle,"walk":walk}},"units_per_pixel":1.0}}})).unwrap()).unwrap();
        let before = bytes(&fixture.root);
        let prepared = PreparedRuntime::open(&fixture.root).unwrap();
        let entity = prepared
            .index()
            .entity(&orr_reflect::Guid::parse(HERO).unwrap())
            .unwrap();
        let mut bridge = prepared.bridge().unwrap();
        // An independent authoritative PlayHost receives the same inputs and is
        // never passed to a presenter, compositor, or GUI.
        let mut control = prepared.bridge().unwrap();
        let (_, mut presentation) = prepared.into_parts().unwrap();
        presentation.update(bridge.snapshot().as_ref()).unwrap();
        assert_eq!(presentation.assets().len(), 1);
        let admitted = &presentation.assets()[&(name.into(), "sprites.json".into())];
        assert_eq!(admitted.document.clips().len(), 2);
        assert_eq!(
            admitted.document.regions().len(),
            3,
            "shared/repeated refs must not repack source images"
        );
        assert_eq!(
            (
                admitted.document.atlas().width,
                admitted.document.atlas().height
            ),
            (12, 10)
        );
        let binding = presentation.document().unwrap().bindings[HERO].clone();
        let doc = admitted.document.clone();
        let mut compositor =
            ProjectCompositor::new(gpu.clone(), target.format(), presentation.assets()).unwrap();
        let dx = usize::from(variant == 2);
        let mut failures_against_good = 0;
        for moving in [false, true] {
            for time in TIMES {
                let mut sprite = presentation.sprites()[0].clone();
                sprite.instance.region = binding.region_for_motion(&doc, time, moving).unwrap();
                compositor
                    .draw(
                        target.render_view(),
                        target.size(),
                        &RenderList::new(),
                        &[sprite],
                        &Camera::new([0., 0.], 8.),
                    )
                    .unwrap();
                let capture = target.read_rgba8();
                let expected = source_frame(time, moving ^ wrong_mapping, variant);
                assert_pixels(
                    &capture,
                    &oracle(expected, dx),
                    &format!("{case} direct moving={moving} time={time}"),
                );
                failures_against_good += usize::from(!matches(
                    &capture,
                    &oracle(source_frame(time, moving, 0), 0),
                ));
                record(
                    &evidence,
                    &mut records,
                    &format!("{case}-direct-{}-{time}", u8::from(moving)),
                    &capture,
                );
            }
        }
        assert_eq!(
            bridge.host().session().frame().to_bytes(),
            control.host().session().frame().to_bytes()
        );
        let mut host_failures = 0;
        for tick in 0..=104u64 {
            let moving = (21..75).contains(&tick);
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
            let time = (tick
                - if moving {
                    21
                } else if tick >= 75 {
                    75
                } else {
                    0
                })
                * 1000
                / 60;
            assert_eq!(presentation.state(HERO).moving, moving);
            assert_eq!(presentation.state(HERO).elapsed_ms, time);
            let position = bridge
                .host()
                .session()
                .frame()
                .get::<Position>(entity)
                .unwrap()
                .pos;
            let camera = Camera::new([position.x.to_f32(), position.y.to_f32()], 8.);
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
            let capture = target.read_rgba8();
            assert_pixels(
                &capture,
                &oracle(source_frame(time, moving ^ wrong_mapping, variant), dx),
                &format!("{case} production PlayHost tick={tick}"),
            );
            host_failures += usize::from(!matches(
                &capture,
                &oracle(source_frame(time, moving, 0), 0),
            ));
            record(
                &evidence,
                &mut records,
                &format!("{case}-host-{tick}"),
                &capture,
            );
            assert_eq!(
                bridge.host().session().frame().to_bytes(),
                immutable,
                "presentation mutated authoritative state"
            );
            assert_eq!(
                bridge.host().session().frame().to_bytes(),
                control.host().session().frame().to_bytes(),
                "presentation-free state mismatch at {tick}"
            );
            assert_eq!(
                checksum,
                control.host().session().frame().checksum(),
                "presentation-free checksum mismatch at {tick}"
            );
        }
        if case == "good" {
            assert_eq!((failures_against_good, host_failures), (0, 0));
        } else {
            assert!(failures_against_good>0 && host_failures>0,"{case} must fail the correct source-pixel oracle in direct AND real locomotion paths");
        }
        negative_records.push(json!({"case":case,"direct_rejected":failures_against_good,"host_rejected":host_failures}));
        assert_eq!(
            bytes(&fixture.root),
            before,
            "admission/playback changed installed bytes"
        );
    }
    assert_eq!(records.len(), 675);
    fs::write(evidence.join("receipt.json"),serde_json::to_vec_pretty(&json!({"schema":1,"test":"cli_named_clips_same_document_locomotion_pixels_and_authoritative_parity",
        "adapter":gpu.adapter_name(),"backend":backend,"device_type":format!("{:?}",gpu.adapter().get_info().device_type),"is_software":gpu.is_software(),"installed_bytes_preserved":true,"capture_count":records.len(),"captures":records,"negative_controls":negative_records,
        "source_hidden":true,"same_document_locomotion":true,"every_host_tick_state_and_checksum_equal":true})).unwrap()).unwrap();
    eprintln!("PNG named clips: 675 Vulkan captures, same-document real locomotion, Loop wrap/Once end, mapping/order/offset/duration negatives, every-tick authoritative state/checksum parity");
}
