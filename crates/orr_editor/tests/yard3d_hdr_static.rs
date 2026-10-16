//! Mandatory static-only EditorApp HDR route. Run with --features models to
//! exercise the non-animated preflight and render_post_processed cfg branches.
#![cfg(all(feature = "models", not(feature = "animated-models")))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui_kittest::Harness;
use orr_editor::{Editor, EditorApp, app::BottomTab, game::EditorGame, viewport3d::Viewport3dGpu};
use orr_package::{Project, Runtime};
use orr_rhi::{Rhi, TextureFormat, Wgpu, WgpuOptions};
use std::path::Path;
use std::time::{Duration, Instant};

fn settle(h: &mut Harness<'_, EditorApp>) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        h.state_mut().editor.sync();
        h.run_steps(2);
        if h.state().editor.snapshot().is_some() && h.state().editor.yard_rows_coherent() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "static-only coherent Yard snapshot"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(h.state().editor.down().is_none());
}

#[test]
fn static_only_editor_hdr_mode_uses_complete_frame_and_restores_legacy() {
    let gpu =
        Wgpu::headless(WgpuOptions::default()).expect("mandatory static-only Yard HDR adapter");
    eprintln!(
        "static-only Yard HDR adapter: {} software={}",
        gpu.adapter_name(),
        gpu.is_software()
    );
    drop(gpu);
    let root = tempfile::tempdir().unwrap();
    let asset = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/imported_scene_demo")
        .canonicalize()
        .unwrap();
    Project::open_for_install(root.path(), Runtime::content_only().engine_version)
        .unwrap()
        .install(&[asset])
        .unwrap();
    let scene = root.path().join("yard-static-hdr.scene.yaml");
    std::fs::write(
        &scene,
        include_str!("../../../scenes/yard3d_authoring.scene.yaml"),
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::Yard3D).unwrap();
    editor.camera3d = orr_render::OrbitCamera::new([0.0, 1.5, 0.0], 0.45, 0.65, 14.0);
    let mut comparison = None;
    let mut h = Harness::builder()
        .with_size([1001.0, 803.0])
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
    {
        let app = h.state_mut();
        assert!(app.editor.select_named("box_right"));
        app.editor.sync();
        app.models.open_for_editor(&app.editor, true).unwrap();
        app.models.package = "sample-imported-scene".into();
        app.models.asset = "foreground.glb".into();
        app.models.assign(&app.editor).unwrap();
        app.editor.select(None);
    }
    settle(&mut h);
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().entries.len();
    let mut reference = Viewport3dGpu::new(&comparison.unwrap(), (1, 1));
    let legacy = h.state().viewport3d_gpu().unwrap().gpu().read_rgba8();
    assert!(!h.state().ui.yard_post_process.enabled);
    assert!(
        h.state()
            .viewport3d_gpu()
            .unwrap()
            .gpu()
            .read_hdr_rgba()
            .is_none()
    );
    for enabled in [true, false, true, false] {
        h.state_mut().ui.yard_post_process.enabled = enabled;
        h.state_mut().ui.yard_post_process.bloom = false;
        settle(&mut h);
        let app = h.state();
        let placements = app.models.placements(&app.editor);
        assert_eq!(placements.len(), 1);
        let hidden: Vec<_> = placements
            .iter()
            .map(|placement| placement.entity)
            .collect();
        let list = app.editor.yard_frame().list(&hidden, None);
        assert_eq!((list.boxes.len(), list.planes.len()), (1, 1));
        reference
            .render_post_processed(
                app.ui.viewport_px,
                &list,
                &app.editor.camera3d.camera(),
                &placements,
                app.ui.yard_post_process,
            )
            .unwrap();
        let actual = app.viewport3d_gpu().unwrap().gpu();
        assert_eq!(actual.target().format(), TextureFormat::Rgba8Unorm);
        assert_eq!(actual.read_rgba8(), reference.read_rgba8());
        assert_eq!(actual.read_hdr_rgba(), reference.read_hdr_rgba());
        if enabled {
            let hdr = actual.read_hdr_rgba().unwrap();
            assert!(hdr.iter().flatten().all(|value| value.is_finite()));
            let maximum = hdr
                .iter()
                .flat_map(|pixel| &pixel[..3])
                .copied()
                .fold(0.0_f32, f32::max);
            assert!(maximum > 1.0, "static+procedural unclamped HDR={maximum}");
            assert_eq!(actual.scene_format(), TextureFormat::Rgba16Float);
            assert!(actual.post_process_allocated_bytes() > 0);
        } else {
            assert_eq!(actual.read_rgba8(), legacy);
            assert!(actual.read_hdr_rgba().is_none());
            assert_eq!(actual.post_process_allocated_bytes(), 0);
            assert_eq!(actual.scene_format(), TextureFormat::Rgba8Unorm);
        }
        assert_eq!(actual.model_cache_counts(), (1, 0));
        h.render()
            .expect("static-only final native texture composes in egui");
    }
    let old_generation = reference.target().generation();
    let old_pixels = reference.read_rgba8();
    let old_policy = reference.post_process_settings();
    let mut bad = old_policy;
    bad.enabled = true;
    bad.threshold = f32::NAN;
    let app = h.state();
    let placements = app.models.placements(&app.editor);
    let hidden: Vec<_> = placements
        .iter()
        .map(|placement| placement.entity)
        .collect();
    let list = app.editor.yard_frame().list(&hidden, None);
    assert!(
        reference
            .render_post_processed(
                (319, 253),
                &list,
                &app.editor.camera3d.camera(),
                &placements,
                bad
            )
            .is_err()
    );
    assert_eq!(reference.target().generation(), old_generation);
    assert_eq!(reference.read_rgba8(), old_pixels);
    assert_eq!(reference.post_process_settings(), old_policy);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history().entries.len(), history);
}
