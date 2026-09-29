//! QUIC-specific tests: certificate trust modes and malformed input.
#![allow(clippy::float_arithmetic)]
#![allow(clippy::disallowed_types)]

mod common;
use common::*;
use orr_net::{
    build_quic_client_config, Channel, DisconnectReason, Endpoint, Event, NetConfig, QuicServerTls, QuicTrust,
};
use std::time::Duration;

fn quic_server(cfg: NetConfig) -> Server {
    server(Backend::Quic, cfg)
}

fn connects(s: &Server, trust: QuicTrust, name: &str) -> bool {
    let mut c = Endpoint::connect_quic(s.addr, name, trust, NetConfig { connect_timeout: Duration::from_secs(2), ..NetConfig::default() })
        .unwrap();
    matches!(
        wait_for(&mut c, Duration::from_secs(5), |e| matches!(e, Event::Connected { .. } | Event::Disconnected { .. })),
        Some(Event::Connected { .. })
    )
}

#[test]
fn trust_modes() {
    let s = quic_server(NetConfig::default());
    let cert = s.cert.clone().unwrap();
    assert!(connects(&s, QuicTrust::CertDer(cert.clone()), "localhost"));
    assert!(connects(&s, QuicTrust::Sha256Fingerprint(s.ep.server_cert_sha256().unwrap()), "anything.example"));
    assert!(connects(&s, QuicTrust::DangerousSkipVerificationDevOnly, "localhost"));
    // rejected: wrong fingerprint, cert of another server, wrong name, public roots only
    assert!(!connects(&s, QuicTrust::Sha256Fingerprint([7; 32]), "localhost"));
    let other = quic_server(NetConfig::default());
    assert!(!connects(&s, QuicTrust::CertDer(other.cert.clone().unwrap()), "localhost"));
    assert!(!connects(&s, QuicTrust::CertDer(cert), "not-in-the-cert.example"));
    assert!(!connects(&s, QuicTrust::WebPki, "localhost"));
}

#[test]
fn pem_files_production_path() {
    let dir = std::env::temp_dir().join(format!("orr_net_pem_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let ck = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let (cert_path, key_path) = (dir.join("cert.pem"), dir.join("key.pem"));
    std::fs::write(&cert_path, ck.cert.pem()).unwrap();
    std::fs::write(&key_path, ck.signing_key.serialize_pem()).unwrap();

    let ep = Endpoint::listen_quic(
        loopback(),
        QuicServerTls::PemFiles { cert_chain: cert_path.clone(), private_key: key_path },
        NetConfig::default(),
    )
    .unwrap();
    assert!(ep.server_cert_der().is_none());
    let mut s = Server { addr: ep.local_addr(), ep, backend: Backend::Quic, cert: None };
    let mut c = Endpoint::connect_quic(s.addr, "localhost", QuicTrust::PemFile(cert_path), NetConfig::default()).unwrap();
    let (_, cc) = pair(&mut s.ep, &mut c);
    c.send(cc, Channel::Reliable, b"tls").unwrap();
    assert_eq!(expect_message(&mut s.ep).2, b"tls");
    let _ = std::fs::remove_dir_all(dir);

    // missing files are an error, not a panic
    let bad = Endpoint::listen_quic(
        loopback(),
        QuicServerTls::PemFiles { cert_chain: "/no/such/cert.pem".into(), private_key: "/no/such/key.pem".into() },
        NetConfig::default(),
    );
    assert!(bad.is_err());
}

/// Raw quinn client with the hello (or other bytes) under test's control.
struct Raw {
    rt: tokio::runtime::Runtime,
    _ep: quinn::Endpoint,
    conn: quinn::Connection,
    send: quinn::SendStream,
}

fn raw_connect(s: &Server) -> Raw {
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
    let cfg = build_quic_client_config(&QuicTrust::CertDer(s.cert.clone().unwrap()), &NetConfig::default()).unwrap();
    let (ep, conn, send) = rt.block_on(async {
        let mut ep = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
        ep.set_default_client_config(cfg);
        let conn = ep.connect(s.addr, "localhost").unwrap().await.unwrap();
        let (send, _recv) = conn.open_bi().await.unwrap();
        (ep, conn, send)
    });
    Raw { rt, _ep: ep, conn, send }
}

impl Raw {
    fn write(&mut self, bytes: &[u8]) {
        let (send, bytes) = (&mut self.send, bytes.to_vec());
        self.rt.block_on(async { let _ = send.write_all(&bytes).await; });
    }
    fn frame(&mut self, tag: u8, payload: &[u8]) {
        let mut v = (payload.len() as u32 + 1).to_le_bytes().to_vec();
        v.push(tag);
        v.extend_from_slice(payload);
        self.write(&v);
    }
    fn hello(&mut self) {
        self.frame(2, b"ORRN\x01");
    }
    fn closed_within(&self, d: Duration) -> bool {
        self.rt.block_on(async { tokio::time::timeout(d, self.conn.closed()).await.is_ok() })
    }
}

fn violation_cfg() -> NetConfig {
    NetConfig { max_message_size: 1000, connect_timeout: Duration::from_secs(1), ..NetConfig::default() }
}

/// Runs a bad-frame scenario and checks that the server drops only that
/// connection, reports a protocol violation and still serves a good client.
fn assert_violation(bad: impl FnOnce(&mut Raw)) {
    let mut s = quic_server(violation_cfg());
    let mut good = Endpoint::connect_quic(
        s.addr,
        "localhost",
        QuicTrust::CertDer(s.cert.clone().unwrap()),
        violation_cfg(),
    )
    .unwrap();
    let (good_s, good_c) = pair(&mut s.ep, &mut good);

    let mut raw = raw_connect(&s);
    raw.hello();
    let raw_id = expect_connected(&mut s.ep);
    bad(&mut raw);
    let (conn, reason) = expect_disconnect(&mut s.ep, WAIT);
    assert_eq!(conn, raw_id);
    assert!(matches!(reason, DisconnectReason::ProtocolViolation(_)), "{reason:?}");
    assert!(raw.closed_within(Duration::from_secs(3)), "server did not close the bad connection");

    // the other client is unaffected
    good.send(good_c, Channel::Reliable, b"still fine").unwrap();
    let (conn, _, bytes) = expect_message(&mut s.ep);
    assert_eq!((conn, bytes.as_slice()), (good_s, &b"still fine"[..]));
}

#[test]
fn oversized_frame_rejected() {
    assert_violation(|r| {
        // claims 1 MB but the limit is 1000; only a few bytes follow
        r.write(&(1_000_000u32).to_le_bytes());
        r.write(&[0, 1, 2, 3]);
    });
}

#[test]
fn huge_length_prefix_rejected_without_allocating() {
    assert_violation(|r| r.write(&[0xff, 0xff, 0xff, 0xff, 0]));
}

#[test]
fn zero_length_frame_rejected() {
    assert_violation(|r| r.write(&[0, 0, 0, 0]));
}

#[test]
fn unknown_tag_rejected() {
    assert_violation(|r| r.frame(9, b"x"));
}

#[test]
fn second_hello_rejected() {
    assert_violation(|r| r.hello());
}

#[test]
fn truncated_frame_rejected() {
    assert_violation(|r| {
        r.write(&[10, 0, 0, 0, 0, 1, 2]); // promises 9 payload bytes, sends 2
        let send = &mut r.send;
        let _ = send.finish();
    });
}

#[test]
fn bad_hello_never_becomes_a_connection() {
    let mut s = quic_server(violation_cfg());
    let mut raw = raw_connect(&s);
    raw.frame(2, b"WRONG");
    assert!(raw.closed_within(Duration::from_secs(3)));
    // and a stream that never says hello
    let mut silent = raw_connect(&s);
    silent.write(&[1]);
    assert!(silent.closed_within(Duration::from_secs(3)), "hello timeout should close");
    assert!(wait_for(&mut s.ep, Duration::from_millis(200), |_| true).is_none(), "no events expected");
    assert!(s.ep.connections().is_empty());
}

#[test]
fn garbage_datagrams_are_just_messages() {
    let mut s = quic_server(violation_cfg());
    let mut raw = raw_connect(&s);
    raw.hello();
    let id = expect_connected(&mut s.ep);
    raw.rt.block_on(async {
        raw.conn.send_datagram(bytes::Bytes::from_static(&[0xde, 0xad])).unwrap();
        raw.conn.send_datagram(bytes::Bytes::new()).unwrap();
    });
    for expected in [&[0xde, 0xad][..], &[][..]] {
        let (conn, ch, bytes) = expect_message(&mut s.ep);
        assert_eq!((conn, ch, bytes.as_slice()), (id, Channel::Unreliable, expected));
    }
}
