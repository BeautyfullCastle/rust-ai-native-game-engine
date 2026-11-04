//! Reusable, immutable terrain patches. Copy uses inclusive cell coordinates and
//! captures enclosing vertices too. Invalid Copy clears the previous clipboard.
use super::{index_field, parse_fixed, text_field, Editor, TerrainPanel, TerrainSource};
use orr_fp::FP;
use orr_terrain::{Edit, Terrain, MAX_EDITS};
use std::path::PathBuf;

mod asset;
mod files;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Geometry {
    asset_id: String,
    dimensions: [u32; 2],
    origin: [FP; 2],
    spacing: FP,
}
impl Geometry {
    fn of(terrain: &Terrain) -> Self {
        Self {
            asset_id: terrain.asset_id().to_owned(),
            dimensions: [terrain.width(), terrain.depth()],
            origin: terrain.origin(),
            spacing: terrain.spacing(),
        }
    }
}

/// An immutable snapshot, independent of subsequent terrain edits and Undo.
#[derive(Clone, Debug)]
pub struct Clipboard {
    geometry: Geometry,
    cells: [u32; 2],
    heights: Vec<FP>,
    holes: Vec<bool>,
}
impl Clipboard {
    pub fn capture(terrain: &Terrain, min: [u32; 2], max: [u32; 2]) -> Result<Self, String> {
        let dimensions = [terrain.width(), terrain.depth()];
        let mut cells = [0; 2];
        for axis in 0..2 {
            if min[axis] > max[axis] || max[axis] >= dimensions[axis] - 1 {
                return Err(
                    "Patch source must be ordered and entirely inside terrain cells".into(),
                );
            }
            cells[axis] = max[axis] - min[axis] + 1;
        }
        let count = footprint(cells)?;
        let mut heights = Vec::with_capacity(count);
        let mut holes = Vec::new();
        for z in min[1]..=max[1] + 1 {
            for x in min[0]..=max[0] + 1 {
                heights.push(terrain.heights()[(z * terrain.width() + x) as usize]);
            }
        }
        for z in min[1]..=max[1] {
            for x in min[0]..=max[0] {
                holes.push(terrain.holes()[(z * (terrain.width() - 1) + x) as usize]);
            }
        }
        Ok(Self {
            geometry: Geometry::of(terrain),
            cells,
            heights,
            holes,
        })
    }

    /// Count all enclosing vertices and cells, including unchanged values.
    pub fn footprint(&self) -> usize {
        self.heights.len() + self.holes.len()
    }

    /// Plan a complete paste or fail without mutation. No clipping is performed.
    pub fn edits(
        &self,
        terrain: &Terrain,
        destination: [u32; 2],
        offset: FP,
    ) -> Result<Vec<Edit>, String> {
        if Geometry::of(terrain) != self.geometry {
            return Err("Terrain patch geometry changed; copy again".into());
        }
        footprint(self.cells)?;
        for (axis, start) in destination.iter().enumerate() {
            let end = start
                .checked_add(self.cells[axis])
                .ok_or("Patch destination extent overflow")?;
            if end >= self.geometry.dimensions[axis] {
                return Err(
                    "Patch destination must be entirely inside terrain; no clipping".into(),
                );
            }
        }
        let mut edits = Vec::with_capacity(self.footprint());
        let width = self.cells[0] + 1;
        for z in 0..=self.cells[1] {
            for x in 0..=self.cells[0] {
                let raw = self.heights[(z * width + x) as usize]
                    .raw()
                    .checked_add(offset.raw())
                    .ok_or("Patch height offset overflow; nothing changed")?;
                let height = FP::from_raw(raw);
                let x = destination[0] + x;
                let z = destination[1] + z;
                if terrain.heights()[(z * terrain.width() + x) as usize] != height {
                    edits.push(Edit::SetHeight { x, z, height });
                }
            }
        }
        for z in 0..self.cells[1] {
            for x in 0..self.cells[0] {
                let hole = self.holes[(z * self.cells[0] + x) as usize];
                let x = destination[0] + x;
                let z = destination[1] + z;
                if terrain.holes()[(z * (terrain.width() - 1) + x) as usize] != hole {
                    edits.push(Edit::SetHole { x, z, hole });
                }
            }
        }
        Ok(edits)
    }
}

fn footprint(cells: [u32; 2]) -> Result<usize, String> {
    let [x, z] = cells.map(u128::from);
    let count = x * z + (x + 1) * (z + 1);
    if count > MAX_EDITS as u128 {
        return Err(format!(
            "Patch full footprint exceeds {MAX_EDITS} vertices and cells; nothing changed"
        ));
    }
    Ok(count as usize)
}

struct BoundClipboard {
    patch: Clipboard,
    source: TerrainSource,
    scene: PathBuf,
    epoch: u64,
}

pub struct State {
    pub source_min: [u32; 2],
    pub source_max: [u32; 2],
    pub destination: [u32; 2],
    pub height_offset: String,
    clipboard: Option<BoundClipboard>,
    pub asset_path: String,
    notice: Option<String>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            source_min: [0; 2],
            source_max: [0; 2],
            destination: [0; 2],
            height_offset: "0".into(),
            clipboard: None,
            asset_path: String::new(),
            notice: None,
        }
    }
}
impl State {
    pub fn has_clipboard(&self) -> bool {
        self.clipboard.is_some()
    }
}

impl TerrainPanel {
    fn patch_context(&self, editor: &Editor) -> Result<(), String> {
        self.require_attached(editor)?;
        if self.document.read_only() {
            return Err(
                "Copy installed terrain to the scene before copying or pasting patches".into(),
            );
        }
        Ok(())
    }

    fn patch_edits(&self, editor: &Editor) -> Result<Vec<Edit>, String> {
        self.patch_context(editor)?;
        let clipboard = self
            .patch
            .clipboard
            .as_ref()
            .ok_or("Copy a terrain patch first")?;
        if self.document.source() != Some(&clipboard.source)
            || self.document.scene() != Some(clipboard.scene.as_path())
            || editor.scene_load_epoch() != clipboard.epoch
        {
            return Err("Terrain patch context changed; copy again".into());
        }
        if self.patch.height_offset.len() > 128 {
            return Err("Patch height offset exceeds 128 UTF-8 bytes".into());
        }
        clipboard.patch.edits(
            self.document.terrain().ok_or("Open terrain first")?,
            self.patch.destination,
            parse_fixed(&self.patch.height_offset)?,
        )
    }

    /// Copy changes neither the document nor history. Failure clears old content.
    pub fn copy_terrain_patch(&mut self, editor: &Editor) -> Result<(), String> {
        self.patch.clipboard = None;
        let result = (|| {
            self.patch_context(editor)?;
            let patch = Clipboard::capture(
                self.document.terrain().ok_or("Open terrain first")?,
                self.patch.source_min,
                self.patch.source_max,
            )?;
            self.patch.clipboard = Some(BoundClipboard {
                patch,
                source: self
                    .document
                    .source()
                    .ok_or("Missing terrain source")?
                    .clone(),
                scene: self
                    .document
                    .scene()
                    .ok_or("Missing terrain scene")?
                    .to_owned(),
                epoch: editor.scene_load_epoch(),
            });
            Ok(())
        })();
        self.report(result)
    }

    /// One transaction; success retains the snapshot for repeated/overlapping use.
    pub fn paste_terrain_patch(&mut self, editor: &Editor) -> Result<(), String> {
        let result = (|| {
            let edits = self.patch_edits(editor)?;
            self.document.apply(&edits)?;
            self.refresh_selection();
            Ok(())
        })();
        self.report(result)
    }

    pub(super) fn show_patch(&mut self, ui: &mut egui::Ui, editor: &Editor, writable: bool) {
        ui.collapsing("Terrain patch clipboard", |ui| {
            ui.weak("Copy inclusive cells and their enclosing heights. Paste replaces heights and holes; no clipping. Save is separate. Invalid Copy clears the clipboard.");
            index_field(ui, "Patch source minimum X", &mut self.patch.source_min[0]);
            index_field(ui, "Patch source minimum Z", &mut self.patch.source_min[1]);
            index_field(ui, "Patch source maximum X", &mut self.patch.source_max[0]);
            index_field(ui, "Patch source maximum Z", &mut self.patch.source_max[1]);
            index_field(ui, "Patch destination X", &mut self.patch.destination[0]);
            index_field(ui, "Patch destination Z", &mut self.patch.destination[1]);
            text_field(ui, "Patch height offset", &mut self.patch.height_offset);
            ui.label(format!("Full footprint limit: {MAX_EDITS} vertices and cells, including no-ops."));
            if ui.add_enabled(writable, egui::Button::new("Copy terrain patch")).clicked() {
                let _ = self.copy_terrain_patch(editor);
            }
            if let Some(clipboard) = &self.patch.clipboard {
                ui.label(format!("Copied footprint: {} vertices and cells.", clipboard.patch.footprint()));
            }
            self.show_patch_files(ui, editor, writable);
            let preview = self.patch_edits(editor);
            match &preview {
                Ok(edits) if edits.is_empty() => { ui.label("No values will change; history is preserved."); }
                Ok(edits) => { ui.label(format!("{} values will change; one Undo step.", edits.len())); }
                Err(error) => { ui.colored_label(egui::Color32::YELLOW, error); }
            }
            if ui.add_enabled(writable && preview.is_ok(), egui::Button::new("Paste terrain patch")).clicked() {
                let _ = self.paste_terrain_patch(editor);
            }
        });
    }
}

#[cfg(test)]
mod tests;
