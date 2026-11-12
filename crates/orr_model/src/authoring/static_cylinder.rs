//! Byte-only capped-cylinder authoring with the existing fixed angular-grid contract.
//! The recipe is the sole input; this module performs no filesystem or collision work.
#![allow(clippy::float_arithmetic)]
use crate::{
    Dependency, Image, Material, ModelSource, Primitive, StaticModel, Vertex, Wrap, IDENTITY,
};
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use sha2::{Digest, Sha256};
use std::fmt;

pub const MAX_RECIPE_BYTES: usize = 16 * 1024;

struct Recipe {
    format: String,
    version: u32,
    radius: f64,
    height: f64,
    segments: u32,
}
// Require a map, rather than accepting serde's positional sequence-to-struct
// alias. The visitor retains JSON types and rejects duplicate keys directly.
impl<'de> Deserialize<'de> for Recipe {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RecipeVisitor;
        impl<'de> Visitor<'de> for RecipeVisitor {
            type Value = Recipe;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a cylinder recipe object with exactly five required fields")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Recipe, M::Error> {
                let (mut format, mut version, mut radius, mut height, mut segments) =
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
                        "radius" => {
                            if radius.is_some() {
                                return Err(de::Error::duplicate_field("radius"));
                            }
                            radius = Some(map.next_value()?);
                        }
                        "height" => {
                            if height.is_some() {
                                return Err(de::Error::duplicate_field("height"));
                            }
                            height = Some(map.next_value()?);
                        }
                        "segments" => {
                            if segments.is_some() {
                                return Err(de::Error::duplicate_field("segments"));
                            }
                            segments = Some(map.next_value()?);
                        }
                        _ => {
                            return Err(de::Error::unknown_field(
                                &key,
                                &["format", "version", "radius", "height", "segments"],
                            ))
                        }
                    }
                }
                Ok(Recipe {
                    format: format.ok_or_else(|| de::Error::missing_field("format"))?,
                    version: version.ok_or_else(|| de::Error::missing_field("version"))?,
                    radius: radius.ok_or_else(|| de::Error::missing_field("radius"))?,
                    height: height.ok_or_else(|| de::Error::missing_field("height"))?,
                    segments: segments.ok_or_else(|| de::Error::missing_field("segments"))?,
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
    if recipe.format != "orr_static_model_cylinder" || recipe.version != 1 {
        return Err("unsupported cylinder recipe format/version".into());
    }
    logical_path(asset_id, ".cylinder.json")?;
    // Check the parsed f64 before narrowing: an out-of-bounds value that rounds
    // to a valid f32 boundary must still be rejected, never clamped.
    for (name, value) in [("radius", recipe.radius), ("height", recipe.height)] {
        if !value.is_finite() || !(0.001..=1_000_000.0).contains(&value) {
            return Err(format!("{name} must be finite and within [0.001,1000000]"));
        }
    }
    if !matches!(recipe.segments, 8 | 16 | 32 | 64 | 128) {
        return Err("segments must be an integer in {8,16,32,64,128}".into());
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

// Definition of the angular grid: IEEE-754 binary32 sine values for the first
// quadrant at multiples of pi/64, rounded once and stored as bits. All allowed
// segment counts are exact subdivisions of this fixed 128-point circle. Neither
// cooking nor loading calls libm; this table is part of this private recipe v1
// contract. The cardinal endpoints are exactly 0 and 1. This is presentation
// geometry, not simulation math or a claim about untested platform serializers.
const QUARTER_SIN_BITS: [u32; 33] = [
    0x00000000, 0x3d48fb30, 0x3dc8bd36, 0x3e164083, 0x3e47c5c2, 0x3e78cfcc, 0x3e94a031, 0x3eac7cd4,
    0x3ec3ef15, 0x3edae880, 0x3ef15aea, 0x3f039c3d, 0x3f0e39da, 0x3f187fc0, 0x3f226799, 0x3f2beb4a,
    0x3f3504f3, 0x3f3daef9, 0x3f45e403, 0x3f4d9f02, 0x3f54db31, 0x3f5b941a, 0x3f61c598, 0x3f676bd8,
    0x3f6c835e, 0x3f710908, 0x3f74fa0b, 0x3f7853f8, 0x3f7b14be, 0x3f7d3aac, 0x3f7ec46d, 0x3f7fb10f,
    0x3f800000,
];
fn circle_point(index: u32, segments: u32) -> [f32; 2] {
    let grid = (index % segments) * (128 / segments);
    let quadrant = grid / 32;
    let step = (grid % 32) as usize;
    let sine = f32::from_bits(QUARTER_SIN_BITS[step]);
    let cosine = f32::from_bits(QUARTER_SIN_BITS[32 - step]);
    let point = match quadrant {
        0 => [cosine, sine],
        1 => [-sine, cosine],
        2 => [-cosine, -sine],
        3 => [sine, -cosine],
        _ => unreachable!("bounded 128-point angular grid"),
    };
    // Exact positive-zero cardinal coordinates make welded positions and the
    // duplicate side seam bit-identical across all three primitives.
    point.map(|value| if value == 0.0 { 0.0 } else { value })
}

/// Build a closed, Y-axis centered polygonal cylinder. Primitive/material slots
/// are stable: 0 is the smooth side, 1 the hard top cap, 2 the hard bottom cap.
/// Side U runs once around +X toward +Z and V runs top=0 to bottom=1. Cap UV is
/// the X/Z projection into [0,1]^2, including center [0.5,0.5]. Separate cap
/// vertices preserve hard normals; only the side duplicates the UV seam.
/// Recipe bytes are the sole direct input and their exact digest is retained.
pub fn author_cylinder(recipe_bytes: &[u8], recipe_asset_id: &str) -> Result<StaticModel, String> {
    let recipe = recipe(recipe_bytes, recipe_asset_id)?;
    let radius = recipe.radius as f32;
    let half_height = (recipe.height as f32) * 0.5;
    let segments = recipe.segments;
    let mut sides = Primitive {
        id: format!("{recipe_asset_id}#node=0/mesh=0/primitive=0"),
        vertices: Vec::with_capacity(2 * (segments as usize + 1)),
        indices: Vec::with_capacity(6 * segments as usize),
        material: 0,
        transform: IDENTITY,
    };
    for index in 0..=segments {
        let [x, z] = circle_point(index, segments);
        let u = index as f32 / segments as f32;
        for (y, v) in [(-half_height, 1.0), (half_height, 0.0)] {
            sides.vertices.push(Vertex {
                position: [radius * x, y, radius * z],
                normal: [x, 0.0, z],
                uv: [u, v],
            });
        }
    }
    for index in 0..segments {
        let bottom = 2 * index;
        sides.indices.extend_from_slice(&[
            bottom,
            bottom + 1,
            bottom + 2,
            bottom + 1,
            bottom + 3,
            bottom + 2,
        ]);
    }
    let mut primitives = vec![sides];
    for (slot, y, normal_y) in [(1, half_height, 1.0), (2, -half_height, -1.0)] {
        let normal = [0.0, normal_y, 0.0];
        let mut cap = Primitive {
            id: format!("{recipe_asset_id}#node=0/mesh=0/primitive={slot}"),
            vertices: Vec::with_capacity(segments as usize + 1),
            indices: Vec::with_capacity(3 * segments as usize),
            material: slot,
            transform: IDENTITY,
        };
        cap.vertices.push(Vertex {
            position: [0.0, y, 0.0],
            normal,
            uv: [0.5, 0.5],
        });
        for index in 0..segments {
            let [x, z] = circle_point(index, segments);
            cap.vertices.push(Vertex {
                position: [radius * x, y, radius * z],
                normal,
                uv: [0.5 + 0.5 * x, 0.5 + 0.5 * z],
            });
            let current = index + 1;
            let next = (index + 1) % segments + 1;
            cap.indices.extend_from_slice(&if slot == 1 {
                [0, next, current]
            } else {
                [0, current, next]
            });
        }
        primitives.push(cap);
    }
    let source = ModelSource {
        format: "orr_static_model".into(),
        version: 1,
        asset_id: recipe_asset_id.into(),
        dependencies: vec![Dependency {
            uri: "$source".into(),
            sha256: digest(recipe_bytes),
        }],
        primitives,
        materials: vec![
            Material {
                base_color: [1.0; 4],
                image: 0,
                linear_filter: false,
                wrap_s: Wrap::Clamp,
                wrap_t: Wrap::Clamp,
            };
            3
        ],
        images: vec![Image {
            width: 1,
            height: 1,
            rgba8: vec![255; 4],
        }],
    };
    let model = StaticModel::new(source).map_err(|e| format!("cylinder model validation: {e}"))?;
    let cooked = model
        .to_bytes()
        .map_err(|e| format!("cylinder model cook: {e}"))?;
    let reloaded =
        StaticModel::from_bytes(&cooked).map_err(|e| format!("cylinder model reload: {e}"))?;
    if !same_source_bits(reloaded.source(), model.source()) {
        return Err("cylinder model changed during cooked round-trip".into());
    }
    Ok(reloaded)
}

#[cfg(test)]
#[path = "static_cylinder_tests.rs"]
mod tests;
