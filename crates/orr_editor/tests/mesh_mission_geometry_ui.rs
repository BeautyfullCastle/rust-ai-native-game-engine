#![cfg(all(
    windows,
    target_arch = "x86_64",
    feature = "mesh-mission",
    feature = "mesh-export"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::{kittest::Queryable, Harness};
use egui_wgpu::{wgpu, WgpuSetupCreateNew};
use orr_editor::{
    app::{LBL_PAUSE, LBL_PLAY, LBL_STEP, LBL_STOP},
    game::EditorGame,
    Editor, EditorApp, Mode,
};
use orr_fp::FP;
use orr_navigation::NavigationStatus;
use orr_remote::mesh_navigation::{
    mesh_from_view,
    mission::{self, MissionView},
};
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

struct Fixture {
    root: PathBuf,
    scene: PathBuf,
    physics_scene: PathBuf,
    physics_bytes: Vec<u8>,
    runtime: PathBuf,
    export: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        assert_eq!(
            std::env::var("ORR_MESH_GEOMETRY_ACCEPTANCE").as_deref(),
            Ok("1")
        );
        assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
        assert_eq!(std::env::var("WGPU_BACKEND").unwrap_or_default(), "vulkan");
        let root = PathBuf::from(
            std::env::var_os("ORR_MESH_GEOMETRY_CAPTURE_DIR")
                .expect("set a fresh mission-geometry capture root"),
        );
        assert!(root.is_absolute());
        fs::create_dir(&root).expect("capture root must be fresh");
        let root = root.canonicalize().unwrap();
        let captures = root.join("captures");
        fs::create_dir(&captures).unwrap();
        let scene = PathBuf::from(
            std::env::var_os("ORR_MESH_GEOMETRY_OUTPUT")
                .unwrap_or_else(|| root.join("mission-geometry.scene.yaml").into_os_string()),
        );
        assert!(scene.is_absolute() && !scene.exists());
        let scene_parent = scene.parent().unwrap().canonicalize().unwrap();
        let scene = scene_parent.join(scene.file_name().unwrap());
        let physics_scene = root.join("owned-stock-physics.scene.yaml");
        fs::copy(EditorGame::PhysGame.default_scene_path(), &physics_scene).unwrap();
        let physics_bytes = fs::read(&physics_scene).unwrap();
        let runtime = PathBuf::from(
            std::env::var_os("ORR_MESH_EXPORT_RUNTIME")
                .expect("mission-geometry acceptance requires the matching mesh player"),
        );
        assert!(runtime.is_absolute() && runtime.is_file());
        let export = PathBuf::from(
            std::env::var_os("ORR_MESH_GEOMETRY_EXPORT_OUTPUT")
                .expect("mission-geometry acceptance requires a fresh export directory"),
        );
        assert!(export.is_absolute() && !export.exists());
        assert_eq!(
            export.parent().unwrap().canonicalize().unwrap(),
            root.parent().unwrap()
        );
        Self {
            root,
            scene,
            physics_scene,
            physics_bytes,
            runtime,
            export,
        }
    }

    fn harness(&self) -> Harness<'static, EditorApp> {
        let mut editor = Editor::open_game(&self.physics_scene, EditorGame::PhysGame).unwrap();
        editor.sync();
        Harness::builder()
            .with_size([1500.0, 1000.0])
            .with_step_dt(1.0 / 60.0)
            .wgpu_setup(hardware_vulkan_setup())
            .build_eframe(move |context| EditorApp::new(editor, context.wgpu_render_state.clone()))
    }
}

fn hardware_vulkan_setup() -> egui_wgpu::WgpuSetup {
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    setup.native_adapter_selector = Some(Arc::new(|adapters, _surface| {
        adapters
            .iter()
            .find(|adapter| {
                let info = adapter.get_info();
                info.backend == wgpu::Backend::Vulkan
                    && matches!(
                        info.device_type,
                        wgpu::DeviceType::DiscreteGpu | wgpu::DeviceType::IntegratedGpu
                    )
            })
            .cloned()
            .ok_or_else(|| "No hardware Vulkan adapter is available".to_owned())
    }));
    setup.into()
}

fn gpu_available() -> bool {
    match Wgpu::headless(WgpuOptions::default()) {
        Ok(rhi) => {
            eprintln!(
                "Mesh mission geometry GPU preflight: {} (software: {})",
                rhi.adapter_name(),
                rhi.is_software()
            );
            assert!(
                !rhi.is_software(),
                "software rendering is not GPU acceptance"
            );
            true
        }
        Err(error) => {
            assert!(
                std::env::var_os("ORR_MESH_GEOMETRY_ACCEPTANCE").is_none()
                    && std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "required mission-geometry GPU acceptance unavailable: {error}"
            );
            eprintln!("SKIP: mission-geometry GPU unavailable: {error}");
            false
        }
    }
}

fn settle(harness: &mut Harness<'_, EditorApp>) {
    for _ in 0..4 {
        harness.state_mut().editor.sync();
        harness.run_steps(2);
    }
}

fn click(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness.get_by_label(label).scroll_to_me();
    harness.run_steps(30);
    harness.get_by_label(label).click_accesskit();
    settle(harness);
}

fn click_button(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness
        .get_by_role_and_label(Role::Button, label)
        .scroll_to_me();
    harness.run_steps(30);
    harness
        .get_by_role_and_label(Role::Button, label)
        .click_accesskit();
    settle(harness);
}

fn click_transport(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness
        .get_by_role_and_label(Role::Button, label)
        .click_accesskit();
    harness.run_steps(1);
    harness.state_mut().editor.sync();
}

fn type_field(harness: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    harness.run_steps(30);
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .focus();
    harness.run_steps(2);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    settle(harness);
}

fn menu_click(harness: &mut Harness<'_, EditorApp>, menu: &str, item: &str) {
    if harness.query_by_label(item).is_none() {
        click(harness, menu);
    }
    harness.get_by_label(item).scroll_to_me();
    harness.run_steps(30);
    harness.get_by_label(item).click_accesskit();
    settle(harness);
}

fn wait_for(
    harness: &mut Harness<'_, EditorApp>,
    operation: &str,
    ready: impl Fn(&EditorApp) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        settle(harness);
        if ready(harness.state()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {operation}: game={:?}, path={:?}, checksum={}, status={:?}",
            harness.state().editor.game(),
            harness.state().editor.path(),
            harness.state().editor.checksum(),
            harness.state().editor.status()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn mission_view(app: &EditorApp) -> MissionView {
    mission::from_view(app.editor.snapshot().unwrap().predicted())
        .unwrap()
        .expect("created mesh game has an active mission")
}

fn mesh_status(app: &EditorApp) -> NavigationStatus {
    mesh_from_view(app.editor.snapshot().unwrap().predicted())
        .unwrap()
        .agent
        .status
}

fn viewport_pixels(harness: &Harness<'_, EditorApp>) -> ((u32, u32), Vec<u8>) {
    let viewport = harness
        .state()
        .viewport3d_gpu()
        .expect("production App viewport GPU is initialized");
    let size = viewport.gpu().target().size();
    let pixels = viewport.gpu().read_rgba8();
    assert_eq!(pixels.len(), (size.0 * size.1 * 4) as usize);
    (size, pixels)
}

fn normalize_viewport_panels(harness: &mut Harness<'_, EditorApp>) {
    for (panel_name, width) in [("inspector", 340.0), ("hierarchy", 230.0)] {
        let identifier = egui::Id::new(panel_name);
        let mut state = egui::containers::panel::PanelState::load(&harness.ctx, identifier)
            .expect("rendered production panel state");
        if panel_name == "inspector" {
            state.outer_rect.min.x = state.outer_rect.max.x - width;
        } else {
            state.outer_rect.max.x = state.outer_rect.min.x + width;
        }
        harness
            .ctx
            .data_mut(|data| data.insert_persisted(identifier, state));
    }
    harness.run_steps(30);
    settle(harness);
}

fn capture(harness: &mut Harness<'_, EditorApp>, root: &Path, name: &str) -> Vec<u8> {
    settle(harness);
    let frame = harness.render().expect("render the production editor");
    let pixels = frame.as_raw().clone();
    let image = egui::ColorImage::from_rgba_unmultiplied(
        [frame.width() as usize, frame.height() as usize],
        &pixels,
    );
    orr_editor::app::write_png(&root.join("captures").join(name), &image).unwrap();
    pixels
}

fn changed_pixel_count(left: &[u8], right: &[u8]) -> usize {
    assert_eq!(left.len(), right.len());
    left.as_chunks::<4>().0.iter()
        .zip(right.as_chunks::<4>().0.iter())
        .filter(|(left_pixel, right_pixel)| {
            left_pixel[..3]
                .iter()
                .zip(right_pixel[..3].iter())
                .any(|(left_channel, right_channel)| left_channel.abs_diff(*right_channel) > 12)
        })
        .count()
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn capture_record(root: &Path, name: &str) -> Value {
    let bytes = fs::read(root.join("captures").join(name)).unwrap();
    serde_json::json!({"name":name,"bytes":bytes.len(),"sha256":sha256(&bytes)})
}

fn advance_until_won(harness: &mut Harness<'_, EditorApp>) -> (u64, u64) {
    click_transport(harness, LBL_PLAY);
    wait_for(harness, "mission geometry Play", |app| {
        app.editor
            .timeline()
            .is_some_and(|timeline| timeline.playing)
    });
    click_transport(harness, LBL_PAUSE);
    if harness
        .state()
        .editor
        .timeline()
        .is_some_and(|timeline| timeline.tick != 0)
    {
        click(harness, "⏮");
    }
    assert!(harness
        .state()
        .editor
        .timeline()
        .is_some_and(|timeline| timeline.tick == 0 && !timeline.playing));
    let mut previous_tick = 0;
    for _ in 0..2048 {
        let mission_state = mission_view(harness.state());
        if mission_state.run.won == 1 && mesh_status(harness.state()) == NavigationStatus::Arrived {
            let checksum = harness
                .state()
                .editor
                .snapshot()
                .unwrap()
                .predicted()
                .checksum();
            let tick = harness.state().editor.timeline().unwrap().tick;
            assert!(tick > previous_tick);
            return (tick, checksum);
        }
        previous_tick = harness.state().editor.timeline().unwrap().tick;
        click_transport(harness, LBL_STEP);
        assert_eq!(
            harness.state().editor.timeline().unwrap().tick,
            previous_tick + 1,
            "each UI Step advances one deterministic tick"
        );
        let progress = mission_view(harness.state());
        let all_collected = (1 << progress.config.count) - 1;
        if progress.run.collected != all_collected {
            assert_eq!(progress.run.won, 0, "mission cannot win before collection");
        }
    }
    panic!("reprojected mission did not reach the goal within 2048 ticks")
}

#[test]
#[ignore = "requires ORR_MESH_GEOMETRY_ACCEPTANCE=1 and a hardware Vulkan adapter"]
fn mission_geometry_reprojects_and_preserves_playable_mission() {
    if std::env::var_os("ORR_MESH_GEOMETRY_ACCEPTANCE").is_none()
        && std::env::var_os("ORR_REQUIRE_GPU").is_none()
    {
        eprintln!("SKIP: opt in to the actual mission-geometry GPU acceptance");
        return;
    }
    assert_eq!(
        std::env::var("ORR_MESH_GEOMETRY_ACCEPTANCE").as_deref(),
        Ok("1")
    );
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    assert_eq!(std::env::var("WGPU_BACKEND").unwrap_or_default(), "vulkan");
    assert!(gpu_available());

    let root = PathBuf::from(
        std::env::var_os("ORR_MESH_GEOMETRY_CAPTURE_DIR")
            .expect("set a fresh mission-geometry capture root"),
    );
    let fixture = Fixture::new();
    assert_eq!(fixture.root, root.canonicalize().unwrap());
    let mut harness = fixture.harness();
    harness.run_steps(4);
    assert!(harness.state().has_gpu());

    menu_click(&mut harness, "File", "New mesh game…");
    type_field(
        &mut harness,
        "New mesh game scene path",
        &fixture.scene.display().to_string(),
    );
    type_field(&mut harness, "Game floor width", "12");
    type_field(&mut harness, "Game floor depth", "8");
    type_field(&mut harness, "Game collectible count", "3");
    type_field(&mut harness, "Game distance per tick", "0.25");
    click_button(&mut harness, "Create mesh game");
    wait_for(&mut harness, "created mission scene", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && app.editor.path().as_deref().is_some_and(|path| {
                path == fixture.scene.as_path()
                    || path.canonicalize().ok() == fixture.scene.canonicalize().ok()
            })
            && fixture.scene.is_file()
            && mission::from_view(app.editor.snapshot().unwrap().predicted())
                .ok()
                .flatten()
                .is_some()
    });

    if harness.query_by_label("Move vertex index").is_none() {
        click(&mut harness, "Static-mesh navigation");
    }
    let initial_snapshot = harness.state().editor.snapshot().unwrap();
    let initial_frame = initial_snapshot.predicted();
    let initial_checksum = initial_frame.checksum();
    let initial_history_len = harness.state().editor.history().entries.len();
    let initial_mesh = mesh_from_view(initial_frame).unwrap();
    let initial_mission = mission_view(harness.state());
    assert_eq!(initial_mission.config.count, 3);
    assert_eq!(initial_mission.run.collected, 0);
    assert_eq!(initial_mission.run.won, 0);
    let initial_bytes = fs::read(&fixture.scene).unwrap();
    normalize_viewport_panels(&mut harness);
    let (initial_viewport_size, initial_viewport) = viewport_pixels(&harness);
    let _ = capture(&mut harness, &fixture.root, "initial-mission.png");

    click(
        &mut harness,
        "Preserve and reproject the active mesh mission",
    );
    type_field(&mut harness, "Maximum vertical projection", "1");
    for (label, value) in [
        ("Move vertex index", "3"),
        ("Move vertex X", "12"),
        ("Move vertex Y", "1"),
        ("Move vertex Z", "8"),
    ] {
        type_field(&mut harness, label, value);
    }
    click_button(&mut harness, "Stage vertex move");
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        initial_history_len
    );
    assert_eq!(
        mesh_from_view(harness.state().editor.snapshot().unwrap().predicted())
            .unwrap()
            .graph
            .revision(),
        initial_mesh.graph.revision()
    );
    normalize_viewport_panels(&mut harness);
    let (staged_viewport_size, staged_viewport) = viewport_pixels(&harness);
    assert_eq!(staged_viewport_size, initial_viewport_size);
    let staged_pixel_diff = changed_pixel_count(&initial_viewport, &staged_viewport);
    assert!(
        staged_pixel_diff > 20,
        "staged vertex geometry did not visibly differ: {staged_pixel_diff}"
    );

    type_field(&mut harness, "Maximum vertical projection", "0");
    click_button(&mut harness, "Preview mission-preserving geometry");
    assert!(harness
        .query_by_label_contains("Mission geometry preview failed:")
        .is_some());
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        initial_history_len
    );
    type_field(&mut harness, "Maximum vertical projection", "1");
    click_button(&mut harness, "Preview mission-preserving geometry");
    assert!(harness
        .query_by_label("Mission reprojection preview is ready; the host is unchanged.")
        .is_some());
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        initial_history_len
    );
    let _ = capture(&mut harness, &fixture.root, "mission-preview.png");
    click_button(&mut harness, "Cancel mission geometry preview");
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        initial_history_len
    );
    assert!(harness
        .query_by_label("Mission reprojection preview is ready; the host is unchanged.")
        .is_none());

    click_button(&mut harness, "Preview mission-preserving geometry");
    assert!(harness.query_by_label_contains("Collectible 1:").is_some());
    click(
        &mut harness,
        "I confirm reprojecting the agent and active mission onto this staged graph",
    );
    click_button(&mut harness, "Apply geometry and reproject mission");
    wait_for(&mut harness, "atomic geometry and mission commit", |app| {
        app.editor.checksum() != initial_checksum
            && mesh_from_view(app.editor.snapshot().unwrap().predicted())
                .is_ok_and(|view| view.graph.revision() != initial_mesh.graph.revision())
    });
    let applied_snapshot = harness.state().editor.snapshot().unwrap();
    let applied_frame = applied_snapshot.predicted();
    let applied_checksum = applied_frame.checksum();
    let applied_history_len = harness.state().editor.history().entries.len();
    let applied_mesh = mesh_from_view(applied_frame).unwrap();
    let applied_mission = mission_view(harness.state());
    assert_eq!(applied_history_len, initial_history_len + 1);
    assert_ne!(applied_mesh.graph.revision(), initial_mesh.graph.revision());
    assert_eq!(applied_mesh.graph.vertices()[3][1], FP::ONE);
    assert_eq!(applied_mesh.agent.spec.max_vertical, FP::ONE);
    assert_eq!(
        applied_mesh.agent.spec.distance_per_tick,
        initial_mesh.agent.spec.distance_per_tick
    );
    assert_eq!(
        applied_mesh.agent.spec.start[0],
        initial_mesh.agent.spec.start[0]
    );
    assert_eq!(
        applied_mesh.agent.spec.start[2],
        initial_mesh.agent.spec.start[2]
    );
    assert_eq!(
        applied_mesh.agent.spec.goal[0],
        initial_mesh.agent.spec.goal[0]
    );
    assert_eq!(
        applied_mesh.agent.spec.goal[2],
        initial_mesh.agent.spec.goal[2]
    );
    assert!(applied_mesh.agent.spec.start[1] > initial_mesh.agent.spec.start[1]);
    assert!(applied_mesh.agent.spec.goal[1] > initial_mesh.agent.spec.goal[1]);
    assert_eq!(
        applied_mission.config.enabled,
        initial_mission.config.enabled
    );
    assert_eq!(applied_mission.config.count, initial_mission.config.count);
    assert_eq!(
        applied_mission.config.graph_revision,
        applied_mesh.graph.revision()
    );
    assert_eq!(applied_mission.run, initial_mission.run);
    for (old_point, new_point) in initial_mission
        .config
        .points
        .iter()
        .zip(applied_mission.config.points.iter())
        .take(initial_mission.config.count as usize)
    {
        assert_eq!(old_point.position[0], new_point.position[0]);
        assert_eq!(old_point.position[2], new_point.position[2]);
        assert!(new_point.position[1] > old_point.position[1]);
        assert_eq!(old_point.radius, new_point.radius);
    }
    assert_eq!(
        applied_mission.config.exit.position[0],
        initial_mission.config.exit.position[0]
    );
    assert_eq!(
        applied_mission.config.exit.position[2],
        initial_mission.config.exit.position[2]
    );
    assert!(applied_mission.config.exit.position[1] > initial_mission.config.exit.position[1]);
    assert_eq!(
        applied_mission.config.exit.radius,
        initial_mission.config.exit.radius
    );
    harness.state_mut().editor.fit_camera();
    normalize_viewport_panels(&mut harness);
    let _ = capture(&mut harness, &fixture.root, "mission-reprojected.png");
    let (applied_viewport_size, applied_viewport) = viewport_pixels(&harness);
    assert_eq!(applied_viewport_size, initial_viewport_size);
    let applied_pixel_diff = changed_pixel_count(&initial_viewport, &applied_viewport);
    assert!(
        applied_pixel_diff > 20,
        "reprojected mission was not visible: {applied_pixel_diff}"
    );
    menu_click(&mut harness, "Edit", "Undo Ctrl+Z");
    wait_for(&mut harness, "mission geometry undo", |app| {
        app.editor.checksum() == initial_checksum
    });
    assert_eq!(
        mesh_from_view(harness.state().editor.snapshot().unwrap().predicted())
            .unwrap()
            .graph
            .revision(),
        initial_mesh.graph.revision()
    );
    assert_eq!(mission_view(harness.state()).config, initial_mission.config);
    menu_click(&mut harness, "Edit", "Redo Ctrl+Y");
    wait_for(&mut harness, "mission geometry redo", |app| {
        app.editor.checksum() == applied_checksum
    });
    assert_eq!(mission_view(harness.state()).config, applied_mission.config);

    menu_click(&mut harness, "File", "Save Ctrl+S");
    let saved_bytes = fs::read(&fixture.scene).unwrap();
    let saved_checksum = harness.state().editor.checksum();
    assert_ne!(saved_bytes, initial_bytes);
    drop(harness);

    let mut reopened = fixture.harness();
    reopened.run_steps(4);
    menu_click(&mut reopened, "File", "Open mesh scene…");
    type_field(
        &mut reopened,
        "Scene path",
        &fixture.scene.display().to_string(),
    );
    click_button(&mut reopened, "OK");
    wait_for(
        &mut reopened,
        "mission-preserving geometry fresh reopen",
        |app| {
            app.editor.game() == EditorGame::MeshNavigation
                && app.editor.checksum() == saved_checksum
                && app.editor.path().as_deref().is_some_and(|path| {
                    path == fixture.scene.as_path()
                        || path.canonicalize().ok() == fixture.scene.canonicalize().ok()
                })
        },
    );
    assert_eq!(fs::read(&fixture.scene).unwrap(), saved_bytes);
    let reopened_mesh =
        mesh_from_view(reopened.state().editor.snapshot().unwrap().predicted()).unwrap();
    let reopened_mission = mission_view(reopened.state());
    assert_eq!(
        reopened_mesh.graph.revision(),
        applied_mesh.graph.revision()
    );
    assert_eq!(reopened_mesh.agent.spec, applied_mesh.agent.spec);
    assert_eq!(reopened_mission.config, applied_mission.config);
    assert_eq!(reopened_mission.run, initial_mission.run);
    reopened.state_mut().editor.fit_camera();
    normalize_viewport_panels(&mut reopened);
    let _ = capture(&mut reopened, &fixture.root, "fresh-reopen.png");
    let (reopened_viewport_size, reopened_viewport) = viewport_pixels(&reopened);
    assert_eq!(reopened_viewport_size, applied_viewport_size);
    let reopen_pixel_diff = changed_pixel_count(&applied_viewport, &reopened_viewport);
    assert!(
        reopen_pixel_diff < 20,
        "reopened reprojected world pixels differ: {reopen_pixel_diff}"
    );
    let (win_tick, win_checksum) = advance_until_won(&mut reopened);
    let won = mission_view(reopened.state());
    assert_eq!(won.run.collected, (1 << won.config.count) - 1);
    assert_eq!(won.run.won, 1);
    assert_eq!(mesh_status(reopened.state()), NavigationStatus::Arrived);
    assert!(reopened.query_by_label("Game mission complete").is_some());
    assert!(!reopened.state().editor.timeline().unwrap().playing);
    let _ = capture(&mut reopened, &fixture.root, "reprojected-mission-won.png");
    click_transport(&mut reopened, LBL_STOP);
    assert_eq!(reopened.state().editor.mode(), Mode::Edit);
    assert_eq!(reopened.state().editor.checksum(), saved_checksum);
    assert_eq!(mission_view(reopened.state()).run, initial_mission.run);
    let (replay_tick, replay_checksum) = advance_until_won(&mut reopened);
    assert_eq!(replay_tick, win_tick);
    assert_eq!(
        replay_checksum, win_checksum,
        "same saved mission must replay to the identical full Frame"
    );
    click_transport(&mut reopened, LBL_STOP);
    assert_eq!(reopened.state().editor.checksum(), saved_checksum);

    menu_click(&mut reopened, "File", "Export mesh game…");
    type_field(
        &mut reopened,
        "Mesh player executable",
        &fixture.runtime.display().to_string(),
    );
    type_field(
        &mut reopened,
        "Mesh game output directory",
        &fixture.export.display().to_string(),
    );
    click(&mut reopened, "I trust this local mesh player executable");
    click_button(&mut reopened, "Export mesh game");
    let export_deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < export_deadline
        && reopened
            .query_by_label_contains("Mesh export complete:")
            .is_none()
    {
        settle(&mut reopened);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(reopened
        .query_by_label_contains("Mesh export complete:")
        .is_some());
    let scene_name = fixture
        .scene
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let mut export_names = fs::read_dir(&fixture.export)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    export_names.sort();
    let mut expected_names = vec![
        scene_name.clone(),
        "orr.mesh-bundle.json".into(),
        "orr_mesh_player.exe".into(),
    ];
    expected_names.sort();
    assert_eq!(export_names, expected_names);
    let manifest_bytes = fs::read(fixture.export.join("orr.mesh-bundle.json")).unwrap();
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let runtime_bytes = fs::read(fixture.export.join("orr_mesh_player.exe")).unwrap();
    let requested_runtime_bytes = fs::read(&fixture.runtime).unwrap();
    assert_eq!(
        fs::read(fixture.export.join(&scene_name)).unwrap(),
        saved_bytes
    );
    assert_eq!(manifest["scene"]["sha256"], sha256(&saved_bytes));
    assert_eq!(manifest["runtime"]["sha256"], sha256(&runtime_bytes));
    assert_eq!(runtime_bytes, requested_runtime_bytes);
    assert_eq!(manifest["input_profile"], "mesh-click-to-move-v1");
    assert_eq!(manifest["game_profile"], "mesh-mission-v1");
    assert_eq!(manifest["checksum"], saved_checksum);
    assert_eq!(
        manifest["graph_revision"],
        serde_json::json!(applied_mesh.graph.revision())
    );

    assert_eq!(
        fs::read(&fixture.physics_scene).unwrap(),
        fixture.physics_bytes
    );
    let captures = [
        "initial-mission.png",
        "mission-preview.png",
        "mission-reprojected.png",
        "fresh-reopen.png",
        "reprojected-mission-won.png",
    ]
    .iter()
    .map(|name| capture_record(&fixture.root, name))
    .collect::<Vec<_>>();
    let receipt = serde_json::json!({
        "artifact_root":fixture.root,
        "scene_path":fixture.scene,
        "scene_sha256":sha256(&saved_bytes),
        "scene_checksum":format!("0x{saved_checksum:016x}"),
        "physics_source_path":fixture.physics_scene,
        "physics_source_sha256":sha256(&fixture.physics_bytes),
        "physics_source_unchanged":fs::read(&fixture.physics_scene).unwrap() == fixture.physics_bytes,
        "floor_width":12,
        "floor_depth":8,
        "collectible_count":3,
        "distance_per_tick":"0.25",
        "source_graph_revision":initial_mesh.graph.revision().iter().map(|byte|format!("{byte:02x}")).collect::<String>(),
        "reprojected_graph_revision":applied_mesh.graph.revision().iter().map(|byte|format!("{byte:02x}")).collect::<String>(),
        "preview_cancel_preserved_host":true,
        "invalid_tolerance_rejected_without_host_mutation":true,
        "apply_created_one_undo_entry":applied_history_len == initial_history_len + 1,
        "undo_redo_checksums_verified":true,
        "all_mission_positions_reprojected":applied_mission.config.points.iter().take(applied_mission.config.count as usize).all(|point| point.position[1] > FP::ZERO) && applied_mission.config.exit.position[1] > FP::ZERO,
        "mission_identity_preserved":applied_mission.config.count == initial_mission.config.count && applied_mission.config.points.iter().zip(initial_mission.config.points.iter()).take(initial_mission.config.count as usize).all(|(new_point, old_point)| new_point.position[0] == old_point.position[0] && new_point.position[2] == old_point.position[2] && new_point.radius == old_point.radius),
        "agent_route_playable":applied_mesh.agent.spec.start[1] > FP::ZERO && applied_mesh.agent.spec.goal[1] > FP::ZERO,
        "fresh_reopen_mission_matches":reopened_mission.config == applied_mission.config,
        "comparison_scope":"raw_viewport_gpu",
        "comparison_camera_normalization":"Editor::fit_camera",
        "staged_geometry_pixel_diff":staged_pixel_diff,
        "applied_world_pixel_diff":applied_pixel_diff,
        "reopened_world_pixel_diff":reopen_pixel_diff,
        "win_tick":win_tick,
        "win_frame_checksum":format!("0x{win_checksum:016x}"),
        "fresh_replay_same_tick_and_frame":replay_tick == win_tick && replay_checksum == win_checksum,
        "stop_preserved_saved_scene":reopened.state().editor.checksum() == saved_checksum,
        "export_directory":fixture.export,
        "export_files":export_names,
        "export_manifest_sha256":sha256(&manifest_bytes),
        "export_runtime_sha256":sha256(&runtime_bytes),
        "export_game_profile":manifest["game_profile"],
        "export_input_profile":manifest["input_profile"],
        "physical_os_input":false,
        "captures":captures
    });
    fs::write(
        fixture.root.join("acceptance.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
}
