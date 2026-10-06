//! Instrumented phase diagnostics for the eight existing `physics3d` benchmarks.
//!
//! Run with `cargo run --release -p orr_physics3d --example phase_costs`.
//! No arguments: the workload is deliberately fixed. This is not a replacement
//! for the uninstrumented Criterion benchmark or evidence of a speedup.
//! Wall-clock probes live only in this diagnostic executable, never in the sim.
//!
//! JSONL output includes every tick and actual snapshot-reset copy. Checksums,
//! StepStats, assertions, output, and the unprobed validation replay are outside
//! the timed step, but can affect cache state. Times include probe overhead.
#[path = "../tests/common/mod.rs"]
mod common;

use common::scenes::{bench_field, bench_pile};
use orr_ecs::Frame;
use orr_physics3d::{step, step_probed, Phase, PhysicsState, Scratch, StepStats};
use std::hint::black_box;
use std::time::Instant;

const PHASE_COUNT: usize = 9;

fn phase_index(phase: Phase) -> usize {
    match phase {
        Phase::Gather => 0,
        Phase::Transforms => 1,
        Phase::Broad => 2,
        Phase::Narrow => 3,
        Phase::Integrate => 4,
        Phase::Prepare => 5,
        Phase::Solve => 6,
        Phase::Sleep => 7,
        Phase::Finish => 8,
    }
}

#[derive(Default)]
struct PhaseTimes {
    ns: [u128; PHASE_COUNT],
    calls: [u32; PHASE_COUNT],
}

impl PhaseTimes {
    fn record(&mut self, phase: Phase, ns: u128) {
        let i = phase_index(phase);
        self.ns[i] += ns;
        self.calls[i] += 1;
    }

    fn check(&self, total_ns: u128) {
        assert_eq!(self.calls[0], 1, "one Gather per tick");
        assert_eq!(self.calls[8], 1, "one Finish per tick");
        assert_eq!(self.calls[2], self.calls[3], "paired Broad/Narrow");
        if self.calls[1] == 0 {
            assert_eq!(&self.calls[1..8], &[0; 7], "no-mover early return");
        } else {
            assert_eq!(self.calls[1], 1);
            assert!(
                (1..=4).contains(&self.calls[2]),
                "initial plus at most three wake rounds"
            );
            assert_eq!(&self.calls[4..8], &[1; 4]);
        }
        assert!(
            self.ns.iter().sum::<u128>() <= total_ns,
            "phase intervals fit inside step"
        );
    }
}

struct TickRecord {
    tick: u64,
    checksum: u64,
    stats: StepStats,
    phases: PhaseTimes,
    total_ns: u128,
}

fn tick(frame: &mut Frame, scratch: &mut Scratch) {
    frame.set_tick(frame.tick() + 1);
    step(frame, scratch);
}

fn warmed(mut frame: Frame, ticks: u32) -> Frame {
    let mut scratch = Scratch::new();
    for _ in 0..ticks {
        tick(&mut frame, &mut scratch);
    }
    frame
}

fn measured_tick(frame: &mut Frame, scratch: &mut Scratch) -> TickRecord {
    frame.set_tick(frame.tick() + 1);
    let mut phases = PhaseTimes::default();
    let start = Instant::now();
    let mut previous = start;
    step_probed(
        black_box(&mut *frame),
        black_box(&mut *scratch),
        &mut |phase| {
            let now = Instant::now();
            phases.record(phase, now.duration_since(previous).as_nanos());
            previous = now;
        },
    );
    let total_ns = start.elapsed().as_nanos();
    phases.check(total_ns);
    TickRecord {
        tick: frame.tick(),
        checksum: frame.checksum(),
        stats: scratch.stats(),
        phases,
        total_ns,
    }
}

fn stats_values(stats: StepStats) -> [u32; 6] {
    [
        stats.bodies,
        stats.awake,
        stats.asleep,
        stats.pairs,
        stats.manifolds,
        stats.points,
    ]
}

fn run_case(
    name: &str,
    mut frame: Frame,
    repetitions: u32,
    ticks_per_repetition: u32,
    reset_every: u32,
) {
    let snapshot = frame.clone();
    let total_ticks = repetitions * ticks_per_repetition;
    assert_eq!(total_ticks % reset_every, 0);
    let mut scratch = Scratch::new();
    let mut records = Vec::with_capacity(total_ticks as usize);
    let mut copies = Vec::with_capacity((total_ticks / reset_every) as usize);
    for index in 0..total_ticks {
        if index % reset_every == 0 {
            let start = Instant::now();
            black_box(&mut frame).copy_from(black_box(&snapshot));
            copies.push((index, start.elapsed().as_nanos()));
        }
        records.push(measured_tick(&mut frame, &mut scratch));
    }

    // Replay every measured tick with the ordinary step. Fresh Scratch matches
    // the measured sequence, and each reset uses exactly the benchmark cadence.
    // This is an independent result check, not a second timing benchmark.
    let mut plain = snapshot.clone();
    let mut plain_scratch = Scratch::new();
    for (index, record) in records.iter().enumerate() {
        if index % reset_every as usize == 0 {
            plain.copy_from(&snapshot);
        }
        tick(&mut plain, &mut plain_scratch);
        assert_eq!(record.tick, plain.tick(), "{name}: tick {index}");
        assert_eq!(
            record.checksum,
            plain.checksum(),
            "{name}: probed/unprobed checksum at {index}"
        );
        assert_eq!(
            record.stats,
            plain_scratch.stats(),
            "{name}: probed/unprobed stats at {index}"
        );
        let first = &records[index % reset_every as usize];
        assert_eq!(
            record.checksum, first.checksum,
            "{name}: reset checksum at {index}"
        );
        assert_eq!(record.stats, first.stats, "{name}: reset stats at {index}");
        assert_eq!(
            record.phases.calls, first.phases.calls,
            "{name}: reset phase counts at {index}"
        );
        assert_eq!(
            record.tick,
            snapshot.tick() + (index % reset_every as usize) as u64 + 1
        );
    }
    let cfg = snapshot.singleton::<PhysicsState>().config;
    println!(
        "{{\"kind\":\"case\",\"name\":\"{name}\",\"warmup_ticks\":{},\"snapshot_checksum\":\"0x{:016x}\",\"repetitions\":{repetitions},\"ticks_per_repetition\":{ticks_per_repetition},\"instrumented_ticks\":{total_ticks},\"reset_every\":{reset_every},\"copy_count\":{},\"substeps\":{},\"velocity_iterations\":{},\"sleep_ticks\":{}}}",
        snapshot.tick(), snapshot.checksum(), copies.len(), cfg.substeps, cfg.velocity_iterations, cfg.sleep_ticks
    );
    for (before_index, ns) in copies {
        println!(
            "{{\"kind\":\"copy\",\"case\":\"{name}\",\"before_index\":{before_index},\"ns\":{ns}}}"
        );
    }
    for (index, record) in records.iter().enumerate() {
        let tail_ns = record.total_ns - record.phases.ns.iter().sum::<u128>();
        println!(
            "{{\"kind\":\"tick\",\"case\":\"{name}\",\"index\":{index},\"tick\":{},\"checksum\":\"0x{:016x}\",\"stats\":{:?},\"phase_ns\":{:?},\"phase_calls\":{:?},\"total_ns\":{},\"tail_ns\":{tail_ns}}}",
            record.tick, record.checksum, stats_values(record.stats), record.phases.ns, record.phases.calls, record.total_ns
        );
    }
    println!("{{\"kind\":\"validation\",\"case\":\"{name}\",\"probed_unprobed_equal_ticks\":{total_ticks},\"reset_replay_equal_ticks\":{total_ticks}}}");
}

fn accumulator_self_check() {
    // Synthetic markers test repeated phases and the legal early-return shape
    // without adding any instrumented physics ticks to the fixed workload.
    let mut repeated = PhaseTimes::default();
    for phase in [
        Phase::Gather,
        Phase::Transforms,
        Phase::Broad,
        Phase::Narrow,
        Phase::Broad,
        Phase::Narrow,
        Phase::Integrate,
        Phase::Prepare,
        Phase::Solve,
        Phase::Sleep,
        Phase::Finish,
    ] {
        repeated.record(phase, 7);
    }
    repeated.check(77);
    assert_eq!(repeated.ns, [7, 7, 14, 14, 7, 7, 7, 7, 7]);
    assert_eq!(repeated.calls, [1, 1, 2, 2, 1, 1, 1, 1, 1]);
    let mut early = PhaseTimes::default();
    early.record(Phase::Gather, 3);
    early.record(Phase::Finish, 5);
    early.check(8);
    assert_eq!(early.ns, [3, 0, 0, 0, 0, 0, 0, 0, 5]);
    assert_eq!(early.calls, [1, 0, 0, 0, 0, 0, 0, 0, 1]);
}

fn clock_calibration() {
    // Empty clock brackets are a timer-resolution/overhead diagnostic only.
    // They do not reproduce phase callbacks, and are never subtracted from data.
    let mut samples = Vec::with_capacity(1000);
    for _ in 0..1000 {
        let start = Instant::now();
        black_box(());
        samples.push(start.elapsed().as_nanos());
    }
    println!(
        "{{\"kind\":\"clock_calibration\",\"empty_bracket_ns\":{samples:?},\"subtracted\":false}}"
    );
}

fn main() {
    assert_eq!(
        std::env::args_os().len(),
        1,
        "phase_costs takes no arguments"
    );
    accumulator_self_check();
    println!("{{\"kind\":\"schema\",\"version\":1,\"units\":\"nanoseconds\",\"phases\":[\"gather\",\"transforms\",\"broad\",\"narrow\",\"integrate\",\"prepare\",\"solve\",\"sleep\",\"finish\"],\"stats\":[\"bodies\",\"awake\",\"asleep\",\"pairs\",\"manifolds\",\"points\"]}}");
    clock_calibration();
    for n in [500, 1000] {
        run_case(
            &format!("mixed_settling_{n}"),
            warmed(bench_pile(n, false), 90),
            200,
            1,
            100,
        );
    }
    for n in [500, 1000] {
        run_case(
            &format!("mixed_settled_{n}"),
            warmed(bench_pile(n, false), 600),
            200,
            1,
            100,
        );
    }
    for n in [500, 1000] {
        run_case(
            &format!("field_sleeping_{n}"),
            warmed(bench_field(n, true), 900),
            200,
            1,
            100,
        );
    }
    run_case(
        "rollback8_mixed_settling_1000",
        warmed(bench_pile(1000, false), 90),
        20,
        8,
        8,
    );
    run_case(
        "rollback8_field_sleeping_1000",
        warmed(bench_field(1000, true), 900),
        20,
        8,
        8,
    );
    println!("{{\"kind\":\"complete\",\"cases\":8,\"instrumented_ticks\":1520,\"unprobed_validation_ticks\":1520,\"accumulator_self_check\":true}}");
}
