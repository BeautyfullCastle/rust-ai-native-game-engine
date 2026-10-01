//! Echo server for browser transport trials: QUIC + WebTransport on one UDP
//! port, plus a plain WebSocket listener. Every message comes back on the
//! channel it arrived on.
//!
//! `cargo run -p orr_net --release --example wt_echo -- [--udp PORT] [--ws PORT] [--bind IP]
//!     [--cert cert.pem --key key.pem] [--no-webtransport] [--long-lived-cert]`
//!
//! Prints one line `READY {json}` with the ports and the certificate hash
//! (hex SHA-256) for `serverCertificateHashes`. Stops on stdin EOF.
#![allow(clippy::disallowed_types)]

use orr_net::{Endpoint, Event, NetConfig, QuicServerTls};
use std::io::Read;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Arc;
use std::time::Duration;

fn main() {
    let (mut udp, mut ws, mut ip) = (0u16, 0u16, "127.0.0.1".to_string());
    let (mut cert, mut key, mut wt, mut long) = (None, None, true, false);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--udp" => udp = args.next().unwrap().parse().unwrap(),
            "--ws" => ws = args.next().unwrap().parse().unwrap(),
            "--bind" => ip = args.next().unwrap(),
            "--cert" => cert = args.next(),
            "--key" => key = args.next(),
            "--no-webtransport" => wt = false,
            // Negative test: a certificate that is valid far longer than 14 days.
            "--long-lived-cert" => long = true,
            other => panic!("unknown option {other}"),
        }
    }
    let tls = match (cert, key) {
        (Some(c), Some(k)) => QuicServerTls::PemFiles { cert_chain: c.into(), private_key: k.into() },
        _ if long => QuicServerTls::dev_localhost(),
        _ => QuicServerTls::dev_localhost_webtransport(),
    };
    let bind: SocketAddr = format!("{ip}:{udp}").parse().unwrap();
    let mut ep = Endpoint::listen_quic_with(bind, tls, NetConfig::default(), wt).unwrap();
    let ws_addr = ep.add_ws_listener(format!("{ip}:{ws}").parse().unwrap()).unwrap();
    let hash = ep.server_cert_sha256().map(|h| h.iter().map(|b| format!("{b:02x}")).collect::<String>());
    println!(
        "READY {{\"udp_port\":{},\"ws_port\":{},\"hash\":{}}}",
        ep.local_addr().port(),
        ws_addr.port(),
        hash.map_or("null".to_string(), |h| format!("\"{h}\""))
    );
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    std::thread::spawn(move || {
        let mut b = [0u8; 64];
        while std::io::stdin().read(&mut b).is_ok_and(|n| n > 0) {}
        s2.store(true, Relaxed);
    });
    while !stop.load(Relaxed) {
        if let Some(Event::Message { conn, channel, bytes }) = ep.wait_event(Duration::from_millis(20)) {
            let _ = ep.send(conn, channel, &bytes);
        }
    }
}
