//! CPU-only real egui -> App -> Session acceptance. No window or GPU is created.
use super::*;
use crate::defense_run_save::{Journal, Origin, MAX_ADVANCES};
use crate::defense_ui::RunSaveMode;

fn app(path: &std::path::Path, scene: Option<String>) -> App {
    let origin = Origin { scene };
    let session = session(origin.config().unwrap());
    let journal = Journal::new(origin, &session);
    App {
        #[cfg(feature = "coop-defense-audio")]
        audio: Default::default(),
        session,
        ui: DefenseUi::new(),
        gfx: None,
        last: Instant::now(),
        accumulator: 0.0,
        commands: VecDeque::new(),
        error: None,
        focused: true,
        focus_lost_pending: false,
        board: PendingBoard::default(),
        saves: Some(RunSaves {
            store: crate::defense_run_save_store::RunStore::open(path, &[]).unwrap(),
            // Process identity/parity has separate tests; avoid repeatedly hashing here.
            identity: "a".repeat(64),
            journal,
            candidate: None,
        }),
    }
}
fn frame(app: &mut App, events: Vec<egui::Event>, delta: f64) {
    app.process_input(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 940.0),
            )),
            events,
            ..Default::default()
        },
        delta,
    );
}
fn key(key: egui::Key) -> Vec<egui::Event> {
    [true, false]
        .into_iter()
        .map(|pressed| egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers: Default::default(),
        })
        .collect()
}
fn clicks(x: f32, y: f32) -> Vec<egui::Event> {
    [true, false]
        .into_iter()
        .map(|pressed| egui::Event::PointerButton {
            pos: egui::pos2(x, y),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        })
        .collect()
}
fn click(app: &mut App, x: f32, y: f32) {
    frame(app, clicks(x, y), 0.0);
}
fn encoded(app: &mut App) -> Vec<u8> {
    let saves = app.saves.as_mut().unwrap();
    saves
        .journal
        .encode(&app.session, app.ui.high_water(), &saves.identity)
        .unwrap()
}
fn snapshot(app: &mut App) -> (Vec<u8>, Vec<u8>, u32) {
    (
        app.session.predicted_frame().to_bytes(),
        encoded(app),
        app.ui.high_water(),
    )
}
fn begin_replace(app: &mut App) {
    click(app, 750.0, 582.0);
    assert!(matches!(app.ui.run_save.mode, RunSaveMode::List(_)));
    click(app, 600.0, 321.0);
    assert!(matches!(app.ui.run_save.mode, RunSaveMode::Confirm(_)));
    click(app, 450.0, 642.0);
}
fn finish_load(app: &mut App) {
    for _ in 0..302 {
        if app.saves.as_ref().unwrap().candidate.is_none() {
            break;
        }
        frame(app, vec![], 0.1);
    }
    assert!(app.saves.as_ref().unwrap().candidate.is_none());
    assert!(app.ui.paused, "{}", app.ui.run_save.status);
}
// Arrange an exact admission boundary without rendering 36,000 egui frames.
// The transition under test below still uses the production App input consumer.
fn admit_defaults(app: &mut App, n: u32) {
    for _ in 0..n {
        app.session.advance(game::DefenseInput::default(), vec![]);
        app.saves
            .as_mut()
            .unwrap()
            .journal
            .record(game::DefenseInput::default());
    }
}

#[test]
fn detached_loading_cancel_busy_mixed_input_and_focus_preserve_live_state() {
    let dir = private_dir();
    let mut a = app(dir.path(), None);
    frame(&mut a, key(egui::Key::Enter), 0.0);
    for _ in 0..500 {
        frame(&mut a, vec![], 1.0 / 60.0);
    }
    frame(&mut a, key(egui::Key::P), 0.0);
    click(&mut a, 450.0, 582.0);
    assert_eq!(a.saves.as_ref().unwrap().store.list().unwrap().len(), 1);
    frame(&mut a, key(egui::Key::Enter), 0.0);
    frame(&mut a, key(egui::Key::Space), 1.0 / 60.0);
    frame(&mut a, key(egui::Key::U), 0.0); // discarded high-water must survive cancellation
    frame(&mut a, key(egui::Key::P), 0.0);
    let before = snapshot(&mut a);
    let board_frontier = a.board.through;
    begin_replace(&mut a);
    assert!(matches!(a.ui.run_save.mode, RunSaveMode::Loading));
    assert!(a.saves.as_ref().unwrap().candidate.is_some());
    assert_eq!(snapshot(&mut a), before);
    let mut mixed = clicks(450.0, 582.0);
    mixed.extend(clicks(750.0, 582.0));
    mixed.extend(clicks(600.0, 517.0));
    for k in [egui::Key::Enter, egui::Key::Space, egui::Key::R] {
        mixed.extend(key(k));
    }
    frame(&mut a, mixed, 10.0);
    assert_eq!(snapshot(&mut a), before);
    a.set_focused(false);
    a.set_focused(true);
    frame(&mut a, key(egui::Key::Enter), 10.0);
    assert_eq!(snapshot(&mut a), before);
    assert!(a.saves.as_ref().unwrap().candidate.is_some());
    click(&mut a, 800.0, 642.0);
    assert!(a.saves.as_ref().unwrap().candidate.is_none());
    assert!(matches!(a.ui.run_save.mode, RunSaveMode::Closed));
    assert!(a.ui.paused);
    assert!(a.commands.is_empty());
    assert_eq!(a.accumulator, 0.0);
    assert_eq!(snapshot(&mut a), before);
    assert_eq!(a.board.through, board_frontier);
}

#[test]
fn replay_checksum_error_is_atomic_after_multiple_detached_batches() {
    let dir = private_dir();
    let mut a = app(dir.path(), None);
    frame(&mut a, key(egui::Key::Enter), 0.0);
    for _ in 0..300 {
        frame(&mut a, vec![], 1.0 / 60.0);
    }
    frame(&mut a, key(egui::Key::P), 0.0);
    let before = snapshot(&mut a);
    let board_frontier = a.board.through;
    let mut damaged: serde_json::Value = serde_json::from_slice(&before.1).unwrap();
    let checksum = damaged["final_checksum"].as_u64().unwrap();
    damaged["final_checksum"] = serde_json::json!(checksum ^ 1);
    a.saves
        .as_ref()
        .unwrap()
        .store
        .save(&crate::defense_run_save::resign_test_payload(
            &serde_json::to_vec(&damaged).unwrap(),
        ))
        .unwrap();
    begin_replace(&mut a);
    assert!(a.saves.as_ref().unwrap().candidate.is_some());
    finish_load(&mut a);
    assert!(
        a.ui.run_save.status.contains("checksum"),
        "{}",
        a.ui.run_save.status
    );
    assert_eq!(snapshot(&mut a), before);
    assert_eq!(a.board.through, board_frontier);
    assert!(a.commands.is_empty());
    assert_eq!(a.accumulator, 0.0);
}

#[test]
fn journal_cap_disables_save_but_app_keeps_advancing_and_old_copy_survives() {
    let dir = private_dir();
    let mut a = app(dir.path(), None);
    frame(&mut a, key(egui::Key::Enter), 0.0);
    frame(&mut a, key(egui::Key::P), 0.0);
    click(&mut a, 450.0, 582.0);
    let old = a
        .saves
        .as_ref()
        .unwrap()
        .store
        .list()
        .unwrap()
        .pop()
        .unwrap();
    let old_bytes = a.saves.as_ref().unwrap().store.read(&old).unwrap();
    admit_defaults(&mut a, MAX_ADVANCES);
    frame(&mut a, key(egui::Key::Enter), 0.0);
    frame(&mut a, vec![], 1.0 / 60.0);
    assert_eq!(
        a.session.predicted_frame().tick(),
        u64::from(MAX_ADVANCES) + 1
    );
    assert!(!a.saves.as_ref().unwrap().journal.can_save());
    frame(&mut a, vec![], 1.0 / 60.0);
    assert_eq!(
        a.session.predicted_frame().tick(),
        u64::from(MAX_ADVANCES) + 2
    );
    assert!(a.ui.run_save.status.contains("limit reached"));
    frame(&mut a, key(egui::Key::P), 0.0);
    assert!(a.ui.paused);
    click(&mut a, 450.0, 582.0);
    assert!(!a.ui.run_save.can_save);
    assert_eq!(
        a.saves.as_ref().unwrap().store.list().unwrap(),
        vec![old.clone()]
    );
    assert_eq!(
        a.saves.as_ref().unwrap().store.read(&old).unwrap(),
        old_bytes
    );
    // A later load error must remain visible after the cap notice has been shown.
    a.saves
        .as_ref()
        .unwrap()
        .store
        .save(b"invalid JSON")
        .unwrap();
    begin_replace(&mut a);
    let error = a.ui.run_save.status.clone();
    assert!(error.contains("invalid save"), "{error}");
    frame(&mut a, vec![], 0.1);
    assert_eq!(a.ui.run_save.status, error);
}

#[test]
fn authored_natural_terminal_and_restart_restore_exact_original_source() {
    let source="schema: orr.scene/1\nentities: {}\nsingletons:\n  CoopDefense::Scenario: {starting_gold: 0, reactor_hp: 1}\n  CoopDefense::EnemyHealth: {multiplier: 4}\n  CoopDefense::WaveCounts: {pair_counts: [28,28,28,28,28,28,28,28,28,28]}\n";
    let dir = private_dir();
    let mut a = app(dir.path(), Some(source.into()));
    frame(&mut a, key(egui::Key::Enter), 0.0);
    for _ in 0..5000 {
        if game::run(a.session.predicted_frame()).phase != game::PLAYING {
            break;
        }
        frame(&mut a, vec![], 0.1);
    }
    assert_eq!(game::run(a.session.predicted_frame()).phase, game::LOST);
    frame(&mut a, key(egui::Key::P), 0.0);
    assert!(a.ui.paused);
    click(&mut a, 450.0, 582.0);
    let terminal = snapshot(&mut a);
    frame(&mut a, key(egui::Key::Enter), 0.0);
    frame(&mut a, key(egui::Key::R), 0.1);
    assert_eq!(game::run(a.session.predicted_frame()).generation, 1);
    assert_eq!(game::run(a.session.predicted_frame()).reactor_hp, 1);
    frame(&mut a, key(egui::Key::P), 0.0);
    let restarted = snapshot(&mut a);
    begin_replace(&mut a);
    finish_load(&mut a);
    assert_eq!(snapshot(&mut a), terminal);
    // Save the restarted journal independently, then explicitly select it in a fresh title.
    let restart_dir = private_dir();
    let mut restored = app(restart_dir.path(), None);
    restored
        .saves
        .as_ref()
        .unwrap()
        .store
        .save(&restarted.1)
        .unwrap();
    click(&mut restored, 600.0, 682.0);
    click(&mut restored, 600.0, 321.0);
    finish_load(&mut restored);
    assert_eq!(snapshot(&mut restored), restarted);
    let envelope: serde_json::Value = serde_json::from_slice(&encoded(&mut restored)).unwrap();
    assert_eq!(envelope["origin"]["scene"].as_str(), Some(source));
}

#[test]
fn admitted_scene_bytes_survive_source_replacement() {
    let source = "schema: orr.scene/1\nentities: {}\nsingletons: {CoopDefense::Scenario: {starting_gold: 250, reactor_hp: 75}}\n";
    let source_dir = private_dir();
    let path = source_dir.path().join("scene.orr");
    std::fs::write(&path, source).unwrap();
    let options = Options {
        scene: Some(path.clone()),
        ..Default::default()
    };
    let admitted = prepare_scene(&options).unwrap();
    std::fs::write(&path, "replaced source is invalid").unwrap();
    let store = private_dir();
    let mut original = app(store.path(), admitted);
    frame(&mut original, key(egui::Key::Enter), 0.0);
    frame(&mut original, key(egui::Key::P), 0.0);
    click(&mut original, 450.0, 582.0);
    let before = snapshot(&mut original);
    let mut restored = app(store.path(), None);
    click(&mut restored, 600.0, 682.0);
    click(&mut restored, 600.0, 321.0);
    finish_load(&mut restored);
    assert_eq!(snapshot(&mut restored), before);
    assert_eq!(
        restored
            .session
            .predicted_frame()
            .singleton::<game::Scenario>()
            .reactor_hp,
        75
    );
}

fn private_dir() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

#[test]
fn hostile_parse_errors_are_bounded_sanitized_and_survive_redraw() {
    for scene_error in [false, true] {
        let dir = private_dir();
        let mut a = app(dir.path(), None);
        let before = snapshot(&mut a);
        let bytes = if scene_error {
            let mut e: serde_json::Value = serde_json::from_slice(&before.1).unwrap();
            e["origin"]["scene"] = serde_json::json!(format!(
                "schema: orr.scene/1\nentities: {{}}\nsingletons: {{{}: {{}}}}\n",
                "界".repeat(10_000)
            ));
            crate::defense_run_save::resign_test_payload(&serde_json::to_vec(&e).unwrap())
        } else {
            format!("{{\"{}\":0}}", "x".repeat(1024 * 1024)).into_bytes()
        };
        a.saves.as_ref().unwrap().store.save(&bytes).unwrap();
        click(&mut a, 600.0, 682.0);
        click(&mut a, 600.0, 321.0);
        assert!(a.saves.as_ref().unwrap().candidate.is_none());
        let error = a.ui.run_save.status.clone();
        assert!(!error.is_empty());
        assert!(error.chars().count() <= 241);
        assert!(!error.chars().any(char::is_control));
        frame(&mut a, vec![], 0.1);
        assert_eq!(a.ui.run_save.status, error);
        assert_eq!(snapshot(&mut a), before);
    }
}

#[test]
fn restoring_earlier_pending_tail_replaces_live_board_frontier() {
    let dir = private_dir();
    let mut a = app(dir.path(), None);
    frame(&mut a, key(egui::Key::Enter), 0.0);
    frame(&mut a, key(egui::Key::Space), 1.0 / 60.0);
    assert!(a.ui.board_pending);
    frame(&mut a, key(egui::Key::P), 0.0);
    click(&mut a, 450.0, 582.0);
    let saved_tick = a.session.predicted_frame().tick();
    let saved_tail = a.session.next_send_tick().saturating_sub(1);
    frame(&mut a, key(egui::Key::Enter), 0.0);
    for _ in 0..20 {
        frame(&mut a, vec![], 0.1);
    }
    frame(&mut a, key(egui::Key::Space), 1.0 / 60.0);
    frame(&mut a, key(egui::Key::P), 0.0);
    assert!(a.board.through.unwrap() > saved_tail);
    begin_replace(&mut a);
    finish_load(&mut a);
    assert_eq!(a.session.predicted_frame().tick(), saved_tick);
    assert_eq!(a.board.through, Some(saved_tail));
    assert!(a.ui.paused);
    assert_eq!(a.ui.selected, None);
    frame(&mut a, key(egui::Key::Enter), 0.0);
    assert!(a.ui.board_pending);
    for _ in 0..2 {
        frame(&mut a, vec![], 1.0 / 60.0);
    }
    assert!(!a.ui.board_pending);
    assert!(game::units(a.session.predicted_frame())
        .iter()
        .any(|u| u.board == 0 && u.active != 0));
    assert_eq!(a.ui.selected, None);
}

mod terminal_focus {
    use super::*;

    const TICK: f64 = 1.0 / 60.0;

    fn lost(path: &std::path::Path) -> App {
        let source = "schema: orr.scene/1\nentities: {}\nsingletons:\n  CoopDefense::Scenario: {starting_gold: 0, reactor_hp: 1}\n  CoopDefense::EnemyHealth: {multiplier: 4}\n  CoopDefense::WaveCounts: {pair_counts: [28,28,28,28,28,28,28,28,28,28]}\n";
        let mut a = app(path, Some(source.into()));
        frame(&mut a, key(egui::Key::Enter), 0.0);
        for _ in 0..5000 {
            if game::run(a.session.predicted_frame()).phase != game::PLAYING {
                break;
            }
            frame(&mut a, vec![], 0.1);
        }
        assert_eq!(game::run(a.session.predicted_frame()).phase, game::LOST);
        assert!(a.session.predicted_frame().tick() > 120);
        a
    }

    fn tail(a: &App) -> Vec<(u64, PlayerSlot, game::DefenseInput)> {
        a.session
            .authored_since(a.session.predicted_frame().tick())
            .into_iter()
            .map(|entry| {
                assert!(entry.commands.is_empty());
                assert!(!entry.disconnected);
                (entry.tick, entry.slot, entry.input)
            })
            .collect()
    }

    fn admit_restart_and_pause(a: &mut App) {
        frame(a, key(egui::Key::R), TICK);
        assert!(a.commands.is_empty());
        assert_eq!(game::run(a.session.predicted_frame()).phase, game::LOST);
        let admitted = tail(a);
        assert_eq!(
            admitted
                .iter()
                .filter(|(_, _, input)| input.action == game::RESTART)
                .count(),
            1
        );
        frame(a, key(egui::Key::P), 0.0);
        assert!(a.ui.paused);
        assert_eq!(tail(a), admitted);
    }

    // Continuing a restore is not enough: losing focus before the admitted
    // Restart arrives must require another Continue without changing its tail.
    fn continue_blur_and_consume_once(a: &mut App) {
        assert!(a.ui.paused);
        let before = snapshot(a);
        let admitted = tail(a);
        let tick = a.session.predicted_frame().tick();
        let generation = game::run(a.session.predicted_frame()).generation;
        let last_sequence = game::run(a.session.predicted_frame()).last_sequence;
        let restart = admitted
            .iter()
            .find(|(_, _, input)| input.action == game::RESTART)
            .unwrap();
        assert_eq!(restart.0, tick + 2);
        assert_eq!(restart.1, PlayerSlot(0));
        assert!(restart.2.sequence > last_sequence);

        frame(a, key(egui::Key::Enter), 0.0);
        assert!(!a.ui.paused);
        assert_eq!(snapshot(a), before);
        assert_eq!(tail(a), admitted);
        a.set_focused(false);
        a.set_focused(true);
        for delta in [0.0, 0.1, 3600.0] {
            frame(a, vec![], delta);
            assert!(a.ui.paused);
            assert_eq!(snapshot(a), before);
            assert_eq!(tail(a), admitted);
            assert_eq!(a.board.through, Some(restart.0));
            assert!(a.ui.board_pending);
            assert!(a.commands.is_empty());
            assert_eq!(a.accumulator, 0.0);
        }

        frame(a, key(egui::Key::Enter), 0.0);
        assert!(!a.ui.paused);
        assert_eq!(snapshot(a), before);
        assert_eq!(tail(a), admitted);
        frame(a, vec![], TICK);
        assert_eq!(a.session.predicted_frame().tick(), tick + 1);
        let run = game::run(a.session.predicted_frame());
        assert_eq!((run.phase, run.generation), (game::LOST, generation));
        assert_eq!(run.last_sequence, last_sequence);
        assert!(tail(a).contains(restart));
        frame(a, vec![], TICK);
        assert_eq!(a.session.predicted_frame().tick(), restart.0);
        let run = game::run(a.session.predicted_frame());
        assert_eq!((run.phase, run.generation), (game::PLAYING, generation + 1));
        assert_eq!(run.last_sequence, restart.2.sequence);
        assert!(!a.ui.board_pending);
        assert!(tail(a)
            .iter()
            .all(|(_, _, input)| input.action != game::RESTART));
        frame(a, vec![], 0.1);
        assert_eq!(
            game::run(a.session.predicted_frame()).generation,
            generation + 1
        );
        assert_eq!(a.ui.high_water(), restart.2.sequence);
        assert!(a.error.is_none());
    }

    #[test]
    fn restored_terminal_restart_waits_for_second_continue_after_focus_return() {
        let dir = private_dir();
        let mut original = lost(dir.path());
        admit_restart_and_pause(&mut original);
        click(&mut original, 450.0, 582.0);
        let saved = snapshot(&mut original);
        let admitted = tail(&original);
        let mut restored = app(dir.path(), None);
        click(&mut restored, 600.0, 682.0);
        click(&mut restored, 600.0, 321.0);
        finish_load(&mut restored);
        assert_eq!(snapshot(&mut restored), saved);
        assert_eq!(tail(&restored), admitted);
        continue_blur_and_consume_once(&mut restored);
    }

    #[test]
    fn restoring_idle_terminal_clears_live_pending_restart_focus_guard() {
        let dir = private_dir();
        let mut a = lost(dir.path());
        frame(&mut a, key(egui::Key::P), 0.0);
        click(&mut a, 450.0, 582.0);
        let idle = snapshot(&mut a);
        let idle_tail = tail(&a);
        assert!(idle_tail
            .iter()
            .all(|(_, _, input)| *input == game::DefenseInput::default()));
        frame(&mut a, key(egui::Key::Enter), 0.0);
        admit_restart_and_pause(&mut a);
        begin_replace(&mut a);
        finish_load(&mut a);
        assert_eq!(snapshot(&mut a), idle);
        assert_eq!(tail(&a), idle_tail);
        frame(&mut a, key(egui::Key::Enter), 0.0);
        assert!(!a.ui.paused);
        assert!(
            a.ui.board_pending,
            "restored neutral tail still has a frontier"
        );
        a.set_focused(false);
        a.set_focused(true);
        frame(&mut a, vec![], 0.0);
        assert!(
            !a.ui.paused,
            "neutral frontier must not retain Restart evidence"
        );
        assert_eq!(snapshot(&mut a), idle);
        assert_eq!(tail(&a), idle_tail);
        frame(&mut a, vec![], 0.1);
        assert!(!a.ui.paused);
        assert!(!a.ui.board_pending);
        let run = game::run(a.session.predicted_frame());
        assert_eq!((run.phase, run.generation), (game::LOST, 0));
        assert_eq!(run.last_sequence, 0);
    }

    fn replacement_preserves_pending_restart(cancel: bool) {
        let dir = private_dir();
        let mut a = lost(dir.path());
        frame(&mut a, key(egui::Key::P), 0.0);
        click(&mut a, 450.0, 582.0);
        if !cancel {
            let mut damaged: serde_json::Value = serde_json::from_slice(&encoded(&mut a)).unwrap();
            let checksum = damaged["final_checksum"].as_u64().unwrap();
            damaged["final_checksum"] = serde_json::json!(checksum ^ 1);
            a.saves
                .as_ref()
                .unwrap()
                .store
                .save(&crate::defense_run_save::resign_test_payload(
                    &serde_json::to_vec(&damaged).unwrap(),
                ))
                .unwrap();
        }
        frame(&mut a, key(egui::Key::Enter), 0.0);
        admit_restart_and_pause(&mut a);
        let before = snapshot(&mut a);
        let admitted = tail(&a);
        begin_replace(&mut a);
        assert!(a.saves.as_ref().unwrap().candidate.is_some());
        assert!(matches!(a.ui.run_save.mode, RunSaveMode::Loading));
        assert_eq!(snapshot(&mut a), before);
        assert_eq!(tail(&a), admitted);
        if cancel {
            click(&mut a, 800.0, 642.0);
            assert!(a.saves.as_ref().unwrap().candidate.is_none());
        } else {
            finish_load(&mut a);
            assert!(
                a.ui.run_save.status.contains("checksum"),
                "{}",
                a.ui.run_save.status
            );
        }
        assert!(matches!(a.ui.run_save.mode, RunSaveMode::Closed));
        assert_eq!(snapshot(&mut a), before);
        assert_eq!(tail(&a), admitted);
        continue_blur_and_consume_once(&mut a);
    }

    #[test]
    fn cancelled_terminal_replacement_preserves_pending_restart_focus_guard() {
        replacement_preserves_pending_restart(true);
    }

    #[test]
    fn failed_terminal_replacement_preserves_pending_restart_focus_guard() {
        replacement_preserves_pending_restart(false);
    }
}
