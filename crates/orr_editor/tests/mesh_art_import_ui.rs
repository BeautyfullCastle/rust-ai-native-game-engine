#![cfg(all(
    windows,
    target_arch = "x86_64",
    feature = "mesh-navigation",
    feature = "mesh-export"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::accesskit::Role;
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

const MODEL_ASSET: &str = "model.glb";
const LICENSE_ASSET: &str = "LICENSE.txt";
const PACKAGE_NAME: &str = "raw-agent-art";
const PACKAGE_VERSION: &str = "1.0.0";

struct Fixture {
    _temporary: tempfile::TempDir,
    physics_scene: PathBuf,
    physics_bytes: Vec<u8>,
    scene: PathBuf,
    raw_source: PathBuf,
    hidden_raw_source: PathBuf,
    package_output: PathBuf,
    hidden_package_output: PathBuf,
    bundle_output: PathBuf,
    runtime: PathBuf,
    capture_directory: Option<PathBuf>,
    model_bytes: Vec<u8>,
    license_bytes: Vec<u8>,
    raw_hidden: bool,
    package_hidden: bool,
    sources_restored: bool,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let capture_directory =
            std::env::var_os("ORR_MESH_ART_IMPORT_CAPTURE_DIR").map(PathBuf::from);
        if let Some(directory) = capture_directory.as_ref() {
            assert!(
                directory.is_absolute(),
                "capture directory must be absolute"
            );
            fs::create_dir(directory).expect("capture directory must be new");
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
        let scene = std::env::var_os("ORR_MESH_ART_IMPORT_OUTPUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("mesh-art-import.scene.yaml"));
        assert!(scene.is_absolute(), "scene output must be absolute");
        assert!(scene.parent().is_some_and(Path::is_dir));
        assert!(scene.to_string_lossy().ends_with(".scene.yaml"));
        assert!(!scene.exists(), "scene output must be new");
        assert!(!sidecar_path(&scene).exists(), "sidecar output must be new");

        let raw_source = root.join("raw agent art inputs");
        let hidden_raw_source = root.join("raw agent art inputs.hidden");
        fs::create_dir(&raw_source).expect("raw input directory must be new");
        let source_fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/imported_scene_demo")
            .canonicalize()
            .expect("checked-in static GLB fixture exists");
        let model_bytes = fs::read(source_fixture.join("foreground.glb")).unwrap();
        let license_bytes = fs::read(source_fixture.join(LICENSE_ASSET)).unwrap();
        fs::write(raw_source.join(MODEL_ASSET), &model_bytes).unwrap();
        fs::write(raw_source.join(LICENSE_ASSET), &license_bytes).unwrap();
        assert_eq!(
            fs::read_dir(&raw_source).unwrap().count(),
            2,
            "raw input contains only the model and its license"
        );
        assert!(!raw_source.join("orr.package.json").exists());

        let package_output = std::env::var_os("ORR_MESH_ART_IMPORT_PACKAGE_OUTPUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("generated raw agent package – 世界"));
        assert!(package_output.is_absolute());
        assert!(package_output.parent().is_some_and(Path::is_dir));
        assert!(!package_output.exists(), "package destination must be new");
        let hidden_package_output = package_output.with_file_name(format!(
            "{}.hidden",
            package_output
                .file_name()
                .and_then(|name| name.to_str())
                .expect("package output has a Unicode file name")
        ));
        assert!(!hidden_package_output.exists());

        let bundle_output = std::env::var_os("ORR_MESH_ART_IMPORT_EXPORT_OUTPUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("mesh-art-import-bundle"));
        assert!(bundle_output.is_absolute());
        assert!(bundle_output.parent().is_some_and(Path::is_dir));
        assert!(!bundle_output.exists(), "bundle destination must be new");
        let runtime = std::env::var_os("ORR_MESH_EXPORT_RUNTIME")
            .map(PathBuf::from)
            .expect("opted-in acceptance requires the archived mesh runtime");
        assert!(runtime.is_absolute() && runtime.is_file());

        Self {
            _temporary: temporary,
            physics_scene,
            physics_bytes,
            scene,
            raw_source,
            hidden_raw_source,
            package_output,
            hidden_package_output,
            bundle_output,
            runtime,
            capture_directory,
            model_bytes,
            license_bytes,
            raw_hidden: false,
            package_hidden: false,
            sources_restored: false,
        }
    }

    fn harness(&self) -> Harness<'static, EditorApp> {
        let mut editor = Editor::start(&HostSpec::local_game(
            self.physics_scene.clone(),
            EditorGame::PhysGame,
        ))
        .expect("start the normal local Physics editor");
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

    fn hide_sources(&mut self) {
        fs::rename(&self.raw_source, &self.hidden_raw_source)
            .expect("hide raw GLB and license inputs");
        self.raw_hidden = true;
        fs::rename(&self.package_output, &self.hidden_package_output)
            .expect("hide generated package source");
        self.package_hidden = true;
        assert!(!self.raw_source.exists());
        assert!(!self.package_output.exists());
    }

    fn restore_sources(&mut self) -> Result<(), String> {
        if self.raw_hidden {
            if self.raw_source.exists() || !self.hidden_raw_source.is_dir() {
                return Err("raw inputs are not in the expected hidden state".into());
            }
            fs::rename(&self.hidden_raw_source, &self.raw_source)
                .map_err(|error| format!("restore raw inputs: {error}"))?;
            self.raw_hidden = false;
        }
        if self.package_hidden {
            if self.package_output.exists() || !self.hidden_package_output.is_dir() {
                return Err("generated package is not in the expected hidden state".into());
            }
            fs::rename(&self.hidden_package_output, &self.package_output)
                .map_err(|error| format!("restore generated package: {error}"))?;
            self.package_hidden = false;
        }
        if fs::read(self.raw_source.join(MODEL_ASSET)).map_err(|error| error.to_string())?
            != self.model_bytes
            || fs::read(self.raw_source.join(LICENSE_ASSET)).map_err(|error| error.to_string())?
                != self.license_bytes
        {
            return Err("restored raw source bytes differ from originals".into());
        }
        if file_inventory(&self.raw_source)
            != vec![LICENSE_ASSET.to_owned(), MODEL_ASSET.to_owned()]
        {
            return Err("restored raw input inventory differs from originals".into());
        }
        let expected_package_files =
            [LICENSE_ASSET, MODEL_ASSET, "orr.package.json"].map(str::to_owned);
        let mut package_files = fs::read_dir(&self.package_output)
            .map_err(|error| error.to_string())?
            .map(|entry| {
                entry
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        package_files.sort();
        if package_files != expected_package_files.to_vec() {
            return Err("restored package file inventory differs from generated package".into());
        }
        if fs::read(self.package_output.join(MODEL_ASSET)).map_err(|error| error.to_string())?
            != self.model_bytes
            || fs::read(self.package_output.join(LICENSE_ASSET))
                .map_err(|error| error.to_string())?
                != self.license_bytes
        {
            return Err("restored package source bytes differ from raw inputs".into());
        }
        self.sources_restored = true;
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if self.raw_hidden && !self.raw_source.exists() && self.hidden_raw_source.exists() {
            if let Err(error) = fs::rename(&self.hidden_raw_source, &self.raw_source) {
                eprintln!("Could not restore hidden raw agent-art inputs: {error}");
            }
        }
        if self.package_hidden
            && !self.package_output.exists()
            && self.hidden_package_output.exists()
        {
            if let Err(error) = fs::rename(&self.hidden_package_output, &self.package_output) {
                eprintln!("Could not restore hidden generated agent-art package: {error}");
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

fn preflight_gpu() -> String {
    let gpu = Wgpu::headless(WgpuOptions::default())
        .expect("mesh agent-art import acceptance requires a hardware GPU");
    assert!(!gpu.is_software(), "software renderer is not GPU evidence");
    let adapter = gpu.adapter_name().to_owned();
    eprintln!("Raw mesh agent-art import GPU preflight: {adapter}");
    adapter
}

fn settle(harness: &mut Harness<'_, EditorApp>) {
    for _ in 0..4 {
        harness.state_mut().editor.sync();
        harness.run_steps(2);
    }
}

fn click(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness.get_by_label(label).scroll_to_me();
    harness.run_steps(2);
    harness.get_by_label(label).click_accesskit();
    settle(harness);
}

fn click_fast(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness.get_by_label(label).click_accesskit();
    harness.run_steps(1);
    harness.state_mut().editor.sync();
}

fn click_button(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness
        .get_by_role_and_label(Role::Button, label)
        .scroll_to_me();
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::Button, label)
        .click_accesskit();
    settle(harness);
}

fn set_field(harness: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .focus();
    harness.run_steps(2);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    harness.run_steps(2);
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
            "timed out waiting for {operation}: {:?}",
            harness.state().editor.status()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

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

fn path_matches(actual: Option<&Path>, expected: &Path) -> bool {
    actual
        .and_then(|path| path.canonicalize().ok())
        .zip(expected.canonicalize().ok())
        .is_some_and(|(actual_path, expected_path)| actual_path == expected_path)
}

fn type_scene_path(harness: &mut Harness<'_, EditorApp>, scene: &Path, action: &str) {
    set_field(harness, "Scene path", &scene.display().to_string());
    click(harness, action);
}

fn sidecar_path(scene: &Path) -> PathBuf {
    let mut value = scene.as_os_str().to_os_string();
    value.push(".mesh-art.json");
    PathBuf::from(value)
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
        .expect("the production MeshNavigation viewport rendered");
    let size = viewport.gpu().target().size();
    assert_eq!(size, target, "GPU viewport dimensions must be stable");
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

fn capture(directory: &Path, name: &str, harness: &mut Harness<'_, EditorApp>) {
    let pixels = viewport_pixels(harness);
    let viewport = harness.state().viewport3d_gpu().unwrap();
    let size = viewport.gpu().target().size();
    let image =
        egui::ColorImage::from_rgba_unmultiplied([size.0 as usize, size.1 as usize], &pixels);
    orr_editor::app::write_png(&directory.join(name), &image).unwrap();
}

fn file_inventory(root: &Path) -> Vec<String> {
    let mut names = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn verify_bundle(
    fixture: &Fixture,
    scene_bytes: &[u8],
    sidecar_bytes: &[u8],
    expected_checksum: u64,
    expected_graph_revision: [u8; 32],
) -> serde_json::Value {
    let manifest_path = fixture.bundle_output.join("orr.mesh-bundle.json");
    let manifest_bytes = fs::read(&manifest_path).expect("export writes its bundle manifest");
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let runtime_bytes = fs::read(&fixture.runtime).unwrap();
    let scene_record = &manifest["scene"];
    let sidecar_record = &manifest["sidecar"];
    let runtime_record = &manifest["runtime"];
    let exported_scene = fixture
        .bundle_output
        .join(scene_record["name"].as_str().unwrap());
    let exported_sidecar = fixture
        .bundle_output
        .join(sidecar_record["name"].as_str().unwrap());
    let exported_runtime = fixture
        .bundle_output
        .join(runtime_record["name"].as_str().unwrap());
    assert_eq!(fs::read(&exported_scene).unwrap(), scene_bytes);
    assert_eq!(fs::read(&exported_sidecar).unwrap(), sidecar_bytes);
    assert_eq!(fs::read(&exported_runtime).unwrap(), runtime_bytes);
    for (record, bytes) in [
        (scene_record, scene_bytes),
        (sidecar_record, sidecar_bytes),
        (runtime_record, runtime_bytes.as_slice()),
    ] {
        assert_eq!(record["bytes"].as_u64(), Some(bytes.len() as u64));
        assert_eq!(record["sha256"].as_str(), Some(sha256_hex(bytes).as_str()));
    }
    assert_eq!(
        file_inventory(&fixture.bundle_output),
        vec![
            "orr.mesh-bundle.json".to_owned(),
            scene_record["name"].as_str().unwrap().to_owned(),
            sidecar_record["name"].as_str().unwrap().to_owned(),
            runtime_record["name"].as_str().unwrap().to_owned(),
        ]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
    );
    assert_eq!(manifest["checksum"].as_u64(), Some(expected_checksum));
    assert_eq!(
        manifest["graph_revision"],
        serde_json::json!(expected_graph_revision)
    );
    manifest
}

#[test]
fn raw_agent_glb_import_is_applied_saved_reopened_and_exported_in_the_editor() {
    let requested = [
        "ORR_REQUIRE_GPU",
        "ORR_MESH_ART_IMPORT_ACCEPTANCE",
        "ORR_MESH_ART_IMPORT_CAPTURE_DIR",
        "ORR_MESH_ART_IMPORT_OUTPUT",
        "ORR_MESH_ART_IMPORT_PACKAGE_OUTPUT",
        "ORR_MESH_ART_IMPORT_EXPORT_OUTPUT",
        "ORR_MESH_EXPORT_RUNTIME",
    ]
    .iter()
    .any(|name| std::env::var_os(name).is_some());
    if !requested {
        eprintln!("SKIP: opt into actual raw agent-art UI acceptance with ORR_MESH_ART_IMPORT_ACCEPTANCE=1");
        return;
    }
    assert_eq!(
        std::env::var("WGPU_BACKEND").unwrap_or_default(),
        "vulkan",
        "raw agent-art acceptance requires WGPU_BACKEND=vulkan"
    );
    for name in [
        "ORR_MESH_ART_IMPORT_CAPTURE_DIR",
        "ORR_MESH_ART_IMPORT_OUTPUT",
        "ORR_MESH_ART_IMPORT_PACKAGE_OUTPUT",
        "ORR_MESH_ART_IMPORT_EXPORT_OUTPUT",
        "ORR_MESH_EXPORT_RUNTIME",
    ] {
        assert!(
            std::env::var_os(name).is_some(),
            "acceptance requires {name}"
        );
    }
    let expected_adapter = preflight_gpu();
    let mut fixture = Fixture::new();
    let capture_directory = fixture.capture_directory.clone();
    let mut editor = Editor::start(&HostSpec::local_game(
        fixture.physics_scene.clone(),
        EditorGame::PhysGame,
    ))
    .expect("start the normal local Physics editor");
    editor.sync();
    let mut harness = Harness::builder()
        .with_size([1600.0, 1100.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu_setup(hardware_vulkan_setup())
        .build_eframe(move |creation_context| {
            EditorApp::new(editor, creation_context.wgpu_render_state.clone())
        });
    harness.run_steps(4);
    assert_eq!(harness.state().editor.game(), EditorGame::PhysGame);
    assert!(harness.state().has_gpu());

    menu_click(&mut harness, "File", "New mesh scene…");
    type_scene_path(&mut harness, &fixture.scene, "Create");
    wait_for(&mut harness, "new MeshNavigation scene", |app| {
        app.editor.game().is_mesh_navigation()
            && path_matches(app.editor.path().as_deref(), &fixture.scene)
    });
    assert_eq!(
        harness
            .state()
            .viewport3d_gpu()
            .unwrap()
            .gpu()
            .adapter_name(),
        expected_adapter
    );
    assert!(!sidecar_path(&fixture.scene).exists());
    if harness.query_by_label("Create ramp and bridge").is_none() {
        click(&mut harness, "Static-mesh navigation");
    }
    click(&mut harness, "Create ramp and bridge");
    click(&mut harness, "Place mesh agent");
    menu_click(&mut harness, "File", "Save Ctrl+S");
    let scene_bytes = fs::read(&fixture.scene).unwrap();
    let scene_checksum = harness.state().editor.checksum();
    let initial_view = mesh_view(&harness);
    let graph_revision = initial_view.graph.revision();
    let initial_pixels = viewport_pixels(&mut harness);

    if harness.query_by_label("Raw agent GLB").is_none() {
        click(&mut harness, "Import raw agent art");
    }
    set_field(
        &mut harness,
        "Raw agent GLB",
        &fixture.raw_source.join(MODEL_ASSET).display().to_string(),
    );
    set_field(
        &mut harness,
        "Raw agent license",
        &fixture.raw_source.join(LICENSE_ASSET).display().to_string(),
    );
    set_field(
        &mut harness,
        "New agent package directory",
        &fixture.package_output.display().to_string(),
    );
    set_field(&mut harness, "Agent package name", PACKAGE_NAME);
    set_field(&mut harness, "Agent package version", PACKAGE_VERSION);
    assert!(harness
        .get_by_label("Import agent GLB")
        .accesskit_node()
        .is_disabled());
    click(
        &mut harness,
        "I have reviewed rights to import this agent art",
    );
    assert!(!harness
        .get_by_label("Import agent GLB")
        .accesskit_node()
        .is_disabled());
    click(&mut harness, "Import agent GLB");
    wait_for_label(
        &mut harness,
        "raw agent-art importer result",
        "Imported agent art package:",
    );
    let expected_import_status = format!(
        "Imported agent art package: {} (model.glb/LICENSE.txt). Source SHA-256: {}; license SHA-256: {}. Not applied; use Apply agent art and Save agent art explicitly.",
        fixture.package_output.canonicalize().unwrap().display(),
        sha256_hex(&fixture.model_bytes),
        sha256_hex(&fixture.license_bytes)
    );
    assert!(harness.query_by_label(&expected_import_status).is_some());
    assert!(harness
        .query_by_label_contains("Not applied; use Apply agent art and Save agent art explicitly.")
        .is_some());
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    assert_eq!(mesh_view(&harness).graph.revision(), graph_revision);
    assert!(!sidecar_path(&fixture.scene).exists());
    let candidate_pixels = viewport_pixels(&mut harness);
    assert_eq!(changed_pixel_count(&initial_pixels, &candidate_pixels), 0);

    assert_eq!(
        file_inventory(&fixture.raw_source),
        vec![LICENSE_ASSET.to_owned(), MODEL_ASSET.to_owned()]
    );
    let package_files = file_inventory(&fixture.package_output);
    assert_eq!(
        package_files,
        vec![
            LICENSE_ASSET.to_owned(),
            MODEL_ASSET.to_owned(),
            "orr.package.json".to_owned()
        ]
    );
    assert_eq!(
        fs::read(fixture.package_output.join(MODEL_ASSET)).unwrap(),
        fixture.model_bytes
    );
    assert_eq!(
        fs::read(fixture.package_output.join(LICENSE_ASSET)).unwrap(),
        fixture.license_bytes
    );
    let manifest_bytes = fs::read(fixture.package_output.join("orr.package.json")).unwrap();
    let package_manifest_sha256 = sha256_hex(&manifest_bytes);
    let package_manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(package_manifest["name"], PACKAGE_NAME);
    assert_eq!(package_manifest["version"], PACKAGE_VERSION);
    assert!(package_manifest["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|capability| capability == "models"));
    assert!(!fixture
        .scene
        .parent()
        .unwrap()
        .join("orr.packages.lock.json")
        .exists());

    if harness.query_by_label("Agent art offset X").is_none() {
        click(&mut harness, "Mesh agent art");
    }
    for (label, value) in [
        ("Agent art offset X", "0.25"),
        ("Agent art offset Y", "0.5"),
        ("Agent art offset Z", "0.25"),
        ("Agent art scale", "0.5"),
    ] {
        set_field(&mut harness, label, value);
    }
    click(&mut harness, "Apply agent art");
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    assert_eq!(mesh_view(&harness).graph.revision(), graph_revision);
    let applied_pixels = viewport_pixels(&mut harness);
    assert!(changed_pixel_count(&initial_pixels, &applied_pixels) > 20);
    assert!(harness
        .get_by_label(LBL_PLAY)
        .accesskit_node()
        .is_disabled());
    assert!(!sidecar_path(&fixture.scene).exists());
    click(&mut harness, "Undo agent art");
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    let undone_pixels = viewport_pixels(&mut harness);
    assert_eq!(changed_pixel_count(&initial_pixels, &undone_pixels), 0);
    click(&mut harness, "Redo agent art");
    let redone_pixels = viewport_pixels(&mut harness);
    assert!(changed_pixel_count(&applied_pixels, &redone_pixels) < 20);
    click(&mut harness, "Save agent art");

    let sidecar = sidecar_path(&fixture.scene);
    let sidecar_bytes = fs::read(&sidecar).expect("explicit save writes the scene art sidecar");
    assert_eq!(fs::read(&fixture.scene).unwrap(), scene_bytes);
    assert_eq!(harness.state().editor.checksum(), scene_checksum);
    let saved_art: serde_json::Value = serde_json::from_slice(&sidecar_bytes).unwrap();
    assert_eq!(saved_art["manifest"]["name"], PACKAGE_NAME);
    assert_eq!(saved_art["manifest"]["version"], PACKAGE_VERSION);
    assert_eq!(saved_art["asset"], MODEL_ASSET);
    assert_eq!(saved_art["asset_sha256"], sha256_hex(&fixture.model_bytes));
    assert_eq!(saved_art["license"], LICENSE_ASSET);
    assert_eq!(
        saved_art["license_sha256"],
        sha256_hex(&fixture.license_bytes)
    );
    assert_eq!(
        saved_art["license_text"],
        String::from_utf8(fixture.license_bytes.clone()).unwrap()
    );
    assert!(saved_art["model"].is_object());
    assert!(saved_art["cooked_sha256"].as_str().is_some());
    assert_eq!(
        saved_art["graph_revision"],
        serde_json::json!(graph_revision)
    );
    assert_eq!(saved_art["offset"], serde_json::json!([0.25, 0.5, 0.25]));
    assert_eq!(saved_art["scale"], serde_json::json!(0.5));
    if let Some(directory) = capture_directory.as_deref() {
        capture(directory, "saved-agent-art-import.png", &mut harness);
    }
    drop(harness);

    fixture.hide_sources();
    let source_hidden_during_reopen = true;
    let mut harness = fixture.harness();
    harness.run_steps(4);
    assert_eq!(harness.state().editor.game(), EditorGame::PhysGame);
    menu_click(&mut harness, "File", "Open mesh scene…");
    type_scene_path(&mut harness, &fixture.scene, "OK");
    wait_for(&mut harness, "fresh Physics editor mesh admission", |app| {
        app.editor.game().is_mesh_navigation()
            && path_matches(app.editor.path().as_deref(), &fixture.scene)
            && app.editor.checksum() == scene_checksum
    });
    assert_eq!(mesh_view(&harness).graph.revision(), graph_revision);
    assert_eq!(fs::read(&fixture.scene).unwrap(), scene_bytes);
    assert_eq!(fs::read(&sidecar).unwrap(), sidecar_bytes);
    assert!(!fixture.raw_source.exists());
    assert!(!fixture.package_output.exists());
    let reopened_pixels = viewport_pixels(&mut harness);
    assert!(changed_pixel_count(&applied_pixels, &reopened_pixels) < 20);
    if let Some(directory) = capture_directory.as_deref() {
        capture(
            directory,
            "reopened-source-hidden-agent-art.png",
            &mut harness,
        );
    }

    click_fast(&mut harness, LBL_PLAY);
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
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(harness
        .state()
        .editor
        .timeline()
        .is_some_and(|timeline| timeline.playing));
    click_fast(&mut harness, LBL_PAUSE);
    assert!(!harness.state().editor.timeline().unwrap().playing);
    harness.state_mut().editor.seek(0);
    settle(&mut harness);
    let start_position = mesh_view(&harness).agent.position;
    for _ in 0..16 {
        click_fast(&mut harness, LBL_STEP);
    }
    let stepped = mesh_view(&harness);
    assert_eq!(stepped.agent.status, NavigationStatus::Moving);
    assert_eq!(harness.state().editor.timeline().unwrap().tick, 16);
    assert_ne!(stepped.agent.position, start_position);
    let stepped_pixels = viewport_pixels(&mut harness);
    assert!(changed_pixel_count(&reopened_pixels, &stepped_pixels) > 10);
    if let Some(directory) = capture_directory.as_deref() {
        capture(
            directory,
            "source-hidden-agent-art-play16.png",
            &mut harness,
        );
    }
    click(&mut harness, LBL_STOP);
    assert_eq!(harness.state().editor.mode(), Mode::Edit);
    assert_eq!(harness.state().editor.checksum(), scene_checksum);

    menu_click(&mut harness, "File", "Export mesh game…");
    set_field(
        &mut harness,
        "Mesh player executable",
        &fixture.runtime.display().to_string(),
    );
    set_field(
        &mut harness,
        "Mesh game output directory",
        &fixture.bundle_output.display().to_string(),
    );
    assert!(harness
        .get_by_role_and_label(Role::Button, "Export mesh game")
        .accesskit_node()
        .is_disabled());
    click(&mut harness, "I trust this local mesh player executable");
    assert!(!harness
        .get_by_role_and_label(Role::Button, "Export mesh game")
        .accesskit_node()
        .is_disabled());
    click_button(&mut harness, "Export mesh game");
    wait_for_label(
        &mut harness,
        "same-editor mesh bundle export",
        "Mesh export complete:",
    );
    let final_scene_bytes = fs::read(&fixture.scene).unwrap();
    let final_sidecar_bytes = fs::read(&sidecar).unwrap();
    assert_eq!(final_scene_bytes, scene_bytes);
    assert_eq!(final_sidecar_bytes, sidecar_bytes);
    let export_manifest = verify_bundle(
        &fixture,
        &final_scene_bytes,
        &final_sidecar_bytes,
        scene_checksum,
        graph_revision,
    );
    let export_manifest_bytes =
        fs::read(fixture.bundle_output.join("orr.mesh-bundle.json")).unwrap();

    fixture
        .restore_sources()
        .expect("restore raw inputs and generated package source byte-for-byte");
    assert_eq!(
        sha256_hex(&fs::read(fixture.package_output.join("orr.package.json")).unwrap()),
        package_manifest_sha256,
        "restored generated package manifest hash matches the original"
    );
    assert_eq!(
        fs::read(&fixture.physics_scene).unwrap(),
        fixture.physics_bytes
    );
    if let Some(directory) = capture_directory.as_deref() {
        let receipt = serde_json::json!({
            "artifact_root": fixture.raw_source.parent().unwrap().display().to_string(),
            "scene_path": fixture.scene.display().to_string(),
            "sidecar_path": sidecar.display().to_string(),
            "raw_source_directory": fixture.raw_source.display().to_string(),
            "raw_source_hidden_path": fixture.hidden_raw_source.display().to_string(),
            "package_source_directory": fixture.package_output.display().to_string(),
            "package_source_hidden_path": fixture.hidden_package_output.display().to_string(),
            "bundle_path": fixture.bundle_output.display().to_string(),
            "runtime_path": fixture.runtime.display().to_string(),
            "scene_sha256": sha256_hex(&final_scene_bytes),
            "sidecar_sha256": sha256_hex(&final_sidecar_bytes),
            "raw_model_sha256": sha256_hex(&fixture.model_bytes),
            "raw_license_sha256": sha256_hex(&fixture.license_bytes),
            "package_manifest_sha256": package_manifest_sha256,
            "bundle_manifest_sha256": sha256_hex(&export_manifest_bytes),
            "runtime_sha256": export_manifest["runtime"]["sha256"],
            "bundle_checksum": export_manifest["checksum"],
            "source_hidden_during_reopen": source_hidden_during_reopen,
            "sources_restored": fixture.sources_restored,
            "game": "MeshNavigation",
            "ticks": 16,
            "viewport_changed_pixels_after_reopen": changed_pixel_count(&applied_pixels, &reopened_pixels),
            "viewport_changed_pixels_after_16_steps": changed_pixel_count(&reopened_pixels, &stepped_pixels),
            "bundle_contains_admitted_sidecar": true,
            "player_executed": false,
            "physical_os_input": false,
            "gpu_adapter": expected_adapter,
        });
        fs::write(
            directory.join("acceptance.json"),
            serde_json::to_vec_pretty(&receipt).unwrap(),
        )
        .unwrap();
    }
}
