//! Step 2 without a window: scoring is deterministic, a replay reproduces a run, and the
//! views follow the game. A player that chases the nearest star plays the game.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use common::{chase, SEED, TICK_RATE};
use orr_app::{AudioConfig, FrameView, OfflineAudio, Views};
use orr_bridge::{Bridge, BridgeConfig, InProc, PlayHost};
use orr_session::{PlayConfig, PlaySession};
use orr_sim::{Frame, PlayerSlot};
use star_pick::sound;
use star_pick::step2::{Juice, Move, Pos, Score, Shared, Star, StarPick, TrailView};

const TICKS: u64 = 900;

fn steer_frame(frame: &Frame) -> Move {
    let (entities, scores) = frame.dense::<Score>();
    assert_eq!((entities.len(), scores.len()), (1, 1), "one player");
    let at = |e| frame.get::<Pos>(e).map(|pos| (pos.x, pos.y)).unwrap();
    let stars: Vec<_> = frame.dense::<Star>().0.iter().map(|&star| at(star)).collect();
    let (dx, dy) = chase(at(entities[0]), &stars);
    Move { dx, dy }
}

fn steer_view(view: FrameView<'_>) -> Move {
    let (mut me, mut stars) = (None, Vec::new());
    for (entity, pos) in view.iter::<Pos>() {
        if view.has::<Star>(entity) {
            stars.push((pos.x, pos.y));
        } else {
            me = Some((pos.x, pos.y));
        }
    }
    let (dx, dy) = chase(me.expect("the player"), &stars);
    Move { dx, dy }
}

/// Plays `TICKS` ticks with the chasing player. Returns the session and how many `Picked` events came.
fn play() -> (PlaySession<StarPick>, u32) {
    let mut session = PlaySession::<StarPick>::new(PlayConfig::new(1, SEED, TICK_RATE), ());
    let mut picked = 0;
    for _ in 0..TICKS {
        let input = steer_frame(session.frame());
        session.set_input(PlayerSlot(0), input);
        picked += session.step_now().expect("a recording session runs").len() as u32;
    }
    (session, picked)
}

fn points(session: &PlaySession<StarPick>) -> u32 {
    session.frame().dense::<Score>().1[0].points
}

#[test]
fn scoring_is_deterministic() {
    let (first, picked) = play();
    let (second, picked_again) = play();
    assert!(points(&first) >= 5, "the chasing player picks stars: {} points", points(&first));
    assert_eq!(picked, points(&first), "one event for every point");
    assert_eq!((picked, points(&first)), (picked_again, points(&second)));
    assert_eq!(first.simulation().checksum(), second.simulation().checksum());
}

#[test]
fn a_replay_reproduces_the_run() {
    let (recorded, _) = play();
    let bytes = recorded.save_replay();

    let mut replay = PlaySession::<StarPick>::open_replay(&bytes, (), 0).expect("the replay opens");
    replay.seek(TICKS).expect("seek to the last tick");
    assert_eq!(replay.simulation().tick(), TICKS);
    assert_eq!(replay.simulation().checksum(), recorded.simulation().checksum());
    assert_eq!(points(&replay), points(&recorded));
}

#[test]
fn the_views_follow_the_game() {
    // The same pieces the app uses offline: a play session in a bridge, and the views on top.
    let fx = Shared::default();
    let mut views = Views::<StarPick>::new();
    // The sounds go to an offline output, so the test can count them.
    let audio = Rc::new(RefCell::new(OfflineAudio::new_offline(AudioConfig::default(), 44_100).unwrap()));
    views.set_sound_out(audio.clone());
    views.sound(sound::PICKUP, sound::pickup());
    let for_trails = fx.clone();
    views.entity_view::<Score>(move || TrailView { fx: for_trails.clone() });
    views.scene_view(Juice { fx: fx.clone() });

    let session = PlaySession::<StarPick>::new(PlayConfig::new(1, SEED, TICK_RATE), ());
    let mut bridge = InProc::new(PlayHost::new(session, PlayerSlot(0)), BridgeConfig::default());
    let mut saw_ring = false;
    let mut block = [0.0f32; 1470]; // 735 stereo frames: a 60th of a second at 44.1 kHz
    for _ in 0..TICKS {
        if let Some(snapshot) = bridge.snapshot() {
            let input = steer_view(snapshot.predicted());
            bridge.set_input(PlayerSlot(0), input).unwrap();
        }
        bridge.update(Duration::from_secs(1) / TICK_RATE);
        let update = bridge.poll_view();
        views.dispatch(&mut bridge, &update, &[]);
        audio.borrow_mut().render(&mut block).unwrap(); // one tick of sound time passes
        saw_ring |= !fx.borrow().rings.is_empty();
    }

    let snapshot = bridge.snapshot().expect("a snapshot");
    let verified = snapshot.verified().expect("a verified frame");
    let score = verified.iter::<Score>().next().expect("the player").1.points;
    assert!(score >= 5);
    assert_eq!(fx.borrow().score, score, "the score shows the verified score");
    assert_eq!(views.overlay().texts().len(), 1, "the score is one piece of text");
    assert_eq!(views.overlay().texts()[0].text, format!("SCORE {score}"));
    assert_eq!(audio.borrow().stats().started, u64::from(score), "every pick-up played its sound once");
    assert!(saw_ring, "a pick-up started the ring");
    assert_eq!(fx.borrow().trails.len(), 1, "the entity view was created for the player");
}
