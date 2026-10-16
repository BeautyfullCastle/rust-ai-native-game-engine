//! Same-world, pre-activation source replacement uses real Session evidence.
//! Transport fencing and the final-readiness boundary are covered separately.

use orr_proto::{Channel, ConnId};
use orr_relay_net::p2p_input::P2pRollingWindow;
use orr_relay_net::{
    encode_p2p_input, P2pInputAccepted, P2pInputCodec, P2pInputDriver, P2pInputError as Error,
    P2pInputLimits, P2pInputSource,
};
use orr_session::{
    serve_checked_join, CheckedJoinContext, CheckedJoinTicket, InputSource, JoinBootstrap,
    JoinHoldRelease, JoinRoster, PlayerSlot, RemoteInput, Session, SessionConfig,
};
use orr_sim::FP;
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd};

const HOST: PlayerSlot = PlayerSlot(0);
const JOINER: PlayerSlot = PlayerSlot(1);
const OLD_CONN: ConnId = ConnId(23);
const NEW_CONN: ConnId = ConnId(24);
const RECORD_BYTES: usize = 41 + 20;
const FUTURE: u64 = 8;

struct PortableArena;
impl P2pInputCodec<Arena> for PortableArena {
    const SCHEMA: u64 = 0x0807_0605_0403_0201;

    fn encode_input(input: &ArenaInput, out: &mut Vec<u8>) {
        out.extend_from_slice(&input.axis_x.0.to_le_bytes());
        out.extend_from_slice(&input.axis_y.0.to_le_bytes());
        out.extend_from_slice(&input.buttons.to_le_bytes());
    }

    fn decode_input(bytes: &[u8]) -> Option<ArenaInput> {
        if bytes.len() != 20 {
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
        let owner = u32::from_le_bytes(bytes.try_into().ok()?);
        (owner < 2).then_some(SpawnBulletCmd { owner })
    }
}

type Driver = P2pInputDriver<Arena, PortableArena>;
type Source = P2pInputSource<Arena, PortableArena>;
type Peer = Session<Arena, Source>;

fn context(generation: u64, attempt: u32) -> CheckedJoinContext {
    CheckedJoinContext::new(
        generation,
        attempt,
        JoinRoster::completed(2, JOINER, HOST, vec![HOST], &[]).unwrap(),
    )
    .unwrap()
}

fn limits() -> P2pInputLimits {
    P2pInputLimits {
        max_packet_bytes: 96,
        max_input_bytes: 20,
        max_commands: 3,
        max_command_bytes: 4,
        max_records: 32,
        max_retained_bytes: 4096,
        ..P2pInputLimits::default()
    }
}

fn window() -> P2pRollingWindow {
    P2pRollingWindow {
        recent_ticks: 1,
        future_ticks: FUTURE,
    }
}

fn config(local: PlayerSlot, delay: u32) -> SessionConfig {
    let mut cfg = SessionConfig::new(2, local, 42, 60);
    cfg.input_delay = delay;
    cfg.max_prediction = 8;
    cfg.checksum_interval = 1;
    cfg.input_log_ticks = 1;
    cfg.keep_anchors = 3;
    cfg
}

fn host(delay: u32, bounds: P2pInputLimits) -> (Driver, Peer) {
    let (driver, source) = Driver::new_rolling(HOST, context(1, 1), bounds, window()).unwrap();
    let mut cfg = config(HOST, delay);
    cfg.vacant_slots.push(JOINER);
    let session = Peer::new(ArenaConfig { player_count: 2 }, cfg, source);
    (driver, session)
}

fn input(tick: u64) -> ArenaInput {
    ArenaInput::new(FP::from_raw((tick % 3) as i64 - 1), FP::ZERO, false)
}

fn commands() -> Vec<SpawnBulletCmd> {
    vec![
        SpawnBulletCmd { owner: 0 },
        SpawnBulletCmd { owner: 1 },
        SpawnBulletCmd { owner: 0 },
    ]
}

fn advance_verified(driver: &Driver, session: &mut Peer, until: u64) {
    while session.head_tick() < until {
        let tick = session.next_send_tick();
        session.advance(input(tick), vec![]);
        session.poll_confirmed();
        assert_eq!(session.verified_tick(), session.head_tick());
        driver.check().unwrap();
    }
}

fn grant(session: &mut Peer, ctx: &CheckedJoinContext) -> CheckedJoinTicket {
    let mut cfg = config(JOINER, session.config().input_delay);
    cfg.join_id = ctx.join_id();
    let mut bootstrap =
        JoinBootstrap::<Arena, Source>::new(cfg, ctx.roster().clone(), 4096, 1 << 20).unwrap();
    let mut request = Vec::new();
    for _ in 0..ctx.attempt() {
        request = bootstrap.next_request().unwrap();
    }
    assert_eq!(bootstrap.context(), Some(ctx.clone()));
    serve_checked_join(session, ctx, &request).unwrap().1
}

fn bind_grant(driver: &Driver, session: &mut Peer) -> CheckedJoinTicket {
    let ticket = grant(session, &context(1, 1));
    let raw = ticket.ticket();
    driver
        .bind(
            OLD_CONN,
            ticket.context(),
            raw.snapshot_tick,
            raw.first_input_tick,
        )
        .unwrap();
    ticket
}

fn retire(driver: &Driver, session: &mut Peer) {
    // The exact hold is released without deleting its still-pending grant.
    if let Some(lease) = session.join_hold_lease(JOINER) {
        assert_eq!(
            session.release_owned_join_hold(&lease),
            JoinHoldRelease::ReleasedAssignmentRetained
        );
    }
    driver.cancel();
    assert_eq!(driver.check(), Err(Error::Cancelled));
    assert_eq!(driver.pending(), (0, 0));
    assert_eq!(driver.retained_evidence(), (0, 0));
}

fn encoded(ctx: &CheckedJoinContext, record: &RemoteInput<Arena>) -> Vec<u8> {
    encode_p2p_input::<Arena, PortableArena>(ctx, record, &limits()).unwrap()
}

#[derive(Debug, PartialEq, Eq)]
struct World {
    head: u64,
    verified: u64,
    next_send: u64,
    predicted: Vec<u8>,
    checkpoint: Option<Vec<u8>>,
    checksums: Vec<(u64, u64)>,
    anchors: Vec<(u64, u64, Vec<u8>)>,
    rollbacks: u64,
    pending_grant: Option<(u64, u64, u32)>,
    tail: Vec<Vec<u8>>,
}

fn world(session: &Peer) -> World {
    World {
        head: session.head_tick(),
        verified: session.verified_tick(),
        next_send: session.next_send_tick(),
        predicted: session.predicted_frame().to_bytes(),
        checkpoint: session.verified_frame().map(|frame| frame.to_bytes()),
        checksums: session.checksums().to_vec(),
        anchors: session
            .anchors()
            .iter()
            .map(|anchor| (anchor.tick, anchor.checksum, anchor.frame_bytes.clone()))
            .collect(),
        rollbacks: session.rollback_count(),
        pending_grant: session.pending_join(JOINER).map(|ticket| {
            (
                ticket.snapshot_tick,
                ticket.first_input_tick,
                ticket.attempt,
            )
        }),
        tail: session
            .authored_since(session.verified_tick())
            .iter()
            .map(|record| encoded(&context(1, 1), record))
            .collect(),
    }
}

fn rejected(driver: &Driver, session: &mut Peer, ctx: CheckedJoinContext, error: Error) {
    let before = world(session);
    let old_status = driver.check();
    let old_pending = driver.pending();
    let old_evidence = driver.retained_evidence();
    // A repeated coverage/budget error also proves the first attempt did not
    // secretly install a different source: the exact-Rc check would then fail.
    for _ in 0..2 {
        assert_eq!(
            driver
                .replace_cancelled_host_rolling(session, ctx.clone())
                .err(),
            Some(error.clone())
        );
        assert_eq!(world(session), before);
        assert_eq!(driver.check(), old_status);
        assert_eq!(driver.pending(), old_pending);
        assert_eq!(driver.retained_evidence(), old_evidence);
    }
}

#[test]
fn cancelled_bound_host_preserves_world_and_reencodes_the_complete_tail() {
    let (old, mut session) = host(2, limits());
    advance_verified(&old, &mut session, 24);
    assert!(session.verified_tick() > FUTURE);
    let ticket = bind_grant(&old, &mut session).ticket();
    assert_eq!((ticket.snapshot_tick, ticket.first_input_tick), (24, 27));
    for _ in 0..3 {
        let tick = session.next_send_tick();
        session.advance(input(tick), commands());
        session.poll_confirmed();
        old.check().unwrap();
    }
    assert_eq!((session.head_tick(), session.verified_tick()), (27, 26));
    let expected_host_tail = session.authored_since(session.verified_tick());
    assert_eq!(expected_host_tail.len(), 3);
    assert!(expected_host_tail.iter().all(|record| record.slot == HOST));
    let old_lease = session.join_hold_lease(JOINER).unwrap();
    retire(&old, &mut session);
    let before = world(&session);
    let fresh_ctx = context(2, 1);
    let fresh = old
        .replace_cancelled_host_rolling(&mut session, fresh_ctx.clone())
        .unwrap();
    assert_eq!(world(&session), before);
    assert_eq!(fresh.rolling_progress(), Some((26, 26)));
    assert_eq!(fresh.pending().0, 3);
    assert_eq!(
        fresh.flush(|_, _| panic!("replacement starts unbound")),
        Err(Error::NotBound)
    );

    session
        .mark_slot_vacant(JOINER, ticket.first_input_tick)
        .unwrap();
    assert_eq!(session.head_tick(), before.head);
    assert_eq!(session.verified_tick(), before.verified);
    assert_eq!(session.predicted_frame().to_bytes(), before.predicted);
    assert_eq!(fresh.pending().0, 6);
    fresh.check().unwrap();

    // Repeated retirement/late delivery through the old driver cannot reach the
    // new source, mutate its buffers, or advance its progress.
    old.cancel();
    assert_eq!(old.disconnected(OLD_CONN), Err(Error::Cancelled));
    assert_eq!(
        old.receive(OLD_CONN, Channel::Reliable, b"late"),
        Err(Error::Cancelled)
    );
    assert_eq!(
        old.flush(|_, _| panic!("retired driver must not send")),
        Err(Error::Cancelled)
    );
    assert_eq!(fresh.rolling_progress(), Some((26, 26)));
    assert_eq!(fresh.pending().0, 6);
    fresh.check().unwrap();

    let fresh_ticket = grant(&mut session, &fresh_ctx).ticket();
    assert_eq!(
        (fresh_ticket.snapshot_tick, fresh_ticket.first_input_tick),
        (26, 30)
    );
    let fresh_world = world(&session);
    let fresh_pending = fresh.pending();
    let fresh_evidence = fresh.retained_evidence();
    assert_eq!(
        session.release_owned_join_hold(&old_lease),
        JoinHoldRelease::NoMatchingHold
    );
    assert!(session.join_hold_lease(JOINER).is_some());
    assert_eq!(world(&session), fresh_world);
    assert_eq!(fresh.pending(), fresh_pending);
    assert_eq!(fresh.retained_evidence(), fresh_evidence);
    fresh.check().unwrap();
    fresh.bind(NEW_CONN, &fresh_ctx, 26, 30).unwrap();
    let (receiver, mut source) =
        Driver::new_rolling(JOINER, fresh_ctx.clone(), limits(), window()).unwrap();
    receiver.bind(NEW_CONN, &fresh_ctx, 26, 30).unwrap();
    let mut sent = Vec::new();
    fresh
        .flush(|conn, bytes| {
            assert_eq!(conn, NEW_CONN);
            assert_eq!(
                receiver.receive(conn, Channel::Reliable, bytes),
                Ok(P2pInputAccepted::New)
            );
            sent.push(bytes.to_vec());
            Ok(())
        })
        .unwrap();
    let mut expected: Vec<_> = expected_host_tail
        .iter()
        .map(|r| encoded(&fresh_ctx, r))
        .collect();
    for tick in 27..30 {
        expected.push(encoded(
            &fresh_ctx,
            &RemoteInput {
                tick,
                slot: JOINER,
                input: ArenaInput::default(),
                commands: vec![],
                disconnected: false,
            },
        ));
    }
    assert_eq!(
        sent, expected,
        "every original input and repeated command is preserved"
    );
    let received = source.poll_remote();
    assert_eq!(received.len(), 6);
    for record in &received[..3] {
        assert_eq!(record.input, input(record.tick));
        assert_eq!(record.commands, commands());
    }
    assert!(received[3..]
        .iter()
        .all(|r| r.slot == JOINER && r.commands.is_empty()));
    assert!(source.poll_remote().is_empty());
    assert_eq!(fresh.pending(), (0, 0));
}

#[test]
fn unbound_retry_seeds_actual_verified_progress_and_retains_both_authored_slots() {
    let (old, mut session) = host(2, limits());
    advance_verified(&old, &mut session, 24);
    let tail = session.authored_since(24);
    assert_eq!(
        tail.iter().map(|r| (r.tick, r.slot)).collect::<Vec<_>>(),
        vec![(25, HOST), (25, JOINER), (26, HOST), (26, JOINER)]
    );
    retire(&old, &mut session);
    let before = world(&session);
    let fresh_ctx = context(2, 1);
    let fresh = old
        .replace_cancelled_host_rolling(&mut session, fresh_ctx.clone())
        .unwrap();
    assert_eq!(world(&session), before);
    assert_eq!(fresh.rolling_progress(), Some((24, 24)));
    assert_eq!(fresh.pending(), (4, 4 * RECORD_BYTES));
    let ticket = grant(&mut session, &fresh_ctx).ticket();
    fresh
        .bind(
            NEW_CONN,
            &fresh_ctx,
            ticket.snapshot_tick,
            ticket.first_input_tick,
        )
        .unwrap();
    let mut sent = Vec::new();
    fresh
        .flush(|_, bytes| {
            sent.push(bytes.to_vec());
            Ok(())
        })
        .unwrap();
    assert_eq!(
        sent,
        tail.iter()
            .map(|r| encoded(&fresh_ctx, r))
            .collect::<Vec<_>>()
    );
    session.advance(input(27), commands());
    fresh.check().unwrap();
    assert_eq!(
        fresh.pending().0,
        1,
        "a tick beyond the initial horizon remains valid"
    );
}

#[test]
fn replacement_requires_a_fresh_generation_and_the_exact_cancelled_source() {
    let (old, mut session) = host(0, limits());
    rejected(&old, &mut session, context(2, 1), Error::UnsafeReplacement);
    old.cancel();
    for ctx in [
        context(1, 1),
        context(1, 2),
        CheckedJoinContext::new(
            2,
            1,
            JoinRoster::completed(3, JOINER, HOST, vec![HOST], &[PlayerSlot(2)]).unwrap(),
        )
        .unwrap(),
    ] {
        rejected(&old, &mut session, ctx, Error::UnsafeReplacement);
    }
    let (foreign, _) = host(0, limits());
    foreign.cancel();
    rejected(
        &foreign,
        &mut session,
        context(2, 1),
        Error::UnsafeReplacement,
    );
    let fresh = old
        .replace_cancelled_host_rolling(&mut session, context(2, 1))
        .unwrap();
    assert_eq!(fresh.check(), Ok(()));
    // Even the original legitimate handle becomes foreign after the swap.
    rejected(&old, &mut session, context(3, 1), Error::UnsafeReplacement);
    assert_eq!(fresh.check(), Ok(()));
}

#[test]
fn cancel_does_not_make_another_terminal_error_recoverable() {
    for disconnected in [false, true] {
        let (old, mut session) = host(0, limits());
        bind_grant(&old, &mut session);
        let expected = if disconnected {
            old.disconnected(OLD_CONN).unwrap_err()
        } else {
            old.receive(OLD_CONN, Channel::Unreliable, b"bad")
                .unwrap_err()
        };
        old.cancel();
        assert_eq!(old.check(), Err(expected));
        rejected(&old, &mut session, context(2, 1), Error::UnsafeReplacement);
    }
}

#[test]
fn finite_joiner_and_nonhost_session_owners_are_not_recoverable() {
    let (finite, source) = Driver::new(HOST, context(1, 1), limits()).unwrap();
    let mut session = Peer::new(ArenaConfig { player_count: 2 }, config(HOST, 0), source);
    finite.cancel();
    rejected(
        &finite,
        &mut session,
        context(2, 1),
        Error::UnsafeReplacement,
    );

    for (driver_slot, session_slot, relay) in [
        (JOINER, JOINER, false),
        (HOST, JOINER, false),
        (HOST, HOST, true),
    ] {
        let (old, source) =
            Driver::new_rolling(driver_slot, context(1, 1), limits(), window()).unwrap();
        let mut cfg = config(session_slot, 0);
        cfg.relay = relay;
        let mut session = Peer::new(ArenaConfig { player_count: 2 }, cfg, source);
        old.cancel();
        rejected(&old, &mut session, context(2, 1), Error::UnsafeReplacement);
    }
}

#[test]
fn pending_grant_and_driver_binding_must_agree() {
    for case in 0..3 {
        let (old, mut session) = host(0, limits());
        if case != 0 {
            grant(&mut session, &context(1, 1));
        }
        if case != 1 {
            old.bind(OLD_CONN, &context(1, 1), 0, if case == 2 { 2 } else { 1 })
                .unwrap();
        }
        retire(&old, &mut session);
        rejected(&old, &mut session, context(2, 1), Error::UnsafeReplacement);
    }
}

#[test]
fn even_unpolled_remote_admission_survives_cancel_and_forbids_replacement() {
    for poll in [false, true] {
        let (old, mut session) = host(0, limits());
        advance_verified(&old, &mut session, 16);
        let ticket = bind_grant(&old, &mut session).ticket();
        let bytes = encoded(
            &context(1, 1),
            &RemoteInput {
                tick: ticket.first_input_tick,
                slot: JOINER,
                input: input(17),
                commands: commands(),
                disconnected: false,
            },
        );
        assert_eq!(
            old.receive(OLD_CONN, Channel::Reliable, &bytes),
            Ok(P2pInputAccepted::New)
        );
        if poll {
            session.poll_confirmed();
            assert_eq!(
                session.last_remote_tick(JOINER),
                Some(ticket.first_input_tick)
            );
        } else {
            assert_eq!(session.last_remote_tick(JOINER), None);
            assert!(session.pending_join(JOINER).is_some());
        }
        retire(&old, &mut session);
        assert!(
            session.source_mut().poll_remote().is_empty(),
            "cancel drained the admitted input"
        );
        rejected(&old, &mut session, context(2, 1), Error::UnsafeReplacement);
    }
}

#[test]
fn missing_local_history_is_rejected_without_installing_the_candidate_source() {
    let (old, source) = Driver::new_rolling(HOST, context(1, 1), limits(), window()).unwrap();
    // This real Session has never authored the other slot. No synthetic source
    // callback or private-field mutation is needed to expose incomplete coverage.
    let mut session = Peer::new(ArenaConfig { player_count: 2 }, config(HOST, 0), source);
    session.advance(input(1), commands());
    assert_eq!(session.verified_tick(), 0);
    assert_eq!(session.authored_since(0).len(), 1);
    old.cancel();
    rejected(
        &old,
        &mut session,
        context(2, 1),
        Error::MissingHistory {
            tick: 1,
            slot: JOINER,
        },
    );
}

#[test]
fn genuinely_pruned_authored_ticks_cannot_be_reconstructed_after_a_restore() {
    let (old, mut session) = host(0, limits());
    advance_verified(&old, &mut session, 20);
    let frame = session.verified_frame().unwrap();
    let checkpoint = frame.to_bytes();
    let checksum = frame.checksum();
    advance_verified(&old, &mut session, 24);
    assert_eq!(
        session
            .authored_since(20)
            .iter()
            .map(|record| (record.tick, record.slot))
            .collect::<Vec<_>>(),
        vec![(23, HOST), (23, JOINER), (24, HOST), (24, JOINER)]
    );
    // Invalidate the old source before the public restore operation. Restoring
    // a real checkpoint does not recreate the already-pruned authored log.
    old.cancel();
    session
        .restore_confirmed(20, checksum, &checkpoint, 21)
        .unwrap();
    assert_eq!(session.verified_tick(), 20);
    assert_eq!(session.next_send_tick(), 25);
    rejected(
        &old,
        &mut session,
        context(2, 1),
        Error::MissingHistory {
            tick: 21,
            slot: HOST,
        },
    );
}

#[test]
fn record_capacity_includes_future_vacancy_defaults_before_installation() {
    for capacity in [5, 6] {
        let bounds = P2pInputLimits {
            max_records: capacity,
            ..limits()
        };
        assert_vacancy_capacity(bounds, (capacity == 5).then_some(Error::RecordLimit));
    }
}

#[test]
fn byte_capacity_includes_future_vacancy_defaults_before_installation() {
    for bytes in [6 * RECORD_BYTES - 1, 6 * RECORD_BYTES] {
        let bounds = P2pInputLimits {
            max_retained_bytes: bytes,
            ..limits()
        };
        assert_vacancy_capacity(
            bounds,
            (bytes < 6 * RECORD_BYTES).then_some(Error::ByteLimit),
        );
    }
}

fn assert_vacancy_capacity(bounds: P2pInputLimits, expected: Option<Error>) {
    let (old, mut session) = host(0, bounds);
    advance_verified(&old, &mut session, 16);
    let ticket = bind_grant(&old, &mut session).ticket();
    for _ in 0..3 {
        session.advance(ArenaInput::default(), vec![]);
        old.check().unwrap();
    }
    assert_eq!(session.verified_tick(), 16);
    assert_eq!(old.pending(), (3, 3 * RECORD_BYTES));
    retire(&old, &mut session);
    if let Some(error) = expected {
        rejected(&old, &mut session, context(2, 1), error);
    } else {
        let before = world(&session);
        let fresh = old
            .replace_cancelled_host_rolling(&mut session, context(2, 1))
            .unwrap();
        assert_eq!(world(&session), before);
        assert_eq!(fresh.pending(), (3, 3 * RECORD_BYTES));
        session
            .mark_slot_vacant(JOINER, ticket.first_input_tick)
            .unwrap();
        fresh.check().unwrap();
        assert_eq!(fresh.pending(), (6, 6 * RECORD_BYTES));
        assert_eq!(fresh.retained_evidence(), (6, 6 * RECORD_BYTES));
        session.poll_confirmed();
        fresh.check().unwrap();
        assert_eq!(session.verified_tick(), session.head_tick());
    }
}
