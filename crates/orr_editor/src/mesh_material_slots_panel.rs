use egui::Ui;
use orr_model::{StaticModel, Wrap};
use std::sync::Arc;

#[derive(Clone)]
struct SlotFields {
    material: u32,
    tint: [String; 4],
    linear_filter: bool,
    wrap_s: Wrap,
    wrap_t: Wrap,
}

struct Staged {
    overrides: Vec<crate::mesh_material_slots::Settings>,
    all_slots: Vec<crate::mesh_material_slots::Settings>,
}

#[derive(Default)]
pub(crate) struct Panel {
    fields: Vec<SlotFields>,
    selected: Option<u32>,
    baseline: Vec<crate::mesh_material_slots::Settings>,
    baseline_overrides: Vec<crate::mesh_material_slots::Settings>,
    staged: Option<Staged>,
    error: Option<String>,
    model: Option<Arc<StaticModel>>,
}

impl Panel {
    pub(crate) fn load(
        &mut self,
        model: &Arc<StaticModel>,
        overrides: &[crate::mesh_material_slots::Settings],
    ) -> Result<(), String> {
        let used = used_materials(model)?;
        if overrides.len() > 256 {
            return Err("Material slot override limit exceeded".into());
        }
        let mut sorted_overrides = overrides.to_vec();
        sorted_overrides.sort_by_key(|settings| settings.material);
        if sorted_overrides
            .windows(2)
            .any(|pair| pair[0].material == pair[1].material)
        {
            return Err("Duplicate material slot override".into());
        }
        for settings in &sorted_overrides {
            if used.binary_search(&settings.material).is_err() {
                return Err("Material slot override targets an unused or missing slot".into());
            }
            validate_settings(settings)?;
        }

        let mut fields = Vec::with_capacity(used.len());
        let mut baseline = Vec::with_capacity(used.len());
        for material_index in &used {
            let source = model
                .source()
                .materials
                .get(*material_index as usize)
                .ok_or_else(|| "Material slot is outside the admitted model".to_owned())?;
            let default_settings =
                crate::mesh_material_slots::Settings::identity(*material_index, source);
            let settings = sorted_overrides
                .binary_search_by_key(material_index, |settings| settings.material)
                .ok()
                .and_then(|index| sorted_overrides.get(index))
                .unwrap_or(&default_settings);
            let field = SlotFields {
                material: *material_index,
                tint: settings.tint.map(format_number),
                linear_filter: settings.linear_filter,
                wrap_s: settings.wrap_s,
                wrap_t: settings.wrap_t,
            };
            baseline.push(*settings);
            fields.push(field);
        }
        self.selected = used.first().copied();
        self.fields = fields;
        self.baseline = baseline;
        self.baseline_overrides = sorted_overrides;
        self.staged = None;
        self.error = None;
        self.model = Some(Arc::clone(model));
        Ok(())
    }

    pub(crate) fn show(
        &mut self,
        ui: &mut Ui,
        model: &Arc<StaticModel>,
    ) -> Option<Vec<crate::mesh_material_slots::Settings>> {
        if !self.model_matches(model) {
            self.error = Some("The admitted model changed; reload this panel".into());
            ui.label(self.error.as_deref().unwrap_or_default());
            return None;
        }
        let used = match used_materials(model) {
            Ok(used) => used,
            Err(error) => {
                self.error = Some(error);
                ui.label(self.error.as_deref().unwrap_or_default());
                return None;
            }
        };
        if self
            .fields
            .iter()
            .map(|field| field.material)
            .collect::<Vec<_>>()
            != used
        {
            self.error =
                Some("The admitted model's material slots changed; reload this panel".into());
            ui.label(self.error.as_deref().unwrap_or_default());
            return None;
        }
        if used.is_empty() {
            ui.label("This model has no used material slots.");
            return None;
        }

        egui::ComboBox::from_label("Material slot index")
            .selected_text(
                self.selected
                    .map_or_else(|| "None".into(), |index| index.to_string()),
            )
            .show_ui(ui, |ui| {
                for material_index in &used {
                    ui.selectable_value(
                        &mut self.selected,
                        Some(*material_index),
                        material_index.to_string(),
                    );
                }
            });

        let selected = self.selected?;
        let Some(field) = self
            .fields
            .iter_mut()
            .find(|field| field.material == selected)
        else {
            self.error = Some("Selected material slot is no longer available".into());
            ui.label(self.error.as_deref().unwrap_or_default());
            return None;
        };
        for (channel, suffix) in ["R", "G", "B", "A"].into_iter().enumerate() {
            let label = ui.label(format!("Material slot tint {suffix}"));
            ui.add(egui::TextEdit::singleline(&mut field.tint[channel]).desired_width(88.0))
                .labelled_by(label.id);
        }
        ui.checkbox(&mut field.linear_filter, "Material slot linear filtering");
        wrap_combo(ui, "Material slot wrap S", &mut field.wrap_s);
        wrap_combo(ui, "Material slot wrap T", &mut field.wrap_t);

        let current = self.current_overrides(model);
        self.error = current.as_ref().err().cloned();
        if let Some(error) = &self.error {
            ui.label(error);
        }
        let mut staged = None;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    current.is_ok(),
                    egui::Button::new("Stage material overrides"),
                )
                .clicked()
            {
                if let Ok(overrides) = current.as_ref() {
                    if let Ok(all_slots) = self.current_all_slots() {
                        self.staged = Some(Staged {
                            overrides: overrides.clone(),
                            all_slots,
                        });
                        staged = Some(overrides.clone());
                    }
                }
            }
            if ui.button("Reset material overrides").clicked() {
                staged = self.reset_to_baseline();
            }
        });
        staged
    }

    pub(crate) fn dirty(&self) -> bool {
        self.staged.is_some()
            || self.error.is_some()
            || self
                .fields
                .iter()
                .any(|field| field.tint.iter().any(|value| value.parse::<f32>().is_err()))
            || self
                .current_cached()
                .is_none_or(|current| !same_settings(&current, &self.baseline))
    }

    fn model_matches(&self, model: &Arc<StaticModel>) -> bool {
        self.model
            .as_ref()
            .is_some_and(|loaded| Arc::ptr_eq(loaded, model))
    }

    pub(crate) fn staged_matches_fields(&self) -> bool {
        let Some(staged) = &self.staged else {
            return false;
        };
        self.current_all_slots()
            .is_ok_and(|current| same_settings(&current, &staged.all_slots))
    }

    pub(crate) fn acknowledge(&mut self) {
        if let Some(staged) = self.staged.take() {
            self.baseline = staged.all_slots;
            self.baseline_overrides = staged.overrides;
            self.error = None;
        }
    }

    fn reset_to_baseline(&mut self) -> Option<Vec<crate::mesh_material_slots::Settings>> {
        if self.fields.len() != self.baseline.len() {
            self.error =
                Some("Reload the admitted model before resetting material overrides".into());
            return None;
        }
        for (field, settings) in self.fields.iter_mut().zip(&self.baseline) {
            if field.material != settings.material {
                self.error =
                    Some("Reload the admitted model before resetting material overrides".into());
                return None;
            }
            field.tint = settings.tint.map(format_number);
            field.linear_filter = settings.linear_filter;
            field.wrap_s = settings.wrap_s;
            field.wrap_t = settings.wrap_t;
        }
        self.error = None;
        self.staged = Some(Staged {
            overrides: self.baseline_overrides.clone(),
            all_slots: self.baseline.clone(),
        });
        Some(self.baseline_overrides.clone())
    }

    fn current_overrides(
        &self,
        model: &StaticModel,
    ) -> Result<Vec<crate::mesh_material_slots::Settings>, String> {
        let mut overrides = Vec::new();
        for field in &self.fields {
            let source = model
                .source()
                .materials
                .get(field.material as usize)
                .ok_or_else(|| "Material slot is outside the admitted model".to_owned())?;
            let mut tint = [0.0; 4];
            for (index, value) in field.tint.iter().enumerate() {
                tint[index] = value
                    .parse::<f32>()
                    .map_err(|_| "Material slot tint must be finite RGBA in 0..=1")?;
            }
            let settings = crate::mesh_material_slots::Settings {
                material: field.material,
                tint,
                linear_filter: field.linear_filter,
                wrap_s: field.wrap_s,
                wrap_t: field.wrap_t,
            };
            validate_settings(&settings)?;
            if !settings.is_identity_for(source) {
                overrides.push(settings);
            }
        }
        Ok(overrides)
    }

    fn current_cached(&self) -> Option<Vec<crate::mesh_material_slots::Settings>> {
        self.fields
            .iter()
            .map(|field| {
                let mut tint = [0.0; 4];
                for (index, value) in field.tint.iter().enumerate() {
                    tint[index] = value.parse().ok()?;
                }
                let settings = crate::mesh_material_slots::Settings {
                    material: field.material,
                    tint,
                    linear_filter: field.linear_filter,
                    wrap_s: field.wrap_s,
                    wrap_t: field.wrap_t,
                };
                validate_settings(&settings).ok()?;
                Some(settings)
            })
            .collect()
    }

    fn current_all_slots(&self) -> Result<Vec<crate::mesh_material_slots::Settings>, String> {
        self.current_cached()
            .ok_or_else(|| "Material slot fields contain an invalid value".into())
    }
}

fn used_materials(model: &StaticModel) -> Result<Vec<u32>, String> {
    let used = model
        .source()
        .primitives
        .iter()
        .map(|primitive| primitive.material)
        .collect::<std::collections::BTreeSet<_>>();
    if used.len() > 256
        || used
            .iter()
            .any(|slot| *slot as usize >= model.source().materials.len())
    {
        return Err("Admitted model has invalid or excessive used material slots".into());
    }
    Ok(used.into_iter().collect())
}

fn validate_settings(settings: &crate::mesh_material_slots::Settings) -> Result<(), String> {
    if !settings
        .tint
        .iter()
        .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
    {
        return Err("Material slot tint must be finite RGBA in 0..=1".into());
    }
    Ok(())
}

fn same_settings(
    left: &[crate::mesh_material_slots::Settings],
    right: &[crate::mesh_material_slots::Settings],
) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.material == right.material
                && left.tint.map(f32::to_bits) == right.tint.map(f32::to_bits)
                && left.linear_filter == right.linear_filter
                && left.wrap_s == right.wrap_s
                && left.wrap_t == right.wrap_t
        })
}

fn format_number(value: f32) -> String {
    value.to_string()
}

fn wrap_combo(ui: &mut Ui, label: &str, value: &mut Wrap) {
    egui::ComboBox::from_label(label)
        .selected_text(match value {
            Wrap::Clamp => "Clamp",
            Wrap::Repeat => "Repeat",
            Wrap::Mirror => "Mirror",
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(value, Wrap::Clamp, "Clamp");
            ui.selectable_value(value, Wrap::Repeat, "Repeat");
            ui.selectable_value(value, Wrap::Mirror, "Mirror");
        });
}

#[cfg(test)]
#[path = "mesh_material_slots_panel/tests.rs"]
mod tests;
