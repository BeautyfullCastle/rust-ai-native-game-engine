//! Byte-only bounded model-space planar UV authoring. No filesystem access.
//! Selected material UVs are generated from transformed positions; other payload bits remain intact.
#![allow(clippy::float_arithmetic)]
use crate::{Dependency, StaticModel, MAX_FILE_BYTES};
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use sha2::{Digest, Sha256};
use std::fmt;

pub const MAX_RECIPE_BYTES: usize = 16 * 1024;

struct Recipe {
    format: String,
    version: u32,
    source: String,
    material: u32,
    origin: [f32; 3],
    units_per_repeat: [f32; 2],
    plane: String,
}
// Map-only admission rejects positional struct aliases and duplicate fields.
impl<'de> Deserialize<'de> for Recipe {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RecipeVisitor;
        impl<'de> Visitor<'de> for RecipeVisitor {
            type Value = Recipe;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a UV recipe object with exactly seven required fields")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Recipe, M::Error> {
                let (mut format, mut version, mut source, mut material) = (None, None, None, None);
                let (mut origin, mut units_per_repeat, mut plane) = (None, None, None);
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "format" => {
                            if format.is_some() {
                                return Err(de::Error::duplicate_field("format"));
                            }
                            format = Some(map.next_value()?);
                        }
                        "version" => {
                            if version.is_some() {
                                return Err(de::Error::duplicate_field("version"));
                            }
                            version = Some(map.next_value()?);
                        }
                        "source" => {
                            if source.is_some() {
                                return Err(de::Error::duplicate_field("source"));
                            }
                            source = Some(map.next_value()?);
                        }
                        "material" => {
                            if material.is_some() {
                                return Err(de::Error::duplicate_field("material"));
                            }
                            material = Some(map.next_value()?);
                        }
                        "origin" => {
                            if origin.is_some() {
                                return Err(de::Error::duplicate_field("origin"));
                            }
                            origin = Some(map.next_value()?);
                        }
                        "units_per_repeat" => {
                            if units_per_repeat.is_some() {
                                return Err(de::Error::duplicate_field("units_per_repeat"));
                            }
                            units_per_repeat = Some(map.next_value()?);
                        }
                        "plane" => {
                            if plane.is_some() {
                                return Err(de::Error::duplicate_field("plane"));
                            }
                            plane = Some(map.next_value()?);
                        }
                        _ => {
                            return Err(de::Error::unknown_field(
                                &key,
                                &[
                                    "format",
                                    "version",
                                    "source",
                                    "material",
                                    "origin",
                                    "units_per_repeat",
                                    "plane",
                                ],
                            ))
                        }
                    }
                }
                Ok(Recipe {
                    format: format.ok_or_else(|| de::Error::missing_field("format"))?,
                    version: version.ok_or_else(|| de::Error::missing_field("version"))?,
                    source: source.ok_or_else(|| de::Error::missing_field("source"))?,
                    material: material.ok_or_else(|| de::Error::missing_field("material"))?,
                    origin: origin.ok_or_else(|| de::Error::missing_field("origin"))?,
                    units_per_repeat: units_per_repeat
                        .ok_or_else(|| de::Error::missing_field("units_per_repeat"))?,
                    plane: plane.ok_or_else(|| de::Error::missing_field("plane"))?,
                })
            }
        }
        deserializer.deserialize_map(RecipeVisitor)
    }
}
fn logical_path(path: &str, suffix: &str) -> Result<(), String> {
    if path.is_empty()
        || path.len() > 240
        || !path.ends_with(suffix)
        || !path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._/".contains(&b))
    {
        return Err(format!(
            "expected a portable relative ASCII {suffix} path of at most 240 bytes"
        ));
    }
    for segment in path.split('/') {
        let device = segment.split('.').next().unwrap_or("").to_ascii_uppercase();
        if segment.is_empty()
            || segment.starts_with('.')
            || segment.ends_with('.')
            || ["CON", "PRN", "AUX", "NUL"].contains(&device.as_str())
            || (device.len() == 4
                && (device.starts_with("COM") || device.starts_with("LPT"))
                && device.as_bytes()[3].is_ascii_digit())
        {
            return Err("path has an empty, hidden, traversal or reserved device segment".into());
        }
    }
    Ok(())
}
fn recipe(bytes: &[u8], asset_id: &str) -> Result<Recipe, String> {
    if bytes.len() > MAX_RECIPE_BYTES {
        return Err("recipe exceeds 16 KiB limit".into());
    }
    let recipe: Recipe = serde_json::from_slice(bytes).map_err(|e| format!("recipe JSON: {e}"))?;
    if recipe.format != "orr_static_planar_uv" || recipe.version != 1 {
        return Err("unsupported UV recipe format/version".into());
    }
    logical_path(asset_id, ".planar-uv.json")?;
    logical_path(&recipe.source, ".model.json")?;
    if asset_id.eq_ignore_ascii_case(&recipe.source) {
        return Err("recipe identity and direct input path must be distinct".into());
    }
    if !recipe.origin.iter().all(|v| v.is_finite())
        || !recipe
            .units_per_repeat
            .iter()
            .all(|v| v.is_finite() && *v > 0.0)
        || !["xy", "xz", "yz"].contains(&recipe.plane.as_str())
    {
        return Err("planar UV recipe requires finite origin, positive finite units_per_repeat and plane xy/xz/yz".into());
    }
    Ok(recipe)
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn projected(
    position: [f32; 3],
    transform: [[f32; 4]; 4],
    recipe: &Recipe,
) -> Result<[f32; 2], String> {
    // Source admission requires an affine bottom row [0,0,0,1]. There is no
    // perspective divide. Explicit ordered f64 multiply/add, never mul_add.
    let mut model_position = [0.0_f64; 3];
    for axis in 0..3 {
        let x = f64::from(transform[0][axis]) * f64::from(position[0]);
        let y = f64::from(transform[1][axis]) * f64::from(position[1]);
        let z = f64::from(transform[2][axis]) * f64::from(position[2]);
        let xy = x + y;
        let xyz = xy + z;
        model_position[axis] = xyz + f64::from(transform[3][axis]);
        if !model_position[axis].is_finite() {
            return Err("model-space position must be finite".into());
        }
    }
    let axes = match recipe.plane.as_str() {
        "xy" => [0, 1],
        "xz" => [0, 2],
        "yz" => [1, 2],
        _ => return Err("unsupported projection plane".into()),
    };
    let mut result = [0.0_f32; 2];
    for (component, axis) in axes.into_iter().enumerate() {
        let shifted = model_position[axis] - f64::from(recipe.origin[axis]);
        let uv = shifted / f64::from(recipe.units_per_repeat[component]);
        if !uv.is_finite() || uv.abs() > 65536.0 {
            return Err("projected UV must be finite and within +/-65536".into());
        }
        result[component] = uv as f32;
        if !result[component].is_finite() || result[component].abs() > 65536.0 {
            return Err("projected f32 UV must be finite and within +/-65536".into());
        }
    }
    Ok(result)
}

/// Generate planar UV0 from model-space positions for one used material slot.
/// No source dependency is followed: exact cooked bytes bind their provenance.
pub fn project_model_uv(
    recipe_bytes: &[u8],
    recipe_asset_id: &str,
    mut resolver: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<StaticModel, String> {
    let recipe = recipe(recipe_bytes, recipe_asset_id)?;
    let source_bytes = resolver(&recipe.source)?;
    recipe_bytes
        .len()
        .checked_add(source_bytes.len())
        .filter(|&sum| sum <= MAX_FILE_BYTES)
        .ok_or("recipe plus cooked source exceeds 64 MiB input budget")?;
    let original =
        StaticModel::from_bytes(&source_bytes).map_err(|e| format!("source model: {e}"))?;
    let source = original.source();
    let material = usize::try_from(recipe.material).map_err(|_| "material index overflow")?;
    if material >= source.materials.len()
        || !source
            .primitives
            .iter()
            .any(|p| p.material == recipe.material)
    {
        return Err("selected material must exist and be used by a primitive".into());
    }
    let mut result = source.clone();
    result.asset_id = recipe_asset_id.to_owned();
    for primitive in &mut result.primitives {
        let suffix = primitive
            .id
            .strip_prefix(&source.asset_id)
            .filter(|s| s.starts_with("#node="))
            .ok_or("unexpected source primitive identity")?;
        primitive.id = format!("{recipe_asset_id}{suffix}");
        if primitive.material == recipe.material {
            for vertex in &mut primitive.vertices {
                vertex.uv = projected(vertex.position, primitive.transform, &recipe)?;
            }
        }
    }
    result.dependencies = vec![
        Dependency {
            uri: "$source".into(),
            sha256: digest(recipe_bytes),
        },
        Dependency {
            uri: recipe.source,
            sha256: digest(&source_bytes),
        },
    ];
    let model = StaticModel::new(result).map_err(|e| format!("UV model validation: {e}"))?;
    let cooked = model
        .to_bytes()
        .map_err(|e| format!("UV model cook: {e}"))?;
    let reloaded = StaticModel::from_bytes(&cooked).map_err(|e| format!("UV model reload: {e}"))?;
    if reloaded.source() != model.source()
        || reloaded.to_bytes().map_err(|e| e.to_string())? != cooked
    {
        return Err("UV model changed during cooked round-trip".into());
    }
    Ok(reloaded)
}

#[cfg(test)]
#[path = "static_planar_uv_tests.rs"]
mod tests;
