use super::{EditorApp, LBL_PAUSE, LBL_PLAY, LBL_STOP};
use crate::{editor::Editor, game::EditorGame, mesh_export::InputProfile, Mode};
use egui::{accesskit::Role, Event, Key, Modifiers};
use egui_kittest::{kittest::Queryable, Harness};
use egui_wgpu::{wgpu, WgpuSetupCreateNew};
use orr_fp::FP;
use orr_navigation::NavigationStatus;
use orr_remote::mesh_navigation::{mesh_from_view, mission};
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

struct Fixture {
    root: PathBuf,
    captures: PathBuf,
    physics_scene: PathBuf,
    scene: PathBuf,
    runtime: PathBuf,
    export: PathBuf,
}

struct PlayerApp {
    inner: EditorApp,
}

struct HiddenSourceScene {
    visible: PathBuf,
    hidden: PathBuf,
}

impl HiddenSourceScene {
    fn hide(fixture: &Fixture) -> Self {
        let visible = fixture.scene.clone();
        let hidden = fixture.root.join("hidden-original-mesh-game.scene.yaml");
        assert!(visible.is_file());
        assert!(!hidden.exists());
        fs::rename(&visible, &hidden).unwrap();
        Self { visible, hidden }
    }

    fn restore(self) -> std::io::Result<()> {
        fs::rename(&self.hidden, &self.visible)
    }
}

impl Drop for HiddenSourceScene {
    fn drop(&mut self) {
        if self.hidden.exists() && !self.visible.exists() {
            let _ = fs::rename(&self.hidden, &self.visible);
        }
    }
}

impl eframe::App for PlayerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.inner.mesh_player_ui(ui);
    }
}

impl Fixture {
    fn new() -> Self {
        assert_eq!(
            std::env::var("ORR_MESH_KEYBOARD_ACCEPTANCE").as_deref(),
            Ok("1")
        );
        assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
        assert_eq!(std::env::var("WGPU_BACKEND").unwrap_or_default(), "vulkan");
        let root = PathBuf::from(
            std::env::var_os("ORR_MESH_KEYBOARD_CAPTURE_DIR")
                .expect("set a fresh keyboard-play capture root"),
        );
        assert!(root.is_absolute());
        fs::create_dir(&root).expect("capture root must be fresh");
        let root = root.canonicalize().unwrap();
        let captures = root.join("captures");
        fs::create_dir(&captures).unwrap();
        let physics_scene = root.join("owned-physics.scene.yaml");
        fs::copy(EditorGame::PhysGame.default_scene_path(), &physics_scene).unwrap();
        let scene = root.join("keyboard-mesh-game.scene.yaml");
        assert!(!scene.exists());
        let runtime = PathBuf::from(
            std::env::var_os("ORR_MESH_EXPORT_RUNTIME")
                .expect("set the fresh matching Mesh Player executable"),
        );
        assert!(runtime.is_absolute() && runtime.is_file());
        let export = PathBuf::from(
            std::env::var_os("ORR_MESH_KEYBOARD_EXPORT_OUTPUT")
                .expect("set a fresh sibling keyboard-play export directory"),
        );
        assert!(export.is_absolute() && !export.exists());
        assert_eq!(
            export.parent().unwrap().canonicalize().unwrap(),
            root.parent().unwrap()
        );
        Self {
            root,
            captures,
            physics_scene,
            scene,
            runtime,
            export,
        }
    }

    fn editor_harness(&self) -> Harness<'static, EditorApp> {
        let mut editor = Editor::open_game(&self.physics_scene, EditorGame::PhysGame).unwrap();
        editor.sync();
        Harness::builder()
            .with_size([1600.0, 1100.0])
            .with_step_dt(1.0 / 60.0)
            .wgpu_setup(hardware_vulkan_setup())
            .build_eframe(move |context| EditorApp::new(editor, context.wgpu_render_state.clone()))
    }

    fn player_harness(&self, bundle_scene: &Path) -> Harness<'static, PlayerApp> {
        let mut editor = Editor::open_game(bundle_scene, EditorGame::MeshNavigation).unwrap();
        editor.sync();
        assert!(editor.start_play());
        editor.sync();
        Harness::builder()
            .with_size([1200.0, 800.0])
            .with_step_dt(1.0 / 60.0)
            .wgpu_setup(hardware_vulkan_setup())
            .build_eframe(move |context| {
                let mut inner = EditorApp::new(editor, context.wgpu_render_state.clone());
                inner.set_mesh_player_input_allowed(true);
                PlayerApp { inner }
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

fn settle_player(harness: &mut Harness<'_, PlayerApp>) {
    for _ in 0..4 {
        harness.state_mut().inner.editor.sync();
        harness.run_steps(2);
    }
}

fn click(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness.get_by_label(label).scroll_to_me();
    harness.run_steps(20);
    harness.get_by_label(label).click_accesskit();
    settle(harness);
}

fn click_button(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness
        .get_by_role_and_label(Role::Button, label)
        .scroll_to_me();
    harness.run_steps(20);
    harness
        .get_by_role_and_label(Role::Button, label)
        .click_accesskit();
    settle(harness);
}

fn click_live(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness.get_by_label(label).scroll_to_me();
    harness.run_steps(1);
    harness.get_by_label(label).click_accesskit();
    settle(harness);
}

fn click_live_button(harness: &mut Harness<'_, EditorApp>, label: &str) {
    harness
        .get_by_role_and_label(Role::Button, label)
        .scroll_to_me();
    harness.run_steps(1);
    harness
        .get_by_role_and_label(Role::Button, label)
        .click_accesskit();
    settle(harness);
}

fn click_live_player(harness: &mut Harness<'_, PlayerApp>, label: &str) {
    harness.get_by_label(label).scroll_to_me();
    harness.run_steps(1);
    harness.get_by_label(label).click_accesskit();
    settle_player(harness);
}

fn click_live_player_button(harness: &mut Harness<'_, PlayerApp>, label: &str) {
    harness
        .get_by_role_and_label(Role::Button, label)
        .scroll_to_me();
    harness.run_steps(1);
    harness
        .get_by_role_and_label(Role::Button, label)
        .click_accesskit();
    settle_player(harness);
}

fn menu_click(harness: &mut Harness<'_, EditorApp>, menu: &str, item: &str) {
    if harness.query_by_label(item).is_none() {
        click(harness, menu);
    }
    harness.get_by_label(item).click_accesskit();
    settle(harness);
}

fn type_field(harness: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    harness.run_steps(20);
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .focus();
    harness.run_steps(2);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    harness.run_steps(2);
}

fn type_game_path(harness: &mut Harness<'_, EditorApp>, path: &Path) {
    type_field(
        harness,
        "New mesh game scene path",
        &path.display().to_string(),
    );
}

fn wait_for_editor(
    harness: &mut Harness<'_, EditorApp>,
    reason: &str,
    ready: impl Fn(&EditorApp) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        harness.state_mut().editor.sync();
        harness.run_steps(1);
        if ready(harness.state()) {
            settle(harness);
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("timed out waiting for {reason}");
}

fn wait_for_mesh_input_phase(
    harness: &mut Harness<'_, EditorApp>,
    phase: crate::editor::input::Phase,
) {
    wait_for_editor(harness, "managed mesh input phase", |app| {
        app.editor.mesh_play_input_phase() == phase
    });
}

fn enable_and_focus_mesh_keyboard(harness: &mut Harness<'_, EditorApp>) {
    wait_for_mesh_input_phase(harness, crate::editor::input::Phase::Off);
    assert!(!harness.state().editor.mesh_play_input_enabled());
    click_live(harness, "Enable mesh goal input");
    wait_for_mesh_input_phase(harness, crate::editor::input::Phase::Active);
    click_live_button(harness, "Focus mesh keyboard controls");
    wait_for_label_editor(
        harness,
        "focused mesh keyboard viewport",
        "Mesh keyboard controls focused",
    );
}

fn wait_for_player(
    harness: &mut Harness<'_, PlayerApp>,
    reason: &str,
    ready: impl Fn(&PlayerApp) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        harness.state_mut().inner.editor.sync();
        harness.run_steps(1);
        if ready(harness.state()) {
            settle_player(harness);
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("timed out waiting for {reason}");
}

fn wait_for_player_mission_win(harness: &mut Harness<'_, PlayerApp>) {
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        harness.state_mut().inner.editor.sync();
        harness.run_steps(1);
        if player_mission_view(harness.state()).run.won == 1 {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("timed out waiting for recorded Player mission win");
}

fn editor_mesh_view(app: &EditorApp) -> orr_remote::mesh_navigation::MeshView {
    let snapshot = app.editor.snapshot().unwrap();
    mesh_from_view(snapshot.predicted()).unwrap()
}

fn editor_mission_view(app: &EditorApp) -> mission::MissionView {
    let snapshot = app.editor.snapshot().unwrap();
    mission::from_view(snapshot.predicted()).unwrap().unwrap()
}

fn player_mesh_view(app: &PlayerApp) -> orr_remote::mesh_navigation::MeshView {
    let snapshot = app.inner.editor.snapshot().unwrap();
    mesh_from_view(snapshot.predicted()).unwrap()
}

fn player_mission_view(app: &PlayerApp) -> mission::MissionView {
    let snapshot = app.inner.editor.snapshot().unwrap();
    mission::from_view(snapshot.predicted()).unwrap().unwrap()
}

fn send_key(harness: &mut Harness<'_, EditorApp>, key: Key, repeat: bool) {
    harness.event(Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat,
        modifiers: Modifiers::NONE,
    });
    harness.run_steps(1);
    harness.event(Event::Key {
        key,
        physical_key: Some(key),
        pressed: false,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    harness.run_steps(1);
}

fn send_player_key(harness: &mut Harness<'_, PlayerApp>, key: Key, repeat: bool) {
    harness.event(Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat,
        modifiers: Modifiers::NONE,
    });
    harness.run_steps(1);
    harness.event(Event::Key {
        key,
        physical_key: Some(key),
        pressed: false,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    harness.run_steps(1);
}

fn wait_editor_goal_change(harness: &mut Harness<'_, EditorApp>, previous: [FP; 3]) -> bool {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        harness.state_mut().editor.sync();
        harness.run_steps(1);
        if editor_mesh_view(harness.state()).pin.goal != previous {
            settle(harness);
            return true;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    false
}

fn wait_player_goal_change(harness: &mut Harness<'_, PlayerApp>, previous: [FP; 3]) -> bool {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        harness.state_mut().inner.editor.sync();
        harness.run_steps(1);
        if player_mesh_view(harness.state()).pin.goal != previous {
            settle_player(harness);
            return true;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    false
}

fn wait_editor_arrived(harness: &mut Harness<'_, EditorApp>) {
    wait_for_editor(harness, "keyboard destination arrival", |app| {
        editor_mesh_view(app).agent.status == NavigationStatus::Arrived
    });
}

fn wait_player_arrived(harness: &mut Harness<'_, PlayerApp>) {
    wait_for_player(harness, "keyboard destination arrival in Player", |app| {
        player_mesh_view(app).agent.status == NavigationStatus::Arrived
    });
}

fn advance_editor_direction(harness: &mut Harness<'_, EditorApp>, key: Key) -> bool {
    let observed_tick = harness.state().editor.sim().head_tick;
    wait_for_editor(harness, "next live input tick", |app| {
        app.editor.sim().playing
            && app.editor.sim().head_tick > observed_tick
            && app.editor.sim().head_tick == app.editor.sim().last_tick
            && app.editor.mesh_play_input_phase() == crate::editor::input::Phase::Active
    });
    let previous_goal = editor_mesh_view(harness.state()).pin.goal;
    send_key(harness, key, false);
    if !wait_editor_goal_change(harness, previous_goal) {
        return false;
    }
    wait_editor_arrived(harness);
    true
}

fn advance_player_east(harness: &mut Harness<'_, PlayerApp>) -> bool {
    let observed_tick = harness.state().inner.editor.sim().head_tick;
    wait_for_player(harness, "next live input tick in Player", |app| {
        app.inner.editor.sim().playing
            && app.inner.editor.sim().head_tick > observed_tick
            && app.inner.editor.sim().head_tick == app.inner.editor.sim().last_tick
            && app.inner.editor.mesh_play_input_phase() == crate::editor::input::Phase::Active
    });
    let previous_goal = player_mesh_view(harness.state()).pin.goal;
    send_player_key(harness, Key::D, false);
    if !wait_player_goal_change(harness, previous_goal) {
        return false;
    }
    wait_player_arrived(harness);
    true
}

fn drive_editor_mission(harness: &mut Harness<'_, EditorApp>) -> usize {
    let mut key_steps = 0;
    while editor_mission_view(harness.state()).run.won == 0 && key_steps < 32 {
        assert!(
            advance_editor_direction(harness, Key::D),
            "East key should queue a valid goal"
        );
        key_steps += 1;
    }
    assert_eq!(editor_mission_view(harness.state()).run.won, 1);
    key_steps
}

fn drive_player_mission(harness: &mut Harness<'_, PlayerApp>) -> usize {
    let mut key_steps = 0;
    while player_mission_view(harness.state()).run.won == 0 && key_steps < 32 {
        assert!(
            advance_player_east(harness),
            "East key should queue a valid Player goal"
        );
        key_steps += 1;
    }
    assert_eq!(player_mission_view(harness.state()).run.won, 1);
    key_steps
}

fn capture_editor(
    fixture: &Fixture,
    harness: &mut Harness<'_, EditorApp>,
    name: &str,
) -> serde_json::Value {
    let frame = harness.render().expect("render full EditorApp frame");
    let pixels = frame.as_raw().clone();
    capture_bytes(
        &fixture.captures,
        name,
        pixels,
        frame.width(),
        frame.height(),
    )
}

fn capture_player(
    fixture: &Fixture,
    harness: &mut Harness<'_, PlayerApp>,
    name: &str,
) -> serde_json::Value {
    let frame = harness.render().expect("render full Mesh Player UI frame");
    capture_bytes(
        &fixture.captures,
        name,
        frame.as_raw().clone(),
        frame.width(),
        frame.height(),
    )
}

fn capture_bytes(
    captures: &Path,
    name: &str,
    pixels: Vec<u8>,
    width: u32,
    height: u32,
) -> serde_json::Value {
    let path = captures.join(name);
    crate::app::write_png(
        &path,
        &egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &pixels),
    )
    .unwrap();
    let bytes = fs::read(path).unwrap();
    serde_json::json!({"name":name,"bytes":bytes.len(),"sha256":sha256(&bytes)})
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
#[ignore = "requires ORR_MESH_KEYBOARD_ACCEPTANCE=1 and a hardware Vulkan adapter"]
fn mesh_keyboard_controls_drive_mission_in_editor_and_source_free_player() {
    assert_eq!(
        std::env::var("ORR_MESH_KEYBOARD_ACCEPTANCE").as_deref(),
        Ok("1")
    );
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    assert_eq!(std::env::var("WGPU_BACKEND").unwrap_or_default(), "vulkan");
    let preflight = Wgpu::headless(WgpuOptions::default()).expect("Vulkan GPU preflight");
    assert!(
        !preflight.is_software(),
        "software adapter is not GPU evidence"
    );
    eprintln!(
        "Mesh keyboard acceptance adapter: {}",
        preflight.adapter_name()
    );
    drop(preflight);

    let fixture = Fixture::new();
    let mut editor = fixture.editor_harness();
    editor.run_steps(4);
    assert!(editor.state().has_gpu());
    menu_click(&mut editor, "File", "New mesh game…");
    type_game_path(&mut editor, &fixture.scene);
    for (label, value) in [
        ("Game floor width", "12"),
        ("Game floor depth", "8"),
        ("Game collectible count", "3"),
        ("Game distance per tick", "0.25"),
    ] {
        type_field(&mut editor, label, value);
    }
    click_button(&mut editor, "Create mesh game");
    wait_for_editor(&mut editor, "creating an authored Mesh game", |app| {
        app.editor.game() == EditorGame::MeshNavigation
            && app.editor.path().is_some_and(|path| {
                path == fixture.scene
                    || path.canonicalize().ok() == fixture.scene.canonicalize().ok()
            })
            && !app.mesh_game_creator.busy()
    });
    let initial_view = editor_mesh_view(editor.state());
    assert_ne!(initial_view.pin.goal, initial_view.pin.start);
    let initial_mission = editor_mission_view(editor.state());
    assert_eq!(initial_mission.config.count, 3);
    assert_eq!(initial_mission.run.collected, 0);
    assert_eq!(initial_mission.run.won, 0);
    click(&mut editor, "Static-mesh navigation");
    for (axis, label) in ["X", "Y", "Z"].into_iter().enumerate() {
        type_field(
            &mut editor,
            &format!("Goal {label}"),
            &orr_reflect::decimal::fp_to_decimal(initial_view.pin.start[axis]),
        );
    }
    click_button(&mut editor, "Place mesh agent");
    wait_for_editor(
        &mut editor,
        "placing a stationary initial agent goal",
        |app| editor_mesh_view(app).pin.goal == initial_view.pin.start,
    );
    menu_click(&mut editor, "File", "Save Ctrl+S");
    wait_for_editor(&mut editor, "saving the stationary mission start", |app| {
        !app.editor.is_dirty()
    });
    let initial_view = editor_mesh_view(editor.state());
    let authored_checksum = editor.state().editor.checksum();
    let authored_history = editor.state().editor.history().entries.len();
    let editor_initial_capture = capture_editor(&fixture, &mut editor, "editor-authored-game.png");

    click(&mut editor, LBL_PLAY);
    wait_for_editor(&mut editor, "starting editor Mesh Play", |app| {
        app.editor.mode() == Mode::Play && app.editor.sim().playing
    });
    enable_and_focus_mesh_keyboard(&mut editor);
    assert!(editor
        .query_by_label("Mesh keyboard controls focused")
        .is_some());

    let input_goal_before_negatives = editor_mesh_view(editor.state()).pin.goal;
    assert!(editor
        .query_by_role_and_label(Role::TextInput, "Goal X")
        .is_some());
    editor
        .get_by_role_and_label(Role::TextInput, "Goal X")
        .focus();
    editor.run_steps(2);
    send_key(&mut editor, Key::D, false);
    assert_eq!(
        editor_mesh_view(editor.state()).pin.goal,
        input_goal_before_negatives
    );
    click_live_button(&mut editor, "Focus mesh keyboard controls");
    let text_focus_preserved_goal =
        editor_mesh_view(editor.state()).pin.goal == input_goal_before_negatives;

    click_live(&mut editor, LBL_PAUSE);
    wait_for_mesh_input_phase(&mut editor, crate::editor::input::Phase::Off);
    let paused_goal = editor_mesh_view(editor.state()).pin.goal;
    send_key(&mut editor, Key::D, false);
    assert_eq!(editor_mesh_view(editor.state()).pin.goal, paused_goal);
    let paused_input_preserved_goal = true;
    click_live(&mut editor, LBL_PLAY);
    enable_and_focus_mesh_keyboard(&mut editor);

    editor.input_mut().focused = false;
    editor.event(Event::WindowFocused(false));
    editor.run_steps(2);
    assert!(!editor.state().editor.mesh_play_input_enabled());
    wait_for_mesh_input_phase(&mut editor, crate::editor::input::Phase::Off);
    let unfocused_goal = editor_mesh_view(editor.state()).pin.goal;
    send_key(&mut editor, Key::D, false);
    assert_eq!(editor_mesh_view(editor.state()).pin.goal, unfocused_goal);
    editor.input_mut().focused = true;
    editor.event(Event::WindowFocused(true));
    editor.run_steps(2);
    wait_for_mesh_input_phase(&mut editor, crate::editor::input::Phase::Off);
    enable_and_focus_mesh_keyboard(&mut editor);
    let window_focus_preserved_goal = true;

    let mut west_steps = 0;
    while editor_mesh_view(editor.state()).agent.position[2] > FP::ZERO && west_steps < 12 {
        assert!(advance_editor_direction(&mut editor, Key::W));
        west_steps += 1;
    }
    assert_eq!(editor_mesh_view(editor.state()).agent.position[2], FP::ZERO);
    let edge_goal = editor_mesh_view(editor.state()).pin.goal;
    send_key(&mut editor, Key::W, false);
    assert_eq!(editor_mesh_view(editor.state()).pin.goal, edge_goal);
    let invalid_edge_preserved_goal = true;
    let mut east_steps = 0;
    while editor_mesh_view(editor.state()).agent.position[2] < FP::from_int(4) && east_steps < 12 {
        assert!(advance_editor_direction(&mut editor, Key::S));
        east_steps += 1;
    }
    assert_eq!(
        editor_mesh_view(editor.state()).agent.position[2],
        FP::from_int(4)
    );

    editor.key_press(Key::Escape);
    settle(&mut editor);
    assert!(!editor.state().editor.mesh_play_input_enabled());
    wait_for_mesh_input_phase(&mut editor, crate::editor::input::Phase::Off);
    let escape_goal = editor_mesh_view(editor.state()).pin.goal;
    send_key(&mut editor, Key::D, false);
    assert_eq!(editor_mesh_view(editor.state()).pin.goal, escape_goal);
    let escape_released_input = !editor.state().editor.mesh_play_input_enabled();
    assert!(escape_released_input);
    enable_and_focus_mesh_keyboard(&mut editor);

    let keyboard_controls_capture =
        capture_editor(&fixture, &mut editor, "editor-keyboard-controls.png");
    let editor_key_steps = drive_editor_mission(&mut editor);
    let editor_win = editor_mission_view(editor.state());
    assert_eq!(editor_win.run.collected, 0b111);
    assert_eq!(editor_win.run.won, 1);
    let editor_win_tick = editor.state().editor.timeline().unwrap().tick;
    assert!(editor_win_tick > 0);
    let editor_win_frame = editor
        .state()
        .editor
        .snapshot()
        .unwrap()
        .predicted()
        .checksum();
    let editor_win_capture = capture_editor(&fixture, &mut editor, "editor-keyboard-win.png");
    click_live(&mut editor, "Disable mesh goal input");
    click(&mut editor, LBL_STOP);
    wait_for_editor(&mut editor, "stopping editor gameplay", |app| {
        app.editor.mode() == Mode::Edit
    });
    assert_eq!(editor.state().editor.checksum(), authored_checksum);
    assert_eq!(
        editor.state().editor.history().entries.len(),
        authored_history
    );
    click(&mut editor, "File");
    click(&mut editor, "Save Ctrl+S");
    settle(&mut editor);
    let saved_scene = fs::read(&fixture.scene).unwrap();
    assert!(!editor.state().editor.is_dirty());

    click(&mut editor, "File");
    click(&mut editor, "Export mesh game…");
    type_field(
        &mut editor,
        "Mesh player executable",
        &fixture.runtime.display().to_string(),
    );
    type_field(
        &mut editor,
        "Mesh game output directory",
        &fixture.export.display().to_string(),
    );
    click(&mut editor, "I trust this local mesh player executable");
    click_button(&mut editor, "Export mesh game");
    wait_for_label_contains_editor(
        &mut editor,
        "interactive Mesh bundle export",
        "Mesh export complete:",
    );
    let bundle =
        crate::mesh_export::open_bundle(&fixture.export).expect("admitted source-free bundle");
    assert_eq!(bundle.input_profile, Some(InputProfile::MeshClickToMoveV1));
    assert_eq!(bundle.checksum, authored_checksum);
    assert_eq!(bundle.graph_revision, initial_view.graph.revision());
    assert_eq!(fs::read(&bundle.scene).unwrap(), saved_scene);
    let mut bundle_names = fs::read_dir(&fixture.export)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    bundle_names.sort();
    assert_eq!(
        bundle_names,
        vec![
            "keyboard-mesh-game.scene.yaml",
            "orr.mesh-bundle.json",
            "orr_mesh_player.exe"
        ]
    );
    let export_manifest = fs::read(fixture.export.join("orr.mesh-bundle.json")).unwrap();
    let export_runtime = fs::read(fixture.export.join("orr_mesh_player.exe")).unwrap();
    assert_eq!(
        sha256(&export_runtime),
        sha256(&fs::read(&fixture.runtime).unwrap())
    );

    let hidden_source = HiddenSourceScene::hide(&fixture);
    assert!(!fixture.scene.exists());
    let mut player = fixture.player_harness(&bundle.scene);
    player.run_steps(4);
    assert!(player.state().inner.has_gpu());
    assert_eq!(player.state().inner.editor.mode(), Mode::Play);
    assert!(!player.state().inner.editor.sim().playing);
    click_live_player_button(&mut player, "Resume");
    wait_for_player(&mut player, "source-free Player initial state", |app| {
        app.inner.editor.mode() == Mode::Play && app.inner.editor.sim().playing
    });
    click_live_player(&mut player, "Enable mesh goal input");
    click_live_player_button(&mut player, "Focus mesh keyboard controls");
    wait_for_label_player(
        &mut player,
        "Player managed input lease",
        "Managed player input active",
    );
    let player_initial_capture =
        capture_player(&fixture, &mut player, "player-source-free-open.png");
    let player_key_steps = drive_player_mission(&mut player);
    let player_win = player_mission_view(player.state());
    assert_eq!(player_win.run.collected, 0b111);
    assert_eq!(player_win.run.won, 1);
    let player_win_tick = player.state().inner.editor.timeline().unwrap().tick;
    let player_win_checksum = player
        .state()
        .inner
        .editor
        .snapshot()
        .unwrap()
        .predicted()
        .checksum();
    let player_win_capture = capture_player(&fixture, &mut player, "player-keyboard-win.png");
    let post_win_tail_target = player_win_tick.saturating_add(120);
    wait_for_player(
        &mut player,
        "recording a neutral post-win timeline tail",
        |app| {
            app.inner
                .editor
                .timeline()
                .is_some_and(|timeline| timeline.tick >= post_win_tail_target && timeline.playing)
        },
    );
    click_live_player_button(&mut player, "Pause");
    wait_for_player(&mut player, "pausing the recorded Player run", |app| {
        app.inner
            .editor
            .timeline()
            .is_some_and(|timeline| !timeline.playing)
    });
    let recorded_last_tick = player.state().inner.editor.sim().last_tick;
    let recorded_branches = player.state().inner.editor.sim().branches;
    assert!(recorded_last_tick >= post_win_tail_target);
    click_live_player_button(&mut player, "Replay");
    wait_for_player(&mut player, "source-free Player replay reset", |app| {
        let run = player_mission_view(app).run;
        run.collected == 0 && run.won == 0
    });
    wait_for_label_player(
        &mut player,
        "recorded timeline playback status",
        "Replaying recorded timeline",
    );
    let player_replay_kept_host_paused = !player.state().inner.editor.sim().playing;
    assert!(player_replay_kept_host_paused);
    let replay_pause_target = (recorded_last_tick / 4).max(1);
    wait_for_player(
        &mut player,
        "recorded playback progress before pause",
        |app| {
            let timeline = app.inner.editor.timeline().unwrap();
            timeline.tick >= replay_pause_target && timeline.tick < recorded_last_tick
        },
    );
    click_live_player_button(&mut player, "Pause");
    wait_for_label_player(&mut player, "pausing recorded timeline playback", "Resume");
    let replay_paused_tick = player.state().inner.editor.sim().head_tick;
    assert!(replay_paused_tick > 0 && replay_paused_tick < recorded_last_tick);
    assert_eq!(
        player.state().inner.editor.sim().head_tick,
        replay_paused_tick
    );
    assert_eq!(
        player.state().inner.editor.sim().last_tick,
        recorded_last_tick
    );
    assert_eq!(
        player.state().inner.editor.sim().branches,
        recorded_branches
    );
    click_live_player_button(&mut player, "Resume");
    wait_for_label_player(
        &mut player,
        "resuming recorded timeline playback",
        "Replaying recorded timeline",
    );
    let player_recorded_playback_resumed_from_cursor = player.state().inner.editor.sim().head_tick
        > replay_paused_tick
        && !player.state().inner.editor.sim().playing;
    assert!(player_recorded_playback_resumed_from_cursor);
    wait_for_player_mission_win(&mut player);
    click_live_player_button(&mut player, "Pause");
    wait_for_label_player(&mut player, "pausing recorded timeline playback", "Resume");
    assert_eq!(
        player.state().inner.editor.sim().last_tick,
        recorded_last_tick
    );
    assert_eq!(
        player.state().inner.editor.sim().branches,
        recorded_branches
    );
    player.state_mut().inner.editor.seek(player_win_tick);
    wait_for_player(
        &mut player,
        "seeking the replay to the recorded win tick",
        |app| {
            app.inner
                .editor
                .timeline()
                .is_some_and(|timeline| timeline.tick == player_win_tick && !timeline.playing)
        },
    );
    assert_eq!(
        player.state().inner.editor.sim().last_tick,
        recorded_last_tick
    );
    assert_eq!(
        player.state().inner.editor.sim().branches,
        recorded_branches
    );
    let replay_checksum = player
        .state()
        .inner
        .editor
        .snapshot()
        .unwrap()
        .predicted()
        .checksum();
    assert_eq!(replay_checksum, player_win_checksum);
    assert_eq!(
        player.state().inner.editor.timeline().unwrap().tick,
        player_win_tick
    );
    let player_replay_capture = capture_player(&fixture, &mut player, "player-keyboard-replay.png");
    click_live_player_button(&mut player, "Resume");
    wait_for_player(&mut player, "recorded timeline reaching its end", |app| {
        app.inner.editor.sim().head_tick == recorded_last_tick
            && !app.inner.editor.sim().playing
            && !app.inner.mesh_recorded_playback.active()
    });
    let player_recorded_playback_auto_paused_at_end = player.state().inner.editor.sim().head_tick
        == recorded_last_tick
        && !player.state().inner.editor.sim().playing
        && !player.state().inner.mesh_recorded_playback.active();
    assert!(player_recorded_playback_auto_paused_at_end);
    assert_eq!(
        player.state().inner.editor.sim().last_tick,
        recorded_last_tick
    );
    assert_eq!(
        player.state().inner.editor.sim().branches,
        recorded_branches
    );
    let player_recorded_timeline_future_preserved =
        player.state().inner.editor.sim().last_tick == recorded_last_tick;
    let player_recorded_timeline_branches_preserved =
        player.state().inner.editor.sim().branches == recorded_branches;
    assert!(player_recorded_timeline_future_preserved);
    assert!(player_recorded_timeline_branches_preserved);
    let original_scene_hidden_during_player = !fixture.scene.exists();
    assert!(player
        .state()
        .inner
        .mesh_recorded_playback
        .error()
        .is_none());
    assert!(original_scene_hidden_during_player);
    drop(player);
    hidden_source.restore().unwrap();
    let original_scene_restored_after_player =
        fixture.scene.is_file() && fs::read(&fixture.scene).unwrap() == saved_scene;
    assert!(original_scene_restored_after_player);

    let manifest_value: serde_json::Value = serde_json::from_slice(&export_manifest).unwrap();
    let receipt = serde_json::json!({
        "artifact_root":fixture.root,
        "scene_path":fixture.scene,
        "scene_sha256":sha256(&saved_scene),
        "scene_checksum":format!("0x{:016x}", authored_checksum),
        "graph_revision":hex(&initial_view.graph.revision()),
        "mission_collectibles":editor_win.run.collected,
        "editor_mission_won":editor_win.run.won == 1,
        "editor_keyboard_goal_steps":editor_key_steps,
        "editor_win_frame_checksum":format!("0x{editor_win_frame:016x}"),
        "text_focus_preserved_goal":text_focus_preserved_goal,
        "paused_input_preserved_goal":paused_input_preserved_goal,
        "window_focus_preserved_goal":window_focus_preserved_goal,
        "escape_released_input":escape_released_input,
        "invalid_edge_preserved_goal":invalid_edge_preserved_goal,
        "stop_restored_authored_scene":editor.state().editor.checksum() == authored_checksum,
        "export":{
            "bundle_directory":fixture.export,
            "bundle_manifest_sha256":sha256(&export_manifest),
            "bundle_runtime_sha256":sha256(&export_runtime),
            "bundle_input_profile":manifest_value["input_profile"],
            "bundle_scene_sha256":sha256(&saved_scene),
            "bundle_files":bundle_names,
            "runtime_source":fixture.runtime
        },
        "player_mission_won":player_win.run.won == 1,
        "player_keyboard_goal_steps":player_key_steps,
        "player_keyboard_exact_replay":replay_checksum == player_win_checksum,
        "player_replay_mode":"recorded-timeline-seek",
        "player_replay_kept_host_paused":player_replay_kept_host_paused,
        "player_recorded_playback_pause_resume_verified":player_recorded_playback_resumed_from_cursor,
        "player_recorded_playback_paused_tick":replay_paused_tick,
        "player_recorded_playback_auto_paused_at_end":player_recorded_playback_auto_paused_at_end,
        "player_recorded_timeline_future_preserved":player_recorded_timeline_future_preserved,
        "player_recorded_timeline_branches_preserved":player_recorded_timeline_branches_preserved,
        "player_recorded_last_tick":recorded_last_tick,
        "player_recorded_branches":recorded_branches,
        "player_win_tick":player_win_tick,
        "player_win_frame_checksum":format!("0x{player_win_checksum:016x}"),
        "original_scene_hidden_during_player":original_scene_hidden_during_player,
        "original_scene_restored_after_player":original_scene_restored_after_player,
        "captures":[editor_initial_capture, keyboard_controls_capture, editor_win_capture, player_initial_capture, player_win_capture, player_replay_capture],
        "physical_os_input":false
    });
    fs::write(
        fixture.root.join("acceptance.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
}

fn wait_for_label_editor(harness: &mut Harness<'_, EditorApp>, reason: &str, label: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        harness.state_mut().editor.sync();
        harness.run_steps(1);
        if harness.query_by_label(label).is_some() {
            settle(harness);
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("timed out waiting for {reason}: {label}");
}

fn wait_for_label_contains_editor(harness: &mut Harness<'_, EditorApp>, reason: &str, label: &str) {
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        harness.state_mut().editor.sync();
        harness.run_steps(1);
        if harness.query_by_label_contains(label).is_some() {
            settle(harness);
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("timed out waiting for {reason}: {label}");
}

fn wait_for_label_player(harness: &mut Harness<'_, PlayerApp>, reason: &str, label: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        harness.state_mut().inner.editor.sync();
        harness.run_steps(1);
        if harness.query_by_label(label).is_some() {
            settle_player(harness);
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("timed out waiting for {reason}: {label}");
}
