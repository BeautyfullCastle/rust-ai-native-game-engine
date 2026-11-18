//! Step 5 without a window or a socket: a relay server and clients on the simulated network.
//! `Disconnected`, `Reconnected` and `Removed` reach the game, and every client, also the one
//! that joins late from a snapshot, computes the same game.

mod common;

use std::rc::Rc;

use common::{assert_clients_agree, path, room, verified};
use orr_server::harness::{ClientSpec, Harness};
use orr_sim::{Entity, Frame, PlayerLink, PlayerSlot, FP};
use star_pick::step5::{Away, Move, Pos, Setup, StarPick, PLAYERS};

/// Everybody walks back and forth: client 0 along x, the others along y. A moving avatar is then
/// easy to tell from a frozen one. (The client that takes slot 1 after client 1 left is the
/// third one.)
fn harness(reserve_ticks: Option<u32>) -> Harness<StarPick> {
    let script = Rc::new(|client: usize, tick: u64| {
        let direction = if tick / 30 % 2 == 0 { 1 } else { -1 };
        let input = if client == 0 { Move { dx: direction, dy: 0 } } else { Move { dx: 0, dy: direction } };
        (input, Vec::new())
    });
    let mut cfg = room(PLAYERS, std::mem::size_of::<Move>());
    if let Some(ticks) = reserve_ticks {
        cfg.reserve_ticks = ticks;
    }
    Harness::<StarPick>::new(11, cfg, script, |_welcome| Setup::default())
}

fn join_both(h: &mut Harness<StarPick>) {
    for slot in 0..PLAYERS {
        h.add_client(&ClientSpec { want_slot: Some(PlayerSlot(slot)), ..ClientSpec::new(path(30, 4, 20_000)) });
    }
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(200, 60_000_000));
}

fn avatar(frame: &Frame, slot: u8) -> Option<(Entity, (FP, FP))> {
    let (entities, links) = frame.dense::<PlayerLink>();
    let at = links.iter().position(|link| link.player == PlayerSlot(slot))?;
    let pos = frame.get::<Pos>(entities[at])?;
    Some((entities[at], (pos.x, pos.y)))
}

fn is_away(frame: &Frame, slot: u8) -> bool {
    let (entity, _) = avatar(frame, slot).expect("the avatar exists");
    frame.has::<Away>(entity)
}

/// The position of the avatar of `slot` in the verified frame of client `i`, sampled `count`
/// times with 10 ticks between the samples.
fn positions(h: &mut Harness<StarPick>, i: usize, slot: u8, count: usize) -> Vec<(FP, FP)> {
    let mut out = Vec::new();
    for _ in 0..count {
        let tick = h.server.finalized_tick(h.room).unwrap();
        assert!(h.run_until_tick(tick + 10, 60_000_000));
        out.push(avatar(verified(h, i), slot).expect("the avatar exists").1);
    }
    out
}

#[test]
fn a_returning_player_is_frozen_while_away_and_moves_again_after_reconnecting() {
    let mut h = harness(None);
    join_both(&mut h);
    assert!(!is_away(verified(&h, 0), 1));

    // Client 1 loses its connection. Its seat stays reserved (600 ticks by default).
    let token = h.clients[1].client.token().unwrap();
    h.cut(1);
    let cut_tick = h.server.finalized_tick(h.room).unwrap();
    assert!(h.run_until_tick(cut_tick + 120, 60_000_000));
    h.run_for_us(500_000);

    // `Disconnected` reached the game of client 0: the avatar is marked and frozen.
    assert!(is_away(verified(&h, 0), 1), "Disconnected marks the avatar");
    let frozen = positions(&mut h, 0, 1, 8);
    assert!(frozen.iter().all(|at| *at == frozen[0]), "an away avatar does not move: {frozen:?}");
    let walking = positions(&mut h, 0, 0, 8);
    assert!(walking.iter().any(|at| *at != walking[0]), "the other avatar keeps walking: {walking:?}");

    // It comes back with its token and gets a snapshot of the game.
    let back = h.add_client(&ClientSpec { want_slot: Some(PlayerSlot(1)), token, ..ClientSpec::new(path(30, 4, 20_000)) });
    assert!(h.run_until_all_playing_except(&[1], 20_000_000));
    assert_eq!(h.clients[back].client.welcome().unwrap().slot, 1);
    let t = h.server.finalized_tick(h.room).unwrap();
    assert!(h.run_until_tick(t + 300, 60_000_000));
    h.run_for_us(500_000);

    // `Reconnected` reached both clients that are playing: the mark is gone and the avatar moves.
    for i in [0, back] {
        assert!(!is_away(verified(&h, i), 1), "client {i}: Reconnected removes the mark");
        let moving = positions(&mut h, i, 1, 8);
        assert!(moving.iter().any(|at| *at != moving[0]), "client {i}: the avatar moves again: {moving:?}");
    }
    // The client that came from a snapshot and the one that stayed compute the same game.
    let checked_together = assert_clients_agree(&h);
    assert!(checked_together > 5, "the clients compared {checked_together} checksums");
}

#[test]
fn a_seat_that_runs_out_removes_the_avatar_and_a_new_player_gets_one() {
    // The seat is kept for 120 ticks only.
    let mut h = harness(Some(120));
    join_both(&mut h);

    h.cut(1);
    let cut_tick = h.server.finalized_tick(h.room).unwrap();
    assert!(h.run_until_tick(cut_tick + 300, 60_000_000));
    h.run_for_us(500_000);
    let frame = verified(&h, 0);
    assert_eq!(frame.dense::<PlayerLink>().1.len(), 1, "Removed took the avatar of slot 1 out");
    assert!(avatar(frame, 1).is_none());

    // A new player takes the free seat: `Added` makes a new avatar, also on the new client.
    let fresh = h.add_client(&ClientSpec::new(path(30, 4, 20_000)));
    assert!(h.run_until_all_playing_except(&[1], 20_000_000));
    assert_eq!(h.clients[fresh].client.welcome().unwrap().slot, 1);
    let t = h.server.finalized_tick(h.room).unwrap();
    assert!(h.run_until_tick(t + 300, 60_000_000));
    h.run_for_us(500_000);
    for i in [0, fresh] {
        let frame = verified(&h, i);
        assert_eq!(frame.dense::<PlayerLink>().1.len(), 2, "client {i}: two avatars again");
        assert!(!is_away(frame, 1));
    }
    let checked_together = assert_clients_agree(&h);
    assert!(checked_together > 5, "the clients compared {checked_together} checksums");
}
