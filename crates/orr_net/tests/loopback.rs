//! Loopback integration tests. Every generic test runs for QUIC and WebSocket.
#![allow(clippy::float_arithmetic)]
#![allow(clippy::disallowed_types)]

mod common;
use common::*;
use orr_net::{Channel, DisconnectReason, Endpoint, Event, NetConfig, SendError};
use std::sync::atomic::Ordering::Relaxed;
use std::thread;
use std::time::Duration;

macro_rules! both {
    ($name:ident) => {
        mod $name {
            #[test]
            fn quic() {
                super::$name(super::Backend::Quic)
            }
            #[test]
            fn ws() {
                super::$name(super::Backend::Ws)
            }
        }
    };
}

fn connect_and_echo(b: Backend) {
    let mut s = server(b, NetConfig::default());
    let mut c = client(&s, NetConfig::default());
    let (sc, cc) = pair(&mut s.ep, &mut c);
    assert_eq!(c.client_conn(), Some(cc));
    assert_eq!(s.ep.connections(), vec![sc]);

    c.send(cc, Channel::Reliable, b"hello").unwrap();
    let (conn, ch, bytes) = expect_message(&mut s.ep);
    assert_eq!((conn, ch, bytes.as_slice()), (sc, Channel::Reliable, &b"hello"[..]));
    s.ep.send(sc, Channel::Reliable, b"world").unwrap();
    let (conn, _, bytes) = expect_message(&mut c);
    assert_eq!((conn, bytes.as_slice()), (cc, &b"world"[..]));

    // empty payloads are valid messages
    c.send(cc, Channel::Reliable, b"").unwrap();
    assert_eq!(expect_message(&mut s.ep).2, Vec::<u8>::new());

    // stats
    thread::sleep(Duration::from_millis(250));
    let st = c.stats(cc).unwrap();
    assert_eq!(st.messages_sent, 2);
    assert_eq!(st.payload_bytes_sent, 5);
    assert_eq!(st.messages_received, 1);
    assert!(st.bytes_sent > 0 && st.packets_sent > 0);
    assert_eq!(st.queued_bytes, 0);
    assert!(s.ep.stats(999).is_none());
}
both!(connect_and_echo);

fn reliable_ordered_many_and_large(b: Backend) {
    let mut s = server(b, NetConfig::default());
    let mut c = client(&s, NetConfig::default());
    let (sc, cc) = pair(&mut s.ep, &mut c);

    let mut sizes: Vec<usize> = (0..3000).map(|i| 4 + (i * 37) % 900).collect();
    sizes.insert(1000, 1024 * 1024);
    sizes.insert(2000, 3 * 1024 * 1024 + 123);
    let sizes_for_sender = sizes.clone();
    let sender = thread::spawn(move || {
        for (i, len) in sizes_for_sender.iter().enumerate() {
            send_retry(&c, cc, Channel::Reliable, &pattern(i as u32, *len));
        }
        c // keep the endpoint alive until joined
    });
    for (i, len) in sizes.iter().enumerate() {
        let (conn, ch, bytes) = expect_message(&mut s.ep);
        assert_eq!((conn, ch), (sc, Channel::Reliable));
        assert_eq!(bytes.len(), *len, "message {i}");
        assert!(bytes == pattern(i as u32, *len), "message {i} content");
    }
    drop(sender.join().unwrap());
}
both!(reliable_ordered_many_and_large);

fn unreliable_channel(b: Backend) {
    let mut s = server(b, NetConfig::default());
    let mut c = client(&s, NetConfig::default());
    let (sc, cc) = pair(&mut s.ep, &mut c);
    thread::sleep(Duration::from_millis(250)); // let stats settle

    let st = c.stats(cc).unwrap();
    assert_eq!(st.native_datagrams, b == Backend::Quic);
    if b == Backend::Quic {
        assert!(st.max_unreliable_size > 1000 && st.max_unreliable_size < 1500, "{}", st.max_unreliable_size);
        let too_big = vec![0u8; st.max_unreliable_size + 1];
        assert_eq!(
            c.send(cc, Channel::Unreliable, &too_big),
            Err(SendError::TooLarge { max: st.max_unreliable_size })
        );
    }

    let n = 100u32;
    for i in 0..n {
        c.send(cc, Channel::Unreliable, &pattern(i, 200)).unwrap();
        thread::sleep(Duration::from_micros(300));
    }
    let mut got = 0;
    while let Some(Event::Message { conn, channel, bytes }) =
        wait_for(&mut s.ep, Duration::from_millis(500), |e| matches!(e, Event::Message { .. }))
    {
        assert_eq!((conn, channel), (sc, Channel::Unreliable));
        assert_eq!(bytes.len(), 200);
        got += 1;
    }
    // loopback UDP is not lossless under load, so allow some loss on QUIC
    let min = if b == Backend::Quic { 80 } else { n };
    assert!(got >= min, "received {got}/{n}");
    assert!(s.ep.stats(sc).unwrap().unreliable_received >= got as u64);
}
both!(unreliable_channel);

fn graceful_close_flushes_and_reports(b: Backend) {
    let mut s = server(b, NetConfig::default());
    let mut c = client(&s, NetConfig::default());
    let (sc, cc) = pair(&mut s.ep, &mut c);
    for i in 0..50u32 {
        c.send(cc, Channel::Reliable, &pattern(i, 5000)).unwrap();
    }
    c.close(cc);
    // all queued messages arrive before the disconnect
    for i in 0..50u32 {
        let (_, _, bytes) = expect_message(&mut s.ep);
        assert_eq!(bytes, pattern(i, 5000));
    }
    let (conn, reason) = expect_disconnect(&mut s.ep, WAIT);
    assert_eq!((conn, reason), (sc, DisconnectReason::RemoteClose));
    let (conn, reason) = expect_disconnect(&mut c, WAIT);
    assert_eq!((conn, reason), (cc, DisconnectReason::LocalClose));
    assert_eq!(c.send(cc, Channel::Reliable, b"x"), Err(SendError::UnknownConnection));
    assert!(s.ep.connections().is_empty());
}
both!(graceful_close_flushes_and_reports);

fn server_close_and_endpoint_drop(b: Backend) {
    let mut s = server(b, NetConfig::default());
    let mut c1 = client(&s, NetConfig::default());
    let (sc1, cc1) = pair(&mut s.ep, &mut c1);
    s.ep.close(sc1);
    assert_eq!(expect_disconnect(&mut c1, WAIT), (cc1, DisconnectReason::RemoteClose));
    assert_eq!(expect_disconnect(&mut s.ep, WAIT), (sc1, DisconnectReason::LocalClose));

    // dropping a client endpoint is seen as an orderly close
    let mut c2 = client(&s, NetConfig::default());
    let (sc2, _) = pair(&mut s.ep, &mut c2);
    drop(c2);
    assert_eq!(expect_disconnect(&mut s.ep, WAIT), (sc2, DisconnectReason::RemoteClose));
}
both!(server_close_and_endpoint_drop);

fn eight_clients(b: Backend) {
    let mut s = server(b, NetConfig::default());
    let mut clients: Vec<Endpoint> = (0..8).map(|_| client(&s, NetConfig::default())).collect();
    let mut server_ids = std::collections::BTreeSet::new();
    for _ in 0..8 {
        server_ids.insert(expect_connected(&mut s.ep));
    }
    assert_eq!(server_ids.len(), 8);
    let cids: Vec<_> = clients.iter_mut().map(expect_connected).collect();
    for (i, c) in clients.iter().enumerate() {
        for k in 0..20u32 {
            c.send(cids[i], Channel::Reliable, &pattern(i as u32 * 1000 + k, 300)).unwrap();
        }
    }
    let mut per_conn: std::collections::BTreeMap<u64, Vec<u32>> = Default::default();
    for _ in 0..160 {
        let (conn, _, bytes) = expect_message(&mut s.ep);
        per_conn.entry(conn).or_default().push(u32::from_le_bytes(bytes[..4].try_into().unwrap()));
        // echo the first 4 bytes back on the same connection
        s.ep.send(conn, Channel::Reliable, &bytes[..4]).unwrap();
    }
    assert_eq!(per_conn.len(), 8);
    for seqs in per_conn.values() {
        let base = seqs[0] / 1000 * 1000;
        assert_eq!(*seqs, (0..20).map(|k| base + k).collect::<Vec<_>>());
    }
    for (i, c) in clients.iter_mut().enumerate() {
        for k in 0..20u32 {
            let (conn, _, bytes) = expect_message(c);
            assert_eq!(conn, cids[i]);
            assert_eq!(u32::from_le_bytes(bytes[..4].try_into().unwrap()), i as u32 * 1000 + k);
        }
    }
}
both!(eight_clients);

fn oversized_send_rejected_locally(b: Backend) {
    let cfg = NetConfig { max_message_size: 1000, ..NetConfig::default() };
    let mut s = server(b, cfg.clone());
    let mut c = client(&s, cfg);
    let (_, cc) = pair(&mut s.ep, &mut c);
    assert_eq!(c.send(cc, Channel::Reliable, &[0; 1001]), Err(SendError::TooLarge { max: 1000 }));
    assert_eq!(c.send(cc, Channel::Reliable, &[0; 1000]), Ok(()));
    assert_eq!(expect_message(&mut s.ep).2.len(), 1000);
}
both!(oversized_send_rejected_locally);

fn idle_timeout_detected(b: Backend) {
    let mut s = server(b, quick_cfg());
    // Route the client through a proxy that can go silent.
    let (client_ep, _proxy_keepalive): (Endpoint, Box<dyn std::any::Any>) = match b {
        Backend::Quic => {
            let p = udp_proxy(s.addr);
            (connect_to(b, p.addr, s.cert.as_deref(), quick_cfg()), Box::new(p) as Box<dyn std::any::Any>)
        }
        Backend::Ws => {
            let p = tcp_proxy(s.addr);
            (connect_to(b, p.addr, None, quick_cfg()), Box::new(p) as Box<dyn std::any::Any>)
        }
    };
    let mut c = client_ep;
    let (sc, cc) = pair(&mut s.ep, &mut c);
    // traffic flows
    c.send(cc, Channel::Reliable, b"a").unwrap();
    expect_message(&mut s.ep);
    // cut the link
    match b {
        Backend::Quic => _proxy_keepalive.downcast_ref::<UdpProxy>().unwrap().blackhole.store(true, Relaxed),
        Backend::Ws => _proxy_keepalive.downcast_ref::<TcpProxy>().unwrap().blackhole.store(true, Relaxed),
    }
    let (conn, reason) = expect_disconnect(&mut c, Duration::from_secs(5));
    assert_eq!((conn, reason), (cc, DisconnectReason::TimedOut));
    let (conn, reason) = expect_disconnect(&mut s.ep, Duration::from_secs(5));
    assert_eq!((conn, reason), (sc, DisconnectReason::TimedOut));
}
both!(idle_timeout_detected);

fn connect_to_nothing_fails(b: Backend) {
    // a port that was just closed
    let addr = {
        let s = server(b, NetConfig::default());
        s.addr
    };
    let cfg = NetConfig { connect_timeout: Duration::from_millis(700), ..NetConfig::default() };
    let cert = matches!(b, Backend::Quic).then(|| vec![0u8; 0]);
    let mut c = match b {
        Backend::Quic => {
            // any trust mode will do, the handshake never gets an answer
            Endpoint::connect_quic(addr, "localhost", orr_net::QuicTrust::DangerousSkipVerificationDevOnly, cfg).unwrap()
        }
        Backend::Ws => connect_to(b, addr, cert.as_deref(), cfg),
    };
    let (conn, reason) = expect_disconnect(&mut c, Duration::from_secs(5));
    assert_eq!(Some(conn), c.client_conn());
    assert!(matches!(reason, DisconnectReason::ConnectFailed(_)), "{reason:?}");
}
both!(connect_to_nothing_fails);

fn max_connections_enforced(b: Backend) {
    let cfg = NetConfig { max_connections: 2, connect_timeout: Duration::from_secs(1), ..NetConfig::default() };
    let s = server(b, cfg.clone());
    let mut c1 = client(&s, cfg.clone());
    let mut c2 = client(&s, cfg.clone());
    expect_connected(&mut c1);
    expect_connected(&mut c2);
    let mut c3 = client(&s, cfg);
    let (_, reason) = expect_disconnect(&mut c3, Duration::from_secs(5));
    assert!(matches!(reason, DisconnectReason::ConnectFailed(_)), "{reason:?}");
    assert_eq!(s.ep.connections().len(), 2);
}
both!(max_connections_enforced);

fn send_to_unknown_connection(b: Backend) {
    let s = server(b, NetConfig::default());
    assert_eq!(s.ep.send(42, Channel::Reliable, b"x"), Err(SendError::UnknownConnection));
    s.ep.close(42); // no panic
}
both!(send_to_unknown_connection);

fn event_queue_backpressure_does_not_lose_messages(b: Backend) {
    // The caller stops polling for a while. A tiny event queue forces the
    // reader to wait, and nothing may be lost or reordered.
    let cfg = NetConfig { event_queue: 16, ..NetConfig::default() };
    let mut s = server(b, cfg.clone());
    let mut c = client(&s, cfg);
    let (_, cc) = pair(&mut s.ep, &mut c);
    for i in 0..500u32 {
        c.send(cc, Channel::Reliable, &pattern(i, 100)).unwrap();
    }
    thread::sleep(Duration::from_millis(300));
    for i in 0..500u32 {
        let (_, _, bytes) = expect_message(&mut s.ep);
        assert_eq!(bytes, pattern(i, 100));
    }
}
both!(event_queue_backpressure_does_not_lose_messages);

#[test]
fn poll_never_blocks() {
    let mut s = server(Backend::Quic, NetConfig::default());
    let t = std::time::Instant::now();
    for _ in 0..10_000 {
        assert!(s.ep.poll_event().is_none());
    }
    assert!(t.elapsed() < Duration::from_millis(500), "{:?}", t.elapsed());
}
