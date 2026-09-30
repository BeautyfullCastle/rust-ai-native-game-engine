//! Headless end-to-end runs of the physics sample: the game through the
//! bridge with a latency loopback (so rollbacks happen with physics in the
//! resimulation), checked against each other and against one plain sim.
//! No window, no GPU.
#![allow(clippy::float_arithmetic)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use orr_bridge::{Bridge, BridgeConfig, InProc, Pacing, PlayerSlot, Threaded, ThreadedConfig};
use orr_ecs::Entity;
use orr_sample::arena_view::Loopback;
use orr_sample::physics_app::{run_headless, HeadlessLimit};
use orr_sample::physics_game::{dynamic_count, NoCommand, PhysConfig, PhysGame, PhysInput, SceneMode};
use orr_sample::physics_host::{phys_bot, physics_pair, scripted_local, session_configs, SimMetrics, TimedPair};
use orr_sample::physics_view::PhysExtractor;
use orr_session::compare_checksums;
use orr_sim::{Simulation, TickInputs};
use orr_view::{Extractor, InterpMode, RenderItem, ViewConfig, ViewWorld};

const NET: Loopback = Loopback { latency_ticks: 6, jitter_ticks: 2 };
const INPUT_DELAY: u64 = 2;

fn scene() -> PhysConfig {
    // Small, with spawning on so the entity set changes during the run.
    let mut scene = PhysConfig::new(120, SceneMode::Rain);
    scene.spawn_rate = 30;
    scene
}

type InputLog = Arc<Mutex<BTreeMap<u64, PhysInput>>>;

/// The physics session; the bot writes its inputs (by the tick they are stamped for) to `bot_log`.
fn pair(scene: PhysConfig, bot_log: InputLog) -> TimedPair<PhysGame> {
    let (cfg_a, cfg_b) = session_configs();
    let mut bot = phys_bot(1234);
    TimedPair::new(
        move || scene,
        cfg_a,
        cfg_b,
        NET,
        777,
        move |tick| {
            let (input, commands) = bot(tick);
            bot_log.lock().unwrap().insert(tick, input);
            (input, commands)
        },
        SimMetrics::new(),
    )
}

/// A plain sim fed the inputs both peers used, stepped to `tick`.
fn reference(scene: PhysConfig, local: &BTreeMap<u64, PhysInput>, bot: &InputLog, tick: u64) -> Simulation<PhysGame> {
    let (cfg, _) = session_configs();
    let mut sim = Simulation::<PhysGame>::new(scene, cfg.tick_rate, cfg.seed);
    let bot = bot.lock().unwrap();
    for t in 1..=tick {
        let mut inputs = TickInputs::<PhysInput, NoCommand>::new(t, 2);
        inputs.set_input(PlayerSlot(0), local.get(&t).copied().unwrap_or_default());
        inputs.set_input(PlayerSlot(1), bot.get(&t).copied().unwrap_or_default());
        sim.step(&inputs);
    }
    sim
}

#[test]
fn two_peers_with_rollbacks_match_each_other_and_a_plain_sim() {
    const CALLS: u64 = 300;
    let scene = scene();
    let bot_log = InputLog::default();
    let mut bridge = InProc::new(pair(scene, bot_log.clone()), BridgeConfig::default());
    let mut local: BTreeMap<u64, PhysInput> = BTreeMap::new();
    for call in 1..=CALLS {
        // The session stamps the input of call `n` for tick `n + input_delay`.
        let input = scripted_local(call);
        local.insert(call + INPUT_DELAY, input);
        bridge.set_input(PlayerSlot(0), input).unwrap();
        bridge.step(1);
    }

    let (a, b) = (bridge.host().peer_a(), bridge.host().peer_b());
    assert!(a.rollback_count() > 0, "latency 6 with input delay 2 must cause rollbacks");
    assert!(!a.checksums().is_empty() && !b.checksums().is_empty(), "no checkpoints recorded");
    let desyncs = compare_checksums(a.checksums(), b.checksums());
    assert!(desyncs.is_empty(), "peers disagree: {desyncs:?}");

    let &(tick, checksum) = a.checksums().last().unwrap();
    let mut sim = reference(scene, &local, &bot_log, tick);
    assert_eq!(sim.checksum(), checksum, "peer A's verified state differs from a plain sim at tick {tick}");
    assert!(dynamic_count(sim.frame_mut()) > 120, "spawning added bodies");
}

#[test]
fn threaded_bridge_gives_the_same_verified_state() {
    const CALLS: u64 = 240;
    let scene = scene();
    let bot_log = InputLog::default();
    let factory_log = bot_log.clone();
    let mut bridge = Threaded::spawn(
        move || pair(scene, factory_log),
        BridgeConfig::default(),
        ThreadedConfig { pacing: Pacing::Manual, max_catchup: 8 },
    )
    .unwrap();
    let mut local: BTreeMap<u64, PhysInput> = BTreeMap::new();
    for call in 1..=CALLS {
        let input = scripted_local(call);
        local.insert(call + INPUT_DELAY, input);
        bridge.set_input(PlayerSlot(0), input).unwrap();
        bridge.step(1);
    }
    let snap = bridge.snapshot().unwrap();
    assert!(snap.stats().rollbacks > 0);
    let verified = snap.verified().unwrap();
    let mut sim = reference(scene, &local, &bot_log, snap.verified_tick());
    assert_eq!(verified.checksum(), sim.checksum(), "verified frame at tick {} differs from a plain sim", snap.verified_tick());
    drop(bridge);
    let _ = dynamic_count(sim.frame_mut());
}

#[test]
fn headless_benchmark_runs_and_reports() {
    let summary = run_headless(scene(), NET, HeadlessLimit::Steps(200));
    assert!(summary.sim_tick >= 190, "sim reached tick {}", summary.sim_tick);
    assert!(summary.rollbacks > 0);
    assert!(summary.sim.tick.count > 0 && summary.sim.rollback.count > 0);
    assert!(summary.sim.rollback.max_ms >= summary.sim.rollback.avg_ms());
    assert_eq!(summary.sim.max_resim_ticks, summary.max_rollback_depth);
    assert!(summary.sim.span_secs > 0.0);
}

#[test]
fn extractor_reads_every_body_with_its_shape() {
    let scene = scene();
    let mut bridge = InProc::new(physics_pair(scene, NET, SimMetrics::new()), BridgeConfig::default());
    bridge.step(5);
    let snap = bridge.snapshot().unwrap();
    let mut out = Vec::new();
    PhysExtractor { remote_mode: InterpMode::Prediction, local_slot: 0 }.extract(snap.predicted(), &mut out);
    // Every entity is a body: walls (3), obstacles, paddles (2) and the dynamic ones.
    assert_eq!(out.len() as u32, snap.predicted().alive_count());
    let boxes = out.iter().filter(|e| e.style.half_y > 0.0).count();
    let circles = out.iter().filter(|e| e.style.half_y == 0.0).count();
    assert!(boxes >= 5 && circles > 10, "boxes {boxes}, circles {circles}");
    assert!(out.iter().all(|e| e.transform.pos.x.is_finite() && e.style.size > 0.0));
    let statics = out.iter().filter(|e| e.mode == InterpMode::None).count();
    assert!(statics >= 3 + 4, "walls and obstacles are drawn without interpolation, got {statics}");
}

/// Renders `ticks` ticks at 3 frames per tick and returns the largest per-frame
/// step of any dynamic body that existed in both frames, and the rollback count.
fn worst_step(cfg: ViewConfig, ticks: u32) -> (f32, u32, u64) {
    const FRAMES_PER_TICK: u32 = 3;
    let scene = PhysConfig::new(200, SceneMode::Mixer);
    let mut bridge = InProc::new(physics_pair(scene, NET, SimMetrics::new()), BridgeConfig::default());
    let mut view = ViewWorld::new(PhysExtractor { remote_mode: InterpMode::Prediction, local_slot: 0 }, cfg);
    let dt = Duration::from_secs_f64(1.0 / 60.0 / f64::from(FRAMES_PER_TICK));
    let mut last: BTreeMap<Entity, (f32, f32)> = BTreeMap::new();
    let mut worst = 0.0_f32;
    let mut pops = 0u32;
    let mut items: Vec<RenderItem> = Vec::new();
    for tick in 0..ticks {
        bridge.set_input(PlayerSlot(0), scripted_local(u64::from(tick))).unwrap();
        for f in 0..FRAMES_PER_TICK {
            bridge.update(if f == 0 { Duration::from_nanos(16_666_667) } else { Duration::ZERO });
            let snap = bridge.snapshot();
            view.update(dt.as_secs_f32(), snap.as_ref());
            items.clear();
            view.render_items(&mut items);
            let mut now = BTreeMap::new();
            for item in items.iter().filter(|i| !i.entity.is_none() && i.style.half_y < 1.0) {
                let pos = (item.transform.pos.x, item.transform.pos.y);
                if let Some(prev) = last.get(&item.entity) {
                    if tick > 60 {
                        let step = (pos.0 - prev.0).hypot(pos.1 - prev.1);
                        // A step above 5 is a recycled body (sim teleport), not a correction.
                        if step < 5.0 {
                            worst = worst.max(step);
                            if step > 0.5 {
                                pops += 1;
                            }
                        }
                    }
                }
                now.insert(item.entity, pos);
            }
            last = now;
        }
    }
    (worst, pops, bridge.snapshot().unwrap().stats().rollbacks)
}

#[test]
fn rollbacks_do_not_pop_the_physics_view() {
    let (smooth, smooth_pops, rollbacks) = worst_step(ViewConfig::default(), 600);
    let (raw, raw_pops, _) = worst_step(ViewConfig { correction_tau: 0.0, ..ViewConfig::default() }, 600);
    eprintln!(
        "per-frame body step: smoothed worst {smooth:.2} ({smooth_pops} over 0.5), unsmoothed worst {raw:.2} ({raw_pops} over 0.5), rollbacks {rollbacks}"
    );
    assert!(rollbacks > 10, "expected many rollbacks, got {rollbacks}");
    // A body moves at most 40 units/s = 0.67 per tick = 0.22 per frame (a paddle 0.07 per frame).
    assert!(smooth < 1.0, "smoothed view popped by {smooth}");
    assert!(raw > 2.0 && raw_pops > smooth_pops + 10, "control run should pop: raw {raw} ({raw_pops}), smoothed {smooth} ({smooth_pops})");
}
