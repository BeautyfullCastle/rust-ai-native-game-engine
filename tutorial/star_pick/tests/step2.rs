//! Step 2 without a window: scoring is deterministic, a replay reproduces a run, and the
//! views follow the game. A player that chases the nearest star plays the game.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use common::{chase, SEED, TICK_RATE};
use orr_app::{Action, AudioConfig, FrameView, OfflineAudio, Screen, Views};
use orr_bridge::{Bridge, BridgeConfig, InProc, PlayHost};
use orr_session::{PlayConfig, PlaySession};
use orr_sim::{Frame, PlayerSlot};
use star_pick::sound;
use star_pick::step2::{screens, Juice, Move, Pos, Restart, Score, Shared, Star, StarPick, TrailView, WIN_STARS};

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

#[test]
fn a_restart_command_empties_the_round_and_a_replay_reproduces_it() {
    const RESTART_AT: u64 = 400;
    let mut session = PlaySession::<StarPick>::new(PlayConfig::new(1, SEED, TICK_RATE), ());
    for tick in 0..TICKS {
        if tick == RESTART_AT {
            assert!(points(&session) >= 1, "a round to throw away");
            session.push_command(PlayerSlot(0), Restart::default()).unwrap();
        }
        let input = steer_frame(session.frame());
        session.set_input(PlayerSlot(0), input);
        session.step_now().expect("a recording session runs");
        if tick == RESTART_AT {
            assert_eq!(points(&session), 0, "the score starts again");
            assert!(session.frame().dense::<Star>().0.len() <= 1, "the stars are gone (one may appear in the same tick)");
        }
    }
    let mut replay = PlaySession::<StarPick>::open_replay(&session.save_replay(), (), 0).expect("the replay opens");
    replay.seek(TICKS).expect("seek to the last tick");
    assert_eq!(replay.simulation().checksum(), session.simulation().checksum(), "the restart is part of the recorded game");
}

/// The player of this frame (the entity that is not a star).
fn player_at(view: FrameView<'_>) -> (orr_sim::FP, orr_sim::FP) {
    view.iter::<Pos>().find(|(entity, _)| !view.has::<Star>(*entity)).map(|(_, pos)| (pos.x, pos.y)).expect("the player")
}

#[test]
fn the_screens_start_pause_win_and_restart_a_game_that_only_sees_neutral_input() {
    let (fx, screens) = (Shared::default(), screens());
    fx.borrow_mut().screens = Some(screens.clone());
    let mut views = Views::<StarPick>::new();
    views.scene_view(Juice { fx: fx.clone() });
    let session = PlaySession::<StarPick>::new(PlayConfig::new(1, SEED, TICK_RATE), ());
    let mut bridge = InProc::new(PlayHost::new(session, PlayerSlot(0)), BridgeConfig::default());
    // One frame of the window: the input (the chasing player, neutral on a screen), the tick, the views.
    let mut frames = |bridge: &mut InProc<StarPick, PlayHost<StarPick>>, n: u32| {
        for _ in 0..n {
            if let Some(snapshot) = bridge.snapshot() {
                let input = if screens.is_playing() { steer_view(snapshot.predicted()) } else { Move::default() };
                bridge.set_input(PlayerSlot(0), input).unwrap();
            }
            bridge.update(Duration::from_secs(1) / TICK_RATE);
            let update = bridge.poll_view();
            views.dispatch(bridge, &update, &[]);
        }
        let snapshot = bridge.snapshot().expect("a snapshot");
        (player_at(snapshot.predicted()), views.overlay().texts().iter().map(|t| t.text.clone()).collect::<Vec<_>>())
    };

    // Title: the words are up and the square stays in the middle.
    let (at, texts) = frames(&mut bridge, 60);
    assert_eq!(screens.screen(), Screen::Title);
    assert!(texts.contains(&"STAR PICK".to_string()), "{texts:?}");
    assert_eq!(at, (orr_sim::FP::ZERO, orr_sim::FP::ZERO));

    // Start: the first round begins with the reset command, and the screen words are gone.
    assert!(screens.act(Action::Confirm));
    let (_, texts) = frames(&mut bridge, 5);
    assert_eq!(texts.len(), 1, "only the score: {texts:?}");

    // Pause: the square holds still while the sim keeps ticking.
    frames(&mut bridge, 100);
    assert!(screens.act(Action::Pause));
    let (held, texts) = frames(&mut bridge, 1);
    assert!(texts.contains(&"PAUSED".to_string()));
    let tick = bridge.snapshot().unwrap().tick();
    let (still, _) = frames(&mut bridge, 30);
    assert_eq!(still, held, "paused: neutral input");
    assert_eq!(bridge.snapshot().unwrap().tick(), tick + 30, "the clock does not stop");
    assert!(screens.act(Action::Pause));

    // Win: the fifth star stops the game on the win screen.
    let mut waited = 0;
    while screens.screen() == Screen::Playing {
        frames(&mut bridge, 10);
        waited += 10;
        assert!(waited < 6000, "the chasing player wins");
    }
    assert_eq!(screens.screen(), Screen::Win);
    let (won_at, texts) = frames(&mut bridge, 1);
    assert!(texts.contains(&"YOU WIN".to_string()), "{texts:?}");
    assert!(fx.borrow().score >= WIN_STARS, "the win came from a verified score of {}", fx.borrow().score);
    assert_eq!(frames(&mut bridge, 40).0, won_at, "the square holds still on the win screen");

    // Restart: a new round, the score is back to 0, and the game takes input again.
    assert!(screens.act(Action::Restart));
    frames(&mut bridge, 10);
    assert_eq!(screens.screen(), Screen::Playing);
    assert_eq!(fx.borrow().score, 0, "the reset command reached the game");
}
