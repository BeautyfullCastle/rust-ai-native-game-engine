use super::*;
use egui::{pos2, Event, Pos2};
type Sim = orr_sim::Simulation<game::DefenseGame>;

fn simulation() -> Sim {
    Sim::new(game::DefenseConfig::default(), 60, 1)
}
fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    }
}
fn click(pos: Pos2) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        button(pos, true),
        button(pos, false),
    ]
}
fn draw(ui: &mut DefenseUi, sim: &Sim, events: Vec<Event>) {
    draw_input(
        ui,
        sim,
        egui::RawInput {
            events,
            ..Default::default()
        },
    );
}
fn draw_input(ui: &mut DefenseUi, sim: &Sim, mut input: egui::RawInput) {
    input
        .screen_rect
        .get_or_insert(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 940.0)));
    let (_, action) = ui.show(input, sim.frame());
    assert!(
        action.is_none(),
        "audio input must never emit a game command"
    );
    assert_eq!(
        ui.sequence, 0,
        "audio input must never consume a command sequence"
    );
}
fn active(sim: &Sim) -> DefenseUi {
    let mut ui = DefenseUi::new();
    ui.audio.enabled = true;
    draw(&mut ui, sim, vec![]);
    ui
}

#[test]
fn defaults_and_disabled_controls_are_silent() {
    let sim = simulation();
    let mut ui = DefenseUi::new();
    assert!(!ui.audio.enabled);
    assert!(ui.audio.muted);
    assert_eq!(ui.audio.gain_percent, 100);
    for pos in [pos2(580.0, 30.0), pos2(690.0, 30.0), pos2(745.0, 30.0)] {
        draw(&mut ui, &sim, click(pos));
    }
    assert!(ui.audio.muted);
    assert_eq!(ui.audio.gain_percent, 100);
    assert!(!ui.started);
}

#[test]
fn mute_repeat_hold_duplicate_press_and_release_are_click_safe() {
    let sim = simulation();
    let mut ui = active(&sim);
    let mute = pos2(580.0, 30.0);
    draw(&mut ui, &sim, vec![button(mute, true)]);
    assert!(ui.audio.muted);
    draw(&mut ui, &sim, vec![]);
    assert!(ui.audio.muted);
    draw(&mut ui, &sim, vec![button(mute, false)]);
    assert!(!ui.audio.muted);
    draw(&mut ui, &sim, vec![button(mute, false)]);
    assert!(!ui.audio.muted);
    draw(&mut ui, &sim, click(mute));
    assert!(ui.audio.muted);
    draw(&mut ui, &sim, vec![button(mute, true)]);
    draw(&mut ui, &sim, vec![button(mute, true), button(mute, false)]);
    assert!(ui.audio.muted);
    for _ in 0..4 {
        draw(&mut ui, &sim, click(mute));
    }
    assert!(ui.audio.muted);
}

#[test]
fn cross_target_drag_focus_resize_and_enable_changes_cancel_press() {
    let sim = simulation();
    let mute = pos2(580.0, 30.0);
    let down = pos2(690.0, 30.0);
    let mut ui = active(&sim);
    draw(&mut ui, &sim, vec![button(mute, true), button(down, false)]);
    draw(
        &mut ui,
        &sim,
        vec![
            button(mute, true),
            Event::PointerMoved(down),
            Event::PointerMoved(mute),
            button(mute, false),
        ],
    );
    assert!(ui.audio.muted);
    assert_eq!(ui.audio.gain_percent, 100);
    draw(&mut ui, &sim, vec![button(mute, true)]);
    draw(&mut ui, &sim, vec![Event::WindowFocused(false)]);
    draw(
        &mut ui,
        &sim,
        vec![Event::WindowFocused(true), button(mute, false)],
    );
    assert!(ui.audio.muted);
    draw_input(
        &mut ui,
        &sim,
        egui::RawInput {
            focused: false,
            events: click(mute),
            ..Default::default()
        },
    );
    assert!(ui.audio.muted);
    draw(
        &mut ui,
        &sim,
        vec![Event::WindowFocused(true), button(mute, true)],
    );
    draw_input(
        &mut ui,
        &sim,
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 940.0))),
            events: vec![button(mute, false)],
            ..Default::default()
        },
    );
    assert!(ui.audio.muted);
    draw(&mut ui, &sim, vec![button(mute, true)]);
    ui.audio.enabled = false;
    draw(&mut ui, &sim, vec![]);
    ui.audio.enabled = true;
    draw(&mut ui, &sim, vec![button(mute, false)]);
    assert!(ui.audio.muted);
}

#[test]
fn gain_is_bounded_and_audio_consumes_simultaneous_game_inputs() {
    let sim = simulation();
    let mut ui = active(&sim);
    ui.started = true;
    ui.selected = Some(0);
    for _ in 0..15 {
        draw(&mut ui, &sim, click(pos2(690.0, 30.0)));
    }
    assert_eq!(ui.audio.gain_percent, 0);
    for _ in 0..15 {
        draw(&mut ui, &sim, click(pos2(745.0, 30.0)));
    }
    assert_eq!(ui.audio.gain_percent, 100);
    let mut events = click(pos2(1050.0, 868.0)); // Restart precedes audio this frame.
    events.extend(click(pos2(580.0, 30.0)));
    events.push(Event::Key {
        key: Key::Space,
        physical_key: Some(Key::Space),
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    });
    draw(&mut ui, &sim, events);
    assert!(!ui.audio.muted);
    assert_eq!(ui.selected, Some(0));
    ui.pause();
    draw(&mut ui, &sim, click(pos2(580.0, 30.0)));
    assert!(ui.audio.muted);
    assert!(ui.paused);
}

#[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
#[test]
fn save_modals_block_audio_and_discard_existing_gestures() {
    let sim = simulation();
    for mode in [
        RunSaveMode::List(vec![]),
        RunSaveMode::Confirm("copy".into()),
        RunSaveMode::Loading,
    ] {
        let mut ui = active(&sim);
        draw(&mut ui, &sim, vec![button(pos2(580.0, 30.0), true)]);
        ui.run_save.mode = mode;
        draw(&mut ui, &sim, vec![button(pos2(580.0, 30.0), false)]);
        draw(&mut ui, &sim, click(pos2(580.0, 30.0)));
        draw(&mut ui, &sim, click(pos2(690.0, 30.0)));
        assert!(ui.audio.muted);
        assert_eq!(ui.audio.gain_percent, 100);
        assert!(ui.run_save.action.is_none());
        ui.run_save.mode = RunSaveMode::Closed;
        draw(&mut ui, &sim, vec![button(pos2(580.0, 30.0), false)]);
        assert!(ui.audio.muted);
    }
}

#[test]
fn safety_pause_keys_dominate_audio_in_both_event_orders() {
    let sim = simulation();
    for key in [Key::P, Key::Escape] {
        for audio_first in [false, true] {
            let mut ui = active(&sim);
            ui.started = true;
            let key_event = Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            };
            let mut events = click(pos2(580.0, 30.0));
            if audio_first {
                events.push(key_event);
            } else {
                events.insert(0, key_event);
            }
            draw(&mut ui, &sim, events);
            assert!(ui.paused);
            assert!(!ui.audio.muted);
        }
    }
}

#[test]
fn first_audio_click_wins_and_pointer_pause_retains_priority() {
    let sim = simulation();
    let mut ui = active(&sim);
    ui.started = true;
    let mut events = click(pos2(580.0, 30.0));
    events.extend(click(pos2(690.0, 30.0)));
    draw(&mut ui, &sim, events);
    assert!(!ui.audio.muted);
    assert_eq!(ui.audio.gain_percent, 100);
    for audio_first in [false, true] {
        ui.paused = false;
        let mut events = if audio_first {
            click(pos2(580.0, 30.0))
        } else {
            click(pos2(1050.0, 762.0))
        };
        events.extend(if audio_first {
            click(pos2(1050.0, 762.0))
        } else {
            click(pos2(580.0, 30.0))
        });
        draw(&mut ui, &sim, events);
        assert!(ui.paused);
        assert!(!ui.audio.muted);
    }
}
