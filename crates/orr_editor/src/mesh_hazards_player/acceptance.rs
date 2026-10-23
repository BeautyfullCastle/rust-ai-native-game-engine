#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
//! Opt-in relocated production PlayerApp proof. Input is synthetic egui input;
//! rendering is real software Vulkan. This does not establish native-window,
//! physical-input, hardware-GPU, Windows, or shipped-executable UI acceptance.
use super::{PlayerApp, start_playback, verify_executable};
use crate::{Editor, EditorApp, game::EditorGame};
use egui::{Event, Key, Modifiers, PointerButton, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use egui_wgpu::{WgpuSetupCreateNew, wgpu};
use orr_remote::mesh_navigation::{hazards, mesh_from_view, mission};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

fn env_path(name: &str) -> PathBuf {
    let path = PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name}")));
    assert!(path.is_absolute(), "{name} must be absolute");
    path
}

fn inventory(root: &Path) -> BTreeMap<String, String> {
    fs::read_dir(root)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            assert!(path.is_file());
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            let hash = Sha256::digest(fs::read(path).unwrap())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            (name, hash)
        })
        .collect()
}

fn settle(h: &mut Harness<'_, PlayerApp>) {
    for _ in 0..3 {
        h.state_mut().inner.editor.sync();
        h.run_steps(2);
    }
}

fn click(h: &mut Harness<'_, PlayerApp>, label: &str) {
    assert!(
        !h.get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .is_disabled(),
        "{label} must be enabled"
    );
    h.get_by_role_and_label(Role::Button, label)
        .click_accesskit();
    h.step();
    h.state_mut().inner.editor.sync();
    // Rebuild dismissed-dialog/accessibility state only while simulation is
    // paused; extra live frames consume real wall time on software rendering.
    if !h.state().inner.editor.sim().playing {
        settle(h);
    } else {
        // AccessKit reflects the previously drawn label until the next frame.
        h.step();
        h.state_mut().inner.editor.sync();
    }
}

fn wait_for(h: &mut Harness<'_, PlayerApp>, description: &str, ready: impl Fn(&PlayerApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if ready(h.state()) {
            return;
        }
        h.step();
        h.state_mut().inner.editor.sync();
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {description}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn lost(h: &Harness<'_, PlayerApp>) -> bool {
    hazards::from_view(h.state().inner.editor.snapshot().unwrap().predicted())
        .unwrap()
        .unwrap()
        .run
        .lost
        != 0
}

fn capture(h: &mut Harness<'_, PlayerApp>, root: &Path, name: &str) {
    settle(h);
    let frame = h.render().expect("actual PlayerApp software-GPU composite");
    assert!(frame.width() as u64 * frame.height() as u64 <= super::MAX_CAPTURE_PIXELS);
    let output = root.join(name);
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .unwrap();
    let mut encoder =
        png::Encoder::new(std::io::BufWriter::new(file), frame.width(), frame.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(frame.as_raw()).unwrap();
    writer.finish().unwrap();
}

fn click_world(h: &mut Harness<'_, PlayerApp>, point: [f32; 3]) {
    let app = &h.state().inner;
    let viewport = app.ui.viewport_rect.unwrap();
    let size = app.ui.viewport_px;
    let camera = app.editor.presentation_camera3d(size).unwrap();
    let pixel = camera
        .world_to_screen(point, size)
        .expect("hazard center projects into viewport");
    let scale = viewport.width() / size.0 as f32;
    let pointer = viewport.min + egui::vec2(pixel[0] * scale, pixel[1] * scale);
    assert!(viewport.contains(pointer));
    h.event(Event::PointerMoved(pointer));
    h.event(Event::PointerButton {
        pos: pointer,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.event(Event::PointerButton {
        pos: pointer,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.state_mut().inner.editor.sync();
}

#[test]
#[ignore = "requires relocated real UI export, source-masked process and software Vulkan"]
fn relocated_bundle_player_ui_pause_loss_cancel_escape_restart() {
    assert_eq!(
        std::env::var("ORR_HAZARDS_PLAYER_ACCEPTANCE").as_deref(),
        Ok("1")
    );
    let bundle_root = env_path("ORR_HAZARDS_PLAYER_BUNDLE");
    let root = env_path("ORR_HAZARDS_PLAYER_CAPTURE_DIR");
    let hidden = [
        env_path("ORR_HAZARDS_PLAYER_SOURCE_SCENE"),
        env_path("ORR_HAZARDS_PLAYER_SOURCE_RUNTIME"),
        env_path("ORR_HAZARDS_PLAYER_SOURCE_REPO").join("Cargo.toml"),
    ];
    for path in &hidden {
        assert!(
            !path.exists(),
            "source must be unavailable in this process: {}",
            path.display()
        );
    }
    fs::create_dir(&root).expect("fresh immutable acceptance directory");
    let before = inventory(&bundle_root);
    assert_eq!(before.len(), 3);
    let bundle = crate::mesh_hazards_export::open_bundle(&bundle_root).unwrap();
    // This verifies the exported runtime artifact. The process executing this
    // harness is the libtest executable, not the standalone Player executable.
    verify_executable(&bundle.runtime, &bundle).unwrap();
    assert_eq!(
        bundle.runtime_sha256,
        std::env::var("ORR_HAZARDS_PLAYER_EXPECTED_RUNTIME_SHA256").unwrap()
    );
    let mut editor = Editor::open_game(&bundle.scene, EditorGame::MeshHazardsV1).unwrap();
    editor.sync();
    assert_eq!(editor.checksum(), bundle.checksum);
    assert_eq!(
        mesh_from_view(editor.snapshot().unwrap().predicted())
            .unwrap()
            .graph
            .revision(),
        bundle.graph_revision
    );
    let mut navigation = crate::mesh_navigation::Panel::default();
    navigation.sync(&editor);
    let mut camera = crate::mesh_camera_panel::Panel::default();
    camera.sync(&mut editor);
    let mut lighting = crate::mesh_lighting_panel::Panel::default();
    lighting.sync(&editor);
    editor.set_mesh_navigation_blocker(
        navigation
            .play_blocker(&editor)
            .or_else(|| camera.play_blocker(&editor))
            .or_else(|| lighting.play_blocker(&editor)),
    );
    start_playback(&mut editor, Some(0), false).unwrap();
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    setup.native_adapter_selector = Some(Arc::new(|adapters, _| {
        adapters
            .iter()
            .find(|adapter| {
                let info = adapter.get_info();
                info.backend == wgpu::Backend::Vulkan && info.device_type == wgpu::DeviceType::Cpu
            })
            .cloned()
            .ok_or_else(|| "software Vulkan adapter required".to_owned())
    }));
    let mut h = Harness::builder()
        .with_size([960.0, 600.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu_setup(setup.into())
        .build_eframe(move |cc| {
            let mut inner = EditorApp::new(editor, cc.wgpu_render_state.clone());
            inner.set_mesh_player_input_allowed(true);
            PlayerApp {
                inner,
                session: Default::default(),
            }
        });
    settle(&mut h);
    crate::app::mesh_hazards_player::validate_fresh_session(&h.state().inner.editor).unwrap();
    let adapter = h
        .state()
        .inner
        .viewport3d_gpu()
        .unwrap()
        .gpu()
        .adapter_name();
    eprintln!("Relocated PlayerApp software adapter: {adapter}");
    assert!(adapter.to_lowercase().contains("llvmpipe"));
    capture(&mut h, &root, "01-relocated-paused.png");

    click(&mut h, "Resume");
    wait_for(&mut h, "live simulation advances", |app| {
        app.inner.editor.sim().playing && app.inner.editor.sim().head_tick > 0
    });
    click(&mut h, "Pause");
    let paused_tick = h.state().inner.editor.sim().head_tick;
    let paused_checksum = h.state().inner.editor.checksum();
    assert!(paused_tick > 0);
    std::thread::sleep(Duration::from_millis(60));
    settle(&mut h);
    assert!(!h.state().inner.editor.sim().playing);
    assert_eq!(h.state().inner.editor.sim().head_tick, paused_tick);
    assert_eq!(h.state().inner.editor.checksum(), paused_checksum);
    capture(&mut h, &root, "02-paused-after-resume.png");

    // Rendering can be much slower than realtime on llvmpipe. Start a fresh
    // session via the actual UI, then use the existing speed control solely to
    // give the synthetic pointer sequence time to arrive before mission victory.
    click(&mut h, "Restart game…");
    click(&mut h, "Restart game");
    eprintln!(
        "First UI restart: mode={:?}, tick={}, status={:?}",
        h.state().inner.editor.mode(),
        h.state().inner.editor.sim().head_tick,
        h.state()
            .inner
            .editor
            .status()
            .map(|status| (&status.text, status.error))
    );
    capture(&mut h, &root, "02b-first-restart-result.png");
    crate::app::mesh_hazards_player::validate_fresh_session(&h.state().inner.editor).unwrap();
    h.state_mut().inner.editor.set_speed(0.25);
    h.state_mut().inner.editor.sync();
    h.step();
    assert_eq!(h.state().inner.editor.sim().head_tick, 0);
    assert_eq!(h.state().inner.editor.checksum(), bundle.checksum);
    assert!(!h.state().inner.editor.sim().playing);
    click(&mut h, "Resume");
    click(&mut h, "Enable mesh goal input");
    wait_for(&mut h, "managed input acknowledged", |app| {
        app.inner.editor.mesh_play_input_hint() == "Managed player input active"
    });
    let hazard_center = hazards::from_view(h.state().inner.editor.snapshot().unwrap().predicted())
        .unwrap()
        .unwrap()
        .config
        .spheres[0]
        .center
        .map(|v| v.to_f32());
    // Use actual projected pointer events through the production viewport picker;
    // do not patch simulation state or inject a goal through an internal API.
    click_world(&mut h, hazard_center);
    let deadline = Instant::now() + Duration::from_secs(15);
    while !lost(&h) {
        h.step();
        h.state_mut().inner.editor.sync();
        assert!(
            Instant::now() < deadline,
            "projected live goal must reach hazard and lose"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    h.step();
    h.state_mut().inner.editor.sync();
    assert!(h.query_by_label("Lost · restart to try again").is_some());
    if h.state().inner.editor.sim().playing {
        click(&mut h, "Pause");
    }
    assert!(!h.state().inner.editor.mesh_play_input_enabled());
    capture(&mut h, &root, "03-live-input-loss.png");
    let loss_tick = h.state().inner.editor.sim().head_tick;
    let loss_checksum = h.state().inner.editor.checksum();

    click(&mut h, "Restart game…");
    assert!(h.state().session.dialog_open());
    assert!(
        h.get_by_role_and_label(Role::Button, "Resume")
            .accesskit_node()
            .is_disabled()
    );
    capture(&mut h, &root, "04-restart-confirmation.png");
    click(&mut h, "Cancel restart");
    assert!(!h.state().session.dialog_open());
    assert!(lost(&h));
    assert_eq!(h.state().inner.editor.sim().head_tick, loss_tick);
    assert_eq!(h.state().inner.editor.checksum(), loss_checksum);
    click(&mut h, "Restart game…");
    h.key_press(Key::Escape);
    settle(&mut h);
    assert!(!h.state().session.dialog_open());
    assert!(lost(&h));
    assert_eq!(h.state().inner.editor.sim().head_tick, loss_tick);
    assert_eq!(h.state().inner.editor.checksum(), loss_checksum);
    click(&mut h, "Restart game…");
    click(&mut h, "Restart game");
    crate::app::mesh_hazards_player::validate_fresh_session(&h.state().inner.editor).unwrap();
    assert_eq!(h.state().inner.editor.checksum(), bundle.checksum);
    assert_eq!(
        h.state().inner.editor.last_stopped().unwrap().tick,
        loss_tick
    );
    assert!(!lost(&h));
    assert!(!h.state().inner.editor.mesh_play_input_enabled());
    assert_eq!(
        mission::from_view(h.state().inner.editor.snapshot().unwrap().predicted())
            .unwrap()
            .unwrap()
            .run,
        mission::MeshMissionRun::default()
    );
    capture(&mut h, &root, "05-restarted-tick-zero.png");
    // A repeated confirmed restart must remain coherent and paused.
    click(&mut h, "Restart game…");
    click(&mut h, "Restart game");
    crate::app::mesh_hazards_player::validate_fresh_session(&h.state().inner.editor).unwrap();
    assert_eq!(h.state().inner.editor.checksum(), bundle.checksum);
    let after = inventory(&bundle_root);
    assert_eq!(
        before, after,
        "read-only relocated bundle must remain byte-identical"
    );
    let receipt = serde_json::json!({
        "schema": 1, "ui": "production PlayerApp via egui_kittest", "input": "synthetic AccessKit buttons and projected egui pointer events", "native_window": false,
        "standalone_executable_ui": false, "hardware_gpu": false, "adapter": adapter,
        "bundle": bundle_root, "hidden_source_paths": hidden, "bundle_hashes_before": before, "bundle_hashes_after": after,
        "runtime_sha256": bundle.runtime_sha256, "initial_checksum": bundle.checksum, "paused_tick": paused_tick, "loss_tick": loss_tick,
        "test_only_wall_clock_speed_permille": 250, "pause_resume": true, "actual_live_goal_loss": true, "cancel_preserves_loss": true, "escape_preserves_loss": true, "confirmed_restart_resets": true, "repeated_restart": true
    });
    fs::write(
        root.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
}
