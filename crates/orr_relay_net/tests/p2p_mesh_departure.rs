//! Normal mesh quiescence, bounded accounting and same-source drain fixtures.
use orr_session::{DepartureAck, DepartureFence, DepartureTarget};
use orr_sim::{Simulation, TickInputs};
use std::cell::Cell;

use orr_proto::{Channel, ConnId};
use orr_relay_net::p2p_mesh_input::{
    P2pMeshEdge as Edge, P2pMeshInputDriver, P2pMeshInputError as Error,
    P2pMeshInputLimits as Limits, P2pMeshInputSource, P2pMeshRollingWindow,
};
use orr_relay_net::P2pInputCodec;
use orr_session::{
    checked_backlog_notice, serve_checked_join, CheckedJoinContext, InputSource, JoinBootstrap,
    JoinRoster, PlayerSlot, RemoteInput, Session, SessionConfig,
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
fn cfg(slot: u8, delay: u32) -> SessionConfig {
    let mut c = SessionConfig::new(3, PlayerSlot(slot), 42, 60);
    c.join_id = if slot == 2 { 17 } else { 0 };
    c.input_delay = delay;
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
type Peer = Session<Arena, Source>;
fn small_limits() -> Limits {
    Limits {
        max_logical_records: 64,
        max_incoming_records: 16,
        max_destination_records: 64,
        max_edge_pending_records: 32,
        max_logical_encoded_bytes: 64 * 256,
        max_incoming_encoded_bytes: 16 * 256,
        max_pending_encoded_bytes: 64 * 256,
        max_edge_pending_encoded_bytes: 32 * 256,
        ..Limits::default()
    }
}
fn mesh(rolling: bool, delay: u32) -> ([Driver; 3], [Peer; 3]) {
    mesh_with_limits(rolling, delay, small_limits())
}
fn mesh_with_limits(rolling: bool, delay: u32, limits: Limits) -> ([Driver; 3], [Peer; 3]) {
    let make = |slot| {
        if rolling {
            Driver::new_rolling(
                PlayerSlot(slot),
                context(),
                limits.clone(),
                P2pMeshRollingWindow {
                    recent_ticks: 1,
                    future_ticks: 16,
                },
            )
            .unwrap()
        } else {
            new(slot, limits.clone())
        }
    };
    let (a, sa) = make(0);
    let (b, sb) = make(1);
    let (c, sc) = make(2);
    let mut ca = cfg(0, delay);
    ca.vacant_slots = vec![PlayerSlot(2)];
    let mut pa = Peer::new(game(), ca, sa);
    let mut pb = Peer::new(game(), cfg(1, delay), sb);
    a.admit_edge(edge(1)).unwrap();
    b.admit_edge(edge(0)).unwrap();
    pa.advance(ArenaInput::default(), vec![]);
    pb.advance(ArenaInput::default(), vec![]);
    a.flush(|_, bytes| {
        b.receive(edge(0).connection, Channel::Reliable, bytes)
            .unwrap();
        // A caller may drain each accepted packet under a one-record incoming cap.
        pb.poll_confirmed();
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

    let mut boot = Bootstrap::new(cfg(2, delay), roster(), 4096, 1 << 20).unwrap();
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
    let ds = [a, b, c];
    let mut ps = [pa, pb, pc];
    transfer_all(&ds);
    for p in &mut ps {
        p.poll_confirmed();
    }
    (ds, ps)
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
fn fresh_edge(remote: u8) -> Edge {
    Edge {
        generation: 78,
        ..edge(remote)
    }
}
fn reports(ds: &[Driver; 3], ps: &[Peer; 3]) -> ([DepartureFence; 2], [DepartureAck; 2]) {
    let fences = std::array::from_fn(|i| {
        let q = ds[i].quiesce(&ps[i]).unwrap();
        DepartureFence {
            recovery_id: 91,
            revision: 2,
            survivor: PlayerSlot(i as u8),
            verified_tick: ps[i].verified_tick(),
            departed_max: q.accepted_remote_max_by_slot[2],
        }
    });
    let t = ps[0].verified_tick();
    let target = DepartureTarget {
        recovery_id: 91,
        revision: 2,
        departed: PlayerSlot(2),
        owner: PlayerSlot(0),
        target: t,
        cutoff: t + 1,
    };
    let acks = std::array::from_fn(|i| DepartureAck {
        target,
        survivor: PlayerSlot(i as u8),
        checksum: ps[i].verified_frame().unwrap().checksum(),
    });
    (fences, acks)
}
fn commands(slot: u8) -> Vec<SpawnBulletCmd> {
    vec![
        SpawnBulletCmd {
            owner: u32::from(slot),
        },
        SpawnBulletCmd { owner: 2 },
        SpawnBulletCmd {
            owner: u32::from(slot),
        },
    ]
}
fn settled(rolling: bool, delay: u32) -> ([Driver; 3], [Peer; 3]) {
    let (ds, mut ps) = mesh(rolling, delay);
    // Align simulation heads after the snapshot without authoring or altering cursors.
    for p in &mut ps {
        while p.head_tick() < p.next_send_tick() - 1 {
            p.step();
        }
        p.poll_confirmed();
    }
    for _ in 0..4 {
        for (i, p) in ps.iter_mut().enumerate() {
            p.advance(ArenaInput::default(), commands(i as u8));
        }
        transfer_all(&ds);
        for p in &mut ps {
            p.poll_confirmed();
        }
    }
    for p in &mut ps {
        while p.head_tick() < p.next_send_tick() - 1 {
            p.step();
        }
        p.poll_confirmed();
    }
    assert!(ps
        .iter()
        .all(|p| p.verified_tick() == p.next_send_tick() - 1));
    (ds, ps)
}
fn checksum_reference(t: u64, first: u64, cutoff: u64) -> u64 {
    let mut sim = Simulation::<Arena>::new(game(), 60, 42);
    for tick in 1..=t {
        let mut input = TickInputs::new(tick, 3);
        if tick >= first {
            for slot in 0..3 {
                if slot != 2 || tick < cutoff {
                    for c in commands(slot) {
                        input.push_command(PlayerSlot(slot), c);
                    }
                }
            }
        }
        sim.step(&input);
    }
    sim.frame().checksum()
}
#[test]
fn command_bearing_planned_departure_preserves_session_and_matches_reference() {
    for (rolling, delay) in [(false, 0), (false, 2), (true, 0), (true, 2)] {
        let (ds, mut ps) = settled(rolling, delay);
        let (fences, acks) = reports(&ds, &ps);
        let t = ps[0].verified_tick();
        let cursor = ps[0].next_send_tick();
        let before = ps[0].verified_frame().unwrap().checksum();
        assert_eq!(before, checksum_reference(t, 2 + u64::from(delay), cursor));
        let old_usage = usage(&ds[0]);
        let old_clone = ds[0].clone();
        let mut result0 = ds[0]
            .continue_planned_departure(&mut ps[0], fences, acks, fresh_edge(1))
            .unwrap();
        let result1 = ds[1]
            .continue_planned_departure(&mut ps[1], fences, acks, fresh_edge(0))
            .unwrap();
        let fresh = [result0.fresh_driver, result1.fresh_driver];
        assert_eq!(ps[0].verified_frame().unwrap().checksum(), before);
        assert_eq!(ps[0].next_send_tick(), cursor);
        assert_eq!(ps[0].head_tick(), t);
        assert_eq!(usage(&ds[0]), old_usage);
        assert_eq!(
            old_clone.send_local(record(cursor, 0, &[])),
            Err(Error::Quiesced)
        );
        assert_eq!(ds[0].quiesce(&ps[0]), Err(Error::QuiescenceUnavailable));
        if rolling {
            assert_eq!(fresh[0].rolling_progress(), Some((t, t)));
        }
        for _ in 0..12 {
            for (i, p) in ps[..2].iter_mut().enumerate() {
                p.advance(ArenaInput::default(), commands(i as u8));
            }
            for (i, d) in fresh.iter().enumerate() {
                d.flush(|id, bytes| {
                    assert_eq!(id, fresh_edge((1 - i) as u8).connection);
                    let slot = bytes[43];
                    if slot == 2 {
                        assert_eq!(i, 0);
                        assert_eq!(u16::from_le_bytes(bytes[48..50].try_into().unwrap()), 0);
                    }
                    fresh[1 - i]
                        .receive(fresh_edge(i as u8).connection, Channel::Reliable, bytes)
                        .unwrap();
                    Ok(())
                })
                .unwrap();
            }
            for p in &mut ps[..2] {
                p.poll_confirmed();
                assert_eq!(
                    p.verified_frame().unwrap().checksum(),
                    checksum_reference(p.verified_tick(), 2 + u64::from(delay), cursor)
                );
            }
        }
        if rolling {
            assert!(fresh[0].rolling_progress().unwrap().1 > cursor);
            assert!(fresh[0].retained_evidence().0 <= 6);
        }
        result0
            .retired_source
            .send_local(cursor, PlayerSlot(0), ArenaInput::default(), vec![]);
        assert_eq!(ds[0].check(), Err(Error::Quiesced));
        fresh[0].check().unwrap();
        fresh[1].check().unwrap();
    }
}
#[test]
fn preparation_rejections_preserve_old_state() {
    for case in 0..11 {
        let (ds, mut ps) = settled(false, 0);
        let (mut fences, mut acks) = reports(&ds, &ps);
        let mut edge = fresh_edge(1);
        match case {
            0 => fences[1] = fences[0],
            1 => acks[1] = acks[0],
            2 => fences[1].recovery_id += 1,
            3 => acks[1].checksum ^= 1,
            4 => acks[0].target.cutoff += 1,
            5 => edge.generation = 77,
            6 => edge.remote = PlayerSlot(2),
            7 => edge.adapter_id = 0,
            8 => fences[0].departed_max = None,
            9 => fences[1].verified_tick += 1,
            10 => {
                acks[0].checksum ^= 1;
                acks[1].checksum ^= 1;
            }
            _ => unreachable!(),
        }
        let before = (
            usage(&ds[0]),
            ps[0].verified_tick(),
            ps[0].head_tick(),
            ps[0].next_send_tick(),
            ps[0].verified_frame().unwrap().checksum(),
        );
        assert!(ds[0]
            .continue_planned_departure(&mut ps[0], fences, acks, edge)
            .is_err());
        assert_eq!(
            before,
            (
                usage(&ds[0]),
                ps[0].verified_tick(),
                ps[0].head_tick(),
                ps[0].next_send_tick(),
                ps[0].verified_frame().unwrap().checksum()
            )
        );
        ds[0].check().unwrap();
        assert!(ds[0].quiesce(&ps[0]).is_ok());
    }
}
#[test]
fn queues_and_direct_driver_future_tail_cannot_be_discarded() {
    for case in 0..3 {
        let (ds, mut ps) = settled(false, 0);
        let tick = ps[0].next_send_tick();
        if case == 0 {
            ds[0].send_local(record(tick, 0, &[])).unwrap();
        } else {
            ds[1].send_local(record(tick, 1, &[])).unwrap();
            transfer_all(&ds);
            if case == 2 {
                ps[0].poll_confirmed();
            }
        }
        let (fences, acks) = reports(&ds, &ps);
        let before = usage(&ds[0]);
        assert!(ds[0]
            .continue_planned_departure(&mut ps[0], fences, acks, fresh_edge(1))
            .is_err());
        assert_eq!(usage(&ds[0]), before);
        ds[0].check().unwrap();
    }
}
#[test]
fn fresh_generation_rejects_old_wire_and_pre_cutoff_records() {
    for case in 0..6 {
        let (ds, mut ps) = settled(false, 0);
        let (fences, acks) = reports(&ds, &ps);
        let t = ps[0].verified_tick();
        let fresh = ds[0]
            .continue_planned_departure(&mut ps[0], fences, acks, fresh_edge(1))
            .unwrap()
            .fresh_driver;
        if case == 0 {
            assert_eq!(fresh.admit_edge(edge(2)), Err(Error::InvalidEdge));
        }
        if case == 1 {
            assert_eq!(fresh.send_local(record(t, 0, &[])), Err(Error::TickRange));
        }
        if case == 3 {
            assert_eq!(
                fresh.send_local(record(t + 1, 2, &[0])),
                Err(Error::InvalidVacancy)
            );
        }
        if case == 4 {
            let mut r = record(t + 1, 2, &[]);
            r.input.buttons = 1;
            assert_eq!(fresh.send_local(r), Err(Error::InvalidVacancy));
        }
        if case == 5 {
            assert_eq!(
                fresh.send_local(record(t + 1, 1, &[])),
                Err(Error::WrongAuthority)
            );
        }
        if case == 2 {
            ds[1].send_local(record(t + 1, 1, &[])).unwrap_err();
            // A separately admitted old-generation sender models delayed old wire.
            let (sender, _) = new(1, small_limits());
            sender.admit_edge(edge(0)).unwrap();
            sender.send_local(record(t + 1, 1, &[])).unwrap();
            sender
                .flush(|_, b| {
                    assert_eq!(
                        fresh.receive(edge(1).connection, Channel::Reliable, b),
                        Err(Error::WrongGeneration)
                    );
                    Ok(())
                })
                .unwrap();
        }
    }
}

#[test]
fn transition_requires_quiescence_exact_source_and_established_joiner() {
    let (ds, mut ps) = settled(false, 0);
    let (fences, acks) = reports(&ds, &ps);
    let before = usage(&ds[0]);
    assert!(ds[0]
        .continue_planned_departure(&mut ps[1], fences, acks, fresh_edge(1))
        .is_err());
    assert_eq!(usage(&ds[0]), before);
    ds[0].check().unwrap();
    let (live, mut sessions) = settled(false, 0);
    assert!(live[0]
        .continue_planned_departure(&mut sessions[0], fences, acks, fresh_edge(1))
        .is_err());
    assert!(!live[0].is_quiesced());
    live[0].check().unwrap();
    let (prejoin, source) = new(0, small_limits());
    let mut session = Peer::new(game(), cfg(0, 0), source);
    prejoin.quiesce(&session).unwrap();
    assert!(prejoin
        .continue_planned_departure(&mut session, fences, acks, fresh_edge(1))
        .is_err());
    prejoin.check().unwrap();
}

#[test]
fn fully_flushed_direct_tail_and_predicted_head_still_reject() {
    for case in 0..2 {
        let (ds, mut ps) = settled(false, 0);
        if case == 0 {
            ds[0]
                .send_local(record(ps[0].next_send_tick(), 0, &[]))
                .unwrap();
            ds[0].flush(|_, _| Ok(())).unwrap();
            assert_eq!(ds[0].pending(), (0, 0));
            assert!(ps[0].authored_since(ps[0].verified_tick()).is_empty());
        } else {
            ps[0].step();
        }
        let (fences, acks) = reports(&ds, &ps);
        let before = usage(&ds[0]);
        let head = ps[0].head_tick();
        assert!(ds[0]
            .continue_planned_departure(&mut ps[0], fences, acks, fresh_edge(1))
            .is_err());
        assert_eq!(usage(&ds[0]), before);
        assert_eq!(ps[0].head_tick(), head);
        ds[0].check().unwrap();
    }
}

#[test]
fn finite_cutoff_must_remain_in_original_range() {
    let (ds, mut ps) = mesh_with_limits(
        false,
        0,
        Limits {
            end_tick: 3,
            ..small_limits()
        },
    );
    for (i, p) in ps.iter_mut().enumerate() {
        p.advance(ArenaInput::default(), commands(i as u8));
    }
    transfer_all(&ds);
    for p in &mut ps {
        p.poll_confirmed();
    }
    let (fences, acks) = reports(&ds, &ps);
    let before = usage(&ds[0]);
    assert!(matches!(
        ds[0].continue_planned_departure(&mut ps[0], fences, acks, fresh_edge(1)),
        Err(orr_relay_net::p2p_mesh_input::P2pMeshDepartureError::Input(
            Error::TickRange
        ))
    ));
    assert_eq!(usage(&ds[0]), before);
    ds[0].check().unwrap();
}

#[test]
fn rolling_receive_rejects_pre_departure_cutoff_before_retired_handling() {
    let (ds, mut ps) = settled(true, 0);
    let (fences, acks) = reports(&ds, &ps);
    let t = ps[0].verified_tick();
    let fresh = ds[0]
        .continue_planned_departure(&mut ps[0], fences, acks, fresh_edge(1))
        .unwrap()
        .fresh_driver;
    assert_eq!(fresh.rolling_progress(), Some((t, t)));
    let (sender, _) = new(1, small_limits());
    sender.admit_edge(fresh_edge(0)).unwrap();
    sender.send_local(record(t, 1, &[])).unwrap();
    let before = usage(&fresh);
    sender
        .flush(|_, bytes| {
            assert_eq!(
                fresh.receive(fresh_edge(1).connection, Channel::Reliable, bytes),
                Err(Error::TickRange)
            );
            Ok(())
        })
        .unwrap();
    assert_eq!(usage(&fresh), before);
    assert_eq!(fresh.check(), Err(Error::TickRange));
    assert!(ps[0].source_mut().poll_remote().is_empty());
}

#[test]
fn departed_generation_rejects_even_a_previously_valid_ticket() {
    let mut config = cfg(0, 0);
    config.vacant_slots = vec![PlayerSlot(2)];
    let mut donor = Session::<Arena, _>::new(game(), config, orr_session::LocalInputSource);
    donor.advance(ArenaInput::default(), vec![]);
    let mut bootstrap = Bootstrap::new(cfg(2, 0), roster(), 4096, 1 << 20).unwrap();
    let request = bootstrap.next_request().unwrap();
    let (_, ticket) = serve_checked_join(&mut donor, &context(), &request).unwrap();
    let (ds, mut ps) = settled(false, 0);
    let (fences, acks) = reports(&ds, &ps);
    let fresh = ds[0]
        .continue_planned_departure(&mut ps[0], fences, acks, fresh_edge(1))
        .unwrap()
        .fresh_driver;
    let before = usage(&fresh);
    assert_eq!(fresh.install_ticket(&ticket), Err(Error::InvalidConfig));
    assert_eq!(usage(&fresh), before);
    assert_eq!(fresh.check(), Err(Error::InvalidConfig));
}

#[test]
fn mandatory_default_incoming_budget_rejects_without_mutation() {
    // Normal three-peer traffic can be drained packet by packet under this cap.
    // The fresh peer-1 generation must preflight both owner-0 records per tick.
    let (ds, mut ps) = mesh_with_limits(
        false,
        0,
        Limits {
            max_incoming_records: 1,
            ..small_limits()
        },
    );
    for (slot, p) in ps.iter_mut().enumerate() {
        p.advance(ArenaInput::default(), commands(slot as u8));
    }
    for (local, driver) in ds.iter().enumerate() {
        driver
            .flush(|id, bytes| {
                let remote = (id.0 - 10) as usize;
                ds[remote]
                    .receive(edge(local as u8).connection, Channel::Reliable, bytes)
                    .unwrap();
                ps[remote].poll_confirmed();
                Ok(())
            })
            .unwrap();
    }
    for p in &mut ps {
        p.poll_confirmed();
    }
    let (fences, acks) = reports(&ds, &ps);
    let before = (
        usage(&ds[1]),
        ps[1].head_tick(),
        ps[1].verified_tick(),
        ps[1].next_send_tick(),
        ps[1].verified_frame().unwrap().checksum(),
    );
    assert!(matches!(
        ds[1].continue_planned_departure(&mut ps[1], fences, acks, fresh_edge(0)),
        Err(orr_relay_net::p2p_mesh_input::P2pMeshDepartureError::Input(
            Error::RecordLimit
        ))
    ));
    assert_eq!(
        before,
        (
            usage(&ds[1]),
            ps[1].head_tick(),
            ps[1].verified_tick(),
            ps[1].next_send_tick(),
            ps[1].verified_frame().unwrap().checksum()
        )
    );
    ds[1].check().unwrap();
    assert!(ds[1].quiesce(&ps[1]).is_ok());
}
