//! Cues of the making flow: which entity is selected, edit or play, unsaved
//! or saved, where a refusal happened and what to do next.
//!
//! The cues only read state that the panels already have. They change no
//! document, take no keyboard focus and send no request to the host.

use std::path::PathBuf;

use egui::{RichText, Ui};

use crate::model::Target;
use crate::theme;

/// The menu path that starts a project of the user in this build.
#[cfg(all(windows, target_arch = "x86_64", feature = "windows-projects"))]
pub const NEW_PROJECT_ROUTE: &str = "File > New Room project\u{2026}";
/// The menu path that starts a project of the user in this build.
#[cfg(not(all(windows, target_arch = "x86_64", feature = "windows-projects")))]
pub const NEW_PROJECT_ROUTE: &str = "File > Open scene\u{2026}";

/// What the window knows about the work at this moment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// The simulation host is gone.
    pub host_down: bool,
    /// A replay viewer: nothing can be edited.
    pub viewer: bool,
    /// The simulation plays.
    pub playing: bool,
    /// The scene has unsaved changes.
    pub dirty: bool,
    /// An entity is selected.
    pub selected: bool,
    /// The open scene is the built-in sample, not a project of the user.
    pub sample: bool,
}

/// One short sentence for the status bar: the next useful action.
pub fn next_step(facts: Facts) -> String {
    if facts.host_down {
        "Host stopped \u{B7} use the Restart button above".into()
    } else if facts.viewer {
        "Replay viewer \u{B7} read-only".into()
    } else if facts.playing {
        "Playing \u{B7} Stop returns to editing".into()
    } else if facts.dirty {
        "Unsaved \u{B7} Ctrl+S saves, Ctrl+Z undoes".into()
    } else if !facts.selected && facts.sample {
        format!("Select an entity, or {NEW_PROJECT_ROUTE}")
    } else if !facts.selected {
        "Select an entity in Hierarchy to edit it".into()
    } else {
        "Edit in Inspector \u{B7} Ctrl+S saves \u{B7} Play tests".into()
    }
}

/// A refusal that belongs to the Inspector: shown there, next to the fields.
struct Notice {
    target: Option<Target>,
    serial: u64,
    text: String,
}

/// Window state of the cues. It is never saved and never part of a document.
#[derive(Default)]
pub struct State {
    /// `None` until the first frame. Then the path of the built-in sample if it was opened.
    sample: Option<Option<PathBuf>>,
    notice: Option<Notice>,
    /// Messages up to this serial came with the start of the editor. They are not news.
    startup_serial: u64,
    /// The newest message and the time the window first saw it.
    seen: Option<(u64, f64)>,
}

/// How long a message of success stays in the status bar before the next step replaces it.
pub const INFO_SECONDS: f64 = 8.0;

impl State {
    /// Remembers on the first frame whether the editor started on the built-in sample.
    pub fn observe_start(&mut self, path: Option<PathBuf>, message_serial: u64) {
        if self.sample.is_none() {
            self.startup_serial = message_serial;
            // The host may report the path in another spelling: compare the real files once.
            let sample = std::fs::canonicalize(crate::editor::default_scene_path()).ok();
            self.sample = Some(path.filter(|path| sample.is_some() && std::fs::canonicalize(path).ok() == sample));
        }
    }

    /// True while the status bar shows this message. A refusal stays until a newer message
    /// replaces it. A message of success goes after [`INFO_SECONDS`], and a message of the
    /// start never shows. The next step takes the place of a message that does not show.
    pub fn status_fresh(&mut self, serial: u64, error: bool, now: f64) -> bool {
        if error {
            return true;
        }
        if serial <= self.startup_serial {
            return false;
        }
        let first = match self.seen {
            Some((seen, first)) if seen == serial => first,
            _ => {
                self.seen = Some((serial, now));
                now
            }
        };
        now - first < INFO_SECONDS
    }

    /// True while the open scene is still the built-in sample.
    pub fn on_sample(&self, path: Option<PathBuf>) -> bool {
        match (&self.sample, path) {
            (Some(Some(sample)), Some(path)) => *sample == path,
            _ => false,
        }
    }

    /// Drops a refusal that was shown for another selection.
    pub fn observe_selection(&mut self, target: Option<&Target>) {
        if self.notice.as_ref().is_some_and(|notice| notice.target.as_ref() != target) {
            self.notice = None;
        }
    }

    /// An edit of the selected entity was refused with this message.
    pub fn refuse(&mut self, target: Option<Target>, serial: u64, text: String) {
        self.notice = Some(Notice { target, serial, text });
    }

    /// A later edit worked: the old refusal no longer applies.
    pub fn clear_notice(&mut self) {
        self.notice = None;
    }

    /// The refusal to show for the selected entity. A newer status message replaces it.
    pub fn notice_for(&self, target: Option<&Target>, serial: u64) -> Option<&str> {
        self.notice
            .as_ref()
            .filter(|notice| notice.serial == serial && notice.target.as_ref() == target)
            .map(|notice| notice.text.as_str())
    }
}

/// The refusal text, in the error color, with the way out.
pub fn refusal(ui: &mut Ui, text: &str) {
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, theme::ERROR))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS))
        .inner_margin(egui::Margin::symmetric(6, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(format!("Not applied: {text}")).color(theme::ERROR));
            ui.label(RichText::new("The field keeps its old value. Type a new value or press Ctrl+Z.").size(11.5).color(theme::TEXT_WEAK));
        });
}

/// The cue at the top of the Inspector while the simulation plays.
pub fn play_cue(ui: &mut Ui) {
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, theme::PLAY))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS))
        .inner_margin(egui::Margin::symmetric(6, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Play mode: edits here are temporary.").color(theme::PLAY));
            ui.label(RichText::new("The scene file does not change. Press Stop to edit the scene.").size(11.5).color(theme::TEXT_WEAK));
        });
}

/// What the Inspector shows when nothing is selected.
pub fn nothing_selected(ui: &mut Ui, sample: bool) {
    ui.label(RichText::new("Nothing selected").strong().color(theme::TEXT_STRONG));
    ui.label("Select an entity to edit it. Click a row in Hierarchy or an entity in the viewport.");
    if sample {
        ui.add_space(4.0);
        ui.label(RichText::new(format!("This is the built-in sample scene. To start your own project, use {NEW_PROJECT_ROUTE}")).color(theme::TEXT_WEAK));
    }
}
