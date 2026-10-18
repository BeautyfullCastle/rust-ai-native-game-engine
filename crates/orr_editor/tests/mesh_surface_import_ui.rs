#![cfg(all(
    windows,
    target_arch = "x86_64",
    feature = "mesh-mission",
    feature = "mesh-export"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
#![recursion_limit = "256"]

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
use orr_remote::mesh_navigation::{
    mesh_from_view,
    mission::{self, MissionView},
};
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

struct Fixture {
    root: PathBuf,
    package: PathBuf,
    hidden_package: PathBuf,
    scene: PathBuf,
    physics_scene: PathBuf,
    runtime: PathBuf,
    export: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        assert_eq!(
            std::env::var("ORR_MESH_SURFACE_IMPORT_ACCEPTANCE").as_deref(),
            Ok("1")
        );
        assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
        assert_eq!(std::env::var("WGPU_BACKEND").unwrap_or_default(), "vulkan");
        let root = PathBuf::from(
            std::env::var_os("ORR_MESH_SURFACE_IMPORT_CAPTURE_DIR")
                .expect("set a fresh mesh-surface-import capture root"),
        );
        assert!(root.is_absolute());
        fs::create_dir(&root).expect("capture root must be fresh");
        let root = root.canonicalize().unwrap();
        fs::create_dir(root.join("captures")).unwrap();

        let package = root.join("generated-walkable-floor");
        fs::create_dir(&package).unwrap();
        let glb = floor_glb();
        let license = b"Example walkable-floor license\n";
        fs::write(package.join("floor.glb"), &glb).unwrap();
        fs::write(package.join("LICENSE.txt"), license).unwrap();
        let runtime_package = orr_package::Runtime::content_only();
        let manifest = orr_package::Manifest {
            schema: 1,
            name: "generated-floor".into(),
            version: "1.0.0".into(),
            engine: format!("={}", runtime_package.engine_version),
            capabilities: BTreeSet::from(["models".into()]),
            dependencies: BTreeMap::new(),
            files: BTreeSet::from(["LICENSE.txt".into(), "floor.glb".into()]),
        };
        fs::write(
            package.join("orr.package.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        let scene = PathBuf::from(
            std::env::var_os("ORR_MESH_SURFACE_IMPORT_SCENE")
                .unwrap_or_else(|| root.join("walkable-floor.scene.yaml").into_os_string()),
        );
        assert!(scene.is_absolute() && !scene.exists());
        let scene_parent = scene.parent().unwrap().canonicalize().unwrap();
        let scene = scene_parent.join(scene.file_name().unwrap());
        let physics_scene = root.join("owned-stock-physics.scene.yaml");
        fs::copy(EditorGame::PhysGame.default_scene_path(), &physics_scene).unwrap();
        let runtime = PathBuf::from(
            std::env::var_os("ORR_MESH_EXPORT_RUNTIME")
                .expect("mesh-surface-import acceptance requires the matching mesh player"),
        );
        assert!(runtime.is_absolute() && runtime.is_file());
        let export = PathBuf::from(
            std::env::var_os("ORR_MESH_SURFACE_IMPORT_EXPORT_OUTPUT")
                .expect("mesh-surface-import acceptance requires a fresh export directory"),
        );
        assert!(export.is_absolute() && !export.exists());
        assert_eq!(
            export.parent().unwrap().canonicalize().unwrap(),
            root.parent().unwrap()
        );
        Self {
            root: root.clone(),
            package,
            hidden_package: root.join("hidden-generated-floor-package"),
            scene,
            physics_scene,
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

impl Drop for Fixture {
    fn drop(&mut self) {
        if self.hidden_package.exists() && !self.package.exists() {
            let _ = fs::rename(&self.hidden_package, &self.package);
        }
    }
}

fn floor_glb() -> Vec<u8> {
    let positions: [[f32; 3]; 6] = [
        [-4.0, 0.0, -2.0],
        [0.0, 0.0, -2.0],
        [0.0, 0.0, 2.0],
        [-4.0, 0.0, 2.0],
        [4.0, 0.0, -2.0],
        [4.0, 0.0, 2.0],
    ];
    let indices: [u16; 12] = [0, 3, 2, 0, 2, 1, 1, 2, 5, 1, 5, 4];
    let mut binary = Vec::new();
    for position in positions {
        for coordinate in position {
            binary.extend_from_slice(&coordinate.to_le_bytes());
        }
    }
    for _ in positions {
        binary.extend_from_slice(&0.0f32.to_le_bytes());
        binary.extend_from_slice(&1.0f32.to_le_bytes());
        binary.extend_from_slice(&0.0f32.to_le_bytes());
    }
    for index in indices {
        binary.extend_from_slice(&index.to_le_bytes());
    }
    let document = serde_json::json!({
        "asset": {"version": "2.0", "generator": "Orrery mesh surface import acceptance"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0, "translation": [1.0, 0.0, 0.5]}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "NORMAL": 1}, "indices": 2, "mode": 4, "material": 0}]}],
        "materials": [{"pbrMetallicRoughness": {"baseColorFactor": [1.0, 1.0, 1.0, 1.0], "metallicFactor": 0.0, "roughnessFactor": 1.0}}],
        "buffers": [{"byteLength": binary.len()}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": 72, "target": 34962},
            {"buffer": 0, "byteOffset": 72, "byteLength": 72, "target": 34962},
            {"buffer": 0, "byteOffset": 144, "byteLength": 24, "target": 34963}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 6, "type": "VEC3", "min": [-4.0, 0.0, -2.0], "max": [4.0, 0.0, 2.0]},
            {"bufferView": 1, "componentType": 5126, "count": 6, "type": "VEC3"},
            {"bufferView": 2, "componentType": 5123, "count": 12, "type": "SCALAR", "min": [0], "max": [5]}
        ]
    });
    let mut json = serde_json::to_vec(&document).unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    while !binary.len().is_multiple_of(4) {
        binary.push(0);
    }
    let total_length = 12 + 8 + json.len() + 8 + binary.len();
    let mut glb = Vec::with_capacity(total_length);
    glb.extend_from_slice(&0x4654_6c67u32.to_le_bytes());
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total_length as u32).to_le_bytes());
    glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
    glb.extend_from_slice(&0x4e4f_534au32.to_le_bytes());
    glb.extend_from_slice(&json);
    glb.extend_from_slice(&(binary.len() as u32).to_le_bytes());
    glb.extend_from_slice(&0x004e_4942u32.to_le_bytes());
    glb.extend_from_slice(&binary);
    glb
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
                "Mesh surface import GPU preflight: {} (software: {})",
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
                std::env::var_os("ORR_MESH_SURFACE_IMPORT_ACCEPTANCE").is_none()
                    && std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "required mesh-surface-import GPU acceptance unavailable: {error}"
            );
            eprintln!("SKIP: mesh-surface-import GPU unavailable: {error}");
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

fn capture(harness: &mut Harness<'_, EditorApp>, root: &Path, name: &str) {
    settle(harness);
    let frame = harness.render().expect("render production editor capture");
    let image = egui::ColorImage::from_rgba_unmultiplied(
        [frame.width() as usize, frame.height() as usize],
        frame.as_raw(),
    );
    orr_editor::app::write_png(&root.join("captures").join(name), &image).unwrap();
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

fn mission_view(app: &EditorApp) -> MissionView {
    mission::from_view(app.editor.snapshot().unwrap().predicted())
        .unwrap()
        .expect("mesh mission was explicitly configured")
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
    click_button(harness, LBL_PLAY);
    wait_for(harness, "mesh Play", |app| {
        app.editor
            .timeline()
            .is_some_and(|timeline| timeline.playing)
    });
    click_button(harness, LBL_PAUSE);
    if harness
        .state()
        .editor
        .timeline()
        .is_some_and(|timeline| timeline.tick != 0)
    {
        click(harness, "⏮");
    }
    for _ in 0..1024 {
        if mission_view(harness.state()).run.won == 1 {
            let frame = harness.state().editor.snapshot().unwrap().predicted();
            let view = mesh_from_view(frame).unwrap();
            let mission = mission_view(harness.state());
            assert_eq!(mission.run.collected, (1u32 << mission.config.count) - 1);
            assert!(matches!(
                view.agent.status,
                NavigationStatus::Moving | NavigationStatus::Arrived
            ));
            let distance_squared_raw = (0..3)
                .map(|axis| {
                    let delta = i128::from(view.agent.position[axis].raw())
                        - i128::from(mission.config.exit.position[axis].raw());
                    delta * delta
                })
                .sum::<i128>();
            let radius_raw = i128::from(mission.config.exit.radius.raw());
            assert!(distance_squared_raw <= radius_raw * radius_raw);
            return (
                harness.state().editor.timeline().unwrap().tick,
                frame.checksum(),
            );
        }
        click_button(harness, LBL_STEP);
    }
    panic!("mesh mission did not reach its authored collection and exit conditions")
}

#[test]
#[ignore = "requires ORR_MESH_SURFACE_IMPORT_ACCEPTANCE=1 and a hardware Vulkan adapter"]
fn imported_model_becomes_saved_playable_mesh_geometry_without_source_dependency() {
    assert!(gpu_available());
    let fixture = Fixture::new();
    let mut harness = fixture.harness();
    harness.run_steps(4);
    assert!(harness.state().has_gpu());
    menu_click(&mut harness, "File", "New mesh scene…");
    type_field(
        &mut harness,
        "Scene path",
        &fixture.scene.display().to_string(),
    );
    click_button(&mut harness, "Create");
    wait_for(&mut harness, "new blank mesh scene", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && app.editor.path().as_deref() == Some(fixture.scene.as_path())
    });
    let initial_snapshot = harness.state().editor.snapshot().unwrap();
    let initial_checksum = initial_snapshot.predicted().checksum();
    let initial_history = harness.state().editor.history().entries.len();
    let (initial_viewport_size, initial_viewport) = viewport_pixels(&mut harness);
    capture(&mut harness, &fixture.root, "blank-mesh.png");

    if harness.query_by_label("Import walkable model").is_none() {
        click(&mut harness, "Static-mesh navigation");
    }
    click(&mut harness, "Import walkable model");
    type_field(
        &mut harness,
        "Walkable model package root",
        &fixture
            .root
            .join("missing-walkable-floor-package")
            .display()
            .to_string(),
    );
    type_field(&mut harness, "Walkable model asset", "floor.glb");
    type_field(&mut harness, "Walkable model license asset", "LICENSE.txt");
    click(
        &mut harness,
        "I have reviewed the rights to use this model license",
    );
    click(
        &mut harness,
        "I confirm replacing staged geometry with this imported walkable surface",
    );
    click_button(&mut harness, "Stage imported walkable surface");
    assert!(harness
        .query_by_label_contains("missing-walkable-floor-package:")
        .is_some());
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        initial_history
    );
    click_button(&mut harness, "Reset walkable model import form");

    if harness
        .query_by_label("Walkable model package root")
        .is_none()
    {
        click(&mut harness, "Import walkable model");
    }
    type_field(
        &mut harness,
        "Walkable model package root",
        &fixture.package.display().to_string(),
    );
    type_field(&mut harness, "Walkable model asset", "floor.glb");
    type_field(&mut harness, "Walkable model license asset", "LICENSE.txt");
    click(
        &mut harness,
        "I have reviewed the rights to use this model license",
    );
    click(
        &mut harness,
        "I confirm replacing staged geometry with this imported walkable surface",
    );
    click_button(&mut harness, "Stage imported walkable surface");
    if harness
        .get_by_role_and_label(Role::Button, "Undo staged geometry")
        .accesskit_node()
        .is_disabled()
    {
        capture(&mut harness, &fixture.root, "import-rejected.png");
    }
    assert!(!harness
        .get_by_role_and_label(Role::Button, "Undo staged geometry")
        .accesskit_node()
        .is_disabled());
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        initial_history
    );
    harness.state_mut().editor.fit_camera();
    let (staged_size, staged_viewport) = viewport_pixels(&mut harness);
    assert_eq!(staged_size, initial_viewport_size);
    let staged_pixel_diff = changed_pixel_count(&initial_viewport, &staged_viewport);
    assert!(
        staged_pixel_diff > 20,
        "imported candidate was not visible: {staged_pixel_diff}"
    );
    capture(&mut harness, &fixture.root, "import-candidate.png");
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        initial_history
    );
    click_button(&mut harness, "Undo staged geometry");
    let undone_candidate = viewport_pixels(&mut harness).1;
    assert!(changed_pixel_count(&staged_viewport, &undone_candidate) > 20);
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        initial_history
    );
    click_button(&mut harness, "Redo staged geometry");
    let redone_candidate = viewport_pixels(&mut harness).1;
    assert!(changed_pixel_count(&staged_viewport, &redone_candidate) < 20);
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        initial_history
    );

    for (label, value) in [
        ("Start X", "-2"),
        ("Start Y", "0"),
        ("Start Z", "0"),
        ("Goal X", "4"),
        ("Goal Y", "0"),
        ("Goal Z", "0"),
        ("Maximum slope", "4"),
        ("Maximum vertical projection", "2"),
        ("Distance per tick", "0.25"),
    ] {
        type_field(&mut harness, label, value);
    }
    click_button(&mut harness, "Place mesh agent");
    wait_for(&mut harness, "mesh agent placed on imported graph", |app| {
        app.editor.checksum() != initial_checksum
            && mesh_from_view(app.editor.snapshot().unwrap().predicted()).is_ok()
    });
    let placed_checksum = harness.state().editor.checksum();
    let placed_history = harness.state().editor.history().entries.len();
    let placed_graph = mesh_from_view(harness.state().editor.snapshot().unwrap().predicted())
        .unwrap()
        .graph;
    assert_eq!(placed_graph.triangles().len(), 4);
    assert_eq!(placed_history, initial_history + 1);
    menu_click(&mut harness, "Edit", "Undo Ctrl+Z");
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    menu_click(&mut harness, "Edit", "Redo Ctrl+Y");
    assert_eq!(harness.state().editor.checksum(), placed_checksum);

    if harness.query_by_label("Enable mesh mission").is_none() {
        click(&mut harness, "Mesh mission");
    }
    click(&mut harness, "Enable mesh mission");
    type_field(&mut harness, "Mission collectible count", "2");
    for (label, value) in [
        ("Collectible 1 X", "-1"),
        ("Collectible 1 Y", "0"),
        ("Collectible 1 Z", "0.5"),
        ("Collectible 1 radius", "0.5"),
        ("Collectible 2 X", "2"),
        ("Collectible 2 Y", "0"),
        ("Collectible 2 Z", "0.5"),
        ("Collectible 2 radius", "0.5"),
        ("Mission exit X", "4"),
        ("Mission exit Y", "0"),
        ("Mission exit Z", "0"),
        ("Mission exit radius", "0.5"),
    ] {
        type_field(&mut harness, label, value);
    }
    let pre_mission_checksum = harness.state().editor.checksum();
    let pre_mission_history = harness.state().editor.history().entries.len();
    click_button(&mut harness, "Apply mesh mission");
    wait_for(&mut harness, "mission authoring applied", |app| {
        app.editor.checksum() != pre_mission_checksum
            && mission::from_view(app.editor.snapshot().unwrap().predicted())
                .is_ok_and(|mission| mission.is_some())
    });
    let saved_checksum = harness.state().editor.checksum();
    let saved_history = harness.state().editor.history().entries.len();
    let mission_config = mission_view(harness.state()).config;
    assert_eq!(mission_config.count, 2);
    assert_eq!(saved_history, pre_mission_history + 1);
    harness.state_mut().editor.fit_camera();
    let (_, saved_viewport) = viewport_pixels(&mut harness);
    menu_click(&mut harness, "File", "Save mesh workspace");
    let scene_bytes = fs::read(&fixture.scene).unwrap();
    let source_glb_before_hide = fs::read(fixture.package.join("floor.glb")).unwrap();
    let source_license_before_hide = fs::read(fixture.package.join("LICENSE.txt")).unwrap();
    let source_manifest_before_hide = fs::read(fixture.package.join("orr.package.json")).unwrap();
    capture(&mut harness, &fixture.root, "imported-mission-edit.png");
    drop(harness);

    fs::rename(&fixture.package, &fixture.hidden_package).unwrap();
    let mut reopened = fixture.harness();
    reopened.run_steps(4);
    menu_click(&mut reopened, "File", "Open mesh scene…");
    type_field(
        &mut reopened,
        "Scene path",
        &fixture.scene.display().to_string(),
    );
    click_button(&mut reopened, "OK");
    wait_for(&mut reopened, "source-free imported mesh reopen", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && app.editor.checksum() == saved_checksum
            && app.editor.path().as_deref() == Some(fixture.scene.as_path())
    });
    let reopened_frame = reopened.state().editor.snapshot().unwrap().predicted();
    let reopened_graph = mesh_from_view(reopened_frame).unwrap().graph;
    assert_eq!(reopened_graph.revision(), placed_graph.revision());
    assert_eq!(reopened_graph.triangles().len(), 4);
    assert_eq!(mission_view(reopened.state()).config, mission_config);
    reopened.state_mut().editor.fit_camera();
    let (_, reopened_viewport) = viewport_pixels(&mut reopened);
    let reopen_pixel_diff = changed_pixel_count(&saved_viewport, &reopened_viewport);
    assert!(
        reopen_pixel_diff < 20,
        "reopened imported floor differs: {reopen_pixel_diff}"
    );
    capture(
        &mut reopened,
        &fixture.root,
        "imported-source-free-reopen.png",
    );
    assert!(!fixture.package.exists());
    assert_eq!(
        fs::read(fixture.hidden_package.join("floor.glb")).unwrap(),
        source_glb_before_hide
    );
    assert_eq!(
        fs::read(fixture.hidden_package.join("LICENSE.txt")).unwrap(),
        source_license_before_hide
    );

    let (win_tick, win_checksum) = advance_until_won(&mut reopened);
    assert_eq!(mission_view(reopened.state()).run.won, 1);
    let observed_win_checksum = reopened
        .state()
        .editor
        .snapshot()
        .unwrap()
        .predicted()
        .checksum();
    let won_viewport = viewport_pixels(&mut reopened).1;
    assert_eq!(observed_win_checksum, win_checksum);
    capture(&mut reopened, &fixture.root, "imported-mission-won.png");
    click_button(&mut reopened, LBL_STOP);
    assert_eq!(reopened.state().editor.mode(), Mode::Edit);
    assert_eq!(reopened.state().editor.checksum(), saved_checksum);
    let (replay_tick, replay_checksum) = advance_until_won(&mut reopened);
    assert_eq!(replay_tick, win_tick);
    assert_eq!(replay_checksum, win_checksum);
    let replay_viewport = viewport_pixels(&mut reopened).1;
    let replay_pixel_diff = changed_pixel_count(&won_viewport, &replay_viewport);
    assert!(
        replay_pixel_diff < 20,
        "mission replay render differs: {replay_pixel_diff}"
    );
    capture(&mut reopened, &fixture.root, "imported-mission-replay.png");
    click_button(&mut reopened, LBL_STOP);
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
    let scene_name = fixture.scene.file_name().unwrap().to_string_lossy();
    let mut export_names = fs::read_dir(&fixture.export)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    export_names.sort();
    let mut expected_export_names = vec![
        scene_name.into_owned(),
        "orr.mesh-bundle.json".into(),
        "orr_mesh_player.exe".into(),
    ];
    expected_export_names.sort();
    assert_eq!(export_names, expected_export_names);
    let manifest_bytes = fs::read(fixture.export.join("orr.mesh-bundle.json")).unwrap();
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(manifest["game_profile"], "mesh-mission-v1");
    assert_eq!(manifest["input_profile"], "mesh-click-to-move-v1");
    assert_eq!(
        manifest["scene"]["sha256"],
        sha256(&fs::read(fixture.export.join(fixture.scene.file_name().unwrap())).unwrap())
    );
    assert_eq!(
        fs::read(fixture.export.join(fixture.scene.file_name().unwrap())).unwrap(),
        scene_bytes
    );
    assert_eq!(manifest["checksum"], saved_checksum);
    let export_inventory = export_names
        .iter()
        .map(|name| {
            let bytes = fs::read(fixture.export.join(name)).unwrap();
            serde_json::json!({"name":name,"bytes":bytes.len(),"sha256":sha256(&bytes)})
        })
        .collect::<Vec<_>>();

    fs::rename(&fixture.hidden_package, &fixture.package).unwrap();
    assert_eq!(
        fs::read(fixture.package.join("floor.glb")).unwrap(),
        source_glb_before_hide
    );
    assert_eq!(
        fs::read(fixture.package.join("LICENSE.txt")).unwrap(),
        source_license_before_hide
    );
    assert_eq!(
        fs::read(fixture.package.join("orr.package.json")).unwrap(),
        source_manifest_before_hide
    );

    let report = serde_json::json!({
        "scene_path": fixture.scene.display().to_string(),
        "scene_sha256": sha256(&scene_bytes),
        "scene_checksum": format!("0x{saved_checksum:016x}"),
        "graph_revision": reopened_graph.revision().iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
        "source_package": fixture.package.display().to_string(),
        "source_package_hidden_during_reopen": true,
        "source_package_restored_and_hash_checked": true,
        "source_model_sha256": sha256(&source_glb_before_hide),
        "source_license_sha256": sha256(&source_license_before_hide),
        "source_manifest_sha256": sha256(&source_manifest_before_hide),
        "invalid_package_rejected_without_host_or_history_change": true,
        "candidate_and_stage_left_host_unchanged": true,
        "staged_geometry_undo_redo_verified": true,
        "imported_floor_visible_pixels_changed": staged_pixel_diff,
        "reopen_pixel_diff": reopen_pixel_diff,
        "mission_win_tick": win_tick,
        "mission_win_frame_checksum": format!("0x{win_checksum:016x}"),
        "replay_pixel_diff": replay_pixel_diff,
        "export_directory": fixture.export.display().to_string(),
        "export_manifest_sha256": sha256(&manifest_bytes),
        "export_runtime_sha256": sha256(&fs::read(fixture.export.join("orr_mesh_player.exe")).unwrap()),
        "export_game_profile": manifest["game_profile"],
        "export_input_profile": manifest["input_profile"],
        "export_files": export_names,
        "export_inventory": export_inventory,
        "physical_os_input": false,
        "captures": [
            capture_record(&fixture.root, "blank-mesh.png"),
            capture_record(&fixture.root, "import-candidate.png"),
            capture_record(&fixture.root, "imported-mission-edit.png"),
            capture_record(&fixture.root, "imported-source-free-reopen.png"),
            capture_record(&fixture.root, "imported-mission-won.png"),
            capture_record(&fixture.root, "imported-mission-replay.png"),
        ],
    });
    fs::write(
        fixture.root.join("acceptance.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    fs::write(fixture.root.join("scene.bytes"), scene_bytes).unwrap();
}
