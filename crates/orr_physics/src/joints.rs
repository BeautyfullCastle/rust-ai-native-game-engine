//! Joints between two bodies, solved with the contacts in the sequential
//! impulse iterations: a pin holds two anchor points together (a hinge), a
//! rod keeps them a fixed distance apart, and a rope keeps them at most that
//! far apart. A pin can also drive its bodies' relative turn with a motor and
//! hold it between two angles, for wheels, hinges and ragdoll limbs. Joints
//! are an opt-in component: a game that never calls
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

/// [`Joint::flags`]: a pin's motor drives the relative turn toward [`Joint::motor_speed`].
pub const JOINT_MOTOR: u32 = 1;
/// [`Joint::flags`]: a pin holds the relative angle between [`Joint::lower`] and [`Joint::upper`].
pub const JOINT_LIMIT: u32 = 2;

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
    /// [`JOINT_MOTOR`] and [`JOINT_LIMIT`], for a pin; 0 for none.
    pub flags: u32,
    /// `b`'s angle minus `a`'s when the joint was added; [`add_joint`] records it.
    /// The limits are measured from it.
    pub ref_angle: FP,
    /// Lowest relative angle in radians, above `-pi`, with [`JOINT_LIMIT`].
    pub lower: FP,
    /// Highest relative angle in radians, below `pi`, with [`JOINT_LIMIT`].
    pub upper: FP,
    /// Relative turn rate (`b` minus `a`) in radians per second the motor aims for.
    pub motor_speed: FP,
    /// Largest torque the motor applies; 0 lets it do nothing.
    pub max_motor_torque: FP,
}

impl Joint {
    /// A joint of `kind` with no motor and no limits.
    #[must_use]
    pub fn new(kind: u32, a: Entity, b: Entity, anchor_a: FPVec2, anchor_b: FPVec2, length: FP) -> Joint {
        Joint { a, b, anchor_a, anchor_b, length, kind, ..Joint::zeroed() }
    }

    /// The same joint with a motor turning `b` against `a` at `speed`, with at most `max_torque`.
    #[must_use]
    pub fn with_motor(self, speed: FP, max_torque: FP) -> Joint {
        Joint { flags: self.flags | JOINT_MOTOR, motor_speed: speed, max_motor_torque: max_torque, ..self }
    }

    /// The same joint held between `lower` and `upper` radians of relative turn.
    #[must_use]
    pub fn with_limits(self, lower: FP, upper: FP) -> Joint {
        Joint { flags: self.flags | JOINT_LIMIT, lower, upper, ..self }
    }
}

/// Registers [`Joint`]. Call it after [`crate::register`] in a game that uses joints.
pub fn register_joints(builder: &mut ComponentRegistryBuilder) {
    builder.register_component::<Joint>("orr_physics::Joint");
}

/// Spawns an entity holding `joint`, records the bodies' current relative
/// angle as [`Joint::ref_angle`] and wakes both bodies. Bodies held by a joint
/// do not fall asleep.
pub fn add_joint(frame: &mut Frame, mut joint: Joint) -> Entity {
    if let (Some(a), Some(b)) = (frame.get::<Body>(joint.a), frame.get::<Body>(joint.b)) {
        joint.ref_angle = (b.angle - a.angle).wrap_angle();
    }
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
    flags: u32,
    /// One over the effective mass of a turn with the anchors held together; 0 when it cannot turn.
    angular_mass: FP,
    /// Anchor impulse per unit of turn impulse that keeps the anchors together.
    couple: FPVec2,
    /// Relative angle less the reference, in `(-pi, pi]`.
    angle: FP,
    lower: FP,
    upper: FP,
    motor_speed: FP,
    /// Largest motor impulse this tick.
    motor_max: FP,
    motor_total: FP,
    lower_total: FP,
    upper_total: FP,
    inv_dt: FP,
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
            flags: if joint.kind == JOINT_PIN { joint.flags } else { 0 },
            angular_mass: FP::ZERO,
            couple: FPVec2::ZERO,
            angle: (bb.angle - ba.angle - joint.ref_angle).wrap_angle(),
            lower: joint.lower,
            upper: joint.upper,
            motor_speed: joint.motor_speed,
            motor_max: joint.max_motor_torque * dt,
            motor_total: FP::ZERO,
            lower_total: FP::ZERO,
            upper_total: FP::ZERO,
            inv_dt,
        };
        if row.flags != 0 {
            (row.angular_mass, row.couple) = turn_mass(&row);
        }
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

/// The pin's 2x2 anchor mass `K` and its coupling `c` to the turn: a turn
/// impulse comes with the anchor impulse `-K^-1 c` per unit, so the anchors
/// stay together, and its effective mass is the Schur complement of `K`.
fn turn_mass(row: &JointRow) -> (FP, FPVec2) {
    let (ra, rb) = (row.ra, row.rb);
    let m = row.ima + row.imb;
    let k11 = m + row.iia * ra.y * ra.y + row.iib * rb.y * rb.y;
    let k12 = -(row.iia * ra.x * ra.y) - row.iib * rb.x * rb.y;
    let k22 = m + row.iia * ra.x * ra.x + row.iib * rb.x * rb.x;
    let c = FPVec2::new(-(row.iia * ra.y) - row.iib * rb.y, row.iia * ra.x + row.iib * rb.x);
    let det = k11 * k22 - k12 * k12;
    let couple = if det == FP::ZERO { FPVec2::ZERO } else { -FPVec2::new((k22 * c.x - k12 * c.y) / det, (k11 * c.y - k12 * c.x) / det) };
    let k = row.iia + row.iib + c.dot(couple);
    (if k > FP::ZERO { FP::ONE / k } else { FP::ZERO }, couple)
}

/// Applies a turn impulse `lambda` (on `b`, against `a`) with its anchor impulse.
fn turn(row: &JointRow, va: &mut Vw, vb: &mut Vw, lambda: FP) {
    let p = row.couple * lambda;
    va.v -= p * row.ima;
    va.w -= row.iia * (row.ra.perp_dot(p) + lambda);
    vb.v += p * row.imb;
    vb.w += row.iib * (row.rb.perp_dot(p) + lambda);
}

/// The motor, then the limits, of a pin.
fn solve_angular(row: &mut JointRow, va: &mut Vw, vb: &mut Vw) {
    if row.angular_mass == FP::ZERO {
        return;
    }
    if row.flags & JOINT_MOTOR != 0 {
        let lambda = -(row.angular_mass * (vb.w - va.w - row.motor_speed));
        let total = (row.motor_total + lambda).clamp(-row.motor_max, row.motor_max);
        let lambda = total - row.motor_total;
        row.motor_total = total;
        turn(row, va, vb, lambda);
    }
    if row.flags & JOINT_LIMIT != 0 {
        // Speculative: a bound not yet reached only stops a turn that would pass it this tick.
        let bias = |c: FP| if c < FP::ZERO { c * BAUMGARTE * row.inv_dt } else { c * row.inv_dt };
        let lambda = -(row.angular_mass * (vb.w - va.w + bias(row.angle - row.lower)));
        let total = (row.lower_total + lambda).max(FP::ZERO);
        let lambda = total - row.lower_total;
        row.lower_total = total;
        turn(row, va, vb, lambda);
        let lambda = -(row.angular_mass * (va.w - vb.w + bias(row.upper - row.angle)));
        let total = (row.upper_total + lambda).max(FP::ZERO);
        let lambda = total - row.upper_total;
        row.upper_total = total;
        turn(row, va, vb, -lambda);
    }
}

/// One iteration over every joint.
pub(crate) fn solve(rows: &mut [JointRow], vw: &mut [Vw]) {
    for row in rows.iter_mut() {
        let (mut va, mut vb) = (vw[row.a], vw[row.b]);
        if row.flags != 0 {
            solve_angular(row, &mut va, &mut vb);
            vw[row.a] = va;
            vw[row.b] = vb;
        }
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
