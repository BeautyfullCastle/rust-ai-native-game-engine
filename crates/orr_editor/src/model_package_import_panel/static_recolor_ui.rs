//! Linear RGB factor replacement for one explicitly inspected used static material.
use super::*;
use crate::model_package_import::StaticColorVariant;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Controls {
    pub(super) enabled: bool,
    pub(super) material: Option<u32>,
    pub(super) linear_rgb: [String; 3],
    pub(super) asset_id: String,
}
impl Default for Controls {
    fn default() -> Self {
        Self {
            enabled: false,
            material: None,
            linear_rgb: ["1".into(), "1".into(), "1".into()],
            asset_id: "variants/material.recolor.json".into(),
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
    spec: StaticColorVariant,
}
fn number(raw: &str) -> Result<f64, String> {
    let value = raw
        .trim()
        .parse::<f64>()
        .map_err(|_| "Static linear RGB must be finite decimal values in 0..1")?;
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(
            "Static linear RGB must be finite f64 values in 0..1 before f32 conversion".into(),
        );
    }
    Ok(value)
}
impl Panel {
    pub(super) fn static_recolor_request(&self) -> Result<Option<StaticColorVariant>, String> {
        if !self.static_recolor.enabled {
            return Ok(None);
        }
        if self.source_format != SourceFormat::CookedStatic
            || self.kind != ModelKind::Static
            || self.static_uv.enabled
            || self.static_texture.enabled
            || self.static_assembly.enabled
            || self.static_pivot.enabled
            || self.static_sampler.enabled
            || self.glb_subtree.enabled
            || self.color_variant_enabled
            || self.texture_variant_enabled
            || self.trim.enabled
            || self.sampler.enabled
            || self.sequence.enabled
            || self.uv.enabled
        {
            return Err("Choose only cooked static color mode".into());
        }
        let catalog = self
            .inspection
            .current_catalog(&self.source, self.kind, self.source_format)
            .ok_or("Inspect current static source first")?;
        let material = self
            .static_recolor
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
        let spec = StaticColorVariant {
            material,
            linear_rgb: [
                number(&self.static_recolor.linear_rgb[0])?,
                number(&self.static_recolor.linear_rgb[1])?,
                number(&self.static_recolor.linear_rgb[2])?,
            ],
            asset_id: self.static_recolor.asset_id.clone(),
        };
        model_package_import::validate_static_color_variant(&spec)?;
        Ok(Some(spec))
    }
    pub(super) fn static_recolor_snapshot(
        &self,
        root: &Path,
        project: u64,
        generation: u64,
    ) -> Result<Option<Snapshot>, String> {
        let Some(spec) = self.static_recolor_request()? else {
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
    pub(super) fn show_static_recolor_controls(&mut self, ui: &mut Ui) {
        if self.source_format != SourceFormat::CookedStatic || self.kind != ModelKind::Static {
            return;
        }
        let before = self.static_recolor.clone();
        if ui
            .checkbox(
                &mut self.static_recolor.enabled,
                "Create static color variant",
            )
            .changed()
        {
            self.static_recolor.material = None;
            self.inspected_hash = None;
            if self.static_recolor.enabled {
                self.static_uv.enabled = false;
                self.static_texture.enabled = false;
                self.static_assembly.enabled = false;
                self.static_pivot.enabled = false;
                self.static_sampler.enabled = false;
                self.glb_subtree.enabled = false;
                self.color_variant_enabled = false;
                self.texture_variant_enabled = false;
                self.trim.enabled = false;
                self.sampler.enabled = false;
                self.sequence.enabled = false;
                self.uv.enabled = false;
            }
        }
        if self.static_recolor.enabled {
            ui.label(
                self.static_recolor
                    .material
                    .map(|material| format!("Static color material: {material}"))
                    .unwrap_or_else(|| "Inspect static source and Use material first".into()),
            );
            for (axis, label) in ["R", "G", "B"].into_iter().enumerate() {
                text_field(
                    ui,
                    &format!("Static linear {label} (0..1)"),
                    &mut self.static_recolor.linear_rgb[axis],
                    64,
                );
            }
            text_field(
                ui,
                "Static color variant asset ID",
                &mut self.static_recolor.asset_id,
                240,
            );
            ui.weak("Replace the selected material's linear RGB factor in 0..1. This is not an sRGB color picker and performs no sRGB conversion. Alpha, image pixels, other materials and geometry stay unchanged. Import requires rights acknowledgement and a new destination; preview uses the existing imported-package preview after import.");
        }
        if before != self.static_recolor {
            self.source_inputs_changed();
        }
        if self.static_recolor.enabled {
            if let Err(error) = self.static_recolor_request() {
                ui.weak(error);
            }
        }
    }
}
#[cfg(test)]
#[path = "static_recolor_ui_tests.rs"]
mod tests;
