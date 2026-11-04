//! Circular terrain shaping through the reusable, GPU-free terrain operation.
use super::{index_field, parse_fixed, text_field, Editor, TerrainPanel};
use orr_terrain::sculpt::radial_edits;

#[derive(Clone, Debug)]
pub struct Spec {
    pub center: [u32; 2],
    pub radius: u32,
    pub delta: String,
}
impl Default for Spec {
    fn default() -> Self {
        Self {
            center: [0; 2],
            radius: 2,
            delta: "1".into(),
        }
    }
}

impl TerrainPanel {
    pub fn apply_sculpt(&mut self, editor: &Editor) -> Result<(), String> {
        let result = (|| {
            self.require_attached(editor)?;
            let terrain = self
                .document
                .terrain()
                .ok_or("Create or open terrain first")?;
            let edits = radial_edits(
                terrain,
                self.sculpt.center,
                self.sculpt.radius,
                parse_fixed(&self.sculpt.delta)?,
            )
            .map_err(|e| e.to_string())?;
            self.document.apply(&edits)?;
            self.refresh_selection();
            Ok(())
        })();
        self.report(result)
    }

    pub(super) fn show_sculpt(&mut self, ui: &mut egui::Ui, editor: &Editor, writable: bool) {
        ui.collapsing("Radial terrain sculpt", |ui| {
            ui.weak("Raise or lower a circular hill with parabolic falloff. Clipped at terrain edges; holes stay holes.");
            index_field(ui, "Sculpt center X", &mut self.sculpt.center[0]);
            index_field(ui, "Sculpt center Z", &mut self.sculpt.center[1]);
            index_field(ui, "Sculpt radius in grid intervals", &mut self.sculpt.radius);
            text_field(ui, "Sculpt center height delta", &mut self.sculpt.delta);
            if ui.add_enabled(writable, egui::Button::new("Use selected vertex as sculpt center")).clicked() {
                self.sculpt.center = self.selected_vertex;
            }
            let preview = self.document.terrain().ok_or_else(|| "Create or open terrain first".to_string())
                .and_then(|terrain| {
                    radial_edits(terrain, self.sculpt.center, self.sculpt.radius,
                        parse_fixed(&self.sculpt.delta)?).map_err(|e| e.to_string())
                });
            match &preview {
                Ok(edits) if edits.is_empty() => { ui.label("No vertices will change; history is preserved."); }
                Ok(edits) => { ui.label(format!("{} vertices will change; one Undo step. Save is separate.", edits.len())); }
                Err(error) => { ui.colored_label(egui::Color32::YELLOW, error); }
            }
            if ui.add_enabled(writable && preview.is_ok(), egui::Button::new("Apply radial sculpt")).clicked() {
                let _ = self.apply_sculpt(editor);
            }
        });
    }
}

#[cfg(test)]
mod tests;
