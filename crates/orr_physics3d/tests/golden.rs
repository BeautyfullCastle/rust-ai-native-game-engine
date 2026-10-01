//! Determinism proofs for the 3D physics: golden checksums over fixed
//! scenes (they run on every CI target including wasm32-wasip1, and must
//! match bit for bit), rollback and serialization replays.
//!
//! Tests whose names start with `golden_` also run on wasm32 in CI.
mod common;

use common::scenes::*;
use common::*;
use orr_ecs::Frame;
use orr_fp::{fp, FP};
use orr_physics3d::{apply_impulse, Body, Scratch, BODY_DYNAMIC};

/// Pinned checksums. Update only for an intended behavior change, and say
/// why in the commit message.
const STACK_GOLDEN: u64 = 0;
const PYRAMID_GOLDEN: u64 = 0;
const RAIN_GOLDEN: u64 = 0;
const RAMP_GOLDEN: u64 = 0;
const MIXED_GOLDEN: u64 = 0;

fn sane(f: &mut Frame, limit: FP) {
    for (_, (b,)) in f.query::<(&Body,)>().filter(|(_, (b,))| b.kind == BODY_DYNAMIC) {
        assert!(b.pos.x.abs() < limit && b.pos.z.abs() < limit && b.pos.y > -FP::ONE && b.pos.y < limit, "body out of bounds {:?}", b.pos);
        assert!(b.vel.length() < fp!(80), "speed {}", b.vel.length());
    }
}

fn check(name: &str, f: &mut Frame, ticks: u32, golden: u64) {
    let mut sc = Scratch::new();
    run(f, &mut sc, ticks);
    sane(f, fp!(60));
    let cs = f.checksum();
    println!("{name} golden checksum: {cs:#018x}");
    assert_eq!(cs, golden, "{name} golden changed; only update for an intended behavior change");
}

#[test]
fn golden_box_stack_checksum() {
    let (mut f, _) = box_stack(10);
    check("stack10", &mut f, 600, STACK_GOLDEN);
}

#[test]
fn golden_pyramid_checksum() {
    let mut f = pyramid(5);
    check("pyramid5", &mut f, 600, PYRAMID_GOLDEN);
}

#[test]
fn golden_sphere_rain_checksum() {
    let mut f = sphere_rain(75);
    check("rain75", &mut f, 500, RAIN_GOLDEN);
}

#[test]
fn golden_ramp_capsules_checksum() {
    let mut f = ramp_capsules(6);
    check("ramp6", &mut f, 500, RAMP_GOLDEN);
}

#[test]
fn golden_mixed_pile_checksum() {
    let mut f = mixed_pile(120);
    check("mixed120", &mut f, 500, MIXED_GOLDEN);
}

#[test]
fn golden_two_runs_agree_and_scratch_history_is_irrelevant() {
    let (mut a, mut b) = (mixed_pile(60), mixed_pile(60));
    let mut sc = Scratch::new();
    run(&mut a, &mut sc, 200);
    // `b` gets a new Scratch every 7 ticks.
    for _ in 0..200 / 7 {
        run(&mut b, &mut Scratch::new(), 7);
    }
    run(&mut b, &mut Scratch::new(), 200 % 7);
    assert_eq!(a.checksum(), b.checksum());
}

#[test]
fn golden_rollback_replay() {
    let mut f = mixed_pile(80);
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 100);
    let snapshot = f.clone();
    run(&mut f, &mut sc, 100);
    let expected = f.checksum();
    assert_ne!(expected, snapshot.checksum());

    // Restore mid-simulation and resimulate with a fresh Scratch.
    f.copy_from(&snapshot);
    assert_eq!(f.checksum(), snapshot.checksum());
    run(&mut f, &mut Scratch::new(), 100);
    assert_eq!(f.checksum(), expected);

    // Eight-tick rollbacks, the way a session predicts: snapshot, run 8,
    // restore, run 8 again, every tick.
    let mut g = mixed_pile(80);
    let mut h = mixed_pile(80);
    let mut sg = Scratch::new();
    let mut sh = Scratch::new();
    for _ in 0..30 {
        let snap = g.clone();
        run(&mut g, &mut sg, 8);
        g.copy_from(&snap);
        run(&mut g, &mut sg, 1);
        run(&mut h, &mut sh, 1);
        assert_eq!(g.checksum(), h.checksum());
    }
}

#[test]
fn golden_sleep_and_wake_rollback() {
    let (mut f, es) = box_stack(6);
    let mut sc = Scratch::new();
    // Snapshot while the stack is falling asleep, then hit it.
    run(&mut f, &mut sc, 120);
    let snapshot = f.clone();
    let hit = |f: &mut Frame| {
        let p = body(f, es[5]).pos;
        apply_impulse(f, es[5], v3!(3, 1, 0), p);
    };
    run(&mut f, &mut sc, 250);
    hit(&mut f);
    run(&mut f, &mut sc, 200);
    let expected = f.checksum();

    f.copy_from(&snapshot);
    run(&mut f, &mut Scratch::new(), 250);
    hit(&mut f);
    run(&mut f, &mut Scratch::new(), 200);
    assert_eq!(f.checksum(), expected);
}

#[test]
fn golden_serialize_roundtrip() {
    let mut f = mixed_pile(80);
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 150);
    let bytes = f.to_bytes();
    let mut g = Frame::from_bytes(f.registry().clone(), &bytes).expect("decode");
    assert_eq!(g.checksum(), f.checksum());
    run(&mut f, &mut sc, 100);
    run(&mut g, &mut Scratch::new(), 100);
    assert_eq!(f.checksum(), g.checksum());
}

#[test]
fn spawn_and_despawn_order_does_not_change_the_result_of_the_same_bodies() {
    // The same set of bodies spawned with a despawned hole in the entity
    // table in one run must still agree with itself after a rollback.
    let mut f = mixed_pile(40);
    let victim = f.query::<(&Body,)>().filter(|(_, (b,))| b.kind == BODY_DYNAMIC).map(|(e, _)| e).nth(5).unwrap();
    let mut sc = Scratch::new();
    run(&mut f, &mut sc, 50);
    let snap = f.clone();
    f.despawn(victim);
    spawn_sphere(&mut f, v3!(0, 6, 0), fp!(0.4));
    run(&mut f, &mut sc, 100);
    let cs = f.checksum();
    f.copy_from(&snap);
    f.despawn(victim);
    spawn_sphere(&mut f, v3!(0, 6, 0), fp!(0.4));
    run(&mut f, &mut Scratch::new(), 100);
    assert_eq!(f.checksum(), cs);
}
