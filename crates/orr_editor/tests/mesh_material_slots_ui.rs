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
            std::env::var("ORR_MESH_MATERIAL_SLOTS_ACCEPTANCE").as_deref(),
            Ok("1")
        );
        assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
        assert_eq!(std::env::var("WGPU_BACKEND").unwrap_or_default(), "vulkan");
        let root = PathBuf::from(
            std::env::var_os("ORR_MESH_MATERIAL_SLOTS_CAPTURE_DIR").expect("capture root"),
        );
        assert!(root.is_absolute());
        fs::create_dir(&root).expect("capture root must be fresh");
        let root = root.canonicalize().unwrap();
        fs::create_dir(root.join("captures")).unwrap();
        let raw_source = root.join("two-material-floor.glb");
        let raw_license = root.join("LICENSE.txt");
        fs::write(&raw_source, floor_glb()).unwrap();
        fs::write(&raw_license, b"Two-material walkable floor license\n").unwrap();
        let package = PathBuf::from(
            std::env::var_os("ORR_MESH_MATERIAL_SLOTS_PACKAGE_OUTPUT")
                .expect("fresh package output"),
        );
        assert!(package.is_absolute() && !package.exists());
        assert_eq!(
            package.parent().unwrap().canonicalize().unwrap(),
            root.parent().unwrap()
        );
        let scene = PathBuf::from(
            std::env::var_os("ORR_MESH_MATERIAL_SLOTS_SCENE")
                .unwrap_or_else(|| root.join("material-slots.scene.yaml").into_os_string()),
        );
        assert!(scene.is_absolute() && !scene.exists());
        let scene = scene
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap()
            .join(scene.file_name().unwrap());
        assert_eq!(scene.parent().unwrap(), root.as_path());
        let physics_scene = root.join("owned-stock-physics.scene.yaml");
        fs::copy(EditorGame::PhysGame.default_scene_path(), &physics_scene).unwrap();
        let runtime = PathBuf::from(
            std::env::var_os("ORR_MESH_EXPORT_RUNTIME").expect("matching material-slot player"),
        );
        assert!(runtime.is_absolute() && runtime.is_file());
        let export = PathBuf::from(
            std::env::var_os("ORR_MESH_MATERIAL_SLOTS_EXPORT_OUTPUT").expect("fresh export output"),
        );
        assert!(export.is_absolute() && !export.exists());
        assert_eq!(
            export.parent().unwrap().canonicalize().unwrap(),
            root.parent().unwrap()
        );
        assert_ne!(package, export);
        assert_ne!(package, scene);
        Self {
            root: root.clone(),
            package,
            hidden_package: root.parent().unwrap().join("hidden-two-material-package"),
            raw_source,
            hidden_raw_source: root.join("hidden-two-material-floor.glb"),
            raw_license,
            hidden_raw_license: root.join("hidden-two-material-LICENSE.txt"),
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
    let first_indices: [u16; 6] = [0, 3, 2, 0, 2, 1];
    let second_indices: [u16; 6] = [1, 2, 5, 1, 5, 4];
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
    let first_offset = binary.len();
    for index in first_indices {
        binary.extend_from_slice(&index.to_le_bytes());
    }
    let second_offset = binary.len();
    for index in second_indices {
        binary.extend_from_slice(&index.to_le_bytes());
    }
    let document = serde_json::json!({
        "asset":{"version":"2.0","generator":"Orrery two-material acceptance"},
        "scene":0,"scenes":[{"nodes":[0]}],
        "nodes":[{"mesh":0,"translation":[1.0,0.0,0.5]}],
        "meshes":[{"primitives":[
            {"attributes":{"POSITION":0,"NORMAL":1},"indices":2,"material":0,"mode":4},
            {"attributes":{"POSITION":0,"NORMAL":1},"indices":3,"material":1,"mode":4}
        ]}],
        "materials":[
            {"pbrMetallicRoughness":{"baseColorFactor":[0.65,0.75,0.85,1.0],"metallicFactor":0.0,"roughnessFactor":1.0}},
            {"pbrMetallicRoughness":{"baseColorFactor":[0.9,0.8,0.7,1.0],"metallicFactor":0.0,"roughnessFactor":1.0}}
        ],
        "buffers":[{"byteLength":binary.len()}],
        "bufferViews":[
            {"buffer":0,"byteOffset":0,"byteLength":72,"target":34962},
            {"buffer":0,"byteOffset":72,"byteLength":72,"target":34962},
            {"buffer":0,"byteOffset":first_offset,"byteLength":12,"target":34963},
            {"buffer":0,"byteOffset":second_offset,"byteLength":12,"target":34963}
        ],
        "accessors":[
            {"bufferView":0,"componentType":5126,"count":6,"type":"VEC3","min":[-4.0,0.0,-2.0],"max":[4.0,0.0,2.0]},
            {"bufferView":1,"componentType":5126,"count":6,"type":"VEC3"},
            {"bufferView":2,"componentType":5123,"count":6,"type":"SCALAR","min":[0],"max":[3]},
            {"bufferView":3,"componentType":5123,"count":6,"type":"SCALAR","min":[1],"max":[5]}
        ]
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
                std::env::var_os("ORR_MESH_MATERIAL_SLOTS_ACCEPTANCE").is_none()
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

fn choose_combo(harness: &mut Harness<'_, EditorApp>, label: &str, option: &str) {
    harness
        .get_by_role_and_label(Role::ComboBox, label)
        .scroll_to_me();
    harness.run_steps(30);
    harness.get_by_role_and_label(Role::ComboBox, label).click();
    harness.run_steps(3);
    harness
        .get_by_role_and_label(Role::Button, option)
        .scroll_to_me();
    harness.run_steps(3);
    harness.get_by_role_and_label(Role::Button, option).click();
    settle(harness);
    assert_eq!(
        harness
            .get_by_role_and_label(Role::ComboBox, label)
            .accesskit_node()
            .value()
            .as_deref(),
        Some(option)
    );
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
            "timed out waiting for {operation}: game={:?} path={:?} checksum={} status={:?}",
            harness.state().editor.game(),
            harness.state().editor.path(),
            harness.state().editor.checksum(),
            harness.state().editor.status()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn viewport_pixels(harness: &mut Harness<'_, EditorApp>) -> Vec<u8> {
    harness.render().expect("render production viewport");
    harness.state().viewport3d_gpu().unwrap().gpu().read_rgba8()
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

fn capture_record(root: &Path, name: &str) -> Value {
    let bytes = fs::read(root.join("captures").join(name)).unwrap();
    serde_json::json!({"name":name,"bytes":bytes.len(),"sha256":sha256(&bytes)})
}

fn changed_pixels(left: &[u8], right: &[u8]) -> usize {
    assert_eq!(left.len(), right.len());
    left.chunks_exact(4)
        .zip(right.chunks_exact(4))
        .filter(|(left_pixel, right_pixel)| {
            left_pixel[..3]
                .iter()
                .zip(right_pixel[..3].iter())
                .any(|(left, right)| left.abs_diff(*right) > 12)
        })
        .count()
}

fn wait_for_label(harness: &mut Harness<'_, EditorApp>, label: &str) {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        settle(harness);
        if harness.query_by_label_contains(label).is_some() {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {label}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn mission_view(app: &EditorApp) -> MissionView {
    mission::from_view(app.editor.snapshot().unwrap().predicted())
        .unwrap()
        .expect("authored mission")
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
        let mission = mission_view(harness.state());
        if mission.run.won == 1 {
            assert_eq!(mission.run.collected, (1u32 << mission.config.count) - 1);
            return harness.state().editor.timeline().unwrap().tick;
        }
        click_button(harness, LBL_STEP);
    }
    panic!(
        "mission did not complete at the configured exit: {:?}",
        mesh_from_view(harness.state().editor.snapshot().unwrap().predicted())
            .unwrap()
            .agent
            .position
    );
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
#[ignore = "requires ORR_MESH_MATERIAL_SLOTS_ACCEPTANCE=1 and hardware Vulkan"]
fn material_slot_overrides_survive_save_reopen_play_and_source_free_export() {
    assert!(gpu_available());
    let fixture = Fixture::new();
    let source_glb_bytes = fs::read(&fixture.raw_source).unwrap();
    let license_bytes = fs::read(&fixture.raw_license).unwrap();
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
    let host_checksum = harness.state().editor.checksum();
    let host_history = harness.state().editor.history().entries.len();

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
        ("Walkable package name", "two-material-floor".to_owned()),
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
    let cooked_manifest = fs::read(fixture.package.join("orr.package.json")).unwrap();
    let cooked_glb = fs::read(fixture.package.join("model.glb")).unwrap();
    let cooked_license = fs::read(fixture.package.join("LICENSE.txt")).unwrap();
    assert_eq!(cooked_glb, source_glb_bytes);
    assert_eq!(cooked_license, license_bytes);
    let cooked_manifest_value: Value = serde_json::from_slice(&cooked_manifest).unwrap();
    assert_eq!(
        cooked_manifest_value["capabilities"],
        serde_json::json!(["models"])
    );
    click(&mut harness, "Cook GLB as walkable package");

    if harness.query_by_label("Import walkable model").is_none() {
        click(&mut harness, "Static-mesh navigation");
    }
    click(&mut harness, "Import walkable model");
    click(
        &mut harness,
        "I have reviewed the rights to use this model license",
    );
    click(
        &mut harness,
        "I confirm replacing staged geometry with this imported walkable surface",
    );
    click_button(&mut harness, "Stage imported walkable surface");
    wait_for_label(&mut harness, "PENDING GEOMETRY PREVIEW · host unchanged");
    assert_eq!(harness.state().editor.checksum(), host_checksum);
    assert_eq!(harness.state().editor.history().entries.len(), host_history);
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
        "placed agent on the imported two-slot floor",
        |app| {
            app.editor.checksum() != host_checksum
                && mesh_from_view(app.editor.snapshot().unwrap().predicted())
                    .is_ok_and(|view| view.graph.triangles().len() == 4)
        },
    );
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
    wait_for(&mut harness, "authored mission", |app| {
        mission::from_view(app.editor.snapshot().unwrap().predicted())
            .is_ok_and(|mission| mission.is_some())
    });
    let checksum_before_art = harness.state().editor.checksum();
    let history_before_art = harness.state().editor.history().entries.len();

    if harness
        .query_by_label("Art package source directory")
        .is_none()
    {
        click(&mut harness, "Mesh agent art");
    }
    for (label, value) in [
        (
            "Art package source directory",
            fixture.package.display().to_string(),
        ),
        ("Art model asset", "model.glb".to_owned()),
        ("Art license asset", "LICENSE.txt".to_owned()),
    ] {
        type_field(&mut harness, label, &value);
    }
    click(&mut harness, "I have reviewed rights to use this agent art");
    click_button(&mut harness, "Load agent art");
    wait_for_label(&mut harness, "Loaded static package art");
    type_field(&mut harness, "Agent art offset Y", "0.2");
    click_button(&mut harness, "Apply agent art");
    if harness.query_by_label("Material slot index").is_none() {
        click(&mut harness, "Per-material-slot overrides");
    }
    assert_eq!(
        harness
            .get_by_label("Material slot index")
            .accesskit_node()
            .role(),
        Role::ComboBox
    );
    assert_eq!(
        harness
            .get_by_role_and_label(Role::TextInput, "Material slot tint R")
            .accesskit_node()
            .value()
            .as_deref(),
        Some("1")
    );
    choose_combo(&mut harness, "Material slot index", "1");
    assert_eq!(
        harness
            .get_by_role_and_label(Role::TextInput, "Material slot tint R")
            .accesskit_node()
            .value()
            .as_deref(),
        Some("1")
    );
    assert_eq!(
        harness
            .get_by_label("Material slot linear filtering")
            .accesskit_node()
            .toggled(),
        Some(Toggled::False)
    );
    choose_combo(&mut harness, "Material slot index", "0");
    assert_eq!(
        harness
            .get_by_label("Material slot linear filtering")
            .accesskit_node()
            .toggled(),
        Some(Toggled::False)
    );
    let base_art_pixels = viewport_pixels(&mut harness);
    capture(&mut harness, &fixture.root, "slots-unmodified.png");
    type_field(&mut harness, "Material slot tint R", "0.35");
    click(&mut harness, "Material slot linear filtering");
    choose_combo(&mut harness, "Material slot wrap S", "Repeat");
    click_button(&mut harness, "Stage material overrides");
    assert_eq!(harness.state().editor.checksum(), checksum_before_art);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        history_before_art
    );
    click_button(&mut harness, "Apply agent art");
    let applied_pixels = viewport_pixels(&mut harness);
    let apply_diff = changed_pixels(&base_art_pixels, &applied_pixels);
    assert!(
        apply_diff > 20,
        "material override did not affect rendering: {apply_diff}"
    );
    capture(&mut harness, &fixture.root, "slots-applied.png");
    assert_eq!(harness.state().editor.checksum(), checksum_before_art);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        history_before_art
    );
    click_button(&mut harness, "Undo agent art");
    let undo_pixels = viewport_pixels(&mut harness);
    assert!(changed_pixels(&applied_pixels, &undo_pixels) > 20);
    capture(&mut harness, &fixture.root, "slots-undo.png");
    assert!(changed_pixels(&base_art_pixels, &undo_pixels) < 20);
    click_button(&mut harness, "Redo agent art");
    let redo_pixels = viewport_pixels(&mut harness);
    assert!(changed_pixels(&applied_pixels, &redo_pixels) < 20);
    capture(&mut harness, &fixture.root, "slots-redo.png");
    assert_eq!(harness.state().editor.checksum(), checksum_before_art);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        history_before_art
    );

    let sidecar = PathBuf::from(format!("{}.mesh-art.json", fixture.scene.display()));
    let old_art_sha = fs::read(&sidecar).ok().map(|bytes| sha256(&bytes));
    menu_click(&mut harness, "File", "Save mesh workspace");
    let scene_bytes = fs::read(&fixture.scene).unwrap();
    let art_bytes = fs::read(&sidecar).unwrap();
    assert_ne!(old_art_sha, Some(sha256(&art_bytes)));
    let art_json: Value = serde_json::from_slice(&art_bytes).unwrap();
    assert_eq!(art_json["material_slots"].as_array().unwrap().len(), 1);
    assert_eq!(art_json["material_slots"][0]["material"], 0);
    assert_eq!(
        art_json["material_slots"][0]["tint"],
        serde_json::json!([0.35, 1.0, 1.0, 1.0])
    );
    assert_eq!(art_json["material_slots"][0]["linear_filter"], true);
    assert_eq!(art_json["material_slots"][0]["wrap_s"], "repeat");
    assert_eq!(art_json["material_slots"][0]["wrap_t"], "clamp");
    assert_eq!(
        art_json["model"]["materials"][0]["base_color"],
        serde_json::json!([0.65, 0.75, 0.85, 1.0])
    );
    assert_eq!(art_json["model"]["materials"][0]["linear_filter"], false);
    assert_eq!(art_json["model"]["materials"][0]["wrap_s"], "clamp");
    assert_eq!(
        art_json["model"]["materials"][1]["base_color"],
        serde_json::json!([0.9, 0.8, 0.7, 1.0])
    );
    assert_eq!(art_json["model"]["materials"][1]["linear_filter"], false);
    assert_eq!(art_json["asset_sha256"], sha256(&cooked_glb));
    assert_eq!(art_json["license_sha256"], sha256(&cooked_license));
    assert!(art_json["cooked_sha256"].as_str().is_some());
    let saved_checksum = harness.state().editor.checksum();
    let saved_pixels = viewport_pixels(&mut harness);
    capture(&mut harness, &fixture.root, "slots-saved.png");
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
        "source-free material-slot scene reopen",
        |app| {
            app.editor.game() == EditorGame::MeshNavigation
                && app.editor.checksum() == saved_checksum
                && app.editor.path().as_deref() == Some(fixture.scene.as_path())
        },
    );
    assert_eq!(fs::read(&sidecar).unwrap(), art_bytes);
    reopened.state_mut().editor.fit_camera();
    let reopened_pixels = viewport_pixels(&mut reopened);
    let reopen_diff = changed_pixels(&saved_pixels, &reopened_pixels);
    assert!(
        reopen_diff < 20,
        "source-free material override changed pixels: {reopen_diff}"
    );
    capture(&mut reopened, &fixture.root, "slots-source-free-reopen.png");
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
    capture(&mut reopened, &fixture.root, "slots-mission-won.png");
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
    capture(&mut reopened, &fixture.root, "slots-mission-replay.png");
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
    wait_for_label(&mut reopened, "Mesh export complete:");
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
    let mut expected = vec![
        scene_name,
        "orr.mesh-bundle.json".into(),
        "orr_mesh_player.exe".into(),
        sidecar.file_name().unwrap().to_string_lossy().into_owned(),
    ];
    expected.sort();
    assert_eq!(export_names, expected);
    let manifest_bytes = fs::read(fixture.export.join("orr.mesh-bundle.json")).unwrap();
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(manifest["game_profile"], "mesh-mission-v1");
    assert_eq!(manifest["input_profile"], "mesh-click-to-move-v1");
    assert_eq!(manifest["sidecar"]["sha256"], sha256(&art_bytes));
    assert_eq!(
        manifest["sidecar"]["name"],
        sidecar.file_name().unwrap().to_string_lossy().as_ref()
    );
    assert_eq!(manifest["checksum"], saved_checksum);
    assert_eq!(manifest["scene"]["sha256"], sha256(&scene_bytes));
    assert_eq!(
        manifest["runtime"]["sha256"],
        sha256(&fs::read(fixture.export.join("orr_mesh_player.exe")).unwrap())
    );
    assert_eq!(
        fs::read(fixture.export.join(sidecar.file_name().unwrap())).unwrap(),
        art_bytes
    );

    fs::rename(&fixture.hidden_package, &fixture.package).unwrap();
    fs::rename(&fixture.hidden_raw_source, &fixture.raw_source).unwrap();
    fs::rename(&fixture.hidden_raw_license, &fixture.raw_license).unwrap();
    assert_eq!(fs::read(&fixture.raw_source).unwrap(), source_glb_bytes);
    assert_eq!(fs::read(&fixture.raw_license).unwrap(), license_bytes);
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
    let report = serde_json::json!({
        "scene_path":fixture.scene.display().to_string(),
        "scene_sha256":sha256(&scene_bytes),
        "scene_checksum":format!("0x{saved_checksum:016x}"),
        "package_path":fixture.package.display().to_string(),
        "package_manifest_sha256":sha256(&cooked_manifest),
        "package_model_sha256":sha256(&cooked_glb),
        "package_license_sha256":sha256(&cooked_license),
        "used_material_slots":2,
        "material_overrides":art_json["material_slots"],
        "model_source_payload_unchanged":true,
        "untouched_second_material_unchanged":true,
        "art_history_undo_redo_verified":true,
        "material_apply_pixel_diff":apply_diff,
        "undo_restores_prior_art_pixels":true,
        "redo_restores_material_slot_pixels":true,
        "source_hidden_during_reopen":true,
        "source_restored_and_hash_checked":true,
        "reopen_pixel_diff":reopen_diff,
        "mission_win_tick":win_tick,
        "mission_win_frame_checksum":format!("0x{win_checksum:016x}"),
        "replay_tick":replay_tick,
        "replay_checksum":format!("0x{replay_checksum:016x}"),
        "export_directory":fixture.export.display().to_string(),
        "export_manifest_sha256":sha256(&manifest_bytes),
        "export_runtime_sha256":sha256(&fs::read(fixture.export.join("orr_mesh_player.exe")).unwrap()),
        "export_files":export_names,
        "physical_os_input":false,
        "captures":[capture_record(&fixture.root,"slots-unmodified.png"),capture_record(&fixture.root,"slots-applied.png"),capture_record(&fixture.root,"slots-undo.png"),capture_record(&fixture.root,"slots-redo.png"),capture_record(&fixture.root,"slots-saved.png"),capture_record(&fixture.root,"slots-source-free-reopen.png"),capture_record(&fixture.root,"slots-mission-won.png"),capture_record(&fixture.root,"slots-mission-replay.png")]
    });
    fs::write(
        fixture.root.join("acceptance.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}
