//! Joints between two bodies, solved with the contacts in the sequential
//! impulse iterations: a pin holds two anchor points together (a hinge), a
//! rod keeps them a fixed distance apart, and a rope keeps them at most that
//! far apart. Joints are an opt-in component: a game that never calls
//! [`register_joints`] steps exactly as before.

use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Entity, Frame};
use orr_fp::{FPVec2, FP};

use crate::collide::Xf;
use crate::solver::Vw;
use crate::types::Body;

/// [`Joint::kind`]: the two anchors stay together; the bodies turn freely about them.
pub const JOINT_PIN: u32 = 0;
/// [`Joint::kind`]: the two anchors stay [`Joint::length`] apart.
pub const JOINT_ROD: u32 = 1;
/// [`Joint::kind`]: the two anchors stay at most [`Joint::length`] apart.
pub const JOINT_ROPE: u32 = 2;

/// A joint between bodies `a` and `b`, as a component on an entity of its own.
/// Both bodies need a [`crate::Collider`] to take part in the step.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Joint {
    /// The first body.
    pub a: Entity,
    /// The second body.
    pub b: Entity,
    /// Anchor on `a`, in `a`'s own frame (relative to its center of mass, unrotated).
    pub anchor_a: FPVec2,
    /// Anchor on `b`, in `b`'s own frame.
    pub anchor_b: FPVec2,
    /// Distance between the anchors for a rod or rope; unused by a pin.
    pub length: FP,
    /// [`JOINT_PIN`], [`JOINT_ROD`] or [`JOINT_ROPE`].
    pub kind: u32,
    /// Explicit padding, keep 0.
    pub _pad: u32,
}

/// Registers [`Joint`]. Call it after [`crate::register`] in a game that uses joints.
pub fn register_joints(builder: &mut ComponentRegistryBuilder) {
    builder.register_component::<Joint>("orr_physics::Joint");
}

/// Spawns an entity holding `joint` and wakes both bodies. Bodies held by a
/// joint do not fall asleep.
pub fn add_joint(frame: &mut Frame, joint: Joint) -> Entity {
    let e = frame.spawn();
    frame.add(e, joint);
    crate::wake(frame, joint.a);
    crate::wake(frame, joint.b);
    e
}

/// Share of the remaining position error a joint removes per tick.
const BAUMGARTE: FP = FP::from_ratio(2, 10);

/// One joint prepared for this tick's iterations.
#[derive(Clone, Copy)]
pub(crate) struct JointRow {
    a: usize,
    b: usize,
    kind: u32,
    ra: FPVec2,
    rb: FPVec2,
    ima: FP,
    iia: FP,
    imb: FP,
    iib: FP,
    /// Pin: the position error as a velocity to remove. Rod and rope: along `axis`.
    bias: FPVec2,
    axis: FPVec2,
    /// Rod and rope: one over the effective mass along `axis`.
    mass: FP,
    /// Rope: the impulse so far, which may only pull.
    total: FP,
}

fn rotate(xf: &Xf, v: FPVec2) -> FPVec2 {
    FPVec2::new(xf.c * v.x - xf.s * v.y, xf.s * v.x + xf.c * v.y)
}

/// `w x r` for a scalar angular velocity.
fn spin(w: FP, r: FPVec2) -> FPVec2 {
    FPVec2::new(-(w * r.y), w * r.x)
}

/// Body index of `e` in the step's ascending entity list.
fn slot(ents: &[Entity], e: Entity) -> Option<usize> {
    ents.binary_search_by_key(&e.index, |x| x.index).ok().filter(|&i| ents[i] == e)
}

/// The joints of `frame` between gathered bodies, ready to solve, in joint
/// entity order. `held` marks every body a joint holds.
pub(crate) fn prepare(frame: &Frame, ents: &[Entity], bslot: &[u32], bd: &[Body], xfs: &[Xf], dt: FP, held: &mut Vec<bool>) -> Vec<JointRow> {
    held.clear();
    held.resize(ents.len(), false);
    if frame.registry().component_id::<Joint>().is_none() {
        return Vec::new();
    }
    let (je, jd) = frame.dense::<Joint>();
    let mut order: Vec<usize> = (0..je.len()).collect();
    order.sort_unstable_by_key(|&k| je[k].index);
    let inv_dt = FP::ONE / dt;
    let mut rows = Vec::with_capacity(order.len());
    for k in order {
        let joint = jd[k];
        let (Some(a), Some(b)) = (slot(ents, joint.a), slot(ents, joint.b)) else { continue };
        if a == b {
            continue;
        }
        held[a] = true;
        held[b] = true;
        let (ba, bb) = (&bd[bslot[a] as usize], &bd[bslot[b] as usize]);
        let (ra, rb) = (rotate(&xfs[a], joint.anchor_a), rotate(&xfs[b], joint.anchor_b));
        let gap = (xfs[b].p + rb) - (xfs[a].p + ra);
        let mut row = JointRow {
            a,
            b,
            kind: joint.kind,
            ra,
            rb,
            ima: ba.inv_mass,
            iia: ba.inv_inertia,
            imb: bb.inv_mass,
            iib: bb.inv_inertia,
            bias: gap * (BAUMGARTE * inv_dt),
            axis: FPVec2::ZERO,
            mass: FP::ZERO,
            total: FP::ZERO,
        };
        if joint.kind != JOINT_PIN {
            let distance = gap.length();
            if distance <= FP::from_ratio(1, 1000) {
                continue;
            }
            let error = distance - joint.length;
            if joint.kind == JOINT_ROPE && error <= FP::ZERO {
                continue;
            }
            row.axis = gap / distance;
            let (rna, rnb) = (ra.perp_dot(row.axis), rb.perp_dot(row.axis));
            let k = row.ima + row.imb + row.iia * rna * rna + row.iib * rnb * rnb;
            if k <= FP::ZERO {
                continue;
            }
            row.mass = FP::ONE / k;
            row.bias = FPVec2::new(error * BAUMGARTE * inv_dt, FP::ZERO);
        }
        rows.push(row);
    }
    rows
}

/// One iteration over every joint.
pub(crate) fn solve(rows: &mut [JointRow], vw: &mut [Vw]) {
    for row in rows.iter_mut() {
        let (mut va, mut vb) = (vw[row.a], vw[row.b]);
        let relative = (vb.v + spin(vb.w, row.rb)) - (va.v + spin(va.w, row.ra));
        let impulse = if row.kind == JOINT_PIN {
            let (ra, rb) = (row.ra, row.rb);
            let m = row.ima + row.imb;
            let k11 = m + row.iia * ra.y * ra.y + row.iib * rb.y * rb.y;
            let k12 = -(row.iia * ra.x * ra.y) - row.iib * rb.x * rb.y;
            let k22 = m + row.iia * ra.x * ra.x + row.iib * rb.x * rb.x;
            let det = k11 * k22 - k12 * k12;
            if det == FP::ZERO {
                continue;
            }
            let rhs = -(relative + row.bias);
            FPVec2::new((k22 * rhs.x - k12 * rhs.y) / det, (k11 * rhs.y - k12 * rhs.x) / det)
        } else {
            let mut lambda = -(row.mass * (relative.dot(row.axis) + row.bias.x));
            if row.kind == JOINT_ROPE {
                // A rope pulls the anchors together and never pushes them apart.
                let total = (row.total + lambda).min(FP::ZERO);
                lambda = total - row.total;
                row.total = total;
            }
            row.axis * lambda
        };
        va.v -= impulse * row.ima;
        va.w -= row.iia * row.ra.perp_dot(impulse);
        vb.v += impulse * row.imb;
        vb.w += row.iib * row.rb.perp_dot(impulse);
        vw[row.a] = va;
        vw[row.b] = vb;
    }
}
