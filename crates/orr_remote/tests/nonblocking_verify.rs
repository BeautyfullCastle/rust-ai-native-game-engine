//! Nonblocking verification integration tests. A game metric is used as a
//! deterministic barrier inside the verification worker; the host itself
//! never sleeps or waits on the test thread.
#![allow(clippy::disallowed_types)]

mod common;

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use common::demo_doc;
use orr_bridge::Bridge;
use orr_ecs::Frame;
use orr_edit::{MetricValue, Metrics};
use orr_remote::{
    call_local, Auth, Caps, ErpClient, ErpTarget, GameHooks, HostLimits, LocalHost, RemoteBridge,
    RemoteConfig, Request, ServerConfig, Transport, LIMIT_EXCEEDED,
};
use orr_sample::physics_game::{PhysGame, PhysMetrics};
use serde_json::{json, Value as J};

const BODY: &str = "orr_physics::Body";
const RPC_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Default)]
struct GateState {
    armed: bool,
    used: bool,
    entered: bool,
    released: bool,
}

struct Gate {
    state: Mutex<GateState>,
    changed: Condvar,
}

impl Gate {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(GateState::default()),
            changed: Condvar::new(),
        })
    }

    fn wait_in_worker(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.armed || state.used {
            return;
        }
        state.used = true;
        state.entered = true;
        self.changed.notify_all();
        let deadline = Instant::now() + RPC_TIMEOUT;
        while !state.released {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "test verification gate was never released");
            let (next, timeout) = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(|e| e.into_inner());
            state = next;
            assert!(
                !timeout.timed_out() || state.released,
                "test verification gate timed out"
            );
        }
    }

    fn arm(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.armed = true;
    }

    fn wait_entered(&self) {
        let deadline = Instant::now() + RPC_TIMEOUT;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        while !state.entered {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "verification worker did not reach its gate"
            );
            let (next, timeout) = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(|e| e.into_inner());
            state = next;
            assert!(
                !timeout.timed_out() || state.entered,
                "verification worker did not reach its gate"
            );
        }
    }

    fn release(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.released = true;
        self.changed.notify_all();
    }
}

/// Declared after the host so unwinding releases a blocked worker before the
/// host's destructor joins its loop.
struct ReleaseOnDrop(Arc<Gate>);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

struct GatedMetrics(Arc<Gate>);

impl Metrics for GatedMetrics {
    fn sample(&self, _frame: &Frame) -> Vec<(String, MetricValue)> {
        self.0.wait_in_worker();
        Vec::new()
    }
}

fn host(listen: bool, gate: Arc<Gate>) -> LocalHost {
    LocalHost::spawn::<PhysGame>(move || {
        let mut cfg = ServerConfig::new(Auth::DevNoAuth);
        cfg.listen = listen;
        cfg.limits.game =
            GameHooks::new("PhysGame").with_metrics((PhysMetrics, GatedMetrics(gate)));
        Ok((demo_doc(), cfg))
    })
    .expect("start LocalHost")
}

fn local_client(host: &LocalHost) -> ErpClient {
    ErpClient::with_transport(Box::new(
        host.connector().connect("agent", Caps::ALL).unwrap(),
    ))
}

fn client(host: &LocalHost, websocket: bool) -> ErpClient {
    if websocket {
        ErpClient::connect(host.url().expect("listening host"), None).unwrap()
    } else {
        local_client(host)
    }
}

fn view_bridge(host: &LocalHost, websocket: bool) -> RemoteBridge<PhysGame> {
    let mut cfg = RemoteConfig::new(host.url().unwrap_or(""));
    cfg.source = "view".into();
    if websocket {
        RemoteBridge::connect(cfg).unwrap()
    } else {
        let transport = host.connector().connect("view", Caps::ALL).unwrap();
        RemoteBridge::connect_transport(Box::new(transport), cfg).unwrap()
    }
}

fn wait_response(c: &mut ErpClient, id: u64) -> Result<J, orr_remote::RpcError> {
    let deadline = Instant::now() + RPC_TIMEOUT;
    loop {
        let _ = c.poll().expect("client connection stays live");
        if let Some(result) = c.take_response(id) {
            return result;
        }
        assert!(Instant::now() < deadline, "RPC response did not arrive");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn wait_snapshot(
    bridge: &RemoteBridge<PhysGame>,
    predicate: impl Fn(&orr_bridge::Snapshot) -> bool,
) -> orr_bridge::Snapshot {
    let deadline = Instant::now() + RPC_TIMEOUT;
    loop {
        if let Some(snapshot) = bridge.snapshot() {
            if predicate(&snapshot) {
                return snapshot;
            }
        }
        assert!(
            Instant::now() < deadline,
            "view did not publish the expected snapshot"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn activity_methods(c: &mut ErpClient, method: &str) -> Vec<J> {
    c.call(
        "activity.list",
        json!({"include_reads": true, "limit": 200}),
    )
    .unwrap()["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["method"] == method)
        .cloned()
        .collect()
}

#[test]
fn verification_does_not_stall_local_or_websocket_host_work() {
    // Exercise both in-process and real WebSocket clients against LocalHost.
    for websocket in [false, true] {
        let gate = Gate::new();
        let host = host(websocket, gate.clone());
        let _release = ReleaseOnDrop(gate.clone());
        let mut c = client(&host, websocket);
        c.call_timeout = RPC_TIMEOUT;
        let view = view_bridge(&host, websocket);

        gate.arm();
        let verify_id = c
            .post(
                "verify.self",
                json!({"inputs": {"kind": "idle", "ticks": 120}}),
            )
            .unwrap();
        gate.wait_entered();

        // Reads and edits are served on the host thread while the replay is
        // held in its worker. A running play also continues to advance.
        let before = c.call("world.query", json!({"name": "body_05"})).unwrap();
        let entity = before["entities"][0]["guid"].as_str().unwrap();
        c.call(
            "world.patch",
            json!({"entity": entity, "component": BODY, "path": "pos.x", "value": 4}),
        )
        .unwrap();
        c.call("sim.start", json!({"run": true})).unwrap();
        let deadline = Instant::now() + RPC_TIMEOUT;
        let mut live = 0;
        while Instant::now() < deadline {
            live = c.call("sim.state", J::Null).unwrap()["head_tick"]
                .as_u64()
                .unwrap();
            if live >= 2 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(live >= 2, "live play stalled while verification was held");
        let snapshot = wait_snapshot(&view, |s| s.tick() >= 1);
        assert!(
            snapshot.tick() >= 1,
            "view frames remain live during verification"
        );
        c.call("sim.pause", J::Null).unwrap();

        // The original RPC has not completed yet, but the other work above
        // has. Releasing the metric lets it finish and records it exactly once.
        c.poll().unwrap();
        assert!(
            c.take_response(verify_id).is_none(),
            "verification must still be held"
        );
        gate.release();
        let report = wait_response(&mut c, verify_id).unwrap();
        assert_eq!(report["ticks"], 120);
        assert_eq!(activity_methods(&mut c, "verify.self").len(), 1);
        assert!(host.is_running());
    }
}

#[test]
fn busy_and_disconnected_verification_keep_the_slot_until_worker_exit() {
    let gate = Gate::new();
    let host = host(false, gate.clone());
    let _release = ReleaseOnDrop(gate.clone());
    let mut first = local_client(&host);
    let mut second = local_client(&host);
    first.call_timeout = RPC_TIMEOUT;
    second.call_timeout = Duration::from_secs(2);

    gate.arm();
    first
        .post(
            "verify.self",
            json!({"inputs": {"kind": "idle", "ticks": 40}}),
        )
        .unwrap();
    gate.wait_entered();

    let start = Instant::now();
    let busy = second.call_err(
        "verify.self",
        json!({"inputs": {"kind": "idle", "ticks": 40}}),
    );
    assert_eq!(busy.code, LIMIT_EXCEEDED);
    assert_eq!(busy.kind(), Some("verify_busy"));
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "busy admission should return promptly"
    );

    // LocalTransport drop is a deterministic disconnect signal. The still-
    // blocked worker retains capacity even after cancellation was requested.
    drop(first);
    let busy = second.call_err(
        "verify.self",
        json!({"inputs": {"kind": "idle", "ticks": 40}}),
    );
    assert_eq!(busy.kind(), Some("verify_busy"));

    gate.release();
    let deadline = Instant::now() + RPC_TIMEOUT;
    loop {
        let result = second.call(
            "verify.self",
            json!({"inputs": {"kind": "idle", "ticks": 5}}),
        );
        match result {
            Ok(report) => {
                assert_eq!(report["ticks"], 5);
                break;
            }
            Err(orr_remote::ClientError::Rpc(error)) if error.kind() == Some("verify_busy") => {
                assert!(
                    Instant::now() < deadline,
                    "cancelled worker never released its slot"
                );
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(other) => panic!("slot reuse failed: {other}"),
        }
    }
    let history = activity_methods(&mut second, "verify.self");
    let cancelled: Vec<_> = history
        .iter()
        .filter(|entry| {
            entry["error"]
                .as_str()
                .is_some_and(|e| e.contains("cancelled"))
        })
        .collect();
    assert_eq!(
        cancelled.len(),
        1,
        "the disconnected request has one terminal activity entry"
    );
    assert!(
        cancelled[0].get("verify").is_none(),
        "a cancelled run has no VerifyDetail"
    );
    assert_eq!(
        history.iter().filter(|entry| entry["ok"] == true).count(),
        1,
        "slot reuse completes once"
    );
}

#[test]
fn proposal_verification_keeps_its_admission_snapshot_and_stamp() {
    let gate = Gate::new();
    let host = host(false, gate.clone());
    let _release = ReleaseOnDrop(gate.clone());
    let mut c = local_client(&host);
    c.call_timeout = RPC_TIMEOUT;

    let guid = c.call("world.query", json!({"name": "body_05"})).unwrap()["entities"][0]["guid"]
        .as_str()
        .unwrap()
        .to_string();
    let before = c.call("scene.save", J::Null).unwrap();
    let proposal = c
        .call("proposal.begin", json!({"label": "captured proposal"}))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let staged = c
        .call("proposal.apply", json!({"id": proposal, "ops": [{"op": "patch", "entity": guid, "component": BODY, "path": "pos", "value": [0, 20]}]}))
        .unwrap();

    gate.arm();
    let verify_id = c
        .post(
            "proposal.verify",
            json!({"id": proposal, "inputs": {"kind": "idle", "ticks": 60}}),
        )
        .unwrap();
    gate.wait_entered();

    // Advance both document and proposal revisions while the worker owns its
    // already-captured frames and verification stamp.
    c.call(
        "world.patch",
        json!({"entity": guid, "component": BODY, "path": "pos.x", "value": 3}),
    )
    .unwrap();
    c.call("proposal.apply", json!({"id": proposal, "ops": [{"op": "patch", "entity": guid, "component": BODY, "path": "angle", "value": 0.25}]})).unwrap();
    gate.release();

    let report = wait_response(&mut c, verify_id).unwrap();
    assert_eq!(report["proposal"], proposal);
    assert_eq!(report["ticks"], 60);
    assert_eq!(report["checksums"]["base_start"], before["checksum"]);
    assert_eq!(report["checksums"]["candidate_start"], staged["checksum"]);

    let stale = c.call_err(
        "proposal.accept_verified",
        json!({"id": proposal, "verified_state": report["verified_state"]}),
    );
    assert_eq!(stale.kind(), Some("stale_verification"));
    assert_eq!(
        c.call("proposal.get", json!({"id": proposal})).unwrap()["op_count"],
        2
    );
    c.call("proposal.reject", json!({"id": proposal})).unwrap();
}

#[test]
fn proposal_can_be_rejected_while_its_captured_verification_finishes() {
    let gate = Gate::new();
    let host = host(false, gate.clone());
    let _release = ReleaseOnDrop(gate.clone());
    let mut c = local_client(&host);
    c.call_timeout = RPC_TIMEOUT;
    let guid = c.call("world.query", json!({"name": "body_05"})).unwrap()["entities"][0]["guid"]
        .as_str()
        .unwrap()
        .to_string();
    let proposal = c
        .call("proposal.begin", json!({"label": "reject during verify"}))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    c.call("proposal.apply", json!({"id": proposal, "ops": [{"op": "patch", "entity": guid, "component": BODY, "path": "pos.y", "value": 22}]})).unwrap();

    gate.arm();
    let verify_id = c
        .post(
            "proposal.verify",
            json!({"id": proposal, "inputs": {"kind": "idle", "ticks": 20}}),
        )
        .unwrap();
    gate.wait_entered();
    c.call("proposal.reject", json!({"id": proposal})).unwrap();
    gate.release();

    let report = wait_response(&mut c, verify_id).unwrap();
    assert_eq!(report["proposal"], proposal);
    assert_eq!(report["verified_state"]["id"], proposal);
    assert_eq!(report["ticks"], 20);
    assert_eq!(
        c.call_err("proposal.get", json!({"id": proposal})).kind(),
        Some("unknown_proposal")
    );
}

#[test]
fn last_play_is_captured_before_a_new_play_replaces_it() {
    let gate = Gate::new();
    let host = host(false, gate.clone());
    let _release = ReleaseOnDrop(gate.clone());
    let mut c = local_client(&host);
    c.call_timeout = RPC_TIMEOUT;

    c.call("sim.start", J::Null).unwrap();
    c.call("sim.step", json!({"n": 5})).unwrap();
    assert_eq!(c.call("sim.stop", J::Null).unwrap()["tick"], 5);

    gate.arm();
    let verify_id = c
        .post("verify.self", json!({"inputs": {"kind": "last_play"}}))
        .unwrap();
    gate.wait_entered();

    c.call("sim.start", J::Null).unwrap();
    c.call("sim.step", json!({"n": 11})).unwrap();
    assert_eq!(c.call("sim.stop", J::Null).unwrap()["tick"], 11);
    gate.release();

    let report = wait_response(&mut c, verify_id).unwrap();
    assert_eq!(report["inputs"]["kind"], "last_play");
    assert_eq!(
        report["ticks"], 5,
        "the running verifier uses the earlier recording"
    );
    assert_eq!(c.call("sim.state", J::Null).unwrap()["mode"], "edit");
}

#[test]
fn dropping_host_closes_local_transport_even_if_a_worker_is_still_held() {
    let gate = Gate::new();
    let host = host(false, gate.clone());
    let _release = ReleaseOnDrop(gate.clone());
    let mut c = local_client(&host);
    c.call_timeout = Duration::from_secs(2);
    gate.arm();
    c.post(
        "verify.self",
        json!({"inputs": {"kind": "idle", "ticks": 80}}),
    )
    .unwrap();
    gate.wait_entered();

    // Dropping on another thread lets the test observe transport shutdown
    // while the worker's hook is still blocked. Joining the dropper before
    // release proves host shutdown doesn't join an active verification worker.
    let (tx, rx) = std::sync::mpsc::channel();
    let dropper = std::thread::spawn(move || {
        drop(host);
        tx.send(()).unwrap();
    });
    let shutdown_bound = Duration::from_secs(2);
    let deadline = Instant::now() + shutdown_bound;
    let mut closed = false;
    while Instant::now() < deadline {
        match c.call("sim.state", J::Null) {
            Err(orr_remote::ClientError::Transport(message))
                if message.contains("host stopped") || message.contains("host has stopped") =>
            {
                closed = true;
                break;
            }
            Err(other) => panic!("expected host transport closure, got {other}"),
            Ok(_) => std::thread::sleep(Duration::from_millis(2)),
        }
    }
    assert!(closed, "host drop should close its local transport");
    rx.recv_timeout(shutdown_bound)
        .expect("LocalHost drop should finish promptly even with the verification hook held");
    dropper.join().unwrap();
    gate.release();
}

#[test]
fn worker_completion_wakes_an_idle_server_poll() {
    let gate = Gate::new();
    let mut cfg = ServerConfig::new(Auth::DevNoAuth);
    cfg.listen = false;
    cfg.limits.game =
        GameHooks::new("PhysGame").with_metrics((PhysMetrics, GatedMetrics(gate.clone())));
    let mut server = orr_remote::ErpServer::start(cfg).unwrap();
    let _release = ReleaseOnDrop(gate.clone());
    let transport = server.connector().connect("agent", Caps::ALL).unwrap();
    let mut c = ErpClient::with_transport(Box::new(transport));
    c.call_timeout = RPC_TIMEOUT;
    let mut doc = demo_doc();
    let mut play: Option<orr_edit::PlayController<PhysGame>> = None;
    let mut target = orr_remote::ErpTarget {
        doc: &mut doc,
        play: &mut play,
    };

    gate.arm();
    let verify_id = c
        .post(
            "verify.self",
            json!({"inputs": {"kind": "idle", "ticks": 80}}),
        )
        .unwrap();
    server.poll(&mut target);
    gate.wait_entered();

    gate.release();
    let start = Instant::now();
    server.wait_for_request(Duration::from_secs(5));
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "worker completion should wake the idle host"
    );

    let deadline = Instant::now() + RPC_TIMEOUT;
    loop {
        server.poll(&mut target);
        c.poll().unwrap();
        if let Some(result) = c.take_response(verify_id) {
            assert_eq!(result.unwrap()["ticks"], 80);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "idle server did not finish the verification response"
        );
        server.wait_for_request(Duration::from_secs(5));
    }
}

#[test]
fn verification_notification_records_once_without_a_response() {
    let gate = Gate::new();
    let host = host(false, gate.clone());
    let _release = ReleaseOnDrop(gate.clone());
    let mut notification = host.connector().connect("agent", Caps::ALL).unwrap();
    let mut observer = local_client(&host);

    gate.arm();
    notification
        .send(Request {
            id: None,
            method: "verify.self".into(),
            params: json!({"inputs": {"kind": "idle", "ticks": 25}}),
        })
        .unwrap();
    gate.wait_entered();
    gate.release();

    let deadline = Instant::now() + RPC_TIMEOUT;
    loop {
        let entries = activity_methods(&mut observer, "verify.self");
        if !entries.is_empty() {
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0]["ok"], true);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "notification verification was not recorded"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        notification
            .recv(Duration::from_millis(50))
            .unwrap()
            .is_none(),
        "a JSON-RPC notification must not get a response"
    );
    assert_eq!(observer.call("sim.state", J::Null).unwrap()["mode"], "edit");
}

struct CallerThread(std::thread::ThreadId);

impl Metrics for CallerThread {
    fn sample(&self, _frame: &Frame) -> Vec<(String, MetricValue)> {
        assert_eq!(
            std::thread::current().id(),
            self.0,
            "call_local verification stays synchronous"
        );
        Vec::new()
    }
}

#[test]
fn call_local_verification_remains_synchronous() {
    let caller = std::thread::current().id();
    let limits = HostLimits {
        game: GameHooks::new("PhysGame").with_metrics((PhysMetrics, CallerThread(caller))),
        ..HostLimits::default()
    };
    let mut doc = demo_doc();
    let mut play: Option<orr_edit::PlayController<PhysGame>> = None;
    let mut target = ErpTarget {
        doc: &mut doc,
        play: &mut play,
    };

    let report = call_local(
        &mut target,
        &limits,
        "agent",
        Caps::ALL,
        "verify.self",
        &json!({"inputs": {"kind": "idle", "ticks": 8}}),
    )
    .unwrap();
    assert_eq!(report["ticks"], 8);
}
