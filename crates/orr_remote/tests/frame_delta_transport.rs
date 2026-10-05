//! Negotiated transport boundaries, including real WebSocket compatibility.
#![allow(clippy::disallowed_types)]

mod common;

use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use orr_bridge::Bridge;
use orr_ecs::{ComponentRegistry, ComponentRegistryBuilder, Frame};
use orr_remote::frame_delta::{Decoder, Encoder, FrameRecord, FrameScope};
use orr_remote::wire::{
    decode_frame_message, decode_frame_record_message, encode_frame_record_message,
    FrameCodecLimits, FrameCodecMeta,
};
use orr_remote::{
    ClientError, FrameCodecMode, FrameCodecPolicy, Incoming, RemoteBridge, RemoteConfig,
    RemoteViewDelivery, Request, Transport, TxHandle, ViewDeliveryMode,
};
use orr_sample::physics_game::PhysGame;
use orr_sim::Simulation;
use serde_json::{json, Value as J};

const WAIT: Duration = Duration::from_secs(10);
const SCOPE: FrameScope = FrameScope {
    stream_generation: 7,
    play_epoch: 11,
    timeline_epoch: 1,
};

fn limits() -> FrameCodecLimits {
    FrameCodecLimits {
        max_frame_bytes: 64 * 1024,
        max_baseline_bytes: 64 * 1024,
        max_message_bytes: 128 * 1024,
    }
}

fn policy(mode: FrameCodecMode) -> FrameCodecPolicy {
    FrameCodecPolicy {
        mode,
        limits: limits(),
        reset_timeout: WAIT,
    }
}

fn fixture(tick: u64) -> (Arc<ComponentRegistry>, Frame) {
    let mut builder = ComponentRegistryBuilder::new();
    builder.register_component::<u32>("Counter");
    let registry = builder.build();
    let mut frame = Frame::new(registry.clone());
    for value in 0..256_u32 {
        let entity = frame.spawn();
        frame.add(entity, value);
    }
    frame.set_tick(tick);
    (registry, frame)
}

fn identity(sequence: u64) -> FrameCodecMeta {
    FrameCodecMeta {
        subscription: 7,
        sequence,
        reset_generation: 1,
    }
}

#[test]
fn refused_enqueue_does_not_advance_the_next_record_base() {
    let (_, first) = fixture(10);
    let mut encoder = Encoder::new(64 * 1024, 64 * 1024);
    let initial = encoder.prepare(&first, SCOPE).unwrap();
    encoder.commit(initial).unwrap();
    let committed = encoder.baseline_stamp();

    let mut rejected = first.clone();
    rejected.set_tick(11);
    let candidate = encoder.prepare(&rejected, SCOPE).unwrap();
    assert_eq!(encoder.baseline_stamp(), committed);
    // A false bounded enqueue result drops this prepared state.
    drop(candidate);

    let mut accepted = first.clone();
    accepted.set_tick(12);
    let candidate = encoder.prepare(&accepted, SCOPE).unwrap();
    match candidate.delta_record().unwrap() {
        FrameRecord::Delta { base, target, .. } => {
            assert_eq!(Some(*base), committed);
            assert_eq!(target.tick, 12);
        }
        _ => panic!("the sparse candidate must name the admitted base"),
    }
    encoder.commit(candidate).unwrap();
    assert_eq!(encoder.baseline_stamp().unwrap().tick, 12);
}

#[test]
fn refused_publication_preserves_decoder_until_the_next_valid_record() {
    let (registry, first) = fixture(10);
    let mut encoder = Encoder::new(64 * 1024, 64 * 1024);
    let initial = encoder.encode(&first, SCOPE).unwrap();
    let mut decoder = Decoder::new(registry, 64 * 1024, 64 * 1024);
    decoder.decode(&initial).unwrap();
    let committed = decoder.baseline_stamp();

    let mut next = first.clone();
    next.set_tick(11);
    let prepared = encoder.prepare(&next, SCOPE).unwrap();
    let record = prepared.delta_record().unwrap();
    let publication = decoder.prepare_decode(record).unwrap();
    assert_eq!(publication.frame().to_bytes(), next.to_bytes());
    assert_eq!(decoder.baseline_stamp(), committed);
    // Outer subscription/mailbox validation rejects the prepared publication.
    drop(publication);
    assert_eq!(decoder.baseline_stamp(), committed);
    let publication = decoder.prepare_decode(record).unwrap();
    let result = decoder.commit(publication).unwrap();
    assert_eq!(result.to_bytes(), next.to_bytes());
    assert_eq!(decoder.baseline_stamp().unwrap().tick, 11);
}

#[test]
fn negotiated_wire_restores_exact_bytes_and_keeps_u64_identity_exact() {
    let (registry, first) = fixture(10);
    let mut encoder = Encoder::new(64 * 1024, 64 * 1024);
    let record = encoder.encode(&first, SCOPE).unwrap();
    let context = FrameCodecMeta {
        subscription: 7,
        sequence: u64::MAX - 1,
        reset_generation: 1,
    };
    let bytes =
        encode_frame_record_message(&json!({"tick":10}), context, &record, limits()).unwrap();
    assert_eq!(&bytes[..4], b"ORRS");
    assert_eq!(bytes[4], 2);
    let (_, actual_context, restored) = decode_frame_record_message(&bytes, limits()).unwrap();
    assert_eq!(actual_context.sequence, context.sequence);
    assert_eq!(actual_context.subscription, context.subscription);
    let mut decoder = Decoder::new(registry, 64 * 1024, 64 * 1024);
    assert_eq!(
        decoder.decode(&restored).unwrap().to_bytes(),
        first.to_bytes()
    );
    assert!(
        decode_frame_message(&bytes).is_err(),
        "v1 must not reinterpret a v2 record"
    );
}

#[test]
fn negotiated_wire_rejects_hostile_prefix_reserved_bytes_and_message_cap() {
    let (_, first) = fixture(10);
    let mut encoder = Encoder::new(64 * 1024, 64 * 1024);
    let record = encoder.encode(&first, SCOPE).unwrap();
    let bytes = encode_frame_record_message(&json!({}), identity(1), &record, limits()).unwrap();
    let mut reserved = bytes.clone();
    reserved[5] = 1;
    assert!(decode_frame_record_message(&reserved, limits()).is_err());
    let mut small = limits();
    small.max_message_bytes = bytes.len() - 1;
    assert!(decode_frame_record_message(&bytes, small).is_err());
    let metadata_len = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let payload = 12 + metadata_len;
    let mut hostile = bytes.clone();
    hostile[payload..payload + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode_frame_record_message(&hostile, limits()).is_err());
    assert!(decode_frame_record_message(&bytes[..bytes.len() - 1], limits()).is_err());
}

fn wait_snapshot(bridge: &RemoteBridge<PhysGame>, tick: u64) -> orr_bridge::Snapshot {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(snapshot) = bridge.snapshot().filter(|snapshot| snapshot.tick() == tick) {
            return snapshot;
        }
        assert!(
            bridge.is_alive(),
            "negotiated bridge failed: {:?}",
            bridge.take_errors()
        );
        assert!(
            Instant::now() < deadline,
            "no negotiated snapshot for tick {tick}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn new_host_preserves_legacy_websocket_full_lz4() {
    let host = common::TestHost::dev();
    let mut control = orr_remote::ErpClient::connect(&host.url, None).unwrap();
    control.call("sim.start", J::Null).unwrap();
    let mut viewer = orr_remote::ErpClient::connect(&host.url, None).unwrap();
    let acknowledgement = viewer
        .call(
            "watch.subscribe",
            json!({"topics":["frames"],"source":"sim"}),
        )
        .unwrap();
    assert!(acknowledgement.get("frame_codec").is_none());
    let bytes = viewer.wait_frame(WAIT).unwrap().unwrap();
    assert_eq!(bytes[4], 1);
    let (_, full) = decode_frame_message(&bytes).unwrap();
    let frame = Frame::from_bytes(Simulation::<PhysGame>::build_registry(), &full).unwrap();
    assert_eq!(
        Some(frame.checksum()),
        orr_remote::wire::parse_checksum(
            &control.call("sim.checksum", J::Null).unwrap()["checksum"]
        )
    );
}

#[test]
fn negotiated_websocket_seek_and_independent_subscribers_restore_host_checksum() {
    let host = common::TestHost::dev();
    let mut control = orr_remote::ErpClient::connect(&host.url, None).unwrap();
    control.call("sim.start", J::Null).unwrap();
    let mut config = RemoteConfig::new(&host.url);
    config.view_delivery = ViewDeliveryMode::RequireFenced;
    let first = RemoteBridge::<PhysGame>::connect_with_frame_codec(
        config.clone(),
        policy(FrameCodecMode::Require),
    )
    .unwrap();
    wait_snapshot(&first, 0);
    control.call("sim.step", json!({"n":8})).unwrap();
    wait_snapshot(&first, 8);
    let second =
        RemoteBridge::<PhysGame>::connect_with_frame_codec(config, policy(FrameCodecMode::Require))
            .unwrap();
    wait_snapshot(&second, 8);
    control.call("sim.step", json!({"n":4})).unwrap();
    let checksum = orr_remote::wire::parse_checksum(
        &control.call("sim.checksum", J::Null).unwrap()["checksum"],
    )
    .unwrap();
    assert_eq!(wait_snapshot(&first, 12).predicted().checksum(), checksum);
    assert_eq!(wait_snapshot(&second, 12).predicted().checksum(), checksum);
    control.call("sim.seek", json!({"tick":4})).unwrap();
    let checksum = orr_remote::wire::parse_checksum(
        &control.call("sim.checksum", J::Null).unwrap()["checksum"],
    )
    .unwrap();
    assert_eq!(wait_snapshot(&first, 4).predicted().checksum(), checksum);
    assert_eq!(wait_snapshot(&second, 4).predicted().checksum(), checksum);
    assert!(first.is_alive() && second.is_alive());
}

#[test]
fn negotiated_zero_retention_accepts_successive_full_and_backward_reset() {
    let host = common::TestHost::dev();
    let mut control = orr_remote::ErpClient::connect(&host.url, None).unwrap();
    control.call("sim.start", J::Null).unwrap();
    let mut config = RemoteConfig::new(&host.url);
    config.view_delivery = ViewDeliveryMode::RequireFenced;
    let mut policy = policy(FrameCodecMode::Require);
    policy.limits.max_baseline_bytes = 0;
    let bridge = RemoteBridge::<PhysGame>::connect_with_frame_codec(config, policy).unwrap();
    wait_snapshot(&bridge, 0);
    for tick in [2, 4] {
        control.call("sim.step", json!({"n":2})).unwrap();
        let expected = orr_remote::wire::parse_checksum(
            &control.call("sim.checksum", J::Null).unwrap()["checksum"],
        )
        .unwrap();
        assert_eq!(
            wait_snapshot(&bridge, tick).predicted().checksum(),
            expected
        );
    }
    control.call("sim.seek", json!({"tick":1})).unwrap();
    let expected = orr_remote::wire::parse_checksum(
        &control.call("sim.checksum", J::Null).unwrap()["checksum"],
    )
    .unwrap();
    assert_eq!(wait_snapshot(&bridge, 1).predicted().checksum(), expected);
    assert!(bridge.is_alive());
}

#[test]
fn negotiated_baseline_cap_can_exceed_the_independent_frame_cap() {
    let host = common::TestHost::dev();
    let mut control = orr_remote::ErpClient::connect(&host.url, None).unwrap();
    control.call("sim.start", J::Null).unwrap();
    let mut config = RemoteConfig::new(&host.url);
    config.view_delivery = ViewDeliveryMode::RequireFenced;
    let mut policy = policy(FrameCodecMode::Require);
    policy.limits.max_baseline_bytes = 2 * policy.limits.max_frame_bytes;
    let bridge = RemoteBridge::<PhysGame>::connect_with_frame_codec(config, policy).unwrap();
    wait_snapshot(&bridge, 0);
    assert!(bridge.is_alive());
}

struct OldHostTx {
    replies: Sender<Incoming>,
    requests: Arc<Mutex<Vec<Request>>>,
    advertise_codec: bool,
    subscription: &'static str,
}

impl TxHandle for OldHostTx {
    fn send(&self, request: Request) -> Result<(), ClientError> {
        let result = match request.method.as_str() {
            "rpc.discover" if self.advertise_codec => {
                json!({"features":{"view_delivery":[1],"frame_codec":[1]}})
            }
            "rpc.discover" => json!({"features":{"view_delivery":[1]}}),
            "watch.subscribe" if self.advertise_codec => json!({
                "topics":["frames","events","notes"],"view_delivery":1,
                "subscription":self.subscription,"cursor":"0","count":"0",
                "sequence":"0","reset_generation":"1",
                "frame_codec":{"version":1,"max_frame_bytes":limits().max_frame_bytes,
                    "max_baseline_bytes":limits().max_baseline_bytes,
                    "max_message_bytes":limits().max_message_bytes}}),
            "watch.subscribe" => {
                json!({"topics":["frames","events","notes"],"view_delivery":1,"subscription":"7","cursor":"0","count":"0"})
            }
            "sim.state" => json!({"tick_rate":60,"player_count":2}),
            _ => J::Null,
        };
        let id = request.id;
        self.requests.lock().unwrap().push(request);
        self.replies
            .send(Incoming::Text(
                json!({"jsonrpc":"2.0","id":id,"result":result}).to_string(),
            ))
            .map_err(|e| ClientError::Transport(e.to_string()))
    }
}

struct OldHostTransport {
    replies: Receiver<Incoming>,
    sender: Arc<OldHostTx>,
}

impl Transport for OldHostTransport {
    fn send(&mut self, request: Request) -> Result<(), ClientError> {
        self.sender.send(request)
    }
    fn recv(&mut self, timeout: Duration) -> Result<Option<Incoming>, ClientError> {
        match self.replies.recv_timeout(timeout) {
            Ok(message) => Ok(Some(message)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => {
                Err(ClientError::Transport("old host closed".into()))
            }
        }
    }
    fn sender(&self) -> Option<Arc<dyn TxHandle>> {
        Some(self.sender.clone())
    }
}

fn old_host() -> (Box<dyn Transport>, Arc<Mutex<Vec<Request>>>) {
    scripted_host(false, "7")
}

fn scripted_host(
    advertise_codec: bool,
    subscription: &'static str,
) -> (Box<dyn Transport>, Arc<Mutex<Vec<Request>>>) {
    let (tx, replies) = channel();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let sender = Arc::new(OldHostTx {
        replies: tx,
        requests: requests.clone(),
        advertise_codec,
        subscription,
    });
    (Box::new(OldHostTransport { replies, sender }), requests)
}

#[test]
fn prefer_old_host_negotiates_legacy_before_activation() {
    let (transport, requests) = old_host();
    let mut config = RemoteConfig::new("");
    config.view_delivery = ViewDeliveryMode::RequireFenced;
    let bridge = RemoteBridge::<PhysGame>::connect_transport_with_frame_codec(
        transport,
        config,
        policy(FrameCodecMode::Prefer),
    )
    .unwrap();
    assert_eq!(bridge.view_delivery(), RemoteViewDelivery::Fenced);
    let requests = requests.lock().unwrap();
    assert!(requests
        .iter()
        .any(|request| request.method == "rpc.discover"));
    let subscribe = requests
        .iter()
        .find(|request| request.method == "watch.subscribe")
        .unwrap();
    assert!(subscribe.params.get("frame_codec").is_none());
}

#[test]
fn require_old_host_refuses_without_subscribing_or_downgrading() {
    let (transport, requests) = old_host();
    let mut config = RemoteConfig::new("");
    config.view_delivery = ViewDeliveryMode::RequireFenced;
    let result = RemoteBridge::<PhysGame>::connect_transport_with_frame_codec(
        transport,
        config,
        policy(FrameCodecMode::Require),
    );
    assert!(result.is_err());
    assert!(!requests
        .lock()
        .unwrap()
        .iter()
        .any(|request| request.method == "watch.subscribe"));
}

#[test]
fn negotiated_zero_subscription_ack_is_rejected_before_state_publication() {
    let (transport, _) = scripted_host(true, "0");
    let mut config = RemoteConfig::new("");
    config.view_delivery = ViewDeliveryMode::RequireFenced;
    let result = RemoteBridge::<PhysGame>::connect_transport_with_frame_codec(
        transport,
        config,
        policy(FrameCodecMode::Require),
    );
    match result {
        Err(message) => assert!(message.contains("zero subscription"), "{message}"),
        Ok(_) => panic!("a zero subscription acknowledgement must not activate a bridge"),
    }
}
