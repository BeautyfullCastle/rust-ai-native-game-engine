//! Acceptance checks: a small declarative rule list evaluated on a
//! [`VerifyReport`], so an agent can say "verify that no body falls out of
//! the world" and get pass/fail with reasons.
//!
//! Rules are values ([`Check`]) or text ([`Check::parse`]):
//!
//! ```text
//! lost_bodies.max == 0          metric, stat (start|final|min|max|delta), comparison, number
//! mean_height >= 2.5            no stat = final (the value after the last tick)
//! base:lost_bodies.final == 0   the same metric of the base run (default: the candidate)
//! kinetic_energy.delta <= 10    delta = candidate final - base final
//! no_divergence                 checksums equal at every tick
//! no_divergence_before 300      the first checksum divergence is at tick 300 or later
//! recording_matches             the base run reproduces the recording's checksums
//! ```

use core::fmt;

use orr_reflect::decimal::{is_plain_integer, parse_fp, parse_int};
use orr_sim::MetricValue;

use crate::error::EditError;
use crate::verify::{MetricStats, VerifyReport};

/// A comparison operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmp {
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `>=`
    Ge,
    /// `>`
    Gt,
}

impl Cmp {
    fn holds(self, o: core::cmp::Ordering) -> bool {
        match self {
            Cmp::Lt => o.is_lt(),
            Cmp::Le => o.is_le(),
            Cmp::Eq => o.is_eq(),
            Cmp::Ne => o.is_ne(),
            Cmp::Ge => o.is_ge(),
            Cmp::Gt => o.is_gt(),
        }
    }
    fn text(self) -> &'static str {
        match self {
            Cmp::Lt => "<",
            Cmp::Le => "<=",
            Cmp::Eq => "==",
            Cmp::Ne => "!=",
            Cmp::Ge => ">=",
            Cmp::Gt => ">",
        }
    }
    fn parse(s: &str) -> Option<Cmp> {
        Some(match s {
            "<" => Cmp::Lt,
            "<=" => Cmp::Le,
            "==" | "=" => Cmp::Eq,
            "!=" => Cmp::Ne,
            ">=" => Cmp::Ge,
            ">" => Cmp::Gt,
            _ => return None,
        })
    }
}

/// Which number of a metric a rule looks at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricStat {
    /// Before the first tick.
    Start,
    /// After the last tick (the default).
    Final,
    /// Smallest over the run.
    Min,
    /// Largest over the run.
    Max,
    /// Candidate final minus base final (ignores `side`).
    Delta,
}

impl MetricStat {
    fn text(self) -> &'static str {
        match self {
            MetricStat::Start => "start",
            MetricStat::Final => "final",
            MetricStat::Min => "min",
            MetricStat::Max => "max",
            MetricStat::Delta => "delta",
        }
    }
    fn parse(s: &str) -> Option<MetricStat> {
        Some(match s {
            "start" => MetricStat::Start,
            "final" | "end" => MetricStat::Final,
            "min" => MetricStat::Min,
            "max" => MetricStat::Max,
            "delta" => MetricStat::Delta,
            _ => return None,
        })
    }
}

/// Which run a metric rule reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The base run.
    Base,
    /// The candidate run.
    Candidate,
}

/// One acceptance rule.
#[derive(Clone, Debug, PartialEq)]
pub enum Check {
    /// `metric <stat> <cmp> <value>` on one side (`Delta` ignores the side).
    Metric {
        /// Metric name.
        name: String,
        /// Which run.
        side: Side,
        /// Which number of the series.
        stat: MetricStat,
        /// Comparison.
        cmp: Cmp,
        /// The bound (an `Int` bound compares exactly with `Fixed` values).
        value: MetricValue,
    },
    /// The checksums are equal at every tick.
    NoDivergence,
    /// The first checksum divergence is at this tick or later (or none).
    NoDivergenceBefore(u64),
    /// The base run reproduces the checksums stored in the recording
    /// (fails if the run was not from a recording).
    RecordingMatches,
}

impl Check {
    /// A rule on the candidate's metric.
    pub fn metric(name: &str, stat: MetricStat, cmp: Cmp, value: MetricValue) -> Check {
        Check::Metric { name: name.to_string(), side: Side::Candidate, stat, cmp, value }
    }

    /// Parses one rule from text (see the module docs).
    pub fn parse(text: &str) -> Result<Check, EditError> {
        let bad = |why: &str| EditError::Verify(format!("bad check '{text}': {why}"));
        let words: Vec<&str> = text.split_whitespace().collect();
        match words.as_slice() {
            ["no_divergence"] => Ok(Check::NoDivergence),
            ["recording_matches"] => Ok(Check::RecordingMatches),
            ["no_divergence_before", t] => Ok(Check::NoDivergenceBefore(t.parse().map_err(|_| bad("tick is not a number"))?)),
            [lhs, op, num] => {
                let cmp = Cmp::parse(op).ok_or_else(|| bad("comparison must be one of < <= == != >= >"))?;
                let (side, lhs) = match lhs.strip_prefix("base:") {
                    Some(rest) => (Side::Base, rest),
                    None => (Side::Candidate, *lhs),
                };
                let (name, stat) = match lhs.rsplit_once('.') {
                    Some((n, s)) if MetricStat::parse(s).is_some() => (n, MetricStat::parse(s).unwrap_or(MetricStat::Final)),
                    _ => (lhs, MetricStat::Final),
                };
                if name.is_empty() {
                    return Err(bad("missing metric name"));
                }
                let value = if is_plain_integer(num) {
                    let v = parse_int(num).map_err(|_| bad("number out of range"))?;
                    MetricValue::Int(i64::try_from(v).map_err(|_| bad("number out of range"))?)
                } else {
                    MetricValue::Fixed(parse_fp(num).map_err(|_| bad("not a decimal number"))?)
                };
                Ok(Check::Metric { name: name.to_string(), side, stat, cmp, value })
            }
            _ => Err(bad("expected '<metric>[.stat] <cmp> <number>', 'no_divergence', 'no_divergence_before <tick>' or 'recording_matches'")),
        }
    }

    /// Parses several rules.
    pub fn parse_all<S: AsRef<str>>(texts: &[S]) -> Result<Vec<Check>, EditError> {
        texts.iter().map(|t| Check::parse(t.as_ref())).collect()
    }
}

impl fmt::Display for Check {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Check::Metric { name, side, stat, cmp, value } => {
                let base = if *side == Side::Base && *stat != MetricStat::Delta { "base:" } else { "" };
                write!(f, "{base}{name}.{} {} {value}", stat.text(), cmp.text())
            }
            Check::NoDivergence => f.write_str("no_divergence"),
            Check::NoDivergenceBefore(t) => write!(f, "no_divergence_before {t}"),
            Check::RecordingMatches => f.write_str("recording_matches"),
        }
    }
}

/// The verdict on one rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckResult {
    /// The rule, as text.
    pub check: String,
    /// True if it holds.
    pub passed: bool,
    /// Why: the observed value, or what was missing.
    pub reason: String,
}

/// The verdict on a rule list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckOutcome {
    /// True if every rule passed (an empty list passes).
    pub passed: bool,
    /// One result per rule, in order.
    pub results: Vec<CheckResult>,
}

fn pick(stats: &MetricStats, stat: MetricStat) -> MetricValue {
    match stat {
        MetricStat::Start => stats.start,
        MetricStat::Min => stats.min,
        MetricStat::Max => stats.max,
        MetricStat::Final | MetricStat::Delta => stats.end,
    }
}

impl Check {
    /// Evaluates this rule on a report.
    pub fn evaluate(&self, report: &VerifyReport) -> CheckResult {
        let (passed, reason) = match self {
            Check::Metric { name, side, stat, cmp, value } => match report.metric(name) {
                None => (false, format!("metric '{name}' was not reported")),
                Some(m) => {
                    let actual = match (stat, side) {
                        (MetricStat::Delta, _) => m.delta,
                        (_, Side::Base) => pick(&m.base, *stat),
                        (_, Side::Candidate) => pick(&m.candidate, *stat),
                    };
                    let mut reason = format!("{name}.{} is {actual}", stat.text());
                    if matches!(stat, MetricStat::Min | MetricStat::Max) {
                        reason.push_str(if report.every_tick_boundary_observed() {
                            " (sampled at every tick boundary)"
                        } else {
                            " (sampled min/max; between samples not checked)"
                        });
                    }
                    (cmp.holds(actual.cmp_value(*value)), reason)
                }
            },
            Check::NoDivergence => match report.first_divergence {
                None => (true, "checksums equal at every tick".to_string()),
                Some(t) => (false, format!("checksums differ from tick {t}")),
            },
            Check::NoDivergenceBefore(limit) => match report.first_divergence {
                None => (true, "checksums equal at every tick".to_string()),
                Some(t) => (t >= *limit, format!("first divergence at tick {t}")),
            },
            Check::RecordingMatches => match &report.recording {
                None => (false, "the run was not from a recording".to_string()),
                Some(r) if r.checked == 0 => (false, "the recording has no checksums inside the run".to_string()),
                Some(r) if r.mismatches == 0 => (true, format!("{} recorded checksums reproduced", r.checked)),
                Some(r) => (false, format!("{} of {} recorded checksums differ (first at tick {:?})", r.mismatches, r.checked, r.first_mismatch)),
            },
        };
        CheckResult { check: self.to_string(), passed, reason }
    }
}

/// Evaluates every rule on `report`.
pub fn evaluate_checks(report: &VerifyReport, checks: &[Check]) -> CheckOutcome {
    let results: Vec<CheckResult> = checks.iter().map(|c| c.evaluate(report)).collect();
    CheckOutcome { passed: results.iter().all(|r| r.passed), results }
}

impl VerifyReport {
    /// [`evaluate_checks`] on this report.
    pub fn check(&self, checks: &[Check]) -> CheckOutcome {
        evaluate_checks(self, checks)
    }
}
