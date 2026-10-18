#![cfg(all(
    windows,
    target_arch = "x86_64",
    feature = "mesh-navigation",
    feature = "mesh-mission",
    feature = "mesh-export"
))]
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
use orr_remote::mesh_navigation::{mesh_from_view, mission};
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

const MODEL_ASSET: &str = "foreground.glb";
const LICENSE_ASSET: &str = "LICENSE.txt";

struct Fixture {
    root: PathBuf,
    physics_scene: PathBuf,
    source_scene: PathBuf,
    target_scene: PathBuf,
    package: PathBuf,
    hidden_package: PathBuf,
    model_sha256: String,
    license_sha256: String,
    prefab: PathBuf,
    hidden_prefab: PathBuf,
    export: PathBuf,
    runtime: PathBuf,
    package_hidden: bool,
    prefab_hidden: bool,
}

impl Fixture {
    fn new() -> Self {
        assert_eq!(
            std::env::var("ORR_MESH_PROP_PREFAB_ACCEPTANCE").as_deref(),
            Ok("1")
        );
        assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
        assert_eq!(std::env::var("WGPU_BACKEND").as_deref(), Ok("vulkan"));
        let root = PathBuf::from(
            std::env::var_os("ORR_MESH_PROP_PREFAB_CAPTURE_DIR")
                .expect("set a fresh absolute prefab acceptance root"),
        );
        assert!(root.is_absolute());
        fs::create_dir(&root).expect("capture root must be fresh");
        let root = root.canonicalize().unwrap();
        fs::create_dir(root.join("captures")).unwrap();
        let physics_scene = root.join("owned-stock-physics.scene.yaml");
        fs::copy(EditorGame::PhysGame.default_scene_path(), &physics_scene).unwrap();
        let source_scene = root.join("source-mesh.scene.yaml");
        let target_scene = root.join("target-mesh.scene.yaml");
        let package = root.join("licensed static package");
        let hidden_package = root.join("licensed static package.hidden");
        fs::create_dir(&package).unwrap();
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/imported_scene_demo");
        for filename in [
            "orr.package.json",
            MODEL_ASSET,
            "background.glb",
            LICENSE_ASSET,
            "generate.py",
        ] {
            fs::copy(assets.join(filename), package.join(filename)).unwrap();
        }
        let model_sha256 = sha256(&fs::read(package.join(MODEL_ASSET)).unwrap());
        let license_sha256 = sha256(&fs::read(package.join(LICENSE_ASSET)).unwrap());
        let prefab = root.join("bridge.mesh-prop-prefab.json");
        let hidden_prefab = root.join("bridge.mesh-prop-prefab.json.hidden");
        let export = PathBuf::from(
            std::env::var_os("ORR_MESH_PROP_PREFAB_EXPORT_OUTPUT")
                .expect("set an absent export output directory"),
        );
        assert!(export.is_absolute() && !export.exists());
        assert_eq!(
            export.parent().unwrap().canonicalize().unwrap(),
            root.parent().unwrap()
        );
        let runtime = PathBuf::from(
            std::env::var_os("ORR_MESH_EXPORT_RUNTIME")
                .expect("set the matching archived mesh player"),
        );
        assert!(runtime.is_absolute() && runtime.is_file());
        Self {
            root,
            physics_scene,
            source_scene,
            target_scene,
            package,
            hidden_package,
            model_sha256,
            license_sha256,
            prefab,
            hidden_prefab,
            export,
            runtime,
            package_hidden: false,
            prefab_hidden: false,
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

    fn hide_inputs(&mut self) -> Result<(), String> {
        fs::rename(&self.package, &self.hidden_package)
            .map_err(|error| format!("hide source package: {error}"))?;
        self.package_hidden = true;
        fs::rename(&self.prefab, &self.hidden_prefab)
            .map_err(|error| format!("hide prefab asset: {error}"))?;
        self.prefab_hidden = true;
        Ok(())
    }

    fn restore_inputs(&mut self) -> Result<(), String> {
        if self.prefab_hidden {
            fs::rename(&self.hidden_prefab, &self.prefab)
                .map_err(|error| format!("restore prefab asset: {error}"))?;
            self.prefab_hidden = false;
        }
        if self.package_hidden {
            fs::rename(&self.hidden_package, &self.package)
                .map_err(|error| format!("restore package: {error}"))?;
            self.package_hidden = false;
        }
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.restore_inputs();
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

fn assert_hardware_gpu() -> String {
    let rhi = Wgpu::headless(WgpuOptions::default()).expect("hardware Vulkan GPU is required");
    assert!(
        !rhi.is_software(),
        "software rendering is not GPU acceptance evidence"
    );
    rhi.adapter_name().to_owned()
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
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    settle(harness);
}

fn menu_click(harness: &mut Harness<'_, EditorApp>, menu: &str, item: &str) {
    if harness.query_by_label(item).is_none() {
        click(harness, menu);
    }
    click(harness, item);
}

fn wait_for(
    harness: &mut Harness<'_, EditorApp>,
    operation: &str,
    ready: impl Fn(&EditorApp) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        settle(harness);
        if ready(harness.state()) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {operation}");
}

fn wait_for_label(harness: &mut Harness<'_, EditorApp>, label: &str) {
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        settle(harness);
        if harness.query_by_label_contains(label).is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for UI status containing {label:?}");
}

fn create_mesh_game(harness: &mut Harness<'_, EditorApp>, scene: &Path, width: &str, depth: &str) {
    menu_click(harness, "File", "New mesh game…");
    type_field(
        harness,
        "New mesh game scene path",
        &scene.display().to_string(),
    );
    type_field(harness, "Game floor width", width);
    type_field(harness, "Game floor depth", depth);
    type_field(harness, "Game collectible count", "3");
    type_field(harness, "Game distance per tick", "0.25");
    click_button(harness, "Create mesh game");
    wait_for(harness, "mesh game creation", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && app.editor.path().as_deref().is_some_and(|actual| {
                actual == scene || actual.canonicalize().ok() == scene.canonicalize().ok()
            })
            && scene.is_file()
            && mission::from_view(app.editor.snapshot().unwrap().predicted())
                .ok()
                .flatten()
                .is_some()
    });
}

fn mesh_view(harness: &Harness<'_, EditorApp>) -> orr_remote::mesh_navigation::MeshView {
    mesh_from_view(
        harness
            .state()
            .editor
            .snapshot()
            .expect("editor has an admitted frame")
            .predicted(),
    )
    .expect("frame contains an admitted mesh graph and scene pin")
}

fn mission_won(app: &EditorApp) -> bool {
    mission::from_view(app.editor.snapshot().expect("editor frame").predicted())
        .unwrap()
        .is_some_and(|mission| mission.run.won == 1)
}

fn viewport_pixels(harness: &mut Harness<'_, EditorApp>, filename: &str) -> Vec<u8> {
    let target_size = (900, 850);
    for _ in 0..6 {
        settle(harness);
        let actual = harness
            .state()
            .viewport3d_gpu()
            .unwrap()
            .gpu()
            .target()
            .size();
        if actual == target_size {
            break;
        }
        let window = harness.ctx.viewport_rect().size();
        let pixels_per_point = harness.ctx.pixels_per_point();
        harness.set_size(egui::vec2(
            window.x + (target_size.0 as f32 - actual.0 as f32) / pixels_per_point,
            window.y + (target_size.1 as f32 - actual.1 as f32) / pixels_per_point,
        ));
    }
    harness.render().expect("render production mesh viewport");
    let viewport = harness
        .state()
        .viewport3d_gpu()
        .expect("EditorApp owns the production GPU viewport");
    let size = viewport.gpu().target().size();
    assert_eq!(size, target_size);
    let pixels = viewport.gpu().read_rgba8();
    let image =
        egui::ColorImage::from_rgba_unmultiplied([size.0 as usize, size.1 as usize], &pixels);
    let output = PathBuf::from(std::env::var_os("ORR_MESH_PROP_PREFAB_CAPTURE_DIR").unwrap())
        .join("captures")
        .join(filename);
    orr_editor::app::write_png(&output, &image).unwrap();
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

fn sidecar_path(scene: &Path, suffix: &str) -> PathBuf {
    let mut path = scene.as_os_str().to_os_string();
    path.push(suffix);
    PathBuf::from(path)
}

fn standard_local_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    value
        .strip_prefix(r"\\?\")
        .unwrap_or(value.as_ref())
        .to_owned()
}

fn capture_record(root: &Path) -> Vec<Value> {
    let mut records = fs::read_dir(root.join("captures"))
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = fs::read(&path).unwrap();
            serde_json::json!({
                "name": path.file_name().unwrap().to_string_lossy(),
                "bytes": bytes.len(),
                "sha256": sha256(&bytes),
            })
        })
        .collect::<Vec<_>>();
    records.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
    records
}

#[test]
#[ignore = "requires ORR_MESH_PROP_PREFAB_ACCEPTANCE=1 and hardware Vulkan"]
fn prop_prefab_rebinds_across_mesh_graphs_and_exports_source_free() {
    let adapter = assert_hardware_gpu();
    let mut fixture = Fixture::new();
    let mut harness = fixture.harness();
    harness.run_steps(4);
    assert!(harness.state().has_gpu());

    create_mesh_game(&mut harness, &fixture.source_scene, "8", "6");
    let source_revision = mesh_view(&harness).graph.revision();
    let source_checksum = harness.state().editor.checksum();
    if harness
        .query_by_label("Prop package source directory")
        .is_none()
    {
        click(&mut harness, "Mesh props");
    }
    for (label, value) in [
        (
            "Prop package source directory",
            fixture.package.display().to_string(),
        ),
        ("Prop model asset", MODEL_ASSET.to_owned()),
        ("Prop license asset", LICENSE_ASSET.to_owned()),
    ] {
        type_field(&mut harness, label, &value);
    }
    click(&mut harness, "I have reviewed rights to use these props");
    click_button(&mut harness, "Load prop candidate");
    wait_for_label(
        &mut harness,
        "Loaded prop candidate; add or update a prop explicitly.",
    );
    for (label, value) in [
        ("Prop name", "Bridge prefab source"),
        ("Prop position X", "2.25"),
        ("Prop position Y", "0.25"),
        ("Prop position Z", "1.5"),
        ("Prop scale", "0.5"),
        ("Prop rotation X (degrees)", "17"),
        ("Prop rotation Y (degrees)", "31"),
        ("Prop rotation Z (degrees)", "53"),
        ("Prop tint R", "0.75"),
        ("Prop tint G", "0.6"),
        ("Prop tint B", "0.85"),
    ] {
        type_field(&mut harness, label, value);
    }
    if harness.query_by_label("Material slot index").is_none() {
        click(&mut harness, "Per-material-slot overrides");
    }
    type_field(&mut harness, "Material slot tint R", "0.65");
    choose_combo(&mut harness, "Material slot wrap S", "Mirror");
    click_button(&mut harness, "Stage material overrides");
    click_button(&mut harness, "Add prop");
    click_button(&mut harness, "Save props");
    let source_props_path = sidecar_path(&fixture.source_scene, ".mesh-props.json");
    let source_props_bytes = fs::read(&source_props_path).expect("source prop was saved");
    let source_props: Value = serde_json::from_slice(&source_props_bytes).unwrap();
    let source_prop = &source_props["props"][0];
    assert_eq!(source_props["props"].as_array().unwrap().len(), 1);
    assert_eq!(
        source_props["graph_revision"],
        serde_json::json!(source_revision)
    );
    assert_eq!(source_prop["name"], "Bridge prefab source");
    assert_eq!(source_prop["art"]["asset"], MODEL_ASSET);
    assert_eq!(source_prop["art"]["license"], LICENSE_ASSET);
    assert_eq!(source_prop["art"]["asset_sha256"], fixture.model_sha256);
    assert_eq!(source_prop["art"]["license_sha256"], fixture.license_sha256);
    assert_eq!(
        source_prop["art"]["tint"],
        serde_json::json!([0.75, 0.6, 0.85, 1.0])
    );
    assert_eq!(
        source_prop["art"]["material_slots"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(source_prop["art"]["material_slots"][0]["material"], 0);
    assert_eq!(
        source_prop["art"]["material_slots"][0]["tint"],
        serde_json::json!([0.65, 1.0, 1.0, 1.0])
    );
    assert_eq!(source_prop["art"]["material_slots"][0]["wrap_s"], "mirror");
    assert_eq!(
        source_prop["art"]["offset"],
        serde_json::json!([2.25, 0.25, 1.5])
    );
    assert_eq!(source_prop["art"]["scale"], serde_json::json!(0.5));
    assert_eq!(harness.state().editor.checksum(), source_checksum);
    let source_pixels = viewport_pixels(&mut harness, "01-source-applied-prop.png");

    if harness.query_by_label("Prop prefab file").is_none() {
        click(&mut harness, "Prop prefab");
    }
    type_field(
        &mut harness,
        "Prop prefab file",
        &standard_local_path(&fixture.prefab),
    );
    click_button(&mut harness, "Save selected prop prefab");
    wait_for_label(&mut harness, "Saved prop prefab asset. SHA-256:");
    assert!(fixture.prefab.is_file());
    let prefab_bytes = fs::read(&fixture.prefab).unwrap();
    let prefab_document: Value = serde_json::from_slice(&prefab_bytes).unwrap();
    assert_eq!(prefab_document["schema"], 1);
    assert_eq!(prefab_document["name"], "Bridge prefab source");
    assert_eq!(
        prefab_document["graph_revision"],
        serde_json::json!(source_revision)
    );
    let prefab_art: Value =
        serde_json::from_str(prefab_document["art_json"].as_str().unwrap()).unwrap();
    assert_eq!(
        prefab_art["asset_sha256"],
        source_prop["art"]["asset_sha256"]
    );
    assert_eq!(
        prefab_art["license_sha256"],
        source_prop["art"]["license_sha256"]
    );
    assert_eq!(prefab_art["asset_sha256"], fixture.model_sha256);
    assert_eq!(prefab_art["license_sha256"], fixture.license_sha256);
    assert_eq!(prefab_art["tint"], source_prop["art"]["tint"]);
    assert_eq!(
        prefab_art["material_slots"],
        source_prop["art"]["material_slots"]
    );

    create_mesh_game(&mut harness, &fixture.target_scene, "12", "8");
    let target_revision = mesh_view(&harness).graph.revision();
    assert_ne!(
        source_revision, target_revision,
        "prefab must cross distinct graph identities"
    );
    let target_checksum = harness.state().editor.checksum();
    let target_history = harness.state().editor.history().entries.len();
    let target_props_path = sidecar_path(&fixture.target_scene, ".mesh-props.json");
    assert!(!target_props_path.exists());
    if harness
        .query_by_label("Prop package source directory")
        .is_none()
    {
        click(&mut harness, "Mesh props");
    }
    if harness.query_by_label("Prop prefab file").is_none() {
        click(&mut harness, "Prop prefab");
    }
    type_field(
        &mut harness,
        "Prop prefab file",
        &standard_local_path(&fixture.prefab),
    );
    click(
        &mut harness,
        "I have reviewed rights to use this prop prefab",
    );
    click_button(&mut harness, "Load prop prefab");
    wait_for_label(
        &mut harness,
        "Loaded prop prefab candidate; no prop was added.",
    );
    assert_eq!(harness.state().editor.checksum(), target_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        target_history
    );
    assert!(
        !target_props_path.exists(),
        "loading is a candidate, not an Add"
    );
    assert!(harness.query_by_label("Props: 0/6").is_some());

    click_button(&mut harness, "Add prop");
    assert_eq!(harness.state().editor.checksum(), target_checksum);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        target_history
    );
    assert!(
        !target_props_path.exists(),
        "Add remains an explicit sidecar save"
    );
    assert!(harness.query_by_label("Props: 1/6").is_some());
    click_button(&mut harness, "Undo prop edit");
    assert!(harness.query_by_label("Props: 0/6").is_some());
    click_button(&mut harness, "Redo prop edit");
    assert!(harness.query_by_label("Props: 1/6").is_some());
    click_button(&mut harness, "Save props");
    let target_props_bytes = fs::read(&target_props_path).expect("target prop sidecar saved");
    let target_props: Value = serde_json::from_slice(&target_props_bytes).unwrap();
    let target_prop = &target_props["props"][0];
    assert_eq!(
        target_props["graph_revision"],
        serde_json::json!(target_revision)
    );
    assert_ne!(
        target_props["graph_revision"],
        prefab_document["graph_revision"]
    );
    assert_eq!(target_prop["name"], "Bridge prefab source");
    for property in [
        "asset",
        "asset_sha256",
        "cooked_sha256",
        "license",
        "license_sha256",
        "license_text",
        "manifest",
        "model",
        "offset",
        "scale",
        "rotation",
        "tint",
        "material_slots",
        "surface",
    ] {
        assert_eq!(
            target_prop["art"][property], prefab_art[property],
            "prefab load preserves authored {property}"
        );
    }
    assert_eq!(
        target_prop["art"]["graph_revision"],
        serde_json::json!(target_revision)
    );
    let mut rebound_art = target_prop["art"].clone();
    let mut saved_prefab_art = prefab_art.clone();
    for art in [&mut rebound_art, &mut saved_prefab_art] {
        art["scene_filename"] = Value::Null;
        art["graph_revision"] = Value::Null;
    }
    assert_eq!(
        rebound_art, saved_prefab_art,
        "prefab preserves all authored art data except target scene binding"
    );
    let target_pixels = viewport_pixels(&mut harness, "02-target-rebound-prop.png");
    assert!(changed_pixel_count(&source_pixels, &target_pixels) > 20);
    menu_click(&mut harness, "File", "Save Ctrl+S");
    let scene_bytes = fs::read(&fixture.target_scene).unwrap();
    let scene_checksum = harness.state().editor.checksum();
    drop(harness);

    fixture.hide_inputs().unwrap();
    let inputs_hidden_for_reopen = !fixture.package.exists() && !fixture.prefab.exists();
    assert!(inputs_hidden_for_reopen);
    let mut reopened = fixture.harness();
    reopened.run_steps(4);
    menu_click(&mut reopened, "File", "Open mesh scene…");
    type_field(
        &mut reopened,
        "Scene path",
        &fixture.target_scene.display().to_string(),
    );
    click_button(&mut reopened, "OK");
    wait_for(&mut reopened, "source-hidden target reopen", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && app.editor.path().as_deref().is_some_and(|path| {
                path == fixture.target_scene.as_path()
                    || path.canonicalize().ok() == fixture.target_scene.canonicalize().ok()
            })
            && app.editor.checksum() == scene_checksum
    });
    if reopened
        .query_by_label("Prop package source directory")
        .is_none()
    {
        click(&mut reopened, "Mesh props");
    }
    assert!(reopened.query_by_label("Props: 1/6").is_some());
    assert!(!fixture.package.exists());
    assert!(!fixture.prefab.exists());
    assert_eq!(fs::read(&fixture.target_scene).unwrap(), scene_bytes);
    assert_eq!(fs::read(&target_props_path).unwrap(), target_props_bytes);
    let reopened_prop = viewport_pixels(&mut reopened, "03-source-hidden-reopen.png");
    assert!(changed_pixel_count(&target_pixels, &reopened_prop) < 20);
    let reopened_view = mesh_view(&reopened);
    assert_eq!(reopened_view.graph.revision(), target_revision);
    let reopened_history = reopened.state().editor.history().entries.len();
    let reopened_props: Value =
        serde_json::from_slice(&fs::read(&target_props_path).unwrap()).unwrap();
    assert_eq!(
        reopened_props["props"][0]["art"]["graph_revision"],
        serde_json::json!(target_revision)
    );

    click_button(&mut reopened, LBL_PLAY);
    wait_for(&mut reopened, "mesh Play", |app| {
        app.editor
            .timeline()
            .is_some_and(|timeline| timeline.playing)
    });
    click_button(&mut reopened, LBL_PAUSE);
    if reopened.state().editor.timeline().unwrap().tick != 0 {
        click_button(&mut reopened, "⏮");
    }
    let mut win_tick = None;
    for _ in 0..2048 {
        if mission_won(reopened.state()) {
            win_tick = Some(reopened.state().editor.timeline().unwrap().tick);
            break;
        }
        click_button(&mut reopened, LBL_STEP);
    }
    let win_tick = win_tick.expect("source-free mission reaches all pickups and its exit");
    let won_view = mission::from_view(reopened.state().editor.snapshot().unwrap().predicted())
        .unwrap()
        .unwrap();
    assert_eq!(won_view.run.collected, (1 << won_view.config.count) - 1);
    assert_eq!(won_view.run.won, 1);
    assert!(matches!(
        mesh_view(&reopened).agent.status,
        NavigationStatus::Moving | NavigationStatus::Arrived
    ));
    let win_frame_checksum = reopened
        .state()
        .editor
        .snapshot()
        .unwrap()
        .predicted()
        .checksum();
    let win_pixels = viewport_pixels(&mut reopened, "04-mission-win.png");
    reopened.state_mut().editor.seek(0);
    wait_for(&mut reopened, "replay reset", |app| {
        app.editor
            .timeline()
            .is_some_and(|timeline| timeline.tick == 0 && !timeline.playing)
    });
    assert!(!mission_won(reopened.state()));
    reopened.state_mut().editor.seek(win_tick);
    wait_for(&mut reopened, "exact winning replay", |app| {
        app.editor
            .timeline()
            .is_some_and(|timeline| timeline.tick == win_tick)
            && mission_won(app)
    });
    assert_eq!(
        reopened
            .state()
            .editor
            .snapshot()
            .unwrap()
            .predicted()
            .checksum(),
        win_frame_checksum
    );
    let replay_pixels = viewport_pixels(&mut reopened, "05-mission-win-replay.png");
    assert!(changed_pixel_count(&win_pixels, &replay_pixels) < 20);
    click_button(&mut reopened, LBL_STOP);
    assert_eq!(reopened.state().editor.mode(), Mode::Edit);
    assert_eq!(reopened.state().editor.checksum(), scene_checksum);
    assert_eq!(
        reopened.state().editor.history().entries.len(),
        reopened_history
    );

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
    let scene_name = fixture.target_scene.file_name().unwrap().to_string_lossy();
    let expected = [
        scene_name.to_string(),
        format!("{scene_name}.mesh-props.json"),
        "orr.mesh-bundle.json".into(),
        "orr_mesh_player.exe".into(),
    ];
    let mut actual = fs::read_dir(&fixture.export)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    actual.sort();
    let mut expected = expected.to_vec();
    expected.sort();
    assert_eq!(actual, expected);
    let manifest_bytes = fs::read(fixture.export.join("orr.mesh-bundle.json")).unwrap();
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(manifest["props"]["sha256"], sha256(&target_props_bytes));
    assert_eq!(
        manifest["props"]["name"],
        format!("{scene_name}.mesh-props.json")
    );
    assert_eq!(
        fs::read(fixture.export.join(format!("{scene_name}.mesh-props.json"))).unwrap(),
        target_props_bytes
    );
    assert_eq!(
        fs::read(fixture.export.join(&*scene_name)).unwrap(),
        scene_bytes
    );
    fixture.restore_inputs().unwrap();
    assert_eq!(fs::read(&fixture.prefab).unwrap(), prefab_bytes);
    assert_eq!(
        sha256(&fs::read(fixture.package.join(MODEL_ASSET)).unwrap()),
        fixture.model_sha256
    );
    assert_eq!(
        sha256(&fs::read(fixture.package.join(LICENSE_ASSET)).unwrap()),
        fixture.license_sha256
    );
    let captures = capture_record(&fixture.root);
    assert_eq!(captures.len(), 5);
    let receipt = serde_json::json!({
        "artifact_root": fixture.root,
        "hardware_vulkan_adapter": adapter,
        "source_scene": fixture.source_scene,
        "source_graph_revision": source_revision.iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
        "source_prop_sidecar_sha256": sha256(&source_props_bytes),
        "prefab_path": fixture.prefab,
        "prefab_sha256": sha256(&prefab_bytes),
        "model_sha256": fixture.model_sha256,
        "license_sha256": fixture.license_sha256,
        "prefab_source_graph_revision": prefab_document["graph_revision"],
        "target_scene": fixture.target_scene,
        "target_scene_sha256": sha256(&scene_bytes),
        "target_graph_revision": target_revision.iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
        "target_scene_checksum": format!("0x{scene_checksum:016x}"),
        "source_and_target_graph_revisions_differ": source_revision != target_revision,
        "candidate_load_did_not_add": true,
        "explicit_add_undo_redo_save": true,
        "prefab_source_and_license_provenance_preserved": true,
        "prefab_transform_and_appearance_preserved": true,
        "source_package_and_prefab_hidden_during_fresh_reopen": inputs_hidden_for_reopen,
        "mission_won_after_source_free_reopen": true,
        "win_tick": win_tick,
        "win_frame_checksum": format!("0x{win_frame_checksum:016x}"),
        "exact_replay": true,
        "export_files": actual,
        "export_manifest_sha256": sha256(&manifest_bytes),
        "export_runtime_sha256": manifest["runtime"]["sha256"],
        "physical_os_input": false,
        "captures": captures,
    });
    fs::write(
        fixture.root.join("acceptance.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
}
