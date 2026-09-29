//! Determinism, rollback, serialization, physical sanity and query tests
//! for `orr_physics`. Tests whose names start with `golden_` also run on
//! wasm32-wasip1 in the determinism CI.
use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Entity, Frame};
use orr_fp::{fp, FPVec2, FrameRng, FP};
use orr_physics::{
    apply_impulse, circle_cast, init, is_asleep, move_and_slide, move_and_slide_capsule, raycast, shape_cast,
    CapsuleCharacterParams, CharacterParams, register, spawn_body, step, Body, Collider, PhysicsConfig, PhysicsSystem, QueryFilter,
    Scratch, Shape, TriggerEvent, BODY_DYNAMIC, TRIGGER_ENTER, TRIGGER_EXIT,
};
use orr_sim::{Game, SimCommand, Simulation, System, TickInputs};

/// Checksum of the scripted scene in [`golden_scene`] after `GOLDEN_TICKS`.
const GOLDEN_CHECKSUM: u64 = 0x9407b6ed32022f2b;
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


// ---- (f) sleeping ----

fn stack(f: &mut Frame, x: FP, n: i32) -> Vec<Entity> {
    (0..n).map(|i| box_body(f, x, fp!(0.5) + FP::from_int(i), fp!(0.5), fp!(0.5))).collect()
}

fn asleep_count(f: &Frame, es: &[Entity]) -> usize {
    es.iter().filter(|&&e| is_asleep(f, e)).count()
}

#[test]
fn resting_stack_falls_asleep_and_stays_put() {
    let mut f = new_frame(PhysicsConfig::default());
    ground(&mut f);
    let es = stack(&mut f, FP::ZERO, 4);
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 240);
    assert_eq!(asleep_count(&f, &es), 4, "a resting stack must be asleep after 4 s");
    let before: Vec<Body> = es.iter().map(|&e| body(&f, e)).collect();
    run(&mut f, &mut sc, 120);
    let stats = sc.stats();
    assert_eq!((stats.awake, stats.asleep, stats.pairs), (0, 4, 0), "a sleeping world costs no contacts: {stats:?}");
    for (i, &e) in es.iter().enumerate() {
        assert_eq!(body(&f, e), before[i], "sleeping box {i} must not change");
        assert_eq!(body(&f, e).vel, FPVec2::ZERO);
    }
    // All members share one island id: the lowest entity index plus one.
    let ids: Vec<u32> = es.iter().map(|&e| body(&f, e).island).collect();
    assert!(ids.iter().all(|&i| i == ids[0] && i == es[0].index + 1), "ids {ids:?}");
}

#[test]
fn sleeping_can_be_turned_off() {
    let cfg = PhysicsConfig { sleep_ticks: 0, ..PhysicsConfig::default() };
    let mut f = new_frame(cfg);
    ground(&mut f);
    let es = stack(&mut f, FP::ZERO, 3);
    run(&mut f, &mut Scratch::new(), 300);
    assert_eq!(asleep_count(&f, &es), 0);
}

#[test]
fn falling_body_wakes_sleeping_stack() {
    let mut f = new_frame(PhysicsConfig::default());
    ground(&mut f);
    let es = stack(&mut f, FP::ZERO, 3);
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 240);
    assert_eq!(asleep_count(&f, &es), 3);
    // A box lands on the stack from above.
    let shape = Shape::box_shape(fp!(0.5), fp!(0.5));
    let hit = spawn_body(
        &mut f,
        Body::new_dynamic(v2(FP::ZERO, fp!(6)), &shape, FP::ONE),
        Collider::new(shape),
    );
    let mut woke_at = None;
    for t in 0..120 {
        run(&mut f, &mut sc, 1);
        if woke_at.is_none() && asleep_count(&f, &es) == 0 {
            woke_at = Some(t);
        }
    }
    assert!(woke_at.is_some(), "the impact must wake the whole stack");
    // Falling from y=6 to the top of the stack takes about 0.75 s.
    assert!(woke_at.unwrap() > 30, "woke at {woke_at:?}: too early, nothing touched it");
    run(&mut f, &mut sc, 300);
    assert_eq!(asleep_count(&f, &es), 3, "and it falls asleep again");
    assert!(is_asleep(&f, hit));
    assert!(abs(body(&f, hit).pos.y - fp!(3.5)) < fp!(0.15), "box on top: y = {}", body(&f, hit).pos.y);
}

#[test]
fn impulse_wakes_whole_island() {
    let mut f = new_frame(PhysicsConfig::default());
    ground(&mut f);
    let es = stack(&mut f, FP::ZERO, 3);
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 240);
    assert_eq!(asleep_count(&f, &es), 3);
    let p0 = body(&f, es[0]).pos;
    apply_impulse(&mut f, es[0], v2(fp!(6), FP::ZERO), p0);
    assert!(!is_asleep(&f, es[0]));
    run(&mut f, &mut sc, 1);
    assert_eq!(asleep_count(&f, &es), 0, "one step wakes every body of the island");
    run(&mut f, &mut sc, 60);
    assert!(body(&f, es[0]).pos.x > fp!(0.5), "the pushed box moved");
}

#[test]
fn writing_a_velocity_wakes_the_island() {
    let mut f = new_frame(PhysicsConfig::default());
    ground(&mut f);
    let es = stack(&mut f, FP::ZERO, 2);
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 240);
    assert_eq!(asleep_count(&f, &es), 2);
    // Game code writes a velocity straight into the component.
    f.get_mut::<Body>(es[1]).unwrap().vel = v2(fp!(3), FP::ZERO);
    run(&mut f, &mut sc, 1);
    assert_eq!(asleep_count(&f, &es), 0);
    assert!(body(&f, es[1]).pos.x > FP::ZERO);
}

#[test]
fn despawned_support_wakes_the_body_above() {
    let mut f = new_frame(PhysicsConfig::default());
    ground(&mut f);
    let es = stack(&mut f, FP::ZERO, 2);
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 240);
    assert_eq!(asleep_count(&f, &es), 2);
    let y0 = body(&f, es[1]).pos.y;
    f.despawn(es[0]);
    run(&mut f, &mut sc, 30);
    assert!(!is_asleep(&f, es[1]));
    assert!(body(&f, es[1]).pos.y < y0 - fp!(0.5), "box fell: {} -> {}", y0, body(&f, es[1]).pos.y);
}

#[test]
fn moving_kinematic_wakes_and_pushes_sleeper() {
    let mut f = new_frame(PhysicsConfig::default());
    ground(&mut f);
    let b = box_body(&mut f, FP::ZERO, fp!(0.5), fp!(0.5), fp!(0.5));
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 200);
    assert!(is_asleep(&f, b));
    let mut k = Body::new_kinematic(v2(fp!(-4), fp!(0.5)));
    k.vel = v2(fp!(2), FP::ZERO);
    spawn_body(&mut f, k, Collider::new(Shape::box_shape(FP::HALF, FP::HALF)));
    run(&mut f, &mut sc, 240);
    assert!(body(&f, b).pos.x > fp!(2), "pushed to x = {}", body(&f, b).pos.x);
    // A kinematic body that stands still does not wake anything.
    let mut f2 = new_frame(PhysicsConfig::default());
    ground(&mut f2);
    let b2 = box_body(&mut f2, FP::ZERO, fp!(0.5), fp!(0.5), fp!(0.5));
    spawn_body(&mut f2, Body::new_kinematic(v2(fp!(1.0), fp!(0.5))), Collider::new(Shape::box_shape(FP::HALF, FP::HALF)));
    run(&mut f2, &mut sc, 300);
    assert!(is_asleep(&f2, b2));
}

#[test]
fn sleeping_body_in_a_sensor_raises_no_events() {
    let mut f = new_frame(PhysicsConfig::default());
    ground(&mut f);
    spawn_body(
        &mut f,
        Body::new_static(v2(FP::ZERO, fp!(1)), FP::ZERO),
        Collider::new(Shape::box_shape(fp!(3), fp!(2))).sensor(),
    );
    let b = box_body(&mut f, FP::ZERO, fp!(0.5), fp!(0.5), fp!(0.5));
    let mut sc = Scratch::new();
    let mut events = Vec::new();
    let mut kinds = Vec::new();
    for _ in 0..400 {
        events.clear();
        f.set_tick(f.tick() + 1);
        step(&mut f, &mut sc, &mut events);
        kinds.extend(events.iter().map(|e| e.kind));
    }
    assert!(is_asleep(&f, b));
    // One enter for the ground and one for the box (both overlap the zone), never an exit.
    assert_eq!(kinds, vec![TRIGGER_ENTER, TRIGGER_ENTER], "asleep in the zone: no events after the enters");
}

/// Advances with a brand-new `Scratch` every tick.
fn run_fresh(frame: &mut Frame, ticks: u32) {
    let mut events = Vec::new();
    for _ in 0..ticks {
        events.clear();
        frame.set_tick(frame.tick() + 1);
        step(frame, &mut Scratch::new(), &mut events);
    }
}

/// A scattered set of small stacks: they settle, fall asleep, and one
/// gets hit later.
fn sleepy_scene() -> Frame {
    let mut f = new_frame(PhysicsConfig::default());
    ground(&mut f);
    let mut rng = FrameRng::new(99);
    for k in 0..12 {
        let x = FP::from_int(k * 4 - 22) + rng.range_fp(-fp!(0.3), fp!(0.3));
        for i in 0..3 {
            if (k + i) % 3 == 0 {
                ball(&mut f, x, fp!(0.5) + FP::from_int(i) * fp!(1.2), fp!(0.45), FP::ZERO);
            } else {
                box_body(&mut f, x, fp!(0.5) + FP::from_int(i) * fp!(1.1), fp!(0.5), fp!(0.5));
            }
        }
    }
    f
}

#[test]
fn golden_sleep_state_is_scratch_independent() {
    let mut a = sleepy_scene();
    let mut b = sleepy_scene();
    let mut sc = Scratch::new();
    run(&mut a, &mut sc, 500);
    run_fresh(&mut b, 500);
    assert_eq!(a.checksum(), b.checksum(), "state must not depend on Scratch history");
    let asleep = a.query::<(&Body,)>().filter(|(_, (b,))| b.sleep >> 31 != 0).count();
    assert!(asleep >= 30, "most bodies should sleep, only {asleep}");
}

#[test]
fn golden_sleep_rollback_replay() {
    let mut f = sleepy_scene();
    let mut sc = Scratch::new();
    // Snapshot while bodies are falling asleep, then hit one stack.
    run(&mut f, &mut sc, 150);
    let snapshot = f.clone();
    let hit = |f: &mut Frame| {
        let e = f.query::<(&Body,)>().filter(|(_, (b,))| b.kind == BODY_DYNAMIC).map(|(e, _)| e).nth(10).unwrap();
        let p = body(f, e).pos;
        apply_impulse(f, e, v2(fp!(30), fp!(10)), p);
    };
    run(&mut f, &mut sc, 200);
    hit(&mut f);
    run(&mut f, &mut sc, 200);
    let expected = f.checksum();

    f.copy_from(&snapshot);
    run(&mut f, &mut Scratch::new(), 200);
    hit(&mut f);
    run_fresh(&mut f, 200);
    assert_eq!(f.checksum(), expected);

    // Serialization in the middle of the sleep phase.
    let mut g = sleepy_scene();
    run(&mut g, &mut sc, 300);
    let bytes = g.to_bytes();
    let mut h = Frame::from_bytes(g.registry().clone(), &bytes).expect("decode");
    assert_eq!(h.checksum(), g.checksum());
    run(&mut g, &mut sc, 60);
    run(&mut h, &mut Scratch::new(), 60);
    assert_eq!(g.checksum(), h.checksum());
}

#[test]
fn sleeping_matches_awake_physics_for_a_resting_stack() {
    // With sleeping the stack ends where the always-awake run ends.
    let end = |ticks: u32, sleep: bool| {
        let mut cfg = PhysicsConfig::default();
        if !sleep {
            cfg.sleep_ticks = 0;
        }
        let mut f = new_frame(cfg);
        ground(&mut f);
        let es = stack(&mut f, FP::ZERO, 5);
        run(&mut f, &mut Scratch::new(), ticks);
        es.iter().map(|&e| body(&f, e).pos).collect::<Vec<_>>()
    };
    let (a, b) = (end(300, true), end(300, false));
    for (p, q) in a.iter().zip(&b) {
        assert!(abs(p.y - q.y) < fp!(0.05) && abs(p.x - q.x) < fp!(0.05), "{p:?} vs {q:?}");
    }
}

#[test]
fn tall_stack_of_twelve_stays_up() {
    // Guards the solver quality: with the default 8 iterations a stack of
    // 12 unit boxes settles almost where it started, awake or asleep.
    for sleep in [true, false] {
        let mut cfg = PhysicsConfig::default();
        if !sleep {
            cfg.sleep_ticks = 0;
        }
        let mut f = new_frame(cfg);
        ground(&mut f);
        let es = stack(&mut f, FP::ZERO, 12);
        run(&mut f, &mut Scratch::new(), 600);
        let top = body(&f, es[11]);
        assert!(abs(top.pos.y - fp!(11.5)) < fp!(0.3), "sleep={sleep}: top y = {}", top.pos.y);
        assert!(abs(top.pos.x) < fp!(0.3), "sleep={sleep}: top x = {}", top.pos.x);
    }
}

/// Spawn/despawn churn while bodies sleep: the dense component order stops
/// being ascending, and entities that lack a body or a collider show up.
/// With `fresh`, every step gets a brand-new `Scratch`.
fn churn(frame: &mut Frame, ticks: u32, fresh: bool) {
    let mut rng = FrameRng::new(4242);
    let mut sc = Scratch::new();
    let mut events = Vec::new();
    for t in 0..ticks {
        if t % 20 == 10 {
            let victims: Vec<Entity> =
                frame.query::<(&Body, &Collider)>().filter(|(_, (b, _))| b.kind == BODY_DYNAMIC).map(|(e, _)| e).collect();
            if !victims.is_empty() {
                frame.despawn(victims[rng.range_i32(0, victims.len() as i32) as usize]);
            }
            let x = rng.range_fp(fp!(-20), fp!(20));
            box_body(frame, x, fp!(8), fp!(0.5), fp!(0.5));
        }
        if t == 30 {
            // A body without a collider and a collider without a body.
            let e = frame.spawn();
            frame.add(e, Body::new_kinematic(v2(fp!(50), fp!(50))));
            let e = frame.spawn();
            frame.add(e, Collider::new(Shape::circle(FP::HALF)));
        }
        if fresh {
            sc = Scratch::new();
        }
        events.clear();
        frame.set_tick(frame.tick() + 1);
        step(frame, &mut sc, &mut events);
    }
}

#[test]
fn golden_churn_with_sleeping_is_scratch_independent() {
    let (mut a, mut b) = (sleepy_scene(), sleepy_scene());
    churn(&mut a, 400, false);
    churn(&mut b, 400, true);
    assert_eq!(a.checksum(), b.checksum());
    assert!(a.query::<(&Body,)>().all(|(_, (bd,))| bd.pos.y > -FP::ONE && bd.pos.y < fp!(60)), "nothing escaped");
    let asleep = a.query::<(&Body,)>().filter(|(_, (bd,))| bd.sleep >> 31 != 0).count();
    assert!(asleep >= 20, "only {asleep} bodies asleep");
}

#[test]
fn documented_range_limits_stay_inside_the_solver_ranges() {
    // The solver multiplies in 64 bits (debug builds assert on overflow).
    // Push mass, size and speed to the documented limits together.
    let mut f = new_frame(PhysicsConfig::default());
    spawn_body(
        &mut f,
        Body::new_static(v2(FP::ZERO, -fp!(500)), FP::ZERO),
        Collider::new(Shape::box_shape(fp!(1000), fp!(500))),
    );
    // A huge, heavy box (mass 10 000) and tiny light balls (mass 0.001).
    let big = Shape::box_shape(fp!(50), fp!(50));
    let heavy = spawn_body(
        &mut f,
        Body::new_dynamic(v2(FP::ZERO, fp!(60)), &big, FP::ONE).with_velocity(v2(FP::ZERO, -fp!(500))),
        Collider::new(big),
    );
    let tiny = Shape::circle(fp!(0.05));
    let mut balls = Vec::new();
    for i in 0..30 {
        let x = FP::from_int(i - 15) * fp!(6);
        balls.push(spawn_body(
            &mut f,
            Body::new_dynamic(v2(x, fp!(200)), &tiny, fp!(0.127)).with_velocity(v2(fp!(-300), -fp!(500))),
            Collider::new(tiny),
        ));
    }
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 240);
    for e in balls.iter().chain(&[heavy]) {
        let b = body(&f, *e);
        assert!(b.pos.x.abs() < fp!(30000) && b.pos.y.abs() < fp!(30000), "escaped: {:?}", b.pos);
        assert!(b.vel.x.abs() <= fp!(500) && b.vel.y.abs() <= fp!(500));
    }
}

/// Checksum of [`sleepy_scene`] after settling, one impulse and settling
/// again: pins the sleep, island and wake code on every platform.
const SLEEPY_GOLDEN_CHECKSUM: u64 = 0xd7045bf847e9c5e9;

#[test]
fn golden_sleepy_scene_checksum() {
    let mut f = sleepy_scene();
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 300);
    let asleep = f.query::<(&Body,)>().filter(|(_, (b,))| b.sleep >> 31 != 0).count();
    assert!(asleep >= 25, "only {asleep} bodies asleep before the impulse");
    let target = f.query::<(&Body,)>().filter(|(_, (b,))| b.kind == BODY_DYNAMIC).map(|(e, _)| e).nth(7).unwrap();
    let p = body(&f, target).pos;
    apply_impulse(&mut f, target, v2(fp!(25), fp!(15)), p);
    run(&mut f, &mut sc, 300);
    let asleep_after = f.query::<(&Body,)>().filter(|(_, (b,))| b.sleep >> 31 != 0).count();
    assert!(asleep_after >= 25, "only {asleep_after} bodies asleep at the end");
    println!("sleepy golden checksum: {:#018x}", f.checksum());
    assert_eq!(f.checksum(), SLEEPY_GOLDEN_CHECKSUM, "sleepy golden changed; only update for an intended behavior change");
}

// ---- (g) capsules ----

/// Checksum of [`capsule_scene`] after `CAPSULE_GOLDEN_TICKS`.
const CAPSULE_GOLDEN_CHECKSUM: u64 = 0x9a7a7a033ee3efe1;
/// Hash of the ray and shape cast results over the settled capsule scene.
const CAPSULE_QUERY_GOLDEN: u64 = 0x6b0ddaa803998756;
const CAPSULE_GOLDEN_TICKS: u32 = 300;

/// ~130 bodies: capsules lying and standing in stacks, capsules of random
/// angle dropped together with circles, boxes and polygons, a spinning
/// kinematic capsule, a capsule sensor zone, and a static capsule and slope.
fn capsule_scene() -> Frame {
    let mut f = new_frame(PhysicsConfig::default());
    ground(&mut f);
    for sx in [-1, 1] {
        spawn_body(
            &mut f,
            Body::new_static(v2(FP::from_int(25 * sx), fp!(40)), FP::ZERO),
            Collider::new(Shape::box_shape(FP::ONE, fp!(40))),
        );
    }
    let cap = |f: &mut Frame, x: FP, y: FP, hl: FP, r: FP, angle: FP| {
        let s = Shape::capsule(hl, r);
        spawn_body(f, Body::new_dynamic(v2(x, y), &s, FP::ONE).with_angle(angle), Collider::new(s).with_restitution(fp!(0.1)))
    };
    // A stack of capsules on their sides and a row of upright ones.
    for i in 0..5 {
        cap(&mut f, fp!(-18), fp!(0.4) + FP::from_int(i) * fp!(0.85), fp!(1), fp!(0.4), FP::HALF_PI);
    }
    for k in 0..6 {
        cap(&mut f, fp!(-12) + FP::from_int(k) * fp!(1.1), fp!(1.5), fp!(0.6), fp!(0.4), FP::ZERO);
    }
    // A static tilted capsule and a static ramp to roll over.
    spawn_body(&mut f, Body::new_static(v2(fp!(-4), fp!(1.5)), fp!(0.6)), Collider::new(Shape::capsule(fp!(2), fp!(0.4))));
    spawn_body(&mut f, Body::new_static(v2(fp!(6), fp!(0.5)), fp!(0.3)), Collider::new(Shape::box_shape(fp!(3), fp!(0.3))));
    // A spinning kinematic capsule and a capsule sensor zone.
    let mut rotor = Body::new_kinematic(v2(fp!(18), fp!(9)));
    rotor.omega = FP::ONE;
    spawn_body(&mut f, rotor, Collider::new(Shape::capsule(fp!(1.5), fp!(0.5))));
    spawn_body(
        &mut f,
        Body::new_static(v2(fp!(4), fp!(3)), FP::HALF_PI),
        Collider::new(Shape::capsule(fp!(3), fp!(1.5))).sensor(),
    );
    let mut rng = FrameRng::new(0xCA95);
    for i in 0..140 {
        let x = rng.range_fp(fp!(-14), fp!(14));
        let y = rng.range_fp(fp!(6), fp!(50));
        match i % 5 {
            0..=2 => {
                let hl = rng.range_fp(fp!(0.2), fp!(1.2));
                let r = rng.range_fp(fp!(0.25), fp!(0.5));
                let a = rng.range_fp(-FP::PI, FP::PI);
                cap(&mut f, x, y, hl, r, a);
            }
            3 => {
                ball(&mut f, x, y, rng.range_fp(fp!(0.3), fp!(0.6)), fp!(0.3));
            }
            _ => {
                let shape = if i % 10 == 4 {
                    Shape::box_shape(rng.range_fp(fp!(0.3), fp!(0.7)), rng.range_fp(fp!(0.3), fp!(0.7)))
                } else {
                    regular_polygon(3 + (i % 4) as u32, rng.range_fp(fp!(0.4), fp!(0.7)))
                };
                let angle = rng.range_fp(-FP::PI, FP::PI);
                spawn_body(&mut f, Body::new_dynamic(v2(x, y), &shape, FP::ONE).with_angle(angle), Collider::new(shape));
            }
        }
    }
    // Rolling circles never stop without damping, and one moving body keeps
    // its whole pile awake.
    for (_, (b,)) in f.query::<(&mut Body,)>() {
        if b.kind == BODY_DYNAMIC {
            b.linear_damping = fp!(0.05);
            b.angular_damping = fp!(0.3);
        }
    }
    f
}

#[test]
fn golden_capsule_scene_checksum() {
    let mut f = capsule_scene();
    assert!(f.alive_count() >= 150, "scene has {} bodies", f.alive_count());
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, CAPSULE_GOLDEN_TICKS);
    let mut max_speed = FP::ZERO;
    for (_, (b,)) in f.query::<(&Body,)>().filter(|(_, (b,))| b.kind == BODY_DYNAMIC) {
        assert!(b.pos.x.abs() < fp!(26) && b.pos.y > -FP::ONE && b.pos.y < fp!(80), "body out of bounds: {:?}", b.pos);
        max_speed = max_speed.max(b.vel.length());
    }
    assert!(max_speed < fp!(60), "max speed {max_speed}");
    println!("capsule golden checksum: {:#018x}", f.checksum());
    assert_eq!(f.checksum(), CAPSULE_GOLDEN_CHECKSUM, "capsule golden changed; only update for an intended behavior change");
}

#[test]
fn golden_capsule_rollback_replay() {
    let mut f = capsule_scene();
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 100);
    let snapshot = f.clone();
    run(&mut f, &mut sc, 150);
    let expected = f.checksum();
    assert_ne!(expected, snapshot.checksum());
    f.copy_from(&snapshot);
    run(&mut f, &mut Scratch::new(), 150);
    assert_eq!(f.checksum(), expected);
    // Fresh scratch every tick gives the same state (no hidden history).
    let mut g = capsule_scene();
    run_fresh(&mut g, 250);
    assert_eq!(g.checksum(), expected);
}

#[test]
fn golden_capsule_serialize_roundtrip() {
    let mut f = capsule_scene();
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 120);
    let bytes = f.to_bytes();
    let mut g = Frame::from_bytes(f.registry().clone(), &bytes).expect("decode");
    assert_eq!(g.checksum(), f.checksum());
    run(&mut f, &mut sc, 120);
    run(&mut g, &mut Scratch::new(), 120);
    assert_eq!(f.checksum(), g.checksum());
    // Snapshot again later, once the pile has landed.
    run(&mut f, &mut sc, 300);
    let mut h = Frame::from_bytes(f.registry().clone(), &f.to_bytes()).expect("decode");
    assert_eq!(h.checksum(), f.checksum());
    run(&mut f, &mut sc, 30);
    run(&mut h, &mut Scratch::new(), 30);
    assert_eq!(f.checksum(), h.checksum());
}

/// Folds the ray casts, shape casts and character moves over the settled
/// capsule scene into one hash.
fn capsule_query_hash() -> u64 {
    let mut f = capsule_scene();
    run(&mut f, &mut Scratch::new(), CAPSULE_GOLDEN_TICKS);
    let mut h = 0xcbf29ce484222325u64;
    let mut mix = |v: i64| {
        h ^= v as u64;
        h = h.wrapping_mul(0x100000001b3);
    };
    let flt = QueryFilter::default();
    let shapes =
        [Shape::circle(fp!(0.4)), Shape::capsule(fp!(0.6), fp!(0.3)), Shape::box_shape(fp!(0.4), fp!(0.5)), regular_polygon(5, fp!(0.5))];
    for i in 0..48 {
        let a = FP::TWO_PI * FP::from_int(i) / FP::from_int(48);
        let dir = FPVec2::from_angle(a);
        let origin = v2(FP::from_int(i % 9 - 4) * fp!(2), fp!(3) + FP::from_int(i % 4));
        if let Some(r) = raycast(&mut f, origin, dir, fp!(60), flt) {
            mix(r.entity.index as i64);
            mix(r.distance.raw());
            mix(r.normal.x.raw());
        } else {
            mix(-1);
        }
        for (k, s) in shapes.iter().enumerate() {
            let hit = shape_cast(&mut f, s, origin, a / 3 * FP::from_int(k as i32), dir, fp!(30), flt, Entity::NONE);
            match hit {
                Some(x) => {
                    mix(x.entity.index as i64);
                    mix(x.distance.raw());
                    mix(x.point.x.raw());
                    mix(x.point.y.raw());
                    mix(x.normal.x.raw());
                    mix(x.normal.y.raw());
                }
                None => mix(-1),
            }
        }
        let p = CapsuleCharacterParams::new(fp!(0.5), fp!(0.3));
        let m = move_and_slide_capsule(&mut f, Entity::NONE, origin, dir * fp!(4), &p);
        mix(m.pos.x.raw());
        mix(m.pos.y.raw());
        mix(m.grounded as i64);
        mix(m.hits as i64);
    }
    h
}

#[test]
fn golden_capsule_query_hash() {
    let a = capsule_query_hash();
    assert_eq!(a, capsule_query_hash(), "queries are a pure function of the frame");
    println!("capsule query hash: {a:#018x}");
    assert_eq!(a, CAPSULE_QUERY_GOLDEN, "capsule query golden changed; only update for an intended behavior change");
}
