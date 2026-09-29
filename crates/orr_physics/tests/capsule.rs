//! Capsule shapes, general shape casts and the capsule character
//! controller. No floats: reference values come from `FP` geometry.
use orr_ecs::{ComponentRegistryBuilder, Entity, Frame};
use orr_fp::{fp, FPVec2, FrameRng, FP};
use orr_physics::{
    init, is_asleep, move_and_slide_capsule, raycast, register, shape_cast, spawn_body, step, Body, CapsuleCharacterParams,
    Collider, PhysicsConfig, QueryFilter, Scratch, Shape, TriggerEvent, SHAPE_CAPSULE, TRIGGER_ENTER,
};

type P = FPVec2;

fn new_frame() -> Frame {
    let mut b = ComponentRegistryBuilder::new();
    register(&mut b);
    let mut f = Frame::new(b.build());
    init(&mut f, PhysicsConfig::default());
    f
}

fn v2(x: FP, y: FP) -> P {
    FPVec2::new(x, y)
}

fn ground(f: &mut Frame) -> Entity {
    spawn_body(f, Body::new_static(v2(FP::ZERO, -FP::HALF), FP::ZERO), Collider::new(Shape::box_shape(fp!(200), FP::HALF)))
}

fn run(f: &mut Frame, sc: &mut Scratch, ticks: u32) {
    let mut ev = Vec::new();
    for _ in 0..ticks {
        ev.clear();
        f.set_tick(f.tick() + 1);
        step(f, sc, &mut ev);
    }
}

fn body(f: &Frame, e: Entity) -> Body {
    *f.get::<Body>(e).unwrap()
}

fn capsule(f: &mut Frame, x: FP, y: FP, hl: FP, r: FP, angle: FP) -> Entity {
    let s = Shape::capsule(hl, r);
    spawn_body(f, Body::new_dynamic(v2(x, y), &s, FP::ONE).with_angle(angle), Collider::new(s))
}

fn regular_polygon(n: u32, r: FP) -> Shape {
    let pts: Vec<P> =
        (0..n).map(|k| FPVec2::from_angle(FP::TWO_PI * FP::from_int(k as i32) / FP::from_int(n as i32)) * r).collect();
    Shape::polygon(&pts).expect("regular polygon")
}

// ---- reference geometry (independent of the crate internals) ----

fn seg_point_dist(p: P, a: P, b: P) -> FP {
    let ab = b - a;
    let l2 = ab.dot(ab);
    let t = if l2 == FP::ZERO { FP::ZERO } else { ((p - a).dot(ab) / l2).clamp(FP::ZERO, FP::ONE) };
    (p - (a + ab * t)).length()
}

fn proper_cross(a: P, b: P, c: P, d: P) -> bool {
    let d1 = (b - a).perp_dot(c - a);
    let d2 = (b - a).perp_dot(d - a);
    let d3 = (d - c).perp_dot(a - c);
    let d4 = (d - c).perp_dot(b - c);
    ((d1 > FP::ZERO && d2 < FP::ZERO) || (d1 < FP::ZERO && d2 > FP::ZERO))
        && ((d3 > FP::ZERO && d4 < FP::ZERO) || (d3 < FP::ZERO && d4 > FP::ZERO))
}

fn edges(core: &[P]) -> Vec<(P, P)> {
    match core.len() {
        1 => vec![(core[0], core[0])],
        2 => vec![(core[0], core[1])],
        n => (0..n).map(|i| (core[i], core[(i + 1) % n])).collect(),
    }
}

fn inside(poly: &[P], p: P) -> bool {
    poly.len() >= 3 && (0..poly.len()).all(|i| (poly[(i + 1) % poly.len()] - poly[i]).perp_dot(p - poly[i]) >= FP::ZERO)
}

fn core_dist(a: &[P], b: &[P]) -> FP {
    let (ea, eb) = (edges(a), edges(b));
    for x in &ea {
        for y in &eb {
            if proper_cross(x.0, x.1, y.0, y.1) {
                return FP::ZERO;
            }
        }
    }
    if a.iter().any(|&p| inside(b, p)) || b.iter().any(|&p| inside(a, p)) {
        return FP::ZERO;
    }
    let mut best = FP::MAX;
    for &p in a {
        for e in &eb {
            best = best.min(seg_point_dist(p, e.0, e.1));
        }
    }
    for &p in b {
        for e in &ea {
            best = best.min(seg_point_dist(p, e.0, e.1));
        }
    }
    best
}

fn world_core(s: &Shape, pos: P, angle: FP) -> (Vec<P>, FP) {
    if s.kind == 0 {
        return (vec![pos], s.radius);
    }
    let n = s.count as usize;
    ((0..n).map(|i| s.verts[i].rotate(angle) + pos).collect(), s.radius)
}

/// Signed distance between two shapes (negative when they overlap by their
/// cores' distance being smaller than the radii; zero core distance is
/// reported as `-(ra + rb)`, so deep overlap is only known to be negative).
fn dist(a: &Shape, pa: P, aa: FP, b: &Shape, pb: P, ab: FP) -> FP {
    let (ca, ra) = world_core(a, pa, aa);
    let (cb, rb) = world_core(b, pb, ab);
    core_dist(&ca, &cb) - ra - rb
}

/// True if the two shapes overlap (or touch), asked of the engine through a
/// sensor pair.
fn engine_overlaps(a: &Shape, pa: P, aa: FP, b: &Shape, pb: P, ab: FP) -> bool {
    let mut f = new_frame();
    spawn_body(&mut f, Body::new_static(pa, aa), Collider::new(*a).sensor());
    spawn_body(&mut f, Body::new_static(pb, ab), Collider::new(*b));
    let mut ev: Vec<TriggerEvent> = Vec::new();
    step(&mut f, &mut Scratch::new(), &mut ev);
    ev.iter().any(|e| e.kind == TRIGGER_ENTER)
}

// ---- shape construction ----

#[test]
fn capsule_shape_data_mass_and_bounds() {
    let s = Shape::capsule(fp!(1), fp!(0.5));
    assert_eq!(s.kind, SHAPE_CAPSULE);
    assert_eq!(s.count, 2);
    assert!((s.verts[0] - v2(FP::ZERO, -FP::ONE)).length() < fp!(0.001));
    assert!((s.normals[0] - FPVec2::Y).length() < fp!(0.001));
    assert!((s.bounding_radius() - fp!(1.5)).abs() < fp!(0.001));
    // Zero length is a circle.
    assert_eq!(Shape::capsule(FP::ZERO, fp!(0.5)), Shape::circle(fp!(0.5)));
    // A general segment is re-centered on its midpoint.
    let g = Shape::capsule_segment(v2(fp!(2), fp!(1)), v2(fp!(4), fp!(1)), fp!(0.25));
    assert!((g.verts[0] - v2(-FP::ONE, FP::ZERO)).length() < fp!(0.001));
    assert!((g.normals[0] - FPVec2::X).length() < fp!(0.001));

    // Mass and inertia against a grid integration over the stadium.
    let (hl, r, density) = (fp!(1.2), fp!(0.5), fp!(2));
    let s = Shape::capsule(hl, r);
    let md = s.mass_data(density);
    // Cells of 1/128 units: the products are exact in `FP`.
    let step = FP::from_ratio(1, 128);
    let (mut count, mut sum_r2) = (0i64, 0i64);
    let n = 512;
    for i in -n..n {
        for j in -n..n {
            let p = v2(step * i + step / 2, step * j + step / 2);
            let clamped = v2(FP::ZERO, p.y.clamp(-hl, hl));
            if (p - clamped).length_sq() <= r * r {
                count += 1;
                sum_r2 += p.length_sq().raw();
            }
        }
    }
    let cell = density * step * step;
    let mass = FP::from_raw(cell.raw() * count);
    let inertia = FP::from_raw(sum_r2) * cell;
    assert!((md.mass - mass).abs() < fp!(0.02), "mass {} vs grid {}", md.mass, mass);
    assert!((md.inertia - inertia).abs() < inertia / 100, "inertia {} vs grid {}", md.inertia, inertia);
    let exact = density * (hl * 2 * r * 2 + FP::PI * r * r);
    assert!((md.mass - exact).abs() < fp!(0.001));
    // Body::new_dynamic picks it up.
    let b = Body::new_dynamic(FPVec2::ZERO, &s, density);
    assert!((b.inv_mass - FP::ONE / md.mass).abs() < fp!(0.0001));
}

// ---- narrow phase: overlap against every shape ----

#[test]
fn capsule_overlap_thresholds_against_each_shape() {
    let cap = Shape::capsule(fp!(1), fp!(0.5));
    let z = FP::ZERO;
    let o = v2(z, z);
    let touch = |other: &Shape, p: P, ang: FP, cap_ang: FP| engine_overlaps(&cap, o, cap_ang, other, p, ang);

    // Circle (r 0.4): distance from the axis is 0.9 to touch.
    let c = Shape::circle(fp!(0.4));
    assert!(touch(&c, v2(fp!(0.88), fp!(0.3)), z, z));
    assert!(!touch(&c, v2(fp!(0.92), fp!(0.3)), z, z));
    // Past the end: measured from the end point (0, 1).
    assert!(touch(&c, v2(z, fp!(1.88)), z, z));
    assert!(!touch(&c, v2(z, fp!(1.92)), z, z));
    // Diagonal from the cap.
    assert!(touch(&c, v2(fp!(0.63), fp!(1.63)), z, z));
    assert!(!touch(&c, v2(fp!(0.66), fp!(1.66)), z, z));
    // Rotated capsule: the same tests along x.
    assert!(touch(&c, v2(fp!(1.88), z), z, FP::HALF_PI));
    assert!(!touch(&c, v2(fp!(1.92), z), z, FP::HALF_PI));

    // Box (half 0.5) beside the capsule: face contact at 0.5 + 0.5.
    let b = Shape::box_shape(FP::HALF, FP::HALF);
    assert!(touch(&b, v2(fp!(0.98), fp!(0.2)), z, z));
    assert!(!touch(&b, v2(fp!(1.02), fp!(0.2)), z, z));
    // Box corner against the round cap: the corner sits at distance d from
    // the end point (0, 1) along the unit direction (0.6, 0.8).
    let box_with_corner_at = |d: FP| {
        let corner = v2(z, fp!(1)) + v2(fp!(0.6), fp!(0.8)) * d;
        v2(corner.x + FP::HALF, corner.y + FP::HALF)
    };
    assert!(touch(&b, box_with_corner_at(fp!(0.48)), z, z), "corner just inside the cap");
    // A naive face-only SAT would report contact here: 0.6 * 0.52 = 0.31
    // beyond the side face and 0.8 * 0.52 > 0.5 beyond the top... the true
    // distance is 0.52 > 0.5.
    assert!(!touch(&b, box_with_corner_at(fp!(0.53)), z, z), "corner just outside the cap");
    // Rotated box (diamond) tip on the side of the capsule.
    let diamond_tip = FP::HALF * fp!(1.41421356);
    assert!(touch(&b, v2(fp!(0.5) + diamond_tip - fp!(0.02), z), FP::PI / 4, z));
    assert!(!touch(&b, v2(fp!(0.5) + diamond_tip + fp!(0.02), z), FP::PI / 4, z));

    // Capsule against capsule: parallel side by side, crossing, end to end.
    assert!(touch(&cap, v2(fp!(0.98), fp!(0.5)), z, z));
    assert!(!touch(&cap, v2(fp!(1.02), fp!(0.5)), z, z));
    assert!(touch(&cap, v2(z, fp!(2.98)), z, z), "end to end");
    assert!(!touch(&cap, v2(z, fp!(3.02)), z, z));
    assert!(touch(&cap, v2(fp!(0.3), fp!(0.2)), FP::HALF_PI, z), "crossing axes");
    assert!(touch(&cap, v2(fp!(1.4), fp!(1)), FP::HALF_PI, z), "T: B's axis end at the side of A");
    assert!(!touch(&cap, v2(fp!(2.1), fp!(1)), FP::HALF_PI, z));

    // Regular hexagon.
    let hex = regular_polygon(6, fp!(0.6));
    let hex_ang = FP::PI / 6; // a flat side faces the capsule
    assert!(touch(&hex, v2(fp!(0.5) + fp!(0.6) * fp!(0.8660254) - fp!(0.02), fp!(0.1)), hex_ang, z));
    assert!(!touch(&hex, v2(fp!(0.5) + fp!(0.6) * fp!(0.8660254) + fp!(0.02), fp!(0.1)), hex_ang, z));
}

#[test]
fn capsule_overlap_matches_reference_distance_on_random_pairs() {
    let mut rng = FrameRng::new(4242);
    let mut checked = 0;
    for _ in 0..500 {
        let cap = Shape::capsule(rng.range_fp(fp!(0.2), fp!(1.2)), rng.range_fp(fp!(0.2), fp!(0.6)));
        let other = match rng.next_u32() % 4 {
            0 => Shape::circle(rng.range_fp(fp!(0.2), fp!(0.7))),
            1 => Shape::capsule(rng.range_fp(fp!(0.2), fp!(1.2)), rng.range_fp(fp!(0.2), fp!(0.6))),
            2 => Shape::box_shape(rng.range_fp(fp!(0.2), fp!(1)), rng.range_fp(fp!(0.2), fp!(1))),
            _ => regular_polygon(3 + rng.next_u32() % 5, rng.range_fp(fp!(0.3), fp!(1))),
        };
        let (pa, pb) = (v2(FP::ZERO, FP::ZERO), v2(rng.range_fp(-fp!(3), fp!(3)), rng.range_fp(-fp!(3), fp!(3))));
        let (aa, ab) = (rng.range_fp(-FP::PI, FP::PI), rng.range_fp(-FP::PI, FP::PI));
        let d = dist(&cap, pa, aa, &other, pb, ab);
        if d.abs() < fp!(0.01) {
            continue; // too close to call with rounding
        }
        let (a, b) = if rng.next_u32() % 2 == 0 { (&cap, &other) } else { (&other, &cap) };
        let (xa, xb, ya, yb) = if std::ptr::eq(a, &cap) { (pa, pb, aa, ab) } else { (pb, pa, ab, aa) };
        let got = engine_overlaps(a, xa, ya, b, xb, yb);
        assert_eq!(got, d < FP::ZERO, "distance {d}: cap {:?} other kind {} at {:?} angles {aa} {ab}", cap.radius, other.kind, pb);
        checked += 1;
    }
    assert!(checked > 400, "only {checked} pairs checked");
}

// ---- dynamics ----

#[test]
fn capsule_on_its_side_rests_stably_and_sleeps() {
    let mut f = new_frame();
    ground(&mut f);
    let c = capsule(&mut f, fp!(0.3), fp!(2), fp!(1), fp!(0.4), FP::HALF_PI);
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 300);
    let b = body(&f, c);
    assert!((b.pos.y - fp!(0.4)).abs() < fp!(0.03), "y = {}", b.pos.y);
    assert!((b.angle - FP::HALF_PI).abs() < fp!(0.02), "angle = {}", b.angle);
    assert!(b.vel.length() < fp!(0.05) && (b.pos.x - fp!(0.3)).abs() < fp!(0.1));
    assert!(is_asleep(&f, c), "a capsule at rest must fall asleep");
    // No creep while asleep.
    let before = body(&f, c);
    run(&mut f, &mut sc, 120);
    assert_eq!(body(&f, c), before);
}

#[test]
fn tilted_capsule_topples_onto_its_side() {
    let mut f = new_frame();
    ground(&mut f);
    let c = capsule(&mut f, FP::ZERO, fp!(1.5), fp!(1), fp!(0.4), fp!(0.5));
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 400);
    let b = body(&f, c);
    // Lying flat: axis horizontal, so the angle is near +-pi/2.
    assert!((b.angle.abs() - FP::HALF_PI).abs() < fp!(0.03), "angle = {}", b.angle);
    assert!((b.pos.y - fp!(0.4)).abs() < fp!(0.03), "y = {}", b.pos.y);
    assert!(is_asleep(&f, c));
}

#[test]
fn upright_capsule_drop_settles_on_its_cap() {
    let mut f = new_frame();
    ground(&mut f);
    let c = capsule(&mut f, FP::ZERO, fp!(3), fp!(0.7), fp!(0.4), FP::ZERO);
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 240);
    let b = body(&f, c);
    // Either balanced upright (y = hl + r) or fallen flat (y = r).
    let up = (b.pos.y - fp!(1.1)).abs() < fp!(0.03);
    let flat = (b.pos.y - fp!(0.4)).abs() < fp!(0.03);
    assert!(up || flat, "y = {} angle = {}", b.pos.y, b.angle);
    assert!(b.vel.length() < fp!(0.1));
}

#[test]
fn capsule_stack_and_pyramid_stay_up() {
    let mut f = new_frame();
    ground(&mut f);
    // A stack of four capsules lying on their sides.
    let hl = fp!(1);
    let r = fp!(0.3);
    let stack: Vec<Entity> = (0..4).map(|i| capsule(&mut f, FP::ZERO, r + FP::from_int(i) * r * 2 + fp!(0.01) * i, hl, r, FP::HALF_PI)).collect();
    // A pyramid of 3 + 2 + 1 (side by side pairs) further away.
    let mut pyr = Vec::new();
    for row in 0..3 {
        for k in 0..(3 - row) {
            let x = fp!(8) + FP::from_int(k) * (r * 2 + fp!(0.02)) + FP::from_int(row) * (r + fp!(0.01));
            pyr.push(capsule(&mut f, x, r + FP::from_int(row) * (r * 2 - fp!(0.05)) + fp!(0.02), hl, r, FP::HALF_PI));
        }
    }
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 600);
    for (i, &e) in stack.iter().enumerate() {
        let b = body(&f, e);
        let want = r + FP::from_int(i as i32) * r * 2;
        // The stack may creep a little along the capsule axis while it settles.
        assert!(b.pos.x.abs() < fp!(0.25), "capsule {i} x = {}", b.pos.x);
        assert!((b.pos.y - want).abs() < fp!(0.08), "capsule {i} y = {} want {want}", b.pos.y);
        assert!((b.angle - FP::HALF_PI).abs() < fp!(0.05), "capsule {i} angle = {}", b.angle);
    }
    // The pyramid may settle, but nothing falls out of it.
    for &e in &pyr {
        let b = body(&f, e);
        assert!(b.pos.y > fp!(0.2) && b.pos.y < fp!(1.4), "pyramid capsule y = {}", b.pos.y);
        assert!(b.vel.length() < fp!(0.2));
    }
    assert!(stack.iter().all(|&e| is_asleep(&f, e)), "the stack sleeps");
}

#[test]
fn capsule_holds_on_a_slope_with_friction_and_rolls_without() {
    let build = |friction: FP| {
        let mut f = new_frame();
        // 20 degree ramp, top surface through the origin.
        let angle = fp!(0.35);
        let n = v2(-angle.sin_cos().0, angle.sin_cos().1);
        let center = v2(FP::ZERO, FP::ZERO) - n * FP::HALF;
        spawn_body(
            &mut f,
            Body::new_static(center, angle),
            Collider::new(Shape::box_shape(fp!(30), FP::HALF)).with_friction(friction),
        );
        // A capsule lying along the slope on its side.
        let s = Shape::capsule(fp!(0.8), fp!(0.3));
        let pos = n * fp!(0.32) + v2(fp!(2), fp!(2) * angle.sin_cos().0 / angle.sin_cos().1);
        let c = spawn_body(
            &mut f,
            Body::new_dynamic(pos, &s, FP::ONE).with_angle(angle + FP::HALF_PI),
            Collider::new(s).with_friction(friction),
        );
        (f, c, pos)
    };
    let (mut f, c, start) = build(fp!(0.9));
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 300);
    let b = body(&f, c);
    assert!((b.pos - start).length() < fp!(0.3), "friction holds: moved {}", (b.pos - start).length());

    let (mut f, c, start) = build(FP::ZERO);
    run(&mut f, &mut sc, 120);
    let b = body(&f, c);
    assert!(b.pos.x < start.x - fp!(1.5), "frictionless capsule slides down: x {} -> {}", start.x, b.pos.x);
    // It stays on the ramp surface (no tunnelling): distance to the ramp line.
    let angle = fp!(0.35);
    let n = v2(-angle.sin_cos().0, angle.sin_cos().1);
    assert!(n.dot(b.pos) > fp!(0.2) && n.dot(b.pos) < fp!(0.6), "height above the ramp {}", n.dot(b.pos));
}

#[test]
fn capsule_lands_on_circle_and_box_corner_without_sinking() {
    let mut f = new_frame();
    ground(&mut f);
    // A static circle and a static rotated box, capsules dropped on both.
    spawn_body(&mut f, Body::new_static(v2(-fp!(4), fp!(0.5)), FP::ZERO), Collider::new(Shape::circle(fp!(0.5))));
    spawn_body(&mut f, Body::new_static(v2(fp!(4), fp!(0.5)), FP::PI / 4), Collider::new(Shape::box_shape(fp!(0.5), fp!(0.5))));
    let a = capsule(&mut f, -fp!(4), fp!(4), fp!(0.6), fp!(0.3), FP::HALF_PI + fp!(0.1));
    let b = capsule(&mut f, fp!(4.2), fp!(4), fp!(0.6), fp!(0.3), fp!(0.1));
    let mut sc = Scratch::new();
    let mut lowest = FP::MAX;
    for _ in 0..300 {
        run(&mut f, &mut sc, 1);
        lowest = lowest.min(body(&f, a).pos.y).min(body(&f, b).pos.y);
    }
    // Both end up on the floor, lying or leaning, never inside the ground.
    assert!(lowest > fp!(0.25), "lowest center y = {lowest}");
    for e in [a, b] {
        let p = body(&f, e).pos;
        assert!(p.y < fp!(1.4) && p.x.abs() < fp!(9), "capsule at {p:?}");
    }
}

#[test]
fn deeply_overlapping_capsules_are_pushed_out() {
    let mut f = new_frame();
    ground(&mut f);
    // Half sunk into the ground (its axis is inside the box).
    let a = capsule(&mut f, FP::ZERO, fp!(0.1), fp!(1), fp!(0.4), FP::HALF_PI);
    // Crossing a static capsule like an X.
    spawn_body(
        &mut f,
        Body::new_static(v2(fp!(6), fp!(3)), FP::ZERO),
        Collider::new(Shape::capsule(fp!(1), fp!(0.3))),
    );
    let b = capsule(&mut f, fp!(6), fp!(3), fp!(1), fp!(0.3), FP::HALF_PI);
    let mut sc = Scratch::new();
    f.set_tick(1);
    let mut ev = Vec::new();
    step(&mut f, &mut sc, &mut ev);
    run(&mut f, &mut sc, 240);
    assert!(body(&f, a).pos.y > fp!(0.35), "sunk capsule resurfaced: y = {}", body(&f, a).pos.y);
    let pb = body(&f, b);
    assert!(pb.pos.y < fp!(1) || pb.pos.y > fp!(4), "crossed capsule separated: y = {}", pb.pos.y);
}

#[test]
fn capsule_sensor_reports_enter_and_exit() {
    let mut f = new_frame();
    let zone = spawn_body(
        &mut f,
        Body::new_static(v2(FP::ZERO, fp!(3)), FP::HALF_PI),
        Collider::new(Shape::capsule(fp!(2), fp!(0.5))).sensor(),
    );
    let ball_shape = Shape::circle(fp!(0.3));
    let ball = spawn_body(&mut f, Body::new_dynamic(v2(fp!(1.5), fp!(9)), &ball_shape, FP::ONE), Collider::new(ball_shape));
    let mut sc = Scratch::new();
    let mut kinds = Vec::new();
    let mut ev: Vec<TriggerEvent> = Vec::new();
    for _ in 0..120 {
        ev.clear();
        f.set_tick(f.tick() + 1);
        step(&mut f, &mut sc, &mut ev);
        kinds.extend(ev.iter().map(|e| (e.a.index.min(e.b.index), e.a.index.max(e.b.index), e.kind)));
    }
    let (lo, hi) = (zone.index.min(ball.index), zone.index.max(ball.index));
    assert_eq!(kinds, vec![(lo, hi, 1), (lo, hi, 2)], "one enter then one exit");
}

// ---- ray casts ----

#[test]
fn raycast_hits_capsules() {
    let mut f = new_frame();
    let up = spawn_body(&mut f, Body::new_static(v2(fp!(5), fp!(2)), FP::ZERO), Collider::new(Shape::capsule(fp!(1), fp!(0.5))));
    let flat = spawn_body(&mut f, Body::new_static(v2(fp!(-5), fp!(2)), FP::HALF_PI), Collider::new(Shape::capsule(fp!(1), fp!(0.5))));
    let flt = QueryFilter::default();
    let near = |a: FP, b: FP| (a - b).abs() < fp!(0.01);

    // Side of the upright capsule.
    let h = raycast(&mut f, v2(FP::ZERO, fp!(2)), FPVec2::X, fp!(100), flt).unwrap();
    assert_eq!(h.entity, up);
    assert!(near(h.distance, fp!(4.5)) && near(h.normal.x, -FP::ONE), "{h:?}");
    // The round cap: ray along y from above hits at the top of the cap.
    let h = raycast(&mut f, v2(fp!(5), fp!(10)), -FPVec2::Y, fp!(100), flt).unwrap();
    assert!(near(h.distance, fp!(10) - fp!(3.5)) && near(h.normal.y, FP::ONE), "{h:?}");
    // Off-center on the cap: the normal tilts (hit x offset 0.3 from the axis).
    let h = raycast(&mut f, v2(fp!(5.3), fp!(10)), -FPVec2::Y, fp!(100), flt).unwrap();
    let top = fp!(3) + (fp!(0.25) - fp!(0.09)).sqrt();
    assert!(near(h.point.y, top), "{h:?} top {top}");
    assert!(near(h.normal.x, fp!(0.6)) && near(h.normal.y, fp!(0.8)), "{h:?}");
    // Rotated capsule: the flat one from above and end-on.
    let h = raycast(&mut f, v2(fp!(-5), fp!(10)), -FPVec2::Y, fp!(100), flt).unwrap();
    assert_eq!(h.entity, flat);
    assert!(near(h.distance, fp!(7.5)), "{h:?}");
    let h = raycast(&mut f, v2(-fp!(10), fp!(2)), FPVec2::X, fp!(100), flt).unwrap();
    assert!(near(h.distance, fp!(3.5)), "{h:?}");
    // Grazing miss and short range.
    assert!(raycast(&mut f, v2(fp!(5.6), fp!(10)), -FPVec2::Y, fp!(100), flt).is_none());
    assert!(raycast(&mut f, v2(fp!(5), fp!(10)), -FPVec2::Y, fp!(5), flt).is_none());
    // Origin inside: distance 0.
    let h = raycast(&mut f, v2(fp!(5), fp!(2.2)), FPVec2::X, fp!(10), flt).unwrap();
    assert_eq!((h.entity, h.distance), (up, FP::ZERO));
}

// ---- shape casts ----

fn cast_world(targets: &[(Shape, P, FP)]) -> (Frame, Vec<Entity>) {
    let mut f = new_frame();
    let es = targets.iter().map(|(s, p, a)| spawn_body(&mut f, Body::new_static(*p, *a), Collider::new(*s))).collect();
    (f, es)
}

#[test]
fn shape_cast_matches_reference_for_every_shape_pair() {
    let mut rng = FrameRng::new(777);
    let mut hits = 0;
    let mut misses = 0;
    let mut starts_inside = 0;
    for case in 0..600 {
        let mk = |rng: &mut FrameRng| match rng.next_u32() % 4 {
            0 => Shape::circle(rng.range_fp(fp!(0.3), fp!(0.7))),
            1 => Shape::capsule(rng.range_fp(fp!(0.2), fp!(0.9)), rng.range_fp(fp!(0.2), fp!(0.5))),
            2 => Shape::box_shape(rng.range_fp(fp!(0.3), fp!(0.9)), rng.range_fp(fp!(0.3), fp!(0.9))),
            _ => regular_polygon(3 + rng.next_u32() % 5, rng.range_fp(fp!(0.4), fp!(0.9))),
        };
        let caster = mk(&mut rng);
        let target = mk(&mut rng);
        let cang = rng.range_fp(-FP::PI, FP::PI);
        let tang = rng.range_fp(-FP::PI, FP::PI);
        let tpos = v2(rng.range_fp(-fp!(5), fp!(5)), rng.range_fp(-fp!(5), fp!(5)));
        let dir = FPVec2::from_angle(rng.range_fp(-FP::PI, FP::PI));
        let max = fp!(8);
        let (mut f, es) = cast_world(&[(target, tpos, tang)]);
        let hit = shape_cast(&mut f, &caster, FPVec2::ZERO, cang, dir * fp!(3), max, QueryFilter::default(), Entity::NONE);
        let d_at = |t: FP| dist(&caster, dir * t, cang, &target, tpos, tang);
        let tol = fp!(0.006);
        match hit {
            Some(h) => {
                hits += 1;
                assert_eq!(h.entity, es[0]);
                assert!(h.distance >= FP::ZERO && h.distance <= max);
                assert!((h.fraction * max - h.distance).abs() < fp!(0.001));
                if h.distance == FP::ZERO {
                    starts_inside += 1;
                    assert!(d_at(FP::ZERO) <= tol, "case {case}: start distance {}", d_at(FP::ZERO));
                } else {
                    assert!(d_at(h.distance).abs() <= tol, "case {case}: gap at impact {}", d_at(h.distance));
                    let mut t = FP::ZERO;
                    while t < h.distance - fp!(0.02) {
                        assert!(d_at(t) > -tol, "case {case}: overlap before impact at t = {t}");
                        t += fp!(0.01);
                    }
                    assert!((h.normal.length() - FP::ONE).abs() < fp!(0.01), "unit normal");
                }
            }
            None => {
                misses += 1;
                let mut t = FP::ZERO;
                while t <= max {
                    assert!(d_at(t) > -tol, "case {case}: missed an overlap at t = {t}: {}", d_at(t));
                    t += fp!(0.01);
                }
            }
        }
    }
    assert!(hits > 60 && misses > 60 && starts_inside > 0, "hits {hits} misses {misses} inside {starts_inside}");
}

#[test]
fn shape_cast_exact_cases() {
    let z = FP::ZERO;
    let near = |a: FP, b: FP| (a - b).abs() < fp!(0.005);
    let flt = QueryFilter::default();
    let wall_box = Shape::box_shape(fp!(0.5), fp!(2));
    let (mut f, es) = cast_world(&[(wall_box, v2(fp!(5), z), z)]);

    // Circle straight at a box face.
    let c = Shape::circle(fp!(0.5));
    let h = shape_cast(&mut f, &c, v2(z, z), z, FPVec2::X, fp!(20), flt, Entity::NONE).unwrap();
    assert_eq!(h.entity, es[0]);
    assert!(near(h.distance, fp!(4)) && near(h.fraction, fp!(0.2)), "{h:?}");
    assert!(near(h.normal.x, -FP::ONE) && near(h.normal.y, z));
    assert!(near(h.point.x, fp!(4.5)), "contact point on the face: {:?}", h.point);
    // Upright capsule and a rotated box also stop at the face.
    let cap = Shape::capsule(fp!(1), fp!(0.4));
    let h = shape_cast(&mut f, &cap, v2(z, z), z, FPVec2::X, fp!(20), flt, Entity::NONE).unwrap();
    assert!(near(h.distance, fp!(4.1)), "{h:?}");
    // Capsule lying on its side: the end cap arrives first.
    let h = shape_cast(&mut f, &cap, v2(z, z), FP::HALF_PI, FPVec2::X, fp!(20), flt, Entity::NONE).unwrap();
    assert!(near(h.distance, fp!(4.5) - fp!(1.4)), "{h:?}");
    // 45 degree box: its tip leads.
    let b = Shape::box_shape(fp!(0.5), fp!(0.5));
    let h = shape_cast(&mut f, &b, v2(z, z), FP::PI / 4, FPVec2::X, fp!(20), flt, Entity::NONE).unwrap();
    assert!(near(h.distance, fp!(4.5) - fp!(0.70711)), "{h:?}");

    // Grazing: the target's top face is at y = 2. A circle of radius 0.5
    // passing at y = 2.5 touches it, at y = 2.51 it misses.
    let (mut f, _) = cast_world(&[(Shape::box_shape(fp!(3), fp!(1)), v2(fp!(6), fp!(1)), z)]);
    assert!(shape_cast(&mut f, &c, v2(z, fp!(2.49)), z, FPVec2::X, fp!(20), flt, Entity::NONE).is_some());
    assert!(shape_cast(&mut f, &c, v2(z, fp!(2.51)), z, FPVec2::X, fp!(20), flt, Entity::NONE).is_none());
    // Grazing a corner with a rotated polygon caster.
    let tri = regular_polygon(3, fp!(0.6));
    let h = shape_cast(&mut f, &tri, v2(z, fp!(2.3)), z, FPVec2::X, fp!(20), flt, Entity::NONE);
    assert!(h.is_some());

    // Starting in contact: overlapping reports fraction 0 with a normal that
    // pushes the caster out.
    let (mut f, es) = cast_world(&[(Shape::circle(fp!(1)), v2(fp!(2), z), z)]);
    let h = shape_cast(&mut f, &c, v2(fp!(1), z), z, FPVec2::Y, fp!(5), flt, Entity::NONE).unwrap();
    assert_eq!((h.entity, h.distance, h.fraction), (es[0], z, z));
    assert!(near(h.normal.x, -FP::ONE), "normal separates: {:?}", h.normal);
    // Exactly touching and moving away: no hit. Moving in: hit at 0.
    assert!(shape_cast(&mut f, &c, v2(fp!(0.5), z), z, -FPVec2::X, fp!(5), flt, Entity::NONE).is_none());
    let h = shape_cast(&mut f, &c, v2(fp!(0.5), z), z, FPVec2::X, fp!(5), flt, Entity::NONE).unwrap();
    assert_eq!(h.distance, z);
    // Zero direction and negative range hit nothing.
    assert!(shape_cast(&mut f, &c, v2(fp!(1), z), z, FPVec2::ZERO, fp!(5), flt, Entity::NONE).is_none());
    assert!(shape_cast(&mut f, &c, v2(fp!(-5), z), z, FPVec2::X, -FP::ONE, flt, Entity::NONE).is_none());
    // Zero distance: only an existing overlap counts.
    assert!(shape_cast(&mut f, &c, v2(fp!(1), z), z, FPVec2::X, z, flt, Entity::NONE).is_some());
    assert!(shape_cast(&mut f, &c, v2(-fp!(3), z), z, FPVec2::X, z, flt, Entity::NONE).is_none());
}

#[test]
fn shape_cast_rotated_polygon_targets() {
    let z = FP::ZERO;
    // A box rotated by 30 degrees: a circle coming from the left hits the
    // rotated face with the rotated normal.
    let ang = FP::PI / 6;
    let (mut f, _) = cast_world(&[(Shape::box_shape(fp!(1), fp!(1)), v2(fp!(5), z), ang)]);
    let c = Shape::circle(fp!(0.3));
    let h = shape_cast(&mut f, &c, v2(z, z), z, FPVec2::X, fp!(20), QueryFilter::default(), Entity::NONE).unwrap();
    // Reference through the distance function.
    let d = |t: FP| dist(&c, v2(t, z), z, &Shape::box_shape(fp!(1), fp!(1)), v2(fp!(5), z), ang);
    assert!(d(h.distance).abs() < fp!(0.006), "gap {} at {}", d(h.distance), h.distance);
    assert!(d(h.distance - fp!(0.05)) > fp!(0.02));
    // The normal points away from the box: against +x travel.
    assert!(h.normal.x < -fp!(0.3), "{:?}", h.normal);
    // A capsule falling onto a rotated polygon from above.
    let hex = regular_polygon(6, fp!(1));
    let (mut f, _) = cast_world(&[(hex, v2(z, z), fp!(0.2))]);
    let cap = Shape::capsule(fp!(0.5), fp!(0.3));
    let h = shape_cast(&mut f, &cap, v2(fp!(0.2), fp!(6)), fp!(0.7), -FPVec2::Y, fp!(20), QueryFilter::default(), Entity::NONE).unwrap();
    let d = |t: FP| dist(&cap, v2(fp!(0.2), fp!(6) - t), fp!(0.7), &hex, v2(z, z), fp!(0.2));
    assert!(d(h.distance).abs() < fp!(0.006) && d(h.distance - fp!(0.05)) > fp!(0.02));
}

#[test]
fn shape_cast_ties_filters_and_ignore() {
    let z = FP::ZERO;
    let flt = QueryFilter::default();
    let c = Shape::circle(fp!(0.5));
    let mk = |layer: u32, sensor: bool| {
        let mut col = Collider::new(Shape::box_shape(FP::HALF, FP::ONE)).with_filter(layer, u32::MAX);
        if sensor {
            col = col.sensor();
        }
        col
    };
    let mut f = new_frame();
    // Three identical targets at the same place: the lowest index wins.
    let a = spawn_body(&mut f, Body::new_static(v2(fp!(4), z), z), mk(1, false));
    let b = spawn_body(&mut f, Body::new_static(v2(fp!(4), z), z), mk(1, false));
    let s = spawn_body(&mut f, Body::new_static(v2(fp!(2), z), z), mk(1, true));
    let far = spawn_body(&mut f, Body::new_static(v2(fp!(9), z), z), mk(2, false));
    let h = shape_cast(&mut f, &c, v2(z, z), z, FPVec2::X, fp!(20), flt, Entity::NONE).unwrap();
    assert_eq!(h.entity, a, "tie goes to the lower entity index");
    // Ignoring the winner reveals its twin.
    let h = shape_cast(&mut f, &c, v2(z, z), z, FPVec2::X, fp!(20), flt, a).unwrap();
    assert_eq!(h.entity, b);
    // Sensors are skipped unless asked for.
    let with_sensors = QueryFilter { mask: u32::MAX, include_sensors: true };
    let h = shape_cast(&mut f, &c, v2(z, z), z, FPVec2::X, fp!(20), with_sensors, Entity::NONE).unwrap();
    assert_eq!(h.entity, s);
    // Layer masks: only layer 2 is visible.
    let only2 = QueryFilter { mask: 2, include_sensors: false };
    let h = shape_cast(&mut f, &c, v2(z, z), z, FPVec2::X, fp!(20), only2, Entity::NONE).unwrap();
    assert_eq!(h.entity, far);
    let none = QueryFilter { mask: 4, include_sensors: false };
    assert!(shape_cast(&mut f, &c, v2(z, z), z, FPVec2::X, fp!(20), none, Entity::NONE).is_none());
    // Range limit.
    assert!(shape_cast(&mut f, &c, v2(z, z), z, FPVec2::X, fp!(2), flt, Entity::NONE).is_none());
    // Same answer twice (pure function of the frame).
    let h1 = shape_cast(&mut f, &c, v2(z, z), z, FPVec2::X, fp!(20), flt, Entity::NONE);
    let h2 = shape_cast(&mut f, &c, v2(z, z), z, FPVec2::X, fp!(20), flt, Entity::NONE);
    assert_eq!(h1, h2);
}

// ---- character controller ----

fn params() -> CapsuleCharacterParams {
    CapsuleCharacterParams::new(fp!(0.5), fp!(0.3))
}

#[test]
fn capsule_controller_lands_stands_and_hits_walls() {
    let mut f = new_frame();
    ground(&mut f);
    spawn_body(&mut f, Body::new_static(v2(fp!(5), fp!(5)), FP::ZERO), Collider::new(Shape::box_shape(FP::HALF, fp!(5))));
    let p = params();
    let me = Entity::NONE;
    let z = FP::ZERO;

    let m = move_and_slide_capsule(&mut f, me, v2(z, fp!(3)), v2(z, -fp!(4)), &p);
    assert!(m.grounded && (m.ground_normal.y - FP::ONE).abs() < fp!(0.01));
    assert!((m.pos.y - fp!(0.81)).abs() < fp!(0.02), "landed at y = {}", m.pos.y);

    // Standing still with a little gravity: stays put, still grounded.
    let m2 = move_and_slide_capsule(&mut f, me, m.pos, v2(z, -fp!(0.05)), &p);
    assert!(m2.grounded && (m2.pos - m.pos).length() < fp!(0.02), "{:?} -> {:?}", m.pos, m2.pos);
    // Even with no movement at all the ground probe reports grounded.
    let m3 = move_and_slide_capsule(&mut f, me, m.pos, FPVec2::ZERO, &p);
    assert!(m3.grounded && m3.pos == m.pos);

    // Into the wall: stops one skin from its face at x = 4.5.
    let m = move_and_slide_capsule(&mut f, me, v2(z, fp!(0.81)), v2(fp!(10), z), &p);
    assert!(m.hits >= 1 && m.grounded);
    assert!((m.pos.x - fp!(4.19)).abs() < fp!(0.03), "stopped at x = {}", m.pos.x);
    assert!((m.pos.y - fp!(0.81)).abs() < fp!(0.03), "y = {}", m.pos.y);
    // A fast diagonal into the wall slides down it and lands.
    let m = move_and_slide_capsule(&mut f, me, v2(fp!(3.5), fp!(3)), v2(fp!(2), -fp!(4)), &p);
    assert!((m.pos.x - fp!(4.19)).abs() < fp!(0.03), "x = {}", m.pos.x);
    assert!((m.pos.y - fp!(0.81)).abs() < fp!(0.03) && m.grounded, "slid to y = {}", m.pos.y);
    // Jumping up against the ceiling of nothing: free move.
    let m = move_and_slide_capsule(&mut f, me, v2(z, fp!(2)), v2(z, fp!(2)), &p);
    assert!(!m.grounded && (m.pos.y - fp!(4)).abs() < fp!(0.001));
}

#[test]
fn capsule_controller_cannot_tunnel_or_pass_ignored_only() {
    let mut f = new_frame();
    let z = FP::ZERO;
    // A very thin wall and a capsule collider that belongs to the character.
    spawn_body(&mut f, Body::new_static(v2(fp!(3), z), z), Collider::new(Shape::box_shape(fp!(0.025), fp!(3))));
    let own = spawn_body(&mut f, Body::new_static(v2(fp!(-5), z), z), Collider::new(Shape::capsule(fp!(0.5), fp!(0.3))));
    let p = params();
    let m = move_and_slide_capsule(&mut f, Entity::NONE, v2(z, z), v2(fp!(40), z), &p);
    assert!(m.pos.x < fp!(3), "tunneled to x = {}", m.pos.x);
    // The own collider blocks unless it is the ignored entity.
    let blocked = move_and_slide_capsule(&mut f, Entity::NONE, v2(-fp!(8), z), v2(fp!(2), z), &p);
    assert!(blocked.pos.x < -fp!(5.5), "blocked at x = {}", blocked.pos.x);
    let free = move_and_slide_capsule(&mut f, own, v2(-fp!(8), z), v2(fp!(2), z), &p);
    assert!((free.pos.x - -fp!(6)).abs() < fp!(0.001));
    // Layer filter: nothing blocks with an unmatched mask.
    let mut q = p;
    q.filter = QueryFilter { mask: 8, include_sensors: false };
    let m = move_and_slide_capsule(&mut f, Entity::NONE, v2(z, z), v2(fp!(10), z), &q);
    assert!(m.pos.x == fp!(10) && m.hits == 0);
}

fn ramp(f: &mut Frame, angle: FP) -> P {
    // Slab whose top surface passes through the origin, rising to +x.
    let (s, c) = angle.sin_cos();
    let n = v2(-s, c);
    spawn_body(f, Body::new_static(v2(FP::ZERO, FP::ZERO) - n * FP::HALF, angle), Collider::new(Shape::box_shape(fp!(30), FP::HALF)));
    n
}

#[test]
fn capsule_controller_slopes() {
    let z = FP::ZERO;
    let p = params();
    // 30 degrees: walkable.
    let mut f = new_frame();
    let n = ramp(&mut f, FP::PI / 6);
    let m = move_and_slide_capsule(&mut f, Entity::NONE, v2(z, fp!(3)), v2(z, -fp!(5)), &p);
    assert!(m.grounded && (m.ground_normal - n).length() < fp!(0.02), "{m:?}");
    let start = m.pos;
    // Standing on it under gravity: no sliding.
    let mut pos = start;
    for _ in 0..90 {
        let m = move_and_slide_capsule(&mut f, Entity::NONE, pos, v2(z, -fp!(0.1)), &p);
        assert!(m.grounded);
        pos = m.pos;
    }
    assert!((pos - start).length() < fp!(0.02), "crept from {start:?} to {pos:?}");
    // Walking up it.
    for _ in 0..30 {
        let m = move_and_slide_capsule(&mut f, Entity::NONE, pos, v2(fp!(0.1), -fp!(0.05)), &p);
        assert!(m.grounded, "still grounded while climbing");
        pos = m.pos;
    }
    assert!(pos.x - start.x > fp!(2), "climbed only {} in x", pos.x - start.x);
    let tan = (FP::PI / 6).sin_cos().0 / (FP::PI / 6).sin_cos().1;
    let want_y = pos.x * tan + (fp!(0.3) + fp!(0.01)) / (FP::PI / 6).sin_cos().1 + fp!(0.5);
    assert!((pos.y - want_y).abs() < fp!(0.06), "y {} vs slope line {}", pos.y, want_y);

    // 60 degrees: too steep, the character slides down and is not grounded.
    let mut f = new_frame();
    ramp(&mut f, FP::PI / 3);
    let m = move_and_slide_capsule(&mut f, Entity::NONE, v2(fp!(2), fp!(5)), v2(z, -fp!(8)), &p);
    assert!(!m.grounded, "steep slope is not ground");
    let mut pos = m.pos;
    for _ in 0..30 {
        let m = move_and_slide_capsule(&mut f, Entity::NONE, pos, v2(z, -fp!(0.2)), &p);
        assert!(!m.grounded);
        pos = m.pos;
    }
    assert!(pos.x < m.pos.x - fp!(1) || pos.y < m.pos.y - fp!(1.5), "slid from {:?} to {:?}", m.pos, pos);
    // It never enters the slab.
    let n = v2(-(FP::PI / 3).sin_cos().0, (FP::PI / 3).sin_cos().1);
    assert!(n.dot(pos) > fp!(0.25), "height over the slab {}", n.dot(pos));

    // Walking down a slope: snapping keeps the character on the ground.
    let mut f = new_frame();
    ramp(&mut f, FP::PI / 6);
    let start = move_and_slide_capsule(&mut f, Entity::NONE, v2(fp!(6), fp!(6)), v2(z, -fp!(8)), &p).pos;
    let mut walk = |q: &CapsuleCharacterParams| {
        let mut pos = start;
        let mut grounded = 0;
        for _ in 0..20 {
            let m = move_and_slide_capsule(&mut f, Entity::NONE, pos, v2(-fp!(0.1), z), q);
            grounded += m.grounded as u32;
            pos = m.pos;
        }
        (pos, grounded)
    };
    let (_, without) = walk(&p);
    let mut snap = p;
    snap.snap_distance = fp!(0.3);
    let (end, with) = walk(&snap);
    assert_eq!(with, 20, "snapping keeps the character grounded");
    assert!(without < 20, "without snapping it walks off the surface ({without})");
    assert!(end.x < start.x - fp!(1.5));
}

#[test]
fn capsule_controller_steps() {
    let z = FP::ZERO;
    let build = |step_h: FP| {
        let mut f = new_frame();
        ground(&mut f);
        // A ledge 0.3 high and a tall block 0.8 high.
        spawn_body(&mut f, Body::new_static(v2(fp!(4.5), fp!(0.15)), z), Collider::new(Shape::box_shape(fp!(1.5), fp!(0.15))));
        spawn_body(&mut f, Body::new_static(v2(fp!(-4.5), fp!(0.4)), z), Collider::new(Shape::box_shape(fp!(1.5), fp!(0.4))));
        let mut p = params();
        p.step_height = step_h;
        (f, p)
    };
    let walk = |f: &mut Frame, p: &CapsuleCharacterParams, dir: FP, from: FP, steps: u32| {
        let mut pos = v2(from, fp!(0.81));
        for _ in 0..steps {
            pos = move_and_slide_capsule(f, Entity::NONE, pos, v2(fp!(0.1) * dir, -fp!(0.05)), p).pos;
        }
        pos
    };
    // With a step height of 0.4 the character climbs the ledge and walks on it.
    let (mut f, p) = build(fp!(0.4));
    let pos = walk(&mut f, &p, FP::ONE, z, 45);
    assert!(pos.x > fp!(4), "stuck at x = {}", pos.x);
    assert!((pos.y - fp!(1.11)).abs() < fp!(0.03), "on the ledge at y = {}", pos.y);
    // The ledge is a wall without stepping and with a too low step height.
    for h in [z, fp!(0.2)] {
        let (mut f, p) = build(h);
        let pos = walk(&mut f, &p, FP::ONE, z, 70);
        assert!(pos.x < fp!(3.0), "step height {h}: x = {}", pos.x);
        assert!((pos.y - fp!(0.81)).abs() < fp!(0.03));
    }
    // A 0.8 block is too tall for a 0.4 step.
    let (mut f, p) = build(fp!(0.4));
    let pos = walk(&mut f, &p, -FP::ONE, z, 70);
    assert!(pos.x > -fp!(3.1) && (pos.y - fp!(0.81)).abs() < fp!(0.03), "blocked by the tall block: {pos:?}");
    // Stepping needs headroom: a low ceiling above the ledge stops the step.
    let (mut f, p) = build(fp!(0.4));
    spawn_body(&mut f, Body::new_static(v2(fp!(4.5), fp!(2.0)), z), Collider::new(Shape::box_shape(fp!(1.5), fp!(0.5))));
    let pos = walk(&mut f, &p, FP::ONE, z, 70);
    assert!(pos.x < fp!(3.0), "ceiling above the ledge blocks the step: x = {}", pos.x);
}

#[test]
fn capsule_controller_is_deterministic_and_read_only() {
    let mut f = new_frame();
    ground(&mut f);
    ramp_free(&mut f);
    let before = f.checksum();
    let p = params();
    let run_it = |f: &mut Frame| {
        let mut pos = v2(fp!(-3), fp!(2));
        let mut trace = Vec::new();
        for i in 0..80 {
            let dx = if i % 20 < 10 { fp!(0.12) } else { -fp!(0.05) };
            let m = move_and_slide_capsule(f, Entity::NONE, pos, v2(dx, -fp!(0.08)), &p);
            pos = m.pos;
            trace.push((pos, m.grounded, m.hits));
        }
        trace
    };
    let a = run_it(&mut f);
    let b = run_it(&mut f);
    assert_eq!(a, b);
    assert_eq!(f.checksum(), before, "queries never modify the frame");
}

fn ramp_free(f: &mut Frame) {
    spawn_body(f, Body::new_static(v2(fp!(2), fp!(1)), FP::PI / 8), Collider::new(Shape::box_shape(fp!(3), FP::HALF)));
    spawn_body(f, Body::new_static(v2(fp!(6), fp!(1)), FP::ZERO), Collider::new(Shape::capsule(fp!(1), fp!(0.5))));
    let s = Shape::capsule(fp!(0.5), fp!(0.3));
    spawn_body(f, Body::new_dynamic(v2(fp!(1), fp!(8)), &s, FP::ONE), Collider::new(s));
}

#[test]
fn capsule_range_limits_stay_inside_the_solver_ranges() {
    // Debug builds assert on solver products that overflow 64 bits. Push
    // capsule size, mass and speed to the documented limits together.
    let mut f = new_frame();
    spawn_body(
        &mut f,
        Body::new_static(v2(FP::ZERO, -fp!(500)), FP::ZERO),
        Collider::new(Shape::box_shape(fp!(2000), fp!(500))),
    );
    // Heavy capsule (mass about 10 000), 900 units long and 100 thick.
    let big = Shape::capsule(fp!(400), fp!(50));
    let heavy = spawn_body(
        &mut f,
        Body::new_dynamic(v2(FP::ZERO, fp!(300)), &big, fp!(0.11)).with_velocity(v2(fp!(30), -fp!(500))).with_angle(fp!(0.3)),
        Collider::new(big),
    );
    // The longest allowed capsule: half length + radius = 1000, light.
    let rod = Shape::capsule(fp!(999.95), fp!(0.05));
    let rod_e = spawn_body(
        &mut f,
        Body::new_dynamic(v2(-fp!(1500), fp!(200)), &rod, fp!(0.1)).with_velocity(v2(fp!(100), -fp!(400))).with_angle(fp!(1.0)),
        Collider::new(rod),
    );
    // Smallest capsules (mass 0.001) at the speed limit.
    let tiny = Shape::capsule(fp!(0.05), fp!(0.05));
    let mut small = Vec::new();
    for i in 0..30 {
        let x = FP::from_int(i - 15) * fp!(9) + fp!(700);
        small.push(spawn_body(
            &mut f,
            Body::new_dynamic(v2(x, fp!(250)), &tiny, fp!(0.06)).with_velocity(v2(-fp!(300), -fp!(500))).with_angle(FP::from_int(i) / 7),
            Collider::new(tiny),
        ));
    }
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 300);
    for e in small.iter().chain(&[heavy, rod_e]) {
        let b = body(&f, *e);
        assert!(b.pos.x.abs() < fp!(30000) && b.pos.y.abs() < fp!(30000), "escaped: {:?}", b.pos);
        assert!(b.vel.x.abs() <= fp!(500) && b.vel.y.abs() <= fp!(500));
    }
    // Queries at the limits do not overflow either.
    let h = shape_cast(&mut f, &rod, v2(-fp!(2000), fp!(300)), FP::ZERO, FPVec2::X, fp!(3000), QueryFilter::default(), Entity::NONE);
    assert!(h.is_some());
}

#[test]
fn capsule_rollback_and_serialization_mid_sleep() {
    // Small stacks of capsules that settle, fall asleep, and get hit.
    let build = || {
        let mut f = new_frame();
        ground(&mut f);
        for k in 0..6 {
            let x = FP::from_int(k * 5 - 12);
            for i in 0..3 {
                let hl = if (k + i) % 2 == 0 { fp!(1) } else { fp!(0.6) };
                capsule(&mut f, x, fp!(0.3) + FP::from_int(i) * fp!(0.62), hl, fp!(0.3), FP::HALF_PI);
            }
        }
        f
    };
    let asleep = |f: &mut Frame| f.query::<(&Body,)>().filter(|(_, (b,))| b.sleep >> 31 != 0).count();
    let mut f = build();
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 60);
    let snapshot = f.clone();
    run(&mut f, &mut sc, 240);
    let n = asleep(&mut f);
    assert!(n >= 14, "{n} of 18 asleep");
    let expected = f.checksum();
    f.copy_from(&snapshot);
    run(&mut f, &mut Scratch::new(), 240);
    assert_eq!(f.checksum(), expected, "rollback replay");

    // Serialize while asleep, then wake them with a hit and compare.
    let bytes = f.to_bytes();
    let mut g = Frame::from_bytes(f.registry().clone(), &bytes).expect("decode");
    assert_eq!(g.checksum(), f.checksum());
    let dropper = |f: &mut Frame| {
        let s = Shape::circle(fp!(0.5));
        spawn_body(f, Body::new_dynamic(v2(-fp!(12), fp!(5)), &s, FP::ONE), Collider::new(s));
    };
    dropper(&mut f);
    dropper(&mut g);
    run(&mut f, &mut sc, 200);
    run(&mut g, &mut Scratch::new(), 200);
    assert_eq!(f.checksum(), g.checksum());
    let n = asleep(&mut f);
    assert!(n >= 12, "{n} asleep after the hit");
}
