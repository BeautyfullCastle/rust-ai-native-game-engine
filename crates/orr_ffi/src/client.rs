//! Client mode of the C ABI: the handle plays on an `orr_server` room instead of hosting a
//! simulation of its own.
//!
//! This is the path the Rust sample uses (`orr_sample --connect`): a `RelayClient` runs on the sim
//! thread of a `Threaded` bridge (`orr_bridge::RelayHost`), which predicts the local and the remote
//! players, rolls back when a confirmed input differs and reconciles events as predicted, verified
//! or canceled. The view stream comes from `ViewStreamSource` over that bridge's snapshots, so
//! frames carry the rollback flag and range and events carry their deterministic keys, exactly as
//! `docs/view-stream.md` says.
//!
//! Joining blocks until the room starts (every player present), so the handle is opened without
//! waiting: the connection runs on a thread and the handle reports `ORR_STATE_CONNECTING` until it
//! is done (`orr_session_status`).

use std::collections::VecDeque;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Duration;

use orr_bridge::{Bridge, RelayMetrics, Threaded};
use orr_sample::net_client::{physics_bridge, NetArgs, PHYSICS_BUILD_ID};
use orr_sample::physics_game::{NoCommand, PhysGame, PhysInput};
use orr_sample::physics_host::SimMetrics;
use orr_sample::physics_stream::{phys_client_stream_source, PhysKinds};
use orr_sample::physics_view::PhysExtractor;
use orr_sim::{PlayerSlot, SimCommand};
use orr_viewstream::{EventBatch, ViewStreamSource};

type Joined = Result<(Threaded<PhysGame>, orr_sample::physics_game::PhysConfig), String>;

/// A joined game: the bridge, and what turns its snapshots into stream messages.
pub(crate) struct Playing {
    bridge: Threaded<PhysGame>,
    source: ViewStreamSource<PhysExtractor, PhysKinds>,
    slot: u8,
    players: u8,
}

pub(crate) enum Link {
    /// The connection thread is still joining.
    Connecting(Receiver<Joined>),
    Playing(Box<Playing>),
    /// Joining failed (the reason), or the connection thread died.
    Failed(String),
}

/// A client session of the handle.
pub(crate) struct Client {
    pub link: Link,
    metrics: Arc<RelayMetrics>,
}

/// What `orr_session_status` reports (see the header's `OrrSessionStatus`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Status {
    pub state: u32,
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

pub(crate) const STATE_CONNECTING: u32 = 0;
pub(crate) const STATE_PLAYING: u32 = 1;
pub(crate) const STATE_DISCONNECTED: u32 = 2;
pub(crate) const STATE_FAILED: u32 = 3;

/// What the schema needs once the room has started.
pub(crate) struct Ready {
    pub schema_json: String,
    pub input_size: usize,
    pub player_count: u8,
}

impl Client {
    /// Starts joining on a thread.
    pub fn start(args: NetArgs) -> Result<Client, String> {
        let metrics = RelayMetrics::new();
        let (tx, rx) = channel::<Joined>();
        let (thread_metrics, sim) = (metrics.clone(), SimMetrics::new());
        std::thread::Builder::new()
            .name("orr-ffi-join".to_string())
            .spawn(move || {
                // The receiver is gone if the handle was closed meanwhile: the bridge then drops here.
                let _ = tx.send(physics_bridge(&args, thread_metrics, sim));
            })
            .map_err(|e| format!("cannot start the connection thread: {e}"))?;
        Ok(Client { link: Link::Connecting(rx), metrics })
    }

    /// Moves a finished join into `Playing` (or `Failed`). Returns what the handle must learn when
    /// it becomes ready.
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
                let ready = Ready {
                    schema_json: source.schema().to_json_string(),
                    input_size: std::mem::size_of::<PhysInput>(),
                    player_count: players,
                };
                self.link = Link::Playing(Box::new(Playing { bridge, source, slot, players }));
                Some(ready)
            }
            Err(e) => {
                self.link = Link::Failed(e);
                None
            }
        }
    }

    /// Moves what the bridge produced since the last call into the frame slot and the event queue.
    pub fn pump(&mut self, latest: &mut Option<Vec<u8>>, events: &mut VecDeque<Vec<u8>>, max_batches: usize) {
        let Link::Playing(p) = &mut self.link else { return };
        let out = p.source.pump(&mut p.bridge);
        if let Some(frame) = out.frame {
            *latest = Some(frame.encode());
        }
        if !out.events.is_empty() {
            if events.len() >= max_batches {
                events.pop_front();
            }
            events.push_back(EventBatch { events: out.events }.encode());
        }
    }

    /// Sets the held input of the local player (the bytes of a `PhysInput`).
    pub fn set_input(&mut self, player: u8, bytes: &[u8]) -> Result<(), (i32, String)> {
        let down = self.status().state == STATE_DISCONNECTED;
        let Link::Playing(p) = &mut self.link else { return Err(not_playing(&self.link)) };
        if down {
            return Err((crate::ORR_ERR_HOST, "disconnected from the server".to_string()));
        }
        if player != p.slot {
            return Err((crate::ORR_ERR_ARG, format!("this client plays slot {}: only its own input can be set (got player {player})", p.slot)));
        }
        let input = bytemuck::try_pod_read_unaligned::<PhysInput>(bytes)
            .map_err(|_| (crate::ORR_ERR_ARG, format!("input must be {} bytes", std::mem::size_of::<PhysInput>())))?;
        p.bridge
            .set_input(PlayerSlot(p.slot), input)
            .map_err(|e| (crate::ORR_ERR_HOST, format!("the session is gone: {e}")))
    }

    /// Sends a command of the game to the server with the next tick (`PhysGame` has only the no-op one).
    pub fn send_command(&mut self, player: u8, bytes: &[u8]) -> Result<(), (i32, String)> {
        let Link::Playing(p) = &mut self.link else { return Err(not_playing(&self.link)) };
        if player != p.slot {
            return Err((crate::ORR_ERR_ARG, format!("this client plays slot {}: only its own commands can be sent (got player {player})", p.slot)));
        }
        let command = <NoCommand as SimCommand>::decode(bytes)
            .ok_or((crate::ORR_ERR_ARG, format!("command must be {} bytes", std::mem::size_of::<NoCommand>())))?;
        p.bridge.send_command(command).map_err(|e| (crate::ORR_ERR_HOST, format!("the session is gone: {e}")))
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

    pub fn status(&self) -> Status {
        let m = self.metrics.status();
        let mut s = Status {
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
            ..Status::default()
        };
        match &self.link {
            Link::Connecting(_) => s.state = STATE_CONNECTING,
            Link::Failed(_) => s.state = STATE_FAILED,
            Link::Playing(p) => {
                s.slot = u32::from(p.slot);
                s.player_count = u32::from(p.players);
                s.state = if m.connected && p.bridge.is_alive() { STATE_PLAYING } else { STATE_DISCONNECTED };
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

/// The error of a call that needs a joined game.
pub(crate) fn not_playing(link: &Link) -> (i32, String) {
    match link {
        Link::Connecting(_) => (crate::ORR_ERR_NOT_READY, "the client is still joining the room (see orr_session_status)".to_string()),
        Link::Failed(e) => (crate::ORR_ERR_HOST, format!("joining failed: {e}")),
        Link::Playing(_) => (crate::ORR_ERR_HOST, "the session is gone".to_string()),
    }
}

/// How long `orr_client_open` with `ORR_CLIENT_WAIT` waits at most beyond the connect timeout.
pub(crate) const WAIT_SLACK: Duration = Duration::from_secs(5);
