#![cfg(all(
    windows,
    target_arch = "x86_64",
    feature = "mesh-mission",
    feature = "mesh-export"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
#![recursion_limit = "256"]

use egui::accesskit::{Role, Toggled};
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
use orr_fp::{fp, FP};
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
            std::env::var("ORR_MESH_GRID_ACCEPTANCE").as_deref(),
            Ok("1")
        );
        assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
        assert_eq!(std::env::var("WGPU_BACKEND").unwrap_or_default(), "vulkan");
        let root = PathBuf::from(
            std::env::var_os("ORR_MESH_GRID_CAPTURE_DIR")
                .expect("set a fresh mesh-grid capture root"),
        );
        assert!(root.is_absolute());
        fs::create_dir(&root).expect("capture root must be fresh");
        let root = root.canonicalize().unwrap();
        fs::create_dir(root.join("captures")).unwrap();
        let scene = PathBuf::from(
            std::env::var_os("ORR_MESH_GRID_OUTPUT")
                .unwrap_or_else(|| root.join("mesh-grid-game.scene.yaml").into_os_string()),
        );
        assert!(scene.is_absolute() && !scene.exists());
        let scene_parent = scene.parent().unwrap().canonicalize().unwrap();
        let scene = scene_parent.join(scene.file_name().unwrap());
        let physics_scene = root.join("owned-stock-physics.scene.yaml");
        fs::copy(EditorGame::PhysGame.default_scene_path(), &physics_scene).unwrap();
        let physics_bytes = fs::read(&physics_scene).unwrap();
        let runtime = PathBuf::from(
            std::env::var_os("ORR_MESH_EXPORT_RUNTIME")
                .expect("mesh-grid acceptance requires the matching mesh player"),
        );
        assert!(runtime.is_absolute() && runtime.is_file());
        let export = PathBuf::from(
            std::env::var_os("ORR_MESH_GRID_EXPORT_OUTPUT")
                .expect("mesh-grid acceptance requires a fresh export directory"),
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
                "Mesh grid GPU preflight: {} (software: {})",
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
                std::env::var_os("ORR_MESH_GRID_ACCEPTANCE").is_none()
                    && std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "required mesh-grid GPU acceptance unavailable: {error}"
            );
            eprintln!("SKIP: mesh-grid GPU unavailable: {error}");
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
    if value.is_empty() {
        harness.key_press(Key::Backspace);
    } else {
        harness
            .get_by_role_and_label(Role::TextInput, label)
            .type_text(value);
    }
    settle(harness);
}

fn fixed_field_value(harness: &Harness<'_, EditorApp>, label: &str) -> FP {
    let value = harness
        .get_by_role_and_label(Role::TextInput, label)
        .accesskit_node()
        .value()
        .expect("accessible fixed-point field value");
    orr_reflect::decimal::parse_fp(&value).expect("valid fixed-point field")
}

fn text_field_value(harness: &Harness<'_, EditorApp>, label: &str) -> String {
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .accesskit_node()
        .value()
        .expect("accessible text field value")
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

fn viewport_pixels(harness: &mut Harness<'_, EditorApp>) -> ((u32, u32), Vec<u8>) {
    harness
        .render()
        .expect("render the production editor before reading viewport pixels");
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
    let inspector =
        egui::containers::panel::PanelState::load(&harness.ctx, egui::Id::new("inspector"))
            .expect("normalized inspector panel state");
    let hierarchy =
        egui::containers::panel::PanelState::load(&harness.ctx, egui::Id::new("hierarchy"))
            .expect("normalized hierarchy panel state");
    let (viewport_size, _) = viewport_pixels(harness);
    let screen_rect = harness.ctx.input(|input| input.viewport_rect());
    let panel_rects = [
        "top",
        "status",
        "bottom",
        "bottom_agent",
        "hierarchy",
        "inspector",
    ]
    .map(|panel_name| {
        (
            panel_name,
            egui::containers::panel::PanelState::load(&harness.ctx, egui::Id::new(panel_name))
                .map(|state| state.outer_rect),
        )
    });
    let editor_status = harness
        .state()
        .editor
        .status()
        .map(|message| (message.error, message.text.clone()));
    let last_action = harness
        .state()
        .editor
        .feed()
        .last_action()
        .map(str::to_owned);
    eprintln!(
        "mesh-grid viewport layout: screen={screen_rect:?}, inspector={:?}, hierarchy={:?}, panels={panel_rects:?}, gpu={viewport_size:?}, status={editor_status:?}, last_action={last_action:?}",
        inspector.outer_rect, hierarchy.outer_rect
    );
}

fn capture(harness: &mut Harness<'_, EditorApp>, root: &Path, name: &str) {
    settle(harness);
    let frame = harness.render().expect("render the production editor");
    let image = egui::ColorImage::from_rgba_unmultiplied(
        [frame.width() as usize, frame.height() as usize],
        frame.as_raw(),
    );
    orr_editor::app::write_png(&root.join("captures").join(name), &image).unwrap();
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

fn revision_hex(revision: [u8; 32]) -> String {
    revision.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn capture_record(root: &Path, name: &str) -> Value {
    let bytes = fs::read(root.join("captures").join(name)).unwrap();
    serde_json::json!({"name":name,"bytes":bytes.len(),"sha256":sha256(&bytes)})
}

struct MissionWin {
    tick: u64,
    checksum: u64,
    agent_status: String,
    agent_position: [String; 3],
    exit_position: [String; 3],
    exit_radius: String,
    inside_exit_radius: bool,
}

fn advance_until_won(harness: &mut Harness<'_, EditorApp>) -> MissionWin {
    click_transport(harness, LBL_PLAY);
    wait_for(harness, "mesh-grid Play", |app| {
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
        let progress = mission_view(harness.state());
        if progress.run.won == 1 {
            let frame = harness.state().editor.snapshot().unwrap().predicted();
            let tick = harness.state().editor.timeline().unwrap().tick;
            let view = mesh_from_view(frame).unwrap();
            let collected_mask = (1u32 << progress.config.count) - 1;
            assert_eq!(progress.run.collected, collected_mask);
            assert!(matches!(
                view.agent.status,
                NavigationStatus::Moving | NavigationStatus::Arrived
            ));
            assert_eq!(progress.config.exit.position, view.agent.spec.goal);
            let distance_squared_raw = (0..3)
                .map(|axis| {
                    let delta = i128::from(view.agent.position[axis].raw())
                        - i128::from(progress.config.exit.position[axis].raw());
                    delta * delta
                })
                .sum::<i128>();
            let radius_raw = i128::from(progress.config.exit.radius.raw());
            let inside_exit_radius = distance_squared_raw <= radius_raw * radius_raw;
            assert!(inside_exit_radius);
            assert!(tick > previous_tick);
            return MissionWin {
                tick,
                checksum: frame.checksum(),
                agent_status: format!("{:?}", view.agent.status),
                agent_position: view.agent.position.map(orr_reflect::decimal::fp_to_decimal),
                exit_position: progress
                    .config
                    .exit
                    .position
                    .map(orr_reflect::decimal::fp_to_decimal),
                exit_radius: orr_reflect::decimal::fp_to_decimal(progress.config.exit.radius),
                inside_exit_radius,
            };
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
    let frame = harness.state().editor.snapshot().unwrap().predicted();
    let view = mesh_from_view(frame).unwrap();
    let mission = mission_view(harness.state());
    panic!(
        "mesh-grid mission did not win within 2048 ticks: tick={}, checksum=0x{:016x}, collected={}/{}, won={}, position={:?}, goal={:?}, status={:?}",
        harness.state().editor.timeline().unwrap().tick,
        frame.checksum(),
        mission.run.collected,
        (1 << mission.config.count) - 1,
        mission.run.won,
        view.agent.position,
        view.agent.spec.goal,
        view.agent.status
    )
}

#[test]
#[ignore = "requires ORR_MESH_GRID_ACCEPTANCE=1 and a hardware Vulkan adapter"]
fn mesh_grid_authoring_builds_hole_and_preserves_a_playable_mission() {
    assert_eq!(
        std::env::var("ORR_MESH_GRID_ACCEPTANCE").as_deref(),
        Ok("1")
    );
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    assert_eq!(std::env::var("WGPU_BACKEND").unwrap_or_default(), "vulkan");
    assert!(gpu_available());

    let fixture = Fixture::new();
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
    wait_for(&mut harness, "created mesh mission", |app| {
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
    if harness.query_by_label("Grid origin X").is_none() {
        click(&mut harness, "Walkable grid");
    }
    for (label, expected_value) in [
        ("Grid width", "12"),
        ("Grid depth", "8"),
        ("Grid columns", "3"),
        ("Grid rows", "3"),
    ] {
        assert_eq!(
            harness
                .get_by_role_and_label(Role::TextInput, label)
                .accesskit_node()
                .value()
                .as_deref(),
            Some(expected_value),
            "walkable grid default for {label}"
        );
    }
    let initial_snapshot = harness.state().editor.snapshot().unwrap();
    let initial_frame = initial_snapshot.predicted();
    let initial_checksum = initial_frame.checksum();
    let initial_history_len = harness.state().editor.history().entries.len();
    let initial_graph = mesh_from_view(initial_frame).unwrap().graph;
    let initial_mission = mission_view(harness.state());
    let initial_bytes = fs::read(&fixture.scene).unwrap();
    normalize_viewport_panels(&mut harness);
    harness.state_mut().editor.fit_camera();
    harness.run_steps(4);
    let (initial_viewport_size, initial_viewport) = viewport_pixels(&mut harness);
    capture(&mut harness, &fixture.root, "initial-grid.png");

    click(
        &mut harness,
        "Preserve and reproject the active mesh mission",
    );
    click(
        &mut harness,
        "I confirm replacing staged geometry with this grid",
    );
    type_field(&mut harness, "Grid columns", "0");
    let invalid_checksum = harness.state().editor.checksum();
    let invalid_history_len = harness.state().editor.history().entries.len();
    click_button(&mut harness, "Stage walkable grid");
    assert_eq!(harness.state().editor.checksum(), invalid_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        invalid_history_len
    );
    assert!(harness
        .query_by_label("grid columns must be between 1 and 7")
        .is_some());
    assert_eq!(
        mesh_from_view(harness.state().editor.snapshot().unwrap().predicted())
            .unwrap()
            .graph
            .revision(),
        initial_graph.revision()
    );

    type_field(&mut harness, "Grid columns", "5");
    type_field(&mut harness, "Grid rows", "3");
    type_field(&mut harness, "Excluded grid cells", "15");
    let invalid_cell_checksum = harness.state().editor.checksum();
    let invalid_cell_history = harness.state().editor.history().entries.len();
    click_button(&mut harness, "Stage walkable grid");
    assert_eq!(harness.state().editor.checksum(), invalid_cell_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        invalid_cell_history
    );
    assert!(harness
        .query_by_label("excluded grid cell index is outside the configured grid")
        .is_some());
    assert_eq!(
        mesh_from_view(harness.state().editor.snapshot().unwrap().predicted())
            .unwrap()
            .graph
            .revision(),
        initial_graph.revision()
    );

    type_field(&mut harness, "Grid columns", "5");
    type_field(&mut harness, "Grid rows", "3");
    type_field(&mut harness, "Excluded grid cells", "");
    let palette_checksum = harness.state().editor.checksum();
    let palette_history_len = harness.state().editor.history().entries.len();
    assert_eq!(
        harness
            .get_by_label("I confirm replacing staged geometry with this grid")
            .accesskit_node()
            .toggled(),
        Some(Toggled::True)
    );
    assert!(harness.query_by_label("Faces (2)").is_some());
    click_button(&mut harness, "+0");
    assert_eq!(text_field_value(&harness, "Excluded grid cells"), "0");
    assert_eq!(harness.state().editor.checksum(), palette_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        palette_history_len
    );
    assert!(harness.query_by_label("Faces (2)").is_some());
    assert!(harness
        .get_by_role_and_label(Role::Button, LBL_PLAY)
        .accesskit_node()
        .is_disabled());
    assert_eq!(
        harness
            .get_by_label("I confirm replacing staged geometry with this grid")
            .accesskit_node()
            .toggled(),
        Some(Toggled::True)
    );
    click_button(&mut harness, "-0");
    assert_eq!(text_field_value(&harness, "Excluded grid cells"), "");
    assert_eq!(harness.state().editor.checksum(), palette_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        palette_history_len
    );
    assert!(harness.query_by_label("Faces (2)").is_some());
    click_button(&mut harness, "+0");
    assert_eq!(text_field_value(&harness, "Excluded grid cells"), "0");
    assert_eq!(harness.state().editor.checksum(), palette_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        palette_history_len
    );
    assert!(harness.query_by_label("Faces (2)").is_some());
    let palette_roundtrip_preserved_host_and_confirmation = harness.state().editor.checksum()
        == palette_checksum
        && harness.state().editor.history().entries.len() == palette_history_len
        && harness.query_by_label("Faces (2)").is_some()
        && text_field_value(&harness, "Excluded grid cells") == "0"
        && harness
            .get_by_label("I confirm replacing staged geometry with this grid")
            .accesskit_node()
            .toggled()
            == Some(Toggled::True)
        && harness
            .get_by_role_and_label(Role::Button, LBL_PLAY)
            .accesskit_node()
            .is_disabled();
    assert!(palette_roundtrip_preserved_host_and_confirmation);
    type_field(&mut harness, "Grid origin X", "0");
    type_field(&mut harness, "Grid origin Y", "0");
    type_field(&mut harness, "Grid origin Z", "0");
    type_field(&mut harness, "Grid width", "12");
    type_field(&mut harness, "Grid depth", "8");
    type_field(&mut harness, "Grid columns", "5");
    type_field(&mut harness, "Grid rows", "3");
    click_button(&mut harness, "Stage walkable grid");
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        initial_history_len
    );
    assert!(harness.query_by_label("Faces (28)").is_some());
    let staged_host_graph = mesh_from_view(harness.state().editor.snapshot().unwrap().predicted())
        .unwrap()
        .graph;
    assert_eq!(staged_host_graph.revision(), initial_graph.revision());
    assert_eq!(staged_host_graph.triangles().len(), 2);
    click_button(&mut harness, "Undo staged geometry");
    assert!(harness.query_by_label("Faces (2)").is_some());
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    click_button(&mut harness, "Redo staged geometry");
    assert!(harness.query_by_label("Faces (28)").is_some());
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    let stage_preserved_host_and_history = harness.state().editor.checksum() == initial_checksum
        && harness.state().editor.history().entries.len() == initial_history_len;
    assert!(stage_preserved_host_and_history);
    harness.state_mut().editor.fit_camera();
    normalize_viewport_panels(&mut harness);
    harness.run_steps(4);
    let (staged_viewport_size, staged_viewport) = viewport_pixels(&mut harness);
    assert_eq!(staged_viewport_size, initial_viewport_size);
    let staged_pixel_diff = changed_pixel_count(&initial_viewport, &staged_viewport);
    assert!(
        staged_pixel_diff > 20,
        "grid staging was not visible: {staged_pixel_diff}"
    );
    capture(&mut harness, &fixture.root, "grid-preview.png");

    click_button(&mut harness, "Preview mission-preserving geometry");
    assert!(harness
        .query_by_label("Mission reprojection preview is ready; the host is unchanged.")
        .is_some());
    click_button(&mut harness, "Cancel mission geometry preview");
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
        initial_graph.revision()
    );
    click_button(&mut harness, "Preview mission-preserving geometry");
    assert!(harness
        .query_by_label("Mission reprojection preview is ready; the host is unchanged.")
        .is_some());
    click(
        &mut harness,
        "I confirm reprojecting the agent and active mission onto this staged graph",
    );
    click_button(&mut harness, "Apply geometry and reproject mission");
    wait_for(&mut harness, "grid and mission admission", |app| {
        app.editor.checksum() != initial_checksum
            && mesh_from_view(app.editor.snapshot().unwrap().predicted())
                .is_ok_and(|view| view.graph.triangles().len() == 28)
    });
    let applied_snapshot = harness.state().editor.snapshot().unwrap();
    let applied_frame = applied_snapshot.predicted();
    let applied_checksum = applied_frame.checksum();
    let applied_history_len = harness.state().editor.history().entries.len();
    let applied_mesh = mesh_from_view(applied_frame).unwrap();
    let staged_revision = applied_mesh.graph.revision();
    let applied_mission = mission_view(harness.state());
    assert_eq!(applied_history_len, initial_history_len + 1);
    assert_eq!(applied_mesh.graph.triangles().len(), 28);
    assert!(applied_mesh
        .graph
        .project([fp!(1.2), FP::ZERO, fp!(1.333)], FP::ZERO)
        .is_err());
    assert!(applied_mesh
        .graph
        .project([fp!(1), FP::ZERO, fp!(4)], FP::ZERO)
        .is_ok());
    assert_eq!(applied_mission.config.count, initial_mission.config.count);
    assert_eq!(applied_mission.config.points, initial_mission.config.points);
    assert_eq!(
        applied_mission.config.exit.radius,
        initial_mission.config.exit.radius
    );
    assert_eq!(applied_mission.run, initial_mission.run);
    assert_eq!(applied_mesh.agent.spec.distance_per_tick, fp!(0.25));
    assert_eq!(applied_mission.config.graph_revision, staged_revision);
    harness.state_mut().editor.fit_camera();
    normalize_viewport_panels(&mut harness);
    harness.run_steps(4);
    let (grid_applied_viewport_size, grid_applied_viewport) = viewport_pixels(&mut harness);
    assert_eq!(grid_applied_viewport_size, initial_viewport_size);
    let grid_applied_pixel_diff = changed_pixel_count(&initial_viewport, &grid_applied_viewport);
    assert!(
        grid_applied_pixel_diff > 20,
        "applied grid was not visible: {grid_applied_pixel_diff}"
    );

    menu_click(&mut harness, "Edit", "Undo Ctrl+Z");
    wait_for(&mut harness, "grid geometry undo", |app| {
        app.editor.checksum() == initial_checksum
    });
    assert_eq!(
        mesh_from_view(harness.state().editor.snapshot().unwrap().predicted())
            .unwrap()
            .graph
            .revision(),
        initial_graph.revision()
    );
    menu_click(&mut harness, "Edit", "Redo Ctrl+Y");
    wait_for(&mut harness, "grid geometry redo", |app| {
        app.editor.checksum() == applied_checksum
    });

    if harness
        .query_by_label("Mission collectible count")
        .is_none()
    {
        click(&mut harness, "Mesh mission");
    }
    let pre_checkpoint_checksum = harness.state().editor.checksum();
    let pre_checkpoint_history_len = harness.state().editor.history().entries.len();
    let pre_checkpoint_mission = mission_view(harness.state());
    let pre_checkpoint_graph =
        mesh_from_view(harness.state().editor.snapshot().unwrap().predicted()).unwrap();
    click_button(&mut harness, "Stage mission route checkpoints");
    assert_eq!(
        harness.state().editor.checksum(),
        pre_checkpoint_checksum,
        "staging route checkpoints must not patch the host"
    );
    assert_eq!(
        harness.state().editor.history().entries.len(),
        pre_checkpoint_history_len
    );
    assert_eq!(
        mission_view(harness.state()).config,
        pre_checkpoint_mission.config
    );
    assert_eq!(
        mission_view(harness.state()).run,
        pre_checkpoint_mission.run
    );
    let route_checkpoint_draft_preserved_host = harness.state().editor.checksum()
        == pre_checkpoint_checksum
        && harness.state().editor.history().entries.len() == pre_checkpoint_history_len
        && mission_view(harness.state()).config == pre_checkpoint_mission.config
        && mission_view(harness.state()).run == pre_checkpoint_mission.run;
    assert!(route_checkpoint_draft_preserved_host);
    assert!(harness
        .get_by_role_and_label(Role::Button, LBL_PLAY)
        .accesskit_node()
        .is_disabled());
    assert_eq!(
        fixed_field_value(&harness, "Mission collectible count"),
        fp!(3)
    );
    let staged_checkpoint_positions = (1..=pre_checkpoint_mission.config.count)
        .map(|index| {
            ["X", "Y", "Z"]
                .map(|axis| fixed_field_value(&harness, &format!("Collectible {index} {axis}")))
        })
        .collect::<Vec<_>>();
    assert_ne!(
        staged_checkpoint_positions,
        pre_checkpoint_mission.config.points[..3]
            .iter()
            .map(|point| point.position)
            .collect::<Vec<_>>(),
        "route staging should update the editable checkpoint fields"
    );
    click_button(&mut harness, "Apply mesh mission");
    wait_for(&mut harness, "route checkpoint mission apply", |app| {
        app.editor.checksum() != pre_checkpoint_checksum
    });
    let route_checkpoint_checksum = harness.state().editor.checksum();
    let route_checkpoint_history_len = harness.state().editor.history().entries.len();
    assert_eq!(route_checkpoint_history_len, pre_checkpoint_history_len + 1);
    assert!(!harness
        .get_by_role_and_label(Role::Button, LBL_PLAY)
        .accesskit_node()
        .is_disabled());
    let route_checkpoint_apply_created_one_undo_entry =
        route_checkpoint_history_len == pre_checkpoint_history_len + 1;
    assert!(route_checkpoint_apply_created_one_undo_entry);
    let route_checkpoint_mission = mission_view(harness.state());
    let route_checkpoint_view =
        mesh_from_view(harness.state().editor.snapshot().unwrap().predicted()).unwrap();
    assert_ne!(
        route_checkpoint_mission.config.points,
        pre_checkpoint_mission.config.points
    );
    assert_eq!(
        route_checkpoint_mission.config.count,
        pre_checkpoint_mission.config.count
    );
    assert_eq!(
        route_checkpoint_mission.config.exit.position,
        route_checkpoint_view.agent.spec.goal
    );
    assert_eq!(
        route_checkpoint_mission.config.exit.radius,
        pre_checkpoint_mission.config.exit.radius
    );
    assert_eq!(route_checkpoint_mission.run, pre_checkpoint_mission.run);
    assert_eq!(
        route_checkpoint_mission.config.graph_revision,
        pre_checkpoint_mission.config.graph_revision
    );
    let canonical_route_points = &pre_checkpoint_graph.agent.path.points;
    let applied_checkpoint_positions = &route_checkpoint_mission.config.points[..3];
    assert!(applied_checkpoint_positions
        .iter()
        .all(|point| canonical_route_points.contains(&point.position)));
    assert!(applied_checkpoint_positions
        .iter()
        .enumerate()
        .all(|(index, point)| !applied_checkpoint_positions[..index]
            .iter()
            .any(|prior| prior.position == point.position)));
    for index in 0..3 {
        assert_eq!(
            route_checkpoint_mission.config.points[index].radius,
            pre_checkpoint_mission.config.points[index].radius
        );
    }
    menu_click(&mut harness, "Edit", "Undo Ctrl+Z");
    wait_for(&mut harness, "route checkpoint undo", |app| {
        app.editor.checksum() == pre_checkpoint_checksum
    });
    assert_eq!(
        mission_view(harness.state()).config,
        pre_checkpoint_mission.config
    );
    menu_click(&mut harness, "Edit", "Redo Ctrl+Y");
    wait_for(&mut harness, "route checkpoint redo", |app| {
        app.editor.checksum() == route_checkpoint_checksum
    });
    assert_eq!(
        mission_view(harness.state()).config,
        route_checkpoint_mission.config
    );
    let route_checkpoint_undo_redo_verified = harness.state().editor.checksum()
        == route_checkpoint_checksum
        && mission_view(harness.state()).config == route_checkpoint_mission.config;
    assert!(route_checkpoint_undo_redo_verified);
    let route_checkpoint_positions = route_checkpoint_mission.config.points[..3]
        .iter()
        .map(|point| point.position.map(orr_reflect::decimal::fp_to_decimal))
        .collect::<Vec<_>>();
    harness.state_mut().editor.fit_camera();
    normalize_viewport_panels(&mut harness);
    harness.run_steps(4);
    let (applied_viewport_size, applied_viewport) = viewport_pixels(&mut harness);
    capture(&mut harness, &fixture.root, "grid-applied.png");
    assert_eq!(applied_viewport_size, initial_viewport_size);
    let applied_pixel_diff = changed_pixel_count(&initial_viewport, &applied_viewport);
    assert!(
        applied_pixel_diff > 20,
        "applied grid and route checkpoints were not visible: {applied_pixel_diff}"
    );
    menu_click(&mut harness, "File", "Save Ctrl+S");
    let saved_bytes = fs::read(&fixture.scene).unwrap();
    let saved_checksum = harness.state().editor.checksum();
    let saved_mission = mission_view(harness.state());
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
    wait_for(&mut reopened, "grid scene reopen", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && app.editor.checksum() == saved_checksum
            && app.editor.path().as_deref().is_some_and(|path| {
                path == fixture.scene.as_path()
                    || path.canonicalize().ok() == fixture.scene.canonicalize().ok()
            })
    });
    let reopened_mesh =
        mesh_from_view(reopened.state().editor.snapshot().unwrap().predicted()).unwrap();
    let reopened_mission = mission_view(reopened.state());
    assert_eq!(reopened_mesh.graph.revision(), staged_revision);
    assert_eq!(reopened_mesh.graph.triangles().len(), 28);
    assert!(reopened_mesh
        .graph
        .project([fp!(1.2), FP::ZERO, fp!(1.333)], FP::ZERO)
        .is_err());
    assert_eq!(reopened_mission.config, saved_mission.config);
    reopened.state_mut().editor.fit_camera();
    normalize_viewport_panels(&mut reopened);
    reopened.run_steps(4);
    let (reopened_viewport_size, reopened_viewport) = viewport_pixels(&mut reopened);
    assert_eq!(reopened_viewport_size, applied_viewport_size);
    let reopen_pixel_diff = changed_pixel_count(&applied_viewport, &reopened_viewport);
    assert!(
        reopen_pixel_diff < 20,
        "grid reopen differs: {reopen_pixel_diff}"
    );
    capture(&mut reopened, &fixture.root, "grid-reopened.png");

    let win = advance_until_won(&mut reopened);
    assert_eq!(mission_view(reopened.state()).run.won, 1);
    settle(&mut reopened);
    assert!(reopened.query_by_label("Game mission complete").is_some());
    capture(&mut reopened, &fixture.root, "grid-won.png");
    click_transport(&mut reopened, LBL_STOP);
    assert_eq!(reopened.state().editor.mode(), Mode::Edit);
    assert_eq!(reopened.state().editor.checksum(), saved_checksum);
    let replay = advance_until_won(&mut reopened);
    assert_eq!(replay.tick, win.tick);
    assert_eq!(replay.checksum, win.checksum);
    click_transport(&mut reopened, LBL_STOP);

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
    let scene_bytes = fs::read(fixture.export.join(&scene_name)).unwrap();
    let runtime_bytes = fs::read(fixture.export.join("orr_mesh_player.exe")).unwrap();
    assert_eq!(scene_bytes, saved_bytes);
    assert_eq!(runtime_bytes, fs::read(&fixture.runtime).unwrap());
    assert_eq!(manifest["scene"]["sha256"], sha256(&saved_bytes));
    assert_eq!(manifest["runtime"]["sha256"], sha256(&runtime_bytes));
    assert_eq!(manifest["checksum"], saved_checksum);
    assert_eq!(
        manifest["graph_revision"],
        serde_json::json!(staged_revision)
    );
    assert_eq!(manifest["game_profile"], "mesh-mission-v1");
    assert_eq!(manifest["input_profile"], "mesh-click-to-move-v1");
    assert_eq!(
        fs::read(&fixture.physics_scene).unwrap(),
        fixture.physics_bytes
    );

    let captures = [
        "initial-grid.png",
        "grid-preview.png",
        "grid-applied.png",
        "grid-reopened.png",
        "grid-won.png",
    ]
    .iter()
    .map(|name| capture_record(&fixture.root, name))
    .collect::<Vec<_>>();
    let receipt = serde_json::json!({
        "artifact_root":fixture.root,
        "scene_path":fixture.scene,
        "scene_sha256":sha256(&saved_bytes),
        "scene_checksum":format!("0x{saved_checksum:016x}"),
        "source_graph_revision":revision_hex(initial_graph.revision()),
        "reprojected_graph_revision":revision_hex(staged_revision),
        "physics_source_path":fixture.physics_scene,
        "physics_source_sha256":sha256(&fixture.physics_bytes),
        "physics_source_unchanged":fs::read(&fixture.physics_scene).unwrap() == fixture.physics_bytes,
        "grid_origin":[0,0,0],
        "grid_width":12,
        "grid_depth":8,
        "grid_columns":5,
        "grid_rows":3,
        "excluded_cells":[0],
        "grid_triangle_count":28,
        "excluded_cell_unprojectable":true,
        "route_position_projectable":true,
        "collectible_count":3,
        "invalid_zero_columns_preserved_host":true,
        "invalid_excluded_cell_preserved_host":true,
        "palette_roundtrip_preserved_host_and_confirmation":palette_roundtrip_preserved_host_and_confirmation,
        "stage_preserved_host_and_history":stage_preserved_host_and_history,
        "local_geometry_undo_redo_verified":true,
        "mission_preview_and_apply_verified":true,
        "global_undo_redo_checksums_verified":true,
        "fresh_reopen_grid_and_mission_match":reopened_mesh.graph.revision() == staged_revision && reopened_mission.config == saved_mission.config,
        "route_checkpoint_draft_preserved_host":route_checkpoint_draft_preserved_host,
        "route_checkpoint_apply_created_one_undo_entry":route_checkpoint_apply_created_one_undo_entry,
        "route_checkpoint_undo_redo_verified":route_checkpoint_undo_redo_verified,
        "route_checkpoint_positions":route_checkpoint_positions,
        "route_checkpoints_are_distinct_canonical_route_points":applied_checkpoint_positions.iter().all(|point| canonical_route_points.contains(&point.position)) && applied_checkpoint_positions.iter().enumerate().all(|(index, point)| !applied_checkpoint_positions[..index].iter().any(|prior| prior.position == point.position)),
        "win_tick":win.tick,
        "win_frame_checksum":format!("0x{:016x}", win.checksum),
        "actualwin_agent_status":win.agent_status,
        "actualwin_agent_position":win.agent_position,
        "actualwin_exit_position":win.exit_position,
        "actualwin_exit_radius":win.exit_radius,
        "win_inside_authored_exit_radius":win.inside_exit_radius,
        "fresh_replay_same_tick_and_frame":replay.tick == win.tick && replay.checksum == win.checksum,
        "stop_preserved_saved_scene":reopened.state().editor.checksum() == saved_checksum,
        "comparison_scope":"raw_viewport_gpu",
        "comparison_camera_normalization":"Editor::fit_camera",
        "staged_grid_pixel_diff":staged_pixel_diff,
        "applied_grid_pixel_diff":applied_pixel_diff,
        "grid_applied_before_route_checkpoint_pixel_diff":grid_applied_pixel_diff,
        "reopened_grid_pixel_diff":reopen_pixel_diff,
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
