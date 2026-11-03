//! Actual AccessKit widgets and egui pointer events, not native desktop input.
//! This explicit software-GPU gate retains artifacts before semantic assertions.
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
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
    time::{Duration, Instant},
};

const SOURCE: &[u8] = include_bytes!("../../../../assets/sprite_demo/lantern_keeper.rgba");
// Literal 7x4 states: source motif is [10, None, 11; 20, 21, 10].
// Some(0) requires an atlas region absent from the original starter and is
// exercised by CPU tests; this GPU gate retains the original admitted atlas.
const BEFORE: [Option<u32>; 28] = [
    Some(10),
    None,
    Some(11),
    Some(20),
    Some(21),
    Some(20),
    Some(11),
    Some(20),
    Some(21),
    Some(10),
    Some(11),
    Some(20),
    Some(21),
    Some(10),
    Some(11),
    Some(20),
    Some(21),
    Some(10),
    Some(20),
    Some(21),
    Some(11),
    Some(21),
    Some(11),
    Some(20),
    None,
    Some(11),
    Some(20),
    Some(21),
];
const STAMPED: [Option<u32>; 28] = [
    Some(10),
    None,
    Some(11),
    Some(20),
    Some(21),
    Some(20),
    Some(11),
    Some(20),
    Some(21),
    Some(10),
    Some(11),
    Some(20),
    Some(21),
    Some(10),
    Some(11),
    Some(20),
    Some(21),
    Some(10),
    Some(10),
    None,
    Some(11),
    Some(21),
    Some(11),
    Some(20),
    None,
    Some(20),
    Some(21),
    Some(10),
];
const OVERLAPPED: [Option<u32>; 28] = [
    Some(10),
    None,
    Some(11),
    Some(20),
    Some(21),
    Some(20),
    Some(11),
    Some(20),
    Some(10),
    None,
    Some(11),
    Some(20),
    Some(21),
    Some(10),
    Some(11),
    Some(20),
    Some(21),
    Some(10),
    Some(10),
    None,
    Some(11),
    Some(21),
    Some(11),
    Some(20),
    None,
    Some(20),
    Some(21),
    Some(10),
];
const REPEATED: [Option<u32>; 28] = [
    Some(10),
    None,
    Some(11),
    Some(20),
    Some(21),
    Some(20),
    Some(11),
    Some(20),
    Some(10),
    None,
    Some(11),
    Some(20),
    Some(21),
    Some(10),
    Some(10),
    None,
    Some(11),
    Some(10),
    Some(10),
    None,
    Some(11),
    Some(20),
    Some(21),
    Some(10),
    None,
    Some(20),
    Some(21),
    Some(10),
];
// Negative control: apply final (0,2) stamp with height 4 as row stride,
// writing source rows at indices 8..10 and 12..14 rather than 14..16/21..23.
const WRONG_STRIDE: [Option<u32>; 28] = [
    Some(10),
    None,
    Some(11),
    Some(20),
    Some(21),
    Some(20),
    Some(11),
    Some(20),
    Some(10),
    None,
    Some(11),
    Some(20),
    Some(20),
    Some(21),
    Some(10),
    Some(20),
    Some(21),
    Some(10),
    Some(10),
    None,
    Some(11),
    Some(21),
    Some(11),
    Some(20),
    None,
    Some(20),
    Some(21),
    Some(10),
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
            egui::Area::new(egui::Id::new("copy-release-obstruction"))
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
    h.get_by_label(label).scroll_to_me();
    drain(h, 3);
    assert!(
        !h.get_by_label(label).accesskit_node().is_disabled(),
        "disabled widget: {label}"
    );
    h.get_by_label(label).click_accesskit();
    drain(h, 3);
}
fn open(root: &Path) -> Harness<'static, TestApp> {
    let prepared = crate::project::PreparedProject::open(root).unwrap();
    let mut h = Harness::builder()
        .with_size([1800., 2000.])
        .with_pixels_per_point(1.)
        .with_render_options(egui_wgpu::RendererOptions {
            predictable_texture_filtering: false,
            ..egui_wgpu::RendererOptions::PREDICTABLE
        })
        .wgpu()
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
        .agent_client("tile-copy-authority-probe")
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
    eprintln!(
        "copy/stamp adapter={} backend={:?} type={:?}",
        info.name, info.backend, info.device_type
    );
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
            21 => 48,
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

// Independently sample opaque original region-10 texels of the selected hero.
// Transparent samples are excluded because actor colliders underlie the sprite.
fn actor_oracle(h: &Harness<'_, TestApp>, path: &Path) -> Vec<[u8; 4]> {
    let mut reader = png::Decoder::new(fs::File::open(path).unwrap())
        .read_info()
        .unwrap();
    let mut rgba = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut rgba).unwrap();
    let rect = h.state().ui.viewport_rect.unwrap();
    let size = h.state().ui.viewport_px;
    let mut samples = Vec::new();
    let mut colors = std::collections::BTreeSet::new();
    for y in 0..16 {
        for x in 0..16 {
            let source = (y * 64 + x) * 4;
            if SOURCE[source + 3] != 255 {
                continue;
            }
            let world = [-60. + (x as f32 - 7.5) * 2., (7.5 - y as f32) * 2.];
            let screen = h.state().editor.camera.world_to_screen(world, size);
            let px = ((rect.min.x + screen[0] * rect.width() / size.0 as f32)
                * h.ctx.pixels_per_point())
            .floor() as usize;
            let py = ((rect.min.y + screen[1] * rect.height() / size.1 as f32)
                * h.ctx.pixels_per_point())
            .floor() as usize;
            let offset = (py * info.width as usize + px) * 4;
            let actual: [u8; 4] = rgba[offset..offset + 4].try_into().unwrap();
            assert!(
                actual
                    .iter()
                    .zip(&SOURCE[source..source + 4])
                    .all(|(a, b)| a.abs_diff(*b) <= 3),
                "selected hero original atlas texel ({x},{y}) mismatch"
            );
            colors.insert(SOURCE[source..source + 4].to_vec());
            samples.push(actual);
        }
    }
    assert!(
        samples.len() >= 90 && colors.len() >= 3,
        "selected actor oracle must be multicolor"
    );
    samples
}

fn stamp(h: &mut Harness<'_, TestApp>, at: (u32, u32)) {
    let p = point(h, at.0, at.1);
    button(h, p, true);
    button(h, p, false);
}

// Read retained native viewport bytes, not a reconstructed compositor result.
// Geometry and source texels are literal; no production placement/UV helper.
fn app_play_oracle(h: &Harness<'_, TestApp>, dir: &Path, name: &str) {
    let size = h.state().viewport_gpu().unwrap().gpu().target().size();
    let pixels = fs::read(dir.join(format!("{name}.rgba"))).unwrap();
    assert_eq!(h.state().editor.camera, Camera::new([0., 80.], 160.));
    let scale = f64::from(size.0.min(size.1)) / 320.;
    assert!(
        scale > 1.,
        "oracle requires more than one pixel per atlas texel"
    );
    for row in 0..4 {
        for col in 0..7 {
            for y in 0..16 {
                for x in 0..16 {
                    let px = (f64::from(size.0) / 2. + (-55.5 + (col * 16 + x) as f64) * scale)
                        .floor() as usize;
                    let py = (f64::from(size.1) / 2. - (63.5 - (row * 16 + y) as f64) * scale)
                        .floor() as usize;
                    let index = (py * size.0 as usize + px) * 4;
                    let want = expected(REPEATED[row * 7 + col], x, y);
                    assert!(
                        pixels[index..index + 4]
                            .iter()
                            .zip(want)
                            .all(|(a, b)| a.abs_diff(b) <= 3),
                        "actual App Play literal cell ({col},{row}) texel ({x},{y})"
                    );
                }
            }
        }
    }
}

#[test]
#[ignore = "explicit actual EditorApp Copy/Stamp GPU acceptance; ORR_REQUIRE_GPU=1"]
fn actual_copy_stamp_save_reopen_play_literal_cells() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let evidence = PathBuf::from(
        std::env::var_os("ORR_TILEMAP_COPY_STAMP_EVIDENCE")
            .expect("fresh retained Copy/Stamp evidence directory required"),
    );
    assert!(evidence.is_absolute() && evidence.is_dir());
    assert!(
        fs::read_dir(&evidence).unwrap().next().is_none(),
        "evidence directory must be fresh"
    );
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("created");
    orr_sample::project_create::create_tilemap(&orr_sample::project_create::CreateOptions {
        output: root.clone(),
        template: "arena-tilemap-2d-v1".into(),
        seed: "copy-stamp-acceptance".into(),
    })
    .unwrap();
    let mut map: orr_tilemap::Document =
        serde_json::from_slice(&fs::read(root.join("arena.tilemap.json")).unwrap()).unwrap();
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
    click(&mut h, "hero");
    let selection = h.state().editor.selected_guids().to_vec();
    assert!(!selection.is_empty());
    let initial = capture(&mut h, &evidence, "01-before", &mut captures).to_bytes();
    let actor_before = actor_oracle(&h, &evidence.join("01-before-app.png"));
    let scene_history = h.state().editor.history().clone();
    let key = ("sample-sprites".into(), "sprites.json".into());
    let (asset, texture) = h.state().sprites.tilemap_asset(&key).unwrap();
    let atlas_rgba = asset.rgba.clone();
    let atlas_document = asset.document.to_json().unwrap();
    let atlas_texture = texture;
    let initial_history = h.state().tilemap.as_ref().unwrap().history_snapshot();
    click(&mut h, "Tile tool (owns viewport clicks)");
    click(&mut h, "Copy");
    rectangle(&mut h, (2, 1), (0, 0));
    assert_eq!(
        capture(&mut h, &evidence, "02-copied", &mut captures).to_bytes(),
        initial
    );
    document(&h, &map, &BEFORE);
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        initial_history
    );
    click(&mut h, "Stamp");
    let p = point(&h, 4, 2);
    button(&mut h, p, true);
    // Commit occurs on one primary press, never release or continued drag.
    document(&h, &map, &STAMPED);
    let stamped_history = h.state().tilemap.as_ref().unwrap().history_snapshot();
    assert_eq!(stamped_history.2.len(), initial_history.2.len() + 1);
    let drag = point(&h, 0, 2);
    move_to(&mut h, drag);
    button(&mut h, drag, false);
    assert_eq!(
        capture(&mut h, &evidence, "03-stamped", &mut captures).to_bytes(),
        initial
    );
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        stamped_history
    );
    document(&h, &map, &STAMPED);
    click(&mut h, "Undo tile");
    let undone = h.state().tilemap.as_ref().unwrap().history_snapshot();
    document(&h, &map, &BEFORE);
    stamp(&mut h, (0, 0)); // exact no-op preserves redo and saved marker
    assert_eq!(
        capture(&mut h, &evidence, "04-noop-keeps-redo", &mut captures).to_bytes(),
        initial
    );
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        undone
    );
    click(&mut h, "Redo tile");
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        stamped_history
    );
    stamp(&mut h, (1, 1)); // overlaps and changes the original copy source
    assert_eq!(
        capture(&mut h, &evidence, "05-overlap", &mut captures).to_bytes(),
        initial
    );
    document(&h, &map, &OVERLAPPED);
    let overlapped_history = h.state().tilemap.as_ref().unwrap().history_snapshot();
    assert_eq!(overlapped_history.2.len(), stamped_history.2.len() + 1);
    click(&mut h, "Undo tile");
    document(&h, &map, &STAMPED);
    click(&mut h, "Redo tile");
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        overlapped_history
    );
    stamp(&mut h, (0, 2)); // must use immutable original 3x2, not changed source
    assert_eq!(
        capture(&mut h, &evidence, "06-reused-snapshot", &mut captures).to_bytes(),
        initial
    );
    document(&h, &map, &REPEATED);
    let final_history = h.state().tilemap.as_ref().unwrap().history_snapshot();
    assert_eq!(final_history.2.len(), overlapped_history.2.len() + 1);
    stamp(&mut h, (5, 3)); // whole-pattern rejection, never clipping
    assert_eq!(
        capture(&mut h, &evidence, "07-outside-rejected", &mut captures).to_bytes(),
        initial
    );
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        final_history
    );
    // A real foreground widget blocks the primary press entirely.
    let blocked = point(&h, 2, 0);
    h.state_mut().obstruction = Some(egui::Rect::from_center_size(blocked, egui::vec2(100., 80.)));
    drain(&mut h, 3);
    assert_eq!(
        h.ctx.layer_id_at(blocked).unwrap().order,
        egui::Order::Foreground
    );
    button(&mut h, blocked, true);
    button(&mut h, blocked, false);
    assert_eq!(
        capture(&mut h, &evidence, "08-obstructed", &mut captures).to_bytes(),
        initial
    );
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        final_history
    );
    h.state_mut().obstruction = None;
    drain(&mut h, 3);
    assert_eq!(
        h.state().editor.selected_guids(),
        &selection,
        "Copy, Stamp, history and blocked presses preserve the selected actor"
    );
    click(&mut h, "Copy");
    let end = begin(&mut h, (3, 0), (5, 2));
    h.key_press(egui::Key::Escape);
    drain(&mut h, 2);
    button(&mut h, end, false);
    assert_eq!(
        capture(&mut h, &evidence, "09-copy-cancelled", &mut captures).to_bytes(),
        initial
    );
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        final_history
    );
    // Tile rectangles are not Editor scene-edit gestures. The existing global
    // Escape shortcut therefore clears GUID selection while cancelling this
    // rectangle. This is independent of successful Copy/Stamp pointer input.
    assert!(h.state().editor.selected_guids().is_empty());
    click(&mut h, "hero");
    assert_eq!(h.state().editor.selected_guids(), &selection);
    click(&mut h, "Stamp");
    stamp(&mut h, (0, 2)); // cancelled replacement did not destroy original pattern
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        final_history
    );
    assert_eq!(h.state().editor.history(), &scene_history);
    assert_eq!(h.state().editor.selected_guids(), &selection);
    assert!(h.state().ui.body_drag.is_none());
    let (asset, texture) = h.state().sprites.tilemap_asset(&key).unwrap();
    assert_eq!(asset.rgba, atlas_rgba);
    assert_eq!(asset.document.to_json().unwrap(), atlas_document);
    assert_eq!(texture, atlas_texture);
    assert_eq!(
        files(&root),
        source_files,
        "no autosave or actor/package mutation"
    );
    click(&mut h, "Save tilemap");
    assert_eq!(
        capture(&mut h, &evidence, "10-saved", &mut captures).to_bytes(),
        initial
    );
    assert!(!h.state().tilemap.as_ref().unwrap().dirty());
    assert_eq!(
        actor_oracle(&h, &evidence.join("10-saved-app.png")),
        actor_before
    );
    let saved_bytes = fs::read(root.join("arena.tilemap.json")).unwrap();
    let mut expected_map = map.clone();
    expected_map.cells = REPEATED.to_vec();
    assert_eq!(
        serde_json::from_slice::<orr_tilemap::Document>(&saved_bytes).unwrap(),
        expected_map
    );
    fs::write(evidence.join("saved.tilemap.json"), &saved_bytes).unwrap();
    let mut expected_files = source_files.clone();
    expected_files.insert(PathBuf::from("arena.tilemap.json"), saved_bytes.clone());
    assert_eq!(files(&root), expected_files);
    drop(h);
    let mut reopened = open(&root);
    assert_eq!(
        capture(&mut reopened, &evidence, "11-fresh-reopen", &mut captures).to_bytes(),
        initial
    );
    document(&reopened, &map, &REPEATED);
    assert!(!reopened.state().tilemap.as_ref().unwrap().dirty());
    assert!(reopened
        .state()
        .tilemap
        .as_ref()
        .unwrap()
        .history_snapshot()
        .2
        .is_empty());
    click(&mut reopened, LBL_STEP);
    reopened.state_mut().sprites.playback.suspend_follow();
    reopened.state_mut().editor.camera = Camera::new([0., 80.], 160.);
    drain(&mut reopened, 3);
    let play = capture(&mut reopened, &evidence, "12-play", &mut captures);
    assert_eq!(reopened.state().editor.mode(), Mode::Play);
    assert_eq!(play.tick(), 1);
    document(&reopened, &map, &REPEATED);
    app_play_oracle(&reopened, &evidence, "12-play");
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
        "13-runtime-saved",
        &REPEATED,
        &mut captures,
    );
    assert_eq!(
        play.to_bytes(),
        good_frame,
        "App Play and saved runtime same-tick full Frame bytes"
    );
    let mut bare = PreparedRuntime::open(&root).unwrap().bridge().unwrap();
    bare.step(1);
    let bare_frame = bare.host().session().frame().to_bytes();
    fs::write(evidence.join("presentation-free.frame"), &bare_frame).unwrap();
    retained_frame(
        &evidence.join("presentation-free.frame"),
        bare.host().session().frame(),
    );
    assert_eq!(bare_frame, good_frame);
    drop(reopened);
    expected_map.cells = WRONG_STRIDE.to_vec();
    fs::write(
        root.join("arena.tilemap.json"),
        serde_json::to_vec(&expected_map).unwrap(),
    )
    .unwrap();
    let (wrong_pixels, wrong_frame) = consume_saved(
        &root,
        &evidence,
        &gpu,
        "14-runtime-wrong-stride",
        &WRONG_STRIDE,
        &mut captures,
    );
    assert_eq!(wrong_frame, good_frame);
    let wrong_stride_visible_pixels = visible_difference(&wrong_pixels, &good_pixels, 1, 2);
    assert!(wrong_stride_visible_pixels >= 8);
    fs::write(root.join("arena.tilemap.json"), saved_bytes).unwrap();
    assert_eq!(files(&root), expected_files);
    assert_eq!(captures.len(), 14);
    let file_bindings: Vec<_> = source_files
        .iter()
        .map(|(path, bytes)| json!({"path":path,"fnv1a64":fingerprint(bytes),"bytes":bytes.len()}))
        .collect();
    write_json(
        &evidence.join("receipt.json"),
        &json!({
            "completed":true,"capture_count":captures.len(),"captures":captures,
            "literal_before":BEFORE,"literal_stamped":STAMPED,"literal_overlap":OVERLAPPED,
            "literal_cells":REPEATED,"literal_wrong_stride":WRONG_STRIDE,
            "runtime_oracle_pixels_per_capture":7168,"actual_app_play_oracle_texels":7168,
            "wrong_stride_visible_pixels":wrong_stride_visible_pixels,
            "frame_parity":"authoring unchanged full Frame bytes; actual App Play, saved runtime, wrong-stride and presentation-free same-tick full Frame bytes equal",
            "source_rgba_fnv1a64":fingerprint(SOURCE),"fixture_file_bindings":file_bindings,
            "test_source_fnv1a64":fingerprint(include_bytes!("tilemap_copy_stamp_tests.rs")),
            "panel_source_fnv1a64":fingerprint(include_bytes!("../tilemap_panel.rs")),
            "core_source_fnv1a64":fingerprint(include_bytes!("../../../orr_tilemap/src/lib.rs")),
            "original_atlas_zero_id_present":false,"native_os_input":false,"native_window":false,
            "selected_actor_opaque_texels_checked":actor_before.len(),"predictable_texture_filtering":false,"export":false,"windows":false
        }),
    );
}
