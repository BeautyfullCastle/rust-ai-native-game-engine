//! `orr_relay_net`: the Relay protocol over real sockets.
//!
//! - [`NetLink`]: an `orr_proto::Link` over an `orr_net` client endpoint
//!   (QUIC or WebSocket, optionally with the network conditioner).
//! - [`NetEndpoint`]: an `orr_proto::Endpoint` over an `orr_net` listening
//!   endpoint, mapping `orr_net`'s `u64` connection ids to `orr_proto`'s
//!   `u32` ones.
//! - [`connect`] / [`listen`]: build them from plain options (address,
//!   transport, certificate trust or PEM files, simulated latency and loss).
//! - [`p2p_input`]: bounded two-peer reliable input codec/source/driver, separate
//!   from relay messages and checked join controls.
//! - [`driver`]: a real-time loop for a headless `RelayClient` (bots, tests)
//!   and the file sink for desync dumps.
//!
//! The sim crates (`orr_fp`, `orr_ecs`, `orr_sim`, `orr_session`,
//! `orr_proto`) do not depend on this crate, nor on `orr_net` or tokio. This
//! crate reads the wall clock and uses threads through `orr_net`.
#![allow(clippy::disallowed_types)]
// Not a sim crate: the reports and logs use floats.
#![allow(clippy::float_arithmetic)]

mod connect;
pub mod driver;
mod link;
pub mod p2p_input;
pub mod p2p_mesh_input;
mod server_ep;

pub use connect::{
    connect, format_fingerprint, fresh_seed, listen, parse_fingerprint, ConnectOptions, ListenOptions, SimConditions, Tls, TransportKind,
    Trust,
};
pub use driver::{drive, report, ClientReport, DirSink, DriveOptions};
pub use link::NetLink;
pub use server_ep::NetEndpoint;

pub use orr_net::{DisconnectReason, NetConfig};

pub use p2p_input::{
    encode_p2p_input, P2pInputAccepted, P2pInputCodec, P2pInputDriver, P2pInputError,
    P2pInputFlush, P2pInputLimits, P2pInputSource, P2P_INPUT_VERSION,
};
