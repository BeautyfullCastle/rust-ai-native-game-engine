//! WebSocket backend. Both channels share the ordered WS stream. Each binary
//! message is `tag | payload`; see the crate docs.

use crate::endpoint::{ConnShared, Link, Out, Shared};
use crate::stats::StatsCell;
use crate::{Channel, ConnId, DisconnectReason, Event, NetError, PROTOCOL, TAG_RELIABLE, TAG_UNRELIABLE};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::error::{CapacityError, Error as WsError};
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

pub(crate) struct Target {
    host: String,
    port: u16,
    uri: String,
}

pub(crate) fn parse_target(url: &str) -> Result<Target, NetError> {
    let rest = url
        .strip_prefix("ws://")
        .ok_or_else(|| NetError("only ws:// URLs are supported (terminate TLS in a proxy)".into()))?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !p.contains(']') => (h, p.parse::<u16>().map_err(NetError::new)?),
        _ => (authority, 80),
    };
    if host.is_empty() {
        return Err(NetError("empty host in URL".into()));
    }
    Ok(Target {
        host: host.trim_matches(|c| c == '[' || c == ']').to_string(),
        port,
        uri: format!("ws://{authority}{path}"),
    })
}

fn ws_config(max_message: usize) -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(max_message + 1))
        .max_frame_size(Some(max_message + 1))
}

pub(crate) async fn accept_loop(listener: TcpListener, shared: Arc<Shared>, mut shutdown: watch::Receiver<bool>) {
    loop {
        tokio::select! {
            res = listener.accept() => {
                let Ok((stream, peer)) = res else { continue };
                if shared.conn_count() >= shared.cfg.max_connections {
                    continue; // dropping the socket refuses the connection
                }
                shared.pending.fetch_add(1, Relaxed);
                tokio::spawn(server_conn(shared.clone(), stream, peer));
            }
            _ = shutdown.changed() => break,
        }
    }
}

#[allow(clippy::result_large_err)] // signature fixed by the tungstenite callback trait
fn check_protocol(req: &Request, mut resp: Response) -> Result<Response, ErrorResponse> {
    let ok = req
        .headers()
        .get_all("sec-websocket-protocol")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|p| p.trim() == PROTOCOL);
    if !ok {
        let mut e = ErrorResponse::new(Some("expected sub-protocol orrery/1".to_string()));
        *e.status_mut() = http::StatusCode::BAD_REQUEST;
        return Err(e);
    }
    resp.headers_mut()
        .insert("sec-websocket-protocol", http::HeaderValue::from_static(PROTOCOL));
    Ok(resp)
}

async fn server_conn(shared: Arc<Shared>, stream: TcpStream, peer: SocketAddr) {
    let _ = stream.set_nodelay(true);
    let cfg = ws_config(shared.cfg.max_message_size);
    let ws = timeout(
        shared.cfg.connect_timeout,
        tokio_tungstenite::accept_hdr_async_with_config(stream, check_protocol, Some(cfg)),
    )
    .await;
    shared.pending.fetch_sub(1, Relaxed);
    let Ok(Ok(ws)) = ws else { return };
    let id = shared.alloc_id();
    run(shared, id, ws, Some(peer)).await;
}

pub(crate) async fn client_conn(shared: Arc<Shared>, id: ConnId, target: Target) {
    let fail = |m: String| DisconnectReason::ConnectFailed(m);
    let setup = timeout(shared.cfg.connect_timeout, async {
        let stream = TcpStream::connect((target.host.as_str(), target.port))
            .await
            .map_err(|e| e.to_string())?;
        let _ = stream.set_nodelay(true);
        let peer = stream.peer_addr().ok();
        let req = http::Request::builder()
            .uri(&target.uri)
            .header("Host", format!("{}:{}", target.host, target.port))
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", generate_key())
            .header("Sec-WebSocket-Protocol", PROTOCOL)
            .body(())
            .map_err(|e| e.to_string())?;
        let cfg = ws_config(shared.cfg.max_message_size);
        let (ws, _) = tokio_tungstenite::client_async_with_config(req, stream, Some(cfg))
            .await
            .map_err(|e| e.to_string())?;
        Ok::<_, String>((ws, peer))
    })
    .await;
    match setup {
        Ok(Ok((ws, peer))) => run(shared, id, ws, peer).await,
        Ok(Err(e)) => shared.finish(id, fail(e)).await,
        Err(_) => shared.finish(id, fail("connect timed out".into())).await,
    }
}

fn map_err(e: WsError) -> DisconnectReason {
    match e {
        WsError::ConnectionClosed | WsError::AlreadyClosed => DisconnectReason::RemoteClose,
        WsError::Capacity(c) => DisconnectReason::ProtocolViolation(match c {
            CapacityError::MessageTooLong { size, max_size } => {
                format!("message of {size} bytes exceeds limit {max_size}")
            }
            other => other.to_string(),
        }),
        WsError::Protocol(p) => DisconnectReason::ProtocolViolation(p.to_string()),
        WsError::Utf8(_) => DisconnectReason::ProtocolViolation("invalid utf-8".into()),
        WsError::Io(io) if io.kind() == std::io::ErrorKind::TimedOut => DisconnectReason::TimedOut,
        // A peer that vanishes without a close handshake.
        WsError::Io(io)
            if matches!(
                io.kind(),
                std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::BrokenPipe
            ) =>
        {
            DisconnectReason::RemoteClose
        }
        other => DisconnectReason::Error(other.to_string()),
    }
}

async fn run(shared: Arc<Shared>, id: ConnId, ws: WebSocketStream<TcpStream>, peer: Option<SocketAddr>) {
    let max = shared.cfg.max_message_size;
    let (cs, rx) = shared.register(id, Link::Ws, max, false);
    shared.emit(Event::Connected { conn: id, peer }).await;
    let reason = drive(&shared, id, &cs, rx, ws).await;
    shared.finish(id, reason).await;
}

async fn drive(
    shared: &Shared,
    id: ConnId,
    cs: &ConnShared,
    mut rx: mpsc::UnboundedReceiver<Out>,
    ws: WebSocketStream<TcpStream>,
) -> DisconnectReason {
    let cfg = &shared.cfg;
    let st = &cs.stats;
    let (mut sink, mut stream) = ws.split();
    let t0 = Instant::now();
    let mut last_rx = Instant::now();
    let mut ping = tokio::time::interval(cfg.keepalive_interval);
    let mut idle = tokio::time::interval((cfg.idle_timeout / 4).max(Duration::from_millis(10)));
    let write_timeout = cfg.idle_timeout;
    loop {
        tokio::select! {
            msg = stream.next() => {
                last_rx = Instant::now();
                match msg {
                    None => return DisconnectReason::RemoteClose,
                    Some(Err(e)) => return map_err(e),
                    Some(Ok(Message::Binary(b))) => {
                        let Some((&tag, payload)) = b.split_first() else {
                            return DisconnectReason::ProtocolViolation("empty message".into());
                        };
                        let channel = match tag {
                            TAG_RELIABLE => Channel::Reliable,
                            TAG_UNRELIABLE => Channel::Unreliable,
                            t => return DisconnectReason::ProtocolViolation(format!("unknown tag {t}")),
                        };
                        StatsCell::add(&st.messages_received, 1);
                        StatsCell::add(&st.payload_received, payload.len() as u64);
                        StatsCell::add(&st.packets_received, 1);
                        StatsCell::add(&st.bytes_received, b.len() as u64);
                        if channel == Channel::Unreliable {
                            StatsCell::add(&st.unreliable_received, 1);
                        }
                        shared.emit(Event::Message { conn: id, channel, bytes: payload.to_vec() }).await;
                    }
                    Some(Ok(Message::Pong(p))) => {
                        if let Ok(a) = <[u8; 8]>::try_from(&p[..]) {
                            let sent = u64::from_le_bytes(a);
                            let now = t0.elapsed().as_micros() as u64;
                            st.set_rtt(Duration::from_micros(now.saturating_sub(sent)));
                        }
                    }
                    Some(Ok(Message::Ping(_))) => {} // tungstenite queues the pong itself
                    Some(Ok(Message::Close(_))) => return DisconnectReason::RemoteClose,
                    Some(Ok(_)) => {
                        return DisconnectReason::ProtocolViolation("text or raw frame not allowed".into())
                    }
                }
            }
            out = rx.recv() => match out {
                None => return DisconnectReason::LocalClose,
                Some(Out::Stream { tag, data }) => {
                    let n = data.len();
                    let mut v = Vec::with_capacity(n + 1);
                    v.push(tag);
                    v.extend_from_slice(&data);
                    let wire = v.len() as u64;
                    let res = timeout(write_timeout, sink.send(Message::Binary(Bytes::from(v)))).await;
                    st.queued.fetch_sub(n, Relaxed);
                    match res {
                        Err(_) => return DisconnectReason::TimedOut,
                        Ok(Err(e)) => return map_err(e),
                        Ok(Ok(())) => {}
                    }
                    StatsCell::add(&st.messages_sent, 1);
                    StatsCell::add(&st.payload_sent, n as u64);
                    StatsCell::add(&st.packets_sent, 1);
                    StatsCell::add(&st.bytes_sent, wire);
                    if tag == TAG_UNRELIABLE {
                        StatsCell::add(&st.unreliable_sent, 1);
                    }
                }
                Some(Out::Datagram(_)) => {} // not used on this backend
                Some(Out::Close) => {
                    let _ = timeout(Duration::from_secs(1), sink.send(Message::Close(None))).await;
                    // Give the peer a moment to answer with its own close frame.
                    let _ = timeout(Duration::from_millis(500), async { while stream.next().await.is_some() {} }).await;
                    return DisconnectReason::LocalClose;
                }
            },
            _ = ping.tick() => {
                let ts = (t0.elapsed().as_micros() as u64).to_le_bytes();
                let res = timeout(write_timeout, sink.send(Message::Ping(Bytes::copy_from_slice(&ts)))).await;
                match res {
                    Err(_) => return DisconnectReason::TimedOut,
                    Ok(Err(e)) => return map_err(e),
                    Ok(Ok(())) => {}
                }
            }
            _ = idle.tick() => {
                if last_rx.elapsed() >= cfg.idle_timeout {
                    return DisconnectReason::TimedOut;
                }
            }
        }
    }
}
