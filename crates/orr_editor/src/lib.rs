//! `orr_editor`: the egui editor of Orrery (M4 MVP).
//!
//! The success criterion is the loop *scene edit, play, rewind*: open a scene
//! (default `scenes/physics_demo.scene.yaml`), change bodies with the
//! inspector or by dragging them in the viewport, press Play, scrub the
//! timeline back, and play forward again to the same checksums.
//!
//! # Pieces
//!
//! - [`editor`]: [`Editor`], the state machine (no egui). Wraps
//!   `orr_edit::EditorDoc` (edit mode: undoable scene edits, `Origin::User`)
//!   and `orr_edit::PlayController` (play mode: recorded debug commands).
//! - [`inspector`]: widgets generated from `orr_reflect` descriptors.
//! - [`viewport`]: the render list of the frame on screen (`orr_render`),
//!   picking, and the offscreen texture shared with egui on eframe's own
//!   wgpu device.
//! - [`app`]: [`EditorApp`], the `eframe::App` with all panels.
//! - [`cli`] and [`script`]: command line flags (`--scene`, `--screenshot`,
//!   `--frames`, `--play-ticks`, `--select`, `--script`) for headless checks.
//!
//! The editor is `PhysGame` specific (the sample physics game): the play
//! session type and the viewport shapes come from `orr_sample`. Making it
//! generic over `Game` is future work. The play session runs on the UI thread
//! (see [`Editor`]).
//!
//! This is view layer code: floats and the wall clock are fine here, but every
//! value that reaches the document goes through exact decimal parsing.
#![allow(clippy::float_arithmetic)]
#![allow(clippy::disallowed_types)]

pub mod app;
pub mod cli;
pub mod editor;
pub mod inspector;
pub mod script;
pub mod viewport;

pub use app::{EditorApp, ScreenshotJob};
pub use editor::{Editor, Mode, Owner};
