//! Explicit P2P diagnostic retention must not change simulation or recovery evidence.
use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_session::{
    AdvanceResult, ChecksumRetireError, EventBatch, EventStatus, InputSource,
    LocallyVerifiedHistory, LocallyVerifiedTick, RemoteInput, RollbackInfo, Session, SessionConfig,
    VerifiedHistoryLimits, VerifiedHistorySource,
};
use orr_sim::{EventKey, Game, PlayerSlot, SimCommand, SimContext, System};
use orr_testgame::SpawnBulletCmd;

struct DiagnosticGame;
impl Game for DiagnosticGame {
    type Input = u32;
    type Command = SpawnBulletCmd;
    type Event = u32;
    type Config = ();

    fn register(registry: &mut ComponentRegistryBuilder) {
        registry.register_singleton::<u64>("Sum");
    }
    fn setup(frame: &mut Frame, _: &()) {
        frame.set_singleton(0u64);
    }
    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(simulate as fn(&mut SimContext<Self>))]
    }
}
fn simulate(ctx: &mut SimContext<DiagnosticGame>) {
    let sum = *ctx.inputs.input(PlayerSlot(0)) + *ctx.inputs.input(PlayerSlot(1));
    *ctx.frame.singleton_mut::<u64>() += u64::from(sum);
    ctx.emit(sum);
}

type Packet = (u64, PlayerSlot, u32, Vec<Vec<u8>>, bool);
type Observation = (u64, Vec<(PlayerSlot, Vec<u8>)>);
type Outcome = (
    Option<u64>,
    Vec<(EventKey, EventStatus<u32>)>,
    Option<RollbackInfo>,
);
fn packet_view(packet: &RemoteInput<DiagnosticGame>) -> Packet {
    (
        packet.tick,
        packet.slot,
        packet.input,
        encoded(&packet.commands),
        packet.disconnected,
    )
}
fn encoded(commands: &[SpawnBulletCmd]) -> Vec<Vec<u8>> {
    commands
        .iter()
        .map(|command| {
            let mut bytes = Vec::new();
            command.encode(&mut bytes);
            bytes
        })
        .collect()
}
#[derive(Default)]
struct Queue {
    incoming: Vec<RemoteInput<DiagnosticGame>>,
    sent: Vec<Packet>,
    verified: Vec<Observation>,
}
impl InputSource<DiagnosticGame> for Queue {
    fn send_local(
        &mut self,
        tick: u64,
        slot: PlayerSlot,
        input: u32,
        commands: Vec<SpawnBulletCmd>,
    ) {
        self.sent
            .push((tick, slot, input, encoded(&commands), false));
    }
    fn poll_remote(&mut self) -> Vec<RemoteInput<DiagnosticGame>> {
        std::mem::take(&mut self.incoming)
    }
    fn on_locally_verified(&mut self, tick: LocallyVerifiedTick<'_, DiagnosticGame>) {
        self.verified
            .push((tick.simulated.tick(), tick.simulated_commands.to_vec()));
    }
}
type Peer = Session<DiagnosticGame, VerifiedHistorySource<DiagnosticGame, Queue>>;
fn peer(interval: u32, relay: bool) -> Peer {
    let mut cfg = SessionConfig::new(2, PlayerSlot(0), 42, 60);
    cfg.input_delay = 0;
    cfg.input_log_ticks = 5;
    cfg.checksum_interval = interval;
    cfg.keep_anchors = 4;
    cfg.relay = relay;
    let history = LocallyVerifiedHistory::new(
        7,
        2,
        VerifiedHistoryLimits {
            max_records: 128,
            max_encoded_bytes: 100_000,
        },
    )
    .unwrap();
    Session::new(
        (),
        cfg,
        VerifiedHistorySource::new(Queue::default(), history),
    )
}
fn packet(tick: u64, slot: u8) -> RemoteInput<DiagnosticGame> {
    RemoteInput {
        tick,
        slot: PlayerSlot(slot),
        input: tick as u32 + u32::from(slot),
        commands: vec![
            SpawnBulletCmd {
                owner: u32::from(slot)
            };
            2
        ],
        disconnected: false,
    }
}
fn feed(peer: &mut Peer, tick: u64) {
    if peer.config().relay {
        peer.source_mut().inner_mut().incoming.push(packet(tick, 0));
    }
    peer.source_mut().inner_mut().incoming.push(packet(tick, 1));
}
fn advance(peer: &mut Peer, tick: u64) -> AdvanceResult<DiagnosticGame> {
    peer.advance(tick as u32, packet(tick, 0).commands)
}
fn settle(peer: &mut Peer, through: u64) {
    while peer.head_tick() < through {
        let tick = peer.head_tick() + 1;
        feed(peer, tick);
        advance(peer, tick);
        peer.poll_confirmed();
        assert_eq!(peer.verified_tick(), tick);
    }
}

/// Public state other than the diagnostic vector, including every available
/// frame and the separate verified recovery archive and its callback observer.
#[derive(Debug, PartialEq, Eq)]
struct State {
    head: u64,
    verified: u64,
    next_send: u64,
    predicted: Vec<u8>,
    verified_frame: Option<Vec<u8>>,
    frames: Vec<(u64, Vec<u8>)>,
    anchors: Vec<(u64, u64, Vec<u8>)>,
    rollbacks: (u64, Option<RollbackInfo>),
    authored: Vec<Packet>,
    incoming: Vec<Packet>,
    sent: Vec<Packet>,
    callbacks: Vec<Observation>,
    archive: Vec<orr_session::LocallyVerifiedRecord<u32>>,
    archive_usage: (u64, usize, usize, u64),
    build: u64,
    config: String,
}
fn state(peer: &Peer) -> State {
    let history = peer.source().history();
    let archive = if peer.verified_tick() == 0 {
        Vec::new()
    } else {
        (0..2)
            .flat_map(|slot| {
                history
                    .export_range(PlayerSlot(slot), 1, peer.verified_tick())
                    .unwrap()
            })
            .collect()
    };
    State {
        head: peer.head_tick(),
        verified: peer.verified_tick(),
        next_send: peer.next_send_tick(),
        predicted: peer.predicted_frame().to_bytes(),
        verified_frame: peer.verified_frame().map(Frame::to_bytes),
        frames: (0..=peer.head_tick())
            .filter_map(|tick| peer.frame_at(tick).map(|f| (tick, f.to_bytes())))
            .collect(),
        anchors: peer
            .anchors()
            .iter()
            .map(|a| (a.tick, a.checksum, a.frame_bytes.clone()))
            .collect(),
        rollbacks: (peer.rollback_count(), peer.last_rollback()),
        authored: peer.authored_since(0).iter().map(packet_view).collect(),
        incoming: peer
            .source()
            .inner()
            .incoming
            .iter()
            .map(packet_view)
            .collect(),
        sent: peer.source().inner().sent.clone(),
        callbacks: peer.source().inner().verified.clone(),
        archive,
        archive_usage: (
            history.generation(),
            history.len(),
            history.retained_encoded_bytes(),
            history.retired_through(),
        ),
        build: peer.build_hash(),
        config: format!("{:?}", peer.config()),
    }
}

#[test]
fn default_keeps_full_history_and_retirement_is_inclusive_sparse_and_repeatable() {
    let mut peer = peer(3, false);
    assert_eq!(peer.retire_checksums_through(0), Ok(0));
    settle(&mut peer, 18);
    let original = peer.checksums().to_vec();
    assert_eq!(
        original.iter().map(|&(tick, _)| tick).collect::<Vec<_>>(),
        [3, 6, 9, 12, 15, 18]
    );
    let unchanged = state(&peer);
    assert_eq!(peer.retire_checksums_through(2), Ok(0));
    assert_eq!(peer.retire_checksums_through(6), Ok(2));
    assert_eq!(peer.checksums(), &original[2..]);
    assert_eq!(peer.retire_checksums_through(6), Ok(0));
    assert_eq!(peer.retire_checksums_through(4), Ok(0));
    assert_eq!(peer.retire_checksums_through(11), Ok(1));
    assert_eq!(peer.checksums(), &original[3..]);
    assert_eq!(state(&peer), unchanged);
    assert_eq!(peer.retire_checksums_through(18), Ok(3));
    assert!(peer.checksums().is_empty());
    settle(&mut peer, 24);
    assert_eq!(
        peer.checksums()
            .iter()
            .map(|&(tick, _)| tick)
            .collect::<Vec<_>>(),
        [21, 24]
    );
}

#[test]
fn rejection_is_transactional_and_relay_takes_precedence() {
    for relay in [false, true] {
        let mut peer = peer(1, relay);
        settle(&mut peer, 4);
        // Leave queued input in place: retirement may neither poll nor author.
        feed(&mut peer, 5);
        let unchanged = state(&peer);
        let checksums = peer.checksums().to_vec();
        for through in if relay {
            vec![0, 2, 4, 5, u64::MAX]
        } else {
            vec![5, u64::MAX]
        } {
            let expected = if relay {
                ChecksumRetireError::RelaySession
            } else {
                ChecksumRetireError::BeyondVerified {
                    through,
                    verified: 4,
                }
            };
            assert_eq!(peer.retire_checksums_through(through), Err(expected));
            assert_eq!(peer.checksums(), checksums);
            assert_eq!(state(&peer), unchanged);
        }
    }
}

fn outcome(result: AdvanceResult<DiagnosticGame>) -> Outcome {
    match result {
        AdvanceResult::Advanced {
            tick,
            events,
            rollback,
        } => (Some(tick), events.into_vec(), rollback),
        AdvanceResult::Stalled { events } => (None, events.into_vec(), None),
    }
}
fn count_canceled(events: &EventBatch<u32>) -> usize {
    events
        .iter()
        .filter(|(_, status)| matches!(status, EventStatus::Canceled))
        .count()
}

#[test]
fn retired_and_unretired_sessions_have_identical_rollback_events_and_recovery_history() {
    let mut full = peer(1, false);
    let mut retired = peer(1, false);
    let mut canceled = 0;
    let mut through = 0;
    for tick in 1..=30 {
        assert_eq!(
            outcome(advance(&mut full, tick)),
            outcome(advance(&mut retired, tick))
        );
        if tick % 3 == 0 {
            for confirmed in tick - 2..=tick {
                feed(&mut full, confirmed);
                feed(&mut retired, confirmed);
            }
            let (full_events, full_rollback) = full.poll_confirmed();
            let (retired_events, retired_rollback) = retired.poll_confirmed();
            canceled += count_canceled(&retired_events);
            assert_eq!(full_rollback, retired_rollback);
            assert_eq!(full_events.into_vec(), retired_events.into_vec());
            assert_eq!(full.verified_tick(), tick);
            // Compare before retiring the newly verified evidence.
            assert_eq!(
                retired.checksums(),
                full.checksums()
                    .iter()
                    .copied()
                    .filter(|&(t, _)| t > through)
                    .collect::<Vec<_>>()
            );
            through = tick.saturating_sub(2);
            retired.retire_checksums_through(through).unwrap();
        }
        assert_eq!(state(&full), state(&retired));
    }
    assert!(canceled > 0);
    assert!(retired.rollback_count() > 0);
    assert_eq!(retired.source().inner().verified.len(), 30);
    assert_eq!(full.checksums().len(), 30);
    assert_eq!(retired.checksums(), &full.checksums()[28..]);
}

#[test]
fn restore_and_reverification_keep_existing_append_order_without_a_retirement_floor() {
    let mut peer = peer(1, false);
    settle(&mut peer, 2);
    let frame = peer.verified_frame().unwrap().to_bytes();
    let checksum = peer.verified_frame().unwrap().checksum();
    settle(&mut peer, 6);
    let original = peer.checksums().to_vec();
    assert_eq!(peer.retire_checksums_through(4), Ok(4));
    // Recovery evidence has its separate, unchanged invalidation contract.
    peer.source_mut().history_mut().invalidate();
    peer.restore_confirmed(2, checksum, &frame, 6).unwrap();
    assert_eq!(peer.checksums(), &original[4..5]);
    // Restore keeps next_send_tick unchanged; replay both confirmed slots using
    // step(), rather than authoring new local inputs for already-authored ticks.
    for tick in 3..=6 {
        peer.source_mut().inner_mut().incoming.push(packet(tick, 0));
        feed(&mut peer, tick);
        peer.step();
        peer.poll_confirmed();
    }
    assert_eq!(
        peer.checksums(),
        [
            original[4],
            original[2],
            original[3],
            original[4],
            original[5]
        ]
    );
    // Existing restore semantics can leave append order non-monotonic. Retire
    // every matching tick, not just a sorted prefix, and preserve retained order.
    assert_eq!(peer.retire_checksums_through(4), Ok(2));
    assert_eq!(peer.checksums(), [original[4], original[4], original[5]]);
}
