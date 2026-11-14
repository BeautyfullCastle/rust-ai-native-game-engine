//! Explicit inclusive vertex rectangles, applied as one existing terrain transaction.
use super::{index_field, parse_fixed, text_field, Editor, TerrainPanel};
use orr_fp::FP;
use orr_terrain::{Edit, Terrain, MAX_EDITS};

#[derive(Clone, Debug)]
pub struct Spec {
    pub min: [u32; 2],
    pub max: [u32; 2],
    pub base: String,
    pub step: String,
}
impl Default for Spec {
    fn default() -> Self {
        Self {
            min: [0; 2],
            max: [0; 2],
            base: "0".into(),
            step: "1".into(),
        }
    }
}

/// No clipping or falloff. Budget counts the whole footprint, including no-ops.
/// Holes and all vertices outside the rectangle remain untouched.
pub fn edits(
    terrain: &Terrain,
    min: [u32; 2],
    max: [u32; 2],
    base: FP,
    step: FP,
) -> Result<Vec<Edit>, String> {
    if step.raw() <= 0 {
        return Err("Terrace step must be positive".into());
    }
    let sides = [terrain.width(), terrain.depth()];
    for axis in 0..2 {
        if min[axis] > max[axis] || max[axis] >= sides[axis] {
            return Err(
                "Terrace rectangle must be ordered and entirely inside terrain vertices".into(),
            );
        }
    }
    let count = (max[0] - min[0] + 1) as usize * (max[1] - min[1] + 1) as usize;
    if count > MAX_EDITS {
        return Err(format!(
            "Terrace rectangle exceeds {MAX_EDITS} vertices; nothing changed"
        ));
    }
    let mut result = Vec::with_capacity(count);
    for z in min[1]..=max[1] {
        for x in min[0]..=max[0] {
            let original = terrain.heights()[(z * terrain.width() + x) as usize];
            let delta = i128::from(original.raw()) - i128::from(base.raw());
            let bands = delta.div_euclid(i128::from(step.raw()));
            let raw = i128::from(base.raw())
                .checked_add(
                    bands
                        .checked_mul(i128::from(step.raw()))
                        .ok_or("Terrace multiplication overflow")?,
                )
                .ok_or("Terrace addition overflow")?;
            let height = FP::from_raw(
                i64::try_from(raw)
                    .map_err(|_| "Terrace height exceeds fixed-point range; nothing changed")?,
            );
            if height != original {
                result.push(Edit::SetHeight { x, z, height });
            }
        }
    }
    Ok(result)
}

impl TerrainPanel {
    pub fn apply_terrace(&mut self, editor: &Editor) -> Result<(), String> {
        let result = (|| {
            self.require_attached(editor)?;
            let base = parse_fixed(&self.terrace.base)?;
            let step = parse_fixed(&self.terrace.step)?;
            let terrain = self
                .document
                .terrain()
                .ok_or("Create or open terrain first")?;
            let edits = edits(terrain, self.terrace.min, self.terrace.max, base, step)?;
            self.document.apply(&edits)?;
            self.refresh_selection();
            Ok(())
        })();
        self.report(result)
    }

    pub(super) fn show_terrace(&mut self, ui: &mut egui::Ui, editor: &Editor, writable: bool) {
        crate::theme::collapsing(ui, "Terrace terrain rectangle", |ui| {
            ui.weak("Floor heights to base + n * positive step (including negative bands). Inclusive vertices; holes and outside heights stay unchanged.");
            index_field(ui, "Terrace minimum X", &mut self.terrace.min[0]);
            index_field(ui, "Terrace minimum Z", &mut self.terrace.min[1]);
            index_field(ui, "Terrace maximum X", &mut self.terrace.max[0]);
            index_field(ui, "Terrace maximum Z", &mut self.terrace.max[1]);
            text_field(ui, "Terrace base height", &mut self.terrace.base);
            text_field(ui, "Terrace positive step", &mut self.terrace.step);
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
                    parse_fixed(&self.terrace.base).and_then(|base|parse_fixed(&self.terrace.step).and_then(|step|edits(terrain,self.terrace.min,self.terrace.max,base,step)))
                });
            match &preview {
                Ok(edits) => {
                    if edits.is_empty() {ui.label("0 vertices will change; no Undo step.");}
                    else {ui.label(format!("{} vertices will change; one Undo step. Save is separate.",edits.len()));}
                }
                Err(error) => {
                    ui.colored_label(egui::Color32::YELLOW, error);
                }
            }
            if ui
                .add_enabled(
                    writable && self.require_attached(editor).is_ok() && preview.is_ok(),
                    egui::Button::new("Apply terrace rectangle"),
                )
                .clicked()
            {
                let _ = self.apply_terrace(editor);
            }
        });
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, target_os = "linux"))]
#[path = "terrace_integration_tests.rs"]
mod integration_tests;
