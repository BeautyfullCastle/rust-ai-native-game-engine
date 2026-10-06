//! `orr_sample`: a runnable demo of the M2 view side.
//!
//! The arena test game runs through an `orr_bridge` adapter, as a local peer
//! of a two-peer loopback session with simulated latency (so rollbacks
//! happen). `orr_view` interpolates it and smooths rollback corrections,
//! and `orr_render` (wgpu through `orr_rhi`) draws it in a winit window.
//!
//! - [`arena_view`]: what to draw, input mapping, the bot, the loopback session. No GPU.
//! - [`app`]: the winit loop.
//!
//! This crate is view layer: floats and the wall clock are fine here.
#![allow(clippy::float_arithmetic)]
#![allow(clippy::disallowed_types)]

pub mod app;
pub mod arena_view;
pub mod editor_view;
/// Compiled Arena game and descriptors exposed to view clients.
pub use orr_testgame as arena_game;
pub mod arena_audio;
pub mod net_client;
pub mod physics_app;
pub use orr_games::physics_game;
pub mod physics_host;
pub mod physics_stream;
pub mod physics_view;
pub mod relay_view;
pub mod yard3d_app;
pub use orr_games::yard3d_game;
pub mod yard3d_host;
pub mod yard3d_stream;
pub mod yard3d_view;

#[cfg(feature = "sprites")]
pub mod sprite_scene;
