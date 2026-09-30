//! The agent side of the editor: the state of the Agent tab, with no egui in
//! here (the tab is drawn in [`crate::agent_ui`]).
//!
//! The person tells an external AI agent what to do and the agent works
//! through ERP; the editor does not ask for approval. The tab is a read-only
//! **activity feed** ([`Feed`]: what every ERP request did, from the server's
//! activity log) plus a view-only preview of the proposals an agent has open.
//! Taking something back is Undo (Ctrl+Z / the History tab), which works on
//! agent edits like on the person's own.
//!
//! Proposals live in the [`EditorDoc`] (an ERP client or a script stages
//! them; see `EditorDoc::propose`). This module adds the per-window state
//! around them: which proposal the viewport previews, the feed and its
//! filters, and the entities to pulse in the viewport.

use std::collections::VecDeque;

use orr_edit::{EditorDoc, ProposalId, ProposalSummary};
use orr_remote::{ActivityEntry, ActivityKind};

/// Entries the feed keeps (the server keeps the same number).
pub const FEED_LIMIT: usize = 2000;

/// Which kinds of entries the feed shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeedFilter {
    /// Scene edits.
    pub edits: bool,
    /// Proposal begin / apply / accept / reject.
    pub proposals: bool,
    /// Verifications.
    pub verify: bool,
    /// Play session control.
    pub sim: bool,
    /// Read-only requests (hidden by default).
    pub reads: bool,
}

impl Default for FeedFilter {
    fn default() -> Self {
        Self { edits: true, proposals: true, verify: true, sim: true, reads: false }
    }
}

impl FeedFilter {
    /// True if `e` passes. Session events always do, and so do errors (a
    /// failed read is worth seeing).
    pub fn shows(&self, e: &ActivityEntry) -> bool {
        match e.kind {
            ActivityKind::Edit => self.edits,
            ActivityKind::Proposal => self.proposals,
            ActivityKind::Verify => self.verify,
            ActivityKind::Sim => self.sim,
            ActivityKind::Read => self.reads || !e.ok,
            ActivityKind::Session => true,
        }
    }
}

/// The activity feed of the window (copied from the ERP server's log).
#[derive(Default)]
pub struct Feed {
    entries: VecDeque<ActivityEntry>,
    /// The newest `seq` taken from the server.
    pub(crate) last_seq: u64,
    /// The newest `seq` the person has had on screen.
    seen_seq: u64,
    /// Entities touched by entries not yet turned into pulses.
    touched: Vec<String>,
    last_action: Option<String>,
    /// Which kinds are shown.
    pub filter: FeedFilter,
}

impl Feed {
    /// Adds entries from the server (in `seq` order).
    pub(crate) fn push(&mut self, list: Vec<ActivityEntry>) {
        for e in list {
            self.last_seq = self.last_seq.max(e.seq);
            if e.ok && !e.read && e.kind != ActivityKind::Session {
                self.touched.extend(e.entities.iter().cloned());
            }
            if !e.read && e.kind != ActivityKind::Session {
                self.last_action = Some(format!("agent {}: {}", e.client, e.summary));
            }
            if self.entries.len() >= FEED_LIMIT {
                self.entries.pop_front();
            }
            self.entries.push_back(e);
        }
    }

    /// All entries, oldest first (filter with [`Feed::filter`]).
    pub fn entries(&self) -> &VecDeque<ActivityEntry> {
        &self.entries
    }

    /// The entries the filter shows.
    pub fn visible(&self) -> impl Iterator<Item = &ActivityEntry> {
        self.entries.iter().filter(|e| self.filter.shows(e))
    }

    /// Entries the person has not seen yet, reads excluded (the tab badge).
    pub fn unseen(&self) -> usize {
        self.entries.iter().filter(|e| e.seq > self.seen_seq && !e.read).count()
    }

    /// Marks everything so far as seen (the tab is on screen).
    pub fn mark_seen(&mut self) {
        self.seen_seq = self.last_seq;
    }

    /// The entities touched since the last call (the window turns them into viewport pulses).
    pub fn take_touched(&mut self) -> Vec<String> {
        let mut v = std::mem::take(&mut self.touched);
        v.sort();
        v.dedup();
        v
    }

    /// `agent claude: proposal.verify p2 ...`: the last thing an agent did (reads excluded).
    pub fn last_action(&self) -> Option<&str> {
        self.last_action.as_deref()
    }

    /// The diff captured at accept / reject of proposal `id`, if the feed saw one.
    pub fn closed_diff(&self, id: &str) -> Option<&ActivityEntry> {
        self.entries.iter().rev().find(|e| e.diff.is_some() && e.proposal.as_deref() == Some(id))
    }
}

/// See the module docs.
#[derive(Default)]
pub struct AgentState {
    pub(crate) preview: Option<ProposalId>,
    pub(crate) show_tab: bool,
    /// Methods whose feed rows start expanded (scripts use it for screenshots).
    pub expand_methods: Vec<String>,
}

impl AgentState {
    /// The proposal the viewport previews.
    pub fn preview(&self) -> Option<ProposalId> {
        self.preview
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
        self.preview = self.preview.filter(|&i| doc.proposal_info(i).is_ok());
    }
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
