//! Bounded presentation-only Room lighting. Simulation state never contains it.
use orr_render::{PointLight, PointLightSettings};
use serde::{Deserialize, Deserializer};

pub const MAX_BYTES: usize = orr_render::point_light::MAX_POINT_LIGHT_SETTINGS_BYTES;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document {
    pub point_light: Option<PointLight>,
}

// Derived struct decoding accepts positional sequences. Require maps explicitly
// at both object boundaries while preserving duplicate and unknown-field errors.
fn object<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Object<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Object<T> {
        type Value = T;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("lighting object")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
            T::deserialize(serde::de::value::MapAccessDeserializer::new(map))
        }
    }
    deserializer.deserialize_map(Object(std::marker::PhantomData))
}
struct Light(PointLight);
impl<'de> Deserialize<'de> for Light {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        object(deserializer).map(Self)
    }
}
fn required_light<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Light>, D::Error> {
    Option::<Light>::deserialize(deserializer)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    version: u32,
    #[serde(deserialize_with = "required_light")]
    point_light: Option<Light>,
}
impl Document {
    pub fn enabled_default() -> Self {
        Self {
            point_light: Some(PointLight {
                position: [0.0, 4.0, 0.0],
                color: [1.0, 0.9, 0.7],
                intensity: 3.0,
                range: 12.0,
            }),
        }
    }
    pub fn settings(&self) -> PointLightSettings {
        PointLightSettings {
            point_light: self.point_light,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        self.settings()
            .validate()
            .map_err(|e| format!("room lighting: {e}"))
    }
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err("room lighting exceeds 4096 bytes".into());
        }
        let mut decoder = serde_json::Deserializer::from_slice(bytes);
        let wire: Wire = object(&mut decoder).map_err(|e| format!("room lighting: {e}"))?;
        decoder.end().map_err(|e| format!("room lighting: {e}"))?;
        if wire.version != orr_render::point_light::POINT_LIGHT_SETTINGS_VERSION {
            return Err(format!(
                "unsupported room lighting version {}",
                wire.version
            ));
        }
        let document = Self {
            point_light: wire.point_light.map(|light| light.0),
        };
        document.validate()?;
        Ok(document)
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        self.settings()
            .to_bytes()
            .map_err(|e| format!("room lighting: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_bounded_renderer_compatible_document() {
        for doc in [Document::default(), Document::enabled_default()] {
            let bytes = doc.to_bytes().unwrap();
            assert_eq!(Document::parse(&bytes).unwrap(), doc);
            assert_eq!(
                PointLightSettings::from_bytes(&bytes).unwrap(),
                doc.settings()
            );
        }
        for bad in [
            r#"[]"#,
            r#"[1,null]"#,
            r#"null"#,
            r#"{}"#,
            r#"{"version":1}"#,
            r#"{"version":2,"point_light":null}"#,
            r#"{"version":1,"version":1,"point_light":null}"#,
            r#"{"version":1,"point_light":null,"point_light":null}"#,
            r#"{"version":1,"point_light":null,"shadow":true}"#,
            r#"{"version":1,"point_light":[[0,4,0],[1,1,1],3,12]}"#,
            r#"{"version":1,"point_light":{"position":[0,4,0],"color":[1,1,1],"intensity":3,"range":0}}"#,
            r#"{"version":1,"point_light":{"position":[0,4,0],"color":[1,1,1],"intensity":3,"range":12,"range":12}}"#,
            r#"{"version":1,"point_light":{"position":[0,4,0],"color":[1,1,1],"intensity":3,"range":12,"shadow":false}}"#,
            r#"{"version":1,"point_light":null} {}"#,
        ] {
            assert!(Document::parse(bad.as_bytes()).is_err(), "{bad}");
        }
        assert!(Document::parse(&vec![b' '; MAX_BYTES + 1]).is_err());
        for value in [f32::NAN, f32::INFINITY, -1.0, 1e5] {
            let mut doc = Document::enabled_default();
            doc.point_light.as_mut().unwrap().intensity = value;
            assert!(doc.to_bytes().is_err());
        }
    }
}
