//! `orr_view`: the view layer's view of the simulation (design doc 4.1).
//!
//! The view world is separate from the sim `Frame` and uses `f32`. It reads
//! sim state only through `orr_bridge` snapshots. Each frame it
//! - extracts drawable entities from the snapshot frames with a game-supplied
//!   [`Extractor`] (this is the only place `FP` becomes `f32`);
//! - shows every entity in one of three [`InterpMode`]s:
//!   `Prediction` (interpolate the last two predicted ticks; smooth away
//!   rollback corrections), `Snapshot` (play the confirmed frames a little
//!   late, so wrong guesses are never seen) or `None` (newest tick as is);
//! - keys entities by `(index, version)`, so a reused entity index is a new
//!   entity and never interpolates from the old one.
//!
//! There is no GPU code here: [`ViewWorld::render_items`] returns plain data
//! for a renderer.
//!
//! # Rollback smoothing
//!
//! When a rollback changes the past, an entity's predicted position for the
//! same tick changes. Showing the new position at once would make it jump.
//! Instead, when a snapshot arrives, each `Prediction` entity gets an *error
//! offset*: the difference between what was on screen (old track) and what
//! the new track would show at the same moment. The rendered position is
//! `interpolated + offset`, so the screen is continuous at the moment of the
//! correction. The offset then decays exponentially to zero
//! ([`ViewConfig::correction_tau`]). A correction larger than
//! [`ViewConfig::snap_distance`] (a teleport) is not smoothed.
// The view layer is where floats are allowed (design doc, decision 5).
#![allow(clippy::float_arithmetic)]
#![allow(clippy::disallowed_types)]

mod extract;
mod math;
mod world;

pub use extract::{fp_to_f32, fp_to_vec2, Extracted, Extractor, InterpMode, Shape, Style};
pub use math::{lerp_angle, Transform2, Vec2};
pub use world::{RenderItem, ViewConfig, ViewLifecycle, ViewWorld};
