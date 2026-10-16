//! Exact recovery evidence is observed at verification, never at network ingress.
use std::cell::RefCell;
use std::rc::Rc;

use orr_fp::FP;
use orr_session::{
    DepartureAck, DepartureBarrier, DepartureError, DepartureFence, InputSource,
    LocallyVerifiedHistory, RemoteInput, Session, SessionConfig, VerifiedHistoryLimits,
    VerifiedHistorySource,
};
use orr_sim::{PlayerSlot, SimCommand, Simulation, TickInputs};
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd};

type Archive = LocallyVerifiedHistory<Arena>;
type Inputs = TickInputs<ArenaInput, SpawnBulletCmd>;
const LIMITS: VerifiedHistoryLimits = VerifiedHistoryLimits {
    max_records: 300,
    max_encoded_bytes: 100_000,
};

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
    vec![
        SpawnBulletCmd {
            owner: u32::from(slot)
        };
        usize::from(slot) + 1
    ]
}
fn packet(
    tick: u64,
    slot: u8,
    input: ArenaInput,
    commands: Vec<SpawnBulletCmd>,
) -> RemoteInput<Arena> {
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
    fn send_local(
        &mut self,
        tick: u64,
        slot: PlayerSlot,
        input: ArenaInput,
        commands: Vec<SpawnBulletCmd>,
    ) {
        if self.owner == 2 && *self.c_fenced.borrow() {
            return;
        }
        for (destination, queue) in self.queues.borrow_mut().iter_mut().enumerate() {
            if destination == usize::from(self.owner)
                || (self.owner == 2 && destination == 1 && tick >= 3)
            {
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
    assert!(peers[0]
        .authored_since(0)
        .iter()
        .all(|r| r.slot != PlayerSlot(2)));
    let survivors = [PlayerSlot(0), PlayerSlot(1)];
    let mut barrier = DepartureBarrier::new(
        7,
        3,
        PlayerSlot(2),
        PlayerSlot(0),
        survivors.to_vec(),
        &[],
        peers[0].config(),
    )
    .unwrap();
    assert!(matches!(
        barrier.target(),
        Err(DepartureError::MissingFences)
    ));
    for peer in &peers[..2] {
        barrier
            .report_fence(DepartureFence {
                recovery_id: 7,
                revision: 3,
                survivor: peer.config().local_slot,
                verified_tick: peer.verified_tick(),
                departed_max: peer.last_remote_tick(PlayerSlot(2)),
            })
            .unwrap();
    }
    assert_eq!(barrier.repair_range(PlayerSlot(0)).unwrap(), None);
    assert_eq!(barrier.repair_range(PlayerSlot(1)).unwrap(), Some((3, 5)));
    assert!(matches!(
        barrier.acknowledgment(&peers[1]),
        Err(DepartureError::UnverifiedTarget)
    ));
    assert!(!barrier.ready().unwrap());
    assert!(matches!(
        barrier.commit(&mut peers[0], 3, &survivors),
        Err(DepartureError::MissingAcknowledgments)
    ));
    let retained = peers[0]
        .source()
        .history()
        .export_range(PlayerSlot(2), 3, 5)
        .unwrap();
    assert_eq!(retained.len(), 3);
    for record in &retained {
        assert_eq!(record.input, input(2, record.tick));
        assert_eq!(
            record.commands,
            commands(2).iter().map(encode).collect::<Vec<_>>()
        );
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
    for peer in &peers[..2] {
        let ack = barrier.acknowledgment(peer).unwrap();
        assert_eq!(ack.checksum, reference.checksum());
        barrier.acknowledge(ack).unwrap();
        barrier.acknowledge(ack).unwrap();
    }
    assert!(barrier.ready().unwrap());
    assert!(matches!(
        barrier.commit(&mut peers[1], 3, &survivors),
        Err(DepartureError::SessionMismatch)
    ));
    assert!(!barrier.is_committed());
    barrier.commit(&mut peers[0], 3, &survivors).unwrap();
    assert!(barrier.is_committed());
    assert!(matches!(
        barrier.commit(&mut peers[0], 3, &survivors),
        Err(DepartureError::Committed)
    ));
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

fn barrier() -> DepartureBarrier {
    DepartureBarrier::new(
        7,
        3,
        PlayerSlot(2),
        PlayerSlot(0),
        vec![PlayerSlot(0), PlayerSlot(1)],
        &[],
        &cfg(3, 0),
    )
    .unwrap()
}
fn fence(slot: u8, verified: u64, highest: Option<u64>) -> DepartureFence {
    DepartureFence {
        recovery_id: 7,
        revision: 3,
        survivor: PlayerSlot(slot),
        verified_tick: verified,
        departed_max: highest,
    }
}
#[test]
fn duplicates_stale_unknown_and_contradictory_fences() {
    let mut b = barrier();
    assert!(matches!(
        b.report_fence(fence(2, 5, None)),
        Err(DepartureError::UnknownSurvivor)
    ));
    let mut stale = fence(0, 5, None);
    stale.recovery_id = 8;
    assert!(matches!(
        b.report_fence(stale),
        Err(DepartureError::StaleContext)
    ));
    stale.recovery_id = 7;
    stale.revision = 4;
    assert!(matches!(
        b.report_fence(stale),
        Err(DepartureError::StaleContext)
    ));
    b.report_fence(fence(0, 5, None)).unwrap();
    b.report_fence(fence(0, 5, None)).unwrap();
    assert!(matches!(b.target(), Err(DepartureError::MissingFences)));
    assert!(matches!(
        b.report_fence(fence(0, 6, None)),
        Err(DepartureError::Contradiction)
    ));
    assert!(matches!(
        b.report_fence(fence(1, 5, None)),
        Err(DepartureError::Invalidated)
    ));
}
#[test]
fn snapshot_checkpoints_empty_ranges_and_delay_floor() {
    let mut config = cfg(3, 0);
    config.input_delay = 2;
    let mut b = DepartureBarrier::new(
        7,
        3,
        PlayerSlot(2),
        PlayerSlot(0),
        vec![PlayerSlot(0), PlayerSlot(1)],
        &[],
        &config,
    )
    .unwrap();
    // Snapshot peers need not have observed any remote input after restoration.
    b.report_fence(fence(0, 20, None)).unwrap();
    b.report_fence(fence(1, 0, None)).unwrap();
    assert_eq!(b.target().unwrap().target, 20);
    assert_eq!(b.target().unwrap().cutoff, 21);
    assert_eq!(b.repair_range(PlayerSlot(0)).unwrap(), None);
    assert_eq!(b.repair_range(PlayerSlot(1)).unwrap(), Some((3, 20)));
    let mut b = DepartureBarrier::new(
        7,
        3,
        PlayerSlot(2),
        PlayerSlot(0),
        vec![PlayerSlot(0), PlayerSlot(1)],
        &[],
        &config,
    )
    .unwrap();
    b.report_fence(fence(0, 0, None)).unwrap();
    b.report_fence(fence(1, 0, None)).unwrap();
    assert_eq!(b.target().unwrap().target, 2);
    assert_eq!(b.repair_range(PlayerSlot(0)).unwrap(), None);
}
#[test]
fn membership_changes_never_shrink_or_replace_survivors() {
    for (revision, survivors) in [
        (4, vec![PlayerSlot(0), PlayerSlot(1)]),
        (3, vec![PlayerSlot(0)]),
        (3, vec![PlayerSlot(0), PlayerSlot(2)]),
    ] {
        let mut b = barrier();
        assert!(matches!(
            b.check_membership(revision, &survivors),
            Err(DepartureError::MembershipChanged)
        ));
        assert!(matches!(b.target(), Err(DepartureError::Invalidated)));
    }
}
#[test]
fn overflow_is_terminal_from_verified_or_remote_tick() {
    for report in [fence(0, u64::MAX, None), fence(0, 5, Some(u64::MAX))] {
        let mut b = barrier();
        b.report_fence(report).unwrap();
        assert!(matches!(
            b.report_fence(fence(1, 5, None)),
            Err(DepartureError::TickExhausted)
        ));
        assert!(matches!(b.target(), Err(DepartureError::Invalidated)));
    }
}
#[test]
fn acks_require_exact_target_and_common_checksum() {
    let mut b = barrier();
    b.report_fence(fence(0, 5, None)).unwrap();
    b.report_fence(fence(1, 4, Some(7))).unwrap();
    let target = b.target().unwrap();
    assert_eq!(target.target, 7);
    let ack = DepartureAck {
        target,
        survivor: PlayerSlot(0),
        checksum: 55,
    };
    for mut wrong in [ack; 6].into_iter().enumerate() {
        match wrong.0 {
            0 => wrong.1.target.target += 1,
            1 => wrong.1.target.cutoff += 1,
            2 => wrong.1.target.owner = PlayerSlot(1),
            3 => wrong.1.target.revision += 1,
            4 => wrong.1.target.recovery_id += 1,
            _ => wrong.1.target.departed = PlayerSlot(1),
        }
        assert!(matches!(
            b.acknowledge(wrong.1),
            Err(DepartureError::StaleContext)
        ));
    }
    assert!(matches!(
        b.acknowledge(DepartureAck {
            survivor: PlayerSlot(9),
            ..ack
        }),
        Err(DepartureError::UnknownSurvivor)
    ));
    b.acknowledge(ack).unwrap();
    b.acknowledge(ack).unwrap();
    assert!(!b.ready().unwrap());
    assert!(matches!(
        b.acknowledge(DepartureAck {
            survivor: PlayerSlot(1),
            checksum: 66,
            ..ack
        }),
        Err(DepartureError::Contradiction)
    ));
    assert!(matches!(b.ready(), Err(DepartureError::Invalidated)));
}

struct Quiet;
impl InputSource<Arena> for Quiet {
    fn send_local(&mut self, _: u64, _: PlayerSlot, _: ArenaInput, _: Vec<SpawnBulletCmd>) {}
    fn poll_remote(&mut self) -> Vec<RemoteInput<Arena>> {
        Vec::new()
    }
}
#[test]
fn unavailable_history_and_unreconciled_prediction_cannot_acknowledge() {
    let mut peer = Session::new(ArenaConfig { player_count: 3 }, cfg(3, 1), Quiet);
    for _ in 0..5 {
        peer.advance(ArenaInput::default(), vec![]);
    }
    assert_eq!(peer.head_tick(), 5);
    assert_eq!(peer.verified_tick(), 0);
    let mut b = barrier();
    b.report_fence(fence(0, 5, Some(5))).unwrap();
    b.report_fence(fence(1, 0, None)).unwrap();
    let (first, through) = b.repair_range(PlayerSlot(1)).unwrap().unwrap();
    let unavailable = Archive::new(1, 3, LIMITS).unwrap();
    assert!(unavailable
        .export_range(PlayerSlot(2), first, through)
        .is_err());
    assert!(matches!(
        b.acknowledgment(&peer),
        Err(DepartureError::UnverifiedTarget)
    ));
    assert!(!b.ready().unwrap());
}
#[test]
fn snapshot_without_remote_maxima_and_failed_commit_stays_uncommitted() {
    let mut reference = Simulation::<Arena>::new(ArenaConfig { player_count: 3 }, 60, 42);
    for tick in 1..=5 {
        reference.step(&Inputs::new(tick, 3));
    }
    let mut peer = Session::new(ArenaConfig { player_count: 3 }, cfg(3, 0), Quiet);
    peer.restore_confirmed(5, reference.checksum(), &reference.frame().to_bytes(), 1)
        .unwrap();
    assert_eq!(peer.last_remote_tick(PlayerSlot(2)), None);
    let mut b = barrier();
    b.report_fence(fence(0, peer.verified_tick(), None))
        .unwrap();
    b.report_fence(fence(1, 5, None)).unwrap();
    let ack = b.acknowledgment(&peer).unwrap();
    b.acknowledge(ack).unwrap();
    b.acknowledge(DepartureAck {
        survivor: PlayerSlot(1),
        ..ack
    })
    .unwrap();
    let survivors = [PlayerSlot(0), PlayerSlot(1)];
    // Snapshot alone does not advance the existing owner's next send tick.
    assert!(matches!(
        b.commit(&mut peer, 3, &survivors),
        Err(DepartureError::Vacate(_))
    ));
    assert!(!b.is_committed());
    let mut changed = cfg(3, 0);
    changed.seed += 1;
    let mut replacement = Session::new(ArenaConfig { player_count: 3 }, changed, Quiet);
    assert!(matches!(
        b.commit(&mut replacement, 3, &survivors),
        Err(DepartureError::SessionMismatch)
    ));
    assert!(!b.is_committed());
}
#[test]
fn invalid_rosters_are_rejected() {
    for survivors in [
        vec![PlayerSlot(0)],
        vec![PlayerSlot(0), PlayerSlot(0), PlayerSlot(1)],
        vec![PlayerSlot(1), PlayerSlot(2)],
    ] {
        assert!(matches!(
            DepartureBarrier::new(
                7,
                3,
                PlayerSlot(2),
                PlayerSlot(0),
                survivors,
                &[],
                &cfg(3, 0)
            ),
            Err(DepartureError::InvalidConfig)
        ));
    }
}

#[test]
fn changed_owner_checkpoint_cannot_commit_stale_acknowledgments() {
    let mut peer = Session::new(ArenaConfig { player_count: 3 }, cfg(3, 0), Quiet);
    for _ in 0..5 {
        peer.advance(ArenaInput::default(), vec![]);
    }
    let mut reference = Simulation::<Arena>::new(ArenaConfig { player_count: 3 }, 60, 42);
    for tick in 1..=5 {
        reference.step(&Inputs::new(tick, 3));
    }
    peer.restore_confirmed(5, reference.checksum(), &reference.frame().to_bytes(), 1)
        .unwrap();
    let mut b = barrier();
    b.report_fence(fence(0, 5, None)).unwrap();
    b.report_fence(fence(1, 5, None)).unwrap();
    let ack = b.acknowledgment(&peer).unwrap();
    b.acknowledge(ack).unwrap();
    b.acknowledge(DepartureAck {
        survivor: PlayerSlot(1),
        ..ack
    })
    .unwrap();
    let mut changed = Simulation::<Arena>::new(ArenaConfig { player_count: 3 }, 60, 42);
    for tick in 1..=5 {
        let mut inputs = Inputs::new(tick, 3);
        inputs.set_input(PlayerSlot(0), input(0, tick));
        changed.step(&inputs);
    }
    assert_ne!(reference.checksum(), changed.checksum());
    peer.restore_confirmed(5, changed.checksum(), &changed.frame().to_bytes(), 1)
        .unwrap();
    assert!(matches!(
        b.commit(&mut peer, 3, &[PlayerSlot(0), PlayerSlot(1)]),
        Err(DepartureError::Contradiction)
    ));
    assert!(!b.is_committed());
    assert!(matches!(b.ready(), Err(DepartureError::Invalidated)));
}

#[test]
fn verified_tick_changes_and_explicit_session_invalidation_block_commit() {
    let mut peer = Session::new(ArenaConfig { player_count: 3 }, cfg(3, 0), Quiet);
    for _ in 0..6 {
        peer.advance(ArenaInput::default(), vec![]);
    }
    let mut reference = Simulation::<Arena>::new(ArenaConfig { player_count: 3 }, 60, 42);
    for tick in 1..=5 {
        reference.step(&Inputs::new(tick, 3));
    }
    peer.restore_confirmed(5, reference.checksum(), &reference.frame().to_bytes(), 1)
        .unwrap();
    let mut b = barrier();
    b.report_fence(fence(0, 5, None)).unwrap();
    b.report_fence(fence(1, 5, None)).unwrap();
    let ack = b.acknowledgment(&peer).unwrap();
    b.acknowledge(ack).unwrap();
    b.acknowledge(DepartureAck {
        survivor: PlayerSlot(1),
        ..ack
    })
    .unwrap();
    reference.step(&Inputs::new(6, 3));
    peer.restore_confirmed(6, reference.checksum(), &reference.frame().to_bytes(), 1)
        .unwrap();
    assert!(matches!(
        b.commit(&mut peer, 3, &[PlayerSlot(0), PlayerSlot(1)]),
        Err(DepartureError::UnverifiedTarget)
    ));
    assert!(!b.is_committed());
    // Even a same-configuration restoration must be explicitly invalidated.
    b.invalidate();
    assert!(matches!(
        b.commit(&mut peer, 3, &[PlayerSlot(0), PlayerSlot(1)]),
        Err(DepartureError::Invalidated)
    ));
    assert!(!b.is_committed());
}
