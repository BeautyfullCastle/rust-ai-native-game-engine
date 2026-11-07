//! Append an explicitly bounded clip excerpt; original clips and rig stay intact.
#![allow(clippy::float_arithmetic)]
use crate::{
    animation::{
        AnimatedModel, AnimationChannel, AnimationClip, ChannelValues, MAX_ANIMATION_BYTES,
        MAX_ANIMATION_KEYS, MAX_CLIPS,
    },
    animation_sequence::{cooked, digest, logical_path},
    Dependency, MAX_FILE_BYTES,
};
use serde::{
    de::{MapAccess, Visitor},
    Deserialize, Deserializer,
};
use std::fmt;

pub const MAX_RECIPE_BYTES: usize = 16 * 1024;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recipe {
    format: String,
    version: u32,
    source: String,
    clip: u32,
    start_seconds: f64,
    end_seconds: f64,
    name: String,
}
fn parse(bytes: &[u8], id: &str) -> Result<Recipe, String> {
    if bytes.len() > MAX_RECIPE_BYTES {
        return Err("trim recipe exceeds 16 KiB limit".into());
    }
    struct Object;
    impl<'de> Visitor<'de> for Object {
        type Value = Recipe;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a complete clip trim recipe object")
        }
        fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<Recipe, M::Error> {
            Recipe::deserialize(serde::de::value::MapAccessDeserializer::new(map))
        }
    }
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let r = decoder
        .deserialize_map(Object)
        .map_err(|e| format!("trim recipe JSON: {e}"))?;
    decoder
        .end()
        .map_err(|e| format!("trim recipe JSON: {e}"))?;
    if r.format != "orr_animation_clip_trim" || r.version != 1 {
        return Err("unsupported clip trim format/version".into());
    }
    logical_path(id, ".trim.json")?;
    logical_path(&r.source, ".animated.json")?;
    if r.name.trim().is_empty() || r.name.len() > 128 || r.name.chars().any(char::is_control) {
        return Err("new clip name must have 1..=128 UTF-8 bytes and no control characters".into());
    }
    if r.clip as usize >= MAX_CLIPS
        || !r.start_seconds.is_finite()
        || !r.end_seconds.is_finite()
        || r.start_seconds < 0.0
        || r.end_seconds <= r.start_seconds
        || r.end_seconds > 3600.0
    {
        return Err("trim range must be finite, 0 <= start < end <= 3600 and clip within the supported limit".into());
    }
    let (start, end) = (r.start_seconds as f32, r.end_seconds as f32);
    if !start.is_finite() || !end.is_finite() || end <= start || !(end - start).is_finite() {
        return Err("trim range collapses in f32".into());
    }
    Ok(r)
}
fn charge(total: &mut usize, n: usize, limit: usize) -> Result<(), String> {
    *total = total
        .checked_add(n)
        .filter(|n| *n <= limit)
        .ok_or("trim animation budget exceeded")?;
    Ok(())
}
/// The resolver is called once, after recipe admission, for the direct cooked
/// source only. Historical dependency hashes are provenance, never resolved.
pub fn trim_clip(
    recipe_bytes: &[u8],
    recipe_asset_id: &str,
    mut resolver: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<AnimatedModel, String> {
    let r = parse(recipe_bytes, recipe_asset_id)?;
    let input = resolver(&r.source)?;
    recipe_bytes
        .len()
        .checked_add(input.len())
        .filter(|n| *n <= MAX_FILE_BYTES)
        .ok_or("trim recipe plus source exceeds 64 MiB input budget")?;
    let original = AnimatedModel::from_bytes(&input).map_err(|e| format!("trim source: {e}"))?;
    let source = original.source();
    if recipe_asset_id.eq_ignore_ascii_case(&source.asset_id) {
        return Err("new asset identity must differ from source identity".into());
    }
    if source.clips.len() >= MAX_CLIPS {
        return Err("clip count limit exceeded".into());
    }
    if source.clips.iter().any(|clip| clip.name == r.name) {
        return Err("new clip name already exists".into());
    }
    let clip = source
        .clips
        .get(r.clip as usize)
        .ok_or("trim clip index out of range")?;
    if r.end_seconds > f64::from(clip.duration()) {
        return Err("trim end exceeds selected clip duration".into());
    }
    let (start, end) = (r.start_seconds as f32, r.end_seconds as f32);
    let duration = end - start;
    let a = original
        .sample_clip(r.clip, start)
        .map_err(|e| e.to_string())?;
    let b = original
        .sample_clip(r.clip, end)
        .map_err(|e| e.to_string())?;
    let (mut keys, mut bytes) = (0, 0);
    for channel in source.clips.iter().flat_map(|clip| &clip.channels) {
        charge(&mut keys, channel.times.len(), MAX_ANIMATION_KEYS)?;
        charge(
            &mut bytes,
            channel.times.len()
                * if matches!(channel.values, ChannelValues::Rotation(_)) {
                    20
                } else {
                    16
                },
            MAX_ANIMATION_BYTES,
        )?;
    }
    let mut channels = Vec::with_capacity(clip.channels.len());
    for c in &clip.channels {
        let interior: Vec<usize> = c
            .times
            .iter()
            .enumerate()
            .filter_map(|(i, t)| (*t > start && *t < end).then_some(i))
            .collect();
        charge(&mut keys, interior.len() + 2, MAX_ANIMATION_KEYS)?;
        charge(
            &mut bytes,
            (interior.len() + 2)
                * if matches!(c.values, ChannelValues::Rotation(_)) {
                    20
                } else {
                    16
                },
            MAX_ANIMATION_BYTES,
        )?;
        let times: Vec<f32> = std::iter::once(0.0)
            .chain(interior.iter().map(|i| c.times[*i] - start))
            .chain(std::iter::once(duration))
            .collect();
        if times.windows(2).any(|w| !w[1].is_finite() || w[1] <= w[0]) {
            return Err("trim key timeline collapses after f32 rebasing".into());
        }
        let first = a.local()[c.node as usize];
        let last = b.local()[c.node as usize];
        macro_rules! values {
            ($v:ident,$kind:ident,$field:ident) => {
                ChannelValues::$kind(
                    std::iter::once(first.$field)
                        .chain(interior.iter().map(|i| $v[*i]))
                        .chain(std::iter::once(last.$field))
                        .collect(),
                )
            };
        }
        let values = match &c.values {
            ChannelValues::Translation(v) => values!(v, Translation, translation),
            ChannelValues::Rotation(v) => values!(v, Rotation, rotation),
            ChannelValues::Scale(v) => values!(v, Scale, scale),
        };
        channels.push(AnimationChannel {
            node: c.node,
            interpolation: c.interpolation,
            times,
            values,
        });
    }
    let mut result = source.clone();
    for primitive in &mut result.primitives {
        let suffix = primitive
            .id
            .strip_prefix(&source.asset_id)
            .filter(|s| s.starts_with("#node="))
            .ok_or("unexpected source primitive identity")?;
        primitive.id = format!("{recipe_asset_id}{suffix}");
    }
    result.asset_id = recipe_asset_id.into();
    result.dependencies = vec![
        Dependency {
            uri: "$source".into(),
            sha256: digest(recipe_bytes),
        },
        Dependency {
            uri: r.source,
            sha256: digest(&input),
        },
    ];
    result.clips.push(AnimationClip {
        name: r.name,
        channels,
    });
    let model = AnimatedModel::new(result).map_err(|e| format!("trim model validation: {e}"))?;
    let bytes = cooked(&model)?;
    let reloaded =
        AnimatedModel::from_bytes(&bytes).map_err(|e| format!("trim cooked reload: {e}"))?;
    if cooked(&reloaded)? != bytes {
        return Err("trim changed during cooked round-trip".into());
    }
    Ok(reloaded)
}
#[cfg(test)]
#[path = "animation_trim_tests.rs"]
mod tests;
