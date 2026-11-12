//! Presentation-only heading from coherent observed displacement.
#![allow(clippy::float_arithmetic)]
use orr_fp::FPVec3;
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ForwardAxis {
    #[serde(rename = "positive-z")]
    PositiveZ,
    #[serde(rename = "negative-z")]
    NegativeZ,
    #[serde(rename = "positive-x")]
    PositiveX,
    #[serde(rename = "negative-x")]
    NegativeX,
}
impl<'de> Deserialize<'de> for ForwardAxis {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match String::deserialize(d)?.as_str() {
            "positive-z" => Ok(Self::PositiveZ),
            "negative-z" => Ok(Self::NegativeZ),
            "positive-x" => Ok(Self::PositiveX),
            "negative-x" => Ok(Self::NegativeX),
            _ => Err(serde::de::Error::custom(
                "facing must be positive-z, negative-z, positive-x or negative-x",
            )),
        }
    }
}
pub(super) fn present<'de, D: Deserializer<'de>>(d: D) -> Result<Option<ForwardAxis>, D::Error> {
    ForwardAxis::deserialize(d).map(Some)
}
pub(super) fn advance(axis: ForwardAxis, old: Option<(FPVec3, f32)>, position: FPVec3) -> f32 {
    let Some((old, yaw)) = old else {
        return 0.0;
    };
    // Subtract raw fixed-point values in i128: opposite i64 extremes are valid
    // observations and must not overflow before conversion to presentation math.
    let dx = i128::from(position.x.raw()) - i128::from(old.x.raw());
    let dz = i128::from(position.z.raw()) - i128::from(old.z.raw());
    if dx == 0 && dz == 0 {
        return yaw;
    }
    let offset = match axis {
        ForwardAxis::PositiveZ => 0.0,
        ForwardAxis::NegativeZ => std::f64::consts::PI,
        ForwardAxis::PositiveX => std::f64::consts::FRAC_PI_2,
        ForwardAxis::NegativeX => -std::f64::consts::FRAC_PI_2,
    };
    let angle = (dx as f64).atan2(dz as f64) - offset;
    angle.sin().atan2(angle.cos()) as f32
}
