//! Shared by the tests of the steps: a player that chases the nearest star.
#![allow(dead_code)]

use std::collections::BTreeMap;

use orr_proto::netsim::{LinkParams, PathParams};
use orr_server::harness::Harness;
use orr_server::RoomConfig;
use orr_sim::{fp, Frame, Game, FP};

pub const SEED: u64 = 42;
pub const TICK_RATE: u32 = 60;

/// The direction (-1, 0 or 1 on each axis) from `me` to the nearest of `stars`.
/// It is a pure function of the positions, so a test run is the same every time.
pub fn chase(me: (FP, FP), stars: &[(FP, FP)]) -> (i32, i32) {
    let distance = |star: &(FP, FP)| (star.0 - me.0).abs() + (star.1 - me.1).abs();
    let Some(nearest) = stars.iter().min_by_key(|star| distance(star).raw()) else { return (0, 0) };
    let toward = |from: FP, to: FP| {
        let gap = to - from;
        if gap > fp!(0.05) {
            1
        } else if gap < fp!(-0.05) {
            -1
        } else {
            0
        }
    };
    (toward(me.0, nearest.0), toward(me.1, nearest.1))
}

/// A server room for `players` seats that expects inputs of `input_size` bytes.
pub fn room(players: u8, input_size: usize) -> RoomConfig {
    let mut cfg = RoomConfig::new(players, TICK_RATE, SEED, input_size as u32);
    cfg.build_hash = orr_sim::build_hash_of(1, 0);
    cfg
}

/// A path with `one_way_ms` of delay each way, `jitter_ms` of jitter, and packet loss
/// (in parts per million) on the unreliable channel.
pub fn path(one_way_ms: u64, jitter_ms: u64, loss_ppm: u32) -> PathParams {
    PathParams::symmetric(
        LinkParams::new(one_way_ms * 1000, jitter_ms * 1000)
            .with_loss_ppm(loss_ppm)
            .with_dup_ppm(loss_ppm / 4)
            .with_reorder(loss_ppm, jitter_ms * 1500),
    )
}

/// The newest verified frame of client `i`.
pub fn verified<G: Game>(h: &Harness<G>, i: usize) -> &Frame {
    h.clients[i].client.session().expect("the client plays").verified_frame().expect("a verified frame")
}

/// Every client that checked a tick got the same checksum for it. Returns how many ticks were
/// checked by more than one client.
pub fn assert_clients_agree<G: Game>(h: &Harness<G>) -> usize {
    let mut by_tick: BTreeMap<u64, Vec<(usize, u64)>> = BTreeMap::new();
    for (i, client) in h.clients.iter().enumerate() {
        if let Some(session) = client.client.session() {
            for &(tick, checksum) in session.checksums() {
                by_tick.entry(tick).or_default().push((i, checksum));
            }
        }
    }
    let mut shared = 0;
    for (tick, list) in by_tick {
        assert!(list.iter().all(|&(_, checksum)| checksum == list[0].1), "clients disagree at tick {tick}: {list:?}");
        shared += usize::from(list.len() > 1);
    }
    shared
}
