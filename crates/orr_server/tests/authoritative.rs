//! Authoritative mode on the simulated network (virtual clock, no sockets):
//! the server simulates the confirmed stream, referees the clients'
//! checksums, corrects a diverged client, serves late joiners itself, plays
//! server-driven slots, and kicks cheaters.
//!
//! Waits are progress based (run until the server reaches a tick) with
//! generous virtual-time limits.
#![allow(clippy::float_arithmetic)]
#![allow(clippy::disallowed_types)]

mod common;

use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;

use bytemuck::{Pod, Zeroable};
use common::*;
use orr_fp::FP;
use orr_proto::{FLAG_ABSENT, BYE_KICKED_CHEAT, BYE_KICKED_DESYNC, SERVER_SLOT};
use orr_server::harness::{current_client, ClientSpec, Harness, Script};
use orr_server::presets::arena_sim;
use orr_server::{AuthoritativeConfig, GameSim, InputCtx, InputValidator, RoomConfig, ServerNote, Verdict};
use orr_session::{ClientEvent, ClientState, DesyncDump};
use orr_sim::{decode_pod, encode_pod, Game, PlayerSlot, SimCommand, SimContext, System};
use orr_testgame::{Arena, ArenaConfig, ArenaInput, PlayerTag, Position};

fn arena_cfg(w: &orr_proto::Welcome) -> ArenaConfig {
    ArenaConfig { player_count: w.player_count }
}

fn auth_arena(players: u8, ai: &[u8], auth: AuthoritativeConfig, script: Script<Arena>, seed: u64) -> Harness<Arena> {
    let mut room = arena_room(players);
    room.min_players_to_start = players - ai.len() as u8;
    Harness::authoritative(seed, room, auth, Box::new(arena_sim(players, TICK_RATE, SEED, 1, ai)), script, arena_cfg)
}

fn server_checks(h: &Harness<impl Game>) -> BTreeMap<u64, u64> {
    h.server.server_checksums(h.room).iter().copied().collect()
}

/// Every verified checksum of every client must equal the server's own at
/// that tick (and exist in it). Returns how many were compared.
fn assert_clients_match_server<G: Game>(h: &Harness<G>, from_tick: u64, only: Option<&[usize]>) -> usize {
    let server = server_checks(h);
    let mut compared = 0;
    for (i, c) in h.clients.iter().enumerate() {
        if only.is_some_and(|o| !o.contains(&i)) {
            continue;
        }
        let Some(s) = c.client.session() else { continue };
        for &(tick, cs) in s.checksums() {
            if tick < from_tick {
                continue;
            }
            let want = server.get(&tick).unwrap_or_else(|| panic!("the server has no checksum for tick {tick}"));
            assert_eq!(cs, *want, "client {i} differs from the server at tick {tick}");
            compared += 1;
        }
    }
    compared
}

// ---- (a) a clean run -------------------------------------------------------

#[test]
fn three_clients_with_latency_and_loss_agree_with_the_server() {
    let p = path(40, 6, 15_000);
    let mut h = auth_arena(3, &[], AuthoritativeConfig::default(), arena_script_rc(), 21);
    for _ in 0..3 {
        h.add_client(&ClientSpec::new(p));
    }
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(900, 120_000_000));
    h.run_for_us(500_000);

    for c in &h.clients {
        assert!(c.client.welcome().unwrap().authoritative());
        assert_eq!(c.client.stats().corrections, 0);
        assert_eq!(c.client.stats().desyncs, 0);
        assert_eq!(*c.client.state(), ClientState::Playing);
        assert!(c.dumps.is_empty());
    }
    let compared = assert_clients_match_server(&h, 0, None);
    // 3 clients x about 28 checkpoints.
    assert!(compared >= 60, "only {compared} checkpoints compared");
    // The server's own sim equals a headless replay of its confirmed stream.
    assert_matches_headless::<Arena>(&h, || ArenaConfig { player_count: 3 }, 3, None);
    let st = h.server.room_stats(h.room).unwrap();
    assert_eq!((st.desyncs, st.corrections, st.kicks, st.violations), (0, 0, 0, 0));
    assert!(st.server_ticks >= 900);
    assert!(h.server_dumps.is_empty());
    assert!(!h.server.drain_notes().iter().any(|n| matches!(n, ServerNote::Desync { .. } | ServerNote::Correction { .. })));
    eprintln!("(a) {compared} client checkpoints equal the server's; server simulated {} ticks, no corrections", st.server_ticks);
}

// ---- a tiny game whose state one client can corrupt -------------------------

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
    /// `(client, first tick, every tick after too)`.
    static CORRUPT: Cell<Option<(usize, u64, bool)>> = const { Cell::new(None) };
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
        if let Some((client, from, always)) = CORRUPT.with(Cell::get) {
            if client == current_client() && (ctx.tick == from || (always && ctx.tick > from)) {
                ctx.frame.singleton_mut::<Acc>().sums[0] ^= 0x8000;
            }
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

fn counter_harness(players: u8, auth: AuthoritativeConfig) -> Harness<Counter> {
    let mut cfg = RoomConfig::new(players, TICK_RATE, SEED, 8);
    cfg.record_all = true;
    let sim = GameSim::<Counter>::new((), TICK_RATE, SEED, 1, players);
    Harness::authoritative(3, cfg, auth, Box::new(sim), counter_script(), |_| ())
}

// ---- (b) a forced desync is detected, corrected and dumped -------------------

#[test]
fn a_corrupted_client_is_corrected_by_the_server() {
    let p = path(40, 5, 10_000);
    let mut h = counter_harness(3, AuthoritativeConfig::default());
    for _ in 0..3 {
        h.add_client(&ClientSpec::new(p));
    }
    // Client 1 flips a bit in its own state at tick 100.
    CORRUPT.with(|c| c.set(Some((1, 100, false))));
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(500, 120_000_000));
    h.run_for_us(500_000);
    CORRUPT.with(|c| c.set(None));

    let notes = h.server.drain_notes();
    let (tick, found_at, reports) = notes
        .iter()
        .find_map(|n| match n {
            ServerNote::Desync { tick, finalized, reports, .. } => Some((*tick, *finalized, reports.clone())),
            _ => None,
        })
        .expect("the server found no desync");
    let slot1 = h.clients[1].client.welcome().unwrap().slot;
    // Diverged at 100: the first checkpoint after it is 120.
    assert_eq!(tick, 120);
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[0].0, slot1, "the client that is wrong is named, no vote needed");
    assert_eq!(reports[1].0, SERVER_SLOT);
    let detection_ticks = found_at - 100;
    assert!(found_at <= 120 + 30, "found only at server tick {found_at}");

    let (corr_tick, corr_bytes) = notes
        .iter()
        .find_map(|n| match n {
            ServerNote::Correction { slot, from_tick, tick, bytes, .. } if *slot == slot1 && *from_tick == 120 => Some((*tick, *bytes)),
            _ => None,
        })
        .expect("no correction sent");
    assert_eq!(notes.iter().filter(|n| matches!(n, ServerNote::Correction { .. })).count(), 1);

    // The client recovered: one correction, no kick, still playing.
    let c1 = &h.clients[1].client;
    assert_eq!(c1.stats().corrections, 1);
    assert_eq!(*c1.state(), ClientState::Playing);
    assert!(c1.kicked().is_none());
    // The other clients were not disturbed.
    for i in [0usize, 2] {
        assert_eq!(h.clients[i].client.stats().corrections, 0);
        assert_eq!(h.clients[i].client.stats().desyncs, 0);
    }
    // Its checksums agree with the server again, after the correction.
    let compared = assert_clients_match_server(&h, 0, None);
    let after = c1.session().unwrap().checksums().iter().filter(|(t, _)| *t > corr_tick).count();
    assert!(after >= 8, "only {after} checkpoints after the correction");
    // The corrupt checkpoints were dropped from its record.
    assert!(c1.session().unwrap().checksums().iter().all(|(t, _)| *t < 120 || *t > corr_tick));

    // Both sides wrote a dump. The server's replays to its own checksum.
    let server_dumps = h.server_dumps.take();
    assert_eq!(server_dumps.len(), 1);
    assert_eq!(server_dumps[0].0, format!("server_desync_tick120_slot{slot1}.orrd"));
    let dump = DesyncDump::from_bytes(&server_dumps[0].1).unwrap();
    assert_eq!((dump.local_slot, dump.desync_tick), (SERVER_SLOT, 120));
    assert_eq!(dump.anchor_tick, 90);
    let clean = dump.replay::<Counter>((), 1).unwrap().into_iter().find(|(t, _)| *t == 120).unwrap().1;
    assert_eq!(clean, dump.local_checksum);
    assert_eq!(dump.reports.iter().find(|r| r.0 == slot1).map(|r| r.1 != clean), Some(true));
    let client_dumps = h.clients[1].dumps.take();
    assert_eq!(client_dumps.len(), 1, "the diverged client also writes its own dump");
    assert!(h.clients[1].client.drain_events().iter().any(|e| matches!(e, ClientEvent::Corrected { from_tick: 120, .. })));

    eprintln!(
        "(b) corrupted at tick 100, mismatching checkpoint 120, server found it at tick {found_at} ({detection_ticks} ticks after the \
         corruption, {} after the checkpoint), correction of tick {corr_tick}: {corr_bytes} bytes lz4; {after} checkpoints agree afterwards ({compared} compared)",
        found_at - 120
    );
}

// ---- (c) a client that keeps diverging, and one that cheats, are kicked ---------

#[test]
fn a_client_that_keeps_diverging_is_kicked_with_a_reason() {
    let p = path(30, 4, 5_000);
    let auth = AuthoritativeConfig { kick_after_corrections: 3, kick_window_secs: 60, ..AuthoritativeConfig::default() };
    let mut h = counter_harness(3, auth);
    for _ in 0..3 {
        h.add_client(&ClientSpec::new(p));
    }
    // Client 2's state goes wrong at every tick from 100 on.
    CORRUPT.with(|c| c.set(Some((2, 100, true))));
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(600, 120_000_000));
    h.run_for_us(500_000);
    CORRUPT.with(|c| c.set(None));

    let notes = h.server.drain_notes();
    let slot2 = h.clients[2].client.welcome().unwrap().slot;
    let kicked: Vec<_> = notes
        .iter()
        .filter_map(|n| match n {
            ServerNote::Kicked { slot, code, reason, .. } => Some((*slot, *code, reason.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(kicked.len(), 1, "{kicked:?}");
    assert_eq!((kicked[0].0, kicked[0].1), (slot2, BYE_KICKED_DESYNC));
    assert!(kicked[0].2.contains("diverged 3 times"), "{}", kicked[0].2);
    let st = h.server.room_stats(h.room).unwrap();
    // Corrected on its first two divergences, kicked on the third.
    assert_eq!((st.corrections, st.kicks), (2, 1));
    assert_eq!(h.clients[2].client.stats().corrections, 2);
    assert_eq!(h.clients[2].client.kicked(), Some(BYE_KICKED_DESYNC));
    assert_eq!(*h.clients[2].client.state(), ClientState::Disconnected);
    // The others never noticed.
    for i in [0usize, 1] {
        assert_eq!(*h.clients[i].client.state(), ClientState::Playing);
        assert_eq!(h.clients[i].client.stats().corrections, 0);
    }
    assert_clients_match_server(&h, 0, Some(&[0, 1]));
    eprintln!("(c) diverging client: {} corrections, then kicked: {}", st.corrections, kicked[0].2);
}

#[test]
fn a_cheating_client_is_flagged_by_the_state_audit_and_kicked() {
    let p = path(30, 4, 5_000);
    // From tick 200 client 2 sends a forged axis value: three times the legal speed.
    let script: Script<Arena> = Rc::new(|client, tick| {
        let (mut input, cmds) = arena_script(client, tick);
        if client == 2 && tick >= 200 {
            input.axis_x = FP::from_int(3);
        }
        (input, cmds)
    });
    let auth = AuthoritativeConfig { violation_limit: 5, ..AuthoritativeConfig::default() };
    let mut h = auth_arena(3, &[], auth, script, 31);
    for _ in 0..3 {
        h.add_client(&ClientSpec::new(p));
    }
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(500, 120_000_000));
    h.run_for_us(500_000);

    let notes = h.server.drain_notes();
    let slot2 = h.clients[2].client.welcome().unwrap().slot;
    let violations: Vec<u64> = notes
        .iter()
        .filter_map(|n| match n {
            ServerNote::Violation { slot, tick, .. } if *slot == slot2 => Some(*tick),
            _ => None,
        })
        .collect();
    assert_eq!(violations.len(), 5, "{violations:?}");
    // About 200 ticks of the script plus the input delay: first flagged near tick 200..260.
    assert!(violations[0] >= 200, "{violations:?}");
    let kicked: Vec<_> = notes
        .iter()
        .filter_map(|n| match n {
            ServerNote::Kicked { slot, code, reason, .. } => Some((*slot, *code, reason.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(kicked.len(), 1);
    assert_eq!((kicked[0].0, kicked[0].1), (slot2, BYE_KICKED_CHEAT));
    assert_eq!(h.clients[2].client.kicked(), Some(BYE_KICKED_CHEAT));
    // Honest players are untouched, and still agree with the server.
    for i in [0usize, 1] {
        assert_eq!(*h.clients[i].client.state(), ClientState::Playing);
    }
    assert_clients_match_server(&h, 0, Some(&[0, 1]));
    let st = h.server.room_stats(h.room).unwrap();
    assert_eq!((st.violations, st.kicks), (5, 1));
    eprintln!("(c) cheater: first violation at tick {}, kicked after 5: {}", violations[0], kicked[0].2);
}

struct NoWildAxes;
impl InputValidator for NoWildAxes {
    fn validate(&mut self, _ctx: &InputCtx<'_>, input: &[u8], _commands: &[Vec<u8>]) -> Verdict {
        let i: ArenaInput = bytemuck::pod_read_unaligned(input);
        if i.axis_x.abs() > FP::ONE || i.axis_y.abs() > FP::ONE {
            Verdict::Kick
        } else {
            Verdict::Accept
        }
    }
}

#[test]
fn the_validate_hook_can_kick_in_an_authoritative_room() {
    let p = path(30, 4, 0);
    let script: Script<Arena> = Rc::new(|client, tick| {
        let (mut input, cmds) = arena_script(client, tick);
        if client == 1 && tick == 150 {
            input.axis_y = FP::from_int(-9);
        }
        (input, cmds)
    });
    let mut room = arena_room(2);
    room.min_players_to_start = 2;
    let mut h = Harness::<Arena>::with_authoritative_validator(
        5,
        room,
        AuthoritativeConfig::default(),
        Box::new(arena_sim(2, TICK_RATE, SEED, 1, &[])),
        script,
        arena_cfg,
        Box::new(NoWildAxes),
    );
    for _ in 0..2 {
        h.add_client(&ClientSpec::new(p));
    }
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(400, 120_000_000));
    assert_eq!(h.clients[1].client.kicked(), Some(BYE_KICKED_CHEAT));
    assert_eq!(h.server.room_stats(h.room).unwrap().kicks, 1);
    assert_eq!(*h.clients[0].client.state(), ClientState::Playing);
}

// ---- (d) late join and rejoin from the server -----------------------------------

#[test]
fn late_joiner_and_rejoiner_get_the_servers_own_snapshot() {
    let p = path(50, 8, 10_000);
    let mut room = arena_room(4);
    room.min_players_to_start = 3;
    let mut h = Harness::<Arena>::authoritative(
        9,
        room,
        AuthoritativeConfig::default(),
        Box::new(arena_sim(4, TICK_RATE, SEED, 1, &[])),
        arena_script_rc(),
        arena_cfg,
    );
    for _ in 0..3 {
        h.add_client(&ClientSpec::new(p));
    }
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(600, 120_000_000));

    let joiner = h.add_client(&ClientSpec::new(p));
    assert!(h.run_until_all_playing(20_000_000), "late joiner never started playing");
    let st = h.server.room_stats(h.room).unwrap();
    assert_eq!((st.server_snapshots, st.snapshots_relayed), (1, 1));
    let start_tick = h.server.finalized_tick(h.room).unwrap();
    assert!(h.run_until_tick(start_tick + 600, 120_000_000));

    // Client 1 drops and comes back with its token.
    let token = h.clients[1].client.token().unwrap();
    h.cut(1);
    let cut_tick = h.server.finalized_tick(h.room).unwrap();
    assert!(h.run_until_tick(cut_tick + 300, 120_000_000));
    let out = &h.recorded()[(cut_tick + 200) as usize - 1];
    assert!(out.slots[1].flags & FLAG_ABSENT != 0);
    let back = h.add_client(&ClientSpec { want_slot: Some(PlayerSlot(1)), token, ..ClientSpec::new(p) });
    assert!(h.run_until_all_playing_except(&[1], 20_000_000));
    assert_eq!(h.clients[back].client.welcome().unwrap().slot, 1);
    let t = h.server.finalized_tick(h.room).unwrap();
    assert!(h.run_until_tick(t + 900, 120_000_000));
    h.run_for_us(500_000);

    let st = h.server.room_stats(h.room).unwrap();
    assert_eq!(st.server_snapshots, 2);
    let notes = h.server.drain_notes();
    // No peer was asked to donate.
    assert!(!notes.iter().any(|n| matches!(n, ServerNote::SnapshotRequested { .. })));
    let snaps: Vec<_> = notes
        .iter()
        .filter_map(|n| match n {
            ServerNote::ServerSnapshot { tick, bytes, .. } => Some((*tick, *bytes)),
            _ => None,
        })
        .collect();
    assert_eq!(snaps.len(), 2);
    // Joiners (and everyone) match the server's own checksums, from the join on.
    let compared = assert_clients_match_server(&h, 0, Some(&[0, 2, joiner, back]));
    assert!(compared > 40);
    assert_eq!(h.clients[joiner].client.stats().corrections, 0);
    for i in [0usize, 2, joiner, back] {
        assert_eq!(*h.clients[i].client.state(), ClientState::Playing, "client {i}");
        assert!(h.clients[i].dumps.is_empty());
    }
    eprintln!("(d) late join snapshot at tick {} ({} B lz4), rejoin snapshot at tick {} ({} B); {compared} checkpoints equal the server's", snaps[0].0, snaps[0].1, snaps[1].0, snaps[1].1);
}

// ---- (e) a server-side AI slot ------------------------------------------------

#[test]
fn a_server_driven_ai_slot_is_seen_identically_by_every_client() {
    let p = path(40, 6, 10_000);
    // 4 slots; slot 3 is the server's.
    let mut h = auth_arena(4, &[3], AuthoritativeConfig { server_slots: vec![3], ..Default::default() }, arena_script_rc(), 17);
    for _ in 0..3 {
        h.add_client(&ClientSpec::new(p));
    }
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(900, 120_000_000));
    h.run_for_us(500_000);

    // Nobody can take the AI's slot.
    let intruder = h.add_client(&ClientSpec { want_slot: Some(PlayerSlot(3)), ..ClientSpec::new(p) });
    h.run_for_us(1_000_000);
    assert!(matches!(h.clients[intruder].client.state(), ClientState::Rejected(orr_proto::RejectReason::SlotTaken)));

    // The AI's inputs are in the confirmed stream: marked neither repeated nor absent, with commands.
    let bundles = h.recorded();
    let ai_shots = bundles.iter().filter(|b| !b.slots[3].commands.is_empty()).count();
    assert!(ai_shots >= 60, "the AI fired only {ai_shots} times");
    assert!(bundles.iter().all(|b| b.slots[3].flags == 0));
    let idle = vec![0u8; std::mem::size_of::<ArenaInput>()];
    let moved = bundles.iter().filter(|b| b.slots[3].input != idle).count();
    assert!(moved > 100, "the AI moved on only {moved} ticks");
    let st = h.server.room_stats(h.room).unwrap();
    assert_eq!(st.slots[3].delivered, st.finalized);

    // Every client's own simulation agrees with the server's, including the AI's effects.
    let compared = assert_clients_match_server(&h, 0, Some(&[0, 1, 2]));
    assert!(compared >= 60);
    assert_matches_headless::<Arena>(&h, || ArenaConfig { player_count: 4 }, 4, Some(&[0, 1, 2]));
    // And the AI visibly acted: it left its starting point, and all clients show the same place.
    let mut positions = Vec::new();
    for i in 0..3 {
        let s = h.clients[i].client.session().unwrap();
        let f = s.verified_frame().unwrap();
        let (ents, tags) = f.dense::<PlayerTag>();
        let e = ents.iter().zip(tags).find(|(_, t)| t.slot == 3).map(|(e, _)| *e).unwrap();
        positions.push((s.verified_tick(), f.get::<Position>(e).unwrap().pos));
    }
    let fresh = orr_sim::Simulation::<Arena>::with_build_id(ArenaConfig { player_count: 4 }, TICK_RATE, SEED, 1);
    let (ents, tags) = fresh.frame().dense::<PlayerTag>();
    let e0 = ents.iter().zip(tags).find(|(_, t)| t.slot == 3).map(|(e, _)| *e).unwrap();
    let start = fresh.frame().get::<Position>(e0).unwrap().pos;
    for (tick, pos) in &positions {
        assert_ne!(*pos, start, "the AI never moved (tick {tick})");
    }
    eprintln!("(e) AI fired {ai_shots} times, moved on {moved} ticks, {compared} checkpoints equal the server's; positions {positions:?}");
    assert!(h.clients[0..3].iter().all(|c| *c.client.state() == ClientState::Playing));
}

// ---- relay rooms are untouched by all of this ------------------------------------

#[test]
fn a_relay_room_stays_a_relay_room() {
    let p = path(40, 5, 5_000);
    let mut h = arena_harness(2);
    for _ in 0..2 {
        h.add_client(&ClientSpec::new(p));
    }
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(300, 60_000_000));
    for c in &h.clients {
        assert!(!c.client.welcome().unwrap().authoritative());
    }
    let st = h.server.room_stats(h.room).unwrap();
    assert_eq!((st.server_ticks, st.corrections, st.server_snapshots), (0, 0, 0));
    assert!(h.server.server_checksums(h.room).is_empty());
    assert!(h.server_dumps.is_empty());
}

// ---- cost ----------------------------------------------------------------------

/// What the arena server sim costs per tick (4 slots, one AI, state audit on),
/// and what a snapshot costs. Printed; the assert only catches pathologies.
#[test]
fn arena_server_sim_cost_per_tick() {
    use orr_proto::{Bundle, SlotConfirmed};
    use orr_server::ServerSim;
    use std::time::{Duration, Instant};

    let players = 4u8;
    let mut sim = arena_sim(players, TICK_RATE, SEED, 1, &[3]);
    let ticks = 6000u64;
    let mut step = Vec::new();
    let mut drive = Vec::new();
    for tick in 1..=ticks {
        let t = Instant::now();
        let driven = sim.drive(tick, &[3]);
        drive.push(t.elapsed());
        let slots: Vec<SlotConfirmed> = (0..players)
            .map(|s| {
                if s == 3 {
                    SlotConfirmed { input: driven[0].1.clone(), commands: driven[0].2.clone(), flags: 0 }
                } else {
                    let (i, c) = arena_script(usize::from(s), tick);
                    let commands = c.iter().map(|c| { let mut b = Vec::new(); c.encode(&mut b); b }).collect();
                    SlotConfirmed { input: bytemuck::bytes_of(&i).to_vec(), commands, flags: 0 }
                }
            })
            .collect();
        let t = Instant::now();
        let v = sim.step(&Bundle { tick, slots });
        step.push(t.elapsed());
        assert!(v.is_empty(), "honest input broke a rule at tick {tick}: {v:?}");
    }
    step.sort();
    drive.sort();
    let avg = step.iter().sum::<Duration>() / step.len() as u32;
    let t = Instant::now();
    let cs = sim.checksum();
    let checksum = t.elapsed();
    let t = Instant::now();
    let bytes = sim.frame_bytes();
    let to_bytes = t.elapsed();
    let t = Instant::now();
    let packed = lz4_flex_pack(&bytes);
    let lz4 = t.elapsed();
    let p = |v: &[Duration], q: f64| v[((v.len() as f64 * q) as usize).min(v.len() - 1)].as_secs_f64() * 1e6;
    eprintln!(
        "arena, 4 slots (1 AI, audit on), {ticks} ticks: step avg {:.1} us, p99 {:.1}, max {:.1} us; ai drive p99 {:.1} us; checksum {:.1} us (every 30 ticks); \
         snapshot to_bytes {:.1} us + lz4 {:.1} us, {} B raw -> {} B (checksum {cs:#x})",
        avg.as_secs_f64() * 1e6,
        p(&step, 0.99),
        step.last().unwrap().as_secs_f64() * 1e6,
        p(&drive, 0.99),
        checksum.as_secs_f64() * 1e6,
        to_bytes.as_secs_f64() * 1e6,
        lz4.as_secs_f64() * 1e6,
        bytes.len(),
        packed
    );
    assert!(avg < Duration::from_millis(5), "arena sim step takes {avg:?}");
}

fn lz4_flex_pack(bytes: &[u8]) -> usize {
    lz4_flex::block::compress_prepend_size(bytes).len()
}
