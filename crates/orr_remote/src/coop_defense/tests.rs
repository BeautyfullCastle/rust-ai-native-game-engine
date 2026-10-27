use super::*;
use crate::{Auth, Caps, ErpClient};
use orr_edit::{Op, Origin, PlayController};
use orr_games::coop_defense_game::{DefenseInput, DefenseRun, Scenario, SCENARIO_NAME, SUMMON};
use orr_session::{ControlOp, LocalInputSource, Session, SessionConfig};
use orr_sim::PlayerSlot;
use serde_json::Value as J;
use std::collections::VecDeque;

fn native_config() -> SessionConfig {
    SessionConfig::new(
        1,
        PlayerSlot(0),
        u64::from(DefenseConfig::default().seed),
        TICK_RATE,
    )
}

fn native_session() -> Session<DefenseGame, LocalInputSource> {
    let cfg = DefenseConfig::default();
    Session::new(cfg, native_config(), LocalInputSource)
}

#[test]
fn initial_admission_matches_native_pools_rng_and_bytes() {
    let doc = document(INITIAL_SCENE).unwrap();
    let native = native_session();
    assert_eq!(doc.frame().alive_count(), 216);
    assert_eq!(doc.frame().to_bytes(), native.predicted_frame().to_bytes());
    assert_eq!(doc.checksum(), native.predicted_frame().checksum());
    assert_eq!(doc.frame().singleton::<DefenseRun>().gold, [150, 150]);
    assert!(doc.history().is_empty());
    assert!(!doc.is_dirty());
}

#[test]
fn strict_initial_source_rejects_edits_atomically() {
    let text = format!("# fixed native preset\n{INITIAL_SCENE}");
    let mut doc = document(&text).unwrap();
    let bytes = doc.frame().to_bytes();
    let revision = doc.revision();
    for invalid in [
        "schema: orr.scene/1\nentities: {e_00000001: {name: injected}}\n".to_string(),
        "schema: orr.scene/1\nentities: {}\nsingletons: {CoopDefense::Run: {}}\n".into(),
        authored_scene(10001, 30),
        authored_scene(150, 0),
        format!("{INITIAL_SCENE}metadata: {{seed: 1}}\n"),
        format!("{INITIAL_SCENE}entities: {{}}\n"),
        format!("{INITIAL_SCENE}---\n{INITIAL_SCENE}"),
        format!("{INITIAL_SCENE}#{}", "x".repeat(MAX_SOURCE_BYTES)),
    ] {
        assert!(document(&invalid).is_err());
        assert!(doc.load_yaml(&invalid).is_err());
        assert_eq!(doc.frame().to_bytes(), bytes);
        assert_eq!(doc.to_yaml(), text);
        assert_eq!(doc.revision(), revision);
        assert!(doc.history().is_empty());
        assert!(!doc.can_undo() && !doc.can_redo() && !doc.is_dirty());
    }
    assert!(doc
        .apply(
            Op::SpawnEntity {
                guid: None,
                name: Some("injected".into()),
                components: vec![]
            },
            Origin::User
        )
        .is_err());
    assert_eq!(doc.frame().to_bytes(), bytes);
    assert!(doc.history().is_empty());
}

#[test]
fn play_trace_pause_step_stop_and_restart_preserve_document() {
    let text = format!("# keep this source\n{INITIAL_SCENE}");
    let doc = document(&text).unwrap();
    let initial = doc.frame().to_bytes();
    let mut native = native_session();
    let mut play =
        PlayController::<DefenseGame>::start_play(&doc, doc.play_config(1, TICK_RATE)).unwrap();
    assert!(!play.allow_play_edits());
    assert!(play.capture_scene().is_err());
    // Pause performs no hidden ticks. A single Step advances exactly once.
    for _ in 0..3 {
        play.session_mut().tick();
    }
    assert_eq!(play.session().head_tick(), 0);
    play.control(ControlOp::Step(1));
    native.advance(DefenseInput::default(), vec![]);
    assert_eq!(play.session().head_tick(), 1);
    assert_eq!(
        play.session().frame().to_bytes(),
        native.predicted_frame().to_bytes()
    );
    play.control(ControlOp::Play);
    // Native Session keeps its real two-tick input delay. PlaySession accepts
    // the input for the next tick directly, so compare identical *consumed*
    // inputs rather than pretending equal submission time means equal ticks.
    let delay = native.config().input_delay;
    let mut pending = VecDeque::from(vec![DefenseInput::default(); delay as usize]);
    let mut sequence = 0;
    for tick in 1..600 {
        let value = if tick % 30 == 0 {
            sequence += 1;
            DefenseInput {
                action: SUMMON,
                target_cell: sequence % 12,
                sequence,
                ..Default::default()
            }
        } else {
            DefenseInput::default()
        };
        pending.push_back(value);
        let consumed = pending.pop_front().unwrap();
        play.session_mut().set_input(PlayerSlot(0), consumed);
        play.session_mut().tick();
        native.advance(value, vec![]);
        assert!(
            play.session().frame().to_bytes() == native.predicted_frame().to_bytes(),
            "consumed tick {}: Play checksum {:016x}, native {:016x}",
            tick + 1,
            play.session().frame().checksum(),
            native.predicted_frame().checksum()
        );
    }
    // Consume every submitted input, including the tail still queued by the
    // native session, without changing the native session configuration.
    for _ in 0..delay {
        let consumed = pending.pop_front().unwrap();
        play.session_mut().set_input(PlayerSlot(0), consumed);
        play.session_mut().tick();
        native.advance(DefenseInput::default(), vec![]);
        assert!(
            play.session().frame().to_bytes() == native.predicted_frame().to_bytes(),
            "flushed tick {}: Play checksum {:016x}, native {:016x}",
            play.session().head_tick(),
            play.session().frame().checksum(),
            native.predicted_frame().checksum()
        );
    }
    assert!(pending.is_empty());
    play.control(ControlOp::Pause);
    let head = play.session().head_tick();
    play.session_mut().tick();
    assert_eq!(play.session().head_tick(), head);
    let stopped = play.stop_play();
    assert_eq!(stopped.tick, 600 + u64::from(delay));
    assert!(!stopped.replay.is_empty());
    assert_eq!(doc.frame().to_bytes(), initial);
    assert_eq!(doc.to_yaml(), text);
    assert_eq!(doc.revision(), 0);
    assert!(doc.history().is_empty());
    assert!(!doc.is_dirty() && !doc.can_undo() && !doc.can_redo());
    let again =
        PlayController::<DefenseGame>::start_play(&doc, doc.play_config(1, TICK_RATE)).unwrap();
    assert_eq!(again.session().head_tick(), 0);
    assert_eq!(again.session().frame().to_bytes(), initial);
}

fn client(host: &LocalHost, caps: Caps) -> ErpClient {
    ErpClient::with_transport(Box::new(
        host.connector().connect("same-label", caps).unwrap(),
    ))
}

fn tagged(grant: &J, sequence: u64, value: J) -> J {
    json!({"player":0,"grant":grant["grant"],"generation":grant["generation"],"sequence":sequence.to_string(),"value":value})
}

#[test]
fn typed_host_rejects_bad_input_stale_grants_and_duplicate_spending() {
    let mut invalid_auth = ServerConfig::new(Auth::Tokens(vec![]));
    invalid_auth.listen = false;
    let error = spawn_host(INITIAL_SCENE.into(), None, invalid_auth, None)
        .err()
        .expect("host must retain the server's invalid-token rejection");
    assert!(error.contains("no tokens configured"), "{error}");
    // In-process transport uses the same capability checks without a socket.
    let mut cfg = ServerConfig::new(Auth::DevNoAuth);
    cfg.listen = false;
    let host = spawn_host(INITIAL_SCENE.into(), None, cfg, None).unwrap();
    assert!(host.url().is_none());
    let mut c = client(&host, Caps::ALL);
    let mut other = client(&host, Caps::ALL);
    let mut read = client(&host, Caps::parse("read").unwrap());
    assert!(read.call("sim.start", json!({})).is_err());
    let descriptor = c.call("registry.input", J::Null).unwrap();
    assert_eq!(descriptor["max_players"], 1);
    assert_eq!(descriptor["managed_held"]["version"], 1);
    assert!(c.call("sim.start", json!({"player_count":2})).is_err());
    let initial = c.call("sim.start", json!({})).unwrap();
    assert_eq!(initial["head_tick"], 0);
    let saved = c.call("scene.save", json!({})).unwrap();
    let history = c.call("history.list", json!({})).unwrap();
    let grant = c
        .call("sim.input_claim", json!({"player":0,"replace_held":true}))
        .unwrap();
    let value = json!({"action":SUMMON,"source_cell":0,"target_cell":0,"sequence":1});
    assert!(other
        .call("sim.input_value", tagged(&grant, 1, value.clone()))
        .is_err());
    for bad in [
        json!({"action":6,"source_cell":0,"target_cell":0,"sequence":1}),
        json!({"action":1,"source_cell":12,"target_cell":0,"sequence":1}),
        json!({"action":1,"source_cell":0,"target_cell":12,"sequence":1}),
        json!({"action":"bad","source_cell":0,"target_cell":0,"sequence":1}),
        json!({"action":1,"source_cell":0,"target_cell":0,"sequence":1,"extra":0}),
        J::Null,
    ] {
        assert!(c.call("sim.input_value", tagged(&grant, 1, bad)).is_err());
        assert_eq!(
            c.call("sim.state", json!({})).unwrap()["checksum"],
            initial["checksum"]
        );
    }
    c.call("sim.input_value", tagged(&grant, 1, value.clone()))
        .unwrap();
    assert!(c
        .call("sim.input_value", tagged(&grant, 1, value.clone()))
        .is_err());
    // Invalid updates preserve the previously accepted held value and do
    // not consume transport sequence 2: the later valid seq2 still succeeds.
    assert!(c
        .call(
            "sim.input_value",
            tagged(
                &grant,
                2,
                json!({"action":6,"source_cell":0,"target_cell":0,"sequence":2})
            )
        )
        .is_err());
    c.call("sim.step", json!({"n":1})).unwrap();
    // A repeated inner action ID under a new valid transport sequence must
    // not spend again, even though held samples reach subsequent game ticks.
    c.call("sim.input_value", tagged(&grant, 2, value)).unwrap();
    c.call("sim.step", json!({"n":2})).unwrap();
    let mut native = native_session();
    let held = DefenseInput {
        action: SUMMON,
        sequence: 1,
        ..Default::default()
    };
    for _ in 0..3 {
        native.advance(held, vec![]);
    }
    assert_eq!(
        native.predicted_frame().singleton::<DefenseRun>().summons,
        1
    );
    assert_eq!(
        c.call("sim.state", json!({})).unwrap()["checksum"],
        crate::wire::checksum_text(native.predicted_frame().checksum())
    );
    c.call("sim.input_release", tagged(&grant, 3, J::Null))
        .unwrap();
    assert!(c
        .call("sim.input_value", tagged(&grant, 4, json!({})))
        .is_err());
    let replacement = c
        .call("sim.input_claim", json!({"player":0,"replace_held":true}))
        .unwrap();
    assert_ne!(replacement["grant"], grant["grant"]);
    assert!(c
        .call("sim.input_renew", tagged(&grant, 5, J::Null))
        .is_err());
    c.call("sim.step", json!({"n":1})).unwrap();
    native.advance(DefenseInput::default(), vec![]);
    assert_eq!(
        c.call("sim.state", json!({})).unwrap()["checksum"],
        crate::wire::checksum_text(native.predicted_frame().checksum())
    );
    assert!(c.call("scene.save", json!({"source":"play"})).is_err());
    c.call("sim.stop", json!({})).unwrap();
    assert_eq!(c.call("scene.save", json!({})).unwrap(), saved);
    assert_eq!(c.call("history.list", json!({})).unwrap(), history);
    assert_eq!(
        c.call("sim.start", json!({})).unwrap()["checksum"],
        initial["checksum"]
    );
    assert!(c
        .call("sim.input_renew", tagged(&replacement, 1, J::Null))
        .is_err());
    assert!(c
        .call(
            "sim.input_value",
            tagged(
                &replacement,
                1,
                json!({"action":SUMMON,"source_cell":0,"target_cell":0,"sequence":2})
            )
        )
        .is_err());
    assert_eq!(
        c.call("sim.state", json!({})).unwrap()["checksum"],
        initial["checksum"]
    );
}

fn authored_scene(gold: u32, hp: u32) -> String {
    format!("schema: orr.scene/1\nsingletons:\n  CoopDefense::Scenario: {{starting_gold: {gold}, reactor_hp: {hp}}}\nentities: {{}}\n")
}

#[test]
fn authored_start_restart_and_legacy_defaults_are_checksummed() {
    use orr_games::coop_defense_game::RESTART;
    let legacy = document(INITIAL_SCENE).unwrap();
    let defaults = document(&authored_scene(150, 30)).unwrap();
    assert_eq!(legacy.frame().to_bytes(), defaults.frame().to_bytes());
    let doc = document(&authored_scene(250, 80)).unwrap();
    assert_ne!(doc.checksum(), legacy.checksum());
    let mut play =
        PlayController::<DefenseGame>::start_play(&doc, doc.play_config(1, TICK_RATE)).unwrap();
    let run = play.session().frame().singleton::<DefenseRun>();
    assert_eq!(run.gold, [250; 2]);
    assert_eq!(run.reactor_hp, 80);
    play.session_mut().set_input(
        PlayerSlot(0),
        DefenseInput {
            action: SUMMON,
            sequence: 1,
            ..Default::default()
        },
    );
    play.control(ControlOp::Step(1));
    assert_eq!(
        play.session().frame().singleton::<DefenseRun>().gold[0],
        225
    );
    play.session_mut().set_input(
        PlayerSlot(0),
        DefenseInput {
            action: RESTART,
            sequence: 2,
            ..Default::default()
        },
    );
    play.control(ControlOp::Step(1));
    let run = play.session().frame().singleton::<DefenseRun>();
    assert_eq!(run.gold, [250; 2]);
    assert_eq!(run.reactor_hp, 80);
    assert_eq!(run.generation, 1);
    assert_eq!(
        *play.session().frame().singleton::<Scenario>(),
        Scenario {
            starting_gold: 250,
            reactor_hp: 80
        }
    );
    assert_eq!(document(&doc.to_yaml()).unwrap().checksum(), doc.checksum());
}

#[test]
fn scenario_erp_edit_undo_save_reopen_and_invalid_atomicity() {
    let mut cfg = ServerConfig::new(Auth::DevNoAuth);
    cfg.listen = false;
    let host = spawn_host(INITIAL_SCENE.into(), None, cfg, None).unwrap();
    let mut c = client(&host, Caps::ALL);
    let original = c.call("scene.save", json!({})).unwrap();
    let patch = |path: &str, value: J| json!({"name":SCENARIO_NAME,"path":path,"value":value});
    c.call(
        "world.singleton.patch",
        patch("", json!({"starting_gold":250,"reactor_hp":80})),
    )
    .unwrap();
    let edited = c.call("scene.save", json!({})).unwrap();
    assert_ne!(edited["checksum"], original["checksum"]);
    let reopened = document(edited["text"].as_str().unwrap()).unwrap();
    assert_eq!(reopened.frame().singleton::<DefenseRun>().gold, [250; 2]);
    assert_eq!(reopened.frame().singleton::<DefenseRun>().reactor_hp, 80);
    c.call("history.undo", json!({})).unwrap();
    let undone = c.call("scene.save", json!({})).unwrap();
    assert_eq!(undone["text"], original["text"]);
    assert_eq!(undone["checksum"], original["checksum"]);
    c.call("history.redo", json!({})).unwrap();
    let history = c.call("history.list", json!({})).unwrap();
    for bad in [
        patch("starting_gold", json!(-1)),
        patch("starting_gold", json!(10001)),
        patch("reactor_hp", json!(0)),
        patch("reactor_hp", json!(1001)),
        patch("starting_gold", json!(1.5)),
        patch("reactor_hp", json!("invalid")),
        patch("", json!({"starting_gold":300,"reactor_hp":0})),
        patch("", json!({"starting_gold":300,"reactor_hp":80,"seed":1})),
        json!({"name":"CoopDefense::Run","path":"gold","value":[999,999]}),
    ] {
        assert!(c.call("world.singleton.patch", bad).is_err());
        assert_eq!(c.call("scene.save", json!({})).unwrap(), edited);
        assert_eq!(c.call("history.list", json!({})).unwrap(), history);
    }
    let initial = c.call("sim.start", json!({})).unwrap();
    assert_eq!(initial["checksum"], edited["checksum"]);
    assert!(c
        .call("world.singleton.patch", patch("starting_gold", json!(300)))
        .is_err());
    assert_eq!(
        c.call("sim.state", json!({})).unwrap()["checksum"],
        initial["checksum"]
    );
    c.call("sim.stop", json!({})).unwrap();
    assert_eq!(c.call("scene.save", json!({})).unwrap(), edited);
}

#[test]
fn first_singleton_field_edit_uses_safe_defaults_and_bounds() {
    let mut doc = document(INITIAL_SCENE).unwrap();
    doc.apply(
        Op::SetSingletonField {
            singleton: SCENARIO_NAME.into(),
            path: "starting_gold".into(),
            value: orr_reflect::Value::Int(0),
        },
        Origin::User,
    )
    .unwrap();
    assert_eq!(
        *doc.frame().singleton::<Scenario>(),
        Scenario {
            starting_gold: 0,
            reactor_hp: 30
        }
    );
    doc.undo().unwrap();
    assert_eq!(doc.to_yaml(), INITIAL_SCENE);
    for (gold, hp) in [(0, 1), (10000, 1000)] {
        let doc = document(&authored_scene(gold, hp)).unwrap();
        assert_eq!(doc.frame().singleton::<DefenseRun>().gold, [gold; 2]);
        assert_eq!(doc.frame().singleton::<DefenseRun>().reactor_hp, hp);
    }
    let mut limits = HostLimits::default();
    configure(&mut limits, None);
    assert_eq!(limits.build_id, SCENARIO_BUILD_ID);
    assert_ne!(limits.build_id, crate::default_build_id(GAME));
}

#[test]
fn nonzero_replay_contract_rejects_previous_host_identity() {
    use orr_session::{replay_verify_checked, ReplayError, ReplayHeader, ReplayWriter};
    let old = crate::default_build_id(GAME);
    let bytes = ReplayWriter::<DefenseGame>::new(ReplayHeader {
        format_version: 3,
        game_id: GAME.into(),
        build_hash: orr_sim::build_hash_of(old, 0),
        seed: u64::from(DefenseConfig::default().seed),
        player_count: 1,
        tick_rate: TICK_RATE,
        input_size: std::mem::size_of::<DefenseInput>() as u32,
    })
    .finish();
    assert!(matches!(
        replay_verify_checked::<DefenseGame>(&bytes, DefenseConfig::default(), SCENARIO_BUILD_ID),
        Err(ReplayError::BuildHashMismatch { .. })
    ));
}

#[test]
fn saved_authored_scene_matches_native_setup_and_consumed_trace() {
    use orr_games::coop_defense_scene::parse_scenario;
    let authored = document(&authored_scene(250, 75)).unwrap().to_yaml();
    let doc = document(&authored).unwrap();
    let mut native = Session::<DefenseGame, LocalInputSource>::new(
        DefenseConfig {
            scenario: parse_scenario(&authored).unwrap(),
            ..Default::default()
        },
        native_config(),
        LocalInputSource,
    );
    let mut play =
        PlayController::<DefenseGame>::start_play(&doc, doc.play_config(1, TICK_RATE)).unwrap();
    assert_eq!(native.predicted_frame().to_bytes(), doc.frame().to_bytes());
    assert_eq!(
        native.predicted_frame().singleton::<DefenseRun>().gold,
        [250; 2]
    );
    assert_eq!(
        native
            .predicted_frame()
            .singleton::<DefenseRun>()
            .reactor_hp,
        75
    );
    let delay = native.config().input_delay;
    let mut pending = VecDeque::from(vec![DefenseInput::default(); delay as usize]);
    for tick in 0..120 {
        let value = if tick == 4 {
            DefenseInput {
                action: SUMMON,
                sequence: 1,
                ..Default::default()
            }
        } else if tick == 90 {
            DefenseInput {
                action: orr_games::coop_defense_game::RESTART,
                sequence: 2,
                ..Default::default()
            }
        } else {
            DefenseInput::default()
        };
        pending.push_back(value);
        play.session_mut()
            .set_input(PlayerSlot(0), pending.pop_front().unwrap());
        play.control(ControlOp::Step(1));
        native.advance(value, vec![]);
        assert_eq!(
            native.predicted_frame().to_bytes(),
            play.session().frame().to_bytes(),
            "tick {tick}"
        );
        if tick == 90 + delay {
            let run = native.predicted_frame().singleton::<DefenseRun>();
            assert_eq!(run.gold, [250; 2]);
            assert_eq!(run.reactor_hp, 75);
            assert_eq!(run.generation, 1);
        }
    }
}
