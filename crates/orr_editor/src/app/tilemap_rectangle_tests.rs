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
// Hand-written asymmetric fixtures. No candidate, placement, or cell helper
// computes expected paint, erase, stride, or extra-cell results.
const BEFORE: [Option<u32>; 28] = [
    Some(10),
    None,
    Some(20),
    None,
    Some(11),
    None,
    Some(20),
    None,
    Some(20),
    None,
    Some(10),
    None,
    Some(20),
    None,
    Some(20),
    None,
    Some(11),
    None,
    Some(20),
    None,
    Some(10),
    None,
    Some(10),
    None,
    Some(20),
    None,
    Some(11),
    None,
];
const PAINTED: [Option<u32>; 28] = [
    Some(10),
    None,
    Some(20),
    None,
    Some(11),
    None,
    Some(20),
    None,
    Some(11),
    Some(11),
    Some(11),
    Some(11),
    Some(20),
    None,
    Some(20),
    Some(11),
    Some(11),
    Some(11),
    Some(11),
    None,
    Some(10),
    None,
    Some(10),
    None,
    Some(20),
    None,
    Some(11),
    None,
];
const ERASED: [Option<u32>; 28] = [
    Some(10),
    None,
    Some(20),
    None,
    Some(11),
    None,
    Some(20),
    None,
    Some(11),
    None,
    None,
    Some(11),
    Some(20),
    None,
    Some(20),
    Some(11),
    None,
    None,
    Some(11),
    None,
    Some(10),
    None,
    Some(10),
    None,
    Some(20),
    None,
    Some(11),
    None,
];
// Paint/erase using height 4 as row stride instead of width 7.
const WRONG_STRIDE: [Option<u32>; 28] = [
    Some(10),
    None,
    Some(20),
    None,
    Some(11),
    Some(11),
    None,
    None,
    Some(11),
    Some(11),
    None,
    None,
    Some(11),
    None,
    Some(20),
    None,
    Some(11),
    None,
    Some(20),
    None,
    Some(10),
    None,
    Some(10),
    None,
    Some(20),
    None,
    Some(11),
    None,
];
// An extra painted cell beyond both accepted rectangles, at (6,3).
const EXTRA_CELL: [Option<u32>; 28] = [
    Some(10),
    None,
    Some(20),
    None,
    Some(11),
    None,
    Some(20),
    None,
    Some(11),
    None,
    None,
    Some(11),
    Some(20),
    None,
    Some(20),
    Some(11),
    None,
    None,
    Some(11),
    None,
    Some(10),
    None,
    Some(10),
    None,
    Some(20),
    None,
    Some(11),
    Some(11),
];

// The production App owns all project, UI, pointer and rendering operations.
// This wrapper adds only a real foreground egui obstruction for a release test;
// it cannot access or manufacture a tile gesture or history entry.
struct TestApp {
    app: EditorApp,
    obstruction: Option<egui::Rect>,
}
impl std::ops::Deref for TestApp {
    type Target = EditorApp;
    fn deref(&self) -> &EditorApp {
        &self.app
    }
}
impl std::ops::DerefMut for TestApp {
    fn deref_mut(&mut self) -> &mut EditorApp {
        &mut self.app
    }
}
impl eframe::App for TestApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        eframe::App::ui(&mut self.app, ui, frame);
        if let Some(rect) = self.obstruction {
            egui::Area::new(egui::Id::new("rectangle-release-obstruction"))
                .order(egui::Order::Foreground)
                .fixed_pos(rect.min)
                .movable(false)
                .show(ui.ctx(), |ui| {
                    let (actual, _) =
                        ui.allocate_exact_size(rect.size(), egui::Sense::click_and_drag());
                    ui.painter()
                        .rect_filled(actual, 0., egui::Color32::DARK_GRAY);
                    ui.painter().text(
                        actual.center(),
                        egui::Align2::CENTER_CENTER,
                        "Obstruction",
                        egui::FontId::proportional(12.),
                        egui::Color32::WHITE,
                    );
                });
        }
    }
}
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
fn drain(h: &mut Harness<'_, TestApp>, count: usize) {
    for _ in 0..count {
        h.run_steps(1);
        // Poll the real renderer every UI step, not hundreds of queued submits.
        h.render().expect("per-step actual app GPU drain");
    }
}
fn click(h: &mut Harness<'_, TestApp>, label: &str) {
    h.get_by_label(label).click();
    drain(h, 3);
}
fn open(root: &Path) -> Harness<'static, TestApp> {
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
        .with_pixels_per_point(1.)
        .wgpu_setup(setup.into())
        .build_eframe(move |cc| {
            let mut editor = Editor::start(&prepared.host_spec()).unwrap();
            editor.sync();
            editor.camera = Camera::new([0., 80.], 160.);
            TestApp {
                app: prepared
                    .into_app(editor, cc.wgpu_render_state.clone(), &cc.egui_ctx)
                    .unwrap(),
                obstruction: None,
            }
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
fn point(h: &Harness<'_, TestApp>, column: u32, row: u32) -> egui::Pos2 {
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
fn button(h: &mut Harness<'_, TestApp>, p: egui::Pos2, pressed: bool) {
    h.event(egui::Event::PointerMoved(p));
    h.event(egui::Event::PointerButton {
        pos: p,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    });
    drain(h, 2);
}
fn cells<'a>(h: &'a Harness<'_, TestApp>) -> &'a [Option<u32>] {
    &h.state()
        .tilemap
        .as_ref()
        .unwrap()
        .document()
        .document()
        .cells
}
fn authoritative_frame(h: &mut Harness<'_, TestApp>) -> orr_ecs::Frame {
    h.state_mut().editor.sync();
    let snapshot = h.state().editor.snapshot().unwrap();
    let tick = snapshot.predicted().tick();
    let checksum = snapshot.predicted().checksum();
    let client = h
        .state_mut()
        .editor
        .agent_client("tile-rectangle-authority-probe")
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
    h: &mut Harness<'_, TestApp>,
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
    let history = h.state().tilemap.as_ref().unwrap().history_snapshot();
    captures.push(json!({"name":name,"history":{"undo":history.2.len(),"redo":history.3.len(),"dirty":h.state().tilemap.as_ref().unwrap().dirty(),"saved_fnv1a64":fingerprint(&history.1)},"files":[format!("{name}.rgba"),format!("{name}.png"),format!("{name}.frame"),format!("{name}-app.png")],"scope":"actual App viewport; authoring grid, no pixel verdict","size":size,"rgba_fnv1a64":fingerprint(&pixels),"frame_fnv1a64":fingerprint(&raw_frame),"frame_source":"watch.subscribe source=view matched to predicted tick/checksum","tick":frame.tick(),"checksum":frame.checksum(),"adapter":info.name,"backend":format!("{:?}",info.backend),"device_type":format!("{:?}",info.device_type),"cells":cells(h),"dimensions":[h.state().tilemap.as_ref().unwrap().document().document().width,h.state().tilemap.as_ref().unwrap().document().document().height]}));
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
    literal: &[Option<u32>; 28],
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
    // Entire literal 7x4 envelope [56,168) x [48,112) is above actors,
    // including orientation marks and collider AA. Neither saved dimensions
    // nor production extraction, UV, placement or candidate helpers form this oracle.
    for row in 0..4 {
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
    captures.last_mut().unwrap()["oracle_pixels_checked"] = json!(7168);
    assert_eq!(
        host.host().session().frame().to_bytes(),
        frame,
        "presentation must not mutate Frame"
    );
    (pixels, frame)
}

// AccessKit activation changes a widget without inventing a second physical
// pointer press/release while a rectangle is held.
fn activate(h: &mut Harness<'_, TestApp>, label: &str) {
    h.get_by_label(label).click_accesskit();
    drain(h, 3);
}
fn move_to(h: &mut Harness<'_, TestApp>, p: egui::Pos2) {
    h.event(egui::Event::PointerMoved(p));
    drain(h, 2);
}
fn begin(h: &mut Harness<'_, TestApp>, from: (u32, u32), to: (u32, u32)) -> egui::Pos2 {
    let start = point(h, from.0, from.1);
    let end = point(h, to.0, to.1);
    button(h, start, true);
    move_to(h, end);
    end
}
fn rectangle(h: &mut Harness<'_, TestApp>, from: (u32, u32), to: (u32, u32)) {
    let before = h.state().tilemap.as_ref().unwrap().history_snapshot();
    let end = begin(h, from, to);
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        before,
        "held rectangle is preview only"
    );
    button(h, end, false);
}
fn document(h: &Harness<'_, TestApp>, original: &orr_tilemap::Document, literal: &[Option<u32>]) {
    let actual = h.state().tilemap.as_ref().unwrap().document().document();
    let mut want = original.clone();
    want.cells = literal.to_vec();
    assert_eq!(actual, &want, "all metadata and literal cells must match");
    assert_eq!(
        actual.origin.map(f32::to_bits),
        original.origin.map(f32::to_bits)
    );
    assert_eq!(
        actual.units_per_pixel.to_bits(),
        original.units_per_pixel.to_bits()
    );
}
fn visible_difference(a: &[u8], b: &[u8], column: usize, row: usize) -> usize {
    (48 + row * 16..64 + row * 16)
        .flat_map(|y| (56 + column * 16..72 + column * 16).map(move |x| (y * 224 + x) * 4))
        .filter(|i| {
            a[*i..*i + 4]
                .iter()
                .zip(&b[*i..*i + 4])
                .any(|(a, b)| a.abs_diff(*b) > 3)
        })
        .count()
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
#[ignore = "mandatory explicit actual EditorApp Rectangle software Vulkan acceptance"]
fn actual_cli_rectangle_save_reopen_play_literal_cells() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let evidence = PathBuf::from(
        std::env::var_os("ORR_TILEMAP_RECTANGLE_EVIDENCE")
            .expect("retained Rectangle evidence directory required"),
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
            "rectangle-acceptance",
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
    // Fixture setup occurs before admission. Every authoring action below uses
    // real widgets and pointer input, never candidate/history mutation helpers.
    map.width = 7;
    map.height = 4;
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
    document(&h, &map, &BEFORE);
    click(&mut h, "Tile tool (owns viewport clicks)");
    click(&mut h, "Rectangle");
    click(&mut h, "Static region 11");
    let initial_history = h.state().tilemap.as_ref().unwrap().history_snapshot();

    // Every cancellation compares the entire document, saved marker, Undo and
    // Redo vectors, source file inventory, authoritative Frame and dirty flag.
    macro_rules! cancelled {
        ($phase:literal) => {{
            let frame = capture(&mut h, &evidence, $phase, &mut captures);
            assert_eq!(
                h.state().tilemap.as_ref().unwrap().history_snapshot(),
                initial_history,
                $phase
            );
            document(&h, &map, &BEFORE);
            assert!(!h.state().tilemap.as_ref().unwrap().dirty(), $phase);
            assert_eq!(frame.to_bytes(), initial, $phase);
            assert_eq!(files(&root), source_files, $phase);
            assert!(h.state().editor.selected_guids().is_empty(), $phase);
            assert!(h.state().ui.body_drag.is_none(), $phase);
        }};
    }
    let inside = point(&h, 4, 2);
    button(&mut h, egui::pos2(0., 0.), true);
    move_to(&mut h, inside);
    button(&mut h, inside, false);
    cancelled!("outside-origin");
    // Inside viewport but outside the map cannot acquire a rectangle either.
    let outside_cell = point(&h, 7, 1);
    button(&mut h, outside_cell, true);
    move_to(&mut h, inside);
    button(&mut h, inside, false);
    cancelled!("outside-cell-origin");

    // A native integration may coalesce input events. Event positions, not the
    // final hover position, must determine both origin and release validity.
    h.input_mut().events.extend([
        egui::Event::PointerMoved(egui::pos2(0., 0.)),
        egui::Event::PointerButton {
            pos: egui::pos2(0., 0.),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
        egui::Event::PointerButton {
            pos: egui::pos2(0., 0.),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        },
        egui::Event::PointerMoved(inside),
    ]);
    drain(&mut h, 2);
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        initial_history
    );

    let baseline_app = h.render().expect("preview baseline App readback");
    png(
        &evidence.join("preview-baseline-app.png"),
        baseline_app.dimensions(),
        baseline_app.as_raw(),
    );
    retained_png(
        &evidence.join("preview-baseline-app.png"),
        baseline_app.dimensions(),
        baseline_app.as_raw(),
    );
    let start = point(&h, 1, 1);
    let end = begin(&mut h, (1, 1), (4, 2));
    cancelled!("preview");
    let preview_app = h.render().expect("held preview App readback");
    assert_eq!(preview_app.dimensions(), baseline_app.dimensions());
    let preview_rect = egui::Rect::from_two_pos(start, end).expand(64.);
    let preview_pixel_rect = [
        preview_rect.min.x.ceil() as u32,
        preview_rect.min.y.ceil() as u32,
        preview_rect.max.x.floor() as u32,
        preview_rect.max.y.floor() as u32,
    ];
    assert!(
        preview_pixel_rect[2] <= baseline_app.width()
            && preview_pixel_rect[3] <= baseline_app.height()
    );
    let preview_visible_pixels = (preview_pixel_rect[1]..preview_pixel_rect[3])
        .flat_map(|y| (preview_pixel_rect[0]..preview_pixel_rect[2]).map(move |x| (x, y)))
        .filter(|(x, y)| {
            baseline_app
                .get_pixel(*x, *y)
                .0
                .iter()
                .zip(preview_app.get_pixel(*x, *y).0)
                .any(|(a, b)| a.abs_diff(b) > 3)
        })
        .count();
    assert!(
        preview_visible_pixels >= 8,
        "actual App must visibly display the preview while document remains unchanged"
    );
    h.key_press(egui::Key::Escape);
    drain(&mut h, 2);
    button(&mut h, end, false);
    cancelled!("escape-cancelled");

    let end = begin(&mut h, (1, 1), (4, 2));
    h.input_mut().focused = false;
    h.event(egui::Event::WindowFocused(false));
    drain(&mut h, 2);
    button(&mut h, end, false);
    h.input_mut().focused = true;
    h.event(egui::Event::WindowFocused(true));
    drain(&mut h, 2);
    cancelled!("focus-cancelled");

    begin(&mut h, (1, 1), (4, 2));
    let outside_cell = point(&h, 7, 2);
    // Release outside then hover inside in one frame must not reuse inside hover.
    h.input_mut().events.extend([
        egui::Event::PointerMoved(outside_cell),
        egui::Event::PointerButton {
            pos: outside_cell,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        },
        egui::Event::PointerMoved(inside),
    ]);
    drain(&mut h, 2);
    cancelled!("outside-release");

    let end = begin(&mut h, (1, 1), (4, 2));
    h.state_mut().obstruction = Some(egui::Rect::from_center_size(end, egui::vec2(100., 80.)));
    drain(&mut h, 3);
    assert_eq!(
        h.ctx.layer_id_at(end).unwrap().order,
        egui::Order::Foreground,
        "real foreground layer must cover the release cell"
    );
    button(&mut h, end, false);
    cancelled!("obstructed-release");
    h.state_mut().obstruction = None;
    drain(&mut h, 3);

    let end = begin(&mut h, (1, 1), (4, 2));
    activate(&mut h, "Paint");
    button(&mut h, end, false);
    cancelled!("mode-cancelled");
    click(&mut h, "Rectangle");
    let end = begin(&mut h, (1, 1), (4, 2));
    activate(&mut h, "Tile tool (owns viewport clicks)");
    assert!(!h.state().tilemap.as_ref().unwrap().active_tool());
    button(&mut h, end, false);
    cancelled!("tool-cancelled");
    click(&mut h, "Tile tool (owns viewport clicks)");

    // An external sidecar byte change cannot be silently adopted during a drag.
    let end = begin(&mut h, (1, 1), (4, 2));
    let original_sidecar = fs::read(root.join("arena.tilemap.json")).unwrap();
    let mut changed_sidecar = original_sidecar.clone();
    changed_sidecar.push(b'\n');
    fs::write(root.join("arena.tilemap.json"), &changed_sidecar).unwrap();
    drain(&mut h, 2);
    button(&mut h, end, false);
    assert_eq!(
        fs::read(root.join("arena.tilemap.json")).unwrap(),
        changed_sidecar,
        "rejected gesture cannot overwrite external bytes"
    );
    fs::write(root.join("arena.tilemap.json"), original_sidecar).unwrap();
    cancelled!("source-invalidated");
    // Scene content can change without a scene path or host epoch change.
    let end = begin(&mut h, (1, 1), (4, 2));
    let original_scene = fs::read(root.join("arena.scene.yaml")).unwrap();
    let mut changed_scene = original_scene.clone();
    changed_scene.extend_from_slice(b"\n# externally changed during rectangle\n");
    fs::write(root.join("arena.scene.yaml"), &changed_scene).unwrap();
    drain(&mut h, 2);
    button(&mut h, end, false);
    assert_eq!(
        fs::read(root.join("arena.scene.yaml")).unwrap(),
        changed_scene
    );
    fs::write(root.join("arena.scene.yaml"), original_scene).unwrap();
    cancelled!("scene-bytes-invalidated");

    // A real proposal preview makes the existing Edit viewport read-only.
    // Use an unchanged proposal so the guard is isolated from scene mutation.
    let proposal = h
        .state_mut()
        .editor
        .host_call("proposal.begin", json!({"label":"Rectangle preview guard"}))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    h.state_mut().editor.sync();
    assert!(h
        .state()
        .editor
        .proposals()
        .iter()
        .any(|p| p.id == proposal));
    let end = begin(&mut h, (1, 1), (4, 2));
    assert!(h.state_mut().editor.set_preview(Some(proposal.clone())));
    drain(&mut h, 3);
    assert_eq!(h.state().editor.previewing(), Some(proposal.as_str()));
    button(&mut h, end, false);
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        initial_history
    );
    assert_eq!(authoritative_frame(&mut h).to_bytes(), initial);
    assert_eq!(files(&root), source_files);
    assert!(h.state_mut().editor.set_preview(None));
    h.state_mut()
        .editor
        .host_call("proposal.reject", json!({"id":proposal}))
        .unwrap();
    drain(&mut h, 3);

    let end = begin(&mut h, (1, 1), (4, 2));
    activate(&mut h, LBL_STEP);
    button(&mut h, end, false);
    let play_cancelled = capture(&mut h, &evidence, "play-cancelled", &mut captures);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    assert_eq!(play_cancelled.tick(), 1);
    // This is the reachable record-mode PlaySession transition, not a
    // fabricated replay Viewer or a disconnected-host state.
    let timeline = h
        .state()
        .editor
        .timeline()
        .expect("actual PlaySession timeline");
    assert!(timeline.mode != orr_bridge::PlayMode::Viewer);
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        initial_history
    );
    assert_eq!(files(&root), source_files);
    click(&mut h, LBL_STOP);
    assert!(
        h.state().editor.timeline().is_none(),
        "Stop leaves the PlaySession timeline"
    );
    cancelled!("play-stopped");

    let end = begin(&mut h, (1, 1), (4, 2));
    let epoch = h.state().editor.scene_load_epoch();
    // The real host scene-load route, with identical path and bytes, changes
    // epoch. A stale pending rectangle cannot survive this identity transition.
    assert!(h
        .state_mut()
        .editor
        .open_path(&root.join("arena.scene.yaml")));
    drain(&mut h, 3);
    assert_ne!(h.state().editor.scene_load_epoch(), epoch);
    button(&mut h, end, false);
    cancelled!("stale-scene-epoch");
    drop(h);
    let mut h = open(&root);
    assert_eq!(
        capture(&mut h, &evidence, "fresh-authoring", &mut captures).to_bytes(),
        initial
    );
    click(&mut h, "Tile tool (owns viewport clicks)");
    click(&mut h, "Rectangle");
    click(&mut h, "Static region 11");

    let end = begin(&mut h, (1, 1), (4, 2));
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        initial_history
    );
    // The selected paint value is frozen on press, not reread on release.
    activate(&mut h, "Static region 20");
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        initial_history
    );
    button(&mut h, end, false);
    let painted = capture(&mut h, &evidence, "rectangle-painted", &mut captures);
    document(&h, &map, &PAINTED);
    assert_eq!(painted.to_bytes(), initial);
    let painted_history = h.state().tilemap.as_ref().unwrap().history_snapshot();
    assert_eq!(
        painted_history.2.len(),
        initial_history.2.len() + 1,
        "one valid release creates exactly one Undo"
    );
    assert!(painted_history.3.is_empty());
    assert_eq!(
        painted_history.1, initial_history.1,
        "release cannot move the saved marker"
    );
    assert!(h.state().editor.selected_guids().is_empty());
    assert!(h.state().ui.body_drag.is_none());
    assert_eq!(files(&root), source_files);
    click(&mut h, "Undo tile");
    assert_eq!(
        capture(&mut h, &evidence, "paint-undo", &mut captures).to_bytes(),
        initial
    );
    document(&h, &map, &BEFORE);
    assert!(!h.state().tilemap.as_ref().unwrap().dirty());
    let undone = h.state().tilemap.as_ref().unwrap().history_snapshot();
    click(&mut h, "Static region 10");
    rectangle(&mut h, (0, 0), (0, 0));
    assert_eq!(
        capture(&mut h, &evidence, "noop-preserves-redo", &mut captures).to_bytes(),
        initial
    );
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        undone
    );
    assert!(!h.state().tilemap.as_ref().unwrap().dirty());
    click(&mut h, "Redo tile");
    assert_eq!(
        capture(&mut h, &evidence, "paint-redo", &mut captures).to_bytes(),
        initial
    );
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        painted_history
    );
    click(&mut h, "Undo tile");
    click(&mut h, "Static region 11");
    rectangle(&mut h, (4, 2), (1, 1));
    assert_eq!(
        capture(&mut h, &evidence, "reverse-painted", &mut captures).to_bytes(),
        initial
    );
    document(&h, &map, &PAINTED);
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        painted_history
    );

    click(&mut h, "Erase");
    rectangle(&mut h, (3, 2), (2, 1));
    assert_eq!(
        capture(&mut h, &evidence, "rectangle-erased", &mut captures).to_bytes(),
        initial
    );
    document(&h, &map, &ERASED);
    let erased_history = h.state().tilemap.as_ref().unwrap().history_snapshot();
    assert_eq!(erased_history.2.len(), painted_history.2.len() + 1);
    assert_eq!(erased_history.1, initial_history.1);
    click(&mut h, "Undo tile");
    assert_eq!(
        capture(&mut h, &evidence, "erase-undo", &mut captures).to_bytes(),
        initial
    );
    document(&h, &map, &PAINTED);
    click(&mut h, "Redo tile");
    assert_eq!(
        capture(&mut h, &evidence, "erase-redo", &mut captures).to_bytes(),
        initial
    );
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        erased_history
    );
    assert_eq!(files(&root), source_files, "authoring never autosaves");
    click(&mut h, "Save tilemap");
    assert_eq!(
        capture(&mut h, &evidence, "saved", &mut captures).to_bytes(),
        initial
    );
    assert!(!h.state().tilemap.as_ref().unwrap().dirty());
    let saved_history = h.state().tilemap.as_ref().unwrap().history_snapshot();
    let saved_bytes = fs::read(root.join("arena.tilemap.json")).unwrap();
    fs::write(evidence.join("saved.tilemap.json"), &saved_bytes).unwrap();
    let saved_map: orr_tilemap::Document = serde_json::from_slice(&saved_bytes).unwrap();
    let mut expected_map = map.clone();
    expected_map.cells = ERASED.to_vec();
    assert_eq!(saved_map, expected_map);
    let mut expected_files = source_files.clone();
    expected_files.insert(PathBuf::from("arena.tilemap.json"), saved_bytes.clone());
    assert_eq!(files(&root), expected_files);
    rectangle(&mut h, (3, 2), (2, 1));
    assert_eq!(
        capture(&mut h, &evidence, "saved-noop", &mut captures).to_bytes(),
        initial
    );
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        saved_history
    );
    assert!(!h.state().tilemap.as_ref().unwrap().dirty());
    click(&mut h, "Undo tile");
    let saved_undo = h.state().tilemap.as_ref().unwrap().history_snapshot();
    rectangle(&mut h, (6, 3), (6, 3));
    assert_eq!(
        capture(&mut h, &evidence, "saved-undo-noop", &mut captures).to_bytes(),
        initial
    );
    document(&h, &map, &PAINTED);
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        saved_undo
    );
    assert!(h.state().tilemap.as_ref().unwrap().dirty());
    click(&mut h, "Redo tile");
    assert_eq!(
        capture(&mut h, &evidence, "saved-redo", &mut captures).to_bytes(),
        initial
    );
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        saved_history
    );
    assert!(!h.state().tilemap.as_ref().unwrap().dirty());
    assert_eq!(files(&root), expected_files);
    drop(h);

    let mut reopened = open(&root);
    let fresh = capture(&mut reopened, &evidence, "fresh-reopen", &mut captures);
    document(&reopened, &map, &ERASED);
    assert_eq!(fresh.to_bytes(), initial);
    assert!(!reopened.state().tilemap.as_ref().unwrap().dirty());
    click(&mut reopened, LBL_STEP);
    let play = capture(&mut reopened, &evidence, "play", &mut captures);
    assert_eq!(reopened.state().editor.mode(), Mode::Play);
    assert_eq!(play.tick(), 1);
    document(&reopened, &map, &ERASED);
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
        &ERASED,
        &mut captures,
    );
    assert_eq!(
        play.to_bytes(),
        good_frame,
        "actual App and saved runtime same-tick Frame parity"
    );
    assert_eq!(
        play_cancelled.to_bytes(),
        good_frame,
        "rectangle authoring never changes authoritative simulation"
    );
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
        &WRONG_STRIDE,
        &mut captures,
    );
    assert_eq!(
        wrong_frame, good_frame,
        "wrong row stride changes presentation only"
    );
    let wrong_stride_visible_pixels = visible_difference(&wrong_pixels, &good_pixels, 5, 0);
    assert!(
        wrong_stride_visible_pixels >= 8,
        "wrong-stride control must visibly differ in exposed cell (5,0)"
    );
    expected_map.cells = EXTRA_CELL.to_vec();
    let extra_bytes = serde_json::to_vec(&expected_map).unwrap();
    fs::write(root.join("arena.tilemap.json"), &extra_bytes).unwrap();
    fs::write(evidence.join("extra-cell.tilemap.json"), &extra_bytes).unwrap();
    let (extra_pixels, extra_frame) = consume_saved(
        &root,
        &evidence,
        &gpu,
        "runtime-extra-cell",
        &EXTRA_CELL,
        &mut captures,
    );
    assert_eq!(
        extra_frame, good_frame,
        "extra rectangle cell changes presentation only"
    );
    let extra_cell_visible_pixels = visible_difference(&extra_pixels, &good_pixels, 6, 3);
    assert!(
        extra_cell_visible_pixels >= 8,
        "extra-cell control must visibly differ in exposed cell (6,3)"
    );
    // Restore the accepted saved fixture after negative controls and compare the
    // complete source tree: no manifest, lock, package, scene or actor mutation.
    fs::write(root.join("arena.tilemap.json"), saved_bytes).unwrap();
    assert_eq!(files(&root), expected_files);
    let file_bindings: Vec<_> = source_files
        .iter()
        .map(|(path, bytes)| json!({"path":path,"fnv1a64":fingerprint(bytes),"bytes":bytes.len()}))
        .collect();
    write_json(
        &evidence.join("receipt.json"),
        &json!({
            "completed":true,"captures":captures,"literal_before":BEFORE,
            "literal_painted":PAINTED,"literal_cells":ERASED,"literal_wrong_stride":WRONG_STRIDE,
            "literal_extra_cell":EXTRA_CELL,"oracle_pixels_per_runtime_capture":7168,
            "preview_visible_pixels":preview_visible_pixels,"preview_pixel_rect":preview_pixel_rect,
            "wrong_stride_visible_pixels":wrong_stride_visible_pixels,"extra_cell_visible_pixels":extra_cell_visible_pixels,
            "frame_parity":"authoring unchanged; actual App Play and saved runtime tick 1; wrong-stride, extra-cell and presentation-free authoritative Frame bytes equal",
            "presentation_free_frame_fnv1a64":fingerprint(&bare_frame),
            "creator_fnv1a64":fingerprint(&fs::read(cli).unwrap()),
            "test_source_fnv1a64":fingerprint(include_bytes!("tilemap_rectangle_tests.rs")),
            "panel_source_fnv1a64":fingerprint(include_bytes!("../tilemap_panel.rs")),
            "panel_rectangle_source_fnv1a64":fingerprint(include_bytes!("../tilemap_panel/rectangle.rs")),
            "core_source_fnv1a64":fingerprint(include_bytes!("../../../orr_tilemap/src/lib.rs")),
            "source_rgba_fnv1a64":fingerprint(SOURCE),"fixture_file_bindings":file_bindings,
            "native_os_input":false,"export":false,"windows":false
        }),
    );
}
