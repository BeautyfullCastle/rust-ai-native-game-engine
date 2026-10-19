use super::Panel;
use orr_sample::particle_preset::PresetBlend;
use std::path::PathBuf;

/// Production standalone authoring; optional preview is isolated presentation state.
#[derive(Default)]
pub struct App {
    panel: Panel,
    title_has_pending_draft: bool,
    #[cfg(feature = "particle-preset-preview")]
    render_state: Option<egui_wgpu::RenderState>,
    #[cfg(feature = "particle-preset-preview")]
    preview: super::preview::Preview,
    #[cfg(feature = "particle-preset-preview")]
    preview_gpu: Option<super::preview_gpu::PreviewGpu>,
    #[cfg(feature = "particle-preset-preview")]
    preview_clock: Option<f64>,
    #[cfg(feature = "particle-preset-preview")]
    preview_error: String,
}
impl App {
    pub fn new(render_state: Option<egui_wgpu::RenderState>) -> Self {
        #[cfg(not(feature = "particle-preset-preview"))]
        let _ = render_state;
        Self {
            #[cfg(feature = "particle-preset-preview")]
            render_state,
            ..Self::default()
        }
    }
    pub fn panel(&self) -> &Panel {
        &self.panel
    }
}
impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // A close frame may still contain a mesh sampling this texture. Release
        // its registration only on the following update, after that frame paints.
        #[cfg(feature = "particle-preset-preview")]
        if !self.preview.active() {
            self.preview_gpu = None;
        }
        egui::CentralPanel::default().show(ui, |ui| {
            #[cfg(feature = "particle-preset-preview")]
            self.preview_toolbar(ui);
            egui::ScrollArea::vertical().show(ui, |ui| self.panel.show(ui));
        });
        #[cfg(feature = "particle-preset-preview")]
        self.preview_window(ui.ctx());
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
        #[cfg(not(feature = "particle-preset-preview"))]
        ui.small(
            "Preview saved file with particle_demo --preset FILE. Embedded preview feature is off.",
        );
    }
}

#[cfg(feature = "particle-preset-preview")]
impl App {
    #[cfg(test)]
    pub fn preview(&self) -> &super::preview::Preview {
        &self.preview
    }
    #[cfg(test)]
    pub fn preview_gpu(&self) -> Option<&super::preview_gpu::PreviewGpu> {
        self.preview_gpu.as_ref()
    }
    fn preview_toolbar(&mut self, ui: &mut egui::Ui) {
        if ui.button("Preview accepted preset").clicked() {
            self.preview_clock = None;
            self.preview_error.clear();
            match self.preview.start(self.panel.document()) {
                Err(error) => self.preview_error = error.to_string(),
                Ok(()) => {
                    if self.preview_gpu.is_none() {
                        match self.render_state.as_ref() {
                            Some(state) => match super::preview_gpu::PreviewGpu::new(state) {
                                Ok(gpu) => self.preview_gpu = Some(gpu),
                                Err(error) => self.preview_error = error.to_string(),
                            },
                            None => {
                                self.preview_error =
                                    "GPU preview unavailable; authoring remains available.".into()
                            }
                        }
                    }
                }
            }
        }
        if self.panel.has_pending_draft() {
            ui.label("Unapplied draft excluded from preview");
        }
        if self
            .preview
            .snapshot()
            .is_some_and(|p| p != self.panel.document())
        {
            ui.label("Preview is stale · click Preview accepted preset to refresh");
        }
    }
    fn preview_window(&mut self, ctx: &egui::Context) {
        if !self.preview.active() {
            self.preview_clock = None;
            return;
        }
        let mut open = true;
        let mut close = false;
        let mut visible = false;
        egui::Window::new("Particle preview")
            .open(&mut open)
            .resizable(false)
            .default_pos([400.0, 80.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Restart preview").clicked() {
                        self.preview_clock = None;
                        if let Err(error) = self.preview.restart() {
                            self.preview_error = error.to_string();
                        }
                    }
                    if self.preview.playing() {
                        if ui.button("Pause preview").clicked() {
                            self.preview.pause();
                            self.preview_clock = None;
                        }
                    } else if ui.button("Resume preview").clicked() {
                        self.preview.resume();
                        self.preview_clock = None;
                    }
                    close = ui.button("Close preview").clicked();
                });
                let (time, focused, root_visible) = ctx.input(|i| {
                    (
                        i.time,
                        i.focused
                            && !i
                                .events
                                .iter()
                                .any(|event| matches!(event, egui::Event::WindowFocused(false))),
                        i.viewport().visible() != Some(false),
                    )
                });
                visible = root_visible && ui.is_visible() && !close;
                if visible && focused && self.preview.playing() {
                    if let Some(previous) = self.preview_clock.replace(time) {
                        // Wall-clock stalls cannot produce an unbounded presentation step.
                        #[allow(clippy::float_arithmetic)]
                        let dt = (time - previous).clamp(0.0, 0.25) as f32;
                        if let Err(error) = self.preview.advance(dt, focused, visible) {
                            self.preview_error = error.to_string();
                        }
                    }
                } else {
                    self.preview_clock = None;
                }
                if let Some(gpu) = self.preview_gpu.as_mut() {
                    let draws: Vec<_> = self.preview.draws().collect();
                    if let Err(error) = gpu.render(&draws) {
                        self.preview_error = error.to_string();
                    }
                    let (width, height) = gpu.size();
                    ui.image((gpu.texture_id(), egui::vec2(width as f32, height as f32)));
                    if visible && focused && self.preview.playing() && !draws.is_empty() {
                        ctx.request_repaint();
                    }
                }
                if !self.preview_error.is_empty() {
                    ui.label(&self.preview_error);
                }
                if self.panel.has_pending_draft() {
                    ui.label("Unapplied draft excluded from preview");
                }
                if self
                    .preview
                    .snapshot()
                    .is_some_and(|p| p != self.panel.document())
                {
                    ui.label("Preview is stale · refresh to show accepted changes");
                }
            });
        if !visible {
            self.preview_clock = None;
        }
        if !open || close {
            self.preview.close();
            self.preview_clock = None;
            ctx.request_repaint();
        }
    }
}
