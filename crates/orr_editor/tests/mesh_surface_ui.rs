#![cfg(all(windows, target_arch = "x86_64", feature = "mesh-navigation"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::{accesskit::Role, Key, Modifiers};
use egui_kittest::{kittest::Queryable, Harness};
use egui_wgpu::{wgpu, WgpuSetupCreateNew};
use orr_editor::{
    app::{LBL_PAUSE, LBL_PLAY, LBL_STEP, LBL_STOP},
    game::EditorGame,
    Editor, EditorApp, HostSpec,
};
use orr_remote::mesh_navigation::mesh_from_view;
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

struct Fixture {
    _temporary: tempfile::TempDir,
    scene: PathBuf,
    physics: PathBuf,
    package: PathBuf,
    capture: PathBuf,
    hidden_package: Option<PathBuf>,
    export: Option<ExportFixture>,
}

struct ExportFixture {
    output: PathBuf,
    runtime: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        assert_eq!(
            std::env::var("ORR_MESH_SURFACE_ACCEPTANCE").as_deref(),
            Ok("1")
        );
        assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
        assert_eq!(std::env::var("WGPU_BACKEND").as_deref(), Ok("vulkan"));
        let temporary = tempfile::tempdir().unwrap();
        let capture = PathBuf::from(
            std::env::var_os("ORR_MESH_SURFACE_CAPTURE_DIR")
                .expect("surface acceptance requires a fresh capture directory"),
        );
        assert!(capture.is_absolute());
        fs::create_dir(&capture).expect("capture directory must be fresh");
        let capture = capture.canonicalize().unwrap();
        let scene_candidate = capture.join("surface.scene.yaml");
        let scene_parent = scene_candidate.parent().unwrap().canonicalize().unwrap();
        let scene = scene_parent.join(scene_candidate.file_name().unwrap());
        assert!(!scene.exists());
        let physics = capture.join("physics.scene.yaml");
        fs::copy(EditorGame::PhysGame.default_scene_path(), &physics).unwrap();
        let package_source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/imported_scene_demo")
            .canonicalize()
            .expect("checked-in model package is present");
        let package = capture.join("owned-model-package");
        copy_tree(&package_source, &package).unwrap();
        assert!(package.join("foreground.glb").is_file());
        assert!(package.join("LICENSE.txt").is_file());
        let export = match (
            std::env::var_os("ORR_MESH_SURFACE_EXPORT_OUTPUT"),
            std::env::var_os("ORR_MESH_EXPORT_RUNTIME"),
        ) {
            (None, None) => None,
            (Some(output), Some(runtime)) => {
                let output = PathBuf::from(output);
                let runtime = PathBuf::from(runtime);
                assert!(output.is_absolute() && output.parent().is_some_and(Path::is_dir));
                assert!(!output.exists(), "export output must be a new directory");
                assert!(runtime.is_absolute() && runtime.is_file());
                Some(ExportFixture { output, runtime })
            }
            _ => panic!("optional export requires both ORR_MESH_SURFACE_EXPORT_OUTPUT and ORR_MESH_EXPORT_RUNTIME"),
        };
        Self {
            _temporary: temporary,
            scene,
            physics,
            package,
            capture,
            hidden_package: None,
            export,
        }
    }

    fn hide_package(&mut self) {
        let hidden = self.package.with_file_name("owned-model-package-hidden");
        assert!(!hidden.exists());
        fs::rename(&self.package, &hidden).expect("hide source package before reopen");
        self.hidden_package = Some(hidden);
    }

    fn restore_package(&mut self) {
        let hidden = self
            .hidden_package
            .as_ref()
            .expect("package source was hidden");
        assert!(!self.package.exists());
        fs::rename(hidden, &self.package).expect("restore owned package source");
        self.hidden_package = None;
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(hidden) = self.hidden_package.take() {
            if !self.package.exists() && hidden.exists() {
                if let Err(error) = fs::rename(hidden, &self.package) {
                    eprintln!("Could not restore hidden surface-test package: {error}");
                }
            }
        }
    }
}

fn copy_tree(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if entry.file_type()?.is_file() {
            fs::copy(entry.path(), target)?;
        } else {
            return Err(std::io::Error::other(
                "model package contains a non-file entry",
            ));
        }
    }
    Ok(())
}

fn gpu_setup() -> egui_wgpu::WgpuSetup {
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

fn harness(physics: &Path) -> Harness<'static, EditorApp> {
    let mut editor = Editor::start(&HostSpec::local_game(
        physics.to_path_buf(),
        EditorGame::PhysGame,
    ))
    .expect("start a normal Physics editor");
    editor.sync();
    Harness::builder()
        .with_size([1600.0, 1100.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu_setup(gpu_setup())
        .build_eframe(move |creation_context| {
            EditorApp::new(editor, creation_context.wgpu_render_state.clone())
        })
}

fn settle(harness: &mut Harness<'_, EditorApp>) {
    for _ in 0..4 {
        harness.state_mut().editor.sync();
        harness.run_steps(2);
    }
}

fn click(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness.get_by_label(label).scroll_to_me();
    harness.run_steps(6);
    harness.get_by_label(label).click_accesskit();
    settle(harness);
}

fn click_button(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness
        .get_by_role_and_label(Role::Button, label)
        .scroll_to_me();
    harness.run_steps(6);
    harness
        .get_by_role_and_label(Role::Button, label)
        .click_accesskit();
    settle(harness);
}

fn field(harness: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    harness.run_steps(6);
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

fn menu(harness: &mut Harness<'_, EditorApp>, item: &str) {
    if harness.query_by_label(item).is_none() {
        click(harness, "File");
        harness.run_steps(2);
    }
    click(harness, item);
}

fn wait_for(
    harness: &mut Harness<'_, EditorApp>,
    reason: &str,
    ready: impl Fn(&EditorApp) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        settle(harness);
        if ready(harness.state()) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let app = harness.state();
    panic!(
        "timed out waiting for {reason}: game={:?}, path={:?}, frame_checksum={}, sim_checksum={}",
        app.editor.game(),
        app.editor.path(),
        app.editor.checksum(),
        app.editor.sim().checksum,
    );
}

fn viewport_pixels(harness: &mut Harness<'_, EditorApp>, name: &str) -> Vec<u8> {
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
    let viewport = harness.state().viewport3d_gpu().unwrap();
    let size = viewport.gpu().target().size();
    assert_eq!(size, target);
    let pixels = viewport.gpu().read_rgba8();
    let image =
        egui::ColorImage::from_rgba_unmultiplied([size.0 as usize, size.1 as usize], &pixels);
    orr_editor::app::write_png(
        &PathBuf::from(std::env::var_os("ORR_MESH_SURFACE_CAPTURE_DIR").unwrap())
            .join(format!("{name}.png")),
        &image,
    )
    .unwrap();
    pixels
}

fn changed_pixels(left: &[u8], right: &[u8]) -> usize {
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

fn mesh_view(harness: &Harness<'_, EditorApp>) -> orr_remote::mesh_navigation::MeshView {
    mesh_from_view(harness.state().editor.snapshot().unwrap().predicted()).unwrap()
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn first_material_index(package: &Path) -> u32 {
    orr_model::import::import_path(package, "foreground.glb")
        .expect("fixture package imports as a static model")
        .source()
        .primitives
        .first()
        .expect("fixture model has a primitive")
        .material
}

#[cfg(feature = "mesh-export")]
fn export_bundle(harness: &mut Harness<'_, EditorApp>, fixture: &Fixture) -> serde_json::Value {
    let Some(export) = fixture.export.as_ref() else {
        return serde_json::Value::Null;
    };
    menu(harness, "Export mesh game…");
    field(
        harness,
        "Mesh player executable",
        &export.runtime.display().to_string(),
    );
    field(
        harness,
        "Mesh game output directory",
        &export.output.display().to_string(),
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
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(harness
        .query_by_label_contains("Mesh export complete:")
        .is_some());
    let manifest_path = export.output.join("orr.mesh-bundle.json");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    let scene_name = fixture
        .scene
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let art_name = format!("{scene_name}.mesh-art.json");
    let props_name = format!("{scene_name}.mesh-props.json");
    let scene_copy = export.output.join(&scene_name);
    let art_copy = export.output.join(&art_name);
    let props_copy = export.output.join(&props_name);
    let runtime_copy = export.output.join("orr_mesh_player.exe");
    assert!(
        scene_copy.is_file()
            && art_copy.is_file()
            && props_copy.is_file()
            && runtime_copy.is_file()
    );
    assert_eq!(fs::read_dir(&export.output).unwrap().count(), 5);
    assert!(manifest_path.is_file());
    assert_eq!(manifest["scene"]["name"], scene_name);
    assert_eq!(manifest["sidecar"]["name"], art_name);
    assert_eq!(manifest["props"]["name"], props_name);
    assert_eq!(
        fs::read(&scene_copy).unwrap(),
        fs::read(&fixture.scene).unwrap()
    );
    assert_eq!(
        fs::read(&art_copy).unwrap(),
        fs::read(format!("{}.mesh-art.json", fixture.scene.display())).unwrap()
    );
    assert_eq!(
        fs::read(&props_copy).unwrap(),
        fs::read(format!("{}.mesh-props.json", fixture.scene.display())).unwrap()
    );
    assert_eq!(
        fs::read(&runtime_copy).unwrap(),
        fs::read(&export.runtime).unwrap()
    );
    let runtime_bytes = fs::read(&runtime_copy).unwrap();
    assert!(runtime_bytes
        .windows(b"ORR_MESH_SURFACE_V1".len())
        .any(|window| window == b"ORR_MESH_SURFACE_V1"));
    let artifact_sha256 = |path: &Path| sha256(&fs::read(path).unwrap());
    for (record, path) in [
        (&manifest["scene"], &scene_copy),
        (&manifest["sidecar"], &art_copy),
        (&manifest["props"], &props_copy),
        (&manifest["runtime"], &runtime_copy),
    ] {
        let bytes = fs::read(path).unwrap();
        assert_eq!(record["bytes"].as_u64(), Some(bytes.len() as u64));
        assert_eq!(
            record["sha256"].as_str(),
            Some(artifact_sha256(path).as_str())
        );
    }
    serde_json::json!({
        "directory": export.output,
        "manifest": manifest,
        "scene_sha256": artifact_sha256(&scene_copy),
        "art_sidecar_sha256": artifact_sha256(&art_copy),
        "props_sidecar_sha256": artifact_sha256(&props_copy),
        "runtime_sha256": artifact_sha256(&runtime_copy),
        "manifest_path": manifest_path,
        "manifest_sha256": artifact_sha256(&export.output.join("orr.mesh-bundle.json")),
        "scene_path": scene_copy,
        "art_sidecar_path": art_copy,
        "props_sidecar_path": props_copy,
        "runtime_path": runtime_copy,
    })
}

#[cfg(not(feature = "mesh-export"))]
fn export_bundle(_harness: &mut Harness<'_, EditorApp>, fixture: &Fixture) -> serde_json::Value {
    assert!(
        fixture.export.is_none(),
        "export variables require mesh-export"
    );
    serde_json::Value::Null
}

#[test]
#[ignore = "requires ORR_MESH_SURFACE_ACCEPTANCE=1 and a hardware Vulkan adapter"]
fn mesh_surface_uv_and_pivot_render_persist_and_reopen_source_free() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    assert_eq!(std::env::var("WGPU_BACKEND").as_deref(), Ok("vulkan"));
    let gpu = Wgpu::headless(WgpuOptions::default()).expect("hardware GPU is required");
    assert!(
        !gpu.is_software(),
        "software renderer is not hardware GPU evidence"
    );
    eprintln!("Mesh UV/pivot GPU: {}", gpu.adapter_name());

    let mut fixture = Fixture::new();
    let surface_material = first_material_index(&fixture.package);
    let mut app = harness(&fixture.physics);
    menu(&mut app, "New mesh scene…");
    field(&mut app, "Scene path", &fixture.scene.display().to_string());
    click_button(&mut app, "Create");
    wait_for(&mut app, "new mesh scene", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && app.editor.path().as_deref().is_some_and(|path| {
                path == fixture.scene.as_path()
                    || path.canonicalize().ok() == fixture.scene.canonicalize().ok()
            })
    });
    click(&mut app, "Static-mesh navigation");
    click_button(&mut app, "Create ramp and bridge");
    click_button(&mut app, "Place mesh agent");

    if app.query_by_label("Agent art offset X").is_none() {
        click(&mut app, "Mesh agent art");
    }
    for (label, value) in [
        (
            "Art package source directory",
            fixture.package.display().to_string(),
        ),
        ("Art model asset", "foreground.glb".to_owned()),
        ("Art license asset", "LICENSE.txt".to_owned()),
    ] {
        field(&mut app, label, &value);
    }
    click(&mut app, "I have reviewed rights to use this agent art");
    click_button(&mut app, "Load agent art");
    click_button(&mut app, "Apply agent art");
    let art_identity = viewport_pixels(&mut app, "agent-art-identity");
    let scene_checksum = app.state().editor.checksum();
    click(&mut app, "Edit art UV and pivot");
    field(
        &mut app,
        "Art UV material index",
        &surface_material.to_string(),
    );
    field(&mut app, "Art UV offset U", "0.35");
    field(&mut app, "Art UV scale U", "1.75");
    click_button(&mut app, "Apply agent art");
    let art_uv = viewport_pixels(&mut app, "agent-art-uv");
    let art_uv_diff = changed_pixels(&art_identity, &art_uv);
    assert!(
        art_uv_diff > 20,
        "UV edits must change the actual rendered artwork at a fixed agent pose"
    );
    assert_eq!(
        app.state().editor.checksum(),
        scene_checksum,
        "art sidecar edits do not mutate host scene bytes"
    );

    field(&mut app, "Art pivot X", "0.35");
    click_button(&mut app, "Apply agent art");
    let art_pivot = viewport_pixels(&mut app, "agent-art-pivot");
    let art_pivot_diff = changed_pixels(&art_uv, &art_pivot);
    assert!(
        art_pivot_diff > 20,
        "pivot edits must affect the actual rendered model placement"
    );
    click_button(&mut app, "Undo agent art");
    assert!(changed_pixels(&art_uv, &viewport_pixels(&mut app, "agent-art-pivot-undo")) < 20);
    click_button(&mut app, "Redo agent art");
    assert!(
        changed_pixels(
            &art_pivot,
            &viewport_pixels(&mut app, "agent-art-pivot-redo")
        ) < 20
    );

    click(&mut app, "Mesh props");
    for (label, value) in [
        (
            "Prop package source directory",
            fixture.package.display().to_string(),
        ),
        ("Prop model asset", "foreground.glb".to_owned()),
        ("Prop license asset", "LICENSE.txt".to_owned()),
        ("Prop name", "Surface prop".to_owned()),
    ] {
        field(&mut app, label, &value);
    }
    field(&mut app, "Prop position X", "1");
    field(&mut app, "Prop position Z", "1.5");
    click(&mut app, "I have reviewed rights to use these props");
    click_button(&mut app, "Load prop candidate");
    click_button(&mut app, "Add prop");
    let prop_identity = viewport_pixels(&mut app, "prop-identity");
    click(&mut app, "Edit prop UV and pivot");
    field(
        &mut app,
        "Prop UV material index",
        &surface_material.to_string(),
    );
    field(&mut app, "Prop UV offset V", "0.4");
    field(&mut app, "Prop UV scale V", "1.5");
    click_button(&mut app, "Update selected prop");
    let prop_uv = viewport_pixels(&mut app, "prop-uv");
    let prop_uv_diff = changed_pixels(&prop_identity, &prop_uv);
    assert!(
        prop_uv_diff > 20,
        "prop UV edits must change the actual rendered prop"
    );
    field(&mut app, "Prop pivot Z", "0.3");
    click_button(&mut app, "Update selected prop");
    let prop_pivot = viewport_pixels(&mut app, "prop-pivot");
    let prop_pivot_diff = changed_pixels(&prop_uv, &prop_pivot);
    assert!(
        prop_pivot_diff > 20,
        "prop pivot edits must affect rendered prop geometry"
    );
    click_button(&mut app, "Undo prop edit");
    assert!(changed_pixels(&prop_uv, &viewport_pixels(&mut app, "prop-pivot-undo")) < 20);
    click_button(&mut app, "Redo prop edit");
    assert!(changed_pixels(&prop_pivot, &viewport_pixels(&mut app, "prop-pivot-redo")) < 20);

    click_button(&mut app, "Save agent art");
    click_button(&mut app, "Save props");
    let art_path = PathBuf::from(format!("{}.mesh-art.json", fixture.scene.display()));
    let props_path = PathBuf::from(format!("{}.mesh-props.json", fixture.scene.display()));
    let art_bytes = fs::read(&art_path).unwrap();
    let props_bytes = fs::read(&props_path).unwrap();
    let art_json: serde_json::Value = serde_json::from_slice(&art_bytes).unwrap();
    let props_json: serde_json::Value = serde_json::from_slice(&props_bytes).unwrap();
    assert_eq!(art_json["surface"]["material"], surface_material);
    assert!((art_json["surface"]["pivot"][0].as_f64().unwrap() - 0.35).abs() < 1.0e-6);
    assert_eq!(
        props_json["props"][0]["art"]["surface"]["material"],
        surface_material
    );
    assert!(
        (props_json["props"][0]["art"]["surface"]["pivot"][2]
            .as_f64()
            .unwrap()
            - 0.3)
            .abs()
            < 1.0e-6
    );
    menu(&mut app, "Save Ctrl+S");
    let scene_bytes = fs::read(&fixture.scene).unwrap();
    let scene_sha256 = sha256(&scene_bytes);
    let source_glb = fs::read(fixture.package.join("foreground.glb")).unwrap();
    let source_glb_sha256 = sha256(&source_glb);
    let source_license = fs::read(fixture.package.join("LICENSE.txt")).unwrap();
    let source_license_sha256 = sha256(&source_license);
    fixture.hide_package();
    assert!(!fixture.package.exists());
    let hidden_package = fixture.hidden_package.as_ref().unwrap();
    let hidden_glb_sha256 = sha256(&fs::read(hidden_package.join("foreground.glb")).unwrap());
    let hidden_license_sha256 = sha256(&fs::read(hidden_package.join("LICENSE.txt")).unwrap());
    assert_eq!(
        hidden_glb_sha256, source_glb_sha256,
        "surface transforms leave the imported source asset unchanged"
    );
    assert_eq!(hidden_license_sha256, source_license_sha256);

    app = harness(&fixture.physics);
    menu(&mut app, "Open mesh scene…");
    field(&mut app, "Scene path", &fixture.scene.display().to_string());
    click_button(&mut app, "OK");
    wait_for(&mut app, "fresh source-free mesh scene reopen", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && app.editor.path().as_deref().is_some_and(|path| {
                path == fixture.scene.as_path()
                    || path.canonicalize().ok() == fixture.scene.canonicalize().ok()
            })
    });
    let reopened = viewport_pixels(&mut app, "source-free-reopen");
    assert!(
        changed_pixels(&prop_pivot, &reopened) < 20,
        "source-free reopen reproduces saved surface presentation"
    );
    click_button(&mut app, LBL_PLAY);
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && app.state().editor.mode() != orr_editor::Mode::Play {
        settle(&mut app);
    }
    assert_eq!(app.state().editor.mode(), orr_editor::Mode::Play);
    click_button(&mut app, LBL_PAUSE);
    if app
        .query_by_role_and_label(Role::Button, "\u{23EE}")
        .is_none()
    {
        click(&mut app, "Timeline");
    }
    click_button(&mut app, "\u{23EE}");
    wait_for(&mut app, "paused playback rewound to tick zero", |app| {
        app.editor
            .timeline()
            .is_some_and(|timeline| timeline.tick == 0 && !timeline.playing)
    });
    let start_position = mesh_view(&app).agent.position;
    for _ in 0..16 {
        click_button(&mut app, LBL_STEP);
    }
    let play_view = mesh_view(&app);
    assert_eq!(app.state().editor.timeline().unwrap().tick, 16);
    assert_ne!(play_view.agent.position, start_position);
    let playing = viewport_pixels(&mut app, "source-free-play");
    assert!(
        changed_pixels(&reopened, &playing) > 0,
        "runtime presentation continues with embedded models"
    );
    click_button(&mut app, LBL_STOP);
    let stopped = viewport_pixels(&mut app, "source-free-stop");
    assert!(changed_pixels(&reopened, &stopped) < 20);
    assert_eq!(app.state().editor.checksum(), scene_checksum);
    assert_eq!(fs::read(&fixture.scene).unwrap(), scene_bytes);
    let export = export_bundle(&mut app, &fixture);

    fixture.restore_package();
    assert_eq!(
        sha256(&fs::read(fixture.package.join("foreground.glb")).unwrap()),
        source_glb_sha256
    );
    assert_eq!(
        sha256(&fs::read(fixture.package.join("LICENSE.txt")).unwrap()),
        source_license_sha256
    );
    let captures = fs::read_dir(&fixture.capture)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "png")
        })
        .map(|entry| {
            let bytes = fs::read(entry.path()).unwrap();
            serde_json::json!({
                "path": entry.file_name().to_string_lossy().to_string(),
                "bytes": bytes.len(),
                "sha256": sha256(&bytes),
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(captures.len(), 13, "receipt inventories every GPU capture");

    let receipt = serde_json::json!({
        "artifact_root": fixture.capture,
        "scene_path": fixture.scene,
        "scene_sha256": scene_sha256,
        "scene_checksum": format!("0x{scene_checksum:016x}"),
        "art_sidecar": art_path,
        "props_sidecar": props_path,
        "source_package_directory": fixture.package,
        "source_glb_sha256_before": source_glb_sha256,
        "source_glb_sha256_hidden": hidden_glb_sha256,
        "source_glb_sha256_restored": sha256(&fs::read(fixture.package.join("foreground.glb")).unwrap()),
        "source_license_sha256_before": source_license_sha256,
        "source_license_sha256_hidden": hidden_license_sha256,
        "source_license_sha256_restored": sha256(&fs::read(fixture.package.join("LICENSE.txt")).unwrap()),
        "source_package_hidden_during_reopen": true,
        "source_package_restored": fixture.hidden_package.is_none() && fixture.package.exists(),
        "source_free_reopen_pixels_verified": changed_pixels(&prop_pivot, &reopened) < 20,
        "play_tick": 16,
        "play_agent_moved": play_view.agent.position != start_position,
        "play_start_position_raw": start_position.map(orr_fp::FP::raw),
        "play_end_position_raw": play_view.agent.position.map(orr_fp::FP::raw),
        "scene_checksum_unchanged": app.state().editor.checksum() == scene_checksum,
        "art_sidecar_sha256": sha256(&art_bytes),
        "props_sidecar_sha256": sha256(&props_bytes),
        "art_uv_pixel_diff": art_uv_diff,
        "art_pivot_pixel_diff": art_pivot_diff,
        "prop_uv_pixel_diff": prop_uv_diff,
        "prop_pivot_pixel_diff": prop_pivot_diff,
        "art_undo_redo_pixels_verified": true,
        "prop_undo_redo_pixels_verified": true,
        "play_stop_source_free_verified": true,
        "export": export,
        "captures": captures,
        "physical_os_input": false,
    });
    fs::write(
        fixture.capture.join("acceptance.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
}
