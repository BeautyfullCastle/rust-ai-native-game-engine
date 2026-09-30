//! The real `orr` binary as a child process against an in-process ERP host
//! (PhysGame, PhysMetrics, scripted players, a real socket on port 0).
// Test harness: threads and wall-clock waits are fine here.
#![allow(clippy::disallowed_types)]

mod common;

use std::io::{BufRead, BufReader};
use std::process::Stdio;
use std::sync::mpsc::channel;
use std::time::Duration;

use common::*;
use serde_json::{json, Value as J};

#[test]
fn status_scene_get_by_name_and_schema() {
    let host = TestHost::start();
    let st = host.orr(&["status"]);
    assert_eq!(st.code, 0, "{}", st.err);
    for want in ["PhysGame", "mode: edit", "entities 49", "you: claude", "open proposals: 0", "unsaved changes: no", "build id 0x"] {
        assert!(st.out.contains(want), "status mentions {want}: {}", st.out);
    }
    let sc = host.orr(&["scene"]);
    assert_eq!(sc.code, 0);
    assert!(sc.out.contains("body_05") && sc.out.contains("e_0000000e") && sc.out.contains("Singletons:"), "{}", sc.out);
    let filtered = host.orr(&["scene", "--filter", "paddle", "--limit", "5"]);
    assert!(filtered.out.contains("paddle_0") && !filtered.out.contains("body_05"), "{}", filtered.out);
    let has = host.orr(&["scene", "--has", "PaddleTag", "--components"]);
    assert_eq!(has.code, 0, "{}", has.err);
    assert!(has.out.contains("paddle_1") && has.out.contains("orr_physics::Body ="), "{}", has.out);

    // get by name and by GUID give the same exact values.
    let by_name = host.orr(&["get", "body_05", "Body.pos"]);
    let by_guid = host.orr(&["get", "e_0000000e", "orr_physics::Body.pos"]);
    assert_eq!(by_name.code, 0, "{}", by_name.err);
    assert_eq!(by_name.out, by_guid.out);
    let pos: J = serde_json::from_str(&by_name.out).unwrap();
    assert_eq!(pos.as_array().unwrap().len(), 2);
    let whole = host.orr(&["get", "body_05"]);
    assert!(whole.out.contains("\"orr_physics::Collider\"") && whole.out.contains("body_05"), "{}", whole.out);
    assert_eq!(host.orr(&["get", "@Scene.height"]).out.trim(), "40");
    let missing = host.orr(&["get", "no_such_body"]);
    assert_eq!(missing.code, 1);
    assert!(missing.err.contains("no entity named 'no_such_body'"), "{}", missing.err);

    let types = host.orr(&["schema", "--types"]);
    assert!(types.out.contains("orr_physics::Body (component)"), "{}", types.out);
    let one = host.orr(&["schema", "Body"]);
    assert_eq!(one.code, 0, "{}", one.err);
    assert!(one.out.contains("\"title\":\"orr_physics::Body\""), "{}", one.out);
}

#[test]
fn set_two_fields_is_one_history_entry_and_undo_restores_the_yaml() {
    let host = TestHost::start();
    let before = host.yaml();
    let r = host.orr(&["set", "body_05", "Body.pos=[6,18]", "orr_physics::Body.angle=0.5"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.out.contains("[6,18]") && r.out.contains("one undo step"), "{}", r.out);
    let entries = host.history();
    assert_eq!(entries.len(), 1, "two assignments, one entry: {entries:?}");
    assert_eq!(entries[0]["origin"], "agent:claude");
    assert!(entries[0]["label"].as_str().unwrap().starts_with("orr set body_05"));
    let after = host.yaml();
    assert_ne!(after, before);
    assert!(after.contains("pos: [6, 18]") && after.contains("angle: 0.5"), "exact values written");

    let u = host.orr(&["undo"]);
    assert_eq!(u.code, 0, "{}", u.err);
    assert_eq!(host.yaml(), before, "undo restores the exact YAML");
    let redo = host.orr(&["redo"]);
    assert_eq!(redo.code, 0, "{}", redo.err);
    assert_eq!(host.yaml(), after);
    assert_eq!(host.orr(&["history"]).out.matches("agent:claude").count(), 1);

    // A bad assignment in the middle changes nothing at all.
    let bad = host.orr(&["set", "body_06", "Body.pos=[1,1]", "Collider.no_such_field=1"]);
    assert_eq!(bad.code, 1);
    assert!(bad.err.contains("nothing was changed") && bad.err.contains("no_such_field"), "{}", bad.err);
    assert_eq!(host.yaml(), after);
    assert_eq!(host.history().len(), 1);
}

#[test]
fn value_forms_and_short_names() {
    let host = TestHost::start();
    // A bare string for an enum, a bare decimal, JSON.
    let r = host.orr(&["set", "body_05", "Body.angle=.25", "Body.omega=-1.5", "Body.pos=[0, 12.5]"]);
    assert_eq!(r.code, 0, "{}", r.err);
    let y = host.yaml();
    assert!(y.contains("angle: 0.25") && y.contains("omega: -1.5") && y.contains("pos: [0, 12.5]"), "{y}");
    let kind = host.orr(&["get", "paddle_0", "Body.kind"]).out;
    assert!(kind.trim().starts_with('"') || kind.trim().starts_with(char::is_alphabetic), "{kind}");
    let s = host.orr(&["set", "paddle_0", "Body.kind=kinematic"]);
    assert_eq!(s.code, 0, "{}", s.err);
    assert!(host.orr(&["get", "paddle_0", "Body.kind"]).out.contains("kinematic"));
    // A singleton.
    let ss = host.orr(&["propose", "more spawn", "sset", "Scene.spawn_batch=3"]);
    assert_eq!(ss.code, 0, "{}", ss.err);
    assert!(ss.out.contains("spawn_batch"), "{}", ss.out);
}

#[test]
fn ambiguous_names_and_unknown_types_are_errors_with_candidates() {
    let host = TestHost::start();
    // Two entities with one name.
    let mut c = host.erp("claude-tok");
    c.call("world.rename", json!({"entity": "e_0000000a", "name": "twin"})).unwrap();
    c.call("world.rename", json!({"entity": "e_0000000b", "name": "twin"})).unwrap();
    let r = host.orr(&["get", "twin"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("ambiguous") && r.err.contains("e_0000000a") && r.err.contains("e_0000000b"), "{}", r.err);
    let r = host.orr(&["set", "twin", "Body.pos=[0,0]"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("ambiguous"), "{}", r.err);
    // By GUID it works.
    assert_eq!(host.orr(&["get", "e_0000000a", "Body.pos"]).code, 0);
    // An unknown component lists the known ones.
    let r = host.orr(&["get", "e_0000000a", "Nope.pos"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("unknown component type 'Nope'") && r.err.contains("orr_physics::Body"), "{}", r.err);
    // Short names are case-insensitive as a fallback, full names always work.
    assert_eq!(host.orr(&["get", "e_0000000a", "body.pos"]).code, 0);
    assert_eq!(host.orr(&["get", "e_0000000a", "orr_physics::Body.pos"]).code, 0);
}

#[test]
fn spawn_add_remove_rename_despawn() {
    let host = TestHost::start();
    let before = host.yaml();
    let r = host.orr(&["spawn", "--name", "crate", "Body={\"pos\":[0,20],\"kind\":\"dynamic\"}"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.out.contains("spawned e_") && r.out.contains("\"crate\""), "{}", r.out);
    assert_eq!(host.orr(&["get", "crate", "Body.pos"]).code, 0);
    assert_eq!(host.orr(&["add", "crate", "PaddleTag"]).code, 0);
    assert!(host.orr(&["scene", "--has", "PaddleTag"]).out.contains("crate"));
    assert_eq!(host.orr(&["remove", "crate", "PaddleTag"]).code, 0);
    assert_eq!(host.orr(&["rename", "crate", "box"]).code, 0);
    assert_eq!(host.orr(&["get", "box", "Body.pos"]).code, 0);
    assert_eq!(host.orr(&["despawn", "box"]).code, 0);
    assert_eq!(host.history().len(), 5);
    for _ in 0..5 {
        assert_eq!(host.orr(&["undo"]).code, 0);
    }
    assert_eq!(host.yaml(), before);
}

#[test]
fn apply_passing_is_accepted_with_one_history_entry() {
    let host = TestHost::start();
    let r = host.orr(&["apply", "lift hero", "set", "body_05", "Body.pos=[6,18]", "rename", "body_05", "hero", "--check", "lost_bodies.max == 0", "--idle", "120"]);
    assert_eq!(r.code, 0, "{}\n{}", r.out, r.err);
    for want in ["Proposal p1 \"lift hero\"", "CHECKS PASSED: 1/1", "[pass] lost_bodies.max == 0", "APPLIED: p1 accepted as history entry #"] {
        assert!(r.out.contains(want), "apply prints {want}: {}", r.out);
    }
    let entries = host.history();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["label"], "lift hero");
    assert_eq!(entries[0]["origin"], "agent:claude");
    assert_eq!(entries[0]["op_count"], 2);
    let y = host.yaml();
    assert!(y.contains("name: hero") && y.contains("pos: [6, 18]"));
    assert!(host.orr(&["proposals"]).out.contains("No open proposals"));
    // The rename is visible to the next command by its new name.
    assert_eq!(host.orr(&["get", "hero", "Body.pos"]).out.replace(char::is_whitespace, ""), "[6,18]");
    assert_eq!(host.orr(&["undo"]).code, 0);
    assert!(host.yaml().contains("name: body_05"));
}

#[test]
fn apply_default_check_and_json() {
    let host = TestHost::start();
    // No --check: the game reports lost_bodies, so it is the default.
    let r = host.orr(&["apply", "nudge", "set", "body_05", "Body.angle=1", "--json", "--idle", "60"]);
    assert_eq!(r.code, 0, "{}\n{}", r.out, r.err);
    let j = r.json();
    assert_eq!(j["accepted"], true);
    assert_eq!(j["outcome"], "accepted");
    assert_eq!(j["proposal"], "p1");
    assert_eq!(j["verify"]["checks"]["passed"], true);
    assert_eq!(j["verify"]["checks"]["results"][0]["check"], "lost_bodies.max == 0");
    assert!(j["history_id"].as_u64().is_some());
}

#[test]
fn apply_failing_check_rejects_and_exits_4() {
    let host = TestHost::start();
    let before = host.yaml();
    // Below the floor: the body is lost.
    let r = host.orr(&["apply", "sink body", "set", "body_06", "Body.pos=[0,-100]", "--check", "lost_bodies.max == 0", "--idle", "120"]);
    assert_eq!(r.code, 4, "{}\n{}", r.out, r.err);
    for want in ["CHECKS FAILED: 0/1", "[FAIL] lost_bodies.max == 0", "NOT APPLIED", "rejected", "lost_bodies: "] {
        assert!(r.out.contains(want), "the report has {want}: {}", r.out);
    }
    assert_eq!(host.yaml(), before, "the document is unchanged");
    assert!(host.history().is_empty());
    assert!(host.orr(&["proposals"]).out.contains("No open proposals"));

    // --keep leaves the proposal open.
    let k = host.orr(&["apply", "sink body", "set", "body_06", "Body.pos=[0,-100]", "--keep", "--idle", "120"]);
    assert_eq!(k.code, 4);
    assert!(k.out.contains("left open (--keep)"), "{}", k.out);
    let list = host.orr(&["proposals"]);
    assert!(list.out.contains("p2") && list.out.contains("sink body"), "{}", list.out);
    // The same checks fail again by hand; then reject.
    let v = host.orr(&["verify", "p2", "--idle", "120", "--check", "lost_bodies.max == 0"]);
    assert_eq!(v.code, 4);
    assert_eq!(host.orr(&["reject", "p2"]).code, 0);
    assert_eq!(host.yaml(), before);

    // A nonsense check is a failed command (exit 1), and the proposal is cleaned up.
    let bad = host.orr(&["apply", "x", "set", "body_06", "Body.angle=1", "--check", "this is not a rule"]);
    assert_eq!(bad.code, 1, "{}\n{}", bad.out, bad.err);
    assert!(host.orr(&["proposals"]).out.contains("No open proposals"));
}

#[test]
fn propose_verify_accept_step_by_step_and_ops_from_stdin() {
    let host = TestHost::start();
    let p = host.orr(&["propose", "lift hero", "set", "body_05", "Body.pos=[6,18]", "rename", "body_05", "hero"]);
    assert_eq!(p.code, 0, "{}", p.err);
    assert!(p.out.contains("Proposal p1") && p.out.contains("Diff of the scene text") && p.out.contains("+    name: hero"), "{}", p.out);
    assert!(p.out.contains("The scene is unchanged"));
    assert!(host.history().is_empty());
    assert!(host.orr(&["diff", "p1"]).out.contains("body_05 -> hero"));
    let g = host.orr(&["get", "e_0000000e", "Body.pos", "--proposal", "p1"]);
    assert_eq!(g.out.replace(char::is_whitespace, ""), "[6,18]", "{}", g.out);
    let v = host.orr(&["verify", "p1", "--idle", "60", "--check", "lost_bodies.max == 0"]);
    assert_eq!(v.code, 0, "{}", v.err);
    assert!(v.out.contains("Proposal p1: replayed 60 ticks"), "{}", v.out);
    let a = host.orr(&["accept", "p1"]);
    assert_eq!(a.code, 0, "{}", a.err);
    assert_eq!(host.history().len(), 1);

    // Ops as JSON from stdin, with a name for the entity.
    let ops = r#"[{"op":"patch","entity":"hero","component":"Body","path":"angle","value":2}]"#;
    let p = host.orr_stdin(ops, &["propose", "spin", "-"]);
    assert_eq!(p.code, 0, "{}\n{}", p.out, p.err);
    assert!(p.out.contains("Proposal p2"), "{}", p.out);
    assert_eq!(host.orr(&["reject", "p2"]).code, 0);
    // Or from a file, through apply.
    let path = std::env::temp_dir().join(format!("orr_cli_ops_{}.json", std::process::id()));
    std::fs::write(&path, r#"{"ops":[{"op":"rename","entity":"e_0000000f","name":"copy"}]}"#).unwrap();
    let f = host.orr(&["apply", "name it", "--ops-file", path.to_str().unwrap(), "--idle", "30"]);
    let _ = std::fs::remove_file(&path);
    assert_eq!(f.code, 0, "{}\n{}", f.out, f.err);
    assert!(host.yaml().contains("name: copy"));
    // Errors in ops are usage errors and stage nothing.
    let bad = host.orr(&["propose", "bad", "frobnicate", "x"]);
    assert_eq!(bad.code, 2, "{}", bad.err);
    assert!(host.orr(&["proposals"]).out.contains("No open proposals"));
    let bad = host.orr(&["reject", "hero"]);
    assert_eq!(bad.code, 2);
}

#[test]
fn verify_baseline() {
    let host = TestHost::start();
    let r = host.orr(&["verify", "--idle", "120"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.out.contains("Baseline (scene alone): replayed 120 ticks") && r.out.contains("lost_bodies: 0 -> 0") && r.out.contains("No checks given"), "{}", r.out);
    let b = host.orr(&["verify", "--bot", "60", "--seed", "3", "--check", "entities.start == 49", "--check", "mean_height.final > 0"]);
    assert_eq!(b.code, 0, "{}\n{}", b.out, b.err);
    assert!(b.out.contains("seed 3") && b.out.contains("CHECKS PASSED: 2/2"), "{}", b.out);
    let f = host.orr(&["verify", "--idle", "30", "--check", "entities.final == 0"]);
    assert_eq!(f.code, 4);
    assert!(f.out.contains("[FAIL] entities.final == 0"), "{}", f.out);
    // The same numbers with --json.
    let j = host.orr(&["verify", "--idle", "30", "--json"]).json();
    assert_eq!(j["ticks"], 30);
    // Two inputs at once is a usage error.
    assert_eq!(host.orr(&["verify", "--idle", "30", "--bot", "30"]).code, 2);
    // No play session yet: last-play is an ERP error with a hint.
    let lp = host.orr(&["verify", "--last-play"]);
    assert_eq!(lp.code, 1);
    assert!(lp.err.contains("orr sim"), "{}", lp.err);
}

#[test]
fn sim_step_seek_state_stop_and_last_play() {
    let host = TestHost::start();
    assert_eq!(host.orr(&["sim", "state"]).out.trim_start().chars().next(), Some('E'), "edit mode first");
    let s = host.orr(&["sim", "start"]);
    assert_eq!(s.code, 0, "{}", s.err);
    assert!(s.out.contains("Play session: tick 0"), "{}", s.out);
    let st = host.orr(&["sim", "step", "60"]);
    assert!(st.out.contains("tick 60"), "{}", st.out);
    let sk = host.orr(&["sim", "seek", "30"]);
    assert!(sk.out.contains("tick 30") && sk.out.contains("recorded 0..=60"), "{}", sk.out);
    let state = host.orr(&["sim", "state", "--json"]).json();
    assert_eq!(state["mode"], "play");
    assert_eq!(state["head_tick"], 30);
    assert!(host.orr(&["status"]).out.contains("tick 30"));
    let sp = host.orr(&["sim", "speed", "2"]);
    assert!(sp.out.contains("Speed 2.000x"), "{}", sp.out);
    assert_eq!(host.orr(&["sim", "seek"]).code, 2);
    assert_eq!(host.orr(&["sim", "speed", "fast"]).code, 2);
    // Edits are refused while playing (history undo/accept), stop first.
    let stop = host.orr(&["sim", "stop"]);
    assert_eq!(stop.code, 0, "{}", stop.err);
    assert!(stop.out.contains("Play stopped at tick 30") && stop.out.contains("orr verify --last-play"), "{}", stop.out);
    // The recording verifies against the scene.
    let v = host.orr(&["verify", "--last-play", "--check", "recording_matches"]);
    assert_eq!(v.code, 0, "{}\n{}", v.out, v.err);
}

#[test]
fn activity_shows_the_previous_commands() {
    let host = TestHost::start();
    assert_eq!(host.orr(&["activity"]).out.trim(), "No activity.");
    host.orr(&["set", "body_05", "Body.pos=[6,18]"]);
    host.orr(&["apply", "nudge", "set", "body_06", "Body.angle=1", "--idle", "30"]);
    let a = host.orr(&["activity"]);
    assert_eq!(a.code, 0, "{}", a.err);
    for want in ["claude", "world.patch e_0000000e", "(was [-12.90433,19.97954])", "proposal.begin", "proposal.verify p1", "proposal.accept p1"] {
        assert!(a.out.contains(want), "activity has {want}: {}", a.out);
    }
    assert!(!a.out.contains("session"), "connect lines are hidden by default: {}", a.out);
    assert!(host.orr(&["activity", "--reads"]).out.contains("connected"));
    // --since and -n.
    let last = host.orr(&["activity", "--json"]).json()["last_seq"].as_u64().unwrap();
    assert_eq!(host.orr(&["activity", "--since", &last.to_string()]).out.trim(), "No activity.");
    assert_eq!(host.orr(&["activity", "-n", "1"]).out.trim().lines().count(), 1);
    // The status counts the connected clients.
    let j = host.orr(&["status", "--json"]).json();
    assert!(j["clients"].as_array().unwrap().iter().any(|c| c["client"] == "claude"));
}

#[test]
fn activity_follow_streams_new_entries() {
    let host = TestHost::start();
    host.orr(&["set", "body_05", "Body.pos=[1,1]"]);
    let mut child = command(&host.url, Some("claude-tok"), &["activity", "-f"]).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let out = child.stdout.take().unwrap();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        for l in BufReader::new(out).lines().map_while(Result::ok) {
            if tx.send(l).is_err() {
                break;
            }
        }
    });
    // The backlog first.
    let first = rx.recv_timeout(Duration::from_secs(20)).expect("a backlog line");
    assert!(first.contains("claude") && first.contains("tx.begin"), "{first}");
    // Then what happens while it follows.
    host.orr(&["set", "body_06", "Body.pos=[2,2]"]);
    let mut seen = false;
    while let Ok(l) = rx.recv_timeout(Duration::from_secs(20)) {
        if l.contains("e_0000000f") && l.contains("world.patch") {
            seen = true;
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    assert!(seen, "the follower printed the new entry");
}

#[test]
fn json_output_parses() {
    let host = TestHost::start();
    for args in [
        vec!["status", "--json"],
        vec!["scene", "--json"],
        vec!["scene", "--components", "--limit", "3", "--json"],
        vec!["get", "body_05", "--json"],
        vec!["get", "body_05", "Body.pos", "--json"],
        vec!["schema", "--types", "--json"],
        vec!["schema", "Body", "--json"],
        vec!["history", "--json"],
        vec!["proposals", "--json"],
        vec!["sim", "state", "--json"],
        vec!["activity", "--json"],
        vec!["save", "--json"],
        vec!["agents-md", "--json"],
    ] {
        let r = host.orr(&args);
        assert_eq!(r.code, 0, "{args:?}: {}", r.err);
        let _ = r.json();
    }
    let s = host.orr(&["set", "body_05", "Body.angle=1", "--json"]).json();
    assert_eq!(s["history"]["origin"], "agent:claude");
    let p = host.orr(&["propose", "x", "set", "body_05", "Body.angle=2", "--json"]).json();
    assert_eq!(p["id"], "p1");
    assert_eq!(p["op_count"], 1);
    // Errors as JSON go to stderr.
    let e = host.orr(&["get", "nobody", "--json"]);
    assert_eq!(e.code, 1);
    assert!(e.out.is_empty());
    let ej: J = serde_json::from_str(e.err.trim()).unwrap();
    assert_eq!(ej["exit_code"], 1);
}

#[test]
fn save_prints_yaml_and_agents_md_matches_the_committed_guide() {
    let host = TestHost::start();
    let y = host.orr(&["save"]);
    assert_eq!(y.code, 0);
    assert_eq!(y.out, host.yaml());
    // The host has no scene file: --write says so.
    let w = host.orr(&["save", "--write"]);
    assert_eq!(w.code, 1, "{}", w.out);
    let guide = host.orr(&["agents-md"]);
    assert_eq!(guide.code, 0, "{}", guide.err);
    assert_eq!(guide.out, std::fs::read_to_string(AGENTS_PATH).unwrap(), "`orr agents-md` prints docs/AGENTS.md for the demo scene");
}

#[test]
fn unreachable_host_is_exit_3_with_the_command_to_start_one() {
    let r = finish(command("ws://127.0.0.1:1", None, &["status"]).output().unwrap());
    assert_eq!(r.code, 3, "{}", r.err);
    assert!(r.err.contains("cannot reach an Orrery host at ws://127.0.0.1:1"), "{}", r.err);
    assert!(r.err.contains("orr_editor --erp 127.0.0.1:7777 --erp-dev") && r.err.contains("orr_remote_host --dev-no-auth"), "{}", r.err);
    assert!(r.out.is_empty());
    // Every kind of command, and the streaming one, agree.
    for args in [vec!["scene"], vec!["set", "a", "Body.pos=[0,0]"], vec!["apply", "x", "despawn", "e_00000001"], vec!["activity", "-f"]] {
        let r = finish(command("ws://127.0.0.1:1", None, &args).output().unwrap());
        assert_eq!(r.code, 3, "{args:?}: {}", r.err);
    }
    // No network access to `orr help`.
    assert_eq!(finish(command("ws://127.0.0.1:1", None, &["help", "apply"]).output().unwrap()).code, 0);
}

#[test]
fn bad_usage_is_exit_2() {
    let host = TestHost::start();
    for args in [
        vec![],
        vec!["frobnicate"],
        vec!["set"],
        vec!["set", "body_05"],
        vec!["set", "body_05", "not-an-assignment"],
        vec!["get"],
        vec!["scene", "--nope"],
        vec!["scene", "extra"],
        vec!["scene", "--limit", "many"],
        vec!["accept"],
        vec!["accept", "p"],
        vec!["propose", "label only"],
        vec!["apply"],
        vec!["verify", "--bot"],
        vec!["sim"],
        vec!["sim", "warp"],
        vec!["history", "-n", "x"],
        vec!["--timeout", "soon", "status"],
    ] {
        let r = host.orr(&args);
        assert_eq!(r.code, 2, "{args:?}: {}\n{}", r.out, r.err);
        assert!(r.err.contains("error:"), "{args:?}: {}", r.err);
    }
    assert!(finish(command("http://x", None, &["status"]).output().unwrap()).err.contains("WebSocket"));
    // Help is not an error.
    let h = host.orr(&["help"]);
    assert_eq!(h.code, 0);
    assert!(h.out.contains("Exit codes"), "{}", h.out);
    for c in ["status", "scene", "get", "schema", "set", "apply", "verify", "activity", "sim", "propose"] {
        let h = host.orr(&[c, "--help"]);
        assert_eq!(h.code, 0, "{c}");
        assert!(h.out.contains(&format!("orr {c}")) && h.out.contains("Example"), "{c}: {}", h.out);
        assert_eq!(h.out, host.orr(&["help", c]).out);
    }
    assert_eq!(host.orr(&["help", "nope"]).code, 2);
    assert!(host.orr(&["--version"]).out.starts_with("orr "));
    // `--name` is informational, except where a command has its own.
    assert_eq!(host.orr(&["--name", "codex", "status"]).code, 0);
    assert_eq!(host.orr(&["status", "--name", "codex"]).code, 0);
    let r = host.orr(&["spawn", "--name", "thing"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(host.orr(&["get", "thing"]).out.contains("\"name\": \"thing\""));
}

#[test]
fn tokens_come_from_the_environment_or_the_flag_and_capabilities_are_enforced() {
    let host = TestHost::start();
    // A host with tokens refuses no token and a wrong token: exit 3.
    let none = host.orr_as(None, &["status"]);
    assert_eq!(none.code, 3, "{}", none.err);
    let wrong = host.orr_as(Some("sekrit-wrong"), &["status"]);
    assert_eq!(wrong.code, 3, "{}", wrong.err);
    assert!(!wrong.all().contains("sekrit-wrong"), "a wrong token is never echoed: {}", wrong.all());
    // The environment, the flag, and the ORR_ERP_URL alias.
    assert_eq!(host.orr_as(Some("claude-tok"), &["status"]).code, 0);
    let flag = finish(command(&host.url, None, &["--token", "claude-tok", "status"]).output().unwrap());
    assert_eq!(flag.code, 0, "{}", flag.err);
    let alias = finish(
        std::process::Command::new(env!("CARGO_BIN_EXE_orr"))
            .args(["status"])
            .env_remove("ORR_ERP")
            .env("ORR_ERP_URL", &host.url)
            .env("ORR_ERP_TOKEN", "claude-tok")
            .output()
            .unwrap(),
    );
    assert_eq!(alias.code, 0, "{}", alias.err);
    let explicit = finish(command("ws://127.0.0.1:1", Some("claude-tok"), &["--erp", &host.url, "status"]).output().unwrap());
    assert_eq!(explicit.code, 0, "--erp beats $ORR_ERP: {}", explicit.err);
    // A limited token: reads and edits work, accepting does not.
    let l = |args: &[&str]| host.orr_as(Some("limited-tok"), args);
    assert_eq!(l(&["scene"]).code, 0);
    let p = l(&["propose", "x", "set", "body_05", "Body.angle=1"]);
    assert_eq!(p.code, 0, "{}", p.err);
    let a = l(&["accept", "p1"]);
    assert_eq!(a.code, 1);
    assert!(a.err.contains("permission_denied") && a.err.contains("approve") || a.err.contains("capability"), "{}", a.err);
    let ap = l(&["apply", "y", "set", "body_06", "Body.angle=1", "--idle", "30"]);
    assert_eq!(ap.code, 1, "{}\n{}", ap.out, ap.err);
    assert!(ap.out.contains("CHECKS PASSED") && ap.err.contains("still open"), "{}\n{}", ap.out, ap.err);
    assert!(host.history().is_empty(), "nothing was applied");
    assert_eq!(l(&["sim", "start"]).code, 1);
}

#[test]
fn secrets_are_never_printed() {
    let host = TestHost::start();
    let secret = "claude-tok";
    // With the token in the environment and on the command line, on success and on failure.
    let runs = [
        host.orr(&["status"]),
        host.orr(&["status", "--json"]),
        host.orr(&["set", "body_05", "Body.pos=[1,1]"]),
        host.orr(&["get", "nobody"]),
        host.orr(&["activity", "--reads"]),
        host.orr(&["agents-md"]),
        host.orr(&["frobnicate"]),
        finish(command(&host.url, None, &["--token", secret, "status"]).output().unwrap()),
        finish(command(&host.url, None, &["--token", secret, "bogus"]).output().unwrap()),
        finish(command("ws://127.0.0.1:1", Some(secret), &["--token", secret, "status"]).output().unwrap()),
        finish(command("ws://127.0.0.1:1/?token=hunter2secret", None, &["status"]).output().unwrap()),
        finish(command("ws://127.0.0.1:1/?token=hunter2secret", None, &["--json", "status"]).output().unwrap()),
        finish(command("nonsense://x/?token=hunter2secret", None, &["status"]).output().unwrap()),
    ];
    for r in &runs {
        let all = r.all();
        assert!(!all.contains(secret), "the token leaked: {all}");
        assert!(!all.contains("hunter2secret"), "a URL token leaked: {all}");
    }
    assert_eq!(runs[9].code, 3);
    assert_eq!(runs[10].code, 3);
    // The activity feed knows the client name, not the token.
    assert!(!host.orr(&["activity", "--reads", "-n", "500"]).all().contains(secret));
}

/// `orr sim step` without `sim start` starts a paused session first, like
/// the editor's Step button.
#[test]
fn sim_step_starts_a_session_when_none_runs() {
    let host = TestHost::start();
    let st = host.orr(&["sim", "step", "45"]);
    assert_eq!(st.code, 0, "{}\n{}", st.out, st.err);
    assert!(st.out.contains("tick 45"), "{}", st.out);
    let state = host.orr(&["sim", "state", "--json"]).json();
    assert_eq!(state["mode"], "play");
    assert_eq!(state["head_tick"], 45);
}
