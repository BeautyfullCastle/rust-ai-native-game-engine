//! Actual Yard Edit → CPU bake → native viewport → save/reopen acceptance.
//! GPU absence is a failure. STATIC_BAKE_CAPTURE_DIR optionally records PNGs.
#![cfg(feature = "irradiance-probes")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui_kittest::{kittest::Queryable, Harness};
#[cfg(feature = "animated-models")]
use orr_editor::model_bindings::PlaybackMode;
use orr_editor::{
    app::BottomTab, game::EditorGame, model_bindings::ModelKind, Editor, EditorApp, Mode,
};
use orr_package::{Project, Runtime};
use orr_render::irradiance::{constant_irradiance, IrradianceProvenance};
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

static GPU_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
type HostFrame = (u64, u64, Vec<u8>);

fn fixture(root: &Path) -> PathBuf {
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let sources = vec![
        assets.join("imported_scene_demo").canonicalize().unwrap(),
        assets.join("irradiance_demo").canonicalize().unwrap(),
    ];
    #[cfg(feature = "animated-models")]
    let sources = {
        let mut sources = sources;
        sources.push(assets.join("animation_demo").canonicalize().unwrap());
        sources
    };
    Project::open_for_install(root, Runtime::content_only().engine_version)
        .unwrap()
        .install(&sources)
        .unwrap();
    let text = include_str!("../../../scenes/yard3d_authoring.scene.yaml")
        .replace("kind: dynamic", "kind: static")
        .replace("inv_mass: 1", "inv_mass: 0");
    // Keep a procedural box beside the static and animated imports. The
    // static-only lane leaves box_left procedural instead.
    #[cfg(feature = "animated-models")]
    let text = {
        let fourth = text
            .split("  e_00000003:\n")
            .nth(1)
            .unwrap()
            .replace("name: box_right", "name: procedural_box")
            .replace("pos: [2, 3, 0]", "pos: [0, 1, -3]");
        format!("{text}  e_00000004:\n{fourth}")
    };
    let scene = root.join("yard-irradiance.scene.yaml");
    std::fs::write(&scene, text).unwrap();
    scene
}

fn sync_clock(editor: &mut Editor) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        editor.sync();
        if editor.snapshot().is_some_and(|snapshot| {
            snapshot.timeline().is_some() == (editor.mode() == Mode::Play)
                && snapshot.tick() == snapshot.predicted().tick()
                && snapshot.timeline().is_none_or(|timeline| {
                    !timeline.playing
                        && timeline.tick == snapshot.tick()
                        && timeline.tick == editor.timeline().unwrap().tick
                })
        }) && editor.yard_rows_coherent()
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "coherent paused Yard frame missing"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn host_frame(editor: &mut Editor) -> HostFrame {
    sync_clock(editor);
    let snapshot = editor.snapshot().unwrap();
    let tick = snapshot.tick();
    let checksum = snapshot.predicted().checksum();
    let client = editor.agent_client("yard-irradiance-byte-probe").unwrap();
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
                return (tick, checksum, frame.frame.to_bytes());
            }
        }
        assert!(
            Instant::now() < deadline,
            "matching host frame bytes missing"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn gpu() -> Wgpu {
    let gpu = Wgpu::headless(WgpuOptions::default())
        .expect("mandatory production static-bake GPU adapter");
    eprintln!(
        "Static sun bounce adapter: {} software={} animated={}",
        gpu.adapter_name(),
        gpu.is_software(),
        cfg!(feature = "animated-models")
    );
    gpu
}

fn assign(app: &mut EditorApp, name: &str, kind: ModelKind) {
    assert!(app.editor.select_named(name));
    sync_clock(&mut app.editor);
    app.models.kind = kind;
    app.models.package = match kind {
        ModelKind::Static => "sample-imported-scene",
        ModelKind::Animated => "sample-animation",
    }
    .into();
    app.models.asset = match kind {
        ModelKind::Static => "foreground.glb",
        ModelKind::Animated => "animated.glb",
    }
    .into();
    #[cfg(feature = "animated-models")]
    {
        app.models.animation.clip_index = 0;
        app.models.animation.playback = PlaybackMode::Loop;
    }
    app.models.assign(&app.editor).unwrap();
}

fn configure(app: &mut EditorApp) {
    app.models.open_for_editor(&app.editor, true).unwrap();
    assign(app, "box_right", ModelKind::Static);
    #[cfg(feature = "animated-models")]
    assign(app, "box_left", ModelKind::Animated);
    app.editor.select(None);
    app.irradiance.open_for_editor(&app.editor, true).unwrap();
}

fn settle(harness: &mut Harness<'_, EditorApp>) {
    for _ in 0..3 {
        sync_clock(&mut harness.state_mut().editor);
        harness.run_steps(2);
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while harness.state().irradiance.is_validating_bake() {
        harness.run_steps(1);
        assert!(
            Instant::now() < deadline,
            "source validation did not finish"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(harness.state().editor.down().is_none());
    assert!(harness.state().editor.yard_rows_coherent());
}

fn grid_geometry(app: &mut EditorApp) {
    let mut grid = app
        .irradiance
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .grid
        .clone();
    grid.dimensions = [4; 3];
    grid.origin = [-8.0, 0.0, -8.0];
    grid.spacing = [6.0, 3.0, 6.0];
    grid.coefficients = vec![[[0.0; 3]; 9]; 64];
    app.irradiance
        .apply_grid_for_editor(&app.editor, grid)
        .unwrap();
}

fn finish_bake(h: &mut Harness<'_, EditorApp>) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while h.state().irradiance.is_baking() {
        h.run_steps(1);
        assert!(
            Instant::now() < deadline,
            "bounded bake did not finish: {:?}",
            h.state().irradiance.bake_status()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    settle(h);
}

fn pixels(h: &mut Harness<'_, EditorApp>, name: &str) -> Vec<u8> {
    settle(h);
    let actual = h
        .state()
        .viewport3d_gpu()
        .expect("production GPU viewport")
        .gpu();
    assert_eq!(
        actual.irradiance(),
        h.state().irradiance.grid_for_editor(&h.state().editor)
    );
    let pixels = actual.read_rgba8();
    if let Some(directory) = std::env::var_os("STATIC_BAKE_CAPTURE_DIR") {
        let directory = PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        let file = std::fs::File::create(directory.join(format!("{name}-viewport.png"))).unwrap();
        let size = actual.target().size();
        let mut encoder = png::Encoder::new(file, size.0, size.1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        h.render()
            .unwrap()
            .save(directory.join(format!("{name}-editor.png")))
            .unwrap();
    }
    pixels
}

fn matching_pixels(h: &mut Harness<'_, EditorApp>, name: &str, target: (u32, u32)) -> Vec<u8> {
    // Real editor chrome can grow to accommodate a source-error message. Keep
    // the *actual native viewport* at the baseline dimensions by resizing the
    // surrounding test window, as a user can, before exact pixel comparison.
    // This changes neither camera nor lighting, and does not resize readbacks.
    for _ in 0..4 {
        settle(h);
        let actual = h.state().ui.viewport_px;
        if actual == target {
            let pixels = pixels(h, name);
            assert_eq!(
                h.state().ui.viewport_px,
                target,
                "{name}: capture dimensions changed after settling"
            );
            return pixels;
        }
        let window = h.ctx.viewport_rect().size();
        let ppp = h.ctx.pixels_per_point();
        h.set_size(egui::vec2(
            window.x + (target.0 as f32 - actual.0 as f32) / ppp,
            window.y + (target.1 as f32 - actual.1 as f32) / ppp,
        ));
    }
    panic!(
        "{name}: viewport did not stabilize at {target:?}; actual={:?}",
        h.state().ui.viewport_px
    );
}

fn assert_pixels_equal(actual: &[u8], expected: &[u8], stage: &str) {
    let changed_pixels = actual
        .as_chunks::<4>().0.iter()
        .zip(expected.as_chunks::<4>().0.iter())
        .filter(|(a, b)| a != b)
        .count();
    let hex = |bytes: &[u8]| {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    assert!(actual == expected,
        "{stage}: changed pixels={changed_pixels}; actual bytes={} SHA256={}; expected bytes={} SHA256={}",
        actual.len(), hex(actual), expected.len(), hex(expected));
}

fn package_bytes(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn visit(path: &Path, files: &mut Vec<(PathBuf, Vec<u8>)>) {
        if path.is_dir() {
            let mut children = std::fs::read_dir(path)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect::<Vec<_>>();
            children.sort();
            for child in children {
                visit(&child, files);
            }
        } else {
            files.push((path.to_owned(), std::fs::read(path).unwrap()));
        }
    }
    let mut files = Vec::new();
    // All installed content is immutable; omit author-owned scene and sidecars.
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir()
            || path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("orr."))
        {
            visit(&path, &mut files);
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

#[test]
fn actual_bake_button_commits_once_lights_viewport_and_reopens_verified_receipt() {
    let _serial = GPU_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    drop(gpu());
    let temp = tempfile::tempdir().unwrap();
    let scene = fixture(temp.path());
    let mut editor = Editor::open_game(&scene, EditorGame::Yard3D).unwrap();
    sync_clock(&mut editor);
    // A real host edit is saved before the presentation-only bake boundary.
    assert!(editor.select_named("ground"));
    assert!(editor.set_yard_transform(
        orr_fp::FPVec3::new(orr_fp::FP::ZERO, "-0.6".parse().unwrap(), orr_fp::FP::ZERO),
        [
            orr_fp::FP::ZERO,
            orr_fp::FP::ZERO,
            orr_fp::FP::ZERO,
            orr_fp::FP::ONE
        ],
    ));
    sync_clock(&mut editor);
    assert!(editor.save());
    sync_clock(&mut editor);
    editor.select(None);
    editor.camera3d = orr_render::OrbitCamera::new([0.0, 1.5, 0.0], 0.45, 0.65, 14.0);
    let camera = editor.camera3d;
    let expected_host = host_frame(&mut editor);
    let scene_bytes = std::fs::read(&scene).unwrap();
    let package_before = package_bytes(temp.path());
    let mut h = Harness::builder()
        .with_size([1180.0, 1300.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(|cc| {
            let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
            // Edit Timeline has identical content before and after reopening.
            // History intentionally differs because the real edit is saved.
            app.ui.bottom_tab = BottomTab::Timeline;
            app
        });
    settle(&mut h);
    configure(h.state_mut());
    grid_geometry(h.state_mut());
    h.get_by_label("Irradiance probes").click();
    settle(&mut h);
    let host_history = h.state().editor.history().clone();
    let before = h
        .state()
        .irradiance
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    let revision = h.state().irradiance.bindings.as_ref().unwrap().revision();
    let off = pixels(&mut h, "bake-before");
    let viewport_size = h.state().ui.viewport_px;
    let off_composed = h.render().unwrap();
    // Use the actual production button, not a fabricated authored SH write.
    h.get_by_label("Bake static sun bounce").click();
    h.run_steps(1);
    finish_bake(&mut h);
    let document = h
        .state()
        .irradiance
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    let receipt = document.bake_receipt.as_ref().expect("CPU worker receipt");
    assert_eq!(document.grid.provenance, IrradianceProvenance::Baked);
    assert!(document.grid.enabled);
    assert_eq!(receipt.directions, 1024);
    assert!(receipt.triangle_count > 0);
    assert!(receipt.participant_count > 0);
    assert_eq!(
        h.state().irradiance.bindings.as_ref().unwrap().revision(),
        revision + 1,
        "exactly one grid+receipt transaction"
    );
    let on = matching_pixels(&mut h, "bake-complete", viewport_size);
    assert_eq!(
        h.state().ui.viewport_px,
        viewport_size,
        "baking must not change the compared viewport dimensions"
    );
    let changed = on
        .as_chunks::<4>().0.iter()
        .zip(off.as_chunks::<4>().0.iter())
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        changed > 100,
        "real traced sun bounce must change native viewport pixels: {changed}"
    );
    let composed = h.render().unwrap();
    let rect = h.state().ui.viewport_rect.unwrap().shrink(4.0);
    let changed = composed
        .enumerate_pixels()
        .filter(|(x, y, p)| {
            rect.contains(egui::pos2(*x as f32, *y as f32)) && *p != off_composed.get_pixel(*x, *y)
        })
        .count();
    assert!(
        changed > 100,
        "baked GPU lighting must reach displayed egui viewport"
    );
    {
        let app = h.state_mut();
        app.irradiance.undo(&app.editor).unwrap();
        assert_eq!(
            app.irradiance.bindings.as_ref().unwrap().document(),
            &before
        );
        app.irradiance.redo(&app.editor).unwrap();
        assert_eq!(
            app.irradiance.bindings.as_ref().unwrap().document(),
            &document
        );
    }
    assert_pixels_equal(
        &matching_pixels(&mut h, "bake-redo", viewport_size),
        &on,
        "bake-redo",
    );
    let sidecar = {
        let app = h.state_mut();
        app.models.bindings.as_mut().unwrap().save().unwrap();
        app.irradiance.save().unwrap();
        app.irradiance.bindings.as_ref().unwrap().path.clone()
    };
    let saved = std::fs::read(&sidecar).unwrap();
    assert_eq!(host_frame(&mut h.state_mut().editor), expected_host);
    assert_eq!(h.state().editor.history(), &host_history);
    assert_eq!(std::fs::read(&scene).unwrap(), scene_bytes);
    assert_eq!(package_bytes(temp.path()), package_before);
    drop(h);
    let mut editor = Editor::open_game(&scene, EditorGame::Yard3D).unwrap();
    editor.camera3d = camera;
    let mut h = Harness::builder()
        .with_size([1180.0, 1300.0])
        .wgpu()
        .build_eframe(|cc| {
            let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
            // Edit Timeline has identical content before and after reopening.
            // History intentionally differs because the real edit is saved.
            app.ui.bottom_tab = BottomTab::Timeline;
            app
        });
    settle(&mut h);
    {
        let app = h.state_mut();
        app.models.open_for_editor(&app.editor, false).unwrap();
        app.irradiance.open_for_editor(&app.editor, false).unwrap();
        app.irradiance
            .refresh_baked_validity(&app.editor, &app.models);
    }
    settle(&mut h);
    assert_eq!(
        h.state().irradiance.grid_for_editor(&h.state().editor),
        Some(&document.grid)
    );
    h.get_by_label("Irradiance probes").click();
    let reopened = matching_pixels(&mut h, "bake-reopened", viewport_size);
    assert_eq!(
        h.state().ui.viewport_px,
        viewport_size,
        "reopen must use the exact saved-run viewport dimensions"
    );
    assert_pixels_equal(&reopened, &on, "bake-reopened");
    assert_eq!(
        h.state().irradiance.bindings.as_ref().unwrap().document(),
        &document
    );
    assert_eq!(std::fs::read(&sidecar).unwrap(), saved);
    assert_eq!(host_frame(&mut h.state_mut().editor), expected_host);
    assert_eq!(std::fs::read(&scene).unwrap(), scene_bytes);
    assert_eq!(package_bytes(temp.path()), package_before);

    // Tampering only with the installed source bytes invalidates a retained
    // cached model's bake too; GPU fallback and sidecar state stay atomic.
    let asset_path = {
        let binding = h
            .state()
            .models
            .bindings
            .as_ref()
            .unwrap()
            .document()
            .bindings
            .values()
            .find(|binding| binding.kind == ModelKind::Static)
            .unwrap();
        temp.path()
            .join(".orr/packages/objects")
            .join(&binding.package_digest)
            .join(&binding.asset)
    };
    let asset_bytes = std::fs::read(&asset_path).unwrap();
    let mut tampered = asset_bytes.clone();
    *tampered.last_mut().unwrap() ^= 1;
    std::fs::write(&asset_path, tampered).unwrap();
    assert_pixels_equal(
        &matching_pixels(&mut h, "bake-source-stale", viewport_size),
        &off,
        "bake-source-stale",
    );
    assert!(h.state().irradiance.baked_stale_reason().is_some());
    assert_eq!(
        h.state().irradiance.bindings.as_ref().unwrap().document(),
        &document
    );
    assert!(!h.state().irradiance.bindings.as_ref().unwrap().dirty());
    assert_eq!(std::fs::read(&sidecar).unwrap(), saved);
    {
        let app = h.state_mut();
        app.irradiance.open_for_editor(&app.editor, false).unwrap();
    }
    assert_pixels_equal(
        &matching_pixels(&mut h, "bake-reopen-tampered", viewport_size),
        &off,
        "bake-reopen-tampered",
    );
    assert!(h.state().irradiance.baked_stale_reason().is_some());
    for _ in 0..4 {
        h.run_steps(1);
        assert!(
            !h.state().irradiance.is_validating_bake(),
            "failed source verification does not respawn every frame"
        );
    }
    std::fs::write(&asset_path, asset_bytes).unwrap();
    {
        let app = h.state_mut();
        app.irradiance.open_for_editor(&app.editor, false).unwrap();
    }
    assert_pixels_equal(
        &matching_pixels(&mut h, "bake-source-restored", viewport_size),
        &on,
        "bake-source-restored",
    );
    assert_eq!(std::fs::read(&sidecar).unwrap(), saved);
    assert_eq!(package_bytes(temp.path()), package_before);
}

fn plain_app(root: &Path) -> EditorApp {
    let scene = root.join("transaction.scene.yaml");
    std::fs::write(
        &scene,
        include_str!("../../../scenes/yard3d_authoring.scene.yaml"),
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::Yard3D).unwrap();
    sync_clock(&mut editor);
    let mut app = EditorApp::new(editor, None);
    app.irradiance.open_for_editor(&app.editor, true).unwrap();
    let mut grid = app
        .irradiance
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .grid
        .clone();
    grid.enabled = true;
    grid.coefficients
        .fill(constant_irradiance([0.5, 0.2, 0.1]).unwrap());
    app.irradiance
        .apply_grid_for_editor(&app.editor, grid)
        .unwrap();
    app.irradiance.save().unwrap();
    app
}

fn drain(app: &mut EditorApp) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while app.irradiance.is_baking() {
        sync_clock(&mut app.editor);
        app.irradiance
            .sync_bake_for_editor(&app.editor, &app.models);
        assert!(Instant::now() < deadline, "worker did not stop");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn cancellation_document_aba_mode_and_scene_guards_never_publish_or_write() {
    let _serial = GPU_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let mut app = plain_app(temp.path());
    let document = app.irradiance.bindings.as_ref().unwrap().document().clone();
    let path = app.irradiance.bindings.as_ref().unwrap().path.clone();
    let bytes = std::fs::read(&path).unwrap();
    let revision = app.irradiance.bindings.as_ref().unwrap().revision();
    let host = host_frame(&mut app.editor);
    let history = app.editor.history().clone();
    app.irradiance.start_bake(&app.editor, &app.models).unwrap();
    assert_eq!(
        app.irradiance.grid_for_editor(&app.editor),
        Some(&document.grid),
        "old accepted grid stays applied during bake"
    );
    app.irradiance.cancel_bake();
    drain(&mut app);
    assert!(app.irradiance.bake_status().unwrap().contains("cancelled"));
    assert_eq!(
        app.irradiance.bindings.as_ref().unwrap().revision(),
        revision
    );
    assert_eq!(
        app.irradiance.bindings.as_ref().unwrap().document(),
        &document
    );
    assert!(!app.irradiance.bindings.as_ref().unwrap().dirty());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(host_frame(&mut app.editor), host);
    assert_eq!(app.editor.history(), &history);

    // Replacing the bindings instance can reuse revision zero and the same
    // path. The panel generation/cancel guard still rejects that old job.
    app.irradiance.open_for_editor(&app.editor, false).unwrap();
    assert_eq!(app.irradiance.bindings.as_ref().unwrap().revision(), 0);
    app.irradiance.start_bake(&app.editor, &app.models).unwrap();
    app.irradiance.open_for_editor(&app.editor, false).unwrap();
    drain(&mut app);
    assert_eq!(app.irradiance.bindings.as_ref().unwrap().revision(), 0);
    assert_eq!(
        app.irradiance.bindings.as_ref().unwrap().document(),
        &document
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);

    // Same final value is still a changed document revision; stale completion
    // cannot erase independent authoring history through an ABA edit.
    app.irradiance.start_bake(&app.editor, &app.models).unwrap();
    let mut changed = document.grid.clone();
    changed.enabled = !changed.enabled;
    app.irradiance
        .apply_grid_for_editor(&app.editor, changed)
        .unwrap();
    app.irradiance.undo(&app.editor).unwrap();
    let revision = app.irradiance.bindings.as_ref().unwrap().revision();
    assert_eq!(
        app.irradiance.bindings.as_ref().unwrap().document(),
        &document
    );
    assert!(app.irradiance.bindings.as_ref().unwrap().can_redo());
    drain(&mut app);
    assert!(app.irradiance.bake_status().unwrap().contains("discarded"));
    assert_eq!(
        app.irradiance.bindings.as_ref().unwrap().revision(),
        revision
    );
    assert!(app.irradiance.bindings.as_ref().unwrap().can_redo());
    assert_eq!(
        app.irradiance.bindings.as_ref().unwrap().document(),
        &document
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);

    app.irradiance.start_bake(&app.editor, &app.models).unwrap();
    let proposal = app
        .editor
        .agent_client("bake-preview-guard")
        .unwrap()
        .call("proposal.begin", json!({"label":"bake preview guard"}))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    app.editor.sync();
    assert!(app.editor.set_preview(Some(proposal)));
    drain(&mut app);
    assert!(app.irradiance.bake_status().unwrap().contains("discarded"));
    assert_eq!(
        app.irradiance.bindings.as_ref().unwrap().document(),
        &document
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(app.editor.set_preview(None));
    sync_clock(&mut app.editor);

    app.irradiance.start_bake(&app.editor, &app.models).unwrap();
    assert!(app.editor.start_play());
    sync_clock(&mut app.editor);
    drain(&mut app);
    assert!(app.irradiance.bake_status().unwrap().contains("discarded"));
    assert_eq!(
        app.irradiance.bindings.as_ref().unwrap().document(),
        &document
    );
    assert!(app.irradiance.start_bake(&app.editor, &app.models).is_err());
    app.editor.stop();
    sync_clock(&mut app.editor);
    app.irradiance.start_bake(&app.editor, &app.models).unwrap();
    let other = temp.path().join("other.scene.yaml");
    std::fs::write(
        &other,
        include_str!("../../../scenes/yard3d_authoring.scene.yaml"),
    )
    .unwrap();
    assert!(app.editor.open_path(&other));
    sync_clock(&mut app.editor);
    drain(&mut app);
    assert!(app.irradiance.bake_status().unwrap().contains("discarded"));
    assert_eq!(
        app.irradiance.bindings.as_ref().unwrap().document(),
        &document
    );
    assert!(app.irradiance.grid_for_editor(&app.editor).is_none());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(!orr_editor::irradiance_bindings::IrradianceBindings::sidecar_path(&other).exists());
}

#[test]
fn baked_receipt_ignores_excluded_motion_but_disables_static_edits_and_save_failure_is_atomic() {
    let _serial = GPU_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let mut app = plain_app(temp.path());
    // Include a real textured imported static participant, so the cache
    // regression protects mesh/image copying rather than only a tiny plane.
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/imported_scene_demo");
    Project::open_for_install(temp.path(), Runtime::content_only().engine_version)
        .unwrap()
        .install(&[assets.canonicalize().unwrap()])
        .unwrap();
    app.models.open_for_editor(&app.editor, true).unwrap();
    assign(&mut app, "ground", ModelKind::Static);
    app.irradiance.start_bake(&app.editor, &app.models).unwrap();
    drain(&mut app);
    let document = app.irradiance.bindings.as_ref().unwrap().document().clone();
    assert_eq!(
        document.grid.provenance,
        IrradianceProvenance::Baked,
        "{:?}",
        app.irradiance.bake_status()
    );
    assert_eq!(document.bake_receipt.as_ref().unwrap().participant_count, 1);
    app.irradiance.save().unwrap();
    let path = app.irradiance.bindings.as_ref().unwrap().path.clone();
    let bytes = std::fs::read(&path).unwrap();
    let revision = app.irradiance.bindings.as_ref().unwrap().revision();
    assert!(app.editor.select_named("box_left"));
    assert!(app.editor.set_yard_transform(
        orr_fp::FPVec3::new(
            orr_fp::FP::from_int(-4),
            orr_fp::FP::from_int(6),
            orr_fp::FP::ONE
        ),
        [
            orr_fp::FP::ZERO,
            orr_fp::FP::ZERO,
            orr_fp::FP::ZERO,
            orr_fp::FP::ONE
        ],
    ));
    sync_clock(&mut app.editor);
    app.editor.camera3d.yaw += 0.2;
    app.ui.yard_post_process.threshold = 2.0;
    app.irradiance
        .refresh_baked_validity(&app.editor, &app.models);
    assert_eq!(
        app.irradiance.grid_for_editor(&app.editor),
        Some(&document.grid),
        "camera, selection, post-processing and dynamic body motion are excluded"
    );
    assert_eq!(
        app.irradiance.bindings.as_ref().unwrap().revision(),
        revision
    );
    assert!(app.editor.start_play());
    sync_clock(&mut app.editor);
    app.irradiance
        .refresh_baked_validity(&app.editor, &app.models);
    assert_eq!(
        app.irradiance.grid_for_editor(&app.editor),
        Some(&document.grid),
        "static receipt remains valid in Play"
    );
    let capture_count = app.irradiance.bake_snapshot_captures();
    let initial_checksum = app.editor.checksum();
    for _ in 0..20 {
        app.editor.step(1);
        sync_clock(&mut app.editor);
        app.irradiance
            .refresh_baked_validity(&app.editor, &app.models);
        assert_eq!(
            app.irradiance.grid_for_editor(&app.editor),
            Some(&document.grid)
        );
        assert_eq!(app.irradiance.bake_snapshot_captures(), capture_count,
            "Play tick and dynamic-only motion must not rebuild/copy static mesh or texture snapshots");
    }
    assert_ne!(
        app.editor.checksum(),
        initial_checksum,
        "the dynamic Play frames actually advanced"
    );
    app.editor.stop();
    sync_clock(&mut app.editor);

    // Missing destination fails without clearing dirty state, undo, receipt or
    // the already saved bytes. No permission-sensitive filesystem trickery.
    app.irradiance.undo(&app.editor).unwrap();
    app.irradiance.redo(&app.editor).unwrap();
    let retained_revision = app.irradiance.bindings.as_ref().unwrap().revision();
    let retained_dirty = app.irradiance.bindings.as_ref().unwrap().dirty();
    app.irradiance.bindings.as_mut().unwrap().path = temp.path().join("missing/failure.json");
    assert!(app.irradiance.save().is_err());
    app.irradiance.bindings.as_mut().unwrap().path = path.clone();
    assert_eq!(
        app.irradiance.bindings.as_ref().unwrap().document(),
        &document
    );
    assert_eq!(
        app.irradiance.bindings.as_ref().unwrap().revision(),
        retained_revision
    );
    assert_eq!(
        app.irradiance.bindings.as_ref().unwrap().dirty(),
        retained_dirty
    );
    assert!(app.irradiance.bindings.as_ref().unwrap().can_undo());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);

    // A traced work-budget failure also publishes nothing, even after a
    // previous valid bake has been saved and remains in the viewport.
    app.irradiance.bake_settings.max_triangle_tests = 1;
    app.irradiance.start_bake(&app.editor, &app.models).unwrap();
    drain(&mut app);
    assert!(app.irradiance.bake_status().unwrap().contains("failed"));
    assert_eq!(
        app.irradiance.bindings.as_ref().unwrap().document(),
        &document
    );
    assert_eq!(
        app.irradiance.bindings.as_ref().unwrap().revision(),
        retained_revision
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    app.irradiance.bake_settings = orr_render::irradiance_bake::BakeSettings::default();
    app.irradiance.start_bake(&app.editor, &app.models).unwrap();
    assert!(app.editor.select_named("ground"));
    assert!(app.editor.set_yard_transform(
        orr_fp::FPVec3::new(orr_fp::FP::ZERO, orr_fp::FP::from_int(-2), orr_fp::FP::ZERO),
        [
            orr_fp::FP::ZERO,
            orr_fp::FP::ZERO,
            orr_fp::FP::ZERO,
            orr_fp::FP::ONE
        ],
    ));
    sync_clock(&mut app.editor);
    drain(&mut app);
    assert!(app.irradiance.bake_status().unwrap().contains("discarded"));
    assert!(app.irradiance.grid_for_editor(&app.editor).is_none());
    assert!(app.irradiance.baked_stale_reason().is_some());
    let capture_count = app.irradiance.bake_snapshot_captures();
    for _ in 0..8 {
        app.irradiance
            .refresh_baked_validity(&app.editor, &app.models);
        assert!(app.irradiance.grid_for_editor(&app.editor).is_none());
        assert_eq!(
            app.irradiance.bake_snapshot_captures(),
            capture_count,
            "unchanged stale input must not repeat a full snapshot each redraw"
        );
    }
    assert_eq!(
        app.irradiance.bindings.as_ref().unwrap().document(),
        &document
    );
    assert_eq!(
        app.irradiance.bindings.as_ref().unwrap().revision(),
        retained_revision
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

fn retarget_model_project(app: &mut EditorApp, project: &str) {
    let bindings = app.models.bindings.as_ref().unwrap();
    let path = bindings.path.clone();
    let mut document = bindings.document().clone();
    document.project = project.into();
    document.validate().unwrap();
    std::fs::write(path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
    app.models.open_for_editor(&app.editor, false).unwrap();
}

fn await_verified_grid(app: &mut EditorApp) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        sync_clock(&mut app.editor);
        app.irradiance
            .refresh_baked_validity(&app.editor, &app.models);
        if app.irradiance.grid_for_editor(&app.editor).is_some() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "source validation did not accept current root: {:?}",
            app.irradiance.baked_stale_reason()
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn same_content_project_retarget_revalidates_current_root_and_rejects_old_worker_authority() {
    let _serial = GPU_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/imported_scene_demo")
        .canonicalize()
        .unwrap();
    for root in [&first, &second] {
        Project::open_for_install(root, Runtime::content_only().engine_version)
            .unwrap()
            .install(std::slice::from_ref(&source))
            .unwrap();
    }
    let mut app = plain_app(&first);
    app.models.open_for_editor(&app.editor, true).unwrap();
    assign(&mut app, "ground", ModelKind::Static);
    app.models.bindings.as_mut().unwrap().save().unwrap();
    app.irradiance.start_bake(&app.editor, &app.models).unwrap();
    drain(&mut app);
    app.irradiance.save().unwrap();
    let document = app.irradiance.bindings.as_ref().unwrap().document().clone();
    assert_eq!(document.grid.provenance, IrradianceProvenance::Baked);
    let sidecar = app.irradiance.bindings.as_ref().unwrap().path.clone();
    let saved = std::fs::read(&sidecar).unwrap();
    let revision = app.irradiance.bindings.as_ref().unwrap().revision();
    let binding = app
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings
        .values()
        .next()
        .unwrap();
    let asset_suffix = Path::new(".orr/packages/objects")
        .join(&binding.package_digest)
        .join(&binding.asset);
    let first_asset = first.join(&asset_suffix);
    let second_asset = second.join(&asset_suffix);
    let source_bytes = std::fs::read(&first_asset).unwrap();
    assert_eq!(std::fs::read(&second_asset).unwrap(), source_bytes);

    // A valid published receipt has portable content identity. Repointing its
    // model sidecar requires fresh stamps for the currently selected location.
    retarget_model_project(&mut app, "../second");
    app.irradiance
        .refresh_baked_validity(&app.editor, &app.models);
    assert!(
        app.irradiance.grid_for_editor(&app.editor).is_none(),
        "old-root stamps cannot immediately enable the new root"
    );
    await_verified_grid(&mut app);
    assert_eq!(
        app.irradiance.bindings.as_ref().unwrap().document(),
        &document
    );
    let mut tampered = source_bytes.clone();
    *tampered.last_mut().unwrap() ^= 1;
    std::fs::write(&second_asset, &tampered).unwrap();
    app.irradiance
        .refresh_baked_validity(&app.editor, &app.models);
    assert!(
        app.irradiance.grid_for_editor(&app.editor).is_none(),
        "tampering with the current root must disable baked lighting"
    );
    assert!(app.irradiance.baked_stale_reason().is_some());
    assert_eq!(std::fs::read(&first_asset).unwrap(), source_bytes);
    assert_eq!(
        app.irradiance.bindings.as_ref().unwrap().revision(),
        revision
    );
    assert_eq!(std::fs::read(&sidecar).unwrap(), saved);

    // A failed source check at one root must not poison identical valid content
    // at another root through a portable-fingerprint-only failure memo.
    retarget_model_project(&mut app, ".");
    await_verified_grid(&mut app);
    assert_eq!(
        app.irradiance.grid_for_editor(&app.editor),
        Some(&document.grid)
    );
    std::fs::write(&second_asset, &source_bytes).unwrap();

    app.irradiance.start_bake(&app.editor, &app.models).unwrap();
    retarget_model_project(&mut app, "../second");
    drain(&mut app);
    assert!(
        app.irradiance.bake_status().unwrap().contains("discarded"),
        "a bake cannot commit old-root verification after retarget"
    );
    assert_eq!(
        app.irradiance.bindings.as_ref().unwrap().document(),
        &document
    );
    assert_eq!(
        app.irradiance.bindings.as_ref().unwrap().revision(),
        revision
    );
    assert_eq!(std::fs::read(&sidecar).unwrap(), saved);
    await_verified_grid(&mut app);

    // Also retarget while source-only reopen validation is outstanding. Its
    // result must be tied to the input location that launched it.
    app.irradiance.open_for_editor(&app.editor, false).unwrap();
    app.irradiance
        .refresh_baked_validity(&app.editor, &app.models);
    assert!(app.irradiance.is_validating_bake());
    retarget_model_project(&mut app, ".");
    await_verified_grid(&mut app);
    std::fs::write(&first_asset, &tampered).unwrap();
    app.irradiance
        .refresh_baked_validity(&app.editor, &app.models);
    assert!(
        app.irradiance.grid_for_editor(&app.editor).is_none(),
        "reopen validation must watch the newly selected root"
    );
    assert_eq!(std::fs::read(&second_asset).unwrap(), source_bytes);
    assert_eq!(
        app.irradiance.bindings.as_ref().unwrap().document(),
        &document
    );
    assert_eq!(std::fs::read(&sidecar).unwrap(), saved);
}

#[cfg(feature = "terrain")]
#[test]
fn terrain_attachment_suspends_baked_receipts_without_mutating_probes_and_detach_revalidates() {
    use orr_editor::terrain_document::NewTerrain;
    use orr_terrain::Edit;
    use orr_fp::FP;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut app = plain_app(&root);
    // Manual probes remain available in the bounded terrain slice.
    let scene = app.editor.sim().scene_path.as_ref().unwrap().clone();
    app.terrain.document.set_scene(Some(Path::new(&scene))).unwrap();
    app.terrain.document.new_local("terrain.orrt", NewTerrain::default()).unwrap();
    app.irradiance.set_terrain_attached(app.terrain.attached_for_editor(&app.editor));
    assert!(app.irradiance.grid_for_editor(&app.editor).is_some());
    assert!(app.irradiance.start_bake(&app.editor, &app.models).unwrap_err().contains("terrain"));
    app.terrain.document.discard();
    app.irradiance.set_terrain_attached(false);
    let manual = app.irradiance.bindings.as_ref().unwrap().document().clone();
    app.irradiance.start_bake(&app.editor, &app.models).unwrap();
    app.irradiance.set_terrain_attached(true);
    drain(&mut app);
    assert_eq!(app.irradiance.bindings.as_ref().unwrap().document(), &manual,
        "attachment cancels an in-flight bake before admission");
    app.irradiance.set_terrain_attached(false);
    app.irradiance.start_bake(&app.editor, &app.models).unwrap();
    drain(&mut app);await_verified_grid(&mut app);
    app.irradiance.save().unwrap();
    let path = app.irradiance.bindings.as_ref().unwrap().path.clone();
    let bytes = std::fs::read(&path).unwrap();
    let document = app.irradiance.bindings.as_ref().unwrap().document().clone();
    let revision = app.irradiance.bindings.as_ref().unwrap().revision();
    for height in [FP::ZERO, FP::from_int(2)] {
        if app.terrain.document.terrain().is_none() {
            app.terrain.document.new_local("terrain.orrt", NewTerrain::default()).unwrap();
        }
        app.terrain.document.apply(&[Edit::SetHeight{x:0,z:0,height}]).unwrap();
        app.irradiance.set_terrain_attached(app.terrain.attached_for_editor(&app.editor));
        app.irradiance.sync_bake_for_editor(&app.editor, &app.models);
        assert!(app.irradiance.grid_for_editor(&app.editor).is_none());
        assert!(app.irradiance.baked_stale_reason().unwrap().contains("terrain"));
        assert!(app.irradiance.start_bake(&app.editor, &app.models).is_err());
        assert_eq!(app.irradiance.bindings.as_ref().unwrap().document(), &document);
        assert_eq!(app.irradiance.bindings.as_ref().unwrap().revision(), revision);
        assert!(!app.irradiance.bindings.as_ref().unwrap().dirty());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    app.terrain.document.discard();app.irradiance.set_terrain_attached(false);
    app.irradiance.refresh_baked_validity(&app.editor, &app.models);
    assert!(app.irradiance.is_validating_bake(), "detach starts source revalidation");
    // Race: cancel validation with a transient attach/detach before it drains.
    app.irradiance.set_terrain_attached(true);
    app.irradiance.set_terrain_attached(false);
    await_verified_grid(&mut app);
    assert_eq!(app.irradiance.grid_for_editor(&app.editor), Some(&document.grid));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}
