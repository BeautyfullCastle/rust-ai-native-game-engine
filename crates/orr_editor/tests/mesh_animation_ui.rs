#![cfg(all(feature = "mesh-navigation", feature = "mesh-animated-models"))]
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
    Editor, EditorApp, Mode,
};
use orr_navigation::NavigationStatus;
use orr_remote::mesh_navigation::mesh_from_view;
use orr_rhi::Rhi;
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
    _temp: tempfile::TempDir,
    root: PathBuf,
    scene: PathBuf,
    static_source: PathBuf,
    hidden_static: PathBuf,
    animation_source: PathBuf,
    hidden_animation: PathBuf,
    physics_scene: PathBuf,
    capture_directory: PathBuf,
    static_model_sha256: String,
    static_license_sha256: String,
    animation_model_sha256: String,
    animation_license_sha256: String,
    sources_hidden: bool,
    sources_restored: bool,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let capture_directory = PathBuf::from(
            std::env::var_os("ORR_MESH_ANIMATION_CAPTURE_DIR")
                .expect("animation acceptance capture directory is required"),
        );
        assert!(capture_directory.is_absolute());
        fs::create_dir(&capture_directory).expect("capture directory is fresh");
        let root = capture_directory.canonicalize().unwrap();
        let requested_scene = PathBuf::from(
            std::env::var_os("ORR_MESH_ANIMATION_OUTPUT")
                .unwrap_or_else(|| root.join("mesh-animation.scene.yaml").into_os_string()),
        );
        assert!(requested_scene.is_absolute() && !requested_scene.exists());
        let scene_parent = requested_scene
            .parent()
            .filter(|parent| parent.is_dir())
            .expect("animation scene parent must exist")
            .canonicalize()
            .expect("canonicalize animation scene parent");
        let scene = scene_parent.join(
            requested_scene
                .file_name()
                .expect("animation scene path must have a filename"),
        );
        assert!(!scene.exists());
        let static_source = root.join("local static art package");
        let hidden_static = root.join("local static art package.hidden");
        let animation_source = root.join("local animation package");
        let hidden_animation = root.join("local animation package.hidden");
        fs::create_dir(&static_source).unwrap();
        fs::create_dir(&animation_source).unwrap();
        let static_assets =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/imported_scene_demo");
        for file_name in [
            "orr.package.json",
            STATIC_MODEL,
            "background.glb",
            LICENSE,
            "generate.py",
        ] {
            fs::copy(static_assets.join(file_name), static_source.join(file_name)).unwrap();
        }
        let animation_assets =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/animation_demo");
        for file_name in ["orr.package.json", ANIMATED_MODEL, LICENSE] {
            fs::copy(
                animation_assets.join(file_name),
                animation_source.join(file_name),
            )
            .unwrap();
        }
        let static_model_sha256 = sha256_hex(&fs::read(static_source.join(STATIC_MODEL)).unwrap());
        let static_license_sha256 = sha256_hex(&fs::read(static_source.join(LICENSE)).unwrap());
        let animation_model_sha256 =
            sha256_hex(&fs::read(animation_source.join(ANIMATED_MODEL)).unwrap());
        let animation_license_sha256 =
            sha256_hex(&fs::read(animation_source.join(LICENSE)).unwrap());
        let physics_scene = root.join("owned-stock-physics.scene.yaml");
        fs::copy(EditorGame::PhysGame.default_scene_path(), &physics_scene)
            .expect("copy the shipped Physics scene into the owned fixture");
        Self {
            _temp: temporary,
            root: root.clone(),
            scene,
            static_source,
            hidden_static,
            animation_source,
            hidden_animation,
            physics_scene,
            capture_directory,
            static_model_sha256,
            static_license_sha256,
            animation_model_sha256,
            animation_license_sha256,
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

    fn hide_sources(&mut self) -> Result<(), String> {
        fs::rename(&self.static_source, &self.hidden_static)
            .map_err(|error| format!("hide static package source: {error}"))?;
        self.sources_hidden = true;
        fs::rename(&self.animation_source, &self.hidden_animation)
            .map_err(|error| format!("hide animation package source: {error}"))?;
        assert!(!self.static_source.exists() && !self.animation_source.exists());
        Ok(())
    }

    fn restore_sources(&mut self) -> Result<(), String> {
        if !self.sources_hidden
            || self.static_source.exists()
            || self.animation_source.exists()
            || !self.hidden_static.is_dir()
            || !self.hidden_animation.is_dir()
        {
            return Err("package sources are not in the expected hidden state".into());
        }
        fs::rename(&self.hidden_static, &self.static_source)
            .map_err(|error| format!("restore static package source: {error}"))?;
        fs::rename(&self.hidden_animation, &self.animation_source)
            .map_err(|error| format!("restore animation package source: {error}"))?;
        let hashes = [
            sha256_hex(
                &fs::read(self.static_source.join(STATIC_MODEL))
                    .map_err(|error| error.to_string())?,
            ),
            sha256_hex(
                &fs::read(self.static_source.join(LICENSE)).map_err(|error| error.to_string())?,
            ),
            sha256_hex(
                &fs::read(self.animation_source.join(ANIMATED_MODEL))
                    .map_err(|error| error.to_string())?,
            ),
            sha256_hex(
                &fs::read(self.animation_source.join(LICENSE))
                    .map_err(|error| error.to_string())?,
            ),
        ];
        if hashes
            != [
                self.static_model_sha256.clone(),
                self.static_license_sha256.clone(),
                self.animation_model_sha256.clone(),
                self.animation_license_sha256.clone(),
            ]
        {
            return Err("restored package hashes do not match their originals".into());
        }
        self.sources_hidden = false;
        self.sources_restored = true;
        Ok(())
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

fn gpu_available() -> bool {
    match orr_rhi::Wgpu::headless(orr_rhi::WgpuOptions::default()) {
        Ok(rhi) => {
            eprintln!(
                "Mesh animation GPU preflight: {} (software: {})",
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
                std::env::var_os("ORR_MESH_ANIMATION_ACCEPTANCE").is_none()
                    && std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "required Mesh animation GPU acceptance unavailable: {error}"
            );
            eprintln!("SKIP: Mesh animation GPU unavailable: {error}");
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
            "timed out waiting for {operation}: game={:?}, path={:?}, frame_checksum={}, sim_checksum={}, status={:?}",
            app.editor.game(),
            app.editor.path(),
            app.editor.checksum(),
            app.editor.sim().checksum,
            app.editor.status()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn mesh_view(harness: &Harness<'_, EditorApp>) -> orr_remote::mesh_navigation::MeshView {
    mesh_from_view(harness.state().editor.snapshot().unwrap().predicted()).unwrap()
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
        let ratio = harness.ctx.pixels_per_point();
        harness.set_size(egui::vec2(
            window.x + (target.0 as f32 - actual.0 as f32) / ratio,
            window.y + (target.1 as f32 - actual.1 as f32) / ratio,
        ));
    }
    settle(harness);
    let viewport = harness
        .state()
        .viewport3d_gpu()
        .expect("MeshNavigation production viewport rendered");
    assert_eq!(viewport.gpu().target().size(), target);
    eprintln!("Mesh animation App GPU: {}", viewport.gpu().adapter_name());
    let pixels = viewport.gpu().read_rgba8();
    assert_eq!(pixels.len(), (target.0 * target.1 * 4) as usize);
    assert!(pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0));
    let image =
        egui::ColorImage::from_rgba_unmultiplied([target.0 as usize, target.1 as usize], &pixels);
    orr_editor::app::write_png(
        &PathBuf::from(std::env::var_os("ORR_MESH_ANIMATION_CAPTURE_DIR").unwrap()).join(label),
        &image,
    )
    .unwrap();
    pixels
}

fn changed_pixel_count(left: &[u8], right: &[u8]) -> usize {
    assert_eq!(left.len(), right.len());
    left.as_chunks::<4>()
        .0
        .iter()
        .zip(right.as_chunks::<4>().0.iter())
        .filter(|(first_pixel, second_pixel)| {
            first_pixel[..3]
                .iter()
                .zip(second_pixel[..3].iter())
                .any(|(first, second)| first.abs_diff(*second) > 12)
        })
        .count()
}

fn skeletal_pose_difference(document: &serde_json::Value, tick: u64) -> usize {
    let source: orr_model::animation::AnimatedSource =
        serde_json::from_value(document["model"].clone()).unwrap();
    let model = orr_model::animation::AnimatedModel::new(source).unwrap();
    let settings = &document["settings"];
    let clip = settings["clip"].as_u64().unwrap() as u32;
    let rate = settings["rate"].as_f64().unwrap() as f32;
    let duration = model.source().clips[clip as usize].duration();
    let sample_time = if duration > 0.0 {
        ((tick as f32 / 60.0) * rate) % duration
    } else {
        0.0
    };
    let initial_pose = model.sample_clip(clip, 0.0).unwrap();
    let sampled_pose = model.sample_clip(clip, sample_time).unwrap();
    let global_difference = initial_pose
        .global()
        .iter()
        .flatten()
        .flatten()
        .zip(sampled_pose.global().iter().flatten().flatten())
        .filter(|(left, right)| (**left - **right).abs() > 0.00001)
        .count();
    let skin_difference = initial_pose
        .skin_matrices()
        .iter()
        .flatten()
        .flatten()
        .flatten()
        .zip(
            sampled_pose
                .skin_matrices()
                .iter()
                .flatten()
                .flatten()
                .flatten(),
        )
        .filter(|(left, right)| (**left - **right).abs() > 0.00001)
        .count();
    global_difference + skin_difference
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sidecar_path(scene: &Path, suffix: &str) -> PathBuf {
    let mut name = scene.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
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
fn export_bundle(
    harness: &mut Harness<'_, EditorApp>,
    fixture: &Fixture,
    animation_bytes: &[u8],
    art_bytes: &[u8],
) -> serde_json::Value {
    let Some(output) = std::env::var_os("ORR_MESH_ANIMATION_EXPORT_OUTPUT") else {
        return serde_json::Value::Null;
    };
    let output = PathBuf::from(output);
    let runtime = PathBuf::from(
        std::env::var_os("ORR_MESH_EXPORT_RUNTIME")
            .expect("export acceptance requires the archived player"),
    );
    assert!(output.is_absolute() && output.parent().is_some_and(Path::is_dir) && !output.exists());
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
        "mesh animation export did not complete"
    );
    let animation_name = format!(
        "{}.mesh-animation.json",
        fixture.scene.file_name().unwrap().to_string_lossy()
    );
    let art_name = format!(
        "{}.mesh-art.json",
        fixture.scene.file_name().unwrap().to_string_lossy()
    );
    let scene_name = fixture
        .scene
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        fs::read(output.join(&animation_name)).unwrap(),
        animation_bytes
    );
    assert_eq!(fs::read(output.join(&art_name)).unwrap(), art_bytes);
    assert_eq!(
        fs::read(output.join(&scene_name)).unwrap(),
        fs::read(&fixture.scene).unwrap()
    );
    let mut names = fs::read_dir(&output)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    let mut expected = vec![
        "orr.mesh-bundle.json".to_owned(),
        scene_name,
        animation_name,
        art_name,
        "orr_mesh_player.exe".to_owned(),
    ];
    expected.sort();
    assert_eq!(names, expected);
    serde_json::json!({"directory": output.display().to_string(), "files": names, "animation_sidecar_sha256": sha256_hex(animation_bytes), "art_sidecar_sha256": sha256_hex(art_bytes)})
}

#[cfg(not(feature = "mesh-export"))]
fn export_bundle(
    _harness: &mut Harness<'_, EditorApp>,
    _fixture: &Fixture,
    _animation_bytes: &[u8],
    _art_bytes: &[u8],
) -> serde_json::Value {
    assert!(
        std::env::var_os("ORR_MESH_ANIMATION_EXPORT_OUTPUT").is_none(),
        "export output requested without mesh-export feature"
    );
    serde_json::Value::Null
}

#[test]
#[ignore = "requires ORR_MESH_ANIMATION_ACCEPTANCE=1 and a hardware Vulkan adapter"]
fn mesh_animation_is_saved_source_free_and_overrides_only_static_agent_art() {
    let acceptance = std::env::var_os("ORR_MESH_ANIMATION_ACCEPTANCE").is_some();
    let requested = acceptance || std::env::var_os("ORR_REQUIRE_GPU").is_some();
    if !requested {
        eprintln!(
            "SKIP: set ORR_MESH_ANIMATION_ACCEPTANCE=1 for actual GPU mesh-animation UI acceptance"
        );
        return;
    }
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
    let graph_revision = mesh_view(&harness).graph.revision();
    let baseline_pixels = viewport_pixels(&mut harness, "baseline.png");
    let art_sidecar = sidecar_path(&fixture.scene, ".mesh-art.json");
    let animation_sidecar = sidecar_path(&fixture.scene, ".mesh-animation.json");
    assert!(!art_sidecar.exists() && !animation_sidecar.exists());

    if harness
        .query_by_label("Art package source directory")
        .is_none()
    {
        click(&mut harness, "Mesh agent art");
    }
    type_field(
        &mut harness,
        "Art package source directory",
        &fixture.static_source.display().to_string(),
    );
    type_field(&mut harness, "Art model asset", STATIC_MODEL);
    type_field(&mut harness, "Art license asset", LICENSE);
    click(&mut harness, "I have reviewed rights to use this agent art");
    click_button(&mut harness, "Load agent art");
    assert!(harness
        .query_by_label_contains("Loaded static package art sample-imported-scene/foreground.glb")
        .is_some());
    click_button(&mut harness, "Apply agent art");
    click_button(&mut harness, "Save agent art");
    let static_art_bytes = fs::read(&art_sidecar).expect("static agent art saved before animation");
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    let static_art_pixels = viewport_pixels(&mut harness, "static-art.png");
    assert!(changed_pixel_count(&baseline_pixels, &static_art_pixels) > 20);

    if harness
        .query_by_label("Animation package source directory")
        .is_none()
    {
        click(&mut harness, "Mesh agent animation");
    }
    type_field(
        &mut harness,
        "Animation package source directory",
        &fixture.animation_source.display().to_string(),
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
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    assert_eq!(mesh_view(&harness).graph.revision(), graph_revision);
    let loaded_pixels = viewport_pixels(&mut harness, "loaded-not-applied.png");
    assert_eq!(changed_pixel_count(&static_art_pixels, &loaded_pixels), 0);

    type_field(&mut harness, "Animation offset X", "0.25");
    type_field(&mut harness, "Animation offset Y", "0.5");
    type_field(&mut harness, "Animation offset Z", "0.25");
    type_field(&mut harness, "Animation scale", "0.5");
    type_field(&mut harness, "Animation rotation X (degrees)", "15");
    type_field(&mut harness, "Animation clip index", "1");
    type_field(&mut harness, "Animation playback rate", "1.5");
    click_button(&mut harness, "Apply mesh animation");
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    assert_eq!(mesh_view(&harness).graph.revision(), graph_revision);
    assert!(harness
        .get_by_label(LBL_PLAY)
        .accesskit_node()
        .is_disabled());
    let animation_pixels = viewport_pixels(&mut harness, "animation-applied.png");
    let applied_pixel_diff = changed_pixel_count(&static_art_pixels, &animation_pixels);
    assert!(
        applied_pixel_diff > 20,
        "skinned animation presentation should replace static art pixels"
    );
    click_button(&mut harness, "Undo mesh animation");
    let undone_pixels = viewport_pixels(&mut harness, "animation-undone.png");
    assert!(changed_pixel_count(&static_art_pixels, &undone_pixels) < 20);
    click_button(&mut harness, "Redo mesh animation");
    let redone_pixels = viewport_pixels(&mut harness, "animation-redone.png");
    assert!(changed_pixel_count(&animation_pixels, &redone_pixels) < 20);
    click_button(&mut harness, "Save mesh animation");
    let animation_bytes =
        fs::read(&animation_sidecar).expect("explicit Save mesh animation writes sidecar");
    let animation_document: serde_json::Value = serde_json::from_slice(&animation_bytes).unwrap();
    assert_eq!(
        animation_document["scene_filename"],
        fixture
            .scene
            .file_name()
            .unwrap()
            .to_string_lossy()
            .as_ref()
    );
    assert_eq!(animation_document["manifest"]["name"], "sample-animation");
    assert_eq!(animation_document["asset"], ANIMATED_MODEL);
    assert_eq!(
        animation_document["asset_sha256"],
        fixture.animation_model_sha256
    );
    assert_eq!(animation_document["license"], LICENSE);
    assert_eq!(
        animation_document["license_sha256"],
        fixture.animation_license_sha256
    );
    assert_eq!(
        animation_document["license_text"],
        fs::read_to_string(fixture.animation_source.join(LICENSE)).unwrap()
    );
    assert!(animation_document["model"].is_object());
    assert!(animation_document["cooked_sha256"].as_str().is_some());
    assert_eq!(
        animation_document["graph_revision"],
        serde_json::json!(graph_revision)
    );
    assert_eq!(
        animation_document["settings"]["offset"],
        serde_json::json!([0.25, 0.5, 0.25])
    );
    assert_eq!(
        animation_document["settings"]["scale"],
        serde_json::json!(0.5)
    );
    assert_eq!(animation_document["settings"]["clip"], serde_json::json!(1));
    assert_eq!(
        animation_document["settings"]["rate"],
        serde_json::json!(1.5)
    );
    assert_eq!(
        animation_document["settings"]["looped"],
        serde_json::json!(true)
    );
    let skeletal_pose_difference = skeletal_pose_difference(&animation_document, 16);
    assert!(
        skeletal_pose_difference > 0,
        "the selected clip must change its validated skeletal pose over 16 ticks"
    );
    assert_eq!(fs::read(&art_sidecar).unwrap(), static_art_bytes);
    assert_eq!(fs::read(&fixture.scene).unwrap(), scene_bytes);
    let optional_export =
        export_bundle(&mut harness, &fixture, &animation_bytes, &static_art_bytes);

    fixture
        .hide_sources()
        .expect("hide both owned package sources transactionally");
    drop(harness);
    let mut reopened = fixture.harness(&fixture.physics_scene, EditorGame::PhysGame);
    reopened.run_steps(4);
    menu_click(&mut reopened, "File", "Open mesh scene…");
    type_scene_path(&mut reopened, &fixture.scene, "OK");
    wait_for(&mut reopened, "fresh source-free animation scene", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && app.editor.path().as_deref().is_some_and(|path| {
                path == fixture.scene.as_path()
                    || path.canonicalize().ok() == fixture.scene.canonicalize().ok()
            })
            && app.editor.checksum() == scene_checksum
    });
    assert!(!fixture.static_source.exists() && !fixture.animation_source.exists());
    assert_eq!(fs::read(&fixture.scene).unwrap(), scene_bytes);
    assert_eq!(fs::read(&art_sidecar).unwrap(), static_art_bytes);
    assert_eq!(fs::read(&animation_sidecar).unwrap(), animation_bytes);
    let reopened_pixels = viewport_pixels(&mut reopened, "source-free-reopen.png");
    assert!(changed_pixel_count(&animation_pixels, &reopened_pixels) < 20);
    assert!(changed_pixel_count(&static_art_pixels, &reopened_pixels) > 20);
    click_button(&mut reopened, LBL_PLAY);
    wait_for_playing(&mut reopened);
    click_button(&mut reopened, LBL_PAUSE);
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
    let play_pixels = viewport_pixels(&mut reopened, "source-free-play16.png");
    let play_pixel_diff = changed_pixel_count(&reopened_pixels, &play_pixels);
    assert!(
        play_pixel_diff > 10,
        "animation should follow the moving agent on actual GPU frames"
    );
    click_button(&mut reopened, LBL_STOP);
    assert_eq!(reopened.state().editor.mode(), Mode::Edit);
    assert_eq!(reopened.state().editor.checksum(), scene_checksum);
    assert_eq!(fs::read(&art_sidecar).unwrap(), static_art_bytes);
    if reopened
        .query_by_label("I confirm removing saved mesh animation")
        .is_none()
    {
        click(&mut reopened, "Mesh agent animation");
    }
    click(&mut reopened, "I confirm removing saved mesh animation");
    click_button(&mut reopened, "Remove mesh animation");
    assert!(
        animation_sidecar.exists(),
        "removal remains unsaved until explicit Save"
    );
    click_button(&mut reopened, "Save mesh animation");
    assert!(!animation_sidecar.exists());
    let restored_static_pixels = viewport_pixels(&mut reopened, "static-art-restored.png");
    assert!(changed_pixel_count(&static_art_pixels, &restored_static_pixels) < 20);
    assert_eq!(fs::read(&art_sidecar).unwrap(), static_art_bytes);
    click_button(&mut reopened, "Undo mesh animation");
    click_button(&mut reopened, "Save mesh animation");
    assert_eq!(fs::read(&animation_sidecar).unwrap(), animation_bytes);
    let restored_animation_pixels = viewport_pixels(&mut reopened, "animation-restored.png");
    assert!(changed_pixel_count(&animation_pixels, &restored_animation_pixels) < 20);
    drop(reopened);
    fixture
        .restore_sources()
        .expect("restore both owned package sources and verify hashes");

    let capture_names = [
        "baseline.png",
        "static-art.png",
        "loaded-not-applied.png",
        "animation-applied.png",
        "animation-undone.png",
        "animation-redone.png",
        "source-free-reopen.png",
        "source-free-play16.png",
        "static-art-restored.png",
        "animation-restored.png",
    ];
    let captures = capture_names
        .iter()
        .map(|name| {
            let bytes = fs::read(fixture.capture_directory.join(name)).unwrap();
            serde_json::json!({"path": name, "bytes": bytes.len(), "sha256": sha256_hex(&bytes)})
        })
        .collect::<Vec<_>>();
    fs::write(fixture.capture_directory.join("acceptance.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "artifact_root": fixture.root.display().to_string(),
        "scene_path": fixture.scene.display().to_string(),
        "scene_sha256": sha256_hex(&scene_bytes),
        "scene_checksum": format!("0x{scene_checksum:016x}"),
        "graph_revision": graph_revision,
        "static_art_sidecar": art_sidecar.display().to_string(),
        "static_art_sidecar_sha256": sha256_hex(&static_art_bytes),
        "animation_sidecar": animation_sidecar.display().to_string(),
        "animation_sidecar_sha256": sha256_hex(&animation_bytes),
        "animation_source_package": fixture.animation_source.display().to_string(),
        "animation_asset_sha256": fixture.animation_model_sha256,
        "animation_license_sha256": fixture.animation_license_sha256,
        "static_source_package": fixture.static_source.display().to_string(),
        "static_asset_sha256": fixture.static_model_sha256,
        "static_license_sha256": fixture.static_license_sha256,
        "sources_hidden_during_reopen": true,
        "sources_restored": fixture.sources_restored,
        "animation_clip": animation_document["settings"]["clip"],
        "animation_rate": animation_document["settings"]["rate"],
        "animation_looped": animation_document["settings"]["looped"],
        "skeletal_pose_difference_elements_at_tick_16": skeletal_pose_difference,
        "animation_restored_after_remove_undo": fs::read(&animation_sidecar).ok().as_deref() == Some(animation_bytes.as_slice()),
        "static_art_preserved_and_restored": changed_pixel_count(&static_art_pixels, &restored_static_pixels) < 20,
        "loaded_candidate_left_view_unchanged": changed_pixel_count(&static_art_pixels, &loaded_pixels) == 0,
        "animation_apply_pixel_diff": applied_pixel_diff,
        "fresh_reopen_pixels_match": changed_pixel_count(&animation_pixels, &reopened_pixels) < 20,
        "play_tick": 16,
        "play_agent_moved": stepped.agent.position != start_position,
        "play_pixel_diff": play_pixel_diff,
        "scene_checksum_unchanged": true,
        "optional_export": optional_export,
        "captures": captures,
        "physical_os_input": false
    })).unwrap()).unwrap();
}
