//! Assemble exactly two transformed copies of one explicitly inspected source.
use super::*;
use crate::model_package_import::{StaticAssemblyPart, StaticAssemblyVariant};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct PartControls {
    pub(super) offset: [String; 3],
    pub(super) scale: [String; 3],
    pub(super) quarter_turns: u32,
}
impl Default for PartControls {
    fn default() -> Self {
        Self {
            offset: ["0".into(), "0".into(), "0".into()],
            scale: ["1".into(), "1".into(), "1".into()],
            quarter_turns: 0,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Controls {
    pub(super) enabled: bool,
    pub(super) source_sha256: Option<String>,
    pub(super) parts: [PartControls; 2],
    pub(super) asset_id: String,
}
impl Default for Controls {
    fn default() -> Self {
        let first = PartControls::default();
        let mut second = PartControls::default();
        second.offset[0] = "2".into();
        Self {
            enabled: false,
            source_sha256: None,
            parts: [first, second],
            asset_id: "variants/pair.assemble.json".into(),
        }
    }
}

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
    spec: StaticAssemblyVariant,
}

fn number(raw: &str, positive: bool) -> Result<f64, String> {
    let value = raw
        .trim()
        .parse::<f64>()
        .map_err(|_| "Assembly value must be finite decimal")?;
    if !value.is_finite()
        || value.abs() > f64::from(f32::MAX)
        || (positive && (value <= 0.0 || value as f32 <= 0.0))
    {
        return Err(
            "Assembly values must fit finite f32; scales must stay positive after conversion"
                .into(),
        );
    }
    Ok(value)
}
impl Panel {
    pub(super) fn static_assembly_request(&self) -> Result<Option<StaticAssemblyVariant>, String> {
        if !self.static_assembly.enabled {
            return Ok(None);
        }
        if self.source_format != SourceFormat::CookedStatic
            || self.kind != ModelKind::Static
            || self.static_uv.enabled
            || self.static_texture.enabled
            || self.color_variant_enabled
            || self.texture_variant_enabled
            || self.trim.enabled
            || self.sampler.enabled
            || self.sequence.enabled
            || self.uv.enabled
        {
            return Err("Choose only cooked static two-copy assembly mode".into());
        }
        let catalog = self
            .inspection
            .current_catalog(&self.source, self.kind, self.source_format)
            .ok_or("Inspect current static source first")?;
        if self.static_assembly.source_sha256.as_ref() != Some(&catalog.source_sha256)
            || self.inspected_hash.as_ref() != Some(&catalog.source_sha256)
        {
            return Err("Use inspected source for assembly explicitly before import".into());
        }
        let part = |controls: &PartControls| -> Result<StaticAssemblyPart, String> {
            Ok(StaticAssemblyPart {
                offset: [
                    number(&controls.offset[0], false)?,
                    number(&controls.offset[1], false)?,
                    number(&controls.offset[2], false)?,
                ],
                scale: [
                    number(&controls.scale[0], true)?,
                    number(&controls.scale[1], true)?,
                    number(&controls.scale[2], true)?,
                ],
                quarter_turns: controls.quarter_turns,
            })
        };
        let spec = StaticAssemblyVariant {
            parts: [
                part(&self.static_assembly.parts[0])?,
                part(&self.static_assembly.parts[1])?,
            ],
            asset_id: self.static_assembly.asset_id.clone(),
        };
        model_package_import::validate_static_assembly_variant(&spec)?;
        Ok(Some(spec))
    }
    pub(super) fn static_assembly_snapshot(
        &self,
        root: &Path,
        project: u64,
        generation: u64,
    ) -> Result<Option<Snapshot>, String> {
        let Some(spec) = self.static_assembly_request()? else {
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
    pub(super) fn show_static_assembly_controls(&mut self, ui: &mut Ui) {
        if self.source_format != SourceFormat::CookedStatic || self.kind != ModelKind::Static {
            return;
        }
        let before = self.static_assembly.clone();
        if ui
            .checkbox(
                &mut self.static_assembly.enabled,
                "Create static two-copy assembly",
            )
            .changed()
            && self.static_assembly.enabled
        {
            self.static_uv.enabled = false;
            self.static_texture.enabled = false;
            self.color_variant_enabled = false;
            self.texture_variant_enabled = false;
            self.trim.enabled = false;
            self.sampler.enabled = false;
            self.sequence.enabled = false;
            self.uv.enabled = false;
            self.static_assembly.source_sha256 = None;
            self.inspected_hash = None;
        }
        if self.static_assembly.enabled {
            let catalog_hash = self
                .inspection
                .current_catalog(&self.source, self.kind, self.source_format)
                .map(|catalog| catalog.source_sha256.clone());
            if ui
                .add_enabled(
                    !self.busy() && catalog_hash.is_some(),
                    egui::Button::new("Use inspected source for assembly"),
                )
                .clicked()
            {
                self.static_assembly.source_sha256 = catalog_hash.clone();
                self.inspected_hash = catalog_hash;
            }
            ui.label(if self.static_assembly.source_sha256.is_some() {
                "Assembly source chosen: both copies use this inspected static source"
            } else {
                "Inspect static source, then Use inspected source for assembly"
            });
            for (index, part) in self.static_assembly.parts.iter_mut().enumerate() {
                let part_number = index + 1;
                for (axis_index, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
                    text_field(
                        ui,
                        &format!("Assembly part {part_number} offset {axis}"),
                        &mut part.offset[axis_index],
                        64,
                    );
                    text_field(
                        ui,
                        &format!("Assembly part {part_number} scale {axis}"),
                        &mut part.scale[axis_index],
                        64,
                    );
                }
                egui::ComboBox::from_label(format!("Assembly part {part_number} yaw"))
                    .selected_text(format!("{} degrees", part.quarter_turns.saturating_mul(90)))
                    .show_ui(ui, |ui| {
                        for turns in 0..4 {
                            ui.selectable_value(
                                &mut part.quarter_turns,
                                turns,
                                format!("{} degrees", turns * 90),
                            );
                        }
                    });
            }
            text_field(
                ui,
                "Static assembly variant asset ID",
                &mut self.static_assembly.asset_id,
                240,
            );
            ui.weak("Create two copies of the same inspected source. Each copy scales, then turns about +Y, then translates; 90 degrees maps +X toward -Z. Import validates combined model limits. Model preview uses the existing imported-package preview after import.");
        }
        if before != self.static_assembly {
            self.source_inputs_changed();
        }
        if self.static_assembly.enabled {
            if let Err(error) = self.static_assembly_request() {
                ui.weak(error);
            }
        }
    }
}

#[cfg(test)]
#[path = "static_assembly_ui_tests.rs"]
mod tests;
