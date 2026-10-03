//! `orr_viewstream`: a language-neutral view of a simulation (design doc
//! decision 12).
//!
//! A view written in C#, C++, GDScript or anything that can read bytes, or
//! running on another machine or engine, should not have to link Rust types
//! or read a Rust `Frame`. This crate defines what it reads instead:
//!
//! - [`format`]: the versioned, little-endian binary messages. A
//!   [`ViewFrame`] per published tick (entity records with their transform at
//!   the previous and the current tick, look, kind, custom properties) and an
//!   [`EventBatch`] (sim events with the 3 states predicted, verified and
//!   canceled and their deterministic key). Byte layouts: `docs/view-stream.md`.
//!   Format version 2 adds the 3D frame, a [`ViewFrame3`] (position, quaternion,
//!   shape sizes and material per entity); version 1 messages are unchanged.
//! - [`Schema`]: the JSON message that says once per connection what the game
//!   is: its entity kinds, the byte layout of its input, its event types.
//! - [`source`]: the producer. It runs the game's `orr_view::Extractor` on the
//!   host side, on the predicted frame and the one before it, and is where
//!   `FP` becomes `f32` for the stream. [`ViewStreamSource`] drives it from any
//!   `orr_bridge::Bridge`; [`StreamProducer`] serves hosts that hold frames
//!   directly (the ERP server's `viewstream` topic, the C ABI of `orr_ffi`).
//!
//! Only [`format`] is needed to read a stream. The rest (`schema`, `source`) is the
//! producer and sits behind the `producer` feature (on by default); a reader that must
//! not link the simulation, like `orr_tui`, turns default features off.
//!
//! The view layer, not the sim, owns floats: this is a view-boundary crate.
#![allow(clippy::float_arithmetic)]
#![allow(clippy::disallowed_types)]

pub mod format;
#[cfg(feature = "producer")]
pub mod schema;
#[cfg(feature = "producer")]
pub mod source;

pub use format::{
    color_to_u8, message_type, reset_frame_events, DecodeError, EntityRecord, EntityRecord3, EventBatch, EventRecord, Pose3, ViewFrame, ViewFrame3,
    EVENT_BATCH_HEADER_LEN, EVENT_HEAD_LEN, FLAG_DISCONTINUITY, FLAG_EVENTS_RESET, FLAG_PAUSED, FLAG_ROLLED_BACK, HEADER_LEN, MAGIC, MAX_VERSION,
    MODE_NONE, MODE_PREDICTION, MODE_SNAPSHOT, MSG_EVENTS, MSG_FRAME, MSG_FRAME3D, RECORD3D_LEN, RECORD_LEN, SHAPE3_BOX,
    SHAPE3_CAPSULE, SHAPE3_PLANE, SHAPE3_SPHERE, SHAPE_CAPSULE, SHAPE_CIRCLE, SHAPE_QUAD, STATE_CANCELED, STATE_PREDICTED,
    STATE_VERIFIED, STYLE_CHECKER, VERSION, VERSION_3D,
};
#[cfg(feature = "producer")]
pub use schema::{input_layout_of, EventDef, InputLayout, KindDef, PropDef, PropType, Schema};
#[cfg(feature = "producer")]
pub use source::{
    entity_id, FrameEncoder, FrameEncoder3, FrameMeta, NoKinds, NoKinds3, Pumped, Pumped3, StreamKinds, StreamKinds3, StreamProducer,
    ViewStreamSource, ViewStreamSource3,
};
