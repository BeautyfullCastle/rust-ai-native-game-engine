//! Schema-2 common-canvas acceptance through the actual CLI and GPU runtime.
//! The oracle uses only hand-authored source pixels and analytic camera geometry.
#![cfg(all(feature = "project-create", target_os = "linux"))]
#![allow(clippy::float_arithmetic)]

use orr_bridge::Bridge;
use orr_render::orr_rhi::{Rhi, TextureFormat, Wgpu, WgpuOptions};
use orr_render::{Camera, OffscreenTarget, RenderList};
use orr_sample::project_compositor::ProjectCompositor;
use orr_sample::project_runtime::{compiled_runtime, PreparedRuntime};
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

const Z: [u8; 4] = [0, 0, 0, 0];
const R: [u8; 4] = [233, 31, 19, 255];
const G: [u8; 4] = [19, 211, 47, 255];
const B: [u8; 4] = [27, 61, 229, 255];
const P: [u8; 4] = [128, 71, 199, 128];
const H: [u8; 4] = [231, 117, 53, 0];
const FOOT: [u8; 4] = [255, 0, 255, 255];
// Asymmetric moving upper bodies; all feet register at canvas (2,3).
const F0: [[u8; 4]; 9] = [R, P, H, B, G, R, Z, FOOT, B];
const F1: [[u8; 4]; 16] = [G, Z, B, H, P, B, R, G, Z, R, G, P, B, Z, FOOT, R];
const F2: [[u8; 4]; 4] = [B, P, FOOT, H];
const SOURCES: [&[[u8; 4]]; 3] = [&F0, &F1, &F2];
const SIZES: [(usize, usize); 3] = [(3, 3), (4, 4), (2, 2)];
const OFFSETS: [(usize, usize); 3] = [(1, 1), (0, 0), (2, 2)];
const TIMES: [u64; 10] = [0, 99, 100, 249, 250, 449, 450, 550, 900, u64::MAX];
const PACKAGES: [(&str, bool, usize, usize); 4] = [
    ("canvas-loop", false, 0, 0),
    ("canvas-once", true, 0, 0),
    ("canvas-wrong-x", false, 1, 0),
    ("canvas-wrong-y", false, 0, 1),
];
fn write_png(path: &Path, width: u32, height: u32, bytes: &[u8]) {
    let mut encoder = png::Encoder::new(fs::File::create(path).unwrap(), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(bytes)
        .unwrap();
}
fn tree_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
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
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}
fn expected_frame(time: u64, once: bool) -> usize {
    let time = if once { time.min(449) } else { time % 450 };
    if time < 100 {
        0
    } else if time < 250 {
        1
    } else {
        2
    }
}
fn source_pixel(frame: usize, x: usize, y: usize, dx: usize, dy: usize) -> [u8; 4] {
    let (ox, oy) = OFFSETS[frame];
    let (w, h) = SIZES[frame];
    if x >= ox + dx && x < ox + dx + w && y >= oy + dy && y < oy + dy + h {
        SOURCES[frame][(y - oy - dy) * w + x - ox - dx]
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
fn blended(pixel: [u8; 4]) -> [u8; 4] {
    // GPU samples straight RGBA in an sRGB texture, decodes RGB, then applies
    // source-over in linear space over the documented compositor clear color.
    // Re-encoding to sRGB UNORM permits <=3 LSB backend rounding error. Exact
    // straight-alpha and hidden RGB preservation are checked by CPU contracts.
    let a = f64::from(pixel[3]) / 255.;
    let bg = [0.02, 0.02, 0.035];
    [
        srgb(linear(pixel[0]) * a + bg[0] * (1. - a)),
        srgb(linear(pixel[1]) * a + bg[1] * (1. - a)),
        srgb(linear(pixel[2]) * a + bg[2] * (1. - a)),
        255,
    ]
}
fn oracle(frame: usize, dx: usize, dy: usize, transformed: bool) -> Vec<u8> {
    let mut image = Vec::with_capacity(128 * 128 * 4);
    for y in 0..128 {
        for x in 0..128 {
            // Camera half extent 8 on 128 square =>8 screen pixels/world unit.
            // Pixel centers -> world -> inverse 90-degree rotation and 1.5 scale.
            // This uses neither generated atlas regions nor observed GPU pixels.
            let wx = (f64::from(x) + 0.5 - 64.) / 8.;
            let wy = (64. - f64::from(y) - 0.5) / 8.;
            let (lx, ly) = if transformed {
                (wy / 1.5, -wx / 1.5)
            } else {
                (wx, wy)
            };
            let cx = lx + 3.;
            let cy = 2.5 - ly;
            let p = if (0.0..6.0).contains(&cx) && (0.0..5.0).contains(&cy) {
                source_pixel(frame, cx.floor() as usize, cy.floor() as usize, dx, dy)
            } else {
                Z
            };
            image.extend(blended(p));
        }
    }
    image
}
fn matches(actual: &[u8], expected: &[u8]) -> bool {
    actual.len() == expected.len()
        && actual
            .iter()
            .zip(expected)
            .all(|(a, b)| a.abs_diff(*b) <= 3)
}
fn pixel(image: &[u8], x: usize, y: usize) -> &[u8] {
    &image[(y * 128 + x) * 4..(y * 128 + x + 1) * 4]
}

#[test]
fn cli_canvas_sequence_installed_anchor_loop_once_and_wrong_offsets_gpu_readback() {
    let Some(binary) = std::env::var_os("ORR_PNG_SEQUENCE_PACKAGE_BIN") else {
        assert!(
            std::env::var_os("ORR_REQUIRE_GPU").is_none(),
            "required CLI unavailable"
        );
        eprintln!("SKIP: ORR_PNG_SEQUENCE_PACKAGE_BIN not set");
        return;
    };
    let binary = Path::new(&binary).canonicalize().unwrap();
    let fixture = ProjectFixture::new();
    let temporary_evidence = tempfile::tempdir().unwrap();
    let evidence = std::env::var_os("ORR_CANVAS_EVIDENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| temporary_evidence.path().join("captures"));
    fs::create_dir_all(&evidence).unwrap();
    assert!(
        !evidence.join("receipt.json").exists(),
        "refuse stale acceptance receipt"
    );
    let inputs = fixture.root.join("inputs");
    fs::create_dir(&inputs).unwrap();
    let names = ["z-first.png", "a-second.png", "m-third.png"];
    for frame in 0..3 {
        write_png(
            &inputs.join(names[frame]),
            SIZES[frame].0 as u32,
            SIZES[frame].1 as u32,
            &SOURCES[frame].iter().flatten().copied().collect::<Vec<_>>(),
        );
    }
    fs::write(inputs.join("license.txt"), "CC0-1.0 test-owned pixels\n").unwrap();
    let outputs = fixture.root.join("outputs");
    fs::create_dir(&outputs).unwrap();
    let mut sources = Vec::new();
    for (name, once, dx, dy) in PACKAGES {
        let frames: Vec<_> = (0..3)
            .map(|i| {
                serde_json::json!({"png":names[i],"duration_ms":([100,150,200][i]),
            "offset":{"x":OFFSETS[i].0+dx,"y":OFFSETS[i].1+dy}})
            })
            .collect();
        let recipe = serde_json::json!({"schema":2,"name":name,"version":"1.0.0","license":"license.txt",
            "clip":"run","mode":if once {"once"} else {"loop"},"columns":2,
            "canvas":{"width":6,"height":5},"frames":frames});
        let path = inputs.join(format!("{name}.json"));
        fs::write(&path, serde_json::to_vec(&recipe).unwrap()).unwrap();
        let output = outputs.join(name);
        let result = Command::new(&binary)
            .current_dir(&inputs)
            .arg("--recipe")
            .arg(path)
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
            "CLI {name}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        sources.push(output);
    }
    orr_package::Project::open(&fixture.root, compiled_runtime())
        .unwrap()
        .install(&sources)
        .unwrap();
    fs::remove_dir_all(inputs).unwrap();
    fs::remove_dir_all(outputs).unwrap();
    let installed = orr_package::Project::open(&fixture.root, compiled_runtime()).unwrap();
    installed.verify().unwrap();
    for (name, _, _, _) in PACKAGES {
        assert!(installed
            .verify_package(name)
            .unwrap()
            .verified_packages
            .contains_key(name));
    }
    fs::write(fixture.root.join("arena.scene.yaml"),"schema: orr.scene/1\nsingletons:\n  Score: { kills: [0,0,0,0,0,0,0,0] }\nentities:\n  e_00000001:\n    Position: { pos: [0,0] }\n    PlayerTag: { slot: 0 }\n").unwrap();
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
        "acceptance must use actual Vulkan readback"
    );
    eprintln!(
        "PNG canvas adapter: {} backend={backend} software={}",
        gpu.adapter_name(),
        gpu.is_software()
    );
    let target = OffscreenTarget::new(&gpu, 128, 128, TextureFormat::Rgba8UnormSrgb);
    let camera = Camera::new([0., 0.], 8.);
    let mut records = Vec::new();
    for (name, once, dx, dy) in PACKAGES {
        fs::write(fixture.root.join("arena.sprites.json"),serde_json::to_vec(&serde_json::json!({"version":1,"scene":"arena.scene.yaml","project":".","bindings":{"e_00000001":{"package":name,"document":"sprites.json","source":{"Clip":"run"},"units_per_pixel":1.0}}})).unwrap()).unwrap();
        let before = tree_bytes(&fixture.root);
        let prepared = PreparedRuntime::open(&fixture.root).unwrap();
        let initial = prepared.initial_frame().checksum();
        let mut bridge = prepared.bridge().unwrap();
        let (_, mut presentation) = prepared.into_parts().unwrap();
        presentation.update(bridge.snapshot().as_ref()).unwrap();
        assert_eq!(presentation.sprites().len(), 1);
        let admitted = &presentation.assets()[&(name.into(), "sprites.json".into())];
        assert_eq!(
            (
                admitted.document.atlas().width,
                admitted.document.atlas().height
            ),
            (12, 10)
        );
        let binding = &presentation.document().unwrap().bindings["e_00000001"];
        let mut compositor =
            ProjectCompositor::new(gpu.clone(), target.format(), presentation.assets()).unwrap();
        let mut capture_checked = |capture: Vec<u8>,
                                   phase: &str,
                                   value: u64,
                                   frame: usize,
                                   transformed: bool| {
            let expected = oracle(frame, dx, dy, transformed);
            if !matches(&capture, &expected) {
                let first = capture
                    .chunks_exact(4)
                    .zip(expected.chunks_exact(4))
                    .position(|(a, b)| !matches(a, b))
                    .unwrap();
                panic!("{name} {phase} {value} transform={transformed}, pixel ({},{}): actual {:?}, expected {:?}",first%128,first/128,&capture[first*4..first*4+4],&expected[first*4..first*4+4]);
            }
            // The known foot center is screen (60,72), or (76,70) after the
            // explicit rotation/scale. Wrong offsets must visibly move it.
            let (fx, fy) = if transformed { (76, 70) } else { (60, 72) };
            if dx + dy == 0 {
                assert_eq!(pixel(&capture, fx, fy), FOOT);
            } else {
                assert_ne!(pixel(&capture, fx, fy), FOOT);
                assert!(!matches(&capture, &oracle(frame, 0, 0, transformed)));
            }
            let file = format!("{name}-{phase}-{value}-{}.png", u8::from(transformed));
            write_png(&evidence.join(&file), 128, 128, &capture);
            let sha256 = Sha256::digest(fs::read(evidence.join(&file)).unwrap())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            records.push(serde_json::json!({"file":file,"sha256":sha256,"package":name,"phase":phase,"time_or_tick":value,"frame":frame,"transformed":transformed,"wrong_offset_proof":dx+dy>0}));
        };
        for time in TIMES {
            for transformed in [false, true] {
                let mut sprite = presentation.sprites()[0].clone();
                sprite.instance.region = binding.region(&admitted.document, time).unwrap();
                if transformed {
                    sprite.instance.size = [9., 7.5];
                    sprite.instance.rotation = std::f32::consts::FRAC_PI_2;
                }
                compositor
                    .draw(
                        target.render_view(),
                        target.size(),
                        &RenderList::new(),
                        &[sprite],
                        &camera,
                    )
                    .unwrap();
                capture_checked(
                    target.read_rgba8(),
                    "direct",
                    time,
                    expected_frame(time, once),
                    transformed,
                );
            }
        }
        assert_eq!(
            bridge.host().session().frame().checksum(),
            initial,
            "direct rendering mutated simulation"
        );
        // Every intermediate snapshot AND capture is required; skipping >8 ticks
        // intentionally resets the production presenter's animation cursor.
        let mut identities: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
        for tick in 0..=54 {
            if tick > 0 {
                bridge.step(1);
            }
            let checksum = bridge.host().session().frame().checksum();
            presentation.update(bridge.snapshot().as_ref()).unwrap();
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
            let frame = expected_frame(tick * 1000 / 60, once);
            if let Some(previous) = identities.get(&frame) {
                assert_eq!(previous, &capture, "frame changed on repeat/loop");
            } else {
                for other in identities.values() {
                    assert_ne!(
                        other, &capture,
                        "distinct animation frames must visibly differ"
                    );
                }
                identities.insert(frame, capture.clone());
            }
            capture_checked(capture, "host", tick, frame, false);
            assert_eq!(
                bridge.host().session().frame().checksum(),
                checksum,
                "GPU/presentation mutated PlayHost at {tick}"
            );
        }
        assert_eq!(identities.len(), 3);
        assert_eq!(
            tree_bytes(&fixture.root),
            before,
            "installed/project bytes changed"
        );
    }
    assert_eq!(records.len(), 300);
    fs::write(evidence.join("receipt.json"),serde_json::to_vec_pretty(&serde_json::json!({"schema":1,
        "test":"cli_canvas_sequence_installed_anchor_loop_once_and_wrong_offsets_gpu_readback",
        "adapter":gpu.adapter_name(),"backend":backend,"capture_count":records.len(),"captures":records})).unwrap()).unwrap();
    eprintln!("PNG canvas sequence: 300 production GPU captures; source-hidden installed packages; stationary feet; Loop/Once; wrong-X/wrong-Y; every PlayHost tick; rotated/scaled; immutable bytes");
}
