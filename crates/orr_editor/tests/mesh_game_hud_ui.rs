#![cfg(all(
    windows,
    target_arch = "x86_64",
    feature = "mesh-navigation",
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
use orr_navigation::NavigationStatus;
use orr_remote::mesh_navigation::mesh_from_view;
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
            std::env::var("ORR_MESH_GAME_HUD_ACCEPTANCE").as_deref(),
            Ok("1")
        );
        assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
        assert_eq!(std::env::var("WGPU_BACKEND").unwrap_or_default(), "vulkan");
        let temporary = tempfile::tempdir().unwrap();
        let root = PathBuf::from(
            std::env::var_os("ORR_MESH_GAME_HUD_CAPTURE_DIR")
                .expect("set a fresh HUD capture root"),
        );
        assert!(root.is_absolute());
        fs::create_dir(&root).expect("HUD capture root must be fresh");
        let root = root.canonicalize().unwrap();
        let captures = root.join("captures");
        fs::create_dir(&captures).unwrap();
        let scene = PathBuf::from(
            std::env::var_os("ORR_MESH_GAME_HUD_OUTPUT")
                .unwrap_or_else(|| root.join("mesh-game-hud.scene.yaml").into_os_string()),
        );
        assert!(scene.is_absolute() && !scene.exists());
        let scene_parent = scene.parent().unwrap().canonicalize().unwrap();
        let scene = scene_parent.join(scene.file_name().unwrap());
        let physics_scene = root.join("owned-stock-physics.scene.yaml");
        fs::copy(EditorGame::PhysGame.default_scene_path(), &physics_scene).unwrap();
        let runtime = PathBuf::from(
            std::env::var_os("ORR_MESH_EXPORT_RUNTIME")
                .expect("HUD export requires the matching mesh player"),
        );
        assert!(runtime.is_absolute() && runtime.is_file());
        let export = PathBuf::from(
            std::env::var_os("ORR_MESH_GAME_HUD_EXPORT_OUTPUT")
                .expect("HUD acceptance requires an export directory"),
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
                "Mesh game HUD GPU preflight: {} (software: {})",
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
                std::env::var_os("ORR_MESH_GAME_HUD_ACCEPTANCE").is_none()
                    && std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "required HUD GPU acceptance unavailable: {error}"
            );
            eprintln!("SKIP: HUD GPU unavailable: {error}");
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

fn seek_comparison_tick(harness: &mut Harness<'_, EditorApp>, tick: u64) {
    for _ in 0..16 {
        if harness.state().editor.timeline().unwrap().tick >= tick {
            break;
        }
        click_transport(harness, LBL_STEP);
    }
    assert!(harness.state().editor.timeline().unwrap().tick >= tick);
    harness.state_mut().editor.seek(tick);
    wait_for(harness, "the recorded HUD comparison tick", |app| {
        app.editor
            .timeline()
            .is_some_and(|timeline| timeline.tick == tick)
            && app
                .editor
                .snapshot()
                .is_some_and(|snapshot| snapshot.predicted().tick() == tick)
    });
}

fn mesh_view(app: &EditorApp) -> orr_remote::mesh_navigation::MeshView {
    mesh_from_view(app.editor.snapshot().unwrap().predicted()).unwrap()
}

fn composite_pixels(harness: &mut Harness<'_, EditorApp>, filename: &str) -> Vec<u8> {
    let viewport = harness
        .state()
        .viewport3d_gpu()
        .expect("production App viewport GPU is initialized");
    eprintln!("Mesh game HUD App GPU: {}", viewport.gpu().adapter_name());
    let frame = harness
        .render()
        .expect("render complete production App frame");
    let size = [frame.width() as usize, frame.height() as usize];
    let pixels = frame.as_raw().clone();
    let image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
    orr_editor::app::write_png(&fixture_capture_path(filename), &image).unwrap();
    pixels
}

fn fixture_capture_path(filename: &str) -> PathBuf {
    PathBuf::from(std::env::var_os("ORR_MESH_GAME_HUD_CAPTURE_DIR").unwrap())
        .join("captures")
        .join(filename)
}

fn changed_pixel_count(left: &[u8], right: &[u8]) -> usize {
    assert_eq!(left.len(), right.len());
    left.chunks_exact(4)
        .zip(right.chunks_exact(4))
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

fn sidecar_path(scene: &Path) -> PathBuf {
    let mut name = scene.as_os_str().to_os_string();
    name.push(".mesh-hud.json");
    PathBuf::from(name)
}

fn capture_record(root: &Path, name: &str) -> Value {
    let bytes = fs::read(root.join(name)).unwrap();
    serde_json::json!({"name":name,"bytes":bytes.len(),"sha256":sha256(&bytes)})
}

#[test]
#[ignore = "requires ORR_MESH_GAME_HUD_ACCEPTANCE=1 and a hardware Vulkan adapter"]
fn mesh_game_hud_applies_renders_saves_reopens_and_exports() {
    let requested = std::env::var_os("ORR_MESH_GAME_HUD_ACCEPTANCE").is_some()
        || std::env::var_os("ORR_REQUIRE_GPU").is_some();
    if !requested {
        eprintln!("SKIP: opt in to actual Mesh game HUD GPU acceptance");
        return;
    }
    assert_eq!(
        std::env::var("ORR_MESH_GAME_HUD_ACCEPTANCE").as_deref(),
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
    click_button(&mut harness, "Create ramp and bridge");
    click_button(&mut harness, "Place mesh agent");
    menu_click(&mut harness, "File", "Save Ctrl+S");
    let scene_bytes = fs::read(&fixture.scene).unwrap();
    let scene_checksum = harness.state().editor.checksum();
    let graph_revision = mesh_view(harness.state()).graph.revision();
    let hud_sidecar = sidecar_path(&fixture.scene);
    assert!(!hud_sidecar.exists());

    click_transport(&mut harness, LBL_PLAY);
    wait_for(&mut harness, "baseline mesh Play", |app| {
        app.editor
            .timeline()
            .is_some_and(|timeline| timeline.playing)
    });
    click_transport(&mut harness, LBL_PAUSE);
    seek_comparison_tick(&mut harness, 3);
    let baseline_tick = harness.state().editor.timeline().unwrap().tick;
    let baseline_play_checksum = harness.state().editor.checksum();
    let baseline_view = mesh_view(harness.state());
    assert_eq!(baseline_view.agent.status, NavigationStatus::Moving);
    let no_hud_pixels = composite_pixels(&mut harness, "no-hud-baseline-tick-three.png");
    click_transport(&mut harness, LBL_STOP);
    assert_eq!(harness.state().editor.mode(), Mode::Edit);
    assert_eq!(harness.state().editor.checksum(), scene_checksum);

    if harness.query_by_label("HUD title").is_none() {
        click(&mut harness, "Mesh game HUD");
    }
    type_field(&mut harness, "HUD title", "THE GLASS HARBOR");
    type_field(&mut harness, "HUD idle text", "Waiting at the harbor");
    type_field(&mut harness, "HUD moving text", "Crossing the bridge");
    type_field(&mut harness, "HUD arrived text", "The far shore is safe");
    type_field(&mut harness, "HUD font size", "27");
    click(&mut harness, "Show mesh game HUD");
    click(&mut harness, "Show mesh game HUD");
    click(&mut harness, "Show HUD tick");
    click(&mut harness, "Show HUD tick");
    click_button(&mut harness, "Apply game HUD");
    assert!(
        harness
            .get_by_role_and_label(Role::Button, LBL_PLAY)
            .accesskit_node()
            .is_disabled(),
        "unsaved HUD presentation must block Play"
    );
    click_button(&mut harness, "Undo HUD edit");
    assert!(!hud_sidecar.exists());
    click_button(&mut harness, "Redo HUD edit");
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    click_button(&mut harness, "Save game HUD");
    let hud_bytes = fs::read(&hud_sidecar).expect("explicitly saved HUD sidecar");
    let hud_document: Value = serde_json::from_slice(&hud_bytes).unwrap();
    assert_eq!(hud_document["schema"], 1);
    assert_eq!(hud_document["marker"], "ORR_MESH_HUD_V1");
    assert_eq!(
        hud_document["scene_filename"],
        fixture
            .scene
            .file_name()
            .unwrap()
            .to_string_lossy()
            .as_ref()
    );
    assert_eq!(
        hud_document["graph_revision"],
        serde_json::json!(graph_revision)
    );
    assert_eq!(hud_document["hud"]["enabled"], true);
    assert_eq!(hud_document["hud"]["title"], "THE GLASS HARBOR");
    assert_eq!(hud_document["hud"]["idle_text"], "Waiting at the harbor");
    assert_eq!(hud_document["hud"]["moving_text"], "Crossing the bridge");
    assert_eq!(hud_document["hud"]["arrived_text"], "The far shore is safe");
    assert_eq!(hud_document["hud"]["show_tick"], true);
    assert_eq!(hud_document["hud"]["font_size"].as_f64(), Some(27.0));
    assert_eq!(harness.state().editor.checksum(), scene_checksum);

    menu_click(&mut harness, "File", "Save Ctrl+S");
    drop(harness);
    let mut reopened = fixture.harness(&fixture.physics_scene, EditorGame::PhysGame);
    reopened.run_steps(4);
    menu_click(&mut reopened, "File", "Open mesh scene…");
    type_scene_path(&mut reopened, &fixture.scene, "OK");
    wait_for(&mut reopened, "saved HUD scene reopen", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && app.editor.checksum() == scene_checksum
            && app.editor.path().as_deref().is_some_and(|path| {
                path == fixture.scene.as_path()
                    || path.canonicalize().ok() == fixture.scene.canonicalize().ok()
            })
    });
    assert_eq!(fs::read(&fixture.scene).unwrap(), scene_bytes);
    assert_eq!(fs::read(&hud_sidecar).unwrap(), hud_bytes);
    composite_pixels(&mut reopened, "fresh-hud-reopen-edit.png");

    click_transport(&mut reopened, LBL_PLAY);
    wait_for(&mut reopened, "HUD Play", |app| {
        app.editor
            .timeline()
            .is_some_and(|timeline| timeline.playing)
    });
    click_transport(&mut reopened, LBL_PAUSE);
    seek_comparison_tick(&mut reopened, baseline_tick);
    let hud_tick = reopened.state().editor.timeline().unwrap().tick;
    assert_eq!(hud_tick, baseline_tick);
    assert_eq!(
        mesh_view(reopened.state()).agent.status,
        NavigationStatus::Moving
    );
    let moving_hud_pixels = composite_pixels(&mut reopened, "hud-moving-tick-three.png");
    let hud_pixel_diff = changed_pixel_count(&no_hud_pixels, &moving_hud_pixels);
    assert!(
        hud_pixel_diff > 40,
        "full composed App frame must show HUD pixels: {hud_pixel_diff}"
    );
    assert_eq!(reopened.state().editor.checksum(), baseline_play_checksum);
    click_transport(&mut reopened, LBL_PLAY);
    wait_for(&mut reopened, "agent arrival for HUD state", |app| {
        mesh_view(app).agent.status == NavigationStatus::Arrived
    });
    let arrived_hud_pixels = composite_pixels(&mut reopened, "hud-arrived.png");
    assert!(changed_pixel_count(&moving_hud_pixels, &arrived_hud_pixels) > 10);
    let arrival_tick = reopened.state().editor.timeline().unwrap().tick;
    assert!((1..=8192).contains(&arrival_tick));
    click_transport(&mut reopened, LBL_STOP);
    assert_eq!(reopened.state().editor.mode(), Mode::Edit);
    assert_eq!(reopened.state().editor.checksum(), scene_checksum);
    assert_eq!(fs::read(&fixture.scene).unwrap(), scene_bytes);
    assert_eq!(fs::read(&hud_sidecar).unwrap(), hud_bytes);

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
    let hud_name = format!("{scene_name}.mesh-hud.json");
    let mut export_names = fs::read_dir(&fixture.export)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    export_names.sort();
    let mut expected_names = vec![
        "orr.mesh-bundle.json".to_owned(),
        "orr_mesh_player.exe".to_owned(),
        scene_name.clone(),
        hud_name.clone(),
    ];
    expected_names.sort();
    assert_eq!(export_names, expected_names);
    let exported_scene = fs::read(fixture.export.join(&scene_name)).unwrap();
    let exported_hud = fs::read(fixture.export.join(&hud_name)).unwrap();
    let manifest_bytes = fs::read(fixture.export.join("orr.mesh-bundle.json")).unwrap();
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let runtime_bytes = fs::read(fixture.export.join("orr_mesh_player.exe")).unwrap();
    assert_eq!(exported_scene, scene_bytes);
    assert_eq!(exported_hud, hud_bytes);
    assert_eq!(manifest["schema"], 1);
    assert_eq!(manifest["scene"]["name"], scene_name);
    assert_eq!(manifest["scene"]["bytes"], scene_bytes.len() as u64);
    assert_eq!(manifest["scene"]["sha256"], sha256(&scene_bytes));
    assert_eq!(manifest["hud"]["name"], hud_name);
    assert_eq!(manifest["hud"]["bytes"], hud_bytes.len() as u64);
    assert_eq!(manifest["hud"]["sha256"], sha256(&hud_bytes));
    assert_eq!(manifest["runtime"]["name"], "orr_mesh_player.exe");
    assert_eq!(manifest["runtime"]["bytes"], runtime_bytes.len() as u64);
    assert_eq!(manifest["runtime"]["sha256"], sha256(&runtime_bytes));
    assert_eq!(
        manifest["graph_revision"],
        serde_json::json!(graph_revision)
    );
    assert_eq!(manifest["checksum"], scene_checksum);
    assert_eq!(manifest["input_profile"], "mesh-click-to-move-v1");
    drop(reopened);

    let captures = [
        "no-hud-baseline-tick-three.png",
        "fresh-hud-reopen-edit.png",
        "hud-moving-tick-three.png",
        "hud-arrived.png",
    ]
    .iter()
    .map(|name| capture_record(&fixture.captures, name))
    .collect::<Vec<_>>();
    let receipt = serde_json::json!({
        "artifact_root":fixture.root,
        "scene_path":fixture.scene,
        "scene_sha256":sha256(&scene_bytes),
        "scene_checksum":format!("0x{scene_checksum:016x}"),
        "hud_sidecar_path":hud_sidecar,
        "hud_sidecar_sha256":sha256(&hud_bytes),
        "hud_title":hud_document["hud"]["title"],
        "hud_idle_text":hud_document["hud"]["idle_text"],
        "hud_moving_text":hud_document["hud"]["moving_text"],
        "hud_arrived_text":hud_document["hud"]["arrived_text"],
        "hud_show_tick":hud_document["hud"]["show_tick"],
        "tick_comparison":hud_tick,
        "arrival_tick":arrival_tick,
        "scene_checksum_unchanged":true,
        "fresh_reopen_pixels_recorded":true,
        "hud_composite_pixel_diff":hud_pixel_diff,
        "agent_arrived":true,
        "export_directory":fixture.export,
        "export_files":export_names,
        "export_manifest_sha256":sha256(&manifest_bytes),
        "export_runtime_sha256":sha256(&runtime_bytes),
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
