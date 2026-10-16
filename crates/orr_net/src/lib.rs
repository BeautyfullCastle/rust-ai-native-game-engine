//! `orr_net`: transport layer for Orrery multiplayer.
//!
//! The crate carries **opaque byte messages** between one server endpoint and
//! many client endpoints. It knows nothing about the relay or game protocol.
//!
//! * [`Endpoint`] is the concrete type. Build a server with
//!   [`Endpoint::listen_quic`] or [`Endpoint::listen_ws`], and a client with
//!   [`Endpoint::connect_quic`], [`Endpoint::connect_ws`] or [`Endpoint::connect_wt`].
//! * The caller's view is synchronous and never blocks: call
//!   [`Endpoint::poll_event`] / [`Endpoint::drain_events`] from the game loop
//!   and [`Endpoint::send`] to queue messages. Async I/O runs on a private
//!   tokio runtime owned by the endpoint.
//! * [`Transport`] is the trait implemented by [`Endpoint`] (and by
//!   [`Conditioned`], with the `conditioner` feature) for code that is generic
//!   over the backend.
//! * [`timesync::TimeSync`] is a pure NTP-style RTT/offset estimator.
//!
//! # Channels
//!
//! | Backend   | `Reliable`                                  | `Unreliable`                                      |
//! |-----------|---------------------------------------------|---------------------------------------------------|
//! | QUIC      | one ordered bidirectional stream, framed    | QUIC datagrams (no retransmit, may reorder/drop)  |
//! | WebTransport | one bidirectional stream, framed as QUIC | WebTransport datagrams                       |
//! | WebSocket | the WS stream                               | best effort over the same ordered stream          |
//!
//! Over WebSocket the unreliable channel cannot lose or reorder messages on
//! the wire, because TCP is under it. It only drops a message on the sender
//! when the outgoing queue is already longer than
//! [`NetConfig::unreliable_backlog_limit`]. Head-of-line blocking can still
//! delay it. If a QUIC peer does not support datagrams, unreliable messages
//! fall back to the reliable stream when [`NetConfig::datagram_fallback`] is
//! set. The receiver still sees them tagged [`Channel::Unreliable`].
//!
//! # Wire framing (both backends)
//!
//! Reliable stream frame: `u32 LE length | u8 tag | payload`, where `length`
//! counts the tag and payload. Tags: `0` reliable message, `1` unreliable
//! message carried on the stream, `2` hello (first frame from the client,
//! payload `b"ORRN\x01"`). On WebSocket, each binary message is `tag | payload`
//! (the WS frame already has a length). QUIC datagrams carry the raw payload.
//! ALPN for QUIC is `orrery/1`; WebSocket uses sub-protocol `orrery/1`.
//!
//! # Browsers
//!
//! A browser connects over WebTransport (HTTP/3 on QUIC) or, where UDP is
//! blocked or the browser lacks it, a WebSocket. Both are server-side
//! backends of this crate; the browser's code is `orr_web`.
//!
//! * **WebTransport** shares the UDP port of the QUIC backend:
//!   [`Endpoint::listen_quic_with`] with `webtransport = true` offers the ALPNs
//!   `orrery/1` and `h3`, and each connection goes to the backend its ALPN
//!   names. The session is an HTTP/3 extended CONNECT; the reliable channel is
//!   the first bidirectional stream the browser opens (hello frame first, then
//!   the same framing as QUIC), the unreliable channel is WebTransport
//!   datagrams (a browser limits them, 1024 bytes in Chrome). Built on
//!   `h3` + `h3-webtransport` + `h3-quinn` (draft-02 style WebTransport, what
//!   Chrome and Firefox speak; Safari is untested, see `docs/webtransport-trial.md`).
//!   A dev certificate for `serverCertificateHashes` is
//!   [`QuicServerTls::SelfSignedWebTransport`] (ECDSA P-256, 13 days).
//! * **WebSocket**: [`Endpoint::add_ws_listener`] / [`Endpoint::add_wss_listener`]
//!   add plain and TLS WebSocket listeners to the same endpoint. A browser cannot
//!   offer the sub-protocol `orrery/1` (`/` is not a token character), so the
//!   server also accepts `orrery.1`.
#![allow(clippy::float_arithmetic)]
#![allow(clippy::disallowed_types)]

mod config;
mod endpoint;
mod quic;
mod stats;
mod tls;
mod ws;
mod wt;
mod wt_client;

pub mod timesync;

#[cfg(feature = "conditioner")]
mod conditioner;


pub use config::NetConfig;
pub use endpoint::{Endpoint, Transport};
pub use stats::ConnStats;
pub use tls::{QuicServerTls, QuicTrust};

#[cfg(feature = "conditioner")]
pub use conditioner::{Conditioned, LinkConditions};


#[doc(hidden)]
pub use tls::build_quic_client_config;

use std::fmt;
use std::net::SocketAddr;

/// Connection identifier. Unique per [`Endpoint`], never reused, never `0`.
pub type ConnId = u64;

/// Delivery class of a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Channel {
    /// Ordered, lossless while the connection lives.
    Reliable,
    /// Best effort. See the crate docs for the per-backend meaning.
    Unreliable,
}

/// Why a connection ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisconnectReason {
    /// This side called [`Endpoint::close`] (or the endpoint shut down).
    LocalClose,
    /// The peer closed the connection in an orderly way.
    RemoteClose,
    /// No traffic for [`NetConfig::idle_timeout`].
    TimedOut,
    /// The peer sent bytes that break the framing or size limits.
    ProtocolViolation(String),
    /// The connection could not be established (client side only).
    ConnectFailed(String),
    /// Any other transport error.
    Error(String),
}

/// One event from [`Endpoint::poll_event`]. Events of one connection arrive in
/// order: `Connected`, any number of `Message`, then `Disconnected`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Connected { conn: ConnId, peer: Option<SocketAddr> },
    Disconnected { conn: ConnId, reason: DisconnectReason },
    Message { conn: ConnId, channel: Channel, bytes: Vec<u8> },
}

/// Error from [`Endpoint::send`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendError {
    /// No such connection, or it already ended.
    UnknownConnection,
    /// The message is longer than `max` bytes (the config limit, or the QUIC
    /// datagram limit for the unreliable channel).
    TooLarge { max: usize },
    /// Too many bytes are queued for this connection. Retry later.
    Backpressure,
    /// The peer has no datagram support and fallback is off.
    DatagramsUnsupported,
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendError::UnknownConnection => write!(f, "unknown connection"),
            SendError::TooLarge { max } => write!(f, "message larger than {max} bytes"),
            SendError::Backpressure => write!(f, "send queue full"),
            SendError::DatagramsUnsupported => write!(f, "peer does not support datagrams"),
        }
    }
}
impl std::error::Error for SendError {}

/// Error while creating an endpoint.
#[derive(Debug)]
pub struct NetError(pub String);

impl NetError {
    pub(crate) fn new(e: impl fmt::Display) -> Self {
        NetError(e.to_string())
    }
}
impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for NetError {}

/// Frame tags shared by both backends.
pub(crate) const TAG_RELIABLE: u8 = 0;
pub(crate) const TAG_UNRELIABLE: u8 = 1;
pub(crate) const TAG_HELLO: u8 = 2;
pub(crate) const HELLO_PAYLOAD: &[u8] = b"ORRN\x01";
pub(crate) const PROTOCOL: &str = "orrery/1";
/// WebSocket sub-protocol offered by browsers (`/` is not a legal token character).
pub(crate) const PROTOCOL_BROWSER: &str = "orrery.1";
