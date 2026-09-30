//! The abstract message-passing interface of the relay code.
//!
//! A transport gives each connection two channels: a reliable, ordered one
//! (for handshake, commands, snapshots, checksums) and an unreliable one
//! (for per-tick input and confirmed-tick traffic; a message may be lost,
//! duplicated or reordered). Messages are whole byte strings: the transport
//! frames them. Implement [`Link`] for a client connection and [`Endpoint`]
//! for a listening server.

/// Largest unreliable message a transport is expected to carry (one MTU
/// worth of payload). The relay code never sends a larger unreliable
/// message; the network simulator drops (and counts) one that is.
pub const UNRELIABLE_MTU: usize = 1200;

/// Which channel a message travels on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    /// Delivered exactly once and in order, or the connection fails.
    Reliable,
    /// Best effort: may be lost, duplicated or reordered.
    Unreliable,
}

/// Identifies one client connection at a server endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConnId(pub u32);

/// What a client [`Link`] reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkEvent {
    /// The connection to the server is up. Send `Hello` after this.
    Connected,
    /// The connection is gone (closed by either side, or timed out).
    Disconnected,
    Message { channel: Channel, data: Vec<u8> },
}

/// The client side of one connection to the server.
pub trait Link {
    /// Queues `data` on `channel`. Never blocks. A message sent while the
    /// connection is not up is dropped.
    fn send(&mut self, channel: Channel, data: &[u8]);
    /// Next pending event, or `None` when there is nothing new.
    fn poll(&mut self) -> Option<LinkEvent>;
    /// Closes the connection.
    fn close(&mut self);
}

/// What a server [`Endpoint`] reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerEvent {
    Connected(ConnId),
    Disconnected(ConnId),
    Message { conn: ConnId, channel: Channel, data: Vec<u8> },
}

/// The server side: many client connections.
pub trait Endpoint {
    /// Queues `data` for `conn` on `channel`. Unknown or closed
    /// connections are ignored.
    fn send(&mut self, conn: ConnId, channel: Channel, data: &[u8]);
    /// Next pending event, or `None`.
    fn poll(&mut self) -> Option<ServerEvent>;
    /// Closes `conn` (its `Disconnected` event is still reported).
    fn disconnect(&mut self, conn: ConnId);
}
