//! Native verified WSS client loopback tests.
#![allow(clippy::disallowed_types)]
#![allow(clippy::float_arithmetic)]

mod common;

use common::{expect_disconnect, expect_message, loopback, wait_for, WAIT};
use orr_net::{
    Channel, ConnId, ConnStats, DisconnectReason, Endpoint, Event, NetConfig, QuicServerTls,
    QuicTrust,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct WssServer {
    ep: Endpoint,
    addr: SocketAddr,
    cert: Vec<u8>,
}

fn server_with_names(names: &[&str]) -> WssServer {
    let tls = QuicServerTls::SelfSigned {
        names: names.iter().map(|name| (*name).to_owned()).collect(),
    };
    let mut ep = Endpoint::listen_quic(loopback(), tls, NetConfig::default()).unwrap();
    let cert = ep.server_cert_der().unwrap().to_vec();
    let addr = ep.add_wss_listener(loopback()).unwrap();
    WssServer { ep, addr, cert }
}

fn url(host: &str, addr: SocketAddr, path: &str) -> String {
    format!("wss://{host}:{}{path}", addr.port())
}

fn terminal(ep: &mut Endpoint) -> Event {
    wait_for(ep, Duration::from_secs(3), |event| {
        matches!(event, Event::Connected { .. } | Event::Disconnected { .. })
    })
    .expect("client should finish connecting")
}

fn connected(ep: &mut Endpoint) -> u64 {
    match terminal(ep) {
        Event::Connected { conn, .. } => conn,
        other => panic!("expected WSS Connected, got {other:?}"),
    }
}

fn connect_failed(ep: &mut Endpoint) -> String {
    match terminal(ep) {
        Event::Disconnected {
            reason: DisconnectReason::ConnectFailed(message),
            ..
        } => message,
        other => panic!("expected WSS ConnectFailed, got {other:?}"),
    }
}

fn connect(server: &WssServer, host: &str, trust: QuicTrust) -> Endpoint {
    Endpoint::connect_wss(
        &url(host, server.addr, "/native"),
        None,
        trust,
        NetConfig::default(),
    )
    .unwrap()
}

fn wait_for_sent_stats(
    ep: &Endpoint,
    conn: ConnId,
    messages: u64,
    unreliable: u64,
    payload_bytes: u64,
) -> ConnStats {
    let deadline = Instant::now() + WAIT;
    loop {
        let stats = ep.stats(conn).expect("connection should remain live");
        // Receipt can precede sink.send completing on the sender, and its counters
        // are published separately. Wait for all of them, not just messages_sent.
        // Lower bounds here let the exact assertions below diagnose over-counting.
        if stats.messages_sent >= messages
            && stats.unreliable_sent >= unreliable
            && stats.packets_sent >= messages
            && stats.payload_bytes_sent >= payload_bytes
            && stats.bytes_sent >= payload_bytes + messages
        {
            return stats;
        }
        assert!(
            Instant::now() < deadline,
            "sender counters did not finish publishing: {stats:?}"
        );
        thread::yield_now();
    }
}

#[test]
fn wss_loopback_bidirectional_channels_stats_and_graceful_close() {
    let mut server = server_with_names(&["localhost"]);
    // URL host supplies the default certificate name; the query is preserved by the request target.
    let target = url("localhost", server.addr, "/relay?mode=wss-loopback");
    let mut client = Endpoint::connect_wss(
        &target,
        None,
        QuicTrust::CertDer(server.cert.clone()),
        NetConfig::default(),
    )
    .unwrap();
    let client_id = connected(&mut client);
    let server_id = connected(&mut server.ep);

    client
        .send(client_id, Channel::Reliable, b"client reliable")
        .unwrap();
    client
        .send(client_id, Channel::Unreliable, b"client best effort")
        .unwrap();
    let (conn, channel, bytes) = expect_message(&mut server.ep);
    assert_eq!(
        (conn, channel, bytes.as_slice()),
        (server_id, Channel::Reliable, &b"client reliable"[..])
    );
    let (conn, channel, bytes) = expect_message(&mut server.ep);
    assert_eq!(
        (conn, channel, bytes.as_slice()),
        (server_id, Channel::Unreliable, &b"client best effort"[..])
    );

    server
        .ep
        .send(server_id, Channel::Reliable, b"server reliable")
        .unwrap();
    server
        .ep
        .send(server_id, Channel::Unreliable, b"server best effort")
        .unwrap();
    let (conn, channel, bytes) = expect_message(&mut client);
    assert_eq!(
        (conn, channel, bytes.as_slice()),
        (client_id, Channel::Reliable, &b"server reliable"[..])
    );
    let (conn, channel, bytes) = expect_message(&mut client);
    assert_eq!(
        (conn, channel, bytes.as_slice()),
        (client_id, Channel::Unreliable, &b"server best effort"[..])
    );

    let client_stats = wait_for_sent_stats(
        &client,
        client_id,
        2,
        1,
        b"client reliable".len() as u64 + b"client best effort".len() as u64,
    );
    assert_eq!(client_stats.messages_sent, 2);
    assert_eq!(client_stats.messages_received, 2);
    assert_eq!(client_stats.unreliable_sent, 1);
    assert_eq!(client_stats.unreliable_received, 1);
    assert_eq!(client_stats.packets_sent, 2);
    assert_eq!(client_stats.packets_received, 2);
    assert_eq!(
        client_stats.payload_bytes_sent,
        b"client reliable".len() as u64 + b"client best effort".len() as u64
    );
    assert_eq!(
        client_stats.payload_bytes_received,
        b"server reliable".len() as u64 + b"server best effort".len() as u64
    );
    assert_eq!(client_stats.bytes_sent, client_stats.payload_bytes_sent + 2);
    assert_eq!(
        client_stats.bytes_received,
        client_stats.payload_bytes_received + 2
    );
    assert!(!client_stats.native_datagrams);

    let server_stats = wait_for_sent_stats(
        &server.ep,
        server_id,
        2,
        1,
        b"server reliable".len() as u64 + b"server best effort".len() as u64,
    );
    assert_eq!(server_stats.messages_sent, 2);
    assert_eq!(server_stats.messages_received, 2);
    assert_eq!(server_stats.unreliable_sent, 1);
    assert_eq!(server_stats.unreliable_received, 1);
    assert_eq!(server_stats.packets_sent, 2);
    assert_eq!(server_stats.packets_received, 2);
    assert_eq!(
        server_stats.payload_bytes_sent,
        client_stats.payload_bytes_received
    );
    assert_eq!(
        server_stats.payload_bytes_received,
        client_stats.payload_bytes_sent
    );
    assert_eq!(server_stats.bytes_sent, server_stats.payload_bytes_sent + 2);
    assert_eq!(
        server_stats.bytes_received,
        server_stats.payload_bytes_received + 2
    );
    assert!(!server_stats.native_datagrams);

    client.close(client_id);
    assert_eq!(
        expect_disconnect(&mut client, WAIT),
        (client_id, DisconnectReason::LocalClose)
    );
    assert_eq!(
        expect_disconnect(&mut server.ep, WAIT),
        (server_id, DisconnectReason::RemoteClose)
    );
}

#[test]
fn wss_trust_modes_and_url_hostname_validation() {
    let mut server = server_with_names(&["localhost"]);

    // A self-signed leaf is not trusted by the public Web PKI roots.
    let mut untrusted = connect(&server, "localhost", QuicTrust::WebPki);
    assert!(matches!(
        terminal(&mut untrusted),
        Event::Disconnected {
            reason: DisconnectReason::ConnectFailed(_),
            ..
        }
    ));

    // The default verification name is the URL host. This IP does not appear in this certificate.
    let mut wrong_default_name = connect(
        &server,
        "127.0.0.1",
        QuicTrust::CertDer(server.cert.clone()),
    );
    assert!(matches!(
        terminal(&mut wrong_default_name),
        Event::Disconnected {
            reason: DisconnectReason::ConnectFailed(_),
            ..
        }
    ));

    // An explicit TLS name can be used when the URL authority is an IP or proxy name.
    let mut override_name = Endpoint::connect_wss(
        &url("127.0.0.1", server.addr, "/"),
        Some("localhost"),
        QuicTrust::CertDer(server.cert.clone()),
        NetConfig::default(),
    )
    .unwrap();
    let client_id = connected(&mut override_name);
    let server_id = connected(&mut server.ep);
    override_name.close(client_id);
    let _ = expect_disconnect(&mut override_name, WAIT);
    let _ = expect_disconnect(&mut server.ep, WAIT);
    let _ = server_id;

    // Modes that would bypass ordinary certificate verification are rejected before connecting.
    assert!(Endpoint::connect_wss(
        &url("localhost", server.addr, "/"),
        None,
        QuicTrust::Sha256Fingerprint([0; 32]),
        NetConfig::default(),
    )
    .is_err());
    assert!(Endpoint::connect_wss(
        &url("localhost", server.addr, "/"),
        None,
        QuicTrust::DangerousSkipVerificationDevOnly,
        NetConfig::default(),
    )
    .is_err());
}

fn temp_dir(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "orr_net_wss_{label}_{}_{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ))
}

#[test]
fn wss_pem_file_trust_and_expired_certificate_rejection() {
    // Exercise the same PEM identity source used by the WSS server and the client's PEM trust path.
    let dir = temp_dir("pem");
    std::fs::create_dir_all(&dir).unwrap();
    let ck = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");
    std::fs::write(&cert_path, ck.cert.pem()).unwrap();
    std::fs::write(&key_path, ck.signing_key.serialize_pem()).unwrap();
    let mut server = Endpoint::listen_quic(
        loopback(),
        QuicServerTls::PemFiles {
            cert_chain: cert_path.clone(),
            private_key: key_path,
        },
        NetConfig::default(),
    )
    .unwrap();
    let addr = server.add_wss_listener(loopback()).unwrap();
    let mut client = Endpoint::connect_wss(
        &url("localhost", addr, "/pem"),
        None,
        QuicTrust::PemFile(cert_path),
        NetConfig::default(),
    )
    .unwrap();
    let client_id = connected(&mut client);
    let server_id = connected(&mut server);
    client
        .send(client_id, Channel::Reliable, b"trusted PEM")
        .unwrap();
    assert_eq!(expect_message(&mut server).2, b"trusted PEM");
    client.close(client_id);
    let _ = expect_disconnect(&mut client, WAIT);
    let _ = expect_disconnect(&mut server, WAIT);

    let wrong_ca_dir = temp_dir("wrong_ca");
    std::fs::create_dir_all(&wrong_ca_dir).unwrap();
    let wrong_ca = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let wrong_ca_path = wrong_ca_dir.join("wrong-ca.pem");
    std::fs::write(&wrong_ca_path, wrong_ca.cert.pem()).unwrap();
    let mut wrong_ca_client = Endpoint::connect_wss(
        &url("localhost", addr, "/wrong-ca"),
        None,
        QuicTrust::PemFile(wrong_ca_path),
        NetConfig::default(),
    )
    .unwrap();
    assert!(matches!(
        terminal(&mut wrong_ca_client),
        Event::Disconnected {
            reason: DisconnectReason::ConnectFailed(_),
            ..
        }
    ));
    drop(wrong_ca_client);
    std::fs::remove_dir_all(wrong_ca_dir).unwrap();

    drop(client);
    drop(server);
    std::fs::remove_dir_all(&dir).unwrap();
    let _ = server_id;

    let empty_ca = temp_dir("empty_ca.pem");
    std::fs::write(&empty_ca, "\n").unwrap();
    assert!(Endpoint::connect_wss(
        &url("localhost", addr, "/empty-ca"),
        None,
        QuicTrust::PemFile(empty_ca.clone()),
        NetConfig::default(),
    )
    .is_err());
    std::fs::remove_file(empty_ca).unwrap();

    // Expiry is still enforced when the certificate itself is supplied as a trust anchor.
    let expired_dir = temp_dir("expired");
    std::fs::create_dir_all(&expired_dir).unwrap();
    let key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::new(vec!["localhost".to_owned()]).unwrap();
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now - time::Duration::days(4);
    params.not_after = now - time::Duration::days(2);
    let expired = params.self_signed(&key).unwrap();
    let cert_path = expired_dir.join("expired.pem");
    let key_path = expired_dir.join("key.pem");
    std::fs::write(&cert_path, expired.pem()).unwrap();
    std::fs::write(&key_path, key.serialize_pem()).unwrap();
    let mut expired_server = Endpoint::listen_quic(
        loopback(),
        QuicServerTls::PemFiles {
            cert_chain: cert_path.clone(),
            private_key: key_path,
        },
        NetConfig::default(),
    )
    .unwrap();
    let addr = expired_server.add_wss_listener(loopback()).unwrap();
    let mut expired_client = Endpoint::connect_wss(
        &url("localhost", addr, "/expired"),
        None,
        QuicTrust::PemFile(cert_path),
        NetConfig::default(),
    )
    .unwrap();
    assert!(matches!(
        terminal(&mut expired_client),
        Event::Disconnected {
            reason: DisconnectReason::ConnectFailed(_),
            ..
        }
    ));
    drop(expired_client);
    drop(expired_server);
    std::fs::remove_dir_all(expired_dir).unwrap();
}

#[derive(Clone, Copy, Debug)]
enum MockResponse {
    StallTls,
    SlowHttp,
    CumulativeDelay,
    ReflectError,
    NoProtocol,
}

// Each delayed phase is below the budget, but together they exceed it. The
// original 250 ms / 160 ms fixture left only 90 ms for TLS and scheduling.
// Preserve the cumulative-deadline oracle while giving real loopback TLS room.
const CONNECT_BUDGET: Duration = Duration::from_secs(1);
const PHASE_DELAY: Duration = Duration::from_millis(650);
const STALL_DELAY: Duration = Duration::from_secs(4);

#[derive(Debug, PartialEq)]
enum MockPhase {
    TcpAccepted,
    TlsAccepted,
    HttpRequestStarted,
    Failed(&'static str, String),
}

async fn observe_http_start<S: tokio::io::AsyncRead + Unpin>(
    stream: &mut tokio::io::BufReader<S>,
    phases: &mpsc::Sender<MockPhase>,
) -> bool {
    use tokio::io::AsyncBufReadExt;
    // Observe decrypted request bytes without consuming them: tungstenite must
    // still validate the original complete request in the cumulative case.
    match stream.fill_buf().await {
        Ok(bytes) if !bytes.is_empty() => {
            let _ = phases.send(MockPhase::HttpRequestStarted);
            true
        }
        result => {
            let error = match result {
                Ok(_) => "EOF before HTTP request".to_owned(),
                Err(error) => error.to_string(),
            };
            let _ = phases.send(MockPhase::Failed("HTTP request", error));
            false
        }
    }
}

struct MockServer {
    addr: SocketAddr,
    cert: Vec<u8>,
    alpn: mpsc::Receiver<Option<Vec<u8>>>,
    request: mpsc::Receiver<Vec<u8>>,
    phases: mpsc::Receiver<MockPhase>,
}

/// A tiny external TLS peer for timeout and malicious-server-response tests.
fn mock_tls_server(mode: MockResponse) -> MockServer {
    let ck = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let cert = ck.cert.der().to_vec();
    let key = rustls::pki_types::PrivateKeyDer::Pkcs8(ck.signing_key.serialize_der().into());
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![ck.cert.der().clone()], key)
        .unwrap();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let (addr_tx, addr_rx) = mpsc::channel();
    let (alpn_tx, alpn_rx) = mpsc::channel();
    let (request_tx, request_rx) = mpsc::channel();
    let (phase_tx, phase_rx) = mpsc::channel();

    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let listener = tokio::net::TcpListener::bind(loopback()).await.unwrap();
            addr_tx.send(listener.local_addr().unwrap()).unwrap();
            let stream = match listener.accept().await {
                Ok((stream, _)) => stream,
                Err(error) => {
                    let _ = phase_tx.send(MockPhase::Failed("TCP accept", error.to_string()));
                    return;
                }
            };
            let _ = phase_tx.send(MockPhase::TcpAccepted);
            if matches!(mode, MockResponse::StallTls) {
                tokio::time::sleep(STALL_DELAY).await;
                drop(stream);
                return;
            }
            if matches!(mode, MockResponse::CumulativeDelay) {
                tokio::time::sleep(PHASE_DELAY).await;
            }

            let mut stream = match acceptor.accept(stream).await {
                Ok(stream) => stream,
                Err(error) => {
                    let _ = phase_tx.send(MockPhase::Failed("TLS accept", error.to_string()));
                    return;
                }
            };
            let _ = phase_tx.send(MockPhase::TlsAccepted);
            let _ = alpn_tx.send(stream.get_ref().1.alpn_protocol().map(|protocol| protocol.to_vec()));
            match mode {
                MockResponse::StallTls => unreachable!(),
                MockResponse::SlowHttp => {
                    let mut stream = tokio::io::BufReader::new(stream);
                    if !observe_http_start(&mut stream, &phase_tx).await { return; }
                    tokio::time::sleep(STALL_DELAY).await;
                }
                MockResponse::CumulativeDelay => {
                    use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
                    let mut stream = tokio::io::BufReader::new(stream);
                    if !observe_http_start(&mut stream, &phase_tx).await { return; }
                    tokio::time::sleep(PHASE_DELAY).await;
                    #[allow(clippy::result_large_err)] // tungstenite callback requires this error type
                    let callback = |_request: &Request, mut response: Response| {
                        response.headers_mut().insert(
                            "sec-websocket-protocol",
                            tokio_tungstenite::tungstenite::http::HeaderValue::from_static("orrery/1"),
                        );
                        Ok(response)
                    };
                    let _ = tokio_tungstenite::accept_hdr_async_with_config(stream, callback, None).await;
                }
                MockResponse::ReflectError => {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut request = Vec::new();
                    let mut chunk = [0u8; 512];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") && request.len() < 8192 {
                        let Ok(n) = stream.read(&mut chunk).await else { return };
                        if n == 0 { return; }
                        request.extend_from_slice(&chunk[..n]);
                    }
                    let _ = request_tx.send(request.clone());
                    let body = format!(
                        "mock-server-reflection-secret {}",
                        String::from_utf8_lossy(&request),
                    );
                    let response = format!(
                        "HTTP/1.1 400 Bad Request\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(), body,
                    );
                    let mut stream = stream;
                    let _ = stream.write_all(response.as_bytes()).await;
                }
                MockResponse::NoProtocol => {
                    use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
                    #[allow(clippy::result_large_err)] // tungstenite callback requires this error type
                    let callback = move |request: &Request, mut response: Response| {
                        let _ = request_tx.send(
                            format!(
                                "{}\n{}",
                                request.uri(),
                                request.headers().get("host").and_then(|v| v.to_str().ok()).unwrap_or(""),
                            )
                            .into_bytes(),
                        );
                        response.headers_mut().remove("sec-websocket-protocol");
                        Ok(response)
                    };
                    let _ = tokio_tungstenite::accept_hdr_async_with_config(stream, callback, None).await;
                }
            }
        });
    });

    MockServer {
        addr: addr_rx.recv_timeout(Duration::from_secs(3)).unwrap(),
        cert,
        alpn: alpn_rx,
        request: request_rx,
        phases: phase_rx,
    }
}

fn assert_http11_alpn(alpn: &mpsc::Receiver<Option<Vec<u8>>>) {
    assert_eq!(
        alpn.recv_timeout(Duration::from_secs(3))
            .unwrap()
            .as_deref(),
        Some(b"http/1.1".as_slice())
    );
}

fn timeout_cfg() -> NetConfig {
    NetConfig {
        connect_timeout: CONNECT_BUDGET,
        ..NetConfig::default()
    }
}

#[test]
fn wss_tls_and_http_handshakes_share_the_connect_timeout() {
    assert!(PHASE_DELAY < CONNECT_BUDGET);
    assert!(PHASE_DELAY + PHASE_DELAY > CONNECT_BUDGET);
    assert!(STALL_DELAY > CONNECT_BUDGET);
    for mode in [
        MockResponse::StallTls,
        MockResponse::SlowHttp,
        MockResponse::CumulativeDelay,
    ] {
        let MockServer { addr, cert, alpn, phases, .. } = mock_tls_server(mode);
        // Match the IPv4 listener so localhost's IPv6 fallback cannot consume the timeout budget.
        let target = url("127.0.0.1", addr, "/timeout");
        let mut client = Endpoint::connect_wss(
            &target,
            Some("localhost"),
            QuicTrust::CertDer(cert),
            timeout_cfg(),
        )
        .unwrap();
        let outcome = terminal(&mut client);
        let expected = if matches!(mode, MockResponse::StallTls) {
            &[MockPhase::TcpAccepted][..]
        } else {
            &[MockPhase::TcpAccepted, MockPhase::TlsAccepted, MockPhase::HttpRequestStarted][..]
        };
        for phase in expected {
            let observed = phases.recv_timeout(Duration::from_secs(3));
            assert!(
                observed.as_ref() == Ok(phase),
                "{mode:?}: expected phase {phase:?}, got {observed:?}; client: {outcome:?}"
            );
        }
        if !matches!(mode, MockResponse::StallTls) {
            let observed = alpn.recv_timeout(Duration::from_secs(3));
            assert_eq!(
                observed.as_ref().map(|protocol| protocol.as_deref()),
                Ok(Some(b"http/1.1".as_slice())),
                "{mode:?}: TLS ALPN observation failed; client: {outcome:?}"
            );
        }
        assert!(
            matches!(&outcome, Event::Disconnected {
                reason: DisconnectReason::ConnectFailed(error), ..
            } if error.contains("timed out")),
            "{mode:?}: expected shared-budget timeout, got {outcome:?}"
        );
    }
}

#[test]
fn wss_errors_do_not_leak_server_or_url_secrets() {
    let MockServer { addr, cert, alpn, request: request_rx, .. } = mock_tls_server(MockResponse::ReflectError);
    let path_secret = "wss_path_secret_71a3";
    let query_secret = "wss_query_secret_28bf";
    let url = url(
        "localhost",
        addr,
        &format!("/{path_secret}?token={query_secret}"),
    );
    let mut client =
        Endpoint::connect_wss(&url, None, QuicTrust::CertDer(cert), NetConfig::default()).unwrap();
    let error = connect_failed(&mut client);
    assert_http11_alpn(&alpn);
    let request =
        String::from_utf8(request_rx.recv_timeout(Duration::from_secs(3)).unwrap()).unwrap();
    assert!(request.starts_with(&format!(
        "GET /{path_secret}?token={query_secret} HTTP/1.1\r\n"
    )));
    assert!(request
        .to_ascii_lowercase()
        .contains("sec-websocket-protocol: orrery/1\r\n"));
    for secret in ["mock-server-reflection-secret", path_secret, query_secret] {
        assert!(
            !error.contains(secret),
            "connection error leaked {secret:?}: {error:?}"
        );
    }
}

#[test]
fn wss_requires_server_to_select_the_native_subprotocol() {
    let MockServer { addr, cert, alpn, request: request_rx, .. } = mock_tls_server(MockResponse::NoProtocol);
    let mut client = Endpoint::connect_wss(
        &url("127.0.0.1", addr, "/no-subprotocol"),
        Some("localhost"),
        QuicTrust::CertDer(cert),
        NetConfig::default(),
    )
    .unwrap();
    assert!(matches!(
        terminal(&mut client),
        Event::Disconnected {
            reason: DisconnectReason::ConnectFailed(_),
            ..
        }
    ));
    assert_http11_alpn(&alpn);
    let request =
        String::from_utf8(request_rx.recv_timeout(Duration::from_secs(3)).unwrap()).unwrap();
    assert!(
        request.starts_with("/no-subprotocol\n127.0.0.1:"),
        "Host should remain the URL authority: {request:?}"
    );
}
