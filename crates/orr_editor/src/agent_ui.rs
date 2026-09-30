//! The Agent tab of the bottom panel: a read-only **activity feed** of what
//! AI agents do through ERP, the agents that are connected, and the
//! proposals that are still open (view-only preview).
//!
//! The person tells an agent what to do; the agent does it. There is no
//! Accept / Reject / Verify here: the tab shows what happened. Undo (Ctrl+Z,
//! the History tab) takes an agent edit back like any other.
//!
//! Layout: a header (connected agents, or how to connect one), filter
//! toggles, the feed (newest at the bottom, follows new entries unless the
//! person scrolled up; a row expands to its detail) and, on the right, the
//! open proposals.

use std::collections::BTreeMap;

use egui::text::LayoutJob;
use egui::{Color32, RichText, TextFormat, Ui};
use orr_edit::{format_value, CheckOutcome, ProposalDiff, ProposalId, Target, VerifyReport};
use orr_remote::{ActivityEntry, ActivityKind, VerifyDetail};
use orr_sim::MetricValue;

use crate::agent::summary_line;
use crate::app::EditorApp;
use crate::editor::{Editor, Mode};

/// The preview toggle of an open proposal (tests click it by this text).
pub const LBL_PREVIEW: &str = "Preview";
/// The words of the verdict line when every check holds.
pub const TXT_PASSED: &str = "all checks passed";
/// The header when no agent is connected.
pub const TXT_NO_AGENT: &str = "no agent connected";
/// How long an agent counts as recently active in the header (ms).
pub const RECENT_MS: u64 = 60_000;
/// Rows the feed draws at most (the newest ones); older ones stay in the log.
pub const MAX_ROWS: usize = 400;

const GREEN: Color32 = Color32::from_rgb(110, 210, 120);
const RED: Color32 = Color32::from_rgb(240, 100, 100);
const AMBER: Color32 = Color32::from_rgb(240, 190, 80);
const CYAN: Color32 = Color32::from_rgb(90, 200, 240);
const VIOLET: Color32 = Color32::from_rgb(190, 150, 250);
const BLUE: Color32 = Color32::from_rgb(120, 160, 250);

/// Cached diffs of the open proposals: `id -> (ops when computed, diff)`.
/// A proposal's diff only changes when ops are added, so `op_count` is the key.
pub type DiffCache = BTreeMap<u64, (usize, ProposalDiff)>;

/// The diff of proposal `id`, computed once per op count.
pub fn cached_diff<'a>(editor: &Editor, cache: &'a mut DiffCache, id: ProposalId) -> Option<&'a ProposalDiff> {
    let info = editor.doc().proposal_info(id).ok()?;
    let stale = cache.get(&id.0).is_none_or(|(n, _)| *n != info.op_count);
    if stale {
        let diff = editor.doc().proposal_diff(id).ok()?;
        cache.insert(id.0, (info.op_count, diff));
    }
    cache.get(&id.0).map(|(_, d)| d)
}

/// Color and tag of an entry kind.
fn kind_style(kind: ActivityKind) -> (&'static str, Color32) {
    match kind {
        ActivityKind::Edit => ("edit", AMBER),
        ActivityKind::Proposal => ("prop", CYAN),
        ActivityKind::Verify => ("vfy ", VIOLET),
        ActivityKind::Sim => ("sim ", GREEN),
        ActivityKind::Read => ("read", Color32::GRAY),
        ActivityKind::Session => ("conn", BLUE),
    }
}

/// A stable color per agent name.
fn agent_color(name: &str) -> Color32 {
    const PALETTE: [Color32; 6] = [
        Color32::from_rgb(120, 200, 255),
        Color32::from_rgb(255, 170, 120),
        Color32::from_rgb(170, 230, 140),
        Color32::from_rgb(230, 150, 230),
        Color32::from_rgb(240, 220, 120),
        Color32::from_rgb(140, 220, 210),
    ];
    let h = name.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
    PALETTE[(h % 6) as usize]
}

/// `m:ss.d` of a millisecond count.
fn clock(ms: u64) -> String {
    format!("{}:{:02}.{}", ms / 60_000, (ms / 1000) % 60, (ms % 1000) / 100)
}

/// The header row of a feed entry as one colored line.
fn row_job(e: &ActivityEntry, font: &egui::FontId) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut add = |text: String, color: Color32| {
        job.append(&text, 0.0, TextFormat { font_id: font.clone(), color, ..Default::default() });
    };
    let (tag, color) = kind_style(e.kind);
    add(format!("{}  ", clock(e.at_ms)), Color32::GRAY);
    add(format!("{:<8} ", e.client), agent_color(&e.client));
    add(format!("{tag} "), color);
    add(e.summary.clone(), if e.ok { if e.read { Color32::GRAY } else { Color32::from_gray(225) } } else { RED });
    add(if e.ok { "  \u{2714}".to_string() } else { "  \u{d7}".to_string() }, if e.ok { GREEN } else { RED });
    job
}

/// A small status dot (painted: the default fonts have no circle glyph everywhere).
fn dot(ui: &mut Ui, color: Color32, filled: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    if filled {
        ui.painter().circle_filled(rect.center(), 4.0, color);
    } else {
        ui.painter().circle_stroke(rect.center(), 3.5, egui::Stroke::new(1.5, color));
    }
}

impl EditorApp {
    /// The Agent tab.
    pub(crate) fn agent_tab(&mut self, ui: &mut Ui) {
        self.agent_header(ui);
        self.agent_toolbar(ui);
        ui.separator();
        let infos = self.editor.doc().list_proposals();
        self.ui.diffs.retain(|id, _| infos.iter().any(|i| i.id.0 == *id));
        egui::Panel::right("agent_open").resizable(true).default_size(290.0).show_separator_line(true).show(ui, |ui| {
            self.open_proposals(ui, &infos);
        });
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            feed_ui(ui, &self.editor, &mut self.ui.diffs);
        });
    }

    /// Connected agents, or how to connect one.
    fn agent_header(&mut self, ui: &mut Ui) {
        let agents = self.editor.agents();
        let erp = self.editor.erp_status();
        // A CLI agent connects once per command: also show who acted lately.
        let now = self.editor.erp_elapsed_ms().unwrap_or(0);
        let recent: Vec<_> = self
            .editor
            .feed()
            .recent_agents(now.saturating_sub(RECENT_MS))
            .into_iter()
            .filter(|r| !agents.iter().any(|a| a.name == r.name))
            .collect();
        if !agents.is_empty() || !recent.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Agents").strong());
                for a in &agents {
                    dot(ui, agent_color(&a.name), true);
                    ui.label(RichText::new(&a.name).strong().color(agent_color(&a.name)));
                    ui.weak(format!("{} \u{b7} {} req", a.caps, a.requests));
                    ui.add_space(10.0);
                }
                for r in &recent {
                    dot(ui, agent_color(&r.name), false);
                    ui.label(RichText::new(&r.name).strong().color(agent_color(&r.name)));
                    let ago = now.saturating_sub(r.last_ms) / 1000;
                    ui.weak(format!("last action {ago}s ago \u{b7} {} action{}", r.actions, if r.actions == 1 { "" } else { "s" }));
                    ui.add_space(10.0);
                }
            });
            return;
        }
        let url = erp.as_ref().map_or_else(|| "ws://127.0.0.1:7777".to_string(), |(u, _)| u.clone());
        ui.horizontal_wrapped(|ui| {
            dot(ui, AMBER, false);
            ui.label(RichText::new(TXT_NO_AGENT).strong().color(AMBER));
            if erp.is_some() {
                ui.weak(format!("(ERP listening on {url}) connect your agent:"));
            } else {
                ui.weak("\u{2014} start the editor with --erp, then connect your agent:");
            }
        });
        let mono = |ui: &mut Ui, text: String| {
            ui.add(egui::Label::new(RichText::new(text).monospace().small()).selectable(true).wrap_mode(egui::TextWrapMode::Extend));
        };
        if erp.is_none() {
            mono(ui, "orr_editor --erp 127.0.0.1:7777 --erp-dev".to_string());
        }
        mono(ui, format!("ORR_ERP={url} orr status        # CLI (see AGENTS.md)"));
        mono(ui, format!("claude mcp add orrery -- orr_mcp --erp {url}      # or MCP"));
    }

    /// The filter toggles.
    fn agent_toolbar(&mut self, ui: &mut Ui) {
        let unseen_reads = self.editor.feed().entries().iter().filter(|e| e.read).count();
        let f = &mut self.editor.feed_mut().filter;
        ui.horizontal(|ui| {
            ui.weak("show");
            ui.toggle_value(&mut f.edits, "edits");
            ui.toggle_value(&mut f.proposals, "proposals");
            ui.toggle_value(&mut f.verify, "verify");
            ui.toggle_value(&mut f.sim, "sim");
            ui.toggle_value(&mut f.reads, format!("reads ({unseen_reads})"));
            ui.toggle_value(&mut f.sessions, "connections");
            ui.weak("\u{b7} read-only view: Undo (Ctrl+Z) takes an agent edit back");
        });
    }

    /// Proposals an agent has open: label, origin, op count, a view-only preview toggle.
    fn open_proposals(&mut self, ui: &mut Ui, infos: &[orr_edit::ProposalInfo]) {
        ui.label(RichText::new(format!("Open proposals ({})", infos.len())).strong());
        if infos.is_empty() {
            ui.weak("none");
            return;
        }
        let editing = self.editor.mode() == Mode::Edit;
        let mut toggle: Option<(ProposalId, bool)> = None;
        egui::ScrollArea::vertical().id_salt("agent_open_scroll").auto_shrink([false, false]).show(ui, |ui| {
            for info in infos {
                let previewing = self.editor.previewing() == Some(info.id);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}  {}", info.id, info.label)).strong());
                    if info.stale {
                        ui.label(RichText::new("stale").color(AMBER)).on_hover_text("The document changed since this proposal was made.");
                    }
                });
                ui.horizontal(|ui| {
                    ui.weak(format!("{} \u{b7} {} op{}", info.origin, info.op_count, if info.op_count == 1 { "" } else { "s" }));
                    let b = ui.add_enabled(editing, egui::Button::new(LBL_PREVIEW).selected(previewing));
                    if b.on_hover_text("Draw the proposal's staged scene in the viewport (view only)").on_disabled_hover_text("Preview needs edit mode").clicked() {
                        toggle = Some((info.id, !previewing));
                    }
                });
                let line = cached_diff(&self.editor, &mut self.ui.diffs, info.id).map_or_else(|| "?".to_string(), |d| summary_line(&d.summary));
                ui.label(RichText::new(line).monospace().small());
                ui.separator();
            }
        });
        if let Some((id, on)) = toggle {
            self.editor.set_preview(on.then_some(id));
        }
    }
}

/// The feed: newest at the bottom, sticks to the bottom until the person scrolls up.
fn feed_ui(ui: &mut Ui, editor: &Editor, diffs: &mut DiffCache) {
    let rows: Vec<&ActivityEntry> = editor.feed().visible().collect();
    if rows.is_empty() {
        let total = editor.feed().entries().len();
        if total == 0 {
            ui.weak("Nothing yet. What an agent does through ERP shows up here as it happens.");
        } else {
            ui.weak(format!("{total} entries are hidden by the filters."));
        }
        return;
    }
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    egui::ScrollArea::vertical().id_salt("agent_feed").auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
        if rows.len() > MAX_ROWS {
            ui.weak(format!("{} older entries not drawn", rows.len() - MAX_ROWS));
        }
        for e in rows.iter().skip(rows.len().saturating_sub(MAX_ROWS)) {
            let job = row_job(e, &font);
            if !has_detail(e) {
                ui.horizontal(|ui| {
                    ui.add_space(ui.spacing().indent);
                    ui.add(egui::Label::new(job).wrap_mode(egui::TextWrapMode::Extend));
                });
                continue;
            }
            let open = editor.agent().expand_methods.contains(&e.method);
            egui::CollapsingHeader::new(job).id_salt(("agent_row", e.seq)).default_open(open).show(ui, |ui| detail_ui(ui, editor, diffs, e));
        }
    });
}

/// True if a row has something to expand.
fn has_detail(e: &ActivityEntry) -> bool {
    e.error.is_some() || e.change.is_some() || e.diff.is_some() || e.verify.is_some() || e.proposal.is_some() || !e.entities.is_empty()
}

fn detail_ui(ui: &mut Ui, editor: &Editor, diffs: &mut DiffCache, e: &ActivityEntry) {
    let name_of = |s: &str| -> String {
        let target = orr_remote::json::parse_handle(s).map(Target::Entity).or_else(|| orr_reflect::Guid::parse(s).ok().map(Target::Guid));
        match target.and_then(|t| editor.view().entity(&t).ok()).and_then(|i| i.name) {
            Some(n) => format!("{n} ({s})"),
            None => s.to_string(),
        }
    };
    if let Some(err) = &e.error {
        ui.label(RichText::new(format!("error: {err}")).color(RED));
    }
    if let Some(c) = &e.change {
        let who = c.entity.as_deref().map_or_else(|| "singleton".to_string(), name_of);
        let dot = if c.path.is_empty() { String::new() } else { format!(".{}", c.path) };
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(format!("{who}  {}{dot}:", c.component)).monospace());
            let show = |v: &Option<orr_reflect::Value>| v.as_ref().map_or_else(|| "?".to_string(), format_value);
            ui.label(RichText::new(show(&c.old)).monospace().color(RED));
            ui.label(RichText::new("\u{2192}").color(Color32::GRAY));
            ui.label(RichText::new(show(&c.new)).monospace().color(GREEN));
        });
    } else if !e.entities.is_empty() && e.diff.is_none() {
        for g in e.entities.iter().take(8) {
            ui.label(RichText::new(name_of(g)).monospace());
        }
    }
    if let Some(v) = &e.verify {
        report_ui(ui, v);
    }
    if let Some(p) = &e.proposal {
        proposal_detail(ui, editor, diffs, e, p);
    }
}

/// The structured summary and the colored diff of a proposal: taken when the
/// entry was made (accept / reject), else from the open proposal, else from
/// the accept / reject entry the feed saw later.
fn proposal_detail(ui: &mut Ui, editor: &Editor, diffs: &mut DiffCache, e: &ActivityEntry, id: &str) {
    if e.method == "proposal.verify" {
        return;
    }
    let open = id.strip_prefix('p').and_then(|n| n.parse().ok()).map(ProposalId).filter(|i| editor.doc().proposal_info(*i).is_ok());
    let closed;
    let diff: Option<&ProposalDiff> = match (&e.diff, open) {
        (Some(d), _) => Some(d.as_ref()),
        (None, Some(i)) => cached_diff(editor, diffs, i),
        (None, None) => {
            closed = editor.feed().closed_diff(id).and_then(|c| c.diff.clone());
            closed.as_deref()
        }
    };
    let Some(diff) = diff else {
        ui.weak(format!("{id} is no longer open"));
        return;
    };
    ui.label(RichText::new(summary_line(&diff.summary)).strong());
    summary_ui(ui, editor, diff);
    diff_ui(ui, &diff.text);
}

fn line_color(line: &str) -> Option<Color32> {
    if line.starts_with("+++") || line.starts_with("---") {
        None
    } else if line.starts_with('+') {
        Some(GREEN)
    } else if line.starts_with('-') {
        Some(RED)
    } else if line.starts_with("@@") {
        Some(CYAN)
    } else {
        None
    }
}

/// The unified diff text: monospace, `+`/`-` lines colored, scrollable.
fn diff_ui(ui: &mut Ui, text: &str) {
    if text.is_empty() {
        ui.weak("(no change)");
        return;
    }
    let lines: Vec<&str> = text.lines().collect();
    let row_h = ui.text_style_height(&egui::TextStyle::Monospace) + ui.spacing().item_spacing.y;
    egui::ScrollArea::both().id_salt("agent_diff").max_height(170.0).auto_shrink([false, true]).show_rows(ui, row_h, lines.len(), |ui, range| {
        for line in &lines[range] {
            let mut t = RichText::new(*line).monospace();
            t = match line_color(line) {
                Some(c) => t.color(c),
                None => t.weak(),
            };
            ui.add(egui::Label::new(t).wrap_mode(egui::TextWrapMode::Extend));
        }
    });
}

/// The structural summary: entities added, removed, renamed; components;
/// fields old -> new.
fn summary_ui(ui: &mut Ui, editor: &Editor, diff: &ProposalDiff) {
    let s = &diff.summary;
    if s.is_empty() {
        return;
    }
    let name_of = |g: &orr_reflect::Guid| {
        editor.view().entity(&Target::Guid(g.clone())).ok().and_then(|i| i.name).map_or_else(|| g.to_string(), |n| format!("{n} ({g})"))
    };
    for e in &s.entities_added {
        ui.label(RichText::new(format!("+ entity {}", entity_text(e))).color(GREEN));
    }
    for e in &s.entities_removed {
        ui.label(RichText::new(format!("- entity {}", entity_text(e))).color(RED));
    }
    for r in &s.entities_renamed {
        let n = |o: &Option<String>| o.clone().unwrap_or_else(|| "(none)".into());
        ui.label(RichText::new(format!("~ rename {}: {} -> {}", r.guid, n(&r.old), n(&r.new))).color(AMBER));
    }
    for (g, c) in &s.components_added {
        ui.label(RichText::new(format!("+ {c} on {}", name_of(g))).color(GREEN));
    }
    for (g, c) in &s.components_removed {
        ui.label(RichText::new(format!("- {c} on {}", name_of(g))).color(RED));
    }
    for n in &s.singletons_added {
        ui.label(RichText::new(format!("+ singleton {n}")).color(GREEN));
    }
    for n in &s.singletons_removed {
        ui.label(RichText::new(format!("- singleton {n}")).color(RED));
    }
    for f in &s.fields_changed {
        let who = f.entity.as_ref().map_or_else(|| "singleton".to_string(), &name_of);
        let dot = if f.path.is_empty() { String::new() } else { format!(".{}", f.path) };
        ui.label(RichText::new(format!("~ {who}  {}{dot}:  {} -> {}", f.component, format_value(&f.old), format_value(&f.new))).color(AMBER));
    }
}

fn entity_text(e: &orr_edit::EntityRef) -> String {
    match &e.name {
        Some(n) => format!("{n} ({})", e.guid),
        None => e.guid.to_string(),
    }
}

/// The verification report of a feed row.
fn report_ui(ui: &mut Ui, v: &VerifyDetail) {
    let text = v.inputs.as_object().map_or_else(String::new, |o| {
        let kind = o.get("kind").and_then(|k| k.as_str()).unwrap_or("?");
        match o.get("ticks") {
            Some(t) => format!("{kind}, {t} ticks"),
            None => kind.to_string(),
        }
    });
    report_widget(ui, &v.report, v.outcome.as_ref(), &text);
}

fn changed(a: MetricValue, b: MetricValue) -> bool {
    a != b
}

/// The verification report: verdict, per-check results, divergence, metric table.
fn report_widget(ui: &mut Ui, report: &VerifyReport, outcome: Option<&CheckOutcome>, inputs: &str) {
    if let Some(outcome) = outcome {
        if outcome.results.is_empty() {
            ui.label(RichText::new("no checks were set (report only)").color(AMBER));
        } else if outcome.passed {
            ui.label(RichText::new(TXT_PASSED).color(GREEN).strong().size(14.0));
        } else {
            let bad = outcome.results.iter().filter(|r| !r.passed).count();
            ui.label(RichText::new(format!("{bad} of {} checks FAILED", outcome.results.len())).color(RED).strong().size(14.0));
        }
        for r in &outcome.results {
            let (mark, color) = if r.passed { ("\u{2714}", GREEN) } else { ("\u{d7}", RED) };
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(mark).color(color));
                ui.label(RichText::new(&r.check).monospace());
                ui.weak(format!("\u{2014} {}", r.reason));
            });
        }
    } else {
        ui.label(RichText::new("no checks were set (report only)").color(AMBER));
    }
    ui.weak(format!("{} ticks ({}..{}) on {inputs}", report.ticks, report.start_tick, report.end_tick));
    match report.first_divergence {
        None => ui.label("no checksum divergence"),
        Some(t) if t == report.start_tick => ui.label(format!("first checksum divergence: tick {t} (the scenes differ from the start)")),
        Some(t) => ui.label(format!("first checksum divergence: tick {t}")),
    };
    match report.first_metric_difference {
        None => ui.label("metrics identical at every sample"),
        Some(t) => ui.label(format!("first metric difference: tick {t}")),
    };
    // Metrics the proposal changed are listed; the ones that did not change are folded away.
    let differs = |m: &orr_edit::MetricComparison| changed(m.base.end, m.candidate.end) || changed(m.base.min, m.candidate.min) || changed(m.base.max, m.candidate.max);
    let (hot, cold): (Vec<_>, Vec<_>) = report.metrics.iter().partition(|m| differs(m));
    let table = |ui: &mut Ui, id: &str, rows: &[&orr_edit::MetricComparison]| {
        egui::Grid::new(id).striped(true).spacing([14.0, 2.0]).show(ui, |ui| {
            for h in ["metric", "end (base -> cand)", "min", "max", "delta"] {
                ui.label(RichText::new(h).weak().small());
            }
            ui.end_row();
            for m in rows {
                let pair = |a: MetricValue, b: MetricValue| if a == b { a.to_string() } else { format!("{a} -> {b}") };
                let cell = |ui: &mut Ui, text: String, hot: bool| {
                    let t = RichText::new(text).monospace();
                    ui.label(if hot { t.color(AMBER).strong() } else { t });
                };
                cell(ui, m.name.clone(), differs(m));
                cell(ui, pair(m.base.end, m.candidate.end), changed(m.base.end, m.candidate.end));
                cell(ui, pair(m.base.min, m.candidate.min), changed(m.base.min, m.candidate.min));
                cell(ui, pair(m.base.max, m.candidate.max), changed(m.base.max, m.candidate.max));
                cell(ui, m.delta.to_string(), m.delta != m.delta.zero_like());
                ui.end_row();
            }
        });
    };
    if !hot.is_empty() {
        table(ui, "agent_metric_grid", &hot);
    }
    if !cold.is_empty() {
        egui::CollapsingHeader::new(RichText::new(format!("{} unchanged metrics", cold.len())).weak()).id_salt("agent_metric_cold").show(ui, |ui| table(ui, "agent_metric_grid_cold", &cold));
    }
}
