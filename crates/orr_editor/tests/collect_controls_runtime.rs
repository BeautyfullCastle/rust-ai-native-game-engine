//! CPU acceptance through the actual authoring and main EditorApp widgets,
//! keyboard events, managed ERP ownership and Collect's authoritative snapshot.
#![cfg(all(feature = "collect-ui", feature = "collect-input-map-editor"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
#[path = "../src/collect_input_map_authoring.rs"]
mod collect_input_map_authoring;

use egui::{accesskit::Role, Key};
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{editor::input::Phase, Editor, EditorApp, HostSpec, Mode};
use orr_fp::{FPVec2, FP};
use orr_input::{ActionMap, Button, Key as MapKey};
use orr_sample::{collect_game::*, collect_project::PreparedProject};
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

fn settle(h: &mut Harness<'_, collect_input_map_authoring::App>) {
    h.try_run().expect("authoring layout must settle");
}
fn author_click(h: &mut Harness<'_, collect_input_map_authoring::App>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).scroll_to_me();
    settle(h);
    h.get_by_role_and_label(Role::Button, label).click();
    settle(h);
}
fn author_combo(h: &mut Harness<'_, collect_input_map_authoring::App>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::ComboBox, label)
        .scroll_to_me();
    settle(h);
    h.get_by_role_and_label(Role::ComboBox, label).click();
    settle(h);
    author_click(h, value);
}
fn authored_map(path: &Path) -> ActionMap {
    let mut h = Harness::builder()
        .with_size([720.0, 650.0])
        .with_max_steps(120)
        .build_eframe(|_| collect_input_map_authoring::App::default());
    settle(&mut h);
    for (action, binding) in [
        ("left", Key::J),
        ("right", Key::L),
        ("up", Key::I),
        ("down", Key::K),
        ("restart", Key::Enter),
        ("quit", Key::Q),
    ] {
        author_combo(&mut h, "Action", action);
        author_click(&mut h, "Clear action draft");
        author_click(&mut h, "Add binding");
        author_click(&mut h, "Capture binding 1");
        // A physical edge, deliberately distinct from its logical layout key.
        // Down/up belong to one RawInput batch, not Harness::event queued frames.
        for pressed in [true, false] {
            h.input_mut().events.push(egui::Event::Key {
                key: Key::F1,
                physical_key: Some(binding),
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
        settle(&mut h);
        author_click(&mut h, "Apply action");
        assert!(h.state().panel().message().starts_with("Action applied"));
    }
    h.get_by_role(Role::TextInput).scroll_to_me();
    settle(&mut h);
    h.get_by_role(Role::TextInput).focus();
    settle(&mut h);
    h.get_by_role(Role::TextInput)
        .type_text(path.to_str().unwrap());
    settle(&mut h);
    author_click(&mut h, "Save As new file");
    assert!(h
        .state()
        .panel()
        .message()
        .starts_with("Saved accepted map."));
    let map = ActionMap::load_file(path).unwrap();
    assert_eq!(&map, h.state().panel().document());
    map
}
struct Project {
    dir: tempfile::TempDir,
    config: Vec<u8>,
    scene: Vec<u8>,
}
impl Project {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config =
            include_bytes!("../../../assets/collect_dodge_project/orr.project.json").to_vec();
        let scene = include_bytes!("../../../scenes/collect_dodge_v1.scene.yaml").to_vec();
        fs::write(dir.path().join("orr.project.json"), &config).unwrap();
        fs::write(dir.path().join("level.scene.yaml"), &scene).unwrap();
        Self { dir, config, scene }
    }
    fn harness(&self) -> Harness<'static, EditorApp> {
        let project = PreparedProject::open(self.dir.path()).unwrap();
        let mut editor = Editor::start(&HostSpec::PreparedCollect {
            scene: project.path().to_path_buf(),
            text: project.scene().text().into(),
            listen: None,
            debug_hooks: false,
        })
        .unwrap();
        editor.sync();
        Harness::builder()
            .with_size([1400.0, 900.0])
            .build_eframe(|_| EditorApp::new(editor, None))
    }
    fn unchanged(&self) {
        assert_eq!(
            fs::read(self.dir.path().join("orr.project.json")).unwrap(),
            self.config
        );
        assert_eq!(
            fs::read(self.dir.path().join("level.scene.yaml")).unwrap(),
            self.scene
        );
    }
}
fn wait(h: &mut Harness<'_, EditorApp>, label: &str, ready: impl Fn(&EditorApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !ready(h.state()) {
        assert!(
            Instant::now() < deadline,
            "{label}: {:?}",
            h.state().editor.status()
        );
        h.run_steps(1);
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn step_ticks(h: &mut Harness<'_, EditorApp>, ticks: u64) {
    let target = h.state().editor.snapshot().unwrap().tick() + ticks;
    wait(h, "authoritative ticks", |app| {
        app.editor.snapshot().is_some_and(|s| s.tick() >= target)
    });
}
fn position(app: &EditorApp) -> FPVec2 {
    app.editor
        .snapshot()
        .unwrap()
        .predicted()
        .iter::<CollectActor>()
        .find(|(_, actor)| actor.kind == PLAYER)
        .unwrap()
        .1
        .position
}
fn run(app: &EditorApp) -> CollectRun {
    *app.editor
        .snapshot()
        .unwrap()
        .predicted()
        .singleton::<CollectRun>()
}
fn key(h: &mut Harness<'_, EditorApp>, logical: Key, physical: Key, down: bool) {
    h.input_mut().events.push(egui::Event::Key {
        key: logical,
        physical_key: Some(physical),
        pressed: down,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
}
fn tap(h: &mut Harness<'_, EditorApp>, physical: Key) {
    key(h, physical, physical, true);
    key(h, physical, physical, false);
}
fn claim(h: &mut Harness<'_, EditorApp>) {
    h.get_by_label("Take control").click();
    wait(h, "claim", |app| app.editor.input_phase() == Phase::Active);
    h.run_steps(2);
}
fn play(h: &mut Harness<'_, EditorApp>) {
    h.get_by_label(orr_editor::app::LBL_PLAY).click();
    wait(h, "play", |app| app.editor.can_take_control());
    claim(h);
}
fn select_map(h: &mut Harness<'_, EditorApp>, path: &Path) {
    h.run_steps(3);
    h.get_by_role_and_label(Role::TextInput, "Collect controls")
        .focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, "Collect controls")
        .type_text(path.to_str().unwrap());
    h.run_steps(2);
    h.get_by_label("Load controls").click();
    h.run_steps(3);
}

#[test]
#[ignore = "explicit producer for the production Keys acceptance; requires ORR_COLLECT_ACCEPTANCE_MAP"]
fn authoring_app_exports_standalone_acceptance_map() {
    let path = std::path::PathBuf::from(
        std::env::var_os("ORR_COLLECT_ACCEPTANCE_MAP")
            .expect("set ORR_COLLECT_ACCEPTANCE_MAP to a new acceptance artifact file"),
    );
    assert!(
        !path.exists(),
        "preserve previous evidence: producer requires a new path"
    );
    let map = authored_map(&path);
    let before = fs::read(&path).unwrap();
    assert_eq!(ActionMap::load_file(&path).unwrap(), map);
    assert_eq!(fs::read(&path).unwrap(), before);
    eprintln!(
        "authoring App saved {} ({} bytes)",
        path.display(),
        before.len()
    );
}

#[test]
fn authored_map_main_app_uses_physical_keys_and_preserves_project_bytes() {
    let project = Project::new();
    let path = project.dir.path().join("controls.json");
    authored_map(&path);
    let bytes = fs::read(&path).unwrap();
    let mut h = project.harness();
    select_map(&mut h, &path);
    project.unchanged();
    play(&mut h);
    key(&mut h, Key::D, Key::D, true);
    tap(&mut h, Key::Space);
    step_ticks(&mut h, 6);
    assert_eq!(position(h.state()), FPVec2::ZERO, "old D is inert");
    assert_eq!(run(h.state()).restart_held, 0, "old Space is inert");
    key(&mut h, Key::D, Key::D, false);
    assert!(h.state_mut().editor.select_named("player"));
    h.state_mut().editor.sync();
    let selected = h
        .state()
        .editor
        .selected_guid()
        .expect("selected player GUID");
    // A logical default movement key with a different physical identity must
    // follow the physical remap, never the host keyboard layout's logical key.
    key(&mut h, Key::Escape, Key::L, true);
    wait(&mut h, "physical L wins", |app| run(app).phase == WON);
    assert_eq!(run(h.state()).score, 2);
    assert_eq!(
        h.state().editor.selected_guid(),
        Some(selected.clone()),
        "logical Escape on physical L must not clear editor selection"
    );
    key(&mut h, Key::Escape, Key::L, false);
    // Logical Tab must likewise preserve physical movement and ownership.
    key(&mut h, Key::Tab, Key::L, true);
    h.run_steps(2);
    assert_eq!(h.state().editor.input_phase(), Phase::Active);
    assert_eq!(
        h.state().editor.selected_guid(),
        Some(selected),
        "logical Tab on physical L must not change editor selection"
    );
    key(&mut h, Key::Tab, Key::L, false);
    key(&mut h, Key::Space, Key::Enter, true);
    key(&mut h, Key::Space, Key::Enter, false);
    wait(&mut h, "physical Enter restarts", |app| {
        run(app).phase == PLAYING && run(app).score == 0
    });
    assert_eq!(position(h.state()), FPVec2::ZERO);
    key(&mut h, Key::Tab, Key::Tab, true);
    wait(&mut h, "unbound physical Tab releases", |app| {
        app.editor.input_phase() == Phase::Off
    });
    key(&mut h, Key::Tab, Key::Tab, false);
    claim(&mut h);
    key(&mut h, Key::Q, Key::Q, true);
    wait(&mut h, "custom quit releases", |app| {
        app.editor.input_phase() == Phase::Off
    });
    assert_eq!(
        h.state().editor.mode(),
        Mode::Play,
        "quit only releases editor control"
    );
    h.get_by_label(orr_editor::app::LBL_STOP).click();
    wait(&mut h, "stop", |app| app.editor.mode() == Mode::Edit);
    assert!(h.state_mut().editor.select_named("player"));
    h.state_mut().editor.sync();
    assert!(h.state().editor.selected_guid().is_some());
    tap(&mut h, Key::Escape);
    h.run_steps(2);
    assert!(
        h.state().editor.selected_guid().is_none(),
        "ordinary uncontrolled Escape retains editor selection cancellation"
    );
    project.unchanged();
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

fn aliased_map(path: &Path) {
    let mut map = orr_sample::collect_app::input_actions::default_map();
    for action in &mut map.actions {
        let keys = match action.name.as_str() {
            "left" => vec![MapKey::J],
            "right" => vec![MapKey::L, MapKey::U],
            "up" => vec![MapKey::I],
            "down" => vec![MapKey::K],
            "restart" => vec![MapKey::Enter, MapKey::R],
            "quit" => vec![MapKey::Q],
            _ => unreachable!(),
        };
        action.bindings = keys
            .into_iter()
            .map(|key| Button::Keyboard { key })
            .collect();
    }
    map.save_new(path).unwrap();
}
#[test]
fn aliases_opposition_and_focus_dialog_stop_reclaim_require_fresh_press() {
    let project = Project::new();
    let path = project.dir.path().join("aliases.json");
    aliased_map(&path);
    let mut h = project.harness();
    select_map(&mut h, &path);
    play(&mut h);
    for k in [Key::L, Key::U, Key::J, Key::I, Key::K] {
        key(&mut h, k, k, true);
    }
    step_ticks(&mut h, 4);
    assert_eq!(position(h.state()), FPVec2::ZERO, "opposing aliases cancel");
    key(&mut h, Key::L, Key::L, false);
    step_ticks(&mut h, 3);
    assert_eq!(
        position(h.state()),
        FPVec2::ZERO,
        "U stays held after L release"
    );
    key(&mut h, Key::J, Key::J, false);
    wait(&mut h, "remaining right alias", |app| {
        position(app).x > FP::ZERO
    });
    h.input_mut().focused = false;
    wait(&mut h, "focus loss releases", |app| {
        app.editor.input_phase() == Phase::Off
    });
    h.input_mut().focused = true;
    h.run_steps(3);
    claim(&mut h);
    step_ticks(&mut h, 3);
    let neutral = position(h.state());
    step_ticks(&mut h, 4);
    assert_eq!(
        position(h.state()),
        neutral,
        "held U must not resume on reclaim"
    );
    key(&mut h, Key::U, Key::U, false);
    key(&mut h, Key::U, Key::U, true);
    wait(&mut h, "fresh U after focus", |app| {
        position(app).x > neutral.x
    });
    h.state_mut().ui.dialog = Some(orr_editor::app::Dialog {
        kind: orr_editor::app::DialogKind::SaveAs,
        text: String::new(),
    });
    wait(&mut h, "dialog releases", |app| {
        app.editor.input_phase() == Phase::Off
    });
    h.state_mut().ui.dialog = None;
    h.run_steps(3);
    claim(&mut h);
    step_ticks(&mut h, 3);
    let neutral = position(h.state());
    step_ticks(&mut h, 4);
    assert_eq!(
        position(h.state()),
        neutral,
        "dialog-held U must not resume"
    );
    h.get_by_label(orr_editor::app::LBL_STOP).click();
    wait(&mut h, "stop", |app| {
        app.editor.mode() == Mode::Edit && app.editor.input_phase() == Phase::Off
    });
    play(&mut h);
    step_ticks(&mut h, 4);
    assert_eq!(
        position(h.state()),
        FPVec2::ZERO,
        "Stop/Play cannot replay held U"
    );
    key(&mut h, Key::U, Key::U, false);
    key(&mut h, Key::U, Key::U, true);
    wait(&mut h, "fresh U after Stop", |app| {
        position(app).x > FP::ZERO
    });
    project.unchanged();
}

#[test]
fn same_path_scene_replacement_discards_session_map() {
    let project = Project::new();
    let path = project.dir.path().join("aliases.json");
    aliased_map(&path);
    let mut h = project.harness();
    select_map(&mut h, &path);
    assert!(h
        .state_mut()
        .editor
        .open_path(&project.dir.path().join("level.scene.yaml")));
    h.state_mut().editor.sync();
    h.run_steps(3);
    play(&mut h);
    key(&mut h, Key::L, Key::L, true);
    step_ticks(&mut h, 4);
    assert_eq!(
        position(h.state()),
        FPVec2::ZERO,
        "previous map cannot cross replacement epoch"
    );
    key(&mut h, Key::L, Key::L, false);
    key(&mut h, Key::D, Key::D, true);
    wait(&mut h, "default restored after replacement", |app| {
        position(app).x > FP::ZERO
    });
    project.unchanged();
}

#[test]
fn mapped_tab_retains_viewport_control_and_physical_escape_always_releases() {
    let project = Project::new();
    let path = project.dir.path().join("tab.json");
    let mut map = orr_sample::collect_app::input_actions::default_map();
    map.actions
        .iter_mut()
        .find(|a| a.name == "right")
        .unwrap()
        .bindings = vec![Button::Keyboard { key: MapKey::Tab }];
    map.save_new(&path).unwrap();
    let mut h = project.harness();
    select_map(&mut h, &path);
    play(&mut h);
    key(&mut h, Key::Tab, Key::Tab, true);
    wait(&mut h, "mapped Tab movement", |app| {
        position(app).x > FP::ZERO
    });
    assert_eq!(h.state().editor.input_phase(), Phase::Active);
    key(&mut h, Key::Tab, Key::Tab, false);
    key(&mut h, Key::L, Key::Escape, true);
    wait(&mut h, "physical Escape release", |app| {
        app.editor.input_phase() == Phase::Off
    });
    assert_eq!(h.state().editor.mode(), Mode::Play);
}

#[test]
fn ordered_restart_aliases_cross_authoritative_ticks() {
    let project = Project::new();
    let path = project.dir.path().join("aliases.json");
    aliased_map(&path);
    let mut h = project.harness();
    select_map(&mut h, &path);
    play(&mut h);
    h.state_mut().editor.set_speed(0.25);
    h.state_mut().editor.sync();
    step_ticks(&mut h, 2);
    // All alias presses occur within one GUI frame. Holding Enter while pressing
    // R must enqueue another independent edge.
    key(&mut h, Key::Enter, Key::Enter, true);
    key(&mut h, Key::R, Key::R, true);
    key(&mut h, Key::Enter, Key::Enter, false);
    key(&mut h, Key::R, Key::R, false);
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut last_tick = h.state().editor.snapshot().unwrap().tick();
    let mut previous_held = false;
    let mut rising_edges = 0;
    let mut neutral_after_second = None;
    loop {
        assert!(
            Instant::now() < deadline,
            "queued restarts did not settle: {rising_edges} observed"
        );
        h.run_steps(1);
        let tick = h.state().editor.snapshot().unwrap().tick();
        if tick != last_tick {
            last_tick = tick;
            let state = run(h.state());
            let held = state.restart_held != 0;
            if held && !previous_held {
                rising_edges += 1;
                assert_eq!(state.elapsed_ticks, 0);
                assert!(
                    rising_edges <= 2,
                    "extra restart beyond the two distinct alias presses"
                );
            }
            if rising_edges == 2 && !held {
                let start = *neutral_after_second.get_or_insert(tick);
                if tick >= start + 3 {
                    break;
                }
            }
            previous_held = held;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(rising_edges, 2);
    assert!(run(h.state()).elapsed_ticks >= 4);
}

#[test]
fn same_frame_restart_then_quit_discards_undelivered_intent() {
    let project = Project::new();
    let path = project.dir.path().join("aliases.json");
    aliased_map(&path);
    let mut h = project.harness();
    select_map(&mut h, &path);
    play(&mut h);
    step_ticks(&mut h, 4);
    let elapsed = run(h.state()).elapsed_ticks;
    tap(&mut h, Key::Enter);
    tap(&mut h, Key::R);
    key(&mut h, Key::Q, Key::Q, true);
    wait(&mut h, "Quit discards frame restart edges", |app| {
        app.editor.input_phase() == Phase::Off
    });
    step_ticks(&mut h, 3);
    assert!(
        run(h.state()).elapsed_ticks >= elapsed,
        "Quit must not deliver pending restart"
    );
    assert_eq!(run(h.state()).restart_held, 0);
}
