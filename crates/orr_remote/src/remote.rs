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
//! - Inputs, controls and debug commands are sent as `sim.*` requests. `Ok`
//!   means the request was queued to the transport, not accepted by the host;
//!   an asynchronous refusal shows up in [`RemoteBridge::take_errors`] (and,
//!   for debug commands, as `Lifecycle::DebugRejected`). The held-input
//!   dedupe cache is invalidated after RPC errors, local play-session
//!   changes and received Branch notes.
//!
//! # Bandwidth
//!
//! Every published tick is a full frame (no delta yet), so the cost is
//! frame size x rate. The server caps the rate per subscriber
//! ([`RemoteConfig::max_fps`]) and skips frames for a subscriber whose
//! socket is behind. See `tests/measure.rs` for numbers.

//!
//! # Presentation recovery
//!
//! New hosts explicitly acknowledge `view_delivery: 1`. Negotiated views use a
//! bounded presentation mailbox and expose only events covered by their pinned
//! snapshot. Overflow replaces missing transient effects with a newest-state
//! reset; the simulation keeps running. Old-host fallback is explicitly visible
//! through [`RemoteBridge::view_delivery`] and retains legacy best-effort,
//! unbounded notification behavior. Transport ingress and RPC/error queues are
//! separate and are not bounded by this presentation budget.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use arc_swap::ArcSwapOption;
use orr_bridge::{
    Bridge, BridgeError, BridgeEvent, BridgeStats, ControlOp, DebugCommand, EventKey, EventStatus,
    Lifecycle, SimControl, Snapshot, SnapshotParts, ViewUpdate,
};
use orr_ecs::Frame;
use orr_sim::{Game, PlayerSlot, SimCommand, Simulation};
use serde_json::{json, Value as J};

use crate::codec::{hex_decode, hex_encode};
use crate::error::RpcError;
use crate::link::{Incoming, PumpedWs, Request, Transport, TxHandle};
use crate::remote_view::{note, Stamp, ViewMailbox};
use crate::wire::{debug_error_from_name, debug_to_json, decode_frame_message, timeline_from_json};

/// Whether a remote view requires the coherent ERP presentation extension.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewDeliveryMode {
    /// Reject an old host that does not explicitly acknowledge support.
    RequireFenced,
    /// Negotiate when supported; expose an explicit legacy fallback otherwise.
    #[default]
    PreferFenced,
    /// Do not negotiate. This retains the old best-effort, unbounded behavior.
    Legacy,
}

/// The delivery contract actually acknowledged by the connected host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteViewDelivery {
    Fenced,
    Legacy,
}

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
    /// Negotiation policy (default: prefer the fenced extension).
    pub view_delivery: ViewDeliveryMode,
    /// Retained presentation notifications across staging and render queues.
    /// Minimum one; excludes transport ingress, RPC replies and snapshots.
    pub view_event_capacity: usize,
}

impl RemoteConfig {
    /// Defaults for `url`.
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_string(),
            token: None,
            local_slot: PlayerSlot(0),
            max_fps: 60,
            connect_timeout: Duration::from_secs(10),
            source: "sim".to_string(),
            view_delivery: ViewDeliveryMode::default(),
            view_event_capacity: orr_bridge::DEFAULT_VIEW_EVENT_CAPACITY,
        }
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

struct Shared<E> {
    view: Mutex<ViewMailbox<E>>,
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
    shared: Arc<Shared<G::Event>>,
    delivery: RemoteViewDelivery,
    tick_rate: u32,
    player_count: u8,
    local: PlayerSlot,
    last_input: Arc<Mutex<Option<G::Input>>>,
    thread: Option<JoinHandle<()>>,
}

fn clear_last_input<I>(last_input: &Mutex<Option<I>>) {
    *last_input.lock().unwrap_or_else(|p| p.into_inner()) = None;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pending {
    Auth,
    Subscribe,
    State,
}

struct Info {
    delivery: RemoteViewDelivery,
    tick_rate: u32,
    player_count: u8,
}

impl<G: Game> RemoteBridge<G> {
    /// Connects over WebSocket, authenticates, subscribes and waits for the first `sim.state`.
    pub fn connect(cfg: RemoteConfig) -> Result<Self, String> {
        let t = PumpedWs::connect(&cfg.url, cfg.connect_timeout)
            .map_err(|e| format!("cannot connect to {}: {e}", cfg.url))?;
        Self::connect_transport(Box::new(t), cfg)
    }

    /// Like [`connect`](Self::connect) on a connection that is already
    /// open, for example an in-process [`LocalTransport`](crate::LocalTransport)
    /// to a host thread of this process: frames then arrive as shared
    /// copies, never serialized. `cfg.url` is ignored. The transport must
    /// offer a [`TxHandle`].
    pub fn connect_transport(t: Box<dyn Transport>, cfg: RemoteConfig) -> Result<Self, String> {
        let tx = t
            .sender()
            .ok_or_else(|| "this transport cannot send from several threads".to_string())?;
        let shared = Arc::new(Shared {
            view: Mutex::new(ViewMailbox::new(cfg.view_event_capacity)),
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
        let last_input = Arc::new(Mutex::new(None));
        let thread_last_input = last_input.clone();
        let thread = std::thread::Builder::new()
            .name("orr-remote".to_string())
            .spawn(move || {
                run::<G>(t, thread_cfg, &thread_shared, &event_tx, ready_tx, thread_last_input);
                thread_shared.alive.store(false, Ordering::Release);
                let mut view = thread_shared.view.lock().unwrap_or_else(|p| p.into_inner());
                if view.negotiated() {
                    view.disconnect();
                } else {
                    let _ = event_tx.send(BridgeEvent::Lifecycle(Lifecycle::Disconnected));
                }
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
            delivery: info.delivery,
            tick_rate: info.tick_rate,
            player_count: info.player_count,
            local: cfg.local_slot,
            last_input,
            thread: Some(thread),
        })
    }

    /// The acknowledged delivery guarantee. A successful subscription alone
    /// does not establish fenced delivery on old hosts.
    pub fn view_delivery(&self) -> RemoteViewDelivery {
        self.delivery
    }

    /// Sends any ERP request without waiting for the answer (for example
    /// `sim.start`). `Ok` means it was queued to the transport; a host refusal
    /// shows up asynchronously in [`take_errors`](Self::take_errors). A
    /// `sim.start`, `sim.stop` or `sim.branch` request also invalidates the
    /// held-input dedupe cache.
    pub fn request(&self, method: &str, params: J) -> Result<(), BridgeError> {
        if matches!(method, "sim.start" | "sim.stop" | "sim.branch") {
            clear_last_input(&self.last_input);
        }
        if !self.shared.alive.load(Ordering::Acquire) {
            return Err(BridgeError::Disconnected);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.tx
            .send(Request {
                id: Some(id),
                method: method.to_string(),
                params,
            })
            .map_err(|_| BridgeError::Disconnected)
    }

    /// Errors the server answered to controls, commands and requests since the last call.
    pub fn take_errors(&self) -> Vec<RpcError> {
        std::mem::take(&mut *self.shared.errors.lock().unwrap_or_else(|p| p.into_inner()))
    }

    /// Bytes, counts and latencies of the frame stream.
    pub fn metrics(&self) -> RemoteMetrics {
        *self
            .shared
            .metrics
            .lock()
            .unwrap_or_else(|p| p.into_inner())
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
            return Err(BridgeError::NotLocalPlayer {
                player,
                local: self.local,
            });
        }
        if !self.shared.alive.load(Ordering::Acquire) {
            return Err(BridgeError::Disconnected);
        }
        // Inputs are held until changed: send only changes. This cache is
        // optimistic because RPC replies are asynchronous, so the receiver
        // clears it on any RPC error or a received Branch note.
        {
            let mut last_input = self.last_input.lock().unwrap_or_else(|p| p.into_inner());
            if last_input.as_ref() == Some(&input) {
                return Ok(());
            }
            *last_input = Some(input);
        }
        if let Err(e) = self.request(
            "sim.input",
            json!({"player": player.0, "input": hex_encode(bytemuck::bytes_of(&input))}),
        ) {
            clear_last_input(&self.last_input);
            return Err(e);
        }
        Ok(())
    }

    fn send_command(&mut self, command: G::Command) -> Result<(), BridgeError> {
        let mut bytes = Vec::new();
        SimCommand::encode(&command, &mut bytes);
        self.request(
            "sim.command",
            json!({"player": self.local.0, "command": hex_encode(&bytes)}),
        )
    }

    fn update(&mut self, _elapsed: Duration) {}

    fn snapshot(&self) -> Option<Snapshot> {
        self.shared.snapshot.load_full().map(|s| (*s).clone())
    }

    fn drain_events(&mut self) -> Vec<BridgeEvent<G::Event>> {
        let mut update = self.poll_view();
        if let Some(reset) = update.resync {
            update.events.insert(0, BridgeEvent::ViewResynced(reset));
        }
        update.events
    }

    fn poll_view(&mut self) -> ViewUpdate<G::Event> {
        if self.delivery == RemoteViewDelivery::Fenced {
            self.shared
                .view
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .poll()
        } else {
            ViewUpdate {
                snapshot: self.snapshot(),
                events: self.events.try_iter().collect(),
                resync: None,
            }
        }
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
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_micros() as u64)
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
    shared: &Shared<G::Event>,
    events: &Sender<BridgeEvent<G::Event>>,
    ready: Sender<Result<Info, String>>,
    last_input: Arc<Mutex<Option<G::Input>>>,
) {
    let registry = Simulation::<G>::build_registry();
    let mut pending: BTreeMap<u64, Pending> = BTreeMap::new();
    let mut ready = Some(ready);
    let mut state = StreamState {
        last: None,
        seq: 0,
        frames: 0,
    };
    let mut info: Option<Info> = None;
    let mut delivery = None;

    // Handshake requests, in order (the server answers in order).
    let mut first: Vec<(Pending, &str, J)> = Vec::new();
    if let Some(tok) = &cfg.token {
        first.push((Pending::Auth, "auth", json!({"token": tok})));
    }
    let mut subscribe = json!({"topics": ["frames", "events", "notes"], "max_fps": cfg.max_fps.max(1), "source": cfg.source});
    if cfg.view_delivery != ViewDeliveryMode::Legacy {
        subscribe["view_delivery"] = json!(1);
    }
    first.push((Pending::Subscribe, "watch.subscribe", subscribe));
    first.push((Pending::State, "sim.state", J::Null));
    for (i, (kind, method, params)) in first.into_iter().enumerate() {
        let id = i as u64 + 1;
        pending.insert(id, kind);
        if t.send(Request {
            id: Some(id),
            method: method.to_string(),
            params,
        })
        .is_err()
        {
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
                let Ok(j) = serde_json::from_str::<J>(&text) else {
                    if delivery == Some(RemoteViewDelivery::Fenced) {
                        fail(shared, &mut ready, "malformed negotiated JSON".into());
                        return;
                    }
                    continue;
                };
                if let Some(id) = j.get("id").and_then(J::as_u64) {
                    let error = j.get("error").map(RpcError::from_json);
                    match (pending.remove(&id), error) {
                        (Some(Pending::Auth | Pending::Subscribe | Pending::State), Some(e)) => {
                            if let Some(r) = ready.take() {
                                let _ = r.send(Err(format!("{e}")));
                            }
                            return;
                        }
                        (Some(Pending::Subscribe), None) => {
                            let result = j.get("result").unwrap_or(&J::Null);
                            let acknowledged =
                                result.get("view_delivery").and_then(J::as_u64) == Some(1);
                            if acknowledged && cfg.view_delivery != ViewDeliveryMode::Legacy {
                                let negotiated = shared
                                    .view
                                    .lock()
                                    .unwrap_or_else(|p| p.into_inner())
                                    .negotiate(result);
                                if let Err(e) = negotiated {
                                    fail(shared, &mut ready, e);
                                    return;
                                }
                                delivery = Some(RemoteViewDelivery::Fenced);
                            } else if cfg.view_delivery == ViewDeliveryMode::RequireFenced {
                                fail(
                                    shared,
                                    &mut ready,
                                    "the host did not acknowledge fenced view delivery v1".into(),
                                );
                                return;
                            } else {
                                delivery = Some(RemoteViewDelivery::Legacy);
                            }
                        }
                        (Some(Pending::State), None) => {
                            let Some(delivery) = delivery else {
                                fail(
                                    shared,
                                    &mut ready,
                                    "state arrived before the view subscription acknowledgement"
                                        .into(),
                                );
                                return;
                            };
                            let res = j.get("result").cloned().unwrap_or(J::Null);
                            let tick_rate =
                                res.get("tick_rate").and_then(J::as_u64).unwrap_or(60) as u32;
                            let players =
                                res.get("player_count").and_then(J::as_u64).unwrap_or(1) as u8;
                            info = Some(Info {
                                tick_rate,
                                player_count: players,
                                delivery,
                            });
                            if let Some(r) = ready.take() {
                                let started = Lifecycle::SessionStarted {
                                    tick_rate,
                                    local_slot: cfg.local_slot,
                                    player_count: players,
                                };
                                if delivery == RemoteViewDelivery::Fenced {
                                    shared
                                        .view
                                        .lock()
                                        .unwrap_or_else(|p| p.into_inner())
                                        .started(started);
                                } else {
                                    let _ = events.send(BridgeEvent::Lifecycle(started));
                                }
                                let _ = r.send(Ok(Info {
                                    tick_rate,
                                    player_count: players,
                                    delivery,
                                }));
                            }
                        }
                        (None, Some(e)) => {
                            // The bridge cannot synchronously know whether an
                            // input request was accepted; conservatively
                            // forget its optimistic dedupe value on refusal.
                            clear_last_input(&last_input);
                            shared.errors.lock().unwrap_or_else(|p| p.into_inner()).push(e);
                        }
                        _ => {}
                    }
                } else if let Some(method) = j.get("method").and_then(J::as_str) {
                    let params = j.get("params").cloned().unwrap_or(J::Null);
                    if has_branch_note(method, &params) {
                        clear_last_input(&last_input);
                    }
                    if delivery == Some(RemoteViewDelivery::Fenced) {
                        if let Err(e) = on_fenced_notification::<G>(method, &params, shared) {
                            fail(shared, &mut ready, e);
                            return;
                        }
                    } else {
                        on_notification::<G>(method, &params, events);
                    }
                }
            }
            Incoming::Wire(b) => {
                let tick_rate = info.as_ref().map_or(60, |i| i.tick_rate);
                let started = std::time::Instant::now();
                let Ok((meta, bytes)) = decode_frame_message(&b) else {
                    if delivery == Some(RemoteViewDelivery::Fenced) {
                        fail(
                            shared,
                            &mut ready,
                            "malformed negotiated frame message".into(),
                        );
                        return;
                    }
                    continue;
                };
                let Ok(frame) = Frame::from_bytes(registry.clone(), &bytes) else {
                    if delivery == Some(RemoteViewDelivery::Fenced) {
                        fail(
                            shared,
                            &mut ready,
                            "invalid negotiated frame payload".into(),
                        );
                        return;
                    }
                    continue;
                };
                let sizes = FrameSizes {
                    message: b.len() as u64,
                    raw: bytes.len() as u64,
                    decode_us: started.elapsed().as_micros() as u64,
                };
                if let Err(e) =
                    on_frame(&meta, Arc::new(frame), tick_rate, shared, &mut state, sizes)
                {
                    fail(shared, &mut ready, e);
                    return;
                }
            }
            Incoming::Local(lf) => {
                let tick_rate = info.as_ref().map_or(60, |i| i.tick_rate);
                if let Err(e) = on_frame(
                    &lf.meta,
                    lf.frame.clone(),
                    tick_rate,
                    shared,
                    &mut state,
                    FrameSizes::default(),
                ) {
                    fail(shared, &mut ready, e);
                    return;
                }
            }
        }
    }
}

fn has_branch_note(method: &str, params: &J) -> bool {
    method == "watch.notes"
        && params.get("notes").and_then(J::as_array).is_some_and(|notes| {
            notes.iter().any(|n| n.get("kind").and_then(J::as_str) == Some("branched"))
        })
}

fn fail<E>(shared: &Shared<E>, ready: &mut Option<Sender<Result<Info, String>>>, message: String) {
    if let Some(ready) = ready.take() {
        let _ = ready.send(Err(message.clone()));
    }
    // Make the reason visible before the terminal presentation status. A UI
    // may stop polling immediately after it observes Disconnected.
    shared
        .errors
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .push(RpcError::state("view_delivery_invalid", message));
    shared
        .view
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .fail_closed();
}

fn on_fenced_notification<G: Game>(
    method: &str,
    params: &J,
    shared: &Shared<G::Event>,
) -> Result<(), String> {
    if !matches!(
        method,
        "watch.events" | "watch.notes" | "watch.view.inactive"
    ) {
        return Ok(());
    }
    let delivery = params
        .get("delivery")
        .ok_or("missing negotiated notification metadata")?;
    if method == "watch.view.inactive" {
        shared
            .view
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .frame(delivery, None)?;
        shared.snapshot.store(None);
        return Ok(());
    }
    let stamp = Stamp::parse(delivery, false)?;
    let mut events = Vec::new();
    if method == "watch.events" {
        for e in params
            .get("events")
            .and_then(J::as_array)
            .ok_or("missing negotiated events")?
        {
            let u = |key| {
                e.get(key)
                    .and_then(J::as_u64)
                    .ok_or("invalid negotiated event key")
            };
            let tick = u("tick")?;
            let system = u16::try_from(u("system")?).map_err(|_| "event system out of range")?;
            let seq = u32::try_from(u("seq")?).map_err(|_| "event sequence out of range")?;
            let bytes = e
                .get("payload")
                .and_then(J::as_str)
                .and_then(hex_decode)
                .ok_or("invalid event payload")?;
            let payload = bytemuck::try_pod_read_unaligned::<G::Event>(&bytes)
                .map_err(|_| "invalid event payload size")?;
            events.push(BridgeEvent::Sim {
                key: EventKey::new(tick, system, seq),
                status: EventStatus::Verified(payload),
            });
        }
    } else {
        for n in params
            .get("notes")
            .and_then(J::as_array)
            .ok_or("missing negotiated notes")?
        {
            events.push(BridgeEvent::Lifecycle(note(n)?));
        }
    }
    shared
        .view
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .stage(stamp, events)
}

fn on_notification<G: Game>(method: &str, params: &J, events: &Sender<BridgeEvent<G::Event>>) {
    match method {
        "watch.events" => {
            let Some(list) = params.get("events").and_then(J::as_array) else {
                return;
            };
            for e in list {
                let (Some(tick), Some(system), Some(seq), Some(payload)) = (
                    e.get("tick").and_then(J::as_u64),
                    e.get("system").and_then(J::as_u64),
                    e.get("seq").and_then(J::as_u64),
                    e.get("payload").and_then(J::as_str).and_then(hex_decode),
                ) else {
                    continue;
                };
                let Ok(payload) = bytemuck::try_pod_read_unaligned::<G::Event>(&payload) else {
                    continue;
                };
                let key = EventKey::new(tick, system as u16, seq as u32);
                let _ = events.send(BridgeEvent::Sim {
                    key,
                    status: EventStatus::Verified(payload),
                });
            }
        }
        "watch.notes" => {
            let Some(list) = params.get("notes").and_then(J::as_array) else {
                return;
            };
            for n in list {
                let u = |k: &str| n.get(k).and_then(J::as_u64).unwrap_or(0);
                let life = match n.get("kind").and_then(J::as_str) {
                    Some("seeked") => Lifecycle::Seeked {
                        from: u("from"),
                        to: u("to"),
                    },
                    Some("branched") => Lifecycle::Branched {
                        tick: u("tick"),
                        dropped: u("dropped"),
                    },
                    Some("paused") => Lifecycle::Paused { tick: u("tick") },
                    Some("resumed") => Lifecycle::Resumed { tick: u("tick") },
                    Some("debug_rejected") => Lifecycle::DebugRejected(debug_error_from_name(
                        n.get("error").and_then(J::as_str).unwrap_or(""),
                    )),
                    Some("seek_rejected") => Lifecycle::SeekRejected {
                        target: u("target"),
                    },
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

fn on_frame<E>(
    meta: &J,
    frame: Arc<Frame>,
    tick_rate: u32,
    shared: &Shared<E>,
    st: &mut StreamState,
    sizes: FrameSizes,
) -> Result<(), String> {
    let mut view = shared.view.lock().unwrap_or_else(|p| p.into_inner());
    let fenced = view.negotiated();
    let delivery = if fenced {
        Some(
            meta.get("delivery")
                .ok_or("missing negotiated frame metadata")?,
        )
    } else {
        None
    };
    let tick = meta
        .get("tick")
        .and_then(J::as_u64)
        .unwrap_or_else(|| frame.tick());
    let epoch = match delivery {
        Some(delivery) => Stamp::parse(delivery, true)?.timeline,
        None => meta.get("epoch").and_then(J::as_u64).unwrap_or(0),
    };
    let timeline = meta.get("timeline").and_then(timeline_from_json);
    if fenced
        && (meta.get("tick").and_then(J::as_u64) != Some(frame.tick())
            || meta.get("timeline").is_none()
            || (meta.get("timeline").is_some_and(|v| !v.is_null()) && timeline.is_none())
            || timeline
                .as_ref()
                .is_some_and(|t| t.tick != tick || t.verified_tick > tick))
    {
        return Err("negotiated frame metadata does not describe its payload".into());
    }
    let prev = match &st.last {
        Some((t, e, f)) if *e == epoch && t.checked_add(1) == Some(tick) => Some(f.clone()),
        _ => None,
    };
    st.seq += 1;
    st.frames += 1;
    let snap = Snapshot::from_parts(SnapshotParts {
        seq: st.seq,
        tick,
        verified_tick: timeline.as_ref().map_or(tick, |t| t.verified_tick),
        tick_rate: meta
            .get("tick_rate")
            .and_then(J::as_u64)
            .filter(|r| *r > 0)
            .map_or(tick_rate, |r| r as u32),
        predicted: frame.clone(),
        predicted_prev: prev,
        verified: Some(frame.clone()),
        stats: BridgeStats {
            ticks: st.frames,
            ..BridgeStats::default()
        },
        last_rollback: None,
        timeline,
    });
    st.last = Some((tick, epoch, frame));
    if let Some(delivery) = delivery {
        view.frame(delivery, Some(snap.clone()))?;
    }
    shared.snapshot.store(Some(Arc::new(snap)));
    drop(view);
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
    Ok(())
}
