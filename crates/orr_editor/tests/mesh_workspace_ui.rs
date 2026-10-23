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
    Editor, EditorApp, HostSpec, Mode, Owner,
};
use orr_reflect::Value;
use orr_rhi::Rhi;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

const SOURCE_PACKAGE: &str = "sample-imported-scene";
const MODEL_ASSET: &str = "foreground.glb";
const LICENSE_ASSET: &str = "LICENSE.txt";

struct Fixture {
    _temporary: tempfile::TempDir,
    physics_scene: PathBuf,
    physics_bytes: Vec<u8>,
    malformed_scene: PathBuf,
    scene: PathBuf,
    other_scene: PathBuf,
    blocked_scene: PathBuf,
    source: PathBuf,
    hidden_source: PathBuf,
    capture_directory: Option<PathBuf>,
    source_model_sha256: String,
    source_license_sha256: String,
    source_hidden: bool,
    source_restored: bool,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let capture_directory =
            std::env::var_os("ORR_MESH_WORKSPACE_CAPTURE_DIR").map(PathBuf::from);
        if let Some(directory) = capture_directory.as_ref() {
            assert!(
                directory.is_absolute(),
                "capture directory must be absolute"
            );
            fs::create_dir(directory).expect("capture directory must be fresh");
        }
        let root = capture_directory
            .as_deref()
            .unwrap_or(temporary.path())
            .canonicalize()
            .unwrap();
        let physics_scene = root.join("owned stock Physics.scene.yaml");
        fs::copy(EditorGame::PhysGame.default_scene_path(), &physics_scene)
            .expect("copy the stock Physics scene into the owned fixture");
        let physics_bytes = fs::read(&physics_scene).unwrap();
        let scene = std::env::var_os("ORR_MESH_WORKSPACE_OUTPUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("mesh-workspace.scene.yaml"));
        assert!(scene.is_absolute(), "mesh scene output must be absolute");
        assert!(scene.parent().is_some_and(Path::is_dir));
        assert!(!scene.exists(), "mesh scene output must be new");
        assert!(
            !sidecar_path(&scene).exists(),
            "mesh sidecar output must be new"
        );
        let malformed_scene = root.join("malformed-mesh.scene.yaml");
        fs::write(&malformed_scene, b"not a valid admitted scene\n").unwrap();
        let other_scene = root.join("other-mesh.scene.yaml");
        let blocked_scene = root.join("blocked-mesh.scene.yaml");
        let source = root.join("local mesh-art package");
        let hidden_source = root.join("local mesh-art package.hidden");
        fs::create_dir(&source).unwrap();
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/imported_scene_demo");
        for file_name in [
            "orr.package.json",
            "foreground.glb",
            "background.glb",
            "LICENSE.txt",
            "generate.py",
        ] {
            fs::copy(assets.join(file_name), source.join(file_name)).unwrap();
        }
        let source_model_sha256 = sha256_hex(&fs::read(source.join(MODEL_ASSET)).unwrap());
        let source_license_sha256 = sha256_hex(&fs::read(source.join(LICENSE_ASSET)).unwrap());
        Self {
            _temporary: temporary,
            physics_scene,
            physics_bytes,
            malformed_scene,
            scene,
            other_scene,
            blocked_scene,
            source,
            hidden_source,
            capture_directory,
            source_model_sha256,
            source_license_sha256,
            source_hidden: false,
            source_restored: false,
        }
    }

    fn harness(&self) -> Harness<'static, EditorApp> {
        let mut editor = Editor::start(&HostSpec::local_game(
            self.physics_scene.clone(),
            EditorGame::PhysGame,
        ))
        .unwrap();
        editor.sync();
        assert_eq!(editor.game(), EditorGame::PhysGame);
        Harness::builder()
            .with_size([1600.0, 1100.0])
            .with_step_dt(1.0 / 60.0)
            .wgpu_setup(hardware_vulkan_setup())
            .build_eframe(move |creation_context| {
                EditorApp::new(editor, creation_context.wgpu_render_state.clone())
            })
    }

    fn hide_source(&mut self) {
        fs::rename(&self.source, &self.hidden_source)
            .expect("hide the local art package after saving embedded art");
        self.source_hidden = true;
        assert!(!self.source.exists());
        assert!(self.hidden_source.is_dir());
    }

    fn restore_source(&mut self) -> Result<(), String> {
        if !self.source_hidden || self.source.exists() || !self.hidden_source.is_dir() {
            return Err("art package source is not in its expected hidden state".into());
        }
        fs::rename(&self.hidden_source, &self.source)
            .map_err(|error| format!("restore art package source: {error}"))?;
        let model_sha256 = sha256_hex(
            &fs::read(self.source.join(MODEL_ASSET))
                .map_err(|error| format!("read restored model asset: {error}"))?,
        );
        let license_sha256 = sha256_hex(
            &fs::read(self.source.join(LICENSE_ASSET))
                .map_err(|error| format!("read restored license: {error}"))?,
        );
        if model_sha256 != self.source_model_sha256 || license_sha256 != self.source_license_sha256
        {
            return Err("restored package source hashes differ from the originals".into());
        }
        self.source_hidden = false;
        self.source_restored = true;
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if self.source_hidden && !self.source.exists() && self.hidden_source.exists() {
            if let Err(error) = fs::rename(&self.hidden_source, &self.source) {
                eprintln!("Could not restore hidden mesh workspace package: {error}");
            }
        }
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
    match orr_rhi::Wgpu::headless(orr_rhi::WgpuOptions::default()) {
        Ok(rhi) => {
            eprintln!(
                "Mesh workspace GPU preflight: {} (software: {})",
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
            let required = std::env::var_os("ORR_REQUIRE_GPU").is_some()
                || std::env::var_os("ORR_MESH_WORKSPACE_ACCEPTANCE").is_some();
            assert!(
                !required,
                "Mesh workspace GPU acceptance unavailable: {error}"
            );
            eprintln!("SKIP: Mesh workspace GPU is unavailable: {error}");
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

fn click_fast(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness.get_by_label(label).click_accesskit();
    harness.run_steps(1);
    harness.state_mut().editor.sync();
}

fn menu_click(harness: &mut Harness<'_, EditorApp>, menu: &str, item: &str) {
    if harness.query_by_label(item).is_none() {
        harness.get_by_label(menu).click_accesskit();
        harness.run_steps(2);
    }
    assert!(
        harness.query_by_label(item).is_some(),
        "missing {item} menu item"
    );
    harness.get_by_label(item).click_accesskit();
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
    loop {
        settle(harness);
        if ready(harness.state()) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {reason}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn path_matches(actual: Option<&Path>, expected: &Path) -> bool {
    actual
        .and_then(|path| path.canonicalize().ok())
        .zip(expected.canonicalize().ok())
        .is_some_and(|(actual_path, expected_path)| actual_path == expected_path)
}

fn sidecar_path(scene: &Path) -> PathBuf {
    let mut name = scene.as_os_str().to_os_string();
    name.push(".mesh-art.json");
    PathBuf::from(name)
}

fn mesh_view(harness: &Harness<'_, EditorApp>) -> orr_remote::mesh_navigation::MeshView {
    let snapshot = harness.state().editor.snapshot().unwrap();
    orr_remote::mesh_navigation::mesh_from_view(snapshot.predicted()).unwrap()
}

fn viewport_pixels(harness: &mut Harness<'_, EditorApp>) -> Vec<u8> {
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
        .expect("the production MeshNavigation viewport rendered");
    let size = viewport.gpu().target().size();
    assert_eq!(
        size, target,
        "Actual GPU viewport dimensions must be stable"
    );
    let pixels = viewport.gpu().read_rgba8();
    assert_eq!(pixels.len(), (size.0 * size.1 * 4) as usize);
    assert!(pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0));
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

fn capture(directory: &Path, filename: &str, harness: &mut Harness<'_, EditorApp>) {
    let pixels = viewport_pixels(harness);
    let viewport = harness.state().viewport3d_gpu().unwrap();
    let size = viewport.gpu().target().size();
    let image =
        egui::ColorImage::from_rgba_unmultiplied([size.0 as usize, size.1 as usize], &pixels);
    orr_editor::app::write_png(&directory.join(filename), &image).unwrap();
}

fn wait_for_playing(harness: &mut Harness<'_, EditorApp>) {
    let deadline = Instant::now() + Duration::from_secs(5);
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

#[test]
fn default_physics_editor_creates_and_opens_mesh_scenes_without_losing_work() {
    let requested = [
        "ORR_REQUIRE_GPU",
        "ORR_MESH_WORKSPACE_ACCEPTANCE",
        "ORR_MESH_WORKSPACE_OUTPUT",
        "ORR_MESH_WORKSPACE_CAPTURE_DIR",
    ]
    .iter()
    .any(|name| std::env::var_os(name).is_some());
    if !requested {
        eprintln!("SKIP: opt into actual Mesh workspace GPU acceptance with ORR_MESH_WORKSPACE_ACCEPTANCE=1");
        return;
    }
    assert_eq!(
        std::env::var("WGPU_BACKEND").unwrap_or_default(),
        "vulkan",
        "Mesh workspace acceptance requires WGPU_BACKEND=vulkan"
    );
    assert!(gpu_available());

    let mut fixture = Fixture::new();
    let capture_directory = fixture.capture_directory.clone();
    let mut harness = fixture.harness();
    harness.run_steps(4);
    assert_eq!(harness.state().editor.game(), EditorGame::PhysGame);
    assert!(harness.state().has_gpu());
    let physics_checksum = harness.state().editor.checksum();

    menu_click(&mut harness, "File", "Open mesh scene…");
    type_scene_path(&mut harness, &fixture.malformed_scene, "OK");
    assert_eq!(harness.state().editor.game(), EditorGame::PhysGame);
    assert!(path_matches(
        harness.state().editor.path().as_deref(),
        &fixture.physics_scene
    ));
    assert_eq!(harness.state().editor.checksum(), physics_checksum);
    assert_eq!(
        fs::read(&fixture.physics_scene).unwrap(),
        fixture.physics_bytes
    );
    assert!(harness.state().ui.dialog.is_some());
    click(&mut harness, "Cancel");

    assert!(harness.state_mut().editor.select_named("body_05"));
    assert!(harness.state_mut().editor.set_field(
        &Owner::Component("orr_physics::Body".into()),
        "pos.x",
        Value::Fixed(orr_fp::FP::from_int(1)),
    ));
    settle(&mut harness);
    let dirty_physics_checksum = harness.state().editor.checksum();
    assert_ne!(dirty_physics_checksum, physics_checksum);
    assert!(harness.state().editor.is_dirty());
    menu_click(&mut harness, "File", "New mesh scene…");
    type_scene_path(&mut harness, &fixture.scene, "Create");
    assert!(
        !fixture.scene.exists(),
        "dirty Physics must refuse creating a mesh scene"
    );
    assert_eq!(harness.state().editor.game(), EditorGame::PhysGame);
    assert_eq!(harness.state().editor.checksum(), dirty_physics_checksum);
    assert!(harness.state().editor.is_dirty());
    assert!(harness.state().ui.dialog.is_some());
    click(&mut harness, "Cancel");
    menu_click(&mut harness, "Edit", "Undo Ctrl+Z");
    assert!(!harness.state().editor.is_dirty());
    assert_eq!(harness.state().editor.checksum(), physics_checksum);
    assert_eq!(
        fs::read(&fixture.physics_scene).unwrap(),
        fixture.physics_bytes
    );

    menu_click(&mut harness, "File", "New mesh scene…");
    type_scene_path(&mut harness, &fixture.scene, "Create");
    wait_for(&mut harness, "new MeshNavigation host", |app| {
        app.editor.game().is_mesh_navigation()
            && path_matches(app.editor.path().as_deref(), &fixture.scene)
    });
    assert!(fixture.scene.is_file());
    assert!(!sidecar_path(&fixture.scene).exists());
    if harness.query_by_label("Create ramp and bridge").is_none() {
        click(&mut harness, "Static-mesh navigation");
    }
    click(&mut harness, "Create ramp and bridge");
    click(&mut harness, "Place mesh agent");
    menu_click(&mut harness, "File", "Save Ctrl+S");
    let saved_scene_bytes = fs::read(&fixture.scene).unwrap();
    let saved_checksum = harness.state().editor.checksum();
    let baseline_view = mesh_view(&harness);
    let graph_revision = baseline_view.graph.revision();
    let baseline_pixels = viewport_pixels(&mut harness);

    if harness
        .query_by_label("Art package source directory")
        .is_none()
    {
        click(&mut harness, "Mesh agent art");
    }
    type_field(
        &mut harness,
        "Art package source directory",
        &fixture.source.display().to_string(),
    );
    type_field(&mut harness, "Art model asset", MODEL_ASSET);
    type_field(&mut harness, "Art license asset", LICENSE_ASSET);
    assert!(harness
        .get_by_label("Load agent art")
        .accesskit_node()
        .is_disabled());
    click(&mut harness, "I have reviewed rights to use this agent art");
    click(&mut harness, "Load agent art");
    assert!(harness
        .query_by_label(&format!(
            "Loaded static package art {SOURCE_PACKAGE}/{MODEL_ASSET}; not applied."
        ))
        .is_some());
    assert_eq!(harness.state().editor.checksum(), saved_checksum);
    assert_eq!(mesh_view(&harness).graph.revision(), graph_revision);
    type_field(&mut harness, "Agent art offset X", "0.25");
    type_field(&mut harness, "Agent art offset Y", "0.5");
    type_field(&mut harness, "Agent art offset Z", "0.25");
    type_field(&mut harness, "Agent art scale", "0.5");
    click(&mut harness, "Apply agent art");
    assert_eq!(harness.state().editor.checksum(), saved_checksum);
    assert!(harness
        .get_by_label(LBL_PLAY)
        .accesskit_node()
        .is_disabled());
    let dirty_art_pixels = viewport_pixels(&mut harness);
    assert!(changed_pixel_count(&baseline_pixels, &dirty_art_pixels) > 20);
    let blocked_open_checksum = harness.state().editor.checksum();
    menu_click(&mut harness, "File", "Open mesh scene…");
    type_scene_path(&mut harness, &fixture.other_scene, "OK");
    assert!(harness.state().editor.game().is_mesh_navigation());
    assert!(path_matches(
        harness.state().editor.path().as_deref(),
        &fixture.scene
    ));
    assert_eq!(harness.state().editor.checksum(), blocked_open_checksum);
    assert!(harness.state().ui.dialog.is_some());
    click(&mut harness, "Cancel");
    menu_click(&mut harness, "File", "New mesh scene…");
    type_scene_path(&mut harness, &fixture.blocked_scene, "Create");
    assert!(!fixture.blocked_scene.exists());
    assert!(path_matches(
        harness.state().editor.path().as_deref(),
        &fixture.scene
    ));
    assert_eq!(harness.state().editor.checksum(), blocked_open_checksum);
    assert!(harness.state().ui.dialog.is_some());
    click(&mut harness, "Cancel");
    click(&mut harness, "Undo agent art");
    assert_eq!(harness.state().editor.checksum(), saved_checksum);
    let reverted_pixels = viewport_pixels(&mut harness);
    assert_eq!(changed_pixel_count(&baseline_pixels, &reverted_pixels), 0);

    type_field(&mut harness, "Agent art offset X", "0.25");
    type_field(&mut harness, "Agent art offset Y", "0.5");
    type_field(&mut harness, "Agent art offset Z", "0.25");
    type_field(&mut harness, "Agent art scale", "0.5");
    click(&mut harness, "Apply agent art");
    click(&mut harness, "Save agent art");
    let sidecar = sidecar_path(&fixture.scene);
    let saved_sidecar_bytes =
        fs::read(&sidecar).expect("Save agent art writes the embedded sidecar");
    assert_eq!(fs::read(&fixture.scene).unwrap(), saved_scene_bytes);
    assert_eq!(harness.state().editor.checksum(), saved_checksum);
    let saved_art: serde_json::Value = serde_json::from_slice(&saved_sidecar_bytes).unwrap();
    assert_eq!(saved_art["manifest"]["name"], SOURCE_PACKAGE);
    assert_eq!(saved_art["asset"], MODEL_ASSET);
    assert_eq!(saved_art["asset_sha256"], fixture.source_model_sha256);
    assert_eq!(saved_art["license"], LICENSE_ASSET);
    assert_eq!(saved_art["license_sha256"], fixture.source_license_sha256);
    assert!(saved_art["model"].is_object());
    assert!(saved_art["cooked_sha256"].as_str().is_some());

    if capture_directory.is_some() {
        capture(
            capture_directory.as_deref().unwrap(),
            "saved-agent-art.png",
            &mut harness,
        );
    }
    menu_click(&mut harness, "File", "New mesh scene…");
    type_scene_path(&mut harness, &fixture.other_scene, "Create");
    wait_for(&mut harness, "second MeshNavigation scene", |app| {
        app.editor.game().is_mesh_navigation()
            && path_matches(app.editor.path().as_deref(), &fixture.other_scene)
    });
    if harness.query_by_label("Create ramp and bridge").is_none() {
        click(&mut harness, "Static-mesh navigation");
    }
    click(&mut harness, "Create ramp and bridge");
    click(&mut harness, "Place mesh agent");
    menu_click(&mut harness, "File", "Save Ctrl+S");
    assert!(fixture.other_scene.is_file());
    menu_click(&mut harness, "File", "Open mesh scene…");
    type_scene_path(&mut harness, &fixture.scene, "OK");
    wait_for(&mut harness, "reopened original mesh scene", |app| {
        app.editor.game().is_mesh_navigation()
            && path_matches(app.editor.path().as_deref(), &fixture.scene)
            && app.editor.checksum() == saved_checksum
    });
    assert_eq!(mesh_view(&harness).graph.revision(), graph_revision);
    assert_eq!(fs::read(&fixture.scene).unwrap(), saved_scene_bytes);
    assert_eq!(fs::read(&sidecar).unwrap(), saved_sidecar_bytes);
    drop(harness);

    fixture.hide_source();
    let source_hidden_during_physics_open = true;
    let mut harness = fixture.harness();
    harness.run_steps(4);
    assert_eq!(harness.state().editor.game(), EditorGame::PhysGame);
    menu_click(&mut harness, "File", "Open mesh scene…");
    type_scene_path(&mut harness, &fixture.scene, "OK");
    wait_for(
        &mut harness,
        "MeshNavigation host loaded from Physics editor",
        |app| {
            app.editor.game().is_mesh_navigation()
                && path_matches(app.editor.path().as_deref(), &fixture.scene)
                && app.editor.checksum() == saved_checksum
        },
    );
    assert_eq!(mesh_view(&harness).graph.revision(), graph_revision);
    assert_eq!(fs::read(&fixture.scene).unwrap(), saved_scene_bytes);
    assert_eq!(fs::read(&sidecar).unwrap(), saved_sidecar_bytes);
    assert!(!fixture.source.exists());
    let reopened_pixels = viewport_pixels(&mut harness);
    assert!(changed_pixel_count(&dirty_art_pixels, &reopened_pixels) < 20);
    if capture_directory.is_some() {
        capture(
            capture_directory.as_deref().unwrap(),
            "physics-opened-agent-art.png",
            &mut harness,
        );
    }

    click_fast(&mut harness, LBL_PLAY);
    wait_for_playing(&mut harness);
    click_fast(&mut harness, LBL_PAUSE);
    assert!(!harness.state().editor.timeline().unwrap().playing);
    harness.state_mut().editor.seek(0);
    settle(&mut harness);
    let initial_position = mesh_view(&harness).agent.position;
    for _ in 0..16 {
        click_fast(&mut harness, LBL_STEP);
    }
    let stepped_view = mesh_view(&harness);
    assert_eq!(
        stepped_view.agent.status,
        orr_navigation::NavigationStatus::Moving
    );
    assert_eq!(harness.state().editor.timeline().unwrap().tick, 16);
    assert_ne!(stepped_view.agent.position, initial_position);
    let stepped_pixels = viewport_pixels(&mut harness);
    assert!(changed_pixel_count(&reopened_pixels, &stepped_pixels) > 10);
    if capture_directory.is_some() {
        capture(
            capture_directory.as_deref().unwrap(),
            "mesh-agent-16-ticks.png",
            &mut harness,
        );
    }
    click(&mut harness, LBL_STOP);
    assert_eq!(harness.state().editor.mode(), Mode::Edit);
    assert_eq!(harness.state().editor.checksum(), saved_checksum);

    fixture
        .restore_source()
        .expect("restore the exact package source before writing evidence");
    let scene_bytes = fs::read(&fixture.scene).unwrap();
    let sidecar_bytes = fs::read(&sidecar).unwrap();
    assert_eq!(scene_bytes, saved_scene_bytes);
    assert_eq!(sidecar_bytes, saved_sidecar_bytes);
    if let Some(directory) = capture_directory.as_deref() {
        let receipt = serde_json::json!({
            "scene_path": fixture.scene,
            "sidecar_path": sidecar,
            "package_source": fixture.source,
            "scene_sha256": sha256_hex(&scene_bytes),
            "sidecar_sha256": sha256_hex(&sidecar_bytes),
            "graph_revision": graph_revision,
            "editor_checksum": saved_checksum,
            "source_model_sha256": fixture.source_model_sha256,
            "source_license_sha256": fixture.source_license_sha256,
            "source_hidden_during_physics_open": source_hidden_during_physics_open,
            "source_restored": fixture.source_restored,
            "initial_viewport_changed_pixels_after_reopen": changed_pixel_count(&dirty_art_pixels, &reopened_pixels),
            "viewport_changed_pixels_after_16_steps": changed_pixel_count(&reopened_pixels, &stepped_pixels),
            "ticks": 16,
        });
        fs::write(
            directory.join("acceptance.json"),
            serde_json::to_vec_pretty(&receipt).unwrap(),
        )
        .unwrap();
    }
}
