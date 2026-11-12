//! Explicit persistence, separate from terrain Save and Paste.
use super::*;
use std::path::Path;

impl TerrainPanel {
    pub(super) fn bound_patch(&self, editor: &Editor) -> Result<&Clipboard, String> {
        self.patch_context(editor)?;
        let bound = self
            .patch
            .clipboard
            .as_ref()
            .ok_or("Copy or Load a terrain patch first")?;
        if self.document.source() != Some(&bound.source)
            || self.document.scene() != Some(bound.scene.as_path())
            || editor.scene_load_epoch() != bound.epoch
            || self.document.terrain().map(Geometry::of).as_ref() != Some(&bound.patch.geometry)
        {
            return Err("Terrain patch context changed; copy or load again".into());
        }
        Ok(&bound.patch)
    }
    fn save_patch_asset(&self, editor: &Editor) -> Result<crate::json_output::Published, String> {
        let bytes = self.bound_patch(editor)?.encode_asset()?;
        crate::json_output::publish_new(Path::new(&self.patch.asset_path), &bytes, || {
            self.bound_patch(editor)?;
            Ok(())
        })
    }
    fn load_patch_asset(&mut self, editor: &Editor) -> Result<(), String> {
        self.patch_context(editor)?;
        let bytes = asset::read_patch(Path::new(&self.patch.asset_path))?;
        let patch =
            Clipboard::decode_asset(&bytes, self.document.terrain().ok_or("Open terrain first")?)?;
        let bound = BoundClipboard {
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
        };
        self.patch_context(editor)?;
        self.patch.clipboard = Some(bound);
        Ok(())
    }
    pub(super) fn show_patch_files(&mut self, ui: &mut egui::Ui, editor: &Editor, writable: bool) {
        text_field(ui, "Terrain patch file", &mut self.patch.asset_path);
        ui.weak("Absolute .json path in trusted, idle directories. Save creates a new file. Load only replaces the clipboard; matching spacing is required.");
        if ui
            .add_enabled(
                writable && self.bound_patch(editor).is_ok(),
                egui::Button::new("Save terrain patch asset"),
            )
            .clicked()
        {
            self.patch.notice = None;
            match self.save_patch_asset(editor) {
                Ok(report) => {
                    self.error = None;
                    self.patch.notice = Some(match report.warning {
                        Some(warning) => {
                            format!("Patch exists or may exist; do not retry: {warning}")
                        }
                        None => format!(
                            "Saved patch ({} bytes). Terrain is unchanged.",
                            report.bytes
                        ),
                    });
                }
                Err(error) => self.error = Some(error),
            }
        }
        if ui
            .add_enabled(writable, egui::Button::new("Load terrain patch asset"))
            .clicked()
        {
            self.patch.notice = None;
            match self.load_patch_asset(editor) {
                Ok(()) => {
                    self.error = None;
                    self.patch.notice =
                        Some("Patch loaded. Choose Paste to change terrain.".into());
                }
                Err(error) => self.error = Some(error),
            }
        }
        if let Some(notice) = &self.patch.notice {
            ui.label(notice);
        }
    }
}

#[cfg(all(test, unix))]
mod tests;
