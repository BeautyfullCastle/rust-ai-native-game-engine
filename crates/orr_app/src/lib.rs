//! Runs a [`orr_sim::Game`] in a window with almost no code of its own.
//!
//! Stage 1 of `docs/view-framework.md`: offline only (one local player, no server),
//! a fixed tick from the clock, interpolation through `orr_view`, a default 2D
//! renderer, and `--screenshot` / `--ticks` for pictures that can be compared.
//! The simulation is never touched from here: the window reads snapshots and sends
//! input through the bridge.

// View layer: floats and the wall clock are fine here, the sim crates forbid them.
#![allow(clippy::float_arithmetic)]
#![allow(clippy::disallowed_types)]

mod app;
mod cli;
mod input;
mod pace;
mod shot;
mod view2d;

pub use app::{App, Error};
pub use input::InputContext;
pub use orr_bridge::{FrameView, PlayerSlot};
pub use orr_render::Camera;
pub use orr_view::{fp_to_f32, fp_to_vec2, Extracted, Extractor, InterpMode, RenderItem};
pub use view2d::{solid_quad, View2D};
pub use winit::keyboard::KeyCode;
