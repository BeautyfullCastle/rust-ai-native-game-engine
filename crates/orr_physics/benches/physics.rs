//! Bench: cost of one physics `step` with 1000 and 5000 dynamic bodies.
//!
//! - `pile_settling_*`: bodies piled in a walled arena after 300 ticks
//!   (still moving, contacts and warm starting active).
//! - `pile_settled_*`: same deep pile after 1200 ticks. Loose piles keep
//!   jittering, so most of it stays awake.
//! - `field_settled_*`: bodies scattered in small stacks, after 1600 ticks
//!   (at rest, sleep-eligible: the typical game case).
//! - `shaker_awake_*`: the pile on a kinematic plate shaken at 3 Hz, so
//!   nothing ever rests. This is the all-awake worst case.
//! - `mixed_*`: the pile and shaker scenes with capsules (upright and
//!   tilted) as two of five body kinds.
//! - `rollback8_*`: the 8-tick resimulation a rollback would pay.
mod scenes;
use criterion::{criterion_group, criterion_main, Criterion};
use orr_ecs::Frame;
use orr_physics::{step, Scratch};
use scenes::{build, build_field, build_mixed, build_mixed_shaker, build_shaker, shake};

/// A scene plus whether the shaker plate has to be driven each tick.
struct Scene {
    frame: Frame,
    shaken: bool,
}

fn warmed(frame: Frame, shaken: bool, ticks: u32) -> Scene {
    let mut sc = Scratch::new();
    let mut events = Vec::new();
    let mut s = Scene { frame, shaken };
    for _ in 0..ticks {
        tick(&mut s, &mut sc, &mut events);
    }
    s
}

fn tick(s: &mut Scene, sc: &mut Scratch, events: &mut Vec<orr_physics::TriggerEvent>) {
    let t = s.frame.tick() + 1;
    s.frame.set_tick(t);
    if s.shaken {
        shake(&mut s.frame, t);
    }
    step(&mut s.frame, sc, events);
}

/// Steps forever, but returns to the warmed-up snapshot every 100 steps so
/// the measured state does not drift (a settling pile stays settling).
fn bench_step(c: &mut Criterion, name: &str, mut s: Scene) {
    let mut sc = Scratch::new();
    let mut events = Vec::new();
    let snapshot = s.frame.clone();
    let mut n = 0u32;
    c.bench_function(name, |b| {
        b.iter(|| {
            if n % 100 == 0 {
                s.frame.copy_from(&snapshot);
            }
            n += 1;
            events.clear();
            tick(&mut s, &mut sc, &mut events);
        })
    });
}

fn bench_resim(c: &mut Criterion, name: &str, mut s: Scene) {
    let mut sc = Scratch::new();
    let mut events = Vec::new();
    let snapshot = s.frame.clone();
    c.bench_function(name, |b| {
        b.iter(|| {
            s.frame.copy_from(&snapshot);
            for _ in 0..8 {
                events.clear();
                tick(&mut s, &mut sc, &mut events);
            }
        })
    });
}

fn benches(c: &mut Criterion) {
    for n in [1000, 5000] {
        bench_step(c, &format!("pile_settling_{n}"), warmed(build(n), false, 300));
    }
    for n in [1000, 5000] {
        bench_step(c, &format!("pile_settled_{n}"), warmed(build(n), false, 1200));
    }
    for n in [1000, 5000] {
        bench_step(c, &format!("shaker_awake_{n}"), warmed(build_shaker(n), true, 400));
    }
    for n in [1000, 5000] {
        bench_step(c, &format!("field_settled_{n}"), warmed(build_field(n), false, 1600));
    }
    bench_step(c, "mixed_settling_1000", warmed(build_mixed(1000), false, 300));
    bench_step(c, "mixed_shaker_awake_1000", warmed(build_mixed_shaker(1000), true, 400));
    bench_resim(c, "rollback8_pile_settling_1000", warmed(build(1000), false, 300));
    bench_resim(c, "rollback8_pile_settled_1000", warmed(build(1000), false, 1200));
    bench_resim(c, "rollback8_shaker_awake_1000", warmed(build_shaker(1000), true, 400));
    bench_resim(c, "rollback8_field_settled_1000", warmed(build_field(1000), false, 1600));
    bench_resim(c, "rollback8_mixed_shaker_awake_1000", warmed(build_mixed_shaker(1000), true, 400));
}

criterion_group!(physics, benches);
criterion_main!(physics);
