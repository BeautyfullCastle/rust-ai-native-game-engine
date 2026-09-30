mod common;

use std::time::Duration;

use common::*;
use orr_edit::PlayController;
use orr_remote::codec::b64_decode;
use orr_remote::{Auth, ErpClient, ServerConfig, PERMISSION_DENIED, UNAUTHENTICATED};
use orr_sample::physics_game::{PhysConfig, PhysGame, SceneMode};
use orr_session::{ControlOp, PlaySession};
use serde_json::{json, Value as J};

const BODY: &str = "orr_physics::Body";

fn scene_text(c: &mut ErpClient) -> String {
    c.call("scene.save", json!({})).unwrap()["text"].as_str().unwrap().to_string()
}

#[test]
fn discover_lists_methods_with_capabilities() {
    let host = TestHost::standard();
    let mut c = host.client("tok-read");
    let d = c.call("rpc.discover", J::Null).unwrap();
    let methods = d["methods"].as_array().unwrap();
    let find = |n: &str| methods.iter().find(|m| m["name"] == n).unwrap_or_else(|| panic!("method {n}"));
    assert_eq!(find("world.patch")["capability"], "scene_edit");
    assert_eq!(find("world.patch")["allowed"], false);
    assert_eq!(find("world.query")["allowed"], true);
    assert_eq!(find("sim.step")["capability"], "sim_control");
    assert_eq!(d["you"]["client"], "reader");
    for m in ["registry.schema", "world.spawn", "tx.begin", "history.undo", "scene.load", "sim.seek", "sim.branch", "sim.speed", "watch.subscribe"] {
        find(m);
    }
    assert!(find("world.patch")["params"].as_array().unwrap().iter().any(|p| p["name"] == "value" && p["required"] == true));
}

#[test]
fn agent_edit_shows_in_history_and_undo_restores_the_exact_yaml() {
    let host = TestHost::standard();
    let mut c = host.client("tok-all");
    let before = scene_text(&mut c);
    assert_eq!(before, demo_text(), "an unedited scene saves as the loaded text");
    let sum0 = checksum(&c.call("sim.checksum", json!({})).unwrap());

    let body = guid_of(&mut c, "body_05");
    let r = c.call("world.patch", json!({"entity": body, "component": BODY, "path": "vel", "value": [7, 3]})).unwrap();
    assert_eq!(r["changed"], true);
    assert_ne!(checksum(&r), sum0);
    let got = c.call("world.get", json!({"entity": body, "component": BODY, "path": "vel"})).unwrap();
    assert_eq!(got["value"], json!([7, 3]));
    assert_ne!(scene_text(&mut c), before);

    let h = c.call("history.list", J::Null).unwrap();
    let entries = h["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["origin"], "agent:claude");
    assert_eq!(entries[0]["undone"], false);
    assert_eq!(h["dirty"], true);

    let u = c.call("history.undo", J::Null).unwrap();
    assert_eq!(checksum(&u), sum0);
    assert_eq!(scene_text(&mut c), before, "undo restores the exact scene text");
    let h = c.call("history.list", J::Null).unwrap();
    assert_eq!(h["entries"][0]["undone"], true);
    assert_eq!(h["dirty"], false);

    c.call("history.redo", J::Null).unwrap();
    let got = c.call("world.get", json!({"entity": body, "component": BODY, "path": "vel"})).unwrap();
    assert_eq!(got["value"], json!([7, 3]));
    // Nothing more to redo: a clean state error.
    let e = c.call_err("history.redo", J::Null);
    assert_eq!(e.kind(), Some("nothing_to_redo"));
}

#[test]
fn a_batch_in_one_transaction_undoes_as_one() {
    let host = TestHost::standard();
    let mut c = host.client("tok-all");
    let before = scene_text(&mut c);
    let a = guid_of(&mut c, "body_01");
    let b = guid_of(&mut c, "body_02");

    c.call("tx.begin", json!({"label": "tune two bodies"})).unwrap();
    c.call("world.patch", json!({"entity": a, "component": BODY, "path": "pos.x", "value": "1.5"})).unwrap();
    c.call("world.patch", json!({"entity": b, "component": BODY, "path": "pos.y", "value": 20})).unwrap();
    c.call("world.rename", json!({"entity": b, "name": "renamed_body"})).unwrap();
    let s = c.call("world.spawn", json!({"name": "marker", "components": {"PaddleTag": {"slot": 1}}})).unwrap();
    assert!(s["guid"].as_str().unwrap().starts_with("e_"));
    // Not a history entry until commit.
    assert_eq!(c.call("history.list", J::Null).unwrap()["entries"].as_array().unwrap().len(), 0);
    c.call("tx.commit", J::Null).unwrap();

    let h = c.call("history.list", J::Null).unwrap();
    let entries = h["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "one transaction, one history entry");
    assert_eq!(entries[0]["label"], "tune two bodies");
    assert_eq!(entries[0]["op_count"], 4);
    assert_eq!(entries[0]["origin"], "agent:claude");
    assert_ne!(scene_text(&mut c), before);

    c.call("history.undo", J::Null).unwrap();
    assert_eq!(scene_text(&mut c), before);
    // Rollback takes back what a transaction did.
    c.call("tx.begin", J::Null).unwrap();
    c.call("world.patch", json!({"entity": a, "component": BODY, "path": "pos.x", "value": 3})).unwrap();
    c.call("tx.rollback", J::Null).unwrap();
    assert_eq!(scene_text(&mut c), before);
    // (Editing inside the second transaction dropped the undone entry: nothing to redo.)
    assert_eq!(c.call("history.list", J::Null).unwrap()["entries"].as_array().unwrap().len(), 0);
}

#[test]
fn a_transaction_belongs_to_its_client_and_dies_with_the_connection() {
    let host = TestHost::standard();
    let mut a = host.client("tok-all");
    let mut b = host.client("tok-other");
    let before = scene_text(&mut a);
    let body = guid_of(&mut a, "body_03");

    a.call("tx.begin", J::Null).unwrap();
    a.call("world.patch", json!({"entity": body, "component": BODY, "path": "pos.x", "value": 2})).unwrap();
    // Another client cannot edit or undo meanwhile.
    let e = b.call_err("world.patch", json!({"entity": body, "component": BODY, "path": "pos.x", "value": 9}));
    assert_eq!(e.kind(), Some("tx_busy"));
    assert_eq!(b.call_err("history.undo", J::Null).kind(), Some("tx_open"));
    assert_eq!(b.call_err("tx.begin", J::Null).kind(), Some("tx_open"));
    assert_eq!(b.call_err("tx.commit", J::Null).kind(), Some("tx_busy"));
    assert_eq!(b.call_err("tx.rollback", J::Null).kind(), Some("tx_busy"));

    // The client vanishes with the transaction open: it is rolled back.
    drop(a);
    let mut restored = false;
    for _ in 0..200 {
        if !b.call("history.list", J::Null).unwrap()["in_tx"].as_bool().unwrap() {
            restored = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(restored, "transaction was not rolled back");
    assert_eq!(scene_text(&mut b), before);
}

#[test]
fn a_transaction_left_open_times_out() {
    let mut cfg = ServerConfig::new(Auth::Tokens(vec![token("claude", "tok-all", "all")]));
    cfg.tx_timeout = Duration::from_millis(100);
    let host = TestHost::start(cfg);
    let mut c = host.client("tok-all");
    c.call("tx.begin", J::Null).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    // The host loop runs `poll` while idle, so the timeout has fired.
    assert_eq!(c.call("history.list", J::Null).unwrap()["in_tx"], false);
}

#[test]
fn capabilities_are_enforced() {
    let host = TestHost::standard();
    let mut r = host.client("tok-read");
    let body = guid_of(&mut r, "body_05");

    // Reading is fine.
    r.call("world.get", json!({"entity": body})).unwrap();
    r.call("registry.schema", J::Null).unwrap();
    r.call("sim.state", J::Null).unwrap();
    r.call("history.list", J::Null).unwrap();
    r.call("scene.save", json!({})).unwrap();

    for (method, params) in [
        ("world.patch", json!({"entity": body, "component": BODY, "path": "pos.x", "value": 1})),
        ("world.spawn", json!({})),
        ("world.despawn", json!({"entity": body})),
        ("tx.begin", J::Null),
        ("history.undo", J::Null),
        ("scene.load", json!({"text": "x"})),
        ("scene.save", json!({"write": true})),
        ("sim.start", J::Null),
        ("sim.step", json!({"n": 1})),
        ("sim.seek", json!({"tick": 0})),
        ("sim.play", J::Null),
        ("sim.debug", json!({"cmd": "despawn", "entity": "1v0"})),
    ] {
        let e = r.call_err(method, params);
        assert_eq!(e.code, PERMISSION_DENIED, "{method}: {e}");
        assert_eq!(e.kind(), Some("permission_denied"));
        assert!(e.message.contains("capability"), "{}", e.message);
    }
    // Nothing changed.
    assert_eq!(r.call("history.list", J::Null).unwrap()["entries"].as_array().unwrap().len(), 0);
    assert_eq!(r.call("sim.state", J::Null).unwrap()["mode"], "edit");

    // scene_edit alone edits the scene but does not run the sim.
    let mut e = host.client("tok-edit");
    e.call("world.patch", json!({"entity": body, "component": BODY, "path": "pos.x", "value": 1})).unwrap();
    assert_eq!(e.call_err("sim.start", J::Null).code, PERMISSION_DENIED);
    // sim_control alone runs the sim but does not edit the scene.
    let mut d = host.client("tok-sim");
    assert_eq!(d.call_err("world.patch", json!({"entity": body, "component": BODY, "path": "pos.x", "value": 2})).code, PERMISSION_DENIED);
    d.call("sim.start", J::Null).unwrap();
    d.call("sim.step", json!({"n": 3})).unwrap();
    // While playing, a world edit is a change of the sim and needs both.
    let err = e.call_err("world.patch", json!({"entity": body, "component": BODY, "path": "pos.x", "value": 5}));
    assert_eq!(err.code, PERMISSION_DENIED);
    assert_eq!(err.data.as_ref().unwrap()["required"], "sim_control");
    assert_eq!(d.call_err("world.patch", json!({"entity": body, "component": BODY, "path": "pos.x", "value": 5})).code, PERMISSION_DENIED);
    // The full token may.
    let mut all = host.client("tok-all");
    all.call("world.patch", json!({"entity": body, "component": BODY, "path": "pos.x", "value": 5})).unwrap();
}

#[test]
fn authentication() {
    let host = TestHost::standard();
    // Nothing works before `auth`.
    let mut raw = ErpClient::connect(&host.url, None).unwrap();
    let e = raw.call_err("world.query", J::Null);
    assert_eq!(e.code, UNAUTHENTICATED);
    // A wrong token is refused.
    let e = raw.call_err("auth", json!({"token": "nope"}));
    assert_eq!(e.code, UNAUTHENTICATED);
    // The right one works, on the same connection.
    let who = raw.call("auth", json!({"token": "tok-read"})).unwrap();
    assert_eq!(who["client"], "reader");
    assert_eq!(who["capabilities"], json!(["read"]));
    raw.call("world.query", J::Null).unwrap();
    // The token in the URL.
    let mut viaurl = ErpClient::connect_with_url_token(&host.url, "tok-edit").unwrap();
    viaurl.call("world.query", J::Null).unwrap();
    let d = viaurl.call("rpc.discover", J::Null).unwrap();
    assert_eq!(d["you"]["client"], "editor");
    // A wrong token in the URL refuses the connection.
    assert!(ErpClient::connect_with_url_token(&host.url, "nope").is_err());
    // Repeated failures close the connection.
    let mut bad = ErpClient::connect(&host.url, None).unwrap();
    let mut closed = false;
    for _ in 0..10 {
        if bad.call("auth", json!({"token": "x"})).is_ok() {
            panic!("bad token accepted");
        }
        if bad.call("rpc.discover", J::Null).is_err() && bad.call("rpc.discover", J::Null).unwrap_err().rpc().is_none() {
            closed = true;
            break;
        }
    }
    assert!(closed, "the connection should close after repeated auth failures");
}

#[test]
fn dev_mode_is_explicit_and_loopback_only() {
    let host = TestHost::dev();
    let mut c = ErpClient::connect(&host.url, None).unwrap();
    let d = c.call("rpc.discover", J::Null).unwrap();
    assert_eq!(d["you"]["client"], "dev");
    assert_eq!(d["you"]["capabilities"], json!(["read", "scene_edit", "sim_control", "approve"]));
    // Refused off loopback, and with no tokens at all.
    let mut cfg = ServerConfig::new(Auth::DevNoAuth);
    cfg.bind = "0.0.0.0:0".parse().unwrap();
    assert!(orr_remote::ErpServer::start(cfg).is_err());
    assert!(orr_remote::ErpServer::start(ServerConfig::new(Auth::Tokens(Vec::new()))).is_err());
}

#[test]
fn sim_start_step_seek_matches_an_in_process_run() {
    // In-process reference.
    let doc = demo_doc();
    let mut pc = PlayController::<PhysGame>::start_play(&doc, doc.play_config(2, 60)).unwrap();
    pc.control(ControlOp::Step(120));
    let want120 = pc.session().frame().checksum();
    let want30 = pc.session().checksum_at(30).expect("recorded");

    let host = TestHost::standard();
    let mut c = host.client("tok-all");
    let s = c.call("sim.start", json!({})).unwrap();
    assert_eq!(s["mode"], "play");
    assert_eq!(s["playing"], false);
    assert_eq!(u(&s, "head_tick"), 0);
    assert_eq!(checksum(&s), doc.checksum(), "play starts from the scene");

    let s = c.call("sim.step", json!({"n": 120})).unwrap();
    assert_eq!(u(&s, "head_tick"), 120);
    assert_eq!(checksum(&s), want120);
    assert_eq!(checksum(&c.call("sim.checksum", json!({"tick": 30})).unwrap()), want30);

    let s = c.call("sim.seek", json!({"tick": 30})).unwrap();
    assert_eq!(u(&s, "head_tick"), 30);
    assert_eq!(s["playing"], false);
    assert_eq!(checksum(&s), want30);
    assert_eq!(checksum(&c.call("sim.checksum", json!({})).unwrap()), want30);

    let s = c.call("sim.step", json!({"n": 90})).unwrap();
    assert_eq!(u(&s, "head_tick"), 120);
    assert_eq!(checksum(&s), want120, "seek + step lands on the straight run's checksum");
    assert_eq!(u(&s, "last_tick"), 120);
    assert_eq!(u(&s, "branches"), 1, "running from a rewound tick cuts the recorded future (a branch)");

    // Out of range and other refusals are errors, not panics.
    assert_eq!(c.call_err("sim.seek", json!({"tick": 9999})).code, orr_remote::INVALID_PARAMS);
    assert_eq!(c.call_err("sim.step", json!({"n": 0})).code, orr_remote::INVALID_PARAMS);
    assert_eq!(c.call_err("sim.step", json!({"n": 100000})).code, orr_remote::LIMIT_EXCEEDED);
    assert_eq!(c.call_err("sim.start", json!({})).kind(), Some("sim_running"));
    assert_eq!(c.call_err("scene.load", json!({"text": ""})).kind(), Some("sim_running"));

    // Speed, branch, pause.
    let s = c.call("sim.speed", json!({"permille": 2000})).unwrap();
    assert_eq!(u(&s, "speed_permille"), 2000);
    let s = c.call("sim.speed", json!({"permille": 99999})).unwrap();
    assert_eq!(u(&s, "speed_permille"), 4000, "clamped");
    c.call("sim.seek", json!({"tick": 60})).unwrap();
    let s = c.call("sim.branch", J::Null).unwrap();
    assert_eq!(u(&s, "branches"), 2, "the earlier rewound step branched once, this is the second");
    assert_eq!(u(&s, "last_tick"), 60);
    let s = c.call("sim.stop", json!({})).unwrap();
    assert_eq!(u(&s, "tick"), 60);
    assert_eq!(c.call("sim.state", J::Null).unwrap()["mode"], "edit");
    assert_eq!(c.call_err("sim.step", json!({"n": 1})).kind(), Some("no_play"));
}

#[test]
fn real_time_play_runs_by_the_host_clock() {
    let host = TestHost::standard();
    let mut c = host.client("tok-all");
    c.call("sim.start", json!({})).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(u(&c.call("sim.state", J::Null).unwrap(), "head_tick"), 0, "paused until told to play");
    c.call("sim.speed", json!({"permille": 4000})).unwrap();
    c.call("sim.play", J::Null).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let s = c.call("sim.pause", J::Null).unwrap();
    let head = u(&s, "head_tick");
    assert!(head >= 30, "4x speed for half a second should run well over 30 ticks, ran {head}");
    assert!(head <= 200, "ran {head}");
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(u(&c.call("sim.state", J::Null).unwrap(), "head_tick"), head, "paused stays paused");
    // The run is the deterministic one.
    let doc = demo_doc();
    let mut pc = PlayController::<PhysGame>::start_play(&doc, doc.play_config(2, 60)).unwrap();
    pc.control(ControlOp::Step(head as u32));
    assert_eq!(checksum(&c.call("sim.state", J::Null).unwrap()), pc.session().frame().checksum());
}

#[test]
fn an_edit_while_playing_is_recorded_and_replays_to_the_same_checksum() {
    let host = TestHost::standard();
    let mut c = host.client("tok-all");
    let body = guid_of(&mut c, "body_05");
    c.call("sim.start", json!({})).unwrap();
    c.call("sim.step", json!({"n": 20})).unwrap();
    let before = checksum(&c.call("sim.state", J::Null).unwrap());
    let r = c.call("world.patch", json!({"entity": body, "component": BODY, "path": "vel", "value": [7, 3]})).unwrap();
    assert_eq!(r["changed"], true);
    assert_ne!(checksum(&r), before, "the edit shows at once");
    assert_eq!(u(&c.call("sim.state", J::Null).unwrap(), "head_tick"), 20);
    assert_eq!(c.call("world.get", json!({"entity": body, "component": BODY, "path": "vel"})).unwrap()["value"], json!([7, 3]));
    // Same value again records nothing.
    assert_eq!(c.call("world.patch", json!({"entity": body, "component": BODY, "path": "vel", "value": [7, 3]})).unwrap()["changed"], false);
    c.call("world.singleton.patch", json!({"name": "Scene", "path": "max_entities", "value": 12345})).unwrap();
    // A spawn during play has no GUID, only a handle.
    let sp = c.call("world.spawn", json!({"components": {"PaddleTag": {"slot": 1}}})).unwrap();
    let handle = sp["handle"].as_str().unwrap().to_string();
    assert!(sp["guid"].is_null());
    assert_eq!(c.call("world.get", json!({"entity": handle, "component": "PaddleTag"})).unwrap()["value"], json!({"slot": 1}));
    c.call("world.despawn", json!({"entity": handle})).unwrap();
    // The document is untouched by play edits.
    assert_eq!(c.call("history.list", J::Null).unwrap()["entries"].as_array().unwrap().len(), 0);

    let s = c.call("sim.step", json!({"n": 40})).unwrap();
    let live = checksum(&s);
    let head = u(&s, "head_tick");
    let stopped = c.call("sim.stop", json!({"include_replay": true})).unwrap();
    assert_eq!(u(&stopped, "tick"), head);
    assert_eq!(checksum(&stopped), live);
    let bytes = b64_decode(stopped["replay"].as_str().unwrap()).unwrap();
    assert_eq!(bytes.len() as u64, u(&stopped, "replay_bytes"));

    // Replay it: the recorded debug commands land on the same frame.
    let cfg = PhysConfig::new(40, SceneMode::Rain);
    let mut viewer = PlaySession::<PhysGame>::open_replay(&bytes, cfg, 0).expect("open replay");
    let last = viewer.last_tick();
    viewer.control(ControlOp::Seek(last));
    assert_eq!(last, head);
    assert_eq!(viewer.frame().checksum(), live);

    // And the same edits made in process give the same result.
    let doc = demo_doc();
    let mut pc = PlayController::<PhysGame>::start_play(&doc, doc.play_config(2, 60)).unwrap();
    pc.control(ControlOp::Step(20));
    let target = orr_edit::Target::Guid(orr_reflect::Guid::parse(&body).unwrap());
    pc.set_field(&target, BODY, "vel", orr_reflect::Value::Vec2(orr_fp::FPVec2::new(orr_fp::FP::from_int(7), orr_fp::FP::from_int(3)))).unwrap();
    pc.set_singleton_field("Scene", "max_entities", orr_reflect::Value::Int(12345)).unwrap();
    let e = pc.spawn(&[("PaddleTag".to_string(), orr_reflect::Value::Struct(vec![("slot".into(), orr_reflect::Value::Int(1))]))]).unwrap();
    pc.despawn(&orr_edit::Target::Entity(e)).unwrap();
    pc.control(ControlOp::Step(40));
    assert_eq!(pc.session().frame().checksum(), live);
}

#[test]
fn scene_load_save_and_file_write() {
    let dir = std::env::temp_dir().join(format!("orr_remote_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("scene.scene.yaml");
    let mut cfg = ServerConfig::new(Auth::Tokens(vec![token("claude", "tok-all", "all"), token("reader", "tok-read", "read")]));
    cfg.limits.scene_path = Some(path.clone());
    let host = TestHost::start(cfg);
    let mut c = host.client("tok-all");
    let body = guid_of(&mut c, "body_07");
    c.call("world.patch", json!({"entity": body, "component": BODY, "path": "pos.x", "value": "2.25"})).unwrap();
    assert_eq!(c.call("history.list", J::Null).unwrap()["dirty"], true);
    let saved = c.call("scene.save", json!({"write": true})).unwrap();
    assert_eq!(saved["written"], path.display().to_string());
    assert_eq!(saved["dirty"], false);
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(on_disk, saved["text"].as_str().unwrap());
    assert!(on_disk.contains("2.25"));

    // A round trip through scene.load gives the same text and checksum.
    let sum = checksum(&saved);
    let mut c2 = host.client("tok-all");
    c2.call("scene.load", json!({"text": demo_text()})).unwrap();
    assert_ne!(checksum(&c2.call("sim.checksum", json!({})).unwrap()), sum);
    let loaded = c2.call("scene.load", json!({"text": on_disk})).unwrap();
    assert_eq!(checksum(&loaded), sum);
    assert_eq!(c2.call("scene.save", json!({})).unwrap()["text"], on_disk.as_str());
    // History was cleared by the load.
    assert_eq!(c2.call("history.list", J::Null).unwrap()["entries"].as_array().unwrap().len(), 0);

    // Bad scene text: an error with the position, and nothing changes.
    let e = c2.call_err("scene.load", json!({"text": "schema: orr.scene/1\nentities:\n  e_00000001:\n    name: x\n    Nope: {}\n"}));
    assert_eq!(e.kind(), Some("invalid_scene"));
    assert!(e.message.contains("Nope"), "{}", e.message);
    assert_eq!(checksum(&c2.call("sim.checksum", json!({})).unwrap()), sum);

    // A server without a scene file never writes one.
    let host2 = TestHost::standard();
    let mut c3 = host2.client("tok-all");
    assert_eq!(c3.call_err("scene.save", json!({"write": true})).kind(), Some("no_scene_path"));
    // A reader may not write even with a path configured.
    let mut r = host.client("tok-read");
    assert_eq!(r.call_err("scene.save", json!({"write": true})).code, PERMISSION_DENIED);
    // The client cannot pick the path.
    let e = c.call("scene.save", json!({"write": true, "path": "/tmp/evil"})).unwrap();
    assert_eq!(e["written"], path.display().to_string());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn queries_and_filters() {
    let host = TestHost::standard();
    let mut c = host.client("tok-read");
    let all = c.call("world.query", json!({})).unwrap();
    assert_eq!(u(&all, "total"), 49);
    assert_eq!(all["entities"].as_array().unwrap().len(), 49);
    let e0 = &all["entities"][0];
    assert!(e0["guid"].as_str().unwrap().starts_with("e_"));
    assert!(e0["components"].as_array().unwrap().iter().any(|n| n == BODY));

    let bodies = c.call("world.query", json!({"components": [BODY, "orr_physics::Collider"], "name": "body_", "limit": 5, "offset": 2})).unwrap();
    assert_eq!(u(&bodies, "total"), 40);
    assert_eq!(bodies["entities"].as_array().unwrap().len(), 5);
    assert_eq!(bodies["truncated"], true);
    let tags = c.call("world.query", json!({"components": ["PaddleTag"], "values": true})).unwrap();
    assert_eq!(u(&tags, "total"), 2);
    assert!(tags["entities"][0]["values"]["PaddleTag"]["slot"].is_number());

    assert_eq!(c.call_err("world.query", json!({"components": ["Nope"]})).kind(), Some("unknown_type"));
    assert_eq!(c.call_err("world.get", json!({"entity": "e_deadbeef"})).kind(), Some("unknown_entity"));
    assert_eq!(c.call_err("world.get", json!({"entity": "nonsense"})).code, orr_remote::INVALID_PARAMS);
    let one = c.call("world.get", json!({"entity": all["entities"][0]["guid"]})).unwrap();
    assert!(one["components"].is_object());
    let all_singletons = c.call("world.singleton.get", json!({})).unwrap();
    assert!(all_singletons["singletons"]["Scene"]["half_w"].is_number());
    assert_eq!(c.call("world.singleton.get", json!({"name": "Scene", "path": "max_entities"})).unwrap()["value"], 20000);
    let types = c.call("registry.types", J::Null).unwrap();
    assert!(types["types"].as_array().unwrap().iter().any(|t| t["name"] == "Scene" && t["kind"] == "singleton"));
}

#[test]
fn entity_edits_spawn_insert_remove_despawn_and_conflicts() {
    let host = TestHost::standard();
    let mut c = host.client("tok-all");
    let before = scene_text(&mut c);
    let s = c.call("world.spawn", json!({"name": "probe", "components": {"PaddleTag": {"slot": 0}}})).unwrap();
    let guid = s["guid"].as_str().unwrap().to_string();
    let q = c.call("world.query", json!({"name": "probe"})).unwrap();
    assert_eq!(q["entities"][0]["guid"], guid.as_str());
    assert_eq!(c.call_err("world.insert", json!({"entity": guid, "component": "PaddleTag"})).kind(), Some("has_component"));
    c.call("world.remove", json!({"entity": guid, "component": "PaddleTag"})).unwrap();
    c.call("world.insert", json!({"entity": guid, "component": "PaddleTag", "value": {"slot": 1}})).unwrap();
    assert_eq!(c.call("world.get", json!({"entity": guid, "component": "PaddleTag"})).unwrap()["value"]["slot"], 1);
    c.call("world.rename", json!({"entity": guid, "name": null})).unwrap();
    c.call("world.despawn", json!({"entity": guid})).unwrap();
    assert_eq!(c.call_err("world.despawn", json!({"entity": guid})).kind(), Some("unknown_entity"));
    assert_eq!(c.call_err("world.spawn", json!({"guid": "e_00000001"})).kind(), Some("guid_exists"));
    // Six entries in the history; undo them all, byte-identical.
    let n = c.call("history.list", J::Null).unwrap()["entries"].as_array().unwrap().len();
    assert_eq!(n, 5, "spawn, remove, insert, rename, despawn");
    for _ in 0..n {
        c.call("history.undo", J::Null).unwrap();
    }
    assert_eq!(scene_text(&mut c), before);
}

#[test]
fn invalid_values_are_refused_with_the_path() {
    let host = TestHost::standard();
    let mut c = host.client("tok-all");
    let before = scene_text(&mut c);
    let body = guid_of(&mut c, "body_05");
    let bad = |c: &mut ErpClient, path: &str, value: J| {
        let e = c.call_err("world.patch", json!({"entity": body, "component": BODY, "path": path, "value": value}));
        assert_eq!(e.code, orr_remote::INVALID_VALUE, "{path} {e}");
        e.message
    };
    assert!(bad(&mut c, "pos.x", json!("abc")).contains("plain decimal"));
    assert!(bad(&mut c, "pos.x", json!(true)).contains("expected a number"));
    assert!(bad(&mut c, "pos", json!([1])).contains("list of 2"));
    assert!(bad(&mut c, "pos", json!({"x": 1})).contains("list of 2"));
    assert!(bad(&mut c, "nope", json!(1)).contains("no field"));
    assert!(bad(&mut c, "kind", json!("flying")).contains("is not one of"));
    assert!(bad(&mut c, "pos.x", json!(1e300_f64)).contains("too large"));
    assert!(bad(&mut c, "", json!({"pos": [1, 2], "bogus": 1})).contains("unknown field"));
    assert_eq!(scene_text(&mut c), before, "refused edits change nothing");
    let e = c.call_err("world.patch", json!({"entity": body, "component": "Nope", "value": 1}));
    assert_eq!(e.kind(), Some("unknown_type"));
    let e = c.call_err("world.patch", json!({"entity": body, "component": BODY}));
    assert_eq!(e.code, orr_remote::INVALID_PARAMS);
    // A partial whole-component write fills the rest from the default.
    c.call("world.patch", json!({"entity": body, "component": BODY, "path": "", "value": {"pos": [3, 4]}})).unwrap();
    assert_eq!(c.call("world.get", json!({"entity": body, "component": BODY, "path": "pos"})).unwrap()["value"], json!([3, 4]));
}

#[test]
fn oversized_and_malformed_messages_get_errors_and_the_connection_survives() {
    let mut cfg = ServerConfig::new(Auth::DevNoAuth);
    cfg.max_message_bytes = 4096;
    let host = TestHost::start(cfg);
    let mut c = ErpClient::connect(&host.url, None).unwrap();
    let reply = |c: &mut ErpClient| -> J { serde_json::from_str(&c.recv_text(Duration::from_secs(5)).unwrap().expect("a reply")).unwrap() };

    c.send_text(&format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"scene.load\",\"params\":{{\"text\":\"{}\"}}}}", "a".repeat(10_000))).unwrap();
    let r = reply(&mut c);
    assert_eq!(r["error"]["code"], orr_remote::INVALID_REQUEST);
    assert_eq!(r["error"]["data"]["kind"], "too_large");

    c.send_text("{not json").unwrap();
    assert_eq!(reply(&mut c)["error"]["code"], orr_remote::PARSE_ERROR);
    c.send_text("[1,2,3]").unwrap();
    assert_eq!(reply(&mut c)["error"]["code"], orr_remote::INVALID_REQUEST);
    c.send_text("42").unwrap();
    assert_eq!(reply(&mut c)["error"]["code"], orr_remote::INVALID_REQUEST);
    c.send_text(r#"{"jsonrpc":"1.0","id":1,"method":"x"}"#).unwrap();
    assert_eq!(reply(&mut c)["error"]["code"], orr_remote::INVALID_REQUEST);
    c.send_text(r#"{"jsonrpc":"2.0","id":[1],"method":"x"}"#).unwrap();
    assert_eq!(reply(&mut c)["error"]["code"], orr_remote::INVALID_REQUEST);
    c.send_text(r#"{"jsonrpc":"2.0","id":7,"method":5}"#).unwrap();
    let r = reply(&mut c);
    assert_eq!(r["error"]["code"], orr_remote::INVALID_REQUEST);
    assert_eq!(r["id"], 7);
    c.send_text(r#"{"jsonrpc":"2.0","id":8,"method":"nope.nothing"}"#).unwrap();
    let r = reply(&mut c);
    assert_eq!(r["error"]["code"], orr_remote::METHOD_NOT_FOUND);
    assert_eq!(r["id"], 8);
    c.send_text(r#"{"jsonrpc":"2.0","id":9,"method":"world.query","params":[1]}"#).unwrap();
    assert_eq!(reply(&mut c)["error"]["code"], orr_remote::INVALID_PARAMS);
    c.send_text(r#"{"jsonrpc":"2.0","id":10,"method":"world.query","params":{"limit":"many"}}"#).unwrap();
    assert_eq!(reply(&mut c)["error"]["code"], orr_remote::INVALID_PARAMS);
    c.send_binary(&[1, 2, 3]).unwrap();
    assert_eq!(reply(&mut c)["error"]["code"], orr_remote::INVALID_REQUEST);
    // A very deep document is a parse error, not a stack overflow.
    c.send_text(&"[".repeat(3000)).unwrap();
    assert_eq!(reply(&mut c)["error"]["code"], orr_remote::PARSE_ERROR);
    // A notification (no id) gets no reply, and the connection still works.
    c.send_text(r#"{"jsonrpc":"2.0","method":"sim.state"}"#).unwrap();
    assert_eq!(c.call("sim.state", J::Null).unwrap()["mode"], "edit");
    assert!(host.is_running());

    // Far over the limit (4x): the connection is closed.
    let huge = format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"x\",\"params\":{{\"t\":\"{}\"}}}}", "a".repeat(200_000));
    let _ = c.send_text(&huge);
    let mut closed = false;
    for _ in 0..20 {
        if c.call("sim.state", J::Null).is_err() {
            closed = true;
            break;
        }
    }
    assert!(closed);
    // The server is fine for others.
    let mut c2 = ErpClient::connect(&host.url, None).unwrap();
    c2.call("sim.state", J::Null).unwrap();
}

#[test]
fn newline_delimited_json_over_plain_tcp() {
    use std::io::{BufRead, BufReader, Write};
    let host = TestHost::standard();
    let stream = std::net::TcpStream::connect(host.addr).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut w = stream.try_clone().unwrap();
    let mut r = BufReader::new(stream);
    let mut ask = |line: &str| -> J {
        w.write_all(line.as_bytes()).unwrap();
        w.write_all(b"\n").unwrap();
        let mut out = String::new();
        r.read_line(&mut out).unwrap();
        serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out:?}"))
    };
    assert_eq!(ask(r#"{"jsonrpc":"2.0","id":1,"method":"sim.state"}"#)["error"]["code"], UNAUTHENTICATED);
    assert_eq!(ask(r#"{"jsonrpc":"2.0","id":2,"method":"auth","params":{"token":"tok-read"}}"#)["result"]["client"], "reader");
    let q = ask(r#"{"jsonrpc":"2.0","id":3,"method":"world.query","params":{"limit":1}}"#);
    assert_eq!(q["result"]["total"], 49);
    assert_eq!(ask(r#"{"jsonrpc":"2.0","id":4,"method":"world.spawn"}"#)["error"]["code"], PERMISSION_DENIED);
    assert_eq!(ask("garbage")["error"]["code"], orr_remote::PARSE_ERROR);
    let e = ask(r#"{"jsonrpc":"2.0","id":5,"method":"watch.subscribe","params":{"topics":["frames"]}}"#);
    assert!(e["error"]["message"].as_str().unwrap().contains("WebSocket"));
    assert_eq!(ask(r#"{"jsonrpc":"2.0","id":6,"method":"sim.state"}"#)["result"]["mode"], "edit");
}

#[test]
fn browser_origins_are_refused() {
    use tungstenite::client::IntoClientRequest;
    let host = TestHost::dev();
    let mut req = host.url.as_str().into_client_request().unwrap();
    req.headers_mut().insert("Origin", "http://evil.example".parse().unwrap());
    let err = tungstenite::connect(req).expect_err("a browser origin must be refused");
    assert!(err.to_string().contains("403"), "{err}");
    // Allowed when listed.
    let mut cfg = ServerConfig::new(Auth::DevNoAuth);
    cfg.allowed_origins = vec!["http://localhost:3000".into()];
    let host = TestHost::start(cfg);
    let mut req = host.url.as_str().into_client_request().unwrap();
    req.headers_mut().insert("Origin", "http://localhost:3000".parse().unwrap());
    tungstenite::connect(req).expect("listed origin connects");
}
