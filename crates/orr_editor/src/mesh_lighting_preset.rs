//! Portable lighting values, separate from scene-bound lighting sidecars.
//!
//! Spatial shadow coverage is deliberately absent: importing a preset keeps the
//! destination document's shadow center and radius. All numeric policy remains
//! in `mesh_lighting::Document::validate`.
use crate::mesh_lighting::Document;
use serde::{Deserialize, Deserializer, Serialize};

pub(crate) mod store;

pub(crate) const MAX_BYTES: usize = 16 * 1024;
const SCHEMA: u32 = 1;
const IDENTITY: &str = "ORR_MESH_LIGHTING_PRESET_V1";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Preset {
    direction: [f32; 3],
    color: [f32; 3],
    intensity: f32,
    sky: [f32; 3],
    ground: [f32; 3],
    ambient: f32,
    shadows: bool,
    tonemap: bool,
    exposure: f32,
}

impl Preset {
    pub(crate) fn from_document(document: &Document) -> Result<Self, String> {
        document.validate()?;
        Ok(Self {
            direction: document.direction,
            color: document.color,
            intensity: document.intensity,
            sky: document.sky,
            ground: document.ground,
            ambient: document.ambient,
            shadows: document.shadows,
            tonemap: document.tonemap,
            exposure: document.exposure,
        })
    }

    /// Construct a new draft; this never mutates the target or writes a sidecar.
    pub(crate) fn merge_into(&self, target: &Document) -> Result<Document, String> {
        let merged = self.with_shadow_bounds(target.shadow_center, target.shadow_radius);
        merged.validate()?;
        Ok(merged)
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        #[derive(Serialize)]
        struct Wire<'a> {
            schema: u32,
            identity: &'static str,
            lighting: &'a Preset,
        }
        let mut bytes = serde_json::to_vec_pretty(&Wire {
            schema: SCHEMA,
            identity: IDENTITY,
            lighting: self,
        })
        .map_err(|error| format!("serialize mesh lighting preset: {error}"))?;
        bytes.push(b'\n');
        if bytes.len() > MAX_BYTES {
            return Err(format!("mesh lighting preset exceeds {MAX_BYTES} bytes"));
        }
        Ok(bytes)
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.is_empty() || bytes.len() > MAX_BYTES {
            return Err(format!(
                "mesh lighting preset must be 1..={MAX_BYTES} bytes"
            ));
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            schema: u32,
            identity: String,
            #[serde(deserialize_with = "deserialize_object")]
            lighting: LightingValues,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct LightingValues {
            direction: [f32; 3],
            color: [f32; 3],
            intensity: f32,
            sky: [f32; 3],
            ground: [f32; 3],
            ambient: f32,
            shadows: bool,
            tonemap: bool,
            exposure: f32,
        }
        let mut deserializer = serde_json::Deserializer::from_slice(bytes);
        let wire: Wire = deserialize_object(&mut deserializer)
            .map_err(|error| format!("mesh lighting preset JSON: {error}"))?;
        deserializer
            .end()
            .map_err(|error| format!("mesh lighting preset JSON: {error}"))?;
        if wire.schema != SCHEMA || wire.identity != IDENTITY {
            return Err("unsupported mesh lighting preset schema or identity".into());
        }
        let lighting = wire.lighting;
        let preset = Self {
            direction: lighting.direction,
            color: lighting.color,
            intensity: lighting.intensity,
            sky: lighting.sky,
            ground: lighting.ground,
            ambient: lighting.ambient,
            shadows: lighting.shadows,
            tonemap: lighting.tonemap,
            exposure: lighting.exposure,
        };
        preset.validate()?;
        Ok(preset)
    }

    fn validate(&self) -> Result<(), String> {
        // Fixed valid spatial values let the existing document validator own
        // every reusable field's policy without encoding any scene geometry.
        self.with_shadow_bounds([0.0; 3], 1.0).validate()
    }

    fn with_shadow_bounds(&self, shadow_center: [f32; 3], shadow_radius: f32) -> Document {
        Document {
            direction: self.direction,
            color: self.color,
            intensity: self.intensity,
            sky: self.sky,
            ground: self.ground,
            ambient: self.ambient,
            shadows: self.shadows,
            shadow_center,
            shadow_radius,
            tonemap: self.tonemap,
            exposure: self.exposure,
        }
    }
}

// A derived struct deserializer also accepts positional arrays. Both JSON
// objects must be maps so the portable format has only its documented shape.
fn deserialize_object<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Object<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Object<T> {
        type Value = T;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a mesh lighting preset JSON object")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(self, map: M) -> Result<T, M::Error> {
            T::deserialize(serde::de::value::MapAccessDeserializer::new(map))
        }
    }
    deserializer.deserialize_map(Object(std::marker::PhantomData))
}

#[cfg(test)]
mod tests;
