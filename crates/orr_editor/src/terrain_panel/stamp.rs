//! One-shot additive diamond relief. Integer falloff; no clipping or continuous brush.
use super::{
    index_field, parse_fixed, text_field, Editor, TerrainPanel, TerrainSelectionMode, TerrainSource,
};
use orr_fp::FP;
use orr_terrain::{Edit, Terrain, MAX_EDITS};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fields {
    pub radius: u32,
    pub delta: String,
}
impl Default for Fields {
    fn default() -> Self {
        Self {
            radius: 2,
            delta: "1".into(),
        }
    }
}
struct Preview {
    fields: Fields,
    center: [u32; 2],
    terrain: Vec<u8>,
    source: TerrainSource,
    scene: PathBuf,
    epoch: u64,
    edits: Vec<Edit>,
}
#[derive(Default)]
pub struct State {
    pub fields: Fields,
    candidate: Option<Preview>,
}
impl State {
    pub fn has_preview(&self) -> bool {
        self.candidate.is_some()
    }
}

/// Budget includes the zero-weight rim and no-ops. Any incident hole rejects
/// the entire stamp; holes outside that exact footprint do not matter.
pub fn edits(
    terrain: &Terrain,
    center: [u32; 2],
    radius: u32,
    delta: FP,
) -> Result<Vec<Edit>, String> {
    let r = u128::from(radius);
    let count = 1 + 2 * r * (r + 1);
    if radius == 0 || count > MAX_EDITS as u128 {
        return Err(format!(
            "Stamp radius must be positive and its full diamond fit {MAX_EDITS} vertices"
        ));
    }
    let [cx, cz] = center;
    let xmax = cx.checked_add(radius).ok_or("Stamp extent overflow")?;
    let zmax = cz.checked_add(radius).ok_or("Stamp extent overflow")?;
    if cx < radius || cz < radius || xmax >= terrain.width() || zmax >= terrain.depth() {
        return Err("Stamp diamond must be entirely inside terrain vertices; no clipping".into());
    }
    let mut result = Vec::with_capacity(count as usize);
    for z in cz - radius..=zmax {
        let reach = radius - z.abs_diff(cz);
        for x in cx - reach..=cx + reach {
            // Existing cells touching this vertex, including zero-weight rim.
            for cell_z in z.saturating_sub(1)..=z.min(terrain.depth() - 2) {
                for cell_x in x.saturating_sub(1)..=x.min(terrain.width() - 2) {
                    if terrain.holes()[(cell_z * (terrain.width() - 1) + cell_x) as usize] {
                        return Err("Stamp diamond touches a hole; nothing changed".into());
                    }
                }
            }
            let distance = x.abs_diff(cx) + z.abs_diff(cz);
            let change =
                i128::from(delta.raw()) * i128::from(radius - distance) / i128::from(radius);
            let old = terrain.heights()[(z * terrain.width() + x) as usize];
            let raw = i64::try_from(i128::from(old.raw()) + change)
                .map_err(|_| "Stamp height overflow; nothing changed")?;
            let height = FP::from_raw(raw);
            if height != old {
                result.push(Edit::SetHeight { x, z, height });
            }
        }
    }
    Ok(result)
}

impl TerrainPanel {
    fn stamp_context(&self, editor: &Editor) -> Result<(), String> {
        self.require_attached(editor)?;
        if self.document.read_only() {
            return Err("Copy installed terrain to the scene before stamping".into());
        }
        if self.selection_mode != TerrainSelectionMode::Vertex {
            return Err("Select a terrain vertex before stamping".into());
        }
        Ok(())
    }
    fn stamp_matches(&self, editor: &Editor, preview: &Preview) -> Result<(), String> {
        self.stamp_context(editor)?;
        if self.stamp.fields != preview.fields
            || self.selected_vertex != preview.center
            || self.document.bytes() != Some(preview.terrain.as_slice())
            || self.document.source() != Some(&preview.source)
            || self.document.scene() != Some(preview.scene.as_path())
            || editor.scene_load_epoch() != preview.epoch
        {
            return Err("Terrain stamp preview is stale; preview again".into());
        }
        Ok(())
    }
    pub fn preview_stamp(&mut self, editor: &Editor) -> Result<(), String> {
        self.stamp.candidate = None;
        let result = (|| {
            self.stamp_context(editor)?;
            if self.stamp.fields.delta.len() > 128 {
                return Err("Stamp height delta exceeds 128 UTF-8 bytes".into());
            }
            let delta = parse_fixed(&self.stamp.fields.delta)?;
            let changes = edits(
                self.document.terrain().ok_or("Open terrain first")?,
                self.selected_vertex,
                self.stamp.fields.radius,
                delta,
            )?;
            self.stamp.candidate = Some(Preview {
                fields: self.stamp.fields.clone(),
                center: self.selected_vertex,
                terrain: self.document.bytes().ok_or("Open terrain first")?.to_vec(),
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
                edits: changes,
            });
            Ok(())
        })();
        self.report(result)
    }
    pub fn apply_stamp(&mut self, editor: &Editor) -> Result<(), String> {
        let candidate = self.stamp.candidate.take();
        let result = (|| {
            let candidate = candidate.ok_or("Preview terrain stamp before applying")?;
            self.stamp_matches(editor, &candidate)?;
            self.document.apply(&candidate.edits)?;
            self.refresh_selection();
            Ok(())
        })();
        self.report(result)
    }
    pub(super) fn show_stamp(&mut self, ui: &mut egui::Ui, editor: &Editor, writable: bool) {
        // Run even while collapsed: selection/context changes revoke the old preview.
        if self
            .stamp
            .candidate
            .as_ref()
            .is_some_and(|p| self.stamp_matches(editor, p).is_err())
        {
            self.stamp.candidate = None;
        }
        ui.collapsing("Stamp terrain relief", |ui| {
            ui.weak("Add a diamond hill or depression around the selected vertex. No clipping; touching holes is refused.");
            ui.label(format!("Center vertex: [{}, {}]. Full diamond limit: {MAX_EDITS} vertices (radius 1..=44), including zero-weight rim.", self.selected_vertex[0], self.selected_vertex[1]));
            let before = self.stamp.fields.clone();
            index_field(ui, "Stamp radius", &mut self.stamp.fields.radius);
            text_field(ui, "Stamp height delta", &mut self.stamp.fields.delta);
            if before != self.stamp.fields { self.stamp.candidate = None; }
            let enabled = writable && self.selection_mode == TerrainSelectionMode::Vertex;
            if ui.add_enabled(enabled, egui::Button::new("Preview terrain stamp")).clicked() { let _ = self.preview_stamp(editor); }
            if let Some(p) = &self.stamp.candidate { ui.label(format!("{} heights change; one Undo. Save is separate.", p.edits.len())); }
            if ui.add_enabled(enabled && self.stamp.has_preview(), egui::Button::new("Apply terrain stamp")).clicked() { let _ = self.apply_stamp(editor); }
        });
    }
}

#[cfg(test)]
mod tests;
