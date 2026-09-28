//! `orr_session`: prediction, rollback, replay and desync detection on top
//! of `orr_sim`.
//!
//! [`Session<G, S>`] owns a rollback-capable `orr_sim::Simulation<G>`: it
//! predicts ahead of the last fully-confirmed ("verified") tick using
//! repeat-last-input prediction, detects mispredictions as confirmed
//! inputs arrive (via an [`InputSource`]), and resimulates from the
//! earliest wrong tick when needed. It also reconciles the events a `Game`
//! emits into [`EventBatch`]es the view layer can trust (`Predicted` /
//! `Verified` / `Canceled`, deduplicated across a rollback by
//! `(EventKey, payload)`), and offers `.orrp` [`ReplayWriter`]/
//! [`ReplayReader`] recording and headless verification.
#![deny(clippy::disallowed_types)]
#![deny(clippy::float_arithmetic)]

mod events;
mod input_source;
mod replay;
mod session;

pub use events::{EventBatch, EventStatus};
pub use input_source::{InputSource, LocalInputSource, LoopbackClock, LoopbackEnd, LoopbackNetwork, RemoteInput};
pub use replay::{replay_verify, replay_verify_checked, ReplayError, ReplayHeader, ReplayReader, ReplayWriter, VerifyReport};
pub use session::{
    compare_checksums, require_same_build_hash, AdvanceResult, BuildHashMismatch, Desync, RollbackInfo, Session,
    SessionConfig,
};

pub use orr_sim::{EventKey, Game, PlayerSlot, SimEvent};
