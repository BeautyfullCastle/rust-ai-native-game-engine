#![cfg(all(windows, target_arch = "x86_64", feature = "windows-export"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::accesskit::Role;
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{game::EditorGame, Editor, EditorApp, HostSpec};
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
use orr_package::{Project, Runtime};
use orr_reflect::Scene;
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use orr_sample::{
    room_game::{RoomConfig, RoomEscapeV1, EXIT, KEY, PLAYER},
    room_project::{self, PreparedProject, PreparedScene, SEED},
};
use orr_sim::Simulation;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("room-project");
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let simulation = Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
        let text = Scene::unbake(&room_project::types(), simulation.frame(), None)
            .unwrap()
            .to_yaml();
        fs::write(root.join("room.scene.yaml"), &text).unwrap();
        fs::write(
            root.join("orr.project.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema": 2,
                "engine": "*",
                "entry": {
                    "game": "room-escape-v1",
                    "scene": "room.scene.yaml",
                    "models": "room.models.json"
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/imported_scene_demo")
            .canonicalize()
            .unwrap();
        Project::open_for_install(&root, Runtime::content_only().engine_version)
            .unwrap()
            .install(&[source])
            .unwrap();
        let loaded =
            model_bindings::load_asset(&root, "sample-imported-scene", "foreground.glb").unwrap();
        let binding = Binding::from_asset(
            "sample-imported-scene".into(),
            "foreground.glb".into(),
            &loaded,
            LocalTransform::default(),
        )
        .unwrap();
        let scene = PreparedScene::parse(&text).unwrap();
        let mut actors = BTreeMap::new();
        for entity in scene.frame().entities() {
            let kind = scene
                .frame()
                .get::<orr_sample::room_game::RoomActor>(entity)
                .unwrap()
                .kind;
            if [PLAYER, KEY, EXIT].contains(&kind) {
                actors.insert(kind, scene.index().guid(entity).unwrap().to_string());
            }
        }
        let document = Document {
            version: 2,
            scene: "room.scene.yaml".into(),
            project: ".".into(),
            bindings: [PLAYER, KEY]
                .into_iter()
                .map(|kind| (actors[&kind].clone(), binding.clone()))
                .collect(),
        };
        fs::write(
            root.join("room.models.json"),
            serde_json::to_vec_pretty(&document).unwrap(),
        )
        .unwrap();
        Self { _temp: temp, root }
    }

    fn app(&self) -> Harness<'static, EditorApp> {
        let prepared = PreparedProject::open(&self.root).unwrap();
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
        Harness::builder()
            .with_size([1400.0, 1000.0])
            .with_step_dt(1.0 / 60.0)
            .wgpu()
            .build_eframe(move |creation_context| {
                let mut app = EditorApp::new(editor, creation_context.wgpu_render_state.clone());
                app.models.install_room(models).unwrap();
                app
            })
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

fn menu_click(harness: &mut Harness<'_, EditorApp>, menu: &str, item: &str) {
    harness.get_by_label(menu).click();
    harness.run_steps(2);
    harness.get_by_label(item).click();
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

fn wait_for(
    harness: &mut Harness<'_, EditorApp>,
    reason: &str,
    ready: impl Fn(&EditorApp) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        settle(harness);
        if ready(harness.state()) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {reason}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn production_windows_export_ui_measures_trusted_runtime_and_publishes_admitted_room() {
    let runtime = std::env::var_os("ORR_WINDOWS_EXPORT_RUNTIME").map(PathBuf::from);
    let acceptance_requested = std::env::var_os("ORR_REQUIRE_GPU").is_some()
        || std::env::var_os("ORR_WINDOWS_EXPORT_ACCEPTANCE").is_some()
        || runtime.is_some();
    let Some(runtime) = runtime else {
        assert!(
            !acceptance_requested,
            "ORR_WINDOWS_EXPORT_RUNTIME is required for Windows export acceptance"
        );
        eprintln!("SKIP: set ORR_WINDOWS_EXPORT_RUNTIME to a trusted Room runtime for export UI acceptance");
        return;
    };
    assert!(runtime.is_absolute(), "runtime path must be absolute");
    assert!(
        runtime.is_file(),
        "runtime path does not exist: {}",
        runtime.display()
    );
    let gpu = Wgpu::headless(WgpuOptions::default())
        .expect("Windows export UI acceptance requires an available GPU");
    eprintln!(
        "Windows export independent GPU preflight: {}",
        gpu.adapter_name()
    );
    drop(gpu);

    let fixture = Fixture::new();
    let output = if let Some(bundle) = std::env::var_os("ORR_WINDOWS_EXPORT_BUNDLE") {
        let bundle = PathBuf::from(bundle);
        assert!(bundle.is_absolute(), "export bundle path must be absolute");
        assert!(!bundle.exists(), "export bundle path already exists");
        bundle
    } else {
        fixture._temp.path().join("windows-room-export")
    };
    let mut harness = fixture.app();
    settle(&mut harness);
    assert!(harness.state().has_gpu());
    let viewport_gpu = harness
        .state()
        .viewport3d_gpu()
        .expect("production EditorApp 3D viewport")
        .gpu();
    let viewport_adapter = viewport_gpu.adapter_name().to_owned();
    eprintln!(
        "Windows export actual EditorApp viewport GPU: {}",
        viewport_adapter
    );
    let viewport_size = viewport_gpu.target().size();
    let viewport_pixels = viewport_gpu.read_rgba8();
    assert!(viewport_size.0 > 0 && viewport_size.1 > 0);
    assert_eq!(
        viewport_pixels.len(),
        (viewport_size.0 * viewport_size.1 * 4) as usize
    );
    assert!(
        viewport_pixels
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[3] > 0),
        "actual EditorApp viewport should contain rendered pixels"
    );
    let initial_checksum = harness.state().editor.checksum();

    menu_click(&mut harness, "File", "Export Windows game…");
    assert_eq!(
        harness.state().windows_export.project,
        fixture.root.display().to_string()
    );
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
    let measured_sha256 = harness.state().windows_export.runtime_sha256.clone();
    assert!(!harness.state().windows_export.trusted_runtime);
    set_field(
        &mut harness,
        "Export destination",
        &output.display().to_string(),
    );
    click_role(&mut harness, Role::Button, "Export Windows game");
    assert!(!harness.state().windows_export.busy());
    assert!(harness.state().windows_export.last_output.is_none());
    assert!(!harness.state().windows_export.trusted_runtime);
    click_role(
        &mut harness,
        Role::CheckBox,
        "I trust this runtime executable",
    );
    click_role(&mut harness, Role::Button, "Export Windows game");
    wait_for(&mut harness, "Windows export", |app| {
        !app.windows_export.busy()
    });
    assert!(
        harness.state().windows_export.last_output.is_some(),
        "export failed: {}",
        harness.state().windows_export.status
    );

    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    let published_output = output.canonicalize().unwrap();
    assert_eq!(
        harness.state().windows_export.last_output.as_deref(),
        Some(published_output.as_path())
    );
    let manifest_bytes = fs::read(output.join("orr.export.json")).unwrap();
    let manifest: orr_sample::windows_export::ExportManifest =
        serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(
        manifest.payload.initial_checksum,
        format!("0x{initial_checksum:016x}")
    );
    assert_eq!(manifest.payload.runtime.sha256, measured_sha256);
    assert_eq!(
        harness.state().windows_export.last_digest.as_deref(),
        Some(manifest.content_digest.as_str())
    );
    let staged = PreparedProject::open(output.join("project")).unwrap();
    assert_eq!(staged.scene().frame().checksum(), initial_checksum);
    assert_eq!(
        staged.root(),
        output.join("project").canonicalize().unwrap()
    );

    let mut render_retries = 0;
    let frame = loop {
        match harness.render() {
            Ok(frame) => break frame,
            Err(error) if error.contains("requested Wait timed out") && render_retries < 2 => {
                render_retries += 1;
                eprintln!(
                    "Full EditorApp capture queue timed out; retry {render_retries}/2: {error}"
                );
                harness.run_steps(2);
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => panic!("full EditorApp framebuffer after completed export: {error}"),
        }
    };
    assert!(frame.width() >= 800 && frame.height() >= 600);
    if let Some(directory) = std::env::var_os("ORR_WINDOWS_EXPORT_CAPTURE_DIR") {
        let directory = PathBuf::from(directory);
        fs::create_dir_all(&directory).unwrap();
        let viewport_image = egui::ColorImage::from_rgba_unmultiplied(
            [viewport_size.0 as usize, viewport_size.1 as usize],
            &viewport_pixels,
        );
        orr_editor::app::write_png(&directory.join("room-export-viewport.png"), &viewport_image)
            .unwrap();
        frame
            .save(directory.join("windows-export-complete.png"))
            .unwrap();
        fs::write(
            directory.join("receipt.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "source_project": fixture.root.display().to_string(),
                "output": published_output.display().to_string(),
                "initial_checksum": format!("0x{initial_checksum:016x}"),
                "runtime_sha256": measured_sha256,
                "content_digest": manifest.content_digest,
                "viewport_adapter": viewport_adapter,
                "viewport_size": [viewport_size.0, viewport_size.1],
            }))
            .unwrap(),
        )
        .unwrap();
    }
}
