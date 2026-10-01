//! `orr_physics3d`: deterministic 3D rigid body physics on `orr_fp::FP`.
//!
//! It mirrors what the 2D `orr_physics` crate proved: every piece of state
//! is a Pod component or a `FrameList` in the `Frame`, all arithmetic is
//! fixed point (Q48.16), every ordering is by entity index, and a step is
//! a pure function of the frame. Snapshot, rollback, checksum and
//! `Frame::to_bytes` cover the whole physics state; [`Scratch`] only holds
//! per-tick temporaries that are rebuilt from the frame.
//!
//! # State layout (all in the `Frame`)
//!
//! - [`Body`] and [`Collider`]: Pod components, one pair per physical
//!   entity. A body holds position, orientation quaternion, linear and
//!   angular velocity, inverse mass, the principal inverse inertia
//!   (body frame), damping and the sleep state.
//! - [`PhysicsState`]: singleton with the [`PhysicsConfig`] and the
//!   `FrameList<ContactCache>` handle.
//! - `FrameList<ContactCache>`: warm-starting impulses, sorted by
//!   `(entity index a, entity index b, feature id)`.
//!
//! # Shapes
//!
//! [`Shape::sphere`], [`Shape::capsule`] (axis is the local y axis) and
//! [`Shape::cuboid`] (an OBB). Spheres and capsules are segments grown by
//! a radius, so the narrow phase has three routines: segment/segment,
//! segment/box and box/box. All six pairs are supported. Convex hulls are
//! not implemented.
//!
//! # Pipeline (one [`step`])
//!
//! 1. Gather bodies in ascending entity index order, classify them
//!    (dynamic, asleep, moving) and apply the wake rules that need no
//!    contacts.
//! 2. Broad phase: sort and sweep on the axis with the largest spread of
//!    box centers; the pair list is sorted by `(lo, hi)` index. A pair
//!    needs one dynamic body and one body that moves this tick, so pairs
//!    of sleeping, static and idle bodies cost nothing.
//! 3. Narrow phase, up to 4 contact points per pair, with speculative
//!    contacts up to `contact_margin` of separation:
//!    - segment/segment: closest points with exact 128-bit dot products;
//!      two near-parallel capsules get two clipped points.
//!    - segment/box: exact distance from the segment end points and the 12
//!      box edges when apart; a SAT over the box faces and the
//!      segment-edge cross axes when the segment reaches into the box. A
//!      face-aligned normal clips the segment to the face (two points) so
//!      a capsule lies flat.
//!    - box/box: SAT over 15 axes (faces preferred over edges by a fixed
//!      tolerance), reference/incident face clipping with at most 8
//!      vertices reduced to 4 (deepest, farthest, two largest areas), or
//!      one point from the closest points of two edges.
//!
//!    A contact between a moving body and a sleeping one wakes the
//!    sleeper's island, and steps 2 and 3 run again (at most 3 times).
//! 4. Semi-implicit Euler for velocities, warm start, a fixed count of
//!    sequential impulse iterations (friction along two tangents, then
//!    normal; the sweep direction alternates), Baumgarte position
//!    correction with slop, speculative contacts and restitution above a
//!    closing speed.
//! 5. Sleep timers and islands (union-find over contacts between dynamic
//!    bodies), then integrate positions and orientations of the moving
//!    bodies, write back, store the contact cache.
//!
//! Contact points carry feature ids (clip vertex or edge, reference face)
//! that stay the same while the same features touch, so the cache keeps
//! warm starting through a resting stack. Tangent impulses are cached as
//! world vectors and projected onto the new tangent basis.
//!
//! # Queries
//!
//! [`raycast`] supports every shape. [`sphere_cast`] sweeps a sphere
//! along a direction and returns the first hit; it is exact (a ray against
//! the Minkowski sum of the target and the sphere), not iterated.
//!
//! # Sleeping
//!
//! Same rules as the 2D crate: a dynamic body is *still* when its linear
//! and angular speed are below the config limits; a group of touching
//! dynamic bodies sleeps when all of them have been still for
//! `sleep_ticks`. A sleeping body is skipped by the solver but still
//! collides. Waking: contact with a moving body, [`apply_impulse`],
//! [`set_velocity`], [`wake`], a non-zero velocity written into the body,
//! or despawn of a supporting body.
//!
//! # Numeric ranges (Q48.16)
//!
//! `FP` operators multiply through `i128`; the solver rows multiply in
//! `i64` (checked in debug builds). Keep these limits:
//!
//! - body positions `|x|, |y|, |z| <= 30_000` units,
//! - shape sizes `0.05 ..= 1_000`; mass between `0.01` and `10_000`
//!   (inverse mass above `~0.0005` keeps most of its bits),
//! - speeds `<= max_linear_speed` (default 500 units/s) and angular speed
//!   `<= max_angular_speed` per axis,
//! - no continuous collision detection: a body faster than
//!   `shape size * 60` units/s can tunnel through thin shapes (queries do
//!   not tunnel).
//!
//! Precision is 1/65536 unit, which sets the floor on slow rotation: the
//! orientation is a Q16 quaternion, so a body rotating slower than about
//! `0.002` rad/s does not turn at all (below the sleep speed anyway).
//!
//! # Not included yet
//!
//! Convex hulls, joints, continuous collision detection, sensors/triggers,
//! gyroscopic torque (angular velocity is integrated as a world-frame
//! vector, which is stable but not exact for spinning non-spherical
//! bodies), a circular friction cone (each tangent is clamped on its
//! own), shape casts of boxes and capsules.
#![deny(clippy::disallowed_types)]
#![deny(clippy::float_arithmetic)]
#![warn(missing_docs)]

mod collide;
mod fastmath;
mod geom;
mod query;
mod sleep;
mod solver;
mod step;
mod system;
#[cfg(test)]
mod tests_dbg;
mod types;

pub use query::{raycast, sphere_cast, QueryFilter, RayHit};
pub use sleep::{apply_impulse, is_asleep, set_velocity, wake, wake_all};
pub use step::{step, step_probed, Phase, Scratch, StepStats};
pub use system::PhysicsSystem;
pub use types::{
    Body, Collider, ContactCache, MassData, PhysicsConfig, PhysicsState, Shape, BODY_DYNAMIC, BODY_KINEMATIC, BODY_STATIC, SHAPE_BOX,
    SHAPE_CAPSULE, SHAPE_SPHERE, SLEEP_FLAG,
};

use orr_ecs::{ComponentRegistryBuilder, Entity, Frame};

/// Registers every physics type. Call from `Game::register`.
pub fn register(builder: &mut ComponentRegistryBuilder) {
    builder.register_component::<Body>("orr_physics3d::Body");
    builder.register_component::<Collider>("orr_physics3d::Collider");
    builder.register_singleton::<PhysicsState>("orr_physics3d::PhysicsState");
    builder.register_list::<ContactCache>("orr_physics3d::ContactCache");
}

/// Allocates the persistent list and stores `config`. Call once from
/// `Game::setup`, before the first [`step`].
pub fn init(frame: &mut Frame, config: PhysicsConfig) {
    let contacts = frame.alloc_list::<ContactCache>();
    frame.set_singleton(PhysicsState { config, contacts });
}

/// Spawns an entity with a body and a collider.
pub fn spawn_body(frame: &mut Frame, body: Body, collider: Collider) -> Entity {
    let e = frame.spawn();
    frame.add(e, body);
    frame.add(e, collider);
    e
}
