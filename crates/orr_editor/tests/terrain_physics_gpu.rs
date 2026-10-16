//! Production main viewport on a real headless GPU. Absence fails, never skips.
#![cfg(feature = "terrain-physics")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui_kittest::Harness;
use orr_editor::{game::EditorGame, Editor, EditorApp};
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use std::path::PathBuf;
fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..4 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
    assert!(h.state().editor.yard_rows_coherent());
}
fn pixels(h: &mut Harness<'_, EditorApp>, name: &str) -> Vec<u8> {
    settle(h);
    let gpu = h
        .state()
        .viewport3d_gpu()
        .expect("actual EditorApp terrain viewport")
        .gpu();
    let bytes = gpu.read_rgba8();
    if let Some(root) = std::env::var_os("TERRAIN_PHYSICS_CAPTURE_DIR") {
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
    }
    bytes
}
#[test]
fn admitted_frame_terrain_and_spheres_render_in_main_viewport_during_play() {
    let rhi =
        Wgpu::headless(WgpuOptions::default()).expect("mandatory terrain physics GPU adapter");
    eprintln!(
        "terrain physics GPU {} software={}",
        rhi.adapter_name(),
        rhi.is_software()
    );
    drop(rhi);
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("terrain")).unwrap();
    let scene = root.path().join("terrain.scene.yaml");
    std::fs::write(
        &scene,
        include_str!("../../../scenes/terrain_sphere.scene.yaml"),
    )
    .unwrap();
    let asset = root.path().join("terrain/sphere_demo.orrt");
    std::fs::write(
        &asset,
        include_bytes!("../../../scenes/terrain/sphere_demo.orrt"),
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::TerrainYard3D).unwrap();
    editor.sync();
    editor.camera3d = orr_render::OrbitCamera::new([0.0, 1.0, 0.0], 0.45, 0.7, 14.0);
    let mut h = Harness::builder()
        .with_size([1280.0, 900.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(|cc| EditorApp::new(editor, cc.wgpu_render_state.clone()));
    let initial = pixels(&mut h, "01-admitted");
    assert_eq!(
        h.state()
            .viewport3d_gpu()
            .unwrap()
            .gpu()
            .model_cache_counts()
            .0,
        1
    );
    assert_eq!(h.state().editor.yard_frame().items.len(), 2);
    std::fs::remove_file(asset).unwrap();
    h.state_mut().editor.step(120);
    let settled = pixels(&mut h, "02-play-solid-and-hole");
    assert_ne!(
        settled, initial,
        "visible spheres move under terrain physics"
    );
    assert!(h.state().editor.admitted_terrain().error.is_none());
    assert_eq!(
        h.state()
            .viewport3d_gpu()
            .unwrap()
            .gpu()
            .model_cache_counts()
            .0,
        1
    );
    h.state_mut().editor.stop();
    let stopped = pixels(&mut h, "03-stop-restores-scene");
    assert_eq!(
        stopped, initial,
        "scene and admitted visual asset restore exactly"
    );
}
