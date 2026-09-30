//! Relay mode end to end with the arena test game: 4 clients and a server
//! on the simulated network at 150 ms round trip.
mod common;

use common::*;
use orr_proto::netsim::PathParams;
use orr_server::harness::ClientSpec;
use orr_testgame::{Arena, ArenaConfig};

const TICKS: u64 = 3000;

#[test]
fn four_clients_at_150ms_rtt_match_the_headless_run() {
    // 70 ms + up to 10 ms jitter one way = about 150 ms round trip; 2% loss
    // (plus duplication and reordering) on the unreliable channel.
    let p = path(70, 10, 20_000);
    let mut h = arena_harness(4);
    for _ in 0..4 {
        h.add_client(&ClientSpec::new(p));
    }
    assert!(h.run_until_all_playing(10_000_000), "clients did not start");
    assert!(h.run_until_tick(300, 30_000_000));
    let warm: Vec<_> = (0..4).map(|i| client_counters(&h, i)).collect();
    assert!(h.run_until_tick(TICKS, 120_000_000));
    // Let the last confirmations arrive.
    h.run_for_us(500_000);

    let compared = assert_matches_headless::<Arena>(&h, || ArenaConfig { player_count: 4 }, 4, None);
    assert_clients_agree(&h);
    assert!(compared >= 4 * 90, "only {compared} checkpoints compared");

    let secs = (TICKS - 300) as f64 / 60.0;
    for i in 0..4 {
        let end = client_counters(&h, i);
        let c = &h.clients[i].client;
        let stats = c.stats();
        let src = c.source_stats();
        let verified = c.session().unwrap().verified_tick();
        eprintln!(
            "client {i}: rollbacks {} ({:.1}/s), stall episodes {} ({} ms), delay {}, rate {} ppm, srtt {} us, \
             max depth {}, own repeated {}, own overridden {}, verified {}",
            end.0 - warm[i].0,
            (end.0 - warm[i].0) as f64 / secs,
            end.1 - warm[i].1,
            end.2 - warm[i].2,
            c.delay(),
            c.rate_ppm(),
            c.srtt_us(),
            stats.max_prediction_depth,
            src.own_repeated,
            src.own_overridden,
            verified
        );
        assert!(verified + 60 >= TICKS, "client {i} verified only {verified}");
        assert!(h.clients[i].dumps.is_empty(), "unexpected desync dump");
    }
    let net = h.net.stats();
    eprintln!("net: {net:?}");
    assert_eq!(net.oversize_dropped, 0, "an unreliable message was over the MTU");
    let room = h.server.room_stats(h.room).unwrap();
    eprintln!("server slots: {:?}", room.slots);
    assert_eq!(room.desyncs, 0);
    let _ = PathParams::default();
}
