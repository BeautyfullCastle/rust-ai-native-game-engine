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

#[cfg(feature = "input-actions")]
pub mod arena_input;

#[cfg(feature = "game-ui")]
pub mod game_ui;
#[cfg(feature = "game-ui")]
pub mod game_ui_gpu;

#[cfg(feature = "project")]
pub mod project_sprites;
#[cfg(feature = "project")]
pub mod project;
#[cfg(feature = "project")]
pub mod project_playback;
#[cfg(feature = "project")]
pub mod project_runtime;
#[cfg(feature = "project")]
pub mod project_compositor;

#[cfg(all(feature = "project-export", target_os = "linux", target_arch = "x86_64"))]
pub mod project_export;

#[cfg(feature = "player-settings")]
pub mod player_settings;
#[cfg(feature = "player-settings")]
pub mod player_controls;

#[cfg(all(feature = "project-create", target_os = "linux"))]
pub mod project_create;

#[cfg(all(
    target_os = "linux",
    any(feature = "project-create", all(feature = "project-export", target_arch = "x86_64"))
))]
mod project_publish;

#[cfg(feature = "collect-dodge")]
pub mod collect_project;
#[cfg(feature = "collect-dodge")]
pub use orr_games::collect_dodge_game as collect_game;

#[cfg(feature = "collect-dodge")]
pub mod collect_view;
#[cfg(feature = "collect-dodge")]
pub mod collect_app;

#[cfg(all(feature = "collect-progress", target_os = "linux"))]
pub mod game_progress;
#[cfg(all(feature = "collect-progress", target_os = "linux"))]
mod collect_progress;

#[cfg(all(feature = "collect-progress", target_os = "linux"))]
mod collect_progress_host;
