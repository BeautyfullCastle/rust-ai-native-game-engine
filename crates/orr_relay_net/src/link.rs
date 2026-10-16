//! Client side: an `orr_proto::Link` over an `orr_net` client transport.

use std::collections::VecDeque;

use orr_net::{ConnId, ConnStats, DisconnectReason, Event, SendError, Transport};
use orr_proto::{Channel, ConnId as ProtoConnId, Link, LinkEvent, UNRELIABLE_MTU};
use orr_session::{
    P2pConnectionIssuer, P2pConnectionLease, P2pConnectionOwnership, P2pConnectionRetirement,
    P2pConnectionRetirer,
};

/// One client connection as an [`orr_proto::Link`].
///
/// Channel mapping is one to one. An unreliable message that does not fit
/// one datagram (`UNRELIABLE_MTU`, or the transport's own limit) goes on the
/// reliable channel instead, so it is never lost for size.
pub struct NetLink {
    t: Box<dyn Transport + Send>,
    conn: ConnId,
    up: bool,
    pending: VecDeque<LinkEvent>,
    last_reason: Option<DisconnectReason>,
    oversize_to_reliable: u64,
    issuer: P2pConnectionIssuer,
    exclusive: Option<P2pConnectionOwnership>,
    retired: bool,
}

impl NetLink {
    /// `conn` is `Endpoint::client_conn()` of the client endpoint inside `t`.
    pub fn new(t: impl Transport + Send + 'static, conn: ConnId) -> Self {
        Self::new_boxed(Box::new(t), conn)
    }

    pub(crate) fn new_boxed(t: Box<dyn Transport + Send>, conn: ConnId) -> Self {
        Self {
            t,
            conn,
            up: false,
            pending: VecDeque::new(),
            last_reason: None,
            oversize_to_reliable: 0,
            issuer: P2pConnectionIssuer::default(),
            exclusive: None,
            retired: false,
        }
    }

    /// Try one reliable enqueue, preserving the immediate transport result.
    /// Success is queue acceptance, not delivery. An optional conditioner keeps
    /// its own weaker acceptance semantics, just as with `NetEndpoint`.
    /// Before Connected and after disconnect/retirement, returns UnknownConnection.
    /// Legacy `Link::send` remains best effort and unchanged.
    pub fn try_send_reliable(&mut self, data: &[u8]) -> Result<(), SendError> {
        if !self.up || self.retired {
            return Err(SendError::UnknownConnection);
        }
        self.t.send(self.conn, orr_net::Channel::Reliable, data)
    }

    /// The sole link's local membership ID. It is scoped to this adapter only:
    /// applications combining endpoints must remap it into a unique namespace.
    pub fn connection_id(&self) -> ProtoConnId {
        ProtoConnId(1)
    }

    /// Claim this connected, otherwise unshared link for checked-join cleanup.
    /// Like NetEndpoint ownership, equal numeric IDs on another link do not match.
    pub fn claim_exclusive_connection(&mut self) -> Option<P2pConnectionLease> {
        if !self.up || self.retired || self.exclusive.is_some() {
            return None;
        }
        let (owner, lease) = self.issuer.issue(self.connection_id());
        self.exclusive = Some(owner);
        Some(lease)
    }

    /// Keep this exact live link after promotion; stale cleanup becomes a no-op.
    pub fn relinquish_exclusive_connection(&mut self, lease: &P2pConnectionLease) -> bool {
        if !self.owns(lease) {
            return false;
        }
        self.exclusive = None;
        true
    }

    fn owns(&self, lease: &P2pConnectionLease) -> bool {
        self.up
            && !self.retired
            && self
                .exclusive
                .as_ref()
                .is_some_and(|o| self.issuer.matches(o, lease))
    }

    /// Why the connection ended, once a `Disconnected` event was seen.
    pub fn disconnect_reason(&self) -> Option<&DisconnectReason> {
        self.last_reason.as_ref()
    }

    /// Counters of the live connection (round trip as the transport sees it, loss, bytes).
    pub fn stats(&self) -> Option<ConnStats> {
        self.t.stats(self.conn)
    }

    /// Unreliable sends that were moved to the reliable channel for their size.
    pub fn oversize_to_reliable(&self) -> u64 {
        self.oversize_to_reliable
    }
}

impl Link for NetLink {
    fn send(&mut self, channel: Channel, data: &[u8]) {
        if !self.up {
            return;
        }
        let channel = match channel {
            Channel::Reliable => orr_net::Channel::Reliable,
            Channel::Unreliable => {
                let limit = self.t.stats(self.conn).map_or(UNRELIABLE_MTU, |s| {
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
        match self.t.send(self.conn, channel, data) {
            Err(SendError::TooLarge { .. }) if channel == orr_net::Channel::Unreliable => {
                self.oversize_to_reliable += 1;
                let _ = self.t.send(self.conn, orr_net::Channel::Reliable, data);
            }
            // Backpressure, a closed connection: the relay protocol copes with lost messages.
            _ => {}
        }
    }

    fn poll(&mut self) -> Option<LinkEvent> {
        if let Some(e) = self.pending.pop_front() {
            return Some(e);
        }
        loop {
            match self.t.poll_event()? {
                Event::Connected { conn, .. } if conn == self.conn && !self.retired => {
                    self.up = true;
                    return Some(LinkEvent::Connected);
                }
                Event::Disconnected { conn, reason } if conn == self.conn => {
                    self.up = false;
                    self.exclusive = None;
                    self.last_reason = Some(reason);
                    return Some(LinkEvent::Disconnected);
                }
                Event::Message {
                    conn,
                    channel,
                    bytes,
                } if conn == self.conn && !self.retired => {
                    let channel = match channel {
                        orr_net::Channel::Reliable => Channel::Reliable,
                        orr_net::Channel::Unreliable => Channel::Unreliable,
                    };
                    return Some(LinkEvent::Message {
                        channel,
                        data: bytes,
                    });
                }
                _ => {}
            }
        }
    }

    fn close(&mut self) {
        self.t.close(self.conn);
    }
}

impl P2pConnectionRetirer for NetLink {
    fn retire_exclusive_connection(
        &mut self,
        lease: &P2pConnectionLease,
    ) -> P2pConnectionRetirement {
        if !self.owns(lease) {
            return P2pConnectionRetirement::NoMatchingConnection;
        }
        self.up = false;
        self.retired = true;
        self.exclusive = None;
        self.pending.clear();
        self.t.close(self.conn);
        P2pConnectionRetirement::Retired
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Wire {
        events: VecDeque<Event>,
        error: Option<SendError>,
        sends: Vec<(ConnId, orr_net::Channel, Vec<u8>)>,
        closed: Vec<ConnId>,
    }
    struct Mock(Arc<Mutex<Wire>>);
    impl Transport for Mock {
        fn poll_event(&mut self) -> Option<Event> {
            self.0.lock().unwrap().events.pop_front()
        }
        fn send(
            &mut self,
            conn: ConnId,
            channel: orr_net::Channel,
            bytes: &[u8],
        ) -> Result<(), SendError> {
            let mut wire = self.0.lock().unwrap();
            if let Some(error) = &wire.error {
                return Err(error.clone());
            }
            wire.sends.push((conn, channel, bytes.to_vec()));
            Ok(())
        }
        fn close(&mut self, conn: ConnId) {
            self.0.lock().unwrap().closed.push(conn);
        }
        fn stats(&self, _: ConnId) -> Option<ConnStats> {
            Some(ConnStats {
                max_unreliable_size: 1200,
                ..Default::default()
            })
        }
    }
    fn link() -> (NetLink, Arc<Mutex<Wire>>) {
        let wire = Arc::new(Mutex::new(Wire::default()));
        (NetLink::new(Mock(wire.clone()), 7), wire)
    }
    fn connected(link: &mut NetLink, wire: &Arc<Mutex<Wire>>) {
        wire.lock().unwrap().events.push_back(Event::Connected {
            conn: 7,
            peer: None,
        });
        assert_eq!(link.poll(), Some(LinkEvent::Connected));
    }
    #[test]
    fn fallible_reliable_preserves_errors_and_legacy_send_stays_best_effort() {
        let (mut link, wire) = link();
        assert_eq!(
            link.try_send_reliable(b"before"),
            Err(SendError::UnknownConnection)
        );
        connected(&mut link, &wire);
        for error in [
            SendError::Backpressure,
            SendError::TooLarge { max: 1 },
            SendError::UnknownConnection,
        ] {
            wire.lock().unwrap().error = Some(error.clone());
            assert_eq!(link.try_send_reliable(b"checked"), Err(error));
            link.send(Channel::Reliable, b"legacy"); // unchanged void/drop behavior
        }
        wire.lock().unwrap().error = None;
        link.try_send_reliable(b"accepted").unwrap();
        assert_eq!(
            wire.lock().unwrap().sends,
            [(7, orr_net::Channel::Reliable, b"accepted".to_vec())]
        );
        wire.lock().unwrap().events.push_back(Event::Disconnected {
            conn: 7,
            reason: DisconnectReason::RemoteClose,
        });
        assert_eq!(link.poll(), Some(LinkEvent::Disconnected));
        assert_eq!(
            link.try_send_reliable(b"after"),
            Err(SendError::UnknownConnection)
        );
    }
    #[test]
    fn exclusive_retirement_fences_buffered_traffic_and_is_identity_safe() {
        let (mut first, a) = link();
        let (mut other, b) = link();
        connected(&mut first, &a);
        connected(&mut other, &b);
        let old = first.claim_exclusive_connection().unwrap();
        assert_eq!(first.connection_id(), other.connection_id());
        assert_eq!(
            other.retire_exclusive_connection(&old),
            P2pConnectionRetirement::NoMatchingConnection
        );
        assert!(first.relinquish_exclusive_connection(&old));
        let current = first.claim_exclusive_connection().unwrap();
        assert_eq!(
            first.retire_exclusive_connection(&old),
            P2pConnectionRetirement::NoMatchingConnection
        );
        a.lock().unwrap().events.extend([
            Event::Message {
                conn: 7,
                channel: orr_net::Channel::Reliable,
                bytes: b"buffered".to_vec(),
            },
            Event::Connected {
                conn: 7,
                peer: None,
            },
            Event::Message {
                conn: 7,
                channel: orr_net::Channel::Reliable,
                bytes: b"late".to_vec(),
            },
        ]);
        assert_eq!(
            first.retire_exclusive_connection(&current),
            P2pConnectionRetirement::Retired
        );
        assert_eq!(
            first.retire_exclusive_connection(&current),
            P2pConnectionRetirement::NoMatchingConnection
        );
        assert!(first.poll().is_none());
        assert!(first.claim_exclusive_connection().is_none());
        assert_eq!(
            first.try_send_reliable(b"no"),
            Err(SendError::UnknownConnection)
        );
        assert_eq!(a.lock().unwrap().closed, [7]);
        other.try_send_reliable(b"unaffected").unwrap();
        assert_eq!(b.lock().unwrap().sends.len(), 1);
    }
    #[test]
    fn disconnect_expires_exclusive_ownership() {
        let (mut link, wire) = link();
        connected(&mut link, &wire);
        let lease = link.claim_exclusive_connection().unwrap();
        assert!(lease.is_live());
        wire.lock().unwrap().events.push_back(Event::Disconnected {
            conn: 7,
            reason: DisconnectReason::RemoteClose,
        });
        assert_eq!(link.poll(), Some(LinkEvent::Disconnected));
        assert!(!lease.is_live());
        assert_eq!(
            link.retire_exclusive_connection(&lease),
            P2pConnectionRetirement::NoMatchingConnection
        );
    }
}
