//! Shared shadows through the real Yard host, EditorApp and native egui texture.
//! Set ORR_REQUIRE_GPU=1 for acceptance; ORR_YARD_CAPTURE_DIR saves PPM evidence.
#![cfg(feature = "animated-models")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui_kittest::Harness;
use orr_editor::{
    app::BottomTab,
    game::EditorGame,
    model_bindings::{ModelKind, PlaybackMode},
    viewport3d::Viewport3dGpu,
    Editor, EditorApp, Mode,
};
use orr_package::{Project, Runtime};
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use serde_json::json;
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

type HostFrame = (u64, u64, Vec<u8>);

fn fixture(root: &Path) -> PathBuf {
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    Project::open_for_install(root, Runtime::content_only().engine_version)
        .unwrap()
        .install(&[
            assets.join("animation_demo").canonicalize().unwrap(),
            assets.join("imported_scene_demo").canonicalize().unwrap(),
        ])
        .unwrap();
    // Freeze body motion, so moving shadows can only come from the sampled
    // skeleton. Leave one box and the ground procedural beside both imports.
    let mut text = include_str!("../../../scenes/yard3d_authoring.scene.yaml")
        .replace("kind: dynamic", "kind: static")
        .replace("inv_mass: 1", "inv_mass: 0");
    let fourth = text
        .split("  e_00000003:\n")
        .nth(1)
        .unwrap()
        .replace("name: box_right", "name: procedural_box")
        .replace("pos: [2, 3, 0]", "pos: [0, 1, -3]");
    text.push_str("  e_00000004:\n");
    text.push_str(&fourth);
    let path = root.join("yard-shadows.scene.yaml");
    std::fs::write(&path, text).unwrap();
    path
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
    let client = editor.agent_client("yard-shadow-byte-probe").unwrap();
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

fn baseline(editor: &mut Editor) -> [HostFrame; 3] {
    let edit = host_frame(editor);
    assert!(editor.start_play());
    let zero = host_frame(editor);
    editor.step(30);
    let thirty = host_frame(editor);
    assert!(editor.stop().is_some());
    assert_eq!(edit, host_frame(editor));
    [edit, zero, thirty]
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
    app.models.animation.clip_index = 0;
    app.models.animation.playback = PlaybackMode::Loop;
    app.models.assign(&app.editor).unwrap();
}

fn settle(harness: &mut Harness<'_, EditorApp>) {
    for _ in 0..3 {
        sync_clock(&mut harness.state_mut().editor);
        harness.run_steps(2);
    }
    assert!(harness.state().editor.down().is_none());
    assert!(harness.state().editor.yard_rows_coherent());
}

fn save(name: &str, size: (u32, u32), rgba: &[u8]) {
    if let Some(directory) = std::env::var_os("ORR_YARD_CAPTURE_DIR") {
        std::fs::create_dir_all(&directory).unwrap();
        let mut file =
            std::fs::File::create(Path::new(&directory).join(format!("{name}.ppm"))).unwrap();
        write!(file, "P6\n{} {}\n255\n", size.0, size.1).unwrap();
        for pixel in rgba.as_chunks::<4>().0.iter() {
            file.write_all(&pixel[..3]).unwrap();
        }
    }
}

struct Capture {
    shadowed: Vec<u8>,
    unshadowed: Vec<u8>,
}

fn darkened(shadowed: &[u8], unshadowed: &[u8]) -> bool {
    (0..3)
        .map(|c| unshadowed[c] as i32 - shadowed[c] as i32)
        .sum::<i32>()
        > 24
}

fn capture(
    harness: &mut Harness<'_, EditorApp>,
    reference: &mut Viewport3dGpu,
    name: &str,
    expected: &HostFrame,
) -> Capture {
    settle(harness);
    assert_eq!(&host_frame(&mut harness.state_mut().editor), expected);
    let app = harness.state();
    let size = app.ui.viewport_px;
    let camera = app.editor.camera3d.camera();
    let static_models = app.models.placements(&app.editor);
    let animated = app.models.animated_placements(&app.editor).unwrap();
    assert_eq!(static_models.len(), 1);
    assert_eq!(animated.len(), 1);
    let hidden: Vec<_> = static_models
        .iter()
        .map(|placement| placement.entity)
        .chain(animated.iter().map(|placement| placement.entity))
        .collect();
    let mut list = app.editor.yard_frame().list(&hidden, None);
    assert!(list.lighting.shadows, "production Yard must enable shadows");
    assert_eq!(list.planes.len(), 1, "procedural ground receiver");
    assert_eq!(list.boxes.len(), 1, "procedural caster beside both imports");
    let shadowed = app
        .viewport3d_gpu()
        .expect("real EditorApp main viewport")
        .gpu()
        .read_rgba8();
    reference
        .render_mixed(size, &list, &camera, &static_models, &animated)
        .unwrap();
    assert_eq!(
        shadowed,
        reference.read_rgba8(),
        "the actual native viewport renders the complete admitted shadowed frame"
    );
    list.lighting.shadows = false;
    reference
        .render_mixed(size, &list, &camera, &static_models, &animated)
        .unwrap();
    let unshadowed = reference.read_rgba8();
    let changed = shadowed
        .as_chunks::<4>().0.iter()
        .zip(unshadowed.as_chunks::<4>().0.iter())
        .filter(|(on, off)| darkened(*on, *off))
        .count();
    eprintln!("{name}: {changed} shadow-darkened main-viewport pixels");
    assert!(
        changed > 40,
        "{name}: actual Yard shared shadows darken {changed} pixels"
    );
    save(name, size, &shadowed);
    save(&format!("{name}-no-shadows"), size, &unshadowed);
    let composed = harness
        .render()
        .expect("egui composes the native Yard texture");
    save(
        &format!("{name}-egui"),
        (composed.width(), composed.height()),
        composed.as_raw(),
    );
    assert_eq!(
        &host_frame(&mut harness.state_mut().editor),
        expected,
        "shadow map rendering and toggling never mutate authoritative bytes"
    );
    Capture {
        shadowed,
        unshadowed,
    }
}

#[test]
fn production_yard_shared_shadows_follow_coherent_pause_step_seek_stop_and_reload() {
    match Wgpu::headless(WgpuOptions::default()) {
        Ok(gpu) => eprintln!(
            "Yard shadow adapter: {} software={}",
            gpu.adapter_name(),
            gpu.is_software()
        ),
        Err(error) => {
            assert!(
                std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "mandatory Yard shadow GPU unavailable: {error}"
            );
            eprintln!("SKIP: Yard shadow GPU unavailable: {error}");
            return;
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let scene = fixture(temp.path());
    let scene_bytes = std::fs::read(&scene).unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::Yard3D).unwrap();
    editor.camera3d = orr_render::OrbitCamera::new([0.0, 1.5, 0.0], 0.45, 0.65, 14.0);
    let expected = baseline(&mut editor);
    let mut comparison_rhi = None;
    let mut harness = Harness::builder()
        .with_size([1200.0, 900.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(|cc| {
            let state = cc.wgpu_render_state.as_ref().unwrap();
            comparison_rhi = Some(Wgpu::from_parts(
                state.instance.clone(),
                state.adapter.clone(),
                state.device.clone(),
                state.queue.clone(),
            ));
            let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
            // Fixed panel height across Edit/Play makes exact pixel comparisons useful.
            app.ui.bottom_tab = BottomTab::History;
            app
        });
    settle(&mut harness);
    {
        let app = harness.state_mut();
        app.models.open_for_editor(&app.editor, true).unwrap();
        assign(app, "box_left", ModelKind::Animated);
        assign(app, "box_right", ModelKind::Static);
        app.editor.select(None);
        app.models.bindings.as_mut().unwrap().save().unwrap();
    }
    let mut reference = Viewport3dGpu::new(&comparison_rhi.unwrap(), (1, 1));
    let edit = capture(&mut harness, &mut reference, "shadows-edit", &expected[0]);
    let size = harness.state().ui.viewport_px;
    // Validate the editor wrapper's fail-closed boundary after a visible frame.
    // A bad light must not resize the target, touch cached assets or erase it.
    {
        let app = harness.state();
        let static_models = app.models.placements(&app.editor);
        let animated = app.models.animated_placements(&app.editor).unwrap();
        let hidden: Vec<_> = static_models
            .iter()
            .map(|placement| placement.entity)
            .chain(animated.iter().map(|placement| placement.entity))
            .collect();
        let list = app.editor.yard_frame().list(&hidden, None);
        let camera = app.editor.camera3d.camera();
        reference
            .render_mixed(size, &list, &camera, &static_models, &animated)
            .unwrap();
        let generation = reference.target().generation();
        let visible = reference.read_rgba8();
        for bad_radius in [-1.0, 0.0, f32::NAN, f32::INFINITY] {
            let mut bad = list.clone();
            bad.lighting.shadow_radius = bad_radius;
            assert!(reference
                .render_mixed(
                    (size.0 + 8, size.1 + 8),
                    &bad,
                    &camera,
                    &static_models,
                    &animated
                )
                .is_err());
            assert_eq!(reference.target().generation(), generation);
            assert_eq!(reference.read_rgba8(), visible);
        }
        let mut bad = list.clone();
        bad.lighting.shadow_center[1] = f32::NAN;
        assert!(reference
            .render_mixed(
                (size.0 + 8, size.1 + 8),
                &bad,
                &camera,
                &static_models,
                &animated
            )
            .is_err());
        assert_eq!(reference.target().generation(), generation);
        assert_eq!(reference.read_rgba8(), visible);
        reference
            .render_mixed(size, &list, &camera, &static_models, &animated)
            .unwrap();
        assert_eq!(reference.read_rgba8(), visible);
    }
    let history_len = harness.state().editor.history().entries.len();
    assert!(harness.state_mut().editor.start_play());
    let zero = capture(&mut harness, &mut reference, "shadows-play0", &expected[1]);
    assert_eq!(harness.state().ui.viewport_px, size);
    harness.state_mut().editor.step(30);
    let thirty = capture(&mut harness, &mut reference, "shadows-step30", &expected[2]);
    let moving_shadow_on_fixed_receiver = zero
        .shadowed
        .as_chunks::<4>().0.iter()
        .zip(zero.unshadowed.as_chunks::<4>().0.iter())
        .zip(
            thirty
                .shadowed
                .as_chunks::<4>().0.iter()
                .zip(thirty.unshadowed.as_chunks::<4>().0.iter()),
        )
        .filter(|((on_zero, off_zero), (on_thirty, off_thirty))| {
            off_zero == off_thirty && darkened(*on_zero, *off_zero) != darkened(*on_thirty, *off_thirty)
        })
        .count();
    eprintln!(
        "moving skeletal shadow: {moving_shadow_on_fixed_receiver} unchanged receiver pixels"
    );
    assert!(
        moving_shadow_on_fixed_receiver > 20,
        "skeletal shadow moves on unchanged receiver pixels: {moving_shadow_on_fixed_receiver}"
    );
    harness.state_mut().editor.pause();
    let paused = capture(
        &mut harness,
        &mut reference,
        "shadows-pause30",
        &expected[2],
    );
    assert_eq!(thirty.shadowed, paused.shadowed);
    harness.state_mut().editor.seek(0);
    let seek_zero = capture(&mut harness, &mut reference, "shadows-seek0", &expected[1]);
    assert_eq!(zero.shadowed, seek_zero.shadowed);
    harness.state_mut().editor.seek(30);
    let seek_thirty = capture(&mut harness, &mut reference, "shadows-seek30", &expected[2]);
    assert_eq!(thirty.shadowed, seek_thirty.shadowed);
    assert!(harness.state_mut().editor.stop().is_some());
    let stopped = capture(&mut harness, &mut reference, "shadows-stop", &expected[0]);
    assert_eq!(edit.shadowed, stopped.shadowed);
    {
        let app = harness.state_mut();
        app.models.reload(&app.editor).unwrap();
    }
    let reloaded = capture(&mut harness, &mut reference, "shadows-reload", &expected[0]);
    assert_eq!(edit.shadowed, reloaded.shadowed);
    assert_eq!(harness.state().editor.history().entries.len(), history_len);
    assert_eq!(std::fs::read(&scene).unwrap(), scene_bytes);
    assert!(harness.state_mut().editor.open_path(&scene));
    // Reopen replaces the host session. Wait for its fenced GUID map before
    // resolving the persisted sidecar against the new authoritative snapshot.
    settle(&mut harness);
    {
        let app = harness.state_mut();
        app.models.open_for_editor(&app.editor, false).unwrap();
    }
    let reopened = capture(&mut harness, &mut reference, "shadows-reopen", &expected[0]);
    assert_eq!(edit.shadowed, reopened.shadowed);
    assert_eq!(std::fs::read(&scene).unwrap(), scene_bytes);
}
