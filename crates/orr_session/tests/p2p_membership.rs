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
    assert_eq!(
        cleanup.release_local_hold(&mut env.a),
        orr_session::JoinHoldRelease::ReleasedAssignmentRetained
    );
    assert!(env.a.pending_join(JOINER).is_some());
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

// The fixture retains the two existing peers' shared input link. Only a new
// attempt's exclusive links are attached here; cleanup never closes the mesh.
fn attach_join_backlogs(env: &mut Env, snapshot_tick: u64) -> Mesh {
    let mut source = Mesh::default();
    for (i, peer) in [&mut env.a, &mut env.b].into_iter().enumerate() {
        let (mut outgoing, incoming) =
            LoopbackNetwork::with_clock::<Arena>(&env.clock, 2, 1, 301 + i as u64);
        for r in peer.authored_since(snapshot_tick) {
            outgoing.send_local(r.tick, r.slot, r.input, r.commands);
        }
        peer.source_mut().links.push(outgoing);
        source.links.push(incoming);
    }
    source
}

#[test]
fn cancelled_slow_transfer_releases_real_retention_on_ordinary_poll() {
    use orr_session::JoinHoldRelease as Release;
    let mut env = Env::new(4);
    // Existing-peer input is delayed while the donor authors ahead. This gives
    // a real verified snapshot older than the defaults already in flight.
    for _ in 0..80 {
        drive(&mut env.a);
    }
    let mut a = members(0, 1 << 20);
    let mut b = members(1, 1 << 20);
    let aa = attempt(&mut a);
    let ba = attempt(&mut b);
    let mut join = bootstrap::<Mesh>(4096);
    let request = join.next_request().unwrap();
    let (snapshot, ticket) = a.serve_request(&aa, conn(2), &mut env.a, &request).unwrap();
    let peer_ticket = b
        .import_ticket(&ba, conn(0), &env.b, &snapshot, 1 << 20)
        .unwrap();
    b.backlog_notice(&ba, &mut env.b, &peer_ticket).unwrap();
    let start = ticket.ticket().snapshot_tick;
    for _ in 0..120 {
        env.round(None);
    }
    for peer in [&env.a, &env.b] {
        assert!(peer.verified_tick() > start + 40);
        assert_eq!(peer.authored_since(start).first().unwrap().tick, start + 1);
    }
    let a_before = (
        env.a.source().sent.clone(),
        env.a.next_send_tick(),
        env.a.predicted_frame().checksum(),
    );
    let cleanup_a = a.cancel_attempt(&aa).unwrap();
    let cleanup_b = b.cancel_attempt(&ba).unwrap();
    assert_eq!(
        cleanup_a.release_local_hold(&mut env.a),
        Release::ReleasedAssignmentRetained
    );
    assert_eq!(cleanup_b.release_local_hold(&mut env.b), Release::Released);
    assert_eq!(env.a.pending_join(JOINER), Some(ticket.ticket()));
    assert_eq!(
        a_before,
        (
            env.a.source().sent.clone(),
            env.a.next_send_tick(),
            env.a.predicted_frame().checksum()
        )
    );
    assert_eq!(a.connection(PlayerSlot(1)).unwrap(), Some(conn(1)));
    for peer in [&mut env.a, &mut env.b] {
        // The lease only releases retention; normal polling performs the prune.
        assert_eq!(peer.authored_since(start).first().unwrap().tick, start + 1);
        peer.poll_confirmed();
        assert!(peer.authored_since(start).first().unwrap().tick >= peer.verified_tick() - 4);
        assert!(peer.backlog_notice(JOINER).is_none());
        assert_eq!(peer.source().links.len(), 1);
    }
    assert_eq!(
        cleanup_a.release_local_hold(&mut env.a),
        Release::NoMatchingHold
    );
}

#[test]
fn delayed_cleanup_cannot_release_equal_ticket_replacement_or_foreign_session() {
    use orr_session::JoinHoldRelease as Release;
    let mut env = Env::new(512);
    let mut donor = members(0, 1 << 20);
    let da = attempt(&mut donor);
    let mut join = bootstrap::<Mesh>(4096);
    let (_, ticket) = donor
        .serve_request(&da, conn(2), &mut env.a, &join.next_request().unwrap())
        .unwrap();
    let mut b = members(1, 1 << 20);
    let old = attempt(&mut b);
    b.backlog_notice(&old, &mut env.b, &ticket).unwrap();
    let cleanup_old = b.cancel_attempt(&old).unwrap();
    // Deliberately reuse equal control metadata to prove local identity fencing.
    // Real wire retries still require a fresh attempt or generation.
    let new = attempt(&mut b);
    b.backlog_notice(&new, &mut env.b, &ticket).unwrap();
    let mut foreign = Session::new(game(), cfg(1, 512), Mesh::default());
    foreign.hold_inputs_for_join(ticket.ticket());
    assert_eq!(
        cleanup_old.release_local_hold(&mut foreign),
        Release::NoMatchingHold
    );
    assert_eq!(
        cleanup_old.release_local_hold(&mut env.b),
        Release::NoMatchingHold
    );
    assert!(env.b.backlog_notice(JOINER).is_some());
    assert!(foreign.backlog_notice(JOINER).is_some());
    let cleanup_new = b.cancel_attempt(&new).unwrap();
    assert_eq!(
        cleanup_new.release_local_hold(&mut foreign),
        Release::NoMatchingHold
    );
    assert_eq!(
        cleanup_new.release_local_hold(&mut env.b),
        Release::Released
    );
    assert_eq!(
        cleanup_new.release_local_hold(&mut env.b),
        Release::NoMatchingHold
    );
    // Replacing the entire Session with the same config/ticket is also fenced.
    env.b = Session::new(game(), cfg(1, 512), Mesh::default());
    env.b.hold_inputs_for_join(ticket.ticket());
    assert_eq!(
        cleanup_new.release_local_hold(&mut env.b),
        Release::NoMatchingHold
    );
    assert!(env.b.backlog_notice(JOINER).is_some());
}

#[test]
fn queue_budget_and_enqueue_failure_preserve_exact_cleanup_ownership() {
    use orr_session::JoinHoldRelease as Release;
    let mut env = Env::new(512);
    let mut a = members(0, 1);
    let aa = attempt(&mut a);
    let mut join = bootstrap::<Mesh>(4096);
    let (snapshot, ticket) = a
        .serve_request(&aa, conn(2), &mut env.a, &join.next_request().unwrap())
        .unwrap();
    assert!(matches!(
        a.queue_control(&aa, JOINER, &snapshot),
        Err(Error::OutboundBudget { .. })
    ));
    let cleanup_a = a.cancel_attempt(&aa).unwrap();
    assert_eq!(cleanup_a.discarded_outbound_bytes, 0);
    assert_eq!(
        cleanup_a.release_local_hold(&mut env.a),
        Release::ReleasedAssignmentRetained
    );
    assert_eq!(env.a.pending_join(JOINER), Some(ticket.ticket()));

    let mut b = members(1, 1 << 20);
    let ba = attempt(&mut b);
    let notice = b.backlog_notice(&ba, &mut env.b, &ticket).unwrap();
    b.queue_control(&ba, JOINER, &notice).unwrap();
    let result = b.flush(|_, _, _| Err("queue full"));
    assert_eq!(result.queued, 0);
    assert_eq!(result.blocked.len(), 1);
    let cleanup_b = b.cancel_attempt(&ba).unwrap();
    assert_eq!(cleanup_b.discarded_outbound_bytes, notice.len());
    assert_eq!(cleanup_b.release_local_hold(&mut env.b), Release::Released);
}

#[test]
fn pre_ready_cancel_retains_assignment_and_higher_attempt_converges() {
    use orr_session::JoinHoldRelease as Release;
    let mut env = Env::new(512);
    let mut a = members(0, 1 << 20);
    let mut b = members(1, 1 << 20);
    let mut j = members(2, 1 << 20);
    let aa = attempt(&mut a);
    let ba = attempt(&mut b);
    let ja = attempt(&mut j);
    let mut join = bootstrap::<Mesh>(4096);
    let (snapshot, ticket) = a
        .serve_request(&aa, conn(2), &mut env.a, &join.next_request().unwrap())
        .unwrap();
    let notice = a.backlog_notice(&aa, &mut env.a, &ticket).unwrap();
    b.backlog_notice(&ba, &mut env.b, &ticket).unwrap();
    j.receive_snapshot(&ja, conn(0), &mut join, game(), Mesh::default(), &snapshot)
        .unwrap();
    j.receive_notice(&ja, conn(0), &mut join, &notice).unwrap();
    for _ in 0..25 {
        env.round(Some(&mut join));
    }
    assert!(matches!(join.status().unwrap(), Status::Syncing { .. }));
    assert!(join.session().unwrap().source().sent.is_empty());
    let ca = a.cancel_attempt(&aa).unwrap();
    let cb = b.cancel_attempt(&ba).unwrap();
    let _cj = j.cancel_attempt(&ja).unwrap();
    assert_eq!(
        ca.release_local_hold(&mut env.a),
        Release::ReleasedAssignmentRetained
    );
    assert_eq!(cb.release_local_hold(&mut env.b), Release::Released);
    drop(join.cancel());
    assert_eq!(env.a.pending_join(JOINER), Some(ticket.ticket()));
    let aa = a.begin_attempt(JOINER, PlayerSlot(0), 7, 2).unwrap();
    let ba = b.begin_attempt(JOINER, PlayerSlot(0), 7, 2).unwrap();
    let ja = j.begin_attempt(JOINER, PlayerSlot(0), 7, 2).unwrap();
    let (snapshot, ticket) = a
        .serve_request(&aa, conn(2), &mut env.a, &join.next_request().unwrap())
        .unwrap();
    assert_eq!(ticket.ticket().attempt, 2);
    let notices = [
        a.backlog_notice(&aa, &mut env.a, &ticket).unwrap(),
        b.backlog_notice(&ba, &mut env.b, &ticket).unwrap(),
    ];
    let source = attach_join_backlogs(&mut env, ticket.ticket().snapshot_tick);
    j.receive_snapshot(&ja, conn(0), &mut join, game(), source, &snapshot)
        .unwrap();
    for (i, notice) in notices.iter().enumerate() {
        j.receive_notice(&ja, conn(i as u8), &mut join, notice)
            .unwrap();
    }
    assert_eq!(join.status().unwrap(), Status::Ready);
    // Delayed duplicates from the abandoned attempt cannot damage this retry.
    assert_eq!(ca.release_local_hold(&mut env.a), Release::NoMatchingHold);
    assert_eq!(cb.release_local_hold(&mut env.b), Release::NoMatchingHold);
    env.assert_converged(&mut join);
}

#[test]
fn input_arrival_after_ready_cleanup_confirms_assignment_and_prohibits_retry() {
    use orr_session::{CheckedJoinError, JoinError, JoinHoldRelease as Release};
    let mut env = Env::new(512);
    let mut a = members(0, 1 << 20);
    let mut b = members(1, 1 << 20);
    let aa = attempt(&mut a);
    let ba = attempt(&mut b);
    let mut join = bootstrap::<Mesh>(4096);
    let (snapshot, ticket) = a
        .serve_request(&aa, conn(2), &mut env.a, &join.next_request().unwrap())
        .unwrap();
    let notices = [
        a.backlog_notice(&aa, &mut env.a, &ticket).unwrap(),
        b.backlog_notice(&ba, &mut env.b, &ticket).unwrap(),
    ];
    let source = attach_join_backlogs(&mut env, ticket.ticket().snapshot_tick);
    join.receive_snapshot(PlayerSlot(0), game(), source, &snapshot)
        .unwrap();
    for (i, notice) in notices.iter().enumerate() {
        join.receive_notice(PlayerSlot(i as u8), notice).unwrap();
    }
    assert_eq!(join.status().unwrap(), Status::Ready);
    assert!(env.a.pending_join(JOINER).is_some()); // input not delivered yet
    let sent = env.a.source().sent.clone();
    let next = env.a.next_send_tick();
    let ca = a.cancel_attempt(&aa).unwrap();
    let cb = b.cancel_attempt(&ba).unwrap();
    assert_eq!(
        ca.release_local_hold(&mut env.a),
        Release::ReleasedAssignmentRetained
    );
    assert_eq!(cb.release_local_hold(&mut env.b), Release::Released);
    assert_eq!(env.a.source().sent, sent);
    assert_eq!(env.a.next_send_tick(), next);
    assert_eq!(env.a.source().links.len(), 2);
    env.assert_converged(&mut join);
    assert!(env.a.pending_join(JOINER).is_none()); // retained grant was confirmed
    let existing_input = env.a.last_remote_tick(JOINER).unwrap();
    let authored = env.a.source().sent.clone();
    // Retry must not author defaults over the active joiner's input.
    drop(join.cancel());
    let aa2 = a.begin_attempt(JOINER, PlayerSlot(0), 7, 2).unwrap();
    let retry = join.next_request().unwrap();
    assert!(matches!(
        a.serve_request(&aa2, conn(2), &mut env.a, &retry),
        Err(Error::Checked(CheckedJoinError::Join(
            JoinError::SlotNotVacant(JOINER)
        )))
    ));
    assert_eq!(env.a.last_remote_tick(JOINER), Some(existing_input));
    assert_eq!(env.a.source().sent, authored);
    assert_eq!(ca.release_local_hold(&mut env.a), Release::NoMatchingHold);
    assert!(env.a.backlog_notice(JOINER).is_none());
}

#[test]
fn cloned_owned_lease_is_idempotent_and_legacy_release_remains_compatible() {
    use orr_session::JoinHoldRelease as Release;
    let mut env = Env::new(512);
    let mut a = members(0, 1 << 20);
    let aa = attempt(&mut a);
    let mut join = bootstrap::<Mesh>(4096);
    let (_, ticket) = a
        .serve_request(&aa, conn(2), &mut env.a, &join.next_request().unwrap())
        .unwrap();
    let old = env.a.join_hold_lease(JOINER).unwrap();
    let fresh = env.a.hold_inputs_for_join_owned(ticket.ticket());
    assert_eq!(env.a.release_owned_join_hold(&old), Release::NoMatchingHold);
    assert_eq!(
        env.a.release_owned_join_hold(&fresh.clone()),
        Release::ReleasedAssignmentRetained
    );
    assert_eq!(
        env.a.release_owned_join_hold(&fresh),
        Release::NoMatchingHold
    );
    assert!(env.a.pending_join(JOINER).is_some());
    env.a.hold_inputs_for_join(ticket.ticket());
    env.a.release_join_hold(JOINER);
    assert!(env.a.backlog_notice(JOINER).is_none());
    assert!(env.a.pending_join(JOINER).is_none());
}

#[test]
fn donor_rejects_live_foreign_session_and_accepts_expired_owner_replacement() {
    use orr_session::JoinHoldRelease as Release;
    let mut env = Env::new(512);
    let mut m = members(0, 1 << 20);
    let a = attempt(&mut m);
    let mut join = bootstrap::<Mesh>(4096);
    let request = join.next_request().unwrap();
    m.serve_request(&a, conn(2), &mut env.a, &request).unwrap();
    let mut config = cfg(0, 512);
    config.vacant_slots.push(JOINER);
    let mut foreign = Session::new(game(), config.clone(), Mesh::default());
    assert!(matches!(
        m.serve_request(&a, conn(2), &mut foreign, &request),
        Err(Error::LocalSessionMismatch)
    ));
    assert!(foreign.pending_join(JOINER).is_none());
    assert!(foreign.join_hold_lease(JOINER).is_none());
    // Replace the whole owner, expiring the registry's weak lease. The same
    // attempt can now capture the new Session without accumulating identities.
    env.a = Session::new(game(), config, Mesh::default());
    m.serve_request(&a, conn(2), &mut env.a, &request).unwrap();
    let cleanup = m.cancel_attempt(&a).unwrap();
    assert_eq!(
        cleanup.release_local_hold(&mut foreign),
        Release::NoMatchingHold
    );
    assert_eq!(
        cleanup.release_local_hold(&mut env.a),
        Release::ReleasedAssignmentRetained
    );
}

#[test]
fn failed_checked_calls_never_adopt_unrelated_preexisting_holds() {
    use orr_session::{CheckedJoinContext, JoinHoldRelease as Release};
    let mut env = Env::new(512);
    let mut a = members(0, 1 << 20);
    let aa = attempt(&mut a);
    let mut join = bootstrap::<Mesh>(4096);
    let request = join.next_request().unwrap();
    // An independently installed donor hold/grant, not created by the router.
    let (_, ticket) = orr_session::serve_checked_join(&mut env.a, aa.context(), &request).unwrap();
    let old = env.a.join_hold_lease(JOINER).unwrap();
    assert!(a.serve_request(&aa, conn(2), &mut env.a, &request).is_err());
    let cleanup_a = a.cancel_attempt(&aa).unwrap();
    assert_eq!(
        cleanup_a.release_local_hold(&mut env.a),
        Release::NoMatchingHold
    );
    assert!(env.a.join_hold_lease(JOINER).is_some());
    assert_eq!(
        env.a.release_owned_join_hold(&old),
        Release::ReleasedAssignmentRetained
    );

    let context = CheckedJoinContext::new(7, 1, roster()).unwrap();
    orr_session::checked_backlog_notice(&mut env.b, &context, &ticket).unwrap();
    let old = env.b.join_hold_lease(JOINER).unwrap();
    let mut b = members(1, 1 << 20);
    let ba = b.begin_attempt(JOINER, PlayerSlot(0), 8, 1).unwrap();
    assert!(b.backlog_notice(&ba, &mut env.b, &ticket).is_err());
    let cleanup_b = b.cancel_attempt(&ba).unwrap();
    assert_eq!(
        cleanup_b.release_local_hold(&mut env.b),
        Release::NoMatchingHold
    );
    assert_eq!(env.b.release_owned_join_hold(&old), Release::Released);
}
