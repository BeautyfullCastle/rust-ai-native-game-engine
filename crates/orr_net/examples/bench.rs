//! Localhost round-trip latency and throughput of the QUIC and WebSocket backends.
//!
//! `cargo run -p orr_net --release --example bench`
//!
//! Server thread: echoes messages that start with byte 0 (ping), counts bytes
//! of messages that start with byte 1 (data) and answers byte 2 (end) with an ack.
#![allow(clippy::float_arithmetic)]
#![allow(clippy::disallowed_types)]

use orr_net::{Channel, Endpoint, Event, NetConfig, QuicServerTls, QuicTrust, SendError};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

fn wait_message(ep: &mut Endpoint) -> Vec<u8> {
    loop {
        match ep.wait_event(Duration::from_secs(10)) {
            Some(Event::Message { bytes, .. }) => return bytes,
            Some(_) => {}
            None => panic!("timeout"),
        }
    }
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    sorted[((sorted.len() - 1) as f64 * p) as usize]
}

fn run(name: &str, quic: bool) {
    let cfg = NetConfig::default();
    let bind = ([127, 0, 0, 1], 0).into();
    let mut server = if quic {
        Endpoint::listen_quic(bind, QuicServerTls::dev_localhost(), cfg.clone()).unwrap()
    } else {
        Endpoint::listen_ws(bind, cfg.clone()).unwrap()
    };
    let addr = server.local_addr();
    let trust = QuicTrust::Sha256Fingerprint(server.server_cert_sha256().unwrap_or([0; 32]));
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let srv = thread::spawn(move || {
        let mut received = 0u64;
        while !stop2.load(Relaxed) {
            if let Some(Event::Message { conn, channel, bytes }) = server.wait_event(Duration::from_millis(50)) {
                match bytes.first() {
                    Some(0) => {
                        let _ = server.send(conn, channel, &bytes);
                    }
                    Some(1) => received += bytes.len() as u64,
                    Some(2) => {
                        let _ = server.send(conn, Channel::Reliable, &received.to_le_bytes());
                        received = 0;
                    }
                    _ => {}
                }
            }
        }
    });

    let mut client = if quic {
        Endpoint::connect_quic(addr, "localhost", trust, cfg).unwrap()
    } else {
        Endpoint::connect_ws(&format!("ws://{addr}/"), NetConfig::default()).unwrap()
    };
    let conn = match client.wait_event(Duration::from_secs(5)) {
        Some(Event::Connected { conn, .. }) => conn,
        other => panic!("{other:?}"),
    };

    // Latency: 5000 sequential ping-pongs of 32 bytes.
    let channels: &[(Channel, &str)] = if quic {
        &[(Channel::Reliable, "reliable"), (Channel::Unreliable, "unreliable(datagram)")]
    } else {
        &[(Channel::Reliable, "reliable"), (Channel::Unreliable, "unreliable(over stream)")]
    };
    for &(ch, label) in channels {
        let mut ping = [0u8; 32];
        let mut samples = Vec::new();
        for _ in 0..5000 {
            let t = Instant::now();
            client.send(conn, ch, &ping).unwrap();
            let _ = wait_message(&mut client);
            samples.push(t.elapsed());
            ping[1] = ping[1].wrapping_add(1);
        }
        samples.sort();
        println!(
            "{name:9} rtt {label:24} min {:>6.0}us  p50 {:>6.0}us  p99 {:>6.0}us  max {:>6.0}us",
            samples[0].as_secs_f64() * 1e6,
            percentile(&samples, 0.5).as_secs_f64() * 1e6,
            percentile(&samples, 0.99).as_secs_f64() * 1e6,
            samples[samples.len() - 1].as_secs_f64() * 1e6,
        );
    }

    // Throughput: one-way reliable, sizes 1 KB and 64 KB, 256 MiB total each.
    for size in [1024usize, 65536] {
        let total: usize = 256 * 1024 * 1024;
        let msgs = total / size;
        let mut payload = vec![7u8; size];
        payload[0] = 1;
        let t = Instant::now();
        for _ in 0..msgs {
            loop {
                match client.send(conn, Channel::Reliable, &payload) {
                    Ok(()) => break,
                    Err(SendError::Backpressure) => thread::sleep(Duration::from_micros(100)),
                    Err(e) => panic!("{e}"),
                }
            }
        }
        client.send(conn, Channel::Reliable, &[2]).unwrap();
        let ack = wait_message(&mut client);
        let secs = t.elapsed().as_secs_f64();
        let got = u64::from_le_bytes(ack[..8].try_into().unwrap());
        assert_eq!(got as usize, msgs * size);
        println!(
            "{name:9} throughput {:>5} B msgs: {:>7.1} MB/s  {:>9.0} msgs/s",
            size,
            total as f64 / secs / 1e6,
            msgs as f64 / secs
        );
    }

    drop(client);
    stop.store(true, Relaxed);
    srv.join().unwrap();
}

fn main() {
    println!("orr_net localhost benchmark (release build recommended)");
    run("QUIC", true);
    run("WebSocket", false);
}
