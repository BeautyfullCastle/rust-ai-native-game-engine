//! `orr_wasm_bench`: the same workloads on every target.
//!
//! Each [`Case`] builds its state once (`make`, not timed) and returns a closure
//! that runs `iters` iterations and returns a checksum. The checksum keeps the
//! optimizer honest and doubles as a cross-target determinism check: it is
//! identical on x86_64, aarch64 and wasm32 (see `tests/checksums.rs`).
//!
//! Timing lives outside the library so the library has no clock:
//! * `bench_runner` (bin): native and `wasm32-wasip1` (wasmtime), `std::time::Instant`.
//! * `wasm` module (wasm32-unknown-unknown): exports for a page that times with `performance.now()`.
//!
//! Run: `tools/wasm_bench.sh` (see docs/wasm-bench.md).
#![deny(clippy::disallowed_types)]
#![deny(clippy::float_arithmetic)]

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistry, ComponentRegistryBuilder, Frame};
use orr_fp::{FPVec3, FP};
use orr_games::physics_game::{bot_input, NoCommand, PhysConfig, PhysGame, PhysInput, SceneMode};
use orr_games::yard3d_game::{NoCommand as YardNoCommand, Yard3D, YardConfig, YardInput};
use orr_sim::{PlayerSlot, Simulation, TickInputs};

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod wasm;

/// One workload: `make` builds the state, the returned closure runs `iters` iterations.
pub struct Case {
    pub name: &'static str,
    /// What one iteration is (for the report).
    pub unit: &'static str,
    /// Iterations per `iters` count that make one "unit" (1 for per-op cases; the entity count for ECS passes).
    pub units_per_iter: u64,
    pub make: fn() -> Box<dyn FnMut(u64) -> u64>,
}

/// Runs a case for a fixed number of iterations from a fresh state and returns the checksum.
pub fn run_fresh(case: &Case, iters: u64) -> u64 {
    (case.make)()(iters)
}

const N: usize = 4096;

fn lcg(s: &mut u64) -> u64 {
    *s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    *s >> 11
}

/// Game-like raw values: random magnitude between 2^8 and 2^40, random sign.
fn raws(seed: u64) -> Vec<i64> {
    let mut s = seed;
    (0..N)
        .map(|_| {
            let bits = 8 + (lcg(&mut s) % 33) as u32;
            let v = (lcg(&mut s) & ((1u64 << bits) - 1)) as i64;
            if lcg(&mut s) & 1 == 0 {
                v
            } else {
                -v
            }
        })
        .collect()
}

fn micro(f: impl Fn(usize) -> u64 + 'static) -> Box<dyn FnMut(u64) -> u64> {
    Box::new(move |iters| {
        let mut acc = 0u64;
        for i in 0..iters as usize {
            acc = acc.wrapping_add(f(i & (N - 1)));
        }
        acc
    })
}

fn make_mul() -> Box<dyn FnMut(u64) -> u64> {
    let (a, b) = (raws(1), raws(2));
    micro(move |i| (FP::from_raw(a[i]) * FP::from_raw(b[i])).raw() as u64)
}

/// Small operands (products fit in i64): the physics fast path.
fn make_mul_small() -> Box<dyn FnMut(u64) -> u64> {
    let (a, b) = (raws(3), raws(4));
    micro(move |i| (FP::from_raw(a[i] >> 12) * FP::from_raw(b[i] >> 12)).raw() as u64)
}

fn make_div() -> Box<dyn FnMut(u64) -> u64> {
    let (a, b) = (raws(5), raws(6));
    micro(move |i| (FP::from_raw(a[i]) / FP::from_raw(b[i] | 1)).raw() as u64)
}

fn make_sqrt() -> Box<dyn FnMut(u64) -> u64> {
    let a = raws(7);
    micro(move |i| FP::from_raw(a[i].abs()).sqrt().raw() as u64)
}

fn make_sin_cos() -> Box<dyn FnMut(u64) -> u64> {
    let a = raws(8);
    micro(move |i| {
        let (s, c) = FP::from_raw(a[i] % 411_775).sin_cos();
        (s.raw() ^ c.raw().rotate_left(7)) as u64
    })
}

fn make_atan2() -> Box<dyn FnMut(u64) -> u64> {
    let (a, b) = (raws(9), raws(10));
    micro(move |i| FP::from_raw(a[i]).atan2(FP::from_raw(b[i] | 1)).raw() as u64)
}

fn make_normalize() -> Box<dyn FnMut(u64) -> u64> {
    let (a, b, c) = (raws(11), raws(12), raws(13));
    micro(move |i| {
        let v = FPVec3::new(FP::from_raw(a[i] >> 8), FP::from_raw(b[i] >> 8), FP::from_raw(c[i] >> 8) + FP::ONE);
        let n = v.normalize();
        (n.x.raw() ^ n.y.raw() ^ n.z.raw()) as u64
    })
}

// ---- ECS ----

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Pos {
    x: i64,
    y: i64,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vel {
    x: i64,
    y: i64,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Extra {
    a: i64,
    b: i64,
}

const ECS_N: i64 = 100_000;

fn registry() -> Arc<ComponentRegistry> {
    let mut b = ComponentRegistryBuilder::new();
    b.register_component::<Pos>("Pos");
    b.register_component::<Vel>("Vel");
    b.register_component::<Extra>("Extra");
    b.build()
}

fn ecs_frame() -> Frame {
    let mut f = Frame::new(registry());
    for i in 0..ECS_N {
        let e = f.spawn();
        f.add(e, Pos { x: i, y: -i });
        f.add(e, Vel { x: i % 7, y: i % 5 });
        f.add(e, Extra { a: i, b: i ^ 0x55 });
    }
    f
}

fn make_ecs_integrate() -> Box<dyn FnMut(u64) -> u64> {
    let mut f = ecs_frame();
    Box::new(move |iters| {
        for _ in 0..iters {
            f.query::<(&mut Pos, &Vel)>().for_each(|_, (p, v)| {
                p.x += v.x;
                p.y += v.y;
            });
        }
        f.checksum()
    })
}

fn make_ecs_checksum() -> Box<dyn FnMut(u64) -> u64> {
    let f = ecs_frame();
    Box::new(move |iters| {
        let mut acc = 0u64;
        for _ in 0..iters {
            acc = acc.wrapping_add(f.checksum());
        }
        acc
    })
}

fn make_ecs_copy() -> Box<dyn FnMut(u64) -> u64> {
    let src = ecs_frame();
    let mut dst = Frame::new(src.registry().clone());
    Box::new(move |iters| {
        for _ in 0..iters {
            dst.copy_from(&src);
        }
        dst.checksum()
    })
}

// ---- physics games ----

fn make_phys2d() -> Box<dyn FnMut(u64) -> u64> {
    let mut sim = Simulation::<PhysGame>::new(PhysConfig::new(1000, SceneMode::Mixer), 60, 777);
    let mut tick = 0u64;
    let mut step = move |sim: &mut Simulation<PhysGame>| {
        tick += 1;
        let mut i = TickInputs::<PhysInput, NoCommand>::new(tick, 2);
        i.set_input(PlayerSlot(0), bot_input(1234, tick, PlayerSlot(0)));
        i.set_input(PlayerSlot(1), bot_input(1234, tick, PlayerSlot(1)));
        sim.step(&i);
    };
    for _ in 0..60 {
        step(&mut sim);
    }
    Box::new(move |iters| {
        for _ in 0..iters {
            step(&mut sim);
        }
        sim.checksum()
    })
}

fn make_yard3d() -> Box<dyn FnMut(u64) -> u64> {
    let mut sim = Simulation::<Yard3D>::new(YardConfig::new(1000), 60, 777);
    let mut tick = 0u64;
    let mut step = move |sim: &mut Simulation<Yard3D>| {
        tick += 1;
        let mut i = TickInputs::<YardInput, YardNoCommand>::new(tick, 2);
        i.set_input(PlayerSlot(0), YardInput::default());
        i.set_input(PlayerSlot(1), YardInput::default());
        sim.step(&i);
    };
    for _ in 0..60 {
        step(&mut sim);
    }
    Box::new(move |iters| {
        for _ in 0..iters {
            step(&mut sim);
        }
        sim.checksum()
    })
}

/// Every case, in report order.
pub fn cases() -> Vec<Case> {
    let c = |name, unit, units_per_iter, make| Case { name, unit, units_per_iter, make };
    vec![
        c("fp_mul", "op", 1, make_mul),
        c("fp_mul_small", "op", 1, make_mul_small),
        c("fp_div", "op", 1, make_div),
        c("fp_sqrt", "op", 1, make_sqrt),
        c("fp_sin_cos", "op", 1, make_sin_cos),
        c("fp_atan2", "op", 1, make_atan2),
        c("fp_vec3_normalize", "op", 1, make_normalize),
        c("ecs_integrate_100k", "pass", ECS_N as u64, make_ecs_integrate),
        c("ecs_checksum_100k", "pass", ECS_N as u64, make_ecs_checksum),
        c("ecs_copy_100k", "pass", ECS_N as u64, make_ecs_copy),
        c("physics2d_1000_tick", "tick", 1, make_phys2d),
        c("physics3d_1000_tick", "tick", 1, make_yard3d),
    ]
}
