//! Explicit PNG preview/apply and independent presentation history.
use crate::{editor::Editor, terrain_panel::TerrainPanel, terrain_surface::State};
#[derive(Default)]
pub(crate) struct Panel {
    pub(crate) state: State,
    thumbnail: Option<([u8; 32], egui::TextureHandle)>,
    confirm_discard: bool,
}
impl Panel {
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, editor: &Editor, terrain: &TerrainPanel) {
        self.state.sync(editor, terrain);
        crate::theme::header("Terrain appearance").default_open(true).show(ui, |ui| {
            ui.weak("Presentation only. One opaque PNG, at most 512 × 512. Terrain and host history are unchanged.");
            ui.weak("Terrain Open does not reload appearance; use Reload appearance for external sidecar changes.");
            let editable = State::editable(editor).is_ok() && !self.state.operation_blocked;
            ui.add_enabled_ui(editable, |ui| {
                let label = ui.label("Appearance PNG path");
                if ui.text_edit_singleline(&mut self.state.png_path).labelled_by(label.id).changed() { self.state.cancel_preview(); self.thumbnail = None; }
                if ui.button("Preview appearance PNG").clicked() {
                    let result = self.state.preview(editor, terrain);
                    self.report(result, "Preview ready. Apply explicitly to change the terrain appearance.");
                }
                if let Some(surface) = self.state.preview_surface() {
                    let digest = surface.digest();
                    if self.thumbnail.as_ref().is_none_or(|(key, _)| *key != digest) {
                        let image = surface.image();
                        self.thumbnail = Some((digest, ui.ctx().load_texture("terrain-appearance-preview", egui::ColorImage::from_rgba_unmultiplied([image.width as usize, image.height as usize], &image.rgba8), egui::TextureOptions::NEAREST)));
                    }
                    if let Some((_, texture)) = &self.thumbnail { ui.image((texture.id(), egui::vec2(128.0, 128.0))); }
                }
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(self.state.pending(), egui::Button::new("Apply appearance")).clicked() {
                        let result = self.state.apply(editor, terrain); self.report(result, "Appearance applied; Save appearance to persist it.");
                    }
                    if ui.add_enabled(self.state.pending(), egui::Button::new("Cancel appearance preview")).clicked() { self.state.cancel_preview(); }
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(self.state.can_undo() && !self.state.pending(), egui::Button::new("Undo appearance")).clicked() {
                        let result = self.state.history(editor, terrain, false); self.report(result, "Appearance undone.");
                    }
                    if ui.add_enabled(self.state.can_redo() && !self.state.pending(), egui::Button::new("Redo appearance")).clicked() {
                        let result = self.state.history(editor, terrain, true); self.report(result, "Appearance redone.");
                    }
                    if ui.button("Save appearance").clicked() { let result = self.state.save(editor, terrain); self.report(result, "Appearance saved in the scene-relative sidecar. Source PNG is no longer needed."); }
                    if ui.button("Reload appearance").clicked() { let result = self.state.reload(editor, terrain); self.report(result, "Appearance reloaded."); }
                });
            });
            if !editable { ui.weak("Stop Play to edit appearance."); }
            if editable && self.state.busy() && ui.button("Discard appearance work…").clicked() { self.confirm_discard = true; }
            if editable && self.confirm_discard {
                ui.label("Discard pending preview and unsaved appearance history? Saved files remain unchanged.");
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Confirm discard appearance").clicked() { self.state.discard(); self.confirm_discard = false; }
                    if ui.button("Keep appearance work").clicked() { self.confirm_discard = false; }
                });
            }
            if self.state.dirty() { ui.label("Appearance has unsaved changes."); }
            if !self.state.status.is_empty() { ui.label(&self.state.status); }
        });
    }
    fn report(&mut self, result: Result<(), String>, success: &str) {
        self.state.status = result.err().unwrap_or_else(|| success.into());
    }
}
