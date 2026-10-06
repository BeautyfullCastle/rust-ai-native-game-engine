//! Native WebTransport client, using the same framing and queues as the server.
use crate::endpoint::{ConnShared, Link, Out, Shared};
use crate::stats::StatsCell;
use crate::{Channel, ConnId, DisconnectReason, Event, NetConfig, NetError, QuicTrust, HELLO_PAYLOAD, TAG_HELLO, TAG_RELIABLE, TAG_UNRELIABLE};
use std::sync::{atomic::Ordering::Relaxed, Arc};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;
use wtransport::{Connection, RecvStream, SendStream};

pub(crate) fn target(url: &str) -> Result<String, NetError> {
    let invalid = || NetError("invalid https:// WebTransport URL".into());
    let rest = url.strip_prefix("https://").ok_or_else(invalid)?;
    // Reuse strict authority/port/userinfo/fragment validation without exposing
    // the URL. wtransport performs its own URL parse inside the deadline.
    crate::ws::parse_wss_target(&format!("wss://{rest}")).map_err(|_| invalid())?;
    Ok(url.into())
}

pub(crate) fn config(trust: &QuicTrust, cfg: &NetConfig) -> Result<wtransport::ClientConfig, NetError> {
    let error = || NetError("WT TLS configuration failed (use WebPki or a valid certificate CA)".into());
    // The verified-only WSS builder already rejects fingerprint/bypass trust.
    let mut tls = crate::tls::build_client_rustls(trust, true).map_err(|_| error())?;
    tls.alpn_protocols = vec![crate::tls::ALPN_H3.to_vec()];
    let transport = crate::tls::transport_config(cfg, false, true).map_err(|_| error())?;
    Ok(wtransport::ClientConfig::builder().with_bind_default()
        .with_custom_tls_and_transport(tls, transport).build())
}

pub(crate) async fn client_conn(ep: wtransport::Endpoint<wtransport::endpoint::endpoint_side::Client>, shared: Arc<Shared>, id: ConnId, url: String) {
    let mut shutdown = shared.shutdown.subscribe();
    let setup = async {
        let conn = ep.connect(url).await.map_err(|_| ())?;
        let (mut send, recv) = conn.open_bi().await.map_err(|_| ())?.await.map_err(|_| ())?;
        crate::wt::write_frame(&mut send, TAG_HELLO, HELLO_PAYLOAD).await.map_err(|_| ())?;
        Ok::<_, ()>((conn, send, recv))
    };
    let result = tokio::select! {
        r = timeout(shared.cfg.connect_timeout, setup) => r,
        _ = shutdown.changed() => { ep.close(0u32.into(), b"bye"); return; }
    };
    let (conn, send, recv) = match result {
        Ok(Ok(v)) => v,
        failure => {
            ep.close(0u32.into(), b"bye");
            let message = if failure.is_err() { "WT connect timed out" } else { "WT connection failed" };
            shared.finish(id, DisconnectReason::ConnectFailed(message.into())).await;
            return;
        }
    };
    let max = max_datagram_size(&conn);
    let (cs, rx) = shared.register(id, Link::Wt, max.unwrap_or(0), max.is_some());
    shared.emit(Event::Connected { conn: id, peer: Some(conn.remote_address()) }).await;
    let reason = tokio::select! {
        r = reader(&shared, &cs, &conn, recv) => r,
        r = datagrams(&shared, &cs, &conn) => r,
        r = writer(&cs, &conn, rx, send) => r,
        _ = conn.closed() => close_reason(&conn),
    };
    conn.close(0u32.into(), b"bye");
    ep.close(0u32.into(), b"bye");
    shared.finish(id, reason).await;
}

// wtransport 0.7.2 subtracts this header unchecked in max_datagram_size.
// A peer may legitimately advertise a QUIC datagram limit below the header.
fn datagram_header(conn: &Connection) -> usize {
    wtransport::proto::datagram::Datagram::header_size(
        wtransport::proto::ids::QStreamId::from_session_id(conn.session_id()),
    )
}

fn payload_limit(quic_limit: Option<usize>, header: usize) -> Option<usize> {
    quic_limit.and_then(|n| n.checked_sub(header))
}

fn max_datagram_size(conn: &Connection) -> Option<usize> {
    payload_limit(conn.quic_connection().max_datagram_size(), datagram_header(conn))
}

fn close_reason(conn: &Connection) -> DisconnectReason {
    match conn.quic_connection().close_reason() {
        Some(quinn::ConnectionError::TimedOut) => DisconnectReason::TimedOut,
        Some(quinn::ConnectionError::LocallyClosed) => DisconnectReason::LocalClose,
        Some(quinn::ConnectionError::ApplicationClosed(_) | quinn::ConnectionError::ConnectionClosed(_)) => DisconnectReason::RemoteClose,
        _ => DisconnectReason::Error("WT transport ended".into()),
    }
}

async fn reader(shared: &Shared, cs: &ConnShared, conn: &Connection, mut recv: RecvStream) -> DisconnectReason {
    loop {
        let (tag, bytes) = match crate::wt::read_frame(&mut recv, shared.cfg.max_message_size).await {
            Ok(v) => v,
            Err(crate::wt::FrameError::Io(_)) => return close_reason(conn),
            Err(e) => return e.into(),
        };
        let channel = match tag {
            TAG_RELIABLE => Channel::Reliable,
            TAG_UNRELIABLE => Channel::Unreliable,
            _ => return DisconnectReason::ProtocolViolation("unknown frame tag".into()),
        };
        crate::wt::count_in(&cs.stats, channel, bytes.len());
        shared.emit(Event::Message { conn: cs.id, channel, bytes }).await;
    }
}

async fn datagrams(shared: &Shared, cs: &ConnShared, conn: &Connection) -> DisconnectReason {
    loop {
        let d = match conn.receive_datagram().await {
            Ok(d) => d,
            Err(_) => return close_reason(conn),
        };
        let bytes = d.payload();
        if bytes.len() > shared.cfg.max_message_size {
            StatsCell::add(&cs.stats.unreliable_dropped, 1);
            continue;
        }
        crate::wt::count_in(&cs.stats, Channel::Unreliable, bytes.len());
        if shared.events.try_send(Event::Message { conn: cs.id, channel: Channel::Unreliable, bytes: bytes.to_vec() }).is_err() {
            StatsCell::add(&cs.stats.unreliable_dropped, 1);
        }
    }
}

async fn writer(cs: &ConnShared, conn: &Connection, mut rx: mpsc::UnboundedReceiver<Out>, mut send: SendStream) -> DisconnectReason {
    let st = &cs.stats;
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            out = rx.recv() => match out {
                None => return DisconnectReason::LocalClose,
                Some(Out::Stream { tag, data }) => {
                    let n = data.len();
                    let result = crate::wt::write_frame(&mut send, tag, &data).await;
                    st.queued.fetch_sub(n.max(1), Relaxed);
                    if result.is_err() { return close_reason(conn); }
                    StatsCell::add(&st.messages_sent, 1);
                    StatsCell::add(&st.payload_sent, n as u64);
                    if tag == TAG_UNRELIABLE { StatsCell::add(&st.unreliable_sent, 1); }
                }
                Some(Out::Datagram(data)) => {
                    let n = data.len();
                    st.queued.fetch_sub(n.max(1), Relaxed);
                    if conn.send_datagram(data).is_ok() {
                        StatsCell::add(&st.messages_sent, 1);
                        StatsCell::add(&st.payload_sent, n as u64);
                        StatsCell::add(&st.unreliable_sent, 1);
                    } else { StatsCell::add(&st.unreliable_dropped, 1); }
                }
                Some(Out::Close) => {
                    let _ = timeout(Duration::from_secs(1), send.finish()).await;
                    let _ = timeout(Duration::from_millis(300), conn.closed()).await;
                    return DisconnectReason::LocalClose;
                }
            },
            _ = tick.tick() => {
                crate::quic::update_stats(cs, conn.quic_connection(), datagram_header(conn));
                let max = max_datagram_size(conn);
                st.max_unreliable.store(max.unwrap_or(0), Relaxed);
                st.native_datagrams.store(max.is_some(), Relaxed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::payload_limit;

    #[test]
    fn tiny_peer_datagram_limits_do_not_underflow() {
        for header in [1, 2, 4, 8] {
            assert_eq!(payload_limit(None, header), None);
            assert_eq!(payload_limit(Some(0), header), None);
            assert_eq!(payload_limit(Some(header - 1), header), None);
            assert_eq!(payload_limit(Some(header), header), Some(0));
            assert_eq!(payload_limit(Some(header + 1), header), Some(1));
        }
    }
}
