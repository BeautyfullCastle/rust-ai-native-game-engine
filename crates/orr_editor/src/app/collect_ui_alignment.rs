//! Sibling alignment drafts, committed only through the authoring panel.
use crate::{
    collect_ui_alignment::{Context, Mode, State},
    collect_ui_preview,
};
use orr_sample::authored_ui::{Document, Screen};

#[derive(Default)]
pub(super) enum Command {
    #[default]
    None,
    Stage(Mode),
    Apply,
    Cancel,
}

#[derive(Default)]
pub(super) struct AlignmentControls {
    pub(super) state: State,
    pub(super) aligning: bool,
    reference: Option<String>,
    selected: Vec<String>,
    status: String,
}
impl AlignmentControls {
    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        screen: Screen,
        editable: bool,
    ) -> Command {
        let before = (self.aligning, self.reference.clone(), self.selected.clone());
        let mut command = Command::None;
        // Popups hold potentially long ID lists; the strip has invariant height.
        ui.allocate_ui_with_layout(egui::vec2(660.0, 100.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.horizontal(|ui| {
                ui.add_enabled(editable, egui::Checkbox::new(&mut self.aligning, "Align siblings"));
                egui::ComboBox::from_label("Alignment reference")
                    .width(130.0).height(180.0)
                    .selected_text(self.reference.as_deref().unwrap_or("Select reference"))
                    .show_ui(ui, |ui| {
                        for node in document.nodes.iter().filter(|node| node.screen == screen) {
                            ui.selectable_value(&mut self.reference, Some(node.id.clone()), &node.id);
                        }
                    });
                if before.1 != self.reference { self.selected.clear(); }
                egui::containers::menu::MenuButton::new("Alignment targets")
                    .config(egui::containers::menu::MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside))
                    .ui(ui, |ui| {
                    ui.set_max_width(240.0);
                    egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                        if let Some(reference) = self.reference.as_deref() {
                            if let Ok(ids) = crate::collect_ui_alignment::sibling_ids(document, screen, reference) {
                                for id in ids.into_iter().filter(|id| id != reference) {
                                    let mut checked = self.selected.contains(&id);
                                    if ui.checkbox(&mut checked, &id).changed() {
                                        if checked { self.selected.push(id); } else { self.selected.retain(|selected| selected != &id); }
                                    }
                                }
                            }
                        } else { ui.label("Choose a reference first"); }
                    });
                });
            });
            ui.horizontal(|ui| {
                for (mode, label) in [(Mode::Left,"Left"), (Mode::Center,"Center"), (Mode::Right,"Right"), (Mode::Top,"Top"), (Mode::Middle,"Middle"), (Mode::Bottom,"Bottom")] {
                    if ui.add_enabled(editable && self.aligning && self.reference.is_some() && !self.selected.is_empty(), egui::Button::new(label)).clicked() { command = Command::Stage(mode); }
                }
                if ui.add_enabled(editable && self.state.is_ready(), egui::Button::new("Apply alignment")).clicked() { command = Command::Apply; }
                if ui.add_enabled(self.state.is_ready(), egui::Button::new("Cancel alignment")).clicked() { command = Command::Cancel; }
            });
            ui.add(egui::Label::new("Reference stays fixed; integer offsets round nearest (ties away from zero), ≤0.5 px error").truncate());
            ui.add(egui::Label::new(if self.status.is_empty() { "Choose a reference and at least one sibling target" } else { &self.status }).truncate());
        });
        if before != (self.aligning, self.reference.clone(), self.selected.clone()) {
            command = Command::Cancel;
        }
        command
    }
    pub(super) fn cancel(&mut self, preview: &mut collect_ui_preview::State, message: &str) {
        let active = self.state.is_ready();
        self.state.cancel();
        self.status = if active {
            preview
                .cancel_staged()
                .err()
                .unwrap_or_else(|| message.into())
        } else {
            message.into()
        };
    }
    pub(super) fn validate(
        &mut self,
        context: &Context,
        document: &Document,
        editable: bool,
        preview: &mut collect_ui_preview::State,
    ) {
        if !editable {
            self.aligning = false;
            self.cancel(preview, "Stop Play mode to align siblings");
        } else if let Err(error) = self.state.validate_context(context, document) {
            self.status = preview.cancel_staged().err().unwrap_or(error);
        }
    }
    pub(super) fn stage(
        &mut self,
        mode: Mode,
        context: &Context,
        document: &Document,
        preview: &mut collect_ui_preview::State,
    ) {
        // Restore the accepted document before replacing any previous draft,
        // including a new no-op or a rejected operation.
        self.cancel(preview, "Alignment cleared");
        let Some(reference) = self.reference.as_deref() else {
            return;
        };
        let mut selected = vec![reference.to_owned()];
        selected.extend(self.selected.clone());
        let result = self
            .state
            .stage(context.clone(), document, &selected, reference, mode)
            .and_then(|()| {
                if let Some(candidate) = self.state.document() {
                    preview.stage_document(candidate)?;
                    self.status = "Staged alignment: Apply to accept, or Cancel to restore".into();
                } else {
                    self.status = "No alignment change".into();
                }
                Ok(())
            });
        if let Err(error) = result {
            self.cancel(preview, &error);
        }
    }
    pub(super) fn apply(
        &mut self,
        context: &Context,
        panel: &mut crate::collect_ui_panel::Panel,
        preview: &mut collect_ui_preview::State,
    ) {
        match self
            .state
            .candidate(context, panel.document())
            .and_then(|candidate| panel.apply_document(candidate))
        {
            Ok(()) => {
                self.state.cancel();
                self.status =
                    "Alignment applied as one UI edit; Save UI document to keep it".into();
            }
            Err(error) => self.cancel(preview, &error),
        }
    }
}
