//! A model-session-local value clipboard. No OS clipboard or asset mutation.
use super::{format_number, Panel, StaticModel, Ui};
use std::sync::Arc;

pub(super) struct Copied {
    settings: crate::mesh_material_slots::Settings,
}

impl Panel {
    fn copy_selected(&mut self, model: &Arc<StaticModel>) -> Result<(), String> {
        if !self.model_matches(model) {
            self.copied = None;
            return Err(
                "The admitted model changed; reload before copying material settings".into(),
            );
        }
        let field = self
            .fields
            .iter()
            .find(|field| Some(field.material) == self.selected)
            .ok_or("Select an available material slot before copying")?;
        let settings = field.settings().map_err(|error| error.to_string())?;
        self.copied = Some(Copied { settings });
        Ok(())
    }

    fn paste_selected(&mut self, model: &Arc<StaticModel>) -> Result<(), String> {
        if !self.model_matches(model) {
            self.copied = None;
            return Err(
                "The admitted model changed; reload before pasting material settings".into(),
            );
        }
        let settings = self
            .copied
            .as_ref()
            .ok_or("Copy valid material settings first")?
            .settings;
        let field = self
            .fields
            .iter_mut()
            .find(|field| Some(field.material) == self.selected)
            .ok_or("Select an available material slot before pasting")?;
        // The destination material ID and its source image are never copied.
        field.tint = settings.tint.map(format_number);
        field.linear_filter = settings.linear_filter;
        field.wrap_s = settings.wrap_s;
        field.wrap_t = settings.wrap_t;
        Ok(())
    }

    pub(super) fn show_clipboard(&mut self, ui: &mut Ui, model: &Arc<StaticModel>) {
        let selected = self
            .fields
            .iter()
            .find(|field| Some(field.material) == self.selected);
        let can_copy = selected.is_some_and(|field| field.settings().is_ok());
        let can_paste = selected.is_some() && self.copied.is_some();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(can_copy, egui::Button::new("Copy material slot settings"))
                .clicked()
            {
                let _ = self.copy_selected(model);
            }
            if ui
                .add_enabled(can_paste, egui::Button::new("Paste material slot settings"))
                .clicked()
            {
                let _ = self.paste_selected(model);
            }
        });
        if let Some(copied) = &self.copied {
            ui.weak(format!("Copied slot {} tint/filter/wrap. Paste edits fields only; Stage remains explicit. Textures are not copied.", copied.settings.material));
        } else {
            ui.weak("Copy/paste stays inside this model session; reload clears it.");
        }
    }
}

#[cfg(test)]
mod tests;
