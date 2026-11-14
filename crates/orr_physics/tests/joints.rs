//! Pins, rods and ropes between bodies, and their determinism.
// Distances are measured in f64 for the assertions only; the simulation stays fixed point.
#![allow(clippy::float_arithmetic)]
use orr_ecs::{ComponentRegistryBuilder, Entity, Frame};
use orr_fp::{fp, FPVec2, FP};
use orr_physics::{add_joint, init, register, register_joints, spawn_body, step, Body, Collider, Joint, PhysicsConfig, Scratch, Shape, JOINT_PIN, JOINT_ROD, JOINT_ROPE};

fn new_frame() -> Frame {
    let mut b = ComponentRegistryBuilder::new();
    register(&mut b);
    register_joints(&mut b);
    let mut f = Frame::new(b.build());
    init(&mut f, PhysicsConfig::default());
    f
}

fn run(frame: &mut Frame, sc: &mut Scratch, ticks: u32) {
    let mut events = Vec::new();
    for _ in 0..ticks {
        frame.set_tick(frame.tick() + 1);
        step(frame, sc, &mut events);
    }
}

/// A small fixed pivot at (0, 10) and a ball `offset` from it, joined by `kind` of `length`.
fn hanging(kind: u32, offset: FPVec2, length: FP) -> (Frame, Entity, Entity) {
    let mut f = new_frame();
    let pivot = spawn_body(&mut f, Body::new_static(FPVec2::new(FP::ZERO, fp!(10)), FP::ZERO), Collider::new(Shape::circle(fp!(0.1))));
    let shape = Shape::circle(fp!(0.2));
    let ball = spawn_body(&mut f, Body::new_dynamic(FPVec2::new(FP::ZERO, fp!(10)) + offset, &shape, FP::ONE), Collider::new(shape));
    let anchor_b = if kind == JOINT_PIN { -offset } else { FPVec2::ZERO };
    add_joint(&mut f, Joint::new(kind, pivot, ball, FPVec2::ZERO, anchor_b, length));
    (f, pivot, ball)
}

fn distance(f: &Frame, a: Entity, b: Entity) -> f64 {
    let (pa, pb) = (f.get::<Body>(a).unwrap().pos, f.get::<Body>(b).unwrap().pos);
    let d = pb - pa;
    let (x, y) = (d.x.raw() as f64 / 65536.0, d.y.raw() as f64 / 65536.0);
    (x * x + y * y).sqrt()
}

fn y(f: &Frame, e: Entity) -> f64 {
    f.get::<Body>(e).unwrap().pos.y.raw() as f64 / 65536.0
}

#[test]
fn a_pinned_ball_swings_on_its_pivot() {
    // The ball starts level with the pivot; the pin's anchor on the ball sits on the pivot.
    let (mut f, pivot, ball) = hanging(JOINT_PIN, FPVec2::new(fp!(3), FP::ZERO), FP::ZERO);
    let mut sc = Scratch::new();
    let mut lowest = f64::MAX;
    for _ in 0..90 {
        run(&mut f, &mut sc, 1);
        let d = distance(&f, pivot, ball);
        assert!((d - 3.0).abs() < 0.08, "the ball stays 3 from the pivot: {d}");
        lowest = lowest.min(y(&f, ball));
    }
    assert!(lowest < 7.5, "it swings down under the pivot: lowest {lowest}");
}

#[test]
fn a_rod_keeps_its_length_and_a_rope_only_its_maximum() {
    let (mut f, pivot, ball) = hanging(JOINT_ROD, FPVec2::new(fp!(2), FP::ZERO), fp!(2));
    let mut sc = Scratch::new();
    for _ in 0..120 {
        run(&mut f, &mut sc, 1);
        let d = distance(&f, pivot, ball);
        assert!((d - 2.0).abs() < 0.08, "the rod holds 2: {d}");
    }
    // A ball 1 below the pivot on a 3 long rope falls freely, then hangs at 3.
    let (mut f, pivot, ball) = hanging(JOINT_ROPE, FPVec2::new(FP::ZERO, fp!(-1)), fp!(3));
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 10);
    assert!(distance(&f, pivot, ball) < 2.9, "slack rope: the ball falls freely at first");
    for _ in 0..200 {
        run(&mut f, &mut sc, 1);
        assert!(distance(&f, pivot, ball) < 3.1, "the rope holds the ball within 3");
    }
    assert!((distance(&f, pivot, ball) - 3.0).abs() < 0.08, "the ball hangs at the rope's end");
}

#[test]
fn joints_are_deterministic_across_runs_and_rollback() {
    let scene = || hanging(JOINT_PIN, FPVec2::new(fp!(3), fp!(1)), FP::ZERO).0;
    let (mut a, mut b) = (scene(), scene());
    let (mut sa, mut sb) = (Scratch::new(), Scratch::new());
    run(&mut a, &mut sa, 60);
    let snapshot = a.clone();
    run(&mut a, &mut sa, 60);
    run(&mut b, &mut sb, 120);
    assert_eq!(a.checksum(), b.checksum(), "two runs agree");
    // Back to the snapshot in the same frame, then forward again with fresh scratch.
    let ahead = a.checksum();
    a.copy_from(&snapshot);
    run(&mut a, &mut Scratch::new(), 60);
    let c = a;
    let a_checksum = ahead;
    assert_eq!(c.checksum(), a_checksum, "a rollback replays to the same state");
}

#[test]
fn a_frame_without_joints_registered_steps_as_before() {
    let mut b = ComponentRegistryBuilder::new();
    register(&mut b);
    let mut f = Frame::new(b.build());
    init(&mut f, PhysicsConfig::default());
    let shape = Shape::circle(fp!(0.2));
    let ball = spawn_body(&mut f, Body::new_dynamic(FPVec2::new(FP::ZERO, fp!(5)), &shape, FP::ONE), Collider::new(shape));
    run(&mut f, &mut Scratch::new(), 30);
    assert!(y(&f, ball) < 5.0, "it falls");
}

fn angle(f: &Frame, e: Entity) -> f64 {
    f.get::<Body>(e).unwrap().angle.raw() as f64 / 65536.0
}

/// A wheel pinned at its center to a fixed hub, with no gravity on it.
fn wheel(joint: impl Fn(Entity, Entity) -> Joint) -> (Frame, Entity) {
    let mut f = new_frame();
    // Joined bodies still collide; the hub's empty mask keeps it from rubbing on the wheel.
    let hub = spawn_body(&mut f, Body::new_static(FPVec2::new(FP::ZERO, fp!(10)), FP::ZERO), Collider { mask: 0, ..Collider::new(Shape::circle(fp!(0.1))) });
    let shape = Shape::circle(fp!(1));
    let wheel = spawn_body(&mut f, Body::new_dynamic(FPVec2::new(FP::ZERO, fp!(10)), &shape, FP::ONE), Collider::new(shape));
    add_joint(&mut f, joint(hub, wheel));
    (f, wheel)
}

#[test]
fn a_pin_motor_spins_a_wheel_up_to_its_speed_within_its_torque() {
    let (mut f, w) = wheel(|hub, w| Joint::new(JOINT_PIN, hub, w, FPVec2::ZERO, FPVec2::ZERO, FP::ZERO).with_motor(fp!(2), fp!(100)));
    run(&mut f, &mut Scratch::new(), 30);
    let omega = f.get::<Body>(w).unwrap().omega.raw() as f64 / 65536.0;
    assert!((omega - 2.0).abs() < 0.01, "the strong motor holds 2 rad/s: {omega}");
    // A weak motor only gets part of the way in the same time.
    let (mut f, w) = wheel(|hub, w| Joint::new(JOINT_PIN, hub, w, FPVec2::ZERO, FPVec2::ZERO, FP::ZERO).with_motor(fp!(2), fp!(0.5)));
    run(&mut f, &mut Scratch::new(), 30);
    let omega = f.get::<Body>(w).unwrap().omega.raw() as f64 / 65536.0;
    assert!(omega > 0.1 && omega < 1.9, "the weak motor is still speeding up: {omega}");
}

#[test]
fn a_pin_limit_holds_a_swing_between_its_angles() {
    // A ball level with its pivot swings down; the limit stops it a quarter turn early.
    let mut f = new_frame();
    let pivot = spawn_body(&mut f, Body::new_static(FPVec2::new(FP::ZERO, fp!(10)), FP::ZERO), Collider::new(Shape::circle(fp!(0.1))));
    let shape = Shape::circle(fp!(0.2));
    let ball = spawn_body(&mut f, Body::new_dynamic(FPVec2::new(fp!(3), fp!(10)), &shape, FP::ONE), Collider::new(shape));
    let joint = Joint::new(JOINT_PIN, pivot, ball, FPVec2::ZERO, FPVec2::new(fp!(-3), FP::ZERO), FP::ZERO).with_limits(fp!(-0.5), fp!(0.5));
    add_joint(&mut f, joint);
    let mut sc = Scratch::new();
    let mut lowest = f64::MAX;
    for _ in 0..120 {
        run(&mut f, &mut sc, 1);
        let a = angle(&f, ball);
        assert!((-0.55..=0.55).contains(&a), "the swing stays within the limits: {a}");
        assert!((distance(&f, pivot, ball) - 3.0).abs() < 0.08, "the pin still holds");
        lowest = lowest.min(a);
    }
    assert!(lowest < -0.45, "it reaches the lower limit: {lowest}");
    let rest = angle(&f, ball);
    assert!((rest + 0.5).abs() < 0.03, "it comes to rest on the lower limit: {rest}");
}

#[test]
fn a_motor_and_limits_are_deterministic_across_rollback() {
    let scene = || wheel(|hub, w| Joint::new(JOINT_PIN, hub, w, FPVec2::ZERO, FPVec2::ZERO, FP::ZERO).with_motor(fp!(3), fp!(5)).with_limits(fp!(-1), fp!(1))).0;
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
    let turned = angle(&a, a.dense::<Body>().0[1]);
    assert!((turned - 1.0).abs() < 0.05, "the motor drives the wheel to the upper limit and holds it there: {turned}");
}
