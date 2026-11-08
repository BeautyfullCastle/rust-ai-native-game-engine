//! Explicit appending of two compatible inspected clips and a timed bridge.
use super::*;
use crate::model_package_import::SequenceVariant;
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Controls {
    pub(super) enabled: bool,
    pub(super) first: Option<u32>,
    pub(super) second: Option<u32>,
    pub(super) first_hash: Option<String>,
    pub(super) second_hash: Option<String>,
    pub(super) transition: String,
    pub(super) name: String,
    pub(super) asset_id: String,
}
impl Default for Controls {
    fn default() -> Self {
        Self {
            enabled: false,
            first: None,
            second: None,
            first_hash: None,
            second_hash: None,
            transition: "0.25".into(),
            name: "sequenced".into(),
            asset_id: "variants/clip.sequence.json".into(),
        }
    }
}
impl Panel {
    pub(super) fn sequence_request(&self) -> Result<Option<SequenceVariant>, String> {
        if !self.sequence.enabled {
            return Ok(None);
        }
        if !cfg!(feature = "room-animated-models")
            || self.source_format != SourceFormat::CookedAnimated
            || self.kind != ModelKind::Animated
            || self.color_variant_enabled
            || self.texture_variant_enabled
            || self.trim.enabled
            || self.sampler.enabled
            || self.uv.enabled
        {
            return Err("Choose only cooked animated clip sequence mode".into());
        }
        let catalog = self
            .inspection
            .catalog()
            .ok_or("Inspect the current animated source first")?;
        let first = self
            .sequence
            .first
            .ok_or("Select the first inspected clip")?;
        let second = self
            .sequence
            .second
            .ok_or("Select the second inspected clip")?;
        for (index, hash) in [
            (first, &self.sequence.first_hash),
            (second, &self.sequence.second_hash),
        ] {
            if hash.as_ref() != Some(&catalog.source_sha256)
                || self.inspected_hash.as_ref() != Some(&catalog.source_sha256)
                || !catalog.clips.iter().any(|c| c.index == index)
            {
                return Err("Inspect and select both current clips again".into());
            }
        }
        let transition = self
            .sequence
            .transition
            .trim()
            .parse::<f64>()
            .map_err(|_| "Transition seconds must be finite and positive")?;
        if !transition.is_finite()
            || transition <= 0.0
            || transition > 60.0
            || transition as f32 <= 0.0
        {
            return Err("Transition seconds must be finite, positive, and at most 60".into());
        }
        let name = self.sequence.name.trim();
        if name.is_empty()
            || name.len() > 128
            || name.chars().any(char::is_control)
            || catalog.clips.iter().any(|c| c.name == name)
        {
            return Err("Use a new 1..128-byte clip name without control characters".into());
        }
        let id = &self.sequence.asset_id;
        if id != id.trim()
            || !id.ends_with(".sequence.json")
            || id.contains('\\')
            || id.contains(':')
            || id.chars().any(char::is_control)
            || id.split('/').any(|p| p.is_empty() || p == "." || p == "..")
        {
            return Err("Sequence asset ID must be a relative .sequence.json logical path".into());
        }
        Ok(Some(SequenceVariant {
            first_clip: first,
            second_clip: second,
            transition_seconds: transition,
            name: name.into(),
            asset_id: id.clone(),
        }))
    }
    pub(super) fn show_sequence_controls(&mut self, ui: &mut Ui) {
        if !cfg!(feature = "room-animated-models")
            || self.source_format != SourceFormat::CookedAnimated
            || self.kind != ModelKind::Animated
        {
            return;
        }
        let before = self.sequence.clone();
        if ui
            .checkbox(&mut self.sequence.enabled, "Create animation clip sequence")
            .changed()
            && self.sequence.enabled
        {
            self.uv.enabled=false;
            self.color_variant_enabled = false;
            self.texture_variant_enabled = false;
            self.trim.enabled = false;
            self.sampler.enabled = false;
        }
        if self.sequence.enabled {
            let catalog = self.inspection.catalog().cloned();
            for first in [true, false] {
                let label = if first {
                    "Sequence first clip"
                } else {
                    "Sequence second clip"
                };
                let selected = if first {
                    self.sequence.first
                } else {
                    self.sequence.second
                };
                let text = catalog
                    .as_ref()
                    .and_then(|c| c.clips.iter().find(|c| Some(c.index) == selected))
                    .map(|c| format!("{}: {} ({:.3}s)", c.index, c.name, c.duration))
                    .unwrap_or_else(|| "Inspect source and select a clip".into());
                egui::ComboBox::from_label(label)
                    .selected_text(text)
                    .show_ui(ui, |ui| {
                        if let Some(catalog) = &catalog {
                            for clip in &catalog.clips {
                                if ui
                                    .selectable_label(
                                        selected == Some(clip.index),
                                        format!(
                                            "{label} {}: {} ({:.3}s)",
                                            clip.index, clip.name, clip.duration
                                        ),
                                    )
                                    .clicked()
                                {
                                    if first {
                                        self.sequence.first = Some(clip.index);
                                        self.sequence.first_hash =
                                            Some(catalog.source_sha256.clone());
                                    } else {
                                        self.sequence.second = Some(clip.index);
                                        self.sequence.second_hash =
                                            Some(catalog.source_sha256.clone());
                                    }
                                    self.inspected_hash = Some(catalog.source_sha256.clone());
                                }
                            }
                        } else {
                            ui.label("Inspect animated source first.");
                        }
                    });
            }
            text_field(
                ui,
                "Sequence transition seconds",
                &mut self.sequence.transition,
                64,
            );
            text_field(ui, "New sequence clip name", &mut self.sequence.name, 128);
            text_field(
                ui,
                "Sequence variant asset ID",
                &mut self.sequence.asset_id,
                240,
            );
            ui.weak("Append first clip, a timed bridge, then second clip; duration is their sum. Both clips must use LINEAR tracks with identical node/property targets. Originals remain unchanged. Preview validates compatibility without publishing; import requires rights and a new destination.");
        }
        if before != self.sequence {
            self.source_inputs_changed();
        }
        if self.sequence.enabled {
            if let Err(error) = self.sequence_request() {
                ui.weak(error);
            }
        }
    }
}
#[cfg(test)]
#[path = "sequence_ui_tests.rs"]
mod tests;
