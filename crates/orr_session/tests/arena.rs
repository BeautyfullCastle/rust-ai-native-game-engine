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

    let mut predicted_seen: BTreeSet<orr_sim::EventKey> = BTreeSet::new();
    let mut verified_seen: BTreeSet<orr_sim::EventKey> = BTreeSet::new();
    let mut verified_count: BTreeMap<orr_sim::EventKey, u32> = BTreeMap::new();
    let mut canceled: BTreeSet<orr_sim::EventKey> = BTreeSet::new();

    let mut rng = orr_fp::FrameRng::new(9001);
    for call in 1..=800u64 {
        let target = call + 2;
        let ia = scripted_input(&mut rng, 0, target);
        let ib = scripted_input(&mut rng, 1, target);

        clock.tick();
        let ra = session_a.advance(ia, commands_for(ia, 0));
        let _ = session_b.advance(ib, commands_for(ib, 1));

        let events = match ra {
            AdvanceResult::Advanced { events, .. } => events,
            AdvanceResult::Stalled { events } => events,
        };
        for (key, status) in events.iter() {
            match status {
                EventStatus::Predicted(_) => {
                    predicted_seen.insert(*key);
                }
                EventStatus::Verified(_) => {
                    assert!(!canceled.contains(key), "event {key:?} was Canceled and then Verified");
                    *verified_count.entry(*key).or_insert(0) += 1;
                    verified_seen.insert(*key);
                }
                EventStatus::Canceled => {
                    assert!(!verified_seen.contains(key), "event {key:?} was Verified and then Canceled");
                    canceled.insert(*key);
                }
            }
        }
    }

    for (key, count) in &verified_count {
        assert_eq!(*count, 1, "event {key:?} was announced Verified more than once");
    }
    for key in &predicted_seen {
        assert!(
            verified_seen.contains(key) || canceled.contains(key),
            "predicted event {key:?} was never verified nor canceled"
        );
    }
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
const GOLDEN_CHECKSUM_1000: u64 = 0x13cdc3c810d65459;

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
