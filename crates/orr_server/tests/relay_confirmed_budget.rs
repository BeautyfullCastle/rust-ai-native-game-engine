//! A `Confirmed` message that carries many signal count changes stays inside
//! `max_unreliable_payload`. The count changes ride in a trailer after the
//! bundles, so the server must count the trailer when it packs a datagram.
//!
//! The test stops the acks of one client while six other players leave one
//! after the other. Every leaving player is one signal, so the unacked
//! bundles that the server repeats to the silent client hold six count
//! changes near their start. The input never changes, so a repeated bundle is
//! small and a datagram holds as many bundles as the budget allows. The limit
//! is the network's own datagram limit, so the simulated network counts a
//! message over the limit as dropped.

mod common;

use std::rc::Rc;

use common::{path, SEED, TICK_RATE};
use orr_proto::netsim::{LinkParams, PathParams};
use orr_proto::UNRELIABLE_MTU;
use orr_server::harness::{ClientSpec, Harness};
use orr_server::RoomConfig;
use orr_sim::PlayerSlot;
use orr_testgame::roster::{Roster, RosterInput};

const PLAYERS: u8 = 8;
const SILENT: usize = 1;

fn harness(limit: u32) -> Harness<Roster> {
    let mut cfg = RoomConfig::new(PLAYERS, TICK_RATE, SEED, std::mem::size_of::<RosterInput>() as u32);
    cfg.build_hash = orr_sim::build_hash_of(1, 0);
    cfg.record_all = true;
    cfg.max_unreliable_payload = limit;
    let script = Rc::new(|_: usize, _: u64| (RosterInput { dx: 1 }, Vec::new()));
    Harness::<Roster>::new(11, cfg, script, |_| ())
}

fn run(limit: u32) {
    let p = path(20, 2, 0);
    let mut h = harness(limit);
    for slot in 0..PLAYERS {
        h.add_client(&ClientSpec { want_slot: Some(PlayerSlot(slot)), ..ClientSpec::new(p) });
    }
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(120, 60_000_000));
    assert_eq!(h.net.stats().oversize_dropped, 0);

    // The server hears nothing more from this client, so it never acks.
    let start = h.server.finalized_tick(h.room).unwrap();
    h.set_path(SILENT, PathParams { up: LinkParams::new(20_000, 0).with_loss_ppm(1_000_000), down: p.down });
    for leaving in 2..PLAYERS as usize {
        h.cut(leaving);
        let until = h.server.finalized_tick(h.room).unwrap() + 3;
        assert!(h.run_until_tick(until, 60_000_000));
    }
    // A backlog that needs several datagrams.
    assert!(h.run_until_tick(start + 400, 60_000_000));
    h.run_for_us(500_000);

    // The scene holds what the test is about: six count changes among the
    // bundles that the silent client has not acked, and more bundles than
    // one datagram holds.
    let backlog: Vec<_> = h.recorded().iter().filter(|b| b.tick > start).collect();
    let changes = backlog.windows(2).filter(|w| w[0].signals != w[1].signals).count();
    assert!(changes >= 6, "{changes} count changes");
    assert!(backlog.len() > 300, "{} bundles", backlog.len());
    let last_change = backlog.windows(2).rposition(|w| w[0].signals != w[1].signals).unwrap();
    assert!(last_change < 100, "the count changes are not near the start: {last_change}");

    assert_eq!(h.net.stats().oversize_dropped, 0, "the server sent a datagram over the limit");
}

#[test]
fn a_backlog_with_many_count_changes_stays_inside_the_payload_limit() {
    run(UNRELIABLE_MTU as u32);
}
