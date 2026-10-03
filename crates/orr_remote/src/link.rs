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
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use orr_ecs::Frame;
use serde_json::{json, Value as J};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use crate::caps::Caps;
use crate::client::ClientError;
use crate::net::{ConnTx, Inbound, NetShared, RequestPermit};

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
            Some(id) => json!({"jsonrpc": "2.0", "id": id, "method": self.method, "params": self.params}).to_string(),
            None => json!({"jsonrpc": "2.0", "method": self.method, "params": self.params}).to_string(),
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

    /// Sends raw text (for tests of malformed input). Sockets only.
    fn send_raw_text(&mut self, _text: &str) -> Result<(), ClientError> {
        Err(ClientError::Transport("this transport has no raw text".into()))
    }

    /// Sends raw bytes as a binary message. Sockets only.
    fn send_raw_binary(&mut self, _bytes: &[u8]) -> Result<(), ClientError> {
        Err(ClientError::Transport("this transport has no raw binary messages".into()))
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
        let conn = self.shared.next_id.fetch_add(1, Relaxed);
        let (tx, rx) = channel::<Incoming>();
        let pending = Arc::new(AtomicUsize::new(0));
        let to_client = ConnTx::local(tx, pending.clone());
        let hello = Inbound::Connected { conn, client: client.to_string(), caps, tx: to_client, binary: true };
        self.shared.inbox.send(hello).map_err(|_| transport("the host has stopped"))?;
        Ok(LocalTransport { tx: LocalTx { conn, shared: self.shared.clone() }, rx, pending })
    }
}

#[derive(Clone)]
struct LocalTx {
    conn: u64,
    shared: Arc<NetShared>,
}

impl TxHandle for LocalTx {
    fn send(&self, req: Request) -> Result<(), ClientError> {
        let permit = RequestPermit::reserve(&self.shared.queued, self.shared.max_queued)
            .ok_or_else(|| transport("the host has too many requests queued"))?;
        let id = req.id.map(|i| json!(i));
        let msg = Inbound::Request { permit, conn: self.conn, id, method: req.method, params: req.params };
        self.shared.inbox.send(msg).map_err(|_| transport("the host has stopped"))
    }
}

/// An in-process connection (see [`LocalConnector`]).
pub struct LocalTransport {
    tx: LocalTx,
    rx: Receiver<Incoming>,
    pending: Arc<AtomicUsize>,
}

impl Transport for LocalTransport {
    fn send(&mut self, req: Request) -> Result<(), ClientError> {
        self.tx.send(req)
    }

    fn recv(&mut self, timeout: Duration) -> Result<Option<Incoming>, ClientError> {
        match self.rx.recv_timeout(timeout) {
            Ok(m) => {
                match &m {
                    Incoming::Text(s) => self.pending.fetch_sub(s.len(), Relaxed),
                    Incoming::Local(f) => self.pending.fetch_sub(f.cost, Relaxed),
                    Incoming::Wire(b) => self.pending.fetch_sub(b.len(), Relaxed),
                };
                Ok(Some(m))
            }
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(transport("the host stopped")),
        }
    }

    fn sender(&self) -> Option<Arc<dyn TxHandle>> {
        Some(Arc::new(self.tx.clone()))
    }
}

impl Drop for LocalTransport {
    fn drop(&mut self) {
        let _ = self.tx.shared.inbox.send(Inbound::Disconnected { conn: self.tx.conn });
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
        Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted) => Ok(None),
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
        self.ws.send(Message::binary(bytes.to_vec())).map_err(transport)
    }
}

// ---- WebSocket, on a background thread ----

#[derive(Clone)]
struct WsTx(UnboundedSender<Request>);

impl TxHandle for WsTx {
    fn send(&self, req: Request) -> Result<(), ClientError> {
        self.0.send(req).map_err(|_| transport("the connection is closed"))
    }
}

/// A WebSocket owned by a background thread (one tokio current-thread
/// runtime). Sending queues the request; receiving reads what the thread
/// has collected. Dropping it (and every [`TxHandle`]) closes the socket.
pub struct PumpedWs {
    tx: WsTx,
    rx: Receiver<Incoming>,
}

impl PumpedWs {
    /// Connects and starts the thread (the URL may carry `?token=`).
    pub fn connect(url: &str, timeout: Duration) -> Result<Self, ClientError> {
        let (out_tx, mut out_rx) = unbounded_channel::<Request>();
        let (in_tx, in_rx) = channel::<Incoming>();
        let (ready_tx, ready_rx) = channel::<Result<(), String>>();
        let url = url.to_string();
        std::thread::Builder::new()
            .name("orr-erp-link".to_string())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = ready_tx.send(Err(format!("runtime: {e}")));
                        return;
                    }
                };
                rt.block_on(async move {
                    let cfg = WebSocketConfig::default().max_message_size(Some(1 << 30)).max_frame_size(Some(1 << 30));
                    let ws = match tokio_tungstenite::connect_async_with_config(url.as_str(), Some(cfg), true).await {
                        Ok((ws, _)) => ws,
                        Err(e) => {
                            let _ = ready_tx.send(Err(format!("cannot connect to {url}: {e}")));
                            return;
                        }
                    };
                    let _ = ready_tx.send(Ok(()));
                    let (mut sink, mut source) = ws.split();
                    loop {
                        tokio::select! {
                            req = out_rx.recv() => {
                                let Some(req) = req else {
                                    let _ = sink.close().await;
                                    return;
                                };
                                if sink.send(Message::Text(req.to_text().into())).await.is_err() {
                                    return;
                                }
                            }
                            msg = source.next() => {
                                let incoming = match msg {
                                    Some(Ok(Message::Text(t))) => Incoming::Text(t.as_str().to_string()),
                                    Some(Ok(Message::Binary(b))) => Incoming::Wire(b.to_vec()),
                                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                                    Some(Ok(_)) => continue,
                                };
                                if in_tx.send(incoming).is_err() {
                                    return;
                                }
                            }
                        }
                    }
                });
            })
            .map_err(transport)?;
        match ready_rx.recv_timeout(timeout) {
            Ok(Ok(())) => Ok(Self { tx: WsTx(out_tx), rx: in_rx }),
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
        match self.rx.recv_timeout(timeout) {
            Ok(m) => Ok(Some(m)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(transport("the connection closed")),
        }
    }

    fn sender(&self) -> Option<Arc<dyn TxHandle>> {
        Some(Arc::new(self.tx.clone()))
    }
}

#[cfg(test)]
#[path = "link_tests.rs"]
mod tests;
