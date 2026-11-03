//! Actual sequence widgets -> exported JSON -> built Rust producer -> fresh installed
//! EditorApp -> Save/reopen/real Play -> composed framebuffer source-pixel oracle.
#![cfg(all(feature = "sprites", target_os = "linux"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
#[path = "common/arena_sprites.rs"]
mod common;
use common::{body_position, document, no_error, settle, wait_for, HERO};
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_editor::{
    app::{LBL_PLAY, LBL_STOP},
    game::EditorGame,
    sprite_bindings::Source,
    Editor, EditorApp,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
const PACKAGE: &str = "sequence-fixture";
const SCENE: &str = "schema: orr.scene/1\nsingletons:\n  Score: { kills: [0,0,0,0,0,0,0,0] }\nentities:\n  e_00000001:\n    name: hero\n    Position: { pos: [0,0] }\n    PlayerTag: { slot: 0 }\n";
// Three distinct asymmetric 2x2 cells in scattered, non-grid atlas rectangles.
// Expected colors and center-pivot locations never come from exported JSON,
// renderer UVs, sampled-region APIs, or previous captures.
const CELLS: [[[u8; 4]; 4]; 3] = [
    [
        [233, 31, 19, 255],
        [19, 211, 47, 255],
        [27, 61, 229, 255],
        [241, 181, 29, 255],
    ],
    [
        [19, 211, 47, 255],
        [241, 181, 29, 255],
        [233, 31, 19, 255],
        [27, 61, 229, 255],
    ],
    [
        [27, 61, 229, 255],
        [233, 31, 19, 255],
        [241, 181, 29, 255],
        [19, 211, 47, 255],
    ],
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
fn inventory(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, at: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}
fn app(scene: &Path, evidence: &Path, label: &str) -> Harness<'static, EditorApp> {
    let mut editor = Editor::open_game(scene, EditorGame::Arena).unwrap();
    editor.sync();
    editor.camera = orr_render::Camera::new([0., 0.], 10.);
    let evidence = evidence.to_owned();
    let label = label.to_owned();
    Harness::builder()
        .with_size([1500., 1400.])
        .with_step_dt(1. / 60.)
        .with_render_options(egui_wgpu::RendererOptions::default())
        .wgpu()
        .build_eframe(move |cc| {
            let info = cc
                .wgpu_render_state
                .as_ref()
                .expect("mandatory composed GPU")
                .adapter
                .get_info();
            eprintln!(
                "sequence {label} adapter: {} ({:?}, {:?})",
                info.name, info.device_type, info.backend
            );
            fs::write(
                evidence.join(format!("{label}-adapter.json")),
                serde_json::to_vec_pretty(&json!({
                    "name":info.name,"backend":format!("{:?}",info.backend),
                    "device_type":format!("{:?}",info.device_type),
                    "native_window":false,"physical_input":false
                }))
                .unwrap(),
            )
            .unwrap();
            EditorApp::new(editor, cc.wgpu_render_state.clone())
        })
}
fn elapsed_ms(app: &EditorApp) -> u64 {
    app.editor.snapshot().map_or(0, |snapshot| {
        snapshot.timeline().map_or(0, |timeline| {
            timeline.tick.saturating_mul(1000) / u64::from(snapshot.tick_rate())
        })
    })
}
// Read the unchanged authoritative Frame, not just a scene or sidecar hash.
fn frame_bytes(editor: &mut Editor) -> Vec<u8> {
    editor.sync();
    let snapshot = editor.snapshot().unwrap();
    let tick = snapshot.tick();
    let checksum = snapshot.predicted().checksum();
    let client = editor.agent_client("clip-sequence-frame-proof").unwrap();
    client.local_frames.clear();
    client
        .call(
            "watch.subscribe",
            json!({"topics":["frames"],"source":"view","max_fps":60}),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        client.poll().unwrap();
        while let Some(frame) = client.local_frames.pop_front() {
            if frame.frame.tick() == tick && frame.frame.checksum() == checksum {
                return frame.frame.to_bytes();
            }
        }
        assert!(
            Instant::now() < deadline,
            "matching authoritative Frame missing"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
// Scroll layout and pointer hit-testing use different passes. Require the
// actual, unclipped widget hit before sending a single geometric click; waiting
// never retries an action or dispatches an accessibility Click instead.
fn pointer_ready(
    h: &mut Harness<'_, EditorApp>,
    role: Role,
    label: &str,
) -> egui::accesskit::NodeId {
    h.get_by_role_and_label(role, label).scroll_to_me();
    settle(h);
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut previous = None;
    loop {
        let node = h.get_by_role_and_label(role, label);
        let rect = node.rect();
        let target = node.accesskit_node().locate().0;
        node.hover();
        h.run_steps(2);
        let node = h.get_by_role_and_label(role, label);
        let current = node.rect();
        let disabled = node.accesskit_node().is_disabled();
        let hovered = h.ctx.interaction_snapshot(|interaction| {
            interaction.hovered.iter().copied().collect::<Vec<_>>()
        });
        let response = hovered
            .iter()
            .find(|id| id.accesskit_id() == target)
            .and_then(|id| h.ctx.read_response(*id));
        let ready = !disabled
            && node.accesskit_node().locate().0 == target
            && previous == Some(current)
            && current == rect
            && h.ctx.content_rect().contains(current.center())
            && response.as_ref().is_some_and(|response| {
                response.enabled() && response.interact_rect.contains(current.center())
            });
        if ready {
            return target;
        }
        if Instant::now() >= deadline {
            let clipped = response.as_ref().map(|response| response.interact_rect);
            let hovered: Vec<_> = hovered.iter().map(egui::Id::accesskit_id).collect();
            if let Ok(image) = h.render() {
                let evidence =
                    PathBuf::from(std::env::var_os("ORR_SPRITE_SEQUENCE_EVIDENCE").unwrap());
                png(
                    &evidence.join("failed-pointer.png"),
                    image.width(),
                    image.height(),
                    image.as_raw(),
                );
            }
            panic!("pointer target {label:?} not stable/reachable: target={target:?}, previous={previous:?}, rect={rect:?}, current={current:?}, disabled={disabled}, clipped={clipped:?}, hovered={hovered:?}");
        }
        previous = Some(current);
        if response.is_none() {
            h.get_by_role_and_label(role, label).scroll_to_me();
        }
        h.run_steps(1);
    }
}
fn click_role(h: &mut Harness<'_, EditorApp>, role: Role, label: &str) {
    let target = pointer_ready(h, role, label);
    h.get_by_role_and_label(role, label).click();
    h.step();
    let clicked = h
        .ctx
        .interaction_snapshot(|interaction| interaction.clicked.map(|id| id.accesskit_id()));
    assert_eq!(
        clicked,
        Some(target),
        "geometric click must dispatch exactly once to {label:?}"
    );
    settle(h);
}
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    click_role(h, Role::Button, label);
}
fn text(h: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    settle(h);
    h.get_by_role_and_label(Role::TextInput, label).focus();
    settle(h);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    settle(h);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    settle(h);
}
fn choose(h: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    click_role(h, Role::ComboBox, label);
    click(h, value);
}
fn configure(h: &mut Harness<'_, EditorApp>, sidecar: &Path, source: &str) {
    settle(h);
    click(h, "hero");
    click(h, "Sprite bindings (view only)");
    {
        let p = &mut h.state_mut().sprites;
        p.path = sidecar.to_string_lossy().into();
        p.scene = "arena.scene.yaml".into();
        p.project = ".".into();
    }
    click(h, "Create bindings");
    no_error(h);
    {
        let p = &mut h.state_mut().sprites;
        p.package = PACKAGE.into();
        p.document = "sprites.json".into();
        p.scale = 2.;
    }
    click(h, "Load sprite document");
    no_error(h);
    choose(h, "Sprite source", source);
    click(h, "Assign sprite to selection");
    click(h, "Save bindings");
    no_error(h);
}
fn producer(
    binary: &Path,
    source: &Path,
    recipe: &Path,
    output: &Path,
    evidence: &Path,
    label: &str,
) {
    assert!(
        !output.exists(),
        "producer requires a genuinely new destination"
    );
    let result = Command::new(binary)
        .arg("create-sprite")
        .arg("--png")
        .arg(source.join("atlas.png"))
        .arg("--license")
        .arg(source.join("LICENSE.txt"))
        .args(["--name", PACKAGE, "--version", "1.0.0"])
        .arg("--out")
        .arg(output)
        .arg("--json")
        .arg(recipe)
        .output()
        .unwrap();
    fs::write(
        evidence.join(format!("{label}.log")),
        [result.stdout.as_slice(), result.stderr.as_slice()].concat(),
    )
    .unwrap();
    assert!(
        result.status.success(),
        "producer failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}
fn install(project: &Path, package: &Path) {
    fs::create_dir(project).unwrap();
    // Capabilities are selected by the compiled consumer, never by package data.
    orr_package::Project::open(project, orr_sample::project_runtime::compiled_runtime())
        .unwrap()
        .install(&[package.to_owned()])
        .unwrap();
    fs::write(project.join("arena.scene.yaml"), SCENE).unwrap();
}
// Observe the real admitted texture through egui's existing read-only metadata;
// no test-only production API and no private-draft injection are needed.
fn atlas_texture(
    h: &Harness<'_, EditorApp>,
) -> (egui::TextureId, egui::epaint::textures::TextureMeta) {
    let manager = h.ctx.tex_manager();
    let manager = manager.read();
    let textures: Vec<_> = manager
        .allocated()
        .filter(|(_, meta)| meta.name == format!("sprite:{PACKAGE}:sprites.json"))
        .map(|(id, meta)| (*id, meta.clone()))
        .collect();
    assert_eq!(textures.len(), 1, "one admitted, unchanged atlas texture");
    assert_eq!(textures[0].1.options, egui::TextureOptions::NEAREST);
    textures.into_iter().next().unwrap()
}
fn capture(
    h: &mut Harness<'_, EditorApp>,
    evidence: &Path,
    label: &str,
    expected: usize,
    records: &mut Vec<Value>,
) -> Vec<[u8; 3]> {
    h.event(egui::Event::PointerMoved(egui::pos2(2., 2.)));
    settle(h);
    let elapsed = elapsed_ms(h.state());
    let image = h
        .render()
        .expect("mandatory actual composed EditorApp framebuffer");
    let (width, height) = (image.width(), image.height());
    let rgba = image.into_raw();
    let rect = h.state().ui.viewport_rect.unwrap();
    let pixels = h.state().ui.viewport_px;
    let body = body_position(h.state(), HERO);
    let mut samples = Vec::new();
    // Each source texel is 2 world units wide, with the 2x2 sprite centered on
    // the actor. Off-center probes on both sides of each texel edge reject
    // bilinear filtering as well as wrong UVs, top-left anchoring, or flips.
    for y in 0..4 {
        for x in 0..4 {
            let at = h
                .state()
                .editor
                .camera
                .world_to_screen([body[0] + x as f32 - 1.5, body[1] + 1.5 - y as f32], pixels);
            let sx = (rect.min.x + at[0] * rect.width() / pixels.0 as f32).floor() as u32;
            let sy = (rect.min.y + at[1] * rect.height() / pixels.1 as f32).floor() as u32;
            assert!(sx < width && sy < height);
            let offset = ((sy * width + sx) * 4) as usize;
            let sample: [u8; 3] = rgba[offset..offset + 3].try_into().unwrap();
            let expected_rgba = CELLS[expected][(y / 2) * 2 + x / 2];
            assert!(
                sample
                    .iter()
                    .zip(expected_rgba)
                    .all(|(a, e)| a.abs_diff(e) <= 3),
                "{label}: cell {expected}, probe {x},{y}: {sample:?}, expected {expected_rgba:?}"
            );
            samples.push(sample);
        }
    }
    png(&evidence.join(format!("{label}.png")), width, height, &rgba);
    records.push(
        json!({"file":format!("{label}.png"),"expected_region":([10,20,30][expected]),
        "elapsed_ms":elapsed,"center_pivot_world":body,"samples":samples}),
    );
    samples
}
fn append(h: &mut Harness<'_, EditorApp>, region: u32) {
    choose(
        h,
        "Sequence append region",
        &format!("Sequence region {region}"),
    );
    click(h, "Append sequence frame");
    no_error(h);
}
fn select_frame(h: &mut Harness<'_, EditorApp>, index: usize, region: u32) {
    click(h, &format!("Sequence frame {index} (region {region})"));
}
fn duration(h: &mut Harness<'_, EditorApp>, value: &str) {
    text(h, "Selected sequence frame duration ms", value);
    click(h, "Apply sequence frame duration");
}
fn pending_blocks_publication(h: &Harness<'_, EditorApp>) {
    for label in ["Preview clip sequence", "Export clip sequence JSON"] {
        assert!(
            h.get_by_role_and_label(Role::Button, label)
                .accesskit_node()
                .is_disabled(),
            "pending sequence text must block {label}"
        );
    }
}
fn click_export(h: &mut Harness<'_, EditorApp>, path: &Path) {
    h.get_by_role_and_label(Role::Button, "Export clip sequence JSON")
        .scroll_to_me();
    settle(h);
    assert_eq!(
        h.get_by_role_and_label(Role::TextInput, "Sequence output path")
            .value()
            .as_deref(),
        path.to_str(),
        "export must use the exact requested output path"
    );
    click(h, "Export clip sequence JSON");
}
fn export(h: &mut Harness<'_, EditorApp>, path: &Path) -> Value {
    assert!(!path.exists());
    text(h, "Sequence output path", path.to_str().unwrap());
    click_export(h, path);
    no_error(h);
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
fn exact_boundaries(exported: &Value) {
    let document = orr_sprite::SpriteDocument::from_json(&exported.to_string()).unwrap();
    let clip = document.clip("sequenced").unwrap();
    assert_eq!(clip.mode(), orr_sprite::PlaybackMode::Once);
    assert_eq!(clip.duration_ms(), 9500);
    // Literal expectations, independent of construction/export order. In
    // particular, 30 repeats after 10, and Once holds 20 rather than wrapping.
    for (ms, frame, region, finished) in [
        (0, 0, 30, false),
        (1999, 0, 30, false),
        (2000, 1, 10, false),
        (4499, 1, 10, false),
        (4500, 2, 30, false),
        (7499, 2, 30, false),
        (7500, 3, 20, false),
        (9499, 3, 20, false),
        (9500, 3, 20, true),
        (10000, 3, 20, true),
        (u64::MAX, 3, 20, true),
    ] {
        let sample = clip.sample(ms);
        assert_eq!(
            (sample.frame_index, sample.region, sample.finished),
            (frame, region, finished),
            "sample at {ms} ms"
        );
    }
}
#[test]
#[ignore = "mandatory explicit composed GPU and built Rust orr_pkg sprite-authoring gate; run --ignored --exact and require exactly one test"]
fn sequence_widgets_export_package_reopen_and_real_play_pixels() {
    assert_eq!(
        std::env::var("ORR_REQUIRE_GPU").as_deref(),
        Ok("1"),
        "mandatory GPU gate"
    );
    let binary =
        PathBuf::from(std::env::var_os("ORR_RUST_PACKAGE_BIN").expect("built orr_pkg required"));
    assert!(
        binary.is_absolute(),
        "producer must be an explicit absolute path"
    );
    let binary = binary.canonicalize().unwrap();
    assert!(binary.is_file());
    let evidence = PathBuf::from(
        std::env::var_os("ORR_SPRITE_SEQUENCE_EVIDENCE")
            .expect("fresh caller-created evidence directory required"),
    );
    assert!(evidence.is_absolute() && evidence.is_dir());
    assert_eq!(
        fs::read_dir(&evidence).unwrap().count(),
        0,
        "refuse stale evidence"
    );
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("source");
    fs::create_dir(&source).unwrap();
    // Three scattered rectangles deliberately disagree with both numeric ID
    // order and a row-major/even-cell importer. Purple gap pixels are not art.
    let mut rgba = vec![0u8; 11 * 9 * 4];
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel.copy_from_slice(&[127, 13, 149, 255]);
    }
    for (cell, left, top) in [(0, 7, 1), (1, 1, 6), (2, 2, 1)] {
        for y in 0..2 {
            for x in 0..2 {
                let offset = ((top + y) * 11 + left + x) * 4;
                rgba[offset..offset + 4].copy_from_slice(&CELLS[cell][y * 2 + x]);
            }
        }
    }
    png(&source.join("atlas.png"), 11, 9, &rgba);
    fs::write(
        source.join("LICENSE.txt"),
        "Synthetic test-owned art CC0-1.0\n",
    )
    .unwrap();
    let original = json!({"format":"orr_sprite","version":1,"atlas":{"image":"atlas.png","width":11,"height":9},
        "regions":[{"id":10,"x":7,"y":1,"width":2,"height":2},{"id":20,"x":1,"y":6,"width":2,"height":2},{"id":30,"x":2,"y":1,"width":2,"height":2}],"clips":[]});
    let recipe = source.join("original.json");
    fs::write(&recipe, serde_json::to_vec_pretty(&original).unwrap()).unwrap();
    let package = work.path().join("original-package");
    producer(
        &binary,
        &source,
        &recipe,
        &package,
        &evidence,
        "original-producer",
    );
    let project = work.path().join("original-project");
    install(&project, &package);
    let mut h = app(&project.join("arena.scene.yaml"), &evidence, "original");
    configure(&mut h, &project.join("arena.sprites.json"), "Region 10");
    let before = inventory(&project);
    let source_before = inventory(&source);
    let package_before = inventory(&package);
    let original_frame = frame_bytes(&mut h.state_mut().editor);
    let bindings = document(&h).clone();
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().entries.len();
    let texture = atlas_texture(&h);
    let mut records = Vec::new();
    let original_pixels = capture(&mut h, &evidence, "01-original-region-10", 0, &mut records);
    // Selecting Region 30 is only the authoring context. The bound actor stays
    // Region 10 until a different installed project is explicitly assigned.
    choose(&mut h, "Sprite source", "Region 30");
    click(&mut h, "Begin clip sequence");
    no_error(&h);
    text(&mut h, "Sequence clip ID", "sequenced");
    pending_blocks_publication(&h);
    click(&mut h, "Apply sequence ID");
    no_error(&h);
    click(&mut h, "Preview clip sequence");
    no_error(&h);
    assert_eq!(
        capture(
            &mut h,
            &evidence,
            "02-private-first-sequence",
            0,
            &mut records
        ),
        original_pixels
    );
    click(&mut h, "Stop clip sequence preview");
    let drafts = work.path().join("drafts");
    fs::create_dir(&drafts).unwrap();
    let initial = export(&mut h, &drafts.join("initial.json"));
    assert_eq!(
        initial["clips"],
        json!([{"id":"sequenced","mode":"loop","frames":[{"region":30,"duration_ms":100}]}])
    );
    // Actual enabled boundary actions must be harmless, as must identical ID
    // and hold edits. Refusing the final removal cannot empty the sequence.
    click(&mut h, "Move sequence frame up");
    click(&mut h, "Move sequence frame down");
    text(&mut h, "Sequence clip ID", "sequenced");
    click(&mut h, "Apply sequence ID");
    duration(&mut h, "100");
    no_error(&h);
    click(&mut h, "Remove sequence frame");
    assert_eq!(
        h.state().sprites.error(),
        Some("Cannot remove the last sequence frame")
    );
    assert_eq!(export(&mut h, &drafts.join("boundary-noop.json")), initial);
    // Invalid text never becomes an accepted zero/overflowing hold or empty
    // ID, nor may export silently publish old accepted values behind it.
    let blocked = drafts.join("blocked-invalid.json");
    text(&mut h, "Sequence output path", blocked.to_str().unwrap());
    for invalid in ["0", "18446744073709551616"] {
        duration(&mut h, invalid);
        assert!(h.state().sprites.error().is_some());
        pending_blocks_publication(&h);
        assert!(!blocked.exists());
        duration(&mut h, "100");
        no_error(&h);
    }
    for invalid in [" ".to_owned(), "é".repeat(65)] {
        text(&mut h, "Sequence clip ID", &invalid);
        click(&mut h, "Apply sequence ID");
        assert!(h.state().sprites.error().is_some());
        pending_blocks_publication(&h);
        assert!(!blocked.exists());
        text(&mut h, "Sequence clip ID", "sequenced");
        click(&mut h, "Apply sequence ID");
        no_error(&h);
    }
    // u64::MAX is a valid single hold, but appending even 100 ms must fail
    // atomically instead of overflowing the total or mutating frame order.
    duration(&mut h, "18446744073709551615");
    no_error(&h);
    choose(&mut h, "Sequence append region", "Sequence region 20");
    click(&mut h, "Append sequence frame");
    assert!(h.state().sprites.error().is_some());
    duration(&mut h, "100");
    no_error(&h);
    assert_eq!(
        export(&mut h, &drafts.join("invalid-no-mutation.json")),
        initial
    );
    assert_eq!(
        capture(
            &mut h,
            &evidence,
            "03-invalid-and-noop-no-mutation",
            0,
            &mut records
        ),
        original_pixels
    );
    append(&mut h, 20);
    append(&mut h, 10);
    select_frame(&mut h, 1, 20);
    click(&mut h, "Remove sequence frame");
    no_error(&h);
    append(&mut h, 20);
    append(&mut h, 30);
    // Set unequal holds before reordering. Moving a region without its hold
    // would fail the exact exported JSON and half-open boundary assertions.
    for (index, region, ms) in [
        (0, 30, "2000"),
        (1, 10, "2500"),
        (2, 20, "2000"),
        (3, 30, "3000"),
    ] {
        select_frame(&mut h, index, region);
        duration(&mut h, ms);
        no_error(&h);
    }
    click(&mut h, "Move sequence frame up");
    click(&mut h, "Move sequence frame down");
    click(&mut h, "Move sequence frame up");
    no_error(&h);
    for label in ["Sequence Once", "Sequence Loop", "Sequence Once"] {
        click_role(&mut h, Role::RadioButton, label);
    }
    click(&mut h, "Preview clip sequence");
    no_error(&h);
    assert_eq!(
        capture(
            &mut h,
            &evidence,
            "04-reordered-private-preview",
            0,
            &mut records
        ),
        original_pixels
    );
    click(&mut h, "Stop clip sequence preview");
    let existing = drafts.join("existing.json");
    let sentinel = b"caller-owned existing JSON must not be replaced\n";
    fs::write(&existing, sentinel).unwrap();
    text(&mut h, "Sequence output path", existing.to_str().unwrap());
    click_export(&mut h, &existing);
    assert!(h.state().sprites.error().is_some());
    assert_eq!(fs::read(&existing).unwrap(), sentinel);
    let exported = drafts.join("sequenced.json");
    let exported_json = export(&mut h, &exported);
    assert_eq!(exported_json["atlas"], original["atlas"]);
    assert_eq!(exported_json["regions"], original["regions"]);
    assert_eq!(original["clips"], json!([]));
    assert_eq!(
        exported_json["clips"],
        json!([{"id":"sequenced","mode":"once","frames":[{"region":30,"duration_ms":2000},{"region":10,"duration_ms":2500},{"region":30,"duration_ms":3000},{"region":20,"duration_ms":2000}]}])
    );
    exact_boundaries(&exported_json);
    let published = fs::read(&exported).unwrap();
    click_export(&mut h, &exported);
    assert!(h.state().sprites.error().is_some());
    assert_eq!(
        fs::read(&exported).unwrap(),
        published,
        "repeat export is create-only"
    );
    assert_eq!(document(&h), &bindings);
    assert_eq!(document(&h).bindings[HERO].source, Source::Region(10));
    assert!(!h.state().sprites.bindings.as_ref().unwrap().dirty());
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history().entries.len(), history);
    assert_eq!(frame_bytes(&mut h.state_mut().editor), original_frame);
    assert_eq!(atlas_texture(&h), texture);
    assert_eq!(inventory(&project), before);
    assert_eq!(inventory(&source), source_before);
    assert_eq!(inventory(&package), package_before);
    drop(h);
    let new_package = work.path().join("sequenced-package");
    producer(
        &binary,
        &source,
        &exported,
        &new_package,
        &evidence,
        "sequence-producer",
    );
    assert_eq!(
        inventory(&source),
        source_before,
        "same exact PNG and license"
    );
    assert_eq!(inventory(&package), package_before);
    let fresh = work.path().join("fresh-project");
    install(&fresh, &new_package);
    // Keep evidence bytes in memory until after the fresh consumer has run, so
    // there is no accessible copy of the authoring JSON at its original path.
    for path in [&source, &package, &new_package, &project, &drafts] {
        fs::rename(path, path.with_extension("hidden")).unwrap();
        assert!(!path.exists());
    }
    assert!(!exported.exists());
    let installed = inventory(&fresh.join(".orr"));
    let scene = fresh.join("arena.scene.yaml");
    let sidecar = fresh.join("arena.sprites.json");
    let mut h = app(&scene, &evidence, "assigned");
    configure(&mut h, &sidecar, "Clip sequenced");
    assert_eq!(
        document(&h).bindings[HERO].source,
        Source::Clip("sequenced".into())
    );
    let first = capture(
        &mut h,
        &evidence,
        "05-new-package-assigned-region-30",
        2,
        &mut records,
    );
    assert_ne!(first, original_pixels, "reject stale original Region 10");
    let saved = fs::read(&sidecar).unwrap();
    drop(h);
    let mut h = app(&scene, &evidence, "reopened");
    settle(&mut h);
    click(&mut h, "hero");
    click(&mut h, "Sprite bindings (view only)");
    h.state_mut().sprites.path = sidecar.to_string_lossy().into();
    click(&mut h, "Open bindings");
    no_error(&h);
    assert_eq!(
        capture(
            &mut h,
            &evidence,
            "06-fresh-reopened-region-30",
            2,
            &mut records
        ),
        first
    );
    click(&mut h, LBL_PLAY);
    for (label, minimum, maximum, expected) in [
        ("07-real-play-second-region-10", 2100, 4500, 0),
        ("08-real-play-repeated-region-30", 4600, 7500, 2),
        ("09-real-play-final-region-20", 7600, 9500, 1),
        ("10-real-play-once-still-region-20", 9800, 60000, 1),
    ] {
        wait_for(&mut h, label, |app| elapsed_ms(app) >= minimum);
        let samples = capture(&mut h, &evidence, label, expected, &mut records);
        assert!(
            (minimum..maximum).contains(&elapsed_ms(h.state())),
            "{label}: capture escaped literal timing interval"
        );
        if expected == 1 {
            assert_ne!(samples, first, "Once must hold its actual final region");
            assert_ne!(
                samples, original_pixels,
                "new sequence differs from original binding"
            );
        }
    }
    click(&mut h, LBL_STOP);
    no_error(&h);
    assert_eq!(fs::read(&sidecar).unwrap(), saved);
    assert_eq!(inventory(&fresh.join(".orr")), installed);
    assert_eq!(fs::read_to_string(&scene).unwrap(), SCENE);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(records.len(), 10);
    fs::write(evidence.join("exported-sprites.json"), published).unwrap();
    let adapters: Vec<Value> = ["original", "assigned", "reopened"]
        .iter()
        .map(|label| {
            serde_json::from_slice(
                &fs::read(evidence.join(format!("{label}-adapter.json"))).unwrap(),
            )
            .unwrap()
        })
        .collect();
    fs::write(evidence.join("receipt.json"), serde_json::to_vec_pretty(&json!({
        "test":"sequence_widgets_export_package_reopen_and_real_play_pixels","mandatory_test_count":1,
        "adapters":adapters,"capture_count":records.len(),"captures":records,"actual_widgets":true,
        "actual_rust_producer":true,"fresh_installed_consumer":true,"known_source_paths_hidden":true,
        "private_draft_did_not_mutate_originals":true,"admitted_texture_preserved":true,
        "zero_clip_source":true,"nongrid_repeated_region_order":[30,10,30,20],"holds_ms":[2000,2500,3000,2000],
        "exact_half_open_boundaries":true,"no_replace_export":true,"save_reopen_real_play":true,
        "literal_source_pixel_oracle":true,"center_pivot_oracle":true,"nearest_filter_oracle":true,
        "native_window":false,"physical_input":false
    })).unwrap()).unwrap();
}
