//! Stability: a tall stack stays standing, a resting sphere does not
//! jitter, energy does not explode. The numbers are printed so a run with
//! `--nocapture` reports the drift.
mod common;

use common::scenes::*;
use common::*;
use orr_fp::{fp, FPVec3, FP};
use orr_physics3d::{Body, PhysicsConfig, Scratch, BODY_DYNAMIC};

fn no_sleep() -> PhysicsConfig {
    let mut c = PhysicsConfig::default();
    c.sleep_ticks = 0;
    c
}

#[test]
fn ten_box_stack_stays_standing_for_600_ticks_without_sleeping() {
    let (mut f, es) = box_stack(10);
    // Same scene, but sleeping off so the solver has to hold it up.
    let mut cfg = no_sleep();
    cfg.velocity_iterations = PhysicsConfig::default().velocity_iterations;
    f.singleton_mut::<orr_physics3d::PhysicsState>().config = cfg;
    let mut sc = Scratch::new();
    let (mut max_xz, mut max_speed, mut min_y_err, mut max_y_err) = (FP::ZERO, FP::ZERO, FP::ZERO, FP::ZERO);
    for t in 0..600 {
        orr_physics3d::step(&mut f, &mut sc);
        if t >= 120 {
            for (i, &e) in es.iter().enumerate() {
                let b = body(&f, e);
                max_xz = max_xz.max(b.pos.x.abs()).max(b.pos.z.abs());
                max_speed = max_speed.max(b.vel.length());
                let err = b.pos.y - (fp!(0.5) + FP::from_int(i as i32) * fp!(1.0));
                min_y_err = min_y_err.min(err);
                max_y_err = max_y_err.max(err);
            }
        }
    }
    let top = body(&f, es[9]);
    println!(
        "stack10 after 600 ticks: top at {:?}; max lateral drift {max_xz}, max speed {max_speed}, y error [{min_y_err}, {max_y_err}]",
        top.pos
    );
    assert!(max_xz < fp!(0.05), "lateral drift {max_xz}");
    assert!(max_speed < fp!(0.1), "speed {max_speed}");
    // The gap in the scene is 0.002 per box, the slop sinks them slightly.
    assert!(min_y_err > -fp!(0.1) && max_y_err < fp!(0.1), "vertical error [{min_y_err}, {max_y_err}]");
}

#[test]
fn twenty_box_stack_with_sleeping_stays_standing() {
    let (mut f, es) = box_stack(20);
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 900);
    let top = body(&f, es[19]);
    println!("stack20 after 900 ticks: top at {:?}, asleep {}", top.pos, sc.stats().asleep);
    assert!(top.pos.x.abs() < fp!(0.1) && top.pos.z.abs() < fp!(0.1));
    assert!(top.pos.y > fp!(19.0));
}

#[test]
fn resting_sphere_does_not_jitter() {
    let mut f = new_frame_with(no_sleep());
    ground(&mut f);
    let e = spawn_sphere(&mut f, v3!(0, 0.6, 0), FP::HALF);
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 120);
    let mut max_step = FP::ZERO;
    let mut max_speed = FP::ZERO;
    let mut prev = body(&f, e).pos;
    let start = prev;
    for _ in 0..600 {
        orr_physics3d::step(&mut f, &mut sc);
        let b = body(&f, e);
        max_step = max_step.max((b.pos - prev).length());
        max_speed = max_speed.max(b.vel.length());
        prev = b.pos;
    }
    let drift = (prev - start).length();
    println!("resting sphere: max position change per tick {max_step}, max speed {max_speed}, drift in 600 ticks {drift}");
    assert!(max_step < fp!(0.0005), "jitter {max_step}");
    assert!(max_speed < fp!(0.01));
    assert!(drift < fp!(0.002));
}

#[test]
fn resting_box_and_capsule_do_not_jitter() {
    let mut f = new_frame_with(no_sleep());
    ground(&mut f);
    let b = spawn_box(&mut f, v3!(-2, 0.6, 0), v3!(0.5, 0.4, 0.6));
    let lie = orr_fp::FPQuat::from_axis_angle(FPVec3::Z, FP::HALF_PI);
    let c = spawn_capsule(&mut f, v3!(2, 0.4, 0), fp!(0.7), fp!(0.25), lie);
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 180);
    let (pb, pc) = (body(&f, b).pos, body(&f, c).pos);
    let (mut worst, mut max_w) = (FP::ZERO, FP::ZERO);
    for _ in 0..600 {
        orr_physics3d::step(&mut f, &mut sc);
        for e in [b, c] {
            max_w = max_w.max(body(&f, e).omega.length());
            worst = worst.max(body(&f, e).vel.length());
        }
    }
    let drift = (body(&f, b).pos - pb).length().max((body(&f, c).pos - pc).length());
    println!("resting box/capsule: max speed {worst}, max spin {max_w}, drift {drift}");
    assert!(worst < fp!(0.02) && max_w < fp!(0.05), "speed {worst} spin {max_w}");
    assert!(drift < fp!(0.005));
}

#[test]
fn energy_stays_bounded_in_a_dense_mixed_pile() {
    let mut f = mixed_pile(100);
    let g = fp!(10);
    let energy = |f: &mut orr_ecs::Frame| {
        let mut e = FP::ZERO;
        for (_, (b,)) in f.query::<(&Body,)>() {
            if b.kind == BODY_DYNAMIC {
                let m = FP::ONE / b.inv_mass;
                e += m * g * b.pos.y + m * b.vel.length_sq() / 2;
            }
        }
        e
    };
    let e0 = energy(&mut f);
    let mut sc = Scratch::new();
    let mut peak = e0;
    for _ in 0..600 {
        orr_physics3d::step(&mut f, &mut sc);
        peak = peak.max(energy(&mut f));
    }
    println!("mixed pile energy: start {e0}, peak {peak}, end {}", energy(&mut f));
    assert!(peak <= e0 + e0 / 50, "energy grew from {e0} to {peak}");
    // At the end everything is on or near the floor and slow.
    let mut max_v = FP::ZERO;
    for (_, (b,)) in f.query::<(&Body,)>() {
        if b.kind == BODY_DYNAMIC {
            max_v = max_v.max(b.vel.length());
        }
    }
    assert!(max_v < fp!(3), "max speed {max_v}");
}
