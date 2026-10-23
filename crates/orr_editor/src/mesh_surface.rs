use orr_model::StaticModel;
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Serialize};
use std::fmt;

const MAX_OFFSET: f32 = 1_000_000.0;
const MAX_UV: f32 = 65_536.0;
const MIN_UV_SCALE: f32 = 0.001;
const MAX_UV_SCALE: f32 = 1_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(crate) struct Settings {
    pub(crate) material: u32,
    pub(crate) uv_offset: [f32; 2],
    pub(crate) uv_scale: [f32; 2],
    pub(crate) uv_quarter_turns: u32,
    pub(crate) pivot: [f32; 3],
}

impl<'de> Deserialize<'de> for Settings {
    fn deserialize<Deserializer>(deserializer: Deserializer) -> Result<Self, Deserializer::Error>
    where
        Deserializer: serde::Deserializer<'de>,
    {
        struct SettingsVisitor;

        impl<'de> Visitor<'de> for SettingsVisitor {
            type Value = Settings;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a surface settings object with five required fields")
            }

            fn visit_map<Access>(self, mut map: Access) -> Result<Self::Value, Access::Error>
            where
                Access: MapAccess<'de>,
            {
                let mut material = None;
                let mut uv_offset = None;
                let mut uv_scale = None;
                let mut uv_quarter_turns = None;
                let mut pivot = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "material" => {
                            if material.is_some() {
                                return Err(de::Error::duplicate_field("material"));
                            }
                            material = Some(map.next_value()?);
                        }
                        "uv_offset" => {
                            if uv_offset.is_some() {
                                return Err(de::Error::duplicate_field("uv_offset"));
                            }
                            uv_offset = Some(map.next_value()?);
                        }
                        "uv_scale" => {
                            if uv_scale.is_some() {
                                return Err(de::Error::duplicate_field("uv_scale"));
                            }
                            uv_scale = Some(map.next_value()?);
                        }
                        "uv_quarter_turns" => {
                            if uv_quarter_turns.is_some() {
                                return Err(de::Error::duplicate_field("uv_quarter_turns"));
                            }
                            uv_quarter_turns = Some(map.next_value()?);
                        }
                        "pivot" => {
                            if pivot.is_some() {
                                return Err(de::Error::duplicate_field("pivot"));
                            }
                            pivot = Some(map.next_value()?);
                        }
                        _ => {
                            return Err(de::Error::unknown_field(
                                &key,
                                &[
                                    "material",
                                    "uv_offset",
                                    "uv_scale",
                                    "uv_quarter_turns",
                                    "pivot",
                                ],
                            ));
                        }
                    }
                }
                Ok(Settings {
                    material: material.ok_or_else(|| de::Error::missing_field("material"))?,
                    uv_offset: uv_offset.ok_or_else(|| de::Error::missing_field("uv_offset"))?,
                    uv_scale: uv_scale.ok_or_else(|| de::Error::missing_field("uv_scale"))?,
                    uv_quarter_turns: uv_quarter_turns
                        .ok_or_else(|| de::Error::missing_field("uv_quarter_turns"))?,
                    pivot: pivot.ok_or_else(|| de::Error::missing_field("pivot"))?,
                })
            }
        }

        deserializer.deserialize_map(SettingsVisitor)
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            material: 0,
            uv_offset: [0.0; 2],
            uv_scale: [1.0; 2],
            uv_quarter_turns: 0,
            pivot: [0.0; 3],
        }
    }
}

pub(crate) fn identity() -> Settings {
    Settings::default()
}

pub(crate) fn is_identity(settings: &Settings) -> bool {
    let defaults = identity();
    settings.uv_offset == defaults.uv_offset
        && settings.uv_scale == defaults.uv_scale
        && settings.uv_quarter_turns == defaults.uv_quarter_turns
        && settings.pivot == defaults.pivot
}

pub(crate) fn validate(settings: Settings, model: &StaticModel) -> Result<Settings, String> {
    if settings
        .uv_offset
        .iter()
        .chain(settings.pivot.iter())
        .any(|value| !value.is_finite() || value.abs() > MAX_OFFSET)
        || settings
            .uv_scale
            .iter()
            .any(|value| !value.is_finite() || !(MIN_UV_SCALE..=MAX_UV_SCALE).contains(value))
        || settings.uv_quarter_turns > 3
    {
        return Err("surface UV and pivot settings are outside their finite bounds".into());
    }
    let material_index = usize::try_from(settings.material)
        .map_err(|_| "surface material index exceeds this platform")?;
    if material_index >= model.source().materials.len()
        || !model
            .source()
            .primitives
            .iter()
            .any(|primitive| primitive.material == settings.material)
    {
        return Err("selected surface material must exist and be used by a primitive".into());
    }
    Ok(settings)
}

pub(crate) fn derived_model(
    model: &StaticModel,
    settings: Settings,
    tint: [f32; 4],
) -> Result<StaticModel, String> {
    let settings = validate(settings, model)?;
    let tint = crate::mesh_material::validate(tint)?;
    crate::mesh_material::check_derived_work(model.source())?;
    let uv_identity = settings.uv_offset == [0.0; 2]
        && settings.uv_scale == [1.0; 2]
        && settings.uv_quarter_turns == 0;
    let pivot_identity = settings.pivot == [0.0; 3];
    let tint_identity = crate::mesh_material::is_identity(&tint);
    let mut source = model.source().clone();
    if !uv_identity || !pivot_identity || !tint_identity {
        for primitive in &mut source.primitives {
            if !uv_identity && primitive.material == settings.material {
                for vertex in &mut primitive.vertices {
                    vertex.uv = transform_uv(vertex.uv, settings)?;
                }
            }
            if !pivot_identity {
                for axis in 0..3 {
                    let pivot = settings.pivot[axis];
                    if pivot != 0.0 {
                        let translated = primitive.transform[3][axis] - pivot;
                        if !translated.is_finite() || translated.abs() > MAX_OFFSET {
                            return Err(format!(
                                "pivot translation axis {axis} exceeds transform bounds"
                            ));
                        }
                        primitive.transform[3][axis] = translated;
                    }
                }
            }
        }
        if !tint_identity {
            for material in &mut source.materials {
                for (component, multiplier) in material.base_color.iter_mut().zip(tint) {
                    *component *= multiplier;
                }
            }
        }
    }
    StaticModel::new(source).map_err(|error| format!("validate derived mesh surface: {error}"))
}

fn transform_uv(uv: [f32; 2], settings: Settings) -> Result<[f32; 2], String> {
    let scaled = [uv[0] * settings.uv_scale[0], uv[1] * settings.uv_scale[1]];
    if !scaled.iter().all(|value| value.is_finite()) {
        return Err("surface UV scale produced a nonfinite intermediate".into());
    }
    let rotated = match settings.uv_quarter_turns {
        0 => scaled,
        1 => [-scaled[1], scaled[0]],
        2 => [-scaled[0], -scaled[1]],
        3 => [scaled[1], -scaled[0]],
        _ => return Err("surface UV quarter turns must be in 0..=3".into()),
    };
    let result = [
        rotated[0] + settings.uv_offset[0],
        rotated[1] + settings.uv_offset[1],
    ];
    if !result
        .iter()
        .all(|value| value.is_finite() && value.abs() <= MAX_UV)
    {
        return Err("transformed surface UV must be finite and within +/-65536".into());
    }
    Ok(result)
}

#[cfg(test)]
#[path = "mesh_surface/tests.rs"]
mod tests;
