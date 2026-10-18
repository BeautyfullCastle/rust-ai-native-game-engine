//! Calls the production keyboard policy and a real local Bridge. These tests do
//! not create an OS window, inject physical keys or exercise a native display.
use super::*;
use orr_fp::FPVec2;
use orr_input::{ActionMap, Button, Key};
use std::time::Duration;

fn remap() -> ActionMap {
    let mut map = input_actions::default_map();
    for action in &mut map.actions {
        let key = match action.name.as_str() {
            "left" => Key::J,
            "right" => Key::L,
            "up" => Key::I,
            "down" => Key::K,
            "restart" => Key::Enter,
            "quit" => Key::Q,
            _ => unreachable!(),
        };
        action.bindings = vec![Button::Keyboard { key }];
    }
    map
}
fn remapped_keys() -> Keys {
    Keys::with_bindings(CollectBindings::from_map(&remap()).unwrap())
}
fn test_bridge() -> LocalBridge {
    let level = game::CollectLevel::new(
        FPVec2::ZERO,
        vec![FPVec2::new(FP::from_int(20), FP::ZERO)],
        vec![],
        600,
    )
    .unwrap();
    let simulation = orr_sim::Simulation::<CollectDodgeV1>::new(level, 60, 42);
    let mut config = orr_bridge::PlayConfig::new(1, 42, 60);
    config.start_paused = false;
    let session =
        orr_bridge::PlaySession::<CollectDodgeV1>::from_frame(config, simulation.frame()).unwrap();
    InProc::new(
        PlayHost::new(session, PlayerSlot(0)),
        BridgeConfig::default(),
    )
}
#[derive(Debug, PartialEq, Eq)]
struct Sample {
    input: CollectInput,
    tick: u64,
    checksum: u64,
    run: game::CollectRun,
    pending_restarts: u8,
}
fn deliver(keys: &mut Keys, bridge: &mut LocalBridge, dt: Duration) -> Sample {
    let input = keys.input();
    bridge.set_input(PlayerSlot(0), input).unwrap();
    bridge.update(dt);
    let snapshot = bridge.poll_view().snapshot.unwrap();
    let run = *snapshot.predicted().singleton::<game::CollectRun>();
    keys.observe(snapshot.tick(), run.restart_held != 0);
    Sample {
        input,
        tick: snapshot.tick(),
        checksum: snapshot.predicted().checksum(),
        run,
        pending_restarts: keys.pending_restarts,
    }
}
fn key(keys: &mut Keys, code: KeyCode, down: bool) -> bool {
    keys.keyboard_event(code, down, false, false, false)
}
fn tap(keys: &mut Keys, code: KeyCode) {
    assert!(!key(keys, code, true));
    assert!(!key(keys, code, false));
}
fn play_trace(mut keys: Keys, right: KeyCode, restart: KeyCode, quit: KeyCode) -> Vec<Sample> {
    let mut bridge = test_bridge();
    let dt = Duration::from_millis(17);
    let mut trace = Vec::new();
    keys.focus(true);
    assert!(!key(&mut keys, right, true));
    for _ in 0..20 {
        trace.push(deliver(&mut keys, &mut bridge, dt));
    }
    assert_eq!(trace.last().unwrap().run.phase, game::WON);
    assert_eq!(trace.last().unwrap().run.score, 1);
    assert!(!key(&mut keys, right, false));
    tap(&mut keys, restart);
    tap(&mut keys, restart);
    trace.push(deliver(&mut keys, &mut bridge, Duration::ZERO));
    assert_eq!(trace.last().unwrap().pending_restarts, 2);
    trace.push(deliver(&mut keys, &mut bridge, dt));
    assert_eq!(trace.last().unwrap().run.phase, game::PLAYING);
    assert_eq!(trace.last().unwrap().run.score, 0);
    assert_eq!(trace.last().unwrap().run.elapsed_ticks, 0);
    trace.push(deliver(&mut keys, &mut bridge, Duration::ZERO));
    assert_eq!(trace.last().unwrap().pending_restarts, 1);
    trace.push(deliver(&mut keys, &mut bridge, dt));
    assert_eq!(trace.last().unwrap().run.restart_held, 0);
    assert_eq!(trace.last().unwrap().pending_restarts, 1);
    trace.push(deliver(&mut keys, &mut bridge, dt));
    assert_eq!(trace.last().unwrap().run.elapsed_ticks, 0);
    assert_eq!(trace.last().unwrap().pending_restarts, 0);
    assert!(key(&mut keys, quit, true));
    trace
}

#[test]
fn remapped_keys_match_default_collect_win_restart_and_quit_bridge_trace() {
    let default = play_trace(
        Keys::default(),
        KeyCode::KeyD,
        KeyCode::Space,
        KeyCode::Escape,
    );
    let remapped = play_trace(
        remapped_keys(),
        KeyCode::KeyL,
        KeyCode::Enter,
        KeyCode::KeyQ,
    );
    assert_eq!(default, remapped);
}

#[test]
fn remapped_old_keys_are_inert_in_production_policy_and_bridge() {
    let mut keys = remapped_keys();
    let mut bridge = test_bridge();
    keys.focus(true);
    for code in [
        KeyCode::KeyA,
        KeyCode::KeyD,
        KeyCode::KeyW,
        KeyCode::KeyS,
        KeyCode::ArrowLeft,
        KeyCode::ArrowRight,
        KeyCode::ArrowUp,
        KeyCode::ArrowDown,
        KeyCode::Space,
        KeyCode::Escape,
    ] {
        assert!(!key(&mut keys, code, true));
        assert_eq!(keys.input(), CollectInput::default());
        let sample = deliver(&mut keys, &mut bridge, Duration::from_millis(17));
        assert_eq!(sample.run.score, 0);
        assert_eq!(sample.pending_restarts, 0);
        assert!(!key(&mut keys, code, false));
    }
    let snapshot = bridge.snapshot().unwrap();
    let player = snapshot
        .predicted()
        .iter::<game::CollectActor>()
        .find(|(_, actor)| actor.kind == game::PLAYER)
        .unwrap()
        .1;
    assert_eq!(player.position, FPVec2::ZERO);
    assert!(key(&mut keys, KeyCode::KeyQ, true));
}

#[test]
fn remapped_queue_cap_repeat_subtick_and_neutral_ack_use_real_bridge() {
    let mut keys = remapped_keys();
    let mut bridge = test_bridge();
    keys.focus(true);
    assert!(!keys.keyboard_event(KeyCode::Enter, true, true, false, false));
    assert_eq!(keys.pending_restarts, 0);
    for _ in 0..100 {
        tap(&mut keys, KeyCode::Enter);
    }
    assert_eq!(keys.pending_restarts, MAX_PENDING_RESTARTS);
    for _ in 0..3 {
        let sample = deliver(&mut keys, &mut bridge, Duration::ZERO);
        assert_eq!(sample.tick, 0);
        assert_eq!(sample.pending_restarts, MAX_PENDING_RESTARTS);
    }
    let mut restarts = 0;
    for n in 0..15 {
        let sample = deliver(&mut keys, &mut bridge, Duration::from_millis(17));
        if n % 2 == 0 {
            assert_eq!(sample.run.restart_held, 1);
            assert_eq!(sample.run.elapsed_ticks, 0);
            restarts += 1;
        } else {
            assert_eq!(sample.run.restart_held, 0);
            assert_eq!(sample.run.elapsed_ticks, 1);
        }
    }
    assert_eq!(restarts, 8);
    assert_eq!(keys.pending_restarts, 0);
    assert_eq!(keys.input().buttons, 0);
}

#[test]
fn remapped_focus_synthetic_capture_and_quit_keep_existing_policy() {
    let mut keys = remapped_keys();
    keys.focus(true);
    assert!(!keys.keyboard_event(KeyCode::KeyL, true, false, false, true));
    assert_eq!(keys.input().x, FP::ZERO);
    assert!(!keys.keyboard_event(KeyCode::KeyL, true, true, false, false));
    assert_eq!(keys.input().x, FP::ZERO);
    key(&mut keys, KeyCode::KeyL, false);
    key(&mut keys, KeyCode::KeyL, true);
    assert_eq!(keys.input().x, FP::ONE);
    tap(&mut keys, KeyCode::Enter);
    keys.focus(false);
    assert_eq!(keys.pending_restarts, 0);
    assert_eq!(keys.input(), CollectInput::default());
    assert!(!key(&mut keys, KeyCode::KeyQ, true));
    keys.focus(true);
    assert!(!keys.keyboard_event(KeyCode::KeyL, true, false, true, false));
    assert!(!keys.keyboard_event(KeyCode::Enter, true, false, true, false));
    assert!(!keys.keyboard_event(KeyCode::KeyQ, true, false, true, false));
    assert_eq!(keys.input(), CollectInput::default());
    key(&mut keys, KeyCode::Enter, false);
    assert!(!keys.keyboard_event(KeyCode::Enter, true, false, false, true));
    assert_eq!(keys.pending_restarts, 0);
    key(&mut keys, KeyCode::Enter, false);
    tap(&mut keys, KeyCode::Enter);
    assert!(!keys.keyboard_event(KeyCode::KeyI, true, false, false, true));
    assert_eq!(keys.input().buttons, game::RESTART);
    assert_eq!(keys.input().y, FP::ZERO);
    // Quit has always bypassed gameplay capture, including repeated presses.
    assert!(keys.keyboard_event(KeyCode::KeyQ, true, true, false, true));
    assert!(!key(&mut keys, KeyCode::KeyQ, false));
}

#[test]
fn remapped_aliases_are_independent_and_opposing_axes_cancel() {
    let mut map = remap();
    map.actions
        .iter_mut()
        .find(|a| a.name == "right")
        .unwrap()
        .bindings
        .push(Button::Keyboard { key: Key::U });
    let mut keys = Keys::with_bindings(CollectBindings::from_map(&map).unwrap());
    keys.focus(true);
    key(&mut keys, KeyCode::KeyL, true);
    key(&mut keys, KeyCode::KeyU, true);
    key(&mut keys, KeyCode::KeyL, false);
    assert_eq!(keys.input().x, FP::ONE);
    key(&mut keys, KeyCode::KeyJ, true);
    key(&mut keys, KeyCode::KeyI, true);
    key(&mut keys, KeyCode::KeyK, true);
    assert_eq!(keys.input(), CollectInput::default());
    key(&mut keys, KeyCode::KeyU, false);
    key(&mut keys, KeyCode::KeyK, false);
    assert_eq!(keys.input().x, -FP::ONE);
    assert_eq!(keys.input().y, FP::ONE);
}

#[test]
fn map_reload_reproduces_production_bridge_trace_without_writing_map() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("controls.json");
    remap().save_new(&path).unwrap();
    let before = std::fs::read(&path).unwrap();
    let run = || {
        play_trace(
            Keys::with_bindings(CollectBindings::load_file(&path).unwrap()),
            KeyCode::KeyL,
            KeyCode::Enter,
            KeyCode::KeyQ,
        )
    };
    assert_eq!(run(), run());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn invalid_map_is_rejected_before_project_open_and_direct_headless_options_are_guarded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invalid.json");
    let mut map = remap();
    map.actions.retain(|a| a.name != "restart");
    map.save_new(&path).unwrap();
    let mut options = Options::parse([
        "--project".into(),
        dir.path().join("missing-project").display().to_string(),
        "--input-bindings".into(),
        path.display().to_string(),
    ])
    .unwrap();
    let before = std::fs::read(&path).unwrap();
    let error = run(options).unwrap_err();
    assert!(
        error.contains("exactly left/right/up/down/restart/quit"),
        "{error}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    options = Options::parse(["--project".into(), "unused".into()]).unwrap();
    options.headless = true;
    options.input_bindings = Some(path);
    assert!(run(options).unwrap_err().contains("window-only"));
}

#[test]
fn remapped_restart_aliases_retain_queued_edges_and_held_state_after_ack() {
    let mut map = remap();
    map.actions
        .iter_mut()
        .find(|a| a.name == "restart")
        .unwrap()
        .bindings
        .push(Button::Keyboard { key: Key::R });
    let mut keys = Keys::with_bindings(CollectBindings::from_map(&map).unwrap());
    let mut bridge = test_bridge();
    let dt = Duration::from_millis(17);
    keys.focus(true);
    key(&mut keys, KeyCode::Enter, true);
    key(&mut keys, KeyCode::KeyR, true);
    key(&mut keys, KeyCode::Enter, false);
    assert_eq!(keys.pending_restarts, 2);
    let first = deliver(&mut keys, &mut bridge, dt);
    assert_eq!(first.pending_restarts, 1);
    assert_eq!(keys.input().buttons, 0);
    let neutral = deliver(&mut keys, &mut bridge, dt);
    assert_eq!(neutral.run.restart_held, 0);
    let second = deliver(&mut keys, &mut bridge, dt);
    assert_eq!(second.pending_restarts, 0);
    assert_eq!(second.run.elapsed_ticks, 0);
    assert_eq!(keys.input().buttons, game::RESTART); // R remains physically held.
    let held = deliver(&mut keys, &mut bridge, dt);
    assert_eq!(held.run.elapsed_ticks, 1); // No duplicate restart while held.
    key(&mut keys, KeyCode::KeyR, false);
    assert_eq!(deliver(&mut keys, &mut bridge, dt).run.restart_held, 0);
    tap(&mut keys, KeyCode::Enter);
    assert_eq!(deliver(&mut keys, &mut bridge, dt).run.elapsed_ticks, 0);
    key(&mut keys, KeyCode::Enter, true);
    key(&mut keys, KeyCode::KeyR, true);
    keys.keyboard_event(KeyCode::KeyR, true, true, false, false);
    assert_eq!(keys.pending_restarts, 2);
    keys.focus(false);
    assert_eq!(keys.pending_restarts, 0);
    keys.focus(true);
    keys.keyboard_event(KeyCode::KeyR, true, true, false, false);
    assert_eq!(keys.pending_restarts, 0);
    key(&mut keys, KeyCode::Enter, false);
    key(&mut keys, KeyCode::KeyR, false);
    for _ in 0..12 {
        tap(&mut keys, KeyCode::Enter);
        tap(&mut keys, KeyCode::KeyR);
    }
    assert_eq!(keys.pending_restarts, MAX_PENDING_RESTARTS);
}
