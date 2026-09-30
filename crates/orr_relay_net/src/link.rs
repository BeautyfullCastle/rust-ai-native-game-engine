//! Client side: an `orr_proto::Link` over an `orr_net` client transport.

use std::collections::VecDeque;

use orr_net::{ConnId, ConnStats, DisconnectReason, Event, SendError, Transport};
use orr_proto::{Channel, Link, LinkEvent, UNRELIABLE_MTU};

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
}

impl NetLink {
    /// `conn` is `Endpoint::client_conn()` of the client endpoint inside `t`.
    pub fn new(t: impl Transport + Send + 'static, conn: ConnId) -> Self {
        Self::new_boxed(Box::new(t), conn)
    }

    pub(crate) fn new_boxed(t: Box<dyn Transport + Send>, conn: ConnId) -> Self {
        Self { t, conn, up: false, pending: VecDeque::new(), last_reason: None, oversize_to_reliable: 0 }
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
                let limit = self.t.stats(self.conn).map_or(UNRELIABLE_MTU, |s| s.max_unreliable_size.min(UNRELIABLE_MTU));
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
                Event::Connected { conn, .. } if conn == self.conn => {
                    self.up = true;
                    return Some(LinkEvent::Connected);
                }
                Event::Disconnected { conn, reason } if conn == self.conn => {
                    self.up = false;
                    self.last_reason = Some(reason);
                    return Some(LinkEvent::Disconnected);
                }
                Event::Message { conn, channel, bytes } if conn == self.conn => {
                    let channel = match channel {
                        orr_net::Channel::Reliable => Channel::Reliable,
                        orr_net::Channel::Unreliable => Channel::Unreliable,
                    };
                    return Some(LinkEvent::Message { channel, data: bytes });
                }
                _ => {}
            }
        }
    }

    fn close(&mut self) {
        self.t.close(self.conn);
    }
}
