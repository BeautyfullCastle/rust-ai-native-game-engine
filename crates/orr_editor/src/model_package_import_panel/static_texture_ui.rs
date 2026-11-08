//! Cooked static PNG skins use the inspected source catalog before import.
use super::*;
use crate::model_package_import::StaticTextureVariant;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Controls {
    pub(super) enabled: bool,
    pub(super) material: Option<u32>,
    pub(super) texture: PathBuf,
    pub(super) asset_id: String,
}
impl Default for Controls {
    fn default() -> Self {
        Self {
            enabled: false,
            material: None,
            texture: PathBuf::new(),
            asset_id: "variants/skin.retexture.json".into(),
        }
    }
}

/// All static skin inputs accompany the immutable worker request. PNG bytes
/// are validated and decoded by the producer after checking the source hash.
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
    spec: StaticTextureVariant,
}

impl Panel {
    pub(super) fn static_texture_request(&self) -> Result<Option<StaticTextureVariant>, String> {
        if !self.static_texture.enabled {
            return Ok(None);
        }
        if self.source_format != SourceFormat::CookedStatic
            || self.kind != ModelKind::Static
            || self.static_uv.enabled
            || self.static_assembly.enabled
            || self.color_variant_enabled
            || self.texture_variant_enabled
            || self.trim.enabled
            || self.sampler.enabled
            || self.sequence.enabled
            || self.uv.enabled
        {
            return Err("Choose only cooked static PNG skin mode".into());
        }
        let material = self
            .static_texture
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
        let spec = StaticTextureVariant {
            material,
            texture: self.static_texture.texture.clone(),
            asset_id: self.static_texture.asset_id.clone(),
        };
        model_package_import::validate_static_texture_variant(&spec)?;
        Ok(Some(spec))
    }

    pub(super) fn static_texture_snapshot(
        &self,
        root: &Path,
        project: u64,
        generation: u64,
    ) -> Result<Option<Snapshot>, String> {
        let Some(spec) = self.static_texture_request()? else {
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

    pub(super) fn show_static_texture_controls(&mut self, ui: &mut Ui) {
        if self.source_format != SourceFormat::CookedStatic || self.kind != ModelKind::Static {
            return;
        }
        let before = self.static_texture.clone();
        if ui
            .checkbox(&mut self.static_texture.enabled, "Create static PNG skin")
            .changed()
            && self.static_texture.enabled
        {
            self.static_uv.enabled = false;
            self.static_assembly.enabled = false;
            self.color_variant_enabled = false;
            self.texture_variant_enabled = false;
            self.trim.enabled = false;
            self.sampler.enabled = false;
            self.sequence.enabled = false;
            self.uv.enabled = false;
        }
        if self.static_texture.enabled {
            ui.label(
                self.static_texture
                    .material
                    .map(|m| format!("Static skin material: {m}"))
                    .unwrap_or_else(|| "Inspect static source and Use material first".into()),
            );
            let mut texture = self.static_texture.texture.to_string_lossy().into_owned();
            path_field(ui, "Static variant PNG file", &mut texture);
            self.static_texture.texture = texture.into();
            text_field(
                ui,
                "Static texture variant asset ID",
                &mut self.static_texture.asset_id,
                240,
            );
            ui.weak("Static catalog and settings validation before Import: replace only the selected material's PNG image. Use a non-animated 8-bit RGBA PNG; Import validates and decodes it. The rights acknowledgement covers the model, PNG and license. Model preview uses the existing imported-package preview after import.");
        }
        if before != self.static_texture {
            self.source_inputs_changed();
        }
        if self.static_texture.enabled {
            if let Err(error) = self.static_texture_request() {
                ui.weak(error);
            }
        }
    }
}

#[cfg(test)]
#[path = "static_texture_ui_tests.rs"]
mod tests;
