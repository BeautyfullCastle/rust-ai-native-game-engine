//! The agent side of the editor: the state of the Agent tab, with no egui in
//! here (the tab is drawn in [`crate::agent_ui`]).
//!
//! The person tells an external AI agent what to do and the agent works
//! through ERP; the editor does not ask for approval. The tab is a read-only
//! **activity feed** ([`Feed`]: what every ERP request did, from the host's
//! activity log, pushed as `watch.activity`) plus a view-only preview of the
//! proposals an agent has open. Taking something back is Undo (Ctrl+Z / the
//! History tab), which works on agent edits like on the person's own.
//!
//! Proposals live on the host (an ERP client stages them). This module holds
//! the per-window state around them: which proposal the viewport previews,
//! the feed and its filters, and the entities to pulse in the viewport.

use std::collections::VecDeque;

use orr_remote::ActivityKind;
use serde_json::Value as J;

use crate::model::Summary;

/// Entries the feed keeps (the host keeps the same number).
pub const FEED_LIMIT: usize = 2000;

/// One field write of a feed entry: the entity (None = a singleton), the type,
/// the path and the value before and after (ERP JSON).
#[derive(Clone, Debug, PartialEq)]
pub struct FeedChange {
    /// The entity as the client named it (GUID or handle).
    pub entity: Option<String>,
    /// Component or singleton type name.
    pub component: String,
    /// Field path (empty = the whole value).
    pub path: String,
    /// Value before.
    pub old: Option<J>,
    /// Value after.
    pub new: Option<J>,
}

/// The diff of a proposal at accept / reject.
#[derive(Clone, Debug, PartialEq)]
pub struct FeedDiff {
    /// Unified text diff.
    pub text: String,
    /// Structured summary.
    pub summary: Summary,
}

/// One entry of the activity log, as the host sent it.
#[derive(Clone, Debug, PartialEq)]
pub struct FeedEntry {
    /// 1, 2, 3, ... in the order recorded.
    pub seq: u64,
    /// Milliseconds since the host started (display only).
    pub at_ms: u64,
    /// The client name.
    pub client: String,
    /// The kind.
    pub kind: ActivityKind,
    /// The method (`world.patch`), or `session.connect` / `session.disconnect`.
    pub method: String,
    /// One line: `world.patch e_0000000e orr_physics::Body.pos = [6, 18]`.
    pub summary: String,
    /// False if the request ended in an error.
    pub ok: bool,
    /// The error message, if not ok.
    pub error: Option<String>,
    /// True for a read-only request.
    pub read: bool,
    /// GUIDs (or handles) of the entities the request touched.
    pub entities: Vec<String>,
    /// The proposal the request was about (`p2`).
    pub proposal: Option<String>,
    /// The field write, for `world.patch` and `world.singleton.patch`.
    pub change: Option<FeedChange>,
    /// The proposal's diff at `proposal.accept` / `proposal.reject`.
    pub diff: Option<FeedDiff>,
    /// The verification report (ERP JSON), for `proposal.verify` and `verify.self`.
    pub verify: Option<J>,
}

impl FeedEntry {
    /// Parses one entry of `activity.list` / `watch.activity`.
    pub fn from_json(j: &J) -> Option<FeedEntry> {
        let s = |k: &str| j.get(k).and_then(J::as_str).map(str::to_string);
        let change = j.get("change").filter(|c| !c.is_null()).map(|c| FeedChange {
            entity: c.get("entity").and_then(J::as_str).map(str::to_string),
            component: c.get("component").and_then(J::as_str).unwrap_or_default().to_string(),
            path: c.get("path").and_then(J::as_str).unwrap_or_default().to_string(),
            old: c.get("old").filter(|v| !v.is_null()).cloned(),
            new: c.get("new").filter(|v| !v.is_null()).cloned(),
        });
        let diff = j.get("diff").filter(|d| !d.is_null()).map(|d| FeedDiff {
            text: d.get("text").and_then(J::as_str).unwrap_or_default().to_string(),
            summary: d.get("summary").map(Summary::from_json).unwrap_or_default(),
        });
        Some(FeedEntry {
            seq: j.get("seq")?.as_u64()?,
            at_ms: j.get("at_ms")?.as_u64()?,
            client: s("client")?,
            kind: ActivityKind::from_name(j.get("kind")?.as_str()?)?,
            method: s("method")?,
            summary: s("summary")?,
            ok: j.get("ok").and_then(J::as_bool).unwrap_or(true),
            error: s("error"),
            read: j.get("read").and_then(J::as_bool).unwrap_or(false),
            entities: j.get("entities").and_then(J::as_array).map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default(),
            proposal: s("proposal"),
            change,
            diff,
            verify: j.get("verify").filter(|v| !v.is_null()).cloned(),
        })
    }
}

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
    pub fn shows(&self, e: &FeedEntry) -> bool {
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
    /// Its client name.
    pub name: String,
    /// Host time of its newest non-read, non-session entry.
    pub last_ms: u64,
    /// Its non-read, non-session entries in the feed.
    pub actions: usize,
}

/// The activity feed of the window (copied from the host's log). Entries of
/// the editor's own client are left out: the feed is about what others did.
#[derive(Default)]
pub struct Feed {
    entries: VecDeque<FeedEntry>,
    /// The newest `seq` taken from the host.
    pub(crate) last_seq: u64,
    /// The newest `seq` the person has had on screen.
    seen_seq: u64,
    /// Entities touched by entries not yet turned into pulses.
    touched: Vec<String>,
    last_action: Option<String>,
    /// The client name of this editor's own connection (entries of it are not shown).
    pub(crate) own: String,
    /// Which kinds are shown.
    pub filter: FeedFilter,
}

impl Feed {
    /// Agents with actions (not reads or connects) at or after `since_ms`
    /// host time, newest first.
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

    /// Adds entries from the host (in `seq` order).
    pub fn push(&mut self, list: Vec<FeedEntry>) {
        for e in list {
            if e.seq <= self.last_seq {
                continue;
            }
            self.last_seq = e.seq;
            if !self.own.is_empty() && e.client == self.own {
                continue;
            }
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
    pub fn entries(&self) -> &VecDeque<FeedEntry> {
        &self.entries
    }

    /// The entries the filter shows.
    pub fn visible(&self) -> impl Iterator<Item = &FeedEntry> {
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
    pub fn closed_diff(&self, id: &str) -> Option<&FeedEntry> {
        self.entries.iter().rev().find(|e| e.diff.is_some() && e.proposal.as_deref() == Some(id))
    }
}

/// See the module docs.
#[derive(Default)]
pub struct AgentState {
    pub(crate) preview: Option<String>,
    pub(crate) show_tab: bool,
    /// Methods whose feed rows start expanded (scripts use it for screenshots).
    pub expand_methods: Vec<String>,
}

impl AgentState {
    /// The proposal (`p3`) the viewport previews.
    pub fn preview(&self) -> Option<&str> {
        self.preview.as_deref()
    }

    /// Asks the window to switch to the Agent tab (scripts use it).
    pub fn request_tab(&mut self) {
        self.show_tab = true;
    }

    /// True once after [`request_tab`](Self::request_tab).
    pub fn take_tab_request(&mut self) -> bool {
        std::mem::take(&mut self.show_tab)
    }
}

/// One line about what a proposal changes: `~ 2 fields, + 1 entity`.
pub fn summary_line(s: &Summary) -> String {
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
