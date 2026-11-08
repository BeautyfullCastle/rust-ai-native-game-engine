//! Cooked-static sampler settings are selected from the current used-material catalog.
use super::*;
use crate::model_package_import::StaticSamplerVariant;
use orr_model::Wrap;

#[derive(Clone, Debug, PartialEq, Eq)]
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
    spec: StaticSamplerVariant,
}
fn wrap(ui: &mut Ui, label: &str, axis: &str, value: &mut Wrap) {
    egui::ComboBox::from_label(label)
        .selected_text(format!("{value:?}"))
        .show_ui(ui, |ui| {
            for (wrap, name) in [
                (Wrap::Clamp, "Clamp"),
                (Wrap::Repeat, "Repeat"),
                (Wrap::Mirror, "Mirror"),
            ] {
                ui.selectable_value(value, wrap, format!("{name} static {axis}"));
            }
        });
}
impl Panel {
    pub(super) fn static_sampler_request(&self) -> Result<Option<StaticSamplerVariant>, String> {
        if !self.static_sampler.enabled {
            return Ok(None);
        }
        if self.source_format != SourceFormat::CookedStatic
            || self.kind != ModelKind::Static
            || self.static_uv.enabled
            || self.static_texture.enabled
            || self.static_assembly.enabled
            || self.static_pivot.enabled
            || self.glb_subtree.enabled
            || self.color_variant_enabled
            || self.texture_variant_enabled
            || self.trim.enabled
            || self.sampler.enabled
            || self.sequence.enabled
            || self.uv.enabled
        {
            return Err("Choose only cooked static sampler mode".into());
        }
        let catalog = self
            .inspection
            .current_catalog(&self.source, self.kind, self.source_format)
            .ok_or("Inspect current static source first")?;
        let material = self
            .static_sampler
            .material
            .ok_or("Inspect static source and choose a used material")?;
        if self.inspected_hash.as_ref() != Some(&catalog.source_sha256)
            || !catalog
                .materials
                .iter()
                .any(|entry| entry.index == material && entry.used_primitive_count > 0)
        {
            return Err("Inspect and select a current used static material again".into());
        }
        let spec = StaticSamplerVariant {
            material,
            linear_filter: self.static_sampler.linear_filter,
            wrap_s: self.static_sampler.wrap_s,
            wrap_t: self.static_sampler.wrap_t,
            asset_id: self.static_sampler.asset_id.clone(),
        };
        model_package_import::validate_static_sampler_variant(&spec)?;
        Ok(Some(spec))
    }
    pub(super) fn static_sampler_snapshot(
        &self,
        root: &Path,
        project: u64,
        generation: u64,
    ) -> Result<Option<Snapshot>, String> {
        let Some(spec) = self.static_sampler_request()? else {
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
    pub(super) fn show_static_sampler_controls(&mut self, ui: &mut Ui) {
        if self.source_format != SourceFormat::CookedStatic || self.kind != ModelKind::Static {
            return;
        }
        let before = self.static_sampler.clone();
        if ui
            .checkbox(
                &mut self.static_sampler.enabled,
                "Create static sampler variant",
            )
            .changed()
        {
            self.static_sampler.material = None;
            self.inspected_hash = None;
            if self.static_sampler.enabled {
                self.static_uv.enabled = false;
                self.static_texture.enabled = false;
                self.static_assembly.enabled = false;
                self.static_pivot.enabled = false;
                self.glb_subtree.enabled = false;
                self.color_variant_enabled = false;
                self.texture_variant_enabled = false;
                self.trim.enabled = false;
                self.sampler.enabled = false;
                self.sequence.enabled = false;
                self.uv.enabled = false;
            }
        }
        if self.static_sampler.enabled {
            ui.label(
                self.static_sampler
                    .material
                    .map(|material| format!("Static sampler material: {material}"))
                    .unwrap_or_else(|| "Inspect static source and Use material first".into()),
            );
            egui::ComboBox::from_label("Static sampler filtering")
                .selected_text(if self.static_sampler.linear_filter {
                    "Bilinear static filtering"
                } else {
                    "Nearest static filtering"
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.static_sampler.linear_filter,
                        false,
                        "Nearest static filtering",
                    );
                    ui.selectable_value(
                        &mut self.static_sampler.linear_filter,
                        true,
                        "Bilinear static filtering",
                    );
                });
            wrap(
                ui,
                "Static sampler wrap S",
                "S",
                &mut self.static_sampler.wrap_s,
            );
            wrap(
                ui,
                "Static sampler wrap T",
                "T",
                &mut self.static_sampler.wrap_t,
            );
            text_field(
                ui,
                "Static sampler variant asset ID",
                &mut self.static_sampler.asset_id,
                240,
            );
            ui.weak("Change filtering and S/T wrapping only on the explicitly chosen used material. Shared image pixels, other materials and geometry stay unchanged. Import requires rights acknowledgement and a new destination; preview uses the existing imported-package preview after import.");
        }
        if before != self.static_sampler {
            self.source_inputs_changed();
        }
        if self.static_sampler.enabled {
            if let Err(error) = self.static_sampler_request() {
                ui.weak(error);
            }
        }
    }
}
#[cfg(test)]
#[path = "static_sampler_ui_tests.rs"]
mod tests;
