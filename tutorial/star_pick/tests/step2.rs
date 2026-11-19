//! Step 2 without a window: scoring is deterministic, a replay reproduces a run, and the
//! views follow the game. A player that chases the nearest star plays the game.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use common::{chase, SEED, TICK_RATE};
use bytemuck::Zeroable;
use orr_app::{clip_from_wav, Action, AudioConfig, FrameView, OfflineAudio, SaveStore, Screen, Views};
use orr_bridge::{Bridge, BridgeConfig, BridgeEvent, EventKey, EventStatus, InProc, PlayHost, ViewResync};
use orr_session::{PlayConfig, PlaySession};
use orr_sim::{Frame, PlayerSlot};
use star_pick::sound;
use star_pick::step2::{pickup_sound, screens, Juice, Move, Picked, Pos, Restart, Score, Shared, Star, StarPick, TrailView, GAME_ID, PICKUP_WAV, QUICK_SLOT, SAVE_VERSION, WIN_STARS};

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
    // The sound is a WAV file; the app plays it for every pick-up, the views have no sound code.
    views.sound(sound::PICKUP, clip_from_wav(PICKUP_WAV).expect("the tutorial sound is a valid WAV file"));
    views.on_event_sound(pickup_sound);
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
    assert_eq!(views.overlay().texts().len(), 2, "the score and the goal");
    assert_eq!(views.overlay().texts()[0].text, format!("점수 {score}"));
    assert_eq!(views.overlay().texts()[1].text, format!("별 {WIN_STARS}개를 모으세요"));
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
    assert!(texts.contains(&"별 줍기".to_string()), "{texts:?}");
    assert_eq!(at, (orr_sim::FP::ZERO, orr_sim::FP::ZERO));

    // Start: the first round begins with the reset command, and the screen words are gone.
    assert!(screens.act(Action::Confirm));
    let (_, texts) = frames(&mut bridge, 5);
    assert_eq!(texts.len(), 2, "only the score and the goal: {texts:?}");

    // Pause: the square holds still while the sim keeps ticking.
    frames(&mut bridge, 100);
    assert!(screens.act(Action::Pause));
    let (held, texts) = frames(&mut bridge, 1);
    assert!(texts.contains(&"일시정지".to_string()));
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
    assert!(texts.contains(&"승리!".to_string()), "{texts:?}");
    assert!(fx.borrow().score >= WIN_STARS, "the win came from a verified score of {}", fx.borrow().score);
    assert_eq!(frames(&mut bridge, 40).0, won_at, "the square holds still on the win screen");

    // Restart: a new round, the score is back to 0, and the game takes input again.
    assert!(screens.act(Action::Restart));
    frames(&mut bridge, 10);
    assert_eq!(screens.screen(), Screen::Playing);
    assert_eq!(fx.borrow().score, 0, "the reset command reached the game");
}

fn sound_views(fx: &Shared) -> (Views<StarPick>, Rc<RefCell<OfflineAudio>>) {
    let mut views = Views::<StarPick>::new();
    let audio = Rc::new(RefCell::new(OfflineAudio::new_offline(AudioConfig::default(), 44_100).unwrap()));
    views.set_sound_out(audio.clone());
    views.sound(sound::PICKUP, clip_from_wav(PICKUP_WAV).unwrap());
    views.on_event_sound(pickup_sound);
    views.scene_view(Juice { fx: fx.clone() });
    (views, audio)
}

fn pickup_update(bridge: &mut InProc<StarPick, PlayHost<StarPick>>, events: Vec<BridgeEvent<Picked>>) -> orr_bridge::ViewUpdate<Picked> {
    let mut update = bridge.poll_view();
    update.events = events;
    update
}

fn predicted(seq: u32) -> BridgeEvent<Picked> {
    BridgeEvent::Sim { key: EventKey::new(5, 0, seq), status: EventStatus::Predicted(Picked::zeroed()) }
}

#[test]
fn the_app_stops_the_sound_of_a_pickup_that_a_rollback_or_a_resync_took_back() {
    let fx = Shared::default();
    let (mut views, audio) = sound_views(&fx);
    let session = PlaySession::<StarPick>::new(PlayConfig::new(1, SEED, TICK_RATE), ());
    let mut bridge = InProc::new(PlayHost::new(session, PlayerSlot(0)), BridgeConfig::default());
    bridge.step(1);
    // The fade is 15 ms and the ding is 180 ms: after 2 x 735 frames (33 ms) only a stopped sound is quiet.
    let mut block = [0.0f32; 1470];

    // A predicted pick-up starts the ding. A cancel stops it, and the ring goes with it.
    let update = pickup_update(&mut bridge, vec![predicted(0)]);
    views.dispatch(&mut bridge, &update, &[]);
    assert_eq!((audio.borrow().voice_count(), fx.borrow().rings.len()), (1, 1));
    let update = pickup_update(&mut bridge, vec![BridgeEvent::Sim { key: EventKey::new(5, 0, 0), status: EventStatus::Canceled }]);
    views.dispatch(&mut bridge, &update, &[]);
    audio.borrow_mut().render(&mut block).unwrap();
    audio.borrow_mut().render(&mut block).unwrap();
    assert_eq!((audio.borrow().voice_count(), fx.borrow().rings.len()), (0, 0), "the sound was stopped before its natural end");

    // A resync forgets the predictions without a cancel: the sound and the ring still go.
    let update = pickup_update(&mut bridge, vec![predicted(1)]);
    views.dispatch(&mut bridge, &update, &[]);
    assert_eq!(audio.borrow().voice_count(), 1);
    let mut update = pickup_update(&mut bridge, Vec::new());
    update.resync = Some(ViewResync { generation: 1, discarded_events: 1, head_tick: 1, verified_tick: 1, disconnected: false, last_desync: None, lifecycle: Vec::new() });
    views.dispatch(&mut bridge, &update, &[]);
    audio.borrow_mut().render(&mut block).unwrap();
    audio.borrow_mut().render(&mut block).unwrap();
    assert_eq!((audio.borrow().voice_count(), fx.borrow().rings.len()), (0, 0));
    assert!(block.iter().all(|v| v.abs() < 1e-4), "nothing is heard after the fade");
}

#[test]
fn a_saved_game_comes_back_with_its_score_and_goes_on_like_the_game_that_was_never_saved() {
    let dir = std::env::temp_dir().join(format!("star_pick_save_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let fx = Shared::default();
    let (mut views, _audio) = sound_views(&fx);
    views.enable_saves(SaveStore::at(&dir, GAME_ID, SAVE_VERSION).unwrap(), 1, TICK_RATE);
    let new_bridge = |session: PlaySession<StarPick>| InProc::new(PlayHost::new(session, PlayerSlot(0)), BridgeConfig::default());
    let run = |bridge: &mut InProc<StarPick, PlayHost<StarPick>>, views: &mut Views<StarPick>, ticks: u32| {
        for _ in 0..ticks {
            let input = steer_view(bridge.snapshot().expect("a snapshot").predicted());
            bridge.set_input(PlayerSlot(0), input).unwrap();
            bridge.step(1); // exactly one tick, so the two runs can be compared tick for tick
            let update = bridge.poll_view();
            views.dispatch(bridge, &update, &[]);
        }
    };
    let mut bridge = new_bridge(PlaySession::<StarPick>::new(PlayConfig::new(1, SEED, TICK_RATE), ()));
    bridge.step(1);
    run(&mut bridge, &mut views, 300);
    let (saved_score, saved_checksum) = (fx.borrow().score, bridge.snapshot().unwrap().verified().unwrap().checksum());
    assert!(saved_score >= 1, "something to save");
    views.save_game(QUICK_SLOT, "").expect("F5");
    run(&mut bridge, &mut views, 400);
    assert_ne!(bridge.snapshot().unwrap().verified().unwrap().checksum(), saved_checksum);

    // F9: the app starts a new session from the save. The views start again from its first snapshot.
    views.load_game(QUICK_SLOT).expect("F9");
    let loaded = views.take_load().expect("an accepted load");
    let mut bridge = new_bridge(loaded.into_session::<StarPick>(SEED).unwrap());
    views.session_replaced();
    let first = bridge.poll_view().snapshot.expect("the first snapshot of the new session");
    assert_eq!(first.verified().unwrap().checksum(), saved_checksum, "the checksum after the load is the saved one");
    run(&mut bridge, &mut views, 1);
    assert_eq!(fx.borrow().score, saved_score, "the score on the screen is the saved score");
    assert!(fx.borrow().rings.is_empty(), "the rings of the old session are gone");

    // The chasing player is a function of the frame, so the loaded game walks the same road as a game that never stopped.
    run(&mut bridge, &mut views, 399);
    let mut straight = new_bridge(PlaySession::<StarPick>::new(PlayConfig::new(1, SEED, TICK_RATE), ()));
    straight.step(1);
    let mut scratch = Views::<StarPick>::new();
    run(&mut straight, &mut scratch, 300 + 400);
    assert_eq!(bridge.snapshot().unwrap().tick(), straight.snapshot().unwrap().tick());
    assert_eq!(bridge.snapshot().unwrap().verified().unwrap().checksum(), straight.snapshot().unwrap().verified().unwrap().checksum());
    let _ = std::fs::remove_dir_all(dir);
}
