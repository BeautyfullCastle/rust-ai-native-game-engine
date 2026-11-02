//! Draft-only static-prop scalar controls; caller owns Apply/history/Save.
use crate::mesh_prop_shading::{Settings, Slot};
use orr_model::StaticModel;
use std::sync::Arc;

struct Fields {
    material: u32,
    enabled: bool,
    metallic: String,
    roughness: String,
}
#[derive(Default)]
pub(crate) struct Panel {
    model: Option<Arc<StaticModel>>,
    fields: Vec<Fields>,
    selected: usize,
}
impl Panel {
    pub(crate) fn load(&mut self, model: &Arc<StaticModel>, settings: Option<&Settings>) {
        let used: std::collections::BTreeSet<_> = model
            .source()
            .primitives
            .iter()
            .map(|p| p.material)
            .collect();
        self.fields = used
            .into_iter()
            .map(|material| {
                let slot = settings.and_then(|s| s.slots().iter().find(|s| s.material == material));
                Fields {
                    material,
                    enabled: slot.is_some(),
                    metallic: slot.map_or(0.0, |s| s.metallic).to_string(),
                    roughness: slot.map_or(0.5, |s| s.roughness).to_string(),
                }
            })
            .collect();
        self.selected = 0;
        self.model = Some(Arc::clone(model));
    }
    pub(crate) fn settings(&self, model: &Arc<StaticModel>) -> Result<Option<Settings>, String> {
        if !self.model.as_ref().is_some_and(|m| Arc::ptr_eq(m, model)) {
            return Err("The prop shading model changed; reload the selected prop".into());
        }
        let mut slots = Vec::new();
        for fields in &self.fields {
            if fields.enabled {
                let parse = |text: &str, name| -> Result<f32, String> {
                    let value = text.parse::<f32>().map_err(|_| {
                        format!(
                            "Prop slot {} {name} must be a number in 0..=1",
                            fields.material
                        )
                    })?;
                    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                        return Err(format!(
                            "Prop slot {} {name} must be finite and in 0..=1",
                            fields.material
                        ));
                    }
                    Ok(value)
                };
                slots.push(Slot {
                    material: fields.material,
                    metallic: parse(&fields.metallic, "metallic")?,
                    roughness: parse(&fields.roughness, "roughness")?,
                });
            }
        }
        if slots.is_empty() {
            Ok(None)
        } else {
            Settings::new(slots, model).map(Some)
        }
    }
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, model: &Arc<StaticModel>) {
        if !self.model.as_ref().is_some_and(|m| Arc::ptr_eq(m, model)) {
            ui.label("Reload the prop to edit scalar shading");
            return;
        }
        if self.fields.is_empty() {
            ui.label("No used material slots");
            return;
        }
        egui::ComboBox::from_label("Prop shading material slot")
            .selected_text(self.fields[self.selected].material.to_string())
            .show_ui(ui, |ui| {
                for (index, field) in self.fields.iter().enumerate() {
                    ui.selectable_value(&mut self.selected, index, field.material.to_string());
                }
            });
        let fields = &mut self.fields[self.selected];
        ui.checkbox(&mut fields.enabled, "Use scalar shading for prop slot");
        ui.add_enabled_ui(fields.enabled, |ui| {
            ui.horizontal(|ui| {
                let label = ui.label("Prop slot metallic");
                ui.text_edit_singleline(&mut fields.metallic)
                    .labelled_by(label.id);
            });
            ui.horizontal(|ui| {
                let label = ui.label("Prop slot roughness");
                ui.text_edit_singleline(&mut fields.roughness)
                    .labelled_by(label.id);
            });
        });
        if let Err(error) = self.settings(model) {
            ui.colored_label(egui::Color32::RED, error);
        }
        ui.weak("Draft only: Update selected prop or Add prop, then Save. Shaded export and prefab capture are not supported yet.");
    }
}
