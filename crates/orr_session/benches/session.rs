//! Bench: per-tick `Simulation::step` cost at 1k/10k entities, and the cost
//! of resimulating 8 ticks (the design doc's default `max_prediction`) at
//! 10k entities — the number that actually matters for rollback netcode,
//! since it's `tick cost * rollback depth` that has to fit inside a render
//! frame budget.
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use orr_fp::{FPVec2, FP};
use orr_sim::{PlayerSlot, Simulation, TickInputs};
use orr_testgame::{Arena, ArenaConfig, ArenaInput, Bullet, Position};

/// Spawns `extra` additional non-player entities (moving bullets) directly
/// into the simulation's frame, so `step`'s per-tick cost reflects a
/// realistic entity count rather than just the 2 players the arena game
/// itself spawns.
fn pad_entities(sim: &mut Simulation<Arena>, extra: u32) {
    let frame = sim.frame_mut();
    for i in 0..extra {
        let e = frame.spawn();
        let x = FP::from_int((i % 4000) as i32 - 2000);
        // Both arena players spawn at y=0; keep padding bullets well clear
        // of the hit-radius band around y=0 so they never trigger a `Hit`
        // (which would index `Score::kills` by `owner_slot`, and these are
        // synthetic load, not a real player).
        let pos = FPVec2::new(x, FP::from_int(1500));
        frame.add(e, Position { pos });
        frame.add(e, Bullet { velocity: FPVec2::new(FP::from_int(1), FP::ZERO), owner_slot: 0, ttl: u32::MAX });
    }
}

fn trivial_inputs(tick: u64) -> TickInputs<ArenaInput, orr_testgame::SpawnBulletCmd> {
    let mut ti = TickInputs::new(tick, 2);
    ti.set_input(PlayerSlot(0), ArenaInput::default());
    ti.set_input(PlayerSlot(1), ArenaInput::default());
    ti
}

fn bench_step(c: &mut Criterion, entity_count: u32, label: &str) {
    c.bench_function(label, |b| {
        b.iter_batched(
            || {
                let mut sim = Simulation::<Arena>::new(ArenaConfig { player_count: 2 }, 60, 1);
                pad_entities(&mut sim, entity_count - 2);
                sim
            },
            |mut sim| {
                let tick = sim.tick() + 1;
                sim.step(&trivial_inputs(tick));
                sim
            },
            BatchSize::LargeInput,
        );
    });
}

fn bench_resim_8_ticks_10k(c: &mut Criterion) {
    c.bench_function("resim_8_ticks_10k_entities", |b| {
        b.iter_batched(
            || {
                let mut sim = Simulation::<Arena>::new(ArenaConfig { player_count: 2 }, 60, 1);
                pad_entities(&mut sim, 10_000 - 2);
                // Warm up: step once so the snapshot isn't the pristine
                // initial frame (closer to a mid-session rollback target).
                sim.step(&trivial_inputs(1));
                let snapshot = sim.frame().clone();
                (sim, snapshot)
            },
            |(mut sim, snapshot)| {
                sim.restore(&snapshot);
                for _ in 0..8 {
                    let tick = sim.tick() + 1;
                    sim.step(&trivial_inputs(tick));
                }
                sim
            },
            BatchSize::LargeInput,
        );
    });
}

fn benches(c: &mut Criterion) {
    bench_step(c, 1_000, "step_1k_entities");
    bench_step(c, 10_000, "step_10k_entities");
    bench_resim_8_ticks_10k(c);
}

criterion_group!(session_benches, benches);
criterion_main!(session_benches);
