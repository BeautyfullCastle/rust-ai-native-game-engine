#![allow(clippy::disallowed_types)]
//! The in-process transport and the host thread: a client in the same
//! process as the host (the editor's way), next to socket clients.

mod common;

use std::time::{Duration, Instant};

use common::*;
use orr_bridge::{Bridge, SimControl};
use orr_remote::sample::spawn_phys_host;
use orr_remote::{Auth, Caps, ErpClient, LocalHost, RemoteBridge, RemoteConfig, ServerConfig, Transport, USER_CLIENT};
use orr_sample::physics_game::PhysGame;
use serde_json::{json, Value as J};

const BODY: &str = "orr_physics::Body";

/// A local-only host (no socket unless `listen`).
fn host(listen: bool) -> LocalHost {
    let mut cfg = ServerConfig::new(Auth::DevNoAuth);
    cfg.listen = listen;
    cfg.limits.debug_hooks = true;
    spawn_phys_host(demo_text(), None, cfg).expect("start the host thread")
}

fn local_client(h: &LocalHost, name: &str) -> ErpClient {
    ErpClient::with_transport(Box::new(h.connector().connect(name, Caps::ALL).expect("connect")))
}

fn wait_snapshot(b: &RemoteBridge<PhysGame>, pred: impl Fn(&orr_bridge::Snapshot) -> bool) -> orr_bridge::Snapshot {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(s) = b.snapshot() {
            if pred(&s) {
                return s;
            }
        }
        assert!(Instant::now() < end, "no matching snapshot");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn an_in_process_client_serves_the_same_methods_without_a_socket() {
    let h = host(false);
    assert!(h.url().is_none(), "no socket");
    let mut c = local_client(&h, "user");
    let r = c.call("world.query", json!({"name": "body_05"})).unwrap();
    let guid = r["entities"][0]["guid"].as_str().unwrap().to_string();
    c.call("world.patch", json!({"entity": guid, "component": BODY, "path": "pos.x", "value": "1.5"})).unwrap();
    // A client named `user` is a person's view: its edits carry the user origin.
    let hist = c.call("history.list", J::Null).unwrap();
    assert_eq!(hist["entries"][0]["origin"], "user");
    // An error is an error as over a socket.
    let e = c.call_err("world.patch", json!({"entity": "e_deadbeef", "component": BODY, "path": "pos.x", "value": 1}));
    assert_eq!(e.kind(), Some("unknown_entity"));
    // Notifications reach the client.
    c.call("watch.subscribe", json!({"topics": ["history"]})).unwrap();
    assert!(c.wait_notification("watch.history", Duration::from_secs(5)).unwrap().is_some());
    // post + poll never wait.
    let id = c.post("sim.state", J::Null).unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    let state = loop {
        c.poll().unwrap();
        if let Some(r) = c.take_response(id) {
            break r.unwrap();
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(state["mode"], "edit");
    assert_eq!(USER_CLIENT, "user");
}

#[test]
fn a_socket_agent_and_the_in_process_view_edit_one_document() {
    let h = host(true);
    let url = h.url().expect("listening").to_string();
    let mut view = local_client(&h, "user");
    let mut agent = ErpClient::connect(&url, None).unwrap();
    let guid = {
        let r = view.call("world.query", json!({"name": "body_05"})).unwrap();
        r["entities"][0]["guid"].as_str().unwrap().to_string()
    };
    agent.call("world.patch", json!({"entity": guid, "component": BODY, "path": "pos.x", "value": "2.5"})).unwrap();
    // The view sees what the agent did, and the agent sees the view's edit, in one history.
    let v = view.call("world.get", json!({"entity": guid, "component": BODY, "path": "pos.x"})).unwrap();
    assert_eq!(v["value"].to_string(), "2.5");
    view.call("world.patch", json!({"entity": guid, "component": BODY, "path": "pos.x", "value": "3.5"})).unwrap();
    let hist = agent.call("history.list", J::Null).unwrap();
    let origins: Vec<&str> = hist["entries"].as_array().unwrap().iter().map(|e| e["origin"].as_str().unwrap()).collect();
    assert_eq!(origins, ["agent:dev", "user"]);
    // The agent's undo takes back the view's edit.
    agent.call("history.undo", J::Null).unwrap();
    let v = view.call("world.get", json!({"entity": guid, "component": BODY, "path": "pos.x"})).unwrap();
    assert_eq!(v["value"].to_string(), "2.5");
}

#[test]
fn dev_mode_clients_may_say_they_are_the_user() {
    let h = host(true);
    let url = h.url().unwrap().to_string();
    let mut c = ErpClient::connect(&format!("{url}/?client=user"), None).unwrap();
    let r = c.call("world.query", json!({"name": "body_05"})).unwrap();
    let guid = r["entities"][0]["guid"].as_str().unwrap().to_string();
    c.call("world.patch", json!({"entity": guid, "component": BODY, "path": "pos.x", "value": 1})).unwrap();
    let hist = c.call("history.list", J::Null).unwrap();
    assert_eq!(hist["entries"][0]["origin"], "user");
    let who = c.call("rpc.discover", J::Null).unwrap();
    assert_eq!(who["you"]["client"], "user");
    // A token host ignores the query: the name comes from the token.
    let th = TestHost::standard();
    let mut t = ErpClient::connect(&format!("{}/?client=user", th.url), Some("tok-all")).unwrap();
    assert_eq!(t.call("rpc.discover", J::Null).unwrap()["you"]["client"], "claude");
}

#[test]
fn frames_reach_an_in_process_bridge_as_shared_copies_and_follow_edits() {
    let h = host(false);
    let mut view = local_client(&h, "user");
    let t = h.connector().connect("user", Caps::ALL).unwrap();
    let mut cfg = RemoteConfig::new("");
    cfg.source = "view".into();
    let mut b = RemoteBridge::<PhysGame>::connect_transport(Box::new(t) as Box<dyn Transport>, cfg).expect("bridge");

    // Edit mode: the scene's preview frame (tick 0, no timeline).
    let s0 = wait_snapshot(&b, |_| true);
    assert_eq!(s0.tick(), 0);
    assert!(s0.timeline().is_none());
    let sum0 = s0.predicted().checksum();
    assert_eq!(b.metrics().last_frame_bytes, 0, "nothing was serialized");

    // An edit publishes a new frame.
    let r = view.call("world.query", json!({"name": "body_05"})).unwrap();
    let guid = r["entities"][0]["guid"].as_str().unwrap().to_string();
    view.call("world.patch", json!({"entity": guid, "component": BODY, "path": "pos.x", "value": "1.25"})).unwrap();
    let s1 = wait_snapshot(&b, |s| s.predicted().checksum() != sum0);
    assert!(s1.seq() > s0.seq());

    // Play: the same stream carries the session's frames, with a timeline.
    view.call("sim.start", json!({})).unwrap();
    b.control(orr_bridge::ControlOp::Step(10)).unwrap();
    let s = wait_snapshot(&b, |s| s.tick() == 10);
    assert!(s.timeline().is_some());
    assert!(b.take_errors().is_empty());
    // Stop: back to the scene frame, which has the edit.
    view.call("sim.stop", json!({})).unwrap();
    let s = wait_snapshot(&b, |s| s.timeline().is_none() && s.tick() == 0 && s.seq() > s1.seq());
    assert_eq!(s.predicted().checksum(), s1.predicted().checksum());
    assert!(b.is_alive());
}

#[test]
fn a_proposal_preview_frame_is_a_frame_source() {
    let h = host(false);
    let mut view = local_client(&h, "user");
    let base = {
        let t = h.connector().connect("user", Caps::ALL).unwrap();
        let mut cfg = RemoteConfig::new("");
        cfg.source = "view".into();
        RemoteBridge::<PhysGame>::connect_transport(Box::new(t), cfg).unwrap()
    };
    let guid = view.call("world.query", json!({"name": "body_05"})).unwrap()["entities"][0]["guid"].as_str().unwrap().to_string();
    let id = view.call("proposal.begin", json!({"label": "lift"})).unwrap()["id"].as_str().unwrap().to_string();
    view.call("proposal.apply", json!({"id": id, "ops": [{"op": "patch", "entity": guid, "component": BODY, "path": "pos", "value": [2, 30]}]})).unwrap();
    let t = h.connector().connect("user", Caps::ALL).unwrap();
    let mut cfg = RemoteConfig::new("");
    cfg.source = format!("proposal:{id}");
    let prev = RemoteBridge::<PhysGame>::connect_transport(Box::new(t), cfg).expect("preview bridge");
    let staged = wait_snapshot(&prev, |_| true);
    let plain = wait_snapshot(&base, |_| true);
    assert_ne!(staged.predicted().checksum(), plain.predicted().checksum(), "the staged scene differs from the document's");
    // The document is untouched.
    let s = view.call("scene.save", J::Null).unwrap();
    assert_eq!(s["dirty"], false);
    // An unknown proposal is an error at subscribe time? No: it just never sends a frame.
    let t = h.connector().connect("user", Caps::ALL).unwrap();
    let mut cfg = RemoteConfig::new("");
    cfg.source = "proposal:p99".into();
    cfg.connect_timeout = Duration::from_secs(5);
    let none = RemoteBridge::<PhysGame>::connect_transport(Box::new(t), cfg).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert!(none.snapshot().is_none());
}

#[test]
fn a_panic_in_the_host_thread_is_caught_and_reported() {
    let h = host(true);
    let url = h.url().unwrap().to_string();
    let mut view = local_client(&h, "user");
    let mut agent = ErpClient::connect(&url, None).unwrap();
    assert!(h.is_running());
    // The request itself answers fine; the host thread panics right after.
    view.call("debug.panic", J::Null).unwrap();
    let why = h.stopped_reason(Duration::from_secs(10)).expect("the host stopped");
    assert!(why.contains("debug.panic"), "{why}");
    // The clients see the host go away instead of hanging.
    let e = view.call("sim.state", J::Null).unwrap_err();
    assert!(e.to_string().contains("host"), "{e}");
    assert!(agent.call("sim.state", J::Null).is_err());
    assert!(h.connector().connect("user", Caps::ALL).is_err(), "no new connections");
}

#[test]
fn debug_hooks_are_off_by_default() {
    let mut cfg = ServerConfig::new(Auth::DevNoAuth);
    cfg.listen = false;
    let h = spawn_phys_host(demo_text(), None, cfg).unwrap();
    let mut c = local_client(&h, "user");
    assert_eq!(c.call_err("debug.panic", J::Null).kind(), Some("disabled"));
    assert!(h.is_running());
}
