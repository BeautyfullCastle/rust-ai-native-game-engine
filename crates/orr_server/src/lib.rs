//! `orr_server`: the server core (design §6.1: "Relay", "Relay + Validate"
//! and "Authoritative" modes).
//!
//! In relay mode (the default) the server does not simulate. It collects each player's input per tick,
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
//! Late join: the server picks a checksum tick that the other clients have
//! reported and asks a playing client (the donor) for its verified frame of
//! exactly that tick. It judges the snapshot against those reports, relays it
//! to the joiner and follows it with every
//! confirmed bundle after the snapshot tick (the server keeps them, so
//! there is never a gap). The joiner takes over its slot at the tick the
//! server sees its `Ready`.
//!
//! # Authoritative mode
//!
//! A room created with [`RelayServer::create_authoritative_room`] also runs
//! the game's `orr_sim` headless ([`ServerSim`], [`GameSim`]) on the
//! confirmed bundles, one tick at a time, as each tick is finalized. The
//! wire protocol is the relay's (the one protocol version, see
//! `orr_proto::msg`; `Welcome::flags` and `ServerMsg::Correction` serve this
//! mode), so clients are the same `RelayClient`s. What the
//! server adds:
//!
//! - **Checksum referee**: a client's checkpoint checksum is compared with
//!   the server's own. The server is the truth, so a mismatch names that
//!   client (no vote). It gets a `Desync` notice and a `Correction`: the
//!   server's frame of the newest confirmed tick, lz4, restored by the client
//!   like a late join (the bundles after it come through the normal confirmed
//!   stream). The server writes a `.orrd` dump (`set_dump_sink`). A client
//!   that is wrong `kick_after_corrections` times within `kick_window_secs`
//!   is kicked (`Bye` with `BYE_KICKED_DESYNC`).
//! - **Late join and rejoin** are served from the server's own frame, no
//!   donor client is asked.
//! - **Server-driven slots** ([`AuthoritativeConfig::server_slots`]): a
//!   brain ([`GameSim::with_brain`]) decides the input and commands of AI
//!   players or scripted events from the server frame; they travel in the
//!   confirmed bundle, so every client simulates them identically.
//! - **Cheat checks**: [`InputValidator`] (a [`Verdict::Kick`] removes the
//!   player) and a state audit ([`GameSim::with_audit`]) that sees the
//!   frames before and after each tick. Violations are logged
//!   ([`ServerNote::Violation`]) and kick at `violation_limit`.
//!
//! See `docs/authoritative.md` for the flow, messages, costs and limits.
#![deny(clippy::disallowed_types)]
#![deny(clippy::float_arithmetic)]

pub mod authoritative;
#[cfg(feature = "harness")]
pub mod harness;
pub mod presets;
mod room;
pub mod serve;
mod server;
mod validate;

pub use authoritative::{Audit, AuditFn, Brain, DirDumps, DumpWriter, GameSim, NoDumps, ServerSim, SharedDumps, Violation};
pub use room::{AuthoritativeConfig, RoomConfig, RoomStats, ServerNote, SlotStats, VacantPolicy};
pub use server::RelayServer;
pub use validate::{AcceptAll, InputCtx, InputValidator, Verdict};
