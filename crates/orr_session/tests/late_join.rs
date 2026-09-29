//! Late join: a third peer joins a running two-peer session from a
//! confirmed snapshot and must stay bit-identical to the others, and to a
//! headless run of the same inputs. Also build-hash and corruption
//! rejection.
use std::collections::BTreeMap;

use orr_fp::{FrameRng, FP};
use orr_session::{
    compare_checksums, join_request, AdvanceResult, InputSource, JoinError, LocalInputSource,
    LoopbackEnd, LoopbackNetwork, PlayerSlot, RemoteInput, Session, SessionConfig,
};
use orr_sim::{Simulation, TickInputs};
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd, FIRE};
use xxhash_rust::xxh3::xxh3_64;

const SEED: u64 = 42;

fn scripted_input(rng: &mut FrameRng, slot: u8, tick: u64) -> ArenaInput {
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

/// A peer's connections to every other peer: sends go to all, receives
/// come from all.
struct Mesh {
    links: Vec<LoopbackEnd<Arena>>,
}

impl InputSource<Arena> for Mesh {
    fn send_local(&mut self, tick: u64, slot: PlayerSlot, input: ArenaInput, commands: Vec<SpawnBulletCmd>) {
        for link in &mut self.links {
            link.send_local(tick, slot, input, commands.clone());
        }
    }
    fn poll_remote(&mut self) -> Vec<RemoteInput<Arena>> {
        self.links.iter_mut().flat_map(|l| l.poll_remote()).collect()
    }
}

type Peer = Session<Arena, Mesh>;

fn cfg3(local: u8) -> SessionConfig {
    let mut cfg = SessionConfig::new(3, PlayerSlot(local), SEED, 60);
    cfg.checksum_interval = 10;
    cfg.build_id = 0xC0FFEE;
    cfg
}

fn arena3() -> ArenaConfig {
    ArenaConfig { player_count: 3 }
}

/// Every input any peer authored, by tick: what a headless run replays.
type Table = BTreeMap<u64, [ArenaInput; 3]>;

/// One `advance` for `slot`: scripted input, or idle while a joiner is
/// still catching up. Records what was authored for the headless run.
fn drive(peer: &mut Peer, slot: u8, rng: &mut FrameRng, table: &mut Table, idle: bool) -> AdvanceResult<Arena> {
    let stamp = peer.next_send_tick();
    let input = if idle { ArenaInput::default() } else { scripted_input(rng, slot, stamp) };
    table.entry(stamp).or_default()[slot as usize] = input;
    peer.advance(input, commands_for(input, slot))
}

fn headless_checksums(table: &Table, up_to: u64) -> BTreeMap<u64, u64> {
    let mut sim = Simulation::<Arena>::new(arena3(), 60, SEED);
    let mut out = BTreeMap::new();
    for tick in 1..=up_to {
        let inputs = table.get(&tick).copied().unwrap_or_default();
        let mut ti = TickInputs::<ArenaInput, SpawnBulletCmd>::new(tick, 3);
        let mut cmds = Vec::new();
        for (slot, &input) in inputs.iter().enumerate() {
            ti.set_input(PlayerSlot(slot as u8), input);
            cmds.extend(commands_for(input, slot as u8).into_iter().map(|c| (PlayerSlot(slot as u8), c)));
        }
        ti.set_commands(cmds);
        sim.step(&ti);
        out.insert(tick, sim.checksum());
    }
    out
}

enum Msg {
    Request(Vec<u8>),
    Notify,
    Snapshot(Vec<u8>),
}

/// Sends every input `peer` authored down `link` before it joins the
/// peer's mesh, so the joiner misses nothing sent before the link existed.
fn attach(peer: &mut Peer, mut link: LoopbackEnd<Arena>) {
    for r in peer.authored_since(0) {
        link.send_local(r.tick, r.slot, r.input, r.commands);
    }
    peer.source_mut().links.push(link);
}

#[test]
fn late_joiner_matches_peers_and_headless() {
    const JOIN_ROUND: u64 = 150;
    const ROUNDS: u64 = 600;
    const LINK_LATENCY: u64 = 3;

    let (end_ab, end_ba, clock) = LoopbackNetwork::new::<Arena>(4, 1, 777);
    let mut cfg_a = cfg3(0);
    cfg_a.vacant_slots = vec![PlayerSlot(2)];
    let cfg_j = cfg3(2);
    let mut a = Session::<Arena, _>::new(arena3(), cfg_a, Mesh { links: vec![end_ab] });
    let mut b = Session::<Arena, _>::new(arena3(), cfg3(1), Mesh { links: vec![end_ba] });
    let mut j: Option<Peer> = None;

    let mut rng = [FrameRng::new(SEED), FrameRng::new(SEED ^ 0xABCD), FrameRng::new(SEED ^ 0x1234)];
    let mut table = Table::new();
    let mut pending: Vec<(u64, Msg)> = Vec::new();
    let mut b_end = None;
    let mut j_ends = None;
    let (mut snapshot_tick, mut first_input_tick) = (0, 0);
    let mut caught_up = false;
    let mut rollbacks_after_join = 0u32;

    for round in 1..=ROUNDS {
        clock.tick();
        if round == JOIN_ROUND {
            pending.push((round + LINK_LATENCY, Msg::Request(join_request(&cfg_j))));
        }

        let (due, later): (Vec<_>, Vec<_>) = pending.drain(..).partition(|(at, _)| *at <= round);
        pending = later;
        for (_, msg) in due {
            match msg {
                Msg::Request(request) => {
                    // The host serves its *verified* frame; the head is
                    // ahead of it (predicted), so a predicted tick would
                    // have been a different, wrong, tick.
                    assert!(a.head_tick() > a.verified_tick());
                    snapshot_tick = a.verified_tick();
                    first_input_tick = a.next_send_tick();
                    let snapshot = a.serve_join(&request).expect("host serves the join");
                    let (a_end, j_a) = LoopbackNetwork::with_clock::<Arena>(&clock, LINK_LATENCY, 1, 11);
                    let (link_b, j_b) = LoopbackNetwork::with_clock::<Arena>(&clock, LINK_LATENCY, 1, 12);
                    attach(&mut a, a_end);
                    b_end = Some(link_b);
                    j_ends = Some(vec![j_a, j_b]);
                    pending.push((round + 2, Msg::Notify));
                    pending.push((round + LINK_LATENCY, Msg::Snapshot(snapshot)));
                }
                Msg::Notify => attach(&mut b, b_end.take().unwrap()),
                Msg::Snapshot(bytes) => {
                    let joiner = Session::<Arena, _>::from_join_snapshot(
                        arena3(),
                        cfg_j.clone(),
                        Mesh { links: j_ends.take().unwrap() },
                        &bytes,
                    )
                    .expect("joiner accepts the snapshot");
                    assert_eq!(joiner.verified_tick(), snapshot_tick);
                    assert_eq!(joiner.head_tick(), snapshot_tick);
                    assert_eq!(joiner.next_send_tick(), first_input_tick);
                    j = Some(joiner);
                }
            }
        }

        let mut results = vec![drive(&mut a, 0, &mut rng[0], &mut table, false)];
        results.push(drive(&mut b, 1, &mut rng[1], &mut table, false));
        if let Some(j) = j.as_mut() {
            if caught_up {
                results.push(drive(j, 2, &mut rng[2], &mut table, false));
            } else {
                // Fast-forward to the others' head, authoring idle input.
                loop {
                    let r = drive(j, 2, &mut rng[2], &mut table, true);
                    let stalled = matches!(r, AdvanceResult::Stalled { .. });
                    results.push(r);
                    if stalled || j.head_tick() >= a.head_tick() {
                        break;
                    }
                }
                caught_up = j.head_tick() >= a.head_tick();
            }
        }
        if j.is_some() {
            rollbacks_after_join +=
                results.iter().filter(|r| matches!(r, AdvanceResult::Advanced { rollback: Some(_), .. })).count() as u32;
        }
    }

    let j = j.expect("joiner joined");
    assert!(caught_up, "joiner never caught up");
    assert!(rollbacks_after_join > 0, "expected rollbacks after the join");

    let (cs_a, cs_b, cs_j) = (a.checksums(), b.checksums(), j.checksums());
    assert!(cs_j.len() >= 20, "joiner recorded too few checksums: {}", cs_j.len());
    assert!(cs_j.iter().all(|&(t, _)| t > snapshot_tick));
    assert!(cs_j.last().unwrap().0 + 60 >= cs_a.last().unwrap().0, "joiner fell behind");
    for other in [cs_a, cs_b] {
        assert!(compare_checksums(cs_j, other).is_empty(), "joiner desynced");
        for &(tick, _) in cs_j {
            assert!(other.iter().any(|&(t, _)| t == tick), "no peer checksum at tick {tick}");
        }
    }

    let last = [cs_a, cs_b, cs_j].iter().map(|c| c.last().unwrap().0).max().unwrap();
    let headless = headless_checksums(&table, last);
    for (name, cs) in [("A", cs_a), ("B", cs_b), ("joiner", cs_j)] {
        for &(tick, checksum) in cs {
            assert_eq!(headless[&tick], checksum, "peer {name} differs from the headless run at tick {tick}");
        }
    }
    eprintln!(
        "late join: snapshot at verified tick {snapshot_tick}, joiner first input tick {first_input_tick}, \
         {} joiner checksums, {rollbacks_after_join} rollbacks after join",
        cs_j.len()
    );
}

/// A host running alone (slot 1 vacant) and a request/snapshot for it.
fn lone_host(build_id: u64) -> Session<Arena, LocalInputSource> {
    let mut cfg = SessionConfig::new(2, PlayerSlot(0), SEED, 60);
    cfg.build_id = build_id;
    cfg.vacant_slots = vec![PlayerSlot(1)];
    let mut host = Session::<Arena, _>::new(ArenaConfig { player_count: 2 }, cfg, LocalInputSource);
    for _ in 0..40 {
        host.advance(ArenaInput::default(), Vec::new());
    }
    assert!(host.verified_tick() > 0);
    host
}

fn joiner_cfg(build_id: u64) -> SessionConfig {
    let mut cfg = SessionConfig::new(2, PlayerSlot(1), SEED, 60);
    cfg.build_id = build_id;
    cfg
}

fn join2(cfg: SessionConfig, msg: &[u8]) -> Result<Session<Arena, LocalInputSource>, JoinError> {
    Session::<Arena, _>::from_join_snapshot(ArenaConfig { player_count: 2 }, cfg, LocalInputSource, msg)
}

#[test]
fn build_hash_mismatch_rejected() {
    let mut host = lone_host(0xC0FFEE);

    // The host refuses a joiner running another build, and stays open.
    let err = host.serve_join(&join_request(&joiner_cfg(0xDEAD))).unwrap_err();
    assert!(matches!(err, JoinError::BuildHashMismatch(_)), "{err}");

    // A joiner refuses a snapshot from a host on another build.
    let snapshot = host.serve_join(&join_request(&joiner_cfg(0xC0FFEE))).expect("same build joins");
    let err = join2(joiner_cfg(0xDEAD), &snapshot).err().expect("wrong build rejected");
    assert!(matches!(err, JoinError::BuildHashMismatch(_)), "{err}");
    let joined = join2(joiner_cfg(0xC0FFEE), &snapshot).expect("matching build accepted");
    assert_eq!(joined.build_hash(), host.build_hash());
    assert_eq!(joined.verified_tick(), joined.head_tick());
    assert_eq!(joined.verified_frame().unwrap().checksum(), host.verified_frame().unwrap().checksum());
}

#[test]
fn setting_mismatch_and_slot_rules() {
    let mut host = lone_host(0xC0FFEE);

    let mut other_seed = joiner_cfg(0xC0FFEE);
    other_seed.seed += 1;
    assert!(matches!(host.serve_join(&join_request(&other_seed)), Err(JoinError::ConfigMismatch("seed"))));

    // Slot 0 is played by the host, not vacant.
    let mut not_vacant = joiner_cfg(0xC0FFEE);
    not_vacant.local_slot = PlayerSlot(0);
    assert!(matches!(host.serve_join(&join_request(&not_vacant)), Err(JoinError::SlotNotVacant(_))));

    // A joiner that asked for slot 1 cannot take the snapshot as slot 0.
    let snapshot = host.serve_join(&join_request(&joiner_cfg(0xC0FFEE))).unwrap();
    assert!(matches!(join2(not_vacant, &snapshot), Err(JoinError::ConfigMismatch("slot"))));

    // The slot is handed out once.
    assert!(matches!(
        host.serve_join(&join_request(&joiner_cfg(0xC0FFEE))),
        Err(JoinError::SlotNotVacant(_))
    ));
}

/// Layout of a snapshot message (see `join.rs`).
const OFF_SNAPSHOT_TICK: usize = 30;
const OFF_CHECKSUM: usize = 46;
const OFF_LEN: usize = 54;
const OFF_PAYLOAD: usize = 58;

/// Fixes the trailing message checksum after a test edited the body.
fn reseal(mut body: Vec<u8>) -> Vec<u8> {
    let sum = xxh3_64(&body);
    body.extend_from_slice(&sum.to_le_bytes());
    body
}

fn body_of(message: &[u8]) -> Vec<u8> {
    message[..message.len() - 8].to_vec()
}

#[test]
fn corrupted_snapshot_rejected() {
    let mut host = lone_host(0xC0FFEE);
    let snapshot = host.serve_join(&join_request(&joiner_cfg(0xC0FFEE))).unwrap();
    join2(joiner_cfg(0xC0FFEE), &snapshot).expect("untouched snapshot is accepted");
    let reject = |msg: &[u8]| join2(joiner_cfg(0xC0FFEE), msg).err().expect("must be rejected");

    // Any single flipped byte, anywhere, trips the message checksum.
    for i in 0..snapshot.len() {
        let mut bad = snapshot.clone();
        bad[i] ^= 0x5A;
        assert!(matches!(reject(&bad), JoinError::BadMessageChecksum), "flip at {i}");
    }

    // Every truncation is rejected.
    for len in 0..snapshot.len() {
        reject(&snapshot[..len]);
    }

    // Damage that a transport checksum would not see (the sender itself
    // is wrong, or the message was re-sealed): the frame checksum and the
    // decoder still catch it.
    let payload_len = u32::from_le_bytes(snapshot[OFF_LEN..OFF_LEN + 4].try_into().unwrap()) as usize;
    let payload = &snapshot[OFF_PAYLOAD..OFF_PAYLOAD + payload_len];

    let mut bad = body_of(&snapshot);
    bad[OFF_CHECKSUM] ^= 1;
    assert!(matches!(reject(&reseal(bad)), JoinError::SnapshotMismatch { .. }));

    let mut bad = body_of(&snapshot);
    bad[OFF_SNAPSHOT_TICK] ^= 1;
    assert!(matches!(reject(&reseal(bad)), JoinError::SnapshotMismatch { .. }));

    // Frame bytes changed, then recompressed: the frame's own checksum fails.
    let mut frame = lz4_flex::block::decompress_size_prepended(payload).unwrap();
    let mid = frame.len() / 2;
    frame[mid] ^= 0xFF;
    let recompressed = lz4_flex::block::compress_prepend_size(&frame);
    let mut bad = snapshot[..OFF_LEN].to_vec();
    bad.extend_from_slice(&(recompressed.len() as u32).to_le_bytes());
    bad.extend_from_slice(&recompressed);
    assert!(matches!(reject(&reseal(bad)), JoinError::BadSnapshot(_)));

    // A forged huge size prefix is refused without allocating it.
    let mut bad = snapshot[..OFF_PAYLOAD].to_vec();
    bad.extend_from_slice(&u32::MAX.to_le_bytes());
    bad.extend_from_slice(&payload[4..]);
    assert!(matches!(reject(&reseal(bad)), JoinError::Decompress(_)));

    // Random garbage, and garbage that carries a valid seal.
    let mut rng = FrameRng::new(0xF00D);
    for _ in 0..500 {
        let len = (rng.next_u32() % 200) as usize;
        let junk: Vec<u8> = (0..len).map(|_| rng.next_u32() as u8).collect();
        reject(&junk);
        reject(&reseal(junk));
    }
}
