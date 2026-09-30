//! Deterministic scalar metrics of a frame, for replay-based verification
//! (compare a change against a baseline run). Integers and `FP` only.

use core::fmt;

use orr_ecs::Frame;
use orr_fp::FP;

/// One metric value: an integer count or a fixed-point quantity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricValue {
    /// A count or any integer quantity.
    Int(i64),
    /// A fixed-point quantity (height, speed, ...).
    Fixed(FP),
}

impl MetricValue {
    /// The value on one common scale (Q48.16 raw units), so an `Int` and a
    /// `Fixed` compare and subtract exactly.
    pub fn scaled(self) -> i128 {
        match self {
            MetricValue::Int(v) => i128::from(v) << 16,
            MetricValue::Fixed(v) => i128::from(v.raw()),
        }
    }

    /// The zero of the same kind.
    pub fn zero_like(self) -> MetricValue {
        match self {
            MetricValue::Int(_) => MetricValue::Int(0),
            MetricValue::Fixed(_) => MetricValue::Fixed(FP::ZERO),
        }
    }

    /// `self - other`: `Int` when both are `Int`, else `Fixed` (saturating).
    pub fn delta(self, other: MetricValue) -> MetricValue {
        match (self, other) {
            (MetricValue::Int(a), MetricValue::Int(b)) => MetricValue::Int(a.saturating_sub(b)),
            (a, b) => {
                let d = a.scaled() - b.scaled();
                MetricValue::Fixed(FP(d.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64))
            }
        }
    }

    /// Total order on the common scale.
    pub fn cmp_value(self, other: MetricValue) -> core::cmp::Ordering {
        self.scaled().cmp(&other.scaled())
    }
}

impl fmt::Display for MetricValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MetricValue::Int(v) => write!(f, "{v}"),
            MetricValue::Fixed(v) => write!(f, "{v}"),
        }
    }
}

/// Reads named metrics off a frame. Must be deterministic: same frame, same
/// values, in the same order, with the same names every call (a name a frame
/// happens not to have a value for is reported as 0 by the verifier).
pub trait Metrics: Sync {
    /// The metrics of `frame`.
    fn sample(&self, frame: &Frame) -> Vec<(String, MetricValue)>;
}

/// No metrics.
pub struct NoMetrics;

impl Metrics for NoMetrics {
    fn sample(&self, _frame: &Frame) -> Vec<(String, MetricValue)> {
        Vec::new()
    }
}

/// Two metric sets in one (`(a, b)` reports a's metrics then b's).
impl<A: Metrics, B: Metrics> Metrics for (A, B) {
    fn sample(&self, frame: &Frame) -> Vec<(String, MetricValue)> {
        let mut v = self.0.sample(frame);
        v.extend(self.1.sample(frame));
        v
    }
}
