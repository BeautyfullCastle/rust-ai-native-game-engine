//! Where the stream comes from. [`Source`] is all the viewer needs: the schema text, the next
//! message, and the two ways a view writes the sim (input, timeline control). [`SocketSource`]
//! speaks ERP (`docs/view-stream.md`, "Socket handshake") over a WebSocket or plain TCP with
//! nothing but the documented messages; the C ABI one is in [`crate::ffi`].

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::Arc;
use rustls::pki_types::{pem::PemObject, CertificateDer, ServerName};
use std::time::{Duration, Instant};

use orr_viewstream::{message_type, MSG_EVENTS, MSG_FRAME, MSG_FRAME3D, VERSION, VERSION_3D};
use serde_json::{json, Value as J};
use tungstenite::{Message, WebSocket};

/// One thing that came from the host.
#[derive(Debug)]
pub enum Incoming {
    /// The bytes of a ViewFrame.
    Frame(Vec<u8>),
    /// The bytes of a ViewFrame3.
    Frame3(Vec<u8>),
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

/// Where a network client is in joining a game.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NetState {
    #[default]
    Connecting,
    Playing,
    Disconnected,
    Failed,
}

impl NetState {
    pub fn name(self) -> &'static str {
        match self {
            NetState::Connecting => "connecting",
            NetState::Playing => "playing",
            NetState::Disconnected => "DISCONNECTED",
            NetState::Failed => "FAILED",
        }
    }
}

/// What a source that plays on a server knows about its session (the C ABI's `OrrSessionStatus`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetStatus {
    pub state: NetState,
    pub slot: u32,
    pub players: u32,
    pub rtt_ms: u32,
    pub input_delay: u32,
    pub head_tick: u64,
    pub verified_tick: u64,
    pub rollbacks: u64,
    pub resim_ticks: u64,
    /// The latest rollback resimulated these ticks (0, 0 = none yet).
    pub last_rollback: (u64, u64),
    pub desyncs: u64,
    pub stall_episodes: u64,
}

impl NetStatus {
    /// Ticks resimulated by the latest rollback.
    pub fn last_depth(&self) -> u64 {
        if self.last_rollback.0 == 0 {
            0
        } else {
            self.last_rollback.1 + 1 - self.last_rollback.0
        }
    }
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
    /// The state of the session if this source plays on a server (`None` for a local host).
    fn net_status(&mut self) -> Option<NetStatus> {
        None
    }
    /// The checksum of the confirmed state at `tick` (a checkpoint tick), if this source knows it.
    fn confirmed_checksum(&mut self, _tick: u64) -> Option<u64> {
        None
    }
}

/// Bytes of a binary message as a frame or an event batch (anything else is not ours).
pub fn classify(bytes: Vec<u8>) -> Option<Incoming> {
    let version = u16::from_le_bytes(bytes.get(4..6)?.try_into().ok()?);
    match (version, message_type(&bytes).ok()?) {
        (VERSION, MSG_FRAME) => Some(Incoming::Frame(bytes)),
        (VERSION, MSG_EVENTS) => Some(Incoming::Events(bytes)),
        (VERSION_3D, MSG_FRAME3D) => Some(Incoming::Frame3(bytes)),
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
    Ws(Box<WebSocket<SocketIo>>),
    /// Newline-delimited JSON; `partial` keeps a line that a read timeout cut in two.
    Tcp { reader: BufReader<TcpStream>, partial: Vec<u8> },
}

/// An interrupted/timed-out read yields to the caller so its existing deadline is still checked.
fn read_pending(e: &std::io::Error) -> bool {
    matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted)
}

/// Poll the same WebSocket again after a recoverable read; it retains any partial frame.
fn read_ws_text_or_queue(ws: &mut WebSocket<impl Read + Write>, queue: &mut VecDeque<Incoming>) -> Result<Option<String>, String> {
    match ws.read() {
        Ok(Message::Text(t)) => Ok(Some(t.as_str().to_string())),
        Ok(Message::Binary(b)) => {
            if let Some(m) = classify(b.to_vec()) {
                queue.push_back(m);
            }
            Ok(None)
        }
        Ok(Message::Close(_)) => Err("the host closed the connection".into()),
        Ok(_) => Ok(None),
        Err(tungstenite::Error::Io(e)) if read_pending(&e) => Ok(None),
        Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => Err("the host closed the connection".into()),
        Err(e) => Err(format!("connection: {e}")),
    }
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
    /// Set when the host is a relay client (`orr_remote_host --join`): its `session.status`.
    client: Option<ClientLink>,
}

/// The status of a host in client mode, polled with `session.status` (an asked-for status is
/// answered by `handle_text`; at most one call is on its way).
struct ClientLink {
    status: NetStatus,
    /// The id of the `session.status` call that has not been answered yet.
    pending: Option<u64>,
    asked: Instant,
}

/// How often a socket source asks a client-mode host for its status.
const STATUS_EVERY: Duration = Duration::from_millis(40);

/// The `session.status` result of a host in client mode as a [`NetStatus`].
fn net_status_of(r: &J) -> NetStatus {
    let n = |k: &str| r[k].as_u64().unwrap_or(0);
    NetStatus {
        state: match r["state"].as_str() {
            Some("playing") => NetState::Playing,
            Some("disconnected") => NetState::Disconnected,
            Some("failed") => NetState::Failed,
            _ => NetState::Connecting,
        },
        slot: n("slot") as u32,
        players: n("player_count") as u32,
        rtt_ms: n("rtt_ms") as u32,
        input_delay: n("input_delay") as u32,
        head_tick: n("head_tick"),
        verified_tick: n("verified_tick"),
        rollbacks: n("rollbacks"),
        resim_ticks: n("resim_ticks"),
        last_rollback: (n("last_rollback_from"), n("last_rollback_to")),
        desyncs: n("desyncs"),
        stall_episodes: n("stall_episodes"),
    }
}

fn rpc_error(msg: &J) -> Option<String> {
    let e = msg.get("error")?;
    Some(format!("{}: {}", e["code"], e["message"].as_str().unwrap_or("error")))
}

/// TLS uses Mozilla roots by default. An explicit PEM CA file replaces that root set;
/// chain, validity and hostname verification remain mandatory. No insecure mode exists.
#[derive(Clone, Debug)]
pub struct SocketOptions {
    pub ca_file: Option<PathBuf>,
    /// Shared TCP/TLS/WebSocket establishment deadline (DNS resolution is OS-managed).
    pub handshake_timeout: Duration,
}

impl Default for SocketOptions {
    fn default() -> Self {
        Self { ca_file: None, handshake_timeout: Duration::from_secs(10) }
    }
}

/// Bound every underlying operation, including TLS records and partial HTTP messages.
struct DeadlineStream {
    tcp: TcpStream,
    deadline: Instant,
}

impl DeadlineStream {
    fn remaining(&self) -> std::io::Result<Duration> {
        self.deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero())
            .ok_or_else(|| std::io::ErrorKind::TimedOut.into())
    }
}

impl Read for DeadlineStream {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        loop {
            self.tcp.set_read_timeout(Some(self.remaining()?))?;
            match self.tcp.read(bytes) {
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                result => return result,
            }
        }
    }
}

impl Write for DeadlineStream {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        loop {
            self.tcp.set_write_timeout(Some(self.remaining()?))?;
            match self.tcp.write(bytes) {
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                result => return result,
            }
        }
    }
    fn flush(&mut self) -> std::io::Result<()> { self.tcp.flush() }
}

enum SocketIo {
    Plain(DeadlineStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, DeadlineStream>>),
}

impl SocketIo {
    fn set_deadline(&mut self, deadline: Instant) {
        match self {
            Self::Plain(s) => s.deadline = deadline,
            Self::Tls(s) => s.sock.deadline = deadline,
        }
    }
}

impl Read for SocketIo {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        match self { Self::Plain(s) => s.read(bytes), Self::Tls(s) => s.read(bytes) }
    }
}
impl Write for SocketIo {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        match self { Self::Plain(s) => s.write(bytes), Self::Tls(s) => s.write(bytes) }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self { Self::Plain(s) => s.flush(), Self::Tls(s) => s.flush() }
    }
}

struct Endpoint {
    host: String,
    port: u16,
    ws: bool,
    tls: bool,
    // Deliberately excludes userinfo, path, query and fragment from all diagnostics.
    display: String,
}

fn split_url(url: &str) -> Result<Endpoint, String> {
    let uri: tungstenite::http::Uri = url.parse().map_err(|_| "invalid endpoint URL")?;
    let scheme = uri.scheme_str().ok_or("expected ws://, wss:// or tcp:// endpoint")?;
    if !matches!(scheme, "ws" | "wss" | "tcp") {
        return Err("unsupported endpoint scheme (use ws://, wss:// or tcp://)".into());
    }
    let authority = uri.authority().ok_or("endpoint has no host")?;
    if authority.as_str().contains('@') || url.contains('#') {
        return Err("endpoint userinfo and fragments are not supported".into());
    }
    let host = uri.host().filter(|s| !s.is_empty()).ok_or("endpoint has no host")?;
    if authority.as_str() != host && uri.port_u16().is_none() {
        return Err("invalid endpoint port".into());
    }
    let port = uri.port_u16().unwrap_or(if scheme == "wss" { 443 } else { 80 });
    Ok(Endpoint { host: host.trim_matches(['[', ']']).into(), port, ws: scheme != "tcp", tls: scheme == "wss", display: format!("{scheme}://{host}:{port}") })
}

fn tls_config(options: &SocketOptions) -> Result<rustls::ClientConfig, String> {
    let roots = if let Some(path) = &options.ca_file {
        let mut roots = rustls::RootCertStore::empty();
        for cert in CertificateDer::pem_file_iter(path).map_err(|_| "cannot read CA PEM file")? {
            roots.add(cert.map_err(|_| "invalid CA PEM file")?).map_err(|_| "invalid CA certificate")?;
        }
        if roots.is_empty() { return Err("CA PEM file contains no certificates".into()); }
        roots
    } else {
        rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() }
    };
    let mut config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions().map_err(|_| "TLS configuration failed")?
        .with_root_certificates(roots).with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}

impl SocketSource {
    /// Connects, authenticates (`token` as an `auth` message; a `?token=` in the URL also works),
    /// subscribes to the `viewstream` topic and waits for the schema.
    pub fn connect(url: &str, token: Option<&str>, max_fps: u32, session: Session) -> Result<SocketSource, String> {
        Self::connect_with_options(url, token, max_fps, session, &SocketOptions::default())
    }

    /// Like [`Self::connect`], with explicit CA trust and an establishment deadline.
    pub fn connect_with_options(url: &str, token: Option<&str>, max_fps: u32, session: Session, options: &SocketOptions) -> Result<SocketSource, String> {
        let endpoint = split_url(url)?;
        if options.ca_file.is_some() && !endpoint.tls { return Err("CA trust requires a wss:// endpoint".into()); }
        let config = if endpoint.tls { Some(tls_config(options)?) } else { None };
        let end = Instant::now().checked_add(options.handshake_timeout).ok_or("invalid handshake timeout")?;
        let addresses = (endpoint.host.as_str(), endpoint.port).to_socket_addrs().map_err(|_| "endpoint lookup failed")?;
        let mut stream = None;
        for address in addresses {
            let remaining = end.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()).ok_or("connection deadline exceeded")?;
            if let Ok(tcp) = TcpStream::connect_timeout(&address, remaining) { stream = Some(tcp); break; }
        }
        let stream = stream.ok_or("cannot connect to endpoint")?;
        let _ = stream.set_nodelay(true);
        let wire = if endpoint.ws {
            let raw = DeadlineStream { tcp: stream, deadline: end };
            let io = if let Some(config) = config {
                let name = ServerName::try_from(endpoint.host.clone()).map_err(|_| "invalid TLS server name")?;
                let connection = rustls::ClientConnection::new(Arc::new(config), name).map_err(|_| "TLS configuration failed")?;
                SocketIo::Tls(Box::new(rustls::StreamOwned::new(connection, raw)))
            } else { SocketIo::Plain(raw) };
            let mut result = tungstenite::client(url, io);
            let sock = loop {
                match result {
                    Ok((sock, _)) => break sock,
                    Err(tungstenite::HandshakeError::Interrupted(mid)) if Instant::now() < end => result = mid.handshake(),
                    // The HTTP error can carry a sensitive URI or reflected response. Never display it.
                    Err(_) => return Err("TLS/WebSocket handshake failed (check trust, hostname, peer and deadline)".into()),
                }
            };
            Wire::Ws(Box::new(sock))
        } else {
            stream.set_write_timeout(Some(Duration::from_secs(30))).map_err(|_| "cannot set write deadline")?;
            Wire::Tcp { reader: BufReader::new(stream), partial: Vec::new() }
        };
        let mut s = SocketSource { wire, schema: String::new(), next_id: 1, queue: VecDeque::new(), target: endpoint.display, client: None };
        if let Some(t) = token {
            s.call_wait("auth", json!({"token": t})).map_err(|_| "ERP authentication failed")?;
        }
        if let Session::Ensure { run } = session {
            let state = s.call_wait("sim.state", json!({}))?;
            if state["mode"] == "client" {
                // A host that plays on a relay server (`orr_remote_host --join`): there is no session to start.
                s.client = Some(ClientLink { status: net_status_of(&state), pending: None, asked: Instant::now() });
            } else if state["mode"] != "play" {
                s.call_wait("sim.start", json!({"run": run}))?;
            }
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
            Wire::Ws(ws) => {
                ws.get_mut().set_deadline(Instant::now() + Duration::from_secs(30));
                ws.send(Message::text(text)).map_err(|_| "WebSocket send failed".into())
            },
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
                ws.get_mut().set_deadline(Instant::now() + timeout.max(Duration::from_millis(1)));
                read_ws_text_or_queue(ws, &mut self.queue)
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
                    Err(e) if read_pending(&e) => Ok(None),
                    Err(e) => Err(format!("connection: {e}")),
                }
            }
        }
    }

    /// Handles a text message that is not the answer to a call being waited for.
    fn handle_text(&mut self, msg: &J) {
        if let Some(c) = self.client.as_mut() {
            if c.pending.is_some() && msg.get("id").and_then(J::as_u64) == c.pending {
                c.pending = None;
                if let Some(r) = msg.get("result") {
                    c.status = net_status_of(r);
                }
                return;
            }
        }
        match msg.get("method").and_then(J::as_str) {
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
        match &self.client {
            Some(_) => format!("socket {} (client of a relay server)", self.target),
            None => format!("socket {}", self.target),
        }
    }

    fn net_status(&mut self) -> Option<NetStatus> {
        let due = self.client.as_ref().is_some_and(|c| c.pending.is_none() && c.asked.elapsed() >= STATUS_EVERY);
        if due {
            let id = self.send_call("session.status", json!({})).ok();
            if let Some(c) = self.client.as_mut() {
                c.pending = id;
                c.asked = Instant::now();
            }
        }
        self.client.as_ref().map(|c| c.status)
    }

    fn confirmed_checksum(&mut self, tick: u64) -> Option<u64> {
        self.client.as_ref()?;
        let r = self.call_wait("sim.checksum", json!({"tick": tick})).ok()?;
        u64::from_str_radix(r["checksum"].as_str()?.strip_prefix("0x")?, 16).ok()
    }
}

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
