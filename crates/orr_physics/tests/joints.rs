//! Pins, rods, ropes and prismatic joints between bodies, and their determinism.
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

fn px(f: &Frame, e: Entity) -> f64 {
    f.get::<Body>(e).unwrap().pos.x.raw() as f64 / 65536.0
}

fn speed_x(f: &Frame, e: Entity) -> f64 {
    f.get::<Body>(e).unwrap().vel.x.raw() as f64 / 65536.0
}

fn bare_frame(gravity: FPVec2) -> Frame {
    let mut b = ComponentRegistryBuilder::new();
    register(&mut b);
    register_joints(&mut b);
    let mut f = Frame::new(b.build());
    init(&mut f, PhysicsConfig { gravity, ..PhysicsConfig::default() });
    f
}

fn gravity() -> FPVec2 {
    PhysicsConfig::default().gravity
}

/// A static rail body at (0, 10) that collides with nothing and a ball on it at (0, 10).
fn rail(gravity: FPVec2, joint: impl Fn(Entity, Entity) -> Joint) -> (Frame, Entity, Entity) {
    let mut f = bare_frame(gravity);
    let at = FPVec2::new(FP::ZERO, fp!(10));
    let hub = spawn_body(&mut f, Body::new_static(at, FP::ZERO), Collider { mask: 0, ..Collider::new(Shape::circle(fp!(0.1))) });
    let shape = Shape::circle(fp!(0.5));
    let ball = spawn_body(&mut f, Body::new_dynamic(at, &shape, FP::ONE), Collider::new(shape));
    add_joint(&mut f, joint(hub, ball));
    (f, hub, ball)
}

fn x_axis() -> FPVec2 {
    FPVec2::new(FP::ONE, FP::ZERO)
}

fn slider(a: Entity, b: Entity, anchor_b: FPVec2, axis: FPVec2) -> Joint {
    Joint::prismatic(a, b, FPVec2::ZERO, anchor_b, axis)
}

#[test]
fn a_prismatic_joint_slides_along_its_axis_and_not_across() {
    // Level rail under gravity: the push slides the ball and gravity cannot pull it off.
    let (mut f, _, ball) = rail(gravity(), |a, b| slider(a, b, FPVec2::ZERO, x_axis()));
    f.get_mut::<Body>(ball).unwrap().vel = FPVec2::new(fp!(3), FP::ZERO);
    run(&mut f, &mut Scratch::new(), 60);
    assert!(px(&f, ball) > 2.5, "it slid along the axis: {}", px(&f, ball));
    assert!((y(&f, ball) - 10.0).abs() < 0.05, "gravity does not pull it off the rail: {}", y(&f, ball));
    // A rail at 45 degrees: gravity slides the ball down it, and it stays on the rail line.
    let diagonal = FPVec2::new(FP::ONE, FP::ONE);
    let (mut f, _, ball) = rail(gravity(), |a, b| slider(a, b, FPVec2::ZERO, diagonal));
    let mut sc = Scratch::new();
    for _ in 0..90 {
        run(&mut f, &mut sc, 1);
        let across = px(&f, ball) - (y(&f, ball) - 10.0);
        assert!(across.abs() < 0.05, "the ball stays on the rail line: {across}");
    }
    assert!(px(&f, ball) < -2.0, "gravity slid it down the rail: {}", px(&f, ball));
}

#[test]
fn a_prismatic_joint_locks_the_rotation() {
    let (mut f, _, ball) = rail(FPVec2::ZERO, |a, b| slider(a, b, FPVec2::ZERO, x_axis()));
    f.get_mut::<Body>(ball).unwrap().omega = fp!(5);
    run(&mut f, &mut Scratch::new(), 30);
    assert!(angle(&f, ball).abs() < 0.05, "it does not turn: {}", angle(&f, ball));
    // An anchor off its center must not let the ball swing round the rail either.
    let (mut f, _, ball) = rail(gravity(), |a, b| slider(a, b, FPVec2::new(fp!(0.4), FP::ZERO), x_axis()));
    run(&mut f, &mut Scratch::new(), 60);
    assert!(angle(&f, ball).abs() < 0.05, "it does not tip: {}", angle(&f, ball));
    assert!((y(&f, ball) - 10.0).abs() < 0.05, "and does not sag: {}", y(&f, ball));
}

#[test]
fn a_prismatic_joint_follows_a_turning_parent() {
    // Two free bodies, the second slid 3 out on the first's axis, the first turning.
    let mut f = bare_frame(FPVec2::ZERO);
    let shape = Shape::circle(fp!(0.5));
    let no_contact = |shape| Collider { mask: 0, ..Collider::new(shape) };
    let a = spawn_body(&mut f, Body::new_dynamic(FPVec2::new(FP::ZERO, fp!(10)), &shape, fp!(500)), no_contact(shape));
    let ball = spawn_body(&mut f, Body::new_dynamic(FPVec2::new(fp!(3), fp!(10)), &shape, FP::ONE), no_contact(shape));
    add_joint(&mut f, slider(a, ball, FPVec2::ZERO, x_axis()));
    f.get_mut::<Body>(a).unwrap().omega = fp!(1);
    let mut sc = Scratch::new();
    for _ in 0..60 {
        run(&mut f, &mut sc, 1);
        let (ba, bb) = (*f.get::<Body>(a).unwrap(), *f.get::<Body>(ball).unwrap());
        let gap = bb.pos - ba.pos;
        let (s, c) = (-(ba.angle.raw() as f64 / 65536.0)).sin_cos();
        let (gx, gy) = (gap.x.raw() as f64 / 65536.0, gap.y.raw() as f64 / 65536.0);
        let across = s * gx + c * gy;
        assert!(across.abs() < 0.1, "the ball stays on the turning axis: {across}");
        assert!(((bb.angle - ba.angle).raw() as f64 / 65536.0).abs() < 0.05, "and turns with it");
    }
    assert!(angle(&f, a) > 0.5, "the parent really turned: {}", angle(&f, a));
}

#[test]
fn a_prismatic_limit_stops_the_slide_measured_from_where_it_was_added() {
    // The ball is added to the joint at x = 0.5, so 0 is that pose and the bounds are -0.5 and 2.5.
    for (push, rest) in [(5, 2.5), (-5, -0.5)] {
        let mut f = bare_frame(FPVec2::ZERO);
        let hub = spawn_body(&mut f, Body::new_static(FPVec2::new(FP::ZERO, fp!(10)), FP::ZERO), Collider { mask: 0, ..Collider::new(Shape::circle(fp!(0.1))) });
        let shape = Shape::circle(fp!(0.5));
        let ball = spawn_body(&mut f, Body::new_dynamic(FPVec2::new(fp!(0.5), fp!(10)), &shape, FP::ONE), Collider::new(shape));
        add_joint(&mut f, slider(hub, ball, FPVec2::ZERO, x_axis()).with_limits(fp!(-1), fp!(2)));
        f.get_mut::<Body>(ball).unwrap().vel = FPVec2::new(FP::from_int(push), FP::ZERO);
        let mut sc = Scratch::new();
        for _ in 0..120 {
            run(&mut f, &mut sc, 1);
            let x = px(&f, ball);
            assert!((-0.56..=2.56).contains(&x), "the slide stays inside the limits: {x}");
        }
        assert!((px(&f, ball) - rest).abs() < 0.03, "it rests on the bound {rest}: {}", px(&f, ball));
        assert!(speed_x(&f, ball).abs() < 0.05, "and has stopped: {}", speed_x(&f, ball));
    }
}

#[test]
fn a_prismatic_motor_drives_the_slide_at_its_speed_within_its_force() {
    let (mut f, _, ball) = rail(FPVec2::ZERO, |a, b| slider(a, b, FPVec2::ZERO, x_axis()).with_motor(fp!(2), fp!(100)));
    run(&mut f, &mut Scratch::new(), 30);
    assert!((speed_x(&f, ball) - 2.0).abs() < 0.01, "the strong motor holds 2: {}", speed_x(&f, ball));
    assert!((y(&f, ball) - 10.0).abs() < 0.01, "and stays on the rail");
    // A weak motor pushes with a fixed force: the speed follows force / mass * time.
    let (mut f, _, ball) = rail(FPVec2::ZERO, |a, b| slider(a, b, FPVec2::ZERO, x_axis()).with_motor(fp!(2), fp!(0.5)));
    let mass = 1.0 / (f.get::<Body>(ball).unwrap().inv_mass.raw() as f64 / 65536.0);
    run(&mut f, &mut Scratch::new(), 30);
    let seconds = 30.0 * PhysicsConfig::default().dt.raw() as f64 / 65536.0;
    let expected = 0.5 / mass * seconds;
    assert!((speed_x(&f, ball) - expected).abs() < expected * 0.05, "the force cap holds: {} against {expected}", speed_x(&f, ball));
    // A motor of speed 0 and plenty of force brakes a moving ball.
    let (mut f, _, ball) = rail(FPVec2::ZERO, |a, b| slider(a, b, FPVec2::ZERO, x_axis()).with_motor(FP::ZERO, fp!(100)));
    f.get_mut::<Body>(ball).unwrap().vel = FPVec2::new(fp!(4), FP::ZERO);
    run(&mut f, &mut Scratch::new(), 10);
    assert!(speed_x(&f, ball).abs() < 0.01, "a zero-speed motor with force is a brake: {}", speed_x(&f, ball));
}

#[test]
fn a_prismatic_joint_is_deterministic_across_rollback() {
    let diagonal = FPVec2::new(FP::ONE, fp!(0.5));
    let scene = || rail(gravity(), |a, b| slider(a, b, FPVec2::new(fp!(0.2), FP::ZERO), diagonal).with_motor(fp!(1), fp!(3)).with_limits(fp!(-1), fp!(1))).0;
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

#[test]
fn a_negative_motor_torque_limits_like_its_magnitude_and_zero_torque_does_nothing() {
    let spun = |torque: FP| {
        let (mut f, w) = wheel(|hub, w| Joint::new(JOINT_PIN, hub, w, FPVec2::ZERO, FPVec2::ZERO, FP::ZERO).with_motor(fp!(2), torque));
        run(&mut f, &mut Scratch::new(), 30);
        f.get::<Body>(w).unwrap().omega
    };
    let positive = spun(fp!(0.5));
    let omega = positive.raw() as f64 / 65536.0;
    assert!(omega > 0.1 && omega < 1.9, "the weak motor is still speeding up: {omega}");
    assert_eq!(spun(fp!(-0.5)), positive, "a negative torque limits like its magnitude");
    assert!((spun(fp!(-100)).raw() as f64 / 65536.0 - 2.0).abs() < 0.01, "a strong negative torque still reaches the speed");
    assert!((spun(FP::ZERO).raw() as f64 / 65536.0).abs() < 0.01, "no torque turns nothing");
}
