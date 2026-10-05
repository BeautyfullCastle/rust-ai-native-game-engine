//! Actual loopback TLS termination in front of unchanged ERP hosts. No trust bypass.
#![allow(clippy::disallowed_types)]
mod common;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, atomic::{AtomicBool, AtomicU64, Ordering}};
use std::time::{Duration, Instant};
use orr_tui::source::{Incoming, Session, SocketOptions, SocketSource, Source};
use rustls::pki_types::PrivateKeyDer;

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Proxy {
    url: String,
    ca: PathBuf,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Proxy {
    fn new(upstream: &str, name: &str) -> Self {
        let identity = rcgen::generate_simple_self_signed(vec![name.into()]).unwrap();
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions().unwrap().with_no_client_auth()
            .with_single_cert(vec![identity.cert.der().clone()], PrivateKeyDer::Pkcs8(identity.signing_key.serialize_der().into())).unwrap();
        let ca = std::env::temp_dir().join(format!("orr-wss-{}-{}.pem", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::write(&ca, identity.cert.pem()).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("wss://127.0.0.1:{}", listener.local_addr().unwrap().port());
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let upstream = upstream.strip_prefix("ws://").unwrap().to_string();
        let worker = std::thread::spawn(move || {
            let tcp = loop {
                if stopped.load(Ordering::Relaxed) { return; }
                match listener.accept() {
                    Ok((tcp, _)) => break tcp,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(1)),
                    Err(e) => panic!("accept: {e}"),
                }
            };
            tcp.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            tcp.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut tls = rustls::StreamOwned::new(rustls::ServerConnection::new(Arc::new(config)).unwrap(), tcp);
            while tls.conn.is_handshaking() {
                if tls.conn.complete_io(&mut tls.sock).is_err() { return; }
            }
            tls.sock.set_read_timeout(Some(Duration::from_millis(2))).unwrap();
            let mut backend = TcpStream::connect(upstream).unwrap();
            backend.set_read_timeout(Some(Duration::from_millis(2))).unwrap();
            backend.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut bytes = [0u8; 65536];
            while !stopped.load(Ordering::Relaxed) {
                match tls.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(n) => if backend.write_all(&bytes[..n]).is_err() { break; },
                    Err(e) if pending(&e) => {},
                    Err(_) => break,
                }
                match backend.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(n) => if tls.write_all(&bytes[..n]).and_then(|()| tls.flush()).is_err() { break; },
                    Err(e) if pending(&e) => {},
                    Err(_) => break,
                }
            }
        });
        Self { url, ca, stop, worker: Some(worker) }
    }
    fn options(&self) -> SocketOptions { SocketOptions { ca_file: Some(self.ca.clone()), ..SocketOptions::default() } }
}
fn pending(e: &std::io::Error) -> bool {
    matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted)
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
        std::fs::remove_file(&self.ca).unwrap();
    }
}

#[test]
fn wss_authenticated_headless_checksum_equals_existing_rust_bridge() {
    use orr_remote::{Auth, Caps, ServerConfig, TokenEntry};
    let token = "secret-auth-token-do-not-display";
    let auth = Auth::Tokens(vec![TokenEntry { client: "tui".into(), token: token.into(), caps: Caps::ALL }]);
    let host = orr_remote::sample::spawn_phys_host(common::DEMO_SCENE.into(), None, ServerConfig::new(auth)).unwrap();
    let proxy = Proxy::new(host.url().unwrap(), "127.0.0.1");
    let mut src = SocketSource::connect_with_options(&proxy.url, Some(token), 1000, Session::Ensure { run: false }, &proxy.options()).unwrap();
    assert!(!src.describe().contains(token));
    let opts = orr_tui::headless::HeadlessOpts { frames: 120, size: (80, 30), dump: None };
    assert_eq!(orr_tui::headless::run(&mut src, &opts).unwrap().to_string(), common::rust_result(120));
}

fn first_frame(src: &mut SocketSource) -> Vec<u8> {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        match src.recv(Duration::from_millis(50)).unwrap() {
            Some(Incoming::Frame3(bytes)) => return bytes,
            Some(Incoming::Error(e)) => panic!("{e}"),
            _ => {},
        }
    }
    panic!("no 3D frame");
}

#[test]
fn wss_yard3d_schema_and_frame_bytes_equal_plain_ws() {
    let config = orr_sample::yard3d_game::YardConfig { rain_per_second: 0, max_entities: 256, ..orr_sample::yard3d_game::YardConfig::new(4) };
    let host = orr_remote::yard3d::spawn_yard3d_host(config, orr_remote::ServerConfig::new(orr_remote::Auth::DevNoAuth)).unwrap();
    let proxy = Proxy::new(host.url().unwrap(), "127.0.0.1");
    let mut tls = SocketSource::connect_with_options(&proxy.url, None, 1000, Session::Ensure { run: false }, &proxy.options()).unwrap();
    let mut plain = SocketSource::connect(host.url().unwrap(), None, 1000, Session::Leave).unwrap();
    assert_eq!(tls.schema_text(), plain.schema_text());
    assert_eq!(first_frame(&mut tls), first_frame(&mut plain));
}

#[test]
fn default_trust_rejects_untrusted_ca_without_disclosing_url() {
    let host = common::host();
    let proxy = Proxy::new(host.url().unwrap(), "127.0.0.1");
    let error = SocketSource::connect(&format!("{}/private?token=secret-query", proxy.url), None, 30, Session::Leave).err().unwrap();
    assert!(!error.contains("secret-query") && !error.contains("private"), "{error}");
    assert!(error.contains("handshake"), "{error}");
}

#[test]
fn explicit_ca_still_rejects_wrong_hostname() {
    let host = common::host();
    let proxy = Proxy::new(host.url().unwrap(), "different.example");
    let error = SocketSource::connect_with_options(&proxy.url, None, 30, Session::Leave, &proxy.options()).err().unwrap();
    assert!(error.contains("handshake"), "{error}");
}

#[test]
fn stalled_tls_handshake_has_a_deadline_and_redacted_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("wss://{}/secret-path?token=secret-query", listener.local_addr().unwrap());
    let peer = std::thread::spawn(move || {
        let (_tcp, _) = listener.accept().unwrap();
        std::thread::sleep(Duration::from_millis(300));
    });
    let start = Instant::now();
    let options = SocketOptions { handshake_timeout: Duration::from_millis(40), ..SocketOptions::default() };
    let error = SocketSource::connect_with_options(&url, None, 30, Session::Leave, &options).err().unwrap();
    assert!(start.elapsed() < Duration::from_millis(250), "{error}");
    assert!(!error.contains("secret"), "{error}");
    peer.join().unwrap();
}

#[allow(clippy::result_large_err)] // error type fixed by the tungstenite handshake callback trait
fn check_query_token(
    request: &tungstenite::handshake::server::Request,
    response: tungstenite::handshake::server::Response,
) -> Result<tungstenite::handshake::server::Response, tungstenite::handshake::server::ErrorResponse> {
    assert_eq!(request.uri().query(), Some("token=secret-query"));
    Ok(response)
}

#[test]
fn wss_query_auth_is_redacted_read_deadline_and_close_are_preserved() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let backend_url = format!("ws://{}", listener.local_addr().unwrap());
    let (close, closed) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (tcp, _) = listener.accept().unwrap();
        let mut ws = tungstenite::accept_hdr(tcp, check_query_token).unwrap();
        let request: serde_json::Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(request["method"], "watch.subscribe");
        ws.send(tungstenite::Message::text(serde_json::json!({"jsonrpc":"2.0","id":request["id"],"result":{}}).to_string())).unwrap();
        ws.send(tungstenite::Message::text(serde_json::json!({"method":"watch.viewstream.schema","params":{"format":"orrery.viewstream","version":1}}).to_string())).unwrap();
        closed.recv_timeout(Duration::from_secs(5)).unwrap();
        ws.close(None).unwrap();
    });
    let proxy = Proxy::new(&backend_url, "127.0.0.1");
    let mut source = SocketSource::connect_with_options(&format!("{}/?token=secret-query", proxy.url), None, 30, Session::Leave, &proxy.options()).unwrap();
    assert_eq!(source.describe(), format!("socket {}", proxy.url));
    let start = Instant::now();
    assert!(source.recv(Duration::from_millis(40)).unwrap().is_none());
    assert!(start.elapsed() < Duration::from_millis(250));
    close.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match source.recv(Duration::from_millis(50)) {
            Err(error) => { assert_eq!(error, "the host closed the connection"); break; },
            _ => assert!(Instant::now() < deadline, "close was not observed"),
        }
    }
    server.join().unwrap();
}

#[test]
fn tls_succeeds_but_stalled_http_upgrade_still_obeys_shared_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let backend_url = format!("ws://{}", listener.local_addr().unwrap());
    let peer = std::thread::spawn(move || {
        let (_tcp, _) = listener.accept().unwrap();
        std::thread::sleep(Duration::from_millis(300));
    });
    let proxy = Proxy::new(&backend_url, "127.0.0.1");
    let options = SocketOptions { handshake_timeout: Duration::from_millis(80), ..proxy.options() };
    let start = Instant::now();
    let error = SocketSource::connect_with_options(&proxy.url, None, 30, Session::Leave, &options).err().unwrap();
    assert!(start.elapsed() < Duration::from_millis(250), "{error}");
    assert!(error.contains("handshake"), "{error}");
    peer.join().unwrap();
}

#[test]
fn cli_rejects_ca_for_plain_transport_and_never_allows_erp_insecure() {
    for arguments in [
        vec!["--connect", "ws://127.0.0.1:1", "--ca-file", "missing.pem"],
        vec!["--connect", "wss://127.0.0.1:1", "--insecure"],
    ] {
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_orr_tui")).args(arguments).output().unwrap();
        assert!(!result.status.success());
        let stderr = String::from_utf8(result.stderr).unwrap();
        assert!(stderr.contains("requires --connect wss://") || stderr.contains("not supported for ERP"), "{stderr}");
    }
}

#[test]
fn explicit_ca_file_must_contain_valid_certificates() {
    let host = common::host();
    for pem in ["", "not PEM", "-----BEGIN CERTIFICATE-----\ninvalid\n-----END CERTIFICATE-----\n"] {
        let proxy = Proxy::new(host.url().unwrap(), "127.0.0.1");
        std::fs::write(&proxy.ca, pem).unwrap();
        let error = SocketSource::connect_with_options(&proxy.url, None, 30, Session::Leave, &proxy.options()).err().unwrap();
        assert!(error.contains("CA"), "{error}");
    }
}

#[test]
fn peer_reflected_secrets_are_not_in_http_or_auth_errors() {
    for http_error in [true, false] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let backend_url = format!("ws://{}", listener.local_addr().unwrap());
        let peer = std::thread::spawn(move || {
            let (mut tcp, _) = listener.accept().unwrap();
            if http_error {
                let mut request = [0u8; 4096];
                assert!(tcp.read(&mut request).unwrap() > 0);
                tcp.write_all(b"HTTP/1.1 403 secret-query\r\nContent-Length: 12\r\n\r\nsecret-token").unwrap();
            } else {
                let mut ws = tungstenite::accept(tcp).unwrap();
                let request: serde_json::Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
                assert_eq!(request["method"], "auth");
                assert_eq!(request["params"]["token"], "secret-token");
                ws.send(tungstenite::Message::text(serde_json::json!({"id":request["id"],"error":{"code":-1,"message":"secret-token secret-query"}}).to_string())).unwrap();
            }
        });
        let proxy = Proxy::new(&backend_url, "127.0.0.1");
        let error = SocketSource::connect_with_options(&format!("{}/?token=secret-query", proxy.url), Some("secret-token"), 30, Session::Leave, &proxy.options()).err().unwrap();
        assert!(!error.contains("secret"), "{error}");
        assert!(error.contains(if http_error { "handshake" } else { "authentication" }), "{error}");
        peer.join().unwrap();
    }
}

#[test]
fn slow_drip_http_upgrade_cannot_extend_the_tls_connection_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let backend_url = format!("ws://{}", listener.local_addr().unwrap());
    let peer = std::thread::spawn(move || {
        let (mut tcp, _) = listener.accept().unwrap();
        let mut request = [0u8; 4096];
        assert!(tcp.read(&mut request).unwrap() > 0);
        for byte in b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n" {
            if tcp.write_all(&[*byte]).is_err() { break; }
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    let proxy = Proxy::new(&backend_url, "127.0.0.1");
    let options = SocketOptions { handshake_timeout: Duration::from_millis(80), ..proxy.options() };
    let start = Instant::now();
    let error = SocketSource::connect_with_options(&proxy.url, None, 30, Session::Leave, &options).err().unwrap();
    assert!(start.elapsed() < Duration::from_millis(250), "{error}");
    assert!(error.contains("handshake"), "{error}");
    drop(proxy);
    peer.join().unwrap();
}
