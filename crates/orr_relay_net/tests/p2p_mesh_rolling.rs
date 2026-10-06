//! Bounded rolling mesh retention, admission pin, authority and verification regressions.
use std::cell::Cell;
use std::collections::BTreeSet;

use orr_net::SendError;
use orr_proto::{Channel, ConnId};
use orr_relay_net::p2p_mesh_input::{
    P2pMeshDelivery as Delivery, P2pMeshEdge as Edge, P2pMeshInputAccepted as Accepted,
    P2pMeshInputDriver, P2pMeshInputError as Error, P2pMeshInputLimits as Limits,
    P2pMeshInputSource, P2pMeshRollingWindow as Window,
};
use orr_relay_net::P2pInputCodec;
use orr_session::{
    checked_backlog_notice, serve_checked_join, CheckedJoinContext, CheckedJoinTicket, InputSource,
    JoinBootstrap, JoinRoster, LocallyVerifiedTick, PlayerSlot, RemoteInput, Session,
    SessionConfig,
};
use orr_sim::{Simulation, TickInputs, FP};
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd};

thread_local! {
    static DECODE_COUNT: Cell<usize> = const { Cell::new(0) };
    static ENCODE_COUNT: Cell<usize> = const { Cell::new(0) };
}
struct PortableArena;
impl P2pInputCodec<Arena> for PortableArena {
    const SCHEMA: u64 = 0x414e_454d_4553_4831;
    fn encode_input(i: &ArenaInput, out: &mut Vec<u8>) {
        ENCODE_COUNT.with(|c| c.set(c.get() + 1));
        out.extend_from_slice(&i.axis_x.0.to_le_bytes());
        out.extend_from_slice(&i.axis_y.0.to_le_bytes());
        out.extend_from_slice(&i.buttons.to_le_bytes());
    }
    fn decode_input(b: &[u8]) -> Option<ArenaInput> {
        DECODE_COUNT.with(|c| c.set(c.get() + 1));
        if b.len() != 20 {
            return None;
        }
        let mut i = ArenaInput::default();
        i.axis_x.0 = i64::from_le_bytes(b[0..8].try_into().ok()?);
        i.axis_y.0 = i64::from_le_bytes(b[8..16].try_into().ok()?);
        i.buttons = u32::from_le_bytes(b[16..20].try_into().ok()?);
        ((-65_536..=65_536).contains(&i.axis_x.0)
            && (-65_536..=65_536).contains(&i.axis_y.0)
            && i.buttons <= 1)
            .then_some(i)
    }
    fn encode_command(c: &SpawnBulletCmd, out: &mut Vec<u8>) {
        out.extend_from_slice(&c.owner.to_le_bytes());
    }
    fn decode_command(b: &[u8]) -> Option<SpawnBulletCmd> {
        let owner = u32::from_le_bytes(b.try_into().ok()?);
        (owner < 3).then_some(SpawnBulletCmd { owner })
    }
}
type Driver = P2pMeshInputDriver<Arena, PortableArena>;
type Source = P2pMeshInputSource<Arena, PortableArena>;
type Bootstrap = JoinBootstrap<Arena, Source>;
fn roster() -> JoinRoster {
    JoinRoster::completed(
        3,
        PlayerSlot(2),
        PlayerSlot(0),
        vec![PlayerSlot(0), PlayerSlot(1)],
        &[],
    )
    .unwrap()
}
fn context() -> CheckedJoinContext {
    CheckedJoinContext::new(17, 1, roster()).unwrap()
}
fn cfg(slot: u8) -> SessionConfig {
    let mut c = SessionConfig::new(3, PlayerSlot(slot), 42, 60);
    c.join_id = 17;
    c.input_delay = 0;
    c.input_log_ticks = 512;
    c.checksum_interval = 1;
    c
}
fn game() -> ArenaConfig {
    ArenaConfig { player_count: 3 }
}
fn limits() -> Limits {
    Limits {
        max_logical_records: 64,
        max_incoming_records: 64,
        max_destination_records: 128,
        max_edge_pending_records: 64,
        max_logical_encoded_bytes: 64 * 256,
        max_incoming_encoded_bytes: 64 * 256,
        max_pending_encoded_bytes: 128 * 256,
        max_edge_pending_encoded_bytes: 64 * 256,
        ..Limits::default()
    }
}
fn new(slot: u8, recent: u64, future: u64, limits: Limits) -> (Driver, Source) {
    Driver::new_rolling(
        PlayerSlot(slot),
        context(),
        limits,
        Window {
            recent_ticks: recent,
            future_ticks: future,
        },
    )
    .unwrap()
}
// Synthetic observations are limited to retention/arithmetic adversarial tests.
// The long Session test below exercises the real verification callback.
fn observe(source: &mut Source, tick: u64) {
    let simulated = TickInputs::new(tick, 3);
    let confirmed_inputs = (0..3)
        .map(|slot| (PlayerSlot(slot), ArenaInput::default()))
        .collect();
    source.on_locally_verified(LocallyVerifiedTick {
        simulated: &simulated,
        simulated_commands: &[],
        confirmed_inputs: &confirmed_inputs,
        confirmed_commands: None,
        confirmed_absent: &BTreeSet::new(),
    });
}
fn edge(remote: u8) -> Edge {
    Edge {
        connection: ConnId(10 + u32::from(remote)),
        adapter_id: 100 + u64::from(remote),
        raw_connection: ConnId(1),
        remote: PlayerSlot(remote),
        generation: 77,
    }
}
fn record(tick: u64, slot: u8, owners: &[u32]) -> RemoteInput<Arena> {
    RemoteInput {
        tick,
        slot: PlayerSlot(slot),
        input: ArenaInput::default(),
        commands: owners
            .iter()
            .map(|&owner| SpawnBulletCmd { owner })
            .collect(),
        disconnected: false,
    }
}
struct DonorFeed {
    verified: u64,
    incoming: Vec<RemoteInput<Arena>>,
}
impl InputSource<Arena> for DonorFeed {
    fn send_local(&mut self, tick: u64, slot: PlayerSlot, _: ArenaInput, _: Vec<SpawnBulletCmd>) {
        if slot == PlayerSlot(0) && tick <= self.verified {
            self.incoming.push(record(tick, 1, &[]));
        }
    }
    fn poll_remote(&mut self) -> Vec<RemoteInput<Arena>> {
        std::mem::take(&mut self.incoming)
    }
}
fn grant(snapshot_tick: u64, first_tick: u64) -> (CheckedJoinTicket, Vec<u8>, Bootstrap) {
    assert!(snapshot_tick < first_tick);
    let mut b = Bootstrap::new(cfg(2), roster(), 4096, 1 << 20).unwrap();
    let request = b.next_request().unwrap();
    let mut c = cfg(0);
    c.vacant_slots = vec![PlayerSlot(2)];
    let mut donor = Session::<Arena, _>::new(
        game(),
        c,
        DonorFeed {
            verified: snapshot_tick,
            incoming: vec![],
        },
    );
    for _ in 1..first_tick {
        donor.advance(ArenaInput::default(), vec![]);
    }
    donor.poll_confirmed();
    let (snapshot, ticket) = serve_checked_join(&mut donor, &context(), &request).unwrap();
    assert_eq!(ticket.ticket().snapshot_tick, snapshot_tick);
    assert_eq!(ticket.ticket().first_input_tick, first_tick);
    (ticket, snapshot, b)
}
fn two_edges(recent: u64, future: u64, l: Limits) -> (Driver, Source) {
    let (d, s) = new(0, recent, future, l);
    d.admit_edge(edge(1)).unwrap();
    d.install_ticket(&grant(0, 1).0).unwrap();
    d.admit_edge(edge(2)).unwrap();
    (d, s)
}
fn packet(author: u8, tick: u64, slot: u8, owners: &[u32]) -> Vec<u8> {
    // Reuse a legal finite frame, then change only the test tick. The rolling
    // receiver still checks its own tick horizon before any application decode.
    let (d, _) = Driver::new(PlayerSlot(author), context(), Limits::default()).unwrap();
    d.admit_edge(edge(1 - author)).unwrap();
    d.send_local(record(1, slot, owners)).unwrap();
    let mut out = vec![];
    d.flush(|_, b| {
        out = b.to_vec();
        Ok(())
    })
    .unwrap();
    out[35..43].copy_from_slice(&tick.to_le_bytes());
    out
}
fn wire_tick(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes[35..43].try_into().unwrap())
}
type Usage = (usize, usize);
type Accounting = (Usage, Usage, usize, Usage);
fn stats(d: &Driver) -> Accounting {
    (
        d.retained_evidence(),
        d.pending(),
        d.destination_records(),
        d.incoming(),
    )
}

#[test]
fn rolling_ignores_finite_ticks_but_preserves_all_caps() {
    let l = Limits {
        first_tick: 0,
        end_tick: 0,
        ..limits()
    };
    assert!(Driver::new(PlayerSlot(0), context(), l.clone()).is_err());
    let (d, mut s) = new(0, 1, 2, l);
    d.admit_edge(edge(1)).unwrap();
    for tick in 1..=700 {
        d.send_local(record(tick, 0, &[])).unwrap();
        d.flush(|_, _| Ok(())).unwrap();
        observe(&mut s, tick);
        assert_eq!(d.retained_evidence(), (1, 35));
        assert_eq!(d.destination_records(), 1);
        assert_eq!(d.pending(), (0, 0));
    }
    assert_eq!(d.rolling_progress(), Some((700, 699)));
    for w in [
        Window {
            recent_ticks: 0,
            future_ticks: 1,
        },
        Window {
            recent_ticks: 1,
            future_ticks: 0,
        },
    ] {
        assert_eq!(
            Driver::new_rolling(PlayerSlot(0), context(), limits(), w).err(),
            Some(Error::InvalidConfig)
        );
    }
    assert_eq!(
        Driver::new_rolling(
            PlayerSlot(0),
            context(),
            Limits {
                max_logical_records: 1537,
                ..limits()
            },
            Window {
                recent_ticks: 1,
                future_ticks: 2
            }
        )
        .err(),
        Some(Error::InvalidConfig)
    );
}

#[test]
fn imported_tickets_on_either_existing_peer_do_not_advance_progress_or_horizon() {
    let ticket = grant(20, 21).0;
    for slot in [0, 1] {
        let (d, mut s) = new(slot, 1, 4, limits());
        d.admit_edge(edge(1 - slot)).unwrap();
        d.install_ticket(&ticket).unwrap();
        assert_eq!(d.rolling_progress(), Some((0, 0)));
        observe(&mut s, 3);
        assert_eq!(d.rolling_progress(), Some((3, 2)));
        assert_eq!(d.send_local(record(8, slot, &[])), Err(Error::TickRange));
        assert_eq!(d.rolling_progress(), Some((3, 2)));
    }
}

#[test]
fn exact_joiner_snapshot_seeds_progress_and_rejects_a_different_source() {
    let (_, snapshot, mut boot) = grant(20, 21);
    let (d, s) = new(2, 1, 2, limits());
    boot.receive_snapshot(PlayerSlot(0), game(), s, &snapshot)
        .unwrap();
    d.install_joiner_snapshot(&boot).unwrap();
    assert_eq!(d.rolling_progress(), Some((20, 20)));
    d.install_joiner_snapshot(&boot).unwrap();
    d.admit_edge(edge(0)).unwrap();
    d.send_local(record(22, 2, &[2, 2])).unwrap();
    assert_eq!(d.send_local(record(23, 2, &[])), Err(Error::TickRange));
    let (other, _) = new(2, 1, 2, limits());
    assert_eq!(
        other.install_joiner_snapshot(&boot),
        Err(Error::InvalidSnapshot)
    );
    assert_eq!(other.rolling_progress(), Some((0, 0)));
}

#[test]
fn future_packets_duplicates_polling_prediction_and_sends_are_not_verification() {
    let (d, s) = new(0, 1, 3, limits());
    d.admit_edge(edge(1)).unwrap();
    let mut config = cfg(0);
    config.vacant_slots = vec![PlayerSlot(2)];
    let mut peer = Session::<Arena, _>::new(game(), config, s);
    let p = packet(1, 3, 1, &[]);
    assert_eq!(
        d.receive(edge(1).connection, Channel::Reliable, &p),
        Ok(Accepted::New)
    );
    assert_eq!(
        d.receive(edge(1).connection, Channel::Reliable, &p),
        Ok(Accepted::Duplicate)
    );
    peer.poll_confirmed();
    for _ in 0..2 {
        peer.advance(ArenaInput::default(), vec![]);
        d.flush(|_, _| Ok(())).unwrap();
    }
    assert_eq!(peer.verified_tick(), 0);
    assert_eq!(d.rolling_progress(), Some((0, 0)));
    assert_eq!(
        d.receive(edge(1).connection, Channel::Reliable, &packet(1, 4, 1, &[])),
        Err(Error::TickRange)
    );
}

#[test]
fn fresh_snapshot_pins_old_and_new_local_history_until_atomic_new_edge_admission() {
    let (d, mut s) = new(1, 1, 8, limits());
    d.admit_edge(edge(0)).unwrap();
    for tick in 1..=6 {
        d.send_local(record(tick, 1, &[1, 0, 1])).unwrap();
        d.flush(|_, _| Ok(())).unwrap();
        observe(&mut s, tick);
    }
    assert_eq!(d.retained_evidence(), (1, 59));
    let ticket = grant(5, 7).0;
    d.install_ticket(&ticket).unwrap();
    // Record 6 predates installation; all newly authored post-snapshot records
    // must receive the same pin when subsequent verification crosses them.
    for tick in 7..=12 {
        d.send_local(record(tick, 1, &[1, 0, 1])).unwrap();
        d.flush(|_, _| Ok(())).unwrap();
        observe(&mut s, tick);
    }
    assert_eq!(d.rolling_progress(), Some((12, 11)));
    assert_eq!(d.retained_evidence(), (7, 7 * 59));
    assert_eq!(d.destination_records(), 1);
    assert_eq!(d.delivery(edge(0).connection, 6, PlayerSlot(1)), None);
    d.admit_edge(edge(2)).unwrap();
    assert_eq!(d.retained_evidence(), (1, 59));
    assert_eq!(d.pending(), (7, 7 * 94));
    assert_eq!(d.destination_records(), 8);
    let mut ticks = vec![];
    d.flush(|id, bytes| {
        assert_eq!(id, edge(2).connection);
        ticks.push(wire_tick(bytes));
        Ok(())
    })
    .unwrap();
    assert_eq!(ticks, (6..=12).collect::<Vec<_>>());
    assert_eq!(d.destination_records(), 2);
    assert_eq!(d.pending(), (0, 0));
    // Reinstallation remains idempotent despite newer discarded local history.
    d.install_ticket(&ticket).unwrap();
}

#[test]
fn stale_snapshot_and_original_edge_after_discarded_history_fail_closed() {
    let (d, mut s) = new(1, 1, 4, limits());
    d.admit_edge(edge(0)).unwrap();
    for tick in 1..=6 {
        d.send_local(record(tick, 1, &[])).unwrap();
        d.flush(|_, _| Ok(())).unwrap();
        observe(&mut s, tick);
    }
    let before = stats(&d);
    assert_eq!(d.install_ticket(&grant(4, 7).0), Err(Error::SnapshotTooOld));
    assert_eq!(stats(&d), before);
    assert_eq!(
        d.resolve_connection(edge(2).adapter_id, ConnId(1)),
        Err(Error::SnapshotTooOld)
    );
    let (d, mut s) = new(0, 1, 4, limits());
    d.send_local(record(1, 0, &[])).unwrap();
    observe(&mut s, 2);
    assert_eq!(d.retained_evidence(), (0, 0));
    assert_eq!(d.admit_edge(edge(1)), Err(Error::SnapshotTooOld));
    assert_eq!(d.pending(), (0, 0));
}

#[test]
fn pin_uses_existing_caps_and_failed_backfill_never_partially_admits() {
    for l in [
        Limits {
            max_logical_records: 2,
            ..limits()
        },
        Limits {
            max_logical_encoded_bytes: 70,
            ..limits()
        },
    ] {
        let (d, mut s) = new(1, 1, 4, l);
        d.admit_edge(edge(0)).unwrap();
        d.install_ticket(&grant(0, 1).0).unwrap();
        for tick in 1..=2 {
            d.send_local(record(tick, 1, &[])).unwrap();
            d.flush(|_, _| Ok(())).unwrap();
            observe(&mut s, tick + 1);
        }
        let before = stats(&d);
        assert!(matches!(
            d.send_local(record(3, 1, &[])),
            Err(Error::RecordLimit | Error::ByteLimit)
        ));
        assert_eq!(stats(&d), before);
        assert_eq!(d.retained_evidence(), (2, 70));
    }
    let (d, mut s) = new(
        1,
        1,
        8,
        Limits {
            max_destination_records: 1,
            ..limits()
        },
    );
    d.admit_edge(edge(0)).unwrap();
    d.install_ticket(&grant(0, 1).0).unwrap();
    for tick in 1..=3 {
        d.send_local(record(tick, 1, &[])).unwrap();
        d.flush(|_, _| Ok(())).unwrap();
        observe(&mut s, tick + 1);
    }
    let before = stats(&d);
    assert_eq!(d.admit_edge(edge(2)), Err(Error::RecordLimit));
    assert_eq!(stats(&d), before);
    assert_eq!(
        d.edge_pending(edge(2).connection),
        Err(Error::WrongConnection)
    );
}

#[test]
fn old_pending_survives_verification_snapshot_and_other_destination_acceptance() {
    let (d, mut s) = new(0, 1, 8, limits());
    d.admit_edge(edge(1)).unwrap();
    for tick in 1..=6 {
        d.send_local(record(tick, 0, &[0, 1, 0])).unwrap();
        observe(&mut s, tick);
    }
    assert_eq!(d.retained_evidence(), (1, 59));
    let old_pending = d.pending();
    d.install_ticket(&grant(6, 7).0).unwrap();
    d.admit_edge(edge(2)).unwrap();
    assert_eq!(d.pending(), old_pending);
    d.send_local(record(7, 0, &[0, 1, 0])).unwrap();
    let sent = d
        .flush(|id, _| {
            if id == edge(1).connection {
                Err(SendError::Backpressure)
            } else {
                Ok(())
            }
        })
        .unwrap();
    assert_eq!((sent.sent, sent.pending), (1, 7));
    observe(&mut s, 20);
    assert_eq!(d.retained_evidence(), (0, 0));
    assert_eq!(d.destination_records(), 7);
    assert_eq!(d.pending(), (7, 7 * 94));
    for tick in 1..=7 {
        assert_eq!(
            d.delivery(edge(1).connection, tick, PlayerSlot(0)),
            Some(Delivery::Pending)
        );
    }
    let mut ticks = vec![];
    d.flush(|id, bytes| {
        assert_eq!(id, edge(1).connection);
        ticks.push(wire_tick(bytes));
        Ok(())
    })
    .unwrap();
    assert_eq!(ticks, (1..=7).collect::<Vec<_>>());
    assert_eq!(d.pending(), (0, 0));
    assert_eq!(d.edge_pending(edge(1).connection), Ok((0, 0)));
    assert_eq!(d.destination_records(), 0);
    assert_eq!(d.flush(|_, _| panic!("already accepted")).unwrap().sent, 0);
}

#[test]
fn incoming_is_independently_retained_even_after_its_canonical_evidence_retires() {
    let (d, mut s) = new(1, 1, 4, limits());
    d.admit_edge(edge(0)).unwrap();
    let p = packet(0, 1, 0, &[0, 0, 1]);
    d.receive(edge(0).connection, Channel::Reliable, &p)
        .unwrap();
    observe(&mut s, 3);
    assert_eq!(d.retained_evidence(), (0, 0));
    assert_eq!(d.incoming(), (1, 94));
    assert_eq!(
        d.receive(edge(0).connection, Channel::Reliable, &p),
        Ok(Accepted::IgnoredRetired)
    );
    let records = s.poll_remote();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0]
            .commands
            .iter()
            .map(|c| c.owner)
            .collect::<Vec<_>>(),
        [0, 0, 1]
    );
    assert_eq!(d.incoming(), (0, 0));
}

#[test]
fn verification_inside_flush_preserves_inflight_head_and_bounded_initial_work() {
    let (d, mut s) = two_edges(1, 8, limits());
    d.send_local(record(1, 0, &[])).unwrap();
    let mut calls = vec![];
    let out = d
        .flush(|id, bytes| {
            calls.push((id, wire_tick(bytes)));
            observe(&mut s, 4);
            assert_eq!(d.delivery(id, 1, PlayerSlot(0)), Some(Delivery::Pending));
            if calls.len() == 1 {
                d.send_local(record(5, 0, &[])).unwrap();
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(calls, [(edge(1).connection, 1), (edge(2).connection, 1)]);
    assert_eq!((out.sent, out.pending), (2, 2));
    assert_eq!(d.pending(), (2, 140));
    assert_eq!(d.destination_records(), 2);
    for remote in [1, 2] {
        assert_eq!(d.delivery(edge(remote).connection, 1, PlayerSlot(0)), None);
        assert_eq!(
            d.delivery(edge(remote).connection, 5, PlayerSlot(0)),
            Some(Delivery::Pending)
        );
    }
    observe(&mut s, 6);
    d.flush(|_, _| Ok(())).unwrap();
    assert_eq!(d.destination_records(), 0);
    assert_eq!(d.pending(), (0, 0));
}

#[test]
fn cancellation_and_reentrant_flush_still_preserve_terminal_inert_accounting() {
    for cancel in [true, false] {
        let (d, mut s) = two_edges(1, 8, limits());
        d.send_local(record(1, 0, &[])).unwrap();
        let before = d.pending();
        let error = if cancel {
            Error::Cancelled
        } else {
            Error::ReentrantFlush
        };
        assert_eq!(
            d.flush(|_, _| {
                observe(&mut s, 4);
                if cancel {
                    d.cancel();
                } else {
                    assert_eq!(d.flush(|_, _| Ok(())), Err(error.clone()));
                }
                Ok(())
            }),
            Err(error)
        );
        assert_eq!(d.pending(), before);
        assert_eq!(d.destination_records(), 2);
    }
}

#[test]
fn vacancy_authority_high_water_survives_canonical_pruning_on_author_and_recipient() {
    let ticket = grant(5, 6).0;
    for slot in [0, 1] {
        let (d, mut s) = new(slot, 1, 8, limits());
        d.admit_edge(edge(1 - slot)).unwrap();
        if slot == 0 {
            d.send_local(record(6, 2, &[])).unwrap();
            d.send_local(record(2, 2, &[])).unwrap();
        } else {
            for tick in [6, 2] {
                d.receive(
                    edge(0).connection,
                    Channel::Reliable,
                    &packet(0, tick, 2, &[]),
                )
                .unwrap();
            }
        }
        d.flush(|_, _| Ok(())).unwrap();
        observe(&mut s, 8);
        assert_eq!(d.retained_evidence(), (0, 0));
        // On the author a snapshot covering discarded tick 6 also forces first
        // > 6, so use a retired remotely received vacancy to isolate this guard.
        let result = d.install_ticket(&ticket);
        assert_eq!(
            result,
            Err(if slot == 0 {
                Error::SnapshotTooOld
            } else {
                Error::WrongAuthority
            })
        );
    }
}

#[test]
fn recent_conflicting_duplicates_are_terminal_without_replacing_evidence() {
    let (d, mut s) = new(1, 2, 8, limits());
    d.admit_edge(edge(0)).unwrap();
    d.receive(
        edge(0).connection,
        Channel::Reliable,
        &packet(0, 3, 0, &[0, 1, 0]),
    )
    .unwrap();
    observe(&mut s, 4);
    let before = stats(&d);
    assert_eq!(
        d.receive(
            edge(0).connection,
            Channel::Reliable,
            &packet(0, 3, 0, &[1, 0, 0])
        ),
        Err(Error::ConflictingDuplicate {
            tick: 3,
            slot: PlayerSlot(0)
        })
    );
    assert_eq!(stats(&d), before);
    assert!(s.poll_remote().is_empty());
}

fn retired_receiver() -> (Driver, Source) {
    let (d, mut s) = new(1, 1, 8, limits());
    d.admit_edge(edge(0)).unwrap();
    d.install_ticket(&grant(2, 3).0).unwrap();
    observe(&mut s, 8);
    (d, s)
}
fn reject_retired(bytes: &[u8], connection: ConnId, channel: Channel, expected: Error) {
    let (d, mut s) = retired_receiver();
    DECODE_COUNT.with(|c| c.set(0));
    assert_eq!(d.receive(connection, channel, bytes), Err(expected));
    DECODE_COUNT.with(|c| assert_eq!(c.get(), 0));
    assert_eq!(d.retained_evidence(), (0, 0));
    assert!(s.poll_remote().is_empty());
}
#[test]
fn retired_packets_validate_context_identity_authority_framing_and_vacancy_before_ignore() {
    let p = packet(0, 1, 0, &[0, 1, 0]);
    let (d, mut s) = retired_receiver();
    let mut undecodable = p.clone();
    undecodable[50..58].copy_from_slice(&i64::MAX.to_le_bytes());
    DECODE_COUNT.with(|c| c.set(0));
    assert_eq!(
        d.receive(edge(0).connection, Channel::Reliable, &undecodable),
        Ok(Accepted::IgnoredRetired)
    );
    DECODE_COUNT.with(|c| assert_eq!(c.get(), 0));
    assert_eq!(d.retained_evidence(), (0, 0));
    assert!(s.poll_remote().is_empty());
    for (offset, expected) in [
        (0, Error::Malformed),
        (4, Error::Malformed),
        (6, Error::WrongSchema),
        (14, Error::WrongContext),
        (22, Error::WrongContext),
        (26, Error::WrongGeneration),
        (34, Error::WrongRecipient),
        (43, Error::WrongAuthority),
    ] {
        let mut bad = p.clone();
        bad[offset] ^= 0x80;
        reject_retired(&bad, edge(0).connection, Channel::Reliable, expected);
    }
    reject_retired(&p, ConnId(99), Channel::Reliable, Error::WrongConnection);
    reject_retired(
        &p,
        edge(0).connection,
        Channel::Unreliable,
        Error::WrongChannel,
    );
    for end in 0..p.len() {
        reject_retired(
            &p[..end],
            edge(0).connection,
            Channel::Reliable,
            Error::Malformed,
        );
    }
    reject_retired(
        &vec![0; limits().max_packet_bytes + 1],
        edge(0).connection,
        Channel::Reliable,
        Error::PacketLimit,
    );
    let mut trailing = p.clone();
    trailing.push(0);
    reject_retired(
        &trailing,
        edge(0).connection,
        Channel::Reliable,
        Error::Malformed,
    );
    for (range, replacement, error) in [
        (44..48, u32::MAX.to_le_bytes().to_vec(), Error::InputLimit),
        (48..50, u16::MAX.to_le_bytes().to_vec(), Error::CommandLimit),
        (70..74, u32::MAX.to_le_bytes().to_vec(), Error::CommandLimit),
    ] {
        let mut bad = p.clone();
        bad[range].copy_from_slice(&replacement);
        reject_retired(&bad, edge(0).connection, Channel::Reliable, error);
    }
    let mut bad = packet(0, 1, 2, &[]);
    bad[50] = 1;
    reject_retired(
        &bad,
        edge(0).connection,
        Channel::Reliable,
        Error::InvalidVacancy,
    );
    let mut bad = p.clone();
    bad[43] = 2;
    reject_retired(
        &bad,
        edge(0).connection,
        Channel::Reliable,
        Error::InvalidVacancy,
    );
    reject_retired(
        &packet(0, 3, 2, &[]),
        edge(0).connection,
        Channel::Reliable,
        Error::WrongAuthority,
    );
}

#[test]
fn tiny_independent_incoming_and_pending_caps_remain_fail_closed() {
    for l in [
        Limits {
            max_incoming_records: 1,
            ..limits()
        },
        Limits {
            max_incoming_encoded_bytes: 70,
            ..limits()
        },
    ] {
        let (d, mut s) = new(1, 1, 4, l);
        d.admit_edge(edge(0)).unwrap();
        d.receive(edge(0).connection, Channel::Reliable, &packet(0, 1, 0, &[]))
            .unwrap();
        observe(&mut s, 2);
        let before = stats(&d);
        assert!(matches!(
            d.receive(edge(0).connection, Channel::Reliable, &packet(0, 3, 0, &[])),
            Err(Error::RecordLimit | Error::ByteLimit)
        ));
        assert_eq!(stats(&d), before);
    }
    for l in [
        Limits {
            max_edge_pending_records: 1,
            ..limits()
        },
        Limits {
            max_edge_pending_encoded_bytes: 70,
            ..limits()
        },
        Limits {
            max_pending_encoded_bytes: 140,
            ..limits()
        },
    ] {
        let (d, mut s) = two_edges(1, 4, l);
        d.send_local(record(1, 0, &[])).unwrap();
        observe(&mut s, 2);
        let before = stats(&d);
        assert!(matches!(
            d.send_local(record(3, 0, &[])),
            Err(Error::RecordLimit | Error::ByteLimit)
        ));
        assert_eq!(stats(&d), before);
    }
}

#[test]
fn horizon_reserves_two_send_cursor_increments_and_exhaustion_is_terminal() {
    for future in [u64::MAX - 1, u64::MAX] {
        assert_eq!(
            Driver::new_rolling(
                PlayerSlot(0),
                context(),
                limits(),
                Window {
                    recent_ticks: 1,
                    future_ticks: future
                }
            )
            .err(),
            Some(Error::TickExhausted)
        );
    }
    let (d, mut s) = new(0, u64::MAX, 2, limits());
    d.admit_edge(edge(1)).unwrap();
    observe(&mut s, u64::MAX - 4);
    d.send_local(record(u64::MAX - 2, 0, &[])).unwrap();
    observe(&mut s, u64::MAX - 3);
    assert_eq!(d.check(), Err(Error::TickExhausted));
    assert_eq!(d.rolling_progress(), Some((u64::MAX - 4, 0)));
    assert_eq!(d.pending(), (1, 70));
    assert_eq!(
        d.flush(|_, _| panic!("terminal")),
        Err(Error::TickExhausted)
    );
    for tick in [u64::MAX - 1, u64::MAX] {
        let (d, mut s) = new(0, 1, 2, limits());
        d.admit_edge(edge(1)).unwrap();
        observe(&mut s, u64::MAX - 4);
        assert_eq!(d.send_local(record(tick, 0, &[])), Err(Error::TickRange));
    }
}

#[test]
fn stalled_session_rejects_send_before_its_cursor_overflows() {
    // Validated relay loader provides a high-tick arithmetic fixture for the
    // shared Session::advance pre-increment path, not a supported mesh workflow.
    let tick = u64::MAX - 3;
    let mut sim = Simulation::<Arena>::new(game(), 60, 42);
    sim.frame_mut().set_tick(tick);
    let (d, mut s) = new(0, 1, 1, limits());
    d.admit_edge(edge(1)).unwrap();
    observe(&mut s, tick);
    let mut c = cfg(0);
    c.relay = true;
    let mut session = Session::<Arena, _>::from_relay_snapshot(
        game(),
        c,
        s,
        tick,
        sim.frame().checksum(),
        &sim.frame().to_bytes(),
    )
    .unwrap_or_else(|(e, _)| panic!("fixture: {e}"));
    session.advance(ArenaInput::default(), vec![]);
    d.check().unwrap();
    assert_eq!(session.next_send_tick(), u64::MAX - 1);
    session.advance(ArenaInput::default(), vec![]);
    assert_eq!(session.next_send_tick(), u64::MAX);
    assert_eq!(d.check(), Err(Error::TickRange));
}

fn scripted_input(tick: u64, slot: u8) -> ArenaInput {
    ArenaInput::new(
        FP::from_int((tick % 3) as i32 - 1),
        FP::from_int(i32::from(slot) - 1),
        false,
    )
}
fn scripted_commands(tick: u64, slot: u8) -> Vec<SpawnBulletCmd> {
    if tick % 23 == 0 {
        [slot, (slot + 1) % 3, slot]
            .map(|owner| SpawnBulletCmd {
                owner: u32::from(owner),
            })
            .to_vec()
    } else {
        vec![]
    }
}
fn pump(from: &Driver, peers: &[&Driver], local: u8) {
    from.flush(|id, bytes| {
        let target = peers[(id.0 - 10) as usize];
        target
            .receive(edge(local).connection, Channel::Reliable, bytes)
            .unwrap();
        Ok(())
    })
    .unwrap();
}
#[test]
fn long_prejoin_and_live_sessions_verify_thousands_of_ticks_with_bounded_driver_ledgers() {
    const PRELUDE: u64 = 640;
    const END: u64 = 1800;
    let (a, sa) = new(0, 3, 16, limits());
    let (b, sb) = new(1, 3, 16, limits());
    a.admit_edge(edge(1)).unwrap();
    b.admit_edge(edge(0)).unwrap();
    let mut ca = cfg(0);
    ca.input_delay = 2;
    ca.vacant_slots = vec![PlayerSlot(2)];
    let mut cb = cfg(1);
    cb.input_delay = 2;
    let mut pa = Session::<Arena, _>::new(game(), ca, sa);
    let mut pb = Session::<Arena, _>::new(game(), cb, sb);
    let mut reference = Simulation::<Arena>::new(game(), 60, 42);
    // This test, like the example, bounds source bookkeeping only. Session
    // checksums and this explicit reference table are deliberately not drained.
    let mut sums = std::collections::BTreeMap::new();
    for tick in 1..=END + 2 {
        let mut inputs = TickInputs::new(tick, 3);
        for slot in 0..3 {
            if tick > 2 && (slot != 2 || tick >= PRELUDE + 3) {
                inputs.set_input(PlayerSlot(slot), scripted_input(tick, slot));
                for command in scripted_commands(tick, slot) {
                    inputs.push_command(PlayerSlot(slot), command);
                }
            }
        }
        reference.step(&inputs);
        sums.insert(tick, reference.frame().checksum());
    }
    for tick in 1..=PRELUDE {
        for (slot, session) in [(0, &mut pa), (1, &mut pb)] {
            let authored = session.next_send_tick();
            session.advance(
                scripted_input(authored, slot),
                scripted_commands(authored, slot),
            );
        }
        pump(&a, &[&a, &b], 0);
        pump(&b, &[&a, &b], 1);
        pa.poll_confirmed();
        pb.poll_confirmed();
        for (session, d) in [(&pa, &a), (&pb, &b)] {
            d.check().unwrap();
            assert_eq!(session.verified_tick(), tick);
            assert_eq!(session.verified_frame().unwrap().checksum(), sums[&tick]);
            assert!(d.retained_evidence().0 <= 15);
            assert!(d.destination_records() <= 10);
            assert_eq!(d.pending(), (0, 0));
        }
    }
    let mut cc = cfg(2);
    cc.input_delay = 2;
    let mut boot = Bootstrap::new(cc, roster(), 4096, 1 << 20).unwrap();
    let request = boot.next_request().unwrap();
    let (snapshot, ticket) = serve_checked_join(&mut pa, &context(), &request).unwrap();
    assert_eq!(
        (
            ticket.ticket().snapshot_tick,
            ticket.ticket().first_input_tick
        ),
        (PRELUDE, PRELUDE + 3)
    );
    a.install_ticket(&ticket).unwrap();
    b.install_ticket(&ticket).unwrap();
    let notices = [
        checked_backlog_notice(&mut pa, &context(), &ticket).unwrap(),
        checked_backlog_notice(&mut pb, &context(), &ticket).unwrap(),
    ];
    a.admit_edge(edge(2)).unwrap();
    b.admit_edge(edge(2)).unwrap();
    assert_eq!(a.edge_pending(edge(2).connection).unwrap().0, 4);
    assert_eq!(b.edge_pending(edge(2).connection).unwrap().0, 2);
    assert_eq!(a.edge_pending(edge(1).connection), Ok((0, 0)));
    assert_eq!(b.edge_pending(edge(0).connection), Ok((0, 0)));
    let (c, sc) = new(2, 3, 16, limits());
    boot.receive_snapshot(PlayerSlot(0), game(), sc, &snapshot)
        .unwrap();
    c.install_joiner_snapshot(&boot).unwrap();
    assert_eq!(c.rolling_progress(), Some((PRELUDE, PRELUDE)));
    c.admit_edge(edge(0)).unwrap();
    c.admit_edge(edge(1)).unwrap();
    for (slot, notice) in notices.iter().enumerate() {
        boot.receive_notice(PlayerSlot(slot as u8), notice).unwrap();
    }
    pump(&a, &[&a, &b, &c], 0);
    pump(&b, &[&a, &b, &c], 1);
    for tick in PRELUDE + 1..=END {
        for (slot, session) in [(0, &mut pa), (1, &mut pb)] {
            let authored = session.next_send_tick();
            session.advance(
                scripted_input(authored, slot),
                scripted_commands(authored, slot),
            );
        }
        let authored = boot.session().unwrap().next_send_tick();
        boot.advance(scripted_input(authored, 2), scripted_commands(authored, 2));
        for (slot, d) in [(0, &a), (1, &b), (2, &c)] {
            pump(d, &[&a, &b, &c], slot);
        }
        pa.poll_confirmed();
        pb.poll_confirmed();
        for (session, d) in [(&pa, &a), (&pb, &b), (boot.session().unwrap(), &c)] {
            d.check().unwrap();
            assert!(session.verified_tick() >= tick.saturating_sub(2));
            let verified = session.verified_tick();
            assert_eq!(
                session.verified_frame().unwrap().checksum(),
                sums[&verified],
                "tick {verified}"
            );
            assert!(d.retained_evidence().0 <= 21);
            assert!(d.retained_evidence().1 <= 21 * 59);
            assert!(d.destination_records() <= 20);
            assert!(d.incoming().0 <= 6);
            assert_eq!(d.pending(), (0, 0));
            assert!(d.rolling_progress().unwrap().1 >= tick.saturating_sub(5));
        }
    }
    assert!(boot.session().unwrap().verified_tick() >= END - 1);
    for d in [&a, &b, &c] {
        assert!(d.retained_evidence().0 < 64);
    }
}

#[test]
fn retirement_is_monotonic_and_old_local_replays_do_not_resurrect_obligations() {
    let (d, mut s) = two_edges(2, 8, limits());
    d.send_local(record(1, 0, &[0, 1, 0])).unwrap();
    d.flush(|_, _| Ok(())).unwrap();
    observe(&mut s, 4);
    assert_eq!(stats(&d), ((0, 0), (0, 0), 0, (0, 0)));
    for tick in [4, 3, 0] {
        observe(&mut s, tick);
        assert_eq!(d.rolling_progress(), Some((4, 2)));
    }
    d.send_local(record(1, 0, &[1, 0, 0])).unwrap();
    assert_eq!(stats(&d), ((0, 0), (0, 0), 0, (0, 0)));
    assert_eq!(
        d.flush(|_, _| panic!("retired output must not be requeued"))
            .unwrap()
            .sent,
        0
    );
}

#[test]
fn joiner_ignores_snapshot_covered_records_only_after_authority_and_vacancy_checks() {
    for (slot, tick, bad, expected) in [
        (0, 20, false, Ok(Accepted::IgnoredRetired)),
        (2, 20, false, Ok(Accepted::IgnoredRetired)),
        (2, 20, true, Err(Error::InvalidVacancy)),
        (2, 21, false, Err(Error::WrongAuthority)),
    ] {
        let (_, snapshot, mut boot) = grant(20, 21);
        let (d, s) = new(2, 1, 4, limits());
        boot.receive_snapshot(PlayerSlot(0), game(), s, &snapshot)
            .unwrap();
        d.install_joiner_snapshot(&boot).unwrap();
        d.admit_edge(edge(0)).unwrap();
        let mut bytes = packet(0, tick, slot, &[]);
        bytes[34] = 2;
        if bad {
            bytes[50] = 1;
        }
        DECODE_COUNT.with(|c| c.set(0));
        assert_eq!(
            d.receive(edge(0).connection, Channel::Reliable, &bytes),
            expected
        );
        DECODE_COUNT.with(|c| assert_eq!(c.get(), 0));
        assert_eq!(d.incoming(), (0, 0));
        assert_eq!(d.retained_evidence(), (0, 0));
    }
}
