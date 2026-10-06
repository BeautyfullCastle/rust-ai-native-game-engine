//! `orr_editor`: the egui editor of Orrery (M4 MVP).
//!
//! The success criterion is the loop *scene edit, play, rewind*: open a scene
//! (default `scenes/physics_demo.scene.yaml`), change bodies with the
//! inspector or by dragging them in the viewport, press Play, scrub the
//! timeline back, and play forward again to the same checksums.
//!
//! # The editor is a view of a simulation host
//!
//! The editor owns no document, no play session and no ERP server. The
//! simulation and the scene document live in a **host** (`orr_remote::Host`):
//! a thread of this process (the default) or another process
//! (`orr_remote_host`, `--connect`). The editor is a client: it speaks ERP
//! for everything a person does and reads the frames it draws through the
//! bridge (`orr_bridge`: `Bridge`, `SimControl`, `Snapshot`, here
//! `orr_remote::RemoteBridge`). A host thread that panics, or a remote host
//! that goes away, does not take the editor down: it says so and offers
//! Restart / Reconnect. See [`backend`] and [`editor`].
//!
//! # Pieces
//!
//! - [`editor`]: [`Editor`], the state machine (no egui): caches of host
//!   answers, actions as ERP calls, and the per-frame [`Editor::pump`].
//! - [`backend`]: [`HostSpec`], the two channels to the host.
//! - [`model`]: plain data parsed from ERP answers.
//! - [`inspector`]: widgets generated from `orr_reflect` descriptors.
//! - [`viewport`]: the render list of the frame on screen (`orr_render`),
//!   picking, and the offscreen texture shared with egui on eframe's own
//!   wgpu device.
//! - [`agent`] and [`agent_ui`]: the Agent tab, a read-only activity feed of
//!   what AI agents do through ERP (no approval step; Undo takes a change
//!   back), plus a view-only preview of the proposals an agent has open (the
//!   staged frame comes from the host as a second frame stream).
//! - [`app`]: [`EditorApp`], the `eframe::App` with all panels.
//! - [`cli`] and [`script`]: command line flags (`--scene`, `--screenshot`,
//!   `--frames`, `--play-ticks`, `--select`, `--script`) for headless checks.
//!
//! Compiled adapters support local/remote `PhysGame` and remote `Arena`.
//! Frame decoders, reflection and viewport mappings come through `orr_sample`.
//! Arena optionally claims a focused realtime keyboard slot through negotiated
//! structured ERP input. Ordinary attachment never changes agent-held input.
//!
//! This is view layer code: floats and the wall clock are fine here, but every
//! value that reaches the document goes through exact decimal parsing.
#![allow(clippy::float_arithmetic)]
#![allow(clippy::disallowed_types)]

pub mod agent;
pub mod agent_ui;
pub mod app;
pub mod backend;
pub mod cli;
pub mod diagnostics;
pub mod editor;
pub mod inspector;
pub mod game;
pub mod model;
pub mod script;
pub mod viewport;

pub use app::{EditorApp, ScreenshotJob};
pub use backend::HostSpec;
pub use editor::{Editor, Mode, Owner};
pub use model::Target;

#[cfg(feature = "sprites")]
pub mod sprite_bindings;
#[cfg(feature = "sprites")]
pub mod sprite_panel;

#[cfg(feature = "animated-models")]
pub mod animated_bindings;
#[cfg(feature = "animated-models")]
pub mod animated_preview;
#[cfg(feature = "animated-models")]
pub mod animated_panel;

pub mod viewport3d;
#[cfg(feature = "models")]
pub mod model_bindings;
#[cfg(feature = "models")]
pub mod model_panel;
