//! UV authoring retains decimal precision until bounded validation.
use super::*;
use crate::model_package_import::UvVariant;
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Controls {
    pub(super) enabled: bool,
    pub(super) material: Option<u32>,
    pub(super) offset: [String; 2],
    pub(super) scale: [String; 2],
    pub(super) quarter_turns: u32,
    pub(super) asset_id: String,
}
impl Default for Controls {
    fn default() -> Self {
        Self {
            enabled: false,
            material: None,
            offset: ["0".into(), "0".into()],
            scale: ["1".into(), "1".into()],
            quarter_turns: 0,
            asset_id: "variants/material.uv.json".into(),
        }
    }
}
fn number(raw: &str, positive: bool) -> Result<f64, String> {
    let value = raw
        .trim()
        .parse::<f64>()
        .map_err(|_| "UV value must be finite decimal")?;
    if !value.is_finite()
        || value.abs() > f64::from(f32::MAX)
        || (positive && (value <= 0.0 || value as f32 <= 0.0))
    {
        return Err("UV values must fit finite f32; scales must stay positive after conversion. Output UV bounds are validated during preview/import".into());
    }
    Ok(value)
}
impl Panel {
    pub(super) fn uv_request(&self) -> Result<Option<UvVariant>, String> {
        if !self.uv.enabled {
            return Ok(None);
        }
        if !cfg!(feature = "room-animated-models")
            || self.source_format != SourceFormat::CookedAnimated
            || self.kind != ModelKind::Animated
            || self.color_variant_enabled
            || self.texture_variant_enabled
            || self.trim.enabled
            || self.sampler.enabled
            || self.sequence.enabled
        {
            return Err("Choose only cooked animated UV mode".into());
        }
        let material = self
            .uv
            .material
            .ok_or("Inspect source and select a used material")?;
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
        let offset = [
            number(&self.uv.offset[0], false)?,
            number(&self.uv.offset[1], false)?,
        ];
        let scale = [
            number(&self.uv.scale[0], true)?,
            number(&self.uv.scale[1], true)?,
        ];
        if self.uv.quarter_turns > 3 {
            return Err("Choose a quarter-turn rotation".into());
        }
        let id = &self.uv.asset_id;
        if id != id.trim()
            || !id.ends_with(".uv.json")
            || id.contains('\\')
            || id.contains(':')
            || id.chars().any(char::is_control)
            || id.split('/').any(|p| p.is_empty() || p == "." || p == "..")
        {
            return Err("UV asset ID must be a relative .uv.json logical path".into());
        }
        Ok(Some(UvVariant {
            material,
            offset,
            scale,
            quarter_turns: self.uv.quarter_turns,
            asset_id: id.clone(),
        }))
    }
    pub(super) fn show_uv_controls(&mut self, ui: &mut Ui) {
        if !cfg!(feature = "room-animated-models")
            || self.source_format != SourceFormat::CookedAnimated
            || self.kind != ModelKind::Animated
        {
            return;
        }
        let before = self.uv.clone();
        if ui
            .checkbox(&mut self.uv.enabled, "Create animated UV variant")
            .changed()
            && self.uv.enabled
        {
            self.color_variant_enabled = false;
            self.texture_variant_enabled = false;
            self.trim.enabled = false;
            self.sampler.enabled = false;
            self.sequence.enabled = false;
        }
        if self.uv.enabled {
            ui.label(
                self.uv
                    .material
                    .map(|m| format!("UV material: {m}"))
                    .unwrap_or_else(|| "Inspect source and Use material first".into()),
            );
            for (i, axis) in ["U", "V"].into_iter().enumerate() {
                text_field(ui, &format!("UV offset {axis}"), &mut self.uv.offset[i], 64);
                text_field(ui, &format!("UV scale {axis}"), &mut self.uv.scale[i], 64);
            }
            egui::ComboBox::from_label("UV quarter-turn rotation")
                .selected_text(format!("{} degrees", self.uv.quarter_turns * 90))
                .show_ui(ui, |ui| {
                    for turns in 0..4 {
                        ui.selectable_value(
                            &mut self.uv.quarter_turns,
                            turns,
                            format!("{} degrees", turns * 90),
                        );
                    }
                });
            text_field(ui, "UV variant asset ID", &mut self.uv.asset_id, 240);
            ui.weak("Selected material only: scale, quarter-turn about UV origin, then offset. Geometry, images and animation remain unchanged. Preview validates output UV limits without publishing; import requires rights and a new destination.");
        }
        if before != self.uv {
            self.source_inputs_changed();
        }
        if self.uv.enabled {
            if let Err(error) = self.uv_request() {
                ui.weak(error);
            }
        }
    }
}
#[cfg(test)]
#[path = "uv_ui_tests.rs"]
mod tests;
