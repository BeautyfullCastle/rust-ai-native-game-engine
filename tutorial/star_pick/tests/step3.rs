//! Step 3 without a window: two local slots steer their own squares, and two runs agree.

mod common;

use common::{chase, SEED, TICK_RATE};
use orr_session::{PlayConfig, PlaySession};
use orr_sim::{fp, Entity, Frame, PlayerLink, PlayerSlot, FP};
use star_pick::step3::{Move, Pos, Score, Star, StarPick, PLAYERS};

fn session() -> PlaySession<StarPick> {
    PlaySession::<StarPick>::new(PlayConfig::new(PLAYERS, SEED, TICK_RATE), ())
}

/// The entity and the position of the player of `slot`.
fn player(frame: &Frame, slot: u8) -> (Entity, (FP, FP)) {
    let (entities, links) = frame.dense::<PlayerLink>();
    let at = links.iter().position(|link| link.player == PlayerSlot(slot)).expect("a player for the slot");
    let pos = frame.get::<Pos>(entities[at]).unwrap();
    (entities[at], (pos.x, pos.y))
}

#[test]
fn the_two_slots_move_independently() {
    let mut session = session();
    let start = [player(session.frame(), 0).1, player(session.frame(), 1).1];
    assert_ne!(start[0], start[1], "each player starts on its own place");

    // Slot 0 walks right, slot 1 walks up.
    session.set_input(PlayerSlot(0), Move { dx: 1, dy: 0 });
    session.set_input(PlayerSlot(1), Move { dx: 0, dy: 1 });
    for _ in 0..10 {
        session.step_now().unwrap();
    }
    let (zero, one) = (player(session.frame(), 0).1, player(session.frame(), 1).1);
    let ten_steps = fp!(0.1).mul_int(10);
    assert_eq!(zero, (start[0].0 + ten_steps, start[0].1), "10 ticks to the right");
    assert_eq!(one, (start[1].0, start[1].1 + ten_steps), "10 ticks up");

    // Now only slot 1 keeps its input: slot 0 stands still.
    session.set_input(PlayerSlot(0), Move::default());
    for _ in 0..10 {
        session.step_now().unwrap();
    }
    assert_eq!(player(session.frame(), 0).1, zero);
    assert_ne!(player(session.frame(), 1).1, one);
}

#[test]
fn each_slot_scores_for_its_own_player() {
    let mut session = session();
    let mut picked_by = [0u32; 2];
    for _ in 0..1200 {
        // Slot 0 chases the nearest star. Slot 1 never moves, but a star can appear near it.
        let frame = session.frame();
        let at = |e| frame.get::<Pos>(e).map(|pos| (pos.x, pos.y)).unwrap();
        let stars: Vec<_> = frame.dense::<Star>().0.iter().map(|&star| at(star)).collect();
        let (dx, dy) = chase(player(frame, 0).1, &stars);
        session.set_input(PlayerSlot(0), Move { dx, dy });
        for event in session.step_now().unwrap() {
            picked_by[event.payload.slot as usize] += 1;
        }
    }
    let frame = session.frame();
    let points = |slot| frame.get::<Score>(player(frame, slot).0).unwrap().points;
    assert!(points(0) >= 5, "the chasing player scored: {}", points(0));
    assert_eq!(picked_by, [points(0), points(1)], "every event names the player whose score went up");
    assert!(points(0) > points(1), "the player who walks to the stars scores more than the one who waits");
}

#[test]
fn two_runs_end_the_same() {
    let run = || {
        let mut session = session();
        for tick in 0..600u64 {
            session.set_input(PlayerSlot(0), Move { dx: (tick / 40 % 3) as i32 - 1, dy: 1 });
            session.set_input(PlayerSlot(1), Move { dx: -1, dy: (tick / 25 % 3) as i32 - 1 });
            session.step_now().unwrap();
        }
        session.simulation().checksum()
    };
    assert_eq!(run(), run());
}
