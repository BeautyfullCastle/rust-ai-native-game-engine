//! A deterministic in-memory network for tests.
//!
//! [`SimNet`] owns a virtual clock (microseconds) and a seeded RNG. It hands
//! out [`SimLink`]s (client side, implements [`Link`]) and one
//! [`SimEndpoint`] (server side, implements [`Endpoint`]). Each direction of
//! each connection has [`LinkParams`]: one-way latency, jitter, loss,
//! duplication and reordering. The reliable channel never loses or reorders
//! (a "lost" reliable message is delayed by a retransmit round trip
//! instead); the unreliable channel does all of it.
//!
//! Nothing here reads a wall clock: time moves only with
//! [`SimNet::advance_us`] / [`SimNet::set_now_us`], so a run is a pure
//! function of the seed and the calls made.
use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

use crate::net::{Channel, ConnId, Endpoint, Link, LinkEvent, ServerEvent, UNRELIABLE_MTU};

/// Parameters of one direction of a connection. Probabilities are in parts
/// per million.
#[derive(Clone, Copy, Debug, Default)]
pub struct LinkParams {
    /// Base one-way delay.
    pub latency_us: u64,
    /// Each message gets an extra uniform delay in `0..=jitter_us`.
    pub jitter_us: u64,
    /// Unreliable messages only: chance a message never arrives.
    pub loss_ppm: u32,
    /// Unreliable messages only: chance a message arrives twice.
    pub dup_ppm: u32,
    /// Unreliable messages only: chance a message gets `reorder_extra_us`
    /// more delay (so it arrives after later ones).
    pub reorder_ppm: u32,
    pub reorder_extra_us: u64,
}

impl LinkParams {
    /// Latency and jitter only.
    pub fn new(latency_us: u64, jitter_us: u64) -> Self {
        Self { latency_us, jitter_us, ..Self::default() }
    }

    pub fn with_loss_ppm(mut self, loss_ppm: u32) -> Self {
        self.loss_ppm = loss_ppm;
        self
    }

    pub fn with_dup_ppm(mut self, dup_ppm: u32) -> Self {
        self.dup_ppm = dup_ppm;
        self
    }

    pub fn with_reorder(mut self, reorder_ppm: u32, extra_us: u64) -> Self {
        self.reorder_ppm = reorder_ppm;
        self.reorder_extra_us = extra_us;
        self
    }
}

/// Both directions of one connection.
#[derive(Clone, Copy, Debug, Default)]
pub struct PathParams {
    /// Client to server.
    pub up: LinkParams,
    /// Server to client.
    pub down: LinkParams,
}

impl PathParams {
    /// The same parameters both ways.
    pub fn symmetric(p: LinkParams) -> Self {
        Self { up: p, down: p }
    }
}

/// Counters over everything that went through the network.
#[derive(Clone, Copy, Debug, Default)]
pub struct NetStats {
    pub unreliable_sent: u64,
    pub unreliable_lost: u64,
    pub unreliable_duplicated: u64,
    /// Unreliable messages over [`UNRELIABLE_MTU`]: dropped (a bug in the
    /// sender).
    pub oversize_dropped: u64,
    pub reliable_sent: u64,
    pub bytes_up: u64,
    pub bytes_down: u64,
}

/// Small seeded generator (SplitMix64).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn chance_ppm(&mut self, ppm: u32) -> bool {
        ppm > 0 && (self.next() % 1_000_000) < u64::from(ppm)
    }
    fn upto(&mut self, max_inclusive: u64) -> u64 {
        if max_inclusive == 0 {
            0
        } else {
            self.next() % (max_inclusive + 1)
        }
    }
}

enum Ev {
    Connected,
    Disconnected,
    Message(Channel, Vec<u8>),
}

struct Conn {
    params: PathParams,
    open: bool,
    /// Latest delivery time handed out on the reliable channel, per
    /// direction (keeps the reliable channel ordered).
    last_reliable_up: u64,
    last_reliable_down: u64,
    /// Events for the client of this connection, ordered by delivery time.
    to_client: BTreeMap<(u64, u64), Ev>,
}

struct Inner {
    now_us: u64,
    rng: Rng,
    seq: u64,
    next_conn: u32,
    conns: BTreeMap<u32, Conn>,
    /// Events for the server, ordered by delivery time.
    to_server: BTreeMap<(u64, u64), (u32, Ev)>,
    stats: NetStats,
}

impl Inner {
    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    /// Queues a message `to_server` (or to the client) with the delays and
    /// faults of the path.
    fn transmit(&mut self, conn: u32, to_server: bool, channel: Channel, data: &[u8]) {
        let now = self.now_us;
        let Some(c) = self.conns.get(&conn) else { return };
        if !c.open {
            return;
        }
        let p = if to_server { c.params.up } else { c.params.down };
        if to_server {
            self.stats.bytes_up += data.len() as u64;
        } else {
            self.stats.bytes_down += data.len() as u64;
        }
        match channel {
            Channel::Unreliable => {
                self.stats.unreliable_sent += 1;
                if data.len() > UNRELIABLE_MTU {
                    self.stats.oversize_dropped += 1;
                    return;
                }
                if self.rng.chance_ppm(p.loss_ppm) {
                    self.stats.unreliable_lost += 1;
                    return;
                }
                let copies = if self.rng.chance_ppm(p.dup_ppm) {
                    self.stats.unreliable_duplicated += 1;
                    2
                } else {
                    1
                };
                for _ in 0..copies {
                    let mut delay = p.latency_us + self.rng.upto(p.jitter_us);
                    if self.rng.chance_ppm(p.reorder_ppm) {
                        delay += p.reorder_extra_us;
                    }
                    self.push(conn, to_server, now + delay, Ev::Message(channel, data.to_vec()));
                }
            }
            Channel::Reliable => {
                self.stats.reliable_sent += 1;
                let mut delay = p.latency_us + self.rng.upto(p.jitter_us);
                if self.rng.chance_ppm(p.loss_ppm) {
                    // Lost on the wire: the transport retransmits after
                    // about a round trip.
                    delay += 2 * p.latency_us + p.jitter_us;
                }
                let c = self.conns.get_mut(&conn).unwrap();
                let last = if to_server { &mut c.last_reliable_up } else { &mut c.last_reliable_down };
                let at = (now + delay).max(*last);
                *last = at;
                self.push(conn, to_server, at, Ev::Message(channel, data.to_vec()));
            }
        }
    }

    fn push(&mut self, conn: u32, to_server: bool, at: u64, ev: Ev) {
        let seq = self.next_seq();
        if to_server {
            self.to_server.insert((at, seq), (conn, ev));
        } else if let Some(c) = self.conns.get_mut(&conn) {
            c.to_client.insert((at, seq), ev);
        }
    }
}

/// The shared network. Cloning gives another handle to the same network.
#[derive(Clone)]
pub struct SimNet(Rc<RefCell<Inner>>);

impl SimNet {
    pub fn new(seed: u64) -> Self {
        Self(Rc::new(RefCell::new(Inner {
            now_us: 0,
            rng: Rng(seed),
            seq: 0,
            next_conn: 1,
            conns: BTreeMap::new(),
            to_server: BTreeMap::new(),
            stats: NetStats::default(),
        })))
    }

    pub fn now_us(&self) -> u64 {
        self.0.borrow().now_us
    }

    pub fn set_now_us(&self, now_us: u64) {
        let mut i = self.0.borrow_mut();
        assert!(now_us >= i.now_us, "virtual time cannot go back");
        i.now_us = now_us;
    }

    pub fn advance_us(&self, dt_us: u64) {
        self.0.borrow_mut().now_us += dt_us;
    }

    pub fn stats(&self) -> NetStats {
        self.0.borrow().stats
    }

    /// The server side.
    pub fn endpoint(&self) -> SimEndpoint {
        SimEndpoint(self.clone())
    }

    /// Opens a connection with `params`. The client sees `Connected` after
    /// one round trip (a handshake), the server after one way.
    pub fn connect(&self, params: PathParams) -> SimLink {
        let mut i = self.0.borrow_mut();
        let id = i.next_conn;
        i.next_conn += 1;
        let now = i.now_us;
        i.conns.insert(
            id,
            Conn { params, open: true, last_reliable_up: 0, last_reliable_down: 0, to_client: BTreeMap::new() },
        );
        let up = params.up.latency_us;
        let down = params.down.latency_us;
        i.push(id, true, now + up, Ev::Connected);
        i.push(id, false, now + up + down, Ev::Connected);
        SimLink { net: self.clone(), conn: id }
    }

    /// Changes the fault parameters of an open connection from now on
    /// (a network that gets worse or better).
    pub fn set_params(&self, conn: ConnId, params: PathParams) {
        if let Some(c) = self.0.borrow_mut().conns.get_mut(&conn.0) {
            c.params = params;
        }
    }

    /// Cuts the connection as a network failure would: both sides get
    /// `Disconnected` after their one-way latency, and nothing sent after
    /// this call is delivered.
    pub fn cut(&self, link: &SimLink) {
        self.close_conn(link.conn, true);
    }

    /// [`cut`](Self::cut) by connection id.
    pub fn cut_conn(&self, conn: ConnId) {
        self.close_conn(conn.0, true);
    }

    fn close_conn(&self, conn: u32, notify_server: bool) {
        let mut i = self.0.borrow_mut();
        let now = i.now_us;
        let Some(c) = i.conns.get_mut(&conn) else { return };
        if !c.open {
            return;
        }
        c.open = false;
        let (up, down) = (c.params.up.latency_us, c.params.down.latency_us);
        if notify_server {
            i.push(conn, true, now + up, Ev::Disconnected);
        }
        i.push(conn, false, now + down, Ev::Disconnected);
    }
}

/// Client end of a simulated connection.
pub struct SimLink {
    net: SimNet,
    conn: u32,
}

impl SimLink {
    /// The connection's id at the server endpoint.
    pub fn conn_id(&self) -> ConnId {
        ConnId(self.conn)
    }
}

impl Link for SimLink {
    fn send(&mut self, channel: Channel, data: &[u8]) {
        self.net.0.borrow_mut().transmit(self.conn, true, channel, data);
    }

    fn poll(&mut self) -> Option<LinkEvent> {
        let mut i = self.net.0.borrow_mut();
        let now = i.now_us;
        let c = i.conns.get_mut(&self.conn)?;
        let key = *c.to_client.keys().next()?;
        if key.0 > now {
            return None;
        }
        Some(match c.to_client.remove(&key)? {
            Ev::Connected => LinkEvent::Connected,
            Ev::Disconnected => LinkEvent::Disconnected,
            Ev::Message(channel, data) => LinkEvent::Message { channel, data },
        })
    }

    fn close(&mut self) {
        self.net.close_conn(self.conn, true);
    }
}

/// Server end: every connection made with [`SimNet::connect`].
pub struct SimEndpoint(SimNet);

impl Endpoint for SimEndpoint {
    fn send(&mut self, conn: ConnId, channel: Channel, data: &[u8]) {
        self.0 .0.borrow_mut().transmit(conn.0, false, channel, data);
    }

    fn poll(&mut self) -> Option<ServerEvent> {
        let mut i = self.0 .0.borrow_mut();
        let now = i.now_us;
        let key = *i.to_server.keys().next()?;
        if key.0 > now {
            return None;
        }
        let (conn, ev) = i.to_server.remove(&key)?;
        let conn = ConnId(conn);
        Some(match ev {
            Ev::Connected => ServerEvent::Connected(conn),
            Ev::Disconnected => ServerEvent::Disconnected(conn),
            Ev::Message(channel, data) => ServerEvent::Message { conn, channel, data },
        })
    }

    fn disconnect(&mut self, conn: ConnId) {
        self.0.close_conn(conn.0, false);
    }
}

/// A queue of `(time, item)` that releases items once a virtual clock has
/// reached their time. Handy for scripted test events.
#[derive(Default)]
pub struct Schedule<T> {
    items: VecDeque<(u64, T)>,
}

impl<T> Schedule<T> {
    pub fn new() -> Self {
        Self { items: VecDeque::new() }
    }

    pub fn at(&mut self, time_us: u64, item: T) {
        let pos = self.items.iter().position(|(t, _)| *t > time_us).unwrap_or(self.items.len());
        self.items.insert(pos, (time_us, item));
    }

    /// The next item due at or before `now_us`.
    pub fn pop_due(&mut self, now_us: u64) -> Option<T> {
        if self.items.front().is_some_and(|(t, _)| *t <= now_us) {
            self.items.pop_front().map(|(_, item)| item)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(ep: &mut SimEndpoint) -> Vec<ServerEvent> {
        std::iter::from_fn(|| ep.poll()).collect()
    }

    #[test]
    fn latency_and_order() {
        let net = SimNet::new(1);
        let mut ep = net.endpoint();
        let mut link = net.connect(PathParams::symmetric(LinkParams::new(10_000, 0)));
        assert!(drain(&mut ep).is_empty());
        net.advance_us(10_000);
        assert_eq!(drain(&mut ep), vec![ServerEvent::Connected(link.conn_id())]);
        assert!(link.poll().is_none());
        net.advance_us(10_000);
        assert_eq!(link.poll(), Some(LinkEvent::Connected));
        link.send(Channel::Reliable, b"a");
        link.send(Channel::Reliable, b"b");
        net.advance_us(9_999);
        assert!(drain(&mut ep).is_empty());
        net.advance_us(1);
        let got = drain(&mut ep);
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn unreliable_faults_are_seeded() {
        let run = |seed| {
            let net = SimNet::new(seed);
            let mut ep = net.endpoint();
            let mut link = net.connect(PathParams::symmetric(
                LinkParams::new(1000, 5000).with_loss_ppm(200_000).with_dup_ppm(100_000),
            ));
            net.advance_us(100_000);
            let _ = drain(&mut ep);
            for i in 0..200u8 {
                link.send(Channel::Unreliable, &[i]);
            }
            net.advance_us(1_000_000);
            drain(&mut ep).len()
        };
        assert_eq!(run(5), run(5));
        let n = run(5);
        assert!(n > 100 && n < 240, "{n}");
    }

    #[test]
    fn reliable_channel_stays_ordered_under_loss_and_jitter() {
        let net = SimNet::new(3);
        let mut ep = net.endpoint();
        let mut link =
            net.connect(PathParams::symmetric(LinkParams::new(5000, 20_000).with_loss_ppm(300_000)));
        net.advance_us(100_000);
        let _ = drain(&mut ep);
        for i in 0..100u8 {
            link.send(Channel::Reliable, &[i]);
            net.advance_us(1000);
        }
        net.advance_us(1_000_000);
        let got: Vec<u8> = drain(&mut ep)
            .into_iter()
            .filter_map(|e| if let ServerEvent::Message { data, .. } = e { Some(data[0]) } else { None })
            .collect();
        assert_eq!(got, (0..100u8).collect::<Vec<_>>());
    }

    #[test]
    fn oversize_unreliable_is_dropped_and_counted() {
        let net = SimNet::new(1);
        let mut link = net.connect(PathParams::default());
        link.send(Channel::Unreliable, &vec![0u8; UNRELIABLE_MTU + 1]);
        assert_eq!(net.stats().oversize_dropped, 1);
    }

    #[test]
    fn cut_notifies_both_sides() {
        let net = SimNet::new(1);
        let mut ep = net.endpoint();
        let mut link = net.connect(PathParams::symmetric(LinkParams::new(1000, 0)));
        net.advance_us(5000);
        let _ = drain(&mut ep);
        while link.poll().is_some() {}
        net.cut(&link);
        link.send(Channel::Reliable, b"x");
        net.advance_us(5000);
        assert_eq!(drain(&mut ep), vec![ServerEvent::Disconnected(link.conn_id())]);
        assert_eq!(link.poll(), Some(LinkEvent::Disconnected));
    }
}
