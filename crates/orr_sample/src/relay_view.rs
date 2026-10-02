//! A relay client session that produces the view stream: the one piece the C ABI
//! (`orr_ffi`, `orr_client_open`) and the ERP host (`orr_remote_host --join`) share.
//!
//! A `RelayClient` runs on the sim thread of a `Threaded` bridge (`orr_bridge::RelayHost`). It
//! predicts the local and the remote players, rolls back when a confirmed input differs and
//! reconciles events as predicted, verified or canceled. The view stream comes from
//! `ViewStreamSource` over that bridge's snapshots, so frames carry the rollback flag and range and
//! events carry their deterministic keys, exactly as `docs/view-stream.md` says. Both users hand out
//! the same bytes because both call [`RelayView::pump`].
//!
//! Joining blocks until the room starts (every player present), so [`RelayView::start`] joins on a
//! thread and the session reports [`ViewState::Connecting`] until it is done; [`RelayView::join`]
//! waits for it.

use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use orr_bridge::{Bridge, RelayMetrics, Threaded};
use orr_sim::{PlayerSlot, SimCommand};
use orr_viewstream::{EventBatch, Schema, ViewStreamSource};

use crate::net_client::{physics_bridge, NetArgs, PHYSICS_BUILD_ID};
use crate::physics_game::{NoCommand, PhysGame, PhysInput};
use crate::physics_host::SimMetrics;
use crate::physics_stream::{phys_client_stream_source, PhysKinds};
use crate::physics_view::PhysExtractor;

type Joined = Result<(Threaded<PhysGame>, crate::physics_game::PhysConfig), String>;

/// How long [`RelayView::join`] waits at most beyond the connect timeout.
pub const WAIT_SLACK: Duration = Duration::from_secs(5);

/// Why a call failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewErrorKind {
    /// The session is still joining.
    NotReady,
    /// The call's arguments are wrong (another player's slot, a wrong size).
    Arg,
    /// The session is failed, disconnected or gone.
    Host,
}

/// A failed call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewError {
    pub kind: ViewErrorKind,
    pub message: String,
}

impl ViewError {
    fn new(kind: ViewErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }
}

impl std::fmt::Display for ViewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Where the session is (the numbers are the C ABI's `ORR_STATE_*`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewState {
    #[default]
    Connecting = 0,
    Playing = 1,
    Disconnected = 2,
    Failed = 3,
}

impl ViewState {
    /// The name used in ERP (`session.status`).
    pub fn name(self) -> &'static str {
        match self {
            ViewState::Connecting => "connecting",
            ViewState::Playing => "playing",
            ViewState::Disconnected => "disconnected",
            ViewState::Failed => "failed",
        }
    }
}

/// What `orr_session_status` and ERP's `session.status` report.
#[derive(Clone, Copy, Debug, Default)]
pub struct ViewStatus {
    pub state: ViewState,
    pub slot: u32,
    pub player_count: u32,
    pub rtt_ms: u32,
    pub input_delay: u32,
    pub desyncs: u64,
    pub head_tick: u64,
    pub verified_tick: u64,
    pub rollbacks: u64,
    pub resim_ticks: u64,
    pub last_rollback_from: u64,
    pub last_rollback_to: u64,
    pub stall_episodes: u64,
    pub stalled_ms: u64,
    pub repeats: u64,
}

/// What the schema needs once the room has started.
pub struct Ready {
    pub schema: Schema,
    pub input_size: usize,
    pub player_count: u8,
}

/// What one [`RelayView::pump`] produced: encoded view stream messages.
#[derive(Debug, Default)]
pub struct Pumped {
    /// The newest frame (`MSG_FRAME` bytes), if the bridge moved since the last pump.
    pub frame: Option<Vec<u8>>,
    /// Event states since the last pump (`MSG_EVENTS` bytes).
    pub events: Option<Vec<u8>>,
}

struct Playing {
    bridge: Threaded<PhysGame>,
    source: ViewStreamSource<PhysExtractor, PhysKinds>,
    slot: u8,
    players: u8,
}

enum Link {
    /// The connection thread is still joining.
    Connecting(Receiver<Joined>),
    Playing(Box<Playing>),
    /// Joining failed (the reason), or the connection thread died.
    Failed(String),
}

/// A client session of a relay room that produces the view stream.
pub struct RelayView {
    link: Link,
    metrics: Arc<RelayMetrics>,
}

impl RelayView {
    /// Starts joining on a thread.
    pub fn start(args: NetArgs) -> Result<RelayView, String> {
        let metrics = RelayMetrics::new();
        let (tx, rx) = channel::<Joined>();
        let (thread_metrics, sim) = (metrics.clone(), SimMetrics::new());
        std::thread::Builder::new()
            .name("orr-relay-join".to_string())
            .spawn(move || {
                // The receiver is gone if the session was closed meanwhile: the bridge then drops here.
                let _ = tx.send(physics_bridge(&args, thread_metrics, sim));
            })
            .map_err(|e| format!("cannot start the connection thread: {e}"))?;
        Ok(RelayView { link: Link::Connecting(rx), metrics })
    }

    /// Joins and waits until the room has started (at most the connect timeout plus [`WAIT_SLACK`]).
    pub fn join(args: NetArgs) -> Result<(RelayView, Ready), String> {
        let timeout = args.connect_timeout + WAIT_SLACK;
        let mut view = RelayView::start(args)?;
        let end = Instant::now() + timeout;
        loop {
            if let Some(ready) = view.poll_join() {
                return Ok((view, ready));
            }
            if let Some(why) = view.failure() {
                return Err(why.to_string());
            }
            if Instant::now() > end {
                return Err("timed out waiting for the room to start".to_string());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Moves a finished join into `Playing` (or `Failed`). Returns what the user must learn when
    /// the session becomes ready.
    pub fn poll_join(&mut self) -> Option<Ready> {
        let Link::Connecting(rx) = &self.link else { return None };
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("the connection thread stopped without a result".to_string()),
        };
        match result {
            Ok((bridge, _scene)) => {
                let (slot, players, tick_rate) = (bridge.local_slot().0, bridge.player_count(), bridge.tick_rate());
                let source = phys_client_stream_source(PHYSICS_BUILD_ID, players, tick_rate, slot);
                let ready = Ready { schema: source.schema().clone(), input_size: std::mem::size_of::<PhysInput>(), player_count: players };
                self.link = Link::Playing(Box::new(Playing { bridge, source, slot, players }));
                Some(ready)
            }
            Err(e) => {
                self.link = Link::Failed(e);
                None
            }
        }
    }

    /// Takes what the bridge produced since the last call, as stream messages.
    pub fn pump(&mut self) -> Pumped {
        let Link::Playing(p) = &mut self.link else { return Pumped::default() };
        let out = p.source.pump(&mut p.bridge);
        Pumped {
            frame: out.frame.map(|f| f.encode()),
            events: (!out.events.is_empty()).then(|| EventBatch { events: out.events }.encode()),
        }
    }

    /// Sets the held input of the local player (the bytes of a `PhysInput`).
    pub fn set_input(&mut self, player: u8, bytes: &[u8]) -> Result<(), ViewError> {
        let down = self.status().state == ViewState::Disconnected;
        let Link::Playing(p) = &mut self.link else { return Err(not_playing(&self.link)) };
        if down {
            return Err(ViewError::new(ViewErrorKind::Host, "disconnected from the server"));
        }
        if player != p.slot {
            return Err(ViewError::new(
                ViewErrorKind::Arg,
                format!("this client plays slot {}: only its own input can be set (got player {player})", p.slot),
            ));
        }
        let input = bytemuck::try_pod_read_unaligned::<PhysInput>(bytes)
            .map_err(|_| ViewError::new(ViewErrorKind::Arg, format!("input must be {} bytes", std::mem::size_of::<PhysInput>())))?;
        p.bridge
            .set_input(PlayerSlot(p.slot), input)
            .map_err(|e| ViewError::new(ViewErrorKind::Host, format!("input refused: {e}")))
    }

    /// Sends a command of the game to the server with the next tick (`PhysGame` has only the no-op one).
    pub fn send_command(&mut self, player: u8, bytes: &[u8]) -> Result<(), ViewError> {
        let Link::Playing(p) = &mut self.link else { return Err(not_playing(&self.link)) };
        if player != p.slot {
            return Err(ViewError::new(
                ViewErrorKind::Arg,
                format!("this client plays slot {}: only its own commands can be sent (got player {player})", p.slot),
            ));
        }
        let command = <NoCommand as SimCommand>::decode(bytes)
            .ok_or_else(|| ViewError::new(ViewErrorKind::Arg, format!("command must be {} bytes", std::mem::size_of::<NoCommand>())))?;
        p.bridge.send_command(command).map_err(|e| ViewError::new(ViewErrorKind::Host, format!("command refused: {e}")))
    }

    /// The checksum of the confirmed state at `tick` (0 = the newest checkpoint): `(tick, checksum)`.
    pub fn confirmed_checksum(&self, tick: u64) -> Option<(u64, u64)> {
        if tick == 0 {
            self.metrics.last_checksum()
        } else {
            self.metrics.checksum_at(tick).map(|c| (tick, c))
        }
    }

    /// Whether the join is still going on.
    pub fn is_joining(&self) -> bool {
        matches!(self.link, Link::Connecting(_))
    }

    /// The slot this client plays (once joined).
    pub fn slot(&self) -> Option<u8> {
        match &self.link {
            Link::Playing(p) => Some(p.slot),
            _ => None,
        }
    }

    pub fn status(&self) -> ViewStatus {
        let m = self.metrics.status();
        let mut s = ViewStatus {
            rtt_ms: m.rtt_ms,
            input_delay: m.delay,
            desyncs: m.desyncs,
            head_tick: m.head_tick,
            verified_tick: m.verified_tick,
            rollbacks: m.rollbacks,
            resim_ticks: m.resim_ticks,
            stall_episodes: m.stall_episodes,
            stalled_ms: m.stalled_ms,
            repeats: m.repeats,
            ..ViewStatus::default()
        };
        match &self.link {
            Link::Connecting(_) => s.state = ViewState::Connecting,
            Link::Failed(_) => s.state = ViewState::Failed,
            Link::Playing(p) => {
                s.slot = u32::from(p.slot);
                s.player_count = u32::from(p.players);
                s.state = if m.connected && p.bridge.is_alive() { ViewState::Playing } else { ViewState::Disconnected };
                if let Some(r) = p.bridge.snapshot().and_then(|snap| snap.last_rollback()) {
                    s.last_rollback_from = r.from_tick;
                    s.last_rollback_to = r.to_tick;
                }
            }
        }
        s
    }

    /// The reason a failed join gave.
    pub fn failure(&self) -> Option<&str> {
        match &self.link {
            Link::Failed(e) => Some(e),
            _ => None,
        }
    }
}

fn not_playing(link: &Link) -> ViewError {
    match link {
        Link::Connecting(_) => ViewError::new(ViewErrorKind::NotReady, "the client is still joining the room (see the session status)"),
        Link::Failed(e) => ViewError::new(ViewErrorKind::Host, format!("joining failed: {e}")),
        Link::Playing(_) => ViewError::new(ViewErrorKind::Host, "the session is gone"),
    }
}
