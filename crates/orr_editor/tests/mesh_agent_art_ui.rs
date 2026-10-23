#![cfg(feature = "mesh-navigation")]
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

const BLANK_SCENE: &str = "schema: orr.scene/1\nsingletons: {}\nentities: {}\n";
const SOURCE_PACKAGE: &str = "sample-imported-scene";
const MODEL_ASSET: &str = "foreground.glb";
const LICENSE_ASSET: &str = "LICENSE.txt";

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    scene: PathBuf,
    source: PathBuf,
    hidden_source: PathBuf,
    bootstrap: PathBuf,
    capture_directory: Option<PathBuf>,
    model_sha256: String,
    license_sha256: String,
    source_hidden: bool,
    source_restored: bool,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let capture_directory =
            std::env::var_os("ORR_MESH_AGENT_ART_CAPTURE_DIR").map(PathBuf::from);
        if let Some(directory) = capture_directory.as_ref() {
            assert!(directory.is_absolute());
            fs::create_dir(directory).expect("capture directory must be fresh");
        }
        let root = capture_directory
            .as_deref()
            .unwrap_or(temporary.path())
            .canonicalize()
            .unwrap();
        let scene = std::env::var_os("ORR_MESH_AGENT_ART_OUTPUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("mesh-art.scene.yaml"));
        assert!(scene.is_absolute(), "scene output must be absolute");
        assert!(!scene.exists(), "scene output must not already exist");
        assert!(scene.parent().is_some_and(Path::is_dir));
        let source = root.join("local static art package");
        let hidden_source = root.join("local static art package.hidden");
        let bootstrap = root.join("bootstrap.scene.yaml");
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
        let model_sha256 = sha256_hex(&fs::read(source.join(MODEL_ASSET)).unwrap());
        let license_sha256 = sha256_hex(&fs::read(source.join(LICENSE_ASSET)).unwrap());
        Self {
            _temp: temporary,
            root,
            scene,
            source,
            hidden_source,
            bootstrap,
            capture_directory,
            model_sha256,
            license_sha256,
            source_hidden: false,
            source_restored: false,
        }
    }

    fn harness(&self, scene: &Path, create_blank: bool) -> Harness<'static, EditorApp> {
        if create_blank {
            fs::write(scene, BLANK_SCENE).unwrap();
        }
        let mut editor = Editor::open_game(scene, EditorGame::MeshNavigation).unwrap();
        editor.sync();
        Harness::builder()
            .with_size([1500.0, 1000.0])
            .with_step_dt(1.0 / 60.0)
            .wgpu_setup(hardware_vulkan_setup())
            .build_eframe(move |creation_context| {
                EditorApp::new(editor, creation_context.wgpu_render_state.clone())
            })
    }

    fn hide_source(&mut self) {
        fs::rename(&self.source, &self.hidden_source)
            .expect("hide the local package source after saving embedded art");
        self.source_hidden = true;
        assert!(!self.source.exists());
        assert!(self.hidden_source.is_dir());
    }

    fn restore_source(&mut self) -> Result<(), String> {
        if !self.source_hidden || self.source.exists() || !self.hidden_source.is_dir() {
            return Err("package source is not in the expected hidden state".into());
        }
        fs::rename(&self.hidden_source, &self.source)
            .map_err(|error| format!("restore package source directory: {error}"))?;
        let model_sha256 = sha256_hex(
            &fs::read(self.source.join(MODEL_ASSET))
                .map_err(|error| format!("read restored model source: {error}"))?,
        );
        let license_sha256 = sha256_hex(
            &fs::read(self.source.join(LICENSE_ASSET))
                .map_err(|error| format!("read restored license source: {error}"))?,
        );
        if model_sha256 != self.model_sha256 || license_sha256 != self.license_sha256 {
            return Err("restored package source hashes do not match the originals".into());
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
                eprintln!("Could not restore hidden mesh-art package source: {error}");
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
                "Mesh agent art GPU preflight: {} (software: {})",
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
                || std::env::var_os("ORR_MESH_AGENT_ART_ACCEPTANCE").is_some();
            assert!(
                !required,
                "Mesh agent art GPU acceptance unavailable: {error}"
            );
            eprintln!("SKIP: Mesh agent art GPU is unavailable: {error}");
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

fn path_matches(actual: Option<&Path>, expected: &Path) -> bool {
    actual.is_some_and(|actual| {
        actual == expected
            || actual
                .canonicalize()
                .ok()
                .zip(expected.canonicalize().ok())
                .is_some_and(|(actual, expected)| actual == expected)
    })
}

fn wait_for(
    harness: &mut Harness<'_, EditorApp>,
    label: &str,
    predicate: impl Fn(&EditorApp) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        settle(harness);
        if predicate(harness.state()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Timed out waiting for {label}: {:?}",
            harness.state().editor.status()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
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
        "missing menu item {item}"
    );
    harness.get_by_label(item).click_accesskit();
    settle(harness);
}

fn type_field(harness: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    let field = harness.get_by_role_and_label(Role::TextInput, label);
    field.scroll_to_me();
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

fn mesh_view(harness: &Harness<'_, EditorApp>) -> orr_remote::mesh_navigation::MeshView {
    let snapshot = harness.state().editor.snapshot().unwrap();
    mesh_from_view(snapshot.predicted()).unwrap()
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
        .expect("the production mesh-navigation viewport rendered");
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

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_bytes(&Sha256::digest(bytes))
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
    panic!("mesh-navigation host did not publish a playing frame");
}

fn capture(directory: &Path, name: &str, harness: &mut Harness<'_, EditorApp>) {
    let pixels = viewport_pixels(harness);
    let viewport = harness.state().viewport3d_gpu().unwrap();
    let size = viewport.gpu().target().size();
    let image =
        egui::ColorImage::from_rgba_unmultiplied([size.0 as usize, size.1 as usize], &pixels);
    orr_editor::app::write_png(&directory.join(format!("{name}.png")), &image).unwrap();
}

fn sidecar_for_scene(scene: &Path) -> PathBuf {
    let mut sidecar_name = scene.as_os_str().to_os_string();
    sidecar_name.push(".mesh-art.json");
    PathBuf::from(sidecar_name)
}

#[test]
fn mesh_agent_art_is_authored_saved_and_reopened_without_its_source_package() {
    let requested = [
        "ORR_REQUIRE_GPU",
        "ORR_MESH_AGENT_ART_ACCEPTANCE",
        "ORR_MESH_AGENT_ART_OUTPUT",
        "ORR_MESH_AGENT_ART_CAPTURE_DIR",
    ]
    .iter()
    .any(|name| std::env::var_os(name).is_some());
    if !requested {
        eprintln!(
            "SKIP: set ORR_REQUIRE_GPU=1 or ORR_MESH_AGENT_ART_ACCEPTANCE=1 for actual GPU mesh-agent-art UI acceptance"
        );
        return;
    }
    assert_eq!(
        std::env::var("WGPU_BACKEND").unwrap_or_default(),
        "vulkan",
        "mesh-agent-art acceptance requires WGPU_BACKEND=vulkan"
    );
    assert!(gpu_available());

    let mut fixture = Fixture::new();
    let capture_directory = fixture.capture_directory.clone();
    let mut harness = fixture.harness(&fixture.bootstrap, true);
    harness.run_steps(4);
    assert!(harness.state().has_gpu());
    menu_click(&mut harness, "File", "New mesh scene…");
    type_scene_path(&mut harness, &fixture.scene, "Create");
    assert!(!sidecar_for_scene(&fixture.scene).exists());
    click(&mut harness, "Static-mesh navigation");
    click(&mut harness, "Create ramp and bridge");
    click(&mut harness, "Place mesh agent");
    menu_click(&mut harness, "File", "Save Ctrl+S");

    let saved_scene_bytes = fs::read(&fixture.scene).unwrap();
    let scene_checksum = harness.state().editor.checksum();
    let initial_view = mesh_view(&harness);
    let graph_revision = initial_view.graph.revision();
    let initial_pixels = viewport_pixels(&mut harness);

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
    assert!(!harness
        .get_by_label("Load agent art")
        .accesskit_node()
        .is_disabled());
    click(&mut harness, "Load agent art");
    assert!(harness
        .query_by_label(&format!(
            "Loaded static package art {SOURCE_PACKAGE}/{MODEL_ASSET}; not applied."
        ))
        .is_some());
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    assert_eq!(mesh_view(&harness).graph.revision(), graph_revision);
    let candidate_pixels = viewport_pixels(&mut harness);
    assert_eq!(changed_pixel_count(&initial_pixels, &candidate_pixels), 0);

    type_field(&mut harness, "Agent art offset X", "0.25");
    type_field(&mut harness, "Agent art offset Y", "0.5");
    type_field(&mut harness, "Agent art offset Z", "0.25");
    type_field(&mut harness, "Agent art scale", "0.5");
    click(&mut harness, "Apply agent art");
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    assert_eq!(mesh_view(&harness).graph.revision(), graph_revision);
    let applied_pixels = viewport_pixels(&mut harness);
    assert!(
        changed_pixel_count(&initial_pixels, &applied_pixels) > 20,
        "applying cosmetic art should change actual viewport pixels"
    );
    assert!(harness
        .get_by_label(LBL_PLAY)
        .accesskit_node()
        .is_disabled());

    type_field(&mut harness, "Agent art scale", "0");
    click(&mut harness, "Apply agent art");
    assert!(harness
        .query_by_label_contains("Agent art transform rejected:")
        .is_some());
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    assert_eq!(mesh_view(&harness).graph.revision(), graph_revision);
    let rejected_transform_pixels = viewport_pixels(&mut harness);
    assert_eq!(
        changed_pixel_count(&applied_pixels, &rejected_transform_pixels),
        0
    );
    type_field(&mut harness, "Agent art scale", "0.5");

    click(&mut harness, "Undo agent art");
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    let undone_pixels = viewport_pixels(&mut harness);
    assert_eq!(changed_pixel_count(&initial_pixels, &undone_pixels), 0);
    click(&mut harness, "Redo agent art");
    let redone_pixels = viewport_pixels(&mut harness);
    assert!(changed_pixel_count(&applied_pixels, &redone_pixels) < 20);
    assert_eq!(mesh_view(&harness).graph.revision(), graph_revision);

    click(&mut harness, "Save agent art");
    let sidecar = sidecar_for_scene(&fixture.scene);
    let sidecar_bytes = fs::read(&sidecar).expect("explicit Save agent art writes a sidecar");
    assert!(!sidecar_bytes.is_empty());
    let saved_art: serde_json::Value = serde_json::from_slice(&sidecar_bytes).unwrap();
    assert_eq!(saved_art["manifest"]["name"], SOURCE_PACKAGE);
    assert_eq!(saved_art["asset"], MODEL_ASSET);
    assert_eq!(saved_art["asset_sha256"], fixture.model_sha256);
    assert_eq!(saved_art["license"], LICENSE_ASSET);
    assert_eq!(saved_art["license_sha256"], fixture.license_sha256);
    assert_eq!(
        saved_art["license_text"],
        fs::read_to_string(fixture.source.join(LICENSE_ASSET)).unwrap()
    );
    assert!(saved_art["model"].is_object());
    assert!(saved_art["cooked_sha256"].as_str().is_some());
    assert_eq!(
        saved_art["graph_revision"],
        serde_json::json!(graph_revision)
    );
    assert_eq!(saved_art["offset"], serde_json::json!([0.25, 0.5, 0.25]));
    assert_eq!(saved_art["scale"], serde_json::json!(0.5));
    assert_eq!(fs::read(&fixture.scene).unwrap(), saved_scene_bytes);
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    assert!(!harness
        .get_by_label(LBL_PLAY)
        .accesskit_node()
        .is_disabled());
    type_field(&mut harness, "Art model asset", "missing.glb");
    click(&mut harness, "I have reviewed rights to use this agent art");
    click(&mut harness, "Load agent art");
    assert!(harness
        .query_by_label_contains("Agent art load failed:")
        .is_some());
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    assert_eq!(mesh_view(&harness).graph.revision(), graph_revision);
    assert_eq!(fs::read(&sidecar).unwrap(), sidecar_bytes);
    let rejected_source_pixels = viewport_pixels(&mut harness);
    assert_eq!(
        changed_pixel_count(&applied_pixels, &rejected_source_pixels),
        0
    );
    type_field(&mut harness, "Art model asset", MODEL_ASSET);
    click(&mut harness, "I have reviewed rights to use this agent art");
    click(&mut harness, "Load agent art");
    assert!(harness
        .query_by_label(&format!(
            "Loaded static package art {SOURCE_PACKAGE}/{MODEL_ASSET}; not applied."
        ))
        .is_some());
    if let Some(directory) = capture_directory.as_deref() {
        capture(directory, "saved-art", &mut harness);
    }

    let save_as_scene = fixture.root.join("mesh-art-save-as.scene.yaml");
    let save_as_sidecar = sidecar_for_scene(&save_as_scene);
    assert!(!save_as_scene.exists());
    assert!(!save_as_sidecar.exists());
    menu_click(&mut harness, "File", "Save As…");
    type_scene_path(&mut harness, &save_as_scene, "OK");
    wait_for(&mut harness, "Save As mesh scene path", |app| {
        path_matches(app.editor.path().as_deref(), &save_as_scene)
    });
    assert!(save_as_scene.is_file());
    assert_eq!(fs::read(&fixture.scene).unwrap(), saved_scene_bytes);
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    assert!(harness
        .get_by_label(LBL_PLAY)
        .accesskit_node()
        .is_disabled());
    click(&mut harness, "Save agent art");
    let save_as_sidecar_bytes =
        fs::read(&save_as_sidecar).expect("Save agent art writes the Save As sidecar");
    let save_as_art: serde_json::Value = serde_json::from_slice(&save_as_sidecar_bytes).unwrap();
    let save_as_filename = save_as_scene
        .file_name()
        .and_then(|name| name.to_str())
        .expect("Save As scene has a UTF-8 leaf name");
    assert_eq!(save_as_art["scene_filename"], save_as_filename);
    assert_eq!(
        save_as_art["manifest"]["name"],
        saved_art["manifest"]["name"]
    );
    assert_eq!(save_as_art["package_digest"], saved_art["package_digest"]);
    assert_eq!(save_as_art["asset"], saved_art["asset"]);
    assert_eq!(save_as_art["asset_sha256"], saved_art["asset_sha256"]);
    assert_eq!(save_as_art["license"], saved_art["license"]);
    assert_eq!(save_as_art["license_sha256"], saved_art["license_sha256"]);
    assert_eq!(save_as_art["license_text"], saved_art["license_text"]);
    assert_eq!(save_as_art["cooked_sha256"], saved_art["cooked_sha256"]);
    assert!(!harness
        .get_by_label(LBL_PLAY)
        .accesskit_node()
        .is_disabled());
    menu_click(&mut harness, "File", "Open scene…");
    type_scene_path(&mut harness, &fixture.scene, "OK");
    wait_for(&mut harness, "reopening original mesh-art scene", |app| {
        path_matches(app.editor.path().as_deref(), &fixture.scene)
            && app.editor.checksum() == scene_checksum
    });
    assert_eq!(fs::read(&fixture.scene).unwrap(), saved_scene_bytes);
    assert_eq!(fs::read(&sidecar).unwrap(), sidecar_bytes);
    assert_eq!(mesh_view(&harness).graph.revision(), graph_revision);
    let restored_original_pixels = viewport_pixels(&mut harness);
    assert!(changed_pixel_count(&applied_pixels, &restored_original_pixels) < 20);
    assert!(!harness
        .get_by_label(LBL_PLAY)
        .accesskit_node()
        .is_disabled());

    fixture.hide_source();
    assert!(!fixture.source.exists());
    let source_hidden_during_reopen = true;
    drop(harness);

    let mut harness = fixture.harness(&fixture.scene, false);
    harness.run_steps(4);
    settle(&mut harness);
    assert!(harness.state().has_gpu());
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    assert_eq!(fs::read(&fixture.scene).unwrap(), saved_scene_bytes);
    assert_eq!(fs::read(&sidecar).unwrap(), sidecar_bytes);
    assert_eq!(mesh_view(&harness).graph.revision(), graph_revision);
    let reopened_pixels = viewport_pixels(&mut harness);
    assert!(
        changed_pixel_count(&applied_pixels, &reopened_pixels) < 20,
        "reopening should render the same saved art with its package source hidden"
    );
    let reopened_art_change_pixels = changed_pixel_count(&applied_pixels, &reopened_pixels);
    if let Some(directory) = capture_directory.as_deref() {
        capture(directory, "reopened-art", &mut harness);
    }

    if harness
        .query_by_label("Art package source directory")
        .is_none()
    {
        click(&mut harness, "Mesh agent art");
    }
    let stale_scene = fixture.root.join("stale-revision.scene.yaml");
    let stale_sidecar = sidecar_for_scene(&stale_scene);
    fs::write(&stale_scene, &saved_scene_bytes).unwrap();
    let mut stale_art: serde_json::Value = serde_json::from_slice(&sidecar_bytes).unwrap();
    let stale_scene_name = stale_scene
        .file_name()
        .and_then(|name| name.to_str())
        .expect("stale scene fixture has a UTF-8 file name");
    stale_art["scene_filename"] = serde_json::json!(stale_scene_name);
    let mut stale_revision = graph_revision;
    stale_revision[0] ^= 0xff;
    stale_art["graph_revision"] = serde_json::json!(stale_revision);
    fs::write(&stale_sidecar, serde_json::to_vec(&stale_art).unwrap()).unwrap();
    menu_click(&mut harness, "File", "Open scene…");
    type_scene_path(&mut harness, &stale_scene, "OK");
    assert_eq!(
        harness
            .state()
            .editor
            .path()
            .unwrap()
            .canonicalize()
            .unwrap(),
        stale_scene.canonicalize().unwrap(),
        "stale scene open status: {:?}",
        harness.state().editor.status()
    );
    harness
        .get_by_label("Discard invalid agent art sidecar")
        .scroll_to_me();
    harness.run_steps(30);
    settle(&mut harness);
    assert!(
        harness
            .query_by_label_contains(
                "mesh art sidecar belongs to a different scene or mesh revision"
            )
            .is_some(),
        "stale sidecar UI/status: {:?}\n{:#?}",
        harness.state().editor.status(),
        harness.root()
    );
    assert!(harness
        .get_by_label(LBL_PLAY)
        .accesskit_node()
        .is_disabled());
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    assert_eq!(mesh_view(&harness).graph.revision(), graph_revision);
    click(&mut harness, "I confirm discarding invalid agent art");
    click(&mut harness, "Discard invalid agent art sidecar");
    assert!(!stale_sidecar.exists());
    menu_click(&mut harness, "File", "Open scene…");
    type_scene_path(&mut harness, &fixture.scene, "OK");
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    assert_eq!(mesh_view(&harness).graph.revision(), graph_revision);
    let recovered_pixels = viewport_pixels(&mut harness);
    assert!(changed_pixel_count(&applied_pixels, &recovered_pixels) < 20);

    if harness
        .query_by_label("I have reviewed rights to use this agent art")
        .is_none()
    {
        click(&mut harness, "Mesh agent art");
    }
    click_fast(&mut harness, LBL_PLAY);
    wait_for_playing(&mut harness);
    assert_eq!(harness.state().editor.mode(), Mode::Play);
    click_fast(&mut harness, LBL_PAUSE);
    assert!(!harness.state().editor.timeline().unwrap().playing);
    harness.state_mut().editor.seek(0);
    settle(&mut harness);
    let start_position = mesh_view(&harness).agent.position;
    for _ in 0..16 {
        click(&mut harness, LBL_STEP);
    }
    let paused_view = mesh_view(&harness);
    assert_eq!(paused_view.agent.status, NavigationStatus::Moving);
    assert_eq!(harness.state().editor.timeline().unwrap().tick, 16);
    assert_ne!(paused_view.agent.position, start_position);
    let paused_position = paused_view.agent.position;
    let moving_pixels = viewport_pixels(&mut harness);
    let moved_art_change_pixels = changed_pixel_count(&reopened_pixels, &moving_pixels);
    assert!(moved_art_change_pixels > 10);
    if let Some(directory) = capture_directory.as_deref() {
        capture(directory, "moving-art", &mut harness);
    }
    click(&mut harness, LBL_STOP);
    assert_eq!(harness.state().editor.mode(), Mode::Edit);
    assert_eq!(harness.state().editor.checksum(), scene_checksum);

    if harness
        .query_by_label("I confirm removing saved agent art")
        .is_none()
    {
        click(&mut harness, "Mesh agent art");
    }
    click(&mut harness, "I confirm removing saved agent art");
    click(&mut harness, "Remove agent art");
    assert!(sidecar.exists(), "removal remains an explicit unsaved edit");
    click(&mut harness, "Save agent art");
    assert!(
        !sidecar.exists(),
        "confirmed removal deletes the scene sidecar"
    );
    let removed_pixels = viewport_pixels(&mut harness);
    let removed_art_change_pixels = changed_pixel_count(&applied_pixels, &removed_pixels);
    assert!(removed_art_change_pixels > 20);
    assert_eq!(fs::read(&fixture.scene).unwrap(), saved_scene_bytes);
    click(&mut harness, "Undo agent art");
    click(&mut harness, "Save agent art");
    assert_eq!(fs::read(&sidecar).unwrap(), sidecar_bytes);
    let source_hidden = !fixture.source.exists() && fixture.hidden_source.is_dir();
    fixture
        .restore_source()
        .expect("restore package source and verify original model/license hashes");
    if let Some(directory) = capture_directory.as_deref() {
        fs::write(
            directory.join("acceptance.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "artifact_root": fixture.root.display().to_string(),
                "scene_path": fixture.scene.display().to_string(),
                "sidecar_path": sidecar.display().to_string(),
                "package_source": fixture.source.display().to_string(),
                "package_source_hidden_path": fixture.hidden_source.display().to_string(),
                "scene_sha256": hex_bytes(&Sha256::digest(&saved_scene_bytes)),
                "sidecar_sha256": hex_bytes(&Sha256::digest(&sidecar_bytes)),
                "initial_checksum": format!("0x{scene_checksum:016x}"),
                "graph_revision": graph_revision.iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
                "source_package": SOURCE_PACKAGE,
                "source_asset_sha256": fixture.model_sha256,
                "source_license_sha256": fixture.license_sha256,
                "source_package_digest": saved_art["package_digest"].clone(),
                "cooked_model_sha256": saved_art["cooked_sha256"].clone(),
                "source_hidden_during_reopen": source_hidden_during_reopen && source_hidden,
                "source_restored": fixture.source_restored
                    && fixture.source.is_dir()
                    && !fixture.hidden_source.exists(),
                "sidecar_bytes": sidecar_bytes.len(),
                "agent_status_after_play": "moving",
                "agent_position_after_play_raw": paused_position.map(orr_fp::FP::raw),
                "candidate_vs_baseline_pixels": changed_pixel_count(&initial_pixels, &candidate_pixels),
                "applied_vs_baseline_pixels": changed_pixel_count(&initial_pixels, &applied_pixels),
                "reopened_vs_applied_pixels": reopened_art_change_pixels,
                "moved_vs_reopened_pixels": moved_art_change_pixels,
                "removed_vs_applied_pixels": removed_art_change_pixels,
                "stale_revision_sidecar_recovered": !stale_sidecar.exists()
                    && changed_pixel_count(&applied_pixels, &recovered_pixels) < 20,
                "art_offset": [0.25, 0.5, 0.25],
                "art_scale": 0.5,
                "artifact_claim": "static cosmetic scene art; no mesh-navigation standalone exporter",
            }))
            .unwrap(),
        )
        .unwrap();
    }
}
