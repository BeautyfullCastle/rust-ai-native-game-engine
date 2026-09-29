//! Sleep and wake helpers.
//!
//! Sleep state lives in the frame ([`Body::sleep`], [`Body::island`]), so
//! it is rolled back, checksummed and serialized with everything else.
//! The step itself puts touching groups of resting bodies to sleep and
//! wakes them on contact (see the crate docs). These helpers are for game
//! code that changes a body from outside the step.
//!
//! A sleeping body has zero velocity, so any code that writes a non-zero
//! `Body::vel` or `Body::omega` also wakes it on the next step. Position
//! or shape edits are not detected: call [`wake`] after them.

use orr_ecs::{Entity, Frame};
use orr_fp::{FPVec2, FP};

use crate::types::{Body, BODY_DYNAMIC, SLEEP_FLAG};

/// True if `e` is a sleeping body.
pub fn is_asleep(frame: &Frame, e: Entity) -> bool {
    frame.get::<Body>(e).is_some_and(|b| b.sleep & SLEEP_FLAG != 0)
}

/// Wakes `e` and, on the next step, every body of its sleep island. Its
/// sleep timer restarts. Does nothing for an entity without a body.
pub fn wake(frame: &mut Frame, e: Entity) {
    if let Some(b) = frame.get_mut::<Body>(e) {
        b.sleep = 0;
    }
}

/// Wakes every body immediately (e.g. after changing gravity).
pub fn wake_all(frame: &mut Frame) {
    let ents: Vec<Entity> = frame.query::<(&Body,)>().map(|(e, _)| e).collect();
    for e in ents {
        if let Some(b) = frame.get_mut::<Body>(e) {
            b.sleep = 0;
            b.island = 0;
        }
    }
}

/// Sets the velocity of `e` and wakes it.
pub fn set_velocity(frame: &mut Frame, e: Entity, vel: FPVec2, omega: FP) {
    if let Some(b) = frame.get_mut::<Body>(e) {
        b.vel = vel;
        b.omega = omega;
        b.sleep = 0;
    }
}

/// Applies `impulse` at the world position `point` of dynamic body `e`
/// and wakes it. Bodies that are not dynamic ignore the call.
pub fn apply_impulse(frame: &mut Frame, e: Entity, impulse: FPVec2, point: FPVec2) {
    if let Some(b) = frame.get_mut::<Body>(e) {
        if b.kind != BODY_DYNAMIC {
            return;
        }
        let r = point - b.pos;
        b.vel += impulse * b.inv_mass;
        b.omega += b.inv_inertia * r.perp_dot(impulse);
        b.sleep = 0;
    }
}
