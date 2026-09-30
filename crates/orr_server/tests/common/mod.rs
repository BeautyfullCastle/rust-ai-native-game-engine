//! Shared pieces of the relay end-to-end tests.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::rc::Rc;

use orr_fp::FP;
use orr_proto::netsim::{LinkParams, PathParams};
use orr_proto::Welcome;
use orr_server::harness::{headless_checksums, Harness, Script};
use orr_server::RoomConfig;
use orr_sim::Game;
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd, FIRE};

pub const TICK_RATE: u32 = 60;
pub const SEED: u64 = 0xC0FFEE;

fn mix(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// A player-like input: holds a direction for a few ticks, fires now and
/// then. A pure function of `(client, tick)`.
pub fn arena_script(client: usize, tick: u64) -> (ArenaInput, Vec<SpawnBulletCmd>) {
    let k = tick / 7 + client as u64 * 3;
    let h = mix(k ^ (client as u64) << 40);
    let ax = (h % 3) as i32 - 1;
    let ay = ((h >> 8) % 3) as i32 - 1;
    let fire = tick % 11 == client as u64 % 11 && (h >> 20) % 3 != 0;
    let input = ArenaInput::new(FP::from_int(ax), FP::from_int(ay), fire);
    let cmds = if input.buttons & FIRE != 0 { vec![SpawnBulletCmd { owner: client as u32 }] } else { Vec::new() };
    (input, cmds)
}

pub fn arena_script_rc() -> Script<Arena> {
    Rc::new(arena_script)
}

pub fn arena_room(players: u8) -> RoomConfig {
    let mut cfg = RoomConfig::new(players, TICK_RATE, SEED, std::mem::size_of::<ArenaInput>() as u32);
    cfg.record_all = true;
    cfg.build_hash = orr_sim::build_hash_of(1, 0);
    cfg
}

pub fn arena_harness(players: u8) -> Harness<Arena> {
    Harness::new(7, arena_room(players), arena_script_rc(), |w: &Welcome| ArenaConfig { player_count: w.player_count })
}

/// A path with `one_way_ms` latency each way, `jitter_ms` of jitter, and
/// loss on the unreliable channel.
pub fn path(one_way_ms: u64, jitter_ms: u64, loss_ppm: u32) -> PathParams {
    PathParams::symmetric(
        LinkParams::new(one_way_ms * 1000, jitter_ms * 1000)
            .with_loss_ppm(loss_ppm)
            .with_dup_ppm(loss_ppm / 4)
            .with_reorder(loss_ppm, jitter_ms * 1500),
    )
}

/// Checks every client's verified checksums against a headless run of the
/// server's confirmed stream. Returns how many checkpoints were compared.
pub fn assert_matches_headless<G: Game>(
    h: &Harness<G>,
    config: impl Fn() -> G::Config,
    players: u8,
    only_clients: Option<&[usize]>,
) -> usize {
    let reference = headless_checksums::<G>(
        config(),
        TICK_RATE,
        SEED,
        1,
        players,
        30,
        h.recorded(),
    );
    let mut compared = 0;
    for (i, c) in h.clients.iter().enumerate() {
        if only_clients.is_some_and(|o| !o.contains(&i)) {
            continue;
        }
        let Some(session) = c.client.session() else { continue };
        for &(tick, cs) in session.checksums() {
            let want = reference.get(&tick).unwrap_or_else(|| panic!("no reference for tick {tick}"));
            assert_eq!(cs, *want, "client {i} disagrees with the headless run at tick {tick}");
            compared += 1;
        }
    }
    compared
}

/// Clients' verified checksums must agree with each other at every tick.
pub fn assert_clients_agree<G: Game>(h: &Harness<G>) {
    let mut by_tick: BTreeMap<u64, Vec<(usize, u64)>> = BTreeMap::new();
    for (i, c) in h.clients.iter().enumerate() {
        if let Some(s) = c.client.session() {
            for &(t, cs) in s.checksums() {
                by_tick.entry(t).or_default().push((i, cs));
            }
        }
    }
    for (t, list) in by_tick {
        assert!(list.iter().all(|&(_, cs)| cs == list[0].1), "clients disagree at tick {t}: {list:?}");
    }
}

/// `(rollbacks, stall episodes, stalled ms)` of a client so far.
pub fn client_counters<G: Game>(h: &Harness<G>, i: usize) -> (u64, u64, u64) {
    let c = &h.clients[i].client;
    let rb = c.session().map_or(0, |s| s.rollback_count());
    (rb, c.stats().stall_episodes, c.stats().stalled_us / 1000)
}
