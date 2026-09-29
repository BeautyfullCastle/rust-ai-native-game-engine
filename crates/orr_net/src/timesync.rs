//! NTP-style clock offset and RTT estimation from ping/pong samples.
//!
//! Pure logic with injected timestamps, no clock access. All times are
//! microseconds (`i64`) on the clock of the side that took them.
//!
//! One exchange: the client stamps `t0` and sends a ping. The server stamps
//! `t1` on receipt and `t2` when it sends the pong. The client stamps `t3` on
//! receipt. Then
//!
//! * `rtt = (t3 - t0) - (t2 - t1)` (network time, server processing removed)
//! * `offset = ((t1 - t0) + (t2 - t3)) / 2` (server clock minus client clock)
//!
//! A single sample is noisy because the forward and return delays differ. The
//! estimator keeps the last `window` samples and trusts the ones with the
//! smallest RTT, which suffered the least queueing: it sorts by RTT, keeps the
//! lowest half (at least one), and reports the median RTT and median offset of
//! those. Samples with negative RTT (clock or ordering error) are rejected.

use std::collections::VecDeque;

/// The four timestamps of one ping/pong exchange, in microseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    /// Client clock when the ping was sent.
    pub t0: i64,
    /// Server clock when the ping arrived.
    pub t1: i64,
    /// Server clock when the pong was sent.
    pub t2: i64,
    /// Client clock when the pong arrived.
    pub t3: i64,
}

impl Sample {
    /// Network round trip, excluding the server's hold time.
    pub fn rtt_us(&self) -> i64 {
        (self.t3 - self.t0) - (self.t2 - self.t1)
    }

    /// Server clock minus client clock. Exact when both delays are equal.
    pub fn offset_us(&self) -> i64 {
        ((self.t1 - self.t0) + (self.t2 - self.t3)).div_euclid(2)
    }
}

/// Result of [`TimeSync::estimate`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Estimate {
    /// Filtered RTT (median of the best samples).
    pub rtt_us: i64,
    /// Smallest RTT in the window.
    pub rtt_min_us: i64,
    /// Filtered server-minus-client offset.
    pub offset_us: i64,
    /// Median absolute deviation of RTT over the whole window. A jitter measure.
    pub jitter_us: i64,
    /// Samples currently in the window.
    pub samples: usize,
}

/// Sliding-window estimator.
#[derive(Clone, Debug)]
pub struct TimeSync {
    window: usize,
    samples: VecDeque<(i64, i64)>, // (rtt, offset)
}

impl Default for TimeSync {
    fn default() -> Self {
        TimeSync::new(16)
    }
}

fn median(v: &mut [i64]) -> i64 {
    v.sort_unstable();
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]).div_euclid(2)
    }
}

impl TimeSync {
    /// `window` is the number of recent samples kept (at least 1).
    pub fn new(window: usize) -> Self {
        let window = window.max(1);
        TimeSync { window, samples: VecDeque::with_capacity(window) }
    }

    /// Adds one exchange. Returns `false` (and ignores it) if its RTT is negative.
    pub fn add_sample(&mut self, s: Sample) -> bool {
        let rtt = s.rtt_us();
        if rtt < 0 {
            return false;
        }
        if self.samples.len() == self.window {
            self.samples.pop_front();
        }
        self.samples.push_back((rtt, s.offset_us()));
        true
    }

    /// Convenience for a client that never learns `t1`/`t2` separately (server hold time zero).
    pub fn add_round_trip(&mut self, t0: i64, server_time: i64, t3: i64) -> bool {
        self.add_sample(Sample { t0, t1: server_time, t2: server_time, t3 })
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }

    /// Current estimate, or `None` before the first accepted sample.
    pub fn estimate(&self) -> Option<Estimate> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted: Vec<(i64, i64)> = self.samples.iter().copied().collect();
        sorted.sort_unstable();
        let keep = sorted.len().div_ceil(2);
        let best = &sorted[..keep];
        let rtt = median(&mut best.iter().map(|s| s.0).collect::<Vec<_>>());
        let offset = median(&mut best.iter().map(|s| s.1).collect::<Vec<_>>());
        let all_med = median(&mut sorted.iter().map(|s| s.0).collect::<Vec<_>>());
        let jitter = median(&mut sorted.iter().map(|s| (s.0 - all_med).abs()).collect::<Vec<_>>());
        Some(Estimate {
            rtt_us: rtt,
            rtt_min_us: sorted[0].0,
            offset_us: offset,
            jitter_us: jitter,
            samples: sorted.len(),
        })
    }

    /// Estimated server clock for a local timestamp.
    pub fn server_time_us(&self, local_us: i64) -> Option<i64> {
        self.estimate().map(|e| local_us + e.offset_us)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a sample for a known clock offset and one-way delays.
    fn make(t0: i64, offset: i64, fwd: i64, back: i64, hold: i64) -> Sample {
        let t1 = t0 + fwd + offset;
        let t2 = t1 + hold;
        let t3 = t2 - offset + back;
        Sample { t0, t1, t2, t3 }
    }

    #[test]
    fn symmetric_delay_is_exact() {
        let s = make(1_000, 5_000_000, 20_000, 20_000, 300);
        assert_eq!(s.rtt_us(), 40_000);
        assert_eq!(s.offset_us(), 5_000_000);
    }

    #[test]
    fn asymmetric_delay_error_is_half_the_difference() {
        let s = make(0, -70_000, 30_000, 10_000, 0);
        assert_eq!(s.rtt_us(), 40_000);
        assert_eq!(s.offset_us() - (-70_000), 10_000);
    }

    #[test]
    fn negative_rtt_rejected() {
        let mut ts = TimeSync::default();
        assert!(!ts.add_sample(Sample { t0: 100, t1: 0, t2: 500, t3: 200 }));
        assert!(ts.estimate().is_none());
    }

    #[test]
    fn min_filter_ignores_delay_spikes() {
        let mut ts = TimeSync::new(16);
        // True offset 1_000_000. Most samples are clean, a few have a big
        // one-sided queueing delay that skews their offset.
        for i in 0..16 {
            let spike = i % 4 == 3;
            let (f, b) = if spike { (90_000, 20_000) } else { (20_000, 20_000) };
            ts.add_sample(make(i * 100_000, 1_000_000, f, b, 100));
        }
        let e = ts.estimate().unwrap();
        assert_eq!(e.offset_us, 1_000_000);
        assert_eq!(e.rtt_min_us, 40_000);
        assert_eq!(e.rtt_us, 40_000);
        assert_eq!(e.samples, 16);
    }

    #[test]
    fn window_slides_and_tracks_change() {
        let mut ts = TimeSync::new(4);
        for i in 0..4 {
            ts.add_sample(make(i, 100, 10_000, 10_000, 0));
        }
        assert_eq!(ts.estimate().unwrap().offset_us, 100);
        for i in 0..4 {
            ts.add_sample(make(i, 900, 10_000, 10_000, 0));
        }
        let e = ts.estimate().unwrap();
        assert_eq!(e.offset_us, 900);
        assert_eq!(e.samples, 4);
    }

    #[test]
    fn jitter_is_median_absolute_deviation() {
        let mut ts = TimeSync::new(8);
        for rtt in [40_000, 42_000, 38_000, 40_000, 41_000] {
            ts.add_sample(make(0, 0, rtt / 2, rtt / 2, 0));
        }
        let e = ts.estimate().unwrap();
        // median 40_000; deviations 0, 2000, 2000, 0, 1000 -> median 1000
        assert_eq!(e.jitter_us, 1_000);
    }

    #[test]
    fn server_time_and_single_sample() {
        let mut ts = TimeSync::default();
        assert!(ts.server_time_us(0).is_none());
        ts.add_round_trip(1_000, 51_010, 1_020);
        assert_eq!(ts.server_time_us(2_000), Some(52_000));
        assert_eq!(ts.estimate().unwrap().samples, 1);
    }

    #[test]
    fn median_of_even_count() {
        assert_eq!(median(&mut [1, 3]), 2);
        assert_eq!(median(&mut [-3, -2]), -3);
    }
}
