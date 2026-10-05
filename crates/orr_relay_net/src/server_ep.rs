//! Server side: an `orr_proto::Endpoint` over an `orr_net` listening transport.

use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;

use orr_net::{ConnId as NetConnId, Event, SendError, Transport};
use orr_proto::{Channel, ConnId, Endpoint, ServerEvent, UNRELIABLE_MTU};
use orr_session::{
    P2pConnectionIssuer, P2pConnectionLease, P2pConnectionOwnership, P2pConnectionRetirement,
    P2pConnectionRetirer,
};

/// A listening endpoint as an [`orr_proto::Endpoint`].
///
/// `orr_net` numbers connections with `u64`, `orr_proto` with `u32`. The
/// adapter hands out its own `u32` ids in connection order and never reuses
/// one. When the `u32` space is used up it refuses further connections
/// instead of wrapping. Messages of a connection that is not in the table
/// (already closed) are dropped.
pub struct NetEndpoint {
    t: Box<dyn Transport + Send>,
    local_addr: SocketAddr,
    cert_sha256: Option<[u8; 32]>,
    ws_addr: Option<SocketAddr>,
    wss_addr: Option<SocketAddr>,
    to_proto: BTreeMap<NetConnId, u32>,
    to_net: BTreeMap<u32, NetConnId>,
    ownership_issuer: P2pConnectionIssuer,
    exclusive: BTreeMap<u32, P2pConnectionOwnership>,
    next: u32,
    pending: VecDeque<ServerEvent>,
    refused: u64,
    oversize_to_reliable: u64,
}

impl NetEndpoint {
    pub(crate) fn new(
        t: Box<dyn Transport + Send>,
        local_addr: SocketAddr,
        cert_sha256: Option<[u8; 32]>,
    ) -> Self {
        Self {
            t,
            local_addr,
            cert_sha256,
            ws_addr: None,
            wss_addr: None,
            to_proto: BTreeMap::new(),
            to_net: BTreeMap::new(),
            ownership_issuer: P2pConnectionIssuer::default(),
            exclusive: BTreeMap::new(),
            next: 1,
            pending: VecDeque::new(),
            refused: 0,
            oversize_to_reliable: 0,
        }
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// SHA-256 of the self-signed certificate (QUIC with a generated certificate only).
    pub fn cert_sha256(&self) -> Option<[u8; 32]> {
        self.cert_sha256
    }

    pub(crate) fn set_ws_addr(&mut self, ws: Option<SocketAddr>, wss: Option<SocketAddr>) {
        self.ws_addr = ws;
        self.wss_addr = wss;
    }

    /// Address of the additional `wss://` listener, when `ListenOptions::wss_bind` was set.
    pub fn wss_addr(&self) -> Option<SocketAddr> {
        self.wss_addr
    }

    /// Address of the additional WebSocket listener, when `ListenOptions::ws_bind` was set.
    pub fn ws_addr(&self) -> Option<SocketAddr> {
        self.ws_addr
    }

    /// Connections refused because the `u32` id space was exhausted.
    pub fn refused(&self) -> u64 {
        self.refused
    }

    pub fn oversize_to_reliable(&self) -> u64 {
        self.oversize_to_reliable
    }

    /// Live connections.
    pub fn connection_count(&self) -> usize {
        self.to_net.len()
    }

    /// Transport counters of one live connection.
    pub fn stats(&self, conn: ConnId) -> Option<orr_net::ConnStats> {
        self.to_net.get(&conn.0).and_then(|&c| self.t.stats(c))
    }

    /// Try to enqueue one complete reliable message without blocking.
    ///
    /// Unlike the legacy [`Endpoint::send`], this preserves errors returned by
    /// the installed [`Transport`], including backpressure. Unknown/retired
    /// adapter connections return [`SendError::UnknownConnection`]. `Ok(())`
    /// means accepted by that immediate transport, not delivered or acknowledged
    /// by the peer. An error means no bytes were queued there, so callers may
    /// retain the message. Use as the callback for `P2pMembership::flush`.
    ///
    /// Wrappers retain their own semantics: the optional network conditioner
    /// accepts into its simulated queue and may hide later underlying send
    /// errors. This method does not strengthen a wrapper's delivery guarantees.
    pub fn try_send_reliable(&mut self, conn: ConnId, data: &[u8]) -> Result<(), SendError> {
        let net = *self
            .to_net
            .get(&conn.0)
            .ok_or(SendError::UnknownConnection)?;
        self.t.send(net, orr_net::Channel::Reliable, data)
    }

    /// Claim a live connection for exclusive join cleanup. Connections are
    /// shared/unowned by default. The application must establish that no other
    /// consumer needs this link before claiming it. Unknown/already-owned
    /// connections leave ownership unchanged and return `None`.
    ///
    /// Move the returned lease into `P2pMembership::register_exclusive_connection`.
    /// Registration failures return it for retry, relinquishment or retirement.
    /// Dropping an unregistered lease does not implicitly close the transport.
    pub fn claim_exclusive_connection(&mut self, conn: ConnId) -> Option<P2pConnectionLease> {
        if !self.to_net.contains_key(&conn.0) || self.exclusive.contains_key(&conn.0) {
            return None;
        }
        let (ownership, lease) = self.ownership_issuer.issue(conn);
        self.exclusive.insert(conn.0, ownership);
        Some(lease)
    }

    /// Promote this exact exclusive ownership to shared use without closing it.
    /// Old cleanup can no longer retire it, even if it is claimed again later.
    /// Call before membership promotion when the joining link will be retained.
    pub fn relinquish_exclusive_connection(&mut self, lease: &P2pConnectionLease) -> bool {
        if !self.owns_exclusive_connection(lease) {
            return false;
        }
        self.exclusive.remove(&lease.connection().0);
        true
    }

    fn owns_exclusive_connection(&self, lease: &P2pConnectionLease) -> bool {
        let conn = lease.connection().0;
        self.to_net.contains_key(&conn)
            && self
                .exclusive
                .get(&conn)
                .is_some_and(|owner| self.ownership_issuer.matches(owner, lease))
    }

    fn alloc(&mut self) -> Option<u32> {
        let id = self.next;
        if id == u32::MAX {
            return None;
        }
        self.next += 1;
        Some(id)
    }
}

impl P2pConnectionRetirer for NetEndpoint {
    /// Fence immediately at the adapter boundary, then request graceful close.
    /// Transport-buffered inbound messages are discarded when subsequently
    /// polled. Reliable outbound bytes already accepted by the transport may
    /// still flush; packets already delivered to the application cannot be
    /// retracted. Physical close completion is not acknowledged by this result.
    fn retire_exclusive_connection(
        &mut self,
        lease: &P2pConnectionLease,
    ) -> P2pConnectionRetirement {
        if !self.owns_exclusive_connection(lease) {
            return P2pConnectionRetirement::NoMatchingConnection;
        }
        let conn = lease.connection();
        let net = self
            .to_net
            .remove(&conn.0)
            .expect("validated live connection");
        self.to_proto.remove(&net);
        self.exclusive.remove(&conn.0);
        self.pending.retain(|event| match event {
            ServerEvent::Connected(id) | ServerEvent::Disconnected(id) => *id != conn,
            ServerEvent::Message { conn: id, .. } => *id != conn,
        });
        // Removal of both mappings suppresses the later transport disconnect,
        // without accumulating tombstones or waiting for its graceful close.
        self.pending.push_back(ServerEvent::Disconnected(conn));
        self.t.close(net);
        P2pConnectionRetirement::Retired
    }
}

impl Endpoint for NetEndpoint {
    fn send(&mut self, conn: ConnId, channel: Channel, data: &[u8]) {
        let Some(&net) = self.to_net.get(&conn.0) else {
            return;
        };
        let channel = match channel {
            Channel::Reliable => orr_net::Channel::Reliable,
            Channel::Unreliable => {
                let limit = self.t.stats(net).map_or(UNRELIABLE_MTU, |s| {
                    s.max_unreliable_size.min(UNRELIABLE_MTU)
                });
                if data.len() > limit {
                    self.oversize_to_reliable += 1;
                    orr_net::Channel::Reliable
                } else {
                    orr_net::Channel::Unreliable
                }
            }
        };
        if let Err(SendError::TooLarge { .. }) = self.t.send(net, channel, data) {
            if channel == orr_net::Channel::Unreliable {
                self.oversize_to_reliable += 1;
                let _ = self.t.send(net, orr_net::Channel::Reliable, data);
            }
        }
    }

    fn poll(&mut self) -> Option<ServerEvent> {
        if let Some(e) = self.pending.pop_front() {
            return Some(e);
        }
        loop {
            match self.t.poll_event()? {
                Event::Connected { conn, .. } => match self.alloc() {
                    Some(id) => {
                        self.to_proto.insert(conn, id);
                        self.to_net.insert(id, conn);
                        return Some(ServerEvent::Connected(ConnId(id)));
                    }
                    None => {
                        self.refused += 1;
                        self.t.close(conn);
                    }
                },
                Event::Disconnected { conn, .. } => {
                    if let Some(id) = self.to_proto.remove(&conn) {
                        self.to_net.remove(&id);
                        self.exclusive.remove(&id);
                        return Some(ServerEvent::Disconnected(ConnId(id)));
                    }
                }
                Event::Message {
                    conn,
                    channel,
                    bytes,
                } => {
                    if let Some(&id) = self.to_proto.get(&conn) {
                        let channel = match channel {
                            orr_net::Channel::Reliable => Channel::Reliable,
                            orr_net::Channel::Unreliable => Channel::Unreliable,
                        };
                        return Some(ServerEvent::Message {
                            conn: ConnId(id),
                            channel,
                            data: bytes,
                        });
                    }
                }
            }
        }
    }

    fn disconnect(&mut self, conn: ConnId) {
        if let Some(&net) = self.to_net.get(&conn.0) {
            self.t.close(net);
        }
    }
}

#[cfg(test)]
mod p2p_lifecycle_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use orr_session::{P2pAttempt, P2pMembership, P2pMembershipError, PlayerSlot};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct MockState {
        incoming: VecDeque<Event>,
        sent: Vec<(NetConnId, orr_net::Channel, Vec<u8>)>,
        closed: Vec<NetConnId>,
    }
    #[derive(Clone, Default)]
    struct MockTransport(Arc<Mutex<MockState>>);
    impl MockTransport {
        fn push(&self, event: Event) {
            self.0.lock().unwrap().incoming.push_back(event);
        }
        fn message(&self, conn: NetConnId, bytes: &[u8]) {
            self.push(Event::Message {
                conn,
                channel: orr_net::Channel::Reliable,
                bytes: bytes.to_vec(),
            });
        }
        fn disconnected(&self, conn: NetConnId) {
            self.push(Event::Disconnected {
                conn,
                reason: orr_net::DisconnectReason::LocalClose,
            });
        }
    }
    impl Transport for MockTransport {
        fn poll_event(&mut self) -> Option<Event> {
            self.0.lock().unwrap().incoming.pop_front()
        }
        fn send(
            &mut self,
            conn: NetConnId,
            channel: orr_net::Channel,
            bytes: &[u8],
        ) -> Result<(), SendError> {
            self.0
                .lock()
                .unwrap()
                .sent
                .push((conn, channel, bytes.to_vec()));
            Ok(())
        }
        fn close(&mut self, conn: NetConnId) {
            // Deliberately asynchronous: tests decide when close completes.
            self.0.lock().unwrap().closed.push(conn);
        }
        fn stats(&self, _: NetConnId) -> Option<orr_net::ConnStats> {
            Some(orr_net::ConnStats {
                max_unreliable_size: UNRELIABLE_MTU,
                ..Default::default()
            })
        }
    }
    fn endpoint(count: u32) -> (NetEndpoint, MockTransport) {
        let transport = MockTransport::default();
        let mut endpoint = NetEndpoint::new(
            Box::new(transport.clone()),
            "127.0.0.1:0".parse().unwrap(),
            None,
        );
        for id in 1..=count {
            transport.push(Event::Connected {
                conn: u64::from(id) + 100,
                peer: None,
            });
            assert!(matches!(endpoint.poll(), Some(ServerEvent::Connected(ConnId(c))) if c == id));
        }
        (endpoint, transport)
    }
    fn members() -> (P2pMembership, P2pAttempt) {
        let mut members = P2pMembership::new(3, PlayerSlot(0), 4096).unwrap();
        members.admit_local().unwrap();
        members.admit_active(PlayerSlot(1), ConnId(1)).unwrap();
        members.commit_vacant(PlayerSlot(2)).unwrap();
        members.admit_joiner(PlayerSlot(2), ConnId(2)).unwrap();
        let attempt = members
            .begin_attempt(PlayerSlot(2), PlayerSlot(0), 7, 1)
            .unwrap();
        (members, attempt)
    }
    fn assert_message(event: Option<ServerEvent>, conn: u32, bytes: &[u8]) {
        match event {
            Some(ServerEvent::Message {
                conn: id,
                channel: Channel::Reliable,
                data,
            }) => {
                assert_eq!(id, ConnId(conn));
                assert_eq!(data, bytes);
            }
            other => panic!("expected message, got {other:?}"),
        }
    }

    #[test]
    fn cleanup_fences_only_explicit_exclusive_connection_and_closes_once() {
        let (mut ep, transport) = endpoint(2);
        let (mut membership, attempt) = members();
        let lease = ep.claim_exclusive_connection(ConnId(2)).unwrap();
        assert!(ep.claim_exclusive_connection(ConnId(2)).is_none());
        membership
            .register_exclusive_connection(&attempt, lease)
            .unwrap();
        // A packet handed to the application before cleanup cannot be recalled.
        transport.message(102, b"already delivered");
        assert_message(ep.poll(), 2, b"already delivered");
        transport.message(102, b"transport queued, discarded");
        transport.message(101, b"shared inbound");
        ep.pending.push_back(ServerEvent::Message {
            conn: ConnId(2),
            channel: Channel::Reliable,
            data: b"adapter queued, discarded".to_vec(),
        });
        ep.pending.push_back(ServerEvent::Message {
            conn: ConnId(1),
            channel: Channel::Reliable,
            data: b"shared adapter queued".to_vec(),
        });
        let cleanup = membership.cancel_attempt(&attempt).unwrap();
        assert_eq!(cleanup.connections.len(), 2);
        assert_eq!(cleanup.exclusive_connections().len(), 1);
        assert_eq!(cleanup.retire_exclusive_connections(&mut ep), 1);
        assert_eq!(cleanup.retire_exclusive_connections(&mut ep), 0);
        assert!(!cleanup.exclusive_connections()[0].is_live());
        assert_eq!(ep.connection_count(), 1);
        assert!(ep.stats(ConnId(2)).is_none());
        ep.send(ConnId(2), Channel::Reliable, b"fenced");
        ep.send(ConnId(2), Channel::Unreliable, b"also fenced");
        ep.send(ConnId(1), Channel::Reliable, b"shared outbound");
        ep.disconnect(ConnId(2));
        assert_message(ep.poll(), 1, b"shared adapter queued");
        assert!(matches!(
            ep.poll(),
            Some(ServerEvent::Disconnected(ConnId(2)))
        ));
        assert_message(ep.poll(), 1, b"shared inbound");
        // Later packets and duplicate transport disconnects stay suppressed.
        transport.message(102, b"late input");
        transport.disconnected(102);
        transport.disconnected(102);
        transport.message(101, b"shared still works");
        assert_message(ep.poll(), 1, b"shared still works");
        assert!(ep.poll().is_none());
        let state = transport.0.lock().unwrap();
        assert_eq!(state.closed, vec![102]);
        assert_eq!(
            state.sent,
            vec![(101, orr_net::Channel::Reliable, b"shared outbound".to_vec())]
        );
    }

    #[test]
    fn foreign_endpoint_relinquishment_and_replacement_are_exactly_fenced() {
        let (mut ep, transport) = endpoint(2);
        let (mut foreign, foreign_transport) = endpoint(2);
        let foreign_lease = foreign.claim_exclusive_connection(ConnId(2)).unwrap();
        let (mut membership, attempt) = members();
        membership
            .register_exclusive_connection(
                &attempt,
                ep.claim_exclusive_connection(ConnId(2)).unwrap(),
            )
            .unwrap();
        let registered = &membership.exclusive_connections(&attempt).unwrap()[0];
        assert!(!foreign.relinquish_exclusive_connection(registered));
        assert!(ep.relinquish_exclusive_connection(registered));
        assert!(!ep.relinquish_exclusive_connection(registered));
        assert!(!registered.is_live());
        let replacement = ep.claim_exclusive_connection(ConnId(2)).unwrap();
        let cleanup = membership
            .promote_joiner(PlayerSlot(2))
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(cleanup.retire_exclusive_connections(&mut foreign), 0);
        assert_eq!(cleanup.retire_exclusive_connections(&mut ep), 0);
        assert!(foreign_lease.is_live());
        assert!(replacement.is_live());
        ep.send(ConnId(2), Channel::Reliable, b"replacement lives");
        foreign.send(ConnId(2), Channel::Reliable, b"foreign lives");
        assert!(transport.0.lock().unwrap().closed.is_empty());
        assert!(foreign_transport.0.lock().unwrap().closed.is_empty());
        assert_eq!(
            ep.retire_exclusive_connection(&replacement),
            P2pConnectionRetirement::Retired
        );
        assert_eq!(
            ep.retire_exclusive_connection(&replacement),
            P2pConnectionRetirement::NoMatchingConnection
        );
        assert_eq!(
            foreign.retire_exclusive_connection(&foreign_lease),
            P2pConnectionRetirement::Retired
        );
    }

    #[test]
    fn foreign_cleanup_first_does_not_consume_the_correct_owners_obligation() {
        let (mut ep, transport) = endpoint(2);
        let (mut foreign, foreign_transport) = endpoint(2);
        let foreign_lease = foreign.claim_exclusive_connection(ConnId(2)).unwrap();
        let (mut membership, attempt) = members();
        membership
            .register_exclusive_connection(
                &attempt,
                ep.claim_exclusive_connection(ConnId(2)).unwrap(),
            )
            .unwrap();
        let cleanup = membership.cancel_attempt(&attempt).unwrap();
        assert_eq!(cleanup.retire_exclusive_connections(&mut foreign), 0);
        assert_eq!(cleanup.retire_exclusive_connections(&mut ep), 1);
        assert_eq!(cleanup.retire_exclusive_connections(&mut ep), 0);
        assert_eq!(transport.0.lock().unwrap().closed, vec![102]);
        assert!(foreign_transport.0.lock().unwrap().closed.is_empty());
        assert!(foreign_lease.is_live());
    }

    #[test]
    fn shared_promotion_stays_usable_after_delayed_cleanup() {
        let (mut ep, transport) = endpoint(2);
        let (mut membership, attempt) = members();
        membership
            .register_exclusive_connection(
                &attempt,
                ep.claim_exclusive_connection(ConnId(2)).unwrap(),
            )
            .unwrap();
        assert!(ep.relinquish_exclusive_connection(
            &membership.exclusive_connections(&attempt).unwrap()[0]
        ));
        let cleanup = membership
            .promote_joiner(PlayerSlot(2))
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(cleanup.retire_exclusive_connections(&mut ep), 0);
        transport.message(102, b"shared after promotion");
        assert_message(ep.poll(), 2, b"shared after promotion");
        ep.send(ConnId(2), Channel::Unreliable, b"shared send");
        assert_eq!(transport.0.lock().unwrap().sent.len(), 1);
        assert!(transport.0.lock().unwrap().closed.is_empty());
    }

    #[test]
    fn natural_disconnect_and_new_incarnation_invalidate_old_lease() {
        let (mut ep, transport) = endpoint(2);
        let (mut membership, attempt) = members();
        membership
            .register_exclusive_connection(
                &attempt,
                ep.claim_exclusive_connection(ConnId(2)).unwrap(),
            )
            .unwrap();
        transport.disconnected(102);
        assert!(matches!(
            ep.poll(),
            Some(ServerEvent::Disconnected(ConnId(2)))
        ));
        transport.push(Event::Connected {
            conn: 103,
            peer: None,
        });
        assert!(matches!(ep.poll(), Some(ServerEvent::Connected(ConnId(3)))));
        let replacement = ep.claim_exclusive_connection(ConnId(3)).unwrap();
        let cleanup = membership
            .replace_connection(PlayerSlot(2), ConnId(2), ConnId(3))
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(cleanup.retire_exclusive_connections(&mut ep), 0);
        assert!(replacement.is_live());
        assert!(ep.claim_exclusive_connection(ConnId(2)).is_none());
        assert!(transport.0.lock().unwrap().closed.is_empty());
    }

    #[test]
    fn registration_failure_returns_live_ownership_without_changing_cleanup() {
        let (mut ep, transport) = endpoint(3);
        let (mut membership, attempt) = members();
        let (mut wrong_membership, wrong_attempt) = members();
        let lease = ep.claim_exclusive_connection(ConnId(2)).unwrap();
        let error = membership
            .register_exclusive_connection(&wrong_attempt, lease)
            .unwrap_err();
        assert!(matches!(error.error, P2pMembershipError::AttemptMismatch));
        assert!(error.lease.is_live());
        assert!(membership
            .exclusive_connections(&attempt)
            .unwrap()
            .is_empty());
        assert!(wrong_membership
            .cancel_attempt(&wrong_attempt)
            .unwrap()
            .exclusive_connections()
            .is_empty());
        membership
            .register_exclusive_connection(&attempt, error.lease)
            .unwrap();
        let error = membership
            .register_exclusive_connection(
                &attempt,
                ep.claim_exclusive_connection(ConnId(3)).unwrap(),
            )
            .unwrap_err();
        assert!(matches!(
            error.error,
            P2pMembershipError::UnknownConnection(ConnId(3))
        ));
        assert!(ep.relinquish_exclusive_connection(&error.lease));
        let cleanup = membership.cancel_attempt(&attempt).unwrap();
        let error = membership
            .register_exclusive_connection(
                &attempt,
                ep.claim_exclusive_connection(ConnId(1)).unwrap(),
            )
            .unwrap_err();
        assert!(matches!(error.error, P2pMembershipError::Invalidated));
        assert!(error.lease.is_live());
        assert_eq!(cleanup.retire_exclusive_connections(&mut ep), 1);
        assert_eq!(transport.0.lock().unwrap().closed, vec![102]);
        assert!(ep.relinquish_exclusive_connection(&error.lease));
    }

    #[test]
    fn registration_capacity_is_bounded_and_duplicate_ids_never_add_obligations() {
        let (mut ep, transport) = endpoint(3);
        let (mut foreign, _) = endpoint(2);
        let (mut membership, attempt) = members();
        for conn in [ConnId(1), ConnId(2)] {
            membership
                .register_exclusive_connection(
                    &attempt,
                    ep.claim_exclusive_connection(conn).unwrap(),
                )
                .unwrap();
        }
        let error = membership
            .register_exclusive_connection(
                &attempt,
                foreign.claim_exclusive_connection(ConnId(2)).unwrap(),
            )
            .unwrap_err();
        assert!(matches!(
            error.error,
            P2pMembershipError::ConnectionCollision(ConnId(2))
        ));
        assert!(foreign.relinquish_exclusive_connection(&error.lease));
        let error = membership
            .register_exclusive_connection(
                &attempt,
                ep.claim_exclusive_connection(ConnId(3)).unwrap(),
            )
            .unwrap_err();
        assert!(matches!(
            error.error,
            P2pMembershipError::ExclusiveConnectionLimit { limit: 2 }
        ));
        assert_eq!(membership.exclusive_connections(&attempt).unwrap().len(), 2);
        assert!(ep.relinquish_exclusive_connection(&error.lease));
        let cleanup = membership.cancel_attempt(&attempt).unwrap();
        assert_eq!(cleanup.retire_exclusive_connections(&mut ep), 2);
        assert_eq!(transport.0.lock().unwrap().closed, vec![101, 102]);
        let next = membership
            .begin_attempt(PlayerSlot(2), PlayerSlot(0), 7, 2)
            .unwrap();
        assert!(membership.exclusive_connections(&next).unwrap().is_empty());
    }

    #[test]
    fn expired_leases_and_dropped_endpoints_do_not_register() {
        let (mut ep, _) = endpoint(2);
        let (mut membership, attempt) = members();
        let lease = ep.claim_exclusive_connection(ConnId(2)).unwrap();
        assert!(ep.relinquish_exclusive_connection(&lease));
        let error = membership
            .register_exclusive_connection(&attempt, lease)
            .unwrap_err();
        assert!(matches!(
            error.error,
            P2pMembershipError::ConnectionLeaseExpired
        ));
        let lease = ep.claim_exclusive_connection(ConnId(2)).unwrap();
        drop(ep);
        assert!(!lease.is_live());
        let error = membership
            .register_exclusive_connection(&attempt, lease)
            .unwrap_err();
        assert!(matches!(
            error.error,
            P2pMembershipError::ConnectionLeaseExpired
        ));
        assert!(membership
            .cancel_attempt(&attempt)
            .unwrap()
            .exclusive_connections()
            .is_empty());
    }

    #[test]
    fn legacy_disconnect_waits_for_transport_and_preserves_queued_messages() {
        let (mut ep, transport) = endpoint(2);
        transport.message(102, b"legacy queued");
        ep.disconnect(ConnId(2));
        assert_eq!(ep.connection_count(), 2);
        ep.send(ConnId(2), Channel::Reliable, b"legacy send");
        assert_message(ep.poll(), 2, b"legacy queued");
        assert!(ep.poll().is_none());
        transport.disconnected(102);
        assert!(matches!(
            ep.poll(),
            Some(ServerEvent::Disconnected(ConnId(2)))
        ));
        assert!(ep.poll().is_none());
        assert_eq!(ep.connection_count(), 1);
        assert_eq!(transport.0.lock().unwrap().closed, vec![102]);
        assert_eq!(transport.0.lock().unwrap().sent.len(), 1);
    }
}
