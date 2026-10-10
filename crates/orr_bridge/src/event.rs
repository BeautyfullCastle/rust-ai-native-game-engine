use orr_session::{EventStatus, RollbackInfo};
use orr_sim::{DebugError, EventKey, PlayerSlot};

/// Session-level notifications (design doc 5.2, "Lifecycle").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifecycle {
    /// The sim host is up. Sent once, first.
    SessionStarted { tick_rate: u32, local_slot: PlayerSlot, player_count: u8 },
    /// A misprediction was corrected: the sim went back to `from_tick - 1`
    /// and resimulated up to `to_tick`. Positions of those ticks changed.
    Rollback(RollbackInfo),
    /// The predicted head reached the prediction limit, so the sim waits for
    /// remote input. Sent once per stall, not once per tick.
    Stalled { head_tick: u64 },
    /// Relay play only: the server found different checksums at `tick`
    /// (a desync dump was written on this client).
    Desync { tick: u64 },
    /// Relay play only: the connection to the server is gone. The sim stays
    /// at its last tick.
    Disconnected,
    /// Relay play only: the client changed its input delay (in ticks).
    DelayChanged { delay: u32 },
    /// Play session only: the head jumped from tick `from` to tick `to`.
    /// The view must reset interpolation and smoothing (no blend across it).
    Seeked { from: u64, to: u64 },
    /// Play session only: the recorded future after `tick` was dropped
    /// (`dropped` ticks) and recording goes on from `tick`.
    Branched { tick: u64, dropped: u64 },
    /// Play session only: the session paused at `tick`.
    Paused { tick: u64 },
    /// Play session only: the session started running at `tick`.
    Resumed { tick: u64 },
    /// Play session only: a debug command was refused, nothing changed.
    DebugRejected(DebugError),
    /// Play session only: a seek target was outside the recorded range.
    SeekRejected { target: u64 },
}

/// One notification from the sim to the view.
#[derive(Clone, Debug, PartialEq)]
pub enum BridgeEvent<E> {
    /// Legacy split-read API recovery marker. Prefer Bridge::poll_view, whose
    /// separate resync field pairs this with its exact snapshot baseline.
    ViewResynced(crate::ViewResync),
    /// A game event with its state: `Predicted` (may still be canceled),
    /// `Verified` (final, announced once) or `Canceled` (a rollback removed
    /// it). `key` is the deterministic event identity, the same across a
    /// rollback resimulation, so the view can match a `Canceled` or
    /// `Verified` to the `Predicted` it saw before.
    Sim { key: EventKey, status: EventStatus<E> },
    Lifecycle(Lifecycle),
}

/// Counters for a debug overlay.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BridgeStats {
    /// Ticks the predicted head advanced.
    pub ticks: u64,
    /// Rollbacks done.
    pub rollbacks: u64,
    /// Ticks resimulated by all rollbacks together.
    pub resimulated_ticks: u64,
    /// Deepest rollback, in ticks.
    pub max_rollback_depth: u32,
    /// Sim steps skipped because the prediction limit was reached.
    pub stalls: u64,
}
