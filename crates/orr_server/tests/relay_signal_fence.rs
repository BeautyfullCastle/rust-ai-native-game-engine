//! The signal fence: a player signal travels on the reliable channel and the
//! confirmed bundles on the unreliable one, so a signal can arrive after the
//! bundles of the ticks it applies to. A client must wait with such a tick
//! until it has the signal. The tests hold back everything the server sends
//! to one client (client B) for a while, at the moment the server announces a
//! signal, and let the network go back to normal right after. The bundles that
//! the server sends later then reach B before the held signal does.

mod common;

use std::rc::Rc;

use common::{assert_clients_agree, path, SEED, TICK_RATE};
use orr_proto::netsim::{LinkParams, PathParams};
use orr_server::harness::{ClientSpec, Harness};
use orr_server::{RoomConfig, ServerNote};
use orr_session::{ClientState, EventStatus};
use orr_sim::{PlayerSlot, SignalKind};
use orr_testgame::roster::{Census, Roster, RosterInput};

/// `RoomConfig::signal_lead_ticks` (the default).
const LEAD: u64 = 10;
const A: usize = 0;
const B: usize = 1;
const C: usize = 2;

fn room(reserve_ticks: u32) -> RoomConfig {
    let mut cfg = RoomConfig::new(3, TICK_RATE, SEED, std::mem::size_of::<RosterInput>() as u32);
    cfg.build_hash = orr_sim::build_hash_of(1, 0);
    cfg.min_players_to_start = 3;
    cfg.reserve_ticks = reserve_ticks;
    assert_eq!(u64::from(cfg.signal_lead_ticks), LEAD);
    cfg
}

fn harness(cfg: RoomConfig) -> Harness<Roster> {
    let script = Rc::new(|client: usize, tick: u64| (RosterInput { dx: ((tick / 10 + client as u64) % 3) as i32 - 1 }, Vec::new()));
    Harness::<Roster>::new(11, cfg, script, |_| ())
}

fn census(h: &Harness<Roster>, i: usize) -> Census {
    *h.clients[i].client.session().unwrap().verified_frame().expect("a verified frame").singleton::<Census>()
}

fn verified_tick(h: &Harness<Roster>, i: usize) -> u64 {
    h.clients[i].client.session().unwrap().verified_tick()
}

/// The signals client `i` verified: tick, kind and slot.
fn verified_signals(h: &mut Harness<Roster>, i: usize) -> Vec<(u64, SignalKind, u8)> {
    h.clients[i]
        .client
        .take_signal_notes()
        .into_vec()
        .into_iter()
        .filter_map(|(key, status)| match status {
            EventStatus::Verified(s) => Some((key.tick, s.kind, s.slot.0)),
            _ => None,
        })
        .collect()
}

/// Steps until `found` returns true for the server notes of a step (at most
/// 5 seconds of virtual time).
fn step_until_note(h: &mut Harness<Roster>, mut found: impl FnMut(&ServerNote) -> bool) {
    for _ in 0..5000 {
        h.step();
        if h.server.drain_notes().iter().any(&mut found) {
            return;
        }
    }
    panic!("the server note did not come");
}

/// Everything the server sends to a client takes `hold_us`.
fn held(p: PathParams, hold_us: u64) -> PathParams {
    PathParams { up: p.up, down: LinkParams::new(hold_us, 0) }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Signal {
    Disconnected,
    Reconnected,
    Removed,
}

/// Three players. The signal of `which` is announced while B hears nothing
/// from the server for `hold_us`. Returns nothing: it asserts that B does
/// not verify the signal's tick before the signal arrives, and that A and B
/// end in the same state.
fn run(which: Signal, hold_us: u64) {
    let p = path(20, 2, 20_000);
    let reserve = 120;
    let mut h = harness(room(if which == Signal::Removed { reserve } else { 600 }));
    for slot in 0..3 {
        h.add_client(&ClientSpec { want_slot: Some(PlayerSlot(slot)), ..ClientSpec::new(p) });
    }
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(200, 60_000_000));
    h.server.drain_notes();

    // C leaves. For the later signals the room goes on without it for a while.
    let token = h.clients[C].client.token().unwrap();
    let mut left_at = 0;
    if which == Signal::Disconnected {
        h.set_path(B, held(p, hold_us));
    }
    h.cut(C);
    step_until_note(&mut h, |n| match n {
        ServerNote::PlayerLeft { slot: 2, from_tick, .. } => {
            left_at = *from_tick;
            true
        }
        _ => false,
    });

    // The tick of the signal, and the moment the server sent it.
    let signal_tick = match which {
        Signal::Disconnected => left_at + LEAD,
        Signal::Reconnected => {
            h.set_path(B, p);
            let quiet = left_at + 120;
            assert!(h.run_until_tick(quiet, 60_000_000));
            let back = h.add_client(&ClientSpec { want_slot: Some(PlayerSlot(2)), token, ..ClientSpec::new(p) });
            assert_eq!(back, 3);
            // B is cut off from the moment C starts to catch up.
            while *h.clients[back].client.state() != ClientState::CatchingUp {
                h.step();
                assert!(h.now_us < 60_000_000, "the returning client never started to catch up");
            }
            h.set_path(B, held(p, hold_us));
            let mut from = 0;
            h.server.drain_notes();
            step_until_note(&mut h, |n| match n {
                ServerNote::PlayerJoined { slot: 2, from_tick, .. } => {
                    from = *from_tick;
                    true
                }
                _ => false,
            });
            from + LEAD
        }
        Signal::Removed => {
            h.set_path(B, p);
            // The seat runs out `reserve` ticks after the player left.
            let until = left_at - 1 + u64::from(reserve);
            assert!(h.run_until_tick(until - 3, 60_000_000));
            h.set_path(B, held(p, hold_us));
            while h.server.finalized_tick(h.room).unwrap() < until {
                h.step();
            }
            h.server.finalized_tick(h.room).unwrap() + 1 + LEAD
        }
    };
    let sent_at = h.now_us;
    // The network is normal again: what the server sends from now on is fast.
    h.set_path(B, p);

    // While B waits for the signal, the bundles of the signal's tick and of
    // the ticks after it reach B. B must not verify them. A has the signal.
    while h.now_us + 5_000 < sent_at + hold_us {
        h.step();
        assert!(verified_tick(&h, B) < signal_tick, "B verified tick {} before it had the signal of tick {signal_tick}", verified_tick(&h, B));
    }
    if hold_us > 200_000 {
        // The hold is longer than the lead (10 ticks and the trip of the bundles).
        assert!(verified_tick(&h, A) > signal_tick, "A should have gone past the signal tick {signal_tick}");
        assert!(h.clients[B].client.source_stats().bundles_fenced > 0, "no bundle of B waited for the signal");
    }

    // The signal arrives, and B catches up.
    h.run_for_us(hold_us + 3_000_000);

    let mut alive = vec![A, B];
    if which == Signal::Reconnected {
        alive.push(3);
    }
    let expected = match which {
        Signal::Disconnected => Census { added: 3, disconnected: 1, reconnected: 0, removed: 0 },
        Signal::Reconnected => Census { added: 3, disconnected: 1, reconnected: 1, removed: 0 },
        Signal::Removed => Census { added: 3, disconnected: 1, reconnected: 0, removed: 1 },
    };
    for &i in &alive {
        let c = &h.clients[i].client;
        assert_eq!(*c.state(), ClientState::Playing, "client {i}");
        assert_eq!(c.stats().signals_late, 0, "client {i}");
        assert!(verified_tick(&h, i) > signal_tick + 60, "client {i} has not gone on");
        assert_eq!(census(&h, i), expected, "client {i}");
    }
    assert_clients_agree(&h);
    // A and B verified the same signals at the same ticks.
    let a = verified_signals(&mut h, A);
    assert_eq!(verified_signals(&mut h, B), a);
    assert!(a.iter().any(|s| s.0 == signal_tick), "{a:?}");
}

#[test]
fn a_disconnect_held_back_for_longer_than_the_lead_does_not_desync() {
    run(Signal::Disconnected, 300_000);
}

#[test]
fn a_disconnect_held_back_for_a_very_long_time_does_not_desync() {
    run(Signal::Disconnected, 1_500_000);
}

#[test]
fn a_reconnect_held_back_does_not_desync() {
    run(Signal::Reconnected, 300_000);
}

#[test]
fn a_reconnect_held_back_for_a_very_long_time_does_not_desync() {
    run(Signal::Reconnected, 1_500_000);
}

#[test]
fn a_removal_held_back_does_not_desync() {
    run(Signal::Removed, 300_000);
}

#[test]
fn a_removal_held_back_for_a_very_long_time_does_not_desync() {
    run(Signal::Removed, 1_500_000);
}

#[test]
fn a_disconnect_held_back_for_less_than_the_lead_still_works() {
    run(Signal::Disconnected, 60_000);
}
