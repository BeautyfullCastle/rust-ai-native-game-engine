//! `orr_physics`: deterministic 2D rigid body physics on `orr_fp::FP`.
//!
//! # State layout (all in the `Frame`)
//!
//! - [`Body`] and [`Collider`]: Pod components, one pair per physical entity.
//!   `Body::sleep` and `Body::island` hold the sleep state.
//! - [`PhysicsState`]: singleton with the [`PhysicsConfig`] and two
//!   `FrameList` handles.
//! - `FrameList<ContactCache>`: warm-starting impulses, sorted by
//!   `(entity index a, entity index b, feature id)`.
//! - `FrameList<OverlapPair>`: current trigger overlaps, sorted by entity
//!   index. Enter/exit events come from diffing this set each tick.
//!
//! Snapshot, rollback, checksum and `Frame::to_bytes` therefore cover the
//! whole physics state. [`Scratch`] holds only per-tick temporaries that
//! are rebuilt from the frame on every step. It may keep the previous
//! sweep order as a sort hint, but no result depends on it.
//!
//! # Pipeline (one [`step`])
//!
//! 1. Gather bodies in ascending entity index order, classify them
//!    (dynamic, asleep, moving) and apply the wake rules that need no
//!    contacts (see Sleeping).
//! 2. Broad phase: sort and sweep on the x axis, pair list sorted by index.
//!    A solid pair needs one dynamic body and one body that moves this
//!    tick, so pairs of sleeping, static and idle bodies cost nothing.
//! 3. Narrow phase: circle/circle, circle/polygon, polygon/polygon (SAT,
//!    reference face clipping) with up to 2 contact points per pair. A
//!    contact between a moving body and a sleeping one wakes the sleeper's
//!    island, and steps 2 and 3 run again (at most 3 times).
//! 4. Sensor pairs: exact overlap test, enter/exit events. Sensors always
//!    see sleeping bodies.
//! 5. Semi-implicit Euler for velocities, warm start, fixed count of
//!    sequential impulse iterations (friction then normal), Baumgarte
//!    position correction with slop, restitution above a closing speed.
//! 6. Sleep timers and islands, then integrate positions of the moving
//!    bodies, write back, store the contact cache.
//!
//! # Sleeping
//!
//! A dynamic body is *still* when its speed is below
//! `PhysicsConfig::sleep_linear_speed` and its angular speed below
//! `sleep_angular_speed`. Its timer counts consecutive still ticks. The
//! bodies linked by contacts between dynamic bodies form an island (static
//! and kinematic bodies do not link islands). When every body of an island
//! has a timer of `sleep_ticks`, the whole island falls asleep in the same
//! tick: its velocities become zero and each body stores the island id.
//! `sleep_ticks == 0` turns sleeping off.
//!
//! A sleeping body is skipped by the solver and by integration, but still
//! collides. The rules that wake it, all deterministic functions of the
//! frame:
//!
//! - a contact with a body that moves this tick: an awake dynamic body, or
//!   a kinematic body with a velocity (the timer is kept, so an island that
//!   is still after the touch sleeps again soon);
//! - [`apply_impulse`], [`set_velocity`] or [`wake`], or any write of a
//!   non-zero velocity into `Body::vel` / `Body::omega`;
//! - despawn of a body that supported it: the cached contact of the two
//!   bodies names an entity that no longer exists.
//!
//! Every wake takes the whole island with it. Position or shape edits by
//! game code are not detected: call [`wake`] after them. Changing gravity
//! needs [`wake_all`].
//!
//! The contact cache entries of sleeping pairs are kept, so a woken island
//! restarts with its old impulses.
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
//! most of their bits. The hot paths multiply in `i64` and fall back to
//! `i128` on overflow, so results never depend on the range, only speed.
//!
//! # Not included yet
//!
//! Capsule shapes, capsule character controller (only a circle one,
//! [`move_and_slide`]), general shape casts (only [`circle_cast`]),
//! joints.
#![deny(clippy::disallowed_types)]
#![deny(clippy::float_arithmetic)]
#![warn(missing_docs)]

mod collide;
mod fastmath;
mod query;
mod sleep;
mod solver;
mod step;
mod system;
mod types;

pub use query::{
    circle_cast, circle_cast_ignoring, move_and_slide, raycast, CharacterMove, CharacterParams, QueryFilter, RayHit,
};
pub use sleep::{apply_impulse, is_asleep, set_velocity, wake, wake_all};
pub use step::{step, step_probed, Phase, Scratch, StepStats};
pub use system::PhysicsSystem;
pub use types::{
    Body, Collider, ContactCache, MassData, OverlapPair, PhysicsConfig, PhysicsState, Shape, TriggerEvent, BODY_DYNAMIC,
    BODY_KINEMATIC, BODY_STATIC, COLLIDER_SENSOR, MAX_POLY_VERTS, SHAPE_CIRCLE, SHAPE_POLYGON, SLEEP_FLAG, TRIGGER_ENTER,
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
