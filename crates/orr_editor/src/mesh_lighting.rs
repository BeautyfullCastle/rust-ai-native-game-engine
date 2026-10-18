use orr_render::Lighting;
use serde::{Deserialize, Deserializer, Serialize};

pub(crate) const MAX_BYTES: usize = 16 * 1024;
const IDENTITY: &str = "ORR_MESH_LIGHTING_V1";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Document {
    pub(crate) direction: [f32; 3],
    pub(crate) color: [f32; 3],
    pub(crate) intensity: f32,
    pub(crate) sky: [f32; 3],
    pub(crate) ground: [f32; 3],
    pub(crate) ambient: f32,
    pub(crate) shadows: bool,
    pub(crate) shadow_center: [f32; 3],
    pub(crate) shadow_radius: f32,
    pub(crate) tonemap: bool,
    pub(crate) exposure: f32,
}

fn deserialize_object<'de, DeserializerType, ValueType>(
    deserializer: DeserializerType,
) -> Result<ValueType, DeserializerType::Error>
where
    DeserializerType: Deserializer<'de>,
    ValueType: Deserialize<'de>,
{
    struct Object<ValueType>(std::marker::PhantomData<ValueType>);

    impl<'de, ValueType: Deserialize<'de>> serde::de::Visitor<'de> for Object<ValueType> {
        type Value = ValueType;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a mesh lighting JSON object")
        }

        fn visit_map<MapAccessType: serde::de::MapAccess<'de>>(
            self,
            map: MapAccessType,
        ) -> Result<ValueType, MapAccessType::Error> {
            ValueType::deserialize(serde::de::value::MapAccessDeserializer::new(map))
        }
    }

    deserializer.deserialize_map(Object(std::marker::PhantomData))
}

impl<'de> Deserialize<'de> for Document {
    fn deserialize<DeserializerType: Deserializer<'de>>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            direction: [f32; 3],
            color: [f32; 3],
            intensity: f32,
            sky: [f32; 3],
            ground: [f32; 3],
            ambient: f32,
            shadows: bool,
            shadow_center: [f32; 3],
            shadow_radius: f32,
            tonemap: bool,
            exposure: f32,
        }

        let wire: Wire = deserialize_object(deserializer)?;
        Ok(Self {
            direction: wire.direction,
            color: wire.color,
            intensity: wire.intensity,
            sky: wire.sky,
            ground: wire.ground,
            ambient: wire.ambient,
            shadows: wire.shadows,
            shadow_center: wire.shadow_center,
            shadow_radius: wire.shadow_radius,
            tonemap: wire.tonemap,
            exposure: wire.exposure,
        })
    }
}

#[derive(Serialize)]
struct BoundDocument<'a> {
    schema: u32,
    identity: &'static str,
    scene_filename: &'a str,
    graph_revision: [u8; 32],
    lighting: &'a Document,
}

impl Document {
    pub(crate) fn from_lighting(lighting: Lighting) -> Result<Self, String> {
        let document = Self {
            direction: lighting.direction,
            color: lighting.color,
            intensity: lighting.intensity,
            sky: lighting.sky,
            ground: lighting.ground,
            ambient: lighting.ambient,
            shadows: lighting.shadows,
            shadow_center: lighting.shadow_center,
            shadow_radius: lighting.shadow_radius,
            tonemap: lighting.tonemap,
            exposure: lighting.exposure,
        };
        document.validate()?;
        Ok(document)
    }

    pub(crate) fn lighting(&self) -> Result<Lighting, String> {
        self.validate()?;
        Ok(Lighting {
            direction: self.direction,
            color: self.color,
            intensity: self.intensity,
            sky: self.sky,
            ground: self.ground,
            ambient: self.ambient,
            shadows: self.shadows,
            shadow_center: self.shadow_center,
            shadow_radius: self.shadow_radius,
            tonemap: self.tonemap,
            exposure: self.exposure,
        })
    }

    pub(crate) fn encode(
        &self,
        scene_filename: &str,
        graph_revision: [u8; 32],
    ) -> Result<Vec<u8>, String> {
        self.validate()?;
        validate_scene_filename(scene_filename)?;
        let mut bytes = serde_json::to_vec_pretty(&BoundDocument {
            schema: 1,
            identity: IDENTITY,
            scene_filename,
            graph_revision,
            lighting: self,
        })
        .map_err(|error| format!("serialize mesh lighting: {error}"))?;
        bytes.push(b'\n');
        if bytes.len() > MAX_BYTES {
            return Err(format!("mesh lighting exceeds {MAX_BYTES} bytes"));
        }
        Ok(bytes)
    }

    pub(crate) fn decode(
        bytes: &[u8],
        scene_filename: &str,
        graph_revision: [u8; 32],
    ) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err(format!("mesh lighting exceeds {MAX_BYTES} bytes"));
        }
        validate_scene_filename(scene_filename)?;
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            schema: u32,
            identity: String,
            scene_filename: String,
            graph_revision: [u8; 32],
            lighting: Document,
        }

        let mut deserializer = serde_json::Deserializer::from_slice(bytes);
        let bound: Wire = deserialize_object(&mut deserializer)
            .map_err(|error| format!("mesh lighting JSON: {error}"))?;
        deserializer
            .end()
            .map_err(|error| format!("mesh lighting JSON: {error}"))?;
        if bound.schema != 1
            || bound.identity != IDENTITY
            || bound.scene_filename != scene_filename
            || bound.graph_revision != graph_revision
        {
            return Err("mesh lighting belongs to another scene or graph revision".into());
        }
        bound.lighting.validate()?;
        Ok(bound.lighting)
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if !self
            .direction
            .iter()
            .all(|value| value.is_finite() && value.abs() <= 1_000_000.0)
            || self
                .direction
                .iter()
                .map(|value| f64::from(*value) * f64::from(*value))
                .sum::<f64>()
                <= 1e-24
        {
            return Err("mesh lighting direction must be finite, bounded and nonzero".into());
        }
        validate_color("color", self.color)?;
        validate_color("sky", self.sky)?;
        validate_color("ground", self.ground)?;
        validate_scalar("intensity", self.intensity, 0.0, 10_000.0)?;
        validate_scalar("ambient", self.ambient, 0.0, 10_000.0)?;
        if self.exposure <= 0.0 {
            return Err("mesh lighting exposure must be greater than zero".into());
        }
        validate_scalar("exposure", self.exposure, 0.0, 10_000.0)?;
        for value in self.shadow_center {
            validate_scalar("shadow_center", value, -1_000_000.0, 1_000_000.0)?;
        }
        validate_scalar("shadow_radius", self.shadow_radius, 0.5, 100_000.0)
    }
}

fn validate_color(field: &str, values: [f32; 3]) -> Result<(), String> {
    for value in values {
        validate_scalar(field, value, 0.0, 10_000.0)?;
    }
    Ok(())
}

fn validate_scalar(field: &str, value: f32, minimum: f32, maximum: f32) -> Result<(), String> {
    if value.is_finite() && (minimum..=maximum).contains(&value) {
        Ok(())
    } else {
        Err(format!(
            "mesh lighting {field} must be finite and in {minimum}..={maximum}"
        ))
    }
}

fn validate_scene_filename(filename: &str) -> Result<(), String> {
    if filename.is_empty()
        || filename.len() > 255
        || filename == "."
        || filename == ".."
        || filename
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\' | ':'))
    {
        return Err("mesh lighting requires a bounded scene filename".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "mesh_lighting/tests.rs"]
mod tests;
