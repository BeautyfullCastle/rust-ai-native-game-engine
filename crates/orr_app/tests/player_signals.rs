//! `SceneView::on_player` against real bridges: an offline play session that
//! announces its players, and a prediction session that gets a signal for a
//! tick it already predicted (real rollback). No window, no renderer.
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use orr_app::{EventState, SceneView, ViewContext, Views};
use orr_bridge::{
    Bridge, BridgeConfig, BridgeEvent, EventKey, EventStatus, InProc, PlayConfig, PlayHost, PlaySession, PlayerSlot, RollbackInfo,
    SimHost,
};
use orr_ecs::Frame;
use orr_session::{AdvanceResult, EventBatch, LoopbackClock, LoopbackEnd, LoopbackNetwork, Session, SessionConfig};
use orr_sim::{signal_key, PlayerSignal, RawPlayerSignal};
use orr_testgame::roster::{Avatar, Profile, Roster, RosterInput};

const ONE_TICK: Duration = Duration::from_nanos(16_666_667);

/// What the scene view was told: the tick it saw (`ctx.tick()`), the signal, the state.
type Heard = (u64, PlayerSignal<Profile>, EventState);

struct Listener {
    heard: Shared<Heard>,
    /// The number of avatars in the predicted frame at each call.
    avatars: Shared<usize>,
}

impl SceneView<Roster> for Listener {
    fn on_player(&mut self, ctx: &mut ViewContext<'_, Roster>, signal: &RawPlayerSignal, state: EventState) {
        self.heard.borrow_mut().push((ctx.tick(), signal.typed::<Profile>(), state));
        self.avatars.borrow_mut().push(ctx.predicted().dense::<Avatar>().1.len());
    }
}

type Shared<T> = Rc<RefCell<Vec<T>>>;

fn listener() -> (Listener, Shared<Heard>, Shared<usize>) {
    let heard = Rc::new(RefCell::new(Vec::new()));
    let avatars = Rc::new(RefCell::new(Vec::new()));
    (Listener { heard: heard.clone(), avatars: avatars.clone() }, heard, avatars)
}

fn frame<B: Bridge<Roster>>(bridge: &mut B, views: &mut Views<Roster>) {
    bridge.update(ONE_TICK);
    let update = bridge.poll_view();
    views.dispatch(bridge, &update, &[]);
}

#[test]
fn an_offline_run_tells_the_scene_view_about_each_player_as_verified() {
    let (scene, heard, avatars) = listener();
    let mut views = Views::<Roster>::new();
    views.scene_view(scene);
    let session = PlaySession::<Roster>::new(PlayConfig::new(2, 1, 60), ());
    let mut bridge = InProc::new(PlayHost::new(session, PlayerSlot(0)).with_announced_players(), BridgeConfig::default());
    for _ in 0..3 {
        frame(&mut bridge, &mut views);
    }
    let heard = heard.borrow();
    assert_eq!(heard.len(), 2, "one call per player, once");
    for (slot, (_, signal, state)) in heard.iter().enumerate() {
        assert_eq!(*signal, PlayerSignal::Added { slot: PlayerSlot(slot as u8), data: Profile::default() });
        assert_eq!(*state, EventState::Verified);
    }
    assert_eq!(*avatars.borrow(), [2, 2], "the avatars are already in the frame the view reads");
}

#[test]
fn an_offline_run_gives_the_local_slots_their_start_data_and_the_others_the_default() {
    let (scene, heard, _) = listener();
    let mut views = Views::<Roster>::new();
    views.scene_view(scene);
    let session = PlaySession::<Roster>::new(PlayConfig::new(3, 1, 60), ());
    let host = PlayHost::new(session, PlayerSlot(0)).with_extra_local_slots([PlayerSlot(1)]).with_announced_players_data(|slot| {
        if slot.0 < 2 { bytemuck::bytes_of(&Profile { color: 5 + u32::from(slot.0), _pad: 0 }).to_vec() } else { Vec::new() }
    });
    let mut bridge = InProc::new(host, BridgeConfig::default());
    for _ in 0..3 {
        frame(&mut bridge, &mut views);
    }
    let data: Vec<PlayerSignal<Profile>> = heard.borrow().iter().map(|(_, signal, _)| *signal).collect();
    assert_eq!(
        data,
        [
            PlayerSignal::Added { slot: PlayerSlot(0), data: Profile { color: 5, _pad: 0 } },
            PlayerSignal::Added { slot: PlayerSlot(1), data: Profile { color: 6, _pad: 0 } },
            PlayerSignal::Added { slot: PlayerSlot(2), data: Profile::default() },
        ]
    );
}

// ---- prediction ----

/// The local peer of a two-peer loopback, plus a script: at call `n` the
/// signal is scheduled (on both peers) for `tick`.
struct Host {
    a: Session<Roster, LoopbackEnd<Roster>>,
    b: Session<Roster, LoopbackEnd<Roster>>,
    clock: LoopbackClock,
    calls: u64,
    script: Vec<(u64, u64, RawPlayerSignal)>,
}

impl Host {
    fn new(script: Vec<(u64, u64, RawPlayerSignal)>) -> Self {
        let (end_a, end_b, clock) = LoopbackNetwork::new::<Roster>(6, 0, 3);
        let a = Session::<Roster, _>::new((), SessionConfig::new(2, PlayerSlot(0), 9, 60), end_a);
        let b = Session::<Roster, _>::new((), SessionConfig::new(2, PlayerSlot(1), 9, 60), end_b);
        Self { a, b, clock, calls: 0, script }
    }
}

impl SimHost<Roster> for Host {
    fn tick_rate(&self) -> u32 {
        60
    }
    fn local_slot(&self) -> PlayerSlot {
        PlayerSlot(0)
    }
    fn player_count(&self) -> u8 {
        2
    }
    fn advance(&mut self, input: RosterInput, commands: Vec<()>) -> AdvanceResult<Roster> {
        self.calls += 1;
        for (call, tick, signal) in &self.script {
            if *call == self.calls {
                self.a.schedule_signal(*tick, signal.clone()).unwrap();
                self.b.schedule_signal(*tick, signal.clone()).unwrap();
            }
        }
        self.clock.tick();
        let result = self.a.advance(input, commands);
        let _ = self.b.advance(RosterInput::default(), Vec::new());
        result
    }
    fn take_signal_notes(&mut self) -> EventBatch<RawPlayerSignal> {
        self.a.take_signal_notes()
    }
    fn head_tick(&self) -> u64 {
        self.a.head_tick()
    }
    fn verified_tick(&self) -> u64 {
        self.a.verified_tick()
    }
    fn predicted_frame(&self) -> &Frame {
        self.a.predicted_frame()
    }
    fn verified_frame(&self) -> Option<&Frame> {
        self.a.verified_frame()
    }
    fn frame_at(&self, tick: u64) -> Option<&Frame> {
        self.a.frame_at(tick)
    }
    fn rollback_count(&self) -> u64 {
        self.a.rollback_count()
    }
    fn last_rollback(&self) -> Option<RollbackInfo> {
        self.a.last_rollback()
    }
}

#[test]
fn a_signal_for_a_predicted_tick_reaches_the_scene_view_as_predicted_then_verified_after_a_rollback() {
    let color = |c| Profile { color: c, _pad: 0 };
    let script = vec![
        // Known before tick 3 runs: predicted when the tick runs.
        (1, 3, RawPlayerSignal::added_with(PlayerSlot(0), &color(5))),
        // Arrives at call 12, when the head is past tick 8: a rollback puts it into tick 8.
        (12, 8, RawPlayerSignal::added_with(PlayerSlot(1), &color(6))),
    ];
    let (scene, heard, avatars) = listener();
    let mut views = Views::<Roster>::new();
    views.scene_view(scene);
    let mut bridge = InProc::new(Host::new(script), BridgeConfig::default());
    let mut rollback_calls = Vec::new();
    for call in 1..=40 {
        frame(&mut bridge, &mut views);
        if bridge.snapshot().is_some_and(|s| s.last_rollback().is_some_and(|r| r.from_tick <= 8 && 8 <= r.to_tick)) {
            rollback_calls.push(call);
        }
    }
    assert!(rollback_calls.iter().any(|&call| call >= 12), "the late signal made a rollback over tick 8: {rollback_calls:?}");

    let heard: Vec<_> = heard.borrow().iter().map(|(_, signal, state)| (*signal, *state)).collect();
    let first = PlayerSignal::Added { slot: PlayerSlot(0), data: color(5) };
    let second = PlayerSignal::Added { slot: PlayerSlot(1), data: color(6) };
    let order_of = |signal, state| heard.iter().position(|h| *h == (signal, state)).unwrap_or_else(|| panic!("{signal:?} {state:?} not heard in {heard:?}"));
    assert!(order_of(first, EventState::Predicted) < order_of(first, EventState::Verified));
    assert!(order_of(second, EventState::Predicted) < order_of(second, EventState::Verified));
    assert_eq!(heard.len(), 4, "each signal: once predicted, once verified: {heard:?}");
    assert!(avatars.borrow().iter().all(|&n| n >= 1), "the avatar of a signal is in the frame when the view hears it");
}

#[test]
fn a_cancel_names_the_signal_it_cancels_and_a_resync_forgets_the_prediction() {
    let (scene, heard, _) = listener();
    let mut views = Views::<Roster>::new();
    views.scene_view(scene);
    let session = PlaySession::<Roster>::new(PlayConfig::new(1, 1, 60), ());
    let mut bridge = InProc::new(PlayHost::new(session, PlayerSlot(0)), BridgeConfig::default());
    bridge.step(1);

    let (one, two) = (signal_key(5, PlayerSlot(0), 0), signal_key(6, PlayerSlot(0), 0));
    assert_eq!(one, EventKey::new(5, u16::MAX, 0));
    let removed = RawPlayerSignal::removed(PlayerSlot(0));
    let reconnected = RawPlayerSignal::reconnected(PlayerSlot(0));
    let player = |key, status| BridgeEvent::Player { key, status };
    let with = |bridge: &mut InProc<Roster, PlayHost<Roster>>, events| {
        let mut update = bridge.poll_view();
        update.events = events;
        update
    };

    let update = with(&mut bridge, vec![player(one, EventStatus::Predicted(removed.clone())), player(two, EventStatus::Predicted(reconnected))]);
    views.dispatch(&mut bridge, &update, &[]);
    let update = with(&mut bridge, vec![player(one, EventStatus::Canceled)]);
    views.dispatch(&mut bridge, &update, &[]);
    let states: Vec<_> = heard.borrow().iter().map(|(_, s, st)| (s.slot(), *st)).collect();
    assert_eq!(
        states,
        [(PlayerSlot(0), EventState::Predicted), (PlayerSlot(0), EventState::Predicted), (PlayerSlot(0), EventState::Canceled)]
    );
    assert!(matches!(heard.borrow()[2].1, PlayerSignal::Removed { .. }), "the cancel carries the payload of the prediction");

    // After a resync the prediction was forgotten, so its cancel has nothing to cancel.
    let mut update = with(&mut bridge, vec![player(two, EventStatus::Canceled)]);
    update.resync = Some(orr_bridge::ViewResync {
        generation: 1,
        discarded_events: 2,
        head_tick: 1,
        verified_tick: 1,
        disconnected: false,
        last_desync: None,
        lifecycle: Vec::new(),
    });
    views.dispatch(&mut bridge, &update, &[]);
    assert_eq!(heard.borrow().len(), 3);
}
