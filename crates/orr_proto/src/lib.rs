//! `orr_proto`: the Relay-mode wire protocol and the abstract message
//! interface it runs over.
//!
//! - [`net`]: the [`Link`] (client side) and [`Endpoint`] (server side)
//!   traits. They move opaque byte messages over a reliable-ordered or an
//!   unreliable channel. A real transport (QUIC, WebSocket, ...) implements
//!   them; nothing else in the relay code touches a socket.
//! - [`msg`]: the versioned, checksummed binary messages ([`ClientMsg`],
//!   [`ServerMsg`]). The protocol is game-agnostic: inputs and commands are
//!   opaque byte strings.
//! - [`netsim`]: a deterministic in-memory network (latency, jitter, loss,
//!   duplication, reordering) on a virtual clock, for tests.
#![deny(clippy::disallowed_types)]
#![deny(clippy::float_arithmetic)]

mod codec;
pub mod msg;
pub mod net;
pub mod netsim;

pub use msg::{
    Bundle, ClientMsg, Hello, InputEntry, ProtoError, RejectReason, ServerMsg, SlotConfirmed, TimeSync, Welcome,
    FLAG_ABSENT, FLAG_REPEATED, NO_SLOT, PROTOCOL_VERSION,
};
pub use net::{Channel, ConnId, Endpoint, Link, LinkEvent, ServerEvent, UNRELIABLE_MTU};
