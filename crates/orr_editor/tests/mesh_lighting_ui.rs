#![cfg(all(windows, target_arch = "x86_64", feature = "mesh-navigation"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::accesskit::Role;
use egui::{Key, Modifiers};
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
use orr_remote::mesh_navigation::mesh_from_view;
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

#[cfg(feature = "mesh-export")]
use std::collections::BTreeSet;

struct Fixture {
    capture: PathBuf,
    physics: PathBuf,
    source_workspace: PathBuf,
    hidden_workspace: PathBuf,
    duplicate_directory: PathBuf,
    source_scene: PathBuf,
    scene: PathBuf,
    export: Option<ExportFixture>,
}

struct HiddenWorkspace {
    original: PathBuf,
    hidden: PathBuf,
    restored: bool,
}

impl HiddenWorkspace {
    fn hide(original: &Path, hidden: &Path) -> Self {
        fs::rename(original, hidden).expect("hide original workspace during fresh open");
        Self {
            original: original.to_path_buf(),
            hidden: hidden.to_path_buf(),
            restored: false,
        }
    }

    fn restore(&mut self) {
        if !self.restored {
            fs::rename(&self.hidden, &self.original).expect("restore original workspace");
            self.restored = true;
        }
    }
}

impl Drop for HiddenWorkspace {
    fn drop(&mut self) {
        self.restore();
    }
}

struct ExportFixture {
    output: PathBuf,
    runtime: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        assert_eq!(
            std::env::var("ORR_MESH_LIGHTING_ACCEPTANCE").as_deref(),
            Ok("1")
        );
        assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
        assert_eq!(std::env::var("WGPU_BACKEND").as_deref(), Ok("vulkan"));
        let capture = PathBuf::from(
            std::env::var_os("ORR_MESH_LIGHTING_CAPTURE_DIR")
                .expect("lighting acceptance requires a fresh capture directory"),
        );
        assert!(capture.is_absolute());
        fs::create_dir(&capture).expect("capture directory must be fresh");
        let capture = capture.canonicalize().unwrap();
        let source_workspace = capture.join("source workspace");
        fs::create_dir(&source_workspace).expect("source workspace must be fresh");
        let scene_candidate = std::env::var_os("ORR_MESH_LIGHTING_OUTPUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| source_workspace.join("lighting.scene.yaml"));
        assert!(
            scene_candidate.is_absolute(),
            "scene output must be absolute"
        );
        let scene_parent = scene_candidate
            .parent()
            .expect("scene output must have a parent")
            .canonicalize()
            .expect("scene output parent must already exist");
        assert_eq!(scene_parent, source_workspace.canonicalize().unwrap());
        let source_scene = scene_parent.join(
            scene_candidate
                .file_name()
                .expect("scene output needs a filename"),
        );
        assert!(
            source_scene.starts_with(&capture),
            "scene output must remain inside the fresh fixture root"
        );
        assert!(!source_scene.exists());
        let hidden_workspace = capture.join("hidden source workspace");
        let duplicate_directory = capture.join("duplicate workspace");
        let physics = capture.join("physics.scene.yaml");
        fs::copy(EditorGame::PhysGame.default_scene_path(), &physics).unwrap();
        let export = match (
            std::env::var_os("ORR_MESH_LIGHTING_EXPORT_OUTPUT"),
            std::env::var_os("ORR_MESH_EXPORT_RUNTIME"),
        ) {
            (None, None) => None,
            (Some(output), Some(runtime)) => {
                let output = PathBuf::from(output);
                let runtime = PathBuf::from(runtime);
                assert!(output.is_absolute() && output.parent().is_some_and(Path::is_dir));
                assert!(!output.exists(), "lighting export output must be new");
                let normalized_output = output
                    .parent()
                    .unwrap()
                    .canonicalize()
                    .unwrap()
                    .join(output.file_name().expect("export output needs a directory name"));
                assert!(normalized_output.starts_with(&capture), "export output must stay inside the owned fixture root");
                assert!(runtime.is_absolute() && runtime.is_file());
                Some(ExportFixture { output, runtime })
            }
            _ => panic!("optional export requires both ORR_MESH_LIGHTING_EXPORT_OUTPUT and ORR_MESH_EXPORT_RUNTIME"),
        };
        #[cfg(feature = "mesh-export")]
        assert!(
            export.is_some(),
            "lighting GPU acceptance requires the explicit mesh export inputs"
        );
        Self {
            capture,
            physics,
            source_workspace,
            hidden_workspace,
            duplicate_directory,
            source_scene: source_scene.clone(),
            scene: source_scene,
            export,
        }
    }
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
    .expect("start ordinary Physics editor");
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
    harness.run_steps(8);
    harness.get_by_label(label).click_accesskit();
    settle(harness);
}

fn click_button(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness
        .get_by_role_and_label(Role::Button, label)
        .scroll_to_me();
    harness.run_steps(8);
    harness
        .get_by_role_and_label(Role::Button, label)
        .click_accesskit();
    settle(harness);
}

fn field(harness: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    harness.run_steps(8);
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
        app.editor.sim().checksum
    );
}

fn path_matches(actual: Option<&Path>, expected: &Path) -> bool {
    actual.is_some_and(|actual| {
        actual == expected || actual.canonicalize().ok() == expected.canonicalize().ok()
    })
}

fn pixels(harness: &mut Harness<'_, EditorApp>, fixture: &Fixture, name: &str) -> Vec<u8> {
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
        let scale = harness.ctx.pixels_per_point();
        harness.set_size(egui::vec2(
            window.x + (target.0 as f32 - actual.0 as f32) / scale,
            window.y + (target.1 as f32 - actual.1 as f32) / scale,
        ));
    }
    settle(harness);
    let viewport = harness
        .state()
        .viewport3d_gpu()
        .expect("actual editor GPU viewport");
    let size = viewport.gpu().target().size();
    assert_eq!(size, target);
    let result = viewport.gpu().read_rgba8();
    assert_eq!(result.len(), (size.0 * size.1 * 4) as usize);
    let image =
        egui::ColorImage::from_rgba_unmultiplied([size.0 as usize, size.1 as usize], &result);
    orr_editor::app::write_png(&fixture.capture.join(format!("{name}.png")), &image).unwrap();
    result
}

fn changed_pixels(left: &[u8], right: &[u8]) -> usize {
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

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn mesh_view(harness: &Harness<'_, EditorApp>) -> orr_remote::mesh_navigation::MeshView {
    mesh_from_view(harness.state().editor.snapshot().unwrap().predicted()).unwrap()
}

#[cfg(feature = "mesh-export")]
fn export_bundle(
    harness: &mut Harness<'_, EditorApp>,
    fixture: &Fixture,
    sidecar: &[u8],
) -> serde_json::Value {
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
    let scene_name = fixture
        .scene
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let expected = BTreeSet::from([
        scene_name.clone(),
        format!("{scene_name}.mesh-lighting.json"),
        "orr.mesh-bundle.json".to_owned(),
        "orr_mesh_player.exe".to_owned(),
    ]);
    let actual = fs::read_dir(&export.output)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);
    let manifest_bytes = fs::read(export.output.join("orr.mesh-bundle.json")).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let scene_record = &manifest["scene"];
    let lighting_record = &manifest["lighting"];
    let runtime_record = &manifest["runtime"];
    let scene_bytes = fs::read(export.output.join(&scene_name)).unwrap();
    assert_eq!(scene_bytes, fs::read(&fixture.scene).unwrap());
    assert_eq!(
        scene_record["bytes"].as_u64(),
        Some(scene_bytes.len() as u64)
    );
    assert_eq!(scene_record["sha256"], sha256(&scene_bytes));
    let lighting_name = format!("{scene_name}.mesh-lighting.json");
    assert_eq!(scene_record["name"], scene_name);
    assert_eq!(lighting_record["name"], lighting_name);
    let exported_lighting = fs::read(export.output.join(&lighting_name)).unwrap();
    assert_eq!(exported_lighting, sidecar);
    assert_eq!(
        lighting_record["bytes"].as_u64(),
        Some(exported_lighting.len() as u64)
    );
    assert_eq!(lighting_record["sha256"], sha256(&exported_lighting));
    let runtime = fs::read(export.output.join("orr_mesh_player.exe")).unwrap();
    assert_eq!(runtime, fs::read(&export.runtime).unwrap());
    assert_eq!(runtime_record["bytes"].as_u64(), Some(runtime.len() as u64));
    assert_eq!(runtime_record["sha256"], sha256(&runtime));
    assert!(runtime
        .windows(b"ORR_MESH_LIGHTING_V1".len())
        .any(|window| window == b"ORR_MESH_LIGHTING_V1"));
    serde_json::json!({"directory": export.output, "manifest_sha256": sha256(&manifest_bytes),
        "lighting_sidecar_sha256": sha256(&exported_lighting), "runtime_sha256": sha256(&runtime),
        "files": actual, "lighting_marker": "ORR_MESH_LIGHTING_V1"})
}

#[cfg(not(feature = "mesh-export"))]
fn export_bundle(
    _harness: &mut Harness<'_, EditorApp>,
    fixture: &Fixture,
    _sidecar: &[u8],
) -> serde_json::Value {
    assert!(
        fixture.export.is_none(),
        "export env requires mesh-export feature"
    );
    serde_json::Value::Null
}

#[test]
#[ignore = "requires ORR_MESH_LIGHTING_ACCEPTANCE=1 and a hardware Vulkan adapter"]
fn mesh_lighting_applies_undoes_persists_and_reopens_source_free() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    assert_eq!(std::env::var("WGPU_BACKEND").as_deref(), Ok("vulkan"));
    let gpu = Wgpu::headless(WgpuOptions::default()).expect("hardware GPU is required");
    assert!(
        !gpu.is_software(),
        "software rendering is not GPU acceptance evidence"
    );
    eprintln!("Mesh lighting GPU: {}", gpu.adapter_name());
    let mut fixture = Fixture::new();
    let mut app = harness(&fixture.physics);
    menu(&mut app, "New mesh scene…");
    field(&mut app, "Scene path", &fixture.scene.display().to_string());
    click_button(&mut app, "Create");
    wait_for(&mut app, "new MeshNavigation scene", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && path_matches(app.editor.path().as_deref(), &fixture.scene)
    });
    if app.query_by_label("Create ramp and bridge").is_none() {
        click(&mut app, "Static-mesh navigation");
    }
    if app.query_by_label("Quad origin X").is_none() {
        click(&mut app, "Mesh geometry");
    }
    for (label, value) in [
        ("Maximum slope", "4"),
        ("Quad origin X", "0"),
        ("Quad origin Y", "0"),
        ("Quad origin Z", "0"),
        ("Quad width", "4"),
        ("Quad depth", "4"),
        ("Quad rise", "0"),
    ] {
        field(&mut app, label, value);
    }
    click_button(&mut app, "Stage walkable quad");
    for (label, value) in [
        ("Quad origin X", "0"),
        ("Quad origin Y", "0"),
        ("Quad origin Z", "4"),
        ("Quad width", "4"),
        ("Quad depth", "4"),
        ("Quad rise", "2"),
    ] {
        field(&mut app, label, value);
    }
    click_button(&mut app, "Stage walkable quad");
    for (label, value) in [
        ("Start X", "1"),
        ("Start Y", "0"),
        ("Start Z", "1"),
        ("Goal X", "3"),
        ("Goal Y", "1.5"),
        ("Goal Z", "7"),
        ("Maximum vertical projection", "2"),
        ("Distance per tick", "0.5"),
    ] {
        field(&mut app, label, value);
    }
    click_button(&mut app, "Place mesh agent");
    menu(&mut app, "Save Ctrl+S");
    let initial_scene_checksum = app.state().editor.checksum();
    let initial_graph = mesh_view(&app).graph.revision();
    let scene_bytes = fs::read(&fixture.scene).expect("placed MeshNavigation scene is saved");
    let scene_sha256 = sha256(&scene_bytes);
    let default_pixels = pixels(&mut app, &fixture, "01-default-lighting");

    if app.query_by_label("Lighting direction X").is_none() {
        click(&mut app, "Mesh game lighting");
    }
    click_button(&mut app, "Apply game lighting");
    let default_applied_pixels = pixels(&mut app, &fixture, "02-default-lighting-applied");
    assert!(changed_pixels(&default_pixels, &default_applied_pixels) < 20);
    assert_eq!(app.state().editor.checksum(), initial_scene_checksum);
    click_button(&mut app, "Save game lighting");

    for (label, value) in [
        ("Lighting direction X", "-0.7"),
        ("Lighting direction Y", "-0.45"),
        ("Lighting direction Z", "-0.2"),
        ("Light color R", "0.35"),
        ("Light color G", "0.75"),
        ("Light color B", "0.95"),
        ("Light intensity", "1.35"),
        ("Sky color R", "0.2"),
        ("Sky color G", "0.3"),
        ("Sky color B", "0.45"),
        ("Ground color R", "0.12"),
        ("Ground color G", "0.1"),
        ("Ground color B", "0.08"),
        ("Ambient", "0.18"),
        ("Shadow center X", "1"),
        ("Shadow center Y", "0.5"),
        ("Shadow center Z", "2"),
        ("Shadow radius", "12"),
        ("Exposure", "1.4"),
    ] {
        field(&mut app, label, value);
    }
    click(&mut app, "Enable shadows");
    click(&mut app, "Enable tonemapping");
    assert!(
        app.get_by_label(LBL_PLAY).accesskit_node().is_disabled(),
        "dirty lighting form blocks Play"
    );
    assert_eq!(app.state().editor.mode(), Mode::Edit);
    click_button(&mut app, "Apply game lighting");
    let changed = pixels(&mut app, &fixture, "03-custom-lighting");
    let apply_diff = changed_pixels(&default_pixels, &changed);
    assert!(
        apply_diff > 20,
        "applied lighting changes actual editor GPU pixels"
    );
    assert_eq!(
        app.state().editor.checksum(),
        initial_scene_checksum,
        "lighting is view-only"
    );
    assert_eq!(mesh_view(&app).graph.revision(), initial_graph);
    assert!(
        app.get_by_label(LBL_PLAY).accesskit_node().is_disabled(),
        "unsaved lighting history blocks Play"
    );

    click_button(&mut app, "Undo game lighting");
    let undone = pixels(&mut app, &fixture, "04-undo-lighting");
    assert!(changed_pixels(&default_pixels, &undone) < 20);
    click_button(&mut app, "Redo game lighting");
    let redone = pixels(&mut app, &fixture, "05-redo-lighting");
    assert!(changed_pixels(&changed, &redone) < 20);
    click_button(&mut app, "Save game lighting");
    let sidecar_path = PathBuf::from(format!("{}.mesh-lighting.json", fixture.scene.display()));
    let sidecar = fs::read(&sidecar_path).expect("lighting was explicitly saved");
    let sidecar_sha256 = sha256(&sidecar);
    let document: serde_json::Value = serde_json::from_slice(&sidecar).unwrap();
    assert_eq!(document["identity"], "ORR_MESH_LIGHTING_V1");
    assert_eq!(document["lighting"]["tonemap"], false);
    assert_eq!(document["lighting"]["shadows"], false);
    assert!((document["lighting"]["direction"][0].as_f64().unwrap() + 0.7).abs() < 1.0e-6);
    assert!((document["lighting"]["intensity"].as_f64().unwrap() - 1.35).abs() < 1.0e-6);
    assert!((document["lighting"]["exposure"].as_f64().unwrap() - 1.4).abs() < 1.0e-6);

    menu(&mut app, "Duplicate mesh workspace…");
    field(
        &mut app,
        "Mesh workspace destination directory",
        &fixture.duplicate_directory.display().to_string(),
    );
    click_button(&mut app, "Duplicate mesh workspace");
    let duplicate_scene = fixture
        .duplicate_directory
        .join(fixture.source_scene.file_name().unwrap());
    wait_for(&mut app, "lighting workspace duplicate", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && path_matches(app.editor.path().as_deref(), &duplicate_scene)
    });
    let source_sidecar_path = PathBuf::from(format!(
        "{}.mesh-lighting.json",
        fixture.source_scene.display()
    ));
    let source_sidecar = fs::read(&source_sidecar_path).unwrap();
    let duplicate_sidecar_path =
        PathBuf::from(format!("{}.mesh-lighting.json", duplicate_scene.display()));
    let duplicate_scene_bytes = fs::read(&duplicate_scene).unwrap();
    let duplicate_sidecar = fs::read(&duplicate_sidecar_path).unwrap();
    assert_eq!(duplicate_scene_bytes, scene_bytes);
    assert_eq!(duplicate_sidecar, sidecar);
    fixture.scene = duplicate_scene;
    let active_sidecar_path = duplicate_sidecar_path.clone();

    drop(app);
    let mut hidden_workspace =
        HiddenWorkspace::hide(&fixture.source_workspace, &fixture.hidden_workspace);
    let mut reopened = harness(&fixture.physics);
    menu(&mut reopened, "Open mesh scene…");
    field(
        &mut reopened,
        "Scene path",
        &fixture.scene.display().to_string(),
    );
    click_button(&mut reopened, "OK");
    wait_for(&mut reopened, "source-free lighting reopen", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && path_matches(app.editor.path().as_deref(), &fixture.scene)
    });
    let source_workspace_hidden_during_fresh_open = !fixture.source_workspace.exists();
    assert!(source_workspace_hidden_during_fresh_open);
    assert!(fixture.physics.exists());
    assert_eq!(fs::read(&duplicate_sidecar_path).unwrap(), sidecar);
    assert_eq!(fs::read(&fixture.scene).unwrap(), scene_bytes);
    let reopened_pixels = pixels(&mut reopened, &fixture, "06-source-free-reopen");
    assert!(
        changed_pixels(&redone, &reopened_pixels) < 20,
        "saved lighting renders identically after source-free reopen"
    );
    click_button(&mut reopened, LBL_PLAY);
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && reopened.state().editor.mode() != Mode::Play {
        settle(&mut reopened);
    }
    assert_eq!(reopened.state().editor.mode(), Mode::Play);
    click_button(&mut reopened, LBL_PAUSE);
    click_button(&mut reopened, "⏮");
    wait_for(&mut reopened, "tick zero", |app| {
        app.editor
            .timeline()
            .is_some_and(|timeline| timeline.tick == 0 && !timeline.playing)
    });
    let start_position = mesh_view(&reopened).agent.position;
    let tick_zero = pixels(&mut reopened, &fixture, "07-play-tick-zero");
    let play_tick_zero_preserves_saved_lighting = changed_pixels(&reopened_pixels, &tick_zero) < 20;
    assert!(play_tick_zero_preserves_saved_lighting);
    for _ in 0..16 {
        click_button(&mut reopened, LBL_STEP);
    }
    let tick = reopened.state().editor.timeline().unwrap().tick;
    assert_eq!(tick, 16);
    let end_position = mesh_view(&reopened).agent.position;
    assert_ne!(end_position, start_position);
    let play_pixels = pixels(&mut reopened, &fixture, "08-play-tick-sixteen");
    assert!(changed_pixels(&tick_zero, &play_pixels) > 0);
    click_button(&mut reopened, LBL_STOP);
    assert_eq!(reopened.state().editor.mode(), Mode::Edit);
    assert_eq!(reopened.state().editor.checksum(), initial_scene_checksum);
    assert_eq!(fs::read(&fixture.scene).unwrap(), scene_bytes);
    let stop_pixels = pixels(&mut reopened, &fixture, "09-stop");
    assert!(changed_pixels(&reopened_pixels, &stop_pixels) < 20);
    let export = export_bundle(&mut reopened, &fixture, &sidecar);
    hidden_workspace.restore();
    assert_eq!(fs::read(&fixture.source_scene).unwrap(), scene_bytes);
    assert_eq!(fs::read(&source_sidecar_path).unwrap(), sidecar);

    let captures = fs::read_dir(&fixture.capture).unwrap().filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|extension| extension == "png"))
        .map(|entry| {
            let bytes = fs::read(entry.path()).unwrap();
            serde_json::json!({"name": entry.file_name().to_string_lossy().to_string(), "bytes": bytes.len(), "sha256": sha256(&bytes)})
        }).collect::<Vec<_>>();
    assert_eq!(captures.len(), 9);
    assert!(fixture.physics.exists());
    let receipt = serde_json::json!({
        "artifact_root": fixture.capture,
        "scene_path": fixture.scene,
        "scene_sha256": scene_sha256,
        "scene_checksum": format!("0x{initial_scene_checksum:016x}"),
        "scene_graph_revision": initial_graph,
        "lighting_sidecar_path": active_sidecar_path,
        "lighting_sidecar_sha256": sidecar_sha256,
        "no_external_package_sources": true,
        "source_workspace_path": fixture.source_workspace,
        "source_workspace_hidden_during_fresh_open": source_workspace_hidden_during_fresh_open,
        "source_workspace_restored": fixture.source_workspace.exists(),
        "duplicate_workspace_path": fixture.duplicate_directory,
        "source_scene_path": fixture.source_scene,
        "duplicate_scene_path": fixture.scene,
        "source_lighting_sidecar_path": source_sidecar_path,
        "source_lighting_sidecar_sha256": sha256(&source_sidecar),
        "duplicate_lighting_sidecar_sha256": sha256(&duplicate_sidecar),
        "duplicate_scene_bytes_match_source": duplicate_scene_bytes == scene_bytes,
        "duplicate_lighting_bytes_match_source": duplicate_sidecar == sidecar,
        "default_lighting_apply_pixels_match": changed_pixels(&default_pixels, &default_applied_pixels) < 20,
        "physics_bootstrap_preserved": fixture.physics.exists(),
        "apply_pixel_diff": apply_diff,
        "undo_redo_pixels_verified": true,
        "source_free_reopen_pixels_verified": changed_pixels(&redone, &reopened_pixels) < 20,
        "play_tick": tick,
        "play_tick_zero_preserves_saved_lighting": play_tick_zero_preserves_saved_lighting,
        "play_agent_moved": start_position != end_position,
        "play_stop_restored_pixels": changed_pixels(&reopened_pixels, &stop_pixels) < 20,
        "scene_checksum_unchanged": reopened.state().editor.checksum() == initial_scene_checksum,
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
