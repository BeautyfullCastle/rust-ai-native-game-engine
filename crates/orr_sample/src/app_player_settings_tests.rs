//! Real egui widgets and native App dispatch with manually stepped hosts. This
//! deliberately does not claim physical devices or a native-window run.
use super::project_ui_tests::{app, click, prepare, raw, Steppable};
use super::*;
use crate::game_ui::{Action, Hud, Screen};
use crate::player_controls::PlayerSettingsSession;
use crate::player_settings::{FireBinding, SettingsPaths, SettingsStore};
use orr_bridge::{Pacing, PlayHost, PlayerSlot, Threaded, ThreadedConfig};
use orr_input::{Button, Key};
use std::path::Path;

fn install<B: Bridge<Arena>>(app: &mut App<B>, directory: &Path) {
    let session = PlayerSettingsSession::open(SettingsPaths::from_directory(directory.to_owned()));
    app.controls.replace_map(session.action_map()).unwrap();
    app.ui.as_mut().unwrap().set_player_settings(session);
}

fn exercise<B: Steppable>(mut app: App<B>, profile: &Path) -> Vec<u64> {
    use crate::arena_game::Bullet;
    install(&mut app, profile);
    assert_eq!(
        app.ui.as_ref().unwrap().player_settings().unwrap().active(),
        FireBinding::Space
    );
    click(&mut app, Action::FireLeftMouse);
    click(&mut app, Action::SettingsCancel);
    assert_eq!(
        app.ui.as_ref().unwrap().player_settings().unwrap().draft(),
        FireBinding::Space
    );
    assert!(
        !profile.exists(),
        "Cancel and discovery cannot create a profile"
    );
    click(&mut app, Action::FireLeftMouse);
    click(&mut app, Action::SettingsApply);
    let paths = SettingsPaths::from_directory(profile.to_owned()).unwrap();
    let committed = std::fs::read(paths.primary()).unwrap();
    assert_eq!(
        SettingsStore::new(paths.clone())
            .load()
            .settings
            .fire_binding,
        FireBinding::LeftMouse
    );
    assert_eq!(app.ui.as_ref().unwrap().screen(), Screen::Title);
    // Repeated dispatch is a no-op, not another save or speculative fire.
    app.dispatch_ui_action(Action::SettingsApply).unwrap();
    assert_eq!(std::fs::read(paths.primary()).unwrap(), committed);
    app.bridge.step_one();
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

    click(&mut app, Action::SettingsReset);
    assert_eq!(
        app.ui.as_ref().unwrap().player_settings().unwrap().draft(),
        FireBinding::Space
    );
    assert_eq!(
        std::fs::read(paths.primary()).unwrap(),
        committed,
        "Reset is draft-only"
    );
    click(&mut app, Action::SettingsCancel);
    click(&mut app, Action::Restart);
    assert_eq!(app.bridge.snapshot().unwrap().tick(), 0);
    assert_eq!(
        app.ui.as_ref().unwrap().player_settings().unwrap().active(),
        FireBinding::LeftMouse
    );
    let mut checksums = Vec::new();
    for tick in 0..16 {
        app.controls
            .button(Button::Keyboard { key: Key::D }, tick < 5, false, false);
        app.controls.button(
            Button::Mouse { button: 0 },
            tick == 0 || tick == 1,
            false,
            false,
        );
        app.publish_input_result().unwrap();
        app.bridge.step_one();
        let snapshot = app.bridge.snapshot().unwrap();
        if tick == 0 {
            assert_eq!(
                snapshot.predicted().dense::<Bullet>().1.len(),
                1,
                "saved mouse binding actually fires"
            );
        }
        checksums.push(snapshot.predicted().checksum());
    }
    click(&mut app, Action::Menu);
    app.controls.set_focused(false);
    click(&mut app, Action::FireSpace);
    click(&mut app, Action::SettingsApply);
    click(&mut app, Action::Restart);
    app.controls
        .button(Button::Keyboard { key: Key::Space }, true, false, false);
    assert_eq!(
        app.controls.keys(),
        Keys::default(),
        "Apply and Restart retain focus loss"
    );
    app.controls.set_focused(true);
    app.controls
        .button(Button::Keyboard { key: Key::Space }, true, true, false);
    assert_eq!(
        app.controls.keys(),
        Keys::default(),
        "repeats cannot resurrect input"
    );
    app.controls
        .button(Button::Keyboard { key: Key::Space }, false, false, false);
    app.controls
        .button(Button::Keyboard { key: Key::Space }, true, false, false);
    assert!(app.controls.keys().fire);
    checksums
}

#[test]
fn persisted_widgets_mouse_fire_restart_match_inproc_threaded_and_effective_inputs() {
    let temp = tempfile::tempdir().unwrap();
    let (_fixture, prepared) = prepare();
    let (seed, presentation, ui) = prepared.into_launch_parts();
    let expected_seed = seed.clone();
    let inproc = exercise(
        app(
            seed.bridge().unwrap(),
            Box::new(move || seed.bridge()),
            presentation,
            ui.unwrap().font,
        ),
        &temp.path().join("inproc"),
    );

    let (_fixture, prepared) = prepare();
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
        .map_err(|error| format!("thread: {error:?}"))
    };
    let threaded = exercise(
        app(
            factory().unwrap(),
            Box::new(factory),
            presentation,
            ui.unwrap().font,
        ),
        &temp.path().join("threaded"),
    );
    assert_eq!(inproc, threaded);
    let mut plain = expected_seed.bridge().unwrap();
    let mut checksums = Vec::new();
    for tick in 0..16 {
        plain
            .set_input(
                PlayerSlot(0),
                Keys {
                    right: tick < 5,
                    fire: tick == 0 || tick == 1,
                    ..Default::default()
                }
                .to_input(),
            )
            .unwrap();
        plain.step(1);
        checksums.push(plain.snapshot().unwrap().predicted().checksum());
    }
    assert_eq!(
        inproc, checksums,
        "preferences never change effective-input simulation identity"
    );
}

#[test]
fn failed_save_keeps_active_map_and_visible_error_and_gameplay_available() {
    let temp = tempfile::tempdir().unwrap();
    let profile = temp.path().join("settings");
    let (_fixture, prepared) = prepare();
    let (seed, presentation, ui) = prepared.into_launch_parts();
    let mut app = app(
        seed.bridge().unwrap(),
        Box::new(move || seed.bridge()),
        presentation,
        ui.unwrap().font,
    );
    install(&mut app, &profile);
    let before = app.controls.map().clone();
    click(&mut app, Action::FireLeftMouse);
    // An unanticipated special path appears after admission, before commit.
    std::fs::create_dir_all(profile.join("settings.json")).unwrap();
    click(&mut app, Action::SettingsApply);
    assert_eq!(app.controls.map(), &before);
    let settings = app.ui.as_ref().unwrap().player_settings().unwrap();
    assert_eq!(settings.active(), FireBinding::Space);
    assert!(settings.status().contains("저장 실패"));
    click(&mut app, Action::Play);
    app.controls
        .button(Button::Keyboard { key: Key::Space }, true, false, false);
    assert!(app.controls.keys().fire);
}

#[test]
fn saved_mouse_binding_never_leaks_queued_apply_click_or_stale_dpi_layout() {
    let temp = tempfile::tempdir().unwrap();
    let (_fixture, prepared) = prepare();
    let (seed, presentation, ui) = prepared.into_launch_parts();
    let mut app = app(
        seed.bridge().unwrap(),
        Box::new(move || seed.bridge()),
        presentation,
        ui.unwrap().font,
    );
    install(&mut app, &temp.path().join("settings"));
    click(&mut app, Action::FireLeftMouse);
    click(&mut app, Action::SettingsApply);
    click(&mut app, Action::Play);
    app.ui.as_mut().unwrap().invalidate_pointer_layout();
    let pending = raw(vec![
        egui::Event::PointerMoved(egui::pos2(700.0, 700.0)),
        egui::Event::PointerButton {
            pos: egui::pos2(700.0, 700.0),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        },
    ]);
    let captured = ui_captures_event(app.ui.as_ref().unwrap(), &pending, false);
    assert!(captured);
    app.controls.set_ui_capture(captured);
    app.controls
        .button(Button::Mouse { button: 0 }, true, false, captured);
    app.publish_input_result().unwrap();
    app.bridge.step(1);
    assert!(app
        .bridge
        .snapshot()
        .unwrap()
        .predicted()
        .dense::<crate::arena_game::Bullet>()
        .1
        .is_empty());
    app.ui
        .as_mut()
        .unwrap()
        .show(raw(vec![]), Hud::default())
        .0
        .drop_without_applying_deltas();
    app.controls.set_ui_capture(false);
    app.controls
        .button(Button::Mouse { button: 0 }, true, true, false);
    assert_eq!(app.controls.keys(), Keys::default());
}
