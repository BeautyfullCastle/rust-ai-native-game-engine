#![cfg(all(windows, target_arch = "x86_64", feature = "mesh-navigation"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::{accesskit::Role, Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::{kittest::Queryable, Harness};
use egui_wgpu::{wgpu, WgpuSetupCreateNew};
use orr_editor::{
    app::{LBL_PLAY, LBL_STOP},
    game::EditorGame,
    Editor, EditorApp, HostSpec,
};
use orr_fp::FP;
use orr_navigation::NavigationStatus;
use orr_remote::mesh_navigation::mesh_from_view;
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
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
    physics_scene: PathBuf,
    scene: PathBuf,
    captures: PathBuf,
    runtime: PathBuf,
    export_directory: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        assert_eq!(
            std::env::var("ORR_MESH_PLAY_INPUT_ACCEPTANCE").as_deref(),
            Ok("1")
        );
        assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
        assert_eq!(std::env::var("WGPU_BACKEND").unwrap_or_default(), "vulkan");
        let temporary = tempfile::tempdir().unwrap();
        let root = PathBuf::from(
            std::env::var_os("ORR_MESH_PLAY_INPUT_CAPTURE_DIR")
                .expect("set a fresh ORR_MESH_PLAY_INPUT_CAPTURE_DIR"),
        );
        assert!(root.is_absolute());
        fs::create_dir(&root).expect("capture directory must be fresh");
        let root = root.canonicalize().unwrap();
        let captures = root.join("captures");
        fs::create_dir(&captures).unwrap();
        let physics_scene = root.join("owned-stock-physics.scene.yaml");
        fs::copy(EditorGame::PhysGame.default_scene_path(), &physics_scene).unwrap();
        let scene = root.join("interactive-mesh.scene.yaml");
        assert!(!scene.exists());
        let runtime = PathBuf::from(
            std::env::var_os("ORR_MESH_PLAY_INPUT_RUNTIME")
                .expect("set the freshly archived mesh player runtime"),
        );
        assert!(runtime.is_absolute());
        assert!(runtime.is_file());
        let export_directory = PathBuf::from(
            std::env::var_os("ORR_MESH_PLAY_INPUT_EXPORT_DIR")
                .expect("set a fresh sibling export directory"),
        );
        assert!(export_directory.is_absolute());
        assert!(!export_directory.exists());
        let export_parent = export_directory.parent().unwrap().canonicalize().unwrap();
        assert_eq!(Some(export_parent.as_path()), root.parent());
        Self {
            _temporary: temporary,
            root,
            physics_scene,
            scene,
            captures,
            runtime,
            export_directory,
        }
    }

    fn harness(&self) -> Harness<'static, EditorApp> {
        let mut editor = Editor::start(&HostSpec::local_game(
            self.physics_scene.clone(),
            EditorGame::PhysGame,
        ))
        .unwrap();
        editor.sync();
        Harness::builder()
            .with_size([1600.0, 1100.0])
            .with_step_dt(1.0 / 60.0)
            .wgpu_setup(hardware_vulkan_setup())
            .build_eframe(move |creation_context| {
                EditorApp::new(editor, creation_context.wgpu_render_state.clone())
            })
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

fn settle(harness: &mut Harness<'_, EditorApp>) {
    for _ in 0..4 {
        harness.state_mut().editor.sync();
        harness.run_steps(2);
    }
}

fn click(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness.get_by_label(label).scroll_to_me();
    harness.run_steps(20);
    harness.get_by_label(label).click_accesskit();
    settle(harness);
}

fn menu_click(harness: &mut Harness<'_, EditorApp>, menu: &str, item: &str) {
    if harness.query_by_label(item).is_none() {
        harness.get_by_label(menu).click_accesskit();
        harness.run_steps(2);
    }
    assert!(
        harness.query_by_label(item).is_some(),
        "missing menu item {item}"
    );
    harness.get_by_label(item).click_accesskit();
    settle(harness);
}

fn type_field(harness: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    harness.run_steps(20);
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .focus();
    harness.run_steps(2);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    harness.run_steps(2);
}

fn type_scene_path(harness: &mut Harness<'_, EditorApp>, path: &Path, action: &str) {
    type_field(harness, "Scene path", &path.display().to_string());
    click(harness, action);
}

fn wait_for(
    harness: &mut Harness<'_, EditorApp>,
    reason: &str,
    ready: impl Fn(&EditorApp) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        harness.state_mut().editor.sync();
        harness.run_steps(1);
        if ready(harness.state()) {
            settle(harness);
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("timed out waiting for {reason}");
}

fn wait_for_label(harness: &mut Harness<'_, EditorApp>, reason: &str, label: &str) {
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        harness.state_mut().editor.sync();
        harness.run_steps(1);
        if harness.query_by_label(label).is_some() {
            settle(harness);
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("timed out waiting for {reason}: {label}");
}

fn wait_for_label_contains(harness: &mut Harness<'_, EditorApp>, reason: &str, label: &str) {
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        harness.state_mut().editor.sync();
        harness.run_steps(1);
        if harness.query_by_label_contains(label).is_some() {
            settle(harness);
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("timed out waiting for {reason}: {label}");
}

fn path_matches(actual: Option<&Path>, expected: &Path) -> bool {
    actual.is_some_and(|actual| {
        actual == expected || actual.canonicalize().ok() == expected.canonicalize().ok()
    })
}

fn mesh_view(harness: &Harness<'_, EditorApp>) -> orr_remote::mesh_navigation::MeshView {
    let snapshot = harness
        .state()
        .editor
        .snapshot()
        .expect("MeshNavigation host has a current snapshot");
    mesh_from_view(snapshot.predicted()).expect("admitted mesh scene decodes")
}

fn world_point_for_height(graph: &orr_navigation::mesh::StaticMeshGraph, y_raw: i64) -> [f32; 3] {
    let triangle = graph
        .triangles()
        .iter()
        .find(|triangle| {
            triangle
                .vertices
                .iter()
                .all(|index| graph.vertices()[*index as usize][1].raw() == y_raw)
        })
        .expect("created ramp and bridge have a triangle on the requested floor");
    let raw_sum: [i128; 3] = std::array::from_fn(|axis| {
        triangle
            .vertices
            .iter()
            .map(|index| i128::from(graph.vertices()[*index as usize][axis].raw()))
            .sum()
    });
    raw_sum.map(|coordinate| FP::from_raw((coordinate / 3) as i64).to_f32())
}

fn point_for_world(harness: &Harness<'_, EditorApp>, world: [f32; 3]) -> Pos2 {
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
        .expect("MeshNavigation has an admitted presentation camera");
    let pixel = camera
        .world_to_screen(world, size)
        .expect("walkable point projects into the viewport");
    let points_per_pixel = viewport.width() / size.0 as f32;
    let pointer =
        viewport.min + egui::vec2(pixel[0] * points_per_pixel, pixel[1] * points_per_pixel);
    assert!(viewport.contains(pointer));
    pointer
}

fn click_viewport(harness: &mut Harness<'_, EditorApp>, point: Pos2) {
    harness.event(Event::PointerMoved(point));
    harness.run_steps(2);
    harness.event(Event::PointerButton {
        pos: point,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.step();
    harness.event(Event::PointerButton {
        pos: point,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    settle(harness);
}

fn viewport_pixels(harness: &Harness<'_, EditorApp>) -> (u32, u32, Vec<u8>) {
    let target = harness
        .state()
        .viewport3d_gpu()
        .expect("actual MeshNavigation viewport")
        .gpu();
    let size = target.target().size();
    (size.0, size.1, target.read_rgba8())
}

fn changed_pixel_count(left: &[u8], right: &[u8]) -> usize {
    assert_eq!(left.len(), right.len());
    left.as_chunks::<4>().0.iter()
        .zip(right.as_chunks::<4>().0.iter())
        .filter(|(first, second)| first != second)
        .count()
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn save_capture(
    directory: &Path,
    filename: &str,
    harness: &Harness<'_, EditorApp>,
) -> serde_json::Value {
    let (width, height, pixels) = viewport_pixels(harness);
    let path = directory.join(filename);
    orr_editor::app::write_png(
        &path,
        &egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &pixels),
    )
    .unwrap();
    let bytes = fs::read(path).unwrap();
    serde_json::json!({"name":filename,"bytes":bytes.len(),"sha256":sha256(&bytes)})
}

#[test]
#[ignore = "requires ORR_MESH_PLAY_INPUT_ACCEPTANCE=1 and a hardware Vulkan adapter"]
fn mesh_play_goal_input_retargets_only_through_managed_gameplay() {
    let preflight = Wgpu::headless(WgpuOptions::default()).expect("GPU preflight");
    assert!(
        !preflight.is_software(),
        "software adapter is not acceptance evidence"
    );
    eprintln!("Mesh Play input adapter: {}", preflight.adapter_name());
    drop(preflight);

    let fixture = Fixture::new();
    let mut harness = fixture.harness();
    settle(&mut harness);
    menu_click(&mut harness, "File", "New mesh scene…");
    type_scene_path(&mut harness, &fixture.scene, "Create");
    wait_for(&mut harness, "creating MeshNavigation scene", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && path_matches(app.editor.path().as_deref(), &fixture.scene)
    });
    click(&mut harness, "Static-mesh navigation");
    click(&mut harness, "Create ramp and bridge");
    click(&mut harness, "Place mesh agent");
    menu_click(&mut harness, "File", "Save Ctrl+S");
    wait_for(&mut harness, "saving the created mesh scene", |app| {
        !app.editor.is_dirty()
    });
    let saved_scene = fs::read(&fixture.scene).unwrap();
    let original_checksum = harness.state().editor.checksum();
    let original_view = mesh_view(&harness);
    assert_eq!(original_view.agent.status, NavigationStatus::Moving);
    let original_goal = original_view.pin.goal;
    let before_pixels = viewport_pixels(&harness).2;
    let saved_history_len = harness.state().editor.history().entries.len();
    let saved_capture = save_capture(&fixture.captures, "saved-mesh.png", &harness);

    click(&mut harness, LBL_PLAY);
    wait_for(&mut harness, "starting realtime Mesh Play", |app| {
        app.editor.mode() == orr_editor::Mode::Play && app.editor.sim().playing
    });
    click(&mut harness, "Enable mesh goal input");
    wait_for_label(
        &mut harness,
        "managed Mesh Play input claim",
        "Managed player input active",
    );

    let graph = mesh_view(&harness).graph;
    let target_world = world_point_for_height(&graph, 0);
    let target_pixel = point_for_world(&harness, target_world);
    let before_click_position = mesh_view(&harness).agent.position;
    let before_click_pixels = viewport_pixels(&harness).2;
    click_viewport(&mut harness, target_pixel);
    wait_for(
        &mut harness,
        "managed goal input to reach the MeshStep host",
        |app| {
            mesh_from_view(app.editor.snapshot().unwrap().predicted())
                .is_ok_and(|view| view.pin.goal != original_goal)
        },
    );
    let changed_goal_view = mesh_view(&harness);
    assert_ne!(changed_goal_view.pin.goal, original_goal);
    assert_eq!(
        changed_goal_view.graph.revision(),
        original_view.graph.revision()
    );
    let agent_moved_during_play = changed_goal_view.agent.position != before_click_position
        && changed_goal_view.agent.position != changed_goal_view.agent.spec.start;
    assert!(agent_moved_during_play);
    assert!(changed_goal_view.agent.distance > FP::ZERO);
    assert_eq!(harness.state().editor.mode(), orr_editor::Mode::Play);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        saved_history_len
    );
    let changed_pixels = viewport_pixels(&harness).2;
    let accepted_goal_diff = changed_pixel_count(&before_click_pixels, &changed_pixels);
    assert!(
        accepted_goal_diff > 0,
        "accepted gameplay goal did not change the rendered frame"
    );
    let live_goal_capture = save_capture(&fixture.captures, "live-goal.png", &harness);

    let old_goal = changed_goal_view.pin.goal;
    let viewport = harness.state().ui.viewport_rect.unwrap();
    let sky_click = Pos2::new(viewport.max.x - 4.0, viewport.min.y + 4.0);
    click_viewport(&mut harness, sky_click);
    assert_eq!(mesh_view(&harness).pin.goal, old_goal);
    assert!(harness
        .query_by_label("No walkable surface was hit; the current route is unchanged.")
        .is_some());
    let invalid_click_preserved_goal = true;

    click(&mut harness, "Disable mesh goal input");
    click(&mut harness, LBL_STOP);
    wait_for(&mut harness, "stopping Mesh Play", |app| {
        app.editor.mode() == orr_editor::Mode::Edit
    });
    assert_eq!(harness.state().editor.checksum(), original_checksum);

    assert_eq!(fs::read(&fixture.scene).unwrap(), saved_scene);
    let stopped_pixels = viewport_pixels(&harness).2;
    let stop_restored_pixels = changed_pixel_count(&before_pixels, &stopped_pixels) == 0;
    assert!(stop_restored_pixels);

    menu_click(&mut harness, "File", "Open mesh scene…");
    type_scene_path(&mut harness, &fixture.scene, "OK");
    wait_for(
        &mut harness,
        "reopening saved MeshNavigation scene",
        |app| {
            app.editor.game() == EditorGame::MeshNavigation
                && path_matches(app.editor.path().as_deref(), &fixture.scene)
                && !app.editor.is_dirty()
        },
    );
    let reopened = mesh_view(&harness);
    assert_eq!(reopened.pin.goal, original_goal);
    assert_eq!(reopened.graph.revision(), original_view.graph.revision());
    assert_eq!(harness.state().editor.checksum(), original_checksum);
    let reopened_pixels = viewport_pixels(&harness).2;
    let reopen_pixels_match = changed_pixel_count(&before_pixels, &reopened_pixels) == 0;
    assert!(reopen_pixels_match);
    let reopened_capture = save_capture(&fixture.captures, "reopened-mesh.png", &harness);

    click(&mut harness, LBL_PLAY);
    wait_for(&mut harness, "replaying after scene reopen", |app| {
        app.editor.mode() == orr_editor::Mode::Play && app.editor.sim().playing
    });
    click(&mut harness, "Enable mesh goal input");
    wait_for_label(
        &mut harness,
        "second managed input claim",
        "Managed player input active",
    );
    let second_goal_world = world_point_for_height(&mesh_view(&harness).graph, 0);
    let second_goal_pixel = point_for_world(&harness, second_goal_world);
    click_viewport(&mut harness, second_goal_pixel);
    wait_for(&mut harness, "second managed goal update", |app| {
        mesh_from_view(app.editor.snapshot().unwrap().predicted())
            .is_ok_and(|view| view.pin.goal != original_goal)
    });
    let repeated_click_goal_changed = mesh_view(&harness).pin.goal != original_goal;
    assert!(repeated_click_goal_changed);
    click(&mut harness, "Disable mesh goal input");
    click(&mut harness, LBL_STOP);
    wait_for(&mut harness, "stopping reopened Mesh Play", |app| {
        app.editor.mode() == orr_editor::Mode::Edit
    });
    assert_eq!(harness.state().editor.checksum(), original_checksum);

    menu_click(&mut harness, "File", "Export mesh game…");
    type_field(
        &mut harness,
        "Mesh player executable",
        &fixture.runtime.display().to_string(),
    );
    type_field(
        &mut harness,
        "Mesh game output directory",
        &fixture.export_directory.display().to_string(),
    );
    click(&mut harness, "I trust this local mesh player executable");
    harness
        .get_by_role_and_label(Role::Button, "Export mesh game")
        .click_accesskit();
    settle(&mut harness);
    wait_for_label_contains(
        &mut harness,
        "interactive MeshNavigation bundle export",
        "Mesh export complete:",
    );
    let bundle_directory = fixture.export_directory.canonicalize().unwrap();
    let manifest_path = bundle_directory.join("orr.mesh-bundle.json");
    let bundle_scene = bundle_directory.join(fixture.scene.file_name().unwrap());
    let bundle_runtime = bundle_directory.join("orr_mesh_player.exe");
    let manifest_bytes = fs::read(&manifest_path).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(manifest["input_profile"], "mesh-click-to-move-v1");
    assert_eq!(fs::read(&bundle_scene).unwrap(), saved_scene);
    assert_eq!(
        fs::read(&bundle_runtime).unwrap(),
        fs::read(&fixture.runtime).unwrap()
    );
    let expected_bundle_files = [
        "interactive-mesh.scene.yaml",
        "orr.mesh-bundle.json",
        "orr_mesh_player.exe",
    ];
    let mut actual_bundle_files = fs::read_dir(&bundle_directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    actual_bundle_files.sort();
    assert_eq!(actual_bundle_files, expected_bundle_files);
    let export_manifest_sha256 = sha256(&manifest_bytes);
    let export_scene_sha256 = sha256(&fs::read(&bundle_scene).unwrap());
    let export_runtime_sha256 = sha256(&fs::read(&bundle_runtime).unwrap());
    let export_receipt = serde_json::json!({
        "artifact_root":fixture.root,
        "bundle_directory":bundle_directory,
        "bundle_scene":bundle_scene,
        "manifest_path":manifest_path,
        "manifest_sha256":export_manifest_sha256,
        "runtime_source":fixture.runtime,
        "runtime_sha256":export_runtime_sha256,
        "input_profile":"mesh-click-to-move-v1",
        "scene_sha256":export_scene_sha256
    });

    let captures = vec![saved_capture, live_goal_capture, reopened_capture];
    let receipt = serde_json::json!({
        "scene_path":fixture.scene,
        "scene_sha256":sha256(&saved_scene),
        "scene_checksum":original_checksum,
        "graph_revision":hex(&original_view.graph.revision()),
        "original_goal_raw":original_goal.map(FP::raw),
        "accepted_goal_raw":changed_goal_view.pin.goal.map(FP::raw),
        "managed_goal_changed":changed_goal_view.pin.goal != original_goal,
        "agent_moved_during_play":agent_moved_during_play,
        "invalid_click_preserved_goal":invalid_click_preserved_goal,
        "input_was_managed":true,
        "no_edit_history_entry":true,
        "stop_restored_scene_and_pixels":stop_restored_pixels,
        "fresh_open_restored_saved_goal":reopen_pixels_match,
        "repeated_click_after_reopen":repeated_click_goal_changed,
        "accepted_goal_pixel_diff":accepted_goal_diff,
        "interactive_export":export_receipt,
        "captures":captures,
        "physical_os_input":false
    });
    fs::write(
        fixture.root.join("acceptance.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
