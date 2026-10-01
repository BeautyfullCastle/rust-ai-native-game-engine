//! Deterministic 3D scenes used by the golden, stability and bench code.
#![allow(dead_code)]

use super::*;
use orr_fp::FrameRng;

/// `n` unit boxes stacked with a small gap, on the ground.
pub fn box_stack(n: i32) -> (Frame, Vec<Entity>) {
    let mut f = new_frame();
    ground(&mut f);
    let mut es = vec![];
    for i in 0..n {
        let y = fp!(0.5) + FP::from_int(i) * fp!(1.002);
        es.push(spawn_box(&mut f, FPVec3::new(FP::ZERO, y, FP::ZERO), v3!(0.5, 0.5, 0.5)));
    }
    (f, es)
}

/// Layers of `base x base`, `base-1 x base-1`, ... unit boxes, each layer
/// shifted by half a box so it rests on four below.
pub fn pyramid(base: i32) -> Frame {
    let mut f = new_frame();
    ground(&mut f);
    for layer in 0..base {
        let n = base - layer;
        let off = FP::from_int(n - 1) * FP::HALF;
        for i in 0..n {
            for k in 0..n {
                let x = FP::from_int(i) - off;
                let z = FP::from_int(k) - off;
                let y = fp!(0.5) + FP::from_int(layer) * fp!(1.001);
                spawn_box(&mut f, FPVec3::new(x, y, z), v3!(0.5, 0.5, 0.5));
            }
        }
    }
    f
}

/// Spheres dropped in a grid onto three static boxes (a table, a step and
/// a tilted plate) over the ground.
pub fn sphere_rain(n: i32) -> Frame {
    let mut f = new_frame();
    ground(&mut f);
    spawn_body(&mut f, Body::new_static(v3!(0, 1, 0)), Collider::new(Shape::cuboid(fp!(2), fp!(0.5), fp!(2))));
    spawn_body(&mut f, Body::new_static(v3!(3.5, 0.5, 0)), Collider::new(Shape::cuboid(fp!(1), fp!(0.5), fp!(3))));
    spawn_body(
        &mut f,
        Body::new_static(v3!(-3.5, 1.2, 0)).with_rotation(FPQuat::from_axis_angle(FPVec3::Z, fp!(0.4))),
        Collider::new(Shape::cuboid(fp!(1.5), fp!(0.2), fp!(2))),
    );
    let mut rng = FrameRng::new(31);
    let cols = 5;
    for i in 0..n {
        let (cx, cz, layer) = (i % cols, (i / cols) % cols, i / (cols * cols));
        let x = FP::from_int(cx - 2) * fp!(2.2) + rng.range_fp(fp!(-0.2), fp!(0.2));
        let z = FP::from_int(cz - 2) * fp!(1.1) + rng.range_fp(fp!(-0.2), fp!(0.2));
        let y = fp!(4) + FP::from_int(layer) * fp!(1.3);
        let r = rng.range_fp(fp!(0.2), fp!(0.45));
        spawn_sphere(&mut f, FPVec3::new(x, y, z), r);
    }
    f
}

/// Capsules lying across a 20 degree ramp (axis along z), released at the
/// top; they roll down onto the ground.
pub fn ramp_capsules(n: i32) -> Frame {
    let mut f = new_frame();
    ground(&mut f);
    let tilt = FPQuat::from_axis_angle(FPVec3::Z, fp!(-0.35));
    // Ramp surface runs from (-4, 2.5) down to (4, 0).
    spawn_body(
        &mut f,
        Body::new_static(v3!(0, 1.2, 0)).with_rotation(tilt),
        Collider::new(Shape::cuboid(fp!(4.5), fp!(0.2), fp!(3))).with_friction(fp!(0.7)),
    );
    let axis_to_z = FPQuat::from_axis_angle(FPVec3::X, FP::HALF_PI);
    for i in 0..n {
        let x = fp!(-3) + FP::from_int(i) * fp!(0.9);
        let y = fp!(2.9) - FP::from_int(i) * fp!(0.31);
        let z = FP::from_int((i % 3) - 1) * fp!(0.2);
        spawn_capsule(&mut f, FPVec3::new(x, y + fp!(0.5), z), fp!(0.5), fp!(0.2), axis_to_z);
    }
    f
}

/// A walled arena with `n` mixed bodies (spheres, boxes, capsules; random
/// orientation) dropped from above: exercises every shape pair.
pub fn mixed_pile(n: i32) -> Frame {
    let mut f = new_frame();
    ground(&mut f);
    let half = fp!(6);
    for (sx, sz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        let pos = FPVec3::new(half * sx, fp!(5), half * sz);
        let (hx, hz) = if sx != 0 { (FP::HALF, half) } else { (half, FP::HALF) };
        spawn_body(&mut f, Body::new_static(pos), Collider::new(Shape::cuboid(hx, fp!(5), hz)));
    }
    let mut rng = FrameRng::new(0xB0D1E5);
    for i in 0..n {
        let x = rng.range_fp(fp!(-4.5), fp!(4.5));
        let z = rng.range_fp(fp!(-4.5), fp!(4.5));
        let y = fp!(1) + FP::from_int(i / 12) * fp!(1.1);
        let pos = FPVec3::new(x, y, z);
        let axis = FPVec3::new(rng.range_fp(fp!(-1), fp!(1)), rng.range_fp(fp!(0.2), fp!(1)), rng.range_fp(fp!(-1), fp!(1))).normalize_or_zero();
        let rot = FPQuat::from_axis_angle(axis, rng.range_fp(fp!(-3), fp!(3)));
        match i % 3 {
            0 => {
                spawn_sphere(&mut f, pos, rng.range_fp(fp!(0.25), fp!(0.5)));
            }
            1 => {
                let s = Shape::cuboid(rng.range_fp(fp!(0.25), fp!(0.5)), rng.range_fp(fp!(0.25), fp!(0.5)), rng.range_fp(fp!(0.25), fp!(0.5)));
                spawn_body(&mut f, Body::new_dynamic(pos, &s, FP::ONE).with_rotation(rot), Collider::new(s));
            }
            _ => {
                spawn_capsule(&mut f, pos, rng.range_fp(fp!(0.2), fp!(0.5)), rng.range_fp(fp!(0.2), fp!(0.35)), rot);
            }
        }
    }
    f
}

/// `n` mixed bodies (a third each: spheres, boxes, capsules) dropped in a
/// grid into a walled arena that grows with `n`. Four layers deep, so they
/// pile up and rest on each other: the benchmark scene.
pub fn bench_pile(n: i32, sleeping: bool) -> Frame {
    let mut cfg = PhysicsConfig::default();
    if !sleeping {
        cfg.sleep_ticks = 0;
    }
    let mut f = new_frame_with(cfg);
    // Columns per side so that n bodies fit in 4 layers.
    let mut side = 4;
    while side * side * 4 < n {
        side += 1;
    }
    let cell = fp!(1.3);
    let half = cell * side / 2 + FP::ONE;
    spawn_body(
        &mut f,
        Body::new_static(v3!(0, -0.5, 0)),
        Collider::new(Shape::cuboid(half + FP::ONE, FP::HALF, half + FP::ONE)).with_friction(fp!(0.6)),
    );
    for (sx, sz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        let pos = FPVec3::new((half + FP::HALF) * sx, fp!(10), (half + FP::HALF) * sz);
        let (hx, hz) = if sx != 0 { (FP::HALF, half + FP::ONE) } else { (half + FP::ONE, FP::HALF) };
        spawn_body(&mut f, Body::new_static(pos), Collider::new(Shape::cuboid(hx, fp!(10), hz)));
    }
    let mut rng = FrameRng::new(99);
    for i in 0..n {
        let (cx, cz, layer) = (i % side, (i / side) % side, i / (side * side));
        let x = (FP::from_int(cx) - FP::from_int(side - 1) * FP::HALF) * cell + rng.range_fp(fp!(-0.1), fp!(0.1));
        let z = (FP::from_int(cz) - FP::from_int(side - 1) * FP::HALF) * cell + rng.range_fp(fp!(-0.1), fp!(0.1));
        let y = fp!(0.7) + FP::from_int(layer) * fp!(1.2);
        let pos = FPVec3::new(x, y, z);
        let axis = FPVec3::new(rng.range_fp(fp!(-1), fp!(1)), rng.range_fp(fp!(0.2), fp!(1)), rng.range_fp(fp!(-1), fp!(1))).normalize_or_zero();
        let rot = FPQuat::from_axis_angle(axis, rng.range_fp(fp!(-3), fp!(3)));
        match i % 3 {
            0 => {
                let e = spawn_sphere(&mut f, pos, rng.range_fp(fp!(0.3), fp!(0.45)));
                let b = f.get_mut::<Body>(e).unwrap();
                b.linear_damping = fp!(0.05);
                b.angular_damping = fp!(0.5);
            }
            1 => {
                let h = rng.range_fp(fp!(0.3), fp!(0.45));
                let s = Shape::cuboid(h, h, h);
                spawn_body(&mut f, Body::new_dynamic(pos, &s, FP::ONE).with_rotation(rot).with_damping(fp!(0.05), fp!(0.5)), Collider::new(s));
            }
            _ => {
                let e = spawn_capsule(&mut f, pos, rng.range_fp(fp!(0.2), fp!(0.4)), rng.range_fp(fp!(0.2), fp!(0.3)), rot);
                let b = f.get_mut::<Body>(e).unwrap();
                b.linear_damping = fp!(0.05);
                b.angular_damping = fp!(0.5);
            }
        }
    }
    f
}

/// `n` mixed bodies scattered as small separate groups (single spheres,
/// boxes, lying capsules and two-box stacks) on a large floor, dropped from
/// a small height. They settle and fall asleep group by group: the typical
/// game case.
pub fn bench_field(n: i32, sleeping: bool) -> Frame {
    let mut cfg = PhysicsConfig::default();
    if !sleeping {
        cfg.sleep_ticks = 0;
    }
    let mut f = new_frame_with(cfg);
    let mut side = 4;
    while side * side < n {
        side += 1;
    }
    let cell = fp!(2.4);
    let half = cell * side / 2 + fp!(2);
    spawn_body(
        &mut f,
        Body::new_static(v3!(0, -0.5, 0)),
        Collider::new(Shape::cuboid(half, FP::HALF, half)).with_friction(fp!(0.6)),
    );
    let mut rng = FrameRng::new(1234);
    let lie = FPQuat::from_axis_angle(FPVec3::Z, FP::HALF_PI);
    let (mut count, mut i) = (0, 0);
    while count < n {
        let (cx, cz) = (i % side, i / side);
        i += 1;
        let x = (FP::from_int(cx) - FP::from_int(side - 1) * FP::HALF) * cell + rng.range_fp(fp!(-0.15), fp!(0.15));
        let z = (FP::from_int(cz) - FP::from_int(side - 1) * FP::HALF) * cell + rng.range_fp(fp!(-0.15), fp!(0.15));
        match i % 4 {
            0 => {
                spawn_sphere(&mut f, FPVec3::new(x, fp!(0.6), z), fp!(0.4));
                count += 1;
            }
            1 => {
                spawn_box(&mut f, FPVec3::new(x, fp!(0.6), z), v3!(0.45, 0.45, 0.45));
                count += 1;
            }
            2 => {
                spawn_capsule(&mut f, FPVec3::new(x, fp!(0.5), z), fp!(0.5), fp!(0.3), lie);
                count += 1;
            }
            _ => {
                spawn_box(&mut f, FPVec3::new(x, fp!(0.5), z), v3!(0.45, 0.45, 0.45));
                spawn_box(&mut f, FPVec3::new(x, fp!(1.45), z), v3!(0.4, 0.4, 0.4));
                count += 2;
            }
        }
    }
    f
}
