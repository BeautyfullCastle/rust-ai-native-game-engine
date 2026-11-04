//! Actual headless EditorApp widgets and egui pointer input for a new custom package.
//! Not native desktop input. Retains the created project and independent pixel evidence.
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use super::*;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_render::Camera;
use orr_sample::project_runtime::PreparedRuntime;
use serde_json::json;
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};
const SOURCE: &[u8] = include_bytes!("../../../../assets/sprite_demo/lantern_keeper.rgba");

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
    h.get_by_label(label).scroll_to_me();
    drain(h, 3);
    assert!(
        !h.get_by_label(label).accesskit_node().is_disabled(),
        "disabled widget: {label}"
    );
    h.get_by_label(label).click_accesskit();
    drain(h, 3);
}
fn open(root: &Path) -> Harness<'static, EditorApp> {
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
        .agent_client("custom-tileset-authority-probe")
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
        "custom tileset adapter={} backend={:?} type={:?}",
        info.name, info.backend, info.device_type
    );
    frame
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
fn actor_oracle(h: &Harness<'_, EditorApp>, path: &Path) -> Vec<[u8; 4]> {
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

// Deliberately asymmetric, opaque colors. ID zero is a real tile, not empty.
fn custom_color(id: u32, x: usize, y: usize) -> [u8; 4] {
    match (id, x, y) {
        (0, 0..=1, _) => [241, 37, 79, 255],
        (0, _, 0..=2) => [31, 211, 137, 255],
        (0, _, _) => [37, 73, 229, 255],
        (73, _, 9..=11) => [233, 191, 29, 255],
        (73, 5..=7, _) => [173, 41, 211, 255],
        (73, _, _) => [23, 193, 223, 255],
        _ => panic!("fixture region"),
    }
}
fn package(root: &Path) {
    fs::create_dir(root).unwrap();
    write_json(
        &root.join("orr.package.json"),
        &json!({
            "schema":1,"name":"acceptance-custom-tiles","version":"1.0.0",
            "engine":"^0.0.1","capabilities":["sprite"],"dependencies":{},
            "files":["sprites.json","tiles.png","LICENSE.txt"]
        }),
    );
    write_json(
        &root.join("sprites.json"),
        &json!({
            "format":"orr_sprite","version":1,
            "atlas":{"image":"tiles.png","width":16,"height":12},
            "regions":[{"id":0,"x":0,"y":0,"width":8,"height":12},
                       {"id":73,"x":8,"y":0,"width":8,"height":12}],"clips":[]
        }),
    );
    fs::write(
        root.join("LICENSE.txt"),
        "SPDX-License-Identifier: MIT\nOriginal acceptance fixture colors.\n",
    )
    .unwrap();
    let mut pixels = Vec::new();
    for y in 0..12 {
        for x in 0..16 {
            pixels.extend_from_slice(&custom_color(if x < 8 { 0 } else { 73 }, x % 8, y));
        }
    }
    png(&root.join("tiles.png"), (16, 12), &pixels);
}
fn paint(h: &mut Harness<'_, EditorApp>, column: u32, row: u32) {
    let app = h.state();
    let rect = app.ui.viewport_rect.unwrap();
    let px = app.ui.viewport_px;
    let doc = app.tilemap.as_ref().unwrap().document().document();
    let world = [
        doc.origin[0] + (column as f32 + 0.5) * 8.,
        doc.origin[1] - (row as f32 + 0.5) * 12.,
    ];
    let screen = app.editor.camera.world_to_screen(world, px);
    let p = rect.min
        + egui::vec2(
            screen[0] * rect.width() / px.0 as f32,
            screen[1] * rect.height() / px.1 as f32,
        );
    assert!(rect.contains(p));
    button(h, p, true);
    button(h, p, false);
}
fn custom_play_oracle(h: &Harness<'_, EditorApp>, evidence: &Path) {
    let size = h.state().viewport_gpu().unwrap().gpu().target().size();
    let pixels = fs::read(evidence.join("06-play.rgba")).unwrap();
    assert_eq!(h.state().editor.camera, Camera::new([0., 80.], 160.));
    let scale = f64::from(size.0.min(size.1)) / 320.;
    assert!(scale > 1.);
    // Literal geometry, independent of loaded map metadata and UV/extraction helpers.
    for (col, id) in [(0, 0), (1, 73)] {
        for y in 0..12 {
            for x in 0..8 {
                let world_x = -64. + (col * 8 + x) as f64 + 0.5;
                let world_y = 72. - y as f64 - 0.5;
                let px = (f64::from(size.0) / 2. + world_x * scale).floor() as usize;
                let py = (f64::from(size.1) / 2. - (world_y - 80.) * scale).floor() as usize;
                let i = (py * size.0 as usize + px) * 4;
                assert!(
                    pixels[i..i + 4]
                        .iter()
                        .zip(custom_color(id, x, y))
                        .all(|(a, b)| a.abs_diff(b) <= 3),
                    "custom region {id} original texel ({x},{y})"
                );
            }
        }
    }
}

#[test]
#[ignore = "explicit actual EditorApp custom tileset GPU acceptance; ORR_REQUIRE_GPU=1"]
fn actual_custom_tileset_palette_paint_undo_save_reopen_play() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let evidence = PathBuf::from(
        std::env::var_os("ORR_CUSTOM_TILESET_EVIDENCE")
            .expect("fresh retained custom tileset evidence directory required"),
    );
    assert!(evidence.is_absolute() && evidence.is_dir());
    assert!(fs::read_dir(&evidence).unwrap().next().is_none());
    let source = evidence.join("source-package");
    package(&source);
    let source_before = files(&source);
    let root = evidence.join("created-project");
    orr_sample::project_create::create_custom_tilemap(
        &orr_sample::project_create::CustomTilemapOptions {
            base: orr_sample::project_create::CreateOptions {
                output: root.clone(),
                template: "arena-custom-tileset-2d-v1".into(),
                seed: "custom-tileset-editor-acceptance".into(),
            },
            package: source.clone(),
            tile_size: [8, 12],
        },
    )
    .unwrap();
    assert_eq!(files(&source), source_before);
    let initial_files = files(&root);
    let original: orr_tilemap::Document =
        serde_json::from_slice(&initial_files[&PathBuf::from("arena.tilemap.json")]).unwrap();
    assert_eq!(
        (original.width, original.height, original.tile_size),
        (16, 12, [8, 12])
    );
    assert_eq!(original.origin, [-64., 72.]);
    assert_eq!(original.units_per_pixel, 1.);
    assert_eq!(original.sprite.package, "acceptance-custom-tiles");
    assert_eq!(original.sprite.document, "sprites.json");
    assert_eq!(original.cells, vec![None; 192]);
    let mut captures = Vec::new();
    let mut h = open(&root);
    click(&mut h, "hero");
    let selection = h.state().editor.selected_guids().to_vec();
    assert!(!selection.is_empty());
    let scene_history = h.state().editor.history().clone();
    let initial = capture(&mut h, &evidence, "01-empty", &mut captures).to_bytes();
    let actor = actor_oracle(&h, &evidence.join("01-empty-app.png"));
    let key = ("acceptance-custom-tiles".into(), "sprites.json".into());
    let (asset, texture) = h.state().sprites.tilemap_asset(&key).unwrap();
    let atlas = asset.rgba.clone();
    assert_eq!(asset.document.regions().len(), 2);
    click(&mut h, "Tile tool (owns viewport clicks)");
    click(&mut h, "Static region 0");
    paint(&mut h, 0, 0);
    click(&mut h, "Static region 73");
    paint(&mut h, 1, 0);
    let mut expected = original.clone();
    expected.cells[0] = Some(0);
    expected.cells[1] = Some(73);
    assert_eq!(cells(&h), expected.cells);
    assert!(h.state().tilemap.as_ref().unwrap().dirty());
    let painted = h.state().tilemap.as_ref().unwrap().history_snapshot();
    assert_eq!(painted.2.len(), 2);
    assert_eq!(
        capture(&mut h, &evidence, "02-painted", &mut captures).to_bytes(),
        initial
    );
    click(&mut h, "Undo tile");
    let mut one = original.cells.clone();
    one[0] = Some(0);
    assert_eq!(cells(&h), one);
    click(&mut h, "Undo tile");
    assert_eq!(cells(&h), original.cells);
    assert!(!h.state().tilemap.as_ref().unwrap().dirty());
    assert_eq!(
        capture(&mut h, &evidence, "03-undone", &mut captures).to_bytes(),
        initial
    );
    click(&mut h, "Redo tile");
    click(&mut h, "Redo tile");
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        painted
    );
    assert_eq!(h.state().editor.selected_guids(), &selection);
    assert_eq!(h.state().editor.history(), &scene_history);
    assert!(h.state().ui.body_drag.is_none());
    let (asset, same_texture) = h.state().sprites.tilemap_asset(&key).unwrap();
    assert_eq!(asset.rgba, atlas);
    assert_eq!(same_texture, texture);
    assert_eq!(
        files(&root),
        initial_files,
        "no autosave or actor/package mutation"
    );
    click(&mut h, "Save tilemap");
    assert_eq!(
        capture(&mut h, &evidence, "04-saved", &mut captures).to_bytes(),
        initial
    );
    assert_eq!(actor_oracle(&h, &evidence.join("04-saved-app.png")), actor);
    assert!(!h.state().tilemap.as_ref().unwrap().dirty());
    let saved = fs::read(root.join("arena.tilemap.json")).unwrap();
    assert_eq!(
        serde_json::from_slice::<orr_tilemap::Document>(&saved).unwrap(),
        expected
    );
    let mut expected_files = initial_files;
    expected_files.insert("arena.tilemap.json".into(), saved);
    assert_eq!(files(&root), expected_files);
    drop(h);
    // The original source path is unavailable before fresh admission and Play.
    fs::rename(&source, evidence.join("source-package-hidden")).unwrap();
    assert!(!source.exists());
    let mut h = open(&root);
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().document().document(),
        &expected
    );
    assert!(!h.state().tilemap.as_ref().unwrap().dirty());
    assert!(h
        .state()
        .tilemap
        .as_ref()
        .unwrap()
        .history_snapshot()
        .2
        .is_empty());
    assert_eq!(
        capture(&mut h, &evidence, "05-reopened", &mut captures).to_bytes(),
        initial
    );
    assert_eq!(
        actor_oracle(&h, &evidence.join("05-reopened-app.png")),
        actor
    );
    click(&mut h, LBL_STEP);
    h.state_mut().sprites.playback.suspend_follow();
    h.state_mut().editor.camera = Camera::new([0., 80.], 160.);
    drain(&mut h, 3);
    let play = capture(&mut h, &evidence, "06-play", &mut captures);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    assert_eq!(play.tick(), 1);
    custom_play_oracle(&h, &evidence);
    let mut runtime = PreparedRuntime::open(&root).unwrap().bridge().unwrap();
    runtime.step(1);
    let runtime_frame = runtime.host().session().frame();
    fs::write(
        evidence.join("presentation-free.frame"),
        runtime_frame.to_bytes(),
    )
    .unwrap();
    retained_frame(&evidence.join("presentation-free.frame"), runtime_frame);
    assert_eq!(
        play.to_bytes(),
        runtime_frame.to_bytes(),
        "same-tick actual App Play and presentation-free authoritative Frame bytes"
    );
    assert_eq!(files(&root), expected_files);
    write_json(
        &evidence.join("receipt.json"),
        &json!({
            "completed":true,"captures":captures,"created_project":root,
            "source_hidden_before_reopen":true,"tile_size":[8,12],"painted_regions":[0,73],
            "actual_app_play_custom_texels_checked":192,"actor_opaque_texels_checked":actor.len(),
            "frame_parity":"authoring full bytes unchanged; App Play and independent runtime same-tick full Frame bytes equal",
            "test_source_fnv1a64":fingerprint(include_bytes!("custom_tileset_tests.rs")),
            "native_os_input":false,"native_window":false,"windows":false,"export":false
        }),
    );
}
