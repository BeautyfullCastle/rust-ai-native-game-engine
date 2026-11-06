use super::*;
use orr_input::{Button, Key as InputKey};
type Sim = orr_sim::Simulation<game::DefenseGame>;
fn sim() -> Sim {
    Sim::new(Default::default(), 60, 1)
}
fn key(k: Key, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key: k,
        physical_key: Some(k),
        pressed,
        repeat: false,
        modifiers,
    }
}
fn tap(k: Key) -> Vec<egui::Event> {
    vec![
        key(k, true, Default::default()),
        key(k, false, Default::default()),
    ]
}
fn mouse(x: f32, y: f32, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: egui::pos2(x, y),
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    }
}
fn draw(ui: &mut DefenseUi, sim: &Sim, events: Vec<egui::Event>) -> Option<game::DefenseInput> {
    let (mut output, command) = ui.show(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 940.0))),
            events,
            ..Default::default()
        },
        sim.frame(),
    );
    output.textures_delta.clear();
    command
}
fn click(ui: &mut DefenseUi, sim: &Sim, x: f32, y: f32) {
    let sequence = ui.sequence;
    assert!(draw(ui, sim, vec![mouse(x, y, true)]).is_none());
    assert!(draw(ui, sim, vec![mouse(x, y, false)]).is_none());
    assert_eq!(ui.sequence, sequence);
}
fn started(sim: &Sim) -> DefenseUi {
    let mut ui = DefenseUi::new();
    assert!(draw(&mut ui, sim, tap(Key::Enter)).is_none());
    assert!(ui.started);
    ui
}
fn custom(ui: &mut DefenseUi) {
    for (action, key) in ui.controls.draft.actions.iter_mut().zip([
        InputKey::K,
        InputKey::L,
        InputKey::M,
        InputKey::N,
    ]) {
        action.bindings = vec![Button::Keyboard { key }];
    }
    ui.controls.apply().unwrap();
}
#[test]
fn controls_ui_default_keys_and_event_owned_modifiers_preserve_commands() {
    let sim = sim();
    let mut ui = started(&sim);
    for (key, action) in [
        (Key::Space, game::SUMMON),
        (Key::U, game::UPGRADE),
        (Key::I, game::INVEST),
        (Key::R, game::RESTART),
    ] {
        assert_eq!(draw(&mut ui, &sim, tap(key)).unwrap().action, action);
    }
    for modifiers in [
        egui::Modifiers {
            ctrl: true,
            ..Default::default()
        },
        egui::Modifiers {
            alt: true,
            ..Default::default()
        },
        egui::Modifiers {
            command: true,
            ..Default::default()
        },
        egui::Modifiers {
            mac_cmd: true,
            ..Default::default()
        },
    ] {
        assert!(draw(
            &mut ui,
            &sim,
            vec![
                key(Key::Space, true, modifiers),
                key(Key::Space, false, Default::default())
            ]
        )
        .is_none());
    }
    assert_eq!(
        draw(
            &mut ui,
            &sim,
            vec![
                key(
                    Key::Space,
                    true,
                    egui::Modifiers {
                        shift: true,
                        ..Default::default()
                    }
                ),
                key(Key::Space, false, Default::default())
            ]
        )
        .unwrap()
        .action,
        game::SUMMON
    );
    let repeat = egui::Event::Key {
        key: Key::Space,
        physical_key: Some(Key::Space),
        pressed: true,
        repeat: true,
        modifiers: Default::default(),
    };
    assert!(draw(&mut ui, &sim, vec![repeat]).is_none());
}
#[test]
fn controls_ui_split_click_capture_apply_and_close_require_new_key_and_continue() {
    let sim = sim();
    let mut ui = started(&sim);
    assert!(draw(&mut ui, &sim, vec![mouse(1100.0, 30.0, true)]).is_none());
    assert!(!ui.controls.open);
    assert!(draw(&mut ui, &sim, vec![]).is_none());
    assert!(!ui.controls.open);
    assert!(draw(&mut ui, &sim, vec![mouse(1100.0, 30.0, false)]).is_none());
    assert!(ui.controls.open);
    assert!(ui.paused);
    click(&mut ui, &sim, 500.0, 370.0);
    assert_eq!(ui.controls.waiting, Some(0));
    assert!(draw(&mut ui, &sim, vec![key(Key::K, true, Default::default())]).is_none());
    assert_eq!(ui.controls.hint(0, true), "K");
    assert_eq!(ui.controls.hint(0, false), "SPACE");
    click(&mut ui, &sim, 400.0, 630.0);
    assert_eq!(ui.controls.hint(0, false), "K");
    click(&mut ui, &sim, 770.0, 630.0);
    assert!(!ui.controls.open);
    assert!(ui.paused);
    assert!(draw(&mut ui, &sim, tap(Key::Enter)).is_none());
    assert!(!ui.paused);
    assert!(draw(&mut ui, &sim, vec![key(Key::K, true, Default::default())]).is_none());
    assert!(draw(&mut ui, &sim, vec![key(Key::K, false, Default::default())]).is_none());
    assert_eq!(
        draw(&mut ui, &sim, tap(Key::K)).unwrap().action,
        game::SUMMON
    );
    assert!(draw(&mut ui, &sim, tap(Key::Space)).is_none());
}
#[test]
fn controls_ui_cancel_discards_draft_but_retains_applied_session_and_reset_is_draft_only() {
    let sim = sim();
    let mut ui = started(&sim);
    custom(&mut ui);
    click(&mut ui, &sim, 1100.0, 30.0);
    click(&mut ui, &sim, 500.0, 422.0);
    assert!(draw(&mut ui, &sim, tap(Key::Y)).is_none());
    assert_eq!(ui.controls.hint(1, true), "Y");
    assert!(draw(&mut ui, &sim, tap(Key::Escape)).is_none());
    assert!(!ui.controls.open);
    assert_eq!(ui.controls.hint(1, false), "L");
    click(&mut ui, &sim, 1100.0, 30.0);
    click(&mut ui, &sim, 640.0, 630.0);
    assert_eq!(ui.controls.hint(0, true), "SPACE");
    assert_eq!(ui.controls.hint(0, false), "K");
    click(&mut ui, &sim, 770.0, 630.0);
    assert_eq!(ui.controls.hint(0, false), "K");
}
#[test]
fn controls_ui_capture_rejects_reserved_duplicate_and_modified_keys_without_start_or_actions() {
    let sim = sim();
    let mut ui = DefenseUi::new();
    click(&mut ui, &sim, 1100.0, 30.0);
    click(&mut ui, &sim, 500.0, 370.0);
    for k in [Key::P, Key::Enter, Key::U] {
        assert!(draw(&mut ui, &sim, tap(k)).is_none());
        assert_eq!(ui.controls.waiting, Some(0));
        assert_eq!(ui.controls.hint(0, true), "SPACE");
    }
    assert!(draw(
        &mut ui,
        &sim,
        vec![key(
            Key::K,
            true,
            egui::Modifiers {
                ctrl: true,
                ..Default::default()
            }
        )]
    )
    .is_none());
    assert!(draw(&mut ui, &sim, vec![]).is_none());
    assert_eq!(ui.controls.waiting, Some(0));
    assert!(draw(&mut ui, &sim, vec![key(Key::K, false, Default::default())]).is_none());
    assert!(draw(&mut ui, &sim, tap(Key::K)).is_none());
    assert!(ui.controls.waiting.is_none());
    assert!(!ui.started);
    assert_eq!(ui.sequence, 0);
    click(&mut ui, &sim, 520.0, 630.0);
    assert!(
        !ui.controls.save_requested,
        "Save is disabled without a directory"
    );
}
#[test]
fn controls_ui_physical_keys_share_normalization_with_fixed_pause_and_pointer_actions() {
    let sim = sim();
    let mut ui = started(&sim);
    custom(&mut ui);
    let physical = |logical, physical, pressed| egui::Event::Key {
        key: logical,
        physical_key: Some(physical),
        pressed,
        repeat: false,
        modifiers: Default::default(),
    };
    assert_eq!(
        draw(
            &mut ui,
            &sim,
            vec![
                physical(Key::P, Key::K, true),
                physical(Key::P, Key::K, false)
            ]
        )
        .unwrap()
        .action,
        game::SUMMON
    );
    assert!(draw(
        &mut ui,
        &sim,
        vec![
            physical(Key::K, Key::P, true),
            physical(Key::K, Key::P, false)
        ]
    )
    .is_none());
    assert!(ui.paused);
    assert!(draw(&mut ui, &sim, tap(Key::Enter)).is_none());
    assert!(draw(&mut ui, &sim, tap(Key::Space)).is_none());
    let command = draw(
        &mut ui,
        &sim,
        vec![mouse(350.0, 860.0, true), mouse(350.0, 860.0, false)],
    )
    .unwrap();
    assert_eq!(command.action, game::SUMMON);
}
#[test]
fn controls_ui_modal_owns_board_audio_and_simultaneous_game_keys() {
    let sim = sim();
    let mut ui = started(&sim);
    ui.audio.enabled = true;
    click(&mut ui, &sim, 1100.0, 30.0);
    let muted = ui.audio.muted;
    let sequence = ui.sequence;
    let mut events = vec![
        mouse(580.0, 30.0, true),
        mouse(580.0, 30.0, false),
        mouse(350.0, 860.0, true),
        mouse(350.0, 860.0, false),
    ];
    events.extend(tap(Key::Space));
    events.extend(tap(Key::R));
    events.extend(tap(Key::Enter));
    assert!(draw(&mut ui, &sim, events).is_none());
    assert_eq!(ui.audio.muted, muted);
    assert_eq!(ui.sequence, sequence);
    assert!(ui.paused);
}
#[test]
fn controls_ui_focus_loss_cancels_capture_and_blocks_same_held_key_until_release() {
    let sim = sim();
    let mut ui = started(&sim);
    click(&mut ui, &sim, 1100.0, 30.0);
    click(&mut ui, &sim, 500.0, 370.0);
    assert!(draw(
        &mut ui,
        &sim,
        vec![
            key(Key::K, true, Default::default()),
            egui::Event::WindowFocused(false)
        ]
    )
    .is_none());
    assert!(ui.controls.waiting.is_none());
    assert_eq!(ui.controls.hint(0, true), "SPACE");
    draw(&mut ui, &sim, vec![egui::Event::WindowFocused(true)]);
    click(&mut ui, &sim, 500.0, 370.0);
    assert!(draw(&mut ui, &sim, vec![key(Key::K, true, Default::default())]).is_none());
    assert_eq!(ui.controls.waiting, Some(0));
    draw(&mut ui, &sim, vec![key(Key::K, false, Default::default())]);
    draw(&mut ui, &sim, tap(Key::K));
    assert_eq!(ui.controls.hint(0, true), "K");
}

#[test]
fn controls_ui_held_start_key_cannot_cross_modal_focus_reset() {
    let sim = sim();
    let mut ui = DefenseUi::new();
    click(&mut ui, &sim, 1100.0, 30.0);
    draw(
        &mut ui,
        &sim,
        vec![key(Key::Space, true, Default::default())],
    );
    draw(&mut ui, &sim, vec![egui::Event::WindowFocused(false)]);
    draw(&mut ui, &sim, vec![egui::Event::WindowFocused(true)]);
    click(&mut ui, &sim, 770.0, 630.0);
    assert!(!ui.started);
    draw(
        &mut ui,
        &sim,
        vec![key(Key::Space, true, Default::default())],
    );
    assert!(!ui.started);
    draw(
        &mut ui,
        &sim,
        vec![key(Key::Space, false, Default::default())],
    );
    draw(&mut ui, &sim, tap(Key::Space));
    assert!(ui.started);
}

#[test]
fn controls_ui_ready_and_gameplay_hints_show_active_bindings() {
    let sim = sim();
    let mut ui = DefenseUi::new();
    custom(&mut ui);
    let text = |ui: &mut DefenseUi| {
        let (mut output, command) = ui.show(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 940.0))),
                ..Default::default()
            },
            sim.frame(),
        );
        output.textures_delta.clear();
        assert!(command.is_none());
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let ready = text(&mut ui);
    for hint in ["K or SUMMON", "L / FORGE", "M / INVEST"] {
        assert!(ready.contains(hint), "{hint}");
    }
    for old in ["SPACE or SUMMON", "U / FORGE", "I / INVEST"] {
        assert!(!ready.contains(old));
    }
    ui.started = true;
    let running = text(&mut ui);
    for hint in ["[K] SUMMON", "[L] FORGE", "[M] INVEST", "[N] RESTART"] {
        assert!(running.contains(hint), "{hint}");
    }
}

#[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
#[test]
fn controls_ui_cannot_open_through_run_save_modal_or_mutate_its_selection() {
    let sim = sim();
    let mut ui = started(&sim);
    ui.run_save.enabled = true;
    ui.paused = true;
    ui.run_save.mode = RunSaveMode::List(vec!["saved.orr-run".into()]);
    click(&mut ui, &sim, 1100.0, 30.0);
    assert!(!ui.controls.open);
    assert!(matches!(ui.run_save.mode, RunSaveMode::List(_)));
    assert!(ui.run_save.action.is_none());
}
