//! One immutable-neighborhood pass, committed through the existing terrain session.
use super::{index_field, Editor, TerrainPanel};
use orr_fp::FP;
use orr_terrain::{Edit, Terrain, MAX_EDITS};

#[derive(Clone, Debug, Default)]
pub struct Spec {
    pub min: [u32; 2],
    pub max: [u32; 2],
}

/// Changed-only edits in z/x order. Every average reads the original terrain.
///
/// At terrain edges only existing neighbors count. Signed raw FP division rounds
/// toward zero. Any hole touching the full read-vertex halo rejects the operation;
/// this deliberately includes some cells outside the selected rectangle.
pub fn edits(terrain: &Terrain, min: [u32; 2], max: [u32; 2]) -> Result<Vec<Edit>, String> {
    let sides = [terrain.width(), terrain.depth()];
    for axis in 0..2 {
        if min[axis] > max[axis] || max[axis] >= sides[axis] {
            return Err(
                "Smooth rectangle must be ordered and entirely inside terrain vertices".into(),
            );
        }
    }
    // Terrain's validated sides are 2..=129, so these products and +1 are bounded.
    let count = (max[0] - min[0] + 1) as usize * (max[1] - min[1] + 1) as usize;
    if count > MAX_EDITS {
        return Err(format!(
            "Smooth rectangle exceeds {MAX_EDITS} vertices; nothing changed"
        ));
    }
    let cell_min = [min[0].saturating_sub(2), min[1].saturating_sub(2)];
    let cell_max = [
        (max[0] + 1).min(sides[0] - 2),
        (max[1] + 1).min(sides[1] - 2),
    ];
    for z in cell_min[1]..=cell_max[1] {
        for x in cell_min[0]..=cell_max[0] {
            if terrain.holes()[(z * (sides[0] - 1) + x) as usize] {
                return Err(format!(
                    "Smooth read halo touches hole at cell {x}, {z}; nothing changed"
                ));
            }
        }
    }
    let mut result = Vec::with_capacity(count);
    for z in min[1]..=max[1] {
        for x in min[0]..=max[0] {
            let mut sum = 0i128;
            let mut samples = 0i128;
            for nz in z.saturating_sub(1)..=(z + 1).min(sides[1] - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(sides[0] - 1) {
                    sum += i128::from(terrain.heights()[(nz * sides[0] + nx) as usize].raw());
                    samples += 1;
                }
            }
            // At most nine i64 values: sum cannot overflow i128. Their average
            // lies within the original height range; keep the conversion checked.
            let raw = i64::try_from(sum / samples).map_err(|_| "Smooth height outside FP range")?;
            let height = FP::from_raw(raw);
            if terrain.heights()[(z * sides[0] + x) as usize] != height {
                result.push(Edit::SetHeight { x, z, height });
            }
        }
    }
    Ok(result)
}

impl TerrainPanel {
    pub fn apply_smooth(&mut self, editor: &Editor) -> Result<(), String> {
        let result = (|| {
            self.require_attached(editor)?;
            let terrain = self
                .document
                .terrain()
                .ok_or("Create or open terrain first")?;
            let edits = edits(terrain, self.smooth.min, self.smooth.max)?;
            self.document.apply(&edits)?;
            self.refresh_selection();
            Ok(())
        })();
        self.report(result)
    }

    pub(super) fn show_smooth(&mut self, ui: &mut egui::Ui, editor: &Editor, writable: bool) {
        crate::theme::collapsing(ui, "Smooth terrain rectangle", |ui| {
            ui.weak(
                "One original 3×3 average pass; signed FP rounds toward zero. Save is separate.",
            );
            ui.weak("Nearby holes, including outside this rectangle, reject the whole pass.");
            index_field(ui, "Smooth minimum X", &mut self.smooth.min[0]);
            index_field(ui, "Smooth minimum Z", &mut self.smooth.min[1]);
            index_field(ui, "Smooth maximum X", &mut self.smooth.max[0]);
            index_field(ui, "Smooth maximum Z", &mut self.smooth.max[1]);
            if let Some(t) = self.document.terrain() {
                ui.label(format!(
                    "Inclusive X: 0..={} · Z: 0..={} · maximum {MAX_EDITS} vertices",
                    t.width() - 1,
                    t.depth() - 1
                ));
            }
            let preview = self
                .document
                .terrain()
                .ok_or_else(|| "Create or open terrain first".into())
                .and_then(|t| edits(t, self.smooth.min, self.smooth.max));
            match &preview {
                Ok(edits) => {
                    ui.label(format!(
                        "{} vertices will change; one Undo step. No clipping or falloff.",
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
                    egui::Button::new("Apply one smoothing pass"),
                )
                .clicked()
            {
                let _ = self.apply_smooth(editor);
            }
        });
    }
}

#[cfg(test)]
mod tests;
