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
    /// Connect / disconnect / auth events (hidden by default: a CLI agent
    /// connects once per command).
    pub sessions: bool,
}

impl Default for FeedFilter {
    fn default() -> Self {
        Self { edits: true, proposals: true, verify: true, sim: true, reads: false, sessions: false }
    }
}

impl FeedFilter {
    /// True if `e` passes. Errors always do (a failed read or a refused
    /// token is worth seeing).
    pub fn shows(&self, e: &ActivityEntry) -> bool {
        match e.kind {
            ActivityKind::Edit => self.edits,
            ActivityKind::Proposal => self.proposals,
            ActivityKind::Verify => self.verify,
            ActivityKind::Sim => self.sim,
            ActivityKind::Read => self.reads || !e.ok,
            ActivityKind::Session => self.sessions || !e.ok,
        }
    }
}

/// An agent seen in the feed recently (a CLI agent is connected only while
/// a command runs, so "connected now" alone would mostly read "nobody").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentAgent {
    pub name: String,
    /// Server time of its newest non-read, non-session entry.
    pub last_ms: u64,
    /// Its non-read, non-session entries in the feed.
    pub actions: usize,
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
    /// Agents with actions (not reads or connects) at or after `since_ms`
    /// server time, newest first.
    pub fn recent_agents(&self, since_ms: u64) -> Vec<RecentAgent> {
        let mut out: Vec<RecentAgent> = Vec::new();
        for e in self.entries.iter().filter(|e| !e.read && e.kind != ActivityKind::Session && e.at_ms >= since_ms) {
            match out.iter_mut().find(|a| a.name == e.client) {
                Some(a) => {
                    a.last_ms = a.last_ms.max(e.at_ms);
                    a.actions += 1;
                }
                None => out.push(RecentAgent { name: e.client.clone(), last_ms: e.at_ms, actions: 1 }),
            }
        }
        out.sort_by(|a, b| b.last_ms.cmp(&a.last_ms).then_with(|| a.name.cmp(&b.name)));
        out
    }

    /// Adds entries from the server (in `seq` order).
    pub(crate) fn push(&mut self, list: Vec<ActivityEntry>) {
        for e in list {
            self.last_seq = self.last_seq.max(e.seq);
            if e.ok && !e.read && e.kind != ActivityKind::Session {
                self.touched.extend(e.entities.iter().cloned());
            }
            if !e.read && e.kind != ActivityKind::Session {
                // The status bar font has no arrow glyph.
                self.last_action = Some(format!("agent {}: {}", e.client, e.summary.replace('\u{2192}', "->")));
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
