//! HDR/bloom through the real Yard host, EditorApp and native egui texture.
//! Adapter creation is mandatory; ORR_YARD_CAPTURE_DIR saves PPM and RGBA16F evidence.
#![cfg(feature = "animated-models")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui_kittest::{Harness, kittest::Queryable};
use orr_editor::{
    Editor, EditorApp, Mode,
    app::BottomTab,
    game::EditorGame,
    model_bindings::{ModelKind, PlaybackMode},
    viewport3d::{AnimatedPlacement, ModelPlacement, Viewport3dGpu},
};
use orr_package::{Project, Runtime};
use orr_render::{PostProcessSettings, RenderList3D};
use orr_rhi::{Rhi, TextureFormat, Wgpu, WgpuOptions};
use serde_json::json;
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

type HostFrame = (u64, u64, Vec<u8>);
static GPU_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

fn gpu() -> Wgpu {
    let gpu = Wgpu::headless(WgpuOptions::default()).expect("mandatory Yard HDR GPU adapter");
    eprintln!(
        "Yard HDR adapter: {} software={}",
        gpu.adapter_name(),
        gpu.is_software()
    );
    gpu
}

fn save_hdr(name: &str, gpu: &Viewport3dGpu) {
    if let (Some(directory), Some(bytes)) = (
        std::env::var_os("ORR_YARD_CAPTURE_DIR"),
        gpu.read_hdr_rgba16f(),
    ) {
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(Path::new(&directory).join(format!("{name}.rgba16f")), bytes).unwrap();
        let size = gpu.target().size();
        std::fs::write(Path::new(&directory).join(format!("{name}.rgba16f.json")), format!("{{\"width\":{},\"height\":{},\"format\":\"RGBA16Float little-endian, tightly packed, top row first\"}}", size.0, size.1)).unwrap();
    }
}

fn configure(app: &mut EditorApp) {
    app.models.open_for_editor(&app.editor, true).unwrap();
    assign(app, "box_left", ModelKind::Animated);
    assign(app, "box_right", ModelKind::Static);
    app.editor.select(None);
}

fn list(app: &EditorApp) -> (RenderList3D, Vec<ModelPlacement>, Vec<AnimatedPlacement>) {
    let models = app.models.placements(&app.editor);
    let animated = app.models.animated_placements(&app.editor).unwrap();
    let hidden: Vec<_> = models
        .iter()
        .map(|p| p.entity)
        .chain(animated.iter().map(|p| p.entity))
        .collect();
    (
        app.editor.yard_frame().list(&hidden, None),
        models,
        animated,
    )
}

fn capture(h: &mut Harness<'_, EditorApp>, reference: &mut Viewport3dGpu, name: &str) -> Vec<u8> {
    settle(h);
    let app = h.state();
    let (list, models, animated) = list(app);
    assert_eq!(
        (
            models.len(),
            animated.len(),
            list.boxes.len(),
            list.planes.len()
        ),
        (1, 1, 1, 1)
    );
    let gpu = app.viewport3d_gpu().unwrap().gpu();
    assert_eq!(
        gpu.target().format(),
        TextureFormat::Rgba8Unorm,
        "egui only samples the final display-referred native texture"
    );
    assert_eq!(gpu.post_process_settings(), app.ui.yard_post_process);
    reference
        .render_mixed_post_processed(
            app.ui.viewport_px,
            &list,
            &app.editor.camera3d.camera(),
            &models,
            &animated,
            app.ui.yard_post_process,
        )
        .unwrap();
    let pixels = gpu.read_rgba8();
    assert_eq!(
        pixels,
        reference.read_rgba8(),
        "actual EditorApp texture equals the complete admitted mixed frame"
    );
    assert_eq!(gpu.read_hdr_rgba(), reference.read_hdr_rgba());
    save(name, app.ui.viewport_px, &pixels);
    save_hdr(name, gpu);
    pixels
}

fn display_channel(linear: f32, exposure: f32, tonemap: bool) -> u8 {
    let x = linear * exposure;
    let mapped = if tonemap {
        ((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14)).clamp(0.0, 1.0)
    } else {
        x.clamp(0.0, 1.0)
    };
    let srgb = if mapped <= 0.0031308 {
        mapped * 12.92
    } else {
        1.055 * mapped.powf(1.0 / 2.4) - 0.055
    };
    (srgb * 255.0).round() as u8
}

#[test]
fn production_editor_hdr_bloom_mixed_frame_toggles_resize_readback_and_display_encoding() {
    let _serial = GPU_SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    drop(gpu());
    let temp = tempfile::tempdir().unwrap();
    let scene = fixture(temp.path());
    let scene_bytes = std::fs::read(&scene).unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::Yard3D).unwrap();
    editor.camera3d = orr_render::OrbitCamera::new([0.0, 1.5, 0.0], 0.45, 0.65, 14.0);
    let expected = host_frame(&mut editor);
    let mut rhi = None;
    let mut h = Harness::builder()
        .with_size([1001.0, 803.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(|cc| {
            let state = cc.wgpu_render_state.as_ref().unwrap();
            rhi = Some(Wgpu::from_parts(
                state.instance.clone(),
                state.adapter.clone(),
                state.device.clone(),
                state.queue.clone(),
            ));
            let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
            app.ui.bottom_tab = BottomTab::History;
            app
        });
    settle(&mut h);
    configure(h.state_mut());
    let mut reference = Viewport3dGpu::new(&rhi.unwrap(), (1, 1));
    let legacy = capture(&mut h, &mut reference, "hdr-default-off");
    assert!(!h.state().ui.yard_post_process.enabled);
    assert!(
        h.state()
            .viewport3d_gpu()
            .unwrap()
            .gpu()
            .read_hdr_rgba()
            .is_none()
    );
    assert_eq!(
        h.state()
            .viewport3d_gpu()
            .unwrap()
            .gpu()
            .pipeline_generation(),
        0
    );
    let initial_size = h.state().ui.viewport_px;
    let history = h.state().editor.history().entries.len();

    // Exercise the actual user-visible opt-in rather than only a renderer flag.
    h.get_by_label("Viewport effects").click();
    h.run_steps(2);
    h.get_by_label("HDR post-processing").click();
    h.run_steps(2);
    h.key_press(egui::Key::Escape);
    h.state_mut().ui.yard_post_process.bloom = false;
    let no_bloom = capture(&mut h, &mut reference, "hdr-no-bloom");
    let raw = h
        .state()
        .viewport3d_gpu()
        .unwrap()
        .gpu()
        .read_hdr_rgba()
        .unwrap();
    assert_eq!(raw.len(), (initial_size.0 * initial_size.1) as usize);
    assert!(raw.iter().flatten().all(|v| v.is_finite()));
    let maximum = raw
        .iter()
        .flat_map(|p| &p[..3])
        .copied()
        .fold(0.0_f32, f32::max);
    eprintln!("Actual Yard mixed scene max unclamped linear HDR: {maximum}");
    assert!(
        maximum > 1.0,
        "real scene preserves highlights above one before output mapping"
    );
    let lighting = orr_sample::yard3d_view::yard_lighting();
    for (hdr, rgba) in raw.iter().zip(no_bloom.as_chunks::<4>().0.iter()) {
        for channel in 0..3 {
            let expected = display_channel(hdr[channel], lighting.exposure, lighting.tonemap);
            assert!(
                (i16::from(expected) - i16::from(rgba[channel])).abs() <= 2,
                "one exposure/ACES/sRGB conversion: HDR={hdr:?}, stored={rgba:?}, expected={expected}"
            );
        }
        assert_eq!(rgba[3], 255);
    }
    assert_eq!(
        h.state().viewport3d_gpu().unwrap().gpu().scene_format(),
        TextureFormat::Rgba16Float
    );
    assert_eq!(
        h.state()
            .viewport3d_gpu()
            .unwrap()
            .gpu()
            .model_cache_counts(),
        (1, 1)
    );
    let ui_without_bloom = h.render().unwrap();

    h.state_mut().ui.yard_post_process.bloom = true;
    h.state_mut().ui.yard_post_process.threshold = 0.7;
    h.state_mut().ui.yard_post_process.strength = 0.8;
    h.state_mut().ui.yard_post_process.radius = 4;
    h.state_mut().ui.yard_post_process.iterations = 2;
    let bloom = capture(&mut h, &mut reference, "hdr-bloom");
    assert_eq!(
        raw,
        h.state()
            .viewport3d_gpu()
            .unwrap()
            .gpu()
            .read_hdr_rgba()
            .unwrap(),
        "bloom never feeds back into scene color"
    );
    let expanded = raw
        .iter()
        .zip(no_bloom.as_chunks::<4>().0.iter())
        .zip(bloom.as_chunks::<4>().0.iter())
        .filter(|((hdr, off), on)| {
            hdr[..3].iter().copied().fold(0.0_f32, f32::max) < 0.7
                && (0..3).any(|c| i16::from(on[c]) - i16::from(off[c]) > 2)
        })
        .count();
    eprintln!("Actual Yard bloom reaches {expanded} sub-threshold pixels");
    assert!(expanded > 10, "bloom expands beyond bright source pixels");
    let ui_with_bloom = h.render().unwrap();
    let rect = h.state().ui.viewport_rect.unwrap();
    // UI panels are composed after the post process. The menu/title region is
    // outside the native scene texture and cannot be bloomed or tone mapped.
    let top = rect.min.y.max(1.0) as u32;
    assert_eq!(
        &ui_without_bloom.as_raw()[..(ui_without_bloom.width() * top * 4) as usize],
        &ui_with_bloom.as_raw()[..(ui_with_bloom.width() * top * 4) as usize]
    );
    save(
        "hdr-bloom-egui",
        (ui_with_bloom.width(), ui_with_bloom.height()),
        ui_with_bloom.as_raw(),
    );

    // Shared shadows affect the actual HDR scene before its final conversion.
    let app = h.state();
    let (mut unshadowed, models, animated) = list(app);
    unshadowed.lighting.shadows = false;
    reference
        .render_mixed_post_processed(
            initial_size,
            &unshadowed,
            &app.editor.camera3d.camera(),
            &models,
            &animated,
            app.ui.yard_post_process,
        )
        .unwrap();
    let lit = reference.read_hdr_rgba().unwrap();
    let shadow_pixels = raw
        .iter()
        .zip(&lit)
        .filter(|(on, off)| (0..3).map(|c| off[c] - on[c]).sum::<f32>() > 0.04)
        .count();
    assert!(
        shadow_pixels > 40,
        "mixed casters share scene-depth/shadows before post: {shadow_pixels}"
    );

    // Repeated accepted toggles rebuild every pipeline family coherently and
    // restore legacy bytes exactly. No hidden postprocess allocation survives off.
    for _ in 0..3 {
        h.state_mut().ui.yard_post_process.enabled = false;
        assert_eq!(capture(&mut h, &mut reference, "hdr-restored-off"), legacy);
        assert_eq!(
            h.state()
                .viewport3d_gpu()
                .unwrap()
                .gpu()
                .post_process_allocated_bytes(),
            0
        );
        h.state_mut().ui.yard_post_process.enabled = true;
        assert_eq!(capture(&mut h, &mut reference, "hdr-restored-on"), bloom);
    }
    assert_eq!(
        h.state()
            .viewport3d_gpu()
            .unwrap()
            .gpu()
            .pipeline_generation(),
        7
    );
    for dimensions in [[1013.0, 817.0], [991.0, 799.0], [1001.0, 803.0]] {
        h.set_size(egui::vec2(dimensions[0], dimensions[1]));
        capture(&mut h, &mut reference, "hdr-resized");
        let gpu = h.state().viewport3d_gpu().unwrap().gpu();
        assert_eq!(gpu.post_process_size(), Some(h.state().ui.viewport_px));
        assert_eq!(
            gpu.pipeline_generation(),
            7,
            "size changes do not leave or rebuild stale-format models"
        );
    }
    assert_eq!(
        capture(&mut h, &mut reference, "hdr-resize-restored"),
        bloom
    );
    // Put both imports wholly inside the procedural cube using view-only
    // placements. With shared main depth their color is completely occluded,
    // even though static/skinned batches are submitted after the procedural one.
    {
        let app = h.state_mut();
        app.models.transform.scale = [0.04; 3];
        app.models.transform.translation = [2.0, 0.0, -3.0];
        assign(app, "box_left", ModelKind::Animated);
        app.models.transform.scale = [0.04; 3];
        app.models.transform.translation = [-2.0, -2.0, -3.0];
        assign(app, "box_right", ModelKind::Static);
        app.editor.select(None);
    }
    capture(&mut h, &mut reference, "hdr-mixed-occluded");
    let app = h.state();
    let (occluder, _, _) = list(app);
    let occluded_hdr = app.viewport3d_gpu().unwrap().gpu().read_hdr_rgba().unwrap();
    let occluded_final = app.viewport3d_gpu().unwrap().gpu().read_rgba8();
    reference
        .render_mixed_post_processed(
            app.ui.viewport_px,
            &occluder,
            &app.editor.camera3d.camera(),
            &[],
            &[],
            app.ui.yard_post_process,
        )
        .unwrap();
    assert_eq!(
        occluded_hdr,
        reference.read_hdr_rgba().unwrap(),
        "procedural depth hides later static and skinned batches before bloom"
    );
    assert_eq!(
        occluded_final,
        reference.read_rgba8(),
        "occluded imports cannot leak into bloom or final color"
    );
    assert_eq!(
        host_frame(&mut h.state_mut().editor),
        expected,
        "view settings never mutate host bytes"
    );
    assert_eq!(h.state().editor.history().entries.len(), history);
    assert_eq!(std::fs::read(scene).unwrap(), scene_bytes);
}

#[derive(Debug, PartialEq)]
struct Retained {
    pixels: Vec<u8>,
    hdr: Option<Vec<[f32; 4]>>,
    size: (u32, u32),
    target_generation: u64,
    pipeline_generation: u64,
    post_generation: u64,
    allocated: u64,
    settings: PostProcessSettings,
    format: TextureFormat,
    caches: (usize, usize),
    bounds: Vec<Vec<orr_render::SkinnedBounds>>,
}
fn retained(gpu: &Viewport3dGpu) -> Retained {
    Retained {
        pixels: gpu.read_rgba8(),
        hdr: gpu.read_hdr_rgba(),
        size: gpu.target().size(),
        target_generation: gpu.target().generation(),
        pipeline_generation: gpu.pipeline_generation(),
        post_generation: gpu.post_process_generation(),
        allocated: gpu.post_process_allocated_bytes(),
        settings: gpu.post_process_settings(),
        format: gpu.scene_format(),
        caches: gpu.model_cache_counts(),
        bounds: gpu.animated_bounds(),
    }
}

#[test]
fn mixed_hdr_viewport_rejects_late_invalid_batches_settings_budgets_and_unsupported_atomically() {
    let _serial = GPU_SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let rhi = gpu();
    let temp = tempfile::tempdir().unwrap();
    let scene = fixture(temp.path());
    let mut app = EditorApp::new(Editor::open_game(&scene, EditorGame::Yard3D).unwrap(), None);
    sync_clock(&mut app.editor);
    configure(&mut app);
    let (list, models, animated) = list(&app);
    let camera = app.editor.camera3d.camera();
    let mut viewport = Viewport3dGpu::new(&rhi, (161, 129));
    let hdr = PostProcessSettings {
        enabled: true,
        bloom: true,
        ..PostProcessSettings::default()
    };
    viewport
        .render_mixed_post_processed((161, 129), &list, &camera, &models, &animated, hdr)
        .unwrap();
    let accepted = retained(&viewport);
    let mut invalid_settings = Vec::new();
    for threshold in [f32::NAN, f32::INFINITY, -0.1, 65505.0] {
        invalid_settings.push(PostProcessSettings { threshold, ..hdr });
    }
    for strength in [f32::NAN, -0.1, 8.1] {
        invalid_settings.push(PostProcessSettings { strength, ..hdr });
    }
    for radius in [0, 9] {
        invalid_settings.push(PostProcessSettings { radius, ..hdr });
    }
    for iterations in [0, 5] {
        invalid_settings.push(PostProcessSettings { iterations, ..hdr });
    }
    for settings in invalid_settings {
        assert!(
            viewport
                .render_mixed_post_processed(
                    (173, 137),
                    &list,
                    &camera,
                    &models,
                    &animated,
                    settings
                )
                .is_err()
        );
        assert_eq!(retained(&viewport), accepted);
    }
    for size in [
        (0, 1),
        (1, 0),
        (8193, 4),
        (8192, 8192),
        (u32::MAX, u32::MAX),
    ] {
        assert!(
            viewport
                .render_mixed_post_processed(size, &list, &camera, &models, &animated, hdr)
                .is_err(),
            "oversize {size:?}"
        );
        assert_eq!(retained(&viewport), accepted);
    }
    // A new valid static asset before a late invalid skinned batch must not enter
    // the cache or evict/rebuild admitted renderers on a requested HDR-off switch.
    let new_model = ModelPlacement {
        entity: models[0].entity,
        model: std::sync::Arc::new((*models[0].model).clone()),
        instance: models[0].instance,
    };
    let foreign =
        orr_model::animation::AnimatedModel::new(animated[0].model.source().clone()).unwrap();
    let bad_animated = AnimatedPlacement {
        entity: animated[0].entity,
        model: animated[0].model.clone(),
        instance: animated[0].instance,
        pose: foreign.rest_pose().unwrap(),
    };
    assert!(
        viewport
            .render_mixed_post_processed(
                (173, 137),
                &list,
                &camera,
                &[new_model],
                &[bad_animated],
                PostProcessSettings::default()
            )
            .is_err()
    );
    assert_eq!(retained(&viewport), accepted);
    let mut bad_static = ModelPlacement {
        entity: models[0].entity,
        model: models[0].model.clone(),
        instance: models[0].instance,
    };
    bad_static.instance.translation[0] = f32::INFINITY;
    assert!(
        viewport
            .render_mixed_post_processed((173, 137), &list, &camera, &[bad_static], &animated, hdr)
            .is_err()
    );
    assert_eq!(retained(&viewport), accepted);
    let mut bad_animated = AnimatedPlacement {
        entity: animated[0].entity,
        model: animated[0].model.clone(),
        instance: animated[0].instance,
        pose: animated[0].pose.clone(),
    };
    bad_animated.instance[3][0] = 1e20;
    assert!(
        viewport
            .render_mixed_post_processed((173, 137), &list, &camera, &models, &[bad_animated], hdr)
            .is_err()
    );
    assert_eq!(retained(&viewport), accepted);
    let mut bad_list = list.clone();
    bad_list.boxes[0].color[0] = f32::NAN;
    assert!(
        viewport
            .render_mixed_post_processed((173, 137), &bad_list, &camera, &models, &animated, hdr)
            .is_err()
    );
    assert_eq!(retained(&viewport), accepted);
    bad_list = list.clone();
    bad_list.lighting.exposure = f32::INFINITY;
    assert!(
        viewport
            .render_mixed_post_processed((173, 137), &bad_list, &camera, &models, &animated, hdr)
            .is_err()
    );
    assert_eq!(retained(&viewport), accepted);

    let restricted = rhi.clone().without_hdr_support();
    assert_eq!(restricted.adapter_name(), rhi.adapter_name());
    let mut legacy = Viewport3dGpu::new(&restricted, (161, 129));
    legacy
        .render_mixed((161, 129), &list, &camera, &models, &animated)
        .unwrap();
    let prior = retained(&legacy);
    let error = legacy
        .render_mixed_post_processed((173, 137), &list, &camera, &models, &animated, hdr)
        .unwrap_err();
    assert!(
        error.to_lowercase().contains("unsupported") || error.to_lowercase().contains("support"),
        "{error}"
    );
    assert_eq!(retained(&legacy), prior);
    legacy
        .render_mixed((161, 129), &list, &camera, &models, &animated)
        .unwrap();
    assert_eq!(
        retained(&legacy),
        prior,
        "legacy remains usable after unsupported HDR"
    );

    // Empty/odd/tiny frames clear the HDR scene and all temporary bloom targets.
    let empty = RenderList3D::default();
    for size in [(1, 1), (7, 5), (163, 131)] {
        viewport
            .render_mixed_post_processed(
                size,
                &empty,
                &camera,
                &[],
                &[],
                PostProcessSettings {
                    threshold: 10.0,
                    ..hdr
                },
            )
            .unwrap();
        let raw = viewport.read_hdr_rgba().unwrap();
        assert!(raw.iter().all(|p| p.iter().all(|c| c.is_finite())));
        assert!(
            raw.windows(2).all(|p| p[0] == p[1]),
            "empty HDR frame is its clear color"
        );
        let bytes = viewport.read_rgba8();
        assert!(
            bytes.as_chunks::<4>().0.iter().all(|p| p == &bytes[..4]),
            "no stale bright geometry in an empty final frame"
        );
        for c in 0..3 {
            assert!(
                (i16::from(bytes[c])
                    - i16::from(display_channel(
                        raw[0][c],
                        empty.lighting.exposure,
                        empty.lighting.tonemap
                    )))
                .abs()
                    <= 2
            );
        }
        assert_eq!(viewport.model_cache_counts(), (0, 0));
    }
}

#[test]
fn actual_editor_unsupported_opt_in_keeps_legacy_texture_mode_and_adapter() {
    let _serial = GPU_SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    drop(gpu());
    let temp = tempfile::tempdir().unwrap();
    let scene = fixture(temp.path());
    let editor = Editor::open_game(&scene, EditorGame::Yard3D).unwrap();
    let mut h = Harness::builder()
        .with_size([901.0, 703.0])
        .wgpu()
        .build_eframe(|cc| {
            EditorApp::new(editor, cc.wgpu_render_state.clone()).without_hdr_support()
        });
    settle(&mut h);
    configure(h.state_mut());
    settle(&mut h);
    let prior = retained(h.state().viewport3d_gpu().unwrap().gpu());
    let adapter = h.state().viewport3d_gpu().unwrap().gpu().adapter_name();
    h.state_mut().ui.yard_post_process.enabled = true;
    h.run_steps(1);
    assert_eq!(retained(h.state().viewport3d_gpu().unwrap().gpu()), prior);
    assert!(
        !h.state().ui.yard_post_process.enabled,
        "rejected opt-in restores the admitted mode"
    );
    assert!(h.state().ui.yard_post_process_error.is_some());
    assert_eq!(
        h.state().viewport3d_gpu().unwrap().gpu().adapter_name(),
        adapter
    );
    settle(&mut h);
    assert_eq!(retained(h.state().viewport3d_gpu().unwrap().gpu()), prior);
    h.render()
        .expect("egui continues composing the previous native texture");
}
