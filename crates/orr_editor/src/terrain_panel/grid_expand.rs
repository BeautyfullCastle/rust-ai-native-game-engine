//! Explicit +X/+Z grid growth; source vertices and cells keep their coordinates.
use super::{parse_fixed, text_field, Editor, TerrainPanel};
use orr_fp::FP;
use orr_terrain::{Terrain, MAX_SIDE};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    pub dimensions: [u32; 2],
    pub fill: String,
}
impl Default for Spec {
    fn default() -> Self {
        Self::for_dimensions([9, 9])
    }
}
impl Spec {
    pub fn for_dimensions(dimensions: [u32; 2]) -> Self {
        Self {
            dimensions,
            fill: "0".into(),
        }
    }

    /// Pure decimal and bounds admission; no allocation or document mutation.
    pub fn request(&self, terrain: &Terrain) -> Result<Request, String> {
        let fill = parse_fixed(&self.fill)?;
        let [width, depth] = self.dimensions;
        terrain
            .validate_expansion(width, depth)
            .map_err(|e| e.to_string())?;
        Ok(Request { width, depth, fill })
    }

    fn show_controls(&mut self, ui: &mut egui::Ui, terrain: &Terrain, writable: bool) -> bool {
        ui.weak("Grow toward +X and +Z only. Existing vertices and holes keep their coordinates; new vertices use the fill height and new cells are solid. Save terrain and navigation Build are separate actions.");
        for (axis, label) in ["Grid expansion target width", "Grid expansion target depth"]
            .into_iter()
            .enumerate()
        {
            ui.horizontal(|ui| {
                let label = ui.label(label);
                ui.add(
                    egui::DragValue::new(&mut self.dimensions[axis])
                        .range(2..=MAX_SIDE)
                        .speed(1),
                )
                .labelled_by(label.id);
            });
        }
        text_field(ui, "Grid expansion fill height", &mut self.fill);
        ui.label(format!(
            "Current grid: {} x {}; targets: 2..={MAX_SIDE} vertices per side, without shrinking",
            terrain.width(),
            terrain.depth()
        ));
        #[cfg(feature = "navigation")]
        if self.dimensions.iter().any(|side| *side > 17) {
            ui.weak("Navigation rebuild keeps its existing 17 x 17 grid limit.");
        }
        let request = self.request(terrain);
        match &request {
            Ok(request) if request.width == terrain.width() && request.depth == terrain.depth() => {
                ui.label("Dimensions unchanged; no Undo step.");
            }
            Ok(request) => {
                let added = request.width * request.depth - terrain.width() * terrain.depth();
                ui.label(format!(
                    "{added} new vertices; one terrain Undo step. Save is separate."
                ));
            }
            Err(error) => {
                ui.colored_label(egui::Color32::YELLOW, error);
            }
        }
        ui.add_enabled(
            writable && request.is_ok(),
            egui::Button::new("Apply grid expansion"),
        )
        .clicked()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub width: u32,
    pub depth: u32,
    pub fill: FP,
}

impl TerrainPanel {
    pub fn apply_grid_expand(&mut self, editor: &Editor) -> Result<(), String> {
        let result = (|| {
            self.require_attached(editor)?;
            let terrain = self
                .document
                .terrain()
                .ok_or("Create or open terrain first")?;
            let request = self.grid_expand.request(terrain)?;
            self.document
                .expand_grid(request.width, request.depth, request.fill)?;
            self.refresh_grid_dimensions();
            self.refresh_selection();
            Ok(())
        })();
        self.report(result)
    }

    pub(super) fn show_grid_expand(&mut self, ui: &mut egui::Ui, editor: &Editor, writable: bool) {
        ui.collapsing("Expand terrain grid", |ui| {
            let permitted = writable && self.require_attached(editor).is_ok();
            let Some(terrain) = self.document.terrain() else {
                return;
            };
            if self.grid_expand.show_controls(ui, terrain, permitted) {
                let _ = self.apply_grid_expand(editor);
            }
        });
    }
}

#[cfg(test)]
#[path = "grid_expand_tests.rs"]
mod tests;

#[cfg(all(test, target_os = "linux"))]
#[path = "grid_expand_integration_tests.rs"]
mod integration_tests;
