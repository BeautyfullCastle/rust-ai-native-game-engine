//! [`ErpServer`]: the host-facing half of the ERP server.
//!
//! # Threading
//!
//! The network runs on its own tokio runtime (worker threads named
//! `orr-erp`). It accepts, authenticates and parses, then queues requests.
//! The host calls [`ErpServer::poll`] once per frame with an [`ErpTarget`]
//! that borrows its state. Most requests run there synchronously; verification
//! captures an immutable snapshot and runs on one bounded worker. Delayed
//! responses and notifications are sent by the host through the connections'
//! write queues. `poll` never waits for the network or a running worker.
//!
//! # Notifications and the frame stream
//!
//! `watch.subscribe` turns on pushes (see [`crate::methods`]). Frame
//! snapshots are binary WebSocket messages ([`crate::wire`]): the full
//! `Frame` bytes, lz4-compressed, once per published tick at most
//! `max_fps` times a second per subscriber. A subscriber whose socket
//! is behind (over [`MAX_PENDING_BYTES`] queued) skips frames instead of
//! making the queue grow.

use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::thread::JoinHandle;

use orr_edit::{EditorDoc, StoppedPlay};
use orr_session::PlayNote;
use orr_sim::{EventKey, Game, SimEvent};
use serde_json::{json, Value as J};

use crate::activity::{self, ActivityEntry, ActivityKind, ClientInfo, DEFAULT_ACTIVITY_CAPACITY};
use crate::caps::{Auth, Caps};
use crate::codec::hex_encode;
use crate::dispatch::{authorize, call, CallCtx, Effects, ErpTarget, HostLimits, TxChange};
use crate::error::*;
use crate::link::{LocalConnector, LocalFrame};
use crate::net::{
    accept_loop, bind, notification, response_err, response_ok, ConnTx, Inbound, NetShared,
};
use crate::proposals::{self, ProposalWatch, VerifyResult};
use crate::wire::{checksum_text, debug_error_name, encode_frame_message, timeline_to_json};

/// Frame data queued to one connection above which frames are skipped for it.
pub const MAX_PENDING_BYTES: usize = 16 << 20;

/// Settings of an [`ErpServer`].
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// Address to listen on (default `127.0.0.1:0`: loopback, any free port).
    pub bind: SocketAddr,
    /// How clients authenticate. No default: see [`Auth`].
    pub auth: Auth,
    /// Largest accepted request, in bytes (default 4 MiB). A larger one gets
    /// an error reply; one over four times as large closes the connection.
    pub max_message_bytes: usize,
    /// Browser `Origin` values that may connect (default none: only clients
    /// that send no `Origin`, that is, not web pages).
    pub allowed_origins: Vec<String>,
    /// Most simultaneous connections (default 64).
    pub max_connections: usize,
    /// Most requests waiting for the host, shared by local and network producers
    /// (default 4096). Zero rejects all forwarded requests; more get a "busy"
    /// error. Includes the inbox and stash, not running requests or replies.
    /// This is a request count, not a total memory bound: local payload bytes,
    /// control events, PumpedWs channels and outbound channels are not bounded
    /// by it. The network's default 4 MiB per-message limit is separate.
    pub max_queued_requests: usize,
    /// Most compact canonical request-envelope bytes waiting for the host,
    /// shared by local and network producers (default 64 MiB). Zero rejects all
    /// forwarded requests; usize::MAX is practically unrestricted. Includes
    /// inbox/stash, not active work, JSON heap overhead, pre-parse allocations,
    /// control events, replies or PumpedWs channels. This is not an RSS bound.
    /// Adding this field requires updates to exhaustive config struct literals.
    pub max_queued_request_bytes: usize,
    /// Most requests `poll` runs per call (default 256), so a flood cannot stall a frame.
    pub max_requests_per_poll: usize,
    /// A transaction that a client leaves open this long is rolled back (default 60 s).
    pub tx_timeout: Duration,
    /// Settings of the methods (players, tick rate, step limit, scene file).
    pub limits: HostLimits,
    /// Entries of the activity log the server keeps (default 2000).
    pub activity_capacity: usize,
    /// Listen on `bind` (default true). Off, the server only serves in-process
    /// clients that connect through [`ErpServer::connector`]: no socket, no
    /// network threads.
    pub listen: bool,
    /// Optional local editor framebuffer capture endpoint; absent on headless hosts.
    pub screenshot: Option<crate::screenshot::ScreenshotService>,
}

impl ServerConfig {
    /// Loopback, any port, default limits.
    pub fn new(auth: Auth) -> Self {
        Self {
            bind: SocketAddr::from(([127, 0, 0, 1], 0)),
            auth,
            max_message_bytes: 4 << 20,
            allowed_origins: Vec::new(),
            max_connections: 64,
            max_queued_requests: 4096,
            max_queued_request_bytes: 64 << 20,
            max_requests_per_poll: 256,
            tx_timeout: Duration::from_secs(60),
            limits: HostLimits::default(),
            activity_capacity: DEFAULT_ACTIVITY_CAPACITY,
            listen: true,
            screenshot: None,
        }
    }
}

/// Why the server could not start.
#[derive(Debug)]
pub struct ServerError(pub String);

impl core::fmt::Display for ServerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ServerError {}

#[derive(Default)]
struct Subs {
    tick: bool,
    /// The tick key the connection was last told about.
    tick_seen: Option<TickKey>,
    history: bool,
    /// The history signature the connection was last told about.
    hist_seen: Option<u64>,
    events: bool,
    notes: bool,
    proposals: bool,
    activity: Option<ActivitySub>,
    frames: Option<FrameSub>,
    /// The view stream (`viewstream` topic); reuses the frame subscription's pacing.
    viewstream: Option<FrameSub>,
    /// Opt-in, cursor-fenced presentation delivery. Legacy topics stay unchanged.
    delivery: Option<ViewDelivery>,
}

const MAX_VIEW_BATCH_EVENTS: usize = 100_000;

#[derive(Clone)]
struct ViewNoteTally {
    count: u64,
    order: u64,
    note: PlayNote,
}

struct ViewDelivery {
    subscription: u64,
    timeline: u64,
    identity: Option<FrameKey>,
    cursor: u64,
    count: u64,
    loss_generation: u64,
    notes: [Option<ViewNoteTally>; 6],
    last_cut: Option<(Option<FrameKey>, u64, u64, u64)>,
}

impl ViewDelivery {
    fn new(subscription: u64) -> Self {
        Self {
            subscription,
            timeline: 1,
            identity: None,
            cursor: 0,
            count: 0,
            // The first complete snapshot is an explicit presentation baseline.
            loss_generation: 1,
            notes: std::array::from_fn(|_| None),
            last_cut: None,
        }
    }

    fn advance(&mut self, count: usize) {
        self.cursor = self.cursor.checked_add(1).expect("view cursor exhausted");
        self.count = self.count.saturating_add(count as u64);
    }

    fn lost(&mut self) {
        self.loss_generation = self
            .loss_generation
            .checked_add(1)
            .expect("view loss generation exhausted");
    }

    fn observe(&mut self, identity: FrameKey) {
        if self.identity.is_some_and(|old| old != identity) {
            self.timeline = self
                .timeline
                .checked_add(1)
                .expect("view timeline exhausted");
            self.lost();
        }
        self.identity = Some(identity);
    }

    fn notification_meta(&self) -> J {
        json!({"subscription": self.subscription.to_string(), "timeline": self.timeline.to_string(),
            "cursor": self.cursor.to_string(), "count": self.count.to_string()})
    }

    fn frame_meta(&self) -> J {
        let mut notes: Vec<_> = self.notes.iter().flatten().collect();
        notes.sort_by_key(|n| n.order);
        json!({"subscription": self.subscription.to_string(), "timeline": self.timeline.to_string(),
            "through_cursor": self.cursor.to_string(), "count": self.count.to_string(),
            "loss_generation": self.loss_generation.to_string(),
            "lifecycle": notes.into_iter().map(|n| json!({"count": n.count.to_string(),
                "order": n.order.to_string(), "note": note_json(n.note)})).collect::<Vec<_>>()})
    }

    fn record_notes(&mut self, notes: &[PlayNote]) {
        let start = self.count;
        self.advance(notes.len());
        for (i, note) in notes.iter().copied().enumerate() {
            let kind = match note {
                PlayNote::Seeked { .. } => 0,
                PlayNote::Branched { .. } => 1,
                PlayNote::Paused { .. } => 2,
                PlayNote::Resumed { .. } => 3,
                PlayNote::DebugRejected(_) => 4,
                PlayNote::SeekRejected { .. } => 5,
            };
            let count = self.notes[kind]
                .as_ref()
                .map_or(1, |n| n.count.saturating_add(1));
            self.notes[kind] = Some(ViewNoteTally {
                count,
                order: start.saturating_add(i as u64).saturating_add(1),
                note,
            });
        }
    }
}

struct ActivitySub {
    /// The newest entry the connection was sent (or was there when it subscribed).
    seen: u64,
    reads: bool,
}

/// What a `frames` subscription carries (`watch.subscribe` parameter `source`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FrameSource {
    /// The play session's frames; nothing in edit mode.
    Sim,
    /// The play frame while playing, else the scene's preview frame.
    View,
    /// The staged scene of a proposal (edit mode only).
    Proposal(u64),
}

struct FrameSub {
    min_interval: Duration,
    last_sent: Option<Instant>,
    last_key: Option<FrameKey>,
    source: FrameSource,
    /// Client mode only: keep the next complete snapshot as an event-reset
    /// baseline until this subscriber can receive it.
    client_reset_pending: bool,
    /// Once a reset baseline is delivered, reject delayed event records at or
    /// before its tick until a later normal discontinuity frame is delivered.
    client_event_floor: Option<u64>,
    /// Release the event floor only after the later timeline's frame is queued.
    client_clear_event_floor_after_frame: bool,
}

impl FrameSub {
    /// Observe reset/discontinuity flags on a client-mode frame this
    /// subscriber has not received yet. `last_key` stops a cached reset pulse
    /// from rearming after its baseline was already delivered.
    fn observe_client_frame(&mut self, key: FrameKey, bytes: &[u8]) {
        if self.last_key == Some(key) {
            return;
        }
        if let Some(info) = client_frame_info(bytes) {
            if info.events_reset {
                self.client_reset_pending = true;
                self.client_clear_event_floor_after_frame = false;
            } else if !self.client_reset_pending
                && self.client_event_floor.is_some()
                && info.discontinuity
            {
                self.client_clear_event_floor_after_frame = true;
            }
        }
    }

    /// Drop event batches while a reset snapshot is pending and filter delayed
    /// event records at or before the delivered baseline tick.
    fn client_events_for_send(&self, bytes: &Arc<Vec<u8>>) -> Option<Arc<Vec<u8>>> {
        if self.client_reset_pending {
            return None;
        }
        let Some(floor) = self.client_event_floor else {
            return Some(bytes.clone());
        };
        let batch = orr_viewstream::EventBatch::decode(bytes).ok()?;
        let events: Vec<_> = batch
            .events
            .into_iter()
            .filter(|event| event.tick > floor)
            .collect();
        if events.is_empty() {
            return None;
        }
        Some(Arc::new(orr_viewstream::EventBatch { events }.encode()))
    }

    /// Turn the newest cached frame into a reset baseline if needed. A normal
    /// discontinuity clears an older baseline's event floor only after its
    /// snapshot has been queued.
    fn client_frame_for_send(&mut self, bytes: &Arc<Vec<u8>>) -> Option<Arc<Vec<u8>>> {
        if !self.client_reset_pending && !self.client_clear_event_floor_after_frame {
            return Some(bytes.clone());
        }
        let info = client_frame_info(bytes)?;
        if self.client_reset_pending {
            let baseline = orr_viewstream::reset_frame_events(bytes).ok()?;
            self.client_event_floor = Some(info.tick);
            self.client_reset_pending = false;
            self.client_clear_event_floor_after_frame = false;
            Some(Arc::new(baseline))
        } else {
            self.client_event_floor = None;
            self.client_clear_event_floor_after_frame = false;
            Some(bytes.clone())
        }
    }
}

struct Conn {
    tx: ConnTx,
    client: String,
    caps: Caps,
    binary: bool,
    subs: Subs,
    requests: u64,
    connected_ms: u64,
}

/// Counters, for logs and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ServerStats {
    /// Requests executed.
    pub requests: u64,
    /// Requests that ended in an error.
    pub errors: u64,
    /// Frame messages built.
    pub frames_built: u64,
    /// Frame messages sent (one per subscriber).
    pub frames_sent: u64,
    /// Frame messages skipped because a subscriber was behind.
    pub frames_skipped: u64,
    /// Size in bytes of the newest frame message.
    pub last_frame_bytes: u64,
}

/// What one [`ErpServer::poll`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PollReport {
    /// Requests executed.
    pub requests: usize,
    /// True if requests are left in the queue (the cap per poll was reached).
    pub more: bool,
    /// A client called `debug.panic`: the host is to panic (see [`Host::frame`](crate::Host::frame)).
    pub crash: bool,
    /// A subscriber is waiting out its frame rate cap with a newer frame to
    /// get: the host should look again soon, not sleep.
    pub frame_pending: bool,
}

type TickKey = (bool, u64, u64, bool);
/// Identifies one published frame: `(kind, a, b)`. Play: (1, head tick, epoch);
/// scene preview: (2, document revision, 0); proposal preview: (3, id, checksum).
/// The last number of a play key stands for what the timeline shows besides the
/// tick (playing or paused, speed, recorded range, branches): pausing changes
/// the timeline of a frame without changing its tick, and a view must hear of it.
type FrameKey = (u8, u64, u64, u64);

/// A frame built for the subscribers that want it: the wire message (sockets)
/// and the shared copy (in-process), each made on first use.
#[derive(Default)]
struct BuiltFrame {
    wire: Option<Arc<Vec<u8>>>,
    local: Option<Arc<LocalFrame>>,
    /// The view stream message of this frame.
    stream: Option<Arc<Vec<u8>>>,
}

/// Host-owned routing and activity context. Never moved to a worker.
struct VerifyRequest {
    conn: u64,
    id: Option<J>,
    client: String,
    method: String,
    /// Only the proposal id is needed by the activity log; do not retain replay text.
    params: J,
}

struct VerifyTask {
    serial: u64,
    request: VerifyRequest,
    cancel: Arc<AtomicBool>,
    thread: JoinHandle<Result<VerifyResult, RpcError>>,
    notified: bool,
}

struct PendingScreenshot {
    request: crate::screenshot::CaptureRequest,
    conn: u64,
    id: J,
    view_key: Option<FrameKey>,
    incarnation: u64,
}

fn screenshot_trace_rpc_id(id: &J) -> u64 {
    if let Some(id) = id.as_u64() {
        return id;
    }
    if let Some(id) = id.as_str() {
        let hash = id.bytes().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        });
        return hash | (1 << 63);
    }
    0
}

fn verification_panic() -> RpcError {
    RpcError::new(INTERNAL_ERROR, "panic", "verification panicked (a bug); the live play session was not changed")
}

/// The ERP server. See the module docs.
pub struct ErpServer {
    rt: Option<tokio::runtime::Runtime>,
    addr: SocketAddr,
    inbox: Receiver<Inbound>,
    cfg: ServerConfig,
    conns: BTreeMap<u64, Conn>,
    tx_owner: Option<(u64, Instant)>,
    net: Arc<NetShared>,
    stash: Option<Inbound>,
    crash: bool,
    frame_pending: bool,
    events_out: Vec<(EventKey, Vec<u8>)>,
    notes_out: Vec<PlayNote>,
    pending_view_losses: usize,
    view_incarnation: u64,
    next_view_subscription: u64,
    last_hist_check: Option<Instant>,
    last_prop_check: Option<Instant>,
    prop_watch: Option<ProposalWatch>,
    last_play: Option<StoppedPlay>,
    frame_cache: Vec<(FrameKey, BuiltFrame)>,
    /// What the last view stream frame was built from (kind, epoch or document revision), to flag a jump.
    vs_last: Option<(u8, u64)>,
    /// Client mode: the newest frame message of the session and its sequence number.
    client_frame: Option<(u64, Arc<Vec<u8>>)>,
    stats: ServerStats,
    started: Instant,
    activity: VecDeque<ActivityEntry>,
    next_seq: u64,
    verify_task: Option<VerifyTask>,
    next_verify_serial: u64,
    structured_input: Option<Box<dyn std::any::Any + Send + Sync>>,
    managed_input: crate::input::ManagedHeld,
    pending_screenshot: Option<PendingScreenshot>,
}

impl ErpServer {
    /// Binds the socket and starts the network threads.
    ///
    /// Refuses `Auth::DevNoAuth` on a non-loopback address, and an empty token list.
    pub fn start(cfg: ServerConfig) -> Result<ErpServer, ServerError> {
        match &cfg.auth {
            Auth::DevNoAuth if !cfg.bind.ip().is_loopback() => {
                return Err(ServerError(format!(
                    "refusing to serve without tokens on {}: dev mode is only allowed on a loopback address",
                    cfg.bind
                )));
            }
            Auth::Tokens(t) if t.is_empty() => {
                return Err(ServerError(
                    "no tokens configured (use Auth::DevNoAuth for local development)".into(),
                ))
            }
            _ => {}
        }
        let (rt, listener) = if cfg.listen {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("orr-erp")
                .enable_all()
                .build()
                .map_err(|e| ServerError(format!("cannot start the network runtime: {e}")))?;
            let listener = bind(&rt, cfg.bind)
                .map_err(|e| ServerError(format!("cannot listen on {}: {e}", cfg.bind)))?;
            (Some(rt), Some(listener))
        } else {
            (None, None)
        };
        let addr = match &listener {
            Some(l) => l.local_addr().map_err(|e| ServerError(e.to_string()))?,
            None => SocketAddr::from(([0, 0, 0, 0], 0)),
        };
        let (tx, inbox) = channel();
        let queued = Arc::new(AtomicUsize::new(0));
        let shared = Arc::new(NetShared {
            auth: cfg.auth.clone(),
            max_message_bytes: cfg.max_message_bytes,
            allowed_origins: cfg.allowed_origins.clone(),
            max_connections: cfg.max_connections,
            max_queued: cfg.max_queued_requests,
            max_queued_bytes: cfg.max_queued_request_bytes,
            inbox: tx,
            next_id: AtomicU64::new(1),
            conns: AtomicUsize::new(0),
            queued: queued.clone(),
            queued_bytes: Arc::new(AtomicUsize::new(0)),
        });
        if let (Some(rt), Some(listener)) = (&rt, listener) {
            rt.spawn(accept_loop(listener, shared.clone()));
        }
        Ok(ErpServer {
            rt,
            net: shared,
            stash: None,
            crash: false,
            frame_pending: false,
            addr,
            inbox,
            cfg,
            conns: BTreeMap::new(),
            tx_owner: None,
            events_out: Vec::new(),
            notes_out: Vec::new(),
            pending_view_losses: 0,
            view_incarnation: 1,
            next_view_subscription: 1,
            last_hist_check: None,
            last_prop_check: None,
            prop_watch: None,
            last_play: None,
            frame_cache: Vec::new(),
            vs_last: None,
            client_frame: None,
            stats: ServerStats::default(),
            started: Instant::now(),
            activity: VecDeque::new(),
            next_seq: 1,
            verify_task: None,
            next_verify_serial: 1,
            structured_input: None,
            managed_input: crate::input::ManagedHeld::default(),
            pending_screenshot: None,
        })
    }

    /// Enables reflected held input for one game through `registry.input` and
    /// `sim.input_value`. This is opt-in: raw `sim.input` remains unchanged.
    /// Derivation is installed only when ERP starts that game's local session.
    /// Existing bridge sessions must keep their own command derivation.
    /// `max_players` must be 1..=16 and is enforced by `sim.start`.
    pub fn set_structured_input<G: Game>(
        &mut self,
        name: &'static str,
        max_players: u8,
        commands: impl Fn(orr_sim::PlayerSlot, &G::Input) -> Vec<G::Command> + Send + Sync + 'static,
    ) where
        G::Input: orr_reflect::Reflect,
    {
        assert!((1..=16).contains(&max_players), "structured input player limit must be 1..=16");
        self.structured_input = Some(Box::new(crate::input::StructuredInput::<G>::new(name, max_players, commands)));
    }

    /// Enables connection-bound, two-second renewable held-input grants.
    /// Install a structured input adapter first. Legacy writes retain their
    /// existing behavior except while their slot has an active managed grant.
    pub fn enable_managed_input(&mut self) {
        assert!(self.structured_input.is_some(), "managed input requires a structured adapter");
        self.managed_input.enabled = true;
    }

    /// Neutralizes managed slots and invalidates all grants synchronously.
    /// Embedders replacing public `Host::play` or controlling it out of band
    /// must call this BEFORE the replacement/transition. ERP transitions do so
    /// automatically. This never clears legacy slots or accepted commands.
    pub fn invalidate_managed_input<G: Game>(&mut self, target: &mut ErpTarget<'_, G>) {
        self.managed_input.invalidate(target);
    }

    /// Tells the server about a play session the host stopped itself (for
    /// example the editor's Stop button), so `{"kind":"last_play"}`
    /// verification inputs can use its recording. Sessions stopped through
    /// `sim.stop` are remembered without this call.
    pub fn note_stopped_play(&mut self, stopped: StoppedPlay) {
        self.last_play = Some(stopped);
    }

    /// Whether the host is a relay client (see [`HostLimits::client_session`]): its frames come from
    /// that session and it has work to do every frame.
    pub fn is_client_mode(&self) -> bool {
        self.cfg.limits.client_session.is_some()
    }

    /// The address the server listens on (with the real port); `0.0.0.0:0`
    /// if it does not listen (see [`ServerConfig::listen`]).
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// True if the server listens on a socket.
    pub fn is_listening(&self) -> bool {
        self.rt.is_some()
    }

    /// `ws://host:port` (empty if the server does not listen).
    pub fn url(&self) -> String {
        if self.is_listening() {
            format!("ws://{}", self.addr)
        } else {
            String::new()
        }
    }

    /// Makes in-process connections to this server: hand it to the thread
    /// that owns the client (an editor UI) while the host thread runs
    /// [`poll`](Self::poll). See [`crate::link`].
    pub fn connector(&self) -> LocalConnector {
        LocalConnector {
            shared: self.net.clone(),
        }
    }

    /// Sleeps until a request or a connection event arrives, at most
    /// `timeout`. A host with nothing to tick calls it instead of polling in
    /// a sleep loop, so a request is served at once and an idle host costs
    /// nothing.
    pub fn wait_for_request(&mut self, timeout: Duration) {
        if self.stash.is_some() {
            return;
        }
        // The completion wake is sent just before thread return. If it raced
        // the end of poll, check again soon without joining a running thread.
        let timeout = if self.verify_task.as_ref().is_some_and(|t| t.notified || t.thread.is_finished()) {
            timeout.min(Duration::from_millis(1))
        } else {
            timeout
        };
        if let Ok(m) = self.inbox.recv_timeout(timeout) {
            self.stash = Some(m);
        }
    }

    /// The settings the server runs with.
    pub fn config(&self) -> &ServerConfig {
        &self.cfg
    }

    /// Authenticated connections right now (as of the last `poll`).
    pub fn connection_count(&self) -> usize {
        self.conns.len()
    }

    /// The clients connected right now (as of the last `poll`), oldest first.
    pub fn clients(&self) -> Vec<ClientInfo> {
        self.conns
            .iter()
            .map(|(id, c)| ClientInfo {
                id: *id,
                name: c.client.clone(),
                caps: c.caps,
                connected_ms: c.connected_ms,
                requests: c.requests,
            })
            .collect()
    }

    /// The activity entries recorded after `seq` (0 = all still kept), oldest
    /// first, reads included. Hosts that draw the log (the editor) call this
    /// once per frame with the last `seq` they saw.
    pub fn activity_since(&self, seq: u64) -> Vec<ActivityEntry> {
        let start = self.activity.partition_point(|e| e.seq <= seq);
        self.activity.iter().skip(start).cloned().collect()
    }

    /// The `seq` of the newest entry (0 = none yet).
    pub fn activity_last_seq(&self) -> u64 {
        self.next_seq - 1
    }

    /// Milliseconds since the server started (the clock of `ActivityEntry::at_ms`).
    pub fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn push_activity(&mut self, mut e: ActivityEntry) {
        e.seq = self.next_seq;
        self.next_seq += 1;
        e.at_ms = self.elapsed_ms();
        while self.activity.len() >= self.cfg.activity_capacity.max(1) {
            self.activity.pop_front();
        }
        self.activity.push_back(e);
    }

    fn session_event(&mut self, client: &str, method: &str, summary: String, ok: bool) {
        self.push_activity(ActivityEntry {
            seq: 0,
            at_ms: 0,
            client: client.to_string(),
            kind: ActivityKind::Session,
            method: method.to_string(),
            summary,
            ok,
            error: (!ok).then(|| "the token was refused".to_string()),
            read: false,
            entities: Vec::new(),
            proposal: None,
            change: None,
            diff: None,
            verify: None,
        });
    }

    fn activity_list(&self, params: &J) -> J {
        let lp = activity::list_params(params).unwrap_or(activity::ListParams {
            since: 0,
            limit: 200,
            include_reads: false,
        });
        let start = self.activity.partition_point(|e| e.seq <= lp.since);
        let mut list: Vec<&ActivityEntry> = self
            .activity
            .iter()
            .skip(start)
            .filter(|e| lp.include_reads || !e.read)
            .collect();
        let truncated = list.len() > lp.limit;
        if truncated {
            list.drain(..list.len() - lp.limit);
        }
        let clients: Vec<J> = self
            .conns
            .values()
            .map(|c| {
                json!({
                    "client": c.client,
                    "capabilities": c.caps.list().into_iter().map(crate::caps::Cap::name).collect::<Vec<_>>(),
                    "requests": c.requests,
                    "connected_ms": c.connected_ms,
                })
            })
            .collect();
        json!({"entries": list.iter().map(|e| e.to_json()).collect::<Vec<_>>(), "last_seq": self.next_seq - 1, "truncated": truncated, "clients": clients, "now_ms": self.elapsed_ms()})
    }

    /// Counters.
    pub fn stats(&self) -> ServerStats {
        self.stats
    }

    /// Hands the server sim events of ticks the host ran itself (real-time
    /// play), to forward to `events` subscribers. Dropped if nobody listens.
    pub fn push_events<E: bytemuck::Pod>(&mut self, events: &[SimEvent<E>]) {
        if !self
            .conns
            .values()
            .any(|c| c.subs.events || c.subs.viewstream.is_some())
        {
            return;
        }
        for (i, e) in events.iter().enumerate() {
            if self.events_out.len() >= MAX_VIEW_BATCH_EVENTS {
                self.pending_view_losses =
                    self.pending_view_losses.saturating_add(events.len() - i);
                break;
            }
            self.events_out
                .push((e.key, bytemuck::bytes_of(&e.payload).to_vec()));
        }
    }

    /// Runs the queued requests against `target`, then sends notifications
    /// and frame snapshots. Call it once per host frame. Never blocks on the network.
    /// Managed held-input expiry is serviced at poll/request boundaries, not
    /// during a long synchronous step. Grants do not fence explicit commands.
    pub fn poll<G: Game>(&mut self, target: &mut ErpTarget<'_, G>) -> PollReport {
        let now = Instant::now();
        self.poll_screenshot(target, now);
        self.managed_input.expire(target, now);
        self.complete_verification(target);
        // These events came from the host's real-time advance after the last
        // poll. Fence them BEFORE any queued request can seek/replace its session.
        self.collect_view_notes(target);
        let pending_events = std::mem::take(&mut self.events_out);
        self.publish_delivery_events(&pending_events);
        self.events_out = pending_events;
        let losses = std::mem::take(&mut self.pending_view_losses);
        if losses > 0 {
            for c in self.conns.values_mut() {
                if let Some(d) = c
                    .subs
                    .delivery
                    .as_mut()
                    .filter(|d| d.identity.is_some_and(|identity| identity.0 == 1))
                {
                    d.advance(losses);
                    d.lost();
                }
            }
        }
        let mut report = PollReport::default();
        let mut budget = self.cfg.max_requests_per_poll;
        loop {
            let msg = match self.stash.take() {
                Some(m) => m,
                None => match self.inbox.try_recv() {
                    Ok(m) => m,
                    Err(_) => break,
                },
            };
            match msg {
                Inbound::Connected {
                    conn,
                    client,
                    caps,
                    tx,
                    binary,
                } => {
                    let connected_ms = self.elapsed_ms();
                    if client != crate::caps::USER_CLIENT {
                        self.session_event(
                            &client,
                            "session.connect",
                            activity::session_summary("connected", Some(caps)),
                            true,
                        );
                    }
                    self.conns.insert(
                        conn,
                        Conn {
                            tx,
                            client,
                            caps,
                            binary,
                            subs: Subs::default(),
                            requests: 0,
                            connected_ms,
                        },
                    );
                }
                Inbound::Disconnected { conn } => {
                    if let Some(c) = self
                        .conns
                        .get(&conn)
                        .filter(|c| c.client != crate::caps::USER_CLIENT)
                    {
                        let (name, n) = (c.client.clone(), c.requests);
                        self.session_event(
                            &name,
                            "session.disconnect",
                            format!("disconnected ({n} requests)"),
                            true,
                        );
                    }
                    self.disconnected(conn, target);
                }
                Inbound::VerificationReady { serial } => {
                    if let Some(task) = self.verify_task.as_mut().filter(|t| t.serial == serial) {
                        task.notified = true;
                    }
                    self.complete_verification(target);
                }
                Inbound::AuthFailed => self.session_event(
                    "?",
                    "session.auth_failed",
                    "authentication failed".to_string(),
                    false,
                ),
                Inbound::Request {
                    permit,
                    conn,
                    id,
                    method,
                    params,
                } => {
                    drop(permit); // Capacity counts waiting requests, not running work or replies.
                    self.request(target, conn, id, &method, &params);
                    report.requests += 1;
                    budget -= 1;
                    if budget == 0 {
                        report.more = true;
                        break;
                    }
                }
            }
        }
        if let Some((_, since)) = self.tx_owner {
            if now.duration_since(since) > self.cfg.tx_timeout && target.doc.in_tx() {
                let _ = target.doc.rollback_tx();
                self.tx_owner = None;
            }
        }
        self.complete_verification(target);
        self.poll_screenshot(target, Instant::now());
        self.publish(target, now);
        self.managed_input.expire(target, Instant::now());
        report.crash = std::mem::take(&mut self.crash);
        report.frame_pending = std::mem::take(&mut self.frame_pending);
        report
    }

    fn disconnected<G: Game>(&mut self, conn: u64, target: &mut ErpTarget<'_, G>) {
        self.managed_input.disconnect(target, conn);
        self.conns.remove(&conn);
        if self.pending_screenshot.as_ref().is_some_and(|pending| pending.conn == conn) {
            if let (Some(service), Some(pending)) = (&self.cfg.screenshot, self.pending_screenshot.take()) {
                service.cancel(pending.request.serial);
            }
        }
        if let Some(task) = self.verify_task.as_ref().filter(|t| t.request.conn == conn) {
            task.cancel.store(true, Ordering::Relaxed);
        }
        if self.tx_owner.is_some_and(|(c, _)| c == conn) {
            // The client left with a transaction open: take it back.
            if target.doc.in_tx() {
                let _ = target.doc.rollback_tx();
            }
            self.tx_owner = None;
        }
    }

    fn screenshot_request<G: Game>(
        &mut self,
        target: &ErpTarget<'_, G>,
        conn: u64,
        id: Option<J>,
        caps: Caps,
        params: &J,
    ) {
        let authorized = authorize("view.screenshot", caps, target.play.is_some(), params);
        let Some(id) = id else {
            self.stats.requests += 1;
            if let Some(c) = self.conns.get_mut(&conn) { c.requests += 1; }
            return;
        };
        if self.pending_screenshot.as_ref().is_some_and(|pending| pending.conn == conn && pending.id == id) {
            // JSON-RPC IDs are unique per connection while pending; share the
            // original terminal result rather than issuing a second capture.
            return;
        }
        let (options, timeout) = match authorized.and_then(|_| parse_screenshot_options(params)) {
            Ok(value) => value,
            Err(error) => {
                self.respond_screenshot_error(conn, &id, error);
                return;
            }
        };
        let Some(service) = self.cfg.screenshot.clone() else {
            self.respond_screenshot_error(conn, &id, view_unavailable());
            return;
        };
        if self.cfg.limits.game.name.len() > 256 {
            self.respond_screenshot_error(conn, &id, view_capture_failed());
            return;
        }
        let requested = current_view_state(target);
        let deadline = Instant::now() + timeout;
        match service.submit(options, requested, deadline) {
            Ok(request) => {
                crate::screenshot::diagnostic_trace_begin(request.serial, conn, screenshot_trace_rpc_id(&id));
                // Scalar fields: requested timeout milliseconds; then play=1/edit=0 and requested tick.
                crate::screenshot::diagnostic_trace(request.serial, "server_admit_timeout", timeout.as_millis() as u64, 0);
                crate::screenshot::diagnostic_trace(
                    request.serial,
                    "server_admit_state",
                    matches!(requested.mode, crate::screenshot::ViewMode::Play) as u64,
                    requested.tick,
                );
                self.pending_screenshot = Some(PendingScreenshot {
                    request,
                    conn,
                    id,
                    view_key: frame_key(target, FrameSource::View),
                    incarnation: self.view_incarnation,
                });
                self.stats.requests += 1;
                if let Some(c) = self.conns.get_mut(&conn) { c.requests += 1; }
            }
            Err(crate::screenshot::ScreenshotAdmissionError::Busy) => self.respond_screenshot_error(conn, &id, view_busy()),
            Err(crate::screenshot::ScreenshotAdmissionError::Unavailable) => self.respond_screenshot_error(conn, &id, view_unavailable()),
            Err(crate::screenshot::ScreenshotAdmissionError::Invalid) => self.respond_screenshot_error(conn, &id, RpcError::params("screenshot limits are invalid")),
        }
    }

    fn poll_screenshot<G: Game>(&mut self, target: &ErpTarget<'_, G>, now: Instant) {
        let Some(pending) = self.pending_screenshot.as_ref() else { return };
        let serial = pending.request.serial;
        let conn = pending.conn;
        let id = pending.id.clone();
        let deadline = pending.request.deadline;
        let requested = pending.request.requested;
        let view_key = pending.view_key;
        let incarnation = pending.incarnation;
        let service = self.cfg.screenshot.clone();
        let outcome = if !self.conns.contains_key(&conn) {
            crate::screenshot::diagnostic_trace(serial, "server_connection_gone", 0, 0);
            None
        } else if service.as_ref().is_none_or(|service| !service.owner_available()) {
            crate::screenshot::diagnostic_trace(serial, "server_service_unavailable", 0, 0);
            Some(Err(view_unavailable()))
        } else if now >= deadline {
            crate::screenshot::diagnostic_trace(serial, "server_deadline_pre_result", 0, 0);
            Some(Err(view_timeout()))
        } else if self.view_incarnation != incarnation || frame_key(target, FrameSource::View) != view_key || current_view_state(target) != requested {
            crate::screenshot::diagnostic_trace(serial, "server_view_stale", (self.view_incarnation != incarnation) as u64, 0);
            Some(Err(view_stale()))
        } else {
            service.as_ref().and_then(|service| service.poll_result(serial)).map(|result| match result {
                Ok(image) if image.captured == requested => Ok(image),
                Ok(_) => Err(view_stale()),
                Err(crate::screenshot::CaptureError::Unavailable) => Err(view_unavailable()),
                Err(crate::screenshot::CaptureError::Stale) => Err(view_stale()),
                Err(crate::screenshot::CaptureError::Failed) => Err(view_capture_failed()),
            })
        };
        let Some(outcome) = outcome else { return };
        if let Some(service) = service { service.cancel(serial); }
        self.pending_screenshot = None;
        match outcome {
            Ok(image) => {
                crate::screenshot::diagnostic_trace(serial, "server_encode_begin", 0, 0);
                let state = image.captured;
                let mode = match state.mode { crate::screenshot::ViewMode::Edit => "edit", crate::screenshot::ViewMode::Play => "play" };
                let result = json!({
                    "status": "captured",
                    "source": "editor.app_framebuffer",
                    "game": self.cfg.limits.game.name.as_str(),
                    "build_id": crate::wire::checksum_text(self.cfg.limits.build_id),
                    "mode": mode,
                    "paused": state.paused,
                    "tick": state.tick.to_string(),
                    "epoch": state.epoch.to_string(),
                    "checksum": crate::wire::checksum_text(state.checksum),
                    "frame_seq": image.frame_seq.to_string(),
                    "ui_frame": image.ui_frame.to_string(),
                    "width": image.width,
                    "height": image.height,
                    "mime_type": "image/png",
                    "png_base64": crate::codec::b64_encode(&image.png),
                });
                let response = response_ok(&id, result);
                crate::screenshot::diagnostic_trace(
                    serial,
                    "server_encode_end",
                    image.png.len() as u64,
                    response.len() as u64,
                );
                if Instant::now() >= deadline {
                    crate::screenshot::diagnostic_trace(serial, "server_deadline_post_encode", 0, 0);
                    self.respond_screenshot_error_traced(conn, &id, view_timeout(), serial);
                } else if let Some(c) = self.conns.get(&conn) {
                    let enqueued = c.tx.send_control_text(response);
                    // a = control-text enqueue accepted by the transport queue.
                    crate::screenshot::diagnostic_trace(serial, "server_response_enqueue", enqueued as u64, 0);
                } else {
                    crate::screenshot::diagnostic_trace(serial, "server_response_connection_gone", 0, 0);
                }
            }
            Err(error) => {
                crate::screenshot::diagnostic_trace(serial, "server_terminal_error", 1, 0);
                self.respond_screenshot_error_traced(conn, &id, error, serial)
            }
        }
    }

    fn respond_screenshot_error(&mut self, conn: u64, id: &J, error: RpcError) {
        self.stats.errors += 1;
        if let Some(c) = self.conns.get_mut(&conn) {
            c.requests += 1;
            c.tx.send_control_text(response_err(id, &error));
        }
    }

    fn respond_screenshot_error_traced(&mut self, conn: u64, id: &J, error: RpcError, serial: u64) {
        self.stats.errors += 1;
        if let Some(c) = self.conns.get_mut(&conn) {
            c.requests += 1;
            let enqueued = c.tx.send_control_text(response_err(id, &error));
            crate::screenshot::diagnostic_trace(serial, "server_response_enqueue", enqueued as u64, 0);
        } else {
            crate::screenshot::diagnostic_trace(serial, "server_response_connection_gone", 0, 0);
        }
    }

    fn request<G: Game>(
        &mut self,
        target: &mut ErpTarget<'_, G>,
        conn: u64,
        id: Option<J>,
        method: &str,
        params: &J,
    ) {
        self.managed_input.expire(target, Instant::now());
        // In particular, take diagnostics before sim.stop destroys the session.
        self.collect_view_notes(target);
        let Some(c) = self.conns.get(&conn) else {
            return;
        };
        let (client, caps, tx) = (c.client.clone(), c.caps, c.tx.clone());
        if method == "view.screenshot" {
            self.screenshot_request(target, conn, id, caps, params);
            return;
        }
        if matches!(method, "proposal.verify" | "verify.self") && self.cfg.limits.client_session.is_none() {
            self.request_verification(target, conn, id, client, caps, method, params);
            return;
        }
        let mut fx = Effects::default();
        // A person's own view reads all the time (the editor refreshes its panels): not recorded,
        // so the log keeps what agents did. Its edits are recorded like anyone's.
        let recorded = method != "activity.list"
            && !(client == crate::caps::USER_CLIENT && activity::classify(method, params).1);
        let pre = if recorded && !method.starts_with("watch.") {
            activity::before(target, method, params)
        } else {
            Default::default()
        };
        let result = if method == "activity.list" {
            authorize(method, caps, false, params)
                .and_then(|_| activity::list_params(params))
                .map(|_| self.activity_list(params))
        } else if method.starts_with("watch.") {
            self.watch(target, conn, caps, method, params)
        } else if let (Some(hook), true) = (
            self.cfg.limits.client_session.clone(),
            method != "rpc.discover",
        ) {
            authorize(method, caps, false, params)
                .and_then(|_| crate::client_mode::call(&hook, method, params))
        } else {
            let limits = self.cfg.limits.clone();
            let ctx = CallCtx {
                client: &client,
                caps,
                last_play: self.last_play.as_ref(),
                tx_check: Some((conn, self.tx_owner.map(|(c, _)| c))),
            };
            match std::panic::catch_unwind(AssertUnwindSafe(|| {
                let input = self.structured_input.as_ref().and_then(|a| a.downcast_ref::<crate::input::StructuredInput<G>>());
                authorize(method, caps, target.play.is_some(), params)?;
                let mut result = if let Some(result) = self.managed_input.handle(target, input, (conn, Instant::now()), method, params) {
                    result
                } else {
                    call(target, &limits, input, &ctx, &mut fx, method, params)
                }?;
                if matches!(method, "sim.start" | "sim.stop" | "sim.pause" | "sim.seek" | "sim.branch") {
                    self.managed_input.invalidate(target);
                }
                if self.managed_input.enabled && input.is_some() {
                    match method {
                        "sim.state" => result["managed_held"] = self.managed_input.status(conn),
                        "registry.input" => result["managed_held"] = crate::input::ManagedHeld::descriptor(),
                        "rpc.discover" => result["engine"]["input"]["managed_held"] = crate::input::ManagedHeld::descriptor(),
                        _ => {}
                    }
                }
                Ok(result)
            })) {
                Ok(r) => r,
                Err(_) => {
                    // A panic may have left the sim half-stepped: drop the play session
                    // rather than keep serving from an inconsistent state.
                    self.managed_input.invalidate(target);
                    let dropped = target.play.take().is_some();
                    let note = if dropped {
                        "; the play session was dropped"
                    } else {
                        ""
                    };
                    Err(RpcError::new(
                        INTERNAL_ERROR,
                        "panic",
                        format!("the request made the host panic (a bug){note}"),
                    ))
                }
            }
        };
        if result.is_ok() && matches!(method, "sim.start" | "sim.stop" | "scene.load") {
            self.view_incarnation = self
                .view_incarnation
                .checked_add(1)
                .expect("view incarnation exhausted");
        }
        self.observe_view_timelines(target);
        self.publish_delivery_events(&fx.events);
        self.stats.requests += 1;
        if !matches!(
            method,
            "world.query"
                | "world.get"
                | "world.singleton.get"
                | "sim.state"
                | "sim.checksum"
                | "registry.schema"
                | "registry.input"
                | "registry.types"
                | "rpc.discover"
                | "history.list"
                | "proposal.list"
                | "proposal.get"
                | "proposal.preview"
                | "activity.list"
        ) && !method.starts_with("watch.")
        {
            // Something may have changed under the same frame key (a stopped and restarted session).
            self.frame_cache.clear();
            // A new or ended session is a jump for the view stream (the epoch may repeat).
            if matches!(method, "sim.start" | "sim.stop" | "scene.load") {
                self.vs_last = None;
            }
        }
        if let Some(c) = self.conns.get_mut(&conn) {
            c.requests += 1;
        }
        if recorded {
            let entry = activity::build(
                target,
                &client,
                method,
                params,
                &result,
                pre,
                fx.verify.take(),
            );
            self.push_activity(entry);
        }
        match fx.tx {
            TxChange::Opened => self.tx_owner = Some((conn, Instant::now())),
            TxChange::Closed => self.tx_owner = None,
            TxChange::None => {}
        }
        if let Some(stopped) = fx.stopped.take() {
            self.last_play = Some(stopped);
        }
        if let Some(path) = fx.scene_path.take() {
            self.cfg.limits.scene_path = Some(path);
        }
        if fx.crash {
            self.crash = true;
        }
        if !fx.events.is_empty()
            && self
                .conns
                .values()
                .any(|c| c.subs.events || c.subs.viewstream.is_some())
        {
            self.events_out.append(&mut fx.events);
        }
        if result.is_err() {
            self.stats.errors += 1;
        }
        if let Some(id) = id {
            let response = match result {
                Ok(r) => response_ok(&id, r),
                Err(e) => response_err(&id, &e),
            };
            tx.send_control_text(response);
        }
        // An opt-in subscription is acknowledged before its first fenced note.
        self.collect_view_notes(target);
        // Look right after each request that can change the proposals, so a client that
        // chains requests quickly still produces one event per step.
        if proposals_may_change(method) && self.conns.values().any(|c| c.subs.proposals) {
            self.publish_proposals(target);
        }
    }

    /// Reserve the single worker slot before copying frames, replay bytes or
    /// parsing checks. Admission stays on the host; execution never borrows it.
    #[allow(clippy::too_many_arguments)]
    fn request_verification<G: Game>(
        &mut self,
        target: &ErpTarget<'_, G>,
        conn: u64,
        id: Option<J>,
        client: String,
        caps: Caps,
        method: &str,
        params: &J,
    ) {
        self.complete_verification(target);
        self.stats.requests += 1;
        if let Some(c) = self.conns.get_mut(&conn) {
            c.requests += 1;
        }
        let request = VerifyRequest {
            conn, id, client, method: method.to_string(),
            params: json!({"id": params.get("id").and_then(J::as_str)}),
        };
        let prepared = authorize(method, caps, target.play.is_some(), params).and_then(|_| {
            if self.verify_task.is_some() {
                return Err(RpcError::new(LIMIT_EXCEEDED, "verify_busy", "this host already has a verification running; retry after it finishes"));
            }
            // Capture errors/panics are also isolated: this only reads the doc.
            std::panic::catch_unwind(AssertUnwindSafe(|| {
                proposals::prepare_verify(target.doc, &self.cfg.limits, self.last_play.as_ref(), params, method == "proposal.verify")
            })).unwrap_or_else(|_| Err(verification_panic()))
        });
        let prepared = match prepared {
            Ok(p) => p,
            Err(e) => {
                self.finish_verification(target, request, Err(e));
                return;
            }
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let serial = self.next_verify_serial;
        self.next_verify_serial = self.next_verify_serial.checked_add(1).expect("verification serial exhausted");
        // The worker owns only an inbox sender, never NetShared or ConnTx:
        // dropping the host still closes every connection immediately.
        let wake = self.net.inbox.clone();
        let spawned = std::thread::Builder::new().name("orr-verify".into()).spawn(move || {
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| prepared.run::<G>(&worker_cancel)))
                .unwrap_or_else(|_| Err(verification_panic()));
            let _ = wake.send(Inbound::VerificationReady { serial });
            result
        });
        match spawned {
            Ok(thread) => self.verify_task = Some(VerifyTask { serial, request, cancel, thread, notified: false }),
            Err(e) => self.finish_verification(target, request, Err(RpcError::new(INTERNAL_ERROR, "verify_worker", format!("cannot start verification worker: {e}")))),
        }
    }

    fn complete_verification<G: Game>(&mut self, target: &ErpTarget<'_, G>) {
        if !self.verify_task.as_ref().is_some_and(|t| t.thread.is_finished()) {
            return;
        }
        let task = self.verify_task.take().expect("finished verification exists");
        // Join only after actual exit. Cancellation alone must never release
        // capacity: an uncooperative hook could still be using a worker.
        let result = task.thread.join().unwrap_or_else(|_| Err(verification_panic()));
        let result = if task.cancel.load(Ordering::Relaxed) { Err(proposals::verify_cancelled()) } else { result };
        self.finish_verification(target, task.request, result);
    }

    fn finish_verification<G: Game>(&mut self, target: &ErpTarget<'_, G>, request: VerifyRequest, result: Result<VerifyResult, RpcError>) {
        let (result, detail) = match result {
            Ok(result) => (Ok(result.value), Some(result.detail)),
            Err(e) => (Err(e), None),
        };
        if result.is_err() {
            self.stats.errors += 1;
        }
        let entry = activity::build(target, &request.client, &request.method, &request.params, &result, Default::default(), detail);
        self.push_activity(entry);
        if let (Some(id), Some(c)) = (request.id, self.conns.get(&request.conn)) {
            let response = match result {
                Ok(value) => response_ok(&id, value),
                Err(error) => response_err(&id, &error),
            };
            c.tx.send_control_text(response);
        }
    }

    /// Sends `watch.proposals` to its subscribers if the proposals changed since last looked.
    fn publish_proposals<G: Game>(&mut self, target: &ErpTarget<'_, G>) {
        let watch = self
            .prop_watch
            .get_or_insert_with(|| ProposalWatch::capture(target.doc));
        if let Some(params) = watch.advance(target.doc) {
            let n = notification("watch.proposals", params);
            for c in self.conns.values().filter(|c| c.subs.proposals) {
                c.tx.send_text(n.clone());
            }
        }
    }

    // ---- watch ----

    fn watch<G: Game>(
        &mut self,
        target: &mut ErpTarget<'_, G>,
        conn: u64,
        caps: Caps,
        method: &str,
        params: &J,
    ) -> Result<J, RpcError> {
        authorize(method, caps, false, params)?;
        let topics: Vec<String> = match params.get("topics") {
            Some(J::Array(a)) => a
                .iter()
                .map(|t| t.as_str().map(str::to_string).ok_or_else(|| RpcError::params("'topics' must be a list of strings")))
                .collect::<Result<_, _>>()?,
            None | Some(J::Null) if method == "watch.unsubscribe" => Vec::new(),
            _ => return Err(RpcError::params("'topics' must be a list of strings (tick, history, events, notes, proposals, activity, frames, viewstream)")),
        };
        let include_reads = params
            .get("include_reads")
            .and_then(J::as_bool)
            .unwrap_or(false);
        let last_seq = self.next_seq - 1;
        let max_fps = match params.get("max_fps") {
            None | Some(J::Null) => 60,
            Some(v) => v
                .as_u64()
                .filter(|n| *n >= 1)
                .ok_or_else(|| RpcError::params("'max_fps' must be a positive integer"))?
                .min(1000),
        };
        let source = frame_source(params)?;
        let fenced = match params.get("view_delivery") {
            None | Some(J::Null) => false,
            Some(v) if v.as_u64() == Some(1) && method == "watch.subscribe" => true,
            Some(_) => {
                return Err(RpcError::params(
                    "'view_delivery' must be 1 on watch.subscribe",
                ))
            }
        };
        if fenced
            && !["frames", "events", "notes"]
                .iter()
                .all(|want| topics.iter().any(|t| t == want))
        {
            return Err(RpcError::params(
                "view_delivery 1 requires frames, events and notes together",
            ));
        }
        let client_mode = self.cfg.limits.client_session.clone();
        if let (Some(_), "watch.subscribe") = (&client_mode, method) {
            if let Some(t) = topics
                .iter()
                .find(|t| !matches!(t.as_str(), "viewstream" | "activity"))
            {
                return Err(RpcError::state(
                    "not_in_client_mode",
                    format!("topic '{t}' is not available in client mode (this host is a relay client): use viewstream or activity, and session.status for the status"),
                ));
            }
        }
        let Some(c) = self.conns.get_mut(&conn) else {
            return Err(RpcError::new(INTERNAL_ERROR, "gone", "connection is gone"));
        };
        let mut initial: Vec<String> = Vec::new();
        if method == "watch.subscribe" {
            if topics.is_empty() {
                return Err(RpcError::params("'topics' must not be empty"));
            }
            for t in &topics {
                match t.as_str() {
                    "tick" => {
                        c.subs.tick = true;
                        c.subs.tick_seen = Some(tick_key(target));
                        initial.push(tick_note(target));
                    }
                    "history" => {
                        c.subs.history = true;
                        c.subs.hist_seen = Some(history_signature(target.doc));
                        initial.push(history_note(target.doc));
                    }
                    "events" => c.subs.events = true,
                    "notes" => c.subs.notes = true,
                    "proposals" => {
                        c.subs.proposals = true;
                        if self.prop_watch.is_none() {
                            self.prop_watch = Some(ProposalWatch::capture(target.doc));
                        }
                        initial.push(notification("watch.proposals", ProposalWatch::state_params(target.doc)));
                    }
                    "activity" => c.subs.activity = Some(ActivitySub { seen: last_seq, reads: include_reads }),
                    "frames" => {
                        if !c.binary {
                            return Err(RpcError::params("frames are binary messages: connect with WebSocket, not plain TCP"));
                        }
                        c.subs.frames = Some(FrameSub { min_interval: Duration::from_micros(1_000_000 / max_fps), last_sent: None, last_key: None, source, client_reset_pending: false, client_event_floor: None, client_clear_event_floor_after_frame: false });
                    }
                    "viewstream" if client_mode.is_some() => {
                        let Some(schema) = client_mode.as_ref().and_then(|h| h.lock().schema()) else {
                            return Err(RpcError::state("not_ready", "the client is still joining the room (no schema yet)"));
                        };
                        // Frames wait for their subscriber's rate cap but are never skipped from the host's side:
                        // events are sent at once, the newest frame goes out as soon as the cap allows.
                        c.subs.viewstream = Some(FrameSub { min_interval: Duration::from_micros(1_000_000 / max_fps), last_sent: None, last_key: None, source, client_reset_pending: false, client_event_floor: None, client_clear_event_floor_after_frame: false });
                        initial.push(notification("watch.viewstream.schema", schema.to_json()));
                    }
                    "viewstream" => {
                        let Some(hook) = self.cfg.limits.view_stream.as_ref() else {
                            return Err(RpcError::params("this host has no view stream (the game did not configure one)"));
                        };
                        if matches!(source, FrameSource::Proposal(_)) {
                            return Err(RpcError::params("'viewstream' shows the play session or the scene (`source`: sim or view), not a proposal"));
                        }
                        c.subs.viewstream = Some(FrameSub { min_interval: Duration::from_micros(1_000_000 / max_fps), last_sent: None, last_key: None, source, client_reset_pending: false, client_event_floor: None, client_clear_event_floor_after_frame: false });
                        // The schema goes out right after the response, before any frame.
                        initial.push(notification("watch.viewstream.schema", hook.lock().schema().to_json()));
                    }
                    other => return Err(RpcError::params(format!("unknown topic '{other}' (tick, history, events, notes, proposals, activity, frames, viewstream)"))),
                }
            }
            if fenced {
                let subscription = self.next_view_subscription;
                self.next_view_subscription = self
                    .next_view_subscription
                    .checked_add(1)
                    .expect("view subscription exhausted");
                c.subs.delivery = Some(ViewDelivery::new(subscription));
            } else if topics
                .iter()
                .any(|t| matches!(t.as_str(), "frames" | "events" | "notes"))
            {
                c.subs.delivery = None;
            }
        } else if topics.is_empty() {
            c.subs = Subs::default();
        } else {
            if topics
                .iter()
                .any(|t| matches!(t.as_str(), "frames" | "events" | "notes"))
            {
                c.subs.delivery = None;
            }
            for t in &topics {
                match t.as_str() {
                    "tick" => c.subs.tick = false,
                    "history" => c.subs.history = false,
                    "events" => c.subs.events = false,
                    "notes" => c.subs.notes = false,
                    "proposals" => c.subs.proposals = false,
                    "activity" => c.subs.activity = None,
                    "frames" => c.subs.frames = None,
                    "viewstream" => c.subs.viewstream = None,
                    other => return Err(RpcError::params(format!("unknown topic '{other}'"))),
                }
            }
        }
        let active: Vec<&str> = [
            ("tick", c.subs.tick),
            ("history", c.subs.history),
            ("events", c.subs.events),
            ("notes", c.subs.notes),
            ("proposals", c.subs.proposals),
            ("activity", c.subs.activity.is_some()),
            ("frames", c.subs.frames.is_some()),
            ("viewstream", c.subs.viewstream.is_some()),
        ]
        .into_iter()
        .filter(|(_, on)| *on)
        .map(|(n, _)| n)
        .collect();
        let mut result = json!({"topics": active});
        if let Some(d) = &c.subs.delivery {
            result["view_delivery"] = json!(1);
            result["subscription"] = json!(d.subscription.to_string());
            result["cursor"] = json!(d.cursor.to_string());
            result["count"] = json!(d.count.to_string());
        }
        // The current state goes out right after the response.
        for n in initial {
            c.tx.send_text(n);
        }
        Ok(result)
    }

    // ---- publishing ----

    fn observe_view_timelines<G: Game>(&mut self, target: &ErpTarget<'_, G>) {
        for c in self.conns.values_mut() {
            let (Some(d), Some(f)) = (c.subs.delivery.as_mut(), c.subs.frames.as_ref()) else {
                continue;
            };
            d.observe(delivery_identity(target, f.source, self.view_incarnation));
        }
    }

    fn collect_view_notes<G: Game>(&mut self, target: &mut ErpTarget<'_, G>) {
        self.observe_view_timelines(target);
        if !self.conns.values().any(|c| c.subs.notes) {
            return;
        }
        let notes = target
            .play
            .as_mut()
            .map_or_else(Vec::new, |pc| pc.session_mut().take_notes());
        if notes.is_empty() {
            return;
        }
        for c in self.conns.values_mut() {
            let Some(d) = c.subs.delivery.as_mut() else {
                continue;
            };
            d.record_notes(&notes);
            let n = notification(
                "watch.notes",
                json!({"notes": notes.iter().copied().map(note_json).collect::<Vec<_>>(),
                "delivery": d.notification_meta()}),
            );
            if !c.tx.try_send_text(n) {
                d.lost();
            }
        }
        self.notes_out.extend(notes);
    }

    fn publish_delivery_events(&mut self, events: &[(EventKey, Vec<u8>)]) {
        if events.is_empty() || !self.conns.values().any(|c| c.subs.delivery.is_some()) {
            return;
        }
        let oversized = events.len() > MAX_VIEW_BATCH_EVENTS;
        let list: Vec<J> = if oversized {
            Vec::new()
        } else {
            events
                .iter()
                .map(|(k, payload)| {
                    json!({"tick": k.tick, "system": k.system_index,
                "seq": k.seq, "payload": hex_encode(payload)})
                })
                .collect()
        };
        for c in self.conns.values_mut() {
            let Some(d) = c.subs.delivery.as_mut() else {
                continue;
            };
            // A proposal/scene preview is not the live simulation's event source.
            if !d.identity.is_some_and(|identity| identity.0 == 1) {
                continue;
            }
            d.advance(events.len());
            if oversized
                || !c.tx.try_send_text(notification(
                    "watch.events",
                    json!({"events": list, "delivery": d.notification_meta()}),
                ))
            {
                d.lost();
            }
        }
    }

    fn publish_delivery_frames<G: Game>(&mut self, target: &ErpTarget<'_, G>, now: Instant) {
        for c in self.conns.values_mut() {
            let (Some(d), Some(f)) = (c.subs.delivery.as_mut(), c.subs.frames.as_mut()) else {
                continue;
            };
            let key = frame_key(target, f.source);
            let cut = (key, d.cursor, d.loss_generation, d.timeline);
            if d.last_cut == Some(cut) {
                continue;
            }
            if f.last_sent
                .is_some_and(|sent| now.duration_since(sent) < f.min_interval)
                || c.tx.pending() > MAX_PENDING_BYTES
            {
                self.frame_pending = true;
                continue;
            }
            let sent = if let Some(key) = key {
                let lim = &self.cfg.limits;
                let Some((mut meta, frame)) =
                    frame_parts(target, f.source, key, (lim.tick_rate, lim.player_count))
                else {
                    continue;
                };
                meta["delivery"] = d.frame_meta();
                // Fenced metadata is built from this exact frame and output cut.
                // Never reuse a legacy cache entry carrying an older watermark.
                self.stats.frames_built += 1;
                if c.tx.is_local() {
                    let cost = 4096 + 160 * frame.alive_count() as usize;
                    c.tx.try_send_local_frame(Arc::new(LocalFrame {
                        meta,
                        frame: Arc::new(frame.clone()),
                        cost,
                    }))
                } else {
                    let bytes = Arc::new(encode_frame_message(&meta, &frame.to_bytes()));
                    self.stats.last_frame_bytes = bytes.len() as u64;
                    c.tx.try_send_binary(bytes)
                }
            } else {
                c.tx.try_send_text(notification(
                    "watch.view.inactive",
                    json!({
                        "delivery": d.frame_meta(), "reason": "source_unavailable",
                    }),
                ))
            };
            if sent {
                f.last_sent = Some(now);
                f.last_key = key;
                d.last_cut = Some(cut);
                if key.is_some() {
                    self.stats.frames_sent += 1;
                }
            } else {
                d.lost();
                self.frame_pending = true;
                self.stats.frames_skipped += 1;
            }
        }
    }

    fn publish<G: Game>(&mut self, target: &mut ErpTarget<'_, G>, now: Instant) {
        if self.conns.is_empty() {
            self.events_out.clear();
            self.notes_out.clear();
            return;
        }
        self.collect_view_notes(target);
        // tick: each connection is told when its own last-seen key differs
        if self.conns.values().any(|c| c.subs.tick) {
            let key = tick_key(target);
            let mut note: Option<String> = None;
            for c in self
                .conns
                .values_mut()
                .filter(|c| c.subs.tick && c.subs.tick_seen != Some(key))
            {
                c.subs.tick_seen = Some(key);
                c.tx.send_text(note.get_or_insert_with(|| tick_note(target)).clone());
            }
        }
        // history (checked at most every 25 ms: it walks the whole list)
        if self.conns.values().any(|c| c.subs.history)
            && self
                .last_hist_check
                .is_none_or(|t| now.duration_since(t) >= Duration::from_millis(25))
        {
            self.last_hist_check = Some(now);
            let sig = history_signature(target.doc);
            let mut note: Option<String> = None;
            for c in self
                .conns
                .values_mut()
                .filter(|c| c.subs.history && c.subs.hist_seen != Some(sig))
            {
                c.subs.hist_seen = Some(sig);
                c.tx.send_text(note.get_or_insert_with(|| history_note(target.doc)).clone());
            }
        }
        // proposals: checked after each request that can change them (see `request`), and every 25 ms
        // here for changes made outside ERP (the editor UI); nothing is tracked while nobody listens
        if self.conns.values().any(|c| c.subs.proposals) {
            if self
                .last_prop_check
                .is_none_or(|t| now.duration_since(t) >= Duration::from_millis(25))
            {
                self.last_prop_check = Some(now);
                self.publish_proposals(target);
            }
        } else {
            self.prop_watch = None;
        }
        // activity: entries recorded since each subscriber was last sent one
        if self.conns.values().any(|c| c.subs.activity.is_some()) {
            let last = self.next_seq - 1;
            for c in self.conns.values_mut() {
                let Some(sub) = c.subs.activity.as_mut() else {
                    continue;
                };
                if sub.seen >= last {
                    continue;
                }
                let start = self.activity.partition_point(|e| e.seq <= sub.seen);
                let list: Vec<J> = self
                    .activity
                    .iter()
                    .skip(start)
                    .filter(|e| sub.reads || !e.read)
                    .map(ActivityEntry::to_json)
                    .collect();
                sub.seen = last;
                if !list.is_empty() {
                    c.tx.send_text(notification("watch.activity", json!({"entries": list})));
                }
            }
        }
        // client mode: the session's frames and events are the view stream
        if self.cfg.limits.client_session.is_some() {
            self.events_out.clear();
            self.publish_client_stream(now);
            return;
        }
        // sim events
        self.publish_stream_events();
        if !self.events_out.is_empty() {
            let list: Vec<J> = self
                .events_out
                .drain(..)
                .map(|(k, payload)| json!({"tick": k.tick, "system": k.system_index, "seq": k.seq, "payload": hex_encode(&payload)}))
                .collect();
            let n = notification("watch.events", json!({"events": list}));
            for c in self
                .conns
                .values()
                .filter(|c| c.subs.events && c.subs.delivery.is_none())
            {
                c.tx.send_text(n.clone());
            }
        }
        // play notes
        let notes = std::mem::take(&mut self.notes_out);
        if !notes.is_empty() {
            let list: Vec<J> = notes.into_iter().map(note_json).collect();
            let n = notification("watch.notes", json!({"notes": list}));
            for c in self
                .conns
                .values()
                .filter(|c| c.subs.notes && c.subs.delivery.is_none())
            {
                c.tx.send_text(n.clone());
            }
        }
        self.publish_delivery_frames(target, now);
        self.publish_frames(target, now);
        self.publish_viewstream(target, now);
    }

    /// Client mode: pumps the session and sends event batches to subscribers
    /// that have received the current reset baseline, and its newest frame to
    /// those whose rate cap allows it.
    fn publish_client_stream(&mut self, now: Instant) {
        let Some(hook) = self.cfg.limits.client_session.clone() else {
            return;
        };
        if !self.conns.values().any(|c| c.subs.viewstream.is_some()) {
            return;
        }
        let out = hook.lock().pump();
        if let Some(frame) = out.frame {
            let seq = self.client_frame.as_ref().map_or(1, |(s, _)| s + 1);
            self.client_frame = Some((seq, Arc::new(frame)));
        }

        let current = self.client_frame.clone();
        if let Some((seq, bytes)) = &current {
            let key: FrameKey = (4, *seq, 0, 0);
            // A connection may subscribe while a reset frame is still cached.
            // Observe it before forwarding this pump's events, even if a later
            // pump replaces that frame before this subscriber is due.
            for c in self.conns.values_mut() {
                if let Some(f) = c.subs.viewstream.as_mut() {
                    f.observe_client_frame(key, bytes);
                }
            }
        }

        if let Some(events) = out.events {
            let bytes = Arc::new(events);
            for c in self.conns.values_mut() {
                let Some(f) = c.subs.viewstream.as_ref() else {
                    continue;
                };
                let Some(delivery) = f.client_events_for_send(&bytes) else {
                    continue;
                };
                // Events queued before the recovery cut may already be in this
                // reliable transport's prefix. The reset baseline follows
                // them in order. Pending-cut batches are dropped; delayed
                // records at/below the delivered baseline tick are filtered.
                send_stream(c, &delivery);
            }
        }

        let Some((seq, bytes)) = current else { return };
        let key: FrameKey = (4, seq, 0, 0);
        for c in self.conns.values_mut() {
            let Some(f) = c.subs.viewstream.as_mut() else {
                continue;
            };
            if f.last_key == Some(key) {
                continue;
            }
            if f.last_sent
                .is_some_and(|t| now.duration_since(t) < f.min_interval)
            {
                self.frame_pending = true;
                continue;
            }
            if c.tx.pending() > MAX_PENDING_BYTES {
                self.stats.frames_skipped += 1;
                continue;
            }
            let Some(delivery) = f.client_frame_for_send(&bytes) else {
                self.stats.frames_skipped += 1;
                continue;
            };
            f.last_sent = Some(now);
            f.last_key = Some(key);
            send_stream(c, &delivery);
            self.stats.frames_sent += 1;
        }
    }

    /// Sends the sim events of the last ticks to the view stream subscribers,
    /// as one event batch message. A play session has no rollback, so every
    /// event is already verified.
    fn publish_stream_events(&mut self) {
        if self.events_out.is_empty() || !self.conns.values().any(|c| c.subs.viewstream.is_some()) {
            return;
        }
        let Some(hook) = self.cfg.limits.view_stream.clone() else {
            return;
        };
        let events: Vec<orr_viewstream::EventRecord> = {
            let producer = hook.lock();
            self.events_out
                .iter()
                .map(|(k, payload)| orr_viewstream::EventRecord {
                    tick: k.tick,
                    system: u32::from(k.system_index),
                    seq: k.seq,
                    state: orr_viewstream::STATE_VERIFIED,
                    event_type: producer.event_type(payload),
                    payload: payload.clone(),
                })
                .collect()
        };
        let bytes = Arc::new(orr_viewstream::EventBatch { events }.encode());
        for c in self.conns.values().filter(|c| c.subs.viewstream.is_some()) {
            send_stream(c, &bytes);
        }
    }

    fn publish_viewstream<G: Game>(&mut self, target: &mut ErpTarget<'_, G>, now: Instant) {
        let Some(hook) = self.cfg.limits.view_stream.clone() else {
            return;
        };
        let mut due: Vec<(u64, FrameKey)> = Vec::new();
        for (id, c) in &self.conns {
            let Some(f) = c.subs.viewstream.as_ref() else {
                continue;
            };
            let throttled = f
                .last_sent
                .is_some_and(|t| now.duration_since(t) < f.min_interval);
            let Some(key) = frame_key(target, f.source) else {
                continue;
            };
            if f.last_key != Some(key) {
                if throttled {
                    self.frame_pending = true;
                } else {
                    due.push((*id, key));
                }
            }
        }
        for (id, key) in due {
            let Some(c) = self.conns.get(&id) else {
                continue;
            };
            if c.tx.pending() > MAX_PENDING_BYTES {
                self.stats.frames_skipped += 1;
                continue;
            }
            let source = c
                .subs
                .viewstream
                .as_ref()
                .map_or(FrameSource::Sim, |f| f.source);
            let pos = match self.frame_cache.iter().position(|(k, _)| *k == key) {
                Some(p) => p,
                None => {
                    if self.frame_cache.len() >= 4 {
                        self.frame_cache.remove(0);
                    }
                    self.frame_cache.push((key, BuiltFrame::default()));
                    self.frame_cache.len() - 1
                }
            };
            if self.frame_cache[pos].1.stream.is_none() {
                let bytes = build_stream_frame(target, &hook, source, key, &mut self.vs_last);
                self.frame_cache[pos].1.stream = bytes.map(Arc::new);
            }
            let Some(bytes) = self.frame_cache[pos].1.stream.clone() else {
                continue;
            };
            let Some(c) = self.conns.get_mut(&id) else {
                continue;
            };
            send_stream(c, &bytes);
            if let Some(f) = c.subs.viewstream.as_mut() {
                f.last_sent = Some(now);
                f.last_key = Some(key);
            }
            self.stats.frames_sent += 1;
        }
    }

    fn publish_frames<G: Game>(&mut self, target: &mut ErpTarget<'_, G>, now: Instant) {
        // Who is due, and with which frame.
        let mut due: Vec<(u64, FrameKey)> = Vec::new();
        for (id, c) in &self.conns {
            if c.subs.delivery.is_some() {
                continue;
            }
            let Some(f) = c.subs.frames.as_ref() else {
                continue;
            };
            let throttled = f
                .last_sent
                .is_some_and(|t| now.duration_since(t) < f.min_interval);
            let Some(key) = frame_key(target, f.source) else {
                continue;
            };
            if f.last_key != Some(key) {
                if throttled {
                    self.frame_pending = true;
                } else {
                    due.push((*id, key));
                }
            }
        }
        if due.is_empty() {
            return;
        }
        for (id, key) in due {
            let Some(c) = self.conns.get(&id) else {
                continue;
            };
            let local = c.tx.is_local();
            let source = c
                .subs
                .frames
                .as_ref()
                .map_or(FrameSource::Sim, |f| f.source);
            if c.tx.pending() > MAX_PENDING_BYTES {
                self.stats.frames_skipped += 1;
                continue;
            }
            // Build it once per key and kind of subscriber.
            let pos = match self.frame_cache.iter().position(|(k, _)| *k == key) {
                Some(p) => p,
                None => {
                    if self.frame_cache.len() >= 4 {
                        self.frame_cache.remove(0);
                    }
                    self.frame_cache.push((key, BuiltFrame::default()));
                    self.frame_cache.len() - 1
                }
            };
            let built = &mut self.frame_cache[pos].1;
            let missing = if local {
                built.local.is_none()
            } else {
                built.wire.is_none()
            };
            if missing {
                let lim = &self.cfg.limits;
                let Some((meta, frame)) =
                    frame_parts(target, source, key, (lim.tick_rate, lim.player_count))
                else {
                    continue;
                };
                if local {
                    let cost = 4096 + 160 * frame.alive_count() as usize;
                    built.local = Some(Arc::new(LocalFrame {
                        meta,
                        frame: Arc::new(frame.clone()),
                        cost,
                    }));
                } else {
                    let m = Arc::new(encode_frame_message(&meta, &frame.to_bytes()));
                    self.stats.last_frame_bytes = m.len() as u64;
                    built.wire = Some(m);
                }
                self.stats.frames_built += 1;
            }
            let Some(c) = self.conns.get_mut(&id) else {
                continue;
            };
            let Some(f) = c.subs.frames.as_mut() else {
                continue;
            };
            if local {
                if let Some(lf) = &built.local {
                    c.tx.send_local_frame(lf.clone());
                }
            } else if let Some(m) = &built.wire {
                c.tx.send_binary(m.clone());
            }
            f.last_sent = Some(now);
            f.last_key = Some(key);
            self.stats.frames_sent += 1;
        }
    }
}

/// Sends a view stream message to one connection: a binary message, or, on a
/// plain TCP connection, a `watch.viewstream` notification with the bytes in hex.
fn send_stream(c: &Conn, bytes: &Arc<Vec<u8>>) {
    if c.binary {
        c.tx.send_stream(bytes.clone());
    } else {
        c.tx.send_text(notification(
            "watch.viewstream",
            json!({"encoding": "hex", "data": hex_encode(bytes)}),
        ));
    }
}

#[derive(Clone, Copy)]
struct ClientFrameInfo {
    tick: u64,
    events_reset: bool,
    discontinuity: bool,
}

/// The tick and recovery flags of an encoded client-mode frame. Client mode
/// currently produces 2D or 3D viewstream frames.
fn client_frame_info(bytes: &[u8]) -> Option<ClientFrameInfo> {
    let kind = orr_viewstream::message_type(bytes).ok()?;
    if !matches!(
        kind,
        orr_viewstream::MSG_FRAME | orr_viewstream::MSG_FRAME3D
    ) {
        return None;
    }
    let flags = *bytes.get(7)?;
    let tick = u64::from_le_bytes(bytes.get(8..16)?.try_into().ok()?);
    Some(ClientFrameInfo {
        tick,
        events_reset: flags & orr_viewstream::FLAG_EVENTS_RESET != 0,
        discontinuity: flags & orr_viewstream::FLAG_DISCONTINUITY != 0,
    })
}

/// The view stream frame message for `key`: the frame, the one before it
/// (none after a jump) and the flags.
fn build_stream_frame<G: Game>(
    t: &ErpTarget<'_, G>,
    hook: &crate::viewstream::ViewStreamHook,
    source: FrameSource,
    key: FrameKey,
    last: &mut Option<(u8, u64)>,
) -> Option<Vec<u8>> {
    use orr_viewstream::{FrameMeta, FLAG_DISCONTINUITY, FLAG_PAUSED};
    let mut meta = FrameMeta::default();
    // What the frame is built from; a change of it is a jump of the timeline.
    let origin = (key.0, if key.0 == 1 { key.2 } else { key.1 });
    if *last != Some(origin) {
        meta.flags |= FLAG_DISCONTINUITY;
    }
    *last = Some(origin);
    let (cur, prev): (&orr_ecs::Frame, Option<&orr_ecs::Frame>) = match (key.0, t.play.as_ref()) {
        (1, Some(pc)) => {
            let s = pc.session();
            meta.tick = s.head_tick();
            meta.verified_tick = meta.tick;
            if !s.is_playing() {
                meta.flags |= FLAG_PAUSED;
            }
            let prev = if meta.flags & FLAG_DISCONTINUITY == 0 && meta.tick > 0 {
                s.frame_at(meta.tick - 1)
            } else {
                None
            };
            (s.frame(), prev)
        }
        (2, _) if matches!(source, FrameSource::View) => {
            meta.flags |= FLAG_PAUSED;
            (t.doc.frame(), None)
        }
        _ => return None,
    };
    Some(hook.lock().encode_frame(cur, prev, meta))
}

/// The `source` parameter of `watch.subscribe`.
fn frame_source(params: &J) -> Result<FrameSource, RpcError> {
    let bad = || RpcError::params("'source' must be `sim`, `view` or `proposal:p<N>`");
    match params.get("source") {
        None | Some(J::Null) => Ok(FrameSource::Sim),
        Some(J::String(s)) => match s.as_str() {
            "sim" => Ok(FrameSource::Sim),
            "view" => Ok(FrameSource::View),
            other => {
                let id = other.strip_prefix("proposal:").ok_or_else(bad)?;
                id.strip_prefix('p')
                    .unwrap_or(id)
                    .parse::<u64>()
                    .map(FrameSource::Proposal)
                    .map_err(|_| bad())
            }
        },
        Some(_) => Err(bad()),
    }
}

/// The key of the frame a subscription would get now, `None` if it gets none.
fn delivery_identity<G: Game>(
    t: &ErpTarget<'_, G>,
    source: FrameSource,
    incarnation: u64,
) -> FrameKey {
    match source {
        FrameSource::Sim | FrameSource::View if t.play.is_some() => {
            let s = t.play.as_ref().expect("play source").session();
            (1, incarnation, s.epoch(), u64::from(s.branch_count()))
        }
        FrameSource::View => (2, incarnation, t.doc.revision(), 0),
        FrameSource::Proposal(id) if t.play.is_none() => t
            .doc
            .proposal_preview(orr_edit::ProposalId(id))
            .ok()
            .map_or((0, incarnation, id, 0), |p| {
                (3, incarnation, id, p.checksum())
            }),
        FrameSource::Sim => (0, incarnation, 0, 0),
        FrameSource::Proposal(id) => (0, incarnation, id, 0),
    }
}

/// The key of the frame a subscription would get now, `None` if it gets none.
fn frame_key<G: Game>(t: &ErpTarget<'_, G>, source: FrameSource) -> Option<FrameKey> {
    let play = || {
        t.play.as_ref().map(|pc| {
            let s = pc.session();
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for v in [
                u64::from(s.is_playing()),
                u64::from(s.speed().permille()),
                s.last_tick(),
                u64::from(s.branch_count()),
            ] {
                h = (h ^ v).wrapping_mul(0x0100_0000_01b3);
            }
            (1u8, s.head_tick(), s.epoch(), h)
        })
    };
    match source {
        FrameSource::Sim => play(),
        FrameSource::View => play().or(Some((2, t.doc.revision(), 0, 0))),
        FrameSource::Proposal(n) => {
            if t.play.is_some() {
                return None;
            }
            t.doc
                .proposal_preview(orr_edit::ProposalId(n))
                .ok()
                .map(|v| (3, n, v.checksum(), 0))
        }
    }
}

fn current_view_state<G: Game>(target: &ErpTarget<'_, G>) -> crate::screenshot::ViewState {
    match target.play.as_ref() {
        Some(pc) => {
            let session = pc.session();
            crate::screenshot::ViewState {
                mode: crate::screenshot::ViewMode::Play,
                paused: !session.is_playing(),
                tick: session.head_tick(),
                epoch: session.epoch(),
                checksum: session.frame().checksum(),
            }
        }
        None => crate::screenshot::ViewState {
            mode: crate::screenshot::ViewMode::Edit,
            paused: true,
            tick: 0,
            epoch: 0,
            checksum: target.doc.checksum(),
        },
    }
}

fn parse_screenshot_options(params: &J) -> Result<(crate::screenshot::ScreenshotOptions, Duration), RpcError> {
    let object = params.as_object().ok_or_else(|| RpcError::params("params must be an object"))?;
    for key in object.keys() {
        if !matches!(key.as_str(), "target" | "timeout_ms" | "max_width" | "max_height") {
            return Err(RpcError::params(format!("unsupported screenshot parameter '{key}'")));
        }
    }
    if let Some(target) = object.get("target") {
        if target.as_str() != Some("app_framebuffer") {
            return Err(RpcError::params("'target' must be 'app_framebuffer'"));
        }
    }
    let number = |key: &str, default: u64| -> Result<u64, RpcError> {
        match object.get(key) {
            None => Ok(default),
            Some(value) => value.as_u64().ok_or_else(|| RpcError::params(format!("'{key}' must be an unsigned integer"))),
        }
    };
    let timeout_ms = number("timeout_ms", 5000)?;
    if !(50..=5000).contains(&timeout_ms) {
        return Err(RpcError::params("'timeout_ms' must be in 50..=5000"));
    }
    let max_width = number("max_width", 2048)?;
    let max_height = number("max_height", 2048)?;
    if max_width == 0 || max_height == 0 || max_width > 2048 || max_height > 2048 {
        return Err(RpcError::params("'max_width' and 'max_height' must be in 1..=2048"));
    }
    Ok((
        crate::screenshot::ScreenshotOptions { max_width: max_width as u32, max_height: max_height as u32 },
        Duration::from_millis(timeout_ms),
    ))
}

fn view_unavailable() -> RpcError {
    RpcError::new(INVALID_STATE, "view_unavailable", "no local editor screenshot owner is available")
}

fn view_busy() -> RpcError {
    RpcError::new(LIMIT_EXCEEDED, "view_busy", "the editor screenshot endpoint is busy")
}

fn view_timeout() -> RpcError {
    RpcError::new(INVALID_STATE, "view_timeout", "the editor screenshot request timed out")
}

fn view_stale() -> RpcError {
    RpcError::new(INVALID_STATE, "view_stale", "the view changed before its screenshot completed")
}

fn view_capture_failed() -> RpcError {
    RpcError::new(INTERNAL_ERROR, "view_capture_failed", "the editor could not capture or encode its framebuffer")
}

/// The metadata and the frame of `key` (as [`frame_key`] named it).
fn frame_parts<'a, G: Game>(
    t: &'a ErpTarget<'_, G>,
    source: FrameSource,
    key: FrameKey,
    defaults: (u32, u8),
) -> Option<(J, &'a orr_ecs::Frame)> {
    let sent_at_us = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_micros() as u64);
    match (key.0, t.play.as_ref()) {
        (1, Some(pc)) => {
            let s = pc.session();
            let meta = json!({
                "tick": key.1,
                "epoch": key.2,
                "tick_rate": s.tick_rate(),
                "player_count": s.player_count(),
                "sent_at_us": sent_at_us,
                "mode": "play",
                "timeline": timeline_to_json(&pc.timeline()),
            });
            Some((meta, s.frame()))
        }
        (2, _) => {
            let meta = json!({"tick": 0, "epoch": key.1, "tick_rate": defaults.0, "player_count": defaults.1, "sent_at_us": sent_at_us, "mode": "edit", "timeline": J::Null});
            Some((meta, t.doc.frame()))
        }
        (3, _) => {
            let FrameSource::Proposal(n) = source else {
                return None;
            };
            let view = t.doc.proposal_preview(orr_edit::ProposalId(n)).ok()?;
            let meta = json!({"tick": 0, "epoch": key.2, "tick_rate": defaults.0, "player_count": defaults.1, "sent_at_us": sent_at_us, "mode": "edit", "proposal": format!("p{n}"), "timeline": J::Null});
            Some((meta, view.frame()))
        }
        _ => None,
    }
}

impl Drop for ErpServer {
    fn drop(&mut self) {
        if let (Some(service), Some(pending)) = (&self.cfg.screenshot, self.pending_screenshot.take()) {
            service.cancel(pending.request.serial);
        }
        if let Some(task) = self.verify_task.take() {
            task.cancel.store(true, Ordering::Relaxed);
            // Dropping JoinHandle detaches; shutdown never waits for a hook,
            // decoder or simulation tick that has not returned yet.
        }
        if let Some(rt) = self.rt.take() {
            rt.shutdown_background();
        }
    }
}

/// Methods after which the list, size or staleness of the proposals may differ.
fn proposals_may_change(method: &str) -> bool {
    matches!(
        method,
        "proposal.begin"
            | "proposal.apply"
            | "proposal.accept"
            | "proposal.accept_verified"
            | "proposal.reject"
            | "scene.load"
            | "tx.commit"
            | "tx.rollback"
            | "history.undo"
            | "history.redo"
    ) || (method.starts_with("world.")
        && !matches!(method, "world.query" | "world.get" | "world.singleton.get"))
}

fn tick_key<G: Game>(t: &ErpTarget<'_, G>) -> TickKey {
    match t.play.as_ref() {
        Some(pc) => (
            true,
            pc.session().head_tick(),
            pc.session().epoch(),
            pc.session().is_playing(),
        ),
        None => (false, 0, 0, false),
    }
}

/// A `watch.tick` notification.
fn tick_note<G: Game>(t: &ErpTarget<'_, G>) -> String {
    let p = match t.play.as_ref() {
        Some(pc) => {
            let s = pc.session();
            json!({
                "mode": "play",
                "tick": s.head_tick(),
                "last_tick": s.last_tick(),
                "playing": s.is_playing(),
                "epoch": s.epoch(),
                "checksum": checksum_text(s.frame().checksum()),
            })
        }
        None => json!({
            "mode": "edit",
            "tick": 0,
            "last_tick": 0,
            "playing": false,
            "epoch": 0,
            "checksum": checksum_text(t.doc.checksum()),
        }),
    };
    notification("watch.tick", p)
}

fn history_signature(doc: &EditorDoc) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |v: u64| {
        h ^= v;
        h = h.wrapping_mul(0x0100_0000_01b3);
    };
    for e in doc.history() {
        mix(e.id);
        mix(u64::from(e.undone));
    }
    mix(u64::from(doc.in_tx()));
    mix(u64::from(doc.is_dirty()));
    h
}

fn history_note(doc: &EditorDoc) -> String {
    let history = doc.history();
    let last = history.last().map(|e| {
        json!({"id": e.id, "label": e.label, "origin": e.origin.to_string(), "op_count": e.op_count, "undone": e.undone})
    });
    notification(
        "watch.history",
        json!({
            "len": history.len(),
            "can_undo": doc.can_undo(),
            "can_redo": doc.can_redo(),
            "dirty": doc.is_dirty(),
            "in_tx": doc.in_tx(),
            "last": last,
        }),
    )
}

fn note_json(n: PlayNote) -> J {
    match n {
        PlayNote::Seeked { from, to } => json!({"kind": "seeked", "from": from, "to": to}),
        PlayNote::Branched { tick, dropped } => {
            json!({"kind": "branched", "tick": tick, "dropped": dropped})
        }
        PlayNote::Paused { tick } => json!({"kind": "paused", "tick": tick}),
        PlayNote::Resumed { tick } => json!({"kind": "resumed", "tick": tick}),
        PlayNote::DebugRejected(e) => {
            json!({"kind": "debug_rejected", "error": debug_error_name(e)})
        }
        PlayNote::SeekRejected { target } => json!({"kind": "seek_rejected", "target": target}),
    }
}

#[cfg(test)]
mod fenced_delivery_tests {
    use super::*;
    use crate::link::Incoming;
    use orr_edit::PlayController;
    use orr_reflect::TypeRegistry;
    use orr_sample::physics_game::{register_reflect, PhysGame};
    use orr_sim::{DebugCommand, Simulation};
    use std::sync::atomic::Ordering::Relaxed;

    fn fixture() -> (
        ErpServer,
        EditorDoc,
        Option<PlayController<PhysGame>>,
        Receiver<Incoming>,
        Arc<AtomicUsize>,
    ) {
        let mut cfg = ServerConfig::new(Auth::DevNoAuth);
        cfg.listen = false;
        let mut server = ErpServer::start(cfg).unwrap();
        let (send, recv) = channel();
        let pending = Arc::new(AtomicUsize::new(0));
        server.conns.insert(
            1,
            Conn {
                tx: ConnTx::local(send, pending.clone()),
                client: "test".into(),
                caps: Caps::ALL,
                binary: true,
                subs: Subs::default(),
                requests: 0,
                connected_ms: 0,
            },
        );
        let mut types = TypeRegistry::new();
        register_reflect(&mut types);
        let text = include_str!("../../../scenes/physics_demo.scene.yaml");
        let doc =
            EditorDoc::from_yaml(text, types, Simulation::<PhysGame>::build_registry(), 7).unwrap();
        (server, doc, None, recv, pending)
    }

    fn subscribe(server: &mut ErpServer, target: &mut ErpTarget<'_, PhysGame>, source: &str) -> J {
        server.watch(target, 1, Caps::ALL, "watch.subscribe", &json!({
            "topics": ["frames", "events", "notes"], "source": source, "max_fps": 1, "view_delivery": 1,
        })).unwrap()
    }

    fn drain(recv: &Receiver<Incoming>, pending: &AtomicUsize) -> Vec<Incoming> {
        recv.try_iter()
            .inspect(|msg| {
                let cost = match msg {
                    Incoming::Text(t) => t.len(),
                    Incoming::Wire(b) => b.len(),
                    Incoming::Local(f) => f.cost,
                };
                pending.fetch_sub(cost, Relaxed);
            })
            .collect()
    }

    fn frame(messages: &[Incoming]) -> &LocalFrame {
        messages
            .iter()
            .find_map(|m| match m {
                Incoming::Local(f) => Some(f.as_ref()),
                _ => None,
            })
            .expect("frame")
    }

    #[test]
    fn request_permits_follow_inbox_stash_dispatch_and_server_drop() {
        use crate::link::{Request, Transport};
        let (mut server, mut doc, mut play, _recv, _pending) = fixture();
        let connector = server.connector();
        // Unique connection ID: fixture already installed connection 1.
        server.net.next_id.store(2, Relaxed);
        let mut local = connector.connect("local", Caps::ALL).unwrap();
        let mut target = ErpTarget { doc: &mut doc, play: &mut play };
        server.poll(&mut target);
        let queued = server.net.queued.clone();
        let bytes = server.net.queued_bytes.clone();
        let req = Request { id: Some(1), method: "sim.state".into(), params: J::Null };
        let cost = req.to_text().len();
        local.send(req.clone()).unwrap();
        assert_eq!(queued.load(Relaxed), 1);
        assert_eq!(bytes.load(Relaxed), cost);
        server.wait_for_request(Duration::ZERO);
        assert!(matches!(server.stash, Some(Inbound::Request { .. })));
        assert_eq!(queued.load(Relaxed), 1, "stash still owns the permit");
        assert_eq!(bytes.load(Relaxed), cost);
        assert_eq!(server.poll(&mut target).requests, 1);
        assert_eq!(queued.load(Relaxed), 0, "undrained response owns no request slot");
        assert_eq!(bytes.load(Relaxed), 0);

        local.send(req.clone()).unwrap();
        server.wait_for_request(Duration::ZERO);
        local.send(req).unwrap();
        assert_eq!(queued.load(Relaxed), 2, "one in stash, one in inbox");
        assert_eq!(bytes.load(Relaxed), cost * 2);
        drop(local);
        assert_eq!(queued.load(Relaxed), 2, "disconnect does not retract admitted requests");
        assert_eq!(bytes.load(Relaxed), cost * 2);
        drop(server);
        assert_eq!(queued.load(Relaxed), 0, "both receiver and stash release slots");
        assert_eq!(bytes.load(Relaxed), 0);
        assert!(connector.connect("after-drop", Caps::ALL).is_err());
        // Only this observer and the connector's shared state remain.
        assert_eq!(Arc::strong_count(&queued), 2, "no permit or sender ownership cycle");
        assert_eq!(Arc::strong_count(&bytes), 2);
    }

    #[test]
    fn coherent_subscription_requires_all_topics_and_acknowledges_version() {
        let (mut server, mut doc, mut play, _recv, _pending) = fixture();
        let mut target = ErpTarget {
            doc: &mut doc,
            play: &mut play,
        };
        for params in [
            json!({"topics":["frames"], "view_delivery":1}),
            json!({"topics":["frames","events","notes"], "view_delivery":2}),
        ] {
            assert!(server
                .watch(&mut target, 1, Caps::ALL, "watch.subscribe", &params)
                .is_err());
        }
        let result = subscribe(&mut server, &mut target, "view");
        assert_eq!(result["view_delivery"], 1);
        assert_eq!(result["subscription"], "1");
        assert_eq!(result["cursor"], "0");
        assert_eq!(result["count"], "0");
        let again = subscribe(&mut server, &mut target, "view");
        assert_eq!(again["subscription"], "2");
    }

    #[test]
    fn same_tick_rejected_note_forces_new_fence_after_fps_throttle() {
        let (mut server, mut doc, mut play, recv, pending) = fixture();
        let mut target = ErpTarget {
            doc: &mut doc,
            play: &mut play,
        };
        server.request(&mut target, 1, Some(json!(1)), "sim.start", &J::Null);
        subscribe(&mut server, &mut target, "sim");
        let now = Instant::now();
        server.publish(&mut target, now);
        let first = drain(&recv, &pending);
        let old_meta = frame(&first).meta.clone();
        let old_checksum = frame(&first).frame.checksum();
        assert_eq!(old_meta["delivery"]["through_cursor"], "0");
        target
            .play
            .as_mut()
            .unwrap()
            .session_mut()
            .debug(DebugCommand::Despawn {
                entity: orr_ecs::Entity {
                    index: u32::MAX,
                    version: 0,
                },
            })
            .unwrap_err();
        server.publish(&mut target, now + Duration::from_millis(10));
        let notes = drain(&recv, &pending);
        assert!(!notes.iter().any(|m| matches!(m, Incoming::Local(_))));
        server.publish(&mut target, now + Duration::from_secs(1));
        let next = drain(&recv, &pending);
        let next = frame(&next);
        assert_eq!(next.frame.checksum(), old_checksum);
        assert_eq!(next.meta["delivery"]["through_cursor"], "1");
        assert_eq!(next.meta["delivery"]["count"], "1");
        assert_eq!(
            next.meta["delivery"]["lifecycle"][0]["note"]["kind"],
            "debug_rejected"
        );
    }

    #[test]
    fn successful_paused_debug_rebuilds_payload_and_fence_at_same_tick() {
        let (mut server, mut doc, mut play, recv, pending) = fixture();
        let mut target = ErpTarget {
            doc: &mut doc,
            play: &mut play,
        };
        server.request(&mut target, 1, None, "sim.start", &J::Null);
        subscribe(&mut server, &mut target, "sim");
        let now = Instant::now();
        server.publish(&mut target, now);
        let old = drain(&recv, &pending);
        let old = frame(&old);
        let (tick, checksum, alive, meta) = (
            old.frame.tick(),
            old.frame.checksum(),
            old.frame.alive_count(),
            old.meta.clone(),
        );
        server.request(
            &mut target,
            1,
            Some(json!(77)),
            "sim.debug",
            &json!({"cmd":"spawn", "components":[]}),
        );
        let response = drain(&recv, &pending);
        let Incoming::Text(text) = &response[0] else {
            panic!("debug response")
        };
        assert!(serde_json::from_str::<J>(text)
            .unwrap()
            .get("error")
            .is_none());
        server.publish(&mut target, now + Duration::from_secs(1));
        let new = drain(&recv, &pending);
        let new = frame(&new);
        assert_eq!(new.frame.tick(), tick);
        assert_ne!(new.frame.checksum(), checksum);
        assert_eq!(new.frame.alive_count(), alive + 1);
        assert_ne!(new.meta["epoch"], meta["epoch"]);
        assert_ne!(
            new.meta["delivery"]["timeline"],
            meta["delivery"]["timeline"]
        );
        // No transient event is required to force this publication.
        assert_eq!(new.meta["delivery"]["through_cursor"], "0");
    }

    #[test]
    fn source_disappearance_and_same_tick_restart_have_distinct_timelines() {
        let (mut server, mut doc, mut play, recv, pending) = fixture();
        let mut target = ErpTarget {
            doc: &mut doc,
            play: &mut play,
        };
        subscribe(&mut server, &mut target, "sim");
        let now = Instant::now();
        server.publish(&mut target, now);
        let messages = drain(&recv, &pending);
        let inactive = messages
            .iter()
            .find_map(|m| match m {
                Incoming::Text(s) => serde_json::from_str::<J>(s).ok(),
                _ => None,
            })
            .unwrap();
        assert_eq!(inactive["method"], "watch.view.inactive");
        server.request(&mut target, 1, None, "sim.start", &J::Null);
        server.publish(&mut target, now + Duration::from_secs(1));
        let messages = drain(&recv, &pending);
        let first = frame(&messages).meta.clone();
        server.request(&mut target, 1, None, "sim.stop", &J::Null);
        server.request(&mut target, 1, None, "sim.start", &J::Null);
        server.publish(&mut target, now + Duration::from_secs(2));
        let messages = drain(&recv, &pending);
        let restarted = &frame(&messages).meta;
        assert_eq!(first["tick"], restarted["tick"]);
        assert_eq!(first["epoch"], restarted["epoch"]);
        assert_ne!(
            first["delivery"]["timeline"],
            restarted["delivery"]["timeline"]
        );
        assert_ne!(
            first["delivery"]["loss_generation"],
            restarted["delivery"]["loss_generation"]
        );
    }

    #[test]
    fn transport_drops_are_summarized_but_control_responses_are_preserved() {
        let (mut server, mut doc, mut play, recv, pending) = fixture();
        let mut target = ErpTarget {
            doc: &mut doc,
            play: &mut play,
        };
        server.request(&mut target, 1, None, "sim.start", &J::Null);
        subscribe(&mut server, &mut target, "sim");
        server.publish(&mut target, Instant::now());
        drain(&recv, &pending);
        pending.store(65 << 20, Relaxed);
        target
            .play
            .as_mut()
            .unwrap()
            .session_mut()
            .debug(DebugCommand::Despawn {
                entity: orr_ecs::Entity {
                    index: u32::MAX,
                    version: 0,
                },
            })
            .unwrap_err();
        server.request(&mut target, 1, Some(json!(99)), "sim.state", &J::Null);
        let responses = drain(&recv, &pending);
        assert_eq!(responses.len(), 1);
        let Incoming::Text(response) = &responses[0] else {
            panic!("response")
        };
        assert_eq!(serde_json::from_str::<J>(response).unwrap()["id"], 99);
        pending.store(0, Relaxed);
        server.publish(&mut target, Instant::now() + Duration::from_secs(2));
        let messages = drain(&recv, &pending);
        let delivery = &frame(&messages).meta["delivery"];
        assert_eq!(delivery["loss_generation"], "2");
        assert_eq!(delivery["count"], "1");
        assert_eq!(delivery["lifecycle"][0]["count"], "1");
    }

    #[test]
    fn request_boundaries_preserve_seek_and_branch_identity_before_final_frame() {
        let (mut server, mut doc, mut play, recv, pending) = fixture();
        let mut target = ErpTarget {
            doc: &mut doc,
            play: &mut play,
        };
        server.request(&mut target, 1, None, "sim.start", &J::Null);
        subscribe(&mut server, &mut target, "sim");
        let now = Instant::now();
        server.publish(&mut target, now);
        drain(&recv, &pending);
        server.request(&mut target, 1, None, "sim.step", &json!({"n":10}));
        server.request(&mut target, 1, None, "sim.seek", &json!({"tick":2}));
        let after_seek = server.conns[&1].subs.delivery.as_ref().unwrap().timeline;
        let raw_epoch = target.play.as_ref().unwrap().session().epoch();
        server.request(&mut target, 1, None, "sim.branch", &J::Null);
        let after_branch = server.conns[&1].subs.delivery.as_ref().unwrap().timeline;
        assert_eq!(target.play.as_ref().unwrap().session().epoch(), raw_epoch);
        assert!(
            after_branch > after_seek,
            "branch invalidates delivery even without raw epoch change"
        );
        server.request(&mut target, 1, None, "sim.step", &json!({"n":1}));
        server.publish(&mut target, now + Duration::from_secs(1));
        let messages = drain(&recv, &pending);
        let baseline = frame(&messages);
        assert_eq!(baseline.frame.tick(), 3);
        let summaries = baseline.meta["delivery"]["lifecycle"].as_array().unwrap();
        assert!(summaries.iter().any(|n| n["note"]["kind"] == "seeked"));
        assert!(summaries.iter().any(|n| n["note"]["kind"] == "branched"));
    }

    #[test]
    fn legacy_subscriber_keeps_its_original_metadata_next_to_fenced_peer() {
        let (mut server, mut doc, mut play, recv, pending) = fixture();
        let (legacy_tx, legacy_rx) = channel();
        let legacy_pending = Arc::new(AtomicUsize::new(0));
        server.conns.insert(
            2,
            Conn {
                tx: ConnTx::local(legacy_tx, legacy_pending.clone()),
                client: "legacy".into(),
                caps: Caps::ALL,
                binary: true,
                subs: Subs::default(),
                requests: 0,
                connected_ms: 0,
            },
        );
        let mut target = ErpTarget {
            doc: &mut doc,
            play: &mut play,
        };
        server.request(&mut target, 1, None, "sim.start", &J::Null);
        subscribe(&mut server, &mut target, "sim");
        let ack = server
            .watch(
                &mut target,
                2,
                Caps::ALL,
                "watch.subscribe",
                &json!({"topics":["frames","events","notes"]}),
            )
            .unwrap();
        assert!(ack.get("view_delivery").is_none());
        server.publish(&mut target, Instant::now());
        let fenced = drain(&recv, &pending);
        let legacy = drain(&legacy_rx, &legacy_pending);
        assert!(frame(&fenced).meta.get("delivery").is_some());
        assert!(frame(&legacy).meta.get("delivery").is_none());
        assert_eq!(
            frame(&fenced).frame.checksum(),
            frame(&legacy).frame.checksum()
        );
        // Extensible JSON metadata does not require an ORRS wire-version bump.
        let encoded = encode_frame_message(&frame(&fenced).meta, &frame(&fenced).frame.to_bytes());
        assert_eq!(encoded[4], 1);
        let (meta, bytes) = crate::wire::decode_frame_message(&encoded).unwrap();
        assert_eq!(meta, frame(&fenced).meta);
        assert_eq!(bytes, frame(&fenced).frame.to_bytes());
    }

    #[test]
    fn oversized_outcome_is_counted_and_requires_same_state_reset_frame() {
        let (mut server, mut doc, mut play, recv, pending) = fixture();
        let mut target = ErpTarget {
            doc: &mut doc,
            play: &mut play,
        };
        server.request(&mut target, 1, None, "sim.start", &J::Null);
        subscribe(&mut server, &mut target, "sim");
        let now = Instant::now();
        server.publish(&mut target, now);
        drain(&recv, &pending);
        let events = vec![(EventKey::new(0, 0, 0), Vec::new()); MAX_VIEW_BATCH_EVENTS + 1];
        server.publish_delivery_events(&events);
        assert!(drain(&recv, &pending).is_empty());
        server.publish(&mut target, now + Duration::from_secs(1));
        let messages = drain(&recv, &pending);
        let baseline = frame(&messages);
        assert_eq!(baseline.frame.tick(), 0);
        assert_eq!(baseline.meta["delivery"]["through_cursor"], "1");
        assert_eq!(
            baseline.meta["delivery"]["count"],
            (MAX_VIEW_BATCH_EVENTS + 1).to_string()
        );
        assert_eq!(baseline.meta["delivery"]["loss_generation"], "2");
    }

    #[test]
    fn truncated_live_suffix_does_not_advance_inactive_proposal() {
        let (mut server, mut doc, mut play, _recv, _pending) = fixture();
        let (preview_tx, _preview_rx) = channel();
        server.conns.insert(
            2,
            Conn {
                tx: ConnTx::local(preview_tx, Arc::new(AtomicUsize::new(0))),
                client: "preview".into(),
                caps: Caps::ALL,
                binary: true,
                subs: Subs::default(),
                requests: 0,
                connected_ms: 0,
            },
        );
        let mut target = ErpTarget {
            doc: &mut doc,
            play: &mut play,
        };
        server.request(&mut target, 1, None, "sim.start", &J::Null);
        subscribe(&mut server, &mut target, "sim");
        server
            .watch(
                &mut target,
                2,
                Caps::ALL,
                "watch.subscribe",
                &json!({
                    "topics": ["frames", "events", "notes"], "source": "proposal:p999",
                    "view_delivery": 1,
                }),
            )
            .unwrap();
        // Force the actual real-time producer cap. A proposal stream is
        // unavailable during play and must not inherit this unrelated loss.
        let events = vec![
            SimEvent {
                key: EventKey::new(0, 0, 0),
                payload: orr_sample::physics_game::PhysEvent {
                    kind: 1,
                    a: 0,
                    b: 0
                },
            };
            MAX_VIEW_BATCH_EVENTS + 1
        ];
        server.push_events(&events);
        assert_eq!(server.pending_view_losses, 1);
        server.poll(&mut target);
        let live = server.conns[&1].subs.delivery.as_ref().unwrap();
        assert_eq!(live.count, (MAX_VIEW_BATCH_EVENTS + 1) as u64);
        assert_eq!(live.loss_generation, 2);
        let preview = server.conns[&2].subs.delivery.as_ref().unwrap();
        assert_eq!(preview.identity.unwrap().0, 0);
        assert_eq!(preview.cursor, 0);
        assert_eq!(preview.count, 0);
        assert_eq!(preview.loss_generation, 1);
    }
}

#[cfg(test)]
mod client_viewstream_recovery_tests {
    use super::*;
    use crate::client_mode::{ClientPump, ClientSession, ClientSessionHook, SessionError};
    use crate::link::Incoming;
    use orr_viewstream::{
        EntityRecord, EventBatch, EventRecord, ViewFrame, FLAG_DISCONTINUITY, FLAG_EVENTS_RESET,
        MSG_EVENTS, MSG_FRAME, STATE_VERIFIED,
    };

    struct ScriptedSession(VecDeque<ClientPump>);

    impl ClientSession for ScriptedSession {
        fn pump(&mut self) -> ClientPump {
            self.0.pop_front().unwrap_or_default()
        }

        fn schema(&self) -> Option<orr_viewstream::Schema> {
            None
        }

        fn status(&self) -> J {
            J::Null
        }

        fn confirmed_checksum(&self, _tick: u64) -> Option<(u64, u64)> {
            None
        }

        fn set_input(&mut self, _player: u8, _bytes: &[u8]) -> Result<(), SessionError> {
            Ok(())
        }

        fn send_command(&mut self, _player: u8, _bytes: &[u8]) -> Result<(), SessionError> {
            Ok(())
        }
    }

    fn view_frame(flags: u8, seq: u64) -> Vec<u8> {
        view_frame_at(flags, seq, seq)
    }

    fn view_frame_at(flags: u8, seq: u64, tick: u64) -> Vec<u8> {
        ViewFrame {
            flags,
            tick,
            verified_tick: tick,
            seq,
            rollback: None,
            entities: vec![EntityRecord {
                id: 1,
                kind: 0,
                shape: orr_viewstream::SHAPE_CIRCLE,
                mode: orr_viewstream::MODE_PREDICTION,
                size: 1.0,
                half_y: 0.0,
                rgba: [255; 4],
                prev: [seq as f32, 0.0, 0.0],
                cur: [(seq + 1) as f32, 0.0, 0.0],
            }],
            props: Vec::new(),
        }
        .encode()
    }

    fn event_batch(seq: u32) -> Vec<u8> {
        event_batch_records(&[(u64::from(seq), seq)])
    }

    fn event_batch_records(records: &[(u64, u32)]) -> Vec<u8> {
        EventBatch {
            events: records
                .iter()
                .map(|(tick, seq)| EventRecord {
                    tick: *tick,
                    system: 0,
                    seq: *seq,
                    state: STATE_VERIFIED,
                    event_type: 0,
                    payload: vec![],
                })
                .collect(),
        }
        .encode()
    }

    fn test_server(pumps: impl IntoIterator<Item = ClientPump>) -> (ErpServer, Receiver<Incoming>) {
        let mut cfg = ServerConfig::new(Auth::DevNoAuth);
        cfg.listen = false;
        cfg.limits.client_session = Some(ClientSessionHook::new(ScriptedSession(
            pumps.into_iter().collect(),
        )));
        let mut server = ErpServer::start(cfg).unwrap();
        let (send, recv) = channel::<Incoming>();
        let pending = Arc::new(AtomicUsize::new(0));
        let tx = ConnTx::local(send, pending);
        let subs = Subs {
            viewstream: Some(FrameSub {
                min_interval: Duration::from_secs(1),
                last_sent: None,
                last_key: None,
                source: FrameSource::Sim,
                client_reset_pending: false,
                client_event_floor: None,
                client_clear_event_floor_after_frame: false,
            }),
            ..Subs::default()
        };
        server.conns.insert(
            1,
            Conn {
                tx,
                client: "test".into(),
                caps: Caps::ALL,
                binary: true,
                subs,
                requests: 0,
                connected_ms: 0,
            },
        );
        (server, recv)
    }

    fn take_messages(recv: &Receiver<Incoming>) -> Vec<Vec<u8>> {
        let mut messages = Vec::new();
        while let Ok(incoming) = recv.try_recv() {
            match incoming {
                Incoming::Wire(bytes) => messages.push(bytes),
                _ => panic!("expected only binary viewstream messages"),
            }
        }
        messages
    }

    #[test]
    fn client_reset_survives_rate_skip_and_drops_cut_events_without_replay() {
        let (mut server, recv) = test_server([
            ClientPump {
                frame: Some(view_frame(0, 1)),
                events: Some(event_batch(11)),
            },
            ClientPump {
                frame: Some(view_frame(FLAG_DISCONTINUITY | FLAG_EVENTS_RESET, 2)),
                events: Some(event_batch(22)),
            },
            ClientPump {
                frame: Some(view_frame(0, 3)),
                events: Some(event_batch(33)),
            },
            ClientPump {
                frame: Some(view_frame(0, 4)),
                events: Some(event_batch(44)),
            },
            ClientPump {
                frame: None,
                events: Some(event_batch_records(&[(2, 56), (5, 57)])),
            },
            ClientPump {
                frame: Some(view_frame_at(FLAG_DISCONTINUITY, 5, 1)),
                events: Some(event_batch_records(&[(1, 66)])),
            },
            ClientPump {
                frame: None,
                events: Some(event_batch_records(&[(1, 77)])),
            },
            ClientPump {
                frame: None,
                events: Some(event_batch_records(&[(1, 88)])),
            },
        ]);
        let t0 = Instant::now();

        server.publish_client_stream(t0);
        let before_cut = take_messages(&recv);
        assert_eq!(
            before_cut
                .iter()
                .map(|b| orr_viewstream::message_type(b).unwrap())
                .collect::<Vec<_>>(),
            [MSG_EVENTS, MSG_FRAME]
        );
        assert_eq!(
            EventBatch::decode(&before_cut[0]).unwrap().events[0].seq,
            11
        );

        // The reset pulse and two newer frames arrive inside the one-fps
        // interval. The raw reset frame is replaced, while the subscriber
        // keeps its sticky reset and sees no post-cut events yet.
        server.publish_client_stream(t0 + Duration::from_millis(10));
        assert!(take_messages(&recv).is_empty());
        server.publish_client_stream(t0 + Duration::from_millis(20));
        assert!(take_messages(&recv).is_empty());

        // Events from this pump are still held back: frames are delivered
        // after this pump's event portion, so baseline-before-events ordering
        // is preserved even as the rate-limited frame finally becomes due.
        server.publish_client_stream(t0 + Duration::from_secs(1));
        let baseline_only = take_messages(&recv);
        assert_eq!(baseline_only.len(), 1);
        assert_eq!(
            orr_viewstream::message_type(&baseline_only[0]).unwrap(),
            MSG_FRAME
        );
        let baseline = ViewFrame::decode(&baseline_only[0]).unwrap();
        assert!(baseline.has(FLAG_DISCONTINUITY | FLAG_EVENTS_RESET));
        assert_eq!(baseline.entities[0].prev, baseline.entities[0].cur);
        assert_eq!(
            baseline.seq, 4,
            "the newer cached frame replaces the missed reset pulse"
        );

        // A delayed event at or before the delivered baseline tick is filtered,
        // while a newer event in the same batch remains eligible.
        server.publish_client_stream(t0 + Duration::from_millis(1_010));
        let after_cut = take_messages(&recv);
        assert_eq!(after_cut.len(), 1);
        assert_eq!(
            orr_viewstream::message_type(&after_cut[0]).unwrap(),
            MSG_EVENTS
        );
        assert_eq!(
            EventBatch::decode(&after_cut[0])
                .unwrap()
                .events
                .iter()
                .map(|e| e.seq)
                .collect::<Vec<_>>(),
            [57]
        );

        // A non-reset discontinuity releases the tick floor only after its
        // new-timeline snapshot is sent. Same-pump old events remain filtered.
        server.publish_client_stream(t0 + Duration::from_millis(1_020));
        assert!(take_messages(&recv).is_empty());
        server.publish_client_stream(t0 + Duration::from_secs(2));
        let new_timeline_baseline = take_messages(&recv);
        assert_eq!(new_timeline_baseline.len(), 1);
        assert_eq!(
            orr_viewstream::message_type(&new_timeline_baseline[0]).unwrap(),
            MSG_FRAME
        );
        assert_eq!(
            ViewFrame::decode(&new_timeline_baseline[0]).unwrap().tick,
            1
        );

        // Old-timeline tick numbers are valid again after that boundary.
        server.publish_client_stream(t0 + Duration::from_millis(2_010));
        let after_discontinuity = take_messages(&recv);
        assert_eq!(after_discontinuity.len(), 1);
        assert_eq!(
            orr_viewstream::message_type(&after_discontinuity[0]).unwrap(),
            MSG_EVENTS
        );
        assert_eq!(
            EventBatch::decode(&after_discontinuity[0])
                .unwrap()
                .events
                .iter()
                .map(|e| e.seq)
                .collect::<Vec<_>>(),
            [88]
        );
    }
}
