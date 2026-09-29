use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::time::Duration;

/// Snapshot of per-connection counters from [`crate::Endpoint::stats`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConnStats {
    /// Smoothed round-trip time. QUIC: from the QUIC stack. WebSocket: last ping/pong sample.
    pub rtt: Option<Duration>,
    /// Application messages handed to the writer / delivered to the caller.
    pub messages_sent: u64,
    pub messages_received: u64,
    /// Payload bytes of those messages.
    pub payload_bytes_sent: u64,
    pub payload_bytes_received: u64,
    /// Wire level. QUIC: UDP datagrams and bytes. WebSocket: frames and frame bytes (payload + tag).
    pub packets_sent: u64,
    pub packets_received: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    /// Unreliable messages sent / received (subset of the message counters).
    pub unreliable_sent: u64,
    pub unreliable_received: u64,
    /// Unreliable messages dropped locally (backlog limit, datagram buffer, event queue full).
    pub unreliable_dropped: u64,
    /// QUIC only: lost packets / sent packets on the path. `None` where not measured.
    pub loss: Option<f32>,
    /// Largest payload that `Channel::Unreliable` accepts now.
    pub max_unreliable_size: usize,
    /// True when the unreliable channel uses real QUIC datagrams.
    pub native_datagrams: bool,
    /// Reliable bytes waiting in the send queue.
    pub queued_bytes: usize,
}

const NONE: u64 = u64::MAX;

#[derive(Debug)]
pub(crate) struct StatsCell {
    pub messages_sent: AtomicU64,
    pub messages_received: AtomicU64,
    pub payload_sent: AtomicU64,
    pub payload_received: AtomicU64,
    pub packets_sent: AtomicU64,
    pub packets_received: AtomicU64,
    pub bytes_sent: AtomicU64,
    pub bytes_received: AtomicU64,
    pub unreliable_sent: AtomicU64,
    pub unreliable_received: AtomicU64,
    pub unreliable_dropped: AtomicU64,
    pub rtt_us: AtomicU64,
    pub loss_ppm: AtomicU64,
    pub max_unreliable: AtomicUsize,
    pub native_datagrams: AtomicBool,
    pub queued: AtomicUsize,
}

impl StatsCell {
    pub fn new(max_unreliable: usize, native: bool) -> Self {
        StatsCell {
            messages_sent: AtomicU64::new(0),
            messages_received: AtomicU64::new(0),
            payload_sent: AtomicU64::new(0),
            payload_received: AtomicU64::new(0),
            packets_sent: AtomicU64::new(0),
            packets_received: AtomicU64::new(0),
            bytes_sent: AtomicU64::new(0),
            bytes_received: AtomicU64::new(0),
            unreliable_sent: AtomicU64::new(0),
            unreliable_received: AtomicU64::new(0),
            unreliable_dropped: AtomicU64::new(0),
            rtt_us: AtomicU64::new(NONE),
            loss_ppm: AtomicU64::new(NONE),
            max_unreliable: AtomicUsize::new(max_unreliable),
            native_datagrams: AtomicBool::new(native),
            queued: AtomicUsize::new(0),
        }
    }

    pub fn add(counter: &AtomicU64, n: u64) {
        counter.fetch_add(n, Relaxed);
    }

    pub fn set_rtt(&self, rtt: Duration) {
        self.rtt_us.store(rtt.as_micros().min(u64::MAX as u128 - 1) as u64, Relaxed);
    }

    pub fn snapshot(&self) -> ConnStats {
        let rtt = self.rtt_us.load(Relaxed);
        let loss = self.loss_ppm.load(Relaxed);
        ConnStats {
            rtt: (rtt != NONE).then(|| Duration::from_micros(rtt)),
            messages_sent: self.messages_sent.load(Relaxed),
            messages_received: self.messages_received.load(Relaxed),
            payload_bytes_sent: self.payload_sent.load(Relaxed),
            payload_bytes_received: self.payload_received.load(Relaxed),
            packets_sent: self.packets_sent.load(Relaxed),
            packets_received: self.packets_received.load(Relaxed),
            bytes_sent: self.bytes_sent.load(Relaxed),
            bytes_received: self.bytes_received.load(Relaxed),
            unreliable_sent: self.unreliable_sent.load(Relaxed),
            unreliable_received: self.unreliable_received.load(Relaxed),
            unreliable_dropped: self.unreliable_dropped.load(Relaxed),
            loss: (loss != NONE).then(|| loss as f32 / 1_000_000.0),
            max_unreliable_size: self.max_unreliable.load(Relaxed),
            native_datagrams: self.native_datagrams.load(Relaxed),
            queued_bytes: self.queued.load(Relaxed),
        }
    }
}
