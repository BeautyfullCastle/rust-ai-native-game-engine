#![cfg(all(windows, target_arch = "x86_64", feature = "mesh-export"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::{accesskit::Role, Key, Modifiers};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use egui_wgpu::{wgpu, WgpuSetupCreateNew};
use orr_editor::{app::LBL_PLAY, game::EditorGame, Editor, EditorApp, HostSpec};
use orr_rhi::Rhi;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

const SOURCE_PACKAGE: &str = "sample-imported-scene";
const MODEL_ASSET: &str = "foreground.glb";
const LICENSE_ASSET: &str = "LICENSE.txt";
const MANIFEST_FILENAME: &str = "orr.mesh-bundle.json";
const RUNTIME_FILENAME: &str = "orr_mesh_player.exe";

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    physics_scene: PathBuf,
    scene: PathBuf,
    package_source: PathBuf,
    runtime: PathBuf,
    output: PathBuf,
    existing_output: PathBuf,
    capture_directory: PathBuf,
    source_scene_bytes: Vec<u8>,
    source_model_sha256: String,
    source_license_sha256: String,
    source_runtime_sha256: String,
    source_sidecar_sha256: Option<String>,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let capture_directory = PathBuf::from(
            std::env::var_os("ORR_MESH_EXPORT_CAPTURE_DIR")
                .expect("GPU acceptance requires ORR_MESH_EXPORT_CAPTURE_DIR"),
        );
        assert!(capture_directory.is_absolute());
        fs::create_dir(&capture_directory).expect("capture directory must be fresh");
        let root = capture_directory.canonicalize().unwrap();
        let physics_scene = root.join("owned stock Physics.scene.yaml");
        fs::copy(EditorGame::PhysGame.default_scene_path(), &physics_scene)
            .expect("copy the stock Physics scene into the owned fixture");
        let scene = root.join("mesh-export.scene.yaml");
        let requested_output = PathBuf::from(
            std::env::var_os("ORR_MESH_EXPORT_OUTPUT")
                .expect("GPU acceptance requires ORR_MESH_EXPORT_OUTPUT"),
        );
        assert!(
            requested_output.is_absolute(),
            "bundle output must be absolute"
        );
        assert!(requested_output.parent().is_some_and(Path::is_dir));
        assert!(!requested_output.exists(), "bundle output must be fresh");
        let output_leaf = requested_output.file_name().unwrap();
        let output = requested_output
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap()
            .join(output_leaf);
        let runtime = PathBuf::from(
            std::env::var_os("ORR_MESH_EXPORT_RUNTIME")
                .expect("GPU acceptance requires ORR_MESH_EXPORT_RUNTIME"),
        );
        assert!(runtime.is_absolute(), "player runtime must be absolute");
        assert!(runtime.is_file(), "configured player runtime must exist");
        assert_ne!(
            output, root,
            "bundle output must not replace the fixture root"
        );
        let existing_output = root.join("preexisting-bundle");
        fs::create_dir(&existing_output).unwrap();
        fs::write(
            existing_output.join("sentinel.txt"),
            b"preserve existing bundle",
        )
        .unwrap();
        let package_source = root.join("local CC0 mesh art package");
        fs::create_dir(&package_source).unwrap();
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/imported_scene_demo");
        for file_name in [
            "orr.package.json",
            "foreground.glb",
            "background.glb",
            "LICENSE.txt",
            "generate.py",
        ] {
            fs::copy(assets.join(file_name), package_source.join(file_name)).unwrap();
        }
        let source_model_sha256 = sha256_hex(&fs::read(package_source.join(MODEL_ASSET)).unwrap());
        let source_license_sha256 =
            sha256_hex(&fs::read(package_source.join(LICENSE_ASSET)).unwrap());
        let source_runtime_sha256 = sha256_hex(&fs::read(&runtime).unwrap());
        Self {
            _temporary: temporary,
            root,
            physics_scene,
            scene,
            package_source,
            runtime,
            output,
            existing_output,
            capture_directory,
            source_scene_bytes: Vec::new(),
            source_model_sha256,
            source_license_sha256,
            source_runtime_sha256,
            source_sidecar_sha256: None,
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
            .with_size([1500.0, 1050.0])
            .with_step_dt(1.0 / 60.0)
            .wgpu_setup(hardware_vulkan_setup())
            .build_eframe(move |creation_context| {
                EditorApp::new(editor, creation_context.wgpu_render_state.clone())
            })
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

fn require_hardware_gpu() {
    match orr_rhi::Wgpu::headless(orr_rhi::WgpuOptions::default()) {
        Ok(rhi) => {
            eprintln!(
                "Mesh export GPU preflight: {} (software: {})",
                rhi.adapter_name(),
                rhi.is_software()
            );
            assert!(
                !rhi.is_software(),
                "software rendering is not GPU acceptance"
            );
        }
        Err(error) => panic!("Mesh export GPU acceptance unavailable: {error}"),
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

fn wait_for_label(harness: &mut Harness<'_, EditorApp>, label: &str, reason: &str) {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        settle(harness);
        if harness.query_by_label(label).is_some() {
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

fn viewport_pixels(harness: &mut Harness<'_, EditorApp>) -> Vec<u8> {
    let target = (900, 850);
    for _ in 0..6 {
        settle(harness);
        let actual = harness
            .state()
            .viewport3d_gpu()
            .expect("the production MeshNavigation viewport rendered")
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
        .expect("the MeshNavigation viewport remains on the active GPU");
    let size = viewport.gpu().target().size();
    assert_eq!(
        size, target,
        "actual GPU viewport dimensions must be stable"
    );
    let pixels = viewport.gpu().read_rgba8();
    assert_eq!(pixels.len(), (size.0 * size.1 * 4) as usize);
    assert!(pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0));
    pixels
}

fn capture(directory: &Path, filename: &str, harness: &mut Harness<'_, EditorApp>) {
    let pixels = viewport_pixels(harness);
    let viewport = harness.state().viewport3d_gpu().unwrap();
    let size = viewport.gpu().target().size();
    let image =
        egui::ColorImage::from_rgba_unmultiplied([size.0 as usize, size.1 as usize], &pixels);
    orr_editor::app::write_png(&directory.join(filename), &image).unwrap();
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn verify_bundle(fixture: &Fixture, graph_revision: &[u8; 32], checksum: u64) -> Value {
    let root = fixture.output.canonicalize().unwrap();
    let scene_name = fixture.scene.file_name().unwrap().to_str().unwrap();
    let sidecar_name = format!("{scene_name}.mesh-art.json");
    let bundle_scene = root.join(scene_name);
    let bundle_sidecar = root.join(&sidecar_name);
    let bundle_runtime = root.join(RUNTIME_FILENAME);
    let manifest_path = root.join(MANIFEST_FILENAME);
    let scene_bytes = fs::read(&bundle_scene).unwrap();
    let sidecar_bytes = fs::read(&bundle_sidecar).unwrap();
    let runtime_bytes = fs::read(&bundle_runtime).unwrap();
    let manifest_bytes = fs::read(&manifest_path).unwrap();
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let scene_hash = sha256_hex(&scene_bytes);
    let sidecar_hash = sha256_hex(&sidecar_bytes);
    let runtime_hash = sha256_hex(&runtime_bytes);
    assert_eq!(scene_bytes, fixture.source_scene_bytes);
    assert_eq!(runtime_bytes, fs::read(&fixture.runtime).unwrap());
    assert_eq!(manifest["schema"], 1);
    assert_eq!(manifest["checksum"], checksum);
    assert_eq!(
        manifest["graph_revision"],
        serde_json::json!(graph_revision)
    );
    assert_eq!(manifest["scene"]["name"], scene_name);
    assert_eq!(manifest["scene"]["bytes"], scene_bytes.len() as u64);
    assert_eq!(manifest["scene"]["sha256"], scene_hash);
    assert_eq!(manifest["sidecar"]["name"], sidecar_name);
    assert_eq!(manifest["sidecar"]["bytes"], sidecar_bytes.len() as u64);
    assert_eq!(manifest["sidecar"]["sha256"], sidecar_hash);
    assert_eq!(manifest["runtime"]["name"], RUNTIME_FILENAME);
    assert_eq!(manifest["runtime"]["bytes"], runtime_bytes.len() as u64);
    assert_eq!(manifest["runtime"]["sha256"], runtime_hash);
    assert_eq!(runtime_hash, fixture.source_runtime_sha256);
    let inventory: BTreeSet<String> = fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(
        inventory,
        [
            MANIFEST_FILENAME.to_owned(),
            scene_name.to_owned(),
            sidecar_name.clone(),
            RUNTIME_FILENAME.to_owned(),
        ]
        .into_iter()
        .collect()
    );
    let saved_art: Value = serde_json::from_slice(&sidecar_bytes).unwrap();
    assert_eq!(saved_art["manifest"]["name"], SOURCE_PACKAGE);
    assert_eq!(saved_art["asset"], MODEL_ASSET);
    assert_eq!(saved_art["asset_sha256"], fixture.source_model_sha256);
    assert_eq!(saved_art["license"], LICENSE_ASSET);
    assert_eq!(saved_art["license_sha256"], fixture.source_license_sha256);
    assert!(saved_art["model"].is_object());
    assert!(saved_art["cooked_sha256"].as_str().is_some());
    assert_eq!(
        fixture.source_sidecar_sha256.as_deref(),
        Some(sidecar_hash.as_str())
    );
    serde_json::json!({
        "bundle_directory": root,
        "bundle_scene": bundle_scene,
        "bundle_sidecar": bundle_sidecar,
        "bundle_runtime": bundle_runtime,
        "bundle_manifest": manifest_path,
        "bundle_scene_sha256": scene_hash,
        "bundle_sidecar_sha256": sidecar_hash,
        "bundle_runtime_sha256": runtime_hash,
        "bundle_manifest_sha256": sha256_hex(&manifest_bytes),
        "manifest": manifest,
        "sidecar": saved_art,
    })
}

#[test]
fn default_physics_to_mesh_scene_exports_a_verified_bundle_without_overwrites() {
    let requested = [
        "ORR_REQUIRE_GPU",
        "ORR_MESH_EXPORT_ACCEPTANCE",
        "ORR_MESH_EXPORT_RUNTIME",
        "ORR_MESH_EXPORT_OUTPUT",
        "ORR_MESH_EXPORT_CAPTURE_DIR",
    ]
    .iter()
    .any(|name| std::env::var_os(name).is_some());
    if !requested {
        eprintln!(
            "SKIP: opt into actual Mesh export GPU acceptance with ORR_MESH_EXPORT_ACCEPTANCE=1"
        );
        return;
    }
    assert!(
        std::env::var_os("ORR_MESH_EXPORT_ACCEPTANCE").is_some(),
        "set ORR_MESH_EXPORT_ACCEPTANCE=1 to run the acceptance flow"
    );
    assert_eq!(
        std::env::var("WGPU_BACKEND").unwrap_or_default(),
        "vulkan",
        "Mesh export acceptance requires WGPU_BACKEND=vulkan"
    );
    require_hardware_gpu();

    let mut fixture = Fixture::new();
    let mut harness = fixture.harness();
    harness.run_steps(4);
    assert_eq!(harness.state().editor.game(), EditorGame::PhysGame);
    assert!(harness.state().has_gpu());

    menu_click(&mut harness, "File", "New mesh scene…");
    type_scene_path(&mut harness, &fixture.scene, "Create");
    wait_for(&mut harness, "new MeshNavigation host", |app| {
        app.editor.game().is_mesh_navigation()
            && path_matches(app.editor.path().as_deref(), &fixture.scene)
    });
    assert!(harness.state().has_gpu());
    click(&mut harness, "Static-mesh navigation");
    click(&mut harness, "Create ramp and bridge");
    click(&mut harness, "Place mesh agent");
    menu_click(&mut harness, "File", "Save Ctrl+S");
    fixture.source_scene_bytes = fs::read(&fixture.scene).unwrap();
    let initial_checksum = harness.state().editor.checksum();
    let initial_view = orr_remote::mesh_navigation::mesh_from_view(
        harness.state().editor.snapshot().unwrap().predicted(),
    )
    .unwrap();
    let graph_revision = initial_view.graph.revision();
    let baseline_pixels = viewport_pixels(&mut harness);
    capture(
        &fixture.capture_directory,
        "mesh-agent-before-art.png",
        &mut harness,
    );

    if harness
        .query_by_label("Art package source directory")
        .is_none()
    {
        click(&mut harness, "Mesh agent art");
    }
    type_field(
        &mut harness,
        "Art package source directory",
        &fixture.package_source.display().to_string(),
    );
    type_field(&mut harness, "Art model asset", MODEL_ASSET);
    type_field(&mut harness, "Art license asset", LICENSE_ASSET);
    click(&mut harness, "I have reviewed rights to use this agent art");
    click(&mut harness, "Load agent art");
    assert!(harness
        .query_by_label(&format!(
            "Loaded static package art {SOURCE_PACKAGE}/{MODEL_ASSET}; not applied."
        ))
        .is_some());
    type_field(&mut harness, "Agent art offset X", "0.25");
    type_field(&mut harness, "Agent art offset Y", "0.5");
    type_field(&mut harness, "Agent art offset Z", "0.25");
    type_field(&mut harness, "Agent art scale", "0.5");
    click(&mut harness, "Apply agent art");
    assert!(harness
        .get_by_label(LBL_PLAY)
        .accesskit_node()
        .is_disabled());
    let applied_pixels = viewport_pixels(&mut harness);
    assert!(
        changed_pixel_count(&baseline_pixels, &applied_pixels) > 20,
        "applying local package art should alter the actual viewport"
    );
    capture(
        &fixture.capture_directory,
        "mesh-agent-art-applied.png",
        &mut harness,
    );

    menu_click(&mut harness, "File", "Export mesh game…");
    type_field(
        &mut harness,
        "Mesh player executable",
        &fixture.runtime.display().to_string(),
    );
    type_field(
        &mut harness,
        "Mesh game output directory",
        &fixture.output.display().to_string(),
    );
    click(&mut harness, "I trust this local mesh player executable");
    assert!(harness
        .get_by_role_and_label(Role::Button, "Export mesh game")
        .accesskit_node()
        .is_disabled());
    assert!(!fixture.output.exists());
    click(&mut harness, "Close window");
    click(&mut harness, "Save agent art");
    let sidecar = sidecar_path(&fixture.scene);
    let source_sidecar_bytes = fs::read(&sidecar).expect("saved art sidecar exists");
    fixture.source_sidecar_sha256 = Some(sha256_hex(&source_sidecar_bytes));
    assert_eq!(
        fs::read(&fixture.scene).unwrap(),
        fixture.source_scene_bytes
    );
    assert_eq!(harness.state().editor.checksum(), initial_checksum);

    type_field(&mut harness, "Distance per tick", "0.0625");
    click(&mut harness, "Place mesh agent");
    assert!(harness.state().editor.is_dirty());
    menu_click(&mut harness, "File", "Export mesh game…");
    type_field(
        &mut harness,
        "Mesh player executable",
        &fixture.runtime.display().to_string(),
    );
    type_field(
        &mut harness,
        "Mesh game output directory",
        &fixture.output.display().to_string(),
    );
    let dirty_scene_checksum = harness.state().editor.checksum();
    assert_ne!(dirty_scene_checksum, initial_checksum);
    let dirty_scene_blocker =
        "Save or explicitly undo/discard scene, unplaced mesh, agent art and prop changes before changing mesh workspaces";
    assert!(harness
        .get_by_role_and_label(Role::Button, "Export mesh game")
        .accesskit_node()
        .is_disabled());
    wait_for_label(
        &mut harness,
        dirty_scene_blocker,
        "visible dirty scene export blocker",
    );
    assert_eq!(harness.state().editor.checksum(), dirty_scene_checksum);
    assert!(!fixture.output.exists());
    assert_eq!(
        fs::read(&fixture.scene).unwrap(),
        fixture.source_scene_bytes
    );
    assert_eq!(fs::read(&sidecar).unwrap(), source_sidecar_bytes);
    click(&mut harness, "Close window");
    menu_click(&mut harness, "Edit", "Undo Ctrl+Z");
    assert!(!harness.state().editor.is_dirty());
    assert_eq!(harness.state().editor.checksum(), initial_checksum);

    menu_click(&mut harness, "File", "Export mesh game…");
    type_field(
        &mut harness,
        "Mesh player executable",
        &fixture.runtime.display().to_string(),
    );
    type_field(
        &mut harness,
        "Mesh game output directory",
        &fixture.existing_output.display().to_string(),
    );
    assert!(!harness
        .get_by_role_and_label(Role::Button, "Export mesh game")
        .accesskit_node()
        .is_disabled());
    click_button(&mut harness, "Export mesh game");
    wait_for_label(
        &mut harness,
        "Mesh export failed: mesh bundle destination already exists",
        "existing-destination export refusal",
    );
    assert_eq!(
        fs::read(fixture.existing_output.join("sentinel.txt")).unwrap(),
        b"preserve existing bundle"
    );
    assert_eq!(
        fs::read(&fixture.scene).unwrap(),
        fixture.source_scene_bytes
    );
    assert_eq!(fs::read(&sidecar).unwrap(), source_sidecar_bytes);

    type_field(
        &mut harness,
        "Mesh game output directory",
        &fixture.output.display().to_string(),
    );
    click_button(&mut harness, "Export mesh game");
    let expected_status = format!("Mesh export complete: {}", fixture.output.display());
    wait_for_label(
        &mut harness,
        &expected_status,
        "successful mesh bundle creation",
    );
    assert!(fixture.output.is_dir());
    let bundle = verify_bundle(&fixture, &graph_revision, initial_checksum);
    assert_eq!(
        fs::read(&fixture.scene).unwrap(),
        fixture.source_scene_bytes
    );
    assert_eq!(fs::read(&sidecar).unwrap(), source_sidecar_bytes);
    assert!(harness.state().editor.path().as_deref() == Some(fixture.scene.as_path()));
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    capture(
        &fixture.capture_directory,
        "mesh-export-complete.png",
        &mut harness,
    );

    let manifest_bytes = fs::read(fixture.output.join(MANIFEST_FILENAME)).unwrap();
    let sidecar_bytes = fs::read(&sidecar).unwrap();
    let runtime_bytes = fs::read(&fixture.runtime).unwrap();
    let receipt = serde_json::json!({
        "original_scene": fixture.scene,
        "project_root": fixture.root,
        "package_source": fixture.package_source,
        "runtime_source": fixture.runtime,
        "bundle_directory": bundle["bundle_directory"],
        "bundle_scene": bundle["bundle_scene"],
        "bundle_runtime": bundle["bundle_runtime"],
        "bundle_manifest": bundle["bundle_manifest"],
        "original_scene_sha256": sha256_hex(&fixture.source_scene_bytes),
        "bundle_scene_sha256": bundle["bundle_scene_sha256"],
        "source_art_sidecar_sha256": fixture.source_sidecar_sha256,
        "bundle_art_sidecar_sha256": bundle["bundle_sidecar_sha256"],
        "source_model_sha256": fixture.source_model_sha256,
        "source_license_sha256": fixture.source_license_sha256,
        "cooked_model_sha256": bundle["sidecar"]["cooked_sha256"],
        "source_runtime_sha256": fixture.source_runtime_sha256,
        "bundle_runtime_sha256": bundle["bundle_runtime_sha256"],
        "bundle_manifest_sha256": sha256_hex(&manifest_bytes),
        "graph_revision": graph_revision,
        "initial_checksum": initial_checksum,
        "dirty_art_refused_before_worker": true,
        "dirty_scene_refused_before_worker": true,
        "existing_destination_preserved": true,
        "source_scene_and_sidecar_unchanged": true,
        "art_visualized_in_actual_viewport": changed_pixel_count(&baseline_pixels, &applied_pixels) > 20,
        "source_model_bytes": fs::metadata(fixture.package_source.join(MODEL_ASSET)).unwrap().len(),
        "source_license_bytes": fs::metadata(fixture.package_source.join(LICENSE_ASSET)).unwrap().len(),
        "source_sidecar_bytes": sidecar_bytes.len(),
        "source_runtime_bytes": runtime_bytes.len(),
        "manifest": bundle["manifest"],
        "sidecar": bundle["sidecar"],
        "native_player_launched_by_test": false,
        "physical_os_input_used": false,
    });
    fs::write(
        fixture.capture_directory.join("acceptance.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
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
