#![cfg(all(windows, target_arch = "x86_64", feature = "windows-projects"))]
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
    Editor, EditorApp, HostSpec, Owner, Target,
};
use orr_reflect::Value;
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use orr_sample::{
    room_game::{RoomActor, PLAYER},
    room_project::PreparedProject,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

struct Fixture {
    _temp: tempfile::TempDir,
    physics_scene: PathBuf,
    physics_bytes: Vec<u8>,
    output: PathBuf,
    second_output: PathBuf,
    malformed_project: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = EditorGame::PhysGame.default_scene_path();
        let physics_scene = temp.path().join("owned physics demo.scene.yaml");
        fs::copy(&source, &physics_scene).expect("copy the stock Physics scene for this test");
        let physics_bytes = fs::read(&physics_scene).unwrap();
        let output = std::env::var_os("ORR_ROOM_BOOTSTRAP_OUTPUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| temp.path().join("bootstrapped Room project"));
        assert!(output.is_absolute());
        assert!(!output.exists(), "Room project output must be new");
        assert!(output.parent().is_some_and(Path::is_dir));
        let second_output = temp.path().join("second Room project");
        let malformed_project = temp.path().join("malformed Room project");
        fs::create_dir(&malformed_project).unwrap();
        fs::write(
            malformed_project.join("orr.project.json"),
            b"not valid project metadata",
        )
        .unwrap();
        Self {
            _temp: temp,
            physics_scene,
            physics_bytes,
            output,
            second_output,
            malformed_project,
        }
    }

    fn app(&self) -> Harness<'static, EditorApp> {
        let mut editor = Editor::start(&HostSpec::local_game(
            self.physics_scene.clone(),
            EditorGame::PhysGame,
        ))
        .unwrap();
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
    assert!(
        harness.query_by_label(item).is_some(),
        "missing menu item {item}"
    );
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
    let player = frame
        .entities()
        .find_map(|entity| {
            let actor = frame.get::<RoomActor>(entity)?;
            (actor.kind == PLAYER).then(|| index.guid(entity).cloned().unwrap())
        })
        .expect("admitted Room project contains a PLAYER");
    player
}

#[test]
fn windows_room_project_bootstraps_from_clean_physics_editor() {
    let acceptance_requested = [
        "ORR_REQUIRE_GPU",
        "ORR_ROOM_BOOTSTRAP_ACCEPTANCE",
        "ORR_ROOM_BOOTSTRAP_OUTPUT",
        "ORR_ROOM_BOOTSTRAP_CAPTURE_DIR",
        "ORR_ROOM_BOOTSTRAP_EXPORT_BUNDLE",
        "ORR_WINDOWS_EXPORT_RUNTIME",
    ]
    .iter()
    .any(|name| std::env::var_os(name).is_some());
    if !acceptance_requested {
        eprintln!(
            "SKIP: set ORR_REQUIRE_GPU=1 or ORR_ROOM_BOOTSTRAP_ACCEPTANCE=1 for actual startup acceptance"
        );
        return;
    }
    assert_eq!(
        std::env::var("WGPU_BACKEND").unwrap_or_default(),
        "vulkan",
        "Room bootstrap acceptance requires WGPU_BACKEND=vulkan"
    );
    let gpu = Wgpu::headless(WgpuOptions::default())
        .expect("Room bootstrap acceptance requires a hardware Vulkan GPU");
    assert!(
        !gpu.is_software(),
        "software adapter is not acceptance evidence"
    );
    let hardware_adapter = gpu.adapter_name();
    eprintln!("Room bootstrap GPU preflight: {hardware_adapter}");
    drop(gpu);

    let fixture = Fixture::new();
    let mut harness = fixture.app();
    settle(&mut harness);
    assert_eq!(harness.state().editor.game(), EditorGame::PhysGame);
    assert_eq!(
        fs::read(&fixture.physics_scene).unwrap(),
        fixture.physics_bytes,
        "the initial Physics scene starts unmodified"
    );
    let initial_physics_path = harness.state().editor.path().unwrap();
    let initial_physics_checksum = harness.state().editor.checksum();
    let viewport = harness
        .state()
        .viewport_gpu()
        .expect("production EditorApp Physics viewport")
        .gpu();
    let viewport_adapter = viewport.adapter_name();
    eprintln!("Room bootstrap EditorApp GPU: {viewport_adapter}");
    assert_eq!(
        viewport_adapter, hardware_adapter,
        "EditorApp selected the independently verified hardware adapter"
    );
    let initial_viewport_size = viewport.target().size();
    assert!(initial_viewport_size.0 > 0 && initial_viewport_size.1 > 0);

    menu_click(&mut harness, "File", "Open Room project…");
    set_field(
        &mut harness,
        "Existing Room project directory",
        &fixture.malformed_project.display().to_string(),
    );
    click(&mut harness, "Open Room project");
    wait_for(&mut harness, "rejecting malformed Room project", |app| {
        !app.windows_projects.busy()
            && app
                .windows_projects
                .status
                .starts_with("Room project operation failed:")
    });
    assert_eq!(harness.state().editor.game(), EditorGame::PhysGame);
    assert_eq!(
        harness.state().editor.path().as_ref(),
        Some(&initial_physics_path)
    );
    assert_eq!(harness.state().editor.checksum(), initial_physics_checksum);
    assert_eq!(
        fs::read(&fixture.physics_scene).unwrap(),
        fixture.physics_bytes
    );

    assert!(harness.state_mut().editor.select_named("body_05"));
    assert!(harness.state_mut().editor.set_field(
        &Owner::Component("orr_physics::Body".into()),
        "pos.x",
        Value::Fixed(orr_fp::FP::from_int(1)),
    ));
    settle(&mut harness);
    assert!(harness.state().editor.is_dirty());
    let dirty_physics_checksum = harness.state().editor.checksum();
    menu_click(&mut harness, "File", "New Room project…");
    set_field(
        &mut harness,
        "New Room project directory",
        &fixture.output.display().to_string(),
    );
    set_field(&mut harness, "Room project seed", "phys-bootstrap-ui");
    set_field(
        &mut harness,
        "Existing Room project directory",
        &fixture.malformed_project.display().to_string(),
    );
    settle(&mut harness);
    assert!(harness
        .get_by_role_and_label(Role::Button, "Create Room project")
        .accesskit_node()
        .is_disabled());
    assert!(harness
        .get_by_role_and_label(Role::Button, "Open Room project")
        .accesskit_node()
        .is_disabled());
    assert!(!fixture.output.exists());
    assert_eq!(harness.state().editor.game(), EditorGame::PhysGame);
    assert_eq!(harness.state().editor.checksum(), dirty_physics_checksum);
    assert_eq!(
        fs::read(&fixture.physics_scene).unwrap(),
        fixture.physics_bytes
    );
    menu_click(&mut harness, "Edit", "Undo Ctrl+Z");
    wait_for(&mut harness, "undoing the Physics edit", |app| {
        !app.editor.is_dirty()
    });
    assert_eq!(harness.state().editor.checksum(), initial_physics_checksum);

    click(&mut harness, "Create Room project");
    wait_for(&mut harness, "creating and opening Room project", |app| {
        !app.windows_projects.busy()
            && app
                .windows_projects
                .status
                .starts_with("Opened Room project:")
            && app.editor.game() == EditorGame::RoomEscape
            && path_matches(
                app.editor.path().as_deref(),
                &fixture.output.join("room.scene.yaml"),
            )
            && app.models.placements(&app.editor).len() == 2
            && app.room_camera.is_some()
    });
    assert_eq!(
        fs::read(&fixture.physics_scene).unwrap(),
        fixture.physics_bytes,
        "switching to Room leaves the source Physics scene untouched"
    );
    let initial_room_checksum = harness.state().editor.checksum();
    let player = player_guid(&fixture.output);
    harness
        .state_mut()
        .editor
        .select(Some(Target::Guid(player)));
    settle(&mut harness);
    click(&mut harness, "Read 3D transform");
    click(&mut harness, "Exact 3D transform");
    set_field(&mut harness, "Position X", "-1");
    set_field(&mut harness, "Position Y", "0.5");
    set_field(&mut harness, "Position Z", "0");
    click(&mut harness, "Apply XYZ + rotation");
    assert!(harness.state().editor.is_dirty());
    let authored_checksum = harness.state().editor.checksum();
    assert_ne!(authored_checksum, initial_room_checksum);
    menu_click(&mut harness, "File", "Save Ctrl+S");
    wait_for(&mut harness, "saving authored Room layout", |app| {
        !app.editor.is_dirty()
    });
    let saved_checksum = harness.state().editor.checksum();
    assert_eq!(saved_checksum, authored_checksum);

    menu_click(&mut harness, "File", "New Room project…");
    set_field(
        &mut harness,
        "New Room project directory",
        &fixture.second_output.display().to_string(),
    );
    set_field(&mut harness, "Room project seed", "phys-bootstrap-second");
    click(&mut harness, "Create Room project");
    wait_for(&mut harness, "creating second Room project", |app| {
        !app.windows_projects.busy()
            && app.editor.game() == EditorGame::RoomEscape
            && path_matches(
                app.editor.path().as_deref(),
                &fixture.second_output.join("room.scene.yaml"),
            )
    });
    menu_click(&mut harness, "File", "Open Room project…");
    set_field(
        &mut harness,
        "Existing Room project directory",
        &fixture.output.display().to_string(),
    );
    click(&mut harness, "Open Room project");
    wait_for(&mut harness, "reopening authored Room project", |app| {
        !app.windows_projects.busy()
            && app
                .windows_projects
                .status
                .starts_with("Opened Room project:")
            && app.editor.game() == EditorGame::RoomEscape
            && !app.editor.is_dirty()
            && app.editor.checksum() == saved_checksum
            && path_matches(
                app.editor.path().as_deref(),
                &fixture.output.join("room.scene.yaml"),
            )
            && app.models.placements(&app.editor).len() == 2
            && app.room_camera.is_some()
    });
    let reopened = PreparedProject::open(&fixture.output).unwrap();
    assert_eq!(reopened.scene().frame().checksum(), saved_checksum);
    assert!(reopened.camera().is_some());

    click(&mut harness, LBL_PLAY);
    assert_eq!(harness.state().editor.mode(), Mode::Play);
    click(&mut harness, LBL_PAUSE);
    assert!(!harness.state().editor.sim().playing);
    let tick_before_step = harness.state().editor.timeline().unwrap().tick;
    click(&mut harness, LBL_STEP);
    assert!(!harness.state().editor.sim().playing);
    assert_eq!(
        harness.state().editor.timeline().unwrap().tick,
        tick_before_step + 1
    );
    click(&mut harness, LBL_STOP);
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
        wait_for(&mut harness, "measuring trusted Windows runtime", |app| {
            !app.windows_export.busy()
        });
        assert_eq!(harness.state().windows_export.runtime_sha256.len(), 64);
        click_role(
            &mut harness,
            Role::CheckBox,
            "I trust this runtime executable",
        );
        let bundle = std::env::var_os("ORR_ROOM_BOOTSTRAP_EXPORT_BUNDLE")
            .map(PathBuf::from)
            .unwrap_or_else(|| fixture._temp.path().join("Room bootstrap export"));
        assert!(bundle.is_absolute());
        assert!(!bundle.exists());
        set_field(
            &mut harness,
            "Export destination",
            &bundle.display().to_string(),
        );
        click_role(&mut harness, Role::Button, "Export Windows game");
        wait_for(&mut harness, "same-App Room export", |app| {
            !app.windows_export.busy()
        });
        assert!(path_matches(
            harness.state().windows_export.last_output.as_deref(),
            &bundle
        ));
        let manifest: orr_sample::windows_export::ExportManifest =
            serde_json::from_slice(&fs::read(bundle.join("orr.export.json")).unwrap()).unwrap();
        assert_eq!(
            manifest.payload.initial_checksum,
            format!("0x{saved_checksum:016x}")
        );
        assert_eq!(
            harness.state().windows_export.last_digest.as_deref(),
            Some(manifest.content_digest.as_str())
        );
    } else {
        assert!(
            std::env::var_os("ORR_ROOM_BOOTSTRAP_EXPORT_BUNDLE").is_none(),
            "an export bundle requires ORR_WINDOWS_EXPORT_RUNTIME"
        );
    }

    if let Some(capture_root) = std::env::var_os("ORR_ROOM_BOOTSTRAP_CAPTURE_DIR") {
        let capture_root = PathBuf::from(capture_root);
        assert!(capture_root.is_absolute());
        assert!(!capture_root.exists());
        assert!(capture_root.parent().is_some_and(Path::is_dir));
        fs::create_dir(&capture_root).unwrap();
        let frame = harness
            .render()
            .expect("capture the actual hardware-rendered bootstrap EditorApp");
        assert!(frame.width() >= 800 && frame.height() >= 600);
        frame
            .save(capture_root.join("windows-room-bootstrap-ui.png"))
            .unwrap();
        fs::write(
            capture_root.join("acceptance.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "source_physics_scene": fixture.physics_scene,
                "source_physics_scene_preserved": fs::read(&fixture.physics_scene).unwrap() == fixture.physics_bytes,
                "room_project": fixture.output,
                "game": "RoomEscape",
                "scene_checksum": format!("0x{saved_checksum:016x}"),
                "models": 2,
                "camera_installed": harness.state().room_camera.is_some(),
            }))
            .unwrap(),
        )
        .unwrap();
    }
}
