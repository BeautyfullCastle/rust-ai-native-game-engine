//! `orr_games`: the sample games' simulation side, with no view layer.
//!
//! * [`physics_game`]: `PhysGame`, hundreds to thousands of 2D bodies (`orr_physics`).
//! * [`yard3d_game`]: `Yard3D`, the 3D physics yard (`orr_physics3d`).
//!
//! This crate is sim code (no floats, no wall clock, no hash maps) and depends on
//! no GPU or window crate, so it builds for `wasm32-unknown-unknown` (the browser
//! client), `wasm32-wasip1` and the Android/iOS targets. `orr_sample` re-exports
//! both modules under their old paths (`orr_sample::physics_game`, `orr_sample::yard3d_game`).
#![deny(clippy::float_arithmetic)]
#![deny(clippy::disallowed_types)]

pub mod physics_game;
pub mod yard3d_game;

#[cfg(feature = "terrain-physics")]
pub mod terrain_yard3d_game;

#[cfg(feature = "navigation")]
pub mod navigation_yard3d_game;

/// Bounded deterministic collection/dodge game, opt-in and GPU-free.
#[cfg(feature = "collect-dodge")]
pub mod collect_dodge_game;

#[cfg(feature = "room-escape")]
pub mod room_escape_game;
