//! `orr_bridge`: the only path between the simulation and the view layer
//! (design doc section 5).
//!
//! - **View to sim** has two paths only: [`Bridge::set_input`] (one `Input`
//!   sample per player, sampled once per tick) and [`Bridge::send_command`]
//!   (one-off `Command`s). Nothing else reaches the simulation.
//! - **Sim to view** has three read-only channels:
//!   [`Snapshot`] (immutable published frames, read through [`FrameView`]),
//!   [`BridgeEvent::Sim`] (sim events with the 3-state reconciliation of
//!   `orr_session`: predicted / verified / canceled) and
//!   [`BridgeEvent::Lifecycle`] (session started, rollback, stall).
//!
//! Two transport adapters implement the same [`Bridge`] trait:
//! [`InProc`] (the caller drives the sim on the calling thread) and
//! [`Threaded`] (the sim runs on its own thread; the view reads the newest
//! snapshot without a lock; inputs, commands and events cross by channel and
//! never block the view thread).
//!
//! This crate contains no floating point. Conversion from `FP` to `f32`
//! happens in the view layer (`orr_view`).
#![deny(clippy::float_arithmetic)]
// The bridge is not a sim crate: `Threaded` paces its own thread with the wall clock.
#![allow(clippy::disallowed_types)]

mod bridge;
mod core;
mod event;
mod frame_view;
mod host;
mod inproc;
mod snapshot;
mod threaded;

pub use bridge::{Bridge, BridgeConfig, BridgeError, StepObserver, StepTiming};
pub use event::{BridgeEvent, BridgeStats, Lifecycle};
pub use frame_view::FrameView;
pub use host::{LoopbackPair, SimHost};
pub use inproc::InProc;
pub use snapshot::Snapshot;
pub use threaded::{Pacing, Threaded, ThreadedConfig};

pub use orr_session::{EventStatus, RollbackInfo};
pub use orr_sim::{EventKey, PlayerSlot};
