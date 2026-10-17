#![cfg(all(
    windows,
    target_arch = "x86_64",
    feature = "windows-projects",
    feature = "room-authoring"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::accesskit::Role;
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{editor::Mode, game::EditorGame, Editor, EditorApp, HostSpec, Target};
use orr_fp::{FPVec3, FP};
use orr_reflect::Value;
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use orr_sample::{
    room_game::{RoomActor, RoomRun, EXIT, KEY, PLAYER},
    room_project::PreparedProject,
};
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
        let root = temp.path().join("initial room project");
        orr_sample::windows_project::create_room_project(&root, "room-authoring-ui-initial")
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
            .with_size([1500.0, 1050.0])
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

fn path_matches(actual: Option<&Path>, expected: &Path) -> bool {
    actual
        .and_then(|path| path.canonicalize().ok())
        .zip(expected.canonicalize().ok())
        .is_some_and(|(actual_path, expected_path)| actual_path == expected_path)
}

fn role_guids(root: &Path) -> BTreeMap<u32, orr_reflect::Guid> {
    let prepared = PreparedProject::open(root).unwrap();
    let scene = prepared.scene();
    let frame = scene.frame();
    let index = scene.index();
    let guids = frame
        .entities()
        .filter_map(|entity| {
            let actor = frame.get::<RoomActor>(entity)?;
            [PLAYER, KEY, EXIT]
                .contains(&actor.kind)
                .then(|| (actor.kind, index.guid(entity).cloned().unwrap()))
        })
        .collect();
    guids
}

fn assert_no_open_transaction(harness: &mut Harness<'_, EditorApp>) {
    let state = harness
        .state_mut()
        .editor
        .host_call("sim.state", serde_json::Value::Null)
        .unwrap();
    assert_eq!(
        state.get("in_tx").and_then(serde_json::Value::as_bool),
        Some(false)
    );
}

fn body_field(app: &mut EditorApp, guid: &orr_reflect::Guid, path: &str) -> Value {
    app.editor
        .field_of(&Target::Guid(guid.clone()), "orr_physics3d::Body", path)
        .unwrap()
}

fn key(harness: &mut Harness<'_, EditorApp>, key: egui::Key, pressed: bool) {
    harness.event(egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
}

fn room_run(app: &EditorApp) -> RoomRun {
    *app.editor
        .snapshot()
        .unwrap()
        .predicted()
        .singleton::<RoomRun>()
}

#[test]
fn room_authoring_ui_layout_wall_save_reopen_and_play_acceptance() {
    let acceptance_requested = std::env::var_os("ORR_REQUIRE_GPU").is_some()
        || std::env::var_os("ORR_ROOM_AUTHORING_ACCEPTANCE").is_some()
        || std::env::var_os("ORR_WINDOWS_EXPORT_RUNTIME").is_some();
    if !acceptance_requested {
        eprintln!(
            "SKIP: set ORR_REQUIRE_GPU=1 or ORR_ROOM_AUTHORING_ACCEPTANCE=1 for actual Room authoring acceptance"
        );
        return;
    }
    let preflight = Wgpu::headless(WgpuOptions::default())
        .expect("Room authoring acceptance requires an available GPU");
    eprintln!(
        "Room authoring independent GPU preflight: {}",
        preflight.adapter_name()
    );
    drop(preflight);

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
    let viewport = harness
        .state()
        .viewport3d_gpu()
        .expect("actual EditorApp 3D viewport")
        .gpu();
    eprintln!(
        "Room authoring EditorApp viewport GPU: {}",
        viewport.adapter_name()
    );
    let viewport_size = viewport.target().size();
    let viewport_pixels = viewport.read_rgba8();
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

    let output = std::env::var_os("ORR_ROOM_AUTHORING_OUTPUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| fixture._temp.path().join("authored room project"));
    assert!(
        output.is_absolute(),
        "Room authoring output must be absolute"
    );
    assert!(
        !output.exists(),
        "Room authoring output must not already exist"
    );
    assert!(output.parent().is_some_and(Path::is_dir));

    menu_click(&mut harness, "File", "New Room project…");
    set_field(
        &mut harness,
        "New Room project directory",
        &output.display().to_string(),
    );
    set_field(
        &mut harness,
        "Room project seed",
        "room-authoring-ui-created",
    );
    click(&mut harness, "Create Room project");
    wait_for(&mut harness, "new project creation", |app| {
        path_matches(
            app.editor.path().as_deref(),
            &output.join("room.scene.yaml"),
        ) && app.editor.game() == EditorGame::RoomEscape
            && app.models.placements(&app.editor).len() == 2
            && app.room_camera.is_some()
    });
    let original_guids = role_guids(&output);
    assert_eq!(original_guids.len(), 3);

    menu_click(&mut harness, "File", "Room level authoring…");
    assert!(harness.query_by_label("Room level authoring").is_some());
    click(&mut harness, "Read Room layout");
    set_field(&mut harness, "Player X", "-1");
    set_field(&mut harness, "Player Z", "0");
    set_field(&mut harness, "Key X", "-1");
    set_field(&mut harness, "Key Z", "-1");
    set_field(&mut harness, "Exit X", "-1");
    set_field(&mut harness, "Exit Z", "-3");
    let initial_checksum = harness.state().editor.checksum();
    set_field(&mut harness, "Player X", "17");
    click(&mut harness, "Apply Room layout");
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    assert!(!harness.state().editor.is_dirty());
    assert_no_open_transaction(&mut harness);
    set_field(&mut harness, "Player X", "-1");
    let original_body = original_guids
        .iter()
        .map(|(kind, guid)| {
            let app = harness.state_mut();
            (
                *kind,
                (body_field(app, guid, "pos"), body_field(app, guid, "rot")),
            )
        })
        .collect::<BTreeMap<_, _>>();
    click(&mut harness, "Apply Room layout");
    let layout_checksum = harness.state().editor.checksum();
    assert_ne!(
        layout_checksum, initial_checksum,
        "layout widget must edit the scene"
    );
    assert!(harness.state().editor.is_dirty());
    for (kind, guid) in &original_guids {
        let (Value::Vec3(original_position), original_rotation) = &original_body[kind] else {
            panic!("Room actor Body.pos must decode as Vec3");
        };
        let app = harness.state_mut();
        let Value::Vec3(position) = body_field(app, guid, "pos") else {
            panic!("edited Room actor Body.pos must decode as Vec3");
        };
        assert_eq!(position.y, original_position.y, "layout preserves actor Y");
        let expected_position = match *kind {
            PLAYER => FPVec3::new(FP::from_int(-1), original_position.y, FP::ZERO),
            KEY => FPVec3::new(FP::from_int(-1), original_position.y, FP::from_int(-1)),
            EXIT => FPVec3::new(FP::from_int(-1), original_position.y, FP::from_int(-3)),
            _ => unreachable!(),
        };
        assert_eq!(position, expected_position);
        assert_eq!(body_field(app, guid, "rot"), *original_rotation);
    }
    menu_click(&mut harness, "Edit", "Undo Ctrl+Z");
    assert_eq!(harness.state().editor.checksum(), initial_checksum);
    menu_click(&mut harness, "Edit", "Redo Ctrl+Y");
    assert_eq!(harness.state().editor.checksum(), layout_checksum);

    set_field(&mut harness, "Room wall name", "ui_authoring_wall");
    set_field(&mut harness, "Wall center X", "2");
    set_field(&mut harness, "Wall center Z", "2");
    set_field(&mut harness, "Wall half-width X", "0.5");
    set_field(&mut harness, "Wall half-depth Z", "0.5");
    let before_invalid_wall = harness.state().editor.checksum();
    let rows_before_invalid_wall = harness.state().editor.rows().len();
    set_field(&mut harness, "Wall half-width X", "0.01");
    click(&mut harness, "Create Room wall");
    assert_eq!(harness.state().editor.checksum(), before_invalid_wall);
    assert_eq!(
        harness.state().editor.rows().len(),
        rows_before_invalid_wall
    );
    assert_no_open_transaction(&mut harness);
    set_field(&mut harness, "Wall half-width X", "0.5");
    click(&mut harness, "Create Room wall");
    let wall_checksum = harness.state().editor.checksum();
    assert_ne!(
        wall_checksum, layout_checksum,
        "wall widget must add a scene entity"
    );
    assert_eq!(harness.state().editor.rows().len(), 8);
    assert!(harness
        .state()
        .editor
        .rows()
        .iter()
        .any(|row| row.name.as_deref() == Some("ui_authoring_wall")));
    let wall_guid = harness
        .state()
        .editor
        .rows()
        .iter()
        .find(|row| row.name.as_deref() == Some("ui_authoring_wall"))
        .and_then(|row| row.guid.clone())
        .expect("created wall has a persistent GUID");
    menu_click(&mut harness, "Edit", "Undo Ctrl+Z");
    assert_eq!(harness.state().editor.checksum(), layout_checksum);
    assert_eq!(harness.state().editor.rows().len(), 7);
    assert!(!harness
        .state()
        .editor
        .rows()
        .iter()
        .any(|row| row.guid.as_ref() == Some(&wall_guid)));
    menu_click(&mut harness, "Edit", "Redo Ctrl+Y");
    assert_eq!(harness.state().editor.checksum(), wall_checksum);
    assert_eq!(harness.state().editor.rows().len(), 8);
    assert!(harness
        .state()
        .editor
        .rows()
        .iter()
        .any(|row| row.guid.as_ref() == Some(&wall_guid)));
    menu_click(&mut harness, "File", "Save Ctrl+S");
    wait_for(&mut harness, "saving the authored Room scene", |app| {
        !app.editor.is_dirty()
    });
    let wall_checksum = harness.state().editor.checksum();
    set_field(
        &mut harness,
        "Existing Room project directory",
        &fixture.root.display().to_string(),
    );
    click(&mut harness, "Open Room project");
    wait_for(&mut harness, "opening the other Room project", |app| {
        !app.windows_projects.busy()
            && path_matches(
                app.editor.path().as_deref(),
                &fixture.root.join("room.scene.yaml"),
            )
    });
    set_field(
        &mut harness,
        "Existing Room project directory",
        &output.display().to_string(),
    );
    click(&mut harness, "Open Room project");
    wait_for(&mut harness, "reopening the authored Room project", |app| {
        !app.windows_projects.busy()
            && app
                .windows_projects
                .status
                .starts_with("Opened Room project:")
            && path_matches(
                app.editor.path().as_deref(),
                &output.join("room.scene.yaml"),
            )
            && !app.editor.is_dirty()
            && app.editor.game() == EditorGame::RoomEscape
            && app.models.placements(&app.editor).len() == 2
            && app.room_camera.is_some()
            && app.editor.rows().len() == 8
            && app
                .editor
                .rows()
                .iter()
                .any(|row| row.name.as_deref() == Some("ui_authoring_wall"))
    });
    let saved_checksum = harness.state().editor.checksum();
    assert_eq!(saved_checksum, wall_checksum);
    let authored_guids = role_guids(&output);
    assert_eq!(authored_guids, original_guids);
    for (kind, guid) in &authored_guids {
        let (Value::Vec3(original_position), original_rotation) = &original_body[kind] else {
            panic!("Room actor Body.pos must decode as Vec3");
        };
        let app = harness.state_mut();
        let Value::Vec3(position) = body_field(app, guid, "pos") else {
            panic!("reopened Room actor Body.pos must decode as Vec3");
        };
        assert_eq!(
            position.y, original_position.y,
            "save/reopen preserves actor Y"
        );
        assert_eq!(position.x, FP::from_int(-1));
        let expected_z = match *kind {
            PLAYER => FP::ZERO,
            KEY => FP::from_int(-1),
            EXIT => FP::from_int(-3),
            _ => unreachable!(),
        };
        assert_eq!(position.z, expected_z);
        assert_eq!(body_field(app, guid, "rot"), *original_rotation);
    }

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

    click(&mut harness, orr_editor::app::LBL_PLAY);
    wait_for(&mut harness, "live Room simulation", |app| {
        app.editor.can_take_control()
    });
    click(&mut harness, "Take control");
    wait_for(&mut harness, "managed input claim", |app| {
        app.editor.input_phase() == orr_editor::editor::input::Phase::Active
    });
    key(&mut harness, egui::Key::E, true);
    wait_for(&mut harness, "key pickup from authored layout", |app| {
        room_run(app).key_collected == 1
    });
    assert_eq!(room_run(harness.state()).won, 0);
    key(&mut harness, egui::Key::E, false);
    key(&mut harness, egui::Key::W, true);
    wait_for(&mut harness, "player approaches authored exit", |app| {
        app.models.placements(&app.editor).iter().any(|placement| {
            placement.entity
                == app
                    .editor
                    .rows()
                    .iter()
                    .find(|row| row.guid.as_ref() == authored_guids.get(&PLAYER))
                    .unwrap()
                    .entity
                && placement.instance.translation[2] < -2.0
        })
    });
    key(&mut harness, egui::Key::W, false);
    key(&mut harness, egui::Key::E, true);
    wait_for(&mut harness, "exit interaction wins", |app| {
        room_run(app).won == 1
    });
    key(&mut harness, egui::Key::E, false);
    click(&mut harness, orr_editor::app::LBL_PAUSE);
    assert!(!harness.state().editor.sim().playing);
    click(&mut harness, orr_editor::app::LBL_STOP);
    assert_eq!(harness.state().editor.mode(), Mode::Edit);
    assert_eq!(harness.state().editor.checksum(), saved_checksum);

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
        wait_for(&mut harness, "trusted runtime measurement", |app| {
            !app.windows_export.busy()
        });
        assert_eq!(harness.state().windows_export.runtime_sha256.len(), 64);
        click_role(
            &mut harness,
            Role::CheckBox,
            "I trust this runtime executable",
        );
        let export_output = std::env::var_os("ORR_ROOM_AUTHORING_EXPORT_BUNDLE")
            .map(PathBuf::from)
            .unwrap_or_else(|| fixture._temp.path().join("room-authoring-export"));
        assert!(export_output.is_absolute());
        assert!(!export_output.exists());
        set_field(
            &mut harness,
            "Export destination",
            &export_output.display().to_string(),
        );
        click_role(&mut harness, Role::Button, "Export Windows game");
        wait_for(&mut harness, "same-App authored Room export", |app| {
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
            manifest.payload.initial_checksum,
            format!("0x{saved_checksum:016x}")
        );
        assert_eq!(
            manifest.payload.runtime.sha256,
            harness.state().windows_export.runtime_sha256
        );
        assert_eq!(
            harness.state().windows_export.last_digest.as_deref(),
            Some(manifest.content_digest.as_str())
        );
    }

    if let Some(capture_root) =
        std::env::var_os("ORR_ROOM_AUTHORING_CAPTURE_DIR").map(PathBuf::from)
    {
        assert!(capture_root.is_absolute());
        fs::create_dir(&capture_root).expect("capture directory must be new");
        let mut render_retries = 0;
        let frame = loop {
            match harness.render() {
                Ok(frame) => break frame,
                Err(error) if error.contains("requested Wait timed out") && render_retries < 2 => {
                    render_retries += 1;
                    eprintln!("Room authoring framebuffer Wait timeout; retry {render_retries}/2: {error}");
                    harness.run_steps(2);
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(error) => panic!("actual EditorApp final framebuffer: {error}"),
            }
        };
        frame
            .save(capture_root.join("room-authoring-ui.png"))
            .unwrap();
        fs::write(
            capture_root.join("acceptance.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "project": output,
                "checksum": format!("0x{saved_checksum:016x}"),
                "actors": authored_guids.iter().map(|(kind,guid)|(*kind,guid.to_string())).collect::<BTreeMap<_,_>>(),
                "wall": "ui_authoring_wall",
                "model_placements": 2,
                "camera_installed": true,
            }))
            .unwrap(),
        )
        .unwrap();
    }
}
