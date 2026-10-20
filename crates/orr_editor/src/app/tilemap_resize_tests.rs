//! Actual AccessKit widgets and egui pointer events, not native desktop input.
//! This explicit software-GPU gate retains artifacts before semantic assertions.
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use super::*;
use egui_kittest::{kittest::Queryable, Harness};
use egui_wgpu::wgpu;
use orr_bridge::Bridge;
use orr_render::{
    orr_rhi::{TextureFormat, Wgpu},
    Camera, OffscreenTarget,
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
// Literal independent asymmetric fixture and results; never production resize output.
const BEFORE: [Option<u32>; 6] = [Some(10), None, Some(20), Some(11), Some(20), None];
const GROWN: [Option<u32>; 15] = [
    Some(10),
    None,
    Some(20),
    None,
    None,
    Some(11),
    Some(20),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
];
const PAINTED: [Option<u32>; 15] = [
    Some(10),
    None,
    Some(20),
    None,
    None,
    Some(11),
    Some(20),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some(11),
];
const TRIMMED: [Option<u32>; 8] = [
    Some(10),
    None,
    Some(20),
    None,
    Some(11),
    Some(20),
    None,
    None,
];
// Deliberately flattened-prefix bug (3x2 cells copied directly into 4x2).
const WRONG_STRIDE: [Option<u32>; 8] = [
    Some(10),
    None,
    Some(20),
    Some(11),
    Some(20),
    None,
    None,
    None,
];
const WRONG_EXPOSED: [Option<u32>; 15] = [
    Some(10),
    None,
    Some(20),
    Some(11),
    None,
    Some(20),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
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
fn retained_png(path: &Path, size: (u32, u32), raw: &[u8]) {
    let mut reader = png::Decoder::new(fs::File::open(path).unwrap())
        .read_info()
        .unwrap();
    let mut decoded = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut decoded).unwrap();
    assert_eq!((info.width, info.height), size);
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    assert_eq!(
        &decoded[..info.buffer_size()],
        raw,
        "retained PNG must equal unmodified raw RGBA"
    );
}
fn retained_frame(path: &Path, live: &orr_ecs::Frame) {
    let bytes = fs::read(path).unwrap();
    let decoded = orr_ecs::Frame::from_bytes(live.registry().clone(), &bytes).unwrap();
    assert_eq!(decoded.to_bytes(), live.to_bytes());
    assert_eq!(decoded.tick(), live.tick());
    assert_eq!(decoded.checksum(), live.checksum());
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
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        drain(&mut h, 1);
        if h.state().editor.snapshot().is_some()
            && h.state().ui.viewport_rect.is_some()
            && h.state().viewport_gpu().is_some()
            && h.state().tilemap.is_some()
        {
            break;
        }
        assert!(Instant::now() < deadline, "actual App readiness deadline");
    }
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
        .agent_client("tile-resize-authority-probe")
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
    captures.push(json!({"name":name,"files":[format!("{name}.rgba"),format!("{name}.png"),format!("{name}.frame"),format!("{name}-app.png")],"scope":"actual App viewport; authoring grid, no pixel verdict","size":size,"rgba_fnv1a64":fingerprint(&pixels),"frame_fnv1a64":fingerprint(&raw_frame),"frame_source":"watch.subscribe source=view matched to predicted tick/checksum","tick":frame.tick(),"checksum":frame.checksum(),"adapter":info.name,"backend":format!("{:?}",info.backend),"device_type":format!("{:?}",info.device_type),"cells":cells(h),"dimensions":[h.state().tilemap.as_ref().unwrap().document().document().width,h.state().tilemap.as_ref().unwrap().document().document().height]}));
    write_json(
        &dir.join("partial-receipt.json"),
        &json!({"completed":false,"captures":captures,"native_os_input":false}),
    );
    retained_png(&dir.join(format!("{name}.png")), size, &pixels);
    retained_png(
        &dir.join(format!("{name}-app.png")),
        composed.dimensions(),
        composed.as_raw(),
    );
    assert_eq!(fs::read(dir.join(format!("{name}.rgba"))).unwrap(), pixels);
    retained_frame(&dir.join(format!("{name}.frame")), &frame);
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
    literal: &[Option<u32>; 15],
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
    retained_png(&dir.join(format!("{name}.png")), (224, 224), &pixels);
    assert_eq!(fs::read(dir.join(format!("{name}.rgba"))).unwrap(), pixels);
    retained_frame(
        &dir.join(format!("{name}.frame")),
        host.host().session().frame(),
    );
    // The full exposed 5x3 envelope [56,136) x [48,96) is above actors,
    // including orientation marks and collider AA. Check also the empty area
    // removed by trimming: the oracle does not use the saved dimensions.
    for row in 0..3 {
        for col in 0..5 {
            for y in 0..16 {
                for x in 0..16 {
                    let i = ((48 + row * 16 + y) * 224 + 56 + col * 16 + x) * 4;
                    let want = expected(literal[row * 5 + col], x, y);
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
    captures.last_mut().unwrap()["oracle_pixels_checked"] = json!(3840);
    assert_eq!(
        host.host().session().frame().to_bytes(),
        frame,
        "presentation must not mutate Frame"
    );
    (pixels, frame)
}

fn type_dimension(h: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    use egui::accesskit::Role;
    h.get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    drain(h, 2);
    h.get_by_role_and_label(Role::TextInput, label).focus();
    drain(h, 2);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    drain(h, 2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    drain(h, 2);
}
fn resize(h: &mut Harness<'_, EditorApp>, width: &str, height: &str) {
    let before = h.state().tilemap.as_ref().unwrap().history_snapshot();
    type_dimension(h, "Width", width);
    type_dimension(h, "Height", height);
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        before,
        "draft dimensions must not autoapply"
    );
    click(h, "Resize");
}
fn paint(h: &mut Harness<'_, EditorApp>, col: u32, row: u32) {
    let p = point(h, col, row);
    button(h, p, true);
    button(h, p, false);
}
fn document(
    h: &Harness<'_, EditorApp>,
    original: &orr_tilemap::Document,
    width: u32,
    height: u32,
    literal: &[Option<u32>],
) {
    let panel = h.state().tilemap.as_ref().unwrap();
    let actual = panel.document().document();
    let mut want = original.clone();
    want.width = width;
    want.height = height;
    want.cells = literal.to_vec();
    assert_eq!(actual, &want, "all metadata and cells, not just dimensions");
    assert_eq!(
        actual.origin.map(f32::to_bits),
        original.origin.map(f32::to_bits)
    );
    assert_eq!(
        actual.units_per_pixel.to_bits(),
        original.units_per_pixel.to_bits()
    );
    // Independent literal retained world centers. Production placements are the
    // observed values, never the expected result or the resize implementation.
    for (col, row, region, center) in [
        (0, 0, 10, [-48.0f32, 136.0f32]),
        (2, 0, 20, [-16.0, 136.0]),
        (0, 1, 11, [-48.0, 120.0]),
        (1, 1, 20, [-32.0, 120.0]),
    ] {
        let p = panel
            .document()
            .placements()
            .find(|p| p.cell.column == col && p.cell.row == row)
            .unwrap();
        assert_eq!(p.region, region);
        assert_eq!(p.position.map(f32::to_bits), center.map(f32::to_bits));
        assert_eq!(p.size, [16., 16.]);
    }
}
fn files(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, path: &Path, out: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
        for e in fs::read_dir(path).unwrap() {
            let e = e.unwrap();
            let kind = e.file_type().unwrap();
            assert!(
                !kind.is_symlink(),
                "creator fixture cannot contain symlinks"
            );
            if kind.is_dir() {
                walk(root, &e.path(), out);
            } else {
                out.insert(
                    e.path().strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(e.path()).unwrap(),
                );
            }
        }
    }
    let mut out = std::collections::BTreeMap::new();
    walk(root, root, &mut out);
    out
}

#[test]
#[ignore = "mandatory explicit actual EditorApp Resize software Vulkan acceptance"]
fn actual_cli_resize_save_reopen_play_literal_cells() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let evidence = PathBuf::from(
        std::env::var_os("ORR_TILEMAP_RESIZE_EVIDENCE")
            .expect("retained Resize evidence directory required"),
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
            "resize-acceptance",
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
    // Only fixture setup before admission; every authoring operation below uses
    // the actual widgets and pointer path, never panel mutation helpers.
    map.width = 3;
    map.height = 2;
    map.origin = [-56., 144.];
    map.cells = BEFORE.to_vec();
    fs::write(
        root.join("arena.tilemap.json"),
        serde_json::to_vec(&map).unwrap(),
    )
    .unwrap();
    let source_files = files(&root);
    let mut captures = Vec::new();
    let mut h = open(&root);
    let initial = capture(&mut h, &evidence, "before", &mut captures).to_bytes();
    document(&h, &map, 3, 2, &BEFORE);
    click(&mut h, "Tile tool (owns viewport clicks)");
    let initial_history = h.state().tilemap.as_ref().unwrap().history_snapshot();
    resize(&mut h, "5", "3");
    let grown = capture(&mut h, &evidence, "grown", &mut captures);
    document(&h, &map, 5, 3, &GROWN);
    assert_eq!(grown.to_bytes(), initial);
    let grown_history = h.state().tilemap.as_ref().unwrap().history_snapshot();
    assert_eq!(grown_history.2.len(), initial_history.2.len() + 1);
    assert!(grown_history.3.is_empty());
    assert_eq!(files(&root), source_files);
    click(&mut h, "Undo tile");
    capture(&mut h, &evidence, "resize-undo", &mut captures);
    document(&h, &map, 3, 2, &BEFORE);
    assert!(!h.state().tilemap.as_ref().unwrap().dirty());
    let undone = h.state().tilemap.as_ref().unwrap().history_snapshot();
    resize(&mut h, "3", "2");
    capture(&mut h, &evidence, "noop-preserves-redo", &mut captures);
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        undone
    );
    assert!(!h.state().tilemap.as_ref().unwrap().dirty());
    click(&mut h, "Redo tile");
    capture(&mut h, &evidence, "resize-redo", &mut captures);
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        grown_history
    );
    click(&mut h, "Paint");
    click(&mut h, "Static region 11");
    paint(&mut h, 4, 2);
    let painted = capture(&mut h, &evidence, "paint-new-cell", &mut captures);
    document(&h, &map, 5, 3, &PAINTED);
    assert_eq!(painted.to_bytes(), initial);
    assert!(h.state().editor.selected_guids().is_empty());
    assert!(h.state().ui.body_drag.is_none());
    let painted_history = h.state().tilemap.as_ref().unwrap().history_snapshot();
    assert_eq!(painted_history.2.len(), grown_history.2.len() + 1);
    click(&mut h, "Undo tile");
    capture(&mut h, &evidence, "paint-undo", &mut captures);
    document(&h, &map, 5, 3, &GROWN);
    click(&mut h, "Redo tile");
    capture(&mut h, &evidence, "paint-redo", &mut captures);
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        painted_history
    );
    resize(&mut h, "4", "2");
    let rejected = capture(&mut h, &evidence, "occupied-crop-rejected", &mut captures);
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        painted_history
    );
    assert!(h.state().tilemap.as_ref().unwrap().test_error().is_some());
    assert_eq!(rejected.to_bytes(), initial);
    assert_eq!(files(&root), source_files);
    click(&mut h, "Erase");
    paint(&mut h, 4, 2);
    capture(&mut h, &evidence, "erased-crop-cell", &mut captures);
    document(&h, &map, 5, 3, &GROWN);
    let erased_history = h.state().tilemap.as_ref().unwrap().history_snapshot();
    resize(&mut h, "4", "2");
    let trimmed = capture(&mut h, &evidence, "empty-trim", &mut captures);
    document(&h, &map, 4, 2, &TRIMMED);
    assert_eq!(
        h.state()
            .tilemap
            .as_ref()
            .unwrap()
            .history_snapshot()
            .2
            .len(),
        erased_history.2.len() + 1
    );
    assert_eq!(trimmed.to_bytes(), initial);
    assert_eq!(files(&root), source_files);
    click(&mut h, "Save tilemap");
    let saved = capture(&mut h, &evidence, "saved", &mut captures);
    assert_eq!(saved.to_bytes(), initial);
    assert!(!h.state().tilemap.as_ref().unwrap().dirty());
    let saved_bytes = fs::read(root.join("arena.tilemap.json")).unwrap();
    fs::write(evidence.join("saved.tilemap.json"), &saved_bytes).unwrap();
    let saved_map: orr_tilemap::Document = serde_json::from_slice(&saved_bytes).unwrap();
    let mut expected_map = map.clone();
    expected_map.width = 4;
    expected_map.height = 2;
    expected_map.cells = TRIMMED.to_vec();
    assert_eq!(saved_map, expected_map);
    let mut expected_files = source_files.clone();
    expected_files.insert(PathBuf::from("arena.tilemap.json"), saved_bytes);
    assert_eq!(files(&root), expected_files);
    drop(h);
    let mut reopened = open(&root);
    let fresh = capture(&mut reopened, &evidence, "fresh-reopen", &mut captures);
    document(&reopened, &map, 4, 2, &TRIMMED);
    assert_eq!(fresh.to_bytes(), initial);
    click(&mut reopened, LBL_STEP);
    let play = capture(&mut reopened, &evidence, "play", &mut captures);
    assert_eq!(reopened.state().editor.mode(), Mode::Play);
    assert_eq!(play.tick(), 1);
    document(&reopened, &map, 4, 2, &TRIMMED);
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
        &GROWN,
        &mut captures,
    );
    assert_eq!(
        play.to_bytes(),
        good_frame,
        "actual App and saved runtime same-tick Frame parity"
    );
    // A real independently launched authoritative host with no presentation
    // update, extraction or compositor invocation, not a predicted surrogate.
    let mut bare = PreparedRuntime::open(&root).unwrap().bridge().unwrap();
    bare.step(1);
    let bare_frame = bare.host().session().frame().to_bytes();
    fs::write(evidence.join("presentation-free.frame"), &bare_frame).unwrap();
    retained_frame(
        &evidence.join("presentation-free.frame"),
        bare.host().session().frame(),
    );
    assert_eq!(
        bare_frame, good_frame,
        "presentation-free authoritative control"
    );
    drop(reopened);
    expected_map.cells = WRONG_STRIDE.to_vec();
    let wrong_bytes = serde_json::to_vec(&expected_map).unwrap();
    fs::write(root.join("arena.tilemap.json"), &wrong_bytes).unwrap();
    fs::write(evidence.join("wrong-stride.tilemap.json"), &wrong_bytes).unwrap();
    let (wrong_pixels, wrong_frame) = consume_saved(
        &root,
        &evidence,
        &gpu,
        "runtime-wrong-stride",
        &WRONG_EXPOSED,
        &mut captures,
    );
    assert_eq!(
        wrong_frame, good_frame,
        "wrong stride changes presentation only"
    );
    // Exposed cell (3,0): the wrong prefix creates a visible tile where empty.
    let changed = (48..64)
        .flat_map(|y| (104..120).map(move |x| (y * 224 + x) * 4))
        .filter(|i| {
            wrong_pixels[*i..*i + 4]
                .iter()
                .zip(&good_pixels[*i..*i + 4])
                .any(|(a, b)| a.abs_diff(*b) > 3)
        })
        .count();
    assert!(
        changed >= 8,
        "wrong-stride control must visibly differ in exposed cell"
    );
    expected_files.insert(PathBuf::from("arena.tilemap.json"), wrong_bytes);
    assert_eq!(files(&root), expected_files);
    let file_bindings: Vec<_> = source_files
        .iter()
        .map(|(path, bytes)| json!({"path":path,"fnv1a64":fingerprint(bytes),"bytes":bytes.len()}))
        .collect();
    write_json(
        &evidence.join("receipt.json"),
        &json!({
            "completed":true,"captures":captures,"literal_before":BEFORE,"literal_grown":GROWN,
            "literal_painted":PAINTED,"literal_cells":TRIMMED,"literal_wrong_stride":WRONG_STRIDE,
            "oracle_pixels_per_runtime_capture":3840,"wrong_stride_visible_pixels":changed,
            "frame_parity":"authoring unchanged; actual App Play and saved runtime tick 1; wrong-stride and presentation-free authoritative Frame bytes equal",
            "presentation_free_frame_fnv1a64":fingerprint(&bare_frame),
            "creator_fnv1a64":fingerprint(&fs::read(cli).unwrap()),
            "test_source_fnv1a64":fingerprint(include_bytes!("tilemap_resize_tests.rs")),
            "panel_source_fnv1a64":fingerprint(include_bytes!("../tilemap_panel.rs")),
            "panel_resize_source_fnv1a64":fingerprint(include_bytes!("../tilemap_panel/resize.rs")),
            "core_source_fnv1a64":fingerprint(include_bytes!("../../../orr_tilemap/src/lib.rs")),
            "source_rgba_fnv1a64":fingerprint(SOURCE),"fixture_file_bindings":file_bindings,
            "native_os_input":false,"export":false,"windows":false
        }),
    );
}
