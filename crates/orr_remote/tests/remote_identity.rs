#![allow(clippy::disallowed_types)]

use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use orr_remote::{
    ClientError, Incoming, RemoteBridge, RemoteConfig, RemoteIdentity, Request, Transport,
    TxHandle, ViewDeliveryMode,
};
use orr_sample::physics_game::PhysGame;
use serde_json::{json, Value as J};

type RecordedRequests = Arc<Mutex<Vec<Request>>>;
type ConnectResult = Result<RemoteBridge<PhysGame>, String>;

struct ScriptTx {
    responses: Sender<Incoming>,
    requests: Arc<Mutex<Vec<Request>>>,
    discovery: J,
    schema: J,
    fail_method: Option<String>,
}

impl ScriptTx {
    fn answer(&self, req: Request) -> Result<(), ClientError> {
        let result = match req.method.as_str() {
            "rpc.discover" => self.discovery.clone(),
            "registry.schema" => json!({"schema": self.schema}),
            "watch.subscribe" => json!({
                "topics": ["frames", "events", "notes"],
                "view_delivery": 1,
                "subscription": "1",
                "cursor": "0",
                "count": "0"
            }),
            "sim.state" => json!({"tick_rate": 60, "player_count": 2}),
            "auth" => json!({"ok": true}),
            _ => J::Null,
        };
        let id = req.id;
        let failed = self.fail_method.as_deref() == Some(req.method.as_str());
        self.requests.lock().unwrap().push(req);
        let response = if failed {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method unavailable"}})
        } else {
            json!({"jsonrpc": "2.0", "id": id, "result": result})
        };
        self.responses
            .send(Incoming::Text(response.to_string()))
            .map_err(|e| ClientError::Transport(e.to_string()))
    }
}

impl TxHandle for ScriptTx {
    fn send(&self, req: Request) -> Result<(), ClientError> {
        self.answer(req)
    }
}

struct ScriptTransport {
    responses: Receiver<Incoming>,
    tx: Arc<ScriptTx>,
}

impl Transport for ScriptTransport {
    fn send(&mut self, req: Request) -> Result<(), ClientError> {
        self.tx.answer(req)
    }

    fn recv(&mut self, timeout: Duration) -> Result<Option<Incoming>, ClientError> {
        match self.responses.recv_timeout(timeout) {
            Ok(message) => Ok(Some(message)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => {
                Err(ClientError::Transport("scripted transport closed".into()))
            }
        }
    }

    fn sender(&self) -> Option<Arc<dyn TxHandle>> {
        Some(self.tx.clone())
    }
}

fn discovery() -> J {
    json!({
        "erp_version": 1,
        "engine": {"game": "PhysGame", "build_id": "0x0000000000000042"}
    })
}

fn schema() -> J {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "PhysGame",
        "properties": {"components": {"type": "object"}},
        "required": ["components"]
    })
}

fn identity() -> RemoteIdentity {
    RemoteIdentity::from_discovery(&discovery(), schema()).unwrap()
}

fn connect(
    remote_discovery: J,
    remote_schema: J,
    expected: RemoteIdentity,
    source: &str,
    token: Option<&str>,
) -> (ConnectResult, RecordedRequests) {
    connect_with_error(
        remote_discovery,
        remote_schema,
        expected,
        source,
        token,
        None,
    )
}

fn connect_with_error(
    remote_discovery: J,
    remote_schema: J,
    expected: RemoteIdentity,
    source: &str,
    token: Option<&str>,
    fail_method: Option<&str>,
) -> (ConnectResult, RecordedRequests) {
    let (response_tx, response_rx) = channel();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let tx = Arc::new(ScriptTx {
        responses: response_tx,
        requests: requests.clone(),
        discovery: remote_discovery,
        schema: remote_schema,
        fail_method: fail_method.map(str::to_owned),
    });
    let transport = ScriptTransport {
        responses: response_rx,
        tx,
    };
    let mut cfg = RemoteConfig::new("");
    cfg.connect_timeout = Duration::from_secs(2);
    cfg.source = source.to_string();
    cfg.token = token.map(str::to_owned);
    cfg.expected_identity = Some(expected);
    cfg.view_delivery = ViewDeliveryMode::RequireFenced;
    (
        RemoteBridge::connect_transport(Box::new(transport), cfg),
        requests,
    )
}

fn methods(requests: &[Request]) -> Vec<&str> {
    requests.iter().map(|req| req.method.as_str()).collect()
}

fn error(result: Result<RemoteBridge<PhysGame>, String>) -> String {
    match result {
        Err(error) => error,
        Ok(_) => panic!("checked connection should have failed"),
    }
}

#[test]
fn checked_handshake_validates_then_subscribes_to_the_configured_source() {
    let (bridge, requests) = connect(discovery(), schema(), identity(), "proposal:p3", None);
    let bridge = bridge.expect("matching identity should connect");
    assert_eq!(
        bridge.view_delivery(),
        orr_remote::RemoteViewDelivery::Fenced
    );

    let requests = requests.lock().unwrap();
    assert_eq!(
        methods(&requests),
        vec![
            "rpc.discover",
            "registry.schema",
            "watch.subscribe",
            "sim.state"
        ]
    );
    assert_eq!(requests[2].params["source"], "proposal:p3");
}

#[test]
fn checked_handshake_authenticates_before_discovery() {
    let (bridge, requests) = connect(discovery(), schema(), identity(), "view", Some("token"));
    let bridge = bridge.expect("matching authenticated identity should connect");
    drop(bridge);

    let requests = requests.lock().unwrap();
    assert_eq!(
        methods(&requests),
        vec![
            "auth",
            "rpc.discover",
            "registry.schema",
            "watch.subscribe",
            "sim.state"
        ]
    );
}

#[test]
fn checked_handshake_fails_closed_on_game_or_build_mismatch() {
    let mut wrong_game = identity();
    wrong_game.game = "DifferentGame".into();
    let (result, requests) = connect(discovery(), schema(), wrong_game, "view", None);
    assert!(error(result).contains("remote identity check: game mismatch"));
    assert!(!methods(&requests.lock().unwrap()).contains(&"watch.subscribe"));

    let mut wrong_build = identity();
    wrong_build.build_id = "0x0000000000000099".into();
    let (result, requests) = connect(discovery(), schema(), wrong_build, "view", None);
    assert!(error(result).contains("remote identity check: build id mismatch"));
    assert!(!methods(&requests.lock().unwrap()).contains(&"watch.subscribe"));
}

#[test]
fn checked_handshake_fails_closed_on_schema_mismatch() {
    let mut different_schema = schema();
    different_schema["properties"]["extra"] = json!({"type": "string"});
    let (result, requests) = connect(discovery(), different_schema, identity(), "view", None);
    assert!(error(result).contains("remote identity check: registry.schema does not match"));
    assert!(!methods(&requests.lock().unwrap()).contains(&"watch.subscribe"));
}

#[test]
fn checked_handshake_rejects_missing_identity_fields_and_non_erp1() {
    let mut missing_build = discovery();
    missing_build["engine"]
        .as_object_mut()
        .unwrap()
        .remove("build_id");
    let (result, requests) = connect(missing_build, schema(), identity(), "view", None);
    assert!(
        error(result).contains("remote identity check: rpc.discover is missing engine.build_id")
    );
    assert!(!methods(&requests.lock().unwrap()).contains(&"watch.subscribe"));

    let mut unsupported = discovery();
    unsupported["erp_version"] = json!(2);
    let (result, requests) = connect(unsupported, schema(), identity(), "view", None);
    assert!(error(result)
        .contains("remote identity check: rpc.discover does not explicitly report ERP version 1"));
    assert!(!methods(&requests.lock().unwrap()).contains(&"watch.subscribe"));
}

#[test]
fn checked_handshake_rejects_a_missing_full_schema() {
    let (result, requests) = connect(discovery(), J::Null, identity(), "view", None);
    assert!(error(result)
        .contains("remote identity check: registry.schema did not return a full schema object"));
    assert!(!methods(&requests.lock().unwrap()).contains(&"watch.subscribe"));
}

#[test]
fn checked_handshake_classifies_discovery_and_schema_rpc_errors() {
    let (result, requests) = connect_with_error(
        discovery(),
        schema(),
        identity(),
        "view",
        None,
        Some("rpc.discover"),
    );
    assert!(error(result).contains("remote identity check:"));
    assert!(!methods(&requests.lock().unwrap()).contains(&"watch.subscribe"));

    let (result, requests) = connect_with_error(
        discovery(),
        schema(),
        identity(),
        "view",
        None,
        Some("registry.schema"),
    );
    assert!(error(result).contains("remote identity check:"));
    assert!(!methods(&requests.lock().unwrap()).contains(&"watch.subscribe"));
}
