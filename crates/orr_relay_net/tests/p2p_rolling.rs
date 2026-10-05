//! Rolling-mode regressions. The finite ORRI v1 wire contract is deliberately
//! reused; only locally verified progress or an accepted snapshot moves windows.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};

use orr_net::SendError;
use orr_proto::{Channel, ConnId};
use orr_relay_net::p2p_input::P2pRollingWindow;
use orr_relay_net::{
    encode_p2p_input, P2pInputAccepted as Accepted, P2pInputCodec, P2pInputDriver,
    P2pInputError as Error, P2pInputLimits, P2pInputSource, P2P_INPUT_VERSION,
};
use orr_session::{
    checked_backlog_notice, serve_checked_join, CheckedJoinContext, InputSource, JoinBootstrap,
    JoinBootstrapStatus, JoinRoster, LocallyVerifiedTick, PlayerSlot, RemoteInput, Session,
    SessionConfig,
};
use orr_sim::{Simulation, TickInputs, FP};
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd};

const CONN: ConnId = ConnId(23);
const HOST: PlayerSlot = PlayerSlot(0);
const JOINER: PlayerSlot = PlayerSlot(1);
const HEADER: usize = 41;
const INPUT_BYTES: usize = 20;
const RECORD_BYTES: usize = HEADER + INPUT_BYTES;
const COMMAND_RECORD_BYTES: usize = RECORD_BYTES + 3 * 8;

thread_local! {
    static DECODES: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}

struct PortableArena;
impl P2pInputCodec<Arena> for PortableArena {
    const SCHEMA: u64 = 0x0807_0605_0403_0201;

    fn encode_input(input: &ArenaInput, out: &mut Vec<u8>) {
        out.extend_from_slice(&input.axis_x.0.to_le_bytes());
        out.extend_from_slice(&input.axis_y.0.to_le_bytes());
        out.extend_from_slice(&input.buttons.to_le_bytes());
    }

    fn decode_input(bytes: &[u8]) -> Option<ArenaInput> {
        DECODES.with(|c| {
            let (inputs, commands) = c.get();
            c.set((inputs + 1, commands));
        });
        if bytes.len() != INPUT_BYTES {
            return None;
        }
        let x = i64::from_le_bytes(bytes[..8].try_into().ok()?);
        let y = i64::from_le_bytes(bytes[8..16].try_into().ok()?);
        let buttons = u32::from_le_bytes(bytes[16..].try_into().ok()?);
        if !(-65_536..=65_536).contains(&x) || !(-65_536..=65_536).contains(&y) || buttons > 1 {
            return None;
        }
        Some(ArenaInput {
            axis_x: FP::from_raw(x),
            axis_y: FP::from_raw(y),
            buttons,
            _pad: 0,
        })
    }

    fn encode_command(command: &SpawnBulletCmd, out: &mut Vec<u8>) {
        out.extend_from_slice(&command.owner.to_le_bytes());
    }

    fn decode_command(bytes: &[u8]) -> Option<SpawnBulletCmd> {
        DECODES.with(|c| {
            let (inputs, commands) = c.get();
            c.set((inputs, commands + 1));
        });
        let owner = u32::from_le_bytes(bytes.try_into().ok()?);
        (owner < 2).then_some(SpawnBulletCmd { owner })
    }
}

type Driver = P2pInputDriver<Arena, PortableArena>;
type Source = P2pInputSource<Arena, PortableArena>;
type Peer = Session<Arena, Source>;

fn context() -> CheckedJoinContext {
    CheckedJoinContext::new(
        0x1817_1615_1413_1211,
        0x2423_2221,
        JoinRoster::completed(2, JOINER, HOST, vec![HOST], &[]).unwrap(),
    )
    .unwrap()
}

fn limits() -> P2pInputLimits {
    P2pInputLimits {
        max_packet_bytes: 256,
        max_input_bytes: INPUT_BYTES,
        max_commands: 3,
        max_command_bytes: 4,
        max_records: 32,
        max_retained_bytes: 4096,
        ..P2pInputLimits::default()
    }
}

fn window(recent_ticks: u64, future_ticks: u64) -> P2pRollingWindow {
    P2pRollingWindow {
        recent_ticks,
        future_ticks,
    }
}

fn unbound(local: PlayerSlot, recent: u64, future: u64) -> (Driver, Source) {
    Driver::new_rolling(local, context(), limits(), window(recent, future)).unwrap()
}

fn bound(local: PlayerSlot, recent: u64, future: u64) -> (Driver, Source) {
    let (driver, source) = unbound(local, recent, future);
    driver.bind(CONN, &context(), 0, 1).unwrap();
    (driver, source)
}

fn record(tick: u64, slot: PlayerSlot, owners: &[u32]) -> RemoteInput<Arena> {
    RemoteInput {
        tick,
        slot,
        input: ArenaInput::default(),
        commands: owners
            .iter()
            .map(|&owner| SpawnBulletCmd { owner })
            .collect(),
        disconnected: false,
    }
}

fn packet(tick: u64, slot: PlayerSlot, owners: &[u32]) -> Vec<u8> {
    // Standalone encoding remains finite; widen only the test encoder's range.
    // Patch MAX after encoding because an exclusive finite end cannot contain it.
    let encoded_tick = tick.clamp(1, u64::MAX - 1);
    let mut bytes = encode_p2p_input::<Arena, PortableArena>(
        &context(),
        &record(encoded_tick, slot, owners),
        &P2pInputLimits {
            end_tick: u64::MAX,
            ..limits()
        },
    )
    .unwrap();
    bytes[27..35].copy_from_slice(&tick.to_le_bytes());
    bytes
}

fn reset_decodes() {
    DECODES.with(|c| c.set((0, 0)));
}

fn decodes() -> (usize, usize) {
    DECODES.with(Cell::get)
}

// Synthetic callback evidence is reserved for boundary/arithmetic tests. The
// Session tests below prove production verification actually invokes the hook.
fn observe(source: &mut Source, tick: u64) {
    let simulated = TickInputs::new(tick, 2);
    let confirmed_inputs = BTreeMap::from([
        (HOST, ArenaInput::default()),
        (JOINER, ArenaInput::default()),
    ]);
    let confirmed_absent = BTreeSet::new();
    source.on_locally_verified(LocallyVerifiedTick {
        simulated: &simulated,
        simulated_commands: &[],
        confirmed_inputs: &confirmed_inputs,
        confirmed_commands: None,
        confirmed_absent: &confirmed_absent,
    });
}

fn config(local: PlayerSlot) -> SessionConfig {
    let mut cfg = SessionConfig::new(2, local, 42, 60);
    cfg.input_delay = 0;
    cfg.max_prediction = 8;
    cfg.checksum_interval = 1;
    cfg.input_log_ticks = 8;
    cfg
}

fn peer(local: PlayerSlot, source: Source) -> Peer {
    Peer::new(ArenaConfig { player_count: 2 }, config(local), source)
}

fn assert_terminal(driver: &Driver, source: &mut Source, error: Error) {
    assert_eq!(driver.check(), Err(error.clone()));
    assert_eq!(driver.pending(), (0, 0));
    assert_eq!(driver.retained_evidence(), (0, 0));
    assert!(source.poll_remote().is_empty());
    assert_eq!(
        driver.flush(|_, _| panic!("terminal source must not transmit")),
        Err(error.clone())
    );
    observe(source, 99);
    source.send_local(99, HOST, ArenaInput::default(), vec![]);
    assert_eq!(driver.check(), Err(error.clone()));
    assert_eq!(
        driver.receive(CONN, Channel::Reliable, &packet(1, JOINER, &[])),
        Err(error)
    );
    assert_eq!(driver.retained_evidence(), (0, 0));
    assert!(source.poll_remote().is_empty());
}

#[test]
fn snapshot_and_local_verification_are_monotonic_retirement_sources() {
    let (driver, mut source) = unbound(JOINER, 4, 8);
    assert_eq!(driver.rolling_progress(), Some((0, 0)));
    driver.bind(CONN, &context(), 10, 11).unwrap();
    assert_eq!(driver.rolling_progress(), Some((10, 10)));
    for tick in [0, 8, 10, 11, 14] {
        observe(&mut source, tick);
        assert_eq!(driver.rolling_progress(), Some((tick.max(10), 10)));
    }
    observe(&mut source, 16);
    assert_eq!(driver.rolling_progress(), Some((16, 12)));
    for tick in [15, 8, 0] {
        observe(&mut source, tick);
        assert_eq!(driver.rolling_progress(), Some((16, 12)));
    }
    assert_eq!(driver.check(), Ok(()));
}

#[test]
fn future_packets_polling_and_prediction_cannot_advance_the_window() {
    let (driver, source) = bound(HOST, 2, 3);
    let mut session = peer(HOST, source);
    assert_eq!(
        driver.receive(CONN, Channel::Reliable, &packet(3, JOINER, &[])),
        Ok(Accepted::New)
    );
    session.poll_confirmed();
    for _ in 0..2 {
        session.advance(ArenaInput::default(), vec![]);
    }
    session.poll_confirmed();
    assert_eq!(session.head_tick(), 2);
    assert_eq!(session.verified_tick(), 0);
    assert_eq!(driver.rolling_progress(), Some((0, 0)));
    reset_decodes();
    assert_eq!(
        driver.receive(CONN, Channel::Reliable, &packet(4, JOINER, &[])),
        Err(Error::TickRange)
    );
    assert_eq!(decodes(), (0, 0));
    assert_eq!(driver.rolling_progress(), Some((0, 0)));
    assert_terminal(&driver, session.source_mut(), Error::TickRange);
}

#[test]
fn flushing_local_output_does_not_extend_a_stalled_verifiers_future_horizon() {
    let (driver, mut source) = bound(HOST, 1, 2);
    for tick in 1..=2 {
        source.send_local(tick, HOST, ArenaInput::default(), vec![]);
        assert_eq!(driver.flush(|_, _| Ok(())).unwrap().sent, 1);
    }
    assert_eq!(driver.rolling_progress(), Some((0, 0)));
    assert_eq!(driver.retained_evidence(), (2, 2 * RECORD_BYTES));
    source.send_local(3, HOST, ArenaInput::default(), vec![]);
    assert_terminal(&driver, &mut source, Error::TickRange);
}

#[test]
fn snapshot_seeds_future_horizon_but_received_max_tick_does_not_slide_it() {
    let (driver, mut source) = unbound(JOINER, 1, 2);
    driver.bind(CONN, &context(), 100, 101).unwrap();
    assert_eq!(
        driver.receive(CONN, Channel::Reliable, &packet(102, HOST, &[])),
        Ok(Accepted::New)
    );
    assert_eq!(source.poll_remote().len(), 1);
    assert_eq!(driver.rolling_progress(), Some((100, 100)));
    reset_decodes();
    assert_eq!(
        driver.receive(CONN, Channel::Reliable, &packet(103, HOST, &[])),
        Err(Error::TickRange)
    );
    assert_eq!(decodes(), (0, 0));
    assert_terminal(&driver, &mut source, Error::TickRange);
}

#[test]
fn recent_duplicates_keep_exact_ordered_evidence_and_conflicts_fail_after_drain() {
    for changed in [packet(4, JOINER, &[1, 0, 1]), packet(4, JOINER, &[0, 1]), {
        let mut bytes = packet(4, JOINER, &[0, 1, 0]);
        bytes[HEADER + 16] = 1;
        bytes
    }] {
        let (driver, mut source) = bound(HOST, 1, 8);
        let original = packet(4, JOINER, &[0, 1, 0]);
        assert_eq!(
            driver.receive(CONN, Channel::Reliable, &original),
            Ok(Accepted::New)
        );
        let received = source.poll_remote();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].commands, record(4, JOINER, &[0, 1, 0]).commands);
        observe(&mut source, 4);
        assert_eq!(driver.rolling_progress(), Some((4, 3)));
        assert_eq!(driver.retained_evidence(), (1, original.len()));
        reset_decodes();
        for _ in 0..3 {
            assert_eq!(
                driver.receive(CONN, Channel::Reliable, &original),
                Ok(Accepted::Duplicate)
            );
            assert!(source.poll_remote().is_empty());
        }
        let error = Error::ConflictingDuplicate {
            tick: 4,
            slot: JOINER,
        };
        assert_eq!(
            driver.receive(CONN, Channel::Reliable, &changed),
            Err(error.clone())
        );
        assert_eq!(decodes(), (0, 0));
        assert_terminal(&driver, &mut source, error);
    }
}

#[test]
fn retired_authorized_frames_are_ignored_before_application_decode_without_resurrection() {
    let (driver, mut source) = bound(HOST, 1, 8);
    let original = packet(1, JOINER, &[0, 1, 0]);
    driver.receive(CONN, Channel::Reliable, &original).unwrap();
    assert_eq!(source.poll_remote().len(), 1);
    observe(&mut source, 4);
    assert_eq!(driver.rolling_progress(), Some((4, 3)));
    assert_eq!(driver.retained_evidence(), (0, 0));

    let mut invalid_input = original.clone();
    invalid_input[HEADER + 16..HEADER + 20].copy_from_slice(&2_u32.to_le_bytes());
    let mut invalid_command = original.clone();
    invalid_command[RECORD_BYTES + 4..RECORD_BYTES + 8].copy_from_slice(&99_u32.to_le_bytes());
    reset_decodes();
    for bytes in [
        original,
        packet(1, JOINER, &[1, 0]),
        invalid_input,
        invalid_command,
        packet(2, JOINER, &[1]),
    ] {
        assert_eq!(
            driver.receive(CONN, Channel::Reliable, &bytes),
            Ok(Accepted::IgnoredRetired)
        );
        assert!(source.poll_remote().is_empty());
        assert_eq!(driver.retained_evidence(), (0, 0));
        assert_eq!(driver.pending(), (0, 0));
        assert_eq!(driver.rolling_progress(), Some((4, 3)));
    }
    assert_eq!(decodes(), (0, 0));
    // Retired local retries likewise must neither resurrect evidence nor queue.
    source.send_local(
        1,
        HOST,
        ArenaInput::default(),
        vec![SpawnBulletCmd { owner: 1 }],
    );
    assert_eq!(driver.pending(), (0, 0));
    assert_eq!(driver.retained_evidence(), (0, 0));
    assert_eq!(driver.check(), Ok(()));
    assert_eq!(P2P_INPUT_VERSION, 1);
}

fn retired_rejected(bytes: &[u8], conn: ConnId, channel: Channel, expected: Error) {
    let (driver, mut source) = unbound(HOST, 2, 8);
    driver.bind(CONN, &context(), 0, 3).unwrap();
    observe(&mut source, 6);
    assert_eq!(driver.rolling_progress(), Some((6, 4)));
    driver
        .receive(CONN, Channel::Reliable, &packet(6, JOINER, &[]))
        .unwrap();
    source.send_local(6, HOST, ArenaInput::default(), vec![]);
    reset_decodes();
    assert_eq!(driver.receive(conn, channel, bytes), Err(expected.clone()));
    assert_eq!(decodes(), (0, 0));
    assert_terminal(&driver, &mut source, expected);
}

#[test]
fn retired_frames_still_require_complete_bounded_framing() {
    let bytes = packet(3, JOINER, &[0, 1, 0]);
    for end in 0..bytes.len() {
        retired_rejected(&bytes[..end], CONN, Channel::Reliable, Error::Malformed);
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    retired_rejected(&trailing, CONN, Channel::Reliable, Error::Malformed);
    for (range, replacement, error) in [
        (35..39, u32::MAX.to_le_bytes().to_vec(), Error::InputLimit),
        (39..41, u16::MAX.to_le_bytes().to_vec(), Error::CommandLimit),
        (
            RECORD_BYTES..RECORD_BYTES + 4,
            u32::MAX.to_le_bytes().to_vec(),
            Error::CommandLimit,
        ),
    ] {
        let mut malformed = bytes.clone();
        malformed[range].copy_from_slice(&replacement);
        retired_rejected(&malformed, CONN, Channel::Reliable, error);
    }
    retired_rejected(
        &vec![0; limits().max_packet_bytes + 1],
        CONN,
        Channel::Reliable,
        Error::PacketLimit,
    );
}

#[test]
fn retired_frames_still_require_identity_channel_context_and_cutoff_authority() {
    let bytes = packet(3, JOINER, &[]);
    retired_rejected(
        &bytes,
        ConnId(CONN.0 + 1),
        Channel::Reliable,
        Error::WrongConnection,
    );
    retired_rejected(&bytes, CONN, Channel::Unreliable, Error::WrongChannel);
    for (offset, expected) in [
        (0, Error::Malformed),
        (4, Error::Malformed),
        (6, Error::WrongSchema),
        (14, Error::WrongContext),
        (22, Error::WrongContext),
    ] {
        let mut wrong = bytes.clone();
        wrong[offset] ^= 0x80;
        retired_rejected(&wrong, CONN, Channel::Reliable, expected);
    }
    for slot in [0, 2, 255] {
        let mut wrong = bytes.clone();
        wrong[26] = slot;
        retired_rejected(&wrong, CONN, Channel::Reliable, Error::WrongAuthority);
    }
    retired_rejected(
        &packet(2, JOINER, &[]),
        CONN,
        Channel::Reliable,
        Error::WrongAuthority,
    );
    retired_rejected(
        &packet(0, JOINER, &[]),
        CONN,
        Channel::Reliable,
        Error::TickRange,
    );
}

#[test]
fn snapshot_covered_host_backlog_is_ignored_only_with_original_slot_authority() {
    for slot in [HOST, JOINER] {
        let (driver, mut source) = unbound(JOINER, 1, 4);
        driver.bind(CONN, &context(), 10, 11).unwrap();
        reset_decodes();
        assert_eq!(
            driver.receive(CONN, Channel::Reliable, &packet(10, slot, &[0])),
            Ok(Accepted::IgnoredRetired)
        );
        assert_eq!(decodes(), (0, 0));
        assert_eq!(driver.retained_evidence(), (0, 0));
        assert!(source.poll_remote().is_empty());
    }
    let (driver, mut source) = unbound(JOINER, 1, 4);
    driver.bind(CONN, &context(), 10, 11).unwrap();
    observe(&mut source, 13);
    assert_eq!(
        driver.receive(CONN, Channel::Reliable, &packet(11, JOINER, &[])),
        Err(Error::WrongAuthority)
    );
    assert_terminal(&driver, &mut source, Error::WrongAuthority);
}

#[test]
fn unsent_postbind_fifo_survives_actual_verification_pruning_and_delivers_once() {
    let (driver, source) = bound(HOST, 1, 8);
    let mut session = peer(HOST, source);
    let expected: Vec<_> = (1..=6).map(|tick| packet(tick, HOST, &[0, 1, 0])).collect();
    let mut attempted = Vec::new();
    for tick in 1..=6 {
        driver
            .receive(CONN, Channel::Reliable, &packet(tick, JOINER, &[]))
            .unwrap();
        session.advance(
            ArenaInput::default(),
            record(tick, HOST, &[0, 1, 0]).commands,
        );
        if tick == 1 {
            let blocked = driver
                .flush(|_, bytes| {
                    attempted.push(bytes.to_vec());
                    Err(SendError::Backpressure)
                })
                .unwrap();
            assert_eq!(
                (blocked.sent, blocked.pending, blocked.backpressured),
                (0, 1, true)
            );
        }
        session.poll_confirmed();
        assert_eq!(session.verified_tick(), tick);
        assert_eq!(
            driver.rolling_progress(),
            Some((tick, tick.saturating_sub(1)))
        );
        assert!(driver.retained_evidence().0 <= 2);
        assert_eq!(
            driver.pending(),
            (tick as usize, tick as usize * COMMAND_RECORD_BYTES)
        );
        driver.check().unwrap();
    }
    assert_eq!(attempted, expected[..1]);
    let (receiver, mut receiver_source) = bound(JOINER, 1, 8);
    let mut delivered = Vec::new();
    let partial = driver
        .flush(|conn, bytes| {
            assert_eq!(conn, CONN);
            attempted.push(bytes.to_vec());
            if delivered.len() == 1 {
                return Err(SendError::Backpressure);
            }
            assert_eq!(
                receiver.receive(CONN, Channel::Reliable, bytes),
                Ok(Accepted::New)
            );
            delivered.push(bytes.to_vec());
            Ok(())
        })
        .unwrap();
    assert_eq!(
        (partial.sent, partial.pending, partial.backpressured),
        (1, 5, true)
    );
    assert_eq!(
        attempted,
        vec![
            expected[0].clone(),
            expected[0].clone(),
            expected[1].clone()
        ]
    );
    let flushed = driver
        .flush(|conn, bytes| {
            assert_eq!(conn, CONN);
            assert_eq!(
                receiver.receive(CONN, Channel::Reliable, bytes),
                Ok(Accepted::New)
            );
            delivered.push(bytes.to_vec());
            Ok(())
        })
        .unwrap();
    assert_eq!(
        (flushed.sent, flushed.pending, flushed.backpressured),
        (5, 0, false)
    );
    assert_eq!(delivered, expected);
    let received = receiver_source.poll_remote();
    assert_eq!(
        received.iter().map(|r| r.tick).collect::<Vec<_>>(),
        (1..=6).collect::<Vec<_>>()
    );
    for r in received {
        assert_eq!(r.slot, HOST);
        assert_eq!(r.commands, record(r.tick, HOST, &[0, 1, 0]).commands);
    }
    assert!(receiver_source.poll_remote().is_empty());
    assert_eq!(driver.flush(|_, _| panic!("already sent")).unwrap().sent, 0);
    session.source_mut().send_local(
        1,
        HOST,
        ArenaInput::default(),
        vec![SpawnBulletCmd { owner: 1 }],
    );
    assert_eq!(driver.pending(), (0, 0));
    assert_eq!(driver.check(), Ok(()));
}

#[test]
fn prebind_verified_output_can_be_discarded_only_for_a_fresh_enough_snapshot() {
    const SNAPSHOT: u64 = 4300;
    for stale in [false, true] {
        let (driver, source) = unbound(HOST, 2, 4);
        let mut cfg = config(HOST);
        cfg.vacant_slots.push(JOINER);
        let mut session = Peer::new(ArenaConfig { player_count: 2 }, cfg, source);
        for tick in 1..=SNAPSHOT {
            session.advance(ArenaInput::default(), vec![]);
            session.poll_confirmed();
            assert_eq!(session.verified_tick(), tick);
            assert_eq!(driver.pending(), (0, 0));
            assert!(driver.retained_evidence().0 <= 4);
            driver.check().unwrap();
        }
        assert_eq!(driver.rolling_progress(), Some((SNAPSHOT, SNAPSHOT - 2)));
        session
            .source_mut()
            .send_local(SNAPSHOT + 1, HOST, ArenaInput::default(), vec![]);
        assert_eq!(driver.pending(), (1, RECORD_BYTES));
        let snapshot = if stale { SNAPSHOT - 1 } else { SNAPSHOT };
        if stale {
            assert_eq!(
                driver.bind(CONN, &context(), snapshot, SNAPSHOT + 1),
                Err(Error::SnapshotTooOld)
            );
            assert_terminal(&driver, session.source_mut(), Error::SnapshotTooOld);
        } else {
            driver
                .bind(CONN, &context(), snapshot, SNAPSHOT + 1)
                .unwrap();
            assert_eq!(driver.rolling_progress(), Some((SNAPSHOT, SNAPSHOT)));
            assert_eq!(driver.retained_evidence(), (1, RECORD_BYTES));
            assert_eq!(driver.pending(), (1, RECORD_BYTES));
            let mut sent = Vec::new();
            driver
                .flush(|_, bytes| {
                    sent.push(bytes.to_vec());
                    Ok(())
                })
                .unwrap();
            assert_eq!(sent, vec![packet(SNAPSHOT + 1, HOST, &[])]);
        }
    }
}

#[test]
fn stalled_verification_cannot_reclaim_record_or_byte_budgets_by_polling() {
    for (configured, count, expected) in [
        (
            P2pInputLimits {
                max_records: 3,
                ..limits()
            },
            3_u64,
            Error::RecordLimit,
        ),
        (
            P2pInputLimits {
                max_packet_bytes: COMMAND_RECORD_BYTES,
                max_retained_bytes: 2 * COMMAND_RECORD_BYTES,
                ..limits()
            },
            2,
            Error::ByteLimit,
        ),
    ] {
        let (driver, mut source) =
            Driver::new_rolling(HOST, context(), configured, window(1, 32)).unwrap();
        driver.bind(CONN, &context(), 0, 1).unwrap();
        for tick in 1..=count {
            driver
                .receive(CONN, Channel::Reliable, &packet(tick, JOINER, &[0, 1, 0]))
                .unwrap();
            assert_eq!(source.poll_remote().len(), 1);
        }
        assert_eq!(
            driver.retained_evidence(),
            (count as usize, count as usize * COMMAND_RECORD_BYTES)
        );
        assert_eq!(driver.rolling_progress(), Some((0, 0)));
        reset_decodes();
        assert_eq!(
            driver.receive(
                CONN,
                Channel::Reliable,
                &packet(count + 1, JOINER, &[0, 1, 0])
            ),
            Err(expected.clone())
        );
        assert_eq!(decodes(), (0, 0));
        assert_terminal(&driver, &mut source, expected);
    }
}

#[test]
fn queue_limits_remain_independent_of_pruned_duplicate_evidence() {
    for outgoing in [false, true] {
        for (configured, expected) in [
            (
                P2pInputLimits {
                    max_records: 2,
                    ..limits()
                },
                Error::RecordLimit,
            ),
            (
                P2pInputLimits {
                    max_packet_bytes: RECORD_BYTES,
                    max_retained_bytes: 2 * RECORD_BYTES,
                    ..limits()
                },
                Error::ByteLimit,
            ),
        ] {
            let (driver, mut source) =
                Driver::new_rolling(HOST, context(), configured, window(1, 4)).unwrap();
            driver.bind(CONN, &context(), 0, 1).unwrap();
            for tick in 1..=2 {
                if outgoing {
                    source.send_local(tick, HOST, ArenaInput::default(), vec![]);
                } else {
                    driver
                        .receive(CONN, Channel::Reliable, &packet(tick, JOINER, &[]))
                        .unwrap();
                }
                observe(&mut source, tick + 1);
                assert_eq!(driver.retained_evidence(), (0, 0));
            }
            if outgoing {
                assert_eq!(driver.pending(), (2, 2 * RECORD_BYTES));
                source.send_local(3, HOST, ArenaInput::default(), vec![]);
            } else {
                assert_eq!(
                    driver.receive(CONN, Channel::Reliable, &packet(3, JOINER, &[])),
                    Err(expected.clone())
                );
            }
            assert_terminal(&driver, &mut source, expected);
        }
    }
}

#[test]
fn cancel_after_retirement_clears_all_queues_without_resurrecting_progress() {
    let (driver, mut source) = bound(HOST, 1, 4);
    source.send_local(1, HOST, ArenaInput::default(), vec![]);
    driver
        .receive(CONN, Channel::Reliable, &packet(1, JOINER, &[]))
        .unwrap();
    observe(&mut source, 3);
    assert_eq!(driver.retained_evidence(), (0, 0));
    assert_eq!(driver.pending(), (1, RECORD_BYTES));
    driver.cancel();
    driver.cancel();
    assert_terminal(&driver, &mut source, Error::Cancelled);
    assert_eq!(driver.rolling_progress(), Some((3, 2)));
}

#[test]
fn verification_inside_flush_callback_never_removes_the_inflight_fifo_head() {
    let (driver, mut source) = bound(HOST, 1, 4);
    for tick in 1..=3 {
        source.send_local(tick, HOST, ArenaInput::default(), vec![]);
    }
    let mut delivered = Vec::new();
    let result = driver
        .flush(|_, bytes| {
            observe(&mut source, 4);
            assert_eq!(driver.retained_evidence(), (0, 0));
            delivered.push(bytes.to_vec());
            Ok(())
        })
        .unwrap();
    assert_eq!((result.sent, result.pending), (3, 0));
    assert_eq!(
        delivered,
        (1..=3)
            .map(|tick| packet(tick, HOST, &[]))
            .collect::<Vec<_>>()
    );
    assert_eq!(driver.check(), Ok(()));
}

#[test]
fn rolling_configuration_tiny_windows_and_checked_tick_arithmetic() {
    for cfg in [window(0, 1), window(1, 0)] {
        assert_eq!(
            Driver::new_rolling(HOST, context(), limits(), cfg).err(),
            Some(Error::InvalidConfig)
        );
    }
    let (driver, mut source) = Driver::new_rolling(
        HOST,
        context(),
        P2pInputLimits {
            first_tick: 9,
            end_tick: 3,
            ..limits()
        },
        window(1, 1),
    )
    .unwrap();
    driver.bind(CONN, &context(), 0, 1).unwrap();
    for tick in 1..=16 {
        driver
            .receive(CONN, Channel::Reliable, &packet(tick, JOINER, &[]))
            .unwrap();
        assert_eq!(source.poll_remote().len(), 1);
        observe(&mut source, tick);
        assert_eq!(driver.retained_evidence(), (1, RECORD_BYTES));
        assert_eq!(driver.rolling_progress(), Some((tick, tick - 1)));
    }
    assert_eq!(driver.check(), Ok(()));

    let (driver, mut source) = unbound(JOINER, u64::MAX, 2);
    driver
        .bind(CONN, &context(), u64::MAX - 4, u64::MAX - 3)
        .unwrap();
    assert_eq!(
        driver.receive(CONN, Channel::Reliable, &packet(u64::MAX - 2, HOST, &[])),
        Ok(Accepted::New)
    );
    assert_eq!(source.poll_remote()[0].tick, u64::MAX - 2);
    observe(&mut source, u64::MAX - 3);
    assert_terminal(&driver, &mut source, Error::TickExhausted);

    for tick in [u64::MAX - 1, u64::MAX] {
        let (driver, mut source) = unbound(JOINER, 1, 2);
        driver
            .bind(CONN, &context(), u64::MAX - 4, u64::MAX - 3)
            .unwrap();
        assert_eq!(
            driver.receive(CONN, Channel::Reliable, &packet(tick, HOST, &[])),
            Err(Error::TickRange)
        );
        assert_terminal(&driver, &mut source, Error::TickRange);
    }

    for (snapshot, first, future) in [
        (u64::MAX - 3, u64::MAX - 2, 2),
        (u64::MAX - 2, u64::MAX - 1, 1),
        (u64::MAX - 1, u64::MAX, 1),
    ] {
        let (driver, mut source) = unbound(JOINER, 1, future);
        assert_eq!(
            driver.bind(CONN, &context(), snapshot, first),
            Err(Error::TickExhausted)
        );
        assert_terminal(&driver, &mut source, Error::TickExhausted);
    }
    for future in [u64::MAX - 1, u64::MAX] {
        assert_eq!(
            Driver::new_rolling(HOST, context(), limits(), window(1, future)).err(),
            Some(Error::InvalidConfig)
        );
    }
}

fn scripted_input(tick: u64, slot: PlayerSlot) -> ArenaInput {
    ArenaInput::new(
        FP::from_int((tick % 3) as i32 - 1),
        FP::from_int(i32::from(slot.0) * 2 - 1),
        false,
    )
}

fn scripted_commands(tick: u64, slot: PlayerSlot) -> Vec<SpawnBulletCmd> {
    if tick % 17 == 0 {
        [slot.0, 1 - slot.0, slot.0]
            .map(|owner| SpawnBulletCmd {
                owner: u32::from(owner),
            })
            .to_vec()
    } else {
        vec![]
    }
}

#[test]
fn two_peers_verify_beyond_4096_ticks_with_ordered_commands_and_bounded_ledgers() {
    const TICKS: u64 = 4300;
    const RECENT: u64 = 3;
    let (host_driver, host_source) = bound(HOST, RECENT, 4);
    let (join_driver, join_source) = bound(JOINER, RECENT, 4);
    let mut host = peer(HOST, host_source);
    let mut joiner = peer(JOINER, join_source);
    let mut reference = Simulation::<Arena>::new(ArenaConfig { player_count: 2 }, 60, 42);
    let mut ordered_command_ticks = 0;
    for tick in 1..=TICKS {
        let mut inputs = TickInputs::new(tick, 2);
        for slot in [HOST, JOINER] {
            inputs.set_input(slot, scripted_input(tick, slot));
            for command in scripted_commands(tick, slot) {
                inputs.push_command(slot, command);
            }
        }
        reference.step(&inputs);
        host.advance(scripted_input(tick, HOST), scripted_commands(tick, HOST));
        joiner.advance(
            scripted_input(tick, JOINER),
            scripted_commands(tick, JOINER),
        );
        host_driver.check().unwrap();
        join_driver.check().unwrap();
        for (sender, receiver, slot) in [
            (&host_driver, &join_driver, HOST),
            (&join_driver, &host_driver, JOINER),
        ] {
            let mut sent = 0;
            sender
                .flush(|conn, bytes| {
                    assert_eq!(conn, CONN);
                    let expected = encode_p2p_input::<Arena, PortableArena>(
                        &context(),
                        &RemoteInput {
                            tick,
                            slot,
                            input: scripted_input(tick, slot),
                            commands: scripted_commands(tick, slot),
                            disconnected: false,
                        },
                        &P2pInputLimits {
                            end_tick: TICKS + 1,
                            ..limits()
                        },
                    )
                    .unwrap();
                    assert_eq!(bytes, expected, "canonical command order at tick {tick}");
                    assert_eq!(
                        receiver.receive(conn, Channel::Reliable, bytes),
                        Ok(Accepted::New)
                    );
                    assert_eq!(
                        receiver.receive(conn, Channel::Reliable, bytes),
                        Ok(Accepted::Duplicate)
                    );
                    sent += 1;
                    Ok(())
                })
                .unwrap();
            assert_eq!(sent, 1);
        }
        if tick % 17 == 0 {
            ordered_command_ticks += 1;
        }
        host.poll_confirmed();
        joiner.poll_confirmed();
        for (session, driver) in [(&host, &host_driver), (&joiner, &join_driver)] {
            driver.check().unwrap();
            assert_eq!(session.verified_tick(), tick);
            assert_eq!(
                session.verified_frame().unwrap().checksum(),
                reference.frame().checksum(),
                "verified checksum at tick {tick}"
            );
            assert_eq!(
                driver.rolling_progress(),
                Some((tick, tick.saturating_sub(RECENT)))
            );
            let (records, bytes) = driver.retained_evidence();
            assert_eq!(records, 2 * tick.min(RECENT) as usize);
            assert!(bytes <= 2 * RECENT as usize * COMMAND_RECORD_BYTES);
            assert_eq!(driver.pending(), (0, 0));
        }
    }
    assert!(ordered_command_ticks > 250);
    assert_eq!(host.checksums(), joiner.checksums());
    assert_eq!(host.checksums().len(), TICKS as usize);
}

#[test]
fn checked_late_join_after_4300_ticks_delivers_snapshot_tail_and_verifies_live_inputs() {
    const SNAPSHOT_TICK: u64 = 4300;
    let ctx =
        CheckedJoinContext::new(0x4348_4543_4b45_4431, 1, context().roster().clone()).unwrap();
    let (host_driver, host_source) =
        Driver::new_rolling(HOST, ctx.clone(), limits(), window(3, 8)).unwrap();
    let mut host_config = config(HOST);
    host_config.input_delay = 2;
    host_config.vacant_slots.push(JOINER);
    let mut host = Peer::new(ArenaConfig { player_count: 2 }, host_config, host_source);
    for tick in 1..=SNAPSHOT_TICK {
        let authored = host.next_send_tick();
        host.advance(
            scripted_input(authored, HOST),
            scripted_commands(authored, HOST),
        );
        host.poll_confirmed();
        assert_eq!(host.verified_tick(), tick);
        assert_eq!(
            host_driver.rolling_progress(),
            Some((tick, tick.saturating_sub(3)))
        );
        assert!(host_driver.retained_evidence().0 <= 10);
        assert_eq!(
            host_driver.pending().0,
            2 * tick.min(2) as usize,
            "exact two-tick, two-slot unsnapshotted tail"
        );
        host_driver.check().unwrap();
    }
    let mut join_config = config(JOINER);
    join_config.input_delay = 2;
    join_config.join_id = ctx.join_id();
    let mut bootstrap =
        JoinBootstrap::<Arena, Source>::new(join_config, ctx.roster().clone(), 4096, 1 << 20)
            .unwrap();
    let request = bootstrap.next_request().unwrap();
    assert_eq!(bootstrap.context(), Some(ctx.clone()));
    let (snapshot, ticket) = serve_checked_join(&mut host, &ctx, &request).unwrap();
    let raw = ticket.ticket();
    assert_eq!(raw.snapshot_tick, SNAPSHOT_TICK);
    assert_eq!(raw.first_input_tick, SNAPSHOT_TICK + 3);
    host_driver
        .bind(
            CONN,
            ticket.context(),
            raw.snapshot_tick,
            raw.first_input_tick,
        )
        .unwrap();
    let notice = checked_backlog_notice(&mut host, &ctx, &ticket).unwrap();
    let (join_driver, join_source) =
        Driver::new_rolling(JOINER, ctx.clone(), limits(), window(3, 8)).unwrap();
    // Control messages may arrive in either order. Readiness is still coverage,
    // and not proof that the driver's retained post-snapshot tail was delivered.
    bootstrap.receive_notice(HOST, &notice).unwrap();
    assert_eq!(
        bootstrap
            .receive_snapshot(
                HOST,
                ArenaConfig { player_count: 2 },
                join_source,
                &snapshot
            )
            .unwrap(),
        JoinBootstrapStatus::Ready
    );
    let accepted = bootstrap.session().unwrap();
    assert_eq!(accepted.verified_tick(), raw.snapshot_tick);
    assert_eq!(accepted.next_send_tick(), raw.first_input_tick);
    join_driver
        .bind(
            CONN,
            &ctx,
            accepted.verified_tick(),
            accepted.next_send_tick(),
        )
        .unwrap();
    assert!(!bootstrap.caught_up_to(raw.first_input_tick).unwrap());
    assert_eq!(host_driver.pending().0, 4);
    assert_eq!(host_driver.retained_evidence().0, 4);
    assert_eq!(
        join_driver.rolling_progress(),
        Some((SNAPSHOT_TICK, SNAPSHOT_TICK))
    );

    let mut tail = Vec::new();
    let flush = host_driver
        .flush(|conn, bytes| {
            let tick = u64::from_le_bytes(bytes[27..35].try_into().unwrap());
            let slot = PlayerSlot(bytes[26]);
            tail.push((tick, slot));
            assert_eq!(
                join_driver.receive(conn, Channel::Reliable, bytes),
                Ok(Accepted::New)
            );
            Ok(())
        })
        .unwrap();
    assert_eq!((flush.sent, flush.pending), (4, 0));
    assert_eq!(
        tail,
        vec![
            (SNAPSHOT_TICK + 1, HOST),
            (SNAPSHOT_TICK + 1, JOINER),
            (SNAPSHOT_TICK + 2, HOST),
            (SNAPSHOT_TICK + 2, JOINER),
        ]
    );
    bootstrap.poll_confirmed();
    assert_eq!(bootstrap.session().unwrap().verified_tick(), SNAPSHOT_TICK);
    assert!(!bootstrap.caught_up_to(raw.first_input_tick).unwrap());

    for tick in SNAPSHOT_TICK + 1..=SNAPSHOT_TICK + 40 {
        let host_send = host.next_send_tick();
        let join_send = bootstrap.session().unwrap().next_send_tick();
        assert_eq!(host_send, join_send);
        host.advance(
            scripted_input(host_send, HOST),
            scripted_commands(host_send, HOST),
        );
        bootstrap
            .advance(
                scripted_input(join_send, JOINER),
                scripted_commands(join_send, JOINER),
            )
            .unwrap();
        for (sender, receiver) in [(&host_driver, &join_driver), (&join_driver, &host_driver)] {
            let flushed = sender
                .flush(|conn, bytes| {
                    assert_eq!(
                        receiver.receive(conn, Channel::Reliable, bytes),
                        Ok(Accepted::New)
                    );
                    Ok(())
                })
                .unwrap();
            assert_eq!((flushed.sent, flushed.pending), (1, 0));
        }
        host.poll_confirmed();
        bootstrap.poll_confirmed();
        host_driver.check().unwrap();
        join_driver.check().unwrap();
        let joiner = bootstrap.session().unwrap();
        assert_eq!(host.verified_tick(), tick);
        assert_eq!(joiner.verified_tick(), tick);
        assert_eq!(
            host.verified_frame().unwrap().checksum(),
            joiner.verified_frame().unwrap().checksum(),
            "late-join checksum at tick {tick}"
        );
        for driver in [&host_driver, &join_driver] {
            assert!(driver.retained_evidence().0 <= 10);
            assert!(driver.retained_evidence().1 <= 10 * COMMAND_RECORD_BYTES);
        }
    }
    assert!(bootstrap.caught_up_to(SNAPSHOT_TICK + 40).unwrap());
    assert!(host.pending_join(JOINER).is_none());
}

#[test]
fn stalled_session_fails_before_its_local_send_counter_can_overflow() {
    // The public relay snapshot loader supplies a validated high-tick fixture
    // for Session::advance's shared pre-increment path. This does not establish
    // a supported P2P/relay workflow; actual P2P bootstrap is tested above.
    let snapshot_tick = u64::MAX - 3;
    let mut simulation = Simulation::<Arena>::new(ArenaConfig { player_count: 2 }, 60, 42);
    simulation.frame_mut().set_tick(snapshot_tick);
    let bytes = simulation.frame().to_bytes();
    let checksum = simulation.frame().checksum();
    let (driver, source) = unbound(JOINER, 1, 1);
    let mut cfg = config(JOINER);
    cfg.relay = true;
    let mut session = Peer::from_relay_snapshot(
        ArenaConfig { player_count: 2 },
        cfg,
        source,
        snapshot_tick,
        checksum,
        &bytes,
    )
    .unwrap_or_else(|(error, _)| panic!("arithmetic fixture snapshot rejected: {error}"));
    driver
        .bind(
            CONN,
            &context(),
            session.verified_tick(),
            session.next_send_tick(),
        )
        .unwrap();
    assert_eq!(session.next_send_tick(), u64::MAX - 2);
    session.advance(ArenaInput::default(), vec![]);
    driver.check().unwrap();
    assert_eq!(session.next_send_tick(), u64::MAX - 1);
    assert_eq!(session.verified_tick(), snapshot_tick);
    assert_eq!(driver.pending(), (1, RECORD_BYTES));
    // The source observes the out-of-range second send after Session increments
    // MAX-1 to MAX, still safely. The required check ends further advancement.
    session.advance(ArenaInput::default(), vec![]);
    assert_eq!(session.next_send_tick(), u64::MAX);
    assert_eq!(driver.check(), Err(Error::TickRange));
    assert_terminal(&driver, session.source_mut(), Error::TickRange);
}
