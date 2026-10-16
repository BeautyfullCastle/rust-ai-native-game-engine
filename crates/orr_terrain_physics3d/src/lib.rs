//! Frame-owned deterministic sphere contacts for one bounded static heightfield.
//!
//! External canonical assets are resolved only at admission. Physics reads the
//! registered frame state, so snapshots and replay carry their complete terrain.
#![deny(clippy::disallowed_types)]
#![deny(clippy::float_arithmetic)]
#![warn(missing_docs)]

pub mod asset;
pub mod geometry;
pub mod step;

pub use asset::*;
pub use geometry::*;
pub use step::*;
