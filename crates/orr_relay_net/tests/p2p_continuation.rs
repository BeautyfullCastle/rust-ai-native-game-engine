//! A fenced two-player host continues only through exact, contiguous admissions.
//! These tests use real Sessions and public APIs, never caller-reported maxima.

use orr_proto::{Channel, ConnId};
use orr_relay_net::p2p_input::P2pRollingWindow;
use orr_relay_net::{
    encode_p2p_input, P2pInputAccepted as Accepted, P2pInputCodec, P2pInputDriver,
    P2pInputError as Error, P2pInputLimits, P2pInputSource,
};
use orr_session::{
    serve_checked_join, CheckedJoinContext, CheckedJoinTicket, InputSource, JoinBootstrap,
    JoinHoldRelease, JoinRoster, PlayerSlot, RemoteInput, Session, SessionConfig,
};
use orr_sim::{Simulation, TickInputs, FP};
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd};

const HOST: PlayerSlot = PlayerSlot(0);
const JOINER: PlayerSlot = PlayerSlot(1);
const OLD_CONN: ConnId = ConnId(23);
const NEW_CONN: ConnId = ConnId(24);
const OLD_ID: u64 = 10;
const RECORD_BYTES: usize = 41 + 20;
const COMMAND_BYTES: usize = RECORD_BYTES + 3 * 8;

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

fn context(id: u64) -> CheckedJoinContext {
    CheckedJoinContext::new(
        id,
        1,
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
        future_ticks: 8,
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
    let (driver, source) = Driver::new_rolling(HOST, context(OLD_ID), bounds, window()).unwrap();
    let mut cfg = config(HOST, delay);
    cfg.vacant_slots.push(JOINER);
    (
        driver,
        Peer::new(ArenaConfig { player_count: 2 }, cfg, source),
    )
}

fn input(tick: u64, slot: PlayerSlot) -> ArenaInput {
    ArenaInput::new(
        FP::from_int((tick % 3) as i32 - 1),
        FP::from_int(i32::from(slot.0) * 2 - 1),
        false,
    )
}

fn commands(slot: PlayerSlot) -> Vec<SpawnBulletCmd> {
    [slot.0, 1 - slot.0, slot.0]
        .map(|owner| SpawnBulletCmd {
            owner: u32::from(owner),
        })
        .to_vec()
}

fn record(tick: u64, slot: PlayerSlot, with_commands: bool) -> RemoteInput<Arena> {
    RemoteInput {
        tick,
        slot,
        input: input(tick, slot),
        commands: if with_commands {
            commands(slot)
        } else {
            vec![]
        },
        disconnected: false,
    }
}

fn encoded(ctx: &CheckedJoinContext, record: &RemoteInput<Arena>) -> Vec<u8> {
    encode_p2p_input::<Arena, PortableArena>(
        ctx,
        record,
        &P2pInputLimits {
            end_tick: u64::MAX,
            ..limits()
        },
    )
    .unwrap()
}

fn admit(driver: &Driver, tick: u64, with_commands: bool) {
    assert_eq!(
        driver.receive(
            OLD_CONN,
            Channel::Reliable,
            &encoded(&context(OLD_ID), &record(tick, JOINER, with_commands)),
        ),
        Ok(Accepted::New)
    );
}

fn advance_verified(driver: &Driver, session: &mut Peer, until: u64) {
    while session.head_tick() < until {
        session.advance(input(session.next_send_tick(), HOST), vec![]);
        session.poll_confirmed();
        driver.check().unwrap();
        assert_eq!(session.verified_tick(), session.head_tick());
    }
}

fn grant(session: &mut Peer, ctx: &CheckedJoinContext) -> CheckedJoinTicket {
    let mut cfg = config(JOINER, session.config().input_delay);
    cfg.join_id = ctx.join_id();
    let mut bootstrap =
        JoinBootstrap::<Arena, Source>::new(cfg, ctx.roster().clone(), 4096, 1 << 20).unwrap();
    let request = bootstrap.next_request().unwrap();
    serve_checked_join(session, ctx, &request).unwrap().1
}

fn bind_grant(driver: &Driver, session: &mut Peer) -> CheckedJoinTicket {
    let ticket = grant(session, &context(OLD_ID));
    driver
        .bind(
            OLD_CONN,
            ticket.context(),
            ticket.ticket().snapshot_tick,
            ticket.ticket().first_input_tick,
        )
        .unwrap();
    ticket
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
    }
}

fn tail(session: &Peer) -> Vec<Vec<u8>> {
    session
        .authored_since(0)
        .iter()
        .map(|record| encoded(&context(OLD_ID), record))
        .collect()
}

fn rejected_replacement(
    driver: &Driver,
    session: &mut Peer,
    ctx: CheckedJoinContext,
    error: Error,
) {
    let before = world(session);
    let authored = tail(session);
    let grant = session.pending_join(JOINER).map(|ticket| {
        (
            ticket.snapshot_tick,
            ticket.first_input_tick,
            ticket.attempt,
        )
    });
    let remote = session.last_remote_tick(JOINER);
    let retained = driver.retained_evidence();
    let pending = driver.pending();
    let status = driver.check();
    // Repeating the call also proves a failed preflight did not swap the Rc.
    for _ in 0..2 {
        assert_eq!(
            driver
                .replace_fenced_host_rolling(session, ctx.clone())
                .err(),
            Some(error.clone())
        );
        assert_eq!(world(session), before);
        assert_eq!(tail(session), authored);
        assert_eq!(
            session.pending_join(JOINER).map(|ticket| (
                ticket.snapshot_tick,
                ticket.first_input_tick,
                ticket.attempt,
            )),
            grant
        );
        assert_eq!(session.last_remote_tick(JOINER), remote);
        assert_eq!(driver.retained_evidence(), retained);
        assert_eq!(driver.pending(), pending);
        assert_eq!(driver.check(), status);
    }
}

fn assert_fenced_nonclearing(driver: &Driver) {
    let retained = driver.retained_evidence();
    let pending = driver.pending();
    assert_eq!(
        driver.receive(OLD_CONN, Channel::Reliable, b"late malformed bytes"),
        Err(Error::Fenced)
    );
    assert_eq!(
        driver.flush(|_, _| panic!("fenced output must not reach transport")),
        Err(Error::Fenced)
    );
    assert_eq!(driver.disconnected(OLD_CONN), Err(Error::Fenced));
    assert_eq!(
        driver.bind(NEW_CONN, &context(OLD_ID), 0, 1),
        Err(Error::Fenced)
    );
    assert_eq!(driver.retained_evidence(), retained);
    assert_eq!(driver.pending(), pending);
    assert_eq!(driver.check(), Ok(()));
}

#[test]
fn accepted_unpolled_ordered_tail_verifies_before_same_world_replacement() {
    let (old, mut session) = host(2, limits());
    advance_verified(&old, &mut session, 16);
    let ticket = bind_grant(&old, &mut session).ticket();
    let original_lease = session.join_hold_lease(JOINER).unwrap();
    assert_eq!((ticket.snapshot_tick, ticket.first_input_tick), (16, 19));
    for _ in 0..6 {
        session.advance(input(session.next_send_tick(), HOST), commands(HOST));
        old.check().unwrap();
    }
    assert_eq!(
        (
            session.head_tick(),
            session.verified_tick(),
            session.next_send_tick()
        ),
        (22, 18, 25)
    );
    for tick in 19..=21 {
        admit(&old, tick, true);
    }
    assert_eq!(session.last_remote_tick(JOINER), None);
    assert!(session.pending_join(JOINER).is_some());
    let retained = old.retained_evidence();
    let pending = old.pending();
    assert_eq!(old.fence_host_continuation(&session, OLD_CONN), Ok(21));
    assert_eq!(old.retained_evidence(), retained);
    assert_eq!(old.pending(), pending);
    assert_fenced_nonclearing(&old);
    rejected_replacement(
        &old,
        &mut session,
        context(OLD_ID + 1),
        Error::UnverifiedContinuation,
    );

    assert_eq!(old.verify_host_continuation(&mut session), Ok(21));
    assert_eq!(session.verified_tick(), 21);
    assert_eq!(
        session.next_send_tick(),
        25,
        "verification never authors new input"
    );
    assert_eq!(session.last_remote_tick(JOINER), Some(21));
    assert!(
        session.rollback_count() > 0,
        "unpolled commands correct the predicted tail"
    );
    assert_eq!(
        old.check(),
        Ok(()),
        "accepted input was drained before cancellation"
    );

    let mut reference = Simulation::<Arena>::new(ArenaConfig { player_count: 2 }, 60, 42);
    for tick in 1..=21 {
        let mut inputs = TickInputs::new(tick, 2);
        if tick > 2 {
            inputs.set_input(HOST, input(tick, HOST));
        }
        if tick >= 19 {
            inputs.set_input(JOINER, input(tick, JOINER));
            for slot in [HOST, JOINER] {
                for command in commands(slot) {
                    inputs.push_command(slot, command);
                }
            }
        }
        reference.step(&inputs);
    }
    assert_eq!(
        session.verified_frame().unwrap().to_bytes(),
        reference.frame().to_bytes(),
        "all accepted inputs and repeated commands survive in exact order"
    );
    let before = world(&session);
    let host_tail = session.authored_since(21);
    assert_eq!(host_tail.len(), 3);
    assert!(host_tail
        .iter()
        .all(|record| record.slot == HOST && record.commands == commands(HOST)));
    let fresh_ctx = context(OLD_ID + 1);
    let fresh = old
        .replace_fenced_host_rolling(&mut session, fresh_ctx.clone())
        .unwrap();
    assert_eq!(world(&session), before);
    assert_eq!(fresh.rolling_progress(), Some((21, 21)));
    assert_eq!(fresh.pending(), (6, 3 * COMMAND_BYTES + 3 * RECORD_BYTES));
    assert_eq!(fresh.retained_evidence(), fresh.pending());
    assert_eq!(old.check(), Err(Error::Cancelled));
    assert_eq!(old.pending(), (0, 0));
    assert_eq!(old.retained_evidence(), (0, 0));
    let defaults: Vec<_> = session
        .authored_since(21)
        .into_iter()
        .filter(|r| r.slot == JOINER)
        .collect();
    assert_eq!(
        defaults.iter().map(|r| r.tick).collect::<Vec<_>>(),
        vec![22, 23, 24]
    );
    assert!(defaults
        .iter()
        .all(|r| r.input == ArenaInput::default() && r.commands.is_empty()));
    assert!(
        session.mark_slot_vacant(JOINER, 22).is_err(),
        "departure commits exactly once"
    );
    assert_eq!(fresh.pending().0, 6);

    let fresh_ticket = grant(&mut session, &fresh_ctx).ticket();
    fresh
        .bind(
            NEW_CONN,
            &fresh_ctx,
            fresh_ticket.snapshot_tick,
            fresh_ticket.first_input_tick,
        )
        .unwrap();
    let (receiver, mut receiver_source) =
        Driver::new_rolling(JOINER, fresh_ctx.clone(), limits(), window()).unwrap();
    receiver
        .bind(
            NEW_CONN,
            &fresh_ctx,
            fresh_ticket.snapshot_tick,
            fresh_ticket.first_input_tick,
        )
        .unwrap();
    let fresh_world = world(&session);
    let fresh_tail = tail(&session);
    let fresh_pending = fresh.pending();
    let fresh_notice = session.backlog_notice(JOINER).unwrap();
    assert_eq!(
        session.release_owned_join_hold(&original_lease),
        JoinHoldRelease::NoMatchingHold
    );
    assert!(session.join_hold_lease(JOINER).is_some());
    assert_eq!(session.backlog_notice(JOINER), Some(fresh_notice));
    assert_eq!(world(&session), fresh_world);
    assert_eq!(tail(&session), fresh_tail);
    assert_eq!(fresh.pending(), fresh_pending);
    old.cancel();
    assert_eq!(
        old.receive(OLD_CONN, Channel::Reliable, b"late"),
        Err(Error::Cancelled)
    );
    assert_eq!(old.disconnected(OLD_CONN), Err(Error::Cancelled));
    assert_eq!(
        old.flush(|_, _| panic!("cancelled driver must not send")),
        Err(Error::Cancelled)
    );
    assert_eq!(
        old.verify_host_continuation(&mut session),
        Err(Error::Cancelled)
    );
    assert_eq!(world(&session), fresh_world);
    assert_eq!(tail(&session), fresh_tail);
    assert_eq!(fresh.pending(), fresh_pending);
    fresh.check().unwrap();
    let mut sent = Vec::new();
    fresh
        .flush(|conn, bytes| {
            assert_eq!(conn, NEW_CONN);
            assert_eq!(
                receiver.receive(conn, Channel::Reliable, bytes),
                Ok(Accepted::New)
            );
            sent.push(bytes.to_vec());
            Ok(())
        })
        .unwrap();
    let expected: Vec<_> = host_tail
        .iter()
        .chain(&defaults)
        .map(|r| encoded(&fresh_ctx, r))
        .collect();
    assert_eq!(
        sent, expected,
        "only the fresh checked generation transmits preserved history"
    );
    assert_eq!(receiver_source.poll_remote().len(), 6);
    assert!(receiver_source.poll_remote().is_empty());
}

#[test]
fn target_ahead_of_prediction_is_verified_without_authoring() {
    let (old, mut session) = host(2, limits());
    bind_grant(&old, &mut session);
    for _ in 0..2 {
        session.advance(input(session.next_send_tick(), HOST), commands(HOST));
    }
    session.poll_confirmed();
    assert_eq!(
        (
            session.head_tick(),
            session.verified_tick(),
            session.next_send_tick()
        ),
        (2, 2, 5)
    );
    admit(&old, 3, true);
    admit(&old, 4, true);
    assert_eq!(old.fence_host_continuation(&session, OLD_CONN), Ok(4));
    let authored = tail(&session);
    assert_eq!(old.verify_host_continuation(&mut session), Ok(4));
    assert_eq!(
        (
            session.head_tick(),
            session.verified_tick(),
            session.next_send_tick()
        ),
        (4, 4, 5)
    );
    assert_eq!(tail(&session), authored);
}

#[test]
fn pregrant_defaults_and_initial_delay_are_verified_without_any_remote_admission() {
    for initial in [0, 4] {
        let (old, mut session) = host(2, limits());
        advance_verified(&old, &mut session, initial);
        let ticket = bind_grant(&old, &mut session).ticket();
        let target = ticket.first_input_tick - 1;
        assert_eq!(target, initial + 2);
        assert_eq!(session.last_remote_tick(JOINER), None);
        assert_eq!(old.fence_host_continuation(&session, OLD_CONN), Ok(target));
        assert_eq!(old.verify_host_continuation(&mut session), Ok(target));
        let fresh = old
            .replace_fenced_host_rolling(&mut session, context(OLD_ID + 1))
            .unwrap();
        assert_eq!(fresh.pending(), (0, 0));
        session.advance(input(target + 1, HOST), vec![]);
        session.poll_confirmed();
        fresh.check().unwrap();
        assert_eq!(session.verified_tick(), target + 1);
        assert_eq!(
            session
                .authored_since(target)
                .iter()
                .map(|r| (r.tick, r.slot))
                .collect::<Vec<_>>(),
            vec![(target + 1, HOST), (target + 1, JOINER)]
        );
    }
}

#[test]
fn an_interior_gap_is_not_hidden_by_a_later_nonempty_command_record() {
    let (old, mut session) = host(0, limits());
    advance_verified(&old, &mut session, 16);
    bind_grant(&old, &mut session);
    for _ in 0..4 {
        session.advance(input(session.next_send_tick(), HOST), vec![]);
    }
    admit(&old, 17, false);
    session.poll_confirmed();
    assert_eq!(session.verified_tick(), 17);
    admit(&old, 19, true);
    assert_eq!(session.last_remote_tick(JOINER), Some(17));
    let before = world(&session);
    let retained = old.retained_evidence();
    assert_eq!(
        old.fence_host_continuation(&session, OLD_CONN),
        Err(Error::MissingHistory {
            tick: 18,
            slot: JOINER
        })
    );
    assert_eq!(world(&session), before);
    assert_eq!(old.retained_evidence(), retained);
    assert_fenced_nonclearing(&old);
    rejected_replacement(
        &old,
        &mut session,
        context(OLD_ID + 1),
        Error::UnsafeReplacement,
    );
    let admitted = session.source_mut().poll_remote();
    assert_eq!(admitted.len(), 1);
    assert_eq!(admitted[0].tick, 19);
    assert_eq!(admitted[0].commands, commands(JOINER));
    assert!(session.authored_since(17).iter().all(|r| r.slot == HOST));
}

#[test]
fn accepted_target_beyond_locally_authored_horizon_is_rejected_without_filling() {
    let (old, mut session) = host(0, limits());
    advance_verified(&old, &mut session, 16);
    bind_grant(&old, &mut session);
    session.advance(input(17, HOST), vec![]);
    admit(&old, 17, false);
    admit(&old, 18, true);
    let before = world(&session);
    assert_eq!(session.next_send_tick(), 18);
    assert_eq!(
        old.fence_host_continuation(&session, OLD_CONN),
        Err(Error::TickRange)
    );
    assert_eq!(world(&session), before);
    assert_fenced_nonclearing(&old);
    rejected_replacement(
        &old,
        &mut session,
        context(OLD_ID + 1),
        Error::UnsafeReplacement,
    );
    assert_eq!(
        session
            .source_mut()
            .poll_remote()
            .iter()
            .map(|r| r.tick)
            .collect::<Vec<_>>(),
        vec![17, 18]
    );
}

#[test]
fn cancelled_and_terminal_sources_cannot_be_fenced_or_replaced() {
    for terminal in 0..3 {
        let (old, mut session) = host(0, limits());
        bind_grant(&old, &mut session);
        let expected = match terminal {
            0 => {
                old.cancel();
                Error::Cancelled
            }
            1 => old.disconnected(OLD_CONN).unwrap_err(),
            _ => old
                .receive(OLD_CONN, Channel::Unreliable, b"bad")
                .unwrap_err(),
        };
        old.cancel();
        assert_eq!(
            old.fence_host_continuation(&session, OLD_CONN),
            Err(expected.clone())
        );
        assert_eq!(
            old.verify_host_continuation(&mut session),
            Err(expected.clone())
        );
        rejected_replacement(&old, &mut session, context(OLD_ID + 1), expected);
    }
}

#[test]
fn fence_requires_exact_source_connection_rolling_mode_and_host_authority() {
    let (old, mut session) = host(0, limits());
    bind_grant(&old, &mut session);
    assert_eq!(
        old.fence_host_continuation(&session, NEW_CONN),
        Err(Error::UnsafeReplacement)
    );
    assert_eq!(old.check(), Ok(()));
    let (foreign, mut other) = host(0, limits());
    bind_grant(&foreign, &mut other);
    assert_eq!(
        foreign.fence_host_continuation(&session, OLD_CONN),
        Err(Error::UnsafeReplacement)
    );
    assert_eq!(foreign.check(), Ok(()));
    for (driver_slot, session_slot, relay, finite) in [
        (JOINER, JOINER, false, false),
        (HOST, JOINER, false, false),
        (HOST, HOST, true, false),
        (HOST, HOST, false, true),
    ] {
        let (driver, source) = if finite {
            Driver::new(driver_slot, context(OLD_ID), limits()).unwrap()
        } else {
            Driver::new_rolling(driver_slot, context(OLD_ID), limits(), window()).unwrap()
        };
        driver.bind(OLD_CONN, &context(OLD_ID), 0, 1).unwrap();
        let mut cfg = config(session_slot, 0);
        cfg.relay = relay;
        let session = Peer::new(ArenaConfig { player_count: 2 }, cfg, source);
        assert_eq!(
            driver.fence_host_continuation(&session, OLD_CONN),
            Err(Error::UnsafeReplacement)
        );
        assert_eq!(driver.check(), Ok(()));
    }
    let (unbound, session) = host(0, limits());
    assert_eq!(
        unbound.fence_host_continuation(&session, OLD_CONN),
        Err(Error::UnsafeReplacement)
    );
}

fn verified_fence(bounds: P2pInputLimits, tail_ticks: usize) -> (Driver, Peer) {
    let (old, mut session) = host(0, bounds);
    advance_verified(&old, &mut session, 16);
    bind_grant(&old, &mut session);
    for _ in 0..=tail_ticks {
        session.advance(input(session.next_send_tick(), HOST), vec![]);
        old.check().unwrap();
    }
    admit(&old, 17, false);
    assert_eq!(old.fence_host_continuation(&session, OLD_CONN), Ok(17));
    assert_eq!(old.verify_host_continuation(&mut session), Ok(17));
    (old, session)
}

#[test]
fn fresh_generation_must_strictly_increase_and_keep_the_same_roster() {
    let (old, mut session) = verified_fence(limits(), 0);
    for ctx in [
        context(OLD_ID - 1),
        context(OLD_ID),
        CheckedJoinContext::new(OLD_ID, 2, context(OLD_ID).roster().clone()).unwrap(),
        CheckedJoinContext::new(
            OLD_ID + 1,
            1,
            JoinRoster::completed(3, JOINER, HOST, vec![HOST], &[PlayerSlot(2)]).unwrap(),
        )
        .unwrap(),
    ] {
        rejected_replacement(&old, &mut session, ctx, Error::UnsafeReplacement);
    }
    let fresh = old
        .replace_fenced_host_rolling(&mut session, context(OLD_ID + 1))
        .unwrap();
    fresh.check().unwrap();
    assert_eq!(old.check(), Err(Error::Cancelled));
}

#[test]
fn replacement_record_capacity_reserves_every_default_before_installation() {
    for capacity in [5, 6] {
        assert_capacity(
            P2pInputLimits {
                max_records: capacity,
                ..limits()
            },
            (capacity == 5).then_some(Error::RecordLimit),
        );
    }
}

#[test]
fn replacement_byte_capacity_reserves_every_default_before_installation() {
    for bytes in [6 * RECORD_BYTES - 1, 6 * RECORD_BYTES] {
        assert_capacity(
            P2pInputLimits {
                max_retained_bytes: bytes,
                ..limits()
            },
            (bytes < 6 * RECORD_BYTES).then_some(Error::ByteLimit),
        );
    }
}

fn assert_capacity(bounds: P2pInputLimits, expected: Option<Error>) {
    let (old, mut session) = verified_fence(bounds, 3);
    assert_eq!(session.next_send_tick(), 21);
    if let Some(error) = expected {
        rejected_replacement(&old, &mut session, context(OLD_ID + 1), error);
        assert_eq!(old.verify_host_continuation(&mut session), Ok(17));
    } else {
        let before = world(&session);
        let fresh = old
            .replace_fenced_host_rolling(&mut session, context(OLD_ID + 1))
            .unwrap();
        assert_eq!(world(&session), before);
        assert_eq!(fresh.pending(), (6, 6 * RECORD_BYTES));
        assert_eq!(fresh.retained_evidence(), (6, 6 * RECORD_BYTES));
        session.poll_confirmed();
        fresh.check().unwrap();
        assert_eq!(session.verified_tick(), session.head_tick());
    }
}

#[test]
fn failed_departure_commit_restores_the_original_fenced_source() {
    let (old, mut session) = verified_fence(limits(), 0);
    assert_eq!(session.next_send_tick(), 18);
    // Publicly reachable duplicate departure: this empty suffix emits nothing,
    // so it does not poison the fenced source before the barrier checks vacancy.
    session.mark_slot_vacant(JOINER, 18).unwrap();
    old.check().unwrap();
    rejected_replacement(
        &old,
        &mut session,
        context(OLD_ID + 1),
        Error::UnsafeReplacement,
    );
    assert_eq!(
        old.verify_host_continuation(&mut session),
        Ok(17),
        "the exact original source is restored after failed commit"
    );
    assert_fenced_nonclearing(&old);
}

#[test]
fn authoring_after_the_fence_cannot_turn_into_a_fresh_continuation() {
    let (old, mut session) = verified_fence(limits(), 0);
    session.advance(input(18, HOST), commands(HOST));
    assert_eq!(old.check(), Err(Error::Fenced));
    assert_eq!(
        old.verify_host_continuation(&mut session),
        Err(Error::Fenced)
    );
    rejected_replacement(&old, &mut session, context(OLD_ID + 1), Error::Fenced);
}

#[test]
fn rolling_horizon_overflow_while_verifying_is_terminal_without_wrapping() {
    // A validated high-tick snapshot exercises arithmetic through real Session
    // callbacks, like p2p_rolling's public snapshot-loader boundary fixture.
    let snapshot = u64::MAX - 3;
    let mut simulation = Simulation::<Arena>::new(ArenaConfig { player_count: 2 }, 60, 42);
    simulation.frame_mut().set_tick(snapshot);
    let (old, source) = Driver::new_rolling(
        HOST,
        context(OLD_ID),
        limits(),
        P2pRollingWindow {
            recent_ticks: 1,
            future_ticks: 1,
        },
    )
    .unwrap();
    let mut cfg = config(HOST, 0);
    cfg.vacant_slots.push(JOINER);
    let mut session = Peer::from_relay_snapshot(
        ArenaConfig { player_count: 2 },
        cfg,
        source,
        snapshot,
        simulation.frame().checksum(),
        &simulation.frame().to_bytes(),
    )
    .unwrap_or_else(|(error, _)| panic!("arithmetic fixture snapshot rejected: {error}"));
    bind_grant(&old, &mut session);
    session.advance(ArenaInput::default(), vec![]);
    old.check().unwrap();
    admit(&old, snapshot + 1, false);
    assert_eq!(
        old.fence_host_continuation(&session, OLD_CONN),
        Ok(snapshot + 1)
    );
    assert_eq!(
        old.verify_host_continuation(&mut session),
        Err(Error::TickExhausted)
    );
    assert_eq!(session.next_send_tick(), u64::MAX - 1);
    assert_eq!(old.check(), Err(Error::TickExhausted));
    rejected_replacement(
        &old,
        &mut session,
        context(OLD_ID + 1),
        Error::TickExhausted,
    );
}

#[test]
fn genuinely_pruned_host_tail_is_not_reconstructed_after_adversarial_restore() {
    let (old, mut session) = verified_fence(limits(), 5);
    let checkpoint = session.verified_frame().unwrap().to_bytes();
    let checksum = session.verified_frame().unwrap().checksum();
    let next = session.next_send_tick();
    assert_eq!((session.verified_tick(), next), (17, 23));

    // Deliberately unsupported caller misuse: temporarily move the original
    // fenced source out, verify the already-authored suffix through another real
    // source, then restore the old checkpoint and source. This exposes genuinely
    // pruned Session history without synthetic callbacks or private mutation.
    let (other, source) = Driver::new_rolling(HOST, context(OLD_ID), limits(), window()).unwrap();
    other.bind(OLD_CONN, &context(OLD_ID), 17, 18).unwrap();
    let original_source = std::mem::replace(session.source_mut(), source);
    for tick in 18..next {
        admit(&other, tick, false);
    }
    session.poll_confirmed();
    other.check().unwrap();
    assert_eq!(session.verified_tick(), next - 1);
    assert_eq!(
        session
            .authored_since(17)
            .iter()
            .map(|r| r.tick)
            .collect::<Vec<_>>(),
        vec![21, 22]
    );
    other.cancel();
    session
        .restore_confirmed(17, checksum, &checkpoint, 18)
        .unwrap();
    *session.source_mut() = original_source;
    assert_eq!(session.next_send_tick(), next);
    assert_eq!(old.rolling_progress().unwrap().0, 17);
    rejected_replacement(
        &old,
        &mut session,
        context(OLD_ID + 1),
        Error::MissingHistory {
            tick: 18,
            slot: HOST,
        },
    );
}

#[test]
fn a_second_generation_can_continue_before_receiving_new_remote_input() {
    let (old, mut session) = verified_fence(limits(), 2);
    let second_ctx = context(OLD_ID + 1);
    let second = old
        .replace_fenced_host_rolling(&mut session, second_ctx.clone())
        .unwrap();
    assert_eq!(session.last_remote_tick(JOINER), Some(17));
    let ticket = grant(&mut session, &second_ctx).ticket();
    second
        .bind(
            NEW_CONN,
            &second_ctx,
            ticket.snapshot_tick,
            ticket.first_input_tick,
        )
        .unwrap();
    assert_eq!((ticket.snapshot_tick, ticket.first_input_tick), (17, 20));
    // The maximum retained by Session belongs entirely to the prior generation.
    // The fresh source owns defaults up to the new grant's boundary.
    assert_eq!(second.fence_host_continuation(&session, NEW_CONN), Ok(19));
    assert_eq!(second.verify_host_continuation(&mut session), Ok(19));
    let before = world(&session);
    let third = second
        .replace_fenced_host_rolling(&mut session, context(OLD_ID + 2))
        .unwrap();
    assert_eq!(world(&session), before);
    assert_eq!(third.rolling_progress(), Some((19, 19)));
    assert_eq!(third.pending(), (0, 0));
    assert_eq!(session.last_remote_tick(JOINER), Some(17));
    assert_eq!(second.check(), Err(Error::Cancelled));
    third.check().unwrap();
}

#[test]
fn preactivation_retry_after_established_continuation_tolerates_verified_old_remote_history() {
    let (old, mut session) = verified_fence(limits(), 2);
    let second_ctx = context(OLD_ID + 1);
    let second = old
        .replace_fenced_host_rolling(&mut session, second_ctx.clone())
        .unwrap();
    session.poll_confirmed();
    second.check().unwrap();
    assert_eq!(session.verified_tick(), 19);
    assert_eq!(session.last_remote_tick(JOINER), Some(17));
    let ticket = grant(&mut session, &second_ctx).ticket();
    second
        .bind(
            NEW_CONN,
            &second_ctx,
            ticket.snapshot_tick,
            ticket.first_input_tick,
        )
        .unwrap();
    let lease = session.join_hold_lease(JOINER).unwrap();
    let _ = session.release_owned_join_hold(&lease);
    second.cancel();
    let before = world(&session);
    let authored = tail(&session);
    let third = second
        .replace_cancelled_host_rolling(&mut session, context(OLD_ID + 2))
        .unwrap();
    assert_eq!(world(&session), before);
    assert_eq!(tail(&session), authored);
    session
        .mark_slot_vacant(JOINER, ticket.first_input_tick)
        .unwrap();
    third.check().unwrap();
    assert_eq!(third.rolling_progress(), Some((19, 19)));
    assert_eq!(second.check(), Err(Error::Cancelled));
    session.advance(input(20, HOST), vec![]);
    session.poll_confirmed();
    third.check().unwrap();
    assert_eq!(session.verified_tick(), 20);
}

#[test]
fn cancelled_retry_still_rejects_remote_history_above_a_restored_verified_checkpoint() {
    let (old, mut session) = verified_fence(limits(), 0);
    let earlier = session
        .anchors()
        .iter()
        .find(|anchor| anchor.tick == 16)
        .unwrap()
        .clone();
    let second_ctx = context(OLD_ID + 1);
    let second = old
        .replace_fenced_host_rolling(&mut session, second_ctx.clone())
        .unwrap();
    let ticket = grant(&mut session, &second_ctx).ticket();
    second
        .bind(
            NEW_CONN,
            &second_ctx,
            ticket.snapshot_tick,
            ticket.first_input_tick,
        )
        .unwrap();
    let lease = session.join_hold_lease(JOINER).unwrap();
    let _ = session.release_owned_join_hold(&lease);
    second.cancel();
    session
        .restore_confirmed(earlier.tick, earlier.checksum, &earlier.frame_bytes, 17)
        .unwrap();
    assert_eq!(session.verified_tick(), 16);
    assert_eq!(session.last_remote_tick(JOINER), Some(17));
    let before = world(&session);
    let authored = tail(&session);
    for _ in 0..2 {
        assert_eq!(
            second
                .replace_cancelled_host_rolling(&mut session, context(OLD_ID + 2))
                .err(),
            Some(Error::UnsafeReplacement)
        );
        assert_eq!(world(&session), before);
        assert_eq!(tail(&session), authored);
        assert_eq!(second.check(), Err(Error::Cancelled));
    }
}

#[test]
fn cancelled_retry_rejects_current_generation_admission_even_after_full_verification() {
    let (old, mut session) = host(0, limits());
    advance_verified(&old, &mut session, 16);
    bind_grant(&old, &mut session);
    session.advance(input(17, HOST), commands(HOST));
    admit(&old, 17, true);
    session.poll_confirmed();
    old.check().unwrap();
    assert_eq!((session.head_tick(), session.verified_tick()), (17, 17));
    assert_eq!(session.last_remote_tick(JOINER), Some(17));
    assert!(session.pending_join(JOINER).is_none());
    old.cancel();
    let before = world(&session);
    let authored = tail(&session);
    // Unlike history from an older generation, this driver's actual admission
    // permanently rules out the pre-activation retry path, even after verification.
    for _ in 0..2 {
        assert_eq!(
            old.replace_cancelled_host_rolling(&mut session, context(OLD_ID + 1))
                .err(),
            Some(Error::UnsafeReplacement)
        );
        assert_eq!(world(&session), before);
        assert_eq!(tail(&session), authored);
        assert_eq!(old.check(), Err(Error::Cancelled));
        assert_eq!(old.pending(), (0, 0));
        assert_eq!(old.retained_evidence(), (0, 0));
    }
}
