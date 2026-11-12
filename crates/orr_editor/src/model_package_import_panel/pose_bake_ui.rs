//! Bake an explicitly selected animated clip sample into a static prop package.
use super::*;
use crate::model_package_import::PoseBakeVariant;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Controls {
    pub(super) enabled: bool,
    pub(super) clip: Option<u32>,
    pub(super) time_seconds: String,
    pub(super) asset_id: String,
}
impl Default for Controls {
    fn default() -> Self {
        Self {
            enabled: false,
            clip: None,
            time_seconds: "0".into(),
            asset_id: "variants/prop.pose.json".into(),
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
    spec: PoseBakeVariant,
}
fn number(raw: &str, duration: f64) -> Result<f64, String> {
    let value = raw
        .trim()
        .parse::<f64>()
        .map_err(|_| "Pose sample time must be finite decimal seconds")?;
    if !duration.is_finite()
        || duration < 0.0
        || !value.is_finite()
        || value < 0.0
        || value > duration
        || !(value as f32).is_finite()
        || (value > 0.0 && value as f32 == 0.0)
    {
        return Err("Pose time must satisfy 0 <= time <= selected clip duration before f32 conversion; positive times must survive conversion".into());
    }
    Ok(value)
}
impl Panel {
    pub(super) fn pose_bake_request(&self) -> Result<Option<PoseBakeVariant>, String> {
        if !self.pose_bake.enabled {
            return Ok(None);
        }
        if !cfg!(feature = "room-animated-models")
            || self.source_format != SourceFormat::CookedAnimated
            || self.kind != ModelKind::Animated
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
        {
            return Err("Choose only Cooked animated source and Animated mode with room-animated-models for a static pose bake".into());
        }
        let catalog = self
            .inspection
            .current_catalog(&self.source, self.kind, self.source_format)
            .ok_or("Inspect current animated source first")?;
        let clip = self
            .pose_bake
            .clip
            .ok_or("Inspect animated source and choose a clip index explicitly")?;
        let entry = catalog
            .clips
            .iter()
            .find(|entry| entry.index == clip)
            .ok_or("Select a current inspected clip index")?;
        if self.inspected_hash.as_ref() != Some(&catalog.source_sha256) {
            return Err("Inspect and choose a current source clip again".into());
        }
        let spec = PoseBakeVariant {
            clip,
            time_seconds: number(&self.pose_bake.time_seconds, f64::from(entry.duration))?,
            asset_id: self.pose_bake.asset_id.clone(),
        };
        model_package_import::validate_pose_bake_variant(&spec)?;
        Ok(Some(spec))
    }
    pub(super) fn pose_bake_snapshot(
        &self,
        root: &Path,
        project: u64,
        generation: u64,
    ) -> Result<Option<Snapshot>, String> {
        let Some(spec) = self.pose_bake_request()? else {
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
                .ok_or("Inspect current animated source first")?,
            spec,
        }))
    }
    pub(super) fn show_pose_bake_controls(&mut self, ui: &mut Ui) {
        if !cfg!(feature = "room-animated-models")
            || self.source_format != SourceFormat::CookedAnimated
            || self.kind != ModelKind::Animated
        {
            return;
        }
        let before = self.pose_bake.clone();
        if ui
            .checkbox(
                &mut self.pose_bake.enabled,
                "Bake static prop from animation",
            )
            .changed()
        {
            self.pose_bake.clip = None;
            self.inspected_hash = None;
            if self.pose_bake.enabled {
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
            }
        }
        if self.pose_bake.enabled {
            ui.label(
                self.pose_bake
                    .clip
                    .map(|clip| format!("Pose source clip index: {clip}"))
                    .unwrap_or_else(|| "Inspect animated source, then Use clip by index".into()),
            );
            text_field(
                ui,
                "Pose sample time (seconds)",
                &mut self.pose_bake.time_seconds,
                64,
            );
            text_field(
                ui,
                "Static pose asset ID",
                &mut self.pose_bake.asset_id,
                240,
            );
            ui.weak("Output: a static prop, not an animated variant. Choose an inspected clip index; names are advisory. Sample once between zero and the exact final time without looping. Import bakes the pose into model.orrmodel; preview uses the existing imported static package preview afterward.");
        }
        if before != self.pose_bake {
            self.source_inputs_changed();
        }
        if self.pose_bake.enabled {
            if let Err(error) = self.pose_bake_request() {
                ui.weak(error);
            }
        }
    }
}
#[cfg(test)]
#[path = "pose_bake_ui_tests.rs"]
mod tests;
