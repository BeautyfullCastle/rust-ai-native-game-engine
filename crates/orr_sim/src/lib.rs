//! `orr_sim`: the deterministic simulation layer of the Orrery engine.
//!
//! A [`Game`] bundles the fixed set of component/singleton/list types it
//! needs (via [`Game::register`]), an initial-state setup function, and a
//! fixed, ordered list of [`System`]s. [`Simulation<G>`] owns one
//! [`orr_ecs::Frame`] and steps it forward one tick at a time with
//! [`Simulation::step`], given that tick's [`TickInputs`].
//!
//! Nothing in this crate touches floating point, wall-clock time, or
//! non-deterministic hashing (`#![deny(clippy::disallowed_types)]`,
//! `#![deny(clippy::float_arithmetic)]`); the only numeric type used is
//! [`orr_fp::FP`], and the only source of randomness is
//! [`orr_fp::FrameRng`], stored *inside* the `Frame` as a singleton so it
//! rolls back with everything else.
#![deny(clippy::disallowed_types)]
#![deny(clippy::float_arithmetic)]

mod command;
mod context;
mod event;
mod game;
mod hotpatch;
mod input;
mod simulation;
mod system;

pub use command::{decode_pod, encode_pod, SimCommand};
pub use context::SimContext;
pub use event::{EventKey, SimEvent};
pub use game::Game;
pub use hotpatch::{DirectCall, HotPatchHook};
pub use input::{PlayerFlags, PlayerSlot, SimInput, TickInputs};
pub use simulation::{build_hash_of, Simulation};
pub use system::System;

// Re-exported so games don't need a direct `orr_ecs`/`orr_fp` dependency
// just to write systems.
pub use orr_ecs::{Commands, Component, ComponentRegistryBuilder, Entity, Frame};
pub use orr_fp::{fp, FPVec2, FPVec3, FrameRng, FP};
