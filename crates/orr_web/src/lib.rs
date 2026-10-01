//! `orr_web`: the Orrery browser client.
//!
//! It runs the same deterministic simulations (`orr_testgame::Arena` and the
//! physics sample `orr_games::physics_game::PhysGame`) and the
//! same `orr_session::RelayClient` (prediction, rollback, input-delay control,
//! desync checks) as the native clients, compiled to `wasm32-unknown-unknown`.
//! Only the transport is different:
//!
//! * [`WebLink`] is an [`orr_proto::Link`] fed by the browser: a small JS
//!   module (`js/transport.js`) opens a WebTransport session (datagrams =
//!   unreliable channel, one bidirectional stream = reliable channel) and
//!   falls back to a WebSocket. It pushes events into the link through
//!   [`LinkPort`] and the link calls back into it to send.
//! * `wasm` (wasm32 only): the `WebClient` class exported to JavaScript.
//!
//! Nothing in this crate touches a float or the wall clock: the page passes
//! the time in microseconds, and drawing data leaves as integers (world
//! units), so the browser's canvas code does the float work.
//!
//! See `docs/webtransport-trial.md` for how to build and run it.
#![deny(clippy::disallowed_types)]
#![deny(clippy::float_arithmetic)]

mod bot;
mod link;
mod phys;
mod report;

pub use bot::arena_bot_input;
pub use link::{LinkPort, WebLink};
pub use phys::{render_phys, scene_box, DRAW_CAPSULE, DRAW_CIRCLE, DRAW_QUAD, PHYS_STRIDE};
pub use report::{client_report_json, render_arena, ARENA_BUILD_ID, PHYSICS_BUILD_ID};

#[cfg(target_arch = "wasm32")]
mod wasm;
