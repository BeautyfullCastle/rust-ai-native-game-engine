#![cfg(all(
    windows,
    target_arch = "x86_64",
    feature = "windows-projects",
    feature = "room-authoring"
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
    editor::{input::Phase, Mode},
    game::EditorGame,
    Editor, EditorApp, HostSpec, Target,
};
use orr_fp::{FPVec3, FP};
use orr_reflect::Value;
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use orr_sample::{
    room_game::{RoomActor, RoomRun, PLAYER},
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
    other_output: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let physics_scene = temp.path().join("wall iteration Physics scene.yaml");
        fs::copy(EditorGame::PhysGame.default_scene_path(), &physics_scene)
            .expect("copy the stock Physics scene into an owned fixture");
        let physics_bytes = fs::read(&physics_scene).unwrap();
        let output = std::env::var_os("ORR_ROOM_WALL_ITERATION_OUTPUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| temp.path().join("Room wall iteration project"));
        let other_output = temp.path().join("other Room wall project");
        assert!(output.is_absolute());
        assert!(!output.exists(), "Room project output must be new");
        assert!(output.parent().is_some_and(Path::is_dir));
        Self {
            _temp: temp,
            physics_scene,
            physics_bytes,
            output,
            other_output,
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
            .with_size([1800.0, 1250.0])
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
    harness.run_steps(30);
    harness.get_by_label(label).click_accesskit();
    settle(harness);
}

fn click_role(harness: &mut Harness<'_, EditorApp>, role: Role, label: &str) {
    harness.get_by_role_and_label(role, label).scroll_to_me();
    harness.run_steps(30);
    harness.get_by_role_and_label(role, label).click_accesskit();
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
    harness.run_steps(30);
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

fn wall_guid(harness: &EditorApp, name: &str) -> orr_reflect::Guid {
    harness
        .editor
        .rows()
        .iter()
        .find(|row| row.name.as_deref() == Some(name))
        .and_then(|row| row.guid.clone())
        .unwrap_or_else(|| panic!("Room wall {name} has no persistent GUID"))
}

fn component_value(
    harness: &mut Harness<'_, EditorApp>,
    guid: &orr_reflect::Guid,
    component: &str,
) -> Value {
    harness
        .state_mut()
        .editor
        .field_of(&Target::Guid(guid.clone()), component, "")
        .unwrap()
}

fn without_field(value: &Value, excluded_field: &str) -> Value {
    let Value::Struct(fields) = value else {
        panic!("expected reflected struct value, got {value:?}");
    };
    Value::Struct(
        fields
            .iter()
            .filter(|(name, _)| name != excluded_field)
            .cloned()
            .collect(),
    )
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

fn room_run(app: &EditorApp) -> RoomRun {
    *app.editor
        .snapshot()
        .unwrap()
        .predicted()
        .singleton::<RoomRun>()
}

fn player_x(app: &EditorApp, guid: &orr_reflect::Guid) -> f32 {
    let player = app
        .editor
        .rows()
        .iter()
        .find(|row| row.guid.as_ref() == Some(guid))
        .expect("PLAYER GUID remains in the scene")
        .entity;
    app.models
        .placements(&app.editor)
        .into_iter()
        .find(|placement| placement.entity == player)
        .expect("PLAYER keeps its admitted model placement")
        .instance
        .translation[0]
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

#[test]
fn room_wall_iteration_ui_lifecycle_and_collision_acceptance() {
    let acceptance_requested = [
        "ORR_REQUIRE_GPU",
        "ORR_ROOM_WALL_ITERATION_ACCEPTANCE",
        "ORR_ROOM_WALL_ITERATION_OUTPUT",
        "ORR_ROOM_WALL_ITERATION_CAPTURE_DIR",
        "ORR_ROOM_WALL_ITERATION_EXPORT_BUNDLE",
        "ORR_WINDOWS_EXPORT_RUNTIME",
    ]
    .iter()
    .any(|name| std::env::var_os(name).is_some());
    if !acceptance_requested {
        eprintln!(
            "SKIP: set ORR_REQUIRE_GPU=1 or ORR_ROOM_WALL_ITERATION_ACCEPTANCE=1 for actual Room wall UI acceptance"
        );
        return;
    }
    assert_eq!(
        std::env::var("WGPU_BACKEND").unwrap_or_default(),
        "vulkan",
        "Room wall UI acceptance requires WGPU_BACKEND=vulkan"
    );
    let preflight = Wgpu::headless(WgpuOptions::default())
        .expect("Room wall acceptance requires a hardware Vulkan GPU");
    assert!(
        !preflight.is_software(),
        "software GPU is not acceptance evidence"
    );
    let hardware_adapter = preflight.adapter_name();
    eprintln!("Room wall UI GPU preflight: {hardware_adapter}");
    drop(preflight);

    let fixture = Fixture::new();
    let mut harness = fixture.app();
    settle(&mut harness);
    assert_eq!(harness.state().editor.game(), EditorGame::PhysGame);
    assert_eq!(
        fs::read(&fixture.physics_scene).unwrap(),
        fixture.physics_bytes,
        "the copied Physics source starts unchanged"
    );
    menu_click(&mut harness, "File", "New Room project…");
    set_field(
        &mut harness,
        "New Room project directory",
        &fixture.output.display().to_string(),
    );
    set_field(&mut harness, "Room project seed", "wall-iteration-ui");
    click(&mut harness, "Create Room project");
    wait_for(&mut harness, "creating the Room authoring project", |app| {
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
    let initial_roles = {
        let player = player_guid(&fixture.output);
        let prepared = PreparedProject::open(&fixture.output).unwrap();
        let scene = prepared.scene();
        let frame = scene.frame();
        let index = scene.index();
        let roles = frame
            .entities()
            .filter_map(|entity| {
                let actor = frame.get::<RoomActor>(entity)?;
                ([
                    PLAYER,
                    orr_sample::room_game::KEY,
                    orr_sample::room_game::EXIT,
                ]
                .contains(&actor.kind))
                .then(|| (actor.kind, index.guid(entity).cloned().unwrap()))
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(roles.get(&PLAYER), Some(&player));
        roles
    };
    let player = initial_roles[&PLAYER].clone();
    let initial_body_values = [
        PLAYER,
        orr_sample::room_game::KEY,
        orr_sample::room_game::EXIT,
    ]
    .into_iter()
    .map(|kind| {
        let guid = &initial_roles[&kind];
        let body = harness
            .state_mut()
            .editor
            .field_of(&Target::Guid(guid.clone()), "orr_physics3d::Body", "")
            .unwrap();
        (kind, body)
    })
    .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(
        fs::read(&fixture.physics_scene).unwrap(),
        fixture.physics_bytes
    );
    let viewport = harness
        .state()
        .viewport3d_gpu()
        .expect("production EditorApp Room viewport")
        .gpu();
    let viewport_adapter = viewport.adapter_name();
    eprintln!("Room wall EditorApp viewport GPU: {viewport_adapter}");
    assert_eq!(viewport_adapter, hardware_adapter);
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

    menu_click(&mut harness, "File", "Room level authoring…");
    click(&mut harness, "Manage existing Room walls");
    set_field(&mut harness, "Room wall name", "iteration_wall");
    set_field(&mut harness, "Wall center X", "2");
    set_field(&mut harness, "Wall center Z", "2");
    set_field(&mut harness, "Wall half-width X", "0.5");
    set_field(&mut harness, "Wall half-depth Z", "0.5");
    click(&mut harness, "Create Room wall");
    let original_wall = wall_guid(harness.state(), "iteration_wall");
    assert_eq!(
        harness.state().room_authoring.status,
        format!("Created wall {original_wall}; read 5 Room wall(s)."),
        "unexpected Room wall creation status"
    );
    assert_eq!(harness.state().editor.rows().len(), 8);
    click(&mut harness, "Read Room walls");
    assert_eq!(
        harness.state().room_authoring.status,
        "Read 5 Room wall(s).",
        "unexpected Room wall read status"
    );
    let original_wall_label = format!("iteration_wall · {original_wall}");
    click(&mut harness, &original_wall_label);
    assert!(!harness
        .get_by_role_and_label(Role::Button, "Read selected Room wall")
        .accesskit_node()
        .is_disabled());
    click(&mut harness, "Read selected Room wall");
    assert_eq!(
        harness.state().room_authoring.status,
        format!("Read Room wall {original_wall}."),
        "unexpected selected Room wall status"
    );
    let original_actor = component_value(&mut harness, &original_wall, "RoomEscapeV1::Actor");
    let original_body = component_value(&mut harness, &original_wall, "orr_physics3d::Body");
    let original_collider =
        component_value(&mut harness, &original_wall, "orr_physics3d::Collider");
    let Value::Vec3(original_position) = original_body.field("pos").unwrap() else {
        panic!("wall Body.pos must remain a Vec3");
    };
    assert_eq!(
        *original_position,
        FPVec3::new(FP::from_int(2), FP::HALF, FP::from_int(2))
    );
    assert_eq!(
        harness
            .state_mut()
            .editor
            .field_of(
                &Target::Guid(original_wall.clone()),
                "orr_physics3d::Collider",
                "shape.half_extents",
            )
            .unwrap(),
        Value::Vec3(FPVec3::new(FP::HALF, FP::ONE, FP::HALF))
    );
    let create_checksum = harness.state().editor.checksum();
    let create_history_count = harness.state().editor.history().entries.len();
    set_field(&mut harness, "Selected wall name", "iteration_wall_edited");
    set_field(&mut harness, "Selected wall center X", "2.25");
    set_field(&mut harness, "Selected wall center Z", "2.5");
    set_field(&mut harness, "Selected wall half-width X", "0.3");
    set_field(&mut harness, "Selected wall half-depth Z", "0.7");
    let wall_draft = [
        "Selected wall name",
        "Selected wall center X",
        "Selected wall center Z",
        "Selected wall half-width X",
        "Selected wall half-depth Z",
    ]
    .map(|label| {
        harness
            .get_by_role_and_label(Role::TextInput, label)
            .value()
    });
    assert_eq!(
        wall_draft,
        [
            Some("iteration_wall_edited".into()),
            Some("2.25".into()),
            Some("2.5".into()),
            Some("0.3".into()),
            Some("0.7".into()),
        ],
        "the wall edit widgets retain the requested draft values"
    );
    assert!(
        !harness
            .get_by_role_and_label(Role::Button, "Apply Room wall")
            .accesskit_node()
            .is_disabled(),
        "Apply Room wall must be enabled before submit; status={:?}; draft={wall_draft:?}",
        harness.state().room_authoring.status
    );
    click(&mut harness, "Apply Room wall");
    let edited_checksum = harness.state().editor.checksum();
    assert_ne!(
        edited_checksum,
        create_checksum,
        "Apply Room wall did not change the scene; status={:?}; draft={wall_draft:?}",
        harness.state().room_authoring.status
    );
    assert_eq!(
        harness.state().editor.history().entries.len(),
        create_history_count + 1,
        "rename, move and resize are one scene-history edit"
    );
    assert_eq!(
        wall_guid(harness.state(), "iteration_wall_edited"),
        original_wall
    );
    let updated_body = component_value(&mut harness, &original_wall, "orr_physics3d::Body");
    let updated_collider = component_value(&mut harness, &original_wall, "orr_physics3d::Collider");
    let updated_actor = component_value(&mut harness, &original_wall, "RoomEscapeV1::Actor");
    let Value::Vec3(updated_position) = updated_body.field("pos").unwrap() else {
        panic!("wall Body.pos must remain a Vec3");
    };
    assert_eq!(
        *updated_position,
        FPVec3::new(FP::from_ratio(9, 4), FP::HALF, FP::from_ratio(5, 2))
    );
    assert_eq!(
        without_field(&updated_body, "pos"),
        without_field(&original_body, "pos")
    );
    assert_eq!(updated_actor, original_actor);
    assert_eq!(
        without_field(&updated_collider, "shape"),
        without_field(&original_collider, "shape")
    );
    assert_eq!(
        harness
            .state_mut()
            .editor
            .field_of(
                &Target::Guid(original_wall.clone()),
                "orr_physics3d::Collider",
                "shape.half_extents",
            )
            .unwrap(),
        Value::Vec3(FPVec3::new(
            FP::from_ratio(3, 10),
            FP::ONE,
            FP::from_ratio(7, 10)
        ))
    );
    menu_click(&mut harness, "Edit", "Undo Ctrl+Z");
    assert_eq!(harness.state().editor.checksum(), create_checksum);
    assert_eq!(wall_guid(harness.state(), "iteration_wall"), original_wall);
    menu_click(&mut harness, "Edit", "Redo Ctrl+Y");
    assert_eq!(harness.state().editor.checksum(), edited_checksum);
    assert_eq!(
        wall_guid(harness.state(), "iteration_wall_edited"),
        original_wall
    );

    let before_invalid_checksum = harness.state().editor.checksum();
    let before_invalid_count = harness.state().editor.rows().len();
    let before_invalid_history = harness.state().editor.history().entries.len();
    set_field(&mut harness, "Selected wall half-width X", "0.01");
    click(&mut harness, "Apply Room wall");
    assert_eq!(harness.state().editor.checksum(), before_invalid_checksum);
    assert_eq!(harness.state().editor.rows().len(), before_invalid_count);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        before_invalid_history
    );
    assert_no_open_transaction(&mut harness);
    click(&mut harness, "Read Room walls");
    click(
        &mut harness,
        &format!("iteration_wall_edited · {original_wall}"),
    );
    click(&mut harness, "Read selected Room wall");
    set_field(&mut harness, "Selected wall half-width X", "0.3");

    click(&mut harness, "Read Room walls");
    click(
        &mut harness,
        &format!("iteration_wall_edited · {original_wall}"),
    );
    set_field(&mut harness, "Duplicate wall name", "collision_barrier");
    set_field(&mut harness, "Duplicate wall center X", "0.8");
    set_field(&mut harness, "Duplicate wall center Z", "0");
    let before_duplicate_checksum = harness.state().editor.checksum();
    let before_duplicate_history = harness.state().editor.history().entries.len();
    click(&mut harness, "Duplicate Room wall");
    let duplicate_wall = wall_guid(harness.state(), "collision_barrier");
    assert_ne!(
        duplicate_wall, original_wall,
        "duplicate receives a fresh GUID"
    );
    let duplicate_body = component_value(&mut harness, &duplicate_wall, "orr_physics3d::Body");
    let duplicate_collider =
        component_value(&mut harness, &duplicate_wall, "orr_physics3d::Collider");
    let duplicate_actor = component_value(&mut harness, &duplicate_wall, "RoomEscapeV1::Actor");
    let Value::Vec3(duplicate_position) = duplicate_body.field("pos").unwrap() else {
        panic!("duplicate wall Body.pos must remain a Vec3");
    };
    assert_eq!(
        *duplicate_position,
        FPVec3::new(FP::from_ratio(8, 10), FP::HALF, FP::ZERO)
    );
    assert_eq!(
        without_field(&duplicate_body, "pos"),
        without_field(&updated_body, "pos")
    );
    assert_eq!(duplicate_collider, updated_collider);
    assert_eq!(duplicate_actor, updated_actor);
    assert_eq!(harness.state().editor.rows().len(), 9);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        before_duplicate_history + 1
    );
    let duplicated_checksum = harness.state().editor.checksum();
    menu_click(&mut harness, "Edit", "Undo Ctrl+Z");
    assert_eq!(harness.state().editor.checksum(), before_duplicate_checksum);
    assert_eq!(harness.state().editor.rows().len(), 8);
    assert!(!harness
        .state()
        .editor
        .rows()
        .iter()
        .any(|row| row.guid.as_ref() == Some(&duplicate_wall)));
    menu_click(&mut harness, "Edit", "Redo Ctrl+Y");
    assert_eq!(harness.state().editor.checksum(), duplicated_checksum);
    assert_eq!(
        wall_guid(harness.state(), "collision_barrier"),
        duplicate_wall
    );
    assert_eq!(
        wall_guid(harness.state(), "iteration_wall_edited"),
        original_wall
    );

    menu_click(&mut harness, "File", "Save Ctrl+S");
    wait_for(&mut harness, "saving the wall edit and duplicate", |app| {
        !app.editor.is_dirty()
    });
    let saved_duplicate_checksum = harness.state().editor.checksum();
    click(&mut harness, "Read Room walls");
    click(
        &mut harness,
        &format!("iteration_wall_edited · {original_wall}"),
    );
    click(&mut harness, "Read selected Room wall");
    if harness.query_by_label("Load model asset").is_none() {
        click(&mut harness, "Static model binding");
    }
    harness.state_mut().models.package = "sample-imported-scene".into();
    harness.state_mut().models.asset = "foreground.glb".into();
    click(&mut harness, "Load model asset");
    assert!(harness.state().models.error().is_none());
    click(&mut harness, "Assign static model");
    let original_wall_key = original_wall.to_string();
    assert!(harness
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings
        .contains_key(&original_wall_key));
    assert!(harness.state().models.bindings.as_ref().unwrap().dirty());
    assert!(
        harness
            .get_by_role_and_label(Role::Button, "Remove selected Room wall")
            .accesskit_node()
            .is_disabled(),
        "unsaved model sidecar blocks wall removal"
    );
    let bound_wall_checksum = harness.state().editor.checksum();
    let bound_wall_rows = harness.state().editor.rows().len();
    click(&mut harness, "Save model bindings");
    assert!(!harness.state().models.bindings.as_ref().unwrap().dirty());
    click(&mut harness, "I confirm removing this Room wall");
    assert!(!harness
        .get_by_role_and_label(Role::Button, "Remove selected Room wall")
        .accesskit_node()
        .is_disabled());
    let bound_wall_history = harness.state().editor.history().entries.len();
    click(&mut harness, "Remove selected Room wall");
    assert_eq!(harness.state().editor.checksum(), bound_wall_checksum);
    assert_eq!(harness.state().editor.rows().len(), bound_wall_rows);
    assert_eq!(
        harness.state().editor.history().entries.len(),
        bound_wall_history
    );
    assert!(harness
        .state()
        .room_authoring
        .status
        .contains("model binding"));
    assert_no_open_transaction(&mut harness);
    click(&mut harness, "I confirm removing this Room wall");
    assert!(harness
        .get_by_role_and_label(Role::Button, "Remove selected Room wall")
        .accesskit_node()
        .is_disabled());
    click(&mut harness, "Remove model");
    assert!(harness.state().models.bindings.as_ref().unwrap().dirty());
    click(&mut harness, "Save model bindings");
    assert!(!harness.state().models.bindings.as_ref().unwrap().dirty());
    click(&mut harness, "Read Room walls");
    click(
        &mut harness,
        &format!("iteration_wall_edited · {original_wall}"),
    );
    click(&mut harness, "Read selected Room wall");
    click(&mut harness, "I confirm removing this Room wall");
    assert!(!harness
        .get_by_role_and_label(Role::Button, "Remove selected Room wall")
        .accesskit_node()
        .is_disabled());
    click(&mut harness, "Remove selected Room wall");
    let without_original_wall = harness.state().editor.checksum();
    assert_ne!(without_original_wall, saved_duplicate_checksum);
    assert_eq!(harness.state().editor.rows().len(), 8);
    assert!(!harness
        .state()
        .editor
        .rows()
        .iter()
        .any(|row| row.guid.as_ref() == Some(&original_wall)));
    assert!(harness
        .state()
        .editor
        .rows()
        .iter()
        .any(|row| row.guid.as_ref() == Some(&duplicate_wall)));
    assert_no_open_transaction(&mut harness);
    menu_click(&mut harness, "Edit", "Undo Ctrl+Z");
    assert_eq!(harness.state().editor.checksum(), saved_duplicate_checksum);
    assert_eq!(harness.state().editor.rows().len(), 9);
    menu_click(&mut harness, "Edit", "Redo Ctrl+Y");
    assert_eq!(harness.state().editor.checksum(), without_original_wall);
    assert_eq!(harness.state().editor.rows().len(), 8);
    assert_no_open_transaction(&mut harness);
    menu_click(&mut harness, "File", "Save Ctrl+S");
    wait_for(&mut harness, "saving explicit wall removal", |app| {
        !app.editor.is_dirty()
    });
    let saved_checksum = harness.state().editor.checksum();

    menu_click(&mut harness, "File", "New Room project…");
    set_field(
        &mut harness,
        "New Room project directory",
        &fixture.other_output.display().to_string(),
    );
    set_field(&mut harness, "Room project seed", "wall-iteration-other");
    click(&mut harness, "Create Room project");
    wait_for(&mut harness, "creating another Room project", |app| {
        !app.windows_projects.busy()
            && app.editor.game() == EditorGame::RoomEscape
            && path_matches(
                app.editor.path().as_deref(),
                &fixture.other_output.join("room.scene.yaml"),
            )
    });
    menu_click(&mut harness, "File", "Open Room project…");
    set_field(
        &mut harness,
        "Existing Room project directory",
        &fixture.output.display().to_string(),
    );
    click(&mut harness, "Open Room project");
    wait_for(&mut harness, "reopening the authored Room walls", |app| {
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
            && app.editor.checksum() == saved_checksum
            && app.editor.rows().len() == 8
            && app.models.placements(&app.editor).len() == 2
            && app.room_camera.is_some()
    });
    let reopened = PreparedProject::open(&fixture.output).unwrap();
    assert_eq!(reopened.scene().frame().checksum(), saved_checksum);
    assert!(reopened.camera().is_some());
    assert!(!reopened
        .models()
        .document
        .bindings
        .contains_key(&original_wall_key));
    assert_eq!(
        wall_guid(harness.state(), "collision_barrier"),
        duplicate_wall
    );
    assert!(!harness
        .state()
        .editor
        .rows()
        .iter()
        .any(|row| row.guid.as_ref() == Some(&original_wall)));
    for (kind, guid) in &initial_roles {
        let Value::Struct(original_fields) = &initial_body_values[kind] else {
            panic!("Room Body must decode as a struct");
        };
        let original_position = original_fields
            .iter()
            .find(|(name, _)| name == "pos")
            .map(|(_, value)| value)
            .unwrap();
        let current_body = harness
            .state_mut()
            .editor
            .field_of(&Target::Guid(guid.clone()), "orr_physics3d::Body", "")
            .unwrap();
        assert_eq!(current_body, initial_body_values[kind]);
        assert!(matches!(original_position, Value::Vec3(_)));
    }

    click(&mut harness, LBL_PLAY);
    wait_for(&mut harness, "Room simulation ready", |app| {
        app.editor.can_take_control()
    });
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

    click(&mut harness, LBL_PLAY);
    wait_for(&mut harness, "controllable Room simulation", |app| {
        app.editor.can_take_control()
    });
    click(&mut harness, "Take control");
    wait_for(&mut harness, "active gameplay input", |app| {
        app.editor.input_phase() == Phase::Active
    });
    let player_start_x = player_x(harness.state(), &player);
    key(&mut harness, egui::Key::D, true);
    harness.run_steps(90);
    key(&mut harness, egui::Key::D, false);
    settle(&mut harness);
    let player_blocked_x = player_x(harness.state(), &player);
    assert!(player_blocked_x > player_start_x + 0.04);
    assert!(
        player_blocked_x < 0.2,
        "wall collision stops the PLAYER before the wall: {player_blocked_x}"
    );
    assert_eq!(room_run(harness.state()).key_collected, 0);
    key(&mut harness, egui::Key::E, true);
    harness.run_steps(4);
    key(&mut harness, egui::Key::E, false);
    settle(&mut harness);
    assert_eq!(room_run(harness.state()).key_collected, 0);
    assert_eq!(room_run(harness.state()).won, 0);
    click(&mut harness, LBL_PAUSE);
    assert!(!harness.state().editor.sim().playing);
    click(&mut harness, LBL_STOP);
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
        let bundle = std::env::var_os("ORR_ROOM_WALL_ITERATION_EXPORT_BUNDLE")
            .map(PathBuf::from)
            .unwrap_or_else(|| fixture._temp.path().join("Room wall iteration export"));
        assert!(bundle.is_absolute());
        assert!(!bundle.exists());
        assert!(bundle.parent().is_some_and(Path::is_dir));
        set_field(
            &mut harness,
            "Export destination",
            &bundle.display().to_string(),
        );
        click_role(&mut harness, Role::Button, "Export Windows game");
        wait_for(&mut harness, "same-App Room wall export", |app| {
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
            std::env::var_os("ORR_ROOM_WALL_ITERATION_EXPORT_BUNDLE").is_none(),
            "export bundle requires ORR_WINDOWS_EXPORT_RUNTIME"
        );
    }

    if let Some(capture_root) = std::env::var_os("ORR_ROOM_WALL_ITERATION_CAPTURE_DIR") {
        let capture_root = PathBuf::from(capture_root);
        assert!(capture_root.is_absolute());
        assert!(!capture_root.exists());
        assert!(capture_root.parent().is_some_and(Path::is_dir));
        fs::create_dir(&capture_root).unwrap();
        let frame = harness
            .render()
            .expect("capture the actual Room wall EditorApp window");
        assert!(frame.width() >= 800 && frame.height() >= 600);
        frame
            .save(capture_root.join("room-wall-iteration-ui.png"))
            .unwrap();
        let role_guids = initial_roles
            .iter()
            .map(|(kind, guid)| (*kind, guid.to_string()))
            .collect::<std::collections::BTreeMap<_, _>>();
        fs::write(
            capture_root.join("acceptance.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "room_project": fixture.output,
                "game": "RoomEscape",
                "adapter": viewport_adapter,
                "initial_player_key_exit_guids": role_guids,
                "edited_wall_guid": original_wall.to_string(),
                "collision_wall_guid": duplicate_wall.to_string(),
                "removed_original_wall_guid": original_wall.to_string(),
                "scene_checksum": format!("0x{saved_checksum:016x}"),
                "model_placements": harness.state().models.placements(&harness.state().editor).len(),
                "player_blocked_x": player_blocked_x,
                "key_collected": room_run(harness.state()).key_collected,
                "camera_installed": harness.state().room_camera.is_some(),
            }))
            .unwrap(),
        )
        .unwrap();
    }
}
