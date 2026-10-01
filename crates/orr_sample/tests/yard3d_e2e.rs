//! Headless end-to-end runs of the 3D sample: the Yard3D game through the
//! bridge with a latency loopback (so rollbacks happen with 3D physics in the
//! resimulation), checked against a plain sim; the 3D view and the 3D view
//! stream on top. No window, no GPU.
#![allow(clippy::float_arithmetic)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use orr_bridge::{Bridge, BridgeConfig, InProc, PlayerSlot};
use orr_ecs::Entity;
use orr_sample::arena_view::Loopback;
use orr_sample::physics_host::{SimMetrics, TimedPair};
use orr_sample::yard3d_app::{run_headless, HeadlessLimit};
use orr_sample::yard3d_game::{dynamic_count, NoCommand, Yard3D, YardConfig, YardInput};
use orr_sample::yard3d_host::{scripted_local, session_configs, yard_bot, yard_pair};
use orr_sample::yard3d_view::{fill_list, yard_view_config, YardExtractor};
use orr_session::compare_checksums;
use orr_sim::{Simulation, TickInputs};
use orr_view::{Extracted3, Extractor3, InterpMode, RenderItem3, Shape3, ViewConfig, ViewWorld3};
use orr_viewstream::{
    ViewFrame3, ViewStreamSource3, FLAG_ROLLED_BACK, MSG_FRAME3D, SHAPE3_BOX, SHAPE3_CAPSULE, SHAPE3_PLANE, SHAPE3_SPHERE, STYLE_CHECKER,
};

const NET: Loopback = Loopback { latency_ticks: 6, jitter_ticks: 2 };
const INPUT_DELAY: u64 = 2;

fn scene() -> YardConfig {
    // Small, with rain on so the entity set changes during the run.
    YardConfig { rain_per_second: 20, ..YardConfig::new(60) }
}

type InputLog = Arc<Mutex<BTreeMap<u64, YardInput>>>;

fn pair(scene: YardConfig, bot_log: InputLog) -> TimedPair<Yard3D> {
    let (cfg_a, cfg_b) = session_configs();
    let mut bot = yard_bot(1234);
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

fn reference(scene: YardConfig, local: &BTreeMap<u64, YardInput>, bot: &InputLog, tick: u64) -> Simulation<Yard3D> {
    let (cfg, _) = session_configs();
    let mut sim = Simulation::<Yard3D>::new(scene, cfg.tick_rate, cfg.seed);
    let bot = bot.lock().unwrap();
    for t in 1..=tick {
        let mut inputs = TickInputs::<YardInput, NoCommand>::new(t, 2);
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
    let mut local: BTreeMap<u64, YardInput> = BTreeMap::new();
    for call in 1..=CALLS {
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
    assert!(dynamic_count(sim.frame_mut()) > 60, "rain and shots added bodies");
}

#[test]
fn headless_benchmark_runs_the_view_too() {
    let summary = run_headless(scene(), NET, HeadlessLimit::Steps(200));
    assert!(summary.sim_tick >= 190, "sim reached tick {}", summary.sim_tick);
    assert!(summary.rollbacks > 0);
    let r = summary.render.expect("headless runs also time the view side");
    assert!(r.instances > 70 && r.frames >= 200, "{r:?}");
    assert!(r.view_update_ms.0 > 0.0 && r.list_ms.0 > 0.0);
}

#[test]
fn extractor_reads_every_body_with_its_shape_and_a_unit_quaternion() {
    let mut bridge = InProc::new(yard_pair(scene(), NET, SimMetrics::new()), BridgeConfig::default());
    bridge.step(5);
    let snap = bridge.snapshot().unwrap();
    let mut out: Vec<Extracted3> = Vec::new();
    YardExtractor.extract(snap.predicted(), &mut out);
    assert_eq!(out.len() as u32, snap.predicted().alive_count());
    let count = |f: fn(&Shape3) -> bool| out.iter().filter(|e| f(&e.style.shape)).count();
    assert!(count(|s| matches!(s, Shape3::Sphere { .. })) > 5, "spheres");
    assert!(count(|s| matches!(s, Shape3::Capsule { .. })) > 5, "capsules");
    assert!(count(|s| matches!(s, Shape3::Box { .. })) > 30, "boxes");
    assert_eq!(count(|s| matches!(s, Shape3::Plane { .. })), 1, "one floor plane");
    let floor = out.iter().find(|e| matches!(e.style.shape, Shape3::Plane { .. })).unwrap();
    assert!(floor.style.checker && floor.mode == InterpMode::None);
    assert!(floor.transform.pos.y.abs() < 1e-3, "the plane sits on the floor's top face, y = {}", floor.transform.pos.y);
    for e in &out {
        let q = e.transform.rot;
        assert!((q.dot(q) - 1.0).abs() < 1e-4 && e.transform.pos.x.is_finite());
    }
    // Every entity ends up as an instance.
    let items: Vec<RenderItem3> = out.iter().map(|e| RenderItem3 { entity: e.entity, transform: e.transform, style: e.style }).collect();
    let mut list = orr_render::RenderList3D::new();
    fill_list(&items, &mut list);
    assert_eq!(list.instance_count(), out.len());
}

/// Renders `ticks` ticks at 3 frames per tick and returns, over dynamic bodies that existed in
/// both frames, the number of per-frame position steps above 0.2 and rotation steps above 0.25 rad.
fn pops(cfg: ViewConfig, ticks: u32) -> (u32, u32, u64) {
    const FRAMES_PER_TICK: u32 = 3;
    let mut bridge = InProc::new(yard_pair(scene(), NET, SimMetrics::new()), BridgeConfig::default());
    let mut view = ViewWorld3::new(YardExtractor, cfg);
    let dt = Duration::from_secs_f64(1.0 / 60.0 / f64::from(FRAMES_PER_TICK));
    let mut last: BTreeMap<Entity, (orr_view::Vec3, orr_view::Quat)> = BTreeMap::new();
    let (mut pos_pops, mut rot_pops) = (0u32, 0u32);
    let mut items: Vec<RenderItem3> = Vec::new();
    for tick in 0..ticks {
        bridge.set_input(PlayerSlot(0), scripted_local(u64::from(tick))).unwrap();
        for f in 0..FRAMES_PER_TICK {
            bridge.update(if f == 0 { Duration::from_nanos(16_666_667) } else { Duration::ZERO });
            let snap = bridge.snapshot();
            view.update(dt.as_secs_f32(), snap.as_ref());
            items.clear();
            view.render_items(&mut items);
            let mut now = BTreeMap::new();
            for item in items.iter().filter(|i| !i.entity.is_none() && !matches!(i.style.shape, Shape3::Plane { .. })) {
                let t = item.transform;
                if let (Some((p, q)), true) = (last.get(&item.entity), tick > 60) {
                    let step = (t.pos - *p).length();
                    // A step above 5 is a recycled body (sim teleport), not a correction.
                    if step < 5.0 && step > 0.2 {
                        pos_pops += 1;
                    }
                    if t.rot.angle_to(*q) > 0.25 {
                        rot_pops += 1;
                    }
                }
                now.insert(item.entity, (t.pos, t.rot));
            }
            last = now;
        }
    }
    (pos_pops, rot_pops, bridge.snapshot().unwrap().stats().rollbacks)
}

#[test]
fn rollbacks_do_not_pop_the_3d_view() {
    // The scene's rollbacks move most bodies by very little (the synthetic tests in
    // `orr_view/tests/view3.rs` pin the smoothing itself); here the real 3D game must go through
    // many rollbacks with the view on top without jumps, and smoothing must never add any.
    let (smooth_pos, smooth_rot, rollbacks) = pops(yard_view_config(), 400);
    let (raw_pos, raw_rot, _) = pops(ViewConfig { correction_tau: 0.0, ..yard_view_config() }, 400);
    eprintln!("pops (position over 0.2, rotation over 0.25 rad): smoothed {smooth_pos}/{smooth_rot}, unsmoothed {raw_pos}/{raw_rot}, rollbacks {rollbacks}");
    assert!(rollbacks > 5, "the run must roll back");
    assert_eq!(smooth_pos, 0, "no position pops with smoothing");
    assert!(smooth_rot <= raw_rot + raw_rot / 10 + 2, "smoothing does not add rotation pops: {smooth_rot} vs {raw_rot}");
}

#[test]
fn the_3d_view_stream_carries_poses_shapes_and_rollbacks() {
    let metrics = SimMetrics::new();
    let mut bridge = InProc::new(yard_pair(scene(), NET, metrics.clone()), BridgeConfig::default());
    let schema = orr_viewstream::Schema {
        game: "Yard3D".into(),
        dimensions: 3,
        build_id: 0,
        tick_rate: 60,
        player_count: 2,
        kinds: vec![orr_viewstream::KindDef::new(0, "body")],
        input: orr_viewstream::InputLayout { size: std::mem::size_of::<YardInput>(), fields: vec![] },
        command_size: 0,
        events: vec![],
    };
    let mut source = ViewStreamSource3::new(YardExtractor, orr_viewstream::NoKinds3, schema);
    let (mut frames, mut rolled, mut last_seq) = (Vec::new(), 0u32, 0u64);
    for call in 1..=200u64 {
        bridge.set_input(PlayerSlot(0), scripted_local(call)).unwrap();
        bridge.step(1);
        let pumped = source.pump(&mut bridge);
        if let Some(bytes) = pumped.frame.map(|f| f.encode()) {
            let f = ViewFrame3::decode(&bytes).unwrap();
            assert_eq!(orr_viewstream::message_type(&bytes), Ok(MSG_FRAME3D));
            assert!(f.seq > last_seq, "seq counts up");
            last_seq = f.seq;
            if f.has(FLAG_ROLLED_BACK) {
                rolled += 1;
                let (from, to) = f.rollback.expect("a rollback frame carries its range");
                assert!(from <= to && to <= f.tick, "{from}..{to} at tick {}", f.tick);
            }
            frames.push(f);
        }
    }
    assert!(frames.len() > 150 && rolled > 0, "frames {} rolled back {rolled}", frames.len());
    let f = frames.last().unwrap();
    assert!(f.entities.len() > 70);
    let mut shapes = [0u32; 4];
    for e in &f.entities {
        shapes[usize::from(e.shape)] += 1;
        for pose in [e.prev, e.cur] {
            let q = pose.rot;
            let n = q.iter().map(|c| c * c).sum::<f32>();
            assert!((n - 1.0).abs() < 1e-4, "unit quaternion, got {n}");
            assert!(pose.pos.iter().all(|c| c.is_finite()));
        }
        assert!(e.size[0] > 0.0);
    }
    assert!(shapes[usize::from(SHAPE3_SPHERE)] > 5 && shapes[usize::from(SHAPE3_BOX)] > 30 && shapes[usize::from(SHAPE3_CAPSULE)] > 5);
    assert_eq!(shapes[usize::from(SHAPE3_PLANE)], 1);
    let plane = f.entities.iter().find(|e| e.shape == SHAPE3_PLANE).unwrap();
    assert!(plane.style_flags & STYLE_CHECKER != 0 && plane.size[0] > 20.0 && plane.size[2] > 20.0);
    // A body that moved has a previous pose that differs from the current one.
    assert!(f.entities.iter().any(|e| e.prev.pos != e.cur.pos), "dynamic bodies move between ticks");
    // The bytes are exactly 56 + 88 per entity.
    assert_eq!(f.encode().len(), 56 + 88 * f.entities.len());
}

#[test]
fn the_bot_is_a_pure_function_of_its_seed_and_tick() {
    let run = |seed| {
        let mut bot = yard_bot(seed);
        (0..300u64).map(|t| bot(t).0).collect::<Vec<_>>()
    };
    assert_eq!(run(5), run(5));
    assert_ne!(run(5), run(6));
    assert!(run(5).iter().any(|i| i.buttons != 0), "the bot shoots");
}

/// A plain sim stepped `ticks` ticks with `input` held by player 0.
fn run_with_input(input: YardInput, ticks: u64) -> Simulation<Yard3D> {
    let (cfg, _) = session_configs();
    let scene = YardConfig { rain_per_second: 0, ..YardConfig::new(0) };
    let mut sim = Simulation::<Yard3D>::new(scene, cfg.tick_rate, cfg.seed);
    for t in 1..=ticks {
        let mut inputs = TickInputs::<YardInput, NoCommand>::new(t, 2);
        inputs.set_input(PlayerSlot(0), input);
        sim.step(&inputs);
    }
    sim
}

#[test]
fn a_shot_flies_along_the_camera_ray_and_a_spawn_lands_under_the_cursor() {
    use orr_physics3d::{Body, BODY_DYNAMIC};
    // The ray from (0, 10, 20) toward the origin: direction (0, -0.447, -0.894).
    let ray = YardInput { buttons: orr_sample::yard3d_game::SHOOT, _pad: 0, origin: [0, 1000, 2000], dir: [0, -447, -894] };
    let shot = run_with_input(ray, 8);
    // The scene's crates rest on the floor; shots are the bodies that fly.
    let moving: Vec<Body> =
        shot.frame().dense::<Body>().1.iter().filter(|b| b.kind == BODY_DYNAMIC && b.vel.length_sq().to_f32() > 25.0).copied().collect();
    assert!(moving.len() >= 2, "holding the button shoots more than once, got {}", moving.len());
    let b = moving[0];
    assert!(b.vel.z.to_f32() < -15.0 && b.vel.y.to_f32() < 0.0 && b.vel.x.to_f32().abs() < 0.5, "{:?}", b.vel);
    assert!(b.pos.z.to_f32() < 20.0, "it left the muzzle toward -z");

    // Dropping a box: the ray from (10, 10, 0) along (-0.5, -0.5, 0) meets the floor at x = 0.
    let drop = YardInput { buttons: orr_sample::yard3d_game::SPAWN_BOX, _pad: 0, origin: [1000, 1000, 0], dir: [-500, -500, 0] };
    let sim = run_with_input(drop, 5);
    let bodies: Vec<Body> = sim.frame().dense::<Body>().1.iter().filter(|b| b.kind == BODY_DYNAMIC && b.pos.y.to_f32() > 3.0 && b.pos.x.to_f32().abs() < 3.0).copied().collect();
    assert!(!bodies.is_empty(), "a box was spawned");
    let p = bodies[0].pos;
    // The ray meets y = 0 at x = 0, z = 0: the box drops from 8 units above that point.
    assert!(p.x.to_f32().abs() < 0.6 && p.z.to_f32().abs() < 0.6 && p.y.to_f32() > 6.0 && p.y.to_f32() < 8.5, "{p:?}");

    // Inputs outside a sane range (a hostile peer) are clamped, not trusted: nothing breaks.
    let wild = YardInput { buttons: u32::MAX, _pad: 0, origin: [i32::MAX, i32::MIN, 7], dir: [i32::MAX, i32::MIN, 0] };
    let sim = run_with_input(wild, 30);
    assert!(sim.frame().dense::<Body>().1.iter().all(|b| b.pos.x.to_f32().abs() < 30_000.0));
}
