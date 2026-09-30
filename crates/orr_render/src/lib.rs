//! `orr_render`: the 2D renderer of the view layer (design doc 4.2), built on
//! [`orr_rhi`].
//!
//! Flow each frame: the view world (`orr_view`) produces `RenderItem`s;
//! [`extract::extract_items`] turns them into a [`RenderList`] (plus debug
//! lines and shapes the caller adds); a [`Renderer`] draws the list through a
//! [`Camera`] into a texture view. Two targets are provided:
//!
//! - [`WindowRenderer`]: a window surface (the samples).
//! - [`OffscreenTarget`]: a texture (the editor viewport: show
//!   [`OffscreenTarget::sample_view`] in egui).
//!
//! Drawn: filled circles, boxes and capsules (instanced, distance-field
//! edges), and constant-pixel-width lines for gizmos. Text is not drawn yet
//! (use the UI toolkit's text for overlays).
//!
//! This is view layer code: it may use floats. Sim crates never depend on it.
#![allow(clippy::float_arithmetic)]
#![allow(clippy::disallowed_types)]

pub mod camera;
pub mod extract;
pub mod list;
pub mod renderer;
pub mod targets;

pub use camera::Camera;
pub use extract::{extract_items, instance_of};
pub use list::{LineInstance, RenderList, ShapeInstance};
pub use renderer::{Renderer, DEFAULT_CLEAR};
pub use targets::{OffscreenTarget, WindowRenderer};

pub use orr_rhi;
