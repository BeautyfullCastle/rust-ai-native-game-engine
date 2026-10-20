//! Read-only work-status MCP contract, including all negotiated versions.
//!
//! Scripted replies test the adapter, not editor-owner lifecycle semantics.
//! The socket tests use a real authenticated, deliberately unpaired ERP host.
//! Paired EditorApp publication is covered by the editor integration tests;
//! this target stays unconditional without enabling `orr_remote/sample-host`.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use orr_edit::EditorDoc;
use orr_mcp::{Bridge, ErpCall, McpServer, ToolGroups};
use orr_reflect::TypeRegistry;
use orr_remote::work_status::{
    self, DomainId, DomainRecord, EditorSnapshot, HostScene, Operations, WorkStatusService,
};
use orr_remote::{
    Auth, Caps, ClientError, ErpClient, LocalHost, RpcError, ServerConfig, TokenEntry,
};
use orr_sample::physics_game::{register_reflect, PhysGame};
use orr_sim::Simulation;
use serde_json::{json, Value as J};

const VERSIONS: [&str; 3] = ["2024-11-05", "2025-03-26", "2025-06-18"];
const TOOL: &str = "editor_work_status";
const METHOD: &str = "editor.work_status";
const PRIVATE: &str = "status_private_sentinel";

type Calls = Rc<RefCell<Vec<(String, J)>>>;
type Script = Rc<RefCell<VecDeque<(&'static str, Reply)>>>;

enum Reply {
    Ok(J),
    Rpc(RpcError),
    Transport(&'static str),
    Protocol(&'static str),
}

struct Scripted {
    script: Script,
    calls: Calls,
}

impl ErpCall for Scripted {
    fn call(&mut self, method: &str, params: J) -> Result<J, ClientError> {
        self.calls.borrow_mut().push((method.to_string(), params));
        let (expected, reply) = self
            .script
            .borrow_mut()
            .pop_front()
            .expect("unexpected ERP call or automatic retry");
        assert_eq!(method, expected);
        match reply {
            Reply::Ok(value) => Ok(value),
            Reply::Rpc(error) => Err(ClientError::Rpc(error)),
            Reply::Transport(text) => Err(ClientError::Transport(text.into())),
            Reply::Protocol(text) => Err(ClientError::Protocol(text.into())),
        }
    }
}

fn scripted(replies: Vec<(&'static str, Reply)>) -> (Bridge, Calls, Rc<Cell<usize>>) {
    let script = Rc::new(RefCell::new(VecDeque::from(replies)));
    let calls = Rc::new(RefCell::new(Vec::new()));
    let connects = Rc::new(Cell::new(0));
    let (log, attempts) = (calls.clone(), connects.clone());
    let bridge = Bridge::new(
        "scripted ERP",
        Box::new(move || {
            attempts.set(attempts.get() + 1);
            Ok(Box::new(Scripted {
                script: script.clone(),
                calls: log.clone(),
            }))
        }),
    );
    (bridge, calls, connects)
}

fn request(server: &mut McpServer, method: &str, params: J) -> J {
    server
        .handle(&json!({"jsonrpc":"2.0", "id":1, "method":method, "params":params}))
        .expect("request reply")
}

fn initialize(server: &mut McpServer, version: &str) {
    let reply = request(server, "initialize", json!({"protocolVersion":version}));
    assert_eq!(reply["result"]["protocolVersion"], version);
}

fn call(server: &mut McpServer, name: &str, args: J) -> J {
    let reply = request(server, "tools/call", json!({"name":name, "arguments":args}));
    assert!(
        reply.get("error").is_none(),
        "tool failures must not become outer protocol failures: {reply}"
    );
    reply["result"].clone()
}

fn text(result: &J) -> &str {
    result["content"][0]["text"].as_str().expect("tool text")
}

fn assert_text_failure(result: &J) {
    assert_eq!(result["isError"], true, "{result}");
    assert!(result.get("structuredContent").is_none(), "{result}");
    assert_eq!(result["content"].as_array().unwrap().len(), 1);
}

fn assert_erp_failure(result: &J, version: &str, error: &RpcError, kind: Option<&str>) {
    assert_eq!(result["isError"], true, "{result}");
    assert_eq!(text(result), orr_mcp::bridge::rpc_fail(error).0);
    if version == "2025-06-18" {
        let mut details = json!({"origin":"erp", "code":error.code});
        if let Some(kind) = kind {
            details["data"] = json!({"kind":kind});
        }
        let structured = json!({"error":details});
        assert_eq!(result["structuredContent"], structured);
        assert_eq!(result["content"].as_array().unwrap().len(), 2);
        assert_eq!(
            result["content"][1],
            json!({"type":"text", "text":structured.to_string()})
        );
    } else {
        assert_text_failure(result);
    }
}

fn clean_host() -> HostScene {
    HostScene {
        dirty: false,
        in_tx: false,
        preview_revision: 9_007_199_254_740_997,
        doc_checksum: 1,
    }
}

/// Use the actual finite renderer, not a separately invented wire schema.
fn observation(availability: &str, assessment: &str) -> J {
    let (service, mut owner) = WorkStatusService::pair();
    let value = match availability {
        "not_paired" => work_status::response(None, clean_host(), 1000),
        "not_ready" => work_status::response(Some(&service), clean_host(), 1000),
        "owner_absent" => {
            drop(owner);
            work_status::response(Some(&service), clean_host(), 1000)
        }
        "busy" => {
            // The public API deliberately has no lock-contention hook. Start
            // from a real absent-observation schema and change only the two
            // fixed lifecycle enums. Actual contention is a mailbox unit test.
            let mut value = work_status::response(Some(&service), clean_host(), 1000);
            value["availability"] = json!("busy");
            value["assessment"]["reasons"] = json!(["local_busy"]);
            value
        }
        "ready" | "incomplete" | "stale" => {
            let mut domains: Vec<_> = DomainId::ALL
                .into_iter()
                .map(DomainRecord::disabled)
                .collect();
            if assessment == "known_blockers" || availability == "stale" {
                domains[0] = DomainRecord::observed(DomainId::SpriteBindings, true);
            }
            if availability == "incomplete" {
                domains[1] = DomainRecord::unknown(DomainId::Tilemap);
            }
            assert!(owner.publish(EditorSnapshot {
                domains,
                operations: Operations::Idle
            }));
            let max_age = if availability == "stale" {
                std::thread::sleep(Duration::from_millis(3));
                1
            } else {
                1000
            };
            work_status::response(Some(&service), clean_host(), max_age)
        }
        _ => panic!("unsupported test lifecycle"),
    };
    assert_eq!(value["availability"], availability);
    assert_eq!(value["assessment"]["state"], assessment);
    value
}

#[test]
fn static_listing_is_scene_group_read_only_not_proof_of_a_live_owner() {
    let def = orr_mcp::tools::find(TOOL).expect("named work-status tool");
    assert_eq!(def.group, "scene");
    assert_eq!(def.needs, "read");
    assert!(def.read_only);
    for version in VERSIONS {
        let (bridge, calls, connects) = scripted(Vec::new());
        let mut server = McpServer::new(bridge, ToolGroups::parse("scene").unwrap());
        initialize(&mut server, version);
        let listed = request(&mut server, "tools/list", json!({}));
        let tool = listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == TOOL)
            .unwrap();
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        let schema = &tool["inputSchema"];
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["required"], json!([]));
        assert_eq!(schema["properties"].as_object().unwrap().len(), 1);
        assert_eq!(schema["properties"]["max_age_ms"]["type"], "integer");
        assert_eq!(schema["properties"]["max_age_ms"]["minimum"], 1);
        assert_eq!(schema["properties"]["max_age_ms"]["maximum"], 5000);
        assert_eq!(schema["properties"]["max_age_ms"]["default"], 1000);
        assert!(calls.borrow().is_empty());
        assert_eq!(connects.get(), 0, "listing must not add live discovery");

        let (bridge, calls, connects) = scripted(Vec::new());
        let mut excluded = McpServer::new(
            bridge,
            ToolGroups::parse("propose,verify,sim,history").unwrap(),
        );
        initialize(&mut excluded, version);
        let listed = request(&mut excluded, "tools/list", json!({}));
        assert!(!listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == TOOL));
        assert_eq!(
            request(
                &mut excluded,
                "tools/call",
                json!({"name":TOOL, "arguments":{}})
            )["error"]["code"],
            -32602
        );
        assert!(calls.borrow().is_empty());
        assert_eq!(connects.get(), 0);
    }
}

#[test]
fn every_status_is_success_and_preserves_the_erp_payload_in_all_versions() {
    for version in VERSIONS {
        let mut cases = vec![
            observation("ready", "none_observed"),
            observation("ready", "known_blockers"),
        ];
        for availability in [
            "not_ready",
            "stale",
            "owner_absent",
            "not_paired",
            "busy",
            "incomplete",
        ] {
            cases.push(observation(availability, "indeterminate"));
        }
        cases.push(observation("incomplete", "known_blockers"));
        for operations in [
            Operations::PendingMutation,
            Operations::OutcomeUncertain,
            Operations::Unknown,
        ] {
            let (service, mut owner) = WorkStatusService::pair();
            let domains = DomainId::ALL
                .into_iter()
                .map(DomainRecord::disabled)
                .collect();
            assert!(owner.publish(EditorSnapshot {
                domains,
                operations
            }));
            let value = work_status::response(Some(&service), clean_host(), 1000);
            assert_eq!(value["assessment"]["state"], "indeterminate");
            cases.push(value);
        }
        let (bridge, calls, connects) = scripted(
            cases
                .iter()
                .cloned()
                .map(|value| (METHOD, Reply::Ok(value)))
                .collect(),
        );
        let mut server = McpServer::new(bridge, ToolGroups::ALL);
        initialize(&mut server, version);
        let mut fixed_text = None;
        for expected in &cases {
            let result = call(&mut server, TOOL, json!({}));
            assert_eq!(result["isError"], false, "{result}");
            assert_eq!(
                &result["structuredContent"], expected,
                "do not collapse, recompute or coerce observations"
            );
            assert_eq!(result["content"].as_array().unwrap().len(), 1);
            let (message, payload) = text(&result)
                .split_once('\n')
                .expect("fixed observation warning followed by the complete JSON result");
            assert_eq!(
                &serde_json::from_str::<J>(payload).unwrap(),
                expected,
                "legacy clients must receive the same complete observation"
            );
            assert!(message.contains("none_observed"), "{message}");
            assert!(message.to_lowercase().contains("observation"), "{message}");
            assert!(
                message.contains("not") && (message.contains("close") || message.contains("exit")),
                "{message}"
            );
            if let Some(previous) = &fixed_text {
                assert_eq!(
                    message, previous,
                    "fixed warning must not reinterpret lifecycle states"
                );
            } else {
                fixed_text = Some(message.to_string());
            }
        }
        assert_eq!(
            calls.borrow().len(),
            cases.len(),
            "exactly one ERP call per requested observation"
        );
        assert_eq!(connects.get(), 1);
    }
}

#[test]
fn canonical_decimal_identities_remain_exact_above_javascript_integer_precision() {
    for version in VERSIONS {
        let mut expected = observation("ready", "none_observed");
        for (field, value) in [
            ("owner_binding_id", "9007199254740993"),
            ("snapshot_seq", "18446744073709551614"),
            ("ui_frame", "18446744073709551615"),
        ] {
            expected["observation"][field] = json!(value);
        }
        assert!(work_status::validate_response(&expected));
        let (bridge, calls, _) = scripted(vec![(METHOD, Reply::Ok(expected.clone()))]);
        let mut server = McpServer::new(bridge, ToolGroups::ALL);
        initialize(&mut server, version);
        let result = call(&mut server, TOOL, json!({}));
        assert_eq!(result["isError"], false);
        assert_eq!(result["structuredContent"], expected);
        let (_, payload) = text(&result).split_once('\n').unwrap();
        assert_eq!(serde_json::from_str::<J>(payload).unwrap(), expected);
        assert_eq!(calls.borrow().len(), 1);
    }
}

#[test]
fn hostile_success_payloads_fail_locally_without_echoing_private_data_or_retrying() {
    let canonical = observation("ready", "none_observed");
    assert!(work_status::validate_response(&canonical));
    let mut invalid = vec![J::Null, json!([]), json!(PRIVATE)];
    for parent in [
        "",
        "/host_scene",
        "/editor_local",
        "/editor_operations",
        "/observation",
        "/assessment",
    ] {
        let mut value = canonical.clone();
        value
            .pointer_mut(parent)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert(PRIVATE.into(), json!({"token":PRIVATE, "path":PRIVATE}));
        invalid.push(value);
    }
    let mut nested = canonical.clone();
    nested["editor_local"]["domains"][0]["path"] = json!(PRIVATE);
    invalid.push(nested);
    let mut missing = canonical.clone();
    missing.as_object_mut().unwrap().remove("assessment");
    invalid.push(missing);
    for (pointer, bad) in [
        ("/schema_version", json!(2)),
        ("/scope", json!(PRIVATE)),
        ("/target", json!(PRIVATE)),
        ("/availability", json!(PRIVATE)),
        ("/consistency", json!(PRIVATE)),
        ("/host_scene/owner", json!(PRIVATE)),
        ("/host_scene/authority", json!(PRIVATE)),
        ("/host_scene/dirty", json!(PRIVATE)),
        ("/host_scene/doc_checksum", json!(PRIVATE)),
        ("/host_scene/preview_revision", json!(PRIVATE)),
        ("/host_scene/preview_revision_scope", json!(PRIVATE)),
        ("/editor_local/owner", json!(PRIVATE)),
        ("/editor_local/coverage", json!(PRIVATE)),
        ("/editor_local/observation", json!(PRIVATE)),
        ("/editor_local/domains/0/domain_id", json!(PRIVATE)),
        ("/editor_local/domains/0/presence", json!(PRIVATE)),
        ("/editor_local/domains/0/work_state", json!(PRIVATE)),
        ("/editor_local/domains/0/persistence_scope", json!(PRIVATE)),
        ("/editor_local/domains/0/domain_revision", json!(PRIVATE)),
        ("/editor_operations/state", json!(PRIVATE)),
        ("/editor_operations/reason", json!(PRIVATE)),
        ("/editor_operations/observation", json!(PRIVATE)),
        ("/assessment/state", json!(PRIVATE)),
        ("/assessment/reasons", json!([PRIVATE])),
        ("/assessment/blockers", json!([PRIVATE])),
        (
            "/observation/owner_binding_id",
            json!(format!("https://{PRIVATE}@example.test/?token={PRIVATE}")),
        ),
        ("/observation/snapshot_seq", json!(9007199254740993_u64)),
        ("/observation/ui_frame", json!("18446744073709551616")),
        ("/observation/max_age_ms", json!(5001)),
        ("/observation/age_ms", json!(-1)),
        ("/observation/binding_scope", json!(PRIVATE)),
        ("/observation/reconnect_policy", json!(PRIVATE)),
    ] {
        let mut value = canonical.clone();
        *value.pointer_mut(pointer).unwrap() = bad;
        invalid.push(value);
    }
    let mut oversized = canonical.clone();
    oversized["assessment"]["reasons"] = json!(vec!["local_stale"; 2000]);
    assert!(oversized.to_string().len() > 16 * 1024);
    invalid.push(oversized);
    let mut excessive_domains = canonical.clone();
    excessive_domains["editor_local"]["domains"] =
        json!(vec![canonical["editor_local"]["domains"][0].clone(); 1000]);
    invalid.push(excessive_domains);
    for value in &invalid {
        assert!(
            !work_status::validate_response(value),
            "hostile fixture must be rejected before projection"
        );
    }
    for version in VERSIONS {
        let mut replies: Vec<_> = invalid
            .iter()
            .cloned()
            .map(|value| (METHOD, Reply::Ok(value)))
            .collect();
        replies.push((METHOD, Reply::Ok(canonical.clone())));
        let (bridge, calls, connects) = scripted(replies);
        let mut server = McpServer::new(bridge, ToolGroups::ALL);
        initialize(&mut server, version);
        let mut fixed_error = None;
        for _ in &invalid {
            let result = call(&mut server, TOOL, json!({}));
            assert_text_failure(&result);
            assert!(!result.to_string().contains(PRIVATE));
            assert!(!result.to_string().contains("none_observed"));
            if let Some(previous) = &fixed_error {
                assert_eq!(
                    text(&result),
                    previous,
                    "response validation has one fixed, non-echoing local error"
                );
            } else {
                fixed_error = Some(text(&result).to_string());
            }
        }
        let success = call(&mut server, TOOL, json!({}));
        assert_eq!(success["isError"], false);
        assert_eq!(success["structuredContent"], canonical);
        assert_eq!(calls.borrow().len(), invalid.len() + 1);
        assert_eq!(connects.get(), 1);
    }
}

#[test]
fn valid_age_bounds_are_forwarded_once_without_extra_work() {
    for version in VERSIONS {
        for age in [1, 1000, 5000] {
            let expected = observation("ready", "none_observed");
            let (bridge, calls, connects) = scripted(vec![(METHOD, Reply::Ok(expected.clone()))]);
            let mut server = McpServer::new(bridge, ToolGroups::ALL);
            initialize(&mut server, version);
            assert_eq!(
                call(&mut server, TOOL, json!({"max_age_ms":age}))["structuredContent"],
                expected
            );
            assert_eq!(
                calls.borrow().as_slice(),
                [(METHOD.to_string(), json!({"max_age_ms":age}))]
            );
            assert_eq!(connects.get(), 1);
        }
    }
}

#[test]
fn absent_and_null_outer_arguments_keep_legacy_empty_object_normalization() {
    for version in VERSIONS {
        let expected = observation("not_paired", "indeterminate");
        let (bridge, calls, _) = scripted(vec![
            (METHOD, Reply::Ok(expected.clone())),
            (METHOD, Reply::Ok(expected.clone())),
        ]);
        let mut server = McpServer::new(bridge, ToolGroups::ALL);
        initialize(&mut server, version);
        for params in [json!({"name":TOOL}), json!({"name":TOOL, "arguments":null})] {
            let result = request(&mut server, "tools/call", params);
            assert_eq!(result["result"]["isError"], false);
            assert_eq!(result["result"]["structuredContent"], expected);
        }
        let logged = calls.borrow();
        assert_eq!(logged.len(), 2);
        for (method, params) in logged.iter() {
            assert_eq!(method, METHOD);
            assert!(params.is_object());
            assert!(
                params == &json!({}) || params == &json!({"max_age_ms":1000}),
                "only the default age may be materialized"
            );
        }
    }
}

#[test]
fn local_bad_arguments_are_sanitized_and_never_connect_to_erp() {
    let mut invalid = vec![
        json!({"path":PRIVATE}),
        json!({"url":format!("https://{PRIVATE}@private.example/path?token={PRIVATE}")}),
        json!({"force":true}),
        json!({"save":true}),
        json!({"refresh":true}),
        json!({"wait":true}),
        json!({"target":PRIVATE}),
        json!({"max_age_ms":1000, PRIVATE:PRIVATE}),
    ];
    for value in [
        J::Null,
        json!(false),
        json!(PRIVATE),
        json!([]),
        json!({}),
        json!(0),
        json!(-1),
        json!(5001),
        json!(u64::MAX),
    ] {
        invalid.push(json!({"max_age_ms":value}));
    }
    for number in ["1.5", "1.0", "1e3", "18446744073709551616"] {
        invalid.push(json!({"max_age_ms":serde_json::from_str::<J>(number).unwrap()}));
    }
    for version in VERSIONS {
        let (bridge, calls, connects) = scripted(Vec::new());
        let mut server = McpServer::new(bridge, ToolGroups::ALL);
        initialize(&mut server, version);
        for args in &invalid {
            let result = call(&mut server, TOOL, args.clone());
            assert_text_failure(&result);
            assert!(
                !result.to_string().contains(PRIVATE),
                "do not echo unknown argument names or values: {result}"
            );
        }
        assert!(calls.borrow().is_empty());
        assert_eq!(
            connects.get(),
            0,
            "validation must precede connection and RPC"
        );
    }
}

#[test]
fn non_object_outer_arguments_are_protocol_errors_without_erp_calls() {
    for version in VERSIONS {
        let (bridge, calls, connects) = scripted(Vec::new());
        let mut server = McpServer::new(bridge, ToolGroups::ALL);
        initialize(&mut server, version);
        for args in [
            json!([]),
            json!([PRIVATE]),
            json!(PRIVATE),
            json!(42),
            json!(false),
        ] {
            let response = request(
                &mut server,
                "tools/call",
                json!({"name":TOOL, "arguments":args}),
            );
            assert_eq!(response["error"]["code"], -32602);
            assert!(response.get("result").is_none());
            assert!(!response.to_string().contains(PRIVATE));
        }
        assert!(calls.borrow().is_empty());
        assert_eq!(connects.get(), 0);
    }
}

#[test]
fn error_metadata_is_bounded_and_cannot_contaminate_later_results() {
    for version in VERSIONS {
        let received = RpcError::new(
            orr_remote::PERMISSION_DENIED,
            "permission_denied",
            "missing read capability",
        )
        .with("owner", json!(PRIVATE))
        .with("path", json!(format!("/private/{PRIVATE}")))
        .with("token", json!(PRIVATE))
        .with("params", json!({"max_age_ms":PRIVATE}))
        .with("nested", json!({"diagnostic":PRIVATE}));
        let schema = json!({"schema":{"type":"object", "properties":{}}});
        let ready = observation("ready", "none_observed");
        let (bridge, calls, connects) = scripted(vec![
            (METHOD, Reply::Rpc(received.clone())),
            ("registry.input", Reply::Ok(schema.clone())),
            (METHOD, Reply::Ok(ready.clone())),
        ]);
        let mut server = McpServer::new(bridge, ToolGroups::ALL);
        initialize(&mut server, version);
        let denied = call(&mut server, TOOL, json!({}));
        assert_erp_failure(&denied, version, &received, Some("permission_denied"));
        assert!(!denied.to_string().contains(PRIVATE));
        assert_text_failure(&call(&mut server, TOOL, json!({"max_age_ms":0})));
        assert_eq!(
            call(&mut server, "get_schema", json!({"input":true}))["structuredContent"],
            schema
        );
        assert_eq!(
            call(&mut server, TOOL, json!({}))["structuredContent"],
            ready
        );
        assert_eq!(
            calls
                .borrow()
                .iter()
                .map(|(method, _)| method.as_str())
                .collect::<Vec<_>>(),
            [METHOD, "registry.input", METHOD]
        );
        assert_eq!(connects.get(), 1);
    }
}

#[test]
fn unreviewed_error_kind_is_not_promoted_to_structured_metadata() {
    for version in VERSIONS {
        let received = RpcError {
            code: 971,
            message: "host rejected the request".into(),
            data: Some(json!({"kind":"new_status_kind", "private":PRIVATE})),
        };
        let (bridge, calls, _) = scripted(vec![(METHOD, Reply::Rpc(received.clone()))]);
        let mut server = McpServer::new(bridge, ToolGroups::ALL);
        initialize(&mut server, version);
        let result = call(&mut server, TOOL, json!({}));
        assert_erp_failure(&result, version, &received, None);
        assert!(!result.to_string().contains(PRIVATE));
        assert_eq!(calls.borrow().len(), 1);
    }
}

#[test]
fn disconnect_timeout_and_protocol_loss_do_not_invent_status_or_retry() {
    for version in VERSIONS {
        for loss in [
            Reply::Transport("connection closed"),
            Reply::Transport("timed out waiting for the response to editor.work_status"),
            Reply::Protocol("invalid response envelope"),
        ] {
            let schema = json!({"schema":{"type":"object"}});
            let (bridge, calls, connects) = scripted(vec![
                (METHOD, loss),
                ("registry.input", Reply::Ok(schema.clone())),
            ]);
            let mut server = McpServer::new(bridge, ToolGroups::ALL);
            initialize(&mut server, version);
            let failed = call(&mut server, TOOL, json!({}));
            assert_text_failure(&failed);
            for invented in [
                "none_observed",
                "not_paired",
                "known_blockers",
                "\"origin\":\"erp\"",
                "safe_to_close",
            ] {
                assert!(!failed.to_string().contains(invented), "{failed}");
            }
            assert_eq!(calls.borrow().len(), 1);
            assert_eq!(connects.get(), 1, "no reconnect within the failed request");
            assert_text_failure(&call(&mut server, TOOL, json!({"save":true})));
            assert_eq!(connects.get(), 1, "local errors must not reconnect");
            assert_eq!(
                call(&mut server, "get_schema", json!({"input":true}))["structuredContent"],
                schema
            );
            assert_eq!(connects.get(), 2, "only the next explicit call reconnects");
            assert_eq!(
                calls
                    .borrow()
                    .iter()
                    .map(|(method, _)| method.as_str())
                    .collect::<Vec<_>>(),
                [METHOD, "registry.input"]
            );
        }
    }
}

#[test]
fn connection_failure_has_no_erp_code_or_automatic_second_attempt() {
    for version in VERSIONS {
        let connects = Rc::new(Cell::new(0));
        let attempts = connects.clone();
        let bridge = Bridge::new(
            "unreachable ERP",
            Box::new(move || {
                attempts.set(attempts.get() + 1);
                Err("connection refused".into())
            }),
        );
        let mut server = McpServer::new(bridge, ToolGroups::ALL);
        initialize(&mut server, version);
        let failed = call(&mut server, TOOL, json!({}));
        assert_text_failure(&failed);
        assert!(!failed.to_string().contains("none_observed"));
        assert_eq!(connects.get(), 1);
    }
}

fn authenticated_unpaired_host() -> LocalHost {
    LocalHost::spawn::<PhysGame>(|| {
        let mut types = TypeRegistry::new();
        register_reflect(&mut types);
        let doc = EditorDoc::from_yaml(
            include_str!("../../../scenes/physics_demo.scene.yaml"),
            types,
            Simulation::<PhysGame>::build_registry(),
            7,
        )
        .map_err(|error| error.to_string())?;
        let tokens = [
            ("reader", "read"),
            ("scene-only", "scene_edit"),
            ("approve-only", "approve"),
            ("sim-only", "sim_control"),
            ("all-without-read", "scene_edit,approve,sim_control"),
            ("admin", "all"),
        ]
        .into_iter()
        .map(|(name, caps)| TokenEntry {
            client: name.into(),
            token: format!("{name}-test-token"),
            caps: Caps::parse(caps).unwrap(),
        })
        .collect();
        let cfg = ServerConfig::new(Auth::Tokens(tokens));
        Ok((doc, cfg))
    })
    .expect("start real authenticated ERP host")
}

fn socket_mcp(host: &LocalHost, token: &str, version: &str) -> McpServer {
    let bridge = Bridge::to_url(
        host.url().expect("listening ERP host"),
        Some(token.into()),
        Duration::from_secs(5),
    );
    let mut server = McpServer::new(bridge, ToolGroups::ALL);
    initialize(&mut server, version);
    server
}

#[test]
fn real_authenticated_read_only_socket_reports_unpaired_without_mutating_scene() {
    let host = authenticated_unpaired_host();
    let mut direct = ErpClient::connect(host.url().unwrap(), Some("reader-test-token")).unwrap();
    let before_scene = direct.call("scene.save", json!({})).unwrap();
    let before_history = direct.call("history.list", json!({})).unwrap();
    let before_proposals = direct.call("proposal.list", json!({})).unwrap();
    let discovery = direct.call("rpc.discover", json!({})).unwrap();
    assert!(
        discovery["engine"].get("editor_work_status").is_none(),
        "an unpaired host must not advertise a live owner"
    );
    for version in VERSIONS {
        let mut server = socket_mcp(&host, "reader-test-token", version);
        for age in [1, 1000, 5000] {
            let args = json!({"max_age_ms":age});
            let expected = direct.call(METHOD, args.clone()).unwrap();
            let result = call(&mut server, TOOL, args);
            assert_eq!(result["isError"], false, "{result}");
            assert_eq!(result["structuredContent"], expected);
            let value = &result["structuredContent"];
            assert_eq!(value["availability"], "not_paired");
            assert_eq!(value["assessment"]["state"], "indeterminate");
            assert_eq!(value["host_scene"]["authority"], "erp_host");
            assert_eq!(value["host_scene"]["owner"], "host");
            for field in [
                "owner_binding_id",
                "snapshot_seq",
                "ui_frame",
                "age_ms",
                "max_age_ms",
            ] {
                assert!(value["observation"][field].is_null(), "{value}");
            }
        }
    }
    assert_eq!(direct.call("scene.save", json!({})).unwrap(), before_scene);
    assert_eq!(
        direct.call("history.list", json!({})).unwrap(),
        before_history
    );
    assert_eq!(
        direct.call("proposal.list", json!({})).unwrap(),
        before_proposals
    );
}

#[test]
fn real_non_read_capabilities_cannot_learn_even_an_absent_owner() {
    let host = authenticated_unpaired_host();
    for version in VERSIONS {
        for client in ["scene-only", "approve-only", "sim-only", "all-without-read"] {
            let token = format!("{client}-test-token");
            let mut direct = ErpClient::connect(host.url().unwrap(), Some(&token)).unwrap();
            let received = direct.call_err(METHOD, json!({}));
            assert_eq!(received.code, orr_remote::PERMISSION_DENIED);
            assert_eq!(received.kind(), Some("permission_denied"));
            for malformed in [J::Null, json!({PRIVATE:PRIVATE})] {
                let denied = direct.call_err(METHOD, malformed);
                assert_eq!(denied.code, orr_remote::PERMISSION_DENIED);
                assert_eq!(denied.kind(), Some("permission_denied"));
                assert!(!denied.to_json().to_string().contains(PRIVATE));
            }
            let discovery = direct.call("rpc.discover", json!({})).unwrap();
            assert!(discovery["engine"].get("editor_work_status").is_none());
            let mut server = socket_mcp(&host, &token, version);
            for args in [json!({}), json!({"max_age_ms":1})] {
                let result = call(&mut server, TOOL, args);
                assert_erp_failure(&result, version, &received, Some("permission_denied"));
                for secret_state in [
                    "not_paired",
                    "owner_binding_id",
                    "sprite_bindings",
                    "host_scene",
                    "none_observed",
                ] {
                    assert!(!result.to_string().contains(secret_state), "{result}");
                }
            }
        }
    }
}

#[test]
fn real_erp_rejects_non_object_or_invalid_status_params_without_echoing_them() {
    let host = authenticated_unpaired_host();
    let mut direct = ErpClient::connect(host.url().unwrap(), Some("reader-test-token")).unwrap();
    for params in [
        J::Null,
        json!([]),
        json!(PRIVATE),
        json!({"path":PRIVATE}),
        json!({PRIVATE:PRIVATE}),
        json!({"max_age_ms":null}),
        json!({"max_age_ms":PRIVATE}),
        json!({"max_age_ms":0}),
        json!({"max_age_ms":5001}),
    ] {
        let error = direct.call_err(METHOD, params);
        assert_eq!(error.code, orr_remote::INVALID_PARAMS);
        assert_eq!(error.kind(), Some("invalid_params"));
        assert!(!error.to_json().to_string().contains(PRIVATE), "{error:?}");
    }
}

#[test]
fn genuine_erp_method_not_found_is_preserved_as_an_old_host_error() {
    let host = authenticated_unpaired_host();
    let mut direct = ErpClient::connect(host.url().unwrap(), Some("reader-test-token")).unwrap();
    // Source a real server-produced error, then replay that unchanged typed
    // error for this tool as an old host would. This is not a historical binary.
    let received = direct.call_err("editor.unknown_legacy_status_method", json!({}));
    assert_eq!(received.code, orr_remote::METHOD_NOT_FOUND);
    assert_eq!(received.kind(), Some("method_not_found"));
    for version in VERSIONS {
        let (bridge, calls, connects) = scripted(vec![(METHOD, Reply::Rpc(received.clone()))]);
        let mut server = McpServer::new(bridge, ToolGroups::ALL);
        initialize(&mut server, version);
        let result = call(&mut server, TOOL, json!({}));
        assert_erp_failure(&result, version, &received, Some("method_not_found"));
        assert!(
            !result.to_string().contains("not_paired"),
            "missing method is an ERP error, not an invented status"
        );
        assert_eq!(calls.borrow().len(), 1);
        assert_eq!(connects.get(), 1);
    }
}
