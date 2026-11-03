#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use super::*;
use crate::{EditorApp, game::EditorGame};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use orr_fp::FP;

fn png_bytes(color: [u8; 4]) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&color.repeat(4))
            .unwrap();
    }
    bytes
}
fn fixture() -> (tempfile::TempDir, EditorApp) {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().canonicalize().unwrap();
    fs::create_dir(dir.join("navigation")).unwrap();
    let scene = dir.join("navigation.scene.yaml");
    fs::write(
        &scene,
        include_bytes!("../../../../scenes/navigation_point.scene.yaml"),
    )
    .unwrap();
    fs::write(
        dir.join("navigation/point_demo.orrt"),
        include_bytes!("../../../../scenes/navigation/point_demo.orrt"),
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::NavigationYard3D).unwrap();
    editor.sync();
    assert!(editor.yard_rows_coherent());
    editor.camera3d = orr_render::OrbitCamera::new([0.5, 0.1, 0.5], 0.45, 0.75, 3.8);
    let mut app = EditorApp::new(editor, None);
    app.terrain.local_path = "navigation/point_demo.orrt".into();
    app.terrain.open_for_editor(&app.editor).unwrap();
    app.terrain_surface.state.sync(&app.editor, &app.terrain);
    let png = dir.join("appearance.png");
    fs::write(&png, png_bytes([220, 110, 20, 255])).unwrap();
    app.terrain_surface.state.png_path = png.display().to_string();
    (root, app)
}
fn preview_apply(app: &mut EditorApp) {
    app.terrain_surface
        .state
        .preview(&app.editor, &app.terrain)
        .unwrap();
    app.terrain_surface
        .state
        .apply(&app.editor, &app.terrain)
        .unwrap();
}
fn panel(app: EditorApp) -> Harness<'static, EditorApp> {
    Harness::builder().with_size([1200., 700.]).build_ui_state(
        |ui, app: &mut EditorApp| app.terrain_surface.show(ui, &app.editor, &app.terrain),
        app,
    )
}
#[test]
fn actual_widgets_keep_core_host_and_history_unchanged_and_noop_keeps_redo() {
    let (_root, app) = fixture();
    let checksum = app.editor.checksum();
    let history = app.editor.history().clone();
    let terrain = app.terrain.document.bytes().unwrap().to_vec();
    let mut h = panel(app);
    h.get_by_label("Preview appearance PNG").click();
    h.run();
    assert!(!h.state().terrain_surface.state.dirty());
    h.get_by_label("Apply appearance").click();
    h.run();
    assert!(h.state().terrain_surface.state.dirty());
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(*h.state().editor.history(), history);
    assert_eq!(h.state().terrain.document.bytes().unwrap(), terrain);
    h.get_by_label("Undo appearance").click();
    h.run();
    assert!(!h.state().terrain_surface.state.dirty());
    h.get_by_label("Redo appearance").click();
    h.run();
    let app = h.state_mut();
    let first = app.terrain_surface.state.current.as_ref().unwrap().digest();
    fs::write(
        &app.terrain_surface.state.png_path,
        png_bytes([10, 40, 200, 255]),
    )
    .unwrap();
    preview_apply(app);
    app.terrain_surface
        .state
        .history(&app.editor, &app.terrain, false)
        .unwrap();
    fs::write(
        &app.terrain_surface.state.png_path,
        png_bytes([220, 110, 20, 255]),
    )
    .unwrap();
    preview_apply(app); // Same content, not a branch: redo remains available.
    assert!(app.terrain_surface.state.can_redo());
    assert_eq!(
        app.terrain_surface.state.current.as_ref().unwrap().digest(),
        first
    );
    app.terrain_surface
        .state
        .history(&app.editor, &app.terrain, true)
        .unwrap();
    assert_ne!(
        app.terrain_surface.state.current.as_ref().unwrap().digest(),
        first
    );
}
#[test]
fn pins_reject_changed_png_terrain_disk_and_scene_epoch_without_mutation() {
    let (root, mut app) = fixture();
    app.terrain_surface
        .state
        .preview(&app.editor, &app.terrain)
        .unwrap();
    fs::write(
        &app.terrain_surface.state.png_path,
        png_bytes([1, 2, 3, 255]),
    )
    .unwrap();
    assert!(
        app.terrain_surface
            .state
            .apply(&app.editor, &app.terrain)
            .is_err()
    );
    assert!(app.terrain_surface.state.pending());
    assert!(!app.terrain_surface.state.dirty());
    app.terrain_surface
        .state
        .preview(&app.editor, &app.terrain)
        .unwrap();
    app.terrain
        .document
        .apply(&[orr_terrain::Edit::SetHeight {
            x: 0,
            z: 0,
            height: FP::from_raw(100),
        }])
        .unwrap();
    assert!(
        app.terrain_surface
            .state
            .apply(&app.editor, &app.terrain)
            .is_err()
    );
    app.terrain_surface
        .state
        .preview(&app.editor, &app.terrain)
        .unwrap();
    fs::write(root.path().join("navigation/point_demo.orrt"), b"changed").unwrap();
    assert!(
        app.terrain_surface
            .state
            .apply(&app.editor, &app.terrain)
            .is_err()
    );
    app.terrain_surface
        .state
        .context(&app.editor, &app.terrain)
        .unwrap();
    app.terrain_surface.state.binding.as_mut().unwrap().epoch += 1;
    assert!(
        app.terrain_surface
            .state
            .apply(&app.editor, &app.terrain)
            .is_err()
    );
    assert!(app.terrain_surface.state.pending());
}
#[test]
fn saved_surface_retains_height_edits_rejects_tamper_and_missing_png_reopens() {
    let (root, mut app) = fixture();
    preview_apply(&mut app);
    let surface = app.terrain_surface.state.current.as_ref().unwrap().clone();
    let before = app
        .terrain_surface
        .state
        .viewport(&app.editor, &app.terrain)
        .unwrap()
        .unwrap();
    app.terrain
        .document
        .apply(&[orr_terrain::Edit::SetHeight {
            x: 0,
            z: 0,
            height: FP::from_raw(1000),
        }])
        .unwrap();
    let after = app
        .terrain_surface
        .state
        .viewport(&app.editor, &app.terrain)
        .unwrap()
        .unwrap();
    assert_eq!(before.source().images, after.source().images);
    assert_ne!(before.source().primitives, after.source().primitives);
    assert_eq!(
        app.terrain_surface.state.current.as_ref().unwrap().digest(),
        surface.digest()
    );
    app.terrain_surface
        .state
        .save(&app.editor, &app.terrain)
        .unwrap();
    assert!(!app.terrain_surface.state.dirty());
    fs::remove_file(&app.terrain_surface.state.png_path).unwrap();
    app.terrain_surface
        .state
        .reload(&app.editor, &app.terrain)
        .unwrap();
    assert_eq!(
        app.terrain_surface.state.current.as_ref().unwrap().digest(),
        surface.digest()
    );
    let sidecar = root.path().join("navigation/point_demo.orrt.surface.json");
    fs::write(&sidecar, b"malformed").unwrap();
    assert!(
        app.terrain_surface
            .state
            .save(&app.editor, &app.terrain)
            .is_err()
    );
    assert!(
        app.terrain_surface
            .state
            .reload(&app.editor, &app.terrain)
            .is_err()
    );
    assert!(
        app.terrain_surface
            .state
            .viewport(&app.editor, &app.terrain)
            .unwrap()
            .is_none()
    );
}
#[test]
fn bounded_history_and_pending_terrain_replacement_veto() {
    let (_root, mut app) = fixture();
    for value in 0..20 {
        fs::write(
            &app.terrain_surface.state.png_path,
            png_bytes([value, 30, 80, 255]),
        )
        .unwrap();
        preview_apply(&mut app);
    }
    assert!(app.terrain_surface.state.undo.len() <= HISTORY_ENTRIES);
    app.terrain
        .document
        .set_appearance_busy(app.terrain_surface.state.busy());
    assert!(app.terrain.document.close().is_err());
    assert!(app.terrain.discard_for_editor(&app.editor).is_err());
    app.terrain_surface.state.discard();
    app.terrain.document.set_appearance_busy(false);
    assert!(app.terrain.document.close().is_ok());
}
#[test]
fn save_checker_undo_removes_only_expected_sidecar() {
    let (root, mut app) = fixture();
    preview_apply(&mut app);
    app.terrain_surface
        .state
        .save(&app.editor, &app.terrain)
        .unwrap();
    app.terrain_surface
        .state
        .history(&app.editor, &app.terrain, false)
        .unwrap();
    let sidecar = root.path().join("navigation/point_demo.orrt.surface.json");
    let saved = fs::read(&sidecar).unwrap();
    fs::write(&sidecar, b"foreign").unwrap();
    assert!(
        app.terrain_surface
            .state
            .save(&app.editor, &app.terrain)
            .is_err()
    );
    assert_eq!(fs::read(&sidecar).unwrap(), b"foreign");
    fs::write(&sidecar, saved).unwrap();
    app.terrain_surface
        .state
        .save(&app.editor, &app.terrain)
        .unwrap();
    assert!(!sidecar.exists());
    app.terrain_surface
        .state
        .reload(&app.editor, &app.terrain)
        .unwrap();
    assert!(!app.terrain_surface.state.has_surface());
    assert!(!app.terrain_surface.state.dirty());
}
#[test]
fn failed_preview_revokes_pending_and_gesture_and_scene_bytes_are_pinned() {
    let (_root, mut app) = fixture();
    app.terrain_surface
        .state
        .preview(&app.editor, &app.terrain)
        .unwrap();
    app.terrain_surface.state.png_path.push_str("-missing");
    assert!(
        app.terrain_surface
            .state
            .preview(&app.editor, &app.terrain)
            .is_err()
    );
    assert!(!app.terrain_surface.state.pending());
    app.terrain_surface
        .state
        .png_path
        .truncate(app.terrain_surface.state.png_path.len() - 8);
    app.terrain_surface.state.operation_blocked = true;
    assert!(
        app.terrain_surface
            .state
            .preview(&app.editor, &app.terrain)
            .is_err()
    );
    app.terrain_surface.state.operation_blocked = false;
    app.terrain_surface
        .state
        .preview(&app.editor, &app.terrain)
        .unwrap();
    let scene = app.editor.path().unwrap();
    let mut bytes = fs::read(&scene).unwrap();
    bytes.extend_from_slice(b"\n# changed after preview\n");
    fs::write(scene, bytes).unwrap();
    assert!(
        app.terrain_surface
            .state
            .apply(&app.editor, &app.terrain)
            .is_err()
    );
    assert!(!app.terrain_surface.state.dirty());
}
fn click_visible(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(3);
    let node = h.get_by_label(label);
    assert!(
        !node.accesskit_node().is_disabled(),
        "{label} must be enabled"
    );
    let rect = node.rect();
    let mut visible = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1500., 1000.));
    let mut ancestor = node.parent();
    while let Some(parent) = ancestor {
        if parent.accesskit_node().clips_children() {
            visible = visible.intersect(parent.rect());
        }
        ancestor = parent.parent();
    }
    assert!(
        visible.contains(rect.center()),
        "{label} pointer center is clipped or outside the viewport: {rect:?}, visible {visible:?}"
    );
    h.get_by_label(label).click(); // egui_kittest emits real pointer press/release, not accesskit Click.
    h.run_steps(3);
    h.state_mut().editor.sync();
    h.run_steps(2);
}
fn settle_gpu(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..4 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
    assert!(h.state().editor.yard_rows_coherent());
}
fn capture(h: &mut Harness<'_, EditorApp>, name: &str) -> Vec<u8> {
    settle_gpu(h);
    let gpu = h.state().viewport3d_gpu().unwrap().gpu();
    let bytes = gpu.read_rgba8();
    if let Some(root) = std::env::var_os("TERRAIN_SURFACE_CAPTURE_DIR") {
        let root = PathBuf::from(root);
        fs::create_dir_all(&root).unwrap();
        let size = gpu.target().size();
        let mut encoder = png::Encoder::new(
            fs::File::create(root.join(format!("{name}.png"))).unwrap(),
            size.0,
            size.1,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&bytes)
            .unwrap();
        let editor = &h.state().editor;
        let nav = orr_remote::navigation_yard3d::navigation_from_view(
            editor.snapshot().unwrap().predicted(),
        )
        .unwrap();
        fs::write(root.join(format!("{name}.json")), serde_json::to_vec_pretty(&serde_json::json!({"checksum":format!("0x{:016x}", editor.checksum()), "tick":editor.snapshot().unwrap().tick(), "position_raw":nav.navigator.position().map(FP::raw), "surface_digest":h.state().terrain_surface.state.current.as_ref().map(|s| s.digest()), "gpu_path":"production EditorApp headless viewport", "viewport_adapter":gpu.adapter_name(), "native_window":false})).unwrap()).unwrap();
    }
    bytes
}
fn changed(a: &[u8], b: &[u8]) -> usize {
    a.as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0.iter())
        .filter(|(a, b)| a[..3].iter().zip(&b[..3]).any(|(a, b)| a.abs_diff(*b) > 12))
        .count()
}
#[test]
#[ignore = "requires explicit offscreen GPU acceptance"]
fn actual_full_app_gpu_apply_undo_redo_save_hidden_png_reopen_edit_play_and_agent() {
    assert_eq!(
        std::env::var("ORR_REQUIRE_GPU").as_deref(),
        Ok("1"),
        "Explicit GPU acceptance requires ORR_REQUIRE_GPU=1"
    );
    let evidence = PathBuf::from(
        std::env::var_os("TERRAIN_SURFACE_CAPTURE_DIR")
            .expect("GPU acceptance requires TERRAIN_SURFACE_CAPTURE_DIR"),
    );
    assert!(
        evidence.is_absolute(),
        "GPU evidence directory must be absolute"
    );
    fs::create_dir(&evidence)
        .expect("GPU evidence directory must be fresh, with an existing parent");
    let (root, app) = fixture();
    let png_path = app.terrain_surface.state.png_path.clone();
    {
        let file = fs::File::create(&png_path).unwrap();
        let mut encoder = png::Encoder::new(file, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[
                245, 210, 40, 255, 40, 70, 220, 255, 220, 40, 150, 255, 50, 220, 70, 255,
            ])
            .unwrap();
    }
    let editor = app.editor;
    let mut h = Harness::builder()
        .with_size([1500., 1000.])
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(move |cc| {
            let info = cc.wgpu_render_state.as_ref().expect("mandatory Harness GPU adapter").adapter.get_info();
            fs::write(evidence.join("adapter.json"), serde_json::to_vec_pretty(&serde_json::json!({"harness_adapter":info.name,"device_type":format!("{:?}",info.device_type),"backend":format!("{:?}",info.backend),"native_window":false})).unwrap()).unwrap();
            EditorApp::new(editor, cc.wgpu_render_state.clone())
        });
    {
        let app = h.state_mut();
        app.terrain.local_path = "navigation/point_demo.orrt".into();
        app.terrain.open_for_editor(&app.editor).unwrap();
        app.terrain_surface.state.png_path = png_path;
    }
    let checker = capture(&mut h, "01-checker-edit");
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().clone();
    click_visible(&mut h, "Preview appearance PNG");
    assert!(h.state().terrain_surface.state.pending());
    click_visible(&mut h, "Apply appearance");
    assert!(h.state().terrain_surface.state.dirty());
    let applied = capture(&mut h, "02-applied-edit");
    let yellow = applied
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[0] > 120 && p[1] > 100 && p[2] < 90)
        .count();
    let blue = applied
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[0] < 90 && p[1] < 120 && p[2] > 120)
        .count();
    assert!(
        yellow > 30 && blue > 30,
        "literal yellow/blue authored pattern absent: {yellow}/{blue}"
    );
    assert!(
        changed(&checker, &applied) > 100,
        "literal terrain appearance must change pixels"
    );
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(*h.state().editor.history(), history);
    click_visible(&mut h, "Undo appearance");
    let undone = capture(&mut h, "03-undo-edit");
    assert_eq!(checker, undone);
    click_visible(&mut h, "Redo appearance");
    let redone = capture(&mut h, "04-redo-edit");
    assert_eq!(applied, redone);
    click_visible(&mut h, "Save appearance");
    assert!(!h.state().terrain_surface.state.dirty());
    let source = root.path().join("navigation/point_demo.orrt");
    assert_eq!(
        fs::read(&source).unwrap(),
        include_bytes!("../../../../scenes/navigation/point_demo.orrt")
    );
    fs::remove_file(root.path().join("appearance.png")).unwrap();
    if let Some(evidence) = std::env::var_os("TERRAIN_SURFACE_CAPTURE_DIR") {
        let destination = PathBuf::from(evidence).join("authored");
        fs::create_dir_all(destination.join("navigation")).unwrap();
        for relative in [
            "navigation.scene.yaml",
            "navigation/point_demo.orrt",
            "navigation/point_demo.orrt.surface.json",
        ] {
            fs::copy(root.path().join(relative), destination.join(relative)).unwrap();
        }
    }
    drop(h);
    let mut editor = Editor::open_game(
        &root.path().join("navigation.scene.yaml"),
        EditorGame::NavigationYard3D,
    )
    .unwrap();
    editor.sync();
    editor.camera3d = orr_render::OrbitCamera::new([0.5, 0.1, 0.5], 0.45, 0.75, 3.8);
    let mut h = Harness::builder()
        .with_size([1500., 1000.])
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(move |cc| EditorApp::new(editor, cc.wgpu_render_state.clone()));
    let reopened = capture(&mut h, "05-reopened-edit");
    assert_eq!(redone, reopened);
    let start = orr_remote::navigation_yard3d::navigation_from_view(
        h.state().editor.snapshot().unwrap().predicted(),
    )
    .unwrap()
    .navigator
    .position();
    h.state_mut().editor.step(2);
    let moving = capture(&mut h, "06-moving-play");
    assert_eq!(h.state().editor.mode(), Mode::Play);
    let current = orr_remote::navigation_yard3d::navigation_from_view(
        h.state().editor.snapshot().unwrap().predicted(),
    )
    .unwrap()
    .navigator
    .position();
    assert_ne!(start, current);
    assert!(changed(&reopened, &moving) > 10);
    assert!(h.state_mut().terrain_surface.state.current.is_some());
    let app = h.state_mut();
    assert!(
        app.terrain_surface
            .state
            .preview(&app.editor, &app.terrain)
            .is_err()
    );
}
#[test]
fn actual_path_widget_cancels_preview_and_hidden_panel_close_preserves_work() {
    let (_root, app) = fixture();
    let mut h = panel(app);
    h.get_by_label("Preview appearance PNG").click();
    h.run();
    assert!(h.state().terrain_surface.state.pending());
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Appearance PNG path")
        .focus();
    h.run();
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Appearance PNG path")
        .type_text("changed");
    h.run();
    assert!(!h.state().terrain_surface.state.pending());
    drop(h);
    let (_root, mut app) = fixture();
    preview_apply(&mut app);
    let digest = app.terrain_surface.state.current.as_ref().unwrap().digest();
    let mut h = Harness::builder()
        .with_size([1500., 1000.])
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(move |_| app);
    // Collapse the appearance controls before the real root-close request.
    h.get_by_label("Terrain appearance").scroll_to_me();
    h.run_steps(3);
    h.get_by_label("Terrain appearance").click();
    h.run_steps(3);
    h.input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    h.run_steps(1);
    assert!(
        h.output()
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .unwrap()
            .commands
            .iter()
            .any(|command| matches!(command, egui::ViewportCommand::CancelClose))
    );
    assert_eq!(
        h.state()
            .terrain_surface
            .state
            .current
            .as_ref()
            .unwrap()
            .digest(),
        digest
    );
    assert!(h.state().terrain_surface.state.dirty());
}
#[test]
fn actual_app_refuses_malformed_and_foreign_appearance_play_but_allows_valid_dirty() {
    let (root, mut app) = fixture();
    let sidecar = root.path().join("navigation/point_demo.orrt.surface.json");
    fs::write(&sidecar, b"malformed").unwrap();
    assert!(
        app.terrain_surface
            .state
            .reload(&app.editor, &app.terrain)
            .is_err()
    );
    let mut h = Harness::builder()
        .with_size([1500., 1000.])
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(move |_| app);
    h.run_steps(3);
    assert!(
        h.get_by_label(crate::app::LBL_PLAY)
            .accesskit_node()
            .is_disabled()
    );
    h.get_by_label(crate::app::LBL_PLAY).click();
    h.run_steps(3);
    h.state_mut().editor.sync();
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    fs::remove_file(sidecar).unwrap();
    {
        let app = h.state_mut();
        app.terrain_surface
            .state
            .reload(&app.editor, &app.terrain)
            .unwrap();
        preview_apply(app);
        app.terrain_surface.state.binding.as_mut().unwrap().epoch += 1;
    }
    h.run_steps(3);
    assert!(
        h.get_by_label(crate::app::LBL_PLAY)
            .accesskit_node()
            .is_disabled()
    );
    h.get_by_label(crate::app::LBL_PLAY).click();
    h.run_steps(3);
    h.state_mut().editor.sync();
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    {
        let app = h.state_mut();
        app.terrain_surface.state.binding.as_mut().unwrap().epoch = app.editor.scene_load_epoch();
    }
    h.run_steps(3);
    assert!(h.state().terrain_surface.state.dirty());
    click_visible(&mut h, crate::app::LBL_PLAY);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    assert!(
        h.state().terrain_surface.state.dirty(),
        "Play must not implicitly save or discard valid presentation changes"
    );
}
#[test]
fn mixed_size_undo_and_redo_each_enforce_retained_byte_budget() {
    let (_root, mut app) = fixture();
    let context = Context::get(&app.editor, &app.terrain).unwrap();
    let mut large = Vec::new();
    for first in [251, 252, 253, 254] {
        let mut pixels = vec![255; 512 * 512 * 4];
        pixels[0] = first;
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 512, 512);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
        }
        large.push(Arc::new(
            Surface::from_png(&context.binding.relative, &context.binding.asset, &png).unwrap(),
        ));
    }
    let small = Arc::new(
        Surface::from_png(
            &context.binding.relative,
            &context.binding.asset,
            &png_bytes([1, 2, 3, 255]),
        )
        .unwrap(),
    );
    let bytes = |state: &State| {
        state
            .undo
            .iter()
            .chain(&state.redo)
            .filter_map(|s| s.as_ref())
            .map(|s| s.to_bytes().unwrap().len())
            .sum::<usize>()
    };
    let four_large = large
        .iter()
        .map(|s| s.to_bytes().unwrap().len())
        .sum::<usize>();
    assert!(
        four_large > HISTORY_BYTES,
        "fixture must actually exceed the byte cap after transition"
    );
    for redo in [false, true] {
        let state = &mut app.terrain_surface.state;
        state.undo.clear();
        state.redo.clear();
        let stack = if redo {
            &mut state.redo
        } else {
            &mut state.undo
        };
        stack.extend(large[..3].iter().cloned().map(Some));
        stack.push(Some(small.clone()));
        state.current = Some(large[3].clone());
        assert!(bytes(state) <= HISTORY_BYTES);
        state.history(&app.editor, &app.terrain, redo).unwrap();
        assert!(
            bytes(state) <= HISTORY_BYTES,
            "{redo}: history byte cap exceeded on transition"
        );
        assert_eq!(state.current.as_ref().unwrap().digest(), small.digest());
    }
}
#[test]
fn save_rechecks_scene_and_backing_identity_without_requiring_saved_heights() {
    let (root, mut app) = fixture();
    preview_apply(&mut app);
    let terrain_path = root.path().join("navigation/point_demo.orrt");
    let original = fs::read(&terrain_path).unwrap();
    let sidecar = root.path().join("navigation/point_demo.orrt.surface.json");
    let foreign = Terrain::new(
        "different/asset.orrt".into(),
        2,
        2,
        [FP::ZERO; 2],
        FP::ONE,
        vec![FP::ZERO; 4],
        vec![false],
    )
    .unwrap();
    fs::write(&terrain_path, foreign.cook()).unwrap();
    assert!(
        app.terrain_surface
            .state
            .save(&app.editor, &app.terrain)
            .is_err()
    );
    assert!(!sidecar.exists());
    assert!(app.terrain_surface.state.dirty());
    fs::write(&terrain_path, &original).unwrap();
    app.terrain
        .document
        .apply(&[orr_terrain::Edit::SetHeight {
            x: 0,
            z: 0,
            height: FP::from_raw(345),
        }])
        .unwrap();
    assert!(app.terrain.document.dirty());
    let scene = app.editor.path().unwrap();
    let scene_bytes = fs::read(&scene).unwrap();
    let result = app
        .terrain_surface
        .state
        .save_with_checkpoint(&app.editor, &app.terrain, || {
            let mut changed = scene_bytes.clone();
            changed.extend_from_slice(b"\n# concurrent save\n");
            fs::write(&scene, changed).unwrap();
        });
    assert!(result.is_err());
    assert!(!sidecar.exists());
    assert!(app.terrain_surface.state.dirty());
    fs::write(&scene, &scene_bytes).unwrap();
    let result = app
        .terrain_surface
        .state
        .save_with_checkpoint(&app.editor, &app.terrain, || {
            fs::write(&terrain_path, foreign.cook()).unwrap();
        });
    assert!(result.is_err());
    assert!(!sidecar.exists());
    assert!(app.terrain_surface.state.dirty());
    fs::write(&terrain_path, &original).unwrap();
    app.terrain_surface
        .state
        .save(&app.editor, &app.terrain)
        .unwrap();
    assert!(!app.terrain_surface.state.dirty());
    assert!(app.terrain.document.dirty());
    assert_eq!(fs::read(&terrain_path).unwrap(), original);
    // Removing a saved appearance uses the same source pins as replacing it.
    app.terrain_surface
        .state
        .history(&app.editor, &app.terrain, false)
        .unwrap();
    let saved = fs::read(&sidecar).unwrap();
    let result = app
        .terrain_surface
        .state
        .save_with_checkpoint(&app.editor, &app.terrain, || {
            fs::write(&scene, b"changed during removal").unwrap();
        });
    assert!(result.is_err());
    assert_eq!(fs::read(&sidecar).unwrap(), saved);
}
