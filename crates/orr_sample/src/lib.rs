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
pub mod net_client;
pub mod physics_app;
pub mod physics_game;
pub mod physics_host;
pub mod physics_view;
