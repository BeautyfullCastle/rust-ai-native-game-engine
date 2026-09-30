//! The agent side of the editor: what the Agent panel shows and does, with
//! no egui in here (the panel is drawn in [`crate::app`]).
//!
//! Proposals live in the [`EditorDoc`] (an ERP client or a script stages
//! them; see `EditorDoc::propose`). This module adds the per-window state
//! around them:
//!
//! - which proposal is selected and which one the viewport previews;
//! - the last `accept` conflict, to show which op failed and why;
//! - verification: [`VerifySource`] (the last play, or a scripted bot),
//!   the checks text, and a background job. The job runs
//!   `orr_edit::verify_frames::<PhysGame>` on copies of the base and the
//!   candidate frame (`EditorDoc::proposal_frames`), so the window never
//!   freezes and the document stays editable. The report itself is a pure
//!   function of the frames, the inputs and the checks; only
//!   [`VerifyRun::millis`] is a wall clock reading.

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Instant;

use orr_edit::{
    verify_frames, Check, CheckOutcome, EditError, EditorDoc, Op, ProposalId, ProposalSummary, VerifyInputs, VerifyOptions, VerifyReport,
};
use orr_sample::physics_game::{PhysGame, PhysInput, PhysMetrics};
use orr_sim::PlayerSlot;

use crate::editor::PLAYERS;

/// Checks the panel starts with: nothing may fall out of the arena.
pub const DEFAULT_CHECKS: &str = "lost_bodies.max == 0";
/// Ticks of the bot input source the panel starts with.
pub const DEFAULT_BOT_TICKS: u32 = 300;
/// Metrics are sampled every this many ticks in a verification.
pub const SAMPLE_EVERY: u32 = 10;

/// Where the per-tick inputs of a verification come from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VerifySource {
    /// The recording of the last play that was stopped in this window.
    LastPlay,
    /// [`bot_input`] for this many ticks.
    Bot(u32),
}

impl VerifySource {
    /// Text for the report header.
    pub fn label(&self) -> String {
        match self {
            VerifySource::LastPlay => "last play".to_string(),
            VerifySource::Bot(n) => format!("bot {n} ticks"),
        }
    }
}

/// Seed of the editor's bot, the same default as ERP's `bot` inputs, so a
/// verification in the panel and one by an agent over ERP match.
pub const BOT_SEED: u64 = 1;

/// The scripted player of verifications: `orr_sample`'s `bot_input` with
/// [`BOT_SEED`]. A pure function of `(tick, slot)`, so a verification with it
/// is reproducible.
pub fn bot_input(tick: u64, slot: PlayerSlot) -> PhysInput {
    orr_sample::physics_game::bot_input(BOT_SEED, tick, slot)
}

/// Why the last `accept` failed, for the panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictNote {
    /// The proposal.
    pub proposal: ProposalId,
    /// Index of the op that no longer applies.
    pub op_index: usize,
    /// That op, described.
    pub op: String,
    /// Why it fails now.
    pub cause: String,
}

/// The finished verification of one proposal.
#[derive(Clone, Debug)]
pub struct VerifyRun {
    /// The proposal.
    pub proposal: ProposalId,
    /// The inputs used.
    pub source: VerifySource,
    /// The report, or why the run could not be made.
    pub report: Result<VerifyReport, String>,
    /// The checks (lines of the checks box, blank ones dropped) and their verdict.
    pub checks: Vec<String>,
    /// The verdict, if the run produced a report.
    pub outcome: Option<CheckOutcome>,
    /// Wall clock time of the run in milliseconds (the only non-deterministic part).
    pub millis: u64,
}

impl VerifyRun {
    /// True if the run produced a report and every check passed.
    pub fn passed(&self) -> bool {
        self.outcome.as_ref().is_some_and(|o| o.passed)
    }
}

struct Job {
    proposal: ProposalId,
    rx: Receiver<VerifyRun>,
}

/// See the module docs.
pub struct AgentState {
    pub(crate) selected: Option<ProposalId>,
    pub(crate) preview: Option<ProposalId>,
    pub(crate) conflict: Option<ConflictNote>,
    job: Option<Job>,
    pub(crate) last: Option<VerifyRun>,
    /// Input source the Verify button uses.
    pub source: VerifySource,
    /// Ticks of the bot source (kept when `source` is `LastPlay`).
    pub bot_ticks: u32,
    /// The checks box: one check per line.
    pub checks: String,
    pub(crate) show_tab: bool,
}

impl Default for AgentState {
    fn default() -> Self {
        Self {
            selected: None,
            preview: None,
            conflict: None,
            job: None,
            last: None,
            source: VerifySource::Bot(DEFAULT_BOT_TICKS),
            bot_ticks: DEFAULT_BOT_TICKS,
            checks: DEFAULT_CHECKS.to_string(),
            show_tab: false,
        }
    }
}

impl AgentState {
    /// The selected proposal.
    pub fn selected(&self) -> Option<ProposalId> {
        self.selected
    }

    /// The proposal the viewport previews.
    pub fn preview(&self) -> Option<ProposalId> {
        self.preview
    }

    /// The last `accept` conflict, until something else is done.
    pub fn conflict(&self) -> Option<&ConflictNote> {
        self.conflict.as_ref()
    }

    /// The proposal a verification is running for.
    pub fn running(&self) -> Option<ProposalId> {
        self.job.as_ref().map(|j| j.proposal)
    }

    /// The last finished verification.
    pub fn last_run(&self) -> Option<&VerifyRun> {
        self.last.as_ref()
    }

    /// Asks the window to switch to the Agent tab (scripts use it).
    pub fn request_tab(&mut self) {
        self.show_tab = true;
    }

    /// True once after [`request_tab`](Self::request_tab).
    pub fn take_tab_request(&mut self) -> bool {
        std::mem::take(&mut self.show_tab)
    }

    /// Drops everything that names a proposal that no longer exists.
    pub(crate) fn sanitize(&mut self, doc: &EditorDoc) {
        let exists = |id: Option<ProposalId>| id.filter(|&i| doc.proposal_info(i).is_ok());
        self.selected = exists(self.selected);
        self.preview = exists(self.preview);
        if self.conflict.as_ref().is_some_and(|c| doc.proposal_info(c.proposal).is_err()) {
            self.conflict = None;
        }
        if self.last.as_ref().is_some_and(|r| doc.proposal_info(r.proposal).is_err()) {
            self.last = None;
        }
    }

    /// Collects a finished job. Returns true if one finished now.
    pub(crate) fn poll(&mut self) -> bool {
        let Some(job) = &self.job else { return false };
        match job.rx.try_recv() {
            Ok(run) => {
                self.last = Some(run);
                self.job = None;
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                let proposal = job.proposal;
                self.last = Some(VerifyRun {
                    proposal,
                    source: self.source,
                    report: Err("the verification thread stopped without a result".into()),
                    checks: Vec::new(),
                    outcome: None,
                    millis: 0,
                });
                self.job = None;
                true
            }
        }
    }

    /// Blocks until the running job (if any) has finished.
    pub(crate) fn wait(&mut self) {
        while self.job.is_some() {
            if !self.poll() {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
    }

    /// Starts a verification of `id` on a thread. `replay` is the recording
    /// of the last play (for [`VerifySource::LastPlay`]).
    pub(crate) fn start(&mut self, doc: &EditorDoc, id: ProposalId, source: VerifySource, replay: Option<Vec<u8>>) -> Result<(), String> {
        if self.job.is_some() {
            return Err("a verification is already running".into());
        }
        let checks: Vec<String> = self.checks.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(str::to_string).collect();
        let parsed = Check::parse_all(&checks).map_err(|e| e.to_string())?;
        let inputs_replay = match source {
            VerifySource::LastPlay => Some(replay.ok_or("no play to replay yet: press Play, then Stop, or choose a bot")?),
            VerifySource::Bot(0) => return Err("a bot run needs at least one tick".into()),
            VerifySource::Bot(_) => None,
        };
        let (base, candidate) = doc.proposal_frames(id).map_err(|e| e.to_string())?;
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("orr-verify".into())
            .spawn(move || {
                let started = Instant::now();
                let result = run_verify(&base, &candidate, source, inputs_replay.as_deref());
                let outcome = result.as_ref().ok().map(|r| r.check(&parsed));
                let millis = started.elapsed().as_millis() as u64;
                // The window may be gone by now; nobody to tell then.
                let _ = tx.send(VerifyRun { proposal: id, source, report: result, checks, outcome, millis });
            })
            .map_err(|e| format!("cannot start the verification thread: {e}"))?;
        self.job = Some(Job { proposal: id, rx });
        self.last = None;
        Ok(())
    }
}

fn run_verify(base: &orr_ecs::Frame, candidate: &orr_ecs::Frame, source: VerifySource, replay: Option<&[u8]>) -> Result<VerifyReport, String> {
    let opts = VerifyOptions { sample_every: SAMPLE_EVERY, ..VerifyOptions::default() };
    let inputs: VerifyInputs<'_, PhysGame> = match (source, replay) {
        (VerifySource::LastPlay, Some(bytes)) => VerifyInputs::from_replay(bytes).map_err(|e| e.to_string())?,
        (VerifySource::Bot(n), _) => VerifyInputs::scripted(n, PLAYERS, bot_input),
        (VerifySource::LastPlay, None) => return Err("no recording".into()),
    };
    verify_frames::<PhysGame>(base, candidate, &inputs, &PhysMetrics, &opts).map_err(|e| e.to_string())
}

/// One line about what a proposal changes: `~ 2 fields, + 1 entity`.
pub fn summary_line(s: &ProposalSummary) -> String {
    let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut parts = Vec::new();
    if !s.entities_added.is_empty() {
        parts.push(format!("+ {}", plural(s.entities_added.len(), "entity", "entities")));
    }
    if !s.entities_removed.is_empty() {
        parts.push(format!("- {}", plural(s.entities_removed.len(), "entity", "entities")));
    }
    if !s.entities_renamed.is_empty() {
        parts.push(format!("~ {}", plural(s.entities_renamed.len(), "rename", "renames")));
    }
    if !s.fields_changed.is_empty() {
        parts.push(format!("~ {}", plural(s.fields_changed.len(), "field", "fields")));
    }
    let comps = s.components_added.len() + s.components_removed.len();
    if comps > 0 {
        parts.push(format!("\u{b1} {}", plural(comps, "component", "components")));
    }
    let singles = s.singletons_added.len() + s.singletons_removed.len();
    if singles > 0 {
        parts.push(format!("\u{b1} {}", plural(singles, "singleton", "singletons")));
    }
    if parts.is_empty() {
        "no change".to_string()
    } else {
        parts.join(", ")
    }
}

/// The note the panel shows for a failed `accept`, if `err` is a conflict.
pub fn conflict_of(doc: &EditorDoc, err: &EditError) -> Option<ConflictNote> {
    match err {
        EditError::ProposalConflict { proposal, op_index, cause } => {
            let id = ProposalId(*proposal);
            let op = doc.proposal_ops(id).ok().and_then(|ops| ops.get(*op_index).map(Op::describe)).unwrap_or_default();
            Some(ConflictNote { proposal: id, op_index: *op_index, op, cause: cause.to_string() })
        }
        _ => None,
    }
}
