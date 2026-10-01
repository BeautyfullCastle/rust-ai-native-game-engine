//! Server side: an `orr_proto::Endpoint` over an `orr_net` listening transport.

use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;

use orr_net::{ConnId as NetConnId, Event, SendError, Transport};
use orr_proto::{Channel, ConnId, Endpoint, ServerEvent, UNRELIABLE_MTU};

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
    next: u32,
    pending: VecDeque<ServerEvent>,
    refused: u64,
    oversize_to_reliable: u64,
}

impl NetEndpoint {
    pub(crate) fn new(t: Box<dyn Transport + Send>, local_addr: SocketAddr, cert_sha256: Option<[u8; 32]>) -> Self {
        Self {
            t,
            local_addr,
            cert_sha256,
            ws_addr: None,
            wss_addr: None,
            to_proto: BTreeMap::new(),
            to_net: BTreeMap::new(),
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

    fn alloc(&mut self) -> Option<u32> {
        let id = self.next;
        if id == u32::MAX {
            return None;
        }
        self.next += 1;
        Some(id)
    }
}

impl Endpoint for NetEndpoint {
    fn send(&mut self, conn: ConnId, channel: Channel, data: &[u8]) {
        let Some(&net) = self.to_net.get(&conn.0) else { return };
        let channel = match channel {
            Channel::Reliable => orr_net::Channel::Reliable,
            Channel::Unreliable => {
                let limit = self.t.stats(net).map_or(UNRELIABLE_MTU, |s| s.max_unreliable_size.min(UNRELIABLE_MTU));
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
                        return Some(ServerEvent::Disconnected(ConnId(id)));
                    }
                }
                Event::Message { conn, channel, bytes } => {
                    if let Some(&id) = self.to_proto.get(&conn) {
                        let channel = match channel {
                            orr_net::Channel::Reliable => Channel::Reliable,
                            orr_net::Channel::Unreliable => Channel::Unreliable,
                        };
                        return Some(ServerEvent::Message { conn: ConnId(id), channel, data: bytes });
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
