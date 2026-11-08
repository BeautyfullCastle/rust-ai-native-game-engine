//! Byte-only bounded cooked static-model pivot authoring. No filesystem access.
//! Geometry and linear transforms are retained; source-world origins receive T(-pivot).
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
    pivot: [f32; 3],
}
// A map-only visitor also excludes serde's positional sequence-to-struct alias.
// Values retain their JSON types and duplicate keys never get discarded.
impl<'de> Deserialize<'de> for Recipe {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RecipeVisitor;
        impl<'de> Visitor<'de> for RecipeVisitor {
            type Value = Recipe;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a pivot recipe object with exactly four required fields")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Recipe, M::Error> {
                let (mut format, mut version, mut source, mut pivot) = (None, None, None, None);
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
                        "pivot" => {
                            if pivot.is_some() {
                                return Err(de::Error::duplicate_field("pivot"));
                            }
                            pivot = Some(map.next_value()?);
                        }
                        _ => {
                            return Err(de::Error::unknown_field(
                                &key,
                                &["format", "version", "source", "pivot"],
                            ))
                        }
                    }
                }
                Ok(Recipe {
                    format: format.ok_or_else(|| de::Error::missing_field("format"))?,
                    version: version.ok_or_else(|| de::Error::missing_field("version"))?,
                    source: source.ok_or_else(|| de::Error::missing_field("source"))?,
                    pivot: pivot.ok_or_else(|| de::Error::missing_field("pivot"))?,
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
    if recipe.format != "orr_static_model_pivot" || recipe.version != 1 {
        return Err("unsupported pivot recipe format/version".into());
    }
    logical_path(asset_id, ".pivot.json")?;
    logical_path(&recipe.source, ".model.json")?;
    if asset_id.eq_ignore_ascii_case(&recipe.source) {
        return Err("recipe identity and direct input path must be distinct".into());
    }
    if !recipe
        .pivot
        .iter()
        .all(|v| v.is_finite() && v.abs() <= 1.0e6)
    {
        return Err("pivot must contain three finite f32 values within +/-1000000".into());
    }
    Ok(recipe)
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn charge(total: usize, bytes: usize) -> Result<usize, String> {
    total
        .checked_add(bytes)
        .filter(|&sum| sum <= MAX_FILE_BYTES)
        .ok_or_else(|| "recipe plus cooked source exceeds 64 MiB input budget".into())
}
/// Rebase every fully composed cooked primitive by T(-pivot) * M. Only the
/// three translation entries change; recipe identity namespaces primitive IDs.
/// The source is admitted once, with no resolution of historical dependencies.
pub fn rebase_model_pivot(
    recipe_bytes: &[u8],
    recipe_asset_id: &str,
    mut resolver: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<StaticModel, String> {
    let recipe = recipe(recipe_bytes, recipe_asset_id)?;
    let source_bytes = resolver(&recipe.source)?;
    charge(recipe_bytes.len(), source_bytes.len())?;
    let original =
        StaticModel::from_bytes(&source_bytes).map_err(|e| format!("source model: {e}"))?;
    let source = original.source();
    let mut result = source.clone();
    result.asset_id = recipe_asset_id.to_owned();
    for primitive in &mut result.primitives {
        let suffix = primitive
            .id
            .strip_prefix(&source.asset_id)
            .filter(|s| s.starts_with("#node="))
            .ok_or("unexpected source primitive identity")?;
        primitive.id = format!("{recipe_asset_id}{suffix}");
        for (axis, pivot) in recipe.pivot.into_iter().enumerate() {
            // Both signs of zero are an exact identity: do not let subtraction
            // canonicalize a source negative zero or perturb any other bits.
            if pivot != 0.0 {
                let translated = primitive.transform[3][axis] - pivot;
                if !translated.is_finite() || translated.abs() > 1.0e6 {
                    return Err(format!(
                        "pivot translation axis {axis} exceeds transform bounds"
                    ));
                }
                primitive.transform[3][axis] = translated;
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
    let model = StaticModel::new(result).map_err(|e| format!("rebased model validation: {e}"))?;
    let cooked = model
        .to_bytes()
        .map_err(|e| format!("rebased model cook: {e}"))?;
    let reloaded =
        StaticModel::from_bytes(&cooked).map_err(|e| format!("rebased model reload: {e}"))?;
    if reloaded.source() != model.source() {
        return Err("rebased model changed during cooked round-trip".into());
    }
    Ok(reloaded)
}

#[cfg(test)]
#[path = "static_pivot_tests.rs"]
mod tests;
