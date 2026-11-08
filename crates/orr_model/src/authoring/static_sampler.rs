//! Byte-only bounded cooked material sampler authoring. No filesystem access.
//! Only one used material's sampler defaults change; all shared image data stays intact.
use crate::{Dependency, ModelSource, StaticModel, Wrap, MAX_FILE_BYTES};
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use sha2::{Digest, Sha256};
use std::fmt;

pub const MAX_RECIPE_BYTES: usize = 16 * 1024;

struct Recipe {
    format: String,
    version: u32,
    source: String,
    linear_filter: bool,
    wrap_s: Wrap,
    wrap_t: Wrap,
    material: u32,
}
// A map-only visitor also excludes serde's positional sequence-to-struct alias.
// Values retain their exact JSON types and duplicate keys never get discarded.
impl<'de> Deserialize<'de> for Recipe {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RecipeVisitor;
        impl<'de> Visitor<'de> for RecipeVisitor {
            type Value = Recipe;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a sampler recipe object with exactly seven required fields")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Recipe, M::Error> {
                let (
                    mut format,
                    mut version,
                    mut source,
                    mut material,
                    mut linear_filter,
                    mut wrap_s,
                    mut wrap_t,
                ) = (None, None, None, None, None, None, None);
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
                        "linear_filter" => {
                            if linear_filter.is_some() {
                                return Err(de::Error::duplicate_field("linear_filter"));
                            }
                            linear_filter = Some(map.next_value()?);
                        }
                        "wrap_s" => {
                            if wrap_s.is_some() {
                                return Err(de::Error::duplicate_field("wrap_s"));
                            }
                            wrap_s = Some(read_wrap(&mut map)?);
                        }
                        "wrap_t" => {
                            if wrap_t.is_some() {
                                return Err(de::Error::duplicate_field("wrap_t"));
                            }
                            wrap_t = Some(read_wrap(&mut map)?);
                        }
                        "material" => {
                            if material.is_some() {
                                return Err(de::Error::duplicate_field("material"));
                            }
                            material = Some(map.next_value()?);
                        }
                        _ => {
                            return Err(de::Error::unknown_field(
                                &key,
                                &[
                                    "format",
                                    "version",
                                    "source",
                                    "material",
                                    "linear_filter",
                                    "wrap_s",
                                    "wrap_t",
                                ],
                            ))
                        }
                    }
                }
                Ok(Recipe {
                    format: format.ok_or_else(|| de::Error::missing_field("format"))?,
                    version: version.ok_or_else(|| de::Error::missing_field("version"))?,
                    source: source.ok_or_else(|| de::Error::missing_field("source"))?,
                    linear_filter: linear_filter
                        .ok_or_else(|| de::Error::missing_field("linear_filter"))?,
                    wrap_s: wrap_s.ok_or_else(|| de::Error::missing_field("wrap_s"))?,
                    wrap_t: wrap_t.ok_or_else(|| de::Error::missing_field("wrap_t"))?,
                    material: material.ok_or_else(|| de::Error::missing_field("material"))?,
                })
            }
        }
        deserializer.deserialize_map(RecipeVisitor)
    }
}
// Deserialize as a string first: serde enum object aliases such as
// {"clamp": null} are intentionally not part of this private recipe format.
fn read_wrap<'de, M: MapAccess<'de>>(map: &mut M) -> Result<Wrap, M::Error> {
    match map.next_value::<String>()?.as_str() {
        "clamp" => Ok(Wrap::Clamp),
        "repeat" => Ok(Wrap::Repeat),
        "mirror" => Ok(Wrap::Mirror),
        value => Err(de::Error::unknown_variant(
            value,
            &["clamp", "repeat", "mirror"],
        )),
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
    if recipe.format != "orr_static_material_sampler" || recipe.version != 1 {
        return Err("unsupported sampler recipe format/version".into());
    }
    logical_path(asset_id, ".sampler.json")?;
    logical_path(&recipe.source, ".model.json")?;
    if asset_id.eq_ignore_ascii_case(&recipe.source) {
        return Err("recipe identity and direct input path must be distinct".into());
    }
    Ok(recipe)
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
/// Exact semantic comparison, including every floating-point sign bit. Derived
/// PartialEq alone treats positive and negative zero as equal.
fn same_source_bits(a: &ModelSource, b: &ModelSource) -> bool {
    a == b
        && a.materials
            .iter()
            .zip(&b.materials)
            .all(|(a, b)| a.base_color.map(f32::to_bits) == b.base_color.map(f32::to_bits))
        && a.primitives.iter().zip(&b.primitives).all(|(a, b)| {
            a.transform.map(|column| column.map(f32::to_bits))
                == b.transform.map(|column| column.map(f32::to_bits))
                && a.vertices.iter().zip(&b.vertices).all(|(a, b)| {
                    a.position.map(f32::to_bits) == b.position.map(f32::to_bits)
                        && a.normal.map(f32::to_bits) == b.normal.map(f32::to_bits)
                        && a.uv.map(f32::to_bits) == b.uv.map(f32::to_bits)
                })
        })
}

/// Replace the three sampler defaults of one used material in a cooked model.
/// Every primitive sharing that material sees the new defaults. All image bytes,
/// color factors (including alpha), UVs, geometry and transforms stay unchanged.
/// This adds no mipmaps, anisotropy, separate min/mag filters or PBR behavior.
pub fn author_sampler(
    recipe_bytes: &[u8],
    recipe_asset_id: &str,
    mut resolver: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<StaticModel, String> {
    let recipe = recipe(recipe_bytes, recipe_asset_id)?;
    let source_bytes = resolver(&recipe.source)?;
    recipe_bytes
        .len()
        .checked_add(source_bytes.len())
        .filter(|&total| total <= MAX_FILE_BYTES)
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
    }
    result.materials[material].linear_filter = recipe.linear_filter;
    result.materials[material].wrap_s = recipe.wrap_s;
    result.materials[material].wrap_t = recipe.wrap_t;
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
    let model =
        StaticModel::new(result).map_err(|e| format!("sampler-authored model validation: {e}"))?;
    let cooked = model
        .to_bytes()
        .map_err(|e| format!("sampler-authored model cook: {e}"))?;
    let reloaded = StaticModel::from_bytes(&cooked)
        .map_err(|e| format!("sampler-authored model reload: {e}"))?;
    if !same_source_bits(model.source(), reloaded.source()) {
        return Err("sampler-authored model changed during cooked round-trip".into());
    }
    Ok(reloaded)
}

#[cfg(test)]
#[path = "static_sampler_tests.rs"]
mod tests;
