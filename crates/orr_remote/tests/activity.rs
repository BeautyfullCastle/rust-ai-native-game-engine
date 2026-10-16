//! The activity log over a real socket: what every request did, in one line
//! each, with old values, diffs and verification reports; the ring bound,
//! `activity.list`, `watch.activity`, and the in-process API.

mod common;

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use common::*;
use orr_edit::PlayController;
use orr_remote::{Auth, ErpClient, ErpServer, ErpTarget, ServerConfig};
use orr_sample::physics_game::PhysGame;
use serde_json::{json, Value as J};

const BODY: &str = "orr_physics::Body";

fn list(c: &mut ErpClient, params: J) -> Vec<J> {
    c.call("activity.list", params).unwrap()["entries"].as_array().unwrap().clone()
}

fn find<'a>(entries: &'a [J], method: &str) -> &'a J {
    entries.iter().find(|e| e["method"] == method).unwrap_or_else(|| panic!("no {method} entry in {entries:#?}"))
}

#[test]
fn every_kind_of_request_is_recorded_with_a_summary() {
    let host = TestHost::dev();
    let mut c = ErpClient::connect(&host.url, None).unwrap();
    let body = guid_of(&mut c, "body_05");
    let old = c.call("world.get", json!({"entity": body, "component": BODY, "path": "pos"})).unwrap()["value"].clone();

    c.call("world.patch", json!({"entity": body, "component": BODY, "path": "pos", "value": [6, 18]})).unwrap();
    let id = c.call("proposal.begin", json!({"label": "lift hero"})).unwrap()["id"].as_str().unwrap().to_string();
    c.call("proposal.apply", json!({"id": id, "ops": [{"op": "patch", "entity": body, "component": BODY, "path": "pos", "value": [0, 12]}]})).unwrap();
    let verified = c.call("proposal.verify", json!({"id": id, "inputs": {"kind": "bot", "ticks": 60}, "sample_every": 7, "checks": ["lost_bodies.max == 0"]})).unwrap();
    let accepted = c.call("proposal.accept", json!({"id": id})).unwrap();
    c.call("sim.start", json!({})).unwrap();
    c.call("sim.step", json!({"n": 60})).unwrap();
    let total = c.call("world.query", json!({})).unwrap()["total"].as_u64().unwrap();
    let err = c.call_err("world.patch", json!({"entity": "e_deadbeef", "component": BODY, "path": "pos", "value": [1, 1]}));

    // Reads are left out unless asked for.
    let plain = list(&mut c, json!({}));
    assert!(plain.iter().all(|e| e["read"] == false), "{plain:#?}");
    assert!(!plain.iter().any(|e| e["method"] == "world.query"));
    let all = list(&mut c, json!({"include_reads": true}));
    let seqs: Vec<u64> = all.iter().map(|e| e["seq"].as_u64().unwrap()).collect();
    assert!(seqs.windows(2).all(|w| w[0] < w[1]), "seq counts up: {seqs:?}");
    assert_eq!(seqs[0], 1);
    assert!(all.iter().all(|e| e["client"] != J::Null && e["at_ms"].is_u64()));

    // Connect is the first entry.
    assert_eq!(all[0]["method"], "session.connect");
    assert_eq!(all[0]["client"], "dev");
    assert_eq!(all[0]["kind"], "session");

    // The edit: summary, entity, old and new value (the old one read before it ran).
    let patch = &plain.iter().find(|e| e["method"] == "world.patch" && e["ok"] == true).unwrap().clone();
    assert_eq!(patch["summary"], format!("world.patch {body} {BODY}.pos = [6, 18]"));
    assert_eq!(patch["kind"], "edit");
    assert_eq!(patch["entities"], json!([body]));
    assert_eq!(patch["change"]["old"], old);
    assert_eq!(patch["change"]["new"], json!([6, 18]));
    assert_eq!(patch["change"]["component"], BODY);
    assert_eq!(patch["change"]["path"], "pos");

    // Proposal lifecycle.
    let begin = find(&plain, "proposal.begin");
    assert_eq!(begin["summary"], format!("proposal.begin {id} \"lift hero\""));
    assert_eq!(begin["proposal"], id);
    assert!(find(&plain, "proposal.apply")["summary"].as_str().unwrap().starts_with(&format!("proposal.apply {id} +1 op")));
    let verify = find(&plain, "proposal.verify");
    assert_eq!(verify["kind"], "verify");
    assert_eq!(verify["summary"], format!("proposal.verify {id}: 1/1 checks passed (60 ticks)"));
    assert_eq!(verify["verify"]["passed"], true);
    assert_eq!(verified["metric_sampling"], json!({
        "requested_interval": 7, "sample_count": 10, "scope": "sampled_tick_boundaries", "every_tick_boundary_observed": false
    }));
    assert_eq!(verify["verify"]["metric_sampling"], verified["metric_sampling"]);
    assert_eq!(verify["verify"]["checks"]["results"][0]["check"], "lost_bodies.max == 0");
    assert!(verify["verify"]["metrics"].as_array().unwrap().iter().any(|m| m["name"] == "lost_bodies"));
    let accept = find(&plain, "proposal.accept");
    assert_eq!(accept["summary"], format!("proposal.accept {id} \u{2192} history #{}", accepted["history_id"]));
    // The diff was captured before the proposal was gone, with the entity it touched.
    assert!(accept["diff"]["text"].as_str().unwrap().contains("pos: [0, 12]"), "{accept}");
    assert_eq!(accept["entities"], json!([body]));

    // Sim and read.
    let step = find(&plain, "sim.step");
    assert_eq!(step["summary"], "sim.step 60 \u{2192} tick 60");
    assert_eq!(step["kind"], "sim");
    let q = all.iter().rev().find(|e| e["method"] == "world.query").unwrap();
    assert_eq!(q["read"], true);
    assert_eq!(q["kind"], "read");
    assert_eq!(q["summary"], format!("world.query ({total} entities)"));

    // The error.
    let bad = plain.iter().rev().find(|e| e["method"] == "world.patch").unwrap();
    assert_eq!(bad["ok"], false);
    assert_eq!(bad["error"], err.message.as_str());
    assert!(bad["summary"].as_str().unwrap().starts_with("world.patch e_deadbeef"));
    assert!(bad.get("change").is_none());
}

#[test]
fn a_failed_verify_records_the_failing_check_and_reason() {
    let host = TestHost::dev();
    let mut c = ErpClient::connect(&host.url, None).unwrap();
    let body = guid_of(&mut c, "body_05");
    let id = c.call("proposal.begin", json!({"label": "sink"})).unwrap()["id"].as_str().unwrap().to_string();
    c.call("proposal.apply", json!({"id": id, "ops": [{"op": "patch", "entity": body, "component": BODY, "path": "pos", "value": [0, -80]}]})).unwrap();
    c.call("proposal.verify", json!({"id": id, "inputs": {"kind": "bot", "ticks": 60}, "checks": ["lost_bodies.max == 0"]})).unwrap();
    c.call("verify.self", json!({"inputs": {"kind": "idle", "ticks": 30}})).unwrap();
    let entries = list(&mut c, json!({}));
    let v = find(&entries, "proposal.verify");
    assert_eq!(v["ok"], true, "the request worked; the check did not hold");
    assert_eq!(v["verify"]["passed"], false);
    assert!(v["summary"].as_str().unwrap().contains("FAILED"), "{}", v["summary"]);
    let r = &v["verify"]["checks"]["results"][0];
    assert_eq!(r["passed"], false);
    assert!(r["reason"].as_str().is_some_and(|s| !s.is_empty()));
    let own = find(&entries, "verify.self");
    assert_eq!(own["kind"], "verify");
    assert!(own["summary"].as_str().unwrap().starts_with("verify.self: no checks"), "{}", own["summary"]);
}

#[test]
fn since_and_limit_and_the_ring_bound() {
    let mut cfg = ServerConfig::new(Auth::DevNoAuth);
    cfg.activity_capacity = 8;
    let host = TestHost::start(cfg);
    let mut c = ErpClient::connect(&host.url, None).unwrap();
    for _ in 0..20 {
        c.call("world.query", json!({"limit": 1})).unwrap();
    }
    let r = c.call("activity.list", json!({"include_reads": true, "limit": 100})).unwrap();
    let entries = r["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 8, "the ring keeps the newest 8 only");
    let last = r["last_seq"].as_u64().unwrap();
    assert_eq!(entries.last().unwrap()["seq"], last);
    assert_eq!(entries[0]["seq"].as_u64().unwrap(), last - 7, "the oldest entries were dropped");
    assert!(last > 20, "connect + 20 queries, counted past the bound");

    let after = c.call("activity.list", json!({"include_reads": true, "since": last - 3})).unwrap();
    assert_eq!(after["entries"].as_array().unwrap().len(), 3);
    assert_eq!(after["truncated"], false);
    let capped = c.call("activity.list", json!({"include_reads": true, "limit": 2})).unwrap();
    assert_eq!(capped["entries"].as_array().unwrap().len(), 2);
    assert_eq!(capped["truncated"], true);
    assert_eq!(capped["entries"][1]["seq"], last, "the newest ones are kept");
    // activity.list is not recorded itself.
    let again = c.call("activity.list", json!({"include_reads": true, "since": last})).unwrap();
    assert!(again["entries"].as_array().unwrap().is_empty());
    let d = c.call("rpc.discover", J::Null).unwrap();
    assert!(d["methods"].as_array().unwrap().iter().any(|m| m["name"] == "activity.list"));
}

#[test]
fn watch_activity_pushes_new_entries() {
    let host = TestHost::dev();
    let mut watcher = ErpClient::connect(&host.url, None).unwrap();
    let r = watcher.call("watch.subscribe", json!({"topics": ["activity"]})).unwrap();
    assert_eq!(r["topics"], json!(["activity"]));
    let mut agent = ErpClient::connect(&host.url, None).unwrap();
    let body = guid_of(&mut agent, "body_05");
    agent.call("world.patch", json!({"entity": body, "component": BODY, "path": "pos.x", "value": "1.5"})).unwrap();
    let mut seen: Vec<J> = Vec::new();
    while let Some(n) = watcher.wait_notification("watch.activity", Duration::from_millis(600)).unwrap() {
        seen.extend(n["params"]["entries"].as_array().unwrap().iter().cloned());
        if seen.iter().any(|e| e["method"] == "world.patch") {
            break;
        }
    }
    let patch = find(&seen, "world.patch");
    assert_eq!(patch["client"], "dev");
    assert!(patch["change"]["old"].is_string() || patch["change"]["old"].is_number());
    assert!(seen.iter().all(|e| e["read"] == false), "reads are not pushed by default: {seen:#?}");
    assert!(seen.iter().any(|e| e["method"] == "session.connect"), "the second client's connect was pushed");

    watcher.call("watch.unsubscribe", json!({"topics": ["activity"]})).unwrap();
    agent.call("world.patch", json!({"entity": body, "component": BODY, "path": "pos.x", "value": "2"})).unwrap();
    watcher.notifications.clear();
    assert!(watcher.wait_notification("watch.activity", Duration::from_millis(300)).unwrap().is_none());
}

#[test]
fn connect_disconnect_and_a_refused_token_are_recorded() {
    let host = TestHost::standard();
    let mut c = host.client("tok-all");
    {
        let mut other = host.client("tok-read");
        other.call("world.query", json!({"limit": 1})).unwrap();
    }
    assert!(ErpClient::connect(&host.url, Some("wrong-token")).is_err());
    // The disconnect reaches the host thread a moment later.
    let mut entries = Vec::new();
    for _ in 0..50 {
        entries = list(&mut c, json!({"include_reads": true}));
        if entries.iter().any(|e| e["method"] == "session.disconnect") {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let conn: Vec<&J> = entries.iter().filter(|e| e["method"] == "session.connect").collect();
    assert_eq!(conn.iter().map(|e| e["client"].as_str().unwrap()).collect::<Vec<_>>(), ["claude", "reader"]);
    assert!(conn[1]["summary"].as_str().unwrap().contains("read"), "{}", conn[1]["summary"]);
    let gone = find(&entries, "session.disconnect");
    assert_eq!(gone["client"], "reader");
    assert!(gone["summary"].as_str().unwrap().contains("1 requests"));
    let refused = find(&entries, "session.auth_failed");
    assert_eq!(refused["ok"], false);
}

/// The in-process API an embedding host (the editor) uses: `activity_since`, `clients`.
#[test]
fn in_process_api_and_old_values() {
    let mut server = ErpServer::start(ServerConfig::new(Auth::DevNoAuth)).unwrap();
    let url = server.url();
    let mut doc = demo_doc();
    let mut play: Option<PlayController<PhysGame>> = None;
    let (done_tx, done_rx) = mpsc::channel();
    let agent = thread::spawn(move || {
        let mut c = ErpClient::connect(&url, None).unwrap();
        let body = guid_of(&mut c, "body_05");
        c.call("world.patch", json!({"entity": body, "component": BODY, "path": "pos.y", "value": "20"})).unwrap();
        c.call("world.patch", json!({"entity": body, "component": BODY, "path": "pos.y", "value": "25"})).unwrap();
        done_tx.send(()).unwrap();
        // Stay connected until the host has looked.
        thread::sleep(Duration::from_millis(400));
        body
    });
    let mut rounds = 0;
    while done_rx.try_recv().is_err() {
        rounds += 1;
        assert!(rounds < 20_000, "the agent did not finish");
        server.poll(&mut ErpTarget { doc: &mut doc, play: &mut play });
        thread::sleep(Duration::from_millis(1));
    }
    server.poll(&mut ErpTarget { doc: &mut doc, play: &mut play });

    let clients = server.clients();
    assert_eq!(clients.len(), 1);
    assert_eq!((clients[0].name.as_str(), clients[0].caps), ("dev", orr_remote::Caps::ALL));
    assert!(clients[0].requests >= 3);

    let all = server.activity_since(0);
    assert_eq!(all.first().unwrap().method, "session.connect");
    let patches: Vec<_> = all.iter().filter(|e| e.method == "world.patch").collect();
    assert_eq!(patches.len(), 2);
    let (first, second) = (patches[0].change.as_ref().unwrap(), patches[1].change.as_ref().unwrap());
    // The second patch saw the first one's result as its old value.
    assert_eq!(second.old, first.new);
    assert_ne!(first.old, first.new);
    assert!(all.windows(2).all(|w| w[0].at_ms <= w[1].at_ms), "the clock only goes forward");
    // Reads (world.query from guid_of) are in the raw log, flagged.
    assert!(all.iter().any(|e| e.method == "world.query" && e.read));
    let last = server.activity_last_seq();
    assert_eq!(all.last().unwrap().seq, last);
    assert!(server.activity_since(last).is_empty());
    assert_eq!(server.activity_since(last - 1).len(), 1);
    agent.join().unwrap();
}

#[test]
fn activity_list_names_the_connected_clients() {
    let host = TestHost::dev();
    let mut a = ErpClient::connect(&host.url, None).unwrap();
    let r = a.call("activity.list", json!({"limit": 1})).unwrap();
    let clients = r["clients"].as_array().unwrap();
    assert_eq!(clients.len(), 1, "{r}");
    assert_eq!(clients[0]["client"], "dev");
    assert!(clients[0]["capabilities"].as_array().unwrap().iter().any(|c| c == "read"));
    let mut b = ErpClient::connect(&host.url, None).unwrap();
    b.call("sim.state", J::Null).unwrap();
    assert_eq!(a.call("activity.list", json!({"limit": 1})).unwrap()["clients"].as_array().unwrap().len(), 2);
    drop(b);
}
