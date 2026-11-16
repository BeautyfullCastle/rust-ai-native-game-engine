//! Ball joints, rods and ropes between 3D bodies, and their determinism.
// Distances are measured in f64 for the assertions only; the simulation stays fixed point.
#![allow(clippy::float_arithmetic)]
use orr_ecs::{ComponentRegistryBuilder, Entity, Frame};
use orr_fp::{fp, FPVec3, FP};
use orr_physics3d::{add_joint, init, register, register_joints, spawn_body, step, Body, Collider, Joint, PhysicsConfig, Scratch, Shape, JOINT_BALL, JOINT_ROD, JOINT_ROPE};
use orr_fp::FPQuat;

fn new_frame(joints: bool) -> Frame {
    let mut b = ComponentRegistryBuilder::new();
    register(&mut b);
    if joints {
        register_joints(&mut b);
    }
    let mut f = Frame::new(b.build());
    init(&mut f, PhysicsConfig::default());
    f
}

fn run(frame: &mut Frame, sc: &mut Scratch, ticks: u32) {
    for _ in 0..ticks {
        frame.set_tick(frame.tick() + 1);
        step(frame, sc);
    }
}

/// A small fixed pivot at (0, 10, 0) and a ball `offset` from it, joined by `kind` of `length`.
fn hanging(kind: u32, offset: FPVec3, length: FP) -> (Frame, Entity, Entity) {
    let mut f = new_frame(true);
    let pivot = spawn_body(&mut f, Body::new_static(FPVec3::new(FP::ZERO, fp!(10), FP::ZERO)), Collider::new(Shape::sphere(fp!(0.1))));
    let shape = Shape::sphere(fp!(0.2));
    let ball = spawn_body(&mut f, Body::new_dynamic(FPVec3::new(FP::ZERO, fp!(10), FP::ZERO) + offset, &shape, FP::ONE), Collider::new(shape));
    let anchor_b = if kind == JOINT_BALL { -offset } else { FPVec3::ZERO };
    add_joint(&mut f, Joint::new(kind, pivot, ball, FPVec3::ZERO, anchor_b, length));
    (f, pivot, ball)
}

fn distance(f: &Frame, a: Entity, b: Entity) -> f64 {
    let d = f.get::<Body>(b).unwrap().pos - f.get::<Body>(a).unwrap().pos;
    [d.x, d.y, d.z].iter().map(|v| (v.raw() as f64 / 65536.0).powi(2)).sum::<f64>().sqrt()
}

fn y(f: &Frame, e: Entity) -> f64 {
    f.get::<Body>(e).unwrap().pos.y.raw() as f64 / 65536.0
}

#[test]
fn a_ball_joint_swings_a_body_on_its_pivot() {
    let (mut f, pivot, ball) = hanging(JOINT_BALL, FPVec3::new(fp!(3), FP::ZERO, fp!(1)), FP::ZERO);
    let reach = 10f64.sqrt();
    let mut sc = Scratch::new();
    let mut lowest = f64::MAX;
    for _ in 0..90 {
        run(&mut f, &mut sc, 1);
        let d = distance(&f, pivot, ball);
        assert!((d - reach).abs() < 0.1, "the ball stays at its reach from the pivot: {d}");
        lowest = lowest.min(y(&f, ball));
    }
    assert!(lowest < 7.5, "it swings down under the pivot: lowest {lowest}");
}

#[test]
fn a_rod_keeps_its_length_and_a_rope_only_its_maximum() {
    let (mut f, pivot, ball) = hanging(JOINT_ROD, FPVec3::new(fp!(2), FP::ZERO, FP::ZERO), fp!(2));
    let mut sc = Scratch::new();
    for _ in 0..120 {
        run(&mut f, &mut sc, 1);
        let d = distance(&f, pivot, ball);
        assert!((d - 2.0).abs() < 0.1, "the rod holds 2: {d}");
    }
    let (mut f, pivot, ball) = hanging(JOINT_ROPE, FPVec3::new(FP::ZERO, fp!(-1), FP::ZERO), fp!(3));
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 10);
    assert!(distance(&f, pivot, ball) < 2.9, "slack rope: the ball falls freely at first");
    for _ in 0..200 {
        run(&mut f, &mut sc, 1);
        assert!(distance(&f, pivot, ball) < 3.12, "the rope holds the ball within 3");
    }
    assert!((distance(&f, pivot, ball) - 3.0).abs() < 0.1, "the ball hangs at the rope's end");
}

#[test]
fn joints_are_deterministic_across_runs_and_rollback() {
    let scene = || hanging(JOINT_BALL, FPVec3::new(fp!(3), fp!(1), fp!(-1)), FP::ZERO).0;
    let (mut a, mut b) = (scene(), scene());
    let (mut sa, mut sb) = (Scratch::new(), Scratch::new());
    run(&mut a, &mut sa, 60);
    let snapshot = a.clone();
    run(&mut a, &mut sa, 60);
    run(&mut b, &mut sb, 120);
    assert_eq!(a.checksum(), b.checksum(), "two runs agree");
    let ahead = a.checksum();
    a.copy_from(&snapshot);
    run(&mut a, &mut Scratch::new(), 60);
    assert_eq!(a.checksum(), ahead, "a rollback replays to the same state");
}

#[test]
fn a_frame_without_joints_registered_steps_as_before() {
    let mut f = new_frame(false);
    let shape = Shape::sphere(fp!(0.2));
    let ball = spawn_body(&mut f, Body::new_dynamic(FPVec3::new(FP::ZERO, fp!(5), FP::ZERO), &shape, FP::ONE), Collider::new(shape));
    run(&mut f, &mut Scratch::new(), 30);
    assert!(y(&f, ball) < 5.0, "it falls");
}

fn v3(x: FP, y: FP, z: FP) -> FPVec3 {
    FPVec3::new(x, y, z)
}

fn f(v: FP) -> f64 {
    v.raw() as f64 / 65536.0
}

/// Turn of `q` about z in radians, for a rotation that stays about z.
fn turn_z(q: FPQuat) -> f64 {
    2.0 * f(q.z).atan2(f(q.w))
}

/// A ball 3 from a fixed pivot at (0, 10, 0), level with it, on a hinge about z.
fn door(joint: impl Fn(Joint) -> Joint) -> (Frame, Entity, Entity) {
    let mut fr = new_frame(true);
    let pivot = spawn_body(&mut fr, Body::new_static(v3(FP::ZERO, fp!(10), FP::ZERO)), Collider::new(Shape::sphere(fp!(0.1))));
    let shape = Shape::sphere(fp!(0.2));
    let ball = spawn_body(&mut fr, Body::new_dynamic(v3(fp!(3), fp!(10), FP::ZERO), &shape, FP::ONE), Collider::new(shape));
    let z = v3(FP::ZERO, FP::ZERO, FP::ONE);
    add_joint(&mut fr, joint(Joint::hinge(pivot, ball, FPVec3::ZERO, v3(fp!(-3), FP::ZERO, FP::ZERO), z)));
    (fr, pivot, ball)
}

#[test]
fn a_hinge_swings_only_in_its_plane() {
    let (mut fr, pivot, ball) = door(|j| j);
    // A sideways push would swing it out of the plane on a ball joint.
    fr.get_mut::<Body>(ball).unwrap().vel = v3(FP::ZERO, FP::ZERO, fp!(3));
    let mut sc = Scratch::new();
    let mut lowest = f64::MAX;
    for _ in 0..90 {
        run(&mut fr, &mut sc, 1);
        let b = *fr.get::<Body>(ball).unwrap();
        assert!(f(b.pos.z).abs() < 0.05, "it stays in the hinge plane: z {}", f(b.pos.z));
        assert!((distance(&fr, pivot, ball) - 3.0).abs() < 0.1, "the anchors stay together");
        let axis = b.rot.rotate_vec3(v3(FP::ZERO, FP::ZERO, FP::ONE));
        assert!(f(axis.z) > 0.995, "its own axis stays on the hinge axis: {}", f(axis.z));
        lowest = lowest.min(y(&fr, ball));
    }
    assert!(lowest < 7.5, "it swings down under the pivot: lowest {lowest}");
}

/// A wheel turning about y on a fixed hub at its center, which does not collide with it.
fn wheel(joint: impl Fn(Joint) -> Joint) -> (Frame, Entity) {
    let mut fr = new_frame(true);
    let hub = spawn_body(&mut fr, Body::new_static(v3(FP::ZERO, fp!(10), FP::ZERO)), Collider { mask: 0, ..Collider::new(Shape::sphere(fp!(0.1))) });
    let shape = Shape::sphere(fp!(1));
    let wheel = spawn_body(&mut fr, Body::new_dynamic(v3(FP::ZERO, fp!(10), FP::ZERO), &shape, FP::ONE), Collider::new(shape));
    let y_axis = v3(FP::ZERO, FP::ONE, FP::ZERO);
    add_joint(&mut fr, joint(Joint::hinge(hub, wheel, FPVec3::ZERO, FPVec3::ZERO, y_axis)));
    (fr, wheel)
}

#[test]
fn a_hinge_motor_spins_a_wheel_up_to_its_speed() {
    let (mut fr, w) = wheel(|j| j.with_motor(fp!(2), fp!(100)));
    run(&mut fr, &mut Scratch::new(), 30);
    let omega = fr.get::<Body>(w).unwrap().omega;
    assert!((f(omega.y) - 2.0).abs() < 0.02, "the motor holds 2 rad/s about y: {}", f(omega.y));
    assert!(f(omega.x).abs() < 0.01 && f(omega.z).abs() < 0.01, "and turns about nothing else");
}

#[test]
fn a_hinge_limit_holds_a_swing_between_its_angles() {
    let (mut fr, pivot, ball) = door(|j| j.with_limits(fp!(-0.5), fp!(0.5)));
    let mut sc = Scratch::new();
    let mut lowest = f64::MAX;
    for _ in 0..120 {
        run(&mut fr, &mut sc, 1);
        let a = turn_z(fr.get::<Body>(ball).unwrap().rot);
        assert!((-0.55..=0.55).contains(&a), "the swing stays within the limits: {a}");
        assert!((distance(&fr, pivot, ball) - 3.0).abs() < 0.1, "the anchors stay together");
        lowest = lowest.min(a);
    }
    assert!(lowest < -0.45, "it reaches the lower limit: {lowest}");
    let rest = turn_z(fr.get::<Body>(ball).unwrap().rot);
    assert!((rest + 0.5).abs() < 0.03, "it comes to rest on the lower limit: {rest}");
}

#[test]
fn a_hinge_is_deterministic_across_rollback() {
    let scene = || wheel(|j| j.with_motor(fp!(3), fp!(5)).with_limits(fp!(-1), fp!(1))).0;
    let mut a = scene();
    let mut sa = Scratch::new();
    run(&mut a, &mut sa, 20);
    let snapshot = a.clone();
    run(&mut a, &mut sa, 40);
    let ahead = a.checksum();
    let mut b = scene();
    run(&mut b, &mut Scratch::new(), 60);
    assert_eq!(b.checksum(), ahead, "two runs agree");
    a.copy_from(&snapshot);
    run(&mut a, &mut Scratch::new(), 40);
    assert_eq!(a.checksum(), ahead, "a rollback replays to the same state");
}

/// A ball 3 from a fixed pivot at (0, 10, 0) along +x on a ball joint, limited by `joint`.
fn arm(joint: impl Fn(Joint) -> Joint) -> (Frame, Entity, Entity) {
    let mut fr = new_frame(true);
    let pivot = spawn_body(&mut fr, Body::new_static(v3(FP::ZERO, fp!(10), FP::ZERO)), Collider::new(Shape::sphere(fp!(0.1))));
    let shape = Shape::sphere(fp!(0.2));
    let ball = spawn_body(&mut fr, Body::new_dynamic(v3(fp!(3), fp!(10), FP::ZERO), &shape, FP::ONE), Collider::new(shape));
    add_joint(&mut fr, joint(Joint::new(JOINT_BALL, pivot, ball, FPVec3::ZERO, v3(fp!(-3), FP::ZERO, FP::ZERO), FP::ZERO)));
    (fr, pivot, ball)
}

/// Angle in radians between +x and the direction from `a` to `b`.
fn off_x(fr: &Frame, a: Entity, b: Entity) -> f64 {
    let d = fr.get::<Body>(b).unwrap().pos - fr.get::<Body>(a).unwrap().pos;
    (f(d.x) / distance(fr, a, b)).clamp(-1.0, 1.0).acos()
}

#[test]
fn a_ball_joint_cone_holds_the_swing_in_every_direction() {
    let x = v3(FP::ONE, FP::ZERO, FP::ZERO);
    let (mut fr, pivot, ball) = arm(|j| j.with_cone(x, fp!(0.5), fp!(-3), fp!(3)));
    // Down under gravity and sideways from the push.
    fr.get_mut::<Body>(ball).unwrap().vel = v3(FP::ZERO, FP::ZERO, fp!(4));
    let mut sc = Scratch::new();
    let mut widest = 0.0f64;
    for _ in 0..120 {
        run(&mut fr, &mut sc, 1);
        let swing = off_x(&fr, pivot, ball);
        assert!(swing < 0.56, "the swing stays inside the cone: {swing}");
        assert!((distance(&fr, pivot, ball) - 3.0).abs() < 0.1, "the anchors stay together");
        widest = widest.max(swing);
    }
    assert!(widest > 0.45, "it reaches the edge of the cone: {widest}");
    let rest = off_x(&fr, pivot, ball);
    assert!((rest - 0.5).abs() < 0.04, "it comes to rest on the edge below the pivot: {rest}");
}

#[test]
fn a_ball_joint_twist_stays_between_its_angles() {
    let mut fr = new_frame(true);
    let hub = spawn_body(&mut fr, Body::new_static(v3(FP::ZERO, fp!(10), FP::ZERO)), Collider { mask: 0, ..Collider::new(Shape::sphere(fp!(0.1))) });
    let shape = Shape::sphere(fp!(1));
    let w = spawn_body(&mut fr, Body::new_dynamic(v3(FP::ZERO, fp!(10), FP::ZERO), &shape, FP::ONE), Collider::new(shape));
    let y_axis = v3(FP::ZERO, FP::ONE, FP::ZERO);
    add_joint(&mut fr, Joint::new(JOINT_BALL, hub, w, FPVec3::ZERO, FPVec3::ZERO, FP::ZERO).with_cone(y_axis, fp!(1), fp!(-0.3), fp!(0.3)));
    fr.get_mut::<Body>(w).unwrap().omega = v3(FP::ZERO, fp!(5), FP::ZERO);
    let mut sc = Scratch::new();
    for _ in 0..60 {
        run(&mut fr, &mut sc, 1);
        let q = fr.get::<Body>(w).unwrap().rot;
        let twist = 2.0 * f(q.y).atan2(f(q.w));
        assert!((-0.33..=0.33).contains(&twist), "the twist stays within its range: {twist}");
    }
    let q = fr.get::<Body>(w).unwrap().rot;
    let twist = 2.0 * f(q.y).atan2(f(q.w));
    assert!((twist - 0.3).abs() < 0.03, "it stops on the upper twist: {twist}");
}

#[test]
fn inside_its_cone_a_ball_joint_moves_as_a_free_one() {
    let x = v3(FP::ONE, FP::ZERO, FP::ZERO);
    let (mut free, _, a) = arm(|j| j);
    let (mut wide, _, b) = arm(|j| j.with_cone(x, fp!(3), fp!(-3), fp!(3)));
    run(&mut free, &mut Scratch::new(), 40);
    run(&mut wide, &mut Scratch::new(), 40);
    assert_eq!(*free.get::<Body>(a).unwrap(), *wide.get::<Body>(b).unwrap(), "a bound far away adds nothing");
}

#[test]
fn a_ball_joint_cone_is_deterministic_across_rollback() {
    let x = v3(FP::ONE, FP::ZERO, FP::ZERO);
    let scene = || {
        let (mut fr, _, ball) = arm(|j| j.with_cone(x, fp!(0.4), fp!(-0.2), fp!(0.2)));
        fr.get_mut::<Body>(ball).unwrap().omega = v3(fp!(4), FP::ZERO, fp!(1));
        fr
    };
    let mut a = scene();
    let mut sa = Scratch::new();
    run(&mut a, &mut sa, 30);
    let snapshot = a.clone();
    run(&mut a, &mut sa, 40);
    let ahead = a.checksum();
    let mut b = scene();
    run(&mut b, &mut Scratch::new(), 70);
    assert_eq!(b.checksum(), ahead, "two runs agree");
    a.copy_from(&snapshot);
    run(&mut a, &mut Scratch::new(), 40);
    assert_eq!(a.checksum(), ahead, "a rollback replays to the same state");
}

#[test]
fn a_joint_added_as_a_plain_component_is_recorded_at_its_first_step() {
    let limits = |j: Joint| j.with_limits(fp!(-0.5), fp!(0.5));
    let (mut added, _, _) = door(limits);
    // The same door with the joint put on an entity directly, as a scene file or the inspector does.
    let mut plain = new_frame(true);
    let pivot = spawn_body(&mut plain, Body::new_static(v3(FP::ZERO, fp!(10), FP::ZERO)), Collider::new(Shape::sphere(fp!(0.1))));
    let shape = Shape::sphere(fp!(0.2));
    let ball = spawn_body(&mut plain, Body::new_dynamic(v3(fp!(3), fp!(10), FP::ZERO), &shape, FP::ONE), Collider::new(shape));
    let z = v3(FP::ZERO, FP::ZERO, FP::ONE);
    let e = plain.spawn();
    plain.add(e, limits(Joint::hinge(pivot, ball, FPVec3::ZERO, v3(fp!(-3), FP::ZERO, FP::ZERO), z)));
    assert_eq!(plain.get::<Joint>(e).unwrap().ref_a, FPVec3::ZERO, "not recorded before stepping");
    run(&mut added, &mut Scratch::new(), 60);
    run(&mut plain, &mut Scratch::new(), 60);
    assert_eq!(added.checksum(), plain.checksum(), "both doors swing the same and hold the same limits");
}

#[test]
fn a_negative_motor_torque_limits_like_its_magnitude_and_zero_torque_does_nothing() {
    let spun = |torque: FP| {
        let (mut fr, w) = wheel(|j| j.with_motor(fp!(2), torque));
        run(&mut fr, &mut Scratch::new(), 30);
        fr.get::<Body>(w).unwrap().omega.y
    };
    let positive = spun(fp!(0.5));
    let omega = f(positive);
    assert!(omega > 0.1 && omega < 1.9, "the weak motor is still speeding up: {omega}");
    assert_eq!(spun(fp!(-0.5)), positive, "a negative torque limits like its magnitude");
    assert!((f(spun(fp!(-100))) - 2.0).abs() < 0.02, "a strong negative torque still reaches the speed");
    assert!(f(spun(FP::ZERO)).abs() < 0.01, "no torque turns nothing");
}
