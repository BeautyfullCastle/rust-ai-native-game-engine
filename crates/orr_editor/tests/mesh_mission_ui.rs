#![cfg(all(
    windows,
    target_arch = "x86_64",
    feature = "mesh-mission",
    feature = "mesh-export"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::accesskit::Role;
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
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
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    captures: PathBuf,
    scene: PathBuf,
    physics_scene: PathBuf,
    runtime: PathBuf,
    export: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        assert_eq!(
            std::env::var("ORR_MESH_MISSION_ACCEPTANCE").as_deref(),
            Ok("1")
        );
        assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
        assert_eq!(std::env::var("WGPU_BACKEND").unwrap_or_default(), "vulkan");
        let temporary = tempfile::tempdir().unwrap();
        let root = PathBuf::from(
            std::env::var_os("ORR_MESH_MISSION_CAPTURE_DIR")
                .expect("set a fresh mission capture root"),
        );
        assert!(root.is_absolute());
        fs::create_dir(&root).expect("mission capture root must be fresh");
        let root = root.canonicalize().unwrap();
        let captures = root.join("captures");
        fs::create_dir(&captures).unwrap();
        let scene = PathBuf::from(
            std::env::var_os("ORR_MESH_MISSION_OUTPUT")
                .unwrap_or_else(|| root.join("mesh-mission.scene.yaml").into_os_string()),
        );
        assert!(scene.is_absolute() && !scene.exists());
        let scene_parent = scene.parent().unwrap().canonicalize().unwrap();
        let scene = scene_parent.join(scene.file_name().unwrap());
        let physics_scene = root.join("owned-stock-physics.scene.yaml");
        fs::copy(EditorGame::PhysGame.default_scene_path(), &physics_scene).unwrap();
        let runtime = PathBuf::from(
            std::env::var_os("ORR_MESH_EXPORT_RUNTIME")
                .expect("mission acceptance requires the matching mesh player"),
        );
        assert!(runtime.is_absolute() && runtime.is_file());
        let export = PathBuf::from(
            std::env::var_os("ORR_MESH_MISSION_EXPORT_OUTPUT")
                .expect("mission acceptance requires an export directory"),
        );
        assert!(export.is_absolute() && !export.exists());
        assert_eq!(
            export.parent().unwrap().canonicalize().unwrap(),
            root.parent().unwrap()
        );
        Self {
            _temporary: temporary,
            root,
            captures,
            scene,
            physics_scene,
            runtime,
            export,
        }
    }

    fn harness(&self, scene: &Path, game: EditorGame) -> Harness<'static, EditorApp> {
        let mut editor = Editor::open_game(scene, game).unwrap();
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
                "Mesh mission GPU preflight: {} (software: {})",
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
                std::env::var_os("ORR_MESH_MISSION_ACCEPTANCE").is_none()
                    && std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "required mission GPU acceptance unavailable: {error}"
            );
            eprintln!("SKIP: mission GPU unavailable: {error}");
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

fn type_scene_path(harness: &mut Harness<'_, EditorApp>, path: &Path, action: &str) {
    type_field(harness, "Scene path", &path.display().to_string());
    click_button(harness, action);
}

fn pointer_for_world(harness: &Harness<'_, EditorApp>, world: [f32; 3]) -> Pos2 {
    let viewport = harness
        .state()
        .ui
        .viewport_rect
        .expect("EditorApp has a production 3D viewport rectangle");
    let size = harness.state().ui.viewport_px;
    let camera = harness
        .state()
        .editor
        .presentation_camera3d(size)
        .expect("admitted mesh graph has a presentation camera");
    let pixel = camera
        .world_to_screen(world, size)
        .expect("mission point projects into the viewport");
    let points_per_pixel = viewport.width() / size.0 as f32;
    let pointer =
        viewport.min + egui::vec2(pixel[0] * points_per_pixel, pixel[1] * points_per_pixel);
    assert!(viewport.contains(pointer), "projected point is visible");
    pointer
}

fn click_viewport(harness: &mut Harness<'_, EditorApp>, world: [f32; 3]) {
    let pointer = pointer_for_world(harness, world);
    harness.event(Event::PointerMoved(pointer));
    harness.run_steps(2);
    harness.event(Event::PointerButton {
        pos: pointer,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.step();
    harness.event(Event::PointerButton {
        pos: pointer,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    settle(harness);
}

fn field_fp(harness: &Harness<'_, EditorApp>, label: &str) -> FP {
    let field = harness.get_by_role_and_label(Role::TextInput, label);
    let node = field.accesskit_node();
    let value = node
        .value()
        .expect("mission coordinate field exposes its value");
    orr_reflect::decimal::parse_fp(value.trim()).expect("mission coordinate is fixed point")
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

fn mission_view(app: &EditorApp) -> Option<MissionView> {
    let snapshot = app.editor.snapshot().unwrap();
    mission::from_view(snapshot.predicted()).unwrap()
}

fn mesh_view(app: &EditorApp) -> orr_remote::mesh_navigation::MeshView {
    let snapshot = app.editor.snapshot().unwrap();
    mesh_from_view(snapshot.predicted()).unwrap()
}

fn composite_pixels(harness: &mut Harness<'_, EditorApp>, filename: &str) -> Vec<u8> {
    settle(harness);
    let viewport = harness
        .state()
        .viewport3d_gpu()
        .expect("production App viewport GPU is initialized");
    eprintln!("Mesh mission App GPU: {}", viewport.gpu().adapter_name());
    let frame = harness
        .render()
        .expect("render complete production App frame");
    let size = [frame.width() as usize, frame.height() as usize];
    let pixels = frame.as_raw().clone();
    let image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
    orr_editor::app::write_png(&fixture_capture_path(filename), &image).unwrap();
    pixels
}

fn capture_world_pair(
    harness: &mut Harness<'_, EditorApp>,
    composite_name: &str,
    viewport_name: &str,
) -> (Vec<u8>, (u32, u32), Vec<u8>) {
    let expanded = [
        harness.query_by_label("Distance per tick").is_some(),
        harness
            .query_by_label("Mission collectible count")
            .is_some(),
    ];
    for (label, open) in ["Static-mesh navigation", "Mesh mission"]
        .into_iter()
        .zip(expanded)
    {
        if open {
            click(harness, label);
        }
    }
    for (name, width) in [("inspector", 340.0), ("hierarchy", 230.0)] {
        let identifier = egui::Id::new(name);
        let mut state = egui::containers::panel::PanelState::load(&harness.ctx, identifier)
            .expect("rendered production panel state");
        if name == "inspector" {
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
    let composite = composite_pixels(harness, composite_name);
    let (size, viewport) = viewport_pixels(harness, viewport_name);
    assert_eq!(size, (930, 904), "uncropped capture geometry must match");
    for (label, open) in ["Static-mesh navigation", "Mesh mission"]
        .into_iter()
        .zip(expanded)
    {
        if open {
            click(harness, label);
        }
    }
    (composite, size, viewport)
}

fn viewport_pixels(harness: &mut Harness<'_, EditorApp>, filename: &str) -> ((u32, u32), Vec<u8>) {
    let viewport = harness
        .state()
        .viewport3d_gpu()
        .expect("production App viewport GPU is initialized");
    let size = viewport.gpu().target().size();
    let pixels = viewport.gpu().read_rgba8();
    assert_eq!(pixels.len(), (size.0 * size.1 * 4) as usize);
    let image =
        egui::ColorImage::from_rgba_unmultiplied([size.0 as usize, size.1 as usize], &pixels);
    orr_editor::app::write_png(&fixture_capture_path(filename), &image).unwrap();
    (size, pixels)
}

fn fixture_capture_path(filename: &str) -> PathBuf {
    PathBuf::from(std::env::var_os("ORR_MESH_MISSION_CAPTURE_DIR").unwrap())
        .join("captures")
        .join(filename)
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
    let bytes = fs::read(root.join(name)).unwrap();
    serde_json::json!({"name":name,"bytes":bytes.len(),"sha256":sha256(&bytes)})
}

#[test]
#[ignore = "requires ORR_MESH_MISSION_ACCEPTANCE=1 and a hardware Vulkan adapter"]
fn mesh_mission_collects_then_wins_after_reopen_and_exports() {
    let requested = std::env::var_os("ORR_MESH_MISSION_ACCEPTANCE").is_some()
        || std::env::var_os("ORR_REQUIRE_GPU").is_some();
    if !requested {
        eprintln!("SKIP: opt in to actual Mesh mission GPU acceptance");
        return;
    }
    assert_eq!(
        std::env::var("ORR_MESH_MISSION_ACCEPTANCE").as_deref(),
        Ok("1")
    );
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    assert_eq!(std::env::var("WGPU_BACKEND").unwrap_or_default(), "vulkan");
    assert!(gpu_available());
    let fixture = Fixture::new();
    let mut harness = fixture.harness(&fixture.physics_scene, EditorGame::PhysGame);
    harness.run_steps(4);
    assert!(harness.state().has_gpu());
    menu_click(&mut harness, "File", "New mesh scene…");
    type_scene_path(&mut harness, &fixture.scene, "Create");
    click(&mut harness, "Static-mesh navigation");
    type_field(&mut harness, "Distance per tick", "0.25");
    click_button(&mut harness, "Create ramp and bridge");
    click_button(&mut harness, "Place mesh agent");
    menu_click(&mut harness, "File", "Save Ctrl+S");
    let scene_bytes = fs::read(&fixture.scene).unwrap();
    let scene_checksum = harness.state().editor.checksum();
    let graph_revision = mesh_from_view(harness.state().editor.snapshot().unwrap().predicted())
        .unwrap()
        .graph
        .revision();
    assert!(mission_view(harness.state()).is_none());
    let (no_mission_pixels, baseline_viewport_size, no_mission_viewport) =
        capture_world_pair(&mut harness, "no-mission.png", "no-mission-viewport.png");

    if harness
        .query_by_label("Mission collectible count")
        .is_none()
    {
        click(&mut harness, "Mesh mission");
    }
    click(&mut harness, "Enable mesh mission");
    type_field(&mut harness, "Mission collectible count", "2");
    for (label, value) in [
        ("Collectible 1 X", "1.5"),
        ("Collectible 1 Y", "0"),
        ("Collectible 1 Z", "1"),
        ("Collectible 1 radius", "0.4"),
        ("Collectible 2 X", "2"),
        ("Collectible 2 Y", "0"),
        ("Collectible 2 Z", "1"),
        ("Collectible 2 radius", "0.4"),
        ("Mission exit X", "3"),
        ("Mission exit Y", "2"),
        ("Mission exit Z", "1"),
        ("Mission exit radius", "0.5"),
    ] {
        type_field(&mut harness, label, value);
    }
    let checksum_before_viewport_picks = harness.state().editor.checksum();
    let history_before_viewport_picks = harness.state().editor.history().entries.len();
    let collectible_radius_before_pick = field_fp(&harness, "Collectible 1 radius");
    let exit_radius_before_pick = field_fp(&harness, "Mission exit radius");
    click_button(&mut harness, "Pick collectible 1 in viewport");
    assert!(harness
        .query_by_label("Click an admitted walkable surface for collectible 1.")
        .is_some());
    click_viewport(&mut harness, [1.75, 0.0, 1.0]);
    let picked_collectible = [
        field_fp(&harness, "Collectible 1 X"),
        field_fp(&harness, "Collectible 1 Y"),
        field_fp(&harness, "Collectible 1 Z"),
    ];
    assert_ne!(
        picked_collectible,
        [
            orr_reflect::decimal::parse_fp("1.5").unwrap(),
            FP::ZERO,
            FP::ONE,
        ]
    );
    for (actual, expected) in picked_collectible.into_iter().zip([1.75, 0.0, 1.0]) {
        assert!((actual.to_f32() - expected).abs() <= 0.04);
    }
    assert_eq!(
        field_fp(&harness, "Collectible 1 radius"),
        collectible_radius_before_pick
    );
    assert_eq!(
        harness.state().editor.checksum(),
        checksum_before_viewport_picks
    );
    assert_eq!(
        harness.state().editor.history().entries.len(),
        history_before_viewport_picks
    );
    click_button(&mut harness, "Pick mission exit in viewport");
    click_viewport(&mut harness, [3.25, 2.0, 1.0]);
    let picked_exit = [
        field_fp(&harness, "Mission exit X"),
        field_fp(&harness, "Mission exit Y"),
        field_fp(&harness, "Mission exit Z"),
    ];
    for (actual, expected) in picked_exit.into_iter().zip([3.25, 2.0, 1.0]) {
        assert!((actual.to_f32() - expected).abs() <= 0.04);
    }
    assert_eq!(
        field_fp(&harness, "Mission exit radius"),
        exit_radius_before_pick
    );
    assert_eq!(
        harness.state().editor.checksum(),
        checksum_before_viewport_picks
    );
    assert_eq!(
        harness.state().editor.history().entries.len(),
        history_before_viewport_picks
    );
    click_button(&mut harness, "Pick collectible 2 in viewport");
    let viewport = harness
        .state()
        .ui
        .viewport_rect
        .expect("EditorApp has a production 3D viewport rectangle");
    let sky = Pos2::new(viewport.max.x - 4.0, viewport.min.y + 4.0);
    harness.event(Event::PointerMoved(sky));
    harness.run_steps(2);
    harness.event(Event::PointerButton {
        pos: sky,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.step();
    harness.event(Event::PointerButton {
        pos: sky,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    settle(&mut harness);
    assert!(harness
        .query_by_label("No admitted walkable surface was hit; click a surface to try again.")
        .is_some());
    harness.key_press(Key::Escape);
    settle(&mut harness);
    assert!(harness
        .query_by_label("Mission point picking canceled; staged coordinates were preserved.")
        .is_some());
    assert_eq!(field_fp(&harness, "Collectible 2 X"), FP::from_int(2));
    assert_eq!(
        harness.state().editor.checksum(),
        checksum_before_viewport_picks
    );
    let viewport_picks_preserved_history =
        harness.state().editor.history().entries.len() == history_before_viewport_picks;
    assert!(viewport_picks_preserved_history);
    assert!(harness
        .get_by_role_and_label(Role::Button, LBL_PLAY)
        .accesskit_node()
        .is_disabled());
    type_field(&mut harness, "Collectible 2 X", "999");
    click_button(&mut harness, "Apply mesh mission");
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    assert_eq!(fs::read(&fixture.scene).unwrap(), scene_bytes);
    type_field(&mut harness, "Collectible 2 X", "2");
    click_button(&mut harness, "Apply mesh mission");
    assert_ne!(harness.state().editor.checksum(), scene_checksum);
    menu_click(&mut harness, "Edit", "Undo Ctrl+Z");
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    assert!(mission_view(harness.state()).is_none());
    menu_click(&mut harness, "Edit", "Redo Ctrl+Y");
    let authored_view = mission_view(harness.state()).unwrap();
    assert_eq!(authored_view.config.enabled, 1);
    assert_eq!(authored_view.config.count, 2);
    assert_eq!(authored_view.run.collected, 0);
    assert_eq!(authored_view.run.won, 0);
    assert_eq!(authored_view.config.points[0].position, picked_collectible);
    assert_eq!(
        authored_view.config.points[0].radius,
        collectible_radius_before_pick
    );
    assert_eq!(authored_view.config.exit.position, picked_exit);
    assert_eq!(authored_view.config.exit.radius, exit_radius_before_pick);
    let mission_checksum = harness.state().editor.checksum();
    assert_ne!(mission_checksum, scene_checksum);
    let (configured_pixels, configured_viewport_size, configured_viewport) = capture_world_pair(
        &mut harness,
        "mission-configured-edit.png",
        "mission-configured-viewport.png",
    );
    assert_eq!(configured_viewport_size, baseline_viewport_size);
    let marker_pixel_diff = changed_pixel_count(&no_mission_viewport, &configured_viewport);
    assert!(
        marker_pixel_diff > 20,
        "mission world markers did not change viewport pixels: {marker_pixel_diff}"
    );
    let configured_pixel_diff = changed_pixel_count(&no_mission_pixels, &configured_pixels);
    assert!(
        configured_pixel_diff > 20,
        "mission markers/UI did not change the App frame: {configured_pixel_diff}"
    );
    menu_click(&mut harness, "File", "Save Ctrl+S");
    let saved_scene_bytes = fs::read(&fixture.scene).unwrap();
    let saved_checksum = harness.state().editor.checksum();
    assert_eq!(saved_checksum, mission_checksum);
    drop(harness);

    let mut reopened = fixture.harness(&fixture.physics_scene, EditorGame::PhysGame);
    reopened.run_steps(4);
    menu_click(&mut reopened, "File", "Open mesh scene…");
    type_scene_path(&mut reopened, &fixture.scene, "OK");
    wait_for(&mut reopened, "saved mission scene reopen", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && app.editor.checksum() == saved_checksum
            && app.editor.path().as_deref().is_some_and(|path| {
                path == fixture.scene.as_path()
                    || path.canonicalize().ok() == fixture.scene.canonicalize().ok()
            })
    });
    assert_eq!(fs::read(&fixture.scene).unwrap(), saved_scene_bytes);
    let reopened_view = mission_view(reopened.state()).expect("enabled mission survived reopen");
    assert_eq!(reopened_view.config.graph_revision, graph_revision);
    assert_eq!(reopened_view.config.count, 2);
    assert_eq!(reopened_view.run.collected, 0);
    assert_eq!(reopened_view.run.won, 0);
    if reopened
        .query_by_label("Mission collectible count")
        .is_none()
    {
        click(&mut reopened, "Mesh mission");
    }
    let (_, reopened_viewport_size, reopened_viewport) = capture_world_pair(
        &mut reopened,
        "fresh-mission-reopen.png",
        "fresh-mission-reopen-viewport.png",
    );
    assert_eq!(reopened_viewport_size, configured_viewport_size);
    let reopen_marker_pixel_diff = changed_pixel_count(&configured_viewport, &reopened_viewport);
    assert!(
        reopen_marker_pixel_diff < 20,
        "saved mission markers changed across reopen: {reopen_marker_pixel_diff}"
    );

    click_transport(&mut reopened, LBL_PLAY);
    wait_for(&mut reopened, "mission Play", |app| {
        app.editor
            .timeline()
            .is_some_and(|timeline| timeline.playing)
    });
    click_transport(&mut reopened, LBL_PAUSE);
    for _ in 0..64 {
        let current = mission_view(reopened.state()).unwrap();
        if current.run.collected == 0b11 {
            break;
        }
        assert_eq!(
            current.run.won, 0,
            "the exit cannot win before all collectibles"
        );
        click_transport(&mut reopened, LBL_STEP);
    }
    let collected_view = mission_view(reopened.state()).unwrap();
    assert_eq!(
        collected_view.run.collected, 0b11,
        "both collectibles must be reached by actual simulation steps"
    );
    assert_eq!(
        collected_view.run.won, 0,
        "collecting the objectives must not fake an exit win"
    );
    assert_eq!(collected_view.config.graph_revision, graph_revision);
    let collected_pixels = composite_pixels(&mut reopened, "all-objectives-collected.png");

    for _ in 0..64 {
        let current = mission_view(reopened.state()).unwrap();
        let status = mesh_view(reopened.state()).agent.status;
        if current.run.won == 1 && status == NavigationStatus::Arrived {
            break;
        }
        click_transport(&mut reopened, LBL_STEP);
    }
    let won_view = mission_view(reopened.state()).unwrap();
    assert_eq!(won_view.run.collected, 0b11);
    assert_eq!(
        won_view.run.won, 1,
        "mission victory requires the exit radius after collecting every objective"
    );
    assert_eq!(
        mesh_view(reopened.state()).agent.status,
        NavigationStatus::Arrived
    );
    assert!(reopened
        .query_by_label("Game mission in progress")
        .is_none());
    assert!(reopened.query_by_label("Game mission complete").is_some());
    let won_pixels = composite_pixels(&mut reopened, "mission-won.png");
    assert!(changed_pixel_count(&collected_pixels, &won_pixels) > 0);
    assert!(reopened.query_by_label("Mission complete").is_some());
    let win_tick = reopened.state().editor.timeline().unwrap().tick;
    assert!((1..=8192).contains(&win_tick));
    assert!(!reopened.state().editor.timeline().unwrap().playing);
    let win_frame_checksum = reopened
        .state()
        .editor
        .snapshot()
        .unwrap()
        .predicted()
        .checksum();
    reopened.state_mut().editor.seek(0);
    wait_for(&mut reopened, "mission timeline seek to tick zero", |app| {
        app.editor
            .timeline()
            .is_some_and(|timeline| timeline.tick == 0 && !timeline.playing)
            && app
                .editor
                .snapshot()
                .is_some_and(|snapshot| snapshot.predicted().tick() == 0)
    });
    let reset_view = mission_view(reopened.state()).unwrap();
    assert_eq!(reset_view.run.collected, 0);
    assert_eq!(reset_view.run.won, 0);
    let timeline_seek_zero_verified = true;
    reopened.state_mut().editor.seek(win_tick);
    wait_for(
        &mut reopened,
        "mission timeline replay to winning tick",
        |app| {
            app.editor
                .timeline()
                .is_some_and(|timeline| timeline.tick == win_tick && !timeline.playing)
                && app
                    .editor
                    .snapshot()
                    .is_some_and(|snapshot| snapshot.predicted().tick() == win_tick)
        },
    );
    let replayed_view = mission_view(reopened.state()).unwrap();
    assert_eq!(replayed_view.run.collected, 0b11);
    assert_eq!(replayed_view.run.won, 1);
    let replayed_frame_checksum = reopened
        .state()
        .editor
        .snapshot()
        .unwrap()
        .predicted()
        .checksum();
    assert_eq!(replayed_frame_checksum, win_frame_checksum);
    let timeline_seek_win_replay_verified = true;
    click_transport(&mut reopened, LBL_STOP);
    assert_eq!(reopened.state().editor.mode(), Mode::Edit);
    assert_eq!(reopened.state().editor.checksum(), saved_checksum);
    let stopped_view = mission_view(reopened.state()).unwrap();
    assert_eq!(stopped_view.run.collected, 0);
    assert_eq!(stopped_view.run.won, 0);
    let scene_checksum_unchanged = reopened.state().editor.checksum() == saved_checksum;
    assert!(scene_checksum_unchanged);
    assert_eq!(fs::read(&fixture.scene).unwrap(), saved_scene_bytes);

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
    let exported_scene = fs::read(fixture.export.join(&scene_name)).unwrap();
    let manifest_bytes = fs::read(fixture.export.join("orr.mesh-bundle.json")).unwrap();
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let runtime_bytes = fs::read(fixture.export.join("orr_mesh_player.exe")).unwrap();
    assert_eq!(exported_scene, saved_scene_bytes);
    assert_eq!(manifest["scene"]["sha256"], sha256(&saved_scene_bytes));
    assert_eq!(manifest["runtime"]["sha256"], sha256(&runtime_bytes));
    assert_eq!(manifest["input_profile"], "mesh-click-to-move-v1");
    assert_eq!(manifest["game_profile"], "mesh-mission-v1");
    assert_eq!(manifest["checksum"], saved_checksum);
    assert_eq!(
        manifest["graph_revision"],
        serde_json::json!(graph_revision)
    );
    click(&mut reopened, "I confirm removing the mesh mission");
    click_button(&mut reopened, "Remove mesh mission");
    assert!(mission_view(reopened.state()).is_none());
    menu_click(&mut reopened, "Edit", "Undo Ctrl+Z");
    assert_eq!(mission_view(reopened.state()).unwrap().run.won, 0);
    assert_eq!(mission_view(reopened.state()).unwrap().config.enabled, 1);
    drop(reopened);

    let captures = [
        "no-mission.png",
        "no-mission-viewport.png",
        "mission-configured-edit.png",
        "mission-configured-viewport.png",
        "fresh-mission-reopen.png",
        "fresh-mission-reopen-viewport.png",
        "all-objectives-collected.png",
        "mission-won.png",
    ]
    .iter()
    .map(|name| capture_record(&fixture.captures, name))
    .collect::<Vec<_>>();
    let receipt = serde_json::json!({
        "artifact_root":fixture.root,
        "scene_path":fixture.scene,
        "scene_sha256":sha256(&saved_scene_bytes),
        "scene_checksum":format!("0x{saved_checksum:016x}"),
        "graph_revision":graph_revision.iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
        "collectible_count":2,
        "viewport_picks_staged_without_host_mutation":true,
        "invalid_pick_and_escape_preserved_draft":true,
        "viewport_picks_preserved_history":viewport_picks_preserved_history,
        "picked_collectible_raw":picked_collectible.map(|coordinate|coordinate.raw()),
        "picked_exit_raw":picked_exit.map(|coordinate|coordinate.raw()),
        "distance_per_tick":"0.25",
        "collected_mask":won_view.run.collected,
        "won":won_view.run.won == 1,
        "win_tick":win_tick,
        "win_frame_checksum":format!("0x{win_frame_checksum:016x}"),
        "timeline_seek_zero_verified":timeline_seek_zero_verified,
        "timeline_seek_win_replay_verified":timeline_seek_win_replay_verified,
        "viewport_marker_pixel_diff":marker_pixel_diff,
        "reopen_marker_pixel_diff":reopen_marker_pixel_diff,
        "fresh_reopen_markers_match":reopen_marker_pixel_diff < 20,
        "scene_checksum_unchanged_after_stop":scene_checksum_unchanged,
        "configured_pixel_diff":configured_pixel_diff,
        "collected_to_won_pixel_diff":changed_pixel_count(&collected_pixels, &won_pixels),
        "export_directory":fixture.export,
        "export_files":export_names,
        "export_manifest_sha256":sha256(&manifest_bytes),
        "export_runtime_sha256":sha256(&runtime_bytes),
        "export_input_profile":manifest["input_profile"],
        "export_game_profile":manifest["game_profile"],
        "physical_os_input":false,
        "captures":captures
    });
    fs::write(
        fixture.root.join("acceptance.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
}
