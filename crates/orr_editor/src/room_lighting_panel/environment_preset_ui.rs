//! Explicit portable environment capture/load; lighting Apply and Save stay separate.
use super::environment_preset::{self, Preset};
use super::*;
impl Panel {
    pub(super) fn show_environment_presets(&mut self, ui: &mut egui::Ui, editable: bool) {
        crate::theme::collapsing(ui, "Room environment presets", |ui| {
            ui.weak("Use an absolute .room-environment-preset.json path. Save creates a new preset from applied environment settings. Load stages only the environment; Apply lighting and Save lighting remain explicit.");
            let id = ui.make_persistent_id(("room_environment_preset_path", &self.path));
            let mut path = ui.ctx().data_mut(|data| data.get_temp::<String>(id).unwrap_or_default());
            let label = ui.label("Room environment preset file");
            ui.text_edit_singleline(&mut path).labelled_by(label.id);
            ui.ctx().data_mut(|data| data.insert_temp(id, path.clone()));
            let ready = editable && ui.is_enabled() && ui.ctx().input(|input| input.focused && !input.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false))));
            if ui.add_enabled(ready && self.document.environment.is_some(), egui::Button::new("Save applied environment preset")).clicked() && ready && self.document.environment.is_some() {
                let result = self.recheck_disk().and_then(|()| Preset::from_document(&self.document)).and_then(|preset| preset.encode()).and_then(|bytes| environment_preset::save_new(Path::new(&path), &bytes));
                self.error = result.err().map(|error| format!("Save environment preset failed: {error}"));
            }
            if ui.add_enabled(ready, egui::Button::new("Load environment preset")).clicked() && ready {
                let result = (|| {
                    self.recheck_disk()?;
                    let preset = environment_preset::load(Path::new(&path))?;
                    let merged = preset.merge_into(&self.candidate)?;
                    self.recheck_disk()?;
                    Ok::<_,String>(merged)
                })();
                match result {
                    Ok(candidate) => {self.cancel_active_pick(); self.candidate = candidate; self.error = None;}
                    Err(error) => self.error = Some(format!("Load environment preset failed: {error}")),
                }
            }
        });
    }
}
#[cfg(all(test, target_os = "linux"))]
#[path = "environment_preset_ui_tests.rs"]
mod tests;
