//! Ball joints, rods, ropes, hinges and prismatic joints between 3D bodies, and their determinism.
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

fn gravity() -> FPVec3 {
    PhysicsConfig::default().gravity
}

fn frame_with(gravity: FPVec3) -> Frame {
    let mut b = ComponentRegistryBuilder::new();
    register(&mut b);
    register_joints(&mut b);
    let mut fr = Frame::new(b.build());
    init(&mut fr, PhysicsConfig { gravity, ..PhysicsConfig::default() });
    fr
}

fn x_axis() -> FPVec3 {
    v3(FP::ONE, FP::ZERO, FP::ZERO)
}

fn px(fr: &Frame, e: Entity) -> f64 {
    f(fr.get::<Body>(e).unwrap().pos.x)
}

fn speed_x(fr: &Frame, e: Entity) -> f64 {
    f(fr.get::<Body>(e).unwrap().vel.x)
}

/// A static rail body at (0, 10, 0) that collides with nothing and a ball of radius 0.5 at `start`.
fn rail(gravity: FPVec3, start: FPVec3, joint: impl Fn(Entity, Entity) -> Joint) -> (Frame, Entity, Entity) {
    let mut fr = frame_with(gravity);
    let hub = spawn_body(&mut fr, Body::new_static(v3(FP::ZERO, fp!(10), FP::ZERO)), Collider { mask: 0, ..Collider::new(Shape::sphere(fp!(0.1))) });
    let shape = Shape::sphere(fp!(0.5));
    let ball = spawn_body(&mut fr, Body::new_dynamic(start, &shape, FP::ONE), Collider::new(shape));
    add_joint(&mut fr, joint(hub, ball));
    (fr, hub, ball)
}

fn at_hub() -> FPVec3 {
    v3(FP::ZERO, fp!(10), FP::ZERO)
}

fn slider(a: Entity, b: Entity, anchor_b: FPVec3, axis: FPVec3) -> Joint {
    Joint::prismatic(a, b, FPVec3::ZERO, anchor_b, axis)
}

#[test]
fn a_prismatic_joint_slides_along_its_axis_and_not_across() {
    // Level rail under gravity: the push slides the ball and gravity cannot pull it off.
    let (mut fr, _, ball) = rail(gravity(), at_hub(), |a, b| slider(a, b, FPVec3::ZERO, x_axis()));
    fr.get_mut::<Body>(ball).unwrap().vel = v3(fp!(3), FP::ZERO, fp!(2));
    run(&mut fr, &mut Scratch::new(), 60);
    let p = fr.get::<Body>(ball).unwrap().pos;
    assert!(f(p.x) > 2.5, "it slid along the axis: {}", f(p.x));
    assert!((f(p.y) - 10.0).abs() < 0.05 && f(p.z).abs() < 0.05, "and not across it: y {} z {}", f(p.y), f(p.z));
    // A rail at 45 degrees: gravity slides the ball down it, and it stays on the rail line.
    let diagonal = v3(FP::ONE, FP::ONE, FP::ZERO);
    let (mut fr, _, ball) = rail(gravity(), at_hub(), |a, b| slider(a, b, FPVec3::ZERO, diagonal));
    let mut sc = Scratch::new();
    for _ in 0..90 {
        run(&mut fr, &mut sc, 1);
        let p = fr.get::<Body>(ball).unwrap().pos;
        let across = f(p.x) - (f(p.y) - 10.0);
        assert!(across.abs() < 0.05 && f(p.z).abs() < 0.05, "the ball stays on the rail line: {across}, z {}", f(p.z));
    }
    assert!(px(&fr, ball) < -2.0, "gravity slid it down the rail: {}", px(&fr, ball));
}

#[test]
fn a_prismatic_joint_locks_the_rotation() {
    let (mut fr, _, ball) = rail(FPVec3::ZERO, at_hub(), |a, b| slider(a, b, FPVec3::ZERO, x_axis()));
    fr.get_mut::<Body>(ball).unwrap().omega = v3(fp!(3), fp!(4), fp!(5));
    run(&mut fr, &mut Scratch::new(), 40);
    let q = fr.get::<Body>(ball).unwrap().rot;
    assert!(f(q.x).abs() < 0.03 && f(q.y).abs() < 0.03 && f(q.z).abs() < 0.03, "it does not turn: {q:?}");
    // An anchor off its center must not let the ball swing round the rail either.
    let (mut fr, _, ball) = rail(gravity(), at_hub(), |a, b| slider(a, b, v3(fp!(0.4), FP::ZERO, FP::ZERO), x_axis()));
    run(&mut fr, &mut Scratch::new(), 60);
    let b = *fr.get::<Body>(ball).unwrap();
    assert!(f(b.rot.x).abs() < 0.03 && f(b.rot.y).abs() < 0.03 && f(b.rot.z).abs() < 0.03, "it does not tip: {:?}", b.rot);
    assert!((f(b.pos.y) - 10.0).abs() < 0.05, "and does not sag: {}", f(b.pos.y));
}

#[test]
fn a_prismatic_joint_follows_a_turning_parent() {
    // Two free bodies, the second slid 3 out on the first's axis, the first turning about z.
    let mut fr = frame_with(FPVec3::ZERO);
    let big = Shape::sphere(fp!(0.5));
    let no_contact = |shape| Collider { mask: 0, ..Collider::new(shape) };
    let a = spawn_body(&mut fr, Body::new_dynamic(v3(FP::ZERO, fp!(10), FP::ZERO), &big, fp!(500)), no_contact(big));
    let small = Shape::sphere(fp!(0.5));
    let ball = spawn_body(&mut fr, Body::new_dynamic(v3(fp!(3), fp!(10), FP::ZERO), &small, FP::ONE), no_contact(small));
    add_joint(&mut fr, slider(a, ball, FPVec3::ZERO, x_axis()));
    fr.get_mut::<Body>(a).unwrap().omega = v3(FP::ZERO, FP::ZERO, FP::ONE);
    let mut sc = Scratch::new();
    for _ in 0..60 {
        run(&mut fr, &mut sc, 1);
        let (ba, bb) = (*fr.get::<Body>(a).unwrap(), *fr.get::<Body>(ball).unwrap());
        let axis = ba.rot.rotate_vec3(x_axis());
        let gap = bb.pos - ba.pos;
        let across = gap - axis * gap.dot(axis);
        assert!(across.length().raw() as f64 / 65536.0 < 0.1, "the ball stays on the turning axis: {}", f(across.length()));
        let (ta, tb) = (turn_z(ba.rot), turn_z(bb.rot));
        assert!((ta - tb).abs() < 0.05, "and turns with it: {ta} {tb}");
    }
    assert!(turn_z(fr.get::<Body>(a).unwrap().rot) > 0.5, "the parent really turned");
}

#[test]
fn a_prismatic_limit_stops_the_slide_measured_from_where_it_was_added() {
    // The ball is added to the joint at x = 0.5, so 0 is that pose and the bounds are -0.5 and 2.5.
    for (push, rest) in [(5, 2.5), (-5, -0.5)] {
        let (mut fr, _, ball) = rail(FPVec3::ZERO, v3(fp!(0.5), fp!(10), FP::ZERO), |a, b| slider(a, b, FPVec3::ZERO, x_axis()).with_limits(fp!(-1), fp!(2)));
        fr.get_mut::<Body>(ball).unwrap().vel = v3(FP::from_int(push), FP::ZERO, FP::ZERO);
        let mut sc = Scratch::new();
        for _ in 0..120 {
            run(&mut fr, &mut sc, 1);
            let x = px(&fr, ball);
            assert!((-0.56..=2.56).contains(&x), "the slide stays inside the limits: {x}");
        }
        assert!((px(&fr, ball) - rest).abs() < 0.03, "it rests on the bound {rest}: {}", px(&fr, ball));
        assert!(speed_x(&fr, ball).abs() < 0.05, "and has stopped: {}", speed_x(&fr, ball));
    }
}

#[test]
fn a_prismatic_motor_drives_the_slide_at_its_speed_within_its_force() {
    let (mut fr, _, ball) = rail(FPVec3::ZERO, at_hub(), |a, b| slider(a, b, FPVec3::ZERO, x_axis()).with_motor(fp!(2), fp!(100)));
    run(&mut fr, &mut Scratch::new(), 30);
    assert!((speed_x(&fr, ball) - 2.0).abs() < 0.02, "the strong motor holds 2: {}", speed_x(&fr, ball));
    let v = fr.get::<Body>(ball).unwrap().vel;
    assert!(f(v.y).abs() < 0.01 && f(v.z).abs() < 0.01, "and moves along nothing else");
    // A weak motor pushes with a fixed force: the speed follows force / mass * time.
    let (mut fr, _, ball) = rail(FPVec3::ZERO, at_hub(), |a, b| slider(a, b, FPVec3::ZERO, x_axis()).with_motor(fp!(2), fp!(0.5)));
    let mass = 1.0 / f(fr.get::<Body>(ball).unwrap().inv_mass);
    run(&mut fr, &mut Scratch::new(), 30);
    let seconds = 30.0 * f(PhysicsConfig::default().dt);
    let expected = 0.5 / mass * seconds;
    assert!((speed_x(&fr, ball) - expected).abs() < expected * 0.05, "the force cap holds: {} against {expected}", speed_x(&fr, ball));
    // A motor of speed 0 and plenty of force brakes a moving ball.
    let (mut fr, _, ball) = rail(FPVec3::ZERO, at_hub(), |a, b| slider(a, b, FPVec3::ZERO, x_axis()).with_motor(FP::ZERO, fp!(100)));
    fr.get_mut::<Body>(ball).unwrap().vel = v3(fp!(4), FP::ZERO, FP::ZERO);
    run(&mut fr, &mut Scratch::new(), 10);
    assert!(speed_x(&fr, ball).abs() < 0.01, "a zero-speed motor with force is a brake: {}", speed_x(&fr, ball));
}

#[test]
fn a_prismatic_joint_is_deterministic_across_rollback() {
    let skew = v3(FP::ONE, fp!(0.5), fp!(-0.3));
    let scene = || rail(gravity(), at_hub(), |a, b| slider(a, b, v3(fp!(0.2), FP::ZERO, FP::ZERO), skew).with_motor(fp!(1), fp!(3)).with_limits(fp!(-1), fp!(1))).0;
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
fn a_prismatic_joint_added_as_a_plain_component_is_recorded_at_its_first_step() {
    let limited = |j: Joint| j.with_limits(fp!(-0.5), fp!(0.5));
    let start = v3(fp!(0.3), fp!(10.2), fp!(0.1));
    let (mut added, _, _) = rail(gravity(), start, |a, b| limited(slider(a, b, FPVec3::ZERO, x_axis())));
    // The same rail with the joint put on an entity directly, as a scene file or the inspector does.
    let mut plain = frame_with(gravity());
    let hub = spawn_body(&mut plain, Body::new_static(v3(FP::ZERO, fp!(10), FP::ZERO)), Collider { mask: 0, ..Collider::new(Shape::sphere(fp!(0.1))) });
    let shape = Shape::sphere(fp!(0.5));
    let ball = spawn_body(&mut plain, Body::new_dynamic(start, &shape, FP::ONE), Collider::new(shape));
    let e = plain.spawn();
    plain.add(e, limited(slider(hub, ball, FPVec3::ZERO, x_axis())));
    assert_eq!(plain.get::<Joint>(e).unwrap().axis_b, FPVec3::ZERO, "not recorded before stepping");
    run(&mut added, &mut Scratch::new(), 60);
    run(&mut plain, &mut Scratch::new(), 60);
    assert_eq!(added.checksum(), plain.checksum(), "both rails slide the same and hold the same limits");
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

#[test]
fn a_joint_with_a_zero_axis_is_held_like_a_ball_joint_without_panicking() {
    let zero = FPVec3::ZERO;
    let (mut added, pivot, ball) = door(|j| Joint { axis_a: zero, ..j });
    let (mut coned, ..) = arm(|j| j.with_cone(zero, fp!(0.5), fp!(-1), fp!(1)));
    // The same joint put on an entity directly, as a scene file or the inspector does.
    let mut plain = new_frame(true);
    let p = spawn_body(&mut plain, Body::new_static(v3(FP::ZERO, fp!(10), FP::ZERO)), Collider::new(Shape::sphere(fp!(0.1))));
    let shape = Shape::sphere(fp!(0.2));
    let b = spawn_body(&mut plain, Body::new_dynamic(v3(fp!(3), fp!(10), FP::ZERO), &shape, FP::ONE), Collider::new(shape));
    let e = plain.spawn();
    plain.add(e, Joint::hinge(p, b, FPVec3::ZERO, v3(fp!(-3), FP::ZERO, FP::ZERO), zero).with_limits(fp!(-0.5), fp!(0.5)));
    let mut sc = Scratch::new();
    run(&mut added, &mut sc, 60);
    run(&mut coned, &mut sc, 60);
    run(&mut plain, &mut sc, 60);
    assert!((distance(&added, pivot, ball) - 3.0).abs() < 0.1, "the anchors stay together");
    assert!(y(&added, ball) < 7.5, "it swings down under the pivot");
}

#[test]
fn a_cone_pulls_axes_that_point_opposite_back_inside_its_swing() {
    for angle in [fp!(3.14159), fp!(3.1412)] {
        let mut fr = new_frame(true);
        let at = v3(FP::ZERO, fp!(10), FP::ZERO);
        let hub = spawn_body(&mut fr, Body::new_static(at), Collider { mask: 0, ..Collider::new(Shape::sphere(fp!(0.1))) });
        let shape = Shape::sphere(fp!(1));
        let ball = spawn_body(&mut fr, Body::new_dynamic(at, &shape, FP::ONE), Collider::new(shape));
        let x = v3(FP::ONE, FP::ZERO, FP::ZERO);
        add_joint(&mut fr, Joint::new(JOINT_BALL, hub, ball, FPVec3::ZERO, FPVec3::ZERO, FP::ZERO).with_cone(x, fp!(0.5), fp!(-3), fp!(3)));
        fr.get_mut::<Body>(ball).unwrap().rot = FPQuat::from_axis_angle(v3(FP::ZERO, FP::ONE, FP::ZERO), angle);
        run(&mut fr, &mut Scratch::new(), 90);
        let axis = fr.get::<Body>(ball).unwrap().rot.rotate_vec3(x);
        let swing = f(axis.x).clamp(-1.0, 1.0).acos();
        assert!(swing < 0.6, "the swing is back inside the cone from {}: {swing}", f(angle));
    }
}

/// A ball `start` from a hub at (0, 10, 0) that collides with nothing, joined by `joint`, with `gravity`.
fn tied(gravity: FPVec3, start: FPVec3, joint: impl Fn(Entity, Entity) -> Joint) -> (Frame, Entity) {
    let (f, _, ball) = rail(gravity, start, joint);
    (f, ball)
}

fn spring_joint(a: Entity, b: Entity, hertz: FP, damping: FP) -> Joint {
    Joint::spring(a, b, FPVec3::ZERO, FPVec3::ZERO, fp!(2), hertz, damping)
}

/// The stretch (distance from the hub minus the rest length of 2) after each tick, with no gravity.
fn stretch_over(hertz: FP, damping: FP, ticks: u32) -> Vec<f64> {
    let (mut fr, ball) = tied(FPVec3::ZERO, v3(fp!(3), fp!(10), FP::ZERO), |a, b| spring_joint(a, b, hertz, damping));
    let mut sc = Scratch::new();
    (0..ticks)
        .map(|_| {
            run(&mut fr, &mut sc, 1);
            px(&fr, ball) - 2.0
        })
        .collect()
}

#[test]
fn a_spring_swings_about_its_rest_length_at_its_frequency() {
    for hertz in [1, 2] {
        let s = stretch_over(FP::from_int(hertz), FP::ZERO, 300);
        let crossings: Vec<usize> = (1..s.len()).filter(|&i| s[i - 1] > 0.0 && s[i] <= 0.0).collect();
        assert!(crossings.len() >= 3, "it keeps swinging: {crossings:?}");
        let period = (crossings[2] - crossings[0]) as f64 / 2.0 / 60.0;
        assert!((period * hertz as f64 - 1.0).abs() < 0.1, "the period is near 1/{hertz} s: {period}");
        let (high, low) = s.iter().fold((f64::MIN, f64::MAX), |(h, l), &v| (h.max(v), l.min(v)));
        assert!(high > 0.7 && low < -0.7, "it swings both sides of the rest length: {low}..{high}");
    }
}

#[test]
fn a_hanging_spring_settles_below_its_rest_length_by_its_sag() {
    // Mass 1 at 1 Hz: the sag is g / (2 pi)^2, about 0.253.
    let (mut fr, ball) = tied(gravity(), v3(FP::ZERO, fp!(8), FP::ZERO), |a, b| spring_joint(a, b, FP::ONE, FP::ONE));
    run(&mut fr, &mut Scratch::new(), 600);
    let sag = 10.0 - y(&fr, ball) - 2.0;
    assert!((sag - 0.253).abs() < 0.03, "it hangs a sag below the rest length: {sag}");
}

#[test]
fn a_critically_damped_spring_settles_without_overshoot() {
    let s = stretch_over(FP::ONE, FP::ONE, 240);
    let lowest = s.iter().copied().fold(f64::MAX, f64::min);
    assert!(lowest > -0.05, "it does not swing past the rest length: {lowest}");
    assert!(s[239].abs() < 0.03, "it comes to rest: {}", s[239]);
}

#[test]
fn an_undamped_spring_keeps_oscillating() {
    // The implicit step loses a little energy per swing, so the amplitude shrinks slowly.
    let s = stretch_over(FP::ONE, FP::ZERO, 600);
    let swings = (1..s.len()).filter(|&i| s[i - 1] > 0.0 && s[i] <= 0.0).count();
    assert!(swings >= 9, "it crosses the rest length on every swing for ten seconds: {swings}");
    let later = s[180..240].iter().fold(0.0f64, |m, v| m.max(v.abs()));
    assert!(later > 0.2, "after three swings it still swings wide: {later}");
    let damped = stretch_over(FP::ONE, FP::ONE, 240);
    assert!(damped[180..].iter().all(|v| v.abs() < 0.01), "while a damped one is at rest by then");
}

#[test]
fn a_spring_of_zero_frequency_is_a_rod() {
    let start = v3(fp!(3), fp!(10), FP::ZERO);
    let ((mut r, rb), (mut s, sb)) = (
        tied(gravity(), start, |a, b| Joint::new(JOINT_ROD, a, b, FPVec3::ZERO, FPVec3::ZERO, fp!(2))),
        tied(gravity(), start, |a, b| spring_joint(a, b, FP::ZERO, fp!(0.7))),
    );
    let (mut rs, mut ss) = (Scratch::new(), Scratch::new());
    for _ in 0..120 {
        run(&mut r, &mut rs, 1);
        run(&mut s, &mut ss, 1);
        assert_eq!(r.get::<Body>(rb), s.get::<Body>(sb), "same body state every tick");
    }
}

#[test]
fn a_spring_with_negative_inputs_counts_them_as_zero() {
    let state = |hertz: FP, damping: FP| {
        let (mut fr, ball) = tied(gravity(), v3(fp!(3), fp!(10), FP::ZERO), |a, b| spring_joint(a, b, hertz, damping));
        run(&mut fr, &mut Scratch::new(), 90);
        *fr.get::<Body>(ball).unwrap()
    };
    assert_eq!(state(fp!(-3), fp!(0.5)), state(FP::ZERO, fp!(0.5)), "a negative frequency is rigid");
    assert_eq!(state(fp!(2), fp!(-1)), state(fp!(2), FP::ZERO), "a negative damping ratio is none");
}

#[test]
fn a_spring_is_deterministic_across_rollback() {
    let scene = || tied(gravity(), v3(fp!(3), fp!(10), fp!(1)), |a, b| Joint::spring(a, b, v3(fp!(0.1), FP::ZERO, FP::ZERO), FPVec3::ZERO, fp!(2), fp!(1.5), fp!(0.3))).0;
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
fn a_prismatic_joint_holds_the_free_axes_of_a_body_with_a_locked_axis() {
    // The ball cannot turn about y, yet the joint must still stop its turn about x.
    let (mut fr, _, ball) = rail(FPVec3::ZERO, at_hub(), |a, b| slider(a, b, FPVec3::ZERO, x_axis()));
    {
        let b = fr.get_mut::<Body>(ball).unwrap();
        b.inv_inertia = v3(FP::ONE, FP::ZERO, FP::ONE);
        b.omega = v3(FP::ONE, FP::ZERO, FP::ZERO);
    }
    run(&mut fr, &mut Scratch::new(), 40);
    let b = *fr.get::<Body>(ball).unwrap();
    assert!(f(b.omega.x).abs() < 0.05, "it does not keep turning about x: {}", f(b.omega.x));
    assert!(f(b.rot.x).abs() < 0.03 && f(b.rot.y).abs() < 0.03 && f(b.rot.z).abs() < 0.03, "it does not turn: {:?}", b.rot);
}

#[test]
fn a_hinge_holds_its_axis_for_a_body_with_a_locked_axis() {
    // The door cannot turn about y, yet the hinge must still keep its axis from tipping about x.
    let mut fr = new_frame(true);
    let pivot = spawn_body(&mut fr, Body::new_static(v3(FP::ZERO, fp!(10), FP::ZERO)), Collider { mask: 0, ..Collider::new(Shape::sphere(fp!(0.1))) });
    let shape = Shape::sphere(fp!(0.2));
    let ball = spawn_body(&mut fr, Body::new_dynamic(v3(fp!(3), fp!(10), FP::ZERO), &shape, FP::ONE), Collider { mask: 0, ..Collider::new(shape) });
    let z = v3(FP::ZERO, FP::ZERO, FP::ONE);
    add_joint(&mut fr, Joint::hinge(pivot, ball, FPVec3::ZERO, v3(fp!(-3), FP::ZERO, FP::ZERO), z));
    {
        let b = fr.get_mut::<Body>(ball).unwrap();
        b.inv_inertia = v3(FP::ONE, FP::ZERO, FP::ONE);
        b.omega = v3(FP::ONE, FP::ZERO, FP::ZERO);
    }
    run(&mut fr, &mut Scratch::new(), 40);
    let b = *fr.get::<Body>(ball).unwrap();
    let axis = b.rot.rotate_vec3(z);
    assert!(f(axis.z) > 0.995, "its axis stays on the hinge axis: {}", f(axis.z));
    assert!(f(b.omega.x).abs() < 0.05, "it does not keep turning about x: {}", f(b.omega.x));
}
