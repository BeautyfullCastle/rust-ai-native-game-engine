//! Raise or lower an inclusive vertex rectangle without flattening its shape.
use super::{index_field, parse_fixed, text_field, Editor, TerrainPanel};
use orr_fp::FP;
use orr_terrain::{Edit, Terrain, MAX_EDITS};

#[derive(Clone, Debug)]
pub struct Spec {
    pub min: [u32; 2],
    pub max: [u32; 2],
    pub delta: String,
}
impl Default for Spec {
    fn default() -> Self {
        Self {
            min: [0; 2],
            max: [0; 2],
            delta: "0".into(),
        }
    }
}

/// No clipping or falloff. Budget counts the whole footprint, including no-ops.
/// Holes and all vertices outside the rectangle remain untouched.
/// Every addition is checked before any transaction is applied; overflow rejects all edits.
pub fn edits(
    terrain: &Terrain,
    min: [u32; 2],
    max: [u32; 2],
    delta: FP,
) -> Result<Vec<Edit>, String> {
    let sides = [terrain.width(), terrain.depth()];
    for axis in 0..2 {
        if min[axis] > max[axis] || max[axis] >= sides[axis] {
            return Err(
                "Offset rectangle must be ordered and entirely inside terrain vertices".into(),
            );
        }
    }
    let count = (max[0] - min[0] + 1) as usize * (max[1] - min[1] + 1) as usize;
    if count > MAX_EDITS {
        return Err(format!(
            "Offset rectangle exceeds {MAX_EDITS} vertices; nothing changed"
        ));
    }
    let mut result = Vec::with_capacity(count);
    for z in min[1]..=max[1] {
        for x in min[0]..=max[0] {
            let original = terrain.heights()[(z * terrain.width() + x) as usize];
            let raw = original
                .raw()
                .checked_add(delta.raw())
                .ok_or("Offset height exceeds fixed-point range; nothing changed")?;
            if delta != FP::ZERO {
                result.push(Edit::SetHeight {
                    x,
                    z,
                    height: FP::from_raw(raw),
                });
            }
        }
    }
    Ok(result)
}

impl TerrainPanel {
    pub fn apply_offset(&mut self, editor: &Editor) -> Result<(), String> {
        let result = (|| {
            self.require_attached(editor)?;
            let height = parse_fixed(&self.offset.delta)?;
            let terrain = self
                .document
                .terrain()
                .ok_or("Create or open terrain first")?;
            let edits = edits(terrain, self.offset.min, self.offset.max, height)?;
            self.document.apply(&edits)?;
            self.refresh_selection();
            Ok(())
        })();
        self.report(result)
    }

    pub(super) fn show_offset(&mut self, ui: &mut egui::Ui, editor: &Editor, writable: bool) {
        crate::theme::collapsing(ui, "Offset terrain rectangle", |ui| {
            ui.weak("Add a signed height delta while preserving shape. No clipping or falloff. Holes stay holes.");
            index_field(ui, "Offset minimum X", &mut self.offset.min[0]);
            index_field(ui, "Offset minimum Z", &mut self.offset.min[1]);
            index_field(ui, "Offset maximum X", &mut self.offset.max[0]);
            index_field(ui, "Offset maximum Z", &mut self.offset.max[1]);
            text_field(ui, "Offset height delta", &mut self.offset.delta);
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
                    parse_fixed(&self.offset.delta).and_then(|height| {
                        edits(terrain, self.offset.min, self.offset.max, height)
                    })
                });
            match &preview {
                Ok(edits) => {
                    if edits.is_empty() {
                        ui.label("No vertices will change; history is preserved.");
                    } else {
                        ui.label(format!(
                            "{} vertices will change; one Undo step. Save is separate.",
                            edits.len()
                        ));
                    }
                }
                Err(error) => {
                    ui.colored_label(egui::Color32::YELLOW, error);
                }
            }
            if ui
                .add_enabled(
                    writable && preview.is_ok(),
                    egui::Button::new("Apply offset rectangle"),
                )
                .clicked()
            {
                let _ = self.apply_offset(editor);
            }
        });
    }
}

#[cfg(test)]
mod tests;
