#![cfg(all(
    windows,
    target_arch = "x86_64",
    feature = "mesh-navigation",
    feature = "mesh-animated-models"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::{kittest::Queryable, Harness};
use egui_wgpu::{wgpu, WgpuSetupCreateNew};
use orr_editor::{
    app::{LBL_PAUSE, LBL_PLAY, LBL_STEP, LBL_STOP},
    game::EditorGame,
    Editor, EditorApp, Mode,
};
use orr_navigation::NavigationStatus;
use orr_remote::mesh_navigation::mesh_from_view;
use orr_rhi::Rhi;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

const STATIC_MODEL: &str = "foreground.glb";
const ANIMATED_MODEL: &str = "animated.glb";
const LICENSE: &str = "LICENSE.txt";

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    scene: PathBuf,
    physics_scene: PathBuf,
    static_source: PathBuf,
    hidden_static: PathBuf,
    animation_source: PathBuf,
    hidden_animation: PathBuf,
    runtime: Option<PathBuf>,
    export_output: Option<PathBuf>,
    captures: PathBuf,
    static_model_sha256: String,
    static_license_sha256: String,
    static_source_package_sha256: String,
    static_package_files: Vec<String>,
    animation_model_sha256: String,
    animation_license_sha256: String,
    animation_source_package_sha256: String,
    animation_package_files: Vec<String>,
    sources_hidden: bool,
    sources_restored: bool,
}

impl Fixture {
    fn new() -> Self {
        assert_eq!(
            std::env::var("ORR_MESH_ROUTE_FACING_ACCEPTANCE").as_deref(),
            Ok("1")
        );
        assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
        assert_eq!(std::env::var("WGPU_BACKEND").unwrap_or_default(), "vulkan");
        let temporary = tempfile::tempdir().unwrap();
        let root = PathBuf::from(
            std::env::var_os("ORR_MESH_ROUTE_FACING_CAPTURE_DIR")
                .expect("set a fresh route-facing capture directory"),
        );
        assert!(root.is_absolute());
        fs::create_dir(&root).expect("capture directory must be fresh");
        let root = root.canonicalize().unwrap();
        let captures = root.join("captures");
        fs::create_dir(&captures).unwrap();
        let scene = PathBuf::from(
            std::env::var_os("ORR_MESH_ROUTE_FACING_OUTPUT")
                .unwrap_or_else(|| root.join("mesh-route-facing.scene.yaml").into_os_string()),
        );
        assert!(scene.is_absolute() && !scene.exists());
        let scene_parent = scene.parent().unwrap().canonicalize().unwrap();
        let scene = scene_parent.join(scene.file_name().unwrap());
        let physics_scene = root.join("owned-stock-physics.scene.yaml");
        fs::copy(EditorGame::PhysGame.default_scene_path(), &physics_scene).unwrap();
        let static_source = root.join("static model package");
        let hidden_static = root.join("static model package.hidden");
        let animation_source = root.join("animated model package");
        let hidden_animation = root.join("animated model package.hidden");
        fs::create_dir(&static_source).unwrap();
        fs::create_dir(&animation_source).unwrap();
        let static_assets =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/imported_scene_demo");
        let static_package_files = copy_package(
            &static_assets,
            &static_source,
            &["background.glb", "foreground.glb", "generate.py", LICENSE],
        );
        let animation_assets =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/animation_demo");
        let animation_package_files = copy_package(
            &animation_assets,
            &animation_source,
            &[ANIMATED_MODEL, LICENSE],
        );
        let runtime = std::env::var_os("ORR_MESH_EXPORT_RUNTIME").map(PathBuf::from);
        if let Some(path) = &runtime {
            assert!(path.is_absolute() && path.is_file());
        }
        let export_output =
            std::env::var_os("ORR_MESH_ROUTE_FACING_EXPORT_OUTPUT").map(PathBuf::from);
        if let Some(path) = &export_output {
            assert!(path.is_absolute() && !path.exists());
            assert_eq!(
                path.parent().unwrap().canonicalize().unwrap(),
                root.parent().unwrap()
            );
            assert!(
                runtime.is_some(),
                "export output requires the archived player"
            );
        }
        let static_source_package_sha256 =
            package_inventory_sha256(&static_source, &static_package_files);
        let animation_source_package_sha256 =
            package_inventory_sha256(&animation_source, &animation_package_files);
        Self {
            _temporary: temporary,
            root: root.clone(),
            scene,
            physics_scene,
            static_model_sha256: sha256(&fs::read(static_source.join(STATIC_MODEL)).unwrap()),
            static_license_sha256: sha256(&fs::read(static_source.join(LICENSE)).unwrap()),
            static_source_package_sha256,
            static_package_files,
            animation_model_sha256: sha256(
                &fs::read(animation_source.join(ANIMATED_MODEL)).unwrap(),
            ),
            animation_license_sha256: sha256(&fs::read(animation_source.join(LICENSE)).unwrap()),
            animation_source_package_sha256,
            animation_package_files,
            static_source,
            hidden_static,
            animation_source,
            hidden_animation,
            runtime,
            export_output,
            captures,
            sources_hidden: false,
            sources_restored: false,
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

    fn hide_sources(&mut self) {
        fs::rename(&self.static_source, &self.hidden_static).unwrap();
        self.sources_hidden = true;
        fs::rename(&self.animation_source, &self.hidden_animation).unwrap();
        assert!(!self.static_source.exists() && !self.animation_source.exists());
    }

    fn restore_sources(&mut self) {
        assert!(self.sources_hidden);
        fs::rename(&self.hidden_static, &self.static_source).unwrap();
        fs::rename(&self.hidden_animation, &self.animation_source).unwrap();
        assert_eq!(
            sha256(&fs::read(self.static_source.join(STATIC_MODEL)).unwrap()),
            self.static_model_sha256
        );
        assert_eq!(
            sha256(&fs::read(self.static_source.join(LICENSE)).unwrap()),
            self.static_license_sha256
        );
        assert_eq!(
            sha256(&fs::read(self.animation_source.join(ANIMATED_MODEL)).unwrap()),
            self.animation_model_sha256
        );
        assert_eq!(
            sha256(&fs::read(self.animation_source.join(LICENSE)).unwrap()),
            self.animation_license_sha256
        );
        assert_eq!(
            package_inventory_sha256(&self.static_source, &self.static_package_files,),
            self.static_source_package_sha256
        );
        assert_eq!(
            package_inventory_sha256(&self.animation_source, &self.animation_package_files,),
            self.animation_source_package_sha256
        );
        self.sources_hidden = false;
        self.sources_restored = true;
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if self.sources_hidden {
            if !self.static_source.exists() && self.hidden_static.exists() {
                let _ = fs::rename(&self.hidden_static, &self.static_source);
            }
            if !self.animation_source.exists() && self.hidden_animation.exists() {
                let _ = fs::rename(&self.hidden_animation, &self.animation_source);
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

fn scene_path(harness: &mut Harness<'_, EditorApp>, scene: &Path, action: &str) {
    type_field(harness, "Scene path", &scene.display().to_string());
    click_button(harness, action);
}

fn wait_for(harness: &mut Harness<'_, EditorApp>, label: &str, ready: impl Fn(&EditorApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        settle(harness);
        if ready(harness.state()) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {label}");
}

fn mesh_view(harness: &Harness<'_, EditorApp>) -> orr_remote::mesh_navigation::MeshView {
    mesh_from_view(harness.state().editor.snapshot().unwrap().predicted()).unwrap()
}

fn viewport_pixels(harness: &mut Harness<'_, EditorApp>, filename: &str) -> Vec<u8> {
    let target = (900, 850);
    for _ in 0..6 {
        settle(harness);
        let size = harness
            .state()
            .viewport3d_gpu()
            .unwrap()
            .gpu()
            .target()
            .size();
        if size == target {
            break;
        }
        let window = harness.ctx.viewport_rect().size();
        let ratio = harness.ctx.pixels_per_point();
        harness.set_size(egui::vec2(
            window.x + (target.0 as f32 - size.0 as f32) / ratio,
            window.y + (target.1 as f32 - size.1 as f32) / ratio,
        ));
    }
    settle(harness);
    let viewport = harness.state().viewport3d_gpu().unwrap();
    assert_eq!(viewport.gpu().target().size(), target);
    eprintln!(
        "Mesh route-facing App GPU: {}",
        viewport.gpu().adapter_name()
    );
    let pixels = viewport.gpu().read_rgba8();
    assert_eq!(pixels.len(), (target.0 * target.1 * 4) as usize);
    let png =
        egui::ColorImage::from_rgba_unmultiplied([target.0 as usize, target.1 as usize], &pixels);
    orr_editor::app::write_png(
        &PathBuf::from(std::env::var_os("ORR_MESH_ROUTE_FACING_CAPTURE_DIR").unwrap())
            .join("captures")
            .join(filename),
        &png,
    )
    .unwrap();
    pixels
}

fn changed_pixels(left: &[u8], right: &[u8]) -> usize {
    assert_eq!(left.len(), right.len());
    left.as_chunks::<4>().0.iter()
        .zip(right.as_chunks::<4>().0.iter())
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

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn copy_package(source: &Path, destination: &Path, expected_assets: &[&str]) -> Vec<String> {
    let manifest_path = source.join("orr.package.json");
    let manifest_bytes = fs::read(&manifest_path).unwrap();
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let mut declared = manifest["files"]
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
    declared.sort();
    let mut expected = expected_assets
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    expected.sort();
    assert_eq!(
        declared, expected,
        "shipped package manifest inventory changed"
    );
    let mut filenames = vec!["orr.package.json".to_owned()];
    filenames.extend(declared);
    for filename in &filenames {
        assert!(Path::new(filename)
            .file_name()
            .is_some_and(|leaf| leaf.to_string_lossy() == filename.as_str()));
        let source_path = source.join(filename);
        assert!(source_path.is_file());
        fs::copy(source_path, destination.join(filename)).unwrap();
    }
    filenames.sort();
    filenames
}

fn package_inventory_sha256(root: &Path, filenames: &[String]) -> String {
    let mut names = filenames.to_vec();
    names.sort();
    let mut inventory = Vec::new();
    for name in &names {
        let bytes = fs::read(root.join(name)).unwrap();
        let digest = sha256(&bytes);
        inventory.extend_from_slice(name.as_bytes());
        inventory.push(0);
        inventory.extend_from_slice(bytes.len().to_string().as_bytes());
        inventory.push(0);
        inventory.extend_from_slice(digest.as_bytes());
        inventory.push(b'\n');
    }
    sha256(&inventory)
}

fn sidecar_path(scene: &Path, suffix: &str) -> PathBuf {
    let mut name = scene.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

fn capture_record(root: &Path, name: &str) -> Value {
    let path = root.join(name);
    let bytes = fs::read(path).unwrap();
    serde_json::json!({"name":name,"bytes":bytes.len(),"sha256":sha256(&bytes)})
}

fn export_bundle(
    harness: &mut Harness<'_, EditorApp>,
    fixture: &Fixture,
    art: &[u8],
    animation: &[u8],
) -> Value {
    let Some(output) = fixture.export_output.as_ref() else {
        return Value::Null;
    };
    let runtime = fixture
        .runtime
        .as_ref()
        .expect("export requires archived runtime");
    menu_click(harness, "File", "Export mesh game…");
    type_field(
        harness,
        "Mesh player executable",
        &runtime.display().to_string(),
    );
    type_field(
        harness,
        "Mesh game output directory",
        &output.display().to_string(),
    );
    click(harness, "I trust this local mesh player executable");
    click_button(harness, "Export mesh game");
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        settle(harness);
        if harness
            .query_by_label_contains("Mesh export complete:")
            .is_some()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        harness
            .query_by_label_contains("Mesh export complete:")
            .is_some(),
        "same-App Mesh export did not complete"
    );
    let names = [
        fixture
            .scene
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        format!(
            "{}.mesh-art.json",
            fixture.scene.file_name().unwrap().to_string_lossy()
        ),
        format!(
            "{}.mesh-animation.json",
            fixture.scene.file_name().unwrap().to_string_lossy()
        ),
        "orr.mesh-bundle.json".into(),
        "orr_mesh_player.exe".into(),
    ];
    assert_eq!(fs::read(output.join(&names[1])).unwrap(), art);
    assert_eq!(fs::read(output.join(&names[2])).unwrap(), animation);
    assert_eq!(
        fs::read(output.join(&names[0])).unwrap(),
        fs::read(&fixture.scene).unwrap()
    );
    assert_eq!(
        fs::read(output.join("orr_mesh_player.exe")).unwrap(),
        fs::read(runtime).unwrap()
    );
    let mut actual = fs::read_dir(output)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    actual.sort();
    let mut expected = names.to_vec();
    expected.sort();
    assert_eq!(actual, expected);
    serde_json::json!({"directory":output,"files":actual,"art_sidecar_sha256":sha256(art),"animation_sidecar_sha256":sha256(animation)})
}

#[test]
#[ignore = "requires ORR_MESH_ROUTE_FACING_ACCEPTANCE=1 and a hardware Vulkan adapter"]
fn route_facing_is_saved_reopened_and_rendered_for_static_and_animated_art() {
    assert_eq!(
        std::env::var("ORR_MESH_ROUTE_FACING_ACCEPTANCE").as_deref(),
        Ok("1")
    );
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    assert_eq!(std::env::var("WGPU_BACKEND").unwrap_or_default(), "vulkan");
    let preflight = orr_rhi::Wgpu::headless(orr_rhi::WgpuOptions::default())
        .expect("hardware Vulkan preflight");
    assert!(
        !preflight.is_software(),
        "software rendering is not GPU acceptance"
    );
    eprintln!(
        "Mesh route-facing GPU adapter: {}",
        preflight.adapter_name()
    );
    drop(preflight);
    let mut fixture = Fixture::new();
    let mut harness = fixture.harness(&fixture.physics_scene, EditorGame::PhysGame);
    harness.run_steps(4);
    assert!(harness.state().has_gpu());
    menu_click(&mut harness, "File", "New mesh scene…");
    scene_path(&mut harness, &fixture.scene, "Create");
    click(&mut harness, "Static-mesh navigation");
    click_button(&mut harness, "Create ramp and bridge");
    click_button(&mut harness, "Place mesh agent");
    menu_click(&mut harness, "File", "Save Ctrl+S");
    let scene_bytes = fs::read(&fixture.scene).unwrap();
    let scene_checksum = harness.state().editor.checksum();
    let graph_revision = mesh_view(&harness).graph.revision();
    let baseline_pixels = viewport_pixels(&mut harness, "01-baseline.png");
    let art_path = sidecar_path(&fixture.scene, ".mesh-art.json");
    let animation_path = sidecar_path(&fixture.scene, ".mesh-animation.json");
    assert!(!art_path.exists() && !animation_path.exists());

    if harness
        .query_by_label("Art package source directory")
        .is_none()
    {
        click(&mut harness, "Mesh agent art");
    }
    for (label, value) in [
        (
            "Art package source directory",
            fixture.static_source.display().to_string(),
        ),
        ("Art model asset", STATIC_MODEL.into()),
        ("Art license asset", LICENSE.into()),
    ] {
        type_field(&mut harness, label, &value);
    }
    click(&mut harness, "I have reviewed rights to use this agent art");
    click_button(&mut harness, "Load agent art");
    assert!(harness
        .query_by_label_contains("Loaded static package art sample-imported-scene/foreground.glb")
        .is_some());
    click_button(&mut harness, "Apply agent art");
    let fixed_art_pixels = viewport_pixels(&mut harness, "02-static-fixed.png");
    assert!(changed_pixels(&baseline_pixels, &fixed_art_pixels) > 20);
    click(&mut harness, "Face route direction");
    click_button(&mut harness, "Apply agent art");
    let route_art_pixels = viewport_pixels(&mut harness, "03-static-route.png");
    let static_facing_pixel_diff = changed_pixels(&fixed_art_pixels, &route_art_pixels);
    assert!(
        static_facing_pixel_diff > 20,
        "route-facing static art should render a different orientation at the unchanged agent pose"
    );
    click_button(&mut harness, "Undo agent art");
    let static_undo_pixels = viewport_pixels(&mut harness, "04-static-undo.png");
    assert!(changed_pixels(&fixed_art_pixels, &static_undo_pixels) < 20);
    click_button(&mut harness, "Redo agent art");
    let static_redo_pixels = viewport_pixels(&mut harness, "05-static-redo.png");
    assert!(changed_pixels(&route_art_pixels, &static_redo_pixels) < 20);
    click_button(&mut harness, "Save agent art");
    let art_bytes = fs::read(&art_path).expect("explicit static-art save");
    let art_document: Value = serde_json::from_slice(&art_bytes).unwrap();
    assert_eq!(art_document["facing"], "route");
    assert_eq!(
        art_document["graph_revision"],
        serde_json::json!(graph_revision)
    );
    assert_eq!(harness.state().editor.checksum(), scene_checksum);

    if harness
        .query_by_label("Animation package source directory")
        .is_none()
    {
        click(&mut harness, "Mesh agent animation");
    }
    for (label, value) in [
        (
            "Animation package source directory",
            fixture.animation_source.display().to_string(),
        ),
        ("Animation model asset", ANIMATED_MODEL.into()),
        ("Animation license asset", LICENSE.into()),
    ] {
        type_field(&mut harness, label, &value);
    }
    click(&mut harness, "I have reviewed rights to use this animation");
    click_button(&mut harness, "Load mesh animation");
    assert!(harness
        .query_by_label_contains("Loaded animation package sample-animation/animated.glb")
        .is_some());
    for (label, value) in [
        ("Animation offset X", "0.25"),
        ("Animation offset Y", "0.5"),
        ("Animation offset Z", "0.25"),
        ("Animation scale", "0.5"),
        ("Animation rotation X (degrees)", "15"),
        ("Animation clip index", "1"),
        ("Animation playback rate", "1.5"),
    ] {
        type_field(&mut harness, label, value);
    }
    click_button(&mut harness, "Apply mesh animation");
    let fixed_animation_pixels = viewport_pixels(&mut harness, "06-animation-fixed.png");
    click(&mut harness, "Animation face route direction");
    click_button(&mut harness, "Apply mesh animation");
    let route_animation_pixels = viewport_pixels(&mut harness, "07-animation-route.png");
    let animation_facing_pixel_diff =
        changed_pixels(&fixed_animation_pixels, &route_animation_pixels);
    assert!(animation_facing_pixel_diff > 20, "route-facing animated art should render a different orientation at the unchanged agent pose");
    click_button(&mut harness, "Undo mesh animation");
    let animation_undo_pixels = viewport_pixels(&mut harness, "08-animation-undo.png");
    assert!(changed_pixels(&fixed_animation_pixels, &animation_undo_pixels) < 20);
    click_button(&mut harness, "Redo mesh animation");
    let animation_redo_pixels = viewport_pixels(&mut harness, "09-animation-redo.png");
    assert!(changed_pixels(&route_animation_pixels, &animation_redo_pixels) < 20);
    click_button(&mut harness, "Save mesh animation");
    let animation_bytes = fs::read(&animation_path).expect("explicit animation save");
    let animation_document: Value = serde_json::from_slice(&animation_bytes).unwrap();
    assert_eq!(animation_document["settings"]["facing"], "route");
    assert_eq!(
        animation_document["graph_revision"],
        serde_json::json!(graph_revision)
    );
    assert_eq!(fs::read(&fixture.scene).unwrap(), scene_bytes);
    assert_eq!(harness.state().editor.checksum(), scene_checksum);

    let view = mesh_view(&harness);
    assert_eq!(view.graph.revision(), graph_revision);
    assert_eq!(view.agent.status, NavigationStatus::Moving);
    fixture.hide_sources();
    drop(harness);
    let mut reopened = fixture.harness(&fixture.physics_scene, EditorGame::PhysGame);
    reopened.run_steps(4);
    menu_click(&mut reopened, "File", "Open mesh scene…");
    scene_path(&mut reopened, &fixture.scene, "OK");
    wait_for(&mut reopened, "source-free facing scene reopen", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && app.editor.checksum() == scene_checksum
            && app.editor.path().as_deref().is_some_and(|path| {
                path == fixture.scene.as_path()
                    || path.canonicalize().ok() == fixture.scene.canonicalize().ok()
            })
    });
    assert!(!fixture.static_source.exists() && !fixture.animation_source.exists());
    assert_eq!(fs::read(&art_path).unwrap(), art_bytes);
    assert_eq!(fs::read(&animation_path).unwrap(), animation_bytes);
    let reopened_pixels = viewport_pixels(&mut reopened, "10-reopened-source-hidden.png");
    let fresh_reopen_route_pixels_match =
        changed_pixels(&route_animation_pixels, &reopened_pixels) < 20;
    assert!(fresh_reopen_route_pixels_match);
    let sources_hidden_during_reopen =
        !fixture.static_source.exists() && !fixture.animation_source.exists();
    assert!(sources_hidden_during_reopen);
    let reopen_pixel_diff = changed_pixels(&route_art_pixels, &reopened_pixels);
    assert!(
        reopen_pixel_diff > 20,
        "reopened route-facing animated art replaces static-facing art"
    );

    click_button(&mut reopened, LBL_PLAY);
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline
        && !reopened
            .state()
            .editor
            .timeline()
            .is_some_and(|timeline| timeline.playing)
    {
        reopened.state_mut().editor.sync();
        reopened.run_steps(1);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(reopened
        .state()
        .editor
        .timeline()
        .is_some_and(|timeline| timeline.playing));
    click_button(&mut reopened, LBL_PAUSE);
    assert!(!reopened.state().editor.timeline().unwrap().playing);
    reopened.state_mut().editor.seek(0);
    settle(&mut reopened);
    let start_position = mesh_view(&reopened).agent.position;
    for _ in 0..16 {
        click_button(&mut reopened, LBL_STEP);
    }
    let stepped = mesh_view(&reopened);
    assert_eq!(reopened.state().editor.timeline().unwrap().tick, 16);
    assert_eq!(stepped.agent.status, NavigationStatus::Moving);
    assert_ne!(stepped.agent.position, start_position);
    let play_pixels = viewport_pixels(&mut reopened, "11-play-tick-sixteen.png");
    let play_pixel_diff = changed_pixels(&reopened_pixels, &play_pixels);
    assert!(play_pixel_diff > 10);
    click_button(&mut reopened, LBL_STOP);
    assert_eq!(reopened.state().editor.mode(), Mode::Edit);
    assert_eq!(reopened.state().editor.checksum(), scene_checksum);
    assert_eq!(fs::read(&fixture.scene).unwrap(), scene_bytes);
    let export = export_bundle(&mut reopened, &fixture, &art_bytes, &animation_bytes);
    fixture.restore_sources();

    let captures = (1..=11)
        .map(|index| {
            let filename = match index {
                1 => "01-baseline.png",
                2 => "02-static-fixed.png",
                3 => "03-static-route.png",
                4 => "04-static-undo.png",
                5 => "05-static-redo.png",
                6 => "06-animation-fixed.png",
                7 => "07-animation-route.png",
                8 => "08-animation-undo.png",
                9 => "09-animation-redo.png",
                10 => "10-reopened-source-hidden.png",
                _ => "11-play-tick-sixteen.png",
            };
            capture_record(&fixture.captures, filename)
        })
        .collect::<Vec<_>>();
    let graph_revision_hex = hex_bytes(&view.graph.revision());
    let receipt = serde_json::json!({
        "artifact_root":fixture.root,
        "scene_path":fixture.scene,
        "scene_sha256":sha256(&scene_bytes),
        "scene_checksum":scene_checksum,
        "graph_revision":graph_revision_hex,
        "art_sidecar_path":art_path,
        "art_sidecar_sha256":sha256(&art_bytes),
        "static_facing":"route",
        "static_facing_pixel_diff":static_facing_pixel_diff,
        "static_undo_redo_verified":changed_pixels(&fixed_art_pixels,&static_undo_pixels) < 20 && changed_pixels(&route_art_pixels,&static_redo_pixels) < 20,
        "animation_sidecar_path":animation_path,
        "animation_sidecar_sha256":sha256(&animation_bytes),
        "animation_facing":"route",
        "animation_facing_pixel_diff":animation_facing_pixel_diff,
        "animation_undo_redo_verified":changed_pixels(&fixed_animation_pixels,&animation_undo_pixels) < 20 && changed_pixels(&route_animation_pixels,&animation_redo_pixels) < 20,
        "static_source_package":fixture.static_source,
        "static_source_package_sha256":fixture.static_source_package_sha256,
        "static_package_files":fixture.static_package_files,
        "static_model_sha256":fixture.static_model_sha256,
        "static_license_sha256":fixture.static_license_sha256,
        "animation_source_package":fixture.animation_source,
        "animation_source_package_sha256":fixture.animation_source_package_sha256,
        "animation_package_files":fixture.animation_package_files,
        "animation_model_sha256":fixture.animation_model_sha256,
        "animation_license_sha256":fixture.animation_license_sha256,
        "source_packages_hidden_during_reopen":sources_hidden_during_reopen,
        "source_packages_restored_and_hash_checked":fixture.sources_restored,
        "fresh_reopen_route_pixels_match":fresh_reopen_route_pixels_match,
        "fresh_reopen_pixel_diff_from_static":reopen_pixel_diff,
        "play_ticks":16,
        "play_agent_moved":stepped.agent.position != start_position,
        "play_pixel_diff":play_pixel_diff,
        "scene_checksum_unchanged":reopened.state().editor.checksum() == scene_checksum,
        "optional_export":export,
        "captures":captures,
        "physical_os_input":false
    });
    fs::write(
        fixture.root.join("acceptance.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
}
