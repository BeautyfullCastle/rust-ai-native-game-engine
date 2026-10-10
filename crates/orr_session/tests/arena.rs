//! End-to-end tests for `orr_session` using the `orr_testgame` arena game:
//! two-peer loopback prediction/rollback, event reconciliation, replay
//! round-trip, a golden checksum, and stall-on-excess-latency.
use std::collections::{BTreeMap, BTreeSet};

use orr_fp::FP;
use orr_sim::{PlayerSlot, Simulation, TickInputs};
use orr_session::{
    AdvanceResult, EventStatus, LoopbackNetwork, ReplayHeader, ReplayWriter, Session, SessionConfig,
};
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd, FIRE};

/// A small deterministic pseudo-random input pattern, driven by a
/// `FrameRng` outside the simulation (fine — this is test/view-layer code,
/// not sim code) so both peers script the *same* sequence of "player
/// intent" without needing real input devices.
fn scripted_input(rng: &mut orr_fp::FrameRng, slot: u8, tick: u64) -> ArenaInput {
    let ax = rng.range_i32(-1, 2);
    let ay = rng.range_i32(-1, 2);
    let fire = rng.next_u32() % 5 == 0 && tick % 3 == (slot as u64 % 3);
    ArenaInput::new(FP::from_int(ax), FP::from_int(ay), fire)
}

fn commands_for(input: ArenaInput, slot: u8) -> Vec<SpawnBulletCmd> {
    if input.buttons & FIRE != 0 {
        vec![SpawnBulletCmd { owner: slot as u32 }]
    } else {
        Vec::new()
    }
}

fn tagged_commands(input: ArenaInput, slot: u8) -> Vec<(PlayerSlot, SpawnBulletCmd)> {
    commands_for(input, slot).into_iter().map(|c| (PlayerSlot(slot), c)).collect()
}

fn base_cfg(local: PlayerSlot) -> SessionConfig {
    SessionConfig::new(2, local, 42, 60)
}

/// Steps a plain headless `Simulation` through `1..=up_to` using the same
/// scripted input table both sessions were fed, for cross-checking.
fn headless_checksum_at(scripted: &BTreeMap<u64, (ArenaInput, ArenaInput)>, seed: u64, up_to: u64) -> u64 {
    let mut sim = Simulation::<Arena>::new(ArenaConfig { player_count: 2 }, 60, seed);
    for tick in 1..=up_to {
        let (ia, ib) = scripted.get(&tick).copied().unwrap_or((ArenaInput::default(), ArenaInput::default()));
        let mut ti = TickInputs::<ArenaInput, SpawnBulletCmd>::new(tick, 2);
        ti.set_input(PlayerSlot(0), ia);
        ti.set_input(PlayerSlot(1), ib);
        let mut cmds = tagged_commands(ia, 0);
        cmds.extend(tagged_commands(ib, 1));
        ti.set_commands(cmds);
        sim.step(&ti);
    }
    sim.checksum()
}

#[test]
fn two_peer_rollback_matches_headless_and_golden() {
    const TICKS: u64 = 2000;
    const SEED: u64 = 42;

    let (end_a, end_b, clock) = LoopbackNetwork::new::<Arena>(4, 1, 777);
    let cfg_a = base_cfg(PlayerSlot(0));
    let cfg_b = base_cfg(PlayerSlot(1));
    let input_delay = cfg_a.input_delay as u64;

    let mut session_a = Session::<Arena, _>::new(ArenaConfig { player_count: 2 }, cfg_a, end_a);
    let mut session_b = Session::<Arena, _>::new(ArenaConfig { player_count: 2 }, cfg_b, end_b);

    let mut rng_a = orr_fp::FrameRng::new(SEED);
    let mut rng_b = orr_fp::FrameRng::new(SEED ^ 0xABCD);
    let mut scripted: BTreeMap<u64, (ArenaInput, ArenaInput)> = BTreeMap::new();

    let mut rollbacks = 0u32;
    let mut checkpoints_a: Vec<(u64, u64)> = Vec::new();
    let mut checkpoints_b: Vec<(u64, u64)> = Vec::new();

    for call in 1..=(TICKS + 40) {
        let target_tick = call + input_delay;
        let ia = scripted_input(&mut rng_a, 0, target_tick);
        let ib = scripted_input(&mut rng_b, 1, target_tick);
        scripted.insert(target_tick, (ia, ib));

        clock.tick();
        let ra = session_a.advance(ia, commands_for(ia, 0));
        let rb = session_b.advance(ib, commands_for(ib, 1));

        if let AdvanceResult::Advanced { rollback: Some(_), .. } = ra {
            rollbacks += 1;
        }
        if let AdvanceResult::Advanced { rollback: Some(_), .. } = rb {
            rollbacks += 1;
        }

        checkpoints_a.extend(session_a.checksums().iter().skip(checkpoints_a.len()).copied());
        checkpoints_b.extend(session_b.checksums().iter().skip(checkpoints_b.len()).copied());
    }

    // 1a. Both sessions' verified checksums agree at every shared checkpoint.
    assert!(!checkpoints_a.is_empty(), "no checkpoints recorded");
    let desyncs = orr_session::compare_checksums(&checkpoints_a, &checkpoints_b);
    assert!(desyncs.is_empty(), "desyncs between peers: {desyncs:?}");

    // 1b. Rollbacks actually occurred (latency 4+jitter > input_delay 2).
    assert!(rollbacks > 0, "expected at least one rollback given latency > input_delay");

    // 1c. Matches a single non-networked `Simulation` fed the confirmed inputs.
    let (last_tick, last_cs) = *checkpoints_a.last().unwrap();
    let reference_cs = headless_checksum_at(&scripted, SEED, last_tick);
    assert_eq!(reference_cs, last_cs, "session A's verified checksum disagrees with headless reference");
}

#[test]
fn event_reconciliation_each_verified_once() {
    let (end_a, end_b, clock) = LoopbackNetwork::new::<Arena>(3, 0, 5);
    let cfg_a = base_cfg(PlayerSlot(0));
    let cfg_b = base_cfg(PlayerSlot(1));
    let mut session_a = Session::<Arena, _>::new(ArenaConfig { player_count: 2 }, cfg_a, end_a);
    let mut session_b = Session::<Arena, _>::new(ArenaConfig { player_count: 2 }, cfg_b, end_b);

    let mut live = BTreeMap::new();
    let mut verified_seen = BTreeSet::new();
    let mut canceled_count = 0;

    let mut rng = orr_fp::FrameRng::new(9001);
    // The last three calls only deliver the remaining in-flight inputs, so every
    // occurrence at the simulated head must close, including replacement predictions.
    for call in 1..=803u64 {
        clock.tick();
        let events = if call <= 800 {
            let target = call + 2;
            let ia = scripted_input(&mut rng, 0, target);
            let ib = scripted_input(&mut rng, 1, target);
            let ra = session_a.advance(ia, commands_for(ia, 0));
            let _ = session_b.advance(ib, commands_for(ib, 1));
            match ra {
                AdvanceResult::Advanced { events, .. } | AdvanceResult::Stalled { events } => events,
            }
        } else {
            session_a.poll_confirmed().0
        };
        for (key, status) in events.iter() {
            assert!(!verified_seen.contains(key), "event {key:?} was announced after verification: {status:?}");
            match status {
                EventStatus::Predicted(payload) => {
                    assert!(live.insert(*key, *payload).is_none(), "event {key:?} was predicted while still live");
                }
                EventStatus::Verified(payload) => {
                    let prediction = live.remove(key).expect("verification must close a live prediction");
                    assert_eq!(prediction, *payload, "event {key:?} verified a different payload");
                    verified_seen.insert(*key);
                }
                EventStatus::Canceled => {
                    assert!(live.remove(key).is_some(), "event {key:?} was canceled without a live prediction");
                    canceled_count += 1;
                }
            }
        }
    }

    assert_eq!(session_a.verified_tick(), session_a.head_tick(), "all simulated ticks must be confirmed");
    assert!(live.is_empty(), "predicted occurrences were never verified nor canceled: {live:?}");
    assert!(canceled_count > 0, "expected at least one canceled occurrence");
    assert!(!verified_seen.is_empty(), "expected at least one Hit event to occur over 800 ticks");
}

#[test]
fn replay_round_trip_and_verify() {
    const TICKS: u64 = 300;
    let mut sim = Simulation::<Arena>::new(ArenaConfig { player_count: 2 }, 60, 123);
    let mut writer = ReplayWriter::<Arena>::new(ReplayHeader {
        format_version: 1,
        game_id: "arena".to_string(),
        build_hash: 0xdead_beef,
        seed: 123,
        player_count: 2,
        tick_rate: 60,
        input_size: std::mem::size_of::<ArenaInput>() as u32,
    });

    let mut rng = orr_fp::FrameRng::new(555);
    for tick in 1..=TICKS {
        let ia = scripted_input(&mut rng, 0, tick);
        let ib = scripted_input(&mut rng, 1, tick);
        let mut ti = TickInputs::<ArenaInput, SpawnBulletCmd>::new(tick, 2);
        ti.set_input(PlayerSlot(0), ia);
        ti.set_input(PlayerSlot(1), ib);
        let mut cmds = tagged_commands(ia, 0);
        cmds.extend(tagged_commands(ib, 1));
        ti.set_commands(cmds.clone());
        sim.step(&ti);
        writer.record_tick(tick, &[ia, ib], &cmds);
        if tick % 30 == 0 {
            writer.record_checksum(tick, sim.checksum());
        }
    }

    let bytes = writer.finish();
    let report = orr_session::replay_verify::<Arena>(&bytes, ArenaConfig { player_count: 2 }).expect("parse/verify");
    assert!(report.ok(), "replay verify mismatch: {:?}", report.mismatch);
    assert_eq!(report.checksums_checked, TICKS as u32 / 30);
    assert_eq!(report.ticks_simulated, TICKS);

    eprintln!("replay file size for {TICKS} ticks, 2 players: {} bytes", bytes.len());
    assert!(bytes.len() < 200_000, "replay file suspiciously large: {} bytes", bytes.len());
}

/// Golden checksum: a fixed 1000-tick local (single `Simulation`, no
/// networking) scripted run must always produce this exact checksum. If
/// this test fails after a legitimate engine change, the constant below
/// must be updated deliberately (and the reason recorded), not silently.
// ORRF v2 hashes the complete frame body; this is an intentional checksum
// migration, not a change to Arena's scripted run. See docs/frame-compatibility.md.
const GOLDEN_CHECKSUM_1000: u64 = 0xb41f0d35815cd605;

#[test]
fn golden_1000_tick_checksum() {
    let mut sim = Simulation::<Arena>::new(ArenaConfig { player_count: 2 }, 60, 99);
    let mut rng = orr_fp::FrameRng::new(31337);
    for tick in 1..=1000u64 {
        let ia = scripted_input(&mut rng, 0, tick);
        let ib = scripted_input(&mut rng, 1, tick);
        let mut ti = TickInputs::<ArenaInput, SpawnBulletCmd>::new(tick, 2);
        ti.set_input(PlayerSlot(0), ia);
        ti.set_input(PlayerSlot(1), ib);
        let mut cmds = tagged_commands(ia, 0);
        cmds.extend(tagged_commands(ib, 1));
        ti.set_commands(cmds);
        sim.step(&ti);
    }
    let actual = sim.checksum();
    if actual != GOLDEN_CHECKSUM_1000 {
        eprintln!("golden checksum mismatch: expected {GOLDEN_CHECKSUM_1000:#x}, got {actual:#x}");
    }
    assert_eq!(actual, GOLDEN_CHECKSUM_1000, "golden checksum changed — update deliberately if this is expected");
}

#[test]
fn build_hash_mismatch_rejected() {
    const TICKS: u64 = 50;
    const BUILD_ID: u64 = 0xC0FFEE_u64;

    let mut sim = Simulation::<Arena>::with_build_id(ArenaConfig { player_count: 2 }, 60, 321, BUILD_ID);
    let mut writer = ReplayWriter::<Arena>::new(ReplayHeader {
        format_version: 1,
        game_id: "arena".to_string(),
        build_hash: sim.build_hash(),
        seed: 321,
        player_count: 2,
        tick_rate: 60,
        input_size: std::mem::size_of::<ArenaInput>() as u32,
    });

    let mut rng = orr_fp::FrameRng::new(7);
    for tick in 1..=TICKS {
        let ia = scripted_input(&mut rng, 0, tick);
        let ib = scripted_input(&mut rng, 1, tick);
        let mut ti = TickInputs::<ArenaInput, SpawnBulletCmd>::new(tick, 2);
        ti.set_input(PlayerSlot(0), ia);
        ti.set_input(PlayerSlot(1), ib);
        let cmds = {
            let mut c = tagged_commands(ia, 0);
            c.extend(tagged_commands(ib, 1));
            c
        };
        ti.set_commands(cmds.clone());
        sim.step(&ti);
        writer.record_tick(tick, &[ia, ib], &cmds);
        writer.record_checksum(tick, sim.checksum());
    }
    let bytes = writer.finish();

    // Same build id the replay was recorded with: verification proceeds
    // and matches (build_hash is a pure function of (build_id,
    // patch_generation), so a freshly built `Simulation` with the same
    // build id reports the same hash `replay_verify_checked` compares
    // against).
    let ok = orr_session::replay_verify_checked::<Arena>(&bytes, ArenaConfig { player_count: 2 }, BUILD_ID)
        .expect("same build id must verify without error");
    assert!(ok.ok(), "replay checksum mismatch under matching build id: {:?}", ok.mismatch);

    // Different build id: rejected immediately, before any resimulation.
    let err = orr_session::replay_verify_checked::<Arena>(&bytes, ArenaConfig { player_count: 2 }, 0xDEAD_u64)
        .expect_err("mismatched build id must be rejected");
    match err {
        orr_session::ReplayError::BuildHashMismatch { header, expected } => {
            assert_eq!(header, sim.build_hash());
            assert_ne!(expected, header);
        }
        other => panic!("expected BuildHashMismatch, got {other:?}"),
    }

    // `require_same_build_hash` directly, as two sessions would use it in
    // a connection handshake before trusting each other's confirmed
    // input: agreeing build ids/patch generations pass, disagreeing ones
    // (including after a simulated hot patch bumps `patch_generation`) are
    // rejected.
    let mut peer_sim = Simulation::<Arena>::with_build_id(ArenaConfig { player_count: 2 }, 60, 321, BUILD_ID);
    assert!(orr_session::require_same_build_hash(sim.build_hash(), peer_sim.build_hash()).is_ok());
    peer_sim.on_patch_applied();
    assert!(orr_session::require_same_build_hash(sim.build_hash(), peer_sim.build_hash()).is_err());
}

#[test]
fn stall_when_latency_exceeds_max_prediction() {
    // latency 20 ticks, max_prediction 8: session A must stall waiting for
    // B's confirmations to catch up.
    let (end_a, end_b, clock) = LoopbackNetwork::new::<Arena>(20, 0, 1);
    let mut cfg_a = base_cfg(PlayerSlot(0));
    cfg_a.max_prediction = 8;
    cfg_a.input_delay = 2;
    let mut cfg_b = base_cfg(PlayerSlot(1));
    cfg_b.max_prediction = 8;
    cfg_b.input_delay = 2;

    let mut session_a = Session::<Arena, _>::new(ArenaConfig { player_count: 2 }, cfg_a, end_a);
    let mut session_b = Session::<Arena, _>::new(ArenaConfig { player_count: 2 }, cfg_b, end_b);

    let mut stalled = false;
    for _ in 1..=60u64 {
        clock.tick();
        let ra = session_a.advance(ArenaInput::default(), Vec::new());
        let _ = session_b.advance(ArenaInput::default(), Vec::new());
        if matches!(ra, AdvanceResult::Stalled { .. }) {
            stalled = true;
        }
    }
    assert!(stalled, "expected session A to stall given latency 20 > max_prediction 8");
}

/// Records `ticks` scripted ticks and returns the file bytes plus the
/// per-tick checksums of the straight-through run.
fn record_replay(ticks: u64, keyframe_interval: u64) -> (Vec<u8>, BTreeMap<u64, u64>) {
    let mut sim = Simulation::<Arena>::new(ArenaConfig { player_count: 2 }, 60, 808);
    let mut writer = ReplayWriter::<Arena>::new(ReplayHeader {
        format_version: 1,
        game_id: "arena".to_string(),
        build_hash: 0,
        seed: 808,
        player_count: 2,
        tick_rate: 60,
        input_size: std::mem::size_of::<ArenaInput>() as u32,
    })
    .with_keyframe_interval(keyframe_interval);

    let mut rng = orr_fp::FrameRng::new(4242);
    let mut checksums = BTreeMap::new();
    checksums.insert(0, sim.checksum());
    for tick in 1..=ticks {
        let ia = scripted_input(&mut rng, 0, tick);
        let ib = scripted_input(&mut rng, 1, tick);
        let mut ti = TickInputs::<ArenaInput, SpawnBulletCmd>::new(tick, 2);
        ti.set_input(PlayerSlot(0), ia);
        ti.set_input(PlayerSlot(1), ib);
        let mut cmds = tagged_commands(ia, 0);
        cmds.extend(tagged_commands(ib, 1));
        ti.set_commands(cmds.clone());
        sim.step(&ti);
        writer.record_tick(tick, &[ia, ib], &cmds);
        writer.maybe_record_keyframe(sim.frame());
        checksums.insert(tick, sim.checksum());
    }
    (writer.finish(), checksums)
}

#[test]
fn replay_seek_matches_playing_from_start() {
    const TICKS: u64 = 400;
    let (bytes, checksums) = record_replay(TICKS, 50);
    let reader = orr_session::ReplayReader::<Arena>::parse(&bytes).expect("parse");
    assert_eq!(reader.keyframe_count(), 8);
    assert_eq!(reader.nearest_keyframe(49), None);
    assert_eq!(reader.nearest_keyframe(50), Some(50));
    assert_eq!(reader.nearest_keyframe(199), Some(150));

    // Keyframe ticks, ticks just around them, and both ends; backwards and
    // forwards, to show seeking is independent of any earlier position.
    for target in [0, 1, 49, 50, 51, 199, 200, 399, 400, 120, 10, 400, 0] {
        let sim = reader.seek(ArenaConfig { player_count: 2 }, target).expect("seek");
        assert_eq!(sim.tick(), target);
        assert_eq!(sim.checksum(), checksums[&target], "seek to tick {target} diverged from playing from start");
    }

    // A seeked simulation keeps simulating identically to a fresh one: seek
    // to a keyframe, then step with the recorded inputs and compare.
    let mut from_key = reader.seek(ArenaConfig { player_count: 2 }, 250).unwrap();
    let full = reader.seek(ArenaConfig { player_count: 2 }, 260).unwrap();
    let mut rng = orr_fp::FrameRng::new(4242);
    for tick in 1..=260u64 {
        let ia = scripted_input(&mut rng, 0, tick);
        let ib = scripted_input(&mut rng, 1, tick);
        if tick > 250 {
            let mut ti = TickInputs::<ArenaInput, SpawnBulletCmd>::new(tick, 2);
            ti.set_input(PlayerSlot(0), ia);
            ti.set_input(PlayerSlot(1), ib);
            let mut cmds = tagged_commands(ia, 0);
            cmds.extend(tagged_commands(ib, 1));
            ti.set_commands(cmds);
            from_key.step(&ti);
        }
    }
    assert_eq!(from_key.checksum(), full.checksum());

    assert!(matches!(
        reader.seek(ArenaConfig { player_count: 2 }, TICKS + 1),
        Err(orr_session::ReplayError::TickOutOfRange { tick: 401, last: 400 })
    ));
}

#[test]
fn replay_without_keyframes_seeks_from_start_and_verifies() {
    let (bytes, checksums) = record_replay(120, 0);
    let reader = orr_session::ReplayReader::<Arena>::parse(&bytes).unwrap();
    assert_eq!(reader.keyframe_count(), 0);
    let sim = reader.seek(ArenaConfig { player_count: 2 }, 77).unwrap();
    assert_eq!(sim.checksum(), checksums[&77]);
}

#[test]
fn keyframes_add_to_file_size() {
    let (with_keys, _) = record_replay(400, 50);
    let (without, _) = record_replay(400, 0);
    eprintln!("400 ticks: {} bytes with keyframes every 50, {} without", with_keys.len(), without.len());
    assert!(with_keys.len() > without.len());
}

#[test]
fn v1_and_v2_replays_still_parse_and_seek() {
    // Turn a v3 file without keyframes or debug commands into the older
    // layouts: v2 has no trailing debug count, v1 has no keyframe count either.
    let (v3, checksums) = record_replay(60, 0);
    let header_len = 4 + 4 + (4 + "arena".len()) + 8 + 8 + 1 + 4 + 4;
    let body_len = u32::from_le_bytes(v3[header_len..header_len + 4].try_into().unwrap()) as usize;
    let compressed = &v3[header_len + 4..header_len + 4 + body_len];
    let body = lz4_flex::block::decompress_size_prepended(compressed).unwrap();
    assert_eq!(&body[body.len() - 8..], &[0; 8], "expected empty keyframe and debug tables");

    for (version, cut) in [(2u32, 4usize), (1u32, 8usize)] {
        let recompressed = lz4_flex::block::compress_prepend_size(&body[..body.len() - cut]);
        let mut old = v3[..header_len].to_vec();
        old[4..8].copy_from_slice(&version.to_le_bytes());
        old.extend_from_slice(&(recompressed.len() as u32).to_le_bytes());
        old.extend_from_slice(&recompressed);

        let reader = orr_session::ReplayReader::<Arena>::parse(&old).expect("old version parses");
        assert_eq!(reader.header.format_version, version);
        assert_eq!(reader.keyframe_count(), 0);
        let sim = reader.seek(ArenaConfig { player_count: 2 }, 60).unwrap();
        assert_eq!(sim.checksum(), checksums[&60]);
    }

    // Unknown future versions are still rejected.
    let mut v4 = v3.clone();
    v4[4..8].copy_from_slice(&4u32.to_le_bytes());
    assert!(matches!(
        orr_session::ReplayReader::<Arena>::parse(&v4),
        Err(orr_session::ReplayError::UnsupportedVersion(4))
    ));
}

#[test]
fn replay_seek_checked_enforces_build_hash() {
    const BUILD_ID: u64 = 0xC0FFEE_u64;
    let mut sim = Simulation::<Arena>::with_build_id(ArenaConfig { player_count: 2 }, 60, 5, BUILD_ID);
    let mut writer = ReplayWriter::<Arena>::new(ReplayHeader {
        format_version: 2,
        game_id: "arena".to_string(),
        build_hash: sim.build_hash(),
        seed: 5,
        player_count: 2,
        tick_rate: 60,
        input_size: std::mem::size_of::<ArenaInput>() as u32,
    })
    .with_keyframe_interval(10);
    for tick in 1..=30u64 {
        let ti = TickInputs::<ArenaInput, SpawnBulletCmd>::new(tick, 2);
        sim.step(&ti);
        writer.record_tick(tick, &[ArenaInput::default(), ArenaInput::default()], &[]);
        writer.maybe_record_keyframe(sim.frame());
    }
    let bytes = writer.finish();

    let ok = orr_session::replay_seek_checked::<Arena>(&bytes, ArenaConfig { player_count: 2 }, BUILD_ID, 25)
        .expect("matching build id seeks");
    assert_eq!(ok.tick(), 25);
    let err = orr_session::replay_seek_checked::<Arena>(&bytes, ArenaConfig { player_count: 2 }, 0xDEAD, 25)
        .err()
        .expect("mismatched build id must be rejected");
    assert!(matches!(err, orr_session::ReplayError::BuildHashMismatch { .. }));
}
