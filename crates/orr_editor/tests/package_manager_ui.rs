#![cfg(all(
    windows,
    target_arch = "x86_64",
    feature = "windows-projects",
    feature = "package-manager"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_editor::{editor::Mode, game::EditorGame, Editor, EditorApp, HostSpec, Target};
use orr_package::{Lock, Project};
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use orr_sample::{
    room_game::{RoomActor, PLAYER},
    room_project::{compiled_runtime, PreparedProject},
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const NEW_PACKAGE: &str = "ui-added-model";
const MODEL_ASSET: &str = "foreground.glb";
const LICENSE_ASSET: &str = "LICENSE.txt";
const OLD_PACKAGE: &str = "sample-imported-scene";

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    output: PathBuf,
    other: PathBuf,
    source: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let output = std::env::var_os("ORR_PACKAGE_MANAGER_OUTPUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| temp.path().join("package managed Room project"));
        assert!(
            output.is_absolute(),
            "package manager output must be absolute"
        );
        assert!(!output.exists(), "package manager output must be new");
        assert!(output.parent().is_some_and(Path::is_dir));
        let root = output.clone();
        let other = temp.path().join("other Room project");
        let source = temp.path().join("local package source");
        orr_sample::windows_project::create_room_project(&root, "package-manager-ui-initial")
            .expect("create initial admitted Room project");
        orr_sample::windows_project::create_room_project(&other, "package-manager-ui-other")
            .expect("create a second admitted Room project");
        Self {
            _temp: temp,
            root,
            output,
            other,
            source,
        }
    }

    fn app(&self) -> Harness<'static, EditorApp> {
        let mut prepared = PreparedProject::open(&self.root).unwrap();
        let camera = prepared
            .take_camera()
            .expect("Room project declares a camera");
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
            .wgpu()
            .build_eframe(move |creation_context| {
                let mut app = EditorApp::new(editor, creation_context.wgpu_render_state.clone());
                app.models.install_room(models).unwrap();
                app.room_camera = Some(orr_editor::room_camera_panel::Panel::new(camera));
                app
            })
    }

    fn create_package_source(&self) {
        let project = Project::open(&self.root, compiled_runtime()).unwrap();
        let original_lock = project.verify().unwrap();
        assert!(original_lock.packages.contains_key(OLD_PACKAGE));
        let model_bytes = project.read_asset(OLD_PACKAGE, MODEL_ASSET).unwrap();
        let license_bytes = project.read_asset(OLD_PACKAGE, LICENSE_ASSET).unwrap();
        fs::create_dir(&self.source).expect("local package source directory must be new");
        fs::write(self.source.join(MODEL_ASSET), model_bytes).unwrap();
        fs::write(self.source.join(LICENSE_ASSET), license_bytes).unwrap();
        fs::write(
            self.source.join("orr.package.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema": 1,
                "name": NEW_PACKAGE,
                "version": "1.0.0",
                "engine": "*",
                "capabilities": ["models"],
                "dependencies": {},
                "files": [MODEL_ASSET, LICENSE_ASSET],
            }))
            .unwrap(),
        )
        .unwrap();
    }
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
    harness.get_by_label(label).click();
    settle(harness);
}

fn click_role(harness: &mut Harness<'_, EditorApp>, role: Role, label: &str) {
    harness.get_by_role_and_label(role, label).scroll_to_me();
    harness.run_steps(2);
    harness.get_by_role_and_label(role, label).click();
    settle(harness);
}

fn set_field(harness: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    let input = harness.get_by_role_and_label(Role::TextInput, label);
    input.scroll_to_me();
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
        harness.get_by_label(menu).click();
        harness.run_steps(2);
    }
    assert!(
        harness.query_by_label(item).is_some(),
        "missing menu item {item}"
    );
    harness.get_by_label(item).click();
    settle(harness);
}

fn path_matches(actual: Option<&Path>, expected: &Path) -> bool {
    actual
        .and_then(|path| path.canonicalize().ok())
        .zip(expected.canonicalize().ok())
        .is_some_and(|(actual_path, expected_path)| actual_path == expected_path)
}

fn player_guid(root: &Path) -> orr_reflect::Guid {
    let prepared = PreparedProject::open(root).unwrap();
    let scene = prepared.scene();
    let frame = scene.frame();
    let index = scene.index();
    let guid = frame
        .entities()
        .find_map(|entity| {
            let actor = frame.get::<RoomActor>(entity)?;
            (actor.kind == PLAYER).then(|| index.guid(entity).cloned().unwrap())
        })
        .expect("admitted Room project has a PLAYER");
    guid
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

fn binding_file(root: &Path) -> PathBuf {
    root.join("room.models.json")
}

fn capture(harness: &mut Harness<'_, EditorApp>, output: &Path, adapter: &str) {
    fs::create_dir(output).expect("capture directory must be new");
    let mut retries = 0;
    let frame = loop {
        match harness.render() {
            Ok(frame) => break frame,
            Err(error) if error.contains("requested Wait timed out") && retries < 2 => {
                retries += 1;
                eprintln!("full EditorApp capture timed out; retry {retries}/2: {error}");
                harness.run_steps(2);
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => panic!("full EditorApp capture failed: {error}"),
        }
    };
    assert!(frame.width() >= 800 && frame.height() >= 600);
    frame.save(output.join("package-manager-ui.png")).unwrap();
    fs::write(
        output.join("acceptance.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "project": harness.state().editor.path(),
            "game": "RoomEscape",
            "viewport_adapter": adapter,
            "installed_package": NEW_PACKAGE,
            "player_binding_asset": MODEL_ASSET,
            "editor_checksum": format!("0x{:016x}", harness.state().editor.checksum()),
            "model_placements": harness.state().models.placements(&harness.state().editor).len(),
            "camera_installed": harness.state().room_camera.is_some(),
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn content_package_manager_verify_preview_add_bind_save_reopen_and_export() {
    let acceptance_requested = std::env::var_os("ORR_REQUIRE_GPU").is_some()
        || std::env::var_os("ORR_PACKAGE_MANAGER_ACCEPTANCE").is_some()
        || std::env::var_os("ORR_WINDOWS_EXPORT_RUNTIME").is_some();
    if !acceptance_requested {
        eprintln!(
            "SKIP: set ORR_REQUIRE_GPU=1 or ORR_PACKAGE_MANAGER_ACCEPTANCE=1 for actual Content packages UI acceptance"
        );
        return;
    }

    let preflight = Wgpu::headless(WgpuOptions::default())
        .expect("package manager acceptance requires an available GPU");
    eprintln!(
        "package manager independent GPU preflight: {}",
        preflight.adapter_name()
    );
    drop(preflight);

    let fixture = Fixture::new();
    fixture.create_package_source();
    let initial_lock = lock(&fixture.output);
    let initial_lock_bytes = fs::read(fixture.output.join("orr.packages.lock.json")).unwrap();
    let initial_prepared = PreparedProject::open(&fixture.output).unwrap();
    let initial_scene_checksum = initial_prepared.scene().frame().checksum();
    let initial_bindings = initial_prepared.models().document.bindings.clone();
    let initial_models_bytes = fs::read(binding_file(&fixture.output)).unwrap();
    let player = player_guid(&fixture.output);
    let player_key = player.to_string();
    let initial_player_binding = initial_bindings.get(&player_key).unwrap().clone();

    let mut harness = fixture.app();
    settle(&mut harness);
    assert!(harness.state().has_gpu());
    assert_eq!(harness.state().editor.game(), EditorGame::RoomEscape);
    assert_eq!(harness.state().editor.checksum(), initial_scene_checksum);
    assert_eq!(
        harness
            .state()
            .models
            .placements(&harness.state().editor)
            .len(),
        2
    );
    assert!(harness.state().room_camera.is_some());
    let viewport = harness
        .state()
        .viewport3d_gpu()
        .expect("production EditorApp viewport")
        .gpu();
    let viewport_size = viewport.target().size();
    let viewport_pixels = viewport.read_rgba8();
    let viewport_adapter = viewport.adapter_name();
    eprintln!("package manager EditorApp viewport GPU: {viewport_adapter}");
    assert!(viewport_size.0 > 0 && viewport_size.1 > 0);
    assert_eq!(
        viewport_pixels.len(),
        (viewport_size.0 * viewport_size.1 * 4) as usize
    );
    assert!(viewport_pixels
        .as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| pixel[3] > 0));

    menu_click(&mut harness, "File", "Content packages…");
    assert!(harness.query_by_label("Content packages").is_some());
    let package_source = fixture.source.canonicalize().unwrap();
    let canonical_output = fixture.output.canonicalize().unwrap();
    assert!(harness
        .query_by_label(&format!(
            "Current Room project: {}",
            canonical_output.display()
        ))
        .is_some());
    set_field(
        &mut harness,
        "Local content package directory",
        &package_source.display().to_string(),
    );
    click(&mut harness, "Verify installed packages");
    wait_for(&mut harness, "verified existing package bytes", |app| {
        !app.package_manager.busy()
            && app
                .package_manager
                .status
                .starts_with("Installed package bytes match")
    });
    assert!(harness
        .query_by_label("Use model sample-imported-scene/foreground.glb")
        .is_some());
    assert_eq!(
        fs::read(fixture.output.join("orr.packages.lock.json")).unwrap(),
        initial_lock_bytes
    );
    assert_eq!(
        fs::read(binding_file(&fixture.output)).unwrap(),
        initial_models_bytes
    );
    assert!(harness
        .get_by_label("Add previewed package")
        .accesskit_node()
        .is_disabled());

    click(&mut harness, "Preview package addition");
    wait_for(&mut harness, "package addition preview", |app| {
        !app.package_manager.busy()
            && app
                .package_manager
                .status
                .starts_with("Preview ready. No package was added")
    });
    assert_eq!(
        fs::read(fixture.output.join("orr.packages.lock.json")).unwrap(),
        initial_lock_bytes
    );
    assert_eq!(
        fs::read(binding_file(&fixture.output)).unwrap(),
        initial_models_bytes
    );
    assert!(!lock(&fixture.output).packages.contains_key(NEW_PACKAGE));
    assert!(!harness
        .get_by_label("Add previewed package")
        .accesskit_node()
        .is_disabled());

    click(&mut harness, "Add previewed package");
    wait_for(&mut harness, "package addition committed", |app| {
        !app.package_manager.busy() && app.package_manager.status.starts_with("Package added.")
    });
    let added_lock = lock(&fixture.output);
    assert_eq!(added_lock.packages.len(), initial_lock.packages.len() + 1);
    assert_eq!(
        added_lock.packages[OLD_PACKAGE],
        initial_lock.packages[OLD_PACKAGE]
    );
    let new_package = &added_lock.packages[NEW_PACKAGE];
    assert_eq!(new_package.manifest.files.len(), 2);
    assert!(new_package.files.contains_key(MODEL_ASSET));
    assert!(new_package.files.contains_key(LICENSE_ASSET));
    assert_eq!(
        fs::read(binding_file(&fixture.output)).unwrap(),
        initial_models_bytes
    );
    assert_eq!(
        player_binding(harness.state(), &player).unwrap(),
        &initial_player_binding
    );

    click(&mut harness, "Verify installed packages");
    wait_for(&mut harness, "new package byte verification", |app| {
        !app.package_manager.busy()
            && app
                .package_manager
                .status
                .starts_with("Installed package bytes match")
    });
    assert!(harness
        .query_by_label("Use model ui-added-model/foreground.glb")
        .is_some());
    harness
        .state_mut()
        .editor
        .select(Some(Target::Guid(player.clone())));
    settle(&mut harness);
    click(&mut harness, "Use model ui-added-model/foreground.glb");
    wait_for(&mut harness, "verified asset binding request", |app| {
        player_binding(app, &player)
            .is_some_and(|binding| binding.package == NEW_PACKAGE && binding.asset == MODEL_ASSET)
    });
    let current_binding = player_binding(harness.state(), &player).unwrap();
    assert_eq!(current_binding.package_digest, new_package.digest);
    assert_eq!(current_binding.source_hash, new_package.files[MODEL_ASSET]);
    assert_eq!(
        harness
            .state()
            .models
            .placements(&harness.state().editor)
            .len(),
        2
    );
    assert!(harness.state().models.bindings.as_ref().unwrap().dirty());
    assert_eq!(harness.state().editor.checksum(), initial_scene_checksum);

    if harness.query_by_label("Save model bindings").is_none() {
        click(&mut harness, "Static model binding");
    }
    click(&mut harness, "Save model bindings");
    assert!(!harness.state().models.bindings.as_ref().unwrap().dirty());
    let saved_binding_bytes = fs::read(binding_file(&fixture.output)).unwrap();
    assert_ne!(saved_binding_bytes, initial_models_bytes);
    let saved_bindings = harness
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings
        .clone();
    assert_eq!(saved_bindings[&player_key].package, NEW_PACKAGE);
    for (guid, binding) in &initial_bindings {
        if guid != &player_key {
            assert_eq!(
                saved_bindings[guid], *binding,
                "unselected model binding preserved"
            );
        }
    }

    click(&mut harness, "Close window");
    menu_click(&mut harness, "File", "Open Room project…");
    set_field(
        &mut harness,
        "Existing Room project directory",
        &fixture.other.display().to_string(),
    );
    click(&mut harness, "Open Room project");
    wait_for(&mut harness, "opening the second Room project", |app| {
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
        "reopening the package-managed Room project",
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
                && app.models.placements(&app.editor).len() == 2
                && app.room_camera.is_some()
        },
    );
    assert_eq!(
        harness
            .state()
            .models
            .bindings
            .as_ref()
            .unwrap()
            .document()
            .bindings,
        saved_bindings
    );
    assert_eq!(
        player_binding(harness.state(), &player).unwrap(),
        &saved_bindings[&player_key]
    );
    let reopened = PreparedProject::open(&fixture.output).unwrap();
    let reopened_lock = lock(&fixture.output);
    assert_eq!(
        reopened_lock.packages[OLD_PACKAGE],
        initial_lock.packages[OLD_PACKAGE]
    );
    assert_eq!(
        reopened_lock.packages[NEW_PACKAGE],
        added_lock.packages[NEW_PACKAGE]
    );
    assert!(reopened
        .models()
        .assets
        .contains_key(&(NEW_PACKAGE.into(), MODEL_ASSET.into())));
    assert_eq!(reopened.models().document.bindings, saved_bindings);
    assert!(reopened.camera().is_some());

    click(&mut harness, orr_editor::app::LBL_PLAY);
    assert_eq!(harness.state().editor.mode(), Mode::Play);
    click(&mut harness, orr_editor::app::LBL_PAUSE);
    assert!(!harness.state().editor.sim().playing);
    let before_step = harness.state().editor.timeline().unwrap().tick;
    click(&mut harness, orr_editor::app::LBL_STEP);
    assert!(!harness.state().editor.sim().playing);
    assert_eq!(
        harness.state().editor.timeline().unwrap().tick,
        before_step + 1
    );
    click(&mut harness, orr_editor::app::LBL_STOP);
    assert_eq!(harness.state().editor.mode(), Mode::Edit);

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
        wait_for(&mut harness, "runtime measurement", |app| {
            !app.windows_export.busy()
        });
        assert_eq!(harness.state().windows_export.runtime_sha256.len(), 64);
        click_role(
            &mut harness,
            Role::CheckBox,
            "I trust this runtime executable",
        );
        let export_output = std::env::var_os("ORR_PACKAGE_MANAGER_EXPORT_BUNDLE")
            .map(PathBuf::from)
            .unwrap_or_else(|| fixture._temp.path().join("package-manager-export"));
        assert!(export_output.is_absolute());
        assert!(!export_output.exists());
        set_field(
            &mut harness,
            "Export destination",
            &export_output.display().to_string(),
        );
        click_role(&mut harness, Role::Button, "Export Windows game");
        wait_for(&mut harness, "same-App package project export", |app| {
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
            manifest.payload.runtime.sha256,
            harness.state().windows_export.runtime_sha256
        );
        assert_eq!(
            manifest.payload.packages[NEW_PACKAGE],
            added_lock.packages[NEW_PACKAGE].digest
        );
        assert_eq!(
            manifest.payload.packages[OLD_PACKAGE],
            initial_lock.packages[OLD_PACKAGE].digest
        );
        assert_eq!(
            manifest.payload.initial_checksum,
            format!("0x{initial_scene_checksum:016x}")
        );
        assert_eq!(
            harness.state().windows_export.last_digest.as_deref(),
            Some(manifest.content_digest.as_str())
        );
    }

    if let Some(capture_root) =
        std::env::var_os("ORR_PACKAGE_MANAGER_CAPTURE_DIR").map(PathBuf::from)
    {
        assert!(capture_root.is_absolute());
        capture(&mut harness, &capture_root, &viewport_adapter);
    }
}
