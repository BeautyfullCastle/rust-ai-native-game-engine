//! Relay server binary (thin shell). The core is in the library; what is
//! missing here is a real `orr_proto::Endpoint` (QUIC / WebSocket over
//! `orr_net`) and room creation from a config file. Until then this runs
//! the core on a `NullEndpoint` with the wall clock, to show how it is
//! driven.
// The core takes its time from the caller; only this shell reads a clock.
#![allow(clippy::disallowed_types)]

use std::time::{Duration, Instant};

use orr_proto::{ConnId, Endpoint, Channel, ServerEvent};
use orr_server::{RelayServer, RoomConfig};

/// An endpoint with no connections.
struct NullEndpoint;

impl Endpoint for NullEndpoint {
    fn send(&mut self, _conn: ConnId, _channel: Channel, _data: &[u8]) {}
    fn poll(&mut self) -> Option<ServerEvent> {
        None
    }
    fn disconnect(&mut self, _conn: ConnId) {}
}

fn main() {
    let mut server = RelayServer::new(NullEndpoint, 0x00DD_BA11);
    server.create_room(1, RoomConfig::new(4, 60, 1, 16));
    eprintln!("orr_server: no transport wired yet (needs an orr_proto::Endpoint over orr_net); idling for 1s");
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(1) {
        server.update(start.elapsed().as_micros() as u64);
        std::thread::sleep(Duration::from_millis(4));
    }
}
