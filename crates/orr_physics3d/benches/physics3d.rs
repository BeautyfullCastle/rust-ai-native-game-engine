//! Bench: cost of one 3D physics `step`.
//!
//! - `mixed_settling_*`: spheres, boxes and capsules (a third each) dropped
//!   into a walled arena, 90 ticks in: everything is awake, contacts and
//!   warm starting are active (the all-awake worst case).
//! - `mixed_settled_*`: the same pile after 600 ticks with sleeping off:
//!   everything is awake and resting on each other.
//! - `field_sleeping_*`: bodies scattered as small separate groups on a
//!   floor, after 900 ticks with sleeping on (almost everything asleep: the
//!   typical game case). A connected pile is one sleep island and stays
//!   awake as long as any body in it moves.
//! - `rollback8_*`: the 8-tick resimulation a rollback would pay.
#[path = "../tests/common/mod.rs"]
mod common;

use common::scenes::{bench_field, bench_pile};
use criterion::{criterion_group, criterion_main, Criterion};
use orr_ecs::Frame;
use orr_physics3d::{step, Scratch};

fn tick(f: &mut Frame, sc: &mut Scratch) {
    let t = f.tick() + 1;
    f.set_tick(t);
    step(f, sc);
}

fn warmed(n: i32, sleeping: bool, ticks: u32) -> Frame {
    run_warm(bench_pile(n, sleeping), ticks)
}

fn warmed_field(n: i32, ticks: u32) -> Frame {
    run_warm(bench_field(n, true), ticks)
}

fn run_warm(mut f: Frame, ticks: u32) -> Frame {
    let mut sc = Scratch::new();
    for _ in 0..ticks {
        tick(&mut f, &mut sc);
    }
    f
}

/// Steps forever, but returns to the warmed-up snapshot every 100 steps so
/// the measured state does not drift.
fn bench_step(c: &mut Criterion, name: &str, mut f: Frame) {
    let mut sc = Scratch::new();
    let snapshot = f.clone();
    let mut n = 0u32;
    c.bench_function(name, |b| {
        b.iter(|| {
            if n % 100 == 0 {
                f.copy_from(&snapshot);
            }
            n += 1;
            tick(&mut f, &mut sc);
        })
    });
}

fn bench_resim(c: &mut Criterion, name: &str, mut f: Frame) {
    let mut sc = Scratch::new();
    let snapshot = f.clone();
    c.bench_function(name, |b| {
        b.iter(|| {
            f.copy_from(&snapshot);
            for _ in 0..8 {
                tick(&mut f, &mut sc);
            }
        })
    });
}

fn benches(c: &mut Criterion) {
    for n in [500, 1000] {
        bench_step(c, &format!("mixed_settling_{n}"), warmed(n, false, 90));
    }
    for n in [500, 1000] {
        bench_step(c, &format!("mixed_settled_{n}"), warmed(n, false, 600));
    }
    for n in [500, 1000] {
        bench_step(c, &format!("field_sleeping_{n}"), warmed_field(n, 900));
    }
    bench_resim(c, "rollback8_mixed_settling_1000", warmed(1000, false, 90));
    bench_resim(c, "rollback8_field_sleeping_1000", warmed_field(1000, 900));
}

criterion_group!(physics3d, benches);
criterion_main!(physics3d);
