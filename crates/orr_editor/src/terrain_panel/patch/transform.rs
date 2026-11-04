//! Exact grid permutations of both vertex heights and hole cells.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum Transform {
    RotateClockwise,
    MirrorX,
    MirrorZ,
}

fn permute<T: Copy>(values: &[T], width: u32, depth: u32, transform: Transform) -> Vec<T> {
    let mut result = values.to_vec();
    let output_width = match transform {
        Transform::RotateClockwise => depth,
        _ => width,
    };
    for z in 0..depth {
        for x in 0..width {
            let (out_x, out_z) = match transform {
                Transform::RotateClockwise => (depth - 1 - z, x),
                Transform::MirrorX => (width - 1 - x, z),
                Transform::MirrorZ => (x, depth - 1 - z),
            };
            result[(out_z * output_width + out_x) as usize] = values[(z * width + x) as usize];
        }
    }
    result
}

impl Clipboard {
    fn transformed(&self, transform: Transform) -> Self {
        let [x, z] = self.cells;
        Self {
            geometry: self.geometry.clone(),
            cells: match transform {
                Transform::RotateClockwise => [z, x],
                _ => self.cells,
            },
            heights: permute(&self.heights, x + 1, z + 1, transform),
            holes: permute(&self.holes, x, z, transform),
        }
    }
}

impl TerrainPanel {
    fn transform_terrain_patch(
        &mut self,
        editor: &Editor,
        transform: Transform,
    ) -> Result<(), String> {
        let result = (|| {
            let transformed = self.bound_patch(editor)?.transformed(transform);
            // No terrain edits or history entries. Existing pins remain unchanged.
            self.patch
                .clipboard
                .as_mut()
                .ok_or("Copy or Load a terrain patch first")?
                .patch = transformed;
            Ok(())
        })();
        self.report(result)
    }

    pub(super) fn show_patch_transforms(
        &mut self,
        ui: &mut egui::Ui,
        editor: &Editor,
        writable: bool,
    ) {
        ui.weak("Transforms change only the copied heights and holes. Clockwise maps +X toward +Z. Paste and Save are separate.");
        let enabled = writable && self.bound_patch(editor).is_ok();
        for (label, transform) in [
            ("Rotate terrain patch clockwise", Transform::RotateClockwise),
            ("Mirror terrain patch X", Transform::MirrorX),
            ("Mirror terrain patch Z", Transform::MirrorZ),
        ] {
            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                let _ = self.transform_terrain_patch(editor, transform);
            }
        }
        if let Some(bound) = &self.patch.clipboard {
            ui.label(format!(
                "Patch cells: {} X by {} Z",
                bound.patch.cells[0], bound.patch.cells[1]
            ));
        }
    }
}

#[cfg(test)]
mod tests;
