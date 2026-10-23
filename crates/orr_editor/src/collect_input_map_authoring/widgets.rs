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
        egui::CentralPanel::default().show(ui, |ui| self.panel.show(ui));
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
        // Resolve physical input before any ordinary widget can use logical keys,
        // text or paste. Keep the underlying UI disabled for the completion frame
        // as well, so a captured Enter/Space cannot also Apply or Save.
        let owned_on_entry = self.process_capture(ui.ctx());
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_enabled_ui(!owned_on_entry, |ui| self.show_contents(ui));
        });
        self.show_capture_modal(ui.ctx());
    }

    fn process_capture(&mut self, ctx: &egui::Context) -> bool {
        let owned_on_entry = self.capture.is_listening();
        let (events, focused) = ctx.input(|input| (input.events.clone(), input.focused));
        let output = self.capture.process(&events, self.capture_context, focused);
        if owned_on_entry {
            isolate_capture_input(ctx);
        }
        if let Some(message) = output.message {
            self.capture_message = message;
        }
        if let Some((row, key)) = output.captured {
            // A row is always an existing draft binding. Context invalidation
            // prevents stale captures from being applied after document edits.
            let mut draft = self.draft().to_vec();
            if let Some(binding) = draft.get_mut(row) {
                *binding = Button::Keyboard { key };
                if let Err(error) = self.replace_draft(draft) {
                    self.capture_message = error;
                } else {
                    self.capture_message = format!(
                        "Captured Key {key:?} for binding {}. Apply action to accept it.",
                        row + 1
                    );
                }
            }
        }
        owned_on_entry
    }

    fn begin_capture(&mut self, row: usize, ctx: &egui::Context) {
        self.capture.begin(row, self.capture_context);
        self.capture_message.clear();
        isolate_capture_input(ctx);
    }

    fn show_capture_modal(&mut self, ctx: &egui::Context) {
        let Some(row) = self.capture_row() else {
            return;
        };
        egui::Modal::new(egui::Id::new("collect_physical_key_capture")).show(ctx, |ui| {
            ui.heading("Capture physical key");
            ui.label(format!(
                "Action {} · Binding {}",
                self.selected_action().name,
                row + 1
            ));
            ui.label("Release held keys, then press one physical key without modifiers.");
            ui.label("Escape cancels capture. Use the dropdown to bind Escape.");
            if !self.capture_message().is_empty() {
                ui.colored_label(egui::Color32::YELLOW, self.capture_message());
            }
            let cancel = ui.button("Cancel capture");
            if cancel.clicked() {
                self.capture.cancel();
                self.capture_message = "Capture cancelled. Draft unchanged.".into();
                cancel.surrender_focus();
            } else {
                // egui's logical Escape/Tab navigation must never decide capture
                // identity or leave an underlying text field owning the keyboard.
                if !cancel.has_focus() {
                    cancel.request_focus();
                }
                ui.memory_mut(|memory| {
                    memory.set_focus_lock_filter(
                        cancel.id,
                        egui::EventFilter {
                            tab: true,
                            horizontal_arrows: true,
                            vertical_arrows: true,
                            escape: true,
                        },
                    )
                });
            }
        });
        // Deliberately do not use ModalResponse::should_close(): it interprets
        // logical Escape, whereas this modal belongs to the physical key state.
    }

    fn show_contents(&mut self, ui: &mut egui::Ui) {
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
        let mut capture_row = None;
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
                if ui
                    .button(format!("Capture binding {}", index + 1))
                    .clicked()
                {
                    capture_row = Some(index);
                }
                if ui.button(format!("Remove binding {}", index + 1)).clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            draft.remove(index);
        }
        if capture_row.is_some() {
            // Also isolate the activation frame from widgets below the row.
            isolate_capture_input(ui.ctx());
            ui.disable();
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
        if let Some(row) = capture_row.filter(|&row| row < self.draft().len()) {
            self.begin_capture(row, ui.ctx());
        }
        if let Err(error) = self.validate_draft() {
            ui.colored_label(egui::Color32::LIGHT_RED, format!("Draft error: {error}"));
        }
        let candidate = self.draft_candidate();
        if self
            .document()
            .actions
            .iter()
            .chain(&candidate.actions)
            .any(|action| {
                action.name != "quit"
                    && action
                        .bindings
                        .contains(&Button::Keyboard { key: Key::Escape })
            })
        {
            ui.colored_label(egui::Color32::YELLOW,
                "Compatibility warning: Escape on movement or restart is valid for standalone Collect, but cannot be loaded by the main editor, where Escape releases controls.");
        }
        if !self.capture_is_listening() && !self.capture_message().is_empty() {
            ui.label(self.capture_message());
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

/// Remove capture-owned keyboard-derived events for this whole frame, including
/// events after the deciding key. Pointer input remains available to Cancel.
fn isolate_capture_input(ctx: &egui::Context) {
    ctx.input_mut(|input| {
        input.events.retain(|event| {
            !matches!(
                event,
                egui::Event::Key { .. }
                    | egui::Event::Text(_)
                    | egui::Event::Paste(_)
                    | egui::Event::Copy
                    | egui::Event::Cut
                    | egui::Event::Ime(_)
            )
        });
    });
    ctx.memory_mut(|memory| {
        memory.move_focus(egui::FocusDirection::None);
    });
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
