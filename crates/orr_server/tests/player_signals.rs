//! Player signals over the relay: the server announces who is in the game,
//! and every client turns that into the same signals at the same ticks.
//! Real server and clients on the simulated network of the harness, with the
//! `Roster` game (its frame counts the signals it got).

mod common;

use std::rc::Rc;

use common::{assert_clients_agree, path, SEED, TICK_RATE};
use orr_server::harness::{ClientSpec, Harness};
use orr_server::RoomConfig;
use orr_session::{ClientEvent, EventStatus};
use orr_sim::{PlayerSlot, SignalKind};
use orr_testgame::roster::{Census, Roster, RosterInput};

type Seen = (u64, SignalKind, u8);

fn room(players: u8, min_to_start: u8) -> RoomConfig {
    let mut cfg = RoomConfig::new(players, TICK_RATE, SEED, std::mem::size_of::<RosterInput>() as u32);
    cfg.build_hash = orr_sim::build_hash_of(1, 0);
    cfg.min_players_to_start = min_to_start;
    cfg
}

fn harness(cfg: RoomConfig) -> Harness<Roster> {
    let script = Rc::new(|client: usize, tick: u64| (RosterInput { dx: ((tick / 10 + client as u64) % 3) as i32 - 1 }, Vec::new()));
    Harness::<Roster>::new(11, cfg, script, |_| ())
}

/// The signals the session of client `i` verified, in order, since it began.
fn verified(h: &mut Harness<Roster>, i: usize) -> Vec<Seen> {
    let batch = h.clients[i].client.take_signal_notes();
    batch
        .into_vec()
        .into_iter()
        .filter_map(|(key, status)| match status {
            EventStatus::Verified(s) => Some((key.tick, s.kind, s.slot.0)),
            _ => None,
        })
        .collect()
}

fn snapshot_tick(h: &mut Harness<Roster>, i: usize) -> u64 {
    h.clients[i]
        .client
        .drain_events()
        .into_iter()
        .find_map(|e| match e {
            ClientEvent::SnapshotLoaded { tick } => Some(tick),
            _ => None,
        })
        .expect("the client got a snapshot")
}

fn census(h: &Harness<Roster>, i: usize) -> Census {
    *h.clients[i].client.session().unwrap().verified_frame().expect("a verified frame").singleton::<Census>()
}

#[test]
fn a_returning_player_gives_disconnected_then_reconnected_at_the_same_ticks() {
    // With loss, reordering and duplicates on the unreliable channel.
    let p = path(30, 4, 20_000);
    let mut h = harness(room(2, 2));
    for slot in 0..2 {
        h.add_client(&ClientSpec { want_slot: Some(PlayerSlot(slot)), ..ClientSpec::new(p) });
    }
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(200, 60_000_000));

    // Client 1 drops off the network. Its seat stays reserved (600 ticks).
    let token = h.clients[1].client.token().unwrap();
    h.cut(1);
    let cut_tick = h.server.finalized_tick(h.room).unwrap();
    assert!(h.run_until_tick(cut_tick + 120, 60_000_000));

    // It comes back with its token.
    let back = h.add_client(&ClientSpec { want_slot: Some(PlayerSlot(1)), token, ..ClientSpec::new(p) });
    assert!(h.run_until_all_playing_except(&[1], 20_000_000));
    assert_eq!(h.clients[back].client.welcome().unwrap().slot, 1);
    let snap = snapshot_tick(&mut h, back);
    let t = h.server.finalized_tick(h.room).unwrap();
    assert!(h.run_until_tick(t + 300, 60_000_000));
    h.run_for_us(500_000);

    let stayed = verified(&mut h, 0);
    let (disconnected_at, reconnected_at) = (stayed[2].0, stayed[3].0);
    assert_eq!(
        stayed,
        vec![
            (1, SignalKind::Added, 0),
            (1, SignalKind::Added, 1),
            (disconnected_at, SignalKind::Disconnected, 1),
            (reconnected_at, SignalKind::Reconnected, 1),
        ]
    );
    assert!(disconnected_at > cut_tick && disconnected_at < reconnected_at, "{stayed:?}");

    // The returning client gets only what its snapshot does not hold.
    let returned = verified(&mut h, back);
    let expected: Vec<Seen> = stayed.iter().copied().filter(|s| s.0 > snap).collect();
    assert_eq!(returned, expected, "snapshot at tick {snap}");
    assert!(returned.contains(&(reconnected_at, SignalKind::Reconnected, 1)), "{returned:?}");

    // The client that dropped saw the start, and nothing else.
    assert_eq!(verified(&mut h, 1), stayed[..2].to_vec());

    for i in [0, back] {
        let s = h.clients[i].client.session().unwrap();
        assert!(s.verified_tick() > reconnected_at, "client {i} has not verified the reconnect");
        assert_eq!(census(&h, i), Census { added: 2, disconnected: 1, reconnected: 1, removed: 0 }, "client {i}");
        assert_eq!(h.clients[i].client.stats().signals_late, 0);
    }
    assert_clients_agree(&h);
    assert!(h.clients[0].client.session().unwrap().checksums().last().unwrap().0 > reconnected_at);
    assert!(h.clients[back].client.session().unwrap().checksums().last().unwrap().0 > reconnected_at);
}

#[test]
fn a_seat_that_runs_out_gives_removed_and_the_slot_can_be_taken_again() {
    let p = path(20, 2, 0);
    let mut cfg = room(2, 2);
    cfg.reserve_ticks = 120;
    let mut h = harness(cfg);
    for slot in 0..2 {
        h.add_client(&ClientSpec { want_slot: Some(PlayerSlot(slot)), ..ClientSpec::new(p) });
    }
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(200, 60_000_000));

    h.cut(1);
    let cut_tick = h.server.finalized_tick(h.room).unwrap();
    assert!(h.run_until_tick(cut_tick + 300, 60_000_000));

    // A new player takes the free slot.
    let fresh = h.add_client(&ClientSpec::new(p));
    assert!(h.run_until_all_playing_except(&[1], 20_000_000));
    assert_eq!(h.clients[fresh].client.welcome().unwrap().slot, 1);
    let snap = snapshot_tick(&mut h, fresh);
    let t = h.server.finalized_tick(h.room).unwrap();
    assert!(h.run_until_tick(t + 300, 60_000_000));
    h.run_for_us(500_000);

    let stayed = verified(&mut h, 0);
    let (disconnected_at, removed_at, added_at) = (stayed[2].0, stayed[3].0, stayed[4].0);
    assert_eq!(
        stayed,
        vec![
            (1, SignalKind::Added, 0),
            (1, SignalKind::Added, 1),
            (disconnected_at, SignalKind::Disconnected, 1),
            (removed_at, SignalKind::Removed, 1),
            (added_at, SignalKind::Added, 1),
        ]
    );
    assert!(removed_at >= disconnected_at + 120, "the seat is kept for the reserve time: {stayed:?}");
    assert!(removed_at < added_at);

    let joined = verified(&mut h, fresh);
    let expected: Vec<Seen> = stayed.iter().copied().filter(|s| s.0 > snap).collect();
    assert_eq!(joined, expected, "snapshot at tick {snap}");
    assert!(joined.contains(&(added_at, SignalKind::Added, 1)), "{joined:?}");

    for i in [0, fresh] {
        assert_eq!(census(&h, i), Census { added: 3, disconnected: 1, reconnected: 0, removed: 1 }, "client {i}");
    }
    assert_clients_agree(&h);
}

#[test]
fn every_joiner_is_added_once_at_the_same_tick_everywhere() {
    let p = path(30, 4, 0);
    let mut h = harness(room(3, 2));
    for slot in 0..2 {
        h.add_client(&ClientSpec { want_slot: Some(PlayerSlot(slot)), ..ClientSpec::new(p) });
    }
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(300, 60_000_000));
    let late = h.add_client(&ClientSpec::new(p));
    assert!(h.run_until_all_playing(20_000_000));
    assert_eq!(h.clients[late].client.welcome().unwrap().slot, 2);
    let snap = snapshot_tick(&mut h, late);
    let t = h.server.finalized_tick(h.room).unwrap();
    assert!(h.run_until_tick(t + 300, 60_000_000));
    h.run_for_us(500_000);

    let first = verified(&mut h, 0);
    let added_at = first[2].0;
    assert_eq!(first, vec![(1, SignalKind::Added, 0), (1, SignalKind::Added, 1), (added_at, SignalKind::Added, 2)]);
    assert!(added_at > 300, "the late joiner is added when it joins, not at the start");
    assert_eq!(verified(&mut h, 1), first);
    assert_eq!(verified(&mut h, late), vec![(added_at, SignalKind::Added, 2)], "snapshot at tick {snap}");
    for i in 0..3 {
        assert_eq!(census(&h, i), Census { added: 3, ..Census::default() }, "client {i}");
    }
    assert_clients_agree(&h);
}
