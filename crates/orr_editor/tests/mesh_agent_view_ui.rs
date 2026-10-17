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
use orr_navigation::{mesh::StaticMeshGraph, NavigationStatus};
use orr_remote::mesh_navigation::mesh_from_view;
use orr_rhi::Rhi;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

const MODEL_ASSET: &str = "foreground.glb";
const LICENSE_ASSET: &str = "LICENSE.txt";

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    physics_scene: PathBuf,
    physics_bytes: Vec<u8>,
    scene: PathBuf,
    capture_directory: PathBuf,
    package: PathBuf,
    hidden_package: Option<PathBuf>,
    export: Option<ExportFixture>,
}

struct ExportFixture {
    runtime: PathBuf,
    output: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        assert_eq!(
            std::env::var("ORR_MESH_AGENT_VIEW_ACCEPTANCE").as_deref(),
            Ok("1"),
            "set ORR_MESH_AGENT_VIEW_ACCEPTANCE=1 to run the GPU acceptance test"
        );
        assert_eq!(
            std::env::var("ORR_REQUIRE_GPU").as_deref(),
            Ok("1"),
            "GPU acceptance must fail closed when no hardware adapter exists"
        );
        assert_eq!(
            std::env::var("WGPU_BACKEND").unwrap_or_default(),
            "vulkan",
            "the acceptance harness requires Vulkan"
        );

        let temporary = tempfile::tempdir().unwrap();
        let capture_directory = std::env::var_os("ORR_MESH_AGENT_VIEW_CAPTURE_DIR")
            .map(PathBuf::from)
            .expect("set a fresh ORR_MESH_AGENT_VIEW_CAPTURE_DIR");
        assert!(capture_directory.is_absolute());
        fs::create_dir(&capture_directory).expect("capture directory must be fresh");
        let root = capture_directory.canonicalize().unwrap();
        let package = root.with_file_name(format!(
            "{}-rotation-model-package",
            root.file_name().unwrap().to_string_lossy()
        ));
        assert!(!package.exists(), "model package path must be fresh");
        let source_package = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/imported_scene_demo")
            .canonicalize()
            .expect("checked-in model package exists");
        copy_tree(&source_package, &package).expect("copy model package into owned sibling");
        assert!(package.join(MODEL_ASSET).is_file());
        assert!(package.join(LICENSE_ASSET).is_file());
        let physics_scene = root.join("owned-stock-physics.scene.yaml");
        fs::copy(EditorGame::PhysGame.default_scene_path(), &physics_scene).unwrap();
        let physics_bytes = fs::read(&physics_scene).unwrap();
        let scene = std::env::var_os("ORR_MESH_AGENT_VIEW_OUTPUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("mesh-agent-view.scene.yaml"));
        assert!(scene.is_absolute(), "scene output must be absolute");
        assert!(scene.parent().is_some_and(Path::is_dir));
        assert!(!scene.exists(), "scene output must be absent");

        let export = match (
            std::env::var_os("ORR_MESH_AGENT_VIEW_EXPORT_OUTPUT"),
            std::env::var_os("ORR_MESH_EXPORT_RUNTIME"),
        ) {
            (None, None) => None,
            (Some(output), Some(runtime)) => {
                let output = PathBuf::from(output);
                let runtime = PathBuf::from(runtime);
                assert!(output.is_absolute());
                assert!(output.parent().is_some_and(Path::is_dir));
                assert!(!output.exists(), "mesh export output must be absent");
                assert!(runtime.is_absolute() && runtime.is_file());
                Some(ExportFixture { runtime, output })
            }
            _ => panic!(
                "optional export requires both ORR_MESH_AGENT_VIEW_EXPORT_OUTPUT and ORR_MESH_EXPORT_RUNTIME"
            ),
        };
        Self {
            _temporary: temporary,
            root,
            physics_scene,
            physics_bytes,
            scene,
            capture_directory,
            package,
            hidden_package: None,
            export,
        }
    }

    fn physics_harness(&self) -> Harness<'static, EditorApp> {
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

    fn hide_package(&mut self) -> PathBuf {
        let hidden = self.package.with_file_name(format!(
            "{}-hidden",
            self.package.file_name().unwrap().to_string_lossy()
        ));
        assert!(!hidden.exists(), "hidden package path must be fresh");
        fs::rename(&self.package, &hidden).expect("hide source package");
        self.hidden_package = Some(hidden.clone());
        hidden
    }

    fn restore_package(&mut self) {
        if let Some(hidden) = self.hidden_package.take() {
            assert!(
                !self.package.exists(),
                "package restore target was replaced"
            );
            fs::rename(hidden, &self.package).expect("restore source package");
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(hidden) = self.hidden_package.take() {
            if !self.package.exists() && hidden.exists() {
                if let Err(error) = fs::rename(hidden, &self.package) {
                    eprintln!("Could not restore hidden rotation package: {error}");
                }
            }
        }
    }
}

fn copy_tree(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = destination.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), target)?;
        } else {
            return Err(std::io::Error::other(
                "fixture package contains a non-file entry",
            ));
        }
    }
    Ok(())
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
    match orr_rhi::Wgpu::headless(orr_rhi::WgpuOptions::default()) {
        Ok(rhi) => {
            assert!(!rhi.is_software(), "software rendering is not acceptance");
            eprintln!("Mesh agent viewport GPU: {}", rhi.adapter_name());
        }
        Err(error) => panic!("required Mesh agent GPU acceptance unavailable: {error}"),
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
    harness.run_steps(20);
    harness.get_by_label(label).click_accesskit();
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

fn world_point_for_height(graph: &StaticMeshGraph, y_raw: i64) -> [f32; 3] {
    let triangle = graph
        .triangles()
        .iter()
        .find(|triangle| {
            triangle
                .vertices
                .iter()
                .all(|index| graph.vertices()[*index as usize][1].raw() == y_raw)
        })
        .expect("ramp-and-bridge graph has a triangle on each requested floor");
    let raw_sum: [i128; 3] = std::array::from_fn(|axis| {
        triangle
            .vertices
            .iter()
            .map(|index| i128::from(graph.vertices()[*index as usize][axis].raw()))
            .sum()
    });
    raw_sum.map(|coordinate| FP::from_raw((coordinate / 3) as i64).to_f32())
}

fn assert_point_near(actual: [FP; 3], expected: [f32; 3]) {
    for (actual_coordinate, expected_coordinate) in actual.into_iter().zip(expected) {
        assert!(
            (actual_coordinate.to_f32() - expected_coordinate).abs() <= 0.04,
            "picked surface coordinate should remain near the clicked triangle interior"
        );
    }
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
        .expect("triangle centroid projects into the camera");
    let points_per_pixel = viewport.width() / size.0 as f32;
    let pointer =
        viewport.min + egui::vec2(pixel[0] * points_per_pixel, pixel[1] * points_per_pixel);
    assert!(
        viewport.contains(pointer),
        "projected triangle point is visible"
    );
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
    if !pixels.is_empty() {
        let image =
            egui::ColorImage::from_rgba_unmultiplied([size.0 as usize, size.1 as usize], &pixels);
        let directory = std::env::var_os("ORR_MESH_AGENT_VIEW_CAPTURE_DIR")
            .map(PathBuf::from)
            .unwrap();
        orr_editor::app::write_png(&directory.join(format!("{name}.png")), &image).unwrap();
    }
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
                .any(|(first_channel, second_channel)| first_channel.abs_diff(*second_channel) > 12)
        })
        .count()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
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
    harness
        .get_by_role_and_label(Role::Button, "Export mesh game")
        .click_accesskit();
    settle(harness);
    wait_for_label(harness, "Mesh export complete:", "mesh export completion");
    let manifest_path = export.output.join("orr.mesh-bundle.json");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    let scene_copy = export.output.join(fixture.scene.file_name().unwrap());
    let sidecar_name = format!(
        "{}.mesh-art.json",
        fixture.scene.file_name().unwrap().to_string_lossy()
    );
    let props_name = format!(
        "{}.mesh-props.json",
        fixture.scene.file_name().unwrap().to_string_lossy()
    );
    let sidecar_copy = export.output.join(&sidecar_name);
    let props_copy = export.output.join(&props_name);
    let runtime_copy = export.output.join("orr_mesh_player.exe");
    assert!(
        scene_copy.is_file()
            && sidecar_copy.is_file()
            && props_copy.is_file()
            && runtime_copy.is_file()
    );
    assert_eq!(
        fs::read_dir(&export.output).unwrap().count(),
        5,
        "rotation bundle contains scene, both sidecars, player, and manifest"
    );
    assert_eq!(manifest["sidecar"]["name"], sidecar_name);
    assert_eq!(manifest["props"]["name"], props_name);
    assert_eq!(
        manifest["scene"]["name"].as_str(),
        fixture.scene.file_name().unwrap().to_str()
    );
    assert_eq!(manifest["runtime"]["name"], "orr_mesh_player.exe");
    assert_eq!(
        fs::read(&scene_copy).unwrap(),
        fs::read(&fixture.scene).unwrap()
    );
    assert_eq!(
        fs::read(&runtime_copy).unwrap(),
        fs::read(&export.runtime).unwrap()
    );
    let sidecar_bytes = fs::read(&sidecar_copy).unwrap();
    let props_bytes = fs::read(&props_copy).unwrap();
    assert_eq!(
        sidecar_bytes,
        fs::read(format!("{}.mesh-art.json", fixture.scene.display())).unwrap()
    );
    assert_eq!(
        props_bytes,
        fs::read(format!("{}.mesh-props.json", fixture.scene.display())).unwrap()
    );
    for (record, path) in [
        (&manifest["scene"], &scene_copy),
        (&manifest["sidecar"], &sidecar_copy),
        (&manifest["props"], &props_copy),
        (&manifest["runtime"], &runtime_copy),
    ] {
        let bytes = fs::read(path).unwrap();
        assert_eq!(record["bytes"].as_u64(), Some(bytes.len() as u64));
        assert_eq!(record["sha256"].as_str(), Some(sha256_hex(&bytes).as_str()));
    }
    serde_json::json!({
        "bundle_path": export.output,
        "manifest": manifest,
        "scene_sha256": sha256_hex(&fs::read(scene_copy).unwrap()),
        "sidecar_sha256": sha256_hex(&sidecar_bytes),
        "props_sha256": sha256_hex(&props_bytes),
        "runtime_sha256": sha256_hex(&fs::read(runtime_copy).unwrap()),
    })
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
#[ignore = "requires ORR_MESH_AGENT_VIEW_ACCEPTANCE=1 and a hardware Vulkan adapter"]
fn viewport_agent_endpoint_picking_is_staged_then_placed_saved_reopened_and_played() {
    assert_hardware_gpu();
    let mut fixture = Fixture::new();
    let mut harness = fixture.physics_harness();
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
    click(&mut harness, "Create ramp and bridge");
    click(&mut harness, "Place mesh agent");

    let original_view = mesh_view(&harness);
    assert_eq!(original_view.agent.status, NavigationStatus::Moving);
    let original_pin = original_view.pin;
    let original_checksum = harness.state().editor.checksum();
    let original_history_len = harness.state().editor.history().entries.len();
    let graph = original_view.graph.clone();
    assert_eq!(graph.triangles().len(), 6);

    click(&mut harness, "Pick agent start in viewport");
    assert!(harness
        .get_by_label(LBL_PLAY)
        .accesskit_node()
        .is_disabled());
    assert!(harness
        .get_by_role_and_label(Role::Button, "Place mesh agent")
        .accesskit_node()
        .is_disabled());
    click(&mut harness, "Cancel endpoint picking");
    assert_eq!(mesh_view(&harness).pin, original_pin);
    assert_eq!(harness.state().editor.checksum(), original_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        original_history_len
    );

    let staged_start = world_point_for_height(&graph, 0);
    let staged_goal = world_point_for_height(&graph, 2 * FP::SCALE);
    click(&mut harness, "Pick agent start in viewport");
    click_viewport(&mut harness, staged_start);
    assert_eq!(mesh_view(&harness).pin, original_pin);
    assert_eq!(harness.state().editor.checksum(), original_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        original_history_len
    );
    click(&mut harness, "Pick agent goal in viewport");
    click_viewport(&mut harness, staged_goal);
    assert!(harness
        .get_by_label(LBL_PLAY)
        .accesskit_node()
        .is_disabled());
    assert_eq!(mesh_view(&harness).pin, original_pin);
    assert_eq!(harness.state().editor.checksum(), original_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        original_history_len
    );

    click(&mut harness, "Place mesh agent");
    let placed_view = mesh_view(&harness);
    assert_eq!(placed_view.graph.revision(), original_view.graph.revision());
    assert_point_near(placed_view.agent.spec.start, staged_start);
    assert_point_near(placed_view.agent.spec.goal, staged_goal);
    assert_eq!(placed_view.agent.status, NavigationStatus::Moving);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        original_history_len + 1,
        "explicit Place emits one authoritative ERP history entry"
    );
    let placed_checksum = harness.state().editor.checksum();
    let scene_before_cosmetics = fs::read(&fixture.scene).unwrap();
    let placed_without_art_pixels = viewport_pixels(&mut harness, "placed-without-art");

    if harness.query_by_label("Agent art offset X").is_none() {
        click(&mut harness, "Mesh agent art");
    }
    for (label, value) in [
        (
            "Art package source directory",
            fixture.package.display().to_string(),
        ),
        ("Art model asset", MODEL_ASSET.to_owned()),
        ("Art license asset", LICENSE_ASSET.to_owned()),
    ] {
        type_field(&mut harness, label, &value);
    }
    click(&mut harness, "I have reviewed rights to use this agent art");
    click(&mut harness, "Load agent art");
    click(&mut harness, "Apply agent art");
    let identity_agent_art_pixels = viewport_pixels(&mut harness, "identity-agent-art");
    assert!(changed_pixel_count(&placed_without_art_pixels, &identity_agent_art_pixels) > 20);
    for (label, value) in [
        ("Agent art rotation X (degrees)", "25"),
        ("Agent art rotation Y (degrees)", "35"),
        ("Agent art rotation Z (degrees)", "15"),
    ] {
        type_field(&mut harness, label, value);
    }
    click(&mut harness, "Apply agent art");
    assert_eq!(mesh_view(&harness).pin, placed_view.pin);
    assert_eq!(harness.state().editor.checksum(), placed_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        original_history_len + 1
    );
    let rotated_agent_art_pixels = viewport_pixels(&mut harness, "rotated-agent-art");
    assert!(changed_pixel_count(&identity_agent_art_pixels, &rotated_agent_art_pixels) > 20);
    click(&mut harness, "Undo agent art");
    let undone_agent_art_pixels = viewport_pixels(&mut harness, "undone-agent-art");
    assert!(changed_pixel_count(&identity_agent_art_pixels, &undone_agent_art_pixels) < 20);
    click(&mut harness, "Redo agent art");
    let redone_agent_art_pixels = viewport_pixels(&mut harness, "redone-agent-art");
    assert!(changed_pixel_count(&rotated_agent_art_pixels, &redone_agent_art_pixels) < 20);
    click(&mut harness, "Save agent art");

    if harness.query_by_label("Prop position X").is_none() {
        click(&mut harness, "Mesh props");
    }
    for (label, value) in [
        (
            "Prop package source directory",
            fixture.package.display().to_string(),
        ),
        ("Prop model asset", MODEL_ASSET.to_owned()),
        ("Prop license asset", LICENSE_ASSET.to_owned()),
    ] {
        type_field(&mut harness, label, &value);
    }
    click(&mut harness, "I have reviewed rights to use these props");
    click(&mut harness, "Load prop candidate");
    for (label, value) in [
        ("Prop name", "Rotated prop"),
        ("Prop position X", "3"),
        ("Prop position Y", "0.25"),
        ("Prop position Z", "1"),
        ("Prop scale", "0.5"),
    ] {
        type_field(&mut harness, label, value);
    }
    click(&mut harness, "Add prop");
    let identity_prop_pixels = viewport_pixels(&mut harness, "identity-prop");
    assert!(changed_pixel_count(&redone_agent_art_pixels, &identity_prop_pixels) > 20);
    for (label, value) in [
        ("Prop rotation X (degrees)", "20"),
        ("Prop rotation Y (degrees)", "40"),
        ("Prop rotation Z (degrees)", "30"),
    ] {
        type_field(&mut harness, label, value);
    }
    click(&mut harness, "Update selected prop");
    assert_eq!(mesh_view(&harness).pin, placed_view.pin);
    assert_eq!(harness.state().editor.checksum(), placed_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        original_history_len + 1
    );
    let rotated_prop_pixels = viewport_pixels(&mut harness, "rotated-prop");
    assert!(changed_pixel_count(&identity_prop_pixels, &rotated_prop_pixels) > 20);
    click(&mut harness, "Undo prop edit");
    let undone_prop_pixels = viewport_pixels(&mut harness, "undone-prop");
    assert!(changed_pixel_count(&identity_prop_pixels, &undone_prop_pixels) < 20);
    click(&mut harness, "Redo prop edit");
    let redone_prop_pixels = viewport_pixels(&mut harness, "redone-prop");
    assert!(changed_pixel_count(&rotated_prop_pixels, &redone_prop_pixels) < 20);
    click(&mut harness, "Save props");

    let art_sidecar = PathBuf::from(format!("{}.mesh-art.json", fixture.scene.display()));
    let props_sidecar = PathBuf::from(format!("{}.mesh-props.json", fixture.scene.display()));
    let art_bytes = fs::read(&art_sidecar).expect("agent-art sidecar saved by its UI");
    let props_bytes = fs::read(&props_sidecar).expect("props sidecar saved by its UI");
    let model_package_sha256 = sha256_hex(&fs::read(fixture.package.join(MODEL_ASSET)).unwrap());
    let license_sha256 = sha256_hex(&fs::read(fixture.package.join(LICENSE_ASSET)).unwrap());
    let package_manifest_sha256 =
        sha256_hex(&fs::read(fixture.package.join("orr.package.json")).unwrap());
    let art_document: serde_json::Value = serde_json::from_slice(&art_bytes).unwrap();
    let props_document: serde_json::Value = serde_json::from_slice(&props_bytes).unwrap();
    let agent_art_quaternion = art_document["rotation"].as_array().unwrap();
    let prop_quaternion = props_document["props"][0]["art"]["rotation"]
        .as_array()
        .unwrap();
    assert_eq!(agent_art_quaternion.len(), 4);
    assert_eq!(prop_quaternion.len(), 4);
    for quaternion in [agent_art_quaternion, prop_quaternion] {
        let squared_norm = quaternion
            .iter()
            .map(|component| component.as_f64().unwrap().powi(2))
            .sum::<f64>();
        assert!((squared_norm - 1.0).abs() < 1e-5);
    }
    let placed_pixels = viewport_pixels(&mut harness, "saved-art-and-props");
    assert_eq!(mesh_view(&harness).pin, placed_view.pin);
    let host_checksum_after_rotation = harness.state().editor.checksum();
    assert_eq!(host_checksum_after_rotation, placed_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        original_history_len + 1,
        "rotation undo/redo stays out of authoritative ECS history"
    );
    assert_eq!(
        fs::read(&fixture.scene).unwrap(),
        scene_before_cosmetics,
        "cosmetic sidecars do not mutate the saved mesh scene"
    );

    menu_click(&mut harness, "Edit", "Undo Ctrl+Z");
    assert_eq!(mesh_view(&harness).pin, original_pin);
    menu_click(&mut harness, "Edit", "Redo Ctrl+Y");
    assert_eq!(mesh_view(&harness).pin, placed_view.pin);
    assert_eq!(harness.state().editor.checksum(), placed_checksum);
    menu_click(&mut harness, "File", "Save Ctrl+S");
    let saved_scene = fs::read(&fixture.scene).unwrap();
    let saved_scene_sha256 = sha256_hex(&saved_scene);
    assert!(!saved_scene.is_empty());
    let hidden_package = fixture.hide_package();
    assert!(!fixture.package.exists());
    assert!(hidden_package.is_dir());
    let package_hidden_during_reopen = !fixture.package.exists();
    drop(harness);

    let mut reopened = fixture.physics_harness();
    reopened.run_steps(4);
    menu_click(&mut reopened, "File", "Open mesh scene…");
    type_scene_path(&mut reopened, &fixture.scene, "OK");
    wait_for(&mut reopened, "fresh Physics app mesh scene open", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && path_matches(app.editor.path().as_deref(), &fixture.scene)
            && app.editor.checksum() == placed_checksum
    });
    assert!(
        !fixture.package.exists(),
        "package source remains hidden on reopen"
    );
    let reopened_view = mesh_view(&reopened);
    assert_eq!(reopened_view.graph.revision(), placed_view.graph.revision());
    assert_eq!(reopened_view.agent.spec, placed_view.agent.spec);
    assert_eq!(reopened_view.pin, placed_view.pin);
    let reopened_pixels = viewport_pixels(&mut reopened, "fresh-physics-reopen");
    assert!(changed_pixel_count(&placed_pixels, &reopened_pixels) < 20);

    click(&mut reopened, LBL_PLAY);
    wait_for_playing(&mut reopened);
    click(&mut reopened, LBL_PAUSE);
    reopened.state_mut().editor.seek(0);
    settle(&mut reopened);
    let initial_position = mesh_view(&reopened).agent.position;
    let play_start_pixels = viewport_pixels(&mut reopened, "play-tick-zero");
    for _ in 0..16 {
        click(&mut reopened, LBL_STEP);
    }
    let play_ticks = reopened.state().editor.timeline().unwrap().tick;
    assert_eq!(play_ticks, 16);
    let play_view = mesh_view(&reopened);
    assert_ne!(play_view.agent.position, initial_position);
    assert_eq!(play_view.agent.status, NavigationStatus::Moving);
    let play_pixels = viewport_pixels(&mut reopened, "play-tick-sixteen");
    assert!(changed_pixel_count(&play_start_pixels, &play_pixels) > 10);
    assert_eq!(fs::read(&fixture.scene).unwrap(), saved_scene);
    assert_eq!(fs::read(&art_sidecar).unwrap(), art_bytes);
    assert_eq!(fs::read(&props_sidecar).unwrap(), props_bytes);
    click(&mut reopened, LBL_STOP);
    assert_eq!(reopened.state().editor.mode(), Mode::Edit);
    assert_eq!(reopened.state().editor.checksum(), placed_checksum);

    let export = export_if_requested(&mut reopened, &fixture);
    fixture.restore_package();
    assert!(fixture.package.join(MODEL_ASSET).is_file());
    assert_eq!(
        sha256_hex(&fs::read(fixture.package.join(MODEL_ASSET)).unwrap()),
        model_package_sha256
    );
    assert_eq!(
        sha256_hex(&fs::read(fixture.package.join(LICENSE_ASSET)).unwrap()),
        license_sha256
    );
    let rotation_receipt = serde_json::json!({
        "agent_art_rotation_degrees": [25.0, 35.0, 15.0],
        "agent_art_rotation_quaternion_xyzw": agent_art_quaternion,
        "agent_art_rotation_pixel_diff": changed_pixel_count(
            &identity_agent_art_pixels,
            &rotated_agent_art_pixels
        ),
        "agent_art_undo_pixels_match": changed_pixel_count(
            &identity_agent_art_pixels,
            &undone_agent_art_pixels
        ) < 20,
        "agent_art_redo_pixels_match": changed_pixel_count(
            &rotated_agent_art_pixels,
            &redone_agent_art_pixels
        ) < 20,
        "prop_rotation_degrees": [20.0, 40.0, 30.0],
        "prop_rotation_quaternion_xyzw": prop_quaternion,
        "prop_rotation_pixel_diff": changed_pixel_count(
            &identity_prop_pixels,
            &rotated_prop_pixels
        ),
        "prop_undo_pixels_match": changed_pixel_count(
            &identity_prop_pixels,
            &undone_prop_pixels
        ) < 20,
        "prop_redo_pixels_match": changed_pixel_count(&rotated_prop_pixels, &redone_prop_pixels)
            < 20,
        "host_checksum_before_rotation": format!("0x{placed_checksum:016x}"),
        "host_checksum_after_rotation": format!("0x{host_checksum_after_rotation:016x}"),
        "host_pin_unchanged_during_rotation": true,
        "agent_art_sidecar_path": art_sidecar,
        "agent_art_sidecar_sha256": sha256_hex(&art_bytes),
        "props_sidecar_path": props_sidecar,
        "props_sidecar_sha256": sha256_hex(&props_bytes),
    });
    let mut receipt = serde_json::json!({
        "artifact_root": fixture.root,
        "model_package": fixture.package,
        "model_package_model_sha256": model_package_sha256,
        "model_package_license_sha256": license_sha256,
        "model_package_manifest_sha256": package_manifest_sha256,
        "scene_path": fixture.scene,
        "scene_sha256": saved_scene_sha256,
        "scene_checksum": format!("0x{placed_checksum:016x}"),
        "graph_revision": placed_view.graph.revision(),
        "initial_pin_start_raw": original_view.agent.spec.start.map(FP::raw),
        "picked_start_raw": placed_view.agent.spec.start.map(FP::raw),
        "picked_goal_raw": placed_view.agent.spec.goal.map(FP::raw),
        "cancelled_pick_no_mutation": true,
        "staged_picks_no_host_mutation": true,
        "explicit_place_one_history_entry": true,
        "undo_redo_exact_pin": true,
        "fresh_physics_reopen_same_graph_and_agent": true,
        "fresh_reopen_pixels_match": true,
        "fresh_reopen_pixel_diff": changed_pixel_count(&placed_pixels, &reopened_pixels),
        "package_hidden_during_reopen": package_hidden_during_reopen,
        "package_restored_after_test": fixture.package.join(MODEL_ASSET).is_file(),
        "play_ticks": play_ticks,
        "play_moved": play_view.agent.position != initial_position,
        "play_pixels_changed": changed_pixel_count(&play_start_pixels, &play_pixels),
        "optional_export": export,
        "physical_os_input": false,
    });
    receipt
        .as_object_mut()
        .unwrap()
        .extend(rotation_receipt.as_object().unwrap().clone());
    fs::write(
        fixture.capture_directory.join("acceptance.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
    assert_eq!(
        fs::read(&fixture.physics_scene).unwrap(),
        fixture.physics_bytes
    );
}
