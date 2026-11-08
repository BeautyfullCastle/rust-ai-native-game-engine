//! Byte-only sampled-animation pose baking into immutable static model geometry.
//! Uses the authoritative animation deformation path; performs no filesystem access.
use crate::{animation::AnimatedModel, Dependency, ModelSource, StaticModel, MAX_FILE_BYTES};
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use sha2::{Digest, Sha256};
use std::fmt;

pub const MAX_RECIPE_BYTES: usize = 16 * 1024;

struct Recipe {
    format: String,
    version: u32,
    source: String,
    clip: u32,
    time: f64,
}
// A map-only visitor rejects duplicate/unknown keys and positional arrays.
impl<'de> Deserialize<'de> for Recipe {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RecipeVisitor;
        impl<'de> Visitor<'de> for RecipeVisitor {
            type Value = Recipe;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a pose recipe object with exactly five required fields")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Recipe, M::Error> {
                let (mut format, mut version, mut source, mut clip, mut time) =
                    (None, None, None, None, None);
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
                        "clip" => {
                            if clip.is_some() {
                                return Err(de::Error::duplicate_field("clip"));
                            }
                            clip = Some(map.next_value()?);
                        }
                        "time" => {
                            if time.is_some() {
                                return Err(de::Error::duplicate_field("time"));
                            }
                            time = Some(map.next_value()?);
                        }
                        _ => {
                            return Err(de::Error::unknown_field(
                                &key,
                                &["format", "version", "source", "clip", "time"],
                            ))
                        }
                    }
                }
                Ok(Recipe {
                    format: format.ok_or_else(|| de::Error::missing_field("format"))?,
                    version: version.ok_or_else(|| de::Error::missing_field("version"))?,
                    source: source.ok_or_else(|| de::Error::missing_field("source"))?,
                    clip: clip.ok_or_else(|| de::Error::missing_field("clip"))?,
                    time: time.ok_or_else(|| de::Error::missing_field("time"))?,
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
    if recipe.format != "orr_animation_pose_bake" || recipe.version != 1 {
        return Err("unsupported pose recipe format/version".into());
    }
    logical_path(asset_id, ".pose.json")?;
    logical_path(&recipe.source, ".animated.json")?;
    if asset_id.eq_ignore_ascii_case(&recipe.source) {
        return Err("recipe identity and direct input path must be distinct".into());
    }
    if !recipe.time.is_finite() || recipe.time < 0.0 {
        return Err("time must be finite and nonnegative".into());
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

/// Freeze one explicit clip sample into static model-space geometry.
/// The production deformation path includes weighted skinning and normal
/// inverse transposes. No mesh-node transform is reapplied to skinned vertices.
/// Input bytes are read only; caller/entity placement is deliberately absent.
pub fn bake_pose(
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
    let original = AnimatedModel::from_bytes(&source_bytes)
        .map_err(|e| format!("source animated model: {e}"))?;
    let source = original.source();
    let clip_index = usize::try_from(recipe.clip).map_err(|_| "clip index overflow")?;
    let clip = source
        .clips
        .get(clip_index)
        .ok_or("clip index out of range")?;
    // Validate the requested f64 value before narrowing; rounding must not admit
    // an out-of-range request. Sampling itself follows the existing f32 API.
    if recipe.time > f64::from(clip.duration()) {
        return Err("time exceeds selected clip duration".into());
    }
    let time = recipe.time as f32;
    if !time.is_finite() || (recipe.time > 0.0 && time == 0.0) {
        return Err("time cannot narrow to a finite nonzero f32 sample".into());
    }
    let pose = original
        .sample_clip(recipe.clip, time)
        .map_err(|e| format!("sample selected clip: {e}"))?;
    let mut primitives = original
        .deform(&pose)
        .map_err(|e| format!("deform selected pose: {e}"))?;
    for primitive in &mut primitives {
        let suffix = primitive
            .id
            .strip_prefix(&source.asset_id)
            .filter(|s| s.starts_with("#node="))
            .ok_or("unexpected source primitive identity")?;
        primitive.id = format!("{recipe_asset_id}{suffix}");
    }
    let model = StaticModel::new(ModelSource {
        format: "orr_static_model".into(),
        version: 1,
        asset_id: recipe_asset_id.to_owned(),
        dependencies: vec![
            Dependency {
                uri: "$source".into(),
                sha256: digest(recipe_bytes),
            },
            Dependency {
                uri: recipe.source,
                sha256: digest(&source_bytes),
            },
        ],
        primitives,
        materials: source.materials.clone(),
        images: source.images.clone(),
    })
    .map_err(|e| format!("baked static model validation: {e}"))?;
    // Static local-vertex limits are deliberately enforced even when the
    // corresponding animated model-space position was valid before baking.
    let cooked = model
        .to_bytes()
        .map_err(|e| format!("baked model cook: {e}"))?;
    let reloaded =
        StaticModel::from_bytes(&cooked).map_err(|e| format!("baked model reload: {e}"))?;
    if !same_source_bits(model.source(), reloaded.source()) {
        return Err("baked model changed during cooked round-trip".into());
    }
    Ok(reloaded)
}

#[cfg(test)]
#[path = "pose_bake_tests.rs"]
mod tests;
