//! Joints between two bodies, solved with the contacts in every substep: a
//! ball joint holds two anchor points together (the bodies turn freely about
//! it), a rod keeps them a fixed distance apart, and a rope keeps them at most
//! that far apart. Joints are an opt-in component: a game that never calls
//! [`register_joints`] steps exactly as before.

use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Entity, Frame};
use orr_fp::{FPMat3, FPQuat, FPVec3, FP};

use crate::solver::{BodyInv, Vw};
use crate::types::Body;

/// [`Joint::kind`]: the two anchors stay together (a ball joint).
pub const JOINT_BALL: u32 = 0;
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
    pub anchor_a: FPVec3,
    /// Anchor on `b`, in `b`'s own frame.
    pub anchor_b: FPVec3,
    /// Distance between the anchors for a rod or rope; unused by a ball joint.
    pub length: FP,
    /// [`JOINT_BALL`], [`JOINT_ROD`] or [`JOINT_ROPE`].
    pub kind: u32,
    /// Explicit padding, keep 0.
    pub _pad: u32,
}

/// Registers [`Joint`]. Call it after [`crate::register`] in a game that uses joints.
pub fn register_joints(builder: &mut ComponentRegistryBuilder) {
    builder.register_component::<Joint>("orr_physics3d::Joint");
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

/// Share of the remaining position error a joint removes per substep.
const BAUMGARTE: FP = FP::from_ratio(2, 10);

/// One joint of this tick. The pose terms are refreshed at every substep.
#[derive(Clone, Copy)]
pub(crate) struct JointRow {
    a: usize,
    b: usize,
    kind: u32,
    local_a: FPVec3,
    local_b: FPVec3,
    length: FP,
    pos_a: FPVec3,
    pos_b: FPVec3,
    rot_a: FPQuat,
    rot_b: FPQuat,
    ima: FP,
    imb: FP,
    iia: FPMat3,
    iib: FPMat3,
    // Refreshed per substep.
    active: bool,
    ra: FPVec3,
    rb: FPVec3,
    /// Ball: the inverse of the effective mass matrix.
    k_inv: FPMat3,
    /// Ball: the position error as a velocity to remove.
    bias: FPVec3,
    axis: FPVec3,
    /// Rod and rope: one over the effective mass along `axis`, and the bias along it.
    mass: FP,
    bias_axis: FP,
    /// Rope: the impulse this substep, which may only pull.
    total: FP,
}

/// Body index of `e` in the step's ascending entity list.
fn slot(ents: &[Entity], e: Entity) -> Option<usize> {
    ents.binary_search_by_key(&e.index, |x| x.index).ok().filter(|&i| ents[i] == e)
}

/// The joints of `frame` between gathered bodies, in joint entity order.
/// `held` marks every body a joint holds.
pub(crate) fn prepare(frame: &Frame, ents: &[Entity], bslot: &[u32], bd: &[Body], binv: &[BodyInv], held: &mut Vec<bool>) -> Vec<JointRow> {
    held.clear();
    held.resize(ents.len(), false);
    if frame.registry().component_id::<Joint>().is_none() {
        return Vec::new();
    }
    let (je, jd) = frame.dense::<Joint>();
    let mut order: Vec<usize> = (0..je.len()).collect();
    order.sort_unstable_by_key(|&k| je[k].index);
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
        rows.push(JointRow {
            a,
            b,
            kind: joint.kind,
            local_a: joint.anchor_a,
            local_b: joint.anchor_b,
            length: joint.length,
            pos_a: ba.pos,
            pos_b: bb.pos,
            rot_a: ba.rot,
            rot_b: bb.rot,
            ima: binv[a].inv_mass,
            imb: binv[b].inv_mass,
            iia: binv[a].inv_inertia,
            iib: binv[b].inv_inertia,
            active: false,
            ra: FPVec3::ZERO,
            rb: FPVec3::ZERO,
            k_inv: FPMat3::ZERO,
            bias: FPVec3::ZERO,
            axis: FPVec3::ZERO,
            mass: FP::ZERO,
            bias_axis: FP::ZERO,
            total: FP::ZERO,
        });
    }
    rows
}

/// `I^-1 * [r]x * [r]x^T`-style term: `-[r]x I^-1 [r]x`.
fn arm(inv_inertia: &FPMat3, r: FPVec3) -> FPMat3 {
    let s = FPMat3::skew(r);
    let t = s.mul_mat(inv_inertia).mul_mat(&s);
    FPMat3::from_cols(-t.col(0), -t.col(1), -t.col(2))
}

fn add(a: &FPMat3, b: &FPMat3) -> FPMat3 {
    FPMat3::from_cols(a.col(0) + b.col(0), a.col(1) + b.col(1), a.col(2) + b.col(2))
}

/// Refreshes the anchors, errors and effective masses at the start of a
/// substep, from the positions moved so far (`pw`) and the substep length `h`.
pub(crate) fn begin_substep(rows: &mut [JointRow], pw: &[Vw], h: FP, rotate: fn(FPQuat, FPVec3, FP) -> FPQuat) {
    let inv_h = FP::ONE / h;
    for row in rows.iter_mut() {
        let rot_a = if pw[row.a].w == FPVec3::ZERO { row.rot_a } else { rotate(row.rot_a, pw[row.a].w, FP::ONE) };
        let rot_b = if pw[row.b].w == FPVec3::ZERO { row.rot_b } else { rotate(row.rot_b, pw[row.b].w, FP::ONE) };
        row.ra = rot_a.rotate_vec3(row.local_a);
        row.rb = rot_b.rotate_vec3(row.local_b);
        let gap = (row.pos_b + pw[row.b].v + row.rb) - (row.pos_a + pw[row.a].v + row.ra);
        row.total = FP::ZERO;
        row.active = true;
        if row.kind == JOINT_BALL {
            let m = row.ima + row.imb;
            let k = add(&add(&FPMat3::from_diag(FPVec3::new(m, m, m)), &arm(&row.iia, row.ra)), &arm(&row.iib, row.rb));
            match k.inverse() {
                Some(inverse) => row.k_inv = inverse,
                None => row.active = false,
            }
            row.bias = gap * (BAUMGARTE * inv_h);
            continue;
        }
        let distance = gap.length();
        let error = distance - row.length;
        if distance <= FP::from_ratio(1, 1000) || (row.kind == JOINT_ROPE && error <= FP::ZERO) {
            row.active = false;
            continue;
        }
        row.axis = gap / distance;
        let (rna, rnb) = (row.ra.cross(row.axis), row.rb.cross(row.axis));
        let k = row.ima + row.imb + rna.dot(row.iia.mul_vec(rna)) + rnb.dot(row.iib.mul_vec(rnb));
        if k <= FP::ZERO {
            row.active = false;
            continue;
        }
        row.mass = FP::ONE / k;
        row.bias_axis = error * BAUMGARTE * inv_h;
    }
}

/// One iteration over every active joint.
pub(crate) fn solve(rows: &mut [JointRow], vw: &mut [Vw]) {
    for row in rows.iter_mut().filter(|r| r.active) {
        let (mut va, mut vb) = (vw[row.a], vw[row.b]);
        let relative = (vb.v + vb.w.cross(row.rb)) - (va.v + va.w.cross(row.ra));
        let impulse = if row.kind == JOINT_BALL {
            row.k_inv.mul_vec(-(relative + row.bias))
        } else {
            let mut lambda = -(row.mass * (relative.dot(row.axis) + row.bias_axis));
            if row.kind == JOINT_ROPE {
                // A rope pulls the anchors together and never pushes them apart.
                let total = (row.total + lambda).min(FP::ZERO);
                lambda = total - row.total;
                row.total = total;
            }
            row.axis * lambda
        };
        va.v -= impulse * row.ima;
        va.w -= row.iia.mul_vec(row.ra.cross(impulse));
        vb.v += impulse * row.imb;
        vb.w += row.iib.mul_vec(row.rb.cross(impulse));
        vw[row.a] = va;
        vw[row.b] = vb;
    }
}
