//! Mandatory software-GPU evidence through the production EditorApp viewport.
#![cfg(feature = "navigation")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui_kittest::Harness;
use orr_editor::{game::EditorGame, Editor, EditorApp};
use orr_fp::FP;
use orr_navigation::NavigationStatus;
use orr_remote::navigation_yard3d::navigation_from_view;
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use orr_terrain::Edit;
use std::path::PathBuf;

fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..4 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
    assert!(h.state().editor.yard_rows_coherent());
}
fn pixels(h: &mut Harness<'_, EditorApp>, name: &str, adapter: &str) -> Vec<u8> {
    settle(h);
    let gpu = h
        .state()
        .viewport3d_gpu()
        .expect("production 3D viewport")
        .gpu();
    let bytes = gpu.read_rgba8();
    if let Some(root) = std::env::var_os("NAVIGATION_CAPTURE_DIR") {
        let root = PathBuf::from(root);
        std::fs::create_dir_all(&root).unwrap();
        let file = std::fs::File::create(root.join(format!("{name}.png"))).unwrap();
        let size = gpu.target().size();
        let mut encoder = png::Encoder::new(file, size.0, size.1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&bytes)
            .unwrap();
        let editor = &h.state().editor;
        let snapshot = editor.snapshot().unwrap();
        let frame = snapshot.predicted();
        let decoded = navigation_from_view(frame).unwrap();
        let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let metadata = serde_json::json!({
            "tick":snapshot.tick(), "frame_checksum":format!("0x{:016x}",frame.checksum()),
            "terrain_revision":hex(&decoded.terrain.revision()),
            "graph_revision":hex(&decoded.graph.revision()),
            "status":format!("{:?}", decoded.navigator.status()),
            "position_raw":decoded.navigator.position().map(FP::raw),
            "adapter":adapter,
            "model_counts":gpu.model_cache_counts(),
            "source_sha":std::env::var("NAVIGATION_SOURCE_SHA").ok(),
        });
        std::fs::write(
            root.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&metadata).unwrap(),
        )
        .unwrap();
    }
    bytes
}
fn changed_pixels(a: &[u8], b: &[u8]) -> usize {
    a.as_chunks::<4>().0.iter()
        .zip(b.as_chunks::<4>().0.iter())
        .filter(|(x, y)| {
            x[..3]
                .iter()
                .zip(y[..3].iter())
                .any(|(a, b)| a.abs_diff(*b) > 12)
        })
        .count()
}
fn cyan_pixels(bytes: &[u8]) -> usize {
    bytes.as_chunks::<4>().0.iter().filter(|p| strong_cyan(*p)).count()
}
fn strong_cyan(p: &[u8]) -> bool {
    p[0] < 90 && p[1] >= 160 && p[2] >= 175 && p[2] >= p[0].saturating_add(85)
}
fn magenta_pixels(bytes: &[u8]) -> usize {
    bytes.as_chunks::<4>().0.iter().filter(|p| magenta(*p)).count()
}
fn magenta(p: &[u8]) -> bool {
    p[0] >= 150 && p[2] >= 120 && p[1] < 110
}
fn red_pixels(bytes: &[u8]) -> usize {
    bytes.as_chunks::<4>().0.iter().filter(|p| strong_red(*p)).count()
}
fn strong_red(p: &[u8]) -> bool {
    p[0] >= 145 && p[1] < 100 && p[2] < 115
}
fn host_agent(h: &Harness<'_, EditorApp>) -> (NavigationStatus, [FP; 3]) {
    let snapshot = h.state().editor.snapshot().unwrap();
    let nav = navigation_from_view(snapshot.predicted()).unwrap();
    (nav.navigator.status(), nav.navigator.position())
}

#[test]
fn start_mid_arrival_stale_rebuild_use_one_shared_depth_scene() {
    let rhi = Wgpu::headless(WgpuOptions::default()).expect("mandatory navigation GPU adapter");
    let adapter = rhi.adapter_name().to_string();
    eprintln!("navigation GPU {} software={}", adapter, rhi.is_software());
    drop(rhi);
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    std::fs::create_dir(root_path.join("navigation")).unwrap();
    let scene = root_path.join("navigation.scene.yaml");
    let source = root_path.join("navigation/point_demo.orrt");
    std::fs::write(
        &scene,
        include_str!("../../../scenes/navigation_point.scene.yaml"),
    )
    .unwrap();
    std::fs::write(
        &source,
        include_bytes!("../../../scenes/navigation/point_demo.orrt"),
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::NavigationYard3D).unwrap();
    editor.sync();
    assert!(editor.admitted_navigation().admitted);
    editor.camera3d = orr_render::OrbitCamera::new([0.5, 0.1, 0.5], 0.45, 0.75, 3.8);
    let mut h = Harness::builder()
        .with_size([1280.0, 900.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(|cc| EditorApp::new(editor, cc.wgpu_render_state.clone()));
    let start = pixels(&mut h, "01-start", &adapter);
    let (start_status, start_position) = host_agent(&h);
    assert_eq!(start_status, NavigationStatus::Moving);
    assert!(
        cyan_pixels(&start) > 10,
        "route overlay must be visibly cyan, excluding blue sky"
    );
    assert!(
        magenta_pixels(&start) > 10,
        "agent marker must be visibly magenta"
    );
    let overlay = h
        .state()
        .editor
        .admitted_navigation()
        .overlay_model(false)
        .unwrap();
    assert_eq!(
        overlay.source().asset_id,
        "navigation/editor_overlay.orrmodel"
    );
    assert!(overlay
        .source()
        .primitives
        .iter()
        .all(|p| p.id.starts_with("navigation/editor_overlay.orrmodel#")));
    assert_eq!(
        h.state()
            .viewport3d_gpu()
            .unwrap()
            .gpu()
            .model_cache_counts()
            .0,
        2,
        "one terrain model plus one terrain-free overlay model"
    );
    h.state_mut().editor.step(2);
    let mid = pixels(&mut h, "02-mid", &adapter);
    let (mid_status, mid_position) = host_agent(&h);
    assert_eq!(mid_status, NavigationStatus::Moving);
    assert_ne!(mid_position, start_position, "host Frame owns movement");
    assert!(
        changed_pixels(&start, &mid) > 20,
        "agent moves visibly in the viewport"
    );
    h.state_mut().editor.step(60);
    let arrival = pixels(&mut h, "03-arrival", &adapter);
    let (arrival_status, arrival_position) = host_agent(&h);
    assert_eq!(arrival_status, NavigationStatus::Arrived);
    let decoded = navigation_from_view(h.state().editor.snapshot().unwrap().predicted()).unwrap();
    assert_eq!(
        arrival_position,
        decoded
            .graph
            .project(&decoded.terrain, decoded.spec.goal)
            .unwrap()
            .position
    );
    assert!(changed_pixels(&arrival, &mid) > 20);
    h.state_mut().editor.stop();
    settle(&mut h);

    {
        let app = h.state_mut();
        app.terrain.document.set_scene(Some(&scene)).unwrap();
        app.terrain
            .document
            .open_local("navigation/point_demo.orrt")
            .unwrap();
        app.terrain
            .document
            .apply(&[Edit::SetHeight {
                x: 1,
                z: 1,
                height: FP::from_raw(8192),
            }])
            .unwrap();
    }
    let stale = pixels(&mut h, "04-stale", &adapter);
    assert!(h
        .state()
        .navigation
        .route_stale(&h.state().editor, &h.state().terrain));
    assert_ne!(stale, start, "unsaved edit is visible with stale route");
    assert!(
        red_pixels(&stale) > 20,
        "stale route must be visibly red above the working terrain"
    );
    h.state_mut().terrain.document.save().unwrap();
    {
        let app = h.state_mut();
        app.navigation
            .build_route(&mut app.editor, &app.terrain)
            .unwrap();
        app.editor.sync();
    }
    let rebuilt = pixels(&mut h, "05-rebuilt", &adapter);
    assert_ne!(rebuilt, stale);
    assert!(
        red_pixels(&rebuilt) < red_pixels(&stale) / 2,
        "explicit rebuild must replace the stale red route"
    );
    assert!(h.state().editor.admitted_navigation().admitted);
    assert_eq!(
        h.state()
            .viewport3d_gpu()
            .unwrap()
            .gpu()
            .model_cache_counts()
            .0,
        2
    );

    std::fs::remove_file(&source).unwrap();
    h.state_mut().editor.step(8);
    settle(&mut h);
    let after_removed = h.state().editor.checksum();
    h.state_mut().editor.seek(0);
    h.state_mut().editor.step(8);
    settle(&mut h);
    assert_eq!(
        h.state().editor.checksum(),
        after_removed,
        "seek and replay use the admitted Frame, not the removed asset"
    );
}

#[test]
fn overlay_color_classifier_rejects_sky_and_terrain() {
    assert!(!strong_cyan(&[76, 107, 158, 255]));
    assert!(!strong_cyan(&[80, 205, 130, 255]));
    assert!(strong_cyan(&[0, 235, 255, 255]));
    assert!(!strong_cyan(&[250, 25, 235, 255]));
    assert!(!magenta(&[76, 107, 158, 255]));
    assert!(!magenta(&[80, 205, 130, 255]));
    assert!(magenta(&[250, 25, 235, 255]));
    assert!(!magenta(&[0, 235, 255, 255]));
    assert!(strong_red(&[255, 35, 45, 255]));
    assert!(!strong_red(&[76, 107, 158, 255]));
    assert!(!strong_red(&[80, 205, 130, 255]));
    assert!(!strong_red(&[250, 25, 235, 255]));
}
