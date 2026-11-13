//! Strict optional wire object; missing preserves schema 1, explicit null is invalid.
use orr_render::PostProcessSettings;
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    enabled: bool,
    bloom: bool,
    threshold: f32,
    strength: f32,
    radius: u32,
    iterations: u32,
}

pub(super) fn deserialize_present<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Settings>, D::Error> {
    super::deserialize_object(deserializer).map(Some)
}

impl From<PostProcessSettings> for Settings {
    fn from(value: PostProcessSettings) -> Self {
        Self {
            enabled: value.enabled,
            bloom: value.bloom,
            threshold: value.threshold,
            strength: value.strength,
            radius: value.radius,
            iterations: value.iterations,
        }
    }
}
impl From<Settings> for PostProcessSettings {
    fn from(value: Settings) -> Self {
        Self {
            enabled: value.enabled,
            bloom: value.bloom,
            threshold: value.threshold,
            strength: value.strength,
            radius: value.radius,
            iterations: value.iterations,
            // The mesh lighting profile stores bloom only.
            ..Default::default()
        }
    }
}
