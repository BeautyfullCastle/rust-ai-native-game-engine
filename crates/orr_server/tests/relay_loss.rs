//! Loss robustness of the relay: 4 clients at 150 ms round trip under
//! independent loss and under burst loss (Gilbert-Elliott). Measures stalls,
//! rollbacks, server-repeated inputs and bandwidth.
//!
//! `cargo test -p orr_server --release --test relay_loss -- --ignored --nocapture sweep`
//! prints the table used to choose the confirmed-bundle delivery.
#![allow(clippy::float_arithmetic)]
mod common;

use common::*;
use orr_proto::netsim::{LinkParams, PathParams};
use orr_proto::Welcome;
use orr_server::harness::{ClientSpec, Harness};
use orr_server::RoomConfig;
use orr_testgame::{Arena, ArenaConfig};

const PLAYERS: usize = 4;

#[derive(Clone, Copy, Debug)]
enum Loss {
    /// Independent loss, parts per million (with duplication and reordering
    /// like `common::path`).
    Independent(u32),
    /// `avg_ppm` of the messages lost in bursts of `mean` messages.
    Burst { avg_ppm: u32, mean: u32 },
}

fn loss_path(loss: Loss) -> PathParams {
    match loss {
        Loss::Independent(ppm) => path(70, 10, ppm),
        Loss::Burst { avg_ppm, mean } => {
            PathParams::symmetric(LinkParams::new(70_000, 10_000).with_burst_loss(avg_ppm, mean))
        }
    }
}

#[derive(Debug, Default, Clone)]
struct Report {
    /// Per client: stall episodes, stalled ms, longest stall ms, rollbacks per second.
    clients: Vec<(u64, u64, u64, f64)>,
    /// Inputs the server had to repeat (all slots).
    server_repeated: u64,
    /// Bytes per second per client, downlink and uplink.
    down_bps: f64,
    up_bps: f64,
    /// Longest run of lost server packets to one client, in ms of ticks.
    longest_down_burst_ms: u64,
}

impl Report {
    fn max_stall_ms(&self) -> u64 {
        self.clients.iter().map(|c| c.2).max().unwrap_or(0)
    }
    fn total_stall_ms(&self) -> u64 {
        self.clients.iter().map(|c| c.1).sum()
    }
    fn episodes(&self) -> u64 {
        self.clients.iter().map(|c| c.0).sum()
    }
    fn rollbacks_per_s(&self) -> f64 {
        self.clients.iter().map(|c| c.3).sum::<f64>() / self.clients.len() as f64
    }
}

fn run(net_seed: u64, loss: Loss, ticks: u64, input_redundancy: Option<u32>, tweak: &dyn Fn(&mut RoomConfig)) -> Report {
    let mut cfg = arena_room(PLAYERS as u8);
    tweak(&mut cfg);
    let mut h: Harness<Arena> =
        Harness::new(net_seed, cfg, arena_script_rc(), |w: &Welcome| ArenaConfig { player_count: w.player_count });
    let p = loss_path(loss);
    for _ in 0..PLAYERS {
        h.add_client(&ClientSpec { input_redundancy, ..ClientSpec::new(p) });
    }
    assert!(h.run_until_all_playing(10_000_000), "clients did not start");
    assert!(h.run_until_tick(300, 30_000_000));
    let warm: Vec<_> = (0..PLAYERS).map(|i| client_stalls(&h, i)).collect();
    let warm_net = h.net.stats();
    let warm_repeated = repeated(&h);
    assert!(h.run_until_tick(ticks, 120_000_000));
    let secs = (ticks - 300) as f64 / 60.0;
    let mut r = Report::default();
    for (i, w) in warm.iter().enumerate() {
        let end = client_stalls(&h, i);
        // The longest stall is over the whole run, so it includes the warm-up.
        r.clients.push((end.1 - w.1, end.2 - w.2, end.3, (end.0 - w.0) as f64 / secs));
    }
    let net = h.net.stats();
    r.down_bps = (net.bytes_down - warm_net.bytes_down) as f64 / secs / PLAYERS as f64;
    r.up_bps = (net.bytes_up - warm_net.bytes_up) as f64 / secs / PLAYERS as f64;
    r.server_repeated = repeated(&h) - warm_repeated;
    r.longest_down_burst_ms = u64::from(net.longest_loss_run_down) * 1000 / u64::from(TICK_RATE);
    for c in &h.clients {
        assert!(c.dumps.is_empty(), "unexpected desync dump");
    }
    assert_eq!(net.oversize_dropped, 0);
    r
}

/// `(rollbacks, episodes, stalled ms, longest stall ms)`.
fn client_stalls(h: &Harness<Arena>, i: usize) -> (u64, u64, u64, u64) {
    let c = &h.clients[i].client;
    let rb = c.session().map_or(0, |s| s.rollback_count());
    (rb, c.stats().stall_episodes, c.stats().stalled_us / 1000, c.stats().max_stall_us / 1000)
}

fn repeated(h: &Harness<Arena>) -> u64 {
    h.server.room_stats(h.room).unwrap().slots.iter().map(|s| s.repeated).sum()
}

fn print_row(name: &str, loss: Loss, r: &Report) {
    println!(
        "{name:<26} {loss:<38} eps {:>3} total {:>5} ms max {:>4} ms (longest loss run {:>3} ms)  rb/s {:>5.2}  repeated {:>4}  down {:>6.0} B/s  up {:>5.0} B/s",
        r.episodes(),
        r.total_stall_ms(),
        r.max_stall_ms(),
        r.longest_down_burst_ms,
        r.rollbacks_per_s(),
        r.server_repeated,
        r.down_bps,
        r.up_bps,
        loss = format!("{loss:?}"),
    );
}

const SEEDS: [u64; 3] = [11, 22, 33];

/// Runs every seed and merges: sums for counts, max for the maximum.
fn sweep_one(loss: Loss, ticks: u64, input_redundancy: Option<u32>, tweak: &dyn Fn(&mut RoomConfig)) -> Report {
    let mut all = Report::default();
    let n = SEEDS.len() as f64;
    for &s in &SEEDS {
        let r = run(s, loss, ticks, input_redundancy, tweak);
        all.clients.extend(r.clients);
        all.server_repeated += r.server_repeated;
        all.down_bps += r.down_bps / n;
        all.up_bps += r.up_bps / n;
        all.longest_down_burst_ms = all.longest_down_burst_ms.max(r.longest_down_burst_ms);
    }
    all
}

const LOSSES: [Loss; 3] = [
    Loss::Independent(20_000),
    Loss::Burst { avg_ppm: 20_000, mean: 3 },
    Loss::Burst { avg_ppm: 20_000, mean: 5 },
];

type Tweak = Box<dyn Fn(&mut RoomConfig)>;

#[test]
#[ignore = "measurement table; run with --ignored --nocapture"]
fn sweep() {
    let variants: Vec<(&str, Tweak)> = vec![
        ("legacy (3 copies+timeout)", Box::new(|c| c.repeat_gap_ticks = 0)),
        ("gap 1 (always)", Box::new(|c| c.repeat_gap_ticks = 1)),
        ("gap 2", Box::new(|c| c.repeat_gap_ticks = 2)),
        ("gap 3", Box::new(|c| c.repeat_gap_ticks = 3)),
        ("gap 4", Box::new(|c| c.repeat_gap_ticks = 4)),
        (
            "red 2, gap 3",
            Box::new(|c| {
                c.bundle_redundancy = 2;
                c.repeat_gap_ticks = 3;
            }),
        ),
        (
            "red 1, gap 3",
            Box::new(|c| {
                c.bundle_redundancy = 1;
                c.repeat_gap_ticks = 3;
            }),
        ),
    ];
    for (name, tweak) in &variants {
        for loss in LOSSES {
            print_row(name, loss, &sweep_one(loss, 3000, None, tweak.as_ref()));
        }
    }
}

#[test]
#[ignore = "measurement table; run with --ignored --nocapture"]
fn sweep_uplink() {
    for red in [2, 4, 6, 8, 12] {
        for loss in LOSSES {
            let tweak: Tweak = Box::new(|c| c.repeat_gap_ticks = 1);
            print_row(&format!("uplink redundancy {red}"), loss, &sweep_one(loss, 3000, Some(red), tweak.as_ref()));
        }
    }
}

/// Regression: with the default delivery, a burst of lost packets costs a
/// stall of about the burst itself, not the 1.5 round trip resend timeout
/// (about 260 ms at 150 ms round trip), and the downlink stays as small as
/// it was with three copies in the old encoding (about 25 KB/s at 4 players).
#[test]
fn burst_loss_stalls_end_with_the_burst_and_bandwidth_stays_small() {
    for loss in [Loss::Burst { avg_ppm: 20_000, mean: 3 }, Loss::Burst { avg_ppm: 20_000, mean: 5 }] {
        for seed in [11, 22] {
            let r = run(seed, loss, 3000, None, &|_| {});
            // A stall cannot end before the burst does; after it, the next
            // packet carries every missing bundle. Allow 100 ms for jitter
            // and the client's own slack.
            let bound = r.longest_down_burst_ms + 100;
            assert!(r.max_stall_ms() <= bound, "{loss:?} seed {seed}: stall {} ms, bound {bound} ms", r.max_stall_ms());
            assert!(r.down_bps < 30_000.0, "{loss:?}: downlink {:.0} B/s", r.down_bps);
            assert!(r.up_bps < 15_000.0, "{loss:?}: uplink {:.0} B/s", r.up_bps);
        }
    }
    let r = run(11, Loss::Independent(20_000), 3000, None, &|_| {});
    assert!(r.max_stall_ms() < 100, "independent loss: stall {} ms", r.max_stall_ms());
}
