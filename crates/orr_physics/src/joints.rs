//! Joints between two bodies, solved with the contacts in the sequential
//! impulse iterations: a pin holds two anchor points together (a hinge), a
//! rod keeps them a fixed distance apart, and a rope keeps them at most that
//! far apart. A pin can also drive its bodies' relative turn with a motor and
//! hold it between two angles, for wheels, hinges and ragdoll limbs. A
//! prismatic joint lets `b` slide along one axis of `a` and nothing else, with
//! an optional motor and limits on the slide, for pistons and sliding doors.
//! A spring is a rod that gives: it pulls the anchors toward a rest length
//! with a stiffness set by a frequency and a damping ratio.
//! Joints are an opt-in component: a game that never calls
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
/// [`Joint::kind`]: `b` slides along an axis of `a` and cannot turn against it.
pub const JOINT_PRISMATIC: u32 = 3;
/// [`Joint::kind`]: a soft rod. The anchors are pulled toward [`Joint::length`] apart like a
/// spring of frequency [`Joint::motor_speed`] in hertz and damping ratio
/// [`Joint::max_motor_torque`]. A frequency of 0 makes it as stiff as a rod.
pub const JOINT_SPRING: u32 = 4;

/// [`Joint::flags`]: a pin's motor drives the relative turn, and a prismatic joint's motor
/// the slide, toward [`Joint::motor_speed`].
pub const JOINT_MOTOR: u32 = 1;
/// [`Joint::flags`]: a pin holds the relative angle, and a prismatic joint the slide,
/// between [`Joint::lower`] and [`Joint::upper`].
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
    /// Distance between the anchors for a rod, rope or spring; the angle in radians of the slide
    /// axis in `a`'s own frame for a prismatic joint; unused by a pin.
    pub length: FP,
    /// [`JOINT_PIN`], [`JOINT_ROD`], [`JOINT_ROPE`], [`JOINT_PRISMATIC`] or [`JOINT_SPRING`].
    pub kind: u32,
    /// [`JOINT_MOTOR`] and [`JOINT_LIMIT`], for a pin or a prismatic joint; 0 for none.
    pub flags: u32,
    /// `b`'s angle minus `a`'s when the joint was added; [`add_joint`] records it.
    /// The limits are measured from it. A prismatic joint holds `b` at this relative angle.
    pub ref_angle: FP,
    /// Lowest relative angle in radians, above `-pi`, with [`JOINT_LIMIT`]. A prismatic
    /// joint's lowest slide; [`add_joint`] adds the slide at the start to it.
    pub lower: FP,
    /// Highest relative angle in radians, below `pi`, with [`JOINT_LIMIT`]. A prismatic
    /// joint's highest slide; [`add_joint`] adds the slide at the start to it.
    pub upper: FP,
    /// Relative turn rate (`b` minus `a`) in radians per second the motor aims for.
    /// A prismatic joint's speed of `b` along the axis. A spring's frequency in hertz; below 0 counts as 0.
    pub motor_speed: FP,
    /// Largest torque the motor applies, or force for a prismatic joint; 0 lets it do nothing.
    /// A spring's damping ratio: 0 never settles, 1 settles without overshoot; below 0 counts as 0.
    pub max_motor_torque: FP,
}

impl Joint {
    /// A joint of `kind` with no motor and no limits.
    #[must_use]
    pub fn new(kind: u32, a: Entity, b: Entity, anchor_a: FPVec2, anchor_b: FPVec2, length: FP) -> Joint {
        Joint { a, b, anchor_a, anchor_b, length, kind, ..Joint::zeroed() }
    }

    /// A joint that lets `b` slide along `axis`, given in `a`'s own frame, and nothing else.
    /// The anchors are the points that stay on the axis line through `anchor_a`.
    #[must_use]
    pub fn prismatic(a: Entity, b: Entity, anchor_a: FPVec2, anchor_b: FPVec2, axis: FPVec2) -> Joint {
        let axis = axis.normalize_or_zero();
        let angle = if axis == FPVec2::ZERO { FP::ZERO } else { axis.y.atan2(axis.x) };
        Joint::new(JOINT_PRISMATIC, a, b, anchor_a, anchor_b, angle)
    }

    /// A spring that pulls the anchors toward `length` apart. `hertz` is how often it would
    /// swing a mass on a fixed anchor per second, and `damping_ratio` how fast that dies out.
    #[must_use]
    pub fn spring(a: Entity, b: Entity, anchor_a: FPVec2, anchor_b: FPVec2, length: FP, hertz: FP, damping_ratio: FP) -> Joint {
        Joint { motor_speed: hertz, max_motor_torque: damping_ratio, ..Joint::new(JOINT_SPRING, a, b, anchor_a, anchor_b, length) }
    }

    /// The same joint with a motor turning `b` against `a` at `speed`, with at most `max_torque`.
    /// A prismatic joint's motor slides `b` along the axis at `speed`, with at most `max_torque` of force.
    #[must_use]
    pub fn with_motor(self, speed: FP, max_torque: FP) -> Joint {
        Joint { flags: self.flags | JOINT_MOTOR, motor_speed: speed, max_motor_torque: max_torque, ..self }
    }

    /// The same joint held between `lower` and `upper` radians of relative turn.
    /// A prismatic joint is held between those distances along the axis from where it was added.
    #[must_use]
    pub fn with_limits(self, lower: FP, upper: FP) -> Joint {
        Joint { flags: self.flags | JOINT_LIMIT, lower, upper, ..self }
    }
}

/// [`Joint::ref_angle`] of a joint that came in without [`add_joint`] (a scene file, the
/// editor's inspector). The first step records the bodies' relative angle in its place.
pub(crate) const UNRECORDED: FP = FP::from_int(100);

/// Records [`Joint::ref_angle`] of the joints that came in without [`add_joint`] from the
/// poses at the start of their first step. Joints made by [`add_joint`] are never marked.
pub(crate) fn settle(frame: &mut Frame) {
    if frame.registry().component_id::<Joint>().is_none() {
        return;
    }
    let (je, jd) = frame.dense::<Joint>();
    let pending: Vec<(Entity, Joint)> = je.iter().zip(jd).filter(|(_, j)| j.ref_angle == UNRECORDED).map(|(&e, &j)| (e, j)).collect();
    for (e, joint) in pending {
        let (Some(a), Some(b)) = (frame.get::<Body>(joint.a), frame.get::<Body>(joint.b)) else { continue };
        let angle = (b.angle - a.angle).wrap_angle();
        if let Some(stored) = frame.get_mut::<Joint>(e) {
            stored.ref_angle = angle;
        }
    }
}

/// Registers [`Joint`]. Call it after [`crate::register`] in a game that uses joints.
pub fn register_joints(builder: &mut ComponentRegistryBuilder) {
    builder.register_component::<Joint>("orr_physics::Joint");
}

/// Spawns an entity holding `joint`, records the bodies' current relative
/// angle as [`Joint::ref_angle`] and wakes both bodies. The limits of a
/// prismatic joint move by the slide the bodies have now, so this pose is 0.
/// Bodies held by a joint do not fall asleep.
pub fn add_joint(frame: &mut Frame, mut joint: Joint) -> Entity {
    if let (Some(a), Some(b)) = (frame.get::<Body>(joint.a), frame.get::<Body>(joint.b)) {
        joint.ref_angle = (b.angle - a.angle).wrap_angle();
        if joint.kind == JOINT_PRISMATIC && joint.flags & JOINT_LIMIT != 0 {
            let (xa, xb) = (Xf::new(a.pos, a.angle), Xf::new(b.pos, b.angle));
            let slide = (xb.apply(joint.anchor_b) - xa.apply(joint.anchor_a)).dot(xa.rot(local_axis(&joint)));
            joint.lower += slide;
            joint.upper += slide;
        }
    }
    let e = frame.spawn();
    frame.add(e, joint);
    crate::wake(frame, joint.a);
    crate::wake(frame, joint.b);
    e
}

/// Share of the remaining position error a joint removes per tick.
const BAUMGARTE: FP = FP::from_ratio(2, 10);

/// The soft constraint terms of a spring for one step: how much of the position error
/// becomes velocity, and how much of the impulse the row may use and keep.
#[derive(Clone, Copy)]
struct Soft {
    bias_rate: FP,
    mass_scale: FP,
    impulse_scale: FP,
}

/// Largest spring frequency in hertz and damping ratio the solver uses.
const SOFT_HERTZ_MAX: FP = FP::from_int(1000);
const SOFT_DAMPING_MAX: FP = FP::from_int(1000);

/// The soft constraint terms for a step of `h` seconds, or `None` for a frequency of 0 or less,
/// which is rigid.
fn soft(hertz: FP, damping_ratio: FP, h: FP) -> Option<Soft> {
    if hertz <= FP::ZERO {
        return None;
    }
    let omega = FP::TWO_PI * hertz.min(SOFT_HERTZ_MAX);
    let a1 = FP::TWO * damping_ratio.clamp(FP::ZERO, SOFT_DAMPING_MAX) + h * omega;
    if a1 <= FP::ZERO {
        return Some(Soft { bias_rate: FP::ZERO, mass_scale: FP::ZERO, impulse_scale: FP::ONE });
    }
    let a2 = h * omega * a1;
    let a3 = FP::ONE / (FP::ONE + a2);
    Some(Soft { bias_rate: omega / a1, mass_scale: a2 * a3, impulse_scale: a3 })
}

/// A prismatic joint's axis in `a`'s own frame.
fn local_axis(joint: &Joint) -> FPVec2 {
    let (s, c) = joint.length.sin_cos();
    FPVec2::new(c, s)
}

/// What a prismatic joint needs beyond a pin's terms.
#[derive(Clone, Copy)]
struct Slide {
    /// The axis in the world.
    axis: FPVec2,
    /// The direction across the axis.
    across: FPVec2,
    /// From `a`'s center to `b`'s anchor in the world: the lever of `a`'s share of every slide row.
    lever: FPVec2,
    /// How far `b` is from where it was added, along the axis.
    offset: FP,
    /// One over the effective mass along the axis; 0 skips the motor and the limits.
    axis_mass: FP,
    /// The 2x2 mass of the row across the axis and the row that holds the turn: `k11`, `k12`, `k22`.
    block: [FP; 3],
    /// Position error across the axis, as a velocity to remove.
    across_bias: FP,
    /// Relative angle error, as a velocity to remove.
    turn_bias: FP,
}

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
    /// Rope: the impulse so far, which may only pull. Spring: the impulse so far.
    total: FP,
    /// Spring: the soft terms, or `None` when it is rigid.
    soft: Option<Soft>,
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
    slide: Slide,
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
            soft: None,
            flags: if joint.kind == JOINT_PIN || joint.kind == JOINT_PRISMATIC { joint.flags } else { 0 },
            angular_mass: FP::ZERO,
            couple: FPVec2::ZERO,
            angle: (bb.angle - ba.angle - joint.ref_angle).wrap_angle(),
            lower: joint.lower,
            upper: joint.upper,
            motor_speed: joint.motor_speed,
            motor_max: joint.max_motor_torque.abs() * dt,
            motor_total: FP::ZERO,
            lower_total: FP::ZERO,
            upper_total: FP::ZERO,
            inv_dt,
            slide: Slide {
                axis: FPVec2::ZERO,
                across: FPVec2::ZERO,
                lever: FPVec2::ZERO,
                offset: FP::ZERO,
                axis_mass: FP::ZERO,
                block: [FP::ZERO; 3],
                across_bias: FP::ZERO,
                turn_bias: FP::ZERO,
            },
        };
        if joint.kind == JOINT_PRISMATIC {
            row.slide = slide_terms(&row, &joint, &xfs[a], gap);
            rows.push(row);
            continue;
        }
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
            if joint.kind == JOINT_SPRING {
                row.soft = soft(joint.motor_speed, joint.max_motor_torque, dt);
                if let Some(s) = row.soft {
                    row.bias = FPVec2::new(error * s.bias_rate, FP::ZERO);
                }
            }
        }
        rows.push(row);
    }
    rows
}

/// The slide terms of a prismatic joint at this tick's pose. `a` is treated as
/// pushed at the point of its body that lies under `b`'s anchor, so the rows
/// stay right while `a` turns with `b` slid away from it.
fn slide_terms(row: &JointRow, joint: &Joint, xf_a: &Xf, gap: FPVec2) -> Slide {
    let axis = rotate(xf_a, local_axis(joint));
    let across = FPVec2::new(-axis.y, axis.x);
    let lever = row.ra + gap;
    let (sa, sb) = (lever.perp_dot(across), row.rb.perp_dot(across));
    let k11 = row.ima + row.imb + row.iia * sa * sa + row.iib * sb * sb;
    let k12 = row.iia * sa + row.iib * sb;
    let mut k22 = row.iia + row.iib;
    // Two bodies that cannot turn leave only the row across the axis.
    if k22 == FP::ZERO {
        k22 = FP::ONE;
    }
    let (na, nb) = (lever.perp_dot(axis), row.rb.perp_dot(axis));
    let k = row.ima + row.imb + row.iia * na * na + row.iib * nb * nb;
    Slide {
        axis,
        across,
        lever,
        offset: gap.dot(axis),
        axis_mass: if k > FP::ZERO { FP::ONE / k } else { FP::ZERO },
        block: [k11, k12, k22],
        across_bias: gap.dot(across) * (BAUMGARTE * row.inv_dt),
        turn_bias: row.angle * (BAUMGARTE * row.inv_dt),
    }
}

/// Applies an impulse `p` at `b`'s anchor, with the turn impulse `turn` on `b` against `a`.
fn push(row: &JointRow, va: &mut Vw, vb: &mut Vw, p: FPVec2, turn: FP) {
    va.v -= p * row.ima;
    va.w -= row.iia * (row.slide.lever.perp_dot(p) + turn);
    vb.v += p * row.imb;
    vb.w += row.iib * (row.rb.perp_dot(p) + turn);
}

/// Speed of `b`'s anchor against the point of `a` under it, along `dir`.
fn slide_speed(row: &JointRow, va: &Vw, vb: &Vw, dir: FPVec2) -> FP {
    ((vb.v + spin(vb.w, row.rb)) - (va.v + spin(va.w, row.slide.lever))).dot(dir)
}

/// A prismatic joint: the motor and the limits along the axis, then the
/// row across the axis together with the row that holds the turn.
fn solve_slide(row: &mut JointRow, va: &mut Vw, vb: &mut Vw) {
    let s = row.slide;
    if s.axis_mass > FP::ZERO {
        if row.flags & JOINT_MOTOR != 0 {
            let lambda = -(s.axis_mass * (slide_speed(row, va, vb, s.axis) - row.motor_speed));
            let total = (row.motor_total + lambda).clamp(-row.motor_max, row.motor_max);
            let lambda = total - row.motor_total;
            row.motor_total = total;
            push(row, va, vb, s.axis * lambda, FP::ZERO);
        }
        if row.flags & JOINT_LIMIT != 0 {
            // Speculative: a bound not yet reached only stops a slide that would pass it this tick.
            let bias = |c: FP| if c < FP::ZERO { c * BAUMGARTE * row.inv_dt } else { c * row.inv_dt };
            let lambda = -(s.axis_mass * (slide_speed(row, va, vb, s.axis) + bias(s.offset - row.lower)));
            let total = (row.lower_total + lambda).max(FP::ZERO);
            let lambda = total - row.lower_total;
            row.lower_total = total;
            push(row, va, vb, s.axis * lambda, FP::ZERO);
            let lambda = -(s.axis_mass * (-slide_speed(row, va, vb, s.axis) + bias(row.upper - s.offset)));
            let total = (row.upper_total + lambda).max(FP::ZERO);
            let lambda = total - row.upper_total;
            row.upper_total = total;
            push(row, va, vb, s.axis * -lambda, FP::ZERO);
        }
    }
    let [k11, k12, k22] = s.block;
    let det = k11 * k22 - k12 * k12;
    if det == FP::ZERO {
        return;
    }
    let r0 = -(slide_speed(row, va, vb, s.across) + s.across_bias);
    let r1 = -(vb.w - va.w + s.turn_bias);
    let (across, turn) = ((k22 * r0 - k12 * r1) / det, (k11 * r1 - k12 * r0) / det);
    push(row, va, vb, s.across * across, turn);
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
        if row.kind == JOINT_PRISMATIC {
            solve_slide(row, &mut va, &mut vb);
            vw[row.a] = va;
            vw[row.b] = vb;
            continue;
        }
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
            let speed = relative.dot(row.axis) + row.bias.x;
            let mut lambda = match row.soft {
                Some(s) => -(s.mass_scale * row.mass * speed) - s.impulse_scale * row.total,
                None => -(row.mass * speed),
            };
            if row.soft.is_some() {
                row.total += lambda;
            }
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
