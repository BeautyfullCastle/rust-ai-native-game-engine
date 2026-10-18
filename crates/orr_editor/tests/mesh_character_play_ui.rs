#![cfg(all(
    windows,
    target_arch = "x86_64",
    feature = "mesh-navigation",
    feature = "mesh-animated-models",
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

const ANIMATED_MODEL: &str = "animated.glb";
const LICENSE: &str = "LICENSE.txt";

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    captures: PathBuf,
    scene: PathBuf,
    physics_scene: PathBuf,
    package: PathBuf,
    hidden_package: PathBuf,
    runtime: PathBuf,
    export: PathBuf,
    model_sha256: String,
    license_sha256: String,
    package_sha256: String,
    package_files: Vec<String>,
    package_hidden: bool,
    package_restored: bool,
}

impl Fixture {
    fn new() -> Self {
        assert_eq!(
            std::env::var("ORR_MESH_CHARACTER_PLAY_ACCEPTANCE").as_deref(),
            Ok("1")
        );
        assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
        assert_eq!(std::env::var("WGPU_BACKEND").unwrap_or_default(), "vulkan");
        let temporary = tempfile::tempdir().unwrap();
        let root = PathBuf::from(
            std::env::var_os("ORR_MESH_CHARACTER_PLAY_CAPTURE_DIR")
                .expect("set a fresh character-play capture root"),
        );
        assert!(root.is_absolute());
        fs::create_dir(&root).expect("capture root must be fresh");
        let root = root.canonicalize().unwrap();
        let captures = root.join("captures");
        fs::create_dir(&captures).unwrap();
        let scene = PathBuf::from(
            std::env::var_os("ORR_MESH_CHARACTER_PLAY_OUTPUT")
                .unwrap_or_else(|| root.join("mesh-character-play.scene.yaml").into_os_string()),
        );
        assert!(scene.is_absolute() && !scene.exists());
        let scene_parent = scene.parent().unwrap().canonicalize().unwrap();
        let scene = scene_parent.join(scene.file_name().unwrap());
        let physics_scene = root.join("owned-stock-physics.scene.yaml");
        fs::copy(EditorGame::PhysGame.default_scene_path(), &physics_scene).unwrap();
        let package = root.join("animated model package");
        let hidden_package = root.join("animated model package.hidden");
        fs::create_dir(&package).unwrap();
        let package_fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/animation_demo");
        let package_files = copy_declared_package(&package_fixture, &package);
        let runtime = PathBuf::from(
            std::env::var_os("ORR_MESH_EXPORT_RUNTIME")
                .expect("character-play export requires a fresh matching player"),
        );
        assert!(runtime.is_absolute() && runtime.is_file());
        let export = PathBuf::from(
            std::env::var_os("ORR_MESH_CHARACTER_PLAY_EXPORT_OUTPUT")
                .expect("character-play acceptance requires an export directory"),
        );
        assert!(export.is_absolute() && !export.exists());
        assert_eq!(
            export.parent().unwrap().canonicalize().unwrap(),
            root.parent().unwrap()
        );
        let model_sha256 = sha256(&fs::read(package.join(ANIMATED_MODEL)).unwrap());
        let license_sha256 = sha256(&fs::read(package.join(LICENSE)).unwrap());
        let package_sha256 = package_inventory_sha256(&package, &package_files);
        Self {
            _temporary: temporary,
            root,
            captures,
            scene,
            physics_scene,
            package,
            hidden_package,
            runtime,
            export,
            model_sha256,
            license_sha256,
            package_sha256,
            package_files,
            package_hidden: false,
            package_restored: false,
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

    fn hide_package(&mut self) {
        assert!(!self.package_hidden && !self.hidden_package.exists());
        fs::rename(&self.package, &self.hidden_package).unwrap();
        self.package_hidden = true;
        assert!(!self.package.exists());
    }

    fn restore_package(&mut self) {
        assert!(self.package_hidden && !self.package.exists());
        fs::rename(&self.hidden_package, &self.package).unwrap();
        assert_eq!(
            package_inventory_sha256(&self.package, &self.package_files),
            self.package_sha256
        );
        self.package_hidden = false;
        self.package_restored = true;
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if self.package_hidden && !self.package.exists() && self.hidden_package.exists() {
            let _ = fs::rename(&self.hidden_package, &self.package);
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
    match Wgpu::headless(WgpuOptions::default()) {
        Ok(rhi) => {
            eprintln!(
                "Mesh character-play GPU preflight: {} (software: {})",
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
                std::env::var_os("ORR_MESH_CHARACTER_PLAY_ACCEPTANCE").is_none()
                    && std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "required character-play GPU acceptance unavailable: {error}"
            );
            eprintln!("SKIP: character-play GPU unavailable: {error}");
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
        let app = harness.state();
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {operation}: game={:?}, path={:?}, checksum={}, status={:?}",
            app.editor.game(),
            app.editor.path(),
            app.editor.checksum(),
            app.editor.status()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn mesh_view(app: &EditorApp) -> orr_remote::mesh_navigation::MeshView {
    mesh_from_view(app.editor.snapshot().unwrap().predicted()).unwrap()
}

fn viewport_pixels(harness: &mut Harness<'_, EditorApp>, filename: &str) -> Vec<u8> {
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
        let ratio = harness.ctx.pixels_per_point();
        harness.set_size(egui::vec2(
            window.x + (target.0 as f32 - actual.0 as f32) / ratio,
            window.y + (target.1 as f32 - actual.1 as f32) / ratio,
        ));
    }
    settle(harness);
    let viewport = harness.state().viewport3d_gpu().unwrap();
    assert_eq!(viewport.gpu().target().size(), target);
    eprintln!(
        "Mesh character-play App GPU: {}",
        viewport.gpu().adapter_name()
    );
    let pixels = viewport.gpu().read_rgba8();
    assert_eq!(pixels.len(), (target.0 * target.1 * 4) as usize);
    let background = &pixels[..3];
    let world_pixels = pixels
        .chunks_exact(4)
        .filter(|pixel| {
            pixel[..3]
                .iter()
                .zip(background)
                .any(|(channel, clear)| channel.abs_diff(*clear) > 12)
        })
        .count();
    assert!(
        world_pixels > 500,
        "follow camera must retain a visible game world: {world_pixels}"
    );
    let image =
        egui::ColorImage::from_rgba_unmultiplied([target.0 as usize, target.1 as usize], &pixels);
    orr_editor::app::write_png(
        &PathBuf::from(std::env::var_os("ORR_MESH_CHARACTER_PLAY_CAPTURE_DIR").unwrap())
            .join("captures")
            .join(filename),
        &image,
    )
    .unwrap();
    pixels
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

fn copy_declared_package(source: &Path, destination: &Path) -> Vec<String> {
    let manifest_path = source.join("orr.package.json");
    let manifest_bytes = fs::read(&manifest_path).unwrap();
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let mut names = manifest["files"]
        .as_array()
        .expect("package manifest declares files")
        .iter()
        .map(|value| {
            value
                .as_str()
                .expect("manifest file is a string")
                .to_owned()
        })
        .collect::<Vec<_>>();
    for name in &names {
        let path = Path::new(name);
        assert!(!path.is_absolute());
        assert!(path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_))));
    }
    names.push("orr.package.json".into());
    names.sort();
    for name in &names {
        let from = source.join(name);
        let to = destination.join(name);
        assert!(from.is_file());
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::copy(from, to).unwrap();
    }
    names
}

fn package_inventory_sha256(root: &Path, package_files: &[String]) -> String {
    let mut names = package_files.to_vec();
    names.sort();
    let mut inventory = Vec::new();
    for name in &names {
        let bytes = fs::read(root.join(name)).unwrap();
        inventory.extend_from_slice(name.as_bytes());
        inventory.push(0);
        inventory.extend_from_slice(bytes.len().to_string().as_bytes());
        inventory.push(0);
        inventory.extend_from_slice(sha256(&bytes).as_bytes());
        inventory.push(b'\n');
    }
    sha256(&inventory)
}

fn sidecar_path(scene: &Path, suffix: &str) -> PathBuf {
    let mut name = scene.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

fn camera_target(harness: &Harness<'_, EditorApp>) -> [f32; 3] {
    harness
        .state()
        .editor
        .presentation_camera3d((900, 850))
        .expect("admitted follow camera")
        .target
}

fn assert_follow_target(harness: &Harness<'_, EditorApp>, offset: [f32; 3]) -> [f32; 3] {
    let view = mesh_view(harness.state());
    let target = camera_target(harness);
    for axis in 0..3 {
        let expected = view.agent.position[axis].to_f32() + offset[axis];
        assert!(
            (target[axis] - expected).abs() < 0.001,
            "camera target axis {axis} was {}, expected {expected} from the live agent",
            target[axis]
        );
    }
    let camera = harness
        .state()
        .editor
        .presentation_camera3d((900, 850))
        .unwrap();
    let agent_pixel = camera
        .world_to_screen(view.agent.position.map(orr_fp::FP::to_f32), (900, 850))
        .expect("followed agent is in front of the game camera");
    assert!(
        (10.0..890.0).contains(&agent_pixel[0]) && (10.0..840.0).contains(&agent_pixel[1]),
        "followed agent must remain inside the game viewport: {agent_pixel:?}"
    );
    target
}

fn wait_for_arrival(harness: &mut Harness<'_, EditorApp>) {
    wait_for(harness, "mesh agent arrival and idle clip", |app| {
        mesh_view(app).agent.status == NavigationStatus::Arrived
    });
}

fn capture_record(root: &Path, name: &str) -> Value {
    let path = root.join(name);
    let bytes = fs::read(path).unwrap();
    serde_json::json!({"name":name,"bytes":bytes.len(),"sha256":sha256(&bytes)})
}

#[test]
#[ignore = "requires ORR_MESH_CHARACTER_PLAY_ACCEPTANCE=1 and a hardware Vulkan adapter"]
fn mesh_character_play_follows_agent_and_switches_clips_in_source_free_export() {
    let requested = std::env::var_os("ORR_MESH_CHARACTER_PLAY_ACCEPTANCE").is_some()
        || std::env::var_os("ORR_REQUIRE_GPU").is_some();
    if !requested {
        eprintln!("SKIP: opt in to actual Mesh character-play GPU acceptance");
        return;
    }
    assert_eq!(
        std::env::var("ORR_MESH_CHARACTER_PLAY_ACCEPTANCE").as_deref(),
        Ok("1")
    );
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    assert_eq!(std::env::var("WGPU_BACKEND").unwrap_or_default(), "vulkan");
    assert!(gpu_available());
    let mut fixture = Fixture::new();
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
    let animation_path = sidecar_path(&fixture.scene, ".mesh-animation.json");
    let camera_path = sidecar_path(&fixture.scene, ".mesh-camera.json");
    assert!(!animation_path.exists() && !camera_path.exists());

    if harness
        .query_by_label("Animation package source directory")
        .is_none()
    {
        click(&mut harness, "Mesh agent animation");
    }
    type_field(
        &mut harness,
        "Animation package source directory",
        &fixture.package.display().to_string(),
    );
    type_field(&mut harness, "Animation model asset", ANIMATED_MODEL);
    type_field(&mut harness, "Animation license asset", LICENSE);
    assert!(harness
        .get_by_role_and_label(Role::Button, "Load mesh animation")
        .accesskit_node()
        .is_disabled());
    click(&mut harness, "I have reviewed rights to use this animation");
    click_button(&mut harness, "Load mesh animation");
    assert!(harness
        .query_by_label_contains("Loaded animation package sample-animation/animated.glb")
        .is_some());
    type_field(&mut harness, "Animation clip index", "1");
    click(&mut harness, "Use idle and moving clips");
    type_field(&mut harness, "Animation idle clip index", "0");
    click(&mut harness, "Animation face route direction");
    click_button(&mut harness, "Apply mesh animation");
    click_button(&mut harness, "Save mesh animation");
    let animation_bytes = fs::read(&animation_path).expect("explicit animation save");
    let animation_document: Value = serde_json::from_slice(&animation_bytes).unwrap();
    assert_eq!(animation_document["settings"]["clip"], 1);
    assert_eq!(animation_document["settings"]["idle_clip"], 0);
    assert_eq!(animation_document["settings"]["facing"], "route");
    assert_eq!(
        animation_document["graph_revision"],
        serde_json::json!(graph_revision)
    );

    if harness.query_by_label("Game camera target X").is_none() {
        click(&mut harness, "Mesh game camera");
    }
    click(&mut harness, "Follow mesh agent");
    let follow_offset = [0.5, 0.25, 0.0];
    for (axis, label) in [
        "Camera follow offset X",
        "Camera follow offset Y",
        "Camera follow offset Z",
    ]
    .into_iter()
    .enumerate()
    {
        type_field(&mut harness, label, &follow_offset[axis].to_string());
    }
    click_button(&mut harness, "Apply game camera");
    let fixed_camera_target = assert_follow_target(&harness, follow_offset);
    click_button(&mut harness, "Undo game camera");
    assert!(camera_target(&harness)
        .iter()
        .zip(fixed_camera_target)
        .any(|(actual, expected)| (*actual - expected).abs() > 0.01));
    click_button(&mut harness, "Redo game camera");
    assert_eq!(
        assert_follow_target(&harness, follow_offset),
        fixed_camera_target
    );
    click_button(&mut harness, "Save game camera");
    let camera_bytes = fs::read(&camera_path).expect("explicit camera save");
    let camera_document: Value = serde_json::from_slice(&camera_bytes).unwrap();
    assert_eq!(
        camera_document["camera"]["follow_offset"],
        serde_json::json!(follow_offset)
    );
    assert_eq!(
        camera_document["graph_revision"],
        serde_json::json!(graph_revision)
    );
    assert_eq!(harness.state().editor.checksum(), scene_checksum);

    let edit_pixels = viewport_pixels(&mut harness, "edit-follow-camera.png");
    menu_click(&mut harness, "File", "Save Ctrl+S");
    fixture.hide_package();
    drop(harness);
    let mut reopened = fixture.harness(&fixture.physics_scene, EditorGame::PhysGame);
    reopened.run_steps(4);
    menu_click(&mut reopened, "File", "Open mesh scene…");
    type_scene_path(&mut reopened, &fixture.scene, "OK");
    wait_for(&mut reopened, "source-free character scene reopen", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && app.editor.checksum() == scene_checksum
            && app.editor.path().as_deref().is_some_and(|path| {
                path == fixture.scene.as_path()
                    || path.canonicalize().ok() == fixture.scene.canonicalize().ok()
            })
    });
    assert!(!fixture.package.exists());
    assert_eq!(fs::read(&fixture.scene).unwrap(), scene_bytes);
    assert_eq!(fs::read(&animation_path).unwrap(), animation_bytes);
    assert_eq!(fs::read(&camera_path).unwrap(), camera_bytes);
    if reopened
        .query_by_label("Animation package source directory")
        .is_none()
    {
        click(&mut reopened, "Mesh agent animation");
    }
    if reopened.query_by_label("Game camera target X").is_none() {
        click(&mut reopened, "Mesh game camera");
    }
    assert_eq!(
        mesh_view(reopened.state()).agent.status,
        NavigationStatus::Moving
    );
    assert!(reopened
        .query_by_label("Mesh animation state: moving (clip 1)")
        .is_some());
    let reopened_target = assert_follow_target(&reopened, follow_offset);
    assert_eq!(reopened_target, fixed_camera_target);
    let reopened_pixels = viewport_pixels(&mut reopened, "fresh-source-free-reopen.png");
    assert!(changed_pixel_count(&edit_pixels, &reopened_pixels) < 20);

    click_button(&mut reopened, LBL_PLAY);
    wait_for(&mut reopened, "playing character scene", |app| {
        app.editor
            .timeline()
            .is_some_and(|timeline| timeline.playing)
    });
    wait_for(&mut reopened, "moving clip selection", |app| {
        app.editor.status().is_some() && mesh_view(app).agent.status == NavigationStatus::Moving
    });
    assert!(reopened
        .query_by_label("Mesh animation state: moving (clip 1)")
        .is_some());
    let position_before_step = mesh_view(reopened.state()).agent.position;
    click_button(&mut reopened, LBL_PAUSE);
    click_button(&mut reopened, LBL_STEP);
    let moving_view = mesh_view(reopened.state());
    assert_ne!(moving_view.agent.position, position_before_step);
    let moving_camera_target = assert_follow_target(&reopened, follow_offset);
    assert_ne!(moving_camera_target, reopened_target);
    let moving_pixels = viewport_pixels(&mut reopened, "play-moving-follow-camera.png");
    assert!(changed_pixel_count(&reopened_pixels, &moving_pixels) > 10);
    click_button(&mut reopened, LBL_PLAY);
    wait_for_arrival(&mut reopened);
    assert!(reopened
        .query_by_label("Mesh animation state: idle (clip 0)")
        .is_some());
    let arrived_view = mesh_view(reopened.state());
    let arrived_camera_target = assert_follow_target(&reopened, follow_offset);
    assert_ne!(arrived_camera_target, moving_camera_target);
    let arrived_pixels = viewport_pixels(&mut reopened, "play-arrived-idle-follow-camera.png");
    assert!(changed_pixel_count(&moving_pixels, &arrived_pixels) > 10);
    let play_tick = reopened.state().editor.timeline().unwrap().tick;
    let arrived_position = arrived_view.agent.position;

    click_button(&mut reopened, LBL_STOP);
    assert_eq!(reopened.state().editor.mode(), Mode::Edit);
    assert_eq!(reopened.state().editor.checksum(), scene_checksum);
    assert_eq!(fs::read(&fixture.scene).unwrap(), scene_bytes);
    assert_eq!(fs::read(&animation_path).unwrap(), animation_bytes);
    assert_eq!(fs::read(&camera_path).unwrap(), camera_bytes);

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
    let animation_name = format!("{scene_name}.mesh-animation.json");
    let camera_name = format!("{scene_name}.mesh-camera.json");
    assert_eq!(
        fs::read(fixture.export.join(&scene_name)).unwrap(),
        scene_bytes
    );
    assert_eq!(
        fs::read(fixture.export.join(&animation_name)).unwrap(),
        animation_bytes
    );
    assert_eq!(
        fs::read(fixture.export.join(&camera_name)).unwrap(),
        camera_bytes
    );
    let manifest_path = fixture.export.join("orr.mesh-bundle.json");
    let manifest_bytes = fs::read(&manifest_path).expect("exported bundle manifest");
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let runtime_bytes = fs::read(fixture.export.join("orr_mesh_player.exe")).unwrap();
    assert_eq!(manifest["schema"], 1);
    assert_eq!(manifest["scene"]["name"], scene_name);
    assert_eq!(manifest["scene"]["bytes"], scene_bytes.len() as u64);
    assert_eq!(manifest["scene"]["sha256"], sha256(&scene_bytes));
    assert_eq!(manifest["animation"]["name"], animation_name);
    assert_eq!(manifest["animation"]["bytes"], animation_bytes.len() as u64);
    assert_eq!(manifest["animation"]["sha256"], sha256(&animation_bytes));
    assert_eq!(manifest["camera"]["name"], camera_name);
    assert_eq!(manifest["camera"]["bytes"], camera_bytes.len() as u64);
    assert_eq!(manifest["camera"]["sha256"], sha256(&camera_bytes));
    assert_eq!(manifest["runtime"]["name"], "orr_mesh_player.exe");
    assert_eq!(manifest["runtime"]["bytes"], runtime_bytes.len() as u64);
    assert_eq!(manifest["runtime"]["sha256"], sha256(&runtime_bytes));
    assert_eq!(
        manifest["graph_revision"],
        serde_json::json!(graph_revision)
    );
    assert_eq!(manifest["checksum"], scene_checksum);
    assert_eq!(manifest["input_profile"], "mesh-click-to-move-v1");
    let mut export_names = fs::read_dir(&fixture.export)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    export_names.sort();
    let mut expected_names = vec![
        "orr.mesh-bundle.json".to_owned(),
        "orr_mesh_player.exe".to_owned(),
        scene_name,
        animation_name,
        camera_name,
    ];
    expected_names.sort();
    assert_eq!(export_names, expected_names);
    drop(reopened);
    fixture.restore_package();

    let capture_names = [
        "edit-follow-camera.png",
        "fresh-source-free-reopen.png",
        "play-moving-follow-camera.png",
        "play-arrived-idle-follow-camera.png",
    ];
    let captures = capture_names
        .iter()
        .map(|name| capture_record(&fixture.captures, name))
        .collect::<Vec<_>>();
    let receipt = serde_json::json!({
        "artifact_root":fixture.root,
        "scene_path":fixture.scene,
        "scene_sha256":sha256(&scene_bytes),
        "scene_checksum":format!("0x{scene_checksum:016x}"),
        "graph_revision":graph_revision.iter().map(|byte|format!("{byte:02x}")).collect::<String>(),
        "animation_sidecar_path":animation_path,
        "animation_sidecar_sha256":sha256(&animation_bytes),
        "camera_sidecar_path":camera_path,
        "camera_sidecar_sha256":sha256(&camera_bytes),
        "source_package":fixture.package,
        "source_package_sha256":fixture.package_sha256,
        "source_package_files":fixture.package_files,
        "animation_model_sha256":fixture.model_sha256,
        "license_sha256":fixture.license_sha256,
        "source_hidden_during_reopen":true,
        "source_restored_and_hash_checked":fixture.package_restored,
        "moving_clip":animation_document["settings"]["clip"],
        "idle_clip":animation_document["settings"]["idle_clip"],
        "animation_facing":animation_document["settings"]["facing"],
        "follow_offset":follow_offset,
        "camera_follow_target_at_reopen":reopened_target,
        "camera_follow_target_moving":moving_camera_target,
        "camera_follow_target_arrived":arrived_camera_target,
        "moving_state_observed":true,
        "idle_state_observed":true,
        "agent_arrived":arrived_view.agent.status == NavigationStatus::Arrived,
        "play_tick_at_arrival":play_tick,
        "agent_position_at_arrival":arrived_position.map(|value|value.to_f32()),
        "play_and_camera_follow_verified":moving_camera_target != reopened_target && arrived_camera_target != moving_camera_target,
        "fresh_reopen_pixels_match":changed_pixel_count(&edit_pixels,&reopened_pixels) < 20,
        "play_moving_pixel_diff":changed_pixel_count(&reopened_pixels,&moving_pixels),
        "play_idle_pixel_diff":changed_pixel_count(&moving_pixels,&arrived_pixels),
        "scene_checksum_unchanged":true,
        "export_directory":fixture.export,
        "export_files":export_names,
        "export_manifest_sha256":sha256(&manifest_bytes),
        "export_runtime_sha256":sha256(&runtime_bytes),
        "physical_os_input":false,
        "captures":captures
    });
    fs::write(
        fixture.root.join("acceptance.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
}
