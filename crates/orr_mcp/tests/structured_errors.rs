//! Bounded tool-failure projection, independent of legacy text and public wrappers.
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use orr_mcp::{Bridge, ErpCall, Fail, McpServer, Style, ToolGroups};
use orr_remote::{ClientError, RpcError};
use serde_json::{json, Value as J};

type Calls = Rc<RefCell<Vec<String>>>;
type Script = Rc<RefCell<VecDeque<(&'static str, Reply)>>>;

#[derive(Clone)]
enum Reply {
    Ok(J),
    Rpc(RpcError),
    Transport(&'static str),
    Protocol(&'static str),
}

struct Fake {
    script: Script,
    calls: Calls,
}

impl ErpCall for Fake {
    fn call(&mut self, method: &str, _params: J) -> Result<J, ClientError> {
        self.calls.borrow_mut().push(method.to_string());
        let (expected, reply) = self.script.borrow_mut().pop_front().expect("unexpected ERP call (or retry)");
        assert_eq!(method, expected);
        match reply {
            Reply::Ok(value) => Ok(value),
            Reply::Rpc(error) => Err(ClientError::Rpc(error)),
            Reply::Transport(text) => Err(ClientError::Transport(text.into())),
            Reply::Protocol(text) => Err(ClientError::Protocol(text.into())),
        }
    }
}

fn bridge(replies: Vec<(&'static str, Reply)>) -> (Bridge, Calls, Rc<Cell<usize>>) {
    let script = Rc::new(RefCell::new(VecDeque::from(replies)));
    let calls = Rc::new(RefCell::new(Vec::new()));
    let connects = Rc::new(Cell::new(0));
    let (log, attempts) = (calls.clone(), connects.clone());
    let bridge = Bridge::new(
        "fake",
        Box::new(move || {
            attempts.set(attempts.get() + 1);
            Ok(Box::new(Fake { script: script.clone(), calls: log.clone() }))
        }),
    );
    (bridge, calls, connects)
}

fn initialize(server: &mut McpServer, version: &str) -> J {
    server.handle(&json!({"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{"protocolVersion":version}})).unwrap()["result"].clone()
}

fn call(server: &mut McpServer, name: &str, arguments: J) -> J {
    let response = server.handle(&json!({"jsonrpc":"2.0", "id":2, "method":"tools/call", "params":{"name":name, "arguments":arguments}})).unwrap();
    assert!(response.get("error").is_none(), "tool errors must not become protocol errors: {response}");
    response["result"].clone()
}

fn expected(code: i64, kind: Option<&str>) -> J {
    let mut error = json!({"origin":"erp", "code":code});
    if let Some(kind) = kind {
        error["data"] = json!({"kind":kind});
    }
    json!({"error":error})
}

fn assert_details(result: &J, code: i64, kind: Option<&str>) {
    assert_eq!(result["isError"], true);
    let structured = expected(code, kind);
    assert_eq!(result["structuredContent"], structured);
    assert_eq!(result["content"].as_array().unwrap().len(), 2);
    assert_eq!(result["content"][1], json!({"type":"text", "text":structured.to_string()}));
}

fn assert_text_only(result: &J) {
    assert_eq!(result["isError"], true);
    assert!(result.get("structuredContent").is_none());
    assert_eq!(result["content"].as_array().unwrap().len(), 1);
}

fn rpc(code: i64, kind: &str) -> Reply {
    Reply::Rpc(RpcError::new(code, kind, "ordinary engine explanation"))
}

#[test]
fn negotiated_versions_keep_legacy_text_and_gate_only_error_structure() {
    for version in ["2025-06-18", "2025-03-26", "2024-11-05", "unrecognized"] {
        let error = RpcError::new(orr_remote::PERMISSION_DENIED, "permission_denied", "approval unavailable");
        let text = orr_mcp::bridge::rpc_fail(&error).0;
        let (bridge, calls, _) = bridge(vec![("registry.input", Reply::Rpc(error))]);
        let mut server = McpServer::new(bridge, ToolGroups::ALL);
        let negotiated = initialize(&mut server, version);
        let result = call(&mut server, "get_schema", json!({"input":true}));
        assert_eq!(result["content"][0], json!({"type":"text", "text":text}));
        if negotiated["protocolVersion"] == "2025-06-18" {
            assert_details(&result, orr_remote::PERMISSION_DENIED, Some("permission_denied"));
        } else {
            assert_eq!(negotiated["protocolVersion"], version);
            assert_text_only(&result);
        }
        assert_eq!(calls.borrow().as_slice(), ["registry.input"]);
    }
}

#[test]
fn arbitrary_metadata_is_never_copied_and_code_is_the_received_typed_code() {
    let private = "secret_token_7_private";
    for data in [
        None,
        Some(J::Null),
        Some(json!([])),
        Some(json!({})),
        Some(json!({"kind":123})),
        Some(json!({"kind":["permission_denied"]})),
        Some(json!({"kind":private})),
        Some(json!({"kind":"x".repeat(100_000)})),
        Some(json!({"kind":"permission_denied-secret", "nested":{"token":private}})),
        Some(json!({"kind":"ws://user:password@host/path?token=secret#fragment"})),
    ] {
        let (bridge, _, _) = bridge(vec![("registry.input", Reply::Rpc(RpcError { code: 971, message: "explanation".into(), data }))]);
        let mut server = McpServer::new(bridge, ToolGroups::ALL);
        initialize(&mut server, "2025-06-18");
        assert_details(&call(&mut server, "get_schema", json!({"input":true})), 971, None);
    }
    let malicious = RpcError::new(orr_remote::PERMISSION_DENIED, "permission_denied", "ordinary explanation")
        .with("token", json!(private))
        .with("path", json!("/private/file"))
        .with("owner", json!("private owner"))
        .with("required", json!("approve"))
        .with("max", json!(400))
        .with("retryable", json!(true))
        .with("params", json!({"url":"https://private.example/secret"}));
    let (bridge, _, _) = bridge(vec![("registry.input", Reply::Rpc(malicious))]);
    let mut server = McpServer::new(bridge, ToolGroups::ALL);
    initialize(&mut server, "2025-06-18");
    let result = call(&mut server, "get_schema", json!({"input":true}));
    assert_details(&result, orr_remote::PERMISSION_DENIED, Some("permission_denied"));
    assert!(!result.to_string().contains(private));
}

#[test]
fn erp_internal_error_is_a_tool_failure_without_no_change_or_retry_claims() {
    let (bridge, _, _) = bridge(vec![("registry.input", rpc(orr_remote::INTERNAL_ERROR, "panic"))]);
    let mut server = McpServer::new(bridge, ToolGroups::ALL);
    initialize(&mut server, "2025-06-18");
    let result = call(&mut server, "get_schema", json!({"input":true}));
    assert_details(&result, orr_remote::INTERNAL_ERROR, Some("panic"));
    for forbidden in ["no_change", "retryable", "safe_retry", "safe_to_retry", "Nothing"] {
        assert!(!result.to_string().contains(forbidden));
    }
}

#[test]
fn primary_apply_error_survives_cleanup_success_failure_and_disconnect() {
    for cleanup in [Reply::Ok(json!({"ok":true})), rpc(orr_remote::PERMISSION_DENIED, "permission_denied"), Reply::Transport("closed")] {
        let cleanup_ok = matches!(&cleanup, Reply::Ok(_));
        let (bridge, calls, _) = bridge(vec![
            ("proposal.begin", Reply::Ok(json!({"id":"p1"}))),
            ("proposal.apply", rpc(orr_remote::NOT_FOUND, "unknown_entity")),
            ("proposal.reject", cleanup),
            ("registry.input", Reply::Ok(json!({"schema":{}}))),
        ]);
        let mut server = McpServer::new(bridge, ToolGroups::ALL);
        initialize(&mut server, "2025-06-18");
        let result = call(&mut server, "propose_changes", json!({"ops":[{"op":"despawn","entity":"e_deadbeef"}]}));
        assert_details(&result, orr_remote::NOT_FOUND, Some("unknown_entity"));
        let text = result["content"][0]["text"].as_str().unwrap();
        if cleanup_ok {
            assert!(text.contains("Nothing was staged; the proposal was discarded."));
        } else {
            assert!(text.contains("cleanup was not confirmed"));
            assert!(!text.contains("was discarded") && !text.contains("Nothing was staged"));
        }
        assert_text_only(&call(&mut server, "sim_run", json!({"action":"invalid"})));
        assert_eq!(call(&mut server, "get_schema", json!({"input":true}))["structuredContent"], json!({"schema":{}}));
        assert_eq!(calls.borrow().as_slice(), ["proposal.begin", "proposal.apply", "proposal.reject", "registry.input"]);
    }
}

#[test]
fn uncertain_apply_never_claims_it_did_not_stage_or_reexecutes_the_apply() {
    for created in [true, false] {
        for primary in [Reply::Transport("timed out waiting for response"), rpc(orr_remote::INTERNAL_ERROR, "panic")] {
            let received_rpc = matches!(&primary, Reply::Rpc(_));
            let mut replies = Vec::new();
            if created {
                replies.push(("proposal.begin", Reply::Ok(json!({"id":"p1"}))));
            }
            replies.push(("proposal.apply", primary));
            if created {
                replies.push(("proposal.reject", Reply::Ok(json!({"ok":true}))));
            }
            let (bridge, calls, _) = bridge(replies);
            let mut server = McpServer::new(bridge, ToolGroups::ALL);
            initialize(&mut server, "2025-06-18");
            let mut args = json!({"ops":[]});
            if !created {
                args["proposal_id"] = json!("p1");
            }
            let result = call(&mut server, "propose_changes", args);
            if received_rpc {
                assert_details(&result, orr_remote::INTERNAL_ERROR, Some("panic"));
            } else {
                assert_text_only(&result);
            }
            let text = result["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("apply outcome was not confirmed"));
            assert!(!text.contains("Nothing was staged") && !text.contains("None of the ops"));
            assert_eq!(calls.borrow().iter().filter(|method| method.as_str() == "proposal.apply").count(), 1);
        }
    }
}

#[test]
fn ignored_preflight_error_cannot_contaminate_success_or_later_local_error() {
    let (bridge, calls, _) = bridge(vec![
        ("proposal.list", rpc(orr_remote::LIMIT_EXCEEDED, "busy")),
        ("proposal.accept", Reply::Ok(json!({"applied":1, "history_id":4, "checksum":"0x1"}))),
    ]);
    let mut server = McpServer::new(bridge, ToolGroups::ALL);
    initialize(&mut server, "2025-06-18");
    let success = call(&mut server, "accept_proposal", json!({"proposal_id":"p1"}));
    assert_eq!(
        success,
        json!({
            "content":[{"type":"text", "text":"Accepted p1: applied 1 op(s) as history entry #4. Scene checksum 0x1. Use `undo` to take it back.\n"}],
            "structuredContent":{"applied":1,"history_id":4,"checksum":"0x1"},"isError":false
        })
    );
    assert_text_only(&call(&mut server, "accept_proposal", json!({})));
    assert_eq!(calls.borrow().as_slice(), ["proposal.list", "proposal.accept"]);
}

#[test]
fn setup_transport_timeout_and_protocol_failures_have_no_fabricated_erp_cause() {
    for reply in [Reply::Transport("closed"), Reply::Transport("timed out waiting for response"), Reply::Protocol("invalid server response")] {
        let (bridge, calls, connects) = bridge(vec![("registry.input", reply), ("registry.input", Reply::Ok(json!({"schema":{}})))]);
        let mut server = McpServer::new(bridge, ToolGroups::ALL);
        initialize(&mut server, "2025-06-18");
        assert_text_only(&call(&mut server, "get_schema", json!({"input":true})));
        assert_eq!(connects.get(), 1);
        assert_eq!(calls.borrow().len(), 1, "no retry of a possibly executed call");
        assert_text_only(&call(&mut server, "get_entity", json!({})));
        assert_eq!(call(&mut server, "get_schema", json!({"input":true}))["isError"], false);
        assert_eq!(connects.get(), 2, "a later explicit call can reconnect");
    }
    for why in ["connection refused", "[-32001] token refused"] {
        let bridge = Bridge::new("fake", Box::new(move || Err(why.into())));
        let mut server = McpServer::new(bridge, ToolGroups::ALL);
        initialize(&mut server, "2025-06-18");
        assert_text_only(&call(&mut server, "get_schema", json!({"input":true})));
    }
}

#[test]
fn public_fail_tuple_and_wrapper_signatures_remain_usable() {
    let Fail(text) = Fail("legacy tuple".into());
    assert_eq!(Fail::new(&text).0, "legacy tuple");
    let _: fn(&mut Bridge, &str, J) -> Result<J, Fail> = Bridge::call;
    type RunTool = fn(&mut Bridge, &str, &serde_json::Map<String, J>) -> Result<orr_mcp::tools::Out, Fail>;
    let _: RunTool = orr_mcp::tools::run;
    let _: fn(&mut Bridge, &str, J, Option<&str>) -> Result<orr_mcp::tools::Staged, Fail> = orr_mcp::tools::stage_proposal;
    let e = RpcError::new(orr_remote::NOT_FOUND, "unknown_entity", "missing");
    assert_eq!(orr_mcp::bridge::rpc_fail(&e).0, "missing [unknown_entity] Look the name up with scene_overview or get_schema.");
    assert_eq!(orr_mcp::bridge::rpc_fail_styled(&e, Style::Cli).0, "missing [unknown_entity] Look the name up with `orr scene` or `orr schema --types`.");
    let (mut bridge, _, _) = bridge(vec![("world.get", Reply::Rpc(e))]);
    bridge.style = Style::Cli;
    let fail = orr_mcp::tools::run(&mut bridge, "get_entity", json!({"entity":"missing"}).as_object().unwrap()).err().unwrap();
    assert_eq!(fail.0, "missing [unknown_entity] Look the name up with `orr scene` or `orr schema --types`.");
}

#[test]
fn actual_erp_verify_busy_is_preserved_while_other_calls_still_work() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc, Condvar, Mutex};
    use std::time::Duration;

    use orr_remote::{Auth, Caps, ErpClient, GameHooks, LocalHost, ServerConfig};
    use orr_sample::physics_game::{register_reflect, PhysGame};
    use orr_sim::{Frame, MetricValue, Metrics, Simulation};

    // Gate only the first worker sample, not the host. RAII releases it even
    // after an assertion failure; the wait is bounded in both threads.
    struct Gate {
        entered: mpsc::Sender<()>,
        first: AtomicBool,
        released: Mutex<bool>,
        changed: Condvar,
    }
    struct GatedMetrics(Arc<Gate>);
    impl Metrics for GatedMetrics {
        fn sample(&self, _: &Frame) -> Vec<(String, MetricValue)> {
            if !self.0.first.swap(true, Ordering::Relaxed) {
                self.0.entered.send(()).unwrap();
                let released = self.0.released.lock().unwrap();
                let (released, _) = self.0.changed.wait_timeout_while(released, Duration::from_secs(10), |released| !*released).unwrap();
                assert!(*released, "test did not release the verification worker");
            }
            Vec::new()
        }
    }
    struct Release(Arc<Gate>);
    impl Drop for Release {
        fn drop(&mut self) {
            *self.0.released.lock().unwrap() = true;
            self.0.changed.notify_all();
        }
    }
    let (entered, signal) = mpsc::channel();
    let gate = Arc::new(Gate { entered, first: AtomicBool::new(false), released: Mutex::new(false), changed: Condvar::new() });
    let metrics = gate.clone();
    let host = LocalHost::spawn::<PhysGame>(move || {
        let mut types = orr_reflect::TypeRegistry::new();
        register_reflect(&mut types);
        let doc = orr_edit::EditorDoc::from_yaml(include_str!("../../../scenes/physics_demo.scene.yaml"), types, Simulation::<PhysGame>::build_registry(), 7)
            .unwrap();
        let mut cfg = ServerConfig::new(Auth::DevNoAuth);
        cfg.listen = false;
        cfg.limits.game = GameHooks::new("PhysGame").with_metrics(GatedMetrics(metrics));
        Ok((doc, cfg))
    })
    .unwrap();
    let release = Release(gate);
    let mut erp = ErpClient::with_transport(Box::new(host.connector().connect("observer", Caps::ALL).unwrap()));
    let before = erp.call("scene.save", json!({})).unwrap();
    let history = erp.call("history.list", J::Null).unwrap();
    let mut verifier = ErpClient::with_transport(Box::new(host.connector().connect("verifier", Caps::ALL).unwrap()));
    let worker = std::thread::spawn(move || verifier.call("verify.self", json!({"inputs":{"kind":"idle", "ticks":1}})));
    signal.recv_timeout(Duration::from_secs(10)).expect("verification worker entered");
    let connector = host.connector();
    let bridge = Bridge::new(
        "local ERP",
        Box::new(move || {
            let transport = connector.connect("mcp", Caps::ALL).map_err(|error| error.to_string())?;
            Ok(Box::new(ErpClient::with_transport(Box::new(transport))))
        }),
    );
    let mut server = McpServer::new(bridge, ToolGroups::ALL);
    initialize(&mut server, "2025-06-18");
    let busy = call(&mut server, "verify_proposal", json!({"inputs":{"kind":"idle", "ticks":1}}));
    assert_details(&busy, orr_remote::LIMIT_EXCEEDED, Some("verify_busy"));
    assert_eq!(call(&mut server, "history", json!({}))["structuredContent"], history);
    assert_eq!(erp.call("scene.save", json!({})).unwrap(), before);
    drop(release);
    assert!(worker.join().unwrap().is_ok());
    assert_eq!(erp.call("history.list", J::Null).unwrap(), history);
}
