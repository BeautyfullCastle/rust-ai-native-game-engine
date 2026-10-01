//! Client mode of the C ABI: the handle plays on an `orr_server` room instead of hosting a
//! simulation of its own.
//!
//! The session itself is `orr_sample::relay_view::RelayView`, shared with the ERP host
//! (`orr_remote_host --join`): a `RelayClient` on the sim thread of a `Threaded` bridge, and
//! `ViewStreamSource` over that bridge's snapshots, so frames carry the rollback flag and range and
//! events carry their deterministic keys, exactly as `docs/view-stream.md` says. This file only maps
//! it to the ABI's status struct and error codes.
//!
//! Joining blocks until the room starts (every player present), so the handle is opened without
//! waiting: the connection runs on a thread and the handle reports `ORR_STATE_CONNECTING` until it
//! is done (`orr_session_status`).

use std::collections::VecDeque;
use std::time::Duration;

use orr_sample::net_client::NetArgs;
use orr_sample::relay_view::{RelayView, ViewError, ViewErrorKind};

/// A client session of the handle.
pub(crate) struct Client {
    view: RelayView,
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

/// What the schema needs once the room has started.
pub(crate) struct Ready {
    pub schema_json: String,
    pub input_size: usize,
    pub player_count: u8,
}

fn code(e: ViewError) -> (i32, String) {
    let code = match e.kind {
        ViewErrorKind::NotReady => crate::ORR_ERR_NOT_READY,
        ViewErrorKind::Arg => crate::ORR_ERR_ARG,
        ViewErrorKind::Host => crate::ORR_ERR_HOST,
    };
    (code, e.message)
}

impl Client {
    /// Starts joining on a thread.
    pub fn start(args: NetArgs) -> Result<Client, String> {
        Ok(Client { view: RelayView::start(args)? })
    }

    /// Moves a finished join into `Playing` (or `Failed`).
    pub fn poll_join(&mut self) -> Option<Ready> {
        self.view.poll_join().map(|r| Ready { schema_json: r.schema.to_json_string(), input_size: r.input_size, player_count: r.player_count })
    }

    /// Moves what the bridge produced since the last call into the frame slot and the event queue.
    pub fn pump(&mut self, latest: &mut Option<Vec<u8>>, events: &mut VecDeque<Vec<u8>>, max_batches: usize) {
        let out = self.view.pump();
        if let Some(frame) = out.frame {
            *latest = Some(frame);
        }
        if let Some(batch) = out.events {
            if events.len() >= max_batches {
                events.pop_front();
            }
            events.push_back(batch);
        }
    }

    pub fn set_input(&mut self, player: u8, bytes: &[u8]) -> Result<(), (i32, String)> {
        self.view.set_input(player, bytes).map_err(code)
    }

    pub fn send_command(&mut self, player: u8, bytes: &[u8]) -> Result<(), (i32, String)> {
        self.view.send_command(player, bytes).map_err(code)
    }

    /// The checksum of the confirmed state at `tick` (0 = the newest checkpoint): `(tick, checksum)`.
    pub fn confirmed_checksum(&self, tick: u64) -> Option<(u64, u64)> {
        self.view.confirmed_checksum(tick)
    }

    /// Whether the join is still going on.
    pub fn is_joining(&self) -> bool {
        self.view.is_joining()
    }

    pub fn status(&self) -> Status {
        let s = self.view.status();
        Status {
            state: s.state as u32,
            slot: s.slot,
            player_count: s.player_count,
            rtt_ms: s.rtt_ms,
            input_delay: s.input_delay,
            desyncs: s.desyncs,
            head_tick: s.head_tick,
            verified_tick: s.verified_tick,
            rollbacks: s.rollbacks,
            resim_ticks: s.resim_ticks,
            last_rollback_from: s.last_rollback_from,
            last_rollback_to: s.last_rollback_to,
            stall_episodes: s.stall_episodes,
            stalled_ms: s.stalled_ms,
            repeats: s.repeats,
        }
    }

    /// The reason a failed join gave.
    pub fn failure(&self) -> Option<&str> {
        self.view.failure()
    }
}

/// How long `orr_client_open` with `ORR_CLIENT_WAIT` waits at most beyond the connect timeout.
pub(crate) const WAIT_SLACK: Duration = orr_sample::relay_view::WAIT_SLACK;
