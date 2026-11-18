//! Player signals in sessions: recorded with the input, replayed the same,
//! announced offline at the start, and resimulated by a rollback when one
//! arrives for a tick that was already predicted.
use std::collections::BTreeMap;

use orr_session::{
    replay_verify, AdvanceResult, EventKey, EventStatus, LoopbackNetwork, PlayConfig, PlaySession, ReplayReader, Session,
    SessionConfig, SignalError,
};
use orr_sim::{signal_key, PlayerSlot, RawPlayerSignal, Simulation, TickInputs};
use orr_testgame::roster::{Avatar, Census, Profile, Roster, RosterInput};

// ---- offline: PlaySession and recordings ----

fn play(players: u8) -> PlaySession<Roster> {
    PlaySession::<Roster>::new(PlayConfig::new(players, 5, 60), ())
}

fn input(slot: u8, tick: u64) -> RosterInput {
    RosterInput { dx: ((tick * 7 + u64::from(slot) * 3) % 3) as i32 - 1 }
}

/// Runs `ticks` ticks with scripted input for both slots, pushing the
/// signals of `plan` right before the tick they belong to.
fn run(session: &mut PlaySession<Roster>, ticks: u64, plan: &[(u64, RawPlayerSignal)]) {
    for tick in 1..=ticks {
        for slot in 0..session.player_count() {
            session.set_input(PlayerSlot(slot), input(slot, tick));
        }
        for (at, signal) in plan {
            if *at == tick {
                session.push_signal(signal.clone()).unwrap();
            }
        }
        session.step_now().expect("a live session always runs a tick");
    }
}

fn avatar_away(frame: &orr_ecs::Frame, slot: u32) -> Option<u32> {
    frame.dense::<Avatar>().1.iter().find(|a| a.slot == slot).map(|a| a.away)
}

fn plan() -> Vec<(u64, RawPlayerSignal)> {
    vec![
        (1, RawPlayerSignal::added_with(PlayerSlot(0), &Profile { color: 3, _pad: 0 })),
        (1, RawPlayerSignal::added_with(PlayerSlot(1), &Profile { color: 4, _pad: 0 })),
        (6, RawPlayerSignal::disconnected(PlayerSlot(1))),
        (9, RawPlayerSignal::reconnected(PlayerSlot(1))),
        (12, RawPlayerSignal::removed(PlayerSlot(1))),
    ]
}

#[test]
fn a_replay_applies_the_recorded_signals_at_the_same_ticks_as_the_live_run() {
    let mut live = play(2);
    run(&mut live, 14, &plan());
    let live_checksums: Vec<u64> = (0..=14).map(|t| live.checksum_at(t).unwrap()).collect();
    let bytes = live.save_replay();

    // Full verification: every recorded checksum is reproduced from the file alone.
    let report = replay_verify::<Roster>(&bytes, ()).unwrap();
    assert!(report.ok(), "replay diverged: {:?}", report.mismatch);
    assert!(report.checksums_checked >= 14);

    // The signal happens in the tick it was recorded for, in the replay as in the live run.
    let reader = ReplayReader::<Roster>::parse(&bytes).unwrap();
    for (at, signal) in plan() {
        assert!(reader.signals(at).contains(&signal), "tick {at} lost {signal:?}");
    }
    let state_at = |tick: u64| reader.seek((), tick).unwrap();
    assert_eq!(avatar_away(state_at(5).frame(), 1), Some(0));
    assert_eq!(avatar_away(state_at(6).frame(), 1), Some(1), "Disconnected applies in tick 6");
    assert_eq!(avatar_away(state_at(8).frame(), 1), Some(1));
    assert_eq!(avatar_away(state_at(9).frame(), 1), Some(0), "Reconnected applies in tick 9");
    assert_eq!(avatar_away(state_at(11).frame(), 1), Some(0));
    assert_eq!(avatar_away(state_at(12).frame(), 1), None, "Removed applies in tick 12");
    for tick in 1..=14 {
        assert_eq!(state_at(tick).checksum(), live_checksums[tick as usize], "tick {tick}");
    }

    // A viewer session plays the same ticks and tells the view about the signals once.
    let mut viewer = PlaySession::<Roster>::open_replay(&bytes, (), 0).unwrap();
    let mut keys = Vec::new();
    for tick in 1..=14u64 {
        viewer.step_now().unwrap();
        assert_eq!(viewer.checksum_at(tick), Some(live_checksums[tick as usize]));
        for (key, status) in viewer.take_signal_notes().into_vec() {
            assert!(matches!(status, EventStatus::Verified(_)));
            keys.push(key);
        }
    }
    let expected: Vec<EventKey> = plan().iter().map(|(at, s)| signal_key(*at, s.slot, 0)).collect();
    assert_eq!(keys, expected);
}

#[test]
fn seeking_back_and_forth_in_a_recording_keeps_the_signal_effects() {
    let mut live = play(2);
    run(&mut live, 14, &plan());
    let at_end = live.checksum_at(14).unwrap();
    live.seek(7).unwrap();
    assert_eq!(avatar_away(live.frame(), 1), Some(1));
    live.seek(14).unwrap();
    assert_eq!(live.simulation().checksum(), at_end);
    live.seek(2).unwrap();
    assert_eq!(avatar_away(live.frame(), 1), Some(0));
    assert_eq!(live.frame().singleton::<Census>().added, 2);
}

#[test]
fn a_branch_drops_the_recorded_signals_after_it() {
    let mut live = play(2);
    run(&mut live, 14, &plan());
    live.seek(8).unwrap();
    live.control(orr_session::ControlOp::Branch);
    live.step_now().unwrap();
    let reader = ReplayReader::<Roster>::parse(&live.save_replay()).unwrap();
    assert!(reader.signals(9).is_empty(), "Reconnected was after the branch point");
    assert!(reader.signals(12).is_empty());
    assert_eq!(reader.signals(6).len(), 1);
    assert_eq!(avatar_away(live.frame(), 1), Some(1), "still disconnected: the Reconnected was cut");
}

#[test]
fn offline_play_announces_every_player_in_the_first_tick() {
    let mut session = play(3);
    session.announce_players().unwrap();
    session.step_now().unwrap();

    let census = *session.frame().singleton::<Census>();
    assert_eq!(census.added, 3);
    assert_eq!(session.frame().dense::<Avatar>().1.len(), 3);

    let notes = session.take_signal_notes().into_vec();
    assert_eq!(notes.len(), 3);
    for (slot, (key, status)) in notes.iter().enumerate() {
        assert_eq!(*key, signal_key(1, PlayerSlot(slot as u8), 0));
        assert_eq!(*status, EventStatus::Verified(RawPlayerSignal::added(PlayerSlot(slot as u8))));
    }
    assert!(session.take_signal_notes().is_empty(), "notes are taken once");

    session.step_now().unwrap();
    assert!(session.take_signal_notes().is_empty(), "only tick 1 has signals");

    // The announcement is part of the recording: a replay builds the same world.
    let bytes = session.save_replay();
    assert!(replay_verify::<Roster>(&bytes, ()).unwrap().ok());
    let reader = ReplayReader::<Roster>::parse(&bytes).unwrap();
    assert_eq!(reader.signals(1).len(), 3);
}

#[test]
fn a_recording_with_or_without_signals_has_the_same_format_version() {
    let mut quiet = play(2);
    run(&mut quiet, 3, &[]);
    let bytes = quiet.save_replay();
    assert_eq!(&bytes[4..8], &5u32.to_le_bytes(), "no signal: format version 5");
    assert!(replay_verify::<Roster>(&bytes, ()).unwrap().ok());
    assert!(ReplayReader::<Roster>::parse(&bytes).unwrap().signals(1).is_empty());

    let mut loud = play(2);
    run(&mut loud, 3, &plan()[..1]);
    assert_eq!(&loud.save_replay()[4..8], &5u32.to_le_bytes(), "signals: format version 5");
}

#[test]
fn a_viewer_cannot_add_signals() {
    let mut live = play(2);
    run(&mut live, 2, &[]);
    let mut viewer = PlaySession::<Roster>::open_replay(&live.save_replay(), (), 0).unwrap();
    assert!(viewer.push_signal(RawPlayerSignal::added(PlayerSlot(0))).is_err());
    assert!(viewer.announce_players().is_err());
}

// ---- prediction: Session with a loopback peer ----

/// One-way delay in calls. It makes the verified tick lag the head by about
/// four ticks, so a signal can still arrive for a tick that was predicted.
const LATENCY: u64 = 6;

struct Pair {
    a: Session<Roster, orr_session::LoopbackEnd<Roster>>,
    b: Session<Roster, orr_session::LoopbackEnd<Roster>>,
    clock: orr_session::LoopbackClock,
    notes: Vec<(EventKey, EventStatus<RawPlayerSignal>)>,
    events: Vec<(EventKey, EventStatus<orr_testgame::roster::Moved>)>,
    rollbacks: Vec<orr_session::RollbackInfo>,
    call: u64,
}

impl Pair {
    fn new() -> Self {
        let (end_a, end_b, clock) = LoopbackNetwork::new::<Roster>(LATENCY, 0, 11);
        let a = Session::<Roster, _>::new((), SessionConfig::new(2, PlayerSlot(0), 42, 60), end_a);
        let b = Session::<Roster, _>::new((), SessionConfig::new(2, PlayerSlot(1), 42, 60), end_b);
        Self { a, b, clock, notes: Vec::new(), events: Vec::new(), rollbacks: Vec::new(), call: 0 }
    }

    /// Schedules on both peers: every peer must see every signal.
    fn signal(&mut self, tick: u64, signal: RawPlayerSignal) -> Result<(), SignalError> {
        self.a.schedule_signal(tick, signal.clone())?;
        self.b.schedule_signal(tick, signal)
    }

    fn advance(&mut self) {
        self.call += 1;
        let target = self.call + 2;
        self.clock.tick();
        let result = self.a.advance(input(0, target), Vec::new());
        let _ = self.b.advance(input(1, target), Vec::new());
        match result {
            AdvanceResult::Advanced { events, rollback, .. } => {
                self.events.extend(events.into_vec());
                self.rollbacks.extend(rollback);
            }
            AdvanceResult::Stalled { events } => self.events.extend(events.into_vec()),
        }
        self.notes.extend(self.a.take_signal_notes().into_vec());
    }

    /// Delivers what is still in flight, until every tick of A is verified.
    fn drain(&mut self) {
        for _ in 0..(LATENCY + 4) {
            self.clock.tick();
            let (events, rollback) = self.a.poll_confirmed();
            self.events.extend(events.into_vec());
            self.rollbacks.extend(rollback);
            let _ = self.b.poll_confirmed();
            self.notes.extend(self.a.take_signal_notes().into_vec());
        }
        assert_eq!(self.a.verified_tick(), self.a.head_tick(), "every tick is verified");
    }
}

fn signals() -> Vec<(u64, RawPlayerSignal)> {
    vec![
        (4, RawPlayerSignal::added_with(PlayerSlot(0), &Profile { color: 5, _pad: 0 })),
        (5, RawPlayerSignal::added_with(PlayerSlot(1), &Profile { color: 6, _pad: 0 })),
        (20, RawPlayerSignal::disconnected(PlayerSlot(1))),
        (30, RawPlayerSignal::reconnected(PlayerSlot(1))),
    ]
}

const CALLS: u64 = 60;

/// Runs the scenario. `late`: each signal is scheduled when the head has
/// passed its tick (so the tick was predicted without it); otherwise before
/// the run starts.
fn scenario(late: bool) -> Pair {
    let mut pair = Pair::new();
    if !late {
        for (tick, signal) in signals() {
            pair.signal(tick, signal).unwrap();
        }
    }
    let mut done = Vec::new();
    for _ in 0..CALLS {
        let mut due = Vec::new();
        if late {
            for (tick, signal) in signals() {
                if !done.contains(&tick) && pair.a.head_tick() > tick {
                    pair.signal(tick, signal).expect("the tick is not verified yet");
                    done.push(tick);
                    due.push(tick);
                }
            }
        }
        let before = pair.rollbacks.len();
        pair.advance();
        for tick in due {
            let covered = pair.rollbacks[before..].iter().any(|r| r.from_tick <= tick && tick <= r.to_tick);
            assert!(covered, "a signal scheduled for predicted tick {tick} must roll back over it");
        }
    }
    pair.drain();
    pair
}

#[test]
fn a_signal_for_a_tick_already_predicted_rolls_back_and_reaches_the_same_state() {
    let in_time = scenario(false);
    let late = scenario(true);

    assert_eq!(in_time.a.checksums(), late.a.checksums(), "same verified checksums, tick by tick");
    assert!(!late.a.checksums().is_empty());
    assert_eq!(in_time.a.checksums(), in_time.b.checksums(), "the peers agree");
    assert_eq!(late.a.checksums(), late.b.checksums());
    let last = |p: &Pair| p.a.verified_frame().unwrap().checksum();
    assert_eq!(last(&in_time), last(&late));
    assert!(late.a.rollback_count() > in_time.a.rollback_count(), "the late signals cost extra rollbacks");

    let verified_moves = |p: &Pair| {
        p.events
            .iter()
            .filter_map(|(key, status)| match status {
                EventStatus::Verified(moved) => Some((*key, *moved)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(verified_moves(&in_time), verified_moves(&late), "the game events that end up verified are the same");
    assert!(!verified_moves(&late).is_empty());

    let census = *late.a.verified_frame().unwrap().singleton::<Census>();
    assert_eq!(census, Census { added: 2, disconnected: 1, reconnected: 1, removed: 0 }, "each signal applied once");
}

#[test]
fn signal_notes_follow_the_prediction_and_close_once() {
    for late in [false, true] {
        let pair = scenario(late);
        let mut live: BTreeMap<EventKey, RawPlayerSignal> = BTreeMap::new();
        let mut verified = Vec::new();
        for (key, status) in &pair.notes {
            match status {
                EventStatus::Predicted(signal) => {
                    assert!(live.insert(*key, signal.clone()).is_none(), "{key:?} predicted twice while live");
                }
                EventStatus::Verified(signal) => {
                    let predicted = live.remove(key).expect("verified without a live prediction");
                    assert_eq!(&predicted, signal);
                    verified.push((*key, signal.clone()));
                }
                EventStatus::Canceled => {
                    assert!(live.remove(key).is_some(), "canceled without a live prediction");
                }
            }
        }
        assert!(live.is_empty(), "late={late}: left open: {live:?}");
        let expected: Vec<_> = signals().into_iter().map(|(tick, s)| (signal_key(tick, s.slot, 0), s)).collect();
        assert_eq!(verified, expected, "late={late}: each signal verified once, in tick order");
    }
}

#[test]
fn a_late_signal_of_another_slot_does_not_cancel_the_one_that_is_there() {
    let mut pair = Pair::new();
    let slot1 = RawPlayerSignal::added(PlayerSlot(1));
    pair.signal(6, slot1.clone()).unwrap();
    for _ in 0..8 {
        pair.advance();
    }
    // Slot 0 sorts before slot 1 in tick 6, but the key of slot 1 does not move.
    pair.signal(6, RawPlayerSignal::added(PlayerSlot(0))).unwrap();
    pair.advance();
    pair.drain();
    let canceled = pair.notes.iter().filter(|(_, s)| matches!(s, EventStatus::Canceled)).count();
    assert_eq!(canceled, 0, "no signal was canceled");
    let predicted = pair.notes.iter().filter(|(k, s)| *k == signal_key(6, PlayerSlot(1), 0) && matches!(s, EventStatus::Predicted(_))).count();
    assert_eq!(predicted, 1, "the slot 1 signal was predicted once");
}

#[test]
fn a_signal_for_a_verified_tick_is_refused() {
    let mut pair = Pair::new();
    for _ in 0..12 {
        pair.advance();
    }
    pair.drain();
    let verified = pair.a.verified_tick();
    assert!(verified > 4);
    let err = pair.a.schedule_signal(3, RawPlayerSignal::added(PlayerSlot(0))).unwrap_err();
    assert_eq!(err, SignalError::AlreadyVerified { tick: 3, verified_tick: verified });
    assert!(pair.a.schedule_signal(verified + 1, RawPlayerSignal::added(PlayerSlot(0))).is_ok());
}

// ---- authoritative restore ----

/// A frame of the roster game at `tick`, as the authority would send it.
fn authority_frame(tick: u64) -> (u64, Vec<u8>) {
    let mut reference = Simulation::<Roster>::new((), 60, 42);
    for t in 1..=tick {
        reference.step(&TickInputs::new(t, 2));
    }
    (reference.checksum(), reference.frame().to_bytes())
}

fn notes_of(pair: &Pair, key: EventKey) -> Vec<&'static str> {
    pair.notes
        .iter()
        .filter(|(k, _)| *k == key)
        .map(|(_, s)| match s {
            EventStatus::Predicted(_) => "predicted",
            EventStatus::Verified(_) => "verified",
            EventStatus::Canceled => "canceled",
        })
        .collect()
}

#[test]
fn restoring_before_the_signal_tick_cancels_the_prediction_before_it_is_predicted_again() {
    let mut pair = Pair::new();
    let key = signal_key(6, PlayerSlot(1), 0);
    pair.signal(6, RawPlayerSignal::added(PlayerSlot(1))).unwrap();
    for _ in 0..6 {
        pair.advance();
    }
    assert_eq!(pair.a.head_tick(), 6);
    assert!(pair.a.verified_tick() < 5, "the remote input is still unconfirmed");
    assert_eq!(notes_of(&pair, key), ["predicted"]);

    let (checksum, bytes) = authority_frame(5);
    pair.a.restore_confirmed(5, checksum, &bytes, 1).unwrap();
    pair.notes.extend(pair.a.take_signal_notes().into_vec());
    assert_eq!(notes_of(&pair, key), ["predicted", "canceled"], "the restore withdraws the prediction");

    pair.advance();
    assert_eq!(pair.a.head_tick(), 6, "tick 6 runs again");
    assert_eq!(notes_of(&pair, key), ["predicted", "canceled", "predicted"], "never two predictions in a row");
}

#[test]
fn restoring_at_or_after_the_signal_tick_leaves_no_unconfirmed_signal() {
    let mut pair = Pair::new();
    let key = signal_key(6, PlayerSlot(1), 0);
    pair.signal(6, RawPlayerSignal::added(PlayerSlot(1))).unwrap();
    for _ in 0..8 {
        pair.advance();
    }
    assert_eq!(notes_of(&pair, key), ["predicted"]);

    let (checksum, bytes) = authority_frame(7);
    pair.a.restore_confirmed(7, checksum, &bytes, 1).unwrap();
    pair.notes.extend(pair.a.take_signal_notes().into_vec());
    assert_eq!(notes_of(&pair, key), ["predicted", "canceled"]);

    // The signal's tick is part of the restored frame: it can no longer be scheduled and never runs again.
    assert_eq!(
        pair.a.schedule_signal(6, RawPlayerSignal::added(PlayerSlot(1))).unwrap_err(),
        SignalError::AlreadyVerified { tick: 6, verified_tick: 7 }
    );
    for _ in 0..4 {
        pair.advance();
    }
    assert_eq!(notes_of(&pair, key), ["predicted", "canceled"], "nothing stale is left to note");
}
