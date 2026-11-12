//! Explicit cooked-static T(-pivot) rebase; source geometry stays immutable.
use super::*;
use crate::model_package_import::StaticPivotVariant;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Controls {
    pub(super) enabled: bool,
    pub(super) source_sha256: Option<String>,
    pub(super) pivot: [String; 3],
    pub(super) asset_id: String,
}
impl Default for Controls {
    fn default() -> Self {
        Self {
            enabled: false,
            source_sha256: None,
            pivot: ["0".into(), "0".into(), "0".into()],
            asset_id: "variants/model.pivot.json".into(),
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
    spec: StaticPivotVariant,
}

/// Match the CLI's finite f64 -> f32 -> finite +/-1e6 admission. Keep the
/// original decimal value for the backend to perform that same conversion.
fn number(raw: &str) -> Result<f64, String> {
    let value = raw
        .trim()
        .parse::<f64>()
        .map_err(|_| "Pivot must be a finite decimal")?;
    let converted = value as f32;
    if !value.is_finite() || !converted.is_finite() || converted.abs() > 1_000_000.0 {
        return Err("Pivot must convert to a finite f32 within -1000000..=1000000".into());
    }
    Ok(value)
}

impl Panel {
    pub(super) fn static_pivot_request(&self) -> Result<Option<StaticPivotVariant>, String> {
        if !self.static_pivot.enabled {
            return Ok(None);
        }
        if self.source_format != SourceFormat::CookedStatic
            || self.kind != ModelKind::Static
            || self.static_uv.enabled
            || self.static_sampler.enabled
            || self.static_recolor.enabled
            || self.static_planar.enabled
            || self.static_texture.enabled
            || self.static_assembly.enabled
            || self.glb_subtree.enabled
            || self.color_variant_enabled
            || self.texture_variant_enabled
            || self.trim.enabled
            || self.sampler.enabled
            || self.sequence.enabled
            || self.uv.enabled
        {
            return Err("Choose only cooked static pivot mode".into());
        }
        let catalog = self
            .inspection
            .current_catalog(&self.source, self.kind, self.source_format)
            .ok_or("Inspect current static source first")?;
        if self.static_pivot.source_sha256.as_ref() != Some(&catalog.source_sha256)
            || self.inspected_hash.as_ref() != Some(&catalog.source_sha256)
        {
            return Err("Use inspected source for pivot explicitly before import".into());
        }
        let spec = StaticPivotVariant {
            pivot: [
                number(&self.static_pivot.pivot[0])?,
                number(&self.static_pivot.pivot[1])?,
                number(&self.static_pivot.pivot[2])?,
            ],
            asset_id: self.static_pivot.asset_id.clone(),
        };
        model_package_import::validate_static_pivot_variant(&spec)?;
        Ok(Some(spec))
    }
    pub(super) fn static_pivot_snapshot(
        &self,
        root: &Path,
        project: u64,
        generation: u64,
    ) -> Result<Option<Snapshot>, String> {
        let Some(spec) = self.static_pivot_request()? else {
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
    pub(super) fn show_static_pivot_controls(&mut self, ui: &mut Ui) {
        if self.source_format != SourceFormat::CookedStatic || self.kind != ModelKind::Static {
            return;
        }
        let before = self.static_pivot.clone();
        if ui
            .checkbox(
                &mut self.static_pivot.enabled,
                "Create static pivot variant",
            )
            .changed()
        {
            self.static_pivot.source_sha256 = None;
            self.inspected_hash = None;
            if self.static_pivot.enabled {
                self.static_uv.enabled = false;
                self.static_sampler.enabled = false;
                self.static_recolor.enabled = false;
                self.static_planar.enabled = false;
                self.static_texture.enabled = false;
                self.static_assembly.enabled = false;
                self.glb_subtree.enabled = false;
                self.color_variant_enabled = false;
                self.texture_variant_enabled = false;
                self.trim.enabled = false;
                self.sampler.enabled = false;
                self.sequence.enabled = false;
                self.uv.enabled = false;
            }
        }
        if self.static_pivot.enabled {
            let hash = self
                .inspection
                .current_catalog(&self.source, self.kind, self.source_format)
                .map(|catalog| catalog.source_sha256.clone());
            if ui
                .add_enabled(
                    !self.busy() && hash.is_some(),
                    egui::Button::new("Use inspected source for pivot"),
                )
                .clicked()
            {
                self.static_pivot.source_sha256 = hash.clone();
                self.inspected_hash = hash;
            }
            ui.label(if self.static_pivot.source_sha256.is_some() {
                "Pivot source chosen from the current static inspection"
            } else {
                "Inspect static source, then Use inspected source for pivot"
            });
            for (axis, label) in ["X", "Y", "Z"].into_iter().enumerate() {
                text_field(
                    ui,
                    &format!("Static pivot {label}"),
                    &mut self.static_pivot.pivot[axis],
                    64,
                );
            }
            text_field(
                ui,
                "Static pivot variant asset ID",
                &mut self.static_pivot.asset_id,
                240,
            );
            ui.weak("Move the chosen pivot to the model origin: each primitive placement becomes T(-pivot) * placement. Values follow CLI f32 conversion and must then be finite within +/-1000000. Import validates transformed placements; geometry and images stay unchanged. Preview uses the existing imported-package preview after import.");
        }
        if before != self.static_pivot {
            self.source_inputs_changed();
        }
        if self.static_pivot.enabled {
            if let Err(error) = self.static_pivot_request() {
                ui.weak(error);
            }
        }
    }
}
#[cfg(test)]
#[path = "static_pivot_ui_tests.rs"]
mod tests;
