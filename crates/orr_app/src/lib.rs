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
mod audio;
mod cli;
mod font;
mod gamepad;
mod glyphs;
mod input;
mod models;
mod net;
mod overlay;
mod pace;
mod save;
mod screens;
mod shot;
mod typeset;
mod view2d;
mod view3d;
mod views;
mod wav;

pub use app::{App, Error};
pub use audio::{Audio, SoundHandle, SoundOut};
pub use save::{LoadedSave, SaveEntry, SaveError, SaveInfo, SaveNotice, SaveStore, MAX_SAVE_BYTES, NOTICE_FRAMES, SAVE_VERSION};
pub use wav::{clip_from_wav, WavError, MAX_WAV_BYTES};
pub use orr_audio::{AudioConfig, Clip, Listener, OfflineAudio};
pub use font::{system_font, system_font_candidates, Font, FontError, FontHandle};
pub use typeset::{Align, Anchor, ScreenRect};
pub use overlay::{text_size, LineItem, Overlay, RectItem, TextItem};
pub use gamepad::{PadAxis, PadButton, PadEvent, DEADZONE, MAX_PADS, PRESS};
pub use input::{Device, InputContext, KeyLayout, Stick};
pub use screens::{Action, ActionMap, Labels, Screen, Screens};
pub use orr_bridge::{FrameView, Lifecycle, PlayerSignal, PlayerSlot, Players, RawPlayerSignal, SignalKind};
pub use orr_render::{Camera, Camera3D, Lighting, OrbitCamera};
pub use orr_ecs::Entity;
pub use orr_view::{fp_to_f32, fp_to_vec2, Extracted, Extracted3, Extractor, Extractor3, InterpMode, RenderItem, Shape, Shape3, Style, Style3, Transform2, Transform3, Vec2};
pub use view2d::{solid_quad, View2D};
pub use orr_gfx_view::{ViewAsset, ViewAssetError, ViewAssets};
pub use view3d::{default_lighting, default_orbit, extract_default, scene_camera, Look3D, ModelDraw, View3D};
pub use views::{EntityFactory, EntityView, EventState, SceneView, ViewContext, Views};
pub use winit::keyboard::KeyCode;
