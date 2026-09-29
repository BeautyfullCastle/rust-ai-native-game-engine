#![allow(dead_code)]
//! Shared helpers for the loopback tests.

use orr_net::{Channel, ConnId, DisconnectReason, Endpoint, Event, NetConfig, QuicServerTls, QuicTrust, SendError};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub const WAIT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Quic,
    Ws,
}

pub struct Server {
    pub ep: Endpoint,
    pub backend: Backend,
    pub addr: SocketAddr,
    pub cert: Option<Vec<u8>>,
}

pub fn loopback() -> SocketAddr {
    ([127, 0, 0, 1], 0).into()
}

pub fn server(backend: Backend, cfg: NetConfig) -> Server {
    let (ep, cert) = match backend {
        Backend::Quic => {
            let ep = Endpoint::listen_quic(loopback(), QuicServerTls::dev_localhost(), cfg).unwrap();
            let cert = ep.server_cert_der().map(|c| c.to_vec());
            (ep, cert)
        }
        Backend::Ws => (Endpoint::listen_ws(loopback(), cfg).unwrap(), None),
    };
    let addr = ep.local_addr();
    Server { ep, backend, addr, cert }
}

pub fn connect_to(backend: Backend, addr: SocketAddr, cert: Option<&[u8]>, cfg: NetConfig) -> Endpoint {
    match backend {
        Backend::Quic => Endpoint::connect_quic(
            addr,
            "localhost",
            QuicTrust::CertDer(cert.expect("quic needs cert").to_vec()),
            cfg,
        )
        .unwrap(),
        Backend::Ws => Endpoint::connect_ws(&format!("ws://{addr}/"), cfg).unwrap(),
    }
}

pub fn client(s: &Server, cfg: NetConfig) -> Endpoint {
    connect_to(s.backend, s.addr, s.cert.as_deref(), cfg)
}

/// Waits for an event matching `pred`. Other events are skipped.
pub fn wait_for(ep: &mut Endpoint, timeout: Duration, mut pred: impl FnMut(&Event) -> bool) -> Option<Event> {
    let end = Instant::now() + timeout;
    loop {
        let left = end.checked_duration_since(Instant::now())?;
        let ev = ep.wait_event(left)?;
        if pred(&ev) {
            return Some(ev);
        }
    }
}

pub fn expect_connected(ep: &mut Endpoint) -> ConnId {
    match wait_for(ep, WAIT, |e| matches!(e, Event::Connected { .. })) {
        Some(Event::Connected { conn, .. }) => conn,
        other => panic!("expected Connected, got {other:?}"),
    }
}

/// Connects `client` to `server` and returns (server-side id, client-side id).
pub fn pair(server: &mut Endpoint, client: &mut Endpoint) -> (ConnId, ConnId) {
    let c = expect_connected(client);
    let s = expect_connected(server);
    (s, c)
}

pub fn expect_message(ep: &mut Endpoint) -> (ConnId, Channel, Vec<u8>) {
    match wait_for(ep, WAIT, |e| matches!(e, Event::Message { .. })) {
        Some(Event::Message { conn, channel, bytes }) => (conn, channel, bytes),
        other => panic!("expected Message, got {other:?}"),
    }
}

pub fn expect_disconnect(ep: &mut Endpoint, timeout: Duration) -> (ConnId, DisconnectReason) {
    match wait_for(ep, timeout, |e| matches!(e, Event::Disconnected { .. })) {
        Some(Event::Disconnected { conn, reason }) => (conn, reason),
        other => panic!("expected Disconnected, got {other:?}"),
    }
}

/// Sends, retrying while the queue is full.
pub fn send_retry(ep: &Endpoint, conn: ConnId, ch: Channel, bytes: &[u8]) {
    let end = Instant::now() + WAIT;
    loop {
        match ep.send(conn, ch, bytes) {
            Ok(()) => return,
            Err(SendError::Backpressure) if Instant::now() < end => thread::sleep(Duration::from_millis(1)),
            Err(e) => panic!("send failed: {e}"),
        }
    }
}

pub fn pattern(seq: u32, len: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(len.max(4));
    v.extend_from_slice(&seq.to_le_bytes());
    for i in 4..len {
        v.push((i as u32).wrapping_mul(31).wrapping_add(seq) as u8);
    }
    v.truncate(len.max(4));
    v
}

pub fn quick_cfg() -> NetConfig {
    NetConfig {
        keepalive_interval: Duration::from_millis(100),
        idle_timeout: Duration::from_millis(800),
        connect_timeout: Duration::from_secs(2),
        ..NetConfig::default()
    }
}

/// UDP forwarder between one client and a server, with a switch that silently drops everything.
pub struct UdpProxy {
    pub addr: SocketAddr,
    pub blackhole: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

pub fn udp_proxy(target: SocketAddr) -> UdpProxy {
    let front = UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = front.local_addr().unwrap();
    front.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
    let back = UdpSocket::bind("127.0.0.1:0").unwrap();
    back.connect(target).unwrap();
    back.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
    let blackhole = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let client: Arc<Mutex<Option<SocketAddr>>> = Arc::new(Mutex::new(None));
    {
        let (front, back, bh, stop, client) =
            (front.try_clone().unwrap(), back.try_clone().unwrap(), blackhole.clone(), stop.clone(), client.clone());
        thread::spawn(move || {
            let mut buf = vec![0u8; 65536];
            while !stop.load(Relaxed) {
                if let Ok((n, from)) = front.recv_from(&mut buf) {
                    *client.lock().unwrap() = Some(from);
                    if !bh.load(Relaxed) {
                        let _ = back.send(&buf[..n]);
                    }
                }
            }
        });
    }
    {
        let (bh, stop) = (blackhole.clone(), stop.clone());
        thread::spawn(move || {
            let mut buf = vec![0u8; 65536];
            while !stop.load(Relaxed) {
                if let Ok(n) = back.recv(&mut buf) {
                    let c = *client.lock().unwrap();
                    if let (false, Some(c)) = (bh.load(Relaxed), c) {
                        let _ = front.send_to(&buf[..n], c);
                    }
                }
            }
        });
    }
    UdpProxy { addr, blackhole, stop }
}

impl Drop for UdpProxy {
    fn drop(&mut self) {
        self.stop.store(true, Relaxed);
    }
}

/// TCP forwarder with a blackhole switch (bytes are read and discarded).
pub struct TcpProxy {
    pub addr: SocketAddr,
    pub blackhole: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

pub fn tcp_proxy(target: SocketAddr) -> TcpProxy {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let blackhole = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let (bh, st) = (blackhole.clone(), stop.clone());
    thread::spawn(move || {
        while !st.load(Relaxed) {
            let Ok((a, _)) = listener.accept() else {
                thread::sleep(Duration::from_millis(5));
                continue;
            };
            a.set_nonblocking(false).unwrap();
            let b = TcpStream::connect(target).unwrap();
            for (mut from, mut to) in [(a.try_clone().unwrap(), b.try_clone().unwrap()), (b, a)] {
                let (bh, st) = (bh.clone(), st.clone());
                from.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
                thread::spawn(move || {
                    let mut buf = vec![0u8; 65536];
                    while !st.load(Relaxed) {
                        match from.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                if !bh.load(Relaxed) && to.write_all(&buf[..n]).is_err() {
                                    break;
                                }
                            }
                            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                            Err(_) => break,
                        }
                    }
                });
            }
        }
    });
    TcpProxy { addr, blackhole, stop }
}

impl Drop for TcpProxy {
    fn drop(&mut self) {
        self.stop.store(true, Relaxed);
    }
}
