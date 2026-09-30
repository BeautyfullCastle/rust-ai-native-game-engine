//! Play session through the bridge: both adapters give the same timeline,
//! checksums and events; controls, debug commands and lifecycle events work
//! the same way; speed changes pacing only.
#![allow(clippy::disallowed_types)] // the realtime test uses the wall clock
use std::time::{Duration, Instant};

use orr_bridge::{
    Bridge, BridgeConfig, BridgeEvent, ControlOp, DebugCommand, DebugError, InProc, Lifecycle, Pacing, PlayConfig, PlayHost,
    PlayMode, PlaySession, PlayerSlot, SimControl, Speed, Threaded, ThreadedConfig, Timeline,
};
use orr_fp::FP;
use orr_sim::ComponentId;
use orr_testgame::{Arena, ArenaConfig, ArenaInput, PlayerTag};

const POSITION: ComponentId = ComponentId(0);
/// 50 ticks per second: one tick is exactly 20 ms.
const TICK: Duration = Duration::from_millis(20);

fn input_for(tick: u64, salt: u64) -> ArenaInput {
    let h = (tick ^ (salt << 40)).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 24;
    ArenaInput::new(FP::from_int((h % 3) as i32 - 1), FP::from_int(((h >> 3) % 3) as i32 - 1), (h >> 7) % 4 == 0)
}

fn play_config(paused: bool) -> PlayConfig {
    let mut cfg = PlayConfig::new(2, 11, 50);
    cfg.keyframe_interval = 16;
    cfg.ring_capacity = 8;
    cfg.start_paused = paused;
    cfg
}

fn make_host(paused: bool) -> PlayHost<Arena> {
    let session = PlaySession::<Arena>::new(play_config(paused), ArenaConfig { player_count: 2 });
    PlayHost::new(session, PlayerSlot(0))
}

fn inproc() -> InProc<Arena, PlayHost<Arena>> {
    InProc::new(make_host(true), BridgeConfig::default())
}

fn threaded_manual() -> Threaded<Arena> {
    Threaded::spawn(
        || make_host(true),
        BridgeConfig::default(),
        ThreadedConfig { pacing: Pacing::Manual, max_catchup: 8 },
    )
    .unwrap()
}

/// What the view sees after one call.
#[derive(Debug, PartialEq)]
struct Seen {
    tick: u64,
    predicted_checksum: u64,
    timeline: Timeline,
    events: Vec<BridgeEvent<orr_testgame::Hit>>,
}

fn look<B: Bridge<Arena> + SimControl<Arena>>(b: &mut B) -> Seen {
    let snap = b.snapshot().unwrap();
    Seen {
        tick: snap.tick(),
        predicted_checksum: snap.predicted().checksum(),
        timeline: snap.timeline().cloned().unwrap(),
        events: b.drain_events(),
    }
}

/// A scripted editor session: play, seek back, edit (which branches), play
/// on, seek forward, step, and a refused edit.
fn script<B: Bridge<Arena> + SimControl<Arena>>(b: &mut B) -> Vec<Seen> {
    let mut seen = vec![look(b)];
    b.control(ControlOp::Play).unwrap();
    seen.push(look(b));
    for _ in 0..70 {
        let next = seen.last().unwrap().tick + 1;
        b.set_input(PlayerSlot(0), input_for(next, 1)).unwrap();
        b.update(TICK);
        seen.push(look(b));
    }
    b.control(ControlOp::SetSpeed(Speed(2000))).unwrap();
    b.control(ControlOp::Seek(40)).unwrap();
    seen.push(look(b));
    b.control(ControlOp::Seek(41)).unwrap();
    seen.push(look(b));
    // Edit: entity of player 0 gets x = 5. Refused edits report and change nothing.
    let snap = b.snapshot().unwrap();
    let e = snap.predicted().dense::<PlayerTag>().0[0];
    b.debug_command(DebugCommand::SetField {
        entity: e,
        component: POSITION,
        offset: 0,
        bytes: FP::from_int(5).0.to_le_bytes().to_vec(),
    })
    .unwrap();
    seen.push(look(b));
    b.debug_command(DebugCommand::Despawn { entity: orr_sim::Entity { index: 900, version: 0 } }).unwrap();
    seen.push(look(b));
    b.control(ControlOp::Play).unwrap();
    for _ in 0..30 {
        let next = seen.last().unwrap().tick + 1;
        b.set_input(PlayerSlot(0), input_for(next, 2)).unwrap();
        b.update(TICK / 2); // 2x speed: one tick per half tick time
        seen.push(look(b));
    }
    b.control(ControlOp::Pause).unwrap();
    b.control(ControlOp::Step(3)).unwrap();
    seen.push(look(b));
    b.control(ControlOp::Seek(20)).unwrap();
    seen.push(look(b));
    b.control(ControlOp::Seek(60)).unwrap();
    seen.push(look(b));
    seen
}

#[test]
fn inproc_and_threaded_give_the_same_timeline_checksums_and_events() {
    let a = script(&mut inproc());
    let b = script(&mut threaded_manual());
    assert_eq!(a.len(), b.len());
    for (i, (x, y)) in a.iter().zip(&b).enumerate() {
        assert_eq!(x, y, "step {i}");
    }
    let last = a.last().unwrap();
    assert_eq!(last.tick, 60);
    assert_eq!(last.timeline.mode, PlayMode::Record);
    assert_eq!(last.timeline.branches, 1);
    assert!(!last.timeline.playing);
    assert_eq!(last.timeline.first_tick, 0);
    assert!(last.timeline.last_tick >= 60);
    assert!(last.timeline.keyframes.len() >= 3);
    assert_ne!(last.timeline.checksum, 0);
    assert_eq!(last.predicted_checksum, last.timeline.checksum, "the recorded checksum is the frame's");
}

#[test]
fn lifecycle_events_mark_seek_branch_pause_and_refused_edits() {
    let mut b = inproc();
    b.drain_events();
    b.control(ControlOp::Play).unwrap();
    b.step(20);
    b.control(ControlOp::Pause).unwrap();
    b.control(ControlOp::Seek(5)).unwrap();
    b.control(ControlOp::Seek(999)).unwrap();
    b.control(ControlOp::Branch).unwrap();
    let e = b.snapshot().unwrap().predicted().dense::<PlayerTag>().0[0];
    b.debug_command(DebugCommand::Despawn { entity: orr_sim::Entity { index: 77, version: 3 } }).unwrap();
    b.debug_command(DebugCommand::Despawn { entity: e }).unwrap();
    let lifecycle: Vec<Lifecycle> = b
        .drain_events()
        .into_iter()
        .filter_map(|ev| match ev {
            BridgeEvent::Lifecycle(l) => Some(l),
            _ => None,
        })
        .collect();
    assert_eq!(
        lifecycle,
        vec![
            Lifecycle::Resumed { tick: 0 },
            Lifecycle::Paused { tick: 20 },
            Lifecycle::Seeked { from: 20, to: 5 },
            Lifecycle::SeekRejected { target: 999 },
            Lifecycle::Branched { tick: 5, dropped: 15 },
            Lifecycle::DebugRejected(DebugError::EntityNotAlive),
        ]
    );
    assert_eq!(b.timeline().unwrap().last_tick, 5);
}

#[test]
fn seek_makes_the_snapshot_fresh_with_no_stale_previous_frame() {
    let mut b = inproc();
    b.control(ControlOp::Play).unwrap();
    b.step(30);
    let before = b.snapshot().unwrap();
    assert_eq!(before.tick(), 30);
    b.control(ControlOp::Seek(12)).unwrap();
    let after = b.snapshot().unwrap();
    assert_ne!(after.seq(), before.seq());
    assert_eq!(after.tick(), 12);
    assert_eq!(after.predicted().tick(), 12);
    if let Some(prev) = after.predicted_prev() {
        assert_eq!(prev.tick(), 11);
    }
    // An edit at the same tick publishes a new snapshot with the edited frame.
    let e = after.predicted().dense::<PlayerTag>().0[0];
    let seq = after.seq();
    let sum = after.predicted().checksum();
    b.debug_command(DebugCommand::SetField {
        entity: e,
        component: POSITION,
        offset: 0,
        bytes: FP::from_int(123).0.to_le_bytes().to_vec(),
    })
    .unwrap();
    let edited = b.snapshot().unwrap();
    assert_ne!(edited.seq(), seq);
    assert_ne!(edited.predicted().checksum(), sum);
    assert_eq!(edited.tick(), 12);
}

#[test]
fn paused_session_does_not_advance_on_update_and_resume_does_not_burst() {
    let mut b = inproc();
    for _ in 0..10 {
        b.update(TICK);
    }
    assert_eq!(b.snapshot().unwrap().tick(), 0, "starts paused");
    b.control(ControlOp::Play).unwrap();
    b.update(Duration::from_secs(3)); // one long frame: bounded catch-up only
    assert_eq!(b.snapshot().unwrap().tick(), 8);
    b.control(ControlOp::Pause).unwrap();
    b.update(Duration::from_secs(5));
    assert_eq!(b.snapshot().unwrap().tick(), 8);
    b.control(ControlOp::Play).unwrap();
    b.update(TICK);
    assert_eq!(b.snapshot().unwrap().tick(), 9, "no burst after the pause");
}

#[test]
fn speed_changes_pacing_only_never_results() {
    let mut results = Vec::new();
    for permille in [250u32, 500, 1000, 2000, 4000] {
        let mut b = inproc();
        b.control(ControlOp::SetSpeed(Speed(permille))).unwrap();
        b.control(ControlOp::Play).unwrap();
        // One tick of wall time at this speed: exactly one sim tick.
        let frame = TICK * 1000 / permille;
        for tick in 1..=120u64 {
            b.set_input(PlayerSlot(0), input_for(tick, 3)).unwrap();
            b.update(frame);
            assert_eq!(b.snapshot().unwrap().tick(), tick, "speed {permille}");
        }
        let snap = b.snapshot().unwrap();
        results.push((snap.predicted().checksum(), snap.timeline().unwrap().recent_checksums.to_vec()));
    }
    assert!(results.windows(2).all(|w| w[0] == w[1]), "speed must not change any checksum");

    // At 4x one 20 ms frame runs four ticks; at 0.25x it takes four frames.
    let mut fast = inproc();
    fast.control(ControlOp::SetSpeed(Speed::MAX)).unwrap();
    fast.control(ControlOp::Play).unwrap();
    fast.update(TICK);
    assert_eq!(fast.snapshot().unwrap().tick(), 4);
    let mut slow = inproc();
    slow.control(ControlOp::SetSpeed(Speed::MIN)).unwrap();
    slow.control(ControlOp::Play).unwrap();
    for _ in 0..3 {
        slow.update(TICK);
    }
    assert_eq!(slow.snapshot().unwrap().tick(), 0);
    slow.update(TICK);
    assert_eq!(slow.snapshot().unwrap().tick(), 1);
}

#[test]
fn a_bridge_without_a_play_session_ignores_controls_and_refuses_edits() {
    // A plain session host has no timeline.
    use orr_session::{LoopbackNetwork, Session, SessionConfig};
    let (end, _other, _clock) = LoopbackNetwork::new::<Arena>(0, 0, 1);
    let session = Session::<Arena, _>::new(ArenaConfig { player_count: 1 }, SessionConfig::new(1, PlayerSlot(0), 1, 50), end);
    let mut b = InProc::new(session, BridgeConfig::default());
    assert!(b.timeline().is_none());
    b.drain_events();
    b.control(ControlOp::Seek(3)).unwrap();
    b.debug_command(DebugCommand::Despawn { entity: orr_sim::Entity { index: 0, version: 0 } }).unwrap();
    let events = b.drain_events();
    assert!(events.contains(&BridgeEvent::Lifecycle(Lifecycle::DebugRejected(DebugError::Unsupported))));
    b.step(3);
    assert_eq!(b.snapshot().unwrap().tick(), 3);
}

#[test]
fn replay_viewer_scrubs_through_the_bridge() {
    // Record with a session, then open the file as a viewer host.
    let mut rec = PlaySession::<Arena>::new(play_config(false), ArenaConfig { player_count: 2 });
    for tick in 1..=80u64 {
        rec.set_input(PlayerSlot(0), input_for(tick, 5));
        rec.control(ControlOp::Step(1));
    }
    let bytes = rec.save_replay();
    let expected: Vec<u64> = (0..=80).map(|t| rec.checksum_at(t).unwrap()).collect();

    let viewer = PlaySession::<Arena>::open_replay(&bytes, ArenaConfig { player_count: 2 }, 0).unwrap();
    let mut b = InProc::new(PlayHost::new(viewer, PlayerSlot(0)), BridgeConfig::default());
    let tl = b.timeline().unwrap();
    assert_eq!((tl.mode, tl.tick, tl.last_tick), (PlayMode::Viewer, 0, 80));
    for &t in &[80u64, 3, 41, 42, 79, 0] {
        b.control(ControlOp::Seek(t)).unwrap();
        assert_eq!(b.snapshot().unwrap().predicted().checksum(), expected[t as usize], "seek {t}");
    }
    // Edits are refused until a branch.
    b.drain_events();
    let e = b.snapshot().unwrap().predicted().dense::<PlayerTag>().0[0];
    b.debug_command(DebugCommand::Despawn { entity: e }).unwrap();
    assert!(b.drain_events().contains(&BridgeEvent::Lifecycle(Lifecycle::DebugRejected(DebugError::ReadOnly))));
    // Play back to the end: pauses by itself.
    b.control(ControlOp::Play).unwrap();
    for _ in 0..100 {
        b.update(TICK);
    }
    let tl = b.timeline().unwrap();
    assert_eq!((tl.tick, tl.playing), (80, false));
    assert_eq!(b.snapshot().unwrap().predicted().checksum(), expected[80]);
    // Branch: now it records and takes edits.
    b.control(ControlOp::Seek(50)).unwrap();
    b.control(ControlOp::Branch).unwrap();
    assert_eq!(b.timeline().unwrap().mode, PlayMode::Record);
    b.debug_command(DebugCommand::Despawn { entity: e }).unwrap();
    b.control(ControlOp::Step(5)).unwrap();
    let tl = b.timeline().unwrap();
    assert_eq!((tl.tick, tl.last_tick), (55, 55));
}

#[test]
fn threaded_realtime_plays_pauses_seeks_and_never_blocks_the_caller() {
    let mut b = Threaded::spawn(|| make_host(true), BridgeConfig::default(), ThreadedConfig::default()).unwrap();
    let wait_for = |b: &Threaded<Arena>, what: &str, ok: &dyn Fn(&Timeline) -> bool| {
        let start = Instant::now();
        loop {
            if let Some(t) = b.timeline() {
                if ok(&t) {
                    return t;
                }
            }
            assert!(start.elapsed() < Duration::from_secs(5), "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(b.timeline().unwrap().tick, 0, "starts paused");

    let t0 = Instant::now();
    b.control(ControlOp::SetSpeed(Speed::MAX)).unwrap();
    b.control(ControlOp::Play).unwrap();
    assert!(t0.elapsed() < Duration::from_millis(50), "control calls do not wait for the sim");
    wait_for(&b, "40 ticks", &|t| t.tick >= 40);
    b.control(ControlOp::Pause).unwrap();
    let paused = wait_for(&b, "pause", &|t| !t.playing);
    std::thread::sleep(Duration::from_millis(100));
    let later = b.timeline().unwrap();
    assert_eq!(later.tick, paused.tick, "paused: no more ticks");

    b.control(ControlOp::Seek(10)).unwrap();
    let t = wait_for(&b, "seek", &|t| t.tick == 10);
    assert!(t.last_tick >= 40);
    let snap = b.snapshot().unwrap();
    assert_eq!(snap.predicted().checksum(), t.checksum);
    let events = b.drain_events();
    assert!(events.iter().any(|e| matches!(e, BridgeEvent::Lifecycle(Lifecycle::Seeked { to: 10, .. }))));

    // Resume from the rewound point: branches, then ticks on from 10.
    b.control(ControlOp::Play).unwrap();
    let t = wait_for(&b, "branch and play", &|t| t.branches == 1 && t.tick >= 20);
    assert_eq!(t.first_tick, 0);
    assert!(b.is_alive());
}
