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

/// Explicitly create one new private Windows Stormglass settings directory.
/// Existing objects are never changed. Runtime admission still applies at launch.
#[cfg(all(windows, feature = "coop-defense-controls"))]
pub fn prepare_stormglass_settings_directory(path: &std::path::Path) -> Result<(), String> {
    defense_audio_settings::prepare(path, &stormglass_settings_forbidden_roots()?)
}

/// Point-in-time admission result; neither a write probe nor a future Save guarantee.
#[cfg(all(windows, feature = "coop-defense-controls"))]
#[derive(Debug, PartialEq, Eq)]
pub struct StormglassSettingsStatus {
    pub audio_saved: bool,
    pub controls_saved: bool,
}

/// Check both profiles using native startup admission without creating or changing data.
#[cfg(all(windows, feature = "coop-defense-controls"))]
pub fn check_stormglass_settings_directory(
    path: &std::path::Path,
) -> Result<StormglassSettingsStatus, String> {
    let (audio_saved, controls_saved) =
        defense_audio_settings::check(path, &stormglass_settings_forbidden_roots()?)?;
    Ok(StormglassSettingsStatus {
        audio_saved,
        controls_saved,
    })
}

#[cfg(all(windows, feature = "coop-defense-controls"))]
fn stormglass_settings_forbidden_roots() -> Result<Vec<std::path::PathBuf>, String> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let parent = executable.parent().ok_or("missing executable parent")?;
    let mut forbidden = vec![parent.to_owned()];
    let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    if source.exists() {
        forbidden.push(source);
    }
    Ok(forbidden)
}

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
pub use orr_games::dragon;
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

#[cfg(feature = "tilemap")]
pub mod project_tilemap;
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
#[cfg(all(feature = "runtime-export-support", target_os = "linux", target_arch = "x86_64"))]
pub mod trusted_runtime;
#[cfg(all(windows, target_arch = "x86_64", feature = "project-export"))]
pub mod windows_export;
#[cfg(all(windows, target_arch = "x86_64", feature = "project-create", feature = "room-project"))]
pub mod windows_project;
#[cfg(all(windows, target_arch = "x86_64", feature = "room-project", any(feature="project-create", all(test, feature="project-export"))))]
extern crate self as orr_sample;

#[cfg(feature = "player-settings")]
pub mod player_settings;
#[cfg(feature = "player-settings")]
pub mod player_controls;

#[cfg(all(feature = "project-create", target_os = "linux"))]
pub mod project_create;

#[cfg(all(
    target_os = "linux",
    any(feature = "project-create", all(feature = "project-export", target_arch = "x86_64"), feature = "runtime-export-support")
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
#[cfg(feature = "collect-ui")]
pub mod authored_ui;
#[cfg(feature = "collect-ui")]
pub mod collect_ui;

#[cfg(all(any(feature = "collect-progress", feature = "room-progress"), target_os = "linux"))]
pub mod game_progress;
#[cfg(all(feature = "room-progress", target_os = "linux"))]
mod room_progress;
#[cfg(all(feature = "room-progress", target_os = "linux"))]
mod room_progress_digest;
#[cfg(all(feature = "collect-progress", target_os = "linux"))]
mod collect_progress;

#[cfg(all(feature = "collect-progress", target_os = "linux"))]
mod collect_progress_host;

#[cfg(all(feature = "image-reimport", target_os = "linux", target_arch = "x86_64"))]
pub mod image_reimport;

#[cfg(feature = "room-project")]
pub mod room_project;
#[cfg(feature = "room-project")]
pub mod room_layout;
#[cfg(feature = "room-input-actions")]
pub mod room_controls;
#[cfg(feature = "room-project")]
pub mod room_camera;
#[cfg(feature = "room-project")]
pub use orr_games::room_escape_game as room_game;

#[cfg(feature = "room-project")]
pub mod room_view;

#[cfg(feature = "room-project")]
pub mod room_app;

#[cfg(feature = "particle-demo")]
pub mod particle_demo;

#[cfg(feature = "particle-preset")]
pub mod particle_preset;

// Shared by lib tests so fixture paths use one process-wide directory counter.
#[cfg(all(
    test,
    any(feature = "tilemap", all(feature = "project", feature = "game-ui"))
))]
#[path = "../tests/common/project.rs"]
mod project_fixture;

#[cfg(feature = "room-lighting")]
pub mod room_lighting;

#[cfg(feature = "room-character")]
pub mod room_character;

#[cfg(feature = "coop-defense")]
pub mod defense_app;
#[cfg(feature = "coop-defense-presentation")]
pub mod defense_view;
#[cfg(feature = "coop-defense")]
pub mod defense_ui;

#[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
pub mod defense_run_save;
#[cfg(all(feature = "coop-defense-run-save", target_os = "linux"))]
pub mod defense_run_save_store;

#[cfg(feature = "coop-defense-audio")]
pub mod defense_audio;

#[cfg(all(feature = "coop-defense-audio-settings", any(test, target_os = "linux", windows)))]
pub(crate) mod defense_audio_settings;

#[cfg(all(feature = "coop-defense-controls", any(target_os = "linux", windows)))]
pub(crate) mod defense_controls;

#[cfg(feature = "coop-defense-pixelfx")]
pub(crate) mod defense_vfx;

#[cfg(feature = "coop-defense-pixel-motion")]
pub(crate) mod defense_pixel_motion;

#[cfg(all(target_os="linux",feature="project-create",feature="room-lighting"))]
pub mod room_lit_starter;

#[cfg(all(feature = "room-controls-attach", target_os = "linux", target_arch = "x86_64"))]
pub mod room_controls_attach;
