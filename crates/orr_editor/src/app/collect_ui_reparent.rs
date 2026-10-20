//! Explicit, document-bound subtree reparent staging through the production preview.
use crate::{
    collect_ui_position::Context,
    collect_ui_preview,
    collect_ui_reparent::{Proposal, propose},
};
use orr_sample::authored_ui::{Document, Kind, Screen};

#[derive(Default)]
pub(super) enum Command {
    #[default]
    None,
    Stage,
    Apply,
    Cancel,
}
struct Draft {
    context: Context,
    original: Document,
    source: String,
    target: Option<String>,
    candidate: Document,
}
#[derive(Default)]
pub(super) struct ReparentControls {
    pub(super) reparenting: bool,
    source: Option<String>,
    target: Option<String>,
    draft: Option<Draft>,
    status: String,
}
impl ReparentControls {
    pub(super) fn is_ready(&self) -> bool {
        self.draft.is_some()
    }
    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        screen: Screen,
        editable: bool,
    ) -> Command {
        let before = (self.reparenting, self.source.clone(), self.target.clone());
        let mut command = Command::None;
        ui.allocate_ui_with_layout(
            egui::vec2(660.0, 80.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.horizontal(|ui| {
                    ui.add_enabled(
                        editable,
                        egui::Checkbox::new(&mut self.reparenting, "Reparent UI subtree"),
                    );
                    egui::ComboBox::from_label("Reparent source")
                        .width(130.0)
                        .height(180.0)
                        .selected_text(self.source.as_deref().unwrap_or("Select source"))
                        .show_ui(ui, |ui| {
                            for node in document.nodes.iter().filter(|node| node.screen == screen) {
                                ui.selectable_value(
                                    &mut self.source,
                                    Some(node.id.clone()),
                                    &node.id,
                                );
                            }
                        });
                    egui::ComboBox::from_label("Reparent target")
                        .width(130.0)
                        .height(180.0)
                        .selected_text(self.target.as_deref().unwrap_or("Viewport"))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.target, None, "Viewport");
                            if let Some(index) = document.nodes.iter().position(|node| {
                                Some(&node.id) == self.source.as_ref() && node.screen == screen
                            }) {
                                for node in document.nodes[..index].iter().filter(|node| {
                                    node.screen == screen && matches!(node.kind, Kind::Container)
                                }) {
                                    ui.selectable_value(
                                        &mut self.target,
                                        Some(node.id.clone()),
                                        &node.id,
                                    );
                                }
                            }
                        });
                });
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            editable && self.reparenting && self.source.is_some(),
                            egui::Button::new("Stage reparent"),
                        )
                        .clicked()
                    {
                        command = Command::Stage;
                    }
                    if ui
                        .add_enabled(
                            editable && self.is_ready(),
                            egui::Button::new("Apply reparent"),
                        )
                        .clicked()
                    {
                        command = Command::Apply;
                    }
                    if ui
                        .add_enabled(self.is_ready(), egui::Button::new("Cancel reparent"))
                        .clicked()
                    {
                        command = Command::Cancel;
                    }
                    ui.label("New parent clipping can change visibility and hit testing");
                });
                ui.add(
                    egui::Label::new(if self.status.is_empty() {
                        "Keeps subtree positions at this viewport; other viewport sizes may differ"
                    } else {
                        &self.status
                    })
                    .truncate(),
                );
            },
        );
        if before != (self.reparenting, self.source.clone(), self.target.clone()) {
            command = Command::Cancel;
        }
        command
    }
    pub(super) fn cancel(&mut self, preview: &mut collect_ui_preview::State, message: &str) {
        self.status = if self.draft.take().is_some() {
            preview
                .cancel_staged()
                .err()
                .unwrap_or_else(|| message.into())
        } else {
            message.into()
        };
    }
    fn current(&self, context: &Context, document: &Document) -> bool {
        self.draft.as_ref().is_some_and(|draft| {
            self.reparenting
                && draft.context == *context
                && draft.original == *document
                && Some(&draft.source) == self.source.as_ref()
                && draft.target == self.target
        })
    }
    pub(super) fn validate(
        &mut self,
        context: &Context,
        document: &Document,
        editable: bool,
        preview: &mut collect_ui_preview::State,
    ) {
        if !editable {
            self.reparenting = false;
            self.cancel(preview, "Stop Play mode to reparent UI");
        } else if self.is_ready() && !self.current(context, document) {
            self.cancel(preview, "Reparent context changed; staged edit cancelled");
        }
    }
    pub(super) fn stage(
        &mut self,
        context: &Context,
        document: &Document,
        preview: &mut collect_ui_preview::State,
    ) {
        self.cancel(preview, "Reparent cleared");
        let Some(source) = self.source.as_deref() else {
            return;
        };
        if !self.reparenting
            || !document
                .nodes
                .iter()
                .any(|node| node.id == source && node.screen == context.screen)
        {
            self.status = "Select a source on the active screen".into();
            return;
        }
        match propose(document, source, self.target.as_deref(), context.viewport) {
            Ok(Proposal::Unchanged) => self.status = "Same parent: no edit or undo entry".into(),
            Ok(Proposal::Changed(candidate)) => match preview.stage_document(&candidate) {
                Ok(()) => {
                    self.draft = Some(Draft {
                        context: context.clone(),
                        original: document.clone(),
                        source: source.into(),
                        target: self.target.clone(),
                        candidate,
                    });
                    self.status = "Staged reparent: inspect clipping, then Apply or Cancel".into();
                }
                Err(error) => self.status = error,
            },
            Err(error) => self.status = error,
        }
    }
    pub(super) fn apply(
        &mut self,
        context: &Context,
        panel: &mut crate::collect_ui_panel::Panel,
        preview: &mut collect_ui_preview::State,
    ) {
        if !self.current(context, panel.document()) {
            self.cancel(preview, "Stale reparent refused; stage again");
            return;
        }
        let candidate = self
            .draft
            .as_ref()
            .expect("validated draft")
            .candidate
            .clone();
        match panel.apply_document(candidate) {
            Ok(()) => {
                self.draft = None;
                self.status = "Reparent applied as one UI edit; Save UI document to keep it".into();
            }
            Err(error) => self.cancel(preview, &error),
        }
    }
}
