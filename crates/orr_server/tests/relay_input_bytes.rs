//! Bytes a client sends for its inputs: idle, changing every tick, a typical
//! player (changes about every 10 ticks), at 60 ticks/s with 2 and 4 players.
//! Run with `--nocapture` to see the table (the numbers are in
//! `docs/view-framework.md` section 10.2).
mod common;

use std::rc::Rc;

use common::*;
use orr_fp::FP;
use orr_proto::Welcome;
use orr_server::harness::{ClientSpec, Harness, Script};
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd};

/// An input that changes every `period` ticks (`0` = never).
fn script(period: u64) -> Script<Arena> {
    Rc::new(move |client, tick| {
        let k = if period == 0 { 0 } else { tick / period + client as u64 };
        let ax = (k % 3) as i32 - 1;
        let ay = ((k / 3) % 3) as i32 - 1;
        (ArenaInput::new(FP::from_int(ax), FP::from_int(ay), false), Vec::<SpawnBulletCmd>::new())
    })
}

struct Row {
    /// Bytes per second one client sends in input packets.
    input_bps: f64,
    /// Bytes per second one client sends in all, pings and checksums included.
    up_bps: f64,
    /// Bytes of one input packet.
    packet: f64,
}

fn measure(players: u8, period: u64) -> (Row, Vec<u64>) {
    let mut h =
        Harness::<Arena>::new(5, arena_room(players), script(period), |w: &Welcome| ArenaConfig { player_count: w.player_count });
    let p = path(30, 3, 0);
    for _ in 0..players {
        h.add_client(&ClientSpec::new(p));
    }
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(300, 30_000_000));
    let src = |h: &Harness<Arena>| -> (u64, u64) {
        let (mut bytes, mut packets) = (0, 0);
        for c in &h.clients {
            let s = c.client.source_stats();
            bytes += s.input_bytes_sent;
            packets += s.packets_sent;
        }
        (bytes, packets)
    };
    let (b0, k0) = src(&h);
    let up0 = h.net.stats().bytes_up;
    assert!(h.run_until_tick(1500, 60_000_000));
    let (b1, k1) = src(&h);
    let up1 = h.net.stats().bytes_up;
    let secs = 1200.0 / 60.0;
    let n = f64::from(players);
    let row = Row {
        input_bps: (b1 - b0) as f64 / n / secs,
        up_bps: (up1 - up0) as f64 / n / secs,
        packet: (b1 - b0) as f64 / (k1 - k0) as f64,
    };
    // What every client verified, to compare runs.
    let sums = h.clients[0].client.session().unwrap().checksums().iter().map(|&(_, c)| c).collect();
    (row, sums)
}

#[test]
fn input_bytes_per_second() {
    eprintln!("scenario                 players  packet B  input B/s  client up B/s");
    for (name, period) in [("idle", 0u64), ("changes every tick", 1), ("typical (every 10)", 10)] {
        for players in [2u8, 4] {
            let (r, _) = measure(players, period);
            eprintln!("{name:<24} {players:>7}  {:>8.1}  {:>9.0}  {:>13.0}", r.packet, r.input_bps, r.up_bps);
        }
    }
}
