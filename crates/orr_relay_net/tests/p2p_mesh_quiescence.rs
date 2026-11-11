//! Normal mesh quiescence, bounded accounting and same-source drain fixtures.
use std::cell::Cell;

use orr_proto::{Channel, ConnId};
use orr_relay_net::p2p_mesh_input::{
    P2pMeshEdge as Edge, P2pMeshInputDriver, P2pMeshInputError as Error,
    P2pMeshInputLimits as Limits, P2pMeshInputSource, P2pMeshRollingWindow,
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

type Peer = Session<Arena, Source>;
fn small_limits() -> Limits {
    Limits {
        max_logical_records: 32,
        max_incoming_records: 16,
        max_destination_records: 64,
        max_edge_pending_records: 32,
        max_logical_encoded_bytes: 32 * 256,
        max_incoming_encoded_bytes: 16 * 256,
        max_pending_encoded_bytes: 64 * 256,
        max_edge_pending_encoded_bytes: 32 * 256,
        ..Limits::default()
    }
}
fn mesh(rolling: bool) -> ([Driver; 3], [Peer; 3]) {
    let make = |slot| {
        if rolling {
            Driver::new_rolling(
                PlayerSlot(slot),
                context(),
                small_limits(),
                P2pMeshRollingWindow {
                    recent_ticks: 1,
                    future_ticks: 16,
                },
            )
            .unwrap()
        } else {
            new(slot, small_limits())
        }
    };
    let (a, sa) = make(0);
    let (b, sb) = make(1);
    let (c, sc) = make(2);
    let mut ca = cfg(0);
    ca.vacant_slots = vec![PlayerSlot(2)];
    let mut pa = Peer::new(game(), ca, sa);
    let mut pb = Peer::new(game(), cfg(1), sb);
    a.admit_edge(edge(1)).unwrap();
    b.admit_edge(edge(0)).unwrap();
    pa.advance(ArenaInput::default(), vec![]);
    pb.advance(ArenaInput::default(), vec![]);
    a.flush(|_, bytes| {
        b.receive(edge(0).connection, Channel::Reliable, bytes)
            .unwrap();
        Ok(())
    })
    .unwrap();
    b.flush(|_, bytes| {
        a.receive(edge(1).connection, Channel::Reliable, bytes)
            .unwrap();
        Ok(())
    })
    .unwrap();
    pa.poll_confirmed();
    pb.poll_confirmed();
    assert_eq!(pa.verified_tick(), 1);

    let mut boot = Bootstrap::new(cfg(2), roster(), 4096, 1 << 20).unwrap();
    let req = boot.next_request().unwrap();
    let (snapshot, ticket) = serve_checked_join(&mut pa, &context(), &req).unwrap();
    let notices = [
        checked_backlog_notice(&mut pa, &context(), &ticket).unwrap(),
        checked_backlog_notice(&mut pb, &context(), &ticket).unwrap(),
    ];
    a.install_ticket(&ticket).unwrap();
    b.install_ticket(&ticket).unwrap();
    boot.receive_snapshot(PlayerSlot(0), game(), sc, &snapshot)
        .unwrap();
    c.install_joiner_snapshot(&boot).unwrap();
    for (slot, notice) in notices.iter().enumerate() {
        boot.receive_notice(PlayerSlot(slot as u8), notice).unwrap();
    }
    assert_eq!(
        boot.status().unwrap(),
        orr_session::JoinBootstrapStatus::Ready
    );
    let pc = boot.cancel().unwrap();
    for (local, d) in [&a, &b, &c].iter().enumerate() {
        for remote in 0..3 {
            if usize::from(remote) != local && !(local < 2 && remote < 2) {
                d.admit_edge(edge(remote)).unwrap();
            }
        }
    }
    ([a, b, c], [pa, pb, pc])
}
fn transfer_all(ds: &[Driver; 3]) {
    for (local, d) in ds.iter().enumerate() {
        d.flush(|id, bytes| {
            ds[(id.0 - 10) as usize]
                .receive(edge(local as u8).connection, Channel::Reliable, bytes)
                .unwrap();
            Ok(())
        })
        .unwrap();
    }
}
type Usage = (usize, usize);
type Accounting = (Usage, Usage, Usage, usize);
fn usage(d: &Driver) -> Accounting {
    (
        d.retained_evidence(),
        d.incoming(),
        d.pending(),
        d.destination_records(),
    )
}
#[test]
fn queued_ordered_commands_drain_and_verify_with_frozen_summary() {
    let (ds, mut ps) = mesh(false);
    for (slot, p) in ps.iter_mut().enumerate() {
        p.advance(
            ArenaInput::default(),
            record(1, slot as u8, &[slot as u32, 2, slot as u32]).commands,
        );
    }
    transfer_all(&ds);
    let before = usage(&ds[0]);
    let q = ds[0].quiesce(&ps[0]).unwrap();
    assert_eq!(q.verified_tick, 1);
    assert_eq!(q.next_send_tick, 3);
    assert_eq!(q.accepted_remote_max_by_slot, [None, Some(2), Some(2)]);
    assert_eq!(usage(&ds[0]), before);
    ds[0].disconnected(edge(2).connection).unwrap();
    ps[0].poll_confirmed();
    assert_eq!(ps[0].verified_tick(), 2);
    assert_eq!(ds[0].incoming(), (0, 0));
    assert!(ps[0].source_mut().poll_remote().is_empty());
    assert_eq!(ds[0].quiesce(&ps[0]).unwrap(), q);
    for p in &mut ps[1..] {
        p.poll_confirmed();
    }
    for p in &ps[1..] {
        assert_eq!(
            p.verified_frame().unwrap().checksum(),
            ps[0].verified_frame().unwrap().checksum()
        );
    }
    ds[0].check().unwrap();
    assert_eq!(ps[2].join_status().unwrap(), orr_session::JoinStatus::Ready);
    assert_eq!(ds[2].quiesce(&ps[2]).unwrap().verified_tick, 2);
}
#[test]
fn unequal_survivor_maxima_and_holes_do_not_authorize_verification() {
    let (ds, mut ps) = mesh(false);
    for p in &mut ps {
        for _ in 0..3 {
            p.advance(ArenaInput::default(), vec![]);
        }
    }
    // Peer 2's accepted records differ across survivors, and each has a gap.
    ds[2]
        .flush(|id, bytes| {
            let tick = u64::from_le_bytes(bytes[35..43].try_into().unwrap());
            if (id == edge(0).connection && tick == 3) || (id == edge(1).connection && tick == 2) {
                ds[(id.0 - 10) as usize]
                    .receive(edge(2).connection, Channel::Reliable, bytes)
                    .unwrap();
            }
            Ok(())
        })
        .unwrap();
    let q0 = ds[0].quiesce(&ps[0]).unwrap();
    let q1 = ds[1].quiesce(&ps[1]).unwrap();
    assert_eq!(q0.accepted_remote_max_by_slot[2], Some(3));
    assert_eq!(q1.accepted_remote_max_by_slot[2], Some(2));
    for i in 0..2 {
        let pending = ds[i].pending();
        ps[i].poll_confirmed();
        assert_eq!(ps[i].verified_tick(), 1);
        assert_eq!(ds[i].incoming(), (0, 0));
        assert_eq!(ds[i].pending(), pending);
        ds[i].check().unwrap();
    }
}
#[test]
fn quiescence_blocks_mutation_without_discarding_queued_work() {
    let (ds, mut ps) = mesh(false);
    ps[0].advance(ArenaInput::default(), vec![]);
    let q = ds[0].quiesce(&ps[0]).unwrap();
    let before = usage(&ds[0]);
    assert_eq!(ds[0].send_local(record(2, 0, &[])), Err(Error::Quiesced));
    assert_eq!(
        ds[0].receive(edge(1).connection, Channel::Reliable, &[]),
        Err(Error::Quiesced)
    );
    assert_eq!(ds[0].admit_edge(edge(1)), Err(Error::Quiesced));
    assert_eq!(ds[0].install_ticket(&grant().0), Err(Error::Quiesced));
    assert_eq!(
        ds[0].install_joiner_snapshot(&grant().2),
        Err(Error::Quiesced)
    );
    let mut called = false;
    assert_eq!(
        ds[0].flush(|_, _| {
            called = true;
            Ok(())
        }),
        Err(Error::Quiesced)
    );
    assert!(!called);
    ds[0].disconnected(edge(1).connection).unwrap();
    ds[0].disconnected(edge(1).connection).unwrap();
    assert_eq!(ds[0].disconnected(ConnId(999)), Err(Error::WrongConnection));
    assert_eq!(usage(&ds[0]), before);
    assert_eq!(ds[0].quiesce(&ps[0]).unwrap(), q);
    ds[0].check().unwrap();
    ds[0].cancel();
    assert!(ds[0].is_quiesced());
    assert_eq!(ds[0].quiesce(&ps[0]), Err(Error::Cancelled));
}
#[test]
fn session_authoring_after_quiescence_is_terminal() {
    let (ds, mut ps) = mesh(false);
    ds[0].quiesce(&ps[0]).unwrap();
    ps[0].advance(ArenaInput::default(), vec![]);
    assert_eq!(ds[0].check(), Err(Error::Quiesced));
    assert!(ds[0].is_quiesced());
    assert_eq!(ds[0].quiesce(&ps[0]), Err(Error::Quiesced));
    assert!(ps[0].source_mut().poll_remote().is_empty());
}
#[test]
fn active_flush_rejection_preserves_normal_queue_acceptance() {
    let (ds, mut ps) = mesh(false);
    ps[0].advance(ArenaInput::default(), vec![]);
    let before = ds[0].pending();
    let result = ds[0]
        .flush(|_, _| {
            assert_eq!(ds[0].quiesce(&ps[0]), Err(Error::QuiescenceUnavailable));
            assert!(!ds[0].is_quiesced());
            Ok(())
        })
        .unwrap();
    assert_eq!(result.sent, before.0);
    assert_eq!(ds[0].pending(), (0, 0));
    ds[0].check().unwrap();
    ds[0].quiesce(&ps[0]).unwrap();
}
#[test]
fn preconditions_reject_foreign_source_config_syncing_and_failed_driver() {
    let (ds, ps) = mesh(false);
    assert_eq!(ds[0].quiesce(&ps[1]), Err(Error::QuiescenceUnavailable));
    let (d, source) = new(0, Limits::default());
    let wrong_local = Peer::new(game(), cfg(1), source);
    assert_eq!(d.quiesce(&wrong_local), Err(Error::QuiescenceUnavailable));
    let (d, source) = new(0, Limits::default());
    let mut c = cfg(0);
    c.relay = true;
    let relay = Peer::new(game(), c, source);
    assert_eq!(d.quiesce(&relay), Err(Error::QuiescenceUnavailable));
    let (_, snapshot, mut boot) = grant();
    let (d, source) = new(2, Limits::default());
    boot.receive_snapshot(PlayerSlot(0), game(), source, &snapshot)
        .unwrap();
    d.install_joiner_snapshot(&boot).unwrap();
    assert_eq!(
        d.quiesce(boot.session().unwrap()),
        Err(Error::QuiescenceUnavailable)
    );
    d.check().unwrap();
    assert!(!d.is_quiesced());
    assert_eq!(
        ds[0].disconnected(edge(1).connection),
        Err(Error::Disconnected)
    );
    assert_eq!(ds[0].quiesce(&ps[0]), Err(Error::Disconnected));
}
#[test]
fn rolling_retirement_preserves_admission_maxima_and_pending_caps() {
    let (ds, mut ps) = mesh(true);
    for _ in 0..5 {
        for p in &mut ps {
            p.advance(ArenaInput::default(), vec![]);
        }
        transfer_all(&ds);
        for p in &mut ps {
            p.poll_confirmed();
        }
    }
    assert!(ds[0].rolling_progress().unwrap().1 >= 4);
    // Keep a bounded unsent tail: quiescence never acknowledges these obligations.
    ps[0].advance(ArenaInput::default(), vec![]);
    let pending = ds[0].pending();
    let q = ds[0].quiesce(&ps[0]).unwrap();
    assert_eq!(q.accepted_remote_max_by_slot, [None, Some(6), Some(6)]);
    assert_eq!(q.verified_tick, 6);
    assert_eq!(ds[0].pending(), pending);
    assert!(ds[0].retained_evidence().0 <= small_limits().max_logical_records);
    assert!(pending.1 <= small_limits().max_pending_encoded_bytes);
}

#[test]
fn proven_join_gap_cannot_quiesce_or_flush_held_authoring() {
    let (d, source) = new(2, small_limits());
    let mut boot = Bootstrap::new(cfg(2), roster(), 4096, 1 << 20).unwrap();
    let request = boot.next_request().unwrap();
    let mut ca = cfg(0);
    ca.vacant_slots = vec![PlayerSlot(2)];
    let mut donor = Session::<Arena, _>::new(game(), ca, LocalInputSource);
    let mut peer = Session::<Arena, _>::new(game(), cfg(1), LocalInputSource);
    // No authored history: neither existing peer can advertise continuous coverage.
    let (snapshot, ticket) = serve_checked_join(&mut donor, &context(), &request).unwrap();
    boot.receive_snapshot(PlayerSlot(0), game(), source, &snapshot)
        .unwrap();
    d.install_joiner_snapshot(&boot).unwrap();
    boot.advance(ArenaInput::default(), vec![SpawnBulletCmd { owner: 2 }]);
    for (slot, notice) in [
        checked_backlog_notice(&mut donor, &context(), &ticket).unwrap(),
        checked_backlog_notice(&mut peer, &context(), &ticket).unwrap(),
    ]
    .iter()
    .enumerate()
    {
        boot.receive_notice(PlayerSlot(slot as u8), notice).unwrap();
    }
    assert!(boot.session().unwrap().join_status().is_err());
    assert_eq!(
        d.quiesce(boot.session().unwrap()),
        Err(Error::QuiescenceUnavailable)
    );
    assert!(!d.is_quiesced());
    assert_eq!(d.pending(), (0, 0));
    d.check().unwrap();
}

#[test]
fn remote_donor_defaults_are_reported_by_logical_slot() {
    let (a, sa) = new(0, small_limits());
    let (b, sb) = new(1, small_limits());
    a.admit_edge(edge(1)).unwrap();
    b.admit_edge(edge(0)).unwrap();
    let mut ca = cfg(0);
    ca.vacant_slots = vec![PlayerSlot(2)];
    let mut donor = Peer::new(game(), ca, sa);
    let receiver = Peer::new(game(), cfg(1), sb);
    donor.advance(ArenaInput::default(), vec![]);
    a.flush(|_, bytes| {
        b.receive(edge(0).connection, Channel::Reliable, bytes)
            .unwrap();
        Ok(())
    })
    .unwrap();
    let q = b.quiesce(&receiver).unwrap();
    assert_eq!(q.accepted_remote_max_by_slot, [Some(1), None, Some(1)]);
    assert_eq!(q.next_send_tick, 1);
    assert_eq!(b.incoming().0, 2);
}

#[test]
fn cached_summary_rechecks_source_and_cursor_before_returning() {
    let (ds, mut ps) = mesh(false);
    let q = ds[0].quiesce(&ps[0]).unwrap();
    assert_eq!(ds[0].quiesce(&ps[1]), Err(Error::QuiescenceUnavailable));
    // Replacing/swapping the Session is unsupported. This guard still rejects
    // a moved source with a different cursor rather than blessing cached data.
    let (_, source) = new(0, small_limits());
    let mut replacement = Peer::new(game(), cfg(0), source);
    assert_ne!(replacement.next_send_tick(), q.next_send_tick);
    std::mem::swap(ps[0].source_mut(), replacement.source_mut());
    assert_eq!(
        ds[0].quiesce(&replacement),
        Err(Error::QuiescenceUnavailable)
    );
    ds[0].check().unwrap();
}

#[test]
fn maximum_itself_survives_retirement_during_real_preconfirmed_progress() {
    let (sender, _) = new(0, small_limits());
    sender.admit_edge(edge(1)).unwrap();
    let (receiver, source) = Driver::new_rolling(
        PlayerSlot(1),
        context(),
        small_limits(),
        P2pMeshRollingWindow {
            recent_ticks: 1,
            future_ticks: 16,
        },
    )
    .unwrap();
    receiver.admit_edge(edge(0)).unwrap();
    let mut config = cfg(1);
    config.input_delay = 4;
    let mut session = Peer::new(game(), config, source);
    sender.send_local(record(1, 0, &[])).unwrap();
    sender
        .flush(|_, bytes| {
            receiver
                .receive(edge(0).connection, Channel::Reliable, bytes)
                .unwrap();
            Ok(())
        })
        .unwrap();
    assert_eq!(receiver.retained_evidence().0, 1);
    for _ in 0..3 {
        session.step();
        session.poll_confirmed();
    }
    assert_eq!(session.verified_tick(), 3);
    assert!(receiver.rolling_progress().unwrap().1 >= 2);
    assert_eq!(receiver.retained_evidence(), (0, 0));
    let q = receiver.quiesce(&session).unwrap();
    assert_eq!(q.accepted_remote_max_by_slot, [Some(1), None, None]);
    assert_eq!(q.next_send_tick, 5);
}

#[test]
fn quiesced_source_transfers_command_order_once() {
    let (ds, mut ps) = mesh(false);
    for (slot, p) in ps.iter_mut().enumerate() {
        p.advance(
            ArenaInput::default(),
            record(2, slot as u8, &[slot as u32, 2, slot as u32]).commands,
        );
    }
    transfer_all(&ds);
    ds[0].quiesce(&ps[0]).unwrap();
    let records = ps[0].source_mut().poll_remote();
    assert_eq!(records.len(), 2);
    for r in records {
        assert_eq!(r.tick, 2);
        assert_eq!(
            r.commands.iter().map(|c| c.owner).collect::<Vec<_>>(),
            [u32::from(r.slot.0), 2, u32::from(r.slot.0)]
        );
    }
    assert!(ps[0].source_mut().poll_remote().is_empty());
    assert_eq!(ds[0].incoming(), (0, 0));
    for p in &mut ps[1..] {
        p.poll_confirmed();
    }
    assert_eq!(
        ps[1].verified_frame().unwrap().checksum(),
        ps[2].verified_frame().unwrap().checksum()
    );
}
