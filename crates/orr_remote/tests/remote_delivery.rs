//! Protocol-boundary tests use a scripted Transport with real RemoteBridge
//! decoding and mailbox logic. Live host/socket coverage lives in editor and
//! remote recovery integration tests; this fixture models old or malformed peers.
#![allow(clippy::disallowed_types)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use orr_bridge::{Bridge, BridgeEvent, Lifecycle};
use orr_ecs::Frame;
use orr_remote::codec::hex_encode;
use orr_remote::wire::{encode_frame_message, timeline_to_json};
use orr_remote::{ClientError, Incoming, RemoteBridge, RemoteConfig, RemoteViewDelivery, Request, Transport, TxHandle, ViewDeliveryMode};
use orr_sample::physics_game::{PhysEvent, PhysGame};
use orr_session::{PlayMode, Speed, Timeline};
use orr_sim::Simulation;
use serde_json::{json, Value as J};

enum ServerMessage {
    Message(Incoming),
    Closed,
}

struct Pipe {
    tx: Sender<ServerMessage>,
    sent: AtomicUsize,
    received: AtomicUsize,
}

impl Pipe {
    fn send(&self, message: ServerMessage) -> Result<usize, ClientError> {
        let index = self.sent.fetch_add(1, Ordering::AcqRel) + 1;
        self.tx.send(message).map_err(|_| ClientError::Transport("scripted connection closed".into()))?;
        Ok(index)
    }
}

struct ScriptTx {
    pipe: Arc<Pipe>,
    acknowledgement: J,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl TxHandle for ScriptTx {
    fn send(&self, req: Request) -> Result<(), ClientError> {
        let result = match req.method.as_str() {
            "watch.subscribe" => self.acknowledgement.clone(),
            "sim.state" => json!({"tick_rate": 60, "player_count": 2}),
            _ => J::Null,
        };
        let id = req.id;
        self.requests.lock().unwrap().push(req);
        self.pipe.send(ServerMessage::Message(Incoming::Text(json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string())))?;
        Ok(())
    }
}

struct ScriptTransport {
    rx: Receiver<ServerMessage>,
    tx: Arc<ScriptTx>,
}

impl Transport for ScriptTransport {
    fn send(&mut self, req: Request) -> Result<(), ClientError> {
        self.tx.send(req)
    }

    fn recv(&mut self, timeout: Duration) -> Result<Option<Incoming>, ClientError> {
        match self.rx.recv_timeout(timeout) {
            Ok(ServerMessage::Message(message)) => {
                self.tx.pipe.received.fetch_add(1, Ordering::Release);
                Ok(Some(message))
            }
            Ok(ServerMessage::Closed) | Err(RecvTimeoutError::Disconnected) => Err(ClientError::Transport("scripted peer closed".into())),
            Err(RecvTimeoutError::Timeout) => Ok(None),
        }
    }

    fn sender(&self) -> Option<Arc<dyn TxHandle>> {
        Some(self.tx.clone())
    }
}

struct ScriptHost {
    pipe: Arc<Pipe>,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl ScriptHost {
    fn new(acknowledgement: J) -> (Self, Box<dyn Transport>) {
        let (tx, rx) = channel();
        let pipe = Arc::new(Pipe { tx, sent: AtomicUsize::new(0), received: AtomicUsize::new(0) });
        let requests = Arc::new(Mutex::new(Vec::new()));
        let tx = Arc::new(ScriptTx { pipe: pipe.clone(), acknowledgement, requests: requests.clone() });
        (Self { pipe, requests }, Box::new(ScriptTransport { rx, tx }))
    }

    fn send(&self, message: Incoming) {
        self.pipe.send(ServerMessage::Message(message)).unwrap();
    }

    /// When this harmless marker reaches recv, every preceding message has
    /// finished decoding. There is no sleep-based guess about receiver progress.
    fn flush(&self) {
        let marker = json!({"jsonrpc": "2.0", "method": "test.barrier", "params": null});
        let through = self.pipe.send(ServerMessage::Message(Incoming::Text(marker.to_string()))).unwrap();
        wait_until("the protocol barrier", || self.pipe.received.load(Ordering::Acquire) >= through);
    }

    fn close(&self) {
        self.pipe.send(ServerMessage::Closed).unwrap();
    }

    fn requested_mode(&self) -> Option<J> {
        self.requests.lock().unwrap().iter().find(|r| r.method == "watch.subscribe").unwrap().params.get("view_delivery").cloned()
    }
}

fn wait_until(what: &str, mut ready: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn acknowledgement() -> J {
    json!({"topics": ["frames", "events", "notes"], "view_delivery": 1, "subscription": "7", "cursor": "0", "count": "0"})
}

fn connect(mode: ViewDeliveryMode, acknowledgement: J) -> (RemoteBridge<PhysGame>, ScriptHost) {
    let (host, transport) = ScriptHost::new(acknowledgement);
    let mut cfg = RemoteConfig::new("");
    cfg.view_delivery = mode;
    cfg.view_event_capacity = 2;
    cfg.connect_timeout = Duration::from_secs(2);
    (RemoteBridge::connect_transport(transport, cfg).unwrap(), host)
}

fn cut(cursor: u64, count: u64) -> J {
    json!({"subscription": "7", "timeline": "1", "through_cursor": cursor.to_string(), "count": count.to_string(), "loss_generation": "1", "lifecycle": []})
}

fn frame(tick: u64, delivery: Option<J>) -> Incoming {
    let mut frame = Frame::new(Simulation::<PhysGame>::build_registry());
    frame.set_tick(tick);
    let mut meta = json!({"tick": tick, "epoch": 1, "tick_rate": 60, "timeline": null});
    if let Some(delivery) = delivery {
        meta["delivery"] = delivery;
    }
    Incoming::Wire(encode_frame_message(&meta, &frame.to_bytes()))
}

fn timeline_frame(tick: u64, verified_tick: u64, pending_edit: bool) -> Incoming {
    let mut frame = Frame::new(Simulation::<PhysGame>::build_registry());
    frame.set_tick(tick);
    let timeline = Timeline {
        mode: PlayMode::Record,
        tick,
        verified_tick,
        first_tick: 0,
        last_tick: tick,
        playing: false,
        speed: Speed(1000),
        // Paused debug edits can change current frame bytes before replacing
        // the recording's checksum at that tick. This is valid host metadata.
        checksum: frame.checksum().wrapping_add(u64::from(pending_edit)),
        keyframes: Arc::from([]),
        recent_checksums: Arc::from([]),
        pending_edits: u32::from(pending_edit),
        branches: 0,
        epoch: 1,
    };
    let meta = json!({"tick": tick, "epoch": 1, "tick_rate": 60, "timeline": timeline_to_json(&timeline), "delivery": cut(0, 0)});
    Incoming::Wire(encode_frame_message(&meta, &frame.to_bytes()))
}

fn events(cursor: u64, count: u64, ticks: &[u64], fenced: bool) -> Incoming {
    let payload = hex_encode(bytemuck::bytes_of(&PhysEvent { kind: 100, a: 0, b: 0 }));
    let events: Vec<_> = ticks.iter().enumerate().map(|(seq, tick)| json!({"tick": tick, "system": 0, "seq": seq, "payload": payload})).collect();
    let mut params = json!({"events": events});
    if fenced {
        params["delivery"] = json!({"subscription": "7", "timeline": "1", "cursor": cursor.to_string(), "count": count.to_string()});
    }
    Incoming::Text(json!({"jsonrpc": "2.0", "method": "watch.events", "params": params}).to_string())
}

fn baseline(bridge: &mut RemoteBridge<PhysGame>, host: &ScriptHost) {
    host.send(frame(0, Some(cut(0, 0))));
    host.flush();
    let update = bridge.poll_view();
    assert_eq!(update.snapshot.unwrap().tick(), 0);
    assert!(update.resync.is_some());
}

#[test]
fn new_peers_explicitly_acknowledge_fenced_delivery_and_wait_for_event_coverage() {
    let (mut bridge, host) = connect(ViewDeliveryMode::RequireFenced, acknowledgement());
    assert_eq!(host.requested_mode(), Some(json!(1)));
    assert_eq!(bridge.view_delivery(), RemoteViewDelivery::Fenced);
    baseline(&mut bridge, &host);

    host.send(events(1, 1, &[1], true));
    host.flush();
    let before = bridge.poll_view();
    assert_eq!(before.snapshot.unwrap().tick(), 0);
    assert!(before.events.is_empty(), "an event cannot precede its covering baseline");
    assert!(before.resync.is_none());

    host.send(frame(1, Some(cut(1, 1))));
    host.flush();
    let covered = bridge.poll_view();
    assert_eq!(covered.snapshot.unwrap().tick(), 1);
    assert!(covered.resync.is_none());
    assert_eq!(covered.events.len(), 1);
    assert!(matches!(&covered.events[0], BridgeEvent::Sim { key, .. } if key.tick == 1));
}

#[test]
fn requiring_fenced_delivery_refuses_an_old_hosts_success_without_the_echo() {
    let (host, transport) = ScriptHost::new(json!({"topics": ["frames", "events", "notes"]}));
    let mut cfg = RemoteConfig::new("");
    cfg.view_delivery = ViewDeliveryMode::RequireFenced;
    let error = RemoteBridge::<PhysGame>::connect_transport(transport, cfg).err().expect("old host cannot satisfy RequireFenced");
    assert_eq!(host.requested_mode(), Some(json!(1)));
    assert!(error.contains("did not acknowledge fenced view delivery"), "{error}");
}

#[test]
fn a_preferred_old_host_connection_reports_legacy_and_never_fakes_a_reset() {
    let (mut bridge, host) = connect(ViewDeliveryMode::PreferFenced, json!({"topics": ["frames", "events", "notes"]}));
    assert_eq!(host.requested_mode(), Some(json!(1)));
    assert_eq!(bridge.view_delivery(), RemoteViewDelivery::Legacy);
    host.send(frame(0, None));
    host.send(events(0, 0, &[1, 2, 3, 4, 5], false));
    host.flush();
    let update = bridge.poll_view();
    assert_eq!(update.snapshot.unwrap().tick(), 0);
    assert!(update.resync.is_none(), "legacy delivery cannot claim a coherent mailbox reset");
    assert_eq!(
        update.events.iter().filter(|e| matches!(e, BridgeEvent::Sim { .. })).count(),
        5,
        "the capacity setting does not silently drop legacy notifications"
    );
}

#[test]
fn opting_out_of_fenced_delivery_does_not_send_a_negotiation_request() {
    let (mut bridge, host) = connect(ViewDeliveryMode::Legacy, acknowledgement());
    assert_eq!(host.requested_mode(), None);
    assert_eq!(bridge.view_delivery(), RemoteViewDelivery::Legacy);
    host.send(frame(0, None));
    host.flush();
    assert!(bridge.poll_view().resync.is_none());
}

#[test]
fn a_malformed_fenced_acknowledgement_fails_instead_of_downgrading() {
    for mode in [ViewDeliveryMode::RequireFenced, ViewDeliveryMode::PreferFenced] {
        for field in ["subscription", "cursor", "count"] {
            let mut ack = acknowledgement();
            ack[field] = json!(0); // Cursors and identities must be exact decimal strings.
            let (_host, transport) = ScriptHost::new(ack);
            let mut cfg = RemoteConfig::new("");
            cfg.view_delivery = mode;
            let error = RemoteBridge::<PhysGame>::connect_transport(transport, cfg).err().expect("malformed v1 acknowledgement must fail closed");
            assert!(error.contains("exact view delivery"), "{field}: {error}");
        }
    }
}

#[test]
fn malformed_negotiated_frames_fail_closed_with_a_diagnostic() {
    for bad in [frame(1, None), Incoming::Wire(vec![0, 1, 2]), frame(1, Some(json!({"subscription": "7", "timeline": "1", "through_cursor": 0})))] {
        let (mut bridge, host) = connect(ViewDeliveryMode::RequireFenced, acknowledgement());
        baseline(&mut bridge, &host);
        host.send(bad);
        wait_until("the invalid stream to close", || !bridge.is_alive());
        assert!(bridge.take_errors().iter().any(|e| e.kind() == Some("view_delivery_invalid")));
        let mut disconnected = false;
        wait_until("the terminal diagnostic", || {
            let update = bridge.poll_view();
            assert!(update.resync.is_none(), "malformed data is not a recoverable publication");
            assert!(update.events.iter().all(|e| !matches!(e, BridgeEvent::Sim { .. })));
            disconnected |= update.events.iter().any(|e| matches!(e, BridgeEvent::Lifecycle(Lifecycle::Disconnected)));
            disconnected
        });
    }
}

#[test]
fn closing_during_uncovered_loss_reports_disconnect_without_an_invented_baseline() {
    let (mut bridge, host) = connect(ViewDeliveryMode::RequireFenced, acknowledgement());
    baseline(&mut bridge, &host);
    host.send(events(1, 3, &[1, 2, 3], true)); // One whole batch exceeds capacity two.
    host.flush();
    let waiting = bridge.poll_view();
    assert!(waiting.resync.is_none());
    assert!(waiting.events.is_empty());
    assert_eq!(waiting.snapshot.unwrap().tick(), 0);

    host.close();
    let mut disconnected = false;
    wait_until("disconnect while awaiting a covering frame", || {
        let update = bridge.poll_view();
        assert!(update.resync.is_none(), "a close cannot fabricate the missing event-to-frame fence");
        assert!(update.events.iter().all(|e| !matches!(e, BridgeEvent::Sim { .. })));
        assert_eq!(update.snapshot.unwrap().tick(), 0, "the last covered frame stays readable");
        disconnected |= update.events.iter().any(|e| matches!(e, BridgeEvent::Lifecycle(Lifecycle::Disconnected)));
        disconnected
    });
    assert!(!bridge.is_alive());
}

#[test]
fn a_negotiated_verified_tick_past_the_frame_head_fails_closed() {
    let (mut bridge, host) = connect(ViewDeliveryMode::RequireFenced, acknowledgement());
    baseline(&mut bridge, &host);
    host.send(timeline_frame(1, 2, false));
    wait_until("the inconsistent verified tick to close the stream", || !bridge.is_alive());
    assert!(bridge.take_errors().iter().any(|e| e.kind() == Some("view_delivery_invalid")));
    let update = bridge.poll_view();
    assert!(update.resync.is_none());
    assert!(update.snapshot.is_none_or(|s| s.tick() == 0), "the invalid head cannot replace the baseline");
}

#[test]
fn a_paused_pending_edit_may_have_a_recorded_checksum_older_than_the_frame_bytes() {
    let (mut bridge, host) = connect(ViewDeliveryMode::RequireFenced, acknowledgement());
    baseline(&mut bridge, &host);
    host.send(timeline_frame(1, 1, true));
    host.flush();
    let snapshot = bridge.poll_view().snapshot.unwrap();
    assert!(bridge.is_alive());
    assert_eq!(snapshot.tick(), 1);
    assert_eq!(snapshot.verified_tick(), 1);
    assert_eq!(snapshot.timeline().unwrap().pending_edits, 1);
    assert_ne!(snapshot.timeline().unwrap().checksum, snapshot.predicted().checksum());
    assert!(bridge.take_errors().is_empty());
}
