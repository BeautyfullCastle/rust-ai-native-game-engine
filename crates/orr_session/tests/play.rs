//! Play session tests with the arena game: seek and branch are exact,
//! debug commands are recorded and replayed, bad commands never panic.
use orr_ecs::{ComponentId, Entity, SingletonId};
use orr_fp::FP;
use orr_session::{
    replay_verify, ControlOp, DebugCommand, DebugError, PlayConfig, PlayError, PlayMode, PlayNote, PlaySession, PlayerSlot,
    Speed,
};
use orr_sim::{Simulation, TickInputs};
use orr_testgame::{Arena, ArenaConfig, ArenaInput, Bullet, PlayerTag, Position, SpawnBulletCmd};

const POSITION: ComponentId = ComponentId(0);
const PLAYER_TAG: ComponentId = ComponentId(1);
const BULLET: ComponentId = ComponentId(2);
const SCORE: SingletonId = SingletonId(1);

fn input_for(slot: u8, tick: u64, salt: u64) -> ArenaInput {
    let h = (tick ^ (u64::from(slot) << 32) ^ (salt << 40)).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 24;
    let ax = (h % 3) as i32 - 1;
    let ay = ((h >> 3) % 3) as i32 - 1;
    ArenaInput::new(FP::from_int(ax), FP::from_int(ay), (h >> 7) % 4 == 0)
}

fn config() -> PlayConfig {
    let mut cfg = PlayConfig::new(2, 7, 60);
    cfg.keyframe_interval = 20;
    cfg.ring_capacity = 16;
    cfg
}

fn new_session() -> PlaySession<Arena> {
    PlaySession::<Arena>::new(config(), ArenaConfig { player_count: 2 })
}

/// Runs the session `n` more ticks with the scripted inputs.
fn play(s: &mut PlaySession<Arena>, n: u64, salt: u64) {
    for _ in 0..n {
        let next = s.head_tick() + 1;
        for slot in 0..2u8 {
            s.set_input(PlayerSlot(slot), input_for(slot, next, salt));
        }
        s.control(ControlOp::Step(1));
    }
}

/// Checksums of ticks 1..=n from a plain simulation with the same inputs.
fn straight(n: u64, salt: u64) -> Vec<u64> {
    let mut sim = Simulation::<Arena>::new(ArenaConfig { player_count: 2 }, 60, 7);
    (1..=n)
        .map(|tick| {
            let mut ti = TickInputs::new(tick, 2);
            for slot in 0..2u8 {
                ti.set_input(PlayerSlot(slot), input_for(slot, tick, salt));
            }
            sim.step(&ti);
            sim.checksum()
        })
        .collect()
}

fn first_entity(s: &PlaySession<Arena>) -> Entity {
    s.frame().dense::<PlayerTag>().0[0]
}

fn f(x: i32) -> Vec<u8> {
    FP::from_int(x).0.to_le_bytes().to_vec()
}

#[test]
fn seek_back_and_forward_matches_straight_play() {
    let expected = straight(300, 1);
    let mut s = new_session();
    s.control(ControlOp::Pause);
    play(&mut s, 300, 1);
    assert_eq!(s.head_tick(), 300);
    // Ring holds 16 ticks and keyframes are 20 apart, so this walks the
    // ring, keyframe and continue-from-head paths.
    let targets = [299u64, 290, 250, 100, 99, 20, 21, 1, 0, 150, 151, 152, 300, 40, 300, 280, 281, 120];
    for &t in &targets {
        s.control(ControlOp::Seek(t));
        assert_eq!(s.head_tick(), t);
        if t > 0 {
            assert_eq!(s.frame().checksum(), expected[t as usize - 1], "seek to {t}");
        }
        assert_eq!(s.frame().checksum(), s.checksum_at(t).unwrap(), "recorded checksum of {t}");
    }
    // Playing forward after seeking (to the end) still matches.
    s.control(ControlOp::Seek(300));
    play(&mut s, 10, 1);
    let more = straight(310, 1);
    assert_eq!(s.frame().checksum(), more[309]);
}

#[test]
fn seek_outside_range_is_refused() {
    let mut s = new_session();
    play(&mut s, 10, 1);
    s.control(ControlOp::Seek(11));
    assert_eq!(s.head_tick(), 10);
    assert!(s.take_notes().iter().any(|n| matches!(n, PlayNote::SeekRejected { target: 11 })));
    assert!(s.seek(11).is_err());
}

#[test]
fn seek_pauses_and_emits_seeked() {
    let mut s = new_session();
    play(&mut s, 30, 1);
    assert!(s.is_playing());
    let _ = s.take_notes();
    s.control(ControlOp::Seek(10));
    assert!(!s.is_playing());
    let notes = s.take_notes();
    assert!(notes.contains(&PlayNote::Paused { tick: 30 }));
    assert!(notes.contains(&PlayNote::Seeked { from: 30, to: 10 }));
}

#[test]
fn branch_diverges_only_after_the_branch_tick() {
    let original = straight(200, 1);
    let mut s = new_session();
    play(&mut s, 200, 1);
    s.control(ControlOp::Seek(100));
    assert_eq!(s.last_tick(), 200, "seeking alone keeps the future");
    let _ = s.take_notes();
    // Different inputs from tick 101 on: the first tick branches.
    play(&mut s, 60, 2);
    assert!(s.take_notes().contains(&PlayNote::Branched { tick: 100, dropped: 100 }));
    assert_eq!(s.branch_count(), 1);
    assert_eq!(s.last_tick(), 160);
    assert_eq!(s.head_tick(), 160);
    for t in 1..=100u64 {
        assert_eq!(s.checksum_at(t).unwrap(), original[t as usize - 1], "tick {t} is before the branch");
    }
    assert!(
        (101..=110u64).any(|t| s.checksum_at(t).unwrap() != original[t as usize - 1]),
        "the branch has other inputs, so it diverges soon after tick 100"
    );

    // The branch equals a straight run with the mixed inputs.
    let mut sim = Simulation::<Arena>::new(ArenaConfig { player_count: 2 }, 60, 7);
    for tick in 1..=160u64 {
        let salt = if tick <= 100 { 1 } else { 2 };
        let mut ti = TickInputs::new(tick, 2);
        for slot in 0..2u8 {
            ti.set_input(PlayerSlot(slot), input_for(slot, tick, salt));
        }
        sim.step(&ti);
        assert_eq!(s.checksum_at(tick).unwrap(), sim.checksum(), "tick {tick}");
    }
    // And the branch survives a seek through the old keyframes.
    s.control(ControlOp::Seek(130));
    assert_eq!(s.frame().checksum(), s.checksum_at(130).unwrap());
    // The recording verifies from tick 0.
    let report = replay_verify::<Arena>(&s.save_replay(), ArenaConfig { player_count: 2 }).unwrap();
    assert!(report.ok(), "{report:?}");
    assert!(report.checksums_checked >= 160);
}

#[test]
fn explicit_branch_cuts_the_future_and_keeps_head() {
    let mut s = new_session();
    play(&mut s, 50, 1);
    s.control(ControlOp::Seek(30));
    s.control(ControlOp::Branch);
    assert_eq!(s.last_tick(), 30);
    assert_eq!(s.head_tick(), 30);
    assert!(s.seek(31).is_err());
}

#[test]
fn debug_commands_are_recorded_and_replayed() {
    let mut s = new_session();
    play(&mut s, 25, 1);
    let e = first_entity(&s);

    // Move the player: set Position.pos.x (offset 0) to 100.
    s.debug(DebugCommand::SetField { entity: e, component: POSITION, offset: 0, bytes: f(100) }).unwrap();
    play(&mut s, 10, 1);
    // Spawn a bullet entity with whole component values.
    let bullet = Bullet { velocity: orr_fp::FPVec2::new(FP::from_int(3), FP::ZERO), owner_slot: 0, ttl: 50 };
    let pos = Position { pos: orr_fp::FPVec2::new(FP::from_int(-40), FP::from_int(10)) };
    s.debug(DebugCommand::Spawn {
        components: vec![
            (POSITION, bytemuck::bytes_of(&pos).to_vec()),
            (BULLET, bytemuck::bytes_of(&bullet).to_vec()),
        ],
    })
    .unwrap();
    // Two edits in one boundary, in order.
    s.debug(DebugCommand::SetSingletonField { singleton: SCORE, offset: 0, bytes: 5u32.to_le_bytes().to_vec() }).unwrap();
    s.debug(DebugCommand::AddComponent { entity: e, component: BULLET, bytes: bytemuck::bytes_of(&bullet).to_vec() })
        .unwrap();
    assert_eq!(s.timeline().pending_edits, 3);
    play(&mut s, 10, 1);
    s.debug(DebugCommand::RemoveComponent { entity: e, component: BULLET }).unwrap();
    play(&mut s, 5, 1);
    s.debug(DebugCommand::Despawn { entity: e }).unwrap();
    play(&mut s, 20, 1);
    assert_eq!(s.head_tick(), 70);
    assert_eq!(s.timeline().pending_edits, 0);

    // Straight replay from tick 0 reproduces every recorded checksum.
    let bytes = s.save_replay();
    let report = replay_verify::<Arena>(&bytes, ArenaConfig { player_count: 2 }).unwrap();
    assert!(report.ok(), "{report:?}");
    assert!(report.checksums_checked >= 70);

    // The recording differs from a run without the edits.
    let plain = straight(70, 1);
    assert_ne!(s.checksum_at(70).unwrap(), plain[69]);

    // Seeking back over the edits and forward again gives the same checksums.
    let recorded: Vec<u64> = (0..=70).map(|t| s.checksum_at(t).unwrap()).collect();
    for &t in &[69u64, 60, 40, 35, 26, 10, 0, 70, 33, 68] {
        s.control(ControlOp::Seek(t));
        assert_eq!(s.frame().checksum(), recorded[t as usize], "seek to {t}");
    }

    // A viewer opened from the file reproduces the same states.
    let mut v = PlaySession::<Arena>::open_replay(&bytes, ArenaConfig { player_count: 2 }, 0).unwrap();
    assert_eq!(v.mode(), PlayMode::Viewer);
    assert_eq!((v.first_tick(), v.last_tick()), (0, 70));
    for &t in &[70u64, 5, 27, 36, 50, 66, 70] {
        v.control(ControlOp::Seek(t));
        assert_eq!(v.frame().checksum(), recorded[t as usize], "viewer seek to {t}");
    }
}

#[test]
fn edit_while_paused_shows_at_once_and_replays() {
    let mut s = new_session();
    play(&mut s, 12, 1);
    s.control(ControlOp::Pause);
    let e = first_entity(&s);
    let before = s.frame().checksum();
    s.debug(DebugCommand::SetField { entity: e, component: POSITION, offset: 0, bytes: f(7) }).unwrap();
    assert_ne!(s.frame().checksum(), before, "the edit shows without a tick");
    assert_eq!(s.head_tick(), 12);
    play(&mut s, 8, 1);
    // Applied once, at the boundary of tick 13.
    let bytes = s.save_replay();
    assert!(replay_verify::<Arena>(&bytes, ArenaConfig { player_count: 2 }).unwrap().ok());
    // Seek away and back to the edited head, then step: same result.
    let end = s.checksum_at(20).unwrap();
    s.control(ControlOp::Seek(5));
    s.control(ControlOp::Seek(20));
    assert_eq!(s.frame().checksum(), end);
}

#[test]
fn edit_while_rewound_branches() {
    let mut s = new_session();
    play(&mut s, 40, 1);
    s.control(ControlOp::Seek(20));
    let e = first_entity(&s);
    let _ = s.take_notes();
    s.debug(DebugCommand::SetField { entity: e, component: POSITION, offset: 0, bytes: f(-9) }).unwrap();
    assert!(s.take_notes().contains(&PlayNote::Branched { tick: 20, dropped: 20 }));
    assert_eq!(s.last_tick(), 20);
    play(&mut s, 30, 1);
    assert!(replay_verify::<Arena>(&s.save_replay(), ArenaConfig { player_count: 2 }).unwrap().ok());
}

#[test]
fn bad_debug_commands_are_refused_without_change() {
    let mut s = new_session();
    play(&mut s, 5, 1);
    let e = first_entity(&s);
    let stale = Entity { index: e.index, version: e.version + 9 };
    let dead = Entity { index: 4000, version: 0 };
    let no_bullet = e;
    let cases: Vec<(DebugCommand, DebugError)> = vec![
        (DebugCommand::SetField { entity: dead, component: POSITION, offset: 0, bytes: f(1) }, DebugError::EntityNotAlive),
        (DebugCommand::SetField { entity: stale, component: POSITION, offset: 0, bytes: f(1) }, DebugError::EntityNotAlive),
        (DebugCommand::SetField { entity: e, component: ComponentId(99), offset: 0, bytes: f(1) }, DebugError::UnknownComponent),
        (DebugCommand::SetField { entity: e, component: POSITION, offset: 16, bytes: f(1) }, DebugError::OutOfBounds),
        (DebugCommand::SetField { entity: e, component: POSITION, offset: 12, bytes: f(1) }, DebugError::OutOfBounds),
        (DebugCommand::SetField { entity: e, component: POSITION, offset: u32::MAX, bytes: f(1) }, DebugError::OutOfBounds),
        (DebugCommand::SetField { entity: e, component: POSITION, offset: 0, bytes: vec![] }, DebugError::OutOfBounds),
        (DebugCommand::SetField { entity: e, component: POSITION, offset: 0, bytes: vec![0; 17] }, DebugError::OutOfBounds),
        (DebugCommand::SetField { entity: no_bullet, component: BULLET, offset: 0, bytes: f(1) }, DebugError::MissingComponent),
        (DebugCommand::SetSingletonField { singleton: SingletonId(9), offset: 0, bytes: f(1) }, DebugError::UnknownSingleton),
        (DebugCommand::SetSingletonField { singleton: SCORE, offset: 30, bytes: f(1) }, DebugError::OutOfBounds),
        (DebugCommand::Spawn { components: vec![(POSITION, vec![1, 2, 3])] }, DebugError::SizeMismatch),
        (DebugCommand::Spawn { components: vec![(ComponentId(50), vec![])] }, DebugError::UnknownComponent),
        (
            DebugCommand::Spawn { components: vec![(PLAYER_TAG, vec![0; 4]), (PLAYER_TAG, vec![0; 4])] },
            DebugError::DuplicateComponent,
        ),
        (DebugCommand::Despawn { entity: dead }, DebugError::EntityNotAlive),
        (DebugCommand::Despawn { entity: Entity::NONE }, DebugError::EntityNotAlive),
        (DebugCommand::AddComponent { entity: dead, component: POSITION, bytes: vec![0; 16] }, DebugError::EntityNotAlive),
        (DebugCommand::AddComponent { entity: e, component: POSITION, bytes: vec![0; 15] }, DebugError::SizeMismatch),
        (DebugCommand::RemoveComponent { entity: e, component: BULLET }, DebugError::MissingComponent),
        (DebugCommand::RemoveComponent { entity: e, component: ComponentId(77) }, DebugError::UnknownComponent),
    ];
    let checksum = s.frame().checksum();
    let recorded_before = s.save_replay();
    for (cmd, expected) in cases {
        let _ = s.take_notes();
        assert_eq!(s.debug(cmd.clone()), Err(expected), "{cmd:?}");
        assert!(s.take_notes().contains(&PlayNote::DebugRejected(expected)));
        assert_eq!(s.frame().checksum(), checksum, "{cmd:?} changed the frame");
    }
    assert_eq!(s.timeline().pending_edits, 0, "refused commands are not recorded");
    assert_eq!(s.save_replay(), recorded_before);
    // The session still works.
    play(&mut s, 5, 1);
    assert!(replay_verify::<Arena>(&s.save_replay(), ArenaConfig { player_count: 2 }).unwrap().ok());
}

/// A refused command must not branch a rewound session.
#[test]
fn refused_edit_does_not_branch() {
    let mut s = new_session();
    play(&mut s, 20, 1);
    s.control(ControlOp::Seek(10));
    let bad = DebugCommand::Despawn { entity: Entity { index: 999, version: 0 } };
    assert!(s.debug(bad).is_err());
    assert_eq!(s.last_tick(), 20);
    assert_eq!(s.branch_count(), 0);
}

#[test]
fn debug_command_wire_form_round_trips_and_never_panics() {
    let e = Entity { index: 3, version: 2 };
    let cmds = vec![
        DebugCommand::SetField { entity: e, component: POSITION, offset: 4, bytes: vec![1, 2, 3] },
        DebugCommand::SetSingletonField { singleton: SCORE, offset: 0, bytes: vec![9] },
        DebugCommand::Spawn { components: vec![(POSITION, vec![0; 16]), (PLAYER_TAG, vec![1, 2, 3, 4])] },
        DebugCommand::Spawn { components: vec![] },
        DebugCommand::Despawn { entity: e },
        DebugCommand::AddComponent { entity: e, component: BULLET, bytes: vec![7; 24] },
        DebugCommand::RemoveComponent { entity: e, component: BULLET },
    ];
    for cmd in &cmds {
        let mut bytes = Vec::new();
        cmd.encode(&mut bytes);
        assert_eq!(DebugCommand::decode(&bytes).as_ref(), Some(cmd));
        // Every truncation and a trailing byte are rejected, not a panic.
        for cut in 0..bytes.len() {
            assert!(DebugCommand::decode(&bytes[..cut]).is_none(), "cut {cut} of {cmd:?}");
        }
        bytes.push(0);
        assert!(DebugCommand::decode(&bytes).is_none());
    }
    // Random bytes: decode, and apply whatever decodes. No panic, and a
    // refused command leaves the frame alone.
    let mut s = new_session();
    play(&mut s, 3, 1);
    let mut x = 0x1234_5678_9ABC_DEF0u64;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for _ in 0..20_000 {
        let len = (next() % 40) as usize;
        let mut bytes: Vec<u8> = (0..len).map(|_| next() as u8).collect();
        if let Some(first) = bytes.first_mut() {
            *first = (*first % 8).max(1);
        }
        if let Some(cmd) = DebugCommand::decode(&bytes) {
            let before = s.frame().checksum();
            if s.debug(cmd).is_err() {
                assert_eq!(s.frame().checksum(), before);
            }
        }
    }
    play(&mut s, 3, 1);
}

#[test]
fn viewer_is_read_only_and_branch_makes_a_new_recording() {
    let mut s = new_session();
    play(&mut s, 60, 1);
    let bytes = s.save_replay();
    let recorded: Vec<u64> = (0..=60).map(|t| s.checksum_at(t).unwrap()).collect();

    let mut v = PlaySession::<Arena>::open_replay(&bytes, ArenaConfig { player_count: 2 }, 0).unwrap();
    assert_eq!(v.head_tick(), 0);
    assert!(!v.is_playing());
    v.set_input(PlayerSlot(0), ArenaInput::new(FP::ONE, FP::ZERO, true));
    assert_eq!(v.push_command(PlayerSlot(0), SpawnBulletCmd { owner: 0 }), Err(PlayError::ReadOnly));
    assert_eq!(v.pending_command_count(), 0);
    assert_eq!(v.save_replay(), bytes);
    let e = first_entity(&v);
    assert_eq!(
        v.debug(DebugCommand::Despawn { entity: e }),
        Err(DebugError::ReadOnly),
        "a viewer refuses edits"
    );
    // Play runs the recorded inputs and pauses at the end.
    v.control(ControlOp::Play);
    let mut guard = 0;
    while v.wants_tick() && guard < 1000 {
        v.tick();
        guard += 1;
    }
    assert_eq!(v.head_tick(), 60);
    assert!(!v.is_playing());
    assert_eq!(v.frame().checksum(), recorded[60]);
    v.control(ControlOp::Step(3));
    assert_eq!(v.head_tick(), 60, "nothing past the last tick");

    // Seek back, branch: a new recording from tick 25.
    v.control(ControlOp::Seek(25));
    v.control(ControlOp::Branch);
    assert_eq!(v.mode(), PlayMode::Record);
    assert_eq!(v.last_tick(), 25);
    v.set_input(PlayerSlot(0), input_for(0, 26, 9));
    assert_eq!(v.push_command(PlayerSlot(0), SpawnBulletCmd { owner: 0 }), Ok(()));
    assert_eq!(v.pending_command_count(), 1);
    v.control(ControlOp::Step(15));
    assert_eq!(v.pending_command_count(), 0);
    let branched = v.save_replay();
    assert_ne!(branched, bytes);
    assert!(replay_verify::<Arena>(&branched, ArenaConfig { player_count: 2 }).unwrap().ok());
    // The original file bytes still verify and are unchanged.
    assert!(replay_verify::<Arena>(&bytes, ArenaConfig { player_count: 2 }).unwrap().ok());
    assert_eq!(v.checksum_at(25).unwrap(), recorded[25]);
    // An edit in the branched session works.
    let e = first_entity(&v);
    v.debug(DebugCommand::Despawn { entity: e }).unwrap();
    v.control(ControlOp::Step(5));
    assert!(replay_verify::<Arena>(&v.save_replay(), ArenaConfig { player_count: 2 }).unwrap().ok());
}

#[test]
fn viewer_input_does_not_leak_into_branch() {
    let mut recorded = new_session();
    play(&mut recorded, 12, 1);
    let bytes = recorded.save_replay();
    let input = ArenaInput::new(FP::ONE, FP::ZERO, false);

    let mut viewer = PlaySession::<Arena>::open_replay(&bytes, ArenaConfig { player_count: 2 }, 0).unwrap();
    viewer.set_input(PlayerSlot(0), input);
    viewer.control(ControlOp::Seek(6));
    viewer.control(ControlOp::Branch);
    viewer.control(ControlOp::Step(1));
    let actual = viewer.frame().checksum();

    let mut clean = PlaySession::<Arena>::open_replay(&bytes, ArenaConfig { player_count: 2 }, 0).unwrap();
    clean.control(ControlOp::Seek(6));
    clean.control(ControlOp::Branch);
    clean.control(ControlOp::Step(1));
    assert_eq!(actual, clean.frame().checksum(), "viewer input must not be retained by Branch");

    let mut writable_input = PlaySession::<Arena>::open_replay(&bytes, ArenaConfig { player_count: 2 }, 0).unwrap();
    writable_input.control(ControlOp::Seek(6));
    writable_input.control(ControlOp::Branch);
    writable_input.set_input(PlayerSlot(0), input);
    writable_input.control(ControlOp::Step(1));
    assert_ne!(actual, writable_input.frame().checksum(), "the chosen input must affect a writable branch");
}

#[test]
fn start_from_a_frame_and_from_a_setup_closure() {
    let mut a = new_session();
    play(&mut a, 15, 1);
    let start = a.frame().clone();

    // Same start, from a baked frame: continues like the original.
    let mut b = PlaySession::<Arena>::from_frame(config(), &start).unwrap();
    assert_eq!(b.head_tick(), 15);
    assert_eq!(b.first_tick(), 15);
    assert_eq!(b.frame().checksum(), a.frame().checksum());
    play(&mut a, 20, 1);
    play(&mut b, 20, 1);
    assert_eq!(a.frame().checksum(), b.frame().checksum());
    b.control(ControlOp::Seek(18));
    assert_eq!(b.frame().checksum(), a.checksum_at(18).unwrap());
    assert!(b.seek(14).is_err(), "nothing before the initial frame");

    // A setup closure fills the frame.
    let mut c = PlaySession::<Arena>::from_setup(config(), |frame| {
        let e = frame.spawn();
        frame.add(e, Position { pos: orr_fp::FPVec2::new(FP::from_int(1), FP::from_int(2)) });
    });
    play(&mut c, 30, 3);
    let end = c.frame().checksum();
    c.control(ControlOp::Seek(4));
    c.control(ControlOp::Seek(30));
    assert_eq!(c.frame().checksum(), end);
}

#[test]
fn speed_is_clamped_and_pause_stops_the_clock() {
    let mut s = new_session();
    s.control(ControlOp::SetSpeed(Speed::from_permille(10)));
    assert_eq!(s.speed(), Speed::MIN);
    s.control(ControlOp::SetSpeed(Speed(90_000)));
    assert_eq!(s.speed(), Speed::MAX);
    s.control(ControlOp::SetSpeed(Speed(500)));
    assert_eq!(s.speed().permille(), 500);
    assert!(s.wants_tick());
    s.control(ControlOp::Pause);
    assert!(!s.wants_tick());
    assert!(s.tick().is_empty());
    assert_eq!(s.head_tick(), 0);
    s.control(ControlOp::Step(2));
    assert_eq!(s.head_tick(), 2, "step runs while paused");
    s.control(ControlOp::Play);
    s.tick();
    assert_eq!(s.head_tick(), 3);
}

#[test]
fn timeline_reports_range_keyframes_and_checksums() {
    let mut s = new_session();
    play(&mut s, 70, 1);
    let t = s.timeline();
    assert_eq!((t.tick, t.verified_tick, t.first_tick, t.last_tick), (70, 70, 0, 70));
    assert_eq!(&*t.keyframes, &[0, 20, 40, 60]);
    assert_eq!(t.checksum, s.checksum_at(70).unwrap());
    assert_eq!(t.recent_checksums.len(), orr_session::TIMELINE_CHECKSUM_WINDOW);
    assert_eq!(t.recent_checksums.last().copied(), Some((70, t.checksum)));
    s.control(ControlOp::Seek(45));
    let t = s.timeline();
    assert_eq!((t.tick, t.last_tick), (45, 70));
    assert_eq!(t.checksum, s.checksum_at(45).unwrap());
    // A branch drops the keyframes of the dropped future.
    s.control(ControlOp::Branch);
    assert_eq!(&*s.timeline().keyframes, &[0, 20, 40]);
}

#[test]
fn keyframes_are_thinned_within_the_budget() {
    let mut cfg = config();
    cfg.keyframe_interval = 5;
    cfg.keyframe_budget_bytes = 4_000;
    let mut s = PlaySession::<Arena>::new(cfg, ArenaConfig { player_count: 2 });
    play(&mut s, 400, 1);
    let kf = s.timeline().keyframes.clone();
    assert!(kf.len() < 80, "thinned: {} keyframes", kf.len());
    assert_eq!(kf[0], 0);
    // Seeking still works, from the sparser keyframes.
    let expected = straight(400, 1);
    for &t in &[399u64, 200, 77, 1, 350, 5] {
        s.control(ControlOp::Seek(t));
        assert_eq!(s.frame().checksum(), expected[t as usize - 1], "seek to {t}");
    }
}

#[test]
fn replay_file_is_format_version_3() {
    let mut s = new_session();
    play(&mut s, 10, 1);
    let bytes = s.save_replay();
    assert_eq!(&bytes[..4], b"ORRP");
    assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 3);
}
