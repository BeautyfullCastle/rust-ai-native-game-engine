//! Actual offscreen Yard RGB coefficient gain; not a physical bake/window test.
#![cfg(feature = "irradiance-probes")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui::accesskit::Role;
use egui_kittest::{kittest::{NodeT, Queryable}, Harness};
use orr_editor::{app::BottomTab, game::EditorGame, Editor, EditorApp, Mode};
use orr_render::irradiance::{IrradianceGrid, IrradianceProvenance};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
type HostFrame = (u64, u64, Vec<u8>);
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
    let client = editor.agent_client("irradiance-gain-byte-probe").unwrap();
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
                let result = (tick, checksum, frame.frame.to_bytes());
                client
                    .call("watch.unsubscribe", json!({"topics":["frames"]}))
                    .unwrap();
                client.local_frames.clear();
                return result;
            }
        }
        assert!(
            Instant::now() < deadline,
            "matching host frame bytes missing"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..3 {
        sync_clock(&mut h.state_mut().editor);
        h.run_steps(2);
    }
    assert!(h.state().editor.down().is_none());
}
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).scroll_to_me();
    h.run_steps(12);
    assert!(
        !h.get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .is_disabled(),
        "disabled {label}"
    );
    h.get_by_role_and_label(Role::Button, label)
        .click_accesskit();
    settle(h);
}
fn harness(scene: &Path) -> Harness<'static, EditorApp> {
    let mut editor = Editor::open_game(scene, EditorGame::Yard3D).unwrap();
    editor.camera3d = orr_render::OrbitCamera::new([0.0, 1.5, 0.0], 0.45, 0.65, 14.0);
    editor.select(None);
    sync_clock(&mut editor);
    Harness::builder()
        .with_size([1200., 900.])
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(move |cc| {
            let info = cc
                .wgpu_render_state
                .as_ref()
                .expect("GPU required")
                .adapter
                .get_info();
            eprintln!(
                "irradiance gain adapter={} backend={:?} device={:?}",
                info.name, info.backend, info.device_type
            );
            let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
            app.ui.bottom_tab = BottomTab::History;
            app
        })
}
fn capture(h: &mut Harness<'_, EditorApp>, root: &Path, name: &str) -> Vec<u8> {
    let target = (800, 600);
    for _ in 0..6 {
        settle(h);
        let actual = h
            .state()
            .viewport3d_gpu()
            .expect("Yard viewport")
            .gpu()
            .target()
            .size();
        if actual == target {
            break;
        }
        let size = h.ctx.viewport_rect().size();
        let ratio = h.ctx.pixels_per_point();
        h.set_size(egui::vec2(
            size.x + (target.0 as f32 - actual.0 as f32) / ratio,
            size.y + (target.1 as f32 - actual.1 as f32) / ratio,
        ));
    }
    settle(h);
    let app = h.state();
    let gpu = app.viewport3d_gpu().unwrap().gpu();
    assert_eq!(gpu.target().size(), target);
    assert_eq!(
        gpu.irradiance(),
        app.irradiance.grid_for_editor(&app.editor),
        "production viewport consumes the admitted grid"
    );
    let pixels = gpu.read_rgba8();
    assert_eq!(pixels.len(), 800 * 600 * 4);
    let image = egui::ColorImage::from_rgba_unmultiplied([800, 600], &pixels);
    orr_editor::app::write_png(&root.join(name), &image).unwrap();
    pixels
}
const ORIGINAL: [[f32; 3]; 9] = [
    [2.0, 1.0, 0.5],
    [-0.25, 0.125, -0.0625],
    [0.125, -0.25, 0.0625],
    [-0.125, 0.0625, -0.125],
    [0.03125, -0.0625, 0.125],
    [-0.0625, 0.03125, -0.125],
    [0.125, -0.03125, 0.0625],
    [-0.03125, 0.125, -0.0625],
    [0.0625, -0.125, 0.03125],
];
// Literal binary-exact products for RGB gain [2, 1/2, 0], independently
// enumerated rather than calling the production gain or SH evaluation helper.
const EXPECTED: [[f32; 3]; 9] = [
    [4.0, 0.5, 0.0],
    [-0.5, 0.0625, 0.0],
    [0.25, -0.125, 0.0],
    [-0.25, 0.03125, 0.0],
    [0.0625, -0.03125, 0.0],
    [-0.125, 0.015625, 0.0],
    [0.25, -0.015625, 0.0],
    [-0.0625, 0.0625, 0.0],
    [0.125, -0.0625, 0.0],
];
#[test]
#[ignore = "actual GPU; requires fresh absolute ORR_IRRADIANCE_GAIN_CAPTURE_DIR"]
fn actual_rgb_gain_preserves_signed_sh9_history_host_and_hidden_source_reopen() {
    let root = PathBuf::from(
        std::env::var_os("ORR_IRRADIANCE_GAIN_CAPTURE_DIR").expect("capture directory required"),
    );
    assert!(root.is_absolute());
    fs::create_dir(&root).unwrap();
    let scene = root.join("gain.scene.yaml");
    let text = include_str!("../../../scenes/yard3d_authoring.scene.yaml")
        .replace("kind: dynamic", "kind: static")
        .replace("inv_mass: 1", "inv_mass: 0");
    fs::write(&scene, &text).unwrap();
    let original = IrradianceGrid {
        enabled: true,
        provenance: IrradianceProvenance::Imported,
        dimensions: [4; 3],
        origin: [-8.0, -4.0, -8.0],
        spacing: [6.0, 4.0, 6.0],
        coefficients: vec![ORIGINAL; 64],
        ..IrradianceGrid::default()
    };
    let source = root.join("directional-source.json");
    let source_bytes = serde_json::to_vec(&original).unwrap();
    fs::write(&source, &source_bytes).unwrap();
    let mut h = harness(&scene);
    settle(&mut h);
    let expected_host = host_frame(&mut h.state_mut().editor);
    let history = h.state().editor.history().clone();
    {
        let app = h.state_mut();
        app.irradiance.open_for_editor(&app.editor, true).unwrap();
        app.irradiance
            .import_json_for_editor(&app.editor, &fs::read(&source).unwrap())
            .unwrap();
        app.irradiance.save().unwrap();
    }
    settle(&mut h);
    h.get_by_label("Irradiance probes").click_accesskit();
    settle(&mut h);
    assert_eq!(
        h.state().irradiance.grid_for_editor(&h.state().editor),
        Some(&original)
    );
    let sidecar = h.state().irradiance.bindings.as_ref().unwrap().path.clone();
    let initial_file = fs::read(&sidecar).unwrap();
    let before = capture(&mut h, &root, "01-imported-directional.png");
    h.state_mut().irradiance.coefficient_gain = [2.0, 0.5, 0.0];
    settle(&mut h);
    assert_eq!(capture(&mut h, &root, "02-gain-controls-only.png"), before);
    assert_eq!(fs::read(&sidecar).unwrap(), initial_file);
    click(&mut h, "Apply irradiance RGB gain");
    assert!(
        h.state().irradiance.error().is_none(),
        "{:?}",
        h.state().irradiance.error()
    );
    let expected = IrradianceGrid {
        coefficients: vec![EXPECTED; 64],
        provenance: IrradianceProvenance::Authored,
        ..original.clone()
    };
    assert_eq!(
        h.state().irradiance.grid_for_editor(&h.state().editor),
        Some(&expected)
    );
    let after = capture(&mut h, &root, "03-gain-applied.png");
    let changed = before
        .as_chunks::<4>()
        .0
        .iter()
        .zip(after.as_chunks::<4>().0)
        .filter(|(a, b)| (0..3).any(|i| a[i].abs_diff(b[i]) > 8))
        .count();
    assert!(
        changed > 50,
        "actual signed coefficient gain must change visible surface pixels: {changed}"
    );
    assert_eq!(fs::read(&sidecar).unwrap(), initial_file);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(host_frame(&mut h.state_mut().editor), expected_host);
    click(&mut h, "Undo irradiance");
    assert_eq!(
        h.state().irradiance.grid_for_editor(&h.state().editor),
        Some(&original)
    );
    assert_eq!(capture(&mut h, &root, "04-undo.png"), before);
    click(&mut h, "Redo irradiance");
    assert_eq!(capture(&mut h, &root, "05-redo.png"), after);
    let doc_before = h
        .state()
        .irradiance
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    let dirty = h.state().irradiance.bindings.as_ref().unwrap().dirty();
    // DragValue's range clamps public fields during UI rendering. Exercise the
    // backend's invalid-gain guard directly before another UI frame; valid gain
    // activation above genuinely used the accessible production button.
    {
        let app = h.state_mut();
        app.irradiance.coefficient_gain = [17.0, 1.0, 1.0];
        assert!(app.irradiance.apply_coefficient_gain(&app.editor).is_err());
    }
    assert!(h.state().irradiance.error().is_some());
    assert_eq!(
        h.state().irradiance.bindings.as_ref().unwrap().document(),
        &doc_before
    );
    assert_eq!(
        h.state().irradiance.bindings.as_ref().unwrap().dirty(),
        dirty
    );
    assert_eq!(
        capture(&mut h, &root, "06-invalid-gain-unchanged.png"),
        after
    );
    assert_eq!(fs::read(&sidecar).unwrap(), initial_file);
    h.state_mut().irradiance.coefficient_gain = [1.0; 3];
    click(&mut h, "Save irradiance");
    assert!(!h.state().irradiance.bindings.as_ref().unwrap().dirty());
    let saved = fs::read(&sidecar).unwrap();
    assert_ne!(saved, initial_file);
    assert_eq!(host_frame(&mut h.state_mut().editor), expected_host);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(fs::read(&scene).unwrap(), text.as_bytes());
    drop(h);
    let hidden = root.join("hidden-directional-source.json");
    fs::rename(&source, &hidden).unwrap();
    assert!(!source.exists());
    let mut reopened = harness(&scene);
    settle(&mut reopened);
    {
        let app = reopened.state_mut();
        app.irradiance.open_for_editor(&app.editor, false).unwrap();
        app.irradiance.reload(&app.editor).unwrap();
    }
    settle(&mut reopened);
    assert_eq!(
        reopened
            .state()
            .irradiance
            .grid_for_editor(&reopened.state().editor),
        Some(&expected)
    );
    assert_eq!(
        capture(&mut reopened, &root, "07-source-hidden-reopen.png"),
        after
    );
    assert_eq!(host_frame(&mut reopened.state_mut().editor), expected_host);
    assert_eq!(fs::read(&scene).unwrap(), text.as_bytes());
    assert_eq!(fs::read(&sidecar).unwrap(), saved);
    assert_eq!(fs::read(hidden).unwrap(), source_bytes);
}
