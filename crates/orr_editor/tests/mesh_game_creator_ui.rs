#![cfg(all(
    windows,
    target_arch = "x86_64",
    feature = "mesh-mission",
    feature = "mesh-export"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
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
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

struct Fixture {
    root: PathBuf,
    scene: PathBuf,
    physics_scene: PathBuf,
    physics_bytes: Vec<u8>,
    existing_scene: PathBuf,
    runtime: PathBuf,
    export: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        assert_eq!(
            std::env::var("ORR_MESH_CREATOR_ACCEPTANCE").as_deref(),
            Ok("1")
        );
        assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
        assert_eq!(std::env::var("WGPU_BACKEND").unwrap_or_default(), "vulkan");
        let root = PathBuf::from(
            std::env::var_os("ORR_MESH_CREATOR_CAPTURE_DIR")
                .expect("set a fresh mesh-game creator capture root"),
        );
        assert!(root.is_absolute());
        fs::create_dir(&root).expect("capture root must be fresh");
        let root = root.canonicalize().unwrap();
        let captures = root.join("captures");
        fs::create_dir(&captures).unwrap();
        let scene = PathBuf::from(
            std::env::var_os("ORR_MESH_CREATOR_OUTPUT")
                .unwrap_or_else(|| root.join("created-mesh-game.scene.yaml").into_os_string()),
        );
        assert!(scene.is_absolute() && !scene.exists());
        let scene_parent = scene.parent().unwrap().canonicalize().unwrap();
        let scene = scene_parent.join(scene.file_name().unwrap());
        let physics_scene = root.join("owned-stock-physics.scene.yaml");
        fs::copy(EditorGame::PhysGame.default_scene_path(), &physics_scene).unwrap();
        let physics_bytes = fs::read(&physics_scene).unwrap();
        let existing_scene = root.join("existing.scene.yaml");
        fs::write(&existing_scene, b"preserve this existing destination\n").unwrap();
        let runtime = PathBuf::from(
            std::env::var_os("ORR_MESH_EXPORT_RUNTIME")
                .expect("creator acceptance requires the matching mesh player"),
        );
        assert!(runtime.is_absolute() && runtime.is_file());
        let export = PathBuf::from(
            std::env::var_os("ORR_MESH_CREATOR_EXPORT_OUTPUT")
                .expect("creator acceptance requires a fresh export directory"),
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
            existing_scene,
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
                "Mesh game creator GPU preflight: {} (software: {})",
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
                std::env::var_os("ORR_MESH_CREATOR_ACCEPTANCE").is_none()
                    && std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "required creator GPU acceptance unavailable: {error}"
            );
            eprintln!("SKIP: mesh-game creator GPU unavailable: {error}");
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

fn field_value(harness: &Harness<'_, EditorApp>, label: &str) -> String {
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .accesskit_node()
        .value()
        .expect("wizard field exposes its current value")
        .to_owned()
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
        .expect("creator writes an enabled mission")
}

fn mesh_status(app: &EditorApp) -> NavigationStatus {
    mesh_from_view(app.editor.snapshot().unwrap().predicted())
        .unwrap()
        .agent
        .status
}

fn advance_until_won(harness: &mut Harness<'_, EditorApp>) -> (u64, u64) {
    click_transport(harness, LBL_PLAY);
    wait_for(harness, "Play session", |app| {
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
        let current = mission_view(harness.state());
        if current.run.won == 1 && mesh_status(harness.state()) == NavigationStatus::Arrived {
            let timeline = harness.state().editor.timeline().unwrap();
            let checksum = harness
                .state()
                .editor
                .snapshot()
                .unwrap()
                .predicted()
                .checksum();
            assert!(timeline.tick > previous_tick);
            return (timeline.tick, checksum);
        }
        previous_tick = harness.state().editor.timeline().unwrap().tick;
        click_transport(harness, LBL_STEP);
        let stepped_tick = harness.state().editor.timeline().unwrap().tick;
        assert_eq!(
            stepped_tick,
            previous_tick + 1,
            "each UI Step advances one deterministic frame"
        );
        let progress = mission_view(harness.state());
        let all_collected = (1 << progress.config.count) - 1;
        if progress.run.collected != all_collected {
            assert_eq!(
                progress.run.won, 0,
                "creator cannot win before all objectives and exit"
            );
        }
    }
    panic!("generated mission did not become won/Arrived within the bounded step count")
}

fn capture(harness: &mut Harness<'_, EditorApp>, name: &str) -> (Vec<u8>, u32, u32) {
    settle(harness);
    let frame = harness.render().expect("render production editor frame");
    let pixels = frame.as_raw().clone();
    let size = [frame.width() as usize, frame.height() as usize];
    let image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
    orr_editor::app::write_png(&capture_path(name), &image).unwrap();
    (pixels, frame.width(), frame.height())
}

fn capture_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os("ORR_MESH_CREATOR_CAPTURE_DIR").unwrap())
        .join("captures")
        .join(name)
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

fn changed_pixel_count(left: &[u8], right: &[u8]) -> usize {
    assert_eq!(left.len(), right.len());
    left.as_chunks::<4>().0.iter()
        .zip(right.as_chunks::<4>().0.iter())
        .filter(|(left, right)| {
            left[..3]
                .iter()
                .zip(right[..3].iter())
                .any(|(a, b)| a.abs_diff(*b) > 12)
        })
        .count()
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn capture_record(name: &str) -> Value {
    let bytes = fs::read(capture_path(name)).unwrap();
    serde_json::json!({"name":name,"bytes":bytes.len(),"sha256":sha256(&bytes)})
}

#[test]
#[ignore = "requires ORR_MESH_CREATOR_ACCEPTANCE=1 and hardware Vulkan"]
fn new_mesh_game_wizard_creates_playable_mission_and_exports() {
    if std::env::var_os("ORR_MESH_CREATOR_ACCEPTANCE").is_none()
        && std::env::var_os("ORR_REQUIRE_GPU").is_none()
    {
        eprintln!("SKIP: opt in to actual mesh-game creator GPU acceptance");
        return;
    }
    assert_eq!(
        std::env::var("ORR_MESH_CREATOR_ACCEPTANCE").as_deref(),
        Ok("1")
    );
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    assert_eq!(std::env::var("WGPU_BACKEND").unwrap_or_default(), "vulkan");
    assert!(gpu_available());
    let fixture = Fixture::new();
    let mut harness = fixture.harness();
    harness.run_steps(4);
    assert!(harness.state().has_gpu());
    let initial_checksum = harness.state().editor.checksum();
    let existing_bytes = fs::read(&fixture.existing_scene).unwrap();

    menu_click(&mut harness, "File", "New mesh game…");
    assert_eq!(field_value(&harness, "Game floor width"), "8");
    assert_eq!(field_value(&harness, "Game floor depth"), "6");
    assert_eq!(field_value(&harness, "Game collectible count"), "2");
    assert_eq!(field_value(&harness, "Game distance per tick"), "0.125");
    click_button(&mut harness, "Cancel new mesh game");
    assert_eq!(harness.state().editor.game(), EditorGame::PhysGame);
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    assert!(!fixture.scene.exists());

    menu_click(&mut harness, "File", "New mesh game…");
    type_field(
        &mut harness,
        "New mesh game scene path",
        &fixture.existing_scene.display().to_string(),
    );
    click_button(&mut harness, "Create mesh game");
    assert_eq!(harness.state().editor.game(), EditorGame::PhysGame);
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    assert_eq!(fs::read(&fixture.existing_scene).unwrap(), existing_bytes);
    assert!(!fixture.scene.exists());

    type_field(
        &mut harness,
        "New mesh game scene path",
        &fixture.scene.display().to_string(),
    );
    type_field(&mut harness, "Game floor width", "0");
    click_button(&mut harness, "Create mesh game");
    let invalid_width_rejected_without_output = !fixture.scene.exists();
    assert!(invalid_width_rejected_without_output);
    assert_eq!(harness.state().editor.game(), EditorGame::PhysGame);
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    assert!(!fixture.scene.exists());

    type_field(&mut harness, "Game floor width", "12");
    type_field(&mut harness, "Game floor depth", "8");
    type_field(&mut harness, "Game collectible count", "3");
    type_field(&mut harness, "Game distance per tick", "0.25");
    click_button(&mut harness, "Create mesh game");
    wait_for(&mut harness, "wizard-created mesh game", |app| {
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
    let created_bytes = fs::read(&fixture.scene).unwrap();
    assert_eq!(
        fs::read(&fixture.physics_scene).unwrap(),
        fixture.physics_bytes
    );
    let created_checksum = harness.state().editor.checksum();
    let created_mission = mission_view(harness.state());
    let created_mesh =
        mesh_from_view(harness.state().editor.snapshot().unwrap().predicted()).unwrap();
    assert_eq!(created_mission.config.enabled, 1);
    assert_eq!(created_mission.config.count, 3);
    assert_eq!(created_mission.run.collected, 0);
    assert_eq!(created_mission.run.won, 0);
    assert_eq!(
        created_mesh.agent.spec.distance_per_tick,
        orr_reflect::decimal::parse_fp("0.25").unwrap()
    );
    assert_eq!(
        created_mesh.agent.spec.start,
        [FP::ONE, FP::ZERO, FP::from_int(4)]
    );
    assert_eq!(
        created_mesh.agent.spec.goal,
        [FP::from_int(11), FP::ZERO, FP::from_int(4)]
    );
    for (objective, expected_x) in created_mission
        .config
        .points
        .iter()
        .take(created_mission.config.count as usize)
        .zip(["3.5", "6", "8.5"])
    {
        assert_eq!(
            objective.position,
            [
                orr_reflect::decimal::parse_fp(expected_x).unwrap(),
                FP::ZERO,
                FP::from_int(4)
            ]
        );
        assert_eq!(
            objective.radius,
            orr_reflect::decimal::parse_fp("0.5").unwrap()
        );
    }
    assert_eq!(
        created_mesh
            .graph
            .vertices()
            .iter()
            .map(|vertex| vertex[0])
            .min(),
        Some(FP::ZERO)
    );
    assert_eq!(
        created_mesh
            .graph
            .vertices()
            .iter()
            .map(|vertex| vertex[0])
            .max(),
        Some(FP::from_int(12))
    );
    assert_eq!(
        created_mesh
            .graph
            .vertices()
            .iter()
            .map(|vertex| vertex[2])
            .min(),
        Some(FP::ZERO)
    );
    assert_eq!(
        created_mesh
            .graph
            .vertices()
            .iter()
            .map(|vertex| vertex[2])
            .max(),
        Some(FP::from_int(8))
    );
    let graph_revision = created_mesh.graph.revision();
    let (before_pixels, before_width, before_height) = capture(&mut harness, "created-game.png");
    let (before_viewport_size, before_viewport_pixels) = viewport_pixels(&harness);

    let (win_tick, win_checksum) = advance_until_won(&mut harness);
    let won = mission_view(harness.state());
    assert_eq!(won.run.won, 1);
    assert_eq!(won.run.collected, (1 << created_mission.config.count) - 1);
    assert_eq!(mesh_status(harness.state()), NavigationStatus::Arrived);
    assert!(harness.query_by_label("Game mission complete").is_some());
    assert!(!harness.state().editor.timeline().unwrap().playing);
    let (won_pixels, won_width, won_height) = capture(&mut harness, "game-won.png");
    assert_eq!((won_width, won_height), (before_width, before_height));
    let won_pixel_diff = changed_pixel_count(&before_pixels, &won_pixels);
    assert!(
        won_pixel_diff > 20,
        "mission completion did not change rendered editor pixels: {won_pixel_diff}"
    );
    click_transport(&mut harness, LBL_STOP);
    assert_eq!(harness.state().editor.mode(), Mode::Edit);
    assert_eq!(harness.state().editor.checksum(), created_checksum);
    menu_click(&mut harness, "File", "Save Ctrl+S");
    let saved_bytes = fs::read(&fixture.scene).unwrap();
    let saved_checksum = harness.state().editor.checksum();
    assert_eq!(saved_bytes, created_bytes);
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
    wait_for(&mut reopened, "created game fresh reopen", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && app.editor.checksum() == saved_checksum
            && app.editor.path().as_deref().is_some_and(|path| {
                path == fixture.scene.as_path()
                    || path.canonicalize().ok() == fixture.scene.canonicalize().ok()
            })
    });
    assert_eq!(fs::read(&fixture.scene).unwrap(), saved_bytes);
    let reopened_mission = mission_view(reopened.state());
    assert_eq!(reopened_mission.config.graph_revision, graph_revision);
    assert_eq!(reopened_mission.config.count, 3);
    assert_eq!(reopened_mission.run.collected, 0);
    assert_eq!(reopened_mission.run.won, 0);
    let (_, reopened_width, reopened_height) = capture(&mut reopened, "fresh-reopen.png");
    let (reopened_viewport_size, reopened_viewport_pixels) = viewport_pixels(&reopened);
    assert_eq!(
        (reopened_width, reopened_height),
        (before_width, before_height)
    );
    assert_eq!(reopened_viewport_size, before_viewport_size);
    let reopened_pixel_diff =
        changed_pixel_count(&before_viewport_pixels, &reopened_viewport_pixels);
    assert!(
        reopened_pixel_diff < 20,
        "saved wizard game changed pixels after reopen: {reopened_pixel_diff}"
    );

    let (replay_tick, replay_checksum) = advance_until_won(&mut reopened);
    assert_eq!(replay_tick, win_tick);
    assert_eq!(
        replay_checksum, win_checksum,
        "fresh-open run must reproduce the same complete Frame checksum"
    );
    click_transport(&mut reopened, LBL_STOP);
    assert_eq!(reopened.state().editor.checksum(), saved_checksum);
    assert_eq!(mission_view(reopened.state()).run.won, 0);

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
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline
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
        "orr.mesh-bundle.json".to_owned(),
        "orr_mesh_player.exe".to_owned(),
        scene_name.clone(),
    ];
    expected_names.sort();
    assert_eq!(export_names, expected_names);
    let manifest_bytes = fs::read(fixture.export.join("orr.mesh-bundle.json")).unwrap();
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let runtime_bytes = fs::read(fixture.export.join("orr_mesh_player.exe")).unwrap();
    assert_eq!(
        fs::read(fixture.export.join(&scene_name)).unwrap(),
        saved_bytes
    );
    assert_eq!(manifest["scene"]["sha256"], sha256(&saved_bytes));
    assert_eq!(manifest["runtime"]["sha256"], sha256(&runtime_bytes));
    assert_eq!(manifest["input_profile"], "mesh-click-to-move-v1");
    assert_eq!(manifest["game_profile"], "mesh-mission-v1");
    assert_eq!(manifest["checksum"], saved_checksum);
    assert_eq!(
        manifest["graph_revision"],
        serde_json::json!(graph_revision)
    );
    drop(reopened);

    let capture_records = ["created-game.png", "game-won.png", "fresh-reopen.png"]
        .iter()
        .map(|name| capture_record(name))
        .collect::<Vec<_>>();
    let receipt = serde_json::json!({
        "artifact_root":fixture.root,
        "scene_path":fixture.scene,
        "physics_source_path":fixture.physics_scene,
        "physics_source_sha256":sha256(&fixture.physics_bytes),
        "physics_source_unchanged":fs::read(&fixture.physics_scene).unwrap() == fixture.physics_bytes,
        "existing_destination_path":fixture.existing_scene,
        "existing_destination_preserved":fs::read(&fixture.existing_scene).unwrap() == existing_bytes,
        "cancel_kept_physics_scene":true,
        "invalid_width_rejected_without_output":invalid_width_rejected_without_output,
        "created_scene_sha256":sha256(&saved_bytes),
        "created_scene_checksum":format!("0x{saved_checksum:016x}"),
        "floor_width":"12",
        "floor_depth":"8",
        "collectible_count":created_mission.config.count,
        "distance_per_tick":"0.25",
        "graph_revision":graph_revision.iter().map(|byte|format!("{byte:02x}")).collect::<String>(),
        "win_tick":win_tick,
        "win_frame_checksum":format!("0x{win_checksum:016x}"),
        "fresh_reopen_same_full_frame_win":replay_tick == win_tick && replay_checksum == win_checksum,
        "fresh_reopen_pixel_diff":reopened_pixel_diff,
        "comparison_scope":"raw_viewport_gpu",
        "won_pixel_diff":won_pixel_diff,
        "export_directory":fixture.export,
        "export_files":export_names,
        "export_manifest_sha256":sha256(&manifest_bytes),
        "export_runtime_sha256":sha256(&runtime_bytes),
        "export_input_profile":manifest["input_profile"],
        "export_game_profile":manifest["game_profile"],
        "physical_os_input":false,
        "captures":capture_records
    });
    fs::write(
        fixture.root.join("acceptance.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
}
