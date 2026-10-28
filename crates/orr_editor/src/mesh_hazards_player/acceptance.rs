#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
//! Headless production EditorApp/PlayerApp acceptance with synthetic egui input.
//! Opt-in software-GPU lanes capture pixels; no lane establishes
//! native-window, physical-input, hardware-GPU, Windows, or standalone-process UI acceptance.
use super::{PlayerApp, start_playback, verify_executable, verify_loaded_camera_bundle};
use crate::{Editor, EditorApp, game::EditorGame};
use egui::{Event, Key, Modifiers, PointerButton, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use egui_wgpu::{WgpuSetupCreateNew, wgpu};
use orr_fp::FP;
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
    lost_app(h.state())
}

fn lost_app(app: &PlayerApp) -> bool {
    hazards::from_view(app.inner.editor.snapshot().unwrap().predicted())
        .unwrap()
        .unwrap()
        .run
        .lost
        != 0
}

fn capture_frame<State: eframe::App>(h: &mut Harness<'_, State>, root: &Path, name: &str) {
    let frame = h
        .render()
        .expect("actual software-GPU EditorApp/PlayerApp composite");
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

fn capture(h: &mut Harness<'_, PlayerApp>, root: &Path, name: &str) {
    settle(h);
    capture_frame(h, root, name);
}

fn capture_editor(h: &mut Harness<'_, EditorApp>, root: &Path, name: &str) {
    settle_editor(h);
    capture_frame(h, root, name);
}

fn software_gpu_setup(backend: wgpu::Backend) -> egui_wgpu::WgpuSetupCreateNew {
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = match backend {
        wgpu::Backend::Vulkan => wgpu::Backends::VULKAN,
        wgpu::Backend::Gl => wgpu::Backends::GL,
        _ => panic!("unsupported software-GPU acceptance backend"),
    };
    setup.native_adapter_selector = Some(Arc::new(move |adapters, _| {
        adapters
            .iter()
            .find(|adapter| {
                let info = adapter.get_info();
                info.backend == backend && info.device_type == wgpu::DeviceType::Cpu
            })
            .cloned()
            .inspect(|adapter| eprintln!("camera acceptance adapter: {:?}", adapter.get_info()))
            .ok_or_else(|| format!("software {backend:?} adapter required"))
    }));
    setup
}

fn click_world(h: &mut Harness<'_, PlayerApp>, point: [f32; 3]) -> [FP; 3] {
    let app = &h.state().inner;
    let viewport = app.ui.viewport_rect.unwrap();
    let size = app.ui.viewport_px;
    let camera = app.editor.presentation_camera3d(size).unwrap();
    let pixel = camera
        .world_to_screen(point, size)
        .expect("hazard center projects into viewport");
    let (origin, direction) = camera.screen_ray(pixel, size);
    let view = mesh_from_view(app.editor.snapshot().unwrap().predicted()).unwrap();
    let expected_goal =
        crate::mesh_agent_view::pick(&view.graph, origin, direction, view.agent.spec.max_vertical)
            .unwrap()
            .expect("projected synthetic point intersects a walkable surface")
            .position;
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
    expected_goal
}

fn assert_authoritative_goal_near(
    h: &Harness<'_, PlayerApp>,
    expected: [FP; 3],
    tolerance_raw: i64,
) {
    let goal = mesh_from_view(h.state().inner.editor.snapshot().unwrap().predicted())
        .unwrap()
        .pin
        .goal;
    for axis in 0..3 {
        assert!(
            goal[axis].raw().abs_diff(expected[axis].raw()) <= tolerance_raw as u64,
            "authoritative goal axis {axis} ({:?}) must be within {tolerance_raw} raw units of projected target {:?}",
            goal[axis],
            expected[axis]
        );
    }
}

fn wait_for_authoritative_goal(
    h: &mut Harness<'_, PlayerApp>,
    expected: [FP; 3],
    tolerance_raw: i64,
    description: &str,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let actual = mesh_from_view(h.state().inner.editor.snapshot().unwrap().predicted())
            .unwrap()
            .pin
            .goal;
        if (0..3)
            .all(|axis| actual[axis].raw().abs_diff(expected[axis].raw()) <= tolerance_raw as u64)
        {
            assert_authoritative_goal_near(h, expected, tolerance_raw);
            return;
        }
        assert!(
            Instant::now() < deadline,
            "projected click did not become the authoritative {description}: got {actual:?}, expected {expected:?}"
        );
        h.step();
        h.state_mut().inner.editor.sync();
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn settle_editor(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..3 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
}

fn click_editor(h: &mut Harness<'_, EditorApp>, role: Role, label: &str) {
    assert!(
        !h.get_by_role_and_label(role, label)
            .accesskit_node()
            .is_disabled(),
        "{label} must be enabled"
    );
    h.get_by_role_and_label(role, label).click_accesskit();
    h.step();
    h.state_mut().editor.sync();
    settle_editor(h);
}

fn type_editor_field(h: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    h.run_steps(20);
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    assert!(
        h.get_by_role_and_label(Role::TextInput, label).is_focused(),
        "widget field {label} must receive focus before editing"
    );
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    h.run_steps(2);
    let typed = h
        .get_by_role_and_label(Role::TextInput, label)
        .value()
        .unwrap_or_default();
    assert_eq!(
        typed, value,
        "widget field {label} must accept synthetic text input"
    );
}

#[test]
fn hazards_editor_camera_widget_apply_undo_save_reopen_and_play() {
    hazards_camera_delivery_scenario(None, None);
}

#[test]
#[ignore = "requires opt-in headless software Vulkan and fresh evidence directory"]
fn hazards_editor_camera_delivery_software_vulkan_evidence() {
    hazards_camera_gpu_evidence(wgpu::Backend::Vulkan);
}

#[test]
#[ignore = "requires opt-in headless software GL and fresh evidence directory"]
fn hazards_editor_camera_delivery_software_gl_evidence() {
    hazards_camera_gpu_evidence(wgpu::Backend::Gl);
}

fn hazards_camera_gpu_evidence(backend: wgpu::Backend) {
    assert_eq!(
        std::env::var("ORR_HAZARDS_CAMERA_PLAYER_GPU_ACCEPTANCE").as_deref(),
        Ok("1")
    );
    let evidence = env_path("ORR_HAZARDS_CAMERA_PLAYER_CAPTURE_DIR");
    fs::create_dir(&evidence).expect("fresh software-GPU evidence directory");
    env_path("ORR_HAZARDS_CAMERA_PLAYER_RUNTIME");
    std::env::var("ORR_HAZARDS_CAMERA_PLAYER_EXPECTED_RUNTIME_SHA256")
        .expect("set the independently built player runtime SHA-256");
    hazards_camera_delivery_scenario(Some(backend), Some(&evidence));
}

fn hazards_camera_delivery_scenario(backend: Option<wgpu::Backend>, evidence: Option<&Path>) {
    let rendered = backend.is_some();
    let directory = tempfile::tempdir().expect("isolated camera acceptance directory");
    let scene = directory.path().join("camera-hazards.scene.yaml");
    let editor = crate::mesh_hazards_creator::prepare(
        &scene,
        &crate::mesh_workspace::mission::Settings::default(),
        true,
    )
    .expect("create admitted hazards project without a listener");

    let mut h = if rendered {
        Harness::builder()
            .with_size([960.0, 600.0])
            .with_step_dt(1.0 / 60.0)
            .wgpu_setup(software_gpu_setup(backend.unwrap()).into())
            .build_eframe(move |cc| EditorApp::new(editor, cc.wgpu_render_state.clone()))
    } else {
        Harness::builder()
            .with_size([960.0, 600.0])
            .with_step_dt(1.0 / 60.0)
            .renderer(egui_kittest::LazyRenderer::Uninitialized {
                textures_delta: Default::default(),
                builder: None,
            })
            .build_eframe(move |_| EditorApp::new(editor, None))
    };
    settle_editor(&mut h);
    let simulation_checksum_before_camera_edits = h.state().editor.checksum();
    if h.query_by_label("Mesh game camera").is_some() {
        h.query_by_label("Mesh game camera")
            .unwrap()
            .click_accesskit();
        h.step();
    }
    settle_editor(&mut h);
    let viewport_size = h.state().ui.viewport_px;
    let original_perspective = h
        .state()
        .editor
        .presentation_camera3d(viewport_size)
        .unwrap();
    assert!(matches!(
        original_perspective.projection,
        orr_render::camera3d::Projection::Perspective { .. }
    ));

    type_editor_field(&mut h, "Game camera target X", "1.25");
    click_editor(&mut h, Role::RadioButton, "Orthographic game camera");
    type_editor_field(&mut h, "Game camera orthographic half height", "6");
    click_editor(&mut h, Role::Button, "Apply game camera");
    let orthographic_camera = h
        .state()
        .editor
        .presentation_camera3d(viewport_size)
        .unwrap();
    // Authored half-height expands in a portrait viewport to preserve horizontal framing.
    let expected_half_height = 6.0 / (viewport_size.0 as f32 / viewport_size.1 as f32).min(1.0);
    assert!(
        matches!(orthographic_camera.projection, orr_render::camera3d::Projection::Orthographic { half_height, .. } if (half_height - expected_half_height).abs() < 1.0e-4),
        "expected an applied 6-unit orthographic camera, got {:?}",
        orthographic_camera.projection
    );
    assert_eq!(
        orthographic_camera.target[0], 1.25,
        "the edited target field must reach the applied camera"
    );

    click_editor(&mut h, Role::Button, "Undo game camera");
    assert!(matches!(
        h.state()
            .editor
            .presentation_camera3d(viewport_size)
            .unwrap()
            .projection,
        orr_render::camera3d::Projection::Perspective { .. }
    ));
    click_editor(&mut h, Role::Button, "Redo game camera");
    click_editor(&mut h, Role::Button, "Save game camera");
    let sidecar = crate::mesh_camera_store::sidecar_path(&scene).unwrap();
    assert!(sidecar.is_file());
    let first_document = crate::mesh_camera_store::load(
        &scene,
        mesh_from_view(h.state().editor.snapshot().unwrap().predicted())
            .unwrap()
            .graph
            .revision(),
    )
    .unwrap()
    .unwrap()
    .0;
    assert!(matches!(
        first_document.projection,
        crate::mesh_camera::Projection::Orthographic { half_height }
            if (half_height - 6.0).abs() < 1.0e-4
    ));
    assert_eq!(first_document.target[0], 1.25);
    if let Some(evidence) = evidence {
        capture_editor(&mut h, evidence, "01-authored-orthographic-camera.png");
    }

    type_editor_field(&mut h, "Game camera target X", "2.5");
    click_editor(&mut h, Role::RadioButton, "Perspective game camera");
    type_editor_field(&mut h, "Game camera field of view (degrees)", "62");
    click_editor(&mut h, Role::Button, "Apply game camera");
    let perspective_camera = h
        .state()
        .editor
        .presentation_camera3d(viewport_size)
        .unwrap();
    assert!(
        matches!(perspective_camera.projection, orr_render::camera3d::Projection::Perspective { fov_y, .. } if (fov_y.to_degrees() - 62.0).abs() < 1.0e-3),
        "expected an applied 62-degree perspective camera, got {:?}",
        perspective_camera.projection
    );
    click_editor(&mut h, Role::Button, "Save game camera");
    let second_document = crate::mesh_camera_store::load(
        &scene,
        mesh_from_view(h.state().editor.snapshot().unwrap().predicted())
            .unwrap()
            .graph
            .revision(),
    )
    .unwrap()
    .unwrap()
    .0;
    assert_eq!(second_document.target[0], 2.5);
    assert!((second_document.fov_y_degrees - 62.0).abs() < 1.0e-4);
    assert!(matches!(
        second_document.projection,
        crate::mesh_camera::Projection::Perspective
    ));
    let authored_target = second_document.target;
    assert_eq!(
        h.state().editor.checksum(),
        simulation_checksum_before_camera_edits,
        "camera edits must not mutate simulation state"
    );
    if let Some(evidence) = evidence {
        capture_editor(&mut h, evidence, "02-authored-perspective-camera.png");
    }

    // Exercise the ordinary editor Stop control before packaging. The
    // camera-only edit must not change the authored simulation checksum.
    let source_checksum = h.state().editor.checksum();
    assert_eq!(source_checksum, simulation_checksum_before_camera_edits);
    click_editor(&mut h, Role::Button, crate::app::LBL_PLAY);
    let tick_deadline = Instant::now() + Duration::from_secs(5);
    while h.state().editor.sim().head_tick == 0 {
        h.step();
        h.state_mut().editor.sync();
        assert!(
            Instant::now() < tick_deadline,
            "EditorApp Play must advance before its Stop widget is exercised"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(h.state().editor.sim().head_tick > 0);
    click_editor(&mut h, Role::Button, crate::app::LBL_STOP);
    assert_eq!(h.state().editor.mode(), crate::model::Mode::Edit);
    assert_eq!(h.state().editor.checksum(), source_checksum);

    let platform = if cfg!(windows) {
        crate::mesh_hazards_export::RuntimePlatform::WindowsX86_64
    } else {
        crate::mesh_hazards_export::RuntimePlatform::LinuxX86_64
    };
    let source_runtime = directory.path().join(platform.runtime_filename());
    let runtime_sha256 = if rendered {
        let built_runtime = env_path("ORR_HAZARDS_CAMERA_PLAYER_RUNTIME");
        let expected_sha256 = std::env::var("ORR_HAZARDS_CAMERA_PLAYER_EXPECTED_RUNTIME_SHA256")
            .expect("set the independently built player runtime SHA-256");
        let built_sha256 = super::sha256_file_bounded(&built_runtime).unwrap();
        assert_eq!(built_sha256, expected_sha256);
        fs::copy(&built_runtime, &source_runtime).unwrap();
        assert_eq!(
            super::sha256_file_bounded(&source_runtime).unwrap(),
            expected_sha256,
            "copied runtime must remain byte-identical"
        );
        built_sha256
    } else {
        fs::write(
            &source_runtime,
            crate::mesh_hazards_export::test_camera_runtime(platform),
        )
        .unwrap();
        super::sha256_file_bounded(&source_runtime).unwrap()
    };
    let bundle_staging = directory.path().join("source-export-staging");
    let bundle_root = directory.path().join("relocated-four-file-bundle");
    let receipt =
        crate::mesh_hazards_export::export_interactive(crate::mesh_hazards_export::ExportRequest {
            scene: &scene,
            expected_checksum: source_checksum,
            destination: &bundle_staging,
            runtime: &source_runtime,
            trusted_runtime: true,
        })
        .unwrap();
    fs::rename(&bundle_staging, &bundle_root).unwrap();
    let bundle = crate::mesh_hazards_export::open_bundle(&bundle_root).unwrap();
    assert_eq!(bundle.camera_sha256, receipt.camera_sha256);
    assert!(bundle.camera_sha256.is_some());
    assert_eq!(bundle.runtime_sha256, runtime_sha256);
    let bundle_before = inventory(&bundle_root);
    assert_eq!(bundle_before.len(), 4);
    assert!(bundle_before.contains_key("game.scene.yaml.mesh-camera.json"));

    // Remove all test-owned project inputs at their original paths before
    // reopening the relocated bundle; the Player path below only reads its
    // four bundled files.
    drop(h);
    fs::remove_file(&scene).unwrap();
    fs::remove_file(&sidecar).unwrap();
    fs::remove_file(&source_runtime).unwrap();
    assert!(!scene.exists() && !sidecar.exists() && !source_runtime.exists());

    let mut reopened = Editor::open_game(&bundle.scene, EditorGame::MeshHazardsV1).unwrap();
    reopened.sync();
    let mut camera_panel = crate::mesh_camera_panel::Panel::default();
    camera_panel.sync(&mut reopened);
    verify_loaded_camera_bundle(&bundle_root, &bundle, &camera_panel).unwrap();
    assert_eq!(camera_panel.play_blocker(&reopened), None);
    assert!(matches!(
        reopened
            .presentation_camera3d((960, 600))
            .unwrap()
            .projection,
        orr_render::camera3d::Projection::Perspective { .. }
    ));
    let camera_blocker = camera_panel.play_blocker(&reopened);
    assert!(camera_blocker.is_none());
    reopened.set_mesh_navigation_blocker(camera_blocker);
    start_playback(&mut reopened, Some(50), false).unwrap();
    let camera_playback_checksum = reopened.snapshot().unwrap().predicted().checksum();
    assert_eq!(reopened.timeline().unwrap().tick, 50);
    assert!(reopened.stop().is_some());
    reopened.sync();
    assert_eq!(reopened.checksum(), source_checksum);

    let mut camera_free = Editor::open_game(&bundle.scene, EditorGame::MeshHazardsV1).unwrap();
    camera_free.sync();
    start_playback(&mut camera_free, Some(50), false).unwrap();
    let camera_free_checksum = camera_free.snapshot().unwrap().predicted().checksum();
    assert_eq!(camera_free.timeline().unwrap().tick, 50);
    assert_eq!(
        camera_playback_checksum, camera_free_checksum,
        "an authored presentation camera must preserve the deterministic 50-tick simulation checksum"
    );
    assert!(camera_free.stop().is_some());
    camera_free.sync();
    assert_eq!(camera_free.checksum(), source_checksum);

    start_playback(&mut reopened, Some(0), false).unwrap();
    assert_eq!(reopened.checksum(), source_checksum);
    assert!(matches!(
        reopened
            .presentation_camera3d((960, 600))
            .unwrap()
            .projection,
        orr_render::camera3d::Projection::Perspective { .. }
    ));

    let initial_checksum = reopened.checksum();
    let mut actual_adapter = None;
    let mut player_harness = if rendered {
        Harness::builder()
            .with_size([960.0, 600.0])
            .with_step_dt(1.0 / 60.0)
            .wgpu_setup(software_gpu_setup(backend.unwrap()).into())
            .build_eframe(|cc| {
                actual_adapter = Some(cc.wgpu_render_state.as_ref().unwrap().adapter.get_info());
                let mut inner = EditorApp::new(reopened, cc.wgpu_render_state.clone());
                inner.install_validated_hazards_camera(camera_panel);
                inner.set_mesh_player_input_allowed(true);
                PlayerApp {
                    inner,
                    session: Default::default(),
                }
            })
    } else {
        Harness::builder()
            .with_size([960.0, 600.0])
            .with_step_dt(1.0 / 60.0)
            .renderer(egui_kittest::LazyRenderer::Uninitialized {
                textures_delta: Default::default(),
                builder: None,
            })
            .build_eframe(move |_| {
                let mut inner = EditorApp::new(reopened, None);
                inner.install_validated_hazards_camera(camera_panel);
                inner.set_mesh_player_input_allowed(true);
                PlayerApp {
                    inner,
                    session: Default::default(),
                }
            })
    };
    settle(&mut player_harness);
    if rendered {
        // Slow only this software-rendered synthetic-input test so the click
        // event is delivered before the deterministic mission reaches an end.
        player_harness.state_mut().inner.editor.set_speed(0.25);
        player_harness.state_mut().inner.editor.sync();
        player_harness.step();
    }
    if let Some(evidence) = evidence {
        capture(
            &mut player_harness,
            evidence,
            "03-relocated-player-tick-zero.png",
        );
    }
    assert_eq!(
        player_harness
            .state()
            .inner
            .editor
            .presentation_camera3d((960, 600))
            .unwrap()
            .target,
        authored_target
    );
    crate::app::mesh_hazards_player::validate_fresh_session(&player_harness.state().inner.editor)
        .unwrap();

    click(&mut player_harness, "Resume");
    click(&mut player_harness, "Enable mesh goal input");
    wait_for(&mut player_harness, "managed input enabled", |app| {
        app.inner.editor.mesh_play_input_hint() == "Managed player input active"
    });
    let hazard_center = hazards::from_view(
        player_harness
            .state()
            .inner
            .editor
            .snapshot()
            .unwrap()
            .predicted(),
    )
    .unwrap()
    .unwrap()
    .config
    .spheres[0]
        .center;
    let projected_hazard_goal = click_world(
        &mut player_harness,
        hazard_center.map(|coordinate| coordinate.to_f32()),
    );
    for axis in 0..3 {
        assert!(
            projected_hazard_goal[axis]
                .raw()
                .abs_diff(hazard_center[axis].raw())
                <= (FP::SCALE / 512) as u64,
            "projected hazard center should pick the expected authoritative point: {:?} vs {:?}",
            projected_hazard_goal,
            hazard_center
        );
    }
    wait_for_authoritative_goal(
        &mut player_harness,
        hazard_center,
        FP::SCALE / 512,
        "hazard goal",
    );
    wait_for(&mut player_harness, "projected hazard loss", lost_app);
    if player_harness.state().inner.editor.sim().playing {
        click(&mut player_harness, "Pause");
    }
    let loss_tick = player_harness.state().inner.editor.sim().head_tick;
    assert!(loss_tick > 0);
    if let Some(evidence) = evidence {
        capture(
            &mut player_harness,
            evidence,
            "04-relocated-player-hazard-loss.png",
        );
    }
    click(&mut player_harness, "Restart game…");
    click(&mut player_harness, "Restart game");
    crate::app::mesh_hazards_player::validate_fresh_session(&player_harness.state().inner.editor)
        .unwrap();
    assert_eq!(
        player_harness.state().inner.editor.checksum(),
        initial_checksum
    );
    assert_eq!(
        player_harness
            .state()
            .inner
            .editor
            .presentation_camera3d((960, 600))
            .unwrap()
            .target,
        authored_target,
        "confirmed Restart must retain the already admitted authored camera"
    );
    assert_eq!(
        player_harness
            .state()
            .inner
            .editor
            .last_stopped()
            .unwrap()
            .tick,
        loss_tick
    );
    if let Some(evidence) = evidence {
        capture(
            &mut player_harness,
            evidence,
            "05-relocated-player-restart.png",
        );
    }

    click(&mut player_harness, "Resume");
    click(&mut player_harness, "Enable mesh goal input");
    wait_for(&mut player_harness, "managed input reenabled", |app| {
        app.inner.editor.mesh_play_input_hint() == "Managed player input active"
    });
    let exit = mission::from_view(
        player_harness
            .state()
            .inner
            .editor
            .snapshot()
            .unwrap()
            .predicted(),
    )
    .unwrap()
    .unwrap()
    .config
    .exit
    .position;
    let exit_goal = click_world(
        &mut player_harness,
        exit.map(|coordinate| coordinate.to_f32()),
    );
    wait_for_authoritative_goal(
        &mut player_harness,
        exit_goal,
        FP::SCALE / 512,
        "mission exit goal",
    );
    wait_for(
        &mut player_harness,
        "projected authoritative mission win",
        |app| {
            mission::from_view(app.inner.editor.snapshot().unwrap().predicted())
                .unwrap()
                .unwrap()
                .run
                .won
                != 0
        },
    );
    if player_harness.state().inner.editor.sim().playing {
        click(&mut player_harness, "Pause");
    }
    assert_eq!(
        mission::from_view(
            player_harness
                .state()
                .inner
                .editor
                .snapshot()
                .unwrap()
                .predicted()
        )
        .unwrap()
        .unwrap()
        .run
        .won,
        1
    );
    let win_tick = player_harness.state().inner.editor.sim().head_tick;
    let win_checksum = player_harness.state().inner.editor.checksum();
    if let Some(evidence) = evidence {
        capture(
            &mut player_harness,
            evidence,
            "06-relocated-player-mission-win.png",
        );
    }
    // The Player exposes one explicit Restart action, which performs a
    // confirmed authoritative Stop before starting the fresh session.
    click(&mut player_harness, "Restart game…");
    click(&mut player_harness, "Restart game");
    crate::app::mesh_hazards_player::validate_fresh_session(&player_harness.state().inner.editor)
        .unwrap();
    assert_eq!(
        player_harness.state().inner.editor.mode(),
        crate::model::Mode::Play
    );
    assert_eq!(
        player_harness.state().inner.editor.checksum(),
        initial_checksum
    );
    assert!(!lost(&player_harness));
    assert_eq!(
        mission::from_view(
            player_harness
                .state()
                .inner
                .editor
                .snapshot()
                .unwrap()
                .predicted()
        )
        .unwrap()
        .unwrap()
        .run,
        mission::MeshMissionRun::default()
    );
    assert_eq!(inventory(&bundle_root), bundle_before);
    if let Some(evidence) = evidence {
        capture(
            &mut player_harness,
            evidence,
            "07-relocated-player-win-restart.png",
        );
        let adapter = actual_adapter.expect("rendered Player creation records actual adapter");
        assert_eq!(Some(adapter.backend), backend);
        assert_eq!(adapter.device_type, wgpu::DeviceType::Cpu);
        let receipt = serde_json::json!({
            "schema": 1,
            "ui": "production EditorApp + PlayerApp via headless egui_kittest software GPU",
            "native_window": false,
            "physical_input": false,
            "hardware_gpu": false,
            "standalone_executable_ui": false,
            "standalone_game_process_launched": false,
            "runtime_fixture": !rendered,
            "adapter": adapter.name,
            "backend": format!("{:?}", adapter.backend),
            "device_type": format!("{:?}", adapter.device_type),
            "driver": adapter.driver,
            "driver_info": adapter.driver_info,
            "bundle_root": bundle_root,
            "four_file_bundle_hashes_before_and_after": bundle_before,
            "camera_sha256": receipt.camera_sha256,
            "runtime_sha256": receipt.runtime_sha256,
            "copied_built_runtime_sha256": runtime_sha256,
            "source_scene_camera_runtime_removed": true,
            "authored_orthographic_and_perspective": true,
            "synthetic_projected_goal_authoritative": true,
            "editor_stop_restores_authored_checksum": true,
            "camera_vs_no_camera_50_tick_checksum": camera_playback_checksum,
            "camera_free_50_tick_checksum": camera_free_checksum,
            "loss_tick": loss_tick,
            "win_tick": win_tick,
            "win_checksum": win_checksum,
            "restart_resets_authored_checksum": true,
            "test_only_rendered_speed_permille": 250
        });
        fs::write(
            evidence.join("receipt.json"),
            serde_json::to_vec_pretty(&receipt).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn player_keeps_admitted_camera_when_sidecar_disappears_before_first_ui_frame() {
    let directory = tempfile::tempdir().expect("isolated Player camera rebind test");
    let scene = directory.path().join("bound-camera.scene.yaml");
    let mut editor = crate::mesh_hazards_creator::prepare(
        &scene,
        &crate::mesh_workspace::mission::Settings::default(),
        true,
    )
    .unwrap();
    let graph_revision = mesh_from_view(editor.snapshot().unwrap().predicted())
        .unwrap()
        .graph
        .revision();
    let camera = crate::mesh_camera::Document::from_orbit(orr_render::OrbitCamera::new(
        [1.25, 2.0, 0.0],
        0.7,
        0.4,
        14.0,
    ))
    .unwrap();
    let sidecar = crate::mesh_camera_store::sidecar_path(&scene).unwrap();
    let camera_sha = crate::mesh_camera_store::save(&scene, Some(&camera), None, graph_revision)
        .unwrap()
        .unwrap();
    let initial_checksum = editor.checksum();
    let mut camera_panel = crate::mesh_camera_panel::Panel::default();
    camera_panel.sync(&mut editor);
    assert!(camera_panel.matches_saved_sidecar(Some(&camera_sha)));
    let camera_blocker = camera_panel.play_blocker(&editor);
    editor.set_mesh_navigation_blocker(camera_blocker);
    start_playback(&mut editor, Some(0), false).unwrap();

    let mut inner = EditorApp::new(editor, None);
    inner.install_validated_hazards_camera(camera_panel);
    inner.set_mesh_player_input_allowed(true);
    fs::remove_file(&sidecar).expect("remove only after camera panel binding");
    let player = PlayerApp {
        inner,
        session: Default::default(),
    };
    let mut h = Harness::builder()
        .with_size([960.0, 600.0])
        .with_step_dt(1.0 / 60.0)
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(move |_| player);
    settle(&mut h);
    assert_eq!(h.state().inner.editor.checksum(), initial_checksum);
    assert_eq!(
        h.state()
            .inner
            .editor
            .presentation_camera3d((960, 600))
            .unwrap()
            .target,
        camera.target
    );
    click(&mut h, "Resume");
    wait_for(&mut h, "Player session advances before Restart", |app| {
        app.inner.editor.sim().playing && app.inner.editor.sim().head_tick > 0
    });
    click(&mut h, "Restart game…");
    click(&mut h, "Restart game");
    crate::app::mesh_hazards_player::validate_fresh_session(&h.state().inner.editor).unwrap();
    assert_eq!(h.state().inner.editor.checksum(), initial_checksum);
    assert_eq!(
        h.state()
            .inner
            .editor
            .presentation_camera3d((960, 600))
            .unwrap()
            .target,
        camera.target,
        "Restart must not reload or erase the admitted authored camera"
    );
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
