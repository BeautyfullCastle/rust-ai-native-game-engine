#![cfg(all(windows, target_arch = "x86_64", feature = "mesh-navigation"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::{accesskit::Role, Key, Modifiers};
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
use orr_navigation::{mesh::StaticMeshGraph, NavigationStatus};
use orr_remote::mesh_navigation::mesh_from_view;
use orr_rhi::Rhi;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    capture_directory: PathBuf,
    physics_scene: PathBuf,
    physics_bytes: Vec<u8>,
    scene: PathBuf,
    export: Option<ExportFixture>,
}

struct ExportFixture {
    output: PathBuf,
    runtime: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        assert_eq!(
            std::env::var("ORR_MESH_DRAFT_ACCEPTANCE").as_deref(),
            Ok("1"),
            "set ORR_MESH_DRAFT_ACCEPTANCE=1 to run the GPU acceptance"
        );
        assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
        assert_eq!(std::env::var("WGPU_BACKEND").unwrap_or_default(), "vulkan");
        let temporary = tempfile::tempdir().unwrap();
        let capture_directory = PathBuf::from(
            std::env::var_os("ORR_MESH_DRAFT_CAPTURE_DIR")
                .expect("set a fresh ORR_MESH_DRAFT_CAPTURE_DIR"),
        );
        assert!(capture_directory.is_absolute());
        fs::create_dir(&capture_directory).expect("capture directory must be fresh");
        let root = capture_directory.canonicalize().unwrap();
        let captures = root.join("captures");
        fs::create_dir(&captures).expect("capture inventory directory must be fresh");

        let physics_scene = root.join("owned-stock-physics.scene.yaml");
        fs::copy(EditorGame::PhysGame.default_scene_path(), &physics_scene).unwrap();
        let physics_bytes = fs::read(&physics_scene).unwrap();
        let scene_candidate = std::env::var_os("ORR_MESH_DRAFT_OUTPUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("authored-mesh.scene.yaml"));
        assert!(scene_candidate.is_absolute());
        assert!(scene_candidate.parent().is_some_and(Path::is_dir));
        let scene_parent = scene_candidate.parent().unwrap().canonicalize().unwrap();
        let scene = scene_parent.join(scene_candidate.file_name().unwrap());
        assert!(!scene.exists(), "mesh scene output must be absent");

        let export = match (
            std::env::var_os("ORR_MESH_DRAFT_EXPORT_OUTPUT"),
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
                Some(ExportFixture { output, runtime })
            }
            _ => panic!(
                "optional export requires both ORR_MESH_DRAFT_EXPORT_OUTPUT and ORR_MESH_EXPORT_RUNTIME"
            ),
        };
        Self {
            _temporary: temporary,
            root,
            capture_directory,
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

fn assert_hardware_gpu() {
    let gpu = orr_rhi::Wgpu::headless(orr_rhi::WgpuOptions::default())
        .expect("mesh geometry acceptance requires a GPU");
    assert!(
        !gpu.is_software(),
        "software rendering is not acceptance evidence"
    );
    eprintln!("Mesh geometry draft viewport GPU: {}", gpu.adapter_name());
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
        harness.get_by_label(menu).click_accesskit();
        harness.run_steps(2);
    }
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
    mesh_from_view(snapshot.predicted()).expect("admitted mesh pin decodes")
}

fn viewport_pixels(harness: &mut Harness<'_, EditorApp>, name: &str) -> Vec<u8> {
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
        .expect("production viewport has a GPU target");
    let size = viewport.gpu().target().size();
    assert_eq!(size, target, "GPU viewport dimensions must be stable");
    let pixels = viewport.gpu().read_rgba8();
    assert_eq!(pixels.len(), (size.0 * size.1 * 4) as usize);
    let image =
        egui::ColorImage::from_rgba_unmultiplied([size.0 as usize, size.1 as usize], &pixels);
    let captures = std::env::var_os("ORR_MESH_DRAFT_CAPTURE_DIR")
        .map(PathBuf::from)
        .unwrap()
        .join("captures");
    orr_editor::app::write_png(&captures.join(format!("{name}.png")), &image).unwrap();
    pixels
}

fn changed_pixel_count(left: &[u8], right: &[u8]) -> usize {
    assert_eq!(left.len(), right.len());
    left.as_chunks::<4>()
        .0
        .iter()
        .zip(right.as_chunks::<4>().0.iter())
        .filter(|(first, second)| {
            first[..3]
                .iter()
                .zip(second[..3].iter())
                .any(|(left_channel, right_channel)| left_channel.abs_diff(*right_channel) > 12)
        })
        .count()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn capture_inventory(directory: &Path) -> Vec<serde_json::Value> {
    let mut captures = fs::read_dir(directory.join("captures"))
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

fn assert_mesh_geometry(graph: &StaticMeshGraph) {
    let expected_vertices = BTreeSet::from([
        [-FP::SCALE / 4, 0, 0],
        [4 * FP::SCALE, 0, 0],
        [0, 0, 4 * FP::SCALE],
        [4 * FP::SCALE, 0, 4 * FP::SCALE],
        [0, 2 * FP::SCALE, 8 * FP::SCALE],
        [4 * FP::SCALE, 2 * FP::SCALE, 8 * FP::SCALE],
    ]);
    let referenced_vertices = graph
        .triangles()
        .iter()
        .flat_map(|triangle| triangle.vertices)
        .collect::<BTreeSet<_>>();
    let actual_referenced_vertices = referenced_vertices
        .iter()
        .map(|index| graph.vertices()[*index as usize].map(FP::raw))
        .collect::<BTreeSet<_>>();
    assert_eq!(actual_referenced_vertices, expected_vertices);
    assert_eq!(graph.triangles().len(), 4);
}

fn wait_for_playing(harness: &mut Harness<'_, EditorApp>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        harness.state_mut().editor.sync();
        harness.run_steps(1);
        if harness
            .state()
            .editor
            .timeline()
            .is_some_and(|timeline| timeline.playing)
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("MeshNavigation host did not publish a playing frame");
}

#[cfg(feature = "mesh-export")]
fn wait_for_label(harness: &mut Harness<'_, EditorApp>, label: &str, reason: &str) {
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
    wait_for_label(harness, "Mesh export complete:", "mesh game export");
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
    let manifest_path = export.output.join("orr.mesh-bundle.json");
    let manifest_bytes = fs::read(&manifest_path).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(
        manifest["scene"]["name"].as_str(),
        Some(scene_name.as_str())
    );
    assert!(manifest["sidecar"].is_null());
    assert!(manifest.get("props").is_none());
    assert!(manifest.get("camera").is_none());
    assert!(manifest.get("animation").is_none());
    let bundle_scene = fs::read(export.output.join(&scene_name)).unwrap();
    let runtime_bytes = fs::read(export.output.join("orr_mesh_player.exe")).unwrap();
    serde_json::json!({
        "directory": export.output,
        "manifest": manifest_path,
        "manifest_sha256": sha256_hex(&manifest_bytes),
        "scene": export.output.join(scene_name),
        "scene_sha256": sha256_hex(&bundle_scene),
        "runtime": export.output.join("orr_mesh_player.exe"),
        "runtime_sha256": sha256_hex(&runtime_bytes),
        "inventory": files,
        "no_external_art_sidecars": true,
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

fn stage_vertex_move(harness: &mut Harness<'_, EditorApp>) {
    for (label, value) in [
        ("Move vertex index", "0"),
        ("Move vertex X", "-0.25"),
        ("Move vertex Y", "0"),
        ("Move vertex Z", "0"),
    ] {
        type_field(harness, label, value);
    }
    click_button(harness, "Stage vertex move");
}

#[test]
#[ignore = "requires ORR_MESH_DRAFT_ACCEPTANCE=1 and a hardware Vulkan adapter"]
fn ui_previews_staged_geometry_with_local_history_then_commits() {
    assert_hardware_gpu();
    let fixture = Fixture::new();
    let mut harness = fixture.harness();
    harness.run_steps(4);
    assert_eq!(harness.state().editor.game(), EditorGame::PhysGame);
    assert!(harness.state().has_gpu());

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

    let initial_checksum = harness.state().editor.checksum();
    let initial_history_len = harness.state().editor.history().entries.len();
    let empty_host_pixels = viewport_pixels(&mut harness, "empty-host");

    stage_two_quads(&mut harness);
    assert!(harness
        .query_by_label_contains("PENDING GEOMETRY PREVIEW · host unchanged")
        .is_some());
    assert!(harness
        .get_by_label(LBL_PLAY)
        .accesskit_node()
        .is_disabled());
    click_button(&mut harness, "Frame staged geometry");
    let staged_pixels = viewport_pixels(&mut harness, "staged-two-quads");
    let preview_pixel_diff = changed_pixel_count(&empty_host_pixels, &staged_pixels);
    assert!(preview_pixel_diff > 20);
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        initial_history_len
    );

    stage_vertex_move(&mut harness);
    let moved_pixels = viewport_pixels(&mut harness, "staged-vertex-move");
    assert!(changed_pixel_count(&staged_pixels, &moved_pixels) > 10);
    click_button(&mut harness, "Undo staged geometry");
    let undo_pixels = viewport_pixels(&mut harness, "staged-local-undo");
    assert_eq!(undo_pixels, staged_pixels);
    click_button(&mut harness, "Redo staged geometry");
    let redo_pixels = viewport_pixels(&mut harness, "staged-local-redo");
    assert_eq!(redo_pixels, moved_pixels);
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        initial_history_len,
        "draft history never writes to ERP"
    );

    for (label, value) in [
        ("Move vertex index", "99"),
        ("Move vertex X", "1"),
        ("Move vertex Y", "1"),
        ("Move vertex Z", "1"),
    ] {
        type_field(&mut harness, label, value);
    }
    click_button(&mut harness, "Stage vertex move");
    assert!(harness
        .query_by_label_contains("vertex index is outside the current mesh")
        .is_some());
    assert!(harness
        .get_by_role_and_label(Role::Button, "Place mesh agent")
        .accesskit_node()
        .is_disabled());
    assert!(harness
        .get_by_label(LBL_PLAY)
        .accesskit_node()
        .is_disabled());
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        initial_history_len
    );
    click_button(&mut harness, "Reset geometry form");

    for (label, value) in [
        ("Quad origin X", "8"),
        ("Quad origin Y", "0"),
        ("Quad origin Z", "0"),
        ("Quad width", "0"),
        ("Quad depth", "4"),
        ("Quad rise", "0"),
    ] {
        type_field(&mut harness, label, value);
    }
    click_button(&mut harness, "Stage walkable quad");
    assert!(harness
        .query_by_label_contains("quad width and depth must be positive")
        .is_some());
    assert!(harness
        .get_by_role_and_label(Role::Button, "Place mesh agent")
        .accesskit_node()
        .is_disabled());
    assert!(harness
        .get_by_label(LBL_PLAY)
        .accesskit_node()
        .is_disabled());
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        initial_history_len
    );
    click_button(&mut harness, "Reset geometry form");

    click_button(&mut harness, "Discard unplaced changes");
    assert!(harness
        .query_by_label_contains("PENDING GEOMETRY PREVIEW · host unchanged")
        .is_none());
    let discarded_pixels = viewport_pixels(&mut harness, "discarded-draft");
    assert!(changed_pixel_count(&moved_pixels, &discarded_pixels) > 20);
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        initial_history_len
    );

    stage_two_quads(&mut harness);
    click_button(&mut harness, "Frame staged geometry");
    stage_vertex_move(&mut harness);
    let restaged_pixels = viewport_pixels(&mut harness, "restaged-geometry");
    assert_eq!(restaged_pixels, moved_pixels);
    assert!(harness
        .query_by_label_contains("PENDING GEOMETRY PREVIEW · host unchanged")
        .is_some());
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        initial_history_len
    );

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
    let placed_view = mesh_view(&harness);
    assert_mesh_geometry(&placed_view.graph);
    assert_eq!(placed_view.agent.status, NavigationStatus::Moving);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        initial_history_len + 1,
        "geometry and agent placement commit as one ERP history entry"
    );
    let placed_checksum = harness.state().editor.checksum();
    let authored_pixels = viewport_pixels(&mut harness, "committed-geometry");

    menu_click(&mut harness, "Edit", "Undo Ctrl+Z");
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    menu_click(&mut harness, "Edit", "Redo Ctrl+Y");
    assert_eq!(harness.state().editor.checksum(), placed_checksum);
    assert_mesh_geometry(&mesh_view(&harness).graph);
    menu_click(&mut harness, "File", "Save Ctrl+S");
    let saved_scene = fs::read(&fixture.scene).unwrap();
    let saved_scene_sha256 = sha256_hex(&saved_scene);
    let view_revision = placed_view.graph.revision();
    let view_vertices = placed_view
        .graph
        .vertices()
        .iter()
        .map(|vertex| vertex.map(FP::raw))
        .collect::<Vec<_>>();
    let view_faces = placed_view
        .graph
        .triangles()
        .iter()
        .map(|triangle| triangle.vertices)
        .collect::<Vec<_>>();
    drop(harness);

    let mut reopened = fixture.harness();
    reopened.run_steps(4);
    menu_click(&mut reopened, "File", "Open mesh scene…");
    type_scene_path(&mut reopened, &fixture.scene, "OK");
    wait_for(
        &mut reopened,
        "fresh Physics app open of authored scene",
        |app| {
            app.editor.game() == EditorGame::MeshNavigation
                && path_matches(app.editor.path().as_deref(), &fixture.scene)
                && app.editor.checksum() == placed_checksum
        },
    );
    assert_eq!(fs::read(&fixture.scene).unwrap(), saved_scene);
    let reopened_view = mesh_view(&reopened);
    assert_eq!(reopened_view.graph.revision(), view_revision);
    assert_eq!(
        reopened_view
            .graph
            .vertices()
            .iter()
            .map(|vertex| vertex.map(FP::raw))
            .collect::<Vec<_>>(),
        view_vertices
    );
    assert_eq!(
        reopened_view
            .graph
            .triangles()
            .iter()
            .map(|triangle| triangle.vertices)
            .collect::<Vec<_>>(),
        view_faces
    );
    let reopened_pixels = viewport_pixels(&mut reopened, "fresh-physics-reopen");
    let reopen_pixel_diff = changed_pixel_count(&authored_pixels, &reopened_pixels);
    assert!(reopen_pixel_diff < 20);

    click(&mut reopened, LBL_PLAY);
    wait_for_playing(&mut reopened);
    click(&mut reopened, LBL_PAUSE);
    click_button(&mut reopened, "\u{23EE}");
    wait_for(&mut reopened, "paused first simulation tick", |app| {
        app.editor
            .timeline()
            .is_some_and(|timeline| timeline.tick == 0 && !timeline.playing)
    });
    let starting_position = mesh_view(&reopened).agent.position;
    let play_start_pixels = viewport_pixels(&mut reopened, "play-tick-zero");
    for _ in 0..16 {
        click(&mut reopened, LBL_STEP);
    }
    let play_ticks = reopened.state().editor.timeline().unwrap().tick;
    assert_eq!(play_ticks, 16);
    let played_view = mesh_view(&reopened);
    assert_ne!(played_view.agent.position, starting_position);
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
    let play_pixels = viewport_pixels(&mut reopened, "play-tick-sixteen");
    let play_pixel_diff = changed_pixel_count(&play_start_pixels, &play_pixels);
    assert!(play_pixel_diff > 10);
    assert_eq!(fs::read(&fixture.scene).unwrap(), saved_scene);
    click(&mut reopened, LBL_STOP);
    assert_eq!(reopened.state().editor.mode(), Mode::Edit);
    assert_eq!(reopened.state().editor.checksum(), placed_checksum);

    let export = export_if_requested(&mut reopened, &fixture);
    assert_eq!(
        fs::read(&fixture.physics_scene).unwrap(),
        fixture.physics_bytes
    );
    let captures = capture_inventory(&fixture.capture_directory);
    assert_eq!(
        captures.len(),
        11,
        "eleven actual viewport captures are recorded"
    );
    let receipt = serde_json::json!({
        "artifact_root": fixture.root,
        "scene_path": fixture.scene,
        "scene_sha256": saved_scene_sha256,
        "scene_checksum": format!("0x{placed_checksum:016x}"),
        "graph_revision": view_revision,
        "graph_vertices_raw": view_vertices,
        "graph_faces": view_faces,
        "vertex_count": placed_view.graph.vertices().len(),
        "face_count": placed_view.graph.triangles().len(),
        "created_from_two_authored_quads": true,
        "staged_preview_visible": preview_pixel_diff > 20,
        "staged_preview_replaced_only_presentation": true,
        "staged_vertex_move_changed_pixels": changed_pixel_count(&staged_pixels, &moved_pixels) > 10,
        "staged_local_undo_redo_pixels_verified": undo_pixels == staged_pixels && redo_pixels == moved_pixels,
        "explicit_discard_cleared_unplaced_preview": changed_pixel_count(&moved_pixels, &discarded_pixels) > 20,
        "builtin_ramp_and_bridge_used": false,
        "invalid_vertex_and_degenerate_quad_preserved_staged_draft": true,
        "successful_staged_vertex_move": true,
        "staging_left_host_checksum_and_history_unchanged": true,
        "placement_undo_redo_verified": true,
        "fresh_physics_reopen_graph_matches": true,
        "fresh_reopen_pixel_diff": reopen_pixel_diff,
        "play_ticks": play_ticks,
        "play_moved": played_view.agent.position != starting_position,
        "play_pixel_diff": play_pixel_diff,
        "source_free_scope": "mesh graph is embedded in the saved scene; no external model package is used",
        "optional_bare_mesh_export": export,
        "captures": captures,
        "physical_os_input": false,
    });
    fs::write(
        fixture.capture_directory.join("acceptance.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
}
