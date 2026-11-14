//! Explicit persistence of a clipboard snapshot, never automatic map application.
use super::*;

impl TilemapPanel {
    fn pattern_file_context(&self, editor: &Editor) -> Result<String, String> {
        if !self.editable(editor) || self.painting || self.rectangle.is_some() {
            return Err("Pattern files require the matching idle Edit viewport".into());
        }
        self.check_stamp()?;
        let project = orr_package::Project::open(
            &self.stamp.root,
            crate::sprite_bindings::compiled_runtime(),
        )
        .map_err(|e| e.to_string())?;
        project.verify().map_err(|e| e.to_string())?;
        let lock = project.list().map_err(|e| e.to_string())?;
        let digest = lock
            .packages
            .get(&self.key().0)
            .ok_or("tilemap package is missing")?
            .digest
            .clone();
        self.check_stamp()?;
        Ok(digest)
    }

    fn save_pattern(
        &self,
        editor: &Editor,
        sprites: &SpritePanel,
    ) -> Result<crate::json_output::Published, String> {
        let digest = self.pattern_file_context(editor)?;
        let (asset, _) = sprites
            .tilemap_asset(&self.key())
            .ok_or("shared atlas missing")?;
        let pattern = self
            .pattern
            .as_ref()
            .ok_or("Copy or Load a pattern first")?;
        let bytes = pattern.encode_asset(
            &self.history.current,
            editor.scene_load_epoch(),
            &asset.document,
            &digest,
        )?;
        crate::json_output::publish_new(Path::new(&self.pattern_file), &bytes, || {
            if self.pattern_file_context(editor)? != digest {
                return Err("tilemap package changed before pattern publication".into());
            }
            Ok(())
        })
    }

    fn load_pattern(&mut self, editor: &Editor, sprites: &SpritePanel) -> Result<(), String> {
        let digest = self.pattern_file_context(editor)?;
        let (asset, texture) = sprites
            .tilemap_asset(&self.key())
            .ok_or("shared atlas missing")?;
        let bytes = pattern_asset::read_pattern(Path::new(&self.pattern_file))?;
        let pattern = copy::Pattern::decode_asset(
            &bytes,
            &self.history.current,
            editor.scene_load_epoch(),
            &asset.document,
            &digest,
        )?;
        if self.pattern_file_context(editor)? != digest {
            return Err("tilemap package changed while loading pattern".into());
        }
        // No map, history, disk or mode change. Every refusal above preserves the
        // existing clipboard, including its texture/context pins.
        self.pattern = Some(pattern);
        self.pattern_texture = Some(texture);
        Ok(())
    }

    pub(super) fn show_pattern_files(
        &mut self,
        ui: &mut egui::Ui,
        editor: &Editor,
        sprites: &SpritePanel,
    ) {
        crate::theme::collapsing(ui, "Reusable pattern files", |ui| {
            let label = ui.label("Pattern file");
            ui.text_edit_singleline(&mut self.pattern_file).labelled_by(label.id);
            ui.label("Absolute .json path. Save creates a new file; Load only replaces the copied pattern. Matching installed artwork is required.");
            ui.add_enabled_ui(!self.painting && self.rectangle.is_none(), |ui| {
                ui.horizontal(|ui| {
                    if ui.add_enabled(self.pattern.is_some(), egui::Button::new("Save pattern")).clicked() {
                        self.pattern_notice = None;
                        match self.save_pattern(editor, sprites) {
                            Ok(report) => {
                                self.error = None;
                                self.pattern_notice = Some(match report.warning {
                                    Some(warning) => format!("Pattern exists or may exist; do not retry: {warning}"),
                                    None => format!("Saved pattern ({} bytes). Tilemap was not saved or changed.", report.bytes),
                                });
                            }
                            Err(error) => self.error = Some(error),
                        }
                    }
                    if ui.button("Load pattern").clicked() {
                        self.pattern_notice = None;
                        match self.load_pattern(editor, sprites) {
                            Ok(()) => {
                                self.error = None;
                                self.pattern_notice = Some("Pattern loaded. Choose Stamp to apply it.".into());
                            }
                            Err(error) => self.error = Some(error),
                        }
                    }
                });
            });
            if let Some(notice) = &self.pattern_notice { ui.label(notice); }
        });
    }
}

#[cfg(all(test, feature = "project-create"))]
#[path = "pattern_files_tests.rs"]
mod tests;
