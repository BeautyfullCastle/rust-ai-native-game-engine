//! The Agent tab of the bottom panel: a read-only **activity feed** of what
//! AI agents do through ERP, the agents that are connected, and the
//! proposals that are still open (view-only preview).
//!
//! The person tells an agent what to do; the agent does it. There is no
//! Accept / Reject / Verify here: the tab shows what happened. Undo (Ctrl+Z,
//! the History tab) takes an agent edit back like any other.
//!
//! Everything shown comes from the host through ERP (`activity.list` /
//! `watch.activity`, `proposal.get`, `watch.proposals`); the tab has no
//! access to a document.
//!
//! Layout: a header (connected agents, or how to connect one), filter
//! toggles, the feed (newest at the bottom, follows new entries unless the
//! person scrolled up; a row expands to its detail) and, on the right, the
//! open proposals.

use egui::text::LayoutJob;
use egui::{Color32, RichText, TextFormat, Ui};
use orr_remote::ActivityKind;
use serde_json::Value as J;

use crate::agent::{summary_line, FeedEntry};
use crate::app::EditorApp;
use crate::editor::{Editor, Mode};
use crate::model::{format_json, EntityRef, ProposalInfo, Summary, Target};

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
fn row_job(e: &FeedEntry, font: &egui::FontId) -> LayoutJob {
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
        let infos = self.editor.proposals().to_vec();
        egui::Panel::right("agent_open").resizable(true).default_size(290.0).show_separator_line(true).show(ui, |ui| {
            self.open_proposals(ui, &infos);
        });
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            feed_ui(ui, &self.editor);
        });
    }

    /// Connected agents, or how to connect one.
    fn agent_header(&mut self, ui: &mut Ui) {
        let agents = self.editor.agents().to_vec();
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
    fn open_proposals(&mut self, ui: &mut Ui, infos: &[ProposalInfo]) {
        ui.label(RichText::new(format!("Open proposals ({})", infos.len())).strong());
        if infos.is_empty() {
            ui.weak("none");
            return;
        }
        let editing = self.editor.mode() == Mode::Edit;
        let mut toggle: Option<(String, bool)> = None;
        egui::ScrollArea::vertical().id_salt("agent_open_scroll").auto_shrink([false, false]).show(ui, |ui| {
            for info in infos {
                let previewing = self.editor.previewing() == Some(info.id.as_str());
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
                        toggle = Some((info.id.clone(), !previewing));
                    }
                });
                let line = self.editor.proposal_detail(&info.id).map_or_else(|| "?".to_string(), |d| summary_line(&d.summary));
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
fn feed_ui(ui: &mut Ui, editor: &Editor) {
    let rows: Vec<&FeedEntry> = editor.feed().visible().collect();
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
            egui::CollapsingHeader::new(job).id_salt(("agent_row", e.seq)).default_open(open).show(ui, |ui| detail_ui(ui, editor, e));
        }
    });
}

/// True if a row has something to expand.
fn has_detail(e: &FeedEntry) -> bool {
    e.error.is_some() || e.change.is_some() || e.diff.is_some() || e.verify.is_some() || e.proposal.is_some() || !e.entities.is_empty()
}

/// An entity as `name (id)` if the hierarchy knows its name.
fn name_of(editor: &Editor, s: &str) -> String {
    match Target::parse(s).and_then(|t| editor.row_of(&t).and_then(|r| r.name.clone())) {
        Some(n) => format!("{n} ({s})"),
        None => s.to_string(),
    }
}

fn detail_ui(ui: &mut Ui, editor: &Editor, e: &FeedEntry) {
    if let Some(err) = &e.error {
        ui.label(RichText::new(format!("error: {err}")).color(RED));
    }
    if let Some(c) = &e.change {
        let who = c.entity.as_deref().map_or_else(|| "singleton".to_string(), |s| name_of(editor, s));
        let dot = if c.path.is_empty() { String::new() } else { format!(".{}", c.path) };
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(format!("{who}  {}{dot}:", c.component)).monospace());
            let show = |v: &Option<J>| v.as_ref().map_or_else(|| "?".to_string(), format_json);
            ui.label(RichText::new(show(&c.old)).monospace().color(RED));
            ui.label(RichText::new("\u{2192}").color(Color32::GRAY));
            ui.label(RichText::new(show(&c.new)).monospace().color(GREEN));
        });
    } else if !e.entities.is_empty() && e.diff.is_none() {
        for g in e.entities.iter().take(8) {
            ui.label(RichText::new(name_of(editor, g)).monospace());
        }
    }
    if let Some(v) = &e.verify {
        report_ui(ui, v);
    }
    if let Some(p) = &e.proposal {
        proposal_detail(ui, editor, e, p);
    }
}

/// The structured summary and the colored diff of a proposal: taken when the
/// entry was made (accept / reject), else from the open proposal, else from
/// the accept / reject entry the feed saw later.
fn proposal_detail(ui: &mut Ui, editor: &Editor, e: &FeedEntry, id: &str) {
    if e.method == "proposal.verify" {
        return;
    }
    let (text, summary): (&str, &Summary) = match (&e.diff, editor.proposal_detail(id)) {
        (Some(d), _) => (&d.text, &d.summary),
        (None, Some(d)) => (&d.diff, &d.summary),
        (None, None) => match editor.feed().closed_diff(id).and_then(|c| c.diff.as_ref()) {
            Some(d) => (&d.text, &d.summary),
            None => {
                ui.weak(format!("{id} is no longer open"));
                return;
            }
        },
    };
    ui.label(RichText::new(summary_line(summary)).strong());
    summary_ui(ui, editor, summary);
    diff_ui(ui, text);
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
fn summary_ui(ui: &mut Ui, editor: &Editor, s: &Summary) {
    if s.is_empty() {
        return;
    }
    let name_of = |g: &orr_reflect::Guid| {
        editor.row_of(&Target::Guid(g.clone())).and_then(|r| r.name.clone()).map_or_else(|| g.to_string(), |n| format!("{n} ({g})"))
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
        ui.label(RichText::new(format!("~ {who}  {}{dot}:  {} -> {}", f.component, format_json(&f.old), format_json(&f.new))).color(AMBER));
    }
}

fn entity_text(e: &EntityRef) -> String {
    match &e.name {
        Some(n) => format!("{n} ({})", e.guid),
        None => e.guid.to_string(),
    }
}

/// The verification report of a feed row (the `verify` JSON of the entry).
fn report_ui(ui: &mut Ui, v: &J) {
    let text = v.get("inputs").and_then(J::as_object).map_or_else(String::new, |o| {
        let kind = o.get("kind").and_then(|k| k.as_str()).unwrap_or("?");
        match o.get("ticks") {
            Some(t) => format!("{kind}, {t} ticks"),
            None => kind.to_string(),
        }
    });
    report_widget(ui, v, &text);
}

/// A metric value of the report as text.
fn metric(j: &J, stat: &str, side: &str) -> J {
    j.get(side).and_then(|s| s.get(stat)).cloned().unwrap_or(J::Null)
}

/// The verification report: verdict, per-check results, divergence, metric table.
fn report_widget(ui: &mut Ui, report: &J, inputs: &str) {
    let checks = report.get("checks").filter(|c| !c.is_null());
    let results = checks.and_then(|c| c.get("results")).and_then(J::as_array).cloned().unwrap_or_default();
    if checks.is_none() || results.is_empty() {
        ui.label(RichText::new("no checks were set (report only)").color(AMBER));
    } else if checks.and_then(|c| c.get("passed")).and_then(J::as_bool) == Some(true) {
        ui.label(RichText::new(TXT_PASSED).color(GREEN).strong().size(14.0));
    } else {
        let bad = results.iter().filter(|r| r.get("passed").and_then(J::as_bool) != Some(true)).count();
        ui.label(RichText::new(format!("{bad} of {} checks FAILED", results.len())).color(RED).strong().size(14.0));
    }
    for r in &results {
        let passed = r.get("passed").and_then(J::as_bool) == Some(true);
        let (mark, color) = if passed { ("\u{2714}", GREEN) } else { ("\u{d7}", RED) };
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(mark).color(color));
            ui.label(RichText::new(r.get("check").and_then(J::as_str).unwrap_or("?")).monospace());
            ui.weak(format!("\u{2014} {}", r.get("reason").and_then(J::as_str).unwrap_or("")));
        });
    }
    let u = |k: &str| report.get(k).and_then(J::as_u64).unwrap_or(0);
    ui.weak(format!("{} ticks ({}..{}) on {inputs}", u("ticks"), u("start_tick"), u("end_tick")));
    match report.get("first_divergence").and_then(J::as_u64) {
        None => ui.label("no checksum divergence"),
        Some(t) if t == u("start_tick") => ui.label(format!("first checksum divergence: tick {t} (the scenes differ from the start)")),
        Some(t) => ui.label(format!("first checksum divergence: tick {t}")),
    };
    match report.get("first_metric_difference").and_then(J::as_u64) {
        None => ui.label("metrics identical at every sample"),
        Some(t) => ui.label(format!("first metric difference: tick {t}")),
    };
    // Metrics the proposal changed are listed; the ones that did not change are folded away.
    let metrics: Vec<&J> = report.get("metrics").and_then(J::as_array).map(|a| a.iter().collect()).unwrap_or_default();
    let differs = |m: &J| ["end", "min", "max"].iter().any(|s| metric(m, s, "base") != metric(m, s, "candidate"));
    let (hot, cold): (Vec<&J>, Vec<&J>) = metrics.into_iter().partition(|m| differs(m));
    let table = |ui: &mut Ui, id: &str, rows: &[&J]| {
        egui::Grid::new(id).striped(true).spacing([14.0, 2.0]).show(ui, |ui| {
            for h in ["metric", "end (base -> cand)", "min", "max", "delta"] {
                ui.label(RichText::new(h).weak().small());
            }
            ui.end_row();
            for m in rows {
                let pair = |s: &str| {
                    let (a, b) = (metric(m, s, "base"), metric(m, s, "candidate"));
                    (if a == b { format_json(&a) } else { format!("{} -> {}", format_json(&a), format_json(&b)) }, a != b)
                };
                let cell = |ui: &mut Ui, text: String, hot: bool| {
                    let t = RichText::new(text).monospace();
                    ui.label(if hot { t.color(AMBER).strong() } else { t });
                };
                cell(ui, m.get("name").and_then(J::as_str).unwrap_or("?").to_string(), differs(m));
                for s in ["end", "min", "max"] {
                    let (text, hot) = pair(s);
                    cell(ui, text, hot);
                }
                let delta = m.get("delta").cloned().unwrap_or(J::Null);
                let zero = matches!(format_json(&delta).as_str(), "0" | "0.0" | "null");
                cell(ui, format_json(&delta), !zero);
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
