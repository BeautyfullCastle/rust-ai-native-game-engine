//! Behavior tests for the 3D physics: shapes, pairs, stability.
mod common;

use common::*;
use orr_fp::{fp, FPQuat, FPVec3, FP};
use orr_physics3d::{is_asleep, Scratch};

#[test]
fn sphere_rests_on_ground() {
    let mut f = new_frame();
    let mut sc = Scratch::new();
    ground(&mut f);
    let s = spawn_sphere(&mut f, v3!(0, 2, 0), FP::HALF);
    run(&mut f, &mut sc, 240);
    let b = body(&f, s);
    println!("sphere pos {:?} vel {:?}", b.pos, b.vel);
    assert!((b.pos.y - FP::HALF).abs() < fp!(0.02));
}

#[test]
fn box_rests_on_ground() {
    let mut f = new_frame();
    let mut sc = Scratch::new();
    ground(&mut f);
    let s = spawn_box(&mut f, v3!(0, 2, 0), v3!(0.5, 0.5, 0.5));
    run(&mut f, &mut sc, 240);
    let b = body(&f, s);
    println!("box pos {:?} vel {:?} rot {:?}", b.pos, b.vel, b.rot);
    assert!((b.pos.y - FP::HALF).abs() < fp!(0.02));
}

#[test]
fn stack_of_five() {
    let mut f = new_frame();
    let mut sc = Scratch::new();
    ground(&mut f);
    let mut es = vec![];
    for i in 0..5 {
        es.push(spawn_box(&mut f, v3!(0, 0.5, 0) + FPVec3::new(FP::ZERO, FP::from_int(i) * fp!(1.01), FP::ZERO), v3!(0.5, 0.5, 0.5)));
    }
    run(&mut f, &mut sc, 300);
    for e in &es {
        let b = body(&f, *e);
        println!("pos {:?} v {:?} asleep {}", b.pos, b.vel, is_asleep(&f, *e));
    }
    println!("{:?}", sc.stats());
    let top = body(&f, es[4]);
    assert!((top.pos.x).abs() < fp!(0.05) && (top.pos.z).abs() < fp!(0.05));
    assert!((top.pos.y - fp!(4.5)).abs() < fp!(0.1));
    let _ = FPQuat::IDENTITY;
}
