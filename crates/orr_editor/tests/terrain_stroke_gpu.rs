//! Mandatory production viewport readback for the sculpt authoring consumer.
//! GPU absence fails this target; it never turns a missing adapter into a skip.
#![cfg(feature = "terrain")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui::{Event, Modifiers, PointerButton, Pos2};
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
    if let Some(root) = std::env::var_os("TERRAIN_STROKE_CAPTURE_DIR") {
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

fn vertex(h: &Harness<'_, EditorApp>, [x, z]: [u32; 2]) -> Pos2 {
    let terrain = h.state().terrain.document.sculpt_pick_terrain().unwrap();
    let point = terrain
        .vertex_position(z * terrain.width() + x)
        .unwrap()
        .map(FP::to_f32);
    let size = h.state().ui.viewport_px;
    let rect = h.state().ui.viewport_rect.unwrap();
    let pixel = h
        .state()
        .editor
        .camera3d
        .camera()
        .world_to_screen(point, size)
        .unwrap();
    let pos = rect.min
        + egui::vec2(
            pixel[0] * rect.width() / size.0 as f32,
            pixel[1] * rect.height() / size.1 as f32,
        );
    assert!(rect.contains(pos));
    pos
}
fn button(h: &mut Harness<'_, EditorApp>, pos: Pos2, pressed: bool) {
    h.event(Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    });
    h.step();
}
#[test]
fn sculpt_drag_live_preview_commits_one_undoable_gpu_visible_stroke() {
    let rhi = Wgpu::headless(WgpuOptions::default()).expect("mandatory terrain stroke GPU adapter");
    eprintln!(
        "terrain stroke preflight GPU {} software={}",
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
        "terrain stroke captured viewport adapter={}",
        h.state()
            .viewport3d_gpu()
            .expect("actual EditorApp viewport")
            .gpu()
            .adapter_name()
    );
    h.state_mut().terrain.initial_height = "1".into();
    h.state_mut().terrain.sculpt_radius = 1;
    h.state_mut().terrain.sculpt_delta = "0.25".into();
    click(&mut h, "Terrain authoring");
    click(&mut h, "Create terrain");
    click(&mut h, "Save terrain");
    click(&mut h, "Sculpt");
    {
        let app = h.state_mut();
        app.terrain.query_xz = ["0".into(), "0".into()];
        app.terrain.query_for_editor(&app.editor).unwrap();
    }
    let original = h.state().terrain.document.bytes().unwrap().to_vec();
    let before = pixels(&mut h, "01-before");
    let points = [[2, 2], [4, 4], [6, 6]].map(|p| vertex(&h, p));
    eprintln!(
        "stroke initial rect={:?} size={:?} camera={:?} points={points:?}",
        h.state().ui.viewport_rect,
        h.state().ui.viewport_px,
        h.state().editor.camera3d.camera()
    );
    h.event(Event::PointerMoved(points[0]));
    h.step();
    // The cyan preview is an egui overlay, not part of the raw viewport texture.
    // Verify actual paint output separately and keep all six terrain captures intact.
    fn footprint_count(shape: &egui::epaint::Shape) -> usize {
        match shape {
            egui::epaint::Shape::Circle(c)
                if c.fill == egui::Color32::from_rgb(110, 220, 240) && c.radius == 2.5 =>
            {
                1
            }
            egui::epaint::Shape::Vec(shapes) => shapes.iter().map(footprint_count).sum(),
            _ => 0,
        }
    }
    assert_eq!(
        h.output()
            .shapes
            .iter()
            .map(|s| footprint_count(&s.shape))
            .sum::<usize>(),
        5
    );
    assert_eq!(h.state().terrain.document.bytes().unwrap(), original);
    assert_eq!(h.state().terrain.selected_vertex, [0, 0]);
    assert_eq!(pixels(&mut h, "01a-idle-hover"), before);
    assert!(!h.state().terrain.document.dirty());
    assert!(!h.state().terrain.document.can_undo());
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    button(&mut h, points[0], true);
    assert!(
        h.state().terrain.document.stroke_active(),
        "{:?}",
        h.state().terrain.error()
    );
    for (sample, at) in points[1..].iter().enumerate() {
        h.event(Event::PointerMoved(*at));
        h.step();
        eprintln!(
            "stroke sample={sample} rect={:?} size={:?} camera={:?} selected={:?}",
            h.state().ui.viewport_rect,
            h.state().ui.viewport_px,
            h.state().editor.camera3d.camera(),
            h.state().terrain.selected_vertex
        );
        assert!(
            h.state().terrain.document.stroke_active(),
            "stroke cancelled after motion sample {sample}: {:?}",
            h.state().terrain.error(),
        );
    }
    let edited = h.state().terrain.document.bytes().unwrap().to_vec();
    assert_ne!(edited, original);
    assert!(!h.state().terrain.document.can_undo());
    assert_eq!(
        h.state()
            .terrain
            .document
            .terrain()
            .unwrap()
            .heights()
            .iter()
            .filter(|&&v| v == FP::from_raw(81920))
            .count(),
        17
    );
    assert_eq!(
        h.state().terrain.document.surface().unwrap().height,
        FP::from_raw(81920)
    );
    let preview = pixels(&mut h, "02-live-preview");
    assert!(
        h.state().terrain.document.stroke_active(),
        "stroke cancelled while settling GPU preview: {:?}",
        h.state().terrain.error(),
    );
    assert_eq!(h.state().terrain.document.bytes().unwrap(), edited);
    assert!(
        preview != before,
        "live terrain preview did not change viewport pixels"
    );
    button(&mut h, points[2], false);
    assert!(!h.state().terrain.document.stroke_active());
    let committed = pixels(&mut h, "03-committed");
    assert_eq!(committed, preview);
    assert_eq!(h.state().terrain.document.bytes().unwrap(), edited);
    click(&mut h, "Terrain Undo");
    assert_eq!(h.state().terrain.document.bytes().unwrap(), original);
    assert!(!h.state().terrain.document.can_undo());
    assert_eq!(pixels(&mut h, "04-undo"), before);
    click(&mut h, "Terrain Redo");
    assert_eq!(pixels(&mut h, "05-redo"), committed);
    click(&mut h, "Save terrain");
    assert_eq!(std::fs::read(root.join("terrain.orrt")).unwrap(), edited);
    click(&mut h, "Close terrain");
    click(&mut h, "Open terrain");
    {
        let app = h.state_mut();
        app.terrain.query_for_editor(&app.editor).unwrap();
    }
    assert_eq!(h.state().terrain.document.bytes().unwrap(), edited);
    assert_eq!(pixels(&mut h, "06-reopened"), committed);
    assert!(!h.state().terrain.document.dirty());
    assert!(!h.state().terrain.document.can_undo());
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
}
