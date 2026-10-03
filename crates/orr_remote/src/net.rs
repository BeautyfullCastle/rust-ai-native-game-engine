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
use crate::link::{Incoming, LocalFrame};

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
    VerificationReady { serial: u64 },
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
    Close,
}

/// Where the messages of one connection go: to a socket writer task, or
/// straight into the channel of an in-process client.
#[derive(Clone)]
pub(crate) enum Sink {
    Net(UnboundedSender<Out>),
    Local(std::sync::mpsc::Sender<Incoming>),
}

/// The way to write to one connection, from any thread.
#[derive(Clone)]
pub(crate) struct ConnTx {
    sink: Sink,
    /// Bytes queued and not yet written or taken (for dropping frames to slow readers).
    pending: Arc<AtomicUsize>,
}

impl ConnTx {
    /// The writing end of an in-process connection (its reading end is the client's).
    pub(crate) fn local(tx: std::sync::mpsc::Sender<Incoming>, pending: Arc<AtomicUsize>) -> Self {
        Self {
            sink: Sink::Local(tx),
            pending,
        }
    }

    pub(crate) fn is_local(&self) -> bool {
        matches!(self.sink, Sink::Local(_))
    }

    pub(crate) fn send_text(&self, s: String) {
        if self.pending.load(Relaxed) > MAX_CONN_QUEUE_BYTES {
            return;
        }
        self.pending.fetch_add(s.len(), Relaxed);
        match &self.sink {
            Sink::Net(tx) => {
                let _ = tx.send(Out::Text(s));
            }
            Sink::Local(tx) => {
                let _ = tx.send(Incoming::Text(s));
            }
        }
    }
    /// Admission-aware presentation delivery. A rejected notification is
    /// accounted for by the negotiated view stream's next snapshot fence.
    pub(crate) fn try_send_text(&self, s: String) -> bool {
        if self.pending.load(Relaxed) > MAX_CONN_QUEUE_BYTES {
            return false;
        }
        self.send_control_text(s)
    }

    /// Request responses are never discarded by presentation backpressure.
    /// This control lane is deliberately outside the presentation queue bound.
    pub(crate) fn send_control_text(&self, s: String) -> bool {
        let cost = s.len();
        self.pending.fetch_add(cost, Relaxed);
        let sent = match &self.sink {
            Sink::Net(tx) => tx.send(Out::Text(s)).is_ok(),
            Sink::Local(tx) => tx.send(Incoming::Text(s)).is_ok(),
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

    pub(crate) fn try_send_local_frame(&self, f: Arc<LocalFrame>) -> bool {
        if self.pending.load(Relaxed) > MAX_CONN_QUEUE_BYTES {
            return false;
        }
        let Sink::Local(tx) = &self.sink else {
            return false;
        };
        let cost = f.cost;
        self.pending.fetch_add(cost, Relaxed);
        let sent = tx.send(Incoming::Local(f)).is_ok();
        if !sent {
            self.pending.fetch_sub(cost, Relaxed);
        }
        sent
    }
    pub(crate) fn send_binary(&self, b: Arc<Vec<u8>>) {
        if self.pending.load(Relaxed) > MAX_CONN_QUEUE_BYTES {
            return;
        }
        if let Sink::Net(tx) = &self.sink {
            self.pending.fetch_add(b.len(), Relaxed);
            let _ = tx.send(Out::Binary(b));
        }
    }
    /// A view stream message: a binary message on a socket, an `Incoming::Wire` in process.
    pub(crate) fn send_stream(&self, b: Arc<Vec<u8>>) {
        if self.pending.load(Relaxed) > MAX_CONN_QUEUE_BYTES {
            return;
        }
        self.pending.fetch_add(b.len(), Relaxed);
        match &self.sink {
            Sink::Net(tx) => {
                let _ = tx.send(Out::Binary(b));
            }
            Sink::Local(tx) => {
                let _ = tx.send(Incoming::Wire(b.to_vec()));
            }
        }
    }
    /// A frame for an in-process client: shared, never serialized.
    pub(crate) fn send_local_frame(&self, f: Arc<LocalFrame>) {
        if let Sink::Local(tx) = &self.sink {
            self.pending.fetch_add(f.cost, Relaxed);
            let _ = tx.send(Incoming::Local(f));
        }
    }
    pub(crate) fn pending(&self) -> usize {
        self.pending.load(Relaxed)
    }
    fn close(&self) {
        if let Sink::Net(tx) = &self.sink {
            let _ = tx.send(Out::Close);
        }
    }
    async fn closed(&self) {
        match &self.sink {
            Sink::Net(tx) => tx.closed().await,
            Sink::Local(_) => std::future::pending::<()>().await,
        }
    }
}

pub(crate) struct NetShared {
    pub auth: Auth,
    pub max_message_bytes: usize,
    pub allowed_origins: Vec<String>,
    pub max_connections: usize,
    pub max_queued: usize,
    pub inbox: Sender<Inbound>,
    pub next_id: AtomicU64,
    pub conns: AtomicUsize,
    /// Reserved requests waiting in producers, the host inbox or its stash.
    pub queued: Arc<AtomicUsize>,
}

/// One reserved waiting-request slot. It follows the request through the inbox
/// and stash, and is dropped immediately before dispatch or when delivery ends.
/// Hold only the counter, never NetShared (whose sender would create a cycle).
pub(crate) struct RequestPermit {
    queued: Arc<AtomicUsize>,
}

impl RequestPermit {
    pub(crate) fn reserve(queued: &Arc<AtomicUsize>, limit: usize) -> Option<Self> {
        // Relaxed is enough for accounting; the channel publishes request data.
        let mut current = queued.load(Relaxed);
        loop {
            if current >= limit {
                return None;
            }
            // current < limit <= usize::MAX, so this addition cannot overflow.
            match queued.compare_exchange_weak(current, current + 1, Relaxed, Relaxed) {
                Ok(_) => return Some(Self { queued: queued.clone() }),
                Err(observed) => current = observed,
            }
        }
    }
}

impl Drop for RequestPermit {
    fn drop(&mut self) {
        self.queued.fetch_sub(1, Relaxed);
    }
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
        let Some(permit) = RequestPermit::reserve(&self.shared.queued, self.shared.max_queued) else {
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
        (Arc::new(NetShared {
            auth: Auth::DevNoAuth,
            max_message_bytes: 4 << 20,
            allowed_origins: Vec::new(),
            max_connections: 64,
            max_queued: limit,
            inbox,
            next_id: AtomicU64::new(1),
            conns: AtomicUsize::new(0),
            queued: Arc::new(AtomicUsize::new(0)),
        }), recv)
    }

    #[test]
    fn request_permit_boundaries_and_release() {
        let queued = Arc::new(AtomicUsize::new(0));
        assert!(RequestPermit::reserve(&queued, 0).is_none());
        let first = RequestPermit::reserve(&queued, 1).unwrap();
        assert!(RequestPermit::reserve(&queued, 1).is_none());
        assert_eq!(queued.load(Relaxed), 1);
        drop(first);
        assert_eq!(queued.load(Relaxed), 0);

        // Synthetic boundary: no allocation of usize::MAX requests is needed.
        queued.store(usize::MAX - 1, Relaxed);
        let last = RequestPermit::reserve(&queued, usize::MAX).unwrap();
        assert_eq!(queued.load(Relaxed), usize::MAX);
        assert!(RequestPermit::reserve(&queued, usize::MAX).is_none());
        assert_eq!(queued.load(Relaxed), usize::MAX);
        drop(last);
        assert_eq!(queued.load(Relaxed), usize::MAX - 1);
    }

    #[test]
    fn local_and_network_failed_delivery_release_their_permits() {
        use crate::link::{LocalConnector, Transport};
        let (shared, recv) = queue_fixture(1);
        let mut local = LocalConnector { shared: shared.clone() }.connect("local", Caps::ALL).unwrap();
        let (mut link, _out, _pending) = new_link(&shared, false);
        link.set_identity("net".into(), Caps::ALL);
        drop(recv);
        let request = crate::link::Request { id: Some(1), method: "sim.state".into(), params: J::Null };
        for _ in 0..3 {
            assert!(local.send(request.clone()).unwrap_err().to_string().contains("stopped"));
            assert_eq!(shared.queued.load(Relaxed), 0);
            assert!(matches!(link.on_text(&request.to_text()), Flow::Close));
            assert_eq!(shared.queued.load(Relaxed), 0);
        }
    }

    #[test]
    fn concurrent_local_and_network_requests_share_one_cap() {
        use crate::link::{LocalConnector, Transport};
        const LIMIT: usize = 7;
        const PRODUCERS: usize = 16;
        // Exercise contention with valid requests, without claiming to force the
        // old check/increment race's particular scheduling window.
        for _ in 0..16 {
            let (shared, recv) = queue_fixture(LIMIT);
            let barrier = Arc::new(std::sync::Barrier::new(PRODUCERS));
            std::thread::scope(|scope| {
                let mut workers = Vec::new();
                for producer in 0..PRODUCERS {
                    let shared = shared.clone();
                    let barrier = barrier.clone();
                    workers.push(scope.spawn(move || {
                        let request = crate::link::Request {
                            id: Some(producer as u64), method: "sim.state".into(), params: J::Null,
                        };
                        if producer % 2 == 0 {
                            let mut local = LocalConnector { shared: shared.clone() }
                                .connect("local", Caps::ALL).unwrap();
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
                                    assert_eq!(reply["id"], producer);
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
            let mut requests = Vec::new();
            for msg in recv.try_iter() {
                if matches!(msg, Inbound::Request { .. }) {
                    requests.push(msg);
                }
            }
            assert_eq!(requests.len(), LIMIT);
            assert_eq!(shared.queued.load(Relaxed), LIMIT, "taking a message retains its slot");
            drop(requests);
            assert_eq!(shared.queued.load(Relaxed), 0);
        }
    }

    #[test]
    fn zero_capacity_rejects_requests_but_keeps_control_events() {
        use crate::link::{LocalConnector, Transport};
        let (shared, recv) = queue_fixture(0);
        let mut local = LocalConnector { shared: shared.clone() }.connect("local", Caps::ALL).unwrap();
        let (mut link, mut out, _pending) = new_link(&shared, false);
        assert!(matches!(link.on_text(r#"{"jsonrpc":"2.0","id":1,"method":"auth"}"#), Flow::Continue));
        assert!(matches!(out.try_recv().unwrap(), Out::Text(_)));
        let request = crate::link::Request { id: Some(2), method: "sim.state".into(), params: J::Null };
        assert!(local.send(request.clone()).is_err());
        assert!(matches!(link.on_text(&request.to_text()), Flow::Continue));
        let Out::Text(text) = out.try_recv().unwrap() else { panic!("busy response") };
        assert_eq!(serde_json::from_str::<J>(&text).unwrap()["error"]["data"]["kind"], "busy");
        drop(local);
        link.finish();
        let events: Vec<_> = recv.try_iter().collect();
        assert_eq!(events.len(), 4, "two connects and two disconnects");
        assert!(!events.iter().any(|m| matches!(m, Inbound::Request { .. })));
        assert_eq!(shared.queued.load(Relaxed), 0);
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
}
