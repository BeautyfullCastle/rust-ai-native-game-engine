//! These tests invoke the same dispatcher as the native window after real widget
//! hit testing, without claiming physical input or native-window coverage.
use super::*;
use crate::{
    game_ui::{Action, GameUi, Hud},
    project_runtime::PreparedRuntime,
};
use orr_bridge::{InProc, Pacing, PlayHost, PlayerSlot, Threaded, ThreadedConfig};
use orr_input::{Button, Key};
use orr_reflect::Guid;
use std::{fs, path::Path};

#[path = "../tests/common/project.rs"]
mod project_fixture;
use project_fixture::ProjectFixture;

trait Steppable: Bridge<Arena> {
    fn step_one(&mut self);
}
impl Steppable for InProc<Arena, PlayHost<Arena>> {
    fn step_one(&mut self) {
        self.step(1);
    }
}
impl Steppable for Threaded<Arena> {
    fn step_one(&mut self) {
        self.step(1);
    }
}

fn prepare() -> (ProjectFixture, PreparedRuntime) {
    let fixture = ProjectFixture::new();
    let project = orr_package::Project::open_for_install(
        &fixture.root,
        orr_package::Runtime::content_only().engine_version,
    )
    .unwrap();
    project
        .install(&[Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/game_ui_font")
            .canonicalize()
            .unwrap()])
        .unwrap();
    let path = fixture.root.join("orr.project.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["entry"]["ui"] = serde_json::json!({"profile":"arena-korean-v1","font":{
        "package":"korean-game-ui", "asset":"OrreryKoreanUI.otf"}});
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    let prepared = PreparedRuntime::open(&fixture.root).unwrap();
    (fixture, prepared)
}

fn app<B: Bridge<Arena>>(
    bridge: B,
    factory: Box<dyn FnMut() -> Result<B, String>>,
    presentation: crate::project_runtime::ProjectPresentation,
    font: Vec<u8>,
) -> App<B> {
    let now = Instant::now();
    let mut map = crate::arena_input::default_map();
    map.actions
        .iter_mut()
        .find(|a| a.name == "fire")
        .unwrap()
        .bindings = vec![Button::Mouse { button: 0 }];
    let opts = Options {
        label: "authored test".into(),
        input_map: Some(map.clone()),
        game_ui_project: None,
        vsync: false,
        audio: AudioMode::Off,
        seconds: None,
        remote_mode: InterpMode::Snapshot,
        view: ViewConfig::default(),
        relay: None,
    };
    let mut controls = crate::arena_input::ArenaControls::new(map).unwrap();
    controls.set_paused(true);
    controls.set_ui_capture(true);
    App {
        bridge,
        restart: Some(factory),
        ui: Some(GameUi::from_font(font, true).unwrap()),
        sprites: None,
        project: Some(presentation),
        audio: ArenaAudio::open(AudioMode::Off).unwrap(),
        view: ViewWorld::new(
            ArenaExtractor {
                remote_mode: opts.remote_mode,
                local_slot: 0,
            },
            opts.view,
        ),
        opts,
        gfx: None,
        keys: Keys::default(),
        controls,
        sent_keys: None,
        items: Vec::new(),
        list: RenderList::new(),
        started: now,
        last_frame: now,
        window_frames: 0,
        window_start: now,
        frames: 0,
        worst_frame: Duration::ZERO,
        summary: Summary::default(),
        error: None,
    }
}

fn raw(events: Vec<egui::Event>) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900.0, 900.0),
        )),
        events,
        ..Default::default()
    }
}
fn click<B: Bridge<Arena>>(app: &mut App<B>, action: Action) {
    for _ in 0..3 {
        app.ui
            .as_mut()
            .unwrap()
            .show(raw(vec![]), Hud::default())
            .0
            .drop_without_applying_deltas();
    }
    let point = app
        .ui
        .as_ref()
        .unwrap()
        .buttons
        .iter()
        .find(|(kind, _)| *kind == action)
        .unwrap()
        .1
        .center();
    app.controls.set_ui_capture(true);
    app.controls
        .button(Button::Mouse { button: 0 }, true, false, true);
    app.ui
        .as_mut()
        .unwrap()
        .show(
            raw(vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ]),
            Hud::default(),
        )
        .0
        .drop_without_applying_deltas();
    app.controls
        .button(Button::Mouse { button: 0 }, false, false, true);
    let (output, dispatched) = app.ui.as_mut().unwrap().show(
        raw(vec![egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }]),
        Hud::default(),
    );
    output.drop_without_applying_deltas();
    assert_eq!(dispatched, Some(action));
    assert_eq!(
        app.dispatch_ui_action(dispatched.unwrap()).unwrap(),
        if action == Action::Restart {
            UiActionOutcome::Restarted
        } else {
            UiActionOutcome::Continue
        },
        "the window frame must discard its old snapshot after restart dispatch"
    );
    assert_eq!(app.keys, Keys::default());
    assert_eq!(app.sent_keys, Some(Keys::default()));
}

fn exercise<B: Steppable>(mut app: App<B>, initial: orr_ecs::Frame, fixture: ProjectFixture) {
    use crate::arena_game::{Bullet, PlayerTag, Position};
    let checksum = initial.checksum();
    let index = app.project.as_ref().unwrap().index();
    let hero = index.entity(&Guid::parse("e_00000001").unwrap()).unwrap();
    let target = index.entity(&Guid::parse("e_00000002").unwrap()).unwrap();
    let target_pos = initial.get::<Position>(target).unwrap().pos;
    assert_eq!(index.len(), 2);
    assert_eq!(initial.dense::<PlayerTag>().1.len(), 2);
    // Once admitted, deletion of the complete source/project cannot change a run
    // or a restart. Fresh admission must now fail normally.
    fs::remove_dir_all(&fixture.root).unwrap();
    assert!(PreparedRuntime::open(&fixture.root).is_err());
    click(&mut app, Action::Play);
    let mut expected = None;
    for _ in 0..3 {
        let mut checksums = Vec::new();
        for tick in 0..12 {
            app.controls
                .button(Button::Keyboard { key: Key::D }, tick < 6, false, false);
            app.controls.button(
                Button::Mouse { button: 0 },
                tick == 2 || tick == 3,
                false,
                false,
            );
            app.publish_input_result().unwrap();
            app.bridge.step_one();
            let update = app.bridge.poll_view();
            let snapshot = update.snapshot.as_ref().unwrap();
            assert_eq!(
                snapshot.predicted().get::<Position>(target).unwrap().pos,
                target_pos,
                "no automatic bot"
            );
            checksums.push(snapshot.predicted().checksum());
            app.project
                .as_mut()
                .unwrap()
                .update(Some(snapshot))
                .unwrap();
            app.view.update_from_bridge(1.0 / 60.0, &update);
        }
        if let Some(expected) = &expected {
            assert_eq!(&checksums, expected);
        } else {
            expected = Some(checksums);
        }
        assert_ne!(
            app.bridge
                .snapshot()
                .unwrap()
                .predicted()
                .get::<Position>(hero)
                .unwrap()
                .pos,
            initial.get::<Position>(hero).unwrap().pos
        );
        assert!(!app.project.as_ref().unwrap().sprites().is_empty());
        click(&mut app, Action::Menu);
        let tick = app.bridge.snapshot().unwrap().tick();
        app.bridge.step_one();
        assert!(
            app.bridge.snapshot().unwrap().tick() > tick,
            "menu does not pause simulation"
        );
        click(&mut app, Action::Continue);
        app.controls
            .button(Button::Keyboard { key: Key::D }, true, true, false);
        assert_eq!(
            app.controls.keys(),
            Keys::default(),
            "resume requires fresh presses"
        );
        click(&mut app, Action::Menu);
        // Queue an old bridge view/event and held fire without draining it.
        app.bridge
            .set_input(
                PlayerSlot(0),
                Keys {
                    fire: true,
                    ..Default::default()
                }
                .to_input(),
            )
            .unwrap();
        app.bridge.step_one();
        app.summary.predicted_hits = 17;
        app.controls.set_focused(false);
        click(&mut app, Action::Restart);
        assert_eq!(app.bridge.snapshot().unwrap().tick(), 0);
        assert_eq!(
            app.bridge.snapshot().unwrap().predicted().checksum(),
            checksum
        );
        assert_eq!(
            app.bridge
                .snapshot()
                .unwrap()
                .predicted()
                .dense::<PlayerTag>()
                .1
                .len(),
            2
        );
        assert_eq!(
            app.bridge
                .snapshot()
                .unwrap()
                .predicted()
                .dense::<Bullet>()
                .1
                .len(),
            0
        );
        assert!(app.project.as_ref().unwrap().sprites().is_empty());
        assert_eq!(app.project.as_ref().unwrap().camera.center, [0.0, 0.0]);
        assert!(app.items.is_empty());
        assert_eq!(app.audio.started(), 0);
        assert_eq!(app.summary.predicted_hits, 0);
        let fresh = app.bridge.poll_view();
        assert!(fresh
            .events
            .iter()
            .all(|event| !matches!(event, BridgeEvent::Sim { .. })));
        app.controls
            .button(Button::Keyboard { key: Key::D }, true, false, false);
        assert_eq!(
            app.controls.keys(),
            Keys::default(),
            "restart retains lost focus"
        );
        app.controls.set_focused(true);
        app.controls
            .button(Button::Keyboard { key: Key::D }, true, true, false);
        assert_eq!(app.controls.keys(), Keys::default());
        // Each next iteration replays the same effective input from exact tick zero.
    }
}

#[test]
fn authored_widgets_restart_inproc_from_admitted_frame_and_replay_identically() {
    let (fixture, prepared) = prepare();
    let initial = prepared.initial_frame().clone();
    let (seed, presentation, ui) = prepared.into_launch_parts();
    let bridge = seed.bridge().unwrap();
    exercise(
        app(
            bridge,
            Box::new(move || seed.bridge()),
            presentation,
            ui.unwrap().font,
        ),
        initial,
        fixture,
    );
}

#[test]
fn authored_widgets_restart_threaded_without_old_input_or_view_mailboxes() {
    let (fixture, prepared) = prepare();
    let initial = prepared.initial_frame().clone();
    let (seed, presentation, ui) = prepared.into_launch_parts();
    let factory = move || {
        let session = seed.session()?;
        Threaded::spawn(
            move || PlayHost::new(session, PlayerSlot(0)),
            crate::arena_view::arena_bridge_config(),
            ThreadedConfig {
                pacing: Pacing::Manual,
                ..Default::default()
            },
        )
        .map_err(|e| format!("thread start: {e:?}"))
    };
    let bridge = factory().unwrap();
    exercise(
        app(bridge, Box::new(factory), presentation, ui.unwrap().font),
        initial,
        fixture,
    );
}

#[test]
fn queued_menu_pointer_events_neutralize_bridge_before_ui_action() {
    use crate::arena_game::{Bullet, Position};
    let (_fixture, prepared) = prepare();
    let hero = prepared
        .index()
        .entity(&Guid::parse("e_00000001").unwrap())
        .unwrap();
    let (seed, presentation, ui) = prepared.into_launch_parts();
    let factory = move || {
        let session = seed.session()?;
        Threaded::spawn(
            move || PlayHost::new(session, PlayerSlot(0)),
            crate::arena_view::arena_bridge_config(),
            ThreadedConfig {
                pacing: Pacing::Manual,
                ..Default::default()
            },
        )
        .map_err(|error| format!("thread start: {error:?}"))
    };
    let mut app = app(
        factory().unwrap(),
        Box::new(factory),
        presentation,
        ui.unwrap().font,
    );
    click(&mut app, Action::Play);
    for _ in 0..3 {
        app.ui
            .as_mut()
            .unwrap()
            .show(
                raw(vec![egui::Event::PointerMoved(egui::pos2(800.0, 800.0))]),
                Hud::default(),
            )
            .0
            .drop_without_applying_deltas();
    }
    let outside = raw(vec![
        egui::Event::PointerMoved(egui::pos2(800.0, 800.0)),
        egui::Event::PointerButton {
            pos: egui::pos2(800.0, 800.0),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        },
    ]);
    let captured = ui_captures_event(app.ui.as_ref().unwrap(), &outside, false);
    assert!(!captured);
    app.controls.set_ui_capture(captured);
    app.controls
        .button(Button::Mouse { button: 0 }, true, false, captured);
    app.controls
        .button(Button::Keyboard { key: Key::D }, true, false, captured);
    app.publish_input_result().unwrap();
    app.bridge.step(1);
    assert_eq!(
        app.bridge
            .snapshot()
            .unwrap()
            .predicted()
            .dense::<Bullet>()
            .1
            .len(),
        1,
        "a genuine outside-HUD press remains gameplay input"
    );
    app.controls
        .button(Button::Mouse { button: 0 }, false, false, false);
    let point = app
        .ui
        .as_ref()
        .unwrap()
        .buttons
        .iter()
        .find(|(action, _)| *action == Action::Menu)
        .unwrap()
        .1
        .center();
    let pending = raw(vec![
        egui::Event::PointerMoved(point),
        egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        },
    ]);
    assert!(
        !app.ui.as_ref().unwrap().context.egui_wants_pointer_input(),
        "previous frame still points outside HUD"
    );
    let captured = ui_captures_event(app.ui.as_ref().unwrap(), &pending, false);
    assert!(captured);
    app.controls.set_ui_capture(captured);
    app.controls
        .button(Button::Mouse { button: 0 }, true, false, captured);
    app.publish_input_result().unwrap();
    let before = app
        .bridge
        .snapshot()
        .unwrap()
        .predicted()
        .get::<Position>(hero)
        .unwrap()
        .pos;
    app.bridge.step(1); // The actual threaded host ticks before any redraw/UI action.
    let snapshot = app.bridge.snapshot().unwrap();
    let frame = snapshot.predicted();
    assert_eq!(frame.get::<Position>(hero).unwrap().pos, before);
    assert_eq!(
        frame.dense::<Bullet>().1.len(),
        1,
        "Menu click cannot publish another fire"
    );
    assert_eq!(
        app.ui.as_ref().unwrap().screen(),
        crate::game_ui::Screen::Playing,
        "neutralization happened before any Menu action"
    );
    app.controls.set_focused(false);
    app.controls
        .button(Button::Mouse { button: 0 }, false, false, true);
    app.controls.set_ui_capture(false);
    app.controls
        .button(Button::Mouse { button: 0 }, true, false, false);
    assert_eq!(app.controls.keys(), Keys::default());
}
