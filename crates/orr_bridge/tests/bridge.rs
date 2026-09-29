//! Bridge tests with the arena game: the two adapters publish the same
//! snapshots and events for the same inputs; the view side cannot change sim
//! state; events keep their 3-state meaning.
#![allow(clippy::disallowed_types)] // tests time the real thread
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use orr_bridge::{
    Bridge, BridgeConfig, BridgeError, BridgeEvent, EventKey, EventStatus, InProc, Lifecycle, LoopbackPair, Pacing, PlayerSlot,
    Snapshot, Threaded, ThreadedConfig,
};
use orr_fp::{FrameRng, FP};
use orr_session::SessionConfig;
use orr_testgame::{Arena, ArenaConfig, ArenaInput, Hit, Position, SpawnBulletCmd, FIRE};

const ONE_TICK: Duration = Duration::from_nanos(16_666_667);

fn scripted(rng: &mut FrameRng, slot: u8, tick: u64) -> ArenaInput {
    let ax = rng.range_i32(-1, 2);
    let ay = rng.range_i32(-1, 2);
    let fire = rng.next_u32() % 5 == 0 && tick % 3 == (slot as u64 % 3);
    ArenaInput::new(FP::from_int(ax), FP::from_int(ay), fire)
}

fn fire_cmd(owner: u32, input: &ArenaInput) -> Vec<SpawnBulletCmd> {
    if input.buttons & FIRE != 0 {
        vec![SpawnBulletCmd { owner }]
    } else {
        Vec::new()
    }
}

fn make_pair() -> LoopbackPair<Arena> {
    let mut rng_b = FrameRng::new(42 ^ 0xABCD);
    LoopbackPair::new(
        || ArenaConfig { player_count: 2 },
        SessionConfig::new(2, PlayerSlot(0), 42, 60),
        SessionConfig::new(2, PlayerSlot(1), 42, 60),
        4,
        1,
        777,
        move |tick| {
            let input = scripted(&mut rng_b, 1, tick);
            let cmds = fire_cmd(1, &input);
            (input, cmds)
        },
    )
}

fn bridge_config() -> BridgeConfig<Arena> {
    BridgeConfig::default().with_commands_from_input(|input| fire_cmd(0, input))
}

/// What one update showed the view.
#[derive(Debug, PartialEq)]
struct Frame1 {
    seq_advanced: bool,
    tick: u64,
    verified_tick: u64,
    predicted: u64,
    prev: Option<u64>,
    verified: Option<u64>,
    rollbacks: u64,
}

#[derive(Debug, PartialEq)]
struct Trace {
    frames: Vec<Frame1>,
    events: Vec<BridgeEvent<Hit>>,
}

fn describe(snap: &Snapshot, last_seq: &mut u64) -> Frame1 {
    let advanced = snap.seq() > *last_seq;
    *last_seq = snap.seq();
    Frame1 {
        seq_advanced: advanced,
        tick: snap.tick(),
        verified_tick: snap.verified_tick(),
        predicted: snap.predicted().checksum(),
        prev: snap.predicted_prev().map(|f| f.checksum()),
        verified: snap.verified().map(|f| f.checksum()),
        rollbacks: snap.stats().rollbacks,
    }
}

/// Drives any adapter through the same `Bridge` API.
fn drive<B: Bridge<Arena>>(bridge: &mut B, ticks: u64) -> Trace {
    let mut rng = FrameRng::new(42);
    let mut trace = Trace { frames: Vec::new(), events: Vec::new() };
    let mut last_seq = 0;
    for t in 1..=ticks {
        let input = scripted(&mut rng, 0, t + 2);
        bridge.set_input(PlayerSlot(0), input).unwrap();
        bridge.update(ONE_TICK);
        let snap = bridge.snapshot().expect("a snapshot exists after the first tick");
        trace.frames.push(describe(&snap, &mut last_seq));
        trace.events.extend(bridge.drain_events());
    }
    trace
}

#[test]
fn inproc_and_threaded_publish_the_same_snapshots_and_events() {
    const TICKS: u64 = 600;
    let mut inproc = InProc::new(make_pair(), bridge_config());
    let a = drive(&mut inproc, TICKS);

    let mut threaded = Threaded::spawn(make_pair, bridge_config(), ThreadedConfig { pacing: Pacing::Manual, max_catchup: 8 }).unwrap();
    let b = drive(&mut threaded, TICKS);

    assert_eq!(a.frames.len(), b.frames.len());
    for (i, (fa, fb)) in a.frames.iter().zip(&b.frames).enumerate() {
        assert_eq!(fa, fb, "snapshot differs after update {i}");
    }
    assert_eq!(a.events, b.events, "event streams differ");

    // The run really exercised rollbacks, events and lifecycle.
    let rollbacks = a.frames.last().unwrap().rollbacks;
    assert!(rollbacks > 0, "latency 4 > input delay 2 must roll back");
    let lifecycle_rollbacks =
        a.events.iter().filter(|e| matches!(e, BridgeEvent::Lifecycle(Lifecycle::Rollback(_)))).count() as u64;
    assert_eq!(lifecycle_rollbacks, rollbacks);
    assert!(matches!(a.events[0], BridgeEvent::Lifecycle(Lifecycle::SessionStarted { .. })));
    assert!(a.events.iter().any(|e| matches!(e, BridgeEvent::Sim { status: EventStatus::Verified(_), .. })), "no verified event");

    // And the sim state itself matches the loopback peers of both runs.
    assert_eq!(inproc.host().peer_a().predicted_frame().checksum(), a.frames.last().unwrap().predicted);
}

#[test]
fn previous_frame_is_the_corrected_history() {
    let mut inproc = InProc::new(make_pair(), bridge_config());
    let mut rng = FrameRng::new(42);
    let mut checked = 0;
    for t in 1..=400u64 {
        inproc.set_input(PlayerSlot(0), scripted(&mut rng, 0, t + 2)).unwrap();
        inproc.update(ONE_TICK);
        let snap = inproc.snapshot().unwrap();
        let Some(prev) = snap.predicted_prev() else { continue };
        assert_eq!(prev.tick() + 1, snap.tick());
        // What the sim itself holds for tick-1 right now, after any rollback.
        let truth = inproc.host().peer_a().frame_at(snap.tick() - 1).expect("still in the ring");
        assert_eq!(prev.checksum(), truth.checksum(), "stale previous frame at head {}", snap.tick());
        checked += 1;
    }
    assert!(checked > 300);
}

#[test]
fn published_snapshots_never_change() {
    let mut inproc = InProc::new(make_pair(), bridge_config());
    let mut kept: Vec<(Snapshot, u64, u64)> = Vec::new();
    let mut rng = FrameRng::new(42);
    for t in 1..=300u64 {
        inproc.set_input(PlayerSlot(0), scripted(&mut rng, 0, t + 2)).unwrap();
        inproc.update(ONE_TICK);
        let snap = inproc.snapshot().unwrap();
        let (p, v) = (snap.predicted().checksum(), snap.verified().map_or(0, |f| f.checksum()));
        kept.push((snap, p, v));
    }
    // The frame pool reuses allocations of dropped snapshots only; kept ones are intact
    // even though the sim went on through rollbacks.
    for (snap, p, v) in &kept {
        assert_eq!(snap.predicted().checksum(), *p);
        assert_eq!(snap.verified().map_or(0, |f| f.checksum()), *v);
    }
}

#[test]
fn only_the_local_player_can_be_driven() {
    let mut inproc = InProc::new(make_pair(), bridge_config());
    let err = inproc.set_input(PlayerSlot(1), ArenaInput::default()).unwrap_err();
    assert_eq!(err, BridgeError::NotLocalPlayer { player: PlayerSlot(1), local: PlayerSlot(0) });
    let mut threaded = Threaded::spawn(make_pair, bridge_config(), ThreadedConfig::default()).unwrap();
    assert_eq!(
        threaded.set_input(PlayerSlot(1), ArenaInput::default()).unwrap_err(),
        BridgeError::NotLocalPlayer { player: PlayerSlot(1), local: PlayerSlot(0) }
    );
}

#[test]
fn input_and_commands_reach_the_sim() {
    // Player 0 walks right; the sim position must follow. Positions come back only via the snapshot.
    let mut inproc = InProc::new(make_pair(), BridgeConfig::default());
    let start = inproc.snapshot().unwrap();
    let (entities, positions) = start.predicted().dense::<Position>();
    let before = positions[0].pos.x;
    let e0 = entities[0];
    inproc.set_input(PlayerSlot(0), ArenaInput::new(FP::ONE, FP::ZERO, false)).unwrap();
    inproc.step(60);
    let after = inproc.snapshot().unwrap().predicted().get::<Position>(e0).unwrap().pos.x;
    assert!(after > before, "input did not move the player");

    // A command spawns a bullet entity.
    let bullets_before = inproc.snapshot().unwrap().predicted().alive_count();
    inproc.send_command(SpawnBulletCmd { owner: 0 }).unwrap();
    inproc.step(10);
    let bullets_after = inproc.snapshot().unwrap().predicted().alive_count();
    assert!(bullets_after > bullets_before, "command did not spawn");
}

#[test]
fn events_keep_their_three_states() {
    let mut inproc = InProc::new(make_pair(), bridge_config());
    let mut rng = FrameRng::new(7);
    let mut predicted: BTreeSet<EventKey> = BTreeSet::new();
    let mut verified: BTreeMap<EventKey, u32> = BTreeMap::new();
    let mut canceled: BTreeSet<EventKey> = BTreeSet::new();
    for t in 1..=1500u64 {
        inproc.set_input(PlayerSlot(0), scripted(&mut rng, 0, t + 2)).unwrap();
        inproc.update(ONE_TICK);
        for ev in inproc.drain_events() {
            if let BridgeEvent::Sim { key, status } = ev {
                match status {
                    EventStatus::Predicted(_) => {
                        predicted.insert(key);
                    }
                    EventStatus::Verified(_) => *verified.entry(key).or_default() += 1,
                    EventStatus::Canceled => {
                        assert!(predicted.contains(&key), "canceled an event never predicted");
                        canceled.insert(key);
                    }
                }
            }
        }
    }
    assert!(!verified.is_empty());
    assert!(verified.values().all(|&n| n == 1), "an event was verified more than once");
}

#[test]
fn realtime_thread_ticks_without_blocking_the_reader() {
    let mut threaded = Threaded::spawn(make_pair, bridge_config(), ThreadedConfig::default()).unwrap();
    assert!(threaded.is_alive());
    threaded.set_input(PlayerSlot(0), ArenaInput::new(FP::ONE, FP::ZERO, false)).unwrap();

    let start = Instant::now();
    let mut reads = 0u64;
    let mut worst = Duration::ZERO;
    let mut last_tick = 0;
    while start.elapsed() < Duration::from_millis(500) {
        let t0 = Instant::now();
        threaded.update(Duration::from_millis(1)); // no-op in realtime
        if let Some(s) = threaded.snapshot() {
            last_tick = s.tick();
        }
        let _ = threaded.drain_events();
        worst = worst.max(t0.elapsed());
        reads += 1;
        std::thread::sleep(Duration::from_micros(200));
    }
    // 60 Hz for 0.5 s is about 30 ticks; allow scheduler noise.
    assert!((15..=40).contains(&last_tick), "sim ticked {last_tick} times in 0.5 s");
    assert!(reads > 100);
    assert!(worst < Duration::from_millis(50), "a view-side call took {worst:?}");
    drop(threaded); // joins the thread
}

#[test]
fn sim_thread_panic_is_reported_not_propagated() {
    let result = Threaded::<Arena>::spawn(
        || -> LoopbackPair<Arena> { panic!("host factory failed") },
        BridgeConfig::default(),
        ThreadedConfig::default(),
    );
    assert!(matches!(result, Err(BridgeError::Disconnected)));
}
