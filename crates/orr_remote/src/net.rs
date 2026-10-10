//! The network side of the ERP server: accepts connections on a tokio
//! runtime, authenticates them, parses JSON-RPC and queues requests for the
//! host. It never touches the host's model.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value as J};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;

use crate::caps::{secret_eq, Auth, Caps};
use crate::error::*;
use crate::link::{Incoming, IncomingSender, LocalFrame, QueueBudget, QueuePermit, SendError};

/// How long a connection may take to finish the WebSocket handshake.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a connection may stay unauthenticated.
const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
/// A write that makes no progress this long closes the connection (a peer that stopped reading).
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
/// Bytes queued for one connection above which further messages to it are dropped, so a peer
/// that never reads cannot make the server's memory grow.
const MAX_CONN_QUEUE_BYTES: usize = 64 << 20;
/// Failed authentications (or calls before authenticating) before the connection is closed.
const MAX_AUTH_FAILURES: u32 = 5;

/// What the host thread gets from the network threads.
pub(crate) enum Inbound {
    Connected {
        conn: u64,
        client: String,
        caps: Caps,
        tx: ConnTx,
        binary: bool,
    },
    Disconnected {
        conn: u64,
    },
    /// An isolated verification is returning; reap only once its thread exits.
    VerificationReady {
        serial: u64,
    },
    /// A connection presented a token that was refused.
    AuthFailed,
    Request {
        permit: RequestPermit,
        conn: u64,
        id: Option<J>,
        method: String,
        params: J,
    },
}

pub(crate) enum Out {
    Text(String),
    Binary(Arc<Vec<u8>>),
    /// A negotiated-mode output admitted and charged as one FIFO item.
    Codec(CodecOut),
    Close,
}

/// Exact queue charge for the new mode. The charge lives with the queued
/// item, so channel refusal, receiver drop and dequeue all reclaim it once.
struct CodecQueueCharge {
    pending: Arc<AtomicUsize>,
    bytes: usize,
}

impl CodecQueueCharge {
    fn reserve(pending: &Arc<AtomicUsize>, bytes: usize, limit: usize) -> Option<Self> {
        if limit == 0 || limit > MAX_CONN_QUEUE_BYTES {
            return None;
        }
        let mut current = pending.load(Relaxed);
        loop {
            let next = current.checked_add(bytes).filter(|next| *next <= limit)?;
            match pending.compare_exchange_weak(current, next, Relaxed, Relaxed) {
                Ok(_) => {
                    return Some(Self {
                        pending: Arc::clone(pending),
                        bytes,
                    })
                }
                Err(observed) => current = observed,
            }
        }
    }
}

impl Drop for CodecQueueCharge {
    fn drop(&mut self) {
        self.pending.fetch_sub(self.bytes, Relaxed);
    }
}

/// At most one control/reset text and one frame, in that exact order. Neither
/// message can be interleaved with another queue item by the socket writer.
pub(crate) struct CodecOut {
    text: Option<String>,
    binary: Option<Arc<Vec<u8>>>,
    charge: CodecQueueCharge,
}

#[cfg(test)]
impl CodecOut {
    pub(crate) fn test_messages(self) -> (Option<String>, Option<Arc<Vec<u8>>>) {
        let Self {
            text,
            binary,
            charge,
        } = self;
        drop(charge);
        (text, binary)
    }
}

/// Where the messages of one connection go: to a socket writer task, or
/// straight into the channel of an in-process client.
#[derive(Clone)]
pub(crate) enum Sink {
    Net(UnboundedSender<Out>),
    Local(IncomingSender),
    // Existing server unit fixtures inject presentation loss through their
    // shared pending counter. Preserve that fixture contract; production
    // LocalConnector always constructs the bounded Local variant above.
    #[cfg(test)]
    TestLocal(std::sync::mpsc::Sender<Incoming>),
}

/// The way to write to one connection, from any thread.
#[derive(Clone)]
pub(crate) struct ConnTx {
    sink: Sink,
    /// Bytes queued and not yet written or taken (for dropping frames to slow readers).
    pending: Arc<AtomicUsize>,
}

impl ConnTx {
    #[cfg(test)]
    pub(crate) fn codec_test_channel() -> (
        Self,
        tokio::sync::mpsc::UnboundedReceiver<Out>,
        Arc<AtomicUsize>,
    ) {
        let (tx, rx) = unbounded_channel();
        let pending = Arc::new(AtomicUsize::new(0));
        (
            Self {
                sink: Sink::Net(tx),
                pending: pending.clone(),
            },
            rx,
            pending,
        )
    }
    /// The writing end of an in-process connection (its reading end is the client's).
    pub(crate) fn bounded_local(tx: IncomingSender) -> Self {
        Self {
            sink: Sink::Local(tx),
            pending: Arc::new(AtomicUsize::new(0)),
        }
    }

    #[cfg(test)]
    pub(crate) fn local(tx: std::sync::mpsc::Sender<Incoming>, pending: Arc<AtomicUsize>) -> Self {
        Self {
            sink: Sink::TestLocal(tx),
            pending,
        }
    }

    pub(crate) fn is_local(&self) -> bool {
        !matches!(self.sink, Sink::Net(_))
    }

    pub(crate) fn send_text(&self, s: String) {
        if let Sink::Local(tx) = &self.sink {
            let cost = s.len();
            tx.send(Incoming::Text(s), cost);
            return;
        }
        if self.pending.load(Relaxed) > MAX_CONN_QUEUE_BYTES {
            return;
        }
        self.send_control_text(s);
    }
    /// Admission-aware presentation delivery. A rejected notification is
    /// accounted for by the negotiated view stream's next snapshot fence.
    pub(crate) fn try_send_text(&self, s: String) -> bool {
        if let Sink::Local(tx) = &self.sink {
            let cost = s.len();
            return tx.send(Incoming::Text(s), cost);
        }
        if self.pending.load(Relaxed) > MAX_CONN_QUEUE_BYTES {
            return false;
        }
        self.send_control_text(s)
    }

    /// Network control responses bypass presentation backpressure. Local
    /// ingress uses one bounded queue; refusal terminates that connection.
    pub(crate) fn send_control_text(&self, s: String) -> bool {
        let cost = s.len();
        if let Sink::Local(tx) = &self.sink {
            return tx.send(Incoming::Text(s), cost);
        }
        self.pending.fetch_add(cost, Relaxed);
        let sent = match &self.sink {
            Sink::Net(tx) => tx.send(Out::Text(s)).is_ok(),
            Sink::Local(_) => unreachable!(),
            #[cfg(test)]
            Sink::TestLocal(tx) => tx.send(Incoming::Text(s)).is_ok(),
        };
        if !sent {
            self.pending.fetch_sub(cost, Relaxed);
        }
        sent
    }

    pub(crate) fn try_send_binary(&self, b: Arc<Vec<u8>>) -> bool {
        if self.pending.load(Relaxed) > MAX_CONN_QUEUE_BYTES {
            return false;
        }
        let Sink::Net(tx) = &self.sink else {
            return false;
        };
        let cost = b.len();
        self.pending.fetch_add(cost, Relaxed);
        let sent = tx.send(Out::Binary(b)).is_ok();
        if !sent {
            self.pending.fetch_sub(cost, Relaxed);
        }
        sent
    }

    /// Exact checked admission for an acknowledged codec control message.
    /// This does not alter the historical control-response bypass.
    pub(crate) fn try_send_codec_text(&self, text: String, queue_limit: usize) -> bool {
        self.try_send_codec_output(Some(text), None, queue_limit)
    }

    /// Exact checked admission for one negotiated frame record.
    pub(crate) fn try_send_codec_binary(&self, bytes: Arc<Vec<u8>>, queue_limit: usize) -> bool {
        self.try_send_codec_output(None, Some(bytes), queue_limit)
    }

    /// Atomically admit the reset announcement and its first Full record.
    pub(crate) fn try_send_codec_reset(
        &self,
        text: String,
        bytes: Arc<Vec<u8>>,
        queue_limit: usize,
    ) -> bool {
        self.try_send_codec_output(Some(text), Some(bytes), queue_limit)
    }

    fn try_send_codec_output(
        &self,
        text: Option<String>,
        binary: Option<Arc<Vec<u8>>>,
        queue_limit: usize,
    ) -> bool {
        let Sink::Net(tx) = &self.sink else {
            return false;
        };
        let Some(bytes) = text
            .as_ref()
            .map_or(0, String::len)
            .checked_add(binary.as_ref().map_or(0, |bytes| bytes.len()))
        else {
            return false;
        };
        let Some(charge) = CodecQueueCharge::reserve(&self.pending, bytes, queue_limit) else {
            return false;
        };
        tx.send(Out::Codec(CodecOut {
            text,
            binary,
            charge,
        }))
        .is_ok()
    }

    pub(crate) fn try_send_local_frame(&self, f: Arc<LocalFrame>) -> bool {
        #[cfg(test)]
        if let Sink::TestLocal(tx) = &self.sink {
            if self.pending.load(Relaxed) > MAX_CONN_QUEUE_BYTES {
                return false;
            }
            let cost = f.cost;
            self.pending.fetch_add(cost, Relaxed);
            let sent = tx.send(Incoming::Local(f)).is_ok();
            if !sent {
                self.pending.fetch_sub(cost, Relaxed);
            }
            return sent;
        }
        let Sink::Local(tx) = &self.sink else {
            return false;
        };
        let cost = f.cost;
        tx.send(Incoming::Local(f), cost)
    }
    pub(crate) fn send_binary(&self, b: Arc<Vec<u8>>) {
        if self.pending.load(Relaxed) > MAX_CONN_QUEUE_BYTES {
            return;
        }
        if let Sink::Net(tx) = &self.sink {
            let cost = b.len();
            self.pending.fetch_add(cost, Relaxed);
            if tx.send(Out::Binary(b)).is_err() {
                self.pending.fetch_sub(cost, Relaxed);
            }
        }
    }
    /// A view stream message: a binary message on a socket, an `Incoming::Wire` in process.
    pub(crate) fn send_stream(&self, b: Arc<Vec<u8>>) {
        if let Sink::Local(tx) = &self.sink {
            let cost = b.len();
            tx.send(Incoming::Wire(b.to_vec()), cost);
            return;
        }
        if self.pending.load(Relaxed) > MAX_CONN_QUEUE_BYTES {
            return;
        }
        let cost = b.len();
        self.pending.fetch_add(cost, Relaxed);
        match &self.sink {
            Sink::Net(tx) => {
                if tx.send(Out::Binary(b)).is_err() {
                    self.pending.fetch_sub(cost, Relaxed);
                }
            }
            Sink::Local(_) => unreachable!(),
            #[cfg(test)]
            Sink::TestLocal(tx) => {
                if tx.send(Incoming::Wire(b.to_vec())).is_err() {
                    self.pending.fetch_sub(cost, Relaxed);
                }
            }
        }
    }
    /// A frame for an in-process client: shared, never serialized.
    pub(crate) fn send_local_frame(&self, f: Arc<LocalFrame>) {
        #[cfg(test)]
        if let Sink::TestLocal(tx) = &self.sink {
            let cost = f.cost;
            self.pending.fetch_add(cost, Relaxed);
            if tx.send(Incoming::Local(f)).is_err() {
                self.pending.fetch_sub(cost, Relaxed);
            }
            return;
        }
        if let Sink::Local(tx) = &self.sink {
            let cost = f.cost;
            tx.send(Incoming::Local(f), cost);
        }
    }
    pub(crate) fn pending(&self) -> usize {
        match &self.sink {
            Sink::Net(_) => self.pending.load(Relaxed),
            Sink::Local(tx) => tx.pending(),
            #[cfg(test)]
            Sink::TestLocal(_) => self.pending.load(Relaxed),
        }
    }
    pub(crate) fn close(&self) {
        match &self.sink {
            Sink::Net(tx) => {
                let _ = tx.send(Out::Close);
            }
            Sink::Local(tx) => tx.close(),
            #[cfg(test)]
            Sink::TestLocal(_) => (),
        }
    }
    async fn closed(&self) {
        match &self.sink {
            Sink::Net(tx) => tx.closed().await,
            Sink::Local(tx) => tx.closed().await,
            #[cfg(test)]
            Sink::TestLocal(_) => std::future::pending::<()>().await,
        }
    }
}

pub(crate) struct NetShared {
    pub auth: Auth,
    pub max_message_bytes: usize,
    pub allowed_origins: Vec<String>,
    pub max_connections: usize,
    pub max_queued: usize,
    pub max_queued_bytes: usize,
    pub inbox: Sender<Inbound>,
    pub next_id: AtomicU64,
    pub conns: AtomicUsize,
    /// Reserved requests waiting in producers, the host inbox or its stash.
    pub queued: Arc<AtomicUsize>,
    pub queued_bytes: Arc<AtomicUsize>,
}

/// One waiting request's count and canonical bytes. Never clone this permit or
/// retain NetShared: its inbox sender would create an ownership cycle.
pub(crate) struct RequestPermit {
    queued: Arc<AtomicUsize>,
    queued_bytes: Arc<AtomicUsize>,
    bytes: usize,
    client: Option<QueuePermit>,
}

impl RequestPermit {
    pub(crate) fn reserve(
        shared: &NetShared,
        id: Option<&J>,
        method: &str,
        params: &J,
    ) -> Option<Self> {
        reserve_counter(&shared.queued, shared.max_queued, 1)?;
        // Own the count immediately: serialization failure and byte rejection
        // both roll it back through the same drop path as failed delivery.
        let mut permit = Self {
            queued: shared.queued.clone(),
            queued_bytes: shared.queued_bytes.clone(),
            bytes: 0,
            client: None,
        };
        let bytes = request_bytes(id, method, params, shared.max_queued_bytes)?;
        reserve_counter(&shared.queued_bytes, shared.max_queued_bytes, bytes)?;
        permit.bytes = bytes;
        Some(permit)
    }

    /// The same waiting request owns both its global host and per-client
    /// quotas. The host already drops this permit on dequeue or failed delivery.
    pub(crate) fn with_client_budget(mut self, budget: &QueueBudget) -> Result<Self, SendError> {
        self.client = Some(budget.reserve(self.bytes)?);
        Ok(self)
    }
}

fn reserve_counter(counter: &AtomicUsize, limit: usize, amount: usize) -> Option<()> {
    // Relaxed is enough for accounting; the channel publishes request data.
    let mut current = counter.load(Relaxed);
    loop {
        let next = current.checked_add(amount).filter(|next| *next <= limit)?;
        match counter.compare_exchange_weak(current, next, Relaxed, Relaxed) {
            Ok(_) => return Some(()),
            Err(observed) => current = observed,
        }
    }
}

impl Drop for RequestPermit {
    fn drop(&mut self) {
        self.queued_bytes.fetch_sub(self.bytes, Relaxed);
        self.queued.fetch_sub(1, Relaxed);
    }
}

/// Count compact JSON-RPC envelope bytes without cloning params or allocating
/// serialized text. Network whitespace/extra fields are intentionally ignored;
/// absent params normalize to null and absent id stays absent on both paths.
fn request_bytes(id: Option<&J>, method: &str, params: &J, limit: usize) -> Option<usize> {
    use std::io::Write;
    struct Counter {
        bytes: usize,
        limit: usize,
    }
    impl Write for Counter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.bytes = self
                .bytes
                .checked_add(buf.len())
                .filter(|n| *n <= self.limit)
                .ok_or_else(|| std::io::Error::other("request byte limit exceeded"))?;
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut out = Counter { bytes: 0, limit };
    out.write_all(br#"{"jsonrpc":"2.0""#).ok()?;
    if let Some(id) = id {
        out.write_all(br#","id":"#).ok()?;
        serde_json::to_writer(&mut out, id).ok()?;
    }
    out.write_all(br#","method":"#).ok()?;
    serde_json::to_writer(&mut out, method).ok()?;
    out.write_all(br#","params":"#).ok()?;
    serde_json::to_writer(&mut out, params).ok()?;
    out.write_all(b"}").ok()?;
    Some(out.bytes)
}

pub(crate) fn response_ok(id: &J, result: J) -> String {
    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
}

pub(crate) fn response_err(id: &J, e: &RpcError) -> String {
    json!({"jsonrpc": "2.0", "id": id, "error": e.to_json()}).to_string()
}

pub(crate) fn notification(method: &str, params: J) -> String {
    json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string()
}

/// The state of one connection on the network side.
struct Link {
    id: u64,
    shared: Arc<NetShared>,
    out: ConnTx,
    binary: bool,
    identity: Option<(String, Caps)>,
    failures: u32,
    announced: bool,
}

enum Flow {
    Continue,
    Close,
}

impl Link {
    fn who(&self) -> J {
        match &self.identity {
            Some((c, caps)) => {
                json!({"client": c, "capabilities": caps.list().into_iter().map(|c| c.name()).collect::<Vec<_>>()})
            }
            None => J::Null,
        }
    }

    fn set_identity(&mut self, client: String, caps: Caps) {
        self.identity = Some((client.clone(), caps));
        self.announced = true;
        let _ = self.shared.inbox.send(Inbound::Connected {
            conn: self.id,
            client,
            caps,
            tx: self.out.clone(),
            binary: self.binary,
        });
    }

    fn reply_err(&self, id: &J, e: RpcError) {
        self.out.send_control_text(response_err(id, &e));
    }

    fn finish(&self) {
        if self.announced {
            let _ = self
                .shared
                .inbox
                .send(Inbound::Disconnected { conn: self.id });
        }
        self.out.close();
    }

    /// Handles one text message (a JSON-RPC request).
    fn on_text(&mut self, text: &str) -> Flow {
        let null = J::Null;
        if text.len() > self.shared.max_message_bytes {
            self.reply_err(
                &null,
                RpcError::new(
                    INVALID_REQUEST,
                    "too_large",
                    format!(
                        "message of {} bytes is over the limit of {} bytes",
                        text.len(),
                        self.shared.max_message_bytes
                    ),
                ),
            );
            return Flow::Continue;
        }
        let parsed: J = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => {
                self.reply_err(
                    &null,
                    RpcError::new(PARSE_ERROR, "parse_error", format!("invalid JSON: {e}")),
                );
                return Flow::Continue;
            }
        };
        let J::Object(mut obj) = parsed else {
            let msg = if parsed.is_array() {
                "batch requests are not supported"
            } else {
                "a request must be a JSON object"
            };
            self.reply_err(
                &null,
                RpcError::new(INVALID_REQUEST, "invalid_request", msg),
            );
            return Flow::Continue;
        };
        let id = obj.get("id").cloned();
        let reply_id = id.clone().unwrap_or(J::Null);
        if !matches!(id, None | Some(J::Null | J::Number(_) | J::String(_))) {
            self.reply_err(
                &null,
                RpcError::new(
                    INVALID_REQUEST,
                    "invalid_request",
                    "'id' must be a string, a number or null",
                ),
            );
            return Flow::Continue;
        }
        if obj.get("jsonrpc").and_then(J::as_str) != Some("2.0") {
            self.reply_err(
                &reply_id,
                RpcError::new(
                    INVALID_REQUEST,
                    "invalid_request",
                    "'jsonrpc' must be \"2.0\"",
                ),
            );
            return Flow::Continue;
        }
        let Some(method) = obj.get("method").and_then(J::as_str).map(str::to_string) else {
            self.reply_err(
                &reply_id,
                RpcError::new(
                    INVALID_REQUEST,
                    "invalid_request",
                    "'method' must be a string",
                ),
            );
            return Flow::Continue;
        };
        let params = obj.remove("params").unwrap_or(J::Null);
        if !matches!(params, J::Null | J::Object(_)) {
            self.reply_err(&reply_id, RpcError::params("params must be an object"));
            return Flow::Continue;
        }

        if method == "auth" {
            return self.on_auth(&reply_id, id.is_some(), &params);
        }
        if self.identity.is_none() {
            self.failures += 1;
            self.reply_err(
                &reply_id,
                RpcError::new(
                    UNAUTHENTICATED,
                    "unauthenticated",
                    "authenticate first: {\"method\":\"auth\",\"params\":{\"token\":\"...\"}}",
                ),
            );
            return if self.failures >= MAX_AUTH_FAILURES {
                Flow::Close
            } else {
                Flow::Continue
            };
        }
        let Some(permit) = RequestPermit::reserve(&self.shared, id.as_ref(), &method, &params)
        else {
            self.reply_err(
                &reply_id,
                RpcError::new(
                    LIMIT_EXCEEDED,
                    "busy",
                    "the host has too many requests queued; retry",
                ),
            );
            return Flow::Continue;
        };
        if self
            .shared
            .inbox
            .send(Inbound::Request {
                permit,
                conn: self.id,
                id,
                method,
                params,
            })
            .is_err()
        {
            return Flow::Close;
        }
        Flow::Continue
    }

    fn on_auth(&mut self, reply_id: &J, wants_reply: bool, params: &J) -> Flow {
        if self.identity.is_some() {
            if wants_reply {
                self.out
                    .send_control_text(response_ok(reply_id, self.who()));
            }
            return Flow::Continue;
        }
        let token = params.get("token").and_then(J::as_str).unwrap_or("");
        let found = match &self.shared.auth {
            Auth::Tokens(list) => list
                .iter()
                .find(|t| secret_eq(&t.token, token))
                .map(|t| (t.client.clone(), t.caps)),
            Auth::DevNoAuth => Some(("dev".to_string(), Caps::ALL)),
        };
        match found {
            Some((client, caps)) => {
                self.set_identity(client, caps);
                if wants_reply {
                    self.out
                        .send_control_text(response_ok(reply_id, self.who()));
                }
                Flow::Continue
            }
            None => {
                self.failures += 1;
                let _ = self.shared.inbox.send(Inbound::AuthFailed);
                self.reply_err(
                    reply_id,
                    RpcError::new(UNAUTHENTICATED, "bad_token", "the token was refused"),
                );
                if self.failures >= MAX_AUTH_FAILURES {
                    Flow::Close
                } else {
                    Flow::Continue
                }
            }
        }
    }
}

pub(crate) async fn accept_loop(listener: TcpListener, shared: Arc<NetShared>) {
    loop {
        let Ok((stream, _peer)) = listener.accept().await else {
            tokio::time::sleep(Duration::from_millis(10)).await;
            continue;
        };
        if shared.conns.load(Relaxed) >= shared.max_connections {
            continue; // dropping the socket refuses the connection
        }
        shared.conns.fetch_add(1, Relaxed);
        let shared = shared.clone();
        tokio::spawn(async move {
            serve(shared.clone(), stream).await;
            shared.conns.fetch_sub(1, Relaxed);
        });
    }
}

async fn serve(shared: Arc<NetShared>, stream: TcpStream) {
    let _ = stream.set_nodelay(true);
    let mut first = [0u8; 1];
    match timeout(HANDSHAKE_TIMEOUT, stream.peek(&mut first)).await {
        Ok(Ok(1)) => {}
        _ => return,
    }
    // A WebSocket upgrade starts with `GET`; anything else is newline-delimited JSON.
    if first[0] == b'G' {
        ws_conn(shared, stream).await;
    } else {
        ndjson_conn(shared, stream).await;
    }
}

fn new_link(
    shared: &Arc<NetShared>,
    binary: bool,
) -> (
    Link,
    tokio::sync::mpsc::UnboundedReceiver<Out>,
    Arc<AtomicUsize>,
) {
    let (tx, rx) = unbounded_channel();
    let pending = Arc::new(AtomicUsize::new(0));
    let link = Link {
        id: shared.next_id.fetch_add(1, Relaxed),
        shared: shared.clone(),
        out: ConnTx {
            sink: Sink::Net(tx),
            pending: pending.clone(),
        },
        binary,
        identity: None,
        failures: 0,
        announced: false,
    };
    (link, rx, pending)
}

/// The value of `key` in a URL query, percent-decoded.
fn query_param(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k == key {
            return Some(percent_decode(v));
        }
    }
    None
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(v) = hex {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A client name a dev-mode client may give: short, printable, no spaces.
fn valid_client_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 32
        && n.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._".contains(&b))
}

fn refuse(status: http::StatusCode, why: &str) -> ErrorResponse {
    let mut e = ErrorResponse::new(Some(why.to_string()));
    *e.status_mut() = status;
    e
}

#[allow(clippy::result_large_err)] // the error type is fixed by the tungstenite callback trait
async fn ws_conn(shared: Arc<NetShared>, stream: TcpStream) {
    let hard = shared.max_message_bytes.saturating_mul(4).max(1 << 16);
    let cfg = WebSocketConfig::default()
        .max_message_size(Some(hard))
        .max_frame_size(Some(hard));
    let mut url_identity: Option<(String, Caps)> = None;
    let mut dev_client: Option<String> = None;
    let check = |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
        // A browser page must not be able to drive a local server: browsers
        // always send `Origin`, tools do not.
        if let Some(origin) = req.headers().get("origin").and_then(|v| v.to_str().ok()) {
            if !shared.allowed_origins.iter().any(|o| o == origin) {
                return Err(refuse(http::StatusCode::FORBIDDEN, "origin not allowed"));
            }
        }
        if matches!(shared.auth, Auth::DevNoAuth) {
            // Dev mode has no tokens, so a client may say who it is: `?client=user` is a person's view.
            dev_client = req
                .uri()
                .query()
                .and_then(|q| query_param(q, "client"))
                .filter(|n| valid_client_name(n));
        }
        if let Auth::Tokens(list) = &shared.auth {
            if let Some(token) = req.uri().query().and_then(|q| query_param(q, "token")) {
                match list.iter().find(|t| secret_eq(&t.token, &token)) {
                    Some(t) => url_identity = Some((t.client.clone(), t.caps)),
                    None => {
                        return Err(refuse(
                            http::StatusCode::UNAUTHORIZED,
                            "the token was refused",
                        ))
                    }
                }
            }
        }
        Ok(resp)
    };
    let ws = timeout(
        HANDSHAKE_TIMEOUT,
        tokio_tungstenite::accept_hdr_async_with_config(stream, check, Some(cfg)),
    )
    .await;
    let Ok(Ok(ws)) = ws else { return };
    let (mut sink, mut source) = ws.split();
    let (mut link, mut rx, pending) = new_link(&shared, true);
    match (&shared.auth, url_identity) {
        (Auth::DevNoAuth, _) => {
            link.set_identity(dev_client.unwrap_or_else(|| "dev".to_string()), Caps::ALL)
        }
        (_, Some((client, caps))) => link.set_identity(client, caps),
        _ => {}
    }

    let writer = tokio::spawn(async move {
        while let Some(out) = rx.recv().await {
            let msg = match out {
                Out::Text(s) => {
                    pending.fetch_sub(s.len(), Relaxed);
                    Message::Text(s.into())
                }
                Out::Binary(b) => {
                    pending.fetch_sub(b.len(), Relaxed);
                    Message::Binary(b.as_slice().to_vec().into())
                }
                Out::Codec(out) => {
                    let CodecOut {
                        text,
                        binary,
                        charge,
                    } = out;
                    // Queue admission counts queued items, not the writer's
                    // currently owned socket message (the legacy boundary).
                    drop(charge);
                    if let Some(text) = text {
                        match timeout(WRITE_TIMEOUT, sink.send(Message::Text(text.into()))).await {
                            Ok(Ok(())) => {}
                            _ => return,
                        }
                    }
                    if let Some(bytes) = binary {
                        match timeout(
                            WRITE_TIMEOUT,
                            sink.send(Message::Binary(bytes.as_slice().to_vec().into())),
                        )
                        .await
                        {
                            Ok(Ok(())) => {}
                            _ => return,
                        }
                    }
                    continue;
                }
                Out::Close => break,
            };
            match timeout(WRITE_TIMEOUT, sink.send(msg)).await {
                Ok(Ok(())) => {}
                _ => return, // the receiver is dropped, which ends the connection
            }
        }
        let _ = sink.close().await;
    });

    let deadline = tokio::time::sleep(AUTH_TIMEOUT);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            msg = source.next() => match msg {
                Some(Ok(Message::Text(t))) => {
                    if matches!(link.on_text(t.as_str()), Flow::Close) {
                        break;
                    }
                }
                Some(Ok(Message::Binary(_))) => {
                    link.reply_err(&J::Null, RpcError::new(INVALID_REQUEST, "invalid_request", "send JSON-RPC requests as text messages"));
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
            () = &mut deadline, if link.identity.is_none() => break,
            () = link.out.closed() => break,
        }
    }
    link.finish();
    let _ = timeout(Duration::from_secs(2), writer).await;
}

async fn ndjson_conn(shared: Arc<NetShared>, stream: TcpStream) {
    let hard = shared.max_message_bytes.saturating_mul(4).max(1 << 16);
    let (rd, mut wr) = stream.into_split();
    let mut rd = BufReader::new(rd);
    let (mut link, mut rx, pending) = new_link(&shared, false);
    if matches!(shared.auth, Auth::DevNoAuth) {
        link.set_identity("dev".to_string(), Caps::ALL);
    }
    let writer = tokio::spawn(async move {
        while let Some(out) = rx.recv().await {
            match out {
                Out::Text(mut s) => {
                    pending.fetch_sub(s.len(), Relaxed);
                    s.push('\n');
                    match timeout(WRITE_TIMEOUT, wr.write_all(s.as_bytes())).await {
                        Ok(Ok(())) => {}
                        _ => return,
                    }
                }
                Out::Binary(b) => {
                    pending.fetch_sub(b.len(), Relaxed);
                }
                // Negotiated Frame records are WebSocket-only. A routing
                // violation closes this writer and drops/reclaims the batch.
                Out::Codec(_) => return,
                Out::Close => break,
            }
        }
        let _ = wr.shutdown().await;
    });

    let deadline = tokio::time::sleep(AUTH_TIMEOUT);
    tokio::pin!(deadline);
    let mut line: Vec<u8> = Vec::new();
    loop {
        line.clear();
        let read = async {
            let mut limited = (&mut rd).take(hard as u64 + 1);
            limited.read_until(b'\n', &mut line).await
        };
        let n = tokio::select! {
            n = read => n,
            () = &mut deadline, if link.identity.is_none() => break,
            () = link.out.closed() => break,
        };
        match n {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if line.len() > hard {
            link.reply_err(
                &J::Null,
                RpcError::new(
                    INVALID_REQUEST,
                    "too_large",
                    "line is over the limit; closing",
                ),
            );
            break;
        }
        let Ok(text) = std::str::from_utf8(&line) else {
            link.reply_err(
                &J::Null,
                RpcError::new(PARSE_ERROR, "parse_error", "the line is not UTF-8"),
            );
            continue;
        };
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        if matches!(link.on_text(text), Flow::Close) {
            break;
        }
    }
    link.finish();
    let _ = timeout(Duration::from_secs(2), writer).await;
}

/// Binds the listener (blocking the caller until it is bound).
pub(crate) fn bind(rt: &tokio::runtime::Runtime, addr: SocketAddr) -> std::io::Result<TcpListener> {
    rt.block_on(TcpListener::bind(addr))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue_fixture(limit: usize) -> (Arc<NetShared>, std::sync::mpsc::Receiver<Inbound>) {
        let (inbox, recv) = std::sync::mpsc::channel();
        (
            Arc::new(NetShared {
                auth: Auth::DevNoAuth,
                max_message_bytes: 4 << 20,
                allowed_origins: Vec::new(),
                max_connections: 64,
                max_queued: limit,
                max_queued_bytes: usize::MAX,
                inbox,
                next_id: AtomicU64::new(1),
                conns: AtomicUsize::new(0),
                queued: Arc::new(AtomicUsize::new(0)),
                queued_bytes: Arc::new(AtomicUsize::new(0)),
            }),
            recv,
        )
    }

    #[test]
    fn request_permit_boundaries_and_release() {
        let (mut shared, _recv) = queue_fixture(1);
        let cost = request_bytes(None, "sim.state", &J::Null, usize::MAX).unwrap();
        for limit in [0, cost - 1, cost, usize::MAX] {
            Arc::get_mut(&mut shared).unwrap().max_queued_bytes = limit;
            let permit = RequestPermit::reserve(&shared, None, "sim.state", &J::Null);
            assert_eq!(permit.is_some(), limit >= cost);
            assert_eq!(shared.queued.load(Relaxed), usize::from(permit.is_some()));
            assert_eq!(
                shared.queued_bytes.load(Relaxed),
                if permit.is_some() { cost } else { 0 }
            );
            if permit.is_some() {
                assert!(RequestPermit::reserve(&shared, None, "sim.state", &J::Null).is_none());
            }
            drop(permit);
            assert_eq!(shared.queued.load(Relaxed), 0);
            assert_eq!(shared.queued_bytes.load(Relaxed), 0);
        }
        // Checked count and byte arithmetic, without enormous allocations.
        Arc::get_mut(&mut shared).unwrap().max_queued = usize::MAX;
        shared.queued.store(usize::MAX - 1, Relaxed);
        shared.queued_bytes.store(usize::MAX - cost, Relaxed);
        let permit = RequestPermit::reserve(&shared, None, "sim.state", &J::Null).unwrap();
        assert_eq!(shared.queued.load(Relaxed), usize::MAX);
        assert_eq!(shared.queued_bytes.load(Relaxed), usize::MAX);
        assert!(RequestPermit::reserve(&shared, None, "sim.state", &J::Null).is_none());
        drop(permit);
        shared.queued.store(0, Relaxed);
        shared.queued_bytes.store(usize::MAX - cost + 1, Relaxed);
        assert!(RequestPermit::reserve(&shared, None, "sim.state", &J::Null).is_none());
        assert_eq!(
            shared.queued.load(Relaxed),
            0,
            "byte overflow restores count"
        );
        assert_eq!(shared.queued_bytes.load(Relaxed), usize::MAX - cost + 1);
    }

    #[test]
    fn canonical_bytes_match_local_and_network_admission() {
        use crate::link::{LocalConnector, Request, Transport};
        for id in [None, Some(0), Some(u64::MAX)] {
            let request = Request {
                id,
                method: "echo\"\n한글".into(),
                params: json!({"text":"雪\"\n", "nested":[null,true,{"number":18446744073709551615u64}]}),
            };
            let cost = request.to_text().len();
            assert_eq!(
                request_bytes(
                    id.map(J::from).as_ref(),
                    &request.method,
                    &request.params,
                    usize::MAX
                ),
                Some(cost)
            );
            for limit in [cost - 1, cost] {
                let (mut shared, recv) = queue_fixture(10);
                Arc::get_mut(&mut shared).unwrap().max_queued_bytes = limit;
                let mut local = LocalConnector {
                    shared: shared.clone(),
                }
                .connect("local", Caps::ALL)
                .unwrap();
                let (mut link, mut out, _pending) = new_link(&shared, false);
                link.set_identity("net".into(), Caps::ALL);
                assert_eq!(local.send(request.clone()).is_ok(), limit == cost);
                assert_eq!(
                    shared.queued_bytes.load(Relaxed),
                    if limit == cost { cost } else { 0 }
                );
                // Release local request, then exercise the same byte budget over
                // the network with whitespace and ignored extra fields.
                drop(recv.try_iter().collect::<Vec<_>>());
                let mut wire: J = serde_json::from_str(&request.to_text()).unwrap();
                wire["ignored"] = json!("not forwarded");
                assert!(matches!(
                    link.on_text(&serde_json::to_string_pretty(&wire).unwrap()),
                    Flow::Continue
                ));
                assert_eq!(
                    shared.queued_bytes.load(Relaxed),
                    if limit == cost { cost } else { 0 }
                );
                if limit < cost {
                    let Out::Text(reply) = out.try_recv().unwrap() else {
                        panic!("busy response")
                    };
                    assert_eq!(
                        serde_json::from_str::<J>(&reply).unwrap()["error"]["data"]["kind"],
                        "busy"
                    );
                }
                drop(recv);
                assert_eq!(shared.queued.load(Relaxed), 0);
                assert_eq!(shared.queued_bytes.load(Relaxed), 0);
            }
        }
        // Network IDs additionally support null, strings and exact JSON numbers.
        for id in [
            J::Null,
            json!("雪\"\n"),
            serde_json::from_str("123456789012345678901234567890.125").unwrap(),
        ] {
            let wire =
                json!({"jsonrpc":"2.0", "id":id, "method":"sim.state", "params":null}).to_string();
            let cost = wire.len();
            for limit in [cost - 1, cost] {
                let (mut shared, recv) = queue_fixture(10);
                Arc::get_mut(&mut shared).unwrap().max_queued_bytes = limit;
                let (mut link, mut out, _pending) = new_link(&shared, false);
                link.set_identity("net".into(), Caps::ALL);
                assert!(matches!(link.on_text(&wire), Flow::Continue));
                assert_eq!(shared.queued.load(Relaxed), usize::from(limit == cost));
                assert_eq!(
                    shared.queued_bytes.load(Relaxed),
                    if limit == cost { cost } else { 0 }
                );
                if limit < cost {
                    let Out::Text(reply) = out.try_recv().unwrap() else {
                        panic!("busy response")
                    };
                    let reply: J = serde_json::from_str(&reply).unwrap();
                    assert_eq!(reply["id"], id);
                    assert_eq!(reply["error"]["data"]["kind"], "busy");
                }
                drop(recv);
                assert_eq!(shared.queued.load(Relaxed), 0);
                assert_eq!(shared.queued_bytes.load(Relaxed), 0);
            }
        }
        // Missing params and explicit null have the same canonical cost.
        let (mut shared, recv) = queue_fixture(10);
        let cost = request_bytes(Some(&json!(1)), "sim.state", &J::Null, usize::MAX).unwrap();
        Arc::get_mut(&mut shared).unwrap().max_queued_bytes = cost;
        let (mut link, _out, _pending) = new_link(&shared, false);
        link.set_identity("net".into(), Caps::ALL);
        link.on_text(r#"{"jsonrpc":"2.0","id":1,"method":"sim.state"}"#);
        assert_eq!(shared.queued_bytes.load(Relaxed), cost);
        drop(recv);
        assert_eq!(shared.queued_bytes.load(Relaxed), 0);
    }

    #[test]
    fn local_and_network_failed_delivery_release_their_permits() {
        use crate::link::{LocalConnector, Transport};
        let (shared, recv) = queue_fixture(1);
        let mut local = LocalConnector {
            shared: shared.clone(),
        }
        .connect("local", Caps::ALL)
        .unwrap();
        let (mut link, _out, _pending) = new_link(&shared, false);
        link.set_identity("net".into(), Caps::ALL);
        drop(recv);
        let request = crate::link::Request {
            id: Some(1),
            method: "sim.state".into(),
            params: J::Null,
        };
        for _ in 0..3 {
            assert!(local
                .send(request.clone())
                .unwrap_err()
                .to_string()
                .contains("stopped"));
            assert_eq!(shared.queued.load(Relaxed), 0);
            assert_eq!(shared.queued_bytes.load(Relaxed), 0);
            assert!(matches!(link.on_text(&request.to_text()), Flow::Close));
            assert_eq!(shared.queued.load(Relaxed), 0);
            assert_eq!(shared.queued_bytes.load(Relaxed), 0);
        }
    }

    #[test]
    fn concurrent_local_and_network_requests_share_one_cap() {
        use crate::link::{LocalConnector, Transport};
        const LIMIT: usize = 7;
        const PRODUCERS: usize = 16;
        // Exercise contention with valid requests, without claiming to force the
        // old check/increment race's particular scheduling window.
        for byte_limited in [false, true] {
            for _ in 0..16 {
                let (mut shared, recv) =
                    queue_fixture(if byte_limited { PRODUCERS } else { LIMIT });
                let cost =
                    request_bytes(Some(&json!(1)), "sim.state", &J::Null, usize::MAX).unwrap();
                Arc::get_mut(&mut shared).unwrap().max_queued_bytes = if byte_limited {
                    LIMIT * cost
                } else {
                    usize::MAX
                };
                let barrier = Arc::new(std::sync::Barrier::new(PRODUCERS));
                std::thread::scope(|scope| {
                    let mut workers = Vec::new();
                    for producer in 0..PRODUCERS {
                        let shared = shared.clone();
                        let barrier = barrier.clone();
                        workers.push(scope.spawn(move || {
                            let request = crate::link::Request {
                                id: Some(1),
                                method: "sim.state".into(),
                                params: J::Null,
                            };
                            if producer % 2 == 0 {
                                let mut local = LocalConnector {
                                    shared: shared.clone(),
                                }
                                .connect("local", Caps::ALL)
                                .unwrap();
                                barrier.wait();
                                usize::from(local.send(request).is_ok())
                            } else {
                                let (mut link, mut out, _pending) = new_link(&shared, false);
                                link.set_identity("net".into(), Caps::ALL);
                                barrier.wait();
                                assert!(matches!(link.on_text(&request.to_text()), Flow::Continue));
                                match out.try_recv() {
                                    Ok(Out::Text(text)) => {
                                        let reply: J = serde_json::from_str(&text).unwrap();
                                        assert_eq!(reply["id"], 1);
                                        assert_eq!(reply["error"]["data"]["kind"], "busy");
                                        0
                                    }
                                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => 1,
                                    _ => panic!("unexpected output"),
                                }
                            }
                        }));
                    }
                    let accepted: usize = workers.into_iter().map(|w| w.join().unwrap()).sum();
                    assert_eq!(accepted, LIMIT);
                });
                assert_eq!(shared.queued.load(Relaxed), LIMIT);
                assert_eq!(shared.queued_bytes.load(Relaxed), LIMIT * cost);
                let mut requests = Vec::new();
                for msg in recv.try_iter() {
                    if matches!(msg, Inbound::Request { .. }) {
                        requests.push(msg);
                    }
                }
                assert_eq!(requests.len(), LIMIT);
                assert_eq!(
                    shared.queued.load(Relaxed),
                    LIMIT,
                    "taking a message retains its slot"
                );
                drop(requests);
                assert_eq!(shared.queued.load(Relaxed), 0);
                assert_eq!(shared.queued_bytes.load(Relaxed), 0);
            }
        }
    }

    #[test]
    fn zero_capacity_rejects_requests_but_keeps_control_events() {
        use crate::link::{LocalConnector, Transport};
        for byte_limited in [false, true] {
            let (mut shared, recv) = queue_fixture(if byte_limited { 10 } else { 0 });
            if byte_limited {
                Arc::get_mut(&mut shared).unwrap().max_queued_bytes = 0;
            }
            let mut local = LocalConnector {
                shared: shared.clone(),
            }
            .connect("local", Caps::ALL)
            .unwrap();
            let (mut link, mut out, _pending) = new_link(&shared, false);
            assert!(matches!(
                link.on_text(r#"{"jsonrpc":"2.0","id":1,"method":"auth"}"#),
                Flow::Continue
            ));
            assert!(matches!(out.try_recv().unwrap(), Out::Text(_)));
            let request = crate::link::Request {
                id: Some(2),
                method: "sim.state".into(),
                params: J::Null,
            };
            assert!(local.send(request.clone()).is_err());
            assert!(matches!(link.on_text(&request.to_text()), Flow::Continue));
            let Out::Text(text) = out.try_recv().unwrap() else {
                panic!("busy response")
            };
            assert_eq!(
                serde_json::from_str::<J>(&text).unwrap()["error"]["data"]["kind"],
                "busy"
            );
            drop(local);
            link.finish();
            let events: Vec<_> = recv.try_iter().collect();
            assert_eq!(events.len(), 4, "two connects and two disconnects");
            assert!(!events.iter().any(|m| matches!(m, Inbound::Request { .. })));
            assert_eq!(shared.queued.load(Relaxed), 0);
            assert_eq!(shared.queued_bytes.load(Relaxed), 0);
        }
    }

    #[test]
    fn query_tokens() {
        assert_eq!(
            query_param("token=abc&x=1", "token").as_deref(),
            Some("abc")
        );
        assert_eq!(
            query_param("x=1&token=a%3Ab", "token").as_deref(),
            Some("a:b")
        );
        assert_eq!(query_param("x=1", "token"), None);
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }

    #[test]
    fn codec_batch_exact_admission_and_fifo_charge() {
        let (tx, mut rx, pending) = ConnTx::codec_test_channel();
        assert!(!tx.try_send_codec_reset("rst".into(), Arc::new(vec![1; 5]), 7));
        assert_eq!(pending.load(Relaxed), 0);
        assert!(rx.try_recv().is_err());
        assert!(tx.try_send_codec_reset("rst".into(), Arc::new(vec![1; 5]), 8));
        assert_eq!(pending.load(Relaxed), 8);
        assert!(!tx.try_send_codec_binary(Arc::new(vec![2]), 8));
        let Out::Codec(item) = rx.try_recv().unwrap() else {
            panic!("codec batch")
        };
        assert_eq!(
            pending.load(Relaxed),
            8,
            "taking the queue item still owns its charge"
        );
        let (text, binary) = item.test_messages();
        assert_eq!(text.as_deref(), Some("rst"));
        assert_eq!(binary.as_deref().unwrap().as_slice(), &[1; 5]);
        assert_eq!(pending.load(Relaxed), 0);
        assert!(rx.try_recv().is_err(), "both messages were one FIFO item");
    }

    #[test]
    fn codec_channel_refusal_and_receiver_drop_reclaim_once() {
        let (tx, rx, pending) = ConnTx::codec_test_channel();
        assert!(tx.try_send_codec_text("ack".into(), 10));
        assert!(tx.try_send_codec_binary(Arc::new(vec![4; 7]), 10));
        assert_eq!(pending.load(Relaxed), 10);
        drop(rx);
        assert_eq!(pending.load(Relaxed), 0);
        assert!(!tx.try_send_codec_reset("rst".into(), Arc::new(vec![1; 5]), 8));
        assert_eq!(pending.load(Relaxed), 0);
    }

    #[test]
    fn codec_charge_rejects_invalid_caps_and_checked_overflow() {
        let pending = Arc::new(AtomicUsize::new(usize::MAX - 1));
        assert!(CodecQueueCharge::reserve(&pending, 2, MAX_CONN_QUEUE_BYTES).is_none());
        assert_eq!(pending.load(Relaxed), usize::MAX - 1);
        pending.store(0, Relaxed);
        assert!(CodecQueueCharge::reserve(&pending, 1, 0).is_none());
        assert!(CodecQueueCharge::reserve(&pending, 1, MAX_CONN_QUEUE_BYTES + 1).is_none());
        let charge =
            CodecQueueCharge::reserve(&pending, MAX_CONN_QUEUE_BYTES, MAX_CONN_QUEUE_BYTES)
                .unwrap();
        assert_eq!(pending.load(Relaxed), MAX_CONN_QUEUE_BYTES);
        assert!(CodecQueueCharge::reserve(&pending, 1, MAX_CONN_QUEUE_BYTES).is_none());
        drop(charge);
        assert_eq!(pending.load(Relaxed), 0);
    }
}
