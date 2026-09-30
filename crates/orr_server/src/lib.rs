//! `orr_server`: the Relay server core (design §6.1, "Relay" and
//! "Relay + Validate" modes).
//!
//! The server does not simulate. It collects each player's input per tick,
//! finalizes tick `T` at its deadline on the server's tick clock (a missing
//! input becomes the slot's previous input, marked *repeated*), and
//! broadcasts one confirmed bundle per tick. Clients simulate and treat the
//! bundle as the truth. See [`RoomConfig`] and the module docs of `room`
//! for the exact rules; the wire messages live in `orr_proto`.
//!
//! The core is a library driven by `RelayServer::update(now_us)` with an
//! injected time, on top of an abstract `orr_proto::Endpoint`, so it is
//! deterministic and testable without sockets (`orr_proto::netsim`).
//!
//! Late join: the server asks a playing client (the donor) for a snapshot
//! of its verified frame, relays it to the joiner and follows it with every
//! confirmed bundle after the snapshot tick (the server keeps them, so
//! there is never a gap). The joiner takes over its slot at the tick the
//! server sees its `Ready`.
#![deny(clippy::disallowed_types)]
#![deny(clippy::float_arithmetic)]

#[cfg(feature = "harness")]
pub mod harness;
mod room;
mod server;
mod validate;

pub use room::{RoomConfig, RoomStats, ServerNote, SlotStats, VacantPolicy};
pub use server::RelayServer;
pub use validate::{AcceptAll, InputCtx, InputValidator, Verdict};
