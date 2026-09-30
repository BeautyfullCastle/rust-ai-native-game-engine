//! Seek latency of a play session on the physics scene: how long the editor's
//! timeline scrubber waits after a seek.
//!
//! ```text
//! cargo bench -p orr_sample --bench seek -- [bodies]
//! ```
//! Runs 1200 ticks, then seeks back 600 ticks (and other distances) with
//! several keyframe settings. Prints the median of a few runs in ms. This is
//! a plain `main`, not criterion: one seek is long, and the state must be
//! rebuilt between runs.
#![allow(clippy::disallowed_types, clippy::float_arithmetic)] // a bench: wall clock and floats are fine
use std::time::Instant;

use orr_sample::physics_game::{PhysConfig, PhysGame, PhysInput, SceneMode};
use orr_session::{ControlOp, PlayConfig, PlaySession, PlayerSlot};

const TICKS: u64 = 1200;
const RUNS: usize = 5;

fn build(bodies: u32, keyframe_interval: u64, ring: u32) -> PlaySession<PhysGame> {
    let mut cfg = PlayConfig::new(2, 42, 60);
    cfg.keyframe_interval = keyframe_interval;
    cfg.ring_capacity = ring;
    let mut s = PlaySession::<PhysGame>::new(cfg, PhysConfig::new(bodies, SceneMode::Rain));
    s.control(ControlOp::Pause);
    for tick in 1..=TICKS {
        let leg = tick / 90;
        let (ax, ay) = match leg % 4 {
            0 => (1, 0),
            1 => (0, -1),
            2 => (-1, 0),
            _ => (0, 1),
        };
        s.set_input(PlayerSlot(0), PhysInput::new(ax, ay, 1, tick % 200 < 15));
        s.control(ControlOp::Step(1));
    }
    s
}

fn median_ms(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn main() {
    let bodies: u32 = std::env::args().skip_while(|a| a != "--").nth(1).and_then(|a| a.parse().ok()).unwrap_or(1000);
    println!("physics scene, {bodies} bodies, {TICKS} ticks recorded, median of {RUNS} runs");
    println!("{:<52} {:>10}", "case", "ms");
    let cases: [(&str, u64, u32, u64); 6] = [
        ("back 600, keyframe every 60, ring 128 (hits a keyframe)", 60, 128, 600),
        ("back 600, keyframe every 60, worst target (+59 resim)", 60, 128, 599),
        ("back 600, keyframe every 300, ring 128", 300, 128, 599),
        ("back 600, no keyframes but the first (600 resim)", 0, 128, 600),
        ("back 60, ring 128 (exact ring frame)", 60, 128, 1140),
        ("back 1, ring 128", 60, 128, 1199),
    ];
    for (name, interval, ring, target) in cases {
        let mut times = Vec::new();
        for _ in 0..RUNS {
            let mut s = build(bodies, interval, ring);
            let t = Instant::now();
            s.control(ControlOp::Seek(target));
            times.push(t.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(s.head_tick(), target);
            assert_eq!(s.frame().checksum(), s.checksum_at(target).unwrap());
        }
        println!("{:<52} {:>10.2}", name, median_ms(times));
    }

    // Forward: from tick 600 to 1050, outside the ring (keyframe 1020, then 30 ticks).
    let mut times = Vec::new();
    for _ in 0..RUNS {
        let mut s = build(bodies, 60, 128);
        s.control(ControlOp::Seek(600));
        let t = Instant::now();
        s.control(ControlOp::Seek(1050));
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(s.frame().checksum(), s.checksum_at(1050).unwrap());
    }
    println!("{:<52} {:>10.2}", "forward 600 -> 1050 (keyframe + 30 resim)", median_ms(times));

    // Cost of recording: a live tick with checksum, ring copy and keyframes.
    let mut s = build(bodies, 60, 128);
    let t = Instant::now();
    for tick in TICKS + 1..=TICKS + 300 {
        s.set_input(PlayerSlot(0), PhysInput::new(1, 0, 1, tick % 200 < 15));
        s.control(ControlOp::Step(1));
    }
    println!("{:<52} {:>10.3}", "live tick incl. recording (avg of 300)", t.elapsed().as_secs_f64() * 1000.0 / 300.0);
    println!("saved .orrp of {} ticks: {} KB", s.last_tick(), s.save_replay().len() / 1024);
}
