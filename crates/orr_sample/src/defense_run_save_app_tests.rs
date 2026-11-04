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
