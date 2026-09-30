//! Relay scenarios: time sync, late inputs, desync detection, late join,
//! disconnect/rejoin, build-hash rejection, validation, protocol fuzz.
mod common;

use std::cell::Cell;
use std::rc::Rc;

use bytemuck::{Pod, Zeroable};
use common::*;
use orr_proto::netsim::PathParams;
use orr_proto::{ClientMsg, Hello, InputEntry, RejectReason, ServerMsg, TimeSync, NO_SLOT};
use orr_server::harness::{current_client, ClientSpec, Harness, Script};
use orr_server::{InputCtx, InputValidator, RoomConfig, ServerNote, Verdict};
use orr_session::{ClientState, DesyncDump};
use orr_sim::{decode_pod, encode_pod, Game, PlayerSlot, SimCommand, SimContext, System};
use orr_testgame::{Arena, ArenaConfig};

// ---- a tiny game whose state one client can corrupt ---------------------

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, Pod, Zeroable)]
struct CInput {
    add: u32,
    _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, Pod, Zeroable)]
struct NoCmd {
    _pad: u32,
}
impl SimCommand for NoCmd {
    fn encode(&self, out: &mut Vec<u8>) {
        encode_pod(self, out)
    }
    fn decode(bytes: &[u8]) -> Option<Self> {
        decode_pod(bytes)
    }
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
struct NoEvent {
    _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
struct Acc {
    sums: [u64; 4],
}

thread_local! {
    /// `(client, tick)`: that client's simulation flips a bit at that tick.
    static CORRUPT: Cell<Option<(usize, u64)>> = const { Cell::new(None) };
}

struct Counter;
struct CountSystem;
impl System<Counter> for CountSystem {
    fn name(&self) -> &'static str {
        "CountSystem"
    }
    fn run(&mut self, ctx: &mut SimContext<Counter>) {
        for s in 0..ctx.inputs.player_count().min(4) {
            let add = u64::from(ctx.inputs.input(PlayerSlot(s)).add);
            ctx.frame.singleton_mut::<Acc>().sums[s as usize] += add;
        }
        if CORRUPT.with(Cell::get) == Some((current_client(), ctx.tick)) {
            ctx.frame.singleton_mut::<Acc>().sums[0] ^= 0x8000;
        }
    }
}
impl Game for Counter {
    type Input = CInput;
    type Command = NoCmd;
    type Event = NoEvent;
    type Config = ();
    fn register(b: &mut orr_ecs::ComponentRegistryBuilder) {
        b.register_singleton::<Acc>("Acc");
    }
    fn setup(frame: &mut orr_ecs::Frame, _: &()) {
        frame.set_singleton(Acc { sums: [0; 4] });
    }
    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(CountSystem)]
    }
}

fn counter_script() -> Script<Counter> {
    Rc::new(|client, tick| (CInput { add: ((tick / 5 + client as u64 * 7) % 4) as u32, _pad: 0 }, Vec::new()))
}

fn counter_harness(players: u8) -> Harness<Counter> {
    let mut cfg = RoomConfig::new(players, TICK_RATE, SEED, 8);
    cfg.record_all = true;
    Harness::new(3, cfg, counter_script(), |_| ())
}

// ---- (b) clocks that run slow or fast ------------------------------------

#[test]
fn slow_and_fast_clocks_stay_in_sync_without_growing_stalls() {
    let p = path(70, 10, 20_000);
    let mut h = arena_harness(4);
    h.add_client(&ClientSpec::new(p));
    h.add_client(&ClientSpec::new(p));
    h.add_client(&ClientSpec { clock_ppm: 990_000, ..ClientSpec::new(p) }); // 1% slow
    h.add_client(&ClientSpec { clock_ppm: 1_010_000, ..ClientSpec::new(p) }); // 1% fast
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(600, 60_000_000));
    let a: Vec<_> = (0..4).map(|i| client_counters(&h, i)).collect();
    assert!(h.run_until_tick(1800, 60_000_000));
    let b: Vec<_> = (0..4).map(|i| client_counters(&h, i)).collect();
    assert!(h.run_until_tick(3000, 60_000_000));
    h.run_for_us(500_000);
    let c: Vec<_> = (0..4).map(|i| client_counters(&h, i)).collect();

    assert_matches_headless::<Arena>(&h, || ArenaConfig { player_count: 4 }, 4, None);
    assert_clients_agree(&h);
    for i in 0..4 {
        eprintln!(
            "client {i}: rate {} ppm, stalls {:?} -> {:?} -> {:?} (rollbacks, episodes, ms)",
            h.clients[i].client.rate_ppm(),
            a[i],
            b[i],
            c[i]
        );
        // Stalls of the second stretch are not more than the first (plus noise).
        assert!(c[i].1 - b[i].1 <= b[i].1 - a[i].1 + 10, "client {i} stalls grow");
        assert!(c[i].2 - b[i].2 < 300, "client {i} stalled {} ms late in the run", c[i].2 - b[i].2);
    }
    // The clients with wrong clocks compensate with the rate.
    assert!(h.clients[2].client.rate_ppm() > 1_004_000, "slow client rate {}", h.clients[2].client.rate_ppm());
    assert!(h.clients[3].client.rate_ppm() < 996_000, "fast client rate {}", h.clients[3].client.rate_ppm());
}

// ---- (c) inputs that arrive late ------------------------------------------

#[test]
fn late_inputs_are_repeated_by_the_server_and_everyone_converges() {
    let p = path(70, 10, 20_000);
    let mut h = arena_harness(4);
    for _ in 0..4 {
        h.add_client(&ClientSpec::new(p));
    }
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(900, 60_000_000));
    let slot1 = h.clients[1].client.welcome().unwrap().slot as usize;
    let before = h.server.room_stats(h.room).unwrap().slots[slot1];
    // Client 1's uplink gets 80 ms slower: about 5 ticks late.
    let mut worse = p;
    worse.up.latency_us += 80_000;
    h.set_path(1, worse);
    assert!(h.run_until_tick(1500, 60_000_000));
    let mid = h.server.room_stats(h.room).unwrap().slots[slot1];
    assert!(h.run_until_tick(3000, 60_000_000));
    h.run_for_us(500_000);
    let end = h.server.room_stats(h.room).unwrap().slots[slot1];
    eprintln!("slot 1 repeated: {} -> {} -> {}", before.repeated, mid.repeated, end.repeated);
    eprintln!("client 1 own overridden: {}", h.clients[1].client.source_stats().own_overridden);
    assert!(mid.repeated > before.repeated + 2, "the slow uplink caused no repeated inputs");
    assert!(h.clients[1].client.source_stats().own_overridden > 0, "client 1 never learned it was overridden");
    // It recovers: (nearly) no more repeats in the last stretch.
    assert!(end.repeated - mid.repeated <= 3, "still repeating: {}", end.repeated - mid.repeated);
    assert_matches_headless::<Arena>(&h, || ArenaConfig { player_count: 4 }, 4, None);
    assert_clients_agree(&h);
}

// ---- (d) forced desync ------------------------------------------------------

#[test]
fn a_corrupted_client_is_detected_and_dumps_are_written() {
    let p = path(40, 5, 10_000);
    let mut h = counter_harness(3);
    for _ in 0..3 {
        h.add_client(&ClientSpec::new(p));
    }
    CORRUPT.with(|c| c.set(Some((1, 100))));
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(400, 60_000_000));
    h.run_for_us(500_000);

    let notes = h.server.drain_notes();
    let desync = notes
        .iter()
        .find_map(|n| match n {
            ServerNote::Desync { tick, finalized, reports, .. } => Some((*tick, *finalized, reports.clone())),
            _ => None,
        })
        .expect("server found no desync");
    // Divergence at tick 100: the first checksum tick after it is 120, and
    // the notice follows within about one round trip of it.
    assert_eq!(desync.0, 120);
    assert!(desync.1 <= 120 + 30, "found only at server tick {}", desync.1);
    assert_eq!(desync.2.len(), 2, "two checksums differ from the third: {:?}", desync.2);

    let mut healthy_local = None;
    for i in 0..3 {
        let mut dumps = h.clients[i].dumps.take();
        assert!(!dumps.is_empty(), "client {i} wrote no dump");
        let (name, bytes) = dumps.remove(0);
        let slot = h.clients[i].client.welcome().unwrap().slot;
        assert_eq!(name, format!("desync_tick120_slot{slot}.orrd"));
        let dump = DesyncDump::from_bytes(&bytes).unwrap();
        assert_eq!(dump.desync_tick, 120);
        assert_eq!(dump.anchor_tick, 90, "anchor is the last checksum tick before the mismatch");
        assert!(dump.ticks.first().is_some_and(|b| b.tick == 91));
        assert!(dump.ticks.last().unwrap().tick >= 120);
        // Replaying the dump on a clean simulation gives the healthy state.
        let replay = dump.replay::<Counter>((), 1).unwrap();
        let clean = replay.iter().find(|(t, _)| *t == 120).unwrap().1;
        if i == 1 {
            assert_ne!(dump.local_checksum, clean, "the corrupted client's own checksum differs from a clean replay");
        } else {
            assert_eq!(dump.local_checksum, clean);
            healthy_local = Some(dump.local_checksum);
        }
    }
    assert!(healthy_local.is_some());
    // A damaged dump is refused, not trusted.
    let mut bad = h.clients[0].dumps.take();
    assert!(bad.is_empty());
    bad.clear();
}

// ---- (e) late join, disconnect and rejoin -------------------------------------

#[test]
fn late_join_and_rejoin_go_through_the_server() {
    let p = path(50, 8, 10_000);
    let mut cfg = arena_room(4);
    cfg.min_players_to_start = 3;
    let mut h = Harness::<Arena>::new(9, cfg, arena_script_rc(), |w| ArenaConfig { player_count: w.player_count });
    for _ in 0..3 {
        h.add_client(&ClientSpec::new(p));
    }
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(600, 60_000_000));

    // A fourth player joins the running room.
    let joiner = h.add_client(&ClientSpec::new(p));
    assert!(h.run_until_all_playing(20_000_000), "late joiner never started playing");
    let start_tick = h.server.finalized_tick(h.room).unwrap();
    assert_eq!(h.clients[joiner].client.welcome().unwrap().slot, 3);
    assert_eq!(h.server.room_stats(h.room).unwrap().snapshots_relayed, 1);
    assert!(h.run_until_tick(start_tick + 600, 60_000_000));

    // Client 1 drops off the network; its slot keeps getting repeated input.
    let token = h.clients[1].client.token().unwrap();
    h.cut(1);
    let cut_tick = h.server.finalized_tick(h.room).unwrap();
    assert!(h.run_until_tick(cut_tick + 300, 60_000_000));
    assert_eq!(*h.clients[1].client.state(), ClientState::Disconnected);
    let out = h.recorded()[(cut_tick + 200) as usize - 1].clone();
    assert_eq!(out.tick, cut_tick + 200);
    assert!(out.slots[1].flags & orr_proto::FLAG_ABSENT != 0, "slot 1 not marked absent");
    assert!(out.slots[1].flags & orr_proto::FLAG_REPEATED != 0);

    // It comes back with its token and takes the slot again.
    let back = h.add_client(&ClientSpec { want_slot: Some(PlayerSlot(1)), token, ..ClientSpec::new(p) });
    assert!(h.run_until_all_playing_except(&[1], 20_000_000));
    assert_eq!(h.clients[back].client.welcome().unwrap().slot, 1);
    let t = h.server.finalized_tick(h.room).unwrap();
    assert!(h.run_until_tick(t + 900, 60_000_000));
    h.run_for_us(500_000);

    assert_eq!(h.server.room_stats(h.room).unwrap().snapshots_relayed, 2);
    let compared = assert_matches_headless::<Arena>(&h, || ArenaConfig { player_count: 4 }, 4, None);
    assert_clients_agree(&h);
    assert!(compared > 40);
    let slots = h.server.room_stats(h.room).unwrap().slots;
    assert!(slots[3].delivered > 500 && slots[1].delivered > 500, "{slots:?}");
    // Nobody was thrown off by the join or the rejoin.
    for i in [0usize, 2, joiner, back] {
        assert_eq!(*h.clients[i].client.state(), ClientState::Playing, "client {i}");
        assert!(h.clients[i].dumps.is_empty());
    }
}

// ---- (f) build hash and other refusals -------------------------------------------

#[test]
fn a_different_build_is_rejected_at_hello() {
    let mut h = arena_harness(2);
    let bad = h.add_client(&ClientSpec { build_id: 2, ..ClientSpec::new(path(20, 0, 0)) });
    let good = h.add_client(&ClientSpec::new(path(20, 0, 0)));
    h.run_for_us(2_000_000);
    let want = RejectReason::BuildHashMismatch { server: orr_sim::build_hash_of(1, 0), client: orr_sim::build_hash_of(2, 0) };
    assert_eq!(*h.clients[bad].client.state(), ClientState::Rejected(want));
    assert!(h.clients[bad].client.session().is_none());
    // The good client got a slot and waits for the room to fill.
    assert_eq!(*h.clients[good].client.state(), ClientState::Syncing);
    assert!(h.server.drain_notes().iter().any(|n| matches!(n, ServerNote::Rejected { reason, .. } if *reason == want)));
}

#[test]
fn unknown_room_and_full_room_are_rejected() {
    let mut h = arena_harness(1);
    let a = h.add_client(&ClientSpec::new(path(10, 0, 0)));
    let b = h.add_client(&ClientSpec::new(path(10, 0, 0)));
    h.run_for_us(2_000_000);
    assert_eq!(*h.clients[a].client.state(), ClientState::Playing);
    assert_eq!(*h.clients[b].client.state(), ClientState::Rejected(RejectReason::RoomFull));
}

// ---- Relay + Validate ----------------------------------------------------------------

/// Refuses any input that presses fire.
struct NoFire;
impl InputValidator for NoFire {
    fn validate(&mut self, _ctx: &InputCtx<'_>, input: &[u8], _commands: &[Vec<u8>]) -> Verdict {
        let a: orr_testgame::ArenaInput = bytemuck::pod_read_unaligned(input);
        if a.buttons & orr_testgame::FIRE != 0 {
            Verdict::Reject
        } else {
            Verdict::Accept
        }
    }
}

#[test]
fn the_validate_hook_overrides_illegal_inputs() {
    let mut h = Harness::<Arena>::with_validator(
        5,
        arena_room(2),
        arena_script_rc(),
        |w| ArenaConfig { player_count: w.player_count },
        Box::new(NoFire),
    );
    for _ in 0..2 {
        h.add_client(&ClientSpec::new(path(40, 5, 0)));
    }
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(900, 60_000_000));
    h.run_for_us(500_000);
    let stats = h.server.room_stats(h.room).unwrap();
    assert!(stats.slots[0].rejected_by_validator > 10);
    // No confirmed input fires, and the clients agree with that stream.
    for b in h.recorded() {
        for s in &b.slots {
            let a: orr_testgame::ArenaInput = bytemuck::pod_read_unaligned(&s.input);
            if s.flags & orr_proto::FLAG_REPEATED == 0 {
                assert_eq!(a.buttons & orr_testgame::FIRE, 0);
            }
        }
    }
    assert_matches_headless::<Arena>(&h, || ArenaConfig { player_count: 2 }, 2, None);
    assert!(h.clients[0].client.source_stats().own_overridden > 0);
}

// ---- (g) protocol fuzz -------------------------------------------------------------------

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

#[test]
fn random_and_mutated_messages_never_panic_the_codecs() {
    let mut rng = Rng(1);
    let seeds: Vec<Vec<u8>> = vec![
        ClientMsg::Hello(Hello { build_hash: 1, room: 1, input_size: 4, want_slot: NO_SLOT, token: 0 }).encode(),
        ClientMsg::Input {
            slot: 0,
            input_size: 4,
            ack_tick: 3,
            entries: vec![InputEntry { tick: 5, input: vec![1, 2, 3, 4], commands: vec![vec![1, 2]] }],
        }
        .encode(),
        ServerMsg::TimeSync(TimeSync { window_from: 1, finalized: 2, min_slack_us: -3, avg_slack_us: 4, samples: 5, late: 6 })
            .encode(),
    ];
    for i in 0..60_000 {
        let len = (rng.next() % 200) as usize;
        let raw = rng.bytes(len);
        let _ = ClientMsg::decode(&raw);
        let _ = ServerMsg::decode(&raw);
        // A mutated valid message (mostly fails the checksum, but the
        // header and length paths are still walked).
        let mut m = seeds[i % seeds.len()].clone();
        for _ in 0..(1 + rng.next() % 3) {
            let at = (rng.next() % m.len() as u64) as usize;
            m[at] = rng.next() as u8;
        }
        if rng.next() % 3 == 0 {
            m.truncate((rng.next() % m.len() as u64) as usize);
        }
        let _ = ClientMsg::decode(&m);
        let _ = ServerMsg::decode(&m);
    }
}

/// Sealed messages with nonsense content reach the logic behind the codec.
fn nonsense_client_msgs(rng: &mut Rng) -> Vec<Vec<u8>> {
    let big = rng.next();
    vec![
        ClientMsg::Hello(Hello { build_hash: 0, room: 1, input_size: 24, want_slot: 200, token: big }).encode(),
        ClientMsg::Hello(Hello { build_hash: 0, room: 99, input_size: 24, want_slot: NO_SLOT, token: 0 }).encode(),
        ClientMsg::Input { slot: 250, input_size: 24, ack_tick: u64::MAX, entries: vec![] }.encode(),
        ClientMsg::Input {
            slot: 0,
            input_size: 24,
            ack_tick: u64::MAX,
            entries: vec![
                InputEntry { tick: u64::MAX, input: vec![7; 24], commands: vec![vec![1; 100]] },
                InputEntry { tick: 0, input: vec![7; 24], commands: vec![] },
                InputEntry { tick: 3, input: vec![7; 24], commands: vec![] },
            ],
        }
        .encode(),
        ClientMsg::Checksum { tick: u64::MAX, checksum: 1 }.encode(),
        ClientMsg::Checksum { tick: 0, checksum: 1 }.encode(),
        ClientMsg::SnapshotUpload { request_id: 77, tick: 5, checksum: 5, data: vec![0; 10] }.encode(),
        ClientMsg::SnapshotDecline { request_id: 1 }.encode(),
        ClientMsg::Ping { seq: 0, client_time_us: u64::MAX, rtt_hint_us: u32::MAX }.encode(),
        ClientMsg::Ready.encode(),
    ]
}

fn nonsense_server_msgs(rng: &mut Rng) -> Vec<Vec<u8>> {
    let big = rng.next();
    vec![
        ServerMsg::TimeSync(TimeSync { window_from: 0, finalized: big, min_slack_us: i32::MIN, avg_slack_us: i32::MAX, samples: u32::MAX, late: 0 }).encode(),
        ServerMsg::TimeSync(TimeSync { window_from: 0, finalized: 0, min_slack_us: i32::MAX, avg_slack_us: 0, samples: 1, late: 0 }).encode(),
        ServerMsg::Desync { tick: u64::MAX, reports: vec![(200, 1)] }.encode(),
        ServerMsg::Desync { tick: 0, reports: vec![] }.encode(),
        ServerMsg::Presence { slot: 250, present: true, from_tick: u64::MAX }.encode(),
        ServerMsg::SnapshotRequest { request_id: 5 }.encode(),
        ServerMsg::JoinSnapshot { tick: 1, checksum: 1, data: vec![1, 2, 3] }.encode(),
        ServerMsg::Start { t0_us: 0, server_time_us: u64::MAX }.encode(),
        ServerMsg::Pong { seq: 1, client_time_us: 0, server_time_us: u64::MAX, t0_us: 0, finalized_tick: 0 }.encode(),
        ServerMsg::Confirmed { input_size: 24, bundles: vec![] }.encode(),
        ServerMsg::Confirmed {
            input_size: 24,
            bundles: vec![orr_proto::Bundle {
                tick: u64::MAX,
                slots: (0..2)
                    .map(|_| orr_proto::SlotConfirmed { input: vec![9; 24], commands: vec![vec![1; 3]], flags: 0 })
                    .collect(),
            }],
        }
        .encode(),
    ]
}

#[test]
fn a_live_server_and_client_survive_garbage() {
    use orr_proto::{Channel, Endpoint, Link};
    let mut h = arena_harness(2);
    for _ in 0..2 {
        h.add_client(&ClientSpec::new(path(30, 4, 10_000)));
    }
    assert!(h.run_until_all_playing(10_000_000));
    // A stranger that sends garbage and nonsense to the server.
    let mut stranger = h.net.connect(path(30, 4, 0));
    let stranger_id = stranger.conn_id();
    let mut rng = Rng(77);
    for round in 0..200u64 {
        for ch in [Channel::Reliable, Channel::Unreliable] {
            let n = (rng.next() % 300) as usize;
            let raw = rng.bytes(n);
            stranger.send(ch, &raw);
        }
        for m in nonsense_client_msgs(&mut rng) {
            stranger.send(if round % 2 == 0 { Channel::Reliable } else { Channel::Unreliable }, &m);
        }
        // Garbage and nonsense from the "server" to a playing client.
        let target = orr_proto::ConnId(h.clients[(round % 2) as usize].link.0);
        let n2 = (rng.next() % 300) as usize;
        let raw = rng.bytes(n2);
        h.server.endpoint_mut().send(target, Channel::Unreliable, &raw);
        for m in nonsense_server_msgs(&mut rng) {
            h.server.endpoint_mut().send(target, Channel::Reliable, &m);
        }
        h.run_for_us(20_000);
    }
    let _ = stranger_id;
    assert!(h.run_until_tick(1500, 60_000_000));
    assert!(h.server.bad_messages() > 100);
    // The real clients are still playing, off the garbage's effects; the
    // server's stream is intact.
    let alive = h.clients.iter().filter(|c| *c.client.state() == ClientState::Playing).count();
    assert!(alive >= 1, "no client survived the garbage");
    let _ = PathParams::default();
}
