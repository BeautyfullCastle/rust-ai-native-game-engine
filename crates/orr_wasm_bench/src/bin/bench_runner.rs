//! Runs the cases with `std::time::Instant`: natively and as `wasm32-wasip1` under wasmtime.
//!
//! ```text
//! bench_runner [--ms 300] [--list] [case-substring ...]
//! ```
//! Prints `RESULT <name> <ns_per_iter> <ns_per_unit> <iters> <checksum>` lines.
#![allow(clippy::disallowed_types)]
#![allow(clippy::float_arithmetic)]

use std::time::{Duration, Instant};

use orr_wasm_bench::cases;

fn main() {
    let mut target_ms = 300u64;
    let mut filters = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--ms" => target_ms = args.next().and_then(|v| v.parse().ok()).expect("--ms N"),
            "--list" => {
                for c in cases() {
                    println!("{}", c.name);
                }
                return;
            }
            other => filters.push(other.to_string()),
        }
    }
    let target = Duration::from_millis(target_ms);
    println!("TARGET {}", std::env::consts::ARCH);
    for c in cases() {
        if !filters.is_empty() && !filters.iter().any(|f| c.name.contains(f.as_str())) {
            continue;
        }
        let mut run = (c.make)();
        // Calibrate: grow the iteration count until one run takes a good fraction of the target.
        let mut iters = 1u64;
        loop {
            let t = Instant::now();
            run(iters);
            let el = t.elapsed();
            if el >= target / 4 || iters >= 1 << 40 {
                let per = el.as_secs_f64() / iters as f64;
                iters = ((target.as_secs_f64() / per) as u64).max(1);
                break;
            }
            iters *= if el < Duration::from_micros(500) { 16 } else { 2 };
        }
        // Best of 3 timed runs (lowest noise); the checksum of the last run is printed.
        let mut best = f64::MAX;
        let mut sum = 0;
        for _ in 0..3 {
            let t = Instant::now();
            sum = run(iters);
            best = best.min(t.elapsed().as_secs_f64());
        }
        let ns_iter = best * 1e9 / iters as f64;
        println!("RESULT {} {:.3} {:.3} {} {:016x}", c.name, ns_iter, ns_iter / c.units_per_iter as f64, iters, sum);
    }
}
