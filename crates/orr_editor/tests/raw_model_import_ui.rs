#![cfg(all(
    windows,
    target_arch = "x86_64",
    feature = "windows-projects",
    feature = "room-animated-models"
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
    model_bindings::{ModelKind, PlaybackMode},
    Editor, EditorApp, HostSpec, Target,
};
use orr_package::{Lock, Project};
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use orr_sample::{
    room_game::{RoomActor, KEY, PLAYER},
    room_project::{compiled_runtime, PreparedProject},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

const STATIC_PACKAGE: &str = "raw-static-import";
const ANIMATED_PACKAGE: &str = "raw-animated-import";
const STATIC_ASSET: &str = "model.glb";
const ANIMATED_ASSET: &str = "model.glb";
const EXISTING_PACKAGE: &str = "sample-imported-scene";

struct Fixture {
    _temp: tempfile::TempDir,
    output: PathBuf,
    other: PathBuf,
    evidence_root: PathBuf,
    capture_root: Option<PathBuf>,
    raw_source: PathBuf,
    package_source: PathBuf,
    static_model: PathBuf,
    static_license: PathBuf,
    animated_model: PathBuf,
    animated_license: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let output = std::env::var_os("ORR_RAW_MODEL_IMPORT_OUTPUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| temp.path().join("raw model import Room project"));
        assert!(output.is_absolute(), "Room project output must be absolute");
        assert!(!output.exists(), "Room project output must be new");
        assert!(output.parent().is_some_and(Path::is_dir));

        let capture_root = std::env::var_os("ORR_RAW_MODEL_IMPORT_CAPTURE_DIR").map(PathBuf::from);
        if let Some(path) = &capture_root {
            assert!(path.is_absolute(), "capture directory must be absolute");
            fs::create_dir(path).expect("capture directory must be new");
        }
        let evidence_root = capture_root
            .as_ref()
            .map_or_else(|| temp.path().to_path_buf(), PathBuf::clone);
        let raw_source = evidence_root.join("raw source files");
        fs::create_dir(&raw_source).expect("raw source directory must be new");
        let static_input = raw_source.join("static input");
        let animated_input = raw_source.join("animated input");
        fs::create_dir(&static_input).unwrap();
        fs::create_dir(&animated_input).unwrap();
        let static_assets = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/imported_scene_demo")
            .canonicalize()
            .expect("repository static GLB fixture exists");
        let animated_assets = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/animation_demo")
            .canonicalize()
            .expect("repository animated GLB fixture exists");
        let static_model = static_input.join("model.glb");
        let static_license = static_input.join("LICENSE.txt");
        let animated_model = animated_input.join("model.glb");
        let animated_license = animated_input.join("LICENSE.txt");
        fs::copy(static_assets.join("foreground.glb"), &static_model).unwrap();
        fs::copy(static_assets.join("LICENSE.txt"), &static_license).unwrap();
        fs::copy(animated_assets.join("animated.glb"), &animated_model).unwrap();
        fs::copy(animated_assets.join("LICENSE.txt"), &animated_license).unwrap();
        assert!(!static_input.join("orr.package.json").exists());
        assert!(!animated_input.join("orr.package.json").exists());

        let package_source = evidence_root.join("generated package sources");
        fs::create_dir(&package_source).expect("package source parent must be new");
        let other = temp.path().join("other Room project");
        orr_sample::windows_project::create_room_project(&output, "raw-model-import-room")
            .expect("create initial admitted Room project");
        orr_sample::windows_project::create_room_project(&other, "raw-model-import-other")
            .expect("create second admitted Room project");

        Self {
            _temp: temp,
            output,
            other,
            evidence_root,
            capture_root,
            raw_source,
            package_source,
            static_model,
            static_license,
            animated_model,
            animated_license,
        }
    }

    fn app(&self) -> Harness<'static, EditorApp> {
        let mut prepared = PreparedProject::open(&self.output).unwrap();
        let camera = prepared
            .take_camera()
            .expect("admitted Room project declares its camera");
        let (_, path, scene, models) = prepared.into_parts();
        let mut editor = Editor::start(&HostSpec::PreparedRoom {
            scene: path,
            text: scene.text().into(),
            listen: None,
            debug_hooks: false,
        })
        .unwrap();
        editor.sync();
        assert_eq!(editor.game(), EditorGame::RoomEscape);
        editor.install_room_camera(camera.document.clone()).unwrap();
        Harness::builder()
            .with_size([1800.0, 1200.0])
            .with_step_dt(1.0 / 60.0)
            .wgpu_setup(hardware_vulkan_setup())
            .build_eframe(move |creation_context| {
                let mut app = EditorApp::new(editor, creation_context.wgpu_render_state.clone());
                app.models.install_room(models).unwrap();
                app.room_camera = Some(orr_editor::room_camera_panel::Panel::new(camera));
                app
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

fn settle(harness: &mut Harness<'_, EditorApp>) {
    for _ in 0..4 {
        harness.state_mut().editor.sync();
        harness.run_steps(2);
    }
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

fn click(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness.get_by_label(label).scroll_to_me();
    harness.run_steps(2);
    harness.get_by_label(label).click_accesskit();
    settle(harness);
}

fn click_role(harness: &mut Harness<'_, EditorApp>, role: Role, label: &str) {
    harness.get_by_role_and_label(role, label).scroll_to_me();
    harness.run_steps(2);
    harness.get_by_role_and_label(role, label).click_accesskit();
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
    }
    assert!(
        harness.query_by_label(item).is_some(),
        "missing menu item {item}"
    );
    click(harness, item);
}

fn path_matches(actual: Option<&Path>, expected: &Path) -> bool {
    actual
        .and_then(|path| path.canonicalize().ok())
        .zip(expected.canonicalize().ok())
        .is_some_and(|(actual_path, expected_path)| actual_path == expected_path)
}

fn player_guid(root: &Path) -> orr_reflect::Guid {
    actor_guid(root, PLAYER)
}

fn actor_guid(root: &Path, kind: u32) -> orr_reflect::Guid {
    let prepared = PreparedProject::open(root).unwrap();
    let scene = prepared.scene();
    let frame = scene.frame();
    let index = scene.index();
    let guid = frame
        .entities()
        .find_map(|entity| {
            let actor = frame.get::<RoomActor>(entity)?;
            (actor.kind == kind).then(|| index.guid(entity).cloned().unwrap())
        })
        .expect("admitted Room project contains the requested role");
    guid
}

fn select_actor(harness: &mut Harness<'_, EditorApp>, guid: &orr_reflect::Guid) {
    let label = harness
        .state()
        .editor
        .rows()
        .iter()
        .find(|row| row.guid.as_ref() == Some(guid))
        .expect("role is present in the production hierarchy")
        .label();
    click(harness, &label);
    assert_eq!(
        harness.state().editor.selection(),
        Some(&Target::Guid(guid.clone()))
    );
}

fn player_binding<'a>(
    app: &'a EditorApp,
    player: &orr_reflect::Guid,
) -> Option<&'a orr_editor::model_bindings::Binding> {
    app.models
        .bindings
        .as_ref()?
        .document()
        .bindings
        .get(&player.to_string())
}

fn lock(root: &Path) -> Lock {
    Project::open(root, compiled_runtime())
        .unwrap()
        .verify()
        .unwrap()
}

fn model_bindings_path(root: &Path) -> PathBuf {
    root.join("room.models.json")
}

fn prepare_import(
    harness: &mut Harness<'_, EditorApp>,
    source: &Path,
    license: &Path,
    destination: &Path,
    package: &str,
    kind: &str,
) {
    set_field(
        harness,
        "Raw GLB file",
        &source.canonicalize().unwrap().display().to_string(),
    );
    set_field(
        harness,
        "License file",
        &license.canonicalize().unwrap().display().to_string(),
    );
    set_field(
        harness,
        "Package destination directory",
        &destination.display().to_string(),
    );
    set_field(harness, "Package name", package);
    set_field(harness, "Package version", "1.0.0");
    click_role(harness, Role::RadioButton, kind);
    assert!(
        harness
            .get_by_label("Import raw GLB package")
            .accesskit_node()
            .is_disabled(),
        "import remains disabled until explicit license acknowledgement"
    );
    click_role(
        harness,
        Role::CheckBox,
        "I have reviewed rights to distribute this content",
    );
    assert!(
        !harness
            .get_by_label("Import raw GLB package")
            .accesskit_node()
            .is_disabled(),
        "explicit acknowledgement enables import"
    );
}

fn start_import(harness: &mut Harness<'_, EditorApp>, expected_status: &str) {
    click(harness, "Import raw GLB package");
    wait_for(harness, "raw GLB importer result", |app| {
        !app.package_manager.busy() && app.package_manager.status.contains(expected_status)
    });
}

fn preview_add_verify(
    harness: &mut Harness<'_, EditorApp>,
    root: &Path,
    package_name: &str,
    previous_lock_bytes: &[u8],
    previous_lock: &Lock,
) -> Lock {
    click(harness, "Preview package addition");
    wait_for(harness, "raw package preview", |app| {
        !app.package_manager.busy()
            && app
                .package_manager
                .status
                .starts_with("Preview ready. No package was added")
    });
    assert_eq!(
        fs::read(root.join("orr.packages.lock.json")).unwrap(),
        previous_lock_bytes,
        "preview does not install or activate the package"
    );
    assert!(!lock(root).packages.contains_key(package_name));

    click(harness, "Add previewed package");
    wait_for(harness, "raw package addition", |app| {
        !app.package_manager.busy() && app.package_manager.status.starts_with("Package added.")
    });
    let added = lock(root);
    assert_eq!(added.packages.len(), previous_lock.packages.len() + 1);
    for (name, package) in &previous_lock.packages {
        assert_eq!(added.packages[name], *package, "existing package preserved");
    }
    assert_ne!(
        fs::read(root.join("orr.packages.lock.json")).unwrap(),
        previous_lock_bytes,
        "explicit Add persists the reviewed package lock"
    );
    click(harness, "Verify installed packages");
    wait_for(harness, "raw package byte verification", |app| {
        !app.package_manager.busy()
            && app
                .package_manager
                .status
                .starts_with("Installed package bytes match")
    });
    added
}

fn source_hashes(status: &str) -> (String, String) {
    let hashes = status
        .split_once("Source SHA-256: ")
        .expect("successful importer status includes the source digest")
        .1;
    let (source, license) = hashes
        .split_once("; license SHA-256: ")
        .expect("successful importer status includes the license digest");
    let license = license.trim_end_matches('.');
    for digest in [source, license] {
        assert_eq!(digest.len(), 64);
        assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
    (source.to_owned(), license.to_owned())
}

fn glb_with_external_buffer(source: &[u8]) -> Vec<u8> {
    let json_length = u32::from_le_bytes(source[12..16].try_into().unwrap()) as usize;
    let json_end = 20 + json_length;
    let mut document: serde_json::Value = serde_json::from_slice(&source[20..json_end]).unwrap();
    document["buffers"][0]["uri"] = serde_json::Value::String("external.bin".into());
    let mut json = serde_json::to_vec(&document).unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let binary_chunk = &source[json_end..];
    let padded_len = json.len();
    let total_len = 12 + 8 + padded_len + binary_chunk.len();
    let mut bytes = Vec::with_capacity(total_len);
    bytes.extend_from_slice(b"glTF");
    bytes.extend_from_slice(&2_u32.to_le_bytes());
    bytes.extend_from_slice(&(total_len as u32).to_le_bytes());
    bytes.extend_from_slice(&(padded_len as u32).to_le_bytes());
    bytes.extend_from_slice(&0x4E4F_534A_u32.to_le_bytes());
    bytes.extend_from_slice(&json);
    bytes.extend_from_slice(binary_chunk);
    bytes.resize(total_len, b' ');
    bytes
}

#[test]
fn raw_static_and_animated_models_import_through_editor_and_survive_reopen() {
    let acceptance_requested = [
        "ORR_REQUIRE_GPU",
        "ORR_RAW_MODEL_IMPORT_ACCEPTANCE",
        "ORR_RAW_MODEL_IMPORT_OUTPUT",
        "ORR_RAW_MODEL_IMPORT_CAPTURE_DIR",
        "ORR_RAW_MODEL_IMPORT_EXPORT_BUNDLE",
        "ORR_WINDOWS_EXPORT_RUNTIME",
    ]
    .iter()
    .any(|name| std::env::var_os(name).is_some());
    if !acceptance_requested {
        eprintln!("SKIP: set ORR_REQUIRE_GPU=1 or ORR_RAW_MODEL_IMPORT_ACCEPTANCE=1 for actual raw model import UI acceptance");
        return;
    }
    assert_eq!(
        std::env::var("WGPU_BACKEND").unwrap_or_default(),
        "vulkan",
        "raw model import acceptance requires WGPU_BACKEND=vulkan"
    );
    let gpu = Wgpu::headless(WgpuOptions::default())
        .expect("raw model import acceptance requires a hardware Vulkan GPU");
    assert!(
        !gpu.is_software(),
        "software GPU is not acceptance evidence"
    );
    let expected_adapter = gpu.adapter_name();
    eprintln!("Raw model import GPU preflight: {expected_adapter}");
    drop(gpu);

    let fixture = Fixture::new();
    let initial_lock = lock(&fixture.output);
    assert!(initial_lock.packages.contains_key(EXISTING_PACKAGE));
    let initial_lock_bytes = fs::read(fixture.output.join("orr.packages.lock.json")).unwrap();
    let initial_prepared = PreparedProject::open(&fixture.output).unwrap();
    let initial_scene_checksum = initial_prepared.scene().frame().checksum();
    let initial_bindings = initial_prepared.models().document.bindings.clone();
    let initial_bindings_bytes = fs::read(model_bindings_path(&fixture.output)).unwrap();
    let player = player_guid(&fixture.output);
    let key = actor_guid(&fixture.output, KEY);
    let player_key = player.to_string();
    let key_key = key.to_string();
    let original_player_binding = initial_bindings[&player_key].clone();
    let expected_static_placements = initial_bindings
        .iter()
        .filter(|(_, binding)| binding.kind == ModelKind::Static)
        .count();

    let mut harness = fixture.app();
    settle(&mut harness);
    assert!(harness.state().has_gpu());
    assert_eq!(harness.state().editor.game(), EditorGame::RoomEscape);
    assert_eq!(harness.state().editor.checksum(), initial_scene_checksum);
    assert!(harness.state().room_camera.is_some());
    assert_eq!(
        harness
            .state()
            .viewport3d_gpu()
            .unwrap()
            .gpu()
            .adapter_name(),
        expected_adapter
    );
    select_actor(&mut harness, &player);

    menu_click(&mut harness, "File", "Content packages…");
    assert!(harness.query_by_label("Content packages").is_some());
    click(&mut harness, "Import a raw GLB package");

    let malformed = fixture.raw_source.join("malformed.glb");
    let malformed_destination = fixture.package_source.join("malformed package");
    fs::write(&malformed, b"not a GLB").unwrap();
    let malformed_bytes = fs::read(&malformed).unwrap();
    prepare_import(
        &mut harness,
        &malformed,
        &fixture.static_license,
        &malformed_destination,
        "invalid-raw-model",
        "Static model",
    );
    start_import(&mut harness, "static GLB import:");
    assert!(harness
        .state()
        .package_manager
        .status
        .contains("Raw GLB package import failed:"));
    assert!(!malformed_destination.exists());
    assert_eq!(fs::read(&malformed).unwrap(), malformed_bytes);
    assert_eq!(
        fs::read(fixture.output.join("orr.packages.lock.json")).unwrap(),
        initial_lock_bytes
    );
    assert_eq!(
        fs::read(model_bindings_path(&fixture.output)).unwrap(),
        initial_bindings_bytes
    );

    let external_model = fixture.raw_source.join("external-resource.glb");
    let static_source_bytes = fs::read(&fixture.static_model).unwrap();
    let external_model_bytes = glb_with_external_buffer(&static_source_bytes);
    fs::write(&external_model, &external_model_bytes).unwrap();
    fs::write(fixture.raw_source.join("external.bin"), [1_u8, 2, 3, 4]).unwrap();
    let external_destination = fixture.package_source.join("external package");
    prepare_import(
        &mut harness,
        &external_model,
        &fixture.static_license,
        &external_destination,
        "external-raw-model",
        "Static model",
    );
    start_import(&mut harness, "external resources are unsupported");
    assert!(harness
        .state()
        .package_manager
        .status
        .contains("external resources are unsupported"));
    assert!(!external_destination.exists());
    assert_eq!(fs::read(&external_model).unwrap(), external_model_bytes);
    assert_eq!(
        fs::read(fixture.raw_source.join("external.bin")).unwrap(),
        [1_u8, 2, 3, 4]
    );
    assert_eq!(
        fs::read(fixture.output.join("orr.packages.lock.json")).unwrap(),
        initial_lock_bytes
    );

    let existing_destination = fixture.package_source.join("existing destination");
    fs::create_dir(&existing_destination).unwrap();
    let sentinel_path = existing_destination.join("keep.bin");
    let sentinel_bytes = b"preserve the pre-existing directory";
    fs::write(&sentinel_path, sentinel_bytes).unwrap();
    prepare_import(
        &mut harness,
        &fixture.static_model,
        &fixture.static_license,
        &existing_destination,
        "existing-raw-model",
        "Static model",
    );
    start_import(&mut harness, "package destination already exists");
    assert!(harness
        .state()
        .package_manager
        .status
        .contains("package destination already exists"));
    assert_eq!(fs::read(&sentinel_path).unwrap(), sentinel_bytes);
    assert_eq!(
        fs::read(fixture.output.join("orr.packages.lock.json")).unwrap(),
        initial_lock_bytes
    );
    assert_eq!(
        fs::read(model_bindings_path(&fixture.output)).unwrap(),
        initial_bindings_bytes
    );

    let static_destination = fixture.package_source.join("static package – 世界");
    prepare_import(
        &mut harness,
        &fixture.static_model,
        &fixture.static_license,
        &static_destination,
        STATIC_PACKAGE,
        "Static model",
    );
    start_import(
        &mut harness,
        &format!("Imported raw package {STATIC_PACKAGE}/{STATIC_ASSET};"),
    );
    let static_status = harness.state().package_manager.status.clone();
    assert!(static_status.starts_with(&format!(
        "Imported raw package {STATIC_PACKAGE}/{STATIC_ASSET}; not installed or assigned. Preview and add it explicitly."
    )));
    let (static_source_hash, static_license_hash) = source_hashes(&static_status);
    assert!(path_matches(
        Some(Path::new(&harness.state().package_manager.source)),
        &static_destination
    ));
    assert_eq!(
        fs::read(static_destination.join(STATIC_ASSET)).unwrap(),
        fs::read(&fixture.static_model).unwrap()
    );
    assert_eq!(
        fs::read(static_destination.join("LICENSE.txt")).unwrap(),
        fs::read(&fixture.static_license).unwrap()
    );
    let static_manifest: orr_package::Manifest =
        serde_json::from_slice(&fs::read(static_destination.join("orr.package.json")).unwrap())
            .unwrap();
    assert_eq!(static_manifest.name, STATIC_PACKAGE);
    assert!(static_manifest.capabilities.contains("models"));
    assert!(!static_manifest.capabilities.contains("animation"));
    assert_eq!(
        fs::read(fixture.output.join("orr.packages.lock.json")).unwrap(),
        initial_lock_bytes,
        "raw import creates an external source package only"
    );
    assert_eq!(
        fs::read(model_bindings_path(&fixture.output)).unwrap(),
        initial_bindings_bytes
    );
    assert!(harness
        .query_by_label(&format!("Use model {STATIC_PACKAGE}/{STATIC_ASSET}"))
        .is_none());

    let static_added_lock = preview_add_verify(
        &mut harness,
        &fixture.output,
        STATIC_PACKAGE,
        &initial_lock_bytes,
        &initial_lock,
    );
    let static_installed_lock_bytes =
        fs::read(fixture.output.join("orr.packages.lock.json")).unwrap();
    assert_eq!(
        static_added_lock.packages[STATIC_PACKAGE].files[STATIC_ASSET],
        static_source_hash
    );
    assert_eq!(
        static_added_lock.packages[STATIC_PACKAGE].files["LICENSE.txt"],
        static_license_hash
    );
    assert!(harness
        .query_by_label(&format!("Use model {STATIC_PACKAGE}/{STATIC_ASSET}"))
        .is_some());
    click(&mut harness, "Close window");
    select_actor(&mut harness, &key);
    menu_click(&mut harness, "File", "Content packages…");
    click(
        &mut harness,
        &format!("Use model {STATIC_PACKAGE}/{STATIC_ASSET}"),
    );
    wait_for(&mut harness, "static KEY model assignment", |app| {
        player_binding(app, &key).is_some_and(|binding| {
            binding.kind == ModelKind::Static
                && binding.package == STATIC_PACKAGE
                && binding.asset == STATIC_ASSET
        })
    });
    let static_binding = player_binding(harness.state(), &key)
        .expect("static importer action binds the selected KEY")
        .clone();
    assert_eq!(
        static_binding.package_digest,
        static_added_lock.packages[STATIC_PACKAGE].digest
    );
    assert_eq!(static_binding.source_hash, static_source_hash);
    assert_eq!(harness.state().editor.checksum(), initial_scene_checksum);
    assert_eq!(
        fs::read(model_bindings_path(&fixture.output)).unwrap(),
        initial_bindings_bytes,
        "assigning is not a sidecar save"
    );
    click(&mut harness, "Close window");
    if harness.query_by_label("Save model bindings").is_none() {
        click(&mut harness, "Static model binding");
    }
    click(&mut harness, "Save model bindings");
    let static_saved_bindings = fs::read(model_bindings_path(&fixture.output)).unwrap();
    assert_ne!(static_saved_bindings, initial_bindings_bytes);
    assert!(!harness.state().models.bindings.as_ref().unwrap().dirty());

    select_actor(&mut harness, &player);
    menu_click(&mut harness, "File", "Content packages…");
    let animated_destination = fixture.package_source.join("animated package – 動き");
    prepare_import(
        &mut harness,
        &fixture.animated_model,
        &fixture.animated_license,
        &animated_destination,
        ANIMATED_PACKAGE,
        "Animated model",
    );
    start_import(
        &mut harness,
        &format!("Imported raw package {ANIMATED_PACKAGE}/{ANIMATED_ASSET};"),
    );
    let animated_status = harness.state().package_manager.status.clone();
    assert!(animated_status.starts_with(&format!(
        "Imported raw package {ANIMATED_PACKAGE}/{ANIMATED_ASSET}; not installed or assigned. Preview and add it explicitly."
    )));
    let (animated_source_hash, animated_license_hash) = source_hashes(&animated_status);
    let animated_manifest: orr_package::Manifest =
        serde_json::from_slice(&fs::read(animated_destination.join("orr.package.json")).unwrap())
            .unwrap();
    assert_eq!(animated_manifest.name, ANIMATED_PACKAGE);
    assert!(animated_manifest.capabilities.contains("models"));
    assert!(animated_manifest.capabilities.contains("animation"));
    assert_eq!(
        fs::read(animated_destination.join(ANIMATED_ASSET)).unwrap(),
        fs::read(&fixture.animated_model).unwrap()
    );
    assert_eq!(
        fs::read(animated_destination.join("LICENSE.txt")).unwrap(),
        fs::read(&fixture.animated_license).unwrap()
    );
    assert_eq!(
        fs::read(fixture.output.join("orr.packages.lock.json")).unwrap(),
        static_installed_lock_bytes,
        "a raw package source does not alter the installed lock"
    );
    assert_eq!(
        fs::read(model_bindings_path(&fixture.output)).unwrap(),
        static_saved_bindings,
        "a raw package source does not change saved model bindings"
    );
    assert_eq!(
        player_binding(harness.state(), &player),
        Some(&original_player_binding),
        "raw import alone does not replace the original PLAYER binding"
    );
    assert_eq!(player_binding(harness.state(), &key), Some(&static_binding));

    let animated_added_lock = preview_add_verify(
        &mut harness,
        &fixture.output,
        ANIMATED_PACKAGE,
        &static_installed_lock_bytes,
        &static_added_lock,
    );
    let animated_installed_lock_bytes =
        fs::read(fixture.output.join("orr.packages.lock.json")).unwrap();
    assert_eq!(
        animated_added_lock.packages[ANIMATED_PACKAGE].files[ANIMATED_ASSET],
        animated_source_hash
    );
    assert_eq!(
        animated_added_lock.packages[ANIMATED_PACKAGE].files["LICENSE.txt"],
        animated_license_hash
    );
    assert!(harness
        .query_by_label(&format!(
            "Use animated model {ANIMATED_PACKAGE}/{ANIMATED_ASSET}"
        ))
        .is_some());
    click(
        &mut harness,
        &format!("Use animated model {ANIMATED_PACKAGE}/{ANIMATED_ASSET}"),
    );
    wait_for(&mut harness, "animated model assignment", |app| {
        player_binding(app, &player).is_some_and(|binding| {
            binding.kind == ModelKind::Animated
                && binding.package == ANIMATED_PACKAGE
                && binding.asset == ANIMATED_ASSET
                && binding.animation.is_some()
        })
    });
    let default_animated_binding = player_binding(harness.state(), &player)
        .expect("animated importer action binds the selected PLAYER")
        .clone();
    assert_eq!(default_animated_binding.source_hash, animated_source_hash);
    assert_eq!(
        default_animated_binding.package_digest,
        animated_added_lock.packages[ANIMATED_PACKAGE].digest
    );
    assert_eq!(harness.state().editor.checksum(), initial_scene_checksum);

    click(&mut harness, "Close window");
    if harness.query_by_label("Animation clip").is_none() {
        click(&mut harness, "Static model binding");
    }
    let clip_label = {
        let placements = harness
            .state()
            .models
            .animated_placements(&harness.state().editor)
            .unwrap();
        let player_entity = harness
            .state()
            .editor
            .rows()
            .iter()
            .find(|row| row.guid.as_ref() == Some(&player))
            .unwrap()
            .entity;
        let placement = placements
            .iter()
            .find(|placement| placement.entity == player_entity)
            .unwrap();
        let clip = &placement.model.source().clips[1];
        format!("1: {} ({:.2}s)", clip.name, clip.duration())
    };
    click_role(&mut harness, Role::ComboBox, "Animation clip");
    harness.run_steps(2);
    harness.get_by_label(&clip_label).click_accesskit();
    settle(&mut harness);
    click(&mut harness, "Once");
    click(&mut harness, "Assign animated model");
    wait_for(&mut harness, "animated clip selection", |app| {
        player_binding(app, &player).is_some_and(|binding| {
            binding.animation.is_some_and(|animation| {
                animation.clip_index == 1 && animation.playback == PlaybackMode::Once
            })
        })
    });
    let animated_binding = player_binding(harness.state(), &player)
        .expect("chosen animation clip is assigned")
        .clone();
    assert_eq!(
        fs::read(model_bindings_path(&fixture.output)).unwrap(),
        static_saved_bindings,
        "animated assignment is not implicitly saved"
    );
    click(&mut harness, "Save model bindings");
    let saved_bindings_bytes = fs::read(model_bindings_path(&fixture.output)).unwrap();
    assert_ne!(saved_bindings_bytes, static_saved_bindings);
    assert!(!harness.state().models.bindings.as_ref().unwrap().dirty());
    let saved_bindings = harness
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings
        .clone();
    assert_eq!(saved_bindings[&player_key], animated_binding);
    for (guid, binding) in &initial_bindings {
        if guid != &player_key && guid != &key_key {
            assert_eq!(saved_bindings[guid], *binding, "other bindings preserved");
        }
    }
    assert_eq!(saved_bindings[&key_key], static_binding);

    click(&mut harness, "Static model binding");
    menu_click(&mut harness, "File", "Open Room project…");
    set_field(
        &mut harness,
        "Existing Room project directory",
        &fixture.other.display().to_string(),
    );
    click(&mut harness, "Open Room project");
    wait_for(&mut harness, "opening another Room project", |app| {
        !app.windows_projects.busy()
            && path_matches(
                app.editor.path().as_deref(),
                &fixture.other.join("room.scene.yaml"),
            )
    });
    menu_click(&mut harness, "File", "Open Room project…");
    set_field(
        &mut harness,
        "Existing Room project directory",
        &fixture.output.display().to_string(),
    );
    click(&mut harness, "Open Room project");
    wait_for(
        &mut harness,
        "reopening the imported model project",
        |app| {
            !app.windows_projects.busy()
                && app
                    .windows_projects
                    .status
                    .starts_with("Opened Room project:")
                && path_matches(
                    app.editor.path().as_deref(),
                    &fixture.output.join("room.scene.yaml"),
                )
                && app.editor.game() == EditorGame::RoomEscape
                && !app.editor.is_dirty()
                && app.editor.checksum() == initial_scene_checksum
                && app.room_camera.is_some()
                && app
                    .models
                    .animated_placements(&app.editor)
                    .is_ok_and(|placements| placements.len() == 1)
                && app.models.placements(&app.editor).len() == expected_static_placements - 1
        },
    );
    assert_eq!(
        player_binding(harness.state(), &player),
        Some(&animated_binding)
    );
    let reopened = PreparedProject::open(&fixture.output).unwrap();
    assert_eq!(reopened.scene().frame().checksum(), initial_scene_checksum);
    assert!(reopened.camera().is_some());
    let reopened_lock = lock(&fixture.output);
    assert_eq!(
        reopened_lock.packages[STATIC_PACKAGE],
        animated_added_lock.packages[STATIC_PACKAGE]
    );
    assert_eq!(
        reopened_lock.packages[ANIMATED_PACKAGE],
        animated_added_lock.packages[ANIMATED_PACKAGE]
    );
    assert_eq!(reopened.models().document.bindings, saved_bindings);
    assert!(reopened
        .models()
        .assets
        .contains_key(&(STATIC_PACKAGE.into(), STATIC_ASSET.into())));
    assert!(reopened
        .models()
        .assets
        .contains_key(&(ANIMATED_PACKAGE.into(), ANIMATED_ASSET.into())));
    assert_eq!(
        reopened.models().document.bindings[&player_key],
        animated_binding
    );

    let player_entity = harness
        .state()
        .editor
        .rows()
        .iter()
        .find(|row| row.guid.as_ref() == Some(&player))
        .unwrap()
        .entity;
    let rest_pose = harness
        .state()
        .models
        .animated_placements(&harness.state().editor)
        .unwrap()
        .into_iter()
        .find(|placement| placement.entity == player_entity)
        .unwrap()
        .pose;
    click(&mut harness, LBL_PLAY);
    assert_eq!(harness.state().editor.mode(), Mode::Play);
    harness.run_steps(60);
    click(&mut harness, LBL_PAUSE);
    assert!(!harness.state().editor.sim().playing);
    let snapshot = harness.state().editor.snapshot().unwrap();
    let tick = snapshot.tick();
    let tick_rate = snapshot.tick_rate();
    let sample_time = tick as f32 / tick_rate as f32;
    let animated_player = harness
        .state()
        .models
        .animated_placements(&harness.state().editor)
        .unwrap()
        .into_iter()
        .find(|placement| placement.entity == player_entity)
        .expect("animated PLAYER follows the reopened Room snapshot");
    assert_ne!(
        animated_player.pose, rest_pose,
        "simulation advances the animated pose"
    );
    assert_eq!(
        animated_player.pose,
        animated_player
            .model
            .sample_clip(1, sample_time)
            .expect("selected clip samples at the authoritative paused tick")
    );
    let before_step = harness.state().editor.timeline().unwrap().tick;
    click(&mut harness, LBL_STEP);
    assert!(!harness.state().editor.sim().playing);
    assert_eq!(
        harness.state().editor.timeline().unwrap().tick,
        before_step + 1
    );
    click(&mut harness, LBL_STOP);
    assert_eq!(harness.state().editor.mode(), Mode::Edit);
    assert_eq!(harness.state().editor.checksum(), initial_scene_checksum);

    let mut export_manifest_digest = None;
    if let Some(runtime) = std::env::var_os("ORR_WINDOWS_EXPORT_RUNTIME").map(PathBuf::from) {
        assert!(runtime.is_absolute());
        assert!(runtime.is_file());
        menu_click(&mut harness, "File", "Export Windows game…");
        set_field(
            &mut harness,
            "Runtime executable",
            &runtime.display().to_string(),
        );
        click(&mut harness, "Check runtime SHA-256");
        wait_for(&mut harness, "trusted Windows runtime measurement", |app| {
            !app.windows_export.busy()
        });
        assert_eq!(harness.state().windows_export.runtime_sha256.len(), 64);
        click_role(
            &mut harness,
            Role::CheckBox,
            "I trust this runtime executable",
        );
        let export_output = std::env::var_os("ORR_RAW_MODEL_IMPORT_EXPORT_BUNDLE")
            .map(PathBuf::from)
            .unwrap_or_else(|| fixture.evidence_root.join("raw-model-import-export"));
        assert!(export_output.is_absolute());
        assert!(!export_output.exists());
        set_field(
            &mut harness,
            "Export destination",
            &export_output.display().to_string(),
        );
        click_role(&mut harness, Role::Button, "Export Windows game");
        wait_for(&mut harness, "same-App Windows export", |app| {
            !app.windows_export.busy()
        });
        assert!(
            harness.state().windows_export.last_output.is_some(),
            "{}",
            harness.state().windows_export.status
        );
        assert!(path_matches(
            harness.state().windows_export.last_output.as_deref(),
            &export_output
        ));
        let manifest: orr_sample::windows_export::ExportManifest =
            serde_json::from_slice(&fs::read(export_output.join("orr.export.json")).unwrap())
                .unwrap();
        assert_eq!(
            manifest.payload.packages[STATIC_PACKAGE],
            reopened_lock.packages[STATIC_PACKAGE].digest
        );
        assert_eq!(
            manifest.payload.packages[ANIMATED_PACKAGE],
            reopened_lock.packages[ANIMATED_PACKAGE].digest
        );
        assert_eq!(
            manifest.payload.initial_checksum,
            format!("0x{initial_scene_checksum:016x}")
        );
        assert_eq!(
            manifest.payload.runtime.sha256,
            harness.state().windows_export.runtime_sha256
        );
        assert_eq!(
            harness.state().windows_export.last_digest.as_deref(),
            Some(manifest.content_digest.as_str())
        );
        export_manifest_digest = Some(manifest.content_digest);
    } else {
        assert!(
            std::env::var_os("ORR_RAW_MODEL_IMPORT_EXPORT_BUNDLE").is_none(),
            "an export destination requires ORR_WINDOWS_EXPORT_RUNTIME"
        );
    }

    if let Some(capture_root) = &fixture.capture_root {
        let frame = harness
            .render()
            .expect("capture the actual hardware-rendered EditorApp");
        assert!(frame.width() >= 800 && frame.height() >= 600);
        frame
            .save(capture_root.join("raw-model-import-ui.png"))
            .unwrap();
    }
    let receipt = serde_json::json!({
        "room_project": fixture.output.display().to_string(),
        "raw_source_directory": fixture.raw_source.display().to_string(),
        "game": "RoomEscape",
        "gpu_adapter": expected_adapter,
        "player_guid": player.to_string(),
        "initial_scene_checksum": format!("0x{initial_scene_checksum:016x}"),
        "final_edit_checksum": format!("0x{:016x}", harness.state().editor.checksum()),
        "static_package": {
            "name": STATIC_PACKAGE,
            "asset": STATIC_ASSET,
            "root": static_destination.display().to_string(),
            "source_sha256": static_source_hash,
            "license_sha256": static_license_hash,
            "installed_digest": reopened_lock.packages[STATIC_PACKAGE].digest,
        },
        "animated_package": {
            "name": ANIMATED_PACKAGE,
            "asset": ANIMATED_ASSET,
            "root": animated_destination.display().to_string(),
            "source_sha256": animated_source_hash,
            "license_sha256": animated_license_hash,
            "installed_digest": reopened_lock.packages[ANIMATED_PACKAGE].digest,
        },
        "player_binding_kind": "animated",
        "player_binding_clip": animated_binding.animation.unwrap().clip_index,
        "player_binding_playback": "once",
        "initial_model_bindings": initial_bindings.len(),
        "final_model_bindings": reopened.models().document.bindings.len(),
        "static_key_binding": {
            "guid": key.to_string(),
            "package": STATIC_PACKAGE,
            "asset": STATIC_ASSET,
            "kind": "static",
        },
        "static_model_placements": harness.state().models.placements(&harness.state().editor).len(),
        "animated_model_placements": harness.state().models.animated_placements(&harness.state().editor).unwrap().len(),
        "camera_installed": harness.state().room_camera.is_some(),
        "animation_sample_tick": tick,
        "export_manifest_digest": export_manifest_digest,
    });
    eprintln!("Raw model import acceptance receipt: {receipt}");
    if let Some(capture_root) = &fixture.capture_root {
        fs::write(
            capture_root.join("acceptance.json"),
            serde_json::to_vec_pretty(&receipt).unwrap(),
        )
        .unwrap();
    }
    assert_eq!(
        fs::read(fixture.output.join("orr.packages.lock.json")).unwrap(),
        animated_installed_lock_bytes
    );
    assert_eq!(
        player_binding(harness.state(), &player),
        Some(&animated_binding)
    );
    assert_eq!(player_binding(harness.state(), &key), Some(&static_binding));
    assert_eq!(
        original_player_binding.kind,
        initial_bindings[&player_key].kind
    );
    assert!(fixture.raw_source.join("static input/model.glb").is_file());
    assert!(fixture
        .raw_source
        .join("animated input/model.glb")
        .is_file());
    assert_eq!(animated_license_hash.len(), 64);
}
