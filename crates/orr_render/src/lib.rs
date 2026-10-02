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
//! 3D: [`Renderer3D`] draws a [`RenderList3D`] (instanced sphere, box, capsule
//! and plane meshes with a [`Material`] each, 3D debug lines, one shadow
//! casting sun plus hemisphere ambient) through a [`Camera3D`] with a depth
//! buffer and MSAA. See `renderer3d.rs` for the frame and the shading model.
//!
//! Drawn in 2D: filled circles, boxes and capsules (instanced, distance-field
//! edges), and constant-pixel-width lines for gizmos. Text is not drawn yet
//! (use the UI toolkit's text for overlays).
//!
//! This is view layer code: it may use floats. Sim crates never depend on it.
#![allow(clippy::float_arithmetic)]
#![allow(clippy::disallowed_types)]

pub mod camera;
pub mod camera3d;
pub mod extract;
pub mod list;
pub mod list3d;
pub mod math3;
pub mod mesh;
pub mod renderer;
pub mod renderer3d;
mod stats;
pub mod targets;
pub mod text;

pub use camera::Camera;
pub use camera3d::{Camera3D, OrbitCamera, Projection};
pub use list3d::{Instance3D, Lighting, LineInstance3D, Material, RenderList3D, IDENTITY_ROT};
pub use mesh::{MeshKind, MeshSet, Vertex3, DEFAULT_SEGMENTS};
pub use renderer3d::{light_view_proj, Renderer3D, Settings3D, DEFAULT_CLEAR_3D};
pub use extract::{extract_items, instance_of};
pub use list::{LineInstance, RenderList, ShapeInstance};
pub use renderer::{Renderer, DEFAULT_CLEAR};
pub use stats::{FrameStats, PassStats};
pub use targets::{OffscreenTarget, WindowRenderer, WindowRenderer3D};

pub use orr_rhi;
