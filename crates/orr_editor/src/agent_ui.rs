//! The Agent tab of the bottom panel: open proposals, the selected one's
//! summary and diff, and verification. Draws and turns clicks into
//! [`Editor`] calls; the state is in [`crate::agent`] and [`Editor`].
//!
//! Layout (design doc 7.3, the AI AGENT panel): proposals on the left, the
//! proposal in the middle (buttons, summary, unified diff), the verify
//! controls and report on the right.

use std::collections::BTreeMap;

use egui::{Color32, RichText, Ui};
use orr_edit::{format_value, ProposalDiff, ProposalId, ProposalInfo, Target};
use orr_sim::MetricValue;

use crate::agent::{summary_line, VerifyRun, VerifySource};
use crate::app::EditorApp;
use crate::editor::{Editor, Mode};

/// Button labels (tests click them by these texts).
pub const LBL_PREVIEW: &str = "Preview";
/// See [`LBL_PREVIEW`].
pub const LBL_VERIFY: &str = "Verify";
/// See [`LBL_PREVIEW`].
pub const LBL_ACCEPT: &str = "Accept";
/// See [`LBL_PREVIEW`].
pub const LBL_REJECT: &str = "Reject";
/// The words of the verdict line when every check holds.
pub const TXT_PASSED: &str = "all checks passed";

const GREEN: Color32 = Color32::from_rgb(110, 210, 120);
const RED: Color32 = Color32::from_rgb(240, 100, 100);
const AMBER: Color32 = Color32::from_rgb(240, 190, 80);
const CYAN: Color32 = Color32::from_rgb(90, 200, 240);

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

enum Act {
    Select(ProposalId),
    Preview(ProposalId, bool),
    Verify(ProposalId),
    Accept(ProposalId),
    Reject(ProposalId),
}

impl EditorApp {
    /// Number of open proposals (the tab badge).
    pub(crate) fn proposal_count(&self) -> usize {
        self.editor.doc().list_proposals().len()
    }

    /// The Agent tab.
    pub(crate) fn agent_tab(&mut self, ui: &mut Ui) {
        let infos = self.editor.doc().list_proposals();
        self.ui.diffs.retain(|id, _| infos.iter().any(|i| i.id.0 == *id));
        let mut act: Option<Act> = None;
        if infos.is_empty() {
            ui.weak("No proposals. An AI agent stages changes here (ERP proposal.*), or use the script command `propose`; you review the diff, verify it against a replay and accept or reject it.");
            return;
        }
        // The selected proposal defaults to the first one.
        let selected = self.editor.agent().selected().filter(|s| infos.iter().any(|i| i.id == *s));
        if selected.is_none() {
            self.editor.select_proposal(infos.first().map(|i| i.id));
        }
        let selected = self.editor.agent().selected();

        egui::Panel::left("agent_list").resizable(true).default_size(300.0).show_separator_line(true).show(ui, |ui| {
            self.agent_list(ui, &infos, selected, &mut act);
        });
        egui::Panel::right("agent_verify").resizable(true).default_size(560.0).show(ui, |ui| {
            if let Some(id) = selected {
                self.agent_verify(ui, id);
            }
        });
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            if let Some(info) = infos.iter().find(|i| Some(i.id) == selected) {
                self.agent_detail(ui, info, &mut act);
            }
        });

        match act {
            Some(Act::Select(id)) => self.editor.select_proposal(Some(id)),
            Some(Act::Preview(id, on)) => {
                self.editor.set_preview(on.then_some(id));
            }
            Some(Act::Verify(id)) => {
                let source = self.editor.agent().source;
                let _ = self.editor.start_verify(id, source);
            }
            Some(Act::Accept(id)) => {
                let _ = self.editor.accept_proposal(id);
            }
            Some(Act::Reject(id)) => {
                let _ = self.editor.reject_proposal(id);
            }
            None => {}
        }
    }

    fn agent_list(&mut self, ui: &mut Ui, infos: &[ProposalInfo], selected: Option<ProposalId>, act: &mut Option<Act>) {
        ui.label(RichText::new(format!("Proposals ({})", infos.len())).strong());
        egui::ScrollArea::vertical().id_salt("agent_list_scroll").auto_shrink([false, false]).show(ui, |ui| {
            for info in infos {
                let head = format!("{}  {}", info.id, info.label);
                if ui.selectable_label(selected == Some(info.id), RichText::new(head).strong()).clicked() {
                    *act = Some(Act::Select(info.id));
                }
                let line = cached_diff(&self.editor, &mut self.ui.diffs, info.id).map_or_else(|| "?".to_string(), |d| summary_line(&d.summary));
                ui.horizontal_wrapped(|ui| {
                    ui.weak(format!("{} \u{b7} {} op{}", info.origin, info.op_count, if info.op_count == 1 { "" } else { "s" }));
                    if info.stale {
                        ui.label(RichText::new("stale").color(AMBER)).on_hover_text("The document changed since this proposal was made. It may still accept cleanly; Verify runs against the document as it is now.");
                    }
                    if self.editor.previewing() == Some(info.id) {
                        ui.label(RichText::new("[previewing]").color(CYAN));
                    }
                });
                ui.label(RichText::new(line).monospace());
                ui.add_space(4.0);
                ui.separator();
            }
        });
    }

    fn agent_detail(&mut self, ui: &mut Ui, info: &ProposalInfo, act: &mut Option<Act>) {
        let id = info.id;
        let editing = self.editor.mode() == Mode::Edit;
        let previewing = self.editor.previewing() == Some(id);
        let running = self.editor.agent().running().is_some();
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{}  {}", id, info.label)).strong().size(15.0));
            ui.weak(format!("[{}]", info.origin));
            if info.stale {
                ui.label(RichText::new("stale").color(AMBER));
            }
        });
        ui.horizontal(|ui| {
            let preview = ui.add_enabled(editing, egui::Button::new(LBL_PREVIEW).selected(previewing));
            if preview.on_hover_text("Draw the proposal's staged scene in the viewport (edit mode)").on_disabled_hover_text("Preview needs edit mode").clicked() {
                *act = Some(Act::Preview(id, !previewing));
            }
            if ui.add_enabled(!running, egui::Button::new(LBL_VERIFY)).on_hover_text("Replay base and candidate on the same inputs and run the checks").clicked() {
                *act = Some(Act::Verify(id));
            }
            if ui.add_enabled(editing, egui::Button::new(LBL_ACCEPT)).on_hover_text("Apply all ops as one undoable history entry").clicked() {
                *act = Some(Act::Accept(id));
            }
            if ui.button(LBL_REJECT).on_hover_text("Discard the proposal").clicked() {
                *act = Some(Act::Reject(id));
            }
        });
        if let Some(c) = self.editor.agent().conflict().filter(|c| c.proposal == id) {
            egui::Frame::group(ui.style()).stroke(egui::Stroke::new(1.5, RED)).show(ui, |ui| {
                ui.label(RichText::new(format!("Conflict: op {} no longer applies", c.op_index + 1)).color(RED).strong());
                ui.label(RichText::new(&c.op).monospace());
                ui.label(&c.cause);
                ui.weak("Nothing was changed. Reject the proposal or ask the agent to redo it.");
            });
        }
        ui.separator();

        let Some(diff) = cached_diff(&self.editor, &mut self.ui.diffs, id) else { return };
        ui.label(RichText::new("Changes").strong());
        summary_ui(ui, &self.editor, diff);
        ui.add_space(4.0);
        ui.label(RichText::new("Diff").strong());
        diff_ui(ui, &diff.text);
    }

    fn agent_verify(&mut self, ui: &mut Ui, id: ProposalId) {
        ui.label(RichText::new("Verification").strong());
        let running = self.editor.agent().running();
        let has_play = self.editor.last_stopped().is_some();
        let ag = self.editor.agent_mut();
        ui.horizontal(|ui| {
            ui.label("inputs");
            let mut bot = matches!(ag.source, VerifySource::Bot(_));
            let play_label = if has_play { "last play" } else { "last play (none recorded)" };
            let a = ui.radio_value(&mut bot, false, play_label).changed();
            let b = ui.radio_value(&mut bot, true, "bot").changed();
            let mut ticks = ag.bot_ticks;
            let dv = ui.add_enabled(bot, egui::DragValue::new(&mut ticks).range(1..=20_000).speed(5.0).suffix(" ticks"));
            if dv.changed() {
                ag.bot_ticks = ticks;
            }
            if a || b || dv.changed() {
                ag.source = if bot { VerifySource::Bot(ag.bot_ticks) } else { VerifySource::LastPlay };
            }
        });
        ui.label("checks (one per line)");
        ui.add(
            egui::TextEdit::multiline(&mut ag.checks)
                .font(egui::TextStyle::Monospace)
                .desired_rows(2)
                .desired_width(f32::INFINITY)
                .hint_text("lost_bodies.max == 0"),
        );
        if let Some(r) = running {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("verifying {r}\u{2026}"));
            });
        }
        ui.separator();
        match self.editor.verify_result() {
            None if running.is_none() => {
                ui.weak("No report yet. Verify replays the document and the proposal on the same inputs and compares them.");
            }
            None => {}
            Some(run) if run.proposal != id => {
                ui.weak(format!("The last report is for {}. Select it or run Verify for {id}.", run.proposal));
            }
            Some(run) => {
                egui::ScrollArea::vertical().id_salt("agent_report").auto_shrink([false, false]).show(ui, |ui| report_ui(ui, run));
            }
        }
    }
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
    egui::ScrollArea::both().id_salt("agent_diff").auto_shrink([false, false]).show_rows(ui, row_h, lines.len(), |ui, range| {
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
        ui.weak("(no change)");
        return;
    }
    let name_of = |g: &orr_reflect::Guid| {
        editor.view().entity(&Target::Guid(g.clone())).ok().and_then(|i| i.name).map_or_else(|| g.to_string(), |n| format!("{n} ({g})"))
    };
    egui::ScrollArea::vertical().id_salt("agent_summary").max_height(120.0).auto_shrink([false, true]).show(ui, |ui| {
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
    });
}

fn entity_text(e: &orr_edit::EntityRef) -> String {
    match &e.name {
        Some(n) => format!("{n} ({})", e.guid),
        None => e.guid.to_string(),
    }
}

fn changed(a: MetricValue, b: MetricValue) -> bool {
    a != b
}

/// The verification report: verdict, per-check results, divergence, metric table, timing.
fn report_ui(ui: &mut Ui, run: &VerifyRun) {
    let report = match &run.report {
        Ok(r) => r,
        Err(e) => {
            ui.label(RichText::new(format!("verification failed to run: {e}")).color(RED));
            return;
        }
    };
    if let Some(outcome) = &run.outcome {
        if outcome.results.is_empty() {
            ui.label(RichText::new("no checks were set (report only)").color(AMBER));
        } else if outcome.passed {
            ui.label(RichText::new(TXT_PASSED).color(GREEN).strong().size(15.0));
        } else {
            let bad = outcome.results.iter().filter(|r| !r.passed).count();
            ui.label(RichText::new(format!("{bad} of {} checks FAILED", outcome.results.len())).color(RED).strong().size(15.0));
        }
        for r in &outcome.results {
            let (mark, color) = if r.passed { ("\u{2714}", GREEN) } else { ("\u{2718}", RED) };
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(mark).color(color));
                ui.label(RichText::new(&r.check).monospace());
                ui.weak(format!("\u{2014} {}", r.reason));
            });
        }
    }
    ui.add_space(4.0);
    ui.weak(format!("{} ticks ({}..{}) on {} in {} ms", report.ticks, report.start_tick, report.end_tick, run.source.label(), run.millis));
    match report.first_divergence {
        None => ui.label("no checksum divergence"),
        Some(t) if t == report.start_tick => ui.label(format!("first checksum divergence: tick {t} (the scenes differ from the start)")),
        Some(t) => ui.label(format!("first checksum divergence: tick {t}")),
    };
    match report.first_metric_difference {
        None => ui.label("metrics identical at every sample"),
        Some(t) => ui.label(format!("first metric difference: tick {t}")),
    };
    ui.add_space(4.0);
    egui::Grid::new("agent_metric_grid").striped(true).spacing([14.0, 2.0]).show(ui, |ui| {
        for h in ["metric", "end (base -> cand)", "min", "max", "delta"] {
            ui.label(RichText::new(h).weak().small());
        }
        ui.end_row();
        for m in &report.metrics {
            let pair = |a: MetricValue, b: MetricValue| if a == b { a.to_string() } else { format!("{a} -> {b}") };
            let diff = changed(m.base.end, m.candidate.end) || changed(m.base.min, m.candidate.min) || changed(m.base.max, m.candidate.max);
            let cell = |ui: &mut Ui, text: String, hot: bool| {
                let t = RichText::new(text).monospace();
                ui.label(if hot { t.color(AMBER).strong() } else { t });
            };
            cell(ui, m.name.clone(), diff);
            cell(ui, pair(m.base.end, m.candidate.end), changed(m.base.end, m.candidate.end));
            cell(ui, pair(m.base.min, m.candidate.min), changed(m.base.min, m.candidate.min));
            cell(ui, pair(m.base.max, m.candidate.max), changed(m.base.max, m.candidate.max));
            cell(ui, m.delta.to_string(), m.delta != m.delta.zero_like());
            ui.end_row();
        }
    });
}
