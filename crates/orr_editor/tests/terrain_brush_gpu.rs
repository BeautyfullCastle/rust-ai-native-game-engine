//! Mandatory production viewport readback for the sculpt authoring consumer.
//! GPU absence fails this target; it never turns a missing adapter into a skip.
#![cfg(feature = "terrain")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui::{Event, Modifiers, PointerButton};
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{game::EditorGame, Editor, EditorApp};
use orr_fp::FP;
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use std::path::PathBuf;

fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..4 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
    assert!(h.state().editor.yard_rows_coherent());
}

fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(3);
    h.get_by_label(label).click();
    settle(h);
}

fn pixels(h: &mut Harness<'_, EditorApp>, name: &str) -> Vec<u8> {
    settle(h);
    let gpu = h
        .state()
        .viewport3d_gpu()
        .expect("actual EditorApp viewport")
        .gpu();
    let bytes = gpu.read_rgba8();
    if let Some(root) = std::env::var_os("TERRAIN_SCULPT_CAPTURE_DIR") {
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

fn select_vertex(h: &mut Harness<'_, EditorApp>, vertex: [u32; 2]) {
    settle(h);
    let app = h.state();
    let terrain = app.terrain.document.terrain().unwrap();
    let world = terrain
        .vertex_position(vertex[1] * terrain.width() + vertex[0])
        .unwrap()
        .map(FP::to_f32);
    let size = app.ui.viewport_px;
    let rect = app.ui.viewport_rect.unwrap();
    let pixel = app
        .editor
        .camera3d
        .camera()
        .world_to_screen(world, size)
        .unwrap();
    let at = rect.min
        + egui::vec2(
            pixel[0] * rect.width() / size.0 as f32,
            pixel[1] * rect.height() / size.1 as f32,
        );
    assert!(rect.contains(at));
    h.event(Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    settle(h);
    assert_eq!(h.state().terrain.selected_vertex, vertex);
}

#[test]
fn sculpt_panel_viewport_changes_and_exact_undo_redo_saved_restore() {
    let rhi = Wgpu::headless(WgpuOptions::default()).expect("mandatory terrain sculpt GPU adapter");
    eprintln!(
        "terrain sculpt GPU {} software={}",
        rhi.adapter_name(),
        rhi.is_software()
    );
    drop(rhi);
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let scene = root.join("yard.scene.yaml");
    std::fs::write(
        &scene,
        include_str!("../../../scenes/yard3d_authoring.scene.yaml"),
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::Yard3D).unwrap();
    editor.sync();
    editor.camera3d = orr_render::OrbitCamera::new([0.0, 1.5, 0.0], 0.45, 0.75, 16.0);
    let checksum = editor.checksum();
    let history = editor.history().clone();
    let mut h = Harness::builder()
        .with_size([1280.0, 1000.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(|cc| EditorApp::new(editor, cc.wgpu_render_state.clone()));
    settle(&mut h);
    eprintln!(
        "terrain sculpt captured viewport adapter={}",
        h.state()
            .viewport3d_gpu()
            .expect("actual EditorApp viewport")
            .gpu()
            .adapter_name()
    );
    h.state_mut().terrain.initial_height = "1".into();
    click(&mut h, "Terrain authoring");
    click(&mut h, "Create terrain");
    click(&mut h, "Vertex");
    click(&mut h, "Terrain sculpt stamp");
    {
        let app = h.state_mut();
        app.terrain.selected_vertex = [4, 4];
        app.terrain.sculpt_radius = 2;
        app.terrain.sculpt_delta = "2".into();
        app.terrain.query_xz = ["0".into(), "0".into()];
        app.terrain.query_for_editor(&app.editor).unwrap();
    }
    let flat_bytes = h.state().terrain.document.bytes().unwrap().to_vec();
    let flat = pixels(&mut h, "01-flat");
    click(&mut h, "Apply sculpt stamp");
    assert!(
        h.state().terrain.error().is_none(),
        "{:?}",
        h.state().terrain.error()
    );
    let raised = pixels(&mut h, "02-raised-stamp");
    assert_ne!(
        raised, flat,
        "an authored sculpt stamp must reach the actual viewport"
    );
    let raised_bytes = h.state().terrain.document.bytes().unwrap().to_vec();
    assert_ne!(raised_bytes, flat_bytes);
    let terrain = h.state().terrain.document.terrain().unwrap();
    assert_eq!(
        terrain
            .heights()
            .iter()
            .filter(|&&height| height == FP::from_int(3))
            .count(),
        13
    );
    assert_eq!(
        h.state().terrain.document.surface().unwrap().height,
        FP::from_int(3)
    );
    assert_eq!(
        h.state().terrain.document.marker().unwrap().anchor(),
        [FP::ZERO, FP::from_int(3), FP::ZERO]
    );
    click(&mut h, "Terrain Undo");
    assert_eq!(h.state().terrain.document.bytes().unwrap(), flat_bytes);
    assert!(!h.state().terrain.document.can_undo());
    assert_eq!(pixels(&mut h, "03-undo"), flat);
    click(&mut h, "Terrain Redo");
    assert_eq!(pixels(&mut h, "04-redo"), raised);
    click(&mut h, "Save terrain");
    let revision = h.state().terrain.document.revision();
    assert_eq!(
        std::fs::read(root.join("terrain.orrt")).unwrap(),
        raised_bytes
    );
    click(&mut h, "Close terrain");
    assert_ne!(pixels(&mut h, "05-detached"), raised);
    click(&mut h, "Open terrain");
    {
        let app = h.state_mut();
        app.terrain.query_for_editor(&app.editor).unwrap();
    }
    assert_eq!(h.state().terrain.document.bytes().unwrap(), raised_bytes);
    assert_eq!(h.state().terrain.document.revision(), revision);
    assert_eq!(pixels(&mut h, "06-reopened"), raised);
    assert!(!h.state().terrain.document.dirty());
    assert!(!h.state().terrain.document.can_undo());
    // Sample a known plateau through the real controls, then flatten a second
    // region. The sample itself must not create a terrain transaction.
    click(&mut h, "Flatten");
    select_vertex(&mut h, [4, 4]);
    let sampled_before = h.state().terrain.document.bytes().unwrap().to_vec();
    let sampled_revision = h.state().terrain.document.revision();
    click(&mut h, "Sample selected vertex height");
    assert_eq!(h.state().terrain.sculpt_height, "3");
    assert_eq!(h.state().terrain.document.bytes().unwrap(), sampled_before);
    assert_eq!(h.state().terrain.document.revision(), sampled_revision);
    assert!(!h.state().terrain.document.dirty());
    assert!(!h.state().terrain.document.can_undo());
    assert_eq!(pixels(&mut h, "07-sampled-setting"), raised);
    select_vertex(&mut h, [7, 4]);
    h.state_mut().terrain.sculpt_radius = 1;
    click(&mut h, "Apply sculpt stamp");
    let sampled_flat = pixels(&mut h, "08-sampled-flatten");
    assert!(
        sampled_flat != raised,
        "sampled flatten must change actual viewport pixels"
    );
    assert_eq!(
        h.state().terrain.document.terrain().unwrap().heights()[4 * 9 + 7],
        FP::from_int(3)
    );
    let sampled_bytes = h.state().terrain.document.bytes().unwrap().to_vec();
    click(&mut h, "Terrain Undo");
    assert_eq!(h.state().terrain.document.bytes().unwrap(), sampled_before);
    assert!(!h.state().terrain.document.can_undo());
    assert_eq!(pixels(&mut h, "09-sampled-undo"), raised);
    click(&mut h, "Terrain Redo");
    assert_eq!(pixels(&mut h, "10-sampled-redo"), sampled_flat);
    click(&mut h, "Save terrain");
    click(&mut h, "Close terrain");
    click(&mut h, "Open terrain");
    {
        let app = h.state_mut();
        app.terrain.query_for_editor(&app.editor).unwrap();
    }
    assert_eq!(h.state().terrain.document.bytes().unwrap(), sampled_bytes);
    assert_eq!(
        std::fs::read(root.join("terrain.orrt")).unwrap(),
        sampled_bytes
    );
    assert_eq!(pixels(&mut h, "11-sampled-reopened"), sampled_flat);
    assert!(!h.state().terrain.document.dirty());
    assert!(!h.state().terrain.document.can_undo());
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
}
