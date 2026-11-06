use super::*;
use egui::{Event, Modifiers};

type Sim = orr_sim::Simulation<game::DefenseGame>;

fn key(key: Key, modifiers: Modifiers, pressed: bool) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat: false,
        modifiers,
    }
}

fn draw(
    ui: &mut DefenseUi,
    sim: &Sim,
    mut events: Vec<Event>,
    final_modifiers: Modifiers,
) -> Option<game::DefenseInput> {
    events.push(Event::ModifiersChanged(final_modifiers));
    ui.show(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 940.0))),
            events,
            ..Default::default()
        },
        sim.frame(),
    )
    .1
}

fn apply(sim: &mut Sim, command: game::DefenseInput) {
    let mut inputs = orr_sim::TickInputs::new(sim.frame().tick(), 1);
    inputs.set_input(orr_sim::PlayerSlot(0), command);
    sim.step(&inputs);
}

fn fixture() -> (DefenseUi, Sim) {
    let mut sim = Sim::new(game::DefenseConfig::default(), 60, 1);
    apply(
        &mut sim,
        game::DefenseInput {
            action: game::SUMMON,
            sequence: 7,
            ..Default::default()
        },
    );
    let mut ui = DefenseUi::new();
    ui.started = true;
    ui.selected = Some(0);
    assert!(draw(&mut ui, &sim, vec![], Modifiers::NONE).is_none());
    assert_eq!(ui.sequence, 7);
    assert_eq!(game::run(sim.frame()).tick, 1);
    (ui, sim)
}

fn shortcut_modifiers() -> Vec<Modifiers> {
    let mut modifiers = Vec::new();
    for shift in [false, true] {
        for bits in 1u8..16 {
            modifiers.push(Modifiers {
                alt: bits & 1 != 0,
                ctrl: bits & 2 != 0,
                mac_cmd: bits & 4 != 0,
                command: bits & 8 != 0,
                shift,
            });
        }
    }
    modifiers
}

#[test]
fn modified_economy_and_restart_preserve_real_simulation_and_sequence() {
    for modifiers in shortcut_modifiers() {
        for key_code in [Key::Space, Key::U, Key::I, Key::R] {
            let (mut ui, mut sim) = fixture();
            let before = sim.frame().to_bytes();
            let command = draw(
                &mut ui,
                &sim,
                vec![key(key_code, modifiers, true)],
                Modifiers::NONE,
            );
            if let Some(command) = command {
                apply(&mut sim, command);
            }
            assert_eq!(sim.frame().to_bytes(), before, "{key_code:?} {modifiers:?}");
            assert!(command.is_none());
            assert_eq!(ui.sequence, 7);
            assert_eq!(ui.selected, Some(0));
            assert!(!ui.paused);
        }
    }
}

#[test]
fn modified_start_pause_and_continue_do_not_change_local_state() {
    for modifiers in shortcut_modifiers() {
        for key_code in [Key::Enter, Key::Space] {
            let sim = Sim::new(game::DefenseConfig::default(), 60, 1);
            let mut ui = DefenseUi::new();
            assert!(draw(
                &mut ui,
                &sim,
                vec![key(key_code, modifiers, true)],
                Modifiers::NONE
            )
            .is_none());
            assert!(!ui.started, "{key_code:?} {modifiers:?}");
            assert_eq!(ui.sequence, 0);
        }
        for key_code in [Key::P, Key::Escape] {
            let (mut ui, sim) = fixture();
            assert!(draw(
                &mut ui,
                &sim,
                vec![key(key_code, modifiers, true)],
                Modifiers::NONE
            )
            .is_none());
            assert!(!ui.paused, "{key_code:?} {modifiers:?}");
            assert_eq!(ui.selected, Some(0));
            assert_eq!(ui.sequence, 7);
        }
        let (mut ui, sim) = fixture();
        ui.pause();
        assert!(draw(
            &mut ui,
            &sim,
            vec![key(Key::Enter, modifiers, true)],
            Modifiers::NONE
        )
        .is_none());
        assert!(ui.paused, "{modifiers:?}");
        assert_eq!(ui.sequence, 7);
    }
}

#[test]
fn plain_and_shift_actions_still_reach_simulation() {
    for modifiers in [Modifiers::NONE, Modifiers::SHIFT] {
        for (key_code, action) in [
            (Key::Space, game::SUMMON),
            (Key::U, game::UPGRADE),
            (Key::I, game::INVEST),
            (Key::R, game::RESTART),
        ] {
            let (mut ui, mut sim) = fixture();
            // The final modifier state must not retroactively modify this event.
            let command = draw(
                &mut ui,
                &sim,
                vec![key(key_code, modifiers, true)],
                Modifiers::CTRL,
            )
            .unwrap();
            assert_eq!(command.action, action);
            assert_eq!(command.sequence, 8);
            apply(&mut sim, command);
            let run = game::run(sim.frame());
            assert_eq!(run.last_sequence, 8);
            match action {
                game::SUMMON => assert_eq!(run.gold[0], 100),
                game::UPGRADE => assert_eq!(run.upgrades[0], 1),
                game::INVEST => assert_eq!(run.investment[0], 1),
                game::RESTART => assert_eq!(run.tick, 0),
                _ => unreachable!(),
            }
        }
    }
}

#[test]
fn plain_and_shift_start_pause_continue_are_preserved() {
    for modifiers in [Modifiers::NONE, Modifiers::SHIFT] {
        for key_code in [Key::Enter, Key::Space] {
            let sim = Sim::new(game::DefenseConfig::default(), 60, 1);
            let mut ui = DefenseUi::new();
            assert!(draw(
                &mut ui,
                &sim,
                vec![key(key_code, modifiers, true)],
                Modifiers::CTRL
            )
            .is_none());
            assert!(ui.started);
        }
        for key_code in [Key::P, Key::Escape] {
            let (mut ui, sim) = fixture();
            assert!(draw(
                &mut ui,
                &sim,
                vec![key(key_code, modifiers, true)],
                Modifiers::CTRL
            )
            .is_none());
            assert!(ui.paused);
            assert_eq!(ui.selected, None);
            assert!(draw(
                &mut ui,
                &sim,
                vec![key(Key::Enter, modifiers, true)],
                Modifiers::CTRL
            )
            .is_none());
            assert!(!ui.paused);
            assert_eq!(ui.sequence, 7);
        }
    }
}

#[test]
fn mixed_frame_uses_each_events_modifiers_without_masking_plain_action() {
    for modified_key in [Key::R, Key::P, Key::Escape] {
        for reverse in [false, true] {
            let (mut ui, sim) = fixture();
            let mut events = vec![
                key(modified_key, Modifiers::CTRL, true),
                key(Key::Space, Modifiers::NONE, true),
            ];
            if reverse {
                events.reverse();
            }
            let command = draw(&mut ui, &sim, events, Modifiers::CTRL).unwrap();
            assert_eq!(command.action, game::SUMMON);
            assert_eq!(command.sequence, 8);
            assert!(!ui.paused);
        }
    }
}

#[test]
fn modifier_release_does_not_replay_old_key_and_fresh_press_still_works() {
    let (mut ui, sim) = fixture();
    assert!(draw(
        &mut ui,
        &sim,
        vec![key(Key::R, Modifiers::CTRL, true)],
        Modifiers::CTRL
    )
    .is_none());
    assert!(draw(&mut ui, &sim, vec![], Modifiers::NONE).is_none());
    // A repeated down after modifiers changed is still held, not a fresh press.
    assert!(draw(
        &mut ui,
        &sim,
        vec![key(Key::R, Modifiers::NONE, true)],
        Modifiers::NONE
    )
    .is_none());
    assert!(draw(
        &mut ui,
        &sim,
        vec![key(Key::R, Modifiers::NONE, false)],
        Modifiers::NONE
    )
    .is_none());
    let command = draw(
        &mut ui,
        &sim,
        vec![key(Key::R, Modifiers::NONE, true)],
        Modifiers::NONE,
    )
    .unwrap();
    assert_eq!(command.action, game::RESTART);
    assert_eq!(command.sequence, 8);
}

#[test]
fn modified_keyboard_does_not_cancel_valid_pointer_action() {
    let (mut ui, sim) = fixture();
    let pos = egui::pos2(350.0, 865.0);
    let mut events = vec![key(Key::R, Modifiers::CTRL, true)];
    for pressed in [true, false] {
        events.push(Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::CTRL,
        });
    }
    let command = draw(&mut ui, &sim, events, Modifiers::CTRL).unwrap();
    assert_eq!(command.action, game::SUMMON);
    assert_eq!(command.sequence, 8);
}

#[test]
fn ctrl_restart_cannot_reset_a_started_run() {
    let (mut ui, mut sim) = fixture();
    let before = sim.frame().to_bytes();
    let command = draw(
        &mut ui,
        &sim,
        vec![key(Key::R, Modifiers::CTRL, true)],
        Modifiers::NONE,
    );
    if let Some(command) = command {
        apply(&mut sim, command);
    }
    assert!(
        sim.frame().to_bytes() == before,
        "Ctrl+R changed a live run"
    );
    assert_eq!(game::run(sim.frame()).tick, 1);
    assert_eq!(game::run(sim.frame()).gold[0], 125);
    assert_eq!(game::run(sim.frame()).last_sequence, 7);
    assert_eq!(ui.sequence, 7);
    assert_eq!(ui.selected, Some(0));
}

#[test]
fn focus_loss_and_explicit_repeat_still_block_plain_shortcuts() {
    for modifiers in [Modifiers::NONE, Modifiers::SHIFT] {
        for blocked_by in 0..3 {
            let (mut ui, sim) = fixture();
            let mut event = key(Key::R, modifiers, true);
            if blocked_by == 2 {
                let Event::Key { repeat, .. } = &mut event else {
                    unreachable!()
                };
                *repeat = true;
            }
            let mut events = vec![event];
            if blocked_by == 1 {
                events.push(Event::WindowFocused(false));
            }
            let action = ui
                .show(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            egui::Pos2::ZERO,
                            vec2(1200.0, 940.0),
                        )),
                        events,
                        focused: blocked_by != 0,
                        ..Default::default()
                    },
                    sim.frame(),
                )
                .1;
            assert!(action.is_none());
            assert_eq!(ui.sequence, 7);
            assert!(ui.started);
            assert!(!ui.paused);
        }
    }
}

#[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
#[test]
fn save_modal_escape_uses_event_modifiers_and_keeps_plain_cancel() {
    let modes = [
        RunSaveMode::List(vec!["copy-1.json".into()]),
        RunSaveMode::Confirm("copy-1.json".into()),
        RunSaveMode::Loading,
    ];
    for mode in modes {
        for modifiers in shortcut_modifiers() {
            let (mut ui, sim) = fixture();
            ui.pause();
            ui.run_save.enabled = true;
            ui.run_save.mode = mode.clone();
            assert!(draw(
                &mut ui,
                &sim,
                vec![key(Key::Escape, modifiers, true)],
                Modifiers::NONE
            )
            .is_none());
            assert_eq!(ui.run_save.mode, mode);
            assert_eq!(ui.run_save.action, None);
            assert!(ui.paused);
            assert_eq!(ui.sequence, 7);
        }
        for modifiers in [Modifiers::NONE, Modifiers::SHIFT] {
            let (mut ui, sim) = fixture();
            ui.pause();
            ui.run_save.enabled = true;
            ui.run_save.mode = mode.clone();
            assert!(draw(
                &mut ui,
                &sim,
                vec![key(Key::Escape, modifiers, true)],
                Modifiers::CTRL
            )
            .is_none());
            assert_eq!(ui.run_save.action, Some(RunSaveAction::Cancel));
            assert!(ui.paused);
            assert_eq!(ui.sequence, 7);
        }
    }
}
