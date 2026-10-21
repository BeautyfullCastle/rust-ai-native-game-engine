#![cfg(all(windows, target_arch = "x86_64", feature = "mesh-navigation"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::{accesskit::Role, Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use egui_wgpu::{wgpu, WgpuSetupCreateNew};
use orr_editor::{
    app::{LBL_PAUSE, LBL_PLAY, LBL_STEP, LBL_STOP},
    game::EditorGame,
    Editor, EditorApp, HostSpec, Mode,
};
use orr_fp::FP;
use orr_navigation::NavigationStatus;
use orr_remote::mesh_navigation::{mesh_from_view, MeshScenePin};
use orr_rhi::Rhi;
use sha2::{Digest, Sha256};
#[cfg(feature = "mesh-export")]
use std::collections::BTreeSet;
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
    physics_scene: PathBuf,
    physics_bytes: Vec<u8>,
    scene: PathBuf,
    export: Option<ExportFixture>,
}

struct ExportFixture {
    #[cfg(feature = "mesh-export")]
    output: PathBuf,
    #[cfg(feature = "mesh-export")]
    runtime: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        assert_eq!(
            std::env::var("ORR_MESH_VERTEX_VIEW_ACCEPTANCE").as_deref(),
            Ok("1"),
            "set ORR_MESH_VERTEX_VIEW_ACCEPTANCE=1 to run the GPU acceptance"
        );
        assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
        assert_eq!(std::env::var("WGPU_BACKEND").as_deref(), Ok("vulkan"));
        let temporary = tempfile::tempdir().unwrap();
        let capture_directory = PathBuf::from(
            std::env::var_os("ORR_MESH_VERTEX_VIEW_CAPTURE_DIR")
                .expect("set a fresh absolute GPU capture directory"),
        );
        assert!(capture_directory.is_absolute());
        fs::create_dir(&capture_directory).expect("capture directory must be fresh");
        let root = capture_directory.canonicalize().unwrap();
        let captures = root.join("captures");
        fs::create_dir(&captures).expect("capture inventory directory must be fresh");

        let physics_scene = root.join("owned-stock-physics.scene.yaml");
        fs::copy(EditorGame::PhysGame.default_scene_path(), &physics_scene).unwrap();
        let physics_bytes = fs::read(&physics_scene).unwrap();
        let scene_candidate = std::env::var_os("ORR_MESH_VERTEX_VIEW_OUTPUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("authored-mesh-vertex.scene.yaml"));
        assert!(scene_candidate.is_absolute());
        assert!(scene_candidate.parent().is_some_and(Path::is_dir));
        let scene_parent = scene_candidate.parent().unwrap().canonicalize().unwrap();
        let scene = scene_parent.join(scene_candidate.file_name().unwrap());
        assert!(!scene.exists(), "mesh scene output must be absent");

        let export = match (
            std::env::var_os("ORR_MESH_VERTEX_VIEW_EXPORT_OUTPUT"),
            std::env::var_os("ORR_MESH_EXPORT_RUNTIME"),
        ) {
            (None, None) => None,
            (Some(output), Some(runtime)) => {
                let output = PathBuf::from(output);
                let runtime = PathBuf::from(runtime);
                assert!(output.is_absolute());
                assert!(output.parent().is_some_and(Path::is_dir));
                assert!(!output.exists(), "mesh game output must be absent");
                assert!(runtime.is_absolute() && runtime.is_file());
                Some(ExportFixture {
                    #[cfg(feature = "mesh-export")]
                    output,
                    #[cfg(feature = "mesh-export")]
                    runtime,
                })
            }
            _ => panic!("optional export requires output and runtime paths"),
        };
        Self {
            _temporary: temporary,
            root,
            captures,
            physics_scene,
            physics_bytes,
            scene,
            export,
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

fn assert_hardware_gpu() -> String {
    let gpu = orr_rhi::Wgpu::headless(orr_rhi::WgpuOptions::default())
        .expect("vertex-view acceptance requires an available GPU");
    assert!(
        !gpu.is_software(),
        "software rendering is not GPU acceptance evidence"
    );
    let adapter = gpu.adapter_name().to_owned();
    eprintln!("Mesh vertex viewport GPU: {adapter}");
    adapter
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

fn click_button(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness
        .get_by_role_and_label(Role::Button, label)
        .scroll_to_me();
    harness.run_steps(20);
    harness
        .get_by_role_and_label(Role::Button, label)
        .click_accesskit();
    settle(harness);
}

fn menu_click(harness: &mut Harness<'_, EditorApp>, menu: &str, item: &str) {
    if harness.query_by_label(item).is_none() {
        click(harness, menu);
        harness.run_steps(2);
    }
    assert!(
        harness.query_by_label(item).is_some(),
        "missing menu item {item}"
    );
    click(harness, item);
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
    settle(harness);
}

fn type_scene_path(harness: &mut Harness<'_, EditorApp>, path: &Path, action: &str) {
    type_field(harness, "Scene path", &path.display().to_string());
    click(harness, action);
}

fn assert_global_save_and_history_disabled(harness: &mut Harness<'_, EditorApp>) {
    click(harness, "File");
    assert!(harness
        .get_by_label("Save Ctrl+S")
        .accesskit_node()
        .is_disabled());
    click(harness, "File");
    click(harness, "Edit");
    for label in ["Undo Ctrl+Z", "Redo Ctrl+Y"] {
        assert!(harness.get_by_label(label).accesskit_node().is_disabled());
    }
    click(harness, "Edit");
}

fn wait_for(
    harness: &mut Harness<'_, EditorApp>,
    operation: &str,
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
    panic!("timed out waiting for {operation}");
}

fn wait_for_playing(harness: &mut Harness<'_, EditorApp>) {
    wait_for(harness, "MeshNavigation Play", |app| {
        app.editor
            .timeline()
            .is_some_and(|timeline| timeline.playing)
    });
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
        .expect("current MeshNavigation Frame");
    mesh_from_view(snapshot.predicted()).expect("admitted mesh pin decodes")
}

fn changed_pixel_count(first: &[u8], second: &[u8]) -> usize {
    assert_eq!(first.len(), second.len());
    first
        .as_chunks::<4>()
        .0
        .iter()
        .zip(second.as_chunks::<4>().0.iter())
        .filter(|(first_pixel, second_pixel)| {
            first_pixel[..3]
                .iter()
                .zip(second_pixel[..3].iter())
                .any(|(first_channel, second_channel)| first_channel.abs_diff(*second_channel) > 12)
        })
        .count()
}

fn capture_png(harness: &mut Harness<'_, EditorApp>, fixture: &Fixture, name: &str) -> Vec<u8> {
    let target = (900, 850);
    for _ in 0..6 {
        settle(harness);
        let actual = harness
            .state()
            .viewport3d_gpu()
            .unwrap()
            .gpu()
            .target()
            .size();
        if actual == target {
            break;
        }
        let window = harness.ctx.viewport_rect().size();
        let pixels_per_point = harness.ctx.pixels_per_point();
        harness.set_size(egui::vec2(
            window.x + (target.0 as f32 - actual.0 as f32) / pixels_per_point,
            window.y + (target.1 as f32 - actual.1 as f32) / pixels_per_point,
        ));
    }
    settle(harness);
    let viewport = harness
        .state()
        .viewport3d_gpu()
        .expect("production viewport GPU target");
    let size = viewport.gpu().target().size();
    assert_eq!(size, target);
    let pixels = viewport.gpu().read_rgba8();
    assert_eq!(pixels.len(), (size.0 * size.1 * 4) as usize);
    let image =
        egui::ColorImage::from_rgba_unmultiplied([size.0 as usize, size.1 as usize], &pixels);
    orr_editor::app::write_png(&fixture.captures.join(format!("{name}.png")), &image).unwrap();
    pixels
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn revision_hex(revision: [u8; 32]) -> String {
    revision.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn capture_inventory(fixture: &Fixture) -> Vec<serde_json::Value> {
    let mut captures = fs::read_dir(&fixture.captures)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = fs::read(&path).unwrap();
            serde_json::json!({
                "name": path.file_name().unwrap().to_string_lossy(),
                "bytes": bytes.len(),
                "sha256": sha256_hex(&bytes),
            })
        })
        .collect::<Vec<_>>();
    captures.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
    captures
}

fn pointer_for_world(harness: &Harness<'_, EditorApp>, world: [f32; 3]) -> Pos2 {
    let viewport = harness
        .state()
        .ui
        .viewport_rect
        .expect("viewport screen rectangle available");
    let size = harness.state().ui.viewport_px;
    let camera = harness
        .state()
        .editor
        .presentation_camera3d(size)
        .expect("mesh viewport camera available");
    let pixel = camera
        .world_to_screen(world, size)
        .expect("vertex drag endpoint is in front of the camera");
    let points_per_pixel = viewport.width() / size.0 as f32;
    let point = viewport.min + egui::vec2(pixel[0] * points_per_pixel, pixel[1] * points_per_pixel);
    assert!(
        viewport.contains(point),
        "vertex screen point lies in viewport"
    );
    point
}

fn screen_pixel(harness: &Harness<'_, EditorApp>, pointer: Pos2) -> [f32; 2] {
    let viewport = harness.state().ui.viewport_rect.unwrap();
    let size = harness.state().ui.viewport_px;
    let points_per_pixel = viewport.width() / size.0 as f32;
    [
        (pointer.x - viewport.min.x) / points_per_pixel,
        (pointer.y - viewport.min.y) / points_per_pixel,
    ]
}

fn snapped_drag_target(
    harness: &Harness<'_, EditorApp>,
    start: Pos2,
    end: Pos2,
    original: [f32; 3],
) -> [f32; 3] {
    let size = harness.state().ui.viewport_px;
    let camera = harness
        .state()
        .editor
        .presentation_camera3d(size)
        .expect("mesh viewport camera available");
    let start_plane = camera
        .screen_to_plane_y(screen_pixel(harness, start), size, original[1])
        .expect("vertex drag starts on its horizontal plane");
    let end_plane = camera
        .screen_to_plane_y(screen_pixel(harness, end), size, original[1])
        .expect("vertex drag ends on its horizontal plane");
    std::array::from_fn(|coordinate_index| {
        let coordinate = original[coordinate_index] + end_plane[coordinate_index]
            - start_plane[coordinate_index];
        if coordinate_index == 1 {
            original[coordinate_index]
        } else {
            (coordinate * 4.0).round() / 4.0
        }
    })
}

fn drag_pointer_with_capture(
    harness: &mut Harness<'_, EditorApp>,
    fixture: &Fixture,
    start: Pos2,
    end: Pos2,
    capture_name: &str,
) -> Vec<u8> {
    harness.event(Event::PointerMoved(start));
    harness.run_steps(2);
    harness.event(Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.step();
    for step_index in 1..=5 {
        let fraction = step_index as f32 / 5.0;
        harness.event(Event::PointerMoved(start + (end - start) * fraction));
        harness.step();
    }
    capture_png(harness, fixture, capture_name)
}

fn release_pointer(harness: &mut Harness<'_, EditorApp>, end: Pos2) {
    harness.event(Event::PointerButton {
        pos: end,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    settle(harness);
}

fn stage_two_quads(harness: &mut Harness<'_, EditorApp>) {
    for (label, value) in [
        ("Maximum slope", "4"),
        ("Quad origin X", "0"),
        ("Quad origin Y", "0"),
        ("Quad origin Z", "0"),
        ("Quad width", "4"),
        ("Quad depth", "4"),
        ("Quad rise", "0"),
    ] {
        type_field(harness, label, value);
    }
    click_button(harness, "Stage walkable quad");
    for (label, value) in [
        ("Quad origin X", "0"),
        ("Quad origin Y", "0"),
        ("Quad origin Z", "4"),
        ("Quad width", "4"),
        ("Quad depth", "4"),
        ("Quad rise", "2"),
    ] {
        type_field(harness, label, value);
    }
    click_button(harness, "Stage walkable quad");
}

#[cfg(feature = "mesh-export")]
fn wait_for_export(harness: &mut Harness<'_, EditorApp>) {
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        harness.state_mut().editor.sync();
        harness.run_steps(1);
        if harness
            .query_by_label_contains("Mesh export complete:")
            .is_some()
        {
            settle(harness);
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("timed out waiting for mesh game export completion");
}

#[cfg(feature = "mesh-export")]
fn export_if_requested(
    harness: &mut Harness<'_, EditorApp>,
    fixture: &Fixture,
) -> serde_json::Value {
    let Some(export) = fixture.export.as_ref() else {
        return serde_json::Value::Null;
    };
    menu_click(harness, "File", "Export mesh game…");
    type_field(
        harness,
        "Mesh player executable",
        &export.runtime.display().to_string(),
    );
    type_field(
        harness,
        "Mesh game output directory",
        &export.output.display().to_string(),
    );
    click(harness, "I trust this local mesh player executable");
    click_button(harness, "Export mesh game");
    wait_for_export(harness);
    let files = fs::read_dir(&export.output)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<BTreeSet<_>>();
    let scene_name = fixture
        .scene
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        files,
        BTreeSet::from([
            scene_name.clone(),
            "orr.mesh-bundle.json".to_owned(),
            "orr_mesh_player.exe".to_owned(),
        ])
    );
    let manifest_bytes = fs::read(export.output.join("orr.mesh-bundle.json")).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(manifest["schema"], 1);
    assert_eq!(
        manifest["scene"]["name"].as_str(),
        Some(scene_name.as_str())
    );
    assert!(manifest["sidecar"].is_null());
    assert!(manifest.get("props").is_none());
    assert!(manifest.get("camera").is_none());
    assert!(manifest.get("animation").is_none());
    let scene_bytes = fs::read(export.output.join(&scene_name)).unwrap();
    let runtime_bytes = fs::read(export.output.join("orr_mesh_player.exe")).unwrap();
    serde_json::json!({
        "bundle_directory": export.output,
        "bundle_manifest_sha256": sha256_hex(&manifest_bytes),
        "bundle_scene_sha256": sha256_hex(&scene_bytes),
        "bundle_runtime_sha256": sha256_hex(&runtime_bytes),
        "exact_three_file_bundle": true,
    })
}

#[cfg(not(feature = "mesh-export"))]
fn export_if_requested(
    _harness: &mut Harness<'_, EditorApp>,
    fixture: &Fixture,
) -> serde_json::Value {
    assert!(
        fixture.export.is_none(),
        "export env requires mesh-export feature"
    );
    serde_json::Value::Null
}

#[test]
#[ignore = "requires ORR_MESH_VERTEX_VIEW_ACCEPTANCE=1 and hardware Vulkan"]
fn viewport_vertex_drag_is_transient_cancelable_snapped_and_persisted() {
    let adapter = assert_hardware_gpu();
    let fixture = Fixture::new();
    let mut harness = fixture.harness();
    harness.run_steps(4);
    assert!(harness.state().has_gpu());
    assert_eq!(harness.state().editor.game(), EditorGame::PhysGame);

    menu_click(&mut harness, "File", "New mesh scene…");
    type_scene_path(&mut harness, &fixture.scene, "Create");
    wait_for(&mut harness, "new mesh scene", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && path_matches(app.editor.path().as_deref(), &fixture.scene)
    });
    click(&mut harness, "Static-mesh navigation");
    if harness.query_by_label("Quad origin X").is_none() {
        click(&mut harness, "Mesh geometry");
    }
    stage_two_quads(&mut harness);
    click_button(&mut harness, "Frame staged geometry");
    let staged_pixels = capture_png(&mut harness, &fixture, "staged-before-vertex-drag");
    assert!(harness
        .query_by_label_contains("PENDING GEOMETRY PREVIEW · host unchanged")
        .is_some());
    let initial_checksum = harness.state().editor.checksum();
    let initial_history = harness.state().editor.history().clone();
    let empty_pin = *harness
        .state()
        .editor
        .snapshot()
        .unwrap()
        .predicted()
        .singleton::<MeshScenePin>();
    assert!(empty_pin.is_empty());

    click_button(&mut harness, "Edit mesh vertices in viewport");
    type_field(&mut harness, "Vertex grid snap", "0.25");
    assert_global_save_and_history_disabled(&mut harness);
    assert!(harness.query_by_label("Finish vertex editing").is_some());
    assert!(harness
        .get_by_label(LBL_PLAY)
        .accesskit_node()
        .is_disabled());
    let original_position = [0.0, 0.0, 0.0];
    let drag_start = pointer_for_world(&harness, original_position);
    let intended_target = [-0.5, 0.0, 0.0];
    let drag_end = pointer_for_world(&harness, intended_target);
    let cancel_preview = drag_pointer_with_capture(
        &mut harness,
        &fixture,
        drag_start,
        drag_end,
        "vertex-cancel-preview",
    );
    assert!(harness
        .query_by_label_contains("VERTEX DRAG PREVIEW · host unchanged")
        .is_some());
    assert!(changed_pixel_count(&staged_pixels, &cancel_preview) > 10);
    assert!(harness
        .get_by_label(LBL_PLAY)
        .accesskit_node()
        .is_disabled());
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    assert_eq!(harness.state().editor.history(), &initial_history);
    assert!(harness
        .state()
        .editor
        .snapshot()
        .unwrap()
        .predicted()
        .singleton::<MeshScenePin>()
        .is_empty());
    harness.key_press(Key::Escape);
    release_pointer(&mut harness, drag_end);
    let cancelled_pixels = capture_png(&mut harness, &fixture, "vertex-cancelled");
    assert_eq!(cancelled_pixels, staged_pixels);
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    assert_eq!(harness.state().editor.history(), &initial_history);

    click_button(&mut harness, "Edit mesh vertices in viewport");
    type_field(&mut harness, "Vertex grid snap", "0.25");
    let expected_position = snapped_drag_target(&harness, drag_start, drag_end, original_position);
    assert_ne!(expected_position, original_position);
    assert_eq!((expected_position[0] * 4.0).fract(), 0.0);
    assert_eq!((expected_position[2] * 4.0).fract(), 0.0);
    let committed_preview = drag_pointer_with_capture(
        &mut harness,
        &fixture,
        drag_start,
        drag_end,
        "vertex-release-preview",
    );
    assert!(changed_pixel_count(&staged_pixels, &committed_preview) > 10);
    release_pointer(&mut harness, drag_end);
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    assert_eq!(harness.state().editor.history(), &initial_history);
    let moved_pixels = capture_png(&mut harness, &fixture, "vertex-moved");
    assert!(changed_pixel_count(&staged_pixels, &moved_pixels) > 10);
    assert!(harness
        .query_by_label_contains("PENDING GEOMETRY PREVIEW · host unchanged")
        .is_some());
    assert!(harness.query_by_label("Finish vertex editing").is_some());
    assert!(harness
        .get_by_role_and_label(Role::Button, "Place mesh agent")
        .accesskit_node()
        .is_disabled());
    click_button(&mut harness, "Finish vertex editing");
    let preview_model = harness
        .state()
        .mesh_navigation
        .model()
        .expect("staged geometry exposes its production preview model");
    assert_eq!(
        preview_model.source().asset_id,
        "mesh/navigation/staged-geometry-preview.orrmodel"
    );
    assert!(
        preview_model
            .source()
            .primitives
            .iter()
            .flat_map(|primitive| primitive.vertices.iter())
            .any(|vertex| {
                (vertex.position[0] - expected_position[0]).abs() <= 1.0e-6
                    && (vertex.position[1] - expected_position[1]).abs() <= 1.0e-6
                    && (vertex.position[2] - expected_position[2]).abs() <= 1.0e-6
            }),
        "the displayed production mesh contains the snapped moved vertex"
    );
    let moved_pixels = capture_png(&mut harness, &fixture, "vertex-moved");
    assert!(changed_pixel_count(&staged_pixels, &moved_pixels) > 10);
    click_button(&mut harness, "Undo staged geometry");
    let undo_pixels = capture_png(&mut harness, &fixture, "vertex-local-undo");
    assert_eq!(undo_pixels, staged_pixels);
    click_button(&mut harness, "Redo staged geometry");
    let redo_pixels = capture_png(&mut harness, &fixture, "vertex-local-redo");
    assert_eq!(redo_pixels, moved_pixels);
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    assert_eq!(harness.state().editor.history(), &initial_history);

    for (label, value) in [
        ("Start X", "1"),
        ("Start Y", "0"),
        ("Start Z", "1"),
        ("Goal X", "3"),
        ("Goal Y", "1.5"),
        ("Goal Z", "7"),
        ("Maximum vertical projection", "2"),
        ("Distance per tick", "0.5"),
    ] {
        type_field(&mut harness, label, value);
    }
    click_button(&mut harness, "Place mesh agent");
    assert_eq!(
        harness.state().editor.history().entries.len(),
        initial_history.entries.len() + 1,
        "geometry and agent placement commit as one ERP history entry"
    );
    let placed_view = mesh_view(&harness);
    let expected_vertex = expected_position
        .map(|coordinate| FP::from_raw((f64::from(coordinate) * FP::SCALE as f64).round() as i64));
    assert_eq!(placed_view.graph.vertices()[0], expected_vertex);
    let moved_vertex = placed_view.graph.vertices()[0].map(FP::to_f32);
    assert_eq!(moved_vertex[1], original_position[1]);
    assert_eq!(placed_view.agent.status, NavigationStatus::Moving);
    assert!(placed_view.graph.triangles().len() >= 4);
    let placed_checksum = harness.state().editor.checksum();
    let committed_pixels = capture_png(&mut harness, &fixture, "committed-geometry");
    menu_click(&mut harness, "Edit", "Undo Ctrl+Z");
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    menu_click(&mut harness, "Edit", "Redo Ctrl+Y");
    assert_eq!(harness.state().editor.checksum(), placed_checksum);
    assert_eq!(
        mesh_view(&harness).graph.vertices()[0].map(FP::to_f32),
        moved_vertex
    );
    menu_click(&mut harness, "File", "Save Ctrl+S");
    let saved_scene = fs::read(&fixture.scene).unwrap();
    let scene_sha256 = sha256_hex(&saved_scene);
    let graph_revision = revision_hex(placed_view.graph.revision());
    drop(harness);

    let mut reopened = fixture.harness();
    reopened.run_steps(4);
    menu_click(&mut reopened, "File", "Open mesh scene…");
    type_scene_path(&mut reopened, &fixture.scene, "OK");
    wait_for(&mut reopened, "fresh Physics open", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && path_matches(app.editor.path().as_deref(), &fixture.scene)
            && app.editor.checksum() == placed_checksum
    });
    assert_eq!(fs::read(&fixture.scene).unwrap(), saved_scene);
    let reopened_view = mesh_view(&reopened);
    assert_eq!(reopened_view.graph.revision(), placed_view.graph.revision());
    assert_eq!(
        reopened_view.graph.vertices()[0].map(FP::to_f32),
        moved_vertex
    );
    let reopened_pixels = capture_png(&mut reopened, &fixture, "fresh-physics-reopen");
    assert!(changed_pixel_count(&committed_pixels, &reopened_pixels) < 20);

    click(&mut reopened, LBL_PLAY);
    wait_for_playing(&mut reopened);
    click(&mut reopened, LBL_PAUSE);
    click_button(&mut reopened, "\u{23ee}");
    wait_for(&mut reopened, "timeline reset to tick zero", |app| {
        app.editor
            .timeline()
            .is_some_and(|timeline| timeline.tick == 0 && !timeline.playing)
    });
    let start_position = mesh_view(&reopened).agent.position;
    let tick_zero_pixels = capture_png(&mut reopened, &fixture, "play-tick-zero");
    for _ in 0..16 {
        click(&mut reopened, LBL_STEP);
    }
    let timeline_tick = reopened.state().editor.timeline().unwrap().tick;
    assert_eq!(timeline_tick, 16);
    let played_view = mesh_view(&reopened);
    assert_ne!(played_view.agent.position, start_position);
    let path_length = played_view
        .graph
        .path_length(&played_view.agent.path)
        .unwrap();
    let expected_distance = FP::from_raw(path_length.raw().min(16 * (FP::SCALE / 2)));
    assert_eq!(played_view.agent.distance, expected_distance);
    assert_eq!(
        played_view.agent.status,
        if expected_distance == path_length {
            NavigationStatus::Arrived
        } else {
            NavigationStatus::Moving
        }
    );
    if played_view.agent.status == NavigationStatus::Arrived {
        assert_eq!(
            played_view.agent.position,
            *played_view.agent.path.points.last().unwrap()
        );
    }
    let play_pixels = capture_png(&mut reopened, &fixture, "play-tick-sixteen");
    let play_pixel_diff = changed_pixel_count(&tick_zero_pixels, &play_pixels);
    assert!(play_pixel_diff > 10);
    click(&mut reopened, LBL_STOP);
    assert_eq!(reopened.state().editor.mode(), Mode::Edit);
    assert_eq!(reopened.state().editor.checksum(), placed_checksum);

    let export = export_if_requested(&mut reopened, &fixture);
    assert_eq!(
        fs::read(&fixture.physics_scene).unwrap(),
        fixture.physics_bytes
    );
    let captures = capture_inventory(&fixture);
    assert_eq!(captures.len(), 11);
    let receipt = serde_json::json!({
        "artifact_root": fixture.root,
        "scene_path": fixture.scene,
        "scene_sha256": scene_sha256,
        "scene_checksum": placed_checksum,
        "graph_revision": graph_revision,
        "hardware_vulkan_adapter": adapter,
        "physics_bootstrap_restored": sha256_hex(&fixture.physics_bytes),
        "vertex_index": 0,
        "original_vertex": original_position,
        "saved_vertex": moved_vertex,
        "snap_verified": true,
        "horizontal_plane_preserved_y": moved_vertex[1] == original_position[1],
        "transient_drag_changes_pixels": changed_pixel_count(&staged_pixels, &cancel_preview) > 10,
        "escape_cancel_restores_pixels": cancelled_pixels == staged_pixels,
        "release_creates_reversible_local_geometry_edit": undo_pixels == staged_pixels && redo_pixels == moved_pixels,
        "host_unchanged_until_explicit_agent_place": true,
        "fresh_physics_reopen_preserves_vertex": reopened_view.graph.vertices()[0].map(FP::to_f32) == moved_vertex,
        "play_tick": timeline_tick,
        "play_moves_agent_and_changes_pixels": played_view.agent.position != start_position && play_pixel_diff > 10,
        "physical_os_input": false,
        "source_free_scope": "walkable graph is embedded in the saved scene; the test does not claim native runtime execution",
        "optional_export": export,
        "captures": captures,
    });
    fs::write(
        fixture.root.join("acceptance.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
}
