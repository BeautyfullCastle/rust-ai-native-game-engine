//! Adversarial checks at the admitted transport/application input boundary.
//! The codec is deliberately field-wise: ArenaInput's padding is never on wire.

use std::cell::Cell;

use orr_net::SendError;
use orr_proto::{Channel, ConnId};
use orr_relay_net::{
    encode_p2p_input, P2pInputAccepted as Accepted, P2pInputCodec, P2pInputDriver,
    P2pInputError as Error, P2pInputLimits, P2pInputSource, P2P_INPUT_VERSION,
};
use orr_session::{
    CheckedJoinContext, InputSource, JoinRoster, PlayerSlot, RemoteInput, Session, SessionConfig,
};
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd};

const CONN: ConnId = ConnId(23);
const HEADER: usize = 41;
const INPUT_BYTES: usize = 20;
const RECORD_BYTES: usize = HEADER + INPUT_BYTES;

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
            let (i, n) = c.get();
            c.set((i + 1, n));
        });
        if bytes.len() != INPUT_BYTES {
            return None;
        }
        let mut input = ArenaInput::default();
        input.axis_x.0 = i64::from_le_bytes(bytes[0..8].try_into().ok()?);
        input.axis_y.0 = i64::from_le_bytes(bytes[8..16].try_into().ok()?);
        input.buttons = u32::from_le_bytes(bytes[16..20].try_into().ok()?);
        if !(-65_536..=65_536).contains(&input.axis_x.0)
            || !(-65_536..=65_536).contains(&input.axis_y.0)
            || input.buttons > 1
        {
            return None;
        }
        Some(input)
    }

    fn encode_command(command: &SpawnBulletCmd, out: &mut Vec<u8>) {
        out.extend_from_slice(&command.owner.to_le_bytes());
    }

    fn decode_command(bytes: &[u8]) -> Option<SpawnBulletCmd> {
        DECODES.with(|c| {
            let (i, n) = c.get();
            c.set((i, n + 1));
        });
        let owner = u32::from_le_bytes(bytes.try_into().ok()?);
        (owner < 2).then_some(SpawnBulletCmd { owner })
    }
}

type Driver = P2pInputDriver<Arena, PortableArena>;
type Source = P2pInputSource<Arena, PortableArena>;

fn context() -> CheckedJoinContext {
    CheckedJoinContext::new(
        0x1817_1615_1413_1211,
        0x2423_2221,
        JoinRoster::completed(2, PlayerSlot(1), PlayerSlot(0), vec![PlayerSlot(0)], &[]).unwrap(),
    )
    .unwrap()
}

fn limits() -> P2pInputLimits {
    P2pInputLimits {
        end_tick: 100,
        ..P2pInputLimits::default()
    }
}

fn bound(local: u8, limits: P2pInputLimits) -> (Driver, Source) {
    let context = context();
    let (driver, source) = Driver::new(PlayerSlot(local), context.clone(), limits).unwrap();
    driver.bind(CONN, &context, 1, 3).unwrap();
    (driver, source)
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

fn packet(tick: u64, slot: u8, owners: &[u32]) -> Vec<u8> {
    encode_p2p_input::<Arena, PortableArena>(&context(), &record(tick, slot, owners), &limits())
        .unwrap()
}

fn reset_decodes() {
    DECODES.with(|c| c.set((0, 0)));
}
fn decodes() -> (usize, usize) {
    DECODES.with(Cell::get)
}

fn assert_terminal(driver: &Driver, source: &mut Source, error: Error) {
    assert_eq!(driver.check(), Err(error.clone()));
    assert_eq!(driver.pending(), (0, 0));
    assert!(source.poll_remote().is_empty());
    assert_eq!(
        driver.flush(|_, _| panic!("a failed source must not transmit")),
        Err(error.clone())
    );
    source.send_local(5, PlayerSlot(0), ArenaInput::default(), vec![]);
    assert_eq!(driver.check(), Err(error.clone()));
    assert_eq!(
        driver.receive(CONN, Channel::Reliable, &packet(5, 1, &[])),
        Err(error)
    );
    assert!(source.poll_remote().is_empty());
}

// A rejection also clears earlier accepted records and pending output, rather
// than leaving a partly accepted batch available to Session::poll_confirmed.
fn rejected(bytes: &[u8], conn: ConnId, channel: Channel, error: Error, calls: (usize, usize)) {
    let (driver, mut source) = bound(0, limits());
    assert_eq!(
        driver.receive(CONN, Channel::Reliable, &packet(3, 1, &[])),
        Ok(Accepted::New)
    );
    source.send_local(3, PlayerSlot(0), ArenaInput::default(), vec![]);
    assert_eq!(driver.pending().0, 1);
    reset_decodes();
    assert_eq!(driver.receive(conn, channel, bytes), Err(error.clone()));
    assert_eq!(decodes(), calls);
    assert_terminal(&driver, &mut source, error);
    assert_eq!(
        decodes(),
        calls,
        "terminal retries must not invoke application decoders"
    );
}

#[test]
fn portable_encoding_has_fixed_little_endian_golden_bytes_and_no_pod_padding() {
    let mut input = record(4, 1, &[1, 0, 1]);
    input.input.axis_x.0 = -65_536;
    input.input.axis_y.0 = 65_536;
    input.input.buttons = 1;
    input.input._pad = 0xaabb_ccdd;
    let bytes = encode_p2p_input::<Arena, PortableArena>(&context(), &input, &limits()).unwrap();
    let mut expected = vec![
        b'O', b'R', b'R', b'I', 1, 0, 1, 2, 3, 4, 5, 6, 7, 8, 17, 18, 19, 20, 21, 22, 23, 24, 33,
        34, 35, 36, 1, 4, 0, 0, 0, 0, 0, 0, 0, 20, 0, 0, 0, 3, 0, 0, 0, 255, 255, 255, 255, 255,
        255, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0,
    ];
    expected.extend_from_slice(&[4, 0, 0, 0, 1, 0, 0, 0]);
    expected.extend_from_slice(&[4, 0, 0, 0, 0, 0, 0, 0]);
    expected.extend_from_slice(&[4, 0, 0, 0, 1, 0, 0, 0]);
    assert_eq!(P2P_INPUT_VERSION, 1);
    assert_eq!(bytes, expected);
    input.input._pad = 0;
    assert_eq!(
        encode_p2p_input::<Arena, PortableArena>(&context(), &input, &limits()).unwrap(),
        bytes
    );

    let (driver, mut source) = bound(0, limits());
    reset_decodes();
    assert_eq!(
        driver.receive(CONN, Channel::Reliable, &bytes),
        Ok(Accepted::New)
    );
    let records = source.poll_remote();
    assert_eq!(records.len(), 1);
    assert_eq!((records[0].tick, records[0].slot), (4, PlayerSlot(1)));
    assert_eq!(records[0].input, input.input);
    assert_eq!(
        records[0].commands, input.commands,
        "ordered repeated commands must survive"
    );
    assert!(!records[0].disconnected);
    assert_eq!(decodes(), (1, 3));
    assert!(source.poll_remote().is_empty());
}

#[test]
fn magic_version_schema_generation_and_attempt_fail_before_application_decode() {
    for (offset, error) in [
        (0, Error::Malformed),
        (4, Error::Malformed),
        (6, Error::WrongSchema),
        (14, Error::WrongContext),
        (22, Error::WrongContext),
    ] {
        let mut bytes = packet(4, 1, &[1]);
        bytes[offset] ^= 0x80;
        rejected(&bytes, CONN, Channel::Reliable, error, (0, 0));
    }
}

#[test]
fn actual_connection_and_reliable_channel_are_checked_before_even_framing() {
    // Empty bytes also show that transport identity takes precedence over parsing.
    rejected(
        &[],
        ConnId(CONN.0 + 1),
        Channel::Reliable,
        Error::WrongConnection,
        (0, 0),
    );
    rejected(&[], CONN, Channel::Unreliable, Error::WrongChannel, (0, 0));
}

#[test]
fn joiner_cannot_claim_host_unknown_slot_snapshot_or_pre_cutoff_ticks() {
    for (tick, slot) in [(4, 0), (4, 2), (4, 255), (1, 1), (2, 1)] {
        let mut bytes = packet(4, 1, &[]);
        bytes[26] = slot;
        bytes[27..35].copy_from_slice(&u64::to_le_bytes(tick));
        rejected(
            &bytes,
            CONN,
            Channel::Reliable,
            Error::WrongAuthority,
            (0, 0),
        );
    }
}

#[test]
fn host_can_supply_joiner_backlog_only_before_exact_cutoff() {
    let (driver, mut source) = bound(1, limits());
    for (tick, slot) in [(2, 0), (2, 1), (3, 0), (4, 0)] {
        assert_eq!(
            driver.receive(CONN, Channel::Reliable, &packet(tick, slot, &[])),
            Ok(Accepted::New)
        );
    }
    let records = source.poll_remote();
    assert_eq!(
        records
            .iter()
            .map(|r| (r.tick, r.slot.0))
            .collect::<Vec<_>>(),
        vec![(2, 0), (2, 1), (3, 0), (4, 0)]
    );
    reset_decodes();
    assert_eq!(
        driver.receive(CONN, Channel::Reliable, &packet(3, 1, &[])),
        Err(Error::WrongAuthority)
    );
    assert_eq!(decodes(), (0, 0));
    assert_terminal(&driver, &mut source, Error::WrongAuthority);
}

#[test]
fn every_truncated_prefix_and_trailing_bytes_fail_before_application_decode() {
    let bytes = packet(4, 1, &[0, 1]);
    for end in 0..bytes.len() {
        rejected(
            &bytes[..end],
            CONN,
            Channel::Reliable,
            Error::Malformed,
            (0, 0),
        );
    }
    let mut trailing = bytes;
    trailing.extend_from_slice(&[0, 0, 0, 0]);
    rejected(&trailing, CONN, Channel::Reliable, Error::Malformed, (0, 0));
}

#[test]
fn declared_lengths_and_command_count_are_bounded_before_decode() {
    let original = packet(4, 1, &[1]);
    let mut bad = original.clone();
    bad[35..39].copy_from_slice(&u32::MAX.to_le_bytes());
    rejected(&bad, CONN, Channel::Reliable, Error::InputLimit, (0, 0));
    let mut bad = original.clone();
    bad[39..41].copy_from_slice(&u16::MAX.to_le_bytes());
    rejected(&bad, CONN, Channel::Reliable, Error::CommandLimit, (0, 0));
    let mut bad = original.clone();
    bad[RECORD_BYTES..RECORD_BYTES + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    rejected(&bad, CONN, Channel::Reliable, Error::CommandLimit, (0, 0));
    let mut bad = original.clone();
    bad[35..39].copy_from_slice(&21_u32.to_le_bytes());
    rejected(&bad, CONN, Channel::Reliable, Error::CommandLimit, (0, 0));
    let mut bad = original;
    bad[39..41].copy_from_slice(&2_u16.to_le_bytes());
    rejected(&bad, CONN, Channel::Reliable, Error::Malformed, (0, 0));
}

#[test]
fn maximum_legal_command_count_still_requires_complete_framing_before_decode() {
    let mut configured = limits();
    configured.max_commands = usize::from(u16::MAX);
    let (driver, mut source) = bound(0, configured);
    let mut bytes = packet(4, 1, &[]);
    bytes[39..41].copy_from_slice(&u16::MAX.to_le_bytes());
    reset_decodes();
    assert_eq!(
        driver.receive(CONN, Channel::Reliable, &bytes),
        Err(Error::Malformed)
    );
    assert_eq!(decodes(), (0, 0));
    assert_terminal(&driver, &mut source, Error::Malformed);
}

#[test]
fn aggregate_packet_budget_applies_even_when_each_field_is_within_its_limit() {
    let mut configured = limits();
    configured.max_packet_bytes = RECORD_BYTES + 15;
    configured.max_input_bytes = INPUT_BYTES;
    configured.max_command_bytes = 4;
    configured.max_commands = 2;
    let input = record(4, 1, &[0, 1]);
    assert_eq!(
        encode_p2p_input::<Arena, PortableArena>(&context(), &input, &configured),
        Err(Error::PacketLimit)
    );
    let (driver, mut source) = bound(0, configured);
    reset_decodes();
    assert_eq!(
        driver.receive(CONN, Channel::Reliable, &packet(4, 1, &[0, 1])),
        Err(Error::PacketLimit)
    );
    assert_eq!(decodes(), (0, 0));
    assert_terminal(&driver, &mut source, Error::PacketLimit);
}

#[test]
fn application_decode_failures_never_publish_partially_decoded_records() {
    let mut bad_input = packet(4, 1, &[0, 1]);
    bad_input[HEADER + 16..HEADER + 20].copy_from_slice(&2_u32.to_le_bytes());
    rejected(
        &bad_input,
        CONN,
        Channel::Reliable,
        Error::Malformed,
        (1, 0),
    );
    let mut bad_second_command = packet(4, 1, &[0, 1]);
    bad_second_command[RECORD_BYTES + 12..RECORD_BYTES + 16].copy_from_slice(&2_u32.to_le_bytes());
    rejected(
        &bad_second_command,
        CONN,
        Channel::Reliable,
        Error::Malformed,
        (1, 2),
    );

    // Structurally complete but semantically short slices are the codec's job.
    let mut short_input = packet(4, 1, &[]);
    short_input[35..39].copy_from_slice(&19_u32.to_le_bytes());
    short_input.pop();
    rejected(
        &short_input,
        CONN,
        Channel::Reliable,
        Error::Malformed,
        (1, 0),
    );
    let mut short_command = packet(4, 1, &[1]);
    short_command[RECORD_BYTES..RECORD_BYTES + 4].copy_from_slice(&3_u32.to_le_bytes());
    short_command.pop();
    rejected(
        &short_command,
        CONN,
        Channel::Reliable,
        Error::Malformed,
        (1, 1),
    );
}

#[test]
fn exact_duplicate_is_harmless_before_and_after_drain_and_at_record_capacity() {
    let (driver, mut source) = bound(
        0,
        P2pInputLimits {
            max_records: 1,
            ..limits()
        },
    );
    let bytes = packet(3, 1, &[1, 1]);
    reset_decodes();
    assert_eq!(
        driver.receive(CONN, Channel::Reliable, &bytes),
        Ok(Accepted::New)
    );
    for _ in 0..3 {
        assert_eq!(
            driver.receive(CONN, Channel::Reliable, &bytes),
            Ok(Accepted::Duplicate)
        );
    }
    let records = source.poll_remote();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].commands.len(), 2);
    assert_eq!(
        driver.receive(CONN, Channel::Reliable, &bytes),
        Ok(Accepted::Duplicate)
    );
    assert!(source.poll_remote().is_empty());
    assert_eq!(decodes(), (1, 2));
    assert_eq!(driver.check(), Ok(()));
}

#[test]
fn conflicting_input_or_ordered_commands_is_terminal_even_after_drain() {
    for changed in [packet(3, 1, &[1, 0]), packet(3, 1, &[0]), {
        let mut p = packet(3, 1, &[0, 1]);
        p[HEADER + 16] = 1;
        p
    }] {
        let (driver, mut source) = bound(0, limits());
        driver
            .receive(CONN, Channel::Reliable, &packet(3, 1, &[0, 1]))
            .unwrap();
        assert_eq!(source.poll_remote().len(), 1);
        reset_decodes();
        let error = Error::ConflictingDuplicate {
            tick: 3,
            slot: PlayerSlot(1),
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
fn local_exact_duplicates_do_not_requeue_but_conflicts_fail_observably() {
    let (driver, mut source) = bound(0, limits());
    source.send_local(3, PlayerSlot(0), ArenaInput::default(), vec![]);
    source.send_local(3, PlayerSlot(0), ArenaInput::default(), vec![]);
    assert_eq!(driver.pending(), (1, RECORD_BYTES));
    assert_eq!(driver.flush(|_, _| Ok(())).unwrap().sent, 1);
    source.send_local(3, PlayerSlot(0), ArenaInput::default(), vec![]);
    assert_eq!(driver.pending(), (0, 0));
    source.send_local(
        3,
        PlayerSlot(0),
        ArenaInput::default(),
        vec![SpawnBulletCmd { owner: 0 }],
    );
    assert_terminal(
        &driver,
        &mut source,
        Error::ConflictingDuplicate {
            tick: 3,
            slot: PlayerSlot(0),
        },
    );
}

#[test]
fn backpressure_retries_identical_fifo_head_without_drop_or_reordering() {
    let (driver, mut source) = bound(0, limits());
    for tick in 2..=4 {
        source.send_local(tick, PlayerSlot(0), ArenaInput::default(), vec![]);
    }
    let expected: Vec<_> = (2..=4).map(|tick| packet(tick, 0, &[])).collect();
    assert_eq!(driver.pending(), (3, 3 * RECORD_BYTES));
    let mut attempted = Vec::new();
    let first = driver
        .flush(|conn, bytes| {
            assert_eq!(conn, CONN);
            attempted.push(bytes.to_vec());
            Err(SendError::Backpressure)
        })
        .unwrap();
    assert_eq!(
        (first.sent, first.pending, first.backpressured),
        (0, 3, true)
    );
    assert_eq!(attempted, expected[..1]);
    assert_eq!(driver.pending(), (3, 3 * RECORD_BYTES));

    let second = driver
        .flush(|conn, bytes| {
            assert_eq!(conn, CONN);
            attempted.push(bytes.to_vec());
            if attempted.len() == 3 {
                Err(SendError::Backpressure)
            } else {
                Ok(())
            }
        })
        .unwrap();
    assert_eq!(
        (second.sent, second.pending, second.backpressured),
        (1, 2, true)
    );
    assert_eq!(
        attempted,
        vec![
            expected[0].clone(),
            expected[0].clone(),
            expected[1].clone()
        ]
    );
    assert_eq!(driver.pending(), (2, 2 * RECORD_BYTES));
    let mut delivered = vec![expected[0].clone()];
    let third = driver
        .flush(|conn, bytes| {
            assert_eq!(conn, CONN);
            delivered.push(bytes.to_vec());
            Ok(())
        })
        .unwrap();
    assert_eq!(
        (third.sent, third.pending, third.backpressured),
        (2, 0, false)
    );
    assert_eq!(delivered, expected);
    assert_eq!(driver.pending(), (0, 0));
    assert_eq!(driver.check(), Ok(()));
}

#[test]
fn record_budget_is_terminal_for_pending_output_and_survives_flush() {
    for drain in [false, true] {
        let (driver, mut source) = bound(
            0,
            P2pInputLimits {
                max_records: 2,
                ..limits()
            },
        );
        for tick in 2..=3 {
            source.send_local(tick, PlayerSlot(0), ArenaInput::default(), vec![]);
        }
        assert_eq!(driver.pending(), (2, 2 * RECORD_BYTES));
        if drain {
            assert_eq!(driver.flush(|_, _| Ok(())).unwrap().sent, 2);
        }
        source.send_local(4, PlayerSlot(0), ArenaInput::default(), vec![]);
        assert_terminal(&driver, &mut source, Error::RecordLimit);
    }
}

#[test]
fn record_budget_is_terminal_for_incoming_queue_and_survives_poll() {
    for drain in [false, true] {
        let (driver, mut source) = bound(
            0,
            P2pInputLimits {
                max_records: 2,
                ..limits()
            },
        );
        for tick in 3..=4 {
            driver
                .receive(CONN, Channel::Reliable, &packet(tick, 1, &[]))
                .unwrap();
        }
        if drain {
            assert_eq!(source.poll_remote().len(), 2);
        }
        reset_decodes();
        assert_eq!(
            driver.receive(CONN, Channel::Reliable, &packet(5, 1, &[])),
            Err(Error::RecordLimit)
        );
        assert_eq!(decodes(), (0, 0));
        assert_terminal(&driver, &mut source, Error::RecordLimit);
    }
}

fn byte_limited() -> P2pInputLimits {
    P2pInputLimits {
        max_packet_bytes: 128,
        max_input_bytes: INPUT_BYTES,
        max_command_bytes: 4,
        max_retained_bytes: 2 * RECORD_BYTES + 6,
        ..limits()
    }
}

#[test]
fn retained_bytes_bound_pending_and_delivered_local_records() {
    for drain in [false, true] {
        let (driver, mut source) = bound(0, byte_limited());
        for tick in 2..=3 {
            source.send_local(tick, PlayerSlot(0), ArenaInput::default(), vec![]);
        }
        assert_eq!(driver.pending(), (2, 2 * RECORD_BYTES));
        if drain {
            driver.flush(|_, _| Ok(())).unwrap();
        }
        source.send_local(4, PlayerSlot(0), ArenaInput::default(), vec![]);
        assert_terminal(&driver, &mut source, Error::ByteLimit);
    }
}

#[test]
fn retained_bytes_bound_incoming_records_even_after_poll() {
    for drain in [false, true] {
        let (driver, mut source) = bound(0, byte_limited());
        for tick in 3..=4 {
            driver
                .receive(CONN, Channel::Reliable, &packet(tick, 1, &[]))
                .unwrap();
        }
        if drain {
            assert_eq!(source.poll_remote().len(), 2);
        }
        reset_decodes();
        assert_eq!(
            driver.receive(CONN, Channel::Reliable, &packet(5, 1, &[])),
            Err(Error::ByteLimit)
        );
        assert_eq!(decodes(), (0, 0));
        assert_terminal(&driver, &mut source, Error::ByteLimit);
    }
}

#[test]
fn tick_window_exhaustion_and_lower_bound_are_observable_on_both_paths() {
    for tick in [0, 100, u64::MAX] {
        let mut bytes = packet(4, 1, &[]);
        bytes[27..35].copy_from_slice(&tick.to_le_bytes());
        rejected(&bytes, CONN, Channel::Reliable, Error::TickRange, (0, 0));
        let (driver, mut source) = bound(0, limits());
        source.send_local(tick, PlayerSlot(0), ArenaInput::default(), vec![]);
        assert_terminal(&driver, &mut source, Error::TickRange);
    }
    let (driver, mut source) = bound(0, limits());
    source.send_local(99, PlayerSlot(0), ArenaInput::default(), vec![]);
    assert_eq!(driver.check(), Ok(()));
    source.send_local(100, PlayerSlot(0), ArenaInput::default(), vec![]);
    assert_terminal(&driver, &mut source, Error::TickRange);
}

#[test]
fn source_side_cutoff_and_pre_admission_joiner_fail_closed() {
    let (driver, mut source) = Driver::new(PlayerSlot(1), context(), limits()).unwrap();
    source.send_local(3, PlayerSlot(1), ArenaInput::default(), vec![]);
    assert_terminal(&driver, &mut source, Error::WrongAuthority);
    for (local, tick, slot) in [(0, 3, 1), (1, 2, 1), (1, 3, 0)] {
        let (driver, mut source) = bound(local, limits());
        source.send_local(tick, PlayerSlot(slot), ArenaInput::default(), vec![]);
        assert_terminal(&driver, &mut source, Error::WrongAuthority);
    }
}

#[test]
fn binding_is_one_shot_and_context_and_snapshot_ranges_are_fenced() {
    let (driver, mut source) = bound(0, limits());
    assert_eq!(
        driver.bind(CONN, &context(), 1, 3),
        Err(Error::AlreadyBound)
    );
    assert_terminal(&driver, &mut source, Error::AlreadyBound);
    let other = CheckedJoinContext::new(
        context().join_id() + 1,
        context().attempt(),
        context().roster().clone(),
    )
    .unwrap();
    let (driver, mut source) = Driver::new(PlayerSlot(0), context(), limits()).unwrap();
    source.send_local(2, PlayerSlot(0), ArenaInput::default(), vec![]);
    assert_eq!(driver.bind(CONN, &other, 1, 3), Err(Error::WrongContext));
    assert_terminal(&driver, &mut source, Error::WrongContext);
    for (snapshot, first) in [(3, 3), (4, 3), (0, 0), (1, 100)] {
        let (driver, mut source) = Driver::new(PlayerSlot(0), context(), limits()).unwrap();
        assert_eq!(
            driver.bind(CONN, &context(), snapshot, first),
            Err(Error::TickRange)
        );
        assert_terminal(&driver, &mut source, Error::TickRange);
    }
}

#[test]
fn binding_discards_snapshotted_output_and_preserves_remaining_fifo() {
    let context = context();
    let (driver, mut source) = Driver::new(PlayerSlot(0), context.clone(), limits()).unwrap();
    for tick in 1..=4 {
        source.send_local(tick, PlayerSlot(0), ArenaInput::default(), vec![]);
    }
    assert_eq!(driver.pending(), (4, 4 * RECORD_BYTES));
    assert_eq!(
        driver.flush(|_, _| panic!("unbound output must not transmit")),
        Err(Error::NotBound)
    );
    assert_eq!(driver.check(), Ok(()));
    driver.bind(CONN, &context, 2, 3).unwrap();
    assert_eq!(driver.pending(), (2, 2 * RECORD_BYTES));
    let mut sent = Vec::new();
    driver
        .flush(|conn, bytes| {
            assert_eq!(conn, CONN);
            sent.push(bytes.to_vec());
            Ok(())
        })
        .unwrap();
    assert_eq!(sent, vec![packet(3, 0, &[]), packet(4, 0, &[])]);
}

#[test]
fn binding_rejects_pre_admission_joiner_records_at_or_after_cutoff() {
    for tick in [3, 4] {
        let (driver, mut source) = Driver::new(PlayerSlot(0), context(), limits()).unwrap();
        source.send_local(2, PlayerSlot(0), ArenaInput::default(), vec![]);
        source.send_local(tick, PlayerSlot(1), ArenaInput::default(), vec![]);
        assert_eq!(
            driver.check(),
            Ok(()),
            "host may author vacant inputs before binding"
        );
        assert_eq!(
            driver.bind(CONN, &context(), 1, 3),
            Err(Error::WrongAuthority)
        );
        assert_terminal(&driver, &mut source, Error::WrongAuthority);
    }
    let (driver, mut source) = Driver::new(PlayerSlot(0), context(), limits()).unwrap();
    source.send_local(2, PlayerSlot(1), ArenaInput::default(), vec![]);
    driver.bind(CONN, &context(), 1, 3).unwrap();
    let mut sent = Vec::new();
    driver
        .flush(|_, bytes| {
            sent.push(bytes.to_vec());
            Ok(())
        })
        .unwrap();
    assert_eq!(sent, vec![packet(2, 1, &[])]);
}

#[test]
fn flush_callback_can_inspect_receive_and_append_but_work_per_flush_is_bounded() {
    let (driver, mut source) = bound(0, limits());
    source.send_local(3, PlayerSlot(0), ArenaInput::default(), vec![]);
    let mut calls = 0;
    let flushed = driver
        .flush(|conn, bytes| {
            calls += 1;
            assert_eq!(conn, CONN);
            assert_eq!(bytes, packet(3, 0, &[]));
            assert_eq!(driver.check(), Ok(()));
            assert_eq!(driver.pending(), (1, RECORD_BYTES));
            driver
                .receive(CONN, Channel::Reliable, &packet(3, 1, &[]))
                .unwrap();
            source.send_local(4, PlayerSlot(0), ArenaInput::default(), vec![]);
            Ok(())
        })
        .unwrap();
    assert_eq!(
        calls, 1,
        "callback-appended output must wait for a later pump"
    );
    assert_eq!(
        (flushed.sent, flushed.pending, flushed.backpressured),
        (1, 1, false)
    );
    assert_eq!(source.poll_remote().len(), 1);
    assert_eq!(driver.pending(), (1, RECORD_BYTES));
    driver
        .flush(|_, bytes| {
            assert_eq!(bytes, packet(4, 0, &[]));
            Ok(())
        })
        .unwrap();
    assert_eq!(driver.pending(), (0, 0));
}

#[test]
fn cancellation_inside_flush_callback_stops_before_next_send() {
    let (driver, mut source) = bound(0, limits());
    for tick in 3..=4 {
        source.send_local(tick, PlayerSlot(0), ArenaInput::default(), vec![]);
    }
    let mut calls = 0;
    assert_eq!(
        driver.flush(|_, _| {
            calls += 1;
            driver.cancel();
            Ok(())
        }),
        Err(Error::Cancelled)
    );
    assert_eq!(calls, 1);
    assert_terminal(&driver, &mut source, Error::Cancelled);
}

#[test]
fn recursive_flush_is_terminal_without_reentrant_send_or_refcell_panic() {
    let (driver, mut source) = bound(0, limits());
    source.send_local(3, PlayerSlot(0), ArenaInput::default(), vec![]);
    assert_eq!(
        driver.flush(|_, _| {
            assert_eq!(
                driver.flush(|_, _| panic!("recursive flush must not transmit")),
                Err(Error::ReentrantFlush)
            );
            Ok(())
        }),
        Err(Error::ReentrantFlush)
    );
    assert_terminal(&driver, &mut source, Error::ReentrantFlush);
}

#[test]
fn encoder_checks_per_field_bounds_and_never_emits_relay_absence_or_unknown_slots() {
    for (configured, error) in [
        (
            P2pInputLimits {
                max_input_bytes: INPUT_BYTES - 1,
                ..limits()
            },
            Error::InputLimit,
        ),
        (
            P2pInputLimits {
                max_commands: 1,
                ..limits()
            },
            Error::CommandLimit,
        ),
        (
            P2pInputLimits {
                max_command_bytes: 3,
                ..limits()
            },
            Error::CommandLimit,
        ),
    ] {
        assert_eq!(
            encode_p2p_input::<Arena, PortableArena>(
                &context(),
                &record(4, 1, &[0, 1]),
                &configured
            ),
            Err(error)
        );
    }
    let mut absent = record(4, 1, &[]);
    absent.disconnected = true;
    assert_eq!(
        encode_p2p_input::<Arena, PortableArena>(&context(), &absent, &limits()),
        Err(Error::WrongAuthority)
    );
    assert_eq!(
        encode_p2p_input::<Arena, PortableArena>(&context(), &record(4, 2, &[]), &limits()),
        Err(Error::WrongAuthority)
    );
}

#[test]
fn unbound_receive_cancel_disconnect_and_transport_failure_are_terminal() {
    let (driver, mut source) = Driver::new(PlayerSlot(0), context(), limits()).unwrap();
    assert_eq!(
        driver.receive(CONN, Channel::Reliable, &[]),
        Err(Error::NotBound)
    );
    assert_terminal(&driver, &mut source, Error::NotBound);
    for transport in [
        SendError::UnknownConnection,
        SendError::TooLarge { max: 16 },
        SendError::DatagramsUnsupported,
    ] {
        let (driver, mut source) = bound(0, limits());
        source.send_local(3, PlayerSlot(0), ArenaInput::default(), vec![]);
        driver
            .receive(CONN, Channel::Reliable, &packet(3, 1, &[]))
            .unwrap();
        let error = Error::Transport(transport.clone());
        assert_eq!(
            driver.flush(|_, _| Err(transport.clone())),
            Err(error.clone())
        );
        assert_terminal(&driver, &mut source, error);
    }
    for (conn, error) in [
        (CONN, Error::Disconnected),
        (ConnId(999), Error::WrongConnection),
    ] {
        let (driver, mut source) = bound(0, limits());
        source.send_local(3, PlayerSlot(0), ArenaInput::default(), vec![]);
        driver
            .receive(CONN, Channel::Reliable, &packet(3, 1, &[]))
            .unwrap();
        assert_eq!(driver.disconnected(conn), Err(error.clone()));
        assert_terminal(&driver, &mut source, error);
    }
    let (driver, mut source) = bound(0, limits());
    source.send_local(3, PlayerSlot(0), ArenaInput::default(), vec![]);
    driver
        .receive(CONN, Channel::Reliable, &packet(3, 1, &[]))
        .unwrap();
    driver.cancel();
    driver.cancel();
    assert_terminal(&driver, &mut source, Error::Cancelled);
}

#[test]
fn invalid_records_and_earlier_buffered_records_never_reach_session_after_failure() {
    let context = context();
    let (driver, source) = Driver::new(PlayerSlot(0), context.clone(), limits()).unwrap();
    driver.bind(CONN, &context, 0, 1).unwrap();
    let mut cfg = SessionConfig::new(2, PlayerSlot(0), 42, 60);
    cfg.input_delay = 0;
    cfg.checksum_interval = 1;
    let mut session = Session::new(ArenaConfig { player_count: 2 }, cfg, source);
    driver
        .receive(CONN, Channel::Reliable, &packet(1, 1, &[1, 1]))
        .unwrap();
    driver.check().unwrap();
    session.advance(ArenaInput::default(), vec![]);
    driver.check().unwrap();
    session.poll_confirmed();
    driver.check().unwrap();
    assert_eq!(session.verified_tick(), 1);
    assert_eq!(session.last_remote_tick(PlayerSlot(1)), Some(1));
    let before = session.predicted_frame().checksum();
    driver
        .receive(CONN, Channel::Reliable, &packet(2, 1, &[1]))
        .unwrap();
    let mut wrong_generation = packet(3, 1, &[1]);
    wrong_generation[14] ^= 1;
    reset_decodes();
    assert_eq!(
        driver.receive(CONN, Channel::Reliable, &wrong_generation),
        Err(Error::WrongContext)
    );
    assert_eq!(driver.check(), Err(Error::WrongContext));
    assert_eq!(decodes(), (0, 0));
    // Do not advance a failed Session. Even a poll cannot import its queued data.
    let _ = session.poll_confirmed();
    assert_eq!(session.last_remote_tick(PlayerSlot(1)), Some(1));
    assert_eq!(session.verified_tick(), 1);
    assert_eq!(session.predicted_frame().checksum(), before);
    assert!(session.source_mut().poll_remote().is_empty());
}
