//! Explicit inclusive vertex rectangles, applied as one existing terrain transaction.
use super::{index_field, parse_fixed, text_field, Editor, TerrainPanel};
use orr_fp::FP;
use orr_terrain::{Edit, Terrain, MAX_EDITS};

#[derive(Clone, Debug)]
pub struct Spec {
    pub min: [u32; 2],
    pub max: [u32; 2],
    pub height: String,
}
impl Default for Spec {
    fn default() -> Self {
        Self {
            min: [0; 2],
            max: [0; 2],
            height: "0".into(),
        }
    }
}

/// No clipping or falloff. Budget counts the whole footprint, including no-ops.
/// Holes and all vertices outside the rectangle remain untouched.
pub fn edits(
    terrain: &Terrain,
    min: [u32; 2],
    max: [u32; 2],
    height: FP,
) -> Result<Vec<Edit>, String> {
    let sides = [terrain.width(), terrain.depth()];
    for axis in 0..2 {
        if min[axis] > max[axis] || max[axis] >= sides[axis] {
            return Err(
                "Flatten rectangle must be ordered and entirely inside terrain vertices".into(),
            );
        }
    }
    let count = (max[0] - min[0] + 1) as usize * (max[1] - min[1] + 1) as usize;
    if count > MAX_EDITS {
        return Err(format!(
            "Flatten rectangle exceeds {MAX_EDITS} vertices; nothing changed"
        ));
    }
    let mut result = Vec::with_capacity(count);
    for z in min[1]..=max[1] {
        for x in min[0]..=max[0] {
            if terrain.heights()[(z * terrain.width() + x) as usize] != height {
                result.push(Edit::SetHeight { x, z, height });
            }
        }
    }
    Ok(result)
}

impl TerrainPanel {
    pub fn apply_flatten(&mut self, editor: &Editor) -> Result<(), String> {
        let result = (|| {
            self.require_attached(editor)?;
            let height = parse_fixed(&self.flatten.height)?;
            let terrain = self
                .document
                .terrain()
                .ok_or("Create or open terrain first")?;
            let edits = edits(terrain, self.flatten.min, self.flatten.max, height)?;
            self.document.apply(&edits)?;
            self.refresh_selection();
            Ok(())
        })();
        self.report(result)
    }

    pub(super) fn show_flatten(&mut self, ui: &mut egui::Ui, editor: &Editor, writable: bool) {
        ui.collapsing("Flatten terrain rectangle", |ui| {
            ui.weak("Inclusive vertex indices; no clipping or falloff. Holes stay holes.");
            index_field(ui, "Flatten minimum X", &mut self.flatten.min[0]);
            index_field(ui, "Flatten minimum Z", &mut self.flatten.min[1]);
            index_field(ui, "Flatten maximum X", &mut self.flatten.max[0]);
            index_field(ui, "Flatten maximum Z", &mut self.flatten.max[1]);
            text_field(ui, "Flatten target height", &mut self.flatten.height);
            if let Some(t) = self.document.terrain() {
                ui.label(format!(
                    "Valid X: 0..={} · Z: 0..={} · maximum {MAX_EDITS} vertices",
                    t.width() - 1,
                    t.depth() - 1
                ));
            }
            let preview = self
                .document
                .terrain()
                .ok_or_else(|| "Create or open terrain first".into())
                .and_then(|terrain| {
                    parse_fixed(&self.flatten.height).and_then(|height| {
                        edits(terrain, self.flatten.min, self.flatten.max, height)
                    })
                });
            match &preview {
                Ok(edits) => {
                    ui.label(format!(
                        "{} vertices will change; one Undo step. Save is separate.",
                        edits.len()
                    ));
                }
                Err(error) => {
                    ui.colored_label(egui::Color32::YELLOW, error);
                }
            }
            if ui
                .add_enabled(
                    writable && preview.is_ok(),
                    egui::Button::new("Apply flatten rectangle"),
                )
                .clicked()
            {
                let _ = self.apply_flatten(editor);
            }
        });
    }
}

#[cfg(test)]
mod tests;
