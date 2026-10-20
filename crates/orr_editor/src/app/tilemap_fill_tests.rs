//! Actual AccessKit widgets and egui pointer events, not native desktop input.
//! This explicit software-GPU gate retains artifacts before semantic assertions.
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use super::*;
use egui_kittest::{Harness, kittest::Queryable};
use egui_wgpu::wgpu;
use orr_bridge::Bridge;
use orr_render::{
    Camera, OffscreenTarget,
    orr_rhi::{TextureFormat, Wgpu},
};
use orr_sample::{project_compositor::ProjectCompositor, project_runtime::PreparedRuntime};
use serde_json::json;
use std::{
    fs,
    path::Path,
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};

const SOURCE: &[u8] = include_bytes!("../../../../assets/sprite_demo/lantern_keeper.rgba");
const BEFORE: [Option<u32>; 42] = [
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    None,
    None,
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    None,
    None,
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    None,
    Some(20),
    None,
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    None,
];
// Hand-written result, never computed with the production fill/neighbour helper.
const FILLED: [Option<u32>; 42] = [
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(11),
    Some(11),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(11),
    Some(11),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(11),
    Some(20),
    None,
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    Some(20),
    None,
];
fn fingerprint(bytes: &[u8]) -> String {
    format!(
        "{:016x}",
        bytes
            .iter()
            .fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(*b))
                .wrapping_mul(0x100000001b3))
    )
}
fn write_json(path: &Path, value: &serde_json::Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}
fn png(path: &Path, size: (u32, u32), bytes: &[u8]) {
    let mut encoder = png::Encoder::new(fs::File::create(path).unwrap(), size.0, size.1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(bytes)
        .unwrap();
}
fn drain(h: &mut Harness<'_, EditorApp>, count: usize) {
    for _ in 0..count {
        h.run_steps(1);
        // Poll the real renderer every UI step, not hundreds of queued submits.
        h.render().expect("per-step actual app GPU drain");
    }
}
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).click();
    drain(h, 3);
}
fn open(root: &Path) -> Harness<'static, EditorApp> {
    let prepared = crate::project::PreparedProject::open(root).unwrap();
    let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    setup.native_adapter_selector = Some(Arc::new(|adapters, _| {
        adapters
            .iter()
            .find(|a| {
                let info = a.get_info();
                info.backend == wgpu::Backend::Vulkan && info.device_type == wgpu::DeviceType::Cpu
            })
            .cloned()
            .ok_or_else(|| "mandatory software Vulkan adapter unavailable".into())
    }));
    let mut h = Harness::builder()
        .with_size([1800., 2000.])
        .wgpu_setup(setup.into())
        .build_eframe(move |cc| {
            let mut editor = Editor::start(&prepared.host_spec()).unwrap();
            editor.sync();
            editor.camera = Camera::new([0., 80.], 160.);
            prepared
                .into_app(editor, cc.wgpu_render_state.clone(), &cc.egui_ctx)
                .unwrap()
        });
    drain(&mut h, 5);
    h
}
fn point(h: &Harness<'_, EditorApp>, column: u32, row: u32) -> egui::Pos2 {
    let app = h.state();
    let rect = app.ui.viewport_rect.unwrap();
    let px = app.ui.viewport_px;
    let screen = app
        .editor
        .camera
        .world_to_screen([-48. + column as f32 * 16., 136. - row as f32 * 16.], px);
    let p = rect.min
        + egui::vec2(
            screen[0] * rect.width() / px.0 as f32,
            screen[1] * rect.height() / px.1 as f32,
        );
    assert!(rect.contains(p));
    p
}
fn button(h: &mut Harness<'_, EditorApp>, p: egui::Pos2, pressed: bool) {
    h.event(egui::Event::PointerMoved(p));
    h.event(egui::Event::PointerButton {
        pos: p,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    });
    drain(h, 2);
}
fn cells<'a>(h: &'a Harness<'_, EditorApp>) -> &'a [Option<u32>] {
    &h.state()
        .tilemap
        .as_ref()
        .unwrap()
        .document()
        .document()
        .cells
}
fn authoritative_frame(h: &mut Harness<'_, EditorApp>) -> orr_ecs::Frame {
    h.state_mut().editor.sync();
    let snapshot = h.state().editor.snapshot().unwrap();
    let tick = snapshot.predicted().tick();
    let checksum = snapshot.predicted().checksum();
    let client = h
        .state_mut()
        .editor
        .agent_client("tile-fill-authority-probe")
        .unwrap();
    client.local_frames.clear();
    client
        .call(
            "watch.subscribe",
            json!({"topics":["frames"],"source":"view","max_fps":60}),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        client.poll().unwrap();
        while let Some(received) = client.local_frames.pop_front() {
            if received.frame.tick() == tick && received.frame.checksum() == checksum {
                return (*received.frame).clone();
            }
        }
        assert!(
            Instant::now() < deadline,
            "same-tick authoritative Frame deadline"
        );
        std::thread::yield_now();
    }
}
fn capture(
    h: &mut Harness<'_, EditorApp>,
    dir: &Path,
    name: &str,
    captures: &mut Vec<serde_json::Value>,
) -> orr_ecs::Frame {
    let composed = h.render().expect("actual App readback");
    png(
        &dir.join(format!("{name}-app.png")),
        composed.dimensions(),
        composed.as_raw(),
    );
    let gpu = h.state().viewport_gpu().unwrap().gpu();
    let size = gpu.target().size();
    let pixels = gpu.read_rgba8();
    fs::write(dir.join(format!("{name}.rgba")), &pixels).unwrap();
    png(&dir.join(format!("{name}.png")), size, &pixels);
    let frame = authoritative_frame(h);
    let raw_frame = frame.to_bytes();
    fs::write(dir.join(format!("{name}.frame")), &raw_frame).unwrap();
    let info = h.state().render_state.as_ref().unwrap().adapter.get_info();
    captures.push(json!({"name":name,"files":[format!("{name}.rgba"),format!("{name}.png"),format!("{name}.frame"),format!("{name}-app.png")],"scope":"actual App viewport; authoring grid, no pixel verdict","size":size,"rgba_fnv1a64":fingerprint(&pixels),"frame_fnv1a64":fingerprint(&raw_frame),"frame_source":"watch.subscribe source=view matched to predicted tick/checksum","tick":frame.tick(),"checksum":frame.checksum(),"adapter":info.name,"backend":format!("{:?}",info.backend),"device_type":format!("{:?}",info.device_type),"cells":cells(h)}));
    write_json(
        &dir.join("partial-receipt.json"),
        &json!({"completed":false,"captures":captures,"native_os_input":false}),
    );
    assert_eq!(info.backend, wgpu::Backend::Vulkan);
    assert_eq!(info.device_type, wgpu::DeviceType::Cpu);
    frame
}
fn linear(v: u8) -> f64 {
    let v = f64::from(v) / 255.;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
fn srgb(v: f64) -> u8 {
    ((if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * v.powf(1. / 2.4) - 0.055
    }) * 255.)
        .round() as u8
}
// Independent original atlas offsets and literal map geometry. No production
// UV, placement, world-to-cell, fill, or draw extraction helper is consulted.
fn expected(region: Option<u32>, x: usize, y: usize) -> [u8; 4] {
    let mut out = [0.02, 0.02, 0.035];
    if let Some(region) = region {
        let offset = match region {
            10 => 0,
            11 => 16,
            20 => 32,
            _ => panic!("fixture region"),
        };
        let i = (y * 64 + offset + x) * 4;
        let alpha = f64::from(SOURCE[i + 3]) / 255.;
        for c in 0..3 {
            out[c] = linear(SOURCE[i + c]) * alpha + out[c] * (1. - alpha);
        }
    }
    [srgb(out[0]), srgb(out[1]), srgb(out[2]), 255]
}
fn consume_saved(
    root: &Path,
    dir: &Path,
    gpu: &Wgpu,
    name: &str,
    literal: &[Option<u32>; 42],
    captures: &mut Vec<serde_json::Value>,
) -> (Vec<u8>, Vec<u8>) {
    let prepared = PreparedRuntime::open(root).unwrap();
    let mut host = prepared.bridge().unwrap();
    let (_, mut presentation) = prepared.into_parts().unwrap();
    host.step(1);
    presentation.update(host.snapshot().as_ref()).unwrap();
    let frame = host.host().session().frame().to_bytes();
    let target = OffscreenTarget::new(gpu, 224, 224, TextureFormat::Rgba8UnormSrgb);
    let mut compositor =
        ProjectCompositor::new(gpu.clone(), target.format(), presentation.assets()).unwrap();
    compositor
        .draw_with_background(
            target.render_view(),
            target.size(),
            presentation.shapes(),
            presentation.background(),
            presentation.sprites(),
            &Camera::new([0., 80.], 112.),
        )
        .unwrap();
    let pixels = target.read_rgba8();
    fs::write(dir.join(format!("{name}.rgba")), &pixels).unwrap();
    fs::write(dir.join(format!("{name}.frame")), &frame).unwrap();
    png(&dir.join(format!("{name}.png")), (224, 224), &pixels);
    captures.push(json!({"name":name,"files":[format!("{name}.rgba"),format!("{name}.png"),format!("{name}.frame")],"scope":"actual saved PreparedRuntime PlayHost production compositor","size":[224,224],"rgba_fnv1a64":fingerprint(&pixels),"frame_fnv1a64":fingerprint(&frame),"tick":host.host().session().frame().tick(),"checksum":host.host().session().frame().checksum(),"pixel_verdict_pending":true}));
    write_json(
        &dir.join("partial-receipt.json"),
        &json!({"completed":false,"captures":captures,"native_os_input":false}),
    );
    // Tile rectangle [56,168)×[48,144) is above all actor geometry, including
    // orientation marks and collider AA. Every pixel in all 42 cells is checked.
    for row in 0..6 {
        for col in 0..7 {
            for y in 0..16 {
                for x in 0..16 {
                    let i = ((48 + row * 16 + y) * 224 + 56 + col * 16 + x) * 4;
                    let want = expected(literal[row * 7 + col], x, y);
                    assert!(
                        pixels[i..i + 4]
                            .iter()
                            .zip(want)
                            .all(|(a, b)| a.abs_diff(b) <= 3),
                        "{name} literal cell ({col},{row}) original texel ({x},{y}) mismatch"
                    );
                }
            }
        }
    }
    captures.last_mut().unwrap()["pixel_verdict_pending"] = json!(false);
    captures.last_mut().unwrap()["oracle_pixels_checked"] = json!(10752);
    assert_eq!(
        host.host().session().frame().to_bytes(),
        frame,
        "presentation must not mutate Frame"
    );
    (pixels, frame)
}

#[test]
#[ignore = "mandatory explicit actual EditorApp connected Fill software Vulkan acceptance"]
fn actual_cli_fill_save_reopen_play_literal_cells() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let evidence = PathBuf::from(
        std::env::var_os("ORR_TILEMAP_FILL_EVIDENCE")
            .expect("retained Fill evidence directory required"),
    );
    let cli = PathBuf::from(
        std::env::var_os("ORR_NEW_ARENA_BIN")
            .expect("verified tilemap-enabled creator CLI required"),
    );
    assert!(evidence.is_absolute() && evidence.is_dir());
    assert!(cli.is_absolute() && cli.is_file());
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("created");
    let output = Command::new(&cli)
        .args([
            "--template",
            "arena-tilemap-2d-v1",
            "--seed",
            "fill-acceptance",
            "--output",
        ])
        .arg(&root)
        .output()
        .unwrap();
    fs::write(
        evidence.join("creator.log"),
        [output.stdout.as_slice(), output.stderr.as_slice()].concat(),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "actual tilemap CLI creation failed"
    );
    let mut map: orr_tilemap::Document =
        serde_json::from_slice(&fs::read(root.join("arena.tilemap.json")).unwrap()).unwrap();
    // Fixture setup edits only the existing CLI-created sidecar, before admission.
    map.width = 7;
    map.height = 6;
    map.origin = [-56., 144.];
    map.cells = BEFORE.to_vec();
    fs::write(
        root.join("arena.tilemap.json"),
        serde_json::to_vec(&map).unwrap(),
    )
    .unwrap();
    let scene = fs::read(root.join("arena.scene.yaml")).unwrap();
    let lock = fs::read(root.join("orr.packages.lock.json")).unwrap();
    let manifest = fs::read(root.join("orr.project.json")).unwrap();
    let mut captures = Vec::new();
    let mut h = open(&root);
    let initial = capture(&mut h, &evidence, "before", &mut captures).to_bytes();
    click(&mut h, "Tile tool (owns viewport clicks)");
    click(&mut h, "Fill");
    click(&mut h, "Static region 11");
    let inside = point(&h, 1, 1);
    // An outside-origin press entering the map cannot initiate a fill.
    button(&mut h, egui::pos2(0., 0.), true);
    h.event(egui::Event::PointerMoved(inside));
    drain(&mut h, 2);
    button(&mut h, inside, false);
    capture(&mut h, &evidence, "outside-origin", &mut captures);
    assert_eq!(cells(&h), BEFORE);
    button(&mut h, inside, true);
    // Held drag over a diagonally touching hole, an outside island, and barrier
    // must not trigger another connected operation at any of those cells.
    for (col, row) in [(4, 3), (6, 5), (0, 0), (1, 1)] {
        let p = point(&h, col, row);
        h.event(egui::Event::PointerMoved(p));
        drain(&mut h, 3);
    }
    button(&mut h, inside, false);
    let dirty = capture(&mut h, &evidence, "filled-held-drag", &mut captures);
    assert_eq!(cells(&h), FILLED);
    assert_eq!(dirty.to_bytes(), initial);
    assert!(h.state().editor.selected_guids().is_empty());
    assert!(h.state().ui.body_drag.is_none());
    // Same-region fill must not insert another history entry.
    button(&mut h, inside, true);
    button(&mut h, inside, false);
    click(&mut h, "Undo tile");
    let undo = capture(&mut h, &evidence, "undo", &mut captures);
    assert_eq!(cells(&h), BEFORE);
    assert!(!h.state().tilemap.as_ref().unwrap().dirty());
    assert_eq!(undo.to_bytes(), initial);
    click(&mut h, "Redo tile");
    capture(&mut h, &evidence, "redo", &mut captures);
    assert_eq!(cells(&h), FILLED);
    click(&mut h, "Save tilemap");
    let saved = capture(&mut h, &evidence, "saved", &mut captures);
    let saved_bytes = fs::read(root.join("arena.tilemap.json")).unwrap();
    fs::write(evidence.join("saved.tilemap.json"), &saved_bytes).unwrap();
    let saved_map: orr_tilemap::Document = serde_json::from_slice(&saved_bytes).unwrap();
    assert_eq!(saved_map.cells, FILLED);
    assert!(!h.state().tilemap.as_ref().unwrap().dirty());
    assert_eq!(saved.to_bytes(), initial);
    drop(h);
    let mut reopened = open(&root);
    let fresh = capture(&mut reopened, &evidence, "fresh-reopen", &mut captures);
    assert_eq!(cells(&reopened), FILLED);
    assert_eq!(fresh.to_bytes(), initial);
    // Actual Step widget starts paused Play and advances exactly one tick.
    click(&mut reopened, LBL_STEP);
    let play = capture(&mut reopened, &evidence, "play", &mut captures);
    assert_eq!(reopened.state().editor.mode(), Mode::Play);
    assert_eq!(play.tick(), 1);
    assert_eq!(cells(&reopened), FILLED);
    let state = reopened.state().render_state.as_ref().unwrap();
    let gpu = Wgpu::from_parts(
        state.instance.clone(),
        state.adapter.clone(),
        state.device.clone(),
        state.queue.clone(),
    );
    let (good_pixels, good_frame) = consume_saved(
        &root,
        &evidence,
        &gpu,
        "runtime-saved",
        &FILLED,
        &mut captures,
    );
    assert_eq!(
        play.to_bytes(),
        good_frame,
        "actual App and saved runtime same-tick Frame parity"
    );
    drop(reopened);
    // Independent wrong-region control, after all actual App lifecycle steps.
    // This changes a visibly exposed filled cell, not a texture hidden by actors.
    let mut wrong = FILLED;
    wrong[8] = Some(10);
    map.cells = wrong.to_vec();
    fs::write(
        root.join("arena.tilemap.json"),
        serde_json::to_vec(&map).unwrap(),
    )
    .unwrap();
    fs::write(
        evidence.join("wrong-region.tilemap.json"),
        serde_json::to_vec(&map).unwrap(),
    )
    .unwrap();
    let (wrong_pixels, wrong_frame) = consume_saved(
        &root,
        &evidence,
        &gpu,
        "runtime-wrong-region",
        &wrong,
        &mut captures,
    );
    assert_eq!(
        wrong_frame, good_frame,
        "tile presentation cannot change simulation Frame"
    );
    let changed = (64..80)
        .flat_map(|y| (72..88).map(move |x| (y * 224 + x) * 4))
        .filter(|i| {
            wrong_pixels[*i..*i + 4]
                .iter()
                .zip(&good_pixels[*i..*i + 4])
                .any(|(a, b)| a.abs_diff(*b) > 3)
        })
        .count();
    assert!(
        changed >= 8,
        "wrong-region control must visibly differ in the exposed filled cell"
    );
    assert_eq!(fs::read(root.join("arena.scene.yaml")).unwrap(), scene);
    assert_eq!(fs::read(root.join("orr.packages.lock.json")).unwrap(), lock);
    assert_eq!(fs::read(root.join("orr.project.json")).unwrap(), manifest);
    write_json(
        &evidence.join("receipt.json"),
        &json!({"completed":true,"captures":captures,"literal_cells":&FILLED[..],"oracle_pixels_per_runtime_capture":10752,"wrong_region_visible_pixels":changed,"frame_parity":"authoring unchanged; actual App Play and saved runtime tick 1; wrong-region control same Frame bytes","creator_fnv1a64":fingerprint(&fs::read(cli).unwrap()),"test_source_fnv1a64":fingerprint(include_bytes!("tilemap_fill_tests.rs")),"panel_source_fnv1a64":fingerprint(include_bytes!("../tilemap_panel.rs")),"panel_fill_source_fnv1a64":fingerprint(include_bytes!("../tilemap_panel/fill.rs")),"core_source_fnv1a64":fingerprint(include_bytes!("../../../orr_tilemap/src/lib.rs")),"source_rgba_fnv1a64":fingerprint(SOURCE),"native_os_input":false,"export":false,"windows":false}),
    );
}
