//! Yard3D recovery through real bridges and ERP sockets. The relay test delays
//! actual downstream WebSocket payloads; the server's finalized input log is
//! the independent simulation oracle. Neither test constructs recovery flags.
#![allow(clippy::disallowed_types)] // socket/progress deadlines are test-side
#![allow(clippy::float_arithmetic)] // decoded view transforms, never sim state

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use orr_bridge::{
    Bridge, BridgeConfig, BridgeError, BridgeEvent, Lifecycle, Pacing, PlayHost, RelayHost,
    RelayHostOptions, RelayMetrics, SimControl, Snapshot, Threaded, ThreadedConfig, ViewResync,
    ViewUpdate,
};
use orr_relay_net::{ConnectOptions, ListenOptions, TransportKind, Trust, connect, listen};
use orr_remote::yard3d::yard3d_doc;
use orr_remote::{Auth, ErpClient, LocalHost, ServerConfig, codec};
use orr_remote::{ClientPump, ClientSession, ClientSessionHook, SessionError, SessionErrorKind};
use orr_sample::yard3d_game::{
    NoCommand, NoEvent, SPAWN_BALL, TICK_RATE, Yard3D, YardConfig, YardInput,
};
use orr_sample::yard3d_stream::{KIND_DYNAMIC, KIND_STATIC, Yard3dStreamProducer};
use orr_sample::yard3d_view::YardExtractor;
use orr_server::serve::run_wall_clock;
use orr_server::{RelayServer, RoomConfig};
use orr_session::{
    ControlOp, DumpCollector, PlayConfig, PlaySession, RelayClient, RelayClientConfig,
};
use orr_sim::{PlayerSlot, Simulation, TickInputs};
use orr_view::{Extracted3, Extractor3, InterpMode, Shape3};
use orr_viewstream::{
    EventBatch, FLAG_DISCONTINUITY, FLAG_EVENTS_RESET, FLAG_PAUSED, FLAG_ROLLED_BACK, FrameMeta,
    MODE_NONE, MODE_PREDICTION, MODE_SNAPSHOT, MSG_FRAME3D, Pose3, SHAPE3_BOX, SHAPE3_CAPSULE,
    SHAPE3_PLANE, SHAPE3_SPHERE, STYLE_CHECKER, Schema, StreamProducer, ViewFrame3, color_to_u8,
    entity_id,
};
use serde_json::{Value as J, json};
use tungstenite::Message;
use tungstenite::client::IntoClientRequest;
use tungstenite::handshake::server::{ErrorResponse, Request, Response};

const WAIT: Duration = Duration::from_secs(20);
const PLAYERS: u8 = 2;
const SEED: u64 = 42;
const BUILD: u64 = 0x5941_5244_5245_434F;
const MAX_PREDICTION: u32 = 8;
const MAX_RECEIPTS: usize = 256;

fn scene() -> YardConfig {
    YardConfig {
        rain_per_second: 0,
        max_entities: 128,
        ..YardConfig::new(4)
    }
}

fn save_evidence(name: &str, value: &J) {
    println!("{name}: {value}");
    if let Some(dir) = std::env::var_os("ORR_YARD3D_RECOVERY_EVIDENCE_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).expect("create recovery evidence directory");
        let path = dir.join(format!("{name}.json"));
        assert!(
            !path.exists(),
            "each actual execution owns a fresh evidence file"
        );
        std::fs::write(path, serde_json::to_vec_pretty(value).unwrap())
            .expect("save actual receipt");
    }
}

#[derive(Clone)]
struct PollReceipt {
    snapshot: Option<Snapshot>,
    resync: Option<ViewResync>,
    event_count: usize,
}

/// Observes then forwards the SAME non-Clone ViewUpdate to production pump.
struct ObservingBridge<'a> {
    inner: &'a mut Threaded<Yard3D>,
    receipt: &'a mut Option<PollReceipt>,
}

impl Bridge<Yard3D> for ObservingBridge<'_> {
    fn tick_rate(&self) -> u32 {
        self.inner.tick_rate()
    }
    fn local_slot(&self) -> PlayerSlot {
        self.inner.local_slot()
    }
    fn player_count(&self) -> u8 {
        self.inner.player_count()
    }
    fn set_input(&mut self, player: PlayerSlot, input: YardInput) -> Result<(), BridgeError> {
        self.inner.set_input(player, input)
    }
    fn send_command(&mut self, command: NoCommand) -> Result<(), BridgeError> {
        self.inner.send_command(command)
    }
    fn update(&mut self, elapsed: Duration) {
        self.inner.update(elapsed);
    }
    fn snapshot(&self) -> Option<Snapshot> {
        self.inner.snapshot()
    }
    fn drain_events(&mut self) -> Vec<BridgeEvent<NoEvent>> {
        self.inner.drain_events()
    }
    fn poll_view(&mut self) -> ViewUpdate<NoEvent> {
        let actual = self.inner.poll_view();
        *self.receipt = Some(PollReceipt {
            snapshot: actual.snapshot.clone(),
            resync: actual.resync.clone(),
            event_count: actual.events.len(),
        });
        actual
    }
    fn is_alive(&self) -> bool {
        self.inner.is_alive()
    }
}

#[derive(Clone)]
struct Publication {
    poll: PollReceipt,
    bytes: Vec<u8>,
    event_bytes: Option<Vec<u8>>,
}

#[derive(Clone)]
struct SessionHandle {
    bridge: Arc<Mutex<Threaded<Yard3D>>>,
    receipts: Arc<Mutex<VecDeque<Publication>>>,
    hold_pump: Arc<AtomicBool>,
    freeze_next: Arc<AtomicBool>,
    freeze_resync: Arc<AtomicBool>,
    target_tick: Arc<AtomicU64>,
    metrics: Option<Arc<RelayMetrics>>,
}

impl SessionHandle {
    fn snapshot(&self) -> Snapshot {
        self.bridge
            .lock()
            .unwrap()
            .snapshot()
            .expect("real bridge snapshot")
    }

    fn publication(&self, seq: u64) -> Publication {
        self.receipts
            .lock()
            .unwrap()
            .iter()
            .find(|p| ViewFrame3::decode(&p.bytes).unwrap().seq == seq)
            .cloned()
            .expect("wire sequence has its actual pump receipt")
    }
}

struct YardClientSession {
    handle: SessionHandle,
    producer: Yard3dStreamProducer,
}

fn adapter(
    bridge: Threaded<Yard3D>,
    metrics: Option<Arc<RelayMetrics>>,
) -> (YardClientSession, SessionHandle) {
    let handle = SessionHandle {
        bridge: Arc::new(Mutex::new(bridge)),
        receipts: Arc::new(Mutex::new(VecDeque::new())),
        hold_pump: Arc::new(AtomicBool::new(false)),
        freeze_next: Arc::new(AtomicBool::new(true)),
        freeze_resync: Arc::new(AtomicBool::new(false)),
        target_tick: Arc::new(AtomicU64::new(0)),
        metrics,
    };
    (
        YardClientSession {
            handle: handle.clone(),
            producer: Yard3dStreamProducer::new(BUILD, PLAYERS),
        },
        handle,
    )
}

fn session_error(kind: SessionErrorKind, message: impl Into<String>) -> SessionError {
    SessionError {
        kind,
        message: message.into(),
    }
}

impl ClientSession for YardClientSession {
    fn pump(&mut self) -> ClientPump {
        // Only presentation is held. The real sim/relay thread keeps advancing.
        // A one-frame cache lets both ERP transports observe the same publication.
        if self.handle.hold_pump.load(Ordering::Acquire) {
            return ClientPump::default();
        }
        let mut bridge = self.handle.bridge.lock().unwrap();
        let mut receipt = None;
        let pumped = self.producer.pump(&mut ObservingBridge {
            inner: &mut bridge,
            receipt: &mut receipt,
        });
        let poll = receipt.expect("production pump called actual poll_view");
        let events = (!pumped.events.is_empty()).then(|| {
            EventBatch {
                events: pumped.events,
            }
            .encode()
        });
        let frame = pumped.frame.map(|frame| {
            let target = self.handle.target_tick.load(Ordering::Acquire);
            let target_rollback = target > 0
                && frame
                    .rollback
                    .is_some_and(|(from, to)| from <= target && target <= to);
            if self.handle.freeze_next.swap(false, Ordering::AcqRel)
                || (self.handle.freeze_resync.load(Ordering::Acquire) && poll.resync.is_some())
                || target_rollback
            {
                self.handle.hold_pump.store(true, Ordering::Release);
            }
            let bytes = frame.encode();
            let mut all = self.handle.receipts.lock().unwrap();
            if all.len() == MAX_RECEIPTS {
                all.pop_front();
            }
            all.push_back(Publication {
                poll,
                bytes: bytes.clone(),
                event_bytes: events.clone(),
            });
            bytes
        });
        ClientPump { frame, events }
    }

    fn schema(&self) -> Option<Schema> {
        Some(self.producer.schema().clone())
    }
    fn status(&self) -> J {
        let snap = self.handle.snapshot();
        let bridge = self.handle.bridge.lock().unwrap();
        let slot = bridge.local_slot().0;
        let m = self.handle.metrics.as_ref().map(|m| m.status());
        let playing = !snap.timeline().is_some_and(|timeline| !timeline.playing);
        json!({
            "mode":"client", "state":if playing { "playing" } else { "paused" }, "playing":playing,
            "slot":slot, "player_count":PLAYERS,
            "head_tick":snap.tick(), "verified_tick":snap.verified_tick(),
            "rollbacks":snap.stats().rollbacks,
            "resim_ticks":m.map_or(0, |m| m.resim_ticks),
            "desyncs":m.map_or(0, |m| m.desyncs),
        })
    }
    fn confirmed_checksum(&self, tick: u64) -> Option<(u64, u64)> {
        self.handle.metrics.as_ref().and_then(|m| {
            if tick == 0 {
                m.last_checksum()
            } else {
                m.checksum_at(tick).map(|c| (tick, c))
            }
        })
    }
    fn set_input(&mut self, player: u8, bytes: &[u8]) -> Result<(), SessionError> {
        let mut bridge = self.handle.bridge.lock().unwrap();
        if player != bridge.local_slot().0 {
            return Err(session_error(
                SessionErrorKind::Arg,
                "only this client's input is accepted",
            ));
        }
        let input = bytemuck::try_pod_read_unaligned::<YardInput>(bytes)
            .map_err(|_| session_error(SessionErrorKind::Arg, "YardInput must be 32 bytes"))?;
        bridge
            .set_input(PlayerSlot(player), input)
            .map_err(|e| session_error(SessionErrorKind::Host, e.to_string()))
    }
    fn send_command(&mut self, player: u8, bytes: &[u8]) -> Result<(), SessionError> {
        let mut bridge = self.handle.bridge.lock().unwrap();
        if player != bridge.local_slot().0 {
            return Err(session_error(
                SessionErrorKind::Arg,
                "only this client's commands are accepted",
            ));
        }
        let command = bytemuck::try_pod_read_unaligned::<NoCommand>(bytes)
            .map_err(|_| session_error(SessionErrorKind::Arg, "NoCommand must be four bytes"))?;
        bridge
            .send_command(command)
            .map_err(|e| session_error(SessionErrorKind::Host, e.to_string()))
    }
}

fn erp_host(session: YardClientSession) -> LocalHost {
    LocalHost::spawn::<Yard3D>(move || {
        let mut cfg = ServerConfig::new(Auth::DevNoAuth);
        cfg.limits.player_count = PLAYERS;
        cfg.limits.tick_rate = TICK_RATE;
        cfg.limits.build_id = BUILD;
        cfg.limits.client_session = Some(ClientSessionHook::new(session));
        Ok((yard3d_doc(scene()).map_err(|e| e.to_string())?, cfg))
    })
    .expect("serve test-only Yard3D client adapter through actual ERP")
}

struct TcpView {
    writer: TcpStream,
    reader: BufReader<TcpStream>,
}

impl TcpView {
    fn connect(url: &str) -> Self {
        let stream =
            TcpStream::connect(url.strip_prefix("ws://").unwrap()).expect("TCP ERP socket");
        stream.set_read_timeout(Some(WAIT)).unwrap();
        Self {
            writer: stream.try_clone().unwrap(),
            reader: BufReader::new(stream),
        }
    }
    fn next(&mut self, deadline: Instant) -> J {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "TCP ERP progress deadline");
        self.reader.get_ref().set_read_timeout(Some(left)).unwrap();
        let mut line = String::new();
        assert_ne!(
            self.reader.read_line(&mut line).expect("TCP ERP JSON line"),
            0
        );
        serde_json::from_str(&line).expect("valid TCP ERP JSON")
    }
    fn subscribe(&mut self) -> J {
        writeln!(self.writer, "{}", json!({"jsonrpc":"2.0","id":1,"method":"watch.subscribe","params":{"topics":["viewstream"],"max_fps":1000}})).unwrap();
        let (mut response, mut schema) = (false, None);
        let end = Instant::now() + WAIT;
        while !response || schema.is_none() {
            let message = self.next(end);
            if message["id"] == 1 {
                assert_eq!(message["result"]["topics"], json!(["viewstream"]));
                response = true;
            } else if message["method"] == "watch.viewstream.schema" {
                schema = Some(message["params"].clone());
            }
        }
        schema.unwrap()
    }
    fn frame(&mut self, seq: u64) -> Vec<u8> {
        let end = Instant::now() + WAIT;
        loop {
            let message = self.next(end);
            if message["method"] != "watch.viewstream" {
                continue;
            }
            assert_eq!(message["params"]["encoding"], "hex");
            let bytes = codec::hex_decode(message["params"]["data"].as_str().unwrap())
                .expect("TCP hex frame");
            if ViewFrame3::decode(&bytes).unwrap().seq == seq {
                return bytes;
            }
        }
    }
}

fn views(url: &str) -> (ErpClient, TcpView) {
    let mut ws = ErpClient::connect_pumped(url, None).unwrap();
    ws.call(
        "watch.subscribe",
        json!({"topics":["viewstream"],"max_fps":1000}),
    )
    .unwrap();
    let schema = ws
        .wait_notification("watch.viewstream.schema", WAIT)
        .unwrap()
        .unwrap()["params"]
        .clone();
    let mut tcp = TcpView::connect(url);
    assert_eq!(schema, tcp.subscribe());
    assert_eq!(schema["game"], "Yard3D");
    assert_eq!(schema["version"], 2);
    assert_eq!(schema["frame3d"]["message_type"], MSG_FRAME3D);
    (ws, tcp)
}

fn ws_frame(ws: &mut ErpClient, accept: impl Fn(&ViewFrame3) -> bool) -> Vec<u8> {
    let end = Instant::now() + WAIT;
    loop {
        let left = end.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "matching WS Frame3 progress deadline");
        let bytes = ws
            .wait_frame(left)
            .unwrap()
            .expect("actual WS binary Frame3");
        let frame = ViewFrame3::decode(&bytes).unwrap();
        if accept(&frame) {
            return bytes;
        }
    }
}

fn assert_snapshot_fields(frame: &ViewFrame3, snap: &Snapshot) {
    assert_eq!(frame.tick, snap.tick());
    assert_eq!(frame.verified_tick, snap.verified_tick());
    let mut now = Vec::<Extracted3>::new();
    let mut before = Vec::<Extracted3>::new();
    YardExtractor.extract(snap.predicted(), &mut now);
    if let Some(prev) = snap.predicted_prev() {
        YardExtractor.extract(prev, &mut before);
    }
    assert_eq!(frame.entities.len(), now.len());
    for (actual, item) in frame.entities.iter().zip(&now) {
        let prior = if frame.flags & FLAG_DISCONTINUITY != 0 {
            item
        } else {
            before
                .iter()
                .find(|p| p.entity == item.entity)
                .unwrap_or(item)
        };
        assert_eq!(actual.id, entity_id(item.entity));
        assert_eq!(
            actual.kind,
            if item.mode == InterpMode::Prediction {
                KIND_DYNAMIC
            } else {
                KIND_STATIC
            }
        );
        assert_eq!(
            actual.mode,
            match item.mode {
                InterpMode::Prediction => MODE_PREDICTION,
                InterpMode::Snapshot => MODE_SNAPSHOT,
                InterpMode::None => MODE_NONE,
            }
        );
        let (shape, size) = match item.style.shape {
            Shape3::Sphere { radius } => (SHAPE3_SPHERE, [radius, 0.0, 0.0]),
            Shape3::Box { half } => (SHAPE3_BOX, half),
            Shape3::Capsule {
                half_length,
                radius,
            } => (SHAPE3_CAPSULE, [radius, half_length, 0.0]),
            Shape3::Plane { half_x, half_z } => (SHAPE3_PLANE, [half_x, 0.0, half_z]),
        };
        assert_eq!((actual.shape, actual.size), (shape, size));
        assert_eq!(actual.rgba, item.style.color.map(color_to_u8));
        assert_eq!(actual.roughness, color_to_u8(item.style.roughness));
        assert_eq!(actual.metallic, color_to_u8(item.style.metallic));
        assert_eq!(
            actual.style_flags,
            if item.style.checker { STYLE_CHECKER } else { 0 }
        );
        assert_eq!(
            actual.prev,
            Pose3 {
                pos: prior.transform.pos.to_array(),
                rot: prior.transform.rot.to_array()
            }
        );
        assert_eq!(
            actual.cur,
            Pose3 {
                pos: item.transform.pos.to_array(),
                rot: item.transform.rot.to_array()
            }
        );
    }
}

#[test]
fn threaded_yard3d_overrun_resets_the_same_snapshot_over_erp() {
    let mut config = PlayConfig::new(PLAYERS, SEED, TICK_RATE);
    config.start_paused = true;
    config.build_id = BUILD;
    let bridge = Threaded::spawn(
        move || PlayHost::new(PlaySession::<Yard3D>::new(config, scene()), PlayerSlot(0)),
        BridgeConfig::<Yard3D>::default().with_view_event_capacity(1),
        ThreadedConfig {
            pacing: Pacing::Manual,
            ..ThreadedConfig::default()
        },
    )
    .expect("actual paused Threaded Yard3D");
    let (session, handle) = adapter(bridge, None);
    let host = erp_host(session);
    let (mut ws, mut tcp) = views(host.url().unwrap());
    let baseline = ws_frame(&mut ws, |_| true);
    let first = ViewFrame3::decode(&baseline).unwrap();
    assert_eq!(baseline, tcp.frame(first.seq));
    let before = handle.publication(first.seq).poll.snapshot.unwrap();
    assert!(handle.hold_pump.load(Ordering::Acquire));
    assert!(!before.timeline().unwrap().playing);

    // Manual controls return only after the actual sim thread processed them.
    // The pump is held, so the capacity-one presentation channel must overflow.
    {
        let mut bridge = handle.bridge.lock().unwrap();
        for _ in 0..20 {
            bridge
                .control(ControlOp::Seek(999))
                .expect("processed real rejected seek");
        }
    }
    handle.freeze_resync.store(true, Ordering::Release);
    handle.hold_pump.store(false, Ordering::Release);
    let bytes = ws_frame(&mut ws, |f| {
        f.seq > first.seq && f.flags & FLAG_EVENTS_RESET != 0
    });
    let frame = ViewFrame3::decode(&bytes).unwrap();
    assert_eq!(
        bytes,
        tcp.frame(frame.seq),
        "one actual reset publication over WS binary and TCP hex"
    );
    let receipt = handle.publication(frame.seq);
    let after = receipt.poll.snapshot.as_ref().unwrap();
    let reset = receipt
        .poll
        .resync
        .as_ref()
        .expect("actual bounded view recovery");
    assert!(reset.generation > 0 && reset.discarded_events > 0);
    assert_eq!(receipt.poll.event_count, 0);
    assert!(
        receipt.event_bytes.is_none(),
        "Yard3D NoEvent does not invent event records"
    );
    assert_eq!(reset.head_tick, after.tick());
    assert_eq!(before.tick(), after.tick());
    assert_eq!(before.predicted().checksum(), after.predicted().checksum());
    assert_eq!(before.timeline(), after.timeline());
    let rejected = reset
        .lifecycle
        .iter()
        .find(|entry| entry.last == Lifecycle::SeekRejected { target: 999 })
        .expect("actual rejected controls survive the bounded lifecycle summary");
    assert_eq!(
        rejected.count, 20,
        "all twenty processed SeekRejected controls were coalesced"
    );
    assert_eq!(
        frame.flags & (FLAG_EVENTS_RESET | FLAG_DISCONTINUITY | FLAG_PAUSED),
        FLAG_EVENTS_RESET | FLAG_DISCONTINUITY | FLAG_PAUSED
    );
    assert!(frame.entities.iter().all(|e| e.prev == e.cur));
    assert_snapshot_fields(&frame, after);
    // The reset is a pulse. A real processed Step produces a new snapshot,
    // and both already-subscribed transports must drop their sticky reset.
    assert_eq!(
        ws.call("session.status", json!({})).unwrap()["playing"],
        false
    );
    handle
        .bridge
        .lock()
        .unwrap()
        .control(ControlOp::Step(1))
        .expect("processed real step after reset");
    handle.freeze_resync.store(false, Ordering::Release);
    handle.freeze_next.store(true, Ordering::Release);
    handle.hold_pump.store(false, Ordering::Release);
    let next_bytes = ws_frame(&mut ws, |f| f.seq > frame.seq && f.tick == after.tick() + 1);
    let next_frame = ViewFrame3::decode(&next_bytes).unwrap();
    assert_eq!(next_bytes, tcp.frame(next_frame.seq));
    let next_receipt = handle.publication(next_frame.seq);
    let next_snapshot = next_receipt.poll.snapshot.as_ref().unwrap();
    assert!(next_receipt.poll.resync.is_none());
    assert_eq!(
        next_frame.flags & FLAG_EVENTS_RESET,
        0,
        "reset sticky flag clears after its delivered baseline"
    );
    assert_ne!(
        next_frame.flags & FLAG_PAUSED,
        0,
        "explicit Step retains the paused timeline"
    );
    assert_eq!(next_snapshot.tick(), after.tick() + 1);
    assert_snapshot_fields(&next_frame, next_snapshot);
    save_evidence(
        "yard3d-overrun",
        &json!({
            "source":"actual Threaded PlayHost capacity 1 / processed SeekRejected lifecycle",
            "snapshot_seq_before":before.seq(), "snapshot_seq_after":after.seq(),
            "frame_seq":frame.seq, "tick":frame.tick, "verified_tick":frame.verified_tick,
            "checksum":format!("{:016x}",after.predicted().checksum()),
            "generation":reset.generation, "discarded_events":reset.discarded_events,
            "actual_seek_rejected_count":rejected.count,
            "event_count":receipt.poll.event_count, "wire_flags":frame.flags,
            "ws_binary_tcp_hex_equal":true, "frame_hex":codec::hex_encode(&bytes),
            "next_actual_step_tick":next_snapshot.tick(), "next_frame_seq":next_frame.seq,
            "next_wire_flags":next_frame.flags, "next_resync":false,
            "next_frame_hex":codec::hex_encode(&next_bytes), "next_ws_tcp_equal":true,
        }),
    );
}

#[derive(Clone)]
struct FinalTick {
    inputs: TickInputs<YardInput, NoCommand>,
    raw_flags: Vec<u8>,
}

struct RelayFixture {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    records: Arc<Mutex<Vec<FinalTick>>>,
    join: Option<JoinHandle<()>>,
}

impl RelayFixture {
    fn start() -> Self {
        let endpoint = listen(&ListenOptions::new(
            "127.0.0.1:0".parse().unwrap(),
            TransportKind::Ws,
        ))
        .unwrap();
        let addr = endpoint.local_addr();
        let mut server = RelayServer::new(endpoint, 11);
        let mut cfg = RoomConfig::new(PLAYERS, TICK_RATE, SEED, 32);
        cfg.build_hash =
            Simulation::<Yard3D>::with_build_id(scene(), TICK_RATE, SEED, BUILD).build_hash();
        cfg.record_all = true;
        cfg.config_blob = b"yard3d-recovery-fixed-scene-v1".to_vec();
        server.create_room(1, cfg);
        let stop = Arc::new(AtomicBool::new(false));
        let records = Arc::new(Mutex::new(Vec::new()));
        let (s, log) = (stop.clone(), records.clone());
        let join = thread::spawn(move || {
            run_wall_clock(&mut server, &s, Duration::from_millis(1), |server, _| {
                let mut log = log.lock().unwrap();
                for bundle in &server.recorded(1)[log.len()..] {
                    assert!(log.len() < 2400, "bounded finalized oracle log");
                    assert_eq!(bundle.slots.len(), usize::from(PLAYERS));
                    let mut inputs = TickInputs::new(bundle.tick, PLAYERS);
                    let mut flags = Vec::new();
                    for (slot, entry) in bundle.slots.iter().enumerate() {
                        // Both clients remain present: no FLAG_ABSENT (public protocol bit 2).
                        assert_eq!(entry.flags & 2, 0, "oracle peers remain connected");
                        inputs.set_input(
                            PlayerSlot(slot as u8),
                            bytemuck::try_pod_read_unaligned::<YardInput>(&entry.input).unwrap(),
                        );
                        for command in &entry.commands {
                            inputs.push_command(
                                PlayerSlot(slot as u8),
                                bytemuck::try_pod_read_unaligned::<NoCommand>(command).unwrap(),
                            );
                        }
                        flags.push(entry.flags);
                    }
                    log.push(FinalTick {
                        inputs,
                        raw_flags: flags,
                    });
                }
                assert_eq!(server.bad_messages(), 0);
                assert_eq!(server.room_stats(1).unwrap().desyncs, 0);
                let _ = server.drain_notes();
            });
        });
        Self {
            addr,
            stop,
            records,
            join: Some(join),
        }
    }

    fn finish(&mut self) {
        // End the observed campaign while both real clients are still alive;
        // their later shutdown must not become a new absent-player tick.
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            join.join().expect("relay server orderly campaign finish");
        }
    }
}

impl Drop for RelayFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let result = join.join();
            if !thread::panicking() {
                result.expect("relay server thread");
            }
        }
    }
}

#[derive(Default)]
struct PacketReceipt {
    holding: bool,
    held: Vec<Vec<u8>>,
    released: Vec<Vec<u8>>,
    uplink_binary: u64,
    downstream_binary: u64,
    held_messages: usize,
    released_messages: usize,
    native_subprotocol: bool,
    error: Option<String>,
}

#[allow(clippy::result_large_err)] // tungstenite handshake callback signature
fn native_proxy_protocol(
    request: &Request,
    mut response: Response,
) -> Result<Response, ErrorResponse> {
    if request
        .headers()
        .get("Sec-WebSocket-Protocol")
        .and_then(|p| p.to_str().ok())
        != Some("orrery/1")
    {
        return Err(ErrorResponse::new(Some(
            "expected native orrery/1 subprotocol".into(),
        )));
    }
    response
        .headers_mut()
        .insert("Sec-WebSocket-Protocol", "orrery/1".parse().unwrap());
    Ok(response)
}

struct DownstreamLatch {
    addr: SocketAddr,
    hold: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    receipt: Arc<Mutex<PacketReceipt>>,
    join: Option<JoinHandle<()>>,
}

impl DownstreamLatch {
    fn start(upstream: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let hold = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let receipt = Arc::new(Mutex::new(PacketReceipt::default()));
        let (h, s, r) = (hold.clone(), stop.clone(), receipt.clone());
        let join = thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let failures = r.clone();
            let result: Result<(), String> = runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).map_err(|e| e.to_string())?;
                let (socket, _) = tokio::time::timeout(WAIT, listener.accept()).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
                let downstream = tokio_tungstenite::accept_hdr_async(socket, native_proxy_protocol).await.map_err(|e| e.to_string())?;
                // The real relay rejects a bare WS handshake. Preserve the
                // native endpoint's subprotocol on both sides of this proxy.
                let mut request = format!("ws://{upstream}/").into_client_request().map_err(|e| e.to_string())?;
                request.headers_mut().insert("Sec-WebSocket-Protocol", "orrery/1".parse().unwrap());
                let (upstream, response) = tokio_tungstenite::connect_async(request).await.map_err(|e| e.to_string())?;
                if response.headers().get("Sec-WebSocket-Protocol").and_then(|p| p.to_str().ok()) != Some("orrery/1") {
                    return Err("relay did not negotiate the native subprotocol".into());
                }
                r.lock().unwrap().native_subprotocol = true;
                let (mut down_tx, mut down_rx) = downstream.split();
                let (mut up_tx, mut up_rx) = upstream.split();
                let mut pending = VecDeque::<Message>::new();
                let mut bytes = 0usize;
                let mut armed_at = None;
                let mut clock = tokio::time::interval(Duration::from_millis(1));
                loop {
                    if s.load(Ordering::Acquire) { break; }
                    let holding = h.load(Ordering::Acquire);
                    {
                        r.lock().unwrap().holding = holding;
                    }
                    if holding {
                        let at = *armed_at.get_or_insert_with(Instant::now);
                        if at.elapsed() > Duration::from_millis(500) { return Err("bounded packet latch exceeded 500 ms".into()); }
                    } else {
                        armed_at = None;
                        while let Some(message) = pending.pop_front() {
                            let binary = if let Message::Binary(payload) = &message { Some(payload.to_vec()) } else { None };
                            tokio::time::timeout(Duration::from_secs(2), down_tx.send(message)).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
                            let mut receipt = r.lock().unwrap();
                            receipt.released_messages += 1;
                            if let Some(payload) = binary { receipt.released.push(payload); }
                        }
                        bytes = 0;
                    }
                    tokio::select! {
                        message = down_rx.next() => {
                            let Some(message) = message else { break; };
                            let message = message.map_err(|e| e.to_string())?;
                            if message.is_close() { break; }
                            if message.is_binary() { r.lock().unwrap().uplink_binary += 1; }
                            up_tx.send(message).await.map_err(|e| e.to_string())?;
                        }
                        message = up_rx.next() => {
                            let Some(message) = message else { break; };
                            let message = message.map_err(|e| e.to_string())?;
                            if message.is_close() { break; }
                            if message.is_binary() {
                                r.lock().unwrap().downstream_binary += 1;
                            }
                            // Retain every non-Close message, including control
                            // frames. A release racing this read cannot let a
                            // newer message bypass the already retained suffix.
                            if h.load(Ordering::Acquire) || !pending.is_empty() {
                                bytes += message.len();
                                if pending.len() >= 128 || bytes > 128 * 1024 { return Err("bounded downstream FIFO overflow".into()); }
                                let mut receipt = r.lock().unwrap();
                                receipt.held_messages += 1;
                                if let Message::Binary(payload) = &message { receipt.held.push(payload.to_vec()); }
                                drop(receipt);
                                pending.push_back(message);
                                continue;
                            }
                            down_tx.send(message).await.map_err(|e| e.to_string())?;
                        }
                        _ = clock.tick() => {}
                    }
                }
                Ok(())
            });
            if let Err(error) = result {
                receipt_error(&failures, error);
            }
        });
        Self {
            addr,
            hold,
            stop,
            receipt,
            join: Some(join),
        }
    }
    fn check(&self) {
        let error = self.receipt.lock().unwrap().error.clone();
        assert!(error.is_none(), "actual proxy failed: {error:?}");
    }
}

fn receipt_error(receipt: &Arc<Mutex<PacketReceipt>>, error: String) {
    receipt.lock().unwrap().error = Some(error);
}

impl Drop for DownstreamLatch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let result = join.join();
            if !thread::panicking() {
                result.expect("packet proxy thread");
            }
        }
    }
}

fn relay_bridge(addr: SocketAddr, slot: u8) -> (Threaded<Yard3D>, Arc<RelayMetrics>) {
    let metrics = RelayMetrics::new();
    let observed = metrics.clone();
    let bridge = Threaded::try_spawn(
        move || {
            let options =
                ConnectOptions::new(addr.to_string(), TransportKind::Ws, Trust::InsecureDev);
            let link = connect(&options)?;
            let mut cfg = RelayClientConfig::new(1, BUILD);
            cfg.want_slot = Some(PlayerSlot(slot));
            cfg.min_delay = 2;
            cfg.max_delay = 2;
            cfg.max_prediction = MAX_PREDICTION;
            let client = RelayClient::<Yard3D, _>::new(
                cfg,
                link,
                |welcome| {
                    assert_eq!(welcome.player_count, PLAYERS);
                    assert_eq!(welcome.tick_rate, TICK_RATE);
                    assert_eq!(welcome.seed, SEED);
                    assert_eq!(welcome.config, b"yard3d-recovery-fixed-scene-v1");
                    scene()
                },
                DumpCollector::default(),
            );
            RelayHost::connect(
                client,
                RelayHostOptions {
                    connect_timeout: WAIT,
                    metrics: Some(observed),
                    ..RelayHostOptions::default()
                },
            )
            .map_err(|e| e.to_string())
        },
        BridgeConfig::default(),
        ThreadedConfig::default(),
    )
    .expect("join actual Yard3D relay room");
    (bridge, metrics)
}

fn progress_until(mut ready: impl FnMut() -> bool, message: &str) {
    let end = Instant::now() + WAIT;
    while !ready() {
        assert!(Instant::now() < end, "{message}");
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn late_confirmed_yard3d_bundle_rolls_back_and_matches_server_replay() {
    let mut server = RelayFixture::start();
    let latch = DownstreamLatch::start(server.addr);
    let address = latch.addr;
    let a = thread::spawn(move || relay_bridge(address, 0));
    let (mut b, b_metrics) = relay_bridge(server.addr, 1);
    let (a, a_metrics) = a.join().expect("client A joining thread");
    let (session, handle) = adapter(a, Some(a_metrics.clone()));
    let host = erp_host(session);
    let (mut ws, mut tcp) = views(host.url().unwrap());
    let baseline = ws_frame(&mut ws, |_| true);
    let baseline_frame = ViewFrame3::decode(&baseline).unwrap();
    assert_eq!(baseline, tcp.frame(baseline_frame.seq));
    handle.hold_pump.store(false, Ordering::Release);
    progress_until(
        || a_metrics.status().verified_tick >= 30 && b_metrics.status().verified_tick >= 30,
        "clients did not verify the neutral baseline",
    );

    // Hold real downstream bytes AFTER handshake. Uplink and client B remain live.
    latch.hold.store(true, Ordering::Release);
    progress_until(
        || {
            latch.check();
            latch.receipt.lock().unwrap().holding
        },
        "proxy did not arm",
    );
    let before = handle.snapshot();
    let before_metrics = a_metrics.status();
    let changed = YardInput {
        buttons: SPAWN_BALL,
        _pad: 0,
        origin: [0, 1000, 2000],
        dir: [0, -447, -894],
    };
    b.set_input(PlayerSlot(1), changed).unwrap();
    let mut target = 0;
    progress_until(
        || {
            latch.check();
            let records = server.records.lock().unwrap();
            if let Some(record) = records.iter().find(|r| {
                r.inputs.tick() > before.verified_tick()
                    && *r.inputs.input(PlayerSlot(1)) == changed
                    && r.raw_flags[1] == 0
            }) {
                target = record.inputs.tick();
                true
            } else {
                false
            }
        },
        "server did not finalize the actually accepted changed input",
    );
    assert!(target > before.verified_tick());
    handle.target_tick.store(target, Ordering::Release);
    progress_until(
        || {
            latch.check();
            let snap = handle.snapshot();
            snap.tick() >= target + 2 && snap.verified_tick() < target
        },
        "client A did not predict the unconfirmed changed tick within its window",
    );
    let release = handle.snapshot();
    assert!(release.tick() - release.verified_tick() <= u64::from(MAX_PREDICTION));
    assert_eq!(
        release.stats().rollbacks,
        before.stats().rollbacks,
        "no confirmation passed the armed socket latch"
    );
    assert!(
        !latch.receipt.lock().unwrap().held.is_empty(),
        "actual binary socket payloads were held"
    );
    println!(
        "actual accepted tick {target}; A held confirmation verified {} / predicted {}; releasing FIFO",
        release.verified_tick(),
        release.tick()
    );
    latch.hold.store(false, Ordering::Release);
    let bytes = ws_frame(&mut ws, |f| {
        f.seq > baseline_frame.seq
            && f.flags & FLAG_ROLLED_BACK != 0
            && f.rollback
                .is_some_and(|(from, to)| from <= target && target <= to)
    });
    let frame = ViewFrame3::decode(&bytes).unwrap();
    assert_eq!(
        bytes,
        tcp.frame(frame.seq),
        "same actual corrected publication over WS and TCP"
    );
    let publication = handle.publication(frame.seq);
    let corrected = publication.poll.snapshot.as_ref().unwrap();
    assert!(
        publication.poll.resync.is_none(),
        "network correction is not a fabricated overrun reset"
    );
    assert_snapshot_fields(&frame, corrected);
    let rollback = corrected
        .last_rollback()
        .expect("actual Session rollback receipt");
    assert_eq!(frame.rollback, Some((rollback.from_tick, rollback.to_tick)));
    assert!(
        rollback.from_tick >= 1
            && rollback.from_tick <= target
            && target <= rollback.to_tick
            && rollback.to_tick <= frame.tick
    );
    assert!(corrected.stats().rollbacks > before.stats().rollbacks);
    assert!(a_metrics.status().resim_ticks > before_metrics.resim_ticks);
    assert_eq!(a_metrics.status().desyncs + b_metrics.status().desyncs, 0);

    let checkpoint = ((frame.tick.max(target) / 30) + 1) * 30;
    progress_until(
        || {
            a_metrics.checksum_at(checkpoint).is_some()
                && b_metrics.checksum_at(checkpoint).is_some()
        },
        "both actual clients did not confirm the common checkpoint",
    );
    progress_until(
        || {
            server
                .records
                .lock()
                .unwrap()
                .last()
                .is_some_and(|r| r.inputs.tick() >= checkpoint)
        },
        "server finalized oracle log did not reach checkpoint",
    );
    let records = server.records.lock().unwrap().clone();
    let mut reference = Simulation::<Yard3D>::with_build_id(scene(), TICK_RATE, SEED, BUILD);
    let mut previous = None;
    let mut corrected_reference = None;
    for record in records.iter().take_while(|r| r.inputs.tick() <= checkpoint) {
        assert_eq!(
            record.inputs.tick(),
            reference.tick() + 1,
            "continuous actual finalized bundles"
        );
        if record.inputs.tick() == frame.tick {
            previous = Some(reference.frame().clone());
        }
        reference.step(&record.inputs);
        if record.inputs.tick() == frame.tick {
            corrected_reference = Some(reference.frame().clone());
        }
    }
    assert_eq!(reference.tick(), checkpoint);
    assert_eq!(
        reference.checksum(),
        a_metrics.checksum_at(checkpoint).unwrap()
    );
    assert_eq!(
        reference.checksum(),
        b_metrics.checksum_at(checkpoint).unwrap()
    );
    let expected_current = corrected_reference.unwrap();
    assert_eq!(
        corrected.predicted().checksum(),
        expected_current.checksum(),
        "captured corrected predicted state matches finalized server input replay at the same tick"
    );
    let mut expected_producer = Yard3dStreamProducer::new(BUILD, PLAYERS);
    // This direct encoding is ONLY a field/property oracle. Recovery flags and
    // range above came exclusively from actual production pump and Session.
    let expected = ViewFrame3::decode(&expected_producer.encode_frame(
        &expected_current,
        previous.as_ref(),
        FrameMeta::default(),
    ))
    .unwrap();
    assert_eq!(frame.entities, expected.entities);
    assert_eq!(
        frame.props, expected.props,
        "every dynamic speed property from same corrected state"
    );
    progress_until(
        || {
            latch.check();
            let r = latch.receipt.lock().unwrap();
            r.released_messages == r.held_messages
        },
        "held FIFO was not fully released",
    );
    let packets = latch.receipt.lock().unwrap();
    assert_eq!(
        packets.held, packets.released,
        "actual downstream binary bytes preserved in FIFO order"
    );
    assert!(packets.uplink_binary > 0 && packets.downstream_binary > 0);
    let target_record = records.iter().find(|r| r.inputs.tick() == target).unwrap();
    assert_eq!(target_record.inputs.input(PlayerSlot(1)), &changed);
    assert_eq!(
        target_record.raw_flags[1], 0,
        "actual input accepted before the server deadline, not retroactively inserted"
    );
    save_evidence(
        "yard3d-network-rollback",
        &json!({
            "transport":"actual WS NetLink via bounded downstream FIFO / no loss or jitter",
            "accepted_tick":target, "server_flags":target_record.raw_flags,
            "before_verified":before.verified_tick(), "before_head":before.tick(),
            "release_verified":release.verified_tick(), "release_head":release.tick(),
            "corrected_snapshot_seq":corrected.seq(), "frame_seq":frame.seq,
            "frame_tick":frame.tick, "verified_tick":frame.verified_tick,
            "rollback_from":rollback.from_tick, "rollback_to":rollback.to_tick,
            "rollbacks_before":before.stats().rollbacks, "rollbacks_after":corrected.stats().rollbacks,
            "resim_ticks_before":before_metrics.resim_ticks, "resim_ticks_after":a_metrics.status().resim_ticks,
            "checkpoint":checkpoint, "full_sim_checksum":format!("{:016x}", reference.checksum()),
            "corrected_checksum":format!("{:016x}", corrected.predicted().checksum()),
            "server_finalized_oracle_ticks":checkpoint,
            "uplink_binary":packets.uplink_binary, "downstream_binary":packets.downstream_binary,
        "native_ws_subprotocol":packets.native_subprotocol,
            "held_non_close_messages":packets.held_messages, "released_non_close_messages":packets.released_messages,
            "held_payloads_hex":packets.held.iter().map(|p| codec::hex_encode(p)).collect::<Vec<_>>(),
            "released_payloads_hex":packets.released.iter().map(|p| codec::hex_encode(p)).collect::<Vec<_>>(),
            "fifo_release_bytes_equal":true, "ws_binary_tcp_hex_equal":true,
            "frame_hex":codec::hex_encode(&bytes),
            "finalized_bundles":records.iter().take_while(|r| r.inputs.tick() <= checkpoint).map(|r| json!({
                "tick":r.inputs.tick(), "flags":r.raw_flags,
                "inputs_hex":(0..PLAYERS).map(|slot| codec::hex_encode(bytemuck::bytes_of(r.inputs.input(PlayerSlot(slot))))).collect::<Vec<_>>(),
                "commands":r.inputs.commands().len(),
            })).collect::<Vec<_>>(),
        }),
    );
    drop(packets);
    server.finish();
}
