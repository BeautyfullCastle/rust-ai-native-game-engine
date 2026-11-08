//! Explicit model-space planar projection for a current inspected used material.
use super::*;
use crate::model_package_import::StaticPlanarUvVariant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Plane {
    Xy,
    Xz,
    Yz,
}
impl Plane {
    fn name(self) -> &'static str {
        match self {
            Self::Xy => "xy",
            Self::Xz => "xz",
            Self::Yz => "yz",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Xy => "XY model plane",
            Self::Xz => "XZ model plane",
            Self::Yz => "YZ model plane",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Controls {
    pub(super) enabled: bool,
    pub(super) material: Option<u32>,
    pub(super) plane: Plane,
    pub(super) origin: [String; 3],
    pub(super) units_per_repeat: [String; 2],
    pub(super) asset_id: String,
}
impl Default for Controls {
    fn default() -> Self {
        Self {
            enabled: false,
            material: None,
            plane: Plane::Xz,
            origin: ["0".into(), "0".into(), "0".into()],
            units_per_repeat: ["1".into(), "1".into()],
            asset_id: "variants/material.planar-uv.json".into(),
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
    spec: StaticPlanarUvVariant,
}
fn number(raw: &str, positive: bool) -> Result<f64, String> {
    let value = raw
        .trim()
        .parse::<f64>()
        .map_err(|_| "Planar UV values must be finite decimal numbers")?;
    if !value.is_finite()
        || !(value as f32).is_finite()
        || (positive && (value <= 0.0 || value as f32 <= 0.0))
    {
        return Err("Planar origins must convert to finite f32; units per repeat must be finite and positive before and after conversion".into());
    }
    Ok(value)
}
impl Panel {
    pub(super) fn static_planar_request(&self) -> Result<Option<StaticPlanarUvVariant>, String> {
        if !self.static_planar.enabled {
            return Ok(None);
        }
        if self.source_format != SourceFormat::CookedStatic
            || self.kind != ModelKind::Static
            || self.static_uv.enabled
            || self.static_texture.enabled
            || self.static_assembly.enabled
            || self.static_pivot.enabled
            || self.static_sampler.enabled
            || self.static_recolor.enabled
            || self.glb_subtree.enabled
            || self.color_variant_enabled
            || self.texture_variant_enabled
            || self.trim.enabled
            || self.sampler.enabled
            || self.sequence.enabled
            || self.uv.enabled
            || self.pose_bake.enabled
        {
            return Err("Choose only cooked static planar UV mode".into());
        }
        let catalog = self
            .inspection
            .current_catalog(&self.source, self.kind, self.source_format)
            .ok_or("Inspect current static source first")?;
        let material = self
            .static_planar
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
        let spec = StaticPlanarUvVariant {
            material,
            plane: self.static_planar.plane.name().into(),
            origin: [
                number(&self.static_planar.origin[0], false)?,
                number(&self.static_planar.origin[1], false)?,
                number(&self.static_planar.origin[2], false)?,
            ],
            units_per_repeat: [
                number(&self.static_planar.units_per_repeat[0], true)?,
                number(&self.static_planar.units_per_repeat[1], true)?,
            ],
            asset_id: self.static_planar.asset_id.clone(),
        };
        model_package_import::validate_static_planar_uv_variant(&spec)?;
        Ok(Some(spec))
    }
    pub(super) fn static_planar_snapshot(
        &self,
        root: &Path,
        project: u64,
        generation: u64,
    ) -> Result<Option<Snapshot>, String> {
        let Some(spec) = self.static_planar_request()? else {
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
    pub(super) fn show_static_planar_controls(&mut self, ui: &mut Ui) {
        if self.source_format != SourceFormat::CookedStatic || self.kind != ModelKind::Static {
            return;
        }
        let before = self.static_planar.clone();
        if ui
            .checkbox(
                &mut self.static_planar.enabled,
                "Create static planar UV variant",
            )
            .changed()
        {
            self.static_planar.material = None;
            self.inspected_hash = None;
            if self.static_planar.enabled {
                self.static_uv.enabled = false;
                self.static_texture.enabled = false;
                self.static_assembly.enabled = false;
                self.static_pivot.enabled = false;
                self.static_sampler.enabled = false;
                self.static_recolor.enabled = false;
                self.glb_subtree.enabled = false;
                self.color_variant_enabled = false;
                self.texture_variant_enabled = false;
                self.trim.enabled = false;
                self.sampler.enabled = false;
                self.sequence.enabled = false;
                self.uv.enabled = false;
                self.pose_bake.enabled = false;
            }
        }
        if self.static_planar.enabled {
            ui.label(
                self.static_planar
                    .material
                    .map(|material| format!("Planar UV material: {material}"))
                    .unwrap_or_else(|| "Inspect static source and Use material first".into()),
            );
            egui::ComboBox::from_label("Static planar UV plane")
                .selected_text(self.static_planar.plane.label())
                .show_ui(ui, |ui| {
                    for plane in [Plane::Xy, Plane::Xz, Plane::Yz] {
                        ui.selectable_value(&mut self.static_planar.plane, plane, plane.label());
                    }
                });
            for (axis, name) in ["X", "Y", "Z"].into_iter().enumerate() {
                text_field(
                    ui,
                    &format!("Planar origin {name}"),
                    &mut self.static_planar.origin[axis],
                    64,
                );
            }
            for (axis, name) in ["U", "V"].into_iter().enumerate() {
                text_field(
                    ui,
                    &format!("Planar units per repeat {name}"),
                    &mut self.static_planar.units_per_repeat[axis],
                    64,
                );
            }
            text_field(
                ui,
                "Static planar UV asset ID",
                &mut self.static_planar.asset_id,
                240,
            );
            ui.weak("Project UVs from each vertex's transformed model-space position on the chosen plane, relative to the origin and units per repeat. Entity placement is not used. This is planar projection, not automatic unwrapping. Import validates output UV limits; geometry, placements, materials and images stay unchanged. Preview uses the existing imported-package preview after import.");
        }
        if before != self.static_planar {
            self.source_inputs_changed();
        }
        if self.static_planar.enabled {
            if let Err(error) = self.static_planar_request() {
                ui.weak(error);
            }
        }
    }
}
#[cfg(test)]
#[path = "static_planar_ui_tests.rs"]
mod tests;
