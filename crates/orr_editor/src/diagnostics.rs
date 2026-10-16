//! Bounded, view-only CPU wall-clock telemetry. No GPU queries or host requests.
//!
//! Recording is allocation-free and O(1). Summaries copy at most 120 samples
//! onto the stack and select the nearest-rank p95 only when requested. All
//! durations are monotonic elapsed wall time, not CPU utilization or GPU time.

use std::time::Duration;

/// Number of recent observations retained per latency metric.
pub const LATENCY_WINDOW: usize = 120;

/// A summary of the most recent [`LATENCY_WINDOW`] observations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LatencyStats {
    /// Observations currently in the rolling window (zero means no measurement).
    pub samples: usize,
    /// Observations since this editor connected, saturating at `u64::MAX`.
    pub total_samples: u64,
    /// Most recent elapsed duration; zero for an empty window.
    pub last: Duration,
    /// Largest elapsed duration in the current window; zero when empty.
    pub max: Duration,
    /// Nearest-rank 95th percentile in the current window; zero when empty.
    pub p95: Duration,
}

/// Local diagnostics; querying these does not poll, wait for or contact a host.
///
/// Windows are per metric, not per frame. They reset after successful restart
/// or reconnect. A headless `Editor` has no UI-frame samples. Timings overlap:
/// pump and synchronous waits can be part of a UI frame, and snapshot extraction
/// can be part of pump. Do not sum the fields as independent phases.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EditorDiagnostics {
    /// `EditorApp::ui` elapsed time, including any calls made by widgets, but
    /// excluding eframe's later tessellation, presentation and GPU execution.
    pub ui_frame: LatencyStats,
    /// `Editor::pump` elapsed time, including notification/reply handling and
    /// snapshot extraction. Calls made while disconnected are recorded too.
    pub pump: LatencyStats,
    /// Blocking calls on the editor's primary ERP client, including failures
    /// and initial state queries. Excludes connect/subscribe/handshake, other
    /// clients, and reply ingestion after the blocking call returns.
    pub sync_erp_wait: LatencyStats,
    /// Time from posting a successful asynchronous request until its reply is
    /// ingested, including host/transport wait and editor polling delay. Error
    /// replies count; requests abandoned on disconnect have no latency sample.
    pub async_request: LatencyStats,
    /// Main-view extraction of body views, checksum and alive count from each
    /// changed snapshot. Excludes transport decoding, preview and GPU work.
    pub snapshot_extract: LatencyStats,
    /// Asynchronous primary-client requests still awaiting ingestion.
    pub pending_requests: usize,
    /// Largest simultaneous pending count since this editor connected.
    pub pending_high_water: usize,
}

pub(crate) struct RollingLatency {
    samples: [Duration; LATENCY_WINDOW],
    len: usize,
    next: usize,
    total: u64,
}

impl Default for RollingLatency {
    fn default() -> Self {
        Self {
            samples: [Duration::ZERO; LATENCY_WINDOW],
            len: 0,
            next: 0,
            total: 0,
        }
    }
}

impl RollingLatency {
    pub(crate) fn record(&mut self, elapsed: Duration) {
        self.samples[self.next] = elapsed;
        self.next = (self.next + 1) % LATENCY_WINDOW;
        self.len = (self.len + 1).min(LATENCY_WINDOW);
        self.total = self.total.saturating_add(1);
    }

    pub(crate) fn summary(&self) -> LatencyStats {
        if self.len == 0 {
            return LatencyStats::default();
        }
        let last = self.samples[(self.next + LATENCY_WINDOW - 1) % LATENCY_WINDOW];
        let mut scratch = self.samples;
        let values = &mut scratch[..self.len];
        let max = values.iter().copied().max().unwrap_or_default();
        let rank = (self.len * 95).div_ceil(100) - 1;
        let (_, p95, _) = values.select_nth_unstable(rank);
        LatencyStats {
            samples: self.len,
            total_samples: self.total,
            last,
            max,
            p95: *p95,
        }
    }
}

#[derive(Default)]
pub(crate) struct Telemetry {
    pub ui_frame: RollingLatency,
    pub pump: RollingLatency,
    pub sync_erp_wait: RollingLatency,
    pub async_request: RollingLatency,
    pub snapshot_extract: RollingLatency,
    pub pending_high_water: usize,
}

impl Telemetry {
    pub(crate) fn summary(&self, pending_requests: usize) -> EditorDiagnostics {
        EditorDiagnostics {
            ui_frame: self.ui_frame.summary(),
            pump: self.pump.summary(),
            sync_erp_wait: self.sync_erp_wait.summary(),
            async_request: self.async_request.summary(),
            snapshot_extract: self.snapshot_extract.summary(),
            pending_requests,
            pending_high_water: self.pending_high_water,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_zero_duration_are_distinct() {
        let mut window = RollingLatency::default();
        assert_eq!(window.summary(), LatencyStats::default());
        window.record(Duration::ZERO);
        assert_eq!(
            window.summary(),
            LatencyStats {
                samples: 1,
                total_samples: 1,
                ..LatencyStats::default()
            }
        );
    }

    #[test]
    fn nearest_rank_p95_and_last_are_not_an_average() {
        let mut window = RollingLatency::default();
        for us in (1..=20).rev() {
            window.record(Duration::from_micros(us));
        }
        let want = LatencyStats {
            samples: 20,
            total_samples: 20,
            last: Duration::from_micros(1),
            max: Duration::from_micros(20),
            p95: Duration::from_micros(19),
        };
        assert_eq!(window.summary(), want);
        assert_eq!(window.summary(), want, "querying never consumes or reorders samples");
    }

    #[test]
    fn wrapping_evicts_old_max_and_keeps_exact_capacity() {
        let mut window = RollingLatency::default();
        window.record(Duration::MAX);
        for _ in 0..LATENCY_WINDOW {
            window.record(Duration::from_nanos(7));
        }
        let stats = window.summary();
        assert_eq!(stats.samples, LATENCY_WINDOW);
        assert_eq!(stats.total_samples, LATENCY_WINDOW as u64 + 1);
        assert_eq!(stats.max, Duration::from_nanos(7));
        assert_eq!(stats.last, stats.max);
        assert_eq!(stats.p95, stats.max);
        for us in 1..=LATENCY_WINDOW as u64 {
            window.record(Duration::from_micros(us));
        }
        assert_eq!(window.summary().p95, Duration::from_micros(114));
    }

    #[test]
    fn single_sample_max_duration_and_saturating_count() {
        let mut window = RollingLatency {
            total: u64::MAX,
            ..RollingLatency::default()
        };
        window.record(Duration::MAX);
        let stats = window.summary();
        assert_eq!(stats.total_samples, u64::MAX);
        assert_eq!(stats.last, Duration::MAX);
        assert_eq!(stats.max, Duration::MAX);
        assert_eq!(stats.p95, Duration::MAX);
    }
}
