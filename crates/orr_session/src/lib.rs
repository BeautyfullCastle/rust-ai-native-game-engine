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

mod dump;
mod events;
mod input_source;
mod join;
mod join_bootstrap;
mod play;
mod relay;
mod replay;
mod session;
mod wire;

pub use dump::{DesyncDump, DumpError};
pub use relay::{
    ClientEvent, ClientState, ClientStats, DumpCollector, DumpSink, RelayClient, RelayClientConfig, RelaySource,
    RelayUpdate, SourceStats,
};
pub use events::{EventBatch, EventStatus};
pub use join_bootstrap::{JoinBootstrap, JoinBootstrapError, JoinBootstrapStatus, JoinRoster};
pub use join::{join_request, JoinAttempts, JoinError, JoinTicket};
pub use input_source::{InputSource, LocalInputSource, LoopbackClock, LoopbackEnd, LoopbackNetwork, RemoteInput};
pub use play::{
    ControlOp, PlayConfig, PlayError, PlayMode, PlayNote, PlaySession, Speed, Timeline, TIMELINE_CHECKSUM_WINDOW,
};
pub use replay::{
    replay_seek_checked, replay_verify, replay_verify_checked, ReplayError, ReplayHeader, ReplayReader, ReplayWriter,
    VerifyReport,
};
pub use session::{
    compare_checksums, require_same_build_hash, AdvanceResult, Anchor, BuildHashMismatch, Desync, JoinStatus, RollbackInfo, Session,
    SessionConfig,
};

pub use orr_sim::{DebugCommand, DebugError, EventKey, Game, PlayerSlot, SimEvent};
