//! WebSocket-specific tests: malformed and oversized input.
#![allow(clippy::float_arithmetic)]
#![allow(clippy::disallowed_types)]

mod common;
use common::*;
use futures_util::{SinkExt, StreamExt};
use orr_net::{Channel, DisconnectReason, NetConfig};
use std::time::Duration;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

fn cfg() -> NetConfig {
    NetConfig { max_message_size: 1000, connect_timeout: Duration::from_secs(1), ..NetConfig::default() }
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
}

async fn raw_connect(
    addr: std::net::SocketAddr,
    protocol: Option<&str>,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    tokio_tungstenite::tungstenite::Error,
> {
    let mut req = format!("ws://{addr}/").into_client_request().unwrap();
    if let Some(p) = protocol {
        req.headers_mut().insert("Sec-WebSocket-Protocol", p.parse().unwrap());
    }
    tokio_tungstenite::connect_async(req).await.map(|(ws, _)| ws)
}

fn assert_violation(msg: Message) {
    let mut s = server(Backend::Ws, cfg());
    let mut good = client(&s, cfg());
    let (good_s, good_c) = pair(&mut s.ep, &mut good);

    let rt = rt();
    let mut raw = rt.block_on(raw_connect(s.addr, Some("orrery/1"))).unwrap();
    let raw_id = expect_connected(&mut s.ep);
    rt.block_on(async { raw.send(msg).await.unwrap() });
    let (conn, reason) = expect_disconnect(&mut s.ep, WAIT);
    assert_eq!(conn, raw_id);
    assert!(matches!(reason, DisconnectReason::ProtocolViolation(_)), "{reason:?}");
    // the raw socket ends
    rt.block_on(async {
        let end = tokio::time::timeout(Duration::from_secs(3), async { while raw.next().await.is_some() {} }).await;
        assert!(end.is_ok(), "server did not close the bad connection");
    });

    good.send(good_c, Channel::Reliable, b"still fine").unwrap();
    let (conn, _, bytes) = expect_message(&mut s.ep);
    assert_eq!((conn, bytes.as_slice()), (good_s, &b"still fine"[..]));
}

#[test]
fn oversized_message_rejected() {
    let mut v = vec![0u8]; // valid tag
    v.extend(vec![7u8; 5000]);
    assert_violation(Message::Binary(v.into()));
}

#[test]
fn text_message_rejected() {
    assert_violation(Message::Text("hello".into()));
}

#[test]
fn empty_message_rejected() {
    assert_violation(Message::Binary(Vec::new().into()));
}

#[test]
fn unknown_tag_rejected() {
    assert_violation(Message::Binary(vec![9u8, 1, 2].into()));
}

#[test]
fn missing_subprotocol_refused() {
    let s = server(Backend::Ws, cfg());
    let rt = rt();
    assert!(rt.block_on(raw_connect(s.addr, None)).is_err());
    assert!(rt.block_on(raw_connect(s.addr, Some("something-else"))).is_err());
    assert!(rt.block_on(raw_connect(s.addr, Some("other, orrery/1"))).is_ok());
    assert!(s.ep.connections().len() <= 1);
}

#[test]
fn non_websocket_garbage_does_not_panic() {
    use std::io::Write;
    let mut s = server(Backend::Ws, cfg());
    let mut sock = std::net::TcpStream::connect(s.addr).unwrap();
    sock.write_all(b"GET / HTTP/1.1\r\n\r\n\x00\x01\x02garbage\r\n\r\n").unwrap();
    drop(sock);
    let mut sock = std::net::TcpStream::connect(s.addr).unwrap();
    sock.write_all(&[0xff; 4096]).unwrap();
    drop(sock);
    // server still serves
    let mut c = client(&s, cfg());
    pair(&mut s.ep, &mut c);
}
