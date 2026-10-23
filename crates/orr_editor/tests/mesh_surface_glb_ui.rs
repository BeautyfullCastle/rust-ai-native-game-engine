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
    Editor, EditorApp,
};
use orr_remote::mesh_navigation::{
    mesh_from_view,
    mission::{self, MissionView},
};
use orr_rhi::{Wgpu, WgpuOptions};
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
    package: PathBuf,
    hidden_package: PathBuf,
    raw_source: PathBuf,
    hidden_raw_source: PathBuf,
    raw_license: PathBuf,
    hidden_raw_license: PathBuf,
    scene: PathBuf,
    physics_scene: PathBuf,
    runtime: PathBuf,
    export: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        assert_eq!(
            std::env::var("ORR_MESH_SURFACE_GLB_ACCEPTANCE").as_deref(),
            Ok("1")
        );
        assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
        assert_eq!(std::env::var("WGPU_BACKEND").unwrap_or_default(), "vulkan");
        let root = PathBuf::from(
            std::env::var_os("ORR_MESH_SURFACE_GLB_CAPTURE_DIR").expect("capture root"),
        );
        assert!(root.is_absolute());
        fs::create_dir(&root).expect("capture root must be fresh");
        let root = root.canonicalize().unwrap();
        fs::create_dir(root.join("captures")).unwrap();

        let raw_source = root.join("floor.glb");
        let raw_license = root.join("LICENSE.txt");
        fs::write(&raw_source, floor_glb()).unwrap();
        fs::write(&raw_license, b"Example walkable-floor license\n").unwrap();
        let package = PathBuf::from(
            std::env::var_os("ORR_MESH_SURFACE_GLB_PACKAGE_OUTPUT").expect("fresh package output"),
        );
        assert!(package.is_absolute() && !package.exists());
        assert_eq!(
            package.parent().unwrap().canonicalize().unwrap(),
            root.parent().unwrap()
        );
        let scene = PathBuf::from(
            std::env::var_os("ORR_MESH_SURFACE_GLB_SCENE")
                .unwrap_or_else(|| root.join("walkable-floor.scene.yaml").into_os_string()),
        );
        assert!(scene.is_absolute() && !scene.exists());
        let scene = scene
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap()
            .join(scene.file_name().unwrap());
        let physics_scene = root.join("owned-stock-physics.scene.yaml");
        fs::copy(EditorGame::PhysGame.default_scene_path(), &physics_scene).unwrap();
        let runtime = PathBuf::from(
            std::env::var_os("ORR_MESH_EXPORT_RUNTIME").expect("matching mesh player"),
        );
        assert!(runtime.is_absolute() && runtime.is_file());
        let export = PathBuf::from(
            std::env::var_os("ORR_MESH_SURFACE_GLB_EXPORT_OUTPUT").expect("fresh export output"),
        );
        assert!(export.is_absolute() && !export.exists());
        assert_eq!(
            export.parent().unwrap().canonicalize().unwrap(),
            root.parent().unwrap()
        );
        assert_ne!(package, export);
        assert_ne!(package, scene);
        assert!(!root.join("hidden-floor.glb").exists());
        assert!(!root.join("hidden-LICENSE.txt").exists());
        Self {
            root: root.clone(),
            package,
            hidden_package: root
                .parent()
                .unwrap()
                .join("hidden-cooked-walkable-package"),
            raw_source,
            hidden_raw_source: root.join("hidden-floor.glb"),
            raw_license,
            hidden_raw_license: root.join("hidden-LICENSE.txt"),
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
        if self.hidden_raw_source.exists() && !self.raw_source.exists() {
            let _ = fs::rename(&self.hidden_raw_source, &self.raw_source);
        }
        if self.hidden_raw_license.exists() && !self.raw_license.exists() {
            let _ = fs::rename(&self.hidden_raw_license, &self.raw_license);
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
        "asset":{"version":"2.0","generator":"Orrery raw GLB surface acceptance"},
        "scene":0,"scenes":[{"nodes":[0]}],
        "nodes":[{"mesh":0,"translation":[1.0,0.0,0.5]}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0,"NORMAL":1},"indices":2,"material":0,"mode":4}]}],
        "materials":[{"pbrMetallicRoughness":{"baseColorFactor":[0.65,0.75,0.85,1.0],"metallicFactor":0.0,"roughnessFactor":1.0}}],
        "buffers":[{"byteLength":binary.len()}],
        "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":72,"target":34962},{"buffer":0,"byteOffset":72,"byteLength":72,"target":34962},{"buffer":0,"byteOffset":144,"byteLength":24,"target":34963}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":6,"type":"VEC3","min":[-4.0,0.0,-2.0],"max":[4.0,0.0,2.0]},{"bufferView":1,"componentType":5126,"count":6,"type":"VEC3"},{"bufferView":2,"componentType":5123,"count":12,"type":"SCALAR","min":[0],"max":[5]}]
    });
    let mut json = serde_json::to_vec(&document).unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    while !binary.len().is_multiple_of(4) {
        binary.push(0);
    }
    let total = 12 + 8 + json.len() + 8 + binary.len();
    let mut glb = Vec::with_capacity(total);
    glb.extend_from_slice(&0x4654_6c67u32.to_le_bytes());
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total as u32).to_le_bytes());
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
    setup.native_adapter_selector = Some(Arc::new(|adapters, _| {
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
            assert!(
                !rhi.is_software(),
                "software rendering is not GPU acceptance"
            );
            true
        }
        Err(error) => {
            assert!(
                std::env::var_os("ORR_MESH_SURFACE_GLB_ACCEPTANCE").is_none()
                    && std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "required hardware Vulkan unavailable: {error}"
            );
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
fn text_field_value(harness: &Harness<'_, EditorApp>, label: &str) -> String {
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .accesskit_node()
        .value()
        .expect("accessible text input value")
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
            "timed out waiting for {operation}: game={:?} path={:?} checksum={} status={:?}",
            harness.state().editor.game(),
            harness.state().editor.path(),
            harness.state().editor.checksum(),
            harness.state().editor.status()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn wait_for_label(harness: &mut Harness<'_, EditorApp>, label: &str) {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        settle(harness);
        if harness.query_by_label_contains(label).is_some() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for UI status {label}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn viewport_pixels(harness: &mut Harness<'_, EditorApp>) -> ((u32, u32), Vec<u8>) {
    harness.render().expect("render production viewport");
    let gpu = harness.state().viewport3d_gpu().unwrap().gpu();
    let size = gpu.target().size();
    let pixels = gpu.read_rgba8();
    assert_eq!(pixels.len(), (size.0 * size.1 * 4) as usize);
    (size, pixels)
}
fn capture(harness: &mut Harness<'_, EditorApp>, root: &Path, name: &str) {
    settle(harness);
    let frame = harness.render().unwrap();
    let image = egui::ColorImage::from_rgba_unmultiplied(
        [frame.width() as usize, frame.height() as usize],
        frame.as_raw(),
    );
    orr_editor::app::write_png(&root.join("captures").join(name), &image).unwrap();
}
fn changed_pixels(left: &[u8], right: &[u8]) -> usize {
    assert_eq!(left.len(), right.len());
    left.as_chunks::<4>().0.iter()
        .zip(right.as_chunks::<4>().0.iter())
        .filter(|(a, b)| {
            a[..3]
                .iter()
                .zip(b[..3].iter())
                .any(|(x, y)| x.abs_diff(*y) > 12)
        })
        .count()
}
fn mission_view(app: &EditorApp) -> MissionView {
    mission::from_view(app.editor.snapshot().unwrap().predicted())
        .unwrap()
        .expect("authored mission")
}
fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn record(root: &Path, name: &str) -> Value {
    let bytes = fs::read(root.join("captures").join(name)).unwrap();
    serde_json::json!({"name":name,"bytes":bytes.len(),"sha256":sha256(&bytes)})
}

#[test]
#[ignore = "requires ORR_MESH_SURFACE_GLB_ACCEPTANCE=1 and hardware Vulkan"]
fn raw_glb_is_cooked_staged_saved_reopened_played_and_exported_without_sources() {
    assert!(gpu_available());
    let fixture = Fixture::new();
    let raw_source_before = fs::read(&fixture.raw_source).unwrap();
    let raw_license_before = fs::read(&fixture.raw_license).unwrap();
    let mut harness = fixture.harness();
    harness.run_steps(4);
    menu_click(&mut harness, "File", "New mesh scene…");
    type_field(
        &mut harness,
        "Scene path",
        &fixture.scene.display().to_string(),
    );
    click_button(&mut harness, "Create");
    wait_for(&mut harness, "new MeshNavigation scene", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && app.editor.path().as_deref() == Some(fixture.scene.as_path())
    });
    if harness.query_by_label("Static-mesh navigation").is_some() {
        click(&mut harness, "Static-mesh navigation");
    }
    click_button(&mut harness, "Create ramp and bridge");
    click_button(&mut harness, "Place mesh agent");
    wait_for(&mut harness, "initial graph and pin", |app| {
        !app.editor.history().entries.is_empty()
            && mesh_from_view(app.editor.snapshot().unwrap().predicted()).is_ok()
    });
    let before_cook_checksum = harness.state().editor.checksum();
    let before_cook_history = harness.state().editor.history().entries.len();
    let original_pixels = viewport_pixels(&mut harness).1;
    capture(&mut harness, &fixture.root, "raw-source-ready.png");

    if harness
        .query_by_label("Cook GLB as walkable package")
        .is_none()
    {
        click(&mut harness, "Static-mesh navigation");
    }
    click(&mut harness, "Cook GLB as walkable package");
    for (label, value) in [
        (
            "Walkable GLB source",
            fixture.raw_source.display().to_string(),
        ),
        (
            "Walkable model license file",
            fixture.raw_license.display().to_string(),
        ),
        (
            "New walkable model package directory",
            fixture.package.display().to_string(),
        ),
        ("Walkable package name", "walkable-floor".to_owned()),
        ("Walkable package version", "1.0.0".to_owned()),
    ] {
        type_field(&mut harness, label, &value);
    }
    click(
        &mut harness,
        "I have reviewed rights to use this walkable model",
    );
    click_button(&mut harness, "Cook walkable model package");
    wait_for_label(&mut harness, "Walkable model package created at");
    assert!(fixture.package.join("orr.package.json").is_file());
    assert!(fixture.package.join("model.glb").is_file());
    assert!(fixture.package.join("LICENSE.txt").is_file());
    assert_eq!(harness.state().editor.checksum(), before_cook_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        before_cook_history
    );
    let cooked_manifest = fs::read(fixture.package.join("orr.package.json")).unwrap();
    let cooked_glb = fs::read(fixture.package.join("model.glb")).unwrap();
    let cooked_license = fs::read(fixture.package.join("LICENSE.txt")).unwrap();
    assert_eq!(cooked_glb, raw_source_before);
    assert_eq!(cooked_license, raw_license_before);
    capture(&mut harness, &fixture.root, "cooked-package-ready.png");
    click(&mut harness, "Cook GLB as walkable package");

    if harness.query_by_label("Import walkable model").is_none() {
        click(&mut harness, "Static-mesh navigation");
    }
    click(&mut harness, "Import walkable model");
    assert_eq!(
        PathBuf::from(text_field_value(&harness, "Walkable model package root")),
        fixture.package.canonicalize().unwrap()
    );
    assert_eq!(
        text_field_value(&harness, "Walkable model asset"),
        "model.glb"
    );
    assert_eq!(
        text_field_value(&harness, "Walkable model license asset"),
        "LICENSE.txt"
    );
    assert_eq!(
        harness
            .get_by_label("I have reviewed the rights to use this model license")
            .accesskit_node()
            .toggled(),
        Some(Toggled::False)
    );
    assert_eq!(
        harness
            .get_by_label("I confirm replacing staged geometry with this imported walkable surface")
            .accesskit_node()
            .toggled(),
        Some(Toggled::False)
    );
    assert!(harness
        .get_by_role_and_label(Role::Button, "Stage imported walkable surface")
        .accesskit_node()
        .is_disabled());
    click(
        &mut harness,
        "I have reviewed the rights to use this model license",
    );
    click(
        &mut harness,
        "I confirm replacing staged geometry with this imported walkable surface",
    );
    assert!(!harness
        .get_by_role_and_label(Role::Button, "Stage imported walkable surface")
        .accesskit_node()
        .is_disabled());
    click_button(&mut harness, "Stage imported walkable surface");
    if harness
        .query_by_label_contains("PENDING GEOMETRY PREVIEW · host unchanged")
        .is_none()
    {
        capture(&mut harness, &fixture.root, "raw-glb-stage-rejected.png");
        panic!("raw GLB surface was not visibly staged");
    }
    assert_eq!(harness.state().editor.checksum(), before_cook_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        before_cook_history
    );
    harness.state_mut().editor.fit_camera();
    let staged_pixels = viewport_pixels(&mut harness).1;
    let staged_diff = changed_pixels(&original_pixels, &staged_pixels);
    assert!(
        staged_diff > 20,
        "raw GLB floor candidate was not visible: {staged_diff}"
    );
    capture(&mut harness, &fixture.root, "raw-glb-staged.png");
    click_button(&mut harness, "Undo staged geometry");
    assert!(changed_pixels(&staged_pixels, &viewport_pixels(&mut harness).1) > 20);
    click_button(&mut harness, "Redo staged geometry");
    assert!(changed_pixels(&staged_pixels, &viewport_pixels(&mut harness).1) < 20);

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
    wait_for(
        &mut harness,
        "agent placement on cooked GLB geometry",
        |app| {
            app.editor.checksum() != before_cook_checksum
                && mesh_from_view(app.editor.snapshot().unwrap().predicted()).is_ok()
        },
    );
    let graph = mesh_from_view(harness.state().editor.snapshot().unwrap().predicted())
        .unwrap()
        .graph;
    assert_eq!(graph.triangles().len(), 4);
    if harness.query_by_label("Enable mesh mission").is_none() {
        click(&mut harness, "Mesh mission");
    }
    click(&mut harness, "Enable mesh mission");
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
    click_button(&mut harness, "Apply mesh mission");
    wait_for(&mut harness, "mission application", |app| {
        mission::from_view(app.editor.snapshot().unwrap().predicted())
            .is_ok_and(|item| item.is_some())
    });
    let saved_checksum = harness.state().editor.checksum();
    let mission_config = mission_view(harness.state()).config;
    harness.state_mut().editor.fit_camera();
    let saved_pixels = viewport_pixels(&mut harness).1;
    menu_click(&mut harness, "File", "Save mesh workspace");
    let scene_bytes = fs::read(&fixture.scene).unwrap();
    capture(&mut harness, &fixture.root, "raw-glb-saved.png");
    drop(harness);

    fs::rename(&fixture.package, &fixture.hidden_package).unwrap();
    fs::rename(&fixture.raw_source, &fixture.hidden_raw_source).unwrap();
    fs::rename(&fixture.raw_license, &fixture.hidden_raw_license).unwrap();
    let mut reopened = fixture.harness();
    reopened.run_steps(4);
    menu_click(&mut reopened, "File", "Open mesh scene…");
    type_field(
        &mut reopened,
        "Scene path",
        &fixture.scene.display().to_string(),
    );
    click_button(&mut reopened, "OK");
    wait_for(
        &mut reopened,
        "source-free GLB cooked scene reopen",
        |app| {
            app.editor.game() == EditorGame::MeshNavigation
                && app.editor.checksum() == saved_checksum
                && app.editor.path().as_deref() == Some(fixture.scene.as_path())
        },
    );
    assert_eq!(
        mesh_from_view(reopened.state().editor.snapshot().unwrap().predicted())
            .unwrap()
            .graph
            .revision(),
        graph.revision()
    );
    assert_eq!(mission_view(reopened.state()).config, mission_config);
    reopened.state_mut().editor.fit_camera();
    let reopen_diff = changed_pixels(&saved_pixels, &viewport_pixels(&mut reopened).1);
    assert!(
        reopen_diff < 20,
        "source-free reopened scene differs: {reopen_diff}"
    );
    capture(
        &mut reopened,
        &fixture.root,
        "raw-glb-source-free-reopen.png",
    );
    assert!(
        !fixture.package.exists() && !fixture.raw_source.exists() && !fixture.raw_license.exists()
    );
    let win_tick = advance_until_won(&mut reopened);
    let win_checksum = reopened
        .state()
        .editor
        .snapshot()
        .unwrap()
        .predicted()
        .checksum();
    capture(&mut reopened, &fixture.root, "raw-glb-mission-won.png");
    click_button(&mut reopened, LBL_STOP);
    assert_eq!(reopened.state().editor.checksum(), saved_checksum);
    let replay_tick = advance_until_won(&mut reopened);
    let replay_checksum = reopened
        .state()
        .editor
        .snapshot()
        .unwrap()
        .predicted()
        .checksum();
    assert_eq!((replay_tick, replay_checksum), (win_tick, win_checksum));
    capture(&mut reopened, &fixture.root, "raw-glb-mission-replay.png");
    click_button(&mut reopened, LBL_STOP);
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
    let mut expected = vec![
        scene_name.into_owned(),
        "orr.mesh-bundle.json".into(),
        "orr_mesh_player.exe".into(),
    ];
    expected.sort();
    assert_eq!(export_names, expected);
    let manifest_bytes = fs::read(fixture.export.join("orr.mesh-bundle.json")).unwrap();
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(manifest["game_profile"], "mesh-mission-v1");
    assert_eq!(manifest["input_profile"], "mesh-click-to-move-v1");
    assert_eq!(manifest["checksum"], saved_checksum);
    assert_eq!(
        fs::read(fixture.export.join(fixture.scene.file_name().unwrap())).unwrap(),
        scene_bytes
    );
    fs::rename(&fixture.hidden_package, &fixture.package).unwrap();
    fs::rename(&fixture.hidden_raw_source, &fixture.raw_source).unwrap();
    fs::rename(&fixture.hidden_raw_license, &fixture.raw_license).unwrap();
    assert_eq!(fs::read(&fixture.raw_source).unwrap(), raw_source_before);
    assert_eq!(fs::read(&fixture.raw_license).unwrap(), raw_license_before);
    assert_eq!(
        fs::read(fixture.package.join("orr.package.json")).unwrap(),
        cooked_manifest
    );
    assert_eq!(
        fs::read(fixture.package.join("model.glb")).unwrap(),
        cooked_glb
    );
    assert_eq!(
        fs::read(fixture.package.join("LICENSE.txt")).unwrap(),
        cooked_license
    );
    let report = serde_json::json!({"scene_path":fixture.scene.display().to_string(),"scene_sha256":sha256(&scene_bytes),"scene_checksum":format!("{saved_checksum:016x}"),"graph_revision":graph.revision().iter().map(|byte|format!("{byte:02x}")).collect::<String>(),"raw_glb_path":fixture.raw_source.display().to_string(),"raw_glb_sha256":sha256(&raw_source_before),"raw_license_sha256":sha256(&raw_license_before),"cooked_package":fixture.package.display().to_string(),"cooked_model_sha256":sha256(&cooked_glb),"cooked_license_sha256":sha256(&cooked_license),"cooked_manifest_sha256":sha256(&cooked_manifest),"source_files_hidden_during_reopen":true,"source_files_restored_and_hash_checked":true,"candidate_and_stage_left_host_unchanged":true,"staged_geometry_undo_redo_verified":true,"staged_visible_pixel_diff":staged_diff,"reopen_pixel_diff":reopen_diff,"mission_win_tick":win_tick,"mission_win_frame_checksum":format!("{win_checksum:016x}"),"replay_tick":replay_tick,"replay_checksum":format!("{replay_checksum:016x}"),"export_directory":fixture.export.display().to_string(),"export_manifest_sha256":sha256(&manifest_bytes),"export_runtime_sha256":sha256(&fs::read(fixture.export.join("orr_mesh_player.exe")).unwrap()),"export_game_profile":manifest["game_profile"],"export_input_profile":manifest["input_profile"],"export_files":export_names,"physical_os_input":false,"captures":[record(&fixture.root,"raw-source-ready.png"),record(&fixture.root,"cooked-package-ready.png"),record(&fixture.root,"raw-glb-staged.png"),record(&fixture.root,"raw-glb-saved.png"),record(&fixture.root,"raw-glb-source-free-reopen.png"),record(&fixture.root,"raw-glb-mission-won.png"),record(&fixture.root,"raw-glb-mission-replay.png")]});
    fs::write(
        fixture.root.join("acceptance.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}

fn advance_until_won(harness: &mut Harness<'_, EditorApp>) -> u64 {
    click_button(harness, LBL_PLAY);
    wait_for(harness, "Mesh Play", |app| {
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
        let view = mesh_from_view(harness.state().editor.snapshot().unwrap().predicted()).unwrap();
        let mission = mission_view(harness.state());
        if mission.run.won == 1 {
            assert_eq!(mission.run.collected, (1u32 << mission.config.count) - 1);
            let distance_squared = (0..3)
                .map(|axis| {
                    let delta = i128::from(view.agent.position[axis].raw())
                        - i128::from(mission.config.exit.position[axis].raw());
                    delta * delta
                })
                .sum::<i128>();
            let exit_radius = i128::from(mission.config.exit.radius.raw());
            assert!(distance_squared <= exit_radius * exit_radius);
            return harness.state().editor.timeline().unwrap().tick;
        }
        click_button(harness, LBL_STEP);
    }
    if let Some(root) = std::env::var_os("ORR_MESH_SURFACE_GLB_CAPTURE_DIR") {
        capture(harness, Path::new(&root), "raw-glb-mission-incomplete.png");
    }
    let view = mesh_from_view(harness.state().editor.snapshot().unwrap().predicted()).unwrap();
    let mission = mission_view(harness.state());
    panic!(
        "mission incomplete: position={:?}, collected={}, configured={}",
        view.agent.position, mission.run.collected, mission.config.count
    )
}
