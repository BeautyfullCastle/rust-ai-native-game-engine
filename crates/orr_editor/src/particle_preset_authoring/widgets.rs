use super::Panel;
use orr_sample::particle_preset::PresetBlend;
use std::path::PathBuf;

/// Production standalone window; no renderer, bridge, seed or origin mutation.
#[derive(Default)]
pub struct App {
    panel: Panel,
    title_has_pending_draft: bool,
}
impl App {
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
                "Particle preset authoring · unapplied draft"
            } else {
                "Particle preset authoring"
            };
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Title(title.into()));
            self.title_has_pending_draft = pending;
        }
    }
}
fn field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.horizontal(|ui| {
        let response = ui.label(label);
        ui.add(
            egui::TextEdit::singleline(value)
                .char_limit(64)
                .desired_width(100.0),
        )
        .labelled_by(response.id);
    });
}
impl Panel {
    pub fn show(&mut self, ui: &mut egui::Ui) {
        ui.heading("Particle preset authoring");
        ui.label("Presentation-only burst preset. Runtime owns origin and seed.");
        ui.strong("Accepted preset · only this state is saved");
        ui.label(format!(
            "{} particles · {} seconds · {:?}",
            self.document().burst,
            self.document().lifetime,
            self.document().blend
        ));
        let pending = self.has_pending_draft();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !pending && self.can_undo(),
                    egui::Button::new("Undo preset edit"),
                )
                .clicked()
            {
                let result = self.undo();
                self.message = result.map_or_else(|e| e, |_| "Undid accepted preset edit.".into());
            }
            if ui
                .add_enabled(
                    !pending && self.can_redo(),
                    egui::Button::new("Redo preset edit"),
                )
                .clicked()
            {
                let result = self.redo();
                self.message = result.map_or_else(|e| e, |_| "Redid accepted preset edit.".into());
            }
            if ui
                .add_enabled(!pending, egui::Button::new("Use defaults"))
                .clicked()
            {
                let result = self.use_defaults();
                self.message = result.map_or_else(|e| e, |()| "Defaults accepted.".into());
            }
        });
        ui.separator();
        ui.strong(if self.has_pending_draft() {
            "Draft · unapplied changes"
        } else {
            "Draft · matches accepted preset"
        });
        field(ui, "Burst", &mut self.draft.burst);
        field(ui, "Lifetime seconds", &mut self.draft.lifetime);
        for (index, axis) in ["X", "Y", "Z"].iter().enumerate() {
            ui.horizontal(|ui| {
                field(
                    ui,
                    &format!("Velocity {axis}"),
                    &mut self.draft.velocity[index],
                );
                field(
                    ui,
                    &format!("Jitter {axis}"),
                    &mut self.draft.velocity_jitter[index],
                );
            });
        }
        ui.horizontal(|ui| {
            field(ui, "Start size", &mut self.draft.size_start);
            field(ui, "End size", &mut self.draft.size_end);
        });
        for (index, channel) in ["R", "G", "B", "A"].iter().enumerate() {
            ui.horizontal(|ui| {
                field(
                    ui,
                    &format!("Start {channel}"),
                    &mut self.draft.color_start[index],
                );
                field(
                    ui,
                    &format!("End {channel}"),
                    &mut self.draft.color_end[index],
                );
            });
        }
        egui::ComboBox::from_label("Blend")
            .selected_text(match self.draft.blend {
                PresetBlend::Alpha => "Alpha",
                PresetBlend::Additive => "Additive",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.draft.blend, PresetBlend::Alpha, "Alpha");
                ui.selectable_value(&mut self.draft.blend, PresetBlend::Additive, "Additive");
            });
        ui.horizontal(|ui| {
            if ui.button("Apply draft").clicked() {
                let result = self.apply_draft();
                self.message = result.map_or_else(
                    |e| e,
                    |()| "Draft applied. Save As to write accepted state.".into(),
                );
            }
            if ui
                .add_enabled(self.has_pending_draft(), egui::Button::new("Discard draft"))
                .clicked()
            {
                self.discard_draft();
                self.message = "Draft discarded; accepted preset unchanged.".into();
            }
        });
        ui.small("Resolve drafts before Open, history or Defaults. Invalid Apply changes nothing.");
        ui.separator();
        let label = ui.label("Preset file path");
        ui.add(
            egui::TextEdit::singleline(&mut self.path)
                .char_limit(4096)
                .desired_width(f32::INFINITY),
        )
        .labelled_by(label.id);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.has_pending_draft(),
                    egui::Button::new("Open preset (undoable)"),
                )
                .clicked()
            {
                let result = self.open(&PathBuf::from(&self.path));
                self.message = result.map_or_else(|e| e, |()| "Opened validated preset.".into());
            }
            if ui.button("Save As new file").clicked() {
                let result = self.save_as(&PathBuf::from(&self.path));
                self.message = result.map_or_else(
                    |e| e,
                    |()| "Saved accepted preset; drafts are not saved.".into(),
                );
            }
        });
        ui.small("Existing files are never overwritten. History: 32 accepted edits.");
        if !self.message().is_empty() {
            ui.label(self.message());
        }
        ui.small("Preview saved file with particle_demo --preset FILE. No embedded preview.");
    }
}
