use super::*;
use crate::{Auth, Caps, ErpClient};
use orr_edit::{Op, Origin, PlayController};
use orr_games::coop_defense_game::{DefenseInput, DefenseRun, SUMMON};
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
