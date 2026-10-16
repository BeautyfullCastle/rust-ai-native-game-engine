//! `orr_ecs`: a deterministic, rollback-friendly ECS for the Orrery engine.
//!
//! The whole simulation state lives in a [`Frame`], built from a shared,
//! immutable [`ComponentRegistry`]. Components are sparse-set stored
//! (cheap structural changes, good for resimulation-heavy workloads);
//! archetype tables may be added later as an alternative storage backend if
//! iteration-heavy workloads need it, but M0 deliberately keeps things
//! simple. Every `Frame` can be snapshotted (`Clone`/`copy_from`, reusing
//! allocations) and checksummed (`checksum`), which is what rollback netcode
//! needs to resimulate a handful of ticks per render frame and detect
//! desyncs across peers.
//!
//! No floating point appears anywhere in this crate, and nothing hashed or
//! serialized uses `usize` — both are required for cross-platform
//! determinism.
#![deny(clippy::disallowed_types)]

mod codec;
mod commands;
mod component;
mod entity;
mod frame;
mod list;
mod query;
mod registry;
mod ring;
mod singleton;
mod store;

pub use codec::FrameDecodeError;
pub use commands::{Commands, EntityRef, PendingEntity};
pub use component::{Component, ComponentId, ListId, SingletonId};
pub use entity::{Entity, EntityAllocator};
pub use frame::Frame;
pub use list::FrameList;
pub use query::{QueryFetch, QueryIter, QueryTuple};
pub use registry::{ComponentRegistry, ComponentRegistryBuilder};
pub use ring::FrameRing;

/// Current ORRF snapshot format and frame-checksum version.
///
/// The checksum covers the encoded body, including this version. Peers and
/// recorded frame checksums from different versions are incompatible; game
/// build identities should incorporate it when guarding that boundary.
pub const FRAME_FORMAT_VERSION: u32 = codec::FORMAT_VERSION;
