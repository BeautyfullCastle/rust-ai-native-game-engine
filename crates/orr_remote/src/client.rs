//! [`ErpClient`]: an ERP client over any [`Transport`] (tests, scripts,
//! tools, and the editor).

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value as J};

use crate::error::RpcError;
use crate::link::{Incoming, LocalFrame, PumpedWs, Request, Transport, TxHandle, WsTransport};

/// Why a call failed.
#[derive(Debug)]
pub enum ClientError {
    /// The server answered with an error.
    Rpc(RpcError),
    /// The connection failed or closed.
    Transport(String),
    /// The server sent something that is not ERP.
    Protocol(String),
}

impl core::fmt::Display for ClientError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ClientError::Rpc(e) => write!(f, "{e}"),
            ClientError::Transport(m) => write!(f, "connection: {m}"),
            ClientError::Protocol(m) => write!(f, "protocol: {m}"),
        }
    }
}
impl std::error::Error for ClientError {}

impl ClientError {
    /// The server's error, if that is what this is.
    pub fn rpc(&self) -> Option<&RpcError> {
        match self {
            ClientError::Rpc(e) => Some(e),
            _ => None,
        }
    }
}

/// Responses that nobody waited for are kept (oldest dropped above this many).
const MAX_STASHED_RESPONSES: usize = 1024;

/// A JSON-RPC client over any [`Transport`]: a blocking WebSocket
/// ([`connect`](Self::connect)), a WebSocket on a background thread
/// ([`connect_pumped`](Self::connect_pumped)) or an in-process link to a
/// host in the same process ([`with_transport`](Self::with_transport) on a
/// [`LocalTransport`](crate::LocalTransport)). The same calls work on all of them.
///
/// Two ways to use it:
///
/// - [`call`](Self::call) sends a request and waits for its response;
///   notifications and frames that arrive meanwhile are kept in
///   [`notifications`](Self::notifications), [`frames`](Self::frames) and
///   [`local_frames`](Self::local_frames).
/// - [`post`](Self::post) sends without waiting, and [`poll`](Self::poll)
///   (non-blocking) collects what came in: responses go to
///   [`responses`](Self::responses). A UI uses this so a request never
///   stalls a frame. The server answers in order, so a `call` after some
///   `post`s returns after the posts were handled.
pub struct ErpClient {
    t: Box<dyn Transport>,
    next_id: u64,
    /// Notifications received so far (oldest first).
    pub notifications: VecDeque<J>,
    /// Binary frame messages received so far (sockets).
    pub frames: VecDeque<Vec<u8>>,
    /// Frames received from an in-process host.
    pub local_frames: VecDeque<Arc<LocalFrame>>,
    /// Responses to [`post`](Self::post)ed requests (and any other response
    /// nobody waited for), as `(id, result)`, oldest first.
    pub responses: VecDeque<(u64, Result<J, RpcError>)>,
    /// How long a call may wait for its response (default 30 s).
    pub call_timeout: Duration,
}

fn parse(text: &str) -> Result<J, ClientError> {
    serde_json::from_str(text).map_err(|e| ClientError::Protocol(format!("bad JSON from server: {e}")))
}

fn result_of(msg: &J) -> Result<J, RpcError> {
    match msg.get("error") {
        Some(e) => Err(RpcError::from_json(e)),
        None => Ok(msg.get("result").cloned().unwrap_or(J::Null)),
    }
}

impl ErpClient {
    /// A client on an already connected transport (no authentication is done).
    pub fn with_transport(t: Box<dyn Transport>) -> ErpClient {
        ErpClient {
            t,
            next_id: 1,
            notifications: VecDeque::new(),
            frames: VecDeque::new(),
            local_frames: VecDeque::new(),
            responses: VecDeque::new(),
            call_timeout: Duration::from_secs(30),
        }
    }

    /// Connects and, if `token` is given, authenticates with an `auth` message.
    pub fn connect(url: &str, token: Option<&str>) -> Result<ErpClient, ClientError> {
        let mut c = ErpClient::with_transport(Box::new(WsTransport::connect(url)?));
        if let Some(t) = token {
            c.call("auth", json!({"token": t}))?;
        }
        Ok(c)
    }

    /// Like [`connect`](Self::connect), but the socket lives on a background
    /// thread: [`post`](Self::post) and [`poll`](Self::poll) never wait for the
    /// network. A UI attaches to a remote host this way.
    pub fn connect_pumped(url: &str, token: Option<&str>) -> Result<ErpClient, ClientError> {
        let mut c = ErpClient::with_transport(Box::new(PumpedWs::connect(url, Duration::from_secs(10))?));
        if let Some(t) = token {
            c.call("auth", json!({"token": t}))?;
        }
        Ok(c)
    }

    /// Connects with the token in the URL (`?token=`), no `auth` message.
    pub fn connect_with_url_token(url: &str, token: &str) -> Result<ErpClient, ClientError> {
        // The request path must exist: `ws://host:port` becomes `ws://host:port/?token=...`.
        let authority_end = url.find("://").map_or(0, |i| i + 3);
        let url = if url[authority_end..].contains('/') { url.to_string() } else { format!("{url}/") };
        let sep = if url.contains('?') { '&' } else { '?' };
        let mut enc = String::new();
        for b in token.bytes() {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                enc.push(b as char);
            } else {
                enc.push_str(&format!("%{b:02X}"));
            }
        }
        ErpClient::connect(&format!("{url}{sep}token={enc}"), None)
    }

    /// A handle other threads can send requests through (not every transport has one).
    pub fn sender(&self) -> Option<Arc<dyn TxHandle>> {
        self.t.sender()
    }

    /// Sends raw text as one message (for tests of malformed input).
    pub fn send_text(&mut self, text: &str) -> Result<(), ClientError> {
        self.t.send_raw_text(text)
    }

    /// Sends raw bytes as one binary message.
    pub fn send_binary(&mut self, bytes: &[u8]) -> Result<(), ClientError> {
        self.t.send_raw_binary(bytes)
    }

    /// Reads the next message: `Ok(None)` on timeout. Text is returned as
    /// text; frames are kept and `Ok(None)` is returned.
    fn read_one(&mut self, timeout: Duration) -> Result<Option<String>, ClientError> {
        match self.t.recv(timeout)? {
            Some(Incoming::Text(t)) => Ok(Some(t)),
            Some(Incoming::Wire(b)) => {
                self.frames.push_back(b);
                Ok(None)
            }
            Some(Incoming::Local(f)) => {
                self.local_frames.push_back(f);
                Ok(None)
            }
            None => Ok(None),
        }
    }

    /// Waits for the next text message (a response or notification), up to `timeout`.
    pub fn recv_text(&mut self, timeout: Duration) -> Result<Option<String>, ClientError> {
        let end = Instant::now() + timeout;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            if let Some(t) = self.read_one(left)? {
                return Ok(Some(t));
            }
            if Instant::now() >= end {
                return Ok(None);
            }
        }
    }

    fn stash_response(&mut self, id: u64, r: Result<J, RpcError>) {
        if self.responses.len() >= MAX_STASHED_RESPONSES {
            self.responses.pop_front();
        }
        self.responses.push_back((id, r));
    }

    /// Sorts one text message: a notification is kept, a response to an
    /// earlier [`post`](Self::post) is stashed. Returns the message if it is
    /// the response to `want`.
    fn route(&mut self, text: &str, want: Option<u64>) -> Result<Option<J>, ClientError> {
        let msg = parse(text)?;
        if msg.get("method").is_some() && msg.get("id").is_none() {
            self.notifications.push_back(msg);
            return Ok(None);
        }
        let rid = msg.get("id");
        if want.is_some() && (rid == want.map(|w| json!(w)).as_ref() || rid == Some(&J::Null)) {
            return Ok(Some(msg));
        }
        if let Some(id) = rid.and_then(J::as_u64) {
            let r = result_of(&msg);
            self.stash_response(id, r);
        }
        Ok(None)
    }

    /// Sends a request and returns its id, without waiting. The response
    /// shows up in [`responses`](Self::responses) after a [`poll`](Self::poll).
    pub fn post(&mut self, method: &str, params: J) -> Result<u64, ClientError> {
        let id = self.next_id;
        self.next_id += 1;
        self.t.send(Request { id: Some(id), method: method.to_string(), params })?;
        Ok(id)
    }

    /// Collects everything that has arrived, without waiting: notifications,
    /// frames and responses are sorted into their queues. Returns how many
    /// messages came in. `Err` once the connection is gone (messages that
    /// arrived before are still queued).
    pub fn poll(&mut self) -> Result<usize, ClientError> {
        let mut n = 0;
        // Bounded, so a flood cannot hold the caller.
        for _ in 0..4096 {
            match self.t.recv(Duration::ZERO)? {
                None => break,
                Some(Incoming::Text(t)) => {
                    self.route(&t, None)?;
                    n += 1;
                }
                Some(Incoming::Wire(b)) => {
                    self.frames.push_back(b);
                    n += 1;
                }
                Some(Incoming::Local(f)) => {
                    self.local_frames.push_back(f);
                    n += 1;
                }
            }
        }
        Ok(n)
    }

    /// Takes the stashed response to request `id`, if it has arrived.
    pub fn take_response(&mut self, id: u64) -> Option<Result<J, RpcError>> {
        let i = self.responses.iter().position(|(rid, _)| *rid == id)?;
        self.responses.remove(i).map(|(_, r)| r)
    }

    /// Calls a method and waits for its response.
    pub fn call(&mut self, method: &str, params: J) -> Result<J, ClientError> {
        let id = self.post(method, params)?;
        let end = Instant::now() + self.call_timeout;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(ClientError::Transport(format!("timed out waiting for the response to {method}")));
            }
            let Some(text) = self.read_one(left)? else { continue };
            if let Some(msg) = self.route(&text, Some(id))? {
                return match msg.get("error") {
                    Some(e) => Err(ClientError::Rpc(RpcError::from_json(e))),
                    None => Ok(msg.get("result").cloned().unwrap_or(J::Null)),
                };
            }
        }
    }

    /// Like [`call`](Self::call), for a call that must fail: returns the server's error. Panics otherwise.
    pub fn call_err(&mut self, method: &str, params: J) -> RpcError {
        match self.call(method, params) {
            Err(ClientError::Rpc(e)) => e,
            Err(other) => panic!("{method}: expected an RPC error, got {other}"),
            Ok(v) => panic!("{method}: expected an error, got {v}"),
        }
    }

    /// Waits for a notification with this method (looking at the ones already received first).
    pub fn wait_notification(&mut self, method: &str, timeout: Duration) -> Result<Option<J>, ClientError> {
        let end = Instant::now() + timeout;
        loop {
            if let Some(i) = self.notifications.iter().position(|n| n.get("method").and_then(J::as_str) == Some(method)) {
                return Ok(self.notifications.remove(i));
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            if let Some(text) = self.read_one(left)? {
                self.route(&text, None)?;
            }
        }
    }

    /// Waits for a binary frame message.
    pub fn wait_frame(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, ClientError> {
        let end = Instant::now() + timeout;
        loop {
            if let Some(f) = self.frames.pop_front() {
                return Ok(Some(f));
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            if let Some(text) = self.read_one(left)? {
                self.route(&text, None)?;
            }
        }
    }
}
