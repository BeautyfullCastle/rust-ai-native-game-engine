//! Start data of a player over the relay: a client sends it in its `Hello`, the server puts it in
//! the `Added` signal of every client, and the sims stay equal. Real server and clients on the
//! simulated network of the harness, with the `Roster` game (its avatar keeps the color of the
//! start data).

mod common;

use std::collections::BTreeMap;
use std::rc::Rc;

use common::{assert_clients_agree, path, SEED, TICK_RATE};
use orr_proto::{RejectReason, MAX_PLAYER_DATA};
use orr_server::harness::{ClientSpec, Harness};
use orr_server::RoomConfig;
use orr_session::{ClientState, EventStatus};
use orr_sim::{PlayerSlot, SignalKind};
use orr_testgame::roster::{Avatar, Profile, Roster, RosterInput};

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

fn profile(color: u32) -> Vec<u8> {
    bytemuck::bytes_of(&Profile { color, _pad: 0 }).to_vec()
}

fn spec(slot: u8, color: u32) -> ClientSpec {
    ClientSpec { want_slot: Some(PlayerSlot(slot)), player_data: profile(color), ..ClientSpec::new(path(30, 4, 20_000)) }
}

/// The color of every avatar in the verified frame of client `i`, by slot.
fn colors(h: &Harness<Roster>, i: usize) -> BTreeMap<u32, u32> {
    let frame = h.clients[i].client.session().unwrap().verified_frame().expect("a verified frame");
    frame.dense::<Avatar>().1.iter().map(|a| (a.slot, a.color)).collect()
}

/// The `Added` signals client `i` verified: slot and start data.
fn added(h: &mut Harness<Roster>, i: usize) -> Vec<(u8, Vec<u8>)> {
    let notes = h.clients[i].client.take_signal_notes().into_vec();
    notes
        .into_iter()
        .filter_map(|(_, status)| match status {
            EventStatus::Verified(s) if s.kind == SignalKind::Added => Some((s.slot.0, s.data)),
            _ => None,
        })
        .collect()
}

#[test]
fn two_clients_get_each_others_start_data_in_the_same_signals_and_agree() {
    let mut h = harness(room(2, 2));
    h.add_client(&spec(0, 7));
    h.add_client(&spec(1, 9));
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(200, 60_000_000));
    h.run_for_us(500_000);

    let expected = vec![(0, profile(7)), (1, profile(9))];
    assert_eq!(added(&mut h, 0), expected);
    assert_eq!(added(&mut h, 1), expected, "the player gets its own data back, as everybody else does");
    for i in 0..2 {
        assert_eq!(colors(&h, i), BTreeMap::from([(0, 7), (1, 9)]), "client {i}");
    }
    assert_clients_agree(&h);
}

#[test]
fn a_late_joiner_sees_the_data_of_the_players_before_it_and_they_see_its_data() {
    let mut h = harness(room(4, 2));
    h.add_client(&spec(0, 7));
    h.add_client(&spec(1, 9));
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(200, 60_000_000));

    // Two players come into the running room at nearly the same time. The server serves the joins
    // one after the other, and sends an `Added` that is newer than a snapshot again from its log.
    let (first, second) = (h.add_client(&spec(2, 11)), h.add_client(&spec(3, 13)));
    assert!(h.run_until_all_playing(30_000_000));
    let t = h.server.finalized_tick(h.room).unwrap();
    assert!(h.run_until_tick(t + 300, 60_000_000));
    h.run_for_us(500_000);

    let everybody = BTreeMap::from([(0, 7), (1, 9), (2, 11), (3, 13)]);
    for i in [0, 1, first, second] {
        assert_eq!(colors(&h, i), everybody, "client {i}");
        assert_eq!(h.clients[i].client.stats().signals_late, 0, "client {i}");
    }
    assert_clients_agree(&h);
}

#[test]
fn a_player_that_comes_back_with_its_token_keeps_the_data_of_its_seat() {
    let mut h = harness(room(2, 2));
    h.add_client(&spec(0, 7));
    h.add_client(&spec(1, 9));
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(200, 60_000_000));

    let token = h.clients[1].client.token().unwrap();
    h.cut(1);
    let cut_tick = h.server.finalized_tick(h.room).unwrap();
    assert!(h.run_until_tick(cut_tick + 120, 60_000_000));

    // The window comes back and states other start data. The seat was announced already: `Reconnected`, no new `Added`.
    let back = h.add_client(&ClientSpec { token, ..spec(1, 3) });
    assert!(h.run_until_all_playing_except(&[1], 20_000_000));
    let t = h.server.finalized_tick(h.room).unwrap();
    assert!(h.run_until_tick(t + 300, 60_000_000));
    h.run_for_us(500_000);

    for i in [0, back] {
        assert_eq!(colors(&h, i), BTreeMap::from([(0, 7), (1, 9)]), "client {i}");
    }
    assert_clients_agree(&h);
}

#[test]
fn start_data_above_the_limit_is_rejected_with_its_size() {
    let mut h = harness(room(2, 2));
    let too_big = h.add_client(&ClientSpec { player_data: vec![1; MAX_PLAYER_DATA + 1], ..ClientSpec::new(path(10, 0, 0)) });
    let at_limit = h.add_client(&ClientSpec { player_data: vec![2; MAX_PLAYER_DATA], ..ClientSpec::new(path(10, 0, 0)) });
    h.run_for_us(2_000_000);

    let reason = RejectReason::PlayerDataTooLarge { max: MAX_PLAYER_DATA as u32, len: MAX_PLAYER_DATA as u32 + 1 };
    assert_eq!(*h.clients[too_big].client.state(), ClientState::Rejected(reason));
    assert!(h.clients[too_big].client.welcome().is_none(), "no seat for a refused Hello");
    assert!(h.clients[at_limit].client.welcome().is_some(), "the largest allowed data is accepted");
    assert!(!matches!(h.clients[at_limit].client.state(), ClientState::Rejected(_)));
}
