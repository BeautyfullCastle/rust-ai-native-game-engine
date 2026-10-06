//! Authored/imported irradiance through the real Yard host and EditorApp texture.
//! Run with irradiance-probes and with irradiance-probes,animated-models. Both
//! configurations require a GPU adapter. IRRADIANCE_CAPTURE_DIR saves PNG proof.
#![cfg(feature = "irradiance-probes")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui_kittest::{Harness, kittest::Queryable};
use orr_editor::{
    Editor, EditorApp, Mode,
    app::BottomTab,
    game::EditorGame,
    model_bindings::ModelKind,
    viewport3d::{ModelPlacement, Viewport3dGpu},
};
#[cfg(feature = "animated-models")]
use orr_editor::{model_bindings::PlaybackMode, viewport3d::AnimatedPlacement};
use orr_package::{Project, Runtime};
use orr_render::{
    Camera3D, PostProcessSettings, RenderList3D,
    irradiance::{IrradianceGrid, IrradianceProvenance, IrradianceUniform, constant_irradiance},
};
use orr_rhi::{Rhi, TextureFormat, Wgpu, WgpuOptions};
use serde_json::json;
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
        .expect("mandatory production Yard irradiance GPU adapter");
    eprintln!(
        "Yard irradiance adapter: {} software={} animated={}",
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
    assert!(harness.state().editor.down().is_none());
    assert!(harness.state().editor.yard_rows_coherent());
}

struct SceneInputs {
    list: RenderList3D,
    models: Vec<ModelPlacement>,
    #[cfg(feature = "animated-models")]
    animated: Vec<AnimatedPlacement>,
}

fn inputs(app: &EditorApp) -> SceneInputs {
    let models = app.models.placements(&app.editor);
    let hidden: Vec<_> = models.iter().map(|p| p.entity).collect();
    #[cfg(feature = "animated-models")]
    let animated = app.models.animated_placements(&app.editor).unwrap();
    #[cfg(feature = "animated-models")]
    let hidden = {
        let mut hidden = hidden;
        hidden.extend(animated.iter().map(|p| p.entity));
        hidden
    };
    let list = app.editor.yard_frame().list(&hidden, None);
    assert_eq!(
        (models.len(), list.boxes.len(), list.planes.len()),
        (1, 1, 1)
    );
    #[cfg(feature = "animated-models")]
    assert_eq!(animated.len(), 1);
    SceneInputs {
        list,
        models,
        #[cfg(feature = "animated-models")]
        animated,
    }
}

fn render(
    gpu: &mut Viewport3dGpu,
    size: (u32, u32),
    inputs: &SceneInputs,
    camera: &Camera3D,
    post: PostProcessSettings,
    grid: Option<&IrradianceGrid>,
) -> Result<(), String> {
    gpu.render_irradiance(
        size,
        &inputs.list,
        camera,
        &inputs.models,
        #[cfg(feature = "animated-models")]
        &inputs.animated,
        post,
        grid,
    )
}

fn capture_dir() -> Option<PathBuf> {
    let root = PathBuf::from(std::env::var_os("IRRADIANCE_CAPTURE_DIR")?);
    let directory = root.join(if cfg!(feature = "animated-models") {
        "mixed"
    } else {
        "static"
    });
    std::fs::create_dir_all(&directory).unwrap();
    Some(directory)
}

fn capture(h: &mut Harness<'_, EditorApp>, reference: &mut Viewport3dGpu, name: &str) -> Vec<u8> {
    settle(h);
    let app = h.state();
    let inputs = inputs(app);
    let actual = app.viewport3d_gpu().unwrap().gpu();
    render(
        reference,
        app.ui.viewport_px,
        &inputs,
        &app.editor.camera3d.camera(),
        app.ui.yard_post_process,
        app.irradiance.grid_for_editor(&app.editor),
    )
    .unwrap();
    assert_eq!(actual.target().format(), TextureFormat::Rgba8Unorm);
    assert_eq!(
        actual.irradiance(),
        app.irradiance.grid_for_editor(&app.editor)
    );
    assert_eq!(actual.post_process_settings(), app.ui.yard_post_process);
    let pixels = actual.read_rgba8();
    assert_eq!(
        pixels,
        reference.read_rgba8(),
        "actual EditorApp texture must equal the complete admitted frame"
    );
    assert_eq!(actual.read_hdr_rgba(), reference.read_hdr_rgba());
    let expected_caches = (1, usize::from(cfg!(feature = "animated-models")));
    assert_eq!(actual.model_cache_counts(), expected_caches);
    assert_eq!(
        actual.irradiance_uniform_bytes(),
        (2 + expected_caches.0 + expected_caches.1) * std::mem::size_of::<IrradianceUniform>(),
        "probe storage is one bounded uniform per renderer, independent of toggles"
    );
    if let Some(directory) = capture_dir() {
        let file = std::fs::File::create(directory.join(format!("{name}-viewport.png"))).unwrap();
        let mut encoder = png::Encoder::new(file, app.ui.viewport_px.0, app.ui.viewport_px.1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        if let Some(bytes) = actual.read_hdr_rgba16f() {
            std::fs::write(directory.join(format!("{name}.rgba16f")), bytes).unwrap();
        }
    }
    let composed = h
        .render()
        .expect("egui samples the production native Yard texture");
    if let Some(directory) = capture_dir() {
        composed
            .save(directory.join(format!("{name}-editor.png")))
            .unwrap();
    }
    pixels
}

fn authored_grid(rgb: [f32; 3]) -> IrradianceGrid {
    IrradianceGrid {
        enabled: true,
        provenance: IrradianceProvenance::Authored,
        dimensions: [4; 3],
        origin: [-8.0, -4.0, -8.0],
        spacing: [6.0, 4.0, 6.0],
        coefficients: vec![constant_irradiance(rgb).unwrap(); 64],
        ..IrradianceGrid::default()
    }
}

fn apply(h: &mut Harness<'_, EditorApp>, grid: IrradianceGrid) {
    let app = h.state_mut();
    app.irradiance
        .apply_grid_for_editor(&app.editor, grid)
        .unwrap();
}

#[test]
fn production_editor_irradiance_import_author_save_reopen_and_toggles_preserve_host() {
    let _serial = GPU_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    drop(gpu());
    let temp = tempfile::tempdir().unwrap();
    let scene = fixture(temp.path());
    let scene_bytes = std::fs::read(&scene).unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::Yard3D).unwrap();
    editor.camera3d = orr_render::OrbitCamera::new([0.0, 1.5, 0.0], 0.45, 0.65, 14.0);
    let expected_host = host_frame(&mut editor);
    let camera = editor.camera3d;
    let mut comparison = None;
    let mut h = Harness::builder()
        .with_size([1001.0, 803.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(|cc| {
            let state = cc.wgpu_render_state.as_ref().unwrap();
            comparison = Some(Wgpu::from_parts(
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
    h.get_by_label("Irradiance probes").click();
    h.run_steps(2);
    let history = h.state().editor.history().entries.len();
    let mut reference = Viewport3dGpu::new(&comparison.unwrap(), (1, 1));
    let off = capture(&mut h, &mut reference, "probe-off");
    assert!(
        h.state()
            .irradiance
            .grid_for_editor(&h.state().editor)
            .is_none()
    );
    assert!(
        !h.state()
            .irradiance
            .bindings
            .as_ref()
            .unwrap()
            .document()
            .grid
            .enabled
    );
    let off_ui = h.render().unwrap();
    let authored = authored_grid([4.0, 0.6, 0.15]);
    authored.validate().unwrap();
    apply(&mut h, authored.clone());
    let on = capture(&mut h, &mut reference, "probe-authored-on");
    let changed = on
        .chunks_exact(4)
        .zip(off.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        changed > 100,
        "authored local diffuse visibly changes scene pixels: {changed}"
    );
    let on_ui = h.render().unwrap();
    let rect = h.state().ui.viewport_rect.unwrap();
    let composed_changed = on_ui
        .enumerate_pixels()
        .filter(|(x, y, p)| {
            let point = egui::pos2(*x as f32, *y as f32);
            rect.shrink(4.0).contains(point)
                && point.y > rect.min.y + 40.0
                && *p != off_ui.get_pixel(*x, *y)
        })
        .count();
    assert!(
        composed_changed > 100,
        "irradiance reaches the displayed native egui viewport: {composed_changed}"
    );
    {
        let app = h.state_mut();
        app.irradiance.undo(&app.editor).unwrap();
    }
    assert_eq!(capture(&mut h, &mut reference, "probe-undo-off"), off);
    {
        let app = h.state_mut();
        app.irradiance.redo(&app.editor).unwrap();
    }
    assert_eq!(capture(&mut h, &mut reference, "probe-redo-on"), on);
    h.get_by_label("Enable irradiance probes").click();
    assert_eq!(capture(&mut h, &mut reference, "probe-checkbox-off"), off);
    h.get_by_label("Enable irradiance probes").click();
    assert_eq!(capture(&mut h, &mut reference, "probe-checkbox-on"), on);

    // A finite valid volume outside the scene has exact legacy fallback,
    // independent of its bright coefficients.
    let mut outside = authored.clone();
    outside.origin = [200.0; 3];
    apply(&mut h, outside);
    assert_eq!(
        capture(&mut h, &mut reference, "probe-outside-fallback"),
        off
    );
    apply(&mut h, authored.clone());

    // Resolve the shipped package through the same verified authoring control
    // used by the panel before exercising a separate full-SH9 JSON import.
    {
        let app = h.state_mut();
        app.irradiance.package = "sample-irradiance-grid".into();
        app.irradiance.asset = "authored-room.irradiance.json".into();
        app.irradiance
            .import_package_for_editor(&app.editor)
            .unwrap();
        assert_eq!(
            app.irradiance
                .grid_for_editor(&app.editor)
                .unwrap()
                .provenance,
            IrradianceProvenance::Imported
        );
    }
    assert_ne!(
        capture(&mut h, &mut reference, "probe-package-imported"),
        off
    );

    let mut imported = authored_grid([0.2, 1.5, 4.0]);
    imported.provenance = IrradianceProvenance::Imported;
    {
        let app = h.state_mut();
        app.irradiance
            .import_json_for_editor(&app.editor, &imported.to_json().unwrap())
            .unwrap();
    }
    let imported_pixels = capture(&mut h, &mut reference, "probe-imported-on");
    assert_ne!(imported_pixels, on);
    assert_ne!(imported_pixels, off);

    // Invalid authoring/imports preserve the accepted panel and actual texture.
    let accepted = retained(h.state().viewport3d_gpu().unwrap().gpu());
    let document = h
        .state()
        .irradiance
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    let dirty = h.state().irradiance.bindings.as_ref().unwrap().dirty();
    {
        let app = h.state_mut();
        let mut invalid = imported.clone();
        invalid.coefficients[63][8][2] = f32::NAN;
        assert!(
            app.irradiance
                .apply_grid_for_editor(&app.editor, invalid)
                .is_err()
        );
        assert!(
            app.irradiance
                .import_json_for_editor(&app.editor, b"{\"version\":999}")
                .is_err()
        );
    }
    settle(&mut h);
    assert_eq!(
        h.state().irradiance.bindings.as_ref().unwrap().document(),
        &document
    );
    assert_eq!(
        h.state().irradiance.bindings.as_ref().unwrap().dirty(),
        dirty
    );
    assert_eq!(
        retained(h.state().viewport3d_gpu().unwrap().gpu()),
        accepted
    );

    for hdr in [false, true] {
        h.state_mut().ui.yard_post_process.enabled = hdr;
        h.state_mut().ui.yard_post_process.bloom = true;
        let mut disabled = imported.clone();
        disabled.enabled = false;
        apply(&mut h, disabled.clone());
        let baseline = capture(&mut h, &mut reference, "probe-disabled");
        apply(&mut h, imported.clone());
        let lit = capture(&mut h, &mut reference, "probe-enabled");
        assert_ne!(lit, baseline);
        let stable = retained(h.state().viewport3d_gpu().unwrap().gpu());
        for _ in 0..3 {
            apply(&mut h, disabled.clone());
            assert_eq!(
                capture(&mut h, &mut reference, "probe-toggle-off"),
                baseline
            );
            apply(&mut h, imported.clone());
            assert_eq!(capture(&mut h, &mut reference, "probe-toggle-on"), lit);
            assert_eq!(
                retained(h.state().viewport3d_gpu().unwrap().gpu()),
                stable,
                "repeated grid toggles reuse admitted caches, targets and bounded uniforms"
            );
        }
        assert_eq!(
            stable.format,
            if hdr {
                TextureFormat::Rgba16Float
            } else {
                TextureFormat::Rgba8Unorm
            }
        );
        assert_eq!(stable.allocated > 0, hdr);
    }
    let resized_pixels = capture(&mut h, &mut reference, "probe-hdr-before-resize");
    let pipeline_generation = h
        .state()
        .viewport3d_gpu()
        .unwrap()
        .gpu()
        .pipeline_generation();
    for dimensions in [[1013.0, 817.0], [991.0, 799.0], [1001.0, 803.0]] {
        h.set_size(egui::vec2(dimensions[0], dimensions[1]));
        capture(&mut h, &mut reference, "probe-hdr-resized");
        let actual = h.state().viewport3d_gpu().unwrap().gpu();
        assert_eq!(actual.pipeline_generation(), pipeline_generation);
        assert_eq!(actual.post_process_size(), Some(h.state().ui.viewport_px));
    }
    assert_eq!(
        capture(&mut h, &mut reference, "probe-hdr-resize-restored"),
        resized_pixels
    );
    h.state_mut().ui.yard_post_process.enabled = false;
    assert_eq!(
        capture(&mut h, &mut reference, "probe-legacy-restored"),
        imported_pixels
    );
    let sidecar = {
        let app = h.state_mut();
        app.models.bindings.as_mut().unwrap().save().unwrap();
        app.irradiance.save().unwrap();
        assert!(!app.irradiance.bindings.as_ref().unwrap().dirty());
        app.irradiance.bindings.as_ref().unwrap().path.clone()
    };
    let saved = std::fs::read(&sidecar).unwrap();
    assert_eq!(host_frame(&mut h.state_mut().editor), expected_host);
    assert_eq!(h.state().editor.history().entries.len(), history);
    assert_eq!(std::fs::read(&scene).unwrap(), scene_bytes);
    drop(h);

    // Close the application and host, reopen the actual saved sidecars, and
    // resolve the same complete frame. This is not just a JSON round-trip.
    let mut editor = Editor::open_game(&scene, EditorGame::Yard3D).unwrap();
    editor.camera3d = camera;
    let mut h = Harness::builder()
        .with_size([1001.0, 803.0])
        .wgpu()
        .build_eframe(|cc| {
            let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
            app.ui.bottom_tab = BottomTab::History;
            app
        });
    settle(&mut h);
    {
        let app = h.state_mut();
        app.models.open_for_editor(&app.editor, false).unwrap();
        app.irradiance.open_for_editor(&app.editor, false).unwrap();
        app.irradiance.reload(&app.editor).unwrap();
    }
    h.get_by_label("Irradiance probes").click();
    h.run_steps(2);
    assert_eq!(
        capture(&mut h, &mut reference, "probe-reopened"),
        imported_pixels
    );
    assert_eq!(
        h.state().irradiance.grid_for_editor(&h.state().editor),
        Some(&imported)
    );
    assert_eq!(host_frame(&mut h.state_mut().editor), expected_host);
    assert_eq!(std::fs::read(&scene).unwrap(), scene_bytes);
    assert_eq!(std::fs::read(&sidecar).unwrap(), saved);

    // Rejected disk reloads do not silently replace a working visible grid.
    std::fs::write(&sidecar, b"{\"version\":999}").unwrap();
    {
        let app = h.state_mut();
        assert!(app.irradiance.reload(&app.editor).is_err());
    }
    assert_eq!(
        capture(&mut h, &mut reference, "probe-rejected-reload"),
        imported_pixels
    );
    std::fs::write(&sidecar, saved).unwrap();
    {
        let app = h.state_mut();
        app.irradiance.reload(&app.editor).unwrap();
    }
    assert_eq!(
        capture(&mut h, &mut reference, "probe-reload-restored"),
        imported_pixels
    );
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
    grid: Option<IrradianceGrid>,
    uniform_bytes: usize,
    #[cfg(feature = "animated-models")]
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
        grid: gpu.irradiance().cloned(),
        uniform_bytes: gpu.irradiance_uniform_bytes(),
        #[cfg(feature = "animated-models")]
        bounds: gpu.animated_bounds(),
    }
}

#[test]
fn irradiance_rejection_with_resize_and_late_invalid_batch_is_atomic() {
    let _serial = GPU_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let rhi = gpu();
    let temp = tempfile::tempdir().unwrap();
    let scene = fixture(temp.path());
    let mut app = EditorApp::new(Editor::open_game(&scene, EditorGame::Yard3D).unwrap(), None);
    sync_clock(&mut app.editor);
    configure(&mut app);
    let mut inputs = inputs(&app);
    let camera = app.editor.camera3d.camera();
    let mut viewport = Viewport3dGpu::new(&rhi, (161, 129));
    let grid = authored_grid([2.0, 0.5, 0.2]);
    let hdr = PostProcessSettings {
        enabled: true,
        bloom: true,
        ..PostProcessSettings::default()
    };
    render(
        &mut viewport,
        (161, 129),
        &inputs,
        &camera,
        hdr,
        Some(&grid),
    )
    .unwrap();
    let accepted = retained(&viewport);
    let mut invalid_grids = Vec::new();
    let mut invalid = grid.clone();
    invalid.version = 999;
    invalid_grids.push(invalid);
    let mut invalid = grid.clone();
    invalid.dimensions = [u32::MAX; 3];
    invalid_grids.push(invalid);
    let mut invalid = grid.clone();
    invalid.coefficients.pop();
    invalid_grids.push(invalid);
    for value in [f32::NAN, f32::INFINITY, -1.0, 0.0] {
        let mut invalid = grid.clone();
        invalid.spacing[2] = value;
        invalid_grids.push(invalid);
    }
    for value in [f32::NAN, f32::INFINITY, 1.0e20] {
        let mut invalid = grid.clone();
        invalid.origin[1] = value;
        invalid_grids.push(invalid);
        let mut invalid = grid.clone();
        invalid.coefficients[63][8][2] = value;
        invalid_grids.push(invalid);
    }
    let mut disabled_invalid = grid.clone();
    disabled_invalid.enabled = false;
    disabled_invalid.coefficients[0][0][0] = f32::NAN;
    invalid_grids.push(disabled_invalid);
    for invalid in invalid_grids {
        assert!(
            render(
                &mut viewport,
                (173, 137),
                &inputs,
                &camera,
                PostProcessSettings::default(),
                Some(&invalid)
            )
            .is_err()
        );
        assert_eq!(
            retained(&viewport),
            accepted,
            "invalid grid cannot resize targets, change HDR, mutate uniforms or evict accepted batches"
        );
    }
    for size in [
        (0, 1),
        (1, 0),
        (8193, 4),
        (8192, 8192),
        (u32::MAX, u32::MAX),
    ] {
        assert!(render(&mut viewport, size, &inputs, &camera, hdr, Some(&grid)).is_err());
        assert_eq!(retained(&viewport), accepted);
    }
    // A different valid grid must not be committed before later instance
    // admission fails, even when the request also changes size and HDR format.
    let replacement = authored_grid([0.1, 0.2, 4.0]);
    let translation = inputs.models[0].instance.translation;
    inputs.models[0].instance.translation[0] = f32::INFINITY;
    assert!(
        render(
            &mut viewport,
            (173, 137),
            &inputs,
            &camera,
            PostProcessSettings::default(),
            Some(&replacement)
        )
        .is_err()
    );
    assert_eq!(retained(&viewport), accepted);
    inputs.models[0].instance.translation = translation;
    #[cfg(feature = "animated-models")]
    {
        let original = inputs.animated[0].pose.clone();
        let foreign =
            orr_model::animation::AnimatedModel::new(inputs.animated[0].model.source().clone())
                .unwrap();
        inputs.animated[0].pose = foreign.rest_pose().unwrap();
        // The new static cache entry is valid, but must never publish after
        // rejection of the final skinned pose.
        inputs.models[0].model = std::sync::Arc::new((*inputs.models[0].model).clone());
        assert!(
            render(
                &mut viewport,
                (173, 137),
                &inputs,
                &camera,
                PostProcessSettings::default(),
                Some(&replacement)
            )
            .is_err()
        );
        assert_eq!(retained(&viewport), accepted);
        inputs.animated[0].pose = original;
    }
    render(
        &mut viewport,
        (161, 129),
        &inputs,
        &camera,
        hdr,
        Some(&grid),
    )
    .unwrap();
    assert_eq!(
        viewport.read_rgba8(),
        accepted.pixels,
        "accepted viewport remains usable after every rejected update"
    );
}
