//! Building client links and server endpoints from plain options (what a
//! command line gives), for QUIC and WebSocket.

use std::net::{SocketAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use orr_net::{Conditioned, Endpoint, LinkConditions, NetConfig, QuicServerTls, QuicTrust, Transport};

use crate::link::NetLink;
use crate::server_ep::NetEndpoint;

/// Which transport a link or server uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportKind {
    Quic,
    Ws,
    Wss,
    /// Native WebTransport client; server uses Quic + webtransport.
    Wt,
}

impl FromStr for TransportKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "quic" => Ok(TransportKind::Quic),
            "ws" | "websocket" => Ok(TransportKind::Ws),
            "wt" | "webtransport" => Ok(TransportKind::Wt),
            "wss" => Ok(TransportKind::Wss),
            other => Err(format!("unknown transport '{other}' (use quic, ws, wss or wt)")),
        }
    }
}

/// Certificate trust. Plain WS ignores it; WSS/WT support PemFile and WebPki.
#[derive(Clone, Debug)]
pub enum Trust {
    /// Pin the SHA-256 of the server certificate (what the dev server prints).
    Fingerprint([u8; 32]),
    /// DEVELOPMENT ONLY: accept any certificate.
    InsecureDev,
    /// Trust the certificates in this PEM file.
    PemFile(PathBuf),
    /// The public root set.
    WebPki,
}

/// Simulated network conditions on one side, as a command line states them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimConditions {
    /// One-way delay added in each direction, milliseconds.
    pub latency_ms: u64,
    pub jitter_ms: u64,
    /// Loss of unreliable messages, `0.0..=1.0`, in each direction.
    pub loss: f32,
    pub seed: u64,
}

impl SimConditions {
    pub fn is_active(&self) -> bool {
        self.latency_ms > 0 || self.jitter_ms > 0 || self.loss > 0.0
    }

    fn link_conditions(&self) -> LinkConditions {
        LinkConditions {
            latency: Duration::from_millis(self.latency_ms),
            jitter: Duration::from_millis(self.jitter_ms),
            loss: self.loss,
            seed: self.seed,
        }
    }
}

/// A seed that differs per call and per process, for simulated conditions
/// when the user gave none: process id, a counter and the clock, mixed.
/// Print it so a run can be repeated with an explicit seed.
pub fn fresh_seed() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    let mut z = u64::from(std::process::id()).rotate_left(32) ^ nanos ^ n.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Wraps `ep` in the conditioner when `sim` asks for any effect.
fn condition(ep: Endpoint, sim: Option<SimConditions>) -> Box<dyn Transport + Send> {
    match sim.filter(SimConditions::is_active) {
        Some(s) => Box::new(Conditioned::new(ep, s.link_conditions())),
        None => Box::new(ep),
    }
}

/// Options of [`connect`].
#[derive(Clone, Debug)]
pub struct ConnectOptions {
    /// `host:port`, `wss://` URL for WSS, or `https://` URL for WT.
    pub addr: String,
    pub kind: TransportKind,
    pub trust: Trust,
    /// TLS name override. WSS/WT default to URL host; WT rejects overrides. QUIC keeps localhost.
    pub server_name: String,
    pub sim: Option<SimConditions>,
    pub net: NetConfig,
}

impl ConnectOptions {
    pub fn new(addr: impl Into<String>, kind: TransportKind, trust: Trust) -> Self {
        Self {
            addr: addr.into(), kind, trust,
            server_name: if matches!(kind, TransportKind::Wss | TransportKind::Wt) { String::new() } else { "localhost".into() },
            sim: None, net: NetConfig::default(),
        }
    }
}

fn resolve(addr: &str) -> Result<SocketAddr, String> {
    let all: Vec<SocketAddr> = addr.to_socket_addrs().map_err(|e| format!("cannot resolve '{addr}': {e}"))?.collect();
    all.iter().copied().find(SocketAddr::is_ipv4).or_else(|| all.first().copied()).ok_or_else(|| format!("'{addr}' has no address"))
}

/// Starts connecting. Returns at once; the link reports `Connected` (or
/// `Disconnected`) through its events.
pub fn connect(opts: &ConnectOptions) -> Result<NetLink, String> {
    let ep = match opts.kind {
        TransportKind::Quic => {
            let trust = match &opts.trust {
                Trust::Fingerprint(fp) => QuicTrust::Sha256Fingerprint(*fp),
                Trust::InsecureDev => QuicTrust::DangerousSkipVerificationDevOnly,
                Trust::PemFile(p) => QuicTrust::PemFile(p.clone()),
                Trust::WebPki => QuicTrust::WebPki,
            };
            Endpoint::connect_quic(resolve(&opts.addr)?, &opts.server_name, trust, opts.net.clone())
        }
        TransportKind::Ws => Endpoint::connect_ws(&format!("ws://{}/", opts.addr), opts.net.clone()),
        TransportKind::Wt => {
            if !opts.server_name.is_empty() {
                return Err("WT verifies the URL host; server_name override is unsupported".into());
            }
            let trust = match &opts.trust {
                Trust::WebPki => QuicTrust::WebPki,
                Trust::PemFile(path) => QuicTrust::PemFile(path.clone()),
                _ => return Err("WT requires WebPki or PEM CA trust; fingerprint/insecure trust is QUIC-only".into()),
            };
            let url = if opts.addr.starts_with("https://") { opts.addr.clone() } else { format!("https://{}/", opts.addr) };
            Endpoint::connect_wt(&url, trust, opts.net.clone())
        }
        TransportKind::Wss => {
            let trust = match &opts.trust {
                Trust::WebPki => QuicTrust::WebPki,
                Trust::PemFile(path) => QuicTrust::PemFile(path.clone()),
                _ => return Err("WSS requires WebPki or PEM CA trust; fingerprint/insecure trust is QUIC-only".into()),
            };
            let url = if opts.addr.starts_with("wss://") { opts.addr.clone() } else { format!("wss://{}/", opts.addr) };
            Endpoint::connect_wss(&url, (!opts.server_name.is_empty()).then_some(opts.server_name.as_str()), trust, opts.net.clone())
        }
    }
    .map_err(|e| if matches!(opts.kind, TransportKind::Wss | TransportKind::Wt) { format!("secure transport connect: {e}") } else { format!("connect to {}: {e}", opts.addr) })?;
    let conn = ep.client_conn().ok_or("client endpoint has no connection")?;
    Ok(NetLink::new_boxed(condition(ep, opts.sim), conn))
}

/// Where the server certificate comes from (QUIC).
#[derive(Clone, Debug)]
pub enum Tls {
    /// Generate a self-signed certificate at startup, for `localhost`, the
    /// loopback addresses, the bind address and `extra_names`.
    SelfSigned { extra_names: Vec<String> },
    Pem { cert_chain: PathBuf, private_key: PathBuf },
}

/// Options of [`listen`].
#[derive(Clone, Debug)]
pub struct ListenOptions {
    pub bind: SocketAddr,
    pub kind: TransportKind,
    pub tls: Tls,
    pub sim: Option<SimConditions>,
    pub net: NetConfig,
    /// QUIC only: the same UDP port also answers browsers over WebTransport
    /// (ALPN `h3`). A generated certificate is then a short-lived P-256 one
    /// that `serverCertificateHashes` accepts.
    pub webtransport: bool,
    /// Also accept plain WebSocket clients (browsers without WebTransport, or
    /// where UDP is blocked) on this TCP address, in the same server.
    pub ws_bind: Option<SocketAddr>,
    /// Also accept `wss://` WebSocket clients on this TCP address, with the same
    /// certificate as QUIC (`Tls::Pem` for browsers on an `https://` page).
    pub wss_bind: Option<SocketAddr>,
}

impl ListenOptions {
    pub fn new(bind: SocketAddr, kind: TransportKind) -> Self {
        Self {
            bind,
            kind,
            tls: Tls::SelfSigned { extra_names: Vec::new() },
            sim: None,
            net: NetConfig::default(),
            webtransport: false,
            ws_bind: None,
            wss_bind: None,
        }
    }
}

/// Starts a server endpoint.
pub fn listen(opts: &ListenOptions) -> Result<NetEndpoint, String> {
    let mut ws_addr = None;
    let mut ep = match opts.kind {
        TransportKind::Quic => {
            let tls = match &opts.tls {
                Tls::SelfSigned { extra_names } => {
                    let mut names: Vec<String> = vec!["localhost".into(), "127.0.0.1".into(), "::1".into()];
                    if !opts.bind.ip().is_unspecified() {
                        names.push(opts.bind.ip().to_string());
                    }
                    names.extend(extra_names.iter().cloned());
                    names.dedup();
                    if opts.webtransport {
                        QuicServerTls::SelfSignedWebTransport { names }
                    } else {
                        QuicServerTls::SelfSigned { names }
                    }
                }
                Tls::Pem { cert_chain, private_key } => {
                    QuicServerTls::PemFiles { cert_chain: cert_chain.clone(), private_key: private_key.clone() }
                }
            };
            Endpoint::listen_quic_with(opts.bind, tls, opts.net.clone(), opts.webtransport)
        }
        TransportKind::Ws => Endpoint::listen_ws(opts.bind, opts.net.clone()),
        TransportKind::Wt => return Err("WT listen requires TransportKind::Quic with webtransport enabled".into()),
        TransportKind::Wss => return Err("WSS listen requires TransportKind::Quic with ListenOptions::wss_bind and TLS identity".into()),
    }
    .map_err(|e| format!("listen on {}: {e}", opts.bind))?;
    if let Some(bind) = opts.ws_bind {
        ws_addr = Some(ep.add_ws_listener(bind).map_err(|e| format!("listen (WebSocket) on {bind}: {e}"))?);
    }
    let mut wss_addr = None;
    if let Some(bind) = opts.wss_bind {
        wss_addr = Some(ep.add_wss_listener(bind).map_err(|e| format!("listen (secure WebSocket) on {bind}: {e}"))?);
    }
    let (addr, fp) = (ep.local_addr(), ep.server_cert_sha256());
    let mut out = NetEndpoint::new(condition(ep, opts.sim), addr, fp);
    out.set_ws_addr(ws_addr, wss_addr);
    Ok(out)
}

/// Parses a SHA-256 fingerprint: 64 hex digits, colons and spaces allowed.
pub fn parse_fingerprint(s: &str) -> Result<[u8; 32], String> {
    let digits: Vec<u8> = s.bytes().filter(|b| !matches!(b, b':' | b' ' | b'-')).collect();
    if digits.len() != 64 {
        return Err(format!("a SHA-256 fingerprint has 64 hex digits, got {}", digits.len()));
    }
    let nibble = |b: u8| match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(format!("'{}' is not a hex digit", b as char)),
    };
    let mut out = [0u8; 32];
    for (i, pair) in digits.chunks(2).enumerate() {
        out[i] = nibble(pair[0])? << 4 | nibble(pair[1])?;
    }
    Ok(out)
}

/// Lowercase hex of a fingerprint (no separators).
pub fn format_fingerprint(fp: &[u8; 32]) -> String {
    fp.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wss_is_client_only_and_rejects_lax_trust_without_url_disclosure() {
        assert_eq!("wss".parse::<TransportKind>().unwrap(), TransportKind::Wss);
        let sentinel = "sensitive-sentinel";
        for trust in [Trust::InsecureDev, Trust::Fingerprint([0; 32]), Trust::PemFile(PathBuf::from(sentinel))] {
            let opts = ConnectOptions::new(format!("wss://localhost/{sentinel}?secret={sentinel}"), TransportKind::Wss, trust);
            let error = match connect(&opts) { Ok(_) => panic!("expected rejection"), Err(e) => e };
            assert!(!error.contains(sentinel), "{error}");
        }
        let error = match listen(&ListenOptions::new("127.0.0.1:0".parse().unwrap(), TransportKind::Wss)) {
            Ok(_) => panic!("standalone WSS listener unexpectedly accepted"), Err(e) => e,
        };
        assert!(error.contains("wss_bind"));
        assert!(error.contains("Quic"));
    }

    #[test]
    fn wt_is_verified_client_only_without_url_disclosure() {
        assert_eq!("wt".parse::<TransportKind>().unwrap(), TransportKind::Wt);
        for trust in [Trust::InsecureDev, Trust::Fingerprint([0; 32]), Trust::PemFile(PathBuf::from("private-sentinel"))] {
            let opts = ConnectOptions::new("https://localhost/private-sentinel?secret=private-sentinel", TransportKind::Wt, trust);
            assert!(opts.server_name.is_empty());
            let error = match connect(&opts) { Ok(_) => panic!("expected rejection"), Err(e) => e };
            assert!(!error.contains("private-sentinel"));
        }
        let mut opts = ConnectOptions::new("https://localhost", TransportKind::Wt, Trust::WebPki);
        opts.server_name = "override.invalid".into();
        assert!(connect(&opts).is_err());
        assert!(listen(&ListenOptions::new("127.0.0.1:0".parse().unwrap(), TransportKind::Wt)).is_err());
    }

    #[test]
    fn fingerprint_roundtrip() {
        let fp: [u8; 32] = std::array::from_fn(|i| (i * 7 + 1) as u8);
        let text = format_fingerprint(&fp);
        assert_eq!(text.len(), 64);
        assert_eq!(parse_fingerprint(&text).unwrap(), fp);
        let colons: Vec<String> = fp.iter().map(|b| format!("{b:02X}")).collect();
        assert_eq!(parse_fingerprint(&colons.join(":")).unwrap(), fp);
        assert!(parse_fingerprint("abcd").is_err());
        assert!(parse_fingerprint(&"zz".repeat(32)).is_err());
    }
}
