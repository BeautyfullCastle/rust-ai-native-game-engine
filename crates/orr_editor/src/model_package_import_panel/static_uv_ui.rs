//! Cooked static UV settings; inspection and settings validation precede import.
use super::*;
use crate::model_package_import::StaticUvVariant;

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

/// Complete static submission identity, including inputs that may change without
/// a widget edit (for example a project reload while a worker is running).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Snapshot {
    root: PathBuf,
    project: u64,
    generation: u64,
    authoring_generation: u64,
    source: PathBuf,
    license: PathBuf,
    destination: PathBuf,
    name: String,
    version: String,
    kind: ModelKind,
    format: SourceFormat,
    source_sha256: String,
    spec: StaticUvVariant,
}

fn number(raw: &str, positive: bool) -> Result<f64, String> {
    let value = raw
        .trim()
        .parse::<f64>()
        .map_err(|_| "Static UV value must be finite decimal")?;
    if !value.is_finite()
        || value.abs() > f64::from(f32::MAX)
        || (positive && (value <= 0.0 || value as f32 <= 0.0))
    {
        return Err("Static UV values must fit finite f32; scales must stay positive after conversion. Import validates output UV bounds.".into());
    }
    Ok(value)
}

impl Panel {
    pub(super) fn static_uv_request(&self) -> Result<Option<StaticUvVariant>, String> {
        if !self.static_uv.enabled {
            return Ok(None);
        }
        if self.source_format != SourceFormat::CookedStatic
            || self.kind != ModelKind::Static
            || self.static_texture.enabled
            || self.static_assembly.enabled
            || self.static_pivot.enabled
            || self.static_sampler.enabled
            || self.static_recolor.enabled
            || self.static_planar.enabled
            || self.color_variant_enabled
            || self.texture_variant_enabled
            || self.trim.enabled
            || self.sampler.enabled
            || self.sequence.enabled
            || self.uv.enabled
        {
            return Err("Choose only cooked static UV mode".into());
        }
        let material = self
            .static_uv
            .material
            .ok_or("Inspect static source and select a used material")?;
        let catalog = self
            .inspection
            .current_catalog(&self.source, self.kind, self.source_format)
            .ok_or("Inspect current static source first")?;
        if self.inspected_hash.as_ref() != Some(&catalog.source_sha256)
            || !catalog
                .materials
                .iter()
                .any(|m| m.index == material && m.used_primitive_count > 0)
        {
            return Err("Inspect and select a current used static material again".into());
        }
        let offset = [
            number(&self.static_uv.offset[0], false)?,
            number(&self.static_uv.offset[1], false)?,
        ];
        let scale = [
            number(&self.static_uv.scale[0], true)?,
            number(&self.static_uv.scale[1], true)?,
        ];
        if self.static_uv.quarter_turns > 3 {
            return Err("Choose a static UV quarter-turn rotation".into());
        }
        let id = &self.static_uv.asset_id;
        if id != id.trim()
            || !id.ends_with(".uv.json")
            || id.contains('\\')
            || id.contains(':')
            || id.chars().any(char::is_control)
            || id
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err("Static UV asset ID must be a relative .uv.json logical path".into());
        }
        Ok(Some(StaticUvVariant {
            material,
            offset,
            scale,
            quarter_turns: self.static_uv.quarter_turns,
            asset_id: id.clone(),
        }))
    }

    pub(super) fn static_uv_snapshot(
        &self,
        root: &Path,
        project: u64,
        generation: u64,
    ) -> Result<Option<Snapshot>, String> {
        let Some(spec) = self.static_uv_request()? else {
            return Ok(None);
        };
        let request = self.request();
        Ok(Some(Snapshot {
            root: root.into(),
            project,
            generation,
            authoring_generation: self.authoring_generation,
            source: request.source,
            license: request.license,
            destination: request.destination,
            name: request.name,
            version: request.version,
            kind: request.kind,
            format: request.source_format,
            source_sha256: self
                .inspected_hash
                .clone()
                .ok_or("Inspect current static source first")?,
            spec,
        }))
    }

    pub(super) fn show_static_uv_controls(&mut self, ui: &mut Ui) {
        if self.source_format != SourceFormat::CookedStatic || self.kind != ModelKind::Static {
            return;
        }
        let before = self.static_uv.clone();
        if ui.checkbox(&mut self.static_uv.enabled, "Create static UV variant").changed() && self.static_uv.enabled {
            self.static_texture.enabled = false;
            self.static_assembly.enabled = false;
            self.static_pivot.enabled = false;
            self.static_sampler.enabled = false;
            self.static_recolor.enabled = false;
            self.static_planar.enabled = false;
        }
        if self.static_uv.enabled {
            ui.label(
                self.static_uv
                    .material
                    .map(|m| format!("Static UV material: {m}"))
                    .unwrap_or_else(|| "Inspect static source and Use material first".into()),
            );
            for (i, axis) in ["U", "V"].into_iter().enumerate() {
                text_field(
                    ui,
                    &format!("Static UV offset {axis}"),
                    &mut self.static_uv.offset[i],
                    64,
                );
                text_field(
                    ui,
                    &format!("Static UV scale {axis}"),
                    &mut self.static_uv.scale[i],
                    64,
                );
            }
            egui::ComboBox::from_label("Static UV quarter-turn rotation")
                .selected_text(format!(
                    "{} degrees",
                    self.static_uv.quarter_turns.saturating_mul(90)
                ))
                .show_ui(ui, |ui| {
                    for turns in 0..4 {
                        ui.selectable_value(
                            &mut self.static_uv.quarter_turns,
                            turns,
                            format!("{} degrees", turns * 90),
                        );
                    }
                });
            text_field(
                ui,
                "Static UV variant asset ID",
                &mut self.static_uv.asset_id,
                240,
            );
            ui.weak("Static catalog and settings validation before Import: selected material only, scale then quarter-turn about UV origin then offset. Import validates output UV bounds. Model preview is available through the existing imported-package preview after import.");
        }
        if before != self.static_uv {
            self.source_inputs_changed();
        }
        if self.static_uv.enabled {
            if let Err(error) = self.static_uv_request() {
                ui.weak(error);
            }
        }
    }
}

#[cfg(test)]
#[path = "static_uv_ui_tests.rs"]
mod tests;
