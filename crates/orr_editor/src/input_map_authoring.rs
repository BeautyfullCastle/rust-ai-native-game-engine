//! Optional standalone Arena input-map authoring. No simulation or project writes.
//! This module is compiled only by the feature-gated tool and its acceptance target.
use orr_input::{ActionMap, Button, Key, MAX_BINDINGS};
use orr_sample::arena_input::{default_map, validate_map};
use std::path::Path;

const MAX_HISTORY: usize = 32;
const KEYS: &[Key] = &[
    Key::A, Key::B, Key::C, Key::D, Key::E, Key::F, Key::G, Key::H, Key::I,
    Key::J, Key::K, Key::L, Key::M, Key::N, Key::O, Key::P, Key::Q, Key::R,
    Key::S, Key::T, Key::U, Key::V, Key::W, Key::X, Key::Y, Key::Z,
    Key::ArrowLeft, Key::ArrowRight, Key::ArrowUp, Key::ArrowDown,
    Key::Space, Key::Escape, Key::Enter, Key::Tab,
];

/// Only accepted complete maps enter document history. Widget draft edits do not.
pub struct Panel {
    document: ActionMap,
    undo: Vec<ActionMap>,
    redo: Vec<ActionMap>,
    selected: usize,
    draft: Vec<Button>,
    path: String,
    message: String,
}

impl Default for Panel {
    fn default() -> Self {
        let document = default_map();
        Self {
            draft: document.actions[0].bindings.clone(), document,
            undo: Vec::new(), redo: Vec::new(), selected: 0,
            path: String::new(), message: String::new(),
        }
    }
}

impl Panel {
    pub fn document(&self) -> &ActionMap { &self.document }
    pub fn message(&self) -> &str { &self.message }
    pub fn can_undo(&self) -> bool { !self.undo.is_empty() }
    pub fn can_redo(&self) -> bool { !self.redo.is_empty() }

    fn reset_draft(&mut self) {
        self.selected = self.selected.min(self.document.actions.len() - 1);
        self.draft = self.document.actions[self.selected].bindings.clone();
    }

    /// Stage only the selected action's defaults; Apply remains the sole commit.
    fn restore_action_defaults(&mut self) -> Result<(), String> {
        let name = &self.document.actions.get(self.selected)
            .ok_or("Select an Arena action before restoring its defaults")?.name;
        let action = default_map().actions.into_iter().find(|action| &action.name == name)
            .ok_or("Selected action has no Arena defaults")?;
        self.draft = action.bindings;
        Ok(())
    }

    pub fn apply_document(&mut self, candidate: ActionMap) -> Result<(), String> {
        validate_map(&candidate)?;
        // Preserve the actual existing encoded-size contract as well as schema limits.
        candidate.save(Vec::new())?;
        if candidate != self.document {
            if self.undo.len() == MAX_HISTORY { self.undo.remove(0); }
            self.undo.push(std::mem::replace(&mut self.document, candidate));
            self.redo.clear();
        }
        self.reset_draft();
        Ok(())
    }

    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop() else { return false; };
        self.redo.push(std::mem::replace(&mut self.document, previous));
        self.reset_draft();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else { return false; };
        self.undo.push(std::mem::replace(&mut self.document, next));
        self.reset_draft();
        true
    }

    /// Explicit Open replaces the current map as one undoable edit, only after admission.
    pub fn open(&mut self, path: &Path) -> Result<(), String> {
        let candidate = ActionMap::load_file(path)?;
        self.apply_document(candidate)
    }

    /// Never overwrites. Neither success nor failure changes document/history.
    pub fn save_as(&self, path: &Path) -> Result<(), String> {
        validate_map(&self.document)?;
        self.document.save_new(path)
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        ui.heading("Arena input map");
        ui.label(format!("{} actions in the accepted map", self.document().actions.len()));
        ui.label("Seven fixed actions · keyboard and mouse · no gamepad adapter");
        ui.label("Changes below are drafts until Apply action. Changing action discards its pending draft.");
        ui.horizontal(|ui| {
            if ui.add_enabled(self.can_undo(), egui::Button::new("Undo map edit")).clicked() {
                self.undo(); self.message.clear();
            }
            if ui.add_enabled(self.can_redo(), egui::Button::new("Redo map edit")).clicked() {
                self.redo(); self.message.clear();
            }
            if ui.button("Use Arena defaults").clicked() {
                self.message = self.apply_document(default_map()).err().unwrap_or_default();
            }
        });
        let previous = self.selected;
        egui::ComboBox::from_label("Action")
            .selected_text(&self.document.actions[self.selected].name)
            .show_ui(ui, |ui| {
                for (index, action) in self.document.actions.iter().enumerate() {
                    ui.selectable_value(&mut self.selected, index, &action.name);
                }
            });
        if previous != self.selected { self.reset_draft(); self.message.clear(); }
        let mut remove = None;
        for (index, binding) in self.draft.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                egui::ComboBox::from_label(format!("Binding {}", index + 1))
                    .selected_text(button_label(*binding))
                    .show_ui(ui, |ui| {
                        for &key in KEYS {
                            ui.selectable_value(binding, Button::Keyboard { key }, format!("Key {key:?}"));
                        }
                        for button in 0..=7 {
                            ui.selectable_value(binding, Button::Mouse { button }, format!("Mouse {button}"));
                        }
                    });
                if ui.button(format!("Remove binding {}", index + 1)).clicked() { remove = Some(index); }
            });
        }
        if let Some(index) = remove { self.draft.remove(index); }
        ui.horizontal(|ui| {
            if ui.add_enabled(self.draft.len() < MAX_BINDINGS, egui::Button::new("Add binding")).clicked() {
                // A deliberate, visible default; Apply reports any existing conflict.
                self.draft.push(Button::Keyboard { key: Key::Z });
            }
            if ui.button("Clear action draft").clicked() { self.draft.clear(); }
            if ui.button("Restore action defaults").clicked() {
                self.message = self.restore_action_defaults().map_or_else(|error| error,
                    |()| "Default bindings loaded into this action draft. Apply action to confirm.".into());
            }
            if ui.button("Apply action").clicked() {
                let mut candidate = self.document.clone();
                candidate.actions[self.selected].bindings.clone_from(&self.draft);
                self.message = match self.apply_document(candidate) {
                    Ok(()) => "Action applied. Save As to publish a new file.".into(),
                    Err(error) => error,
                };
            }
        });
        if ui.add_enabled(
            self.draft != self.document.actions[self.selected].bindings,
            egui::Button::new("Discard action draft"),
        ).clicked() {
            self.reset_draft();
            self.message.clear();
        }
        ui.separator();
        ui.label("Path for explicit Open or Save As (existing files are never overwritten)");
        ui.add(egui::TextEdit::singleline(&mut self.path).hint_text("/path/to/new-bindings.json").char_limit(4096));
        ui.horizontal(|ui| {
            if ui.button("Open map (undoable)").clicked() {
                let path = std::path::PathBuf::from(&self.path);
                self.message = self.open(&path).map_or_else(|error| error, |()| "Opened and validated Arena map.".into());
            }
            if ui.button("Save As new file").clicked() {
                self.message = self.save_as(Path::new(&self.path)).map_or_else(|error| error, |()| "Saved. Arena accepts this file with --input-bindings. Pending action drafts are not saved.".into());
            }
        });
        ui.label("Open, Undo, Redo and Defaults also discard pending action drafts. Empty actions are allowed by Arena.");
        if !self.message.is_empty() { ui.label(self.message()); }
        ui.separator();
        ui.label("Load explicitly: arena --input-bindings FILE. This tool never starts a runtime or changes player preferences.");
    }
}

fn button_label(button: Button) -> String {
    match button {
        Button::Keyboard { key } => format!("Key {key:?}"),
        Button::Mouse { button } => format!("Mouse {button}"),
        Button::Gamepad { .. } => "Unsupported gamepad".into(),
    }
}
