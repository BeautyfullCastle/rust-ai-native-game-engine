//! [`RemoteBridge`]: the `Remote` adapter of design section 5.3. It
//! implements [`Bridge`] and [`SimControl`] over ERP, so view code attaches
//! to a sim that runs in another process (or on another machine) exactly as
//! it would to an in-process one.
//!
//! # How
//!
//! A background thread (one tokio current-thread runtime) holds the
//! WebSocket. After `auth` it subscribes to `frames`, `events` and `notes`.
//!
//! - Frame snapshots arrive as binary messages: the full `Frame` bytes,
//!   lz4-compressed ([`crate::wire`]). The thread rebuilds the `Frame` with
//!   the game's registry (`Frame::from_bytes` verifies the checksum, so a
//!   snapshot's checksum is the host's) and publishes an immutable
//!   [`Snapshot`] into a lock-free slot. `predicted_prev` is the frame
//!   received just before when it is the previous tick of the same epoch;
//!   after a seek, an edit or a skipped tick it is `None`, and the view
//!   simply does not interpolate across the jump.
//! - Sim events arrive as `watch.events` and become
//!   `BridgeEvent::Sim { status: Verified }` (a local play session has no
//!   prediction); `watch.notes` become [`Lifecycle`] events.
//! - Controls and debug commands are sent as `sim.*` requests. The calls
//!   return at once; a refusal shows up in [`RemoteBridge::take_errors`]
//!   (and, for debug commands, as `Lifecycle::DebugRejected`).
//!
//! # Bandwidth
//!
//! Every published tick is a full frame (no delta yet), so the cost is
//! frame size x rate. The server caps the rate per subscriber
//! ([`RemoteConfig::max_fps`]) and skips frames for a subscriber whose
//! socket is behind. See `tests/measure.rs` for numbers.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use arc_swap::ArcSwapOption;
use orr_bridge::{
    Bridge, BridgeError, BridgeEvent, BridgeStats, ControlOp, DebugCommand, EventKey, EventStatus, Lifecycle, SimControl, Snapshot,
    SnapshotParts,
};
use orr_ecs::Frame;
use orr_sim::{Game, PlayerSlot, SimCommand, Simulation};
use serde_json::{json, Value as J};

use crate::codec::{hex_decode, hex_encode};
use crate::error::RpcError;
use crate::link::{Incoming, PumpedWs, Request, Transport, TxHandle};
use crate::wire::{debug_error_from_name, debug_to_json, decode_frame_message, timeline_from_json};

/// Settings of a [`RemoteBridge`].
#[derive(Clone, Debug)]
pub struct RemoteConfig {
    /// `ws://host:port` of the ERP server.
    pub url: String,
    /// Token for `auth` (`None` for a dev-mode server).
    pub token: Option<String>,
    /// The player slot this view controls (default 0).
    pub local_slot: PlayerSlot,
    /// Cap of frame snapshots per second (default 60).
    pub max_fps: u32,
    /// How long `connect` waits for the handshake, auth and first state (default 10 s).
    pub connect_timeout: Duration,
    /// What the frame stream carries: `sim` (default: the play session's frames),
    /// `view` (the play frame, else the scene's preview frame) or `proposal:p3`
    /// (see `watch.subscribe`).
    pub source: String,
}

impl RemoteConfig {
    /// Defaults for `url`.
    pub fn new(url: &str) -> Self {
        Self { url: url.to_string(), token: None, local_slot: PlayerSlot(0), max_fps: 60, connect_timeout: Duration::from_secs(10), source: "sim".to_string() }
    }
}

/// What the connection received, for a debug overlay and the measurements.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RemoteMetrics {
    /// Frame snapshots received.
    pub frames: u64,
    /// Bytes of frame messages received (compressed, as on the wire).
    pub frame_bytes: u64,
    /// Size of the newest frame message.
    pub last_frame_bytes: u64,
    /// Size of the newest frame after decompression (`Frame::to_bytes`).
    pub last_frame_raw_bytes: u64,
    /// Microseconds from the server building the newest frame message to the
    /// snapshot being published here. Meaningful when both share a clock (one machine).
    pub last_latency_us: u64,
    /// The largest of those.
    pub max_latency_us: u64,
    /// The sum of those over all frames (divide by `frames` for the mean).
    pub latency_sum_us: u64,
    /// Microseconds spent decoding (lz4 + `Frame::from_bytes`) the newest frame.
    pub last_decode_us: u64,
}

struct Shared {
    snapshot: ArcSwapOption<Snapshot>,
    alive: AtomicBool,
    stop: AtomicBool,
    metrics: Mutex<RemoteMetrics>,
    errors: Mutex<Vec<RpcError>>,
}

/// Ids of the requests the bridge itself sends (the handshake) are small; the
/// ones of [`RemoteBridge::request`] start here, so a response can be told apart.
const FIRE_ID_BASE: u64 = 1 << 32;

/// See the module docs.
pub struct RemoteBridge<G: Game> {
    tx: Arc<dyn TxHandle>,
    next_id: AtomicU64,
    events: Receiver<BridgeEvent<G::Event>>,
    shared: Arc<Shared>,
    tick_rate: u32,
    player_count: u8,
    local: PlayerSlot,
    last_input: Option<G::Input>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pending {
    Auth,
    Subscribe,
    State,
}

struct Info {
    tick_rate: u32,
    player_count: u8,
}

impl<G: Game> RemoteBridge<G> {
    /// Connects over WebSocket, authenticates, subscribes and waits for the first `sim.state`.
    pub fn connect(cfg: RemoteConfig) -> Result<Self, String> {
        let t = PumpedWs::connect(&cfg.url, cfg.connect_timeout).map_err(|e| format!("cannot connect to {}: {e}", cfg.url))?;
        Self::connect_transport(Box::new(t), cfg)
    }

    /// Like [`connect`](Self::connect) on a connection that is already
    /// open, for example an in-process [`LocalTransport`](crate::LocalTransport)
    /// to a host thread of this process: frames then arrive as shared
    /// copies, never serialized. `cfg.url` is ignored. The transport must
    /// offer a [`TxHandle`].
    pub fn connect_transport(t: Box<dyn Transport>, cfg: RemoteConfig) -> Result<Self, String> {
        let tx = t.sender().ok_or_else(|| "this transport cannot send from several threads".to_string())?;
        let shared = Arc::new(Shared {
            snapshot: ArcSwapOption::empty(),
            alive: AtomicBool::new(true),
            stop: AtomicBool::new(false),
            metrics: Mutex::new(RemoteMetrics::default()),
            errors: Mutex::new(Vec::new()),
        });
        let (event_tx, events) = channel::<BridgeEvent<G::Event>>();
        let (ready_tx, ready_rx) = channel::<Result<Info, String>>();
        let thread_shared = shared.clone();
        let thread_cfg = cfg.clone();
        let thread = std::thread::Builder::new()
            .name("orr-remote".to_string())
            .spawn(move || {
                run::<G>(t, thread_cfg, &thread_shared, &event_tx, ready_tx);
                thread_shared.alive.store(false, Ordering::Release);
                let _ = event_tx.send(BridgeEvent::Lifecycle(Lifecycle::Disconnected));
            })
            .map_err(|e| format!("start thread: {e}"))?;
        let info = match ready_rx.recv_timeout(cfg.connect_timeout) {
            Ok(Ok(info)) => info,
            Ok(Err(e)) => {
                let _ = thread.join();
                return Err(e);
            }
            Err(_) => {
                shared.stop.store(true, Ordering::Release);
                return Err("timed out connecting to the ERP server".to_string());
            }
        };
        Ok(Self {
            tx,
            next_id: AtomicU64::new(FIRE_ID_BASE),
            events,
            shared,
            tick_rate: info.tick_rate,
            player_count: info.player_count,
            local: cfg.local_slot,
            last_input: None,
            thread: Some(thread),
        })
    }

    /// Sends any ERP request without waiting for the answer (for example
    /// `sim.start`). A refusal shows up in [`take_errors`](Self::take_errors).
    pub fn request(&self, method: &str, params: J) -> Result<(), BridgeError> {
        if !self.shared.alive.load(Ordering::Acquire) {
            return Err(BridgeError::Disconnected);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.tx.send(Request { id: Some(id), method: method.to_string(), params }).map_err(|_| BridgeError::Disconnected)
    }

    /// Errors the server answered to controls, commands and requests since the last call.
    pub fn take_errors(&self) -> Vec<RpcError> {
        std::mem::take(&mut *self.shared.errors.lock().unwrap_or_else(|p| p.into_inner()))
    }

    /// Bytes, counts and latencies of the frame stream.
    pub fn metrics(&self) -> RemoteMetrics {
        *self.shared.metrics.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl<G: Game> Drop for RemoteBridge<G> {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl<G: Game> Bridge<G> for RemoteBridge<G> {
    fn tick_rate(&self) -> u32 {
        self.tick_rate
    }
    fn local_slot(&self) -> PlayerSlot {
        self.local
    }
    fn player_count(&self) -> u8 {
        self.player_count
    }

    fn set_input(&mut self, player: PlayerSlot, input: G::Input) -> Result<(), BridgeError> {
        if player != self.local {
            return Err(BridgeError::NotLocalPlayer { player, local: self.local });
        }
        // Inputs are held until changed: send only changes.
        if self.last_input.as_ref() == Some(&input) {
            return Ok(());
        }
        self.request("sim.input", json!({"player": player.0, "input": hex_encode(bytemuck::bytes_of(&input))}))?;
        self.last_input = Some(input);
        Ok(())
    }

    fn send_command(&mut self, command: G::Command) -> Result<(), BridgeError> {
        let mut bytes = Vec::new();
        SimCommand::encode(&command, &mut bytes);
        self.request("sim.command", json!({"player": self.local.0, "command": hex_encode(&bytes)}))
    }

    fn update(&mut self, _elapsed: Duration) {}

    fn snapshot(&self) -> Option<Snapshot> {
        self.shared.snapshot.load_full().map(|s| (*s).clone())
    }

    fn drain_events(&mut self) -> Vec<BridgeEvent<G::Event>> {
        self.events.try_iter().collect()
    }

    fn is_alive(&self) -> bool {
        self.shared.alive.load(Ordering::Acquire)
    }
}

impl<G: Game> SimControl<G> for RemoteBridge<G> {
    fn control(&mut self, op: ControlOp) -> Result<(), BridgeError> {
        match op {
            ControlOp::Play => self.request("sim.play", J::Null),
            ControlOp::Pause => self.request("sim.pause", J::Null),
            ControlOp::Step(n) => self.request("sim.step", json!({"n": n})),
            ControlOp::SetSpeed(s) => self.request("sim.speed", json!({"permille": s.permille()})),
            ControlOp::Seek(t) => self.request("sim.seek", json!({"tick": t})),
            ControlOp::Branch => self.request("sim.branch", J::Null),
        }
    }

    fn debug_command(&mut self, cmd: DebugCommand) -> Result<(), BridgeError> {
        self.request("sim.debug", debug_to_json(&cmd))
    }
}

// ---- the connection thread ----

fn now_us() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_micros() as u64)
}

struct StreamState {
    last: Option<(u64, u64, Arc<Frame>)>,
    seq: u64,
    frames: u64,
}

/// How often the connection thread looks at `stop` while nothing arrives.
const POLL: Duration = Duration::from_millis(50);

fn run<G: Game>(
    mut t: Box<dyn Transport>,
    cfg: RemoteConfig,
    shared: &Shared,
    events: &Sender<BridgeEvent<G::Event>>,
    ready: Sender<Result<Info, String>>,
) {
    let registry = Simulation::<G>::build_registry();
    let mut pending: BTreeMap<u64, Pending> = BTreeMap::new();
    let mut ready = Some(ready);
    let mut state = StreamState { last: None, seq: 0, frames: 0 };
    let mut info: Option<Info> = None;

    // Handshake requests, in order (the server answers in order).
    let mut first: Vec<(Pending, &str, J)> = Vec::new();
    if let Some(tok) = &cfg.token {
        first.push((Pending::Auth, "auth", json!({"token": tok})));
    }
    first.push((
        Pending::Subscribe,
        "watch.subscribe",
        json!({"topics": ["frames", "events", "notes"], "max_fps": cfg.max_fps.max(1), "source": cfg.source}),
    ));
    first.push((Pending::State, "sim.state", J::Null));
    for (i, (kind, method, params)) in first.into_iter().enumerate() {
        let id = i as u64 + 1;
        pending.insert(id, kind);
        if t.send(Request { id: Some(id), method: method.to_string(), params }).is_err() {
            if let Some(r) = ready.take() {
                let _ = r.send(Err("connection closed during the handshake".into()));
            }
            return;
        }
    }

    while !shared.stop.load(Ordering::Acquire) {
        let msg = match t.recv(POLL) {
            Ok(Some(m)) => m,
            Ok(None) => continue,
            Err(e) => {
                if let Some(r) = ready.take() {
                    let _ = r.send(Err(format!("{e}")));
                }
                return;
            }
        };
        match msg {
            Incoming::Text(text) => {
                let Ok(j) = serde_json::from_str::<J>(&text) else { continue };
                if let Some(id) = j.get("id").and_then(J::as_u64) {
                    let error = j.get("error").map(RpcError::from_json);
                    match (pending.remove(&id), error) {
                        (Some(Pending::Auth | Pending::Subscribe | Pending::State), Some(e)) => {
                            if let Some(r) = ready.take() {
                                let _ = r.send(Err(format!("{e}")));
                            }
                            return;
                        }
                        (Some(Pending::State), None) => {
                            let res = j.get("result").cloned().unwrap_or(J::Null);
                            let tick_rate = res.get("tick_rate").and_then(J::as_u64).unwrap_or(60) as u32;
                            let players = res.get("player_count").and_then(J::as_u64).unwrap_or(1) as u8;
                            info = Some(Info { tick_rate, player_count: players });
                            if let Some(r) = ready.take() {
                                let _ = events.send(BridgeEvent::Lifecycle(Lifecycle::SessionStarted {
                                    tick_rate,
                                    local_slot: cfg.local_slot,
                                    player_count: players,
                                }));
                                let _ = r.send(Ok(Info { tick_rate, player_count: players }));
                            }
                        }
                        (None, Some(e)) => shared.errors.lock().unwrap_or_else(|p| p.into_inner()).push(e),
                        _ => {}
                    }
                } else if let Some(method) = j.get("method").and_then(J::as_str) {
                    let params = j.get("params").cloned().unwrap_or(J::Null);
                    on_notification::<G>(method, &params, events);
                }
            }
            Incoming::Wire(b) => {
                let tick_rate = info.as_ref().map_or(60, |i| i.tick_rate);
                let started = std::time::Instant::now();
                let Ok((meta, bytes)) = decode_frame_message(&b) else { continue };
                let Ok(frame) = Frame::from_bytes(registry.clone(), &bytes) else { continue };
                let sizes = FrameSizes { message: b.len() as u64, raw: bytes.len() as u64, decode_us: started.elapsed().as_micros() as u64 };
                on_frame(&meta, Arc::new(frame), tick_rate, shared, &mut state, sizes);
            }
            Incoming::Local(lf) => {
                let tick_rate = info.as_ref().map_or(60, |i| i.tick_rate);
                on_frame(&lf.meta, lf.frame.clone(), tick_rate, shared, &mut state, FrameSizes::default());
            }
        }
    }
}

fn on_notification<G: Game>(method: &str, params: &J, events: &Sender<BridgeEvent<G::Event>>) {
    match method {
        "watch.events" => {
            let Some(list) = params.get("events").and_then(J::as_array) else { return };
            for e in list {
                let (Some(tick), Some(system), Some(seq), Some(payload)) = (
                    e.get("tick").and_then(J::as_u64),
                    e.get("system").and_then(J::as_u64),
                    e.get("seq").and_then(J::as_u64),
                    e.get("payload").and_then(J::as_str).and_then(hex_decode),
                ) else {
                    continue;
                };
                let Ok(payload) = bytemuck::try_pod_read_unaligned::<G::Event>(&payload) else { continue };
                let key = EventKey::new(tick, system as u16, seq as u32);
                let _ = events.send(BridgeEvent::Sim { key, status: EventStatus::Verified(payload) });
            }
        }
        "watch.notes" => {
            let Some(list) = params.get("notes").and_then(J::as_array) else { return };
            for n in list {
                let u = |k: &str| n.get(k).and_then(J::as_u64).unwrap_or(0);
                let life = match n.get("kind").and_then(J::as_str) {
                    Some("seeked") => Lifecycle::Seeked { from: u("from"), to: u("to") },
                    Some("branched") => Lifecycle::Branched { tick: u("tick"), dropped: u("dropped") },
                    Some("paused") => Lifecycle::Paused { tick: u("tick") },
                    Some("resumed") => Lifecycle::Resumed { tick: u("tick") },
                    Some("debug_rejected") => {
                        Lifecycle::DebugRejected(debug_error_from_name(n.get("error").and_then(J::as_str).unwrap_or("")))
                    }
                    Some("seek_rejected") => Lifecycle::SeekRejected { target: u("target") },
                    _ => continue,
                };
                let _ = events.send(BridgeEvent::Lifecycle(life));
            }
        }
        _ => {}
    }
}

/// Sizes and time of one received frame, for [`RemoteMetrics`] (zero for an in-process frame).
#[derive(Default)]
struct FrameSizes {
    message: u64,
    raw: u64,
    decode_us: u64,
}

fn on_frame(meta: &J, frame: Arc<Frame>, tick_rate: u32, shared: &Shared, st: &mut StreamState, sizes: FrameSizes) {
    let tick = meta.get("tick").and_then(J::as_u64).unwrap_or_else(|| frame.tick());
    let epoch = meta.get("epoch").and_then(J::as_u64).unwrap_or(0);
    let timeline = meta.get("timeline").and_then(timeline_from_json);
    let prev = match &st.last {
        Some((t, e, f)) if *e == epoch && t + 1 == tick => Some(f.clone()),
        _ => None,
    };
    st.seq += 1;
    st.frames += 1;
    let snap = Snapshot::from_parts(SnapshotParts {
        seq: st.seq,
        tick,
        verified_tick: timeline.as_ref().map_or(tick, |t| t.verified_tick),
        tick_rate: meta.get("tick_rate").and_then(J::as_u64).filter(|r| *r > 0).map_or(tick_rate, |r| r as u32),
        predicted: frame.clone(),
        predicted_prev: prev,
        verified: Some(frame.clone()),
        stats: BridgeStats { ticks: st.frames, ..BridgeStats::default() },
        last_rollback: None,
        timeline,
    });
    st.last = Some((tick, epoch, frame));
    shared.snapshot.store(Some(Arc::new(snap)));
    let sent = meta.get("sent_at_us").and_then(J::as_u64).unwrap_or(0);
    let latency = now_us().saturating_sub(sent);
    let mut m = shared.metrics.lock().unwrap_or_else(|p| p.into_inner());
    m.frames += 1;
    m.frame_bytes += sizes.message;
    m.last_frame_bytes = sizes.message;
    m.last_frame_raw_bytes = sizes.raw;
    m.last_latency_us = latency;
    m.max_latency_us = m.max_latency_us.max(latency);
    m.latency_sum_us += latency;
    m.last_decode_us = sizes.decode_us;
}
