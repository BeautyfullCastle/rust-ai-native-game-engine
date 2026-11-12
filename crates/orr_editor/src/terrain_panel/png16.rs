//! Explicit PNG16 height replacement through the terrain document transaction.
use super::{text_field, Editor, TerrainPanel, TerrainSource};
use crate::terrain_document::format_fixed;
use orr_terrain::{Edit, MAX_EDITS};
use orr_terrain_view::{
    png16::{import_png16, input::read_png16},
    raw16::{Endian, Recipe},
};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fields {
    pub path: String,
    pub offset: String,
    pub scale: String,
    pub flip_z: bool,
}
impl Default for Fields {
    fn default() -> Self {
        Self {
            path: String::new(),
            offset: "0".into(),
            scale: "1".into(),
            flip_z: false,
        }
    }
}
struct Candidate {
    fields: Fields,
    recipe: Recipe,
    encoded: Vec<u8>,
    terrain: Vec<u8>,
    source: TerrainSource,
    scene: PathBuf,
    epoch: u64,
    edits: Vec<Edit>,
    range: [orr_fp::FP; 2],
}
#[derive(Default)]
pub struct State {
    pub fields: Fields,
    candidate: Option<Candidate>,
}
impl State {
    pub fn has_preview(&self) -> bool {
        self.candidate.is_some()
    }
}

impl TerrainPanel {
    fn png16_context(&self, editor: &Editor) -> Result<(), String> {
        self.require_attached(editor)?;
        if editor.in_gesture() || editor.history().in_tx || editor.sim().in_tx {
            return Err("Finish the active scene edit before replacing PNG16 heights".into());
        }
        if self.document.read_only() {
            return Err("Copy installed terrain to the scene before replacing heights".into());
        }
        let t = self.document.terrain().ok_or("Open terrain first")?;
        if t.heights().len() > MAX_EDITS {
            return Err(format!("PNG16 replacement footprint exceeds {MAX_EDITS} vertices; no splitting or resizing"));
        }
        Ok(())
    }
    fn png16_path(&self) -> Result<PathBuf, String> {
        let text = &self.png16.fields.path;
        if text.is_empty() || text.len() > 4096 || !Path::new(text).is_absolute() {
            return Err(
                "PNG16 input must be an explicit absolute path of at most 4096 UTF-8 bytes".into(),
            );
        }
        Ok(PathBuf::from(text))
    }
    fn png16_not_target(&self, input: &Path) -> Result<(), String> {
        let Some(TerrainSource::Local { relative_path }) = self.document.source() else {
            return Err("PNG16 needs writable scene terrain".into());
        };
        let target = self
            .document
            .scene()
            .and_then(Path::parent)
            .ok_or("Missing terrain scene")?
            .join(relative_path);
        if input == target {
            return Err("PNG16 source must not alias the current terrain file".into());
        }
        if let (Ok(a), Ok(b)) = (std::fs::metadata(input), std::fs::metadata(&target)) {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if a.dev() == b.dev() && a.ino() == b.ino() {
                    return Err("PNG16 source must not alias the current terrain file".into());
                }
            }
            #[cfg(not(unix))]
            {
                let _ = (a, b);
                if std::fs::canonicalize(input).ok() == std::fs::canonicalize(&target).ok() {
                    return Err("PNG16 source must not alias the current terrain file".into());
                }
            }
        }
        Ok(())
    }
    fn png16_recipe(&self) -> Result<Recipe, String> {
        let t = self.document.terrain().ok_or("Open terrain first")?;
        if self.png16.fields.offset.len() > 128 || self.png16.fields.scale.len() > 128 {
            return Err("PNG16 height decimals must be at most 128 UTF-8 bytes".into());
        }
        let offset = orr_fp::FP::parse(&self.png16.fields.offset)
            .map_err(|e| format!("invalid PNG16 offset: {e:?}"))?;
        let scale = orr_fp::FP::parse(&self.png16.fields.scale)
            .map_err(|e| format!("invalid PNG16 scale: {e:?}"))?;
        Ok(Recipe {
            asset_id: t.asset_id().to_owned(),
            width: t.width(),
            depth: t.depth(),
            origin: t.origin(),
            spacing: t.spacing(),
            // PNG sample byte order is fixed, independent of RAW16 settings.
            endian: Endian::Big,
            height_offset: offset,
            height_scale: scale,
            flip_z: self.png16.fields.flip_z,
        })
    }
    pub fn preview_png16(&mut self, editor: &Editor) -> Result<(), String> {
        // A failed replacement preview never leaves a previous candidate usable.
        self.png16.candidate = None;
        let result = (|| {
            self.png16_context(editor)?;
            let path = self.png16_path()?;
            self.png16_not_target(&path)?;
            let recipe = self.png16_recipe()?;
            let encoded = read_png16(&path).map_err(|e| e.to_string())?;
            let imported = import_png16(&encoded, &recipe).map_err(|e| e.to_string())?;
            let current = self.document.terrain().ok_or("Open terrain first")?;
            let mut edits = Vec::new();
            for (index, (&before, &height)) in
                current.heights().iter().zip(imported.heights()).enumerate()
            {
                if before != height {
                    edits.push(Edit::SetHeight {
                        x: index as u32 % current.width(),
                        z: index as u32 / current.width(),
                        height,
                    });
                }
            }
            let range = [
                *imported.heights().iter().min().ok_or("Empty heightmap")?,
                *imported.heights().iter().max().ok_or("Empty heightmap")?,
            ];
            self.png16_context(editor)?;
            self.png16.candidate = Some(Candidate {
                fields: self.png16.fields.clone(),
                recipe,
                encoded,
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
                edits,
                range,
            });
            Ok(())
        })();
        self.report(result)
    }
    fn png16_matches(&self, editor: &Editor, candidate: &Candidate) -> Result<(), String> {
        self.png16_context(editor)?;
        if self.png16.fields != candidate.fields
            || self.png16_recipe()? != candidate.recipe
            || self.document.bytes() != Some(candidate.terrain.as_slice())
            || self.document.source() != Some(&candidate.source)
            || self.document.scene() != Some(candidate.scene.as_path())
            || editor.scene_load_epoch() != candidate.epoch
        {
            return Err(
                "PNG16 preview is stale; preview the current source and terrain again".into(),
            );
        }
        Ok(())
    }
    pub fn apply_png16(&mut self, editor: &Editor) -> Result<(), String> {
        // Every attempt consumes this preview; retries require an explicit new read.
        let candidate = self.png16.candidate.take();
        let result = (|| {
            let candidate = candidate.ok_or("Preview PNG16 heights before applying")?;
            self.png16_matches(editor, &candidate)?;
            let path = self.png16_path()?;
            self.png16_not_target(&path)?;
            let encoded = read_png16(&path).map_err(|e| e.to_string())?;
            if encoded != candidate.encoded {
                return Err("PNG16 source bytes changed after preview; nothing applied".into());
            }
            self.png16_matches(editor, &candidate)?;
            self.document.apply(&candidate.edits)?;
            self.refresh_selection();
            Ok(())
        })();
        self.report(result)
    }
    pub(super) fn show_png16(&mut self, ui: &mut egui::Ui, editor: &Editor, writable: bool) {
        ui.collapsing("PNG16 height replacement", |ui| {
            ui.weak("16-bit grayscale PNG, current dimensions only. Heights replace; holes and X/Z stay unchanged.");
            ui.weak("Stored sample values, no color correction or resizing. PNG byte order is fixed.");
            ui.weak("Trusted stable local input; explicit Preview then Apply. One Undo, Save is separate.");
            if let Some(t) = self.document.terrain() {
                ui.label(format!("{} × {} vertices · maximum {MAX_EDITS} vertices", t.width(), t.depth()));
            }
            let before = self.png16.fields.clone();
            text_field(ui, "PNG16 absolute input path", &mut self.png16.fields.path);
            text_field(ui, "PNG16 height offset", &mut self.png16.fields.offset);
            text_field(ui, "PNG16 full-range height scale", &mut self.png16.fields.scale);
            ui.checkbox(&mut self.png16.fields.flip_z, "PNG16 flip Z rows");
            if before != self.png16.fields {
                self.png16.candidate = None;
            }
            if ui.add_enabled(writable, egui::Button::new("Preview PNG16")).clicked() {
                let _ = self.preview_png16(editor);
            }
            if let Some(c) = &self.png16.candidate {
                ui.label(format!("{} heights change · range {} .. {} · current source snapshot only", c.edits.len(), format_fixed(c.range[0]), format_fixed(c.range[1])));
            }
            if ui.add_enabled(writable && self.png16.has_preview(), egui::Button::new("Apply PNG16 heights")).clicked() {
                let _ = self.apply_png16(editor);
            }
        });
    }
}
#[cfg(test)]
mod tests;
