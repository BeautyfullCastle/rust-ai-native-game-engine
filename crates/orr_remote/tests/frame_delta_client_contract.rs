//! Client idle-source and ordered reset-cut regression coverage.
#![allow(clippy::disallowed_types)]
mod common;

use orr_bridge::Bridge;
use orr_ecs::Frame;
use orr_remote::frame_delta::{Encoder, FrameScope};
use orr_remote::wire::{encode_frame_record_message, FrameCodecMeta};
use orr_remote::{
    ClientError, FrameCodecPolicy, Incoming, RemoteBridge, RemoteConfig, Request, Transport,
    TxHandle, ViewDeliveryMode,
};
use orr_sample::physics_game::PhysGame;
use orr_sim::Simulation;
use serde_json::{json, Value as J};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(5);
fn config(url: &str) -> RemoteConfig {
    let mut config = RemoteConfig::new(url);
    config.view_delivery = ViewDeliveryMode::RequireFenced;
    config
}
fn wait_until(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !predicate() {
        assert!(Instant::now() < deadline, "condition timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn idle_websocket_outlives_full_timeout_then_starts_and_restarts() {
    let host = common::TestHost::dev();
    let timeout = Duration::from_millis(150);
    let bridge = RemoteBridge::<PhysGame>::connect_with_frame_codec(
        config(&host.url),
        FrameCodecPolicy::require_default().with_reset_timeout(timeout),
    )
    .unwrap();
    std::thread::sleep(timeout * 3);
    assert!(
        bridge.is_alive(),
        "valid inactive source timed out: {:?}",
        bridge.take_errors()
    );
    assert!(bridge.snapshot().is_none());
    let mut control = orr_remote::ErpClient::connect(&host.url, None).unwrap();
    for _ in 0..2 {
        control.call("sim.start", J::Null).unwrap();
        wait_until(|| bridge.snapshot().is_some());
        let expected = orr_remote::wire::parse_checksum(
            &control.call("sim.checksum", J::Null).unwrap()["checksum"],
        )
        .unwrap();
        assert_eq!(bridge.snapshot().unwrap().predicted().checksum(), expected);
        control.call("sim.stop", J::Null).unwrap();
        wait_until(|| bridge.snapshot().is_none());
        std::thread::sleep(timeout * 3);
        assert!(
            bridge.is_alive(),
            "stopped source timed out: {:?}",
            bridge.take_errors()
        );
    }
}

struct ScriptTx(Sender<Incoming>);
impl TxHandle for ScriptTx {
    fn send(&self, request: Request) -> Result<(), ClientError> {
        let limits = FrameCodecPolicy::require_default().limits;
        let result = match request.method.as_str() {
            "rpc.discover" => json!({"features":{"view_delivery":[1],"frame_codec":[1]}}),
            "watch.subscribe" => {
                json!({"view_delivery":1,"subscription":"7","cursor":"0","count":"0","sequence":"0","reset_generation":"1","frame_codec":{"version":1,"max_frame_bytes":limits.max_frame_bytes,"max_baseline_bytes":limits.max_baseline_bytes,"max_message_bytes":limits.max_message_bytes}})
            }
            "sim.state" => json!({"tick_rate":60,"player_count":2}),
            _ => J::Null,
        };
        self.0
            .send(Incoming::Text(
                json!({"jsonrpc":"2.0","id":request.id,"result":result}).to_string(),
            ))
            .map_err(|e| ClientError::Transport(e.to_string()))
    }
}
struct ScriptTransport {
    rx: Receiver<Incoming>,
    tx: Arc<ScriptTx>,
}
impl Transport for ScriptTransport {
    fn send(&mut self, request: Request) -> Result<(), ClientError> {
        self.tx.send(request)
    }
    fn recv(&mut self, timeout: Duration) -> Result<Option<Incoming>, ClientError> {
        match self.rx.recv_timeout(timeout) {
            Ok(message) => Ok(Some(message)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(e) => Err(ClientError::Transport(e.to_string())),
        }
    }
    fn sender(&self) -> Option<Arc<dyn TxHandle>> {
        Some(self.tx.clone())
    }
}
fn scripted(timeout: Duration) -> (RemoteBridge<PhysGame>, Sender<Incoming>) {
    let (tx, rx) = channel();
    let transport = ScriptTransport {
        rx,
        tx: Arc::new(ScriptTx(tx.clone())),
    };
    let bridge = RemoteBridge::connect_transport_with_frame_codec(
        Box::new(transport),
        config(""),
        FrameCodecPolicy::require_default().with_reset_timeout(timeout),
    )
    .unwrap();
    (bridge, tx)
}
fn cut(timeline: u64, cursor: u64, count: u64) -> J {
    json!({"subscription":"7","timeline":timeline.to_string(),"through_cursor":cursor.to_string(),"count":count.to_string(),"loss_generation":"0","lifecycle":[]})
}
fn scope(generation: u64) -> FrameScope {
    FrameScope {
        stream_generation: 7,
        play_epoch: 1,
        timeline_epoch: generation,
    }
}
fn wire(tick: u64, generation: u64, delivery: J) -> Incoming {
    let limits = FrameCodecPolicy::require_default().limits;
    let mut encoder = Encoder::new(limits.max_frame_bytes, limits.max_baseline_bytes);
    let mut frame = Frame::new(Simulation::<PhysGame>::build_registry());
    frame.set_tick(tick);
    let record = encoder.encode(&frame, scope(generation)).unwrap();
    Incoming::Wire(
        encode_frame_record_message(
            &json!({"tick":tick,"timeline":null,"play_epoch":"1","delivery":delivery}),
            FrameCodecMeta {
                subscription: 7,
                sequence: generation,
                reset_generation: generation,
            },
            &record,
            limits,
        )
        .unwrap(),
    )
}
fn notice(delivery: J) -> Incoming {
    Incoming::Text(json!({"jsonrpc":"2.0","method":"watch.frame_codec_reset","params":{"subscription":"7","reset_generation":"2","next_sequence":"2","scope":{"stream_generation":"7","play_epoch":"1","timeline_epoch":"2"},"delivery":delivery}}).to_string())
}
#[test]
fn reset_full_must_match_each_announced_delivery_coordinate() {
    for key in ["timeline", "through_cursor", "count"] {
        let (mut bridge, tx) = scripted(WAIT);
        tx.send(wire(10, 1, cut(1, 0, 0))).unwrap();
        wait_until(|| bridge.snapshot().is_some());
        let original = bridge.snapshot().unwrap();
        assert_eq!(bridge.poll_view().snapshot.unwrap().seq(), original.seq());
        tx.send(notice(cut(2, 1, 1))).unwrap();
        let mut mismatch = cut(2, 1, 1);
        mismatch[key] = json!("3");
        tx.send(wire(20, 2, mismatch)).unwrap();
        wait_until(|| !bridge.is_alive() || bridge.snapshot().is_some_and(|s| s.tick() == 20));
        assert!(!bridge.is_alive(), "mismatched {key} was published");
        assert_eq!(bridge.snapshot().unwrap().seq(), original.seq());
        let mailbox = bridge.poll_view();
        assert_eq!(mailbox.snapshot.unwrap().seq(), original.seq());
        assert!(mailbox.resync.is_none());
        assert_eq!(bridge.metrics().frames, 1);
        assert_eq!(
            bridge.snapshot().unwrap().predicted().checksum(),
            original.predicted().checksum()
        );
        assert!(bridge
            .take_errors()
            .iter()
            .any(|e| e.message.contains("ordered reset announcement")));
    }
}
#[test]
fn matching_reset_cut_publishes_and_missing_full_still_times_out() {
    let (bridge, tx) = scripted(Duration::from_millis(150));
    tx.send(wire(10, 1, cut(1, 0, 0))).unwrap();
    wait_until(|| bridge.snapshot().is_some());
    tx.send(notice(cut(2, 1, 1))).unwrap();
    tx.send(wire(20, 2, cut(2, 1, 1))).unwrap();
    wait_until(|| bridge.snapshot().is_some_and(|s| s.tick() == 20));
    assert!(bridge.is_alive());
    let (missing, _tx) = scripted(Duration::from_millis(150));
    wait_until(|| !missing.is_alive());
    assert!(missing
        .take_errors()
        .iter()
        .any(|e| e.message.contains("deadline")));
}

#[test]
fn announced_active_reset_still_requires_full_before_deadline() {
    let (bridge, tx) = scripted(Duration::from_millis(150));
    tx.send(wire(10, 1, cut(1, 0, 0))).unwrap();
    wait_until(|| bridge.snapshot().is_some());
    tx.send(notice(cut(2, 1, 1))).unwrap();
    wait_until(|| !bridge.is_alive());
    assert_eq!(bridge.snapshot().unwrap().tick(), 10);
    assert!(bridge
        .take_errors()
        .iter()
        .any(|e| e.message.contains("deadline")));
}

#[test]
fn malformed_inactive_cut_cannot_clear_a_published_snapshot() {
    let (bridge, tx) = scripted(WAIT);
    tx.send(wire(10, 1, cut(1, 0, 0))).unwrap();
    wait_until(|| bridge.snapshot().is_some());
    let mut invalid = cut(2, 0, 0);
    invalid["subscription"] = json!("8");
    tx.send(Incoming::Text(
        json!({"jsonrpc":"2.0","method":"watch.view.inactive","params":{"delivery":invalid}})
            .to_string(),
    ))
    .unwrap();
    wait_until(|| !bridge.is_alive());
    assert_eq!(bridge.snapshot().unwrap().tick(), 10);
}

#[test]
fn inactivity_cannot_cancel_an_announced_reset() {
    let (bridge, tx) = scripted(WAIT);
    tx.send(wire(10, 1, cut(1, 0, 0))).unwrap();
    wait_until(|| bridge.snapshot().is_some());
    tx.send(notice(cut(2, 1, 1))).unwrap();
    tx.send(Incoming::Text(
        json!({"jsonrpc":"2.0","method":"watch.view.inactive","params":{"delivery":cut(2, 1, 1)}})
            .to_string(),
    ))
    .unwrap();
    wait_until(|| !bridge.is_alive());
    assert_eq!(bridge.snapshot().unwrap().tick(), 10);
    assert!(bridge
        .take_errors()
        .iter()
        .any(|e| e.message.contains("interrupted an announced Full reset")));
}
