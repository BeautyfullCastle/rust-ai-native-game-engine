//! Determinism, rollback, serialization, physical sanity and query tests
//! for `orr_physics`. Tests whose names start with `golden_` also run on
//! wasm32-wasip1 in the determinism CI.
use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Entity, Frame};
use orr_fp::{fp, FPVec2, FrameRng, FP};
use orr_physics::{
    circle_cast, init, move_and_slide, raycast, CharacterParams, register, spawn_body, step, Body, Collider, PhysicsConfig, PhysicsSystem, QueryFilter,
    Scratch, Shape, TriggerEvent, BODY_DYNAMIC, TRIGGER_ENTER, TRIGGER_EXIT,
};
use orr_sim::{Game, SimCommand, Simulation, System, TickInputs};

/// Checksum of the scripted scene in [`golden_scene`] after `GOLDEN_TICKS`.
const GOLDEN_CHECKSUM: u64 = 0x7ed61bde26ce839e;
const GOLDEN_TICKS: u32 = 300;

fn new_frame(cfg: PhysicsConfig) -> Frame {
    let mut b = ComponentRegistryBuilder::new();
    register(&mut b);
    let mut f = Frame::new(b.build());
    init(&mut f, cfg);
    f
}

fn v2(x: FP, y: FP) -> FPVec2 {
    FPVec2::new(x, y)
}

fn ground(frame: &mut Frame) -> Entity {
    spawn_body(
        frame,
        Body::new_static(v2(FP::ZERO, -FP::HALF), FP::ZERO),
        Collider::new(Shape::box_shape(fp!(200), FP::HALF)),
    )
}

fn box_body(frame: &mut Frame, x: FP, y: FP, hx: FP, hy: FP) -> Entity {
    let shape = Shape::box_shape(hx, hy);
    spawn_body(frame, Body::new_dynamic(v2(x, y), &shape, FP::ONE), Collider::new(shape))
}

fn ball(frame: &mut Frame, x: FP, y: FP, r: FP, e: FP) -> Entity {
    let shape = Shape::circle(r);
    spawn_body(frame, Body::new_dynamic(v2(x, y), &shape, FP::ONE), Collider::new(shape).with_restitution(e))
}

fn regular_polygon(n: u32, r: FP) -> Shape {
    let pts: Vec<FPVec2> = (0..n)
        .map(|k| FPVec2::from_angle(FP::TWO_PI * FP::from_int(k as i32) / FP::from_int(n as i32)) * r)
        .collect();
    Shape::polygon(&pts).expect("regular polygon")
}

fn run(frame: &mut Frame, sc: &mut Scratch, ticks: u32) {
    let mut events = Vec::new();
    for _ in 0..ticks {
        events.clear();
        frame.set_tick(frame.tick() + 1);
        step(frame, sc, &mut events);
    }
}

/// ~300 bodies: walls, box stack, pyramid, bouncing circles, random
/// rotated boxes and polygons, a kinematic mover and a sensor zone.
fn golden_scene() -> Frame {
    let mut f = new_frame(PhysicsConfig::default());
    ground(&mut f);
    for sx in [-1, 1] {
        spawn_body(
            &mut f,
            Body::new_static(v2(FP::from_int(30 * sx), fp!(40)), FP::ZERO),
            Collider::new(Shape::box_shape(FP::ONE, fp!(40))),
        );
    }
    // Box stack.
    for i in 0..8 {
        box_body(&mut f, fp!(-20), fp!(0.5) + FP::from_int(i), fp!(0.5), fp!(0.5));
    }
    // Pyramid of 10 rows.
    for row in 0..10 {
        for k in 0..(10 - row) {
            let x = fp!(-6) + FP::from_int(k) + FP::HALF * row;
            box_body(&mut f, x, fp!(0.5) + FP::from_int(row), fp!(0.5), fp!(0.5));
        }
    }
    // Kinematic mover and a sensor zone.
    let mut mover = Body::new_kinematic(v2(fp!(10), fp!(1)));
    mover.vel = v2(fp!(-2), FP::ZERO);
    spawn_body(&mut f, mover, Collider::new(Shape::box_shape(fp!(2), fp!(0.5))));
    spawn_body(
        &mut f,
        Body::new_static(v2(fp!(5), fp!(3)), FP::ZERO),
        Collider::new(Shape::box_shape(fp!(2), fp!(3))).sensor(),
    );
    // Random circles, boxes and polygons dropped from above.
    let mut rng = FrameRng::new(0x5EED);
    for _ in 0..130 {
        let x = rng.range_fp(fp!(-12), fp!(12));
        let y = rng.range_fp(fp!(6), fp!(60));
        let r = rng.range_fp(fp!(0.3), fp!(0.7));
        ball(&mut f, x, y, r, fp!(0.4));
    }
    for _ in 0..70 {
        let x = rng.range_fp(fp!(-12), fp!(12));
        let y = rng.range_fp(fp!(6), fp!(60));
        let hx = rng.range_fp(fp!(0.3), fp!(0.8));
        let hy = rng.range_fp(fp!(0.3), fp!(0.8));
        let shape = Shape::box_shape(hx, hy);
        let angle = rng.range_fp(-FP::PI, FP::PI);
        spawn_body(&mut f, Body::new_dynamic(v2(x, y), &shape, FP::ONE).with_angle(angle), Collider::new(shape));
    }
    for i in 0..40 {
        let x = rng.range_fp(fp!(-12), fp!(12));
        let y = rng.range_fp(fp!(6), fp!(60));
        let shape = regular_polygon(3 + (i % 5) as u32, rng.range_fp(fp!(0.4), fp!(0.8)));
        spawn_body(&mut f, Body::new_dynamic(v2(x, y), &shape, FP::ONE), Collider::new(shape).with_friction(fp!(0.3)));
    }
    f
}

// ---- (a) golden determinism ----

#[test]
fn golden_scene_checksum() {
    let mut f = golden_scene();
    assert!(f.alive_count() >= 300, "scene has {} bodies", f.alive_count());
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, GOLDEN_TICKS);
    // The pinned run must be a sane simulation, not a numerical explosion.
    let mut max_speed = FP::ZERO;
    for (_, (b,)) in f.query::<(&Body,)>().filter(|(_, (b,))| b.kind == BODY_DYNAMIC) {
        assert!(b.pos.x.abs() < fp!(30) && b.pos.y > -FP::ONE && b.pos.y < fp!(80), "body out of bounds: {:?}", b.pos);
        max_speed = max_speed.max(b.vel.length());
    }
    assert!(max_speed < fp!(60), "max speed {max_speed}");
    println!("golden checksum: {:#018x}", f.checksum());
    assert_eq!(f.checksum(), GOLDEN_CHECKSUM, "physics golden changed; only update for an intended behavior change");
}

#[test]
fn golden_two_runs_agree() {
    let (mut a, mut b) = (golden_scene(), golden_scene());
    run(&mut a, &mut Scratch::new(), 60);
    let mut sc = Scratch::new();
    run(&mut b, &mut sc, 30);
    run(&mut b, &mut sc, 30);
    assert_eq!(a.checksum(), b.checksum());
}

// ---- (b) rollback ----

#[test]
fn golden_rollback_replay() {
    let mut f = golden_scene();
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 100);
    let snapshot = f.clone();
    run(&mut f, &mut sc, 100);
    let expected = f.checksum();
    assert_ne!(expected, snapshot.checksum());

    f.copy_from(&snapshot);
    assert_eq!(f.checksum(), snapshot.checksum());
    // A fresh Scratch must give the same result: no hidden state.
    run(&mut f, &mut Scratch::new(), 100);
    assert_eq!(f.checksum(), expected);
}

// ---- (c) serialization ----

#[test]
fn golden_serialize_roundtrip() {
    let mut f = golden_scene();
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 100);
    let bytes = f.to_bytes();
    let mut g = Frame::from_bytes(f.registry().clone(), &bytes).expect("decode");
    assert_eq!(g.checksum(), f.checksum());
    run(&mut f, &mut sc, 100);
    run(&mut g, &mut Scratch::new(), 100);
    assert_eq!(f.checksum(), g.checksum());
}

// ---- (d) physical sanity ----

fn body(f: &Frame, e: Entity) -> Body {
    *f.get::<Body>(e).unwrap()
}

fn abs(v: FP) -> FP {
    v.abs()
}

#[test]
fn box_stack_is_stable() {
    let mut f = new_frame(PhysicsConfig::default());
    ground(&mut f);
    let boxes: Vec<Entity> = (0..6).map(|i| box_body(&mut f, FP::ZERO, fp!(0.5) + FP::from_int(i), fp!(0.5), fp!(0.5))).collect();
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 600);
    for (i, &e) in boxes.iter().enumerate() {
        let b = body(&f, e);
        let expect_y = fp!(0.5) + FP::from_int(i as i32);
        assert!(abs(b.pos.x) < fp!(0.05), "box {i} drifted x = {}", b.pos.x);
        assert!(abs(b.pos.y - expect_y) < fp!(0.1), "box {i} y = {} expected {}", b.pos.y, expect_y);
        assert!(abs(b.angle) < fp!(0.02), "box {i} angle = {}", b.angle);
        assert!(b.vel.length() < fp!(0.1), "box {i} speed = {}", b.vel.length());
    }
}

#[test]
fn circle_rests_on_ground() {
    let mut f = new_frame(PhysicsConfig::default());
    ground(&mut f);
    let c = ball(&mut f, FP::ZERO, fp!(3), fp!(0.5), FP::ZERO);
    run(&mut f, &mut Scratch::new(), 240);
    let b = body(&f, c);
    assert!(abs(b.pos.y - fp!(0.5)) < fp!(0.03), "y = {}", b.pos.y);
    assert!(b.vel.length() < fp!(0.05), "speed = {}", b.vel.length());
}

/// Drops a ball from `h0` (center height) and returns the apex of the
/// first bounce (center height) and the maximum height afterwards.
fn bounce_apex(e: FP) -> FP {
    let mut f = new_frame(PhysicsConfig::default());
    ground(&mut f);
    let c = ball(&mut f, FP::ZERO, fp!(5), fp!(0.5), e);
    let mut sc = Scratch::new();
    let mut prev_vy = FP::ZERO;
    let mut bounced = false;
    let mut apex = FP::ZERO;
    for _ in 0..400 {
        run(&mut f, &mut sc, 1);
        let b = body(&f, c);
        if !bounced && prev_vy < FP::ZERO && b.vel.y > FP::ZERO {
            bounced = true;
        }
        if bounced {
            apex = apex.max(b.pos.y);
            if b.vel.y < FP::ZERO {
                break;
            }
        }
        prev_vy = b.vel.y;
    }
    apex
}

#[test]
fn restitution_energy() {
    // Predicted apex (center height): 0.5 + e^2 * 4.5.
    let half = bounce_apex(fp!(0.5));
    let want = fp!(0.5) + fp!(4.5) * fp!(0.25);
    assert!(abs(half - want) < fp!(0.25), "e=0.5 apex {} want about {}", half, want);
    let full = bounce_apex(FP::ONE);
    assert!(full > fp!(4.4) && full < fp!(5.3), "e=1 apex {} (energy must not grow much)", full);
    let dead = bounce_apex(FP::ZERO);
    assert!(dead < fp!(0.6), "e=0 apex {}", dead);
}

#[test]
fn friction_stops_sliding_box() {
    let mut f = new_frame(PhysicsConfig::default());
    ground(&mut f);
    let shape = Shape::box_shape(fp!(0.5), fp!(0.5));
    let e = spawn_body(
        &mut f,
        Body::new_dynamic(v2(FP::ZERO, fp!(0.5)), &shape, FP::ONE).with_velocity(v2(fp!(5), FP::ZERO)),
        Collider::new(shape),
    );
    run(&mut f, &mut Scratch::new(), 240);
    let b = body(&f, e);
    // v^2 / (2 mu g) = 25 / (2 * 0.5 * 10) = 2.5
    assert!(abs(b.pos.x - fp!(2.5)) < fp!(0.4), "slid to x = {}", b.pos.x);
    assert!(b.vel.length() < fp!(0.02), "speed = {}", b.vel.length());
    assert!(abs(b.angle) < fp!(0.05), "angle = {}", b.angle);
}

#[test]
fn slope_holds_with_friction_and_slides_without() {
    // Static ramp rotated by ~0.3 rad, box on top.
    fn setup(mu: FP) -> (Frame, Entity) {
        let mut f = new_frame(PhysicsConfig::default());
        spawn_body(
            &mut f,
            Body::new_static(FPVec2::ZERO, fp!(0.3)),
            Collider::new(Shape::box_shape(fp!(20), FP::HALF)).with_friction(mu),
        );
        let shape = Shape::box_shape(fp!(0.5), fp!(0.5));
        let (s, c) = fp!(0.3).sin_cos();
        // Place on the ramp surface (top face is 0.5 above the center line).
        let along = fp!(0);
        let pos = v2(c * along - s * fp!(1.0), s * along + c * fp!(1.0));
        let e = spawn_body(
            &mut f,
            Body::new_dynamic(pos, &shape, FP::ONE).with_angle(fp!(0.3)),
            Collider::new(shape).with_friction(mu),
        );
        (f, e)
    }
    let (mut hold, eh) = setup(fp!(0.8));
    run(&mut hold, &mut Scratch::new(), 240);
    let start = body(&hold, eh);
    assert!(start.vel.length() < fp!(0.3), "sticky slope speed = {}", start.vel.length());
    let (mut slide, es) = setup(FP::ZERO);
    run(&mut slide, &mut Scratch::new(), 120);
    assert!(body(&slide, es).vel.length() > fp!(2), "frictionless box must slide");
}

#[test]
fn kinematic_platform_carries_and_pushes() {
    let mut f = new_frame(PhysicsConfig::default());
    let mut plat = Body::new_kinematic(v2(FP::ZERO, -FP::HALF));
    plat.vel = v2(fp!(2), FP::ZERO);
    let p = spawn_body(&mut f, plat, Collider::new(Shape::box_shape(fp!(50), FP::HALF)).with_friction(FP::ONE));
    let b = box_body(&mut f, FP::ZERO, fp!(0.5), fp!(0.5), fp!(0.5));
    run(&mut f, &mut Scratch::new(), 120);
    let (pp, bb) = (body(&f, p), body(&f, b));
    assert!(abs(pp.pos.x - fp!(4)) < fp!(0.1), "platform x = {}", pp.pos.x);
    assert!(abs(bb.pos.x - pp.pos.x) < fp!(1.0), "box stayed near platform: {} vs {}", bb.pos.x, pp.pos.x);
    assert!(abs(bb.pos.y - fp!(0.5)) < fp!(0.05));
}

#[test]
fn layer_mask_disables_collision() {
    let mut f = new_frame(PhysicsConfig::default());
    spawn_body(
        &mut f,
        Body::new_static(v2(FP::ZERO, -FP::HALF), FP::ZERO),
        Collider::new(Shape::box_shape(fp!(20), FP::HALF)).with_filter(2, 2),
    );
    let shape = Shape::circle(fp!(0.5));
    let c = spawn_body(
        &mut f,
        Body::new_dynamic(v2(FP::ZERO, fp!(2)), &shape, FP::ONE),
        Collider::new(shape).with_filter(1, 1),
    );
    run(&mut f, &mut Scratch::new(), 90);
    assert!(body(&f, c).pos.y < fp!(-2), "ball must fall through the other layer");
}

#[test]
fn polygon_validation_and_mass() {
    let cw = [v2(FP::ZERO, FP::ZERO), v2(FP::ZERO, FP::ONE), v2(FP::ONE, FP::ONE), v2(FP::ONE, FP::ZERO)];
    assert!(Shape::polygon(&cw).is_none(), "clockwise rejected");
    let dart = [v2(FP::ZERO, FP::ZERO), v2(fp!(2), fp!(1)), v2(fp!(1), fp!(0.2)), v2(FP::ZERO, fp!(2))];
    assert!(Shape::polygon(&dart).is_none(), "concave rejected");
    let too_many: Vec<FPVec2> = (0..9)
        .map(|k| FPVec2::from_angle(FP::TWO_PI * FP::from_int(k) / FP::from_int(9)))
        .collect();
    assert!(Shape::polygon(&too_many).is_none());

    let m = Shape::box_shape(FP::ONE, FP::HALF).mass_data(FP::ONE);
    assert!(abs(m.mass - FP::TWO) < fp!(0.001), "mass {}", m.mass);
    // m (w^2 + h^2) / 12 = 2 * 5 / 12
    assert!(abs(m.inertia - fp!(0.83333)) < fp!(0.002), "inertia {}", m.inertia);
    let c = Shape::circle(FP::ONE).mass_data(FP::ONE);
    assert!(abs(c.mass - FP::PI) < fp!(0.001));
}

#[test]
fn extreme_values_do_not_panic() {
    // Far from the origin and with absurd velocities (debug build checks
    // arithmetic overflow).
    let mut f = new_frame(PhysicsConfig::default());
    let off = fp!(20000);
    spawn_body(
        &mut f,
        Body::new_static(v2(off, off - FP::HALF), FP::ZERO),
        Collider::new(Shape::box_shape(fp!(500), FP::HALF)),
    );
    for i in 0..5 {
        box_body(&mut f, off, off + fp!(0.5) + FP::from_int(i), fp!(0.5), fp!(0.5));
    }
    let shape = Shape::circle(fp!(0.5));
    spawn_body(
        &mut f,
        Body::new_dynamic(v2(off + fp!(3), off + fp!(20)), &shape, FP::ONE)
            .with_velocity(v2(fp!(100000), fp!(-100000))),
        Collider::new(shape),
    );
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 240);
    for (_, (b,)) in f.query::<(&Body,)>() {
        assert!(b.pos.x.abs() < fp!(60000) && b.pos.y.abs() < fp!(60000), "body escaped: {:?}", b.pos);
    }
}

// ---- (e) queries ----

#[test]
fn raycast_shapes() {
    let mut f = new_frame(PhysicsConfig::default());
    let g = ground(&mut f);
    let c = spawn_body(
        &mut f,
        Body::new_static(v2(fp!(5), fp!(1)), FP::ZERO),
        Collider::new(Shape::circle(FP::ONE)),
    );
    let rot = spawn_body(
        &mut f,
        Body::new_static(v2(fp!(-8), fp!(3)), FP::PI / 4),
        Collider::new(Shape::box_shape(FP::ONE, FP::ONE)),
    );
    let flt = QueryFilter::default();

    let hit = raycast(&mut f, v2(FP::ZERO, fp!(10)), v2(FP::ZERO, -FP::ONE), fp!(100), flt).unwrap();
    assert_eq!(hit.entity, g);
    assert!(abs(hit.distance - fp!(10)) < fp!(0.01), "d = {}", hit.distance);
    assert!(abs(hit.normal.y - FP::ONE) < fp!(0.01) && abs(hit.normal.x) < fp!(0.01));
    assert!(abs(hit.point.y) < fp!(0.01));

    let hit = raycast(&mut f, v2(FP::ZERO, fp!(1)), v2(fp!(3), FP::ZERO), fp!(100), flt).unwrap();
    assert_eq!(hit.entity, c);
    assert!(abs(hit.distance - fp!(4)) < fp!(0.01), "d = {}", hit.distance);
    assert!(abs(hit.normal.x + FP::ONE) < fp!(0.01));

    // Rotated box (diamond): tip at distance sqrt(2) from center.
    let hit = raycast(&mut f, v2(fp!(-8), fp!(10)), v2(FP::ZERO, -FP::ONE), fp!(100), flt).unwrap();
    assert_eq!(hit.entity, rot);
    let want = fp!(10) - fp!(3) - fp!(1.41421);
    assert!(abs(hit.distance - want) < fp!(0.02), "d = {} want {}", hit.distance, want);

    // Too short, wrong mask, and a miss.
    assert!(raycast(&mut f, v2(FP::ZERO, fp!(10)), v2(FP::ZERO, -FP::ONE), fp!(5), flt).is_none());
    let none = QueryFilter { mask: 4, include_sensors: false };
    assert!(raycast(&mut f, v2(FP::ZERO, fp!(10)), v2(FP::ZERO, -FP::ONE), fp!(100), none).is_none());
    assert!(raycast(&mut f, v2(fp!(100), fp!(10)), v2(FP::ZERO, FP::ONE), fp!(100), flt).is_none());
    // Origin inside a shape reports distance 0.
    let inside = raycast(&mut f, v2(fp!(5), fp!(1)), v2(FP::ONE, FP::ZERO), fp!(10), flt).unwrap();
    assert_eq!((inside.entity, inside.distance), (c, FP::ZERO));
    // Nearest wins when several are in line.
    let hit = raycast(&mut f, v2(fp!(-20), fp!(2)), v2(FP::ONE, FP::ZERO), fp!(100), flt).unwrap();
    assert_eq!(hit.entity, rot, "rot box at x=-8 is nearer than the circle");
}

#[test]
fn circle_cast_hits() {
    let mut f = new_frame(PhysicsConfig::default());
    let g = ground(&mut f);
    let b = spawn_body(
        &mut f,
        Body::new_static(v2(fp!(10), fp!(0.5)), FP::ZERO),
        Collider::new(Shape::box_shape(FP::HALF, FP::HALF)),
    );
    let c = spawn_body(
        &mut f,
        Body::new_static(v2(fp!(-10), fp!(3)), FP::ZERO),
        Collider::new(Shape::circle(FP::ONE)),
    );
    let flt = QueryFilter::default();

    let hit = circle_cast(&mut f, v2(FP::ZERO, fp!(10)), v2(FP::ZERO, -FP::ONE), fp!(100), FP::HALF, flt).unwrap();
    assert_eq!(hit.entity, g);
    assert!(abs(hit.distance - fp!(9.5)) < fp!(0.01), "d = {}", hit.distance);
    assert!(abs(hit.point.y) < fp!(0.01));

    // Corner hit: path y = 0.8 vs corner (10.5 -> 9.5 side) ... approach from +x side.
    let hit = circle_cast(&mut f, v2(fp!(15), fp!(1.3)), v2(-FP::ONE, FP::ZERO), fp!(100), FP::HALF, flt).unwrap();
    assert_eq!(hit.entity, b);
    // Corner at (10.5, 1.0); dy = 0.3, dx = 0.4 -> center x = 10.9, distance 4.1.
    assert!(abs(hit.distance - fp!(4.1)) < fp!(0.02), "d = {}", hit.distance);
    assert!(abs(hit.normal.x - fp!(0.8)) < fp!(0.02) && abs(hit.normal.y - fp!(0.6)) < fp!(0.02), "n = {:?}", hit.normal);

    // Flat face hit.
    let hit = circle_cast(&mut f, v2(fp!(15), fp!(0.5)), v2(-FP::ONE, FP::ZERO), fp!(100), fp!(0.3), flt).unwrap();
    assert_eq!(hit.entity, b);
    assert!(abs(hit.distance - fp!(4.2)) < fp!(0.02), "d = {}", hit.distance);

    // Circle target: centers 1.5 apart at contact.
    let hit = circle_cast(&mut f, v2(fp!(-20), fp!(3)), v2(FP::ONE, FP::ZERO), fp!(100), FP::HALF, flt).unwrap();
    assert_eq!(hit.entity, c);
    assert!(abs(hit.distance - fp!(8.5)) < fp!(0.02), "d = {}", hit.distance);

    // Starting in overlap.
    let hit = circle_cast(&mut f, v2(fp!(10), fp!(1)), v2(FP::ONE, FP::ZERO), fp!(100), FP::HALF, flt).unwrap();
    assert_eq!(hit.distance, FP::ZERO);
}

// ---- triggers through orr_sim events ----

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Nop {
    _x: u32,
}
impl SimCommand for Nop {
    fn encode(&self, out: &mut Vec<u8>) {
        orr_sim::encode_pod(self, out)
    }
    fn decode(bytes: &[u8]) -> Option<Self> {
        orr_sim::decode_pod(bytes)
    }
}

struct PhysGame;
impl Game for PhysGame {
    type Input = u32;
    type Command = Nop;
    type Event = TriggerEvent;
    type Config = ();

    fn register(builder: &mut ComponentRegistryBuilder) {
        register(builder);
    }

    fn setup(frame: &mut Frame, _: &()) {
        init(frame, PhysicsConfig::default());
        // Sensor slab at y = 5, ball falling through it (no ground).
        spawn_body(
            frame,
            Body::new_static(v2(FP::ZERO, fp!(5)), FP::ZERO),
            Collider::new(Shape::box_shape(fp!(3), FP::ONE)).sensor(),
        );
        let shape = Shape::circle(FP::HALF);
        spawn_body(frame, Body::new_dynamic(v2(fp!(0.5), fp!(10)), &shape, FP::ONE), Collider::new(shape));
    }

    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(PhysicsSystem::<PhysGame>::new(|e| e))]
    }
}

fn sim_step(sim: &mut Simulation<PhysGame>, log: &mut Vec<(u64, u32)>) {
    let t = sim.tick() + 1;
    let inputs = TickInputs::<u32, Nop>::new(t, 1);
    for ev in sim.step(&inputs) {
        log.push((ev.key.tick, ev.payload.kind));
    }
}

#[test]
fn trigger_enter_exit_events() {
    let mut sim = Simulation::<PhysGame>::new((), 60, 1);
    let mut log = Vec::new();
    for _ in 0..90 {
        sim_step(&mut sim, &mut log);
    }
    let kinds: Vec<u32> = log.iter().map(|e| e.1).collect();
    assert_eq!(kinds, vec![TRIGGER_ENTER, TRIGGER_EXIT], "log = {log:?}");
    // Falling from y=10 at g=10: reaches the slab top (y=6, ball edge at 6.5)
    // after about sqrt(2*3.5/10) = 0.84 s and leaves the bottom (y=4, edge 3.5)
    // near 1.4 s.
    assert!((46..=54).contains(&log[0].0), "enter tick {}", log[0].0);
    assert!((65..=72).contains(&log[1].0), "exit tick {}", log[1].0);
}

#[test]
fn trigger_events_repeat_after_rollback() {
    let mut sim = Simulation::<PhysGame>::new((), 60, 1);
    let mut log = Vec::new();
    for _ in 0..40 {
        sim_step(&mut sim, &mut log);
    }
    let snapshot = sim.frame().clone();
    log.clear();
    for _ in 0..50 {
        sim_step(&mut sim, &mut log);
    }
    let first = log.clone();
    let sum = sim.checksum();
    assert!(!first.is_empty());

    sim.restore(&snapshot);
    log.clear();
    for _ in 0..50 {
        sim_step(&mut sim, &mut log);
    }
    assert_eq!(first, log);
    assert_eq!(sum, sim.checksum());
}

#[test]
fn despawned_sensor_overlap_reports_exit() {
    let mut f = new_frame(PhysicsConfig::default());
    let s = spawn_body(
        &mut f,
        Body::new_static(FPVec2::ZERO, FP::ZERO),
        Collider::new(Shape::box_shape(FP::TWO, FP::TWO)).sensor(),
    );
    let shape = Shape::circle(FP::HALF);
    let c = spawn_body(
        &mut f,
        Body::new_static(v2(FP::ONE, FP::ZERO), FP::ZERO),
        Collider::new(shape),
    );
    let mut sc = Scratch::new();
    let mut ev = Vec::new();
    step(&mut f, &mut sc, &mut ev);
    assert_eq!(ev.len(), 1);
    assert_eq!((ev[0].kind, ev[0].a, ev[0].b), (TRIGGER_ENTER, s, c));
    ev.clear();
    step(&mut f, &mut sc, &mut ev);
    assert!(ev.is_empty(), "no repeat while overlapping");
    f.despawn(c);
    step(&mut f, &mut sc, &mut ev);
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].kind, TRIGGER_EXIT);
}

#[test]
fn character_controller_moves_and_slides() {
    let mut f = new_frame(PhysicsConfig::default());
    let g = ground(&mut f);
    spawn_body(
        &mut f,
        Body::new_static(v2(fp!(5), fp!(5)), FP::ZERO),
        Collider::new(Shape::box_shape(FP::HALF, fp!(5))),
    );
    let p = CharacterParams::new(FP::HALF);
    let me = Entity::NONE;

    let m = move_and_slide(&mut f, me, v2(FP::ZERO, fp!(3)), v2(FP::ZERO, -fp!(4)), &p);
    assert!(m.grounded && abs(m.ground_normal.y - FP::ONE) < fp!(0.01));
    assert!(abs(m.pos.y - fp!(0.51)) < fp!(0.02), "landed at y = {}", m.pos.y);

    let m = move_and_slide(&mut f, me, v2(FP::ZERO, fp!(0.51)), v2(fp!(10), FP::ZERO), &p);
    assert!(!m.grounded && m.hits >= 1);
    assert!(abs(m.pos.x - fp!(3.99)) < fp!(0.03), "stopped at x = {}", m.pos.x);
    assert!(abs(m.pos.y - fp!(0.51)) < fp!(0.03), "y = {}", m.pos.y);

    let m = move_and_slide(&mut f, me, v2(fp!(3.5), fp!(2)), v2(fp!(2), fp!(2)), &p);
    assert!(abs(m.pos.x - fp!(3.99)) < fp!(0.03), "x = {}", m.pos.x);
    assert!(abs(m.pos.y - fp!(4)) < fp!(0.1), "slid to y = {}", m.pos.y);

    // The character's own collider can be ignored.
    let m = move_and_slide(&mut f, g, v2(FP::ZERO, fp!(0.5)), v2(FP::ZERO, -fp!(2)), &p);
    assert!(m.pos.y < fp!(-1), "ignoring ground, character falls through: {}", m.pos.y);
}
