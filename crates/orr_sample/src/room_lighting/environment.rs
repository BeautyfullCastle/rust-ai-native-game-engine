//! Optional saved directional and hemisphere lighting; no shadow/output policy.
use super::object;
#[cfg(test)]
use super::Document;
use orr_render::Lighting;
use serde::{Deserialize, Deserializer, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Environment {
    pub direction: [f32; 3],
    pub color: [f32; 3],
    pub intensity: f32,
    pub sky: [f32; 3],
    pub ground: [f32; 3],
    pub ambient: f32,
}
impl Default for Environment {
    fn default() -> Self {
        let base = crate::yard3d_view::yard_lighting();
        Self {
            direction: base.direction,
            color: base.color,
            intensity: base.intensity,
            sky: base.sky,
            ground: base.ground,
            ambient: base.ambient,
        }
    }
}
impl<'de> Deserialize<'de> for Environment {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            direction: [f32; 3],
            color: [f32; 3],
            intensity: f32,
            sky: [f32; 3],
            ground: [f32; 3],
            ambient: f32,
        }
        let w: Wire = object(d)?;
        let result = Self {
            direction: w.direction,
            color: w.color,
            intensity: w.intensity,
            sky: w.sky,
            ground: w.ground,
            ambient: w.ambient,
        };
        result.validate().map_err(serde::de::Error::custom)?;
        Ok(result)
    }
}
pub(super) fn deserialize_present<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Environment>, D::Error> {
    Environment::deserialize(d).map(Some)
}
impl Environment {
    pub fn validate(&self) -> Result<(), String> {
        if !self
            .direction
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 1.0)
            || self
                .direction
                .iter()
                .map(|v| f64::from(*v) * f64::from(*v))
                .sum::<f64>()
                <= 1e-24
        {
            return Err("room environment direction must be finite, bounded and nonzero".into());
        }
        if !self
            .color
            .iter()
            .chain(self.sky.iter())
            .chain(self.ground.iter())
            .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
        {
            return Err("room environment colors must be finite and in 0..=1".into());
        }
        if ![self.intensity, self.ambient]
            .iter()
            .all(|v| v.is_finite() && (0.0..=8.0).contains(v))
        {
            return Err(
                "room environment intensity and ambient must be finite and in 0..=8".into(),
            );
        }
        Ok(())
    }
    pub fn apply_to(&self, base: &mut Lighting) -> Result<(), String> {
        self.validate()?;
        base.direction = self.direction;
        base.color = self.color;
        base.intensity = self.intensity;
        base.sky = self.sky;
        base.ground = self.ground;
        base.ambient = self.ambient;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_bytes_and_version_selection_remain_exact() {
        let d = Document::enabled_default();
        // Pinned pre-environment (ef49696) pretty JSON, including field order.
        let mut carried = d.clone();
        carried.follow_player = Some([0.0; 3]);
        assert_eq!(
            carried.to_bytes().unwrap(),
            br#"{
  "version": 2,
  "point_light": {
    "position": [
      0.0,
      4.0,
      0.0
    ],
    "color": [
      1.0,
      0.9,
      0.7
    ],
    "intensity": 3.0,
    "range": 12.0
  },
  "follow_player": [
    0.0,
    0.0,
    0.0
  ]
}"#
        );
        let mut conditional = d.clone();
        conditional.activation = Some(super::super::Activation::Escaped);
        assert_eq!(
            conditional.to_bytes().unwrap(),
            br#"{
  "version": 3,
  "point_light": {
    "position": [
      0.0,
      4.0,
      0.0
    ],
    "color": [
      1.0,
      0.9,
      0.7
    ],
    "intensity": 3.0,
    "range": 12.0
  },
  "activation": "escaped"
}"#
        );
        assert_eq!(d.to_bytes().unwrap(), d.settings().to_bytes().unwrap());
        for bytes in [br#"{"version":1,"point_light":null}"#.as_slice(),br#"{"version":2,"point_light":{"position":[0,4,0],"color":[1,0.9,0.7],"intensity":3,"range":12},"follow_player":[0,0,0]}"#.as_slice(),br#"{"version":3,"point_light":{"position":[0,4,0],"color":[1,0.9,0.7],"intensity":3,"range":12},"activation":"escaped"}"#.as_slice()] {let doc=Document::parse(bytes).unwrap();assert!(doc.environment.is_none());let canonical=doc.to_bytes().unwrap();assert_eq!(Document::parse(&canonical).unwrap().to_bytes().unwrap(),canonical);}
    }
    #[test]
    fn environment_independent_of_point_follow_and_activation_roundtrips() {
        for mut d in [Document::default(), Document::enabled_default()] {
            d.environment = Some(Environment::default());
            let bytes = d.to_bytes().unwrap();
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["version"],
                4
            );
            assert_eq!(Document::parse(&bytes).unwrap(), d);
            if d.point_light.is_some() {
                d.follow_player = Some([1.0, 2.0, 3.0]);
                d.activation = Some(super::super::Activation::Escaped);
                assert_eq!(Document::parse(&d.to_bytes().unwrap()).unwrap(), d);
            }
        }
    }
    #[test]
    fn application_preserves_shadow_output_and_invalid_application_is_atomic() {
        let mut base = Lighting::default();
        base.shadows = false;
        base.exposure = 3.0;
        base.tonemap = false;
        let original = base;
        Document::default().apply_environment(&mut base).unwrap();
        assert_eq!(base, original);
        let mut env = Environment::default();
        env.color = [0.2, 0.3, 0.4];
        env.apply_to(&mut base).unwrap();
        assert_eq!(base.color, env.color);
        assert_eq!(
            (
                base.shadows,
                base.shadow_center,
                base.shadow_radius,
                base.exposure,
                base.tonemap
            ),
            (
                original.shadows,
                original.shadow_center,
                original.shadow_radius,
                original.exposure,
                original.tonemap
            )
        );
        let before = base;
        env.direction = [0.0; 3];
        assert!(env.apply_to(&mut base).is_err());
        assert_eq!(base, before);
    }
    #[test]
    fn strict_wire_rejects_null_arrays_unknown_duplicates_and_schema_mismatch() {
        let env = serde_json::to_string(&Environment::default()).unwrap();
        for value in [
            "null".to_owned(),
            "[]".into(),
            "{}".into(),
            env.replace(r#""ambient":"#, r#""unknown":0,"ambient":"#),
            env.replace(r#""ambient":"#, r#""ambient":1,"ambient":"#),
        ] {
            let bytes = format!("{{\"version\":4,\"point_light\":null,\"environment\":{value}}}");
            assert!(Document::parse(bytes.as_bytes()).is_err());
        }
        for version in [1, 2, 3] {
            assert!(Document::parse(
                format!("{{\"version\":{version},\"point_light\":null,\"environment\":{env}}}")
                    .as_bytes()
            )
            .is_err());
        }
        assert!(Document::parse(br#"{"version":4,"point_light":null}"#).is_err());
    }
    #[test]
    fn bounded_direction_and_radiance_reject_nonfinite_and_outside_values() {
        for v in [f32::NAN, f32::INFINITY, -1.0, 8.1] {
            let mut e = Environment::default();
            e.intensity = v;
            assert!(e.validate().is_err());
            e = Environment::default();
            e.sky[0] = if v == 8.1 { 1.01 } else { v };
            assert!(e.validate().is_err());
        }
        for dir in [
            [0.0; 3],
            [1e-13, 0.0, 0.0],
            [1.01, 0.0, 0.0],
            [f32::NAN, 0.0, 0.0],
        ] {
            let mut e = Environment::default();
            e.direction = dir;
            assert!(e.validate().is_err());
        }
        let mut e = Environment::default();
        e.direction = [1.0, 0.0, 0.0];
        e.intensity = 8.0;
        e.ambient = 0.0;
        e.validate().unwrap();
    }
}
