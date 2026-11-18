//! Runs a [`orr_sim::Game`] in a window with almost no code of its own.
//!
//! Stages 1 to 4 of `docs/view-framework.md`: offline (one or more local players on one screen, no
//! server) or on a relay server (`--host`, `--connect`: one player per window),
//! a fixed tick from the clock, interpolation through `orr_view`, a default 2D and 3D
//! renderer, and `--screenshot` / `--ticks` for pictures that can be compared.
//! `EntityView` and `SceneView` get creates, destroys and events (also the canceled ones).
//! The simulation is never touched from here: the window reads snapshots and sends
//! input through the bridge.

// View layer: floats and the wall clock are fine here, the sim crates forbid them.
#![allow(clippy::float_arithmetic)]
#![allow(clippy::disallowed_types)]

mod app;
mod cli;
mod input;
mod net;
mod pace;
mod shot;
mod view2d;
mod view3d;
mod views;

pub use app::{App, Error};
pub use input::{InputContext, KeyLayout, Stick};
pub use orr_bridge::{FrameView, Lifecycle, PlayerSignal, PlayerSlot, Players, RawPlayerSignal, SignalKind};
pub use orr_render::{Camera, Camera3D, Lighting, OrbitCamera};
pub use orr_ecs::Entity;
pub use orr_view::{fp_to_f32, fp_to_vec2, Extracted, Extracted3, Extractor, Extractor3, InterpMode, RenderItem, Shape3, Style3, Transform2, Transform3};
pub use view2d::{solid_quad, View2D};
pub use view3d::{default_lighting, default_orbit, extract_default, scene_camera, Look3D, View3D};
pub use views::{EntityFactory, EntityView, EventState, SceneView, ViewContext, Views};
pub use winit::keyboard::KeyCode;
