//! Explicit appended clip authoring; raw numeric text is validated before workers.
use super::*;
use crate::model_package_import::TrimVariant;
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Controls {
    pub(super) enabled: bool,
    pub(super) clip: Option<u32>,
    pub(super) duration: Option<f64>,
    pub(super) source_hash: Option<String>,
    pub(super) start: String,
    pub(super) end: String,
    pub(super) name: String,
    pub(super) asset_id: String,
}
impl Default for Controls {
    fn default() -> Self {
        Self {
            enabled: false,
            clip: None,
            duration: None,
            source_hash: None,
            start: "0".into(),
            end: String::new(),
            name: "trimmed".into(),
            asset_id: "variants/clip.trim.json".into(),
        }
    }
}
impl Panel {
    pub(super) fn trim_request(&self) -> Result<Option<TrimVariant>, String> {
        if !self.trim.enabled {
            return Ok(None);
        }
        if !cfg!(feature = "room-animated-models")
            || self.source_format != SourceFormat::CookedAnimated
            || self.kind != ModelKind::Animated
            || self.color_variant_enabled
            || self.texture_variant_enabled
            || self.sampler.enabled
            || self.sequence.enabled
            || self.uv.enabled
        {
            return Err("Choose only cooked animated clip trim mode".into());
        }
        let clip = self
            .trim
            .clip
            .ok_or("Inspect the animated source and choose a trim source clip")?;
        if self.trim.source_hash.is_none() || self.trim.source_hash != self.inspected_hash {
            return Err("Inspect and select the current source clip again".into());
        }
        let duration = self
            .trim
            .duration
            .ok_or("Selected source clip duration is unavailable")?;
        let start = self
            .trim
            .start
            .trim()
            .parse::<f64>()
            .map_err(|_| "Trim start seconds must be a finite number")?;
        let end = self
            .trim
            .end
            .trim()
            .parse::<f64>()
            .map_err(|_| "Trim end seconds must be a finite number")?;
        if !start.is_finite()
            || !end.is_finite()
            || start < 0.0
            || start >= end
            || end > duration
            || end > 3600.0
            || start as f32 >= end as f32
        {
            return Err("Trim range must satisfy 0 <= start < end <= selected clip duration with distinct sample times".into());
        }
        let name = self.trim.name.trim();
        if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
            return Err(
                "Trim clip name must contain 1..128 UTF-8 bytes without control characters".into(),
            );
        }
        if self
            .inspection
            .catalog()
            .is_some_and(|catalog| catalog.clips.iter().any(|clip| clip.name == name))
        {
            return Err("New trim clip name must not duplicate an existing clip".into());
        }
        let id = &self.trim.asset_id;
        if id != id.trim()
            || !id.ends_with(".trim.json")
            || id.contains('\\')
            || id.contains(':')
            || id
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
            || id.chars().any(char::is_control)
        {
            return Err("Trim asset ID must be a relative .trim.json logical path".into());
        }
        Ok(Some(TrimVariant {
            clip,
            start_seconds: start,
            end_seconds: end,
            name: name.into(),
            asset_id: id.clone(),
        }))
    }
    pub(super) fn show_trim_controls(&mut self, ui: &mut Ui) {
        if !cfg!(feature = "room-animated-models")
            || self.source_format != SourceFormat::CookedAnimated
            || self.kind != ModelKind::Animated
        {
            return;
        }
        let before = self.trim.clone();
        if ui
            .checkbox(&mut self.trim.enabled, "Create trimmed animation clip")
            .changed()
            && self.trim.enabled
        {
            self.sampler.enabled = false;
            self.sequence.enabled=false;
            self.uv.enabled=false;
            self.color_variant_enabled = false;
            self.texture_variant_enabled = false;
        }
        if self.trim.enabled {
            let catalog = self.inspection.catalog().cloned();
            let selected = catalog
                .as_ref()
                .and_then(|catalog| {
                    catalog
                        .clips
                        .iter()
                        .find(|clip| Some(clip.index) == self.trim.clip)
                })
                .map(|clip| {
                    format!(
                        "{}: {} ({:.3}s)",
                        clip.index,
                        if clip.name.trim().is_empty() {
                            "Unnamed clip"
                        } else {
                            &clip.name
                        },
                        clip.duration
                    )
                })
                .unwrap_or_else(|| "Inspect source and select a clip".into());
            egui::ComboBox::from_label("Trim source clip")
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    if let Some(catalog) = catalog.as_ref() {
                        for clip in &catalog.clips {
                            let name = if clip.name.trim().is_empty() {
                                "Unnamed clip"
                            } else {
                                &clip.name
                            };
                            if ui
                                .selectable_label(
                                    self.trim.clip == Some(clip.index),
                                    format!(
                                        "Trim source clip {}: {} ({:.3}s)",
                                        clip.index, name, clip.duration
                                    ),
                                )
                                .clicked()
                            {
                                self.trim.clip = Some(clip.index);
                                self.trim.duration = Some(f64::from(clip.duration));
                                self.trim.source_hash = Some(catalog.source_sha256.clone());
                                self.inspected_hash = Some(catalog.source_sha256.clone());
                                self.trim.start = "0".into();
                                self.trim.end = clip.duration.to_string();
                            }
                        }
                    } else {
                        ui.label("Inspect animated source first.");
                    }
                });
            text_field(ui, "Trim start seconds", &mut self.trim.start, 64);
            text_field(ui, "Trim end seconds", &mut self.trim.end, 64);
            text_field(ui, "New animation clip name", &mut self.trim.name, 128);
            text_field(ui, "Trim variant asset ID", &mut self.trim.asset_id, 240);
            ui.weak("Append one new clip from the selected time interval. Original clips, rig and materials remain intact. Preview is read-only; import requires the rights acknowledgement and a new package destination.");
        }
        if before != self.trim {
            self.source_inputs_changed();
        }
        if self.trim.enabled {
            if let Err(error) = self.trim_request() {
                ui.weak(error);
            }
        }
    }
}
#[cfg(test)]
#[path = "trim_ui_tests.rs"]
mod tests;
