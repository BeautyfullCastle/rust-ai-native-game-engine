//! WebTransport backend against a second implementation (the `wtransport`
//! client), and ALPN sharing of one UDP port between `orrery/1` QUIC and `h3`.
//! The browser itself is covered by `tools/webtransport/` (Playwright).

#![allow(clippy::disallowed_types)]
#![allow(clippy::float_arithmetic)]

mod common;

use common::*;
use orr_net::{Channel, Endpoint, Event, NetConfig, QuicServerTls, QuicTrust};
use std::net::SocketAddr;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use wtransport::tls::Sha256Digest;

/// What the scripted WebTransport client saw.
#[derive(Debug, Default)]
struct WtResult {
    stream_echo: Vec<u8>,
    datagram_echo: Vec<u8>,
    max_datagram: Option<usize>,
}

fn frame(tag: u8, payload: &[u8]) -> Vec<u8> {
    let mut v = (payload.len() as u32 + 1).to_le_bytes().to_vec();
    v.push(tag);
    v.extend_from_slice(payload);
    v
}

/// Connects with `wtransport`, says hello, sends "hello-wt" on the stream and
/// "dgram-wt" as a datagram, and waits for the echoes.
fn wt_client(addr: SocketAddr, fp: [u8; 32], tx: mpsc::Sender<Result<WtResult, String>>, hello: bool) {
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let res = rt.block_on(async move {
            let cfg = wtransport::ClientConfig::builder()
                .with_bind_address(([127, 0, 0, 1], 0).into())
                .with_server_certificate_hashes([Sha256Digest::new(fp)])
                .build();
            let ep = wtransport::Endpoint::client(cfg).map_err(|e| e.to_string())?;
            let conn = ep.connect(format!("https://127.0.0.1:{}", addr.port())).await.map_err(|e| e.to_string())?;
            let (mut send, mut recv) = conn.open_bi().await.map_err(|e| e.to_string())?.await.map_err(|e| e.to_string())?;
            let mut out = WtResult { max_datagram: conn.max_datagram_size(), ..Default::default() };
            if hello {
                send.write_all(&frame(2, b"ORRN\x01")).await.map_err(|e| e.to_string())?;
            } else {
                send.write_all(&frame(2, b"nope!")).await.map_err(|e| e.to_string())?;
            }
            send.write_all(&frame(0, b"hello-wt")).await.map_err(|e| e.to_string())?;
            conn.send_datagram(b"dgram-wt").map_err(|e| e.to_string())?;
            // Echoed frame: u32 len | tag | payload.
            let mut head = [0u8; 5];
            tokio::time::timeout(Duration::from_secs(5), recv.read_exact(&mut head))
                .await
                .map_err(|_| "no stream echo".to_string())?
                .map_err(|e| e.to_string())?;
            let n = u32::from_le_bytes(head[..4].try_into().unwrap()) as usize - 1;
            let mut body = vec![0u8; n];
            recv.read_exact(&mut body).await.map_err(|e| e.to_string())?;
            out.stream_echo = body;
            let d = tokio::time::timeout(Duration::from_secs(5), conn.receive_datagram())
                .await
                .map_err(|_| "no datagram echo".to_string())?
                .map_err(|e| e.to_string())?;
            out.datagram_echo = d.payload().to_vec();
            conn.close(0u32.into(), b"done");
            ep.wait_idle().await;
            Ok(out)
        });
        let _ = tx.send(res);
    });
}

fn wt_server() -> (Endpoint, SocketAddr, [u8; 32], Vec<u8>) {
    let ep = Endpoint::listen_quic_with(loopback(), QuicServerTls::dev_localhost_webtransport(), NetConfig::default(), true)
        .unwrap();
    let addr = ep.local_addr();
    let fp = ep.server_cert_sha256().unwrap();
    let der = ep.server_cert_der().unwrap().to_vec();
    (ep, addr, fp, der)
}

#[test]
fn webtransport_and_quic_share_one_port() {
    let (mut server, addr, fp, der) = wt_server();

    // A native QUIC client (ALPN orrery/1) on the same UDP port.
    let mut native = Endpoint::connect_quic(addr, "localhost", QuicTrust::CertDer(der), NetConfig::default()).unwrap();
    let native_srv = match wait_for(&mut server, WAIT, |e| matches!(e, Event::Connected { .. })) {
        Some(Event::Connected { conn, .. }) => conn,
        other => panic!("native client not accepted: {other:?}"),
    };
    expect_connected(&mut native);

    // A WebTransport client (ALPN h3) on the same port.
    let (tx, rx) = mpsc::channel();
    wt_client(addr, fp, tx, true);
    let wt_srv = match wait_for(&mut server, WAIT, |e| matches!(e, Event::Connected { .. })) {
        Some(Event::Connected { conn, .. }) => conn,
        other => panic!("webtransport client not accepted: {other:?}; client says {:?}", rx.try_recv()),
    };
    assert_ne!(native_srv, wt_srv);

    // Reliable + unreliable messages from the WT client arrive with the right channels.
    let mut got_stream = false;
    let mut got_dgram = false;
    while !(got_stream && got_dgram) {
        match wait_for(&mut server, WAIT, |e| matches!(e, Event::Message { conn, .. } if *conn == wt_srv)) {
            Some(Event::Message { channel: Channel::Reliable, bytes, .. }) => {
                assert_eq!(bytes, b"hello-wt");
                got_stream = true;
                server.send(wt_srv, Channel::Reliable, b"echo-stream").unwrap();
            }
            Some(Event::Message { channel: Channel::Unreliable, bytes, .. }) => {
                assert_eq!(bytes, b"dgram-wt");
                got_dgram = true;
                server.send(wt_srv, Channel::Unreliable, b"echo-dgram").unwrap();
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    let st = server.stats(wt_srv).unwrap();
    assert!(st.native_datagrams, "WebTransport uses real datagrams");

    // The native client still talks to the same server, with its own channels.
    native.send(native.client_conn().unwrap(), Channel::Reliable, b"native-hi").unwrap();
    match wait_for(&mut server, WAIT, |e| matches!(e, Event::Message { conn, .. } if *conn == native_srv)) {
        Some(Event::Message { bytes, .. }) => assert_eq!(bytes, b"native-hi"),
        other => panic!("{other:?}"),
    }

    let res = rx.recv_timeout(Duration::from_secs(10)).expect("client finished").expect("client ok");
    assert_eq!(res.stream_echo, b"echo-stream");
    assert_eq!(res.datagram_echo, b"echo-dgram");
    assert!(res.max_datagram.is_some());
}

#[test]
fn webtransport_without_hello_is_refused() {
    let (mut server, addr, fp, _) = wt_server();
    let (tx, rx) = mpsc::channel();
    wt_client(addr, fp, tx, false);
    // No Connected event, and the client fails.
    assert!(wait_for(&mut server, Duration::from_millis(1500), |e| matches!(e, Event::Connected { .. })).is_none());
    assert!(rx.recv_timeout(Duration::from_secs(10)).unwrap().is_err());
}

#[test]
fn h3_is_refused_when_webtransport_is_off() {
    // A plain QUIC server only offers `orrery/1`: an `h3` client cannot negotiate an ALPN.
    let ep = Endpoint::listen_quic(loopback(), QuicServerTls::dev_localhost_webtransport(), NetConfig::default()).unwrap();
    let (addr, fp) = (ep.local_addr(), ep.server_cert_sha256().unwrap());
    let (tx, rx) = mpsc::channel();
    wt_client(addr, fp, tx, true);
    assert!(rx.recv_timeout(Duration::from_secs(10)).unwrap().is_err());
    drop(ep);
}

/// `wss://` on the same certificate as QUIC, with the browser sub-protocol.
#[test]
fn wss_listener_uses_the_quic_certificate() {
    use futures_util::SinkExt;
    use rustls::pki_types::CertificateDer;
    use std::sync::Arc;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::Message;

    let (mut server, _addr, _fp, der) = wt_server();
    let wss = server.add_wss_listener(loopback()).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    // Keep the client socket open until the server has read the message: closing it
    // right after the send can reset the connection first (seen on Windows).
    let ws = rt.block_on(async {
        let mut roots = rustls::RootCertStore::empty();
        roots.add(CertificateDer::from(der)).unwrap();
        let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let tcp = tokio::net::TcpStream::connect(wss).await.unwrap();
        let tls = tokio_rustls::TlsConnector::from(Arc::new(cfg))
            .connect("localhost".try_into().unwrap(), tcp)
            .await
            .expect("TLS handshake with the QUIC certificate");
        let mut req = format!("wss://localhost:{}/", wss.port()).into_client_request().unwrap();
        req.headers_mut().insert("Sec-WebSocket-Protocol", "orrery.1".parse().unwrap());
        let (mut ws, _) = tokio_tungstenite::client_async(req, tls).await.expect("websocket handshake over TLS");
        ws.send(Message::Binary(vec![0u8, 4, 5, 6].into())).await.unwrap();
        ws
    });
    match wait_for(&mut server, WAIT, |e| matches!(e, Event::Message { .. })) {
        Some(Event::Message { channel: Channel::Reliable, bytes, .. }) => assert_eq!(bytes, [4, 5, 6]),
        other => panic!("{other:?}"),
    }
    drop(ws);
    drop(rt);
}
