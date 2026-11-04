//! Explicit carve/restore of an inclusive cell rectangle in one terrain transaction.
use super::{index_field, Editor, TerrainPanel};
use orr_terrain::{Edit, Terrain, MAX_EDITS};

#[derive(Clone, Debug, Default)]
pub struct Spec {
    pub min: [u32; 2],
    pub max: [u32; 2],
    pub hole: bool,
}

/// Cells, not vertices. Reject the entire out-of-bounds or over-budget footprint,
/// including unchanged cells. Heights and cells outside the rectangle are untouched.
pub fn edits(
    terrain: &Terrain,
    min: [u32; 2],
    max: [u32; 2],
    hole: bool,
) -> Result<Vec<Edit>, String> {
    let sides = [terrain.width() - 1, terrain.depth() - 1];
    for axis in 0..2 {
        if min[axis] > max[axis] || max[axis] >= sides[axis] {
            return Err("Hole rectangle must be ordered and entirely inside terrain cells".into());
        }
    }
    // Validated terrain sides are at most 129: multiplication cannot overflow.
    let count = (max[0] - min[0] + 1) as usize * (max[1] - min[1] + 1) as usize;
    if count > MAX_EDITS {
        return Err(format!(
            "Hole rectangle exceeds {MAX_EDITS} cells; nothing changed"
        ));
    }
    let mut result = Vec::with_capacity(count);
    for z in min[1]..=max[1] {
        for x in min[0]..=max[0] {
            if terrain.holes()[(z * sides[0] + x) as usize] != hole {
                result.push(Edit::SetHole { x, z, hole });
            }
        }
    }
    Ok(result)
}

impl TerrainPanel {
    pub fn apply_hole_rectangle(&mut self, editor: &Editor) -> Result<(), String> {
        let result = (|| {
            self.require_attached(editor)?;
            let terrain = self
                .document
                .terrain()
                .ok_or("Create or open terrain first")?;
            let edits = edits(terrain, self.holes.min, self.holes.max, self.holes.hole)?;
            self.document.apply(&edits)?;
            self.refresh_selection();
            Ok(())
        })();
        self.report(result)
    }

    pub(super) fn show_holes(&mut self, ui: &mut egui::Ui, editor: &Editor, writable: bool) {
        ui.collapsing("Terrain hole rectangle", |ui| {
            ui.weak("Carve holes or restore existing heights. Inclusive cell coordinates, not vertices. Save is separate.");
            index_field(ui, "Hole minimum X", &mut self.holes.min[0]);
            index_field(ui, "Hole minimum Z", &mut self.holes.min[1]);
            index_field(ui, "Hole maximum X", &mut self.holes.max[0]);
            index_field(ui, "Hole maximum Z", &mut self.holes.max[1]);
            ui.checkbox(&mut self.holes.hole, "Carve holes (unchecked restores surface)");
            if let Some(t) = self.document.terrain() {
                ui.label(format!("Valid cells X: 0..={} · Z: 0..={} · maximum {MAX_EDITS} cells", t.width() - 2, t.depth() - 2));
            }
            let preview = self.document.terrain().ok_or_else(|| "Create or open terrain first".into())
                .and_then(|t| edits(t, self.holes.min, self.holes.max, self.holes.hole));
            match &preview {
                Ok(edits) if edits.is_empty() => { ui.label("No cells will change; history is preserved."); }
                Ok(edits) => { ui.label(format!("{} cells will change; one Undo step.", edits.len())); }
                Err(error) => { ui.colored_label(egui::Color32::YELLOW, error); }
            }
            if ui.add_enabled(writable && preview.is_ok(), egui::Button::new("Apply hole rectangle")).clicked() {
                let _ = self.apply_hole_rectangle(editor);
            }
        });
    }
}

#[cfg(test)]
mod tests;
