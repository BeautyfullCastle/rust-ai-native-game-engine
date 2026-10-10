//! `orr_tui`: a terminal view of an Orrery simulation that knows nothing of Orrery but the
//! view stream (`docs/view-stream.md`). It is the proof of decision 12: the view can be replaced.
//!
//! It depends on `orr_viewstream` for the byte format only (no simulation, session, physics,
//! bridge, view or game crate; `tests/deps.rs` enforces it). The stream comes over a socket
//! ([`source::SocketSource`]: ERP over WebSocket or TCP) or through the C ABI
//! ([`ffi::FfiSource`], feature `ffi`: the shared library loaded at run time).
//!
//! This is a view-boundary crate: floats are allowed here, the simulation has none.
#![allow(clippy::float_arithmetic)]
#![allow(clippy::disallowed_types)] // the view keeps its own wall clock

pub mod headless;
pub mod input;
#[cfg(test)]
mod input_tests;
pub mod render;
pub mod render3d;
#[cfg(test)]
mod render_tests;
pub mod schema;
pub mod source;
pub mod state;
#[cfg(test)]
mod state_tests;
pub mod ui;

#[cfg(feature = "ffi")]
pub mod ffi;
