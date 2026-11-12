//! CPU-only egui -> native App -> delayed Session acceptance, without a window/GPU.
use super::pause_tests::{app, click, frame, pulse};
use super::*;
const TICK: f64 = 1.0 / 60.0;
fn cell(cell: u32) -> egui::Pos2 {
    crate::defense_view::Layout::new(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(1200.0, 940.0),
    ))
    .cell(0, cell)
    .center()
}
fn cells(a: &mut App, index: u32, delta: f64) {
    let p = cell(index);
    frame(a, click(p.x, p.y), delta);
}
fn settle(a: &mut App) {
    for _ in 0..40 {
        if !a.ui.board_pending {
            return;
        }
        frame(a, vec![], TICK);
    }
    panic!("board did not settle");
}
fn summoned() -> App {
    let mut a = app();
    frame(&mut a, pulse(egui::Key::Space), TICK);
    settle(&mut a);
    a
}
fn unit(a: &App, cell: u32) -> game::DefenseUnit {
    game::units(a.session.predicted_frame())
        .into_iter()
        .find(|u| u.board == 0 && u.cell == cell)
        .unwrap()
}
fn output_has(output: &egui::FullOutput, needle: &str) -> bool {
    fn has(shape: &egui::epaint::Shape, needle: &str) -> bool {
        match shape {
            egui::epaint::Shape::Text(text) => text.galley.job.text.contains(needle),
            egui::epaint::Shape::Vec(shapes) => shapes.iter().any(|s| has(s, needle)),
            _ => false,
        }
    }
    output.shapes.iter().any(|shape| has(&shape.shape, needle))
}
fn output(a: &mut App) -> egui::FullOutput {
    a.process_input(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 940.0),
            )),
            ..Default::default()
        },
        0.0,
    )
}
#[test]
fn queued_move_status_ignores_clicks_and_multistep_redraw_settles() {
    let mut a = summoned();
    let id = unit(&a, 0).id;
    cells(&mut a, 0, 0.0);
    cells(&mut a, 1, 0.0);
    assert!(a.ui.board_pending);
    assert_eq!(a.commands.len(), 1);
    cells(&mut a, 2, 0.0);
    assert_eq!(a.ui.selected, Some(1));
    assert_eq!(a.commands.len(), 1);
    assert!(output_has(
        &output(&mut a),
        "Board action pending. Click again when ready."
    ));
    frame(&mut a, vec![], 0.1);
    assert!(!a.ui.board_pending);
    assert_eq!(unit(&a, 1).id, id);
    assert_eq!(a.ui.selected, Some(1));
    assert!(!output_has(&output(&mut a), "Board action pending"));
    frame(&mut a, vec![], 0.1);
    assert_eq!(unit(&a, 1).id, id);
    assert_eq!(unit(&a, 2).active, 0);
}
#[test]
fn multiple_queued_summons_and_restart_wait_through_last_absolute_tick() {
    let mut a = summoned();
    frame(&mut a, pulse(egui::Key::Space), 0.0);
    frame(&mut a, pulse(egui::Key::R), 0.0);
    assert_eq!(a.commands.len(), 2);
    let last = a.session.next_send_tick() + 1;
    frame(&mut a, vec![], TICK);
    frame(&mut a, vec![], TICK);
    assert!(a.commands.is_empty());
    assert_eq!(a.board.through, Some(last));
    while a.session.predicted_frame().tick() < last {
        assert!(a.ui.board_pending);
        cells(&mut a, 0, TICK);
    }
    assert!(!a.ui.board_pending);
    assert_eq!(game::run(a.session.predicted_frame()).tick, 0);
    assert_eq!(game::run(a.session.predicted_frame()).generation, 1);
    assert_eq!(a.ui.selected, None);
}
#[test]
fn full_queue_rejects_optimistic_selection_without_a_frontier() {
    let mut a = summoned();
    cells(&mut a, 0, 0.0);
    a.commands
        .extend(std::iter::repeat_n(game::DefenseInput::default(), 32));
    cells(&mut a, 1, 0.0);
    assert_eq!(a.ui.selected, Some(0));
    assert!(!a.ui.board_pending);
    assert_eq!(a.board.through, None);
    assert!(a.board.selection.is_none());
    assert_eq!(a.commands.len(), 32);
}
#[test]
fn pause_and_focus_discard_intent_but_admitted_move_still_finishes() {
    for focus in [false, true] {
        let mut a = summoned();
        let id = unit(&a, 0).id;
        cells(&mut a, 0, 0.0);
        cells(&mut a, 1, TICK);
        assert!(a.commands.is_empty());
        if focus {
            a.set_focused(false);
            a.set_focused(true);
            frame(&mut a, vec![], 0.0);
        } else {
            frame(&mut a, pulse(egui::Key::P), 0.0);
        }
        assert!(a.ui.paused);
        assert!(a.board.selection.is_none());
        assert_eq!(a.ui.selected, None);
        let through = a.board.through;
        frame(&mut a, vec![], 0.1);
        assert_eq!(a.board.through, through);
        frame(&mut a, pulse(egui::Key::Enter), 0.0);
        cells(&mut a, 0, 0.0);
        assert_eq!(a.ui.selected, None);
        settle(&mut a);
        assert_eq!(unit(&a, 1).id, id);
        assert_eq!(a.ui.selected, None);
        cells(&mut a, 1, 0.0);
        assert_eq!(a.ui.selected, Some(1));
    }
}
#[test]
fn held_pending_cell_release_cannot_become_a_fresh_click() {
    let mut a = summoned();
    cells(&mut a, 0, 0.0);
    cells(&mut a, 1, TICK);
    let p = cell(2);
    let button = |pressed| egui::Event::PointerButton {
        pos: p,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    frame(&mut a, vec![button(true)], 0.0);
    settle(&mut a);
    frame(&mut a, vec![button(false)], 0.0);
    assert!(a.commands.is_empty());
    assert_eq!(a.ui.selected, Some(1));
    cells(&mut a, 2, TICK);
    settle(&mut a);
    assert_eq!(a.ui.selected, Some(2));
    assert_ne!(unit(&a, 2).active, 0);
}
#[test]
fn merge_retains_target_identity_and_incremented_tier() {
    let mut a = app();
    let mut config = game::DefenseConfig::default();
    config.scenario.starting_gold = 1000;
    a.session = session(config);
    // Find a pair via normal summons and deterministic Session play, not fixture mutation.
    let pair = (0..12)
        .find_map(|_| {
            frame(&mut a, pulse(egui::Key::Space), TICK);
            settle(&mut a);
            let units: Vec<_> = game::units(a.session.predicted_frame())
                .into_iter()
                .filter(|u| u.board == 0 && u.active != 0)
                .collect();
            units.iter().find_map(|source| {
                units
                    .iter()
                    .find(|target| {
                        source.cell != target.cell
                            && source.role == target.role
                            && source.tier == target.tier
                    })
                    .map(|target| (*source, *target))
            })
        })
        .expect("default deterministic summons should produce a matching pair");
    let expected = ExpectedSelection::capture(
        a.session.predicted_frame(),
        game::DefenseInput {
            action: game::MOVE_OR_MERGE,
            source_cell: pair.0.cell,
            target_cell: pair.1.cell,
            sequence: 99,
        },
    )
    .unwrap();
    assert!(
        !expected.matches(a.session.predicted_frame()),
        "an occupied target is not a consumed merge"
    );
    cells(&mut a, pair.0.cell, 0.0);
    cells(&mut a, pair.1.cell, TICK);
    assert!(a.ui.board_pending);
    settle(&mut a);
    assert_eq!(a.ui.selected, Some(pair.1.cell));
    assert_eq!(unit(&a, pair.1.cell).id, pair.1.id);
    assert_eq!(unit(&a, pair.1.cell).tier, pair.1.tier + 1);
    assert_eq!(unit(&a, pair.0.cell).active, 0);
    assert!(expected.matches(a.session.predicted_frame()));
}
#[test]
fn failed_predecessor_and_noop_do_not_select_unrelated_destination() {
    let mut a = summoned();
    // Already-admitted source move makes the UI's snapshot stale. Keep the
    // direct admission explicit: the command and its outcome still use Session.
    let first = game::DefenseInput {
        action: game::MOVE_OR_MERGE,
        source_cell: 0,
        target_cell: 2,
        sequence: 2,
    };
    a.session.advance(first, vec![]);
    cells(&mut a, 0, 0.0);
    cells(&mut a, 1, TICK); // duplicate sequence and missing source at consumption
    assert!(a.ui.board_pending);
    settle(&mut a);
    assert_eq!(a.ui.selected, None);
    assert_ne!(unit(&a, 2).active, 0);
    assert_eq!(unit(&a, 1).active, 0);
}
#[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
#[test]
fn sequence_exhaustion_never_creates_pending_or_changes_selection() {
    let mut a = summoned();
    a.ui.restore_paused(u32::MAX);
    a.ui.paused = false;
    cells(&mut a, 0, 0.0);
    cells(&mut a, 1, 0.0);
    assert_eq!(a.ui.selected, Some(0));
    assert!(a.commands.is_empty());
    assert!(!a.ui.board_pending);
    assert!(a.board.through.is_none());
}
#[test]
fn queued_but_unsent_move_is_canceled_by_pause() {
    let mut a = summoned();
    cells(&mut a, 0, 0.0);
    cells(&mut a, 1, 0.0);
    frame(&mut a, pulse(egui::Key::P), 0.0);
    assert!(a.commands.is_empty());
    assert!(a.board.selection.is_none());
    frame(&mut a, pulse(egui::Key::Enter), 0.0);
    settle(&mut a);
    assert_eq!(a.ui.selected, None);
    assert_ne!(unit(&a, 0).active, 0);
    assert_eq!(unit(&a, 1).active, 0);
}
#[test]
fn terminal_board_still_consumes_restart_and_clears_guard() {
    let mut a = app();
    let mut config = game::DefenseConfig::default();
    config.scenario.starting_gold = 0;
    config.scenario.reactor_hp = 1;
    config.enemy_health.multiplier = 4;
    a.session = session(config);
    // Fast-forward ordinary default Session inputs to a natural loss.
    for _ in 0..30_000 {
        if game::run(a.session.predicted_frame()).phase != game::PLAYING {
            break;
        }
        a.session.advance(game::DefenseInput::default(), vec![]);
    }
    assert_eq!(game::run(a.session.predicted_frame()).phase, game::LOST);
    frame(&mut a, pulse(egui::Key::R), TICK);
    assert!(a.ui.board_pending);
    cells(&mut a, 0, 0.0);
    assert_eq!(a.ui.selected, None);
    settle(&mut a);
    assert_eq!(game::run(a.session.predicted_frame()).generation, 1);
    assert_eq!(game::run(a.session.predicted_frame()).phase, game::PLAYING);
}

mod terminal_focus {
    use super::*;

    #[derive(Clone, Copy, Debug)]
    enum FocusRoute {
        Window,
        Event,
        RawInput,
        BatchedEvents,
    }

    const ROUTES: [FocusRoute; 4] = [
        FocusRoute::Window,
        FocusRoute::Event,
        FocusRoute::RawInput,
        FocusRoute::BatchedEvents,
    ];

    fn terminal(phase: u32) -> App {
        let mut a = app();
        let mut config = game::DefenseConfig::default();
        config.scenario.starting_gold = 0;
        config.scenario.reactor_hp = if phase == game::WON { 1000 } else { 1 };
        config.enemy_health.multiplier = 4;
        if phase == game::WON {
            config.campaign.last_wave = 1;
        }
        a.session = session(config);
        // Natural terminal state through the same neutral Session inputs as play.
        for _ in 0..30_000 {
            if game::run(a.session.predicted_frame()).phase != game::PLAYING {
                break;
            }
            a.session.advance(game::DefenseInput::default(), vec![]);
        }
        assert_eq!(game::run(a.session.predicted_frame()).phase, phase);
        a
    }

    fn lose_and_return(a: &mut App, route: FocusRoute) {
        match route {
            FocusRoute::Window => {
                a.set_focused(false);
                a.set_focused(true);
                frame(a, vec![], 0.0);
            }
            FocusRoute::Event => {
                frame(a, vec![egui::Event::WindowFocused(false)], 0.0);
                frame(a, vec![egui::Event::WindowFocused(true)], 0.0);
            }
            FocusRoute::BatchedEvents => {
                frame(
                    a,
                    vec![
                        egui::Event::WindowFocused(false),
                        egui::Event::WindowFocused(true),
                    ],
                    0.0,
                );
            }
            FocusRoute::RawInput => {
                let mut output = a.process_input(
                    egui::RawInput {
                        focused: false,
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1200.0, 940.0),
                        )),
                        ..Default::default()
                    },
                    0.0,
                );
                output.textures_delta.clear(); // CPU-only harness has no renderer for uploads.
                frame(a, vec![], 0.0);
            }
        }
    }

    fn admitted_restart_requires_continue(route: FocusRoute, phase: u32) {
        let mut a = terminal(phase);
        frame(&mut a, pulse(egui::Key::R), TICK);
        assert!(a.commands.is_empty());
        assert!(a.ui.board_pending);
        assert_eq!(game::run(a.session.predicted_frame()).phase, phase);
        let before = a.session.predicted_frame().to_bytes();
        let pending_tick = a.board.through;
        let tick = a.session.predicted_frame().tick();
        lose_and_return(&mut a, route);
        // Include enough redraw time to consume the admitted Restart on the
        // unfixed path, rather than only checking a presentation flag.
        for delta in [0.0, 0.1, 3600.0] {
            frame(&mut a, vec![], delta);
        }
        let run = game::run(a.session.predicted_frame());
        assert_eq!(
            (run.phase, run.generation),
            (phase, 0),
            "{route:?}: focus return consumed Restart before Continue; tick {tick} -> {}",
            a.session.predicted_frame().tick()
        );
        assert!(a.ui.paused);
        assert!(a.session.predicted_frame().to_bytes() == before);
        assert!(a.ui.board_pending);
        assert_eq!(a.board.through, pending_tick);
        assert!(a.commands.is_empty());

        frame(&mut a, pulse(egui::Key::Enter), 3600.0);
        assert!(!a.ui.paused);
        assert!(a.session.predicted_frame().to_bytes() == before);
        assert_eq!(pending_tick, Some(tick + 2));
        frame(&mut a, vec![], TICK);
        assert_eq!(game::run(a.session.predicted_frame()).generation, 0);
        assert!(a.ui.board_pending);
        frame(&mut a, vec![], TICK);
        let run = game::run(a.session.predicted_frame());
        assert_eq!((run.phase, run.generation, run.tick), (game::PLAYING, 1, 0));
        assert_eq!(run.last_sequence, 1);
        assert!(!a.ui.board_pending);
        frame(&mut a, vec![], 0.1);
        assert_eq!(game::run(a.session.predicted_frame()).generation, 1);
        assert!(a.error.is_none());
    }

    #[test]
    fn admitted_restart_waits_for_continue_after_window_focus_return() {
        for phase in [game::LOST, game::WON] {
            admitted_restart_requires_continue(FocusRoute::Window, phase);
        }
    }

    #[test]
    fn admitted_restart_waits_for_continue_after_focus_event_return() {
        for phase in [game::LOST, game::WON] {
            admitted_restart_requires_continue(FocusRoute::Event, phase);
        }
    }

    #[test]
    fn admitted_restart_waits_for_continue_after_raw_input_focus_return() {
        for phase in [game::LOST, game::WON] {
            admitted_restart_requires_continue(FocusRoute::RawInput, phase);
        }
    }

    #[test]
    fn admitted_restart_waits_for_continue_after_batched_focus_return() {
        for phase in [game::LOST, game::WON] {
            admitted_restart_requires_continue(FocusRoute::BatchedEvents, phase);
        }
    }

    #[test]
    fn idle_terminal_focus_return_does_not_require_continue() {
        for phase in [game::LOST, game::WON] {
            for route in ROUTES {
                let mut a = terminal(phase);
                assert!(!a.ui.board_pending);
                lose_and_return(&mut a, route);
                // Cancellation deliberately records even the neutral admission tail.
                assert!(a.board.through.is_some());
                lose_and_return(&mut a, route);
                assert!(!a.ui.paused, "{route:?}: neutral tail must not pause");
                frame(&mut a, vec![], 0.1);
                assert!(
                    !a.ui.paused,
                    "{route:?}: idle terminal must remain unpaused"
                );
                assert_eq!(game::run(a.session.predicted_frame()).generation, 0);
                frame(&mut a, pulse(egui::Key::R), 0.1);
                assert_eq!(game::run(a.session.predicted_frame()).generation, 1);
            }
        }
    }

    #[test]
    fn unsent_terminal_restart_is_canceled_without_requiring_continue() {
        for phase in [game::LOST, game::WON] {
            for route in ROUTES {
                let mut a = terminal(phase);
                frame(&mut a, pulse(egui::Key::R), 0.0);
                assert_eq!(a.commands.len(), 1);
                assert_eq!(a.commands.front().unwrap().action, game::RESTART);
                assert!(a.ui.board_pending);
                assert!(a.board.through.is_none());
                lose_and_return(&mut a, route);
                assert!(a.commands.is_empty());
                assert!(!a.ui.paused, "{route:?}: canceled intent must not pause");
                frame(&mut a, vec![], 0.1);
                let run = game::run(a.session.predicted_frame());
                assert_eq!((run.phase, run.generation), (phase, 0));
                assert!(!a.ui.board_pending);
                frame(&mut a, pulse(egui::Key::R), 0.1);
                assert_eq!(game::run(a.session.predicted_frame()).generation, 1);
            }
        }
    }

    #[test]
    fn continue_then_reblur_before_restart_consumption_pauses_again() {
        for phase in [game::LOST, game::WON] {
            for route in ROUTES {
                let mut a = terminal(phase);
                frame(&mut a, pulse(egui::Key::R), TICK);
                let before = a.session.predicted_frame().to_bytes();
                lose_and_return(&mut a, route);
                frame(&mut a, pulse(egui::Key::Enter), 3600.0);
                assert!(!a.ui.paused);
                assert!(a.session.predicted_frame().to_bytes() == before);
                lose_and_return(&mut a, route);
                frame(&mut a, vec![], 3600.0);
                assert!(a.ui.paused, "{route:?}: unconsumed Restart must re-pause");
                assert!(a.session.predicted_frame().to_bytes() == before);
                frame(&mut a, pulse(egui::Key::Enter), 3600.0);
                assert!(a.session.predicted_frame().to_bytes() == before);
                frame(&mut a, vec![], 2.0 * TICK);
                let run = game::run(a.session.predicted_frame());
                assert_eq!((run.generation, run.tick, run.last_sequence), (1, 0, 1));
                assert!(!a.ui.board_pending);
            }
        }
    }

    #[test]
    fn multiple_admitted_terminal_restarts_resume_in_order_once_each() {
        for phase in [game::LOST, game::WON] {
            for route in ROUTES {
                let mut a = terminal(phase);
                frame(&mut a, pulse(egui::Key::R), TICK);
                frame(&mut a, pulse(egui::Key::R), TICK);
                assert!(a.commands.is_empty());
                assert!(a.ui.board_pending);
                assert_eq!(game::run(a.session.predicted_frame()).phase, phase);
                let before = a.session.predicted_frame().to_bytes();
                lose_and_return(&mut a, route);
                frame(&mut a, vec![], 3600.0);
                assert!(a.ui.paused);
                assert!(a.session.predicted_frame().to_bytes() == before);
                frame(&mut a, pulse(egui::Key::Enter), 3600.0);
                assert!(a.session.predicted_frame().to_bytes() == before);
                for generation in [1, 2] {
                    frame(&mut a, vec![], TICK);
                    let run = game::run(a.session.predicted_frame());
                    assert_eq!(
                        (run.generation, run.last_sequence, run.tick),
                        (generation, generation, 0)
                    );
                    assert_eq!(a.ui.board_pending, generation == 1);
                }
                frame(&mut a, vec![], TICK);
                let run = game::run(a.session.predicted_frame());
                assert_eq!((run.generation, run.last_sequence, run.tick), (2, 2, 1));
            }
        }
    }

    #[test]
    fn non_executable_restart_tail_does_not_pause_terminal() {
        for command in [
            game::DefenseInput {
                action: game::RESTART,
                ..Default::default()
            },
            game::DefenseInput {
                action: game::RESTART,
                sequence: 1,
                source_cell: game::CELLS_PER_BOARD,
                ..Default::default()
            },
            game::DefenseInput {
                action: game::RESTART,
                sequence: 1,
                target_cell: game::CELLS_PER_BOARD,
                ..Default::default()
            },
        ] {
            let mut a = terminal(game::LOST);
            // Exercise invalid public Session inputs without mutating a core frame.
            a.session.advance(command, vec![]);
            lose_and_return(&mut a, FocusRoute::Window);
            assert!(!a.ui.paused);
            frame(&mut a, vec![], 0.1);
            let run = game::run(a.session.predicted_frame());
            assert_eq!(
                (run.phase, run.generation, run.last_sequence),
                (game::LOST, 0, 0)
            );
        }
    }
}
