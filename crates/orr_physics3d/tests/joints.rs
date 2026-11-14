//! Ball joints, rods and ropes between 3D bodies, and their determinism.
// Distances are measured in f64 for the assertions only; the simulation stays fixed point.
#![allow(clippy::float_arithmetic)]
use orr_ecs::{ComponentRegistryBuilder, Entity, Frame};
use orr_fp::{fp, FPVec3, FP};
use orr_physics3d::{add_joint, init, register, register_joints, spawn_body, step, Body, Collider, Joint, PhysicsConfig, Scratch, Shape, JOINT_BALL, JOINT_ROD, JOINT_ROPE};

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
    add_joint(&mut f, Joint { a: pivot, b: ball, anchor_a: FPVec3::ZERO, anchor_b, length, kind, _pad: 0 });
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
