//! Behavior tests for the 3D physics: shapes, pairs, friction, restitution,
//! sleeping, queries.
mod common;

use common::scenes::*;
use common::*;
use orr_fp::{fp, FPMat3, FPQuat, FPVec3, FP};
use orr_physics3d::{
    apply_impulse, is_asleep, raycast, sphere_cast, spawn_body, Body, Collider, PhysicsConfig, QueryFilter, Scratch, Shape,
    BODY_DYNAMIC,
};

fn up_axis(b: &Body) -> FPVec3 {
    FPMat3::from_quat(b.rot).col(1)
}

fn settle(f: &mut orr_ecs::Frame, ticks: u32) -> Scratch {
    let mut sc = Scratch::new();
    run(f, &mut sc, ticks);
    sc
}

// ---- resting on the ground, one shape at a time ----

#[test]
fn sphere_box_capsule_come_to_rest_and_sleep() {
    let mut f = new_frame();
    ground(&mut f);
    let s = spawn_sphere(&mut f, v3!(-3, 2, 0), FP::HALF);
    let b = spawn_box(&mut f, v3!(0, 2, 0), v3!(0.5, 0.3, 0.7));
    let lie = FPQuat::from_axis_angle(FPVec3::Z, FP::HALF_PI);
    let c = spawn_capsule(&mut f, v3!(3, 2, 0), fp!(0.8), fp!(0.2), lie);
    let _ = settle(&mut f, 400);
    let (sb, bb, cb) = (body(&f, s), body(&f, b), body(&f, c));
    assert!((sb.pos.y - FP::HALF).abs() < fp!(0.02), "sphere y {}", sb.pos.y);
    assert!((bb.pos.y - fp!(0.3)).abs() < fp!(0.02), "box y {}", bb.pos.y);
    assert!((cb.pos.y - fp!(0.2)).abs() < fp!(0.02), "capsule y {}", cb.pos.y);
    // The capsule stays on its side.
    assert!(up_axis(&cb).y.abs() < fp!(0.05));
    for e in [s, b, c] {
        assert!(is_asleep(&f, e), "body should be asleep");
    }
}

#[test]
fn capsule_standing_upright_stays_upright() {
    let mut f = new_frame();
    ground(&mut f);
    let c = spawn_capsule(&mut f, v3!(0, 1.5, 0), fp!(0.6), fp!(0.25), FPQuat::IDENTITY);
    let _ = settle(&mut f, 300);
    let b = body(&f, c);
    assert!((b.pos.y - fp!(0.85)).abs() < fp!(0.02), "y {}", b.pos.y);
    assert!(up_axis(&b).y > fp!(0.99));
}

#[test]
fn tilted_capsule_topples_and_lies_down() {
    let mut f = new_frame();
    ground(&mut f);
    let tilt = FPQuat::from_axis_angle(FPVec3::Z, fp!(0.5));
    let c = spawn_capsule(&mut f, v3!(0, 1.4, 0), fp!(0.6), fp!(0.25), tilt);
    let _ = settle(&mut f, 600);
    let b = body(&f, c);
    assert!((b.pos.y - fp!(0.25)).abs() < fp!(0.03), "y {}", b.pos.y);
    assert!(up_axis(&b).y.abs() < fp!(0.1), "axis {:?}", up_axis(&b));
}

#[test]
fn box_dropped_on_its_edge_falls_flat() {
    let mut f = new_frame();
    ground(&mut f);
    let q = FPQuat::from_axis_angle(FPVec3::Z, fp!(0.6)).hamilton_mul(FPQuat::from_axis_angle(FPVec3::X, fp!(0.05)));
    let e = spawn_box(&mut f, v3!(0, 1.2, 0), v3!(0.5, 0.5, 0.5));
    f.get_mut::<Body>(e).unwrap().rot = q;
    let _ = settle(&mut f, 600);
    let b = body(&f, e);
    assert!((b.pos.y - FP::HALF).abs() < fp!(0.03), "y {}", b.pos.y);
    assert!(up_axis(&b).y > fp!(0.98), "up {:?}", up_axis(&b));
    assert!(is_asleep(&f, e));
}

#[test]
fn offset_box_on_box_stays_when_over_the_support() {
    let mut f = new_frame();
    ground(&mut f);
    let lower = spawn_box(&mut f, v3!(0, 0.5, 0), v3!(1, 0.5, 1));
    let upper = spawn_box(&mut f, v3!(0.6, 1.51, 0.3), v3!(0.5, 0.5, 0.5));
    let _ = settle(&mut f, 400);
    let (l, u) = (body(&f, lower), body(&f, upper));
    assert!((l.pos.y - FP::HALF).abs() < fp!(0.02));
    assert!((u.pos.y - fp!(1.5)).abs() < fp!(0.03), "upper y {}", u.pos.y);
    assert!((u.pos.x - fp!(0.6)).abs() < fp!(0.05) && (u.pos.z - fp!(0.3)).abs() < fp!(0.05));
}

#[test]
fn box_overhanging_past_its_center_of_mass_tips_off() {
    let mut f = new_frame();
    ground(&mut f);
    let _lower = spawn_box(&mut f, v3!(0, 0.5, 0), v3!(0.5, 0.5, 0.5));
    let upper = spawn_box(&mut f, v3!(0.8, 1.51, 0), v3!(0.5, 0.5, 0.5));
    let _ = settle(&mut f, 400);
    let u = body(&f, upper);
    // It slid or tipped off the lower box and ended up on the ground.
    assert!(u.pos.y < fp!(0.7), "upper y {}", u.pos.y);
}

#[test]
fn sphere_rolls_off_a_box_edge_and_reaches_the_ground() {
    let mut f = new_frame();
    ground(&mut f);
    spawn_body(&mut f, Body::new_static(v3!(0, 0.5, 0)), Collider::new(Shape::cuboid(fp!(1), fp!(0.5), fp!(1))));
    let s = spawn_sphere(&mut f, v3!(1.02, 1.6, 0), fp!(0.3));
    let _ = settle(&mut f, 600);
    let b = body(&f, s);
    assert!(b.pos.y < fp!(0.35), "sphere y {}", b.pos.y);
    assert!(b.pos.x > fp!(1.0));
}

#[test]
fn capsule_lying_across_a_box_stays_on_top() {
    let mut f = new_frame();
    ground(&mut f);
    spawn_body(&mut f, Body::new_static(v3!(0, 0.5, 0)), Collider::new(Shape::cuboid(fp!(0.6), fp!(0.5), fp!(0.6))));
    // Axis along z, longer than the box.
    let axis_z = FPQuat::from_axis_angle(FPVec3::X, FP::HALF_PI);
    let c = spawn_capsule(&mut f, v3!(0.1, 1.6, 0), fp!(1.2), fp!(0.2), axis_z);
    let _ = settle(&mut f, 500);
    let b = body(&f, c);
    assert!((b.pos.y - fp!(1.2)).abs() < fp!(0.03), "capsule y {}", b.pos.y);
    assert!((b.pos.x - fp!(0.1)).abs() < fp!(0.05));
}

#[test]
fn sphere_lands_on_a_static_capsule_and_slides_off() {
    let mut f = new_frame();
    ground(&mut f);
    let cap = Shape::capsule(fp!(1), fp!(0.3));
    let lie = FPQuat::from_axis_angle(FPVec3::Z, FP::HALF_PI);
    spawn_body(&mut f, Body::new_static(v3!(0, 1, 0)).with_rotation(lie), Collider::new(cap));
    let s = spawn_sphere(&mut f, v3!(0.1, 3, 0.05), fp!(0.3));
    let _ = settle(&mut f, 500);
    let b = body(&f, s);
    assert!(b.pos.y < fp!(0.35), "sphere y {}", b.pos.y);
}

// ---- material behavior ----

fn slope(mu: FP) -> (orr_ecs::Frame, orr_ecs::Entity, FP) {
    let mut f = new_frame();
    let ang = fp!(0.4);
    let q = FPQuat::from_axis_angle(FPVec3::Z, ang);
    let n = q.rotate_vec3(FPVec3::Y);
    spawn_body(
        &mut f,
        Body::new_static(n * -FP::HALF).with_rotation(q),
        Collider::new(Shape::cuboid(fp!(60), FP::HALF, fp!(60))).with_friction(mu),
    );
    let s = Shape::cuboid(fp!(0.5), fp!(0.5), fp!(0.5));
    let e = spawn_body(&mut f, Body::new_dynamic(n * fp!(0.51), &s, FP::ONE).with_rotation(q), Collider::new(s).with_friction(mu));
    (f, e, ang)
}

#[test]
fn friction_holds_a_box_on_a_slope_when_mu_exceeds_tan_angle() {
    // tan(0.4) = 0.42; mu 0.8 holds, mu 0.2 slides.
    let (mut f, e, _) = slope(fp!(0.8));
    let start = body(&f, e).pos;
    let _ = settle(&mut f, 240);
    let moved = (body(&f, e).pos - start).length();
    assert!(moved < fp!(0.05), "held box moved {moved}");

    let (mut f, e, _) = slope(fp!(0.2));
    let start = body(&f, e).pos;
    let _ = settle(&mut f, 120);
    let b = body(&f, e);
    // a = g (sin - mu cos) = 2.05 m/s^2 for 2 s => about 4 m downhill (-x).
    let moved = (b.pos - start).length();
    assert!(moved > fp!(2.5) && moved < fp!(5.5), "sliding box moved {moved}");
    assert!(b.pos.x < start.x);
}

#[test]
fn restitution_bounces_back_to_the_expected_height() {
    let mut f = new_frame();
    spawn_body(
        &mut f,
        Body::new_static(v3!(0, -0.5, 0)),
        Collider::new(Shape::cuboid(fp!(50), FP::HALF, fp!(50))),
    );
    let s = Shape::sphere(FP::HALF);
    let e = spawn_body(&mut f, Body::new_dynamic(v3!(0, 2.5, 0), &s, FP::ONE), Collider::new(s).with_restitution(fp!(0.8)));
    let mut sc = Scratch::new();
    let mut hit = false;
    let mut peak = FP::ZERO;
    for _ in 0..200 {
        orr_physics3d::step(&mut f, &mut sc);
        let b = body(&f, e);
        if b.vel.y > FP::ZERO {
            hit = true;
        }
        if hit {
            peak = peak.max(b.pos.y);
        }
    }
    // Drop height 2.0 -> after one bounce 2.0 * 0.64 above the rest height.
    let height = peak - FP::HALF;
    assert!(height > fp!(1.0) && height < fp!(1.5), "bounce height {height}");
}

#[test]
fn energy_does_not_grow_in_a_bouncing_pile() {
    let mut f = sphere_rain(50);
    for (_, (c,)) in f.query::<(&mut Collider,)>() {
        c.restitution = fp!(0.4);
    }
    let g = fp!(10);
    let energy = |f: &mut orr_ecs::Frame| {
        let mut e = FP::ZERO;
        for (_, (b, c)) in f.query::<(&Body, &Collider)>() {
            if b.kind == BODY_DYNAMIC && b.inv_mass > FP::ZERO {
                let m = FP::ONE / b.inv_mass;
                e += m * g * b.pos.y + m * b.vel.length_sq() / 2;
                let _ = c;
            }
        }
        e
    };
    let e0 = energy(&mut f);
    let mut sc = Scratch::new();
    let mut max_e = e0;
    for _ in 0..400 {
        orr_physics3d::step(&mut f, &mut sc);
        max_e = max_e.max(energy(&mut f));
    }
    // Baumgarte pushes bodies apart a little, but energy must not grow.
    assert!(max_e <= e0 + e0 / 50, "energy rose from {e0} to {max_e}");
}

// ---- dynamics of free bodies ----

#[test]
fn free_spin_keeps_its_rate_and_kinetic_energy() {
    let mut cfg = PhysicsConfig::default();
    cfg.gravity = FPVec3::ZERO;
    cfg.sleep_ticks = 0;
    let mut f = new_frame_with(cfg);
    let s = Shape::cuboid(fp!(0.5), fp!(0.5), fp!(0.5));
    let e = spawn_body(&mut f, Body::new_dynamic(FPVec3::ZERO, &s, FP::ONE).with_omega(v3!(0, 3, 0)), Collider::new(s));
    let _ = settle(&mut f, 60);
    let b = body(&f, e);
    let (axis, angle) = b.rot.to_axis_angle();
    // 3 rad/s for one second.
    assert!((angle - fp!(3)).abs() < fp!(0.05), "angle {angle}");
    assert!(axis.y > fp!(0.99), "axis {axis:?}");
    assert!((b.omega.y - fp!(3)).abs() < fp!(0.001));
    assert!(b.pos.length() < fp!(0.001));
}

#[test]
fn impulse_off_center_produces_spin_and_motion() {
    let mut cfg = PhysicsConfig::default();
    cfg.gravity = FPVec3::ZERO;
    let mut f = new_frame_with(cfg);
    let s = Shape::cuboid(fp!(0.5), fp!(0.5), fp!(0.5));
    let e = spawn_body(&mut f, Body::new_dynamic(FPVec3::ZERO, &s, FP::ONE), Collider::new(s));
    apply_impulse(&mut f, e, v3!(0, 0, 1), v3!(0.5, 0, 0));
    let b = body(&f, e);
    // m = 1, I = 1/6: v = 1, omega = r x J / I = (0.5, 0, 0) x (0, 0, 1) / (1/6) = (0, -3, 0).
    assert!((b.vel.z - FP::ONE).abs() < fp!(0.001), "v {:?}", b.vel);
    assert!((b.omega.y + fp!(3)).abs() < fp!(0.01), "w {:?}", b.omega);
}

#[test]
fn mass_properties_match_numeric_integration() {
    // Integrate the shape on a lattice of 0.05 cells and compare.
    fn lattice(inside: impl Fn(FPVec3) -> bool, extent: FP) -> (FP, FPVec3) {
        // Cell centers of a 1/20 lattice, in units of 1/40: odd integers.
        let n = (extent * 20).to_int();
        let (mut cnt, mut sx, mut sy, mut sz) = (0i64, 0i64, 0i64, 0i64);
        for i in -n..n {
            for j in -n..n {
                for k in -n..n {
                    let (u, v, w) = ((2 * i + 1) as i64, (2 * j + 1) as i64, (2 * k + 1) as i64);
                    let p = FPVec3::new(FP::from_ratio(u, 40), FP::from_ratio(v, 40), FP::from_ratio(w, 40));
                    if inside(p) {
                        cnt += 1;
                        sx += v * v + w * w;
                        sy += u * u + w * w;
                        sz += u * u + v * v;
                    }
                }
            }
        }
        // volume = cnt / 20^3, inertia = sum / (40^2 * 20^3).
        (FP::from_ratio(cnt, 8000), FPVec3::new(FP::from_ratio(sx, 12_800_000), FP::from_ratio(sy, 12_800_000), FP::from_ratio(sz, 12_800_000)))
    }
    let close = |a: FP, b: FP| (a - b).abs() <= b * fp!(0.04) + fp!(0.002);
    let shapes = [
        Shape::sphere(fp!(0.6)),
        Shape::cuboid(fp!(0.4), fp!(0.7), fp!(0.5)),
        Shape::capsule(fp!(0.5), fp!(0.3)),
    ];
    for s in shapes {
        let inside = |p: FPVec3| match s.kind {
            orr_physics3d::SHAPE_SPHERE => p.length_sq() <= s.radius * s.radius,
            orr_physics3d::SHAPE_BOX => p.x.abs() <= s.half.x && p.y.abs() <= s.half.y && p.z.abs() <= s.half.z,
            _ => {
                let y = p.y.clamp(-s.half.y, s.half.y);
                (p - FPVec3::new(FP::ZERO, y, FP::ZERO)).length_sq() <= s.radius * s.radius
            }
        };
        let (m, i) = lattice(inside, fp!(1.3));
        let md = s.mass_data(FP::ONE);
        assert!(close(md.mass, m), "kind {} mass {} vs {}", s.kind, md.mass, m);
        assert!(close(md.inertia.x, i.x), "kind {} Ix {} vs {}", s.kind, md.inertia.x, i.x);
        assert!(close(md.inertia.y, i.y), "kind {} Iy {} vs {}", s.kind, md.inertia.y, i.y);
        assert!(close(md.inertia.z, i.z), "kind {} Iz {} vs {}", s.kind, md.inertia.z, i.z);
    }
}

// ---- kinematic, filters, sleeping ----

#[test]
fn kinematic_platform_carries_a_box() {
    let mut f = new_frame();
    let mut plat = Body::new_kinematic(v3!(0, -0.5, 0));
    plat.vel = v3!(2, 0, 0);
    spawn_body(&mut f, plat, Collider::new(Shape::cuboid(fp!(50), FP::HALF, fp!(5))).with_friction(fp!(0.8)));
    let b = spawn_box(&mut f, v3!(0, 0.5, 0), v3!(0.5, 0.5, 0.5));
    let _ = settle(&mut f, 120);
    let bb = body(&f, b);
    // Two seconds at 2 m/s, carried by friction.
    assert!((bb.pos.x - fp!(4)).abs() < fp!(0.3), "x {}", bb.pos.x);
    assert!((bb.pos.y - FP::HALF).abs() < fp!(0.03));
}

#[test]
fn collision_filters_let_masked_bodies_pass() {
    let mut f = new_frame();
    // Ground only collides with layer 1.
    spawn_body(
        &mut f,
        Body::new_static(v3!(0, -0.5, 0)),
        Collider::new(Shape::cuboid(fp!(50), FP::HALF, fp!(50))).with_filter(1, 1),
    );
    let s = Shape::sphere(FP::HALF);
    let ghost = spawn_body(&mut f, Body::new_dynamic(v3!(-2, 2, 0), &s, FP::ONE), Collider::new(s).with_filter(2, 2));
    let solid = spawn_body(&mut f, Body::new_dynamic(v3!(2, 2, 0), &s, FP::ONE), Collider::new(s));
    let _ = settle(&mut f, 200);
    assert!(body(&f, ghost).pos.y < -FP::ONE, "masked body should fall through");
    assert!((body(&f, solid).pos.y - FP::HALF).abs() < fp!(0.03));
}

#[test]
fn a_resting_stack_sleeps_and_wakes_on_impulse_and_on_contact() {
    let (mut f, es) = box_stack(5);
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 300);
    assert!(es.iter().all(|&e| is_asleep(&f, e)), "stack should sleep");
    assert_eq!(sc.stats().awake, 0);
    // The whole island wakes with one impulse on the top box.
    let top = body(&f, es[4]).pos;
    apply_impulse(&mut f, es[4], v3!(0.2, 0, 0), top);
    orr_physics3d::step(&mut f, &mut sc);
    assert!(es.iter().all(|&e| !is_asleep(&f, e)), "island should wake together");
    run(&mut f, &mut sc, 400);
    assert!(es.iter().all(|&e| is_asleep(&f, e)), "stack should fall asleep again");

    // A falling sphere wakes the sleeping stack it touches.
    let s = spawn_sphere(&mut f, v3!(0.3, 8, 0), fp!(0.3));
    let mut woke = false;
    for _ in 0..200 {
        orr_physics3d::step(&mut f, &mut sc);
        if es.iter().any(|&e| !is_asleep(&f, e)) {
            woke = true;
            break;
        }
    }
    assert!(woke, "contact with the sphere must wake the stack");
    let _ = s;
}

#[test]
fn removing_the_support_wakes_a_sleeping_box() {
    let mut f = new_frame();
    ground(&mut f);
    let lower = spawn_box(&mut f, v3!(0, 0.5, 0), v3!(0.5, 0.5, 0.5));
    let upper = spawn_box(&mut f, v3!(0, 1.51, 0), v3!(0.5, 0.5, 0.5));
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 300);
    // Only the lower box touches static ground. Remove it and the upper
    // box must fall to the ground.
    assert!(is_asleep(&f, upper));
    f.despawn(lower);
    run(&mut f, &mut sc, 200);
    let u = body(&f, upper);
    assert!((u.pos.y - FP::HALF).abs() < fp!(0.03), "upper y {}", u.pos.y);
}

#[test]
fn pyramid_and_big_pile_stay_inside_the_arena() {
    let mut f = pyramid(5);
    let _ = settle(&mut f, 600);
    for (_, (b,)) in f.query::<(&Body,)>() {
        if b.kind == BODY_DYNAMIC {
            assert!(b.pos.y > fp!(0.45) && b.pos.y < fp!(5.6), "{:?}", b.pos);
            assert!(b.vel.length() < fp!(0.1));
        }
    }
    let mut f = mixed_pile(150);
    let _ = settle(&mut f, 800);
    for (_, (b,)) in f.query::<(&Body,)>() {
        if b.kind == BODY_DYNAMIC {
            assert!(b.pos.x.abs() < fp!(6) && b.pos.z.abs() < fp!(6) && b.pos.y > fp!(0.1) && b.pos.y < fp!(8), "{:?}", b.pos);
        }
    }
}

#[test]
fn no_tunneling_through_a_wall_at_moderate_speed() {
    let mut f = new_frame();
    let mut cfg_none = false;
    let _ = &mut cfg_none;
    ground(&mut f);
    spawn_body(&mut f, Body::new_static(v3!(5, 2, 0)), Collider::new(Shape::cuboid(fp!(0.5), fp!(2), fp!(5))));
    let s = Shape::sphere(fp!(0.25));
    let e = spawn_body(&mut f, Body::new_dynamic(v3!(0, 1, 0), &s, FP::ONE).with_velocity(v3!(30, 0, 0)), Collider::new(s));
    let _ = settle(&mut f, 120);
    assert!(body(&f, e).pos.x < fp!(5), "sphere passed the wall at x {}", body(&f, e).pos.x);
}

// ---- queries ----

#[test]
fn raycast_hits_every_shape_with_the_right_distance_and_normal() {
    let mut f = new_frame();
    let sph = spawn_body(&mut f, Body::new_static(v3!(0, 0, 0)), Collider::new(Shape::sphere(fp!(1))));
    let bx = spawn_body(&mut f, Body::new_static(v3!(10, 0, 0)), Collider::new(Shape::cuboid(fp!(1), fp!(1), fp!(1))));
    let lie = FPQuat::from_axis_angle(FPVec3::Z, FP::HALF_PI);
    let cap = spawn_body(&mut f, Body::new_static(v3!(20, 0, 0)).with_rotation(lie), Collider::new(Shape::capsule(fp!(1), fp!(0.5))));
    let filter = QueryFilter::default();
    let h = raycast(&mut f, v3!(-5, 0, 0), FPVec3::X, fp!(100), filter).unwrap();
    assert_eq!(h.entity, sph);
    assert!((h.distance - fp!(4)).abs() < fp!(0.01), "{}", h.distance);
    assert!(h.normal.x < -fp!(0.99));
    let h = raycast(&mut f, v3!(5, 0.2, 0.1), FPVec3::X, fp!(100), filter).unwrap();
    assert_eq!(h.entity, bx);
    assert!((h.distance - fp!(4)).abs() < fp!(0.01));
    assert!(vnear(h.normal, -FPVec3::X));
    // Capsule lying along x at x = 20 spans [-1.5, 1.5] around it.
    let h = raycast(&mut f, v3!(15, 0.1, 0), FPVec3::X, fp!(100), filter).unwrap();
    assert_eq!(h.entity, cap);
    assert!((h.distance - fp!(3.5)).abs() < fp!(0.02), "{}", h.distance);
    // From above the cylinder part.
    let h = raycast(&mut f, v3!(20, 5, 0), -FPVec3::Y, fp!(100), filter).unwrap();
    assert_eq!(h.entity, cap);
    assert!((h.distance - fp!(4.5)).abs() < fp!(0.02), "{}", h.distance);
    assert!(h.normal.y > fp!(0.99));
    // Max distance, miss, and a start inside.
    assert!(raycast(&mut f, v3!(-5, 0, 0), FPVec3::X, fp!(3), filter).is_none());
    assert!(raycast(&mut f, v3!(-5, 3, 0), FPVec3::X, fp!(100), filter).is_none());
    let h = raycast(&mut f, v3!(0.2, 0, 0), FPVec3::X, fp!(10), filter).unwrap();
    assert_eq!((h.entity, h.distance), (sph, FP::ZERO));
    // Layer mask.
    assert!(raycast(&mut f, v3!(-5, 0, 0), FPVec3::X, fp!(5), QueryFilter { mask: 2 }).is_none());
}

fn vnear(a: FPVec3, b: FPVec3) -> bool {
    (a - b).length() < fp!(0.01)
}

#[test]
fn raycast_hits_a_rotated_box_face() {
    let mut f = new_frame();
    let q = FPQuat::from_axis_angle(FPVec3::Y, FP::HALF_PI / 2);
    let bx = spawn_body(&mut f, Body::new_static(v3!(0, 0, 0)).with_rotation(q), Collider::new(Shape::cuboid(fp!(1), fp!(1), fp!(1))));
    // A box turned 45 degrees about y has its corner towards -x at 1.414.
    let h = raycast(&mut f, v3!(-5, 0, 0), FPVec3::X, fp!(100), QueryFilter::default()).unwrap();
    assert_eq!(h.entity, bx);
    assert!((h.distance - (fp!(5) - fp!(1.4142))).abs() < fp!(0.02), "{}", h.distance);
}

#[test]
fn sphere_cast_is_exact_for_sphere_box_and_capsule_targets() {
    let mut f = new_frame();
    let bx = spawn_body(&mut f, Body::new_static(v3!(10, 0, 0)), Collider::new(Shape::cuboid(fp!(1), fp!(1), fp!(1))));
    let filter = QueryFilter::default();
    // Straight at a face: stops one radius in front of it.
    let h = sphere_cast(&mut f, v3!(0, 0, 0), FPVec3::X, fp!(100), fp!(0.5), filter).unwrap();
    assert_eq!(h.entity, bx);
    assert!((h.distance - fp!(8.5)).abs() < fp!(0.01), "{}", h.distance);
    assert!(vnear(h.normal, -FPVec3::X));
    assert!((h.point.x - fp!(9)).abs() < fp!(0.01));
    // Past the edge: grazes the rounded edge at x = 9, y = 1.
    let h = sphere_cast(&mut f, v3!(0, 1.4, 0), FPVec3::X, fp!(100), fp!(0.5), filter).unwrap();
    // The center travels until it is 0.5 from the edge point (9, 1):
    // dy = 0.4, dx = sqrt(0.25 - 0.16) = 0.3 => x = 8.7.
    assert!((h.distance - fp!(8.7)).abs() < fp!(0.02), "{}", h.distance);
    assert!(h.normal.x < -fp!(0.5) && h.normal.y > fp!(0.5), "{:?}", h.normal);
    // A cast that misses by more than the radius.
    assert!(sphere_cast(&mut f, v3!(0, 1.6, 0), FPVec3::X, fp!(100), fp!(0.5), filter).is_none());
    // Sphere target: radii add.
    let sp = spawn_body(&mut f, Body::new_static(v3!(0, 0, 10)), Collider::new(Shape::sphere(fp!(1))));
    let h = sphere_cast(&mut f, v3!(0, 0, 0), FPVec3::Z, fp!(100), fp!(0.5), filter).unwrap();
    assert_eq!(h.entity, sp);
    assert!((h.distance - fp!(8.5)).abs() < fp!(0.01));
}
