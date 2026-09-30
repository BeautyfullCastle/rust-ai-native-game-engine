//! `orr_remote`: ERP, the Engine Remote Protocol, and the `Remote` bridge adapter.
//!
//! # What this is
//!
//! An AI agent (through an MCP adapter), a script, or a second process can
//! look at and change a running Orrery editor or headless host over
//! **JSON-RPC 2.0 on a WebSocket** (or newline-delimited JSON on plain TCP).
//! Agent edits go through the same [`orr_edit::EditorDoc`] as a person's, so
//! they share one undo stack; every agent edit carries
//! `Origin::Agent(<client name of its token>)`.
//!
//! # Pieces
//!
//! - [`ErpServer`]: the server. Its network threads only accept, parse and
//!   queue; the host calls [`ErpServer::poll`] once per frame with an
//!   [`ErpTarget`] that borrows the host's document and play session. The
//!   host never blocks on the network.
//! - [`methods`]: every method with its capability and parameters
//!   (`rpc.discover` prints this table).
//! - [`json`]: the exact JSON form of `orr_reflect::Value` (fixed-point
//!   numbers are exact decimals, never `f64`).
//! - [`Caps`], [`Auth`]: capability tokens (`read`, `scene_edit`,
//!   `sim_control`, `approve`).
//! - Proposals (`proposal.*`, `verify.self`): an agent stages edits on a
//!   private copy of the scene, reads the diff, verifies it by replaying
//!   inputs on the scene with and without the edits (checksums, metrics,
//!   pass/fail `checks`), and accepts it (needs `approve`) as one undoable
//!   history entry. [`GameHooks`] (in [`HostLimits`]) gives the host's game
//!   metrics and scripted players; `watch.proposals` tells every subscriber
//!   what happened to the proposals.
//! - [`RemoteBridge`]: an `orr_bridge::Bridge` + `SimControl` over ERP, so
//!   view code can attach to a sim in another process. The server streams
//!   frame snapshots (binary, lz4) and the bridge rebuilds `Snapshot`s.
//! - [`ErpClient`]: a small blocking client (tests, tools).
//! - [`Host`]: a headless host loop (used by the `orr_remote_host` binary).
//!
//! # Threads and the wall clock
//!
//! This is a tool crate, not a sim crate: it uses the wall clock (pacing,
//! timeouts) and threads. Nothing here runs inside a simulation tick. The
//! only sim state it changes is through `PlayController`'s recorded debug
//! commands, so play stays deterministic and replayable.
#![deny(clippy::float_arithmetic)]
#![allow(clippy::disallowed_types)]

mod caps;
mod client;
pub mod codec;
mod dispatch;
mod error;
mod host;
pub mod json;
pub mod methods;
mod net;
mod proposals;
mod remote;
mod server;
pub mod wire;

pub use caps::{Auth, Cap, Caps, TokenEntry};
pub use client::{ClientError, ErpClient};
pub use dispatch::{call_local, ErpTarget, HostLimits};
pub use error::*;
pub use host::{Host, Pacer};
pub use proposals::{default_build_id, BotFn, GameHooks};
pub use remote::{RemoteBridge, RemoteConfig, RemoteMetrics};
pub use server::{ErpServer, PollReport, ServerConfig, ServerError, ServerStats, MAX_PENDING_BYTES};
