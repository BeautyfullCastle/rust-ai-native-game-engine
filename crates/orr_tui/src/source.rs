//! Where the stream comes from. [`Source`] is all the viewer needs: the schema text, the next
//! message, and the two ways a view writes the sim (input, timeline control). [`SocketSource`]
//! speaks ERP (`docs/view-stream.md`, "Socket handshake") over a WebSocket or plain TCP with
//! nothing but the documented messages; the C ABI one is in [`crate::ffi`].

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use orr_viewstream::{message_type, MSG_EVENTS, MSG_FRAME};
use serde_json::{json, Value as J};
use tungstenite::{Message, WebSocket};

/// One thing that came from the host.
#[derive(Debug)]
pub enum Incoming {
    /// The bytes of a ViewFrame.
    Frame(Vec<u8>),
    /// The bytes of an EventBatch.
    Events(Vec<u8>),
    /// The host refused something we asked (text of the error).
    Error(String),
}

/// Timeline controls a view may use (the editor's play controls).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Play,
    Pause,
    Step(u32),
}

pub trait Source {
    /// The schema text, as received.
    fn schema_text(&self) -> &str;
    /// The next message, or `None` after `timeout` without one.
    fn recv(&mut self, timeout: Duration) -> Result<Option<Incoming>, String>;
    /// Sets the held input of a player: exactly `input.size` bytes of the schema layout.
    fn set_input(&mut self, player: u8, bytes: &[u8]) -> Result<(), String>;
    fn control(&mut self, c: Control) -> Result<(), String>;
    /// A few words about the connection, for the help line.
    fn describe(&self) -> String;
}

/// Bytes of a binary message as a frame or an event batch (anything else is not ours).
pub fn classify(bytes: Vec<u8>) -> Option<Incoming> {
    match message_type(&bytes).ok()? {
        MSG_FRAME => Some(Incoming::Frame(bytes)),
        MSG_EVENTS => Some(Incoming::Events(bytes)),
        _ => None,
    }
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let nib = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    s.as_bytes().chunks(2).map(|p| Some(nib(p[0])? << 4 | nib(p[1])?)).collect()
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

enum Wire {
    Ws(Box<WebSocket<TcpStream>>),
    /// Newline-delimited JSON; `partial` keeps a line that a read timeout cut in two.
    Tcp { reader: BufReader<TcpStream>, partial: Vec<u8> },
}

/// What a connection does about the play session of the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Session {
    /// Only watch what is there.
    Leave,
    /// Make sure a session exists (`sim.start`, paused unless `run`) when the host is not playing.
    Ensure { run: bool },
}

pub struct SocketSource {
    wire: Wire,
    schema: String,
    next_id: u64,
    queue: VecDeque<Incoming>,
    target: String,
}

fn rpc_error(msg: &J) -> Option<String> {
    let e = msg.get("error")?;
    Some(format!("{}: {}", e["code"], e["message"].as_str().unwrap_or("error")))
}

/// `ws://host:port/path?query` -> (`host:port`, whether it is a WebSocket).
fn split_url(url: &str) -> Result<(String, bool), String> {
    let (scheme, rest) = url.split_once("://").ok_or_else(|| format!("{url}: expected ws://host:port or tcp://host:port"))?;
    let ws = match scheme {
        "ws" => true,
        "tcp" => false,
        other => return Err(format!("unsupported scheme {other}:// (use ws:// or tcp://; wss needs TLS, which this viewer does not have)")),
    };
    let authority = rest.split(['/', '?']).next().unwrap_or("");
    if authority.is_empty() {
        return Err(format!("{url}: no host"));
    }
    Ok((if authority.contains(':') { authority.to_string() } else { format!("{authority}:80") }, ws))
}

impl SocketSource {
    /// Connects, authenticates (`token` as an `auth` message; a `?token=` in the URL also works),
    /// subscribes to the `viewstream` topic and waits for the schema.
    pub fn connect(url: &str, token: Option<&str>, max_fps: u32, session: Session) -> Result<SocketSource, String> {
        let (authority, ws) = split_url(url)?;
        let stream = TcpStream::connect(&authority).map_err(|e| format!("cannot connect to {authority}: {e}"))?;
        let _ = stream.set_nodelay(true);
        let wire = if ws {
            let (sock, _) = tungstenite::client(url, stream).map_err(|e| format!("WebSocket handshake with {url} failed: {e}"))?;
            Wire::Ws(Box::new(sock))
        } else {
            Wire::Tcp { reader: BufReader::new(stream), partial: Vec::new() }
        };
        let mut s = SocketSource { wire, schema: String::new(), next_id: 1, queue: VecDeque::new(), target: url.to_string() };
        if let Some(t) = token {
            s.call_wait("auth", json!({"token": t}))?;
        }
        if let Session::Ensure { run } = session {
            let state = s.call_wait("sim.state", json!({}))?;
            if state["mode"] != "play" {
                s.call_wait("sim.start", json!({"run": run}))?;            }
        }
        s.call_wait("watch.subscribe", json!({"topics": ["viewstream"], "max_fps": max_fps, "source": "sim"}))?;
        // The schema is the first thing after the response (it may already have been read).
        let end = Instant::now() + Duration::from_secs(10);
        while s.schema.is_empty() {
            if Instant::now() > end {
                return Err("the host sent no view stream schema (is a play session running?)".into());
            }
            if let Some(Incoming::Error(e)) = s.pump(Duration::from_millis(50))? {
                return Err(e);
            }
        }
        Ok(s)
    }

    fn send_text(&mut self, text: String) -> Result<(), String> {
        match &mut self.wire {
            Wire::Ws(ws) => ws.send(Message::text(text)).map_err(|e| format!("send: {e}")),
            Wire::Tcp { reader, .. } => {
                let s = reader.get_mut();
                s.write_all(text.as_bytes()).and_then(|()| s.write_all(b"\n")).map_err(|e| format!("send: {e}"))
            }
        }
    }

    fn send_call(&mut self, method: &str, params: J) -> Result<u64, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send_text(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string())?;
        Ok(id)
    }

    /// Sends a call and waits for its response; frames that arrive meanwhile are kept.
    fn call_wait(&mut self, method: &str, params: J) -> Result<J, String> {
        let id = self.send_call(method, params)?;
        let end = Instant::now() + Duration::from_secs(30);
        loop {
            if Instant::now() > end {
                return Err(format!("{method}: no answer"));
            }
            let Some(text) = self.read_text_or_queue(Duration::from_millis(100))? else { continue };
            let Ok(msg) = serde_json::from_str::<J>(&text) else { continue };
            if msg.get("id").and_then(J::as_u64) == Some(id) {
                return match rpc_error(&msg) {
                    Some(e) => Err(format!("{method}: {e}")),
                    None => Ok(msg.get("result").cloned().unwrap_or(J::Null)),
                };
            }
            self.handle_text(&msg);
        }
    }

    /// Reads one wire message. Binary frames go to the queue and `None` comes back; text is returned.
    fn read_text_or_queue(&mut self, timeout: Duration) -> Result<Option<String>, String> {
        match &mut self.wire {
            Wire::Ws(ws) => {
                let _ = ws.get_ref().set_read_timeout(Some(timeout.max(Duration::from_millis(1))));
                match ws.read() {
                    Ok(Message::Text(t)) => Ok(Some(t.as_str().to_string())),
                    Ok(Message::Binary(b)) => {
                        if let Some(m) = classify(b.to_vec()) {
                            self.queue.push_back(m);
                        }
                        Ok(None)
                    }
                    Ok(Message::Close(_)) => Err("the host closed the connection".into()),
                    Ok(_) => Ok(None),
                    Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => Ok(None),
                    Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => Err("the host closed the connection".into()),
                    Err(e) => Err(format!("connection: {e}")),
                }
            }
            Wire::Tcp { reader, partial } => {
                let _ = reader.get_ref().set_read_timeout(Some(timeout.max(Duration::from_millis(1))));
                match reader.read_until(b'\n', partial) {
                    Ok(0) => Err("the host closed the connection".into()),
                    Ok(_) if partial.ends_with(b"\n") => {
                        let line = String::from_utf8_lossy(partial).trim().to_string();
                        partial.clear();
                        Ok(Some(line))
                    }
                    Ok(_) => Ok(None),
                    Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => Ok(None),
                    Err(e) => Err(format!("connection: {e}")),
                }
            }
        }
    }

    /// Handles a text message that is not the answer to a call being waited for.
    fn handle_text(&mut self, msg: &J) {        match msg.get("method").and_then(J::as_str) {
            Some("watch.viewstream.schema") => self.schema = msg["params"].to_string(),
            Some("watch.viewstream") => {
                // Plain TCP: the frame as hex in JSON.
                if let Some(m) = msg["params"]["data"].as_str().and_then(from_hex).and_then(classify) {
                    self.queue.push_back(m);
                }
            }
            _ => {
                if let Some(e) = rpc_error(msg) {
                    self.queue.push_back(Incoming::Error(e));
                }
            }
        }
    }

    /// Reads from the wire once and queues what it is; returns the next queued message.
    fn pump(&mut self, timeout: Duration) -> Result<Option<Incoming>, String> {
        if self.queue.is_empty() {
            if let Some(text) = self.read_text_or_queue(timeout)? {
                if let Ok(msg) = serde_json::from_str::<J>(&text) {
                    self.handle_text(&msg);
                }
            }
        }
        Ok(self.queue.pop_front())
    }
}

impl Source for SocketSource {
    fn schema_text(&self) -> &str {
        &self.schema
    }

    fn recv(&mut self, timeout: Duration) -> Result<Option<Incoming>, String> {
        self.pump(timeout)
    }

    fn set_input(&mut self, player: u8, bytes: &[u8]) -> Result<(), String> {
        self.send_call("sim.input", json!({"player": player, "input": to_hex(bytes)})).map(|_| ())
    }

    fn control(&mut self, c: Control) -> Result<(), String> {
        let (method, params) = match c {
            Control::Play => ("sim.play", json!({})),
            Control::Pause => ("sim.pause", json!({})),
            Control::Step(n) => ("sim.step", json!({"n": n})),
        };
        self.send_call(method, params).map(|_| ())
    }

    fn describe(&self) -> String {
        format!("socket {}", self.target)
    }
}
