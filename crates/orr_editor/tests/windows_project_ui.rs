#![cfg(all(windows, target_arch = "x86_64", feature = "windows-projects"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::accesskit::Role;
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{editor::Mode, game::EditorGame, Editor, EditorApp, HostSpec};
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use orr_sample::{
    room_game::{RoomActor, PLAYER},
    room_project::PreparedProject,
};
use std::{
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
        let root = temp.path().join("initial room project");
        orr_sample::windows_project::create_room_project(&root, "editor-ui-initial")
            .expect("create the initial admitted Room project");
        Self { _temp: temp, root }
    }

    fn app(&self) -> Harness<'static, EditorApp> {
        let mut prepared = PreparedProject::open(&self.root).unwrap();
        let camera = prepared
            .take_camera()
            .expect("Room project declares its camera");
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
            .with_size([1400.0, 1000.0])
            .with_step_dt(1.0 / 60.0)
            .wgpu()
            .build_eframe(move |creation_context| {
                let mut app = EditorApp::new(editor, creation_context.wgpu_render_state.clone());
                app.models.install_room(models).unwrap();
                app.room_camera = Some(orr_editor::room_camera_panel::Panel::new(camera));
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
    if harness.query_by_label(item).is_none() {
        harness.get_by_label(menu).click();
        harness.run_steps(2);
    }
    if harness.query_by_label(item).is_none() {
        if let Some(directory) = std::env::var_os("ORR_WINDOWS_PROJECT_CAPTURE_DIR") {
            let path = PathBuf::from(directory).with_extension("menu-missing.png");
            let mut attempts = 0;
            loop {
                match harness.render() {
                    Ok(frame) => {
                        frame.save(&path).unwrap();
                        eprintln!("Missing menu item {item}; captured {}", path.display());
                        break;
                    }
                    Err(error) if error.contains("requested Wait timed out") && attempts < 2 => {
                        attempts += 1;
                        harness.run_steps(2);
                    }
                    Err(error) => {
                        eprintln!("Missing menu item {item}; capture also failed: {error}");
                        break;
                    }
                }
            }
        }
    }
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

fn player_guid(root: &Path) -> orr_reflect::Guid {
    let prepared = PreparedProject::open(root).unwrap();
    let guid = prepared
        .scene()
        .frame()
        .entities()
        .find_map(|entity| {
            let actor = prepared.scene().frame().get::<RoomActor>(entity)?;
            (actor.kind == PLAYER).then(|| prepared.scene().index().guid(entity).cloned().unwrap())
        })
        .expect("generated project has a player");
    guid
}

fn path_matches(actual: Option<&Path>, expected: &Path) -> bool {
    actual
        .and_then(|path| path.canonicalize().ok())
        .zip(expected.canonicalize().ok())
        .is_some_and(|(actual, expected)| actual == expected)
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

fn capture_evidence(app: &EditorApp, capture_root: &Path, project_root: &Path) {
    fs::create_dir(capture_root).expect("capture directory must be new");
    let viewport = app
        .viewport3d_gpu()
        .expect("actual EditorApp 3D viewport")
        .gpu();
    let size = viewport.target().size();
    let pixels = viewport.read_rgba8();
    assert!(size.0 > 0 && size.1 > 0);
    assert_eq!(pixels.len(), (size.0 * size.1 * 4) as usize);
    let file = fs::File::create(capture_root.join("editor-viewport.png")).unwrap();
    let mut encoder = png::Encoder::new(file, size.0, size.1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&pixels)
        .unwrap();
    fs::write(
        capture_root.join("acceptance.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "project": project_root,
            "editor_game": "RoomEscape",
            "model_placements": 2,
            "camera_installed": true,
            "final_scene_checksum": app.editor.checksum(),
            "viewport_size": [size.0, size.1],
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn windows_room_project_ui_create_guard_reopen_and_play_gpu_acceptance() {
    let acceptance_requested = std::env::var_os("ORR_REQUIRE_GPU").is_some()
        || std::env::var_os("ORR_WINDOWS_PROJECT_ACCEPTANCE").is_some()
        || std::env::var_os("ORR_WINDOWS_EXPORT_RUNTIME").is_some();
    if !acceptance_requested {
        eprintln!("SKIP: set ORR_REQUIRE_GPU=1 or ORR_WINDOWS_PROJECT_ACCEPTANCE=1 for actual EditorApp acceptance");
        return;
    }
    let gpu = Wgpu::headless(WgpuOptions::default())
        .expect("Windows Room project UI acceptance requires an available GPU");
    eprintln!(
        "Windows project UI independent GPU preflight: {}",
        gpu.adapter_name()
    );
    drop(gpu);

    let fixture = Fixture::new();
    let mut harness = fixture.app();
    settle(&mut harness);
    assert!(harness.state().has_gpu());
    assert_eq!(harness.state().editor.game(), EditorGame::RoomEscape);
    assert_eq!(
        harness
            .state()
            .models
            .placements(&harness.state().editor)
            .len(),
        2
    );
    assert!(harness.state().room_camera.is_some());
    let viewport_gpu = harness
        .state()
        .viewport3d_gpu()
        .expect("production EditorApp viewport GPU")
        .gpu();
    eprintln!(
        "Windows project UI EditorApp viewport GPU: {}",
        viewport_gpu.adapter_name()
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
        "EditorApp viewport should contain rendered pixels"
    );

    let capture_root = std::env::var_os("ORR_WINDOWS_PROJECT_CAPTURE_DIR").map(PathBuf::from);
    if let Some(capture_root) = &capture_root {
        assert!(
            capture_root.is_absolute(),
            "capture directory must be absolute"
        );
        assert!(!capture_root.exists(), "capture directory must be new");
        assert!(
            capture_root.parent().is_some_and(Path::is_dir),
            "capture directory parent must exist"
        );
    }
    let requested_output = if let Some(value) = std::env::var_os("ORR_WINDOWS_PROJECT_OUTPUT") {
        let output = PathBuf::from(value);
        assert!(output.is_absolute(), "project output must be absolute");
        assert!(!output.exists(), "project output must be new");
        output
    } else {
        fixture._temp.path().join("created room project")
    };
    assert!(
        requested_output.parent().is_some_and(Path::is_dir),
        "project output parent must exist"
    );
    let output = requested_output;
    assert!(output.is_absolute());
    menu_click(&mut harness, "File", "New Room project…");
    set_field(
        &mut harness,
        "New Room project directory",
        &output.display().to_string(),
    );
    set_field(&mut harness, "Room project seed", "editor-ui-created");
    click(&mut harness, "Create Room project");
    wait_for(
        &mut harness,
        "new Room project admission and host swap",
        |app| {
            path_matches(
                app.editor.path().as_deref(),
                &output.join("room.scene.yaml"),
            ) && app.editor.game() == EditorGame::RoomEscape
                && app.models.placements(&app.editor).len() == 2
                && app.room_camera.is_some()
        },
    );
    assert!(output.is_dir());
    assert!(path_matches(
        harness.state().editor.path().as_deref(),
        &output.join("room.scene.yaml")
    ));
    assert_eq!(
        harness
            .state()
            .models
            .placements(&harness.state().editor)
            .len(),
        2
    );
    assert!(harness.state().room_camera.is_some());

    let initial_checksum = harness.state().editor.checksum();
    let guid = player_guid(&output);
    harness
        .state_mut()
        .editor
        .select(Some(orr_editor::Target::Guid(guid)));
    settle(&mut harness);
    click(&mut harness, "Read 3D transform");
    click(&mut harness, "Exact 3D transform");
    set_field(&mut harness, "Position X", "-1");
    set_field(&mut harness, "Position Y", "0.5");
    set_field(&mut harness, "Position Z", "0");
    click(&mut harness, "Apply XYZ + rotation");
    settle(&mut harness);
    assert!(harness.state().editor.is_dirty());
    let dirty_checksum = harness.state().editor.checksum();
    set_field(
        &mut harness,
        "Existing Room project directory",
        &output.display().to_string(),
    );
    let before_blocked_open = harness.state().editor.path().unwrap();
    click(&mut harness, "Open Room project");
    settle(&mut harness);
    assert!(
        harness.state().editor.is_dirty(),
        "Open must not discard dirty edits"
    );
    assert_eq!(harness.state().editor.checksum(), dirty_checksum);
    assert_eq!(harness.state().editor.path().unwrap(), before_blocked_open);
    menu_click(&mut harness, "File", "Save Ctrl+S");
    wait_for(&mut harness, "saving the authored scene", |app| {
        !app.editor.is_dirty()
    });
    let saved_checksum = harness.state().editor.checksum();
    assert_ne!(initial_checksum, saved_checksum);
    menu_click(&mut harness, "File", "Open Room project…");
    set_field(
        &mut harness,
        "Existing Room project directory",
        &fixture.root.display().to_string(),
    );
    click(&mut harness, "Open Room project");
    wait_for(
        &mut harness,
        "opening a different admitted Room project",
        |app| {
            !app.windows_projects.busy()
                && path_matches(
                    app.editor.path().as_deref(),
                    &fixture.root.join("room.scene.yaml"),
                )
                && !app.editor.is_dirty()
        },
    );
    set_field(
        &mut harness,
        "Existing Room project directory",
        &output.display().to_string(),
    );
    click(&mut harness, "Open Room project");
    wait_for(&mut harness, "reopening the saved Room project", |app| {
        !app.windows_projects.busy()
            && app
                .windows_projects
                .status
                .starts_with("Opened Room project:")
            && !app.editor.is_dirty()
            && path_matches(
                app.editor.path().as_deref(),
                &output.join("room.scene.yaml"),
            )
            && app.models.placements(&app.editor).len() == 2
            && app.room_camera.is_some()
            && app.editor.checksum() == saved_checksum
    });
    assert_eq!(harness.state().editor.checksum(), saved_checksum);

    click(&mut harness, "▶ Play");
    assert_eq!(harness.state().editor.mode(), Mode::Play);
    click(&mut harness, "⏸ Pause");
    assert!(!harness.state().editor.sim().playing);
    let tick_before_step = harness.state().editor.timeline().unwrap().tick;
    click(&mut harness, "⏭ Step");
    assert!(!harness.state().editor.sim().playing);
    assert_eq!(
        harness.state().editor.timeline().unwrap().tick,
        tick_before_step + 1
    );
    click(&mut harness, "■ Stop");
    assert_eq!(harness.state().editor.mode(), Mode::Edit);

    if let Some(runtime) = std::env::var_os("ORR_WINDOWS_EXPORT_RUNTIME").map(PathBuf::from) {
        assert!(runtime.is_absolute(), "runtime path must be absolute");
        assert!(
            runtime.is_file(),
            "runtime does not exist: {}",
            runtime.display()
        );
        menu_click(&mut harness, "File", "Export Windows game…");
        assert!(path_matches(
            Some(Path::new(&harness.state().windows_export.project)),
            &output
        ));
        set_field(
            &mut harness,
            "Runtime executable",
            &runtime.display().to_string(),
        );
        click(&mut harness, "Check runtime SHA-256");
        wait_for(&mut harness, "trusted runtime measurement", |app| {
            !app.windows_export.busy()
        });
        assert_eq!(harness.state().windows_export.runtime_sha256.len(), 64);
        assert!(!harness.state().windows_export.trusted_runtime);
        click_role(
            &mut harness,
            Role::CheckBox,
            "I trust this runtime executable",
        );
        let export_output = std::env::var_os("ORR_WINDOWS_EXPORT_BUNDLE")
            .map(PathBuf::from)
            .unwrap_or_else(|| fixture._temp.path().join("windows-project-ui-export"));
        assert!(export_output.is_absolute());
        assert!(!export_output.exists(), "export output must be new");
        set_field(
            &mut harness,
            "Export destination",
            &export_output.display().to_string(),
        );
        click_role(&mut harness, Role::Button, "Export Windows game");
        wait_for(&mut harness, "same-app Windows project export", |app| {
            !app.windows_export.busy()
        });
        assert!(
            harness.state().windows_export.last_output.is_some(),
            "export failed: {}",
            harness.state().windows_export.status
        );
        assert!(path_matches(
            harness.state().windows_export.last_output.as_deref(),
            &export_output,
        ));
        let manifest_bytes = fs::read(export_output.join("orr.export.json")).unwrap();
        let manifest: orr_sample::windows_export::ExportManifest =
            serde_json::from_slice(&manifest_bytes).unwrap();
        assert_eq!(
            manifest.payload.initial_checksum,
            format!("0x{saved_checksum:016x}")
        );
        assert_eq!(
            manifest.payload.runtime.sha256,
            harness.state().windows_export.runtime_sha256
        );
        assert_eq!(
            harness.state().windows_export.last_digest.as_deref(),
            Some(manifest.content_digest.as_str()),
        );
    }

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
            Err(error) => panic!("Full EditorApp framebuffer after project workflow: {error}"),
        }
    };
    assert!(frame.width() >= 800 && frame.height() >= 600);
    if let Some(capture_root) = capture_root {
        capture_evidence(harness.state(), &capture_root, &output);
        frame
            .save(capture_root.join("windows-project-ui-complete.png"))
            .unwrap();
    }
}
