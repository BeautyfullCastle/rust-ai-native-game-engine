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
    editor::Mode,
    game::EditorGame,
    Editor, EditorApp, HostSpec,
};
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

const MODEL_ASSET: &str = "foreground.glb";
const LICENSE_ASSET: &str = "LICENSE.txt";

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    physics_scene: PathBuf,
    scene: PathBuf,
    package: PathBuf,
    capture_directory: PathBuf,
    scene_bytes: Option<Vec<u8>>,
    scene_checksum: Option<u64>,
    graph_revision: Option<[u8; 32]>,
    agent_start: Option<[i64; 3]>,
    agent_goal: Option<[i64; 3]>,
    hidden_package: Option<PathBuf>,
}

impl Fixture {
    fn new() -> Self {
        assert_eq!(
            std::env::var("ORR_MESH_PROP_VIEW_ACCEPTANCE").as_deref(),
            Ok("1")
        );
        assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
        assert_eq!(std::env::var("WGPU_BACKEND").as_deref(), Ok("vulkan"));
        let temporary = tempfile::tempdir().unwrap();
        let capture_directory = PathBuf::from(
            std::env::var_os("ORR_MESH_PROP_VIEW_CAPTURE_DIR")
                .expect("explicit GPU acceptance requires a fresh capture directory"),
        );
        assert!(capture_directory.is_absolute());
        fs::create_dir(&capture_directory).expect("capture directory must be new");
        let root = capture_directory.canonicalize().unwrap();
        let physics_scene = root.join("owned physics bootstrap.scene.yaml");
        fs::copy(EditorGame::PhysGame.default_scene_path(), &physics_scene).unwrap();
        let scene = std::env::var_os("ORR_MESH_PROP_VIEW_OUTPUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("mesh-prop-view.scene.yaml"));
        assert!(scene.is_absolute());
        assert!(scene.parent().is_some_and(Path::is_dir));
        assert!(!scene.exists(), "scene output must be new");
        assert!(!props_sidecar_path(&scene).exists());
        assert!(!art_sidecar_path(&scene).exists());

        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/imported_scene_demo")
            .canonicalize()
            .expect("checked-in model package exists");
        let package = root.join("owned props package");
        copy_tree(&source, &package).expect("copy complete model package into owned fixture");
        assert!(package.join("orr.package.json").is_file());
        assert!(package.join(MODEL_ASSET).is_file());
        assert!(package.join(LICENSE_ASSET).is_file());
        assert_eq!(fs::read_dir(&package).unwrap().count(), 5);

        Self {
            _temporary: temporary,
            root,
            physics_scene,
            scene,
            package,
            capture_directory,
            scene_bytes: None,
            scene_checksum: None,
            graph_revision: None,
            agent_start: None,
            agent_goal: None,
            hidden_package: None,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(hidden) = self.hidden_package.take() {
            if !self.package.exists() && hidden.exists() {
                if let Err(error) = fs::rename(hidden, &self.package) {
                    eprintln!("Could not restore hidden props package: {error}");
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

fn preflight_gpu() -> String {
    let gpu = Wgpu::headless(WgpuOptions::default())
        .expect("mesh prop view acceptance requires an available GPU");
    assert!(
        !gpu.is_software(),
        "software rendering is not GPU acceptance evidence"
    );
    let adapter = gpu.adapter_name().to_owned();
    eprintln!("Mesh prop viewport GPU: {adapter}");
    adapter
}

fn create_harness(fixture: &Fixture) -> Harness<'static, EditorApp> {
    let mut editor = Editor::start(&HostSpec::local_game(
        fixture.physics_scene.clone(),
        EditorGame::PhysGame,
    ))
    .expect("start normal Physics editor");
    editor.sync();
    Harness::builder()
        .with_size([1600.0, 1100.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu_setup(hardware_vulkan_setup())
        .build_eframe(move |creation_context| {
            EditorApp::new(editor, creation_context.wgpu_render_state.clone())
        })
}

fn settle(harness: &mut Harness<'_, EditorApp>) {
    for _ in 0..4 {
        harness.state_mut().editor.sync();
        harness.run_steps(2);
    }
}

fn click(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness.get_by_label(label).scroll_to_me();
    harness.run_steps(3);
    harness.get_by_label(label).click_accesskit();
    settle(harness);
}

fn click_button(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness
        .get_by_role_and_label(Role::Button, label)
        .scroll_to_me();
    harness.run_steps(3);
    harness
        .get_by_role_and_label(Role::Button, label)
        .click_accesskit();
    settle(harness);
}

fn set_field(harness: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    harness.run_steps(3);
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
}

fn type_scene_path(harness: &mut Harness<'_, EditorApp>, path: &Path, action: &str) {
    set_field(harness, "Scene path", &path.display().to_string());
    click(harness, action);
}

fn wait_for(
    harness: &mut Harness<'_, EditorApp>,
    operation: &str,
    ready: impl Fn(&EditorApp) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        settle(harness);
        if ready(harness.state()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {operation}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn same_path(actual: Option<&Path>, expected: &Path) -> bool {
    actual
        .and_then(|path| path.canonicalize().ok())
        .zip(expected.canonicalize().ok())
        .is_some_and(|(actual_path, expected_path)| actual_path == expected_path)
}

#[cfg(feature = "mesh-export")]
fn wait_for_label(harness: &mut Harness<'_, EditorApp>, operation: &str, label: &str) {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        settle(harness);
        if harness.query_by_label_contains(label).is_some() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {operation}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn mesh_view(harness: &Harness<'_, EditorApp>) -> orr_remote::mesh_navigation::MeshView {
    let snapshot = harness.state().editor.snapshot().unwrap();
    mesh_from_view(snapshot.predicted()).unwrap()
}

fn viewport_pixels(harness: &mut Harness<'_, EditorApp>, label: &str) -> Vec<u8> {
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
        .expect("production MeshNavigation viewport rendered");
    eprintln!(
        "Mesh prop viewport App GPU: {}",
        viewport.gpu().adapter_name()
    );
    let size = viewport.gpu().target().size();
    assert_eq!(size, target, "GPU viewport dimensions must be stable");
    let pixels = viewport.gpu().read_rgba8();
    assert_eq!(pixels.len(), (size.0 * size.1 * 4) as usize);
    assert!(pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0));
    let image =
        egui::ColorImage::from_rgba_unmultiplied([size.0 as usize, size.1 as usize], &pixels);
    let capture_directory = PathBuf::from(
        std::env::var_os("ORR_MESH_PROP_VIEW_CAPTURE_DIR").expect("capture directory set"),
    );
    orr_editor::app::write_png(&capture_directory.join(label), &image).unwrap();
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

fn props_sidecar_path(scene: &Path) -> PathBuf {
    let mut path = scene.as_os_str().to_os_string();
    path.push(".mesh-props.json");
    PathBuf::from(path)
}

fn art_sidecar_path(scene: &Path) -> PathBuf {
    let mut path = scene.as_os_str().to_os_string();
    path.push(".mesh-art.json");
    PathBuf::from(path)
}

fn imported_bounds(package: &Path) -> ([[f32; 3]; 2], String, String) {
    let model = orr_model::import::import_path(package, MODEL_ASSET).unwrap();
    let mut minimum = [f32::INFINITY; 3];
    let mut maximum = [f32::NEG_INFINITY; 3];
    for primitive in &model.source().primitives {
        for vertex in &primitive.vertices {
            for (axis, minimum_axis) in minimum.iter_mut().enumerate() {
                let local = primitive.transform[0][axis] * vertex.position[0]
                    + primitive.transform[1][axis] * vertex.position[1]
                    + primitive.transform[2][axis] * vertex.position[2]
                    + primitive.transform[3][axis];
                *minimum_axis = minimum_axis.min(local);
                maximum[axis] = maximum[axis].max(local);
            }
        }
    }
    let license = fs::read(package.join(LICENSE_ASSET)).unwrap();
    let glb = fs::read(package.join(MODEL_ASSET)).unwrap();
    ([minimum, maximum], sha256_hex(&glb), sha256_hex(&license))
}

fn prop_aabb_center(package: &Path, prop: &serde_json::Value) -> [f32; 3] {
    let (bounds, _, _) = imported_bounds(package);
    let offset = prop["art"]["offset"].as_array().unwrap();
    let scale = prop["art"]["scale"].as_f64().unwrap() as f32;
    std::array::from_fn(|axis| {
        (bounds[0][axis] + bounds[1][axis]) * 0.5 * scale + offset[axis].as_f64().unwrap() as f32
    })
}

fn viewport_pointer_for_world(harness: &Harness<'_, EditorApp>, world: [f32; 3]) -> Pos2 {
    let viewport = harness
        .state()
        .ui
        .viewport_rect
        .expect("viewport has a screen rectangle");
    let size = harness.state().ui.viewport_px;
    let camera = harness
        .state()
        .editor
        .presentation_camera3d(size)
        .expect("admitted mesh graph has a presentation camera");
    let pixel = camera
        .world_to_screen(world, size)
        .expect("prop bounds center projects in front of the camera");
    let points_per_pixel = viewport.width() / size.0 as f32;
    let pointer =
        viewport.min + egui::vec2(pixel[0] * points_per_pixel, pixel[1] * points_per_pixel);
    assert!(
        viewport.contains(pointer),
        "projected prop point is in viewport"
    );
    pointer
}

fn ramp_triangle_barycenter(harness: &Harness<'_, EditorApp>) -> [f32; 3] {
    let graph = mesh_view(harness).graph;
    let triangle = graph
        .triangles()
        .iter()
        .find(|triangle| {
            let heights = triangle
                .vertices
                .map(|index| graph.vertices()[index as usize][1].raw());
            heights.contains(&0) && heights.contains(&(2 * 65_536))
        })
        .expect("ramp-and-bridge graph has a sloped triangle");
    let raw_sum: [i128; 3] = std::array::from_fn(|axis| {
        triangle
            .vertices
            .iter()
            .map(|index| i128::from(graph.vertices()[*index as usize][axis].raw()))
            .sum()
    });
    let point = raw_sum.map(|coordinate| orr_fp::FP::from_raw((coordinate / 3) as i64).to_f32());
    assert!(
        point[1] > 0.0,
        "placement target lies above the ground floor"
    );
    point
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

fn pointer_pixel(harness: &Harness<'_, EditorApp>, pointer: Pos2) -> [f32; 2] {
    let viewport = harness.state().ui.viewport_rect.unwrap();
    let size = harness.state().ui.viewport_px;
    let points_per_pixel = viewport.width() / size.0 as f32;
    [
        (pointer.x - viewport.min.x) / points_per_pixel,
        (pointer.y - viewport.min.y) / points_per_pixel,
    ]
}

fn expected_drag_offset(
    harness: &Harness<'_, EditorApp>,
    start: Pos2,
    end: Pos2,
    original: [f64; 3],
) -> [f64; 3] {
    let size = harness.state().ui.viewport_px;
    let camera = harness
        .state()
        .editor
        .presentation_camera3d(size)
        .expect("admitted graph supplies camera for drag-plane calculation");
    let start_plane = camera
        .screen_to_plane_y(pointer_pixel(harness, start), size, original[1] as f32)
        .expect("drag begins on the prop's horizontal plane");
    let end_plane = camera
        .screen_to_plane_y(pointer_pixel(harness, end), size, original[1] as f32)
        .expect("drag ends on the prop's horizontal plane");
    [
        original[0] + f64::from(end_plane[0] - start_plane[0]),
        original[1],
        original[2] + f64::from(end_plane[2] - start_plane[2]),
    ]
}

fn drag_pointer(harness: &mut Harness<'_, EditorApp>, start: Pos2, end: Pos2) {
    harness.event(Event::PointerMoved(start));
    harness.run_steps(2);
    harness.event(Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.step();
    for step in 1..=5 {
        let amount = step as f32 / 5.0;
        harness.event(Event::PointerMoved(start + (end - start) * amount));
        harness.step();
    }
    harness.event(Event::PointerButton {
        pos: end,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    settle(harness);
}

fn prop_offset(document: &serde_json::Value) -> [f64; 3] {
    let offset = document["props"][0]["art"]["offset"].as_array().unwrap();
    [
        offset[0].as_f64().unwrap(),
        offset[1].as_f64().unwrap(),
        offset[2].as_f64().unwrap(),
    ]
}

#[cfg(feature = "mesh-export")]
fn export_bundle_if_requested(
    harness: &mut Harness<'_, EditorApp>,
    fixture: &Fixture,
    source_props: &[u8],
) -> serde_json::Value {
    let Some(output) = std::env::var_os("ORR_MESH_PROP_VIEW_EXPORT_OUTPUT") else {
        return serde_json::Value::Null;
    };
    let output = PathBuf::from(output);
    let runtime = PathBuf::from(
        std::env::var_os("ORR_MESH_EXPORT_RUNTIME")
            .expect("optional export requires the archived mesh player runtime"),
    );
    assert!(output.is_absolute() && output.parent().is_some_and(Path::is_dir));
    assert!(runtime.is_absolute() && runtime.is_file());
    assert!(!output.exists(), "export destination must be fresh");
    assert!(!art_sidecar_path(&fixture.scene).exists());
    menu_click(harness, "File", "Export mesh game…");
    set_field(
        harness,
        "Mesh player executable",
        &runtime.display().to_string(),
    );
    set_field(
        harness,
        "Mesh game output directory",
        &output.display().to_string(),
    );
    click(harness, "I trust this local mesh player executable");
    click_button(harness, "Export mesh game");
    wait_for_label(
        harness,
        "same-App mesh props export",
        "Mesh export complete:",
    );

    let manifest_path = output.join("orr.mesh-bundle.json");
    let manifest_bytes = fs::read(&manifest_path).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let scene_name = fixture.scene.file_name().unwrap().to_string_lossy();
    let props_name = format!("{scene_name}.mesh-props.json");
    let runtime_bytes = fs::read(&runtime).unwrap();
    let scene_bytes = fs::read(output.join(scene_name.as_ref())).unwrap();
    let props_bytes = fs::read(output.join(&props_name)).unwrap();
    let exported_runtime_name = manifest["runtime"]["name"].as_str().unwrap();
    let exported_runtime = fs::read(output.join(exported_runtime_name)).unwrap();
    assert_eq!(manifest["schema"], 1);
    assert!(
        manifest["sidecar"].is_null(),
        "fixture has no agent-art sidecar"
    );
    assert_eq!(manifest["props"]["name"], props_name);
    assert_eq!(
        scene_bytes.as_slice(),
        fixture.scene_bytes.as_deref().unwrap()
    );
    assert_eq!(props_bytes.as_slice(), source_props);
    assert_eq!(exported_runtime, runtime_bytes);
    for (record, bytes) in [
        (&manifest["scene"], scene_bytes.as_slice()),
        (&manifest["props"], props_bytes.as_slice()),
        (&manifest["runtime"], exported_runtime.as_slice()),
    ] {
        assert_eq!(record["bytes"].as_u64(), Some(bytes.len() as u64));
        assert_eq!(record["sha256"].as_str(), Some(sha256_hex(bytes).as_str()));
    }
    let mut files = fs::read_dir(&output)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    files.sort();
    let mut expected_files = vec![
        "orr.mesh-bundle.json".to_owned(),
        scene_name.to_string(),
        props_name,
        exported_runtime_name.to_owned(),
    ];
    expected_files.sort();
    assert_eq!(files, expected_files, "bundle contains exactly four files");
    assert_eq!(fs::read(&fixture.scene).unwrap(), scene_bytes);
    assert_eq!(
        fs::read(props_sidecar_path(&fixture.scene))
            .unwrap()
            .as_slice(),
        source_props
    );
    serde_json::json!({
        "bundle_directory": output.display().to_string(),
        "bundle_manifest": manifest_path.display().to_string(),
        "bundle_checksum": manifest["checksum"],
        "manifest_sha256": sha256_hex(&manifest_bytes),
        "bundle_scene_sha256": sha256_hex(&scene_bytes),
        "bundle_props_sha256": sha256_hex(&props_bytes),
        "bundle_runtime_sha256": sha256_hex(&exported_runtime),
        "runtime_path": runtime.display().to_string(),
        "runtime_source_sha256": sha256_hex(&runtime_bytes),
        "exact_four_file_bundle": true,
        "source_scene_and_props_unchanged": true,
        "agent_art_sidecar_absent": true,
    })
}

#[cfg(not(feature = "mesh-export"))]
fn export_bundle_if_requested(
    _harness: &mut Harness<'_, EditorApp>,
    _fixture: &Fixture,
    _source_props: &[u8],
) -> serde_json::Value {
    serde_json::Value::Null
}

#[test]
#[ignore = "explicit production EditorApp GPU acceptance; requires hardware Vulkan and opt-in env"]
fn production_mesh_prop_view_drag_is_local_reversible_and_persisted() {
    let adapter = preflight_gpu();
    let mut fixture = Fixture::new();
    let mut harness = create_harness(&fixture);
    harness.run_steps(4);
    assert!(harness.state().has_gpu());
    assert_eq!(harness.state().editor.game(), EditorGame::PhysGame);

    menu_click(&mut harness, "File", "New mesh scene…");
    type_scene_path(&mut harness, &fixture.scene, "Create");
    wait_for(&mut harness, "new admitted mesh scene", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && same_path(app.editor.path().as_deref(), &fixture.scene)
    });
    click(&mut harness, "Static-mesh navigation");
    click_button(&mut harness, "Create ramp and bridge");
    for (label, value) in [
        ("Start X", "1.5"),
        ("Start Y", "0"),
        ("Start Z", "1"),
        ("Goal X", "3"),
        ("Goal Y", "2"),
        ("Goal Z", "1"),
        ("Distance per tick", "1"),
    ] {
        set_field(&mut harness, label, value);
    }
    click_button(&mut harness, "Place mesh agent");
    let initial_view = mesh_view(&harness);
    assert_eq!(initial_view.agent.status, NavigationStatus::Moving);
    menu_click(&mut harness, "File", "Save Ctrl+S");
    fixture.scene_bytes = Some(fs::read(&fixture.scene).expect("scene saved by Editor"));
    fixture.scene_checksum = Some(harness.state().editor.checksum());
    fixture.graph_revision = Some(initial_view.graph.revision());
    fixture.agent_start = Some(initial_view.agent.position.map(orr_fp::FP::raw));
    fixture.agent_goal = Some(initial_view.agent.spec.goal.map(orr_fp::FP::raw));
    assert!(!props_sidecar_path(&fixture.scene).exists());

    click(&mut harness, "Mesh props");
    for (label, value) in [
        (
            "Prop package source directory",
            fixture.package.display().to_string(),
        ),
        ("Prop model asset", MODEL_ASSET.to_owned()),
        ("Prop license asset", LICENSE_ASSET.to_owned()),
    ] {
        set_field(&mut harness, label, &value);
    }
    click(&mut harness, "I have reviewed rights to use these props");
    click_button(&mut harness, "Load prop candidate");
    for (label, value) in [
        ("Prop name", "Viewport prop"),
        ("Prop position X", "3"),
        ("Prop position Y", "0.25"),
        ("Prop position Z", "1"),
        ("Prop scale", "0.5"),
    ] {
        set_field(&mut harness, label, value);
    }
    click_button(&mut harness, "Add prop");
    click_button(&mut harness, "Save props");
    let sidecar = props_sidecar_path(&fixture.scene);
    let original_sidecar = fs::read(&sidecar).expect("prop sidecar explicitly saved");
    let original_document: serde_json::Value = serde_json::from_slice(&original_sidecar).unwrap();
    assert_eq!(original_document["props"].as_array().unwrap().len(), 1);
    assert_eq!(
        fs::read(&fixture.scene).unwrap().as_slice(),
        fixture.scene_bytes.as_deref().unwrap()
    );
    assert_eq!(
        harness.state().editor.checksum(),
        fixture.scene_checksum.unwrap()
    );
    assert_eq!(
        mesh_view(&harness).graph.revision(),
        fixture.graph_revision.unwrap()
    );
    assert_eq!(
        mesh_view(&harness).agent.position.map(orr_fp::FP::raw),
        fixture.agent_start.unwrap(),
        "editing cosmetic prop placement leaves actual navigation agent state unchanged"
    );

    let original_offset = prop_offset(&original_document);
    let center = prop_aabb_center(&fixture.package, &original_document["props"][0]);
    click(&mut harness, "Move props in viewport");
    let before_drag_pixels = viewport_pixels(&mut harness, "props-before-drag.png");
    assert!(harness
        .get_by_role_and_label(Role::Button, "Redo prop edit")
        .accesskit_node()
        .is_disabled());
    let host_history_before_drag = harness.state().editor.history().clone();
    let viewport = harness.state().ui.viewport_rect.unwrap();
    let start_pointer = viewport_pointer_for_world(&harness, center);
    let end_pointer = start_pointer + egui::vec2(52.0, 0.0);
    assert!(viewport.contains(end_pointer));
    let expected_raw_offset =
        expected_drag_offset(&harness, start_pointer, end_pointer, original_offset);
    drag_pointer(&mut harness, start_pointer, end_pointer);
    assert_eq!(
        harness.state().editor.history(),
        &host_history_before_drag,
        "cosmetic props do not mutate ECS history"
    );
    assert!(harness
        .get_by_role_and_label(Role::Button, "Redo prop edit")
        .accesskit_node()
        .is_disabled());

    let moved_pixels = viewport_pixels(&mut harness, "props-after-drag.png");
    assert!(changed_pixel_count(&before_drag_pixels, &moved_pixels) > 20);
    assert_eq!(
        fs::read(&sidecar).unwrap(),
        original_sidecar,
        "viewport drag stays unsaved until explicit Save props"
    );
    assert_eq!(
        fs::read(&fixture.scene).unwrap().as_slice(),
        fixture.scene_bytes.as_deref().unwrap()
    );
    assert_eq!(
        harness.state().editor.checksum(),
        fixture.scene_checksum.unwrap()
    );
    let moved_view = mesh_view(&harness);
    assert_eq!(moved_view.graph.revision(), fixture.graph_revision.unwrap());
    assert_eq!(
        moved_view.agent.position.map(orr_fp::FP::raw),
        fixture.agent_start.unwrap()
    );
    click_button(&mut harness, "Undo prop edit");
    let undone_pixels = viewport_pixels(&mut harness, "props-after-undo.png");
    assert_eq!(changed_pixel_count(&before_drag_pixels, &undone_pixels), 0);
    assert!(!harness
        .get_by_role_and_label(Role::Button, "Redo prop edit")
        .accesskit_node()
        .is_disabled());
    assert_eq!(fs::read(&sidecar).unwrap(), original_sidecar);
    click_button(&mut harness, "Redo prop edit");
    let redone_pixels = viewport_pixels(&mut harness, "props-after-redo.png");
    assert!(changed_pixel_count(&moved_pixels, &redone_pixels) < 20);
    assert!(harness
        .get_by_role_and_label(Role::Button, "Redo prop edit")
        .accesskit_node()
        .is_disabled());
    assert_eq!(fs::read(&sidecar).unwrap(), original_sidecar);

    click_button(&mut harness, "Save props");
    let saved_sidecar = fs::read(&sidecar).expect("explicit Save props persists dragged offset");
    let saved_document: serde_json::Value = serde_json::from_slice(&saved_sidecar).unwrap();
    let moved_offset = prop_offset(&saved_document);
    assert_ne!(
        moved_offset, original_offset,
        "pointer movement changes the prop offset"
    );
    assert_eq!(
        moved_offset[1], original_offset[1],
        "horizontal-plane drag preserves height"
    );
    for axis in [0, 2] {
        assert!(
            (moved_offset[axis] - expected_raw_offset[axis]).abs() <= 1.0e-4,
            "saved drag coordinate {axis} matches independent camera-plane delta: {} vs {}",
            moved_offset[axis],
            expected_raw_offset[axis]
        );
    }
    assert_eq!(saved_document["props"][0]["name"], "Viewport prop");
    assert_eq!(
        saved_document["props"][0]["id"],
        original_document["props"][0]["id"]
    );
    let _raw_saved_pixels = viewport_pixels(&mut harness, "props-saved.png");

    click(&mut harness, "Snap movement to 0.25 units");
    let snap_start = viewport_pointer_for_world(
        &harness,
        prop_aabb_center(&fixture.package, &saved_document["props"][0]),
    );
    let snap_end = snap_start + egui::vec2(-26.0, 0.0);
    let unsnapped_offset = expected_drag_offset(&harness, snap_start, snap_end, moved_offset);
    drag_pointer(&mut harness, snap_start, snap_end);
    assert_eq!(fs::read(&sidecar).unwrap(), saved_sidecar);
    click_button(&mut harness, "Save props");
    let snapped_sidecar = fs::read(&sidecar).expect("snapped prop placement explicitly saved");
    let snapped_document: serde_json::Value = serde_json::from_slice(&snapped_sidecar).unwrap();
    let snapped_offset = prop_offset(&snapped_document);
    assert_eq!(snapped_offset[1], moved_offset[1]);
    for axis in [0, 2] {
        let grid_units = snapped_offset[axis] / 0.25;
        assert!(
            (grid_units - grid_units.round()).abs() <= 1.0e-6,
            "snapped coordinate {axis} lies on the absolute 0.25 grid"
        );
        let expected_grid = (unsnapped_offset[axis] / 0.25).round() * 0.25;
        assert!((snapped_offset[axis] - expected_grid).abs() <= 1.0e-4);
    }
    assert_ne!(snapped_offset, moved_offset);
    assert_ne!(snapped_offset, original_offset);
    let snapped_pixels = viewport_pixels(&mut harness, "props-snapped.png");
    assert_eq!(
        fs::read(&fixture.scene).unwrap().as_slice(),
        fixture.scene_bytes.as_deref().unwrap()
    );
    assert_eq!(
        harness.state().editor.checksum(),
        fixture.scene_checksum.unwrap()
    );
    assert_eq!(
        mesh_view(&harness).graph.revision(),
        fixture.graph_revision.unwrap()
    );
    assert_eq!(
        mesh_view(&harness).agent.position.map(orr_fp::FP::raw),
        fixture.agent_start.unwrap()
    );

    let history_before_cancel = harness.state().editor.history().clone();
    let second_start = viewport_pointer_for_world(
        &harness,
        prop_aabb_center(&fixture.package, &snapped_document["props"][0]),
    );
    let second_end = second_start + egui::vec2(-44.0, 0.0);
    harness.event(Event::PointerMoved(second_start));
    harness.run_steps(2);
    harness.event(Event::PointerButton {
        pos: second_start,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.step();
    harness.event(Event::PointerMoved(
        second_start + (second_end - second_start) * 0.5,
    ));
    harness.run_steps(2);
    let during_cancel_pixels = viewport_pixels(&mut harness, "props-during-cancel.png");
    assert!(changed_pixel_count(&snapped_pixels, &during_cancel_pixels) > 10);
    assert!(harness
        .get_by_label(LBL_PLAY)
        .accesskit_node()
        .is_disabled());
    assert!(harness
        .get_by_role_and_label(Role::Button, "Save props")
        .accesskit_node()
        .is_disabled());
    harness.key_press(Key::Escape);
    harness.event(Event::PointerButton {
        pos: second_start + (second_end - second_start) * 0.5,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    settle(&mut harness);
    assert_eq!(
        harness.state().editor.history(),
        &history_before_cancel,
        "Escape cancels without adding an undo entry"
    );
    assert_eq!(fs::read(&sidecar).unwrap(), snapped_sidecar);
    let cancelled_pixels = viewport_pixels(&mut harness, "props-after-escape.png");
    assert!(changed_pixel_count(&snapped_pixels, &cancelled_pixels) < 20);
    assert_eq!(
        fs::read(&fixture.scene).unwrap().as_slice(),
        fixture.scene_bytes.as_deref().unwrap()
    );
    assert_eq!(
        harness.state().editor.checksum(),
        fixture.scene_checksum.unwrap()
    );

    click(&mut harness, "Move props in viewport");
    let expected_moved_pixels = viewport_pixels(&mut harness, "props-saved-tooloff.png");

    drop(harness);
    let hidden_package = fixture.package.with_file_name("owned props package.hidden");
    fs::rename(&fixture.package, &hidden_package).expect("hide package source before reopen");
    fixture.hidden_package = Some(hidden_package.clone());
    assert!(!fixture.package.exists());
    let source_hidden_during_reopen = true;
    let mut harness = create_harness(&fixture);
    harness.run_steps(4);
    settle(&mut harness);
    menu_click(&mut harness, "File", "Open mesh scene…");
    type_scene_path(&mut harness, &fixture.scene, "OK");
    wait_for(&mut harness, "fresh Physics editor mesh reopen", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && same_path(app.editor.path().as_deref(), &fixture.scene)
            && app.editor.checksum() == fixture.scene_checksum.unwrap()
    });
    assert_eq!(
        harness.state().editor.checksum(),
        fixture.scene_checksum.unwrap()
    );
    assert_eq!(
        mesh_view(&harness).graph.revision(),
        fixture.graph_revision.unwrap()
    );
    assert!(
        !fixture.package.exists(),
        "reopen and playback use only embedded prop data"
    );
    let reopened_pixels = viewport_pixels(&mut harness, "props-reopened.png");
    assert!(changed_pixel_count(&expected_moved_pixels, &reopened_pixels) < 20);
    assert_eq!(fs::read(&sidecar).unwrap(), snapped_sidecar);

    click_button(&mut harness, LBL_PLAY);
    assert_eq!(harness.state().editor.mode(), Mode::Play);
    click_button(&mut harness, LBL_PAUSE);
    harness.state_mut().editor.seek(0);
    settle(&mut harness);
    let agent_at_start = mesh_view(&harness).agent.position;
    for _ in 0..16 {
        click_button(&mut harness, LBL_STEP);
    }
    let stepped = mesh_view(&harness);
    assert!(matches!(
        stepped.agent.status,
        NavigationStatus::Moving | NavigationStatus::Arrived
    ));
    assert_ne!(stepped.agent.position, agent_at_start);
    assert_eq!(harness.state().editor.timeline().unwrap().tick, 16);
    assert_eq!(fs::read(&sidecar).unwrap(), snapped_sidecar);
    let play_pixels = viewport_pixels(&mut harness, "props-play16.png");
    assert!(changed_pixel_count(&reopened_pixels, &play_pixels) > 10);
    click_button(&mut harness, LBL_STOP);
    assert_eq!(harness.state().editor.mode(), Mode::Edit);

    let target_scene = fixture
        ._temporary
        .path()
        .join("existing props target.scene.yaml");
    let target_sidecar = props_sidecar_path(&target_scene);
    fs::copy(&fixture.scene, &target_scene).expect("create existing Save As scene target");
    let target_filename = target_scene.file_name().unwrap().to_str().unwrap();
    let mut target_document: serde_json::Value = serde_json::from_slice(&snapped_sidecar).unwrap();
    target_document["scene_filename"] = serde_json::json!(target_filename);
    target_document["props"][0]["name"] = serde_json::json!("Existing target prop");
    target_document["props"][0]["art"]["scene_filename"] = serde_json::json!(target_filename);
    target_document["props"][0]["art"]["offset"] = serde_json::json!([4.0, 0.0, 3.0]);
    let target_sidecar_bytes = serde_json::to_vec_pretty(&target_document).unwrap();
    fs::write(&target_sidecar, &target_sidecar_bytes)
        .expect("create valid existing target sidecar");

    click(&mut harness, "Mesh props");
    menu_click(&mut harness, "File", "Save As…");
    type_scene_path(&mut harness, &target_scene, "OK");
    wait_for(&mut harness, "Save As to existing mesh scene", |app| {
        same_path(app.editor.path().as_deref(), &target_scene)
            && app.editor.checksum() == fixture.scene_checksum.unwrap()
    });
    let target_scene_after_save_as = fs::read(&target_scene).unwrap();
    click_button(&mut harness, "Save props");
    settle(&mut harness);
    assert_eq!(
        harness.state().editor.checksum(),
        fixture.scene_checksum.unwrap()
    );
    assert!(harness
        .get_by_label(LBL_PLAY)
        .accesskit_node()
        .is_disabled());
    assert_eq!(fs::read(&target_sidecar).unwrap(), target_sidecar_bytes);
    assert_eq!(fs::read(&sidecar).unwrap(), snapped_sidecar);
    assert_eq!(fs::read(&target_scene).unwrap(), target_scene_after_save_as);

    assert!(harness
        .get_by_role_and_label(Role::Button, "Discard unsaved props and reload")
        .accesskit_node()
        .is_disabled());
    click(&mut harness, "I confirm discarding unsaved prop changes");
    click_button(&mut harness, "Discard unsaved props and reload");
    assert!(harness
        .query_by_label_contains("Existing target prop")
        .is_some());
    assert!(harness
        .get_by_role_and_label(Role::Button, "Save props")
        .accesskit_node()
        .is_disabled());
    assert!(!harness
        .get_by_label(LBL_PLAY)
        .accesskit_node()
        .is_disabled());
    assert_eq!(fs::read(&target_sidecar).unwrap(), target_sidecar_bytes);
    assert_eq!(fs::read(&sidecar).unwrap(), snapped_sidecar);
    assert_eq!(fs::read(&target_scene).unwrap(), target_scene_after_save_as);
    assert_eq!(
        fs::read(&fixture.scene).unwrap(),
        fixture.scene_bytes.as_deref().unwrap()
    );
    let save_as_collision_recovery_verified = true;

    click(&mut harness, "I confirm removing this prop");
    click_button(&mut harness, "Remove selected prop");
    click_button(&mut harness, "Save props");
    assert!(!target_sidecar.exists());
    assert_eq!(fs::read(&sidecar).unwrap(), snapped_sidecar);
    assert_eq!(
        fs::read(&fixture.scene).unwrap(),
        fixture.scene_bytes.as_deref().unwrap()
    );

    menu_click(&mut harness, "File", "Save As…");
    type_scene_path(&mut harness, &fixture.scene, "OK");
    wait_for(&mut harness, "Save As to the original mesh scene", |app| {
        same_path(app.editor.path().as_deref(), &fixture.scene)
            && app.editor.checksum() == fixture.scene_checksum.unwrap()
    });
    harness.run_steps(4);
    assert!(harness.query_by_label_contains("Viewport prop").is_some());
    assert!(harness
        .get_by_role_and_label(Role::Button, "Save props")
        .accesskit_node()
        .is_disabled());
    assert!(!harness
        .get_by_label(LBL_PLAY)
        .accesskit_node()
        .is_disabled());
    assert_eq!(fs::read(&sidecar).unwrap(), snapped_sidecar);
    assert_eq!(
        fs::read(&fixture.scene).unwrap(),
        fixture.scene_bytes.as_deref().unwrap()
    );
    let empty_props_save_as_target_reload_verified = true;

    if harness.query_by_label("Duplicate selected prop").is_none() {
        click(&mut harness, "Mesh props");
    }
    let existing_prop_row = harness
        .query_by_label_contains("Viewport prop")
        .expect("saved prop is listed after returning to its scene");
    existing_prop_row.scroll_to_me();
    harness.run_steps(3);
    harness
        .query_by_label_contains("Viewport prop")
        .expect("saved prop row remains visible")
        .click_accesskit();
    settle(&mut harness);
    let source_before_duplicate = fs::read(&sidecar).unwrap();
    let scene_history_before_duplicate = harness.state().editor.history().clone();
    let single_prop_pixels = viewport_pixels(&mut harness, "single-prop-before-duplicate.png");
    click_button(&mut harness, "Duplicate selected prop");
    assert_eq!(fs::read(&sidecar).unwrap(), source_before_duplicate);
    assert_eq!(
        harness.state().editor.history(),
        &scene_history_before_duplicate,
        "duplicating a cosmetic prop does not mutate ECS history"
    );
    let duplicate_ready_pixels = viewport_pixels(&mut harness, "duplicate-prop-ready.png");
    click_button(&mut harness, "Place selected prop on mesh");
    let placement_world = ramp_triangle_barycenter(&harness);
    let placement_pointer = viewport_pointer_for_world(&harness, placement_world);
    click_viewport(&mut harness, placement_pointer);
    let staged_cancel_pixels = viewport_pixels(&mut harness, "prop-placement-cancel-preview.png");
    assert!(changed_pixel_count(&duplicate_ready_pixels, &staged_cancel_pixels) > 5);
    click_button(&mut harness, "Cancel prop placement");
    let cancelled_placement_pixels = viewport_pixels(&mut harness, "prop-placement-cancelled.png");
    assert!(changed_pixel_count(&duplicate_ready_pixels, &cancelled_placement_pixels) < 20);
    assert_eq!(fs::read(&sidecar).unwrap(), source_before_duplicate);
    assert_eq!(
        harness.state().editor.history(),
        &scene_history_before_duplicate,
        "cancelling staged placement leaves the host and saved sidecar unchanged"
    );
    click_button(&mut harness, "Place selected prop on mesh");
    click_viewport(&mut harness, placement_pointer);
    assert!(!harness
        .get_by_role_and_label(Role::Button, "Apply prop placement")
        .accesskit_node()
        .is_disabled());
    assert_eq!(fs::read(&sidecar).unwrap(), source_before_duplicate);
    assert_eq!(
        harness.state().editor.history(),
        &scene_history_before_duplicate,
        "placement remains cosmetic until explicit Apply"
    );
    let staged_placement_pixels = viewport_pixels(&mut harness, "prop-placement-staged.png");
    assert!(changed_pixel_count(&duplicate_ready_pixels, &staged_placement_pixels) > 5);
    click_button(&mut harness, "Apply prop placement");
    let applied_placement_pixels = viewport_pixels(&mut harness, "prop-placement-applied.png");
    assert!(changed_pixel_count(&duplicate_ready_pixels, &applied_placement_pixels) > 20);
    assert_eq!(fs::read(&sidecar).unwrap(), source_before_duplicate);
    assert_eq!(
        harness.state().editor.history(),
        &scene_history_before_duplicate,
        "applying cosmetic placement does not mutate ECS history"
    );
    click_button(&mut harness, "Undo prop edit");
    let undone_placement_pixels = viewport_pixels(&mut harness, "prop-placement-undone.png");
    assert!(changed_pixel_count(&duplicate_ready_pixels, &undone_placement_pixels) < 20);
    click_button(&mut harness, "Redo prop edit");
    let redone_placement_pixels = viewport_pixels(&mut harness, "prop-placement-redone.png");
    assert!(changed_pixel_count(&applied_placement_pixels, &redone_placement_pixels) < 20);
    click_button(&mut harness, "Undo prop edit");
    click_button(&mut harness, "Undo prop edit");
    let undone_duplicate_pixels = viewport_pixels(&mut harness, "duplicate-prop-undone.png");
    assert!(changed_pixel_count(&single_prop_pixels, &undone_duplicate_pixels) < 20);
    click_button(&mut harness, "Redo prop edit");
    let redone_duplicate_pixels = viewport_pixels(&mut harness, "duplicate-prop-redone.png");
    assert!(changed_pixel_count(&duplicate_ready_pixels, &redone_duplicate_pixels) < 20);
    click_button(&mut harness, "Redo prop edit");
    let final_placement_pixels = viewport_pixels(&mut harness, "prop-placement-final-redo.png");
    assert!(changed_pixel_count(&applied_placement_pixels, &final_placement_pixels) < 20);
    click_button(&mut harness, "Save props");
    let placement_sidecar = fs::read(&sidecar).expect("placed clone saves explicitly");
    let optional_export = export_bundle_if_requested(&mut harness, &fixture, &placement_sidecar);
    let placement_document: serde_json::Value = serde_json::from_slice(&placement_sidecar).unwrap();
    let original_id = original_document["props"][0]["id"].as_u64().unwrap();
    let placed_prop = placement_document["props"]
        .as_array()
        .unwrap()
        .iter()
        .find(|prop| prop["id"].as_u64() != Some(original_id))
        .expect("duplicate has its own stable identifier");
    assert_eq!(placement_document["props"].as_array().unwrap().len(), 2);
    assert!(placed_prop["name"].as_str().unwrap().contains("copy"));
    assert_eq!(
        placed_prop["art"]["model"],
        original_document["props"][0]["art"]["model"]
    );
    let placed_offset = placed_prop["art"]["offset"].as_array().unwrap();
    assert!(placed_offset[1].as_f64().unwrap() > 0.0);
    for (axis, expected) in placement_world.into_iter().enumerate() {
        assert!(
            (placed_offset[axis].as_f64().unwrap() - f64::from(expected)).abs() < 0.05,
            "placed coordinate {axis} stays on the selected ramp triangle"
        );
    }
    assert_eq!(
        fs::read(&fixture.scene).unwrap(),
        fixture.scene_bytes.as_deref().unwrap()
    );
    let placement_saved_pixels = viewport_pixels(&mut harness, "prop-placement-saved.png");
    menu_click(&mut harness, "File", "Save Ctrl+S");
    let placement_scene_bytes = fs::read(&fixture.scene).unwrap();
    let placement_scene_checksum = harness.state().editor.checksum();
    drop(harness);
    assert!(
        !fixture.package.exists(),
        "package stays hidden during placement reopen"
    );
    let mut placement_reopened = create_harness(&fixture);
    placement_reopened.run_steps(4);
    menu_click(&mut placement_reopened, "File", "Open mesh scene…");
    type_scene_path(&mut placement_reopened, &fixture.scene, "OK");
    wait_for(
        &mut placement_reopened,
        "fresh placement scene reopen",
        |app| {
            app.editor.game() == EditorGame::MeshNavigation
                && same_path(app.editor.path().as_deref(), &fixture.scene)
                && app.editor.checksum() == placement_scene_checksum
        },
    );
    assert!(!fixture.package.exists());
    let placement_reopened_pixels =
        viewport_pixels(&mut placement_reopened, "prop-placement-reopened.png");
    assert!(changed_pixel_count(&placement_saved_pixels, &placement_reopened_pixels) < 20);
    assert_eq!(fs::read(&sidecar).unwrap(), placement_sidecar);
    assert_eq!(fs::read(&fixture.scene).unwrap(), placement_scene_bytes);
    click_button(&mut placement_reopened, LBL_PLAY);
    click_button(&mut placement_reopened, LBL_PAUSE);
    placement_reopened.state_mut().editor.seek(0);
    settle(&mut placement_reopened);
    for _ in 0..16 {
        click_button(&mut placement_reopened, LBL_STEP);
    }
    assert_eq!(
        placement_reopened.state().editor.timeline().unwrap().tick,
        16
    );
    assert_eq!(fs::read(&sidecar).unwrap(), placement_sidecar);
    click_button(&mut placement_reopened, LBL_STOP);
    drop(placement_reopened);
    let placement_workflow_verified = true;

    fs::rename(&hidden_package, &fixture.package).expect("restore owned package source");
    fixture.hidden_package = None;
    assert!(fixture.package.join(MODEL_ASSET).is_file());

    let (bounds, model_sha256, license_sha256) = imported_bounds(&fixture.package);
    let package_manifest_sha256 =
        sha256_hex(&fs::read(fixture.package.join("orr.package.json")).unwrap());
    let capture_names = [
        "props-before-drag.png",
        "props-after-drag.png",
        "props-after-undo.png",
        "props-after-redo.png",
        "props-saved.png",
        "props-snapped.png",
        "props-saved-tooloff.png",
        "props-during-cancel.png",
        "props-after-escape.png",
        "props-reopened.png",
        "props-play16.png",
        "duplicate-prop-ready.png",
        "single-prop-before-duplicate.png",
        "prop-placement-cancelled.png",
        "prop-placement-cancel-preview.png",
        "prop-placement-staged.png",
        "prop-placement-applied.png",
        "prop-placement-undone.png",
        "prop-placement-redone.png",
        "duplicate-prop-undone.png",
        "duplicate-prop-redone.png",
        "prop-placement-final-redo.png",
        "prop-placement-saved.png",
        "prop-placement-reopened.png",
    ];
    let captures = capture_names
        .iter()
        .map(|name| {
            let bytes = fs::read(fixture.capture_directory.join(name)).unwrap();
            serde_json::json!({"path": name, "bytes": bytes.len(), "sha256": sha256_hex(&bytes)})
        })
        .collect::<Vec<_>>();
    let snap_verified = [snapped_offset[0], snapped_offset[2]].iter().all(|value| {
        let units = *value / 0.25;
        (units - units.round()).abs() <= 1.0e-6
    });
    let mut output_receipt = serde_json::json!({
        "artifact_root": fixture.root.display().to_string(),
        "scene_path": fixture.scene.display().to_string(),
        "scene_sha256": sha256_hex(&fs::read(&fixture.scene).unwrap()),
        "scene_checksum": format!("0x{:016x}", fixture.scene_checksum.unwrap()),
        "props_sidecar_path": sidecar.display().to_string(),
        "props_sidecar_sha256": sha256_hex(&placement_sidecar),
        "props_count": placement_document["props"].as_array().unwrap().len(),
        "package_source_directory": fixture.package.display().to_string(),
        "model_sha256": model_sha256,
        "license_sha256": license_sha256,
        "package_manifest_sha256": package_manifest_sha256,
        "imported_model_bounds": bounds,
        "graph_revision": hex_bytes(&fixture.graph_revision.unwrap()),
        "agent_start_raw": fixture.agent_start.unwrap(),
        "agent_goal_raw": fixture.agent_goal.unwrap(),
        "original_offset": original_offset,
        "raw_saved_offset": moved_offset,
        "saved_offset": snapped_offset,
        "snapped_offset": snapped_offset,
        "horizontal_drag_preserved_y": moved_offset[1] == original_offset[1]
            && snapped_offset[1] == moved_offset[1],
        "snap_verified": snap_verified,
        "sidecar_unchanged_until_explicit_save": true,
        "undo_redo_verified": true,
        "escape_cancelled_without_history": true,
        "fresh_reopen_pixels_match": changed_pixel_count(&expected_moved_pixels, &reopened_pixels) < 20,
        "play_ticks": 16,
        "play_moved_agent": stepped.agent.position != agent_at_start,
        "play_changed_view_pixels": changed_pixel_count(&reopened_pixels, &play_pixels),
        "source_hidden_during_reopen": source_hidden_during_reopen,
        "sources_restored": fixture.package.join(MODEL_ASSET).is_file(),
        "captures": captures,
        "scene_art_tool_off_for_reopen_capture": true,
        "optional_export": optional_export,
        "bundle_path": optional_export["bundle_directory"],
        "bundle_manifest_sha256": optional_export["manifest_sha256"],
        "bundle_checksum": optional_export["bundle_checksum"],
        "runtime_path": optional_export["runtime_path"],
        "runtime_sha256": optional_export["runtime_source_sha256"],
        "gpu_adapter": adapter,
        "physical_os_input": false,
    });
    output_receipt["save_as_collision_recovery_verified"] =
        serde_json::json!(save_as_collision_recovery_verified);
    output_receipt["empty_props_save_as_target_reload_verified"] =
        serde_json::json!(empty_props_save_as_target_reload_verified);
    output_receipt["duplicate_placement_workflow_verified"] =
        serde_json::json!(placement_workflow_verified);
    output_receipt["duplicate_prop_id"] = serde_json::json!(placed_prop["id"]);
    output_receipt["duplicate_prop_placement_world"] = serde_json::json!(placement_world);
    output_receipt["duplicate_prop_saved_offset"] = serde_json::json!(placed_prop["art"]["offset"]);
    output_receipt["duplicate_prop_staged_pixels"] = serde_json::json!(changed_pixel_count(
        &duplicate_ready_pixels,
        &staged_placement_pixels
    ));
    output_receipt["duplicate_prop_applied_pixel_diff"] = serde_json::json!(changed_pixel_count(
        &duplicate_ready_pixels,
        &applied_placement_pixels
    ));
    output_receipt["placement_undo_pixels_match"] = serde_json::json!(
        changed_pixel_count(&duplicate_ready_pixels, &undone_placement_pixels) < 20
    );
    output_receipt["placement_redo_pixels_match"] = serde_json::json!(
        changed_pixel_count(&applied_placement_pixels, &redone_placement_pixels) < 20
    );
    output_receipt["duplicate_prop_reopen_pixels_match"] = serde_json::json!(
        changed_pixel_count(&placement_saved_pixels, &placement_reopened_pixels) < 20
    );
    output_receipt["duplicate_prop_undo_pixels_match"] =
        serde_json::json!(changed_pixel_count(&single_prop_pixels, &undone_duplicate_pixels) < 20);
    output_receipt["duplicate_prop_redo_pixels_match"] = serde_json::json!(
        changed_pixel_count(&duplicate_ready_pixels, &redone_duplicate_pixels) < 20
    );
    fs::write(
        fixture.capture_directory.join("acceptance.json"),
        serde_json::to_vec_pretty(&output_receipt).unwrap(),
    )
    .unwrap();
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
