//! [`ErpClient`]: a small blocking ERP client (tests, scripts, tools).

use std::collections::VecDeque;
use std::net::TcpStream;
use std::time::{Duration, Instant};

use serde_json::{json, Value as J};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use crate::error::RpcError;

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

/// A blocking JSON-RPC client over WebSocket. Notifications and binary
/// frame messages that arrive while a call waits are kept in
/// [`notifications`](Self::notifications) and [`frames`](Self::frames).
pub struct ErpClient {
    ws: WebSocket<MaybeTlsStream<TcpStream>>,
    next_id: u64,
    /// Notifications received so far (oldest first).
    pub notifications: VecDeque<J>,
    /// Binary frame messages received so far.
    pub frames: VecDeque<Vec<u8>>,
    /// How long a call may wait for its response (default 30 s).
    pub call_timeout: Duration,
}

fn transport(e: impl core::fmt::Display) -> ClientError {
    ClientError::Transport(e.to_string())
}

impl ErpClient {
    /// Connects and, if `token` is given, authenticates with an `auth` message.
    pub fn connect(url: &str, token: Option<&str>) -> Result<ErpClient, ClientError> {
        let (ws, _) = tungstenite::connect(url).map_err(transport)?;
        if let MaybeTlsStream::Plain(s) = ws.get_ref() {
            let _ = s.set_nodelay(true);
        }
        let mut c = ErpClient { ws, next_id: 1, notifications: VecDeque::new(), frames: VecDeque::new(), call_timeout: Duration::from_secs(30) };
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

    fn set_read_timeout(&mut self, d: Option<Duration>) {
        if let MaybeTlsStream::Plain(s) = self.ws.get_ref() {
            let _ = s.set_read_timeout(d);
        }
    }

    /// Sends raw text as one message (for tests of malformed input).
    pub fn send_text(&mut self, text: &str) -> Result<(), ClientError> {
        self.ws.send(Message::text(text)).map_err(transport)
    }

    /// Sends raw bytes as one binary message.
    pub fn send_binary(&mut self, bytes: &[u8]) -> Result<(), ClientError> {
        self.ws.send(Message::binary(bytes.to_vec())).map_err(transport)
    }

    /// Reads the next message: `Ok(None)` on timeout. Text is returned as
    /// text; binary messages are pushed to `frames` and `Ok(None)` is returned.
    fn read_one(&mut self, timeout: Duration) -> Result<Option<String>, ClientError> {
        self.set_read_timeout(Some(timeout.max(Duration::from_millis(1))));
        match self.ws.read() {
            Ok(Message::Text(t)) => Ok(Some(t.as_str().to_string())),
            Ok(Message::Binary(b)) => {
                self.frames.push_back(b.to_vec());
                Ok(None)
            }
            Ok(Message::Close(_)) => Err(ClientError::Transport("closed by the server".into())),
            Ok(_) => Ok(None),
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => Ok(None),
            Err(e) => Err(transport(e)),
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

    /// Calls a method and waits for its response.
    pub fn call(&mut self, method: &str, params: J) -> Result<J, ClientError> {
        let id = self.next_id;
        self.next_id += 1;
        let req = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.send_text(&req.to_string())?;
        let end = Instant::now() + self.call_timeout;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(ClientError::Transport(format!("timed out waiting for the response to {method}")));
            }
            let Some(text) = self.read_one(left)? else { continue };
            let msg: J = serde_json::from_str(&text).map_err(|e| ClientError::Protocol(format!("bad JSON from server: {e}")))?;
            if msg.get("method").is_some() && msg.get("id").is_none() {
                self.notifications.push_back(msg);
                continue;
            }
            let rid = msg.get("id");
            if rid == Some(&json!(id)) || rid == Some(&J::Null) {
                return match msg.get("error") {
                    Some(e) => Err(ClientError::Rpc(RpcError::from_json(e))),
                    None => Ok(msg.get("result").cloned().unwrap_or(J::Null)),
                };
            }
            // A response to something else: ignore.
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
                let msg: J = serde_json::from_str(&text).map_err(|e| ClientError::Protocol(format!("bad JSON from server: {e}")))?;
                self.notifications.push_back(msg);
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
                let msg: J = serde_json::from_str(&text).map_err(|e| ClientError::Protocol(format!("bad JSON from server: {e}")))?;
                self.notifications.push_back(msg);
            }
        }
    }
}
