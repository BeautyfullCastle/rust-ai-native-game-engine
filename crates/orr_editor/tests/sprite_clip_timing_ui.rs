//! Actual timing widgets -> exported JSON -> built Rust producer -> fresh installed
//! EditorApp -> Save/reopen/real Play -> composed framebuffer source-pixel oracle.
#![cfg(all(feature = "sprites", target_os = "linux"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
#[path = "common/arena_sprites.rs"]
mod common;
use common::{body_position, click, document, no_error, settle, wait_for, HERO};
use egui::accesskit::Role;
use egui_kittest::{kittest::Queryable, Harness};
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
const PACKAGE: &str = "timing-fixture";
const SCENE: &str = "schema: orr.scene/1\nsingletons:\n  Score: { kills: [0,0,0,0,0,0,0,0] }\nentities:\n  e_00000001:\n    name: hero\n    Position: { pos: [0,0] }\n    PlayerTag: { slot: 0 }\n";
// Three distinct asymmetric 2x2 cells. Expected colors never come from the
// exported document, renderer UVs, sampled-region API, or previous captures.
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
fn app(scene: &Path, evidence: &Path) -> Harness<'static, EditorApp> {
    let mut editor = Editor::open_game(scene, EditorGame::Arena).unwrap();
    editor.sync();
    editor.camera = orr_render::Camera::new([0., 0.], 10.);
    let evidence = evidence.to_owned();
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
            fs::write(
                evidence.join("adapter.json"),
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
    let client = editor.agent_client("clip-timing-frame-proof").unwrap();
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
    h.get_by_role_and_label(Role::ComboBox, label)
        .scroll_to_me();
    settle(h);
    h.get_by_role_and_label(Role::ComboBox, label).click();
    h.run_steps(2);
    h.get_by_role_and_label(Role::Button, value).click();
    settle(h);
}
fn configure(h: &mut Harness<'_, EditorApp>, sidecar: &Path, clip: &str) {
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
    choose(h, "Sprite source", &format!("Clip {clip}"));
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
fn capture(
    h: &mut Harness<'_, EditorApp>,
    evidence: &Path,
    label: &str,
    expected: usize,
    records: &mut Vec<Value>,
) -> Vec<[u8; 3]> {
    h.event(egui::Event::PointerMoved(egui::pos2(2., 2.)));
    settle(h);
    let image = h
        .render()
        .expect("mandatory actual composed EditorApp framebuffer");
    let (width, height) = (image.width(), image.height());
    let rgba = image.into_raw();
    let rect = h.state().ui.viewport_rect.unwrap();
    let pixels = h.state().ui.viewport_px;
    let body = body_position(h.state(), HERO);
    let mut samples = Vec::new();
    for y in 0..2 {
        for x in 0..2 {
            let at = h.state().editor.camera.world_to_screen(
                [
                    body[0] + (x as f32 - 0.5) * 2.,
                    body[1] + (0.5 - y as f32) * 2.,
                ],
                pixels,
            );
            let sx = (rect.min.x + at[0] * rect.width() / pixels.0 as f32).floor() as u32;
            let sy = (rect.min.y + at[1] * rect.height() / pixels.1 as f32).floor() as u32;
            assert!(sx < width && sy < height);
            let offset = ((sy * width + sx) * 4) as usize;
            let sample: [u8; 3] = rgba[offset..offset + 3].try_into().unwrap();
            assert!(
                sample
                    .iter()
                    .zip(CELLS[expected][y * 2 + x])
                    .all(|(a, e)| a.abs_diff(e) <= 3),
                "{label}: cell {expected}, pixel {x},{y}: {sample:?}"
            );
            samples.push(sample);
        }
    }
    png(&evidence.join(format!("{label}.png")), width, height, &rgba);
    records.push(json!({"file":format!("{label}.png"),"expected_cell":expected,"elapsed_ms":elapsed_ms(h.state()),"samples":samples}));
    samples
}
#[test]
#[ignore = "mandatory explicit composed GPU and built Rust orr_pkg sprite-authoring gate; run --ignored --exact and require exactly one test"]
fn timing_widgets_export_package_reopen_and_real_play_pixels() {
    assert_eq!(
        std::env::var("ORR_REQUIRE_GPU").as_deref(),
        Ok("1"),
        "mandatory GPU gate"
    );
    let binary =
        PathBuf::from(std::env::var_os("ORR_RUST_PACKAGE_BIN").expect("built orr_pkg required"))
            .canonicalize()
            .unwrap();
    let evidence = PathBuf::from(
        std::env::var_os("ORR_SPRITE_TIMING_EVIDENCE")
            .expect("fresh caller-created evidence directory required"),
    );
    assert!(evidence.is_dir());
    assert_eq!(
        fs::read_dir(&evidence).unwrap().count(),
        0,
        "refuse stale evidence"
    );
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("source");
    fs::create_dir(&source).unwrap();
    let rgba: Vec<_> = (0..2)
        .flat_map(|y| (0..3).flat_map(move |cell| (0..2).flat_map(move |x| CELLS[cell][y * 2 + x])))
        .collect();
    png(&source.join("atlas.png"), 6, 2, &rgba);
    fs::write(
        source.join("LICENSE.txt"),
        "Synthetic test-owned art CC0-1.0\n",
    )
    .unwrap();
    let original = json!({"format":"orr_sprite","version":1,"atlas":{"image":"atlas.png","width":6,"height":2},
        "regions":[{"id":10,"x":0,"y":0,"width":2,"height":2},{"id":20,"x":2,"y":0,"width":2,"height":2},{"id":30,"x":4,"y":0,"width":2,"height":2}],
        "clips":[{"id":"source","mode":"loop","frames":[{"region":10,"duration_ms":60000},{"region":20,"duration_ms":170},{"region":10,"duration_ms":230},{"region":30,"duration_ms":310}]}]});
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
    let mut h = app(&project.join("arena.scene.yaml"), &evidence);
    configure(&mut h, &project.join("arena.sprites.json"), "source");
    let before = inventory(&project);
    let source_before = inventory(&source);
    let package_before = inventory(&package);
    let original_frame = frame_bytes(&mut h.state_mut().editor);
    let bindings = document(&h).clone();
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().entries.len();
    let mut records = Vec::new();
    let original_pixels = capture(&mut h, &evidence, "01-original", 0, &mut records);
    click(&mut h, "Begin clip timing variant");
    text(&mut h, "Timing variant ID", "retimed");
    click(&mut h, "Preview timing variant");
    no_error(&h);
    assert_eq!(
        capture(
            &mut h,
            &evidence,
            "02-noop-private-preview",
            0,
            &mut records
        ),
        original_pixels
    );
    click(&mut h, "Stop timing variant preview");
    let invalid_output = work.path().join("invalid.json");
    text(
        &mut h,
        "Timing variant output path",
        invalid_output.to_str().unwrap(),
    );
    text(&mut h, "Frame 0 duration ms", "0");
    click(&mut h, "Export timing variant JSON");
    assert!(h.state().sprites.error().is_some());
    assert!(!invalid_output.exists());
    assert_eq!(
        capture(
            &mut h,
            &evidence,
            "03-invalid-draft-no-mutation",
            0,
            &mut records
        ),
        original_pixels
    );
    for (i, duration) in [80, 160, 240, 320].iter().enumerate() {
        text(
            &mut h,
            &format!("Frame {i} duration ms"),
            &duration.to_string(),
        );
    }
    h.get_by_role_and_label(Role::RadioButton, "Timing Once")
        .scroll_to_me();
    settle(&mut h);
    h.get_by_role_and_label(Role::RadioButton, "Timing Once")
        .click();
    settle(&mut h);
    let exported = work.path().join("retimed.json");
    text(
        &mut h,
        "Timing variant output path",
        exported.to_str().unwrap(),
    );
    click(&mut h, "Preview timing variant");
    no_error(&h);
    assert_eq!(
        capture(
            &mut h,
            &evidence,
            "04-private-preview-does-not-rebind",
            0,
            &mut records
        ),
        original_pixels
    );
    click(&mut h, "Stop timing variant preview");
    click(&mut h, "Export timing variant JSON");
    no_error(&h);
    let exported_json: Value = serde_json::from_slice(&fs::read(&exported).unwrap()).unwrap();
    assert_eq!(exported_json["atlas"], original["atlas"]);
    assert_eq!(exported_json["regions"], original["regions"]);
    assert_eq!(exported_json["clips"][0], original["clips"][0]);
    let variant = exported_json["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "retimed")
        .unwrap();
    assert_eq!(
        variant,
        &json!({"id":"retimed","mode":"once","frames":[{"region":10,"duration_ms":80},{"region":20,"duration_ms":160},{"region":10,"duration_ms":240},{"region":30,"duration_ms":320}]})
    );
    assert_eq!(document(&h), &bindings);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history().entries.len(), history);
    assert_eq!(frame_bytes(&mut h.state_mut().editor), original_frame);
    assert_eq!(inventory(&project), before);
    assert_eq!(inventory(&source), source_before);
    assert_eq!(inventory(&package), package_before);
    drop(h);
    let new_package = work.path().join("retimed-package");
    producer(
        &binary,
        &source,
        &exported,
        &new_package,
        &evidence,
        "retimed-producer",
    );
    let fresh = work.path().join("fresh-project");
    install(&fresh, &new_package);
    fs::copy(&exported, evidence.join("exported-sprites.json")).unwrap();
    // Remove all authoring originals and both publication directories from their
    // known paths before a new host/app/asset cache consumes installed bytes.
    for path in [&source, &package, &new_package, &project] {
        fs::rename(path, path.with_extension("hidden")).unwrap();
        assert!(!path.exists());
    }
    fs::remove_file(&exported).unwrap();
    let installed = inventory(&fresh.join(".orr"));
    let scene = fresh.join("arena.scene.yaml");
    let sidecar = fresh.join("arena.sprites.json");
    let mut h = app(&scene, &evidence);
    configure(&mut h, &sidecar, "retimed");
    assert_eq!(
        document(&h).bindings[HERO].source,
        Source::Clip("retimed".into())
    );
    capture(
        &mut h,
        &evidence,
        "05-new-package-assigned",
        0,
        &mut records,
    );
    let saved = fs::read(&sidecar).unwrap();
    drop(h);
    let mut h = app(&scene, &evidence);
    settle(&mut h);
    click(&mut h, "hero");
    click(&mut h, "Sprite bindings (view only)");
    h.state_mut().sprites.path = sidecar.to_string_lossy().into();
    click(&mut h, "Open bindings");
    no_error(&h);
    capture(&mut h, &evidence, "06-fresh-reopened", 0, &mut records);
    click(&mut h, LBL_PLAY);
    wait_for(
        &mut h,
        "retimed Once reaches actual later host tick",
        |app| elapsed_ms(app) >= 900,
    );
    let elapsed = elapsed_ms(h.state());
    assert!(
        elapsed < 60000,
        "old clip negative control remains in first frame"
    );
    let later = capture(
        &mut h,
        &evidence,
        "07-real-play-retimed-terminal",
        2,
        &mut records,
    );
    assert!(
        (900..60000).contains(&elapsed_ms(h.state())),
        "actual captured tick must reject old timing"
    );
    assert_ne!(
        later, original_pixels,
        "retimed later-time source pixels must reject old timing"
    );
    click(&mut h, LBL_STOP);
    no_error(&h);
    assert_eq!(fs::read(&sidecar).unwrap(), saved);
    assert_eq!(inventory(&fresh.join(".orr")), installed);
    assert_eq!(fs::read_to_string(&scene).unwrap(), SCENE);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(records.len(), 7);
    fs::write(evidence.join("receipt.json"),serde_json::to_vec_pretty(&json!({"test":"timing_widgets_export_package_reopen_and_real_play_pixels","mandatory_test_count":1,
        "adapter":serde_json::from_slice::<Value>(&fs::read(evidence.join("adapter.json")).unwrap()).unwrap(),
        "capture_count":records.len(),"captures":records,"actual_widgets":true,"actual_rust_producer":true,
        "fresh_installed_consumer":true,"known_source_paths_hidden":true,"private_draft_did_not_mutate_originals":true,
        "save_reopen_real_play":true,"literal_source_pixel_oracle":true,"native_window":false})).unwrap()).unwrap();
}
