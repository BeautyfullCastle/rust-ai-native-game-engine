//! Step 4 without a window or a socket: a relay server and two clients on the simulated network
//! of `orr_server::harness`. The `Added` signals reach the game on both clients, and both
//! clients compute the same game.

mod common;

use std::rc::Rc;

use common::{assert_clients_agree, path, room, verified, SEED, TICK_RATE};
use orr_server::harness::{ClientSpec, Harness};
use orr_session::{PlayConfig, PlaySession};
use orr_sim::{Entity, Frame, PlayerLink, PlayerSlot, FP};
use star_pick::step4::{Choice, Move, Pos, Setup, StarPick, Tint, PLAYERS, SEAT_COLOR};

/// What each player does: client 0 walks along x, client 1 along y, back and forth.
fn harness() -> Harness<StarPick> {
    let script = Rc::new(|client: usize, tick: u64| {
        let direction = if tick / 30 % 2 == 0 { 1 } else { -1 };
        let input = if client == 0 { Move { dx: direction, dy: 0 } } else { Move { dx: 0, dy: direction } };
        (input, Vec::new())
    });
    Harness::<StarPick>::new(11, room(PLAYERS, std::mem::size_of::<Move>()), script, |_welcome| Setup::default())
}

fn avatar(frame: &Frame, slot: u8) -> Option<(Entity, (FP, FP))> {
    let (entities, links) = frame.dense::<PlayerLink>();
    let at = links.iter().position(|link| link.player == PlayerSlot(slot))?;
    let pos = frame.get::<Pos>(entities[at])?;
    Some((entities[at], (pos.x, pos.y)))
}

#[test]
fn added_gives_every_player_an_avatar_on_every_client() {
    let mut h = harness();
    for slot in 0..PLAYERS {
        h.add_client(&ClientSpec { want_slot: Some(PlayerSlot(slot)), ..ClientSpec::new(path(30, 4, 20_000)) });
    }
    assert!(h.run_until_all_playing(10_000_000), "both clients join and the game starts");
    assert!(h.run_until_tick(300, 60_000_000));
    h.run_for_us(500_000);

    for i in 0..2 {
        let frame = verified(&h, i);
        assert_eq!(frame.dense::<PlayerLink>().1.len(), 2, "client {i}: one avatar for each player");
        let (zero, one) = (avatar(frame, 0).expect("avatar 0").1, avatar(frame, 1).expect("avatar 1").1);
        // The steering of each client moved its own avatar, and only that one.
        assert_ne!(zero.0, FP::from_int(-2), "client {i}: avatar 0 moved along x");
        assert_eq!(zero.1, FP::ZERO, "client {i}: avatar 0 never moved along y");
        assert_ne!(one.1, FP::ZERO, "client {i}: avatar 1 moved along y");
        assert_eq!(one.0, FP::from_int(2), "client {i}: avatar 1 never moved along x");
    }
    let checked_together = assert_clients_agree(&h);
    assert!(checked_together > 5, "the clients compared {checked_together} checksums");
}

#[test]
fn without_a_server_the_app_announces_every_player_in_tick_one() {
    // `App` does this offline: `Added` for every slot, before the first tick runs.
    let mut session = PlaySession::<StarPick>::new(PlayConfig::new(PLAYERS, SEED, TICK_RATE), Setup::default());
    session.announce_players().unwrap();
    assert_eq!(session.frame().dense::<PlayerLink>().1.len(), 0, "no avatar before the first tick");
    session.step_now().unwrap();
    let frame = session.frame();
    assert_eq!(frame.dense::<PlayerLink>().1.len(), 2);
    assert!(avatar(frame, 0).is_some() && avatar(frame, 1).is_some());
}

/// The color index on the avatar of `slot`.
fn tint(frame: &Frame, slot: u8) -> Option<u32> {
    let (entity, _) = avatar(frame, slot)?;
    frame.get::<Tint>(entity).map(|tint| tint.color)
}

#[test]
fn the_color_a_window_picks_reaches_the_avatar_on_every_client() {
    let mut h = harness();
    // Window 0 picks color 3, window 1 picks nothing and gets the color of its seat (1).
    let picks = [Some(Choice { color: 3 }), None];
    for (slot, pick) in picks.into_iter().enumerate() {
        let player_data = pick.map(|choice| bytemuck::bytes_of(&choice).to_vec()).unwrap_or_default();
        h.add_client(&ClientSpec { want_slot: Some(PlayerSlot(slot as u8)), player_data, ..ClientSpec::new(path(30, 4, 20_000)) });
    }
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(200, 60_000_000));
    h.run_for_us(500_000);

    for i in 0..2 {
        let frame = verified(&h, i);
        assert_eq!((tint(frame, 0), tint(frame, 1)), (Some(3), Some(1)), "client {i}");
    }
    assert!(assert_clients_agree(&h) > 5);
}

#[test]
fn without_a_server_the_start_data_of_a_slot_gives_its_avatar_color() {
    let mut session = PlaySession::<StarPick>::new(PlayConfig::new(PLAYERS, SEED, TICK_RATE), Setup::default());
    session.announce_players_with(|slot| bytemuck::bytes_of(&Choice { color: 2 + u32::from(slot.0) }).to_vec()).unwrap();
    session.step_now().unwrap();
    assert_eq!((tint(session.frame(), 0), tint(session.frame(), 1)), (Some(2), Some(3)));
    // A pick that is no color (and the default) means the color of the seat.
    let mut session = PlaySession::<StarPick>::new(PlayConfig::new(PLAYERS, SEED, TICK_RATE), Setup::default());
    session.announce_players_with(|_| bytemuck::bytes_of(&Choice { color: SEAT_COLOR - 1 }).to_vec()).unwrap();
    session.step_now().unwrap();
    assert_eq!((tint(session.frame(), 0), tint(session.frame(), 1)), (Some(0), Some(1)));
}
