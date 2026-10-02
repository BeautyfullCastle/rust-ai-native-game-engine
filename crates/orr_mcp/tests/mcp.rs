//! `orr_mcp` as a child process over stdio, against a real ERP host.
//!
//! `an_agent_modifies_verifies_and_accepts_in_one_flow` is the M5 criterion.

mod common;

use std::time::Duration;

use common::*;
use serde_json::{json, Value as J};

const BODY: &str = "orr_physics::Body";

fn structured(r: &J) -> &J {
    &r["structuredContent"]
}

fn guid_from_overview(m: &mut McpChild, name: &str) -> String {
    let r = m.call_tool("scene_overview", json!({"name": name}));
    assert_eq!(r["isError"], false, "{r}");
    structured(&r)["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == name)
        .unwrap_or_else(|| panic!("no entity {name} in {r}"))["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn an_agent_modifies_verifies_and_accepts_in_one_flow() {
    let host = TestHost::start();
    let mut erp = host.erp("claude-tok");
    let original = erp.call("scene.save", json!({})).unwrap()["text"].as_str().unwrap().to_string();

    let mut m = McpChild::against(&host, "claude-tok", &[]);
    // -- handshake
    let init = m.initialize();
    assert_eq!(init["protocolVersion"], "2025-06-18");
    assert_eq!(init["serverInfo"]["name"], "orr_mcp");
    assert!(init["capabilities"]["tools"].is_object() && init["capabilities"]["resources"].is_object());
    assert!(init["instructions"].as_str().unwrap().contains("propose_changes"));

    // -- tools/list: every tool has a description and an object schema
    let list = m.request("tools/list", json!({}));
    let tools = list["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for want in [
        "scene_overview", "get_entity", "get_schema", "propose_changes", "verify_proposal", "accept_proposal", "reject_proposal",
        "list_proposals", "history", "undo", "sim_run",
    ] {
        assert!(names.contains(&want), "{want} in {names:?}");
    }
    for t in tools {
        assert_eq!(t["inputSchema"]["type"], "object", "{}", t["name"]);
        assert!(t["inputSchema"]["properties"].is_object());
        assert!(t["description"].as_str().unwrap().len() > 40, "{}", t["name"]);
    }
    let propose = tools.iter().find(|t| t["name"] == "propose_changes").unwrap();
    assert_eq!(propose["inputSchema"]["required"], json!(["ops"]));
    assert!(propose["description"].as_str().unwrap().contains("\"op\":\"patch\""), "the description has examples");

    // -- look
    let body = guid_from_overview(&mut m, "body_05");
    let schema = m.call_tool("get_schema", json!({"type": BODY}));
    assert_eq!(schema["isError"], false);
    assert!(structured(&schema)["schema"]["properties"]["pos"].is_object());
    assert!(!McpChild::text(&schema).contains("Fixed-point number, Q48.16"), "the boilerplate is stripped");
    let entity = m.call_tool("get_entity", json!({"entity": body, "component": BODY, "path": "pos"}));
    assert_eq!(entity["isError"], false);
    assert!(structured(&entity)["value"].is_array());

    // -- propose: rename + move a body; the scene is untouched
    let proposed = m.call_tool(
        "propose_changes",
        json!({"label": "lift body_05", "ops": [
            {"op": "rename", "entity": body, "name": "hero"},
            {"op": "patch", "entity": body, "component": BODY, "path": "pos", "value": [0, 12.5]},
        ]}),
    );
    assert_eq!(proposed["isError"], false, "{proposed}");
    let text = McpChild::text(&proposed);
    assert!(text.contains("Proposal p1 \"lift body_05\" by agent:claude"), "{text}");
    assert!(text.contains("+    name: hero"), "the diff is in the text: {text}");
    assert!(text.contains("verify_proposal"), "it says what to do next");
    let s = structured(&proposed);
    assert_eq!((s["id"].as_str(), s["op_count"].as_u64()), (Some("p1"), Some(2)));
    assert!(s["diff"].as_str().unwrap().starts_with("--- base\n+++ proposal\n"));
    assert_eq!(erp.call("scene.save", json!({})).unwrap()["text"].as_str().unwrap(), original, "still untouched");
    let preview = m.call_tool("get_entity", json!({"entity": body, "proposal_id": "p1"}));
    assert_eq!(structured(&preview)["entity"]["name"], "hero");

    // -- verify, with acceptance checks
    let verified = m.call_tool(
        "verify_proposal",
        json!({"proposal_id": "p1", "inputs": {"kind": "bot", "ticks": 120, "seed": 5}, "sample_every": 30,
               "checks": ["lost_bodies.max == 0", "dynamic_bodies.start == 40", "mean_height >= 0"]}),
    );
    assert_eq!(verified["isError"], false, "{verified}");
    let vt = McpChild::text(&verified);
    assert!(vt.contains("CHECKS PASSED: 3/3"), "{vt}");
    assert!(vt.contains("[pass] lost_bodies.max == 0"), "{vt}");
    assert!(vt.contains("replayed 120 ticks"), "{vt}");
    assert!(vt.contains("Metrics that differ"), "{vt}");
    assert!(vt.len() < 2500, "the report is compact ({} bytes): {vt}", vt.len());
    let report = structured(&verified);
    assert_eq!(report["checks"]["passed"], true);
    assert_eq!(report["proposal"], "p1");
    assert!(report["checksums"]["candidate_final"].as_str().unwrap().starts_with("0x"));
    assert_eq!(erp.call("scene.save", json!({})).unwrap()["text"].as_str().unwrap(), original, "verifying changes nothing");

    // -- accept
    let accepted = m.call_tool("accept_proposal", json!({"proposal_id": "p1", "verified_state": report["verified_state"]}));
    assert_eq!(accepted["isError"], false, "{accepted}");
    let at = McpChild::text(&accepted);
    assert!(at.contains("Accepted p1 \"lift body_05\" by agent:claude"), "{at}");
    assert!(structured(&accepted)["history_id"].is_u64());

    // -- history shows the agent's entry
    let hist = m.call_tool("history", json!({}));
    let ht = McpChild::text(&hist);
    assert!(ht.contains("\"lift body_05\"") && ht.contains("agent:claude"), "{ht}");
    let entries = structured(&hist)["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!((entries[0]["origin"].as_str(), entries[0]["op_count"].as_u64()), (Some("agent:claude"), Some(2)));
    // The host agrees (looked at directly).
    let direct = erp.call("history.list", J::Null).unwrap();
    assert_eq!(direct["entries"], structured(&hist)["entries"]);
    assert_eq!(erp.call("world.get", json!({"entity": body})).unwrap()["entity"]["name"], "hero");
    let scene = m.request("resources/read", json!({"uri": "orrery://scene"}));
    assert!(scene["contents"][0]["text"].as_str().unwrap().contains("name: hero"));

    // -- undo reverts exactly
    let undone = m.call_tool("undo", json!({}));
    assert!(McpChild::text(&undone).contains("Undid entry"), "{undone}");
    assert_eq!(erp.call("scene.save", json!({})).unwrap()["text"].as_str().unwrap(), original);
    assert!(McpChild::text(&m.call_tool("list_proposals", json!({}))).contains("No open proposals"));

    m.assert_stdout_is_only_json_rpc();
    assert!(m.stderr().contains("orr_mcp"), "the log goes to stderr: {:?}", m.stderr());
    assert_eq!(m.finish(), Some(0), "the server exits cleanly when stdin closes");
}

#[test]
fn a_failing_check_is_reported_and_the_agent_rejects() {
    let host = TestHost::start();
    let mut erp = host.erp("claude-tok");
    let original = erp.call("scene.save", json!({})).unwrap()["text"].as_str().unwrap().to_string();
    let mut m = McpChild::against(&host, "claude-tok", &[]);
    m.initialize();
    let body = guid_from_overview(&mut m, "body_05");

    let proposed = m.call_tool("propose_changes", json!({"label": "sink body_05", "ops": [{"op": "patch", "entity": body, "component": BODY, "path": "pos", "value": [0, -30]}]}));
    assert_eq!(proposed["isError"], false);
    let verified = m.call_tool("verify_proposal", json!({"proposal_id": "p1", "inputs": {"kind": "bot", "ticks": 60}, "checks": ["lost_bodies.max == 0", "dynamic_bodies.start == 40"]}));
    // A failed check is a result, not a tool error.
    assert_eq!(verified["isError"], false);
    let vt = McpChild::text(&verified);
    assert!(vt.contains("CHECKS FAILED: 1/2 passed"), "{vt}");
    assert!(vt.contains("[FAIL] lost_bodies.max == 0"), "{vt}");
    assert!(vt.contains("lost_bodies.max is 1"), "{vt}");
    assert!(vt.contains("[pass] dynamic_bodies.start == 40"), "{vt}");
    assert_eq!(structured(&verified)["checks"]["passed"], false);
    assert_eq!(structured(&verified)["passed"], false);

    let rejected = m.call_tool("reject_proposal", json!({"proposal_id": "p1"}));
    assert_eq!(rejected["isError"], false);
    assert!(McpChild::text(&rejected).contains("Rejected p1"));
    assert!(McpChild::text(&m.call_tool("list_proposals", json!({}))).contains("No open proposals"));
    assert!(McpChild::text(&m.call_tool("history", json!({}))).contains("History is empty"));
    assert_eq!(erp.call("scene.save", json!({})).unwrap()["text"].as_str().unwrap(), original);
    m.assert_stdout_is_only_json_rpc();
}

#[test]
fn baseline_verification_and_the_last_play_recording() {
    let host = TestHost::start();
    let mut m = McpChild::against(&host, "claude-tok", &[]);
    m.initialize();
    // Omitting proposal_id runs the scene alone.
    let base = m.call_tool("verify_proposal", json!({"checks": ["lost_bodies.max == 0", "no_divergence"]}));
    assert_eq!(base["isError"], false, "{base}");
    let bt = McpChild::text(&base);
    assert!(bt.contains("Baseline (scene alone)") && bt.contains("CHECKS PASSED: 2/2"), "{bt}");
    assert!(bt.contains("mean_height:"), "{bt}");
    // No recording yet.
    let none = m.call_tool("verify_proposal", json!({"inputs": {"kind": "last_play"}}));
    assert_eq!(none["isError"], true);
    assert!(McpChild::text(&none).contains("no play session has been stopped"), "{none}");
    // Play, stop, verify against the recording.
    let started = m.call_tool("sim_run", json!({"action": "start"}));
    assert!(McpChild::text(&started).contains("Play session"), "{started}");
    let stepped = m.call_tool("sim_run", json!({"action": "step", "n": 45}));
    assert!(McpChild::text(&stepped).contains("tick 45"), "{stepped}");
    let stopped = m.call_tool("sim_run", json!({"action": "stop"}));
    assert!(McpChild::text(&stopped).contains("last_play"), "{stopped}");
    let rec = m.call_tool("verify_proposal", json!({"inputs": {"kind": "last_play"}, "checks": ["recording_matches"]}));
    assert_eq!(rec["isError"], false, "{rec}");
    assert!(McpChild::text(&rec).contains("CHECKS PASSED: 1/1"), "{rec}");
    assert!(McpChild::text(&m.call_tool("sim_run", json!({"action": "state"}))).contains("Edit mode"));
    // A bad action is a tool error.
    let bad = m.call_tool("sim_run", json!({"action": "explode"}));
    assert_eq!(bad["isError"], true);
}

#[test]
fn tool_errors_are_results_with_is_error_and_the_engine_message() {
    let host = TestHost::start();
    let mut limited = McpChild::against(&host, "limited-tok", &[]);
    limited.initialize();
    let body = guid_from_overview(&mut limited, "body_02");

    // Propose and verify work without `approve`; accept is refused with the reason.
    let p = limited.call_tool("propose_changes", json!({"label": "nudge", "ops": [{"op": "patch", "entity": body, "component": BODY, "path": "vel", "value": [1, 0]}]}));
    assert_eq!(p["isError"], false, "{p}");
    let v = limited.call_tool("verify_proposal", json!({"proposal_id": "p1", "inputs": {"kind": "idle", "ticks": 20}}));
    assert_eq!(v["isError"], false);
    let a = limited.call_tool("accept_proposal", json!({"proposal_id": "p1"}));
    assert_eq!(a["isError"], true);
    let at = McpChild::text(&a);
    assert!(at.contains("approve") && at.contains("permission_denied"), "{at}");
    assert!(a.get("structuredContent").is_none());
    // No sim_control either.
    assert_eq!(limited.call_tool("sim_run", json!({"action": "start"}))["isError"], true);

    // A bad op on a new proposal: an error, and no half-made proposal is left behind.
    let bad = limited.call_tool("propose_changes", json!({"label": "bad", "ops": [{"op": "despawn", "entity": "e_deadbeef"}]}));
    assert_eq!(bad["isError"], true);
    let bt = McpChild::text(&bad);
    assert!(bt.contains("e_deadbeef") && bt.contains("Nothing was staged"), "{bt}");
    let list = McpChild::text(&limited.call_tool("list_proposals", json!({})));
    assert!(list.contains("1 open proposal") && list.contains("p1") && !list.contains("p2"), "{list}");

    // Bad arguments.
    for (tool, args) in [
        ("propose_changes", json!({})),
        ("propose_changes", json!({"ops": "rename"})),
        ("get_entity", json!({})),
        ("get_entity", json!({"entity": "e_deadbeef"})),
        ("get_entity", json!({"entity": 5})),
        ("get_schema", json!({"type": "Nope"})),
        ("accept_proposal", json!({"proposal_id": "p77"})),
        ("verify_proposal", json!({"inputs": {"kind": "bot", "ticks": 0}})),
        ("verify_proposal", json!({"proposal_id": "p1", "checks": ["lost_bodies.max"]})),
        ("list_proposals", json!({"proposal_id": "zzz"})),
    ] {
        let r = limited.call_tool(tool, args.clone());
        assert_eq!(r["isError"], true, "{tool} {args}: {r}");
        assert!(!McpChild::text(&r).is_empty());
    }
    // The connection still works after all those errors.
    assert_eq!(limited.call_tool("history", json!({}))["isError"], false);
    limited.assert_stdout_is_only_json_rpc();
}

#[test]
fn protocol_errors_are_json_rpc_errors_and_the_server_carries_on() {
    let host = TestHost::start();
    let mut m = McpChild::against(&host, "claude-tok", &[]);

    // Before initialize: only ping (and initialize) work.
    assert_eq!(m.request("ping", json!({})), json!({}));
    let e = m.request_raw("tools/list", json!({}));
    assert_eq!(e["error"]["code"], -32002, "{e}");
    assert!(e["error"]["message"].as_str().unwrap().contains("initialize"));
    assert_eq!(m.request_raw("tools/call", json!({"name": "history"}))["error"]["code"], -32002);
    assert_eq!(m.request_raw("resources/list", json!({}))["error"]["code"], -32002);
    // initialize without a version.
    assert_eq!(m.request_raw("initialize", json!({}))["error"]["code"], -32602);
    assert_eq!(m.request_raw("initialize", json!({"protocolVersion": 3}))["error"]["code"], -32602);
    // An unknown revision gets ours.
    let old = m.request("initialize", json!({"protocolVersion": "1999-01-01", "capabilities": {}, "clientInfo": {"name": "x"}}));
    assert_eq!(old["protocolVersion"], "2025-06-18");
    // An older revision we speak is echoed.
    let older = m.request("initialize", json!({"protocolVersion": "2025-03-26", "capabilities": {}}));
    assert_eq!(older["protocolVersion"], "2025-03-26");

    // Notifications get no reply: the next reply is the ping's.
    m.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    m.send(&json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 1}}));
    m.send(&json!({"jsonrpc": "2.0", "method": "notifications/whatever"}));
    m.send(&json!({"jsonrpc": "2.0", "method": "no/such/notification"}));
    assert_eq!(m.request("ping", json!({})), json!({}));

    // Unknown method, bad params.
    let e = m.request_raw("prompts/list", json!({}));
    assert_eq!(e["error"]["code"], -32601, "{e}");
    assert_eq!(m.request_raw("tools/call", json!({}))["error"]["code"], -32602);
    assert_eq!(m.request_raw("tools/call", json!({"name": 5}))["error"]["code"], -32602);
    assert_eq!(m.request_raw("tools/call", json!({"name": "no_such_tool"}))["error"]["code"], -32602);
    assert_eq!(m.request_raw("tools/call", json!({"name": "history", "arguments": "x"}))["error"]["code"], -32602);
    assert_eq!(m.request_raw("tools/call", json!([1, 2]))["error"]["code"], -32602);
    assert_eq!(m.request_raw("resources/read", json!({}))["error"]["code"], -32602);
    let e = m.request_raw("resources/read", json!({"uri": "orrery://nope"}));
    assert_eq!(e["error"]["code"], -32002, "{e}");
    assert_eq!(m.request_raw("notifications/initialized", json!({}))["error"]["code"], -32601, "a notification with an id is not a notification");

    // Malformed lines.
    m.send_raw("this is not json");
    let e = m.recv();
    assert_eq!((e["error"]["code"].as_i64(), &e["id"]), (Some(-32700), &J::Null));
    m.send_raw("");
    m.send_raw("[1,2,3]");
    let e = m.recv();
    assert_eq!(e["error"]["code"], -32600);
    assert!(e["error"]["message"].as_str().unwrap().contains("batch"));
    m.send_raw("42");
    assert_eq!(m.recv()["error"]["code"], -32600);
    m.send_raw(r#"{"jsonrpc":"2.0","id":9}"#);
    let e = m.recv();
    assert_eq!((e["error"]["code"].as_i64(), e["id"].as_i64()), (Some(-32600), Some(9)));
    m.send_raw(r#"{"jsonrpc":"2.0","id":{"a":1},"method":"ping"}"#);
    assert_eq!(m.recv()["error"]["code"], -32600);
    m.send_raw(r#"{"jsonrpc":"1.0","id":10,"method":"ping"}"#);
    assert_eq!(m.recv()["error"]["code"], -32600);
    m.send_raw(r#"{"jsonrpc":"2.0","id":11,"method":"ping","params":[1]}"#);
    let e = m.recv();
    assert_eq!((e["error"]["code"].as_i64(), e["id"].as_i64()), (Some(-32602), Some(11)));
    // A response to a request of ours is ignored (we never ask the client anything).
    m.send_raw(r#"{"jsonrpc":"2.0","id":12,"result":{}}"#);
    // String ids are echoed as strings.
    m.send_raw(r#"{"jsonrpc":"2.0","id":"abc","method":"ping"}"#);
    let r = m.recv();
    assert_eq!((&r["id"], &r["result"]), (&json!("abc"), &json!({})));
    // Non-UTF-8 bytes do not kill the server.
    m.send_raw("{\"jsonrpc\":\"2.0\",\"id\":13,\"method\":\"ping\"}\u{fffd}");
    let e = m.recv();
    assert_eq!(e["error"]["code"], -32700);

    // Still fully working afterwards.
    let ok = m.call_tool("scene_overview", json!({"limit": 2}));
    assert_eq!(ok["isError"], false);
    assert!(m.recv_within(Duration::from_millis(300)).is_none(), "no stray messages");
    m.assert_stdout_is_only_json_rpc();
    assert_eq!(m.finish(), Some(0));
}

#[test]
fn tool_groups_switch_tools_on_and_off() {
    let host = TestHost::start();
    let mut m = McpChild::against(&host, "claude-tok", &["--tools", "scene,verify"]);
    m.initialize();
    let list = m.request("tools/list", json!({}));
    let names: Vec<&str> = list["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["scene_overview", "get_entity", "get_schema", "verify_proposal"]);
    // A tool of a group that is off is unknown.
    assert_eq!(m.request_raw("tools/call", json!({"name": "propose_changes", "arguments": {"ops": []}}))["error"]["code"], -32602);
    assert_eq!(m.call_tool("scene_overview", json!({"limit": 1}))["isError"], false);
    // The guide describes only the groups that are on.
    let agents = m.request("resources/read", json!({"uri": "orrery://agents"}));
    let text = agents["contents"][0]["text"].as_str().unwrap();
    assert!(text.contains("| `verify_proposal` |") && !text.contains("| `propose_changes` |"));
    drop(m);

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_orr_mcp")).args(["--tools", "scene,bogus"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown tool group 'bogus'"), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(out.stdout.is_empty(), "usage errors go to stderr");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_orr_mcp")).args(["--erp", "http://x"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn resources_serve_the_scene_the_schema_and_the_guide() {
    let host = TestHost::start();
    let mut m = McpChild::against(&host, "claude-tok", &[]);
    m.initialize();
    let list = m.request("resources/list", json!({}));
    let uris: Vec<&str> = list["resources"].as_array().unwrap().iter().map(|r| r["uri"].as_str().unwrap()).collect();
    assert_eq!(uris, vec!["orrery://scene", "orrery://schema", "orrery://agents"]);
    assert_eq!(m.request("resources/templates/list", json!({}))["resourceTemplates"], json!([]));

    let scene = m.request("resources/read", json!({"uri": "orrery://scene"}));
    assert_eq!(scene["contents"][0]["text"].as_str().unwrap(), demo_text());
    assert_eq!(scene["contents"][0]["mimeType"], "application/yaml");
    let schema = m.request("resources/read", json!({"uri": "orrery://schema"}));
    let parsed: J = serde_json::from_str(schema["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert!(parsed["$defs"]["orr_physics::Body"].is_object());
    let agents = m.request("resources/read", json!({"uri": "orrery://agents"}));
    let text = agents["contents"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("# Orrery: guide for AI agents"));
    assert_eq!(agents["contents"][0]["mimeType"], "text/markdown");
    assert_eq!(text, std::fs::read_to_string(AGENTS_PATH).unwrap(), "the resource is the committed docs/AGENTS.md");
    m.assert_stdout_is_only_json_rpc();
}

#[test]
fn the_server_starts_without_a_host_and_reports_it_per_tool() {
    // Nothing listens on port 1.
    let mut m = McpChild::start(&["--erp", "ws://127.0.0.1:1", "--token", "x"]);
    m.initialize();
    assert!(m.request("tools/list", json!({}))["tools"].as_array().unwrap().len() >= 10, "listing tools needs no host");
    let r = m.call_tool("scene_overview", json!({}));
    assert_eq!(r["isError"], true);
    let t = McpChild::text(&r);
    assert!(t.contains("cannot talk to the engine at ws://127.0.0.1:1"), "{t}");
    // Again: it tries again rather than staying broken.
    assert_eq!(m.call_tool("history", json!({}))["isError"], true);
    let e = m.request_raw("resources/read", json!({"uri": "orrery://scene"}));
    assert_eq!(e["error"]["code"], -32603);
    // A wrong token is reported the same way.
    let host = TestHost::start();
    let mut w = McpChild::against(&host, "wrong-token", &[]);
    w.initialize();
    let r = w.call_tool("scene_overview", json!({}));
    assert_eq!(r["isError"], true);
    assert!(McpChild::text(&r).contains("cannot talk to the engine"), "{r}");
    w.assert_stdout_is_only_json_rpc();
}

#[test]
fn the_token_can_come_from_the_environment() {
    let host = TestHost::start();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_orr_mcp"))
        .args(["--erp", &host.url])
        .env("ORR_ERP_TOKEN", "claude-tok")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().unwrap();
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-06-18"}}}}"#).unwrap();
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"history","arguments":{{}}}}}}"#).unwrap();
    }
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    let lines: Vec<J> = String::from_utf8(out.stdout).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[1]["result"]["isError"], false, "{}", lines[1]);
}

#[test]
fn guarded_accept_forwards_verification_state_and_never_falls_back_to_manual_accept() {
    let host = TestHost::start();
    let mut m = McpChild::against(&host, "claude-tok", &[]);
    m.initialize();
    let body = guid_from_overview(&mut m, "body_05");
    m.call_tool("propose_changes", json!({"label": "lift", "ops": [{"op": "patch", "entity": body, "component": BODY, "path": "pos", "value": [0, 12]}]}));
    let report = m.call_tool("verify_proposal", json!({"proposal_id": "p1", "inputs": {"kind": "idle", "ticks": 1}, "checks": ["lost_bodies.max == 0"]}));
    assert_eq!(structured(&report)["passed"], true);
    let mut erp = host.erp("claude-tok");
    erp.call("world.rename", json!({"entity": body, "name": "changed"})).unwrap();
    let before = erp.call("scene.save", json!({})).unwrap()["text"].clone();
    let refused = m.call_tool("accept_proposal", json!({"proposal_id": "p1", "verified_state": structured(&report)["verified_state"]}));
    assert_eq!(refused["isError"], true);
    assert!(McpChild::text(&refused).contains("stale_verification"));
    assert_eq!(erp.call("scene.save", json!({})).unwrap()["text"], before);
    assert_eq!(erp.call("proposal.list", J::Null).unwrap()["proposals"].as_array().unwrap().len(), 1);
    let null_state = m.call_tool("accept_proposal", json!({"proposal_id": "p1", "verified_state": null}));
    assert_eq!(null_state["isError"], true, "an explicitly invalid state must not select manual acceptance");
    let manual = m.call_tool("accept_proposal", json!({"proposal_id": "p1"}));
    assert_eq!(manual["isError"], false, "explicit manual acceptance keeps its existing rebase semantics");
}
