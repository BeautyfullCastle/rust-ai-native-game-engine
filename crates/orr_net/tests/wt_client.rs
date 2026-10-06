//! Production native WT client interoperability and safety contracts.
#![allow(clippy::disallowed_types)]
mod common;
use common::*;
use orr_net::{Channel, DisconnectReason, Endpoint, Event, NetConfig, QuicServerTls, QuicTrust, SendError};
use std::time::{Duration, Instant};

fn pair(cfg: NetConfig) -> (Endpoint, Endpoint, u64, u64) {
    let mut server = Endpoint::listen_quic_with(loopback(), QuicServerTls::dev_localhost_webtransport(), cfg.clone(), true).unwrap();
    let mut client = Endpoint::connect_wt(&format!("https://{}/relay?token=secret", server.local_addr()), QuicTrust::CertDer(server.server_cert_der().unwrap().to_vec()), cfg).unwrap();
    let id = client.client_conn().unwrap();
    expect_connected(&mut client);
    let sid = match wait_for(&mut server, WAIT, |e| matches!(e, Event::Connected { .. })).unwrap() { Event::Connected { conn, .. } => conn, _ => unreachable!() };
    (server, client, sid, id)
}

fn message(ep: &mut Endpoint, channel: Channel, payload: &[u8]) {
    match wait_for(ep, WAIT, |e| matches!(e, Event::Message { .. })).unwrap() {
        Event::Message { channel: c, bytes, .. } => { assert_eq!(c, channel); assert_eq!(bytes, payload); }
        _ => unreachable!(),
    }
}

#[test]
fn bidirectional_stream_datagram_fallback_stats_and_close() {
    let (mut server, mut client, sid, id) = pair(NetConfig::default());
    assert!(client.stats(id).unwrap().native_datagrams);
    assert!(server.stats(sid).unwrap().native_datagrams);
    for (channel, payload) in [(Channel::Reliable, vec![1; 64]), (Channel::Unreliable, vec![2; 64]), (Channel::Unreliable, vec![3; 8192])] {
        client.send(id, channel, &payload).unwrap(); message(&mut server, channel, &payload);
        server.send(sid, channel, &payload).unwrap(); message(&mut client, channel, &payload);
    }
    let st = client.stats(id).unwrap();
    assert_eq!(st.messages_received, 3);
    assert_eq!(st.unreliable_received, 2);
    client.send(id, Channel::Reliable, b"last queued message").unwrap();
    client.close(id);
    assert_eq!(client.send(id, Channel::Reliable, b"after close"), Err(SendError::UnknownConnection));
    message(&mut server, Channel::Reliable, b"last queued message");
    assert!(matches!(wait_for(&mut client, WAIT, |e| matches!(e, Event::Disconnected { .. })), Some(Event::Disconnected { reason: DisconnectReason::LocalClose, .. })));
    assert!(client.stats(id).is_none());
    assert!(client.poll_event().is_none());
}

#[test]
fn limits_and_no_fallback() {
    let cfg = NetConfig { max_message_size: 8192, max_queued_send_bytes: 2048, datagram_fallback: false, ..NetConfig::default() };
    let (mut server, client, sid, id) = pair(cfg);
    assert!(matches!(client.send(id, Channel::Unreliable, &[0; 4096]), Err(SendError::TooLarge { .. })));
    assert_eq!(client.send(id, Channel::Reliable, &[0; 4096]), Err(SendError::Backpressure));
    assert!(matches!(client.send(id, Channel::Reliable, &[0; 8193]), Err(SendError::TooLarge { max: 8192 })));
    client.send(id, Channel::Reliable, b"alive").unwrap(); message(&mut server, Channel::Reliable, b"alive");
    assert!(server.stats(sid).is_some());
}

#[test]
fn trust_hostname_and_disabled_server_reject_without_secrets() {
    for (names, enabled, public_roots) in [(vec!["wrong.invalid".into()], true, false), (vec!["127.0.0.1".into()], true, true), (vec!["127.0.0.1".into()], false, false)] {
        let mut server = Endpoint::listen_quic_with(loopback(), QuicServerTls::SelfSigned { names }, NetConfig::default(), enabled).unwrap();
        let trust = if public_roots { QuicTrust::WebPki } else { QuicTrust::CertDer(server.server_cert_der().unwrap().to_vec()) };
        let mut client = Endpoint::connect_wt(&format!("https://{}/private-sentinel?token=private-sentinel", server.local_addr()), trust, NetConfig::default()).unwrap();
        let ev = wait_for(&mut client, WAIT, |e| matches!(e, Event::Disconnected { .. })).unwrap();
        assert!(matches!(ev, Event::Disconnected { reason: DisconnectReason::ConnectFailed(_), .. }));
        assert!(!format!("{ev:?}").contains("private-sentinel"));
        assert!(client.stats(client.client_conn().unwrap()).is_none());
        assert!(server.poll_event().is_none());
    }
}

#[test]
fn malformed_urls_and_lax_trust_reject_without_disclosure() {
    for url in ["http://secret.invalid/private-sentinel", "https://user:private-sentinel@localhost", "https://localhost:private-sentinel", "https://localhost/#private-sentinel", "https:///private-sentinel"] {
        let error = Endpoint::connect_wt(url, QuicTrust::WebPki, NetConfig::default()).err().expect("invalid URL");
        assert!(!error.to_string().contains("private-sentinel"));
    }
    for trust in [QuicTrust::DangerousSkipVerificationDevOnly, QuicTrust::Sha256Fingerprint([0; 32]), QuicTrust::PemFile("private-sentinel".into())] {
        let error = Endpoint::connect_wt("https://localhost/private-sentinel", trust, NetConfig::default()).err().expect("invalid trust");
        assert!(!error.to_string().contains("private-sentinel"));
    }
}

#[test]
fn deadline_and_drop_cleanup() {
    let socket = std::net::UdpSocket::bind(loopback()).unwrap();
    let cfg = NetConfig { connect_timeout: Duration::from_millis(100), ..NetConfig::default() };
    let start = Instant::now();
    let mut client = Endpoint::connect_wt(&format!("https://{}/private-sentinel", socket.local_addr().unwrap()), QuicTrust::WebPki, cfg).unwrap();
    assert!(matches!(client.wait_event(Duration::from_secs(2)), Some(Event::Disconnected { reason: DisconnectReason::ConnectFailed(_), .. })));
    assert!(start.elapsed() < Duration::from_secs(2));
    drop(client);
    let (mut server, client, _, id) = pair(NetConfig::default());
    client.send(id, Channel::Reliable, b"flush on drop").unwrap();
    let start = Instant::now(); drop(client);
    assert!(start.elapsed() < Duration::from_secs(2));
    message(&mut server, Channel::Reliable, b"flush on drop");
    assert!(matches!(wait_for(&mut server, WAIT, |e| matches!(e, Event::Disconnected { .. })), Some(Event::Disconnected { .. })));
}

#[test]
fn expired_and_not_yet_valid_certificates_are_rejected() {
    for future in [false, true] {
        let dir = std::env::temp_dir().join(format!("orr-wt-validity-{}-{future}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        let mut params = rcgen::CertificateParams::new(vec!["127.0.0.1".into()]).unwrap();
        let now = time::OffsetDateTime::now_utc();
        params.not_before = now + time::Duration::days(if future { 1 } else { -2 });
        params.not_after = now + time::Duration::days(if future { 2 } else { -1 });
        let cert = params.self_signed(&key).unwrap();
        let cert_path = dir.join("cert.pem"); let key_path = dir.join("key.pem");
        std::fs::write(&cert_path, cert.pem()).unwrap();
        std::fs::write(&key_path, key.serialize_pem()).unwrap();
        let server = Endpoint::listen_quic_with(loopback(), QuicServerTls::PemFiles { cert_chain: cert_path.clone(), private_key: key_path }, NetConfig::default(), true).unwrap();
        let mut client = Endpoint::connect_wt(&format!("https://{}", server.local_addr()), QuicTrust::PemFile(cert_path), NetConfig::default()).unwrap();
        assert!(matches!(client.wait_event(WAIT), Some(Event::Disconnected { reason: DisconnectReason::ConnectFailed(_), .. })));
        drop(client); drop(server); std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn unread_event_queue_drop_is_bounded_and_zero_budget_applies_to_datagrams() {
    let cfg = NetConfig { event_queue: 16, max_queued_send_bytes: 0, ..NetConfig::default() };
    let (_server, client, _, id) = pair(cfg);
    assert_eq!(client.send(id, Channel::Reliable, b""), Err(SendError::Backpressure));
    assert_eq!(client.send(id, Channel::Unreliable, b""), Err(SendError::Backpressure));
    drop(client);
    let (server, client, sid, _) = pair(NetConfig { event_queue: 16, ..NetConfig::default() });
    for _ in 0..128 { server.send(sid, Channel::Reliable, &[0; 1024]).unwrap(); }
    std::thread::sleep(Duration::from_millis(50));
    let start = Instant::now(); drop(client);
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn reflected_peer_close_reason_is_redacted() {
    let cert = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
    let der = cert.cert.der().to_vec();
    let tls = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(rustls::crypto::ring::default_provider()))
        .with_protocol_versions(&[&rustls::version::TLS13]).unwrap().with_no_client_auth()
        .with_single_cert(vec![cert.cert.der().clone()], rustls::pki_types::PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()).into()).unwrap();
    let mut tls = tls; tls.alpn_protocols = vec![b"h3".to_vec()];
    let server_cfg = quinn::ServerConfig::with_crypto(std::sync::Arc::new(quinn::crypto::rustls::QuicServerConfig::try_from(tls).unwrap()));
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let ep = quinn::Endpoint::server(server_cfg, loopback()).unwrap();
            tx.send(ep.local_addr().unwrap()).unwrap();
            let conn = ep.accept().await.unwrap().await.unwrap();
            conn.close(9u32.into(), b"private-sentinel: echoed credential");
            let _ = tokio::time::timeout(Duration::from_secs(1), ep.wait_idle()).await;
        });
    });
    let mut client = Endpoint::connect_wt(&format!("https://{}/private-sentinel?token=private-sentinel", rx.recv().unwrap()), QuicTrust::CertDer(der), NetConfig::default()).unwrap();
    let ev = client.wait_event(WAIT).unwrap();
    assert!(matches!(ev, Event::Disconnected { reason: DisconnectReason::ConnectFailed(_), .. }));
    assert!(!format!("{ev:?}").contains("private-sentinel"));
    worker.join().unwrap();
}
