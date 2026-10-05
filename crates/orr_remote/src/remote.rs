//! [`RemoteBridge`]: the `Remote` adapter of design section 5.3. It
//! implements [`Bridge`] and [`SimControl`] over ERP, so view code attaches
//! to a sim that runs in another process (or on another machine) exactly as
//! it would to an in-process one.
//!
//! # How
//!
//! A background thread (one tokio current-thread runtime) holds the
//! WebSocket. After `auth`, a checked connection verifies `rpc.discover` and
//! `registry.schema` before subscribing to `frames`, `events` and `notes`.
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
//! Legacy subscribers receive full frames. Callers may opt into bounded
//! frame-codec v1 records with [`FrameCodecPolicy`]; that API does not make a
//! performance or bandwidth claim. The server caps the rate per subscriber
//! ([`RemoteConfig::max_fps`]) and skips frames for a subscriber whose socket
//! is behind. See `tests/measure.rs` for measurements of the legacy stream.

//!
//! # Presentation recovery
//!
//! New hosts explicitly acknowledge `view_delivery: 1`. Negotiated views use a
//! bounded presentation mailbox and expose only events covered by their pinned
//! snapshot. Overflow replaces missing transient effects with a newest-state
//! reset; the simulation keeps running. Old-host fallback is explicitly visible
//! through [`RemoteBridge::view_delivery`] and retains legacy best-effort,
//! unbounded notification behavior. Pumped transport ingress/egress and retained
//! RPC errors have separate item and payload-byte limits. RPC error overflow
//! closes the connection and reports a fixed terminal diagnostic; it does not
//! evict an earlier refusal. These payload-byte counters do not measure heap
//! overhead, decoded temporaries, or snapshots retained by callers.

use std::collections::{BTreeMap, VecDeque};
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
use crate::frame_delta::{
    DecodeError, Decoder, FrameRecord, FrameScope, FrameStamp, NeedFullReason,
};
use crate::link::{
    Incoming, PumpedWs, QueueBudget, QueueLimits, QueuePermit, QueueStats, Request, SendError,
    Transport, TransportQueueLimits, TransportQueueStats, TxHandle,
};
use crate::remote_view::{note, Stamp, ViewMailbox};
use crate::wire::{
    debug_error_from_name, debug_to_json, decode_frame_message, decode_frame_record_message,
    timeline_from_json, FrameCodecLimits, FrameCodecMeta, DEFAULT_FRAME_CODEC_LIMITS,
};

/// Whether negotiated frame deltas are optional or required by this client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameCodecMode {
    /// Use frame deltas when the host advertises and acknowledges the exact limits.
    Prefer,
    /// Refuse a host that does not support or exactly acknowledge frame deltas.
    Require,
}

/// Finite limits and recovery deadline for opt-in remote frame deltas.
///
/// This is passed to additive constructors so existing [`RemoteConfig`]
/// literals and legacy constructors retain their behavior.
#[derive(Clone, Copy, Debug)]
pub struct FrameCodecPolicy {
    pub mode: FrameCodecMode,
    pub limits: FrameCodecLimits,
    pub reset_timeout: Duration,
}

impl FrameCodecPolicy {
    pub fn prefer(limits: FrameCodecLimits) -> Self {
        Self {
            mode: FrameCodecMode::Prefer,
            limits,
            reset_timeout: Duration::from_secs(10),
        }
    }

    pub fn require(limits: FrameCodecLimits) -> Self {
        Self {
            mode: FrameCodecMode::Require,
            limits,
            reset_timeout: Duration::from_secs(10),
        }
    }

    pub fn prefer_default() -> Self {
        Self::prefer(DEFAULT_FRAME_CODEC_LIMITS)
    }

    pub fn require_default() -> Self {
        Self::require(DEFAULT_FRAME_CODEC_LIMITS)
    }

    pub fn with_reset_timeout(mut self, timeout: Duration) -> Self {
        self.reset_timeout = timeout;
        self
    }

    fn validate(&self, source: &str) -> Result<(), String> {
        if source != "sim" {
            return Err("frame codec v1 currently supports only the sim frame source".into());
        }
        self.limits.validate().map_err(|error| error.to_string())?;
        if self.reset_timeout.is_zero() || self.reset_timeout > Duration::from_secs(60) {
            return Err("frame codec limits or reset timeout are invalid".into());
        }
        Ok(())
    }
}

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

/// The ERP identity that a checked [`RemoteBridge`] connection must match.
///
/// Use [`RemoteIdentity::from_discovery`] with an identity and schema that
/// have already been validated against the caller's compiled game registry.
/// Structural equality of the full schema guards the actual attachment.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteIdentity {
    /// Game name reported by `rpc.discover` (`engine.game`).
    pub game: String,
    /// Build id reported by `rpc.discover` (`engine.build_id`).
    pub build_id: String,
    /// Full `registry.schema` schema value expected for this game build.
    pub schema: J,
}

impl RemoteIdentity {
    /// Capture the identity fields from an ERP 1 `rpc.discover` result and pair
    /// them with its separately fetched `registry.schema` value. The caller
    /// should compare `schema` with its compiled descriptors before using it.
    pub fn from_discovery(discovery: &J, schema: J) -> Result<Self, String> {
        if discovery.get("erp_version").and_then(J::as_u64) != Some(1) {
            return Err("rpc.discover does not explicitly report ERP version 1".into());
        }
        let engine = discovery
            .get("engine")
            .ok_or_else(|| "rpc.discover is missing engine identity".to_string())?;
        let field = |name: &str| {
            let value = engine
                .get(name)
                .and_then(J::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("rpc.discover is missing engine.{name}"))?;
            if value.trim().is_empty() {
                return Err(format!("rpc.discover has an empty engine.{name}"));
            }
            Ok(value)
        };
        Ok(Self {
            game: field("game")?,
            build_id: field("build_id")?,
            schema,
        })
    }
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
    /// Optional identity fence. When set, `rpc.discover` and `registry.schema`
    /// must match this identity on the same transport before the frame
    /// subscription is started. `None` preserves the historical handshake.
    pub expected_identity: Option<RemoteIdentity>,
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
            expected_identity: None,
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

const DEFAULT_ERROR_QUEUE_LIMITS: QueueLimits = QueueLimits {
    max_items: 256,
    max_bytes: 1 << 20,
};

struct RetainedError {
    error: RpcError,
    _permit: QueuePermit,
}

#[derive(Default)]
struct ErrorState {
    entries: VecDeque<RetainedError>,
    overflowed: bool,
    overflow_reported: bool,
}

/// Bounded RPC diagnostics. Overflow is terminal because dropping an
/// individual refusal would make the asynchronous request contract ambiguous.
struct ErrorQueue {
    budget: QueueBudget,
    state: Mutex<ErrorState>,
}

impl ErrorQueue {
    fn new(limits: QueueLimits) -> Self {
        Self {
            budget: QueueBudget::new(limits),
            state: Mutex::new(ErrorState::default()),
        }
    }

    fn push(&self, error: RpcError) -> bool {
        let cost = error.to_json().to_string().len();
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.overflowed {
            return false;
        }
        match self.budget.reserve(cost) {
            Ok(permit) => {
                state.entries.push_back(RetainedError {
                    error,
                    _permit: permit,
                });
                true
            }
            Err(SendError::Backpressure | SendError::Disconnected) => {
                state.overflowed = true;
                self.budget
                    .close("remote RPC error queue saturated; connection closed");
                false
            }
        }
    }

    fn take(&self) -> Vec<RpcError> {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let mut errors = state
            .entries
            .drain(..)
            .map(|entry| entry.error)
            .collect::<Vec<_>>();
        if state.overflowed && !state.overflow_reported {
            state.overflow_reported = true;
            errors.push(RpcError::state(
                "remote_error_queue_overflow",
                "RPC error queue reached its limit; the connection was closed and accepted RPC outcomes may be uncertain",
            ));
        }
        errors
    }

    fn stats(&self) -> QueueStats {
        self.budget.stats()
    }
}

struct Shared<E> {
    view: Mutex<ViewMailbox<E>>,
    snapshot: ArcSwapOption<Snapshot>,
    alive: AtomicBool,
    stop: AtomicBool,
    metrics: Mutex<RemoteMetrics>,
    errors: ErrorQueue,
    tx: Arc<dyn TxHandle>,
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
    Discover,
    Schema,
    Subscribe,
    ResetSubscribe,
    State,
}

fn queue_handshake(
    t: &mut dyn Transport,
    pending: &mut BTreeMap<u64, Pending>,
    next_id: &mut u64,
    kind: Pending,
    method: &str,
    params: J,
) -> Result<u64, String> {
    let id = *next_id;
    *next_id = id
        .checked_add(1)
        .ok_or_else(|| "remote handshake request id space exhausted".to_string())?;
    t.send(Request {
        id: Some(id),
        method: method.to_string(),
        params,
    })
    .map_err(|_| "connection closed during the handshake".to_string())?;
    pending.insert(id, kind);
    Ok(id)
}

fn identity_check_error(message: impl core::fmt::Display) -> String {
    format!("remote identity check: {message}")
}

fn validate_discovery(result: &J, expected: &RemoteIdentity) -> Result<(), String> {
    if expected.game.trim().is_empty() || expected.build_id.trim().is_empty() {
        return Err(identity_check_error(
            "the expected identity is missing game or build id",
        ));
    }
    let actual = RemoteIdentity::from_discovery(result, expected.schema.clone())
        .map_err(identity_check_error)?;
    if actual.game != expected.game {
        return Err(identity_check_error(format!(
            "game mismatch: expected {:?}, received {:?}",
            expected.game, actual.game,
        )));
    }
    if actual.build_id != expected.build_id {
        return Err(identity_check_error(format!(
            "build id mismatch: expected {:?}, received {:?}",
            expected.build_id, actual.build_id,
        )));
    }
    Ok(())
}

fn validate_remote_schema(result: &J, expected: &RemoteIdentity) -> Result<(), String> {
    if !expected.schema.is_object() {
        return Err(identity_check_error(
            "the expected game schema is not a JSON object",
        ));
    }
    let actual = result
        .get("schema")
        .ok_or_else(|| identity_check_error("registry.schema is missing the full schema"))?;
    if !actual.is_object() {
        return Err(identity_check_error(
            "registry.schema did not return a full schema object",
        ));
    }
    if actual != &expected.schema {
        return Err(identity_check_error(
            "registry.schema does not match the expected game schema",
        ));
    }
    Ok(())
}

fn begin_subscription(
    t: &mut dyn Transport,
    pending: &mut BTreeMap<u64, Pending>,
    next_id: &mut u64,
    cfg: &RemoteConfig,
    frame_codec: Option<FrameCodecLimits>,
) -> Result<(), String> {
    queue_handshake(
        t,
        pending,
        next_id,
        Pending::Subscribe,
        "watch.subscribe",
        subscription_params(cfg, frame_codec),
    )?;
    queue_handshake(t, pending, next_id, Pending::State, "sim.state", J::Null)?;
    Ok(())
}

fn subscription_params(cfg: &RemoteConfig, frame_codec: Option<FrameCodecLimits>) -> J {
    let mut subscribe = json!({
        "topics": ["frames", "events", "notes"],
        "max_fps": cfg.max_fps.max(1),
        "source": cfg.source,
    });
    if cfg.view_delivery != ViewDeliveryMode::Legacy || frame_codec.is_some() {
        subscribe["view_delivery"] = json!(1);
    }
    if let Some(limits) = frame_codec {
        subscribe["frame_codec"] = json!({
            "version": 1,
            "max_frame_bytes": limits.max_frame_bytes,
            "max_baseline_bytes": limits.max_baseline_bytes,
            "max_message_bytes": limits.max_message_bytes,
        });
    }
    subscribe
}

fn supports_frame_codec(discovery: &J) -> bool {
    discovery
        .get("features")
        .and_then(|features| features.get("frame_codec"))
        .and_then(J::as_array)
        .is_some_and(|versions| versions.iter().any(|v| v.as_u64() == Some(1)))
}

fn codec_exact(j: &J, key: &str) -> Result<u64, String> {
    let text = j
        .get(key)
        .and_then(J::as_str)
        .ok_or_else(|| format!("missing frame codec {key}"))?;
    let value = text
        .parse::<u64>()
        .map_err(|_| format!("invalid frame codec {key}"))?;
    if value.to_string() != text {
        return Err(format!("frame codec {key} is not canonical decimal"));
    }
    Ok(value)
}

fn queue_frame_codec_recovery<E>(
    t: &mut dyn Transport,
    pending: &mut BTreeMap<u64, Pending>,
    next_id: &mut u64,
    cfg: &RemoteConfig,
    codec: &mut FrameCodecClient,
    shared: &Shared<E>,
) -> Result<(), String> {
    if codec.recovery_request.is_some() {
        return Err("a frame codec reset request is already pending".into());
    }
    let deadline = std::time::Instant::now()
        .checked_add(codec.reset_timeout)
        .ok_or_else(|| "frame codec reset deadline is out of range".to_string())?;
    shared
        .view
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .begin_renewal();
    let id = queue_handshake(
        t,
        pending,
        next_id,
        Pending::ResetSubscribe,
        "watch.subscribe",
        subscription_params(cfg, Some(codec.limits)),
    )?;
    codec.recovery_request = Some(id);
    codec.recovery_deadline = Some(deadline);
    Ok(())
}

struct Info {
    delivery: RemoteViewDelivery,
    tick_rate: u32,
    player_count: u8,
}

struct CodecResetNotice {
    generation: u64,
    next_sequence: u64,
    scope: FrameScope,
    delivery: Stamp,
}

struct PreparedCodecRenewal {
    subscription: u64,
    deadline: std::time::Instant,
}

struct FrameCodecClient {
    limits: FrameCodecLimits,
    reset_timeout: Duration,
    decoder: Decoder,
    subscription: u64,
    sequence: u64,
    reset_generation: u64,
    scope: Option<FrameScope>,
    last_target: Option<FrameStamp>,
    awaiting_full: bool,
    source_inactive: bool,
    reset_notice: Option<CodecResetNotice>,
    recovery_request: Option<u64>,
    recovery_deadline: Option<std::time::Instant>,
}

impl FrameCodecClient {
    fn new(
        registry: Arc<orr_ecs::ComponentRegistry>,
        limits: FrameCodecLimits,
        reset_timeout: Duration,
        result: &J,
    ) -> Result<Self, String> {
        validate_frame_codec_ack(result, limits)?;
        let deadline = std::time::Instant::now()
            .checked_add(reset_timeout)
            .ok_or_else(|| "frame codec reset deadline is out of range".to_string())?;
        Ok(Self {
            limits,
            reset_timeout,
            decoder: Decoder::new(registry, limits.max_frame_bytes, limits.max_baseline_bytes),
            subscription: codec_exact(result, "subscription")?,
            sequence: 0,
            reset_generation: 1,
            scope: None,
            last_target: None,
            awaiting_full: true,
            source_inactive: false,
            reset_notice: None,
            recovery_request: None,
            recovery_deadline: Some(deadline),
        })
    }

    fn prepare_renewal(&self, result: &J) -> Result<PreparedCodecRenewal, String> {
        validate_frame_codec_ack(result, self.limits)?;
        let subscription = codec_exact(result, "subscription")?;
        if subscription == self.subscription {
            return Err("frame codec reset did not issue a new subscription".into());
        }
        let deadline = std::time::Instant::now()
            .checked_add(self.reset_timeout)
            .ok_or_else(|| "frame codec reset deadline is out of range".to_string())?;
        Ok(PreparedCodecRenewal {
            subscription,
            deadline,
        })
    }

    fn commit_renewal(
        &mut self,
        registry: Arc<orr_ecs::ComponentRegistry>,
        prepared: PreparedCodecRenewal,
    ) {
        self.decoder = Decoder::new(
            registry,
            self.limits.max_frame_bytes,
            self.limits.max_baseline_bytes,
        );
        self.subscription = prepared.subscription;
        self.sequence = 0;
        self.reset_generation = 1;
        self.scope = None;
        self.last_target = None;
        self.awaiting_full = true;
        self.source_inactive = false;
        self.reset_notice = None;
        self.recovery_request = None;
        self.recovery_deadline = Some(prepared.deadline);
    }

    fn accept_reset_notice(&mut self, params: &J) -> Result<(), String> {
        if self.recovery_request.is_some() {
            // The notice belongs to the old subscription while replacement is
            // pending; queued old-subscription traffic is discarded as a unit.
            return Ok(());
        }
        let subscription = codec_exact(params, "subscription")?;
        if subscription != self.subscription || self.awaiting_full || self.reset_notice.is_some() {
            return Err("unexpected frame codec reset announcement".into());
        }
        let generation = codec_exact(params, "reset_generation")?;
        let next_generation = self
            .reset_generation
            .checked_add(1)
            .ok_or_else(|| "frame codec reset generation exhausted".to_string())?;
        let next_sequence = codec_exact(params, "next_sequence")?;
        let expected_sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| "frame codec sequence exhausted".to_string())?;
        if generation != next_generation || next_sequence != expected_sequence {
            return Err("frame codec reset announcement is out of order".into());
        }
        let scope_value = params
            .get("scope")
            .ok_or("frame codec reset announcement is missing scope")?;
        let scope = FrameScope {
            stream_generation: codec_exact(scope_value, "stream_generation")?,
            play_epoch: codec_exact(scope_value, "play_epoch")?,
            timeline_epoch: codec_exact(scope_value, "timeline_epoch")?,
        };
        if scope.stream_generation != subscription || scope.timeline_epoch != generation {
            return Err("frame codec reset scope does not match its stream identity".into());
        }
        let delivery = params
            .get("delivery")
            .ok_or("frame codec reset announcement is missing delivery cut")?;
        let delivery_stamp = Stamp::parse(delivery, true)?;
        if delivery_stamp.subscription != subscription {
            return Err("frame codec reset delivery cut has a stale subscription".into());
        }
        self.reset_notice = Some(CodecResetNotice {
            generation,
            next_sequence,
            scope,
            delivery: delivery_stamp,
        });
        self.awaiting_full = true;
        self.source_inactive = false;
        self.recovery_deadline = Some(
            std::time::Instant::now()
                .checked_add(self.reset_timeout)
                .ok_or_else(|| "frame codec reset deadline is out of range".to_string())?,
        );
        Ok(())
    }

    fn validate_record_identity(
        &self,
        metadata: &J,
        identity: FrameCodecMeta,
        record: &FrameRecord,
    ) -> Result<(FrameStamp, bool), String> {
        if self.recovery_request.is_some() {
            return Err("frame arrived while a replacement subscription is pending".into());
        }
        if identity.subscription != self.subscription {
            return Err("frame codec record belongs to a stale subscription".into());
        }
        let expected_sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| "frame codec sequence exhausted".to_string())?;
        if identity.sequence != expected_sequence {
            return Err("frame codec record sequence is not the next admitted record".into());
        }
        let target = match record {
            FrameRecord::Full { target, .. } | FrameRecord::Delta { target, .. } => *target,
        };
        let play_epoch = codec_exact(metadata, "play_epoch")?;
        if target.scope.play_epoch != play_epoch {
            return Err("frame codec stamp disagrees with the host play epoch".into());
        }

        if self.source_inactive && !matches!(record, FrameRecord::Full { .. }) {
            return Err("inactive source must resume with a Full".into());
        }
        if self.awaiting_full {
            if !matches!(record, FrameRecord::Full { .. }) {
                return Err("delta arrived before the required Full reset".into());
            }
            if let Some(notice) = &self.reset_notice {
                if identity.sequence != notice.next_sequence
                    || identity.reset_generation != notice.generation
                    || target.scope != notice.scope
                    || Stamp::parse(
                        metadata
                            .get("delivery")
                            .ok_or("missing negotiated frame metadata")?,
                        true,
                    )? != notice.delivery
                {
                    return Err("first Full does not match the ordered reset announcement".into());
                }
            } else if identity.sequence != 1 || identity.reset_generation != 1 {
                return Err("initial Full does not match the subscription acknowledgement".into());
            }
            return Ok((target, true));
        }

        if self.reset_notice.is_some() || identity.reset_generation != self.reset_generation {
            return Err("frame codec reset generation changed without an announcement".into());
        }
        if self.scope != Some(target.scope) {
            return Err("frame codec scope changed without an announced Full reset".into());
        }
        if let FrameRecord::Full { .. } = record {
            // Identity survives a deliberate zero/small retention budget. The
            // next Full remains ordered without retaining any baseline bytes.
            let baseline = self
                .last_target
                .ok_or("frame codec has no admitted identity before an ordinary Full")?;
            if target.tick < baseline.tick
                || (target.tick == baseline.tick
                    && target.frame_checksum != baseline.frame_checksum)
            {
                return Err("stale Full requires an announced reset before admission".into());
            }
        }
        Ok((target, false))
    }

    fn commit_record_identity(&mut self, identity: FrameCodecMeta, target: FrameStamp) {
        self.sequence = identity.sequence;
        self.reset_generation = identity.reset_generation;
        self.scope = Some(target.scope);
        self.last_target = Some(target);
        self.awaiting_full = false;
        self.source_inactive = false;
        self.reset_notice = None;
        self.recovery_deadline = None;
    }
}

fn validate_frame_codec_ack(result: &J, requested: FrameCodecLimits) -> Result<(), String> {
    let accepted = result
        .get("frame_codec")
        .and_then(J::as_object)
        .ok_or("frame codec subscription acknowledgement is missing accepted limits")?;
    if accepted.len() != 4 || accepted.get("version").and_then(J::as_u64) != Some(1) {
        return Err("frame codec acknowledgement has an unexpected version or fields".into());
    }
    for (key, value) in [
        ("max_frame_bytes", requested.max_frame_bytes),
        ("max_baseline_bytes", requested.max_baseline_bytes),
        ("max_message_bytes", requested.max_message_bytes),
    ] {
        if accepted.get(key).and_then(J::as_u64) != Some(value as u64) {
            return Err(format!("frame codec acknowledgement changed {key}"));
        }
    }
    if codec_exact(result, "sequence")? != 0 || codec_exact(result, "reset_generation")? != 1 {
        return Err(
            "frame codec acknowledgement has an invalid initial sequence/generation".into(),
        );
    }
    if codec_exact(result, "subscription")? == 0 {
        return Err("frame codec acknowledgement has an invalid zero subscription".into());
    }
    let _ = codec_exact(result, "cursor")?;
    let _ = codec_exact(result, "count")?;
    Ok(())
}

impl<G: Game> RemoteBridge<G> {
    /// Connects over WebSocket, authenticates, subscribes and waits for the first `sim.state`.
    pub fn connect(cfg: RemoteConfig) -> Result<Self, String> {
        Self::connect_with_queue_limits(
            cfg,
            TransportQueueLimits::default(),
            DEFAULT_ERROR_QUEUE_LIMITS,
        )
    }

    /// Connects with independent retained transport and RPC-error queue limits.
    /// Existing [`RemoteConfig`] fields and the default [`connect`](Self::connect)
    /// behavior remain unchanged.
    pub fn connect_with_queue_limits(
        cfg: RemoteConfig,
        transport_limits: TransportQueueLimits,
        error_limits: QueueLimits,
    ) -> Result<Self, String> {
        let t =
            PumpedWs::connect_with_queue_limits(&cfg.url, cfg.connect_timeout, transport_limits)
                .map_err(|e| format!("cannot connect to {}: {e}", cfg.url))?;
        Self::connect_transport_with_error_limits(Box::new(t), cfg, error_limits)
    }

    /// Connects over WebSocket with the explicitly opted-in v1 Frame codec.
    pub fn connect_with_frame_codec(
        cfg: RemoteConfig,
        policy: FrameCodecPolicy,
    ) -> Result<Self, String> {
        policy.validate(&cfg.source)?;
        let t = PumpedWs::connect_with_queue_limits(
            &cfg.url,
            cfg.connect_timeout,
            TransportQueueLimits::default(),
        )
        .map_err(|e| format!("cannot connect to {}: {e}", cfg.url))?;
        Self::connect_transport_inner(Box::new(t), cfg, DEFAULT_ERROR_QUEUE_LIMITS, Some(policy))
    }

    /// Like [`connect`](Self::connect) on a connection that is already
    /// open, for example an in-process [`LocalTransport`](crate::LocalTransport)
    /// to a host thread of this process: frames then arrive as shared
    /// copies, never serialized. `cfg.url` is ignored. The transport must
    /// offer a [`TxHandle`].
    pub fn connect_transport(t: Box<dyn Transport>, cfg: RemoteConfig) -> Result<Self, String> {
        Self::connect_transport_with_error_limits(t, cfg, DEFAULT_ERROR_QUEUE_LIMITS)
    }

    /// Connects an existing transport with an explicit retained RPC-error
    /// budget. Transport limits belong to the transport and can be configured
    /// when constructing a [`PumpedWs`].
    pub fn connect_transport_with_error_limits(
        t: Box<dyn Transport>,
        cfg: RemoteConfig,
        error_limits: QueueLimits,
    ) -> Result<Self, String> {
        Self::connect_transport_inner(t, cfg, error_limits, None)
    }

    /// Connects an existing serialized transport with explicit frame codec
    /// limits. This is also useful for protocol-failure tests.
    pub fn connect_transport_with_frame_codec(
        t: Box<dyn Transport>,
        cfg: RemoteConfig,
        policy: FrameCodecPolicy,
    ) -> Result<Self, String> {
        policy.validate(&cfg.source)?;
        Self::connect_transport_inner(t, cfg, DEFAULT_ERROR_QUEUE_LIMITS, Some(policy))
    }

    fn connect_transport_inner(
        t: Box<dyn Transport>,
        cfg: RemoteConfig,
        error_limits: QueueLimits,
        frame_codec: Option<FrameCodecPolicy>,
    ) -> Result<Self, String> {
        let tx = t
            .sender()
            .ok_or_else(|| "this transport cannot send from several threads".to_string())?;
        let shared = Arc::new(Shared {
            view: Mutex::new(ViewMailbox::new(cfg.view_event_capacity)),
            snapshot: ArcSwapOption::empty(),
            alive: AtomicBool::new(true),
            stop: AtomicBool::new(false),
            metrics: Mutex::new(RemoteMetrics::default()),
            errors: ErrorQueue::new(error_limits),
            tx: tx.clone(),
        });
        let (event_tx, events) = channel::<BridgeEvent<G::Event>>();
        let (ready_tx, ready_rx) = channel::<Result<Info, String>>();
        let thread_shared = shared.clone();
        let thread_cfg = cfg.clone();
        let thread_codec = frame_codec;
        let last_input = Arc::new(Mutex::new(None));
        let thread_last_input = last_input.clone();
        let thread = std::thread::Builder::new()
            .name("orr-remote".to_string())
            .spawn(move || {
                run::<G>(
                    t,
                    thread_cfg,
                    &thread_shared,
                    &event_tx,
                    ready_tx,
                    thread_last_input,
                    thread_codec,
                );
                thread_shared.alive.store(false, Ordering::Release);
                thread_shared.tx.close();
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
    /// held-input dedupe cache. A full transport queue returns
    /// [`BridgeError::Backpressure`] and accepts nothing. After terminal
    /// disconnect, outcomes of previously accepted fire-and-forget requests
    /// may be uncertain.
    pub fn request(&self, method: &str, params: J) -> Result<(), BridgeError> {
        if matches!(method, "sim.start" | "sim.stop" | "sim.branch") {
            clear_last_input(&self.last_input);
        }
        if !self.shared.alive.load(Ordering::Acquire) {
            return Err(BridgeError::Disconnected);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.tx
            .try_send(Request {
                id: Some(id),
                method: method.to_string(),
                params,
            })
            .map_err(|error| match error {
                SendError::Backpressure => BridgeError::Backpressure,
                SendError::Disconnected => BridgeError::Disconnected,
            })
    }

    /// Errors the server answered to controls, commands and requests since the last call.
    pub fn take_errors(&self) -> Vec<RpcError> {
        self.shared.errors.take()
    }

    /// Current outgoing/incoming transport accounting, when the transport
    /// provides bounded queues.
    pub fn transport_queue_stats(&self) -> Option<TransportQueueStats> {
        self.tx.queue_stats()
    }

    /// Current retained RPC-error queue accounting. `closed` becomes true
    /// after the queue overflows and the bridge terminates.
    pub fn error_queue_stats(&self) -> QueueStats {
        self.shared.errors.stats()
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
        self.tx.close();
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
    frame_codec_policy: Option<FrameCodecPolicy>,
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
    let checked_identity = cfg.expected_identity.is_some();
    let mut identity_validated = false;
    let mut selected_codec_limits = None;
    let mut frame_codec: Option<FrameCodecClient> = None;

    // Unchecked callers keep the historical handshake. Checked callers gate
    // subscription on an explicit ERP identity and the complete schema from
    // this very transport, before any binary frame is decoded.
    let mut first: Vec<(Pending, &str, J)> = Vec::new();
    if let Some(tok) = &cfg.token {
        first.push((Pending::Auth, "auth", json!({"token": tok})));
    }
    if checked_identity || frame_codec_policy.is_some() {
        if cfg.token.is_none() {
            first.push((Pending::Discover, "rpc.discover", J::Null));
        }
    } else {
        let mut subscribe = json!({"topics": ["frames", "events", "notes"], "max_fps": cfg.max_fps.max(1), "source": cfg.source});
        if cfg.view_delivery != ViewDeliveryMode::Legacy {
            subscribe["view_delivery"] = json!(1);
        }
        first.push((Pending::Subscribe, "watch.subscribe", subscribe));
        first.push((Pending::State, "sim.state", J::Null));
    }
    let mut next_id = 1;
    for (kind, method, params) in first {
        if let Err(e) =
            queue_handshake(t.as_mut(), &mut pending, &mut next_id, kind, method, params)
        {
            if let Some(r) = ready.take() {
                let message = if matches!(kind, Pending::Discover | Pending::Schema) {
                    identity_check_error(e)
                } else {
                    e
                };
                let _ = r.send(Err(message));
            }
            return;
        }
    }

    while !shared.stop.load(Ordering::Acquire) {
        if frame_codec
            .as_ref()
            .and_then(|codec| codec.recovery_deadline)
            .is_some_and(|deadline| std::time::Instant::now() >= deadline)
        {
            fail(
                shared,
                &mut ready,
                "frame codec reset did not produce a valid Full before its deadline".into(),
            );
            return;
        }
        let msg = match t.recv(POLL) {
            Ok(Some(m)) => m,
            Ok(None) => continue,
            Err(e) => {
                if let Some(r) = ready.take() {
                    let checking_identity = pending
                        .values()
                        .any(|kind| matches!(kind, Pending::Discover | Pending::Schema));
                    let message = if checking_identity {
                        identity_check_error(e)
                    } else {
                        format!("{e}")
                    };
                    let _ = r.send(Err(message));
                }
                return;
            }
        };
        match msg {
            Incoming::Text(text) => {
                let Ok(j) = serde_json::from_str::<J>(&text) else {
                    let checking_identity = pending
                        .values()
                        .any(|kind| matches!(kind, Pending::Discover | Pending::Schema));
                    if checking_identity {
                        fail(
                            shared,
                            &mut ready,
                            identity_check_error("malformed JSON during the identity handshake"),
                        );
                        return;
                    }
                    if delivery == Some(RemoteViewDelivery::Fenced) {
                        fail(shared, &mut ready, "malformed negotiated JSON".into());
                        return;
                    }
                    continue;
                };
                if let Some(id) = j.get("id").and_then(J::as_u64) {
                    let error = j.get("error").map(RpcError::from_json);
                    match (pending.remove(&id), error) {
                        (Some(Pending::Discover | Pending::Schema), Some(e)) => {
                            if let Some(r) = ready.take() {
                                let _ = r.send(Err(identity_check_error(e)));
                            }
                            return;
                        }
                        (Some(Pending::ResetSubscribe), Some(e)) => {
                            fail(
                                shared,
                                &mut ready,
                                format!("frame codec reset refused: {e}"),
                            );
                            return;
                        }
                        (Some(Pending::Auth | Pending::Subscribe | Pending::State), Some(e)) => {
                            if let Some(r) = ready.take() {
                                let _ = r.send(Err(format!("{e}")));
                            }
                            return;
                        }
                        (Some(Pending::Auth), None)
                            if checked_identity || frame_codec_policy.is_some() =>
                        {
                            if let Err(e) = queue_handshake(
                                t.as_mut(),
                                &mut pending,
                                &mut next_id,
                                Pending::Discover,
                                "rpc.discover",
                                J::Null,
                            ) {
                                if let Some(r) = ready.take() {
                                    let _ = r.send(Err(identity_check_error(e)));
                                }
                                return;
                            }
                        }
                        (Some(Pending::Discover), None) => {
                            let result = j.get("result").unwrap_or(&J::Null);
                            if checked_identity {
                                let expected = cfg
                                    .expected_identity
                                    .as_ref()
                                    .expect("discover is only requested for a checked bridge");
                                if let Err(e) = validate_discovery(result, expected) {
                                    fail(shared, &mut ready, e);
                                    return;
                                }
                            }
                            if let Some(policy) = frame_codec_policy {
                                if supports_frame_codec(result) {
                                    selected_codec_limits = Some(policy.limits);
                                } else if policy.mode == FrameCodecMode::Require {
                                    fail(
                                        shared,
                                        &mut ready,
                                        "the host did not advertise frame codec v1".into(),
                                    );
                                    return;
                                }
                            }
                            if checked_identity {
                                if let Err(e) = queue_handshake(
                                    t.as_mut(),
                                    &mut pending,
                                    &mut next_id,
                                    Pending::Schema,
                                    "registry.schema",
                                    J::Null,
                                ) {
                                    fail(shared, &mut ready, identity_check_error(e));
                                    return;
                                }
                            } else if let Err(e) = begin_subscription(
                                t.as_mut(),
                                &mut pending,
                                &mut next_id,
                                &cfg,
                                selected_codec_limits,
                            ) {
                                fail(shared, &mut ready, e);
                                return;
                            }
                        }
                        (Some(Pending::Schema), None) => {
                            let result = j.get("result").unwrap_or(&J::Null);
                            let expected = cfg
                                .expected_identity
                                .as_ref()
                                .expect("schema is only requested for a checked bridge");
                            if let Err(e) = validate_remote_schema(result, expected) {
                                fail(shared, &mut ready, e);
                                return;
                            }
                            identity_validated = true;
                            if let Err(e) = begin_subscription(
                                t.as_mut(),
                                &mut pending,
                                &mut next_id,
                                &cfg,
                                selected_codec_limits,
                            ) {
                                fail(shared, &mut ready, e);
                                return;
                            }
                        }
                        (Some(Pending::Subscribe), None) => {
                            let result = j.get("result").unwrap_or(&J::Null);
                            let acknowledged =
                                result.get("view_delivery").and_then(J::as_u64) == Some(1);
                            if let Some(limits) = selected_codec_limits {
                                if !acknowledged {
                                    fail(
                                        shared,
                                        &mut ready,
                                        "frame codec requires acknowledged fenced view delivery v1"
                                            .into(),
                                    );
                                    return;
                                }
                                if let Err(e) = validate_frame_codec_ack(result, limits) {
                                    fail(shared, &mut ready, e);
                                    return;
                                }
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
                                let Some(policy) = frame_codec_policy else {
                                    fail(shared, &mut ready, "frame codec policy was lost".into());
                                    return;
                                };
                                match FrameCodecClient::new(
                                    registry.clone(),
                                    limits,
                                    policy.reset_timeout,
                                    result,
                                ) {
                                    Ok(codec) => frame_codec = Some(codec),
                                    Err(e) => {
                                        fail(shared, &mut ready, e);
                                        return;
                                    }
                                }
                            } else if result.get("frame_codec").is_some() {
                                fail(
                                    shared,
                                    &mut ready,
                                    "host acknowledged frame codec without client negotiation"
                                        .into(),
                                );
                                return;
                            } else if acknowledged && cfg.view_delivery != ViewDeliveryMode::Legacy
                            {
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
                        (Some(Pending::ResetSubscribe), None) => {
                            let result = j.get("result").unwrap_or(&J::Null);
                            let Some(codec) = frame_codec.as_mut() else {
                                fail(
                                    shared,
                                    &mut ready,
                                    "unexpected frame codec reset response".into(),
                                );
                                return;
                            };
                            if result.get("view_delivery").and_then(J::as_u64) != Some(1) {
                                fail(
                                    shared,
                                    &mut ready,
                                    "reset subscription lost fenced view delivery".into(),
                                );
                                return;
                            }
                            if let Err(e) = validate_frame_codec_ack(result, codec.limits) {
                                fail(shared, &mut ready, e);
                                return;
                            }
                            let old_subscription = codec.subscription;
                            let new_subscription = match codec_exact(result, "subscription") {
                                Ok(value) if value != old_subscription => value,
                                _ => {
                                    fail(
                                        shared,
                                        &mut ready,
                                        "reset subscription did not change identity".into(),
                                    );
                                    return;
                                }
                            };
                            let prepared_codec = match codec.prepare_renewal(result) {
                                Ok(prepared) if prepared.subscription == new_subscription => {
                                    prepared
                                }
                                Ok(_) => {
                                    fail(
                                        shared,
                                        &mut ready,
                                        "reset subscription identity changed during prepare".into(),
                                    );
                                    return;
                                }
                                Err(e) => {
                                    fail(shared, &mut ready, e);
                                    return;
                                }
                            };
                            let view_result = {
                                let mut view =
                                    shared.view.lock().unwrap_or_else(|p| p.into_inner());
                                match view.prepare_renewal(result) {
                                    Ok(prepared) => {
                                        view.commit_renewal(prepared);
                                        Ok(())
                                    }
                                    Err(error) => Err(error),
                                }
                            };
                            if let Err(e) = view_result {
                                fail(shared, &mut ready, e);
                                return;
                            }
                            codec.commit_renewal(registry.clone(), prepared_codec);
                            state.last = None;
                            codec.recovery_request = None;
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
                            if !shared.errors.push(e) {
                                fail(
                                    shared,
                                    &mut ready,
                                    "remote RPC error queue saturated; connection closed and accepted RPC outcomes may be uncertain".into(),
                                );
                                return;
                            }
                        }
                        _ => {}
                    }
                } else if let Some(method) = j.get("method").and_then(J::as_str) {
                    if checked_identity && !identity_validated {
                        continue;
                    }
                    let params = j.get("params").cloned().unwrap_or(J::Null);
                    if has_branch_note(method, &params) {
                        clear_last_input(&last_input);
                    }
                    if delivery == Some(RemoteViewDelivery::Fenced) {
                        if method == "watch.frame_codec_reset" {
                            if let Some(codec) = frame_codec.as_mut() {
                                if let Err(e) = codec.accept_reset_notice(&params) {
                                    fail(shared, &mut ready, e);
                                    return;
                                }
                            }
                            continue;
                        }
                        if frame_codec
                            .as_ref()
                            .is_some_and(|codec| codec.recovery_request.is_some())
                            && matches!(
                                method,
                                "watch.events" | "watch.notes" | "watch.view.inactive"
                            )
                        {
                            continue;
                        }
                        // An inactive cut is authoritative only after the mailbox
                        // validates it. It cannot cancel an announced Full reset.
                        if method == "watch.view.inactive"
                            && frame_codec
                                .as_ref()
                                .is_some_and(|codec| codec.reset_notice.is_some())
                        {
                            fail(
                                shared,
                                &mut ready,
                                "inactive source interrupted an announced Full reset".into(),
                            );
                            return;
                        }
                        if let Err(e) = on_fenced_notification::<G>(method, &params, shared) {
                            fail(shared, &mut ready, e);
                            return;
                        }
                        if method == "watch.view.inactive" {
                            if let Some(codec) = frame_codec.as_mut() {
                                codec.source_inactive = true;
                                codec.recovery_deadline = None;
                            }
                        }
                    } else {
                        on_notification::<G>(method, &params, events);
                    }
                }
            }
            Incoming::Wire(b) => {
                if checked_identity && !identity_validated {
                    continue;
                }
                let tick_rate = info.as_ref().map_or(60, |i| i.tick_rate);
                if let Some(codec) = frame_codec.as_mut() {
                    if codec.recovery_request.is_some() {
                        // The one correlated replacement subscription is in
                        // flight. Old-subscription frames cannot affect either
                        // the decoder or the displayed snapshot.
                        continue;
                    }
                    let started = std::time::Instant::now();
                    let (meta, identity, record) =
                        match decode_frame_record_message(&b, codec.limits) {
                            Ok(decoded) => decoded,
                            Err(error) => {
                                fail(
                                    shared,
                                    &mut ready,
                                    format!("malformed negotiated frame record: {error}"),
                                );
                                return;
                            }
                        };
                    let (target, is_reset) =
                        match codec.validate_record_identity(&meta, identity, &record) {
                            Ok(validated) => validated,
                            Err(error) => {
                                fail(shared, &mut ready, error);
                                return;
                            }
                        };
                    let prepared_decode = match codec.decoder.prepare_decode(&record) {
                        Ok(prepared) => prepared,
                        Err(DecodeError::NeedFull(
                            NeedFullReason::MissingBaseline | NeedFullReason::StaleBase { .. },
                        )) if !is_reset => {
                            if let Err(error) = queue_frame_codec_recovery(
                                t.as_mut(),
                                &mut pending,
                                &mut next_id,
                                &cfg,
                                codec,
                                shared,
                            ) {
                                fail(shared, &mut ready, error);
                                return;
                            }
                            continue;
                        }
                        Err(error) => {
                            fail(
                                shared,
                                &mut ready,
                                format!("invalid negotiated frame record: {error}"),
                            );
                            return;
                        }
                    };
                    if prepared_decode.target_stamp() != target
                        || prepared_decode.frame().tick() != target.tick
                        || prepared_decode.frame().checksum() != target.frame_checksum
                    {
                        fail(
                            shared,
                            &mut ready,
                            "decoded frame differs from its record stamp".into(),
                        );
                        return;
                    }
                    let raw_len = match &record {
                        FrameRecord::Full { bytes, .. } => bytes.len(),
                        FrameRecord::Delta { target_len, .. } => match usize::try_from(*target_len)
                        {
                            Ok(length) => length,
                            Err(_) => {
                                fail(
                                    shared,
                                    &mut ready,
                                    "frame record length exceeds address space".into(),
                                );
                                return;
                            }
                        },
                    };
                    let sizes = FrameSizes {
                        message: b.len() as u64,
                        raw: raw_len as u64,
                        decode_us: started.elapsed().as_micros() as u64,
                    };
                    if let Err(error) = on_codec_frame(
                        CodecFramePublication {
                            meta: &meta,
                            identity,
                            target,
                            is_reset,
                            prepared_decode,
                            sizes,
                        },
                        tick_rate,
                        shared,
                        &mut state,
                        codec,
                    ) {
                        fail(shared, &mut ready, error);
                        return;
                    }
                    continue;
                }
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
                if checked_identity && !identity_validated {
                    continue;
                }
                if frame_codec.is_some() {
                    fail(
                        shared,
                        &mut ready,
                        "frame codec v1 requires serialized Wire frame records".into(),
                    );
                    return;
                }
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
        && params
            .get("notes")
            .and_then(J::as_array)
            .is_some_and(|notes| {
                notes
                    .iter()
                    .any(|n| n.get("kind").and_then(J::as_str) == Some("branched"))
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
        .push(RpcError::state("view_delivery_invalid", message));
    shared.stop.store(true, Ordering::Release);
    shared.tx.close();
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

struct PreparedSnapshot {
    snapshot: Snapshot,
    last: (u64, u64, Arc<Frame>),
    seq: u64,
    frames: u64,
}

fn prepare_snapshot(
    meta: &J,
    frame: Arc<Frame>,
    tick_rate: u32,
    state: &StreamState,
    fenced: bool,
    epoch_override: Option<u64>,
    force_no_prev: bool,
) -> Result<PreparedSnapshot, String> {
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
    let epoch = if let Some(epoch) = epoch_override {
        epoch
    } else {
        match delivery {
            Some(delivery) => Stamp::parse(delivery, true)?.timeline,
            None => meta.get("epoch").and_then(J::as_u64).unwrap_or(0),
        }
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
    let prev = if force_no_prev {
        None
    } else {
        match &state.last {
            Some((t, e, f)) if *e == epoch && t.checked_add(1) == Some(tick) => Some(f.clone()),
            _ => None,
        }
    };
    let seq = state
        .seq
        .checked_add(1)
        .ok_or_else(|| "remote snapshot sequence exhausted".to_string())?;
    let frames = state
        .frames
        .checked_add(1)
        .ok_or_else(|| "remote frame count exhausted".to_string())?;
    let snapshot = Snapshot::from_parts(SnapshotParts {
        seq,
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
            ticks: frames,
            ..BridgeStats::default()
        },
        last_rollback: None,
        timeline,
    });
    Ok(PreparedSnapshot {
        snapshot,
        last: (tick, epoch, frame),
        seq,
        frames,
    })
}

fn commit_snapshot(state: &mut StreamState, prepared: PreparedSnapshot) -> Snapshot {
    state.last = Some(prepared.last);
    state.seq = prepared.seq;
    state.frames = prepared.frames;
    prepared.snapshot
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
    let prepared = prepare_snapshot(meta, frame, tick_rate, st, fenced, None, false)?;
    let mailbox_frame = if fenced {
        Some(
            view.prepare_frame(
                meta.get("delivery")
                    .ok_or("missing negotiated frame metadata")?,
                Some(prepared.snapshot.clone()),
            )?,
        )
    } else {
        None
    };
    let snap = commit_snapshot(st, prepared);
    if let Some(mailbox_frame) = mailbox_frame {
        view.commit_frame(mailbox_frame);
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

struct CodecFramePublication<'a> {
    meta: &'a J,
    identity: FrameCodecMeta,
    target: FrameStamp,
    is_reset: bool,
    prepared_decode: crate::frame_delta::PreparedDecode,
    sizes: FrameSizes,
}

fn on_codec_frame<E>(
    publication: CodecFramePublication<'_>,
    tick_rate: u32,
    shared: &Shared<E>,
    state: &mut StreamState,
    codec: &mut FrameCodecClient,
) -> Result<(), String> {
    let CodecFramePublication {
        meta,
        identity,
        target,
        is_reset,
        prepared_decode,
        sizes,
    } = publication;
    let frame = Arc::new(prepared_decode.frame().clone());
    if frame.tick() != target.tick || frame.checksum() != target.frame_checksum {
        return Err("prepared frame differs from its validated record stamp".into());
    }
    let prepared_snapshot = prepare_snapshot(
        meta,
        frame,
        tick_rate,
        state,
        true,
        Some(target.scope.timeline_epoch),
        is_reset,
    )?;
    let mut view = shared.view.lock().unwrap_or_else(|p| p.into_inner());
    let delivery = meta
        .get("delivery")
        .ok_or("missing negotiated frame metadata")?;
    let prepared_view = view.prepare_frame(delivery, Some(prepared_snapshot.snapshot.clone()))?;

    // Everything that can reject this publication has been preflighted. The
    // decoder, mailbox, stream counters and visible snapshot now commit as one
    // worker-thread transaction.
    let committed_frame = codec
        .decoder
        .commit(prepared_decode)
        .map_err(|error| error.to_string())?;
    drop(committed_frame);
    view.commit_frame(prepared_view);
    let snapshot = commit_snapshot(state, prepared_snapshot);
    shared.snapshot.store(Some(Arc::new(snapshot)));
    codec.commit_record_identity(identity, target);
    drop(view);

    let sent = meta.get("sent_at_us").and_then(J::as_u64).unwrap_or(0);
    let latency = now_us().saturating_sub(sent);
    let mut metrics = shared.metrics.lock().unwrap_or_else(|p| p.into_inner());
    metrics.frames = metrics.frames.saturating_add(1);
    metrics.frame_bytes = metrics.frame_bytes.saturating_add(sizes.message);
    metrics.last_frame_bytes = sizes.message;
    metrics.last_frame_raw_bytes = sizes.raw;
    metrics.last_latency_us = latency;
    metrics.max_latency_us = metrics.max_latency_us.max(latency);
    metrics.latency_sum_us = metrics.latency_sum_us.saturating_add(latency);
    metrics.last_decode_us = sizes.decode_us;
    Ok(())
}

#[cfg(test)]
mod codec_contract_tests {
    use super::*;
    use crate::frame_delta::Encoder;

    #[test]
    fn rejected_reset_cut_preserves_decoder_and_identity_before_publication() {
        let limits = FrameCodecPolicy::require_default().limits;
        let registry = orr_ecs::ComponentRegistryBuilder::new().build();
        let ack = json!({"subscription":"7","cursor":"0","count":"0","sequence":"0","reset_generation":"1",
            "frame_codec":{"version":1,"max_frame_bytes":limits.max_frame_bytes,
            "max_baseline_bytes":limits.max_baseline_bytes,"max_message_bytes":limits.max_message_bytes}});
        let mut codec =
            FrameCodecClient::new(registry.clone(), limits, Duration::from_secs(1), &ack).unwrap();
        let mut frame = Frame::new(registry);
        frame.set_tick(10);
        let first_scope = FrameScope {
            stream_generation: 7,
            play_epoch: 1,
            timeline_epoch: 1,
        };
        let mut encoder = Encoder::new(limits.max_frame_bytes, limits.max_baseline_bytes);
        let first = encoder.encode(&frame, first_scope).unwrap();
        let identity = FrameCodecMeta {
            subscription: 7,
            sequence: 1,
            reset_generation: 1,
        };
        let (target, _) = codec
            .validate_record_identity(&json!({"play_epoch":"1"}), identity, &first)
            .unwrap();
        codec.decoder.decode(&first).unwrap();
        codec.commit_record_identity(identity, target);
        let baseline = codec.decoder.baseline_stamp();
        let cut = json!({"subscription":"7","timeline":"2","through_cursor":"1","count":"1"});
        codec.accept_reset_notice(&json!({"subscription":"7","reset_generation":"2","next_sequence":"2",
            "scope":{"stream_generation":"7","play_epoch":"1","timeline_epoch":"2"},"delivery":cut})).unwrap();
        frame.set_tick(20);
        let record = encoder
            .encode(
                &frame,
                FrameScope {
                    timeline_epoch: 2,
                    ..first_scope
                },
            )
            .unwrap();
        let identity = FrameCodecMeta {
            subscription: 7,
            sequence: 2,
            reset_generation: 2,
        };
        for key in ["timeline", "through_cursor", "count"] {
            let mut forged = cut.clone();
            forged[key] = json!("3");
            assert!(codec
                .validate_record_identity(
                    &json!({"play_epoch":"1","delivery":forged}),
                    identity,
                    &record
                )
                .is_err());
            assert_eq!(codec.decoder.baseline_stamp(), baseline);
            assert_eq!(codec.sequence, 1);
            assert_eq!(codec.last_target, Some(target));
            assert!(codec.reset_notice.is_some());
        }
        assert!(codec
            .validate_record_identity(&json!({"play_epoch":"1","delivery":cut}), identity, &record)
            .is_ok());
    }
}
