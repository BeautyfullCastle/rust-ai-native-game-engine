//! Connections between an ERP client and a server, and what they carry.
//!
//! An [`ErpClient`](crate::ErpClient) or a [`RemoteBridge`](crate::RemoteBridge)
//! talks through a [`Transport`]. Three exist:
//!
//! - [`LocalTransport`]: **in process**. The server and the client share a
//!   process (the editor and its host thread). Requests are handed to the
//!   server's queue as parsed values (no JSON text, no socket); responses
//!   and notifications come back as text that the client parses; **frames
//!   are shared `Arc<Frame>` copies, never serialized**. A host hands out
//!   such connections through a [`LocalConnector`] (cloneable, usable from
//!   any thread) that [`ErpServer::connector`](crate::ErpServer::connector) gives.
//! - [`WsTransport`]: a blocking WebSocket (tests, CLI tools).
//! - [`PumpedWs`]: a WebSocket owned by a background thread, behind
//!   channels. `recv` and `send` never wait for the network, and a
//!   [`TxHandle`] can send from other threads. This is what a UI uses
//!   to attach to a host in another process.
//!
//! The same client code runs on all three.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::mpsc::{
    channel, sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError,
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use orr_ecs::Frame;
use serde_json::{json, Value as J};
use tokio::sync::mpsc::{channel as async_channel, Sender as AsyncSender};
use tokio::sync::Notify;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use crate::caps::Caps;
use crate::client::ClientError;
use crate::net::{ConnTx, Inbound, NetShared, RequestPermit};

/// Limits for one retained payload queue, not allocator or whole-process memory.
/// Zero in either dimension refuses every item. A zero-byte item still uses a slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueLimits {
    pub max_items: usize,
    pub max_bytes: usize,
}

impl Default for QueueLimits {
    fn default() -> Self {
        Self {
            max_items: 4096,
            max_bytes: 64 << 20,
        }
    }
}

/// Independent request and incoming-message queues of a client connection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransportQueueLimits {
    pub outgoing: QueueLimits,
    pub incoming: QueueLimits,
}

/// Live accounting. Bytes are the documented payload cost, not heap usage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueStats {
    pub limits: QueueLimits,
    pub current_items: usize,
    pub current_bytes: usize,
    pub peak_items: usize,
    pub peak_bytes: usize,
    pub saturations: u64,
    pub closed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransportQueueStats {
    pub outgoing: QueueStats,
    pub incoming: QueueStats,
}

/// A request was refused before enqueue, or its connection is terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    Backpressure,
    Disconnected,
}

impl SendError {
    fn client_error(self) -> ClientError {
        transport(match self {
            Self::Backpressure => "client request queue is full; request was not queued",
            Self::Disconnected => "the connection is closed",
        })
    }
}

struct Accounting {
    stats: QueueStats,
    reason: &'static str,
}

struct BudgetShared {
    accounting: Mutex<Accounting>,
    changed: Notify,
}

/// One counter owner for one queue. Both dimensions are reserved together.
#[derive(Clone)]
pub(crate) struct QueueBudget(Arc<BudgetShared>);

impl QueueBudget {
    pub(crate) fn new(limits: QueueLimits) -> Self {
        Self(Arc::new(BudgetShared {
            accounting: Mutex::new(Accounting {
                stats: QueueStats {
                    limits,
                    current_items: 0,
                    current_bytes: 0,
                    peak_items: 0,
                    peak_bytes: 0,
                    saturations: 0,
                    closed: false,
                },
                reason: "the connection is closed",
            }),
            changed: Notify::new(),
        }))
    }

    pub(crate) fn stats(&self) -> QueueStats {
        self.0
            .accounting
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .stats
    }

    pub(crate) fn reserve(&self, bytes: usize) -> Result<QueuePermit, SendError> {
        let mut accounting = self.0.accounting.lock().unwrap_or_else(|p| p.into_inner());
        let s = &mut accounting.stats;
        if s.closed {
            return Err(SendError::Disconnected);
        }
        let items = s.current_items.checked_add(1);
        let total = s.current_bytes.checked_add(bytes);
        if s.limits.max_bytes == 0
            || items.is_none_or(|n| n > s.limits.max_items)
            || total.is_none_or(|n| n > s.limits.max_bytes)
        {
            s.saturations = s.saturations.saturating_add(1);
            return Err(SendError::Backpressure);
        }
        s.current_items = items.unwrap();
        s.current_bytes = total.unwrap();
        s.peak_items = s.peak_items.max(s.current_items);
        s.peak_bytes = s.peak_bytes.max(s.current_bytes);
        Ok(QueuePermit {
            budget: self.clone(),
            bytes,
        })
    }

    pub(crate) fn reject(&self) {
        let mut a = self.0.accounting.lock().unwrap_or_else(|p| p.into_inner());
        a.stats.saturations = a.stats.saturations.saturating_add(1);
    }

    pub(crate) fn close(&self, reason: &'static str) {
        let mut a = self.0.accounting.lock().unwrap_or_else(|p| p.into_inner());
        if !a.stats.closed {
            a.reason = reason;
            a.stats.closed = true;
        }
        drop(a);
        self.0.changed.notify_one();
    }

    fn closed_error(&self) -> ClientError {
        transport(
            self.0
                .accounting
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .reason,
        )
    }

    async fn wait_closed(&self) {
        loop {
            let changed = self.0.changed.notified();
            if self.stats().closed {
                return;
            }
            changed.await;
        }
    }
}

pub(crate) struct QueuePermit {
    budget: QueueBudget,
    bytes: usize,
}

impl Drop for QueuePermit {
    fn drop(&mut self) {
        let mut a = self
            .budget
            .0
            .accounting
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        a.stats.current_items -= 1;
        a.stats.current_bytes -= self.bytes;
    }
}

pub(crate) struct Queued<T> {
    value: T,
    permit: QueuePermit,
}

impl<T> Queued<T> {
    fn into_inner(self) -> T {
        let Self { value, permit } = self;
        drop(permit);
        value
    }
}

pub(crate) struct IncomingSender {
    tx: SyncSender<Queued<Incoming>>,
    budget: QueueBudget,
}

impl Clone for IncomingSender {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            budget: self.budget.clone(),
        }
    }
}

impl IncomingSender {
    pub(crate) fn send(&self, incoming: Incoming, cost: usize) -> bool {
        let permit = match self.budget.reserve(cost) {
            Ok(p) => p,
            Err(_) => {
                self.budget.close("client receive queue is full or closed");
                return false;
            }
        };
        match self.tx.try_send(Queued {
            value: incoming,
            permit,
        }) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                self.budget.reject();
                self.budget.close("client receive queue is full");
                false
            }
            Err(TrySendError::Disconnected(_)) => {
                self.budget.close("client receive queue was dropped");
                false
            }
        }
    }

    pub(crate) fn pending(&self) -> usize {
        self.budget.stats().current_bytes
    }

    pub(crate) fn close(&self) {
        self.budget.close("the host closed the connection");
    }

    pub(crate) async fn closed(&self) {
        self.budget.wait_closed().await;
    }
}

struct IncomingReceiver {
    rx: Receiver<Queued<Incoming>>,
    budget: QueueBudget,
}

fn incoming_channel(limits: QueueLimits) -> (IncomingSender, IncomingReceiver) {
    let budget = QueueBudget::new(limits);
    let (tx, rx) = sync_channel(limits.max_items.max(1));
    (
        IncomingSender {
            tx,
            budget: budget.clone(),
        },
        IncomingReceiver { rx, budget },
    )
}

impl IncomingReceiver {
    fn recv(&self, timeout: Duration) -> Result<Option<Incoming>, ClientError> {
        let started = Instant::now();
        loop {
            if self.budget.stats().closed {
                // Terminal ingress loss makes every outstanding call uncertain.
                // Discarding the queue here releases its owned permits, never
                // presenting an incomplete fenced tail as a healthy stream.
                for queued in self.rx.try_iter() {
                    drop(queued);
                }
                return Err(self.budget.closed_error());
            }
            let left = timeout.saturating_sub(started.elapsed());
            match self.rx.recv_timeout(left.min(Duration::from_millis(10))) {
                Ok(queued) => {
                    if self.budget.stats().closed {
                        drop(queued);
                        continue;
                    }
                    return Ok(Some(queued.into_inner()));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    self.budget.close("the connection closed");
                    return Err(self.budget.closed_error());
                }
                Err(RecvTimeoutError::Timeout) if started.elapsed() >= timeout => return Ok(None),
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }
}

impl Drop for IncomingReceiver {
    fn drop(&mut self) {
        self.budget.close("client receive queue was dropped");
    }
}

/// One JSON-RPC request. `id: None` is a notification (no response).
#[derive(Clone, Debug)]
pub struct Request {
    /// The id the response will carry.
    pub id: Option<u64>,
    /// The method.
    pub method: String,
    /// The parameters (an object or null).
    pub params: J,
}

impl Request {
    /// The request as the JSON-RPC text that goes on a socket.
    pub fn to_text(&self) -> String {
        match self.id {
            Some(id) => {
                json!({"jsonrpc": "2.0", "id": id, "method": self.method, "params": self.params})
                    .to_string()
            }
            None => {
                json!({"jsonrpc": "2.0", "method": self.method, "params": self.params}).to_string()
            }
        }
    }
}

/// A frame sent to an in-process subscriber: the metadata of the wire
/// frame message (`tick`, `epoch`, `timeline`, ...) and a shared copy of
/// the frame. Nothing is serialized or compressed.
pub struct LocalFrame {
    /// The same metadata object the binary frame message carries.
    pub meta: J,
    /// The frame (a copy: the host keeps simulating its own).
    pub frame: Arc<Frame>,
    /// Queue cost, for the slow-reader limit.
    pub(crate) cost: usize,
}

/// What a server sends to a client.
pub enum Incoming {
    /// A JSON-RPC response or notification, as text.
    Text(String),
    /// A binary frame message of a socket (see [`crate::wire`]).
    Wire(Vec<u8>),
    /// A frame for an in-process client.
    Local(Arc<LocalFrame>),
}

/// A thread-safe way to send requests on a connection.
pub trait TxHandle: Send + Sync {
    /// Sends one request. Never waits for the response.
    fn send(&self, req: Request) -> Result<(), ClientError>;

    /// Distinguishes pre-enqueue refusal. Existing custom handles stay compatible.
    fn try_send(&self, req: Request) -> Result<(), SendError> {
        self.send(req).map_err(|_| SendError::Disconnected)
    }

    /// Live counters, when the transport provides bounded queues.
    fn queue_stats(&self) -> Option<TransportQueueStats> {
        None
    }

    /// Make a bounded connection terminal and wake its worker. Custom handles
    /// may rely on their owning transport being dropped instead.
    fn close(&self) {}
}

/// The client end of a connection to an ERP server.
pub trait Transport: Send {
    /// Sends one request.
    fn send(&mut self, req: Request) -> Result<(), ClientError>;

    /// Waits up to `timeout` for the next message. `Ok(None)` = nothing yet,
    /// `Err` = the connection is gone.
    fn recv(&mut self, timeout: Duration) -> Result<Option<Incoming>, ClientError>;

    /// A handle other threads can send through, if the transport has one.
    fn sender(&self) -> Option<Arc<dyn TxHandle>> {
        None
    }

    fn queue_stats(&self) -> Option<TransportQueueStats> {
        self.sender().and_then(|tx| tx.queue_stats())
    }

    /// Sends raw text (for tests of malformed input). Sockets only.
    fn send_raw_text(&mut self, _text: &str) -> Result<(), ClientError> {
        Err(ClientError::Transport(
            "this transport has no raw text".into(),
        ))
    }

    /// Sends raw bytes as a binary message. Sockets only.
    fn send_raw_binary(&mut self, _bytes: &[u8]) -> Result<(), ClientError> {
        Err(ClientError::Transport(
            "this transport has no raw binary messages".into(),
        ))
    }
}

fn transport(e: impl core::fmt::Display) -> ClientError {
    ClientError::Transport(e.to_string())
}

// ---- in process ----

/// Makes in-process connections to one server. Cloneable and `Send`: hand
/// it to the thread that owns the UI while the server runs on the host thread.
#[derive(Clone)]
pub struct LocalConnector {
    pub(crate) shared: Arc<NetShared>,
}

impl LocalConnector {
    /// A new connection as client `client` with `caps`. A client called
    /// [`USER_CLIENT`](crate::USER_CLIENT) edits as `Origin::User`.
    /// Fails if the server is gone.
    pub fn connect(&self, client: &str, caps: Caps) -> Result<LocalTransport, ClientError> {
        self.connect_with_queue_limits(client, caps, TransportQueueLimits::default())
    }

    /// Connect with independent retained request and incoming payload limits.
    /// The host's existing global request admission limit also applies.
    pub fn connect_with_queue_limits(
        &self,
        client: &str,
        caps: Caps,
        limits: TransportQueueLimits,
    ) -> Result<LocalTransport, ClientError> {
        let conn = self.shared.next_id.fetch_add(1, Relaxed);
        let (tx, rx) = incoming_channel(limits.incoming);
        let incoming = rx.budget.clone();
        let outgoing = QueueBudget::new(limits.outgoing);
        let to_client = ConnTx::bounded_local(tx);
        let hello = Inbound::Connected {
            conn,
            client: client.to_string(),
            caps,
            tx: to_client,
            binary: true,
        };
        self.shared
            .inbox
            .send(hello)
            .map_err(|_| transport("the host has stopped"))?;
        Ok(LocalTransport {
            tx: LocalTx {
                conn,
                shared: self.shared.clone(),
                outgoing,
                incoming,
            },
            rx,
        })
    }
}

#[derive(Clone)]
struct LocalTx {
    conn: u64,
    shared: Arc<NetShared>,
    outgoing: QueueBudget,
    incoming: QueueBudget,
}

impl TxHandle for LocalTx {
    fn send(&self, req: Request) -> Result<(), ClientError> {
        self.try_send(req).map_err(|error| match error {
            SendError::Disconnected if self.incoming.stats().closed => self.incoming.closed_error(),
            other => other.client_error(),
        })
    }

    fn try_send(&self, req: Request) -> Result<(), SendError> {
        if self.incoming.stats().closed {
            return Err(SendError::Disconnected);
        }
        let id = req.id.map(|i| json!(i));
        let permit = RequestPermit::reserve(&self.shared, id.as_ref(), &req.method, &req.params)
            .ok_or_else(|| {
                self.outgoing.reject();
                SendError::Backpressure
            })?
            .with_client_budget(&self.outgoing)?;
        let msg = Inbound::Request {
            permit,
            conn: self.conn,
            id,
            method: req.method,
            params: req.params,
        };
        self.shared.inbox.send(msg).map_err(|_| {
            self.outgoing.close("the host has stopped");
            self.incoming.close("the host has stopped");
            SendError::Disconnected
        })
    }

    fn queue_stats(&self) -> Option<TransportQueueStats> {
        Some(TransportQueueStats {
            outgoing: self.outgoing.stats(),
            incoming: self.incoming.stats(),
        })
    }

    fn close(&self) {
        self.outgoing.close("the client closed the connection");
        self.incoming.close("the client closed the connection");
    }
}

/// An in-process connection (see [`LocalConnector`]).
pub struct LocalTransport {
    tx: LocalTx,
    rx: IncomingReceiver,
}

impl Transport for LocalTransport {
    fn send(&mut self, req: Request) -> Result<(), ClientError> {
        self.tx.send(req)
    }

    fn recv(&mut self, timeout: Duration) -> Result<Option<Incoming>, ClientError> {
        self.rx.recv(timeout)
    }

    fn sender(&self) -> Option<Arc<dyn TxHandle>> {
        Some(Arc::new(self.tx.clone()))
    }
}

impl Drop for LocalTransport {
    fn drop(&mut self) {
        self.tx.outgoing.close("the client dropped the connection");
        self.tx.incoming.close("the client dropped the connection");
        let _ = self
            .tx
            .shared
            .inbox
            .send(Inbound::Disconnected { conn: self.tx.conn });
    }
}

// ---- WebSocket, blocking ----

/// A blocking WebSocket client connection.
pub struct WsTransport {
    ws: WebSocket<MaybeTlsStream<TcpStream>>,
}

impl WsTransport {
    /// Connects (the URL may carry `?token=`).
    pub fn connect(url: &str) -> Result<Self, ClientError> {
        let (ws, _) = tungstenite::connect(url).map_err(transport)?;
        if let MaybeTlsStream::Plain(s) = ws.get_ref() {
            let _ = s.set_nodelay(true);
        }
        Ok(Self { ws })
    }

    fn set_read_timeout(&mut self, d: Option<Duration>) {
        if let MaybeTlsStream::Plain(s) = self.ws.get_ref() {
            let _ = s.set_read_timeout(d);
        }
    }
}

/// Poll without retrying the RPC or resetting its deadline. The WebSocket keeps partial frames.
fn recv_ws(ws: &mut WebSocket<impl Read + Write>) -> Result<Option<Incoming>, ClientError> {
    match ws.read() {
        Ok(Message::Text(t)) => Ok(Some(Incoming::Text(t.as_str().to_string()))),
        Ok(Message::Binary(b)) => Ok(Some(Incoming::Wire(b.to_vec()))),
        Ok(Message::Close(_)) => Err(ClientError::Transport("closed by the server".into())),
        Ok(_) => Ok(None),
        Err(tungstenite::Error::Io(e))
            if matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::Interrupted
            ) =>
        {
            Ok(None)
        }
        Err(e) => Err(transport(e)),
    }
}

impl Transport for WsTransport {
    fn send(&mut self, req: Request) -> Result<(), ClientError> {
        self.send_raw_text(&req.to_text())
    }

    fn recv(&mut self, timeout: Duration) -> Result<Option<Incoming>, ClientError> {
        self.set_read_timeout(Some(timeout.max(Duration::from_millis(1))));
        recv_ws(&mut self.ws)
    }

    fn send_raw_text(&mut self, text: &str) -> Result<(), ClientError> {
        self.ws.send(Message::text(text)).map_err(transport)
    }

    fn send_raw_binary(&mut self, bytes: &[u8]) -> Result<(), ClientError> {
        self.ws
            .send(Message::binary(bytes.to_vec()))
            .map_err(transport)
    }
}

// ---- WebSocket, on a background thread ----

#[derive(Clone)]
struct WsTx {
    tx: AsyncSender<Queued<String>>,
    outgoing: QueueBudget,
    incoming: QueueBudget,
}

impl TxHandle for WsTx {
    fn send(&self, req: Request) -> Result<(), ClientError> {
        self.try_send(req).map_err(SendError::client_error)
    }

    fn try_send(&self, req: Request) -> Result<(), SendError> {
        if self.incoming.stats().closed {
            return Err(SendError::Disconnected);
        }
        let text = req.to_text();
        let permit = self.outgoing.reserve(text.len())?;
        match self.tx.try_send(Queued {
            value: text,
            permit,
        }) {
            Ok(()) => Ok(()),
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                self.outgoing.reject();
                Err(SendError::Backpressure)
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                self.outgoing.close("the connection is closed");
                Err(SendError::Disconnected)
            }
        }
    }

    fn queue_stats(&self) -> Option<TransportQueueStats> {
        Some(TransportQueueStats {
            outgoing: self.outgoing.stats(),
            incoming: self.incoming.stats(),
        })
    }

    fn close(&self) {
        self.outgoing.close("the client closed the connection");
        self.incoming.close("the client closed the connection");
    }
}

struct WsCloseGuard {
    outgoing: QueueBudget,
    incoming: QueueBudget,
}

impl Drop for WsCloseGuard {
    fn drop(&mut self) {
        self.outgoing.close("the connection closed");
        self.incoming.close("the connection closed");
    }
}

/// A WebSocket owned by a background thread (one tokio current-thread
/// runtime). Sending queues the request; receiving reads what the thread
/// has collected. Dropping it closes the socket, even if a [`TxHandle`] survives.
pub struct PumpedWs {
    tx: WsTx,
    rx: IncomingReceiver,
}

impl PumpedWs {
    /// Connects and starts the thread (the URL may carry `?token=`).
    pub fn connect(url: &str, timeout: Duration) -> Result<Self, ClientError> {
        Self::connect_with_queue_limits(url, timeout, TransportQueueLimits::default())
    }

    /// Both queues retain at most their item and payload-byte limits. Refused
    /// outgoing requests never enter the socket worker; incoming refusal ends
    /// the connection. Serialization and one active socket write are outside
    /// retained queue accounting.
    pub fn connect_with_queue_limits(
        url: &str,
        timeout: Duration,
        limits: TransportQueueLimits,
    ) -> Result<Self, ClientError> {
        let (out_tx, mut out_rx) =
            async_channel::<Queued<String>>(limits.outgoing.max_items.max(1));
        let outgoing = QueueBudget::new(limits.outgoing);
        let (in_tx, in_rx) = incoming_channel(limits.incoming);
        let incoming = in_rx.budget.clone();
        let worker_outgoing = outgoing.clone();
        let worker_incoming = incoming.clone();
        let (ready_tx, ready_rx) = channel::<Result<(), String>>();
        let url = url.to_string();
        std::thread::Builder::new()
            .name("orr-erp-link".to_string())
            .spawn(move || {
                let _closed = WsCloseGuard { outgoing: worker_outgoing.clone(), incoming: worker_incoming.clone() };
                let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = ready_tx.send(Err(format!("runtime: {e}")));
                        return;
                    }
                };
                rt.block_on(async move {
                    let cfg = WebSocketConfig::default()
                        .max_message_size(Some(limits.incoming.max_bytes))
                        .max_frame_size(Some(limits.incoming.max_bytes));
                    let connected = tokio::select! {
                        biased;
                        _ = worker_outgoing.wait_closed() => return,
                        _ = worker_incoming.wait_closed() => return,
                        result = tokio_tungstenite::connect_async_with_config(url.as_str(), Some(cfg), true) => result,
                    };
                    let ws = match connected {
                        Ok((ws, _)) => ws,
                        Err(e) => {
                            let _ = ready_tx.send(Err(format!("cannot connect to {url}: {e}")));
                            return;
                        }
                    };
                    let _ = ready_tx.send(Ok(()));
                    let (mut sink, mut source) = ws.split();
                    loop {
                        if worker_outgoing.stats().closed || worker_incoming.stats().closed { return; }
                        tokio::select! {
                            _ = worker_outgoing.wait_closed() => return,
                            _ = worker_incoming.wait_closed() => return,
                            req = out_rx.recv() => {
                                let Some(req) = req else { return; };
                                let text = req.into_inner();
                                tokio::select! {
                                    biased;
                                    _ = worker_outgoing.wait_closed() => return,
                                    _ = worker_incoming.wait_closed() => return,
                                    sent = tokio::time::timeout(Duration::from_secs(30), sink.send(Message::Text(text.into()))) => {
                                        if !matches!(sent, Ok(Ok(()))) { return; }
                                    }
                                }
                            }
                            msg = source.next() => {
                                let incoming = match msg {
                                    Some(Ok(Message::Text(t))) => Incoming::Text(t.as_str().to_string()),
                                    Some(Ok(Message::Binary(b))) => Incoming::Wire(b.to_vec()),
                                    Some(Err(tokio_tungstenite::tungstenite::Error::Capacity(_))) => {
                                        worker_incoming.reject();
                                        worker_incoming.close("client receive payload exceeds byte limit");
                                        return;
                                    }
                                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                                    Some(Ok(_)) => continue,
                                };
                                let cost = match &incoming {
                                    Incoming::Text(s) => s.len(),
                                    Incoming::Wire(b) => b.len(),
                                    Incoming::Local(f) => f.cost,
                                };
                                if !in_tx.send(incoming, cost) {
                                    return;
                                }
                            }
                        }
                    }
                });
            })
            .map_err(transport)?;
        let ready = ready_rx.recv_timeout(timeout);
        if !matches!(ready, Ok(Ok(()))) {
            outgoing.close("the connection did not become ready");
            incoming.close("the connection did not become ready");
        }
        match ready {
            Ok(Ok(())) => Ok(Self {
                tx: WsTx {
                    tx: out_tx,
                    outgoing,
                    incoming,
                },
                rx: in_rx,
            }),
            Ok(Err(e)) => Err(ClientError::Transport(e)),
            Err(_) => Err(ClientError::Transport("timed out connecting".into())),
        }
    }
}

impl Transport for PumpedWs {
    fn send(&mut self, req: Request) -> Result<(), ClientError> {
        self.tx.send(req)
    }

    fn recv(&mut self, timeout: Duration) -> Result<Option<Incoming>, ClientError> {
        self.rx.recv(timeout)
    }

    fn sender(&self) -> Option<Arc<dyn TxHandle>> {
        Some(Arc::new(self.tx.clone()))
    }
}

impl Drop for PumpedWs {
    fn drop(&mut self) {
        self.tx.outgoing.close("the client dropped the connection");
        self.tx.incoming.close("the client dropped the connection");
    }
}

#[cfg(test)]
#[path = "link_tests.rs"]
mod tests;
