#![allow(clippy::disallowed_types)] // a tool crate: waits use the wall clock

mod common;

use std::time::{Duration, Instant};

use common::*;
use orr_bridge::{Bridge, BridgeError, BridgeEvent, DebugCommand, EventKey, Lifecycle, SimControl, Snapshot};
use orr_edit::{EditorDoc, Op, Origin, PlayController};
use orr_physics::{Body, BODY_DYNAMIC};
use orr_remote::{Auth, ErpClient, ErpServer, ErpTarget, RemoteBridge, RemoteConfig, ServerConfig};
use orr_sample::physics_game::{PhysEvent, PhysGame};
use orr_session::{ControlOp, Speed};
use orr_sim::{PlayerSlot, SimEvent};
use serde_json::{json, Value as J};

fn wait_for<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn wait_tick(b: &RemoteBridge<PhysGame>, tick: u64) -> Snapshot {
    wait_for(&format!("a snapshot of tick {tick}"), || b.snapshot().filter(|s| s.tick() == tick))
}

fn bridge(host: &TestHost, token: &str) -> RemoteBridge<PhysGame> {
    let mut cfg = RemoteConfig::new(&host.url);
    cfg.token = Some(token.to_string());
    RemoteBridge::<PhysGame>::connect(cfg).expect("connect bridge")
}

#[test]
fn watch_tick_and_history_notifications_arrive() {
    let host = TestHost::standard();
    let mut w = host.client("tok-read");
    let mut a = host.client("tok-all");
    let subs = w.call("watch.subscribe", json!({"topics": ["tick", "history"]})).unwrap();
    assert_eq!(subs["topics"], json!(["tick", "history"]));
    // The current state comes at once.
    let first = w.wait_notification("watch.tick", Duration::from_secs(5)).unwrap().expect("initial tick note");
    assert_eq!(first["params"]["mode"], "edit");
    let h0 = w.wait_notification("watch.history", Duration::from_secs(5)).unwrap().expect("initial history note");
    assert_eq!(h0["params"]["len"], 0);

    // Someone else edits: a history note with the agent origin.
    let body = guid_of(&mut a, "body_01");
    a.call("world.patch", json!({"entity": body, "component": "orr_physics::Body", "path": "pos.x", "value": 1})).unwrap();
    let h = w.wait_notification("watch.history", Duration::from_secs(5)).unwrap().expect("history note");
    assert_eq!(h["params"]["len"], 1);
    assert_eq!(h["params"]["last"]["origin"], "agent:claude");
    assert_eq!(h["params"]["dirty"], true);
    a.call("history.undo", J::Null).unwrap();
    let h = w.wait_notification("watch.history", Duration::from_secs(5)).unwrap().expect("history note after undo");
    assert_eq!(h["params"]["last"]["undone"], true);

    // Play: tick notes carry tick and checksum.
    a.call("sim.start", J::Null).unwrap();
    let n = w.wait_notification("watch.tick", Duration::from_secs(5)).unwrap().expect("tick note at start");
    assert_eq!(n["params"]["mode"], "play");
    a.call("sim.step", json!({"n": 3})).unwrap();
    let want = checksum(&a.call("sim.checksum", json!({})).unwrap());
    let n = loop {
        let n = w.wait_notification("watch.tick", Duration::from_secs(5)).unwrap().expect("tick note");
        if n["params"]["tick"] == 3 {
            break n;
        }
    };
    assert_eq!(orr_remote::wire::parse_checksum(&n["params"]["checksum"]), Some(want));
    assert_eq!(n["params"]["playing"], false);

    // Real-time play pushes a note per tick.
    a.call("sim.speed", json!({"permille": 4000})).unwrap();
    a.call("sim.play", J::Null).unwrap();
    let mut ticks = std::collections::BTreeSet::new();
    let end = Instant::now() + Duration::from_millis(400);
    while Instant::now() < end {
        if let Some(n) = w.wait_notification("watch.tick", Duration::from_millis(50)).unwrap() {
            ticks.insert(n["params"]["tick"].as_u64().unwrap());
        }
    }
    a.call("sim.pause", J::Null).unwrap();
    assert!(ticks.len() >= 20, "expected a note for most ticks at 4x, got {}", ticks.len());

    // Unsubscribing stops them.
    w.call("watch.unsubscribe", json!({})).unwrap();
    w.notifications.clear();
    a.call("sim.step", json!({"n": 5})).unwrap();
    a.call("sim.state", J::Null).unwrap();
    assert!(w.wait_notification("watch.tick", Duration::from_millis(300)).unwrap().is_none());
    // Errors: unknown topic, no topics, frames need a capability the reader has (read), other clients are unaffected.
    assert_eq!(w.call_err("watch.subscribe", json!({"topics": ["bogus"]})).code, orr_remote::INVALID_PARAMS);
    assert_eq!(w.call_err("watch.subscribe", json!({})).code, orr_remote::INVALID_PARAMS);
}

#[test]
fn notes_are_pushed_to_subscribers() {
    let host = TestHost::standard();
    let mut w = host.client("tok-read");
    let mut a = host.client("tok-all");
    w.call("watch.subscribe", json!({"topics": ["notes"]})).unwrap();
    a.call("sim.start", J::Null).unwrap();
    a.call("sim.step", json!({"n": 10})).unwrap();
    a.call("sim.seek", json!({"tick": 4})).unwrap();
    a.call("sim.play", J::Null).unwrap();
    let mut kinds = Vec::new();
    while kinds.len() < 2 {
        let n = w.wait_notification("watch.notes", Duration::from_secs(5)).unwrap().expect("note");
        for note in n["params"]["notes"].as_array().unwrap() {
            kinds.push(note["kind"].as_str().unwrap().to_string());
        }
    }
    assert_eq!(kinds[0], "seeked");
    assert!(kinds.contains(&"resumed".to_string()), "{kinds:?}");
}

/// The embedding case: the host owns the document and the play session and polls the
/// server itself (as the egui editor does). A person's edit and an agent's edit share one
/// undo stack.
#[test]
fn an_embedded_host_and_an_agent_share_one_undo_stack() {
    let mut doc = demo_doc();
    let mut play: Option<PlayController<PhysGame>> = None;
    let mut server = ErpServer::start(ServerConfig::new(Auth::Tokens(vec![token("claude", "tok", "all")]))).unwrap();
    let url = server.url();
    let original = doc.to_yaml();
    let body = orr_reflect::Guid::from_u32(10);

    // The person edits first.
    doc.apply(
        Op::SetField { guid: body.clone(), component: "orr_physics::Body".into(), path: "pos.x".into(), value: orr_reflect::Value::Fixed(orr_fp::FP::from_int(3)) },
        Origin::User,
    )
    .unwrap();

    let agent = std::thread::spawn(move || {
        let mut c = ErpClient::connect(&url, Some("tok")).unwrap();
        let h = c.call("history.list", J::Null).unwrap();
        assert_eq!(h["entries"][0]["origin"], "user", "the agent sees the person's edit");
        c.call("world.patch", json!({"entity": "e_0000000a", "component": "orr_physics::Body", "path": "pos.y", "value": 9})).unwrap();
        let h = c.call("history.list", J::Null).unwrap();
        assert_eq!(h["entries"].as_array().unwrap().len(), 2);
        assert_eq!(h["entries"][1]["origin"], "agent:claude");
        // The agent undoes its own edit and then the person's (one stack).
        c.call("history.undo", J::Null).unwrap();
        c.call("history.undo", J::Null).unwrap();
        c.call("sim.state", J::Null).unwrap()
    });
    while !agent.is_finished() {
        server.poll(&mut ErpTarget { doc: &mut doc, play: &mut play });
        std::thread::sleep(Duration::from_millis(1));
    }
    let state = agent.join().unwrap();
    assert_eq!(state["dirty"], false);
    assert_eq!(doc.to_yaml(), original, "both edits are undone, byte for byte");
    // The person can redo what the agent undid.
    doc.redo().unwrap();
    let x = doc.view().field(&orr_edit::Target::Guid(body), "orr_physics::Body", "pos.x").unwrap();
    assert_eq!(x, orr_reflect::Value::Fixed(orr_fp::FP::from_int(3)));
    let _ = EditorDoc::from_yaml(&doc.to_yaml(), types(), orr_sim::Simulation::<PhysGame>::build_registry(), SEED).unwrap();
}

#[test]
fn events_are_forwarded_to_subscribers() {
    let mut doc = demo_doc();
    let mut play: Option<PlayController<PhysGame>> = None;
    let mut server = ErpServer::start(ServerConfig::new(Auth::DevNoAuth)).unwrap();
    let url = server.url();
    let t = std::thread::spawn(move || {
        let mut c = ErpClient::connect(&url, None).unwrap();
        c.call("watch.subscribe", json!({"topics": ["events"]})).unwrap();
        c.wait_notification("watch.events", Duration::from_secs(10)).unwrap().expect("events note")
    });
    // Wait until the subscription is in, then push events the way a host does after real-time ticks.
    let mut spins = 0;
    while !t.is_finished() {
        server.poll(&mut ErpTarget { doc: &mut doc, play: &mut play });
        // Events pushed before the subscription is in are dropped, so keep pushing.
        if spins % 20 == 0 {
            server.push_events(&[SimEvent { key: EventKey::new(7, 2, 5), payload: PhysEvent { kind: 1, a: 3, b: 4 } }]);
        }
        spins += 1;
        std::thread::sleep(Duration::from_millis(1));
    }
    let n = t.join().unwrap();
    let e = &n["params"]["events"][0];
    assert_eq!((e["tick"].as_u64(), e["system"].as_u64(), e["seq"].as_u64()), (Some(7), Some(2), Some(5)));
    let bytes = orr_remote::codec::hex_decode(e["payload"].as_str().unwrap()).unwrap();
    let back: PhysEvent = bytemuck::pod_read_unaligned(&bytes);
    assert_eq!(back, PhysEvent { kind: 1, a: 3, b: 4 });
}

#[test]
fn remote_bridge_snapshot_checksum_equals_the_host_frame() {
    let host = TestHost::standard();
    let mut c = host.client("tok-all");
    c.call("sim.start", J::Null).unwrap();
    let mut b = bridge(&host, "tok-all");
    assert_eq!(b.tick_rate(), 60);
    assert_eq!(b.player_count(), 2);
    assert_eq!(b.local_slot(), PlayerSlot(0));
    assert!(b.is_alive());
    let events = b.drain_events();
    assert!(matches!(events[0], BridgeEvent::Lifecycle(Lifecycle::SessionStarted { tick_rate: 60, player_count: 2, .. })), "{events:?}");

    // Tick 0: the scene.
    let s0 = wait_tick(&b, 0);
    assert_eq!(s0.predicted().checksum(), demo_doc().checksum());
    assert_eq!(s0.tick_rate(), 60);
    assert!(s0.predicted_prev().is_none());
    assert!(s0.timeline().is_some());

    // Step 30 (one request: one frame), then compare with the host.
    b.control(ControlOp::Step(30)).unwrap();
    let s30 = wait_tick(&b, 30);
    let want = checksum(&c.call("sim.checksum", json!({"tick": 30})).unwrap());
    assert_eq!(s30.predicted().checksum(), want, "snapshot checksum equals the host frame's");
    assert_eq!(s30.verified().unwrap().checksum(), want);
    assert_eq!(s30.timeline().unwrap().tick, 30);
    assert_eq!(s30.timeline().unwrap().checksum, want);
    assert_eq!(s30.verified_tick(), 30);
    assert_eq!(s30.predicted().alive_count(), 49);
    // The frame is the real thing: bodies are where the host has them.
    let body_count = s30.predicted().iter::<Body>().count();
    assert!(body_count >= 40, "{body_count}");

    // One more tick: the previous frame is offered for interpolation.
    b.control(ControlOp::Step(1)).unwrap();
    let s31 = wait_tick(&b, 31);
    let prev = s31.predicted_prev().expect("previous tick");
    assert_eq!(prev.tick(), 30);
    assert_eq!(prev.checksum(), want);
    assert!(s31.seq() > s30.seq());

    // Seek: a jump, so no previous frame, and a Seeked lifecycle event.
    b.control(ControlOp::Seek(10)).unwrap();
    let s10 = wait_tick(&b, 10);
    assert_eq!(s10.predicted().checksum(), checksum(&c.call("sim.checksum", json!({"tick": 10})).unwrap()));
    assert!(s10.predicted_prev().is_none());
    wait_for("Seeked", || b.drain_events().into_iter().find(|e| matches!(e, BridgeEvent::Lifecycle(Lifecycle::Seeked { from: 31, to: 10 }))));
    assert_eq!(b.timeline().unwrap().tick, 10);

    // Speed, play and pause reach the host.
    b.control(ControlOp::SetSpeed(Speed(4000))).unwrap();
    b.control(ControlOp::Play).unwrap();
    wait_for("Resumed", || b.drain_events().into_iter().find(|e| matches!(e, BridgeEvent::Lifecycle(Lifecycle::Resumed { .. }))));
    let running = wait_for("a running snapshot", || b.snapshot().filter(|s| s.tick() > 20 && s.timeline().unwrap().playing));
    assert!(running.timeline().unwrap().speed == Speed(4000));
    b.control(ControlOp::Pause).unwrap();
    wait_for("Paused", || b.drain_events().into_iter().find(|e| matches!(e, BridgeEvent::Lifecycle(Lifecycle::Paused { .. }))));
    // Once paused, the newest snapshot matches the host's head exactly.
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let st = c.call("sim.state", J::Null).unwrap();
        if let Some(s) = b.snapshot() {
            if s.tick() == u(&st, "head_tick") && s.predicted().checksum() == checksum(&st) {
                break;
            }
        }
        assert!(Instant::now() < end, "bridge did not converge on the host head");
        std::thread::sleep(Duration::from_millis(5));
    }
    let m = b.metrics();
    assert!(m.frames >= 5 && m.frame_bytes > 0 && m.last_frame_raw_bytes > m.last_frame_bytes / 2);
    assert!(b.take_errors().is_empty());
}

#[test]
fn remote_bridge_inputs_debug_commands_and_errors() {
    let host = TestHost::standard();
    let mut c = host.client("tok-all");
    c.call("sim.start", J::Null).unwrap();
    let mut b = bridge(&host, "tok-all");
    let s0 = wait_tick(&b, 0);

    // Only the local slot is accepted.
    let input = orr_sample::physics_game::PhysInput::new(1, 0, 0, false);
    assert!(matches!(b.set_input(PlayerSlot(1), input), Err(BridgeError::NotLocalPlayer { .. })));
    b.set_input(PlayerSlot(0), input).unwrap();
    b.set_input(PlayerSlot(0), input).unwrap(); // unchanged: not sent again
    b.control(ControlOp::Step(20)).unwrap();
    let s20 = wait_tick(&b, 20);
    // The same run in process, with the same held input.
    let doc = demo_doc();
    let mut pc = PlayController::<PhysGame>::start_play(&doc, doc.play_config(2, 60)).unwrap();
    pc.session_mut().set_input(PlayerSlot(0), input);
    pc.control(ControlOp::Step(20));
    assert_eq!(s20.predicted().checksum(), pc.session().frame().checksum(), "the input reached the sim");
    assert_ne!(s20.predicted().checksum(), checksum(&c.call("sim.checksum", json!({"tick": 0})).unwrap()));

    // A raw debug command: set the velocity of a dynamic body.
    let types = types();
    let reg = orr_sim::Simulation::<PhysGame>::build_registry();
    let comp = (0..reg.component_count()).map(orr_ecs::ComponentId).find(|&id| reg.component_name(id) == "orr_physics::Body").unwrap();
    let vel_offset = types.get("orr_physics::Body").unwrap().fields().iter().find(|f| f.name == "vel").unwrap().offset;
    let (entity, _) = s20.predicted().iter::<Body>().find(|(_, body)| body.kind == BODY_DYNAMIC).unwrap();
    let vel = orr_fp::FPVec2::new(orr_fp::FP::from_int(7), orr_fp::FP::from_int(3));
    let before_epoch = s20.timeline().unwrap().epoch;
    b.debug_command(DebugCommand::SetField { entity, component: comp, offset: vel_offset as u32, bytes: bytemuck::bytes_of(&vel).to_vec() }).unwrap();
    let edited = wait_for("the edit snapshot", || b.snapshot().filter(|s| s.timeline().unwrap().epoch != before_epoch));
    assert_eq!(edited.tick(), 20);
    assert_eq!(edited.predicted().get::<Body>(entity).unwrap().vel, vel);
    assert_eq!(edited.timeline().unwrap().pending_edits, 1);

    // A refused debug command: a Lifecycle event, and the error is kept.
    b.debug_command(DebugCommand::Despawn { entity: orr_ecs::Entity { index: 9999, version: 7 } }).unwrap();
    wait_for("DebugRejected", || b.drain_events().into_iter().find(|e| matches!(e, BridgeEvent::Lifecycle(Lifecycle::DebugRejected(_)))));
    let errs = wait_for("the error", || Some(b.take_errors()).filter(|e| !e.is_empty()));
    assert_eq!(errs[0].code, orr_remote::DEBUG_REFUSED);
    let _ = s0;

    // A read-only token can watch but not steer: the calls return, the refusals are kept.
    let mut r = bridge(&host, "tok-read");
    wait_for("the reader's first snapshot", || r.snapshot());
    r.control(ControlOp::Step(5)).unwrap();
    let errs = wait_for("the refusal", || Some(r.take_errors()).filter(|e| !e.is_empty()));
    assert_eq!(errs[0].code, orr_remote::PERMISSION_DENIED);
    assert_eq!(r.snapshot().unwrap().tick(), edited.tick());
    // A wrong token fails to connect.
    let mut cfg = RemoteConfig::new(&host.url);
    cfg.token = Some("nope".into());
    assert!(RemoteBridge::<PhysGame>::connect(cfg).is_err());
    // Connection loss is reported.
    drop(host);
    wait_for("disconnect", || (!b.is_alive()).then_some(()));
    let events = b.drain_events();
    assert!(events.iter().any(|e| matches!(e, BridgeEvent::Lifecycle(Lifecycle::Disconnected))), "{events:?}");
    assert!(matches!(b.control(ControlOp::Pause), Err(BridgeError::Disconnected)));
}

#[test]
fn remote_bridge_events_decode() {
    // The events the server forwards decode into `BridgeEvent::Sim` on the client.
    let mut doc = demo_doc();
    let mut play: Option<PlayController<PhysGame>> = None;
    let mut server = ErpServer::start(ServerConfig::new(Auth::DevNoAuth)).unwrap();
    let url = server.url();
    let t = std::thread::spawn(move || {
        let b = RemoteBridge::<PhysGame>::connect(RemoteConfig::new(&url)).unwrap();
        let mut b = b;
        wait_for("a sim event", || b.drain_events().into_iter().find(|e| matches!(e, BridgeEvent::Sim { .. })))
    });
    let mut spins = 0;
    while !t.is_finished() {
        server.poll(&mut ErpTarget { doc: &mut doc, play: &mut play });
        spins += 1;
        if spins % 20 == 0 {
            server.push_events(&[SimEvent { key: EventKey::new(3, 1, 0), payload: PhysEvent { kind: 2, a: 5, b: 6 } }]);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    match t.join().unwrap() {
        BridgeEvent::Sim { key, status } => {
            assert_eq!(key, EventKey::new(3, 1, 0));
            assert_eq!(status, orr_bridge::EventStatus::Verified(PhysEvent { kind: 2, a: 5, b: 6 }));
        }
        other => panic!("{other:?}"),
    }
}
