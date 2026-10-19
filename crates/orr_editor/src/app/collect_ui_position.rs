//! Staged visual offset editing. The authoring panel remains the only commit
//! owner; this controller never touches a host, history, or filesystem.
use crate::{
    collect_ui_position::{Context, State},
    collect_ui_preview,
};
use orr_sample::authored_ui::{Document, Screen};

#[derive(Default, PartialEq)]
pub(super) enum Command {
    #[default]
    None,
    Apply,
    Cancel,
}

pub(super) struct PositionControls {
    pub(super) state: State,
    selected: Option<String>,
    pub(super) moving: bool,
    pub(super) scale: f32,
    status: String,
}
impl Default for PositionControls {
    fn default() -> Self {
        Self {
            state: State::default(),
            selected: None,
            moving: false,
            scale: 1.0,
            status: "Select a node ID, then enable Move UI position".into(),
        }
    }
}
impl PositionControls {
    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        screen: Screen,
        editable: bool,
    ) -> Command {
        let mut command = Command::None;
        let before = (self.moving, self.selected.clone(), self.scale);
        // Always reserve three rows. A status or Apply state must never move the
        // canvas under a captured pointer and invalidate its own gesture.
        ui.allocate_ui_with_layout(
            egui::vec2(660.0, 76.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.horizontal(|ui| {
                    ui.add_enabled(
                        editable,
                        egui::Checkbox::new(&mut self.moving, "Move UI position"),
                    );
                    egui::ComboBox::from_label("Move node ID")
                        .width(130.0)
                        .selected_text(self.selected.as_deref().unwrap_or("Select node"))
                        .show_ui(ui, |ui| {
                            for node in document.nodes.iter().filter(|node| node.screen == screen) {
                                ui.selectable_value(
                                    &mut self.selected,
                                    Some(node.id.clone()),
                                    &node.id,
                                );
                            }
                        });
                    ui.label("Display");
                    ui.selectable_value(&mut self.scale, 1.0, "100%");
                    ui.selectable_value(&mut self.scale, 0.75, "75%");
                });
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            editable && self.state.is_ready(),
                            egui::Button::new("Apply position"),
                        )
                        .clicked()
                    {
                        command = Command::Apply;
                    }
                    if ui
                        .add_enabled(
                            self.state.is_dragging() || self.state.is_ready(),
                            egui::Button::new("Cancel position"),
                        )
                        .clicked()
                    {
                        command = Command::Cancel;
                    }
                    ui.label("Drag canvas; only the selected node's offset changes");
                });
                ui.add(egui::Label::new(&self.status).truncate());
            },
        );
        if before != (self.moving, self.selected.clone(), self.scale) {
            command = Command::Cancel;
        }
        command
    }

    pub(super) fn cancel(&mut self, preview: &mut collect_ui_preview::State, message: &str) {
        let active = self.state.is_dragging() || self.state.is_ready();
        self.state.cancel();
        self.status = match if active {
            preview.cancel_staged()
        } else {
            Ok(())
        } {
            Ok(()) => message.into(),
            Err(error) => error,
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
            self.moving = false;
            self.cancel(preview, "Stop Play mode to edit UI positions");
        } else if let Err(error) = self.state.validate_context(context, document) {
            // Validation already cleared our draft; restore its staged preview
            // explicitly rather than asking the now-inactive owner to cancel.
            self.status = preview.cancel_staged().err().unwrap_or(error);
        }
    }

    pub(super) fn apply(
        &mut self,
        context: &Context,
        panel: &mut crate::collect_ui_panel::Panel,
        preview: &mut collect_ui_preview::State,
    ) {
        let result = self
            .state
            .candidate(context, panel.document())
            .and_then(|candidate| panel.apply_document(candidate));
        match result {
            Ok(()) => {
                self.state.cancel();
                self.status = "Position applied as one UI edit; Save UI document to keep it".into();
                // synchronize will replace the candidate with this accepted
                // document. Do not reconstruct the old accepted frame here.
            }
            Err(error) => self.cancel(preview, &error),
        }
    }

    pub(super) fn input(
        &mut self,
        ctx: &egui::Context,
        rect: egui::Rect,
        layer: egui::LayerId,
        context: &Context,
        document: &Document,
        preview: &mut collect_ui_preview::State,
    ) {
        if !self.moving {
            return;
        }
        let inside = |point| rect.contains(point) && ctx.layer_id_at(point) == Some(layer);
        let local = |point: egui::Pos2| [point.x - rect.min.x, point.y - rect.min.y];
        for event in ctx.input(|input| input.events.clone()) {
            let result = match event {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    ..
                } if inside(pos) => {
                    self.cancel(preview, "Drag to stage a new position");
                    match self.selected.as_deref() {
                        Some(id) => self.state.begin(context.clone(), document, id, local(pos)),
                        None => Err("Select a Move node ID first".into()),
                    }
                }
                egui::Event::PointerMoved(pos) if self.state.is_dragging() && inside(pos) => {
                    self.state.update(local(pos))
                }
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    ..
                } if self.state.is_dragging() && inside(pos) => {
                    self.state.finish(local(pos)).and_then(|()| {
                        if let Some(candidate) = self.state.document() {
                            preview.stage_document(candidate)?;
                            self.status =
                                "Staged position: Apply to accept, or Cancel to restore".into();
                        } else {
                            self.status = "No offset change".into();
                        }
                        Ok(())
                    })
                }
                egui::Event::PointerMoved(_)
                | egui::Event::PointerButton { .. }
                | egui::Event::PointerGone
                    if self.state.is_dragging() =>
                {
                    self.cancel(preview, "Position drag cancelled outside the canvas");
                    Ok(())
                }
                _ => Ok(()),
            };
            if let Err(error) = result {
                self.cancel(preview, &error);
            }
        }
        if self.state.is_dragging() {
            if let Some(offset) = self.state.offset() {
                self.status = format!(
                    "Dragging {} to offset [{}, {}]; release to preview",
                    self.state.selected_id().unwrap_or("node"),
                    offset[0],
                    offset[1]
                );
            }
        }
    }

    pub(super) fn paint_ghost(
        &self,
        ui: &egui::Ui,
        canvas: egui::Rect,
        document: &Document,
        viewport: [u32; 2],
    ) {
        if !self.state.is_dragging() {
            return;
        }
        let (Some(id), Some(offset)) = (self.state.selected_id(), self.state.offset()) else {
            return;
        };
        // This outline is a drag aid only. Authoritative pixels and hit regions
        // are generated by the unchanged production renderer once on release.
        let viewport = egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(viewport[0] as f32, viewport[1] as f32),
        );
        let mut bounds: Vec<(&str, egui::Rect)> = Vec::new();
        for node in &document.nodes {
            let parent = node
                .parent
                .as_deref()
                .and_then(|id| {
                    bounds
                        .iter()
                        .find(|(key, _)| *key == id)
                        .map(|(_, rect)| *rect)
                })
                .unwrap_or(viewport);
            let node_offset = if node.id == id { offset } else { node.offset };
            let rect = egui::Rect::from_min_size(
                parent.min
                    + egui::vec2(
                        parent.width() * f32::from(node.anchor[0]) / 1000.0
                            + f32::from(node_offset[0]),
                        parent.height() * f32::from(node.anchor[1]) / 1000.0
                            + f32::from(node_offset[1]),
                    ),
                egui::vec2(f32::from(node.size[0]), f32::from(node.size[1])),
            );
            bounds.push((&node.id, rect));
            if node.id == id {
                let scale = egui::vec2(
                    canvas.width() / viewport.width(),
                    canvas.height() / viewport.height(),
                );
                let ghost = egui::Rect::from_min_size(
                    canvas.min + rect.min.to_vec2() * scale,
                    rect.size() * scale,
                );
                ui.painter().with_clip_rect(canvas).rect_stroke(
                    ghost,
                    0.0,
                    egui::Stroke::new(2.0, egui::Color32::LIGHT_BLUE),
                    egui::StrokeKind::Inside,
                );
                break;
            }
        }
    }
}
