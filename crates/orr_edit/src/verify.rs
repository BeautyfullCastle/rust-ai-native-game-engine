//! Replay-based verification: run a base frame and a candidate frame
//! headlessly on the same inputs and compare them.
//!
//! The inputs are either a recorded `.orrp` (its per-tick inputs, commands
//! and debug commands, replayed on each frame) or a scripted generator
//! `(tick, slot) -> Input` for N ticks. Both sides run the same
//! deterministic simulation, so the report (checksums per tick, metric
//! series) is a pure function of the two frames, the inputs and the metric
//! set: running it twice gives an identical [`VerifyReport`].

use std::collections::{BTreeMap, BTreeSet};

use orr_ecs::Frame;
use orr_reflect::TypeRegistry;
use orr_session::ReplayReader;
use orr_sim::{DebugCommand, Game, MetricValue, Metrics, PlayerSlot, Simulation, TickInputs};

use crate::doc::EditorDoc;
use crate::error::EditError;
use crate::op::Origin;
use crate::play::StoppedPlay;
use crate::proposal::ProposalId;

/// Where the per-tick inputs of a verification come from.
pub enum VerifyInputs<'a, G: Game> {
    /// A recorded play (`.orrp`): its inputs, commands and debug commands,
    /// tick by tick. The recording must start at the tick after the frames'
    /// tick (a recording of edit-mode play starts at tick 1).
    Recorded(ReplayReader<G>),
    /// A generator for `ticks` ticks of `players` slots, called as
    /// `(tick, slot)` with the tick being run (first is `frame tick + 1`).
    Scripted {
        /// How many ticks to run.
        ticks: u32,
        /// How many player slots.
        players: u8,
        /// The input of a slot at a tick; must be deterministic.
        input: Box<dyn Fn(u64, PlayerSlot) -> G::Input + Send + Sync + 'a>,
    },
}

impl<'a, G: Game> VerifyInputs<'a, G> {
    /// Parses a `.orrp` recording (for example `StoppedPlay::replay` or a
    /// saved file).
    pub fn from_replay(bytes: &[u8]) -> Result<Self, EditError> {
        let reader = ReplayReader::<G>::parse(bytes).map_err(|e| EditError::Verify(format!("bad replay: {e}")))?;
        Ok(VerifyInputs::Recorded(reader))
    }

    /// [`from_replay`](Self::from_replay) of what play left behind.
    pub fn from_stopped(play: &StoppedPlay) -> Result<Self, EditError> {
        Self::from_replay(&play.replay)
    }

    /// A scripted input generator.
    pub fn scripted(ticks: u32, players: u8, input: impl Fn(u64, PlayerSlot) -> G::Input + Send + Sync + 'a) -> Self {
        VerifyInputs::Scripted { ticks, players, input: Box::new(input) }
    }

    /// The ticks to run, given the tick the frames are at.
    fn tick_list(&self, start: u64, max: Option<u32>) -> Result<Vec<u64>, EditError> {
        let mut ticks: Vec<u64> = match self {
            VerifyInputs::Scripted { ticks, .. } => (start + 1..=start + u64::from(*ticks)).collect(),
            VerifyInputs::Recorded(r) => {
                if r.tick_count() == 0 {
                    return Err(EditError::Verify("the recording has no ticks".into()));
                }
                if r.first_tick() != start + 1 {
                    return Err(EditError::Verify(format!(
                        "the recording starts at tick {} but the frames are at tick {start} (it must start at tick {})",
                        r.first_tick(),
                        start + 1
                    )));
                }
                (r.first_tick()..=r.last_tick()).take_while(|&t| r.tick(t).is_some()).collect()
            }
        };
        if let Some(m) = max {
            ticks.truncate(m as usize);
        }
        if ticks.is_empty() {
            return Err(EditError::Verify("no ticks to run".into()));
        }
        Ok(ticks)
    }

    fn tick_rate(&self, default: u32) -> u32 {
        match self {
            VerifyInputs::Recorded(r) => r.header.tick_rate,
            VerifyInputs::Scripted { .. } => default,
        }
    }

    /// Runs `tick` on `sim`; returns how many debug commands it applied.
    fn step(&self, sim: &mut Simulation<G>, tick: u64, debug: bool) -> u32 {
        match self {
            VerifyInputs::Scripted { players, input, .. } => {
                let mut ti = TickInputs::<G::Input, G::Command>::new(tick, *players);
                for s in 0..*players {
                    ti.set_input(PlayerSlot(s), input(tick, PlayerSlot(s)));
                }
                sim.step_with_debug(&ti, &[]);
                0
            }
            VerifyInputs::Recorded(r) => {
                let Some((inputs, commands)) = r.tick(tick) else { return 0 };
                let mut ti = TickInputs::<G::Input, G::Command>::new(tick, inputs.len() as u8);
                for (i, inp) in inputs.iter().enumerate() {
                    ti.set_input(PlayerSlot(i as u8), *inp);
                }
                ti.set_commands(commands.clone());
                let cmds: &[DebugCommand] = if debug { r.debug_commands(tick) } else { &[] };
                sim.step_with_debug(&ti, cmds);
                cmds.len() as u32
            }
        }
    }
}

/// Settings of a verification run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyOptions {
    /// Sample metrics and checksums every this many ticks (besides the
    /// start and the end). 0 = only the start and the end.
    pub sample_every: u32,
    /// Run at most this many ticks (of a recording or of a script).
    pub max_ticks: Option<u32>,
    /// Tick rate of a scripted run (a recording brings its own).
    pub tick_rate: u32,
    /// Replay the debug commands of a recording. They address entities and
    /// component ids of the frame the recording started from, so they are
    /// meant for frames with that layout; a command that does not fit is
    /// skipped by the simulation (on both sides alike).
    pub debug_commands: bool,
    /// Run the two sides on two threads (same result, less time).
    pub parallel: bool,
    /// Build id of the simulations (see `Simulation::from_frame`).
    pub build_id: u64,
}

impl Default for VerifyOptions {
    fn default() -> Self {
        Self { sample_every: 60, max_ticks: None, tick_rate: 60, debug_commands: true, parallel: true, build_id: 0 }
    }
}

/// Checksums of both sides at one sampled tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChecksumSample {
    /// The tick.
    pub tick: u64,
    /// Base frame checksum.
    pub base: u64,
    /// Candidate frame checksum.
    pub candidate: u64,
}

/// One metric over the run, on one side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetricStats {
    /// At the first sample (before the first tick).
    pub start: MetricValue,
    /// At the last sample (after the last tick).
    pub end: MetricValue,
    /// Smallest over all samples.
    pub min: MetricValue,
    /// Largest over all samples.
    pub max: MetricValue,
    /// Every sample as `(tick, value)`.
    pub series: Vec<(u64, MetricValue)>,
}

/// One metric, base against candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetricComparison {
    /// Metric name.
    pub name: String,
    /// The base run.
    pub base: MetricStats,
    /// The candidate run.
    pub candidate: MetricStats,
    /// `candidate.end - base.end` (`Int` for two `Int`s, else `Fixed`).
    pub delta: MetricValue,
}

/// How the base run compares with the checksums stored in the recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordingCheck {
    /// Recorded checksums inside the run that were compared.
    pub checked: u32,
    /// How many differ from the base run (0 = the base frame reproduces the
    /// recording exactly).
    pub mismatches: u32,
    /// First tick that differs.
    pub first_mismatch: Option<u64>,
}

/// The result of [`verify_frames`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyReport {
    /// The tick both frames started at.
    pub start_tick: u64,
    /// The last tick run.
    pub end_tick: u64,
    /// Ticks run (`end_tick - start_tick`).
    pub ticks: u64,
    /// Base checksum at `start_tick`.
    pub base_start_checksum: u64,
    /// Candidate checksum at `start_tick`.
    pub candidate_start_checksum: u64,
    /// Base checksum at `end_tick`.
    pub base_final_checksum: u64,
    /// Candidate checksum at `end_tick`.
    pub candidate_final_checksum: u64,
    /// First tick (`start_tick` = the frames already differ before any tick)
    /// where the checksums differ; `None` if they are equal at every tick.
    /// Any edit of the scene makes the initial frames differ, so this is
    /// `Some(start_tick)` for a real change: judge behaviour by the metrics.
    pub first_divergence: Option<u64>,
    /// Checksums at the sampled ticks.
    pub samples: Vec<ChecksumSample>,
    /// Every metric, base against candidate, sorted by name.
    pub metrics: Vec<MetricComparison>,
    /// First sampled tick at which any metric differs between the sides.
    pub first_metric_difference: Option<u64>,
    /// Debug commands of the recording that were replayed (per side).
    pub debug_commands_replayed: u32,
    /// For a recorded run: base run against the recording's checksums.
    pub recording: Option<RecordingCheck>,
}

impl VerifyReport {
    /// True if the two runs have the same checksum at every tick.
    pub fn identical(&self) -> bool {
        self.first_divergence.is_none()
    }

    /// One metric by name.
    pub fn metric(&self, name: &str) -> Option<&MetricComparison> {
        self.metrics.iter().find(|m| m.name == name)
    }

    /// A short human-readable summary, one line per fact.
    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![format!(
            "ran {} ticks ({}..{}): base {:016x}, candidate {:016x}",
            self.ticks, self.start_tick, self.end_tick, self.base_final_checksum, self.candidate_final_checksum
        )];
        match self.first_divergence {
            None => out.push("no divergence".to_string()),
            Some(t) => out.push(format!("first checksum divergence at tick {t}")),
        }
        for m in &self.metrics {
            if m.base.end != m.candidate.end || m.base.max != m.candidate.max || m.base.min != m.candidate.min {
                out.push(format!(
                    "{}: end {} -> {} (delta {}), max {} -> {}",
                    m.name, m.base.end, m.candidate.end, m.delta, m.base.max, m.candidate.max
                ));
            }
        }
        out
    }
}

/// Metrics at one sampled tick: `(index, tick, values)`; index 0 = start, k = after k ticks.
type MetricSample = (usize, u64, Vec<(String, MetricValue)>);

/// What one side of the run produced.
struct SideRun {
    start_checksum: u64,
    /// Checksum after each tick run.
    checksums: Vec<u64>,
    /// Metrics at the sampled indices (0 = start, k = after k ticks).
    samples: Vec<MetricSample>,
    debug_replayed: u32,
}

fn run_side<G: Game>(
    frame: &Frame,
    inputs: &VerifyInputs<'_, G>,
    ticks: &[u64],
    tick_rate: u32,
    metrics: &dyn Metrics,
    opts: &VerifyOptions,
) -> Result<SideRun, EditError> {
    let mut sim = Simulation::<G>::from_frame(frame, tick_rate, opts.build_id)
        .map_err(|e| EditError::Verify(format!("frame does not match the game: {e}")))?;
    let n = ticks.len();
    let sampled = |k: usize| k == 0 || k == n || (opts.sample_every > 0 && k % opts.sample_every as usize == 0);
    let mut run = SideRun { start_checksum: sim.checksum(), checksums: Vec::with_capacity(n), samples: Vec::new(), debug_replayed: 0 };
    run.samples.push((0, sim.tick(), metrics.sample(sim.frame())));
    for (i, &t) in ticks.iter().enumerate() {
        run.debug_replayed += inputs.step(&mut sim, t, opts.debug_commands);
        run.checksums.push(sim.checksum());
        if sampled(i + 1) {
            run.samples.push((i + 1, sim.tick(), metrics.sample(sim.frame())));
        }
    }
    Ok(run)
}

/// Runs `base` and `candidate` (frames at the same tick, of game `G`) on the
/// same `inputs` and reports how they compare. Deterministic; see the module
/// docs. Both frames are copied, never changed.
pub fn verify_frames<G: Game>(
    base: &Frame,
    candidate: &Frame,
    inputs: &VerifyInputs<'_, G>,
    metrics: &dyn Metrics,
    opts: &VerifyOptions,
) -> Result<VerifyReport, EditError> {
    if base.tick() != candidate.tick() {
        return Err(EditError::Verify(format!("frames are at different ticks ({} and {})", base.tick(), candidate.tick())));
    }
    let start = base.tick();
    let ticks = inputs.tick_list(start, opts.max_ticks)?;
    let rate = inputs.tick_rate(opts.tick_rate);
    let (b, c) = if opts.parallel && ticks.len() >= 16 {
        std::thread::scope(|s| {
            let h = s.spawn(|| run_side::<G>(base, inputs, &ticks, rate, metrics, opts));
            let c = run_side::<G>(candidate, inputs, &ticks, rate, metrics, opts);
            let b = h.join().unwrap_or_else(|p| std::panic::resume_unwind(p));
            (b, c)
        })
    } else {
        (run_side::<G>(base, inputs, &ticks, rate, metrics, opts), run_side::<G>(candidate, inputs, &ticks, rate, metrics, opts))
    };
    let (b, c) = (b?, c?);
    Ok(assemble(start, &ticks, b, c, inputs))
}

fn assemble<G: Game>(start: u64, ticks: &[u64], b: SideRun, c: SideRun, inputs: &VerifyInputs<'_, G>) -> VerifyReport {
    let end = *ticks.last().unwrap_or(&start);
    let first_divergence = if b.start_checksum != c.start_checksum {
        Some(start)
    } else {
        b.checksums.iter().zip(&c.checksums).position(|(x, y)| x != y).map(|i| ticks[i])
    };

    let mut samples = Vec::with_capacity(b.samples.len());
    for ((k, tick, _), _) in b.samples.iter().zip(&c.samples) {
        let (bc, cc) = if *k == 0 { (b.start_checksum, c.start_checksum) } else { (b.checksums[k - 1], c.checksums[k - 1]) };
        samples.push(ChecksumSample { tick: *tick, base: bc, candidate: cc });
    }

    let (metrics, first_metric_difference) = compare_metrics(&b, &c);

    let recording = match inputs {
        VerifyInputs::Recorded(r) => {
            let mut rc = RecordingCheck { checked: 0, mismatches: 0, first_mismatch: None };
            for &(tick, sum) in &r.checksums {
                let ours = if tick == start {
                    Some(b.start_checksum)
                } else if tick > start && tick <= end {
                    b.checksums.get((tick - start - 1) as usize).copied()
                } else {
                    None
                };
                if let Some(ours) = ours {
                    rc.checked += 1;
                    if ours != sum {
                        rc.mismatches += 1;
                        rc.first_mismatch.get_or_insert(tick);
                    }
                }
            }
            Some(rc)
        }
        VerifyInputs::Scripted { .. } => None,
    };

    VerifyReport {
        start_tick: start,
        end_tick: end,
        ticks: ticks.len() as u64,
        base_start_checksum: b.start_checksum,
        candidate_start_checksum: c.start_checksum,
        base_final_checksum: b.checksums.last().copied().unwrap_or(b.start_checksum),
        candidate_final_checksum: c.checksums.last().copied().unwrap_or(c.start_checksum),
        first_divergence,
        samples,
        metrics,
        first_metric_difference,
        debug_commands_replayed: b.debug_replayed,
        recording,
    }
}

fn stats(series: Vec<(u64, MetricValue)>) -> MetricStats {
    let by = |f: fn(core::cmp::Ordering) -> bool| {
        series.iter().map(|s| s.1).reduce(|a, v| if f(v.cmp_value(a)) { v } else { a }).unwrap_or(MetricValue::Int(0))
    };
    MetricStats {
        start: series.first().map_or(MetricValue::Int(0), |s| s.1),
        end: series.last().map_or(MetricValue::Int(0), |s| s.1),
        min: by(|o| o.is_lt()),
        max: by(|o| o.is_gt()),
        series,
    }
}

/// Metrics of both sides by name; a name one side lacks counts as 0 there.
fn compare_metrics(b: &SideRun, c: &SideRun) -> (Vec<MetricComparison>, Option<u64>) {
    let mut names: BTreeSet<&str> = BTreeSet::new();
    for (_, _, ms) in b.samples.iter().chain(&c.samples) {
        names.extend(ms.iter().map(|(n, _)| n.as_str()));
    }
    let lookup = |ms: &[(String, MetricValue)], name: &str| ms.iter().find(|(n, _)| n == name).map(|(_, v)| *v);
    let mut out = Vec::new();
    let mut first_diff: Option<u64> = None;
    let mut diff_at: BTreeMap<usize, u64> = BTreeMap::new();
    for name in names {
        // The kind of the metric: the first value either side gave.
        let probe = b.samples.iter().chain(&c.samples).find_map(|(_, _, ms)| lookup(ms, name)).unwrap_or(MetricValue::Int(0));
        let series = |run: &SideRun| -> Vec<(u64, MetricValue)> {
            run.samples.iter().map(|(_, t, ms)| (*t, lookup(ms, name).unwrap_or_else(|| probe.zero_like()))).collect()
        };
        let (bs, cs) = (series(b), series(c));
        for (i, (x, y)) in bs.iter().zip(&cs).enumerate() {
            if x.1 != y.1 {
                diff_at.entry(i).or_insert(x.0);
            }
        }
        let (base, candidate) = (stats(bs), stats(cs));
        let delta = candidate.end.delta(base.end);
        out.push(MetricComparison { name: name.to_string(), base, candidate, delta });
    }
    if let Some((_, t)) = diff_at.iter().next() {
        first_diff = Some(*t);
    }
    (out, first_diff)
}

impl EditorDoc {
    /// Verifies a proposal: the base is the document as it is now, the
    /// candidate is the document with the proposal's ops applied (what
    /// `accept` would produce; a conflict is an error). Runs `inputs` on both
    /// and compares. Neither the document nor the proposal changes.
    pub fn verify_proposal<G: Game>(
        &self,
        id: ProposalId,
        inputs: &VerifyInputs<'_, G>,
        metrics: &dyn Metrics,
        opts: &VerifyOptions,
    ) -> Result<VerifyReport, EditError> {
        let mut candidate = self.fork()?;
        let (ops, origin) = {
            let info = self.proposal_info(id)?;
            (self.proposal_ops(id)?.to_vec(), info.origin)
        };
        apply_staged(&mut candidate, id, ops, &origin)?;
        verify_frames::<G>(self.frame(), candidate.frame(), inputs, metrics, opts)
    }

    /// Copies of the two frames [`verify_proposal`](Self::verify_proposal)
    /// runs: `(base, candidate)`. Owned, so a verification can run on
    /// another thread (with [`verify_frames`]) while the document stays
    /// editable. Errors like `verify_proposal` (unknown id, conflict).
    pub fn proposal_frames(&self, id: ProposalId) -> Result<(Frame, Frame), EditError> {
        let mut candidate = self.fork()?;
        let (ops, origin) = {
            let info = self.proposal_info(id)?;
            (self.proposal_ops(id)?.to_vec(), info.origin)
        };
        apply_staged(&mut candidate, id, ops, &origin)?;
        let base = Frame::from_bytes(self.frame_registry.clone(), &self.frame.to_bytes())
            .map_err(|e| EditError::Invalid(format!("cannot copy the preview frame: {e}")))?;
        let cand = Frame::from_bytes(self.frame_registry.clone(), &candidate.frame.to_bytes())
            .map_err(|e| EditError::Invalid(format!("cannot copy the candidate frame: {e}")))?;
        Ok((base, cand))
    }

    /// Runs the document against itself: a baseline run (its metrics, and
    /// for a recording, whether the document reproduces it).
    pub fn verify_self<G: Game>(
        &self,
        inputs: &VerifyInputs<'_, G>,
        metrics: &dyn Metrics,
        opts: &VerifyOptions,
    ) -> Result<VerifyReport, EditError> {
        verify_frames::<G>(self.frame(), self.frame(), inputs, metrics, opts)
    }
}

fn apply_staged(doc: &mut EditorDoc, id: ProposalId, ops: Vec<crate::Op>, origin: &Origin) -> Result<(), EditError> {
    for (i, op) in ops.into_iter().enumerate() {
        doc.apply(op, origin.clone())
            .map_err(|cause| EditError::ProposalConflict { proposal: id.0, op_index: i, cause: Box::new(cause) })?;
    }
    Ok(())
}

/// A generic metric set from reflection: `entities` (live entities) and
/// `components.<type>` (how many entities have each reflected component
/// type, 0 included), all integers.
pub struct ReflectMetrics<'a> {
    types: &'a TypeRegistry,
}

impl<'a> ReflectMetrics<'a> {
    /// Metrics over the types of `types` (an `EditorDoc::types()`).
    pub fn new(types: &'a TypeRegistry) -> Self {
        Self { types }
    }
}

impl Metrics for ReflectMetrics<'_> {
    fn sample(&self, frame: &Frame) -> Vec<(String, MetricValue)> {
        let mut counts: BTreeMap<&str, i64> = self.types.components().map(|t| (t.name(), 0)).collect();
        for e in frame.entities() {
            for name in self.types.component_names(frame, e) {
                *counts.entry(name).or_insert(0) += 1;
            }
        }
        let mut out = vec![("entities".to_string(), MetricValue::Int(i64::from(frame.alive_count())))];
        out.extend(counts.into_iter().map(|(n, c)| (format!("components.{n}"), MetricValue::Int(c))));
        out
    }
}
