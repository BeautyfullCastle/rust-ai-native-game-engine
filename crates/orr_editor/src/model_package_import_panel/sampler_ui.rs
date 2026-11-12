//! Typed sampler authoring uses only a current inspected, used material.
use super::*;
use crate::model_package_import::SamplerVariant;
use orr_model::Wrap;
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Controls {
    pub(super) enabled: bool,
    pub(super) material: Option<u32>,
    pub(super) linear_filter: bool,
    pub(super) wrap_s: Wrap,
    pub(super) wrap_t: Wrap,
    pub(super) asset_id: String,
}
impl Default for Controls {
    fn default() -> Self {
        Self {
            enabled: false,
            material: None,
            linear_filter: true,
            wrap_s: Wrap::Repeat,
            wrap_t: Wrap::Repeat,
            asset_id: "variants/material.sampler.json".into(),
        }
    }
}
fn wrap(ui: &mut Ui, label: &str, axis: &str, value: &mut Wrap) {
    egui::ComboBox::from_label(label)
        .selected_text(format!("{value:?}"))
        .show_ui(ui, |ui| {
            for (w, name) in [
                (Wrap::Clamp, "Clamp"),
                (Wrap::Repeat, "Repeat"),
                (Wrap::Mirror, "Mirror"),
            ] {
                ui.selectable_value(value, w, format!("{name} {axis}"));
            }
        });
}
impl Panel {
    pub(super) fn sampler_request(&self) -> Result<Option<SamplerVariant>, String> {
        if !self.sampler.enabled {
            return Ok(None);
        }
        if !cfg!(feature = "room-animated-models")
            || self.source_format != SourceFormat::CookedAnimated
            || self.kind != ModelKind::Animated
            || self.color_variant_enabled
            || self.texture_variant_enabled
            || self.trim.enabled
            || self.sequence.enabled
            || self.uv.enabled
            || self.pose_bake.enabled
        {
            return Err("Choose only cooked animated sampler mode".into());
        }
        let material = self
            .sampler
            .material
            .ok_or("Inspect source and choose a used material")?;
        let catalog = self
            .inspection
            .catalog()
            .ok_or("Inspect current animated source first")?;
        if self.inspected_hash.as_ref() != Some(&catalog.source_sha256)
            || !catalog
                .materials
                .iter()
                .any(|m| m.index == material && m.used_primitive_count > 0)
        {
            return Err("Inspect and select a current used material again".into());
        }
        let id = &self.sampler.asset_id;
        if id != id.trim()
            || !id.ends_with(".sampler.json")
            || id.contains('\\')
            || id.contains(':')
            || id.chars().any(char::is_control)
            || id.split('/').any(|p| p.is_empty() || p == "." || p == "..")
        {
            return Err("Sampler asset ID must be a relative .sampler.json logical path".into());
        }
        Ok(Some(SamplerVariant {
            material,
            linear_filter: self.sampler.linear_filter,
            wrap_s: self.sampler.wrap_s,
            wrap_t: self.sampler.wrap_t,
            asset_id: id.clone(),
        }))
    }
    pub(super) fn show_sampler_controls(&mut self, ui: &mut Ui) {
        if !cfg!(feature = "room-animated-models")
            || self.source_format != SourceFormat::CookedAnimated
            || self.kind != ModelKind::Animated
        {
            return;
        }
        let before = self.sampler.clone();
        if ui
            .checkbox(&mut self.sampler.enabled, "Create animated sampler variant")
            .changed()
            && self.sampler.enabled
        {
            self.sequence.enabled=false;
            self.pose_bake.enabled = false;
            self.uv.enabled=false;
            self.color_variant_enabled = false;
            self.texture_variant_enabled = false;
            self.trim.enabled = false;
        }
        if self.sampler.enabled {
            ui.label(
                self.sampler
                    .material
                    .map(|m| format!("Sampler material: {m}"))
                    .unwrap_or_else(|| "Inspect source and Use material first".into()),
            );
            egui::ComboBox::from_label("Sampler filtering")
                .selected_text(if self.sampler.linear_filter {
                    "Linear filtering"
                } else {
                    "Nearest filtering"
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.sampler.linear_filter, true, "Linear filtering");
                    ui.selectable_value(
                        &mut self.sampler.linear_filter,
                        false,
                        "Nearest filtering",
                    );
                });
            wrap(ui, "Sampler wrap S", "S", &mut self.sampler.wrap_s);
            wrap(ui, "Sampler wrap T", "T", &mut self.sampler.wrap_t);
            text_field(
                ui,
                "Sampler variant asset ID",
                &mut self.sampler.asset_id,
                240,
            );
            ui.weak("Sampler only: filter and wrapping change the selected used material; image pixels, geometry and clips remain unchanged. Preview is read-only. Import requires rights acknowledgement and a new destination.");
        }
        if self.sampler != before {
            self.source_inputs_changed();
        }
        if self.sampler.enabled {
            if let Err(error) = self.sampler_request() {
                ui.weak(error);
            }
        }
    }
}
#[cfg(test)]
#[path = "sampler_ui_tests.rs"]
mod tests;
