//! Fixed-mesh identity, canonical evidence, destination and cutoff regressions.
use std::cell::Cell;

use orr_net::SendError;
use orr_proto::{Channel, ConnId};
use orr_relay_net::p2p_mesh_input::{
    P2pMeshDelivery as Delivery, P2pMeshEdge as Edge, P2pMeshInputAccepted as Accepted,
    P2pMeshInputDriver, P2pMeshInputError as Error, P2pMeshInputLimits as Limits,
    P2pMeshInputSource,
};
use orr_relay_net::P2pInputCodec;
use orr_session::{
    checked_backlog_notice, serve_checked_join, CheckedJoinContext, CheckedJoinTicket, InputSource,
    JoinBootstrap, JoinRoster, LocalInputSource, PlayerSlot, RemoteInput, Session, SessionConfig,
};
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
fn new(slot: u8, limits: Limits) -> (Driver, Source) {
    Driver::new(PlayerSlot(slot), context(), limits).unwrap()
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
fn grant() -> (CheckedJoinTicket, Vec<u8>, Bootstrap) {
    let mut b = Bootstrap::new(cfg(2), roster(), 4096, 1 << 20).unwrap();
    let request = b.next_request().unwrap();
    let mut c = cfg(0);
    c.vacant_slots = vec![PlayerSlot(2)];
    let mut donor = Session::<Arena, _>::new(game(), c, LocalInputSource);
    for _ in 0..4 {
        donor.advance(ArenaInput::default(), vec![]);
    }
    let (snapshot, ticket) = serve_checked_join(&mut donor, &context(), &request).unwrap();
    assert_eq!(ticket.ticket().first_input_tick, 5);
    (ticket, snapshot, b)
}
fn joined(slot: u8, limits: Limits) -> (Driver, Source) {
    assert!(slot < 2);
    let (d, s) = new(slot, limits);
    let (ticket, _, _) = grant();
    d.install_ticket(&ticket).unwrap();
    (d, s)
}
fn with_two_edges(limits: Limits) -> (Driver, Source) {
    let (d, s) = joined(0, limits);
    d.admit_edge(edge(1)).unwrap();
    d.admit_edge(edge(2)).unwrap();
    (d, s)
}
fn packet(slot: u8, tick: u64, owners: &[u32]) -> Vec<u8> {
    let (d, _) = new(slot, Limits::default());
    d.admit_edge(edge(1 - slot)).unwrap();
    d.send_local(record(tick, slot, owners)).unwrap();
    let mut out = vec![];
    d.flush(|_, b| {
        out = b.to_vec();
        Ok(())
    })
    .unwrap();
    out
}

#[test]
fn distinct_wire_and_repeated_commands_are_canonical() {
    let p = packet(0, 1, &[0, 0, 1]);
    assert_eq!(&p[..6], b"ORRM\x01\x00");
    assert_eq!(p.len(), 94);
    let (d, mut s) = new(1, Limits::default());
    d.admit_edge(edge(0)).unwrap();
    assert_eq!(
        d.receive(edge(0).connection, Channel::Reliable, &p),
        Ok(Accepted::New)
    );
    assert_eq!(
        d.receive(edge(0).connection, Channel::Reliable, &p),
        Ok(Accepted::Duplicate)
    );
    let r = s.poll_remote();
    assert_eq!(r.len(), 1);
    assert_eq!(
        r[0].commands.iter().map(|c| c.owner).collect::<Vec<_>>(),
        [0, 0, 1]
    );
    assert_eq!(d.incoming(), (0, 0));
    assert_eq!(d.retained_evidence().0, 1);
}

#[test]
fn raw_connection_one_is_scoped_by_adapter_and_admission() {
    let (d, _) = with_two_edges(Limits::default());
    assert_eq!(
        d.resolve_connection(edge(1).adapter_id, ConnId(1)).unwrap(),
        edge(1).connection
    );
    assert_eq!(
        d.resolve_connection(edge(2).adapter_id, ConnId(1)).unwrap(),
        edge(2).connection
    );
    assert_eq!(
        d.resolve_connection(999, ConnId(1)),
        Err(Error::WrongConnection)
    );
    assert!(d.check().is_ok());
    let p = packet(1, 1, &[]);
    assert_eq!(
        d.receive(ConnId(1), Channel::Reliable, &p),
        Err(Error::WrongConnection)
    );
    assert_eq!(d.check(), Err(Error::WrongConnection));
}

#[test]
fn envelope_rejection_precedes_application_decode() {
    for (offset, value, error) in [
        (0, b'I', Error::Malformed),
        (4, 2, Error::Malformed),
        (6, 0, Error::WrongSchema),
        (14, 18, Error::WrongContext),
        (22, 2, Error::WrongContext),
        (26, 78, Error::WrongGeneration),
        (34, 2, Error::WrongRecipient),
        (43, 1, Error::WrongAuthority),
    ] {
        let mut p = packet(0, 1, &[]);
        p[offset] = value;
        let (d, _) = new(1, Limits::default());
        d.admit_edge(edge(0)).unwrap();
        DECODE_COUNT.with(|c| c.set(0));
        assert_eq!(
            d.receive(edge(0).connection, Channel::Reliable, &p),
            Err(error.clone())
        );
        assert_eq!(d.check(), Err(error));
        DECODE_COUNT.with(|c| assert_eq!(c.get(), 0));
        assert_eq!(d.retained_evidence(), (0, 0));
    }
    let p = packet(0, 1, &[]);
    let (d, _) = new(1, Limits::default());
    d.admit_edge(edge(0)).unwrap();
    assert_eq!(
        d.receive(edge(0).connection, Channel::Unreliable, &p),
        Err(Error::WrongChannel)
    );
}

#[test]
fn conflicting_duplicate_is_terminal_and_does_not_replace_evidence() {
    let p = packet(0, 1, &[0]);
    let mut bad = p.clone();
    *bad.last_mut().unwrap() = 1;
    let (d, mut s) = new(1, Limits::default());
    d.admit_edge(edge(0)).unwrap();
    d.receive(edge(0).connection, Channel::Reliable, &p)
        .unwrap();
    let before = (d.incoming(), d.retained_evidence());
    let e = Error::ConflictingDuplicate {
        tick: 1,
        slot: PlayerSlot(0),
    };
    assert_eq!(
        d.receive(edge(0).connection, Channel::Reliable, &bad),
        Err(e.clone())
    );
    assert_eq!(d.check(), Err(e));
    assert!(s.poll_remote().is_empty());
    assert_eq!((d.incoming(), d.retained_evidence()), before);
}

#[test]
fn fanout_backpressure_preserves_fifo_without_resending_successful_edge() {
    let (d, _) = with_two_edges(Limits::default());
    for tick in 1..=3 {
        d.send_local(record(tick, 0, &[])).unwrap();
    }
    assert_eq!(d.pending().0, 6);
    let mut good = vec![];
    let first = d
        .flush(|id, b| {
            if id == edge(1).connection {
                return Err(SendError::Backpressure);
            }
            good.push(u64::from_le_bytes(b[35..43].try_into().unwrap()));
            Ok(())
        })
        .unwrap();
    assert_eq!(first.sent, 3);
    assert_eq!(first.pending, 3);
    assert_eq!(first.backpressured_edges, 1);
    assert_eq!(good, [1, 2, 3]);
    for tick in 1..=3 {
        assert_eq!(
            d.delivery(edge(1).connection, tick, PlayerSlot(0)),
            Some(Delivery::Pending)
        );
        assert_eq!(
            d.delivery(edge(2).connection, tick, PlayerSlot(0)),
            Some(Delivery::QueueAccepted)
        );
    }
    let mut retried = vec![];
    d.flush(|id, b| {
        assert_eq!(id, edge(1).connection);
        retried.push(u64::from_le_bytes(b[35..43].try_into().unwrap()));
        Ok(())
    })
    .unwrap();
    assert_eq!(retried, [1, 2, 3]);
    assert_eq!(d.pending(), (0, 0));
    assert_eq!(d.destination_records(), 6);
    assert_eq!(d.flush(|_, _| panic!("already accepted")).unwrap().sent, 0);
}

#[test]
fn fanout_record_and_byte_budgets_preflight_every_destination() {
    for limits in [
        Limits {
            max_destination_records: 3,
            ..Limits::default()
        },
        Limits {
            max_pending_encoded_bytes: 279,
            ..Limits::default()
        },
        Limits {
            max_edge_pending_records: 1,
            ..Limits::default()
        },
        Limits {
            max_edge_pending_encoded_bytes: 139,
            ..Limits::default()
        },
    ] {
        let (d, _) = with_two_edges(limits);
        d.send_local(record(1, 0, &[])).unwrap();
        let before = (d.pending(), d.retained_evidence(), d.destination_records());
        assert!(matches!(
            d.send_local(record(2, 0, &[])),
            Err(Error::RecordLimit | Error::ByteLimit)
        ));
        assert_eq!(
            (d.pending(), d.retained_evidence(), d.destination_records()),
            before
        );
        assert_eq!(d.delivery(edge(1).connection, 2, PlayerSlot(0)), None);
        assert_eq!(d.delivery(edge(2).connection, 2, PlayerSlot(0)), None);
    }
}

#[test]
fn new_destination_gets_already_accepted_logical_records_once() {
    let (d, _) = new(0, Limits::default());
    d.admit_edge(edge(1)).unwrap();
    for tick in 1..=4 {
        d.send_local(record(tick, 0, &[])).unwrap();
        d.send_local(record(tick, 2, &[])).unwrap();
    }
    d.flush(|_, _| Ok(())).unwrap();
    assert_eq!(d.pending().0, 0);
    let before = d.retained_evidence();
    let (ticket, _, _) = grant();
    d.install_ticket(&ticket).unwrap();
    d.admit_edge(edge(2)).unwrap();
    assert_eq!(d.pending().0, 8);
    assert_eq!(d.retained_evidence(), before);
    assert_eq!(d.destination_records(), 16);
    // An exact repeat neither duplicates the new destination nor requeues peer 1.
    d.send_local(record(1, 0, &[])).unwrap();
    assert_eq!(d.pending().0, 8);
    d.flush(|id, _| {
        assert_eq!(id, edge(2).connection);
        Ok(())
    })
    .unwrap();
    assert_eq!(d.pending().0, 0);
}

#[test]
fn admission_backfill_failure_is_transactional() {
    let (d, _) = joined(
        0,
        Limits {
            max_destination_records: 1,
            ..Limits::default()
        },
    );
    d.admit_edge(edge(1)).unwrap();
    d.send_local(record(1, 0, &[])).unwrap();
    d.flush(|_, _| Ok(())).unwrap();
    let before = (d.pending(), d.retained_evidence(), d.destination_records());
    assert_eq!(d.admit_edge(edge(2)), Err(Error::RecordLimit));
    assert_eq!(
        (d.pending(), d.retained_evidence(), d.destination_records()),
        before
    );
    assert_eq!(d.delivery(edge(2).connection, 1, PlayerSlot(0)), None);
}

#[test]
fn exact_cutoff_and_default_only_vacancy_authority() {
    let (d, _) = joined(0, Limits::default());
    d.send_local(record(4, 2, &[])).unwrap();
    assert_eq!(d.send_local(record(5, 2, &[])), Err(Error::WrongAuthority));
    for bad in [record(1, 2, &[0]), {
        let mut r = record(1, 2, &[]);
        r.input.buttons = 1;
        r
    }] {
        let (d, _) = new(0, Limits::default());
        assert_eq!(d.send_local(bad), Err(Error::InvalidVacancy));
    }
    let (d, _) = new(1, Limits::default());
    assert_eq!(d.send_local(record(1, 2, &[])), Err(Error::WrongAuthority));
    let (d, _) = new(2, Limits::default());
    assert_eq!(d.send_local(record(5, 2, &[])), Err(Error::WrongAuthority));
    let (ticket, snapshot, mut b) = grant();
    let (d, s) = new(2, Limits::default());
    b.receive_snapshot(PlayerSlot(0), game(), s, &snapshot)
        .unwrap();
    d.install_joiner_snapshot(&b).unwrap();
    d.send_local(record(ticket.ticket().first_input_tick, 2, &[2, 2]))
        .unwrap();
    assert_eq!(d.send_local(record(4, 2, &[])), Err(Error::WrongAuthority));
}

#[test]
fn checked_cutoff_cannot_bless_previously_authored_post_cutoff_defaults() {
    let (d, _) = new(0, Limits::default());
    d.send_local(record(5, 2, &[])).unwrap();
    let (ticket, _, _) = grant();
    assert_eq!(d.install_ticket(&ticket), Err(Error::WrongAuthority));
    let (d, _) = new(0, Limits::default());
    let mut invalid = edge(2);
    invalid.generation = 0;
    assert_eq!(d.admit_edge(invalid), Err(Error::InvalidEdge));
    let (d, _) = new(0, Limits::default());
    assert_eq!(d.admit_edge(edge(2)), Err(Error::NotBound));
}

#[test]
fn snapshot_binding_requires_exact_checked_bootstrap_source() {
    let (_, snapshot, mut b) = grant();
    let (good, s) = new(2, Limits::default());
    b.receive_snapshot(PlayerSlot(0), game(), s, &snapshot)
        .unwrap();
    let (foreign, _) = new(2, Limits::default());
    assert_eq!(
        foreign.install_joiner_snapshot(&b),
        Err(Error::InvalidSnapshot)
    );
    good.install_joiner_snapshot(&b).unwrap();
    good.admit_edge(edge(0)).unwrap();
    let mut p = packet(0, 1, &[]);
    p[34] = 2;
    assert_eq!(
        good.receive(edge(0).connection, Channel::Reliable, &p),
        Ok(Accepted::New)
    );
}

#[test]
fn third_party_forwarding_and_disconnected_records_are_rejected() {
    let (d, _) = with_two_edges(Limits::default());
    let mut p = packet(1, 6, &[]);
    p[43] = 2;
    assert_eq!(
        d.receive(edge(1).connection, Channel::Reliable, &p),
        Err(Error::WrongAuthority)
    );
    let (d, _) = new(0, Limits::default());
    let mut r = record(1, 0, &[]);
    r.disconnected = true;
    assert_eq!(d.send_local(r), Err(Error::WrongAuthority));
}

#[test]
fn full_finite_horizon_retains_evidence_and_rejects_tick_513() {
    let (d, _) = new(0, Limits::default());
    d.admit_edge(edge(1)).unwrap();
    for tick in 1..=512 {
        d.send_local(record(tick, 0, &[])).unwrap();
        d.send_local(record(tick, 2, &[])).unwrap();
    }
    assert_eq!(d.retained_evidence().0, 1024);
    assert_eq!(d.pending().0, 1024);
    d.flush(|_, _| Ok(())).unwrap();
    assert_eq!(d.retained_evidence().0, 1024);
    assert_eq!(d.send_local(record(513, 0, &[])), Err(Error::TickRange));
}

#[test]
fn truncation_trailing_bytes_command_and_incoming_budgets_fail_closed() {
    let p = packet(0, 1, &[0, 0, 1]);
    for n in 0..p.len() {
        let (d, _) = new(1, Limits::default());
        d.admit_edge(edge(0)).unwrap();
        assert!(d
            .receive(edge(0).connection, Channel::Reliable, &p[..n])
            .is_err());
        assert_eq!(d.retained_evidence().0, 0);
    }
    let mut trailing = p.clone();
    trailing.push(0);
    let (d, _) = new(1, Limits::default());
    d.admit_edge(edge(0)).unwrap();
    assert_eq!(
        d.receive(edge(0).connection, Channel::Reliable, &trailing),
        Err(Error::Malformed)
    );
    let (d, _) = new(0, Limits::default());
    assert_eq!(
        d.send_local(record(1, 0, &[0, 1, 2, 0])),
        Err(Error::CommandLimit)
    );
    for limits in [
        Limits {
            max_incoming_records: 1,
            ..Limits::default()
        },
        Limits {
            max_incoming_encoded_bytes: 139,
            ..Limits::default()
        },
    ] {
        let (d, _) = new(1, limits);
        d.admit_edge(edge(0)).unwrap();
        d.receive(edge(0).connection, Channel::Reliable, &packet(0, 1, &[]))
            .unwrap();
        let before = (d.incoming(), d.retained_evidence());
        assert!(matches!(
            d.receive(edge(0).connection, Channel::Reliable, &packet(0, 2, &[])),
            Err(Error::RecordLimit | Error::ByteLimit)
        ));
        assert_eq!((d.incoming(), d.retained_evidence()), before);
    }
}

#[test]
fn initial_flush_budget_and_nested_flush_are_bounded() {
    let (d, _) = with_two_edges(Limits::default());
    d.send_local(record(1, 0, &[])).unwrap();
    let mut appended = false;
    let out = d
        .flush(|_, _| {
            if !appended {
                appended = true;
                d.send_local(record(2, 0, &[])).unwrap();
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(out.sent, 2);
    assert_eq!(out.pending, 2);
    assert_eq!(
        d.flush(|_, _| {
            assert_eq!(d.flush(|_, _| Ok(())), Err(Error::ReentrantFlush));
            Ok(())
        }),
        Err(Error::ReentrantFlush)
    );
}

#[test]
fn edge_failure_aborts_whole_mesh_without_default_takeover() {
    let (d, mut s) = with_two_edges(Limits::default());
    d.send_local(record(1, 0, &[])).unwrap();
    assert_eq!(d.disconnected(edge(1).connection), Err(Error::Disconnected));
    assert_eq!(
        d.flush(|_, _| panic!("failed mesh cannot send")),
        Err(Error::Disconnected)
    );
    assert!(s.poll_remote().is_empty());
    assert_eq!(d.send_local(record(2, 2, &[])), Err(Error::Disconnected));
}

fn transfer(from: &Driver, to: &Driver, remote: u8) {
    from.flush(|_, bytes| {
        to.receive(edge(remote).connection, Channel::Reliable, bytes)
            .unwrap();
        Ok(())
    })
    .unwrap();
}
#[test]
fn live_sessions_snapshot_backfill_and_three_peer_checksums_match() {
    let (a, sa) = new(0, Limits::default());
    let (b, sb) = new(1, Limits::default());
    a.admit_edge(edge(1)).unwrap();
    b.admit_edge(edge(0)).unwrap();
    let mut ca = cfg(0);
    ca.vacant_slots = vec![PlayerSlot(2)];
    let mut pa = Session::<Arena, _>::new(game(), ca, sa);
    let mut pb = Session::<Arena, _>::new(game(), cfg(1), sb);
    for _ in 0..12 {
        pa.advance(ArenaInput::default(), vec![]);
        pb.advance(ArenaInput::default(), vec![]);
        transfer(&a, &b, 0);
        transfer(&b, &a, 1);
        pa.step();
        pb.step();
        a.check().unwrap();
        b.check().unwrap();
    }
    let old_obligations = a.destination_records();
    let old_evidence = a.retained_evidence();
    let mut boot = Bootstrap::new(cfg(2), roster(), 4096, 1 << 20).unwrap();
    let req = boot.next_request().unwrap();
    let (snapshot, ticket) = serve_checked_join(&mut pa, &context(), &req).unwrap();
    assert!(ticket.ticket().snapshot_tick > 0);
    let notices = [
        checked_backlog_notice(&mut pa, &context(), &ticket).unwrap(),
        checked_backlog_notice(&mut pb, &context(), &ticket).unwrap(),
    ];
    a.install_ticket(&ticket).unwrap();
    b.install_ticket(&ticket).unwrap();
    // New edge excludes snapshot-contained records but preserves old-edge state/evidence.
    a.admit_edge(edge(2)).unwrap();
    b.admit_edge(edge(2)).unwrap();
    assert_eq!(a.retained_evidence(), old_evidence);
    assert!(a.destination_records() >= old_obligations);
    let (c, sc) = new(2, Limits::default());
    boot.receive_snapshot(PlayerSlot(0), game(), sc, &snapshot)
        .unwrap();
    c.install_joiner_snapshot(&boot).unwrap();
    c.admit_edge(edge(0)).unwrap();
    c.admit_edge(edge(1)).unwrap();
    for (slot, n) in notices.iter().enumerate() {
        boot.receive_notice(PlayerSlot(slot as u8), n).unwrap();
    }
    let first = ticket.ticket().first_input_tick;
    for tick in first..=100 {
        for p in [&mut pa, &mut pb] {
            p.advance(ArenaInput::default(), vec![]);
        }
        boot.advance(ArenaInput::default(), vec![]);
        for (local, d) in [(0, &a), (1, &b), (2, &c)] {
            d.flush(|id, bytes| {
                let target = match id {
                    ConnId(10) => &a,
                    ConnId(11) => &b,
                    ConnId(12) => &c,
                    _ => unreachable!(),
                };
                target
                    .receive(edge(local).connection, Channel::Reliable, bytes)
                    .unwrap();
                Ok(())
            })
            .unwrap();
        }
        pa.step();
        pb.step();
        // advance is bootstrap's supported progression; next round polls c's input.
        for d in [&a, &b, &c] {
            d.check().unwrap();
        }
        assert!(pa.verified_tick() >= tick.saturating_sub(2));
    }
    let pc = boot.session().unwrap();
    let common = pa
        .verified_tick()
        .min(pb.verified_tick())
        .min(pc.verified_tick());
    assert!(common >= 98);
    for tick in pc.checksums()[0].0..=common {
        let sum =
            |p: &Session<Arena, Source>| p.checksums().iter().find(|(t, _)| *t == tick).unwrap().1;
        assert_eq!(sum(&pa), sum(&pb), "existing tick {tick}");
        assert_eq!(sum(&pa), sum(pc), "joined tick {tick}");
    }
    assert_eq!(a.pending().0, 0);
    assert_eq!(b.pending().0, 0);
    assert_eq!(c.pending().0, 0);
}

#[test]
fn snapshot_binding_rejects_held_local_advances_before_install() {
    let (_, snapshot, mut boot) = grant();
    let (driver, source) = new(2, Limits::default());
    boot.receive_snapshot(PlayerSlot(0), game(), source, &snapshot)
        .unwrap();
    boot.advance(ArenaInput::default(), vec![]);
    assert_eq!(driver.retained_evidence().0, 0);
    assert_eq!(
        driver.install_joiner_snapshot(&boot),
        Err(Error::InvalidSnapshot)
    );
}

#[test]
fn local_encoding_is_once_per_logical_record_across_destinations() {
    let (d, _) = with_two_edges(Limits::default());
    ENCODE_COUNT.with(|c| c.set(0));
    d.send_local(record(1, 0, &[0, 0])).unwrap();
    ENCODE_COUNT.with(|c| assert_eq!(c.get(), 1));
    d.send_local(record(1, 2, &[])).unwrap();
    ENCODE_COUNT.with(|c| assert_eq!(c.get(), 2));
    assert_eq!(d.pending().0, 4);
}
#[test]
fn logical_caps_and_received_length_fields_reject_before_mutation_or_decode() {
    for limits in [
        Limits {
            max_logical_records: 1,
            ..Limits::default()
        },
        Limits {
            max_logical_encoded_bytes: 69,
            ..Limits::default()
        },
    ] {
        let (d, _) = new(0, limits);
        d.admit_edge(edge(1)).unwrap();
        d.send_local(record(1, 0, &[])).unwrap();
        let before = (d.pending(), d.retained_evidence(), d.destination_records());
        assert!(matches!(
            d.send_local(record(2, 0, &[])),
            Err(Error::ByteLimit | Error::RecordLimit)
        ));
        assert_eq!(
            (d.pending(), d.retained_evidence(), d.destination_records()),
            before
        );
    }
    for (offset, value, error) in [
        (44, 21, Error::InputLimit),
        (48, 4, Error::CommandLimit),
        (70, 5, Error::CommandLimit),
    ] {
        let mut p = packet(0, 1, &[0]);
        p[offset] = value;
        let (d, _) = new(1, Limits::default());
        d.admit_edge(edge(0)).unwrap();
        DECODE_COUNT.with(|c| c.set(0));
        assert_eq!(
            d.receive(edge(0).connection, Channel::Reliable, &p),
            Err(error)
        );
        DECODE_COUNT.with(|c| assert_eq!(c.get(), 0));
        assert_eq!(d.incoming(), (0, 0));
        assert_eq!(d.retained_evidence(), (0, 0));
    }
}
#[test]
fn ordered_and_repeated_command_changes_conflict() {
    for owners in [&[1, 0][..], &[0][..], &[0, 0][..]] {
        let original = packet(0, 1, &[0, 1]);
        let replacement = packet(0, 1, owners);
        let (d, _) = new(1, Limits::default());
        d.admit_edge(edge(0)).unwrap();
        d.receive(edge(0).connection, Channel::Reliable, &original)
            .unwrap();
        assert_eq!(
            d.receive(edge(0).connection, Channel::Reliable, &replacement),
            Err(Error::ConflictingDuplicate {
                tick: 1,
                slot: PlayerSlot(0)
            })
        );
    }
}
#[test]
fn callback_abort_does_not_pop_or_send_another_head() {
    for disconnect in [false, true] {
        let (d, _) = with_two_edges(Limits::default());
        d.send_local(record(1, 0, &[])).unwrap();
        let before = d.pending();
        let mut calls = 0;
        let error = if disconnect {
            Error::Disconnected
        } else {
            Error::Cancelled
        };
        assert_eq!(
            d.flush(|id, _| {
                calls += 1;
                if disconnect {
                    assert_eq!(d.disconnected(id), Err(Error::Disconnected));
                } else {
                    d.cancel();
                }
                Ok(())
            }),
            Err(error)
        );
        assert_eq!(calls, 1);
        assert_eq!(d.pending(), before);
    }
}
#[test]
fn local_verification_never_discards_unsent_destination_obligations() {
    let (d, source) = with_two_edges(Limits::default());
    let mut config = cfg(0);
    config.vacant_slots = vec![PlayerSlot(2)];
    let mut peer = Session::<Arena, _>::new(game(), config, source);
    peer.advance(ArenaInput::default(), vec![]);
    let before = d.pending();
    assert_eq!(before.0, 4);
    d.receive(edge(1).connection, Channel::Reliable, &packet(1, 1, &[]))
        .unwrap();
    peer.step();
    assert_eq!(peer.verified_tick(), 1);
    assert_eq!(d.pending(), before);
    assert_eq!(d.flush(|_, _| Ok(())).unwrap().sent, 4);
}

#[test]
fn new_snapshot_filter_does_not_acknowledge_existing_edge_backlog() {
    let (driver, source) = new(0, Limits::default());
    driver.admit_edge(edge(1)).unwrap();
    let mut config = cfg(0);
    config.vacant_slots = vec![PlayerSlot(2)];
    let mut peer = Session::<Arena, _>::new(game(), config, source);
    for tick in 1..=8 {
        peer.advance(ArenaInput::default(), vec![]);
        driver
            .receive(edge(1).connection, Channel::Reliable, &packet(1, tick, &[]))
            .unwrap();
        peer.step();
    }
    assert_eq!(peer.verified_tick(), 8);
    assert_eq!(driver.pending().0, 16);
    let mut bootstrap = Bootstrap::new(cfg(2), roster(), 4096, 1 << 20).unwrap();
    let request = bootstrap.next_request().unwrap();
    let (_, ticket) = serve_checked_join(&mut peer, &context(), &request).unwrap();
    assert_eq!(ticket.ticket().snapshot_tick, 8);
    driver.install_ticket(&ticket).unwrap();
    driver.admit_edge(edge(2)).unwrap();
    assert_eq!(driver.pending().0, 16);
    assert_eq!(driver.delivery(edge(2).connection, 8, PlayerSlot(0)), None);
    driver.send_local(record(9, 0, &[])).unwrap();
    assert_eq!(driver.pending().0, 18);
    let mut ticks = vec![];
    driver
        .flush(|id, bytes| {
            ticks.push((id, u64::from_le_bytes(bytes[35..43].try_into().unwrap())));
            Ok(())
        })
        .unwrap();
    assert_eq!(
        ticks
            .iter()
            .filter(|(id, _)| *id == edge(1).connection)
            .count(),
        17
    );
    assert_eq!(
        ticks
            .iter()
            .filter(|(id, _)| *id == edge(2).connection)
            .map(|(_, t)| *t)
            .collect::<Vec<_>>(),
        [9]
    );
}
