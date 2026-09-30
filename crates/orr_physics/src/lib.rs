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
//!    reference face clipping) and every pair with a capsule (see Capsules)
//!    with up to 2 contact points per pair. A
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
//! # Capsules
//!
//! [`Shape::capsule`] is a segment grown by a radius, a stadium. It lives in
//! the same `Shape` bytes as circles and polygons (`kind == SHAPE_CAPSULE`,
//! end points in `verts[0..2]`, unit axis in `normals[0]`, length in
//! `normals[1].x`), so `Body` and `Collider` keep their layout. The axis is
//! the local y axis; `Body::angle` turns it. The broad phase box is the
//! segment box grown by the radius. Mass is `density * (2h * 2r + pi r^2)`
//! with the exact inertia of the stadium.
//!
//! Narrow phase, all in `FP` with exact 128-bit dot products for the
//! closest points of segments:
//!
//! - capsule/circle: the circle against the closest point of the segment.
//! - capsule/capsule: closest points of the two segments give the normal
//!   and one point. If the axes are within about 6 degrees of parallel and
//!   overlap along the axis, the overlap range is clipped to two points, so
//!   a capsule rests on its side on another capsule.
//! - capsule/polygon: if the segment is apart from the polygon, the exact
//!   distance to the polygon boundary gives the normal and one point (so a
//!   round end rolls over a corner without a false contact). A segment
//!   within 6 degrees of a polygon edge and over it gets two points from
//!   the clipped overlap. If the segment reaches into the polygon, the
//!   axis of least penetration among the polygon faces and the segment
//!   normal decides, as in SAT.
//! - A sensor capsule uses the same routines with margin 0. Sensors are
//!   exact for every shape pair.
//!
//! # Queries
//!
//! [`raycast`] supports every shape. [`shape_cast`] sweeps a circle, capsule
//! or convex polygon along a direction with fixed rotation and returns the
//! first hit: entity, time of impact as a fraction and distance, a contact
//! point and the normal. It is exact, not iterated. The ray of the caster
//! origin is cast against the Minkowski sum of the target and the mirrored
//! caster (a convex polygon of at most 16 vertices, found with a monotone
//! chain hull), grown by both radii: one plane per edge, offset by the
//! radius, and one circle per vertex. The distance is accurate to a few
//! 1/65536 units (rounding of the square roots and divisions); the tests
//! compare it with an independent distance function on random pairs at
//! 0.006 units. There is no conservative advancement step count, so there
//! is nothing to tune and no tunnelling at any speed. Touching counts as a
//! hit; a caster that already overlaps reports fraction 0; ties in
//! distance go to the lower entity index. [`circle_cast`] keeps its
//! own algorithm. [`move_and_slide_capsule`] is the capsule version of
//! [`move_and_slide`], with slope limit, skin, step up, ground snapping and
//! ground detection, built on [`shape_cast`].
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
//! - capsules: `radius >= 0.05` and `half_length + radius <= 1_000` (so a
//!   capsule is at most 2 000 units long and its segment length is at most
//!   2 000), with any aspect ratio inside those limits. The segment
//!   geometry multiplies raw coordinates in `i128`, which is exact up to
//!   these sizes and world positions up to 30 000 units. The test
//!   `capsule_range_limits_stay_inside_the_solver_ranges` runs both
//!   extremes (a 10 000 mass capsule of 900 x 100 and a 2 000 long rod,
//!   next to 0.001 mass capsules) at the speed limit in debug builds,
//! - shape casts and rays: origin, direction and the swept box inside the
//!   30 000 unit world, cast distance up to 30 000 (the Minkowski hull
//!   adds the two shapes' extents, at most 4 000),
//! - speeds `<= max_linear_speed` (default 500 units/s, enforced for
//!   dynamic bodies), angular speed `<= max_angular_speed`,
//! - no continuous collision detection: a body faster than
//!   `shape size * 60` units/s can tunnel through thin shapes (queries do
//!   not tunnel).
//!
//! Precision is 1/65536 unit, so inverse masses below about `0.0005` lose
//! most of their bits. The hot paths multiply in `i64` and fall back to
//! `i128` on overflow, so results never depend on the range, only speed.
//!
//! # Not included yet
//!
//! Continuous collision detection for bodies (shape casts exist for
//! queries only), joints, rounded polygons, casts with a rotating caster.
#![deny(clippy::disallowed_types)]
#![deny(clippy::float_arithmetic)]
#![warn(missing_docs)]

mod capsule;
mod cast;
mod collide;
mod fastmath;
mod geom;
mod query;
mod reflection;
mod sleep;
mod solver;
mod step;
mod system;
mod types;

pub use query::{
    circle_cast, circle_cast_ignoring, move_and_slide, move_and_slide_capsule, raycast, shape_cast, CapsuleCharacterParams,
    CharacterMove, CharacterParams, QueryFilter, RayHit, ShapeHit,
};
pub use reflection::register_reflect;
pub use sleep::{apply_impulse, is_asleep, set_velocity, wake, wake_all};
pub use step::{step, step_probed, Phase, Scratch, StepStats};
pub use system::PhysicsSystem;
pub use types::{
    Body, Collider, ContactCache, MassData, OverlapPair, PhysicsConfig, PhysicsState, Shape, TriggerEvent, BODY_DYNAMIC,
    BODY_KINEMATIC, BODY_STATIC, COLLIDER_SENSOR, MAX_POLY_VERTS, SHAPE_CAPSULE, SHAPE_CIRCLE, SHAPE_POLYGON, SLEEP_FLAG, TRIGGER_ENTER,
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
