//! `orr_physics`: deterministic 2D rigid body physics on `orr_fp::FP`.
//!
//! # State layout (all in the `Frame`)
//!
//! - [`Body`] and [`Collider`]: Pod components, one pair per physical entity.
//! - [`PhysicsState`]: singleton with the [`PhysicsConfig`] and two
//!   `FrameList` handles.
//! - `FrameList<ContactCache>`: warm-starting impulses, sorted by
//!   `(entity index a, entity index b, feature id)`.
//! - `FrameList<OverlapPair>`: current trigger overlaps, sorted by entity
//!   index. Enter/exit events come from diffing this set each tick.
//!
//! Snapshot, rollback, checksum and `Frame::to_bytes` therefore cover the
//! whole physics state. [`Scratch`] holds only per-tick temporaries that
//! are rebuilt from the frame on every step.
//!
//! # Pipeline (one [`step`])
//!
//! 1. Gather bodies in ascending entity index order.
//! 2. Broad phase: sort and sweep on the x axis, pair list sorted by index.
//! 3. Narrow phase: circle/circle, circle/polygon, polygon/polygon (SAT,
//!    reference face clipping) with up to 2 contact points per pair.
//! 4. Sensor pairs: exact overlap test, enter/exit events.
//! 5. Semi-implicit Euler for velocities, warm start, fixed count of
//!    sequential impulse iterations (friction then normal), Baumgarte
//!    position correction with slop, restitution above a closing speed.
//! 6. Integrate positions, write back, store the contact cache.
//!
//! # Numeric ranges (Q48.16)
//!
//! `FP` operators multiply through `i128`, so products cannot overflow.
//! Sums and the final `i64` raw value can. Keep these limits (checked in
//! the debug test run):
//!
//! - body positions `|x|, |y| <= 30_000` units,
//! - shape sizes `0.05 ..= 1_000`; density and mass so that
//!   `0.001 <= mass <= 10_000`,
//! - speeds `<= max_linear_speed` (default 500 units/s, enforced for
//!   dynamic bodies), angular speed `<= max_angular_speed`,
//! - no continuous collision detection: a body faster than
//!   `shape size * 60` units/s can tunnel through thin shapes.
//!
//! Precision is 1/65536 unit, so inverse masses below about `0.0005` lose
//! most of their bits.
//!
//! # Not included yet
//!
//! Capsule shapes, capsule character controller (only a circle one,
//! [`move_and_slide`]), sleeping/islands, general shape casts (only
//! [`circle_cast`]), joints.
#![deny(clippy::disallowed_types)]
#![deny(clippy::float_arithmetic)]
#![warn(missing_docs)]

mod collide;
mod query;
mod step;
mod system;
mod types;

pub use query::{
    circle_cast, circle_cast_ignoring, move_and_slide, raycast, CharacterMove, CharacterParams, QueryFilter, RayHit,
};
pub use step::{step, Scratch};
pub use system::PhysicsSystem;
pub use types::{
    Body, Collider, ContactCache, MassData, OverlapPair, PhysicsConfig, PhysicsState, Shape, TriggerEvent, BODY_DYNAMIC,
    BODY_KINEMATIC, BODY_STATIC, COLLIDER_SENSOR, MAX_POLY_VERTS, SHAPE_CIRCLE, SHAPE_POLYGON, TRIGGER_ENTER,
    TRIGGER_EXIT,
};

use orr_ecs::{ComponentRegistryBuilder, Entity, Frame};

/// Registers every physics type. Call from `Game::register`.
pub fn register(builder: &mut ComponentRegistryBuilder) {
    builder.register_component::<Body>("orr_physics::Body");
    builder.register_component::<Collider>("orr_physics::Collider");
    builder.register_singleton::<PhysicsState>("orr_physics::PhysicsState");
    builder.register_list::<ContactCache>("orr_physics::ContactCache");
    builder.register_list::<OverlapPair>("orr_physics::OverlapPair");
}

/// Allocates the persistent lists and stores `config`. Call once from
/// `Game::setup`, before the first [`step`].
pub fn init(frame: &mut Frame, config: PhysicsConfig) {
    let contacts = frame.alloc_list::<ContactCache>();
    let overlaps = frame.alloc_list::<OverlapPair>();
    frame.set_singleton(PhysicsState { config, contacts, overlaps });
}

/// Spawns an entity with a body and a collider.
pub fn spawn_body(frame: &mut Frame, body: Body, collider: Collider) -> Entity {
    let e = frame.spawn();
    frame.add(e, body);
    frame.add(e, collider);
    e
}
