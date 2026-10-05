//! Admitted endpoint controls plus real deterministic input catch-up.
use orr_fp::FP;
use orr_session::{
    InputSource, JoinBootstrap, JoinBootstrapStatus as Status, JoinRoster, LoopbackClock,
    LoopbackEnd, LoopbackNetwork, P2pAttempt, P2pMembership, P2pMembershipError as Error,
    P2pRoutedEvent, P2pSlotState as SlotState, PlayerSlot, RemoteInput, Session, SessionConfig,
};
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd};

const JOINER: PlayerSlot = PlayerSlot(2);
fn cfg(slot: u8, log: u32) -> SessionConfig {
    let mut c = SessionConfig::new(3, PlayerSlot(slot), 42, 60);
    c.join_id = 7;
    c.input_log_ticks = log;
    c.checksum_interval = 1;
    c
}
fn roster() -> JoinRoster {
    JoinRoster::completed(
        3,
        JOINER,
        PlayerSlot(0),
        vec![PlayerSlot(1), PlayerSlot(0)],
        &[],
    )
    .unwrap()
}
fn game() -> ArenaConfig {
    ArenaConfig { player_count: 3 }
}
fn bootstrap<S: InputSource<Arena>>(budget: usize) -> JoinBootstrap<Arena, S> {
    JoinBootstrap::new(cfg(2, 512), roster(), budget, 1 << 20).unwrap()
}
#[derive(Default)]
struct Mesh {
    links: Vec<LoopbackEnd<Arena>>,
    sent: Vec<(u64, PlayerSlot, ArenaInput)>,
}
impl InputSource<Arena> for Mesh {
    fn send_local(
        &mut self,
        tick: u64,
        slot: PlayerSlot,
        input: ArenaInput,
        commands: Vec<SpawnBulletCmd>,
    ) {
        self.sent.push((tick, slot, input));
        for link in &mut self.links {
            link.send_local(tick, slot, input, commands.clone());
        }
    }
    fn poll_remote(&mut self) -> Vec<RemoteInput<Arena>> {
        self.links
            .iter_mut()
            .flat_map(InputSource::poll_remote)
            .collect()
    }
}
type Peer = Session<Arena, Mesh>;
struct Env {
    a: Peer,
    b: Peer,
    clock: LoopbackClock,
}
fn drive(peer: &mut Peer) {
    let t = peer.next_send_tick();
    let x = (t % 3) as i32 - 1;
    peer.advance(ArenaInput::new(FP::from_int(x), FP::ZERO, false), vec![]);
}
impl Env {
    fn new(log: u32) -> Self {
        let (a, b, clock) = LoopbackNetwork::new::<Arena>(2, 1, 91);
        let mut c = cfg(0, log);
        c.vacant_slots = vec![JOINER];
        let mut e = Self {
            a: Session::new(
                game(),
                c,
                Mesh {
                    links: vec![a],
                    ..Mesh::default()
                },
            ),
            b: Session::new(
                game(),
                cfg(1, log),
                Mesh {
                    links: vec![b],
                    ..Mesh::default()
                },
            ),
            clock,
        };
        for _ in 0..60 {
            e.round(None);
        }
        e
    }
    fn round(&mut self, join: Option<&mut JoinBootstrap<Arena, Mesh>>) {
        self.clock.tick();
        drive(&mut self.a);
        drive(&mut self.b);
        if let Some(j) = join {
            let t = j.session().unwrap().next_send_tick();
            j.advance(
                ArenaInput::new(FP::from_int((t % 3) as i32 - 1), FP::ZERO, false),
                vec![],
            );
        }
    }
    fn assert_converged(&mut self, join: &mut JoinBootstrap<Arena, Mesh>) {
        let target = self.a.verified_tick() + 80;
        for _ in 0..180 {
            self.round(Some(join));
        }
        assert!(join.caught_up_to(target).unwrap());
        let j = join.session().unwrap();
        let common = self
            .a
            .verified_tick()
            .min(self.b.verified_tick())
            .min(j.verified_tick());
        let start = j.checksums()[0].0;
        for tick in start..=common {
            let sum = |p: &Peer| p.checksums().iter().find(|(t, _)| *t == tick).unwrap().1;
            assert_eq!(sum(&self.a), sum(&self.b), "existing peers tick {tick}");
            assert_eq!(sum(&self.a), sum(j), "joiner tick {tick}");
        }
        assert!(common > start + 80);
    }
}

use orr_proto::{Channel, ConnId, Endpoint, ServerEvent};
use std::collections::VecDeque;

// A fallible enqueue boundary, also exposing the existing Endpoint poll API.
// Production callers must use an equivalent acceptance-reporting send primitive.
#[derive(Default)]
struct ControlEndpoint {
    events: VecDeque<ServerEvent>,
    queued: Vec<(ConnId, Vec<u8>)>,
    backpressure: bool,
}
impl ControlEndpoint {
    fn enqueue(
        &mut self,
        conn: ConnId,
        channel: Channel,
        bytes: &[u8],
    ) -> Result<(), &'static str> {
        assert_eq!(channel, Channel::Reliable);
        if self.backpressure {
            return Err("backpressure");
        }
        self.queued.push((conn, bytes.to_vec()));
        Ok(())
    }
    fn receive(&mut self, conn: ConnId, bytes: Vec<u8>) {
        self.events.push_back(ServerEvent::Message {
            conn,
            channel: Channel::Reliable,
            data: bytes,
        });
    }
    fn control(&mut self, members: &mut P2pMembership) -> (ConnId, Vec<u8>) {
        match members.route_event(self.poll().unwrap()) {
            P2pRoutedEvent::Control { conn, data } => (conn, data),
            other => panic!("expected admitted control: {other:?}"),
        }
    }
}
impl Endpoint for ControlEndpoint {
    fn send(&mut self, conn: ConnId, channel: Channel, data: &[u8]) {
        let _ = self.enqueue(conn, channel, data);
    }
    fn poll(&mut self) -> Option<ServerEvent> {
        self.events.pop_front()
    }
    fn disconnect(&mut self, conn: ConnId) {
        self.events.push_back(ServerEvent::Disconnected(conn));
    }
}
fn conn(slot: u8) -> ConnId {
    ConnId(10 + u32::from(slot))
}
fn members(local: u8, budget: usize) -> P2pMembership {
    let mut m = P2pMembership::new(3, PlayerSlot(local), budget).unwrap();
    for slot in 0..2 {
        if slot == local {
            assert!(m.admit_local().unwrap().is_empty());
        } else {
            assert!(m
                .admit_active(PlayerSlot(slot), conn(slot))
                .unwrap()
                .is_empty());
        }
    }
    assert!(m.commit_vacant(JOINER).unwrap().is_empty());
    if local != 2 {
        assert!(m.admit_joiner(JOINER, conn(2)).unwrap().is_empty());
    }
    m
}
fn attempt(m: &mut P2pMembership) -> P2pAttempt {
    m.begin_attempt(JOINER, PlayerSlot(0), 7, 1).unwrap()
}
fn transmit(m: &mut P2pMembership, a: &P2pAttempt, target: u8, bytes: &[u8]) -> Vec<u8> {
    m.queue_control(a, PlayerSlot(target), bytes).unwrap();
    let mut endpoint = ControlEndpoint::default();
    let result = m.flush(|c, ch, data| endpoint.enqueue(c, ch, data));
    assert_eq!(result.queued, 1);
    assert!(result.blocked.is_empty());
    let (recipient, bytes) = endpoint.queued.pop().unwrap();
    assert_eq!(recipient, conn(target));
    bytes
}

#[test]
fn admitted_endpoint_controls_converge_without_promoting_pending_joiner() {
    let mut env = Env::new(512);
    let mut a = members(0, 1 << 20);
    let mut b = members(1, 1 << 20);
    let mut j = members(2, 1 << 20);
    assert_eq!(a.roster(JOINER, PlayerSlot(0)).unwrap(), roster());
    let aa = attempt(&mut a);
    let ba = attempt(&mut b);
    let ja = attempt(&mut j);
    let mut join = bootstrap::<Mesh>(4096);
    let request = join.next_request().unwrap();
    let mut endpoint = ControlEndpoint::default();
    endpoint.receive(conn(2), transmit(&mut j, &ja, 0, &request));
    let (sender, request) = endpoint.control(&mut a);
    let (snapshot, ticket) = a.serve_request(&aa, sender, &mut env.a, &request).unwrap();
    endpoint.receive(conn(0), transmit(&mut a, &aa, 1, &snapshot));
    let (sender, snapshot_for_peer) = endpoint.control(&mut b);
    let peer_ticket = b
        .import_ticket(&ba, sender, &env.b, &snapshot_for_peer, 1 << 20)
        .unwrap();
    let notices = [
        a.backlog_notice(&aa, &mut env.a, &ticket).unwrap(),
        b.backlog_notice(&ba, &mut env.b, &peer_ticket).unwrap(),
    ];
    let mut source = Mesh::default();
    for (i, peer) in [&mut env.a, &mut env.b].into_iter().enumerate() {
        let (mut outgoing, incoming) =
            LoopbackNetwork::with_clock::<Arena>(&env.clock, 2, 1, 121 + i as u64);
        for r in peer.authored_since(ticket.ticket().snapshot_tick) {
            outgoing.send_local(r.tick, r.slot, r.input, r.commands);
        }
        peer.source_mut().links.push(outgoing);
        source.links.push(incoming);
    }
    // Notice before snapshot and notice after snapshot both traverse the event boundary.
    endpoint.receive(conn(1), transmit(&mut b, &ba, 2, &notices[1]));
    let (sender, data) = endpoint.control(&mut j);
    j.receive_notice(&ja, sender, &mut join, &data).unwrap();
    endpoint.receive(conn(0), transmit(&mut a, &aa, 2, &snapshot));
    let (sender, data) = endpoint.control(&mut j);
    assert_eq!(
        j.receive_snapshot(&ja, sender, &mut join, game(), source, &data)
            .unwrap(),
        Status::Syncing {
            received: 1,
            expected: 2
        }
    );
    endpoint.receive(conn(0), transmit(&mut a, &aa, 2, &notices[0]));
    let (sender, data) = endpoint.control(&mut j);
    assert_eq!(
        j.receive_notice(&ja, sender, &mut join, &data).unwrap(),
        Status::Ready
    );
    for membership in [&a, &b, &j] {
        assert_eq!(membership.state(JOINER).unwrap(), SlotState::Vacant);
    }
    env.assert_converged(&mut join);
    // Promotion is a separate application decision and retires the old attempt.
    let cleanup = a.promote_joiner(JOINER).unwrap();
    assert_eq!(cleanup.len(), 1);
    assert_eq!(a.state(JOINER).unwrap(), SlotState::Active);
    assert_eq!(a.connection(JOINER).unwrap(), Some(conn(2)));
}

#[test]
fn connected_is_not_admission_and_complete_membership_is_required() {
    let mut m = P2pMembership::new(3, PlayerSlot(0), 4096).unwrap();
    let event = ServerEvent::Connected(conn(1));
    assert!(matches!(m.route_event(event.clone()), P2pRoutedEvent::Forward(e) if e == event));
    assert!(matches!(
        m.sender(conn(1)),
        Err(Error::UnknownConnection(_))
    ));
    assert!(m.admit_local().unwrap().is_empty());
    assert!(m.commit_vacant(JOINER).unwrap().is_empty());
    assert!(m.admit_joiner(JOINER, conn(2)).unwrap().is_empty());
    assert!(matches!(
        m.roster(JOINER, PlayerSlot(0)),
        Err(Error::IncompleteMembership)
    ));
    assert!(m.admit_active(PlayerSlot(1), conn(1)).unwrap().is_empty());
    assert_eq!(m.roster(JOINER, PlayerSlot(0)).unwrap(), roster());
    assert!(matches!(
        m.admit_active(JOINER, ConnId(99)),
        Err(Error::SlotCollision(_))
    ));
    assert!(matches!(
        m.replace_connection(PlayerSlot(1), conn(1), conn(2)),
        Err(Error::ConnectionCollision(_))
    ));
    assert_eq!(m.pending_count(), 0);
    let _a = attempt(&mut m);
    assert!(matches!(
        m.begin_attempt(JOINER, PlayerSlot(0), 7, 1),
        Err(Error::AttemptExists(_))
    ));
    assert_eq!(m.pending_count(), 1);
}

#[test]
fn unadmitted_or_wrong_sender_cannot_mutate_donor_or_bootstrap() {
    let mut env = Env::new(512);
    let mut m = members(0, 4096);
    let a = attempt(&mut m);
    let mut join = bootstrap::<Mesh>(4096);
    let request = join.next_request().unwrap();
    assert!(matches!(
        m.serve_request(&a, ConnId(99), &mut env.a, &request),
        Err(Error::UnknownConnection(_))
    ));
    assert!(matches!(
        m.serve_request(&a, conn(1), &mut env.a, &request),
        Err(Error::WrongSender)
    ));
    assert!(env.a.pending_join(JOINER).is_none());
    let event = ServerEvent::Message {
        conn: ConnId(99),
        channel: Channel::Reliable,
        data: request.clone(),
    };
    assert!(matches!(
        m.route_event(event),
        P2pRoutedEvent::Rejected {
            error: Error::UnknownConnection(_),
            ..
        }
    ));
    let event = ServerEvent::Message {
        conn: conn(2),
        channel: Channel::Unreliable,
        data: request,
    };
    assert!(matches!(
        m.route_event(event),
        P2pRoutedEvent::Rejected {
            error: Error::WrongControl,
            ..
        }
    ));
    let event = ServerEvent::Message {
        conn: ConnId(99),
        channel: Channel::Unreliable,
        data: vec![1, 2, 3],
    };
    assert!(matches!(m.route_event(event.clone()), P2pRoutedEvent::Forward(e) if e == event));
}

#[test]
fn disconnect_during_transfer_invalidates_without_reduced_count_ready() {
    let mut env = Env::new(512);
    let mut a = members(0, 1 << 20);
    let mut j = members(2, 1 << 20);
    let aa = attempt(&mut a);
    let ja = attempt(&mut j);
    let mut join = bootstrap::<Mesh>(4096);
    let request = join.next_request().unwrap();
    let (snapshot, ticket) = a.serve_request(&aa, conn(2), &mut env.a, &request).unwrap();
    let notice = a.backlog_notice(&aa, &mut env.a, &ticket).unwrap();
    j.receive_snapshot(&ja, conn(0), &mut join, game(), Mesh::default(), &snapshot)
        .unwrap();
    assert_eq!(
        j.receive_notice(&ja, conn(0), &mut join, &notice).unwrap(),
        Status::Syncing {
            received: 1,
            expected: 2
        }
    );
    let cleanup = match j.route_event(ServerEvent::Disconnected(conn(1))) {
        P2pRoutedEvent::Disconnected { cleanup, .. } => cleanup,
        other => panic!("{other:?}"),
    };
    assert_eq!(cleanup.len(), 1);
    assert_eq!(j.state(PlayerSlot(1)).unwrap(), SlotState::Unknown);
    assert!(matches!(
        j.receive_notice(&ja, conn(0), &mut join, &notice),
        Err(Error::Invalidated)
    ));
    assert_eq!(
        join.status().unwrap(),
        Status::Syncing {
            received: 1,
            expected: 2
        }
    );
    // The application owns this exact retired bootstrap; obligations do not
    // mutate arbitrary bootstraps sharing a wire context.
    let session = join.invalidate_membership();
    assert!(session.is_some());
    assert_eq!(join.status().unwrap(), Status::Invalidated);
    assert_eq!(cleanup[0].context.roster().peers().len(), 2);
    assert!(cleanup[0].connections.contains(&(PlayerSlot(1), conn(1))));
    assert!(matches!(
        j.begin_attempt(JOINER, PlayerSlot(0), 8, 1),
        Err(Error::IncompleteMembership)
    ));
}

#[test]
fn replacement_survives_stale_events_and_clears_queued_generation() {
    let mut env = Env::new(512);
    let mut m = members(0, 1 << 20);
    let a = attempt(&mut m);
    let mut join = bootstrap::<Mesh>(4096);
    let request = join.next_request().unwrap();
    let (snapshot, _) = m.serve_request(&a, conn(2), &mut env.a, &request).unwrap();
    m.queue_control(&a, JOINER, &snapshot).unwrap();
    let cleanup = m.replace_connection(JOINER, conn(2), ConnId(22)).unwrap();
    assert_eq!(cleanup.len(), 1);
    assert_eq!(cleanup[0].discarded_outbound_bytes, snapshot.len());
    assert!(cleanup[0].connections.contains(&(JOINER, conn(2))));
    assert_eq!(m.outbound_usage(), (0, 0));
    assert_eq!(
        m.flush::<()>(|_, _, _| panic!("stale generation sent"))
            .queued,
        0
    );
    assert!(m.disconnect(conn(2)).unwrap().is_empty());
    assert_eq!(m.connection(JOINER).unwrap(), Some(ConnId(22)));
    assert_eq!(m.state(JOINER).unwrap(), SlotState::Vacant);
    assert!(matches!(
        m.serve_request(&a, ConnId(22), &mut env.a, &request),
        Err(Error::Invalidated)
    ));
    let fresh = m.begin_attempt(JOINER, PlayerSlot(0), 8, 1).unwrap();
    assert!(matches!(
        m.serve_request(&fresh, conn(2), &mut env.a, &request),
        Err(Error::UnknownConnection(_))
    ));
    assert!(matches!(
        m.route_event(ServerEvent::Message {
            conn: conn(2),
            channel: Channel::Reliable,
            data: request
        }),
        P2pRoutedEvent::Rejected { .. }
    ));
    // Pending disconnect preserves the explicit vacancy, but no longer allows
    // a donor to derive a join roster without another application admission.
    assert_eq!(m.disconnect(ConnId(22)).unwrap().len(), 1);
    assert_eq!(m.state(JOINER).unwrap(), SlotState::Vacant);
    assert!(m.roster(JOINER, PlayerSlot(0)).is_err());
}

#[test]
fn backpressure_is_bounded_and_cleanup_remains_explicit() {
    let mut join = bootstrap::<Mesh>(4096);
    let request = join.next_request().unwrap();
    let mut m = members(2, request.len());
    let a = attempt(&mut m);
    m.queue_control(&a, PlayerSlot(0), &request).unwrap();
    assert!(matches!(
        m.queue_control(&a, PlayerSlot(0), &request),
        Err(Error::OutboundBudget { .. })
    ));
    let mut endpoint = ControlEndpoint {
        backpressure: true,
        ..Default::default()
    };
    let before = m.outbound_usage();
    for _ in 0..3 {
        let report = m.flush(|c, ch, bytes| endpoint.enqueue(c, ch, bytes));
        assert_eq!(report.queued, 0);
        assert_eq!(report.blocked, vec![(conn(0), "backpressure")]);
        assert_eq!(m.outbound_usage(), before);
    }
    assert_eq!(m.pending_count(), 1);
    assert!(endpoint.queued.is_empty());
    endpoint.backpressure = false;
    assert_eq!(
        m.flush(|c, ch, bytes| endpoint.enqueue(c, ch, bytes))
            .queued,
        1
    );
    assert_eq!(endpoint.queued[0].1, request);
    assert_eq!(m.pending_count(), 1); // enqueue success does not complete an attempt
    m.queue_control(&a, PlayerSlot(0), &request).unwrap();
    let cleanup = m.clear_vacancy(JOINER).unwrap();
    assert_eq!(cleanup.len(), 1);
    assert_eq!(cleanup[0].discarded_outbound_bytes, request.len());
    assert_eq!(m.pending_count(), 0);
    assert_eq!(m.outbound_usage(), (0, 0));
    assert_eq!(
        m.flush::<()>(|_, _, _| panic!("invalidated send")).queued,
        0
    );
}

#[test]
fn cancel_does_not_resurrect_old_token_when_same_context_is_registered() {
    let mut m = members(2, 4096);
    let old = attempt(&mut m);
    let cleanup = m.cancel_attempt(&old).unwrap();
    assert_eq!(cleanup.context, *old.context());
    let new = attempt(&mut m);
    let mut join = bootstrap::<Mesh>(4096);
    let request = join.next_request().unwrap();
    assert!(matches!(
        m.queue_control(&old, PlayerSlot(0), &request),
        Err(Error::AttemptMismatch)
    ));
    m.queue_control(&new, PlayerSlot(0), &request).unwrap();
    assert_eq!(m.pending_count(), 1);
}

#[test]
fn request_manifest_must_match_independently_derived_roster_before_grant() {
    let mut env = Env::new(512);
    let mut m = members(0, 4096);
    let a = attempt(&mut m);
    let wrong = JoinRoster::completed(
        3,
        JOINER,
        PlayerSlot(0),
        vec![PlayerSlot(0)],
        &[PlayerSlot(1)],
    )
    .unwrap();
    let mut join = JoinBootstrap::<Arena, Mesh>::new(cfg(2, 512), wrong, 4096, 1 << 20).unwrap();
    let request = join.next_request().unwrap();
    assert!(m.serve_request(&a, conn(2), &mut env.a, &request).is_err());
    assert!(env.a.pending_join(JOINER).is_none());
    assert_eq!(m.state(JOINER).unwrap(), SlotState::Vacant);
    assert_eq!(m.pending_count(), 1);
}

#[test]
fn explicit_additional_vacancies_and_multiple_attempts_are_bounded() {
    let mut m = P2pMembership::new(4, PlayerSlot(0), 4096).unwrap();
    assert!(m.admit_local().unwrap().is_empty());
    assert!(m.admit_active(PlayerSlot(1), conn(1)).unwrap().is_empty());
    for slot in [2, 3] {
        assert!(m.commit_vacant(PlayerSlot(slot)).unwrap().is_empty());
        assert!(m
            .admit_joiner(PlayerSlot(slot), conn(slot))
            .unwrap()
            .is_empty());
    }
    let a = m.begin_attempt(PlayerSlot(2), PlayerSlot(0), 7, 1).unwrap();
    let b = m.begin_attempt(PlayerSlot(3), PlayerSlot(0), 8, 1).unwrap();
    assert_eq!(
        a.context().roster().peers(),
        &[PlayerSlot(0), PlayerSlot(1)]
    );
    assert_eq!(
        b.context().roster().peers(),
        &[PlayerSlot(0), PlayerSlot(1)]
    );
    assert_eq!(m.pending_count(), 2);
    assert!(m.begin_attempt(PlayerSlot(2), PlayerSlot(0), 9, 1).is_err());
    assert!(m.admit_active(PlayerSlot(4), conn(4)).is_err());
    let cleanup = m.commit_vacant(PlayerSlot(1)).unwrap();
    assert_eq!(cleanup.len(), 2);
    assert_eq!(m.pending_count(), 0);
    for obligation in cleanup {
        assert_eq!(obligation.connections.len(), 3);
        assert!(obligation.connections.contains(&(PlayerSlot(1), conn(1))));
        assert_eq!(obligation.context.roster().peers().len(), 2);
    }
    // A repeated vacancy declaration must preserve a pending connection.
    let revision = m.revision();
    assert!(m.commit_vacant(JOINER).unwrap().is_empty());
    assert_eq!(m.revision(), revision);
    assert_eq!(m.connection(JOINER).unwrap(), Some(conn(2)));
}

#[test]
fn handles_are_local_to_the_registry_even_with_identical_counters() {
    let mut first = members(2, 4096);
    let mut second = members(2, 4096);
    let a = attempt(&mut first);
    let b = attempt(&mut second);
    assert_eq!(a.context(), b.context());
    assert_eq!(a.revision(), b.revision());
    let mut join = bootstrap::<Mesh>(4096);
    let request = join.next_request().unwrap();
    assert!(matches!(
        second.queue_control(&a, PlayerSlot(0), &request),
        Err(Error::AttemptMismatch)
    ));
    second.queue_control(&b, PlayerSlot(0), &request).unwrap();
    let _cleanup = first.cancel_attempt(&a).unwrap();
    assert!(matches!(
        second.cancel_attempt(&a),
        Err(Error::AttemptMismatch)
    ));
    assert_eq!(second.pending_count(), 1);
}

#[test]
fn fair_flush_retains_per_connection_fifo_and_sent_bytes_cannot_be_recalled() {
    let mut env = Env::new(512);
    let mut m = members(0, 1 << 20);
    let a = attempt(&mut m);
    let mut join = bootstrap::<Mesh>(4096);
    let request = join.next_request().unwrap();
    let (snapshot, ticket) = m.serve_request(&a, conn(2), &mut env.a, &request).unwrap();
    let notice = m.backlog_notice(&a, &mut env.a, &ticket).unwrap();
    m.queue_control(&a, JOINER, &snapshot).unwrap();
    m.queue_control(&a, PlayerSlot(1), &snapshot).unwrap();
    m.queue_control(&a, JOINER, &notice).unwrap();
    let mut called = Vec::new();
    let mut endpoint = ControlEndpoint::default();
    let result = m.flush(|c, channel, bytes| {
        called.push(c);
        if c == conn(2) {
            Err("blocked")
        } else {
            endpoint.enqueue(c, channel, bytes)
        }
    });
    assert_eq!(called, vec![conn(2), conn(1)]);
    assert_eq!(result.blocked, vec![(conn(2), "blocked")]);
    assert_eq!(result.queued, 1);
    assert_eq!(result.remaining_bytes, snapshot.len() + notice.len());
    assert_eq!(m.outbound_usage().0, 2);
    assert_eq!(endpoint.queued, vec![(conn(1), snapshot.clone())]);
    assert_eq!(
        m.flush(|c, ch, data| endpoint.enqueue(c, ch, data)).queued,
        2
    );
    assert_eq!(endpoint.queued[1], (conn(2), snapshot));
    assert_eq!(endpoint.queued[2], (conn(2), notice));
    let cleanup = m.cancel_attempt(&a).unwrap();
    assert_eq!(cleanup.discarded_outbound_bytes, 0);
    assert_eq!(endpoint.queued.len(), 3); // queued-to-transport is not delivery or recall
    assert!(cleanup.connections.contains(&(JOINER, conn(2))));
    env.a.release_join_hold(cleanup.context.roster().joiner());
    assert!(env.a.pending_join(JOINER).is_none());
}

#[test]
fn connection_bound_snapshot_and_notice_rejections_do_not_advance_join() {
    let mut env = Env::new(512);
    let mut a = members(0, 1 << 20);
    let mut j = members(2, 1 << 20);
    let aa = attempt(&mut a);
    let ja = attempt(&mut j);
    let mut join = bootstrap::<Mesh>(4096);
    let request = join.next_request().unwrap();
    let (snapshot, ticket) = a.serve_request(&aa, conn(2), &mut env.a, &request).unwrap();
    let notice = a.backlog_notice(&aa, &mut env.a, &ticket).unwrap();
    let before = join.status().unwrap();
    assert!(j
        .receive_snapshot(&ja, conn(1), &mut join, game(), Mesh::default(), &snapshot)
        .is_err());
    assert!(matches!(
        j.receive_snapshot(
            &ja,
            ConnId(99),
            &mut join,
            game(),
            Mesh::default(),
            &snapshot
        ),
        Err(Error::UnknownConnection(_))
    ));
    assert!(join.session().is_none());
    assert!(j.receive_notice(&ja, conn(1), &mut join, &notice).is_err());
    assert!(matches!(
        j.receive_notice(&ja, ConnId(99), &mut join, &notice),
        Err(Error::UnknownConnection(_))
    ));
    assert_eq!(join.notice_usage(), (0, 0));
    assert_eq!(join.status().unwrap(), before);
    j.receive_snapshot(&ja, conn(0), &mut join, game(), Mesh::default(), &snapshot)
        .unwrap();
    assert_eq!(
        j.receive_notice(&ja, conn(0), &mut join, &notice).unwrap(),
        Status::Syncing {
            received: 1,
            expected: 2
        }
    );
}
