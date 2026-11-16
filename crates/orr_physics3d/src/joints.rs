//! Joints between two bodies, solved with the contacts in every substep: a
//! ball joint holds two anchor points together (the bodies turn freely about
//! it), a rod keeps them a fixed distance apart, and a rope keeps them at most
//! that far apart. A hinge is a ball joint that also keeps one axis of each
//! body lined up, so they turn about that axis only, and it can drive the turn
//! with a motor and hold it between two angles. A ball joint can be limited
//! to a cone of swing around an axis and a range of twist about it, as a
//! shoulder or a ragdoll's neck. A prismatic joint lets `b` slide along one
//! axis of `a` and nothing else, with an optional motor and limits on the
//! slide, for pistons and sliding doors. Joints are an opt-in component: a game
//! that never calls [`register_joints`] steps exactly as before.

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

/// [`Joint::kind`]: a ball joint that turns only about [`Joint::axis_a`].
pub const JOINT_HINGE: u32 = 3;
/// [`Joint::kind`]: `b` slides along [`Joint::axis_a`] and cannot turn against `a`.
pub const JOINT_PRISMATIC: u32 = 4;

/// [`Joint::flags`]: a hinge's motor drives the turn, and a prismatic joint's motor the
/// slide, toward [`Joint::motor_speed`].
pub const JOINT_MOTOR: u32 = 1;
/// [`Joint::flags`]: a hinge holds its angle between [`Joint::lower`] and [`Joint::upper`];
/// a ball joint holds its swing within [`Joint::swing`] and its twist between them;
/// a prismatic joint holds its slide between them.
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
    pub anchor_a: FPVec3,
    /// Anchor on `b`, in `b`'s own frame.
    pub anchor_b: FPVec3,
    /// Distance between the anchors for a rod or rope; unused by a ball joint.
    pub length: FP,
    /// [`JOINT_BALL`], [`JOINT_ROD`], [`JOINT_ROPE`], [`JOINT_HINGE`] or [`JOINT_PRISMATIC`].
    pub kind: u32,
    /// [`JOINT_MOTOR`] and [`JOINT_LIMIT`], for a hinge or a prismatic joint; 0 for none.
    pub flags: u32,
    /// Hinge axis, slide axis of a prismatic joint, or the axis a limited ball joint swings
    /// and twists around, in `a`'s own frame.
    pub axis_a: FPVec3,
    /// The same axis in `b`'s own frame; [`add_joint`] fills it.
    pub axis_b: FPVec3,
    /// A direction across the axis in `a`'s frame that the angle is measured from; [`add_joint`] fills it.
    /// A prismatic joint keeps the vector part of the rotation of `b` in `a`'s frame here.
    pub ref_a: FPVec3,
    /// The same direction in `b`'s frame when the joint was added; [`add_joint`] fills it.
    /// A prismatic joint keeps the real part of that rotation in `x` and the slide at the start in `y`.
    pub ref_b: FPVec3,
    /// Lowest hinge angle or ball joint twist in radians, above `-pi`, with [`JOINT_LIMIT`].
    /// A prismatic joint's lowest slide along the axis from where it was added.
    pub lower: FP,
    /// Highest hinge angle or ball joint twist in radians, below `pi`, with [`JOINT_LIMIT`].
    /// A prismatic joint's highest slide along the axis from where it was added.
    pub upper: FP,
    /// Turn rate of `b` against `a` about the axis, in radians per second, the motor aims for.
    /// A prismatic joint's speed of `b` along the axis.
    pub motor_speed: FP,
    /// Largest torque the motor applies, or force for a prismatic joint; 0 lets it do nothing.
    pub max_motor_torque: FP,
    /// Largest angle in radians between the two bodies' axes of a ball joint with [`JOINT_LIMIT`], below `pi`.
    pub swing: FP,
}

impl Joint {
    /// A ball joint, rod or rope with no motor and no limits.
    #[must_use]
    pub fn new(kind: u32, a: Entity, b: Entity, anchor_a: FPVec3, anchor_b: FPVec3, length: FP) -> Joint {
        Joint { a, b, anchor_a, anchor_b, length, kind, ..Joint::zeroed() }
    }

    /// A hinge about `axis`, given in `a`'s own frame.
    #[must_use]
    pub fn hinge(a: Entity, b: Entity, anchor_a: FPVec3, anchor_b: FPVec3, axis: FPVec3) -> Joint {
        Joint { axis_a: axis, ..Joint::new(JOINT_HINGE, a, b, anchor_a, anchor_b, FP::ZERO) }
    }

    /// A joint that lets `b` slide along `axis`, given in `a`'s own frame, and nothing else.
    /// The anchors are the points that stay on the axis line through `anchor_a`.
    #[must_use]
    pub fn prismatic(a: Entity, b: Entity, anchor_a: FPVec3, anchor_b: FPVec3, axis: FPVec3) -> Joint {
        Joint { axis_a: axis, ..Joint::new(JOINT_PRISMATIC, a, b, anchor_a, anchor_b, FP::ZERO) }
    }

    /// The same hinge with a motor turning `b` against `a` at `speed`, with at most `max_torque`.
    /// A prismatic joint's motor slides `b` along the axis at `speed`, with at most `max_torque` of force.
    #[must_use]
    pub fn with_motor(self, speed: FP, max_torque: FP) -> Joint {
        Joint { flags: self.flags | JOINT_MOTOR, motor_speed: speed, max_motor_torque: max_torque, ..self }
    }

    /// The same hinge held between `lower` and `upper` radians from where it was added.
    /// A prismatic joint is held between those distances along the axis from where it was added.
    #[must_use]
    pub fn with_limits(self, lower: FP, upper: FP) -> Joint {
        Joint { flags: self.flags | JOINT_LIMIT, lower, upper, ..self }
    }

    /// The same ball joint kept within `swing` radians of `axis` (in `a`'s own
    /// frame) and twisted about it between `lower` and `upper` radians, both
    /// measured from the pose when it was added.
    #[must_use]
    pub fn with_cone(self, axis: FPVec3, swing: FP, lower: FP, upper: FP) -> Joint {
        Joint { flags: self.flags | JOINT_LIMIT, axis_a: axis, swing, lower, upper, ..self }
    }

    /// Whether [`add_joint`] records the axis and the zero angle for this joint.
    fn has_axis(&self) -> bool {
        self.kind == JOINT_HINGE || self.kind == JOINT_PRISMATIC || self.has_cone()
    }

    /// Whether the recorded terms are still missing. A prismatic joint's `ref_a` can be zero
    /// once recorded, so its `axis_b` tells.
    fn unrecorded(&self) -> bool {
        if self.kind == JOINT_PRISMATIC {
            self.axis_b == FPVec3::ZERO
        } else {
            self.ref_a == FPVec3::ZERO
        }
    }

    fn has_cone(&self) -> bool {
        self.kind == JOINT_BALL && self.flags & JOINT_LIMIT != 0
    }
}

/// A unit direction across `n`.
fn across(n: FPVec3) -> FPVec3 {
    let e = if n.x.abs() < FP::from_ratio(6, 10) { FPVec3::new(FP::ONE, FP::ZERO, FP::ZERO) } else { FPVec3::new(FP::ZERO, FP::ONE, FP::ZERO) };
    n.cross(e).normalize()
}

/// Registers [`Joint`]. Call it after [`crate::register`] in a game that uses joints.
pub fn register_joints(builder: &mut ComponentRegistryBuilder) {
    builder.register_component::<Joint>("orr_physics3d::Joint");
}

/// Records the axis and the zero angle of a hinge or limited ball joint in
/// both bodies' frames from their current poses. A joint whose bodies are
/// missing, or whose axis is zero, is left unrecorded.
fn record(frame: &Frame, joint: &mut Joint) {
    let (Some(a), Some(b)) = (frame.get::<Body>(joint.a), frame.get::<Body>(joint.b)) else { return };
    if joint.kind == JOINT_PRISMATIC {
        record_slide(a, b, joint);
        return;
    }
    let n = a.rot.rotate_vec3(joint.axis_a).normalize_or_zero();
    if n == FPVec3::ZERO {
        return;
    }
    let r = across(n);
    joint.axis_a = a.rot.inverse_rotate_vec3(n);
    joint.axis_b = b.rot.inverse_rotate_vec3(n);
    joint.ref_a = a.rot.inverse_rotate_vec3(r);
    joint.ref_b = b.rot.inverse_rotate_vec3(r);
}

/// Records a prismatic joint's axis in both bodies' frames, the rotation of
/// `b` in `a`'s frame to hold, and the slide at the start.
fn record_slide(a: &Body, b: &Body, joint: &mut Joint) {
    let axis = joint.axis_a.normalize_or_zero();
    if axis == FPVec3::ZERO {
        return;
    }
    let rest = a.rot.conjugate() * b.rot;
    let rest = if rest.w < FP::ZERO { FPQuat::new(-rest.x, -rest.y, -rest.z, -rest.w) } else { rest };
    let gap = (b.pos + b.rot.rotate_vec3(joint.anchor_b)) - (a.pos + a.rot.rotate_vec3(joint.anchor_a));
    joint.axis_a = axis;
    joint.axis_b = b.rot.inverse_rotate_vec3(a.rot.rotate_vec3(axis));
    joint.ref_a = FPVec3::new(rest.x, rest.y, rest.z);
    joint.ref_b = FPVec3::new(rest.w, gap.dot(a.rot.rotate_vec3(axis)), FP::ZERO);
}

/// Records the joints that came in without [`add_joint`] (a scene file, the
/// editor's inspector) from the poses at the start of their first step. A
/// recorded joint has a nonzero `ref_a`.
pub(crate) fn settle(frame: &mut Frame) {
    if frame.registry().component_id::<Joint>().is_none() {
        return;
    }
    let (je, jd) = frame.dense::<Joint>();
    let pending: Vec<(Entity, Joint)> = je.iter().zip(jd).filter(|(_, j)| j.has_axis() && j.unrecorded()).map(|(&e, &j)| (e, j)).collect();
    for (e, mut joint) in pending {
        record(frame, &mut joint);
        if let Some(stored) = frame.get_mut::<Joint>(e) {
            *stored = joint;
        }
    }
}

/// Spawns an entity holding `joint` and wakes both bodies. For a hinge, a
/// prismatic joint or a limited ball joint it also records the axis and the zero angle in both
/// bodies' frames from their current poses. Bodies held by a joint do not fall asleep.
pub fn add_joint(frame: &mut Frame, mut joint: Joint) -> Entity {
    if joint.has_axis() {
        record(frame, &mut joint);
    }
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
    /// Ball and hinge: the effective mass matrix `K`, kept as its adjugate rows and determinant.
    k: Solve3,
    /// Ball: the position error as a velocity to remove.
    bias: FPVec3,
    axis: FPVec3,
    /// Rod and rope: one over the effective mass along `axis`, and the bias along it.
    mass: FP,
    bias_axis: FP,
    /// Rope: the impulse this substep, which may only pull.
    total: FP,
    hinge: Option<Hinge>,
    cone: Option<Cone>,
    slide: Option<Slide>,
}

/// A prismatic joint: the recorded terms and the terms of this substep.
#[derive(Clone, Copy)]
struct Slide {
    flags: u32,
    axis_a: FPVec3,
    /// The rotation of `b` in `a`'s frame to hold.
    rest: FPQuat,
    /// The slide when the joint was added.
    start: FP,
    lower: FP,
    upper: FP,
    motor_speed: FP,
    max_force: FP,
    // Refreshed per substep.
    /// The axis in the world.
    axis: FPVec3,
    /// From `a`'s center to `b`'s anchor in the world: the lever of `a`'s share of every slide row.
    lever: FPVec3,
    /// How far `b` is from where it was added, along the axis.
    offset: FP,
    /// One over the effective mass along the axis; 0 skips the motor and the limits.
    axis_mass: FP,
    /// The anchor mass of the two directions across the axis.
    across: Solve3,
    across_bias: FPVec3,
    /// The turn mass of the two bodies together.
    turn: Solve3,
    turn_bias: FPVec3,
    inv_h: FP,
    motor_max: FP,
    motor_total: FP,
    lower_total: FP,
    upper_total: FP,
}

/// One turn axis of a hinge: its effective mass with the anchors held
/// together, and the anchor impulse per unit of turn impulse that holds them.
#[derive(Clone, Copy, Default)]
struct TurnAxis {
    axis: FPVec3,
    mass: FP,
    couple: FPVec3,
    /// How a turn about `axis` moves the anchors apart, per unit.
    c: FPVec3,
    /// Alignment rows: the misalignment as a velocity to remove.
    bias: FP,
}

#[derive(Clone, Copy)]
struct Hinge {
    flags: u32,
    axis_a: FPVec3,
    axis_b: FPVec3,
    ref_a: FPVec3,
    ref_b: FPVec3,
    lower: FP,
    upper: FP,
    motor_speed: FP,
    max_torque: FP,
    // Refreshed per substep.
    across: [TurnAxis; 2],
    /// The inverse of the 2x2 effective mass of the two alignment rows, solved together.
    across_inv: [FP; 3],
    turn: TurnAxis,
    angle: FP,
    inv_h: FP,
    motor_max: FP,
    motor_total: FP,
    lower_total: FP,
    upper_total: FP,
}

/// The swing and twist limits of a ball joint.
#[derive(Clone, Copy)]
struct Cone {
    axis_a: FPVec3,
    axis_b: FPVec3,
    ref_a: FPVec3,
    ref_b: FPVec3,
    swing: FP,
    lower: FP,
    upper: FP,
    // Refreshed per substep. A row with zero mass is left out.
    /// A turn that closes the swing, and how far the swing is inside its bound.
    close: TurnAxis,
    room: FP,
    twist: TurnAxis,
    angle: FP,
    inv_h: FP,
    swing_total: FP,
    lower_total: FP,
    upper_total: FP,
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
            k: Solve3::default(),
            bias: FPVec3::ZERO,
            axis: FPVec3::ZERO,
            mass: FP::ZERO,
            bias_axis: FP::ZERO,
            total: FP::ZERO,
            hinge: (joint.kind == JOINT_HINGE && joint.axis_a != FPVec3::ZERO).then_some(Hinge {
                flags: joint.flags,
                axis_a: joint.axis_a,
                axis_b: joint.axis_b,
                ref_a: joint.ref_a,
                ref_b: joint.ref_b,
                lower: joint.lower,
                upper: joint.upper,
                motor_speed: joint.motor_speed,
                max_torque: joint.max_motor_torque,
                across: [TurnAxis::default(); 2],
                across_inv: [FP::ZERO; 3],
                turn: TurnAxis::default(),
                angle: FP::ZERO,
                inv_h: FP::ZERO,
                motor_max: FP::ZERO,
                motor_total: FP::ZERO,
                lower_total: FP::ZERO,
                upper_total: FP::ZERO,
            }),
            slide: (joint.kind == JOINT_PRISMATIC && joint.axis_b != FPVec3::ZERO).then_some(Slide {
                flags: joint.flags,
                axis_a: joint.axis_a,
                rest: FPQuat::new(joint.ref_a.x, joint.ref_a.y, joint.ref_a.z, joint.ref_b.x),
                start: joint.ref_b.y,
                lower: joint.lower,
                upper: joint.upper,
                motor_speed: joint.motor_speed,
                max_force: joint.max_motor_torque,
                axis: FPVec3::ZERO,
                lever: FPVec3::ZERO,
                offset: FP::ZERO,
                axis_mass: FP::ZERO,
                across: Solve3::default(),
                across_bias: FPVec3::ZERO,
                turn: Solve3::default(),
                turn_bias: FPVec3::ZERO,
                inv_h: FP::ZERO,
                motor_max: FP::ZERO,
                motor_total: FP::ZERO,
                lower_total: FP::ZERO,
                upper_total: FP::ZERO,
            }),
            cone: (joint.has_cone() && joint.axis_a != FPVec3::ZERO).then_some(Cone {
                axis_a: joint.axis_a,
                axis_b: joint.axis_b,
                ref_a: joint.ref_a,
                ref_b: joint.ref_b,
                swing: joint.swing,
                lower: joint.lower,
                upper: joint.upper,
                close: TurnAxis::default(),
                room: FP::ZERO,
                twist: TurnAxis::default(),
                angle: FP::ZERO,
                inv_h: FP::ZERO,
                swing_total: FP::ZERO,
                lower_total: FP::ZERO,
                upper_total: FP::ZERO,
            }),
        });
    }
    rows
}

/// Solves `K x = v` for a 3x3 `K` without forming `K^-1`: the entries of a
/// light body's `K^-1` far from its anchor fall to a few fixed point units.
#[derive(Clone, Copy, Default)]
struct Solve3 {
    /// The adjugate's rows: the cross products of `K`'s columns.
    rows: [FPVec3; 3],
    det: FP,
}

impl Solve3 {
    fn new(k: &FPMat3) -> Solve3 {
        let (c0, c1, c2) = (k.col(0), k.col(1), k.col(2));
        let rows = [c1.cross(c2), c2.cross(c0), c0.cross(c1)];
        Solve3 { rows, det: c0.dot(rows[0]) }
    }

    fn solve(&self, v: FPVec3) -> FPVec3 {
        if self.det == FP::ZERO {
            return FPVec3::ZERO;
        }
        // Each product carries 32 fraction bits; dividing by the 16-bit determinant leaves 16.
        let x = |r: FPVec3| {
            let n = i128::from(r.x.raw()) * i128::from(v.x.raw()) + i128::from(r.y.raw()) * i128::from(v.y.raw()) + i128::from(r.z.raw()) * i128::from(v.z.raw());
            FP::from_raw((n / i128::from(self.det.raw())).clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64)
        };
        FPVec3::new(x(self.rows[0]), x(self.rows[1]), x(self.rows[2]))
    }
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
        if row.kind == JOINT_PRISMATIC {
            row.active = false;
            if let Some(mut slide) = row.slide {
                row.active = begin_slide(row, &mut slide, rot_a, rot_b, gap, h);
                row.slide = Some(slide);
            }
            continue;
        }
        if row.kind == JOINT_BALL || row.kind == JOINT_HINGE {
            let m = row.ima + row.imb;
            let k = add(&add(&FPMat3::from_diag(FPVec3::new(m, m, m)), &arm(&row.iia, row.ra)), &arm(&row.iib, row.rb));
            row.k = Solve3::new(&k);
            row.active = row.k.det != FP::ZERO;
            row.bias = gap * (BAUMGARTE * inv_h);
            if let (true, Some(mut hinge)) = (row.active, row.hinge) {
                begin_hinge(row, &mut hinge, rot_a, rot_b, h);
                row.hinge = Some(hinge);
            }
            if let (true, Some(mut cone)) = (row.active, row.cone) {
                begin_cone(row, &mut cone, rot_a, rot_b, inv_h);
                row.cone = Some(cone);
            }
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

/// The slide terms of a prismatic joint at this substep's pose, or `false`
/// when the axis is zero. `a` is treated as pushed at the point of its body
/// that lies under `b`'s anchor, so the rows stay right while `a` turns with
/// `b` slid away from it.
fn begin_slide(row: &JointRow, slide: &mut Slide, rot_a: FPQuat, rot_b: FPQuat, gap: FPVec3, h: FP) -> bool {
    let axis = rot_a.rotate_vec3(slide.axis_a);
    if axis == FPVec3::ZERO {
        return false;
    }
    let inv_h = FP::ONE / h;
    let along = gap.dot(axis);
    let lever = row.ra + gap;
    // The two directions across the axis: the anchor mass `K` restricted to them, plus 1 on the axis
    // so that the whole matrix can be solved the way a ball joint is.
    let m = row.ima + row.imb;
    let k = add(&add(&FPMat3::from_diag(FPVec3::new(m, m, m)), &arm(&row.iia, lever)), &arm(&row.iib, row.rb));
    let line = FPMat3::from_cols(axis * axis.x, axis * axis.y, axis * axis.z);
    let flat = FPMat3::from_cols(FPVec3::X - line.col(0), FPVec3::Y - line.col(1), FPVec3::Z - line.col(2));
    slide.across = Solve3::new(&add(&flat.mul_mat(&k).mul_mat(&flat), &line));
    slide.across_bias = (gap - axis * along) * (BAUMGARTE * inv_h);
    slide.turn = Solve3::new(&add(&row.iia, &row.iib));
    // The turn of `b` away from `a`'s pose times the rotation to hold, as a vector.
    let off = rot_b * (rot_a * slide.rest).conjugate();
    let sign = if off.w < FP::ZERO { -FP::TWO } else { FP::TWO };
    slide.turn_bias = FPVec3::new(off.x, off.y, off.z) * (sign * BAUMGARTE * inv_h);
    let (na, nb) = (lever.cross(axis), row.rb.cross(axis));
    let k = m + na.dot(row.iia.mul_vec(na)) + nb.dot(row.iib.mul_vec(nb));
    slide.axis_mass = if k > FP::ZERO { FP::ONE / k } else { FP::ZERO };
    slide.axis = axis;
    slide.lever = lever;
    slide.offset = along - slide.start;
    slide.inv_h = inv_h;
    slide.motor_max = slide.max_force.abs() * h;
    slide.motor_total = FP::ZERO;
    slide.lower_total = FP::ZERO;
    slide.upper_total = FP::ZERO;
    true
}

/// The hinge's axes, misalignment and angle at this substep's pose.
fn begin_hinge(row: &JointRow, hinge: &mut Hinge, rot_a: FPQuat, rot_b: FPQuat, h: FP) {
    let inv_h = FP::ONE / h;
    let n = rot_a.rotate_vec3(hinge.axis_a);
    let misaligned = n.cross(rot_b.rotate_vec3(hinge.axis_b));
    let t1 = across(n);
    let t2 = n.cross(t1);
    for (slot, t) in hinge.across.iter_mut().zip([t1, t2]) {
        *slot = turn_axis(row, t);
        slot.bias = t.dot(misaligned) * BAUMGARTE * inv_h;
    }
    let [s, u] = hinge.across;
    let k = |a: &TurnAxis, b: &TurnAxis| a.axis.dot(row.iia.mul_vec(b.axis) + row.iib.mul_vec(b.axis)) + a.c.dot(b.couple);
    let (k11, k12, k22) = (k(&s, &s), k(&s, &u), k(&u, &u));
    let det = k11 * k22 - k12 * k12;
    hinge.across_inv = if det > FP::ZERO { [k22 / det, -(k12 / det), k11 / det] } else { [FP::ZERO; 3] };
    hinge.turn = turn_axis(row, n);
    let (ra, rb) = (rot_a.rotate_vec3(hinge.ref_a), rot_b.rotate_vec3(hinge.ref_b));
    hinge.angle = n.dot(ra.cross(rb)).atan2(ra.dot(rb));
    hinge.inv_h = inv_h;
    hinge.motor_max = hinge.max_torque.abs() * h;
    hinge.motor_total = FP::ZERO;
    hinge.lower_total = FP::ZERO;
    hinge.upper_total = FP::ZERO;
}

/// The swing of the axes and the twist about their bisector at this substep's pose.
fn begin_cone(row: &JointRow, cone: &mut Cone, rot_a: FPQuat, rot_b: FPQuat, inv_h: FP) {
    let (na, nb) = (rot_a.rotate_vec3(cone.axis_a), rot_b.rotate_vec3(cone.axis_b));
    let bend = na.cross(nb);
    let sine = bend.length();
    // Below about a thousandth of a radian the swing direction is noise; aligned axes are inside any cone.
    let cosine = na.dot(nb);
    cone.close = if sine > FP::from_ratio(1, 1000) {
        turn_axis(row, -(bend / sine))
    } else if cosine < FP::ZERO {
        // Opposite axes have no swing direction of their own, so any direction across `na` serves.
        turn_axis(row, -across(na))
    } else {
        TurnAxis::default()
    };
    cone.room = cone.swing - sine.atan2(cosine);
    let mid = na + nb;
    let length = mid.length();
    // With the axes almost opposite there is no twist axis, so the twist rows wait.
    cone.twist = if length > FP::from_ratio(1, 100) { turn_axis(row, mid / length) } else { TurnAxis::default() };
    let t = cone.twist.axis;
    let flat = |r: FPVec3| r - t * t.dot(r);
    let (ra, rb) = (flat(rot_a.rotate_vec3(cone.ref_a)), flat(rot_b.rotate_vec3(cone.ref_b)));
    cone.angle = t.dot(ra.cross(rb)).atan2(ra.dot(rb));
    cone.inv_h = inv_h;
    cone.swing_total = FP::ZERO;
    cone.lower_total = FP::ZERO;
    cone.upper_total = FP::ZERO;
}

/// A turn about `axis` paired with the anchor impulse `-K^-1 c` that keeps
/// the anchors together; its effective mass is the Schur complement of `K`.
fn turn_axis(row: &JointRow, axis: FPVec3) -> TurnAxis {
    let c = row.iib.mul_vec(axis).cross(row.rb) + row.iia.mul_vec(axis).cross(row.ra);
    let couple = -row.k.solve(c);
    let k = axis.dot(row.iia.mul_vec(axis) + row.iib.mul_vec(axis)) + c.dot(couple);
    TurnAxis { axis, mass: if k > FP::ZERO { FP::ONE / k } else { FP::ZERO }, couple, c, bias: FP::ZERO }
}

impl TurnAxis {
    /// The same turn the other way round.
    fn flip(self) -> TurnAxis {
        TurnAxis { axis: -self.axis, couple: -self.couple, c: -self.c, ..self }
    }
}

/// A one-sided limit: a turn of `b` about `t.axis` grows `room`, the
/// distance inside the bound, which may not go below zero. Speculative: a
/// bound not yet reached only stops a turn that would pass it this substep.
fn limit(row: &JointRow, t: &TurnAxis, room: FP, inv_h: FP, total: &mut FP, va: &mut Vw, vb: &mut Vw) {
    if t.mass == FP::ZERO {
        return;
    }
    let bias = if room < FP::ZERO { room * BAUMGARTE * inv_h } else { room * inv_h };
    let lambda = -(t.mass * (t.axis.dot(vb.w - va.w) + bias));
    let sum = (*total + lambda).max(FP::ZERO);
    turn(row, t, va, vb, sum - *total);
    *total = sum;
}

/// Applies a turn impulse `lambda` about `t.axis` (on `b`, against `a`) with its anchor impulse.
fn turn(row: &JointRow, t: &TurnAxis, va: &mut Vw, vb: &mut Vw, lambda: FP) {
    let p = t.couple * lambda;
    let spin = t.axis * lambda;
    va.v -= p * row.ima;
    va.w -= row.iia.mul_vec(row.ra.cross(p) + spin);
    vb.v += p * row.imb;
    vb.w += row.iib.mul_vec(row.rb.cross(p) + spin);
}

/// The motor, the limits and the axis alignment of a hinge.
fn solve_hinge(row: &JointRow, hinge: &mut Hinge, va: &mut Vw, vb: &mut Vw) {
    let n = hinge.turn;
    if n.mass > FP::ZERO {
        if hinge.flags & JOINT_MOTOR != 0 {
            let lambda = -(n.mass * (n.axis.dot(vb.w - va.w) - hinge.motor_speed));
            let total = (hinge.motor_total + lambda).clamp(-hinge.motor_max, hinge.motor_max);
            let lambda = total - hinge.motor_total;
            hinge.motor_total = total;
            turn(row, &n, va, vb, lambda);
        }
        if hinge.flags & JOINT_LIMIT != 0 {
            limit(row, &n, hinge.angle - hinge.lower, hinge.inv_h, &mut hinge.lower_total, va, vb);
            limit(row, &n.flip(), hinge.upper - hinge.angle, hinge.inv_h, &mut hinge.upper_total, va, vb);
        }
    }
    let [s, u] = hinge.across;
    let [i11, i12, i22] = hinge.across_inv;
    let w = vb.w - va.w;
    let (rs, ru) = (-(s.axis.dot(w) + s.bias), -(u.axis.dot(w) + u.bias));
    turn(row, &s, va, vb, i11 * rs + i12 * ru);
    turn(row, &u, va, vb, i12 * rs + i22 * ru);
}

/// Applies an impulse `p` at `b`'s anchor, and at the point of `a` under it.
fn push(row: &JointRow, slide: &Slide, va: &mut Vw, vb: &mut Vw, p: FPVec3) {
    va.v -= p * row.ima;
    va.w -= row.iia.mul_vec(slide.lever.cross(p));
    vb.v += p * row.imb;
    vb.w += row.iib.mul_vec(row.rb.cross(p));
}

/// Velocity of `b`'s anchor against the point of `a` under it.
fn slide_velocity(row: &JointRow, slide: &Slide, va: &Vw, vb: &Vw) -> FPVec3 {
    (vb.v + vb.w.cross(row.rb)) - (va.v + va.w.cross(slide.lever))
}

/// A prismatic joint: the motor and the limits along the axis, then the
/// rows across the axis and the rows that hold the rotation.
fn solve_slide(row: &JointRow, slide: &mut Slide, va: &mut Vw, vb: &mut Vw) {
    let n = slide.axis;
    if slide.axis_mass > FP::ZERO {
        if slide.flags & JOINT_MOTOR != 0 {
            let lambda = -(slide.axis_mass * (slide_velocity(row, slide, va, vb).dot(n) - slide.motor_speed));
            let total = (slide.motor_total + lambda).clamp(-slide.motor_max, slide.motor_max);
            let lambda = total - slide.motor_total;
            slide.motor_total = total;
            push(row, slide, va, vb, n * lambda);
        }
        if slide.flags & JOINT_LIMIT != 0 {
            // Speculative: a bound not yet reached only stops a slide that would pass it this substep.
            let bias = |c: FP| if c < FP::ZERO { c * BAUMGARTE * slide.inv_h } else { c * slide.inv_h };
            let lambda = -(slide.axis_mass * (slide_velocity(row, slide, va, vb).dot(n) + bias(slide.offset - slide.lower)));
            let total = (slide.lower_total + lambda).max(FP::ZERO);
            let lambda = total - slide.lower_total;
            slide.lower_total = total;
            push(row, slide, va, vb, n * lambda);
            let lambda = -(slide.axis_mass * (-slide_velocity(row, slide, va, vb).dot(n) + bias(slide.upper - slide.offset)));
            let total = (slide.upper_total + lambda).max(FP::ZERO);
            let lambda = total - slide.upper_total;
            slide.upper_total = total;
            push(row, slide, va, vb, n * -lambda);
        }
    }
    let relative = slide_velocity(row, slide, va, vb);
    let across = slide.across.solve(-((relative - n * relative.dot(n)) + slide.across_bias));
    push(row, slide, va, vb, across - n * across.dot(n));
    let turn = slide.turn.solve(-((vb.w - va.w) + slide.turn_bias));
    va.w -= row.iia.mul_vec(turn);
    vb.w += row.iib.mul_vec(turn);
}

/// The swing and twist limits of a ball joint.
fn solve_cone(row: &JointRow, cone: &mut Cone, va: &mut Vw, vb: &mut Vw) {
    limit(row, &cone.close, cone.room, cone.inv_h, &mut cone.swing_total, va, vb);
    limit(row, &cone.twist, cone.angle - cone.lower, cone.inv_h, &mut cone.lower_total, va, vb);
    limit(row, &cone.twist.flip(), cone.upper - cone.angle, cone.inv_h, &mut cone.upper_total, va, vb);
}

/// One iteration over every active joint.
pub(crate) fn solve(rows: &mut [JointRow], vw: &mut [Vw]) {
    for row in rows.iter_mut().filter(|r| r.active) {
        let (mut va, mut vb) = (vw[row.a], vw[row.b]);
        if let Some(mut slide) = row.slide {
            solve_slide(row, &mut slide, &mut va, &mut vb);
            row.slide = Some(slide);
            vw[row.a] = va;
            vw[row.b] = vb;
            continue;
        }
        if let Some(mut hinge) = row.hinge {
            solve_hinge(row, &mut hinge, &mut va, &mut vb);
            row.hinge = Some(hinge);
        }
        if let Some(mut cone) = row.cone {
            solve_cone(row, &mut cone, &mut va, &mut vb);
            row.cone = Some(cone);
        }
        let relative = (vb.v + vb.w.cross(row.rb)) - (va.v + va.w.cross(row.ra));
        let impulse = if row.kind == JOINT_BALL || row.kind == JOINT_HINGE {
            row.k.solve(-(relative + row.bias))
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
