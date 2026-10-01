mod common;
use common::scenes::*;
use orr_physics3d::{step_probed, Phase, Scratch};
use std::time::{Duration, Instant};

fn profile(n: i32, warm: u32, measure: u32) {
    let mut f = bench_pile(n, false);
    let mut sc = Scratch::new();
    for _ in 0..warm {
        orr_physics3d::step(&mut f, &mut sc);
    }
    let mut acc = [Duration::ZERO; 9];
    let mut total = Duration::ZERO;
    for _ in 0..measure {
        let t0 = Instant::now();
        let mut last = t0;
        step_probed(&mut f, &mut sc, &mut |p| {
            let now = Instant::now();
            acc[p as usize] += now - last;
            last = now;
        });
        total += t0.elapsed();
    }
    let names = ["gather", "transforms", "broad", "narrow", "integrate", "prepare", "solve", "sleep", "finish"];
    let mut line = String::new();
    for (i, nm) in names.iter().enumerate() {
        line.push_str(&format!("{nm} {:.0}us  ", acc[i].as_micros() as f64 / measure as f64));
    }
    println!("P n={n} total {:.0}us/tick | {line} | {:?}", total.as_micros() as f64 / measure as f64, sc.stats());
    let _ = Phase::Gather;
}

#[test]
fn prof() {
    profile(500, 90, 60);
    profile(1000, 90, 60);
    profile(500, 400, 60);
}
