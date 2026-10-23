use super::{Document, Editor, Panel, Path, Ui, format_number, text_field};
use crate::mesh_lighting_preset::{Preset, store};

#[derive(Default)]
pub(super) struct Controls {
    path: String,
}

impl Panel {
    pub(super) fn show_preset_controls(&mut self, ui: &mut Ui, editor: &Editor) {
        ui.separator();
        ui.label("Portable lighting presets");
        ui.weak("Use an absolute .mesh-lighting-preset.json path. Save creates a new file; Load stages reusable lighting and preserves this scene's shadow bounds.");
        text_field(ui, "Lighting preset file", &mut self.presets.path);
        let ready = self.can_edit(editor) && !self.form_dirty && self.current_document().is_some();
        if ui
            .add_enabled(ready, egui::Button::new("Save applied lighting preset"))
            .clicked()
        {
            self.save_applied_preset(editor);
        }
        if ui
            .add_enabled(ready, egui::Button::new("Load lighting preset"))
            .clicked()
        {
            self.load_preset(editor);
        }
        ui.weak("Apply lighting first if this scene has no applied lighting. Loading never applies or saves the scene; use Apply game lighting, then Save mesh workspace.");
    }

    fn preset_target(&self, editor: &Editor) -> Result<Document, String> {
        if !self.can_edit(editor) {
            return Err(
                "Lighting presets require the matching editable local mesh scene and graph".into(),
            );
        }
        if self.form_dirty {
            return Err("Apply or discard the current lighting draft before using a preset".into());
        }
        let document = self
            .current_document()
            .ok_or("Apply game lighting before using a lighting preset")?;
        document.lighting()?;
        let scene = self.scene.as_deref().ok_or("Mesh scene is not ready")?;
        if crate::mesh_lighting_store::sidecar_stamp(scene)? != self.sidecar_sha {
            return Err("Saved game lighting changed externally; resolve or reload it before using a preset".into());
        }
        Ok(document.clone())
    }

    fn save_applied_preset(&mut self, editor: &Editor) {
        let result = (|| {
            let document = self.preset_target(editor)?;
            let bytes = Preset::from_document(&document)?.encode()?;
            store::save_new(Path::new(&self.presets.path), &bytes)
        })();
        match result {
            Ok(()) => {
                self.error = None;
                self.status = "Saved applied lighting to a new portable preset; scene lighting and workspace save state are unchanged.".into();
            }
            Err(error) => self.error = Some(format!("Save lighting preset failed: {error}")),
        }
    }

    fn load_preset(&mut self, editor: &Editor) {
        let result = (|| {
            let target = self.preset_target(editor)?;
            let preset = store::load(Path::new(&self.presets.path))?;
            let merged = preset.merge_into(&target)?;
            // Recheck admission after the read, before changing any draft field.
            if self.preset_target(editor)? != target {
                return Err("Applied game lighting changed while loading the preset".into());
            }
            merged.lighting()
        })();
        match result {
            Ok(lighting) => {
                // Shadow center/radius belong to the target scene. Do not even
                // reformat their strings: loading changes reusable fields only.
                self.direction = lighting.direction.map(format_number);
                self.color = lighting.color.map(format_number);
                self.intensity = format_number(lighting.intensity);
                self.sky = lighting.sky.map(format_number);
                self.ground = lighting.ground.map(format_number);
                self.ambient = format_number(lighting.ambient);
                self.shadows = lighting.shadows;
                self.tonemap = lighting.tonemap;
                self.exposure = format_number(lighting.exposure);
                self.form_dirty = true;
                self.error = None;
                self.status = "Lighting preset staged; this scene's shadow bounds are preserved. Apply game lighting to preview, then save explicitly.".into();
            }
            Err(error) => self.error = Some(format!("Load lighting preset failed: {error}")),
        }
    }
}

#[cfg(all(test, feature = "mesh-mission"))]
#[path = "presets_tests.rs"]
mod tests;
