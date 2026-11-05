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
