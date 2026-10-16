//! Agent proposals over a real socket: propose, look at the diff, verify by
//! replay, accept or reject; capabilities, conflicts, recorded inputs.

mod common;

use std::time::Duration;

use common::*;
use orr_remote::{ErpClient, CONFLICT, INVALID_PARAMS, INVALID_STATE, LIMIT_EXCEEDED, NOT_FOUND, PERMISSION_DENIED};
use serde_json::{json, Value as J};

const BODY: &str = "orr_physics::Body";

fn scene_text(c: &mut ErpClient) -> String {
    c.call("scene.save", json!({})).unwrap()["text"].as_str().unwrap().to_string()
}

fn begin(c: &mut ErpClient, label: &str) -> String {
    c.call("proposal.begin", json!({"label": label})).unwrap()["id"].as_str().unwrap().to_string()
}

fn check_passed(report: &J, i: usize) -> bool {
    report["checks"]["results"][i]["passed"].as_bool().unwrap()
}

fn metric<'a>(report: &'a J, name: &str) -> &'a J {
    report["metrics"].as_array().unwrap().iter().find(|m| m["name"] == name).unwrap_or_else(|| panic!("metric {name} in {report}"))
}

#[test]
fn propose_diff_verify_accept_and_undo_over_a_socket() {
    let host = TestHost::standard();
    let mut watcher = host.client("tok-other");
    let subscribed = watcher.call("watch.subscribe", json!({"topics": ["proposals"]})).unwrap();
    assert_eq!(subscribed["topics"], json!(["proposals"]));
    let mut c = host.client("tok-all");
    let before = scene_text(&mut c);
    let body = guid_of(&mut c, "body_05");

    let id = begin(&mut c, "lift body_05");
    assert_eq!(id, "p1");
    let applied = c
        .call(
            "proposal.apply",
            json!({"id": id, "ops": [
                {"op": "rename", "entity": body, "name": "hero"},
                {"op": "patch", "entity": body, "component": BODY, "path": "pos", "value": [0, 12.5]},
            ]}),
        )
        .unwrap();
    assert_eq!((applied["applied"].as_u64(), applied["changed"].as_u64(), applied["op_count"].as_u64()), (Some(2), Some(2), Some(2)));

    // The scene itself has not changed; the proposal shows what would.
    assert_eq!(scene_text(&mut c), before);
    let got = c.call("proposal.get", json!({"id": id})).unwrap();
    assert_eq!(got["origin"], "agent:claude");
    assert_eq!(got["op_count"], 2);
    assert_eq!(got["accepts_cleanly"], true);
    let diff = got["diff"].as_str().unwrap();
    assert!(diff.starts_with("--- base\n+++ proposal\n"), "{diff}");
    assert!(diff.contains("+    name: hero") && diff.contains("-    name: body_05"), "{diff}");
    assert_eq!(got["summary"]["entities_renamed"][0]["new"], "hero");
    assert_eq!(got["summary"]["fields_changed"][0]["component"], BODY);
    assert_eq!(got["summary"]["fields_changed"][0]["new"], json!([0, 12.5]));
    assert_eq!(got["ops"][1]["op"], "patch");
    let listed = c.call("proposal.list", J::Null).unwrap();
    assert_eq!(listed["proposals"][0]["id"], "p1");
    assert_eq!(listed["proposals"][0]["stale"], false);

    let pv = c.call("proposal.preview", json!({"id": id, "entity": body})).unwrap();
    assert_eq!(pv["entity"]["name"], "hero");
    assert_eq!(pv["components"][BODY]["pos"], json!([0, 12.5]));
    let pq = c.call("proposal.preview", json!({"id": id, "name": "hero"})).unwrap();
    assert_eq!(pq["total"], 1);
    assert!(pq["entities"][0]["values"][BODY].is_object(), "the list includes values by default");
    let live = c.call("world.get", json!({"entity": body, "component": BODY, "path": "pos"})).unwrap();
    assert_ne!(live["value"], json!([0, 12.5]));

    // Verify on the scripted bot, with acceptance checks.
    let report = c
        .call(
            "proposal.verify",
            json!({"id": id, "inputs": {"kind": "bot", "ticks": 120, "seed": 3}, "sample_every": 30,
                   "checks": ["lost_bodies.max == 0", "dynamic_bodies.start == 40", "mean_height >= 0"]}),
        )
        .unwrap();
    assert_eq!(report["checks"]["passed"], true, "{report}");
    assert_eq!(report["passed"], true);
    assert!((0..3).all(|i| check_passed(&report, i)));
    assert_eq!((report["ticks"].as_u64(), report["end_tick"].as_u64()), (Some(120), Some(120)));
    assert_eq!(report["identical"], false, "an edit makes the initial frames differ");
    assert_eq!(report["first_divergence"], 0);
    assert_eq!(report["inputs"], json!({"kind": "bot", "ticks": 120, "players": 2, "seed": 3}));
    for k in ["base_start", "candidate_start", "base_final", "candidate_final"] {
        let text = report["checksums"][k].as_str().unwrap();
        assert!(text.starts_with("0x") && text.len() == 18, "{k}: {text}");
    }
    assert_ne!(report["checksums"]["base_final"], report["checksums"]["candidate_final"]);
    assert_eq!(report["samples"].as_array().unwrap().len(), 5);
    assert_eq!(report["metric_sampling"], json!({
        "requested_interval": 30, "sample_count": 5, "scope": "sampled_tick_boundaries", "every_tick_boundary_observed": false
    }));
    let lost = metric(&report, "lost_bodies");
    assert_eq!(lost["kind"], "int");
    assert_eq!(lost["candidate"]["max"], 0);
    // Exact fixed-point numbers, never rounded through a float.
    let maxh = metric(&report, "max_height");
    assert_eq!(maxh["kind"], "fixed");
    assert!(maxh["candidate"]["start"].as_f64().unwrap() >= 12.5);
    assert!(metric(&report, "entities")["base"]["start"].is_number(), "ReflectMetrics are there too");
    assert!(report["metrics"][0]["base"].get("series").is_none(), "series only on request");
    let again = c
        .call("proposal.verify", json!({"id": id, "inputs": {"kind": "bot", "ticks": 120, "seed": 3}, "sample_every": 30, "series": true}))
        .unwrap();
    assert_eq!(metric(&again, "max_height")["candidate"]["series"].as_array().unwrap().len(), 5);
    assert_eq!(again["checksums"], report["checksums"], "verification is deterministic");
    assert_eq!(scene_text(&mut c), before, "verifying changes nothing");

    // Accept: one history entry with the agent origin.
    let acc = c.call("proposal.accept_verified", json!({"id": id, "verified_state": report["verified_state"]})).unwrap();
    assert!(acc["history_id"].is_u64());
    assert_eq!(acc["applied"], 2);
    let h = c.call("history.list", J::Null).unwrap();
    let entries = h["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "{h}");
    assert_eq!(entries[0]["origin"], "agent:claude");
    assert_eq!(entries[0]["label"], "lift body_05");
    assert_eq!(entries[0]["op_count"], 2);
    assert_eq!(entries[0]["id"], acc["history_id"]);
    assert_eq!(c.call("world.get", json!({"entity": body, "component": BODY, "path": "pos"})).unwrap()["value"], json!([0, 12.5]));
    assert_eq!(checksum(&acc), report_checksum_of_scene(&mut c));
    assert!(c.call("proposal.list", J::Null).unwrap()["proposals"].as_array().unwrap().is_empty());
    assert_eq!(c.call_err("proposal.get", json!({"id": id})).code, NOT_FOUND);

    // Undo reverts exactly.
    c.call("history.undo", J::Null).unwrap();
    assert_eq!(scene_text(&mut c), before);

    // The other client saw it all happen.
    let mut events = Vec::new();
    while let Some(n) = watcher.wait_notification("watch.proposals", Duration::from_millis(600)).unwrap() {
        for e in n["params"]["events"].as_array().unwrap() {
            events.push((e["event"].as_str().unwrap().to_string(), e["id"].as_str().unwrap().to_string(), e["origin"].as_str().unwrap().to_string()));
        }
    }
    let kinds: Vec<&str> = events.iter().map(|e| e.0.as_str()).collect();
    assert!(kinds.contains(&"new") && kinds.contains(&"accepted"), "{events:?}");
    assert!(events.iter().all(|e| e.1 == "p1" && e.2 == "agent:claude"), "{events:?}");
    let accepted = events.iter().position(|e| e.0 == "accepted").unwrap();
    assert!(events.iter().position(|e| e.0 == "new").unwrap() < accepted);
}

fn report_checksum_of_scene(c: &mut ErpClient) -> u64 {
    checksum(&c.call("scene.save", json!({})).unwrap())
}

#[test]
fn a_failing_check_shows_the_reason_and_the_agent_rejects() {
    let host = TestHost::standard();
    let mut c = host.client("tok-all");
    let before = scene_text(&mut c);
    let body = guid_of(&mut c, "body_05");
    let id = begin(&mut c, "sink body_05");
    c.call("proposal.apply", json!({"id": id, "ops": [{"op": "patch", "entity": body, "component": BODY, "path": "pos", "value": [0, -30]}]})).unwrap();
    let report = c
        .call("proposal.verify", json!({"id": id, "inputs": {"kind": "bot", "ticks": 60}, "checks": ["lost_bodies.max == 0", "dynamic_bodies.start == 40"]}))
        .unwrap();
    assert_eq!(report["passed"], false);
    assert!(!check_passed(&report, 0), "{report}");
    assert!(check_passed(&report, 1));
    let reason = report["checks"]["results"][0]["reason"].as_str().unwrap();
    assert!(reason.contains("lost_bodies.max is 1"), "{reason}");
    assert_eq!(report["checks"]["results"][0]["check"], "lost_bodies.max == 0");
    // The baseline has no lost body: the regression is the proposal's.
    assert_eq!(metric(&report, "lost_bodies")["base"]["max"], 0);
    assert_eq!(metric(&report, "lost_bodies")["delta"], 1);

    c.call("proposal.reject", json!({"id": id})).unwrap();
    assert!(c.call("proposal.list", J::Null).unwrap()["proposals"].as_array().unwrap().is_empty());
    assert!(c.call("history.list", J::Null).unwrap()["entries"].as_array().unwrap().is_empty());
    assert_eq!(scene_text(&mut c), before);
    assert_eq!(c.call_err("proposal.reject", json!({"id": id})).code, NOT_FOUND);
}

#[test]
fn accepting_needs_the_approve_capability() {
    let host = TestHost::standard();
    let mut editor = host.client("tok-edit");
    let mut reader = host.client("tok-read");
    let mut boss = host.client("tok-all");
    let body = guid_of(&mut boss, "body_02");

    // Without scene_edit: no proposals; reading them is fine.
    let e = reader.call_err("proposal.begin", json!({}));
    assert_eq!(e.code, PERMISSION_DENIED);
    assert_eq!(e.data.as_ref().unwrap()["required"], "scene_edit");
    assert_eq!(reader.call_err("proposal.reject", json!({"id": "p1"})).code, PERMISSION_DENIED);

    // With scene_edit an agent proposes and verifies, but cannot accept.
    let id = begin(&mut editor, "nudge");
    editor.call("proposal.apply", json!({"id": id, "ops": [{"op": "patch", "entity": body, "component": BODY, "path": "vel", "value": [1, 0]}]})).unwrap();
    let r = editor.call("proposal.verify", json!({"id": id, "inputs": {"kind": "idle", "ticks": 30}})).unwrap();
    assert_eq!(r["checks"], J::Null);
    assert_eq!(r["passed"], J::Null);
    let denied = editor.call_err("proposal.accept", json!({"id": id}));
    assert_eq!(denied.code, PERMISSION_DENIED);
    assert_eq!(denied.data.as_ref().unwrap()["required"], "approve");
    let denied = editor.call_err("proposal.accept_verified", json!({"id": id, "verified_state": r["verified_state"]}));
    assert_eq!(denied.code, PERMISSION_DENIED);
    assert_eq!(denied.data.as_ref().unwrap()["required"], "approve");
    assert!(editor.call("history.list", J::Null).unwrap()["entries"].as_array().unwrap().is_empty());

    // The reader sees it (read) and cannot verify-by-accident into the scene either.
    assert_eq!(reader.call("proposal.list", J::Null).unwrap()["proposals"][0]["origin"], "agent:editor");
    assert_eq!(reader.call("proposal.get", json!({"id": id})).unwrap()["op_count"], 1);
    assert_eq!(reader.call("proposal.verify", json!({"id": id, "inputs": {"kind": "idle", "ticks": 5}})).unwrap()["ticks"], 5);
    assert_eq!(reader.call_err("proposal.accept", json!({"id": id})).code, PERMISSION_DENIED);

    // A holder of `approve` accepts; the entry keeps the proposer's origin.
    boss.call("proposal.accept", json!({"id": id})).unwrap();
    let h = boss.call("history.list", J::Null).unwrap();
    assert_eq!(h["entries"][0]["origin"], "agent:editor");
    assert_eq!(h["entries"].as_array().unwrap().len(), 1);

    // The capability text parses and is advertised.
    let d = boss.call("rpc.discover", J::Null).unwrap();
    assert!(d["capabilities"].as_array().unwrap().contains(&json!("approve")));
    let m = d["methods"].as_array().unwrap().iter().find(|m| m["name"] == "proposal.accept").unwrap().clone();
    assert_eq!(m["capability"], "approve");
    let de = editor.call("rpc.discover", J::Null).unwrap();
    let m = de["methods"].as_array().unwrap().iter().find(|m| m["name"] == "proposal.accept").unwrap().clone();
    assert_eq!(m["allowed"], false);
    assert_eq!(de["you"]["capabilities"], json!(["read", "scene_edit"]));
}

#[test]
fn a_conflicting_accept_fails_and_changes_nothing() {
    let host = TestHost::standard();
    let mut agent = host.client("tok-all");
    let mut person = host.client("tok-other");
    let body = guid_of(&mut agent, "body_07");
    let id = begin(&mut agent, "move body_07");
    agent.call("proposal.apply", json!({"id": id, "ops": [{"op": "patch", "entity": body, "component": BODY, "path": "pos", "value": [1, 9]}]})).unwrap();

    // The person deletes the entity meanwhile.
    person.call("world.despawn", json!({"entity": body})).unwrap();
    let after_despawn = scene_text(&mut agent);
    let hist = agent.call("history.list", J::Null).unwrap();
    assert_eq!(hist["entries"].as_array().unwrap().len(), 1);
    assert_eq!(agent.call("proposal.list", J::Null).unwrap()["proposals"][0]["stale"], true);
    let got = agent.call("proposal.get", json!({"id": id})).unwrap();
    assert_eq!(got["accepts_cleanly"], false);
    assert!(got["accept_error"].as_str().unwrap().contains("conflicts"), "{got}");

    let e = agent.call_err("proposal.accept", json!({"id": id}));
    assert_eq!(e.code, CONFLICT);
    assert_eq!(e.kind(), Some("proposal_conflict"));
    assert!(e.message.contains("op 0"), "{}", e.message);
    assert_eq!(scene_text(&mut agent), after_despawn, "nothing changed");
    assert_eq!(agent.call("history.list", J::Null).unwrap(), hist, "no history entry either");
    // The proposal is still there, to be fixed or rejected.
    assert_eq!(agent.call("proposal.list", J::Null).unwrap()["proposals"].as_array().unwrap().len(), 1);
    agent.call("proposal.reject", json!({"id": id})).unwrap();
}

#[test]
fn last_play_and_replay_inputs() {
    let host = TestHost::standard();
    let mut c = host.client("tok-all");
    let body = guid_of(&mut c, "body_05");
    let id = begin(&mut c, "small move");
    c.call("proposal.apply", json!({"id": id, "ops": [{"op": "patch", "entity": body, "component": BODY, "path": "pos", "value": [0, 20]}]})).unwrap();

    // Nothing was played yet.
    let e = c.call_err("proposal.verify", json!({"id": id, "inputs": {"kind": "last_play"}}));
    assert_eq!(e.code, INVALID_STATE);
    assert_eq!(e.kind(), Some("no_last_play"));

    // Play a little (with a debug-free run) and stop.
    c.call("sim.start", json!({"player_count": 2})).unwrap();
    c.call("sim.input", json!({"player": 0, "input": "000000000000000000000000000000000000000000000000"})).unwrap();
    c.call("sim.step", json!({"n": 90})).unwrap();
    let stopped = c.call("sim.stop", json!({"include_replay": true})).unwrap();
    assert_eq!(stopped["tick"], 90);

    let base = c.call("verify.self", json!({"inputs": {"kind": "last_play"}, "checks": ["recording_matches"]})).unwrap();
    assert_eq!(base["passed"], true, "the scene reproduces its own recording: {base}");
    assert_eq!(base["identical"], true);
    assert_eq!(base["ticks"], 90);
    assert_eq!(base["recording"]["mismatches"], 0);
    assert!(base["recording"]["checked"].as_u64().unwrap() > 0);
    assert_eq!(base["inputs"]["kind"], "last_play");

    let r = c.call("proposal.verify", json!({"id": id, "inputs": {"kind": "last_play"}, "checks": ["lost_bodies.max == 0"]})).unwrap();
    assert_eq!(r["identical"], false);
    assert_eq!(r["ticks"], 90);
    assert_eq!(r["passed"], true);
    // The same through an explicit replay, and cut short.
    let r2 = c.call("proposal.verify", json!({"id": id, "inputs": {"kind": "replay", "base64": stopped["replay"]}})).unwrap();
    assert_eq!(r2["checksums"], r["checksums"]);
    let short = c.call("proposal.verify", json!({"id": id, "inputs": {"kind": "last_play"}, "ticks": 30})).unwrap();
    assert_eq!(short["ticks"], 30);

    // Bad recordings are errors, not panics.
    assert_eq!(c.call_err("proposal.verify", json!({"id": id, "inputs": {"kind": "replay", "base64": "!!!"}})).code, INVALID_PARAMS);
    assert!(c.call_err("proposal.verify", json!({"id": id, "inputs": {"kind": "replay", "base64": "AAAA"}})).code != 0);
    assert!(host.is_running());
}

#[test]
fn verify_self_is_a_baseline_and_inputs_are_validated() {
    let host = TestHost::standard();
    let mut c = host.client("tok-read");
    let base = c.call("verify.self", json!({"inputs": {"kind": "bot", "ticks": 90, "seed": 9}, "checks": ["no_divergence", "dynamic_bodies.start == 40"]})).unwrap();
    assert_eq!(base["identical"], true);
    assert_eq!(base["passed"], true);
    let m = metric(&base, "kinetic_energy");
    assert_eq!(m["base"], m["candidate"]);
    // The same seed gives the same run; another seed another one.
    let again = c.call("verify.self", json!({"inputs": {"kind": "bot", "ticks": 90, "seed": 9}})).unwrap();
    assert_eq!(again["checksums"], base["checksums"]);
    let other = c.call("verify.self", json!({"inputs": {"kind": "bot", "ticks": 90, "seed": 10}})).unwrap();
    assert_ne!(other["checksums"]["base_final"], base["checksums"]["base_final"]);
    let idle = c.call("verify.self", json!({"inputs": {"kind": "idle", "ticks": 90}})).unwrap();
    assert_ne!(idle["checksums"]["base_final"], base["checksums"]["base_final"]);

    for (params, code) in [
        (json!({"inputs": {"kind": "bot", "ticks": 0}}), INVALID_PARAMS),
        (json!({"inputs": {"kind": "bot"}}), INVALID_PARAMS),
        (json!({"inputs": {"kind": "bot", "ticks": 100000}}), LIMIT_EXCEEDED),
        (json!({"inputs": {"kind": "idle", "ticks": 10, "players": 99}}), INVALID_PARAMS),
        (json!({"inputs": {"kind": "teleport", "ticks": 10}}), INVALID_PARAMS),
        (json!({"inputs": "bot"}), INVALID_PARAMS),
        (json!({}), INVALID_PARAMS),
        (json!({"inputs": {"kind": "idle", "ticks": 10}, "checks": "no_divergence"}), INVALID_PARAMS),
        (json!({"inputs": {"kind": "idle", "ticks": 10}, "checks": ["lost_bodies.max"]}), orr_remote::INVALID_VALUE),
    ] {
        assert_eq!(c.call_err("verify.self", params.clone()).code, code, "{params}");
    }
}

#[test]
fn discover_describes_the_engine_and_the_metrics() {
    let host = TestHost::standard();
    let mut c = host.client("tok-read");
    let d = c.call("rpc.discover", J::Null).unwrap();
    assert_eq!(d["engine"]["name"], "orrery");
    assert_eq!(d["engine"]["game"], "PhysGame");
    assert!(d["engine"]["version"].as_str().unwrap().contains('.'));
    assert!(d["engine"]["build_id"].as_str().unwrap().starts_with("0x"));
    let names: Vec<&str> = d["engine"]["metrics"].as_array().unwrap().iter().map(|m| m["name"].as_str().unwrap()).collect();
    for n in ["entities", "lost_bodies", "mean_height", "kinetic_energy", "components.orr_physics::Body"] {
        assert!(names.contains(&n), "{n} in {names:?}");
    }
    assert_eq!(d["engine"]["verify"]["bot_available"], true);
    for m in ["proposal.begin", "proposal.apply", "proposal.list", "proposal.get", "proposal.preview", "proposal.verify", "proposal.accept", "proposal.accept_verified", "proposal.reject", "verify.self"] {
        assert!(d["methods"].as_array().unwrap().iter().any(|x| x["name"] == m), "{m}");
    }
    assert!(d["notifications"].as_array().unwrap().contains(&json!("watch.proposals")));
}

#[test]
fn malformed_proposal_requests_are_errors_and_stage_nothing() {
    let host = TestHost::standard();
    let mut c = host.client("tok-all");
    let body = guid_of(&mut c, "body_05");
    let id = begin(&mut c, "fuzz");
    let pos = |v: J| json!({"op": "patch", "entity": body, "component": BODY, "path": "pos", "value": v});
    let bad: Vec<(&str, J)> = vec![
        ("proposal.apply", json!({})),
        ("proposal.apply", json!({"id": id})),
        ("proposal.apply", json!({"id": id, "ops": "patch"})),
        ("proposal.apply", json!({"id": id, "ops": [1]})),
        ("proposal.apply", json!({"id": id, "ops": [{}]})),
        ("proposal.apply", json!({"id": id, "ops": [{"op": "explode"}]})),
        ("proposal.apply", json!({"id": id, "ops": [{"op": "patch"}]})),
        ("proposal.apply", json!({"id": id, "ops": [{"op": "patch", "entity": "nope", "component": BODY, "value": 1}]})),
        ("proposal.apply", json!({"id": id, "ops": [{"op": "patch", "entity": "e_deadbeef", "component": BODY, "path": "pos", "value": [0, 1]}]})),
        ("proposal.apply", json!({"id": id, "ops": [{"op": "patch", "entity": body, "component": "Nope", "value": 1}]})),
        ("proposal.apply", json!({"id": id, "ops": [pos(json!("far")), pos(json!([1, 1]))]})),
        ("proposal.apply", json!({"id": id, "ops": [pos(json!([1, 2, 3]))]})),
        ("proposal.apply", json!({"id": id, "ops": [{"op": "spawn", "components": [1]}]})),
        ("proposal.apply", json!({"id": id, "ops": [{"op": "spawn", "guid": "zzz"}]})),
        ("proposal.apply", json!({"id": id, "ops": [{"op": "rename", "entity": body, "name": 5}]})),
        ("proposal.apply", json!({"id": id, "ops": [{"op": "despawn", "entity": "e_deadbeef"}]})),
        ("proposal.apply", json!({"id": id, "ops": [{"op": "singleton.patch", "name": "Nope", "value": 1}]})),
        ("proposal.apply", json!({"id": "p999", "ops": []})),
        ("proposal.apply", json!({"id": "zzz", "ops": []})),
        ("proposal.apply", json!({"id": [1], "ops": []})),
        ("proposal.apply", json!({"id": -1, "ops": []})),
        ("proposal.get", json!({"id": "p0"})),
        ("proposal.get", J::Null),
        ("proposal.preview", json!({"id": id, "entity": "e_deadbeef"})),
        ("proposal.preview", json!({"id": id, "components": ["Nope"]})),
        ("proposal.preview", json!({"id": id, "entity": 7})),
        ("proposal.accept", json!({"id": "p999"})),
        ("proposal.reject", json!({"id": "p999"})),
        ("proposal.begin", json!({"label": 5})),
        ("proposal.verify", json!({"id": id})),
        ("proposal.verify", json!({"id": "p999", "inputs": {"kind": "idle", "ticks": 3}})),
        ("proposal.verify", json!({"id": id, "inputs": {"kind": "idle", "ticks": 3}, "sample_every": -1})),
        ("proposal.verify", json!({"id": id, "inputs": {"kind": "idle", "ticks": 3}, "checks": [1]})),
    ];
    for (method, params) in &bad {
        let e = c.call_err(method, params.clone());
        assert_ne!(e.code, orr_remote::INTERNAL_ERROR, "{method} {params}: {e}");
        assert!(e.kind().is_some(), "{method} {params}: {e:?}");
    }
    // A call with one bad op stages none of the good ones.
    let e = c.call_err("proposal.apply", json!({"id": id, "ops": [pos(json!([4, 4])), {"op": "despawn", "entity": "e_deadbeef"}]}));
    assert_eq!(e.code, NOT_FOUND);
    let got = c.call("proposal.get", json!({"id": id})).unwrap();
    assert_eq!(got["op_count"], 0);
    assert_eq!(got["diff"], "");
    // A parse error names the op.
    let e = c.call_err("proposal.apply", json!({"id": id, "ops": [pos(json!([4, 4])), {"op": "explode"}]}));
    assert!(e.message.starts_with("ops[1]:"), "{}", e.message);
    let body6 = guid_of(&mut c, "body_06");
    // Numeric ids work too, and spawn/insert/remove/despawn/singleton ops stage.
    let applied = c
        .call(
            "proposal.apply",
            json!({"id": 1, "ops": [
                {"op": "world.spawn", "name": "marker", "components": {}},
                {"op": "singleton.patch", "name": "Scene", "path": "spawn_batch", "value": 3},
                {"op": "remove", "entity": body, "component": "orr_physics::Collider"},
                {"op": "insert", "entity": body, "component": "orr_physics::Collider"},
                {"op": "despawn", "entity": body6},
            ]}),
        )
        .unwrap();
    assert_eq!(applied["spawned"][0]["index"], 0);
    let g = applied["spawned"][0]["guid"].as_str().unwrap().to_string();
    let got = c.call("proposal.get", json!({"id": id})).unwrap();
    assert!(got["summary"]["entities_added"][0]["guid"] == g.as_str(), "{got}");
    assert_eq!(got["summary"]["entities_removed"].as_array().unwrap().len(), 1);
    c.call("proposal.accept", json!({"id": id})).unwrap();
    assert_eq!(c.call("world.get", json!({"entity": g})).unwrap()["entity"]["name"], "marker");
    assert!(host.is_running());
}

#[test]
fn watchers_see_new_changed_and_rejected_proposals_and_the_state_on_subscribe() {
    let host = TestHost::standard();
    let mut agent = host.client("tok-all");
    let body = guid_of(&mut agent, "body_03");
    let early = begin(&mut agent, "early");
    let mut w = host.client("tok-read");
    w.call("watch.subscribe", json!({"topics": ["proposals"]})).unwrap();
    let first = w.wait_notification("watch.proposals", Duration::from_secs(2)).unwrap().unwrap();
    assert_eq!(first["params"]["events"], json!([]));
    assert_eq!(first["params"]["open"][0]["id"], early.as_str());

    let id = begin(&mut agent, "second");
    agent.call("proposal.apply", json!({"id": id, "ops": [{"op": "rename", "entity": body, "name": "x"}]})).unwrap();
    agent.call("proposal.reject", json!({"id": id})).unwrap();
    let mut seen = Vec::new();
    while seen.len() < 3 {
        let n = w.wait_notification("watch.proposals", Duration::from_secs(2)).unwrap().expect("a notification");
        for e in n["params"]["events"].as_array().unwrap() {
            seen.push((e["event"].as_str().unwrap().to_string(), e["id"].as_str().unwrap().to_string(), e["op_count"].as_u64().unwrap()));
        }
    }
    assert_eq!(
        seen,
        vec![("new".to_string(), "p2".to_string(), 0), ("changed".to_string(), "p2".to_string(), 1), ("rejected".to_string(), "p2".to_string(), 1)]
    );
    // After unsubscribing nothing more arrives.
    w.call("watch.unsubscribe", json!({"topics": ["proposals"]})).unwrap();
    w.notifications.clear();
    begin(&mut agent, "third");
    assert!(w.wait_notification("watch.proposals", Duration::from_millis(300)).unwrap().is_none());
}

#[test]
fn accept_is_refused_while_a_play_session_runs() {
    let host = TestHost::standard();
    let mut c = host.client("tok-all");
    let id = begin(&mut c, "empty");
    c.call("sim.start", json!({})).unwrap();
    let e = c.call_err("proposal.accept", json!({"id": id}));
    assert_eq!(e.code, INVALID_STATE);
    assert_eq!(e.kind(), Some("sim_running"));
    // Verifying during play works: it runs on the document, not on the session.
    let r = c.call("proposal.verify", json!({"id": id, "inputs": {"kind": "idle", "ticks": 10}})).unwrap();
    assert_eq!(r["identical"], true);
    let guarded = c.call_err("proposal.accept_verified", json!({"id": id, "verified_state": r["verified_state"]}));
    assert_eq!(guarded.kind(), Some("sim_running"));
    c.call("sim.stop", json!({})).unwrap();
    c.call("proposal.accept", json!({"id": id})).unwrap();
}

#[test]
fn verified_accept_detects_scene_and_proposal_edits_between_requests() {
    let host = TestHost::standard();
    let mut c = host.client("tok-all");
    let mut other = host.client("tok-other");
    let body = guid_of(&mut c, "body_05");
    let id = begin(&mut c, "lift");
    c.call("proposal.apply", json!({"id": id, "ops": [{"op": "patch", "entity": body, "component": BODY, "path": "pos", "value": [0, 12]}]})).unwrap();
    let verify = json!({"id": id, "inputs": {"kind": "idle", "ticks": 1}, "checks": ["lost_bodies.max == 0"]});
    let report = c.call("proposal.verify", verify.clone()).unwrap();
    assert_eq!(report["passed"], true);
    assert_eq!(report["verified_state"]["id"], id);

    // Another client changes only the entity name: the simulation checksum
    // stays the same, but the exact document that was verified has changed.
    other.call("world.rename", json!({"entity": body, "name": "renamed"})).unwrap();
    assert_eq!(c.call("sim.state", J::Null).unwrap()["checksum"], report["checksums"]["base_start"]);
    let before = scene_text(&mut c);
    let history = c.call("history.list", J::Null).unwrap();
    let err = c.call_err("proposal.accept_verified", json!({"id": id, "verified_state": report["verified_state"]}));
    assert_eq!(err.code, CONFLICT);
    assert_eq!(err.kind(), Some("stale_verification"));
    assert_eq!(scene_text(&mut c), before);
    assert_eq!(c.call("history.list", J::Null).unwrap(), history);
    assert_eq!(c.call("proposal.get", json!({"id": id})).unwrap()["accepts_cleanly"], true);

    let fresh = c.call("proposal.verify", verify.clone()).unwrap();
    other.call("proposal.apply", json!({"id": id, "ops": [{"op": "patch", "entity": body, "component": BODY, "path": "pos", "value": [0, -30]}]})).unwrap();
    let err = c.call_err("proposal.accept_verified", json!({"id": id, "verified_state": fresh["verified_state"]}));
    assert_eq!(err.kind(), Some("stale_verification"));
    assert_eq!(scene_text(&mut c), before);
    assert_eq!(c.call("proposal.verify", verify.clone()).unwrap()["passed"], false, "the unverified candidate would fail");

    other.call("proposal.apply", json!({"id": id, "ops": [{"op": "patch", "entity": body, "component": BODY, "path": "pos", "value": [0, 13]}]})).unwrap();
    let final_report = c.call("proposal.verify", verify).unwrap();
    assert_eq!(final_report["passed"], true);
    let accepted = c.call("proposal.accept_verified", json!({"id": id, "verified_state": final_report["verified_state"]})).unwrap();
    assert_eq!(accepted["checksum"], final_report["checksums"]["candidate_start"]);
    let activity = c.call("activity.list", json!({})).unwrap();
    let entry = activity["entries"].as_array().unwrap().iter().find(|e| e["method"] == "proposal.accept_verified" && e["ok"] == true).unwrap();
    assert_eq!(entry["kind"], "proposal");
    assert_eq!(entry["read"], false);
    assert!(entry["summary"].as_str().unwrap().contains(&format!("proposal.accept_verified {id}")));
    assert!(entry["diff"].is_object());
}

#[test]
fn verified_accept_requires_a_complete_matching_state() {
    let host = TestHost::standard();
    let mut c = host.client("tok-all");
    let id = begin(&mut c, "empty");
    let report = c.call("proposal.verify", json!({"id": id, "inputs": {"kind": "idle", "ticks": 1}})).unwrap();
    let state = report["verified_state"].clone();
    for bad in [J::Null, json!({}), json!({"id": id, "document_revision": 0}), json!({"id": id, "document_revision": -1, "proposal_revision": 0})] {
        assert_eq!(c.call_err("proposal.accept_verified", json!({"id": id, "verified_state": bad})).code, INVALID_PARAMS);
    }
    assert_eq!(c.call_err("proposal.accept_verified", json!({"id": id})).code, INVALID_PARAMS);
    for document_id in [json!(""), json!("abcd"), json!("0000000000000000000000000000000g"), json!(0)] {
        let mut bad = state.clone();
        bad["document_id"] = document_id;
        assert_eq!(c.call_err("proposal.accept_verified", json!({"id": id, "verified_state": bad})).code, INVALID_PARAMS);
    }
    for (key, value) in [
        ("document_revision", json!(-1)),
        ("proposal_revision", json!("0")),
        ("id", json!("not-a-proposal")),
    ] {
        let mut bad = state.clone();
        bad[key] = value;
        assert_eq!(c.call_err("proposal.accept_verified", json!({"id": id, "verified_state": bad})).code, INVALID_PARAMS);
    }
    for key in ["document_id", "id", "document_revision", "proposal_revision"] {
        let mut bad = state.clone();
        bad.as_object_mut().unwrap().remove(key);
        assert_eq!(c.call_err("proposal.accept_verified", json!({"id": id, "verified_state": bad})).code, INVALID_PARAMS);
    }
    let other = begin(&mut c, "also empty");
    assert_eq!(c.call_err("proposal.accept_verified", json!({"id": other, "verified_state": state})).kind(), Some("stale_verification"));
    assert_eq!(c.call("proposal.list", J::Null).unwrap()["proposals"].as_array().unwrap().len(), 2);
    assert!(c.call("history.list", J::Null).unwrap()["entries"].as_array().unwrap().is_empty());
    c.call("proposal.accept_verified", json!({"id": id, "verified_state": state})).unwrap();
    assert_eq!(c.call_err("proposal.accept_verified", json!({"id": id, "verified_state": state})).kind(), Some("unknown_proposal"));
}

#[test]
fn verified_accept_rejects_a_state_from_another_host_with_matching_counters() {
    let host = TestHost::standard();
    let other_host = TestHost::standard();
    let mut c = host.client("tok-all");
    let mut other = other_host.client("tok-all");
    let body = guid_of(&mut c, "body_05");
    let id = begin(&mut c, "lift");
    let other_id = begin(&mut other, "sink");
    assert_eq!(id, other_id);
    for (client, y) in [(&mut c, 12), (&mut other, -30)] {
        client.call("proposal.apply", json!({"id": id, "ops": [{"op": "patch", "entity": body, "component": BODY, "path": "pos", "value": [0, y]}]})).unwrap();
    }
    let params = json!({"id": id, "inputs": {"kind": "idle", "ticks": 1}, "checks": ["lost_bodies.max == 0"]});
    let good = c.call("proposal.verify", params.clone()).unwrap();
    let bad = other.call("proposal.verify", params).unwrap();
    assert_eq!(good["passed"], true);
    assert_eq!(bad["passed"], false);
    for key in ["id", "document_revision", "proposal_revision"] {
        assert_eq!(good["verified_state"][key], bad["verified_state"][key]);
    }
    assert_ne!(good["verified_state"]["document_id"], bad["verified_state"]["document_id"]);
    let before = scene_text(&mut other);
    let err = other.call_err("proposal.accept_verified", json!({"id": id, "verified_state": good["verified_state"]}));
    assert_eq!(err.kind(), Some("stale_verification"));
    assert_eq!(scene_text(&mut other), before);
    assert!(other.call("history.list", J::Null).unwrap()["entries"].as_array().unwrap().is_empty());
    assert!(other.call("proposal.get", json!({"id": id})).is_ok());
}
