//! Bounded presentation-only Room lighting. Simulation state never contains it.
use orr_render::{PointLight, PointLightSettings};
use serde::{Deserialize, Deserializer, Serialize};

pub const MAX_BYTES: usize = orr_render::point_light::MAX_POINT_LIGHT_SETTINGS_BYTES;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document {
    pub point_light: Option<PointLight>,
    pub follow_player: Option<[f32;3]>,
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
fn present_offset<'de,D:Deserializer<'de>>(d:D)->Result<Option<[f32;3]>,D::Error> { <[f32;3]>::deserialize(d).map(Some) }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    version: u32,
    #[serde(deserialize_with = "required_light")]
    point_light: Option<Light>,
    #[serde(default,deserialize_with="present_offset")]
    follow_player:Option<[f32;3]>,
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
            follow_player:None,
        }
    }
    pub fn settings(&self) -> PointLightSettings {
        PointLightSettings {
            point_light: self.point_light,
        }
    }
    /// Same immutable Room Frame as rendering, with registered component types.
    /// Edit uses settings() and the authored fallback position instead.
    pub fn settings_for_frame(&self,frame:orr_bridge::FrameView<'_>)->Result<PointLightSettings,String> {
        self.validate()?;
        let Some(offset)=self.follow_player else {return Ok(self.settings());};
        use crate::room_game::{RoomActor,PLAYER};
        let mut players=frame.iter::<RoomActor>().filter(|(_,actor)|actor.kind==PLAYER);
        let (entity,_)=players.next().ok_or("carried light requires one current PLAYER")?;
        if players.next().is_some() {return Err("carried light requires exactly one PLAYER".into());}
        let body=frame.get::<orr_physics3d::Body>(entity).ok_or("carried light PLAYER has no current body generation")?;
        let position=[body.pos.x.to_f32(),body.pos.y.to_f32(),body.pos.z.to_f32()];
        if !position.iter().all(|v|v.is_finite() && v.abs()<=32.0) {return Err("carried light PLAYER position exceeds ±32".into());}
        let mut settings=self.settings();
        settings.point_light.as_mut().expect("validated light").position=std::array::from_fn(|axis|(position[axis] as f64+offset[axis] as f64) as f32);
        settings.validate().map_err(|e|e.to_string())?;
        Ok(settings)
    }
    pub fn validate(&self) -> Result<(), String> {
        if let Some(offset)=self.follow_player {
            if self.point_light.is_none() || !offset.iter().all(|v|v.is_finite() && v.abs()<=32.0) {
                return Err("carried room light requires enabled light and finite follow offset within ±32".into());
            }
        }
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
        match (wire.version,wire.follow_player) {
            (1,None)|(2,Some(_))=>{},
            _=>return Err("room lighting version1 is fixed; version2 requires follow_player offset".into()),
        }
        let document = Self {
            point_light: wire.point_light.map(|light| light.0),
            follow_player: wire.follow_player,
        };
        document.validate()?;
        Ok(document)
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        if self.follow_player.is_none() { return self.settings().to_bytes().map_err(|e|format!("room lighting: {e}")); }
        #[derive(Serialize)]
        struct Output { version:u32,point_light:Option<PointLight>,follow_player:[f32;3] }
        let bytes=serde_json::to_vec_pretty(&Output {version:2,point_light:self.point_light,follow_player:self.follow_player.expect("validated carried light")}).map_err(|e|e.to_string())?;
        if bytes.len()>MAX_BYTES {return Err("room lighting exceeds 4096 bytes".into());}
        Ok(bytes)
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

#[cfg(test)]
#[path="room_lighting/carried_tests.rs"]
mod carried_tests;
