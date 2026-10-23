//! Private, offline grid authoring. Reuses the existing no-replace publisher.
#[path = "../png_sequence_package/offline_io.rs"]
mod offline_io;
use offline_io::{
    checked_path, json_bytes, publish, read_regular, relative_input, rgba_size, validate_name,
    validate_version,
};
use orr_sprite::{
    Atlas, ClipSource, Frame, PlaybackMode, Region, SpriteDocument, SpriteSource, MAX_CLIPS,
    MAX_FRAMES_PER_CLIP, MAX_REGIONS, MAX_TOTAL_FRAMES,
};
use serde::{Deserialize, Deserializer, Serialize};
use std::{collections::BTreeMap, marker::PhantomData, path::PathBuf};

pub const MAX_RECIPE_BYTES: usize = 1024 * 1024;
const MAX_PNG_BYTES: usize = 16 * 1024 * 1024;
const MAX_LICENSE_BYTES: usize = 64 * 1024;
pub struct Request {
    pub recipe: PathBuf,
    pub out: PathBuf,
}
#[derive(Debug, PartialEq, Eq)]
pub struct Report {
    pub width: u32,
    pub height: u32,
    pub cells: usize,
    pub rgba_bytes: usize,
}

// JSON records must be maps, not the positional arrays accepted by derive.
// Forwarding the original map also preserves duplicate-field rejection.
struct Object<T>(T);
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Object<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Visitor<T> {
            type Value = Object<T>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                map: A,
            ) -> Result<Self::Value, A::Error> {
                T::deserialize(serde::de::value::MapAccessDeserializer::new(map)).map(Object)
            }
        }
        d.deserialize_map(Visitor(PhantomData))
    }
}
fn bounded_objects<'de, D, T, const LIMIT: usize>(d: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Visitor<T, const LIMIT: usize>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>, const LIMIT: usize> serde::de::Visitor<'de> for Visitor<T, LIMIT> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "at most {LIMIT} objects")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let mut values = Vec::new();
            while let Some(Object(value)) = seq.next_element::<Object<T>>()? {
                if values.len() == LIMIT {
                    return Err(serde::de::Error::custom("collection limit exceeded"));
                }
                values.push(value);
            }
            Ok(values)
        }
    }
    d.deserialize_seq(Visitor::<T, LIMIT>(PhantomData))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recipe {
    schema: u32,
    name: String,
    version: String,
    license: String,
    png: String,
    cell_width: u32,
    cell_height: u32,
    columns: u32,
    rows: u32,
    #[serde(deserialize_with = "bounded_clips")]
    clips: Vec<NamedClip>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NamedClip {
    id: String,
    mode: PlaybackMode,
    #[serde(deserialize_with = "bounded_references")]
    frames: Vec<Reference>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reference {
    cell: u32,
    duration_ms: u64,
}
fn bounded_references<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Reference>, D::Error> {
    bounded_objects::<D, Reference, MAX_FRAMES_PER_CLIP>(d)
}
fn bounded_clips<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<NamedClip>, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = Vec<NamedClip>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("bounded named clips")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let mut clips = Vec::new();
            let mut total = 0usize;
            while let Some(Object(clip)) = seq.next_element::<Object<NamedClip>>()? {
                if clips.len() == MAX_CLIPS {
                    return Err(serde::de::Error::custom("clip count limit exceeded"));
                }
                total = total
                    .checked_add(clip.frames.len())
                    .filter(|v| *v <= MAX_TOTAL_FRAMES)
                    .ok_or_else(|| serde::de::Error::custom("total frame count limit exceeded"))?;
                clips.push(clip);
            }
            Ok(clips)
        }
    }
    d.deserialize_seq(Visitor)
}
#[derive(Serialize)]
struct Package<'a> {
    schema: u32,
    name: &'a str,
    version: &'a str,
    engine: &'static str,
    capabilities: [&'static str; 1],
    dependencies: BTreeMap<String, String>,
    files: [&'static str; 3],
}

pub fn import_package(request: &Request) -> Result<Report, String> {
    let recipe_path = checked_path(&request.recipe, false)?;
    let Object(recipe): Object<Recipe> =
        serde_json::from_slice(&read_regular(&recipe_path, MAX_RECIPE_BYTES)?)
            .map_err(|e| format!("recipe: {e}"))?;
    if recipe.schema != 1 {
        return Err("grid recipe schema must be 1".into());
    }
    if !validate_name(&recipe.name) || !validate_version(&recipe.version) {
        return Err("valid package name/canonical version required".into());
    }
    if recipe.cell_width == 0 || recipe.cell_height == 0 || recipe.columns == 0 || recipe.rows == 0
    {
        return Err("cell dimensions, columns and rows must be positive".into());
    }
    if recipe.clips.is_empty() {
        return Err("require at least one named clip".into());
    }
    let cells = recipe
        .columns
        .checked_mul(recipe.rows)
        .filter(|v| *v as usize <= MAX_REGIONS)
        .ok_or("grid cell count exceeds runtime limit")?;
    let width = recipe
        .cell_width
        .checked_mul(recipe.columns)
        .ok_or("grid width overflow")?;
    let height = recipe
        .cell_height
        .checked_mul(recipe.rows)
        .ok_or("grid height overflow")?;
    let rgba_bytes = rgba_size(width, height)?;
    let regions = (0..cells)
        .map(|id| Region {
            id,
            x: (id % recipe.columns) * recipe.cell_width,
            y: (id / recipe.columns) * recipe.cell_height,
            width: recipe.cell_width,
            height: recipe.cell_height,
        })
        .collect();
    let source = SpriteSource {
        format: orr_sprite::FORMAT.into(),
        version: orr_sprite::VERSION,
        atlas: Atlas {
            image: "atlas.png".into(),
            width,
            height,
        },
        regions,
        clips: recipe
            .clips
            .into_iter()
            .map(|clip| ClipSource {
                id: clip.id,
                mode: clip.mode,
                frames: clip
                    .frames
                    .into_iter()
                    .map(|r| Frame {
                        region: r.cell,
                        duration_ms: r.duration_ms,
                    })
                    .collect(),
            })
            .collect(),
    };
    // Reuse all ID, duration, reference, cardinality and atlas runtime contracts.
    SpriteDocument::new(source.clone()).map_err(|e| format!("sprite: {e}"))?;
    let json = json_bytes(&source)?;
    if json.len() > orr_sprite::MAX_JSON_BYTES {
        return Err("generated sprite JSON exceeds limit".into());
    }
    let out = checked_path(&request.out, true)?;
    if out.try_exists().map_err(|e| e.to_string())? {
        return Err("output already exists".into());
    }
    let base = recipe_path.parent().ok_or("recipe parent missing")?;
    let png = read_regular(&relative_input(base, &recipe.png)?, MAX_PNG_BYTES)?;
    // Decoder verifies actual PNG dimensions equal the exact full grid, rejects
    // animated/unsupported data and bounds RGBA before allocation. No re-encode.
    let rgba = orr_sample::project_sprites::decode_atlas(&png, width, height)?;
    if rgba.len() != rgba_bytes {
        return Err("decoded RGBA size mismatch".into());
    }
    drop(rgba);
    let license = read_regular(&relative_input(base, &recipe.license)?, MAX_LICENSE_BYTES)?;
    if std::str::from_utf8(&license)
        .map_err(|_| "license must be UTF-8")?
        .trim()
        .is_empty()
    {
        return Err("license must be nonempty UTF-8".into());
    }
    let package = Package {
        schema: 1,
        name: &recipe.name,
        version: &recipe.version,
        engine: "^0.0.1",
        capabilities: ["sprite"],
        dependencies: BTreeMap::new(),
        files: ["sprites.json", "atlas.png", "LICENSE.txt"],
    };
    publish(
        &out,
        &[
            ("sprites.json", &json),
            ("atlas.png", &png),
            ("LICENSE.txt", &license),
            ("orr.package.json", &json_bytes(&package)?),
        ],
    )?;
    Ok(Report {
        width,
        height,
        cells: cells as usize,
        rgba_bytes,
    })
}
