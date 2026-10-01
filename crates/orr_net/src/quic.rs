//! QUIC backend: one ordered bidirectional stream (length-prefixed frames)
//! for `Reliable`, QUIC datagrams for `Unreliable`.

use crate::endpoint::{ConnShared, Link, Out, Shared};
use crate::stats::StatsCell;
use crate::{
    Channel, ConnId, DisconnectReason, Event, HELLO_PAYLOAD, TAG_HELLO, TAG_RELIABLE, TAG_UNRELIABLE,
};
use quinn::{ConnectionError, RecvStream, SendStream, VarInt};
use std::net::SocketAddr;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::sync::{mpsc, watch};
use tokio::time::timeout;

const CLOSE_OK: u32 = 0;
const CLOSE_PROTOCOL: u32 = 1;

pub(crate) async fn accept_loop(ep: quinn::Endpoint, shared: Arc<Shared>, mut shutdown: watch::Receiver<bool>) {
    loop {
        tokio::select! {
            incoming = ep.accept() => {
                let Some(incoming) = incoming else { break };
                if shared.conn_count() >= shared.cfg.max_connections {
                    incoming.refuse();
                    continue;
                }
                shared.pending.fetch_add(1, Relaxed);
                tokio::spawn(server_conn(shared.clone(), incoming));
            }
            _ = shutdown.changed() => break,
        }
    }
}

enum Accepted {
    Quic(quinn::Connection, SendStream, RecvStream),
    Wt(quinn::Connection, Box<crate::wt::Accepted>),
}

async fn server_conn(shared: Arc<Shared>, incoming: quinn::Incoming) {
    let setup = timeout(shared.cfg.connect_timeout, async {
        let conn = incoming.await.ok()?;
        // ALPN dispatch: `h3` is a WebTransport browser, `orrery/1` a native client.
        if shared.webtransport && crate::wt::is_h3(&conn) {
            return match crate::wt::accept(&shared, conn.clone()).await {
                Some(acc) => Some(Accepted::Wt(conn, Box::new(acc))),
                None => {
                    conn.close(VarInt::from_u32(CLOSE_PROTOCOL), b"bad webtransport session");
                    None
                }
            };
        }
        let (send, mut recv) = conn.accept_bi().await.ok()?;
        // The client writes the hello right after opening the stream.
        match read_frame(&mut recv, shared.cfg.max_message_size).await {
            Ok((TAG_HELLO, p)) if p == HELLO_PAYLOAD => Some(Accepted::Quic(conn, send, recv)),
            _ => {
                conn.close(VarInt::from_u32(CLOSE_PROTOCOL), b"bad hello");
                None
            }
        }
    })
    .await;
    shared.pending.fetch_sub(1, Relaxed);
    let (conn, send, recv) = match setup {
        Ok(Some(Accepted::Quic(conn, send, recv))) => (conn, send, recv),
        Ok(Some(Accepted::Wt(conn, acc))) => return crate::wt::server_conn(shared, conn, *acc).await,
        _ => return,
    };
    let id = shared.alloc_id();
    let peer = conn.remote_address();
    let (cs, rx) = register(&shared, id, &conn);
    shared.emit(Event::Connected { conn: id, peer: Some(peer) }).await;
    drive(shared, id, cs, rx, conn, send, recv).await;
}

pub(crate) async fn client_conn(
    ep: quinn::Endpoint,
    shared: Arc<Shared>,
    id: ConnId,
    remote: SocketAddr,
    server_name: String,
) {
    let setup = timeout(shared.cfg.connect_timeout, async {
        let connecting = ep.connect(remote, &server_name).map_err(|e| e.to_string())?;
        let conn = connecting.await.map_err(|e| e.to_string())?;
        let (mut send, recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
        write_frame(&mut send, TAG_HELLO, HELLO_PAYLOAD).await.map_err(|e| e.to_string())?;
        Ok::<_, String>((conn, send, recv))
    })
    .await;
    let (conn, send, recv) = match setup {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return shared.finish(id, DisconnectReason::ConnectFailed(e)).await,
        Err(_) => {
            return shared.finish(id, DisconnectReason::ConnectFailed("connect timed out".into())).await
        }
    };
    let (cs, rx) = register(&shared, id, &conn);
    shared.emit(Event::Connected { conn: id, peer: Some(remote) }).await;
    drive(shared, id, cs, rx, conn, send, recv).await;
}

fn register(
    shared: &Shared,
    id: ConnId,
    conn: &quinn::Connection,
) -> (Arc<ConnShared>, mpsc::UnboundedReceiver<Out>) {
    let max = conn.max_datagram_size();
    shared.register(id, Link::Quic, max.unwrap_or(shared.cfg.max_message_size), max.is_some())
}

async fn drive(
    shared: Arc<Shared>,
    id: ConnId,
    cs: Arc<ConnShared>,
    rx: mpsc::UnboundedReceiver<Out>,
    conn: quinn::Connection,
    send: SendStream,
    recv: RecvStream,
) {
    let reason = tokio::select! {
        r = reader(&shared, &cs, &conn, recv) => r,
        r = datagrams(&shared, &cs, &conn) => r,
        r = writer(&cs, &conn, rx, send) => r,
        e = conn.closed() => map_conn_err(&e),
    };
    match &reason {
        DisconnectReason::ProtocolViolation(m) => conn.close(VarInt::from_u32(CLOSE_PROTOCOL), m.as_bytes()),
        DisconnectReason::TimedOut => {}
        _ => conn.close(VarInt::from_u32(CLOSE_OK), b"bye"),
    }
    shared.finish(id, reason).await;
}

fn map_conn_err(e: &ConnectionError) -> DisconnectReason {
    match e {
        ConnectionError::TimedOut => DisconnectReason::TimedOut,
        ConnectionError::LocallyClosed => DisconnectReason::LocalClose,
        ConnectionError::ApplicationClosed(_) | ConnectionError::ConnectionClosed(_) => {
            DisconnectReason::RemoteClose
        }
        other => DisconnectReason::Error(other.to_string()),
    }
}

/// Reason for an I/O failure: the connection's own close reason if it has one.
fn io_reason(conn: &quinn::Connection, msg: impl ToString) -> DisconnectReason {
    match conn.close_reason() {
        Some(e) => map_conn_err(&e),
        None => DisconnectReason::Error(msg.to_string()),
    }
}

/// Reads one frame `u32 len | u8 tag | payload`. `len` counts tag and payload.
/// Errors: `Ok(None)`-like end is reported as `FrameError::Eof`.
enum FrameError {
    Eof,
    Violation(String),
    Io(String),
}

async fn read_frame(recv: &mut RecvStream, max: usize) -> Result<(u8, Vec<u8>), FrameError> {
    let mut head = [0u8; 4];
    match recv.read_exact(&mut head).await {
        Ok(()) => {}
        Err(quinn::ReadExactError::FinishedEarly(0)) => return Err(FrameError::Eof),
        Err(quinn::ReadExactError::FinishedEarly(_)) => {
            return Err(FrameError::Violation("truncated frame header".into()))
        }
        Err(quinn::ReadExactError::ReadError(e)) => return Err(FrameError::Io(e.to_string())),
    }
    let len = u32::from_le_bytes(head) as usize;
    if len == 0 {
        return Err(FrameError::Violation("zero-length frame".into()));
    }
    if len - 1 > max {
        return Err(FrameError::Violation(format!("frame of {} bytes exceeds limit {max}", len - 1)));
    }
    let n = len - 1;
    let mut tag = [0u8; 1];
    if recv.read_exact(&mut tag).await.is_err() {
        return Err(FrameError::Io("stream ended before frame tag".into()));
    }
    // Grows as bytes arrive, so a bogus length cannot force a big allocation.
    let mut buf = Vec::with_capacity(n.min(64 * 1024));
    match (&mut *recv).take(n as u64).read_to_end(&mut buf).await {
        Ok(_) if buf.len() == n => Ok((tag[0], buf)),
        Ok(_) => Err(FrameError::Violation("truncated frame".into())),
        Err(e) => Err(FrameError::Io(e.to_string())),
    }
}

impl From<FrameError> for DisconnectReason {
    fn from(e: FrameError) -> Self {
        match e {
            FrameError::Eof => DisconnectReason::RemoteClose,
            FrameError::Violation(m) => DisconnectReason::ProtocolViolation(m),
            FrameError::Io(m) => DisconnectReason::Error(m),
        }
    }
}

async fn write_frame(send: &mut SendStream, tag: u8, payload: &[u8]) -> Result<(), quinn::WriteError> {
    let mut head = [0u8; 5];
    head[..4].copy_from_slice(&(payload.len() as u32 + 1).to_le_bytes());
    head[4] = tag;
    send.write_all(&head).await?;
    send.write_all(payload).await
}

async fn reader(
    shared: &Shared,
    cs: &ConnShared,
    conn: &quinn::Connection,
    mut recv: RecvStream,
) -> DisconnectReason {
    let id = cs.id;
    loop {
        let (tag, bytes) = match read_frame(&mut recv, shared.cfg.max_message_size).await {
            Ok(f) => f,
            Err(FrameError::Io(m)) => return io_reason(conn, m),
            Err(e) => return e.into(),
        };
        let channel = match tag {
            TAG_RELIABLE => Channel::Reliable,
            TAG_UNRELIABLE => Channel::Unreliable,
            t => return DisconnectReason::ProtocolViolation(format!("unknown frame tag {t}")),
        };
        count_in(&cs.stats, channel, bytes.len(), bytes.len() as u64 + 5);
        shared.emit(Event::Message { conn: id, channel, bytes }).await;
    }
}

fn count_in(st: &StatsCell, channel: Channel, len: usize, _wire: u64) {
    StatsCell::add(&st.messages_received, 1);
    StatsCell::add(&st.payload_received, len as u64);
    if channel == Channel::Unreliable {
        StatsCell::add(&st.unreliable_received, 1);
    }
}

async fn datagrams(shared: &Shared, cs: &ConnShared, conn: &quinn::Connection) -> DisconnectReason {
    let id = cs.id;
    loop {
        match conn.read_datagram().await {
            Ok(b) => {
                if b.len() > shared.cfg.max_message_size {
                    StatsCell::add(&cs.stats.unreliable_dropped, 1);
                    continue;
                }
                count_in(&cs.stats, Channel::Unreliable, b.len(), 0);
                let ev = Event::Message { conn: id, channel: Channel::Unreliable, bytes: b.to_vec() };
                if shared.events.try_send(ev).is_err() {
                    StatsCell::add(&cs.stats.unreliable_dropped, 1);
                }
            }
            Err(e) => return map_conn_err(&e),
        }
    }
}

async fn writer(
    cs: &ConnShared,
    conn: &quinn::Connection,
    mut rx: mpsc::UnboundedReceiver<Out>,
    mut send: SendStream,
) -> DisconnectReason {
    let st = &cs.stats;
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            out = rx.recv() => match out {
                None => return DisconnectReason::LocalClose,
                Some(Out::Stream { tag, data }) => {
                    let n = data.len();
                    let res = write_frame(&mut send, tag, &data).await;
                    st.queued.fetch_sub(n, Relaxed);
                    if let Err(e) = res {
                        return io_reason(conn, e);
                    }
                    StatsCell::add(&st.messages_sent, 1);
                    StatsCell::add(&st.payload_sent, n as u64);
                    if tag == TAG_UNRELIABLE {
                        StatsCell::add(&st.unreliable_sent, 1);
                    }
                }
                Some(Out::Datagram(b)) => {
                    let n = b.len() as u64;
                    match conn.send_datagram(b) {
                        Ok(()) => {
                            StatsCell::add(&st.messages_sent, 1);
                            StatsCell::add(&st.unreliable_sent, 1);
                            StatsCell::add(&st.payload_sent, n);
                        }
                        Err(quinn::SendDatagramError::ConnectionLost(e)) => return map_conn_err(&e),
                        Err(_) => StatsCell::add(&st.unreliable_dropped, 1),
                    }
                }
                Some(Out::Close) => {
                    // Flush the stream, wait for the peer to read it, then close.
                    let _ = send.finish();
                    let _ = timeout(Duration::from_secs(1), send.stopped()).await;
                    return DisconnectReason::LocalClose;
                }
            },
            _ = tick.tick() => update_stats(cs, conn, 0),
        }
    }
}

/// `overhead`: bytes the datagram framing of the application protocol adds (WebTransport).
pub(crate) fn update_stats(cs: &ConnShared, conn: &quinn::Connection, overhead: usize) {
    let st = &cs.stats;
    st.set_rtt(conn.rtt());
    let s = conn.stats();
    st.packets_sent.store(s.udp_tx.datagrams, Relaxed);
    st.packets_received.store(s.udp_rx.datagrams, Relaxed);
    st.bytes_sent.store(s.udp_tx.bytes, Relaxed);
    st.bytes_received.store(s.udp_rx.bytes, Relaxed);
    if s.path.sent_packets > 0 {
        let ppm = (s.path.lost_packets as u128 * 1_000_000 / s.path.sent_packets as u128) as u64;
        st.loss_ppm.store(ppm, Relaxed);
    }
    match conn.max_datagram_size() {
        Some(m) => {
            st.max_unreliable.store(m.saturating_sub(overhead), Relaxed);
            st.native_datagrams.store(true, Relaxed);
        }
        None => st.native_datagrams.store(false, Relaxed),
    }
}
