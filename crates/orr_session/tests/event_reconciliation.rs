//! Ordered input delivery exercises real rollback reconciliation without wall-clock timing.
use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_session::{AdvanceResult, EventStatus, InputSource, RemoteInput, Session, SessionConfig};
use orr_sim::{EventKey, Game, PlayerSlot, SimContext, System};
use orr_testgame::SpawnBulletCmd;

struct EventGame;

impl Game for EventGame {
    type Input = u32;
    type Command = SpawnBulletCmd;
    type Event = u32;
    type Config = ();

    fn register(_: &mut ComponentRegistryBuilder) {}
    fn setup(_: &mut Frame, _: &()) {}
    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(emit as fn(&mut SimContext<Self>))]
    }
}

fn emit(ctx: &mut SimContext<EventGame>) {
    let input = *ctx.inputs.input(PlayerSlot(1));
    // One stable key. Input 1 removes the event; other values are its payload.
    if ctx.tick == 3 && input != 1 {
        ctx.emit(input);
    }
}

#[derive(Default)]
struct ScriptedSource(Vec<RemoteInput<EventGame>>);

impl InputSource<EventGame> for ScriptedSource {
    fn send_local(&mut self, _: u64, _: PlayerSlot, _: u32, _: Vec<SpawnBulletCmd>) {}
    fn poll_remote(&mut self) -> Vec<RemoteInput<EventGame>> {
        std::mem::take(&mut self.0)
    }
}

type Peer = Session<EventGame, ScriptedSource>;
const KEY: EventKey = EventKey {
    tick: 3,
    system_index: 0,
    _pad: 0,
    seq: 0,
};

fn predict() -> Peer {
    let mut cfg = SessionConfig::new(2, PlayerSlot(0), 42, 60);
    cfg.input_delay = 0;
    let mut peer = Peer::new((), cfg, ScriptedSource::default());
    for tick in 1..=3 {
        let AdvanceResult::Advanced {
            tick: head,
            events,
            rollback,
        } = peer.advance(0, Vec::new())
        else {
            unreachable!("three ticks must fit in the prediction window");
        };
        assert_eq!(head, tick);
        assert!(rollback.is_none());
        let expected = if tick == 3 {
            vec![(KEY, EventStatus::Predicted(0))]
        } else {
            Vec::new()
        };
        assert_eq!(events.into_vec(), expected);
    }
    assert_eq!(peer.verified_tick(), 0);
    peer
}

fn confirm(
    peer: &mut Peer,
    tick: u64,
    input: u32,
    rollback_from: Option<u64>,
) -> Vec<(EventKey, EventStatus<u32>)> {
    peer.source_mut().0.push(RemoteInput {
        tick,
        slot: PlayerSlot(1),
        input,
        commands: Vec::new(),
        disconnected: false,
    });
    let (events, rollback) = peer.poll_confirmed();
    assert_eq!(
        rollback.map(|r| (r.from_tick, r.to_tick, r.resim_count)),
        rollback_from.map(|from| (from, 3, (4 - from) as u32))
    );
    assert_eq!(
        peer.head_tick(),
        3,
        "polling must not add speculative ticks"
    );
    assert_eq!(peer.verified_tick(), tick);
    events.into_vec()
}

fn assert_settled(peer: &mut Peer) {
    assert_eq!(peer.verified_tick(), peer.head_tick());
    for _ in 0..3 {
        let (events, rollback) = peer.poll_confirmed();
        assert!(
            events.is_empty(),
            "a verified occurrence must never be announced again"
        );
        assert!(rollback.is_none());
    }
}

#[test]
fn canceled_event_can_disappear_then_reappear_at_the_same_key() {
    let mut peer = predict();
    // Confirmed tick 1 changes the repeated remote input: tick 3 disappears.
    assert_eq!(
        confirm(&mut peer, 1, 1, Some(1)),
        vec![(KEY, EventStatus::Canceled)]
    );
    // Tick 2 restores the original payload, but this is a new live occurrence.
    assert_eq!(
        confirm(&mut peer, 2, 0, Some(2)),
        vec![(KEY, EventStatus::Predicted(0))]
    );
    assert_eq!(
        confirm(&mut peer, 3, 0, None),
        vec![(KEY, EventStatus::Verified(0))]
    );
    assert_settled(&mut peer);
}

#[test]
fn changed_payload_cancels_each_old_occurrence_before_predicting_the_replacement() {
    let mut peer = predict();
    assert_eq!(
        confirm(&mut peer, 1, 2, Some(1)),
        vec![
            (KEY, EventStatus::Canceled),
            (KEY, EventStatus::Predicted(2))
        ]
    );
    assert_eq!(
        confirm(&mut peer, 2, 3, Some(2)),
        vec![
            (KEY, EventStatus::Canceled),
            (KEY, EventStatus::Predicted(3))
        ]
    );
    assert_eq!(
        confirm(&mut peer, 3, 3, None),
        vec![(KEY, EventStatus::Verified(3))]
    );
    assert_settled(&mut peer);
}
