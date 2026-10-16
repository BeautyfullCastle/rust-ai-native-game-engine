//! Exact recovery evidence is observed at verification, never at network ingress.
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use orr_fp::FP;
use orr_session::{
    InputSource, LocalInputSource, LocallyVerifiedHistory, LocallyVerifiedTick, RemoteInput, Session, SessionConfig,
    VerifiedHistoryEncodeError, VerifiedHistoryError, VerifiedHistoryLimits, VerifiedHistorySource,
};
use orr_sim::{PlayerSlot, SimCommand, Simulation, TickInputs};
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd};

type Archive = LocallyVerifiedHistory<Arena>;
type Inputs = TickInputs<ArenaInput, SpawnBulletCmd>;
const LIMITS: VerifiedHistoryLimits = VerifiedHistoryLimits {
    max_records: 300,
    max_encoded_bytes: 100_000,
};

#[derive(Default)]
struct Queue {
    incoming: Vec<RemoteInput<Arena>>,
    verified: Vec<u64>,
}
impl InputSource<Arena> for Queue {
    fn send_local(&mut self, _: u64, _: PlayerSlot, _: ArenaInput, _: Vec<SpawnBulletCmd>) {}
    fn poll_remote(&mut self) -> Vec<RemoteInput<Arena>> {
        std::mem::take(&mut self.incoming)
    }
    fn on_locally_verified(&mut self, tick: LocallyVerifiedTick<'_, Arena>) {
        self.verified.push(tick.simulated.tick());
    }
}

fn cfg(players: u8, slot: u8) -> SessionConfig {
    let mut cfg = SessionConfig::new(players, PlayerSlot(slot), 42, 60);
    cfg.input_delay = 0;
    cfg.input_log_ticks = 0;
    cfg.checksum_interval = 1;
    cfg
}
fn encode(command: &SpawnBulletCmd) -> Vec<u8> {
    let mut bytes = Vec::new();
    command.encode(&mut bytes);
    bytes
}
fn input(slot: u8, tick: u64) -> ArenaInput {
    ArenaInput::new(
        FP::from_int(i32::from(slot) - 1),
        FP::from_int((tick % 3) as i32 - 1),
        false,
    )
}
fn commands(slot: u8) -> Vec<SpawnBulletCmd> {
    vec![SpawnBulletCmd { owner: u32::from(slot) }; usize::from(slot) + 1]
}
fn packet(tick: u64, slot: u8, input: ArenaInput, commands: Vec<SpawnBulletCmd>) -> RemoteInput<Arena> {
    RemoteInput {
        tick,
        slot: PlayerSlot(slot),
        input,
        commands,
        disconnected: false,
    }
}

/// The three peers really author their own traffic; this test transport drops
/// C's tail only on the C -> B route. Closing C fences its future normal sends.
struct Mesh {
    owner: u8,
    queues: Rc<RefCell<[Vec<RemoteInput<Arena>>; 3]>>,
    c_fenced: Rc<RefCell<bool>>,
}
impl InputSource<Arena> for Mesh {
    fn send_local(&mut self, tick: u64, slot: PlayerSlot, input: ArenaInput, commands: Vec<SpawnBulletCmd>) {
        if self.owner == 2 && *self.c_fenced.borrow() {
            return;
        }
        for (destination, queue) in self.queues.borrow_mut().iter_mut().enumerate() {
            if destination == usize::from(self.owner) || (self.owner == 2 && destination == 1 && tick >= 3) {
                continue;
            }
            queue.push(packet(tick, slot.0, input, commands.clone()));
        }
    }
    fn poll_remote(&mut self) -> Vec<RemoteInput<Arena>> {
        std::mem::take(&mut self.queues.borrow_mut()[usize::from(self.owner)])
    }
}

#[test]
fn three_peer_pruned_tail_repairs_before_single_default_takeover() {
    let queues = Rc::new(RefCell::new(std::array::from_fn(|_| Vec::new())));
    let c_fenced = Rc::new(RefCell::new(false));
    let mut peers: Vec<_> = (0..3)
        .map(|slot| {
            let mesh = Mesh {
                owner: slot,
                queues: queues.clone(),
                c_fenced: c_fenced.clone(),
            };
            let source = VerifiedHistorySource::new(mesh, Archive::new(1, 3, LIMITS).unwrap());
            Session::new(ArenaConfig { player_count: 3 }, cfg(3, slot), source)
        })
        .collect();
    let mut reference = Simulation::<Arena>::new(ArenaConfig { player_count: 3 }, 60, 42);
    let mut expected = Vec::new();
    for tick in 1..=5 {
        let mut inputs = Inputs::new(tick, 3);
        for (slot, peer) in peers.iter_mut().enumerate() {
            let slot = slot as u8;
            let sample = input(slot, tick);
            inputs.set_input(PlayerSlot(slot), sample);
            for command in commands(slot) {
                inputs.push_command(PlayerSlot(slot), command);
            }
            peer.advance(sample, commands(slot));
        }
        for peer in &mut peers {
            peer.poll_confirmed();
        }
        reference.step(&inputs);
        expected.push((tick, reference.checksum()));
    }
    assert_eq!(peers[0].verified_tick(), 5);
    assert_eq!(peers[1].verified_tick(), 2);
    assert_eq!(peers[2].verified_tick(), 5);
    assert_eq!(peers[0].checksums(), expected);
    assert_eq!(peers[2].checksums(), expected);
    assert_eq!(peers[1].checksums(), &expected[..2]);
    // C has stopped. A's Session already pruned the verified tail; its ordinary
    // authored log cannot export another live player's commands.
    *c_fenced.borrow_mut() = true;
    assert!(peers[0].authored_since(0).iter().all(|r| r.slot != PlayerSlot(2)));
    let retained = peers[0].source().history().export_range(PlayerSlot(2), 3, 5).unwrap();
    assert_eq!(retained.len(), 3);
    for record in &retained {
        assert_eq!(record.input, input(2, record.tick));
        assert_eq!(record.commands, commands(2).iter().map(encode).collect::<Vec<_>>());
    }
    // Explicit trusted repair in this harness, after the original author is
    // fenced. No automatic repair path or network format is introduced.
    for record in retained {
        queues.borrow_mut()[1].push(packet(
            record.tick,
            record.slot.0,
            record.input,
            record
                .commands
                .iter()
                .map(|bytes| SpawnBulletCmd::decode(bytes).unwrap())
                .collect(),
        ));
    }
    assert!(peers[1].poll_confirmed().1.is_some());
    assert_eq!(peers[1].verified_tick(), 5);
    assert_eq!(peers[1].checksums(), expected);
    // Only A takes over C's slot at the explicitly agreed boundary, after B
    // holds the complete tail and all common checkpoints match headless.
    peers[0].mark_slot_vacant(PlayerSlot(2), 6).unwrap();
    for tick in 6..=8 {
        let mut inputs = Inputs::new(tick, 3);
        for slot in 0..2 {
            inputs.set_input(PlayerSlot(slot), input(slot, tick));
            for command in commands(slot) {
                inputs.push_command(PlayerSlot(slot), command);
            }
            peers[usize::from(slot)].advance(input(slot, tick), commands(slot));
        }
        for peer in &mut peers[..2] {
            peer.poll_confirmed();
        }
        reference.step(&inputs);
        expected.push((tick, reference.checksum()));
        assert_eq!(peers[0].checksums(), expected);
        assert_eq!(peers[1].checksums(), expected);
    }
}

#[test]
fn late_equal_input_commands_are_observed_once_after_resimulation() {
    let source = VerifiedHistorySource::new(Queue::default(), Archive::new(8, 2, LIMITS).unwrap());
    let mut peer = Session::new(ArenaConfig { player_count: 2 }, cfg(2, 0), source);
    peer.advance(ArenaInput::default(), vec![]);
    assert!(peer.source().history().is_empty());
    peer.source_mut()
        .inner_mut()
        .incoming
        .push(packet(1, 1, ArenaInput::default(), commands(1)));
    assert!(peer.poll_confirmed().1.is_some());
    assert_eq!(peer.source().inner().verified, [1]);
    let record = peer.source().history().export_range(PlayerSlot(1), 1, 1).unwrap();
    assert_eq!(record[0].commands, commands(1).iter().map(encode).collect::<Vec<_>>());
    peer.source_mut()
        .inner_mut()
        .incoming
        .push(packet(1, 1, input(1, 1), vec![]));
    peer.poll_confirmed();
    assert_eq!(peer.source().inner().verified, [1]);
    assert_eq!(
        peer.source().history().export_range(PlayerSlot(1), 1, 1).unwrap(),
        record
    );
}

fn observe(
    source: &mut VerifiedHistorySource<Arena, Queue>,
    tick: u64,
    simulated: &[SpawnBulletCmd],
    confirmed: &[SpawnBulletCmd],
    missing_input: bool,
) {
    let mut inputs = Inputs::new(tick, 1);
    for command in simulated {
        inputs.push_command(PlayerSlot(0), *command);
    }
    let bytes = simulated.iter().map(|c| (PlayerSlot(0), encode(c))).collect::<Vec<_>>();
    let confirmed_inputs = if missing_input {
        BTreeMap::new()
    } else {
        BTreeMap::from([(PlayerSlot(0), ArenaInput::default())])
    };
    let confirmed_commands = BTreeMap::from([(PlayerSlot(0), confirmed.to_vec())]);
    source.on_locally_verified(LocallyVerifiedTick {
        simulated: &inputs,
        simulated_commands: &bytes,
        confirmed_inputs: &confirmed_inputs,
        confirmed_commands: Some(&confirmed_commands),
        confirmed_absent: &BTreeSet::new(),
    });
}

#[test]
fn conflict_missing_evidence_and_missing_callbacks_fail_closed() {
    let mut source = VerifiedHistorySource::new(Queue::default(), Archive::new(2, 1, LIMITS).unwrap());
    observe(&mut source, 1, &commands(0), &commands(0), false);
    // All-slot comparison is independent of the historical predicted bit.
    observe(&mut source, 2, &commands(0), &commands(1), false);
    assert_eq!(source.history().retired_through(), 2);
    assert!(source.history().export_range(PlayerSlot(0), 1, 2).is_err());
    observe(&mut source, 3, &[], &[], true);
    assert_eq!(source.history().retired_through(), 3);
    observe(&mut source, 4, &[], &[], false);
    observe(&mut source, 6, &[], &[], false); // callback 5 never happened
    assert!(source.history().export_range(PlayerSlot(0), 4, 6).is_err());
    assert!(source.history().export_range(PlayerSlot(0), 6, 6).is_ok());
    observe(&mut source, 2, &commands(0), &commands(0), false);
    assert_eq!(
        source.history().export_range(PlayerSlot(0), 2, 2),
        Err(VerifiedHistoryError::Invalidated)
    );
    assert!(source.history().is_empty());
}

#[test]
fn exact_record_and_encoded_byte_caps_retire_whole_ticks() {
    let record_bytes = std::mem::size_of::<ArenaInput>() + 8 + encode(&commands(0)[0]).len();
    for limits in [
        VerifiedHistoryLimits {
            max_records: 2,
            max_encoded_bytes: usize::MAX,
        },
        VerifiedHistoryLimits {
            max_records: usize::MAX,
            max_encoded_bytes: 2 * record_bytes,
        },
    ] {
        let mut source = VerifiedHistorySource::new(Queue::default(), Archive::new(3, 1, limits).unwrap());
        for tick in 1..=3 {
            observe(&mut source, tick, &commands(0), &commands(0), false);
        }
        assert_eq!(source.history().len(), 2);
        assert_eq!(source.history().retained_encoded_bytes(), 2 * record_bytes);
        assert_eq!(source.history().retired_through(), 1);
        assert!(source.history().export_range(PlayerSlot(0), 1, 3).is_err());
        assert!(source.history().export_range(PlayerSlot(0), 2, 3).is_ok());
        observe(&mut source, 1, &commands(0), &commands(0), false);
        assert!(source.history().export_range(PlayerSlot(0), 1, 1).is_err());
    }
    let limits = VerifiedHistoryLimits {
        max_records: 1,
        max_encoded_bytes: record_bytes - 1,
    };
    let mut source = VerifiedHistorySource::new(Queue::default(), Archive::new(4, 1, limits).unwrap());
    observe(&mut source, 1, &commands(0), &commands(0), false);
    assert!(source.history().is_empty());
    assert_eq!(source.history().retired_through(), 1);
    observe(&mut source, 2, &[], &[], false);
    assert!(source.history().export_range(PlayerSlot(0), 2, 2).is_ok());
}

#[test]
fn codec_failure_and_too_small_limits_do_not_change_gameplay() {
    fn fail(_: &SpawnBulletCmd, _: &mut Vec<u8>) -> Result<(), VerifiedHistoryEncodeError> {
        Err(VerifiedHistoryEncodeError)
    }
    let archives = [
        Archive::with_encoder(1, 1, LIMITS, fail).unwrap(),
        Archive::new(
            2,
            1,
            VerifiedHistoryLimits {
                max_records: 0,
                max_encoded_bytes: 0,
            },
        )
        .unwrap(),
    ];
    for history in archives {
        let mut baseline = Session::<Arena, _>::new(ArenaConfig { player_count: 1 }, cfg(1, 0), LocalInputSource);
        let mut archived = Session::new(
            ArenaConfig { player_count: 1 },
            cfg(1, 0),
            VerifiedHistorySource::new(Queue::default(), history),
        );
        for tick in 1..=3 {
            baseline.advance(input(0, tick), commands(0));
            archived.advance(input(0, tick), commands(0));
            baseline.poll_confirmed();
            archived.poll_confirmed();
        }
        assert_eq!(archived.verified_tick(), 3);
        assert_eq!(archived.checksums(), baseline.checksums());
        assert_eq!(archived.source().inner().verified, [1, 2, 3]);
        assert!(archived.source().history().is_empty());
    }
}

#[test]
fn ranges_generation_and_invalidation_are_explicit() {
    assert!(matches!(
        Archive::new(1, 0, LIMITS),
        Err(VerifiedHistoryError::InvalidPlayerCount)
    ));
    let mut source = VerifiedHistorySource::new(Queue::default(), Archive::new(55, 1, LIMITS).unwrap());
    observe(&mut source, 1, &[], &[], false);
    for (slot, first, through) in [(0, 0, 1), (0, 2, 1), (1, 1, 1)] {
        assert_eq!(
            source.history().export_range(PlayerSlot(slot), first, through),
            Err(VerifiedHistoryError::InvalidRange)
        );
    }
    assert_eq!(
        source.history().export_range(PlayerSlot(0), 1, u64::MAX),
        Err(VerifiedHistoryError::Unavailable)
    );
    assert_eq!(
        source.history().export_range(PlayerSlot(0), u64::MAX, u64::MAX),
        Err(VerifiedHistoryError::Unavailable)
    );
    assert_eq!(source.history().generation(), 55);
    source.history_mut().invalidate();
    observe(&mut source, 2, &[], &[], false);
    assert_eq!(
        source.history().export_range(PlayerSlot(0), 1, 1),
        Err(VerifiedHistoryError::Invalidated)
    );
    assert_eq!(source.history().retained_encoded_bytes(), 0);
    let mut fresh = VerifiedHistorySource::new(Queue::default(), Archive::new(56, 1, LIMITS).unwrap());
    observe(&mut fresh, 2, &[], &[], false);
    assert!(fresh.history().export_range(PlayerSlot(0), 2, 2).is_ok());
}

#[test]
fn already_confirmed_conflicting_duplicate_is_not_certified_by_archive() {
    let mut queue = Queue::default();
    queue.incoming.push(packet(1, 1, ArenaInput::default(), commands(1)));
    let source = VerifiedHistorySource::new(queue, Archive::new(90, 2, LIMITS).unwrap());
    let mut peer = Session::new(ArenaConfig { player_count: 2 }, cfg(2, 0), source);
    peer.advance(ArenaInput::default(), vec![]);
    assert!(
        peer.source().history().is_empty(),
        "ingress is not verification evidence"
    );
    // The narrow gameplay fix intentionally does not reconcile arbitrary
    // conflicts in a slot that was already confirmed during simulation.
    peer.source_mut()
        .inner_mut()
        .incoming
        .push(packet(1, 1, ArenaInput::default(), commands(0)));
    peer.poll_confirmed();
    assert_eq!(peer.verified_tick(), 1);
    assert!(peer.source().history().is_empty());
    assert_eq!(peer.source().history().retired_through(), 1);
    assert!(peer.source().history().export_range(PlayerSlot(1), 1, 1).is_err());
    assert_eq!(peer.source().inner().verified, [1]);
}

#[test]
fn ordered_command_and_input_disagreement_are_rejected() {
    let mut source = VerifiedHistorySource::new(Queue::default(), Archive::new(91, 1, LIMITS).unwrap());
    let a = SpawnBulletCmd { owner: 0 };
    let b = SpawnBulletCmd { owner: 1 };
    observe(&mut source, 1, &[a, b], &[b, a], false);
    assert!(source.history().is_empty());
    let inputs = Inputs::new(2, 1);
    source.on_locally_verified(LocallyVerifiedTick {
        simulated: &inputs,
        simulated_commands: &[],
        confirmed_inputs: &BTreeMap::from([(PlayerSlot(0), input(0, 2))]),
        confirmed_commands: None,
        confirmed_absent: &BTreeSet::new(),
    });
    assert_eq!(source.history().retired_through(), 2);
    assert!(source.history().is_empty());
}

#[test]
fn default_callback_does_not_clone_or_encode_and_zero_byte_commands_are_bounded() {
    use orr_ecs::{ComponentRegistryBuilder, Frame};
    use orr_sim::{Game, System};
    use std::sync::atomic::{AtomicUsize, Ordering};
    static ENCODED: AtomicUsize = AtomicUsize::new(0);
    static CLONED: AtomicUsize = AtomicUsize::new(0);
    struct Command;
    impl Clone for Command {
        fn clone(&self) -> Self {
            CLONED.fetch_add(1, Ordering::SeqCst);
            Self
        }
    }
    impl SimCommand for Command {
        fn encode(&self, _: &mut Vec<u8>) {
            ENCODED.fetch_add(1, Ordering::SeqCst);
        }
        fn decode(bytes: &[u8]) -> Option<Self> {
            bytes.is_empty().then_some(Self)
        }
    }
    struct EmptyGame;
    impl Game for EmptyGame {
        type Input = u8;
        type Command = Command;
        type Event = u8;
        type Config = ();
        fn register(_: &mut ComponentRegistryBuilder) {}
        fn setup(_: &mut Frame, _: &()) {}
        fn systems() -> Vec<Box<dyn System<Self>>> {
            Vec::new()
        }
    }
    let mut peer = Session::<EmptyGame, _>::new((), cfg(1, 0), LocalInputSource);
    peer.advance(0, vec![Command]);
    let encoded_before = ENCODED.load(Ordering::SeqCst);
    let cloned_before = CLONED.load(Ordering::SeqCst);
    assert!(encoded_before > 0, "the normal simulation path recorded its bytes");
    peer.poll_confirmed();
    assert_eq!(peer.verified_tick(), 1);
    assert_eq!(ENCODED.load(Ordering::SeqCst), encoded_before);
    assert_eq!(CLONED.load(Ordering::SeqCst), cloned_before);

    // Zero-byte commands still consume length framing, bounding their metadata.
    let archive = LocallyVerifiedHistory::<EmptyGame>::new(
        94,
        1,
        VerifiedHistoryLimits {
            max_records: 10,
            max_encoded_bytes: 9,
        },
    )
    .unwrap();
    let mut archived =
        Session::<EmptyGame, _>::new((), cfg(1, 0), VerifiedHistorySource::new(LocalInputSource, archive));
    archived.advance(0, vec![Command]);
    archived.poll_confirmed();
    assert_eq!(archived.source().history().retained_encoded_bytes(), 9);
    assert_eq!(
        archived.source().history().export_range(PlayerSlot(0), 1, 1).unwrap()[0].commands,
        vec![Vec::<u8>::new()]
    );
    archived.advance(0, vec![Command, Command]);
    archived.poll_confirmed();
    assert_eq!(archived.verified_tick(), 2);
    assert!(archived.source().history().is_empty());
    assert_eq!(archived.source().history().retired_through(), 2);
}

#[test]
fn multi_player_admission_is_atomic_and_metadata_is_fixed() {
    let mut source = VerifiedHistorySource::new(
        Queue::default(),
        Archive::new(
            92,
            2,
            VerifiedHistoryLimits {
                max_records: 3,
                max_encoded_bytes: 1000,
            },
        )
        .unwrap(),
    );
    for tick in 1..=2 {
        let inputs = Inputs::new(tick, 2);
        source.on_locally_verified(LocallyVerifiedTick {
            simulated: &inputs,
            simulated_commands: &[],
            confirmed_inputs: &BTreeMap::from([
                (PlayerSlot(0), ArenaInput::default()),
                (PlayerSlot(1), ArenaInput::default()),
            ]),
            confirmed_commands: None,
            confirmed_absent: &BTreeSet::new(),
        });
    }
    assert_eq!(source.history().len(), 2);
    assert_eq!(source.history().retired_through(), 1);
    for slot in 0..2 {
        assert!(source.history().export_range(PlayerSlot(slot), 1, 1).is_err());
        assert!(source.history().export_range(PlayerSlot(slot), 2, 2).is_ok());
    }
    // A changed player universe cannot add unbounded slot metadata or mix games.
    observe(&mut source, 3, &[], &[], false);
    assert!(source.history().is_empty());
    assert_eq!(source.history().retired_through(), 3);
}

#[test]
fn missing_simulated_command_evidence_is_unavailable() {
    let mut source = VerifiedHistorySource::new(Queue::default(), Archive::new(93, 1, LIMITS).unwrap());
    let mut inputs = Inputs::new(1, 1);
    inputs.push_command(PlayerSlot(0), SpawnBulletCmd { owner: 0 });
    source.on_locally_verified(LocallyVerifiedTick {
        simulated: &inputs,
        simulated_commands: &[],
        confirmed_inputs: &BTreeMap::from([(PlayerSlot(0), ArenaInput::default())]),
        confirmed_commands: None,
        confirmed_absent: &BTreeSet::new(),
    });
    assert!(source.history().is_empty());
    assert_eq!(source.history().retired_through(), 1);
}
