//! Network conditioner (feature `conditioner`): adds latency, jitter and loss
//! to any [`Transport`], for manual testing over real sockets.
//!
//! Wrap the endpoint on one side. Conditions apply to **each direction** of
//! that endpoint: outgoing messages are delayed before they reach the inner
//! transport, and incoming message events are delayed before the caller sees
//! them. The added RTT is therefore about `2 * latency`.
//!
//! * `Unreliable` messages are dropped with probability `loss` and may be
//!   reordered by jitter.
//! * `Reliable` messages are never dropped and never reordered. A "loss" hit
//!   models a retransmit instead, by adding `2 * latency` of delay.
//! * `Connected` / `Disconnected` are not delayed, except that `Disconnected`
//!   waits behind earlier delayed messages of its connection.
//!
//! The wrapper has no thread. Delayed items move only when the caller calls
//! `poll_event`, `send` or `stats`, which a game loop does every frame.

use crate::endpoint::Transport;
use crate::stats::ConnStats;
use crate::{Channel, ConnId, Event, SendError};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::time::{Duration, Instant};

/// Simulated link quality. See the module docs.
#[derive(Clone, Copy, Debug)]
pub struct LinkConditions {
    /// Fixed one-way delay per direction.
    pub latency: Duration,
    /// Extra random delay, uniform in `0..=jitter`.
    pub jitter: Duration,
    /// Loss probability in `0.0..=1.0`.
    pub loss: f32,
    /// PRNG seed, so runs can be repeated.
    pub seed: u64,
}

impl Default for LinkConditions {
    fn default() -> Self {
        LinkConditions { latency: Duration::ZERO, jitter: Duration::ZERO, loss: 0.0, seed: 1 }
    }
}

struct Rng(u64);
impl Rng {
    /// Independent stream `n` of `seed` (splitmix64 of the mixed value).
    fn stream(seed: u64, n: u64) -> Rng {
        let mut z = seed.wrapping_add(n.wrapping_mul(0x9E37_79B9_7F4A_7C15)).wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        Rng((z ^ (z >> 31)).max(1))
    }
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
}

struct Item<T> {
    release: Instant,
    seq: u64,
    item: T,
}
impl<T> PartialEq for Item<T> {
    fn eq(&self, o: &Self) -> bool {
        (self.release, self.seq) == (o.release, o.seq)
    }
}
impl<T> Eq for Item<T> {}
impl<T> PartialOrd for Item<T> {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl<T> Ord for Item<T> {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        (self.release, self.seq).cmp(&(o.release, o.seq))
    }
}

struct DirRng {
    loss: Rng,
    jitter: Rng,
}

const OUT: usize = 0;
const IN: usize = 1;

type Queue<T> = BinaryHeap<Reverse<Item<T>>>;

/// A [`Transport`] with simulated network conditions.
pub struct Conditioned<T: Transport> {
    inner: T,
    cond: LinkConditions,
    /// Loss and jitter draws have their own stream per direction, so a send
    /// never shifts the loss decisions of the receive side (and the reverse).
    rng: [DirRng; 2],
    seq: u64,
    out_q: Queue<(ConnId, Channel, Vec<u8>)>,
    in_q: Queue<Event>,
    /// Latest release time handed out per connection and direction, to keep order.
    out_last: HashMap<ConnId, Instant>,
    in_last: HashMap<ConnId, Instant>,
    /// Closes waiting for delayed sends of their connection.
    closes: Vec<(ConnId, Instant)>,
}

impl<T: Transport> Conditioned<T> {
    pub fn new(inner: T, cond: LinkConditions) -> Self {
        Conditioned {
            inner,
            cond,
            rng: [
                DirRng { loss: Rng::stream(cond.seed, 0), jitter: Rng::stream(cond.seed, 1) },
                DirRng { loss: Rng::stream(cond.seed, 2), jitter: Rng::stream(cond.seed, 3) },
            ],
            seq: 0,
            out_q: BinaryHeap::new(),
            in_q: BinaryHeap::new(),
            out_last: HashMap::new(),
            in_last: HashMap::new(),
            closes: Vec::new(),
        }
    }

    pub fn conditions(&self) -> LinkConditions {
        self.cond
    }

    pub fn set_conditions(&mut self, cond: LinkConditions) {
        self.cond = cond;
    }

    pub fn inner(&self) -> &T {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    /// Release time for a new item, or `None` when it is lost.
    fn schedule(&mut self, dir: usize, channel: Channel, last: Option<Instant>) -> Option<Instant> {
        let lost = self.cond.loss > 0.0 && self.rng[dir].loss.unit() < self.cond.loss;
        let mut delay = self.cond.latency;
        if !self.cond.jitter.is_zero() {
            delay += self.cond.jitter.mul_f32(self.rng[dir].jitter.unit());
        }
        let mut release = Instant::now() + delay;
        match channel {
            Channel::Unreliable if lost => return None,
            Channel::Reliable => {
                if lost {
                    release += self.cond.latency * 2;
                }
                if let Some(l) = last {
                    release = release.max(l);
                }
            }
            Channel::Unreliable => {}
        }
        Some(release)
    }

    fn pump(&mut self) {
        let now = Instant::now();
        while let Some(Reverse(top)) = self.out_q.peek() {
            if top.release > now {
                break;
            }
            let Reverse(it) = self.out_q.pop().unwrap();
            let (conn, ch, bytes) = it.item;
            let _ = self.inner.send(conn, ch, &bytes);
        }
        let mut i = 0;
        while i < self.closes.len() {
            if self.closes[i].1 <= now {
                let (conn, _) = self.closes.swap_remove(i);
                self.inner.close(conn);
            } else {
                i += 1;
            }
        }
        while let Some(ev) = self.inner.poll_event() {
            let now = Instant::now();
            let (conn, release) = match &ev {
                Event::Message { conn, channel, .. } => {
                    let last = self.in_last.get(conn).copied();
                    match self.schedule(IN, *channel, last) {
                        Some(r) => (*conn, r),
                        None => continue,
                    }
                }
                Event::Connected { conn, .. } => (*conn, now),
                Event::Disconnected { conn, .. } => {
                    (*conn, self.in_last.get(conn).copied().unwrap_or(now).max(now))
                }
            };
            // Reliable and control events keep order; unreliable may not, but
            // must not push the per-connection order marker backwards.
            let is_unreliable = matches!(&ev, Event::Message { channel: Channel::Unreliable, .. });
            if !is_unreliable {
                self.in_last.insert(conn, release);
            }
            self.seq += 1;
            self.in_q.push(Reverse(Item { release, seq: self.seq, item: ev }));
        }
    }
}

impl<T: Transport> Transport for Conditioned<T> {
    fn poll_event(&mut self) -> Option<Event> {
        self.pump();
        let now = Instant::now();
        match self.in_q.peek() {
            Some(Reverse(top)) if top.release <= now => {
                let Reverse(it) = self.in_q.pop().unwrap();
                if let Event::Disconnected { conn, .. } = &it.item {
                    self.in_last.remove(conn);
                    self.out_last.remove(conn);
                }
                Some(it.item)
            }
            _ => None,
        }
    }

    fn send(&mut self, conn: ConnId, channel: Channel, bytes: &[u8]) -> Result<(), SendError> {
        let last = self.out_last.get(&conn).copied();
        if let Some(release) = self.schedule(OUT, channel, last) {
            if channel == Channel::Reliable {
                self.out_last.insert(conn, release);
            }
            self.seq += 1;
            self.out_q.push(Reverse(Item { release, seq: self.seq, item: (conn, channel, bytes.to_vec()) }));
        }
        self.pump();
        Ok(())
    }

    fn close(&mut self, conn: ConnId) {
        // Delayed sends of this connection must reach the inner transport first.
        match self.out_last.get(&conn).copied() {
            Some(t) if t > Instant::now() => self.closes.push((conn, t)),
            _ => {
                self.pump();
                self.inner.close(conn);
            }
        }
    }

    fn stats(&self, conn: ConnId) -> Option<ConnStats> {
        self.inner.stats(conn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// In-memory transport: every send is echoed back as a message event.
    #[derive(Default)]
    struct Echo {
        events: VecDeque<Event>,
    }
    impl Transport for Echo {
        fn poll_event(&mut self) -> Option<Event> {
            self.events.pop_front()
        }
        fn send(&mut self, conn: ConnId, channel: Channel, bytes: &[u8]) -> Result<(), SendError> {
            self.events.push_back(Event::Message { conn, channel, bytes: bytes.to_vec() });
            Ok(())
        }
        fn close(&mut self, _conn: ConnId) {}
        fn stats(&self, _conn: ConnId) -> Option<ConnStats> {
            None
        }
    }

    fn cond(latency_ms: u64, jitter_ms: u64, loss: f32) -> LinkConditions {
        LinkConditions {
            latency: Duration::from_millis(latency_ms),
            jitter: Duration::from_millis(jitter_ms),
            loss,
            seed: 7,
        }
    }

    fn drain_for(t: &mut Conditioned<Echo>, d: Duration) -> Vec<Event> {
        let end = Instant::now() + d;
        let mut v = Vec::new();
        while Instant::now() < end {
            while let Some(e) = t.poll_event() {
                v.push(e);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        v
    }

    #[test]
    fn adds_latency_in_both_directions() {
        let mut t = Conditioned::new(Echo::default(), cond(30, 0, 0.0));
        let start = Instant::now();
        t.send(1, Channel::Reliable, b"x").unwrap();
        let mut got = None;
        while start.elapsed() < Duration::from_secs(2) {
            if let Some(e) = t.poll_event() {
                got = Some(e);
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(matches!(got, Some(Event::Message { .. })));
        assert!(start.elapsed() >= Duration::from_millis(60), "{:?}", start.elapsed());
    }

    #[test]
    fn reliable_keeps_order_under_jitter_and_loss() {
        let mut t = Conditioned::new(Echo::default(), cond(5, 20, 0.5));
        for i in 0..100u8 {
            t.send(1, Channel::Reliable, &[i]).unwrap();
        }
        let ev = drain_for(&mut t, Duration::from_millis(700));
        let got: Vec<u8> = ev
            .iter()
            .map(|e| match e {
                Event::Message { bytes, .. } => bytes[0],
                _ => panic!(),
            })
            .collect();
        assert_eq!(got, (0..100).collect::<Vec<u8>>());
    }

    #[test]
    fn unreliable_loses_about_the_configured_share() {
        let mut t = Conditioned::new(Echo::default(), cond(0, 0, 0.3));
        for i in 0..2000u32 {
            t.send(1, Channel::Unreliable, &i.to_le_bytes()).unwrap();
        }
        let n = drain_for(&mut t, Duration::from_millis(50)).len();
        // loss applies once per direction: expected survivors 2000 * 0.7 * 0.7 = 980
        assert!((800..1160).contains(&n), "{n}");
    }
}
