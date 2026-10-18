//! The production standalone window and its keyboard-only authoring widgets.
use super::Panel;
use orr_input::{Button, Key, MAX_BINDINGS};
use std::path::Path;

const KEYS: &[Key] = &[
    Key::A,
    Key::B,
    Key::C,
    Key::D,
    Key::E,
    Key::F,
    Key::G,
    Key::H,
    Key::I,
    Key::J,
    Key::K,
    Key::L,
    Key::M,
    Key::N,
    Key::O,
    Key::P,
    Key::Q,
    Key::R,
    Key::S,
    Key::T,
    Key::U,
    Key::V,
    Key::W,
    Key::X,
    Key::Y,
    Key::Z,
    Key::ArrowLeft,
    Key::ArrowRight,
    Key::ArrowUp,
    Key::ArrowDown,
    Key::Space,
    Key::Escape,
    Key::Enter,
    Key::Tab,
];

/// Standalone authoring app. It owns no project, runtime or game-input lifecycle.
#[derive(Default)]
pub struct App {
    panel: Panel,
    title_has_pending_draft: bool,
}

impl App {
    /// Read-only authoring state, also used by the acceptance harness.
    pub fn panel(&self) -> &Panel {
        &self.panel
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.panel.show(ui));
        });
        let pending = self.panel().has_pending_draft();
        if pending != self.title_has_pending_draft {
            let title = if pending {
                "Collect input-map authoring · unapplied draft"
            } else {
                "Collect input-map authoring"
            };
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Title(title.into()));
            self.title_has_pending_draft = pending;
        }
    }
}

impl Panel {
    pub fn show(&mut self, ui: &mut egui::Ui) {
        ui.heading("Collect input map");
        ui.label("Six fixed actions · keyboard only · up to eight keys per action");
        ui.small("Explicit session file for Collect. This tool does not launch a game or change a project.");
        ui.separator();
        ui.strong("Accepted map · only these bindings are saved");
        egui::Grid::new("accepted_collect_bindings")
            .num_columns(4)
            .spacing([18.0, 3.0])
            .show(ui, |ui| {
                for (index, action) in self.document().actions.iter().enumerate() {
                    ui.label(&action.name);
                    ui.monospace(bindings_label(&action.bindings));
                    if index % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
        for action in self.document().actions.iter().filter(|action| {
            matches!(action.name.as_str(), "restart" | "quit") && action.bindings.is_empty()
        }) {
            let consequence = if action.name == "restart" {
                "The keyboard cannot restart Collect."
            } else {
                "Use the normal window close control to exit."
            };
            ui.colored_label(
                egui::Color32::YELLOW,
                format!("Warning: {} is Unbound. {consequence}", action.name),
            );
        }
        let pending = self.has_pending_draft();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !pending && self.can_undo(),
                    egui::Button::new("Undo map edit"),
                )
                .clicked()
            {
                let result = self.undo();
                self.set_message(
                    result.map_or_else(|error| error, |_| "Undid accepted map edit.".into()),
                );
            }
            if ui
                .add_enabled(
                    !pending && self.can_redo(),
                    egui::Button::new("Redo map edit"),
                )
                .clicked()
            {
                let result = self.redo();
                self.set_message(
                    result.map_or_else(|error| error, |_| "Redid accepted map edit.".into()),
                );
            }
            if ui
                .add_enabled(!pending, egui::Button::new("Use Collect defaults"))
                .clicked()
            {
                let result = self.use_defaults();
                self.set_message(
                    result.map_or_else(|error| error, |()| "Collect defaults accepted.".into()),
                );
            }
        });
        ui.separator();
        ui.strong(if pending {
            "Action draft · unapplied changes"
        } else {
            "Action draft · matches accepted map"
        });
        let mut selected = self.selected_action_index();
        ui.add_enabled_ui(!pending, |ui| {
            egui::ComboBox::from_label("Action")
                .selected_text(&self.selected_action().name)
                .show_ui(ui, |ui| {
                    for (index, action) in self.document().actions.iter().enumerate() {
                        ui.selectable_value(&mut selected, index, &action.name);
                    }
                });
        });
        if selected != self.selected_action_index() {
            if let Err(error) = self.select_action(selected) {
                self.set_message(error);
            }
        }
        let mut draft = self.draft().to_vec();
        let mut remove = None;
        for (index, binding) in draft.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                egui::ComboBox::from_label(format!("Binding {}", index + 1))
                    .selected_text(button_label(*binding))
                    .show_ui(ui, |ui| {
                        for &key in KEYS {
                            ui.selectable_value(
                                binding,
                                Button::Keyboard { key },
                                format!("Key {key:?}"),
                            );
                        }
                    });
                if ui.button(format!("Remove binding {}", index + 1)).clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            draft.remove(index);
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(draft.len() < MAX_BINDINGS, egui::Button::new("Add binding"))
                .clicked()
            {
                draft.push(Button::Keyboard { key: Key::Z });
            }
            if ui.button("Clear action draft").clicked() {
                draft.clear();
            }
        });
        if draft != self.draft() {
            if let Err(error) = self.replace_draft(draft) {
                self.set_message(error);
            }
        }
        ui.horizontal(|ui| {
            if ui.button("Apply action").clicked() {
                let result = self.apply_draft();
                self.set_message(result.map_or_else(
                    |error| error,
                    |()| "Action applied to accepted map. Save As to write a new file.".into(),
                ));
            }
            if ui
                .add_enabled(self.has_pending_draft(), egui::Button::new("Discard draft"))
                .clicked()
            {
                self.discard_draft();
                self.set_message("Draft discarded. Accepted map unchanged.");
            }
        });
        ui.small("Apply or Discard a pending draft before switching action, Open, Undo, Redo or Defaults.");
        ui.separator();
        ui.label("File path for explicit Open or Save As");
        ui.add(
            egui::TextEdit::singleline(&mut self.path)
                .hint_text("/path/to/new-collect-input.json")
                .char_limit(4096)
                .desired_width(f32::INFINITY),
        );
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.has_pending_draft(),
                    egui::Button::new("Open map (undoable)"),
                )
                .clicked()
            {
                let path = std::path::PathBuf::from(&self.path);
                let result = self.open(&path);
                self.set_message(result.map_or_else(
                    |error| error,
                    |()| "Opened and validated Collect map.".into(),
                ));
            }
            if ui.button("Save As new file").clicked() {
                let result = self.save_as(Path::new(&self.path));
                self.set_message(result.map_or_else(
                    |error| error,
                    |()| "Saved accepted map. Unapplied action drafts are not saved.".into(),
                ));
            }
        });
        ui.small("Existing files are never overwritten. Save As preserves any pending draft. History: 32 map edits.");
        if !self.message().is_empty() {
            ui.label(self.message());
        }
        ui.separator();
        ui.label("Load explicitly: collect_dodge --project DIR --input-bindings FILE");
        ui.small(
            "Player requires collect-dodge + input-actions. Maps are window-only and session-only.",
        );
    }
}

fn bindings_label(bindings: &[Button]) -> String {
    if bindings.is_empty() {
        return "Unbound".into();
    }
    bindings
        .iter()
        .map(|&binding| button_label(binding))
        .collect::<Vec<_>>()
        .join(" / ")
}

fn button_label(button: Button) -> String {
    match button {
        Button::Keyboard { key } => format!("Key {key:?}"),
        // Admission never permits these variants; keep rendering total without
        // offering unsupported devices as selectable authoring choices.
        Button::Mouse { .. } | Button::Gamepad { .. } => "Unsupported binding".into(),
    }
}
