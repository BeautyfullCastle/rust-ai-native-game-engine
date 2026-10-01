use crate::config::NetConfig;
use crate::stats::{ConnStats, StatsCell};
use crate::tls::{self, QuicServerTls, QuicTrust};
use crate::{
    Channel, ConnId, DisconnectReason, Event, NetError, SendError, TAG_RELIABLE, TAG_UNRELIABLE,
};
use bytes::Bytes;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::{mpsc, watch};

/// Backend-agnostic view of an endpoint. Implemented by [`Endpoint`] and by
/// `Conditioned` (feature `conditioner`).
pub trait Transport {
    /// Next pending event, or `None`. Never blocks.
    fn poll_event(&mut self) -> Option<Event>;
    /// Moves all pending events into `out`. Never blocks.
    fn drain_events(&mut self, out: &mut Vec<Event>) {
        while let Some(e) = self.poll_event() {
            out.push(e);
        }
    }
    /// Queues a message. Never blocks.
    fn send(&mut self, conn: ConnId, channel: Channel, bytes: &[u8]) -> Result<(), SendError>;
    /// Starts a graceful close: queued reliable messages are flushed first.
    /// A `Disconnected { LocalClose }` event follows.
    fn close(&mut self, conn: ConnId);
    /// Counters of one live connection.
    fn stats(&self, conn: ConnId) -> Option<ConnStats>;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Link {
    Quic,
    /// WebTransport (HTTP/3 on QUIC): datagrams as the unreliable channel.
    Wt,
    Ws,
}

impl Link {
    /// True when the link carries native datagrams (when the peer allows them).
    fn datagram_capable(self) -> bool {
        matches!(self, Link::Quic | Link::Wt)
    }
}

pub(crate) enum Out {
    /// A framed message on the ordered stream.
    Stream { tag: u8, data: Bytes },
    /// A QUIC datagram.
    Datagram(Bytes),
    /// Flush, then close.
    Close,
}

pub(crate) struct ConnShared {
    pub id: ConnId,
    pub tx: mpsc::UnboundedSender<Out>,
    pub link: Link,
    pub stats: StatsCell,
    /// Set when this side asked for the close, so that the peer's answering
    /// close is not reported as a remote close.
    pub closing: std::sync::atomic::AtomicBool,
}

pub(crate) struct Shared {
    pub cfg: NetConfig,
    pub conns: RwLock<HashMap<ConnId, Arc<ConnShared>>>,
    pub next_id: AtomicU64,
    pub events: mpsc::Sender<Event>,
    pub shutdown: watch::Sender<bool>,
    /// Handshakes in progress (server), counted against `max_connections`.
    pub pending: std::sync::atomic::AtomicUsize,
    /// Server: this endpoint answers WebTransport (ALPN `h3`) on its QUIC port.
    pub webtransport: bool,
}

impl Shared {
    pub fn alloc_id(&self) -> ConnId {
        self.next_id.fetch_add(1, Relaxed)
    }

    pub fn register(
        &self,
        id: ConnId,
        link: Link,
        max_unreliable: usize,
        native: bool,
    ) -> (Arc<ConnShared>, mpsc::UnboundedReceiver<Out>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let cs = Arc::new(ConnShared {
            id,
            tx,
            link,
            stats: StatsCell::new(max_unreliable, native),
            closing: std::sync::atomic::AtomicBool::new(false),
        });
        self.conns.write().unwrap().insert(id, cs.clone());
        (cs, rx)
    }

    /// Awaits queue space, so a caller that stops polling pauses the reader.
    pub async fn emit(&self, ev: Event) {
        let _ = self.events.send(ev).await;
    }

    /// Removes the connection, then reports its end. Last event of a connection.
    pub async fn finish(&self, id: ConnId, reason: DisconnectReason) {
        let cs = self.conns.write().unwrap().remove(&id);
        let reason = match (reason, cs) {
            (DisconnectReason::RemoteClose, Some(cs)) if cs.closing.load(Relaxed) => DisconnectReason::LocalClose,
            (r, _) => r,
        };
        self.emit(Event::Disconnected { conn: id, reason }).await;
    }

    pub fn conn_count(&self) -> usize {
        self.conns.read().unwrap().len() + self.pending.load(Relaxed)
    }
}

/// A server or client transport endpoint. See the crate docs.
///
/// Dropping the endpoint closes all connections gracefully (bounded to about
/// one second) and stops its background threads.
pub struct Endpoint {
    shared: Arc<Shared>,
    events: mpsc::Receiver<Event>,
    runtime: Option<Runtime>,
    local_addr: SocketAddr,
    server_cert: Option<Vec<u8>>,
    identity: Option<tls::Identity>,
    client_conn: Option<ConnId>,
    quic: Option<quinn::Endpoint>,
}

impl Endpoint {
    fn new(cfg: NetConfig, webtransport: bool) -> Result<(Endpoint, watch::Receiver<bool>), NetError> {
        let runtime = Builder::new_multi_thread()
            .worker_threads(cfg.worker_threads.max(1))
            .thread_name("orr_net")
            .enable_all()
            .build()
            .map_err(NetError::new)?;
        let (events_tx, events_rx) = mpsc::channel(cfg.event_queue.max(16));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let shared = Arc::new(Shared {
            cfg,
            conns: RwLock::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            events: events_tx,
            shutdown: shutdown_tx,
            pending: std::sync::atomic::AtomicUsize::new(0),
            webtransport,
        });
        let ep = Endpoint {
            shared,
            events: events_rx,
            runtime: Some(runtime),
            local_addr: SocketAddr::from(([0, 0, 0, 0], 0)),
            server_cert: None,
            identity: None,
            client_conn: None,
            quic: None,
        };
        Ok((ep, shutdown_rx))
    }

    fn handle(&self) -> tokio::runtime::Handle {
        self.runtime.as_ref().expect("runtime alive").handle().clone()
    }

    /// Starts a QUIC server. `bind` may use port 0; see [`Endpoint::local_addr`].
    pub fn listen_quic(
        bind: SocketAddr,
        tls: QuicServerTls,
        cfg: NetConfig,
    ) -> Result<Endpoint, NetError> {
        Endpoint::listen_quic_with(bind, tls, cfg, false)
    }

    /// Like [`Endpoint::listen_quic`]; with `webtransport` the same UDP port also
    /// answers browsers over WebTransport (ALPN `h3`, see the crate docs).
    pub fn listen_quic_with(
        bind: SocketAddr,
        tls: QuicServerTls,
        cfg: NetConfig,
        webtransport: bool,
    ) -> Result<Endpoint, NetError> {
        let (mut ep, shutdown) = Endpoint::new(cfg, webtransport)?;
        let identity = tls::Identity::load(&tls)?;
        let server_cfg = tls::build_quic_server_config(&identity, &ep.shared.cfg, webtransport)?;
        let cert = identity.generated.clone();
        ep.identity = Some(identity);
        let handle = ep.handle();
        let _guard = handle.enter();
        let qep = quinn::Endpoint::server(server_cfg, bind).map_err(NetError::new)?;
        ep.local_addr = qep.local_addr().map_err(NetError::new)?;
        ep.server_cert = cert;
        ep.quic = Some(qep.clone());
        handle.spawn(crate::quic::accept_loop(qep, ep.shared.clone(), shutdown));
        Ok(ep)
    }

    /// Starts a plain (non-TLS) WebSocket server. For `wss://`, terminate TLS in a reverse proxy.
    pub fn listen_ws(bind: SocketAddr, cfg: NetConfig) -> Result<Endpoint, NetError> {
        let (mut ep, shutdown) = Endpoint::new(cfg, false)?;
        let handle = ep.handle();
        let listener = handle
            .block_on(tokio::net::TcpListener::bind(bind))
            .map_err(NetError::new)?;
        ep.local_addr = listener.local_addr().map_err(NetError::new)?;
        handle.spawn(crate::ws::accept_loop(listener, ep.shared.clone(), shutdown, None));
        Ok(ep)
    }

    /// Also accepts plain WebSocket connections on `bind`, on this endpoint, so one
    /// server serves QUIC, WebTransport and WebSocket clients through one event
    /// queue. Returns the bound address. Server endpoints only.
    pub fn add_ws_listener(&mut self, bind: SocketAddr) -> Result<SocketAddr, NetError> {
        let handle = self.handle();
        let listener = handle
            .block_on(tokio::net::TcpListener::bind(bind))
            .map_err(NetError::new)?;
        let addr = listener.local_addr().map_err(NetError::new)?;
        handle.spawn(crate::ws::accept_loop(listener, self.shared.clone(), self.shared.shutdown.subscribe(), None));
        Ok(addr)
    }

    /// Like [`Endpoint::add_ws_listener`], but `wss://`: TLS with the certificate
    /// of this QUIC server (the generated one, or the PEM files). Browsers on an
    /// `https://` page need this (no mixed content), and for a self-signed
    /// certificate they must trust it. Only on endpoints made by
    /// [`Endpoint::listen_quic_with`].
    pub fn add_wss_listener(&mut self, bind: SocketAddr) -> Result<SocketAddr, NetError> {
        let id = self.identity.as_ref().ok_or_else(|| NetError("wss needs a QUIC server endpoint (it shares its certificate)".into()))?;
        let acceptor = tokio_rustls::TlsAcceptor::from(id.wss_config()?);
        let handle = self.handle();
        let listener = handle
            .block_on(tokio::net::TcpListener::bind(bind))
            .map_err(NetError::new)?;
        let addr = listener.local_addr().map_err(NetError::new)?;
        handle.spawn(crate::ws::accept_loop(listener, self.shared.clone(), self.shared.shutdown.subscribe(), Some(acceptor)));
        Ok(addr)
    }

    /// Starts connecting over QUIC. Returns at once. The result arrives as
    /// `Connected` or `Disconnected { ConnectFailed }` for [`Endpoint::client_conn`].
    /// `server_name` is the TLS name checked against the certificate.
    pub fn connect_quic(
        remote: SocketAddr,
        server_name: &str,
        trust: QuicTrust,
        cfg: NetConfig,
    ) -> Result<Endpoint, NetError> {
        let (mut ep, _shutdown) = Endpoint::new(cfg, false)?;
        let client_cfg = tls::build_quic_client_config(&trust, &ep.shared.cfg)?;
        let handle = ep.handle();
        let _guard = handle.enter();
        let bind: SocketAddr = if remote.is_ipv4() { ([0, 0, 0, 0], 0).into() } else { ([0u16; 8], 0).into() };
        let mut qep = quinn::Endpoint::client(bind).map_err(NetError::new)?;
        qep.set_default_client_config(client_cfg);
        ep.local_addr = qep.local_addr().map_err(NetError::new)?;
        ep.quic = Some(qep.clone());
        let id = ep.shared.alloc_id();
        ep.client_conn = Some(id);
        handle.spawn(crate::quic::client_conn(qep, ep.shared.clone(), id, remote, server_name.to_string()));
        Ok(ep)
    }

    /// Starts connecting over WebSocket to `ws://host:port/path`. Returns at once.
    pub fn connect_ws(url: &str, cfg: NetConfig) -> Result<Endpoint, NetError> {
        let (mut ep, _shutdown) = Endpoint::new(cfg, false)?;
        let target = crate::ws::parse_target(url)?;
        let id = ep.shared.alloc_id();
        ep.client_conn = Some(id);
        ep.handle().spawn(crate::ws::client_conn(ep.shared.clone(), id, target));
        Ok(ep)
    }

    /// Local socket address (the real port after binding port 0).
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Client endpoints: the id of their only connection.
    pub fn client_conn(&self) -> Option<ConnId> {
        self.client_conn
    }

    /// Server endpoints using [`QuicServerTls::SelfSigned`]: the DER certificate, to give to clients.
    pub fn server_cert_der(&self) -> Option<&[u8]> {
        self.server_cert.as_deref()
    }

    /// SHA-256 of [`Endpoint::server_cert_der`], for [`QuicTrust::Sha256Fingerprint`].
    pub fn server_cert_sha256(&self) -> Option<[u8; 32]> {
        self.server_cert.as_deref().map(tls::sha256)
    }

    /// Ids of live connections.
    pub fn connections(&self) -> Vec<ConnId> {
        let mut v: Vec<_> = self.shared.conns.read().unwrap().keys().copied().collect();
        v.sort_unstable();
        v
    }

    /// Next pending event. Never blocks.
    pub fn poll_event(&mut self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    /// Moves all pending events into `out`. Never blocks.
    pub fn drain_events(&mut self, out: &mut Vec<Event>) {
        while let Some(e) = self.poll_event() {
            out.push(e);
        }
    }

    /// Blocks up to `timeout` for the next event. For tools and tests, not for
    /// the game loop. Must not be called from inside a tokio runtime.
    pub fn wait_event(&mut self, timeout: Duration) -> Option<Event> {
        if let Some(e) = self.poll_event() {
            return Some(e);
        }
        let rt = self.runtime.as_ref()?;
        let rx = &mut self.events;
        rt.block_on(async { tokio::time::timeout(timeout, rx.recv()).await.ok().flatten() })
    }

    /// Queues a message. Never blocks. See [`SendError`].
    pub fn send(&self, conn: ConnId, channel: Channel, bytes: &[u8]) -> Result<(), SendError> {
        let cfg = &self.shared.cfg;
        let cs = self
            .shared
            .conns
            .read()
            .unwrap()
            .get(&conn)
            .cloned()
            .ok_or(SendError::UnknownConnection)?;
        if bytes.len() > cfg.max_message_size {
            return Err(SendError::TooLarge { max: cfg.max_message_size });
        }
        let st = &cs.stats;
        let queued = st.queued.load(Relaxed);
        match channel {
            Channel::Reliable => self.push_stream(&cs, TAG_RELIABLE, bytes),
            Channel::Unreliable => {
                if cs.link.datagram_capable() && st.native_datagrams.load(Relaxed) {
                    let max = st.max_unreliable.load(Relaxed);
                    if bytes.len() > max {
                        return Err(SendError::TooLarge { max });
                    }
                    cs.tx
                        .send(Out::Datagram(Bytes::copy_from_slice(bytes)))
                        .map_err(|_| SendError::UnknownConnection)
                } else if cs.link.datagram_capable() && !cfg.datagram_fallback {
                    Err(SendError::DatagramsUnsupported)
                } else if cs.link == Link::Ws && queued > cfg.unreliable_backlog_limit {
                    StatsCell::add(&st.unreliable_dropped, 1);
                    Ok(())
                } else {
                    self.push_stream(&cs, TAG_UNRELIABLE, bytes)
                }
            }
        }
    }

    fn push_stream(&self, cs: &ConnShared, tag: u8, bytes: &[u8]) -> Result<(), SendError> {
        let st = &cs.stats;
        if st.queued.load(Relaxed) + bytes.len() > self.shared.cfg.max_queued_send_bytes {
            return Err(SendError::Backpressure);
        }
        st.queued.fetch_add(bytes.len(), Relaxed);
        cs.tx
            .send(Out::Stream { tag, data: Bytes::copy_from_slice(bytes) })
            .map_err(|_| {
                st.queued.fetch_sub(bytes.len(), Relaxed);
                SendError::UnknownConnection
            })
    }

    /// Starts a graceful close of one connection.
    pub fn close(&self, conn: ConnId) {
        if let Some(cs) = self.shared.conns.read().unwrap().get(&conn) {
            cs.closing.store(true, Relaxed);
            let _ = cs.tx.send(Out::Close);
        }
    }

    /// Counters of one live connection.
    pub fn stats(&self, conn: ConnId) -> Option<ConnStats> {
        self.shared.conns.read().unwrap().get(&conn).map(|c| c.stats.snapshot())
    }

    fn shutdown_inner(&mut self) {
        let Some(rt) = self.runtime.take() else { return };
        let _ = self.shared.shutdown.send(true);
        for cs in self.shared.conns.read().unwrap().values() {
            cs.closing.store(true, Relaxed);
            let _ = cs.tx.send(Out::Close);
        }
        if tokio::runtime::Handle::try_current().is_ok() {
            rt.shutdown_background();
            return;
        }
        let shared = self.shared.clone();
        let quic = self.quic.take();
        // Drain events so tasks blocked on a full event queue can finish.
        let events = &mut self.events;
        rt.block_on(async move {
            let wait = async {
                while shared.conn_count() > 0 {
                    while events.try_recv().is_ok() {}
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            };
            let _ = tokio::time::timeout(Duration::from_secs(1), wait).await;
            if let Some(q) = quic {
                q.close(0u32.into(), b"bye");
                let _ = tokio::time::timeout(Duration::from_millis(300), q.wait_idle()).await;
            }
        });
        rt.shutdown_timeout(Duration::from_millis(200));
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        self.shutdown_inner();
    }
}

impl Transport for Endpoint {
    fn poll_event(&mut self) -> Option<Event> {
        Endpoint::poll_event(self)
    }
    fn send(&mut self, conn: ConnId, channel: Channel, bytes: &[u8]) -> Result<(), SendError> {
        Endpoint::send(self, conn, channel, bytes)
    }
    fn close(&mut self, conn: ConnId) {
        Endpoint::close(self, conn)
    }
    fn stats(&self, conn: ConnId) -> Option<ConnStats> {
        Endpoint::stats(self, conn)
    }
}
