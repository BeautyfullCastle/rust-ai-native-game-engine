//! WebTransport backend (server side only), for browsers.
//!
//! A WebTransport session is one HTTP/3 extended-CONNECT on a QUIC connection
//! whose ALPN is `h3`. The server shares its UDP port with the `orrery/1`
//! QUIC backend: `quic::accept_loop` looks at the negotiated ALPN and hands
//! `h3` connections to [`accept`] here. Channels:
//!
//! * `Reliable`: the first bidirectional stream the browser opens, with the
//!   same framing as the QUIC backend (`u32 LE length | u8 tag | payload`, and
//!   the hello frame first).
//! * `Unreliable`: WebTransport datagrams, raw payload.
//!
//! Further streams the browser opens are ignored (never read).

use crate::endpoint::{ConnShared, Link, Out, Shared};
use crate::stats::StatsCell;
use crate::{Channel, DisconnectReason, Event, HELLO_PAYLOAD, TAG_HELLO, TAG_RELIABLE, TAG_UNRELIABLE};
use bytes::Bytes;
use h3::ext::Protocol;
use h3::quic::BidiStream as _;
use h3_datagram::datagram_handler::{DatagramReader, DatagramSender};
use h3_webtransport::server::{AcceptedBi, WebTransportSession};
use h3_webtransport::stream::{RecvStream, SendStream};
use quinn::VarInt;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::time::timeout;

type Session = WebTransportSession<h3_quinn::Connection, Bytes>;
type Recv = RecvStream<h3_quinn::RecvStream, Bytes>;
type Send_ = SendStream<h3_quinn::SendStream<Bytes>, Bytes>;

/// Bytes the WebTransport datagram framing adds to a payload (quarter stream id
/// of the session, a varint of one byte for the first sessions), plus a spare byte.
pub(crate) const DATAGRAM_OVERHEAD: usize = 2;

/// A session whose reliable stream has said hello.
pub(crate) struct Accepted {
    session: Session,
    send: Send_,
    recv: Recv,
}

/// True when the QUIC handshake negotiated ALPN `h3`.
pub(crate) fn is_h3(conn: &quinn::Connection) -> bool {
    conn.handshake_data()
        .and_then(|d| d.downcast::<quinn::crypto::rustls::HandshakeData>().ok())
        .is_some_and(|d| d.protocol.as_deref() == Some(crate::tls::ALPN_H3))
}

/// HTTP/3 handshake, the CONNECT request, and the reliable stream's hello.
/// `None` for anything that is not a well-formed Orrery WebTransport client.
pub(crate) async fn accept(shared: &Shared, conn: quinn::Connection) -> Option<Accepted> {
    let h3 = h3::server::builder()
        .enable_webtransport(true)
        .enable_extended_connect(true)
        .enable_datagram(true)
        .max_webtransport_sessions(1)
        .send_grease(false)
        .build(h3_quinn::Connection::new(conn))
        .await
        .ok()?;
    let session = accept_session(h3).await?;
    let max = shared.cfg.max_message_size;
    match session.accept_bi().await.ok()?? {
        AcceptedBi::BidiStream(_, bi) => {
            let (send, mut recv) = bi.split();
            match read_frame(&mut recv, max).await {
                Ok((TAG_HELLO, p)) if p == HELLO_PAYLOAD => Some(Accepted { session, send, recv }),
                _ => None,
            }
        }
        AcceptedBi::Request(..) => None,
    }
}

async fn accept_session<C>(mut h3: h3::server::Connection<C, Bytes>) -> Option<WebTransportSession<C, Bytes>>
where
    C: h3::quic::Connection<Bytes> + h3_datagram::quic_traits::DatagramConnectionExt<Bytes>,
{
    // CONNECT and the peer control stream arrive independently. Consume the
    // initial SETTINGS before WebTransport checks their negotiated capabilities.
    // The caller's existing setup deadline bounds this wait.
    if !matches!(std::future::poll_fn(|cx| h3.inner.poll_control(cx)).await.ok()?, h3::proto::frame::Frame::Settings(_)) {
        return None;
    }
    let (req, stream) = h3.accept().await.ok()??.resolve_request().await.ok()?;
    let is_wt =
        req.method() == http::Method::CONNECT && req.extensions().get::<Protocol>() == Some(&Protocol::WEB_TRANSPORT);
    if !is_wt {
        return None;
    }
    WebTransportSession::accept(req, stream, h3).await.ok()
}

#[cfg(test)]
#[path = "wt/settings_tests.rs"]
mod settings_tests;

pub(crate) async fn server_conn(shared: Arc<Shared>, conn: quinn::Connection, acc: Accepted) {
    let id = shared.alloc_id();
    let peer = conn.remote_address();
    let max = conn.max_datagram_size().map(|m| m.saturating_sub(DATAGRAM_OVERHEAD));
    let (cs, rx) = shared.register(id, Link::Wt, max.unwrap_or(0), max.is_some());
    shared.emit(Event::Connected { conn: id, peer: Some(peer) }).await;
    let reason = drive(&shared, &cs, rx, &conn, acc).await;
    match &reason {
        DisconnectReason::ProtocolViolation(m) => conn.close(VarInt::from_u32(1), m.as_bytes()),
        DisconnectReason::TimedOut => {}
        _ => conn.close(VarInt::from_u32(0), b"bye"),
    }
    shared.finish(id, reason).await;
}

fn map_conn_err(e: &quinn::ConnectionError) -> DisconnectReason {
    match e {
        quinn::ConnectionError::TimedOut => DisconnectReason::TimedOut,
        quinn::ConnectionError::LocallyClosed => DisconnectReason::LocalClose,
        quinn::ConnectionError::ApplicationClosed(_) | quinn::ConnectionError::ConnectionClosed(_) => {
            DisconnectReason::RemoteClose
        }
        other => DisconnectReason::Error(other.to_string()),
    }
}

fn io_reason(conn: &quinn::Connection, msg: impl ToString) -> DisconnectReason {
    match conn.close_reason() {
        Some(e) => map_conn_err(&e),
        None => DisconnectReason::Error(msg.to_string()),
    }
}

async fn drive(
    shared: &Shared,
    cs: &ConnShared,
    rx: mpsc::UnboundedReceiver<Out>,
    conn: &quinn::Connection,
    acc: Accepted,
) -> DisconnectReason {
    let Accepted { session, send, recv } = acc;
    let dgram_reader = session.datagram_reader();
    let dgram_sender = session.datagram_sender();
    tokio::select! {
        r = reader(shared, cs, conn, recv) => r,
        r = datagrams(shared, cs, dgram_reader) => r,
        r = writer(cs, conn, rx, send, dgram_sender) => r,
        r = keep_session(&session) => r,
        e = conn.closed() => map_conn_err(&e),
    }
}

/// Keeps polling the HTTP/3 connection (control streams, further requests) and
/// ignores any extra stream the peer opens.
async fn keep_session(session: &Session) -> DisconnectReason {
    loop {
        match session.accept_bi().await {
            Ok(Some(_)) => {} // dropped: the stream is reset
            Ok(None) => return DisconnectReason::RemoteClose,
            Err(e) => return DisconnectReason::Error(e.to_string()),
        }
    }
}

pub(crate) enum FrameError {
    Eof,
    Violation(String),
    Io(String),
}

pub(crate) async fn read_frame(recv: &mut (impl tokio::io::AsyncRead + Unpin), max: usize) -> Result<(u8, Vec<u8>), FrameError> {
    let mut head = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        match recv.read(&mut head[got..]).await {
            Ok(0) if got == 0 => return Err(FrameError::Eof),
            Ok(0) => return Err(FrameError::Violation("truncated frame header".into())),
            Ok(n) => got += n,
            Err(e) => return Err(FrameError::Io(e.to_string())),
        }
    }
    let len = u32::from_le_bytes(head) as usize;
    if len == 0 {
        return Err(FrameError::Violation("zero-length frame".into()));
    }
    if len - 1 > max {
        return Err(FrameError::Violation(format!("frame of {} bytes exceeds limit {max}", len - 1)));
    }
    let mut tag = [0u8; 1];
    if recv.read_exact(&mut tag).await.is_err() {
        return Err(FrameError::Io("stream ended before frame tag".into()));
    }
    let n = len - 1;
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

async fn reader(shared: &Shared, cs: &ConnShared, conn: &quinn::Connection, mut recv: Recv) -> DisconnectReason {
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
        count_in(&cs.stats, channel, bytes.len());
        shared.emit(Event::Message { conn: cs.id, channel, bytes }).await;
    }
}

pub(crate) fn count_in(st: &StatsCell, channel: Channel, len: usize) {
    StatsCell::add(&st.messages_received, 1);
    StatsCell::add(&st.payload_received, len as u64);
    if channel == Channel::Unreliable {
        StatsCell::add(&st.unreliable_received, 1);
    }
}

async fn datagrams(
    shared: &Shared,
    cs: &ConnShared,
    mut reader: DatagramReader<h3_quinn::datagram::RecvDatagramHandler>,
) -> DisconnectReason {
    loop {
        match reader.read_datagram().await {
            Ok(d) => {
                let b: Bytes = d.into_payload();
                if b.len() > shared.cfg.max_message_size {
                    StatsCell::add(&cs.stats.unreliable_dropped, 1);
                    continue;
                }
                count_in(&cs.stats, Channel::Unreliable, b.len());
                let ev = Event::Message { conn: cs.id, channel: Channel::Unreliable, bytes: b.to_vec() };
                if shared.events.try_send(ev).is_err() {
                    StatsCell::add(&cs.stats.unreliable_dropped, 1);
                }
            }
            Err(e) => return DisconnectReason::Error(e.to_string()),
        }
    }
}

pub(crate) async fn write_frame(send: &mut (impl tokio::io::AsyncWrite + Unpin), tag: u8, payload: &[u8]) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(payload.len() + 5);
    buf.extend_from_slice(&(payload.len() as u32 + 1).to_le_bytes());
    buf.push(tag);
    buf.extend_from_slice(payload);
    send.write_all(&buf).await
}

async fn writer(
    cs: &ConnShared,
    conn: &quinn::Connection,
    mut rx: mpsc::UnboundedReceiver<Out>,
    mut send: Send_,
    mut dgrams: DatagramSender<h3_quinn::datagram::SendDatagramHandler, Bytes>,
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
                    st.queued.fetch_sub(n.max(1), Relaxed);
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
                    st.queued.fetch_sub(b.len().max(1), Relaxed);
                    match dgrams.send_datagram(b) {
                        Ok(()) => {
                            StatsCell::add(&st.messages_sent, 1);
                            StatsCell::add(&st.unreliable_sent, 1);
                            StatsCell::add(&st.payload_sent, n);
                        }
                        Err(_) => StatsCell::add(&st.unreliable_dropped, 1),
                    }
                }
                Some(Out::Close) => {
                    // Flush and finish the stream, give the browser a moment to read it.
                    let _ = timeout(Duration::from_secs(1), send.shutdown()).await;
                    let _ = timeout(Duration::from_millis(300), conn.closed()).await;
                    return DisconnectReason::LocalClose;
                }
            },
            _ = tick.tick() => {
                crate::quic::update_stats(cs, conn, DATAGRAM_OVERHEAD);
            }
        }
    }
}
