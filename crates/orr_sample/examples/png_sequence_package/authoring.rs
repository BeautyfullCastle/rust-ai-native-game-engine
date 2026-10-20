//! Private offline sequence authoring. Directories must remain trusted during use.
#[path = "clips.rs"]
mod clips;
#[path = "offline_io.rs"]
mod offline_io;
use offline_io::{
    checked_path, json_bytes, publish, read_regular, relative_input, rgba_size, validate_name,
    validate_version, MAX_RGBA_BYTES,
};

use orr_sprite::{Atlas, ClipSource, Frame, PlaybackMode, Region, SpriteDocument, SpriteSource};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::PathBuf};

pub const MAX_RECIPE_BYTES: usize = 1024 * 1024;
pub const MAX_FRAMES: usize = 256;
pub const MAX_PNG_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_TOTAL_PNG_BYTES: usize = 32 * 1024 * 1024;

pub const MAX_LICENSE_BYTES: usize = 64 * 1024;

pub struct Request {
    pub recipe: PathBuf,
    pub out: PathBuf,
}
#[derive(Debug, PartialEq, Eq)]
pub struct Report {
    pub width: u32,
    pub height: u32,
    pub frames: usize,
    pub rgba_bytes: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recipe {
    schema: u32,
    name: String,
    version: String,
    license: String,
    clip: String,
    mode: PlaybackMode,
    columns: u32,
    #[serde(default, deserialize_with = "present_object")]
    canvas: Option<Canvas>,
    #[serde(deserialize_with = "bounded_frames")]
    frames: Vec<InputFrame>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputFrame {
    png: String,
    duration_ms: u64,
    #[serde(default, deserialize_with = "present_object")]
    offset: Option<Offset>,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct Canvas {
    width: u32,
    height: u32,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct Offset {
    x: u32,
    y: u32,
}
// Normalized private authoring data. Atlas inputs and animation references have
// independent bounds: repeating a reference never repeats PNG work or pixels.
struct PreparedRecipe {
    schema: u32,
    name: String,
    version: String,
    license: String,
    columns: u32,
    canvas: Option<Canvas>,
    frames: Vec<SourceImage>,
    clips: Vec<ClipSource>,
}
struct SourceImage {
    png: String,
    offset: Option<Offset>,
}
// Absence is schema-dependent, but an explicitly supplied value must be an
// object, never null or a positional array. Map deserialization also preserves
// duplicate-key rejection instead of normalizing through serde_json::Value.
fn present_object<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct ObjectVisitor<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for ObjectVisitor<T> {
        type Value = Option<T>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("an object")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
            T::deserialize(serde::de::value::MapAccessDeserializer::new(map)).map(Some)
        }
    }
    deserializer.deserialize_map(ObjectVisitor(std::marker::PhantomData))
}
fn bounded_frames<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<InputFrame>, D::Error> {
    struct FramesVisitor;
    impl<'de> serde::de::Visitor<'de> for FramesVisitor {
        type Value = Vec<InputFrame>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("at most 256 ordered frames")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            let mut frames = Vec::new();
            while let Some(frame) = sequence.next_element::<InputFrame>()? {
                if frames.len() == MAX_FRAMES {
                    return Err(serde::de::Error::custom("frame count exceeds 256"));
                }
                frames.push(frame);
            }
            Ok(frames)
        }
    }
    deserializer.deserialize_seq(FramesVisitor)
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

fn png_dimensions(bytes: &[u8]) -> Result<(u32, u32), String> {
    if bytes.len() < 24
        || &bytes[..8] != b"\x89PNG\r\n\x1a\n"
        || &bytes[8..12] != 13u32.to_be_bytes().as_slice()
        || &bytes[12..16] != b"IHDR"
    {
        return Err("frame must be a PNG with an IHDR header".into());
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().map_err(|_| "PNG width")?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().map_err(|_| "PNG height")?);
    rgba_size(width, height)?;
    Ok((width, height))
}

fn prepare_recipe(bytes: &[u8]) -> Result<PreparedRecipe, String> {
    if clips::schema(bytes)? == 3 {
        return clips::prepare(bytes);
    }
    let recipe: Recipe = serde_json::from_slice(bytes).map_err(|e| format!("recipe: {e}"))?;
    if !matches!(recipe.schema, 1 | 2) {
        return Err("recipe requires schema 1, 2 or 3".into());
    }
    match recipe.schema {
        1 if recipe.canvas.is_some()
            || recipe.frames.iter().any(|frame| frame.offset.is_some()) =>
        {
            return Err("schema 1 does not allow canvas or frame offsets".into());
        }
        2 => {
            let canvas = recipe.canvas.ok_or("schema 2 requires canvas")?;
            rgba_size(canvas.width, canvas.height)?;
            if recipe.frames.iter().any(|frame| frame.offset.is_none()) {
                return Err("schema 2 requires an offset for every frame".into());
            }
        }
        _ => {}
    }
    if recipe.clip.trim().is_empty() || recipe.clip.len() > orr_sprite::MAX_CLIP_ID_BYTES {
        return Err("invalid clip name".into());
    }
    let mut duration = 0u64;
    for frame in &recipe.frames {
        if frame.duration_ms == 0 {
            return Err("frame duration must be positive".into());
        }
        duration = duration
            .checked_add(frame.duration_ms)
            .ok_or("clip duration overflow")?;
    }
    let clip = ClipSource {
        id: recipe.clip,
        mode: recipe.mode,
        frames: recipe
            .frames
            .iter()
            .enumerate()
            .map(|(index, frame)| Frame {
                region: index as u32,
                duration_ms: frame.duration_ms,
            })
            .collect(),
    };
    Ok(PreparedRecipe {
        schema: recipe.schema,
        name: recipe.name,
        version: recipe.version,
        license: recipe.license,
        columns: recipe.columns,
        canvas: recipe.canvas,
        frames: recipe
            .frames
            .into_iter()
            .map(|frame| SourceImage {
                png: frame.png,
                offset: frame.offset,
            })
            .collect(),
        clips: vec![clip],
    })
}

pub fn import_package(request: &Request) -> Result<Report, String> {
    let recipe_path = checked_path(&request.recipe, false)?;
    let recipe = prepare_recipe(&read_regular(&recipe_path, MAX_RECIPE_BYTES)?)?;
    if !validate_name(&recipe.name) || !validate_version(&recipe.version) {
        return Err("recipe requires valid package name/canonical version".into());
    }
    if recipe.frames.is_empty()
        || recipe.frames.len() > MAX_FRAMES
        || recipe.columns == 0
        || recipe.columns as usize > recipe.frames.len()
    {
        return Err("require 1..=256 frames and columns in 1..=frame count".into());
    }
    let out = checked_path(&request.out, true)?;
    if out.try_exists().map_err(|e| e.to_string())? {
        return Err("output already exists".into());
    }
    let base = recipe_path.parent().ok_or("recipe parent missing")?;
    let license = read_regular(&relative_input(base, &recipe.license)?, MAX_LICENSE_BYTES)?;
    if std::str::from_utf8(&license)
        .map_err(|_| "license must be UTF-8")?
        .trim()
        .is_empty()
    {
        return Err("license must be nonempty UTF-8 text".into());
    }
    // Metadata preflight bounds all compressed reads before retaining any PNG bytes.
    let mut paths = Vec::with_capacity(recipe.frames.len());
    let mut compressed = 0usize;
    for frame in &recipe.frames {
        let path = relative_input(base, &frame.png)?;
        let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if !meta.is_file() || meta.len() > MAX_PNG_BYTES as u64 {
            return Err("PNG must be a bounded regular file".into());
        }
        compressed = compressed
            .checked_add(meta.len() as usize)
            .filter(|n| *n <= MAX_TOTAL_PNG_BYTES)
            .ok_or("aggregate compressed PNG budget exceeded")?;
        paths.push(path);
    }
    let mut images = Vec::with_capacity(paths.len());
    let mut dimensions = None;
    let mut actual_compressed = 0usize;
    let mut decoded_total = 0usize;
    for (path, frame) in paths.into_iter().zip(&recipe.frames) {
        let remaining = MAX_TOTAL_PNG_BYTES - actual_compressed;
        let bytes = read_regular(&path, remaining.min(MAX_PNG_BYTES))?;
        actual_compressed += bytes.len();
        let size = png_dimensions(&bytes)?;
        if recipe.schema == 1 && dimensions.is_some_and(|expected| expected != size) {
            return Err("PNG frames must have equal dimensions".into());
        }
        if let Some(canvas) = recipe.canvas {
            let offset = frame.offset.ok_or("canvas requires frame offset")?;
            // Containment is based on the complete source rectangle, including
            // transparent pixels. Reject overflow or clipping before allocation.
            if offset
                .x
                .checked_add(size.0)
                .is_none_or(|x| x > canvas.width)
                || offset
                    .y
                    .checked_add(size.1)
                    .is_none_or(|y| y > canvas.height)
            {
                return Err("PNG frame placement exceeds canvas".into());
            }
        }
        dimensions = Some(size);
        decoded_total = decoded_total
            .checked_add(rgba_size(size.0, size.1)?)
            .filter(|n| *n <= MAX_RGBA_BYTES)
            .ok_or("aggregate decoded RGBA budget exceeded")?;
        images.push((bytes, size));
    }
    let (frame_width, frame_height) = recipe
        .canvas
        .map(|canvas| (canvas.width, canvas.height))
        .or(dimensions)
        .ok_or("no frames")?;
    let rows = (recipe.frames.len() as u32).div_ceil(recipe.columns);
    let width = frame_width
        .checked_mul(recipe.columns)
        .ok_or("atlas width overflow")?;
    let height = frame_height
        .checked_mul(rows)
        .ok_or("atlas height overflow")?;
    let rgba_bytes = rgba_size(width, height)?;
    // Both decoded and atlas budgets are established before PNG decoding or atlas allocation.
    let mut atlas = vec![0u8; rgba_bytes];
    let mut regions = Vec::with_capacity(recipe.frames.len());
    for (index, ((bytes, size), frame)) in images.iter().zip(&recipe.frames).enumerate() {
        // Decode one actual-size source at a time, with the existing decoder's
        // transformations. Never allocate a padded per-frame canvas.
        let rgba = orr_sample::project_sprites::decode_atlas(bytes, size.0, size.1)?;
        let x = (index as u32 % recipe.columns) * frame_width;
        let y = (index as u32 / recipe.columns) * frame_height;
        let offset = frame.offset.unwrap_or(Offset { x: 0, y: 0 });
        let stride = size.0 as usize * 4;
        // Top-left origin, +x right and +y down. Raw RGBA copy preserves hidden
        // RGB and partial alpha without blending; untouched padding stays zero.
        for row in 0..size.1 as usize {
            let destination =
                (((y + offset.y) as usize + row) * width as usize + (x + offset.x) as usize) * 4;
            atlas[destination..destination + stride]
                .copy_from_slice(&rgba[row * stride..(row + 1) * stride]);
        }
        regions.push(Region {
            id: index as u32,
            x,
            y,
            width: frame_width,
            height: frame_height,
        });
    }
    let source = SpriteSource {
        format: orr_sprite::FORMAT.into(),
        version: orr_sprite::VERSION,
        atlas: Atlas {
            image: "atlas.png".into(),
            width,
            height,
        },
        regions,
        clips: recipe.clips,
    };
    SpriteDocument::new(source.clone()).map_err(|e| e.to_string())?;
    let json = json_bytes(&source)?;
    let mut png = Vec::new();
    {
        let mut encoder = png17::Encoder::new(&mut png, width, height);
        encoder.set_color(png17::ColorType::Rgba);
        encoder.set_depth(png17::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&atlas).map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())?;
    }
    if png.len() > MAX_PNG_BYTES {
        return Err("encoded atlas exceeds PNG budget".into());
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
        frames: recipe.frames.len(),
        rgba_bytes,
    })
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
